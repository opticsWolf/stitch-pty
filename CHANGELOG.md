# Changelog

All notable changes to stitch-pty are documented here, newest first.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Every version below is exactly one commit on `dev` (see `docs/RELEASING.md`).

## [Unreleased]

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
