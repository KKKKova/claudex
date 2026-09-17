use std::process::Command;

use anyhow::{bail, Context, Result};

#[cfg(unix)]
use crate::config::HyperlinksConfig;
use crate::config::{ClaudexConfig, ProfileConfig};
use crate::oauth::{AuthType, OAuthProvider};
#[cfg(unix)]
use crate::terminal;

// hyperlinks_override は PTY モード（#[cfg(unix)] の should_use_pty）でのみ参照される。
// Windows ビルドでは未使用になるが、呼び出し側への波及を避けるため引数はそのまま残す。
#[cfg_attr(not(unix), allow(unused_variables))]
pub fn launch_claude(
    config: &ClaudexConfig,
    profile: &ProfileConfig,
    model_override: Option<&str>,
    extra_args: &[String],
    hyperlinks_override: bool,
) -> Result<()> {
    let proxy_base = format!(
        "http://{}:{}/proxy/{}",
        config.proxy_host, config.proxy_port, profile.name
    );

    let model = model_override
        .map(|m| config.resolve_model(m))
        .unwrap_or_else(|| config.resolve_model(&profile.default_model));

    // 非交互模式检测：含 -p / --print，或首个 arg 不是 flag（裸 prompt）
    let is_noninteractive = extra_args.iter().any(|arg| arg == "-p" || arg == "--print")
        || extra_args.first().is_some_and(|arg| !arg.starts_with('-'));

    let mut cmd = Command::new(&config.claude_binary);

    // 不设 CLAUDE_CONFIG_DIR — 使用全局 ~/.claude，保留用户已有认证和设置。
    // Profile 差异化完全通过环境变量实现。

    let is_claude_subscription = profile.auth_type == AuthType::OAuth
        && profile.oauth_provider == Some(OAuthProvider::Claude);

    if let Some(msg) = redundant_remote_control_warning(profile) {
        eprintln!("warning: {msg}");
    }
    if crate::config::resolve_remote_control(profile).1 {
        eprintln!(
            "warning: `remote_control = true` is deprecated. Use `remote_control_mode = \"proxy\"` instead (see config.example.toml)."
        );
    }

    if is_claude_subscription {
        // Claude subscription：Claude Code 直接使用自身 OAuth
        // 不设 ANTHROPIC_BASE_URL / ANTHROPIC_API_KEY
        if model != profile.default_model {
            cmd.env("ANTHROPIC_MODEL", &model);
        }
    } else if matches!(
        crate::config::resolve_remote_control(profile).0,
        crate::config::RemoteControlMode::Proxy
    ) {
        apply_forward_proxy_env(&mut cmd, profile, &model)?;
    } else {
        // 标准代理流程（Gateway 模式）
        // 用 ANTHROPIC_AUTH_TOKEN（发 Authorization: Bearer header）而非 ANTHROPIC_API_KEY（发 X-Api-Key header）
        // 避免与 claude.ai OAuth token 产生 "Auth conflict"
        cmd.env("ANTHROPIC_BASE_URL", &proxy_base)
            .env("ANTHROPIC_AUTH_TOKEN", "claudex-passthrough")
            .env("ANTHROPIC_MODEL", &model);
    }

    if !profile.custom_headers.is_empty() {
        let headers: Vec<String> = profile
            .custom_headers
            .iter()
            .map(|(k, v)| format!("{k}:{v}"))
            .collect();
        cmd.env("ANTHROPIC_CUSTOM_HEADERS", headers.join(","));
    }

    // 模型 slot 映射 → Claude Code 的 /model 切换
    if let Some(ref h) = profile.models.haiku {
        cmd.env("ANTHROPIC_DEFAULT_HAIKU_MODEL", h);
    }
    if let Some(ref s) = profile.models.sonnet {
        cmd.env("ANTHROPIC_DEFAULT_SONNET_MODEL", s);
    }
    if let Some(ref o) = profile.models.opus {
        cmd.env("ANTHROPIC_DEFAULT_OPUS_MODEL", o);
    }
    if let Some(ref fb) = profile.models.fable {
        cmd.env("ANTHROPIC_DEFAULT_FABLE_MODEL", fb);
    }

    for (k, v) in &profile.extra_env {
        cmd.env(k, v);
    }

    // 自动禁用 Chrome 集成（除非用户显式传了 --chrome）
    if !extra_args.iter().any(|a| a == "--chrome") {
        cmd.arg("--no-chrome");
    }

    // 附加 config.default_args（跳过已在 extra_args 中出现的项，避免重复传参）
    for arg in &config.default_args {
        if !extra_args.contains(arg) {
            cmd.arg(arg);
        }
    }

    cmd.args(extra_args);

    tracing::info!(
        profile = %profile.name,
        model = %model,
        proxy = %proxy_base,
        noninteractive = %is_noninteractive,
        "launching claude"
    );

    // PTY mode (Unix only): 非交互模式跳过 PTY
    #[cfg(unix)]
    let use_pty = !is_noninteractive && should_use_pty(&config.hyperlinks, hyperlinks_override);
    #[cfg(not(unix))]
    let use_pty = false;

    // resume_session_id は PTY モード（#[cfg(unix)]）でのみ書き換わる。
    #[cfg(unix)]
    let mut resume_session_id: Option<String> = None;
    #[cfg(not(unix))]
    let resume_session_id: Option<String> = None;

    if use_pty {
        #[cfg(unix)]
        {
            tracing::info!("hyperlinks enabled, using PTY proxy mode");
            let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("/"));
            resume_session_id = terminal::pty::spawn_with_pty(cmd, cwd)?;
        }
    } else {
        let mut child = cmd.spawn().context("failed to execute claude binary")?;

        // 转发 SIGINT/SIGTERM 到子进程
        #[cfg(unix)]
        unsafe {
            libc::signal(libc::SIGINT, libc::SIG_IGN);
        }

        let status = child.wait().context("failed to wait for claude")?;

        #[cfg(unix)]
        unsafe {
            libc::signal(libc::SIGINT, libc::SIG_DFL);
        }

        if !status.success() {
            #[cfg(unix)]
            {
                use std::os::unix::process::ExitStatusExt;
                if status.signal().is_some() {
                    std::process::exit(128 + status.signal().unwrap());
                }
            }
            bail!("claude exited with status: {}", status);
        }
    }

    // 追加 claudex resume 命令提示
    if let Some(session_id) = resume_session_id {
        print_claudex_resume_hint(&profile.name, &session_id, extra_args);
    }

    Ok(())
}

/// Claude subscription profile に `remote_control = true` が付いている冗長な組み合わせを検出する
///
/// この組み合わせは現状でも害はない（`is_claude_subscription` の分岐がプロキシを
/// 迂回するため、`remote_control` フラグの有無に関わらず Remote Control は素で動く）。
/// ただし利用者の意図が「推論を第二アカウントへ向けたい」であれば、代わりに
/// `provider_type = "DirectAnthropic"` + `api_key` に第二アカウントのトークンを置く
/// 目標2の構成にする必要があるため、案内だけ出して起動は止めない。
fn redundant_remote_control_warning(profile: &ProfileConfig) -> Option<String> {
    let is_claude_subscription = profile.auth_type == AuthType::OAuth
        && profile.oauth_provider == Some(OAuthProvider::Claude);

    let is_remote_control_proxy =
        crate::config::resolve_remote_control(profile).0 == crate::config::RemoteControlMode::Proxy;

    if is_claude_subscription && is_remote_control_proxy {
        Some(
            "remote_control has no effect on a Claude subscription profile (it bypasses the \
             proxy and Remote Control works natively). To route inference to a second account, \
             use provider_type = \"DirectAnthropic\" with base_url = \"https://api.anthropic.com\", \
             the second account's token in api_key, and remote_control_mode = \"proxy\". See config.example.toml."
                .to_string(),
        )
    } else {
        None
    }
}

/// forward proxy 方式で Remote Control を有効にした状態で Claude Code を起動するための
/// 環境変数を組む
///
/// 旧 Unix ドメインソケット方式（`apply_remote_control_env`）を置き換える。proxy が
/// 別プロセスとして立てた forward proxy へ `HTTPS_PROXY` で接続し、CONNECT の
/// `Proxy-Authorization` で認証する。TLS 終端は forward proxy 側が私設 CA で行うため、
/// `NODE_EXTRA_CA_CERTS` でその CA を Claude Code（Node.js）に信頼させる。
fn apply_forward_proxy_env(cmd: &mut Command, profile: &ProfileConfig, model: &str) -> Result<()> {
    if !crate::process::daemon::is_proxy_running()? {
        bail!("claudex proxy is not running. Start it first: claudex proxy start");
    }

    let handoff = crate::proxy::forward::handoff::read().map_err(|e| {
        anyhow::anyhow!(
            "forward proxy handoff file is missing or unreadable ({e}). Restart the proxy: claudex proxy start"
        )
    })?;

    let ca_pem_path = crate::proxy::forward::handoff::ca_pem_path()?;
    if !ca_pem_path.exists() {
        bail!(
            "forward proxy CA certificate is missing at {}. Restart the proxy: claudex proxy start",
            ca_pem_path.display()
        );
    }

    let session = crate::oauth::source::read_claude_ai_session().context(
        "Remote Control requires a claude.ai login. Run `claude auth login` (in plain Claude Code) first",
    )?;

    check_session_lifetime(&session)?;

    eprintln!(
        "notice: claudex terminates TLS for api.anthropic.com in this session using a\n\
         private CA held only in the proxy process. Restarting the proxy invalidates\n\
         that CA — restart this session too if the proxy restarts."
    );

    let parent_no_proxy = std::env::var("NO_PROXY")
        .ok()
        .or_else(|| std::env::var("no_proxy").ok());

    for (key, value) in forward_proxy_env(
        &handoff,
        &ca_pem_path,
        &profile.name,
        model,
        &session,
        parent_no_proxy.as_deref(),
    ) {
        cmd.env(key, value);
    }

    // 親シェルに残っていると旧方式の設定や API キー認証と衝突しうるので、明示的に落とす
    cmd.env_remove("ANTHROPIC_BASE_URL")
        .env_remove("ANTHROPIC_UNIX_SOCKET")
        .env_remove("ANTHROPIC_AUTH_TOKEN")
        .env_remove("ANTHROPIC_API_KEY");

    tracing::info!(
        profile = %profile.name,
        port = handoff.port,
        "forward proxy remote control enabled"
    );

    Ok(())
}

/// 残り時間がこれを切ったら警告する（秒）
const SESSION_EXPIRY_WARN_SECS: i64 = 60 * 60;

/// トークンの残り寿命を確認する
///
/// Claude Code は `CLAUDE_CODE_OAUTH_TOKEN` を起動時にしか読まず、
/// refresh token も持たないため、セッション中に差し替える手段がない。
/// 起動前に判断できることだけを済ませる。
fn check_session_lifetime(session: &crate::oauth::source::ClaudeAiSession) -> Result<()> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    match session.remaining_secs(now) {
        Some(remaining) if remaining <= 0 => {
            bail!("claude.ai token has expired. Run plain `claude` once to refresh it, then retry")
        }
        Some(remaining) if remaining < SESSION_EXPIRY_WARN_SECS => {
            eprintln!(
                "warning: claude.ai token expires in {} minutes. Remote Control will stop working \
                 then, and the token cannot be refreshed mid-session — restart the session to renew.",
                remaining / 60
            );
        }
        _ => {}
    }

    Ok(())
}

/// forward proxy モードで Claude Code に渡す環境変数（純粋関数、テスト用）
///
/// `url::Url` の `set_username` / `set_password` / `set_port` は `Result<(), ()>` を返すが、
/// 失敗するのは URL が host を持たない（"cannot-be-a-base"）か、スキームがユーザー情報や
/// ポートを許さない場合だけ。ここでは固定の `http://127.0.0.1` を土台にしており、host も
/// スキームも常に条件を満たすため、これらの呼び出しは失敗し得ない。呼び出し元が返す
/// 型が `Vec`（`Result` ではない）なので、`let _ =` で結果を捨てる。
fn forward_proxy_env(
    handoff: &crate::proxy::forward::handoff::ForwardHandoff,
    ca_pem_path: &std::path::Path,
    profile_name: &str,
    model: &str,
    session: &crate::oauth::source::ClaudeAiSession,
    parent_no_proxy: Option<&str>,
) -> Vec<(String, String)> {
    // 生产代码では unwrap()/expect() を使わない規約のため、ハードコードされたリテラルの
    // 解析失敗（実際には起こり得ない）は unreachable! で表す
    let mut proxy_url = match url::Url::parse("http://127.0.0.1") {
        Ok(u) => u,
        Err(_) => unreachable!("hardcoded base URL is always valid"),
    };
    let _ = proxy_url.set_username(profile_name);
    let _ = proxy_url.set_password(Some(&handoff.secret));
    let _ = proxy_url.set_port(Some(handoff.port));
    let https_proxy = proxy_url.to_string();

    let no_proxy = match parent_no_proxy {
        Some(v) if !v.is_empty() => format!("{v},localhost,127.0.0.1,::1"),
        _ => "localhost,127.0.0.1,::1".to_string(),
    };

    vec![
        ("HTTPS_PROXY".to_string(), https_proxy.clone()),
        ("https_proxy".to_string(), https_proxy),
        ("NO_PROXY".to_string(), no_proxy.clone()),
        ("no_proxy".to_string(), no_proxy),
        (
            "NODE_EXTRA_CA_CERTS".to_string(),
            ca_pem_path.display().to_string(),
        ),
        (
            "CLAUDE_CODE_OAUTH_TOKEN".to_string(),
            session.access_token.clone(),
        ),
        (
            "CLAUDE_CODE_OAUTH_SCOPES".to_string(),
            session.scopes.join(" "),
        ),
        ("ANTHROPIC_MODEL".to_string(), model.to_string()),
    ]
}

/// 在 Claude Code 退出后追加 claudex resume 命令提示
fn print_claudex_resume_hint(profile_name: &str, session_id: &str, extra_args: &[String]) {
    let hint = build_resume_hint(profile_name, session_id, extra_args);
    eprintln!("\nResume this session with claudex:\n  {hint}");
}

/// 构造 claudex resume 命令字符串（纯函数，便于测试）
fn build_resume_hint(profile_name: &str, session_id: &str, extra_args: &[String]) -> String {
    // 过滤掉原始 extra_args 中的 --resume 及其值参数
    let mut args_clean: Vec<&str> = Vec::new();
    let mut skip_next = false;
    for arg in extra_args {
        if skip_next {
            skip_next = false;
            continue;
        }
        if arg == "--resume" {
            skip_next = true;
            continue;
        }
        args_clean.push(arg);
    }

    let args_str = if args_clean.is_empty() {
        String::new()
    } else {
        format!(" {}", args_clean.join(" "))
    };

    format!("claudex run {profile_name} --resume {session_id}{args_str}")
}

/// Decide whether to use PTY mode based on config + CLI flag.
#[cfg(unix)]
fn should_use_pty(config_hyperlinks: &HyperlinksConfig, cli_override: bool) -> bool {
    if cli_override {
        return true;
    }

    match config_hyperlinks {
        HyperlinksConfig::Enabled => true,
        HyperlinksConfig::Disabled => false,
        HyperlinksConfig::Auto => terminal::detect::terminal_supports_hyperlinks(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_resume_hint_no_extra_args() {
        let hint = build_resume_hint("codex-sub", "abc-123", &[]);
        assert_eq!(hint, "claudex run codex-sub --resume abc-123");
    }

    #[test]
    fn test_build_resume_hint_with_extra_args() {
        let args = vec![
            "--dangerously-skip-permissions".to_string(),
            "--verbose".to_string(),
        ];
        let hint = build_resume_hint("codex-sub", "abc-123", &args);
        assert_eq!(
            hint,
            "claudex run codex-sub --resume abc-123 --dangerously-skip-permissions --verbose"
        );
    }

    #[test]
    fn test_build_resume_hint_filters_existing_resume() {
        let args = vec![
            "--resume".to_string(),
            "old-session-id".to_string(),
            "--dangerously-skip-permissions".to_string(),
        ];
        let hint = build_resume_hint("codex-sub", "new-session-id", &args);
        assert_eq!(
            hint,
            "claudex run codex-sub --resume new-session-id --dangerously-skip-permissions"
        );
    }

    #[test]
    fn test_build_resume_hint_resume_at_end() {
        let args = vec![
            "--verbose".to_string(),
            "--resume".to_string(),
            "old-id".to_string(),
        ];
        let hint = build_resume_hint("my-profile", "new-id", &args);
        assert_eq!(hint, "claudex run my-profile --resume new-id --verbose");
    }

    #[test]
    fn test_build_resume_hint_resume_only() {
        let args = vec!["--resume".to_string(), "old-id".to_string()];
        let hint = build_resume_hint("p", "new-id", &args);
        assert_eq!(hint, "claudex run p --resume new-id");
    }

    fn sample_handoff() -> crate::proxy::forward::handoff::ForwardHandoff {
        crate::proxy::forward::handoff::ForwardHandoff {
            port: 13457,
            secret: "s3cret".to_string(),
        }
    }

    fn sample_session() -> crate::oauth::source::ClaudeAiSession {
        crate::oauth::source::ClaudeAiSession {
            access_token: "sk-ant-oat-example".to_string(),
            scopes: vec!["user:profile".to_string(), "user:inference".to_string()],
            expires_at: None,
        }
    }

    #[test]
    fn test_forward_proxy_env_contains_expected_keys() {
        let env: std::collections::HashMap<_, _> = forward_proxy_env(
            &sample_handoff(),
            std::path::Path::new("/tmp/forward-ca.pem"),
            "codex-sub",
            "gpt-5.6-sol",
            &sample_session(),
            None,
        )
        .into_iter()
        .collect();

        let expected_keys: std::collections::HashSet<&str> = [
            "HTTPS_PROXY",
            "https_proxy",
            "NO_PROXY",
            "no_proxy",
            "NODE_EXTRA_CA_CERTS",
            "CLAUDE_CODE_OAUTH_TOKEN",
            "CLAUDE_CODE_OAUTH_SCOPES",
            "ANTHROPIC_MODEL",
        ]
        .into_iter()
        .collect();
        let actual_keys: std::collections::HashSet<&str> = env.keys().map(|s| s.as_str()).collect();
        assert_eq!(actual_keys, expected_keys);

        // 旧ソケット方式・API キー系は forward proxy 方式では一切渡さない
        assert!(!env.contains_key("ANTHROPIC_BASE_URL"));
        assert!(!env.contains_key("ANTHROPIC_UNIX_SOCKET"));
        assert!(!env.contains_key("ANTHROPIC_API_KEY"));
        assert!(!env.contains_key("ANTHROPIC_AUTH_TOKEN"));
    }

    #[test]
    fn test_forward_proxy_env_https_proxy_url() {
        let handoff = crate::proxy::forward::handoff::ForwardHandoff {
            port: 13457,
            secret: "s3cret".to_string(),
        };
        let env: std::collections::HashMap<_, _> = forward_proxy_env(
            &handoff,
            std::path::Path::new("/tmp/forward-ca.pem"),
            "grok",
            "gpt-5.6-sol",
            &sample_session(),
            None,
        )
        .into_iter()
        .collect();

        assert_eq!(env["HTTPS_PROXY"], "http://grok:s3cret@127.0.0.1:13457/");
        assert_eq!(env["https_proxy"], "http://grok:s3cret@127.0.0.1:13457/");
    }

    #[test]
    fn test_forward_proxy_env_percent_encodes_username() {
        let env: std::collections::HashMap<_, _> = forward_proxy_env(
            &sample_handoff(),
            std::path::Path::new("/tmp/forward-ca.pem"),
            "my:profile",
            "gpt-5.6-sol",
            &sample_session(),
            None,
        )
        .into_iter()
        .collect();

        assert!(env["HTTPS_PROXY"].contains("my%3Aprofile"));
    }

    #[test]
    fn test_forward_proxy_env_no_proxy_appends_parent() {
        let env: std::collections::HashMap<_, _> = forward_proxy_env(
            &sample_handoff(),
            std::path::Path::new("/tmp/forward-ca.pem"),
            "codex-sub",
            "gpt-5.6-sol",
            &sample_session(),
            Some("corp.example"),
        )
        .into_iter()
        .collect();

        assert_eq!(env["NO_PROXY"], "corp.example,localhost,127.0.0.1,::1");
        assert_eq!(env["no_proxy"], "corp.example,localhost,127.0.0.1,::1");
    }

    #[test]
    fn test_forward_proxy_env_no_proxy_without_parent() {
        let env: std::collections::HashMap<_, _> = forward_proxy_env(
            &sample_handoff(),
            std::path::Path::new("/tmp/forward-ca.pem"),
            "codex-sub",
            "gpt-5.6-sol",
            &sample_session(),
            None,
        )
        .into_iter()
        .collect();

        assert_eq!(env["NO_PROXY"], "localhost,127.0.0.1,::1");
        assert_eq!(env["no_proxy"], "localhost,127.0.0.1,::1");
    }

    fn session_expiring_at(expires_at: Option<i64>) -> crate::oauth::source::ClaudeAiSession {
        crate::oauth::source::ClaudeAiSession {
            access_token: "t".to_string(),
            scopes: vec!["user:inference".to_string()],
            expires_at,
        }
    }

    #[test]
    fn test_check_session_lifetime_rejects_expired() {
        let past = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
            - 60;
        assert!(check_session_lifetime(&session_expiring_at(Some(past))).is_err());
    }

    #[test]
    fn test_check_session_lifetime_accepts_fresh_and_unknown() {
        let future = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
            + 8 * 60 * 60;
        assert!(check_session_lifetime(&session_expiring_at(Some(future))).is_ok());
        // 期限が読めないケースは通す（判断材料がないだけで、失効しているとは限らない）
        assert!(check_session_lifetime(&session_expiring_at(None)).is_ok());
    }

    #[test]
    fn test_redundant_remote_control_warning_subscription_with_remote_control() {
        let profile = ProfileConfig {
            auth_type: AuthType::OAuth,
            oauth_provider: Some(OAuthProvider::Claude),
            remote_control: true,
            ..Default::default()
        };
        assert!(redundant_remote_control_warning(&profile).is_some());
    }

    #[test]
    fn test_redundant_remote_control_warning_subscription_without_remote_control() {
        let profile = ProfileConfig {
            auth_type: AuthType::OAuth,
            oauth_provider: Some(OAuthProvider::Claude),
            remote_control: false,
            ..Default::default()
        };
        assert!(redundant_remote_control_warning(&profile).is_none());
    }

    #[test]
    fn test_redundant_remote_control_warning_goal2_profile() {
        // 目標2形式: DirectAnthropic + api_key に第二アカウントのトークン + remote_control = true
        let profile = ProfileConfig {
            provider_type: crate::config::ProviderType::DirectAnthropic,
            base_url: "https://api.anthropic.com".to_string(),
            api_key: "sk-ant-oat-second-account".to_string(),
            remote_control: true,
            ..Default::default()
        };
        assert!(redundant_remote_control_warning(&profile).is_none());
    }

    #[test]
    fn test_redundant_remote_control_warning_normal_api_key_profile() {
        let profile = ProfileConfig {
            api_key: "sk-ant-api-example".to_string(),
            remote_control: false,
            ..Default::default()
        };
        assert!(redundant_remote_control_warning(&profile).is_none());
    }

    // ───── remote_control_mode（新キー）版 ─────

    #[test]
    fn test_redundant_remote_control_warning_subscription_with_remote_control_mode() {
        let profile = ProfileConfig {
            auth_type: AuthType::OAuth,
            oauth_provider: Some(OAuthProvider::Claude),
            remote_control_mode: Some(crate::config::RemoteControlMode::Proxy),
            ..Default::default()
        };
        assert!(redundant_remote_control_warning(&profile).is_some());
    }

    #[test]
    fn test_redundant_remote_control_warning_subscription_without_remote_control_mode() {
        let profile = ProfileConfig {
            auth_type: AuthType::OAuth,
            oauth_provider: Some(OAuthProvider::Claude),
            remote_control_mode: Some(crate::config::RemoteControlMode::Off),
            ..Default::default()
        };
        assert!(redundant_remote_control_warning(&profile).is_none());
    }

    #[test]
    fn test_redundant_remote_control_warning_goal2_profile_remote_control_mode() {
        // 目標2形式: DirectAnthropic + api_key に第二アカウントのトークン + remote_control_mode = "proxy"
        let profile = ProfileConfig {
            provider_type: crate::config::ProviderType::DirectAnthropic,
            base_url: "https://api.anthropic.com".to_string(),
            api_key: "sk-ant-oat-second-account".to_string(),
            remote_control_mode: Some(crate::config::RemoteControlMode::Proxy),
            ..Default::default()
        };
        assert!(redundant_remote_control_warning(&profile).is_none());
    }

    #[test]
    fn test_redundant_remote_control_warning_normal_api_key_profile_remote_control_mode() {
        let profile = ProfileConfig {
            api_key: "sk-ant-api-example".to_string(),
            remote_control_mode: Some(crate::config::RemoteControlMode::Off),
            ..Default::default()
        };
        assert!(redundant_remote_control_warning(&profile).is_none());
    }
}
