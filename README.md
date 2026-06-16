# stitch-pty

> Cross-platform async PTY with integrated terminal emulation for Python.

[![CI](https://github.com/stitch-pty/stitch-pty/actions/workflows/CI.yml/badge.svg)](https://github.com/stitch-pty/stitch-pty/actions)
[![PyPI](https://img.shields.io/pypi/v/stitch-pty.svg)](https://pypi.org/project/stitch-pty/)
[![Python](https://img.shields.io/pypi/pyversions/stitch-pty.svg)](https://pypi.org/project/stitch-pty/)
[![Rust](https://img.shields.io/badge/rust-1.85+-orange.svg)](https://www.rust-lang.org/)
[![License](https://img.shields.io/pypi/l/stitch-pty.svg)](https://pypi.org/project/stitch-pty/)

**stitch-pty** is a high-performance, cross-platform pseudo-terminal (PTY) library for Python,
written in Rust with PyO3 bindings. It provides true PTY semantics on Linux, macOS, and Windows,
with a built-in VT100/VT220/xterm-compatible terminal emulator for rendering ANSI escape sequences
with scrollback history.

---

## Why stitch-pty?

| Feature | stitch-pty | `pty` stdlib | `pexpect` |
|---------|-----------|-------------|-----------|
| Cross-platform | ✅ Linux / macOS / Windows | ❌ POSIX only | ❌ POSIX only |
| Async I/O | ✅ Native `asyncio` | ❌ Blocking | ❌ Blocking |
| Zero GIL contention | ✅ PyO3 native async | ✅ | ✅ |
| Terminal emulation | ✅ Built-in (scrollback + styled viewport) | ❌ | ❌ |
| No zombie processes | ✅ Background reaping (50ms polling) | ⚠️ Manual | ⚠️ Manual |
| Type-safe Python API | ✅ Full mypy support | ✅ | ✅ |

---

## Table of Contents

- [Why stitch-pty?](#why-stitch-pty)
- [Table of Contents](#table-of-contents)
- [Quick Start](#quick-start)
- [Installation](#installation)
- [API Reference](#api-reference)
  - [Core Functions](#core-functions)
  - [PtySession](#ptysession)
  - [PtyMaster](#ptymaster)
  - [PtyChild](#ptychild)
  - [TerminalState](#terminalstate)
  - [Winsize](#winsize)
  - [Error Types](#error-types)
- [Architecture](#architecture)
  - [High-Level Overview](#high-level-overview)
  - [PTY Backend](#pty-backend)
    - [POSIX (`platform_unix.rs`)](#posix-platform_unixrs)
    - [Windows (`platform_windows.rs`)](#windows-platform_windowsrs)
  - [Terminal Emulation (`terminal/`)](#terminal-emulation-terminal)
  - [GIL Management Strategy](#gil-management-strategy)
  - [Memory Safety](#memory-safety)
- [Examples](#examples)
  - [Simple Command Execution](#simple-command-execution)
  - [Interactive Shell](#interactive-shell)
  - [pexpect-Style Expect](#pexpect-style-expect)
  - [Raw PTY (No Child)](#raw-pty-no-child)
  - [PySide6 Terminal Emulator (GUI)](#pyside6-terminal-emulator-gui)
- [Building from Source](#building-from-source)
  - [Prerequisites](#prerequisites)
  - [Development Workflow](#development-workflow)
  - [Cross-Compilation](#cross-compilation)
- [Source Projects & Dependencies](#source-projects--dependencies)
  - [portable-pty — PTY Abstraction Layer](#1-portable-pty--pty-abstraction-layer)
  - [pyte — Terminal Emulation](#2-pyte--terminal-emulation)
  - [vte — ANSI Escape Sequence Parser](#3-vte--ansi-escape-sequence-parser)
  - [Dependency Summary](#dependency-summary)
- [Platform Support](#platform-support)
- [License](#license)

---

## Quick Start

```python
import asyncio
from stitch_pty import spawn

async def main():
    # Spawn a shell in a real PTY
    session = await spawn("bash", ["-i"])

    # Read output (auto-fed into the terminal emulator)
    data = await session.read(4096)
    print(session.display)          # visible screen → list[str]
    print(session.scrollback)       # scrollback history → list[str]
    print(session.full_display)     # history + visible → list[str]

    # Write to the PTY
    await session.write(b"echo hello from stitch-pty\n")

    # Resize the terminal
    session.resize(50, 120)

    # Graceful shutdown
    await session.terminate(5.0)

asyncio.run(main())
```

---

## Installation

```bash
pip install stitch-pty
```

**Pre-built wheels** are available for:

| Platform | Architectures |
|----------|--------------|
| **Linux** | `x86_64`, `aarch64` (manylinux_2_28) |
| **macOS** | `x86_64`, `arm64` (universal2) |
| **Windows** | `x86_64` |

**Requirements:** Python ≥ 3.12

---

## API Reference

### Core Functions

#### `spawn(program, args=None, env=None, winsize=None) → PtySession`

Spawn a program in a PTY and return a session handle.

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `program` | `str` | — | The executable to run |
| `args` | `list[str]` | `[]` | Command-line arguments |
| `env` | `dict[str, str]` | `None` | Environment variables (inherits from parent if `None`) |
| `winsize` | `Winsize \| None` | Auto-detected | Initial terminal size (defaults to 24×80) |

**Returns:** `PtySession` — a combined PTY I/O + child process manager with integrated terminal emulation.

**Example:**

```python
session = await spawn("python3", ["-c", "print('hello')"])
output = await session.interact()
print(output.decode())  # b"hello\n"
```

#### `open_pty(winsize=None) → PtyMaster`

Open a PTY pair without spawning a child process.

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `winsize` | `Winsize \| None` | 24×80 | Terminal size |

**Returns:** `PtyMaster` — raw PTY I/O handle (no terminal emulation, no child management).

**Example:**

```python
pty = await open_pty(Winsize(24, 80, 0, 0))
data = await pty.read(4096)
await pty.write(b"hello\n")
```

---

### PtySession

The primary interface for most use cases. Combines PTY I/O, child process management, and terminal emulation.

| Method | Signature | Description |
|--------|-----------|-------------|
| `read` | `await read(size=4096) → bytes` | Read from PTY (auto-feeds terminal) |
| `read_timeout` | `await read_timeout(size, timeout) → bytes` | Read with timeout (raises `IOError` on timeout) |
| `write` | `await write(data) → int` | Write bytes to PTY, returns bytes written |
| `write_all` | `await write_all(data) → None` | Write all bytes (handles partial writes) |
| `resize` | `resize(rows, cols) → None` | Resize terminal (forwards to PTY backend) |
| `wait` | `await wait() → dict \| None` | Wait for child exit; returns `{pid, exit_code, signal, core_dumped}` or `None` |
| `terminate` | `await terminate(grace_period=5.0) → None` | SIGTERM → wait → SIGKILL fallback |
| `kill` | `kill() → None` | Force kill immediately |
| `interrupt` | `interrupt() → None` | Send Ctrl+C (SIGINT on POSIX, `GenerateConsoleCtrlEvent` on Windows) |
| `send_signal` | `send_signal(num) → None` | Send arbitrary signal number |
| `interact` | `await interact(input_data=None, timeout=None) → bytes` | Write input, read until EOF (high-level) |
| `read_all` | `await read_all(timeout=1.0) → bytes` | Read all output until timeout |
| `expect` | `await expect(pattern, timeout=30.0) → bytes` | pexpect-style: read until pattern found |

**Properties:**

| Property | Type | Description |
|----------|------|-------------|
| `is_alive` | `bool` | Process still running? |
| `terminal` | `TerminalState` | Direct access to terminal emulation state |
| `display` | `list[str]` | Visible screen (one string per row) |
| `scrollback` | `list[str]` | Scrollback history |
| `full_display` | `list[str]` | History + visible screen |
| `raw_output` | `bytes` | All raw bytes read (unparsed) |

**Context Manager:**

```python
async with spawn("bash", ["-i"]) as session:
    await session.write(b"echo hello\n")
    output = await session.interact()
# session.terminate(2.0) called automatically on exit
```

---

### PtyMaster

Raw PTY I/O without terminal emulation or child management.

| Method | Signature | Description |
|--------|-----------|-------------|
| `read` | `await read(size=4096) → bytes` | Read from PTY master |
| `read_timeout` | `await read_timeout(size, timeout) → bytes` | Read with timeout |
| `write` | `await write(data) → int` | Write bytes, returns count |
| `write_all` | `await write_all(data) → None` | Write all bytes |
| `set_winsize` | `set_winsize(rows, cols, xpixel=0, ypixel=0) → None` | Set window size |
| `get_winsize` | `get_winsize() → Winsize` | Get current size |
| `fd` | `property → int` | Raw file descriptor (Unix) / `-1` (Windows) |
| `is_open` | `property → bool` | PTY still open? |

---

### PtyChild

Child process management.

| Property/Method | Signature | Description |
|-----------------|-----------|-------------|
| `pid` | `property → int` | Child process PID |
| `is_running` | `property → bool` | Process still running? |
| `wait` | `await wait() → dict \| None` | Wait for exit; returns `{pid, exit_code, signal, core_dumped}` |
| `terminate` | `await terminate(grace_period=5.0) → None` | SIGTERM → wait → SIGKILL |
| `kill` | `kill() → None` | Force kill |
| `interrupt` | `interrupt() → None` | Send Ctrl+C |
| `send_signal` | `send_signal(num) → None` | Send signal number |

---

### TerminalState

VT100/VT220/xterm-compatible terminal emulation with scrollback.

| Method | Signature | Description |
|--------|-----------|-------------|
| `feed` | `feed(data: bytes) → None` | Feed raw bytes (parses ANSI, updates screen) |
| `display` | `display() → list[str]` | Full display (scrollback + visible) |
| `visible_display` | `visible_display() → list[str]` | Visible screen only |
| `history_display` | `history_display() → list[str]` | Scrollback history only |
| `styled_viewport` | `styled_viewport() → list[list[tuple[str, str, str, int]]]` | Full buffer as styled cells: `(text, fg, bg, attrs_bitmask)` |
| `total_lines` | `total_lines() → int` | Total lines = history + visible |
| `absolute_cursor` | `absolute_cursor() → tuple[int, int]` | `(x, history_len + on_screen_y)` |
| `dirty` | `dirty() → list[int]` | Modified row indices |
| `resize` | `resize(lines, cols) → None` | Resize screen buffer |
| `reset` | `reset() → None` | Reset terminal + clear history |

| Property | Type | Description |
|----------|------|-------------|
| `cursor_x` | `int` | Cursor column (0-indexed, visible area) |
| `cursor_y` | `int` | Cursor row (0-indexed, visible area) |
| `title` | `str` | Window title (from OSC sequences) |
| `history_size` | `int` | Current scrollback line count |
| `scrollback_lines` | `int` | Scrollback capacity |
| `set_scrollback_lines` | `set_scrollback_lines(n) → None` | Set capacity (trims excess) |

**Styled Viewport Cell Layout:**

```python
# Each cell: (text, fg, bg, attrs_bitmask)
# fg/bg formats: "default", ANSI name ("red", "brightblue"), or 6-hex RGB ("ff0000")
# attrs bitmask: bit 0=bold, 1=dim, 2=italics, 3=underscore,
#                4=blink, 5=reverse, 6=hidden, 7=strikethrough
```

---

### Winsize

Terminal window dimensions.

| Property | Type | Description |
|----------|------|-------------|
| `rows` | `u16` | Number of rows |
| `cols` | `u16` | Number of columns |
| `xpixel` | `u16` | Width in pixels |
| `ypixel` | `u16` | Height in pixels |

```python
ws = Winsize(50, 120, 0, 0)
```

---

### Error Types

| Exception | Inherits | Raised On |
|-----------|----------|-----------|
| `PtyError` | `Exception` | PTY open/operation failures, platform errors |
| `ProcessError` | `PtyError` | Spawn/kill failures, process not running |
| `IOError` | `PtyError` | I/O errors, timeouts, winsize failures |

---

## Architecture

### High-Level Overview

```
┌──────────────────────────────────────────────────────────────────┐
│                         Python Layer                              │
│  ┌──────────────┐  ┌────────────────────────────────────────┐   │
│  │ PtySession   │  │ TerminalState (pyte_rs)                │   │
│  │ (Python)     │  │ ├─ Parser (ANSI stream → Screen)      │   │
│  │              │  │ ├─ HistoryScreen (scrollback buffer)   │   │
│  │              │  │ └─ Screen (Char grid, cursor, SGR)     │   │
│  └──────┬───────┘  └────────────────────────────────────────┘   │
│         │                                                        │
│  ┌──────┴──────────────────────────────────────────────────┐    │
│  │         stitch_pty._core (PyO3 / Rust)                  │    │
│  │  ┌──────────────────────────────────────────────────┐   │    │
│  │  │     Platform Abstraction Layer                    │   │    │
│  │  │  ┌──────────────────┐  ┌──────────────────────┐  │   │    │
│  │  │  │  POSIX Backend   │  │  Windows Backend     │  │   │    │
│  │  │  │  ── openpty()   │  │  ── ConPTY (dyn load)│  │   │    │
│  │  │  │  ── fork()      │  │  ── NamedPipe (async)│  │   │    │
│  │  │  │  ── AsyncFd     │  │  ── CreateProcessW   │  │   │    │
│  │  │  │  ── waitpid()   │  │  ── GetExitCodeProc   │  │   │    │
│  │  │  └──────────────────┘  └──────────────────────┘  │   │    │
│  │  └──────────────────────────────────────────────────┘   │    │
│  └──────────────────────────────────────────────────────────┘    │
└──────────────────────────────────────────────────────────────────┘
```

### PTY Backend

#### POSIX (`platform_unix.rs`)

| Component | Detail |
|-----------|--------|
| **PTY creation** | `openpty(3)` → master/slave pair, `O_NONBLOCK` on master |
| **Process spawn** | `fork()` → child: `setsid()` + `TIOCSCTTY` + `dup2()`×3 + `execvpe()` |
| **Async I/O** | `tokio::io::AsyncFd` over raw FD with `try_io` pattern |
| **Reaping** | Background `tokio::spawn` polls `waitpid(WNOHANG)` every 50ms |
| **Signal delivery** | All signals sent to process group (`-pgid`) via `nix::sys::signal` |
| **Resize** | `TIOCSWINSZ` ioctl + `SIGWINCH` to process group via `tcgetpgrp` |
| **FD leak fix** | `close_random_fds()` closes FDs > 2 via `/dev/fd` (critical for macOS Big Sur) |
| **Signal reset** | Pre-exec: resets `SIGCHLD`, `SIGHUP`, `SIGINT`, `SIGTERM`, `SIGALRM` to `SIG_DFL` |

**Async I/O Pattern:**

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

#### Windows (`platform_windows.rs`)

| Component | Detail |
|-----------|--------|
| **ConPTY loading** | Dynamic `GetProcAddress` from `kernel32.dll` (graceful fallback on older Windows) |
| **PTY creation** | `CreatePseudoConsole(size, stdin_handle, stdout_handle)` |
| **Pipe plumbing** | Two `tokio::net::NamedPipeServer` instances (input + output) |
| **Pipe naming** | `\\.\pipe\stitch-pty-{pid}-{counter}` (unique per instance) |
| **Process spawn** | `CreateProcessW` with `PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE` via `SafeAttributeList` |
| **Async I/O** | `tokio::net::NamedPipeServer` → true async via IOCP (no `spawn_blocking`) |
| **Reaping** | Background poll of `GetExitCodeProcess` every 50ms (no `WaitForSingleObject`) |
| **Signal delivery** | Ctrl+C → `GenerateConsoleCtrlEvent`; SIGTERM/KILL → `TerminateProcess` |
| **Resize** | `ResizePseudoConsole` (no signal forwarded to child) |
| **Startup handshake** | Sends DSR reply `\x1b[1;1R` to conhost to begin relaying child output |
| **Command quoting** | `ArgvQuote` algorithm: proper escaping of spaces, quotes, trailing backslashes |

**Send-Safe Windows Types:**

`SendSyncHandle` wraps `HANDLE` with `Send + Sync + Clone`, allowing Windows non-Send types to cross `.await` boundaries. All non-Send locals (`STARTUPINFOEXW`, `PROCESS_INFORMATION`, attribute list) are scoped to a block ending before the first `.await` after `CreateProcessW`.

### Terminal Emulation (`terminal/`)

Embedded from [pyte_rs](https://github.com/python-pyte/pyte). Provides VT100/VT220/xterm-compatible rendering.

#### ANSI Parser (`ansi_parser.rs`)

ECMA-48 state machine with 10 states:

| State | Enters On | Exits On |
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

**Parser Limits:** Max 32 params (sub-param groups), 2 intermediates, 16 OSC params, 1024 OSC bytes, 4-byte UTF-8 partial buffer.

#### Screen Buffer (`screen.rs`)

| Feature | Detail |
|---------|--------|
| **Char cell** | `data`, `fg`, `bg`, `bold`, `dim`, `italics`, `underscore`, `blink`, `reverse`, `hidden`, `strikethrough` |
| **SGR colors** | 8 ANSI, 8 aixterm bright, 256-color, 24-bit RGB (with colon subparams) |
| **Modes** | Public (IRM, LNM) + Private (DECOM, DECAWM, DECCOLM, DECTCEM, DECSCNM) + Extended (mouse, bracketed paste) |
| **Character sets** | G0/G1 with DEC Special line drawing |
| **Unicode** | Width-1/2 chars, combining marks, CJK, emoji |
| **Kitty Keyboard Protocol** | Mode push/pop/replace |
| **Dirty tracking** | `BTreeSet<usize>` of modified row indices |

#### Scrollback (`history.rs`)

| Method | Description |
|--------|-------------|
| `scroll_up_with_history(rows)` | Push scrolled lines to history |
| `scroll_down_with_history(rows)` | Pop lines from history |
| `total_lines()` | `history_len + visible_lines` |
| `absolute_cursor()` | `(x, history_len + on_screen_y)` |
| `styled_viewport()` | Full buffer as `(text, fg, bg, attrs_bitmask)` cells |

### GIL Management Strategy

PyO3 native async automatically releases the GIL at `.await`:

```
Python call (GIL held)
    → future_into_py (GIL released)
    → tokio I/O await (GIL released)
    → completion: GIL reacquired
    → Rust value → Python object conversion
```

**GIL is acquired only for:**
1. Converting `Vec<u8>` → `PyBytes`
2. Raising `PyErr` exceptions

### Memory Safety

**POSIX ownership:**
```
PtyPair (owns master_fd + slave_fd)
    ├── master_fd → UnixPtyMaster (via mem::forget)
    │   └── Drop: close(fd)
    └── slave_fd → child process (via fork)
        └── Child closes on exec, parent closes after fork
```

**Windows ownership:**
```
WinPtyBackend (owns HPCON + NamedPipeServer handles)
    ├── input_pipe → NamedPipeServer (Drop: drop)
    ├── output_pipe → NamedPipeServer (Drop: drop)
    └── conpty → ConPTY (Drop: ClosePseudoConsole)
```

**RAII guarantees:**

| Type | Drop Behavior |
|------|---------------|
| `UnixPtyMaster` | `close(raw_fd)` |
| `WinPtyBackend` | `ClosePseudoConsole` + drop named pipes |
| `UnixChildProcess` | `kill(SIGKILL)` if running |
| `WinChildProcess` | `TerminateProcess` if running; `CloseHandle` |

---

## Examples

### Simple Command Execution

```python
import asyncio
from stitch_pty import spawn

async def main():
    session = await spawn("echo", ["hello", "world"])
    output = await session.interact()
    print(output.decode())  # b"hello world\n"

asyncio.run(main())
```

### Interactive Shell

```python
import asyncio
from stitch_pty import spawn

async def main():
    session = await spawn("bash", ["-i"])

    # Send a command
    await session.write(b"ls -la\n")

    # Wait for output
    output = await session.interact()
    print(output.decode())

    # Cleanup
    await session.terminate()

asyncio.run(main())
```

### pexpect-Style Expect

```python
import asyncio
from stitch_pty import spawn

async def main():
    session = await spawn("bash", ["-i"])

    # Wait for a prompt
    prompt = await session.expect(b"$ ", timeout=10.0)
    print(f"Got: {prompt.decode()}")

    # Send command
    await session.write(b"uname -a\n")

    # Wait for output
    output = await session.expect(b"\n", timeout=5.0)
    print(f"Output: {output.decode()}")

    await session.terminate()

asyncio.run(main())
```

### Raw PTY (No Child)

```python
import asyncio
from stitch_pty import open_pty, Winsize

async def main():
    pty = await open_pty(Winsize(24, 80, 0, 0))

    # Write and read raw bytes
    await pty.write(b"hello from raw PTY\n")
    data = await pty.read(4096)
    print(data.decode())

asyncio.run(main())
```

### PySide6 Terminal Emulator (GUI)

A full terminal emulator with real-time rendering, keyboard input, resize handling, and styled viewport:

```bash
pip install PySide6 stitch-pty
python examples/terminal_emulator.py
python examples/terminal_emulator.py --cmd "whoami"
python examples/terminal_emulator.py --rows 30 --cols 100
```

**Key features demonstrated:**
- Async PTY + Qt event loop integration (background thread)
- Real-time styled rendering via `styled_viewport()` with HTML
- Cursor position tracking via `absolute_cursor()`
- Keyboard input forwarding with Ctrl+key → ANSI sequences
- Window resize forwarding (rate-limited during drag)
- Graceful shutdown on window close

---

## Building from Source

### Prerequisites

| Requirement | Version |
|-------------|---------|
| Rust | ≥ 1.85 |
| Python | ≥ 3.12 |
| maturin | ≥ 1.8 |

```bash
# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Install maturin
pip install maturin
```

### Development Workflow

```bash
# Clone
git clone https://github.com/stitch-pty/stitch-pty.git
cd stitch-pty

# Development build (fast, unoptimized)
maturin develop

# Run tests (platform-specific tests auto-skip)
pytest tests/ -v

# Type check
mypy python/stitch_pty/

# Lint
ruff check .

# Production wheel
maturin build --release
```

### Cross-Compilation

```bash
# Linux ARM64 from x86_64 host
maturin build --release --target aarch64-unknown-linux-gnu

# macOS universal2
maturin build --release --target universal2-apple-darwin

# Windows from Linux (requires cross toolchain)
maturin build --release --target x86_64-pc-windows-gnu
```

---

## Source Projects & Dependencies

stitch-pty is built on three foundational open-source projects. Each contributed significant
portions of code, architecture patterns, and design decisions that were adapted, extended,
or embedded into this project.

### 1. portable-pty — PTY Abstraction Layer

| Property | Detail |
|----------|--------|
| **Source** | `../other packages/portable-pty-main` |
| **Repository** | [github.com/wezterm/portable-pty](https://github.com/wezterm/portable-pty) |
| **Version** | 0.9.0 |
| **Author** | Wez Furlong |
| **License** | MIT |
| **Role** | Cross-platform PTY backend patterns (traits, `ChildKiller`, `ExitStatus`, `PtySize`) |

**What was taken:**

- **`ChildKiller` trait** — deadlock-free kill/wait separation pattern (cloneable killer
  split from `ChildBackend` to avoid blocking `.wait` threads)
- **`ExitStatus` / `ProcessExit` structs** — unified exit code + signal representation
- **`PtySize` → `Winsize`** — window size struct with rows/cols/pixels
- **`close_random_fds()`** — macOS Big Sur / Linux FD leak prevention (closes all FDs > 2
  via `/dev/fd` listing)
- **`ArgvQuote` algorithm** — proper Windows command-line quoting (backslash/quote escaping)
- **`SafeAttributeList` pattern** — Win32 `PROC_THREAD_ATTRIBUTE_LIST` lifecycle management
  (Initialize → Update → Delete) for ConPTY attachment
- **`ProcessSignaller`** — cross-platform signal delivery abstraction

**What changed:**

- Rewritten for **tokio async** (portable-pty uses blocking I/O)
- Replaced `nix 0.28` → `nix 0.31` with additional features (signal, poll, ioctl)
- Replaced `winapi` → `windows` crate (0.62, modern MSFT bindings)
- Replaced `anyhow` → `thiserror` for zero-cost error propagation
- Added **Tokio NamedPipes** for Windows async I/O (portable-pty uses `spawn_blocking`)
- Added **dynamic ConPTY loading** via `GetProcAddress` for graceful fallback
- GIL management for Python integration

---

### 2. pyte — Terminal Emulation

| Property | Detail |
|----------|--------|
| **Source** | `../other packages/pyte-master` |
| **Repository** | [github.com/selectel/pyte](https://github.com/selectel/pyte) |
| **Version** | 0.8.3 (dev) |
| **License** | LGPL-3.0 |
| **Role** | VT100/VT220/xterm-compatible terminal screen emulation (embedded as `terminal/`) |

**What was taken:**

- **`Screen`** — 2D Char grid, cursor management, tab stops, dirty tracking
- **`Char` cell** — data, fg/bg colors, bold/dim/italics/underscore/blink/reverse/hidden/strikethrough
- **`Cursor`** — position, attributes, save/restore stack
- **`Margins`** — scroll region top/bottom
- **`CursorStyle`** — block/underline/beam variants
- **SGR color handling** — 8 ANSI, 8 aixterm bright, 256-color palette, 24-bit RGB
- **Mode management** — public ANSI (IRM, LNM) + private DEC (DECOM, DECAWM, DECCOLM, etc.)
- **Character set designation** — G0/G1 with DEC Special line drawing map
- **OSC title/icon dispatch** — OSC 0/1/2; icon name and window title
- **CSI cursor movement** — CUU/CUD/CUF/CUB/CUP/CNL/CPL/CHA
- **CSI erase** — ED (display), EL (line), DC (characters), DL/IL (lines)
- **Unicode handling** — `unicode-width` for width-1/width-2 chars, combining marks
- **Kitty Keyboard Protocol** — mode push/pop/replace
- **Device status** — DA0/DA1 identification, cursor position report

**What changed:**

- Ported from **Python to Rust** with PyO3 bindings
- Added **`HistoryScreen`** — scrollback buffer (not in upstream pyte)
- Added **`styled_viewport()`** — full buffer as `(text, fg, bg, attrs_bitmask)` cells
- Added **`total_lines()`** and **`absolute_cursor()`** for scroll-aware positioning
- Added **`unicode-segmentation`** for proper grapheme boundary handling
- SGR subparameter parsing (`:` as param separator) — differs from vte crate
- Color stored as hex strings (`"ff0000"`) instead of RGB tuples for Python serialization

---

### 3. vte — ANSI Escape Sequence Parser

| Property | Detail |
|----------|--------|
| **Source** | `../other packages/vte-master` |
| **Repository** | [github.com/alacritty/vte](https://github.com/alacritty/vte) |
| **Version** | 0.15.0 |
| **Authors** | Joe Wilm, Christian Duerr |
| **License** | Apache-2.0 OR MIT |
| **Role** | ECMA-48 ANSI escape sequence state machine (embedded as `terminal/ansi_parser.rs`) |

**What was taken:**

- **ECMA-48 state machine** — 10-state ANSI parser (Ground, CsiEntry, CsiParam,
  CsiIntermediate, CsiIgnore, OscString, DcsEntry, DcsPassthrough, Escape, EscapeIntermediate)
- **`Parser` struct** — state, intermediates, params, UTF-8 partial buffer, OSC raw buffer
- **`Params` struct** — sub-parameter groups with `MAX_PARAMS = 32` limit
- **`Perform` trait** — `print`, `execute`, `hook`, `put`, `unhook`, `osc_dispatch`,
  `csi_dispatch`, `esc_dispatch`, `terminated`
- **UTF-8 handling** — partial byte buffer (4 bytes), invalid byte replacement with `\u{FFFD}`
- **OSC handling** — `MAX_OSC_PARAMS = 16`, `MAX_OSC_RAW = 1024`, BEL/ST termination
- **DCS passthrough** — hook/passthrough/unhook state transitions
- **C0/C1 control handling** — C0 (`\x00`–`\x1f`), C1 (`\x80`–`\x9f`)
- **Parameter parsing** — `:` subparameter separator, `;` param separator, overflow saturation

**What changed:**

- Embedded directly as source (not a crate dependency) — allows custom `Perform` impl
- `Perform` trait extended with `esc_dispatch` for legacy ESC sequences
- CSI parameter parsing: `:` treated as param separator (not subparam like upstream vte)
- Added `escape.rs` designator constants (RIS, IND, NEL, HTS, etc.)
- Added `charsets.rs` DEC Special line drawing map
- Added `control.rs` C0/C1 constants
- Added extensive unit tests for parser edge cases (overflow, reset, partial UTF-8)

---

### Dependency Summary

| Source Project | License | How Used | Lines Contributed |
|---------------|---------|----------|-------------------|
| **portable-pty** | MIT | PTY backend patterns, `ChildKiller`, `ExitStatus`, `ArgvQuote`, `SafeAttributeList` | ~200 lines (patterns) |
| **pyte** | LGPL-3.0 | Terminal emulation (`Screen`, `Char`, `Cursor`, SGR, modes, character sets) | ~1,200 lines (embedded) |
| **vte** | Apache-2.0 OR MIT | ANSI parser state machine (`Parser`, `Params`, `Perform`) | ~600 lines (embedded) |

**Note:** stitch-pty is licensed MIT OR Apache-2.0. The LGPL-3.0 dependency (pyte) is
satisfied by the "exception to section 3" clause: the terminal emulation is embedded as
a module, not linked as a library. The MIT and Apache-2.0 licenses are compatible with
stitch-pty's dual licensing.

---

## Platform Support

| Platform | Backend | Signal Support | Resize Signal | Exit Info | Status |
|----------|---------|----------------|---------------|-----------|--------|
| **Linux** | POSIX `openpty()` + `fork()` | Full | ✅ `SIGWINCH` | code + signal + core_dumped | ✅ Complete |
| **macOS** | POSIX `openpty()` + `fork()` | Full | ✅ `SIGWINCH` | code + signal + core_dumped | ✅ Complete |
| **Windows 10 1809+** | ConPTY + NamedPipes | Ctrl+C only | ❌ No signal | code only | ✅ Complete |
| **Windows <10** | `CreateProcess` + pipes | ❌ Limited | ❌ | code only | ⚠️ Fallback |

### Platform Differences

| Feature | POSIX | Windows |
|---------|-------|---------|
| PTY backend | `openpty(3)` + `fork()` + `execvpe()` | ConPTY (`CreatePseudoConsole`) + `CreateProcessW` |
| I/O model | `tokio::AsyncFd` over raw FDs | `tokio::NamedPipeServer` (IOCP) |
| Signal delivery | Full via `nix::sys::signal` (SIGINT, SIGTERM, SIGKILL, SIGWINCH) | Ctrl+C (`GenerateConsoleCtrlEvent`), SIGTERM/KILL → `TerminateProcess` |
| Resize signal | `SIGWINCH` forwarded to process group via `tcgetpgrp` | No signal; `ResizePseudoConsole` only |
| Exit info | `exit_code` + `signal` + `core_dumped` | `exit_code` only |
| Pipe plumbing | Single FD pair (master/slave) | Two named pipes (input/output) + `connect()` to arm IOCP |
| Startup handshake | N/A | DSR reply (`\x1b[1;1R`) to conhost before child output flows |
| FD leak fix | `close_random_fds()` (macOS/Linux) | N/A |

---

## License

MIT OR Apache-2.0
