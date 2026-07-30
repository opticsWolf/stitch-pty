# Test Suite

## Rust Tests

| Platform | Count | Breakdown |
|----------|-------|-----------|
| **Windows** | **329** | 314 shared + 17 Windows-only |
| **Linux / macOS** | **323** | 314 shared + 9 Unix-only |

### Shared Tests (314)

Run on all platforms via `src/platform.rs`, `src/async_io.rs`, `src/errors.rs`, `src/winsize.rs`, and `src/terminal/`.

- **async_io** (12): `read_timeout` logic, buffer handling, duration edge cases
- **errors** (17): error display formatting, `PtyErrorKind` variants, IO error conversion
- **platform** (18): `ExitStatus` display/clone/debug, `ProcessExit` conversion, trait existence
- **terminal/ansi_parser** (28): CSI/DCS/OSC/ESC parsing, UTF-8 handling, parameter overflow
- **terminal/charsets** (9): ASCII map, DEC special characters, charset designation
- **terminal/control** (16): C0/C1 control characters, escape sequences, `is_control` helpers
- **terminal/escape** (8): CSI param/intermediate/final character classification
- **terminal/graphics** (24): Color (RGB/256/named), SGR attributes, text formatting
- **terminal/history** (25): Scrollback, alt screen swap, resize, cursor delegation
- **terminal/modes** (10): Public/private modes, mouse modes, cursor style
- **terminal/parser** (52): Full parser — CSI/OSC/ESC/SGR handling, cursor positioning, screen manipulation
- **terminal/screen** (56): Screen rendering, cursor movement, line operations, resize, margins
- **winsize** (13): Clone/copy/debug, equality, default, PyO3 conversion

### Windows-Only Tests (17)

In `src/platform_windows.rs`.

- **append_quoted_wide** (10): Command-line quoting for Windows `CreateProcess` — spaces, tabs, quotes, backslashes, newlines, edge cases
- **win_child_process** (2): PID access, clone behavior
- **win_pty_backend** (1): Backend struct fields
- **send_sync_handle** (3): `Send`/`Sync`/`Clone` trait verification for handle wrapper
- **pipe_counter** (1): Pipe counter monotonic increment

### Unix-Only Tests (9)

In `src/platform_unix.rs`.

- **close_random_fds** (2): Safe FD cleanup without IO Safety violations, idempotency
- **pty_pair** (1): PTY open/close, FD cleanup on drop
- **unix_pty_master** (1): Type signature verification
- **unix_child_process** (1): `ChildKiller` trait `Send`/`Sync` verification

> **Note:** 6 `UnixChildProcess` tests (`new`, `pid`, `is_running`, `clone`, `child_killer`, `drop`) were removed because they create a `tokio::spawn` background task that blocks the tokio test runtime from shutting down. The build itself validates that `UnixChildProcess` compiles with the correct API.

---

## Python Tests

| Platform | Count | Breakdown |
|----------|-------|-----------|
| **Windows** | **160** | 159 passed + 1 skipped (`test_interrupt` on CI) |
| **Linux / macOS** | **160** | 160 passed |

### Test Files

| File | Tests | Description |
|------|-------|-------------|
| `test_basic.py` | 16 | Core PTY operations: spawn, read, write, kill, resize, signals, interrupt |
| `test_pty_session.py` | 16 | Session lifecycle: env, winsize, concurrent spawn, wait, terminate |
| `test_terminal.py` | 8 | Terminal display: empty, scrollback, cursor, title, history, dirty rows |
| `test_terminal_altscreen.py` | 9 | Alt screen: enter/exit 1049, 47/1047 swap, scrollback isolation, resize |
| `test_terminal_display.py` | 56 | Full display rendering: ANSI sequences, SGR colors, cursor movement, erase, scroll, tabs |
| `test_terminal_modes.py` | 14 | Mode toggles: DECCKM, DECTCEM, bracketed paste, mouse protocol, cursor style |
| `test_terminal_scrollback.py` | 15 | Scrollback: capacity limits, trim, absolute cursor, history-aware resize |
| `test_terminal_styled.py` | 13 | Styled cells: color (named/256/truecolor), attributes (bold/dim/italic/underline/blink/reverse/hidden/strikethrough) |

### Skipped Tests

| Test | Condition | Reason |
|------|-----------|--------|
| `test_interrupt` | Windows + `CI=true` | Ctrl+C unreliable on Windows CI runners; works locally |
