"""Tests for the integrated terminal emulation layer."""

import asyncio
import platform

import pytest
from stitch_pty import spawn


@pytest.mark.asyncio
async def test_session_cwd_none_until_reported(idle):
    prog, args = idle()
    session = await spawn(prog, args)
    try:
        assert session.cwd is None
        assert session.terminal.cwd is None
    finally:
        if session.is_alive:
            session.kill()
            await asyncio.sleep(0.05)


@pytest.mark.asyncio
async def test_session_cwd_osc7(idle):
    prog, args = idle()
    session = await spawn(prog, args)
    try:
        session.terminal.feed(b"\x1b]7;file://myhost/home/user%20name\x07")
        assert session.cwd == "/home/user name"
    finally:
        if session.is_alive:
            session.kill()
            await asyncio.sleep(0.05)


@pytest.mark.asyncio
async def test_osc_payloads_rejoin_semicolons(idle):
    """';' splits OSC params — payloads must be rejoined, not concatenated
    (v0.7.5 review finding: titles and OSC 7 paths silently dropped ';')."""
    prog, args = idle()
    session = await spawn(prog, args)
    try:
        session.terminal.feed(b"\x1b]0;my;title\x07")
        assert session.terminal.title == "my;title"
        session.terminal.feed(b"\x1b]7;file://host/docs;old\x07")
        assert session.cwd == "/docs;old"
        # The 9;9 branch always rejoined correctly — pin it too.
        session.terminal.feed(b"\x1b]9;9;C:\\docs;old\x07")
        assert session.cwd == "C:\\docs;old"
    finally:
        if session.is_alive:
            session.kill()
            await asyncio.sleep(0.05)


@pytest.mark.asyncio
async def test_terminal_resize_zero_clamped(idle):
    """Zero geometry must clamp to 1, not panic on the next feed
    (v0.7.5 review finding: resize(0, 80) then feed panicked in Rust)."""
    prog, args = idle()
    session = await spawn(prog, args)
    try:
        session.terminal.resize(0, 80)
        session.terminal.feed(b"hello")  # must not panic
        assert any("hello" in row for row in session.display)
        session.terminal.resize(24, 0)
        session.terminal.feed(b"\x1b[1K")  # EL-1 with clamped columns
        assert len(session.display) == 24
    finally:
        if session.is_alive:
            session.kill()
            await asyncio.sleep(0.05)


@pytest.mark.asyncio
async def test_poll_events_ordered(idle):
    prog, args = idle()
    session = await spawn(prog, args)
    try:
        session.terminal.feed(b"\x1b]2;t\x07\x07\x1b[?1049h")
        assert session.poll_events() == [
            ("title", "t"),
            ("bell", None),
            ("altscreen", True),
        ]
        # Draining twice: second drain is empty.
        assert session.poll_events() == []
    finally:
        if session.is_alive:
            session.kill()
            await asyncio.sleep(0.05)


@pytest.mark.asyncio
async def test_poll_events_bell_consistency(idle):
    prog, args = idle()
    session = await spawn(prog, args)
    try:
        session.terminal.feed(b"\x07")
        assert ("bell", None) in session.poll_events()
        assert session.take_bell() is False  # the drain consumed it
        session.terminal.feed(b"\x07\x07")
        assert session.take_bell() is True  # coalesced pair → one True
        assert ("bell", None) not in session.poll_events()
    finally:
        if session.is_alive:
            session.kill()
            await asyncio.sleep(0.05)


@pytest.mark.asyncio
async def test_session_cwd_osc9_9(idle):
    prog, args = idle()
    session = await spawn(prog, args)
    try:
        session.terminal.feed(b"\x1b]9;9;C:\\Users\\Main\x1b\\")
        assert session.cwd == "C:\\Users\\Main"
    finally:
        if session.is_alive:
            session.kill()
            await asyncio.sleep(0.05)


IS_WINDOWS = platform.system() == "Windows"


@pytest.mark.asyncio
async def test_terminal_display_empty():
    """Test that display returns a list of strings."""
    if IS_WINDOWS:
        session = await spawn("cmd.exe", ["/c", "echo hello"])
    else:
        session = await spawn("bash", ["-c", "echo hello"])

    try:
        data = await session.read(4096)
        assert data is not None
        # display should be a list of strings
        display = session.display
        assert isinstance(display, list)
        for line in display:
            assert isinstance(line, str)
    finally:
        await session.terminate()


@pytest.mark.asyncio
async def test_terminal_scrollback_accessible():
    """Test that scrollback property is accessible."""
    if IS_WINDOWS:
        session = await spawn("cmd.exe", ["/c", "echo scrollback_test"])
    else:
        session = await spawn("bash", ["-c", "echo scrollback_test"])

    try:
        data = await session.read(4096)
        assert data is not None
        # scrollback should be a list
        scrollback = session.scrollback
        assert isinstance(scrollback, list)
    finally:
        await session.terminate()


@pytest.mark.asyncio
async def test_terminal_full_display():
    """Test that full_display includes history + visible."""
    if IS_WINDOWS:
        session = await spawn("cmd.exe", ["/c", "echo full_display_test"])
    else:
        session = await spawn("bash", ["-c", "echo full_display_test"])

    try:
        data = await session.read(4096)
        assert data is not None
        full = session.full_display
        assert isinstance(full, list)
        # Should have at least as many lines as visible
        assert len(full) >= len(session.display)
    finally:
        await session.terminate()


@pytest.mark.asyncio
async def test_terminal_cursor_position():
    """Test that cursor position is accessible."""
    if IS_WINDOWS:
        session = await spawn("cmd.exe", ["/c", "echo cursor_test"])
    else:
        session = await spawn("bash", ["-c", "echo cursor_test"])

    try:
        data = await session.read(4096)
        assert data is not None
        cursor = session.terminal
        # cursor_x and cursor_y should be accessible
        cx = cursor.cursor_x
        cy = cursor.cursor_y
        assert isinstance(cx, int)
        assert isinstance(cy, int)
    finally:
        await session.terminate()


@pytest.mark.asyncio
async def test_terminal_title_from_osc():
    """Test that OSC title sequences are parsed."""
    if IS_WINDOWS:
        session = await spawn("cmd.exe", ["/c", "echo title_test"])
    else:
        session = await spawn("bash", ["-c", "printf '\\033]2;Test Title\\007'"])

    try:
        data = await session.read(4096)
        assert data is not None
        # Title should be accessible
        title = session.terminal.title
        assert isinstance(title, str)
    finally:
        await session.terminate()


@pytest.mark.asyncio
async def test_terminal_history_size():
    """Test that history_size is accessible."""
    if IS_WINDOWS:
        session = await spawn("cmd.exe", ["/c", "echo history_test"])
    else:
        session = await spawn("bash", ["-c", "echo history_test"])

    try:
        data = await session.read(4096)
        assert data is not None
        hs = session.terminal.history_size
        assert isinstance(hs, int)
        assert hs >= 0
    finally:
        await session.terminate()


@pytest.mark.asyncio
async def test_terminal_direct_feed():
    """Test feeding raw bytes directly into the terminal state machine."""
    if IS_WINDOWS:
        session = await spawn("cmd.exe", ["/c", "echo direct_feed_test"])
    else:
        session = await spawn("bash", ["-c", "echo direct_feed_test"])

    try:
        data = await session.read(4096)
        assert data is not None

        # Direct feed ANSI escape sequences
        session.terminal.feed(b"\x1b[31mred text\x1b[0m\n")
        display = session.display
        assert isinstance(display, list)
    finally:
        await session.terminate()


@pytest.mark.asyncio
async def test_terminal_dirty_rows():
    """Test that dirty row indices are accessible."""
    if IS_WINDOWS:
        session = await spawn("cmd.exe", ["/c", "echo dirty_test"])
    else:
        session = await spawn("bash", ["-c", "echo dirty_test"])

    try:
        data = await session.read(4096)
        assert data is not None
        dirty = session.terminal.dirty()
        assert isinstance(dirty, list)
        for row in dirty:
            assert isinstance(row, int)
    finally:
        await session.terminate()


@pytest.mark.asyncio
async def test_visible_geometry_cheap(idle):
    """visible_lines/visible_columns are O(1) and track resizes.

    v0.8.0 integration finding: without these, the only way to measure the
    visible grid from Python was len(visible_display()), which builds the
    whole screen as Python strings just to count rows.
    """
    prog, args = idle()
    session = await spawn(prog, args)
    try:
        t = session.terminal
        assert t.visible_lines == len(t.visible_display())
        assert t.visible_columns == 80
        assert session.visible_lines == t.visible_lines
        assert session.visible_columns == t.visible_columns
        t.resize(10, 40)
        assert (t.visible_lines, t.visible_columns) == (10, 40)
        assert (session.visible_lines, session.visible_columns) == (10, 40)
        assert t.visible_lines == len(t.visible_display())
    finally:
        await session.terminate()


@pytest.mark.asyncio
async def test_event_log_bounded(idle):
    """The event log caps at 1024 entries, drop-oldest, newest retained.

    v0.8.0 integration finding: a consumer that only calls styled_range
    (never drains) leaked one TitleChanged per shell prompt, forever.
    """
    prog, args = idle()
    session = await spawn(prog, args)
    try:
        t = session.terminal
        # OSC 2 (title only): one event per feed. (OSC 0 would emit an
        # additional icon event per feed — also capped, tested in Rust.)
        for i in range(1100):
            t.feed(f"\x1b]2;t{i}\x07".encode())
        events = session.poll_events()
        assert len(events) == 1024
        assert events[0] == ("title", "t76")
        assert events[-1] == ("title", "t1099")
        assert session.poll_events() == []
    finally:
        await session.terminate()
