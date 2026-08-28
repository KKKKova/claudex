use std::collections::HashMap;

use serde_json::{json, Value};

/// OpenAI 工具名最大长度
pub const MAX_TOOL_NAME_LEN: usize = 64;

/// 工具名映射（截断名 → 原始名）
pub type ToolNameMap = HashMap<String, String>;

/// 截断过长的工具名，保持可辨识性
pub fn truncate_tool_name(name: &str) -> String {
    if name.len() <= MAX_TOOL_NAME_LEN {
        return name.to_string();
    }
    // 取前 55 字符 + "_" + 8 字符 hash
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    name.hash(&mut hasher);
    let hash = format!("{:08x}", hasher.finish());
    format!("{}_{}", &name[..MAX_TOOL_NAME_LEN - 9], &hash[..8])
}

/// SSE 格式化
pub fn format_sse(event: &str, data: &Value) -> String {
    format!(
        "event: {event}\ndata: {}\n\n",
        serde_json::to_string(data).unwrap_or_default()
    )
}

/// API key 预览（显示首尾各 4 字符）
pub fn format_key_preview(key: &str) -> String {
    if key.is_empty() {
        "(empty)".to_string()
    } else if key.len() > 8 {
        format!("{}...{}", &key[..4], &key[key.len() - 4..])
    } else {
        "***".to_string()
    }
}

/// 把请求里的模型名解析成实际发往上游的模型名。
///
/// Claude Code 有些请求会绕过 `ANTHROPIC_MODEL`，直接写死内置的 `claude-*` 名字。
/// 最典型的是 auto mode 的 classifier（当前固定发 `claude-sonnet-5`）。这类名字
/// 原样转发给非 Anthropic 提供商必定 400，所以按 tier 映射到 profile 的 slot。
///
/// 仅 OpenAI 系翻译层使用；DirectAnthropic 走透传，不经过这里，因此 Anthropic /
/// MiniMax 等 profile 的 `claude-*` 指定不受影响。
pub struct ModelResolver<'a> {
    default_model: &'a str,
    haiku: Option<&'a str>,
    sonnet: Option<&'a str>,
    opus: Option<&'a str>,
    fable: Option<&'a str>,
}

impl<'a> ModelResolver<'a> {
    pub fn from_profile(profile: &'a crate::config::ProfileConfig) -> Self {
        Self {
            default_model: &profile.default_model,
            haiku: profile.models.haiku.as_deref(),
            sonnet: profile.models.sonnet.as_deref(),
            opus: profile.models.opus.as_deref(),
            fable: profile.models.fable.as_deref(),
        }
    }

    /// 只有 default_model、没有 slot 映射（测试与简单场景用）
    pub fn plain(default_model: &'a str) -> Self {
        Self {
            default_model,
            haiku: None,
            sonnet: None,
            opus: None,
            fable: None,
        }
    }

    pub fn resolve(&self, requested: Option<&'a str>) -> &'a str {
        let Some(requested) = requested else {
            return self.default_model;
        };
        let Some(tier) = requested.strip_prefix("claude-") else {
            return requested;
        };
        // slot 未配置时退回 default_model；default_model 也为空则保留原值，
        // 免得送出空模型名。
        self.slot_for(tier)
            .or(if self.default_model.is_empty() {
                None
            } else {
                Some(self.default_model)
            })
            .unwrap_or(requested)
    }

    fn slot_for(&self, tier: &str) -> Option<&'a str> {
        if tier.starts_with("haiku") {
            self.haiku
        } else if tier.starts_with("sonnet") {
            self.sonnet
        } else if tier.starts_with("opus") {
            self.opus
        } else if tier.starts_with("fable") || tier.starts_with("mythos") {
            self.fable
        } else {
            None
        }
    }
}

/// 构造 Anthropic 格式的错误 JSON
pub fn to_anthropic_error(status: u16, message: &str) -> Value {
    let error_type = match status {
        401 => "authentication_error",
        403 => "permission_error",
        404 => "not_found_error",
        429 => "rate_limit_error",
        _ => "invalid_request_error",
    };
    json!({
        "type": "error",
        "error": {
            "type": error_type,
            "message": message,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_truncate_short_name_unchanged() {
        assert_eq!(truncate_tool_name("get_weather"), "get_weather");
    }

    #[test]
    fn test_truncate_exactly_64_unchanged() {
        let name = "a".repeat(64);
        assert_eq!(truncate_tool_name(&name), name);
    }

    #[test]
    fn test_truncate_65_chars() {
        let name = "a".repeat(65);
        let result = truncate_tool_name(&name);
        assert_eq!(result.len(), 64);
        assert!(result.starts_with("aaaa"));
        assert!(result.contains('_'));
    }

    #[test]
    fn test_truncate_preserves_determinism() {
        let name = "mcp__very_long_server_name__extremely_long_tool_function_name_here_v2";
        let r1 = truncate_tool_name(name);
        let r2 = truncate_tool_name(name);
        assert_eq!(r1, r2);
        assert_eq!(r1.len(), 64);
    }

    #[test]
    fn test_format_sse() {
        let data = json!({"type": "test"});
        let result = format_sse("my_event", &data);
        assert!(result.starts_with("event: my_event\ndata: "));
        assert!(result.ends_with("\n\n"));
        assert!(result.contains("\"type\":\"test\""));
    }

    #[test]
    fn test_format_key_preview_empty() {
        assert_eq!(format_key_preview(""), "(empty)");
    }

    #[test]
    fn test_format_key_preview_short() {
        assert_eq!(format_key_preview("12345678"), "***");
    }

    #[test]
    fn test_format_key_preview_long() {
        assert_eq!(format_key_preview("sk-abcd1234efgh5678"), "sk-a...5678");
    }

    fn resolver_with_slots() -> ModelResolver<'static> {
        ModelResolver {
            default_model: "gpt-5.6-sol",
            haiku: Some("gpt-5-mini"),
            sonnet: Some("gpt-5.6-thinking"),
            opus: None,
            fable: None,
        }
    }

    #[test]
    fn test_resolve_passes_through_provider_model() {
        let r = resolver_with_slots();
        assert_eq!(r.resolve(Some("grok-4-fast")), "grok-4-fast");
        assert_eq!(r.resolve(Some("gpt-5.6-sol")), "gpt-5.6-sol");
    }

    #[test]
    fn test_resolve_maps_claude_tier_to_slot() {
        let r = resolver_with_slots();
        // auto mode classifier 当前固定发 claude-sonnet-5
        assert_eq!(r.resolve(Some("claude-sonnet-5")), "gpt-5.6-thinking");
        assert_eq!(r.resolve(Some("claude-haiku-4-5")), "gpt-5-mini");
    }

    #[test]
    fn test_resolve_falls_back_to_default_when_slot_unset() {
        let r = resolver_with_slots();
        // opus / fable slot 未配置
        assert_eq!(r.resolve(Some("claude-opus-5")), "gpt-5.6-sol");
        assert_eq!(r.resolve(Some("claude-fable-5")), "gpt-5.6-sol");
        // 未知 tier（classifier 模型换代时也走这条路）
        assert_eq!(r.resolve(Some("claude-guard-1")), "gpt-5.6-sol");
    }

    #[test]
    fn test_resolve_missing_model_uses_default() {
        assert_eq!(
            ModelResolver::plain("gpt-5.6-sol").resolve(None),
            "gpt-5.6-sol"
        );
    }

    #[test]
    fn test_resolve_keeps_request_when_nothing_configured() {
        // slot も default_model も無い場合、空モデル名を送らずに元の値を保つ
        let r = ModelResolver::plain("");
        assert_eq!(r.resolve(Some("claude-sonnet-5")), "claude-sonnet-5");
    }

    #[test]
    fn test_to_anthropic_error() {
        let err = to_anthropic_error(401, "invalid key");
        assert_eq!(err["error"]["type"], "authentication_error");
        assert_eq!(err["error"]["message"], "invalid key");
    }
}
