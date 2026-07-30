use anyhow::Result;
use reqwest::RequestBuilder;
use serde_json::Value;

use super::{ByteStream, ProviderAdapter, TranslatedRequest};
use crate::config::ProfileConfig;
use crate::proxy::util::ToolNameMap;

pub struct ChatCompletionsAdapter;

impl ProviderAdapter for ChatCompletionsAdapter {
    fn endpoint_path(&self) -> &str {
        "/chat/completions"
    }

    fn translate_request(
        &self,
        body: &Value,
        profile: &ProfileConfig,
    ) -> Result<TranslatedRequest> {
        let (mut openai_body, tool_name_map) =
            crate::proxy::translate::chat_completions::anthropic_to_openai(
                body,
                &crate::proxy::util::ModelResolver::from_profile(profile),
                profile.max_tokens,
            )?;
        // Chat Completions は未知パラメータで 400 になる上流があるため、
        // effort 転送は既定で無効（profile 側で明示的に opt-in させる）
        crate::proxy::translate::effort::apply_to_chat_completions(
            &mut openai_body,
            body,
            profile,
            false,
        );
        Ok(TranslatedRequest {
            body: openai_body,
            tool_name_map,
        })
    }

    fn apply_auth(&self, builder: RequestBuilder, profile: &ProfileConfig) -> RequestBuilder {
        if !profile.api_key.is_empty() {
            if profile.extra_env.contains_key("AZURE_AUTH")
                || profile.base_url.contains("openai.azure.com")
            {
                builder.header("api-key", &profile.api_key)
            } else {
                builder.header("Authorization", format!("Bearer {}", profile.api_key))
            }
        } else {
            builder
        }
    }

    fn apply_extra_headers(
        &self,
        mut builder: RequestBuilder,
        profile: &ProfileConfig,
    ) -> RequestBuilder {
        // GitHub Copilot: 添加伪装 headers
        if profile.extra_env.contains_key("COPILOT_AUTH")
            || profile.base_url.contains("githubcopilot.com")
        {
            for (k, v) in crate::oauth::exchange::copilot_extra_headers() {
                builder = builder.header(k, v);
            }
        }
        builder
    }

    fn translate_response(&self, body: &Value, tool_name_map: &ToolNameMap) -> Result<Value> {
        crate::proxy::translate::chat_completions::openai_to_anthropic(body, tool_name_map)
    }

    fn translate_stream(&self, stream: ByteStream, tool_name_map: ToolNameMap) -> ByteStream {
        crate::proxy::translate::chat_completions_stream::translate_sse_stream(
            stream,
            tool_name_map,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn profile() -> ProfileConfig {
        ProfileConfig {
            name: "test".to_string(),
            base_url: "https://example.com".to_string(),
            default_model: "gpt-4o".to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn test_translate_request_omits_effort_by_default() {
        let adapter = ChatCompletionsAdapter;
        let body = json!({
            "model": "claude-opus-5",
            "messages": [{"role": "user", "content": "hi"}],
            "max_tokens": 100,
            "output_config": {"effort": "high"},
        });
        let translated = adapter.translate_request(&body, &profile()).unwrap();
        assert!(translated.body.get("reasoning_effort").is_none());
    }

    #[test]
    fn test_translate_request_forwards_effort_when_enabled() {
        let adapter = ChatCompletionsAdapter;
        let mut p = profile();
        p.effort.enabled = Some(true);
        let body = json!({
            "model": "claude-opus-5",
            "messages": [{"role": "user", "content": "hi"}],
            "max_tokens": 100,
            "output_config": {"effort": "medium"},
        });
        let translated = adapter.translate_request(&body, &p).unwrap();
        assert_eq!(translated.body["reasoning_effort"], "medium");
    }
}
