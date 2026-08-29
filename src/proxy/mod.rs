pub mod adapter;
pub mod context_engine;
pub mod error;
pub mod fallback;
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

    let http_client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .build()?;

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

    let state = Arc::new(ProxyState {
        config: Arc::new(RwLock::new(config)),
        metrics: MetricsStore::new(),
        http_client,
        health_status: Arc::new(RwLock::new(health::HealthMap::new())),
        circuit_breakers: fallback::new_circuit_breaker_map(),
        shared_context: SharedContext::new(),
        rag_index,
        token_manager,
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
        .with_state(state);

    let bind_addr = format!("{host}:{port}");
    let listener = tokio::net::TcpListener::bind(&bind_addr).await?;

    tracing::info!("proxy listening on {bind_addr}");

    // 名前付きパイプは PID ファイルより先に立てる。TCP の bind と同じく失敗を
    // 致命として扱うので、ここで bail したときに stale な PID ファイルを残さない。
    // （名前を先取りされていれば first_pipe_instance(true) が失敗する。TCP だけで
    // 起動を続けると、launch 側が攻撃者のパイプへトークンを渡しうる）
    #[cfg(windows)]
    let pipe_server = spawn_pipe_listener(app.clone())?;

    let pid_written = crate::process::daemon::write_pid(std::process::id());
    #[cfg(windows)]
    if pid_written.is_err() {
        // PID ファイルが無いとパイプは launch 側から使えないので、掴んだまま残さない
        pipe_server.abort();
    }
    pid_written?;

    #[cfg(unix)]
    let unix_server = spawn_unix_listener(app.clone());

    let result = axum::serve(listener, app).await;

    #[cfg(unix)]
    if let Some(handle) = unix_server {
        handle.abort();
    }

    #[cfg(windows)]
    pipe_server.abort();

    crate::process::daemon::remove_pid()?;
    result?;
    Ok(())
}

/// Remote Control 用の Unix ドメインソケットで待ち受ける
///
/// TCP と同じ Router をそのまま使う。Claude Code は
/// `ANTHROPIC_BASE_URL=http://api.anthropic.com/proxy/<profile>` の
/// パスを保ったままソケットへ流すので、ルーティングは共通のままでよい。
#[cfg(unix)]
fn spawn_unix_listener(app: Router) -> Option<tokio::task::JoinHandle<()>> {
    let path = match crate::process::daemon::socket_path() {
        Ok(path) => path,
        Err(e) => {
            tracing::warn!("cannot determine unix socket path: {e}");
            return None;
        }
    };

    // 前回のプロセスが残した socket ファイルは bind の前に消す
    if path.exists() {
        if let Err(e) = std::fs::remove_file(&path) {
            tracing::warn!(path = %path.display(), "cannot remove stale socket: {e}");
            return None;
        }
    }

    let listener = match tokio::net::UnixListener::bind(&path) {
        Ok(listener) => listener,
        Err(e) => {
            tracing::warn!(path = %path.display(), "cannot bind unix socket: {e}");
            return None;
        }
    };

    tracing::info!(path = %path.display(), "proxy listening on unix socket");

    Some(tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            tracing::error!("unix socket server stopped: {e}");
        }
    }))
}

/// Remote Control 用の名前付きパイプで待ち受けるための `axum::serve::Listener` 実装
///
/// `pending` には次の client がすぐ接続できるよう、常に「作成済みだが未接続」の
/// pipe instance を先回りして保持しておく。`accept()` はそれを消費して `connect()`
/// を待ち、成功したら次の instance を作ってから返す。
#[cfg(windows)]
struct NamedPipeListener {
    pipe_name: String,
    pending: Option<tokio::net::windows::named_pipe::NamedPipeServer>,
    /// 診断用: これまでに accept() が返した接続の本数（1 始まり）。
    /// Claude Code からの接続がそもそも来ているか（(a)）を切り分けるためだけに使う。
    accepted: u64,
}

#[cfg(windows)]
impl axum::serve::Listener for NamedPipeListener {
    type Io = PipeConnection;
    type Addr = String;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        use tokio::net::windows::named_pipe::ServerOptions;

        loop {
            let server = match self.pending.take() {
                Some(server) => server,
                None => match ServerOptions::new()
                    .reject_remote_clients(true)
                    .create(&self.pipe_name)
                {
                    Ok(server) => server,
                    Err(e) => {
                        tracing::warn!(pipe = %self.pipe_name, "cannot create named pipe instance: {e}");
                        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                        continue;
                    }
                },
            };

            if let Err(e) = server.connect().await {
                tracing::warn!(pipe = %self.pipe_name, "named pipe connect failed: {e}");
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                continue;
            }

            self.accepted += 1;
            let seq = self.accepted;
            // 診断用: 接続が実際に来たことを分かるようにする。ここが出ない場合は
            // Claude Code 側がパイプへ到達できていない（切り分け (a)）ことを意味する。
            tracing::info!(pipe = %self.pipe_name, seq, "pipe connection accepted");

            // 次の client がすぐ接続できるよう、先に次の instance を用意してから返す
            match ServerOptions::new()
                .reject_remote_clients(true)
                .create(&self.pipe_name)
            {
                Ok(next) => {
                    self.pending = Some(next);
                    tracing::debug!(pipe = %self.pipe_name, seq, "next named pipe instance pre-created");
                }
                Err(e) => {
                    tracing::warn!(pipe = %self.pipe_name, "cannot pre-create next named pipe instance: {e}");
                }
            }

            let connection = PipeConnection {
                inner: server,
                pipe_name: self.pipe_name.clone(),
                seq,
                bytes_read: 0,
                first_read_logged: false,
            };

            return (connection, self.pipe_name.clone());
        }
    }

    fn local_addr(&self) -> std::io::Result<Self::Addr> {
        Ok(self.pipe_name.clone())
    }
}

/// 診断用: 接続済み `NamedPipeServer` を薄く包み、読み取ったバイト数の累計と
/// 最初の読み取りが起きたかどうかを記録する。接続が閉じられたら `Drop` で
/// 累計をログに出す。切り分け (a)（接続は来るが HTTP のやり取りが成立しない）
/// を見るためだけのもので、リクエスト本文そのものは一切ログに出さない。
#[cfg(windows)]
struct PipeConnection {
    inner: tokio::net::windows::named_pipe::NamedPipeServer,
    pipe_name: String,
    seq: u64,
    bytes_read: u64,
    first_read_logged: bool,
}

#[cfg(windows)]
impl tokio::io::AsyncRead for PipeConnection {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let this = self.get_mut();
        let before = buf.filled().len();
        let poll = std::pin::Pin::new(&mut this.inner).poll_read(cx, buf);
        if let std::task::Poll::Ready(Ok(())) = &poll {
            let n = (buf.filled().len() - before) as u64;
            if n > 0 {
                this.bytes_read += n;
                if !this.first_read_logged {
                    this.first_read_logged = true;
                    tracing::debug!(pipe = %this.pipe_name, seq = this.seq, "pipe first read occurred");
                }
            }
        }
        poll
    }
}

#[cfg(windows)]
impl tokio::io::AsyncWrite for PipeConnection {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        std::pin::Pin::new(&mut self.get_mut().inner).poll_write(cx, buf)
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }

    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}

#[cfg(windows)]
impl Drop for PipeConnection {
    fn drop(&mut self) {
        tracing::info!(
            pipe = %self.pipe_name,
            seq = self.seq,
            bytes_read = self.bytes_read,
            "pipe connection closed"
        );
    }
}

/// Remote Control 用の名前付きパイプで待ち受ける
///
/// TCP と同じ Router をそのまま使う。Windows 版 Claude Code は
/// `ANTHROPIC_UNIX_SOCKET` 相当の値として渡されたパイプパスへ推論リクエストを
/// 流すので、ルーティングは共通のままでよい。
///
/// 失敗は `TcpListener::bind` と同じく致命として呼び出し元へ返す。パイプ名は
/// 秘密ではなく他ローカルユーザーが先取りできるので、`first_pipe_instance(true)`
/// の失敗を warn で流して TCP のみで起動を続けると、launch 側が攻撃者のパイプへ
/// claude.ai のトークンを渡す経路が開く。
#[cfg(windows)]
fn spawn_pipe_listener(app: Router) -> Result<tokio::task::JoinHandle<()>> {
    use anyhow::Context;
    use tokio::net::windows::named_pipe::ServerOptions;

    let pipe_name = crate::process::daemon::socket_path()
        .context("cannot determine named pipe path")?
        .to_string_lossy()
        .into_owned();

    // 最初の instance はここで eager に作る。2 本目以降は accept() 内で先回り作成する。
    // first_pipe_instance(true) は、同名パイプが既にある場合に失敗する。
    let first = ServerOptions::new()
        .first_pipe_instance(true)
        .reject_remote_clients(true)
        .create(&pipe_name)
        .with_context(|| {
            format!(
                "cannot create named pipe {pipe_name}. \
                 Another claudex proxy may already be running, or the name is taken by another process"
            )
        })?;

    tracing::info!(pipe = %pipe_name, "proxy listening on named pipe");

    let listener = NamedPipeListener {
        pipe_name,
        pending: Some(first),
        accepted: 0,
    };

    Ok(tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            tracing::error!("named pipe server stopped: {e}");
        }
    }))
}
