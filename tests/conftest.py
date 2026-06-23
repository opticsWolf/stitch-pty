"""Shared fixtures for stitch-pty tests."""

import asyncio
import platform

import pytest

IS_WINDOWS = platform.system() == "Windows"


@pytest.fixture
def shell():
    """Return (prog, args) for a quick shell command."""
    def _shell(cmd: str):
        if IS_WINDOWS:
            return "cmd.exe", ["/c", cmd]
        else:
            return "bash", ["-c", cmd]
    return _shell


@pytest.fixture
def idle():
    """Return (prog, args) for a long-running idle process."""
    def _idle():
        if IS_WINDOWS:
            return "cmd.exe", ["/c", "timeout /t 30"]
        else:
            return "bash", ["-c", "sleep 30"]
    return _idle


@pytest.fixture
async def read_all():
    """Read all available output from a session within timeout."""
    async def _read_all(session, timeout=3.0, chunk_size=4096):
        data = b''
        deadline = asyncio.get_event_loop().time() + timeout
        while asyncio.get_event_loop().time() < deadline:
            remaining = deadline - asyncio.get_event_loop().time()
            try:
                chunk = await asyncio.wait_for(session.read(chunk_size), timeout=remaining)
                if not chunk:
                    break
                data += chunk
            except asyncio.TimeoutError:
                break
        return data
    return _read_all
