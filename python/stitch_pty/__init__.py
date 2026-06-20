"""stitch-pty: High-performance async PTY for Python, written in Rust.

This package provides a Pythonic async API for POSIX PTY operations,
backed by a high-performance Rust implementation using tokio and PyO3.

Key Features:
    - True POSIX PTY (not pipe emulation)
    - Native asyncio integration with automatic GIL release during I/O
    - Process group signal delivery (SIGINT, SIGTERM, SIGKILL)
    - Automatic window resize forwarding (SIGWINCH)
    - Zero zombie processes via background waitpid polling
    - Type-safe Python API with full mypy support

Example:
    >>> import asyncio
    >>> from stitch_pty import spawn
    >>>
    >>> async def main():
    ...     session = spawn("python3", ["-c", "print('hello from pty')"])
    ...     data = await session.read(1024)
    ...     print(data.decode())
    ...     await session.wait()
    ...
    >>> asyncio.run(main())
"""

from __future__ import annotations

import asyncio
import os
import signal
import struct
import sys
from typing import Any

# Unix-only imports (fcntl, termios) — stubbed on Windows
if sys.platform == "win32":
    fcntl = None  # type: ignore
    termios = None  # type: ignore
else:
    import fcntl
    import termios

# Import the Rust extension module
from stitch_pty._core import (
    PtyChild as _PtyChild,
    PtyError,
    PtyMaster as _PtyMaster,
    PtySession as _PtySession,
    TerminalState as _TerminalState,
    Winsize,
    open_pty as _open_pty,
    spawn as _spawn,
)

__version__ = "0.1.0"
__all__ = [
    "PtySession",
    "PtyMaster",
    "PtyChild",
    "TerminalState",
    "Winsize",
    "spawn",
    "open_pty",
    "PtyError",
    "ProcessError",
    "IOError",
]

# Re-export types for convenience
TerminalState = _TerminalState
ProcessError = type("ProcessError", (PtyError,), {})
IOError = type("IOError", (PtyError,), {})


class PtyMaster:
    """Python wrapper around the Rust PtyMaster with additional conveniences."""

    def __init__(self, inner: _PtyMaster) -> None:
        self._inner = inner

    async def read(self, size: int = 4096) -> bytes:
        """Read up to `size` bytes from the PTY master.

        Returns empty bytes on EOF (child process exited).
        """
        return await self._inner.read(size)

    async def read_timeout(self, size: int, timeout: float) -> bytes:
        """Read with a timeout in seconds.

        Raises stitch_pty.IOError if the timeout expires.
        """
        return await self._inner.read_timeout(size, timeout)

    async def write(self, data: bytes) -> int:
        """Write data to the PTY master. Returns bytes written."""
        return await self._inner.write(data)

    async def write_all(self, data: bytes) -> None:
        """Write all data, handling partial writes automatically."""
        await self._inner.write_all(data)

    def set_winsize(self, rows: int, cols: int, xpixel: int = 0, ypixel: int = 0) -> None:
        """Set the terminal window size and forward SIGWINCH."""
        self._inner.set_winsize(Winsize(rows, cols, xpixel, ypixel))

    def get_winsize(self) -> Winsize:
        """Get the current terminal window size."""
        return self._inner.get_winsize()

    @property
    def fd(self) -> int:
        """Raw file descriptor (for advanced use with select/poll)."""
        return self._inner.raw_fd()

    def __repr__(self) -> str:
        return f"PtyMaster(fd={self.fd})"


class PtyChild:
    """Python wrapper around the Rust PtyChild with additional conveniences."""

    def __init__(self, inner: _PtyChild) -> None:
        self._inner = inner

    @property
    def pid(self) -> int:
        """The child process PID."""
        return self._inner.pid

    @property
    def is_running(self) -> bool:
        """Whether the process is still running."""
        return self._inner.is_running

    async def wait(self) -> dict[str, Any] | None:
        """Wait for the process to exit.

        Returns a dict with keys: pid, exit_code, signal, core_dumped.
        Returns None if already reaped.
        """
        return await self._inner.wait()

    async def terminate(self, grace_period: float = 5.0) -> None:
        """Send SIGTERM, wait for graceful exit, then SIGKILL if needed."""
        await self._inner.terminate(grace_period)

    def kill(self) -> None:
        """Force kill the process immediately (SIGKILL)."""
        self._inner.kill()

    def interrupt(self) -> None:
        """Send Ctrl+C (SIGINT) to the process group."""
        self._inner.interrupt()

    def send_signal(self, signal_num: int) -> None:
        """Send a custom Unix signal to the process group."""
        self._inner.send_signal(signal_num)

    def __repr__(self) -> str:
        return f"PtyChild(pid={self.pid}, running={self.is_running})"


class PtySession:
    """High-level PTY session combining master I/O and child management.

    This is the primary interface for most use cases.
    Includes an integrated terminal emulation layer that parses ANSI
    escape sequences and maintains screen state.
    """

    def __init__(self, inner: _PtySession, scrollback: int = 1000) -> None:
        self._inner = inner
        winsize = inner.get_winsize()
        self._terminal = _TerminalState(winsize.cols, winsize.rows, scrollback)
        self._raw_output: list[bytes] = []

    @property
    def is_alive(self) -> bool:
        """Whether the child process is still running."""
        return self._inner.is_alive

    async def read(self, size: int = 4096) -> bytes:
        """Read from the PTY and feed data through the terminal emulator."""
        data = await self._inner.read(size)
        if data:
            self._terminal.feed(data)
            self._raw_output.append(data)
        return data

    async def read_timeout(self, size: int, timeout: float) -> bytes:
        """Read with timeout and feed data through the terminal emulator."""
        data = await self._inner.read_timeout(size, timeout)
        if data:
            self._terminal.feed(data)
            self._raw_output.append(data)
        return data

    async def write(self, data: bytes) -> int:
        """Write to the PTY."""
        return await self._inner.write(data)

    async def write_all(self, data: bytes) -> None:
        """Write all data."""
        await self._inner.write_all(data)

    def set_winsize(self, rows: int, cols: int, xpixel: int = 0, ypixel: int = 0) -> None:
        """Set window size and forward SIGWINCH."""
        self._inner.set_winsize(Winsize(rows, cols, xpixel, ypixel))

    def get_winsize(self) -> Winsize:
        """Get current window size."""
        return self._inner.get_winsize()

    async def wait(self) -> dict[str, Any] | None:
        """Wait for child to exit."""
        return await self._inner.wait()

    async def terminate(self, grace_period: float = 5.0) -> None:
        """Graceful termination with fallback to SIGKILL."""
        await self._inner.terminate(grace_period)

    def kill(self) -> None:
        """Force kill."""
        self._inner.kill()

    def interrupt(self) -> None:
        """Send Ctrl+C."""
        self._inner.interrupt()

    def resize(self, rows: int, cols: int) -> None:
        """Resize the terminal."""
        self._inner.resize(rows, cols)

    def send_signal(self, signal_num: int) -> None:
        """Send a custom signal."""
        self._inner.send_signal(signal_num)

    async def interact(
        self,
        input_data: bytes | None = None,
        timeout: float | None = None,
    ) -> bytes:
        """High-level interaction: write input, read all output until EOF.

        Args:
            input_data: Data to send to the PTY before reading.
            timeout: Maximum seconds to wait for output. None = wait forever.

        Returns:
            All output collected from the PTY.
        """
        if input_data:
            await self.write_all(input_data)

        chunks: list[bytes] = []
        while True:
            try:
                if timeout is not None:
                    chunk = await self.read_timeout(4096, timeout)
                else:
                    chunk = await self.read(4096)

                if not chunk:
                    break
                chunks.append(chunk)
            except PtyError:
                break

        return b"".join(chunks)

    async def read_all(self, timeout: float = 1.0) -> bytes:
        """Read all available output until timeout.

        Useful for collecting all output from short-lived commands.
        Each chunk is fed through the terminal emulator.

        Args:
            timeout: Seconds to wait between reads before considering output complete.

        Returns:
            All collected output as bytes.
        """
        return await self.interact(timeout=timeout)

    async def expect(self, pattern: bytes, timeout: float = 30.0) -> bytes:
        """Read until `pattern` appears in output (like pexpect).

        Returns all data read up to and including the pattern.
        Raises TimeoutError if pattern not found within timeout.
        """
        buffer = bytearray()
        deadline = asyncio.get_event_loop().time() + timeout

        while True:
            remaining = deadline - asyncio.get_event_loop().time()
            if remaining <= 0:
                raise TimeoutError(f"Pattern {pattern!r} not found within {timeout}s")

            try:
                chunk = await self.read_timeout(4096, remaining)
            except PtyError:
                raise TimeoutError(f"Pattern {pattern!r} not found. Buffer: {bytes(buffer)!r}")

            if not chunk:
                raise TimeoutError(f"EOF before pattern found. Buffer: {bytes(buffer)!r}")

            buffer.extend(chunk)
            if pattern in buffer:
                return bytes(buffer)

    # ── Terminal emulation properties ─────────────────────────────────

    @property
    def terminal(self) -> _TerminalState:
        """Access the terminal emulation state directly."""
        return self._terminal

    @property
    def display(self) -> list[str]:
        """Get the visible screen as a list of strings (one per row)."""
        return self._terminal.visible_display()

    @property
    def scrollback(self) -> list[str]:
        """Get the scrollback history as a list of strings."""
        return self._terminal.history_display()

    @property
    def full_display(self) -> list[str]:
        """Get the full display (scrollback + visible screen) as a list of strings."""
        return self._terminal.display()

    @property
    def raw_output(self) -> bytes:
        """Get all raw bytes read from the PTY (unparsed)."""
        return b"".join(self._raw_output)

    def __repr__(self) -> str:
        return f"PtySession(...)"

    async def __aenter__(self) -> PtySession:
        return self

    async def __aexit__(self, *args: Any) -> None:
        await self.terminate(2.0)


def open_pty(winsize: Winsize | None = None) -> PtyMaster:
    """Open a new PTY pair and return the master handle.

    Use this for fine-grained control over the child process lifecycle.
    """
    return PtyMaster(_open_pty(winsize))


async def spawn(
    program: str,
    args: list[str] | None = None,
    env: dict[str, str] | None = None,
    winsize: Winsize | None = None,
) -> PtySession:
    """Spawn a program in a PTY and return a session handle.

    Args:
        program: The executable to run.
        args: Command-line arguments.
        env: Environment variables. If None, inherits from parent.
        winsize: Initial terminal size. Defaults to 24x80.

    Returns:
        A PtySession for I/O and process management.

    Example:
        >>> import asyncio
        >>> from stitch_pty import spawn
        >>>
        >>> async def demo():
        ...     session = await spawn("echo", ["hello", "world"])
        ...     output = await session.interact()
        ...     print(output.decode())
        ...
        >>> asyncio.run(demo())
        hello world
    """
    if winsize is None:
        # Try to detect terminal size, fallback to 24x80
        try:
            size = os.get_terminal_size()
            winsize = Winsize(size.lines, size.columns, 0, 0)
        except OSError:
            winsize = Winsize(24, 80, 0, 0)

    inner = await _spawn(program, args or [], env, winsize)
    return PtySession(inner)
