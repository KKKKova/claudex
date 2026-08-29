use std::collections::HashMap;

use anyhow::Result;
use reqwest::RequestBuilder;
use serde_json::Value;

use super::{ByteStream, ProviderAdapter, TranslatedRequest};
use crate::config::ProfileConfig;
use crate::proxy::util::ToolNameMap;

/// api.anthropic.com が claude.ai 系 OAuth トークンを受理する際に必要な beta ヘッダ値。
/// T001（実トークンでの受理条件確認）が未実施のため暫定値。定数をここに閉じ、
/// 確定次第この1箇所を差し替える。
const OAUTH_BETA: &str = "oauth-2025-04-20";

/// `api_key` が claude.ai 系 OAuth アクセストークン（`sk-ant-oat...`）かどうかを判定する。
pub(crate) fn is_anthropic_oauth_token(key: &str) -> bool {
    key.starts_with("sk-ant-oat")
}

pub struct DirectAnthropicAdapter;

impl ProviderAdapter for DirectAnthropicAdapter {
    fn endpoint_path(&self) -> &str {
        "/v1/messages"
    }

    fn translate_request(
        &self,
        body: &Value,
        _profile: &ProfileConfig,
    ) -> Result<TranslatedRequest> {
        Ok(TranslatedRequest {
            body: body.clone(),
            tool_name_map: HashMap::new(),
        })
    }

    fn apply_auth(&self, builder: RequestBuilder, profile: &ProfileConfig) -> RequestBuilder {
        let mut b = builder.header("anthropic-version", "2023-06-01");
        if profile.api_key.is_empty() {
            return b;
        }
        if is_anthropic_oauth_token(&profile.api_key) {
            b = b.header("authorization", format!("Bearer {}", profile.api_key));
            // custom_headers に anthropic-beta があればそちらを尊重し、二重付与しない
            // （try_forward は custom_headers を後から append する。キーは TOML 表記のままなので大小無視で照合）
            let has_beta = profile
                .custom_headers
                .keys()
                .any(|k| k.eq_ignore_ascii_case("anthropic-beta"));
            if !has_beta {
                b = b.header("anthropic-beta", OAUTH_BETA);
            }
        } else {
            if profile.api_key.starts_with("sk-ant-") && !profile.api_key.starts_with("sk-ant-api")
            {
                // 毎リクエスト出すと洪水になるのでプロセスで1回だけ
                static PREFIX_WARN: std::sync::Once = std::sync::Once::new();
                PREFIX_WARN.call_once(|| {
                    tracing::warn!("api_key looks like an Anthropic token but is neither an API key (sk-ant-api…) nor an OAuth access token (sk-ant-oat…); sending as x-api-key");
                });
            }
            b = b.header("x-api-key", &profile.api_key);
        }
        b
    }

    fn passthrough(&self) -> bool {
        true
    }

    fn translate_response(&self, body: &Value, _tool_name_map: &ToolNameMap) -> Result<Value> {
        Ok(body.clone())
    }

    fn translate_stream(&self, stream: ByteStream, _tool_name_map: ToolNameMap) -> ByteStream {
        stream
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::HeaderValue;

    fn built_request(profile: &ProfileConfig) -> reqwest::Request {
        let adapter = DirectAnthropicAdapter;
        let client = reqwest::Client::new();
        let builder = client.post("https://api.anthropic.com/v1/messages");
        let builder = adapter.apply_auth(builder, profile);
        builder.build().expect("request should build")
    }

    #[test]
    fn oauth_token_uses_bearer_and_beta_header() {
        let profile = ProfileConfig {
            api_key: "sk-ant-oat01-xxxxx".to_string(),
            ..Default::default()
        };
        let req = built_request(&profile);
        let headers = req.headers();
        assert_eq!(
            headers.get("authorization"),
            Some(&HeaderValue::from_str(&format!("Bearer {}", profile.api_key)).unwrap())
        );
        assert_eq!(
            headers.get("anthropic-beta"),
            Some(&HeaderValue::from_static(OAUTH_BETA))
        );
        assert!(headers.get("x-api-key").is_none());
    }

    #[test]
    fn oauth_token_with_lowercase_custom_beta_header_is_not_overridden() {
        let mut custom_headers = HashMap::new();
        custom_headers.insert("anthropic-beta".to_string(), "custom-beta".to_string());
        let profile = ProfileConfig {
            api_key: "sk-ant-oat01-xxxxx".to_string(),
            custom_headers,
            ..Default::default()
        };
        let req = built_request(&profile);
        // apply_auth 自体は anthropic-beta を付けない（後段の custom_headers 適用に任せる）
        assert!(req.headers().get("anthropic-beta").is_none());
    }

    #[test]
    fn oauth_token_with_mixed_case_custom_beta_header_is_not_overridden() {
        let mut custom_headers = HashMap::new();
        custom_headers.insert("Anthropic-Beta".to_string(), "custom-beta".to_string());
        let profile = ProfileConfig {
            api_key: "sk-ant-oat01-xxxxx".to_string(),
            custom_headers,
            ..Default::default()
        };
        let req = built_request(&profile);
        assert!(req.headers().get("anthropic-beta").is_none());
    }

    #[test]
    fn api_key_uses_x_api_key_header() {
        let profile = ProfileConfig {
            api_key: "sk-ant-api01-xxxxx".to_string(),
            ..Default::default()
        };
        let req = built_request(&profile);
        let headers = req.headers();
        assert_eq!(
            headers.get("x-api-key"),
            Some(&HeaderValue::from_str(&profile.api_key).unwrap())
        );
        assert!(headers.get("authorization").is_none());
    }

    #[test]
    fn empty_api_key_has_no_auth_header() {
        let profile = ProfileConfig {
            api_key: String::new(),
            ..Default::default()
        };
        let req = built_request(&profile);
        let headers = req.headers();
        assert!(headers.get("authorization").is_none());
        assert!(headers.get("x-api-key").is_none());
        assert!(headers.get("anthropic-version").is_some());
    }

    #[test]
    fn is_anthropic_oauth_token_detects_prefix() {
        assert!(is_anthropic_oauth_token("sk-ant-oat01-xxxxx"));
        assert!(!is_anthropic_oauth_token("sk-ant-api01-xxxxx"));
        assert!(!is_anthropic_oauth_token(""));
    }
}
