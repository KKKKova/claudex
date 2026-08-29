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

pub(crate) fn pid_file_path() -> Result<PathBuf> {
    Ok(runtime_dir()?.join("proxy.pid"))
}

/// Unix ドメインソケットのパス長上限（sockaddr_un.sun_path）に対する安全域
///
/// macOS は 104 バイト、Linux は 108 バイト、Windows の AF_UNIX（afunix.h の
/// UNIX_PATH_MAX）も 108 バイト。もっとも短いものに余裕を持たせて揃える。
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

/// Remote Control 用の Unix ドメインソケットのパス（Windows 版）
///
/// `sun_path` は Windows でも108バイト上限（`afunix.h` の `UNIX_PATH_MAX`）。
/// 超過は理由の見えない `FailedToOpenSocket` になるため、ここで明示エラーにする。
/// proxy 側と launch 側の双方がこの関数を通るので、判定は一致する。
#[cfg(windows)]
pub fn socket_path() -> Result<PathBuf> {
    let preferred = runtime_dir()?.join("proxy.sock");
    if preferred.as_os_str().len() <= MAX_SOCKET_PATH_LEN {
        return Ok(preferred);
    }

    let fallback = dirs::home_dir()
        .context("cannot determine home directory")?
        .join(".claudex")
        .join("p.sock");
    if fallback.as_os_str().len() <= MAX_SOCKET_PATH_LEN {
        tracing::debug!(
            preferred = %preferred.display(),
            fallback = %fallback.display(),
            "runtime dir path exceeds unix socket length limit, falling back"
        );
        return Ok(fallback);
    }

    bail!(
        "socket path {} exceeds the {MAX_SOCKET_PATH_LEN}-byte AF_UNIX limit; \
         cannot start remote control on this machine",
        fallback.display()
    );
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
            #[cfg(windows)]
            {
                use windows_sys::Win32::Foundation::{
                    CloseHandle, GetLastError, ERROR_ACCESS_DENIED, STILL_ACTIVE,
                };
                use windows_sys::Win32::System::Threading::{
                    GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
                };

                // SAFETY: dwProcessId に pid を渡すだけで、他プロセスの状態は変更しない
                let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
                if handle.is_null() {
                    // ハンドル取得失敗。ACCESS_DENIED は他ユーザー所有プロセス等で
                    // 実際には生存しているケースなので生存扱いにする（unix の
                    // kill(pid, 0) が EPERM を「生存」とみなすのと同じ限界）
                    let err = unsafe { GetLastError() };
                    return Ok(err == ERROR_ACCESS_DENIED);
                }
                let mut exit_code: u32 = 0;
                // SAFETY: handle は直前に取得した有効なプロセスハンドル、
                // exit_code は書き込み先として有効な u32 バッファ
                let ok = unsafe { GetExitCodeProcess(handle, &mut exit_code) };
                // SAFETY: OpenProcess で取得したハンドルは使用後に必ず閉じる
                unsafe {
                    CloseHandle(handle);
                }
                if ok == 0 {
                    return Ok(false);
                }
                Ok(exit_code == STILL_ACTIVE as u32)
            }
            #[cfg(not(any(unix, windows)))]
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
                {
                    unsafe {
                        libc::kill(pid as i32, libc::SIGTERM);
                    }
                    println!("Sent SIGTERM to proxy (PID {pid})");
                }
                // Windows には SIGTERM 相当の、コンソールプロセス外から捕捉可能な
                // graceful 停止手段が無いため、TerminateProcess で in-flight
                // リクエストごと即時終了する（unix の SIGTERM とは非対称）
                #[cfg(windows)]
                {
                    use windows_sys::Win32::Foundation::{CloseHandle, GetLastError};
                    use windows_sys::Win32::System::Threading::{
                        OpenProcess, TerminateProcess, PROCESS_TERMINATE,
                    };

                    // SAFETY: dwProcessId に pid を渡すだけで、他プロセスの状態は変更しない
                    let handle = unsafe { OpenProcess(PROCESS_TERMINATE, 0, pid) };
                    if handle.is_null() {
                        // is_proxy_running() は ACCESS_DENIED を生存扱いにするので、
                        // 「生きていると判定 → 殺せない」が起こりうる。ここで成功を
                        // 名乗ると PID ファイルまで消えて、掴まれたポートとパイプ名が
                        // 原因不明の連鎖障害になる
                        // SAFETY: 直前の OpenProcess 呼び出し直後に取得するエラーコード
                        let err = unsafe { GetLastError() };
                        bail!("cannot open proxy process (PID {pid}) to terminate it: error code {err}. PID file kept");
                    }
                    // SAFETY: handle は直前に取得した有効なプロセスハンドル
                    let ok = unsafe { TerminateProcess(handle, 0) };
                    // CloseHandle が last error を上書きしうるので、閉じる前に読む
                    let err = if ok == 0 {
                        // SAFETY: 直前の TerminateProcess 呼び出し直後に取得するエラーコード
                        unsafe { GetLastError() }
                    } else {
                        0
                    };
                    // SAFETY: OpenProcess で取得したハンドルは全経路で必ず閉じる
                    unsafe {
                        CloseHandle(handle);
                    }
                    if ok == 0 {
                        bail!("TerminateProcess failed for proxy (PID {pid}): error code {err}. PID file kept");
                    }
                    println!("Terminated proxy (PID {pid})");
                }
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
