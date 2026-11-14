"""Kitty keyboard encoder tests (Rust keyboard model + Python surface)."""

import sys

import pytest
from stitch_pty import TerminalState, spawn


def term(cols=80, lines=24, scrollback=1000):
    return TerminalState(cols, lines, scrollback)


class TestTerminalStateEncoder:
    def test_legacy_fallback_shift_enter(self):
        t = term()
        assert t.encode_key("enter", modifiers=1) == b"\r"
        assert t.encode_key("tab") == b"\t"
        assert t.encode_key("backspace") == b"\x7f"
        assert t.encode_key("escape") == b"\x1b"

    def test_flag1_shift_enter_and_exceptions(self):
        t = term()
        t.feed(b"\x1b[>1u")
        assert t.keyboard_mode == 1
        assert t.encode_key("enter", modifiers=1) == b"\x1b[13;2u"
        assert t.encode_key("enter", modifiers=2) == b"\x1b[13;3u"
        assert t.encode_key("enter") == b"\r"
        assert t.encode_key("tab") == b"\t"
        assert t.encode_key("backspace") == b"\x7f"
        assert t.encode_key("escape") == b"\x1b[27u"

    def test_ctrl_i_is_not_tab(self):
        t = term()
        t.feed(b"\x1b[>1u")
        assert t.encode_key("i", modifiers=4) == b"\x1b[105;5u"
        assert t.encode_key("i", modifiers=5) == b"\x1b[105;6u"
        assert t.encode_key("tab") == b"\t"

    def test_flag8_all_keys_and_opt_in_fields(self):
        t = term()
        t.feed(b"\x1b[=8;1u")
        assert t.encode_key("a") == b"\x1b[97u"
        assert t.encode_key("enter") == b"\x1b[13u"
        assert t.encode_key(
            "a", kind="release", flags=8 | 2
        ) == b"\x1b[97;1:3u"
        assert t.encode_key(
            "a", modifiers=1, text="A", flags=8 | 16
        ) == b"\x1b[97;2;65u"
        assert t.encode_key(
            "a", modifiers=1, shifted_key="A", base_layout_key="a",
            flags=8 | 4,
        ) == b"\x1b[97:65:97;2u"

    def test_named_functional_keys(self):
        t = term()
        assert t.encode_key("left") == b"\x1b[D"
        assert t.encode_key("left", modifiers=1) == b"\x1b[1;2D"
        assert t.encode_key("f1") == b"\x1bOP"
        assert t.encode_key("f5") == b"\x1b[15~"
        assert t.encode_key("f13") == b"\x1b[57376u"
        assert t.encode_key("kp_1") == b"\x1b[57400u"

    def test_numeric_key_code(self):
        t = term()
        assert t.encode_key(13, modifiers=1, flags=1) == b"\x1b[13;2u"

    def test_unknown_key_name_raises(self):
        with pytest.raises(ValueError):
            term().encode_key("not-a-key")


@pytest.mark.asyncio
async def test_pty_session_encode_key_uses_current_mode():
    session = await spawn(
        sys.executable,
        ["-c", "import time; time.sleep(5)"],
    )
    try:
        session.terminal.feed(b"\x1b[>1u")
        assert session.keyboard_mode == 1
        assert session.encode_key("enter", modifiers=1) == b"\x1b[13;2u"
        assert session.encode_key("enter") == b"\r"
    finally:
        await session.terminate()
