# stitch-pty — Improvement Plan

> Working roadmap for the `dev` branch. Baseline: **v0.5.7** at commit `d17f5d6`
> (334+ Rust tests, 160 Python tests, all green).

---

## 1. Governance rules

Every version below is **exactly one commit**, pushed to `dev` immediately.

### Bump rules

| Change class | Bump | Examples in this plan |
|---|---|---|
| Bug fix, test-only, CI/chore, docs | `+0.0.1` (patch) | 0.5.8, 0.5.9, 0.6.1 |
| New feature, major refactor | `+0.1.0` (minor) | 0.6.0, 0.7.0 |

Incremental features that build directly on a just-landed feature and don't
expand the API surface (e.g. 0.6.1 OSC tracking on top of 0.6.0 `cwd`) ride
as patches, per the roadmap annotations.

After a minor bump, the patch sequence continues from the new minor
(`0.6.0 → 0.6.1 → …`), never under the old minor.

### Definition of Done — every version, no exceptions

1. Bump **all three** version fields:
   - `Cargo.toml` → `version = "X.Y.Z"`
   - `pyproject.toml` → `version = "X.Y.Z"`
   - `python/stitch_pty/__init__.py` → `__version__ = "X.Y.Z"`
2. `cargo test --lib` → 0 failures.
3. `uvx maturin develop --release` (rebuild `_core` against the edited Rust).
4. `uv run python -m pytest tests/ -q` → 0 failures.
5. Append a row to `CHANGELOG.md` under the new version heading (from 0.5.13 on).
6. Commit: `<type>: <summary> (vX.Y.Z)` — one commit, nothing else mixed in.
7. `git push origin dev`.

If any suite is red, the version is not released; fix forward within the same
version before pushing.

---

## 2. Roadmap at a glance

| Version | Type | Title | Tier |
|---|---|---|---|
| 0.5.8 | fix | Bounded `_raw_output` capture | 1 |
| 0.5.9 | fix | Typed EOF error replaces `"os error 5"` string matching | 1 |
| 0.5.10 | fix | `get_running_loop()` in `expect()` | 1 |
| 0.5.11 | fix | `spawn()` forwards `scrollback` (and raw-output cap) | 1 |
| 0.5.12 | chore | Repo hygiene: untrack `.pyc`, add `.gitattributes` | 1 |
| 0.5.13 | chore | `CHANGELOG.md` + release checklist | 1 |
| 0.6.0 | feature (minor) | `cwd` parameter on `spawn()` — new user-facing option, platform-layer change | 2 |
| 0.6.1 | feature (patch) | OSC 7 / 9;9 cwd tracking — incremental, builds directly on 0.6.0; small parser addition | 2 |
| 0.7.0 | feature (minor) | Consolidated event polling (`poll_events()`) — new API surface | 2 |
| 0.7.1 | feature (patch) | `expect()` upgrades: regex + multi-pattern | 2 |
| 0.7.2 | test | Property-based invariants for parser→Screen | 3 |
| 0.7.3 | fix | CI lint gates (clippy, fmt, ruff, mypy) | 3 |
| 0.7.4 | test | Concurrency contract tests | 3 |

---

## 3. Tier 1 — Small bugs (patch series `0.5.8 … 0.5.13`)

### v0.5.8 — fix: bounded `_raw_output` capture

**Problem.** `python/stitch_pty/__init__.py`: every `read()`/`read_timeout()`
appends the chunk to `self._raw_output` and nothing ever trims it. A shell
session running for days accumulates its entire byte history in RAM, and the
`raw_output` property re-joins all of it on every access (O(total bytes) per
call). For a polling frontend (Kilim, 60 ms ticks) this is a slow leak with a
growing per-poll cost once it's ever inspected.

**Design.**

- Replace `self._raw_output: list[bytes]` with a size-bounded store:
  - `self._raw_chunks: deque[bytes]`, `self._raw_bytes: int`
  - New constructor kwarg: `PtySession(inner, scrollback=1000, raw_output_cap=1_048_576)`
    — cap in bytes, default 1 MiB; `raw_output_cap=None` restores unlimited
    (documented as the leaky legacy mode).
  - Append path: append chunk, add `len`; while `_raw_bytes > cap`, `popleft()`
    and subtract.
- `raw_output` property unchanged in signature (`b"".join(self._raw_chunks)`)
  but now O(cap) worst case, not O(session).
- Document the cap in the property docstring: *"last N bytes (default 1 MiB),
  not the full session log"*.

**Files.** `python/stitch_pty/__init__.py`, `tests/test_pty_session.py`.

**Tests.**

- Feed > cap bytes through `read()`; assert `len(session.raw_output) <= cap`.
- Assert `raw_output_cap=None` keeps old behavior.
- Assert `PtySession.__init__` still accepts positional `inner` (compat).

**Commit.** `fix: bound PtySession raw_output capture to raw_output_cap bytes (v0.5.8)`

---

### v0.5.9 — fix: typed EOF error replaces `"os error 5"` string matching

**Problem.** `__init__.py:252,272` detect child-exit EOF by
`e.errno == errno.EIO or "os error 5" in str(e)`. That string is Rust's
`io::Error` Display text — it breaks on locale/format changes, on Windows
(where ConPTY yields `ERROR_BROKEN_PIPE`, os error 109, or a clean EOF), and
it conflates *any* EIO with EOF.

**Design.**

- `src/errors.rs`: add `PtyErrorKind::Eof` variant
  (`#[error("PTY EOF: child side closed")]`).
- Read paths (`src/async_io.rs`, platform backends): map
  - Unix EIO on the master after slave close → `Eof`
  - Windows `ERROR_BROKEN_PIPE` / `ERROR_NO_DATA` on read → `Eof`
- `errors.rs` Python mapping: `Eof` maps to the existing `IOError` exception
  but **sets `.errno = 0` and a stable `kind == "eof"` attribute** so Python
  can branch without string matching.
- `python/stitch_pty/__init__.py`: both `read()` and `read_timeout()` catch
  `IOError` with `getattr(e, "kind", None) == "eof"` → `return b""`.
  Keep the legacy `errno.EIO` check as a fallback for one minor cycle.
- Add a comment tying this to the `# Linux PTY returns EIO…` block it replaces.

**Files.** `src/errors.rs`, `src/async_io.rs`, `src/platform_unix.rs`,
`src/platform_windows.rs`, `python/stitch_pty/__init__.py`,
`tests/test_pty_session.py`.

**Tests.**

- Spawn a short-lived child, drain reads → `b""` returned, no raise, on the
  CI matrix (this is exactly the Windows path that motivated `0.5.5`).
- Assert the raised-then-swallowed error path is gone: after EOF, a second
  `read()` still returns `b""`.

**Risk.** This touches the read hot path on both platforms — review the
Windows `error.kind()` mapping against `windows` crate constants before merge.

**Commit.** `fix: typed Eof error for PTY master reads; drop "os error 5" matching (v0.5.9)`

---

### v0.5.10 — fix: `get_running_loop()` in `expect()`

**Problem.** `__init__.py:414,417` use the deprecated `asyncio.get_event_loop()`
inside a coroutine; every other site (lines 172, 181, 191, …) correctly uses
`get_running_loop()`. On Python 3.12+ this is a DeprecationWarning and a
future hard error.

**Design.** Two-line replacement, plus a grep-driven guard:
`grep -n "get_event_loop" python/ src/` must return zero hits (assert this in
the commit body).

**Files.** `python/stitch_pty/__init__.py`.

**Tests.** Existing `test_pty_session.py::test_expect*` run under `-W error::DeprecationWarning`
in this version's pytest invocation (add `filterwarnings = ["error::DeprecationWarning"]`
to `pyproject.toml [tool.pytest.ini_options]`).

**Commit.** `fix: use get_running_loop() in expect(); make deprecation warnings fatal in tests (v0.5.10)`

---

### v0.5.11 — fix: `spawn()` forwards `scrollback`

**Problem.** `PtySession.__init__(inner, scrollback=1000)` accepts a scrollback
capacity, but `spawn()` constructs `PtySession(inner)` with no kwarg — the
documented entry point cannot configure scrollback at all.

**Design.**

- `spawn(program, args=None, env=None, winsize=None, scrollback=1000, raw_output_cap=1_048_576)`
  → forwards both to `PtySession(...)`.
- README Quick Start + `spawn()` docstring mention the new kwargs.

**Files.** `python/stitch_pty/__init__.py`, `README.md` (API Reference → Core Functions),
`tests/test_pty_session.py`.

**Tests.** `spawn(..., scrollback=50)` on a program emitting 200 lines →
`len(session.scrollback) == 50`.

**Commit.** `fix: forward scrollback and raw_output_cap through spawn() (v0.5.11)`

---

### v0.5.12 — chore: repo hygiene

**Problem / design.**

- `git rm --cached tests/__pycache__/*.pyc` — three compiled-caches are tracked
  from before `.gitignore` covered them.
- Add `.gitattributes` with `* text=auto` + explicit eol for `*.rs`, `*.py`,
  `*.toml`, `*.md` (LF) — commits currently emit `LF will be replaced by CRLF`
  warnings on every touch; normalize once, in its own commit, so the diff noise
  is isolated from the following versions.
- Verify `python/stitch_pty/_core.pyd` is untracked (it is today — keep it that
  way; add `*.pyd` to `.gitignore` explicitly).

**Files.** `.gitattributes`, `.gitignore`, index.

**Commit.** `chore: untrack pyc caches, normalize line endings via .gitattributes (v0.5.12)`

---

### v0.5.13 — chore: `CHANGELOG.md` + release checklist

**Design.**

- New `CHANGELOG.md`, Keep-a-Changelog format, backfilled from `git log`
  highlights: 0.5.0 (initial), 0.5.5 (Windows drain fix), 0.5.6 (timers → OS
  threads), 0.5.7 (BEL / `take_bell`), plus everything from this plan as it lands.
- New `docs/RELEASING.md`: the Definition-of-Done checklist from §1 as a
  runnable sequence (version bumps, suites, `cargo publish`, `maturin build`,
  PyPI upload, tag `vX.Y.Z`, GitHub release, bump Kilim).
- Rule recorded there: **the version is released from `dev` only when all
  suites are green on the exact commit that carries the bump.**

**Files.** `CHANGELOG.md`, `docs/RELEASING.md`.

**Commit.** `chore: add CHANGELOG.md and release checklist (v0.5.13)`

---

## 4. Tier 2 — Features (series `0.6.0 … 0.7.1`)

### v0.6.0 — feature: `cwd` parameter on `spawn()`

**Problem.** `spawn_platform(program, args, env, winsize)` (`src/platform.rs:162`)
has no working-directory parameter. Consumers spawn shells and immediately
`cd`, which races output capture and pollutes scrollback.

**Design.**

- Thread `cwd: Option<&str>` through the stack:
  - `python_api.rs::spawn` — new `cwd: Option<String>` arg
    (`#[pyo3(signature = (program, args=None, env=None, winsize=None, cwd=None))]`).
  - `platform.rs::spawn_platform` — new parameter, doc-comment the Windows
    note (spawn must stay async for the IOCP pump; cwd changes nothing there).
  - `platform_unix.rs::spawn` / `platform_windows.rs::spawn` — apply to the
    `CommandBuilder` (`builder.cwd(path)`) before `spawn()`.
  - Validate: nonexistent dir → `PtyErrorKind::ForkFailed` with the OS error
    text (don't pre-check with `fs::exists` — TOCTOU; let the spawner fail).
- Python wrapper: `spawn(..., cwd: str | None = None)`, docstring + README row
  in API Reference.

**Files.** `src/python_api.rs`, `src/platform.rs`, `src/platform_unix.rs`,
`src/platform_windows.rs`, `python/stitch_pty/__init__.py`, `README.md`, tests.

**Tests.**

- `spawn(sys.executable, ["-c", "import os;print(os.getcwd())"], cwd=tmp_path)`
  → output contains the tmp path. Both platforms (CI matrix covers it).
- `spawn(..., cwd="Z:/definitely/missing")` → raises `PtyError`.

**Commit.** `feat: cwd parameter on spawn() (v0.6.0)`

---

### v0.6.1 — feature: OSC 7 / OSC 9;9 cwd tracking

> Patch bump: incremental feature building directly on 0.6.0's `cwd` plumbing;
> small parser addition, no new API surface beyond one getter/event.

**Problem.** Terminals report the shell's cwd via
`OSC 7 ; file://host/path BEL` (macOS/Linux shells, Windows Terminal) or
`OSC 9;9 ; path ST` (ConPTY). The parser currently drops both. Kilim-style UIs
use cwd for tab tooltips/launch defaults.

**Design.**

- `parser.rs::osc_dispatch`: handle `b"7"` (params[1] = URI) and the
  `b"9"` + param `9;9` form. Parse:
  - `file://host/path` → strip scheme/host, percent-decode (small hand-rolled
    decoder or `percent-encoding` crate — prefer the crate, it's tiny).
  - Windows drive forms: accept `file:///C:/…` → `C:\…` normalization helper
    in a new `src/terminal/cwd.rs` with unit tests (this parsing is fiddly —
    isolate it).
- `Screen.cwd: Option<String>`; `enter_alt_screen`/`reset` do **not** clear it
  (cwd survives alt-screen; it's shell state, not screen state).
- 0.6.1 exposes the state: `TerminalState.cwd` getter, `PtySession.cwd`
  property. Emitting `TermEvent::CwdChanged` on change is wired up when the
  0.7.0 event pipeline lands (this section's enum mention is its future hook).

**Files.** `src/terminal/cwd.rs` (new), `src/terminal/parser.rs`,
`src/terminal/screen.rs`, `src/terminal/history.rs`, `src/terminal_api.rs`,
`python/stitch_pty/__init__.py`, `README.md`, tests.

**Tests.**

- `OSC 7 ; file://myhost/home/user%20name BEL` → `"/home/user name"`.
- `OSC 9;9 ; C:\Users\Main ST` → `"C:\Users\Main"`.
- Malformed OSC 7 (no scheme, binary junk) → cwd unchanged, no panic
  (this is untrusted shell output — never `unwrap()`).

**Commit.** `feat: track shell cwd via OSC 7 and OSC 9;9 (v0.6.1)`

---

### v0.7.0 — feature: consolidated event polling (`poll_events()`)

> Minor bump: new API surface (event enum + pymethod) and a refactor of how
> low-frequency signals reach consumers.

**Problem.** A frontend tick must make N separate FFI crossings per frame —
`take_bell()`, `take_dirty_rows()`, `title`, mode getters — and has no way to
know *when* things happened inside a feed. Cost is small but grows with every
new signal (0.6.1 adds cwd; future: scrollback-growth).

**Design.**

- New `src/terminal/events.rs`:
  ```rust
  pub enum TermEvent {
      Bell,
      TitleChanged(String),
      IconChanged(String),
      CwdChanged(String),        // populated from 0.6.1 on
      AltScreen { entered: bool },
      ScrollbackGrew(u64),       // lines pushed since last drain
  }
  ```
- `Screen` gains `events: Vec<TermEvent>` (drained, not coalesced — ordering
  is the point). `ring_bell`, `set_title`, `set_icon_name`,
  `enter/exit_alt_screen` push events. `take_dirty_rows()` stays as-is: rows
  are high-frequency and set-semantics is correct there; events are
  low-frequency and order-semantics is correct there. Both remain drained
  per frame.
- `HistoryScreen::take_events() -> Vec<TermEvent>` forwarder (drain).
- `TerminalState.poll_events() -> list[tuple[str, object]]` pymethod —
  `("bell", None)`, `("title", "…")`, … Stable string tags so Python needs no
  new exception/int class.
- Python: `PtySession.poll_events()` delegate. `take_bell()` is kept and
  documented as equivalent to filtering `poll_events()` for `("bell", …)`.
- Feed path orders events exactly as the parser produced them.

**Files.** `src/terminal/events.rs` (new), `src/terminal/mod.rs`,
`src/terminal/screen.rs`, `src/terminal/history.rs`, `src/terminal_api.rs`,
`python/stitch_pty/__init__.py`, `docs/ARCHITECTURE.md`, `README.md`, tests.

**Tests.**

- Feed `ESC ]2;t BEL`, then BEL, then `?1049h` → events
  `[("title","t"), ("bell",None), ("altscreen",True)]` in order.
- Draining twice → second drain empty.
- Bell via both paths consistent: `take_bell()` and `poll_events()` agree
  within one frame (poll first, then take_bell must be False).

**Docs.** ARCHITECTURE gains an "Event pipeline" subsection; README API table
row for `poll_events`.

**Commit.** `feat: ordered term event pipeline + poll_events() (v0.7.0)`

---

### v0.7.1 — feature: `expect()` upgrades — regex + multi-pattern

> Patch bump: extends an existing function's accepted pattern types; no new
> API surface beyond the return type's shims.

**Problem.** `expect(pattern: bytes)` accepts one literal byte pattern.
pexpect-grade consumers need alternation ("awaiting prompt OR error banner")
and regex.

**Design.**

- Signature:
  ```python
  async def expect(
      self,
      patterns: bytes | str | re.Pattern | list[bytes | str | re.Pattern],
      timeout: float = 30.0,
  ) -> ExpectResult
  ```
- New frozen dataclass `ExpectResult(index: int, match: re.Match | None, buffer: bytes)` —
  `index` indexes into the (normalized) pattern list; `match` is the regex
  match object or `None` for byte-literal hits.
- Literal bytes/str patterns are compiled as `re.escape`d patterns; str is
  encoded UTF-8. Matching runs on the accumulated buffer after each chunk
  (chunk boundaries can split a pattern — matching the whole buffer each time
  is the existing behavior, kept).
- Backward compat: `bytes` positional still works and returns… **breaking
  decision**: old code expects `bytes` back. Keep a `return_buffer: bool = True`
  kwarg? No — cleaner: old call shape returns `ExpectResult`, which defines
  `__bytes__` and `__eq__` with bytes so `assert pattern in await
  session.expect(b"PROMPT")` keeps working; document the migration in the
  changelog as a minor-version behavior note.
- EOF/timeout semantics unchanged (`TimeoutError` with the buffer attached as
  the exception's `.buffer` attribute — new, small QoL).

**Files.** `python/stitch_pty/__init__.py`, `README.md` (pexpect-style section),
`tests/test_pty_session.py`.

**Tests.**

- Literal (legacy shape), regex, list-of-mixed, EOF-before-match, chunk-split
  pattern, timeout carries `.buffer`.

**Commit.** `feat: expect() supports regex and multi-pattern matching (v0.7.1)`

---

## 5. Tier 3 — Robustness (patch series `0.7.2 … 0.7.4`)

### v0.7.2 — test: property-based invariants for parser→Screen

**Problem.** The vte state machine is upstream-tested; the glue
(`Performer`/`Screen`/`HistoryScreen`) is covered by example-based tests only.
Malformed/untrusted byte streams are the *normal input* for a terminal library.

**Design.**

- Dev-dep `proptest = "1"` in `Cargo.toml`.
- Strategies: arbitrary `Vec<u8>`, plus "structured" mixes (valid CSI/OSC
  fragments spliced with random bytes), plus high-bit/UTF-8 boundary noise.
- Invariants asserted after every feed, for feed sizes 1..N and split
  boundaries:
  1. `buffer.len() == lines` and every row `len() == columns`
  2. cursor always `x < columns`, `y < lines` (0.5.4-era bug class)
  3. `scrollback_len <= scrollback_lines` (or unbounded when 0)
  4. `take_bell()` never returns true twice in a row without an intervening
     BEL byte in the stream
  5. `display()` rows are valid UTF-8 (trivially true by construction —
     assert anyway to lock it)
  6. alt-screen round-trip: `?1049h … ?1049l` restores the parked buffer's
     row count
- Run in CI as part of `cargo test` (proptest default cases; the CI job gets
  `PROPTEST_CASES=256`).
- Any shrinking counterexample gets promoted into a named `#[test]` regression
  (record the rule in `docs/TESTS.md`).

**Files.** `Cargo.toml`, `src/terminal/parser.rs` (tests mod),
`src/terminal/history.rs` (tests mod), `docs/TESTS.md`.

**Commit.** `test: property-based invariants for terminal parsing pipeline (v0.7.2)`

---

### v0.7.3 — fix: CI lint gates

**Problem.** `.github/workflows/CI.yml` runs `cargo test` and `pytest` but no
linters — ruff/mypy/clippy configs exist locally and silently drift.

**Design.** Add to the Test job (before tests):

```yaml
- run: cargo fmt --all --check
- run: cargo clippy --all-targets -- -D warnings
- run: ruff check python/ tests/
- run: mypy python/stitch_pty/
```

Fix whatever they flag **in this same commit** so the gate lands green
(expected offenders: clippy lints in `screen.rs` long-line match arms; mypy
strict-mode complaints on the new `ExpectResult`).

**Files.** `.github/workflows/CI.yml`, assorted lint fixes.

**Commit.** `fix: enforce fmt/clippy/ruff/mypy gates in CI (v0.7.3)`

---

### v0.7.4 — test: concurrency contract tests

**Problem.** Behavior for two concurrent `read()`s, `read()` racing
`terminate()`, and `poll_events()` during an active feed is defined only by
accident of the current implementation. Frontends (Kilim) do all three.

**Design.** Python-side tests that pin current sane behavior and turn
violations into explicit decisions:

1. **Single-reader rule**: two tasks `read()` concurrently → document +
   assert no interleaved corruption (reads are serialized by the Tokio
   semaphore/mutex in `async_io.rs` — verify; if there is no guard, that's a
   real finding to fix in this version and note in ARCHITECTURE).
2. **Shutdown race**: `read()` cancelled mid-await while `terminate()` runs →
   no hang, session reports not-alive.
3. **Drain-during-feed**: `take_dirty_rows()`/`poll_events()` between two
   `read()`s never sees a partially-written row set (rows are complete lines
   by construction — assert via an output pattern).
4. Each test gets a comment naming the contract it pins, so failures read as
   decisions, not flakes.

**Files.** `tests/test_concurrency.py` (new), possibly `src/async_io.rs`,
`docs/ARCHITECTURE.md` (Threading/Concurrency section).

**Commit.** `test: pin concurrency contracts for reads/terminate/drains (v0.7.4)`

---

## 6. Deferred backlog (unscheduled, does not consume versions yet)

- **Async iteration protocol** — `async for chunk in session:` sugar over read.
- **`read_line()` / `readuntil(b"\n")`** — common for line-oriented tools.
- **Scrollback-capacity events** — expose scrollback *content* growth beyond
  `history_size` delta if frontends want incremental scrollback rendering
  (pairs with `ScrollbackGrew` from 0.7.0 — promote when a consumer needs it).
- **Suspend/resume flow control** (CSI ? 1007 etc.) — audit which modes the
  parser tracks vs. terminals in the wild.
- **README claims audit** — verify "Zero GIL contention" and "50ms polling"
  rows against current implementation reality before the 1.0 marketing push.
- **1.0.0** — API freeze review: settle the `expect()` return type, event tag
  vocabulary, and error taxonomy; semver promises start there.

---

## 7. Execution order & current status

| Order | Version | Status |
|---|---|---|
| 1 | 0.5.8 | ☐ pending |
| 2 | 0.5.9 | ☐ pending |
| 3 | 0.5.10 | ☐ pending |
| 4 | 0.5.11 | ☐ pending |
| 5 | 0.5.12 | ☐ pending |
| 6 | 0.5.13 | ☐ pending |
| 7 | 0.6.0 | ☐ pending |
| 8 | 0.6.1 | ☐ pending |
| 9 | 0.7.0 | ☐ pending |
| 10 | 0.7.1 | ☐ pending |
| 11 | 0.7.2 | ☐ pending |
| 12 | 0.7.3 | ☐ pending |
| 13 | 0.7.4 | ☐ pending |

Update this table as versions land. Each row = one commit on `dev`, pushed.
