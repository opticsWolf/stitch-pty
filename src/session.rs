//! High-level Rust session API.
//!
//! The crate's platform layer deliberately exposes the raw pieces
//! ([`crate::platform::PtyBackend`], [`crate::platform::ChildBackend`],
//! [`crate::platform::spawn_platform`]); the Python binding
//! then assembles them into `PtySession`. Rust callers previously had to do
//! that assembly themselves and reimplement `write_all`/`terminate`.
//!
//! [`PtySession`] is that assembly, without PyO3:
//!
//! ```no_run
//! use stitch_pty::{PtySession, Winsize};
//! use std::time::Duration;
//!
//! # async fn run() -> Result<(), Box<dyn std::error::Error>> {
//! let session = PtySession::spawn(
//!     "bash",
//!     &["-i".to_string()],
//!     Some(Winsize::new(24, 80, 0, 0)),
//!     None,
//! )
//! .await?;
//!
//! let mut buf = [0u8; 4096];
//! let n = session.read_timeout(&mut buf, Duration::from_secs(5)).await?;
//! // Feed the bytes into the bundled emulator if you want a screen model:
//! // let mut screen = HistoryScreen::new(80, 24, 1000);
//! // screen.feed(&buf[..n]);
//!
//! session.write_all(b"echo hello\n").await?;
//! session.terminate(Duration::from_secs(2)).await?;
//! # Ok(())
//! # }
//! ```
//!
//! The terminal emulator is intentionally *not* owned by the session: a
//! multiplexer wants one emulator per pane, fed from this session's reads, and
//! callers that only relay bytes should not pay for a screen model.

use crate::errors::{PtyErrorKind, PtyResult};
use crate::platform::{ArcChildBackend, ArcPtyBackend, ExitStatus};
use crate::winsize::Winsize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// A child process attached to a PTY.
///
/// Cloning the inner `Arc`s (via [`PtySession::pty`] / [`PtySession::child`])
/// is cheap, so a supervisor can keep the process handle while a reader task
/// owns the PTY.
///
/// Dropping the session drops those handles. The platform child backend kills
/// a still-running child when the last handle is dropped, so clone the `Arc`s
/// if the process should outlive the session.
pub struct PtySession {
    pty: ArcPtyBackend,
    child: ArcChildBackend,
    closed: AtomicBool,
    eof: AtomicBool,
}

impl PtySession {
    /// Spawn `program` in a new PTY, inheriting the parent environment.
    ///
    /// `winsize` defaults to 24×80 when `None`; `cwd` inherits the parent's
    /// working directory when `None`.
    pub async fn spawn(
        program: &str,
        args: &[String],
        winsize: Option<Winsize>,
        cwd: Option<&str>,
    ) -> PtyResult<Self> {
        let (pty, child) =
            crate::platform::spawn_platform(program, args, None, winsize, cwd).await?;
        Ok(Self {
            pty,
            child,
            closed: AtomicBool::new(false),
            eof: AtomicBool::new(false),
        })
    }

    /// Spawn `program` with an explicit environment.
    ///
    /// The slice is the complete child environment — it is not merged with the
    /// parent's. An empty slice spawns with an empty environment; use
    /// [`PtySession::spawn`] to inherit the parent environment instead.
    ///
    /// Duplicate keys collapse to the last occurrence, including its key
    /// spelling and value. Windows compares keys ASCII case-insensitively
    /// (`PATH` and `Path` are one key); Unix compares exact keys. Empty values
    /// are preserved. Explicit entries use `String`, so they cannot represent
    /// non-Unicode keys or values; inheritance does not have that limitation.
    pub async fn spawn_with_env(
        program: &str,
        args: &[String],
        env: &[(String, String)],
        winsize: Option<Winsize>,
        cwd: Option<&str>,
    ) -> PtyResult<Self> {
        let (pty, child) =
            crate::platform::spawn_platform(program, args, Some(env), winsize, cwd).await?;
        Ok(Self {
            pty,
            child,
            closed: AtomicBool::new(false),
            eof: AtomicBool::new(false),
        })
    }

    /// The underlying PTY handle. Use `Arc::clone` to keep it after the
    /// session is dropped.
    pub fn pty(&self) -> &ArcPtyBackend {
        &self.pty
    }

    /// The underlying child-process handle. Use `Arc::clone` to keep it alive
    /// after the session is dropped.
    pub fn child(&self) -> &ArcChildBackend {
        &self.child
    }

    /// Child process id.
    pub fn pid(&self) -> u32 {
        self.child.pid()
    }

    /// True while the child process is running.
    pub fn is_alive(&self) -> bool {
        self.child.is_running()
    }

    /// Read from the PTY.
    ///
    /// EOF is reported as `Ok(0)` when the platform signals it: the Unix
    /// backend maps `EIO` (slave closed) to `Ok(0)`, and ConPTY maps
    /// `ERROR_BROKEN_PIPE` / `ERROR_NO_DATA` to it. On Windows the ConPTY
    /// output pipe can stay open after the child exits, so a read may instead
    /// wait until data arrives or the timeout expires; use [`PtySession::wait`]
    /// or [`PtySession::is_alive`] to detect child exit rather than relying on
    /// EOF alone. After [`PtySession::read_until_exit`] or
    /// [`PtySession::close`], this returns `Ok(0)` immediately. Other I/O
    /// errors are returned as [`PtyErrorKind::AsyncIo`].
    pub async fn read(&self, buf: &mut [u8]) -> PtyResult<usize> {
        if self.eof.load(Ordering::SeqCst) {
            return Ok(0);
        }
        match self.pty.read(buf).await {
            Ok(n) => Ok(n),
            Err(err) => match PtyErrorKind::from_read_error(err) {
                PtyErrorKind::Eof => Ok(0),
                other => Err(other),
            },
        }
    }

    /// Read with a timeout. EOF is reported as `Ok(0)`; a timeout returns
    /// [`PtyErrorKind::Timeout`]. See [`PtySession::read`] for the Windows
    /// caveat: child exit does not necessarily close the ConPTY pipe.
    pub async fn read_timeout(&self, buf: &mut [u8], timeout: Duration) -> PtyResult<usize> {
        match tokio::time::timeout(timeout, self.read(buf)).await {
            Ok(result) => result,
            Err(_) => Err(PtyErrorKind::Timeout(timeout)),
        }
    }

    /// Write bytes, returning the number accepted.
    pub async fn write(&self, data: &[u8]) -> PtyResult<usize> {
        self.pty.write(data).await.map_err(PtyErrorKind::from)
    }

    /// Write the whole buffer, looping over partial writes.
    pub async fn write_all(&self, data: &[u8]) -> PtyResult<()> {
        self.pty.write_all(data).await.map_err(PtyErrorKind::from)
    }

    /// Resize the PTY (rows, cols). The terminal emulator, if any, is owned by
    /// the caller and must be resized separately.
    pub fn resize(&self, rows: u16, cols: u16) -> PtyResult<()> {
        self.pty.set_winsize(Winsize::new(rows, cols, 0, 0))
    }

    /// Set the full window size, including pixel dimensions.
    pub fn set_winsize(&self, winsize: Winsize) -> PtyResult<()> {
        self.pty.set_winsize(winsize)
    }

    /// Current window size.
    pub fn get_winsize(&self) -> PtyResult<Winsize> {
        self.pty.get_winsize()
    }

    /// Wait for the child to exit.
    ///
    /// Returns the exit status, or `None` only if the status was already lost
    /// (for example if the child was reaped externally). Repeated calls after
    /// a normal exit return the same status.
    pub async fn wait(&self) -> Option<ExitStatus> {
        self.child.wait().await
    }

    /// Read until the child exits, then drain remaining output.
    ///
    /// Returns `(output, status)`. Output is accumulated while the child is
    /// alive and for up to `drain_timeout` after exit; the drain stops on EOF
    /// or after a 100 ms quiet period. `status` is `None` only if it was
    /// already lost.
    ///
    /// This is the preferred normal-exit path on Windows, where the ConPTY
    /// output pipe can remain open after the child exits. After it returns,
    /// [`PtySession::read`] returns `Ok(0)`. It does not close the PTY; call
    /// [`PtySession::close`] for explicit teardown.
    pub async fn read_until_exit(
        &self,
        drain_timeout: Duration,
    ) -> PtyResult<(Vec<u8>, Option<ExitStatus>)> {
        let mut output = Vec::new();
        let mut buf = vec![0u8; 8192];
        let wait = self.wait();
        tokio::pin!(wait);
        let status;

        loop {
            tokio::select! {
                biased;
                result = self.read(&mut buf) => match result {
                    Ok(0) => {
                        status = tokio::time::timeout(Duration::from_secs(5), &mut wait)
                            .await
                            .ok()
                            .flatten();
                        break;
                    }
                    Ok(n) => output.extend_from_slice(&buf[..n]),
                    Err(err) => return Err(err),
                },
                result = &mut wait => {
                    status = result;
                    break;
                }
            }
        }

        let deadline = Instant::now() + drain_timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            match tokio::time::timeout(
                remaining.min(Duration::from_millis(100)),
                self.read(&mut buf),
            )
            .await
            {
                Ok(Ok(0)) | Err(_) => break,
                Ok(Ok(n)) => output.extend_from_slice(&buf[..n]),
                Ok(Err(err)) => return Err(err),
            }
        }

        self.eof.store(true, Ordering::SeqCst);
        Ok((output, status))
    }

    /// Close the session and release the PTY.
    ///
    /// A live child is terminated first. Remaining output is drained for a
    /// bounded time (1 s, stopping early on EOF/quiet) so Windows
    /// `ClosePseudoConsole` does not deadlock on an unread pipe. On Windows
    /// the close call runs on a blocking thread because it may wait for the
    /// ConPTY output pipe to drain.
    ///
    /// Idempotent: repeated calls return `Ok(())`. Reads after close return
    /// `Ok(0)`. Do not read concurrently with `close()`. On Unix the master fd
    /// is released when the session is dropped; this call still terminates the
    /// child and drains pending output.
    pub async fn close(&self) -> PtyResult<()> {
        if self.closed.swap(true, Ordering::SeqCst) {
            return Ok(());
        }

        if self.child.is_running() {
            let _ = self.terminate(Duration::from_millis(250)).await;
        }
        // Ensure the reaper/watcher has published the exit before returning;
        // `is_alive()` must be false after a successful close.
        let _ = tokio::time::timeout(Duration::from_secs(1), self.wait()).await;

        let mut buf = [0u8; 8192];
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            match tokio::time::timeout(
                remaining.min(Duration::from_millis(100)),
                self.read(&mut buf),
            )
            .await
            {
                Ok(Ok(0)) | Err(_) => break,
                Ok(Ok(_)) => continue,
                Ok(Err(_)) => break,
            }
        }

        self.eof.store(true, Ordering::SeqCst);
        let pty = self.pty.clone();
        tokio::task::spawn_blocking(move || pty.close())
            .await
            .map_err(|err| PtyErrorKind::AsyncIo(format!("close task failed: {err}")))??;
        Ok(())
    }

    /// Close only the input side, leaving output readable.
    ///
    /// Unix sends a newline plus the terminal `VEOF`; Windows disconnects the
    /// input pipe. Useful for signaling EOF to a line-oriented child without
    /// closing the PTY.
    pub async fn shutdown_write(&self) -> PtyResult<()> {
        self.pty.shutdown_write().await
    }

    /// Force-kill the child.
    pub fn kill(&self) -> PtyResult<()> {
        self.child.kill()
    }

    /// Send a raw signal number to the child's process group.
    ///
    /// Windows supports `9`/`15` (both map to `TerminateProcess`); signal `2`
    /// (SIGINT) returns [`PtyErrorKind::Unsupported`]. POSIX forwards the
    /// signal to the child's process group.
    pub fn send_signal(&self, sig: i32) -> PtyResult<()> {
        self.child.signal(sig)
    }

    /// Send Ctrl+C.
    ///
    /// POSIX: `SIGINT` to the child's process group. Windows: returns
    /// [`PtyErrorKind::Unsupported`] because ConPTY has no reliable
    /// programmatic Ctrl+C; this is an explicit error rather than a
    /// best-effort no-op. Use [`PtySession::terminate`] or
    /// [`PtySession::kill`] to stop the child.
    pub fn interrupt(&self) -> PtyResult<()> {
        self.child.signal(2)
    }

    /// Terminate the child: signal 15, wait up to `grace`, then `kill()`.
    ///
    /// This is the same escalation policy used by the Python binding, with a
    /// tokio sleep injected here and a blocking sleep injected through PyO3.
    /// On Windows signal 15 maps to `TerminateProcess`, so the grace period is
    /// effectively skipped; that is the platform's only option.
    ///
    /// Best effort: signal/kill errors are ignored, and this returns `Ok(())`
    /// even if the child is still alive after the bounded post-kill wait.
    pub async fn terminate(&self, grace: Duration) -> PtyResult<()> {
        crate::terminate::terminate_child(&self.child, grace, |duration| {
            tokio::time::sleep(duration)
        })
        .await;
        Ok(())
    }

    /// Split into the raw backend handles, skipping the session wrapper.
    pub fn into_parts(self) -> (ArcPtyBackend, ArcChildBackend) {
        (self.pty, self.child)
    }
}

impl std::fmt::Debug for PtySession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PtySession")
            .field("pid", &self.child.pid())
            .field("alive", &self.child.is_running())
            .finish()
    }
}
