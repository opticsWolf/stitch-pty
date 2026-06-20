/// Terminal screen buffer with cursor management, margins, and dirty tracking.

use std::collections::{BTreeSet, HashSet};
use std::fmt;
use unicode_width::UnicodeWidthChar;

use super::charsets::CharsetRef;
use super::control;
use super::graphics as g;
use super::modes::{self as mo, Modes};

// ── Character Cell ─────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct Char {
    pub data: String,
    pub fg: String,
    pub bg: String,
    pub bold: bool,
    pub dim: bool,
    pub italics: bool,
    pub underscore: bool,
    pub blink: bool,
    pub reverse: bool,
    pub hidden: bool,
    pub strikethrough: bool,
}

impl Char {
    pub fn new(data: impl Into<String>) -> Self {
        Self { data: data.into(), ..Self::default() }
    }
    pub fn blank() -> Self {
        Self { data: " ".to_string(), fg: "default".to_string(), bg: "default".to_string(), ..Self::default() }
    }
    pub fn is_blank(&self) -> bool {
        self.data == " " && !self.bold && !self.dim && !self.italics
            && !self.underscore && !self.blink && !self.reverse
            && !self.hidden && !self.strikethrough
            && self.fg == "default" && self.bg == "default"
    }
    pub fn width(&self) -> usize {
        self.data.chars().next().map_or(0, |c| c.width().unwrap_or(1))
    }
}

impl fmt::Display for Char {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.data)
    }
}

// ── Cursor ─────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Cursor {
    pub x: usize,
    pub y: usize,
    pub attrs: Char,
    pub hidden: bool,
    pub stack: Vec<(usize, usize, Char, Modes)>,
}

impl Default for Cursor {
    fn default() -> Self {
        Self { x: 0, y: 0, attrs: Char::blank(), hidden: false, stack: Vec::new() }
    }
}

// ── Margins ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Margins {
    pub top: usize,
    pub bottom: usize,
}

impl Margins {
    pub fn height(&self) -> usize {
        if self.bottom >= self.top { self.bottom - self.top + 1 } else { 0 }
    }
}

// ── Cursor Style ───────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CursorStyle {
    #[default] Default,
    Block,
    Underline,
    Beam,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum KeyboardApplyBehavior {
    #[default] Replace,
    Union,
    Difference,
}

// ── Screen ─────────────────────────────────────────────────────────

pub struct Screen {
    pub columns: usize,
    pub lines: usize,
    pub buffer: Vec<Vec<Char>>,
    pub cursor: Cursor,
    pub default_char: Char,
    pub mode: Modes,
    pub margins: Option<Margins>,
    pub tabstops: HashSet<usize>,
    pub g0_charset: CharsetRef,
    pub g1_charset: CharsetRef,
    pub charset: CharsetRef,
    pub charset_index: u8,
    pub dirty: BTreeSet<usize>,
    pub icon_name: String,
    pub title: String,
    pub write_process_input: Box<dyn FnMut(&str) + Send + Sync + 'static>,
    pub cursor_style: CursorStyle,
    pub cursor_blink: bool,
    pub keyboard_mode: u16,
    pub keyboard_mode_stack: Vec<u16>,
}

impl Screen {
    pub fn new(columns: usize, lines: usize) -> Self {
        let default_char = Char::blank();
        let buffer = vec![vec![default_char.clone(); columns]; lines];
        let mut tabstops = HashSet::new();
        for col in (8..columns).step_by(8) { tabstops.insert(col); }
        let mode = Modes::new();
        let mut screen = Self {
            columns, lines, buffer, cursor: Cursor::default(), default_char, mode,
            margins: None, tabstops, g0_charset: CharsetRef::Ascii, g1_charset: CharsetRef::Ascii,
            charset: CharsetRef::Ascii, charset_index: 0, dirty: BTreeSet::new(),
            icon_name: String::new(), title: String::new(),
            write_process_input: Box::new(|_: &str| {}) as Box<dyn FnMut(&str) + Send + Sync + 'static>, cursor_style: CursorStyle::Default,
            cursor_blink: true,
            keyboard_mode: 0, keyboard_mode_stack: Vec::new(),
        };
        screen.init_tabstops();
        screen
    }

    fn init_tabstops(&mut self) {
        self.tabstops.clear();
        for col in (8..self.columns).step_by(8) { self.tabstops.insert(col); }
    }

    pub fn display(&self) -> Vec<String> {
        self.buffer.iter().map(|line| line.iter().map(|c| c.data.as_str()).collect()).collect()
    }

    pub fn reset(&mut self) {
        self.mode = Modes::new(); self.margins = None; self.cursor = Cursor::default();
        self.g0_charset = CharsetRef::Ascii; self.g1_charset = CharsetRef::Ascii;
        self.charset = CharsetRef::Ascii; self.charset_index = 0;
        self.icon_name.clear(); self.title.clear(); self.cursor_style = CursorStyle::Default; self.cursor_blink = true;
        self.keyboard_mode = 0; self.keyboard_mode_stack.clear(); self.init_tabstops();
        self.dirty.clear();
        for line in &mut self.buffer { for ch in line { *ch = self.default_char.clone(); } }
    }

    pub fn resize(&mut self, lines: usize, columns: usize) {
        if self.lines == lines && self.columns == columns { return; }
        let old_buffer = self.buffer.drain(..).map(|line| {
            line.into_iter().take(columns).chain(std::iter::repeat(self.default_char.clone()))
                .take(columns).collect::<Vec<_>>()
        }).take(lines).collect::<Vec<_>>();
        let default_line = vec![self.default_char.clone(); columns];
        self.buffer = old_buffer.into_iter().chain(std::iter::repeat(default_line)).take(lines).collect();
        self.lines = lines; self.columns = columns; self.margins = None;
        self.dirty.clear(); self.init_tabstops();
        self.cursor.x = self.cursor.x.min(self.columns.saturating_sub(1));
        self.cursor.y = self.cursor.y.min(self.lines.saturating_sub(1));
    }

    // ── Mode Management ──────────────────────────────────────────
    pub fn set_mode(&mut self, mode: u16, private: bool) {
        if private { self.set_private_mode(mode); } else { self.set_public_mode(mode); }
    }
    pub fn reset_mode(&mut self, mode: u16, private: bool) {
        if private { self.reset_private_mode(mode); } else { self.reset_public_mode(mode); }
    }
    fn set_public_mode(&mut self, mode: u16) { self.mode.set_public(mode); }
    fn reset_public_mode(&mut self, mode: u16) { self.mode.clear_public(mode); }

    pub fn set_private_mode(&mut self, mode: u16) {
        match mode {
            mo::DECCOLM => { self.save_columns(); self.columns = 132; self.clear_buffer(); self.margins = None; self.cursor = Cursor::default(); self.mode.set_private(mode); self.init_tabstops(); }
            mo::DECOM => { self.mode.set_private(mode); self.cursor.x = 0; self.cursor.y = 0; }
            mo::DECSCNM => { self.mode.set_private(mode); self.default_char.reverse = true; self.apply_reverse_to_buffer(true); }
            mo::DECTCEM => { self.mode.set_private(mode); self.cursor.hidden = false; }
            mo::DECAWM => { self.mode.set_private(mode); }
            _ => { self.mode.set_private(mode); }
        }
    }

    pub fn reset_private_mode(&mut self, mode: u16) {
        match mode {
            mo::DECCOLM => { self.columns = 80; self.margins = None; self.cursor = Cursor::default(); self.mode.clear_private(mode); self.init_tabstops(); }
            mo::DECOM => { self.mode.clear_private(mode); }
            mo::DECSCNM => { self.mode.clear_private(mode); self.default_char.reverse = false; self.apply_reverse_to_buffer(false); }
            mo::DECTCEM => { self.mode.clear_private(mode); self.cursor.hidden = true; }
            mo::DECAWM => { self.mode.clear_private(mode); }
            _ => { self.mode.clear_private(mode); }
        }
    }

    fn save_columns(&mut self) {}
    fn clear_buffer(&mut self) {
        for line in &mut self.buffer { for ch in line { *ch = self.default_char.clone(); } }
        self.dirty.clear();
    }
    fn apply_reverse_to_buffer(&mut self, reverse: bool) {
        for line in &mut self.buffer { for ch in line { ch.reverse = reverse; } }
    }

    // ── Margins ──────────────────────────────────────────────────
    pub fn set_margins(&mut self, top: Option<usize>, bottom: Option<usize>) {
        let top = top.unwrap_or(1).saturating_sub(1);
        let bottom = bottom.unwrap_or(self.lines).saturating_sub(1);
        if top < bottom && bottom < self.lines { self.margins = Some(Margins { top, bottom }); }
    }
    pub fn clear_margins(&mut self) { self.margins = None; }
    fn scroll_region(&self) -> (usize, usize) {
        match self.margins { Some(m) => (m.top, m.bottom), None => (0, self.lines - 1), }
    }

    // ── Cursor Movement ──────────────────────────────────────────
    pub fn cursor_position(&mut self, row: usize, col: usize) {
        let (top, bottom) = self.scroll_region();
        let row = if row == 0 { 1 } else { row };
        let col = if col == 0 { 1 } else { col };
        if self.mode.has_private(mo::DECOM) {
            self.cursor.y = (row - 1 + top).min(bottom);
            self.cursor.x = (col - 1).min(self.columns - 1);
        } else {
            self.cursor.y = (row - 1).min(self.lines - 1);
            self.cursor.x = (col - 1).min(self.columns - 1);
        }
    }
    pub fn cursor_to_line(&mut self, row: usize) { self.cursor_position(row, self.cursor.x + 1); }
    pub fn cursor_to_column(&mut self, col: usize) { self.cursor_position(self.cursor.y + 1, col); }
    pub fn cursor_up(&mut self, rows: usize) {
        let (top, _) = self.scroll_region();
        self.cursor.y = self.cursor.y.saturating_sub(rows).max(top);
    }
    pub fn cursor_down(&mut self, rows: usize) {
        let (_, bottom) = self.scroll_region();
        self.cursor.y = (self.cursor.y + rows).min(bottom);
    }
    pub fn cursor_forward(&mut self, cols: usize) { self.cursor.x = (self.cursor.x + cols).min(self.columns - 1); }
    pub fn cursor_back(&mut self, cols: usize) { self.cursor.x = self.cursor.x.saturating_sub(cols); }
    pub fn carriage_return(&mut self) { self.cursor.x = 0; }
    pub fn linefeed(&mut self) {
        let (_, bottom) = self.scroll_region();
        if self.cursor.y < bottom { self.cursor.y += 1; } else { self.scroll_up(1); }
        if self.mode.has_public(mo::LNM) { self.cursor.x = 0; }
    }
    pub fn index(&mut self) {
        let (_, bottom) = self.scroll_region();
        if self.cursor.y < bottom { self.cursor.y += 1; self.cursor.x = 0; } else { self.scroll_up(1); }
    }
    pub fn reverse_index(&mut self) {
        let (top, _) = self.scroll_region();
        if self.cursor.y > top { self.cursor.y -= 1; self.cursor.x = 0; } else { self.scroll_down(1); }
    }
    pub fn backspace(&mut self) { self.cursor.x = self.cursor.x.saturating_sub(1); }
    pub fn tab(&mut self) {
        let mut next = self.cursor.x + 1;
        while next < self.columns { if self.tabstops.contains(&next) { self.cursor.x = next; return; } next += 1; }
        self.cursor.x = self.columns - 1;
    }
    pub fn set_tab_stop(&mut self) { self.tabstops.insert(self.cursor.x); }
    pub fn clear_tab_stop(&mut self, mode: u16) {
        match mode { 0 => { self.tabstops.remove(&self.cursor.x); } 3 => { self.tabstops.clear(); } _ => {} }
    }
    pub fn save_cursor(&mut self) {
        self.cursor.stack.push((self.cursor.x, self.cursor.y, self.cursor.attrs.clone(), self.mode.clone()));
    }
    pub fn restore_cursor(&mut self) {
        if let Some((x, y, attrs, mode)) = self.cursor.stack.pop() {
            let (top, bottom) = self.scroll_region();
            self.cursor.x = x.min(self.columns - 1);
            self.cursor.y = y.max(top).min(bottom);
            self.cursor.attrs = attrs; self.mode = mode;
        } else { self.cursor.x = 0; self.cursor.y = 0; }
    }

    // ── Drawing ──────────────────────────────────────────────────
    pub fn draw(&mut self, data: &str) { for ch in data.chars() { self.draw_char(ch); } }

    fn draw_char(&mut self, ch: char) {
        let width = ch.width().unwrap_or(1);
        if width == 0 {
            if self.cursor.x > 0 && self.cursor.y < self.lines {
                let cell = &mut self.buffer[self.cursor.y][self.cursor.x - 1];
                cell.data.push(ch);
            }
            return;
        }
        if width >= 2 && self.cursor.x >= self.columns {
            self.cursor.x = 0;
            if self.cursor.y < self.lines - 1 { self.cursor.y += 1; }
        }
        if self.cursor.x >= self.columns {
            if self.mode.has_private(mo::DECAWM) {
                self.cursor.x = 0;
                if self.cursor.y < self.lines - 1 { self.cursor.y += 1; }
                else if self.cursor.y < self.scroll_region().1 { self.scroll_up(1); }
            } else if self.cursor.x == self.columns {
                self.cursor.x = 0;
                if self.cursor.y < self.lines - 1 { self.cursor.y += 1; }
            } else { self.cursor.x = 0; }
        }
        if self.mode.has_public(mo::IRM) && width == 1 { self.insert_characters(1); }
        if self.cursor.x < self.columns && self.cursor.y < self.lines {
            let cell = &mut self.buffer[self.cursor.y][self.cursor.x];
            *cell = self.cursor.attrs.clone(); cell.data = ch.to_string();
            if width >= 2 && self.cursor.x + 1 < self.columns {
                self.buffer[self.cursor.y][self.cursor.x + 1] = self.default_char.clone();
            }
            self.dirty.insert(self.cursor.y);
        }
        if width == 1 { self.cursor.x += 1; }
        else { if self.cursor.x >= self.columns { self.cursor.x = self.columns; } }
        if !self.mode.has_private(mo::DECAWM) && width == 1 {
            self.cursor.x = self.cursor.x.min(self.columns - 1);
        }
    }

    // ── Erase / Delete ──────────────────────────────────────────
    pub fn erase_characters(&mut self, count: usize) {
        let count = count.max(1); let x = self.cursor.x; let end = (x + count).min(self.columns);
        for i in x..end { self.buffer[self.cursor.y][i] = self.default_char.clone(); }
        self.dirty.insert(self.cursor.y);
    }
    pub fn delete_characters(&mut self, count: usize) {
        let count = count.max(1); let x = self.cursor.x; let end = (x + count).min(self.columns);
        for i in x..self.columns - (end - x) { self.buffer[self.cursor.y][i] = self.buffer[self.cursor.y][end + (i - x)].clone(); }
        for i in self.columns - (end - x)..self.columns { self.buffer[self.cursor.y][i] = self.default_char.clone(); }
        self.dirty.insert(self.cursor.y);
    }
    pub fn insert_characters(&mut self, count: usize) {
        let count = count.max(1); let x = self.cursor.x;
        for i in (x..self.columns.saturating_sub(count)).rev() {
            self.buffer[self.cursor.y][i + count] = self.buffer[self.cursor.y][i].clone();
        }
        for i in x..(x + count).min(self.columns) { self.buffer[self.cursor.y][i] = self.default_char.clone(); }
        self.dirty.insert(self.cursor.y);
    }
    pub fn erase_in_line(&mut self, mode: usize) {
        match mode {
            0 => { for i in self.cursor.x..self.columns { self.buffer[self.cursor.y][i] = self.default_char.clone(); } }
            1 => { for i in 0..=self.cursor.x { self.buffer[self.cursor.y][i] = self.default_char.clone(); } }
            _ => { for i in 0..self.columns { self.buffer[self.cursor.y][i] = self.default_char.clone(); } }
        }
        self.dirty.insert(self.cursor.y);
        if mode == 0 { self.cursor.x = self.cursor.x.min(self.columns - 1); }
    }
    pub fn erase_in_display(&mut self, mode: usize) {
        let (top, bottom) = self.scroll_region();
        match mode {
            0 => {
                for i in self.cursor.x..self.columns { self.buffer[self.cursor.y][i] = self.default_char.clone(); }
                self.dirty.insert(self.cursor.y);
                for y in (self.cursor.y + 1)..=bottom {
                    for x in 0..self.columns { self.buffer[y][x] = self.default_char.clone(); } self.dirty.insert(y);
                }
            }
            1 => {
                for y in top..self.cursor.y { for x in 0..self.columns { self.buffer[y][x] = self.default_char.clone(); } self.dirty.insert(y); }
                for i in 0..=self.cursor.x.min(self.columns - 1) { self.buffer[self.cursor.y][i] = self.default_char.clone(); }
                self.dirty.insert(self.cursor.y);
            }
            2 | 3 => {
                for y in top..=bottom { for x in 0..self.columns { self.buffer[y][x] = self.default_char.clone(); } self.dirty.insert(y); }
            }
            _ => {}
        }
    }
    pub fn insert_lines(&mut self, count: usize) {
        let count = count.max(1); let (top, bottom) = self.scroll_region();
        if self.cursor.y < top || self.cursor.y > bottom { return; }
        let n = count.min(bottom - self.cursor.y); let default_line = vec![self.default_char.clone(); self.columns];
        for _ in 0..n {
            for y in (self.cursor.y + 1..=bottom).rev() { self.buffer[y] = self.buffer[y - 1].clone(); }
            self.buffer[self.cursor.y] = default_line.clone(); self.dirty.insert(self.cursor.y);
        }
        self.cursor.x = 0;
    }
    pub fn delete_lines(&mut self, count: usize) {
        let count = count.max(1); let (top, bottom) = self.scroll_region();
        if self.cursor.y < top || self.cursor.y > bottom { return; }
        let n = count.min(bottom - self.cursor.y); let default_line = vec![self.default_char.clone(); self.columns];
        for _ in 0..n {
            for y in self.cursor.y..bottom { self.buffer[y] = std::mem::replace(&mut self.buffer[y + 1], default_line.clone()); }
            self.buffer[bottom] = default_line.clone(); self.dirty.insert(self.cursor.y);
        }
        self.cursor.x = 0;
    }
    pub fn scroll_up(&mut self, rows: usize) {
        let (top, bottom) = self.scroll_region(); let default_line = vec![self.default_char.clone(); self.columns];
        let rows = rows.min(bottom - top + 1);
        for _ in 0..rows {
            for y in top..bottom { self.buffer[y] = std::mem::replace(&mut self.buffer[y + 1], default_line.clone()); }
            self.buffer[bottom] = default_line.clone(); self.dirty.insert(bottom);
        }
    }
    pub fn scroll_down(&mut self, rows: usize) {
        let (top, bottom) = self.scroll_region(); let default_line = vec![self.default_char.clone(); self.columns];
        let rows = rows.min(bottom - top + 1);
        for _ in 0..rows {
            for y in (top + 1)..=bottom { self.buffer[y] = std::mem::replace(&mut self.buffer[y - 1], default_line.clone()); }
            self.buffer[top] = default_line.clone(); self.dirty.insert(top);
        }
    }

    // ── SGR ──────────────────────────────────────────────────────
    pub fn select_graphic_rendition(&mut self, params: &[u16]) {
        let mut i = 0;
        while i < params.len() {
            match params[i] {
                0 => { self.cursor.attrs = self.default_char.clone(); if self.mode.has_private(mo::DECSCNM) { self.cursor.attrs.reverse = true; } }
                1 => self.cursor.attrs.bold = true, 2 => self.cursor.attrs.dim = true,
                3 => self.cursor.attrs.italics = true, 4 => self.cursor.attrs.underscore = true,
                5 => self.cursor.attrs.blink = true, 7 => self.cursor.attrs.reverse = true,
                8 => self.cursor.attrs.hidden = true, 9 => self.cursor.attrs.strikethrough = true,
                22 => { self.cursor.attrs.bold = false; self.cursor.attrs.dim = false; }
                23 => self.cursor.attrs.italics = false, 24 => self.cursor.attrs.underscore = false,
                25 => self.cursor.attrs.blink = false, 27 => self.cursor.attrs.reverse = false,
                28 => self.cursor.attrs.hidden = false, 29 => self.cursor.attrs.strikethrough = false,
                39 => self.cursor.attrs.fg = "default".to_string(),
                49 => self.cursor.attrs.bg = "default".to_string(),
                30..=37 => { if let Some(name) = g::fg_ansi(params[i] as i32) { self.cursor.attrs.fg = name.to_string(); } }
                90..=97 => { if let Some(name) = g::fg_aixterm(params[i] as i32) { self.cursor.attrs.fg = name.to_string(); } }
                40..=47 => { if let Some(name) = g::bg_ansi(params[i] as i32) { self.cursor.attrs.bg = name.to_string(); } }
                100..=107 => { if let Some(name) = g::bg_aixterm(params[i] as i32) { self.cursor.attrs.bg = name.to_string(); } }
                g::FG_256 => {
                    if i + 1 < params.len() && params[i + 1] == 5 {
                        if i + 2 < params.len() && params[i + 2] < 256 {
                            self.cursor.attrs.fg = g::fg_bg_256()[params[i + 2] as usize].clone(); i += 2;
                        }
                    } else if i + 1 < params.len() && params[i + 1] == 2 {
                        if i + 4 < params.len() {
                            let r = params[i + 2] as u8; let g = params[i + 3] as u8; let b = params[i + 4] as u8;
                            self.cursor.attrs.fg = format!("{:02x}{:02x}{:02x}", r, g, b); i += 4;
                        }
                    }
                }
                g::BG_256 => {
                    if i + 1 < params.len() && params[i + 1] == 5 {
                        if i + 2 < params.len() && params[i + 2] < 256 {
                            self.cursor.attrs.bg = g::fg_bg_256()[params[i + 2] as usize].clone(); i += 2;
                        }
                    } else if i + 1 < params.len() && params[i + 1] == 2 {
                        if i + 4 < params.len() {
                            let r = params[i + 2] as u8; let g = params[i + 3] as u8; let b = params[i + 4] as u8;
                            self.cursor.attrs.bg = format!("{:02x}{:02x}{:02x}", r, g, b); i += 4;
                        }
                    }
                }
                _ => {}
            }
            i += 1;
        }
    }

    // ── Alignment Display ────────────────────────────────────────
    pub fn alignment_display(&mut self) {
        for y in 0..self.lines { for x in 0..self.columns { self.buffer[y][x] = self.default_char.clone(); self.buffer[y][x].data = "E".to_string(); } self.dirty.insert(y); }
        self.cursor.x = 0; self.cursor.y = 0;
    }

    // ── Device Status ────────────────────────────────────────────
    pub fn report_device_attributes(&mut self) {
        let response = format!("{}[?6c", control::ESC);
        (self.write_process_input)(&response);
    }
    pub fn report_device_status(&mut self, param: usize) {
        match param {
            5 => { let response = format!("{}[0n", control::ESC); (self.write_process_input)(&response); }
            6 => {
                let (row, col) = if self.mode.has_private(mo::DECOM) {
                    let (top, _) = self.scroll_region(); (self.cursor.y - top + 1, self.cursor.x + 1)
                } else { (self.cursor.y + 1, self.cursor.x + 1) };
                let response = format!("{}{}{};{}R", control::ESC, '[', row, col);
                (self.write_process_input)(&response);
            }
            _ => {}
        }
    }
    pub fn identify_terminal_primary(&mut self) {
        let response = format!("{}[?6c", control::ESC);
        (self.write_process_input)(&response);
    }
    pub fn set_cursor_style(&mut self, style_id: u16) {
        self.cursor_style = match style_id {
            0 => CursorStyle::Default, 1 | 2 => CursorStyle::Block,
            3 | 4 => CursorStyle::Underline, 5 | 6 => CursorStyle::Beam,
            _ => CursorStyle::Default,
        };
        // DECSCUSR: 0 and odd ids blink; even non-zero ids are steady.
        self.cursor_blink = matches!(style_id, 0 | 1 | 3 | 5);
    }
    pub fn set_keyboard_mode(&mut self, mode: u16, behavior: KeyboardApplyBehavior) {
        self.keyboard_mode = match behavior {
            KeyboardApplyBehavior::Replace => mode,
            KeyboardApplyBehavior::Union => self.keyboard_mode | mode,
            KeyboardApplyBehavior::Difference => self.keyboard_mode & !mode,
        };
    }
    pub fn push_keyboard_mode(&mut self, mode: u16) { self.keyboard_mode_stack.push(mode); }
    pub fn pop_keyboard_modes(&mut self, count: usize) {
        let to_pop = count.min(self.keyboard_mode_stack.len());
        for _ in 0..to_pop { if let Some(mode) = self.keyboard_mode_stack.pop() { self.keyboard_mode = mode; } }
    }
    pub fn report_keyboard_mode(&mut self) {
        let response = format!("{}[{};1;0 u", control::ESC, self.keyboard_mode);
        (self.write_process_input)(&response);
    }

    // ── Title / Icon ─────────────────────────────────────────────
    pub fn set_icon_name(&mut self, name: &str) { self.icon_name = name.to_string(); }
    pub fn set_title(&mut self, title: &str) { self.title = title.to_string(); }

    // ── Charset ──────────────────────────────────────────────────
    pub fn define_charset(&mut self, designator: &str, target: char) {
        let charset = CharsetRef::from_designation(designator.chars().next().unwrap_or('B'));
        if let Some(cs) = charset {
            match target { '(' => self.g0_charset = cs, ')' => self.g1_charset = cs, _ => {} }
            if target == '(' { self.charset = cs; }
        }
    }
    pub fn shift_in(&mut self) { self.charset = self.g0_charset; }
    pub fn shift_out(&mut self) { self.charset = self.g1_charset; }

    // ── CSI Dispatch ─────────────────────────────────────────────
    pub fn csi_dispatch(&mut self, params: &[u16], intermediates: &[u8], action: char) {
        match action {
            'A' => { let count = params.first().copied().unwrap_or(1) as usize; self.cursor_up(count); }
            'B' | 'e' => { let count = params.first().copied().unwrap_or(1) as usize; self.cursor_down(count); }
            'C' | 'a' => { let count = params.first().copied().unwrap_or(1) as usize; self.cursor_forward(count); }
            'D' => { let count = params.first().copied().unwrap_or(1) as usize; self.cursor_back(count); }
            'E' => { let count = params.first().copied().unwrap_or(1) as usize; self.cursor_down(count); self.cursor.x = 0; }
            'F' => { let count = params.first().copied().unwrap_or(1) as usize; self.cursor_up(count); self.cursor.x = 0; }
            'G' => { let col = params.first().copied().unwrap_or(1) as usize; self.cursor_to_column(col); }
            'H' | 'f' => { let row = params.first().copied().unwrap_or(1) as usize; let col = params.get(1).copied().unwrap_or(1) as usize; self.cursor_position(row, col); }
            'd' => { let row = params.first().copied().unwrap_or(1) as usize; self.cursor_to_line(row); }
            'J' => { let mode = params.first().copied().unwrap_or(0) as usize; self.erase_in_display(mode); }
            'K' => { let mode = params.first().copied().unwrap_or(0) as usize; self.erase_in_line(mode); }
            'X' => { let count = params.first().copied().unwrap_or(1) as usize; self.erase_characters(count); }
            'P' => { let count = params.first().copied().unwrap_or(1) as usize; self.delete_characters(count); }
            '@' => { let count = params.first().copied().unwrap_or(1) as usize; self.insert_characters(count); }
            'L' => { let count = params.first().copied().unwrap_or(1) as usize; self.insert_lines(count); }
            'M' => { let count = params.first().copied().unwrap_or(1) as usize; self.delete_lines(count); }
            'S' => { let count = params.first().copied().unwrap_or(1) as usize; self.scroll_up(count); }
            'T' => { let count = params.first().copied().unwrap_or(1) as usize; self.scroll_down(count); }
            'm' => { self.select_graphic_rendition(params); }
            'h' if intermediates == [b'?'] => { for &mode in params { self.set_mode(mode, true); } }
            'h' => { for &mode in params { self.set_mode(mode, false); } }
            'l' if intermediates == [b'?'] => { for &mode in params { self.reset_mode(mode, true); } }
            'l' => { for &mode in params { self.reset_mode(mode, false); } }
            'r' => { let top = params.first().copied().map(|v| v as usize); let bottom = params.get(1).copied().map(|v| v as usize); self.set_margins(top, bottom); self.cursor_position(1, 1); }
            's' => { self.save_cursor(); }
            'u' if intermediates == [b'?'] => { self.report_keyboard_mode(); }
            'u' if intermediates == [b'>'] => { let mode = params.first().copied().unwrap_or(0) as u16; self.push_keyboard_mode(mode); }
            'u' if intermediates == [b'<'] => { let count = params.first().copied().unwrap_or(1) as usize; self.pop_keyboard_modes(count); }
            'u' if intermediates.is_empty() && params.len() >= 2 && params[1] == 0 => {
                let mode = params[0] as u16;
                let behavior = match params.get(2).copied().unwrap_or(0) { 3 => KeyboardApplyBehavior::Difference, 2 => KeyboardApplyBehavior::Union, _ => KeyboardApplyBehavior::Replace };
                self.set_keyboard_mode(mode, behavior);
            }
            'u' => { self.restore_cursor(); }
            'g' => { let mode = params.first().copied().unwrap_or(0); self.clear_tab_stop(mode); }
            'n' => { let param = params.first().copied().unwrap_or(0) as usize; self.report_device_status(param); }
            '8' => { self.alignment_display(); }
            'c' => { self.identify_terminal_primary(); }
            'q' if intermediates == [b' '] => { let style_id = params.first().copied().unwrap_or(0) as u16; self.set_cursor_style(style_id); }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_screen(cols: usize, lines: usize) -> Screen {
        Screen::new(cols, lines)
    }

    // ── Char Tests ──────────────────────────────────────────────────────

    #[test]
    fn test_char_default() {
        let c = Char::blank();
        assert_eq!(c.data, " ");
        assert_eq!(c.fg, "default");
        assert_eq!(c.bg, "default");
        assert!(!c.bold);
    }

    #[test]
    fn test_char_width() {
        let c = Char::new("A");
        assert_eq!(c.width(), 1);
        let c2 = Char::new("こ");
        assert_eq!(c2.width(), 2);
    }

    // ── Screen Construction ─────────────────────────────────────────────

    #[test]
    fn test_screen_new() {
        let s = make_screen(80, 24);
        assert_eq!(s.columns, 80);
        assert_eq!(s.lines, 24);
        assert_eq!(s.buffer.len(), 24);
        assert_eq!(s.buffer[0].len(), 80);
        assert_eq!(s.cursor.x, 0);
        assert_eq!(s.cursor.y, 0);
    }

    #[test]
    fn test_initial_tabstops() {
        let s = make_screen(80, 24);
        assert!(s.tabstops.contains(&8));
        assert!(s.tabstops.contains(&16));
        assert!(!s.tabstops.contains(&4));
    }

    // ── Display ─────────────────────────────────────────────────────────

    #[test]
    fn test_display_empty() {
        let s = make_screen(3, 3);
        let d = s.display();
        assert_eq!(d, vec!["   ", "   ", "   "]);
    }

    // ── Draw ────────────────────────────────────────────────────────────

    #[test]
    fn test_draw_basic() {
        let mut s = make_screen(10, 1);
        s.draw("Hello");
        assert_eq!(s.display(), vec!["Hello     "]);
        assert_eq!(s.cursor.x, 5);
    }

    #[test]
    fn test_draw_wrap() {
        let mut s = make_screen(3, 3);
        s.mode.set_public(mo::LNM);
        for ch in "abc".chars() {
            s.draw_char(ch);
        }
        assert_eq!(s.display()[0], "abc");
        assert_eq!(s.cursor.x, 3); // At end of line

        s.draw_char('a'); // Should wrap
        assert_eq!(s.cursor.y, 1);
        assert_eq!(s.cursor.x, 1);
    }

    #[test]
    fn test_draw_width2() {
        let mut s = make_screen(10, 1);
        s.draw("コンニチハ");
        assert_eq!(s.cursor.x, 0);
    }

    #[test]
    fn test_draw_russian() {
        let mut s = make_screen(20, 1);
        s.draw("Нерусский текст");
        assert_eq!(s.display(), vec!["Нерусский текст     "]);
    }

    // ── Cursor Movement ─────────────────────────────────────────────────

    #[test]
    fn test_cursor_position() {
        let mut s = make_screen(10, 10);
        s.cursor_position(5, 10);
        assert_eq!(s.cursor.y, 4);
        assert_eq!(s.cursor.x, 9);
    }

    #[test]
    fn test_cursor_up_down() {
        let mut s = make_screen(10, 10);
        s.cursor_up(1);
        assert_eq!(s.cursor.y, 0); // Clamped
        s.cursor.y = 5;
        s.cursor_up(3);
        assert_eq!(s.cursor.y, 2);
        s.cursor_down(3);
        assert_eq!(s.cursor.y, 5);
    }

    #[test]
    fn test_cursor_forward_back() {
        let mut s = make_screen(10, 10);
        s.cursor_forward(5);
        assert_eq!(s.cursor.x, 5);
        s.cursor_back(3);
        assert_eq!(s.cursor.x, 2);
        s.cursor_back(10);
        assert_eq!(s.cursor.x, 0);
    }

    #[test]
    fn test_carriage_return() {
        let mut s = make_screen(10, 10);
        s.cursor.x = 7;
        s.carriage_return();
        assert_eq!(s.cursor.x, 0);
    }

    // ── SGR ─────────────────────────────────────────────────────────────

    #[test]
    fn test_sgr_bold() {
        let mut s = make_screen(2, 2);
        s.select_graphic_rendition(&[1]);
        assert!(s.cursor.attrs.bold);
        s.select_graphic_rendition(&[22]);
        assert!(!s.cursor.attrs.bold);
    }

    #[test]
    fn test_sgr_colors() {
        let mut s = make_screen(2, 2);
        s.select_graphic_rendition(&[30, 40]);
        assert_eq!(s.cursor.attrs.fg, "black");
        assert_eq!(s.cursor.attrs.bg, "black");
        s.select_graphic_rendition(&[31]);
        assert_eq!(s.cursor.attrs.fg, "red");
    }

    #[test]
    fn test_sgr_256_color() {
        let mut s = make_screen(2, 2);
        s.select_graphic_rendition(&[g::FG_256, 5, 0]);
        s.select_graphic_rendition(&[g::BG_256, 5, 15]);
        assert_eq!(s.cursor.attrs.fg, "000000");
        assert_eq!(s.cursor.attrs.bg, "ffffff");
    }

    #[test]
    fn test_sgr_24bit_color() {
        let mut s = make_screen(2, 2);
        s.select_graphic_rendition(&[38, 2, 0, 0, 0]);
        s.select_graphic_rendition(&[48, 2, 255, 255, 255]);
        assert_eq!(s.cursor.attrs.fg, "000000");
        assert_eq!(s.cursor.attrs.bg, "ffffff");
    }

    #[test]
    fn test_sgr_reset() {
        let mut s = make_screen(2, 2);
        s.select_graphic_rendition(&[30, 40]);
        s.select_graphic_rendition(&[0]);
        assert_eq!(s.cursor.attrs.fg, "default");
        assert_eq!(s.cursor.attrs.bg, "default");
    }

    #[test]
    fn test_sgr_aixterm() {
        let mut s = make_screen(2, 2);
        s.select_graphic_rendition(&[94]);
        assert_eq!(s.cursor.attrs.fg, "brightblue");
        s.select_graphic_rendition(&[104]);
        assert_eq!(s.cursor.attrs.bg, "brightblue");
    }

    // ── Erase ───────────────────────────────────────────────────────────

    #[test]
    fn test_erase_in_line() {
        let mut s = make_screen(5, 1);
        s.draw("sam i");
        s.cursor.x = 2;
        s.erase_in_line(0); // cursor to end
        assert_eq!(s.display(), vec!["sa   "]);
    }

    #[test]
    fn test_erase_in_display() {
        let mut s = make_screen(3, 3);
        s.draw("abc");
        s.linefeed();
        s.draw("def");
        s.cursor.y = 1;
        s.cursor.x = 1;
        s.erase_in_display(0); // cursor to end
        assert_eq!(s.display()[0], "abc");
        assert_eq!(s.display()[1], "d  ");
        assert_eq!(s.display()[2], "   ");
    }

    // ── Margins ─────────────────────────────────────────────────────────

    #[test]
    fn test_set_margins() {
        let mut s = make_screen(80, 24);
        s.set_margins(Some(1), Some(5));
        assert_eq!(s.margins, Some(Margins { top: 0, bottom: 4 }));
    }

    #[test]
    fn test_margins_reset() {
        let mut s = make_screen(80, 24);
        s.set_margins(Some(1), Some(5));
        s.clear_margins();
        assert_eq!(s.margins, None);
    }

    // ── Save/Restore Cursor ─────────────────────────────────────────────

    #[test]
    fn test_save_restore_cursor() {
        let mut s = make_screen(10, 10);
        s.save_cursor();
        s.cursor.x = 3;
        s.cursor.y = 5;
        s.save_cursor();
        s.cursor.x = 4;
        s.cursor.y = 4;

        s.restore_cursor();
        assert_eq!(s.cursor.x, 3);
        assert_eq!(s.cursor.y, 5);

        s.restore_cursor();
        assert_eq!(s.cursor.x, 0);
        assert_eq!(s.cursor.y, 0);
    }

    // ── Modes ───────────────────────────────────────────────────────────

    #[test]
    fn test_set_mode_deccolm() {
        let mut s = make_screen(3, 3);
        s.set_mode(mo::DECCOLM, true);
        assert_eq!(s.columns, 132);
        assert_eq!(s.cursor.x, 0);
        assert_eq!(s.cursor.y, 0);
    }

    #[test]
    fn test_set_mode_dectcem() {
        let mut s = make_screen(10, 10);
        s.cursor.hidden = true;
        s.set_mode(mo::DECTCEM, true);
        assert!(!s.cursor.hidden);
        s.reset_mode(mo::DECTCEM, true);
        assert!(s.cursor.hidden);
    }

    // ── Alignment Display ───────────────────────────────────────────────

    #[test]
    fn test_alignment_display() {
        let mut s = make_screen(5, 5);
        s.alignment_display();
        assert_eq!(s.display(), vec!["EEEEE"; 5]);
    }

    // ── Title ───────────────────────────────────────────────────────────

    #[test]
    fn test_set_title() {
        let mut s = make_screen(10, 1);
        s.set_title("test");
        assert_eq!(s.title, "test");
        s.set_icon_name("icon");
        assert_eq!(s.icon_name, "icon");
    }

    // ── Index / Reverse Index ───────────────────────────────────────────

    #[test]
    fn test_index() {
        let mut s = make_screen(2, 2);
        s.draw("wo");
        s.linefeed();
        s.draw("ot");
        s.index(); // At bottom, scrolls up: buffer[0]="ot", buffer[1]="  "
        assert_eq!(s.cursor.y, 1);
        assert_eq!(s.display()[0], "ot");
        s.index(); // Scrolls again: buffer[0]="  ", buffer[1]="  "
        assert_eq!(s.cursor.y, 1);
        assert_eq!(s.display()[0], "  ");
    }

    #[test]
    fn test_reverse_index() {
        let mut s = make_screen(2, 2);
        s.draw("wo");
        s.reverse_index();
        assert_eq!(s.cursor.y, 0);
    }

    // ── Insert/Delete Lines ─────────────────────────────────────────────

    #[test]
    fn test_insert_lines() {
        let mut s = make_screen(3, 3);
        s.draw("sam");
        s.linefeed();
        s.draw("is ");
        s.cursor.y = 0;
        s.insert_lines(1);
        assert_eq!(s.display()[0], "   ");
        assert_eq!(s.display()[1], "sam");
    }

    #[test]
    fn test_delete_lines() {
        let mut s = make_screen(3, 3);
        s.draw("sam");
        s.linefeed();
        s.draw("is ");
        s.linefeed();
        s.draw("foo");
        s.cursor.y = 0;
        s.delete_lines(1);
        assert_eq!(s.display()[0], "is ");
        assert_eq!(s.display()[1], "foo");
        assert_eq!(s.display()[2], "   ");
    }

    // ── Insert/Delete Characters ────────────────────────────────────────

    #[test]
    fn test_insert_characters() {
        let mut s = make_screen(4, 1);
        s.draw("sam");
        s.cursor.x = 0;
        s.insert_characters(1);
        assert_eq!(s.display(), vec![" sam"]);
    }

    #[test]
    fn test_delete_characters() {
        let mut s = make_screen(3, 1);
        s.draw("sam");
        s.cursor.x = 0;
        s.delete_characters(2);
        assert_eq!(s.display(), vec!["m  "]);
    }

    // ── Resize ──────────────────────────────────────────────────────────

    #[test]
    fn test_resize_wider() {
        let mut s = make_screen(2, 2);
        s.draw("bo");
        s.resize(2, 3);
        assert_eq!(s.display(), vec!["bo ", "   "]);
    }

    #[test]
    fn test_resize_narrower() {
        let mut s = make_screen(2, 2);
        s.draw("bo");
        s.resize(2, 1);
        assert_eq!(s.display(), vec!["b", " "]);
    }

    // ── Tab Stops ───────────────────────────────────────────────────────

    #[test]
    fn test_tab() {
        let mut s = make_screen(10, 10);
        s.clear_tab_stop(3); // Clear all
        s.cursor.x = 1;
        s.set_tab_stop();
        s.cursor.x = 8;
        s.set_tab_stop();
        s.cursor.x = 0;
        s.tab();
        assert_eq!(s.cursor.x, 1);
        s.tab();
        assert_eq!(s.cursor.x, 8);
    }

    // ── IRM Mode ────────────────────────────────────────────────────────

    #[test]
    fn test_irm_mode() {
        let mut s = make_screen(3, 3);
        s.mode.set_public(mo::LNM);
        for ch in "abc".chars() {
            s.draw_char(ch);
        }
        s.mode.set_public(mo::IRM);
        s.cursor.x = 0;
        s.cursor.y = 0;
        s.draw_char('x');
        assert_eq!(s.display()[0], "xab");
    }

    // ── DECAWM off ──────────────────────────────────────────────────────

    #[test]
    fn test_decawm_off() {
        let mut s = make_screen(3, 3);
        s.mode.clear_private(mo::DECAWM);
        for ch in "abc".chars() {
            s.draw_char(ch);
        }
        s.draw_char('a'); // Overwrites instead of wrapping
        assert_eq!(s.display()[0], "aba");
    }

    // ── Linefeed with LNM ───────────────────────────────────────────────

    #[test]
    fn test_linefeed_lnm() {
        let mut s = make_screen(2, 2);
        s.mode.set_public(mo::LNM);
        s.cursor.x = 1;
        s.linefeed();
        assert_eq!(s.cursor.y, 1);
        assert_eq!(s.cursor.x, 0); // CR because LNM is on
    }

    #[test]
    fn test_linefeed_no_lnm() {
        let mut s = make_screen(2, 2);
        s.mode.clear_public(mo::LNM);
        s.cursor.x = 1;
        s.linefeed();
        assert_eq!(s.cursor.y, 1);
        assert_eq!(s.cursor.x, 1); // No CR
    }

    // ── Combining Characters ────────────────────────────────────────────

    #[test]
    fn test_combining_char() {
        let mut s = make_screen(4, 2);
        s.draw("bad");
        s.draw("\u{0308}"); // Combining diaeresis
        assert!(s.display()[0].contains('d'));
    }

    // ── Cursor Style (DECSCUSR) ────────────────────────────────────────

    #[test]
    fn test_cursor_style_default() {
        let s = make_screen(10, 10);
        assert_eq!(s.cursor_style, CursorStyle::Default);
    }

    #[test]
    fn test_set_cursor_style_block() {
        let mut s = make_screen(10, 10);
        s.set_cursor_style(1); // blinking block
        assert_eq!(s.cursor_style, CursorStyle::Block);
        assert!(s.cursor_blink);
        s.set_cursor_style(2); // steady block
        assert_eq!(s.cursor_style, CursorStyle::Block);
        assert!(!s.cursor_blink);
    }

    #[test]
    fn test_set_cursor_style_underline() {
        let mut s = make_screen(10, 10);
        s.set_cursor_style(3);
        assert_eq!(s.cursor_style, CursorStyle::Underline);
        s.set_cursor_style(4); // blinking underline
        assert_eq!(s.cursor_style, CursorStyle::Underline);
    }

    #[test]
    fn test_set_cursor_style_beam() {
        let mut s = make_screen(10, 10);
        s.set_cursor_style(5);
        assert_eq!(s.cursor_style, CursorStyle::Beam);
        s.set_cursor_style(6); // blinking beam
        assert_eq!(s.cursor_style, CursorStyle::Beam);
    }

    #[test]
    fn test_set_cursor_style_reset() {
        let mut s = make_screen(10, 10);
        s.set_cursor_style(5);
        assert_eq!(s.cursor_style, CursorStyle::Beam);
        s.set_cursor_style(0); // reset to default
        assert_eq!(s.cursor_style, CursorStyle::Default);
    }

    // ── Kitty Keyboard Protocol ────────────────────────────────────────

    #[test]
    fn test_keyboard_mode_default() {
        let s = make_screen(10, 10);
        assert_eq!(s.keyboard_mode, 0);
        assert!(s.keyboard_mode_stack.is_empty());
    }

    #[test]
    fn test_set_keyboard_mode_replace() {
        let mut s = make_screen(10, 10);
        s.set_keyboard_mode(5, KeyboardApplyBehavior::Replace);
        assert_eq!(s.keyboard_mode, 5);
    }

    #[test]
    fn test_set_keyboard_mode_union() {
        let mut s = make_screen(10, 10);
        s.keyboard_mode = 0b0101;
        s.set_keyboard_mode(0b1010, KeyboardApplyBehavior::Union);
        assert_eq!(s.keyboard_mode, 0b1111);
    }

    #[test]
    fn test_set_keyboard_mode_difference() {
        let mut s = make_screen(10, 10);
        s.keyboard_mode = 0b1111;
        s.set_keyboard_mode(0b0101, KeyboardApplyBehavior::Difference);
        assert_eq!(s.keyboard_mode, 0b1010);
    }

    #[test]
    fn test_push_pop_keyboard_modes() {
        let mut s = make_screen(10, 10);
        s.keyboard_mode = 0;
        s.push_keyboard_mode(1);
        s.push_keyboard_mode(2);
        s.push_keyboard_mode(3);
        assert_eq!(s.keyboard_mode_stack, vec![1, 2, 3]);

        // Pop 1 → mode = 3, stack = [1, 2]
        s.pop_keyboard_modes(1);
        assert_eq!(s.keyboard_mode_stack, vec![1, 2]);
        assert_eq!(s.keyboard_mode, 3);

        // Pop 2 → pop 2 then 1, last popped = 1, stack = []
        s.pop_keyboard_modes(2);
        assert!(s.keyboard_mode_stack.is_empty());
        assert_eq!(s.keyboard_mode, 1);

        // Pop more than available → no change
        s.pop_keyboard_modes(10);
        assert!(s.keyboard_mode_stack.is_empty());
        assert_eq!(s.keyboard_mode, 1);
    }

    // ── Terminal Identification ────────────────────────────────────────

    #[test]
    fn test_identify_terminal_primary() {
        use std::sync::{Arc, Mutex};
        let mut s = make_screen(10, 10);
        let responses: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let resp_clone = Arc::clone(&responses);
        s.write_process_input = Box::new(move |r| {
            resp_clone.lock().unwrap().push(r.to_string());
        });
        s.identify_terminal_primary();
        assert_eq!(responses.lock().unwrap().len(), 1);
        assert!(responses.lock().unwrap()[0].contains("[?6c"));
    }

    // ── Reset clears new fields ────────────────────────────────────────

    #[test]
    fn test_reset_clears_cursor_style() {
        let mut s = make_screen(10, 10);
        s.set_cursor_style(5);
        assert_eq!(s.cursor_style, CursorStyle::Beam);
        s.reset();
        assert_eq!(s.cursor_style, CursorStyle::Default);
    }

    #[test]
    fn test_reset_clears_keyboard_mode() {
        let mut s = make_screen(10, 10);
        s.keyboard_mode = 5;
        s.keyboard_mode_stack = vec![1, 2];
        s.reset();
        assert_eq!(s.keyboard_mode, 0);
        assert!(s.keyboard_mode_stack.is_empty());
    }

    // ── Additional pyte parity tests ─────────────────────────────────

    #[test]
    fn test_backspace() {
        let mut s = make_screen(10, 10);
        s.cursor.x = 5;
        s.backspace();
        assert_eq!(s.cursor.x, 4);
        s.cursor.x = 0;
        s.backspace();
        assert_eq!(s.cursor.x, 0); // clamped
    }

    #[test]
    fn test_cursor_back_last_column() {
        let mut s = make_screen(5, 5);
        s.cursor.x = 0;
        s.cursor_back(3);
        assert_eq!(s.cursor.x, 0); // clamped to 0
    }

    #[test]
    fn test_draw_utf8() {
        let mut s = make_screen(5, 1);
        s.draw("café");
        let display = s.display();
        assert_eq!(display[0], "café ");
    }

    #[test]
    fn test_draw_multiple_chars() {
        let mut s = make_screen(10, 1);
        s.draw("hello");
        let display = s.display();
        assert_eq!(display[0], "hello     ");
    }

    #[test]
    fn test_draw_with_carriage_return() {
        let mut s = make_screen(5, 1);
        s.draw("abc");
        s.carriage_return();
        s.draw("XYZ");
        let display = s.display();
        assert_eq!(display[0], "XYZ  ");
    }

    #[test]
    fn test_clear_tabstops() {
        let mut s = make_screen(10, 10);
        s.tabstops.insert(4);
        s.tabstops.insert(8);
        s.clear_tab_stop(3);
        assert!(s.tabstops.is_empty());
    }

    #[test]
    fn test_restore_cursor_none_saved() {
        let mut s = make_screen(10, 10);
        s.cursor.x = 5;
        s.cursor.y = 5;
        s.restore_cursor();
        // No saved cursor → cursor at home
        assert_eq!(s.cursor.x, 0);
        assert_eq!(s.cursor.y, 0);
    }

    #[test]
    fn test_restore_cursor_out_of_bounds() {
        let mut s = make_screen(10, 10);
        s.save_cursor();
        s.cursor.x = 5;
        s.cursor.y = 5;
        // Resize so saved position is out of bounds
        s.resize(3, 3);
        s.restore_cursor();
        // Clamped to new bounds
        assert!(s.cursor.x < 3);
        assert!(s.cursor.y < 3);
    }

    #[test]
    fn test_erase_character() {
        let mut s = make_screen(5, 1);
        s.draw("abcde");
        s.cursor.x = 1;
        s.erase_characters(2);
        let display = s.display();
        assert_eq!(display[0], "a  de");
    }

    #[test]
    fn test_display_multi_char_emoji() {
        let mut s = make_screen(5, 1);
        s.draw("🎉x");
        let display = s.display();
        assert!(display[0].len() > 0);
    }

    #[test]
    fn test_resize_same() {
        let mut s = make_screen(5, 5);
        s.draw("abc");
        s.resize(5, 5);
        let display = s.display();
        assert_eq!(display[0], "abc  ");
    }

    #[test]
    fn test_linefeed_margins() {
        let mut s = make_screen(10, 10);
        s.set_margins(Some(2), Some(7));
        s.cursor.y = 2;
        s.cursor.x = 5;
        s.mode.set_public(mo::LNM);
        s.linefeed();
        assert_eq!(s.cursor.y, 3);
        assert_eq!(s.cursor.x, 0); // CR because LNM
    }

    #[test]
    fn test_sgr_ignore_invalid() {
        let mut s = make_screen(10, 10);
        s.select_graphic_rendition(&[99]); // unknown SGR code → no panic
    }

    #[test]
    fn test_reset_resets_colors() {
        let mut s = make_screen(10, 1);
        s.select_graphic_rendition(&[1, 31]); // bold + red
        s.draw("x");
        s.select_graphic_rendition(&[0]); // reset
        s.draw("y");
        assert_eq!(s.buffer[0][0].fg, "red");
        assert_eq!(s.buffer[0][1].fg, "default");
    }

    #[test]
    fn test_multi_attribs() {
        let mut s = make_screen(10, 1);
        s.select_graphic_rendition(&[1, 4, 31]); // bold + underline + red
        s.draw("x");
        assert_eq!(s.buffer[0][0].bold, true);
        assert_eq!(s.buffer[0][0].underscore, true);
        assert_eq!(s.buffer[0][0].fg, "red");
    }

    #[test]
    fn test_blink() {
        let mut s = make_screen(10, 1);
        s.select_graphic_rendition(&[5]);
        s.draw("x");
        assert_eq!(s.buffer[0][0].blink, true);
    }

    #[test]
    fn test_colors256_missing_attrs() {
        let mut s = make_screen(10, 1);
        s.select_graphic_rendition(&[38, 5]); // incomplete 256-color → no change
        s.draw("x");
        assert_eq!(s.buffer[0][0].fg, "default");
    }

    #[test]
    fn test_draw_width2_line_end() {
        let mut s = make_screen(3, 1);
        s.draw("ab");
        s.draw("中"); // width-2 at column 2, only 1 col left
        let display = s.display();
        // width-2 char placed at column 2, overflows beyond screen edge
        assert_eq!(display[0], "ab中");
    }

    #[test]
    fn test_draw_width2_irm() {
        let mut s = make_screen(5, 1);
        s.mode.set_private(mo::IRM);
        s.draw("ab");
        s.draw("中"); // width-2 in IRM mode
        let display = s.display();
        assert_eq!(display[0], "ab中  ");
    }

    #[test]
    fn test_set_margins_zero() {
        let mut s = make_screen(10, 10);
        s.set_margins(Some(0), Some(0));
        // Zero margins → reset to full screen
        assert_eq!(s.margins, None);
    }

    #[test]
    fn test_reset_works_between_attributes() {
        let mut s = make_screen(10, 1);
        s.select_graphic_rendition(&[1, 0, 31]); // bold, reset, red
        s.draw("x");
        assert!(!s.buffer[0][0].bold); // reset cleared bold
        assert_eq!(s.buffer[0][0].fg, "red"); // red still set
    }
}
