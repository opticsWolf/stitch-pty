# Changelog

All notable changes to stitch-pty are documented here, newest first.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Every version below is exactly one commit on `dev` (see `docs/RELEASING.md`).

## [Unreleased]

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
