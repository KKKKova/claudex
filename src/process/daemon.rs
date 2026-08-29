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
                    use windows_sys::Win32::Foundation::CloseHandle;
                    use windows_sys::Win32::System::Threading::{
                        OpenProcess, TerminateProcess, PROCESS_TERMINATE,
                    };

                    // SAFETY: dwProcessId に pid を渡すだけで、他プロセスの状態は変更しない
                    let handle = unsafe { OpenProcess(PROCESS_TERMINATE, 0, pid) };
                    if !handle.is_null() {
                        // SAFETY: handle は直前に取得した有効なプロセスハンドル
                        unsafe {
                            TerminateProcess(handle, 0);
                        }
                        // SAFETY: OpenProcess で取得したハンドルは使用後に必ず閉じる
                        unsafe {
                            CloseHandle(handle);
                        }
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

/// 名前付きパイプリスナーの実在確認（unix の `socket.exists()` に相当）
///
/// 接続は消費しない。全 instance が busy でもリスナー自体は実在するので true を返す。
#[cfg(windows)]
pub fn pipe_exists() -> Result<bool> {
    use windows_sys::Win32::Foundation::{GetLastError, ERROR_FILE_NOT_FOUND, ERROR_SEM_TIMEOUT};
    use windows_sys::Win32::System::Pipes::WaitNamedPipeW;

    let pipe_name = socket_path()?;
    // WaitNamedPipeW は NUL 終端の UTF-16 文字列を要求する
    let wide: Vec<u16> = pipe_name
        .to_string_lossy()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    // SAFETY: wide は直前に構築した NUL 終端の UTF-16 バッファで、
    // 呼び出しが終わるまでスコープ内で生存している
    let result = unsafe { WaitNamedPipeW(wide.as_ptr(), 0) };
    if result != 0 {
        return Ok(true);
    }
    // SAFETY: 直前の WaitNamedPipeW 呼び出し直後に取得するエラーコード
    match unsafe { GetLastError() } {
        ERROR_FILE_NOT_FOUND => Ok(false),
        ERROR_SEM_TIMEOUT => Ok(true),
        err => bail!("WaitNamedPipeW failed: error code {err}"),
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
