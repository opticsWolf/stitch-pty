# Test Suite

## Rust Tests

| Platform | Unit | Integration | Breakdown |
|----------|------|-------------|-----------|
| **Windows** | **401** | **11** | 384 shared + 17 Windows-only |
| **Linux / macOS** | **388** | **13** | 384 shared + 4 Unix-only |

### Shared Tests (384)

Run on all platforms via `src/platform.rs`, `src/async_io.rs`, `src/errors.rs`, `src/winsize.rs`, and `src/terminal/`.

- **async_io** (13): `read_timeout` logic, buffer handling, duration edge cases
- **errors** (17): error display formatting, `PtyErrorKind` variants, IO error conversion
- **platform** (23): `ExitStatus` display/clone/debug, `ProcessExit` conversion, trait existence
- **terminal/ansi_parser** (30): CSI/DCS/OSC/ESC parsing, UTF-8 handling, parameter overflow
- **terminal/charsets** (7): ASCII map, DEC special characters, charset designation
- **terminal/control** (12): C0/C1 control characters, escape sequences, `is_control` helpers
- **terminal/cwd** (8): OSC 7 / OSC 9;9 working-directory parsing
- **terminal/escape** (8): CSI param/intermediate/final character classification
- **terminal/events** (1): event tag mapping/ordering
- **terminal/graphics** (24): Color (RGB/256/named), SGR attributes, text formatting
- **terminal/history** (47): Scrollback, alt screen swap, resize, cursor delegation
- **terminal/modes** (7): Public/private modes, mouse modes, cursor style
- **terminal/parser** (82): Full parser — CSI/OSC/ESC/SGR handling, cursor positioning, screen manipulation
- **terminal/screen** (94): Screen rendering, cursor movement, line operations, resize, margins
- **winsize** (11): Clone/copy/debug, equality, default, PyO3 conversion

### Windows-Only Tests (17)

In `src/platform_windows.rs`.

- **append_quoted_wide** (10): Command-line quoting for Windows `CreateProcess` — spaces, tabs, quotes, backslashes, newlines, edge cases
- **win_child_process** (2): PID access, clone behavior
- **win_pty_backend** (1): Backend struct fields
- **send_sync_handle** (3): `Send`/`Sync`/`Clone` trait verification for handle wrapper
- **pipe_counter** (1): Pipe counter monotonic increment

### Unix-Only Tests (4)

In `src/platform_unix.rs` and `src/errors.rs`.

- **platform_unix** (3): PTY open/close + FD cleanup, type signature checks, `ChildKiller` `Send`/`Sync` verification
- **errors** (1): `from_read_error` maps `EIO` to `Eof`

> **Note:** 6 `UnixChildProcess` tests (`new`, `pid`, `is_running`, `clone`, `child_killer`, `drop`) were removed because they create a `tokio::spawn` background task that blocks the tokio test runtime from shutting down. The build itself validates that `UnixChildProcess` compiles with the correct API.

### Integration Tests (`tests/rust_session.rs`)

Real PTYs, end-to-end, via `cargo test` on all three platforms — 13 tests
total (11 on Windows; the Unix-only tests are marked):

- spawn → read → wait with `ExitStatus`, plus repeated `wait()`
- environment inheritance (`spawn`), explicit empty env, exact replacement (`spawn_with_env`)
- 24×80 defaults for `spawn(None)` and `open_pty_platform(None)`
- resize round-trip, interactive `write_all`, read timeout that keeps the session alive
- `terminate` grace/reap; a SIGTERM-ignoring child is escalated to SIGKILL (Unix)
- EOF after child exit (Unix; Windows ConPTY may keep the pipe open)

---

## Python Tests

| Platform | Count | Breakdown |
|----------|-------|-----------|
| **Windows** | **195** | 194 passed + 1 skipped (`test_interrupt` on CI) |
| **Linux / macOS** | **195** | 195 passed |

### Test Files

| File | Tests | Description |
|------|-------|-------------|
| `test_basic.py` | 15 | Core PTY operations: spawn, read, write, kill, resize, signals, interrupt |
| `test_concurrency.py` | 3 | Concurrent reads conserve bytes, cancel-during-terminate, coherent drains |
| `test_pty_session.py` | 37 | Session lifecycle: env (inherit/empty/exact), winsize, concurrent spawn, wait, terminate |
| `test_terminal.py` | 19 | Terminal display: empty, scrollback, cursor, title, history, dirty rows |
| `test_terminal_altscreen.py` | 10 | Alt screen: enter/exit 1049, 47/1047 swap, scrollback isolation, resize |
| `test_terminal_display.py` | 56 | Full display rendering: ANSI sequences, SGR colors, cursor movement, erase, scroll, tabs |
| `test_terminal_modes.py` | 20 | Mode toggles: DECCKM, DECTCEM, bracketed paste, mouse protocol, cursor style |
| `test_terminal_scrollback.py` | 15 | Scrollback: capacity limits, trim, absolute cursor, history-aware resize |
| `test_terminal_styled.py` | 20 | Styled cells: color (named/256/truecolor), attributes (bold/dim/italic/underline/blink/reverse/hidden/strikethrough) |

### Skipped Tests

| Test | Condition | Reason |
|------|-----------|--------|
| `test_interrupt` | Windows + `CI=true` | Windows Ctrl+C delivery to ConPTY children is unreliable; the test only checks `interrupt()` does not panic |

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
- Python: `wait()` and `terminate()` wrap Rust calls with `asyncio.wait_for(..., timeout=0.1)` polling loops

**If this reappears:** Consider replacing the polling model with a `Condvar`-based notification from the reaper thread (Option D).
