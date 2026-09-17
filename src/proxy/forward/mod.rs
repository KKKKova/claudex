//! Remote Control 用 forward proxy（`api.anthropic.com` の TLS を終端する MITM プロキシ）の土台
//!
//! `ForwardState` は TLS acceptor・合言葉・上流への reqwest クライアントを束ねる。
//! CONNECT の受付・中継処理自体は T006 で実装する。

pub mod handoff;
pub mod identity;
pub mod route;
pub mod tls;

use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper::Request;
use hyper_util::rt::TokioIo;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};

use identity::Identity;

use crate::proxy::ProxyState;

/// 終端対象ホスト。これ以外の CONNECT は素通しする
pub const TERMINATE_HOST: &str = "api.anthropic.com";

/// forward proxy の実行状態
pub struct ForwardState {
    pub acceptor: tokio_rustls::TlsAcceptor,
    pub secret: String,
    pub upstream_base: String,
    pub client: reqwest::Client,
}

impl ForwardState {
    /// 証明書・合言葉・上流クライアントを生成する。CA 証明書の PEM も返す
    pub fn new(forward_port: u16) -> Result<(Self, String)> {
        // rustls の既定プロバイダを立てる（reqwest と同居させるため明示する）。
        // 二重呼び出しは Err を返すだけなので無視してよい。
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

        let generated = tls::build_tls()?;

        let mut buf = [0u8; 32];
        rand::fill(&mut buf);
        let secret = URL_SAFE_NO_PAD.encode(buf);

        let builder = reqwest::Client::builder().timeout(Duration::from_secs(300));
        let builder = if env_proxy_points_at_self(forward_port) {
            builder.no_proxy()
        } else {
            builder
        };
        let client = builder.build()?;

        let state = ForwardState {
            acceptor: generated.acceptor,
            secret,
            upstream_base: "https://api.anthropic.com".to_string(),
            client,
        };

        Ok((state, generated.ca_pem))
    }
}

/// 127.0.0.1 固定で bind する。失敗はポート番号付きの致命エラー（FR-019）
///
/// `config.proxy_host` は参照しない。`proxy_host = "0.0.0.0"` を設定していても、
/// forward proxy はループバックのみで待ち受ける（FR-001 第2受入基準）。
pub async fn bind(port: u16) -> Result<TcpListener> {
    match TcpListener::bind(("127.0.0.1", port)).await {
        Ok(listener) => Ok(listener),
        Err(e) => anyhow::bail!(
            "cannot bind forward proxy port {port} on 127.0.0.1: {e}. Another process is using it — free the port or set forward_proxy_port in config"
        ),
    }
}

/// accept ループを spawn する
///
/// 接続ごとに `tokio::spawn` して並行に捌く。1接続の失敗（`handle_conn` の `Err`）は
/// warn を残すだけで、accept ループ自体は止めない。
pub fn spawn(listener: TcpListener, state: Arc<ProxyState>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let (client, _) = match listener.accept().await {
                Ok(pair) => pair,
                Err(e) => {
                    tracing::warn!("forward proxy: accept error: {e}");
                    continue;
                }
            };
            let state = state.clone();
            tokio::spawn(async move {
                if let Err(e) = handle_conn(client, state).await {
                    tracing::warn!("forward proxy: connection error: {e}");
                }
            });
        }
    })
}

/// forward proxy の1接続を処理する。`examples/mitm_poc.rs` の `handle_conn` が土台
///
/// `TERMINATE_HOST` 宛の CONNECT だけ TLS を終端し、それ以外の CONNECT は素通しトンネルに、
/// CONNECT を使わない絶対 URI 形式の要求はそのまま HTTP/1.1 として捌く。
async fn handle_conn(client: TcpStream, state: Arc<ProxyState>) -> Result<()> {
    let Some(forward) = state.forward.clone() else {
        anyhow::bail!("forward state is not initialized");
    };

    // CONNECT かどうかを、バイトを消費せずに覗いて決める。Remote Control のブリッジは
    // CONNECT を使わず、絶対 URI 形式の要求をそのままプロキシへ投げてくることがある
    // （POST https://api.anthropic.com/... HTTP/1.1）。
    let mut head = [0u8; 8];
    let n = client.peek(&mut head).await?;
    if n == 0 {
        return Ok(());
    }
    if !head[..n].starts_with(b"CONNECT") {
        // この経路は要求ごとに Proxy-Authorization が付きうるので、identity を
        // 接続単位で固定せず、serve() の中で要求ごとに解決させる
        return serve(TokioIo::new(client), state, Identity::Absent, true).await;
    }

    let mut reader = BufReader::new(client);

    let mut request_line = String::new();
    if reader.read_line(&mut request_line).await? == 0 {
        return Ok(());
    }

    let mut proxy_authorization: Option<String> = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).await? == 0 || line == "\r\n" || line == "\n" {
            break;
        }
        const HEADER: &str = "Proxy-Authorization:";
        if line.len() >= HEADER.len() && line[..HEADER.len()].eq_ignore_ascii_case(HEADER) {
            proxy_authorization = Some(line[HEADER.len()..].trim().to_string());
        }
    }

    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let target = parts.next().unwrap_or_default().to_string();

    if !method.eq_ignore_ascii_case("CONNECT") {
        tracing::warn!(
            request_line = %request_line.trim_end(),
            "forward proxy: expected CONNECT request line"
        );
        let mut client = reader.into_inner();
        client
            .write_all(b"HTTP/1.1 405 Method Not Allowed\r\nContent-Length: 0\r\n\r\n")
            .await?;
        return Ok(());
    }

    // Proxy-Authorization は1回だけ解析し、この接続の属性として保持する（FR-015）
    let identity = identity::resolve(&state, proxy_authorization.as_deref()).await;

    if target.split(':').next() == Some(TERMINATE_HOST) {
        let mut client = reader.into_inner();
        client
            .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
            .await?;
        let tls = forward.acceptor.accept(client).await?;
        return serve(TokioIo::new(tls), state, identity, false).await;
    }

    // TERMINATE_HOST 以外は素通しトンネルにする。上流への接続を先に試し、
    // 成功したときだけ 200 を返す
    let pending = reader.buffer().to_vec();
    let mut client = reader.into_inner();

    match TcpStream::connect(&target).await {
        Ok(upstream) => {
            client
                .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
                .await?;
            let (mut cr, mut cw) = client.into_split();
            let (mut ur, mut uw) = upstream.into_split();
            if !pending.is_empty() {
                uw.write_all(&pending).await?;
            }
            let c2u = async { tokio::io::copy(&mut cr, &mut uw).await };
            let u2c = async { tokio::io::copy(&mut ur, &mut cw).await };
            let _ = tokio::join!(c2u, u2c);
            Ok(())
        }
        Err(e) => {
            tracing::warn!(target = %target, "forward proxy: cannot connect upstream: {e}");
            client
                .write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\n\r\n")
                .await?;
            Ok(())
        }
    }
}

/// 終端した（または絶対 URI で届いた）接続を HTTP/1.1 として捌く
///
/// `per_request` が真のときは要求ごとに `Proxy-Authorization` ヘッダから identity を
/// 解決し直す。偽のときは呼び出し側が接続単位で決めた `identity` をそのまま使う。
async fn serve<I>(
    io: I,
    state: Arc<ProxyState>,
    identity: Identity,
    per_request: bool,
) -> Result<()>
where
    I: hyper::rt::Read + hyper::rt::Write + Unpin + Send + 'static,
{
    let service = service_fn(move |req: Request<Incoming>| {
        let state = state.clone();
        let identity = identity.clone();
        async move {
            let identity = if per_request {
                let header = req
                    .headers()
                    .get("proxy-authorization")
                    .and_then(|v| v.to_str().ok());
                identity::resolve(&state, header).await
            } else {
                identity
            };
            let authority = req.uri().authority().map(|a| a.to_string());
            let response = route::dispatch(state, identity, req, authority).await;
            Ok::<_, std::convert::Infallible>(response)
        }
    });

    if let Err(e) = hyper::server::conn::http1::Builder::new()
        .serve_connection(io, service)
        .await
    {
        tracing::warn!("forward proxy: serve_connection error: {e}");
    }
    Ok(())
}

/// 環境変数が claudex 自身の forward proxy を指しているかを判定する（FR-013）
///
/// `HTTPS_PROXY` → `https_proxy` → `ALL_PROXY` → `all_proxy` の順に最初に見つかった値だけを見る。
/// `claudex run` は `HTTPS_PROXY` と `https_proxy` の両方を子プロセスへ撒くため、セッションの
/// 中から `claudex proxy start` を打つと proxy 自身がこれらを継承する。`ALL_PROXY` は reqwest が
/// 既定で読むため、ここに claudex 自身が入っていても自己ループになる。
pub fn env_proxy_points_at_self(forward_port: u16) -> bool {
    for key in ["HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy"] {
        if let Ok(value) = std::env::var(key) {
            return proxy_url_points_at_port(&value, forward_port);
        }
    }
    false
}

/// `env_proxy_points_at_self` の判定本体。純粋関数、テスト用
pub fn proxy_url_points_at_port(value: &str, port: u16) -> bool {
    let Ok(url) = url::Url::parse(value) else {
        return false;
    };
    let Some(host) = url.host_str() else {
        return false;
    };
    // url crate は IPv6 ループバックの host_str() を "[::1]" と返す場合がある
    let is_loopback_host = matches!(host, "127.0.0.1" | "localhost" | "::1" | "[::1]");
    is_loopback_host && url.port() == Some(port)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_points_at_self_loopback() {
        assert!(proxy_url_points_at_port("http://127.0.0.1:13457", 13457));
        assert!(proxy_url_points_at_port("http://localhost:13457", 13457));
        assert!(proxy_url_points_at_port("http://[::1]:13457", 13457));
    }

    #[test]
    fn test_points_at_self_other_port() {
        assert!(!proxy_url_points_at_port("http://127.0.0.1:13456", 13457));
    }

    #[test]
    fn test_points_at_self_corporate_proxy() {
        assert!(!proxy_url_points_at_port(
            "http://proxy.corp.example:8080",
            13457
        ));
    }

    #[test]
    fn test_points_at_self_no_port() {
        assert!(!proxy_url_points_at_port("http://127.0.0.1", 13457));
    }

    #[test]
    fn test_points_at_self_unparsable() {
        assert!(!proxy_url_points_at_port("not a url", 13457));
    }
}
