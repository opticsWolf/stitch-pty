# Test Suite

## Rust Tests

| Platform | Unit | Integration | Breakdown |
|----------|------|-------------|-----------|
| **Windows** | **433** | **18** | 413 shared + 20 Windows-only |
| **Linux / macOS** | **417** | **20** | 413 shared + 4 Unix-only |

### Shared Tests (413)

Run on all platforms via `src/platform.rs`, `src/async_io.rs`, `src/errors.rs`, `src/winsize.rs`, and `src/terminal/`.

- **async_io** (13): `read_timeout` logic, buffer handling, duration edge cases
- **errors** (18): error display formatting, `PtyErrorKind` variants, IO error conversion
- **platform** (23): `ExitStatus` display/clone/debug, Python exit-code/signal mapping, trait existence, explicit env dedupe (exact/Windows casing, empty/Unicode values)
- **terminal/ansi_parser** (30): CSI/DCS/OSC/ESC parsing, UTF-8 handling, parameter overflow
- **terminal/charsets** (7): ASCII map, DEC special characters, charset designation
- **terminal/control** (12): C0/C1 control characters, escape sequences, `is_control` helpers
- **terminal/cwd** (8): OSC 7 / OSC 9;9 working-directory parsing
- **terminal/escape** (8): CSI param/intermediate/final character classification
- **terminal/events** (1): event tag mapping/ordering
- **terminal/graphics** (24): Color (RGB/256/named), SGR attributes, text formatting
- **terminal/keyboard** (11): Kitty key encoding — flag 1 exceptions, flag 8 all keys, event types, alternates, associated text, legacy fallback, functional/keypad codes
- **terminal/history** (55): Scrollback, alt screen swap, resize, cursor delegation, split-feed parser state (P1), keyboard-mode accessor and screen-flag encoder
- **terminal/modes** (7): Public/private modes, mouse modes, cursor style
- **terminal/parser** (83): Full parser — CSI/OSC/ESC/SGR handling, cursor positioning, screen manipulation, Kitty keyboard spec vectors
- **terminate** (5): shared escalation policy — already-exited, graceful, SIGTERM-ignoring, repeated, injected sleep
- **terminal/screen** (97): Screen rendering, cursor movement, line operations, resize, margins, Kitty per-screen keyboard stacks
- **winsize** (11): Clone/copy/debug, equality, default, PyO3 conversion

### Windows-Only Tests (20)

In `src/platform_windows.rs`.

- **append_quoted_wide** (10): Command-line quoting for Windows `CreateProcess` — spaces, tabs, quotes, backslashes, newlines, edge cases
- **win_child_process** (2): PID access, clone behavior
- **win_pty_backend** (1): Backend struct fields
- **send_sync_handle** (3): `Send`/`Sync`/`Clone` trait verification for handle wrapper
- **pipe_counter** (1): Pipe counter monotonic increment
- **environment block** (2): empty double-null block; UTF-16 encoding with case-insensitive dedupe and empty/Unicode values
- **ConPTY flags** (1): `STARTF_USESTDHANDLES` is set; resize quirk and Win32 input mode stay off

### Unix-Only Tests (4)

In `src/platform_unix.rs` and `src/errors.rs`.

- **platform_unix** (3): PTY open/close + FD cleanup, type signature checks, `ChildKiller` `Send`/`Sync` verification
- **errors** (1): `from_read_error` maps `EIO` to `Eof`

> **Note:** 6 `UnixChildProcess` tests (`new`, `pid`, `is_running`, `clone`, `child_killer`, `drop`) were removed because they create a `tokio::spawn` background task that blocks the tokio test runtime from shutting down. The build itself validates that `UnixChildProcess` compiles with the correct API.

### Integration Tests (`tests/rust_session.rs`)

Real PTYs, end-to-end, via `cargo test` on all three platforms — 18 tests in
`rust_session.rs` on Windows and 20 on Unix (the Windows-only interrupt/env
tests and Unix-only signal/EOF tests are marked):

- spawn → read → wait with `ExitStatus`, plus repeated `wait()`
- environment inheritance (`spawn`), explicit empty env, exact replacement (`spawn_with_env`), and Windows case-insensitive duplicate-key collapse
- 24×80 defaults for `spawn(None)` and `open_pty_platform(None)`
- resize round-trip, interactive `write_all`, read timeout that keeps the session alive
- `terminate` grace/reap; a SIGTERM-ignoring child is escalated to SIGKILL (Unix)
- signal exit status reports the portable name (`SIGKILL`) after `kill()` (Unix)
- `read_until_exit` output/status and large-output draining; `close()` idempotence with unread output
- stdin EOF via `shutdown_write` (Unix); EOF after child exit (Unix; Windows ConPTY may keep the pipe open)
- Windows `interrupt()`/`send_signal(2)` return `Unsupported`

### Redirected stdio (`tests/redirected_stdio.rs`)

One Windows-only test in its own process: redirects the parent's stdout/stderr
to a file, spawns a ConPTY child, and verifies the output reaches the PTY, not
the file. This is the H2 `STARTF_USESTDHANDLES` regression.

---

## Python Tests

| Platform | Count | Breakdown |
|----------|-------|-----------|
| **Windows** | **213** | 211 passed + 2 skipped (POSIX-only SIGTERM-ignore and signal-number tests; explicit unsupported `interrupt()` path) |
| **Linux / macOS** | **213** | 212 passed + 1 skipped (Windows-only env-casing test) |

### Test Files

| File | Tests | Description |
|------|-------|-------------|
| `test_basic.py` | 15 | Core PTY operations: spawn, read, write, kill, resize, signals, interrupt |
| `test_concurrency.py` | 3 | Concurrent reads conserve bytes, cancel-during-terminate, coherent drains |
| `test_pty_session.py` | 43 | Session lifecycle: env (inherit/empty/exact, Windows casing dedupe), winsize, concurrent spawn, wait/signal status, terminate, split-read parser, escalation/repeat edge cases |
| `test_terminal.py` | 19 | Terminal display: empty, scrollback, cursor, title, history, dirty rows |
| `test_terminal_altscreen.py` | 10 | Alt screen: enter/exit 1049, 47/1047 swap, scrollback isolation, resize |
| `test_terminal_display.py` | 56 | Full display rendering: ANSI sequences, SGR colors, cursor movement, erase, scroll, tabs |
| `test_terminal_modes.py` | 24 | Mode toggles: DECCKM, DECTCEM, bracketed paste, mouse protocol, cursor style, Kitty keyboard flags/stacks |
| `test_keyboard.py` | 8 | Kitty encoder: Shift+Enter, Ctrl+I vs Tab, legacy fallback, flag 8, event types, alternates, associated text, `PtySession.encode_key` |
| `test_terminal_scrollback.py` | 15 | Scrollback: capacity limits, trim, absolute cursor, history-aware resize |
| `test_terminal_styled.py` | 20 | Styled cells: color (named/256/truecolor), attributes (bold/dim/italic/underline/blink/reverse/hidden/strikethrough) |

### Platform-specific behavior

`test_interrupt` asserts the explicit `NotImplementedError` path for
`interrupt()` and `send_signal(2)` on Windows, and sends SIGINT on POSIX.
Windows skips the two POSIX signal tests; Linux/macOS skip the Windows
environment-casing test.

---

## Property Tests (proptest, v0.7.2+)

Example-based tests cover known sequences; `proptest` covers the hostile
ones. Strategies: arbitrary byte vecs, structured mixes (valid CSI/OSC/ESC
fragments spliced with UTF-8 multibyte and lone high bytes), and random
feed-split boundaries (control sequences straddling feeds).

Invariants (asserted after every feed):

1. Buffer is exactly `lines` rows × `columns` cols.
2. Cursor `x <= columns` (`==` is the legal pending-wrap state), `y < lines`.
3. Scrollback `len <= cap` (cap `0` = unbounded).
4. `take_bell()` never fires twice in a row.
5. `display()` is total and row-shaped (UTF-8 validity is structural).
6. `?1049h … ?1049l` always returns to the primary buffer.

Locations: `src/terminal/parser.rs` (Screen-level), `src/terminal/history.rs`
(HistoryScreen-level). CI pins `PROPTEST_CASES=256`; run more locally with
`PROPTEST_CASES=10000 cargo test --lib prop_`.

Failure workflow (mandatory): a shrunk counterexample is saved under
`proptest-regressions/` (committed — seeds replay every run), promoted into
a named `#[test]` regression next to the related unit tests, and — if it
names a real bug rather than a wrong invariant — fixed in the same version.
First catch: `test_erase_in_line_mode1_at_pending_wrap` (`ESC[1K` with the
cursor in pending-wrap indexed out of bounds; fixed v0.7.2).

---

## Known Issues & Mitigations

### macOS Tokio Timer Starvation (v0.5.6+)

On macOS (especially Python 3.13), `tokio::time::sleep` can fail to fire when bridged through PyO3's `future_into_py` into Python's asyncio event loop. This caused `wait()` and `terminate()` to hang indefinitely.

**Fix applied in v0.5.6:**
- Rust: `wait()` uses `spawn_blocking(|| std::thread::sleep(20ms))` instead of `tokio::time::sleep`
- Rust: `terminate()` uses `std::time::Instant` deadline polling instead of `tokio::time::timeout`
- Python: `wait()` wraps the Rust call with `asyncio.wait_for(..., timeout=0.1)` polling

**0.10.3 update:** `terminate()` is now one shared `src/terminate.rs` policy:
Rust injects `tokio::time::sleep`; the PyO3 adapter injects
`spawn_blocking(std::thread::sleep)`. The Python wrappers await that single
future and no longer re-issue SIGTERM every 100 ms. `wait()` is unchanged.

**If this reappears:** Consider replacing the polling model with a `Condvar`-based notification from the reaper thread (Option D).
