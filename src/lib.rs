//! stitch-pty: Cross-platform async PTY with integrated terminal emulation.
//!
//! Architecture:
//! - PTY layer: `platform` (PtyBackend/ChildBackend traits) → unix/windows
//! - Session layer: `session` (Rust `PtySession`, no PyO3 required)
//! - Terminal layer: `terminal` (pyte_rs: parser + screen + history)
//! - Python API: `python_api` + `terminal_api` (PyO3 bindings)
//!
//! GIL Strategy:
//! - Never hold GIL during I/O waits
//! - Acquire GIL only for: (1) converting buffers to PyBytes, (2) raising exceptions

// PTY layer
pub mod platform;
#[cfg(unix)]
mod platform_unix;
#[cfg(windows)]
mod platform_windows;

// Terminal emulation (embedded pyte_rs)
pub mod terminal;
pub use terminal::{
    Char, Cursor, HistoryScreen, KeyCode, KeyEvent, KeyEventKind, KeyboardFlags, Modifiers, Parser,
    Screen, encode_key,
};

// High-level Rust session API
pub mod session;
pub use session::PtySession;
pub use winsize::Winsize;

// Cross-platform modules
pub mod async_io;
pub mod errors;
#[cfg(feature = "python")]
mod python_api;
#[cfg(feature = "python")]
mod terminal_api;
mod terminate;
pub mod winsize;

#[cfg(feature = "python")]
use pyo3::PyTypeInfo;
#[cfg(feature = "python")]
use pyo3::prelude::*;

/// Initialize the Python module
#[cfg(feature = "python")]
#[pymodule]
fn _core(m: &Bound<'_, PyModule>) -> PyResult<()> {
    // Register Python-facing PTY types
    m.add_class::<python_api::PtyMaster>()?;
    m.add_class::<python_api::PtyChild>()?;
    m.add_class::<python_api::PtySession>()?;
    m.add_class::<winsize::Winsize>()?;

    // Register the terminal emulation class
    m.add_class::<terminal_api::TerminalState>()?;

    // Register exception types
    m.add("PtyError", errors::PtyError::type_object(m.py()))?;
    m.add("ProcessError", errors::ProcessError::type_object(m.py()))?;
    m.add("IOError", errors::IOError::type_object(m.py()))?;

    // Utility functions
    m.add_wrapped(wrap_pyfunction!(python_api::open_pty))?;
    m.add_wrapped(wrap_pyfunction!(python_api::spawn))?;

    // Platform info
    m.add("PLATFORM", if cfg!(unix) { "unix" } else { "windows" })?;

    Ok(())
}
