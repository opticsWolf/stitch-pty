/// HistoryScreen — terminal screen with scrollback buffer.
///
/// Extends ``Screen`` with a fixed-capacity scrollback history.  When the
/// visible area scrolls, lines are pushed into (or popped from) the history
/// buffer.

use super::screen::{Char, Margins, Screen};

/// Pack a cell into the Python tuple shape (text, fg, bg, attrs_bitmask).
fn pack_cell(c: &Char) -> (String, String, String, u8) {
    let mut a = 0u8;
    if c.bold          { a |= 1 << 0; }
    if c.dim           { a |= 1 << 1; }
    if c.italics       { a |= 1 << 2; }
    if c.underscore    { a |= 1 << 3; }
    if c.blink         { a |= 1 << 4; }
    if c.reverse       { a |= 1 << 5; }
    if c.hidden        { a |= 1 << 6; }
    if c.strikethrough { a |= 1 << 7; }
    (c.data.clone(), c.fg.clone(), c.bg.clone(), a)
}

pub struct HistoryScreen {
    inner: Screen,
    history: Vec<Vec<Char>>,
    scrollback_lines: usize,
}

impl HistoryScreen {
    pub fn new(columns: usize, lines: usize, scrollback_lines: usize) -> Self {
        Self {
            inner: Screen::new(columns, lines),
            history: Vec::new(),
            scrollback_lines,
        }
    }

    // ── Accessors ───────────────────────────────────────────────

    pub fn columns(&self) -> usize { self.inner.columns }
    pub fn lines(&self) -> usize { self.inner.lines }
    pub fn history_size(&self) -> usize { self.history.len() }
    pub fn scrollback_lines(&self) -> usize { self.scrollback_lines }

    pub fn set_scrollback_lines(&mut self, lines: usize) {
        self.scrollback_lines = lines;
        self.trim_history();
    }

    pub fn display(&self) -> Vec<String> {
        let history_display: Vec<String> = self.history.iter()
            .map(|line| line.iter().map(|c| c.data.as_str()).collect::<String>())
            .collect();
        let visible_display = self.inner.display();
        [history_display, visible_display].concat()
    }

    pub fn visible_display(&self) -> Vec<String> {
        self.inner.display()
    }

    pub fn history_display(&self) -> Vec<String> {
        self.history.iter()
            .map(|line| line.iter().map(|c| c.data.as_str()).collect::<String>())
            .collect()
    }

    /// Total lines: scrollback history + visible screen.
    pub fn total_lines(&self) -> usize {
        self.history.len() + self.inner.lines
    }

    /// Absolute cursor position: (x, history_len + on-screen_y).
    pub fn absolute_cursor(&self) -> (usize, usize) {
        (self.inner.cursor.x, self.history.len() + self.inner.cursor.y)
    }

    /// History + visible buffer as styled cells: (text, fg, bg, attrs_bitmask).
    pub fn styled_viewport(&self) -> Vec<Vec<(String, String, String, u8)>> {
        self.history.iter()
            .chain(self.inner.buffer.iter())
            .map(|row| row.iter().map(pack_cell).collect())
            .collect()
    }

    /// Styled cells for absolute rows `[start, start + count)`, clamped to the
    /// buffer. Rows below `history_len` come from scrollback; the rest from the
    /// visible screen. Lets callers serialize only the window currently on
    /// screen instead of the entire scrollback (O(window) instead of O(total)).
    pub fn styled_range(&self, start: usize, count: usize)
            -> Vec<Vec<(String, String, String, u8)>> {
        let total = self.total_lines();
        let end = start.saturating_add(count).min(total);
        let hlen = self.history.len();
        let mut out = Vec::with_capacity(end.saturating_sub(start));
        let mut i = start;
        while i < end {
            let row = if i < hlen { &self.history[i] } else { &self.inner.buffer[i - hlen] };
            out.push(row.iter().map(pack_cell).collect());
            i += 1;
        }
        out
    }

    pub fn buffer(&self) -> &Vec<Vec<Char>> { &self.inner.buffer }
    pub fn buffer_mut(&mut self) -> &mut Vec<Vec<Char>> { &mut self.inner.buffer }

    // ── History Management ──────────────────────────────────────

    fn trim_history(&mut self) {
        if self.scrollback_lines > 0 && self.history.len() > self.scrollback_lines {
            let excess = self.history.len() - self.scrollback_lines;
            self.history.drain(..excess);
        }
    }

    fn push_history(&mut self, line: Vec<Char>) {
        self.history.push(line);
        self.trim_history();
    }

    fn pop_history(&mut self) -> Option<Vec<Char>> {
        self.history.pop()
    }

    fn push_from_bottom(&mut self) {
        if let Some(line) = self.inner.buffer.last().cloned() {
            self.push_history(line);
        }
    }

    fn pop_to_bottom(&mut self) {
        if let Some(line) = self.pop_history() {
            self.inner.buffer.push(line);
        }
    }

    pub fn clear_history(&mut self) {
        self.history.clear();
    }

    // ── Scroll Operations (with history) ────────────────────────

    pub fn scroll_up_with_history(&mut self, rows: usize) {
        let (top, bottom) = self.scroll_region();
        let rows = rows.min(bottom - top + 1);
        for _ in 0..rows {
            if top < self.inner.buffer.len() {
                let line = self.inner.buffer[top].clone();
                self.push_history(line);
            }
            for y in top..bottom {
                self.inner.buffer[y] = self.inner.buffer[y + 1].clone();
            }
            self.inner.buffer[bottom] = vec![self.inner.default_char.clone(); self.inner.columns];
            self.inner.dirty.insert(bottom);
        }
    }

    pub fn scroll_down_with_history(&mut self, rows: usize) {
        let (top, _bottom) = self.scroll_region();
        let rows = rows.min(self.inner.lines - top);
        for _ in 0..rows {
            for y in (top + 1)..=self.scroll_region().1 {
                self.inner.buffer[y] = self.inner.buffer[y - 1].clone();
            }
            if let Some(line) = self.pop_history() {
                self.inner.buffer[top] = line;
            } else {
                self.inner.buffer[top] = vec![self.inner.default_char.clone(); self.inner.columns];
            }
            self.inner.dirty.insert(top);
        }
    }

    fn scroll_region(&self) -> (usize, usize) {
        match self.inner.margins {
            Some(m) => (m.top, m.bottom),
            None => (0, self.inner.lines - 1),
        }
    }

    // ── Delegate Methods ────────────────────────────────────────

    pub fn draw(&mut self, data: &str) { self.inner.draw(data); }
    pub fn cursor_position(&mut self, row: usize, col: usize) { self.inner.cursor_position(row, col); }
    pub fn cursor_up(&mut self, rows: usize) { self.inner.cursor_up(rows); }
    pub fn cursor_down(&mut self, rows: usize) { self.inner.cursor_down(rows); }
    pub fn cursor_forward(&mut self, cols: usize) { self.inner.cursor_forward(cols); }
    pub fn cursor_back(&mut self, cols: usize) { self.inner.cursor_back(cols); }
    pub fn carriage_return(&mut self) { self.inner.carriage_return(); }
    pub fn linefeed(&mut self) { self.inner.linefeed(); }

    pub fn index(&mut self) {
        let (_, bottom) = self.scroll_region();
        if self.inner.cursor.y < bottom {
            self.inner.cursor.y += 1;
            self.inner.cursor.x = 0;
        } else {
            self.scroll_up_with_history(1);
        }
    }

    pub fn reverse_index(&mut self) {
        let (top, _) = self.scroll_region();
        if self.inner.cursor.y > top {
            self.inner.cursor.y -= 1;
            self.inner.cursor.x = 0;
        } else {
            self.scroll_down_with_history(1);
        }
    }

    pub fn backspace(&mut self) { self.inner.backspace(); }
    pub fn tab(&mut self) { self.inner.tab(); }
    pub fn set_tab_stop(&mut self) { self.inner.set_tab_stop(); }
    pub fn clear_tab_stop(&mut self, mode: u16) { self.inner.clear_tab_stop(mode); }
    pub fn save_cursor(&mut self) { self.inner.save_cursor(); }
    pub fn restore_cursor(&mut self) { self.inner.restore_cursor(); }
    pub fn set_mode(&mut self, mode: u16, private: bool) { self.inner.set_mode(mode, private); }
    pub fn reset_mode(&mut self, mode: u16, private: bool) { self.inner.reset_mode(mode, private); }
    pub fn select_graphic_rendition(&mut self, params: &[u16]) { self.inner.select_graphic_rendition(params); }
    pub fn erase_in_line(&mut self, mode: usize) { self.inner.erase_in_line(mode); }
    pub fn erase_in_display(&mut self, mode: usize) { self.inner.erase_in_display(mode); }
    pub fn insert_lines(&mut self, count: usize) { self.inner.insert_lines(count); }
    pub fn delete_lines(&mut self, count: usize) { self.inner.delete_lines(count); }
    pub fn insert_characters(&mut self, count: usize) { self.inner.insert_characters(count); }
    pub fn delete_characters(&mut self, count: usize) { self.inner.delete_characters(count); }
    pub fn erase_characters(&mut self, count: usize) { self.inner.erase_characters(count); }
    pub fn set_margins(&mut self, top: Option<usize>, bottom: Option<usize>) { self.inner.set_margins(top, bottom); }
    pub fn clear_margins(&mut self) { self.inner.clear_margins(); }
    pub fn alignment_display(&mut self) { self.inner.alignment_display(); }
    pub fn reset(&mut self) { self.inner.reset(); self.clear_history(); }
    pub fn resize(&mut self, lines: usize, columns: usize) {
        // On the alternate screen there is no scrollback interaction: just
        // reshape both the live alt buffer and the parked primary buffer.
        if self.inner.alt_screen {
            self.inner.resize(lines, columns);
            return;
        }
        let old_lines = self.inner.lines;
        if lines < old_lines {
            // Shrinking: drop unused rows below the cursor first, then push the
            // remaining overflow off the top into the scrollback history.
            let excess = old_lines - lines;
            let rows_below = old_lines.saturating_sub(self.inner.cursor.y + 1);
            let from_bottom = excess.min(rows_below);
            let from_top = excess - from_bottom;
            for _ in 0..from_bottom {
                self.inner.buffer.pop();
            }
            for _ in 0..from_top {
                if !self.inner.buffer.is_empty() {
                    let line = self.inner.buffer.remove(0);
                    self.push_history(line);
                }
            }
            self.inner.cursor.y = self.inner.cursor.y.saturating_sub(from_top);
        }
        // Growing pads at the bottom (handled by inner.resize). We deliberately
        // do NOT pull lines back out of history: the shell redraws on SIGWINCH
        // and would clear those rows, and a subsequent shrink would push the
        // blanks back — repeated fast resizes would then drain real scrollback.
        // Reshape columns and pad/truncate to exactly `lines` rows.
        self.inner.resize(lines, columns);
    }
    pub fn set_title(&mut self, title: &str) { self.inner.set_title(title); }
    pub fn set_icon_name(&mut self, name: &str) { self.inner.set_icon_name(name); }
    pub fn report_device_attributes(&mut self) { self.inner.report_device_attributes(); }
    pub fn report_device_status(&mut self, param: usize) { self.inner.report_device_status(param); }

    // ── Deref-like access ──────────────────────────────────────

    pub fn cursor(&self) -> &super::screen::Cursor { &self.inner.cursor }
    pub fn cursor_mut(&mut self) -> &mut super::screen::Cursor { &mut self.inner.cursor }
    pub fn cursor_style(&self) -> super::screen::CursorStyle { self.inner.cursor_style }
    pub fn cursor_blink(&self) -> bool { self.inner.cursor_blink }
    pub fn alt_screen(&self) -> bool { self.inner.alt_screen }

    /// Cursor shape as a front-end-friendly name: "block", "underline", or "bar".
    pub fn cursor_shape(&self) -> &'static str {
        use super::screen::CursorStyle;
        match self.inner.cursor_style {
            CursorStyle::Underline => "underline",
            CursorStyle::Beam => "bar",
            CursorStyle::Block | CursorStyle::Default => "block",
        }
    }
    pub fn mode(&self) -> &super::modes::Modes { &self.inner.mode }
    pub fn mode_mut(&mut self) -> &mut super::modes::Modes { &mut self.inner.mode }
    pub fn margins(&self) -> Option<Margins> { self.inner.margins }
    pub fn dirty(&self) -> &std::collections::BTreeSet<usize> { &self.inner.dirty }
    pub fn default_char(&self) -> &Char { &self.inner.default_char }
    pub fn tabstops(&self) -> &std::collections::HashSet<usize> { &self.inner.tabstops }
    pub fn write_process_input(&mut self) -> &mut dyn FnMut(&str) { &mut self.inner.write_process_input }
    pub fn title(&self) -> &str { &self.inner.title }
    pub fn icon_name(&self) -> &str { &self.inner.icon_name }
    pub fn g0_charset(&self) -> super::charsets::CharsetRef { self.inner.g0_charset }

    /// Feed raw bytes into the terminal state machine.
    pub fn feed(&mut self, data: &[u8]) {
        use super::parser::Performer;
        use super::ansi_parser::Parser as AnsiParser;
        {
            let mut performer = Performer::new(&mut self.inner);
            let mut parser = AnsiParser::new();
            parser.advance(&mut performer, data);
        }
        // Capture any lines that scrolled off the top during this feed into the
        // scrollback history. (The parser drives the inner Screen directly, so
        // its scroll-up evicts into Screen::scrolled_off for us to collect.)
        if !self.inner.scrolled_off.is_empty() {
            let lines = std::mem::take(&mut self.inner.scrolled_off);
            for line in lines {
                self.push_history(line);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::screen::Char;
    use super::*;
    use crate::terminal::modes as mo;

    fn make_history(cols: usize, lines: usize, scrollback: usize) -> HistoryScreen {
        HistoryScreen::new(cols, lines, scrollback)
    }

    // ── Construction ────────────────────────────────────────────────────

    #[test]
    fn test_new() {
        let hs = make_history(80, 24, 1000);
        assert_eq!(hs.columns(), 80);
        assert_eq!(hs.lines(), 24);
        assert_eq!(hs.history_size(), 0);
        assert_eq!(hs.scrollback_lines(), 1000);
    }

    // ── Basic Scroll ────────────────────────────────────────────────────

    #[test]
    fn test_scroll_up_pushes_to_history() {
        let mut hs = make_history(5, 3, 100);
        hs.draw("line0");
        hs.inner.cursor.y = 0;

        // Manually set up buffer
        for (i, line) in ["line0", "line1", "line2"].iter().enumerate() {
            for (j, ch) in line.chars().enumerate() {
                hs.inner.buffer[i][j] = Char::new(ch.to_string());
            }
        }

        hs.scroll_up_with_history(1);
        assert_eq!(hs.history_size(), 1);
        assert_eq!(hs.history_display()[0], "line0");
    }

    #[test]
    fn test_scroll_down_pops_from_history() {
        let mut hs = make_history(5, 3, 100);
        for (i, line) in ["line0", "line1", "line2"].iter().enumerate() {
            for (j, ch) in line.chars().enumerate() {
                hs.inner.buffer[i][j] = Char::new(ch.to_string());
            }
        }

        // Push line0 into history
        hs.scroll_up_with_history(1);
        assert_eq!(hs.history_size(), 1);

        // Scroll back down
        hs.scroll_down_with_history(1);
        assert_eq!(hs.history_size(), 0);
        assert_eq!(hs.visible_display()[0], "line0");
    }

    // ── History Capacity ────────────────────────────────────────────────

    #[test]
    fn test_feed_scroll_populates_history() {
        // The real feed path (parser -> inner Screen) must still capture lines
        // that scroll off the top into the scrollback history.
        let mut hs = make_history(10, 3, 100);
        hs.feed(b"L0\r\nL1\r\nL2\r\nL3\r\nL4");
        assert!(hs.history_size() >= 2, "history={}", hs.history_size());
        assert_eq!(hs.history_display()[0].trim_end(), "L0");
        // styled_viewport must expose history + visible, matching total_lines.
        assert_eq!(hs.styled_viewport().len(), hs.total_lines());
    }

    #[test]
    fn test_resize_shrink_pushes_top_to_history() {
        let mut hs = make_history(10, 4, 100);
        hs.feed(b"A\r\nB\r\nC\r\nD");      // fills 4 rows, cursor on the last
        assert_eq!(hs.history_size(), 0);
        hs.resize(2, 10);                  // shrink: top rows go to scrollback
        assert!(hs.history_size() >= 2, "history={}", hs.history_size());
        assert_eq!(hs.history_display()[0].trim_end(), "A");
        assert_eq!(hs.visible_display().last().unwrap().trim_end(), "D");
    }

    #[test]
    fn test_resize_grow_preserves_history() {
        let mut hs = make_history(10, 2, 100);
        hs.feed(b"A\r\nB\r\nC\r\nD");      // scrolls A,B into history; C,D visible
        let before = hs.history_size();
        assert!(before >= 2, "history={}", before);
        hs.resize(4, 10);                  // grow: history is NOT drained
        assert_eq!(hs.history_size(), before);
        let vis = hs.visible_display();
        assert_eq!(vis.len(), 4);
        // Existing content stays at the top; new rows pad the bottom.
        assert_eq!(vis[0].trim_end(), "C");
        assert_eq!(vis[1].trim_end(), "D");
        assert_eq!(vis[2].trim_end(), "");
    }

    // ── Alternate screen ────────────────────────────────────────────────

    #[test]
    fn test_alt_screen_swap_and_restore() {
        let mut hs = make_history(10, 3, 100);
        hs.feed(b"primary");
        let saved_x = hs.cursor().x;
        hs.feed(b"\x1b[?1049h");
        assert!(hs.alt_screen());
        assert_eq!(hs.visible_display()[0].trim_end(), ""); // fresh alt buffer
        hs.feed(b"\x1b[H");                                 // home, then draw
        hs.feed(b"ALT");
        assert_eq!(hs.visible_display()[0].trim_end(), "ALT");
        hs.feed(b"\x1b[?1049l");
        assert!(!hs.alt_screen());
        assert_eq!(hs.visible_display()[0].trim_end(), "primary"); // primary back
        assert_eq!(hs.cursor().x, saved_x);                        // cursor restored
    }

    #[test]
    fn test_alt_screen_no_scrollback() {
        let mut hs = make_history(10, 3, 100);
        hs.feed(b"\x1b[?1049h");
        let before = hs.history_size();
        hs.feed(b"a\r\nb\r\nc\r\nd\r\ne\r\nf");   // scroll a lot on the alt screen
        assert_eq!(hs.history_size(), before, "alt screen must not feed scrollback");
        hs.feed(b"\x1b[?1049l");
    }

    #[test]
    fn test_alt_screen_resize_restores_primary() {
        let mut hs = make_history(10, 4, 100);
        hs.feed(b"P0\r\nP1\r\nP2\r\nP3");
        hs.feed(b"\x1b[?1049h");
        hs.resize(6, 10);                 // resize while on the alt screen
        assert!(hs.alt_screen());
        assert_eq!(hs.lines(), 6);
        hs.feed(b"\x1b[?1049l");
        assert_eq!(hs.lines(), 6);
        let vis = hs.visible_display();
        assert_eq!(vis[0].trim_end(), "P0");
        assert_eq!(vis[3].trim_end(), "P3");
    }

    #[test]
    fn test_styled_range_windows_buffer() {
        let mut hs = make_history(10, 3, 100);
        hs.feed(b"L0\r\nL1\r\nL2\r\nL3\r\nL4\r\nL5"); // 6 lines; 3 scroll into history
        let total = hs.total_lines();
        assert_eq!(total, hs.history_size() + 3);
        // Full range equals styled_viewport.
        let full = hs.styled_range(0, total);
        assert_eq!(full.len(), total);
        assert_eq!(full.len(), hs.styled_viewport().len());
        // A 2-row sub-window.
        assert_eq!(hs.styled_range(1, 2).len(), 2);
        // Start past the end clamps to empty.
        assert_eq!(hs.styled_range(total + 5, 4).len(), 0);
        // Count past the end clamps to what remains.
        assert_eq!(hs.styled_range(total - 1, 10).len(), 1);
    }

    #[test]
    fn test_styled_range_matches_viewport_rows() {
        let mut hs = make_history(8, 2, 50);
        hs.feed(b"\x1b[31mAB\x1b[0m\r\nCD\r\nEF"); // styled first row, then scroll
        let vp = hs.styled_viewport();
        let win = hs.styled_range(0, hs.total_lines());
        assert_eq!(win, vp); // identical content and styling
    }

    #[test]
    fn test_scrollback_limit() {
        let mut hs = make_history(5, 3, 2);
        for _ in 0..5 {
            hs.scroll_up_with_history(1);
        }
        assert_eq!(hs.history_size(), 2); // Limited to 2
    }

    #[test]
    fn test_unlimited_scrollback() {
        let mut hs = HistoryScreen::new(5, 3, 0); // 0 = unlimited
        for _ in 0..10 {
            hs.scroll_up_with_history(1);
        }
        assert_eq!(hs.history_size(), 10);
    }

    // ── Display ─────────────────────────────────────────────────────────

    #[test]
    fn test_display_includes_history() {
        let mut hs = make_history(5, 2, 100);
        hs.draw("top");
        hs.inner.buffer[1] = vec![Char::new("bottom".chars().next().unwrap()); 5];

        // Push top line to history
        hs.scroll_up_with_history(1);

        let full = hs.display();
        assert!(full.len() >= 2); // history + visible
    }

    #[test]
    fn test_visible_display() {
        let hs = make_history(5, 2, 100);
        assert_eq!(hs.visible_display().len(), 2);
    }

    // ── Index with History ──────────────────────────────────────────────

    #[test]
    fn test_index_scrolls_to_history() {
        let mut hs = make_history(5, 2, 100);
        hs.draw("line0");
        hs.inner.cursor.y = 1; // At bottom

        hs.index(); // Should scroll and push to history
        assert_eq!(hs.history_size(), 1);
    }

    #[test]
    fn test_reverse_index_pops_from_history() {
        let mut hs = make_history(5, 2, 100);
        // Set up history
        hs.history.push(vec![Char::new("X"); 5]);
        hs.inner.cursor.y = 0; // At top

        hs.reverse_index(); // Should pop from history
        assert_eq!(hs.history_size(), 0);
    }

    // ── Clear History ───────────────────────────────────────────────────

    #[test]
    fn test_clear_history() {
        let mut hs = make_history(5, 2, 100);
        hs.scroll_up_with_history(1);
        hs.scroll_up_with_history(1);
        assert_eq!(hs.history_size(), 2);
        hs.clear_history();
        assert_eq!(hs.history_size(), 0);
    }

    // ── Reset ───────────────────────────────────────────────────────────

    #[test]
    fn test_reset_clears_history() {
        let mut hs = make_history(5, 2, 100);
        hs.scroll_up_with_history(1);
        assert_eq!(hs.history_size(), 1);
        hs.reset();
        assert_eq!(hs.history_size(), 0);
    }

    // ── Resize ──────────────────────────────────────────────────────────

    #[test]
    fn test_resize() {
        let mut hs = make_history(5, 3, 100);
        hs.resize(4, 10);
        assert_eq!(hs.lines(), 4);
        assert_eq!(hs.columns(), 10);
    }

    // ── Delegate Methods ────────────────────────────────────────────────

    #[test]
    fn test_draw_delegation() {
        let mut hs = make_history(10, 1, 100);
        hs.draw("Hello");
        assert_eq!(hs.visible_display(), vec!["Hello     "]);
    }

    #[test]
    fn test_cursor_delegation() {
        let mut hs = make_history(10, 10, 100);
        hs.cursor_position(5, 5);
        assert_eq!(hs.cursor().y, 4);
        assert_eq!(hs.cursor().x, 4);
    }

    #[test]
    fn test_mode_delegation() {
        let mut hs = make_history(10, 10, 100);
        hs.set_mode(mo::DECAWM, true);
        assert!(hs.mode().has_private(mo::DECAWM));
    }

    #[test]
    fn test_sgr_delegation() {
        let mut hs = make_history(10, 10, 100);
        hs.select_graphic_rendition(&[1, 31]);
        assert!(hs.cursor().attrs.bold);
        assert_eq!(hs.cursor().attrs.fg, "red");
    }
}
