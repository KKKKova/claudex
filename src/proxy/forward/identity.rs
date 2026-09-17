//! forward proxy が受け取った `Proxy-Authorization` から利用者を決める
//!
//! `claudex run` は `HTTPS_PROXY` の userinfo へ「プロファイル名 + 合言葉」を埋めて子プロセスへ渡す。
//! ここはその往路を逆に辿り、どのプロファイル（= どの provider と鍵）へ流すかを決める唯一の判断点である。
//! 解析や照合が少しでも崩れたら解決済みに倒さず、`Absent` / `Unverified` / `Unresolved` のまま返す。

use crate::proxy::ProxyState;
use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;

/// `Basic ` の認証スキーム（末尾の空白まで含む）
const BASIC_PREFIX: &str = "Basic ";

/// `Proxy-Authorization` の解析結果
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Identity {
    /// Proxy-Authorization ヘッダが無い、または解析できない
    Absent,
    /// 合言葉が一致しない
    Unverified,
    /// 合言葉は一致するが、その名前のプロファイルが config に無い
    Unresolved(String),
    /// プロファイル名の解決に成功した
    Resolved(String),
}

/// `Basic <base64>` を利用者名と合言葉へ割る。利用者名はパーセントデコードする
///
/// 合言葉側はデコードしない。`ForwardState::secret` は URL-safe base64 であり `%` を含まず、
/// `url` クレートが userinfo をエスケープする文字も含まないため、素の比較で往復が一致する。
pub fn parse_proxy_authorization(header: Option<&str>) -> Option<(String, String)> {
    let header = header?;

    // 認証スキームは大文字小文字を区別しない（RFC 9110）。
    // get(..n) は長さ不足でも文字境界外でも None を返すので、続く slice は安全である。
    let prefix = header.get(..BASIC_PREFIX.len())?;
    if !prefix.eq_ignore_ascii_case(BASIC_PREFIX) {
        return None;
    }

    let decoded = STANDARD.decode(&header[BASIC_PREFIX.len()..]).ok()?;
    let decoded = String::from_utf8(decoded).ok()?;

    // 合言葉に `:` が混ざっても壊れないよう、最初の `:` だけで割る
    let (user, secret) = decoded.split_once(':')?;
    let user = urlencoding::decode(user).ok()?;

    Some((user.into_owned(), secret.to_string()))
}

/// 解析結果を config と照合して Identity にする
pub async fn resolve(state: &ProxyState, header: Option<&str>) -> Identity {
    let Some(forward) = state.forward.as_ref() else {
        return Identity::Absent;
    };
    let Some((user, secret)) = parse_proxy_authorization(header) else {
        return Identity::Absent;
    };

    if secret != forward.secret {
        // ループバック限定かつ 256 ビット乱数なので、定数時間比較は要らない
        tracing::warn!(user = %user, "forward proxy: secret mismatch");
        return Identity::Unverified;
    }

    let config = state.config.read().await;
    if config.profiles.iter().any(|p| p.name == user) {
        tracing::debug!(user = %user, "forward proxy: profile resolved");
        Identity::Resolved(user)
    } else {
        tracing::warn!(user = %user, "forward proxy: no such profile");
        Identity::Unresolved(user)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `Basic ` を付けた認証値を組み立てる
    fn basic(raw: &[u8]) -> String {
        format!("{BASIC_PREFIX}{}", STANDARD.encode(raw))
    }

    #[test]
    fn test_parse_absent() {
        assert_eq!(parse_proxy_authorization(None), None);
    }

    #[test]
    fn test_parse_not_basic() {
        assert_eq!(parse_proxy_authorization(Some("Bearer xyz")), None);
    }

    #[test]
    fn test_parse_invalid_base64() {
        assert_eq!(parse_proxy_authorization(Some("Basic !!!!")), None);
    }

    #[test]
    fn test_parse_plain() {
        let header = basic(b"grok:tokenvalue");
        assert_eq!(
            parse_proxy_authorization(Some(&header)),
            Some(("grok".to_string(), "tokenvalue".to_string()))
        );
    }

    #[test]
    fn test_parse_secret_contains_colon() {
        let header = basic(b"grok:a:b:c");
        assert_eq!(
            parse_proxy_authorization(Some(&header)),
            Some(("grok".to_string(), "a:b:c".to_string()))
        );
    }

    #[test]
    fn test_parse_percent_encoded_user() {
        let header = basic(b"my%3Aprofile:tok");
        assert_eq!(
            parse_proxy_authorization(Some(&header)),
            Some(("my:profile".to_string(), "tok".to_string()))
        );
    }

    #[test]
    fn test_parse_no_colon() {
        let header = basic(b"grokonly");
        assert_eq!(parse_proxy_authorization(Some(&header)), None);
    }

    #[test]
    fn test_parse_non_utf8() {
        let header = basic(&[0xff, 0x3a, 0x61]);
        assert_eq!(parse_proxy_authorization(Some(&header)), None);
    }
}
