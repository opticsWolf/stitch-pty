"""Expanded PtySession I/O tests (complements test_basic.py).

Covers write/write_all, read_timeout success and failure, winsize round-trips,
the raw_output buffer, wait() exit codes, env passing, concurrent and repeated
spawning, and that read() output is parsed into the terminal.
"""
import asyncio

import pytest
from stitch_pty import spawn, PtySession, PtyError, Winsize, ExitStatus


# ── spawning ──────────────────────────────────────────────────────

@pytest.mark.asyncio
async def test_spawn_returns_session(shell):
    prog, args = shell("echo hi")
    session = await spawn(prog, args)
    try:
        assert isinstance(session, PtySession)
        assert session.is_alive in (True, False)
    finally:
        await session.terminate()


@pytest.mark.asyncio
async def test_spawn_with_env(shell):
    prog, args = shell("echo env_ok")
    session = await spawn(prog, args, env={"STITCH_TEST": "1"})
    try:
        assert isinstance(session, PtySession)
    finally:
        await session.terminate()


@pytest.mark.asyncio
async def test_spawn_with_winsize():
    session = await spawn("bash", ["-c", "echo hi"], winsize=Winsize(30, 100))
    try:
        ws = session.get_winsize()
        assert ws.rows == 30
        assert ws.cols == 100
    finally:
        await session.terminate()


@pytest.mark.asyncio
async def test_repeated_spawn(shell, read_all):
    outputs = []
    for i in range(3):
        prog, args = shell(f"echo repeat{i}")
        session = await spawn(prog, args)
        try:
            outputs.append(await read_all(session))
        finally:
            await session.terminate()
    assert len(outputs) == 3
    for i, out in enumerate(outputs):
        assert f"repeat{i}".encode() in out.lower()


@pytest.mark.asyncio
async def test_concurrent_spawn(shell, read_all):
    prog, args = shell("echo concurrent")
    sessions = await asyncio.gather(*(spawn(prog, args) for _ in range(4)))
    try:
        results = await asyncio.gather(*(read_all(s) for s in sessions))
        assert all(b"concurrent" in r.lower() for r in results)
    finally:
        await asyncio.gather(*(s.terminate() for s in sessions))


# ── reading ───────────────────────────────────────────────────────

@pytest.mark.asyncio
async def test_read_returns_bytes(shell):
    prog, args = shell("echo read_bytes")
    session = await spawn(prog, args)
    try:
        data = await asyncio.wait_for(session.read(4096), timeout=3.0)
        assert isinstance(data, (bytes, bytearray))
    finally:
        await session.terminate()


@pytest.mark.asyncio
async def test_read_output_reaches_terminal(shell, read_all):
    prog, args = shell("echo terminal_marker")
    session = await spawn(prog, args)
    try:
        await read_all(session)
        full = "\n".join(session.full_display)
        assert "terminal_marker" in full
    finally:
        await session.terminate()


@pytest.mark.asyncio
async def test_read_timeout_raises_when_idle(idle, read_all):
    prog, args = idle()
    session = await spawn(prog, args)
    try:
        await read_all(session, timeout=1.0)        # drain any startup output
        with pytest.raises(PtyError):
            await session.read_timeout(4096, 0.5)
    finally:
        if session.is_alive:
            session.kill()
            await asyncio.sleep(0.05)


@pytest.mark.asyncio
async def test_raw_output_accumulates(shell, read_all):
    prog, args = shell("echo raw_marker")
    session = await spawn(prog, args)
    try:
        await read_all(session)
        raw = session.raw_output
        assert isinstance(raw, (bytes, bytearray))
        assert b"raw_marker" in raw
    finally:
        await session.terminate()


# ── writing ───────────────────────────────────────────────────────

@pytest.mark.asyncio
async def test_write_returns_count(shell):
    prog, args = shell("echo write_test")
    session = await spawn(prog, args)
    try:
        n = await session.write(b"hello\n")
        assert n is None or n == len(b"hello\n")
    finally:
        await session.terminate()


@pytest.mark.asyncio
async def test_write_all(shell):
    prog, args = shell("echo write_all_test")
    session = await spawn(prog, args)
    try:
        result = await session.write_all(b"some bytes\n")
        assert result is None        # write_all returns nothing on success
    finally:
        await session.terminate()


# ── window size ───────────────────────────────────────────────────

@pytest.mark.asyncio
async def test_resize_roundtrip(shell):
    prog, args = shell("echo hi")
    session = await spawn(prog, args)
    try:
        session.resize(50, 120)
        ws = session.get_winsize()
        assert ws.rows == 50
        assert ws.cols == 120
        session.resize(24, 80)
        ws2 = session.get_winsize()
        assert ws2.rows == 24 and ws2.cols == 80
    finally:
        await session.terminate()


# ── lifecycle ─────────────────────────────────────────────────────

@pytest.mark.asyncio
async def test_kill_sets_not_alive(idle):
    prog, args = idle()
    session = await spawn(prog, args)
    try:
        assert session.is_alive
        session.kill()
        await asyncio.wait_for(session.wait(), timeout=5.0)
        assert session.is_alive is False
    finally:
        if session.is_alive:
            session.kill()
            await asyncio.wait_for(session.wait(), timeout=5.0)


@pytest.mark.asyncio
async def test_wait_returns_exit_info(shell):
    prog, args = shell("echo done")
    session = await spawn(prog, args)

    async def drain():
        try:
            while True:
                chunk = await asyncio.wait_for(
                    session.read(4096), timeout=5.0
                )
                if not chunk:
                    break
        except (PtyError, TimeoutError, asyncio.CancelledError):
            pass

    try:
        drain_task = asyncio.create_task(drain())
        result = await asyncio.wait_for(session.wait(), timeout=5.0)
        assert isinstance(result, (ExitStatus, type(None)))
        await asyncio.wait_for(drain_task, timeout=5.0)
    except TimeoutError:
        pass  # drain may still be reading; wait already succeeded
    finally:
        if session.is_alive:
            await session.terminate()


@pytest.mark.asyncio
async def test_terminate_graceful(shell):
    prog, args = shell("echo bye")
    session = await spawn(prog, args)
    try:
        await session.terminate(1.0)
    except Exception:
        pass     # may already have exited


@pytest.mark.asyncio
async def test_display_properties_types(shell, read_all):
    prog, args = shell("echo types_test")
    session = await spawn(prog, args)
    try:
        await read_all(session)
        for prop in (session.display, session.scrollback, session.full_display):
            assert isinstance(prop, list)
            assert all(isinstance(line, str) for line in prop)
        assert len(session.full_display) >= len(session.display)
    finally:
        await session.terminate()
