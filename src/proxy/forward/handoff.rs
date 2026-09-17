//! proxy と `claudex run` の間で forward proxy の接続情報を受け渡すファイル
//!
//! CA 証明書の PEM と、待受ポート・合言葉を書いた JSON をそれぞれ runtime ディレクトリに書く。
//! `claudex run` はこれを読んで子プロセスへ環境変数として渡す。

use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// proxy から `claudex run` へ渡す forward proxy の接続情報
#[derive(Serialize, Deserialize)]
pub struct ForwardHandoff {
    pub port: u16,
    pub secret: String,
}

/// CA 証明書 PEM の受け渡しファイルパス
pub fn ca_pem_path() -> Result<PathBuf> {
    Ok(crate::process::daemon::runtime_dir()?.join("forward-ca.pem"))
}

/// 接続情報 JSON の受け渡しファイルパス
pub fn handoff_path() -> Result<PathBuf> {
    Ok(crate::process::daemon::runtime_dir()?.join("forward.json"))
}

/// CA PEM と接続情報 JSON をそれぞれ受け渡しファイルへ書く
pub fn write(handoff: &ForwardHandoff, ca_pem: &str) -> Result<()> {
    let ca_path = ca_pem_path()?;
    std::fs::write(&ca_path, ca_pem)
        .with_context(|| format!("cannot write CA pem to {}", ca_path.display()))?;

    let handoff_path = handoff_path()?;
    let json = serde_json::to_string(handoff)?;
    std::fs::write(&handoff_path, json)
        .with_context(|| format!("cannot write handoff to {}", handoff_path.display()))?;

    Ok(())
}

/// 接続情報 JSON を読む
pub fn read() -> Result<ForwardHandoff> {
    let path = handoff_path()?;
    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("cannot read handoff from {}", path.display()))?;
    let handoff = serde_json::from_str(&content)
        .with_context(|| format!("cannot parse handoff json at {}", path.display()))?;
    Ok(handoff)
}

/// CA PEM と接続情報 JSON を best-effort で削除する。存在しなくてもエラーにしない
pub fn cleanup() {
    if let Ok(path) = ca_pem_path() {
        if let Err(e) = std::fs::remove_file(&path) {
            tracing::debug!(path = %path.display(), "cannot remove forward CA pem: {e}");
        }
    }
    if let Ok(path) = handoff_path() {
        if let Err(e) = std::fs::remove_file(&path) {
            tracing::debug!(path = %path.display(), "cannot remove forward handoff: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_handoff_roundtrip() {
        let handoff = ForwardHandoff {
            port: 13457,
            secret: "abc".into(),
        };
        let json = serde_json::to_string(&handoff).expect("serialize");
        let roundtripped: ForwardHandoff = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(roundtripped.port, handoff.port);
        assert_eq!(roundtripped.secret, handoff.secret);
    }
}
