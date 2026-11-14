# Changelog

All notable changes to stitch-pty are documented here, newest first.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Every version below is exactly one commit on `dev` (see `docs/RELEASING.md`).

## [Unreleased](https://github.com/opticsWolf/stitch-pty/compare/v0.10.5...HEAD)

## [0.10.5 — ConPTY stdio isolation and flag decision](https://github.com/opticsWolf/stitch-pty/compare/v0.10.4...v0.10.5)

### Fixed
- Windows `CreateProcessW` now sets `STARTF_USESTDHANDLES` with invalid
  standard handles, so a ConPTY child cannot inherit the parent's redirected
  stdout/stderr. The ConPTY process attribute still supplies the actual child
  console; a dedicated redirected-parent integration test verifies that child
  output reaches the PTY, not the parent's log.
- Documented the ConPTY creation-flag decision: keep
  `PSEUDOCONSOLE_INHERIT_CURSOR` only. `RESIZE_QUIRK` has no reproducible bug
  in the current matrix, and `WIN32_INPUT_MODE` would break raw byte input.

## [0.10.4 — explicit environment deduplication](https://github.com/opticsWolf/stitch-pty/compare/v0.10.3...v0.10.4)

### Fixed
- Explicit `env` entries now collapse duplicate keys to the last occurrence,
  including its key spelling and value. Windows compares keys ASCII
  case-insensitively, so `PATH` plus `Path` can no longer produce an ambiguous
  environment block; Unix collapses exact-key duplicates with the same
  last-wins rule. Empty env remains empty, and the `None` inheritance path is
  unchanged (`vars_os()` on Unix, a null block on Windows).
- Python `spawn()` now forwards environment entries as ordered pairs instead of
  an unordered `HashMap`, so the Windows last-wins result is deterministic.
- Documented that explicit `String` entries cannot represent non-Unicode
  keys/values; inheritance does not have that limitation.

## [0.10.3 — shared terminate policy](https://github.com/opticsWolf/stitch-pty/compare/v0.10.2...v0.10.3)

### Changed
- Python `PtyChild.terminate()` and `PtySession.terminate()` now delegate to
  one Rust escalation policy instead of wrapping each call in a 100 ms
  `asyncio.wait_for` loop that could re-issue SIGTERM. `PtySession::terminate`
  and the PyO3 adapter share the SIGTERM → grace → SIGKILL → bounded-reap
  ladder; the adapter injects blocking sleeps so the Python path does not
  depend on tokio timers firing through the PyO3 bridge on macOS.
- `terminate()` remains idempotent and best-effort: an already-exited child or
  a failed signal is ignored. Added Rust policy tests for already-exited,
  graceful, SIGTERM-ignoring, and repeated calls, plus Python tests for the
  same edge cases.

## [0.10.2 — Windows close/read-until-exit and exit watcher](https://github.com/opticsWolf/stitch-pty/compare/v0.10.1...v0.10.2)

### Added
- `PtySession::read_until_exit(drain_timeout)` accumulates output until the
  child exits and drains remaining ConPTY output, returning
  `(Vec<u8>, Option<ExitStatus>)`.
- `PtySession::close()` terminates a live child, drains pending output, and
  closes the backend idempotently. On Windows `ClosePseudoConsole` runs on a
  blocking thread; repeated calls are safe.
- `PtySession::shutdown_write()` sends stdin EOF without closing the output
  side: newline + `VEOF` on Unix, input-pipe disconnect on Windows.
- `PtyBackend::close()` and `PtyBackend::shutdown_write()` hooks.

### Fixed
- Windows child exit is now event-driven (`WaitForSingleObject` on a watcher
  thread + `watch` channel) instead of a 50 ms `GetExitCodeProcess` poll.
  `wait()` wakes promptly and the polling path remains only as a fallback.
- `read_until_exit` no longer reports repeated timeouts after child exit on
  Windows; it drains the pipe and returns the status.

## [0.10.1 — Windows interrupt is explicitly unsupported](https://github.com/opticsWolf/stitch-pty/compare/v0.10.0...v0.10.1)

### Changed
- Windows `interrupt()` and `send_signal(2)` now return
  `PtyErrorKind::Unsupported` (Python `NotImplementedError`) instead of
  calling `GenerateConsoleCtrlEvent(CTRL_C_EVENT, 0)`, which targets the
  caller's console group and generally does not reach the ConPTY child.
  POSIX SIGINT is unchanged; use `terminate()` or `kill()` on Windows.
  Added a Windows integration test and a Python regression test.

## [0.10.0 — Rust PtySession + explicit environment contract](https://github.com/opticsWolf/stitch-pty/compare/v0.9.1...v0.10.0)

### Added
- `PtySession` Rust API (`stitch_pty::PtySession`): `spawn`,
  `spawn_with_env`, `read`/`read_timeout` (EOF reported as `Ok(0)` when
  the platform closes the PTY; see the Windows caveat),
  `write`/`write_all`, `resize`, `wait` returning `ExitStatus`,
  `terminate`, `interrupt`, `kill`, and `into_parts`. Mirrors the Python
  session composition without requiring PyO3, so Rust callers no longer
  have to assemble `spawn_platform`/`open_pty_platform` by hand.
- `PtyBackend::write_all` and `ChildBackend::wait_status` default methods.
- Public `ArcPtyBackend` / `ArcChildBackend` aliases for the handles
  returned by `spawn_platform` and `open_pty_platform`; previously those
  return types could not be named by callers.
- Root re-export of `Winsize` (`stitch_pty::Winsize`).

### Changed
- `platform::spawn_platform` now takes
  `env: Option<&[(String, String)]>`: `None` inherits the parent
  environment, `Some(&[])` requests an empty environment, and
  `Some(entries)` supplies exactly those variables. This resolves the old
  ambiguity where `&[]` was empty on Unix but inherited on Windows.
- Python `spawn(env=None)` now uses the inherit path, so it no longer
  snapshots/copies the parent environment through `std::env::vars()`;
  `spawn(env={})` now spawns with an empty environment on both platforms
  (it used to inherit on Windows and be empty on Unix).
- README gained a Rust quick start; `cargo test` now runs cross-platform
  `PtySession` integration tests.
- `PtySession` docs corrected: Windows EOF (the ConPTY pipe can stay
  open after exit), repeated `wait()` returns the same status,
  kill-on-drop, `Arc` handle cloning, `terminate`'s best-effort
  semantics, and Windows `interrupt()` being best-effort.
- Docs consistency sweep: README `TerminalState` table gained the
  `take_bell`/`take_dirty_rows`/`poll_events`/`cwd` rows it was missing;
  QUICKREF `TerminalState` properties gained `visible_lines`/
  `visible_columns` and its error table no longer attributes timeouts to
  `IOError`; ARCHITECTURE wrapper paragraph mentions the geometry getters.
- README Platform Support table corrected: no `CreateProcess`+pipes
  fallback exists on pre-1809 Windows (spawn raises `PtyError`), Windows
  signals are best-effort Ctrl+C plus TERM/KILL-via-`TerminateProcess`, and
  `core_dumped` is always `False` (field reserved); matching touch-ups to
  the ConPTY-loading and Platform-Differences rows.
- README full audit (fixes 3–10): 14-state parser table (was 10),
  correct clone URL and dependency version, `SIGQUIT` in the
  signal-reset list, `set_scrollback_lines` moved to methods,
  reflow/wrap rows in the Screen Buffer table, and an honest GIL section.
- ARCHITECTURE + QUICKREF audit: 14-state parser tables, honest GIL
  wording, `SIGQUIT`, `styled_range` + `history_wrapped` in the
  HistoryScreen section, `session.resize` forwarding note, Windows
  no-fallback limitation corrected, `core_dumped` footnotes; README
  `wait(timeout=None)` rows and the POSIX reaper (`std::thread`, not
  `tokio::spawn`) fixed after cross-checking QUICKREF.
- Examples audited headless (both data paths smoke-tested live): advanced
  `save_output` now writes the full buffer (`text_rows`) instead of only
  the rendered window; both resizers model the v0.9.1 single-call
  `session.resize()` (backend + emulator) instead of the redundant
  two-call pattern.

### Fixed
- `spawn_platform`/`open_pty_platform` now apply the documented 24×80
  default when `winsize` is `None`. The Unix backend previously passed
  `None` to `openpty(3)`, which left the kernel's 0×0 size.
- `write_all` returns `WriteZero` instead of looping forever when a
  backend reports a zero-byte write.
- Bumped `async-trait` to 0.1.92 in `Cargo.lock`; 0.1.91 trips
  `clippy::double_must_use` on current stable, so `cargo clippy -- -D
  warnings` could not pass.
- `Screen::erase_characters` now uses an iterator instead of an index loop,
  which Rust 1.100's `clippy::needless_range_loop` rejects under CI's
  `-D warnings` (pre-existing failure, unrelated to the session API).

## [0.9.1] — Resize forwarding + Windows kill-on-drop

### Fixed
- `PtySession.resize()` now resizes **both** the PTY backend and the
  terminal emulator. Previously it only resized the backend, so the
  emulator kept the old geometry (and `display`/`visible_lines` reported
  stale numbers) until a separate `terminal.resize()` call — two calls
  where one was documented. Idempotent for consumers already calling
  both; reflow rides along on column changes.
- `WinChildProcess` Drop now terminates a still-running child before
  closing handles, matching `UnixChildProcess` (SIGKILL + reap). Windows
  sessions dropped without `terminate()` previously orphaned a live
  child that could never be reaped (handles already gone).

## [0.9.0] — Reflow on column resize

### Added
- `HistoryScreen` reflows on column resize: `history ++ buffer` is treated
  as one logical sequence (joined via the v0.8.2 wrap flags), flattened
  and re-split at the new width. Wide glyphs never split (gap pad +
  next row); trailing default-blank padding is trimmed so narrow→wide
  round-trips are lossless; BCE-colored tails survive. Repack is
  bottom-anchored (last `lines` rows stay visible, the rest flows to
  history cap-trimmed); the cursor keeps a logical anchor (line + flat
  offset), clamped into view if its content was trimmed. Primary screen
  only — alt-screen resizes conform without reflowing; DECSTBM × reflow
  is unspecified (margins cleared). No new Python bindings; damage funnels
  through `mark_all_dirty`, so consumers eat a full redraw.

### Fixed
- Alt-screen column resize now conforms parked history to the new width.
  Previously history kept the stale width, so a later scroll could pop a
  ragged line into the live buffer (proptest find, fixed in-version).

## [0.8.2] — Wrap-flag model (reflow groundwork)

### Added
- Soft-wrap tracking: `Screen.wrapped` (parallel `Vec<bool>`) records which
  rows are continuations of the previous row via DECAWM wrap, with
  content-flow semantics — flags travel with rows on scroll/insert/delete
  and into scrollback, and clear only when a row's content is replaced as
  a line unit. No behavior change yet: nothing consumes the flags until
  v0.9.0 reflow. This deliberately deviates from the review sketch (which
  cleared flags on cursor movement): flags describe content, matching
  alacritty, so explicit cursor moves leave them intact.

## [0.8.1] — Background color erase (BCE)

### Fixed
- Erased cells now keep the current SGR state (background color erase)
  instead of reverting to screen defaults. A tool that paints a background
  and then pads or clears (`ESC[K`, `ESC[20X`, `ESC[2J`) keeps its block —
  matching xterm, VTE, kitty, Alacritty, and Windows Terminal. This also
  repairs ConPTY output: ConPTY trims trailing whitespace per row and
  re-emits the padding as EL/ECH with the block's SGR still active, which
  previously came back uncolored. Covers EL/ED/ECH/ICH/DCH/IL/DL, scrolling,
  and wide-char continuations; full reset and DECALN still use defaults.
- Erase and draw agree under reverse video: the template (including SGR
  reverse as-is) is copied verbatim with no DECSCNM adjustment, since the
  render path consumes the cell bits raw.

## [0.8.0] — Cheap geometry getters, bounded event log

### Added
- `TerminalState.visible_lines` / `visible_columns` (and `PtySession`
  delegates): O(1) visible-grid geometry. Previously the only way to
  measure the grid from Python was `len(visible_display())`, which builds
  the whole screen as strings just to count rows.

### Fixed
- The event log is capped at 1024 entries with drop-oldest semantics
  (`Screen::push_event` funnels all six event sites). A consumer that
  never drains (e.g. one calling only `styled_range`) leaked one
  `TitleChanged` per shell prompt, forever; retained order stays parser
  order with the newest event last, and `take_bell()` agreement is
  unaffected.

## [0.7.6] — Unix-only clippy lints (CI repair)

### Fixed
- First CI run on `main` failed the 0.7.3 lint gate on Linux/macOS:
  `platform_unix.rs` is never compiled by the Windows toolchain, so 8
  clippy errors hid there (dead fields, collapsible `if`s, redundant
  closure, `is_ok`+`unwrap`). Fixed; verified with local
  `--target x86_64-unknown-linux-gnu` and `--target aarch64-apple-darwin`
  clippy runs, which now join the pre-push checklist in `docs/RELEASING.md`.
- `PreparedCommand`'s `argv`/`env` fields are intentional ownership anchors
  for the exec-time raw pointers — marked `#[allow(dead_code)]` with the
  rationale, not removed (removing them would be a use-after-free).

### Changed
- Docs accuracy pass: removed the README tagline; ARCHITECTURE and QUICKREF
  updated to the current API surface (cwd/events modules, drain APIs, OSC
  rejoin note, `raw_output_cap` window semantics, lint gates in the build
  and pipeline sections); corrected stale claims — read **timeouts raise
  `PtyError`** (not `IOError`) across README/QUICKREF/ARCHITECTURE and the
  `PtyMaster.read_timeout` docstring; EOF contract documented; duplicate
  `PtyChild.wait` row removed from QUICKREF.
- Examples updated to the current API: the advanced emulator's hand-rolled
  `BellCounter` (raw-stream BEL counting with OSC-stripping regexes) is
  replaced by the edge-triggered `session.take_bell()`; fixed the exit-code
  toast, which always showed "unknown" because it tested `wait()`'s
  `ExitStatus` result with `isinstance(res, dict)`; corrected both usage
  docstrings (wrong script names) and the basic example's stale timeout-
  mapping comment; `examples/requirements.txt` floor raised to
  `stitch-pty>=0.5.7` (the version that introduced `take_bell`).

## [0.7.5] — Post-plan review fixes

### Fixed
- **OSC payload rejoin**: `;` splits OSC params, so titles and OSC 7 file
  URIs containing `;` were silently corrupted (`OSC 0;my;title` →
  `"mytitle"`, `file://host/docs;old` → `"/docsold"`). Titles and OSC 7
  now rejoin sub-params with `;`, like OSC 9;9 always did.
- **Zero-size resize panic**: `terminal.resize(0, 80)` left a zero-row
  screen that panicked in Rust on the next `feed()` (and `resize(24, 0)`
  + `ESC[1K` panicked at the 0.7.2 EL-1 clamp). `Screen::new` and
  `Screen::resize`/`HistoryScreen::resize` now clamp to a 1×1 minimum.
- **Raw window tail**: a single read chunk at/over `raw_output_cap` evicted
  the entire window including itself — `raw_output` was empty right after
  an oversized read. The window now keeps the chunk's last `cap` bytes.

### Changed
- CI now triggers on pushes to `dev` (previously only `main`/tags — the
  lint gates had never executed remotely) and pins `ruff==0.16.5`
  / `mypy==2.3.0` so a silent linter upgrade can't turn `mypy --strict`
  red on its own.
- `expect()` documents why the blanket `except PtyError` is safe.

## [0.7.4] — Concurrency contracts

### Added
- `tests/test_concurrency.py` pins three contracts: C1 concurrent reads
  conserve bytes (safe but partitioned — coherent streams need one reader),
  C2 cancelling a blocked `read()` mid-`terminate()` resolves promptly with
  the session not-alive, C3 drains between reads stay coherent (valid row
  indices, full line coverage, stably empty re-drains).
- `docs/ARCHITECTURE.md` gains a Concurrency Contract section recording the
  verified locking landscape (Windows pipe Mutex, kernel-serialized Unix
  reads, synchronous chunk handoffs — hence deliberately no Python lock).

## [0.7.3] — CI lint gates

### Fixed
- `cargo fmt --all` normalization (the tree was never fmt-clean).
- `cargo clippy --all-targets -- -D warnings` clean: removed same-type
  casts, annotated ConPTY transmutes, derived `Color::default`, dropped dead
  scroll helpers, collapsed conditionals, bool asserts.
- `ruff check` clean: sorted imports, `TimeoutError` alias, explicit
  `raise … from None`, `contextlib.suppress` cleanups, typed conversions.
- `mypy --strict` clean: explicit `bytes`/`int`/`bool`/`list` conversions
  at the FFI boundary (fail-fast shape assertions), `ignore_missing_imports`
  for the unstubbed `_core` extension.
- CI runs all four gates before the test suites.

## [0.7.2] — Property-based parser invariants

### Added
- `proptest` harness over the parser→Screen glue (arbitrary bytes,
  structured CSI/OSC/ESC mixes, split-boundary feeds) locking buffer shape,
  cursor bounds, scrollback caps, bell edge semantics, and alt-screen
  round-trips. Seeds persist under `proptest-regressions/`; CI pins
  `PROPTEST_CASES=256`. See `docs/TESTS.md` for the failure workflow.

### Fixed
- `erase_in_line` mode 1 (`ESC[1K`) panicked with an out-of-bounds index
  when the cursor sat in pending-wrap state (`x == columns` after printing
  exactly to the margin). Now clamps like `erase_in_display` already did.
  Found by the new proptest; locked by `test_erase_in_line_mode1_at_pending_wrap`.

## [0.7.1] — `expect()` upgrades

### Added
- `expect()` accepts `str` literals, bytes regexes (`re.Pattern[bytes]`),
  and lists/tuples of mixed patterns (first in list order wins).
- Returns `ExpectResult(index, match, buffer)`; `match` is `None` for
  literal hits. `TimeoutError`s carry the bytes seen as `.buffer`.

### Changed (minor behavior note)
- `expect()` no longer returns bare `bytes`. Shims keep old call sites
  working (`in`, `== bytes`, `bytes(…)`); only `.decode()` must now be
  spelled `bytes(result).decode()`.

## [0.7.0] — Consolidated event polling

### Added
- Ordered low-frequency event pipeline: `Screen.events` log (bell, title,
  icon, cwd, alt-screen, scrollback growth), drained per frame via
  `HistoryScreen::take_events()` → `TerminalState.poll_events()` →
  `PtySession.poll_events()` as `(tag, payload)` tuples.
- `take_bell()` and the event drain are unified: consuming either path
  consumes the pending bell for both. `reset()` drops pending events to
  keep that agreement across RIS.

## [0.6.1] — OSC 7 / OSC 9;9 cwd tracking

### Added
- Terminal tracks the shell working directory from `OSC 7;file://…`
  (macOS/Linux shells, Windows Terminal) and `OSC 9;9;…` (ConPTY):
  `TerminalState.cwd` getter + `PtySession.cwd` property (`None` until the
  first report). Malformed sequences leave state untouched, never panic.
- `cwd` survives alt-screen switches and `reset()` — it is shell state.

## [0.6.0] — `cwd` on `spawn()`

### Added
- `spawn(..., cwd=...)`: working directory for the child (Unix: `chdir`
  after fork; Windows: `lpCurrentDirectory`). `None` inherits the parent's.
- A missing path or non-directory raises `PtyError` (`ForkFailed`) at spawn
  instead of producing a silently dead child.

## [0.5.13] — Repo hygiene, release process

### Added
- `CHANGELOG.md` (this file), backfilled from git history.
- `docs/RELEASING.md`: runnable release checklist (version bumps, suites,
  `cargo publish`, wheel build, PyPI upload, tag, GitHub release, Kilim bump).

## [0.5.12] — Repo hygiene

### Fixed
- Removed tracked `tests/__pycache__/*.pyc` files; added `.gitattributes`
  (LF normalization) and explicit `*.pyd`/`*.so` ignores.

## [0.5.11] — `spawn()` forwards session options

### Fixed
- `spawn()` now accepts `scrollback` and `raw_output_cap` and forwards them
  to `PtySession` (previously the documented entry point could not configure
  scrollback at all).

## [0.5.10] — Asyncio loop hygiene

### Fixed
- `expect()` uses `asyncio.get_running_loop()` instead of the deprecated
  `get_event_loop()` (same fix in `tests/conftest.py`, `tests/test_basic.py`).
- `filterwarnings = ["error::DeprecationWarning"]` in pytest config: future
  deprecations fail the suite instead of rotting silently.

## [0.5.9] — Typed EOF error

### Fixed
- New `PtyErrorKind::Eof` (`"PTY EOF: child side closed"`), raised for Unix
  EIO on the master and ConPTY `ERROR_BROKEN_PIPE`/`ERROR_NO_DATA` reads.
  Surfaces to Python as `OSError(errno=0, kind="eof")` — `PtySession.read()`
  and `read_timeout()` branch on the stable `kind` attribute.
- The `"os error 5" in str(e)` string match is kept as a legacy fallback only.

## [0.5.8] — Bounded raw-output capture

### Fixed
- `PtySession` raw-output capture is now a sliding window:
  `raw_output_cap` (default 1 MiB, `None` = legacy unbounded).
  Long-lived sessions no longer accumulate their full byte history in RAM,
  and `raw_output` is O(cap) instead of O(session lifetime).

## [0.5.7] — BEL, dirty rows

### Added
- BEL (0x07) support: edge-triggered, coalescing `take_bell()` on `Screen`,
  `HistoryScreen`, `TerminalState`, and `PtySession`. OSC BEL terminators
  (window titles) never ring.
- `take_dirty_rows()`: draining, sorted, coalescing dirty-region API on the
  same four layers, for frame-based repaint consumers.

### Fixed
- `resize()`, `reset()`, and DECCOLM `clear_buffer()` now mark all rows dirty
  instead of silently clearing the set (consumers previously saw `[]` after a
  resize while all content had changed).

## [0.5.6] — Timers → OS threads

### Fixed
- Replaced Tokio timers with OS threads + Python-side polling (macOS runtime
  starvation workaround).

## [0.5.5] — Windows drain fix

### Fixed
- `try_io` `Result<io::Result, TryIoError>` pattern on Unix reads; Windows
  test drain fix for child-exit EOF.

## [0.5.0] — Initial release
