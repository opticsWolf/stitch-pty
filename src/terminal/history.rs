/// HistoryScreen — terminal screen with scrollback buffer.
///
/// Extends ``Screen`` with a fixed-capacity scrollback history.  When the
/// visible area scrolls, lines are pushed into (or popped from) the history
/// buffer.
use super::screen::{Char, Margins, Screen};

/// Pack a cell into the Python tuple shape (text, fg, bg, attrs_bitmask).
fn pack_cell(c: &Char) -> (String, String, String, u8) {
    let mut a = 0u8;
    if c.bold {
        a |= 1 << 0;
    }
    if c.dim {
        a |= 1 << 1;
    }
    if c.italics {
        a |= 1 << 2;
    }
    if c.underscore {
        a |= 1 << 3;
    }
    if c.blink {
        a |= 1 << 4;
    }
    if c.reverse {
        a |= 1 << 5;
    }
    if c.hidden {
        a |= 1 << 6;
    }
    if c.strikethrough {
        a |= 1 << 7;
    }
    (c.data.clone(), c.fg.clone(), c.bg.clone(), a)
}

/// Drop trailing default-blank cells (uncolored, unstyled spaces) from a
/// flattened logical line: reflow- or erase-introduced padding, not content.
/// BCE-colored tails are *not* blank and survive.
fn trim_trailing_blanks(cells: &mut Vec<Char>) {
    while cells.last().is_some_and(Char::is_blank) {
        cells.pop();
    }
}

pub struct HistoryScreen {
    inner: Screen,
    history: Vec<Vec<Char>>,
    /// Wrap flags parallel to `history` — travel with their line on
    /// push/pop/trim/resize so reflow (v0.9.0) sees one logical sequence.
    history_wrapped: Vec<bool>,
    scrollback_lines: usize,
    /// Lines that entered scrollback since the last `take_events()` drain.
    scrollback_grew: u64,
}

impl HistoryScreen {
    pub fn new(columns: usize, lines: usize, scrollback_lines: usize) -> Self {
        Self {
            inner: Screen::new(columns, lines),
            history: Vec::new(),
            history_wrapped: Vec::new(),
            scrollback_lines,
            scrollback_grew: 0,
        }
    }

    // ── Accessors ───────────────────────────────────────────────

    pub fn columns(&self) -> usize {
        self.inner.columns
    }
    pub fn lines(&self) -> usize {
        self.inner.lines
    }
    pub fn history_size(&self) -> usize {
        self.history.len()
    }
    pub fn scrollback_lines(&self) -> usize {
        self.scrollback_lines
    }

    pub fn set_scrollback_lines(&mut self, lines: usize) {
        self.scrollback_lines = lines;
        self.trim_history();
    }

    pub fn display(&self) -> Vec<String> {
        let history_display: Vec<String> = self
            .history
            .iter()
            .map(|line| line.iter().map(|c| c.data.as_str()).collect::<String>())
            .collect();
        let visible_display = self.inner.display();
        [history_display, visible_display].concat()
    }

    pub fn visible_display(&self) -> Vec<String> {
        self.inner.display()
    }

    pub fn history_display(&self) -> Vec<String> {
        self.history
            .iter()
            .map(|line| line.iter().map(|c| c.data.as_str()).collect::<String>())
            .collect()
    }

    /// Total lines: scrollback history + visible screen.
    pub fn total_lines(&self) -> usize {
        self.history.len() + self.inner.lines
    }

    /// Absolute cursor position: (x, history_len + on-screen_y).
    pub fn absolute_cursor(&self) -> (usize, usize) {
        (
            self.inner.cursor.x,
            self.history.len() + self.inner.cursor.y,
        )
    }

    /// History + visible buffer as styled cells: (text, fg, bg, attrs_bitmask).
    pub fn styled_viewport(&self) -> Vec<Vec<(String, String, String, u8)>> {
        self.history
            .iter()
            .chain(self.inner.buffer.iter())
            .map(|row| row.iter().map(pack_cell).collect())
            .collect()
    }

    /// Styled cells for absolute rows `[start, start + count)`, clamped to the
    /// buffer. Rows below `history_len` come from scrollback; the rest from the
    /// visible screen. Lets callers serialize only the window currently on
    /// screen instead of the entire scrollback (O(window) instead of O(total)).
    pub fn styled_range(
        &self,
        start: usize,
        count: usize,
    ) -> Vec<Vec<(String, String, String, u8)>> {
        let total = self.total_lines();
        let end = start.saturating_add(count).min(total);
        let hlen = self.history.len();
        let mut out = Vec::with_capacity(end.saturating_sub(start));
        let mut i = start;
        while i < end {
            let row = if i < hlen {
                &self.history[i]
            } else {
                &self.inner.buffer[i - hlen]
            };
            out.push(row.iter().map(pack_cell).collect());
            i += 1;
        }
        out
    }

    pub fn buffer(&self) -> &Vec<Vec<Char>> {
        &self.inner.buffer
    }
    pub fn buffer_mut(&mut self) -> &mut Vec<Vec<Char>> {
        &mut self.inner.buffer
    }

    // ── History Management ──────────────────────────────────────

    fn trim_history(&mut self) {
        if self.scrollback_lines > 0 && self.history.len() > self.scrollback_lines {
            let excess = self.history.len() - self.scrollback_lines;
            self.history.drain(..excess);
            self.history_wrapped.drain(..excess);
        }
    }

    fn push_history(&mut self, line: Vec<Char>, wrapped: bool) {
        self.history.push(line);
        self.history_wrapped.push(wrapped);
        self.scrollback_grew += 1;
        self.trim_history();
    }

    fn pop_history(&mut self) -> Option<(Vec<Char>, bool)> {
        let line = self.history.pop()?;
        let wrapped = self.history_wrapped.pop().unwrap_or(false);
        Some((line, wrapped))
    }

    pub fn clear_history(&mut self) {
        self.history.clear();
        self.history_wrapped.clear();
    }

    // ── Scroll Operations (with history) ────────────────────────

    pub fn scroll_up_with_history(&mut self, rows: usize) {
        let (top, bottom) = self.scroll_region();
        let rows = rows.min(bottom - top + 1);
        for _ in 0..rows {
            if top < self.inner.buffer.len() {
                let line = self.inner.buffer[top].clone();
                let wrapped = self.inner.wrapped.get(top).copied().unwrap_or(false);
                self.push_history(line, wrapped);
            }
            for y in top..bottom {
                self.inner.buffer[y] = self.inner.buffer[y + 1].clone();
                self.inner.wrapped[y] = self.inner.wrapped[y + 1];
            }
            self.inner.buffer[bottom] = vec![self.inner.default_char.clone(); self.inner.columns];
            self.inner.wrapped[bottom] = false;
            self.inner.dirty.insert(bottom);
        }
    }

    pub fn scroll_down_with_history(&mut self, rows: usize) {
        let (top, _bottom) = self.scroll_region();
        let rows = rows.min(self.inner.lines - top);
        for _ in 0..rows {
            for y in (top + 1)..=self.scroll_region().1 {
                self.inner.buffer[y] = self.inner.buffer[y - 1].clone();
                self.inner.wrapped[y] = self.inner.wrapped[y - 1];
            }
            if let Some((line, wrapped)) = self.pop_history() {
                // History rows always match the buffer width (alt-path
                // resizes conform them), so the flag travels as-is.
                self.inner.buffer[top] = line;
                self.inner.wrapped[top] = wrapped;
            } else {
                self.inner.buffer[top] = vec![self.inner.default_char.clone(); self.inner.columns];
                self.inner.wrapped[top] = false;
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

    pub fn draw(&mut self, data: &str) {
        self.inner.draw(data);
    }
    pub fn cursor_position(&mut self, row: usize, col: usize) {
        self.inner.cursor_position(row, col);
    }
    pub fn cursor_up(&mut self, rows: usize) {
        self.inner.cursor_up(rows);
    }
    pub fn cursor_down(&mut self, rows: usize) {
        self.inner.cursor_down(rows);
    }
    pub fn cursor_forward(&mut self, cols: usize) {
        self.inner.cursor_forward(cols);
    }
    pub fn cursor_back(&mut self, cols: usize) {
        self.inner.cursor_back(cols);
    }
    pub fn carriage_return(&mut self) {
        self.inner.carriage_return();
    }
    pub fn linefeed(&mut self) {
        self.inner.linefeed();
    }

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

    pub fn backspace(&mut self) {
        self.inner.backspace();
    }
    pub fn tab(&mut self) {
        self.inner.tab();
    }
    pub fn set_tab_stop(&mut self) {
        self.inner.set_tab_stop();
    }
    pub fn clear_tab_stop(&mut self, mode: u16) {
        self.inner.clear_tab_stop(mode);
    }
    pub fn save_cursor(&mut self) {
        self.inner.save_cursor();
    }
    pub fn restore_cursor(&mut self) {
        self.inner.restore_cursor();
    }
    pub fn set_mode(&mut self, mode: u16, private: bool) {
        self.inner.set_mode(mode, private);
    }
    pub fn reset_mode(&mut self, mode: u16, private: bool) {
        self.inner.reset_mode(mode, private);
    }
    pub fn select_graphic_rendition(&mut self, params: &[u16]) {
        self.inner.select_graphic_rendition(params);
    }
    pub fn erase_in_line(&mut self, mode: usize) {
        self.inner.erase_in_line(mode);
    }
    pub fn erase_in_display(&mut self, mode: usize) {
        self.inner.erase_in_display(mode);
    }
    pub fn insert_lines(&mut self, count: usize) {
        self.inner.insert_lines(count);
    }
    pub fn delete_lines(&mut self, count: usize) {
        self.inner.delete_lines(count);
    }
    pub fn insert_characters(&mut self, count: usize) {
        self.inner.insert_characters(count);
    }
    pub fn delete_characters(&mut self, count: usize) {
        self.inner.delete_characters(count);
    }
    pub fn erase_characters(&mut self, count: usize) {
        self.inner.erase_characters(count);
    }
    pub fn set_margins(&mut self, top: Option<usize>, bottom: Option<usize>) {
        self.inner.set_margins(top, bottom);
    }
    pub fn clear_margins(&mut self) {
        self.inner.clear_margins();
    }
    pub fn alignment_display(&mut self) {
        self.inner.alignment_display();
    }
    pub fn reset(&mut self) {
        self.inner.reset();
        self.clear_history();
    }
    pub fn resize(&mut self, lines: usize, columns: usize) {
        // Clamp at entry, before the arithmetic below: a 0 would make
        // `old_lines - lines` evict the whole screen into scrollback and
        // desync from inner.resize's own clamp (v0.7.5 review finding).
        let lines = lines.max(1);
        let columns = columns.max(1);
        // On the alternate screen there is no scrollback interaction: just
        // reshape both the live alt buffer and the parked primary buffer.
        // History is conformed (never reflowed) so every parked row stays
        // rectangular at the new width — otherwise a later
        // scroll_down_with_history could pop a stale-width line into the
        // live buffer.
        if self.inner.alt_screen {
            let old_cols = self.inner.columns;
            self.inner.resize(lines, columns);
            if columns != old_cols {
                for line in self.history.iter_mut() {
                    line.truncate(columns);
                    while line.len() < columns {
                        line.push(self.inner.default_char.clone());
                    }
                }
            }
            return;
        }
        // A column change reflows; a row-count-only change keeps the
        // established shrink-push / grow-nop path below (deliberately never
        // pulling history back — see the grow comment). The two never mix:
        // reflow repacks to exactly `lines` visible rows itself.
        if columns != self.inner.columns {
            self.reflow(lines, columns);
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
                self.inner.wrapped.pop();
            }
            for _ in 0..from_top {
                if !self.inner.buffer.is_empty() {
                    let line = self.inner.buffer.remove(0);
                    let wrapped = if self.inner.wrapped.is_empty() {
                        false
                    } else {
                        self.inner.wrapped.remove(0)
                    };
                    self.push_history(line, wrapped);
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

    /// Reflow primary-screen content to a new column width (v0.9.0).
    ///
    /// `history ++ buffer` is treated as one logical sequence. Logical lines
    /// (rows joined via `wrapped` flags) are flattened — dropping each wide
    /// glyph's continuation cell and trimming trailing default-blank cells
    /// (reflow- or erase-introduced padding; BCE-colored tails are *not*
    /// blank and survive) — then re-split at the new width. A wide glyph
    /// that would straddle the margin gets a gap pad and starts the next
    /// row; it is never split. Every emitted row has exactly `columns`
    /// cells, so row indexing downstream cannot panic.
    ///
    /// Repack is bottom-anchored: the last `lines` rows stay visible, the
    /// rest flows to history (cap-trimmed, oldest first, row-granular). This
    /// viewport has no scroll offset, so bottom-anchored *is* the scroll
    /// anchor. The cursor keeps a logical anchor — its logical line plus
    /// flat cell offset — clamped into view if its content was trimmed.
    /// Primary screen only; the alt-screen path returns before this runs.
    /// DECSTBM × reflow is unspecified (margins are cleared, matching the
    /// row-count path). Damage funnels through `mark_all_dirty`, so Python
    /// consumers just eat a full redraw; `scrollback_grew` counts the net
    /// history growth only.
    fn reflow(&mut self, lines: usize, columns: usize) {
        let old_cols = self.inner.columns.max(1);
        // Drain any pending scrolled-off lines first so nothing is lost.
        if !self.inner.scrolled_off.is_empty() {
            let pending = std::mem::take(&mut self.inner.scrolled_off);
            let pending_w = std::mem::take(&mut self.inner.scrolled_off_wrapped);
            for (i, line) in pending.into_iter().enumerate() {
                let wrapped = pending_w.get(i).copied().unwrap_or(false);
                self.push_history(line, wrapped);
            }
        }
        let total_rows = self.history.len() + self.inner.buffer.len();
        // ── 1. Flatten into logical lines (owned: borrows end here). ──
        struct LogicalLine {
            cells: Vec<Char>,
            start_row: usize,
        }
        let mut logical: Vec<LogicalLine> = Vec::new();
        let mut cur: Vec<Char> = Vec::new();
        let mut cur_start = 0usize;
        let mut skip_next_blank = false;
        for r in 0..total_rows {
            let (row, wrapped) = if r < self.history.len() {
                (
                    &self.history[r],
                    self.history_wrapped.get(r).copied().unwrap_or(false),
                )
            } else {
                let i = r - self.history.len();
                (
                    &self.inner.buffer[i],
                    self.inner.wrapped.get(i).copied().unwrap_or(false),
                )
            };
            if r > 0 && !wrapped {
                trim_trailing_blanks(&mut cur);
                logical.push(LogicalLine {
                    cells: std::mem::take(&mut cur),
                    start_row: cur_start,
                });
                cur_start = r;
            }
            for cell in row.iter() {
                // A wide glyph's continuation cell is a blank directly
                // behind it — skip exactly one (a real following space is
                // kept: the one-shot is consumed by the continuation).
                if skip_next_blank && cell.data == " " {
                    skip_next_blank = false;
                    continue;
                }
                skip_next_blank = false;
                if cell.width() >= 2 {
                    skip_next_blank = true;
                }
                cur.push(cell.clone());
            }
        }
        trim_trailing_blanks(&mut cur);
        logical.push(LogicalLine {
            cells: cur,
            start_row: cur_start,
        });
        // ── 2. Cursor anchor: logical line + flat cell offset. ──
        let cur_row = self.history.len() + self.inner.cursor.y;
        let mut anchor_line = 0usize;
        for (i, line) in logical.iter().enumerate() {
            if line.start_row <= cur_row {
                anchor_line = i;
            } else {
                break;
            }
        }
        let flat = (cur_row.saturating_sub(logical[anchor_line].start_row))
            .saturating_mul(old_cols)
            .saturating_add(self.inner.cursor.x.min(old_cols.saturating_sub(1)));
        // ── 3. Re-split at the new width. ──
        // Per line: (first reflowed row, trimmed cell count).
        let mut line_spans: Vec<(usize, usize)> = Vec::with_capacity(logical.len());
        let mut rows: Vec<(Vec<Char>, bool)> = Vec::new();
        for line in &logical {
            let first = rows.len();
            let mut row: Vec<Char> = Vec::new();
            let flush = |row: &mut Vec<Char>, rows: &mut Vec<(Vec<Char>, bool)>| {
                while row.len() < columns {
                    row.push(Char::blank());
                }
                rows.push((std::mem::take(row), true));
            };
            for cell in line.cells.iter() {
                let need = if cell.width() >= 2 { 2 } else { 1 };
                if row.len() + need > columns {
                    // Pad a gap rather than stranding a wide glyph with one
                    // slot left — it starts the next row whole.
                    if need == 2 && row.len() + 1 == columns {
                        row.push(Char::blank());
                    }
                    flush(&mut row, &mut rows);
                }
                row.push(cell.clone());
                if cell.width() >= 2 && row.len() < columns {
                    // Continuation cell: the glyph's style, blank text —
                    // BCE-friendly, and the next flatten skips it again.
                    let mut cont = cell.clone();
                    cont.data = " ".to_string();
                    row.push(cont);
                }
                if row.len() == columns {
                    flush(&mut row, &mut rows);
                }
            }
            // End of logical line: emit the tail — except when the line
            // ended exactly on a row boundary (cur already flushed above).
            // An empty line still emits its single blank row, so hard
            // breaks (including blank lines) survive the round trip.
            if !row.is_empty() || rows.len() == first {
                flush(&mut row, &mut rows);
            }
            rows[first].1 = false;
            line_spans.push((first, line.cells.len()));
        }
        // ── 4. Overflow trim from the front (row-granular). ──
        let cap = self.scrollback_lines;
        let mut overflow = 0usize;
        if cap > 0 {
            let max_total = cap + lines;
            if rows.len() > max_total {
                overflow = rows.len() - max_total;
                rows.drain(..overflow);
                if let Some(first) = rows.first_mut() {
                    // The new first row lost its start — it is hard now.
                    first.1 = false;
                }
            }
        }
        // ── 5. Repack: last `lines` rows visible, rest to history. ──
        let total = rows.len();
        let vis_start = total.saturating_sub(lines);
        let old_hist = self.history.len();
        self.history = rows[..vis_start].iter().map(|(c, _)| c.clone()).collect();
        self.history_wrapped = rows[..vis_start].iter().map(|(_, w)| *w).collect();
        self.trim_history();
        let mut new_buffer: Vec<Vec<Char>> =
            rows[vis_start..].iter().map(|(c, _)| c.clone()).collect();
        let mut new_wrapped: Vec<bool> = rows[vis_start..].iter().map(|(_, w)| *w).collect();
        // Short screens pad at the bottom, like the row-count grow path.
        while new_buffer.len() < lines {
            new_buffer.push(vec![Char::blank(); columns]);
            new_wrapped.push(false);
        }
        self.inner.buffer = new_buffer;
        self.inner.wrapped = new_wrapped;
        self.inner.lines = lines;
        self.inner.columns = columns;
        self.inner.margins = None;
        self.inner.init_tabstops();
        self.inner.dirty.clear();
        self.inner.mark_all_dirty();
        if self.history.len() > old_hist {
            self.scrollback_grew += (self.history.len() - old_hist) as u64;
        }
        // ── 6. Restore the cursor from its logical anchor. ──
        let (a_first, a_len) = line_spans[anchor_line];
        let flat_in_line = flat.min(a_len.saturating_sub(1));
        let (new_row, new_col) = if a_first + flat_in_line / columns < overflow {
            (0usize, 0usize)
        } else {
            (
                a_first + flat_in_line / columns - overflow,
                flat_in_line % columns,
            )
        };
        self.inner.cursor.y = new_row
            .saturating_sub(vis_start)
            .min(lines.saturating_sub(1));
        self.inner.cursor.x = new_col.min(columns.saturating_sub(1));
    }
    pub fn set_title(&mut self, title: &str) {
        self.inner.set_title(title);
    }
    pub fn set_icon_name(&mut self, name: &str) {
        self.inner.set_icon_name(name);
    }
    pub fn report_device_attributes(&mut self) {
        self.inner.report_device_attributes();
    }
    pub fn report_device_status(&mut self, param: usize) {
        self.inner.report_device_status(param);
    }

    // ── Deref-like access ──────────────────────────────────────

    pub fn cursor(&self) -> &super::screen::Cursor {
        &self.inner.cursor
    }
    pub fn cursor_mut(&mut self) -> &mut super::screen::Cursor {
        &mut self.inner.cursor
    }
    pub fn cursor_style(&self) -> super::screen::CursorStyle {
        self.inner.cursor_style
    }
    pub fn cursor_blink(&self) -> bool {
        self.inner.cursor_blink
    }
    pub fn alt_screen(&self) -> bool {
        self.inner.alt_screen
    }

    /// Cursor shape as a front-end-friendly name: "block", "underline", or "bar".
    pub fn cursor_shape(&self) -> &'static str {
        use super::screen::CursorStyle;
        match self.inner.cursor_style {
            CursorStyle::Underline => "underline",
            CursorStyle::Beam => "bar",
            CursorStyle::Block | CursorStyle::Default => "block",
        }
    }
    pub fn mode(&self) -> &super::modes::Modes {
        &self.inner.mode
    }
    pub fn mode_mut(&mut self) -> &mut super::modes::Modes {
        &mut self.inner.mode
    }
    pub fn margins(&self) -> Option<Margins> {
        self.inner.margins
    }
    pub fn dirty(&self) -> &std::collections::BTreeSet<usize> {
        &self.inner.dirty
    }
    pub fn default_char(&self) -> &Char {
        &self.inner.default_char
    }
    pub fn tabstops(&self) -> &std::collections::HashSet<usize> {
        &self.inner.tabstops
    }
    pub fn write_process_input(&mut self) -> &mut dyn FnMut(&str) {
        &mut self.inner.write_process_input
    }
    pub fn title(&self) -> &str {
        &self.inner.title
    }
    pub fn icon_name(&self) -> &str {
        &self.inner.icon_name
    }
    pub fn set_cwd(&mut self, cwd: String) {
        self.inner.set_cwd(cwd);
    }
    pub fn cwd(&self) -> Option<&str> {
        self.inner.cwd()
    }

    /// Whether a BEL (0x07) arrived since the last call, then resets it.
    pub fn take_bell(&mut self) -> bool {
        self.inner.take_bell()
    }

    /// Drain the dirty-row set: sorted indices of rows modified since the
    /// last call, leaving it empty. Coalesces repeated writes to one row.
    pub fn take_dirty_rows(&mut self) -> Vec<usize> {
        self.inner.take_dirty_rows()
    }

    /// Drain the ordered event log. A trailing `ScrollbackGrew(n)` summary is
    /// appended when lines entered scrollback since the last drain — it is a
    /// per-drain summary, not a parser-ordered event.
    pub fn take_events(&mut self) -> Vec<super::events::TermEvent> {
        let mut events = self.inner.take_events();
        let grew = std::mem::replace(&mut self.scrollback_grew, 0);
        if grew > 0 {
            events.push(super::events::TermEvent::ScrollbackGrew(grew));
        }
        events
    }
    pub fn g0_charset(&self) -> super::charsets::CharsetRef {
        self.inner.g0_charset
    }

    /// Feed raw bytes into the terminal state machine.
    pub fn feed(&mut self, data: &[u8]) {
        use super::ansi_parser::Parser as AnsiParser;
        use super::parser::Performer;
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
            let flags = std::mem::take(&mut self.inner.scrolled_off_wrapped);
            for (i, line) in lines.into_iter().enumerate() {
                let wrapped = flags.get(i).copied().unwrap_or(false);
                self.push_history(line, wrapped);
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
        hs.feed(b"A\r\nB\r\nC\r\nD"); // fills 4 rows, cursor on the last
        assert_eq!(hs.history_size(), 0);
        hs.resize(2, 10); // shrink: top rows go to scrollback
        assert!(hs.history_size() >= 2, "history={}", hs.history_size());
        assert_eq!(hs.history_display()[0].trim_end(), "A");
        assert_eq!(hs.visible_display().last().unwrap().trim_end(), "D");
    }

    #[test]
    fn test_resize_grow_preserves_history() {
        let mut hs = make_history(10, 2, 100);
        hs.feed(b"A\r\nB\r\nC\r\nD"); // scrolls A,B into history; C,D visible
        let before = hs.history_size();
        assert!(before >= 2, "history={}", before);
        hs.resize(4, 10); // grow: history is NOT drained
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
        hs.feed(b"\x1b[H"); // home, then draw
        hs.feed(b"ALT");
        assert_eq!(hs.visible_display()[0].trim_end(), "ALT");
        hs.feed(b"\x1b[?1049l");
        assert!(!hs.alt_screen());
        assert_eq!(hs.visible_display()[0].trim_end(), "primary"); // primary back
        assert_eq!(hs.cursor().x, saved_x); // cursor restored
    }

    #[test]
    fn test_alt_screen_no_scrollback() {
        let mut hs = make_history(10, 3, 100);
        hs.feed(b"\x1b[?1049h");
        let before = hs.history_size();
        hs.feed(b"a\r\nb\r\nc\r\nd\r\ne\r\nf"); // scroll a lot on the alt screen
        assert_eq!(
            hs.history_size(),
            before,
            "alt screen must not feed scrollback"
        );
        hs.feed(b"\x1b[?1049l");
    }

    #[test]
    fn test_alt_screen_resize_restores_primary() {
        let mut hs = make_history(10, 4, 100);
        hs.feed(b"P0\r\nP1\r\nP2\r\nP3");
        hs.feed(b"\x1b[?1049h");
        hs.resize(6, 10); // resize while on the alt screen
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

    // ── Property tests: HistoryScreen invariants ──────────────────────

    use proptest::prelude::*;
    use proptest::test_runner::TestCaseResult;

    const PROP_FRAGMENTS: &[&[u8]] = &[
        b"\x1b[38;5;196m",
        b"\x1b[0m",
        b"\x1b[2J",
        b"\x1b[H",
        b"\x1b[1;1H",
        b"\x1b[?1049h",
        b"\x1b[?1049l",
        b"\x1b[?25l",
        b"\x1b[?25h",
        b"\x1b]0;title\x07",
        b"\x1b]2;t\x07",
        b"\x1b]7;file:///x\x07",
        b"\x1b]9;9;C:\\x\x1b\\",
        b"\x1bM",
        b"\x1b7",
        b"\x1b8",
        b"\x07",
        b"\x08",
        b"\r",
        b"\n",
        b"\t",
        "\u{20ac}".as_bytes(),
        "\u{4e2d}".as_bytes(),
        "\u{1f389}".as_bytes(),
        b"\xff",
        b"\xfe\x80",
        b"\xc3",
        b"hello",
        b" ",
    ];

    fn arb_mixed_stream() -> impl Strategy<Value = Vec<u8>> {
        prop::collection::vec(prop::sample::select(PROP_FRAGMENTS.to_vec()), 0..16)
            .prop_map(|parts| parts.concat())
    }

    fn feed_history_chunked(hs: &mut HistoryScreen, data: &[u8], cuts: &[u8]) {
        let mut points: Vec<usize> = cuts
            .iter()
            .map(|&b| b as usize % (data.len() + 1))
            .collect();
        points.sort_unstable();
        let mut start = 0;
        for &p in &points {
            hs.feed(&data[start..p]);
            start = p;
        }
        hs.feed(&data[start..]);
    }

    fn check_history_shape(
        hs: &HistoryScreen,
        cols: usize,
        lines: usize,
        cap: usize,
    ) -> TestCaseResult {
        prop_assert_eq!(hs.inner.buffer.len(), lines);
        for row in hs.inner.buffer.iter() {
            prop_assert_eq!(row.len(), cols);
        }
        // x == cols is the legal pending-wrap state (see parser.rs props).
        prop_assert!(hs.cursor().x <= cols);
        prop_assert!(hs.cursor().y < lines);
        // cap == 0 means unbounded; otherwise history never exceeds capacity.
        prop_assert!(
            cap == 0 || hs.history_size() <= cap,
            "history {} > cap {}",
            hs.history_size(),
            cap
        );
        Ok(())
    }

    proptest! {
        #[test]
        fn prop_history_mixed_shapes(
            data in arb_mixed_stream(),
            cuts in prop::collection::vec(any::<u8>(), 0..4),
            cols in 1..40usize,
            lines in 1..25usize,
            cap in 0..50usize,
        ) {
            let mut hs = make_history(cols, lines, cap);
            feed_history_chunked(&mut hs, &data, &cuts);
            check_history_shape(&hs, cols, lines, cap)?;
            let _ = hs.take_bell();
            prop_assert!(!hs.take_bell(), "take_bell fired twice in a row");
        }

        #[test]
        fn prop_alt_roundtrip_restores_primary(
            prefix in prop::collection::vec(any::<u8>(), 0..64),
        ) {
            // Any prefix state S is normalized: ENTER is a noop when already
            // in alt, and the trailing EXIT always returns to primary.
            let mut hs = make_history(20, 10, 100);
            hs.feed(&prefix);
            hs.feed(b"\x1b[?1049hX\x1b[?1049l");
            prop_assert!(!hs.inner.alt_screen, "stuck in alt screen");
            prop_assert_eq!(hs.inner.buffer.len(), 10, "parked row count");
        }

        #[test]
        fn prop_reflow_shape(
            data in arb_mixed_stream(),
            cols in 2..30usize,
            lines in 1..10usize,
            new_cols in 1..30usize,
            new_lines in 1..10usize,
            cap in 0..30usize,
        ) {
            // Any column resize must preserve the structural invariants:
            // rectangular rows at the new width, synced flag vectors,
            // in-bounds cursor, capped history. Primary screen only —
            // alt-screen resizes conform without reflowing.
            let mut hs = make_history(cols, lines, cap);
            hs.feed(&data);
            hs.resize(new_lines, new_cols);
            check_history_shape(&hs, new_cols, new_lines, cap)?;
            prop_assert_eq!(hs.history_wrapped.len(), hs.history_size());
            prop_assert_eq!(hs.inner.wrapped.len(), new_lines);
            for row in hs.history.iter() {
                prop_assert_eq!(row.len(), new_cols);
            }
        }
    }

    // ── Event pipeline ───────────────────────────────────────────────

    #[test]
    fn test_feed_orders_events() {
        use crate::terminal::events::TermEvent;
        let mut hs = make_history(80, 24, 100);
        hs.feed(b"\x1b]2;t\x07\x07\x1b[?1049h");
        assert_eq!(
            hs.take_events(),
            vec![
                TermEvent::TitleChanged("t".to_string()),
                TermEvent::Bell,
                TermEvent::AltScreen { entered: true },
            ]
        );
    }

    #[test]
    fn test_take_events_drains() {
        let mut hs = make_history(80, 24, 100);
        hs.feed(b"\x07");
        assert_eq!(hs.take_events().len(), 1);
        assert!(hs.take_events().is_empty());
    }

    #[test]
    fn test_bell_paths_agree_poll_first() {
        use crate::terminal::events::TermEvent;
        let mut hs = make_history(80, 24, 100);
        hs.feed(b"\x07");
        assert!(hs.take_events().contains(&TermEvent::Bell));
        assert!(!hs.take_bell()); // the drain consumed it
    }

    #[test]
    fn test_bell_paths_agree_take_first() {
        use crate::terminal::events::TermEvent;
        let mut hs = make_history(80, 24, 100);
        hs.feed(b"\x07\x07");
        assert!(hs.take_bell()); // coalesced pair → one true
        assert!(!hs.take_events().contains(&TermEvent::Bell)); // take consumed them
        assert!(!hs.take_bell());
    }

    #[test]
    fn test_bce_replays_conpty_colored_block() {
        // Exactly what ConPTY hands back for a colored block: it trims the
        // trailing whitespace and emits EL / ECH with the block's SGR still
        // active (captured on Windows 11). With BCE the whole block keeps
        // its background instead of only the visible characters.
        let mut hs = HistoryScreen::new(20, 2, 0);
        hs.feed(b"\x1b[48;2;45;27;61m hello\x1b[K\r\n");
        hs.feed(b"\x1b[m\x1b[48;2;45;27;61m hello\x1b[20X");
        let cells = hs.styled_range(0, 2);
        for (row, line) in cells.iter().enumerate() {
            assert!(
                line.iter().all(|cell| cell.2 == "2d1b3d"),
                "row {row} lost the block background: {line:?}"
            );
        }
    }
    // ── Reflow on column resize (v0.9.0) ──

    fn row_text(hs: &HistoryScreen, y: usize) -> String {
        hs.inner.buffer[y].iter().map(|c| c.data.as_str()).collect()
    }

    #[test]
    fn test_reflow_narrow_to_wide_rejoins() {
        let mut hs = make_history(10, 5, 50);
        hs.feed(&[b'a'; 25]); // rows of 10/10/5, one logical line
        assert!(hs.inner.wrapped[1] && hs.inner.wrapped[2]);
        hs.resize(5, 25);
        assert_eq!(row_text(&hs, 0), "a".repeat(25));
        assert!(
            hs.inner.wrapped.iter().all(|&w| !w),
            "rejoined line starts hard: {:?}",
            hs.inner.wrapped
        );
        assert_eq!(hs.history_size(), 0);
        // Cursor anchor: was flat offset 25 in a 25-cell line → last cell.
        assert_eq!((hs.cursor().x, hs.cursor().y), (24, 0));
        assert_history_wrapped_len(&hs);
    }

    #[test]
    fn test_reflow_wide_to_narrow_splits() {
        // lines=1: no pre-existing blank rows to bottom-anchor against.
        let mut hs = make_history(20, 1, 50);
        hs.feed(b"0123456789");
        hs.resize(1, 5);
        // Bottom-anchored: the head spills to history, the tail shows.
        assert_eq!(hs.history_display(), vec!["01234"]);
        assert_eq!(row_text(&hs, 0), "56789");
        // Visible row 0 continues the history line — flag says so.
        assert_eq!(hs.inner.wrapped, vec![true]);
        assert_history_wrapped_len(&hs);
    }

    #[test]
    fn test_reflow_preserves_hard_breaks() {
        // lines=3: exactly the three fed rows, no blanks in the mix.
        let mut hs = make_history(10, 3, 50);
        hs.feed(b"ab\r\ncdefgh\r\nij");
        hs.resize(4, 4);
        let got: Vec<String> = (0..4).map(|y| row_text(&hs, y)).collect();
        assert_eq!(got, vec!["ab  ", "cdef", "gh  ", "ij  "]);
        assert_eq!(hs.inner.wrapped, vec![false, false, true, false]);
        assert_eq!(hs.history_size(), 0);
    }

    #[test]
    fn test_reflow_round_trip_is_lossless() {
        // Narrowing pads rows with blanks; widening must trim that padding
        // back off instead of leaving phantom rows (the trim case).
        let mut hs = make_history(10, 8, 50);
        hs.feed(b"hello world, this wraps\r\nsecond line here ok\r\nshort\r\n");
        let before = hs.display();
        hs.resize(8, 5);
        hs.resize(8, 10);
        assert_eq!(hs.display(), before, "round trip changed content");
        assert_history_wrapped_len(&hs);
    }

    #[test]
    fn test_reflow_never_splits_wide_glyph() {
        let mut hs = make_history(4, 1, 50);
        hs.feed("ab\u{4e2d}".as_bytes()); // row: a b 中 cont
        hs.resize(2, 2);
        assert_eq!(row_text(&hs, 0), "ab");
        assert_eq!(hs.inner.buffer[1][0].data, "\u{4e2d}");
        assert_eq!(hs.inner.wrapped, vec![false, true]);
    }

    #[test]
    fn test_reflow_gap_pads_stranded_wide_glyph() {
        let mut hs = make_history(5, 1, 50);
        hs.feed("abc\u{4e2d}".as_bytes()); // row: a b c 中 cont
        hs.resize(4, 4);
        // 中 needs two slots with one left: gap pad, next row.
        assert_eq!(row_text(&hs, 0), "abc ");
        assert_eq!(hs.inner.buffer[1][0].data, "\u{4e2d}");
        assert_eq!(hs.inner.wrapped, vec![false, true, false, false]);
    }

    #[test]
    fn test_reflow_moves_history_and_conforms_widths() {
        let mut hs = make_history(10, 3, 100);
        for i in 0..6 {
            let line = format!("L{i}{}", "x".repeat(23)); // 25 cells
            hs.feed(line.as_bytes());
            hs.feed(b"\r\n");
        }
        let nonblank_before: String = hs
            .display()
            .join("")
            .chars()
            .filter(|&c| c != ' ')
            .collect();
        hs.resize(3, 7);
        for row in hs.history.iter().chain(hs.inner.buffer.iter()) {
            assert_eq!(row.len(), 7, "every row conforms to the new width");
        }
        let nonblank_after: String = hs
            .display()
            .join("")
            .chars()
            .filter(|&c| c != ' ')
            .collect();
        assert_eq!(nonblank_after, nonblank_before, "content lost in reflow");
        assert!(
            hs.cursor().y < 3,
            "cursor stays in bounds: {:?}",
            hs.cursor()
        );
        assert_history_wrapped_len(&hs);
    }

    #[test]
    fn test_reflow_alt_screen_passthrough() {
        let mut hs = make_history(10, 4, 50);
        hs.feed(b"primary");
        hs.feed(b"\x1b[?1049h"); // enter alt
        assert!(hs.alt_screen());
        hs.resize(4, 20); // conform only, no reflow
        assert!(hs.alt_screen());
        assert!(hs.inner.wrapped.iter().all(|&w| !w));
        assert_eq!(hs.inner.buffer[0].len(), 20);
        hs.feed(b"\x1b[?1049l"); // exit: primary restored, conformed
        assert!(!hs.alt_screen());
        assert_eq!(hs.inner.buffer[0].len(), 20);
        assert_history_wrapped_len(&hs);
    }

    #[test]
    fn test_reflow_overflow_trims_oldest_and_clamps_cursor() {
        let mut hs = make_history(10, 3, 4); // tiny cap forces overflow
        for _ in 0..8 {
            hs.feed(&[b'a'; 10]);
            hs.feed(b"\r\n");
        }
        hs.resize(3, 5); // 8 lines × 2 rows = 16 rows, cap 4 + 3 visible
        assert!(hs.history_size() <= 4, "cap respected");
        for row in hs.history.iter().chain(hs.inner.buffer.iter()) {
            assert_eq!(row.len(), 5);
        }
        assert!(hs.cursor().y < 3 && hs.cursor().x < 5);
        assert_history_wrapped_len(&hs);
    }

    #[test]
    fn test_reflow_clears_margins() {
        let mut hs = make_history(10, 4, 50);
        hs.feed(b"abcdef");
        hs.set_margins(Some(1), Some(2));
        hs.resize(4, 5); // DECSTBM × reflow: unspecified, must not corrupt
        assert!(hs.inner.margins.is_none());
        assert_history_wrapped_len(&hs);
        for row in hs.history.iter().chain(hs.inner.buffer.iter()) {
            assert_eq!(row.len(), 5);
        }
    }

    #[test]
    fn test_reflow_alt_resize_conforms_parked_history() {
        // Proptest find: a feed ending in alt-screen followed by a column
        // resize left history at the stale width. The alt path conforms
        // parked history now (never reflows — primary-screen only).
        let mut hs = make_history(10, 4, 50);
        hs.feed(b"line1\r\nline2\r\nline3\r\nline4\r\nline5");
        assert!(hs.history_size() > 0);
        hs.feed(b"\x1b[?1049h"); // enter alt
        hs.resize(4, 6);
        for row in hs.history.iter() {
            assert_eq!(row.len(), 6, "parked history must conform");
        }
        hs.feed(b"\x1b[?1049l"); // exit: primary restored
        assert_history_wrapped_len(&hs);
        for row in hs.history.iter().chain(hs.inner.buffer.iter()) {
            assert_eq!(row.len(), 6);
        }
    }

    #[test]
    fn test_scrollback_grew_trails() {
        use crate::terminal::events::TermEvent;
        let mut hs = make_history(80, 3, 100);
        hs.feed(b"A\r\nB\r\nC\r\nD"); // A scrolls into history
        let events = hs.take_events();
        assert_eq!(events.last(), Some(&TermEvent::ScrollbackGrew(1)));
        assert!(hs.take_events().is_empty());
    }

    // ── Wrap flags travel with their line (v0.8.2) ──

    fn assert_history_wrapped_len(hs: &HistoryScreen) {
        assert_eq!(
            hs.history_wrapped.len(),
            hs.history.len(),
            "history flag/line desync"
        );
        assert_eq!(
            hs.inner.wrapped.len(),
            hs.inner.buffer.len(),
            "visible flag/row desync"
        );
    }

    #[test]
    fn test_wrap_flag_flows_into_scrollback_on_scroll() {
        let mut hs = make_history(4, 2, 10);
        hs.inner.mode.set_private(mo::DECAWM);
        // Fill both rows then wrap-scroll: row 0 (hard) exits to history
        // while the wrapped continuation stays visible.
        hs.feed(b"abcdefgh"); // rows: "abcd" "efgh", cursor parks
        hs.feed(b"i"); // wraps onto... row 1 full -> scroll territory
        assert_history_wrapped_len(&hs);
        // Force a scroll: cursor to bottom, feed a hard newline worth.
        hs.feed(b"\r\nX");
        assert_history_wrapped_len(&hs);
        // Flags are recorded on visible rows after wrap.
        let mut hs2 = make_history(4, 2, 10);
        hs2.inner.mode.set_private(mo::DECAWM);
        hs2.feed(b"abcde");
        assert!(
            hs2.inner.wrapped[1],
            "feed wrap must flag the continuation row"
        );
        assert_history_wrapped_len(&hs2);
    }

    #[test]
    fn test_wrap_flag_round_trips_through_history() {
        let mut hs = make_history(4, 2, 10);
        hs.inner.mode.set_private(mo::DECAWM);
        hs.feed(b"abcde"); // wrapped[1] = true
        assert!(hs.inner.wrapped[1]);
        hs.scroll_up_with_history(1); // row 0 -> history, row 1 -> row 0
        assert!(hs.inner.wrapped[0], "flag rotates with its row");
        assert_history_wrapped_len(&hs);
        hs.scroll_down_with_history(1); // history line back on top
        assert_history_wrapped_len(&hs);
    }

    #[test]
    fn test_wrap_flag_carried_on_row_shrink() {
        let mut hs = make_history(4, 3, 10);
        hs.inner.mode.set_private(mo::DECAWM);
        hs.feed(b"abcde"); // wrapped[1] = true
        hs.cursor_position(3, 1); // pin cursor to the bottom
        hs.resize(1, 4); // excess 2, all from the top
        assert_history_wrapped_len(&hs);
        assert_eq!(hs.history_size(), 2);
        // Row 0 was hard, row 1 was a continuation — flags travel intact.
        assert_eq!(hs.history_wrapped, vec![false, true]);
        assert_eq!(hs.inner.wrapped.len(), hs.inner.buffer.len());
    }
}
