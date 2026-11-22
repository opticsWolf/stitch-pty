# Test Suite

## Rust Tests

| Platform | Unit | Integration | Breakdown |
|----------|------|-------------|-----------|
| **Windows** | **435** | **23** | 415 shared + 20 Windows-only |
| **Linux / macOS** | **419** | **29** | 415 shared + 4 Unix-only |

### Shared Tests (415)

Run on all platforms via `src/platform.rs`, `src/async_io.rs`, `src/errors.rs`, `src/winsize.rs`, and `src/terminal/`.

- **async_io** (13): `read_timeout` logic, buffer handling, duration edge cases
- **errors** (18): error display formatting, `PtyErrorKind` variants, IO error conversion
- **expect** (2): literal/string/empty-byte matching; regex is opt-in under `--features regex`
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
`rust_session.rs` on Windows and 21 on Unix, one parity corpus test, two
`terminal_session.rs` tests on Windows / three on Unix, and two `expect.rs`
tests on Windows / four on Unix (the Windows-only interrupt/env tests and
Unix-only signal/EOF/from_parts/scrollback/expect tests are marked; the regex
integration test requires `--features regex`):

- spawn → read → wait with `ExitStatus`, plus repeated `wait()`
- environment inheritance (`spawn`), explicit empty env, exact replacement (`spawn_with_env`), and Windows case-insensitive duplicate-key collapse
- 24×80 defaults for `spawn(None)` and `open_pty_platform(None)`
- resize round-trip, interactive `write_all`, read timeout that keeps the session alive
- `terminate` grace/reap; a SIGTERM-ignoring child is escalated to SIGKILL (Unix)
- signal exit status reports the portable name (`SIGKILL`) after `kill()` (Unix)
- `read_until_exit` output/status and large-output draining; `close()` idempotence with unread output
- stdin EOF via `shutdown_write` (Unix); EOF after child exit (Unix; Windows ConPTY may keep the pipe open)
- Windows `interrupt()`/`send_signal(2)` return `Unsupported`
- `from_parts` / `reader()` / `writer()` round-trip on Unix (custom interop)
- `TerminalSession` auto-feed, dirty rows, resize sync, and Unix scrollback
- `expect()` literal/multi-pattern/timeout/non-UTF-8 matching (regex under the optional feature)

### Redirected stdio (`tests/redirected_stdio.rs`)

One Windows-only test in its own process: redirects the parent's stdout/stderr
to a file, spawns a ConPTY child, and verifies the output reaches the PTY, not
the file. This is the H2 `STARTF_USESTDHANDLES` regression.

---

## Python Tests

| Platform | Count | Breakdown |
|----------|-------|-----------|
| **Windows** | **217** | 215 passed + 2 skipped (POSIX-only SIGTERM-ignore and signal-number tests; explicit unsupported `interrupt()` path) |
| **Linux / macOS** | **217** | 216 passed + 1 skipped (Windows-only env-casing test) |

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
| `test_terminal_parity.py` | 4 | Fixed ANSI/OSC/UTF-8 corpus at 1/3/7/whole-buffer chunk sizes; same rows/styles/title as `tests/terminal_parity.rs` |

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

## Throughput investigation plan

### Baselines and correctness

1. Run baseline and candidate from the same machine and toolchain with the
   release profile and the same lockfile and feature set. Record the revision,
   OS, CPU, profile, viewport dimensions, scrollback cap, feed size, and actual
   byte count. Build into separate Cargo target directories and record which
   executable produced each sample. Warm up each lane, then alternate baseline
   and candidate for at least 30 samples. Report median and p95 elapsed time, MB/s (decimal), and
   the raw samples. Keep setup, teardown, and assertions outside the timer.
2. Before accepting a timing sample, check the expected screen. Compare
   visible and retained rows, glyph text and widths, cursor and pending wrap,
   colors and attributes, links, wrap flags, and parser events at the relevant
   checkpoints. Verify whole-stream and split-stream results against known
   expected output. Exercise split UTF-8, CSI, OSC and ST, scroll margins,
   alternate-screen restore, and history eviction under reflow.
3. Include a mixed ASCII/CJK/combining corpus with explicit expected cell
   positions. Stitch PTY 0.16.0 leaves the cursor stationary for consecutive
   width-two characters. Treat that as a failing Unicode correctness case
   until wide-character placement and wrapping are repaired.

### Workloads and measurements

| Lane | Workload | What to measure |
|---|---|---|
| Native parser | Same raw bytes through `terminal::ansi_parser::Parser` with counting callbacks | Time, printed/control counts, and split-record parity. Label synthetic Kitty APC as skipped string data. |
| Draw without scroll | Printable ASCII overwrites, alternating SGR runs, and mixed-width text | Allocations per MiB, CPU time, and dirty rows. Compare direct draw to parser plus draw to size an ASCII run fast path. |
| Scroll and retain | Numbered ASCII rows, soft wrapping, alternate screen, and partial scroll regions | Compare 80x24, 80x48, and 200x60. Sweep 1,000, 2,000, and 20,000 history rows plus the alternate screen. Measure how copies and allocations scale with width, height, and feed size. |
| Erase and repaint | Separate ED, EL, ECH, cursor-only movement, reverse index, and repeated styled redraws | Measure time and allocations per operation. Verify background-color erase and hyperlink removal, including on a resized screen. |
| Resize and project | Reflow retained wrapped output, then read the viewport or capture history | Report reflow, borrowed row access, styled projection, and serialization separately. Track peak live bytes for large histories. |
| PTY and Mux | One captured PTY byte trace replayed into the native screen and the daemon at the same geometry | Split transport, feed, viewport conversion, fan-out, and client apply. Run zero, one, and two clients and report completion time, throughput, and interactive latency. |

Use 1 KiB, 16 KiB, and 64 KiB feeds for the steady-state runs. Include a
byte-at-a-time correctness run and split boundaries inside escape sequences.
Collect allocator counts and allocated bytes in a separate instrumented run
and a CPU sampling profile for the slowest workloads. Profiling and allocation
instrumentation can perturb the throughput numbers.

### Progress

The native-parser lane is implemented in `benchmark/` with the four payload
families above, three feed sizes, alternating native/vte samples, and a
split-record parity gate. The gate caught a native UTF-8 split-boundary bug:
multi-byte feeds that ended inside a code point lost following characters.
`ansi_parser.rs` now completes one code point at a time, leaves invalid
continuation bytes for the main loop, and has a regression test covering
whole-stream, byte-at-a-time, and chunked feeds. The external `vte` candidate
still shows the same split-boundary mismatch; it is reported as a candidate
note rather than failing the native lane.

Allocator instrumentation is built into the benchmark's global allocator and
reported as allocations/bytes per MiB parsed. It shows **0 allocations** for
native and vte parser-only lanes. The earlier CSI slowness came from
`Params` being `Vec<Vec<u16>>`, which allocated per parameter; it is now a
fixed-capacity array with group offsets, and `Parser` passes `Params::flat()`
directly to `Screen::csi_dispatch`. CSI p50 rose from ~120 MB/s to ~280–340
MB/s while vte stayed ~340–420 MB/s.

An opt-in `bench-profile` feature times the native ground-state phases. On a
single instrumented ASCII pass it attributes ~75% of measured phase time to
`ground_dispatch`, ~23% to the ESC scan, and ~2% to `from_utf8`. These are
indicative percentages from an instrumented pass, not a CPU sampler, and the
remaining gap to vte is still unattributed.

The same `bench-profile` feature now times `HistoryScreen::feed`. On a
single numbered-ASCII pass it attributes ~86% of measured phase time to
`Screen::scroll_up`, almost all of that to blank-row erase, ~0.5% to
scrollback collection, and ~14% to drawing/other work. Direct screen lanes
for draw, scroll, erase, resize, and projection live in `benchmark/` and are
reported as `screen_results`.

Following that profile, `Screen::scroll_down` and both
`HistoryScreen::scroll_*_with_history` methods now rotate row owners and erase
only the moved rows instead of allocating and cloning blank rows. Capped
scrollback returns evicted rows to a bounded `blank_rows` pool, and
`HistoryScreen::feed` collects scrollback every 2 KiB so the pool is reused
during a large burst rather than only at its end. In the direct lanes, an
80x24 screen with cap 100 dropped from ~3.37M to ~0.50M allocations per MiB
and rose from ~5 MB/s to ~19–21 MB/s. Larger caps are unchanged because their
histories do not evict within a 64 KiB payload, which is the expected result.
`reflow` now takes ownership of source rows instead of cloning the whole
screen three times. `resize_reflow` now runs eight alternating 80->60->80 cycles per sample to
measure steady state. It rose from ~13.6 to ~86–110 ops/s. Allocations fell
from ~26M per MiB fed in the original three-clone reflow to ~24.5k per
resize. The gains came from moving row owners instead of cloning the screen,
then recycling trimmed and continuation blank cells into a `blank_cells` pool
used for padding; the pool limit had to be raised to `rows * columns` to keep
enough blanks during a full reflow. Remaining allocations are content cells
(`String` per glyph/color), so further reduction needs a cell representation
change rather than another reuse pool. Allocation fields are now reported as
`allocations` / `allocated_bytes` with an explicit unit per lane.

### Next ablations

- Compare a bounded pool of evicted primary rows against per-scroll allocation.
  Include small and large feeds to show how much reuse survives until the next
  feed. Report peak retained memory per pane and verify OSC dispatch positions
  and scrollback growth events.
- Compare safe printable ASCII run writes to direct character writes. Include
  control-heavy input, SGR transitions, right-edge wrapping, and insert mode.
- Consider a history ring only if large caps with small feeds show meaningful
  time in front eviction after batching. Profile Mux viewport copies once the
  screen-only lane improves.

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
