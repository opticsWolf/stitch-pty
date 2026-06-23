"""Alternate screen buffer tests (?1049 / ?1047 / ?47).

Verifies the real buffer swap: entering shows a blank alternate screen, exiting
restores the primary buffer, ?1049 saves and restores the cursor, scrollback is
not fed while the alt screen is active, and resizing during the alt screen still
restores the primary buffer cleanly.
"""
import pytest
from stitch_pty import TerminalState


def term(cols=20, lines=4, scrollback=100):
    return TerminalState(cols, lines, scrollback)


def rstripped(rows):
    return [r.rstrip() for r in rows]


class TestAltScreenSwap:
    def test_enter_and_exit_1049(self):
        t = term()
        assert t.alt_screen is False
        t.feed(b"\x1b[?1049h")
        assert t.alt_screen is True
        t.feed(b"\x1b[?1049l")
        assert t.alt_screen is False

    def test_alt_buffer_starts_blank(self):
        t = term()
        t.feed(b"primary line")
        t.feed(b"\x1b[?1049h")
        assert t.visible_display()[0].strip() == ""

    def test_primary_restored_on_exit(self):
        t = term()
        t.feed(b"primary line")
        t.feed(b"\x1b[?1049h")
        t.feed(b"\x1b[Halt content")        # home, then draw on the alt screen
        assert "alt content" in t.visible_display()[0]
        t.feed(b"\x1b[?1049l")
        assert "primary line" in t.visible_display()[0]

    def test_1049_saves_and_restores_cursor(self):
        t = term()
        t.feed(b"\x1b[3;5H")                # cursor to row 3, col 5 -> (2, 4)
        saved = (t.cursor_x, t.cursor_y)
        t.feed(b"\x1b[?1049h")
        t.feed(b"\x1b[1;1H")               # move on the alt screen
        assert (t.cursor_x, t.cursor_y) == (0, 0)
        t.feed(b"\x1b[?1049l")
        assert (t.cursor_x, t.cursor_y) == saved

    @pytest.mark.parametrize("enter,leave", [
        (b"\x1b[?47h", b"\x1b[?47l"),
        (b"\x1b[?1047h", b"\x1b[?1047l"),
    ])
    def test_47_and_1047_swap(self, enter, leave):
        t = term()
        t.feed(b"base text")
        t.feed(enter)
        assert t.alt_screen is True
        t.feed(leave)
        assert t.alt_screen is False
        assert "base text" in t.visible_display()[0]

    def test_double_enter_is_idempotent(self):
        t = term()
        t.feed(b"primary")
        t.feed(b"\x1b[?1049h")
        t.feed(b"\x1b[?1049h")             # second enter must not clobber primary
        t.feed(b"\x1b[?1049l")
        assert "primary" in t.visible_display()[0]


class TestAltScreenScrollback:
    def test_no_scrollback_while_alt(self):
        t = term(cols=10, lines=3)
        t.feed(b"\x1b[?1049h")
        before = t.history_size
        for i in range(10):                # scroll a lot on the alt screen
            t.feed(f"x{i}\r\n".encode())
        assert t.history_size == before

    def test_primary_scrollback_survives_alt(self):
        t = term(cols=10, lines=3)
        for i in range(8):                 # build primary scrollback
            t.feed(f"p{i}".encode())
            t.feed(b"\r\n")
        hist_before = t.history_size
        assert hist_before > 0
        t.feed(b"\x1b[?1049h")
        for i in range(5):
            t.feed(f"a{i}\r\n".encode())
        t.feed(b"\x1b[?1049l")
        assert t.history_size == hist_before


class TestAltScreenResize:
    def test_resize_during_alt_restores_primary(self):
        t = term(cols=10, lines=4)
        t.feed(b"P0\r\nP1\r\nP2\r\nP3")
        t.feed(b"\x1b[?1049h")
        t.resize(6, 10)                    # resize while on the alt screen
        assert t.alt_screen is True
        assert len(t.visible_display()) == 6
        t.feed(b"\x1b[?1049l")
        assert len(t.visible_display()) == 6
        vis = rstripped(t.visible_display())
        assert vis[0] == "P0" and vis[3] == "P3"
