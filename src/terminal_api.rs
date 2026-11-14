//! PyO3 bindings for the terminal emulation layer (pyte_rs).
//!
//! Exposes `TerminalState` as a Python class that wraps the Rust `HistoryScreen`
//! + `Parser` combo, providing `feed()`, `display()`, `history_display()`, etc.

use crate::terminal::HistoryScreen;
use crate::terminal::keyboard::{
    KeyCode, KeyEvent, KeyEventKind, KeyboardFlags, Modifiers, encode_key,
};
use crate::terminal::modes as mo;
use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyBytes;

/// PyO3 wrapper that links the pyte_rs terminal emulator state machine to Python.
///
/// This class manages a VT100/VT220/xterm-compatible terminal screen with
/// scrollback history. Raw bytes from the PTY are fed into the parser,
/// which updates the screen state.
#[pyclass]
pub struct TerminalState {
    screen: HistoryScreen,
}

#[pymethods]
impl TerminalState {
    #[new]
    pub fn new(columns: usize, lines: usize, scrollback_lines: usize) -> Self {
        Self {
            screen: HistoryScreen::new(columns, lines, scrollback_lines),
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

    /// Drain the dirty-row set: sorted indices of rows modified since the
    /// last call, leaving it empty. Coalesces repeated writes to one row.
    /// Consuming alternative to `dirty()` — call after each `feed()` to get
    /// exactly the rows to repaint this frame.
    pub fn take_dirty_rows(&mut self) -> Vec<usize> {
        self.screen.take_dirty_rows()
    }

    /// Resize the terminal screen.
    pub fn resize(&mut self, lines: usize, columns: usize) {
        self.screen.resize(lines, columns);
    }

    /// Reset the terminal to its initial state.
    pub fn reset(&mut self) {
        self.screen.reset();
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

    /// Shell working directory from OSC 7 / OSC 9;9, if reported yet.
    /// `None` until the shell emits its first cwd sequence.
    #[getter]
    pub fn cwd(&self) -> Option<String> {
        self.screen.cwd().map(str::to_string)
    }

    /// Whether a BEL (0x07) arrived since the last call, then resets it.
    /// Edge-triggered: two calls in a row return ``True`` then ``False``.
    /// OSC sequences terminated by BEL (e.g. window titles) do NOT ring.
    pub fn take_bell(&mut self) -> bool {
        self.screen.take_bell()
    }

    /// Drain the ordered low-frequency event log as ``(tag, payload)`` tuples.
    ///
    /// Tags: ``("bell", None)``, ``("title", str)``, ``("icon", str)``,
    /// ``("cwd", str)``, ``("altscreen", bool)``, ``("scrollback_grew", int)``.
    /// Drained, not coalesced: order matches the parser. Draining consumes
    /// the pending bell too, so ``take_bell()`` afterwards is ``False`` —
    /// and vice versa. One FFI crossing per frame instead of one per signal.
    pub fn poll_events(&mut self, py: Python<'_>) -> Vec<(String, Py<PyAny>)> {
        use crate::terminal::events::TermEvent;
        self.screen
            .take_events()
            .into_iter()
            .map(|e| {
                let tag = e.tag().to_string();
                let payload: Py<PyAny> = match e {
                    TermEvent::Bell => py.None().into_any(),
                    TermEvent::TitleChanged(t)
                    | TermEvent::IconChanged(t)
                    | TermEvent::CwdChanged(t) => t.into_pyobject(py).unwrap().into_any().unbind(),
                    // `bool` converts to a borrowed reference: own it first.
                    TermEvent::AltScreen { entered } => entered
                        .into_pyobject(py)
                        .unwrap()
                        .to_owned()
                        .into_any()
                        .unbind(),
                    TermEvent::ScrollbackGrew(n) => {
                        n.into_pyobject(py).unwrap().into_any().unbind()
                    }
                };
                (tag, payload)
            })
            .collect()
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

    /// Styled cells for absolute rows ``[start, start + count)``, clamped to the
    /// buffer. Serializes only the on-screen window instead of the whole
    /// scrollback — O(window) rather than O(total_lines) per frame.
    pub fn styled_range(
        &self,
        start: usize,
        count: usize,
    ) -> Vec<Vec<(String, String, String, u8)>> {
        self.screen.styled_range(start, count)
    }

    /// Total lines: scrollback history + visible screen.
    pub fn total_lines(&self) -> usize {
        self.screen.total_lines()
    }

    /// Visible height in rows — O(1). Use this instead of
    /// `len(visible_display())`, which builds the whole visible screen as
    /// Python strings just to measure it.
    #[getter]
    pub fn visible_lines(&self) -> usize {
        self.screen.lines()
    }

    /// Visible width in columns — O(1), sibling of [`Self::visible_lines`].
    #[getter]
    pub fn visible_columns(&self) -> usize {
        self.screen.columns()
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

    /// Current Kitty keyboard enhancement flags (active screen).
    #[getter]
    pub fn keyboard_mode(&self) -> u16 {
        self.screen.keyboard_mode()
    }

    /// Encode a key event using the active screen's Kitty flags.
    ///
    /// ``key`` is a named key (``"enter"``, ``"left"``, ``"f1"``,
    /// ``"kp_enter"``, …), a single printable character, or a numeric key code.
    /// ``modifiers`` is a bitmask (shift=1, alt=2, ctrl=4, super=8,
    /// hyper=16, meta=32, caps_lock=64, num_lock=128). ``flags`` defaults to
    /// the active screen's current ``keyboard_mode``; pass an integer to
    /// override it for tests or a custom frontend.
    ///
    /// Returns the bytes to write to the PTY. The result is empty when the
    /// active flags cannot represent the event (for example, a release event
    /// for a text key without ``REPORT_ALL_KEYS``).
    #[pyo3(signature = (key, modifiers=0, kind="press", text=None, shifted_key=None, base_layout_key=None, flags=None))]
    #[allow(clippy::too_many_arguments)]
    pub fn encode_key<'py>(
        &self,
        py: Python<'py>,
        key: &Bound<'_, PyAny>,
        modifiers: u16,
        kind: &str,
        text: Option<&str>,
        shifted_key: Option<&Bound<'_, PyAny>>,
        base_layout_key: Option<&Bound<'_, PyAny>>,
        flags: Option<u16>,
    ) -> PyResult<Bound<'py, PyBytes>> {
        let code = parse_key_code(key)?;
        let kind = match kind.to_ascii_lowercase().as_str() {
            "press" | "down" => KeyEventKind::Press,
            "repeat" => KeyEventKind::Repeat,
            "release" | "up" => KeyEventKind::Release,
            other => {
                return Err(PyValueError::new_err(format!(
                    "unknown event kind: {other}"
                )));
            }
        };
        let flags = flags.map_or_else(
            || KeyboardFlags::from_bits_truncate(self.screen.keyboard_mode()),
            KeyboardFlags::from_bits_truncate,
        );
        let event = KeyEvent {
            code,
            modifiers: Modifiers::from_bits_truncate(modifiers),
            kind,
            text: text.map(str::to_owned),
            shifted_key: code_point(shifted_key, "shifted_key")?,
            base_layout_key: code_point(base_layout_key, "base_layout_key")?,
        };
        Ok(PyBytes::new(py, &encode_key(&event, flags)))
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

fn parse_key_code(key: &Bound<'_, PyAny>) -> PyResult<KeyCode> {
    if let Ok(name) = key.extract::<String>() {
        return KeyCode::from_name(&name)
            .ok_or_else(|| PyValueError::new_err(format!("unknown key name: {name}")));
    }
    if let Ok(code) = key.extract::<u32>() {
        return Ok(KeyCode::from_u32(code));
    }
    Err(PyTypeError::new_err(
        "key must be a key name (str), a single character, or an integer code",
    ))
}

fn code_point(value: Option<&Bound<'_, PyAny>>, field: &str) -> PyResult<Option<char>> {
    let Some(value) = value else {
        return Ok(None);
    };
    if let Ok(text) = value.extract::<String>() {
        let mut chars = text.chars();
        let ch = chars
            .next()
            .ok_or_else(|| PyValueError::new_err(format!("{field} must be one character")))?;
        if chars.next().is_some() {
            return Err(PyValueError::new_err(format!(
                "{field} must be one character"
            )));
        }
        return Ok(Some(ch));
    }
    if let Ok(number) = value.extract::<u32>() {
        return char::from_u32(number).map(Some).ok_or_else(|| {
            PyValueError::new_err(format!("{field} is not a Unicode code point: {number}"))
        });
    }
    Err(PyTypeError::new_err(format!(
        "{field} must be a character or integer code point"
    )))
}
