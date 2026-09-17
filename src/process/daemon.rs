use std::path::PathBuf;

use anyhow::{bail, Context, Result};

pub(crate) fn runtime_dir() -> Result<PathBuf> {
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
            crate::proxy::forward::handoff::cleanup();
            eprintln!(
                "notice: the private CA for api.anthropic.com is gone with the proxy. Any Claude Code\nsession still running under claudex will fail TLS from now on — restart those sessions\nafter `claudex proxy start`."
            );
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
                crate::proxy::forward::handoff::cleanup();
                remove_pid()?;
            }
        }
        None => {
            println!("Proxy is not running");
        }
    }
    Ok(())
}
