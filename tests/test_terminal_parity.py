"""Fixed-corpus parity check for Python ``TerminalState``.

The Rust suite (``tests/terminal_parity.rs``) feeds the same bytes into
``HistoryScreen``. ``TerminalState`` delegates to that same model; keep both
corpora in sync when changing this test.
"""

import pytest
from stitch_pty import TerminalState

CORPUS = (
    b"hello \x1b[31mred\x1b[0m\n"
    b"second line\x1b]2;parity-title\x07\n"
    b"utf8: \xc3\xa9\n"
)

EXPECTED = [
    "hello red           ",
    "second line         ",
    "utf8: é             ",
    "                    ",
    "                    ",
]


@pytest.mark.parametrize("chunk", [1, 3, 7, len(CORPUS)])
def test_chunked_corpus_matches_expected(chunk):
    screen = TerminalState(20, 5, 10)
    for start in range(0, len(CORPUS), chunk):
        screen.feed(CORPUS[start : start + chunk])
    assert screen.visible_display() == EXPECTED
    assert screen.title == "parity-title"
    viewport = screen.styled_viewport()
    assert viewport[0][6][0] == "r"
    assert viewport[0][6][1] == "red"
    assert viewport[2][6][0] == "é"
