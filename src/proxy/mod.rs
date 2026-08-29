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

    // Windows: AF_UNIX リスナーを TCP バイト中継として立てる。中継先は常に
    // 127.0.0.1 固定。config.proxy_host がループバックを含まない値なら中継を
    // 立てず warn のみ出す（launch 側はソケット不在で明示エラーになるため
    // fail-closed が保たれる）。bind 失敗はここで `?` により致命になる。
    #[cfg(windows)]
    let afunix_socket = if matches!(host.as_str(), "0.0.0.0" | "127.0.0.1" | "localhost") {
        Some(spawn_afunix_relay(port)?)
    } else {
        tracing::warn!(
            %host,
            "proxy_host cannot serve the 127.0.0.1 relay; unix socket relay disabled, remote control unavailable"
        );
        None
    };

    let pid_written = crate::process::daemon::write_pid(std::process::id());
    if pid_written.is_err() {
        // 掴んだソケットファイルを残さない（パイプ版の abort と同じ趣旨）
        #[cfg(windows)]
        if let Some(path) = &afunix_socket {
            let _ = std::fs::remove_file(path);
        }
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
    if let Some(path) = &afunix_socket {
        let _ = std::fs::remove_file(path);
    }

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

/// `from` → `to` へ EOF まで複製し、終端で `to` 側の書き込みを閉じる。転送バイト数を返す
///
/// Windows の AF_UNIX リスナーは同期 API（`uds_windows`）のため、非同期ランタイム
/// には接がず、接続ごとに専用スレッドで `std::io::copy` するだけの単純な中継に
/// 徹する。`#[cfg(any(windows, test))]` は mac の `cargo test` でも検証できる
/// ようにするため（mac の通常ビルドでは使われない）。
#[cfg(any(windows, test))]
fn relay_pump(
    mut from: impl std::io::Read + Send + 'static,
    mut to: impl std::io::Write + Send + 'static,
    shutdown_to: impl FnOnce() + Send + 'static,
) -> std::thread::JoinHandle<u64> {
    std::thread::spawn(move || {
        let copied = match std::io::copy(&mut from, &mut to) {
            Ok(n) => n,
            Err(e) => {
                tracing::debug!("relay_pump copy error: {e}");
                0
            }
        };
        shutdown_to();
        copied
    })
}

/// Windows 用 AF_UNIX → TCP バイト中継リスナー
///
/// Rust stable には Windows の AF_UNIX を非同期で listen する手段がない
/// （`tokio::net::UnixListener` は `cfg(unix)` 限定、mio の対応 PR は未マージの
/// ままクローズ、std は nightly のみ）。`uds_windows` の同期 API で accept し、
/// 接続ごとに `127.0.0.1:<port>` へ TCP を張って双方向にバイトを中継する
/// （HTTP は一切解釈しない）。
///
/// accept ループは detach したスレッドで動かす。プロセス終了で消えるため、
/// graceful 停止機構は作らない。
#[cfg(windows)]
fn spawn_afunix_relay(port: u16) -> Result<std::path::PathBuf> {
    use anyhow::Context;
    use std::io::ErrorKind;

    let path = crate::process::daemon::socket_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("cannot create socket directory {}", parent.display()))?;
    }

    // Windows の AF_UNIX ソケットファイルは IO_REPARSE_TAG_AF_UNIX の
    // リパースポイントであり、Path::exists() は偽陰性を返しうる。存在判定に
    // 依存せず常に remove_file を試み、NotFound のみ許容する。
    if let Err(e) = std::fs::remove_file(&path) {
        if e.kind() != ErrorKind::NotFound {
            return Err(e)
                .with_context(|| format!("cannot remove stale unix socket {}", path.display()));
        }
    }

    let listener = uds_windows::UnixListener::bind(&path)
        .with_context(|| format!("cannot bind unix socket {}", path.display()))?;

    tracing::info!(path = %path.display(), "unix socket relay ready");

    std::thread::spawn(move || {
        let mut seq: u64 = 0;
        for conn in listener.incoming() {
            match conn {
                Ok(unix) => {
                    seq += 1;
                    tracing::info!(seq, "unix socket connection accepted");
                    std::thread::spawn(move || relay_afunix_connection(unix, port));
                }
                Err(e) => {
                    tracing::warn!("unix socket accept error: {e}");
                }
            }
        }
    });

    Ok(path)
}

/// 1本の AF_UNIX 接続を `127.0.0.1:<port>` の既存 TCP proxy へバイト中継する
///
/// HTTP を解釈しないため keep-alive・chunked・SSE ストリーミングがそのまま
/// 透過し、ルーティング・handler・ログは既存の TCP 経路をそのまま使う。
#[cfg(windows)]
fn relay_afunix_connection(unix: uds_windows::UnixStream, port: u16) {
    use std::net::{Shutdown, TcpStream};

    let tcp = match TcpStream::connect(("127.0.0.1", port)) {
        Ok(tcp) => tcp,
        Err(e) => {
            tracing::warn!("cannot connect to local proxy port {port}: {e}");
            return;
        }
    };

    let unix_read = match unix.try_clone() {
        Ok(u) => u,
        Err(e) => {
            tracing::warn!("cannot clone unix socket stream: {e}");
            return;
        }
    };
    let unix_shutdown = match unix.try_clone() {
        Ok(u) => u,
        Err(e) => {
            tracing::warn!("cannot clone unix socket stream: {e}");
            return;
        }
    };
    let tcp_read = match tcp.try_clone() {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!("cannot clone tcp stream: {e}");
            return;
        }
    };
    let tcp_shutdown = match tcp.try_clone() {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!("cannot clone tcp stream: {e}");
            return;
        }
    };

    // unix → tcp（リクエスト方向）。unix 側が EOF になったら tcp の書き込みを閉じる
    relay_pump(unix_read, tcp, move || {
        let _ = tcp_shutdown.shutdown(Shutdown::Write);
    });
    // tcp → unix（レスポンス方向）。tcp 側が EOF になったら unix の書き込みを閉じる
    relay_pump(tcp_read, unix, move || {
        let _ = unix_shutdown.shutdown(Shutdown::Write);
    });
}

#[cfg(test)]
mod relay_pump_tests {
    use super::relay_pump;
    use std::io::{Read, Write};
    use std::net::{Shutdown, TcpListener, TcpStream};
    use std::sync::mpsc;
    use std::time::Duration;

    /// (a) 透過性: 2組の TCP 接続を relay_pump で双方向に繋ぎ、上り・下りそれぞれ
    /// 書いたバイト列が反対側で同一に読めることを確認する。
    #[test]
    fn test_relay_pump_is_transparent_both_directions() {
        let left_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let right_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let left_addr = left_listener.local_addr().unwrap();
        let right_addr = right_listener.local_addr().unwrap();

        let mut left_client = TcpStream::connect(left_addr).unwrap();
        let (left_server, _) = left_listener.accept().unwrap();
        let mut right_client = TcpStream::connect(right_addr).unwrap();
        let (right_server, _) = right_listener.accept().unwrap();

        // left_server <-> right_server を relay_pump で中継する
        // （実運用の unix <-> tcp 中継と同じ構図を TCP-TCP で再現）
        let left_server_read = left_server.try_clone().unwrap();
        let right_server_read = right_server.try_clone().unwrap();
        relay_pump(left_server_read, right_server.try_clone().unwrap(), || {});
        relay_pump(right_server_read, left_server.try_clone().unwrap(), || {});

        left_client.write_all(b"upstream-bytes").unwrap();
        let mut buf = [0u8; 14];
        right_client.read_exact(&mut buf).unwrap();
        assert_eq!(&buf, b"upstream-bytes");

        right_client.write_all(b"downstream-data").unwrap();
        let mut buf2 = [0u8; 15];
        left_client.read_exact(&mut buf2).unwrap();
        assert_eq!(&buf2, b"downstream-data");
    }

    /// (b) 終端伝播: `to` の複製を relay_pump 呼び出しの外で保持したまま渡す
    /// （実運用の Windows 経路と同じ構図。drop だけでは反対側は EOF しない —
    /// TCP の shutdown はソケット単位で効くため、複製が生きていても shutdown()
    /// を呼べば相手には FIN が届く）。`shutdown_to` がチャネル送信で実際に
    /// 発火したことを assert しつつ、反対側の read が EOF になることを確認する。
    #[test]
    fn test_relay_pump_shutdown_propagates_eof_even_with_retained_clone() {
        let to_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let to_addr = to_listener.local_addr().unwrap();
        let mut to_peer = TcpStream::connect(to_addr).unwrap();
        let (to, _) = to_listener.accept().unwrap();

        // 実運用と同じく、relay_pump に渡す `to` とは別に複製を保持し続ける。
        // これが生きたままでも shutdown_to の明示 shutdown() で EOF が伝わることを見る。
        let to_retained = to.try_clone().unwrap();
        let to_for_shutdown = to.try_clone().unwrap();

        // すぐに EOF する読み取り元
        let from = std::io::Cursor::new(Vec::<u8>::new());

        let (tx, rx) = mpsc::channel::<()>();
        let handle = relay_pump(from, to, move || {
            tx.send(()).unwrap();
            let _ = to_for_shutdown.shutdown(Shutdown::Write);
        });
        handle.join().unwrap();

        rx.recv_timeout(Duration::from_secs(1))
            .expect("shutdown_to should have fired");

        to_peer
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut buf = Vec::new();
        // to_retained がまだ生きている間に読み切れること（= drop 待ちではなく
        // shutdown() が EOF を引き起こしたこと）を確認してから解放する
        to_peer.read_to_end(&mut buf).unwrap();
        drop(to_retained);

        assert!(buf.is_empty());
    }
}
