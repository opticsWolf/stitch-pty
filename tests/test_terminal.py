"""Tests for the integrated terminal emulation layer."""

import asyncio
import platform
import pytest
from stitch_pty import spawn


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
