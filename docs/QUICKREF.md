# stitch-pty Quick Reference

## Installation

```bash
pip install stitch-pty
```

## Quick Start

```python
import asyncio
from stitch_pty import spawn

async def main():
    # Spawn a shell
    session = await spawn("bash", ["-i"])

    # Read output (auto-fed into terminal)
    data = await session.read(4096)
    print(session.display)        # visible screen as list[str]
    print(session.scrollback)     # scrollback as list[str]
    print(session.full_display)   # history + visible

    # Write to PTY
    await session.write(b"echo hello\n")

    # Resize
    session.resize(50, 120)

    # Shutdown
    await session.terminate()

asyncio.run(main())
```

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│  Python layer (python/stitch_pty/__init__.py)               │
│  PtySession │ PtyMaster │ PtyChild │ PtyError               │
├─────────────────────────────────────────────────────────────┤
│  PyO3 bindings (python_api.rs + terminal_api.rs)            │
│  (gated by `python` feature flag)                           │
│  PtyMaster │ PtyChild │ PtySession │ TerminalState          │
├─────────────────────────────────────────────────────────────┤
│  Terminal emulation (terminal/ — embedded pyte_rs)          │
│  Parser → Screen → HistoryScreen (scrollback)               │
├─────────────────────────────────────────────────────────────┤
│  Platform layer (platform_unix.rs / platform_windows.rs)    │
│  PtyBackend │ ChildBackend │ ChildKiller traits             │
└─────────────────────────────────────────────────────────────┘
```

## API Summary

### `spawn(program, args=None, env=None, winsize=None) → PtySession`

Async. Spawns a child process in a PTY, auto-feeds output to terminal.

### `open_pty(winsize=None) → PtyMaster`

Async. Opens a raw PTY pair without spawning a child.

### `Winsize(rows, cols, xpixel=0, ypixel=0)`

| Property | Type   |
|----------|--------|
| `rows`   | `u16`  |
| `cols`   | `u16`  |
| `xpixel` | `u16`  |
| `ypixel` | `u16`  |

### `ExitStatus` (returned by `await wait(timeout=None)`)

Frozen dataclass. `await wait()` returns `None` if the child was already reaped, or raises `TimeoutError` if the child does not exit within `timeout` seconds.

| Field | Type | Description |
|-------|------|-------------|
| `pid` | `int` | PID of the process that exited |
| `exit_code` | `int \| None` | Exit code, or `None` if killed by a signal |
| `signal` | `int \| None` | Signal number, or `None` if exited normally |
| `core_dumped` | `bool` | Whether a core dump was produced |

### `PtySession` — primary interface (combines PTY I/O + terminal emulation)

| Method | Description |
|--------|-------------|
| `await read(size=4096)` | Read PTY output (auto-feeds terminal) |
| `await read_timeout(size, timeout)` | Read with timeout (raises `IOError` on timeout) |
| `await write(data)` | Write bytes to PTY, returns bytes written |
| `await write_all(data)` | Write all bytes (handles partial writes) |
| `resize(rows, cols)` | Resize terminal (forwards to PTY backend) |
| `await terminate(grace_period=5.0)` | SIGTERM → wait → SIGKILL fallback |
| `kill()` | Force kill |
| `interrupt()` | Send Ctrl+C (SIGINT) |
| `send_signal(num)` | Send signal number |
| `await wait(timeout=None)` | Wait for exit, returns `ExitStatus \| None`. Raises `TimeoutError` on timeout. |
| `await interact(input_data=None, timeout=None)` | Write input, read until EOF |
| `await expect(pattern, timeout=30.0)` | pexpect-style: read until pattern found |
| `await read_all(timeout=1.0)` | Read all output until timeout |
| `is_alive` | Process still running? |
| `terminal` | Raw `TerminalState` (for direct access) |
| `display` | Visible screen (`list[str]`) |
| `scrollback` | Scrollback history (`list[str]`) |
| `full_display` | History + visible (`list[str]`) |
| `raw_output` | All raw bytes read (unparsed, `bytes`) |

### `PtyMaster` — raw PTY I/O (no terminal emulation)

| Method | Description |
|--------|-------------|
| `await read(size)` | Read PTY output |
| `await read_timeout(size, timeout)` | Read with timeout |
| `await write(data)` | Write bytes, returns count |
| `await write_all(data)` | Write all bytes |
| `set_winsize(rows, cols, xpixel, ypixel)` | Set window size |
| `get_winsize()` | Get current size |
| `fd` | Raw file descriptor (Unix) / `-1` (Windows) |
| `is_open` | PTY still open? |

### `PtyChild` — child process management

| Property/Method | Description |
|-----------------|-------------|
| `pid` | Child process PID |
| `is_running` | Process still running? |
| `await wait()` | Wait for exit, returns `ExitStatus \| None` |
| `await terminate(grace_period)` | SIGTERM → wait → SIGKILL fallback |
| `kill()` | Force kill |
| `interrupt()` | Send Ctrl+C |
| `send_signal(num)` | Send signal number |
| `await wait(timeout=None)` | Wait for exit, returns `ExitStatus \| None`. Raises `TimeoutError` on timeout. |

### `TerminalState` — terminal emulation state

| Method | Description |
|--------|-------------|
| `feed(data)` | Feed raw bytes (parses ANSI, updates screen) |
| `display()` | Full display (history + visible, `list[str]`) |
| `visible_display()` | Visible screen only (`list[str]`) |
| `history_display()` | Scrollback only (`list[str]`) |
| `dirty()` | Modified row indices (`list[int]`) |
| `resize(lines, cols)` | Resize screen buffer |
| `reset()` | Reset terminal + clear history |
| `styled_viewport()` | Full buffer as styled cells: `list[list[(text, fg, bg, attrs_bitmask)]]` |
| `total_lines()` | Total lines = history + visible |
| `absolute_cursor()` | `(x, history_len + on_screen_y)` |

| Property | Description |
|----------|-------------|
| `cursor_x` | Cursor column (0-indexed, visible area) |
| `cursor_y` | Cursor row (0-indexed, visible area) |
| `title` | Window title (from OSC sequences) |
| `history_size` | Current scrollback line count |
| `scrollback_lines` | Scrollback capacity |
| `set_scrollback_lines(n)` | Set capacity (trims excess) |

## Error Types

| Exception | Inherits | On |
|-----------|----------|----|
| `PtyError` | `Exception` | PTY open/operation failures |
| `ProcessError` | `PtyError` | Spawn/kill failures |
| `IOError` | `PtyError` | I/O errors, timeouts, winsize failures |

These are the native exception classes registered by the Rust core and
re-exported from `stitch_pty`, so `except stitch_pty.IOError` / `ProcessError`
matches errors raised from native code. (Note: `stitch_pty.IOError` shadows the
builtin `IOError`/`OSError` within this namespace.)

## Platform Differences

| Feature | POSIX | Windows |
|---------|-------|---------|
| PTY backend | `openpty(3)` + `fork()` + `execvpe()` | ConPTY (kernel32.dll) + tokio NamedPipes |
| I/O model | `tokio::AsyncFd` over raw FDs | `tokio::NamedPipeServer` (IOCP) |
| Signals | Full via `nix::sys::signal` (SIGINT, SIGTERM, SIGKILL, SIGWINCH) | Ctrl+C (`GenerateConsoleCtrlEvent`), SIGTERM/KILL → `TerminateProcess` |
| Resize signal | `SIGWINCH` forwarded to process group via `tcgetpgrp` | No signal; `ResizePseudoConsole` only |
| Exit info | `exit_code` + `signal` + `core_dumped` | `exit_code` only |
| Pipe plumbing | Single FD pair (master/slave) | Two named pipes (input/output) + `connect()` to arm IOCP |
| Startup handshake | N/A | DSR reply (`\x1b[1;1R`) to conhost before child output flows |
| Close random FDs | `close_random_fds()` (macOS/Linux FD leak fix) | N/A |

## Terminal Emulation (pyte_rs)

### Character Cell (`Char`)

| Field | Type |
|-------|------|
| `data` | `str` (character text) |
| `fg` | `str` — `"default"`, ANSI name, or 6-hex RGB |
| `bg` | `str` — same format as fg |
| `bold`, `dim`, `italics`, `underscore`, `blink`, `reverse`, `hidden`, `strikethrough` | `bool` |

### Cursor

| Field | Type |
|-------|------|
| `x`, `y` | `usize` (0-indexed) |
| `attrs` | `Char` (current drawing attributes) |
| `hidden` | `bool` |
| `stack` | `Vec<(x, y, attrs, modes)>` (save/restore) |

### Screen Modes

| Mode | Type | Default | Description |
|------|------|---------|-------------|
| `IRM` | public | off | Insert mode |
| `LNM` | public | on | Line feed = newline (auto CR) |
| `DECAWM` | private | on | Auto-wrap |
| `DECOM` | private | off | Origin mode (relative to margins) |
| `DECCOLM` | private | off | 132-column mode |
| `DECTCEM` | private | on | Text cursor visible |
| `DECSCNM` | private | off | Reverse video |
| `DECCKM` | private | off | Application keypad |
| `BRACKETED_PASTE` | private | off | Mouse paste mode |
| Mouse modes | private | off | 1000/1002/1003/1004 |

### SGR Color Formats

| Format | Code | Example |
|--------|------|---------|
| 8 ANSI | 30–37, 40–47 | `CSI 31 m` → red fg |
| Bright (aixterm) | 90–97, 100–107 | `CSI 91 m` → bright red |
| 256-color | `CSI 38;5;n m` | `CSI 38;5;196 m` |
| 24-bit RGB | `CSI 38;2;R;G;B m` | `CSI 38;2;255;0;0 m` |
| Subparam RGB | `CSI 38:2:R:G:B m` | Colon-separated variant |

### Key ANSI Sequences

| Sequence | Action |
|----------|--------|
| `CSI n A/B/C/D` | Cursor up/down/left/right |
| `CSI r;H` | Cursor position (1-indexed) |
| `CSI J/K` | Erase display/line |
| `CSI S/T` | Scroll up/down |
| `CSI m` | SGR (attributes/colors) |
| `CSI h/l` | Public mode set/reset |
| `CSI ?h/l` | Private mode set/reset |
| `CSI n;mr` | Set scroll region margins |
| `CSI s/u` | Save/restore cursor |
| `ESC 7/8` | Save/restore cursor (legacy) |
| `OSC 0/1/2;… BEL` | Set icon/title name |
| `ESC Z` | DA0 (identify terminal) |
| `CSI c` | DA1 (identify terminal) |
| `CSI ?6c` | DA1 response |
| `CSI n;1;0 u` | Kitty keyboard mode |
| `CSI q` | Cursor style (DECSCUSR) |

### G0/G1 Character Sets

| Designation | Charset |
|-------------|---------|
| `B` / `(` | ASCII (default) |
| `0` | DEC Special (line drawing) |
| `A`, `4`–`9`, `<`, `=`, `>`, `?`, `C`–`S` | Known but no mapping |

### Parser State Machine

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

## Async / GIL Strategy

- All I/O (`read`, `write`, `wait`, `read_timeout`) runs via `pyo3_async_runtimes::tokio::future_into_py`
- GIL is released during tokio I/O waits
- GIL acquired only for: converting buffers to `PyBytes`, raising exceptions
- `spawn`/`open_pty` are `async` — must `await`

## Build

```bash
cargo build              # pure Rust build
cargo test               # pure Rust tests
maturin develop          # Python extension dev build
maturin build --release  # Python extension release
uv run --extra dev pytest # run Python tests
```

## Dependencies

| Crate | Purpose |
|-------|---------|
| `pyo3` + `pyo3-async-runtimes` | Python bindings + tokio runtime bridge |
| `tokio` (full) | Async runtime, named pipes, io-util |
| `parking_lot` | Fast mutexes for pipe handles |
| `thiserror` | Error types |
| `bitflags` | Terminal mode flags |
| `unicode-width` | Unicode character width (for wide chars) |
| `nix` (Unix only) | POSIX PTY, signals, ioctls |
| `windows` (Windows only) | ConPTY, NamedPipes, CreateProcessW |

## License

MIT OR Apache-2.0
