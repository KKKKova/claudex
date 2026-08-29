use std::path::PathBuf;

use anyhow::{bail, Context, Result};

fn runtime_dir() -> Result<PathBuf> {
    let base = dirs::runtime_dir()
        .or_else(dirs::cache_dir)
        .context("cannot determine runtime directory")?;
    let dir = base.join("claudex");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn pid_file_path() -> Result<PathBuf> {
    Ok(runtime_dir()?.join("proxy.pid"))
}

/// Unix ドメインソケットのパス長上限（sockaddr_un.sun_path）に対する安全域
///
/// macOS は 104 バイト、Linux は 108 バイト。短いほうに余裕を持たせて揃える。
#[cfg(unix)]
const MAX_SOCKET_PATH_LEN: usize = 100;

/// Remote Control 用の Unix ドメインソケットのパス
///
/// Claude Code は `ANTHROPIC_UNIX_SOCKET` が指すソケットへ推論リクエストを流す。
/// runtime ディレクトリが深すぎて上限を超える場合は、一時ディレクトリに退避する。
/// proxy 側と launch 側の双方がこの関数を通るので、判定は一致する。
#[cfg(unix)]
pub fn socket_path() -> Result<PathBuf> {
    let preferred = runtime_dir()?.join("proxy.sock");
    if preferred.as_os_str().len() <= MAX_SOCKET_PATH_LEN {
        return Ok(preferred);
    }

    let uid = unsafe { libc::getuid() };
    let fallback = std::env::temp_dir().join(format!("claudex-{uid}-proxy.sock"));
    tracing::debug!(
        preferred = %preferred.display(),
        fallback = %fallback.display(),
        "runtime dir path exceeds unix socket length limit, falling back"
    );
    Ok(fallback)
}

/// Remote Control 用の名前付きパイプのパス
///
/// Claude Code は `ANTHROPIC_UNIX_SOCKET` 相当の値として渡されたパイプパスへ
/// 推論リクエストを流す。proxy 側と launch 側の双方がこの関数を通るので、
/// パイプ名の解決は一致する。パイプ名前空間はファイルシステムではないため、
/// ディレクトリ作成や存在確認は不要。
#[cfg(windows)]
pub fn socket_path() -> Result<PathBuf> {
    let user = std::env::var("USERNAME").unwrap_or_else(|_| "default".to_string());
    Ok(PathBuf::from(format!(r"\\.\pipe\claudex-{user}-proxy")))
}

pub fn write_pid(pid: u32) -> Result<()> {
    let path = pid_file_path()?;
    std::fs::write(&path, pid.to_string())?;
    tracing::info!(pid, path = %path.display(), "wrote PID file");
    Ok(())
}

pub fn read_pid() -> Result<Option<u32>> {
    let path = pid_file_path()?;
    if !path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(&path)?;
    let pid: u32 = content.trim().parse().context("invalid PID file content")?;
    Ok(Some(pid))
}

pub fn remove_pid() -> Result<()> {
    let path = pid_file_path()?;
    if path.exists() {
        std::fs::remove_file(&path)?;
    }
    Ok(())
}

pub fn is_proxy_running() -> Result<bool> {
    match read_pid()? {
        Some(pid) => {
            #[cfg(unix)]
            {
                let result = unsafe { libc::kill(pid as i32, 0) };
                Ok(result == 0)
            }
            #[cfg(not(unix))]
            {
                let _ = pid;
                Ok(false)
            }
        }
        None => Ok(false),
    }
}

pub fn stop_proxy() -> Result<()> {
    match read_pid()? {
        Some(pid) => {
            if is_proxy_running()? {
                #[cfg(unix)]
                unsafe {
                    libc::kill(pid as i32, libc::SIGTERM);
                }
                println!("Sent SIGTERM to proxy (PID {pid})");
            } else {
                println!("Proxy is not running (stale PID file)");
            }
            remove_pid()?;
            Ok(())
        }
        None => {
            bail!("no proxy PID file found — proxy is not running")
        }
    }
}

pub fn proxy_status() -> Result<()> {
    match read_pid()? {
        Some(pid) => {
            if is_proxy_running()? {
                println!("Proxy is running (PID {pid})");
            } else {
                println!("Proxy is NOT running (stale PID file for PID {pid})");
                remove_pid()?;
            }
        }
        None => {
            println!("Proxy is not running");
        }
    }
    Ok(())
}
