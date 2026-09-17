//! forward proxy の統合テスト（T009）
//!
//! `tests/` からの統合テストでは内部モジュール（`ForwardState` 等）に到達できない
//! （このクレートは `[lib]` を持たないバイナリクレートである）。`#[path]` でソースを
//! 直接取り込む方式（plan §3 選択肢 b）も検討したが、`proxy::handler::handle_messages` は
//! `router::classifier` を、`config` は `context` / `oauth` / `router` / `cli` を、`oauth` は
//! 逆に `config` を、と相互に `crate::` パスで依存し合っており、取り込みがクレートの
//! 大半へ芋づる式に広がるため現実的でないと判断した。そのため統合テストは
//! `src/proxy/forward/` 配下の `#[cfg(test)] mod`（選択肢 c）として実装し、
//! `cargo test --bin claudex` で実行する。
//!
//! 上流役は基本的に `wiremock::MockServer` を使う。ただし以下の2つは wiremock では
//! 表現できないため手書きの `TcpListener` を使う:
//! - SSE チャンクを保持したまま応答を閉じない上流（`wiremock` はレスポンスを即座に
//!   完結させる）
//! - 絶対 URI 形式の要求（`route::relay_upstream` は authority がある場合、常に
//!   `https://{authority}` を組み立てて中継する。`ForwardState::upstream_base` の
//!   差し替えが効かない経路のため、`wiremock` の plain HTTP では応答できない。
//!   自前の TLS サーバを `api.anthropic.com` として証明書ごと立て、
//!   `reqwest::ClientBuilder::resolve` で DNS を差し替えることで、本物の
//!   `api.anthropic.com` には一切到達させずに検証する）

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use rustls::pki_types::CertificateDer;
use rustls::{ClientConfig, RootCertStore};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::RwLock;
use tokio_rustls::client::TlsStream;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::tls::build_tls;
use super::{bind, spawn, ForwardState, TERMINATE_HOST};
use crate::config::{ClaudexConfig, ProfileConfig, ProviderType};
use crate::context::sharing::SharedContext;
use crate::oauth::manager::TokenManager;
use crate::proxy::health::HealthMap;
use crate::proxy::metrics::MetricsStore;
use crate::proxy::{fallback, ProxyState};

/// `/v1/messages` 用の最小のリクエストボディ
const MESSAGES_BODY: &[u8] =
    br#"{"model":"claude-3-5-sonnet-20241022","max_tokens":10,"messages":[{"role":"user","content":"hi"}]}"#;

/// `/v1/messages` (stream) 用の最小のリクエストボディ
const STREAMING_MESSAGES_BODY: &[u8] = br#"{"model":"claude-3-5-sonnet-20241022","max_tokens":10,"stream":true,"messages":[{"role":"user","content":"hi"}]}"#;

/// `alpha` / `beta` プロファイルと合言葉を組んだ最小の `ProxyState` を forward proxy に
/// 載せて起動する（plan §5 Step 1）。戻り値は (ポート, 合言葉, CA PEM)。
async fn spawn_forward(
    upstream_base: String,
    alpha_base: String,
    beta_base: String,
) -> (u16, String, String) {
    let (mut forward_state, ca_pem) =
        ForwardState::new(0).expect("ForwardState::new should succeed");
    forward_state.upstream_base = upstream_base;
    // `ForwardState::new(0)` が作るクライアントは `env_proxy_points_at_self(0)` が偽になり
    // 開発機の HTTPS_PROXY を尊重してしまうため組み直す。あわせて、絶対 URI 形式の要求が
    // 強制する `https://api.anthropic.com` 宛だけを自前の TLS サーバへ差し替える
    // （ポートは要求 URL 側が優先されるため、ここで指定する値は使われない）。
    // 他プロファイルは plain HTTP の wiremock を使うのでこの上書きとは無関係であり、
    // `danger_accept_invalid_certs` もそれらには影響しない。
    forward_state.client = reqwest::Client::builder()
        .no_proxy()
        .resolve(TERMINATE_HOST, SocketAddr::from(([127, 0, 0, 1], 1)))
        .danger_accept_invalid_certs(true)
        .build()
        .expect("forward client should build");
    let forward_state = Arc::new(forward_state);
    let secret = forward_state.secret.clone();

    let http_client = reqwest::Client::builder()
        .no_proxy()
        .build()
        .expect("http client should build");
    let token_manager = TokenManager::new(http_client.clone());

    let config = ClaudexConfig {
        profiles: vec![
            ProfileConfig {
                name: "alpha".to_string(),
                provider_type: ProviderType::DirectAnthropic,
                base_url: alpha_base,
                default_model: "claude-3-5-sonnet-20241022".to_string(),
                ..Default::default()
            },
            ProfileConfig {
                name: "beta".to_string(),
                provider_type: ProviderType::DirectAnthropic,
                base_url: beta_base,
                default_model: "claude-3-5-sonnet-20241022".to_string(),
                ..Default::default()
            },
        ],
        ..Default::default()
    };

    let state = Arc::new(ProxyState {
        config: Arc::new(RwLock::new(config)),
        metrics: MetricsStore::new(),
        http_client,
        health_status: Arc::new(RwLock::new(HealthMap::new())),
        circuit_breakers: fallback::new_circuit_breaker_map(),
        shared_context: SharedContext::new(),
        rag_index: None,
        token_manager,
        forward: Some(forward_state),
    });

    let listener = bind(0).await.expect("forward bind should succeed");
    let port = listener.local_addr().expect("local_addr").port();
    spawn(listener, state);

    (port, secret, ca_pem)
}

/// PEM 形式の証明書を DER バイト列にする（ヘッダ/フッタ行を除いて base64 デコードする）
fn pem_to_der(pem: &str) -> Vec<u8> {
    let body: String = pem
        .lines()
        .filter(|line| !line.starts_with("-----"))
        .collect();
    STANDARD
        .decode(body)
        .expect("CA PEM should be valid base64")
}

/// `CONNECT api.anthropic.com:443` を送り、TLS を終端した接続を返す（plan §5 Step 2）
async fn connect_tls(port: u16, ca_pem: &str, auth: Option<&str>) -> TlsStream<TcpStream> {
    let mut stream = TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("connect to forward proxy");

    let connect_request = match auth {
        Some(auth) => {
            format!("CONNECT {TERMINATE_HOST}:443 HTTP/1.1\r\nProxy-Authorization: {auth}\r\n\r\n")
        }
        None => format!("CONNECT {TERMINATE_HOST}:443 HTTP/1.1\r\n\r\n"),
    };
    stream
        .write_all(connect_request.as_bytes())
        .await
        .expect("write CONNECT request");

    let mut reader = BufReader::new(stream);
    let mut status_line = String::new();
    reader
        .read_line(&mut status_line)
        .await
        .expect("read CONNECT status line");
    assert!(
        status_line.starts_with("HTTP/1.1 200"),
        "unexpected CONNECT response: {status_line}"
    );
    consume_headers(&mut reader).await;
    let stream = reader.into_inner();

    let der = pem_to_der(ca_pem);
    let mut root_store = RootCertStore::empty();
    root_store
        .add(CertificateDer::from(der))
        .expect("add CA cert to root store");

    let mut client_config = ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();
    client_config.alpn_protocols = vec![b"http/1.1".to_vec()];

    let connector = tokio_rustls::TlsConnector::from(Arc::new(client_config));
    let server_name =
        rustls::pki_types::ServerName::try_from(TERMINATE_HOST).expect("valid server name");

    connector
        .connect(server_name, stream)
        .await
        .expect("TLS handshake with forward proxy")
}

/// 空行が来るまでヘッダ行を読み捨てる
async fn consume_headers<R: AsyncBufRead + Unpin>(reader: &mut R) {
    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line).await.expect("read header line");
        if n == 0 || line == "\r\n" || line == "\n" {
            break;
        }
    }
}

/// HTTP/1.1 応答を1件、最後まで読む（`Content-Length` と `Transfer-Encoding: chunked` の
/// 両方を扱う）。ステータスコードと本文を返す
async fn read_http_response<R: AsyncBufRead + Unpin>(reader: &mut R) -> (u16, Vec<u8>) {
    let mut status_line = String::new();
    reader
        .read_line(&mut status_line)
        .await
        .expect("read status line");
    let status: u16 = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .unwrap_or_else(|| panic!("cannot parse status from: {status_line}"));

    let mut content_length: Option<usize> = None;
    let mut chunked = false;
    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line).await.expect("read header line");
        if n == 0 || line == "\r\n" || line == "\n" {
            break;
        }
        let lower = line.to_ascii_lowercase();
        if let Some(value) = lower.strip_prefix("content-length:") {
            content_length = value.trim().parse().ok();
        } else if lower.starts_with("transfer-encoding:") && lower.contains("chunked") {
            chunked = true;
        }
    }

    let mut body = Vec::new();
    if chunked {
        loop {
            let mut size_line = String::new();
            reader
                .read_line(&mut size_line)
                .await
                .expect("read chunk size line");
            let size_str = size_line.trim().split(';').next().unwrap_or("").trim();
            let size = usize::from_str_radix(size_str, 16).expect("parse chunk size");
            if size == 0 {
                let mut trailer = String::new();
                let _ = reader.read_line(&mut trailer).await;
                break;
            }
            let mut chunk = vec![0u8; size];
            reader
                .read_exact(&mut chunk)
                .await
                .expect("read chunk data");
            body.extend_from_slice(&chunk);
            let mut crlf = [0u8; 2];
            reader
                .read_exact(&mut crlf)
                .await
                .expect("read chunk trailing CRLF");
        }
    } else if let Some(len) = content_length {
        body.resize(len, 0);
        reader.read_exact(&mut body).await.expect("read body");
    }

    (status, body)
}

/// SSE 応答の最初のチャンクだけを読む（本文の残りを待たない）
async fn read_sse_first_chunk<R: AsyncBufRead + Unpin>(reader: &mut R) -> Vec<u8> {
    let mut status_line = String::new();
    reader
        .read_line(&mut status_line)
        .await
        .expect("read status line");
    assert!(
        status_line.starts_with("HTTP/1.1 200"),
        "unexpected status: {status_line}"
    );
    consume_headers(reader).await;

    let mut size_line = String::new();
    reader
        .read_line(&mut size_line)
        .await
        .expect("read chunk size line");
    let size = usize::from_str_radix(size_line.trim(), 16).expect("parse chunk size");
    let mut chunk = vec![0u8; size];
    reader
        .read_exact(&mut chunk)
        .await
        .expect("read chunk data");
    chunk
}

/// `/v1/messages` へ送る POST リクエストのヘッダ+リクエストラインを組み立てる
fn post_request(path_and_query: &str, body_len: usize) -> String {
    format!(
        "POST {path_and_query} HTTP/1.1\r\nHost: {TERMINATE_HOST}\r\nContent-Type: application/json\r\nContent-Length: {body_len}\r\n\r\n"
    )
}

fn basic_auth(user: &str, secret: &str) -> String {
    format!("Basic {}", STANDARD.encode(format!("{user}:{secret}")))
}

#[tokio::test]
async fn test_connect_terminates_and_passes_through() {
    let upstream = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/claude_code/settings"))
        .respond_with(ResponseTemplate::new(200).set_body_string("settings-ok"))
        .mount(&upstream)
        .await;

    let (port, _secret, ca_pem) = spawn_forward(
        upstream.uri(),
        "http://127.0.0.1:1".to_string(),
        "http://127.0.0.1:1".to_string(),
    )
    .await;

    let tls = connect_tls(port, &ca_pem, None).await;
    let mut buffered = BufReader::new(tls);
    let request =
        format!("GET /api/claude_code/settings HTTP/1.1\r\nHost: {TERMINATE_HOST}\r\n\r\n");
    buffered
        .write_all(request.as_bytes())
        .await
        .expect("write request");

    let (status, body) = read_http_response(&mut buffered).await;
    assert_eq!(status, 200);
    assert_eq!(String::from_utf8_lossy(&body), "settings-ok");

    let received = upstream
        .received_requests()
        .await
        .expect("received requests");
    assert_eq!(received.len(), 1);
    assert_eq!(received[0].url.path(), "/api/claude_code/settings");
}

#[tokio::test]
async fn test_identity_carries_across_requests_on_same_connection() {
    let generic_upstream = MockServer::start().await;
    let alpha_upstream = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"type": "message"})),
        )
        .mount(&alpha_upstream)
        .await;

    let (port, secret, ca_pem) = spawn_forward(
        generic_upstream.uri(),
        alpha_upstream.uri(),
        "http://127.0.0.1:1".to_string(),
    )
    .await;

    let auth = basic_auth("alpha", &secret);
    let tls = connect_tls(port, &ca_pem, Some(&auth)).await;
    let mut buffered = BufReader::new(tls);
    let request = post_request("/v1/messages", MESSAGES_BODY.len());

    // 1回目: CONNECT に Proxy-Authorization が付いている
    buffered
        .write_all(request.as_bytes())
        .await
        .expect("write first request head");
    buffered
        .write_all(MESSAGES_BODY)
        .await
        .expect("write first request body");
    let (status1, _) = read_http_response(&mut buffered).await;
    assert_eq!(status1, 200);

    // 2回目: 同じ TLS 接続上で、今度は Proxy-Authorization を付けずに送る
    buffered
        .write_all(request.as_bytes())
        .await
        .expect("write second request head");
    buffered
        .write_all(MESSAGES_BODY)
        .await
        .expect("write second request body");
    let (status2, _) = read_http_response(&mut buffered).await;
    assert_eq!(status2, 200);

    let received = alpha_upstream
        .received_requests()
        .await
        .expect("received requests");
    assert_eq!(received.len(), 2);
}

#[tokio::test]
async fn test_two_profiles_do_not_cross_route() {
    let generic_upstream = MockServer::start().await;
    let alpha_upstream = MockServer::start().await;
    let beta_upstream = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"type": "message", "profile": "alpha"})),
        )
        .mount(&alpha_upstream)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"type": "message", "profile": "beta"})),
        )
        .mount(&beta_upstream)
        .await;

    let (port, secret, ca_pem) = spawn_forward(
        generic_upstream.uri(),
        alpha_upstream.uri(),
        beta_upstream.uri(),
    )
    .await;

    let alpha_auth = basic_auth("alpha", &secret);
    let beta_auth = basic_auth("beta", &secret);
    let request = post_request("/v1/messages", MESSAGES_BODY.len());

    let alpha_fut = async {
        let tls = connect_tls(port, &ca_pem, Some(&alpha_auth)).await;
        let mut buffered = BufReader::new(tls);
        buffered
            .write_all(request.as_bytes())
            .await
            .expect("write alpha request head");
        buffered
            .write_all(MESSAGES_BODY)
            .await
            .expect("write alpha request body");
        read_http_response(&mut buffered).await
    };
    let beta_fut = async {
        let tls = connect_tls(port, &ca_pem, Some(&beta_auth)).await;
        let mut buffered = BufReader::new(tls);
        buffered
            .write_all(request.as_bytes())
            .await
            .expect("write beta request head");
        buffered
            .write_all(MESSAGES_BODY)
            .await
            .expect("write beta request body");
        read_http_response(&mut buffered).await
    };

    let ((alpha_status, _), (beta_status, _)) = tokio::join!(alpha_fut, beta_fut);
    assert_eq!(alpha_status, 200);
    assert_eq!(beta_status, 200);

    let alpha_received = alpha_upstream
        .received_requests()
        .await
        .expect("alpha received requests");
    let beta_received = beta_upstream
        .received_requests()
        .await
        .expect("beta received requests");
    assert_eq!(alpha_received.len(), 1);
    assert_eq!(beta_received.len(), 1);
}

#[tokio::test]
async fn test_inference_without_credentials_goes_upstream() {
    let upstream = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"type": "message"})),
        )
        .mount(&upstream)
        .await;

    let (port, _secret, ca_pem) = spawn_forward(
        upstream.uri(),
        "http://127.0.0.1:1".to_string(),
        "http://127.0.0.1:1".to_string(),
    )
    .await;

    let tls = connect_tls(port, &ca_pem, None).await;
    let mut buffered = BufReader::new(tls);
    let request = post_request("/v1/messages", MESSAGES_BODY.len());
    buffered
        .write_all(request.as_bytes())
        .await
        .expect("write request head");
    buffered
        .write_all(MESSAGES_BODY)
        .await
        .expect("write request body");

    let (status, _) = read_http_response(&mut buffered).await;
    assert_eq!(status, 200);

    let received = upstream
        .received_requests()
        .await
        .expect("received requests");
    assert_eq!(received.len(), 1);
    assert_eq!(received[0].url.path(), "/v1/messages");
}

#[tokio::test]
async fn test_inference_with_wrong_secret_returns_502() {
    let generic_upstream = MockServer::start().await;
    let alpha_upstream = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&alpha_upstream)
        .await;

    let (port, secret, ca_pem) = spawn_forward(
        generic_upstream.uri(),
        alpha_upstream.uri(),
        "http://127.0.0.1:1".to_string(),
    )
    .await;

    // 合言葉の先頭1文字だけを変える。元の文字が何であれ必ず異なる文字になる
    let mut wrong_secret = secret.clone();
    let first = wrong_secret
        .chars()
        .next()
        .expect("secret should not be empty");
    let replacement = if first == 'A' { 'B' } else { 'A' };
    wrong_secret.replace_range(0..1, &replacement.to_string());

    let auth = basic_auth("alpha", &wrong_secret);
    let tls = connect_tls(port, &ca_pem, Some(&auth)).await;
    let mut buffered = BufReader::new(tls);
    let request = post_request("/v1/messages", MESSAGES_BODY.len());
    buffered
        .write_all(request.as_bytes())
        .await
        .expect("write request head");
    buffered
        .write_all(MESSAGES_BODY)
        .await
        .expect("write request body");

    let (status, _) = read_http_response(&mut buffered).await;
    assert_eq!(status, 502);

    let received = alpha_upstream
        .received_requests()
        .await
        .expect("received requests");
    assert!(received.is_empty());
}

#[tokio::test]
async fn test_count_tokens_never_reaches_upstream() {
    let generic_upstream = MockServer::start().await;
    let alpha_upstream = MockServer::start().await;

    let (port, _secret, ca_pem) = spawn_forward(
        generic_upstream.uri(),
        alpha_upstream.uri(),
        "http://127.0.0.1:1".to_string(),
    )
    .await;

    let tls = connect_tls(port, &ca_pem, None).await;
    let mut buffered = BufReader::new(tls);
    let body: &[u8] =
        br#"{"model":"claude-3-5-sonnet-20241022","messages":[{"role":"user","content":"hi"}]}"#;
    let request = post_request("/v1/messages/count_tokens", body.len());
    buffered
        .write_all(request.as_bytes())
        .await
        .expect("write request head");
    buffered.write_all(body).await.expect("write request body");

    let (status, response_body) = read_http_response(&mut buffered).await;
    assert_eq!(status, 200);
    let parsed: serde_json::Value =
        serde_json::from_slice(&response_body).expect("valid JSON body");
    assert!(parsed.get("input_tokens").is_some());

    let generic_received = generic_upstream
        .received_requests()
        .await
        .expect("generic received requests");
    let alpha_received = alpha_upstream
        .received_requests()
        .await
        .expect("alpha received requests");
    assert!(generic_received.is_empty());
    assert!(alpha_received.is_empty());
}

#[tokio::test]
async fn test_unknown_path_relays_upstream() {
    let upstream = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/organizations"))
        .respond_with(ResponseTemplate::new(200).set_body_string("orgs-ok"))
        .mount(&upstream)
        .await;

    let (port, _secret, ca_pem) = spawn_forward(
        upstream.uri(),
        "http://127.0.0.1:1".to_string(),
        "http://127.0.0.1:1".to_string(),
    )
    .await;

    let tls = connect_tls(port, &ca_pem, None).await;
    let mut buffered = BufReader::new(tls);
    let request = format!("GET /v1/organizations HTTP/1.1\r\nHost: {TERMINATE_HOST}\r\n\r\n");
    buffered
        .write_all(request.as_bytes())
        .await
        .expect("write request");

    let (status, response_body) = read_http_response(&mut buffered).await;
    assert_eq!(status, 200);
    assert_eq!(String::from_utf8_lossy(&response_body), "orgs-ok");

    let received = upstream
        .received_requests()
        .await
        .expect("received requests");
    assert_eq!(received.len(), 1);
}

/// 生の TCP エコーサーバを立てる（plan §3 で手書き `TcpListener` が明示的に許可されている）
async fn spawn_echo_server() -> u16 {
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("bind echo server");
    let port = listener.local_addr().expect("local_addr").port();
    tokio::spawn(async move {
        if let Ok((mut socket, _)) = listener.accept().await {
            let (mut rd, mut wr) = socket.split();
            let _ = tokio::io::copy(&mut rd, &mut wr).await;
        }
    });
    port
}

#[tokio::test]
async fn test_non_terminated_connect_tunnels_bytes() {
    let upstream = MockServer::start().await;
    let echo_port = spawn_echo_server().await;

    let (port, _secret, _ca_pem) = spawn_forward(
        upstream.uri(),
        "http://127.0.0.1:1".to_string(),
        "http://127.0.0.1:1".to_string(),
    )
    .await;

    let mut stream = TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("connect to forward proxy");
    let connect_request = format!("CONNECT 127.0.0.1:{echo_port} HTTP/1.1\r\n\r\n");
    stream
        .write_all(connect_request.as_bytes())
        .await
        .expect("write CONNECT request");

    let mut reader = BufReader::new(stream);
    let mut status_line = String::new();
    reader
        .read_line(&mut status_line)
        .await
        .expect("read CONNECT status line");
    assert!(
        status_line.starts_with("HTTP/1.1 200"),
        "unexpected CONNECT response: {status_line}"
    );
    consume_headers(&mut reader).await;
    let mut stream = reader.into_inner();

    let payload = b"hello-tunnel";
    stream.write_all(payload).await.expect("write payload");

    let mut echoed = vec![0u8; payload.len()];
    stream
        .read_exact(&mut echoed)
        .await
        .expect("read echoed payload");
    assert_eq!(&echoed, payload);
}

/// `api.anthropic.com` を名乗る TLS サーバを1接続だけ受けて、素朴な HTTP/1.1 応答を返す
///
/// `route::relay_upstream` は絶対 URI 形式の要求を常に `https://{authority}` へ中継するため、
/// plain HTTP の `wiremock` では受けられない。`forward::tls::build_tls()`（`TERMINATE_HOST`
/// 向けの証明書を作る既存のヘルパ）を再利用し、クライアント側は `danger_accept_invalid_certs`
/// で検証を省く（本物の CA には署名させていないため）。
async fn spawn_fake_anthropic_tls_upstream() -> u16 {
    let generated = build_tls().expect("build_tls for fake upstream");
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("bind fake upstream");
    let port = listener.local_addr().expect("local_addr").port();
    tokio::spawn(async move {
        let (socket, _) = listener.accept().await.expect("accept fake upstream conn");
        let tls = generated
            .acceptor
            .accept(socket)
            .await
            .expect("tls accept on fake upstream");
        let mut reader = BufReader::new(tls);
        consume_headers(&mut reader).await;
        let mut tls = reader.into_inner();

        let body = b"fake-upstream-ok";
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n\r\n",
            body.len()
        );
        tls.write_all(response.as_bytes())
            .await
            .expect("write fake upstream response head");
        tls.write_all(body)
            .await
            .expect("write fake upstream response body");
    });
    port
}

#[tokio::test]
async fn test_absolute_uri_form_is_accepted() {
    // `spawn_forward` を先に呼び、rustls のプロセス既定プロバイダを確実に立ててから
    // `build_tls()` を呼ぶ（`ForwardState::new` がその役目を担っている）。
    let generic_upstream = MockServer::start().await;
    let (port, _secret, _ca_pem) = spawn_forward(
        generic_upstream.uri(),
        "http://127.0.0.1:1".to_string(),
        "http://127.0.0.1:1".to_string(),
    )
    .await;
    let fake_port = spawn_fake_anthropic_tls_upstream().await;

    let mut stream = TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("connect to forward proxy");
    let request = format!(
        "GET http://{TERMINATE_HOST}:{fake_port}/api/claude_code/settings HTTP/1.1\r\nHost: {TERMINATE_HOST}:{fake_port}\r\n\r\n"
    );
    stream
        .write_all(request.as_bytes())
        .await
        .expect("write absolute-uri request");

    let mut reader = BufReader::new(stream);
    let (status, body) = read_http_response(&mut reader).await;

    assert_ne!(
        status, 405,
        "absolute-URI form request must not be rejected with 405 (FR-005)"
    );
    assert_eq!(status, 200);
    assert_eq!(String::from_utf8_lossy(&body), "fake-upstream-ok");

    let received = generic_upstream
        .received_requests()
        .await
        .expect("generic upstream received requests");
    assert!(
        received.is_empty(),
        "absolute-URI requests targeting api.anthropic.com must not fall back to upstream_base"
    );
}

/// SSE 応答を保持したまま閉じない上流を立てる（plan §3 で手書き `TcpListener` が明示されている）
async fn spawn_sse_upstream() -> u16 {
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("bind sse upstream");
    let port = listener.local_addr().expect("local_addr").port();
    tokio::spawn(async move {
        let (socket, _) = listener.accept().await.expect("accept sse upstream conn");
        let mut reader = BufReader::new(socket);
        consume_headers(&mut reader).await;
        let mut socket = reader.into_inner();

        let chunk: &[u8] = b"event: ping\ndata: {}\n\n";
        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n",
            chunk.len()
        );
        socket
            .write_all(head.as_bytes())
            .await
            .expect("write sse head");
        socket.write_all(chunk).await.expect("write sse chunk");
        socket
            .write_all(b"\r\n")
            .await
            .expect("write sse chunk terminator");
        // 応答を完結させず（終端チャンクを送らず）接続を保持する
        tokio::time::sleep(Duration::from_secs(5)).await;
    });
    port
}

#[tokio::test]
async fn test_sse_first_chunk_arrives_before_upstream_finishes() {
    let generic_upstream = MockServer::start().await;
    let sse_port = spawn_sse_upstream().await;

    let (port, secret, ca_pem) = spawn_forward(
        generic_upstream.uri(),
        format!("http://127.0.0.1:{sse_port}"),
        "http://127.0.0.1:1".to_string(),
    )
    .await;

    let auth = basic_auth("alpha", &secret);
    let tls = connect_tls(port, &ca_pem, Some(&auth)).await;
    let mut buffered = BufReader::new(tls);
    let request = post_request("/v1/messages", STREAMING_MESSAGES_BODY.len());
    buffered
        .write_all(request.as_bytes())
        .await
        .expect("write request head");
    buffered
        .write_all(STREAMING_MESSAGES_BODY)
        .await
        .expect("write request body");

    let first_chunk =
        tokio::time::timeout(Duration::from_secs(2), read_sse_first_chunk(&mut buffered))
            .await
            .expect("first SSE chunk should arrive within 2 seconds");

    assert!(
        String::from_utf8_lossy(&first_chunk).contains("ping"),
        "unexpected first chunk: {:?}",
        String::from_utf8_lossy(&first_chunk)
    );
}
