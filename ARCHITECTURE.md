# stitch-pty Architecture

## Design Goals

1. **Cross-platform**: Single codebase supporting Linux, macOS, Windows 10+
2. **Zero GIL contention**: All blocking operations release the Python GIL
3. **No zombie processes**: Platform-specific reaping strategies
4. **True PTY compliance**: POSIX `openpty` + Windows ConPTY with named pipes
5. **Terminal emulation**: Built-in VT100/VT220/xterm-compatible screen with scrollback
6. **Type safety**: Full mypy support, Rust memory safety

## Module Structure

```
src/
├── lib.rs                    # Module init, PyO3 pymodule registration
├── python_api.rs             # PyO3 bindings: PtyMaster, PtyChild, PtySession
├── terminal_api.rs           # PyO3 bindings: TerminalState
├── winsize.rs                # Winsize struct (POSIX + Windows COORD conversion)
├── errors.rs                 # PtyErrorKind enum + Python exception mapping
├── platform.rs               # Platform abstraction traits + dispatch
│   ├── PtyBackend, ChildBackend, ChildKiller traits
│   ├── ExitStatus, ProcessExit structs
│   ├── spawn_platform(), open_pty_platform() dispatch
├── platform_unix.rs          # POSIX implementation (Linux, macOS, *BSD)
│   ├── UnixPtyMaster, UnixChildProcess
│   ├── PtyPair, close_random_fds()
│   ├── fork/setsid/TIOCSCTTY/dup2/execvpe
├── platform_windows.rs       # Windows ConPTY implementation
│   ├── WinPtyBackend, WinChildProcess, SendSyncHandle
│   ├── NamedPipeServer connect() + ConPTY
│   ├── ArgvQuote algorithm + SafeAttributeList
├── terminal/                 # Embedded pyte_rs terminal emulation
│   ├── mod.rs                # Re-exports
│   ├── ansi_parser.rs        # ECMA-48 ANSI escape sequence state machine
│   ├── parser.rs             # Performer + Parser + Stream (pyte.Stream)
│   ├── screen.rs             # Screen buffer, Char, Cursor, SGR
│   ├── history.rs            # HistoryScreen with scrollback
│   ├── charsets.rs           # G0/G1 character set mappings
│   ├── control.rs            # C0/C1 control character constants
│   ├── escape.rs             # Escape sequence designators
│   ├── graphics.rs           # SGR attributes, color palettes
│   └── modes.rs              # Public/DEC terminal mode flags
```

## PTY Layer

### Platform Abstraction (`platform.rs`)

```rust
// src/platform.rs

#[async_trait::async_trait]
pub trait PtyBackend: Send + Sync {
    async fn read(&self, buf: &mut [u8]) -> io::Result<usize>;
    async fn write(&self, buf: &[u8]) -> io::Result<usize>;
    fn set_winsize(&self, winsize: Winsize) -> PtyResult<()>;
    fn get_winsize(&self) -> PtyResult<Winsize>;
    fn raw_handle(&self) -> RawFd;          // Unix
    fn raw_handle(&self) -> *mut c_void;    // Windows
    fn is_open(&self) -> bool;
}

#[async_trait::async_trait]
pub trait ChildBackend: ChildKiller + Send + Sync {
    fn pid(&self) -> u32;
    fn is_running(&self) -> bool;
    async fn wait(&self) -> Option<ProcessExit>;
    fn signal(&self, sig: i32) -> PtyResult<()>;
}

pub trait ChildKiller: Send + Sync {
    fn kill(&self) -> PtyResult<()>;
    fn clone_killer(&self) -> Box<dyn ChildKiller + Send + Sync>;
}
```

The Python API works with `Box<dyn PtyBackend>` / `Box<dyn ChildBackend>`,
so the same Python code runs on all platforms without changes.

### Exit Status (`platform.rs`)

```rust
pub struct ExitStatus {
    code: u32,
    signal: Option<String>,
}
// .success() → bool, .exit_code() → u32, .signal() → Option<&str>

pub struct ProcessExit {
    pub pid: u32,
    pub exit_code: Option<i32>,
    pub signal: Option<i32>,
}
// .to_exit_status() → ExitStatus (legacy bridge)
```

### POSIX Implementation (`platform_unix.rs`)

**PTY Lifecycle:**
1. `openpty()` → create master/slave pair, set `O_NONBLOCK` on master
2. `fork()` → child calls `setsid()`, `TIOCSCTTY`, `dup2()` ×3, `execvpe()`
3. Parent wraps master FD in `tokio::io::AsyncFd` for async I/O
4. Background task polls `waitpid(WNOHANG)` every 50ms via `tokio::spawn`

**Async I/O:** Uses `tokio::io::AsyncFd` with `try_io` pattern:
```rust
loop {
    let mut guard = self.async_fd.readable().await?;
    match guard.try_io(|inner| {
        libc::read(*inner.get_ref(), buf.as_mut_ptr(), buf.len())
    }) {
        Ok(result) => return result,
        Err(_would_block) => continue,
    }
}
```

**Signal Forwarding:** All signals sent to process group (`-pgid`), not just PID:
```rust
let pgid = Pid::from_raw(-pid.as_raw());
kill(pgid, SIGTERM)?;  // Entire process group receives signal
```

**FD Leak Prevention:** `close_random_fds()` closes all FDs > 2 via `/dev/fd` listing (critical for macOS Big Sur / Linux gnome/mutter).

**Signal Disposition Reset:** Before `execvpe()`, child resets `SIGCHLD`, `SIGHUP`, `SIGINT`, `SIGTERM`, `SIGALRM` to `SIG_DFL` and clears signal mask.

### Windows Implementation (`platform_windows.rs`)

**Key improvement over original stitch-pty:** Uses **Tokio NamedPipes** for true async I/O
instead of `spawn_blocking` on anonymous pipes.

**ConPTY Dynamic Loading:**
```rust
type PfnCreatePseudoConsole = extern "system" fn(COORD, *mut c_void, *mut c_void, u32, *mut HANDLE) -> i32;
// Loaded from kernel32.dll via GetProcAddress at runtime
static mut CREATE_PSEUDO_CONSOLE: Option<PfnCreatePseudoConsole> = None;
static mut CONPTY_LOADED: bool = false;
```
Graceful degradation on older Windows (returns error if ConPTY unavailable).

**PTY Lifecycle:**
1. Create Tokio named pipe server for each direction (input + output)
2. Bridge named pipe clients to Win32 `HANDLE`s for ConPTY
3. `CreatePseudoConsole(size, read_handle, write_handle)`
4. `CreateProcessW` with `PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE` attached via
   `SafeAttributeList` (proper `STARTUPINFOEXW` + attribute list)
5. Async I/O via `tokio::net::windows::named_pipe::NamedPipeServer`
6. Background task polls `GetExitCodeProcess` every 50ms for exit detection

**Pipe naming:** Each pipe gets a unique name:
`\\.\pipe\stitch-pty-{pid}-{counter}` — prevents collisions across PTY instances.

**SafeAttributeList wrapper:** Properly manages the Win32 PROC_THREAD_ATTRIBUTE_LIST
lifecycle (Initialize → Update → Delete), avoiding the incomplete ConPTY attachment
from the original stitch-pty.

**ArgvQuote algorithm:** Proper Windows command line quoting:
- Splits on spaces/tabs/newlines/vertical tabs
- Escapes backslashes before quotes: `"` → `\\"`
- Doubles trailing backslashes before quotes

**Startup Handshake:** After `CreateProcessW`, sends DSR reply to conhost:
```rust
let _ = input_server.write_all(b"\x1b[1;1R").await;  // DSR cursor position
```
This answers conhost's startup query so it begins relaying child output.

**SendSyncHandle:** Wrapper around `HANDLE` implementing `Send + Sync` + `Clone`,
allowing Windows non-Send types to live across `.await` points.

## Terminal Emulation Layer (`terminal/`)

Embedded from [pyte_rs](https://github.com/python-pyte/pyte). Provides:

### `Parser` — ANSI Stream Parser

Wraps the internal `ansi_parser::Parser` (ECMA-48 state machine) to translate
ANSI escape sequences into `Screen` method calls via the `Performer` trait.

```rust
let mut parser = Parser::new();
let mut screen = Screen::new(80, 24);
parser.feed(&mut screen, b"\x1b[31mHello\x1b[0m");
```

**State Machine States:**
| State | Enters on | Exits on |
|-------|-----------|----------|
| `Ground` | Default | ESC (`\x1b`), C0 control |
| `CsiEntry` | ESC `[` | Final byte (`@`–`~`) |
| `CsiParam` | `0`–`9`, `;`, `:` | Final byte |
| `CsiIntermediate` | ` `–`/` | Final byte |
| `CsiIgnore` | `>`–`?` in param | Final byte |
| `OscString` | ESC `]` | BEL, ST, ESC |
| `DcsEntry` | ESC `P` | Hook byte |
| `DcsPassthrough` | After hook | `\x9c`, ESC, SUB, CAN |
| `Escape` | ESC | Final byte / intermediate |
| `EscapeIntermediate` | ESC + ` `–`/` | Final byte |

**Parser Limits:**
- Max params: 32 (sub-parameter groups)
- Max intermediates: 2 bytes
- Max OSC params: 16
- Max OSC raw bytes: 1024
- UTF-8 partial byte buffer: 4 bytes

### `Performer` — Parser → Screen Bridge

Implements `Perform` trait to translate parser events into `Screen` calls:
```rust
impl Perform for Performer {
    fn print(&mut self, c: char) { /* → Screen.draw */ }
    fn execute(&mut self, byte: u8) { /* CR/LF/Tab/BS/ShiftIn/ShiftOut */ }
    fn osc_dispatch(&mut self, params, bell_term) { /* OSC 0/1/2; title/icon */ }
    fn csi_dispatch(&mut self, params, intermediates, action) { /* → Screen.csi_dispatch */ }
    fn esc_dispatch(&mut self, intermediates, ignore, byte) { /* ESC 7/8/Z/H/D/E/M/#8/=/> */ }
}
```

### `Screen` — Terminal Buffer

Manages a 2D grid of `Char` cells, cursor position, tab stops, character sets,
scroll margins, and dirty tracking. Supports:

- **SGR**: 8-color, 256-color, true color (24-bit RGB), aixterm bright colors
- **Cursor**: movement, save/restore, style + blink (DECSCUSR, stored on `Screen` as `cursor_style` / `cursor_blink`)
- **Modes**: public ANSI (IRM, LNM) + private DEC (DECOM, DECAWM, DECCOLM, etc.)
- **Character sets**: G0/G1 with DEC Special line drawing
- **Scroll region**: configurable top/bottom margins; a full-screen `scroll_up` evicts the top line into `scrolled_off` for scrollback capture
- **Alternate screen**: `?1049`/`?1047`/`?47` park the primary buffer in `saved_buffer` and swap in a fresh one (`1049` also saves/restores the cursor); scrollback capture is suppressed while active
- **Kitty Keyboard Protocol**: mode push/pop/replace
- **Unicode**: width-1 and width-2 characters, combining marks, CJK, emoji

**Char Cell:**
```rust
pub struct Char {
    pub data: String,      // Display text
    pub fg: String,        // "default", ANSI name, or 6-hex RGB
    pub bg: String,        // Same format as fg
    pub bold, dim, italics, underscore, blink, reverse, hidden, strikethrough: bool
}
```

**Cursor:**
```rust
pub struct Cursor {
    pub x, y: usize,       // 0-indexed position
    pub attrs: Char,       // Current drawing attributes
    pub hidden: bool,
    pub stack: Vec<(usize, usize, Char, Modes)>,  // Save/restore
}
```

**Modes:**
| Mode | Type | Default | Description |
|------|------|---------|-------------|
| `IRM` (4) | public | off | Insert mode |
| `LNM` (20) | public | on | Line feed = newline (auto CR) |
| `DECCKM` (1) | private | off | Application keypad |
| `DECCOLM` (3) | private | off | 132-column mode |
| `DECOM` (6) | private | off | Origin mode (relative to margins) |
| `DECAWM` (7) | private | on | Auto-wrap |
| `DECTCEM` (25) | private | on | Text cursor visible |
| `DECSCNM` (5) | private | off | Reverse video |
| `X10_MOUSE` (1000) | private | off | Mouse tracking |
| `SGR_MOUSE` (1006) | private | off | SGR mouse encoding |
| Alt screen (1049/1047/47) | private | off | Alternate screen buffer (swapped; 1049 also saves/restores the cursor) |
| `BRACKETED_PASTE` (2004) | private | off | Paste mode |

`Modes` also provides convenience queries used by the Python binding:
`has_private(mode)` (named modes via bitflags, everything else via a
`HashSet<u16>`), plus `mouse_protocol()` (highest active mouse mode, or 0),
`sgr_mouse()`, and `is_alt_screen()`. Because the CSI `?…h/l` dispatch routes
*every* private-mode number through `set_mode`/`reset_mode`, modes the engine
doesn't act on (1006) are still recorded and thus queryable. (For the
alternate screen, `Screen::alt_screen` is the authoritative state — what the
buffer actually did — and is what `TerminalState.alt_screen` reports;
`Modes::is_alt_screen()` reflects the raw mode bit.)

### `HistoryScreen` — Scrollback Buffer

Extends `Screen` with a fixed-capacity scrollback history:

```rust
pub struct HistoryScreen {
    inner: Screen,
    history: Vec<Vec<Char>>,
    scrollback_lines: usize,  // 0 = unlimited
}
```

| Method | Description |
|--------|-------------|
| `feed(data)` | Feed raw bytes (parses ANSI, updates screen, captures scroll-off into history) |
| `display()` | Full display (history + visible) |
| `visible_display()` | Visible screen only |
| `history_display()` | Scrollback only |
| `scroll_up_with_history(rows)` | Push scrolled lines to history |
| `scroll_down_with_history(rows)` | Pop lines from history |
| `total_lines()` | history_len + visible_lines |
| `absolute_cursor()` | `(x, history_len + on_screen_y)` |
| `styled_viewport()` | Full buffer as styled cells |
| `styled_range(start, count)` | Styled cells for absolute rows `[start, start+count)`, clamped — serialize only the on-screen window (O(window), not O(total)) |
| `dirty()` | Modified row indices (BTreeSet) |
| `reset()` | Reset terminal + clear history |
| `resize(lines, cols)` | Resize; on shrink the top overflow is pushed to history; grow pads at the bottom (no pull) |
| `columns()` / `lines()` | Current dimensions |
| `history_size()` / `scrollback_lines()` | Capacity info |
| `set_scrollback_lines(n)` | Set capacity (trims excess) |
| `cursor()` / `mode()` / `title()` | Accessors |
| `cursor_style()` / `cursor_blink()` / `cursor_shape()` | DECSCUSR cursor state (`cursor_shape()` returns `"block"`/`"underline"`/`"bar"`) |
| `alt_screen()` | Whether the alternate screen buffer is active |

**Scrollback capture:** `feed()` drives the parser against the inner `Screen`,
whose `scroll_up` (on a full-screen scroll, i.e. no DECSTBM margins) evicts the
top line into `Screen::scrolled_off`. After parsing, `feed()` drains that buffer
into the scrollback history. Region scrolls inside a margin (pagers, status
lines) are intentionally **not** captured. On `resize`, shrinking pushes the
top overflow into history (after first dropping unused blank rows below the
cursor). Growing pads at the bottom and deliberately does **not** pull lines
back out of history: the shell redraws on SIGWINCH and would clear pulled rows,
so a subsequent shrink would push blanks back — repeated fast resizes would then
drain real scrollback. Pushing-only keeps scrollback stable across resizes.

**`styled_viewport()` / `styled_range()` Output:**
```rust
Vec<Vec<(text: String, fg: String, bg: String, attrs: u8)>>
```
Attrs bitmask: bit 0=bold, 1=dim, 2=italics, 3=underscore, 4=blink, 5=reverse, 6=hidden, 7=strikethrough.

`styled_viewport()` returns the entire buffer (history + visible); `styled_range(start, count)` returns only absolute rows `[start, start+count)`, clamped to `[0, total_lines())` — rows below `history_len` come from scrollback, the rest from the visible screen, with identical cell encoding. A front-end that only paints what's on screen should call `styled_range` with the current viewport window instead of serializing the whole scrollback every frame: cost becomes O(window) rather than O(total_lines), which matters once scrollback is deep.

### `Stream` — High-Level Handler

Equivalent to `pyte.Stream`: attach a screen, feed bytes, detach screen.

## GIL Management

PyO3 native async automatically releases the GIL at `.await`:

```rust
async fn read<'py>(&self, py: Python<'py>, size: usize) -> PyResult<Bound<'py, PyAny>> {
    let mut buf = vec![0u8; size];
    let n = self.inner.read(&mut buf).await?;  // ← GIL released
    buf.truncate(n);
    Ok(buf)  // Vec<u8> → PyBytes (GIL reacquired)
}
```

This works identically for both POSIX (`AsyncFd`) and Windows (`NamedPipeServer`) paths.

**GIL Pattern:**
1. Method called from Python (GIL held)
2. `future_into_py` creates async future (GIL released)
3. Tokio I/O awaits (GIL released)
4. On completion, GIL reacquired to convert Rust value → Python object

## Memory Safety

### FD/Handle Ownership

**POSIX:**
```
PtyPair (owns master_fd + slave_fd)
    ├── master_fd → UnixPtyMaster (via mem::forget)
    │   └── Drop: close(fd)
    └── slave_fd → child process (via fork)
        └── Child closes on exec, parent closes after fork
```

**Windows:**
```
WinPtyBackend (owns HPCON + NamedPipeServer handles)
    ├── input_pipe → NamedPipeServer (Drop: drop)
    ├── output_pipe → NamedPipeServer (Drop: drop)
    └── conpty → ConPTY (Drop: ClosePseudoConsole)
```

### RAII Guarantees

| Type | Drop Behavior |
|------|---------------|
| `UnixPtyMaster` | `close(raw_fd)` |
| `WinPtyBackend` | `ClosePseudoConsole` + drop named pipes |
| `UnixChildProcess` | `kill(SIGKILL)` if running |
| `WinChildProcess` | `TerminateProcess` if running; `CloseHandle` |
| `SafeAttributeList` | `DeleteProcThreadAttributeList` (scoped block) |
| `PtyPair` | `close(slave_fd)` + `close(master_fd)` |

### Send-Safe Windows Types

`SendSyncHandle` wraps `HANDLE` with `Send + Sync + Clone`, allowing Windows
non-Send types to cross `.await` boundaries. All non-Send locals (conpty,
`STARTUPINFOEXW`, `PROCESS_INFORMATION`, attribute list) are scoped to a block
that ends before the first `.await` after `CreateProcessW`.

## Error Propagation

```
POSIX errno / Windows HRESULT
    │
    ▼
nix::Error / windows::core::Error
    │
    ▼
PtyErrorKind (thiserror enum)
    │
    ▼
IntoPy<PyErr> (match on variant)
    │
    ▼
Python exception (PtyError / ProcessError / IOError / PyOSError / PyIOError)
```

**Error Mapping:**
| PtyErrorKind | Python Exception |
|--------------|------------------|
| `OpenFailed`, `OperationFailed`, `Closed`, `PlatformNotSupported` | `PtyError` |
| `ForkFailed`, `ProcessNotRunning` | `ProcessError` |
| `InvalidHandle`, `WinsizeFailed`, `BufferOverflow`, `Timeout` | `IOError` |
| `SignalError`, `WindowsError` | `PyOSError` |
| `AsyncIo` | `PyIOError` |

## Python API (`python_api.rs`)

### `PtyMaster` — Raw PTY I/O

| Method | Description |
|--------|-------------|
| `read(size)` | Read up to `size` bytes |
| `read_timeout(size, timeout)` | Read with timeout (raises `IOError` on timeout) |
| `write(data)` | Write bytes, returns count |
| `write_all(data)` | Write all bytes (handles partial writes) |
| `set_winsize(Winsize)` | Set window size |
| `get_winsize()` | Get current size |
| `raw_fd()` | Raw FD (-1 on Windows) |
| `is_open()` | PTY still open? |

### `PtyChild` — Child Process Management

| Method | Description |
|--------|-------------|
| `pid` | Child PID |
| `is_running` | Process still running? |
| `wait()` | Wait for exit, returns `(pid, exit_code, signal, core_dumped)` |
| `terminate(grace_period)` | SIGTERM → wait → SIGKILL fallback |
| `kill()` | Force kill |
| `interrupt()` | Send Ctrl+C (SIGINT) |
| `send_signal(num)` | Send signal number |

### `PtySession` — Combined PTY + Child

Delegates to `PtyMaster` + `PtyChild`. Adds:
| Method | Description |
|--------|-------------|
| `resize(rows, cols)` | Convenience wrapper |
| `is_alive` | Child still running? |

### `Winsize` — Terminal Size

| Field | Type | Description |
|-------|------|-------------|
| `rows` | `u16` | Number of rows |
| `cols` | `u16` | Number of columns |
| `xpixel` | `u16` | Pixel width (unused) |
| `ypixel` | `u16` | Pixel height (unused) |

### `TerminalState` — Terminal Emulation

| Method | Description |
|--------|-------------|
| `__init__(cols, lines, scrollback)` | Create terminal state |
| `feed(data)` | Feed raw bytes |
| `display()` | Full display (history + visible) |
| `visible_display()` | Visible screen only |
| `history_display()` | Scrollback only |
| `dirty()` | Modified row indices |
| `resize(lines, cols)` | Resize buffer |
| `reset()` | Reset terminal + clear history |
| `cursor_x` / `cursor_y` | Cursor position (0-indexed, visible area) |
| `title` | Window title (from OSC) |
| `history_size` | Current scrollback line count |
| `scrollback_lines` | Scrollback capacity |
| `set_scrollback_lines(n)` | Set capacity (trims excess) |
| `styled_viewport()` | Full buffer as styled cells |
| `styled_range(start, count)` | Styled cells for absolute rows `[start, start+count)` (clamped) — window serialization, O(window) not O(total) |
| `total_lines()` | Total lines = history + visible |
| `absolute_cursor()` | `(x, history_len + on_screen_y)` |
| `app_cursor` | DECCKM (`?1`) active — application cursor keys (`bool`) |
| `cursor_visible` | DECTCEM (`?25`) — cursor visible (`bool`) |
| `cursor_shape` | DECSCUSR shape: `"block"` / `"underline"` / `"bar"` (`str`) |
| `cursor_blink` | DECSCUSR — blinking vs. steady (`bool`) |
| `mouse_proto` | Highest active mouse mode: `1003`/`1002`/`1000`/`0` (`int`) |
| `sgr_mouse` | SGR mouse encoding (`?1006`) active (`bool`) |
| `bracketed_paste` | Bracketed paste (`?2004`) active (`bool`) |
| `alt_screen` | Alternate screen (`?1049`/`?1047`/`?47`) active (`bool`) |

The mode-flag getters read state the parser already tracks (private modes live
in `Modes`, cursor shape/blink on `Screen`), so a front-end can drive cursor
rendering, key encoding, mouse forwarding, and paste wrapping without
re-scanning the raw byte stream.

## Front-end Integration

This section documents the recommended pattern for building an interactive GUI
on top of `PtySession`, as implemented by the reference PySide6 front-end
(`terminal_emulator.py`). The module deliberately stays UI-agnostic; everything
below lives in the front-end, but the design is shaped by how the module
exposes terminal state.

### Threading model

PTY I/O is async and the terminal state machine is mutated as bytes are read, so
both run on a dedicated worker thread that owns an asyncio event loop. The GUI
runs on the main thread. They communicate one way — worker → GUI — by handing
the GUI immutable *frame* snapshots (never the live `TerminalState`, which the
worker keeps mutating). Input flows GUI → worker by scheduling a write onto the
loop.

```
┌──────────────── GUI thread ───────────────┐      ┌──── worker thread (asyncio) ────┐
│  paint(window)  ◄── frame snapshot ──────  │ ◄──  │  read → feed(TerminalState)     │
│  key/mouse      ──► schedule write ───────►│ ──►  │  render pump → emit frame       │
│  scroll/resize  ──► set_viewport ─────────►│ ──►  │  (loop owns the PtySession)     │
└────────────────────────────────────────────┘      └─────────────────────────────────┘
```

A subtlety worth calling out: the worker thread runs `loop.run_forever()`, not a
Qt event loop, so cross-thread Qt slots delivered to worker-thread objects would
never run. Signals whose handlers only flip a few fields on the worker (input,
viewport updates) are therefore connected with `DirectConnection` and execute on
the GUI thread, mutating plain attributes that the worker reads (atomic under the
GIL). Frame delivery worker → GUI uses an ordinary queued signal.

### Render pump (decouple rendering from reading)

Rendering must **not** be driven from the read loop. On Windows ConPTY an idle
`read_timeout` does not reliably time out — it parks waiting for the next byte —
so if the final frame after a command (the new prompt) were flushed only when a
read returns, it would not appear until the next keystroke produced data. This
manifests as "the prompt isn't shown until I type" and as input that feels one
keystroke behind.

The fix is a *render pump*: a `loop.call_later` callback that reschedules itself
every frame interval (~16 ms) and emits a frame whenever there is unrendered
output. It runs on the event loop independently of the read coroutine, so it
fires even while a read is parked. The read loop's only rendering job is to set
a `dirty` flag; the pump clears it on emit. This naturally rate-limits to the
frame interval during heavy output and guarantees the last frame is flushed
within one interval after output settles.

```python
def _pump(self):
    if self._dirty:
        self._emit_frame()      # serialize + emit; clears _dirty
    self._handle = loop.call_later(1/60, self._pump)
```

### Windowed frames

A naive front-end serializes the whole buffer with `styled_viewport()` every
frame — O(total scrollback). Once history is deep this dominates and makes
typing sluggish. Instead, each frame carries only the styled rows the view can
actually display, obtained with `styled_range(start, count)`:

- **`rows`** — styled cells for the on-screen window only (plus a screenful of
  margin each side so small scrolls don't need a round-trip), via `styled_range`.
- **`window_start`** — absolute index of `rows[0]`; the painter maps an absolute
  row `r` to `rows[r - window_start]` and skips rows outside the window.
- **`text_rows`** — the full buffer as plain text (`display()`), used for search,
  selection, and URL detection across the entire scrollback. Plain strings are
  far cheaper than styled cells, so this stays affordable even when full.
- **cursor** — reported in absolute coordinates via `absolute_cursor()`, plus
  `total_lines()` so the scrollbar range is correct.

Per-frame styling cost becomes O(window) instead of O(total_lines); the only
remaining full-buffer work is the cheap plain-text `text_rows` (which can be made
conditional on search/selection being active if even that matters).

### Viewport handshake

The worker needs to know *which* absolute window to serialize. The view reports
its viewport (whether it is following the bottom, its absolute top row, and its
row count) whenever the user scrolls or the grid resizes:

- **Following** (stuck to the bottom, the common case): the view sends a
  "following" flag and the worker computes the window from `total_lines()` each
  frame, so steadily growing output keeps the bottom in view without per-frame
  chatter.
- **Scrolled up**: the view sends its absolute top row; the worker serializes
  that fixed window, so new background output grows the scrollbar range while the
  user's position stays put.

Marking the state dirty on a viewport change lets the pump re-render the new
window within one frame interval, so scrolling stays responsive.

### Graceful capability fallback

`styled_range` is newer than the rest of the binding, so the front-end probes
for it once (`hasattr`) and falls back to `styled_viewport()` with
`window_start = 0` if it is absent. This lets the same Python front-end run
against an older, un-rebuilt module (full-buffer rendering, no crash) and switch
to the windowed fast path automatically once the module is rebuilt.

## Testing Strategy

Tests use `pytest.mark.skipif` to skip platform-incompatible tests:

```python
IS_WINDOWS = platform.system() == "Windows"

@pytest.mark.skipif(IS_WINDOWS, reason="SIGWINCH not available on Windows")
async def test_resize_forwarding(self):
    ...
```

CI runs full test suites on:
- `ubuntu-latest` (POSIX path)
- `macos-latest` (POSIX path)
- `windows-latest` (ConPTY path)

## Deployment Pipeline

```
Developer push
    │
    ▼
GitHub Actions
    ├── test-linux (Python 3.10-3.13, x86_64)
    ├── test-macos (Python 3.10-3.13, x86_64 + ARM64)
    ├── test-windows (Python 3.10-3.13, x86_64)
    │
    └── build-wheels
        ├── Linux x86_64 (manylinux_2_28)
        ├── Linux ARM64 (manylinux_2_28)
        ├── macOS x86_64
        ├── macOS ARM64
        └── Windows x86_64
            │
            ▼
        maturin build --release
            │
            ▼
        dist/*.whl
            │
            ▼
        PyPI publish (trusted publishing)
```

## Known Limitations

### Windows

1. **ConPTY requires Windows 10 version 1809+**. Older Windows falls back to
   `CreateProcess` with `CREATE_NEW_CONSOLE` (limited PTY functionality).

2. **No Unix signals**: Windows uses `GenerateConsoleCtrlEvent` for Ctrl+C
   and `TerminateProcess` for kill. Custom signals are not supported.

3. **Window resize**: `ResizePseudoConsole` changes the buffer size but does
   not send a signal to the child process. Applications must poll for size.

4. **Exit info**: Only `exit_code` available; no `signal` or `core_dumped`.

### POSIX

1. **No Windows-style console API**: Cannot use Windows console functions
   on POSIX systems (obviously).

2. **fork() limitations**: Heavy use of threads in the parent process before
   `fork()` can cause deadlocks due to fork-safety issues in libraries.
   Current implementation forks immediately after PTY creation.

### Terminal Emulation

1. **DCS sequences**: Partially supported (hook/passthrough/unhook tracked but not rendered)
2. **Private modes**: Most DEC private modes supported; extended modes use HashSet
3. **Mouse protocols**: Mode state is tracked and exposed (`mouse_proto`, `sgr_mouse`), but input event encoding is not implemented in the engine — a front-end must encode mouse reports itself
4. **Alternate screen**: `?1049`/`?1047`/`?47` swap to a real alternate buffer (cursor saved/restored for `1049`, scrollback suppressed while active). The alternate buffer is recreated fresh on each entry rather than persisted across switches
5. **Subparameter parsing**: CSI subparameters via `:` treated as param separators (not subparams) — differs from vte crate
6. **Wide character handling**: Width-2 characters (CJK, emoji) handled; combining marks overwrite previous cell
7. **OSC passthrough**: OSC 3 (set icon name) not implemented
