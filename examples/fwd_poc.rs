//! PoC Step A: CONNECT を素通しするだけの forward proxy
//!
//! 目的は二つ。
//! 1. `HTTPS_PROXY` 経由でも Claude Code の Remote Control ゲートが通ることの確認
//! 2. Claude Code がどのホストへ何回繋ぐかの観測（Step B で終端範囲を決めるため）
//!
//! 使い方:
//!   cargo run --example fwd_poc -- 18080
//!   HTTPS_PROXY=http://127.0.0.1:18080 claude doctor

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};

type Counts = Arc<Mutex<HashMap<String, usize>>>;

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let port: u16 = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(18080);

    let listener = TcpListener::bind(("127.0.0.1", port)).await?;
    eprintln!("fwd_poc listening on http://127.0.0.1:{port}");

    let counts: Counts = Arc::new(Mutex::new(HashMap::new()));

    // Ctrl-C で観測結果を出して終了する
    {
        let counts = Arc::clone(&counts);
        tokio::spawn(async move {
            let _ = tokio::signal::ctrl_c().await;
            print_summary(&counts);
            std::process::exit(0);
        });
    }

    loop {
        let (client, _) = listener.accept().await?;
        let counts = Arc::clone(&counts);
        tokio::spawn(async move {
            if let Err(e) = handle(client, counts).await {
                eprintln!("[err] {e}");
            }
        });
    }
}

async fn handle(client: TcpStream, counts: Counts) -> std::io::Result<()> {
    let mut reader = BufReader::new(client);

    let mut request_line = String::new();
    if reader.read_line(&mut request_line).await? == 0 {
        return Ok(());
    }

    // ヘッダを読み捨てる（CONNECT の本文はトンネル開始後に流れる）
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).await? == 0 {
            break;
        }
        if line == "\r\n" || line == "\n" {
            break;
        }
    }

    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default();
    let target = parts.next().unwrap_or_default().to_string();

    if !method.eq_ignore_ascii_case("CONNECT") {
        eprintln!("[skip] non-CONNECT request: {}", request_line.trim_end());
        let mut client = reader.into_inner();
        client
            .write_all(b"HTTP/1.1 405 Method Not Allowed\r\nContent-Length: 0\r\n\r\n")
            .await?;
        return Ok(());
    }

    {
        let mut c = counts.lock().unwrap();
        *c.entry(target.clone()).or_insert(0) += 1;
    }
    eprintln!("[connect] {target}");

    let upstream = match TcpStream::connect(&target).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[err] upstream {target}: {e}");
            let mut client = reader.into_inner();
            client
                .write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\n\r\n")
                .await?;
            return Ok(());
        }
    };

    // ヘッダ読み込みで先読みしてしまった分（通常は空）を取りこぼさない
    let pending = reader.buffer().to_vec();

    let mut client = reader.into_inner();
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

fn print_summary(counts: &Counts) {
    let c = counts.lock().unwrap();
    let mut rows: Vec<_> = c.iter().collect();
    rows.sort_by(|a, b| b.1.cmp(a.1));
    eprintln!("\n--- CONNECT summary ---");
    for (host, n) in rows {
        eprintln!("{n:>5}  {host}");
    }
}
