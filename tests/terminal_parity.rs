//! Fixed-corpus parity check for the Rust terminal model.
//!
//! The Python suite (`tests/test_terminal_parity.py`) feeds the same bytes into
//! `TerminalState`, which delegates to `HistoryScreen`, and asserts the same
//! rows/styles/title. Keep both files in sync when changing the corpus.

use stitch_pty::HistoryScreen;

const CORPUS: &[u8] =
    b"hello \x1b[31mred\x1b[0m\nsecond line\x1b]2;parity-title\x07\nutf8: \xc3\xa9\n";

const EXPECTED: [&str; 5] = [
    "hello red           ",
    "second line         ",
    "utf8: é             ",
    "                    ",
    "                    ",
];

fn feed_chunked(screen: &mut HistoryScreen, chunk: usize) {
    for part in CORPUS.chunks(chunk) {
        screen.feed(part);
    }
}

#[test]
fn chunked_corpus_matches_expected() {
    for chunk in [1, 3, 7, CORPUS.len()] {
        let mut screen = HistoryScreen::new(20, 5, 10);
        feed_chunked(&mut screen, chunk);
        assert_eq!(screen.visible_display(), EXPECTED, "chunk size {chunk}");
        assert_eq!(screen.title(), "parity-title", "chunk size {chunk}");
        let viewport = screen.styled_viewport();
        assert_eq!(viewport[0][6].0, "r", "chunk size {chunk}");
        assert_eq!(viewport[0][6].1, "red", "chunk size {chunk}");
        assert_eq!(viewport[2][6].0, "é", "chunk size {chunk}");
    }
}
