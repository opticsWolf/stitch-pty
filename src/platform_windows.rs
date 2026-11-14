//! Windows PTY implementation using ConPTY (dynamically loaded) and tokio NamedPipes
//!
//! Improvements from portable-pty:
//! - Dynamic ConPTY loading for graceful degradation on older Windows
//! - Proper Windows command line quoting (ArgvQuote algorithm)
//! - ChildKiller trait for deadlock-free kill/wait
//! - Tokio NamedPipeServer connect() to arm mio's IOCP pump

use crate::errors::{PtyErrorKind, PtyResult};
use crate::platform::{ChildBackend, ChildKiller, ProcessExit, PtyBackend};
use crate::winsize::Winsize;
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
use tokio::sync::{Mutex, watch};

use windows::Win32::Foundation::{
    CloseHandle, DUPLICATE_SAME_ACCESS, DuplicateHandle, GENERIC_READ, GENERIC_WRITE, HANDLE,
    INVALID_HANDLE_VALUE, WAIT_OBJECT_0,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::Console::{COORD, PSEUDOCONSOLE_INHERIT_CURSOR};
use windows::Win32::System::Threading::{
    CREATE_UNICODE_ENVIRONMENT, CreateProcessW, DeleteProcThreadAttributeList,
    EXTENDED_STARTUPINFO_PRESENT, GetCurrentProcess, GetExitCodeProcess, GetProcessId, INFINITE,
    InitializeProcThreadAttributeList, LPPROC_THREAD_ATTRIBUTE_LIST, PROCESS_INFORMATION,
    STARTF_USESHOWWINDOW, STARTF_USESTDHANDLES, STARTUPINFOEXW, STARTUPINFOW_FLAGS,
    TerminateProcess, UpdateProcThreadAttribute, WaitForSingleObject,
};
use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;
use windows::core::{PCSTR, PCWSTR, PWSTR};

// ConPTY function pointers (loaded dynamically)
type PfnCreatePseudoConsole = extern "system" fn(
    COORD,
    *mut std::ffi::c_void,
    *mut std::ffi::c_void,
    u32,
    *mut HANDLE,
) -> i32;

/// ConPTY creation flags.
///
/// `PSEUDOCONSOLE_RESIZE_QUIRK` (0x2) is deliberately not set: no reproducible
/// resize bug exists in the current test matrix. `PSEUDOCONSOLE_WIN32_INPUT_MODE`
/// (0x4) would switch input to Win32 `INPUT_RECORD`s and break raw VT byte
/// writers, so it is never enabled without an adapted input path.
const CONPTY_FLAGS: u32 = PSEUDOCONSOLE_INHERIT_CURSOR;

/// Startup flags for the ConPTY child.
///
/// `STARTF_USESTDHANDLES` with invalid standard handles prevents the child from
/// inheriting the parent's redirected stdio handles. The ConPTY attribute
/// supplies the child's actual console; the startup handles are not used for
/// PTY I/O. The handles are pseudo-values, so no handle lifetime is required.
const STARTUP_FLAGS: STARTUPINFOW_FLAGS =
    STARTUPINFOW_FLAGS(STARTF_USESHOWWINDOW.0 | STARTF_USESTDHANDLES.0);

type PfnResizePseudoConsole = extern "system" fn(HANDLE, COORD) -> i32;
type PfnClosePseudoConsole = extern "system" fn(HANDLE) -> i32;

static mut CREATE_PSEUDO_CONSOLE: Option<PfnCreatePseudoConsole> = None;
static mut RESIZE_PSEUDO_CONSOLE: Option<PfnResizePseudoConsole> = None;
static mut CLOSE_PSEUDO_CONSOLE: Option<PfnClosePseudoConsole> = None;

unsafe fn create_pseudoconsole(
    size: COORD,
    h_input: *mut std::ffi::c_void,
    h_output: *mut std::ffi::c_void,
    flags: u32,
    ph_pc: *mut HANDLE,
) -> i32 {
    if let Some(create_fn) = unsafe { CREATE_PSEUDO_CONSOLE } {
        create_fn(size, h_input, h_output, flags, ph_pc)
    } else {
        -1
    }
}

static mut CONPTY_LOADED: bool = false;

fn load_conpty_api() -> Result<(), PtyErrorKind> {
    unsafe {
        if CONPTY_LOADED {
            return Ok(());
        }

        let kernel32_name: Vec<u16> = "kernel32.dll".encode_utf16().chain(Some(0)).collect();
        let kernel32 = windows::Win32::System::LibraryLoader::GetModuleHandleW(
            windows::core::PCWSTR(kernel32_name.as_ptr()),
        )
        .map_err(|e| PtyErrorKind::OpenFailed(format!("GetModuleHandleW: {:?}", e)))?;

        let create_name = b"CreatePseudoConsole\0";
        let create_ptr = windows::Win32::System::LibraryLoader::GetProcAddress(
            kernel32,
            PCSTR::from_raw(create_name.as_ptr()),
        );
        if create_ptr.is_none() {
            return Err(PtyErrorKind::OpenFailed(
                "ConPTY not available on this Windows version".to_string(),
            ));
        }
        CREATE_PSEUDO_CONSOLE = Some(std::mem::transmute::<
            unsafe extern "system" fn() -> isize,
            PfnCreatePseudoConsole,
        >(create_ptr.unwrap()));

        let resize_name = b"ResizePseudoConsole\0";
        if let Some(ptr) = windows::Win32::System::LibraryLoader::GetProcAddress(
            kernel32,
            PCSTR::from_raw(resize_name.as_ptr()),
        ) {
            RESIZE_PSEUDO_CONSOLE = Some(std::mem::transmute::<
                unsafe extern "system" fn() -> isize,
                PfnResizePseudoConsole,
            >(ptr));
        }
        let close_name = b"ClosePseudoConsole\0";
        if let Some(ptr) = windows::Win32::System::LibraryLoader::GetProcAddress(
            kernel32,
            PCSTR::from_raw(close_name.as_ptr()),
        ) {
            CLOSE_PSEUDO_CONSOLE = Some(std::mem::transmute::<
                unsafe extern "system" fn() -> isize,
                PfnClosePseudoConsole,
            >(ptr));
        }

        CONPTY_LOADED = true;
        Ok(())
    }
}

// ============================================================================
// Proper Windows Command Line Quoting (from portable-pty / ArgvQuote)
// ============================================================================

fn append_quoted_wide(arg: &OsStr, cmdline: &mut Vec<u16>) {
    let wide: Vec<u16> = arg.encode_wide().collect();

    if !arg.is_empty()
        && !wide.iter().any(|&c| {
            c == ' ' as u16
                || c == '\t' as u16
                || c == '\n' as u16
                || c == '\x0b' as u16
                || c == '"' as u16
        })
    {
        cmdline.extend(wide);
        return;
    }

    cmdline.push('"' as u16);

    let mut i = 0;
    while i < wide.len() {
        let mut num_backslashes = 0;
        while i < wide.len() && wide[i] == '\\' as u16 {
            i += 1;
            num_backslashes += 1;
        }

        if i == wide.len() {
            for _ in 0..num_backslashes * 2 {
                cmdline.push('\\' as u16);
            }
            break;
        } else if wide[i] == '"' as u16 {
            for _ in 0..num_backslashes * 2 + 1 {
                cmdline.push('\\' as u16);
            }
            cmdline.push('"' as u16);
            i += 1;
        } else {
            for _ in 0..num_backslashes {
                cmdline.push('\\' as u16);
            }
            cmdline.push(wide[i]);
            i += 1;
        }
    }

    cmdline.push('"' as u16);
}

// ============================================================================
// Windows ConPTY Backend
// ============================================================================

struct SendSyncHandle(HANDLE);
unsafe impl Send for SendSyncHandle {}
unsafe impl Sync for SendSyncHandle {}
impl Clone for SendSyncHandle {
    fn clone(&self) -> Self {
        SendSyncHandle(self.0)
    }
}

pub struct WinChildProcess {
    pid: u32,
    process_handle: SendSyncHandle,
    thread_handle: SendSyncHandle,
    exit: Arc<watch::Sender<Option<u32>>>,
}

impl Drop for WinChildProcess {
    fn drop(&mut self) {
        // Kill-on-drop, matching `UnixChildProcess` (SIGKILL + reap): a
        // session dropped without `terminate()` must not orphan a live
        // child whose handles are about to vanish (unreapable forever).
        if self.is_running() {
            let _ = self.do_kill();
        }
        unsafe {
            let _ = CloseHandle(self.process_handle.0);
            let _ = CloseHandle(self.thread_handle.0);
        }
    }
}

impl ChildKiller for WinChildProcess {
    fn kill(&self) -> PtyResult<()> {
        self.do_kill()
    }

    fn clone_killer(&self) -> Box<dyn ChildKiller + Send + Sync> {
        Box::new(self.clone())
    }
}

impl Clone for WinChildProcess {
    fn clone(&self) -> Self {
        WinChildProcess {
            pid: self.pid,
            process_handle: self.process_handle.clone(),
            thread_handle: self.thread_handle.clone(),
            exit: self.exit.clone(),
        }
    }
}

#[async_trait::async_trait]
impl ChildBackend for WinChildProcess {
    fn pid(&self) -> u32 {
        self.pid
    }

    fn is_running(&self) -> bool {
        self.is_complete().is_none()
    }

    async fn wait(&self) -> Option<ProcessExit> {
        let mut rx = self.exit.subscribe();
        loop {
            if let Some(code) = self.is_complete() {
                return Some(ProcessExit {
                    pid: self.pid,
                    exit_code: Some(code as i32),
                    signal: None,
                });
            }
            // The watcher normally wakes this immediately. The timeout is a
            // fallback for a failed DuplicateHandle/GetExitCodeProcess path;
            // it is not the primary exit-detection mechanism.
            let _ = tokio::time::timeout(Duration::from_millis(250), rx.changed()).await;
        }
    }

    fn signal(&self, sig: i32) -> Result<(), PtyErrorKind> {
        match sig {
            2 => Err(PtyErrorKind::Unsupported(
                "SIGINT/interrupt is not supported on Windows ConPTY; use terminate() or kill()"
                    .to_string(),
            )),
            9 | 15 => self.do_kill(),
            _ => Err(PtyErrorKind::SignalError(format!(
                "Signal {} not supported on Windows",
                sig
            ))),
        }
    }
}

impl WinChildProcess {
    fn is_complete(&self) -> Option<u32> {
        if let Some(code) = *self.exit.borrow() {
            return Some(code);
        }

        let mut code: u32 = 0;
        let result = unsafe { GetExitCodeProcess(self.process_handle.0, &mut code) };
        if result.is_ok() && code != 259 {
            self.record_exit(code);
            Some(code)
        } else {
            None
        }
    }

    fn record_exit(&self, code: u32) {
        let _ = self.exit.send_replace(Some(code));
    }

    fn do_kill(&self) -> Result<(), PtyErrorKind> {
        unsafe {
            TerminateProcess(self.process_handle.0, 1)
                .map_err(|e| PtyErrorKind::ForkFailed(format!("TerminateProcess failed: {:?}", e)))
        }
    }
}

/// Wait for the process handle on a dedicated OS thread and publish the exit
/// code through a `watch` channel. This is the Windows H3 watcher: no polling
/// in the normal path.
fn spawn_exit_watcher(process: HANDLE, exit: Arc<watch::Sender<Option<u32>>>) {
    let current = unsafe { GetCurrentProcess() };
    let mut duplicate = INVALID_HANDLE_VALUE;
    let duplicated = unsafe {
        DuplicateHandle(
            current,
            process,
            current,
            &mut duplicate,
            0,
            false,
            DUPLICATE_SAME_ACCESS,
        )
    };
    if duplicated.is_err() {
        return;
    }

    let handle = SendSyncHandle(duplicate);
    std::thread::spawn(move || wait_for_exit(handle, exit));
}

fn wait_for_exit(handle: SendSyncHandle, exit: Arc<watch::Sender<Option<u32>>>) {
    let waited = unsafe { WaitForSingleObject(handle.0, INFINITE) };
    if waited == WAIT_OBJECT_0 {
        let mut code: u32 = 0;
        if unsafe { GetExitCodeProcess(handle.0, &mut code) }.is_ok() {
            let _ = exit.send_replace(Some(code));
        }
    }
    unsafe {
        let _ = CloseHandle(handle.0);
    }
}

pub struct WinPtyBackend {
    conpty_handle: SendSyncHandle,
    input_async: Arc<Mutex<NamedPipeServer>>,
    output_async: Arc<Mutex<NamedPipeServer>>,
    size: std::sync::Mutex<Winsize>,
    closed: Arc<AtomicBool>,
}

impl Clone for WinPtyBackend {
    fn clone(&self) -> Self {
        let size = *self.size.lock().unwrap();
        WinPtyBackend {
            conpty_handle: self.conpty_handle.clone(),
            input_async: self.input_async.clone(),
            output_async: self.output_async.clone(),
            size: std::sync::Mutex::new(size),
            closed: Arc::clone(&self.closed),
        }
    }
}

impl Drop for WinPtyBackend {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

impl WinPtyBackend {
    /// Create a new WinPtyBackend.
    ///
    /// Both pipes must already be connected (connect().await called) before
    /// passing them here, so that mio's IOCP pump is armed. This constructor
    /// is synchronous — the async work (pipe connect) happens in
    /// `create_conpty_pipe_pair`.
    fn new(
        conpty_handle: SendSyncHandle,
        input_async: NamedPipeServer,
        output_async: NamedPipeServer,
        size: Winsize,
    ) -> Self {
        WinPtyBackend {
            conpty_handle,
            input_async: Arc::new(Mutex::new(input_async)),
            output_async: Arc::new(Mutex::new(output_async)),
            size: std::sync::Mutex::new(size),
            closed: Arc::new(AtomicBool::new(false)),
        }
    }
}

#[async_trait::async_trait]
impl PtyBackend for WinPtyBackend {
    async fn read(&self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "ConPTY is closed",
            ));
        }
        let mut pipe = self.output_async.lock().await;
        pipe.read(buf).await
    }

    async fn write(&self, buf: &[u8]) -> std::io::Result<usize> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "ConPTY is closed",
            ));
        }
        let mut pipe = self.input_async.lock().await;
        pipe.write_all(buf).await?;
        Ok(buf.len())
    }

    fn set_winsize(&self, winsize: Winsize) -> PtyResult<()> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(PtyErrorKind::Closed);
        }
        if let Some(resize_fn) = unsafe { RESIZE_PSEUDO_CONSOLE } {
            let hr = resize_fn(
                self.conpty_handle.0,
                COORD {
                    X: winsize.cols as i16,
                    Y: winsize.rows as i16,
                },
            );
            if hr < 0 {
                return Err(PtyErrorKind::OpenFailed(format!(
                    "ResizePseudoConsole failed: HRESULT {}",
                    hr
                )));
            }
        }
        *self.size.lock().unwrap() = winsize;
        Ok(())
    }

    fn get_winsize(&self) -> PtyResult<Winsize> {
        Ok(*self.size.lock().unwrap())
    }

    fn raw_handle(&self) -> *mut std::ffi::c_void {
        if self.closed.load(Ordering::SeqCst) {
            INVALID_HANDLE_VALUE.0
        } else {
            self.conpty_handle.0.0
        }
    }

    fn is_open(&self) -> bool {
        !self.closed.load(Ordering::SeqCst) && !self.conpty_handle.0.is_invalid()
    }

    fn close(&self) -> PtyResult<()> {
        if self.closed.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        if let Some(close_fn) = unsafe { CLOSE_PSEUDO_CONSOLE } {
            let hr = close_fn(self.conpty_handle.0);
            if hr < 0 {
                return Err(PtyErrorKind::WindowsError(format!(
                    "ClosePseudoConsole failed: HRESULT {}",
                    hr
                )));
            }
        }
        Ok(())
    }

    async fn shutdown_write(&self) -> PtyResult<()> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(PtyErrorKind::Closed);
        }
        let pipe = self.input_async.lock().await;
        pipe.disconnect()
            .map_err(|e| PtyErrorKind::AsyncIo(format!("disconnect input pipe: {e}")))
    }
}

// ============================================================================
// Pipe creation for ConPTY (async — calls connect() once to arm IOCP pump)
// ============================================================================

/// Create a ConPTY pipe pair: server (tokio) + client (raw HANDLE).
/// Returns SendSyncHandle for the client so it can live across .await points.
async fn create_conpty_pipe_pair() -> Result<(NamedPipeServer, SendSyncHandle), PtyErrorKind> {
    static PIPE_COUNTER: AtomicUsize = AtomicUsize::new(1);

    let pipe_num = PIPE_COUNTER.fetch_add(1, Ordering::SeqCst);
    let pipe_name = format!(r"\\.\pipe\stitch-pty-{}-{}", std::process::id(), pipe_num);

    let server = ServerOptions::new()
        .first_pipe_instance(true)
        .create(&pipe_name)
        .map_err(|e| PtyErrorKind::OpenFailed(format!("NamedPipeServer create: {}", e)))?;

    let wide_name: Vec<u16> = pipe_name.encode_utf16().chain(Some(0)).collect();
    let handle = unsafe {
        CreateFileW(
            PCWSTR(wide_name.as_ptr()),
            (GENERIC_READ | GENERIC_WRITE).0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_FLAGS_AND_ATTRIBUTES(0),
            None,
        )
    }
    .map_err(|e| PtyErrorKind::OpenFailed(format!("CreateFileW (pipe connect): {:?}", e)))?;

    // Wrap raw HANDLE in SendSyncHandle BEFORE .await so the future is Send.
    let client_handle = SendSyncHandle(handle);

    // Call connect() on the server to arm mio's IOCP read/write pump.
    // The client (CreateFileW) is already attached, so this returns immediately.
    server
        .connect()
        .await
        .map_err(|e| PtyErrorKind::OpenFailed(format!("server connect: {}", e)))?;

    Ok((server, client_handle))
}

// ============================================================================
// Platform spawn / open_pty functions (async — callers must be in tokio runtime)
// ============================================================================

pub async fn open_pty(
    winsize: Option<Winsize>,
) -> Result<std::sync::Arc<dyn PtyBackend>, PtyErrorKind> {
    load_conpty_api()?;

    let ws = winsize.unwrap_or(Winsize {
        rows: 24,
        cols: 80,
        xpixel: 0,
        ypixel: 0,
    });

    let (input_server, stdin_handle) = create_conpty_pipe_pair().await?;
    let (output_server, stdout_handle) = create_conpty_pipe_pair().await?;

    let mut conpty: HANDLE = INVALID_HANDLE_VALUE;
    unsafe {
        let hr = create_pseudoconsole(
            COORD {
                X: ws.cols as i16,
                Y: ws.rows as i16,
            },
            stdin_handle.0.0,
            stdout_handle.0.0,
            CONPTY_FLAGS,
            &mut conpty,
        );
        if hr < 0 {
            return Err(PtyErrorKind::OpenFailed(format!(
                "PseudoConsoleCreate failed: HRESULT {}",
                hr
            )));
        }
        let _ = CloseHandle(stdin_handle.0);
        let _ = CloseHandle(stdout_handle.0);
    }

    // No more .await after this point — conpty is not Send but we're done awaiting.
    let conpty_handle = SendSyncHandle(conpty);
    let backend = WinPtyBackend::new(conpty_handle, input_server, output_server, ws);
    Ok(Arc::new(backend))
}

/// Build a UTF-16 environment block for `CreateProcessW`.
///
/// Duplicate keys are collapsed case-insensitively (ASCII) with last-wins.
/// An empty explicit environment becomes the required double-null block.
fn build_env_block(env: &[(String, String)]) -> Vec<u16> {
    let entries = crate::platform::dedupe_env_windows(env);
    if entries.is_empty() {
        return vec![0, 0];
    }

    let mut block: Vec<u16> = Vec::new();
    for (key, value) in entries {
        block.extend(key.encode_utf16());
        block.push('=' as u16);
        block.extend(value.encode_utf16());
        block.push(0);
    }
    block.push(0);
    block
}

pub async fn spawn(
    program: &str,
    args: &[String],
    env: Option<&[(String, String)]>,
    winsize: Option<Winsize>,
    cwd: Option<&str>,
) -> Result<(Arc<dyn PtyBackend>, Arc<dyn ChildBackend>), PtyErrorKind> {
    let ws = winsize.unwrap_or(Winsize {
        rows: 24,
        cols: 80,
        xpixel: 0,
        ypixel: 0,
    });

    load_conpty_api()?;

    let (mut input_server, stdin_handle) = create_conpty_pipe_pair().await?;
    let (output_server, stdout_handle) = create_conpty_pipe_pair().await?;

    // All non-Send Windows types must be scoped so they don't live across .await
    let (conpty_handle, pid, process_handle, thread_handle) = {
        // Create pseudoconsole (synchronous Windows API call)
        let mut conpty: HANDLE = INVALID_HANDLE_VALUE;
        unsafe {
            let hr = create_pseudoconsole(
                COORD {
                    X: ws.cols as i16,
                    Y: ws.rows as i16,
                },
                stdin_handle.0.0,
                stdout_handle.0.0,
                CONPTY_FLAGS,
                &mut conpty,
            );
            if hr < 0 {
                return Err(PtyErrorKind::OpenFailed(format!(
                    "PseudoConsoleCreate failed: HRESULT {}",
                    hr
                )));
            }
            let _ = CloseHandle(stdin_handle.0);
            let _ = CloseHandle(stdout_handle.0);
        }

        // Build command line with proper quoting and spaces
        let mut cmdline: Vec<u16> = Vec::new();
        append_quoted_wide(OsStr::new(program), &mut cmdline);
        for arg in args {
            cmdline.push(' ' as u16);
            append_quoted_wide(OsStr::new(arg.as_str()), &mut cmdline);
        }
        cmdline.push(0);

        // Child working directory (NUL-terminated wide string for lpCurrentDirectory).
        // None inherits the parent's directory, as before.
        let cwd_wide: Option<Vec<u16>> = cwd.map(|dir| {
            OsStr::new(dir)
                .encode_wide()
                .chain(std::iter::once(0))
                .collect()
        });

        // Build the environment block. `None` means inherit: pass a null
        // lpEnvironment. `Some(&[])` is an explicit empty environment, which
        // Windows represents as a double-null block -- the same block std
        // builds for `Command::env_clear()`.
        let env_block: Option<Vec<u16>> = env.map(build_env_block);

        // --------------------------------------------------------------------
        // ConPTY attribute list setup (required for EXTENDED_STARTUPINFO_PRESENT)
        // --------------------------------------------------------------------
        const PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE: usize = 0x00020016;

        let mut attr_list_size: usize = 0;
        let _ = unsafe {
            InitializeProcThreadAttributeList(
                Some(LPPROC_THREAD_ATTRIBUTE_LIST(std::ptr::null_mut())),
                1,
                Some(0),
                &mut attr_list_size,
            )
        };

        let mut attr_list_buf = vec![0u8; attr_list_size];
        let attr_list = unsafe {
            let ptr = attr_list_buf.as_mut_ptr() as *mut std::ffi::c_void;
            let typed_ptr = LPPROC_THREAD_ATTRIBUTE_LIST(ptr);
            InitializeProcThreadAttributeList(Some(typed_ptr), 1, Some(0), &mut attr_list_size)
                .map_err(|e| {
                    PtyErrorKind::ForkFailed(format!("InitializeProcThreadAttributeList: {:?}", e))
                })?;
            typed_ptr
        };

        // Set the ConPTY attribute on the process thread attribute list.
        unsafe {
            UpdateProcThreadAttribute(
                attr_list,
                0,
                PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE,
                Some(conpty.0),
                std::mem::size_of::<HANDLE>(),
                None,
                None,
            )
            .map_err(|e| PtyErrorKind::ForkFailed(format!("UpdateProcThreadAttribute: {:?}", e)))?;
        }

        let mut startup_info: STARTUPINFOEXW = unsafe { std::mem::zeroed() };
        startup_info.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
        startup_info.StartupInfo.dwFlags = STARTUP_FLAGS;
        // Do not inherit the parent's stdio. Once STARTF_USESTDHANDLES is set,
        // these handles must at least be syntactically valid; ConPTY supplies
        // the child console through PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE.
        startup_info.StartupInfo.hStdInput = INVALID_HANDLE_VALUE;
        startup_info.StartupInfo.hStdOutput = INVALID_HANDLE_VALUE;
        startup_info.StartupInfo.hStdError = INVALID_HANDLE_VALUE;
        startup_info.StartupInfo.wShowWindow = SW_HIDE.0 as u16;
        startup_info.lpAttributeList = attr_list;

        let mut process_info: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };

        let success = unsafe {
            CreateProcessW(
                None,
                Some(PWSTR(cmdline.as_mut_ptr())),
                None,
                None,
                false,
                EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT,
                env_block
                    .as_ref()
                    .map(|block| block.as_ptr() as *const std::ffi::c_void),
                cwd_wide
                    .as_ref()
                    .map(|w| PCWSTR(w.as_ptr()))
                    .unwrap_or(PCWSTR::null()),
                &startup_info.StartupInfo,
                &mut process_info,
            )
        };

        // Clean up attribute list regardless of success
        unsafe {
            DeleteProcThreadAttributeList(attr_list);
        }

        if let Err(e) = success {
            return Err(PtyErrorKind::ForkFailed(format!(
                "CreateProcessW failed: {:?}",
                e
            )));
        }

        // Extract Send-safe values before the block ends (drops all non-Send locals)
        let conpty_handle = SendSyncHandle(conpty);
        let pid = unsafe { GetProcessId(process_info.hProcess) };
        let process_handle = SendSyncHandle(process_info.hProcess);
        let thread_handle = SendSyncHandle(process_info.hThread);

        (conpty_handle, pid, process_handle, thread_handle)
    }; // <-- All non-Send locals (conpty, startup_info, process_info, attr_list_buf, cmdline, env_block) dropped here

    // Answer conhost's startup cursor-position query so it begins relaying child output.
    let _ = input_server.write_all(b"\x1b[1;1R").await;

    let backend = WinPtyBackend::new(conpty_handle, input_server, output_server, ws);
    let (exit_tx, _) = watch::channel::<Option<u32>>(None);
    let exit_tx = Arc::new(exit_tx);
    spawn_exit_watcher(process_handle.0, Arc::clone(&exit_tx));

    Ok((
        Arc::new(backend),
        Arc::new(WinChildProcess {
            pid,
            process_handle,
            thread_handle,
            exit: exit_tx,
        }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_append_quoted_wide_no_spaces() {
        let arg = OsStr::new("hello");
        let mut cmdline: Vec<u16> = Vec::new();
        append_quoted_wide(arg, &mut cmdline);
        let result = String::from_utf16(&cmdline).unwrap();
        assert_eq!(result, "hello");
    }

    #[test]
    fn test_append_quoted_wide_with_spaces() {
        let arg = OsStr::new("hello world");
        let mut cmdline: Vec<u16> = Vec::new();
        append_quoted_wide(arg, &mut cmdline);
        let result = String::from_utf16(&cmdline).unwrap();
        assert_eq!(result, "\"hello world\"");
    }

    #[test]
    fn test_append_quoted_wide_with_tabs() {
        let arg = OsStr::new("hello\tworld");
        let mut cmdline: Vec<u16> = Vec::new();
        append_quoted_wide(arg, &mut cmdline);
        let result = String::from_utf16(&cmdline).unwrap();
        assert!(result.starts_with('"'));
        assert!(result.ends_with('"'));
    }

    #[test]
    fn test_append_quoted_wide_with_quotes() {
        let arg = OsStr::new("hello \"world\"");
        let mut cmdline: Vec<u16> = Vec::new();
        append_quoted_wide(arg, &mut cmdline);
        let result = String::from_utf16(&cmdline).unwrap();
        assert!(result.starts_with('"'));
        assert!(result.ends_with('"'));
    }

    #[test]
    fn test_append_quoted_wide_with_backslashes() {
        let arg = OsStr::new("hello\\world");
        let mut cmdline: Vec<u16> = Vec::new();
        append_quoted_wide(arg, &mut cmdline);
        let result = String::from_utf16(&cmdline).unwrap();
        assert_eq!(result, "hello\\world");
    }

    #[test]
    fn test_append_quoted_wide_empty() {
        let arg = OsStr::new("");
        let mut cmdline: Vec<u16> = Vec::new();
        append_quoted_wide(arg, &mut cmdline);
        let result = String::from_utf16(&cmdline).unwrap();
        assert_eq!(result, "\"\"");
    }

    #[test]
    fn test_append_quoted_wide_newline() {
        let arg = OsStr::new("hello\nworld");
        let mut cmdline: Vec<u16> = Vec::new();
        append_quoted_wide(arg, &mut cmdline);
        let result = String::from_utf16(&cmdline).unwrap();
        assert!(result.starts_with('"'));
        assert!(result.ends_with('"'));
    }

    #[test]
    fn test_append_quoted_wide_vertical_tab() {
        let arg = OsStr::new("hello\x0bworld");
        let mut cmdline: Vec<u16> = Vec::new();
        append_quoted_wide(arg, &mut cmdline);
        let result = String::from_utf16(&cmdline).unwrap();
        assert!(result.starts_with('"'));
        assert!(result.ends_with('"'));
    }

    #[test]
    fn test_append_quoted_wide_complex() {
        let arg = OsStr::new("C:\\Program Files\\App\\app.exe");
        let mut cmdline: Vec<u16> = Vec::new();
        append_quoted_wide(arg, &mut cmdline);
        let result = String::from_utf16(&cmdline).unwrap();
        assert!(result.starts_with('"'));
        assert!(result.ends_with('"'));
    }

    #[test]
    fn test_append_quoted_wide_multiple_trailing_backslashes() {
        let arg = OsStr::new("path\\\\ ");
        let mut cmdline: Vec<u16> = Vec::new();
        append_quoted_wide(arg, &mut cmdline);
        let result = String::from_utf16(&cmdline).unwrap();
        assert!(result.starts_with('"'));
        assert!(result.ends_with('"'));
    }

    #[test]
    fn test_win_child_process_pid() {
        let _check: fn() -> WinChildProcess = || WinChildProcess {
            pid: 42,
            process_handle: SendSyncHandle(HANDLE::default()),
            thread_handle: SendSyncHandle(HANDLE::default()),
            exit: Arc::new(watch::channel::<Option<u32>>(None).0),
        };
    }

    #[test]
    fn test_win_child_process_clone() {
        let cp = WinChildProcess {
            pid: 42,
            process_handle: SendSyncHandle(HANDLE::default()),
            thread_handle: SendSyncHandle(HANDLE::default()),
            exit: Arc::new(watch::channel::<Option<u32>>(None).0),
        };
        let cp2 = cp.clone();
        assert_eq!(cp.pid, cp2.pid);
    }

    #[test]
    fn test_win_pty_backend_fields() {
        fn assert_clone<T: Clone>() {}
        assert_clone::<WinPtyBackend>();
    }

    #[test]
    fn test_send_sync_handle_send() {
        fn assert_send<T: Send>() {}
        assert_send::<SendSyncHandle>();
    }

    #[test]
    fn test_send_sync_handle_sync() {
        fn assert_sync<T: Sync>() {}
        assert_sync::<SendSyncHandle>();
    }

    #[test]
    fn test_send_sync_handle_clone() {
        let h1 = SendSyncHandle(HANDLE::default());
        let h2 = h1.clone();
        assert_eq!(h1.0.0, h2.0.0);
    }

    #[test]
    fn test_pipe_counter_increments() {
        let counter = AtomicUsize::new(1);
        let a = counter.fetch_add(1, Ordering::SeqCst);
        let b = counter.fetch_add(1, Ordering::SeqCst);
        assert_eq!(a, 1);
        assert_eq!(b, 2);
    }

    fn decode_env_block(block: &[u16]) -> Vec<String> {
        block
            .split(|&unit| unit == 0)
            .filter(|entry| !entry.is_empty())
            .map(|entry| String::from_utf16(entry).expect("valid UTF-16 env entry"))
            .collect()
    }

    #[test]
    fn test_conpty_and_startup_flags() {
        // H2: standard handles are explicitly suppressed; ConPTY supplies the
        // child console through the process attribute.
        assert_eq!(
            STARTUP_FLAGS.0 & STARTF_USESTDHANDLES.0,
            STARTF_USESTDHANDLES.0
        );
        assert_eq!(
            STARTUP_FLAGS.0 & STARTF_USESHOWWINDOW.0,
            STARTF_USESHOWWINDOW.0
        );

        // H4: keep cursor inheritance, but do not enable the resize quirk
        // (0x2) or Win32 INPUT_RECORD mode (0x4) without a failing test.
        assert_eq!(CONPTY_FLAGS & 0x1, 0x1);
        assert_eq!(CONPTY_FLAGS & 0x2, 0);
        assert_eq!(CONPTY_FLAGS & 0x4, 0);
    }

    #[test]
    fn test_build_env_block_empty_is_double_null() {
        assert_eq!(build_env_block(&[]), vec![0, 0]);
    }

    #[test]
    fn test_build_env_block_dedupes_casing_and_encodes_unicode() {
        let env = vec![
            ("Path".to_string(), "first".to_string()),
            ("PATH".to_string(), "second".to_string()),
            ("EMPTY".to_string(), String::new()),
            ("ЮНИКОД".to_string(), "значение".to_string()),
        ];
        let block = build_env_block(&env);
        assert_eq!(
            decode_env_block(&block),
            vec!["PATH=second", "EMPTY=", "ЮНИКОД=значение",]
        );
        assert_eq!(block.last(), Some(&0));
    }
}
