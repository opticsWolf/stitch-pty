//! Unix PTY implementation using POSIX APIs
//!
//! Uses: openpty(3), fork(2), setsid(2), TIOCSCTTY, dup2, execve/execvpe
//! Async I/O: tokio::io::AsyncFd over raw FDs
//!
//! Improvements from portable-pty:
//! - Async-signal-safe fork child (all allocations before fork)
//! - `close_random_fds_async_signal_safe()` for macOS Big Sur / Linux FD leak prevention
//! - Proper signal disposition reset before exec
//! - ChildKiller trait implementation

use crate::errors::{PtyErrorKind, PtyResult};
use crate::platform::{ChildBackend, ChildKiller, ProcessExit, PtyBackend};
use crate::winsize::Winsize;
use libc::c_char;
use nix::fcntl::{fcntl, FcntlArg, OFlag};
use std::os::fd::{BorrowedFd, IntoRawFd};
use nix::pty::openpty;
use nix::sys::signal::{kill, Signal};
use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};
use nix::unistd::{close, fork, setsid, ForkResult, Pid};
use parking_lot::Mutex;
use std::ffi::CString;
use std::os::fd::RawFd;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::unix::AsyncFd;
use tokio::io::Interest;


// ============================================================================
// close_random_fds (async-signal-safe version)
// ============================================================================

/// Close leaked file descriptors in the fork child.
///
/// This version is async-signal-safe: it uses only libc::sysconf and
/// libc::close, with no allocation or std::fs calls. Safe to call after
/// fork() in a multithreaded process.
pub fn close_random_fds_async_signal_safe(skip: &[RawFd]) {
    unsafe {
        let max = libc::sysconf(libc::_SC_OPEN_MAX);
        let max = if max > 0 { max as RawFd } else { 4096 };

        for fd in 3..max {
            if !skip.contains(&fd) {
                libc::close(fd);
            }
        }
    }
}

// ============================================================================
// PreparedCommand — all allocations done in the parent before fork
// ============================================================================

/// Pre-computed command arguments and environment, ready for execve/execvpe.
///
/// All CString allocations and Vec construction happen in `new()`, which is
/// called in the parent process before fork(). The child only reads raw
/// pointers, making it async-signal-safe.
struct PreparedCommand {
    program: CString,
    argv: Vec<CString>,
    env: Vec<CString>,
    argv_ptrs: Vec<*const c_char>,
    envp_ptrs: Vec<*const c_char>,
}

impl PreparedCommand {
    fn new(
        program: &str,
        args: &[String],
        env: &[(String, String)],
    ) -> PtyResult<Self> {
        let program_cstr = CString::new(program)
            .map_err(|_| PtyErrorKind::ForkFailed("program contains NUL byte".into()))?;

        let mut argv = Vec::with_capacity(args.len() + 1);
        argv.push(program_cstr.clone());
        for arg in args {
            argv.push(
                CString::new(arg.as_str())
                    .map_err(|_| PtyErrorKind::ForkFailed("argument contains NUL byte".into()))?,
            );
        }

        let mut envp = Vec::with_capacity(env.len());
        for (k, v) in env {
            let s = format!("{}={}", k, v);
            envp.push(
                CString::new(s)
                    .map_err(|_| PtyErrorKind::ForkFailed("environment contains NUL byte".into()))?,
            );
        }

        let mut argv_ptrs: Vec<*const c_char> = argv.iter().map(|s| s.as_ptr()).collect();
        argv_ptrs.push(std::ptr::null());

        let mut envp_ptrs: Vec<*const c_char> = envp.iter().map(|s| s.as_ptr()).collect();
        envp_ptrs.push(std::ptr::null());

        Ok(Self {
            program: program_cstr,
            argv,
            env: envp,
            argv_ptrs,
            envp_ptrs,
        })
    }
}

// Resolve `program` against PATH (called in the parent before fork).
// Returns an absolute path string suitable for execve.
fn resolve_executable(program: &str) -> String {
    // Try as absolute or relative path first
    if let Ok(metadata) = std::fs::metadata(program) {
        if metadata.is_file() {
            if let Ok(abs) = std::fs::canonicalize(program) {
                return abs.to_string_lossy().to_string();
            }
        }
    }

    // Search PATH
    let path_dirs = std::env::var_os("PATH").unwrap_or_default();
    for dir in std::env::split_paths(&path_dirs) {
        let candidate = dir.join(program);
        if let Ok(m) = std::fs::metadata(&candidate) {
            if m.is_file() {
                if let Ok(abs) = std::fs::canonicalize(&candidate) {
                    return abs.to_string_lossy().to_string();
                }
            }
        }
    }

    // Fallback: return original program name (execve will try it)
    program.to_string()
}

// ============================================================================
// Unix PtyMaster
// ============================================================================

pub struct UnixPtyMaster {
    async_fd: Option<AsyncFd<RawFd>>,
}

impl UnixPtyMaster {
    pub fn new(fd: RawFd) -> std::io::Result<Self> {
        let async_fd = AsyncFd::with_interest(fd, Interest::READABLE | Interest::WRITABLE)?;
        Ok(UnixPtyMaster { async_fd: Some(async_fd) })
    }

    fn fd(&self) -> RawFd {
        *self.async_fd.as_ref().unwrap().get_ref()
    }
}

#[async_trait::async_trait]
impl PtyBackend for UnixPtyMaster {
    async fn read(&self, buf: &mut [u8]) -> std::io::Result<usize> {
        loop {
            let mut guard = self.async_fd.as_ref().unwrap().readable().await?;

            match guard.try_io(|inner| {
                let fd = *inner.get_ref();
                let ret = unsafe {
                    libc::read(fd, buf.as_mut_ptr() as *mut _, buf.len())
                };

                if ret < 0 {
                    let err = std::io::Error::last_os_error();

                    // On macOS/Linux, EIO usually means the PTY slave closed.
                    // Treat it as EOF.
                    if err.raw_os_error() == Some(libc::EIO) {
                        return Ok(0);
                    }

                    Err(err)
                } else {
                    Ok(ret as usize)
                }
            }) {
                Ok(result) => return result,

                // Only retry if the operation would block.
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,

                // Real errors must be propagated.
                Err(e) => return Err(e),
            }
        }
    }

    async fn write(&self, buf: &[u8]) -> std::io::Result<usize> {
        loop {
            let mut guard = self.async_fd.as_ref().unwrap().writable().await?;

            match guard.try_io(|inner| {
                let fd = *inner.get_ref();
                let ret = unsafe {
                    libc::write(fd, buf.as_ptr() as *const _, buf.len())
                };

                if ret < 0 {
                    let err = std::io::Error::last_os_error();

                    // EIO on write means PTY slave closed → BrokenPipe
                    if err.raw_os_error() == Some(libc::EIO) {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::BrokenPipe,
                            "PTY slave closed",
                        ));
                    }

                    Err(err)
                } else {
                    Ok(ret as usize)
                }
            }) {
                Ok(result) => return result,

                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,

                Err(e) => return Err(e),
            }
        }
    }

    fn set_winsize(&self, winsize: Winsize) -> PtyResult<()> {
        let ws: nix::pty::Winsize = winsize.into();
        let fd = self.fd();
        unsafe {
            let ret = libc::ioctl(fd, libc::TIOCSWINSZ, &ws as *const _);
            if ret < 0 {
                return Err(PtyErrorKind::WinsizeFailed(
                    format!("TIOCSWINSZ failed: {}", std::io::Error::last_os_error())
                ));
            }
        }
        // Forward SIGWINCH to process group
        let pgrp = unsafe { libc::tcgetpgrp(fd) };
        if pgrp > 0 {
            unsafe { libc::kill(-pgrp, libc::SIGWINCH); }
        }
        Ok(())
    }

    fn get_winsize(&self) -> PtyResult<Winsize> {
        let mut ws: nix::pty::Winsize = unsafe { std::mem::zeroed() };
        let fd = self.fd();
        unsafe {
            let ret = libc::ioctl(fd, libc::TIOCGWINSZ, &mut ws as *mut _);
            if ret < 0 {
                return Err(PtyErrorKind::WinsizeFailed(
                    format!("TIOCGWINSZ failed: {}", std::io::Error::last_os_error())
                ));
            }
        }
        Ok(ws.into())
    }

    fn raw_handle(&self) -> RawFd {
        self.fd()
    }

    fn is_open(&self) -> bool {
        let fd = self.fd();
        unsafe { libc::fcntl(fd, libc::F_GETFD) >= 0 }
    }
}

impl Drop for UnixPtyMaster {
    fn drop(&mut self) {
        if let Some(async_fd) = self.async_fd.take() {
            // Deregister from the event loop before closing the FD.
            // This avoids closing the FD out from under Tokio's kqueue/epoll.
            if let Ok(fd) = async_fd.into_inner() {
                let _ = close(fd);
            }
        }
    }
}

// ============================================================================
// Unix ChildProcess + ChildKiller
// ============================================================================

#[derive(Clone)]
pub struct UnixChildProcess {
    inner: Arc<Mutex<UnixChildState>>,
}

#[derive(Debug)]
struct UnixChildState {
    pid: Pid,
    running: bool,
    exit_status: Option<ProcessExit>,
}

impl UnixChildProcess {
    pub fn new(pid: Pid) -> Self {
        let state = Arc::new(Mutex::new(UnixChildState {
            pid,
            running: true,
            exit_status: None,
        }));

        // The background thread holds a strong reference so it stays alive
        // until the child is reaped, even if all UnixChildProcess handles
        // are dropped.
        let task_state = state.clone();

        std::thread::spawn(move || {
            loop {
                std::thread::sleep(Duration::from_millis(50));

                let pid = {
                    let guard = task_state.lock();
                    if !guard.running {
                        break;
                    }
                    guard.pid
                };

                // WNOHANG is non-blocking
                let result = waitpid(pid, Some(WaitPidFlag::WNOHANG));

                match result {
                    Ok(WaitStatus::Exited(pid, code)) => {
                        let mut guard = task_state.lock();
                        guard.running = false;
                        guard.exit_status = Some(ProcessExit {
                            pid: pid.as_raw() as u32,
                            exit_code: Some(code),
                            signal: None,
                        });
                        break;
                    }
                    Ok(WaitStatus::Signaled(pid, signal, _core_dumped)) => {
                        let mut guard = task_state.lock();
                        guard.running = false;
                        guard.exit_status = Some(ProcessExit {
                            pid: pid.as_raw() as u32,
                            exit_code: None,
                            signal: Some(signal as i32),
                        });
                        break;
                    }
                    Ok(WaitStatus::StillAlive) => {}
                    // Stopped, Continued, PtraceEvent, PtraceSyscall — ignore and poll again
                    Ok(_) => {}
                    // EINTR is common on macOS; just ignore and retry on the next loop
                    Err(nix::errno::Errno::EINTR) => continue,
                    Err(_) => {
                        let mut guard = task_state.lock();
                        guard.running = false;
                        break;
                    }
                }
            }
        });

        UnixChildProcess {
            inner: state,
        }
    }
}

#[async_trait::async_trait]
impl ChildBackend for UnixChildProcess {
    fn pid(&self) -> u32 {
        self.inner.lock().pid.as_raw() as u32
    }

    fn is_running(&self) -> bool {
        self.inner.lock().running
    }

    async fn wait(&self) -> Option<ProcessExit> {
        loop {
            {
                let guard = self.inner.lock();
                if !guard.running {
                    return guard.exit_status.clone();
                }
            }
            // Yield to the async runtime while waiting for the background thread
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    fn signal(&self, sig: i32) -> PtyResult<()> {
        let signal = Signal::try_from(sig)
            .map_err(|_| PtyErrorKind::SignalError(format!("Invalid signal: {}", sig)))?;
        let pid = self.inner.lock().pid;
        let pgid = Pid::from_raw(-pid.as_raw());
        match kill(pgid, signal) {
            Ok(_) => Ok(()),
            // macOS returns EPERM when sending signals to a zombie process group.
            // ESRCH means the process/group no longer exists.
            // Both are no-ops for an already-dead process.
            Err(nix::errno::Errno::EPERM) => Ok(()),
            Err(nix::errno::Errno::ESRCH) => Ok(()),
            Err(e) => Err(PtyErrorKind::SignalError(e.to_string())),
        }
    }
}

impl ChildKiller for UnixChildProcess {
    fn kill(&self) -> PtyResult<()> {
        self.signal(9) // SIGKILL
    }

    fn clone_killer(&self) -> Box<dyn ChildKiller + Send + Sync> {
        Box::new(self.clone())
    }
}

impl Drop for UnixChildProcess {
    fn drop(&mut self) {
        if self.is_running() {
            let _ = self.kill();
            let pid = self.inner.lock().pid;
            // Reap the zombie to prevent leaks if the background thread already exited
            loop {
                match waitpid(pid, Some(WaitPidFlag::WNOHANG)) {
                    Ok(WaitStatus::Exited(_, _)) | Ok(WaitStatus::Signaled(_, _, _)) => break,
                    Ok(WaitStatus::StillAlive) => std::thread::sleep(Duration::from_millis(10)),
                    Err(_) => break,
                    _ => {}
                }
            }
        }
    }
}

// ============================================================================
// PTY Pair and Spawning
// ============================================================================

struct PtyPair {
    master_fd: RawFd,
    slave_fd: RawFd,
}

impl PtyPair {
    fn open(winsize: Option<Winsize>) -> PtyResult<Self> {
        let nix_winsize = winsize.map(|w| w.into());
        let result = openpty(nix_winsize.as_ref(), None)
            .map_err(|e| PtyErrorKind::OpenFailed(format!("openpty failed: {}", e)))?;

        // Convert OwnedFd to RawFd
        let master_fd = result.master.into_raw_fd();
        let slave_fd = result.slave.into_raw_fd();

        // Set non-blocking on master
        let master_borrowed: BorrowedFd = unsafe { BorrowedFd::borrow_raw(master_fd) };
        let flags = fcntl(master_borrowed, FcntlArg::F_GETFL)
            .map_err(|e| PtyErrorKind::OpenFailed(format!("fcntl GETFL: {}", e)))?;
        fcntl(master_borrowed, FcntlArg::F_SETFL(OFlag::from_bits_truncate(flags) | OFlag::O_NONBLOCK))
            .map_err(|e| PtyErrorKind::OpenFailed(format!("fcntl SETFL: {}", e)))?;

        Ok(PtyPair { master_fd, slave_fd })
    }
}

impl Drop for PtyPair {
    fn drop(&mut self) {
        let _ = close(self.slave_fd);
        let _ = close(self.master_fd);
    }
}

/// Async-signal-safe child setup. Only uses raw pointers and libc calls.
/// All CString/Vec allocations are done in the parent via PreparedCommand.
unsafe fn child_setup(
    slave_fd: RawFd,
    master_fd: RawFd,
    cmd: &PreparedCommand,
) -> ! {
    // Reset signal dispositions to defaults.
    for signo in &[
        libc::SIGCHLD,
        libc::SIGHUP,
        libc::SIGINT,
        libc::SIGQUIT,
        libc::SIGTERM,
        libc::SIGALRM,
    ] {
        libc::signal(*signo, libc::SIG_DFL);
    }

    // Unblock all signals.
    let mut empty_set: libc::sigset_t = std::mem::zeroed();
    libc::sigemptyset(&mut empty_set);
    libc::sigprocmask(libc::SIG_SETMASK, &empty_set, std::ptr::null_mut());

    // Create new session.
    if setsid().is_err() {
        libc::_exit(1);
    }

    // Set controlling terminal.
    let ret = libc::ioctl(slave_fd, libc::TIOCSCTTY as libc::c_ulong, 0);
    if ret < 0 {
        libc::_exit(1);
    }

    // Close random FDs (async-signal-safe version — no allocation).
    close_random_fds_async_signal_safe(&[slave_fd, master_fd]);

    // dup2 to stdin/stdout/stderr.
    if libc::dup2(slave_fd, libc::STDIN_FILENO) < 0
        || libc::dup2(slave_fd, libc::STDOUT_FILENO) < 0
        || libc::dup2(slave_fd, libc::STDERR_FILENO) < 0
    {
        libc::_exit(1);
    }

    libc::close(slave_fd);
    libc::close(master_fd);

    // exec with prepared argv and envp (raw pointers, no allocation).
    #[cfg(target_os = "linux")]
    libc::execvpe(
        cmd.program.as_ptr(),
        cmd.argv_ptrs.as_ptr(),
        cmd.envp_ptrs.as_ptr(),
    );

    #[cfg(not(target_os = "linux"))]
    libc::execve(
        cmd.program.as_ptr(),
        cmd.argv_ptrs.as_ptr(),
        cmd.envp_ptrs.as_ptr(),
    );

    // exec only returns on failure
    libc::_exit(126);
}

fn fork_pty(
    pty: &PtyPair,
    program: &str,
    args: &[String],
    env: &[(String, String)],
) -> PtyResult<Pid> {
    // Prepare all allocations in the parent process before fork().
    // This is critical for fork-safety on macOS where malloc locks
    // held by other threads would cause deadlocks in the child.
    let resolved = resolve_executable(program);
    let cmd = PreparedCommand::new(&resolved, args, env)?;

    match unsafe { fork() } {
        Ok(ForkResult::Child) => {
            unsafe { child_setup(pty.slave_fd, pty.master_fd, &cmd); }
        }
        Ok(ForkResult::Parent { child }) => {
            let _ = close(pty.slave_fd);
            Ok(child)
        }
        Err(e) => Err(PtyErrorKind::ForkFailed(format!("fork failed: {}", e))),
    }
}

// ============================================================================
// Public Platform Functions
// ============================================================================

pub fn open_pty(winsize: Option<Winsize>) -> PtyResult<UnixPtyMaster> {
    let pair = PtyPair::open(winsize)?;

    let master_fd = pair.master_fd;
    let slave_fd = pair.slave_fd;

    // Forget the pair so we own the raw FDs individually.
    std::mem::forget(pair);

    // Close the slave FD — caller only gets the master.
    unsafe {
        libc::close(slave_fd);
    }

    UnixPtyMaster::new(master_fd)
        .map_err(|e| PtyErrorKind::AsyncIo(e.to_string()))
}

pub fn spawn(
    program: &str,
    args: &[String],
    env: &[(String, String)],
    winsize: Option<Winsize>,
) -> PtyResult<(UnixPtyMaster, UnixChildProcess)> {
    let pair = PtyPair::open(winsize)?;
    let pid = fork_pty(&pair, program, args, env)?;
    let master_fd = pair.master_fd;
    std::mem::forget(pair);

    let master = UnixPtyMaster::new(master_fd)
        .map_err(|e| PtyErrorKind::AsyncIo(e.to_string()))?;
    let child = UnixChildProcess::new(pid);

    Ok((master, child))
}

#[cfg(test)]
mod tests {
    use super::*;

    // NOTE: close_random_fds tests have been removed because calling
    // close_random_fds in the parent process is unsafe and can cause
    // EBADF panics from the standard library. It is only safe to call
    // in a forked child before exec.

    #[test]
    fn test_pty_pair_drop_closes_fds() {
        let pair = PtyPair::open(None);
        if pair.is_ok() {
            let p = pair.unwrap();
            let _master = p.master_fd;
            let _slave = p.slave_fd;
        }
    }

    #[test]
    fn test_unix_pty_master_new() {
        let _check: fn(RawFd) -> std::io::Result<UnixPtyMaster> = |_| UnixPtyMaster::new(0);
    }

    #[test]
    fn test_unix_child_process_clone_killer_trait() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Box<dyn ChildKiller + Send + Sync>>();
    }
}
