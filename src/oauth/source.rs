//! Layer 1: Token Sources
//!
//! 统一凭证读取层，支持多种来源: 环境变量、config、外部 CLI 文件、keyring、Copilot config。

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::{OAuthProvider, OAuthToken};

// ── Types ────────────────────────────────────────────────────────────────

/// 凭证来源标识
#[derive(Debug, Clone)]
pub enum CredentialSource {
    EnvVar(String),
    ConfigApiKey,
    ExternalCli(String),
    Keyring,
    CopilotConfig,
}

/// 原始凭证（从某来源读取、尚未经过 exchange 处理）
#[derive(Debug, Clone)]
pub struct RawCredential {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: Option<i64>,
    pub token_type: Option<String>,
    pub extra: Option<serde_json::Value>,
    pub source: CredentialSource,
}

impl RawCredential {
    pub fn into_oauth_token(self) -> OAuthToken {
        OAuthToken {
            access_token: self.access_token,
            refresh_token: self.refresh_token,
            expires_at: self.expires_at,
            token_type: self.token_type,
            scopes: None,
            extra: self.extra,
        }
    }
}

// ── Keyring ──────────────────────────────────────────────────────────────

const KEYRING_SERVICE: &str = "claudex";

fn keyring_entry_name(profile_name: &str) -> String {
    format!("{profile_name}-oauth-token")
}

pub fn store_keyring(profile_name: &str, token: &OAuthToken) -> Result<()> {
    let entry_name = keyring_entry_name(profile_name);
    let json = serde_json::to_string(token).context("failed to serialize token")?;
    let entry = keyring::Entry::new(KEYRING_SERVICE, &entry_name)
        .context("failed to create keyring entry")?;
    entry
        .set_password(&json)
        .context("failed to store token in keyring")?;
    Ok(())
}

/// Best-effort 写入 keyring：失败时仅记录 warning，不中断流程。
///
/// 用于 ChatGPT/Claude/Google/Kimi 等 provider —— 它们的源真相是外部 CLI
/// 文件（如 `~/.codex/auth.json`）或本 diff 新增的 per-profile 文件，keyring
/// 仅作为冗余缓存。Windows Credential Manager 单条上限 ~2560 字符，存满
/// OAuth JSON 时会失败；此函数避免那种失败破坏整个 login/refresh 流程。
///
/// GitLab（環境変数のみが情報源）と GitHub device-code 経路
/// （Copilot CLI 未導入・`GITHUB_TOKEN` 未設定時は keyring が唯一の永続化先）
/// は対象外。これらは keyring 以外に保存先を持たないため `store_keyring(...)?`
/// で失敗を呼び出し元へ伝播させる。
pub fn store_keyring_best_effort(profile_name: &str, token: &OAuthToken) {
    if let Err(e) = store_keyring(profile_name, token) {
        tracing::warn!(
            profile = %profile_name,
            error = %format!("{e:#}"),
            "keyring store failed; token still available via external credential file"
        );
    }
}

pub fn load_keyring(profile_name: &str) -> Result<OAuthToken> {
    let entry_name = keyring_entry_name(profile_name);
    let entry = keyring::Entry::new(KEYRING_SERVICE, &entry_name)
        .context("failed to create keyring entry")?;
    let json = entry
        .get_password()
        .context("no OAuth token found in keyring")?;
    let token: OAuthToken = serde_json::from_str(&json).context("failed to parse stored token")?;
    Ok(token)
}

pub fn delete_keyring(profile_name: &str) -> Result<()> {
    let entry_name = keyring_entry_name(profile_name);
    let entry = keyring::Entry::new(KEYRING_SERVICE, &entry_name)
        .context("failed to create keyring entry")?;
    entry
        .delete_credential()
        .context("failed to delete token from keyring")?;
    Ok(())
}

// ── External CLI Readers ─────────────────────────────────────────────────

/// Claude Code 本体が保存している claude.ai セッション
///
/// macOS では keychain（service = "Claude Code-credentials"）、それ以外では
/// `~/.claude/.credentials.json` に置かれる。どちらも同じ JSON 形状なので、
/// 読めたほうを返す。
fn read_claude_ai_oauth() -> Result<serde_json::Value> {
    let home = dirs::home_dir().context("cannot determine home directory")?;
    let cred_path = home.join(".claude").join(".credentials.json");

    let content = match std::fs::read_to_string(&cred_path) {
        Ok(content) => content,
        Err(file_err) => read_claude_keychain().map_err(|keychain_err| {
            anyhow::anyhow!(
                "cannot read {}: {file_err}; keychain fallback failed: {keychain_err}",
                cred_path.display()
            )
        })?,
    };

    let json: serde_json::Value =
        serde_json::from_str(&content).context("invalid JSON in credentials file")?;
    json.get("claudeAiOauth")
        .cloned()
        .context("missing 'claudeAiOauth' in credentials")
}

const CLAUDE_KEYCHAIN_SERVICE: &str = "Claude Code-credentials";

fn read_claude_keychain() -> Result<String> {
    let account =
        std::env::var("USER").context("cannot determine keychain account (USER unset)")?;
    let entry = keyring::Entry::new(CLAUDE_KEYCHAIN_SERVICE, &account)
        .context("failed to create keyring entry")?;
    entry
        .get_password()
        .context("no Claude Code credentials in keychain")
}

/// 读取 Claude CLI 的 credentials（~/.claude/.credentials.json 或 keychain）
pub fn read_claude_credentials() -> Result<RawCredential> {
    let oauth_obj = &read_claude_ai_oauth()?;

    let access_token = oauth_obj
        .get("accessToken")
        .and_then(|v| v.as_str())
        .context("missing 'accessToken' in claudeAiOauth")?
        .to_string();

    let expires_at = oauth_obj
        .get("expiresAt")
        .and_then(|v| v.as_i64())
        .or_else(|| {
            oauth_obj
                .get("expiresAt")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<i64>().ok())
        });

    Ok(RawCredential {
        access_token,
        refresh_token: oauth_obj
            .get("refreshToken")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        expires_at,
        token_type: Some("Bearer".to_string()),
        extra: None,
        source: CredentialSource::ExternalCli("~/.claude/.credentials.json".to_string()),
    })
}

/// Remote Control 用に取り出した claude.ai セッション
#[derive(Debug, Clone)]
pub struct ClaudeAiSession {
    pub access_token: String,
    pub scopes: Vec<String>,
    /// 失効時刻（UNIX epoch 秒）。Claude Code はミリ秒で保存するので変換済み
    pub expires_at: Option<i64>,
}

impl ClaudeAiSession {
    /// 失効までの残り秒数。失効済みなら負、期限不明なら None
    pub fn remaining_secs(&self, now: i64) -> Option<i64> {
        self.expires_at.map(|at| at - now)
    }
}

/// Claude Code は expiresAt をミリ秒で保存する。秒で保存された値も受け付ける
fn normalize_expires_at(raw: i64) -> i64 {
    const YEAR_2001_IN_SECS: i64 = 1_000_000_000;
    if raw > YEAR_2001_IN_SECS * 1000 {
        raw / 1000
    } else {
        raw
    }
}

/// Claude Code が保持している claude.ai のログイン情報を、スコープ付きで読む
///
/// Remote Control は `user:profile` を含むフルスコープのログインを要求する。
/// 保存済みのスコープをそのまま渡すため、`setup-token` の推論専用トークンで
/// 起動した場合は Claude Code 側が正しく弾く。
pub fn read_claude_ai_session() -> Result<ClaudeAiSession> {
    let oauth_obj = read_claude_ai_oauth()?;

    let access_token = oauth_obj
        .get("accessToken")
        .and_then(|v| v.as_str())
        .context("missing 'accessToken' in claudeAiOauth")?
        .to_string();

    let scopes = oauth_obj
        .get("scopes")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    if scopes.is_empty() {
        anyhow::bail!("claude.ai credentials carry no scopes");
    }

    let expires_at = oauth_obj
        .get("expiresAt")
        .and_then(|v| {
            v.as_i64()
                .or_else(|| v.as_str().and_then(|s| s.parse::<i64>().ok()))
        })
        .map(normalize_expires_at);

    Ok(ClaudeAiSession {
        access_token,
        scopes,
        expires_at,
    })
}

/// 展开路径中的 `~` 前缀为用户主目录。
fn expand_user_path(p: &str) -> PathBuf {
    let trimmed = p.trim();
    if let Some(rest) = trimmed
        .strip_prefix("~/")
        .or_else(|| trimmed.strip_prefix("~\\"))
    {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(trimmed)
}

/// 解析某个 profile 使用的 Codex `auth.json` 路径。
///
/// `custom` 为 profile 的 `codex_auth_path` 字段：
/// - None / 空 → 默认 `~/.codex/auth.json`（与 Codex CLI 共用，复用已有登录）
/// - 指定路径 → 独立文件（隔离多账号，不影响 Codex CLI 自身的 auth.json）
pub fn codex_auth_path(custom: Option<&str>) -> Result<PathBuf> {
    match custom {
        Some(p) if !p.trim().is_empty() => Ok(expand_user_path(p)),
        _ => {
            let home = dirs::home_dir().context("cannot determine home directory")?;
            Ok(home.join(".codex").join("auth.json"))
        }
    }
}

/// 读取 Codex CLI 的 credentials（默认 ~/.codex/auth.json）
pub fn read_codex_credentials() -> Result<RawCredential> {
    read_codex_credentials_at(&codex_auth_path(None)?)
}

/// 从指定路径读取 Codex 风格的 credentials（支持每 profile 独立文件）
pub fn read_codex_credentials_at(cred_path: &Path) -> Result<RawCredential> {
    let content = std::fs::read_to_string(cred_path)
        .with_context(|| format!("cannot read {}", cred_path.display()))?;
    let json: serde_json::Value =
        serde_json::from_str(&content).context("invalid JSON in auth file")?;

    let tokens = json.get("tokens");

    let access_token = tokens
        .and_then(|t| t.get("access_token"))
        .and_then(|v| v.as_str())
        .or_else(|| json.get("access_token").and_then(|v| v.as_str()))
        .or_else(|| json.get("OPENAI_API_KEY").and_then(|v| v.as_str()))
        .context("no access_token found in codex auth file")?
        .to_string();

    let refresh_token = tokens
        .and_then(|t| t.get("refresh_token"))
        .and_then(|v| v.as_str())
        .or_else(|| json.get("refresh_token").and_then(|v| v.as_str()))
        .map(|s| s.to_string());

    let expires_at = extract_jwt_exp(&access_token);

    let auth_mode = json
        .get("auth_mode")
        .and_then(|v| v.as_str())
        .unwrap_or("api-key");

    // 提取 account_id: tokens.account_id > id_token JWT > access_token JWT
    //
    // 最后的 access_token 回退很关键: claudex 自身写回 auth.json 时
    // （`write_codex_credentials_atomic_at`）只持久化 access_token/refresh_token，
    // 不写 id_token/account_id。因此 claudex 登录或刷新后的文件常缺这两字段，
    // 但 access_token JWT 内始终带有 `chatgpt_account_id`，由此恢复，避免
    // 代理请求 Codex 后端时漏掉必需的 `ChatGPT-Account-ID` 头。
    let account_id = tokens
        .and_then(|t| t.get("account_id"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| {
            let id_token = tokens
                .and_then(|t| t.get("id_token"))
                .and_then(|v| v.as_str())?;
            extract_jwt_claim(
                id_token,
                "https://api.openai.com/auth",
                "chatgpt_account_id",
            )
        })
        .or_else(|| {
            extract_jwt_claim(
                &access_token,
                "https://api.openai.com/auth",
                "chatgpt_account_id",
            )
        });

    let mut extra = serde_json::json!({ "auth_mode": auth_mode });
    if let Some(ref aid) = account_id {
        extra["account_id"] = serde_json::json!(aid);
    }

    Ok(RawCredential {
        access_token,
        refresh_token,
        expires_at,
        token_type: Some("Bearer".to_string()),
        extra: Some(extra),
        source: CredentialSource::ExternalCli(cred_path.display().to_string()),
    })
}

/// 读取 GitHub Copilot 的已有配置
/// 支持 ~/.config/github-copilot/hosts.json 和 apps.json
/// enterprise_host: 可选企业版 host (如 "company.ghe.com")，用于搜索 apps.json
pub fn read_copilot_config() -> Result<RawCredential> {
    read_copilot_config_with_host(None)
}

pub fn read_copilot_config_with_host(enterprise_host: Option<&str>) -> Result<RawCredential> {
    let config_dir = std::env::var("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| dirs::home_dir().unwrap_or_default().join(".config"));
    let copilot_dir = config_dir.join("github-copilot");

    let host_pattern = enterprise_host.unwrap_or("github.com");

    // 优先尝试 apps.json (key 格式: "github.com:CLIENT_ID" 或 "enterprise.ghe.com:CLIENT_ID")
    let apps_path = copilot_dir.join("apps.json");
    if let Ok(content) = std::fs::read_to_string(&apps_path) {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
            if let Some(obj) = json.as_object() {
                for (key, value) in obj {
                    if key.contains(host_pattern) {
                        if let Some(token) = value.get("oauth_token").and_then(|v| v.as_str()) {
                            return Ok(RawCredential {
                                access_token: token.to_string(),
                                refresh_token: None,
                                expires_at: None,
                                token_type: Some("token".to_string()),
                                extra: Some(serde_json::json!({"source_key": key})),
                                source: CredentialSource::CopilotConfig,
                            });
                        }
                    }
                }
            }
        }
    }

    // 回退到 hosts.json (格式: {"github.com": {"oauth_token": "gho_xxx"}})
    let hosts_path = copilot_dir.join("hosts.json");
    if let Ok(content) = std::fs::read_to_string(&hosts_path) {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
            if let Some(obj) = json.as_object() {
                for (key, value) in obj {
                    if key.contains(host_pattern) {
                        if let Some(token) = value.get("oauth_token").and_then(|v| v.as_str()) {
                            return Ok(RawCredential {
                                access_token: token.to_string(),
                                refresh_token: None,
                                expires_at: None,
                                token_type: Some("token".to_string()),
                                extra: None,
                                source: CredentialSource::CopilotConfig,
                            });
                        }
                    }
                }
            }
        }
    }

    anyhow::bail!(
        "no GitHub Copilot credentials found in {}",
        copilot_dir.display()
    )
}

/// 读取 Gemini CLI 的 credentials
pub fn read_gemini_credentials() -> Result<RawCredential> {
    let home = dirs::home_dir().context("cannot determine home directory")?;
    let candidates = [
        home.join(".gemini").join("oauth_creds.json"),
        home.join(".config").join("gemini").join("oauth_creds.json"),
    ];
    read_cli_credentials(&candidates, "Gemini")
}

/// 读取 Kimi CLI 的 credentials
pub fn read_kimi_credentials() -> Result<RawCredential> {
    let home = dirs::home_dir().context("cannot determine home directory")?;
    let candidates = [
        home.join(".kimi").join("auth.json"),
        home.join(".config").join("kimi").join("auth.json"),
    ];
    read_cli_credentials(&candidates, "Kimi")
}

/// 通用 CLI credentials 读取器
fn read_cli_credentials(
    candidates: &[std::path::PathBuf],
    provider: &str,
) -> Result<RawCredential> {
    for path in candidates {
        if let Ok(content) = std::fs::read_to_string(path) {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                let access_token = json
                    .get("access_token")
                    .or_else(|| json.get("token"))
                    .and_then(|v| v.as_str());

                if let Some(token) = access_token {
                    return Ok(RawCredential {
                        access_token: token.to_string(),
                        refresh_token: json
                            .get("refresh_token")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string()),
                        expires_at: json.get("expires_at").and_then(|v| v.as_i64()),
                        token_type: Some("Bearer".to_string()),
                        extra: None,
                        source: CredentialSource::ExternalCli(path.display().to_string()),
                    });
                }
            }
        }
    }

    anyhow::bail!("no {provider} CLI credentials found")
}

// ── Credential Chain ─────────────────────────────────────────────────────

/// 多源 fallback 链: 按优先级尝试不同来源加载凭证
pub fn load_credential_chain(provider: &OAuthProvider) -> Result<RawCredential> {
    load_credential_chain_with_codex(provider, None)
}

/// 同 `load_credential_chain`，但允许为 ChatGPT/Codex 指定每 profile 独立的
/// `auth.json` 路径（`codex_path`）。其余 provider 忽略该参数。
pub fn load_credential_chain_with_codex(
    provider: &OAuthProvider,
    codex_path: Option<&str>,
) -> Result<RawCredential> {
    // normalize: Openai -> Chatgpt
    let provider = provider.normalize();

    match provider {
        OAuthProvider::Claude => {
            // env ANTHROPIC_API_KEY > ~/.claude/.credentials.json > keyring
            if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
                if !key.is_empty() {
                    return Ok(RawCredential {
                        access_token: key,
                        refresh_token: None,
                        expires_at: None,
                        token_type: Some("Bearer".to_string()),
                        extra: None,
                        source: CredentialSource::EnvVar("ANTHROPIC_API_KEY".to_string()),
                    });
                }
            }
            read_claude_credentials()
        }
        OAuthProvider::Chatgpt => {
            // env CODEX_API_KEY > ~/.codex/auth.json > keyring
            if let Ok(key) = std::env::var("CODEX_API_KEY") {
                if !key.is_empty() {
                    return Ok(RawCredential {
                        access_token: key,
                        refresh_token: None,
                        expires_at: None,
                        token_type: Some("Bearer".to_string()),
                        extra: None,
                        source: CredentialSource::EnvVar("CODEX_API_KEY".to_string()),
                    });
                }
            }
            read_codex_credentials_at(&codex_auth_path(codex_path)?)
        }
        OAuthProvider::Google => {
            // env GEMINI_API_KEY > ~/.gemini/oauth_creds.json
            if let Ok(key) = std::env::var("GEMINI_API_KEY") {
                if !key.is_empty() {
                    return Ok(RawCredential {
                        access_token: key,
                        refresh_token: None,
                        expires_at: None,
                        token_type: Some("Bearer".to_string()),
                        extra: None,
                        source: CredentialSource::EnvVar("GEMINI_API_KEY".to_string()),
                    });
                }
            }
            read_gemini_credentials()
        }
        OAuthProvider::Kimi => {
            // env KIMI_API_KEY > ~/.kimi/auth.json
            if let Ok(key) = std::env::var("KIMI_API_KEY") {
                if !key.is_empty() {
                    return Ok(RawCredential {
                        access_token: key,
                        refresh_token: None,
                        expires_at: None,
                        token_type: Some("Bearer".to_string()),
                        extra: None,
                        source: CredentialSource::EnvVar("KIMI_API_KEY".to_string()),
                    });
                }
            }
            read_kimi_credentials()
        }
        OAuthProvider::Github => {
            // env GITHUB_TOKEN > ~/.config/github-copilot/apps.json > hosts.json
            if let Ok(key) = std::env::var("GITHUB_TOKEN") {
                if !key.is_empty() {
                    return Ok(RawCredential {
                        access_token: key,
                        refresh_token: None,
                        expires_at: None,
                        token_type: Some("token".to_string()),
                        extra: None,
                        source: CredentialSource::EnvVar("GITHUB_TOKEN".to_string()),
                    });
                }
            }
            read_copilot_config()
        }
        OAuthProvider::Gitlab => {
            // env GITLAB_TOKEN > GL_TOKEN
            for var in &["GITLAB_TOKEN", "GL_TOKEN"] {
                if let Ok(key) = std::env::var(var) {
                    if !key.is_empty() {
                        return Ok(RawCredential {
                            access_token: key,
                            refresh_token: None,
                            expires_at: None,
                            token_type: Some("Bearer".to_string()),
                            extra: None,
                            source: CredentialSource::EnvVar(var.to_string()),
                        });
                    }
                }
            }
            anyhow::bail!("no GitLab token found. Set GITLAB_TOKEN or GL_TOKEN environment variable, or run `claudex auth login gitlab`")
        }
        OAuthProvider::Qwen => {
            anyhow::bail!("Qwen does not support credential chain loading; use config api_key or device code login")
        }
        // Openai 已被 normalize() 映射到 Chatgpt，此处不可达
        OAuthProvider::Openai => unreachable!("Openai normalized to Chatgpt"),
    }
}

// ── JWT Utilities ────────────────────────────────────────────────────────

/// 从 JWT payload 提取 exp 字段（秒 -> 毫秒）
pub fn extract_jwt_exp(token: &str) -> Option<i64> {
    use base64::Engine;

    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return None;
    }
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(parts[1])
        .ok()?;
    let json: serde_json::Value = serde_json::from_slice(&payload).ok()?;
    json.get("exp").and_then(|v| v.as_i64()).map(|s| s * 1000)
}

/// 从 JWT payload 的嵌套 namespace 中提取字段
pub fn extract_jwt_claim(token: &str, namespace: &str, field: &str) -> Option<String> {
    use base64::Engine;

    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return None;
    }
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(parts[1])
        .ok()?;
    let json: serde_json::Value = serde_json::from_slice(&payload).ok()?;
    json.get(namespace)
        .and_then(|ns| ns.get(field))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// 从 id_token 或 access_token 中提取 ChatGPT account_id
pub fn extract_account_id(token_response: &serde_json::Value) -> Option<String> {
    // 优先从 id_token 提取
    if let Some(id_token) = token_response.get("id_token").and_then(|v| v.as_str()) {
        if let Some(aid) = extract_jwt_claim(
            id_token,
            "https://api.openai.com/auth",
            "chatgpt_account_id",
        ) {
            return Some(aid);
        }
    }
    // 回退到 access_token
    if let Some(access_token) = token_response.get("access_token").and_then(|v| v.as_str()) {
        if let Some(aid) = extract_jwt_claim(
            access_token,
            "https://api.openai.com/auth",
            "chatgpt_account_id",
        ) {
            return Some(aid);
        }
    }
    None
}

// ── Codex credentials atomic write ───────────────────────────────────────

/// 将刷新后的 token 原子写入默认的 ~/.codex/auth.json
pub fn write_codex_credentials_atomic(token: &OAuthToken) -> Result<()> {
    write_codex_credentials_atomic_at(token, &codex_auth_path(None)?)
}

/// 将刷新后的 token 原子写入指定路径（支持每 profile 独立文件）
pub fn write_codex_credentials_atomic_at(token: &OAuthToken, cred_path: &Path) -> Result<()> {
    let codex_dir = cred_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));

    // 读取现有文件保留 auth_mode 等字段
    let mut json: serde_json::Value = if let Ok(content) = std::fs::read_to_string(cred_path) {
        serde_json::from_str(&content).unwrap_or_else(|_| serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    if json.get("tokens").is_none() {
        json["tokens"] = serde_json::json!({});
    }

    let tokens = json.get_mut("tokens").unwrap();
    tokens["access_token"] = serde_json::json!(token.access_token);
    if let Some(ref rt) = token.refresh_token {
        tokens["refresh_token"] = serde_json::json!(rt);
    }

    json["last_refresh"] = serde_json::json!(chrono::Utc::now().to_rfc3339());

    // 原子写入: tmp 文件 + rename
    std::fs::create_dir_all(&codex_dir)?;
    let tmp_path = cred_path.with_extension("tmp");
    std::fs::write(&tmp_path, serde_json::to_string_pretty(&json)?)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp_path, std::fs::Permissions::from_mode(0o600))?;
    }

    std::fs::rename(&tmp_path, cred_path)?;

    tracing::info!("wrote refreshed token to {}", cred_path.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 操作环境变量的测试必须串行执行，避免竞态条件
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn test_extract_jwt_exp() {
        use base64::Engine;
        let payload = serde_json::json!({"exp": 1700000000_i64});
        let payload_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(&payload).unwrap());
        let fake_jwt = format!("eyJhbGciOiJub25lIn0.{payload_b64}.sig");
        assert_eq!(extract_jwt_exp(&fake_jwt), Some(1700000000000_i64));
    }

    #[test]
    fn test_extract_jwt_exp_invalid_token() {
        assert_eq!(extract_jwt_exp("not-a-jwt"), None);
        assert_eq!(extract_jwt_exp("a.b"), None);
    }

    #[test]
    fn test_extract_jwt_claim() {
        use base64::Engine;
        let payload = serde_json::json!({
            "https://api.openai.com/auth": {
                "chatgpt_account_id": "acc-123"
            }
        });
        let payload_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(&payload).unwrap());
        let fake_jwt = format!("eyJhbGciOiJub25lIn0.{payload_b64}.sig");
        assert_eq!(
            extract_jwt_claim(
                &fake_jwt,
                "https://api.openai.com/auth",
                "chatgpt_account_id"
            ),
            Some("acc-123".to_string())
        );
    }

    #[test]
    fn test_extract_account_id_from_id_token() {
        use base64::Engine;
        let payload = serde_json::json!({
            "https://api.openai.com/auth": {
                "chatgpt_account_id": "id-tok-acc"
            }
        });
        let payload_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(&payload).unwrap());
        let fake_jwt = format!("eyJhbGciOiJub25lIn0.{payload_b64}.sig");
        let resp = serde_json::json!({
            "access_token": "opaque",
            "id_token": fake_jwt,
        });
        assert_eq!(extract_account_id(&resp), Some("id-tok-acc".to_string()));
    }

    #[test]
    fn test_credential_chain_env_var() {
        let _lock = ENV_LOCK.lock().unwrap();
        std::env::set_var("ANTHROPIC_API_KEY", "test-key-123");
        let cred = load_credential_chain(&OAuthProvider::Claude).unwrap();
        assert_eq!(cred.access_token, "test-key-123");
        assert!(matches!(cred.source, CredentialSource::EnvVar(_)));
        std::env::remove_var("ANTHROPIC_API_KEY");
    }

    #[test]
    fn test_credential_chain_empty_env_skipped() {
        let _lock = ENV_LOCK.lock().unwrap();
        std::env::set_var("ANTHROPIC_API_KEY", "");
        // 空值应被跳过，如果文件也不存在则报错
        let result = load_credential_chain(&OAuthProvider::Claude);
        // 在 CI 中文件不存在，应该报错
        // 关键是不会因为空 env var 返回空 token
        if let Ok(cred) = &result {
            assert!(!cred.access_token.is_empty());
        }
        std::env::remove_var("ANTHROPIC_API_KEY");
    }

    #[test]
    fn test_normalize_openai_to_chatgpt() {
        assert_eq!(OAuthProvider::Openai.normalize(), OAuthProvider::Chatgpt);
        assert_eq!(OAuthProvider::Claude.normalize(), OAuthProvider::Claude);
        assert_eq!(OAuthProvider::Github.normalize(), OAuthProvider::Github);
        assert_eq!(OAuthProvider::Chatgpt.normalize(), OAuthProvider::Chatgpt);
    }

    #[test]
    fn test_raw_credential_into_oauth_token() {
        let cred = RawCredential {
            access_token: "tok".to_string(),
            refresh_token: Some("ref".to_string()),
            expires_at: Some(1700000000000),
            token_type: Some("Bearer".to_string()),
            extra: None,
            source: CredentialSource::EnvVar("TEST".to_string()),
        };
        let token = cred.into_oauth_token();
        assert_eq!(token.access_token, "tok");
        assert_eq!(token.refresh_token.as_deref(), Some("ref"));
        assert_eq!(token.expires_at, Some(1700000000000));
    }

    #[test]
    fn test_keyring_entry_name() {
        assert_eq!(keyring_entry_name("chatgpt-pro"), "chatgpt-pro-oauth-token");
    }

    #[test]
    fn test_codex_auth_path_default_points_to_codex_dir() {
        let p = codex_auth_path(None).unwrap();
        assert!(p.ends_with(Path::new(".codex").join("auth.json")));
    }

    #[test]
    fn test_codex_auth_path_empty_falls_back_to_default() {
        let default = codex_auth_path(None).unwrap();
        assert_eq!(codex_auth_path(Some("")).unwrap(), default);
        assert_eq!(codex_auth_path(Some("   ")).unwrap(), default);
    }

    #[test]
    fn test_codex_auth_path_custom_absolute_preserved() {
        let custom = if cfg!(windows) {
            r"C:\tmp\auth-work.json"
        } else {
            "/tmp/auth-work.json"
        };
        assert_eq!(
            codex_auth_path(Some(custom)).unwrap(),
            PathBuf::from(custom)
        );
    }

    #[test]
    fn test_codex_auth_path_tilde_is_expanded() {
        let home = dirs::home_dir().unwrap();
        let p = codex_auth_path(Some("~/.codex/auth-work.json")).unwrap();
        assert_eq!(p, home.join(".codex").join("auth-work.json"));
        // 展开后不应再含有 '~'
        assert!(!p.to_string_lossy().contains('~'));
    }

    /// 构造一个带 `chatgpt_account_id` 声明的假 access_token JWT
    fn fake_access_token_with_account(account_id: &str) -> String {
        use base64::Engine;
        let payload = serde_json::json!({
            "exp": 1700000000_i64,
            "https://api.openai.com/auth": { "chatgpt_account_id": account_id },
        });
        let payload_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(&payload).unwrap());
        format!("eyJhbGciOiJub25lIn0.{payload_b64}.sig")
    }

    /// 回归: claudex 写回的 auth.json 只有 tokens.access_token/refresh_token，
    /// 缺 id_token 和 account_id。读取时必须从 access_token JWT 恢复 account_id，
    /// 否则代理会漏发 `ChatGPT-Account-ID` 头导致 Codex 后端拒绝请求。
    #[test]
    fn test_read_codex_recovers_account_id_from_access_token() {
        let access_token = fake_access_token_with_account("acc-from-at");
        let auth_json = serde_json::json!({
            "tokens": {
                "access_token": access_token,
                "refresh_token": "rt-123",
            },
            "last_refresh": "2026-06-02T10:00:00Z",
        });

        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), serde_json::to_string(&auth_json).unwrap()).unwrap();

        let cred = read_codex_credentials_at(tmp.path()).unwrap();
        let account_id = cred
            .extra
            .as_ref()
            .and_then(|e| e.get("account_id"))
            .and_then(|v| v.as_str());
        assert_eq!(account_id, Some("acc-from-at"));
        assert_eq!(cred.refresh_token.as_deref(), Some("rt-123"));
    }

    /// per-profile の auth.json は claudex が新規作成する平文ファイルであり、
    /// 他ユーザーから読めると access_token/refresh_token が漏洩する。
    /// 書き込み後のファイルは 0600（所有者のみ読み書き可）でなければならない。
    #[cfg(unix)]
    #[test]
    fn test_write_codex_credentials_atomic_at_sets_mode_0600() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let cred_path = dir.path().join("auth-work.json");

        let token = OAuthToken {
            access_token: "at-123".to_string(),
            refresh_token: Some("rt-123".to_string()),
            expires_at: None,
            token_type: Some("Bearer".to_string()),
            scopes: None,
            extra: None,
        };

        write_codex_credentials_atomic_at(&token, &cred_path).unwrap();

        let mode = std::fs::metadata(&cred_path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn test_custom_codex_path_differs_from_default() {
        // 隔离多账号的核心保证: 自定义路径绝不等于默认 Codex CLI 文件
        let default = codex_auth_path(None).unwrap();
        let custom = codex_auth_path(Some("~/.codex/auth-work.json")).unwrap();
        assert_ne!(default, custom);
    }
}
