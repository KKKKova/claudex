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

    if is_claude_subscription {
        // Claude subscription：Claude Code 直接使用自身 OAuth
        // 不设 ANTHROPIC_BASE_URL / ANTHROPIC_API_KEY
        if model != profile.default_model {
            cmd.env("ANTHROPIC_MODEL", &model);
        }
    } else if profile.remote_control {
        apply_remote_control_env(&mut cmd, profile, &model)?;
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

/// Remote Control を有効にした状態で Claude Code を起動するための環境変数を組む
///
/// Claude Code は Remote Control の可否を二つの条件で判定する（2.1.220 で確認）。
///
/// 1. 接続先が api.anthropic.com であること。`ANTHROPIC_BASE_URL` の host しか
///    見ていないので、Unix ソケットに落とすときは host だけ合わせれば通る。
/// 2. claude.ai のログインが API キー認証より優先されていること。
///    `ANTHROPIC_AUTH_TOKEN` や `ANTHROPIC_API_KEY` があると API キー認証と
///    見なされるため、代わりに `CLAUDE_CODE_OAUTH_TOKEN` を渡す。
///
/// `ANTHROPIC_UNIX_SOCKET` が設定されていると推論リクエストだけがソケットへ流れ、
/// claude.ai のブリッジ通信は通常のネットワークに出る。これで推論を第三者
/// プロバイダに向けたまま Remote Control が使える。
fn apply_remote_control_env(cmd: &mut Command, profile: &ProfileConfig, model: &str) -> Result<()> {
    let socket = crate::process::daemon::socket_path()?;

    // unix: ソケットファイルの実在確認
    #[cfg(unix)]
    if !socket.exists() {
        bail!(
            "proxy socket not found at {}. Start the proxy first: claudex proxy start",
            socket.display()
        );
    }

    // Windows: 2段ガード。(a) プロセスの生存確認 → (b) ソケットの実在確認。
    // 実在判定に `exists()` は使わない。Windows の AF_UNIX ソケットはリパースポイントで
    // `exists()` が偽陰性を返しうるため、`symlink_metadata()` で判定する。
    #[cfg(windows)]
    {
        if !crate::process::daemon::is_proxy_running()? {
            bail!(
                "proxy is not running (no live process for the PID file). Start the proxy first: claudex proxy start"
            );
        }
        if socket.symlink_metadata().is_err() {
            bail!(
                "proxy socket not found at {}. The proxy may predate the AF_UNIX rework — restart it: claudex proxy start",
                socket.display()
            );
        }
    }

    let session = crate::oauth::source::read_claude_ai_session().context(
        "Remote Control requires a claude.ai login. Run `claude auth login` (in plain Claude Code) first",
    )?;

    check_session_lifetime(&session)?;

    for (key, value) in remote_control_env(&socket, &profile.name, model, &session) {
        cmd.env(key, value);
    }

    // 親シェルに残っていると API キー認証と判定されるので、明示的に落とす
    cmd.env_remove("ANTHROPIC_AUTH_TOKEN")
        .env_remove("ANTHROPIC_API_KEY");

    tracing::info!(
        profile = %profile.name,
        socket = %socket.display(),
        scopes = %session.scopes.join(","),
        "remote control mode enabled"
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

/// Remote Control モードで Claude Code に渡す環境変数（純粋関数、テスト用）
fn remote_control_env(
    socket: &std::path::Path,
    profile_name: &str,
    model: &str,
    session: &crate::oauth::source::ClaudeAiSession,
) -> Vec<(String, String)> {
    vec![
        (
            "ANTHROPIC_UNIX_SOCKET".to_string(),
            socket.display().to_string(),
        ),
        // host だけが判定対象なので、profile のパスはそのまま保てる
        (
            "ANTHROPIC_BASE_URL".to_string(),
            format!("http://api.anthropic.com/proxy/{profile_name}"),
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

    #[test]
    fn test_remote_control_env() {
        let session = crate::oauth::source::ClaudeAiSession {
            access_token: "sk-ant-oat-example".to_string(),
            scopes: vec!["user:profile".to_string(), "user:inference".to_string()],
            expires_at: None,
        };
        let env: std::collections::HashMap<_, _> = remote_control_env(
            std::path::Path::new("/tmp/claudex-proxy.sock"),
            "codex-sub",
            "gpt-5.6-sol",
            &session,
        )
        .into_iter()
        .collect();

        // host が api.anthropic.com でないと Claude Code が Remote Control を出さない
        assert_eq!(
            env["ANTHROPIC_BASE_URL"],
            "http://api.anthropic.com/proxy/codex-sub"
        );
        assert_eq!(env["ANTHROPIC_UNIX_SOCKET"], "/tmp/claudex-proxy.sock");
        assert_eq!(env["CLAUDE_CODE_OAUTH_TOKEN"], "sk-ant-oat-example");
        assert_eq!(
            env["CLAUDE_CODE_OAUTH_SCOPES"],
            "user:profile user:inference"
        );
        assert_eq!(env["ANTHROPIC_MODEL"], "gpt-5.6-sol");
        // API キー系を渡すと API キー認証と判定されて Remote Control が落ちる
        assert!(!env.contains_key("ANTHROPIC_AUTH_TOKEN"));
        assert!(!env.contains_key("ANTHROPIC_API_KEY"));
    }

    #[test]
    fn test_remote_control_env_windows_path_passthrough() {
        let session = crate::oauth::source::ClaudeAiSession {
            access_token: "sk-ant-oat-example".to_string(),
            scopes: vec!["user:inference".to_string()],
            expires_at: None,
        };
        let socket = std::path::PathBuf::from(r"C:\Users\u\AppData\Local\claudex\proxy.sock");
        let env: std::collections::HashMap<_, _> =
            remote_control_env(&socket, "codex-sub", "gpt-5.6-sol", &session)
                .into_iter()
                .collect();

        // Windows 形式のパスもそのまま同じ文字列で渡る（変換や正規化はしない）
        assert_eq!(
            env["ANTHROPIC_UNIX_SOCKET"],
            r"C:\Users\u\AppData\Local\claudex\proxy.sock"
        );
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
}
