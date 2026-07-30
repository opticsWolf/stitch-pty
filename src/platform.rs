//! Platform abstraction layer for PTY operations
//!
//! Improvements from portable-pty:
//! - `ChildKiller` trait for deadlock-free kill/wait separation
//! - `ExitStatus` struct with `success()`, `exit_code()`, `signal()`
//! - Clean platform dispatch via `spawn_platform` and `open_pty_platform`

use crate::errors::PtyResult;
use crate::winsize::Winsize;

// ── Exit Status (from portable-pty) ───────────────────────────────

/// Represents the exit status of a child process.
///
/// Improvements from portable-pty:
/// - Separate `exit_code` (u32) and `signal` (Option<String>) fields
/// - `success()` method for easy checking
/// - `Display` implementation for human-readable output
#[derive(Clone, Debug)]
pub struct ExitStatus {
    code: u32,
    signal: Option<String>,
}

impl ExitStatus {
    /// Construct an ExitStatus from a process return code.
    pub fn with_exit_code(code: u32) -> Self {
        Self { code, signal: None }
    }

    /// Construct an ExitStatus from a signal.
    pub fn with_signal(signal: &str) -> Self {
        Self {
            code: 1,
            signal: Some(signal.to_string()),
        }
    }

    /// Returns true if the status indicates successful completion.
    pub fn success(&self) -> bool {
        match &self.signal {
            None => self.code == 0,
            Some(_) => false,
        }
    }

    /// Returns the exit code.
    pub fn exit_code(&self) -> u32 {
        self.code
    }

    /// Returns the signal name if the process was terminated by a signal.
    pub fn signal(&self) -> Option<&str> {
        self.signal.as_deref()
    }
}

impl std::fmt::Display for ExitStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        if self.success() {
            write!(f, "Success")
        } else {
            match &self.signal {
                Some(sig) => write!(f, "Terminated by {}", sig),
                None => write!(f, "Exited with code {}", self.code),
            }
        }
    }
}

// ── ProcessExit (legacy alias for Python API) ─────────────────────

/// Legacy struct used by the Python API for backward compatibility.
/// Internally wraps ExitStatus concepts.
#[derive(Clone, Debug)]
pub struct ProcessExit {
    pub pid: u32,
    pub exit_code: Option<i32>,
    pub signal: Option<i32>,
}

impl ProcessExit {
    /// Convert to the new ExitStatus type.
    pub fn to_exit_status(&self) -> ExitStatus {
        match (self.exit_code, &self.signal) {
            (Some(code), None) => ExitStatus::with_exit_code(code as u32),
            (None, Some(sig)) => ExitStatus::with_signal(&format!("SIG{}", sig)),
            _ => ExitStatus::with_exit_code(1),
        }
    }
}

// ── ChildKiller Trait (from portable-pty) ─────────────────────────

/// Represents the ability to signal a Child to terminate.
///
/// This trait is separated from ChildBackend to allow the kill capability
/// to be cloned and sent to any thread independently from a thread that
/// may be blocked in `.wait`. This prevents deadlocks.
#[async_trait::async_trait]
pub trait ChildKiller: Send + Sync {
    /// Get the child process PID.
    fn pid(&self) -> u32;

    /// Terminate the child process.
    fn kill(&self) -> PtyResult<()>;

    /// Clone an object that can be split out from the Child in order
    /// to send it signals independently from a thread that may be
    /// blocked in `.wait`.
    fn clone_killer(&self) -> Box<dyn ChildKiller + Send + Sync>;
}

// ── PTY Backend Traits ────────────────────────────────────────────

/// Platform-agnostic PTY backend trait.
#[async_trait::async_trait]
pub trait PtyBackend: Send + Sync {
    /// Read data from the PTY master asynchronously.
    async fn read(&self, buf: &mut [u8]) -> std::io::Result<usize>;

    /// Write data to the PTY master asynchronously.
    async fn write(&self, buf: &[u8]) -> std::io::Result<usize>;

    /// Set the terminal window size.
    fn set_winsize(&self, winsize: Winsize) -> PtyResult<()>;

    /// Get the current terminal window size.
    fn get_winsize(&self) -> PtyResult<Winsize>;

    /// Get the underlying raw handle/FD.
    #[cfg(unix)]
    fn raw_handle(&self) -> std::os::fd::RawFd;

    #[cfg(windows)]
    fn raw_handle(&self) -> *mut std::ffi::c_void;

    /// Check if the PTY is still open.
    fn is_open(&self) -> bool;
}

/// Platform-agnostic child process handle.
#[async_trait::async_trait]
pub trait ChildBackend: ChildKiller + Send + Sync {
    /// Get the child process PID.
    fn pid(&self) -> u32;

    /// Check if the process is still running.
    fn is_running(&self) -> bool;

    /// Wait for the process to exit asynchronously.
    /// Takes &self (not &mut) so the future is Send-compatible.
    async fn wait(&self) -> Option<ProcessExit>;

    /// Send a signal to the process.
    fn signal(&self, sig: i32) -> PtyResult<()>;
}

// ── Platform Dispatch ─────────────────────────────────────────────

/// Spawn a program in a PTY using the platform-specific backend.
///
/// Windows: async because `NamedPipeServer::connect()` must run in a tokio
/// runtime to arm mio's IOCP pump. Unix: sync wrapped in async (no await).
pub async fn spawn_platform(
    program: &str,
    args: &[String],
    env: &[(String, String)],
    winsize: Option<Winsize>,
) -> PtyResult<(std::sync::Arc<dyn PtyBackend>, std::sync::Arc<dyn ChildBackend>)> {
    #[cfg(unix)]
    {
        let (pty, child) = crate::platform_unix::spawn(program, args, env, winsize)?;
        Ok((std::sync::Arc::new(pty), std::sync::Arc::new(child) as std::sync::Arc<dyn ChildBackend>))
    }

    #[cfg(windows)]
    {
        let (pty, child) = crate::platform_windows::spawn(program, args, env, winsize).await?;
        Ok((pty, child as std::sync::Arc<dyn ChildBackend>))
    }
}

/// Open a PTY pair without spawning a child.
///
/// Windows: async for the same IOCP pump reason as `spawn_platform`.
pub async fn open_pty_platform(winsize: Option<Winsize>) -> PtyResult<std::sync::Arc<dyn PtyBackend>> {
    #[cfg(unix)]
    {
        let pty = crate::platform_unix::open_pty(winsize)?;
        Ok(std::sync::Arc::new(pty))
    }

    #[cfg(windows)]
    {
        let pty = crate::platform_windows::open_pty(winsize).await?;
        Ok(pty)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── ExitStatus Tests ───────────────────────────────────────────

    #[test]
    fn test_exit_status_success_zero() {
        let es = ExitStatus::with_exit_code(0);
        assert!(es.success());
        assert_eq!(es.exit_code(), 0);
        assert!(es.signal().is_none());
    }

    #[test]
    fn test_exit_status_failure_nonzero() {
        let es = ExitStatus::with_exit_code(1);
        assert!(!es.success());
        assert_eq!(es.exit_code(), 1);
        assert!(es.signal().is_none());
    }

    #[test]
    fn test_exit_status_large_code() {
        let es = ExitStatus::with_exit_code(255);
        assert!(!es.success());
        assert_eq!(es.exit_code(), 255);
    }

    #[test]
    fn test_exit_status_from_signal() {
        let es = ExitStatus::with_signal("SIGTERM");
        assert!(!es.success());
        assert_eq!(es.signal(), Some("SIGTERM"));
        // Signal-based exit status has code=1
        assert_eq!(es.exit_code(), 1);
    }

    #[test]
    fn test_exit_status_display_success() {
        let es = ExitStatus::with_exit_code(0);
        assert_eq!(format!("{}", es), "Success");
    }

    #[test]
    fn test_exit_status_display_failure_code() {
        let es = ExitStatus::with_exit_code(42);
        assert_eq!(format!("{}", es), "Exited with code 42");
    }

    #[test]
    fn test_exit_status_display_signal() {
        let es = ExitStatus::with_signal("SIGKILL");
        assert_eq!(format!("{}", es), "Terminated by SIGKILL");
    }

    #[test]
    fn test_exit_status_clone() {
        let es1 = ExitStatus::with_exit_code(42);
        let es2 = es1.clone();
        assert_eq!(es1.exit_code(), es2.exit_code());
        assert_eq!(es1.signal(), es2.signal());
    }

    #[test]
    fn test_exit_status_debug() {
        let es = ExitStatus::with_exit_code(1);
        let debug = format!("{:?}", es);
        assert!(debug.contains("42") || debug.contains("1"));
    }

    // ── ProcessExit Tests ─────────────────────────────────────────

    #[test]
    fn test_process_exit_to_exit_status_code() {
        let pe = ProcessExit { pid: 1234, exit_code: Some(0), signal: None };
        let es = pe.to_exit_status();
        assert!(es.success());
        assert_eq!(es.exit_code(), 0);
    }

    #[test]
    fn test_process_exit_to_exit_status_failure() {
        let pe = ProcessExit { pid: 1234, exit_code: Some(1), signal: None };
        let es = pe.to_exit_status();
        assert!(!es.success());
        assert_eq!(es.exit_code(), 1);
    }

    #[test]
    fn test_process_exit_to_exit_status_signal() {
        let pe = ProcessExit { pid: 1234, exit_code: None, signal: Some(9) };
        let es = pe.to_exit_status();
        assert!(!es.success());
        assert_eq!(es.signal(), Some("SIG9"));
    }

    #[test]
    fn test_process_exit_to_exit_status_both_none() {
        let pe = ProcessExit { pid: 1234, exit_code: None, signal: None };
        let es = pe.to_exit_status();
        assert!(!es.success());
        assert_eq!(es.exit_code(), 1);
    }

    #[test]
    fn test_process_exit_debug() {
        let pe = ProcessExit { pid: 42, exit_code: Some(0), signal: None };
        let debug = format!("{:?}", pe);
        assert!(debug.contains("42"));
    }

    #[test]
    fn test_process_exit_clone() {
        let pe1 = ProcessExit { pid: 42, exit_code: Some(0), signal: None };
        let pe2 = pe1.clone();
        assert_eq!(pe1.pid, pe2.pid);
        assert_eq!(pe1.exit_code, pe2.exit_code);
    }

    // ── ChildKiller trait concept Tests ───────────────────────────

    #[test]
    fn test_child_killer_trait_exists() {
        // Verify the trait is Send + Sync by checking the bound
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Box<dyn ChildKiller + Send + Sync>>();
    }

    // ── PtyBackend trait concept Tests ────────────────────────────

    #[test]
    fn test_pty_backend_trait_exists() {
        // Verify the trait is Send + Sync
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Box<dyn PtyBackend + Send + Sync>>();
    }

    // ── spawn_platform / open_pty_platform concept Tests ──────────

    #[test]
    fn test_platform_dispatch_returns_result() {
        // These functions are async, so we can only check their existence
        // at the type level via a future. The actual calls would need a runtime.
        // This is a compile-time check that the functions exist with the right signature.
        async fn check_spawn() {
            let _f = spawn_platform("", &[], &[], None);
        }
        async fn check_open() {
            let _f = open_pty_platform(None);
        }
    }
}
