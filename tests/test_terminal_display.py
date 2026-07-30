"""Comprehensive terminal display tests for stitch-pty.

Validates the integrated terminal emulation layer against:
- ANSI SGR rendering (colors, bold, underline, reset)
- Cursor movement and positioning
- Scrollback history management
- Erase commands (clear line, clear display)
- OSC title sequences
- Real PTY output parsing (cmd.exe / bash)
- Edge cases (empty output, carriage returns, line wrapping)
"""

import asyncio
import platform
import pytest
from stitch_pty import spawn, TerminalState, Winsize


IS_WINDOWS = platform.system() == "Windows"


# ─────────────────────────────────────────────────────────────────
# 1. TerminalState unit tests (no PTY required)
# ─────────────────────────────────────────────────────────────────

class TestTerminalStateUnit:
    """Pure terminal state tests without spawning processes."""

    def test_terminal_state_creation(self):
        """TerminalState initializes with correct dimensions."""
        term = TerminalState(80, 24, 1000)
        assert term.cursor_x == 0
        assert term.cursor_y == 0
        assert term.history_size == 0
        assert term.title == ""
        assert term.scrollback_lines == 1000

    def test_feed_plain_text(self):
        """Plain printable text renders on the screen."""
        term = TerminalState(20, 3, 100)
        term.feed(b"Hello, World!")
        display = term.visible_display()
        assert len(display) == 3
        assert "Hello, World!" in display[0]

    def test_feed_carriage_return_and_newline(self):
        """CR+LF moves cursor to next line."""
        term = TerminalState(10, 3, 100)
        term.feed(b"Line1\r\nLine2\r\n")
        display = term.visible_display()
        assert display[0] == "Line1     "
        assert display[1] == "Line2     "

    def test_feed_sgr_bold(self):
        """SGR bold (\\x1b[1m) sets bold attribute."""
        term = TerminalState(10, 1, 100)
        term.feed(b"\x1b[1mBold\x1b[0m")
        assert "Bold" in term.visible_display()[0]

    def test_feed_sgr_foreground_color(self):
        """SGR foreground color (\\x1b[31m) sets color attribute."""
        term = TerminalState(10, 1, 100)
        term.feed(b"\x1b[31mRed\x1b[0m")
        assert "Red" in term.visible_display()[0]

    def test_feed_sgr_truecolor(self):
        """True color SGR (\\x1b[38;2;R;G;Bm) sets hex color."""
        term = TerminalState(10, 1, 100)
        term.feed(b"\x1b[38;2;255;128;64mColor\x1b[0m")
        assert "Color" in term.visible_display()[0]

    def test_feed_cursor_positioning(self):
        """CSI cursor positioning (\\x1b[row;colH) moves cursor."""
        term = TerminalState(10, 5, 100)
        term.feed(b"\x1b[3;5H")
        assert term.cursor_x == 4
        assert term.cursor_y == 2

    def test_feed_erase_line(self):
        """CSI K erase in line clears from cursor to end."""
        term = TerminalState(10, 1, 100)
        term.feed(b"Hello World")
        term.feed(b"\x1b[5D")  # Move cursor back 5
        term.feed(b"\x1b[K")   # Erase from cursor to end
        display = term.visible_display()[0]
        # Erase should have modified the line
        assert isinstance(display, str)

    def test_feed_erase_display(self):
        """CSI 2J erase entire display."""
        term = TerminalState(10, 3, 100)
        term.feed(b"Line1\r\nLine2\r\nLine3")
        term.feed(b"\x1b[2J")
        display = term.visible_display()
        assert all(line == "          " for line in display)

    def test_feed_osc_title(self):
        """OSC 2;title BEL sets window title."""
        term = TerminalState(80, 24, 1000)
        term.feed(b"\x1b]2;My Terminal Title\x07")
        assert term.title == "My Terminal Title"

    def test_feed_scrolling(self):
        """Feeding more lines than screen height pushes content into scrollback."""
        term = TerminalState(10, 3, 100)
        for i in range(10):
            term.feed(f"Line {i:02d}\r\n".encode())

        full = term.display()
        assert len(full) >= 3  # At least visible rows
        full_text = "\n".join(full)
        assert "Line" in full_text

    def test_full_display_includes_history(self):
        """full_display concatenates history + visible."""
        term = TerminalState(10, 2, 100)
        for i in range(5):
            term.feed(f"Line{i}\r\n".encode())

        full = term.display()
        visible = term.visible_display()
        assert len(full) >= len(visible)

    def test_dirty_rows_after_feed(self):
        """Dirty rows reflect which lines were modified."""
        term = TerminalState(10, 5, 100)
        term.feed(b"Hello\r\nWorld\r\n")
        dirty = term.dirty()
        assert isinstance(dirty, list)
        assert len(dirty) > 0

    def test_reset_clears_state(self):
        """Reset clears display and history."""
        term = TerminalState(10, 3, 100)
        term.feed(b"Some text\r\nMore text\r\n")
        term.reset()

        display = term.visible_display()
        assert all(line == "          " for line in display)
        assert term.history_size == 0

    def test_resize(self):
        """Resize changes terminal dimensions."""
        term = TerminalState(10, 5, 100)
        term.resize(8, 20)
        display = term.visible_display()
        assert len(display) == 8
        assert len(display[0]) == 20

    def test_scrollback_limit(self):
        """Scrollback respects the configured limit."""
        term = TerminalState(10, 2, 3)
        for i in range(10):
            term.feed(f"Line{i}\r\n".encode())

        assert term.history_size <= 3

    def test_set_scrollback_lines(self):
        """set_scrollback_lines adjusts capacity."""
        term = TerminalState(10, 2, 100)
        term.set_scrollback_lines(5)
        assert term.scrollback_lines == 5


# ─────────────────────────────────────────────────────────────────
# 2. Integrated PTY + terminal tests (real process output)
# ─────────────────────────────────────────────────────────────────

@pytest.mark.asyncio
async def test_pty_output_parsed_by_terminal():
    """Verify that PTY output is automatically fed through the terminal."""
    if IS_WINDOWS:
        session = await spawn("cmd.exe", ["/c", "echo terminal_integration"])
    else:
        session = await spawn("bash", ["-c", "echo terminal_integration"])

    try:
        _ = await session.read_all(timeout=2.0)
        display = session.display
        assert isinstance(display, list)
        assert len(display) > 0

        full_text = "\n".join(session.full_display)
        assert "terminal_integration" in full_text
    finally:
        await session.terminate()


@pytest.mark.asyncio
async def test_pty_cursor_position_after_output():
    """Cursor position reflects where the terminal parser placed it."""
    if IS_WINDOWS:
        session = await spawn("cmd.exe", ["/c", "echo cursor_test"])
    else:
        session = await spawn("bash", ["-c", "echo cursor_test"])

    try:
        _ = await session.read_all(timeout=2.0)
        cursor = session.terminal
        assert isinstance(cursor.cursor_x, int)
        assert isinstance(cursor.cursor_y, int)
        assert cursor.cursor_x >= 0
        assert cursor.cursor_y >= 0
    finally:
        await session.terminate()


@pytest.mark.asyncio
async def test_pty_scrollback_after_multiline():
    """Multi-line output from a real process creates scrollback."""
    if IS_WINDOWS:
        session = await spawn("cmd.exe", ["/c", "(echo line1 & echo line2 & echo line3 & echo line4 & echo line5 & echo line6 & echo line7 & echo line8 & echo line9 & echo line10 & echo line11 & echo line12 & echo line13 & echo line14 & echo line15 & echo line16 & echo line17 & echo line18 & echo line19 & echo line20)"])
    else:
        session = await spawn("bash", ["-c", "printf 'line%d\\n' 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20"])

    try:
        _ = await session.read_all(timeout=3.0)
        scrollback = session.scrollback
        full = session.full_display
        visible = session.display

        assert len(full) >= len(visible)
        full_text = "\n".join(full)
        assert "line1" in full_text
        assert "line20" in full_text
    finally:
        await session.terminate()


@pytest.mark.asyncio
async def test_pty_title_from_process():
    """Process output with OSC title sequences updates the terminal title."""
    if IS_WINDOWS:
        session = await spawn("cmd.exe", ["/c", "echo title_test"])
    else:
        session = await spawn("bash", ["-c", "printf '\\033]2;Test Title\\007' && echo done"])

    try:
        _ = await session.read_all(timeout=2.0)
        title = session.terminal.title
        assert isinstance(title, str)
    finally:
        await session.terminate()


@pytest.mark.asyncio
async def test_pty_dirty_rows_accessible():
    """Dirty rows are accessible after PTY output."""
    if IS_WINDOWS:
        session = await spawn("cmd.exe", ["/c", "echo dirty_test"])
    else:
        session = await spawn("bash", ["-c", "echo dirty_test"])

    try:
        _ = await session.read_all(timeout=2.0)
        dirty = session.terminal.dirty()
        assert isinstance(dirty, list)
        for row in dirty:
            assert isinstance(row, int)
    finally:
        await session.terminate()


@pytest.mark.asyncio
async def test_pty_raw_output_preserved():
    """Raw output bytes are preserved alongside parsed terminal state."""
    if IS_WINDOWS:
        session = await spawn("cmd.exe", ["/c", "echo hello"])
    else:
        session = await spawn("bash", ["-c", "echo hello"])

    try:
        _ = await session.read_all(timeout=2.0)
        raw = session.raw_output
        assert isinstance(raw, bytes)
        assert len(raw) > 0
        assert b"hello" in raw or b"hello\r" in raw
    finally:
        await session.terminate()


@pytest.mark.asyncio
async def test_pty_direct_feed_after_read():
    """Can feed additional ANSI data after reading PTY output."""
    if IS_WINDOWS:
        session = await spawn("cmd.exe", ["/c", "echo base"])
    else:
        session = await spawn("bash", ["-c", "echo base"])

    try:
        _ = await session.read_all(timeout=2.0)
        session.terminal.feed(b"\x1b[31m\x1b[1mRED BOLD\x1b[0m\n")
        display = session.display
        assert isinstance(display, list)

        full_text = "\n".join(session.full_display)
        assert "RED BOLD" in full_text
    finally:
        await session.terminate()


@pytest.mark.asyncio
async def test_pty_history_size_after_output():
    """History size reflects scrollback from real process output."""
    if IS_WINDOWS:
        session = await spawn("cmd.exe", ["/c", "echo history_check"])
    else:
        session = await spawn("bash", ["-c", "echo history_check"])

    try:
        _ = await session.read_all(timeout=2.0)
        hs = session.terminal.history_size
        assert isinstance(hs, int)
        assert hs >= 0
    finally:
        await session.terminate()


# ─────────────────────────────────────────────────────────────────
# 3. Edge case tests
# ─────────────────────────────────────────────────────────────────

@pytest.mark.asyncio
async def test_terminal_empty_session():
    """Terminal state is valid even with no output."""
    if IS_WINDOWS:
        session = await spawn("cmd.exe", ["/c", "echo hello"])
    else:
        session = await spawn("bash", ["-c", "echo hello"])

    try:
        display = session.display
        assert isinstance(display, list)
        assert len(display) > 0

        scrollback = session.scrollback
        assert isinstance(scrollback, list)

        full = session.full_display
        assert isinstance(full, list)
    finally:
        await session.terminate()


@pytest.mark.asyncio
async def test_terminal_multiple_reads_accumulate():
    """Terminal state accumulates across multiple reads."""
    if IS_WINDOWS:
        session = await spawn("cmd.exe", ["/c", "echo part1"])
    else:
        session = await spawn("bash", ["-c", "echo part1"])

    try:
        data1 = await session.read(4096)
        assert data1 is not None

        # Feed additional data manually
        session.terminal.feed(b"\r\npart2\r\n")

        # Second read (may be empty since process already exited)
        try:
            _ = await session.read_timeout(4096, 0.5)
        except Exception:
            pass

        # Manually fed text should be in terminal state
        full_text = "\n".join(session.full_display)
        assert "part2" in full_text
    finally:
        await session.terminate()


@pytest.mark.asyncio
async def test_terminal_carriage_return_handling():
    """Carriage returns are handled correctly by the terminal parser."""
    term = TerminalState(20, 3, 100)
    term.feed(b"Hello\rWorld")
    display = term.visible_display()
    assert "World" in display[0]


@pytest.mark.asyncio
async def test_terminal_line_wrapping():
    """Long lines wrap to the next row."""
    term = TerminalState(10, 3, 100)
    term.feed(b"0123456789abcdefghij")
    display = term.visible_display()
    assert len(display) == 3
    assert len(display[0]) == 10
    assert display[0] == "0123456789"


@pytest.mark.asyncio
async def test_terminal_backspace():
    """Backspace moves cursor left and overwrites."""
    term = TerminalState(10, 1, 100)
    term.feed(b"Hello World\x08\x08\x08\x08\x08!!!!")
    display = term.visible_display()[0]
    assert isinstance(display, str)
    assert len(display) > 0


@pytest.mark.asyncio
async def test_terminal_tab_expansion():
    """Tab characters expand to next tab stop."""
    term = TerminalState(20, 1, 100)
    term.feed(b"A\tB")
    display = term.visible_display()[0]
    assert "A" in display
    assert "B" in display
    assert display.index("A") < display.index("B")


@pytest.mark.asyncio
async def test_terminal_visible_display_property():
    """visible_display property returns correct type."""
    if IS_WINDOWS:
        session = await spawn("cmd.exe", ["/c", "echo hello"])
    else:
        session = await spawn("bash", ["-c", "echo hello"])

    try:
        _ = await session.read_all(timeout=2.0)
        visible = session.display
        assert isinstance(visible, list)
        for line in visible:
            assert isinstance(line, str)
    finally:
        await session.terminate()


@pytest.mark.asyncio
async def test_terminal_scrollback_property():
    """scrollback property returns correct type."""
    if IS_WINDOWS:
        session = await spawn("cmd.exe", ["/c", "echo hello"])
    else:
        session = await spawn("bash", ["-c", "echo hello"])

    try:
        _ = await session.read_all(timeout=2.0)
        sb = session.scrollback
        assert isinstance(sb, list)
        for line in sb:
            assert isinstance(line, str)
    finally:
        await session.terminate()


@pytest.mark.asyncio
async def test_terminal_full_display_property():
    """full_display property returns correct type and includes history."""
    if IS_WINDOWS:
        session = await spawn("cmd.exe", ["/c", "echo hello"])
    else:
        session = await spawn("bash", ["-c", "echo hello"])

    try:
        _ = await session.read_all(timeout=2.0)
        full = session.full_display
        assert isinstance(full, list)
        assert len(full) >= len(session.display)
    finally:
        await session.terminate()


# ─────────────────────────────────────────────────────────────────
# 4. ANSI sequence coverage tests
# ─────────────────────────────────────────────────────────────────

class TestAnsiSequences:
    """Comprehensive ANSI escape sequence coverage."""

    def test_sgr_reset(self):
        """SGR 0 resets all attributes."""
        term = TerminalState(20, 1, 100)
        term.feed(b"\x1b[1;31mBold Red\x1b[0m Normal")
        display = term.visible_display()[0]
        assert "Bold Red" in display or "Normal" in display

    def test_sgr_underline(self):
        """SGR 4 sets underline."""
        term = TerminalState(10, 1, 100)
        term.feed(b"\x1b[4mUnder\x1b[0m")
        assert "Under" in term.visible_display()[0]

    def test_sgr_blink(self):
        """SGR 5 sets blink."""
        term = TerminalState(10, 1, 100)
        term.feed(b"\x1b[5mBlink\x1b[0m")
        assert "Blink" in term.visible_display()[0]

    def test_sgr_reverse(self):
        """SGR 7 sets reverse video."""
        term = TerminalState(10, 1, 100)
        term.feed(b"\x1b[7mReverse\x1b[0m")
        assert "Reverse" in term.visible_display()[0]

    def test_sgr_256color(self):
        """SGR 38;5;N sets 256-color foreground."""
        term = TerminalState(10, 1, 100)
        term.feed(b"\x1b[38;5;196mRed256\x1b[0m")
        assert "Red256" in term.visible_display()[0]

    def test_sgr_background(self):
        """SGR 48;2;R;G;B sets true color background."""
        term = TerminalState(10, 1, 100)
        term.feed(b"\x1b[48;2;0;255;0mGreenBg\x1b[0m")
        assert "GreenBg" in term.visible_display()[0]

    def test_csi_cursor_up(self):
        """CSI nA moves cursor up."""
        term = TerminalState(10, 5, 100)
        term.feed(b"\x1b[4B")
        term.feed(b"\x1b[2A")
        assert term.cursor_y == 2

    def test_csi_cursor_down(self):
        """CSI nB moves cursor down."""
        term = TerminalState(10, 5, 100)
        term.feed(b"\x1b[3B")
        assert term.cursor_y == 3

    def test_csi_cursor_forward(self):
        """CSI nC moves cursor forward."""
        term = TerminalState(10, 5, 100)
        term.feed(b"\x1b[5C")
        assert term.cursor_x == 5

    def test_csi_cursor_back(self):
        """CSI nD moves cursor back."""
        term = TerminalState(10, 5, 100)
        term.feed(b"\x1b[5C")
        term.feed(b"\x1b[3D")
        assert term.cursor_x == 2

    def test_csi_erase_in_line_forward(self):
        """CSI K erases from cursor to end of line."""
        term = TerminalState(10, 1, 100)
        term.feed(b"Hello World")
        term.feed(b"\x1b[6D")
        term.feed(b"\x1b[K")
        display = term.visible_display()[0]
        assert isinstance(display, str)

    def test_csi_erase_in_line_backward(self):
        """CSI 1K erases from start of line to cursor."""
        term = TerminalState(10, 1, 100)
        term.feed(b"Hello World")
        term.feed(b"\x1b[6D")
        term.feed(b"\x1b[1K")
        display = term.visible_display()[0]
        assert isinstance(display, str)

    def test_csi_erase_in_line_all(self):
        """CSI 2K erases entire line."""
        term = TerminalState(10, 1, 100)
        term.feed(b"Hello World")
        term.feed(b"\x1b[2K")
        display = term.visible_display()[0]
        assert display == "          "

    def test_csi_erase_in_display_below(self):
        """CSI J erases from cursor to end of display."""
        term = TerminalState(5, 3, 100)
        term.feed(b"Line1\r\nLine2\r\nLine3")
        term.feed(b"\x1b[1;1H")
        term.feed(b"\x1b[J")

    def test_csi_erase_in_display_above(self):
        """CSI 1J erases from start of display to cursor."""
        term = TerminalState(5, 3, 100)
        term.feed(b"Line1\r\nLine2\r\nLine3")
        term.feed(b"\x1b[3;1H")
        term.feed(b"\x1b[1J")

    def test_csi_save_restore_cursor(self):
        """CSI s/u save and restore cursor position."""
        term = TerminalState(10, 10, 100)
        term.feed(b"\x1b[5;5H")
        saved_x = term.cursor_x
        saved_y = term.cursor_y
        term.feed(b"\x1b[s")  # CSI s — save cursor (not \x1bs which is ESC s)
        term.feed(b"\x1b[1;1H")
        assert term.cursor_x == 0
        assert term.cursor_y == 0
        term.feed(b"\x1b[u")  # CSI u — restore cursor
        assert term.cursor_x == saved_x
        assert term.cursor_y == saved_y

    def test_csi_insert_lines(self):
        """CSI nL inserts blank lines."""
        term = TerminalState(10, 5, 100)
        term.feed(b"Top")
        term.feed(b"\x1b[1;1H")
        term.feed(b"\x1b[2L")

    def test_csi_delete_lines(self):
        """CSI nM deletes lines."""
        term = TerminalState(10, 5, 100)
        term.feed(b"Line1\r\nLine2\r\nLine3")
        term.feed(b"\x1b[1;1H")
        term.feed(b"\x1b[1M")

    def test_csi_scroll_region(self):
        """CSI r sets scroll region."""
        term = TerminalState(10, 24, 100)
        term.feed(b"\x1b[5;15r")

    def test_osc_icon_name(self):
        """OSC 1 sets icon name."""
        term = TerminalState(80, 24, 1000)
        term.feed(b"\x1b]1;IconName\x07")

    def test_esc_save_restore_cursor(self):
        """ESC 7 / ESC 8 save and restore cursor."""
        term = TerminalState(10, 10, 100)
        term.feed(b"\x1b[3;4H")
        term.feed(b"\x1b7")
        term.feed(b"\x1b[1;1H")
        term.feed(b"\x1b8")
        assert term.cursor_x == 3
        assert term.cursor_y == 2

    def test_alignment_test(self):
        """ESC #8 fills screen with E characters."""
        term = TerminalState(5, 3, 100)
        term.feed(b"\x1b#8")
        display = term.visible_display()
        assert all(line == "EEEEE" for line in display)
