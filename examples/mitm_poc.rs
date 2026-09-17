//! PoC Step B: api.anthropic.com だけ TLS を終端する forward proxy
//!
//! Claude Code には `HTTPS_PROXY` でこのプロキシを、`NODE_EXTRA_CA_CERTS` で
//! 起動時に生成した CA を渡す。api.anthropic.com への CONNECT だけ TLS を終端して
//! リクエストの method と path を観測し、そのまま本物の api.anthropic.com へ中継する。
//! 他のホストは終端せず素通しする。
//!
//! 使い方:
//!   cargo run --example mitm_poc -- 18080
//!   HTTPS_PROXY=http://127.0.0.1:18080 NODE_EXTRA_CA_CERTS=<表示されたパス> claude doctor

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper::{Request, Response};
use hyper_util::rt::TokioIo;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio_rustls::rustls::ServerConfig;
use tokio_rustls::TlsAcceptor;

const MITM_HOST: &str = "api.anthropic.com";

type Counts = Arc<Mutex<HashMap<String, usize>>>;

struct Ctx {
    acceptor: TlsAcceptor,
    client: reqwest::Client,
    counts: Counts,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let port: u16 = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(18080);

    // rustls の既定プロバイダを立てる（reqwest と同居させるため明示する）
    let _ = tokio_rustls::rustls::crypto::aws_lc_rs::default_provider().install_default();

    let (ca_pem_path, acceptor) = build_tls()?;
    eprintln!("mitm_poc listening on http://127.0.0.1:{port}");
    eprintln!("NODE_EXTRA_CA_CERTS={}", ca_pem_path.display());

    let ctx = Arc::new(Ctx {
        acceptor,
        // 自分自身がプロキシなので、上流への接続で HTTPS_PROXY を拾わせない
        client: reqwest::Client::builder().no_proxy().build()?,
        counts: Arc::new(Mutex::new(HashMap::new())),
    });

    {
        let counts = Arc::clone(&ctx.counts);
        tokio::spawn(async move {
            let _ = tokio::signal::ctrl_c().await;
            let c = counts.lock().unwrap();
            let mut rows: Vec<_> = c.iter().collect();
            rows.sort_by(|a, b| b.1.cmp(a.1));
            eprintln!("\n--- request summary ---");
            for (k, n) in rows {
                eprintln!("{n:>5}  {k}");
            }
            std::process::exit(0);
        });
    }

    let listener = TcpListener::bind(("127.0.0.1", port)).await?;
    loop {
        let (client, _) = listener.accept().await?;
        let ctx = Arc::clone(&ctx);
        tokio::spawn(async move {
            if let Err(e) = handle_conn(client, ctx).await {
                eprintln!("[err] {e}");
            }
        });
    }
}

/// CA とリーフ証明書を作り、CA の PEM をファイルに書く
fn build_tls() -> anyhow::Result<(std::path::PathBuf, TlsAcceptor)> {
    use rcgen::{
        BasicConstraints, CertificateParams, DnType, IsCa, Issuer, KeyPair, KeyUsagePurpose,
        SanType,
    };

    let mut ca_params = CertificateParams::default();
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca_params
        .distinguished_name
        .push(DnType::CommonName, "claudex local CA (PoC)");
    ca_params.key_usages = vec![
        KeyUsagePurpose::KeyCertSign,
        KeyUsagePurpose::CrlSign,
        KeyUsagePurpose::DigitalSignature,
    ];
    let ca_key = KeyPair::generate()?;
    let ca_cert = ca_params.self_signed(&ca_key)?;

    let mut leaf_params = CertificateParams::default();
    leaf_params
        .distinguished_name
        .push(DnType::CommonName, MITM_HOST);
    leaf_params.subject_alt_names = vec![SanType::DnsName(MITM_HOST.try_into()?)];
    let leaf_key = KeyPair::generate()?;
    let leaf_cert = leaf_params.signed_by(&leaf_key, &Issuer::from_params(&ca_params, &ca_key))?;

    let ca_pem_path = std::env::temp_dir().join("claudex-poc-ca.pem");
    std::fs::write(&ca_pem_path, ca_cert.pem())?;

    let chain = vec![
        CertificateDer::from(leaf_cert.der().to_vec()),
        CertificateDer::from(ca_cert.der().to_vec()),
    ];
    let key = PrivateKeyDer::try_from(leaf_key.serialize_der())
        .map_err(|e| anyhow::anyhow!("private key: {e}"))?;

    let mut config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(chain, key)?;
    // h2 は名乗らず HTTP/1.1 に寄せる
    config.alpn_protocols = vec![b"http/1.1".to_vec()];

    Ok((ca_pem_path, TlsAcceptor::from(Arc::new(config))))
}

async fn handle_conn(client: TcpStream, ctx: Arc<Ctx>) -> anyhow::Result<()> {
    // CONNECT かどうかを、バイトを消費せずに覗いて決める。
    // Remote Control のブリッジは CONNECT を使わず、絶対 URI 形式の要求を
    // そのままプロキシへ投げてくる（POST https://api.anthropic.com/... HTTP/1.1）。
    let mut head = [0u8; 8];
    let n = client.peek(&mut head).await?;
    if n == 0 {
        return Ok(());
    }
    if !head[..n].starts_with(b"CONNECT") {
        return serve_http(client, ctx).await;
    }

    let mut reader = BufReader::new(client);

    let mut request_line = String::new();
    if reader.read_line(&mut request_line).await? == 0 {
        return Ok(());
    }
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).await? == 0 || line == "\r\n" || line == "\n" {
            break;
        }
    }

    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let target = parts.next().unwrap_or_default().to_string();

    if !method.eq_ignore_ascii_case("CONNECT") {
        eprintln!("[non-connect] {}", request_line.trim_end());
        let mut client = reader.into_inner();
        client
            .write_all(b"HTTP/1.1 405 Method Not Allowed\r\nContent-Length: 0\r\n\r\n")
            .await?;
        return Ok(());
    }
    let pending = reader.buffer().to_vec();
    let mut client = reader.into_inner();
    client
        .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
        .await?;

    if target.split(':').next() == Some(MITM_HOST) {
        eprintln!("[mitm] {target}");
        terminate_tls(client, ctx).await
    } else {
        eprintln!("[tunnel] {target}");
        let upstream = TcpStream::connect(&target).await?;
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
}

async fn terminate_tls(client: TcpStream, ctx: Arc<Ctx>) -> anyhow::Result<()> {
    let tls = ctx.acceptor.accept(client).await?;
    serve(TokioIo::new(tls), ctx).await
}

/// 絶対 URI 形式で直接投げられた要求を、平文のまま受けて中継する
async fn serve_http(client: TcpStream, ctx: Arc<Ctx>) -> anyhow::Result<()> {
    serve(TokioIo::new(client), ctx).await
}

async fn serve<I>(io: I, ctx: Arc<Ctx>) -> anyhow::Result<()>
where
    I: hyper::rt::Read + hyper::rt::Write + Unpin + Send + 'static,
{
    let service = service_fn(move |req: Request<Incoming>| {
        let ctx = Arc::clone(&ctx);
        eprintln!(
            "[req] {} {} {:?} upgrade={:?}",
            req.method(),
            req.uri(),
            req.version(),
            req.headers().get("upgrade")
        );
        async move { relay(req, ctx).await }
    });

    if let Err(e) = hyper::server::conn::http1::Builder::new()
        .serve_connection(io, service)
        .await
    {
        eprintln!("[err] serve: {e}");
    }
    Ok(())
}

/// 終端したリクエストを本物の api.anthropic.com へそのまま中継する
async fn relay(
    req: Request<Incoming>,
    ctx: Arc<Ctx>,
) -> Result<Response<reqwest::Body>, hyper::Error> {
    use http_body_util::BodyExt;

    let method = req.method().clone();
    let uri = req.uri().clone();
    let path = uri
        .path_and_query()
        .map(|p| p.as_str().to_string())
        .unwrap_or_else(|| "/".into());
    // 絶対 URI で来たときはその scheme と host を使い、
    // TLS 終端側から来たときは api.anthropic.com とみなす
    let url = match uri.authority() {
        Some(authority) => format!(
            "{}://{authority}{path}",
            uri.scheme_str().unwrap_or("https")
        ),
        None => format!("https://{MITM_HOST}{path}"),
    };
    let headers = req.headers().clone();
    let body = req.into_body().collect().await?.to_bytes();

    {
        let mut c = ctx.counts.lock().unwrap();
        let key = format!("{method} {}", path.split('?').next().unwrap_or(&path));
        *c.entry(key).or_insert(0) += 1;
    }

    let mut upstream = ctx.client.request(method.clone(), &url);
    for (name, value) in headers.iter() {
        // host はクライアント側の接続に紐づくので付け替えさせる
        if name.as_str().eq_ignore_ascii_case("host") {
            continue;
        }
        upstream = upstream.header(name, value);
    }
    if !body.is_empty() {
        upstream = upstream.body(body);
    }

    let resp = match upstream.send().await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[err] upstream {method} {path}: {e}");
            let mut r = Response::new(reqwest::Body::from(format!("upstream error: {e}")));
            *r.status_mut() = hyper::StatusCode::BAD_GATEWAY;
            return Ok(r);
        }
    };

    eprintln!("[relay] {} {method} {path}", resp.status());

    let mut out = Response::builder().status(resp.status());
    for (name, value) in resp.headers().iter() {
        // 本文は reqwest 側でデコード済みなので、転送エンコーディングは持ち越さない
        if matches!(
            name.as_str(),
            "transfer-encoding" | "content-length" | "connection"
        ) {
            continue;
        }
        out = out.header(name, value);
    }
    Ok(out
        .body(reqwest::Body::wrap_stream(resp.bytes_stream()))
        .expect("response build"))
}
