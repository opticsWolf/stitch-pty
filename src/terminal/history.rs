/// HistoryScreen — terminal screen with scrollback buffer.
///
/// Extends ``Screen`` with a fixed-capacity scrollback history.  When the
/// visible area scrolls, lines are pushed into (or popped from) the history
/// buffer.

use super::screen::{Char, Margins, Screen};

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
        fn pack(c: &Char) -> (String, String, String, u8) {
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
        self.history.iter()
            .chain(self.inner.buffer.iter())
            .map(|row| row.iter().map(pack).collect())
            .collect()
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
    pub fn resize(&mut self, lines: usize, columns: usize) { self.inner.resize(lines, columns); }
    pub fn set_title(&mut self, title: &str) { self.inner.set_title(title); }
    pub fn set_icon_name(&mut self, name: &str) { self.inner.set_icon_name(name); }
    pub fn report_device_attributes(&mut self) { self.inner.report_device_attributes(); }
    pub fn report_device_status(&mut self, param: usize) { self.inner.report_device_status(param); }

    // ── Deref-like access ──────────────────────────────────────

    pub fn cursor(&self) -> &super::screen::Cursor { &self.inner.cursor }
    pub fn cursor_mut(&mut self) -> &mut super::screen::Cursor { &mut self.inner.cursor }
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
        let mut performer = Performer::new(&mut self.inner);
        let mut parser = AnsiParser::new();
        parser.advance(&mut performer, data);
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
