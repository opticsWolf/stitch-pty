//! Platform abstraction layer for PTY operations
//!
//! Improvements from portable-pty:
//! - `ChildKiller` trait for deadlock-free kill/wait separation
//! - `ExitStatus` struct with `success()`, `exit_code()`, `signal()`
//! - Clean platform dispatch via `spawn_platform` and `open_pty_platform`

use crate::errors::{PtyErrorKind, PtyResult};
use crate::winsize::Winsize;

// ── Exit Status (from portable-pty) ───────────────────────────────

/// Represents the exit status of a child process.
///
/// Improvements from portable-pty:
/// - Separate `exit_code` (u32) and `signal` (`Option<String>`) fields
/// - `success()` method for easy checking
/// - `Display` implementation for human-readable output
#[derive(Clone, Debug)]
pub struct ExitStatus {
    code: u32,
    signal: Option<String>,
    /// Numeric signal retained for the Python compatibility layer. The public
    /// Rust API exposes the portable signal name through [`Self::signal`].
    #[cfg_attr(not(feature = "python"), allow(dead_code))]
    signal_number: Option<i32>,
}

impl ExitStatus {
    /// Construct an ExitStatus from a process return code.
    pub fn with_exit_code(code: u32) -> Self {
        Self {
            code,
            signal: None,
            signal_number: None,
        }
    }

    /// Construct an ExitStatus from a signal name.
    pub fn with_signal(signal: &str) -> Self {
        Self {
            code: 1,
            signal: Some(signal.to_string()),
            signal_number: None,
        }
    }

    /// Construct an ExitStatus from a Unix signal, retaining both the name and
    /// number. The number is only used by the Python API, which historically
    /// exposes signal numbers.
    #[cfg_attr(not(unix), allow(dead_code))]
    pub(crate) fn with_signal_number(signal: &str, number: i32) -> Self {
        Self {
            code: 1,
            signal: Some(signal.to_string()),
            signal_number: Some(number),
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

    /// Exit code for Python's `ExitStatus.exit_code`, which is `None` when the
    /// process was killed by a signal. Rust callers should use
    /// [`Self::exit_code`] plus [`Self::signal`].
    #[cfg_attr(not(feature = "python"), allow(dead_code))]
    pub(crate) fn exit_code_option(&self) -> Option<i32> {
        if self.signal.is_none() {
            Some(self.code as i32)
        } else {
            None
        }
    }

    /// Numeric signal for Python's `ExitStatus.signal`.
    #[cfg_attr(not(feature = "python"), allow(dead_code))]
    pub(crate) fn signal_number(&self) -> Option<i32> {
        self.signal_number
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

// ── ChildKiller Trait (from portable-pty) ─────────────────────────

/// Represents the ability to signal a Child to terminate.
///
/// This trait is separated from ChildBackend to allow the kill capability
/// to be cloned and sent to any thread independently from a thread that
/// may be blocked in `.wait`. This prevents deadlocks.
#[async_trait::async_trait]
pub trait ChildKiller: Send + Sync {
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

    /// Borrow the underlying platform handle for this PTY master.
    ///
    /// This accessor is deliberately platform-gated instead of returning a
    /// unified handle type:
    ///
    /// - Unix returns a borrowed [`std::os::fd::RawFd`]. It is owned by this
    ///   backend and remains valid only while the backend is alive; callers
    ///   must not close it or use it after the backend is dropped.
    /// - Windows returns a borrowed `HPCON` (`*mut c_void`). `HPCON` is
    ///   pseudo-console state, not an owned kernel handle, so it must not be
    ///   wrapped in `AsHandle` or closed with `CloseHandle`. It is valid only
    ///   while this backend is alive.
    ///
    /// The handle is for interoperability (polling, duplication by the owner,
    /// or passing to a platform API). It conveys no ownership and no
    /// cross-platform guarantee; use the structured [`PtyBackend`] methods for
    /// portable operations.
    #[cfg(unix)]
    fn raw_handle(&self) -> std::os::fd::RawFd;

    /// Borrow the underlying platform handle for this PTY master.
    ///
    /// See the Unix overload's contract: the returned handle is borrowed,
    /// valid only while the backend is alive, and must not be closed by the
    /// caller. On Windows this is an `HPCON`, not a kernel file handle.
    #[cfg(windows)]
    fn raw_handle(&self) -> *mut std::ffi::c_void;

    /// Check if the PTY is still open.
    fn is_open(&self) -> bool;

    /// Close the PTY backend.
    ///
    /// The default is a no-op for backends whose handles close on drop. The
    /// Windows ConPTY backend overrides this to call `ClosePseudoConsole`
    /// exactly once; callers must drain unread output first. The call may
    /// block until the ConPTY output pipe is drained, so async callers should
    /// run it on a blocking thread.
    fn close(&self) -> PtyResult<()> {
        Ok(())
    }

    /// Close only the input/write side, leaving output readable.
    ///
    /// Unix sends a newline plus the terminal `VEOF`; Windows disconnects the
    /// input pipe. Backends that cannot express this return
    /// [`PtyErrorKind::Unsupported`].
    async fn shutdown_write(&self) -> PtyResult<()> {
        Err(PtyErrorKind::Unsupported(
            "shutdown_write is not implemented for this PTY backend".to_string(),
        ))
    }

    /// Write the whole buffer, looping over partial writes.
    ///
    /// Returns [`std::io::ErrorKind::WriteZero`] if the backend reports a
    /// zero-byte write. Without that guard the loop would spin forever — a
    /// backend is allowed to return `Ok(0)` and some closed-pipe paths do.
    async fn write_all(&self, mut buf: &[u8]) -> std::io::Result<()> {
        while !buf.is_empty() {
            let n = self.write(buf).await?;
            if n == 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::WriteZero,
                    "PTY write returned 0 bytes",
                ));
            }
            buf = &buf[n.min(buf.len())..];
        }
        Ok(())
    }
}

/// Platform-agnostic child process handle.
#[async_trait::async_trait]
pub trait ChildBackend: ChildKiller + Send + Sync {
    /// Get the child process PID.
    fn pid(&self) -> u32;

    /// Check if the process is still running.
    fn is_running(&self) -> bool;

    /// Wait for the process to exit asynchronously.
    ///
    /// Takes `&self` (not `&mut`) so the future is `Send`-compatible. Returns
    /// the cached [`ExitStatus`] on repeated calls after a normal exit, or
    /// `None` only when the status was already lost (for example, the child was
    /// reaped externally). Callers that need the PID can use [`Self::pid`].
    async fn wait(&self) -> Option<ExitStatus>;

    /// Send a signal to the process.
    fn signal(&self, sig: i32) -> PtyResult<()>;
}

/// Shared handle to a PTY backend.
///
/// This is the type returned by [`spawn_platform`] and
/// [`open_pty_platform`]; naming it used to require spelling out
/// `Arc<dyn PtyBackend>` (the aliases were not exported).
pub type ArcPtyBackend = std::sync::Arc<dyn PtyBackend>;

/// Shared handle to a child-process backend.
pub type ArcChildBackend = std::sync::Arc<dyn ChildBackend>;

// ── Explicit environment normalization ────────────────────────────

/// Collapse duplicate explicit environment keys, exact-key comparison.
///
/// The last occurrence wins and its key spelling/value are retained. `None`
/// inheritance is not normalized; it keeps using the platform's `vars_os()`.
#[cfg_attr(not(unix), allow(dead_code))] // used by the Unix backend; tested on all platforms
pub(crate) fn dedupe_env_exact(env: &[(String, String)]) -> Vec<(&str, &str)> {
    dedupe_env(env, |a, b| a == b)
}

/// Collapse duplicate explicit Windows environment keys.
///
/// Windows environment names are ASCII case-insensitive, so `PATH` and `Path`
/// are the same key. The last occurrence wins and its key spelling/value are
/// retained.
#[cfg_attr(not(windows), allow(dead_code))] // used by the Windows backend; tested on all platforms
pub(crate) fn dedupe_env_windows(env: &[(String, String)]) -> Vec<(&str, &str)> {
    dedupe_env(env, |a, b| a.eq_ignore_ascii_case(b))
}

fn dedupe_env(
    env: &[(String, String)],
    same_key: impl Fn(&str, &str) -> bool,
) -> Vec<(&str, &str)> {
    let mut entries: Vec<(&str, &str)> = Vec::with_capacity(env.len());
    for (key, value) in env {
        if let Some(pos) = entries
            .iter()
            .position(|(existing, _)| same_key(existing, key.as_str()))
        {
            entries.remove(pos);
        }
        entries.push((key.as_str(), value.as_str()));
    }
    entries
}

// ── Platform Dispatch ─────────────────────────────────────────────

/// Validate `cwd` in the parent before spawning: it must be an existing directory.
#[cfg_attr(not(unix), allow(dead_code))] // used by platform_unix::spawn only
///
/// Used by the Unix backend to turn the common typo case into a `ForkFailed`
/// spawn error instead of a silently dead child. (Windows relies on
/// `CreateProcessW` failing with the OS error instead.) The Unix child still
/// performs its own `chdir` (exiting 127 on failure), so a directory deleted
/// between this check and the fork cannot cause silent misbehavior.
pub(crate) fn validate_cwd(cwd: &str) -> PtyResult<()> {
    match std::fs::metadata(cwd) {
        Ok(m) if m.is_dir() => Ok(()),
        Ok(_) => Err(PtyErrorKind::ForkFailed(format!(
            "cwd is not a directory: {:?}",
            cwd
        ))),
        Err(e) => Err(PtyErrorKind::ForkFailed(format!(
            "invalid cwd {:?}: {}",
            cwd, e
        ))),
    }
}

/// Spawn a program in a PTY using the platform-specific backend.
///
/// Windows: async because `NamedPipeServer::connect()` must run in a tokio
/// runtime to arm mio's IOCP pump. Unix: sync wrapped in async (no await).
/// `cwd` only selects the child's working directory; it changes nothing about
/// the async/IOCP plumbing on either platform.
///
/// `winsize` defaults to 24×80 when `None`, on every platform.
///
/// `env` selects the child environment:
///
/// - `None` inherits the parent environment (the usual case).
/// - `Some(&[])` spawns with an empty environment.
/// - `Some(entries)` supplies exactly those variables (no merging). Duplicate
///   keys collapse to the last occurrence, including its key spelling and
///   value. Windows compares keys ASCII case-insensitively (`PATH` and `Path`
///   are one key); Unix compares exact keys. Empty values are preserved.
///
/// An empty slice used to be ambiguous: Unix passed an empty `envp`, while
/// Windows passed a null environment block and inherited. The `Option` makes
/// both intents explicit and keeps the platforms consistent. The inherit path
/// is built from `vars_os()` on Unix and a null block on Windows, so
/// non-Unicode environment entries do not panic. Explicit entries use `String`
/// and therefore cannot represent non-Unicode keys or values.
pub async fn spawn_platform(
    program: &str,
    args: &[String],
    env: Option<&[(String, String)]>,
    winsize: Option<Winsize>,
    cwd: Option<&str>,
) -> PtyResult<(ArcPtyBackend, ArcChildBackend)> {
    let winsize = Some(winsize.unwrap_or_else(|| Winsize::new(24, 80, 0, 0)));

    #[cfg(unix)]
    {
        let (pty, child) = crate::platform_unix::spawn(program, args, env, winsize, cwd)?;
        Ok((
            std::sync::Arc::new(pty),
            std::sync::Arc::new(child) as std::sync::Arc<dyn ChildBackend>,
        ))
    }

    #[cfg(windows)]
    {
        let (pty, child) = crate::platform_windows::spawn(program, args, env, winsize, cwd).await?;
        Ok((pty, child as std::sync::Arc<dyn ChildBackend>))
    }
}

/// Open the PTY master without spawning a child.
///
/// `winsize` defaults to 24×80 when `None`, on every platform.
/// Windows: async for the same IOCP pump reason as `spawn_platform`.
///
/// **Spawner limitation.** This is a master-only API. On Unix the slave fd is
/// closed before return, and on Windows there is no child attached to the
/// ConPTY. It is intended for callers that only need the master side (for
/// example, to wrap a backend through [`crate::PtySession::from_parts`]), not
/// as a general external-spawner API. The supported native spawn route is
/// [`spawn_platform`]; a custom backend can implement [`PtyBackend`] itself and
/// pass it to `from_parts`.
pub async fn open_pty_platform(winsize: Option<Winsize>) -> PtyResult<ArcPtyBackend> {
    let winsize = Some(winsize.unwrap_or_else(|| Winsize::new(24, 80, 0, 0)));

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

    // ── Explicit environment normalization ─────────────────────────

    #[test]
    fn dedupe_explicit_env_last_wins_exact_keys() {
        let env = vec![
            ("A".to_string(), "1".to_string()),
            ("B".to_string(), "2".to_string()),
            ("A".to_string(), "3".to_string()),
            ("Path".to_string(), "case-distinct-on-unix".to_string()),
        ];
        assert_eq!(
            dedupe_env_exact(&env),
            vec![("B", "2"), ("A", "3"), ("Path", "case-distinct-on-unix")]
        );
    }

    #[test]
    fn dedupe_explicit_env_windows_is_case_insensitive() {
        let env = vec![
            ("Path".to_string(), "first".to_string()),
            ("PATH".to_string(), "second".to_string()),
            ("Other".to_string(), "kept".to_string()),
        ];
        assert_eq!(
            dedupe_env_windows(&env),
            vec![("PATH", "second"), ("Other", "kept")]
        );
    }

    #[test]
    fn dedupe_explicit_env_preserves_empty_and_unicode() {
        let env = vec![
            ("EMPTY".to_string(), String::new()),
            ("КЛЮЧ".to_string(), "значение".to_string()),
        ];
        assert_eq!(
            dedupe_env_windows(&env),
            vec![("EMPTY", ""), ("КЛЮЧ", "значение")]
        );
    }

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

    #[test]
    fn test_exit_status_python_mapping_for_code() {
        let es = ExitStatus::with_exit_code(7);
        assert_eq!(es.exit_code_option(), Some(7));
        assert_eq!(es.signal_number(), None);
    }

    #[test]
    fn test_exit_status_python_mapping_for_signal() {
        let es = ExitStatus::with_signal_number("SIGKILL", 9);
        assert_eq!(es.exit_code_option(), None);
        assert_eq!(es.signal_number(), Some(9));
        assert_eq!(es.signal(), Some("SIGKILL"));
    }

    #[test]
    fn test_exit_status_named_signal_has_no_number() {
        // Public name-only construction (tests and callers that do not have a
        // platform signal number) must not invent one.
        let es = ExitStatus::with_signal("SIGTERM");
        assert_eq!(es.exit_code_option(), None);
        assert_eq!(es.signal_number(), None);
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
    fn test_validate_cwd_accepts_dirs() {
        assert!(validate_cwd(std::env::temp_dir().to_str().unwrap()).is_ok());
    }

    #[test]
    fn test_validate_cwd_rejects_missing() {
        match validate_cwd("Z:/definitely/missing/stitch-pty-test") {
            Err(PtyErrorKind::ForkFailed(msg)) => assert!(msg.contains("invalid cwd")),
            _ => panic!("expected ForkFailed"),
        }
    }

    #[test]
    fn test_validate_cwd_rejects_file() {
        // The current test executable exists but is not a directory.
        let exe = std::env::current_exe().unwrap();
        match validate_cwd(exe.to_str().unwrap()) {
            Err(PtyErrorKind::ForkFailed(msg)) => assert!(msg.contains("not a directory")),
            _ => panic!("expected ForkFailed"),
        }
    }

    #[test]
    fn test_platform_dispatch_returns_result() {
        // These functions are async, so we can only check their existence
        // at the type level via a future. The actual calls would need a runtime.
        // This is a compile-time check that the functions exist with the right signature.
        async fn check_spawn() {
            let _f = spawn_platform("", &[], None, None, None);
        }
        async fn check_open() {
            let _f = open_pty_platform(None);
        }
        // Invoke so the checks count as used; the futures are lazy and
        // never polled, so no PTY is actually opened here.
        let _ = (check_spawn(), check_open());
    }

    // ── write_all default implementation ──────────────────────────

    struct StubPty {
        chunk: usize,
        written: std::sync::Mutex<Vec<u8>>,
    }

    #[async_trait::async_trait]
    impl PtyBackend for StubPty {
        async fn read(&self, _buf: &mut [u8]) -> std::io::Result<usize> {
            Ok(0)
        }

        async fn write(&self, buf: &[u8]) -> std::io::Result<usize> {
            let n = self.chunk.min(buf.len());
            self.written.lock().unwrap().extend_from_slice(&buf[..n]);
            Ok(n)
        }

        fn set_winsize(&self, _winsize: Winsize) -> PtyResult<()> {
            Ok(())
        }

        fn get_winsize(&self) -> PtyResult<Winsize> {
            Ok(Winsize::new(24, 80, 0, 0))
        }

        #[cfg(unix)]
        fn raw_handle(&self) -> std::os::fd::RawFd {
            -1
        }

        #[cfg(windows)]
        fn raw_handle(&self) -> *mut std::ffi::c_void {
            std::ptr::null_mut()
        }

        fn is_open(&self) -> bool {
            true
        }
    }

    #[tokio::test]
    async fn write_all_loops_over_partial_writes() {
        let pty = StubPty {
            chunk: 3,
            written: std::sync::Mutex::new(Vec::new()),
        };
        pty.write_all(b"abcdefgh").await.unwrap();
        assert_eq!(pty.written.into_inner().unwrap(), b"abcdefgh");
    }

    #[tokio::test]
    async fn write_all_rejects_zero_byte_writes() {
        let pty = StubPty {
            chunk: 0,
            written: std::sync::Mutex::new(Vec::new()),
        };
        let err = pty.write_all(b"abc").await.unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::WriteZero);
    }
}
