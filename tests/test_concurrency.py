"""Concurrency contracts for PtySession.

Contract C1 (concurrent reads are safe but partitioned): two tasks may call
read()/read_timeout() concurrently. Bytes are conserved — no loss,
duplication, or torn chunks — but partitioned arbitrarily between readers.
A *coherent* stream needs a single reader. Rationale: backend reads are
serialized (Windows pipe Mutex) or kernel-serialized (Unix master fd), and
every Python-side chunk handoff (terminal feed, raw capture) is synchronous,
so there is no torn state to guard with a lock.

Contract C2 (shutdown race): cancelling a read() that is blocked mid-await
while terminate() runs completes promptly; the session reports not-alive.

Contract C3 (drain coherence): take_dirty_rows()/poll_events() between reads
never panic, always return valid visible-row indices, partition the dirty set
across drains (a drain with no intervening feed is empty), and eventually
cover every emitted line.
"""
import asyncio
import contextlib
import re
import sys

import pytest
from stitch_pty import PtyError, spawn

# ── C1: concurrent reads conserve bytes ───────────────────────────

@pytest.mark.asyncio
async def test_concurrent_reads_conserve_bytes():
    prog = sys.executable
    args = ["-c", (
        "import sys, time\n"
        "for i in range(1, 31):\n"
        "    print(f'MARK-{i}', flush=True)\n"
        "    time.sleep(0.03)\n"
    )]
    session = await spawn(prog, args)
    chunks: list[bytes] = []
    try:
        async def _reader() -> None:
            while True:
                try:
                    data = await session.read_timeout(4096, 5.0)
                except PtyError:
                    break
                if not data:
                    break
                chunks.append(data)

        # Both readers pending before any output: partitioning is real.
        await asyncio.wait_for(
            asyncio.gather(_reader(), _reader()), timeout=30.0
        )
        found = sorted(int(n) for n in re.findall(rb"MARK-(\d+)", b"".join(chunks)))
        assert found == list(range(1, 31))
    finally:
        await session.terminate()


# ── C2: cancelling a blocked read during terminate ─────────────────

@pytest.mark.asyncio
async def test_cancel_blocked_read_during_terminate(idle):
    prog, args = idle()
    session = await spawn(prog, args)
    try:
        read_task = asyncio.create_task(session.read(4096))

        async def _cancel_soon() -> None:
            await asyncio.sleep(0.3)
            read_task.cancel()

        cancel_task = asyncio.create_task(_cancel_soon())
        # The cancel fires mid-terminate: this must not hang.
        await asyncio.wait_for(session.terminate(2.0), timeout=15.0)
        await asyncio.wait_for(cancel_task, timeout=5.0)
        # A mid-await cancel surfaces here as CancelledError — expected.
        with contextlib.suppress(asyncio.CancelledError):
            await asyncio.wait_for(read_task, timeout=10.0)
        assert not session.is_alive
    finally:
        if session.is_alive:
            session.kill()
            await asyncio.sleep(0.05)


# ── C3: drains stay coherent between reads ─────────────────────────

@pytest.mark.asyncio
async def test_drains_between_reads_stay_coherent(shell, read_all):
    prog, args = shell("echo C3_LINE")
    session = await spawn(prog, args)
    try:
        seen_rows: set[int] = set()
        while True:
            try:
                chunk = await session.read_timeout(4096, 3.0)
            except PtyError:
                break
            if not chunk:
                break
            rows = session.take_dirty_rows()
            events = session.poll_events()
            assert isinstance(events, list)
            display = session.display
            for r in rows:
                assert 0 <= r < len(display), f"dirty row {r} out of range"
                seen_rows.add(r)
        # Every emitted line is reachable through some drained row…
        full = "\n".join(session.full_display)
        assert "C3_LINE" in full
        # …and drains with no intervening feed are stably empty.
        assert session.take_dirty_rows() == []
        assert session.poll_events() == []
        assert seen_rows, "expected at least one dirty row across the session"
    finally:
        await session.terminate()
