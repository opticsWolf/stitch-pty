"""``styled_viewport()`` structure, color, and attribute tests.

``styled_viewport()`` returns history + visible as a grid of
``(text, fg, bg, attrs)`` cells. Colors are "default", an ANSI name, or 6 hex
digits; ``attrs`` is a bitmask (bit 0 bold, 1 dim, 2 italics, 3 underscore,
4 blink, 5 reverse, 6 hidden, 7 strikethrough).
"""
import pytest
from stitch_pty import TerminalState


def term(cols=20, lines=2, scrollback=100):
    return TerminalState(cols, lines, scrollback)


def cell(t, row, col):
    return t.styled_viewport()[row][col]


class TestStyledStructure:
    def test_grid_shape_matches_total_lines(self):
        t = term(10, 3)
        t.feed(b"hi")
        vp = t.styled_viewport()
        assert len(vp) == t.total_lines()
        for row in vp:
            assert len(row) == 10

    def test_cell_is_four_tuple(self):
        t = term(5, 1)
        t.feed(b"X")
        text, fg, bg, attrs = cell(t, 0, 0)
        assert text == "X"
        assert isinstance(fg, str) and isinstance(bg, str)
        assert isinstance(attrs, int)

    def test_blank_cell_defaults(self):
        t = term(5, 1)
        text, fg, bg, attrs = cell(t, 0, 0)
        assert text == " "
        assert fg == "default"
        assert bg == "default"
        assert attrs == 0


class TestStyledColors:
    def test_named_foreground(self):
        t = term(10, 1)
        t.feed(b"\x1b[31mR")
        assert cell(t, 0, 0)[1] == "red"

    def test_named_background(self):
        t = term(10, 1)
        t.feed(b"\x1b[42mG")
        assert cell(t, 0, 0)[2] == "green"

    def test_sgr_33_is_brown(self):
        # The palette reports SGR 33 as "brown" rather than "yellow".
        t = term(10, 1)
        t.feed(b"\x1b[33mY")
        assert cell(t, 0, 0)[1] == "brown"

    def test_truecolor_foreground_is_hex(self):
        t = term(10, 1)
        t.feed(b"\x1b[38;2;255;128;64mC")
        assert cell(t, 0, 0)[1] == "ff8040"

    def test_truecolor_background_is_hex(self):
        t = term(10, 1)
        t.feed(b"\x1b[48;2;0;255;0mC")
        assert cell(t, 0, 0)[2] == "00ff00"

    def test_256_color_is_six_hex_digits(self):
        t = term(10, 1)
        t.feed(b"\x1b[38;5;196mC")
        fg = cell(t, 0, 0)[1]
        assert len(fg) == 6
        assert all(ch in "0123456789abcdef" for ch in fg)

    def test_default_color_after_39(self):
        t = term(10, 1)
        t.feed(b"\x1b[31mA\x1b[39mB")
        assert cell(t, 0, 0)[1] == "red"
        assert cell(t, 0, 1)[1] == "default"


class TestStyledAttributes:
    @pytest.mark.parametrize("sgr,bit,name", [
        (1, 0, "bold"),
        (2, 1, "dim"),
        (3, 2, "italics"),
        (4, 3, "underscore"),
        (5, 4, "blink"),
        (7, 5, "reverse"),
        (8, 6, "hidden"),
        (9, 7, "strikethrough"),
    ])
    def test_attribute_bit_set(self, sgr, bit, name):
        t = term(10, 1)
        t.feed(f"\x1b[{sgr}mX".encode())
        attrs = cell(t, 0, 0)[3]
        assert attrs & (1 << bit), f"{name} bit not set"

    def test_combined_attributes(self):
        t = term(10, 1)
        t.feed(b"\x1b[1;4mX")             # bold + underline
        attrs = cell(t, 0, 0)[3]
        assert attrs & (1 << 0)           # bold
        assert attrs & (1 << 3)           # underscore

    def test_reset_clears_attributes_and_color(self):
        t = term(10, 1)
        t.feed(b"\x1b[1;31mA\x1b[0mB")
        text, fg, bg, attrs = cell(t, 0, 1)
        assert text == "B"
        assert attrs == 0
        assert fg == "default"
