//! ANSI escape sequence parser (reimplements the `vte` crate's parser).
//!
//! Implements the parser state machine from
//! <http://vt100.net/emu/dec_ansi_parser>. The parser handles UTF-8 input
//! and delegates parsed actions to a [`Perform`] implementation.

/// CSI parameters — a list of sub-parameter groups.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Params {
    pub subparams: Vec<Vec<u16>>,
    total_len: usize,
}

impl Params {
    pub fn new() -> Self {
        Self {
            subparams: Vec::new(),
            total_len: 0,
        }
    }

    pub fn push(&mut self, value: u16) {
        self.subparams.push(vec![value]);
        self.total_len += 1;
    }

    pub fn extend(&mut self, value: u16) {
        if self.subparams.is_empty() {
            self.subparams.push(vec![value]);
        } else if let Some(last) = self.subparams.last_mut() {
            last.push(value);
        }
        self.total_len += 1;
    }

    pub fn is_full(&self) -> bool {
        self.total_len >= MAX_PARAMS
    }

    pub fn clear(&mut self) {
        self.subparams.clear();
        self.total_len = 0;
    }

    pub fn iter(&self) -> impl Iterator<Item = &Vec<u16>> {
        self.subparams.iter()
    }
}

const MAX_PARAMS: usize = 32;
const MAX_INTERMEDIATES: usize = 2;
const MAX_OSC_PARAMS: usize = 16;
const MAX_OSC_RAW: usize = 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum State {
    CsiEntry,
    CsiIgnore,
    CsiIntermediate,
    CsiParam,
    DcsEntry,
    DcsIgnore,
    DcsIntermediate,
    DcsParam,
    DcsPassthrough,
    Escape,
    EscapeIntermediate,
    OscString,
    SosPmApcString,
    #[default]
    Ground,
}

/// Performs actions requested by the Parser.
pub trait Perform {
    fn print(&mut self, _c: char) {}
    fn execute(&mut self, _byte: u8) {}
    fn hook(&mut self, _params: &Params, _intermediates: &[u8], _ignore: bool, _action: char) {}
    fn put(&mut self, _byte: u8) {}
    fn unhook(&mut self) {}
    fn osc_dispatch(&mut self, _params: &[&[u8]], _bell_terminated: bool) {}
    fn csi_dispatch(
        &mut self,
        _params: &Params,
        _intermediates: &[u8],
        _ignore: bool,
        _action: char,
    ) {}
    fn esc_dispatch(&mut self, _intermediates: &[u8], _ignore: bool, _byte: u8) {}
    #[inline]
    fn terminated(&self) -> bool {
        false
    }
}

#[derive(Default)]
pub struct Parser {
    state: State,
    intermediates: [u8; MAX_INTERMEDIATES],
    intermediate_idx: usize,
    params: Params,
    param: u16,
    ignoring: bool,
    osc_raw: Vec<u8>,
    osc_params: [(usize, usize); MAX_OSC_PARAMS],
    osc_num_params: usize,
    partial_utf8: [u8; 4],
    partial_utf8_len: usize,
    subparam_continuation: bool,
}

impl Parser {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn advance<P: Perform>(&mut self, performer: &mut P, bytes: &[u8]) {
        let mut i = 0;

        if self.partial_utf8_len != 0 {
            i += self.advance_partial_utf8(performer, bytes);
        }

        while i != bytes.len() {
            match self.state {
                State::Ground => i += self.advance_ground(performer, &bytes[i..]),
                _ => {
                    let byte = bytes[i];
                    self.change_state(performer, byte);
                    i += 1;
                }
            }
        }
    }

    fn change_state<P: Perform>(&mut self, performer: &mut P, byte: u8) {
        match self.state {
            State::CsiEntry => self.advance_csi_entry(performer, byte),
            State::CsiIgnore => self.advance_csi_ignore(performer, byte),
            State::CsiIntermediate => self.advance_csi_intermediate(performer, byte),
            State::CsiParam => self.advance_csi_param(performer, byte),
            State::DcsEntry => self.advance_dcs_entry(performer, byte),
            State::DcsIgnore => self.anywhere(performer, byte),
            State::DcsIntermediate => self.advance_dcs_intermediate(performer, byte),
            State::DcsParam => self.advance_dcs_param(performer, byte),
            State::DcsPassthrough => self.advance_dcs_passthrough(performer, byte),
            State::Escape => self.advance_esc(performer, byte),
            State::EscapeIntermediate => self.advance_esc_intermediate(performer, byte),
            State::OscString => self.advance_osc_string(performer, byte),
            State::SosPmApcString => self.anywhere(performer, byte),
            State::Ground => unreachable!(),
        }
    }

    fn advance_ground<P: Perform>(&mut self, performer: &mut P, bytes: &[u8]) -> usize {
        let num_bytes = bytes.len();
        let plain_chars = bytes.iter().position(|&b| b == 0x1B).unwrap_or(num_bytes);

        if plain_chars == 0 {
            self.state = State::Escape;
            self.reset_params();
            return 1;
        }

        match std::str::from_utf8(&bytes[..plain_chars]) {
            Ok(parsed) => {
                Self::ground_dispatch(performer, parsed);
                let mut processed = plain_chars;
                if processed < num_bytes {
                    self.state = State::Escape;
                    self.reset_params();
                    processed += 1;
                }
                processed
            }
            Err(err) => {
                let valid_bytes = err.valid_up_to();
                let parsed = unsafe { std::str::from_utf8_unchecked(&bytes[..valid_bytes]) };
                Self::ground_dispatch(performer, parsed);

                match err.error_len() {
                    Some(len) => {
                        if len == 1 && bytes[valid_bytes] <= 0x9F {
                            performer.execute(bytes[valid_bytes]);
                        } else {
                            performer.print('\u{FFFD}');
                        }
                        valid_bytes + len
                    }
                    None => {
                        if plain_chars < num_bytes {
                            performer.print('\u{FFFD}');
                            self.state = State::Escape;
                            self.reset_params();
                            plain_chars + 1
                        } else {
                            let extra_bytes = num_bytes - valid_bytes;
                            let partial_len = self.partial_utf8_len + extra_bytes;
                            self.partial_utf8[self.partial_utf8_len..partial_len]
                                .copy_from_slice(&bytes[valid_bytes..valid_bytes + extra_bytes]);
                            self.partial_utf8_len = partial_len;
                            num_bytes
                        }
                    }
                }
            }
        }
    }

    fn ground_dispatch<P: Perform>(performer: &mut P, text: &str) {
        for c in text.chars() {
            match c {
                '\x00'..='\x1f' | '\u{80}'..='\u{9f}' => performer.execute(c as u8),
                _ => performer.print(c),
            }
        }
    }

    fn advance_partial_utf8<P: Perform>(&mut self, performer: &mut P, bytes: &[u8]) -> usize {
        let old_bytes = self.partial_utf8_len;
        let to_copy = bytes.len().min(self.partial_utf8.len() - old_bytes);
        self.partial_utf8[old_bytes..old_bytes + to_copy]
            .copy_from_slice(&bytes[..to_copy]);
        self.partial_utf8_len += to_copy;

        match std::str::from_utf8(&self.partial_utf8[..self.partial_utf8_len]) {
            Ok(parsed) => {
                let c = parsed.chars().next().unwrap();
                performer.print(c);
                self.partial_utf8_len = 0;
                c.len_utf8() - old_bytes
            }
            Err(err) => {
                let valid_bytes = err.valid_up_to();
                if valid_bytes > 0 {
                    let parsed = unsafe { std::str::from_utf8_unchecked(&self.partial_utf8[..valid_bytes]) };
                    let c = parsed.chars().next().unwrap();
                    performer.print(c);
                    self.partial_utf8_len = 0;
                    return valid_bytes - old_bytes;
                }
                match err.error_len() {
                    Some(invalid_len) => {
                        performer.print('\u{FFFD}');
                        self.partial_utf8_len = 0;
                        invalid_len - old_bytes
                    }
                    None => to_copy,
                }
            }
        }
    }

    fn advance_csi_entry<P: Perform>(&mut self, performer: &mut P, byte: u8) {
        match byte {
            0x00..=0x17 | 0x19 | 0x1C..=0x1F => performer.execute(byte),
            0x20..=0x2F => { self.action_collect(byte); self.state = State::CsiIntermediate; }
            0x30..=0x39 => { self.action_paramnext(byte); self.state = State::CsiParam; }
            0x3A => { self.action_subparam(); self.state = State::CsiParam; }
            0x3B => { self.action_param(); self.state = State::CsiParam; }
            0x3C..=0x3F => { self.action_collect(byte); self.state = State::CsiParam; }
            0x40..=0x7E => self.action_csi_dispatch(performer, byte),
            _ => self.anywhere(performer, byte),
        }
    }

    fn advance_csi_ignore<P: Perform>(&mut self, performer: &mut P, byte: u8) {
        match byte {
            0x00..=0x17 | 0x19 | 0x1C..=0x1F => performer.execute(byte),
            0x20..=0x3F => (),
            0x40..=0x7E => self.state = State::Ground,
            0x7F => (),
            _ => self.anywhere(performer, byte),
        }
    }

    fn advance_csi_intermediate<P: Perform>(&mut self, performer: &mut P, byte: u8) {
        match byte {
            0x00..=0x17 | 0x19 | 0x1C..=0x1F => performer.execute(byte),
            0x20..=0x2F => self.action_collect(byte),
            0x30..=0x3F => self.state = State::CsiIgnore,
            0x40..=0x7E => self.action_csi_dispatch(performer, byte),
            _ => self.anywhere(performer, byte),
        }
    }

    fn advance_csi_param<P: Perform>(&mut self, performer: &mut P, byte: u8) {
        match byte {
            0x00..=0x17 | 0x19 | 0x1C..=0x1F => performer.execute(byte),
            0x20..=0x2F => { self.action_collect(byte); self.state = State::CsiIntermediate; }
            0x30..=0x39 => self.action_paramnext(byte),
            0x3A => self.action_subparam(),
            0x3B => self.action_param(),
            0x3C => self.action_param(),
            0x3D => self.action_param(),
            0x3E..=0x3F => self.state = State::CsiIgnore,
            0x40..=0x7E => self.action_csi_dispatch(performer, byte),
            0x7F => (),
            _ => self.anywhere(performer, byte),
        }
    }

    fn advance_dcs_entry<P: Perform>(&mut self, performer: &mut P, byte: u8) {
        match byte {
            0x00..=0x17 | 0x19 | 0x1C..=0x1F => (),
            0x20..=0x2F => { self.action_collect(byte); self.state = State::DcsIntermediate; }
            0x30..=0x39 => { self.action_paramnext(byte); self.state = State::DcsParam; }
            0x3A => { self.action_subparam(); self.state = State::DcsParam; }
            0x3B => { self.action_param(); self.state = State::DcsParam; }
            0x3C..=0x3F => { self.action_collect(byte); self.state = State::DcsParam; }
            0x40..=0x7E => self.action_hook(performer, byte),
            0x7F => (),
            _ => self.anywhere(performer, byte),
        }
    }

    fn advance_dcs_intermediate<P: Perform>(&mut self, performer: &mut P, byte: u8) {
        match byte {
            0x00..=0x17 | 0x19 | 0x1C..=0x1F => (),
            0x20..=0x2F => self.action_collect(byte),
            0x30..=0x3F => self.state = State::DcsIgnore,
            0x40..=0x7E => self.action_hook(performer, byte),
            _ => self.anywhere(performer, byte),
        }
    }

    fn advance_dcs_param<P: Perform>(&mut self, performer: &mut P, byte: u8) {
        match byte {
            0x00..=0x17 | 0x19 | 0x1C..=0x1F => (),
            0x20..=0x2F => { self.action_collect(byte); self.state = State::DcsIntermediate; }
            0x30..=0x39 => self.action_paramnext(byte),
            0x3A => self.action_subparam(),
            0x3B => self.action_param(),
            0x3C..=0x3F => self.state = State::DcsIgnore,
            0x40..=0x7E => self.action_hook(performer, byte),
            0x7F => (),
            _ => self.anywhere(performer, byte),
        }
    }

    fn advance_dcs_passthrough<P: Perform>(&mut self, performer: &mut P, byte: u8) {
        match byte {
            0x00..=0x17 | 0x19 | 0x1C..=0x7E => performer.put(byte),
            0x18 | 0x1A => { performer.unhook(); performer.execute(byte); self.state = State::Ground; }
            0x1B => { performer.unhook(); self.reset_params(); self.state = State::Escape; }
            0x7F => (),
            0x9C => { performer.unhook(); self.state = State::Ground; }
            _ => (),
        }
    }

    fn advance_esc<P: Perform>(&mut self, performer: &mut P, byte: u8) {
        match byte {
            0x00..=0x17 | 0x19 | 0x1C..=0x1F => performer.execute(byte),
            0x20..=0x2F => { self.action_collect(byte); self.state = State::EscapeIntermediate; }
            0x30..=0x4F => { performer.esc_dispatch(self.intermediates(), self.ignoring, byte); self.state = State::Ground; }
            0x50 => { self.reset_params(); self.state = State::DcsEntry; }
            0x51..=0x57 => { performer.esc_dispatch(self.intermediates(), self.ignoring, byte); self.state = State::Ground; }
            0x58 => self.state = State::SosPmApcString,
            0x59..=0x5A => { performer.esc_dispatch(self.intermediates(), self.ignoring, byte); self.state = State::Ground; }
            0x5B => { self.reset_params(); self.state = State::CsiEntry; }
            0x5C => { performer.esc_dispatch(self.intermediates(), self.ignoring, byte); self.state = State::Ground; }
            0x5D => { self.osc_raw.clear(); self.osc_num_params = 0; self.state = State::OscString; }
            0x5E..=0x5F => self.state = State::SosPmApcString,
            0x60..=0x7E => { performer.esc_dispatch(self.intermediates(), self.ignoring, byte); self.state = State::Ground; }
            0x18 | 0x1A => { performer.execute(byte); self.state = State::Ground; }
            _ => (),
        }
    }

    fn advance_esc_intermediate<P: Perform>(&mut self, performer: &mut P, byte: u8) {
        match byte {
            0x00..=0x17 | 0x19 | 0x1C..=0x1F => performer.execute(byte),
            0x20..=0x2F => self.action_collect(byte),
            0x30..=0x7E => { performer.esc_dispatch(self.intermediates(), self.ignoring, byte); self.state = State::Ground; }
            0x7F => (),
            _ => self.anywhere(performer, byte),
        }
    }

    fn advance_osc_string<P: Perform>(&mut self, performer: &mut P, byte: u8) {
        match byte {
            0x00..=0x06 | 0x08..=0x17 | 0x19 | 0x1C..=0x1F => (),
            0x07 => { self.osc_end(performer, byte); self.state = State::Ground; }
            0x18 | 0x1A => { self.osc_end(performer, byte); performer.execute(byte); self.state = State::Ground; }
            0x1B => { self.osc_end(performer, byte); self.reset_params(); self.state = State::Escape; }
            0x3B => self.action_osc_put_param(),
            _ => self.action_osc_put(byte),
        }
    }

    fn anywhere<P: Perform>(&mut self, performer: &mut P, byte: u8) {
        match byte {
            0x18 | 0x1A => { performer.execute(byte); self.state = State::Ground; }
            0x1B => { self.reset_params(); self.state = State::Escape; }
            _ => (),
        }
    }

    fn action_csi_dispatch<P: Perform>(&mut self, performer: &mut P, byte: u8) {
        if self.params.is_full() { self.ignoring = true; }
        else {
            if self.subparam_continuation { self.params.extend(self.param); }
            else { self.params.push(self.param); }
        }
        self.param = 0;
        self.subparam_continuation = false;
        performer.csi_dispatch(&self.params, self.intermediates(), self.ignoring, byte as char);
        self.state = State::Ground;
    }

    fn action_hook<P: Perform>(&mut self, performer: &mut P, byte: u8) {
        if self.params.is_full() { self.ignoring = true; }
        else { self.params.push(self.param); }
        performer.hook(&self.params, self.intermediates(), self.ignoring, byte as char);
        self.state = State::DcsPassthrough;
    }

    fn action_collect(&mut self, byte: u8) {
        if self.intermediate_idx < MAX_INTERMEDIATES {
            self.intermediates[self.intermediate_idx] = byte;
            self.intermediate_idx += 1;
        } else { self.ignoring = true; }
    }

    fn action_subparam(&mut self) {
        if self.params.is_full() { self.ignoring = true; }
        else { self.params.extend(self.param); self.param = 0; }
        self.subparam_continuation = true;
    }

    fn action_param(&mut self) {
        if self.params.is_full() { self.ignoring = true; }
        else {
            if self.subparam_continuation { self.params.extend(self.param); }
            else { self.params.push(self.param); }
        }
        self.param = 0;
        self.subparam_continuation = false;
    }

    fn action_paramnext(&mut self, byte: u8) {
        self.param = self.param.saturating_mul(10);
        self.param = self.param.saturating_add((byte - b'0') as u16);
    }

    fn action_osc_put_param(&mut self) {
        let idx = self.osc_raw.len();
        let param_idx = self.osc_num_params;
        if param_idx >= MAX_OSC_PARAMS { return; }
        match param_idx {
            0 => self.osc_params[0] = (0, idx),
            _ => { let prev = self.osc_params[param_idx - 1]; self.osc_params[param_idx] = (prev.1, idx); }
        }
        self.osc_num_params += 1;
    }

    fn action_osc_put(&mut self, byte: u8) {
        if self.osc_raw.len() < MAX_OSC_RAW { self.osc_raw.push(byte); }
    }

    fn osc_end<P: Perform>(&mut self, performer: &mut P, byte: u8) {
        self.action_osc_put_param();
        let mut slices: Vec<&[u8]> = Vec::with_capacity(self.osc_num_params);
        for i in 0..self.osc_num_params {
            let (start, end) = self.osc_params[i];
            slices.push(&self.osc_raw[start..end]);
        }
        performer.osc_dispatch(&slices, byte == 0x07);
        self.osc_raw.clear();
        self.osc_num_params = 0;
    }

    fn intermediates(&self) -> &[u8] {
        &self.intermediates[..self.intermediate_idx]
    }

    fn reset_params(&mut self) {
        self.intermediate_idx = 0;
        self.ignoring = false;
        self.param = 0;
        self.params.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Dispatcher {
        dispatched: Vec<Sequence>,
    }

    #[derive(Debug, PartialEq, Eq)]
    enum Sequence {
        Osc(Vec<Vec<u8>>, bool),
        Csi(Vec<Vec<u16>>, Vec<u8>, bool, char),
        Esc(Vec<u8>, bool, u8),
        DcsHook(Vec<Vec<u16>>, Vec<u8>, bool, char),
        DcsPut(u8),
        Print(char),
        Execute(u8),
        DcsUnhook,
    }

    impl Perform for Dispatcher {
        fn osc_dispatch(&mut self, params: &[&[u8]], bell_terminated: bool) {
            let params = params.iter().map(|p| p.to_vec()).collect();
            self.dispatched.push(Sequence::Osc(params, bell_terminated));
        }

        fn csi_dispatch(&mut self, params: &Params, intermediates: &[u8], ignore: bool, c: char) {
            let params = params.subparams.iter().map(|sp| sp.clone()).collect();
            self.dispatched
                .push(Sequence::Csi(params, intermediates.to_vec(), ignore, c));
        }

        fn esc_dispatch(&mut self, intermediates: &[u8], ignore: bool, byte: u8) {
            self.dispatched
                .push(Sequence::Esc(intermediates.to_vec(), ignore, byte));
        }

        fn hook(&mut self, params: &Params, intermediates: &[u8], ignore: bool, c: char) {
            let params = params.subparams.iter().map(|sp| sp.clone()).collect();
            self.dispatched
                .push(Sequence::DcsHook(params, intermediates.to_vec(), ignore, c));
        }

        fn put(&mut self, byte: u8) {
            self.dispatched.push(Sequence::DcsPut(byte));
        }

        fn unhook(&mut self) {
            self.dispatched.push(Sequence::DcsUnhook);
        }

        fn print(&mut self, c: char) {
            self.dispatched.push(Sequence::Print(c));
        }

        fn execute(&mut self, byte: u8) {
            self.dispatched.push(Sequence::Execute(byte));
        }
    }

    #[test]
    fn test_print_chars() {
        let mut dispatcher = Dispatcher::default();
        let mut parser = Parser::new();
        parser.advance(&mut dispatcher, b"Hello");
        assert_eq!(dispatcher.dispatched.len(), 5);
        assert_eq!(dispatcher.dispatched[0], Sequence::Print('H'));
        assert_eq!(dispatcher.dispatched[4], Sequence::Print('o'));
    }

    #[test]
    fn test_csi_dispatch() {
        let mut dispatcher = Dispatcher::default();
        let mut parser = Parser::new();
        parser.advance(&mut dispatcher, b"\x1b[38;2;255;0;255m");
        assert_eq!(dispatcher.dispatched.len(), 1);
        match &dispatcher.dispatched[0] {
            Sequence::Csi(params, intermediates, ignore, c) => {
                // stitch-pty treats ; as param separator (not subparam)
                assert_eq!(params, &[vec![38], vec![2], vec![255], vec![0], vec![255]]);
                assert_eq!(*intermediates, Vec::<u8>::new());
                assert!(!ignore);
                assert_eq!(*c, 'm');
            }
            _ => panic!("expected CSI"),
        }
    }

    #[test]
    fn test_osc_dispatch() {
        let mut dispatcher = Dispatcher::default();
        let mut parser = Parser::new();
        parser.advance(&mut dispatcher, b"\x1b]2;Hello World\x07");
        assert_eq!(dispatcher.dispatched.len(), 1);
        match &dispatcher.dispatched[0] {
            Sequence::Osc(params, bell) => {
                assert_eq!(params.len(), 2);
                assert!(bell);
            }
            _ => panic!("expected OSC"),
        }
    }

    #[test]
    fn test_esc_dispatch() {
        let mut dispatcher = Dispatcher::default();
        let mut parser = Parser::new();
        parser.advance(&mut dispatcher, b"\x1b(A");
        assert_eq!(dispatcher.dispatched.len(), 1);
        match &dispatcher.dispatched[0] {
            Sequence::Esc(intermediates, _, byte) => {
                assert_eq!(*intermediates, vec![b'(']);
                assert_eq!(*byte, b'A');
            }
            _ => panic!("expected ESC"),
        }
    }

    #[test]
    fn test_partial_utf8() {
        let mut dispatcher = Dispatcher::default();
        let mut parser = Parser::new();
        let input = b"\xF0\x9F\x9A\x80"; // 🚀
        parser.advance(&mut dispatcher, &input[..1]);
        parser.advance(&mut dispatcher, &input[1..2]);
        parser.advance(&mut dispatcher, &input[2..3]);
        parser.advance(&mut dispatcher, &input[3..]);
        assert_eq!(dispatcher.dispatched.len(), 1);
        assert_eq!(dispatcher.dispatched[0], Sequence::Print('\u{1F680}'));
    }

    #[test]
    fn test_dcs() {
        let mut dispatcher = Dispatcher::default();
        let mut parser = Parser::new();
        parser.advance(&mut dispatcher, b"\x1bP0;1|17/ab\x9c");
        assert_eq!(dispatcher.dispatched.len(), 7);
        match &dispatcher.dispatched[0] {
            Sequence::DcsHook(params, _, _, c) => {
                assert_eq!(params, &[vec![0], vec![1]]);
                assert_eq!(*c, '|');
            }
            _ => panic!("expected DCS hook"),
        }
        assert_eq!(dispatcher.dispatched[1], Sequence::DcsPut(b'1'));
        assert_eq!(dispatcher.dispatched[6], Sequence::DcsUnhook);
    }

    // ── CSI param edge cases (from vte) ──────────────────────────────

    #[test]
    fn test_csi_trailing_semicolon() {
        // CSI 4;m → params=[4, 0]
        let mut dispatcher = Dispatcher::default();
        let mut parser = Parser::new();
        parser.advance(&mut dispatcher, b"\x1b[4;m");
        assert_eq!(dispatcher.dispatched.len(), 1);
        match &dispatcher.dispatched[0] {
            Sequence::Csi(params, ..) => assert_eq!(params, &[vec![4], vec![0]]),
            _ => panic!("expected CSI"),
        }
    }

    #[test]
    fn test_csi_leading_semicolon() {
        // CSI ;4m → params=[0, 4]
        let mut dispatcher = Dispatcher::default();
        let mut parser = Parser::new();
        parser.advance(&mut dispatcher, b"\x1b[;4m");
        assert_eq!(dispatcher.dispatched.len(), 1);
        match &dispatcher.dispatched[0] {
            Sequence::Csi(params, ..) => assert_eq!(params, &[vec![0], vec![4]]),
            _ => panic!("expected CSI"),
        }
    }

    #[test]
    fn test_csi_subparameters() {
        // CSI 38:2:255:0:255;1m → subparams [38,2,255,0,255], [1]
        let mut dispatcher = Dispatcher::default();
        let mut parser = Parser::new();
        parser.advance(&mut dispatcher, b"\x1b[38:2:255:0:255;1m");
        assert_eq!(dispatcher.dispatched.len(), 1);
        match &dispatcher.dispatched[0] {
            Sequence::Csi(params, intermediates, ignore, _) => {
                assert_eq!(params, &[vec![38, 2, 255, 0, 255], vec![1]]);
                assert_eq!(intermediates, &[] as &[u8]);
                assert!(!ignore);
            }
            _ => panic!("expected CSI"),
        }
    }

    #[test]
    fn test_csi_param_overflow() {
        // Huge number → capped at u16::MAX
        let mut dispatcher = Dispatcher::default();
        let mut parser = Parser::new();
        parser.advance(&mut dispatcher, b"\x1b[9223372036854775808m");
        assert_eq!(dispatcher.dispatched.len(), 1);
        match &dispatcher.dispatched[0] {
            Sequence::Csi(params, ..) => assert_eq!(params, &[vec![u16::MAX]]),
            _ => panic!("expected CSI"),
        }
    }

    #[test]
    fn test_csi_max_params() {
        // MAX_PARAMS-1 groups of "1;" → MAX_PARAMS params (trailing ; → implicit 0)
        let params = "1;".repeat(MAX_PARAMS - 1);
        let input = format!("\x1b[{}p", &params[..]).into_bytes();
        let mut dispatcher = Dispatcher::default();
        let mut parser = Parser::new();
        parser.advance(&mut dispatcher, &input);
        assert_eq!(dispatcher.dispatched.len(), 1);
        match &dispatcher.dispatched[0] {
            Sequence::Csi(params, _, ignore, _) => {
                assert_eq!(params.len(), MAX_PARAMS);
                assert!(!ignore);
            }
            _ => panic!("expected CSI"),
        }
    }

    #[test]
    fn test_csi_exceed_max_params() {
        // MAX_PARAMS groups of "1;" → MAX_PARAMS params, ignore flag set
        let params = "1;".repeat(MAX_PARAMS);
        let input = format!("\x1b[{}p", &params[..]).into_bytes();
        let mut dispatcher = Dispatcher::default();
        let mut parser = Parser::new();
        parser.advance(&mut dispatcher, &input);
        assert_eq!(dispatcher.dispatched.len(), 1);
        match &dispatcher.dispatched[0] {
            Sequence::Csi(params, _, ignore, _) => {
                assert_eq!(params.len(), MAX_PARAMS);
                assert!(ignore);
            }
            _ => panic!("expected CSI"),
        }
    }

    #[test]
    fn test_subparam_buffer_fill() {
        // Test CSI subparameters via colons (not semicolons)
        let mut dispatcher = Dispatcher::default();
        let mut parser = Parser::new();
        parser.advance(&mut dispatcher, b"\x1b[38:2:255:0:255m");
        assert_eq!(dispatcher.dispatched.len(), 1);
        match &dispatcher.dispatched[0] {
            Sequence::Csi(params, intermediates, ignore, c) => {
                assert_eq!(intermediates, &[] as &[u8]);
                assert!(!ignore);
                assert_eq!(*c, 'm');
                // CSI 38:2:255:0:255m -> subparams [38,2,255,0,255]
                assert_eq!(params, &[vec![38, 2, 255, 0, 255]]);
            }
            _ => panic!("expected CSI"),
        }
    }

    // ── Parser reset / interrupt tests (from vte) ────────────────────

    #[test]
    fn test_csi_reset() {
        // Interrupted CSI then new CSI → only new CSI dispatched
        let mut dispatcher = Dispatcher::default();
        let mut parser = Parser::new();
        parser.advance(&mut dispatcher, b"\x1b[3;1\x1b[?1049h");
        assert_eq!(dispatcher.dispatched.len(), 1);
        match &dispatcher.dispatched[0] {
            Sequence::Csi(params, intermediates, ignore, _) => {
                assert_eq!(intermediates, b"?");
                assert_eq!(params, &[vec![1049]]);
                assert!(!ignore);
            }
            _ => panic!("expected CSI"),
        }
    }

    #[test]
    fn test_esc_reset() {
        // Interrupted CSI then ESC sequence
        let mut dispatcher = Dispatcher::default();
        let mut parser = Parser::new();
        parser.advance(&mut dispatcher, b"\x1b[3;1\x1b(A");
        assert_eq!(dispatcher.dispatched.len(), 1);
        match &dispatcher.dispatched[0] {
            Sequence::Esc(intermediates, ignore, byte) => {
                assert_eq!(intermediates, b"(");
                assert_eq!(*byte, b'A');
                assert!(!ignore);
            }
            _ => panic!("expected ESC"),
        }
    }

    #[test]
    fn test_esc_reset_intermediates() {
        // CSI then ESC with intermediates
        let mut dispatcher = Dispatcher::default();
        let mut parser = Parser::new();
        parser.advance(&mut dispatcher, b"\x1b[?2004l\x1b#8");
        assert_eq!(dispatcher.dispatched.len(), 2);
        assert_eq!(dispatcher.dispatched[0], Sequence::Csi(vec![vec![2004]], vec![63], false, 'l'));
        assert_eq!(dispatcher.dispatched[1], Sequence::Esc(vec![35], false, 56));
    }

    #[test]
    fn test_dcs_reset() {
        // Interrupted CSI then DCS
        let mut dispatcher = Dispatcher::default();
        let mut parser = Parser::new();
        parser.advance(&mut dispatcher, b"\x1b[3;1\x1bP1$tx\x9c");
        assert_eq!(dispatcher.dispatched.len(), 3);
        match &dispatcher.dispatched[0] {
            Sequence::DcsHook(params, intermediates, ignore, _) => {
                assert_eq!(intermediates, b"$");
                assert_eq!(params, &[vec![1]]);
                assert!(!ignore);
            }
            _ => panic!("expected DCS hook"),
        }
        assert_eq!(dispatcher.dispatched[1], Sequence::DcsPut(b'x'));
        assert_eq!(dispatcher.dispatched[2], Sequence::DcsUnhook);
    }

    #[test]
    fn test_intermediate_reset_on_dcs_exit() {
        // DCS exit resets intermediates, then ESC with intermediates
        let mut dispatcher = Dispatcher::default();
        let mut parser = Parser::new();
        parser.advance(&mut dispatcher, b"\x1bP=1sZZZ\x1b+\x5c");
        assert_eq!(dispatcher.dispatched.len(), 6);
        match &dispatcher.dispatched[5] {
            Sequence::Esc(intermediates, ..) => assert_eq!(intermediates, b"+"),
            _ => panic!("expected ESC"),
        }
    }

    // ── OSC edge cases (from vte) ────────────────────────────────────

    #[test]
    fn test_osc_empty() {
        let mut dispatcher = Dispatcher::default();
        let mut parser = Parser::new();
        parser.advance(&mut dispatcher, &[0x1B, 0x5D, 0x07]);
        assert_eq!(dispatcher.dispatched.len(), 1);
        match &dispatcher.dispatched[0] {
            Sequence::Osc(..) => (),
            _ => panic!("expected OSC"),
        }
    }

    #[test]
    fn test_osc_st_terminated() {
        let mut dispatcher = Dispatcher::default();
        let mut parser = Parser::new();
        parser.advance(&mut dispatcher, b"\x1b]11;ff/00/ff\x1b\\");
        assert_eq!(dispatcher.dispatched.len(), 2);
        match &dispatcher.dispatched[0] {
            Sequence::Osc(_, false) => (), // ST terminator
            _ => panic!("expected OSC"),
        }
    }

    #[test]
    fn test_osc_with_utf8() {
        let input: &[u8] = &[0x0D, 0x1B, 0x5D, 0x32, 0x3B, 0x65, 0x63, 0x68, 0x6F, 0x20, 0x27,
            0xC2, 0xAF, 0x5C, 0x5F, 0x28, 0xE3, 0x83, 0x84, 0x29, 0x5F, 0x2F, 0xC2, 0xAF,
            0x27, 0x20, 0x26, 0x26, 0x20, 0x73, 0x6C, 0x65, 0x65, 0x70, 0x20, 0x31, 0x07];
        let mut dispatcher = Dispatcher::default();
        let mut parser = Parser::new();
        parser.advance(&mut dispatcher, input);
        assert_eq!(dispatcher.dispatched.len(), 2);
        assert_eq!(dispatcher.dispatched[0], Sequence::Execute(b'\r'));
        match &dispatcher.dispatched[1] {
            Sequence::Osc(_, true) => (),
            _ => panic!("expected OSC"),
        }
    }

    #[test]
    fn test_osc_containing_string_terminator() {
        // OSC with ST (ESC \\ ) inside content
        let mut dispatcher = Dispatcher::default();
        let mut parser = Parser::new();
        parser.advance(&mut dispatcher, b"\x1b]2;\xe6\x9c\xab\x1b\\");
        assert_eq!(dispatcher.dispatched.len(), 2);
        match &dispatcher.dispatched[0] {
            Sequence::Osc(params, _) => {
                assert_eq!(params[1], b"\xe6\x9c\xab");
            }
            _ => panic!("expected OSC"),
        }
    }

    // ── UTF-8 edge cases (from vte) ──────────────────────────────────

    #[test]
    fn test_invalid_utf8() {
        let mut dispatcher = Dispatcher::default();
        let mut parser = Parser::new();
        parser.advance(&mut dispatcher, b"a\xEF\xBCb");
        assert_eq!(dispatcher.dispatched.len(), 3);
        assert_eq!(dispatcher.dispatched[0], Sequence::Print('a'));
        assert_eq!(dispatcher.dispatched[1], Sequence::Print('\u{FFFD}'));
        assert_eq!(dispatcher.dispatched[2], Sequence::Print('b'));
    }

    #[test]
    fn test_partial_utf8_separating() {
        // "ĸ🎉" split after first byte
        let input = b"\xC4\xB8\xF0\x9F\x8E\x89"; // ĸ🎉
        let mut dispatcher = Dispatcher::default();
        let mut parser = Parser::new();
        parser.advance(&mut dispatcher, &input[..1]);
        parser.advance(&mut dispatcher, &input[1..]);
        assert_eq!(dispatcher.dispatched.len(), 2);
        assert_eq!(dispatcher.dispatched[0], Sequence::Print('\u{0138}')); // ĸ
        assert_eq!(dispatcher.dispatched[1], Sequence::Print('\u{1F389}')); // 🎉
    }

    #[test]
    fn test_partial_invalid_utf8() {
        let input = b"a\xEF\xBCb";
        let mut dispatcher = Dispatcher::default();
        let mut parser = Parser::new();
        parser.advance(&mut dispatcher, &input[..1]);
        parser.advance(&mut dispatcher, &input[1..2]);
        parser.advance(&mut dispatcher, &input[2..3]);
        parser.advance(&mut dispatcher, &input[3..]);
        assert_eq!(dispatcher.dispatched.len(), 3);
        assert_eq!(dispatcher.dispatched[0], Sequence::Print('a'));
        assert_eq!(dispatcher.dispatched[1], Sequence::Print('\u{FFFD}'));
        assert_eq!(dispatcher.dispatched[2], Sequence::Print('b'));
    }

    #[test]
    fn test_partial_invalid_utf8_split() {
        let input = b"\xE4\xBF\x99\xB5"; // U+4FD9 + invalid byte
        let mut dispatcher = Dispatcher::default();
        let mut parser = Parser::new();
        parser.advance(&mut dispatcher, &input[..2]);
        parser.advance(&mut dispatcher, &input[2..]);
        assert_eq!(dispatcher.dispatched[0], Sequence::Print('\u{4FD9}'));
        assert_eq!(dispatcher.dispatched[1], Sequence::Print('\u{FFFD}'));
    }

    #[test]
    fn test_partial_utf8_into_esc() {
        let mut dispatcher = Dispatcher::default();
        let mut parser = Parser::new();
        parser.advance(&mut dispatcher, b"\xD8\x1b012");
        assert_eq!(dispatcher.dispatched.len(), 4);
        assert_eq!(dispatcher.dispatched[0], Sequence::Print('\u{FFFD}'));
        assert_eq!(dispatcher.dispatched[1], Sequence::Esc(Vec::new(), false, b'0'));
        assert_eq!(dispatcher.dispatched[2], Sequence::Print('1'));
        assert_eq!(dispatcher.dispatched[3], Sequence::Print('2'));
    }

    // ── Execute anywhere (from vte) ──────────────────────────────────

    #[test]
    fn test_execute_anywhere() {
        let mut dispatcher = Dispatcher::default();
        let mut parser = Parser::new();
        parser.advance(&mut dispatcher, b"\x18\x1a");
        assert_eq!(dispatcher.dispatched.len(), 2);
        assert_eq!(dispatcher.dispatched[0], Sequence::Execute(0x18));
        assert_eq!(dispatcher.dispatched[1], Sequence::Execute(0x1A));
    }

    // ── Full emoji sequence (from vte) ───────────────────────────────

    #[test]
    fn test_emoji_sequence() {
        // "🎉_🦀🦀_🎉"
        let mut dispatcher = Dispatcher::default();
        let mut parser = Parser::new();
        parser.advance(&mut dispatcher, b"\xF0\x9F\x8E\x89_\xF0\x9F\xA6\x80\xF0\x9F\xA6\x80_\xF0\x9F\x8E\x89");
        assert_eq!(dispatcher.dispatched.len(), 6);
        assert_eq!(dispatcher.dispatched[0], Sequence::Print('\u{1F389}'));
        assert_eq!(dispatcher.dispatched[1], Sequence::Print('_'));
        assert_eq!(dispatcher.dispatched[2], Sequence::Print('\u{1F980}'));
        assert_eq!(dispatcher.dispatched[3], Sequence::Print('\u{1F980}'));
        assert_eq!(dispatcher.dispatched[4], Sequence::Print('_'));
        assert_eq!(dispatcher.dispatched[5], Sequence::Print('\u{1F389}'));
    }

    // ── C1 full range (from vte) ─────────────────────────────────────

    #[test]
    fn test_c1_full() {
        let mut dispatcher = Dispatcher::default();
        let mut parser = Parser::new();
        parser.advance(&mut dispatcher, b"\x00\x1f\x80\x90\x98\x9b\x9c\x9d\x9e\x9fa");
        assert_eq!(dispatcher.dispatched.len(), 11);
        assert_eq!(dispatcher.dispatched[0], Sequence::Execute(0));
        assert_eq!(dispatcher.dispatched[1], Sequence::Execute(31));
        assert_eq!(dispatcher.dispatched[2], Sequence::Execute(128));
        assert_eq!(dispatcher.dispatched[3], Sequence::Execute(144));
        assert_eq!(dispatcher.dispatched[4], Sequence::Execute(152));
        assert_eq!(dispatcher.dispatched[5], Sequence::Execute(155));
        assert_eq!(dispatcher.dispatched[6], Sequence::Execute(156));
        assert_eq!(dispatcher.dispatched[7], Sequence::Execute(157));
        assert_eq!(dispatcher.dispatched[8], Sequence::Execute(158));
        assert_eq!(dispatcher.dispatched[9], Sequence::Execute(159));
        assert_eq!(dispatcher.dispatched[10], Sequence::Print('a'));
    }
}
