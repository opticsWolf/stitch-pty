//! ANSI stream parser using our own ANSI parser state machine.
//!
//! Wraps the internal [`ansi_parser::Parser`] to translate ANSI escape sequences
//! into [`Screen`] method calls, matching the behavior of `pyte.Stream`.

use super::ansi_parser::{self, Params, Perform};
use super::screen::Screen;

/// A `Perform` implementation that translates parser events into `Screen` calls.
pub struct Performer<'a> {
    pub screen: &'a mut Screen,
}

impl<'a> Performer<'a> {
    pub fn new(screen: &'a mut Screen) -> Self {
        Self { screen }
    }
}

impl<'a> Perform for Performer<'a> {
    fn print(&mut self, c: char) {
        self.screen.draw(&c.to_string());
    }

    fn execute(&mut self, byte: u8) {
        match byte {
            b'\r' => self.screen.carriage_return(),
            b'\n' => self.screen.linefeed(),
            b'\t' => self.screen.tab(),
            b'\x08' => self.screen.backspace(),
            b'\x0b' | b'\x0c' => self.screen.linefeed(),
            b'\x0e' => self.screen.shift_out(),
            b'\x0f' => self.screen.shift_in(),
            _ => {}
        }
    }

    fn hook(&mut self, _params: &Params, _intermediates: &[u8], _ignore: bool, _action: char) {}
    fn put(&mut self, _byte: u8) {}
    fn unhook(&mut self) {}

    fn osc_dispatch(&mut self, params: &[&[u8]], _bell_terminated: bool) {
        if params.is_empty() { return; }
        let param_str = params[0];
        match param_str {
            b"0" | b"1" | b"2" => {
                let title_bytes: Vec<u8> = if params.len() > 1 {
                    params[1..].iter().copied().flatten().cloned().collect()
                } else { Vec::new() };
                let title = String::from_utf8_lossy(&title_bytes).to_string();
                if param_str == b"0" {
                    self.screen.set_icon_name(&title);
                    self.screen.set_title(&title);
                } else if param_str == b"1" {
                    self.screen.set_icon_name(&title);
                } else {
                    self.screen.set_title(&title);
                }
            }
            b"l" => {
                if params.len() >= 3 {
                    let icon = String::from_utf8_lossy(params[1]).to_string();
                    let title = String::from_utf8_lossy(params[2]).to_string();
                    self.screen.set_icon_name(&icon);
                    self.screen.set_title(&title);
                }
            }
            _ => {}
        }
    }

    fn csi_dispatch(&mut self, params: &Params, intermediates: &[u8], _ignore: bool, action: char) {
        let params_vec: Vec<u16> = params.subparams.iter()
            .flat_map(|sp| sp.iter().copied()).collect();
        self.screen.csi_dispatch(&params_vec, intermediates, action);
    }

    fn esc_dispatch(&mut self, intermediates: &[u8], _ignore: bool, byte: u8) {
        match (byte, intermediates) {
            (b'8', [b'#']) => self.screen.alignment_display(),
            (b'H', []) => self.screen.set_tab_stop(),
            (b'Z', []) => self.screen.identify_terminal_primary(),
            (b'7', []) => self.screen.save_cursor(),
            (b'8', []) => self.screen.restore_cursor(),
            (b'D', []) => self.screen.linefeed(),
            (b'E', []) => { self.screen.carriage_return(); self.screen.linefeed(); }
            (b'M', []) => self.screen.reverse_index(),
            (_, intermediates) if intermediates.len() == 1 => {
                let target = intermediates[0] as char;
                let designator = byte as char;
                self.screen.define_charset(&designator.to_string(), target);
            }
            (b'=', []) => self.screen.set_private_mode(super::modes::DECCKM),
            (b'>', []) => self.screen.reset_private_mode(super::modes::DECCKM),
            _ => {}
        }
    }
}

/// ANSI stream parser wrapping our internal state machine.
pub struct Parser {
    inner: ansi_parser::Parser,
}

impl Parser {
    pub fn new() -> Self {
        Self { inner: ansi_parser::Parser::new() }
    }

    pub fn feed(&mut self, screen: &mut Screen, data: &[u8]) {
        let mut performer = Performer::new(screen);
        self.inner.advance(&mut performer, data);
    }

    pub fn feed_str(&mut self, screen: &mut Screen, data: &str) {
        self.feed(screen, data.as_bytes());
    }

    pub fn reset(&mut self) {
        self.inner = ansi_parser::Parser::new();
    }
}

impl Default for Parser {
    fn default() -> Self { Self::new() }
}

/// High-level ANSI stream handler (equivalent to `pyte.Stream`).
pub struct Stream {
    parser: Parser,
    screen: Option<Screen>,
}

impl Stream {
    pub fn new() -> Self {
        Self { parser: Parser::new(), screen: None }
    }

    pub fn attach(&mut self, screen: Screen) {
        self.screen = Some(screen);
    }

    pub fn detach(&mut self) -> Option<Screen> {
        self.screen.take()
    }

    pub fn feed(&mut self, data: &[u8]) {
        if let Some(ref mut screen) = self.screen {
            self.parser.feed(screen, data);
        }
    }

    pub fn feed_str(&mut self, data: &str) {
        self.feed(data.as_bytes());
    }

    pub fn screen(&mut self) -> Option<&mut Screen> {
        self.screen.as_mut()
    }
}

impl Default for Stream {
    fn default() -> Self { Self::new() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::modes as mo;

    fn make_screen(cols: usize, lines: usize) -> Screen {
        Screen::new(cols, lines)
    }

    #[test]
    fn test_parser_new() {
        let _parser = Parser::new();
    }

    #[test]
    fn test_stream_new() {
        let mut stream = Stream::new();
        assert!(stream.screen().is_none());
    }

    // ── Cursor Movement ────────────────────────────────────────────────

    #[test]
    fn test_csi_cursor_up() {
        let mut s = make_screen(10, 10);
        s.cursor.y = 5;
        s.csi_dispatch(&[3], &[], 'A');
        assert_eq!(s.cursor.y, 2);
    }

    #[test]
    fn test_csi_cursor_down() {
        let mut s = make_screen(10, 10);
        s.cursor.y = 2;
        s.csi_dispatch(&[3], &[], 'B');
        assert_eq!(s.cursor.y, 5);
    }

    #[test]
    fn test_csi_cursor_forward() {
        let mut s = make_screen(10, 10);
        s.csi_dispatch(&[5], &[], 'C');
        assert_eq!(s.cursor.x, 5);
    }

    #[test]
    fn test_csi_cursor_back() {
        let mut s = make_screen(10, 10);
        s.cursor.x = 8;
        s.csi_dispatch(&[3], &[], 'D');
        assert_eq!(s.cursor.x, 5);
    }

    #[test]
    fn test_csi_cursor_position() {
        let mut s = make_screen(10, 10);
        s.csi_dispatch(&[5, 3], &[], 'H');
        assert_eq!(s.cursor.y, 4);
        assert_eq!(s.cursor.x, 2);
    }

    // ── SGR ────────────────────────────────────────────────────────────

    #[test]
    fn test_csi_sgr() {
        let mut s = make_screen(10, 10);
        s.csi_dispatch(&[1], &[], 'm');
        assert!(s.cursor.attrs.bold);
        s.csi_dispatch(&[31], &[], 'm');
        assert_eq!(s.cursor.attrs.fg, "red");
    }

    // ── Erase ──────────────────────────────────────────────────────────

    #[test]
    fn test_csi_erase_display() {
        let mut s = make_screen(5, 5);
        s.draw("hello");
        s.cursor.x = 0; // Move cursor back to start
        s.cursor.y = 0;
        s.csi_dispatch(&[0], &[], 'J');
        assert_eq!(s.display()[0], "     ");
    }

    #[test]
    fn test_csi_erase_line() {
        let mut s = make_screen(5, 1);
        s.draw("hello");
        s.cursor.x = 2;
        s.csi_dispatch(&[0], &[], 'K');
        assert_eq!(s.display(), vec!["he   "]);
    }

    // ── Modes ──────────────────────────────────────────────────────────

    #[test]
    fn test_csi_set_private_mode() {
        let mut s = make_screen(10, 10);
        s.csi_dispatch(&[mo::DECAWM], &[b'?'], 'h');
        assert!(s.mode.has_private(mo::DECAWM));
    }

    #[test]
    fn test_csi_reset_private_mode() {
        let mut s = make_screen(10, 10);
        s.csi_dispatch(&[mo::DECAWM], &[b'?'], 'l');
        assert!(!s.mode.has_private(mo::DECAWM));
    }

    #[test]
    fn test_csi_set_public_mode() {
        let mut s = make_screen(10, 10);
        s.csi_dispatch(&[mo::IRM], &[], 'h');
        assert!(s.mode.has_public(mo::IRM));
    }

    #[test]
    fn test_csi_reset_public_mode() {
        let mut s = make_screen(10, 10);
        s.csi_dispatch(&[mo::IRM], &[], 'l');
        assert!(!s.mode.has_public(mo::IRM));
    }

    // ── Scroll Region ──────────────────────────────────────────────────

    #[test]
    fn test_csi_scroll_region() {
        let mut s = make_screen(10, 24);
        s.csi_dispatch(&[1, 10], &[], 'r');
        assert_eq!(s.margins, Some(super::super::screen::Margins { top: 0, bottom: 9 }));
    }

    // ── Save/Restore Cursor ────────────────────────────────────────────

    #[test]
    fn test_csi_save_restore_cursor() {
        let mut s = make_screen(10, 10);
        s.cursor.x = 5;
        s.cursor.y = 3;
        s.csi_dispatch(&[], &[], 's');
        s.cursor.x = 0;
        s.cursor.y = 0;
        s.csi_dispatch(&[], &[], 'u');
        assert_eq!(s.cursor.x, 5);
        assert_eq!(s.cursor.y, 3);
    }

    // ── Title ──────────────────────────────────────────────────────────

    #[test]
    fn test_osc_title() {
        let mut s = make_screen(10, 10);
        let mut performer = Performer::new(&mut s);
        performer.osc_dispatch(&[b"2", b"Test Title"], false);
        assert_eq!(s.title, "Test Title");
    }

    #[test]
    fn test_osc_icon_name() {
        let mut s = make_screen(10, 10);
        let mut performer = Performer::new(&mut s);
        performer.osc_dispatch(&[b"1", b"Icon"], false);
        assert_eq!(s.icon_name, "Icon");
        assert_eq!(s.title, ""); // OSC 1 only sets icon name
    }

    #[test]
    fn test_osc_both_name() {
        let mut s = make_screen(10, 10);
        let mut performer = Performer::new(&mut s);
        performer.osc_dispatch(&[b"0", b"Both"], false);
        assert_eq!(s.icon_name, "Both");
        assert_eq!(s.title, "Both");
    }

    // ── Stream Operations ──────────────────────────────────────────────

    #[test]
    fn test_stream_attach_detach() {
        let mut stream = Stream::new();
        let screen = make_screen(10, 10);
        stream.attach(screen);
        assert!(stream.screen().is_some());
        let screen = stream.detach();
        assert_eq!(screen.unwrap().columns, 10);
        assert!(stream.screen().is_none());
    }

    #[test]
    fn test_stream_feed_text() {
        let mut stream = Stream::new();
        stream.attach(make_screen(10, 1));
        stream.feed_str("Hello");
        if let Some(s) = stream.screen() {
            assert_eq!(s.display(), vec!["Hello     "]);
        }
    }

    #[test]
    fn test_stream_feed_csi() {
        let mut stream = Stream::new();
        stream.attach(make_screen(10, 10));
        stream.feed(b"\x1b[5;3H");
        if let Some(s) = stream.screen() {
            assert_eq!(s.cursor.y, 4);
            assert_eq!(s.cursor.x, 2);
        }
    }

    // ── Performer Methods ──────────────────────────────────────────────

    #[test]
    fn test_performer_print() {
        let mut s = make_screen(10, 1);
        let mut perf = Performer::new(&mut s);
        perf.print('H');
        perf.print('i');
        assert_eq!(s.display(), vec!["Hi        "]);
    }

    #[test]
    fn test_performer_execute_cr() {
        let mut s = make_screen(10, 10);
        s.cursor.x = 5;
        let mut perf = Performer::new(&mut s);
        perf.execute(b'\r');
        assert_eq!(s.cursor.x, 0);
    }

    #[test]
    fn test_performer_execute_lf() {
        let mut s = make_screen(10, 10);
        let mut perf = Performer::new(&mut s);
        perf.execute(b'\n');
        assert_eq!(s.cursor.y, 1);
    }

    // ── Alignment Display ──────────────────────────────────────────────

    #[test]
    fn test_csi_alignment_display() {
        let mut s = make_screen(5, 5);
        s.csi_dispatch(&[], &[], '8');
        assert_eq!(s.display(), vec!["EEEEE"; 5]);
    }

    // ── Tab Stops ──────────────────────────────────────────────────────

    #[test]
    fn test_csi_clear_tab_stop() {
        let mut s = make_screen(10, 10);
        s.cursor.x = 8;
        s.csi_dispatch(&[0], &[], 'g');
        assert!(!s.tabstops.contains(&8));
    }

    // ── ESC sequences via Performer ────────────────────────────────────

    #[test]
    fn test_esc_set_tab_stop() {
        let mut s = make_screen(10, 10);
        s.cursor.x = 5;
        let mut perf = Performer::new(&mut s);
        perf.esc_dispatch(&[], false, b'H');
        assert!(s.tabstops.contains(&5));
    }

    #[test]
    fn test_esc_linefeed() {
        let mut s = make_screen(10, 10);
        let mut perf = Performer::new(&mut s);
        perf.esc_dispatch(&[], false, b'D');
        assert_eq!(s.cursor.y, 1);
    }

    #[test]
    fn test_esc_next_line() {
        let mut s = make_screen(10, 10);
        s.cursor.x = 5;
        let mut perf = Performer::new(&mut s);
        perf.esc_dispatch(&[], false, b'E');
        assert_eq!(s.cursor.x, 0);
        assert_eq!(s.cursor.y, 1);
    }

    #[test]
    fn test_esc_reverse_index() {
        let mut s = make_screen(10, 10);
        s.cursor.y = 5;
        let mut perf = Performer::new(&mut s);
        perf.esc_dispatch(&[], false, b'M');
        assert_eq!(s.cursor.y, 4);
    }

    #[test]
    fn test_esc_save_cursor() {
        let mut s = make_screen(10, 10);
        s.cursor.x = 3;
        s.cursor.y = 2;
        let mut perf = Performer::new(&mut s);
        perf.esc_dispatch(&[], false, b'7');
        assert_eq!(s.cursor.stack.len(), 1);
    }

    #[test]
    fn test_esc_restore_cursor() {
        let mut s = make_screen(10, 10);
        s.cursor.x = 3;
        s.cursor.y = 2;
        {
            let mut perf = Performer::new(&mut s);
            perf.esc_dispatch(&[], false, b'7'); // save
        }
        s.cursor.x = 0;
        s.cursor.y = 0;
        {
            let mut perf = Performer::new(&mut s);
            perf.esc_dispatch(&[], false, b'8'); // restore
        }
        assert_eq!(s.cursor.x, 3);
        assert_eq!(s.cursor.y, 2);
    }

    // ── Cursor Style (DECSCUSR) ────────────────────────────────────────

    #[test]
    fn test_cursor_style_block() {
        let mut s = make_screen(10, 10);
        s.csi_dispatch(&[1], &[b' '], 'q');
        assert_eq!(s.cursor_style, super::super::screen::CursorStyle::Block);
    }

    #[test]
    fn test_cursor_style_underline() {
        let mut s = make_screen(10, 10);
        s.csi_dispatch(&[3], &[b' '], 'q');
        assert_eq!(s.cursor_style, super::super::screen::CursorStyle::Underline);
    }

    #[test]
    fn test_cursor_style_beam() {
        let mut s = make_screen(10, 10);
        s.csi_dispatch(&[5], &[b' '], 'q');
        assert_eq!(s.cursor_style, super::super::screen::CursorStyle::Beam);
    }

    #[test]
    fn test_cursor_style_default() {
        let mut s = make_screen(10, 10);
        s.csi_dispatch(&[0], &[b' '], 'q');
        assert_eq!(s.cursor_style, super::super::screen::CursorStyle::Default);
    }

    // ── Kitty Keyboard Protocol ────────────────────────────────────────

    #[test]
    fn test_set_keyboard_mode() {
        // CSI 1=u parses as params=[1, 0], intermediates=[], final='u'
        // because '=' (0x3C) is a param separator
        let mut s = make_screen(10, 10);
        s.csi_dispatch(&[1, 0], &[], 'u');
        assert_eq!(s.keyboard_mode, 1);
    }

    #[test]
    fn test_push_keyboard_mode() {
        let mut s = make_screen(10, 10);
        s.csi_dispatch(&[1], &[b'>'], 'u');
        assert_eq!(s.keyboard_mode_stack, vec![1]);
    }

    #[test]
    fn test_pop_keyboard_modes() {
        let mut s = make_screen(10, 10);
        s.keyboard_mode_stack = vec![1, 2, 3];
        s.csi_dispatch(&[1], &[b'<'], 'u');
        // Pop 1 from [1, 2, 3] → stack = [1, 2], popped value 3 becomes active
        assert_eq!(s.keyboard_mode_stack, vec![1, 2]);
        assert_eq!(s.keyboard_mode, 3);
    }

    // ── Device Identification ──────────────────────────────────────────

    #[test]
    fn test_da1_identification() {
        use std::sync::{Arc, Mutex};
        let mut s = make_screen(10, 10);
        let responses: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let resp_clone = Arc::clone(&responses);
        s.write_process_input = Box::new(move |r| {
            resp_clone.lock().unwrap().push(r.to_string());
        });
        s.csi_dispatch(&[], &[], 'c');
        assert_eq!(responses.lock().unwrap().len(), 1);
        assert!(responses.lock().unwrap()[0].contains("[?6c"));
    }

    #[test]
    fn test_da0_identification() {
        use std::sync::{Arc, Mutex};
        let mut s = make_screen(10, 10);
        let responses: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let resp_clone = Arc::clone(&responses);
        s.write_process_input = Box::new(move |r| {
            resp_clone.lock().unwrap().push(r.to_string());
        });
        let mut performer = Performer::new(&mut s);
        performer.esc_dispatch(&[], false, b'Z');
        assert_eq!(responses.lock().unwrap().len(), 1);
        assert!(responses.lock().unwrap()[0].contains("[?6c"));
    }

    // ── Full parser integration tests ──────────────────────────────────

    #[test]
    fn test_parser_private_mode_via_feed() {
        let mut s = make_screen(10, 10);
        let mut parser = Parser::new();
        // CSI ? 7 h — set DECAWM (private mode)
        parser.feed(&mut s, b"\x1b[?7h");
        assert!(s.mode.has_private(mo::DECAWM));
    }

    #[test]
    fn test_parser_public_mode_via_feed() {
        let mut s = make_screen(10, 10);
        let mut parser = Parser::new();
        // CSI 4 h — set IRM (public mode)
        parser.feed(&mut s, b"\x1b[4h");
        assert!(s.mode.has_public(mo::IRM));
    }

    #[test]
    fn test_parser_osc_title_via_feed() {
        let mut s = make_screen(10, 10);
        let mut parser = Parser::new();
        // OSC 2;My Terminal\x07
        parser.feed(&mut s, b"\x1b]2;My Terminal\x07");
        assert_eq!(s.title, "My Terminal");
    }

    #[test]
    fn test_parser_esc_charset_via_feed() {
        let mut s = make_screen(10, 10);
        let mut parser = Parser::new();
        // ESC ( 0 — designate G0 as DEC special charset
        parser.feed(&mut s, b"\x1b(0");
        assert_eq!(s.g0_charset, super::super::charsets::CharsetRef::DecSpecial);
    }

    #[test]
    fn test_parser_cursor_style_via_feed() {
        let mut s = make_screen(10, 10);
        let mut parser = Parser::new();
        // CSI 2 SP q — blinking block cursor
        parser.feed(&mut s, b"\x1b[2 q");
        assert_eq!(s.cursor_style, super::super::screen::CursorStyle::Block);
    }

    #[test]
    fn test_parser_kitty_keyboard_via_feed() {
        let mut s = make_screen(10, 10);
        let mut parser = Parser::new();
        // CSI 3 = u — set keyboard mode 3
        parser.feed(&mut s, b"\x1b[3=u");
        assert_eq!(s.keyboard_mode, 3);
    }

    #[test]
    fn test_parser_da1_via_feed() {
        use std::sync::{Arc, Mutex};
        let mut s = make_screen(10, 10);
        let responses: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let resp_clone = Arc::clone(&responses);
        s.write_process_input = Box::new(move |r| {
            resp_clone.lock().unwrap().push(r.to_string());
        });
        let mut parser = Parser::new();
        // CSI c — DA1 (ESC [ c)
        parser.feed(&mut s, b"\x1b[c");
        assert_eq!(responses.lock().unwrap().len(), 1);
        assert!(responses.lock().unwrap()[0].contains("[?6c"));
    }

    #[test]
    fn test_parser_da0_via_feed() {
        use std::sync::{Arc, Mutex};
        let mut s = make_screen(10, 10);
        let responses: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let resp_clone = Arc::clone(&responses);
        s.write_process_input = Box::new(move |r| {
            resp_clone.lock().unwrap().push(r.to_string());
        });
        let mut parser = Parser::new();
        // ESC Z — DA0
        parser.feed(&mut s, b"\x1bZ");
        assert_eq!(responses.lock().unwrap().len(), 1);
        assert!(responses.lock().unwrap()[0].contains("[?6c"));
    }

    // ── Additional integration tests ─────────────────────────────────

    #[test]
    fn test_csi_erase_characters() {
        let mut s = make_screen(10, 1);
        s.draw("abcde");
        s.cursor.x = 1;
        s.csi_dispatch(&[3], &[], 'X'); // CSI 3 X
        let display = s.display();
        assert_eq!(display[0], "a   e     ");
    }

    #[test]
    fn test_csi_clear_tab_stop_current() {
        let mut s = make_screen(10, 10);
        s.tabstops.insert(4);
        s.tabstops.insert(8);
        s.csi_dispatch(&[], &[], 'g'); // CSI g — clear current tab stop
        s.cursor.x = 4;
        s.csi_dispatch(&[], &[], 'g');
        assert!(!s.tabstops.contains(&4));
        assert!(s.tabstops.contains(&8));
    }

    #[test]
    fn test_csi_truecolor() {
        let mut s = make_screen(10, 1);
        // CSI 38;2;128;66;255m — true color foreground
        s.csi_dispatch(&[38, 2, 128, 66, 255], &[], 'm');
        s.draw("x");
        assert_eq!(s.buffer[0][0].fg, "8042ff");
    }

    #[test]
    fn test_parser_csi_trailing_semicolon() {
        // CSI 4;m → params=[4, 0] → underline then reset
        let mut s = make_screen(10, 1);
        let mut parser = Parser::new();
        parser.feed(&mut s, b"\x1b[4;m");
        s.draw("x");
        assert_eq!(s.buffer[0][0].underscore, false); // 0 resets
    }

    #[test]
    fn test_parser_csi_leading_semicolon() {
        // CSI ;4m → params=[0, 4] → reset then underline
        let mut s = make_screen(10, 1);
        let mut parser = Parser::new();
        parser.feed(&mut s, b"\x1b[;4m");
        s.draw("x");
        assert_eq!(s.buffer[0][0].underscore, true);
    }

    #[test]
    fn test_parser_csi_subparameters() {
        // CSI 38:2:255:0:255m → true color via parser
        let mut s = make_screen(10, 1);
        let mut parser = Parser::new();
        parser.feed(&mut s, b"\x1b[38:2:255:0:255m");
        s.draw("x");
        assert_eq!(s.buffer[0][0].fg, "ff00ff");
    }

    #[test]
    fn test_parser_csi_reset() {
        // Interrupted CSI then valid CSI
        let mut s = make_screen(10, 1);
        let mut parser = Parser::new();
        parser.feed(&mut s, b"\x1b[3;1\x1b[1m");
        s.draw("x");
        assert_eq!(s.buffer[0][0].bold, true);
    }

    #[test]
    fn test_parser_esc_charset() {
        let mut s = make_screen(10, 10);
        let mut parser = Parser::new();
        parser.feed(&mut s, b"\x1b(B");
        assert_eq!(s.g0_charset, super::super::charsets::CharsetRef::Ascii);
    }

    #[test]
    fn test_parser_esc_linefeed() {
        let mut s = make_screen(10, 10);
        let mut parser = Parser::new();
        s.cursor.y = 3;
        parser.feed(&mut s, b"\x1bM"); // ESC M — reverse index
        assert_eq!(s.cursor.y, 2);
    }

    #[test]
    fn test_parser_esc_next_line() {
        let mut s = make_screen(10, 10);
        let mut parser = Parser::new();
        s.cursor.y = 3;
        parser.feed(&mut s, b"\x1bE"); // ESC E — next line
        assert_eq!(s.cursor.y, 4);
        assert_eq!(s.cursor.x, 0);
    }

    #[test]
    fn test_parser_esc_set_tab_stop() {
        let mut s = make_screen(10, 10);
        let mut parser = Parser::new();
        s.cursor.x = 5;
        parser.feed(&mut s, b"\x1bH"); // ESC H — set tab stop
        assert!(s.tabstops.contains(&5));
    }

    #[test]
    fn test_parser_osc_both_name() {
        let mut s = make_screen(10, 10);
        let mut parser = Parser::new();
        parser.feed(&mut s, b"\x1b]l;icon;title\x07"); // OSC l;icon;title BEL
        assert_eq!(s.icon_name, "icon");
        assert_eq!(s.title, "title");
    }

    #[test]
    fn test_parser_csi_alignment_display() {
        let mut s = make_screen(5, 5);
        let mut parser = Parser::new();
        parser.feed(&mut s, b"\x1b#8"); // ESC # 8 — alignment display
        let display = s.display();
        assert_eq!(display, vec!["EEEEE"; 5]);
    }

    #[test]
    fn test_parser_csi_sgr_256() {
        let mut s = make_screen(10, 1);
        let mut parser = Parser::new();
        parser.feed(&mut s, b"\x1b[38;5;196m"); // 256-color red
        s.draw("x");
        assert_eq!(s.buffer[0][0].fg, "ff0000");
    }

    #[test]
    fn test_parser_csi_sgr_aixterm() {
        let mut s = make_screen(10, 1);
        let mut parser = Parser::new();
        parser.feed(&mut s, b"\x1b[91m"); // AIXTERM bright red
        s.draw("x");
        assert_eq!(s.buffer[0][0].fg, "brightred");
    }

    #[test]
    fn test_parser_csi_scroll_region() {
        let mut s = make_screen(10, 10);
        let mut parser = Parser::new();
        parser.feed(&mut s, b"\x1b[3;7r"); // CSI 3;7 r
        assert_eq!(s.margins, Some(super::super::screen::Margins { top: 2, bottom: 6 }));
    }

    #[test]
    fn test_parser_csi_save_restore_cursor() {
        let mut s = make_screen(10, 10);
        let mut parser = Parser::new();
        s.cursor.x = 3;
        s.cursor.y = 4;
        parser.feed(&mut s, b"\x1b7"); // ESC 7 — save cursor
        s.cursor.x = 7;
        s.cursor.y = 8;
        parser.feed(&mut s, b"\x1b8"); // ESC 8 — restore cursor
        assert_eq!(s.cursor.x, 3);
        assert_eq!(s.cursor.y, 4);
    }
}
