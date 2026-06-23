"""Tests for the live terminal mode flags exposed by ``TerminalState``.

These drive the terminal state machine directly with ``feed()``, so they are
fully deterministic and need no PTY. They cover the mode getters:
``app_cursor``, ``cursor_visible``, ``bracketed_paste``, ``sgr_mouse``,
``mouse_proto``, ``cursor_shape``, and ``cursor_blink`` (``alt_screen`` is
covered in test_terminal_altscreen.py).
"""
import pytest
from stitch_pty import TerminalState


def term(cols=80, lines=24, scrollback=1000):
    return TerminalState(cols, lines, scrollback)


class TestModeDefaults:
    """A fresh terminal reports sensible defaults."""

    def test_defaults(self):
        t = term()
        assert t.app_cursor is False
        assert t.cursor_visible is True          # DECTCEM on by default
        assert t.bracketed_paste is False
        assert t.sgr_mouse is False
        assert t.alt_screen is False
        assert t.mouse_proto == 0
        assert t.cursor_shape == "block"
        assert t.cursor_blink is True


class TestPrivateModeToggles:
    def test_app_cursor_decckm(self):
        t = term()
        t.feed(b"\x1b[?1h")
        assert t.app_cursor is True
        t.feed(b"\x1b[?1l")
        assert t.app_cursor is False

    def test_cursor_visible_dectcem(self):
        t = term()
        t.feed(b"\x1b[?25l")
        assert t.cursor_visible is False
        t.feed(b"\x1b[?25h")
        assert t.cursor_visible is True

    def test_bracketed_paste(self):
        t = term()
        t.feed(b"\x1b[?2004h")
        assert t.bracketed_paste is True
        t.feed(b"\x1b[?2004l")
        assert t.bracketed_paste is False

    def test_sgr_mouse(self):
        t = term()
        t.feed(b"\x1b[?1006h")
        assert t.sgr_mouse is True
        t.feed(b"\x1b[?1006l")
        assert t.sgr_mouse is False


class TestMouseProtocol:
    def test_x10(self):
        t = term()
        t.feed(b"\x1b[?1000h")
        assert t.mouse_proto == 1000

    def test_button_event(self):
        t = term()
        t.feed(b"\x1b[?1002h")
        assert t.mouse_proto == 1002

    def test_any_event(self):
        t = term()
        t.feed(b"\x1b[?1003h")
        assert t.mouse_proto == 1003

    def test_precedence_highest_wins(self):
        t = term()
        t.feed(b"\x1b[?1000h\x1b[?1002h")
        assert t.mouse_proto == 1002        # button-event over normal
        t.feed(b"\x1b[?1003h")
        assert t.mouse_proto == 1003        # any-event is highest
        t.feed(b"\x1b[?1003l")
        assert t.mouse_proto == 1002        # falls back when cleared

    def test_all_off(self):
        t = term()
        t.feed(b"\x1b[?1003h")
        t.feed(b"\x1b[?1000l\x1b[?1002l\x1b[?1003l")
        assert t.mouse_proto == 0


class TestCursorStyle:
    """DECSCUSR (CSI Ps SP q) drives cursor shape and blink."""

    @pytest.mark.parametrize("style_id,shape,blink", [
        (0, "block", True),       # default: blinking block
        (1, "block", True),       # blinking block
        (2, "block", False),      # steady block
        (3, "underline", True),   # blinking underline
        (4, "underline", False),  # steady underline
        (5, "bar", True),         # blinking bar
        (6, "bar", False),        # steady bar
    ])
    def test_decscusr(self, style_id, shape, blink):
        t = term()
        t.feed(f"\x1b[{style_id} q".encode())   # note the space before q
        assert t.cursor_shape == shape
        assert t.cursor_blink is blink

    def test_shape_changes_are_sticky(self):
        t = term()
        t.feed(b"\x1b[4 q")
        assert t.cursor_shape == "underline"
        t.feed(b"\x1b[2 q")
        assert t.cursor_shape == "block"


class TestModeIsolation:
    def test_modes_are_independent(self):
        t = term()
        t.feed(b"\x1b[?1h\x1b[?2004h\x1b[?1006h")
        assert t.app_cursor is True
        assert t.bracketed_paste is True
        assert t.sgr_mouse is True
        assert t.cursor_visible is True         # untouched
        t.feed(b"\x1b[?1l")
        assert t.app_cursor is False
        assert t.bracketed_paste is True        # still set

    def test_reset_restores_defaults(self):
        t = term()
        t.feed(b"\x1b[?1h\x1b[?2004h\x1b[?1006h\x1b[5 q\x1b[?25l")
        t.reset()
        assert t.app_cursor is False
        assert t.bracketed_paste is False
        assert t.sgr_mouse is False
        assert t.cursor_visible is True
        assert t.cursor_shape == "block"
        assert t.cursor_blink is True
        assert t.mouse_proto == 0
