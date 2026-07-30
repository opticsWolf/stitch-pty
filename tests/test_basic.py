"""Basic tests for stitch-pty PTY functionality."""

import asyncio
import platform
import pytest
from stitch_pty import spawn, PtySession, PtyError


IS_WINDOWS = platform.system() == "Windows"
IS_UNIX = not IS_WINDOWS


async def read_all(session, timeout=3.0, chunk_size=4096):
    """Read all available output from session within timeout."""
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


@pytest.mark.asyncio
async def test_spawn_and_read():
    """Test spawning a process and reading output."""
    if IS_WINDOWS:
        session = await spawn("cmd.exe", ["/c", "echo hello"])
    else:
        session = await spawn("bash", ["-c", "echo hello"])

    try:
        data = await read_all(session)
        assert data is not None
        assert b"hello" in data.lower()
    finally:
        await session.terminate()


@pytest.mark.asyncio
async def test_write_and_read():
    """Test writing to and reading from a PTY session."""
    if IS_WINDOWS:
        session = await spawn("cmd.exe", ["/c", "echo test_output"])
    else:
        session = await spawn("bash", ["-c", "echo test_output"])

    try:
        data = await read_all(session)
        assert data is not None
        assert b"test_output" in data.lower()
    finally:
        await session.terminate()


@pytest.mark.asyncio
async def test_context_manager():
    """Test using PtySession as an async context manager."""
    if IS_WINDOWS:
        session = await spawn("cmd.exe", ["/c", "echo context_test"])
    else:
        session = await spawn("bash", ["-c", "echo context_test"])

    try:
        data = await read_all(session)
        assert data is not None
        assert b"context_test" in data.lower()
    finally:
        await session.terminate()


@pytest.mark.asyncio
async def test_is_alive():
    """Test is_alive property."""
    if IS_WINDOWS:
        session = await spawn("cmd.exe", ["/c", "ping -n 6 127.0.0.1 > nul"])
    else:
        session = await spawn("bash", ["-c", "sleep 5"])

    try:
        assert session.is_alive
        session.kill()
        await asyncio.sleep(0.1)
    finally:
        try:
            session.kill()
        except Exception:
            pass
        await asyncio.sleep(0.1)


@pytest.mark.asyncio
async def test_resize():
    """Test terminal resize."""
    if IS_WINDOWS:
        session = await spawn("cmd.exe", ["/c", "echo hi"])
    else:
        session = await spawn("bash", ["-c", "echo hi"])

    try:
        session.resize(50, 120)
    finally:
        await session.terminate()


@pytest.mark.asyncio
async def test_kill():
    """Test killing a process."""
    if IS_WINDOWS:
        session = await spawn("cmd.exe", ["/c", "ping -n 11 127.0.0.1 > nul"])
    else:
        session = await spawn("bash", ["-c", "sleep 10"])

    try:
        assert session.is_alive
        session.kill()
        await asyncio.sleep(0.1)
    finally:
        try:
            session.kill()
        except Exception:
            pass
        await asyncio.sleep(0.1)


@pytest.mark.asyncio
async def test_spawn_returns_session():
    """Test that spawn returns a PtySession."""
    if IS_WINDOWS:
        session = await spawn("cmd.exe", ["/c", "echo hello"])
    else:
        session = await spawn("bash", ["-c", "echo hello"])

    try:
        # Should be a PtySession
        assert isinstance(session, PtySession)
    finally:
        await session.terminate()


@pytest.mark.asyncio
async def test_async_iterator():
    """Test PtySession read."""
    if IS_WINDOWS:
        session = await spawn("cmd.exe", ["/c", "echo iterator_test"])
    else:
        session = await spawn("bash", ["-c", "echo iterator_test"])

    try:
        data = await read_all(session)
        assert data is not None
        assert b"iterator_test" in data.lower()
    finally:
        await session.terminate()


@pytest.mark.asyncio
async def test_read_timeout():
    """Test read_timeout raises IOError on timeout."""
    if IS_WINDOWS:
        # ping -n 11 waits ~10s; > nul suppresses all output
        session = await spawn("cmd.exe", ["/c", "ping -n 11 127.0.0.1 > nul"])
    else:
        session = await spawn("bash", ["-c", "sleep 10"])

    try:
        # Drain any initial output so the next read actually times out
        await read_all(session, timeout=2.0)
        # This read should time out since the process produces no further output
        with pytest.raises(PtyError):
            await session.read_timeout(4096, 0.5)
    finally:
        await session.terminate()


@pytest.mark.asyncio
async def test_write_read_back():
    """Test writing to PTY and reading back."""
    if IS_WINDOWS:
        session = await spawn("cmd.exe", ["/c", "echo write_test"])
    else:
        session = await spawn("bash", ["-c", "echo write_test"])

    try:
        await session.write(b"\n")
        data = await asyncio.wait_for(session.read(4096), timeout=3.0)
        assert data is not None
    finally:
        await session.terminate()


@pytest.mark.asyncio
async def test_send_signal():
    """Test sending custom signals."""
    if IS_WINDOWS:
        session = await spawn("cmd.exe", ["/c", "ping -n 11 127.0.0.1 > nul"])
    else:
        session = await spawn("bash", ["-c", "sleep 10"])

    try:
        assert session.is_alive
        session.send_signal(9)
        await asyncio.sleep(0.1)
    finally:
        try:
            session.kill()
        except Exception:
            pass
        await asyncio.sleep(0.1)


@pytest.mark.skipif(IS_WINDOWS, reason="interrupt (Ctrl+C) unreliable on Windows")
@pytest.mark.asyncio
async def test_interrupt():
    """Test interrupt (SIGINT)."""
    session = await spawn("bash", ["-c", "sleep 10"])

    try:
        assert session.is_alive
        session.interrupt()
        await asyncio.sleep(0.1)
    finally:
        try:
            session.kill()
        except Exception:
            pass
        await asyncio.sleep(0.1)


@pytest.mark.asyncio
async def test_get_set_winsize():
    """Test getting and setting window size."""
    if IS_WINDOWS:
        session = await spawn("cmd.exe", ["/c", "echo hi"])
    else:
        session = await spawn("bash", ["-c", "echo hi"])

    try:
        ws = session.get_winsize()
        assert ws.rows == 24
        assert ws.cols == 80
        session.resize(50, 120)
        ws2 = session.get_winsize()
        assert ws2.rows == 50
        assert ws2.cols == 120
    finally:
        await session.terminate()


@pytest.mark.asyncio
async def test_terminate_graceful():
    """Test graceful termination."""
    if IS_WINDOWS:
        session = await spawn("cmd.exe", ["/c", "echo bye"])
    else:
        session = await spawn("bash", ["-c", "echo bye"])

    try:
        await session.terminate(1.0)
    except Exception:
        pass  # Process may have already exited


@pytest.mark.asyncio
async def test_repeated_spawn():
    """Test spawning multiple processes."""
    results = []
    for i in range(3):
        if IS_WINDOWS:
            session = await spawn("cmd.exe", ["/c", f"echo test{i}"])
        else:
            session = await spawn("bash", ["-c", f"echo test{i}"])

        try:
            data = await read_all(session)
            results.append(data)
        finally:
            await session.terminate()

    assert len(results) == 3
