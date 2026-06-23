//! PyO3 bindings for the terminal emulation layer (pyte_rs).
//!
//! Exposes `TerminalState` as a Python class that wraps the Rust `HistoryScreen`
//! + `Parser` combo, providing `feed()`, `display()`, `history_display()`, etc.

use crate::terminal::modes as mo;
use crate::terminal::{HistoryScreen, Parser};
use pyo3::prelude::*;

/// PyO3 wrapper that links the pyte_rs terminal emulator state machine to Python.
///
/// This class manages a VT100/VT220/xterm-compatible terminal screen with
/// scrollback history. Raw bytes from the PTY are fed into the parser,
/// which updates the screen state.
#[pyclass]
pub struct TerminalState {
    screen: HistoryScreen,
    parser: Parser,
}

#[pymethods]
impl TerminalState {
    #[new]
    pub fn new(columns: usize, lines: usize, scrollback_lines: usize) -> Self {
        Self {
            screen: HistoryScreen::new(columns, lines, scrollback_lines),
            parser: Parser::new(),
        }
    }

    /// Receives raw bytes from the PTY and parses them into the terminal state.
    pub fn feed(&mut self, data: &[u8]) {
        self.screen.feed(data);
    }

    /// Get the full display (history + visible screen) as a list of strings.
    pub fn display(&self) -> Vec<String> {
        self.screen.display()
    }

    /// Get just the visible screen as a list of strings.
    pub fn visible_display(&self) -> Vec<String> {
        self.screen.visible_display()
    }

    /// Get the scrollback history as a list of strings.
    pub fn history_display(&self) -> Vec<String> {
        self.screen.history_display()
    }

    /// Get the indices of modified rows (for efficient rendering).
    pub fn dirty(&self) -> Vec<usize> {
        self.screen.dirty().iter().copied().collect()
    }

    /// Resize the terminal screen.
    pub fn resize(&mut self, lines: usize, columns: usize) {
        self.screen.resize(lines, columns);
    }

    /// Reset the terminal to its initial state.
    pub fn reset(&mut self) {
        self.screen.reset();
        self.parser.reset();
    }

    /// Get the cursor X position.
    #[getter]
    pub fn cursor_x(&self) -> usize {
        self.screen.cursor().x
    }

    /// Get the cursor Y position.
    #[getter]
    pub fn cursor_y(&self) -> usize {
        self.screen.cursor().y
    }

    /// Get the window title (from OSC sequences).
    #[getter]
    pub fn title(&self) -> String {
        self.screen.title().to_string()
    }

    /// Get the number of lines in the scrollback history.
    #[getter]
    pub fn history_size(&self) -> usize {
        self.screen.history_size()
    }

    /// Get the scrollback capacity.
    #[getter]
    pub fn scrollback_lines(&self) -> usize {
        self.screen.scrollback_lines()
    }

    /// Set the scrollback capacity.
    pub fn set_scrollback_lines(&mut self, lines: usize) {
        self.screen.set_scrollback_lines(lines);
    }

    /// History + visible buffer as styled cells: (text, fg, bg, attrs_bitmask).
    pub fn styled_viewport(&self) -> Vec<Vec<(String, String, String, u8)>> {
        self.screen.styled_viewport()
    }

    /// Total lines: scrollback history + visible screen.
    pub fn total_lines(&self) -> usize {
        self.screen.total_lines()
    }

    /// Absolute cursor position: (x, history_len + on-screen_y).
    pub fn absolute_cursor(&self) -> (usize, usize) {
        self.screen.absolute_cursor()
    }

    // ── Live terminal mode flags ─────────────────────────────────────────
    //
    // These surface mode state the parser already tracks, so front-ends no
    // longer need to re-scan the raw byte stream to recover them.

    /// DECCKM (?1) — application cursor keys. Affects arrow/Home/End encoding.
    #[getter]
    pub fn app_cursor(&self) -> bool {
        self.screen.mode().has_private(mo::DECCKM)
    }

    /// DECTCEM (?25) — whether the text cursor is visible.
    #[getter]
    pub fn cursor_visible(&self) -> bool {
        self.screen.mode().has_private(mo::DECTCEM)
    }

    /// ?2004 — bracketed paste mode.
    #[getter]
    pub fn bracketed_paste(&self) -> bool {
        self.screen.mode().has_private(mo::BRACKETED_PASTE)
    }

    /// ?1006 — SGR mouse encoding.
    #[getter]
    pub fn sgr_mouse(&self) -> bool {
        self.screen.mode().sgr_mouse()
    }

    /// Whether the alternate screen buffer is currently active (?1049/?1047/?47).
    #[getter]
    pub fn alt_screen(&self) -> bool {
        self.screen.alt_screen()
    }

    /// Highest-precedence active mouse-tracking mode: 1003, 1002, 1000, or 0.
    #[getter]
    pub fn mouse_proto(&self) -> u16 {
        self.screen.mode().mouse_protocol()
    }

    /// Cursor shape from DECSCUSR: "block", "underline", or "bar".
    #[getter]
    pub fn cursor_shape(&self) -> &'static str {
        self.screen.cursor_shape()
    }

    /// Whether the cursor blinks (DECSCUSR steady vs. blinking).
    #[getter]
    pub fn cursor_blink(&self) -> bool {
        self.screen.cursor_blink()
    }

    fn __repr__(&self) -> String {
        format!(
            "TerminalState({}x{}, scrollback={})",
            self.screen.columns(),
            self.screen.lines(),
            self.screen.history_size()
        )
    }
}
