//! 私設 CA とリーフ証明書を生成し、`api.anthropic.com` を終端する TLS acceptor を作る
//!
//! `examples/mitm_poc.rs` の `build_tls()` を土台にしているが、以下が異なる:
//! - CA の CommonName は `"claudex local CA"`（PoC の `(PoC)` は落とす）
//! - リーフ証明書に有効期間（当日〜30日後）を設定する
//! - CA PEM をファイルへ書かず、戻り値の文字列として返す（FR-002: 秘密鍵はディスクへ書かない）

use std::sync::Arc;

use anyhow::Context;
use chrono::Datelike;
use rcgen::{
    date_time_ymd, BasicConstraints, CertificateParams, DnType, IsCa, Issuer, KeyPair,
    KeyUsagePurpose, SanType,
};
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio_rustls::rustls::ServerConfig;
use tokio_rustls::TlsAcceptor;

use super::TERMINATE_HOST;

/// 生成された TLS acceptor と、クライアントに信頼させる CA 証明書の PEM
pub struct GeneratedTls {
    pub acceptor: TlsAcceptor,
    pub ca_pem: String,
}

/// 私設 CA とリーフ証明書を生成し、`TERMINATE_HOST` 用の TLS acceptor を組み立てる
///
/// CA・リーフいずれの `KeyPair` もこの関数のスコープを出ない（FR-002）。
pub fn build_tls() -> anyhow::Result<GeneratedTls> {
    let mut ca_params = CertificateParams::default();
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca_params
        .distinguished_name
        .push(DnType::CommonName, "claudex local CA");
    ca_params.key_usages = vec![
        KeyUsagePurpose::KeyCertSign,
        KeyUsagePurpose::CrlSign,
        KeyUsagePurpose::DigitalSignature,
    ];
    let ca_key = KeyPair::generate()?;
    let ca_cert = ca_params.self_signed(&ca_key)?;

    let not_before = chrono::Utc::now();
    let not_after = not_before + chrono::Duration::days(30);

    let mut leaf_params = CertificateParams::default();
    leaf_params
        .distinguished_name
        .push(DnType::CommonName, TERMINATE_HOST);
    leaf_params.subject_alt_names = vec![SanType::DnsName(TERMINATE_HOST.try_into()?)];
    leaf_params.not_before = date_time_ymd(
        not_before.year(),
        not_before.month() as u8,
        not_before.day() as u8,
    );
    leaf_params.not_after = date_time_ymd(
        not_after.year(),
        not_after.month() as u8,
        not_after.day() as u8,
    );
    let leaf_key = KeyPair::generate()?;
    let leaf_cert = leaf_params.signed_by(&leaf_key, &Issuer::from_params(&ca_params, &ca_key))?;

    let ca_pem = ca_cert.pem();

    let chain = vec![
        CertificateDer::from(leaf_cert.der().to_vec()),
        CertificateDer::from(ca_cert.der().to_vec()),
    ];
    let key = PrivateKeyDer::try_from(leaf_key.serialize_der())
        .map_err(|e| anyhow::anyhow!("private key: {e}"))?;

    let mut config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(chain, key)
        .context("cannot build TLS server config")?;
    // h2 は名乗らず HTTP/1.1 に寄せる
    config.alpn_protocols = vec![b"http/1.1".to_vec()];

    Ok(GeneratedTls {
        acceptor: TlsAcceptor::from(Arc::new(config)),
        ca_pem,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_tls_returns_ca_pem() {
        // rustls のプロセス既定プロバイダは ForwardState::new() が立てる前提。
        // このテストは build_tls() を単独で呼ぶので、ここで明示的に立てる
        // （二重呼び出しは Err を返すだけなので無視してよい）。
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

        let tls = build_tls().expect("build_tls should succeed");
        assert!(tls.ca_pem.starts_with("-----BEGIN CERTIFICATE-----"));
    }
}
