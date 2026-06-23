"""Scrollback capture and history-aware resize tests.

Verifies that lines scrolled off the top are captured into the scrollback
history, that the styled/plain views stay consistent with ``total_lines()``,
that capacity limits are respected, and that resizing preserves content
(shrink pushes the overflow into history; grow does not drain it).
"""
import pytest
from stitch_pty import TerminalState


def filled(cols, lines, n, scrollback=1000):
    """A terminal with lines ``L0``..``L{n-1}`` fed one per row."""
    t = TerminalState(cols, lines, scrollback)
    for i in range(n):
        t.feed(f"L{i}".encode())
        if i < n - 1:
            t.feed(b"\r\n")
    return t


def rstripped(rows):
    return [r.rstrip() for r in rows]


class TestScrollbackCapture:
    def test_no_history_when_content_fits(self):
        t = filled(20, 5, 5)            # exactly fills the screen
        assert t.history_size == 0

    def test_scroll_pushes_to_history(self):
        t = filled(20, 3, 6)            # 6 lines into a 3-row screen
        assert t.history_size == 3
        hist = "\n".join(t.history_display())
        assert "L0" in hist and "L1" in hist and "L2" in hist

    def test_oldest_line_is_first_in_history(self):
        t = filled(20, 3, 8)
        assert t.history_display()[0].rstrip() == "L0"

    def test_visible_shows_most_recent(self):
        t = filled(20, 3, 6)           # L3, L4, L5 remain visible
        assert rstripped(t.visible_display())[-1] == "L5"

    def test_total_lines_consistent(self):
        t = filled(20, 4, 10)
        assert t.total_lines() == t.history_size + 4
        assert len(t.styled_viewport()) == t.total_lines()
        assert len(t.display()) == t.total_lines()

    def test_display_is_history_plus_visible(self):
        t = filled(20, 3, 7)
        assert t.display() == t.history_display() + t.visible_display()


class TestScrollbackCapacity:
    def test_capacity_limit_respected(self):
        t = filled(10, 2, 20, scrollback=3)
        assert t.history_size <= 3

    def test_capacity_keeps_newest(self):
        t = filled(10, 2, 20, scrollback=3)
        # 18 lines would scroll off; only the 3 newest survive in history.
        assert rstripped(t.history_display())[-1] == "L17"

    def test_set_scrollback_lines_trims(self):
        t = filled(10, 2, 30, scrollback=1000)
        assert t.history_size > 5
        t.set_scrollback_lines(5)
        assert t.scrollback_lines == 5
        assert t.history_size <= 5

    def test_zero_means_unlimited(self):
        t = filled(10, 2, 50, scrollback=0)
        assert t.history_size == 48      # 50 fed - 2 visible


class TestAbsoluteCursor:
    def test_absolute_cursor_includes_history(self):
        t = filled(20, 3, 6)
        cx, cy = t.absolute_cursor()
        assert cy == t.history_size + t.cursor_y
        assert cx == t.cursor_x


class TestHistoryAwareResize:
    def test_shrink_pushes_top_to_history(self):
        t = TerminalState(20, 4, 100)
        t.feed(b"A\r\nB\r\nC\r\nD")     # fills 4 rows, cursor on the last
        assert t.history_size == 0
        t.resize(2, 20)                 # shrink to 2 rows
        assert t.history_size >= 2
        assert t.history_display()[0].rstrip() == "A"
        assert rstripped(t.visible_display())[-1] == "D"

    def test_grow_preserves_history(self):
        t = TerminalState(20, 2, 100)
        t.feed(b"A\r\nB\r\nC\r\nD")     # A,B scroll into history; C,D visible
        before = t.history_size
        assert before >= 2
        t.resize(4, 20)                 # grow must NOT drain scrollback
        assert t.history_size == before
        vis = rstripped(t.visible_display())
        assert vis[0] == "C" and vis[1] == "D"   # content kept, padded below

    def test_resize_changes_dimensions(self):
        t = TerminalState(10, 5, 100)
        t.resize(8, 20)
        assert len(t.visible_display()) == 8
        assert len(t.visible_display()[0]) == 20

    def test_repeated_resize_does_not_lose_history(self):
        t = TerminalState(20, 6, 1000)
        for i in range(12):
            t.feed(f"H{i}".encode())
            if i < 11:
                t.feed(b"\r\n")
        baseline = t.history_size
        # Jitter the height up and down repeatedly (no shell redraw involved).
        for h in [4, 6, 3, 8, 5, 6, 4, 8, 6]:
            t.resize(h, 20)
        # Growing never drains; the original captured lines are never lost.
        assert t.history_size >= baseline
        joined = "\n".join(t.history_display() + t.visible_display())
        assert "H0" in joined
