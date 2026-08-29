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

/// 名前付きパイプのサーバが PID ファイルのプロセス自身であることの照合
///
/// 名前の実在だけを見てはならない。パイプ名 `\\.\pipe\claudex-<USERNAME>-proxy` は
/// 秘密ではなく、NPFS ルートには非管理者でも新しい名前を作れるので、他のローカル
/// ユーザーが先に同名パイプを立てられる。名前の実在で通すと `ANTHROPIC_UNIX_SOCKET`
/// と claude.ai のトークンが攻撃者のパイプへ渡る（unix ではソケットがユーザー所有の
/// runtime ディレクトリ配下にあるため成立しない、Windows 固有の経路）。
///
/// そこでクライアント側のハンドルを開き、`GetNamedPipeServerProcessId` が返す
/// サーバ PID が PID ファイルの値と一致した場合だけ true を返す。素性を確かめ
/// られない場合はすべて fail-closed 側（false もしくは Err）に倒す。
///
/// unix の `socket.exists()` と違い、この確認は pipe instance を 1 本消費する
/// （接続してすぐ閉じる）。リスナーは次の instance を先回りで作るので待ちは生じない。
#[cfg(windows)]
pub fn pipe_served_by_proxy() -> Result<bool> {
    use windows_sys::Win32::Foundation::{
        CloseHandle, GetLastError, ERROR_FILE_NOT_FOUND, ERROR_PIPE_BUSY, GENERIC_READ,
        INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_SHARE_NONE, OPEN_EXISTING, SECURITY_ANONYMOUS, SECURITY_SQOS_PRESENT,
    };
    use windows_sys::Win32::System::Pipes::GetNamedPipeServerProcessId;

    let Some(expected_pid) = read_pid()? else {
        return Ok(false);
    };

    let pipe_name = socket_path()?;
    // CreateFileW は NUL 終端の UTF-16 文字列を要求する
    let wide: Vec<u16> = pipe_name
        .to_string_lossy()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    // SECURITY_SQOS_PRESENT | SECURITY_ANONYMOUS は、万一先取りされたパイプに
    // 繋いでしまっても相手のサーバがこちらのトークンを偽装できないようにする
    // （ImpersonateNamedPipeClient 対策）
    // SAFETY: wide は直前に構築した NUL 終端の UTF-16 バッファで呼び出し中は生存する。
    // lpSecurityAttributes と hTemplateFile は null 可
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            GENERIC_READ,
            FILE_SHARE_NONE,
            std::ptr::null(),
            OPEN_EXISTING,
            SECURITY_SQOS_PRESENT | SECURITY_ANONYMOUS,
            std::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        // SAFETY: 直前の CreateFileW 呼び出し直後に取得するエラーコード
        let err = unsafe { GetLastError() };
        return match err {
            ERROR_FILE_NOT_FOUND => Ok(false),
            ERROR_PIPE_BUSY => {
                // 全 instance が使用中でサーバの素性を確かめられない。
                // 先取りされたパイプへトークンを渡す危険を避けるため未提供扱いにする
                tracing::warn!(
                    pipe = %pipe_name.display(),
                    "all named pipe instances are busy; cannot verify the server process, treating the pipe as unavailable"
                );
                Ok(false)
            }
            err => bail!(
                "cannot open named pipe {}: error code {err}",
                pipe_name.display()
            ),
        };
    }

    let mut server_pid: u32 = 0;
    // SAFETY: handle は直前に取得した有効なパイプハンドル、
    // server_pid は書き込み先として有効な u32 バッファ
    let ok = unsafe { GetNamedPipeServerProcessId(handle, &mut server_pid) };
    // CloseHandle が last error を上書きしうるので、閉じる前に読む
    let err = if ok == 0 {
        // SAFETY: 直前の GetNamedPipeServerProcessId 呼び出し直後に取得するエラーコード
        unsafe { GetLastError() }
    } else {
        0
    };
    // SAFETY: CreateFileW で取得したハンドルは全経路で必ず閉じる
    unsafe {
        CloseHandle(handle);
    }
    if ok == 0 {
        bail!(
            "GetNamedPipeServerProcessId failed for {}: error code {err}",
            pipe_name.display()
        );
    }

    if server_pid != expected_pid {
        tracing::warn!(
            pipe = %pipe_name.display(),
            server_pid,
            expected_pid,
            "named pipe is served by a process other than the recorded proxy; refusing to use it"
        );
        return Ok(false);
    }
    Ok(true)
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
