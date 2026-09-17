pub mod adapter;
pub mod context_engine;
pub mod error;
pub mod fallback;
pub mod forward;
pub mod handler;
pub mod health;
pub mod metrics;
pub mod models;
pub mod translate;
pub mod util;

use std::sync::Arc;

use anyhow::Result;
use axum::routing::{get, post};
use axum::Router;
use tokio::sync::RwLock;

use crate::config::ClaudexConfig;
use crate::context::rag::RagIndex;
use crate::context::sharing::SharedContext;
use metrics::MetricsStore;

/// 未命中任何路由的请求：记录 method/path 后返回 404。
/// 用于发现 Claude Code 打到非 /v1/messages 路径的内部请求。
async fn log_unmatched(
    method: axum::http::Method,
    uri: axum::http::Uri,
    body: axum::body::Bytes,
) -> (axum::http::StatusCode, &'static str) {
    tracing::warn!(
        method = %method,
        path = %uri.path(),
        query = ?uri.query(),
        body_len = body.len(),
        "unmatched request"
    );
    (axum::http::StatusCode::NOT_FOUND, "not found")
}

pub struct ProxyState {
    pub config: Arc<RwLock<ClaudexConfig>>,
    pub metrics: MetricsStore,
    pub http_client: reqwest::Client,
    pub health_status: Arc<RwLock<health::HealthMap>>,
    pub circuit_breakers: fallback::CircuitBreakerMap,
    pub shared_context: SharedContext,
    pub rag_index: Option<RagIndex>,
    pub token_manager: crate::oauth::manager::TokenManager,
    pub forward: Option<std::sync::Arc<forward::ForwardState>>,
}

/// 获取 proxy 日志文件路径（~/.cache/claudex/proxy-{timestamp}-{pid}.log）
/// 每次启动生成独立日志文件，支持多实例并行
pub fn proxy_log_path() -> Option<std::path::PathBuf> {
    dirs::cache_dir().map(|d| {
        let ts = chrono::Local::now().format("%Y%m%d-%H%M%S");
        let pid = std::process::id();
        d.join("claudex").join(format!("proxy-{ts}-{pid}.log"))
    })
}

pub async fn start_proxy(config: ClaudexConfig, port_override: Option<u16>) -> Result<()> {
    let port = port_override.unwrap_or(config.proxy_port);
    let host = config.proxy_host.clone();
    let forward_port = config.forward_proxy_port;

    // forward::ForwardState::client と同じ規則に揃える（FR-013）。推論の上流接続は
    // この http_client を使うため、ForwardState::client だけを直しても FR-013 は満たせない。
    let builder = reqwest::Client::builder().timeout(std::time::Duration::from_secs(300));
    let builder = if forward::env_proxy_points_at_self(forward_port) {
        builder.no_proxy()
    } else {
        builder
    };
    let http_client = builder.build()?;

    // Build RAG index if enabled
    let rag_index = if config.context.rag.enabled {
        let index = RagIndex::new(config.context.rag.clone());
        if let Some((base_url, api_key, _)) = crate::context::resolve_profile_endpoint(
            &config,
            &config.context.rag.profile,
            &config.context.rag.model,
        ) {
            if let Err(e) = index.build_index(&http_client, &base_url, &api_key).await {
                tracing::warn!("failed to build RAG index: {e}");
            }
        } else {
            tracing::warn!(
                profile = %config.context.rag.profile,
                "RAG profile not found, skipping index build"
            );
        }
        Some(index)
    } else {
        None
    };

    let token_manager = crate::oauth::manager::TokenManager::new(http_client.clone());

    let (forward_state, ca_pem) = forward::ForwardState::new(forward_port)?;
    let forward_state = std::sync::Arc::new(forward_state);
    let forward_secret = forward_state.secret.clone();

    let state = Arc::new(ProxyState {
        config: Arc::new(RwLock::new(config)),
        metrics: MetricsStore::new(),
        http_client,
        health_status: Arc::new(RwLock::new(health::HealthMap::new())),
        circuit_breakers: fallback::new_circuit_breaker_map(),
        shared_context: SharedContext::new(),
        rag_index,
        token_manager,
        forward: Some(forward_state.clone()),
    });

    health::spawn_health_checker(state.clone());

    let app = Router::new()
        .route("/v1/models", get(models::list_models))
        .route(
            "/proxy/{profile}/v1/messages",
            post(handler::handle_messages),
        )
        .route("/health", get(|| async { "ok" }))
        .fallback(log_unmatched)
        .with_state(state.clone());

    let bind_addr = format!("{host}:{port}");
    let listener = tokio::net::TcpListener::bind(&bind_addr).await?;
    let forward_listener = forward::bind(forward_port).await?;

    tracing::info!("proxy listening on {bind_addr}");
    tracing::info!("forward proxy listening on 127.0.0.1:{forward_port}");

    let pid_written = crate::process::daemon::write_pid(std::process::id());
    pid_written?;

    let forward_handoff = forward::handoff::ForwardHandoff {
        port: forward_listener.local_addr()?.port(),
        secret: forward_secret,
    };
    forward::handoff::write(&forward_handoff, &ca_pem)?;

    let forward_handle = forward::spawn(forward_listener, state.clone());

    let result = axum::serve(listener, app).await;

    forward_handle.abort();
    forward::handoff::cleanup();

    crate::process::daemon::remove_pid()?;
    result?;
    Ok(())
}
