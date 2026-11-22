# stitch-pty Review Remediation Plan

Status: proposed  
Repository: `D:\User\Documents\Python\stitch-pty`  
Reviewed commit: `5a050d6 perf(terminal): skip blank rows during full-display erase`  
Branch: `main`  
Guidelines: Microsoft Pragmatic Rust Guidelines, captured locally by the `rust-guidelines` skill

This plan converts the guideline-based review of `stitch-pty` into an executable,
phase-ordered remediation program. It is intentionally conservative: the crate is
published as `stitch-pty 0.16.0`, is used by `stitch-mux`, and exposes both a Rust
API and a Python extension. Changes that alter public types or Python-visible
behavior must be additive where possible and separately validated.

---

## Contents

- [Review evidence](#review-evidence)
- [Guideline IDs used](#guideline-ids-used)
- [Phase map](#phase-map)
- [Phase 0 — restore static verification](#phase-0--restore-static-verification)
- [Phase 1 — correctness and unsafe ownership](#phase-1--correctness-and-unsafe-ownership)
- [Phase 2 — error and API hardening](#phase-2--error-and-api-hardening)
- [Phase 3 — documentation and module hygiene](#phase-3--documentation-and-module-hygiene)
- [Phase 4 — measured performance work](#phase-4--measured-performance-work)
- [Phase 5 — test layout](#phase-5--test-layout)
- [Validation matrix](#validation-matrix)
- [Benchmark acceptance](#benchmark-acceptance)
- [Commit sequence](#commit-sequence)
- [Rollback rules](#rollback-rules)
- [Non-goals](#non-goals)

---

## Review evidence

### Repository state

```text
branch: main
HEAD:   5a050d6 perf(terminal): skip blank rows during full-display erase
status: clean at review time
source: ~14,764 Rust LOC under src/
```

### Validation results at review time

```text
cargo fmt --all -- --check                            FAIL
cargo clippy --all-targets -- -D warnings             PASS
cargo clippy --all-targets --all-features -- -D warnings FAIL
cargo test --all-targets                              PASS
  lib: 446 passed, 0 failed
cargo test --all-targets --all-features               local env failure
  STATUS_DLL_NOT_FOUND / Python DLL unavailable
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --lib  PASS
cargo check --no-default-features --lib               PASS
cargo tree --no-default-features -e normal -i pyo3    correctly reports no match
```

`cargo fmt --check` diffs are in:

```text
src/terminal/ansi_parser.rs
src/terminal/history.rs
src/terminal/parser.rs
src/terminal/screen.rs
```

`--all-features` clippy failures:

```text
src/python_api.rs:271  clippy::redundant_locals
src/winsize.rs:5       clippy::clone_on_copy from #[pyclass(from_py_object)]
```

Unused dependency warnings:

```text
tokio-util
tracing
unicode-segmentation
```

### Static counts

```text
unsafe blocks:            57
unsafe fn/impl/extern:     7
SAFETY comments:           0
#[allow(...)] attributes:  3
println!/eprintln!/dbg!:   0 in src/
glob re-exports:           0
```

### Existing strengths

The review found several areas that should be preserved:

- `M-HOTPATH`: the crate has a real headless benchmark crate with p50/p95, raw
  samples, allocator counters, host/profile metadata, and correctness gates.
- `M-MEM-REUSE`: fixed-size `Params`, blank-cell pools, blank-row pools, and
  row-moving reflow are already in place.
- `M-FEATURES-ADDITIVE`: `default = []`; Python, regex, and profiling features
  are opt-in.
- `M-MOCKABLE-SYSCALLS`: platform traits, `PtySession::from_parts`, and injected
  sleep in the termination policy provide useful seams.
- `M-LOG-NOT-PRINT`: no ad-hoc console diagnostics in library code.
- `M-NO-PRELUDE` and `M-NO-GLOB-REEXPORTS`: no broad namespace exports.

---

## Guideline IDs used

| ID | Relevance |
|---|---|
| `M-STATIC-VERIFICATION` | fmt/clippy/CI must be green before deeper refactors. |
| `M-UNSAFE`, `M-UNSOUND` | Unsafe blocks need rationale; handle ownership must be sound. |
| `M-PANIC-IS-STOP`, `M-PANIC-MESSAGE` | Production panics need invariants or typed errors. |
| `M-ERRORS-CANONICAL-STRUCTS`, `M-FROM-ERROR` | Error model needs context and canonical conversion. |
| `M-STRONG-TYPES` | OS paths should be `Path`-like, not `&str`. |
| `M-PUBLIC-DEBUG` | Public handles and screen/parser types need summary `Debug`. |
| `M-LINT-OVERRIDE-EXPECT` | Lint suppressions must carry reasons. |
| `M-DOC-INLINE`, `M-MODULE-DOCS`, `M-CANONICAL-DOCS` | Public docs need discoverability and canonical sections. |
| `M-MEM-REUSE`, `M-INITIAL-CAPACITY`, `M-BOX-DST` | Remaining allocation-heavy serialization paths. |
| `M-FAST-HASHER` | Only if profiling shows hash-set cost. |
| `M-INTEGRATION-TESTS` | Move public-API tests out of source modules incrementally. |
| `M-HOTPATH` | Benchmark before and after every performance change. |

---

## Phase map

| Phase | Priority | Scope | Risk | Acceptance |
|---|---:|---|---|---|
| 0 | P0 | fmt, clippy, unused deps, CI | low | all static gates green |
| 1a | P0 | Windows handle ownership | high | no double-close; Windows tests pass |
| 1b | P0 | `SAFETY` comments | low | every unsafe site explained |
| 1c | P1 | wide-character cursor bug | medium | screen oracle green |
| 1d | P1 | production panic cleanup | medium | no bare unwraps in library paths |
| 2a | P2 | canonical error struct | high / API | additive or versioned |
| 2b | P2 | `Path` APIs | low / additive | old APIs preserved |
| 2c | P2 | `Debug`, `#[expect]` | low | public types covered |
| 3 | P2 | docs and re-exports | low | rustdoc warnings clean |
| 4 | P3 | allocation and hashing work | medium | benchmark-backed only |
| 5 | P3 | test layout | low | no test coverage loss |

Do not start Phase 2 until Phases 0 and 1 are green.  
Do not start Phase 4 until correctness gates are green.

---

## Phase 0 — restore static verification

Guidelines: `M-STATIC-VERIFICATION`, `M-LINT-OVERRIDE-EXPECT`

### 0.1 Apply rustfmt

Run:

```sh
cd D:/User/Documents/Python/stitch-pty
cargo fmt --all
cargo fmt --all -- --check
```

Expected changed files:

```text
src/terminal/ansi_parser.rs
src/terminal/history.rs
src/terminal/parser.rs
src/terminal/screen.rs
```

This should be a formatting-only commit. No logic changes belong in this step.

### 0.2 Fix all-features clippy failure

File: `src/python_api.rs`

Current problem:

```rust
let winsize = winsize;
```

Replace by deleting the redundant binding and using `winsize` directly in the
async block:

```rust
pub fn spawn<'py>(
    py: Python<'py>,
    program: &str,
    args: Option<Vec<String>>,
    env: Option<Vec<(String, String)>>,
    winsize: Option<Winsize>,
    cwd: Option<String>,
) -> PyResult<Bound<'py, PyAny>> {
    let program = program.to_string();
    let args = args.unwrap_or_default();

    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        let (pty_backend, child_backend) =
            spawn_platform(&program, &args, env.as_deref(), winsize, cwd.as_deref())
                .await
                .map_err(|e| PtyErrorKind::ForkFailed(e.to_string()))?;
        // ...
    })
}
```

File: `src/winsize.rs`

Clippy flags `#[pyclass(from_py_object)]` because `Winsize` is `Copy`.

Preferred fix:

- Remove `from_py_object` if Python-side construction does not depend on it.
- If it is required, use a narrow suppression:

```rust
#[cfg(feature = "python")]
#[pyclass]
#[expect(
    clippy::clone_on_copy,
    reason = "PyO3 from_py_object expansion clones Winsize; Winsize is Copy and has no side effects"
)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Winsize {
    // ...
}
```

Verify with:

```sh
cargo clippy --all-targets --all-features -- -D warnings
```

### 0.3 Resolve unused dependencies

Cargo currently reports:

```text
tokio-util
tracing
unicode-segmentation
```

For each dependency, choose one:

1. Remove it from `Cargo.toml` if there is no planned use.
2. Use it intentionally in code.
3. Document why it must remain for downstream feature compatibility and add a
   narrow manifest-level allowance if Cargo supports the needed granularity.

Recommended default: remove all three. None is referenced in `src/`, `tests/`,
or `examples/`.

After removal:

```sh
cargo check --all-targets
cargo check --all-targets --all-features
```

### 0.4 Extend CI

File: `.github/workflows/CI.yml`

Add all-features clippy next to the existing lint command:

```yaml
- name: Lint (fmt, clippy, ruff, mypy)
  run: |
    cargo fmt --all --check
    cargo clippy --all-targets -- -D warnings
    cargo clippy --all-targets --all-features -- -D warnings
    pip install "ruff==0.16.5" "mypy==2.3.0"
    ruff check python/ tests/
    mypy python/stitch_pty/
```

Keep the pinned ruff/mypy versions; the existing comment correctly warns about
silent linter upgrades.

### Phase 0 acceptance

```text
cargo fmt --all -- --check                             ok
cargo clippy --all-targets -- -D warnings              ok
cargo clippy --all-targets --all-features -- -D warnings ok
cargo check --all-targets                              ok
cargo check --all-targets --all-features               ok
no unused-dependency warnings
```

---

## Phase 1 — correctness and unsafe ownership

### 1.1 Fix Windows handle double-close risk

Guidelines: `M-UNSAFE`, `M-UNSOUND`-adjacent resource safety, `M-MOCKABLE-SYSCALLS`

Files:

```text
src/platform_windows.rs
```

Current problem:

```rust
struct SendSyncHandle(HANDLE);

unsafe impl Send for SendSyncHandle {}
unsafe impl Sync for SendSyncHandle {}

impl Clone for SendSyncHandle {
    fn clone(&self) -> Self {
        SendSyncHandle(self.0)
    }
}

impl Clone for WinChildProcess {
    fn clone(&self) -> Self {
        WinChildProcess {
            process_handle: self.process_handle.clone(),
            thread_handle: self.thread_handle.clone(),
            exit: self.exit.clone(),
        }
    }
}

impl Drop for WinChildProcess {
    fn drop(&mut self) {
        ...
        unsafe {
            let _ = CloseHandle(self.process_handle.0);
            let _ = CloseHandle(self.thread_handle.0);
        }
    }
}
```

`clone_killer()` returns a cloned `WinChildProcess`. Each clone owns the same
raw `HANDLE` values and each `Drop` closes them. This can close a kernel object
more than once and, if the numeric handle value is reused, can close an unrelated
object.

#### Target design

Introduce a refcounted owner so the handle closes exactly once:

```rust
#[cfg(windows)]
struct HandleOwner(HANDLE);

#[cfg(windows)]
impl Drop for HandleOwner {
    fn drop(&mut self) {
        // SAFETY: `HandleOwner` is the sole owner of this duplicated or
        // transferred raw handle value. It is closed exactly once here.
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

// SAFETY: Win32 HANDLE values are process-wide kernel object references.
// Sharing the reference across threads is valid because all access is
// synchronized by the owning backend's locks or by the kernel. The handle is
// closed once by `HandleOwner::drop`.
#[cfg(windows)]
unsafe impl Send for HandleOwner {}
#[cfg(windows)]
unsafe impl Sync for HandleOwner {}

#[cfg(windows)]
#[derive(Clone)]
struct SendSyncHandle(Arc<HandleOwner>);
```

Then migrate `WinChildProcess` to share `SendSyncHandle` by `Arc` clone.

Important migration details:

- Audit every `SendSyncHandle(HANDLE)` construction site before adding a global
  `Drop`. Some handles are currently closed by `ClosePseudoConsole`, not by
  `CloseHandle` directly.
- Preserve `WinPtyBackend::close()` idempotence through its shared
  `Arc<AtomicBool>`.
- Do not add a second close path for ConPTY handles.
- `clone_killer()` must produce a handle that can outlive the original child
  object without double-closing.
- Add a mockable close wrapper for tests, following `M-MOCKABLE-SYSCALLS`:

```rust
#[cfg(test)]
type CloseHandleFn = fn(HANDLE) -> bool;
```

or a narrow internal function pointer that defaults to `CloseHandle`.

#### Tests

Add Windows-only tests:

```text
owned_handle_closes_once_on_last_drop
cloned_child_killer_does_not_close_handle_twice
winpty_backend_clone_preserves_close_idempotence
```

Use a mock close counter rather than real kernel handles where possible.

#### Acceptance

```text
cargo test --all-targets
Windows CI job passes
no double-close in clone_killer path
```

### 1.2 Add `SAFETY` comments to unsafe code

Guideline: `M-UNSAFE`

There are 57 `unsafe` blocks and no `SAFETY` comments.

Minimum standard: every `unsafe` block gets a plain-text rationale immediately
above it.

Examples:

#### UTF-8 unchecked prefix

File: `src/terminal/ansi_parser.rs:314`

```rust
Err(err) => {
    let valid_bytes = err.valid_up_to();
    // SAFETY: `Utf8Error::valid_up_to()` returns the byte index at which the
    // first invalid UTF-8 sequence starts, so `&bytes[..valid_bytes]` is a
    // valid UTF-8 slice.
    let parsed = unsafe { std::str::from_utf8_unchecked(&bytes[..valid_bytes]) };
    // ...
}
```

#### Unix child setup

File: `src/platform_unix.rs`, around `child_setup`

Add a module-level comment and per-block comments covering:

- all allocations occur in `PreparedCommand::new` before `fork`;
- child code uses only async-signal-safe calls;
- raw pointers remain valid because their owning allocations stay alive in the
  parent until `execve`/`_exit`;
- `_exit` is used instead of `std::process::exit` to avoid flushing inherited
  buffers.

#### Windows FFI

File: `src/platform_windows.rs`

For dynamic ConPTY function pointers, `DuplicateHandle`, `CloseHandle`,
`WaitForSingleObject`, and STARTUPINFOEX setup, state:

- where the handle/pointer comes from;
- who owns it;
- why its lifetime covers the call;
- why the call is safe on an arbitrary thread;
- what happens on failure.

#### Acceptance

```text
rg -n 'unsafe \{' src | wc -l
rg -n 'SAFETY' src | wc -l
```

The second count should be at least the number of distinct unsafe sites; one
comment may cover a tightly grouped block if the rationale is identical.

### 1.3 Fix the documented width-two cursor bug

File: `src/terminal/screen.rs`  
Evidence: `docs/TESTS.md:163`

```text
Stitch PTY 0.16.0 leaves the cursor stationary for consecutive width-two characters.
Treat that as a failing Unicode correctness case until wide-character placement
and wrapping are repaired.
```

This is the highest-primate behavioral correctness issue in the review.

#### Work

1. Create a regression test that feeds a known wide-character corpus:

   - two or more consecutive CJK characters;
   - an emoji sequence with variation selector;
   - wide glyph at the last column with pending wrap;
   - wide glyph crossing a soft-wrap boundary;
   - same corpus with byte-at-a-time feeds.

2. Define an explicit expected oracle:

   - leading cell contains glyph;
   - trailing cell is a blank continuation cell;
   - cursor advances by two columns per wide glyph;
   - pending wrap is set according to the chosen xterm-compatible behavior;
   - scrollback/reflow preserves continuation semantics.

3. Fix `Screen` write/cursor-advance logic without special-casing only the
   benchmark corpus.

4. Verify both whole-stream and split-stream feeds produce identical screens.

#### Test location

Add a unit test near the screen write logic and an integration-style test under
`tests/terminal_parity.rs` if only public API is needed.

#### Acceptance

```text
correctness.failures = 0 in benchmark/results/parser.json
cargo test --all-targets passes
docs/TESTS.md known issue is removed only after the oracle is green
```

Do not delete the known-issue note before the regression test exists.

### 1.4 Replace panic-prone production unwraps

Guidelines: `M-PANIC-IS-STOP`, `M-PANIC-MESSAGE`

Non-test sites found:

```text
src/platform_unix.rs:177   async_fd.as_ref().unwrap()
src/platform_unix.rs:185   async_fd.as_ref().unwrap()
src/platform_unix.rs:213   async_fd.as_ref().unwrap()
src/platform_windows.rs:364 self.size.lock().unwrap()
src/platform_windows.rs:448 self.size.lock().unwrap()
src/platform_windows.rs:453 self.size.lock().unwrap()
src/terminal_api.rs:127    into_pyobject(...).unwrap()
src/terminal_api.rs:131    .unwrap()
src/terminal_api.rs:136    into_pyobject(...).unwrap()
```

#### Rules

- Recoverable state → return `PtyResult`/`PyResult`.
- True invariant → `expect("...")` with a message that identifies the invariant.
- Lock poisoning → either use `parking_lot::Mutex` or add a poison-specific
  message.

#### Example

```rust
let guard = self
    .async_fd
    .as_ref()
    .expect("PTY async fd must exist while the backend is open")
    .readable()
    .await?;
```

```rust
let size = *self
    .size
    .lock()
    .expect("winsize mutex poisoned: another thread panicked while holding it");
```

For PyO3 conversion:

```rust
let obj = value.into_pyobject(py)?.into_any().unbind();
```

#### Acceptance

```text
no bare .unwrap() in production paths under src/
every production .expect() has a reason
```

---

## Phase 2 — error and API hardening

### 2.1 Canonical error struct

Guidelines: `M-ERRORS-CANONICAL-STRUCTS`, `M-FROM-ERROR`

Current shape:

```rust
pub enum PtyErrorKind {
    OpenFailed(String),
    OperationFailed(String),
    ...
}
```

It implements `Display`/`Error` via `thiserror`, but it is a public enum with
string payloads, no backtrace, and no structured source chain.

#### Target shape

Introduce an additive canonical error type while preserving `PtyErrorKind`:

```rust
#[derive(Debug)]
pub struct PtyError {
    kind: PtyErrorKind,
    backtrace: std::backtrace::Backtrace,
}

impl PtyError {
    pub(crate) fn new(kind: PtyErrorKind) -> Self {
        Self {
            kind,
            backtrace: std::backtrace::Backtrace::capture(),
        }
    }

    pub fn is_eof(&self) -> bool {
        matches!(self.kind, PtyErrorKind::Eof)
    }

    pub fn is_timeout(&self) -> bool {
        matches!(self.kind, PtyErrorKind::Timeout(_))
    }

    pub fn is_closed(&self) -> bool {
        matches!(self.kind, PtyErrorKind::Closed)
    }

    pub fn kind(&self) -> &PtyErrorKind {
        &self.kind
    }
}

impl std::fmt::Display for PtyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.kind)?;
        write!(f, "backtrace: {}", self.backtrace)
    }
}

impl std::error::Error for PtyError {}
```

Add `From` conversions once:

```rust
impl From<std::io::Error> for PtyError { ... }
impl From<nix::Error> for PtyError { ... } // cfg(unix)
impl From<windows::core::Error> for PtyError { ... } // cfg(windows)
```

#### Rollout options

Because `PtyResult<T>` currently aliases `Result<T, PtyErrorKind>`:

- Preferred: introduce new APIs returning `Result<T, PtyError>` and keep old
  aliases for compatibility.
- If a breaking release is planned, change `PtyResult` in a dedicated commit.

Do not mix this with Python exception mapping changes. Preserve the existing
`kind == "eof"` contract in `errors.rs`.

#### Acceptance

```text
Python exception tests pass
Rust error helper tests pass
cargo doc -D warnings passes
```

### 2.2 Add `Path`-based spawn APIs

Guideline: `M-STRONG-TYPES`

Current APIs use `&str` for `program` and `cwd`:

```rust
pub async fn spawn(
    program: &str,
    args: &[String],
    winsize: Option<Winsize>,
    cwd: Option<&str>,
) -> PtyResult<Self>
```

`cwd` is an OS path and should be `Path`-like.

Additive APIs:

```rust
impl PtySession {
    pub async fn spawn_path(
        program: impl AsRef<std::path::Path>,
        args: &[String],
        winsize: Option<Winsize>,
        cwd: Option<&std::path::Path>,
    ) -> PtyResult<Self> {
        // convert with explicit non-UTF8/NUL validation
    }

    pub async fn spawn_path_with_env(
        program: impl AsRef<std::path::Path>,
        args: &[String],
        env: &[(String, String)],
        winsize: Option<Winsize>,
        cwd: Option<&std::path::Path>,
    ) -> PtyResult<Self> {
        // ...
    }
}
```

Keep `spawn` and `spawn_with_env` as wrappers.

Apply the same pattern to `TerminalSession` and platform helpers where public.

#### Acceptance

```text
old &str APIs still compile
new Path APIs accept non-UTF8 paths on Unix where representable
NUL bytes still produce typed errors
```

### 2.3 Add `Debug` to public types

Guideline: `M-PUBLIC-DEBUG`

Missing summary `Debug`:

```text
src/session.rs:447            PtyReader
src/session.rs:482            PtyWriter
src/terminal/history.rs:60    HistoryScreen
src/terminal/screen.rs:258    Screen
src/terminal/ansi_parser.rs:207 Parser
src/terminal/parser.rs:11     Performer
src/terminal/parser.rs:179    Parser
src/terminal/parser.rs:211    Stream
src/terminal_api.rs:21        TerminalState
src/platform_windows.rs:207   WinChildProcess
src/platform_windows.rs:354   WinPtyBackend
```

For large screen types, use summary impls rather than dumping buffers:

```rust
impl std::fmt::Debug for HistoryScreen {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HistoryScreen")
            .field("columns", &self.columns())
            .field("lines", &self.lines())
            .field("scrollback", &self.scrollback_lines())
            .field("history_len", &self.history_len())
            .finish_non_exhaustive()
    }
}
```

For readers/writers, show session identity and state, not PTY internals.

### 2.4 Replace lint allows with reasoned expects

Guideline: `M-LINT-OVERRIDE-EXPECT`

Current:

```text
src/terminal_api.rs:231   #[allow(clippy::too_many_arguments)]
src/platform_unix.rs:66   #[allow(dead_code)]
src/platform_unix.rs:68   #[allow(dead_code)]
```

Target:

```rust
#[expect(
    clippy::too_many_arguments,
    reason = "PyO3 signature mirrors the documented Python API"
)]
```

```rust
#[expect(
    dead_code,
    reason = "owns CString buffers pointed to by argv_ptrs; must stay alive across fork/exec"
)]
argv: Vec<CString>,
```

---

## Phase 3 — documentation and module hygiene

Guidelines: `M-DOC-INLINE`, `M-MODULE-DOCS`, `M-CANONICAL-DOCS`, `M-FIRST-DOC-SENTENCE`

### 3.1 Mark deliberate re-exports inline

Files:

```text
src/lib.rs
src/terminal/mod.rs
```

Current examples:

```rust
pub use terminal::{
    Char, Cursor, HistoryScreen, KeyCode, KeyEvent, KeyEventKind, KeyboardFlags, Modifiers, Parser,
    Screen, encode_key,
};
```

Change to:

```rust
#[doc(inline)]
pub use terminal::{
    Char, Cursor, HistoryScreen, KeyCode, KeyEvent, KeyEventKind, KeyboardFlags, Modifiers, Parser,
    Screen, encode_key,
};
```

Apply the same to:

```text
src/lib.rs pub use expect::{...}
src/lib.rs pub use session::{...}
src/lib.rs pub use terminal_session::TerminalSession
src/lib.rs pub use winsize::Winsize
src/terminal/mod.rs pub use history::HistoryScreen
src/terminal/mod.rs pub use keyboard::{...}
src/terminal/mod.rs pub use parser::{Parser, Stream}
src/terminal/mod.rs pub use screen::{Char, Cursor, Margins, Screen}
```

### 3.2 Add module docs

Missing `//!` module docs:

```text
src/terminal/history.rs
src/terminal/screen.rs
src/winsize.rs
```

Each should state:

- what the module owns;
- relationship to neighboring modules;
- stability/lifetime expectations;
- one short usage example where useful.

### 3.3 Canonical public type sections

For primary public types, add canonical sections where applicable:

```text
Examples
Errors
Panics
Safety
```

Priority order:

```text
PtySession
PtyReader
PtyWriter
TerminalSession
HistoryScreen
Screen
Winsize
ExpectOptions
ExpectError
```

Keep first doc sentences to one line, approximately 15 words, per
`M-FIRST-DOC-SENTENCE`.

### Phase 3 acceptance

```text
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --lib
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
```

---

## Phase 4 — measured performance work

Guidelines: `M-HOTPATH`, `M-MEM-REUSE`, `M-INITIAL-CAPACITY`, `M-BOX-DST`, `M-FAST-HASHER`

Do not start this phase until all correctness gates are green.

### 4.1 Keep the current benchmark baseline

Baseline file:

```text
benchmark/results/parser.json
```

Current host/profile from the report:

```text
os: windows
arch: x86_64
cpu_count: 32
profile: release
```

Current parser profile facts:

- native parser lanes report `0` allocations/MiB;
- ASCII native p50 is roughly ~998 MB/s;
- CSI native p50 is roughly ~292 MB/s;
- `correctness.failures = 0`;
- `vte` Unicode split-boundary mismatch remains a candidate note, not a native
  failure.

Do not compare numbers from different hosts or profiles.

### 4.2 Reduce styled serialization allocations

Files:

```text
src/terminal/history.rs
src/terminal/screen.rs
```

Current hot shape:

```rust
pub fn styled_viewport(&self) -> Vec<Vec<(String, String, String, u8)>> {
    self.history
        .iter()
        .chain(self.inner.buffer.iter())
        .map(|row| row.iter().map(pack_cell).collect())
        .collect()
}
```

Problems:

- allocates one `String` per cell field;
- allocates one row `Vec` without capacity;
- allocates an outer `Vec` without capacity;
- scans all retained lines.

Low-risk fix:

```rust
pub fn styled_range(
    &self,
    start: usize,
    count: usize,
) -> Vec<Vec<(String, String, String, u8)>> {
    let total = self.total_lines();
    let end = start.saturating_add(count).min(total);
    let rows = end.saturating_sub(start);
    let mut out = Vec::with_capacity(rows);

    for row in start..end {
        if let Some(cells) = self.row_cells(row) {
            let mut styled = Vec::with_capacity(cells.len());
            styled.extend(cells.iter().map(pack_cell));
            out.push(styled);
        }
    }
    out
}
```

Document that `styled_viewport()` is O(total retained lines) and that callers
needing a window should use `styled_range()`.

### 4.3 Evaluate compact cell storage, but do not rush it

File: `src/terminal/screen.rs`

Current:

```rust
pub struct Char {
    pub data: String,
    pub fg: String,
    pub bg: String,
    ...
}
```

Guidelines `M-BOX-DST` and `M-MEM-REUSE` suggest a leaner model, but this is a
public breaking change.

Only start this work if:

- the screen benchmark shows string operations still dominate; and
- a design preserves Python behavior and Rust ergonomics; and
- it is isolated behind an additive API or a major-version plan.

Candidate direction:

- inline single-scalar glyph text;
- heap fallback only for multi-codepoint grapheme clusters;
- numeric/interned color representation;
- accessors instead of public `String` fields.

Do not mix this with Phase 1 correctness work.

### 4.4 Fast hashers

Files:

```text
src/terminal/screen.rs
src/terminal/modes.rs
```

Current default `HashSet` uses:

```text
tabstops: HashSet<usize>
extended: HashSet<u16>
```

These sets are probably small. Add a benchmark lane before changing the hasher.
If no measurable difference, leave the standard hasher.

### 4.5 Performance acceptance

For every Phase 4 change:

```text
correctness.failures = 0
parser allocations/MiB do not regress
no p50 regression >5% outside measurement noise
screen oracle / parity tests pass
```

---

## Phase 5 — test layout

Guideline: `M-INTEGRATION-TESTS`

Large in-source test sections:

```text
src/terminal/screen.rs      ~1,268 test LOC
src/terminal/history.rs     ~983 test LOC
src/terminal/parser.rs      ~944 test LOC
src/terminal/ansi_parser.rs ~563 test LOC
```

Approach:

1. Move tests that use only public API to `tests/`.
2. Keep private-invariant tests in `#[cfg(test)]` modules.
3. Preserve proptest regressions under `proptest-regressions/`.
4. Do not move tests in the same commit as behavior changes.

Suggested new integration test files:

```text
tests/terminal_screen_public.rs
tests/terminal_history_public.rs
tests/terminal_parser_public.rs
tests/keyboard_public.rs
```

---

## Validation matrix

Run after each phase and again at the end.

```sh
cd D:/User/Documents/Python/stitch-pty

cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo clippy --all-targets --all-features -- -D warnings

cargo test --all-targets
cargo test --no-default-features --lib
cargo test --no-default-features --features regex --lib

cargo check --no-default-features --lib
cargo check --no-default-features --all-targets
cargo check --all-targets --all-features

RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --lib

if cargo tree --no-default-features -e normal -i pyo3 >/tmp/pyo3-tree.txt 2>&1; then
  echo "pyo3 must not be in the default dependency tree"
  exit 1
fi
```

With a working Python environment:

```sh
pip install -e .[dev] pytest-timeout
pytest tests/ -v --timeout=60 --timeout-method=thread
ruff check python/ tests/
mypy python/stitch_pty/
```

Benchmark:

```sh
cargo run --release --manifest-path benchmark/Cargo.toml -- \
  --samples 30 --warmup 5 --bytes 1048576 \
  --out benchmark/results/parser.json
```

Note: local `cargo test --all-features` currently fails with
`STATUS_DLL_NOT_FOUND` because this Windows environment lacks the required
Python DLL. Treat that as an environment issue; CI with Python installed must
pass.

---

## Benchmark acceptance

After remediation:

```text
correctness.failures: 0
candidate_notes: unchanged unless vte behavior is intentionally revisiting
parser native allocations per MiB: 0
terminal draw/scroll lanes: no unexplained regression
p50 throughput: within noise or improved
```

Do not claim cross-project rankings from this benchmark. It measures
`stitch-pty` parser/screen lanes, not kitty/alacritty/tmux latency.

---

## Commit sequence

Recommended split:

```text
1. style: apply rustfmt to terminal modules
2. fix(ci): restore all-features clippy and remove unused dependencies
3. fix(windows): share OS handle ownership across cloned child killers
4. docs(safety): add unsafe rationale to platform and parser code
5. fix(terminal): correct consecutive wide-character cursor placement
6. fix(errors): add contextual error handling and panic messages
7. feat(api): add Path-based spawn APIs and Debug impls
8. refactor(lints): replace lint allows with reasoned expects
9. docs: mark re-exports inline and add missing module docs
10. perf(terminal): pre-size styled range output buffers
```

Optional later commits:

```text
11. refactor(errors): introduce canonical PtyError struct
12. refactor(cells): compact terminal cell storage
13. test: move public API tests to tests/
```

Do not combine commits 3, 5, and 11.

---

## Rollback rules

- If Windows CI fails after Phase 1a, revert only the handle-ownership commit;
  keep `SAFETY` comments and other correctness fixes.
- If the wide-character fix causes parser/screen regressions, keep the
  regression test and revert only the implementation. A red test with a good
  oracle is useful; deleting it is not.
- If the canonical `PtyError` struct breaks Python exception mapping, keep
  `PtyErrorKind` and continue additively.
- If perf work regresses p50 by more than 5%, revert that perf commit
  independently.
- Never remove benchmark correctness gates to make a run pass.
- Never reintroduce unsafe code without a `SAFETY` comment.
- Never renumber Python exception semantics or the `kind == "eof"` contract
  without an explicit compatibility plan.

---

## Non-goals

This plan deliberately does not include:

- splitting the crate into multiple workspace crates;
- changing the Python module layout;
- changing the wire format of `stitch-mux`;
- broad error-enum redesign in the same commit as safety fixes;
- compact cell storage in the same release as wide-character correctness;
- publishing a new version;
- comparing `stitch-pty` against tmux/RMUX/Zellij with unrelated benchmarks.

These can be follow-up plans after the P0/P1 items are green.

---

## Implementation status (2026-11-22)

Branch: `dev`  
Reviewed commit: `5a050d6`  
Status: implemented in the working tree; not committed by the implementation pass.

### Applied

| Phase | Items | Result |
|---|---|---|
| 0 | rustfmt, all-features clippy, unused dependencies, CI | applied |
| 1a | Windows child-handle ownership | applied with a narrow ownership split, see below |
| 1b | `SAFETY` comments | applied across Unix, Windows, and parser unsafe sites |
| 1c | wide-character cursor placement | applied with regression tests |
| 1d | production panic cleanup | applied to Unix async fd, Windows locks/loader, PyO3 conversion, keyboard parser |
| 2a | canonical error struct | applied additively as `PtyFailure`, see below |
| 2b | `Path` spawn APIs | applied through `spawn_path` / `spawn_path_with_env` |
| 2c | `Debug` impls and reasoned lint expectations | applied |
| 3 | module docs and `#[doc(inline)]` re-exports | applied |
| 4 | pre-sized styled-range serialization | applied |

Phase 5 (bulk movement of existing in-source tests to `tests/`) remains deferred.
It is a large mechanical change with no behavior impact and was kept separate
from correctness work.

### Notable implementation deviations

- **Windows handles:** instead of converting every `SendSyncHandle` to an owning
  wrapper, the implementation introduces `SharedHandle(Arc<HandleOwner>)` for
  `WinChildProcess` process/thread handles. This fixes the `clone_killer()`
  double-close risk without changing ConPTY handle ownership (`ClosePseudoConsole`)
  or temporary pipe-client handles. Raw, non-owning handles are now explicitly
  documented.
- **Canonical error name:** the Python module already exports an exception named
  `PtyError`. The canonical Rust struct is therefore named `PtyFailure` to avoid
  a Rust identifier collision while preserving the Python API. `PtyErrorKind`,
  `PtyResult`, and the `kind == "eof"` contract remain unchanged. New helpers:
  `PtyFailure::new`, `with_source`, `kind`, `backtrace`, `is_eof`, `is_timeout`,
  `is_closed`, `into_kind`, `TypedPtyResult`.
- **PyO3 `Winsize`:** `from_py_object` is retained because Python function
  arguments require `FromPyObject`. The clippy issue is resolved by a manual
  `FromPyObject` implementation that copies fields from a `PyRef`, avoiding
  generated `clone_on_copy` code.
- **Wide-character wrap policy:** a wide glyph that would span the right margin
  is pre-wrapped before drawing when autowrap is enabled. At the bottom-right
  corner it is reanchored to the last cells and parks the cursor at the pending
  wrap column, instead of overflowing past the screen edge.

### New and updated tests

- `test_shared_handle_closes_once` — mockable Windows close-count test.
- `test_draw_consecutive_width2_advances_cursor` — regression for the documented
  consecutive-wide-glyph bug.
- `test_draw_width2_prewraps_at_last_column` — margin pre-wrap oracle.
- `test_draw_width2_line_end` — updated to assert reanchoring, not overflow.
- `test_winsize_python_default_is_zeroed` — renamed after the manual
  `FromPyObject` change.
- `test_pty_failure_helpers_and_source`, `test_pty_failure_from_kind`,
  `test_typed_result_type_alias` — canonical error coverage.
- `spawn_path_accepts_os_native_paths` — public `Path` API integration test.
- `docs/TESTS.md` no longer lists the consecutive-width-two cursor bug as open.

### Final validation

```text
cargo fmt --all -- --check                              ok
cargo clippy --all-targets -- -D warnings               ok
cargo clippy --all-targets --all-features -- -D warnings ok
cargo check --all-targets                               ok
cargo check --all-targets --all-features                ok
cargo test --all-targets                                ok
  lib: 452 passed, 0 failed
cargo test --no-default-features --lib                  ok
  452 passed, 0 failed
cargo test --no-default-features --features regex --lib ok
  453 passed, 0 failed
cargo check --no-default-features --all-targets         ok
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --lib   ok
cargo tree --no-default-features -e normal -i pyo3      correctly reports no match
```

`cargo test --all-targets --all-features` still fails in this local Windows
environment with `STATUS_DLL_NOT_FOUND` while launching the generated Python
test binary. Compilation and clippy with all features pass; CI with Python
installed remains the authoritative all-features test run.

### Benchmark refresh

Command:

```sh
cargo run --release --manifest-path benchmark/Cargo.toml -- \
  --samples 30 --warmup 5 --bytes 1048576 \
  --out benchmark/results/parser.json
```

Final committed report (`benchmark/results/parser.json`):

```text
correctness.failures: 0
parser_native_ascii_feed1024 p50:   1036.83 MB/s
parser_native_ascii_feed65536 p50:  1071.80 MB/s
parser_native_unicode_feed1024 p50:  736.50 MB/s
parser_native_unicode_feed65536 p50: 777.75 MB/s
parser_native_csi_feed1024 p50:      303.71 MB/s
parser_native_csi_feed65536 p50:     287.59 MB/s
allocations per MiB:                 0
vte split-UTF-8 parity note:         unchanged
```

### Same-session A/B comparison

To separate code changes from host noise, the pre-change commit `5a050d6` was
checked out into a detached worktree and the same harness was run alternately
against the baseline and the remediation tree. Eighteen 30-sample reports were
collected per side (six runs in each of three balanced pair sets). Both sides
reported `correctness.failures = 0` on every run.

Using the median p50 across the 18 reports per side:

```text
parser_native_ascii_feed1024:   1021.7 -> 1036.6 MB/s   +1.5%
parser_native_ascii_feed65536:  1052.2 -> 1065.1 MB/s   +1.2%
parser_native_unicode_feed1024:  692.1 ->  735.0 MB/s   +6.2%
parser_native_unicode_feed65536: 710.1 ->  759.9 MB/s   +7.0%
parser_native_csi_feed1024:      310.2 ->  297.1 MB/s   -4.2%
parser_native_csi_feed16384:     305.6 ->  291.8 MB/s   -4.5%
parser_native_csi_feed65536:     302.0 ->  290.3 MB/s   -3.9%
parser_native_images_feed16384: 1160.0 -> 1163.1 MB/s   +0.3%

draw_mixed_width_feed1024:       120.2 ->  127.1 MB/s   +5.7%
draw_ascii_overwrite_feed65536:   71.5 ->   73.7 MB/s   +3.1%
scroll_soft_wrap:                 52.9 ->   54.8 MB/s   +3.6%
scroll_alt_screen:                31.4 ->   33.3 MB/s   +5.9%
scroll_partial_region:            37.1 ->   38.2 MB/s   +3.0%
erase_cursor_mixed:               32.0 ->   32.6 MB/s   +2.1%
project_styled_range:           6311.6 -> 6770.5 ops/s  +7.3%
```

Across all 62 measured lanes, one median p50 value fell more than 5%:
`scroll_numbered_200x60_cap2000_feed16384` at -6.9% on a low-throughput
(~3 MB/s) lane. The native CSI parser lanes landed at -3.9% to -4.5%, inside
the accepted noise band and with best-of-18 results within -5.8%. The working
conclusion is no material regression: ASCII, Unicode, images, wide-character
draw, scroll, erase, and projection lanes are neutral or improved; only the
CSI parser lane shows a small, bounded decrease.
