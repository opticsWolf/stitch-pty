# Rust / Python API Parity

The Rust core is independently usable. The `python` feature adds PyO3
bindings; it is not required to build, test, or use the PTY and terminal
emulation layers. CI's **Rust core (no default features)** job runs
`cargo check/test --no-default-features`, checks all targets, and asserts
`pyo3` is absent from the default dependency tree.

## Session surface

| Capability | Rust `PtySession` | Python `PtySession` | Classification |
|---|---|---|---|
| Spawn with inherited env | `spawn` | `spawn` | Parity |
| Spawn with explicit env | `spawn_with_env` | `spawn(..., env=...)` | Parity; Python accepts a mapping |
| Read / write / write_all | methods | async methods | Parity |
| Timeout read | `read_timeout` | `read_timeout` | Parity |
| Resize / winsize | methods | methods | Parity |
| Wait / exit status | `wait` → `Option<ExitStatus>` | `wait` → `ExitStatus \| None` | Parity (Python adds the legacy `pid` tuple field) |
| Terminate / kill / signal | methods | methods | Parity |
| Close / shutdown_write | methods | not exposed | Rust-only (safety-sensitive lifecycle) |
| Reader / writer handles | `reader()`, `writer()` (Arc session) | not exposed | Rust-only interop |
| Owned PTY + screen | `TerminalSession` | `PtySession` owns `_terminal` | Parity; both auto-feed reads and resize the screen |
| Assemble custom backends | `from_parts(pty, child)` | not exposed | Rust-only interop |
| Raw unparsed output tail | not owned | `raw_output` | Python-only convenience; the transport does not retain an output history |
| Expect-style matching | `expect` / `expect_with_options` | `expect` | Parity; Rust uses byte-first `ExpectPattern` and optional `regex` feature |
| Owned terminal state | `TerminalSession` | `terminal` | Parity via G6; `PtySession` itself stays emulator-free in both layers |

## Terminal surface

| Capability | Rust `HistoryScreen` | Python `TerminalState` | Classification |
|---|---|---|---|
| Feed / resize / reset | methods | methods | Parity; `TerminalState` delegates to `HistoryScreen` |
| Display / visible / history | methods | methods | Parity |
| Styled cells | `styled_viewport`, `styled_range` | same | Parity |
| Dirty rows / bell / events | methods | methods | Parity |
| Kitty flags | `keyboard_mode`, `encode_key` | properties/`encode_key` | Parity |
| Python class wrapper | — | `TerminalState` | Thin PyO3 adapter; no independent parser state |

## Parity evidence

- `tests/terminal_parity.rs` and `tests/test_terminal_parity.py` feed the same
  fixed ANSI/OSC/UTF-8 corpus in 1-, 3-, 7-byte, and whole-buffer chunks and
  assert identical visible rows, styles, and title. The Rust test uses
  `HistoryScreen`; the Python test uses `TerminalState`.
- The split-feed tests from P1 cover CSI/OSC/DCS/UTF-8/alt-screen parser state
  in both surfaces.
- `raw_output` is intentionally Python-only. It is a bounded convenience over
  the session's read history, not a property of the transport; Rust callers
  that need it can accumulate reads themselves.
- `PtySession` remains emulator-free in Rust. Python's owned `_terminal` is a
  binding-level convenience; `TerminalSession` is the Rust equivalent.
