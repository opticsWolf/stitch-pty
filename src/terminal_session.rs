//! A Rust session that owns both the PTY transport and a terminal screen.
//!
//! [`crate::PtySession`] deliberately stays emulator-free: a multiplexer can
//! feed its bytes into any screen model. `TerminalSession` is the convenience
//! for callers that want one owned [`HistoryScreen`] without writing the
//! feed/resize plumbing themselves.

use crate::errors::PtyResult;
use crate::platform::ExitStatus;
use crate::session::PtySession;
use crate::terminal::HistoryScreen;
use crate::winsize::Winsize;
use std::time::Duration;

/// PTY I/O plus an owned terminal emulator.
///
/// Reads feed the screen exactly once. Use [`Self::into_parts`] to take the
/// untouched [`PtySession`] back if a caller wants to manage the emulator
/// itself. This type does not depend on PyO3; the Python wrapper keeps its own
/// terminal state for the Python API.
pub struct TerminalSession {
    session: PtySession,
    screen: HistoryScreen,
    /// True once a read has fed the screen; used only for documentation-level
    /// intent, not behavior.
    fed_any: bool,
}

impl TerminalSession {
    /// Spawn a child and create a screen initialized at the PTY winsize.
    ///
    /// `winsize = None` uses 24×80, matching [`PtySession::spawn`].
    pub async fn spawn(
        program: &str,
        args: &[String],
        winsize: Option<Winsize>,
        cwd: Option<&str>,
        scrollback: usize,
    ) -> PtyResult<Self> {
        let size = winsize.unwrap_or_else(|| Winsize::new(24, 80, 0, 0));
        let session = PtySession::spawn(program, args, Some(size), cwd).await?;
        Ok(Self::new(session, size, scrollback))
    }

    /// Spawn with an explicit environment; see [`PtySession::spawn_with_env`].
    pub async fn spawn_with_env(
        program: &str,
        args: &[String],
        env: &[(String, String)],
        winsize: Option<Winsize>,
        cwd: Option<&str>,
        scrollback: usize,
    ) -> PtyResult<Self> {
        let size = winsize.unwrap_or_else(|| Winsize::new(24, 80, 0, 0));
        let session = PtySession::spawn_with_env(program, args, env, Some(size), cwd).await?;
        Ok(Self::new(session, size, scrollback))
    }

    /// Wrap an existing session, reading the current winsize for the screen.
    pub fn from_session(session: PtySession, scrollback: usize) -> Self {
        let size = session
            .get_winsize()
            .unwrap_or_else(|_| Winsize::new(24, 80, 0, 0));
        Self::new(session, size, scrollback)
    }

    /// Wrap a session and a screen supplied by the caller.
    pub fn from_parts(session: PtySession, screen: HistoryScreen) -> Self {
        Self {
            session,
            screen,
            fed_any: false,
        }
    }

    fn new(session: PtySession, size: Winsize, scrollback: usize) -> Self {
        let screen = HistoryScreen::new(size.cols as usize, size.rows as usize, scrollback);
        Self {
            session,
            screen,
            fed_any: false,
        }
    }

    /// The transport session.
    pub fn session(&self) -> &PtySession {
        &self.session
    }

    /// The owned screen.
    pub fn screen(&self) -> &HistoryScreen {
        &self.screen
    }

    /// The owned screen for advanced direct access.
    ///
    /// Feeding bytes manually through this accessor bypasses the automatic
    /// read-into-screen path and can double-feed; prefer [`Self::read`].
    pub fn screen_mut(&mut self) -> &mut HistoryScreen {
        &mut self.screen
    }

    /// Read from the PTY and feed the bytes into the screen exactly once.
    pub async fn read(&mut self, buf: &mut [u8]) -> PtyResult<usize> {
        let n = self.session.read(buf).await?;
        if n > 0 {
            self.screen.feed(&buf[..n]);
            self.fed_any = true;
        }
        Ok(n)
    }

    /// Read with a timeout and feed the bytes into the screen.
    pub async fn read_timeout(&mut self, buf: &mut [u8], timeout: Duration) -> PtyResult<usize> {
        let n = self.session.read_timeout(buf, timeout).await?;
        if n > 0 {
            self.screen.feed(&buf[..n]);
            self.fed_any = true;
        }
        Ok(n)
    }

    /// Write bytes to the PTY.
    pub async fn write(&self, data: &[u8]) -> PtyResult<usize> {
        self.session.write(data).await
    }

    /// Write all bytes to the PTY.
    pub async fn write_all(&self, data: &[u8]) -> PtyResult<()> {
        self.session.write_all(data).await
    }

    /// Resize both the PTY and the screen.
    pub fn resize(&mut self, rows: u16, cols: u16) -> PtyResult<()> {
        self.session.resize(rows, cols)?;
        self.screen.resize(rows as usize, cols as usize);
        Ok(())
    }

    /// Set the full winsize on both the PTY and the screen.
    pub fn set_winsize(&mut self, winsize: Winsize) -> PtyResult<()> {
        self.session.set_winsize(winsize)?;
        self.screen
            .resize(winsize.rows as usize, winsize.cols as usize);
        Ok(())
    }

    /// Current winsize from the PTY.
    pub fn get_winsize(&self) -> PtyResult<Winsize> {
        self.session.get_winsize()
    }

    /// Wait for the child to exit.
    pub async fn wait(&self) -> Option<ExitStatus> {
        self.session.wait().await
    }

    /// Terminate the child with the shared escalation policy.
    pub async fn terminate(&self, grace: Duration) -> PtyResult<()> {
        self.session.terminate(grace).await
    }

    /// Force-kill the child.
    pub fn kill(&self) -> PtyResult<()> {
        self.session.kill()
    }

    /// Send a signal to the child.
    pub fn send_signal(&self, sig: i32) -> PtyResult<()> {
        self.session.send_signal(sig)
    }

    /// True while the child is running.
    pub fn is_alive(&self) -> bool {
        self.session.is_alive()
    }

    /// Child PID.
    pub fn pid(&self) -> u32 {
        self.session.pid()
    }

    /// Visible display rows.
    pub fn display(&self) -> Vec<String> {
        self.screen.visible_display()
    }

    /// Scrollback + visible rows.
    pub fn full_display(&self) -> Vec<String> {
        self.screen.display()
    }

    /// Scrollback rows.
    pub fn scrollback_display(&self) -> Vec<String> {
        self.screen.history_display()
    }

    /// Styled visible + history buffer.
    pub fn styled_viewport(&self) -> Vec<Vec<(String, String, String, u8)>> {
        self.screen.styled_viewport()
    }

    /// Drain dirty rows since the last call.
    pub fn take_dirty_rows(&mut self) -> Vec<usize> {
        self.screen.take_dirty_rows()
    }

    /// Drain edge-triggered BEL.
    pub fn take_bell(&mut self) -> bool {
        self.screen.take_bell()
    }

    /// Drain ordered terminal events.
    pub fn take_events(&mut self) -> Vec<crate::terminal::events::TermEvent> {
        self.screen.take_events()
    }

    /// Current Kitty keyboard flags.
    pub fn keyboard_mode(&self) -> u16 {
        self.screen.keyboard_mode()
    }

    /// Encode a key event using the screen's current Kitty flags.
    pub fn encode_key(&self, event: &crate::terminal::keyboard::KeyEvent) -> Vec<u8> {
        self.screen.encode_key(event)
    }

    /// Split back into the transport session and the screen.
    pub fn into_parts(self) -> (PtySession, HistoryScreen) {
        (self.session, self.screen)
    }

    /// True after at least one read has fed the screen.
    pub fn has_fed(&self) -> bool {
        self.fed_any
    }
}

impl std::fmt::Debug for TerminalSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TerminalSession")
            .field("pid", &self.session.pid())
            .field("alive", &self.session.is_alive())
            .field("columns", &self.screen.columns())
            .field("lines", &self.screen.lines())
            .finish()
    }
}
