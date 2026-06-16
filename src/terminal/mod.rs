//! stitch-pty terminal emulation layer (embedded pyte_rs).
//!
//! Provides a VT100/VT220/xterm-compatible terminal screen emulator
//! with scrollback history, ANSI escape sequence parsing, and full SGR support.
//!
//! Modules:
//! - `parser` — ANSI stream parser (Performer + Parser + Stream)
//! - `screen` — Screen buffer with Char, Cursor, SGR attributes
//! - `history` — HistoryScreen with scrollback
//! - `ansi_parser` — ECMA-48 ANSI escape sequence state machine
//! - `charsets` — G0/G1 character set mappings
//! - `control` — C0/C1 control character constants
//! - `escape` — Escape sequence designators
//! - `graphics` — SGR attributes, color palettes
//! - `modes` — Public/DEC terminal mode flags

pub mod ansi_parser;
pub mod charsets;
pub mod control;
pub mod escape;
pub mod graphics;
pub mod history;
pub mod modes;
pub mod parser;
pub mod screen;

// Re-export core types for convenience
pub use screen::{Char, Cursor, Margins, Screen};
pub use history::HistoryScreen;
pub use parser::{Parser, Stream};
