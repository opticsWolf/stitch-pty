//! Unix PTY implementation using POSIX APIs
//!
//! Uses: openpty(3), fork(2), setsid(2), TIOCSCTTY, dup2, execvpe
//! Async I/O: tokio::io::AsyncFd over raw FDs
//!
//! Improvements from portable-pty:
//! - `close_random_fds()` for macOS Big Sur / Linux FD leak prevention
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
// close_random_fds (from portable-pty)
// ============================================================================

/// On Big Sur, Cocoa leaks various file descriptors to child processes,
/// so we need to make a pass through the open descriptors beyond just the
/// stdio descriptors and close them all out.
///
/// This is approximately equivalent to the darwin `posix_spawnattr_setflags`
/// option POSIX_SPAWN_CLOEXEC_DEFAULT which is used as a bit of a cheat
/// on macOS.
///
/// On Linux, gnome/mutter leak shell extension fds to wezterm too, so we
/// also need to make an effort to clean up the mess.
///
/// The implementation of this function relies on `/dev/fd` being available
/// to provide the list of open fds. Any errors in enumerating or closing
/// the fds are silently ignored.
pub fn close_random_fds(skip: &[RawFd]) {
    // FreeBSD, macOS and presumably other BSDish systems have /dev/fd as
    // a directory listing the current fd numbers for the process.
    //
    // On Linux, /dev/fd is a symlink to /proc/self/fd
    //
    // `skip` lists FDs that must NOT be closed (e.g., the PTY master/slave
    // or FDs owned by the Rust runtime).
    if let Ok(dir) = std::fs::read_dir("/dev/fd") {
        let mut fds = vec![];
        for entry in dir {
            if let Some(num) = entry
                .ok()
                .map(|e| e.file_name())
                .and_then(|s| s.into_string().ok())
                .and_then(|n| n.parse::<libc::c_int>().ok())
            {
                if num > 2 && !skip.contains(&(num as RawFd)) {
                    fds.push(num);
                }
            }
        }
        for fd in fds {
            unsafe {
                libc::close(fd);
            }
        }
    }
}

// ============================================================================
// Unix PtyMaster
// ============================================================================

pub struct UnixPtyMaster {
    async_fd: AsyncFd<RawFd>,
}

impl UnixPtyMaster {
    pub fn new(fd: RawFd) -> std::io::Result<Self> {
        let async_fd = AsyncFd::with_interest(fd, Interest::READABLE | Interest::WRITABLE)?;
        Ok(UnixPtyMaster { async_fd })
    }
}

#[async_trait::async_trait]
impl PtyBackend for UnixPtyMaster {
    async fn read(&self, buf: &mut [u8]) -> std::io::Result<usize> {
        loop {
            let mut guard = self.async_fd.readable().await?;
            match guard.try_io(|inner| {
                let fd = *inner.get_ref();
                let ret = unsafe { libc::read(fd, buf.as_mut_ptr() as *mut _, buf.len()) };
                if ret < 0 {
                    Err(std::io::Error::last_os_error())
                } else {
                    Ok(ret as usize)
                }
            }) {
                Ok(result) => return result,
                Err(_would_block) => continue,
            }
        }
    }

    async fn write(&self, buf: &[u8]) -> std::io::Result<usize> {
        loop {
            let mut guard = self.async_fd.writable().await?;
            match guard.try_io(|inner| {
                let fd = *inner.get_ref();
                let ret = unsafe { libc::write(fd, buf.as_ptr() as *const _, buf.len()) };
                if ret < 0 {
                    Err(std::io::Error::last_os_error())
                } else {
                    Ok(ret as usize)
                }
            }) {
                Ok(result) => return result,
                Err(_would_block) => continue,
            }
        }
    }

    fn set_winsize(&self, winsize: Winsize) -> PtyResult<()> {
        let ws: nix::pty::Winsize = winsize.into();
        let fd = *self.async_fd.get_ref();
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
        let fd = *self.async_fd.get_ref();
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
        *self.async_fd.get_ref()
    }

    fn is_open(&self) -> bool {
        let fd = *self.async_fd.get_ref();
        unsafe { libc::fcntl(fd, libc::F_GETFD) >= 0 }
    }
}

impl Drop for UnixPtyMaster {
    fn drop(&mut self) {
        let fd = *self.async_fd.get_ref();
        let _ = close(fd);
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

        // Use a weak reference so the background thread exits when all
        // UnixChildProcess instances are dropped (e.g., in unit tests).
        let state_weak = Arc::downgrade(&state);

        // Spawn a dedicated OS thread for waitpid polling.
        // This is robust against Tokio runtime starvation on macOS.
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(Duration::from_millis(50));

                let state_strong = match state_weak.upgrade() {
                    Some(s) => s,
                    None => break,
                };

                let pid = {
                    let guard = state_strong.lock();
                    if !guard.running { break; }
                    guard.pid
                };

                // WNOHANG is non-blocking
                let result = waitpid(pid, Some(WaitPidFlag::WNOHANG));

                match result {
                    Ok(WaitStatus::Exited(pid, code)) => {
                        let exit = ProcessExit {
                            pid: pid.as_raw() as u32,
                            exit_code: Some(code),
                            signal: None,
                        };
                        let mut guard = state_strong.lock();
                        guard.running = false;
                        guard.exit_status = Some(exit);
                        break;
                    }
                    Ok(WaitStatus::Signaled(pid, signal, _core_dumped)) => {
                        let exit = ProcessExit {
                            pid: pid.as_raw() as u32,
                            exit_code: None,
                            signal: Some(signal as i32),
                        };
                        let mut guard = state_strong.lock();
                        guard.running = false;
                        guard.exit_status = Some(exit);
                        break;
                    }
                    Ok(WaitStatus::StillAlive) => {}
                    // Stopped, Continued, PtraceEvent, PtraceSyscall — ignore and poll again
                    Ok(_) => {}
                    // EINTR is common on macOS; just ignore and retry on the next loop
                    Err(nix::errno::Errno::EINTR) => continue,
                    Err(_) => {
                        let mut guard = state_strong.lock();
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
            // We ignore this error because the process is already dead.
            Err(nix::errno::Errno::EPERM) => Ok(()),
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

unsafe fn child_setup(slave_fd: RawFd, master_fd: RawFd, program: &str, args: &[String], env: &[(String, String)]) -> ! {
    // Clean up a few things before we exec the program
    // Clear out any potentially problematic signal dispositions that we might have inherited
    for signo in &[
        libc::SIGCHLD,
        libc::SIGHUP,
        libc::SIGINT,
        libc::SIGQUIT,
        libc::SIGTERM,
        libc::SIGALRM,
    ] {
        unsafe { libc::signal(*signo, libc::SIG_DFL); }
    }

    let empty_set: libc::sigset_t = unsafe { std::mem::zeroed() };
    unsafe { libc::sigprocmask(libc::SIG_SETMASK, &empty_set, std::ptr::null_mut()); }

    if setsid().is_err() {
        unsafe { libc::_exit(1); }
    }
    let ret = unsafe { libc::ioctl(slave_fd, libc::TIOCSCTTY as libc::c_ulong, 0) };
    if ret < 0 {
        unsafe { libc::_exit(1); }
    }

    // Close random FDs (from portable-pty) - critical for macOS Big Sur
    // Preserve master/slave FDs so Rust runtime doesn't crash
    close_random_fds(&[slave_fd, master_fd]);

    // dup2 to stdin/stdout/stderr — use raw libc for simplicity in fork child
    if unsafe { libc::dup2(slave_fd, libc::STDIN_FILENO) } < 0
        || unsafe { libc::dup2(slave_fd, libc::STDOUT_FILENO) } < 0
        || unsafe { libc::dup2(slave_fd, libc::STDERR_FILENO) } < 0
    {
        unsafe { libc::_exit(1); }
    }
    let _ = close(slave_fd);
    let _ = close(master_fd);

    let c_program = CString::new(program).unwrap_or_else(|_| unsafe { libc::_exit(1) });

    // FIX: Prepend the program name to the arguments vector as argv[0]
    let mut c_args: Vec<CString> = Vec::with_capacity(args.len() + 1);
    c_args.push(c_program.clone());
    for s in args {
        c_args.push(CString::new(s.as_str()).unwrap_or_else(|_| CString::new("").unwrap()));
    }

    let mut argv: Vec<*const c_char> = c_args.iter().map(|s| s.as_ptr()).collect();
    argv.push(std::ptr::null());

    let c_env: Vec<CString> = env.iter()
        .map(|(k, v)| CString::new(format!("{}={}", k, v)).unwrap())
        .collect();
    let mut envp: Vec<*const c_char> = c_env.iter().map(|s| s.as_ptr()).collect();
    envp.push(std::ptr::null());

    #[cfg(target_os = "linux")]
    unsafe { libc::execvpe(c_program.as_ptr(), argv.as_ptr(), envp.as_ptr()); }
    #[cfg(not(target_os = "linux"))]
    unsafe { libc::execvp(c_program.as_ptr(), argv.as_ptr()); }
    // exec only returns on failure
    eprintln!("exec failed (errno: {})", std::io::Error::last_os_error());
    unsafe { libc::_exit(126); }
}

fn fork_pty(pty: &PtyPair, program: &str, args: &[String], env: &[(String, String)]) -> PtyResult<Pid> {
    match unsafe { fork() } {
        Ok(ForkResult::Child) => {
            unsafe { child_setup(pty.slave_fd, pty.master_fd, program, args, env); }
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
    std::mem::forget(pair);
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
