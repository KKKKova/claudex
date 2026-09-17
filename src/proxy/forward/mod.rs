//! Remote Control 用 forward proxy（`api.anthropic.com` の TLS を終端する MITM プロキシ）の土台
//!
//! `ForwardState` は TLS acceptor・合言葉・上流への reqwest クライアントを束ねる。
//! CONNECT の受付・中継処理自体は T006 で実装する。

pub mod handoff;
pub mod identity;
pub mod tls;

use std::time::Duration;

use anyhow::Result;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;

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
