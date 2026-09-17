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

/// 1ファイルを削除する。存在しなかった場合（ENOENT）は成功扱いにする
fn remove_if_present(path: &std::path::Path) -> std::io::Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// CA PEM と接続情報 JSON を削除する。
///
/// 戻り値は**消し残したファイルのパス**である。空なら2ファイルともディスク上に無い
/// （もともと無かった場合を含む）。呼び出し元はこれを見て「消えた」と断言してよいかを
/// 決める。パス自体（`ca_pem_path()` / `handoff_path()`）が取得できなかった場合は
/// そのファイルがディスク上のどこにあるか分からないため消し残りリストには入れられない。
/// その場合は `tracing::warn!` のみ残す。
pub fn cleanup() -> Vec<std::path::PathBuf> {
    let mut leftover = Vec::new();

    match ca_pem_path() {
        Ok(path) => {
            if let Err(e) = remove_if_present(&path) {
                tracing::warn!(path = %path.display(), "cannot remove forward CA pem: {e}");
                leftover.push(path);
            }
        }
        Err(e) => {
            tracing::warn!("cannot determine forward CA pem path to clean up: {e}");
        }
    }

    match handoff_path() {
        Ok(path) => {
            if let Err(e) = remove_if_present(&path) {
                tracing::warn!(path = %path.display(), "cannot remove forward handoff: {e}");
                leftover.push(path);
            }
        }
        Err(e) => {
            tracing::warn!("cannot determine forward handoff path to clean up: {e}");
        }
    }

    leftover
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

    #[test]
    fn test_remove_if_present_deletes_existing() {
        let file = tempfile::NamedTempFile::new().expect("create temp file");
        let path = file.path().to_path_buf();
        assert!(path.exists());

        let result = remove_if_present(&path);

        assert!(result.is_ok());
        assert!(!path.exists());
    }

    #[test]
    fn test_remove_if_present_ok_when_absent() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let path = dir.path().join("does-not-exist.pem");
        assert!(!path.exists());

        let result = remove_if_present(&path);

        assert!(result.is_ok());
    }
}
