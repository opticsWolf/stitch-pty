//! High-level Rust session API.
//!
//! The crate's platform layer deliberately exposes the raw pieces
//! ([`PtyBackend`], [`ChildBackend`], [`spawn_platform`]); the Python binding
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
use std::time::Duration;

/// A child process attached to a PTY.
///
/// Cloning the inner `Arc`s (via [`PtySession::pty`] / [`PtySession::child`])
/// is cheap, so a supervisor can keep the process handle while a reader task
/// owns the PTY.
pub struct PtySession {
    pty: ArcPtyBackend,
    child: ArcChildBackend,
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
        Ok(Self { pty, child })
    }

    /// Spawn `program` with an explicit environment.
    ///
    /// The slice is the complete child environment — it is not merged with the
    /// parent's. An empty slice spawns with an empty environment; use
    /// [`PtySession::spawn`] to inherit the parent environment instead.
    pub async fn spawn_with_env(
        program: &str,
        args: &[String],
        env: &[(String, String)],
        winsize: Option<Winsize>,
        cwd: Option<&str>,
    ) -> PtyResult<Self> {
        let (pty, child) =
            crate::platform::spawn_platform(program, args, Some(env), winsize, cwd).await?;
        Ok(Self { pty, child })
    }

    /// The underlying PTY handle (cheap `Arc` clone when returned by value).
    pub fn pty(&self) -> &ArcPtyBackend {
        &self.pty
    }

    /// The underlying child-process handle.
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
    /// EOF alone. Other I/O errors are returned as
    /// [`PtyErrorKind::AsyncIo`].
    pub async fn read(&self, buf: &mut [u8]) -> PtyResult<usize> {
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
    /// Returns `None` if the child was already reaped (for example by a
    /// previous `wait`), matching the Python API.
    pub async fn wait(&self) -> Option<ExitStatus> {
        self.child.wait_status().await
    }

    /// Force-kill the child.
    pub fn kill(&self) -> PtyResult<()> {
        self.child.kill()
    }

    /// Send a raw signal number to the child's process group.
    pub fn send_signal(&self, sig: i32) -> PtyResult<()> {
        self.child.signal(sig)
    }

    /// Send Ctrl+C.
    ///
    /// POSIX: `SIGINT` to the child's process group. Windows: delegates to
    /// [`ChildBackend::signal`] (`GenerateConsoleCtrlEvent(CTRL_C_EVENT, 0)`),
    /// which targets the caller's console group rather than the ConPTY child;
    /// see the Windows notes in the README before relying on it.
    pub fn interrupt(&self) -> PtyResult<()> {
        self.child.signal(2)
    }

    /// Terminate the child: signal 15, wait up to `grace`, then `kill()`.
    ///
    /// On Windows signal 15 maps to `TerminateProcess`, so the grace period is
    /// effectively skipped; that is the platform's only option.
    pub async fn terminate(&self, grace: Duration) -> PtyResult<()> {
        let _ = self.child.signal(15);
        let deadline = std::time::Instant::now() + grace;
        while self.child.is_running() && std::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        if self.child.is_running() {
            let _ = self.child.kill();
            // Bounded wait for the reaper; do not hang a supervisor on a
            // process that refuses to die.
            let kill_deadline = std::time::Instant::now() + Duration::from_secs(2);
            while self.child.is_running() && std::time::Instant::now() < kill_deadline {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }
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
