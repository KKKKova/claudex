//! 推論努力度（reasoning effort）の転送。
//!
//! Claude Code は `/effort` の設定を Anthropic Messages API の
//! `output_config.effort` として送る（low/medium/high/xhigh/max）。翻訳層は
//! 従来この項目を落としていたため、OpenAI 系プロバイダでは effort 指定が
//! 一切効いていなかった。ここで正規化し、各プロトコルのパラメータへ載せ替える。
//!
//! 送り先: OpenAI Responses API は `reasoning.effort`、Chat Completions は
//! top-level `reasoning_effort`。既定の写像は恒等（low→low ... max→max）だが、
//! 上流モデルによっては未対応の値を送ると 400 になるため、`MODEL_EFFORTS` の
//! 対応表でクランプする。`[profiles.effort.map]` による上書きはクランプより
//! 常に優先する。

use serde_json::{json, Value};

use crate::config::ProfileConfig;

/// Anthropic 側の effort レベル
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Effort {
    Low,
    Medium,
    High,
    XHigh,
    Max,
}

impl Effort {
    /// low < medium < high < xhigh < max の順
    const ORDER: [Effort; 5] = [
        Effort::Low,
        Effort::Medium,
        Effort::High,
        Effort::XHigh,
        Effort::Max,
    ];

    /// 設定ファイルの上書き表を引くときのキー
    pub fn key(self) -> &'static str {
        match self {
            Effort::Low => "low",
            Effort::Medium => "medium",
            Effort::High => "high",
            Effort::XHigh => "xhigh",
            Effort::Max => "max",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "low" => Some(Effort::Low),
            "medium" => Some(Effort::Medium),
            "high" => Some(Effort::High),
            // Claude Code / API どちらの綴りでも拾う
            "xhigh" | "x-high" | "extra_high" => Some(Effort::XHigh),
            "max" => Some(Effort::Max),
            _ => None,
        }
    }

    /// 旧 API（`thinking.budget_tokens`）からの近似。
    /// 予算そのものは OpenAI 系に相当物がないので、レベルに畳んで扱う。
    fn from_budget_tokens(budget: u64) -> Self {
        match budget {
            0..=4_095 => Effort::Low,
            4_096..=16_383 => Effort::Medium,
            16_384..=32_767 => Effort::High,
            _ => Effort::Max,
        }
    }

    /// `Self::ORDER` 内での位置（低い順）
    fn rank(self) -> usize {
        Self::ORDER
            .iter()
            .position(|e| *e == self)
            .expect("ORDER covers every Effort variant")
    }
}

/// 上流モデルごとに受け付ける effort。前方一致で最初に当たった行を採用する。
/// より長い／具体的なパターンを先に置くこと（前方一致が重なる場合の曖昧さを避ける）。
/// 出典: Codex CLI の models-manager/models.json、xAI の reasoning ドキュメント。
/// 未登録のモデルは全段階を許可（＝恒等写像）扱いにする。
///
/// Ollama はここに載せない: xhigh を弾くなどの制約はモデルではなくサーバ実装側の
/// 性質で、`huihui_ai/Qwen3.6-abliterated:35b` のような任意のモデル名に対して
/// 前方一致パターンを作れないため。Ollama 利用者は `enabled = true` + `map` で
/// 個別に上書きする運用にする。
const MODEL_EFFORTS: &[(&str, &[Effort])] = &[
    (
        "gpt-5.6",
        &[
            Effort::Low,
            Effort::Medium,
            Effort::High,
            Effort::XHigh,
            Effort::Max,
        ],
    ),
    (
        "gpt-5.5",
        &[Effort::Low, Effort::Medium, Effort::High, Effort::XHigh],
    ),
    (
        "gpt-5.4",
        &[Effort::Low, Effort::Medium, Effort::High, Effort::XHigh],
    ),
    (
        "gpt-5.2",
        &[Effort::Low, Effort::Medium, Effort::High, Effort::XHigh],
    ),
    (
        "grok-4.20",
        &[Effort::Low, Effort::Medium, Effort::High, Effort::XHigh],
    ),
    ("grok-4.5", &[Effort::Low, Effort::Medium, Effort::High]),
];

/// モデル対応表に基づき、要求レベルを上流が受け付けるレベルへ丸める。
/// 対応表に無いモデルは全段階許可（恒等写像）。
/// 要求レベルが対応表に無ければ、まず一段ずつ下げて最初に見つかった対応レベルを
/// 使う。下方向に対応レベルが無ければ、対応レベルのうち最も低いものへ上げる。
fn clamp_for_upstream(effort: Effort, upstream_model: &str) -> Effort {
    let Some((_, allowed)) = MODEL_EFFORTS
        .iter()
        .find(|(prefix, _)| upstream_model.starts_with(prefix))
    else {
        return effort;
    };
    if allowed.contains(&effort) {
        return effort;
    }
    let rank = effort.rank();
    for lower_rank in (0..rank).rev() {
        let candidate = Effort::ORDER[lower_rank];
        if allowed.contains(&candidate) {
            return candidate;
        }
    }
    // 下方向に対応レベルが無ければ、対応レベルのうち最も低いものへ上げる
    *allowed
        .iter()
        .min_by_key(|e| e.rank())
        .expect("MODEL_EFFORTS entries must list at least one allowed level")
}

/// Anthropic リクエストから effort を取り出す。
pub fn extract(anthropic: &Value) -> Option<Effort> {
    // 現行 Claude Code: output_config.effort
    let from_output_config = anthropic
        .get("output_config")
        .and_then(|c| c.get("effort"))
        .and_then(effort_from_value);
    if from_output_config.is_some() {
        return from_output_config;
    }

    // 念のため top-level の effort も見る（将来/他クライアント向け）
    if let Some(effort) = anthropic.get("effort").and_then(effort_from_value) {
        return Some(effort);
    }

    // 旧 API: thinking.budget_tokens
    anthropic
        .get("thinking")
        .and_then(|t| t.get("budget_tokens"))
        .and_then(|b| b.as_u64())
        .map(Effort::from_budget_tokens)
}

/// `"high"` と `{"type": "high"}` の両方を受ける
fn effort_from_value(value: &Value) -> Option<Effort> {
    match value {
        Value::String(s) => Effort::parse(s),
        Value::Object(_) => value
            .get("type")
            .and_then(|t| t.as_str())
            .and_then(Effort::parse),
        _ => None,
    }
}

/// リクエストと profile 設定から、上流に送る effort 文字列を決める。
/// `enabled` 相当が無効、または effort 指定なしのときは `None`。
///
/// 優先順位: `[profiles.effort.map]` の上書き（最優先、クランプを飛び越す）
/// → モデル対応表によるクランプ → 恒等写像。
fn resolve(
    anthropic: &Value,
    profile: &ProfileConfig,
    upstream_model: &str,
    default_enabled: bool,
) -> Option<String> {
    let enabled = profile.effort.enabled.unwrap_or(default_enabled);
    if !enabled {
        return None;
    }
    let effort = extract(anthropic)?;
    let value = match profile.effort.map.get(effort.key()) {
        Some(overridden) => overridden.clone(),
        None => clamp_for_upstream(effort, upstream_model).key().to_string(),
    };
    // 明示的に空文字を設定したレベルは送らない（そのレベルだけ無効化する逃げ道）
    if value.is_empty() {
        return None;
    }
    tracing::info!(
        profile = %profile.name,
        requested = effort.key(),
        forwarded = %value,
        "forwarding reasoning effort"
    );
    Some(value)
}

/// OpenAI Responses API: `reasoning.effort`
/// `default_enabled` はアダプタ側の既定（Responses は true）を渡す。
pub fn apply_to_responses(
    req: &mut Value,
    anthropic: &Value,
    profile: &ProfileConfig,
    default_enabled: bool,
) {
    let upstream_model = req.get("model").and_then(Value::as_str).unwrap_or("");
    let Some(value) = resolve(anthropic, profile, upstream_model, default_enabled) else {
        return;
    };
    match req.get_mut("reasoning").and_then(|r| r.as_object_mut()) {
        Some(reasoning) => {
            reasoning.insert("effort".to_string(), json!(value));
        }
        None => req["reasoning"] = json!({ "effort": value }),
    }
}

/// OpenAI Chat Completions API: top-level `reasoning_effort`
/// `default_enabled` はアダプタ側の既定（Chat Completions は false）を渡す。
pub fn apply_to_chat_completions(
    req: &mut Value,
    anthropic: &Value,
    profile: &ProfileConfig,
    default_enabled: bool,
) {
    let upstream_model = req.get("model").and_then(Value::as_str).unwrap_or("");
    let Some(value) = resolve(anthropic, profile, upstream_model, default_enabled) else {
        return;
    };
    req["reasoning_effort"] = json!(value);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile() -> ProfileConfig {
        ProfileConfig {
            name: "test".to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn test_extract_output_config_effort() {
        let req = json!({"output_config": {"effort": "xhigh"}});
        assert_eq!(extract(&req), Some(Effort::XHigh));
    }

    #[test]
    fn test_extract_effort_object_form() {
        let req = json!({"output_config": {"effort": {"type": "medium"}}});
        assert_eq!(extract(&req), Some(Effort::Medium));
    }

    #[test]
    fn test_extract_falls_back_to_budget_tokens() {
        let req = json!({"thinking": {"type": "enabled", "budget_tokens": 8000}});
        assert_eq!(extract(&req), Some(Effort::Medium));
    }

    #[test]
    fn test_extract_none_when_absent() {
        assert_eq!(extract(&json!({"model": "m"})), None);
        // adaptive thinking には予算がないので effort の手がかりにならない
        assert_eq!(extract(&json!({"thinking": {"type": "adaptive"}})), None);
    }

    #[test]
    fn test_identity_mapping_when_model_supports_all_levels() {
        let mut req = json!({"model": "gpt-5.6-sol"});
        apply_to_responses(
            &mut req,
            &json!({"output_config": {"effort": "xhigh"}}),
            &profile(),
            true,
        );
        assert_eq!(req["reasoning"]["effort"], "xhigh");

        let mut req = json!({"model": "gpt-5.6-sol"});
        apply_to_responses(
            &mut req,
            &json!({"output_config": {"effort": "max"}}),
            &profile(),
            true,
        );
        assert_eq!(req["reasoning"]["effort"], "max");
    }

    #[test]
    fn test_clamp_one_step_down() {
        let mut req = json!({"model": "gpt-5.4-mini"});
        apply_to_responses(
            &mut req,
            &json!({"output_config": {"effort": "max"}}),
            &profile(),
            true,
        );
        assert_eq!(req["reasoning"]["effort"], "xhigh");

        let mut req = json!({"model": "gpt-5.4-mini"});
        apply_to_responses(
            &mut req,
            &json!({"output_config": {"effort": "xhigh"}}),
            &profile(),
            true,
        );
        assert_eq!(req["reasoning"]["effort"], "xhigh");
    }

    #[test]
    fn test_clamp_multiple_steps_down() {
        let mut req = json!({"model": "grok-4.5"});
        apply_to_responses(
            &mut req,
            &json!({"output_config": {"effort": "max"}}),
            &profile(),
            true,
        );
        assert_eq!(req["reasoning"]["effort"], "high");

        let mut req = json!({"model": "grok-4.5"});
        apply_to_responses(
            &mut req,
            &json!({"output_config": {"effort": "xhigh"}}),
            &profile(),
            true,
        );
        assert_eq!(req["reasoning"]["effort"], "high");
    }

    #[test]
    fn test_unregistered_model_is_identity() {
        let mut req = json!({"model": "huihui_ai/Qwen3.6-abliterated:35b"});
        apply_to_responses(
            &mut req,
            &json!({"output_config": {"effort": "max"}}),
            &profile(),
            true,
        );
        assert_eq!(req["reasoning"]["effort"], "max");
    }

    #[test]
    fn test_profile_map_overrides_clamp() {
        let mut p = profile();
        p.effort.map.insert("max".to_string(), "max".to_string());
        let mut req = json!({"model": "gpt-5.4-mini"});
        apply_to_responses(
            &mut req,
            &json!({"output_config": {"effort": "max"}}),
            &p,
            true,
        );
        assert_eq!(req["reasoning"]["effort"], "max");
    }

    #[test]
    fn test_empty_mapping_disables_that_level() {
        let mut p = profile();
        p.effort.map.insert("max".to_string(), String::new());
        let mut req = json!({"model": "gpt-5.6-sol"});
        apply_to_responses(
            &mut req,
            &json!({"output_config": {"effort": "max"}}),
            &p,
            true,
        );
        assert!(req.get("reasoning").is_none());
    }

    #[test]
    fn test_disabled_by_profile_sends_nothing() {
        let mut p = profile();
        p.effort.enabled = Some(false);
        let mut req = json!({"model": "gpt-5.6-sol"});
        apply_to_responses(
            &mut req,
            &json!({"output_config": {"effort": "high"}}),
            &p,
            true,
        );
        apply_to_chat_completions(
            &mut req,
            &json!({"output_config": {"effort": "high"}}),
            &p,
            true,
        );
        assert!(req.get("reasoning").is_none());
        assert!(req.get("reasoning_effort").is_none());
    }

    #[test]
    fn test_default_enabled_false_sends_nothing() {
        let mut req = json!({"model": "gpt-4o"});
        apply_to_chat_completions(
            &mut req,
            &json!({"output_config": {"effort": "high"}}),
            &profile(),
            false,
        );
        assert!(req.get("reasoning_effort").is_none());
    }

    #[test]
    fn test_apply_to_responses_preserves_existing_reasoning_fields() {
        let mut req = json!({"model": "gpt-5.6-sol", "reasoning": {"summary": "auto"}});
        apply_to_responses(
            &mut req,
            &json!({"output_config": {"effort": "high"}}),
            &profile(),
            true,
        );
        assert_eq!(req["reasoning"]["summary"], "auto");
        assert_eq!(req["reasoning"]["effort"], "high");
    }

    #[test]
    fn test_apply_to_chat_completions_when_explicitly_enabled() {
        let mut p = profile();
        p.effort.enabled = Some(true);
        let mut req = json!({"model": "grok-4"});
        apply_to_chat_completions(
            &mut req,
            &json!({"output_config": {"effort": "medium"}}),
            &p,
            false,
        );
        assert_eq!(req["reasoning_effort"], "medium");
    }
}
