# stitch-pty 0.10 — Release and Follow-up Plan

> Status: **0.10.2 through 0.10.6 implemented on `dev`** (G4 + H3, G7, H1, H2/H4, then P1); none of these patches is tagged or published yet. 0.10.1 and 0.10.0 are published; final release steps wait for explicit confirmation.
> Release branch: `dev` (contains `v0.10.0` at `e620cc8` and `v0.10.1` at `97e03ab`; 0.10.2 at `3b3abb2`; 0.10.3 at `b3f2647`; 0.10.4 at `de50992`; 0.10.5 at `5c66fc2`; 0.10.6 follows; `main` is at `b0e4510`)
> Baseline: `dev` after the PR #2 merge (`26ab236`); release commit `e6f10d2`; tag `v0.10.0` at `e620cc8`; 0.10.1 release at `97e03ab`; 0.10.2 at `3b3abb2`; 0.10.3 at `b3f2647`; 0.10.4 at `de50992`; 0.10.5 at `5c66fc2`; all untagged; pre-rebase PR head was `5e68aa1`
> Last updated: 2026-11-14
> Rules: [`RELEASING.md`](RELEASING.md) and the version bump rules in
> [`IMPROVEMENT_PLAN.md`](IMPROVEMENT_PLAN.md)
> Reference review: `portable-pty` 0.9.0 (WezTerm), `alacritty_terminal`
> 0.26.1-dev, and `emux-pty` current `main`

This document defines what ships in 0.10.0, what is deliberately deferred, and
how the deferred gaps become follow-up releases. It is the hand-off document
for reviewing, merging, and releasing PR #2 without expanding its scope.

The reference review in §3 changed three design assumptions:

1. **Do not force a unified raw-handle type.** WezTerm and Alacritty expose
   platform-gated raw handles, not one cross-platform value. `HPCON` is not an
   owned kernel handle, so `AsHandle` is the wrong abstraction.
2. **Windows `interrupt()` is not solved by any reference.** WezTerm uses
   `TerminateProcess`, Alacritty has no signal API, and emux has none. The
   honest default is an explicit “unsupported on ConPTY” error.
3. **Windows `close()` is an order-sensitive, potentially blocking operation.**
   Alacritty documents that `ClosePseudoConsole` blocks until the conout pipe is
   drained and that dropping `conout` first can deadlock. A naive `close()`
   is worse than the current gap.

None of this changes 0.10.0. It changes how G1, G3, G4, and G7 are specified.

## Contents

- [1. Current state](#1-current-state)
- [2. 0.10.0 scope](#2-0100-scope)
- [3. Reference review](#3-reference-review)
- [4. Deferred gaps](#4-deferred-gaps)
- [5. Additional findings and follow-ups](#5-additional-findings-and-follow-ups)
- [6. Sequencing and version buckets](#6-sequencing-and-version-buckets)
- [7. Issue-filing checklist](#7-issue-filing-checklist)
- [8. 0.10.0 release gate](#8-0100-release-gate)
- [9. Open decisions / risks](#9-open-decisions--risks)
- [10. Definition of done for the 0.10 series](#10-definition-of-done-for-the-010-series)
- [Appendix A — source references](#appendix-a--source-references)

---

## 1. Current state

| Item | State |
|---|---|
| PR | [#2 — feat(rust): add first-class PtySession, explicit env contract, winsize/EOF fixes](https://github.com/opticsWolf/stitch-pty/pull/2) — **MERGED** via rebase (`26ab236`) |
| Base / head | `dev` ← `feat/rust-session-api` (merged) |
| Merge state | `MERGED`; pre-merge was `MERGEABLE` / `CLEAN` |
| CI | 0.10.1 tag run `43581964572`: all 6 test jobs, all 4 wheel builds, PyPI, and crates.io green. 0.10.2 push run [`43585964604`](https://github.com/opticsWolf/stitch-pty/actions/runs/43585964604) at `3b3abb2` green; publish jobs skipped. 0.10.3–0.10.6 CI follow their implementation commits; releases are gated on confirmation. |
| Version | `0.10.6` on `dev` (P1); 0.10.5 at `5c66fc2` (H2/H4); 0.10.4 at `de50992` (H1); 0.10.3 at `b3f2647` (G7); 0.10.2 at `3b3abb2` (G4 + H3); `0.10.1` released at `97e03ab`; `0.10.0` at `e620cc8` |
| Publication | PyPI/crates.io 0.10.1 and 0.10.0 published; 0.10.2 through 0.10.6 not tagged or published |
| Local gates | 0.10.6: `fmt`, `clippy` (host + Linux/macOS), `cargo test` (419 unit + 19 integration [Windows; 19 Unix] + 1 doc), `--no-default-features` (419), `check --features python`, `doc`, `ruff`, `mypy`, `maturin develop --release` (0.10.6), `pytest` (199 passed, 1 skipped) |
| Branch sync | `dev` is ahead with 0.10.2 through 0.10.6; `main` is at `b0e4510`; tags `v0.10.0`/`e620cc8` and `v0.10.1`/`97e03ab`; local `feat/rust-session-api` still points at pre-rebase `5e68aa1` |
| Reference review | Completed 2026-11-14; findings folded into §3–§5 |
| Post-merge review | Parser chunking and Rust/Python decoupling findings tracked as P1/D1/D2 in §5.7–§5.9 |
| Follow-up issues | [#3–#17](https://github.com/opticsWolf/stitch-pty/issues) filed; G4/H3/G7/H1/H2/P1 work is implemented but #5/#6/#9/#10/#11/#15 stay open until their releases; H4 (#12) has a documented no-change decision |
| Issue #1 | [Kitty keyboard protocol request](https://github.com/opticsWolf/stitch-pty/issues/1) — OPEN; tracked as K1a/K1b in §5.5–§5.6 |

The pre-rebase PR contained the original review commits plus the review follow-ups. The rebase merge rewrote these SHAs on `dev`; the sequence is:

```text
26ab236 ci: run pull_request checks for dev base
c02cb01 test(rust): probe env with /usr/bin/env on Unix
0a43145 docs: fix session/EOF/wait/interrupt docs; sync test maps
eccc541 test(rust): cover wait idempotency and terminate escalation
9afdd0f docs(rust): correct Windows EOF semantics; test Unix EOF
a8313a4 fix(rust): default winsize to 24x80 on all platforms
2efa0c5 fix(rust): make env inheritance explicit; restore empty env
3bdd283 test: resize uses a non-interactive child for macOS stability
a32746b fix: unblock clippy on Rust 1.100 (needless_range_loop)
7831173 feat: first-class Rust PtySession + spawn/env fixes
```

The release work on top of that sequence is:

```text
f6d73a2 docs: add 0.10 release and follow-up plan
99f7af7 test(rust): wait for ConPTY setup output before timeout assertion
e6f10d2 release: 0.10.0 (v0.10.0)
e620cc8 docs: record 0.10.0 release readiness (tag v0.10.0)
```

The release commit contains only the version bump and CHANGELOG update;
`99f7af7` is the separate test-stability fix for a delayed ConPTY setup-output
race. The docs-only run before the release exposed that race on Windows py3.13.
The tag points at `e620cc8`, the docs commit after the version bump, matching
the `v0.9.1` tag convention.

CI trigger note: the workflow originally ran `pull_request` checks only for
`main`. PR #2 adds `dev` to that trigger so retargeted PRs are tested
(`.github/workflows/CI.yml`). This is part of the PR, not a separate change.

---

## 2. 0.10.0 scope

### 2.1 Shipped in 0.10.0

- **Rust `PtySession`** (`src/session.rs`) mirroring the Python session API:
  `spawn` / `spawn_with_env`, `read` / `read_timeout`, `write` / `write_all`,
  `resize` / `set_winsize` / `get_winsize`, `wait` → `Option<ExitStatus>`,
  `terminate` / `kill` / `interrupt` / `send_signal`, `pid`, `is_alive`,
  `pty()`, `child()`, `into_parts()`.
- **Backend helpers**: `PtyBackend::write_all`,
  `ChildBackend::wait_status`, `ArcPtyBackend` / `ArcChildBackend`, and a root
  `Winsize` re-export.
- **Explicit environment contract** for the low-level API:
  `spawn_platform(..., env: Option<&[(String, String)]>, ...)`.
- **Winsize default**: `None` normalizes to 24×80 on all platforms, including
  Unix `openpty(None)`.
- **Windows EOF truthfulness**: documented that ConPTY can keep the output pipe
  open after child exit; tests no longer pretend reads hit EOF immediately.
- **Unix EOF coverage**: a Unix-only integration test verifies EOF after child
  exit.
- **`write_all` correctness**: handles short writes and `WriteZero` instead of
  looping forever.
- **Docs/tests**: README quick start, QUICKREF, ARCHITECTURE, TESTS, rustdoc
  links, Windows interrupt caveats, test maps.

### 2.2 Behavior changes to sign off at merge

These are intentional, tested, and documented. They are not open questions.

| Change | Before | After |
|---|---|---|
| `spawn_platform` env | `&[(String, String)]`; empty meant inherit on Unix | `Option<&[(String, String)]>`: `None` inherits, `Some(&[])` is empty, `Some(entries)` is exact |
| `open_pty(None)` / `PtySession::spawn` with `winsize=None` | Unix kernel default 0×0 | 24×80 on all platforms |
| Python `spawn(env={})` | Inherited on Windows | Empty on Windows (matches Unix and `None` means inherit) |
| `PtyBackend::write_all` | Could spin/return `WriteZero` on a short write | Retries until all bytes are written |

Migration examples for the env contract:

```rust
// Inherit the parent environment.
spawn_platform(program, &args, None, winsize, cwd).await?;

// Explicit empty environment.
spawn_platform(program, &args, Some(&[]), winsize, cwd).await?;

// Exact environment: no inherited variables.
let env = [("PATH".to_string(), "/usr/bin:/bin".to_string())];
spawn_platform(program, &args, Some(&env), winsize, cwd).await?;
```

The session-level API does not change: `PtySession::spawn` inherits;
`PtySession::spawn_with_env` takes an exact environment set.

### 2.3 Explicitly out of scope

The seven deferred gaps in §4, the four hardening findings in §5, the Kitty
follow-ups in §5.5–§5.6, and the parser/interop follow-ups in §5.7–§5.9 are
**not** part of 0.10.0. Do not add them to PR #2; each gets its own release and
test plan. This keeps 0.10.0 a coherent unit and avoids mixing a breaking Rust
API cleanup into an unreleased feature branch.

---

## 3. Reference review

### 3.1 Sources inspected

| Project | What it provides | Relevant source |
|---|---|---|
| **WezTerm `pty`** (`portable-pty` 0.9.0) | Runtime-selectable PTY traits, `CommandBuilder`, `ChildKiller`, `ExitStatus`, Unix and ConPTY backends | [`pty/src`](https://github.com/wezterm/wezterm/tree/main/pty/src) |
| **WezTerm `term`** | Terminal emulator only; explicitly does not manage a PTY | [`term/src`](https://github.com/wezterm/wezterm/tree/main/term/src) |
| **Alacritty `alacritty_terminal`** | Evented PTY/child lifecycle, Unix SIGCHLD, Windows `ChildExitWatcher`, `drain_on_exit` | [`alacritty_terminal/src/tty`](https://github.com/alacritty/alacritty/tree/master/alacritty_terminal/src/tty) |
| **emux-pty** | Small combined `Pty: Read + Write` facade over Unix `forkpty` and ConPTY | [`crates/emux-pty`](https://github.com/IISweetHeartII/emux/tree/main/crates/emux-pty) |

### 3.2 Architecture lessons at a glance

| Concern | WezTerm `portable-pty` | Alacritty | emux-pty | Lesson for stitch-pty |
|---|---|---|---|---|
| **Transport shape** | Separate `MasterPty` / `SlavePty` / `Child` / `ChildKiller` traits | `EventedReadWrite` + `EventedPty` + child watcher | Combined `Pty: Read + Write` with `child_pid` / `is_alive` | Keep `PtySession` as a facade, but do not merge emulator and PTY. A granular `ChildKiller` is valuable. |
| **Raw handles** | `as_raw_fd() -> Option<RawFd>` on Unix; `as_raw_handle() -> Option<RawHandle>` on Windows | Raw handle only on the Windows child watcher | Unix `master_raw_fd()` only | Do not force one return type. Platform-gated accessors are acceptable and lower risk. |
| **Exit status** | One `ExitStatus { code, signal }`; `wait` returns it | `std::process::ExitStatus`; `ChildEvent::Exited(Option<ExitStatus>)` | `enum { Code(i32), Signal(i32) }` on Unix, only `Code` on Windows | One status type, no `pid`; remove `ProcessExit`. Keep a cached, repeatable wait. |
| **Programmatic signals** | `ChildKiller::kill` → `TerminateProcess` on Windows; no Ctrl+C | No signal API; input is user bytes | None | Do not promise ConPTY Ctrl+C. Prefer an explicit unsupported error. |
| **EOF / close** | `take_writer` drop sends EOF; Unix writer sends newline + `VEOF`; ConPTY `PseudoCon` closes in `Drop` | `ClosePseudoConsole` blocks until conout drained; `drain_on_exit`; child-exit watcher | ConPTY closes in `Drop`, no drain | Close must be order-aware and drain first; distinguish stdin EOF from output EOF. |
| **Environment** | `CommandBuilder` with `OsString`, `env`, `env_remove`, `env_clear`; base env starts inherited | Additive env map; Windows keys deduped case-insensitively; no empty-env API | Additive only; Windows uses `std::env::vars()` (panic on non-Unicode); no empty env | Our `Option` contract is correct; Windows duplicate keys use last-wins dedupe in 0.10.4. A future `OsString` builder remains. |
| **Child exit notification** | Blocking `wait`; async future spawns a waiter thread | `RegisterWaitForSingleObject` callback + event channel | 50 ms polling via `GetExitCodeProcess` | Use an event-driven Windows watcher; avoid polling and thread-per-poll. |
| **Terminal model** | `term` has no PTY; `Terminal::advance_bytes` + writer injection | `Pty` / event loop / `Term` are separate | PTY only | `TerminalSession` should be a thin coordinator, not an emulator-inside-PTY. |

### 3.3 What not to copy from emux-pty

emux-pty is useful as a small reference, but its implementation has avoidable
bugs:

- `UnixPty::is_alive()` calls `waitpid(WNOHANG)` and discards `Exited` /
  `Signaled`; a subsequent `wait()` can fail with `ECHILD` because `is_alive()`
  reaped the child. Our background reaper caches the status, which is the right
  model.
- Windows `build_env_block` starts with `std::env::vars()` and merges overrides.
  That is the exact non-Unicode panic we fixed, and it cannot express an empty
  environment.
- `ExitStatus` is `Code | Signal` on Unix but only `Code` on Windows, so
  cross-platform code cannot name the signal variant.
- `Pty: Read + Write` with `&mut self` makes concurrent read/write awkward.

Do not copy these patterns. Use WezTerm/Alacritty for API shape and lifecycle.

---

## 4. Deferred gaps

These gaps were identified during the PR #2 review and are intentionally left
for follow-up releases. They are listed here so they are not lost when the PR
description is archived. Reference lessons are folded into each item.

### 4.1 At a glance

| # | Gap | What the code does today | Revised follow-up | Bucket |
|---|---|---|---|---|
| 1 | `raw_handle()` has a platform-dependent return type (`RawFd` vs `*mut c_void`) — `src/platform.rs:130` | Callers need `#[cfg]`; `PtyBackend` is object-safe, but the return type is not nameable cross-platform | **Re-scoped:** keep documented platform-gated accessors; optionally add a `RawHandle` enum later. Do not force `AsHandle`; HPCON is not an owned kernel handle | Optional 0.11.0; prefer non-breaking |
| 2 | `ChildBackend::wait()` returns legacy `ProcessExit` — `src/platform.rs:76,169` | `wait_status()` is the additive bridge; `PtySession::wait` already returns `ExitStatus` | Remove `ProcessExit`; make `wait` return `Option<ExitStatus>`; no `pid` in the status; cache repeatable results | 0.11.0 (breaking) |
| 3 | Windows `interrupt()` does not reach the ConPTY child — `src/platform_windows.rs:265-273` | `signal(2)` maps to `GenerateConsoleCtrlEvent(CTRL_C_EVENT, 0)`, which targets the caller's console group; documented best-effort | **Default:** return an explicit “unsupported on ConPTY” error. Ctrl+Break is a separate opt-in spike with different semantics | 0.10.1 (bug/design) |
| 4 | No EOF on Windows child exit — `src/platform_windows.rs:322` closes the pseudo console only in `Drop` | Reads block/time out after `is_alive() == false` until the session is dropped; documented | Add a close-order-aware `close()`, `read_until_exit`, and an event-driven child watcher. Drain output before `ClosePseudoConsole`; distinguish stdin EOF from output EOF | 0.10.2 (bug/design) |
| 5 | `expect()` is Python-only — `python/stitch_pty/__init__.py:536` | Rust users must build matching on raw chunks | Add `ExpectPattern` (`Bytes` / `Str` / regex), a byte-oriented rolling buffer, `ExpectResult`; regex behind a feature | Feature minor after G6 |
| 6 | No emulator-owning session — `src/session.rs` deliberately has no `HistoryScreen` | Rust callers feed `read` output into their own screen | Add a thin `TerminalSession` wrapping `PtySession` + `HistoryScreen`; auto-feed chunks, resize both, expose screen/dirty rows. Do not merge emulator into `PtySession` | Feature minor |
| 7 | Python `terminate()` had its own 100 ms loop (pre-0.10.3) — `python/stitch_pty/__init__.py:190,443` | Uses `spawn_blocking` sleeps because tokio timers through the PyO3 bridge are unreliable on macOS; Rust `PtySession::terminate` uses tokio sleeps | Consolidated around `ArcChildBackend` + a shared escalation policy with injectable sleep; Python delegates once | 0.10.3 (implemented; release pending) |

### 4.2 G1 — `RawHandle` / `raw_handle()`

**Current behavior.** `PtyBackend::raw_handle` is `RawFd` on Unix and
`*mut c_void` on Windows (`src/platform.rs:130`, implementations at
`src/platform_unix.rs:274` and `src/platform_windows.rs:389`). Generic or
cross-platform callers must use `#[cfg]` at every call site, and the trait
method cannot be named without platform-specific code.

**Reference lesson.** WezTerm deliberately does not unify raw handles:

- `MasterPty::as_raw_fd() -> Option<RawFd>` is `#[cfg(unix)]`.
- `Child::as_raw_handle() -> Option<RawHandle>` is `#[cfg(windows)]`.
- The `Option` is meaningful: not every backend exposes a raw handle (serial
  PTYs in the same crate do not).
- Alacritty exposes a raw handle only on its Windows `ChildExitWatcher`; emux
  exposes only `master_raw_fd()` on Unix.

The Windows value is an opaque `HPCON`, not an owned kernel `HANDLE`. A
cross-platform `RawHandle` is therefore possible only as an enum/newtype with a
documented raw-pointer variant. `AsHandle` (borrowed ownership) is the wrong
trait; `AsRawFd` / `AsRawHandle` or explicit accessors fit.

**Revised work.**

- **Preferred:** keep `raw_handle` as a documented platform-gated accessor and
  add `as_raw_fd` / `as_raw_handle`-style helpers if callers need them. This is
  non-breaking and matches the reference crates.
- **Optional:** if a nameable type is required, add a `#[non_exhaustive]`
  `RawHandle` enum, e.g. `Fd(RawFd)` / `ConPty(*mut c_void)`, and return
  `Option<RawHandle>` if future backends may lack a handle.
- If the enum is added, implement `AsRawFd` on Unix and an explicit
  `as_raw_handle`/`as_ptr` accessor on Windows. Do not implement `AsHandle`.
- Decide and document `Send`/`Sync` and nullability. HPCON is a raw pointer; do
  not add an unchecked auto-trait without a safety contract.
- Update tests, docs, and examples on all three targets.

**Acceptance.**

- Downstream code can access the raw handle without hidden `#[cfg]` in a
  cross-platform function, **or** the platform-gated contract is explicitly
  documented as the supported API.
- `PtyBackend` remains object-safe.
- Cross-target clippy passes for Windows, Linux, and macOS.
- No `AsHandle` implementation for `HPCON`.

**Risk.** Low if we keep accessors. A unified enum is a breaking trait change
and should not be forced into 0.11.0 unless there is downstream demand.

---

### 4.3 G2 — `ChildBackend::wait()` should return `ExitStatus`

**Current behavior.** `ChildBackend::wait()` returns the legacy `ProcessExit`
(`src/platform.rs:76,169`); `wait_status()` is the additive bridge that converts
to the modern `ExitStatus`, and `PtySession::wait` already exposes
`Option<ExitStatus>`.

**Reference lesson.**

- WezTerm has exactly one public status type, `ExitStatus { code, signal }`;
  `Child::wait` returns it. There is no legacy status with a `pid`.
- Alacritty uses `std::process::ExitStatus` and reports it through
  `ChildEvent::Exited`.
- emux’s `enum ExitStatus { Code, Signal }` is clean on Unix, but its Windows
  variant is only `Code`; do not copy that platform asymmetry.

Our current `ExitStatus` is already close to WezTerm’s model. The problem is the
duplicate `ProcessExit` with `pid`, `exit_code: Option<i32>`, and
`signal: Option<i32>`.

**Work.**

- Change `ChildBackend::wait()` to return `Option<ExitStatus>`.
- Remove `ProcessExit` from the public API (or deprecate it for one minor if a
  softer migration is preferred).
- Remove the now-redundant `wait_status()` or keep it as a deprecated alias for
  one minor.
- Do not add `pid` to `ExitStatus`; callers already have `child.pid()`.
- Keep the cached status so repeated `wait()` calls return `Some` after exit.
- Prefer signal names (`SIGTERM`) over numeric `SIG15` strings when converting
  the legacy status; WezTerm uses `strsignal` for this.
- Update `PtySession`, `python_api`, examples, docs, and tests.

**Acceptance.**

- No `ProcessExit` in the public Rust API after the release.
- Repeated `wait()` calls remain idempotent (`Some` after exit).
- Unix signal and Windows exit-code behavior is preserved.
- Cross-target clippy and all tests pass.
- Python’s exit tuple/status mapping is unchanged or explicitly migrated.

**Risk.** Breaking change; schedule as the focused 0.11.0 release. If we also
change `ExitStatus` to an enum, do it in the same commit with a single migration
note.

---

### 4.4 G3 — Windows `interrupt()` semantics

**Before 0.10.1.** `signal(2)` mapped to
`GenerateConsoleCtrlEvent(CTRL_C_EVENT, 0)` (`src/platform_windows.rs:265-273`).
That targets the caller's console process group, not the ConPTY child, so the
interrupt may silently do nothing. The docs call it best-effort.

**Reference lesson.** No reference implements a reliable programmatic Ctrl+C for
ConPTY:

- WezTerm’s `ChildKiller::kill()` on Windows is `TerminateProcess`; its ConPTY
  spawn does not set `CREATE_NEW_PROCESS_GROUP`.
- Alacritty has no signal API; it writes user input bytes and closes the PTY.
- emux has no signal API.

A Ctrl+Break approach would be a different signal with different application
behavior, and none of these codebases validates it.

**Revised work.**

1. **Released in 0.10.1:** on Windows, `interrupt()`/`send_signal(2)` return
   `PtyErrorKind::Unsupported` (Python `NotImplementedError`). Do not silently
   succeed. Unix SIGINT is unchanged.
2. Update `PtySession::interrupt`, Python `interrupt()`, README Windows
   caveats, and rustdoc to match.
3. Keep Unix behavior unchanged: `SIGINT` to the child's process group.
4. **Optional spike, separate from 0.10.1:** try `CREATE_NEW_PROCESS_GROUP` +
   `CTRL_BREAK_EVENT` with `cmd`, `ping`, and a Rust child. If it works, expose
   it as an explicitly different “break” operation, not as Ctrl+C parity. If it
   does not, keep the unsupported error.

**Acceptance.**

- Windows integration test asserts the explicit error path, not a silent no-op.
- No caller can mistake a best-effort no-op for delivered Ctrl+C.
- Unix tests still pass.
- Python `interrupt()` raises/maps the same explicit error on Windows.
- If Ctrl+Break ships, it has its own name, docs, and tests.

**Risk.** Ctrl+Break is not Ctrl+C; some applications ignore it. The reference
review favors explicit unsupported over a speculative implementation.

---

### 4.5 G4 — Windows EOF and `close()`

**Current behavior.** `WinPtyBackend` closes the pseudo console in `Drop`
(`src/platform_windows.rs:322`). ConPTY can keep the output pipe open after the
child exits, so `read()`/`read_timeout()` may return `Err(Timeout)` repeatedly
while `is_alive()` is false, until the session is dropped.

**0.10.2 implementation.** `read_until_exit`, idempotent `close`, and
`shutdown_write` are implemented; the Windows child-exit watcher replaces the
50 ms poll. The patch is on `dev`; tag/publish waits for explicit confirmation.

**Reference lesson (the most important one).**

Alacritty’s `Conpty::drop` says:

> This will block until the conout pipe is drained. Will cause a deadlock if the
> conout pipe has already been dropped by this point.

Its Windows `Pty` also puts the backend first:

> Backend is required to be the first field, to ensure correct drop order.
> Dropping `conout` before `backend` will cause a deadlock (with Conpty).

Alacritty handles normal exit with:

- `ChildExitWatcher` built on `RegisterWaitForSingleObject`, not polling.
- `drain_on_exit`: when the child-exit event arrives, read the remaining output
  before exiting the event loop.
- Linux `EIO` treated as an expected PTY hangup; wait for the child-exit event
  instead of erroring.

WezTerm contributes the input/output distinction:

- `MasterPty::take_writer()` is separate from the reader; dropping the writer
  sends EOF to the child.
- Its Unix writer drop sends a newline plus the terminal `VEOF` character
  before closing, so a line-oriented reader sees EOF.
- `PseudoCon` closes ConPTY in `Drop`, but there is no explicit `close()`.

emux-pty closes ConPTY in `Drop` immediately, without draining; that is the
pattern to avoid.

**Revised work.**

1. **Child-exit notification (also H3):** add a Windows watcher using
   `RegisterWaitForSingleObject` (or a single blocking `wait` task) that wakes
   `wait()` immediately. Do not rely on 50 ms polling for close timing.
2. **`read_until_exit`:** add a helper that selects between reads and child
   exit, keeps reading while the child is alive, drains remaining output after
   exit, and returns `(output, Option<ExitStatus>)` (the status is `None` only
   if it was already reaped). This is the Rust equivalent of Alacritty’s
   `drain_on_exit`.
3. **`close()` / `shutdown()`:** make it idempotent and order-aware:
   - ensure a reader is active or drain the conout pipe;
   - close `HPCON` while the conout pipe is still alive;
   - then close/drop the input/output pipes and child handles;
   - keep `conpty_handle` before the pipe fields in `WinPtyBackend`; our current
     field order already matches Alacritty’s, so do not reorder it during a
     refactor;
   - if the close can block, run it on a blocking thread (`spawn_blocking` or a
     dedicated thread) and document that it may wait for output to drain.
4. **Define live-child behavior:** decide whether `close()` requires an exited
   child, kills a live child, or returns an error. Recommended: `read_until_exit`
   for the normal path and `close()` for explicit teardown; if the child is
   alive, `close()` should terminate it (matching Drop) and drain before closing.
5. **Separate stdin EOF from output EOF:** add `take_writer`/`shutdown_write`
   (or equivalent) that drops/sends EOF on the input side without closing the
   output/ConPTY. WezTerm’s writer-drop/VEOF behavior is the model.
6. **Drop safety:** do not call a potentially blocking
   `ClosePseudoConsole` from an async executor thread without a drain plan.
   At minimum, document the blocking behavior and test the unread-output case.
7. Consider exposing `close()` / `read_until_exit` on the Python wrapper.

**Acceptance.**

- Windows `cmd /C echo done`: complete output is returned and
  `read_until_exit` finishes without a timeout; a subsequent read returns EOF.
- Large unread output: `read_until_exit` captures all output; closing or
  dropping a session with pending output does not deadlock. Dropping without
  reading may discard output, but that limitation is documented.
- Slow reader: a reader slower than the child still gets all output.
- `close()` is idempotent; double close is safe.
- Stdin EOF and output EOF are independently testable.
- Unix integration tests still pass.
- A Windows child-exit watcher test observes exit without a 50 ms polling delay.

**Risk.** `ClosePseudoConsole` may block until conout is drained and may discard
buffered output. The current `Drop` already calls it, so this is a latent issue
in 0.10.0, not only a future `close()` concern. This is the main design risk in
the 0.10.x series. Test with large output, slow readers, and unread output
before committing to close timing.

---

### 4.6 G5 — Rust `expect()`

**Current behavior.** Python has `PtySession.expect()` and regex/multi-pattern
support; Rust callers must implement matching over raw `read` chunks.

**Reference lesson.** None of the three provides an `expect` API, so this gap is
not solved by copying a reference. The useful lesson is byte orientation:

- WezTerm’s escape parser and Alacritty’s `vte::Parser` process incremental byte
  chunks; neither requires complete lines or valid UTF-8.
- A matcher should therefore operate on bytes first, with a separate lossy
  string helper only for ergonomics.

**Work.**

- Define an `ExpectPattern` enum: bytes, UTF-8 string, and optional regex
  (behind a `regex` crate feature).
- Implement a rolling byte buffer so a match spanning reads is found; define
  the retained tail and maximum buffer size.
- Return `ExpectResult` with the matched pattern, match span/bytes, and the
  before/after buffer.
- Support timeout and `read_timeout`-style cancellation without losing buffered
  data.
- Document encoding policy (bytes-first; UTF-8 lossy only for string APIs).
- Look at `expectrl`/`rexpect` for pattern and timeout semantics if more prior
  art is needed; the four reviewed projects do not cover this.

**Acceptance.**

- Matches across chunk boundaries.
- Timeout returns the accumulated buffer and does not kill the session.
- Regex feature is optional and documented.
- Tests cover bytes, string, regex, multi-pattern, and timeout.
- No lossy conversion is required to match non-UTF-8 output.

**Risk.** API design and regex dependency weight; keep the default build free
of a new mandatory dependency.

---

### 4.7 G6 — `TerminalSession` with an owned emulator

**Current behavior.** `PtySession` intentionally does not own a `HistoryScreen`;
Rust callers must feed `read` output into the emulator and resize it separately.
Python has a wrapper that owns the screen.

**Reference lesson.**

- WezTerm’s `term` crate explicitly does not manage a PTY. `Terminal` takes a
  writer for input and consumes output through `advance_bytes`, with chunked
  parsing and sequence/dirty tracking.
- Alacritty separates `Pty`, the event loop, and `Term`.
- emux provides only the PTY.

This validates the current split: do not put the emulator inside the PTY
backend.

**Work.**

- Add `TerminalSession` owning `PtySession` + `HistoryScreen`.
- Auto-feed bytes into the screen on `read`/`read_timeout`; expose the screen,
  display, and styled viewport.
- Forward `resize` to both the PTY and the screen; initialize the screen at the
  PTY's winsize.
- Provide spawn helpers mirroring `PtySession`.
- Consider a writer hook for terminal responses (e.g. DSR/DA replies), following
  WezTerm’s `Terminal::new(..., writer)` model. If the current `HistoryScreen`
  does not generate responses, document that.
- Expose dirty rows/sequence information so a renderer does not redraw the
  whole screen.
- Add an example that reads a shell and renders the screen.

**Acceptance.**

- A caller can spawn, read, resize, and inspect the screen without manually
  constructing a `HistoryScreen`.
- No double-feeding when callers also use the raw `PtySession`.
- Resize keeps PTY and screen dimensions in sync.
- Tests cover basic output, resize/reflow, scrollback, and dirty-row access.
- `PtySession` remains emulator-free.

**Risk.** Ownership/borrowing around `Arc` backends and reader loops; screen
reflow performance for large outputs; response-writer feedback loops.

---

### 4.8 G7 — Python `terminate()` consolidation

**Pre-0.10.3 behavior.** Python `PtyChild.terminate` and `PtySession.terminate`
(`python/stitch_pty/__init__.py:190,443`) use `spawn_blocking` sleeps because
tokio timers through the PyO3 bridge were unreliable on macOS. Rust
`PtySession::terminate` uses tokio sleeps, so it cannot simply be called from
the Python wrapper without re-verifying that path.

**Reference lesson.**

- WezTerm separates `ChildKiller` from `Child`: `clone_killer()` produces an
  object that can signal from another thread while a different task is blocked
  in `wait`. Its Unix `kill` tries SIGHUP with a grace period before a hard
  kill.
- Alacritty’s `Drop` sends SIGHUP, unregisters SIGCHLD, and waits.
- Neither has a PyO3 timer problem; the lesson is the policy shape, not the
  runtime.

**Work.**

- Reproduce the macOS PyO3 timer issue on current CI/current dependency
  versions.
- If the Rust `PtySession::terminate` path is reliable through PyO3, delegate to
  it.
- Otherwise factor the escalation policy into a shared helper with an injectable
  sleep function: blocking sleep for Python, tokio sleep for Rust.
- Build the policy on a cloneable child handle (`ArcChildBackend` or an
  extended `ChildKiller` that includes signalling) so SIGTERM/kill are
  independent of `wait`; `ChildKiller::clone_killer` covers the hard-kill leg,
  and `ArcChildBackend::signal` covers SIGTERM. Add a bounded wait for the
  cached status.
- Preserve `grace_period`, SIGTERM → SIGKILL escalation, reaping, and
  `ProcessLookupError`/already-exited behavior.
- Add tests for already-exited children, SIGTERM-ignoring children, and
  repeated calls.

**Acceptance.**

- One terminate implementation/contract, or one shared policy with two thin
  adapters.
- macOS CI passes; no tokio timer panic/hang through PyO3.
- Existing Python tests pass unchanged, plus the new edge cases.
- Killing does not deadlock a concurrent `wait`.

**Risk.** macOS timer behavior and process-reaping races. If verification is
inconclusive, keep the current implementation and document why; do not trade a
working path for theoretical deduplication.

**0.10.3 implementation.** `src/terminate.rs` owns the SIGTERM → grace →
SIGKILL → two-second bounded-reap ladder. `PtySession::terminate` injects
`tokio::time::sleep`; the PyO3 `PtyChild::terminate` adapter injects
`spawn_blocking(std::thread::sleep)`. Both Python wrappers now await one inner
future instead of re-issuing terminate every 100 ms, so a slow grace period
cannot multiply SIGTERMs. Rust unit tests cover already-exited, graceful,
SIGTERM-ignoring, repeated, and injected-sleep paths; Python tests cover an
exited child, repeated live termination, and SIGKILL escalation on POSIX. The
remaining release gate is macOS CI plus the explicit tag/publish confirmation.

---

## 5. Additional findings and follow-ups

These were not in the original seven gaps. H1–H4 came from the reference code
review; K1a/K1b come from
[issue #1](https://github.com/opticsWolf/stitch-pty/issues/1) and the Kitty
keyboard protocol review; P1/D1/D2 come from the post-merge parser and
Rust/Python decoupling review. They are labeled separately to avoid renumbering
the gaps.

### 5.1 H1 — Environment key normalization and `OsString`

**Problem.** Our explicit env contract is `Some(&[(String, String)])`. On
Windows, environment keys are case-insensitive, but our block builder
(`src/platform_windows.rs:555`) writes entries in caller order without
deduplication. `PATH` plus `Path` can therefore produce an ambiguous block. On
Unix, duplicate keys are also possible. WezTerm and Alacritty both normalize or
deduplicate:

- WezTerm’s `CommandBuilder` stores `OsString` keys/values, starts from
  `vars_os()`, and maps Windows keys case-insensitively; `env_remove` and
  `env_clear` make intent explicit.
- Alacritty’s `convert_custom_env` deduplicates Windows keys with
  `to_ascii_uppercase()` before building the block.
- emux uses `std::env::vars()` and merges the parent environment, which is both
  the non-Unicode panic and an inability to express an empty environment.

**Work.**

- Define duplicate-key semantics for `Some(entries)`:
  - Unix: exact-key duplicate rejected or last-wins, documented and tested.
  - Windows: case-insensitive duplicate rejected or last-wins, documented and
    tested.
- Add a Windows test with `PATH` and `Path` that asserts the chosen behavior.
- Keep the `None` inheritance path on `vars_os()`; never use `std::env::vars()`.
- Consider a future `CommandBuilder`/`PtyCommand` with `OsString`, `env`,
  `env_remove`, `env_clear`, matching `std::process::Command` and
  `portable-pty`. That is a new API, not a 0.10.4 patch.
- Document that explicit `String` entries cannot represent non-Unicode values;
  a future builder can remove that limitation.

**Bucket.** 0.10.4 (dedup/normalization); builder is a future feature.

**Acceptance.**

- Duplicate `PATH`/`Path` behavior is documented and tested.
- Empty env remains exactly empty on both platforms.
- Inheritance still uses `vars_os()` and does not panic on non-Unicode entries.
- No regression in the existing env tests.

**0.10.4 implementation.** `platform::dedupe_env_exact` and
`dedupe_env_windows` centralize the policy: last occurrence wins, including its
key spelling/value; Windows uses `eq_ignore_ascii_case`, Unix uses exact keys.
Unix `PreparedCommand` and the Windows `build_env_block` both call the helper.
The Windows block keeps the `\0\0` empty-environment form and encodes UTF-16.
Python `spawn()` now forwards `env` as ordered pairs instead of a `HashMap`, so
the Windows last-wins result follows the mapping order. Tests cover exact and
case-insensitive duplicates, empty values, Unicode, the empty block, and a
real Windows child; inheritance remains on `vars_os()`/null.

---

### 5.2 H2 — `STARTF_USESTDHANDLES` and inherited stdio

**Problem.** Both WezTerm and Alacritty set `STARTF_USESTDHANDLES` with
invalid/null std handles before `CreateProcessW`. WezTerm’s comment explains
that without it, a spawned process can inherit the parent’s redirected stdio
handles (for example, a daemonized parent with a log file) instead of using the
ConPTY. Our `CreateProcessW` setup in `src/platform_windows.rs` zeroes
`STARTUPINFOEXW` and does not set this flag.

**Work.**

- Set `STARTUPINFOEXW.StartupInfo.dwFlags |= STARTF_USESTDHANDLES` and set
  `hStdInput`/`hStdOutput`/`hStdError` to `INVALID_HANDLE_VALUE`, following
  WezTerm (`pseudocon.rs:120-124`) or Alacritty (`conpty.rs:148`).
- Confirm the ConPTY attribute still supplies the child’s actual console
  handles.
- Add a Windows test where the parent has redirected stdout/stderr (or at least
  assert the startup flags in a unit test) and verify child output reaches the
  PTY, not the parent’s log.

**Bucket.** 0.10.5.

**Acceptance.**

- Child output is not written to the parent’s redirected stdio.
- ConPTY I/O still works for `cmd /C echo`.
- No change on Unix.

**0.10.5 implementation.** `STARTUP_FLAGS` combines `STARTF_USESHOWWINDOW` and
`STARTF_USESTDHANDLES`; all three `STARTUPINFO` std handles are
`INVALID_HANDLE_VALUE`. The handles are only inheritance sentinels, not live
handles, so no lifetime is required; `PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE`
still supplies the child console. `tests/redirected_stdio.rs` runs in its own
test process, redirects the parent’s stdout/stderr to a file, and verifies the
child marker reaches the PTY rather than the file.

---

### 5.3 H3 — Event-driven Windows child-exit watcher

**Problem.** `WinChildProcess::wait` polls `GetExitCodeProcess` every 50 ms
(`src/platform_windows.rs:241`), and `is_running()` polls separately. Alacritty
uses `RegisterWaitForSingleObject` + a callback and wakes the event loop; that
is precise and avoids a 50 ms close-timing race.

**Work.**

- Add a Windows child-exit watcher (callback or a single blocking waiter) that
  sends the exit status to `wait()` and any close orchestration.
- Cache the status for repeated `wait()`.
- Use the same signal for G4’s `read_until_exit` so it knows when the child has
  exited without polling.
- Keep the Unix background reaper; it already caches status.
- Do not copy WezTerm’s `Future` implementation that spawns a new waiter thread
  on every poll; Alacritty’s callback/channel design is better.

**Bucket.** Part of G4 / 0.10.2; file as its own issue for review.

**Acceptance.**

- `wait()` wakes promptly after child exit (no fixed 50 ms polling delay).
- Repeated waits return the cached status.
- No leaked waiter thread or handle across repeated waits.
- G4 close tests use the watcher, not a sleep.

---

### 5.4 H4 — ConPTY creation flags review

**Problem.** The three implementations differ:

- WezTerm: `PSEUDOCONSOLE_INHERIT_CURSOR | PSEUDOCONSOLE_RESIZE_QUIRK |
  PSEUDOCONSOLE_WIN32_INPUT_MODE`.
- Alacritty: `0`.
- stitch-pty: `PSEUDOCONSOLE_INHERIT_CURSOR` only.

`PSEUDOCONSOLE_WIN32_INPUT_MODE` changes the input format; setting it blindly
would break raw byte writers. `PSEUDOCONSOLE_RESIZE_QUIRK` is a conhost
workaround that may fix resize glitches.

**Work.**

- Add a focused spike/test matrix for resize, cursor inheritance, and raw input.
- Try `PSEUDOCONSOLE_RESIZE_QUIRK` only if a reproducible resize bug exists.
- Do not enable `PSEUDOCONSOLE_WIN32_INPUT_MODE` unless the input path is
  adapted to Win32 `INPUT_RECORD` mode.
- Document the chosen flags and why.

**Bucket.** Investigation; release only if a reproducible issue is found.

**Acceptance.**

- A documented flags decision exists.
- Resize and input tests cover the chosen combination.
- No speculative flag change without a failing test.

**0.10.5 investigation result.** Keep `PSEUDOCONSOLE_INHERIT_CURSOR` only.
No reproducible resize failure appeared in the existing Windows resize and raw
input tests, so `PSEUDOCONSOLE_RESIZE_QUIRK` (0x2) stays off.
`PSEUDOCONSOLE_WIN32_INPUT_MODE` (0x4) changes input to `INPUT_RECORD`s and
would break byte-oriented writers, so it is not enabled. The choice is
centralized in `CONPTY_FLAGS` and asserted by a Windows unit test.

---

### 5.5 K1a — Kitty parser correctness and mode exposure

**Problem.** The terminal parser has partial Kitty keyboard support, but it is
not correct enough to build on:

- `Screen::push_keyboard_mode` (`src/terminal/screen.rs:1116`) pushes the **new**
  mode and leaves `keyboard_mode` unchanged. Per spec, it must push the
  **current** flags and then set the new flags. The existing push/pop tests
  (`src/terminal/screen.rs:2251`) encode the wrong behavior:
  push 1/2/3 should leave current `3` and stack `[0, 1, 2]`, not stack
  `[1, 2, 3]` with current `0`.
- `Screen::report_keyboard_mode` (`screen.rs:1127`) sends
  `ESC[{mode};1;0 u`; the documented query reply is `ESC[?{flags}u`.
- The `CSI = flags ; mode u` set path (`screen.rs:1353`) does not match the
  documented form; the tests use `CSI 1=u` and `CSI 3=u`, not the spec sequence.
  Audit the parser's `=` handling and the `params[1] == 0` condition.
- The screen has one `keyboard_mode_stack`; the spec requires independent main
  and alternate-screen stacks. The current alt-screen transition does not
  preserve or swap them.
- `keyboard_mode` is a raw `Screen` field, not exposed by `TerminalState` or the
  Python `PtySession`, so a frontend cannot ask which encoding to use.

This is a parser bug-fix item, not the encoder feature. It should land before
K1b.

**Work.**

- Implement spec push/pop semantics: `push` saves the current flags then sets
  the requested flags; `pop n` restores the last saved flags and resets to zero
  when the stack is exhausted.
- Maintain separate main-screen and alternate-screen flag stacks; preserve them
  across `?1049h`/`?1049l` and clear both on RIS/reset.
- Bound the stack (drop-oldest or reject) to match the protocol's DoS guidance.
- Fix the query response to `CSI ? flags u` and add a direct response test.
- Implement/verify `CSI = flags ; mode u` with mode 1=replace, 2=union,
  3=difference; add direct parser tests using the documented sequence.
- Add `Screen::keyboard_mode()`, `HistoryScreen`/`TerminalState` exposure, and a
  Python `TerminalState.keyboard_mode` property.
- Fix the existing push/pop tests to assert the correct stack contents and
  current mode.

**Bucket.** 0.10.7 (parser correctness + accessor, after P1).

**Acceptance.**

- `push 1, push 2, push 3` leaves current `3`, stack `[0, 1, 2]`; popping
  restores `2`, `1`, `0` in order.
- `CSI ? u` produces `CSI ? flags u`.
- `CSI = 1 ; 1 u`, `CSI = 1 ; 2 u`, and `CSI = 1 ; 3 u` set/union/difference
  correctly in direct parser tests.
- Main and alternate screen stacks are independent and survive alt-screen
  transitions.
- `keyboard_mode` is readable from Rust and Python.
- No key encoding is added in this item; K1b builds on the exposed flags.

**Risk.** Fixing push/pop changes behavior that existing tests assert. Those
assertions are wrong per the spec and must be replaced, not preserved.

---

### 5.6 K1b — Kitty key encoder (issue #1)

**Problem.** There is no key-event encoder. The PySide6 example's
`_key_to_bytes()` (`examples/terminal_emulator.py`) is VT100-only and returns
`b"\r"` for every Enter variant, so Shift+Enter is indistinguishable from
Enter even if the child requested Kitty mode.

**Reference lesson.** The blog and the official Kitty spec define the encoding:

- The child application requests flags with `CSI > flags u`; the terminal
  tracks them (K1a) and the frontend encodes keys accordingly.
- Flag 1 (disambiguate) is enough for **modified** Enter: Shift+Enter becomes
  `CSI 13;2u`.
- Unmodified Enter/Tab/Backspace remain legacy (`0x0D`, `0x09`, `0x7F`) under
  flag 1 by design, so a crashed app can still be reset from a shell. Do not
  blindly CSI-encode them under flag 1.
- Flag 8 (report all keys) encodes all keys, including unmodified
  Enter/Tab/Backspace; flag 2 adds repeat/release event types; flags 4 and 16
  add alternate keys and associated text. Start with flag 1 and add the others
  explicitly.

**Work.**

- Add `KeyboardFlags` (bitflags) with the five protocol bits and a `KeyEvent`
  carrying the unshifted key code, modifiers, event kind, text, shifted key, and
  base-layout key.
- Add a `KeyCode` / `Modifiers` / `KeyEventKind` model; keep it UI-toolkit
  agnostic so Qt, egui, and Rust frontends can populate it.
- Add `encode_key(event, flags) -> bytes` with a legacy fallback when flags are
  zero. Under flag 1, apply the Enter/Tab/Backspace exceptions; under flag 8,
  encode all keys as CSI u.
- Support event types (`;mods:type`), alternate keys (`code:shifted:base`), and
  associated text (`;text`) only when the corresponding flags are active.
- Add a Python `TerminalState.encode_key(...)` / `KeyboardEncoder` surface and a
  `PtySession.encode_key(...)` convenience that reads the session's current
  flags; update the PySide6 example to use it.
- Keep `PtySession` transport-only in Rust: the encoder belongs with the
  terminal state/frontend, not the PTY byte stream. Python `PtySession` already
  owns `_terminal`, so it can delegate.

**Bucket.** Next feature minor (ahead of G6/G5), because it unblocks the
reported Shift+Enter use case.

**Acceptance.**

- Shift+Enter with flag 1 emits `CSI 13;2u`; plain Enter/Tab/Backspace remain
  legacy.
- Ctrl+I vs Tab, Escape vs Alt+key, Ctrl+Shift, Alt+Enter, and event types are
  covered by fixed test vectors.
- Flag 8 emits CSI u for all keys; flags 2/4/16 are opt-in and tested.
- Falling back to legacy when the child did not request Kitty mode works.
- Python `PtySession.encode_key` drives the PySide6 example without a local
  `_key_to_bytes` implementation.
- The encoder never invents activation; it consumes the flags recorded by the
  screen parser.

**Risk.** API design and scrollback/terminal-state coupling; the flag semantics
are subtle (especially Enter/Tab/Backspace and event types). Use official Kitty
`kitten show-key` output or `key_encoding.c` as test vectors before shipping.

---

### 5.7 P1 — Parser state is lost across `feed()` chunks

**Problem.** This is a correctness bug independent of PyO3.

- `HistoryScreen::feed` (`src/terminal/history.rs:713`) creates a new
  `AnsiParser::new()` on every call and then parses the chunk. The parser’s
  `state`, CSI parameter buffer, OSC buffer, and partial UTF-8 are therefore
  reset for every `feed()`.
- `TerminalState` stores a `parser: Parser` field (`src/terminal_api.rs:15-19`),
  but `TerminalState::feed` (`src/terminal_api.rs:31-33`) calls
  `self.screen.feed(data)`; the stored parser is only used by `reset()`.
- Reproduction with the current extension:
  - `feed(b"\x1b[31mX")` renders a red `X`.
  - `feed(b"\x1b[")` then `feed(b"31mX")` renders the literal text `31mX`.
- This affects Python `PtySession.read()`, which feeds each read chunk through
  `_terminal.feed(data)`, and any Rust caller using `HistoryScreen::feed` with
  fragmented PTY reads.
- The existing chunked property tests mostly assert buffer shape, not rendered
  content, colors, or cursor state, so they do not catch this.

**Work.**

- Make parser ownership single and persistent:
  - preferred: `HistoryScreen` owns the `AnsiParser` and uses it in `feed()`;
    `reset()` resets it; or
  - use `TerminalState`’s existing `Parser` by exposing a
    `HistoryScreen::feed_with_parser`/inner-screen path that preserves the
    parser and then captures scrollback.
- Remove the duplicate/dead parser field once ownership is settled.
- Ensure scrollback capture and event emission still happen after each chunk.
- Add tests that feed split CSI, OSC, DCS, alt-screen, and UTF-8 sequences one
  byte/small chunk at a time and assert visible text, styles, cursor, and
  scrollback content — not just buffer shape.
- Add a Python test that a `PtySession.read()` sequence split across small reads
  produces the same terminal state as one feed.

**Bucket.** 0.10.6 (critical parser fix; before K1a/K1b).

**Acceptance.**

- `feed(b"\x1b[")` + `feed(b"31mX")` produces the same styled result as
  `feed(b"\x1b[31mX")`.
- Split UTF-8, OSC, DCS, and alt-screen sequences survive chunk boundaries.
- `HistoryScreen::feed` and `TerminalState.feed` share one parser-state model.
- `reset()` clears the persistent parser state.
- No PyO3 types are involved in the Rust fix.

**Risk.** Parser state transitions and partial UTF-8 are easy to get subtly
wrong; test against the existing parser property suite and add content-level
assertions.

**0.10.6 implementation.** `HistoryScreen` owns the sole `Parser` and reuses it
in `feed()`; `reset()` resets it. `TerminalState` no longer keeps a duplicate
parser and simply delegates `feed()`/`reset()` to the screen. The parser
property helper now feeds one persistent parser through its cut points, and
content-level tests cover split SGR/cursor, OSC title, DCS, UTF-8, alt-screen,
and reset. A Python test reads one byte at a time from a child that emits
`ESC[31mX` and asserts the rendered `X` is not literal `31mX`.

---

### 5.8 D1 — Rust interop surface for external PTY/terminal crates

**Problem.** The Rust core is usable without PyO3, but external Rust crates
cannot compose it as freely as they should:

- `PtySession` fields are private and there is no `from_parts`; a custom
  `PtyBackend`/`ChildBackend` implementation cannot be wrapped in a
  `PtySession`. `into_parts()` is one-way.
- `open_pty_platform` returns only the master. On Unix it closes the slave
  (`src/platform_unix.rs:629-644`); on Windows it creates a ConPTY without a
  child. Another crate cannot use it to attach its own spawner. `spawn_platform`
  is the only native spawn path.
- There is no `AsyncRead`/`AsyncWrite` implementation and no `reader()`/
  `writer()` handle; external crates must write their own read/write loops over
  `PtySession::{read, write}`.
- The public docs and tests do not show a Rust crate integrating an external
  terminal emulator or a custom PTY backend.

**Work.**

- Add `PtySession::from_parts(pty: ArcPtyBackend, child: ArcChildBackend)`
  (name/ownership to be decided) and document kill-on-drop and handle-sharing
  semantics.
- Provide a public PTY-pair/open API that retains the slave for external
  spawners, or explicitly document `spawn_platform` as the only supported
  route and add a supported `spawn_with`/pair variant. Do not leave
  `open_pty_platform` looking like a general external-spawn API when it closes
  the slave.
- Add an async integration surface: implement `AsyncRead`/`AsyncWrite` where
  possible, or expose `reader()`/`writer()` handles that own the appropriate
  Arc/Mutex plumbing. Keep `PtySession` methods as the low-level contract.
- Add a compile-checked Rust example that feeds `PtySession` bytes into an
  external emulator crate (for example `alacritty_terminal`, `wezterm-term`,
  or `vt100`) and sends input back.
- Keep `PtySession` emulator-free; do not fold `HistoryScreen` into this item.

**Bucket.** Next feature minor after K1b (with D2).

**Acceptance.**

- A custom `PtyBackend`/`ChildBackend` can be wrapped in `PtySession` without
  private fields or unsafe tricks.
- An external crate can attach its own spawner through a public API, or the
  limitation is explicit and tested.
- An external reader/writer can integrate without reimplementing session
  locking; the chosen `AsyncRead`/handle design is documented.
- At least one external-emulator integration example compiles in CI without
  the `python` feature.

**Risk.** Ownership/lifetime design around `Arc` backends and child kill-on-drop;
`from_parts` can create mismatched PTY/child pairs unless the contract is clear.

---

### 5.9 D2 — Rust/Python parity and no-PyO3 CI

**Problem.** PyO3 is optional, but the repository does not continuously prove
that the Rust core stays PyO3-free, and the two surfaces have drifted:

- CI runs `cargo test` with default features; there is no `--no-default-features`
  job. A future import or feature leak could silently make the core depend on
  PyO3.
- Python `PtySession` owns a `TerminalState` and exposes `raw_output`,
  `expect`, `scrollback`, and terminal properties; Rust `PtySession` owns only
  PTY + child. `expect` is tracked as G5, the owned emulator as G6, but
  `raw_output` and the rest are not classified.
- `TerminalState` is a PyO3 adapter; Rust callers use `HistoryScreen`. There is
  no parity table or test ensuring the adapter delegates correctly (P1 is one
  such bug).
- `Cargo.toml` still describes the crate as “for Python” even though the Rust
  core is independently usable.

**Work.**

- Add CI gates:
  - `cargo check --no-default-features --lib`
  - `cargo test --no-default-features --lib`
  - `cargo check --no-default-features --all-targets`
  - a dependency assertion that `pyo3` is absent from the default-feature tree
    (for example `cargo tree -e normal -i pyo3` must fail or be checked).
- Add a Rust/Python public-API parity table:
  - `session::PtySession` vs Python `PtySession`
  - `HistoryScreen` vs Python `TerminalState`
  - classify intentional Python-only surface (`raw_output`, `expect`, owned
    terminal) versus missing Rust surface (G5, G6, D1).
- Add parity tests for identical byte streams: Rust `HistoryScreen` and Python
  `TerminalState` must produce the same display/styles/events.
- Update README/Cargo metadata to describe the Rust crate as usable without
  Python, with the `python` feature as an optional binding layer.
- Decide whether `raw_output` is a Rust feature, Python-only convenience, or
  removed from the parity promise.

**Bucket.** D1/D2 together in the next feature minor; the no-PyO3 CI gate can
land earlier as a patch if desired.

**Acceptance.**

- CI proves the library and tests build with `--no-default-features`.
- No PyO3 symbol/type is reachable from the Rust core.
- The parity table is checked in and intentional differences are documented.
- Rust `HistoryScreen` and Python `TerminalState` agree on a fixed corpus of
  chunked ANSI/OSC/UTF-8 streams.
- `Cargo.toml`/README no longer imply Python is required.

**Risk.** CI time and dependency-tree assertions can be brittle; pin the
commands and keep them separate from the Python-wheel jobs.

---

## 6. Sequencing and version buckets

The exact version numbers for feature work follow the repository rule **one
version = one commit** and the bump table in `IMPROVEMENT_PLAN.md` (new feature
or major refactor = minor). The buckets below are the proposed order; adjust
the number, not the order, when scheduling.

| Order | Version | Work | Why this bucket |
|---|---|---|---|
| 0 | **0.10.0** | PR #2 merged; version/changelog commit `e6f10d2`; tag `v0.10.0` at `e620cc8`; PyPI + crates.io published | Released |
| 1 | **0.10.1** | G3 — Windows interrupt: explicit unsupported error (**released**) | Bug/behavior fix; no new API surface |
| 2 | **0.10.2** | G4 + H3 — close/read-until-exit, drain, child watcher (**implemented; release pending confirmation**) | Incremental fix on the just-landed `PtySession`; pairs with G6 |
| 3 | **0.10.3** | G7 — shared terminate policy (**implemented; release pending confirmation**) | Tech debt; behavior-preserving, macOS CI verification |
| 4 | **0.10.4** | H1 — env key dedup/normalization (**implemented; release pending confirmation**) | Windows correctness fix; contract already shipped in 0.10.0 |
| 5 | **0.10.5** | H2 — `STARTF_USESTDHANDLES`; H4 — flags decision (**implemented; release pending confirmation**) | Windows spawn hardening; no speculative flag change |
| 6 | **0.10.6** | P1 — persistent parser state across `feed()` chunks (**implemented; release pending confirmation**) | Critical correctness fix; prerequisite for K1a/K1b |
| 7 | **0.10.7** | K1a — Kitty parser correctness + mode exposure | Parser bug fix; builds on P1 |
| 8 | **0.11.0** | G2 — remove `ProcessExit`; G1 only if a unified `RawHandle` is actually needed | Breaking Rust API cleanup; one migration |
| 9 | **Next feature minor** | K1b — Kitty key encoder (issue #1) | New API surface; unblocks Shift+Enter and the PySide6 use case |
| 10 | **Following feature minor** | D1 + D2 — Rust interop surface + no-PyO3/parity gates | New API surface; decouples external Rust crates from the Python wrapper |
| 11 | **Following feature minor** | G6 — `TerminalSession` | New API surface; do this before G5 because it is the multiplexer use case |
| 12 | **Following feature minor** | G5 — Rust `expect()` | New API surface; larger design surface |

Notes:

- **G1 is re-scoped.** The reference-backed default is to keep platform-gated
  accessors and not force a unified type. If the enum is added, it can ride with
  G2; otherwise G1 closes as documentation/tests and 0.11.0 is just G2.
- **G3 cannot fail into a no-op.** If Ctrl+Break is not reliable, 0.10.1 is
  still a valid patch: it ships the explicit unsupported error and docs.
- **G4 must not simply add `close()`.** If draining proves too risky, split it:
  ship `read_until_exit` first, then a separate `close()` design. The child
  watcher (H3) can land with either.
- **H4 is not a release by itself.** It is a flag investigation; only promote it
  to a patch if a reproducible resize/input bug is found.
- **P1 is a prerequisite for K1a/K1b.** A persistent parser state is required
  before Kitty push/pop and split-sequence tests are meaningful. It also fixes
  Python `PtySession.read()` for fragmented reads.
- **K1a is a prerequisite for K1b.** Fix the push/pop/query/alt-stack semantics
  and expose `keyboard_mode` before writing the encoder. Do not add the encoder
  to `PtySession`; it belongs with the terminal state/frontend.
- **K1b is the next feature minor.** It directly addresses issue #1 and the
  Shift+Enter use case.
- **D1 + D2 follow K1b.** They make the Rust core composable by other Rust
  crates and add the no-PyO3/parity CI gate. G6/G5 follow.

### 6.1 Release discipline

- Do not fold G1–G7, H1–H4, K1a/K1b, or P1/D1/D2 into this PR/release.
- One release = one commit = one push to `dev`.
- Fix forward within the same version if a gate fails; do not push a red bump.
- Do not merge `dev` → `main`, tag, or publish without explicit maintainer
  confirmation (`RELEASING.md`).
- Every follow-up needs a CHANGELOG entry and docs updates in the same version
  commit.

---

## 7. Issue-filing checklist

The gaps now live in this plan and the issue tracker. All 15 remaining
follow-ups are filed as #3–#17 and linked below; issue #1 remains the Kitty
umbrella. Checkboxes track implementation, not filing.

**Original gaps**

- [x] **G3** — `fix(windows): make interrupt() fail explicitly on ConPTY` (released in 0.10.1)
- [x] **G4** — `fix(windows): add close()/read_until_exit with drain and close-order safety` (implemented in 0.10.2; release pending) ([#5](https://github.com/opticsWolf/stitch-pty/issues/5))
- [ ] **G2** — `refactor(rust)!: remove ProcessExit and return ExitStatus from wait` ([#4](https://github.com/opticsWolf/stitch-pty/issues/4))
- [ ] **G1** — `docs(rust): document raw_handle platform contract; optional RawHandle enum` ([#3](https://github.com/opticsWolf/stitch-pty/issues/3))
- [ ] **G6** — `feat(rust): add TerminalSession with an owned HistoryScreen` ([#8](https://github.com/opticsWolf/stitch-pty/issues/8))
- [ ] **G5** — `feat(rust): port expect() pattern matching` ([#7](https://github.com/opticsWolf/stitch-pty/issues/7))
- [x] **G7** — `refactor(python): consolidate terminate() with Rust PtySession` (implemented in 0.10.3; release pending) ([#9](https://github.com/opticsWolf/stitch-pty/issues/9))

**Reference-review findings**

- [x] **H1** — `fix(windows): normalize/deduplicate explicit environment keys` (implemented in 0.10.4; release pending) ([#10](https://github.com/opticsWolf/stitch-pty/issues/10))
- [x] **H2** — `fix(windows): set STARTF_USESTDHANDLES for ConPTY spawn` (implemented in 0.10.5; release pending) ([#11](https://github.com/opticsWolf/stitch-pty/issues/11))
- [x] **H3** — `perf(windows): event-driven child-exit watcher` (implemented in 0.10.2; release pending) ([#6](https://github.com/opticsWolf/stitch-pty/issues/6))
- [x] **H4** — `spike(windows): review ConPTY creation flags` (investigation complete in 0.10.5; no flag change) ([#12](https://github.com/opticsWolf/stitch-pty/issues/12))

**Kitty keyboard follow-up (issue #1)**

- [ ] **K1a** — `fix(terminal): correct Kitty push/pop/query, alt-screen stacks, and expose keyboard_mode` ([#13](https://github.com/opticsWolf/stitch-pty/issues/13))
- [ ] **K1b** — `feat(terminal): add a Kitty key encoder and Python PtySession.encode_key` ([#14](https://github.com/opticsWolf/stitch-pty/issues/14))
- [ ] Keep [issue #1](https://github.com/opticsWolf/stitch-pty/issues/1) as the
      umbrella for the user report and link K1a/K1b back to it. Do not close it
      until K1b lands and the PySide6 example uses the encoder.

**Post-merge parser / decoupling findings**

- [x] **P1** — `fix(terminal): persist parser state across feed() chunks` (implemented in 0.10.6; release pending) ([#15](https://github.com/opticsWolf/stitch-pty/issues/15))
- [ ] **D1** — `feat(rust): add PtySession::from_parts and reader/writer interop` ([#16](https://github.com/opticsWolf/stitch-pty/issues/16))
- [ ] **D2** — `test(ci): enforce no-PyO3 core build and Rust/Python parity` ([#17](https://github.com/opticsWolf/stitch-pty/issues/17))

Suggested labels (create if missing): `bug`, `windows`, `rust`, `python`,
`breaking`, `feature`, `tech-debt`, `macos`, `spike`, `keyboard`, `protocol`,
`parser`, `interop`, `ci`.

Add acceptance criteria from §4–§5.9 to each issue; do not open an issue that
only says “fix this”.

---

## 8. 0.10.0 release gate

The release gate was completed on `dev`. The version-bump commit is `e6f10d2`
and the tag is `v0.10.0` at the post-docs commit `e620cc8`. The release commit
contains only the version bump and CHANGELOG update; `99f7af7` is the separate
Windows ConPTY test-stability fix.

### 8.1 Version and changelog

- [x] `Cargo.toml` → `version = "0.10.0"`
- [x] `pyproject.toml` → `version = "0.10.0"`
- [x] `python/stitch_pty/__init__.py` → `__version__ = "0.10.0"`
- [x] `CHANGELOG.md`: added a
      `[0.10.0 — Rust PtySession + explicit environment contract]` heading
      below `[Unreleased]`, kept `[Unreleased]` for the next cycle, added
      compare links, and confirmed the env/winsize/EOF/write_all entries
- [x] The three version fields agree in the release commit (`0.10.0`;
      `Cargo.lock` and `uv.lock` updated too)

### 8.2 Local gates

- [x] `cargo fmt --all --check`
- [x] `cargo clippy --all-targets -- -D warnings`
- [x] `cargo clippy --all-targets --target x86_64-unknown-linux-gnu -- -D warnings`
- [x] `cargo clippy --all-targets --target aarch64-apple-darwin -- -D warnings`
- [x] `cargo test` — 401 unit + 11 integration + 1 doc test passed
- [x] `cargo check --features python`
- [x] `cargo doc --no-deps` — one pre-existing rustdoc warning; command passed
- [x] `ruff check python/ tests/`
- [x] `uvx maturin develop --release` — built and installed 0.10.0
- [x] `uv run python -m pytest tests/ -q` — 195 passed
- [x] README Rust example compiles — checked in a temporary path-dependency crate
- [x] **Windows unread-output Drop smoke test** — passed in ~313 µs with the
      child alive and output unread; no G4 close/drain guard needed

### 8.3 CI gate

- [x] All 6 test jobs pass: ubuntu/macos/windows × py3.12/3.13
- [x] All 4 wheel builds pass: macos universal2, ubuntu x86_64/aarch64,
      windows x86_64
- [x] Publish jobs remain skipped until the explicit release go-ahead
- [x] CI runs on the exact commit that carries the version bump
      (`43577417909` on `e6f10d2`)

CI run: <https://github.com/opticsWolf/stitch-pty/actions/runs/43577417909>

An earlier docs-only run on `f6d73a2` exposed a Windows py3.13 flake in
`read_timeout_returns_timeout_without_killing_the_session`: ConPTY setup
output arrived after the test's first quiet read. `99f7af7` waits for a
sustained quiet window; the release commit includes that fix.

### 8.4 Post-release (maintainer-authorized)

Tag and registry publication are complete:

- [x] `cargo publish` — crates.io 0.10.0 published
- [x] `uvx maturin build --release`; upload `dist/*` to PyPI — PyPI 0.10.0
      published (4 files)
- [x] Tag `v0.10.0` at `e620cc8`
- [x] Create the GitHub release from the CHANGELOG entry
- [ ] Bump Kilim's `stitch-pty` dependency
- [ ] Announce the breaking low-level env signature and the Windows EOF
      caveat

Tag CI: <https://github.com/opticsWolf/stitch-pty/actions/runs/43579326214> —
all 6 test jobs, all 4 wheels, PyPI, and crates.io green. The first tag run hit
a stale `CARGO_REGISTRY_TOKEN`; after refresh, the rerun published cleanly.

### 8.5 0.10.1 patch release (G3)

- [x] Version/changelog/docs in `97e03ab`
- [x] Local gates: `fmt`, `clippy` (host + Linux/macOS), `cargo test`
      (402 unit + 12 integration + 1 doc), `--no-default-features` (402),
      `check --features python`, `doc`, `ruff`, `mypy`, `maturin develop --release`
      (0.10.1), `pytest` (195)
- [x] Tag `v0.10.1` at `97e03ab`
- [x] GitHub release <https://github.com/opticsWolf/stitch-pty/releases/tag/v0.10.1>
- [x] PyPI 0.10.1 (4 files) and crates.io 0.10.1 published

Tag CI: <https://github.com/opticsWolf/stitch-pty/actions/runs/43581964572> —
all 6 test jobs, all 4 wheels, PyPI, and crates.io green.

### 8.6 0.10.2 (G4 + H3) — implementation complete, release pending

- [x] Version/changelog/docs for 0.10.2
- [x] Local gates: `fmt`, `clippy` (host + Linux/macOS), `cargo test`
      (402 unit + 17 integration [Windows; 19 Unix] + 1 doc),
      `--no-default-features` (402), `check --features python`, `doc`, `ruff`,
      `mypy`, `maturin develop --release` (0.10.2), `pytest` (195)
- [x] CI on the exact 0.10.2 commit (`3b3abb2`): run
      [`43585964604`](https://github.com/opticsWolf/stitch-pty/actions/runs/43585964604) — 6 test jobs,
      4 wheel builds green; publish jobs skipped
- [ ] Tag `v0.10.2`; publish PyPI/crates; create the GitHub release

No tag or registry publish has been performed. Do not run the final step until
the maintainer confirms the release.

### 8.7 0.10.3 (G7) — implementation complete, release pending

- [x] Version/changelog/docs for 0.10.3
- [x] Local gates: `fmt`, `clippy` (host + Linux/macOS), `cargo test`
      (407 unit + 17 integration [Windows; 19 Unix] + 1 doc),
      `--no-default-features` (407), `check --features python`, `doc`, `ruff`,
      `mypy`, `maturin develop --release` (0.10.3), `pytest` (197 passed,
      1 skipped on Windows)
- [ ] CI on the exact 0.10.3 commit (run follows this implementation commit)
- [ ] Tag `v0.10.3`; publish PyPI/crates; create the GitHub release

No 0.10.3 tag or registry publish has been performed. 0.10.2 also remains
untagged; its release gate is §8.6. The 0.10.3 release waits for explicit
confirmation.

### 8.8 0.10.4 (H1) — implementation complete, release pending

- [x] Version/changelog/docs for 0.10.4
- [x] Local gates: `fmt`, `clippy` (host + Linux/macOS), `cargo test`
      (412 unit + 18 integration [Windows; 19 Unix] + 1 doc),
      `--no-default-features` (412), `check --features python`, `doc`, `ruff`,
      `mypy`, `maturin develop --release` (0.10.4), `pytest` (198 passed,
      1 skipped)
- [ ] CI on the exact 0.10.4 commit
- [ ] Tag `v0.10.4`; publish PyPI/crates; create the GitHub release

No 0.10.4 tag or registry publish has been performed. 0.10.2 and 0.10.3 also
remain untagged; their release gates are §8.6 and §8.7. The 0.10.4 release
waits for explicit confirmation.

### 8.9 0.10.5 (H2 + H4) — implementation complete, release pending

- [x] Version/changelog/docs for 0.10.5
- [x] Local gates: `fmt`, `clippy` (host + Linux/macOS), `cargo test`
      (413 unit + 19 integration [Windows; 19 Unix] + 1 doc),
      `--no-default-features` (413), `check --features python`, `doc`, `ruff`,
      `mypy`, `maturin develop --release` (0.10.5), `pytest` (198 passed,
      1 skipped)
- [ ] CI on the exact 0.10.5 commit
- [ ] Tag `v0.10.5`; publish PyPI/crates; create the GitHub release

No 0.10.5 tag or registry publish has been performed. 0.10.2 through 0.10.4
also remain untagged; their gates are §8.6–§8.8. The 0.10.5 release waits for
explicit confirmation.

### 8.10 0.10.6 (P1) — implementation complete, release pending

- [x] Version/changelog/docs for 0.10.6
- [x] Local gates: `fmt`, `clippy` (host + Linux/macOS), `cargo test`
      (419 unit + 19 integration [Windows; 19 Unix] + 1 doc),
      `--no-default-features` (419), `check --features python`, `doc`, `ruff`,
      `mypy`, `maturin develop --release` (0.10.6), `pytest` (199 passed,
      1 skipped)
- [ ] CI on the exact 0.10.6 commit
- [ ] Tag `v0.10.6`; publish PyPI/crates; create the GitHub release

No 0.10.6 tag or registry publish has been performed. 0.10.2 through 0.10.5
also remain untagged; their gates are §8.6–§8.9. The 0.10.6 release waits for
explicit confirmation.

---

## 9. Open decisions / risks

| Topic | Decision needed | Owner / gate |
|---|---|---|
| G1 unified handle | Do we need a nameable `RawHandle` at all, or are platform-gated accessors enough? | 0.11.0 design; default is accessors |
| G2 status shape | Keep `ExitStatus { code, signal }` or switch to `Code`/`Signal` enum in the same breaking release? | 0.11.0; one migration |
| G3 Windows interrupt | Is Ctrl+Break acceptable, or is explicit “unsupported” the contract? | Answered: explicit unsupported released in 0.10.1; Ctrl+Break remains an optional separate spike |
| G4 latent Drop | Does the current 0.10.0 `Drop` hang with unread output? | Answered: no; release smoke passed in ~313 µs with unread output, so no guard is needed |
| G4 close semantics | Does `close()` require an exited child, kill a live child, or error? How is drain guaranteed? | Answered: live child is terminated; drain is bounded; implemented in 0.10.2 (pending release) |
| G4 blocking | Is `ClosePseudoConsole` ever called from the async executor, or always from a blocking thread? | Answered: `close()` uses `spawn_blocking`; implemented in 0.10.2 (pending release) |
| G7 macOS timers | Is the Rust terminate path safe through PyO3 on current macOS? | Answered: shared `terminate_child` policy; PyO3 adapter injects blocking sleeps and Python no longer polls. macOS CI verification pending the 0.10.3 release |
| H1 duplicate keys | Reject or last-wins for exact/case-insensitive duplicates? | Answered: last occurrence wins; Windows compares ASCII case-insensitively, Unix exact keys. Python preserves mapping order; implemented in 0.10.4 (pending release) |
| H2 std handles | Does setting `STARTF_USESTDHANDLES` change any current output path? | Answered: child output still reaches ConPTY; redirected-parent test passes. Implemented in 0.10.5 (pending release) |
| H3 watcher | Replace the 50 ms exit poll with an event-driven watcher? | Answered: watcher implemented in 0.10.2 (pending release) |
| H4 flags | Is `RESIZE_QUIRK` needed for a reproducible bug? Is `WIN32_INPUT_MODE` ever desirable? | Answered in 0.10.5: neither; keep `INHERIT_CURSOR` only until a failing test exists |
| K1a parser | Fix push/pop tests to the spec; separate main/alt stacks | 0.10.7; direct spec vectors |
| K1b placement | Encoder on `TerminalState`/`HistoryScreen` vs a standalone `KeyboardEncoder` | K1b design; keep `PtySession` transport-only in Rust |
| K1b flags | Start with flag 1 only, or also 2/8 in the first release? | K1b; flag 1 covers Shift+Enter, 8 covers all keys |
| P1 parser ownership | Should `HistoryScreen` own the parser, or should `TerminalState` feed its stored parser? | Answered: `HistoryScreen` owns the sole persistent parser; `TerminalState` delegates. Implemented in 0.10.6 (pending release) |
| D1 interop | `from_parts` + slave/pair API + AsyncRead/handles; which shape? | D1 design; keep `PtySession` emulator-free |
| D2 parity scope | Which Python-only conveniences stay Python-only (`raw_output`, `expect`, owned terminal)? | D1/D2; classify in parity table |
| Python surface | Should Python expose `close()`/`read_until_exit`/`take_writer`? | Rust-only in 0.10.2; decide with the G4 follow-up |
| Version numbering | Do G6/G5 land as separate minors or one batched release? | Scheduling; follow `IMPROVEMENT_PLAN.md` bump rules |

---

## 10. Definition of done for the 0.10 series

- PR #2 merged to `dev`; 0.10.0 released from tag `v0.10.0` (`e620cc8`);
  0.10.1 released from tag `v0.10.1` (`97e03ab`); PyPI and crates.io
  published for both. 0.10.2 through 0.10.6 are implemented on `dev`; their
  releases are pending explicit confirmation.
- All seven original gaps, H1–H3, K1a/K1b, P1, and D1/D2 have open issues
  linked to PR #2 and this plan; H4 has a recorded spike decision.
- Issue #1 is linked to K1a/K1b and is not closed before the PySide6 example
  uses the encoder.
- Each 0.10.x patch has a CHANGELOG entry, tests, and a green CI run.
- No deferred gap is silently dropped or “fixed” by documentation alone without
  an explicit decision.
- `dev` and `main` are not merged without maintainer confirmation.
- The next breaking Rust API change (G2, plus G1 only if needed) has a migration
  note ready before its release.
- The Windows close/EOF path has a test for unread output and a documented
  blocking/close-order contract.
- `HistoryScreen`/`TerminalState` use one persistent parser; split CSI/OSC/UTF-8
  feeds are content-tested.
- CI proves `--no-default-features` builds/tests without PyO3, and the Rust/
  Python parity table is reviewed.

---

## Appendix A — source references

These are the specific files that informed the plan. Line numbers are approximate
for the review date; use the linked revision for exact context.

**WezTerm / `portable-pty`**

- [`pty/src/lib.rs`](https://github.com/wezterm/wezterm/blob/main/pty/src/lib.rs)
  — `MasterPty`/`Child`/`ChildKiller` traits, `ExitStatus`, `PtyPair`.
  - `MasterPty::as_raw_fd` / `Child::as_raw_handle`: L90–L147.
  - `ExitStatus` and `ChildKiller`: L132–L190.
- [`pty/src/cmdbuilder.rs`](https://github.com/wezterm/wezterm/blob/main/pty/src/cmdbuilder.rs)
  — `OsString` env map, `env`/`env_remove`/`env_clear`, Windows case mapping.
  - Builder: L203–L342; `environment_block`: L654.
- [`pty/src/unix.rs`](https://github.com/wezterm/wezterm/blob/main/pty/src/unix.rs)
  — `UnixMasterWriter` drop sends newline + `VEOF`: L393–L404.
- [`pty/src/win/pseudocon.rs`](https://github.com/wezterm/wezterm/blob/main/pty/src/win/pseudocon.rs)
  — ConPTY flags, `STARTF_USESTDHANDLES` with invalid handles, `PseudoCon` Drop.
  - Flags: L25–L29; Drop: L71; startup handles: L120–L124.
- [`term/src/lib.rs`](https://github.com/wezterm/wezterm/blob/main/term/src/lib.rs)
  — “does not directly manage a PTY”; `advance_bytes` entrypoint.
- [`term/src/terminal.rs`](https://github.com/wezterm/wezterm/blob/main/term/src/terminal.rs)
  — `Terminal::new(..., writer)`, `advance_bytes`, sequence tracking.

**Alacritty**

- [`alacritty_terminal/src/tty/mod.rs`](https://github.com/alacritty/alacritty/blob/master/alacritty_terminal/src/tty/mod.rs)
  — `Options` (`drain_on_exit`, env), `ChildEvent`, `EventedPty`.
  - `Options`: L23–L43; `ChildEvent`/`EventedPty`: L82–L96.
- [`alacritty_terminal/src/tty/unix.rs`](https://github.com/alacritty/alacritty/blob/master/alacritty_terminal/src/tty/unix.rs)
  — SIGCHLD pipe, `Drop` SIGHUP + wait, `next_child_event`.
  - SIGCHLD: L282–L283; Drop: L309; event: L384.
- [`alacritty_terminal/src/tty/windows/conpty.rs`](https://github.com/alacritty/alacritty/blob/master/alacritty_terminal/src/tty/windows/conpty.rs)
  — `Conpty::drop` blocking/drain warning; `STARTF_USESTDHANDLES`; Windows env dedup.
  - Drop: L97–L104; startup handles: L148; env conversion: L250.
- [`alacritty_terminal/src/tty/windows/mod.rs`](https://github.com/alacritty/alacritty/blob/master/alacritty_terminal/src/tty/windows/mod.rs)
  — backend-first drop-order comment: L27–L34.
- [`alacritty_terminal/src/tty/windows/child.rs`](https://github.com/alacritty/alacritty/blob/master/alacritty_terminal/src/tty/windows/child.rs)
  — `ChildExitWatcher` / `RegisterWaitForSingleObject`: L52–L90.
- [`alacritty_terminal/src/event_loop.rs`](https://github.com/alacritty/alacritty/blob/master/alacritty_terminal/src/event_loop.rs)
  — `drain_on_exit`, child-exit event, Linux `EIO`: L259–L290.

**emux-pty**

- [`crates/emux-pty/src/lib.rs`](https://github.com/IISweetHeartII/emux/blob/main/crates/emux-pty/src/lib.rs)
  — combined `Pty: Read + Write` trait: L30.
- [`crates/emux-pty/src/unix.rs`](https://github.com/IISweetHeartII/emux/blob/main/crates/emux-pty/src/unix.rs)
  — `ExitStatus` enum: L86; `wait`/`is_alive`: L213–L229; Drop: L256.
- [`crates/emux-pty/src/windows.rs`](https://github.com/IISweetHeartII/emux/blob/main/crates/emux-pty/src/windows.rs)
  — Windows `ExitStatus`: L89; env block with `std::env::vars()`: L363–L386;
  Drop: L405.
- [`crates/emux-pty/src/cmdbuilder.rs`](https://github.com/IISweetHeartII/emux/blob/main/crates/emux-pty/src/cmdbuilder.rs)
  — additive env map only.

**Issue #1 / Kitty keyboard protocol**

- [stitch-pty issue #1](https://github.com/opticsWolf/stitch-pty/issues/1) —
  Shift+Enter/CSI u feature request and the offered Python implementation.
- [“Your Terminal Can't Tell Shift+Enter from Enter”](https://blog.fsck.com/agent-blog/2026/02/26/terminal-keyboard-protocol/)
  — Jesse Vincent’s explainer; useful for the problem statement, terminal
  support matrix, and framework examples. It is dated 2026-02-26, so treat the
  support matrix as time-sensitive.
- [Kitty keyboard protocol specification](https://sw.kovidgoyal.net/kitty/keyboard-protocol/)
  — authoritative for flags, `CSI > flags u`, `CSI < n u`, `CSI ? u` →
  `CSI ? flags u`, and `CSI = flags ; mode u`.
- [`kitty/screen.c`](https://github.com/kovidgoyal/kitty/blob/master/kitty/screen.c)
  — query response at L2317; push/pop stack at L2330–L2370.
- [`kitty/key_encoding.c`](https://github.com/kovidgoyal/kitty/blob/master/kitty/key_encoding.c)
  — `legacy_mode` at L165, Enter special-case at L192, CSI u encoding at L230.
- Current stitch-pty code: `src/terminal/screen.rs:1109–1129` (mode methods),
  `src/terminal/screen.rs:1353` (parser dispatch), and
  `examples/terminal_emulator.py` (`_key_to_bytes`).

**Licensing note.** WezTerm is MIT; Alacritty is Apache-2.0; emux-pty’s license
is defined by its repository. Use these as design references, not as code to
copy without checking the license and attribution requirements.
