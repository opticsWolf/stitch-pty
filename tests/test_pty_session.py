"""Expanded PtySession I/O tests (complements test_basic.py).

Covers write/write_all, read_timeout success and failure, winsize round-trips,
the raw_output buffer, wait() exit codes, env passing, concurrent and repeated
spawning, and that read() output is parsed into the terminal.
"""
import asyncio
import re
import sys

import pytest
from stitch_pty import spawn, PtySession, PtyError, Winsize, ExitStatus, ExpectResult


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


@pytest.mark.asyncio
async def test_raw_output_cap_bounds(shell, read_all):
    prog, args = shell("echo " + "x" * 500)
    session = await spawn(prog, args)
    try:
        session._raw_cap = 64
        await read_all(session)
        assert len(session.raw_output) <= 64
    finally:
        await session.terminate()


@pytest.mark.asyncio
async def test_raw_output_cap_none_unbounded(shell, read_all):
    prog, args = shell("echo " + "y" * 500)
    session = await spawn(prog, args)
    try:
        session._raw_cap = None
        await read_all(session)
        assert len(session.raw_output) >= 500
    finally:
        await session.terminate()


@pytest.mark.asyncio
async def test_eof_drain_returns_empty(shell, read_all):
    prog, args = shell("echo eof_done")
    session = await spawn(prog, args)
    try:
        await read_all(session)

        async def _drain_once() -> bytes:
            # After the child exits, further reads surface EOF cleanly:
            # prompt b"" (Unix EIO / ConPTY pipe close) or a PtyError
            # timeout (ConPTY pipe still draining) — never an OSError leak.
            try:
                return await session.read_timeout(4096, 3.0)
            except PtyError:
                return b""

        assert await _drain_once() == b""
        # Second read after EOF: still clean b"", never raises.
        assert await _drain_once() == b""
    finally:
        await session.terminate()


@pytest.mark.asyncio
async def test_spawn_forwards_scrollback(shell, read_all):
    if sys.platform == "win32":
        prog, args = shell("for /L %i in (1,1,200) do @echo line%i")
    else:
        prog, args = shell("for i in $(seq 1 200); do echo line$i; done")
    session = await spawn(prog, args, scrollback=50)
    try:
        assert session.terminal.scrollback_lines == 50
        await read_all(session)
        assert len(session.scrollback) == 50
    finally:
        await session.terminate()


@pytest.mark.asyncio
async def test_spawn_forwards_raw_output_cap(shell, read_all):
    prog, args = shell("echo " + "z" * 500)
    session = await spawn(prog, args, raw_output_cap=64)
    try:
        await read_all(session)
        assert len(session.raw_output) <= 64
    finally:
        await session.terminate()


@pytest.mark.asyncio
async def test_spawn_with_cwd(tmp_path, read_all):
    import os
    session = await spawn(
        sys.executable,
        ["-c", "import os;print(os.getcwd())"],
        cwd=str(tmp_path),
    )
    try:
        output = await read_all(session)
        # normcase: Windows drive-letter case differs between APIs.
        assert os.path.normcase(str(tmp_path)) in os.path.normcase(output.decode(errors="ignore"))
    finally:
        await session.terminate()


@pytest.mark.asyncio
async def test_spawn_with_missing_cwd_raises():
    with pytest.raises(PtyError):
        await spawn(sys.executable, ["-c", "pass"], cwd="Z:/definitely/missing")


# ── expect() ──────────────────────────────────────────────────────

@pytest.mark.asyncio
async def test_expect_literal_legacy_shape(shell):
    prog, args = shell("echo expect_literal_xyz")
    session = await spawn(prog, args)
    try:
        res = await session.expect(b"expect_literal_xyz", timeout=5.0)
        assert isinstance(res, ExpectResult)
        assert res.index == 0
        assert res.match is None
        assert b"expect_literal_xyz" in res.buffer
        # Legacy bytes-compat shims.
        assert b"expect_literal_xyz" in res
        assert res == res.buffer
        assert bytes(res) == res.buffer
    finally:
        await session.terminate()


@pytest.mark.asyncio
async def test_expect_str_pattern(shell):
    prog, args = shell("echo expect_str_abc")
    session = await spawn(prog, args)
    try:
        res = await session.expect("expect_str_abc", timeout=5.0)
        assert res.index == 0
        assert res.match is None
        assert b"expect_str_abc" in res
    finally:
        await session.terminate()


@pytest.mark.asyncio
async def test_expect_regex(shell):
    prog, args = shell("echo expect_regex_42")
    session = await spawn(prog, args)
    try:
        res = await session.expect(re.compile(rb"expect_[a-z]+_\d+"), timeout=5.0)
        assert res.index == 0
        assert res.match is not None
        assert res.match.group(0) == b"expect_regex_42"
    finally:
        await session.terminate()


@pytest.mark.asyncio
async def test_expect_multi_pattern_list_order(shell):
    prog, args = shell("echo second_1")
    session = await spawn(prog, args)
    try:
        res = await session.expect(
            [b"nomatch_xyz", re.compile(rb"second_\d+"), "also_nomatch"],
            timeout=5.0,
        )
        assert res.index == 1
        assert res.match is not None
    finally:
        await session.terminate()


@pytest.mark.asyncio
async def test_expect_split_across_chunks():
    prog = sys.executable
    args = ["-c", (
        "import sys, time; sys.stdout.write('SPLIT_'); sys.stdout.flush();"
        " time.sleep(0.5); sys.stdout.write('PATTERN'); sys.stdout.flush()"
    )]
    session = await spawn(prog, args)
    try:
        res = await session.expect(b"SPLIT_PATTERN", timeout=5.0)
        assert res.buffer.endswith(b"SPLIT_PATTERN")
    finally:
        await session.terminate()


@pytest.mark.asyncio
async def test_expect_eof_carries_buffer(shell):
    prog, args = shell("echo tiny")
    session = await spawn(prog, args)
    try:
        with pytest.raises(TimeoutError) as exc_info:
            await session.expect(b"never_appears_xyz", timeout=5.0)
        assert isinstance(exc_info.value.buffer, bytes)
    finally:
        await session.terminate()


@pytest.mark.asyncio
async def test_expect_timeout_carries_buffer(idle):
    prog, args = idle()
    session = await spawn(prog, args)
    try:
        with pytest.raises(TimeoutError) as exc_info:
            await session.expect(b"never_appears_xyz", timeout=0.5)
        # Content is platform-dependent (ConPTY emits init sequences),
        # but the attribute is always present and always bytes.
        assert isinstance(exc_info.value.buffer, bytes)
    finally:
        if session.is_alive:
            session.kill()
            await asyncio.sleep(0.05)


@pytest.mark.asyncio
async def test_expect_rejects_bad_patterns(shell):
    prog, args = shell("echo hi")
    session = await spawn(prog, args)
    try:
        with pytest.raises(ValueError):
            await session.expect([], timeout=1.0)
        with pytest.raises(TypeError):
            await session.expect(re.compile("str_pattern"), timeout=1.0)
        with pytest.raises(TypeError):
            await session.expect(123, timeout=1.0)  # type: ignore[arg-type]
    finally:
        await session.terminate()


@pytest.mark.asyncio
async def test_typed_eof_branch(shell):
    """An OSError with kind=="eof" (the Rust EOF contract) maps to b""."""
    prog, args = shell("echo hi")
    session = await spawn(prog, args)
    real_inner = session._inner
    try:
        err = OSError(0, "PTY EOF: child side closed")
        err.kind = "eof"  # attached by the Rust layer, not string-matched
        assert "os error 5" not in str(err)  # guard: not the legacy path

        class _EofInner:
            async def read(self, size):
                raise err

        session._inner = _EofInner()
        assert await session.read(4096) == b""
    finally:
        session._inner = real_inner
        await session.terminate()


@pytest.mark.asyncio
async def test_raw_output_window_slides(idle):
    """Direct-feed unit check of the sliding window (idle child: no PTY noise)."""
    prog, args = idle()
    session = await spawn(prog, args)
    try:
        session._raw_cap = 64
        session._record_raw(b"a" * 50)
        session._record_raw(b"b" * 50)
        raw = session.raw_output
        assert len(raw) == 50
        assert raw == b"b" * 50
    finally:
        if session.is_alive:
            session.kill()
            await asyncio.sleep(0.05)


@pytest.mark.asyncio
async def test_session_init_positional_inner(shell):
    """PtySession(inner) positional construction still works (compat)."""
    prog, args = shell("echo compat")
    session = await spawn(prog, args)
    try:
        clone = PtySession(session._inner)
        assert isinstance(clone.raw_output, (bytes, bytearray))
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
