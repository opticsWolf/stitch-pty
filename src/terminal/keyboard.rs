//! Kitty keyboard protocol key encoding.
//!
//! This module is deliberately independent of any UI toolkit: a frontend
//! translates its native key event into [`KeyEvent`] and calls [`encode_key`]
//! with the flags currently recorded by the terminal screen. `PtySession` is
//! transport-only; the encoder lives beside the terminal state that owns the
//! protocol mode.
//!
//! References:
//! - <https://sw.kovidgoyal.net/kitty/keyboard-protocol/>

use bitflags::bitflags;

bitflags! {
    /// Kitty progressive-enhancement flags (`CSI = flags ; mode u`).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
    pub struct KeyboardFlags: u16 {
        /// Disambiguate escape codes (flag 1).
        const DISAMBIGUATE = 1;
        /// Report press/repeat/release event types (flag 2).
        const REPORT_EVENT_TYPES = 2;
        /// Report shifted/base-layout alternate keys (flag 4).
        const REPORT_ALTERNATE_KEYS = 4;
        /// Report every key as an escape code (flag 8).
        const REPORT_ALL_KEYS = 8;
        /// Include associated text in CSI u sequences (flag 16).
        const REPORT_ASSOCIATED_TEXT = 16;
    }
}

bitflags! {
    /// Modifiers reported in Kitty key events.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
    pub struct Modifiers: u16 {
        const SHIFT = 1;
        const ALT = 2;
        const CTRL = 4;
        const SUPER = 8;
        const HYPER = 16;
        const META = 32;
        const CAPS_LOCK = 64;
        const NUM_LOCK = 128;
    }
}

impl Modifiers {
    /// Modifiers that affect text encoding; lock modifiers do not.
    pub fn without_locks(self) -> Self {
        self - Self::CAPS_LOCK - Self::NUM_LOCK
    }
}

/// A protocol key code.
///
/// The value is the Kitty/Unicode code point. Printable keys use their
/// unshifted Unicode code point; functional keys use Kitty's Private Use Area
/// values. Use the associated constants and constructors rather than relying
/// on raw numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KeyCode(u32);

impl KeyCode {
    pub const ENTER: Self = Self(13);
    pub const TAB: Self = Self(9);
    pub const BACKSPACE: Self = Self(127);
    pub const ESCAPE: Self = Self(27);
    pub const SPACE: Self = Self(32);

    // Kitty functional-key PUA values (see the protocol table).
    pub const INSERT: Self = Self(57348);
    pub const DELETE: Self = Self(57349);
    pub const LEFT: Self = Self(57350);
    pub const RIGHT: Self = Self(57351);
    pub const UP: Self = Self(57352);
    pub const DOWN: Self = Self(57353);
    pub const PAGE_UP: Self = Self(57354);
    pub const PAGE_DOWN: Self = Self(57355);
    pub const HOME: Self = Self(57356);
    pub const END: Self = Self(57357);
    pub const CAPS_LOCK: Self = Self(57358);
    pub const SCROLL_LOCK: Self = Self(57359);
    pub const NUM_LOCK: Self = Self(57360);
    pub const PRINT_SCREEN: Self = Self(57361);
    pub const PAUSE: Self = Self(57362);
    pub const MENU: Self = Self(57363);

    /// Construct a printable key from its unshifted character.
    pub const fn char(c: char) -> Self {
        Self(c as u32)
    }

    /// Construct F1–F35. Kitty assigns F1 = 57364, F13 = 57376, etc.
    pub const fn function(number: u8) -> Self {
        Self(57363 + number as u32)
    }

    /// Construct from a raw protocol/Unicode code point.
    pub const fn from_u32(code: u32) -> Self {
        Self(code)
    }

    /// The raw protocol/Unicode code point.
    pub const fn code(self) -> u32 {
        self.0
    }

    /// Parse a convenience key name used by the Python surface.
    ///
    /// Single-character names become printable keys. Named functional keys
    /// are case-insensitive and accept common aliases (`esc`, `return`,
    /// `pgup`, `f12`, `kp_enter`, …).
    pub fn from_name(name: &str) -> Option<Self> {
        let trimmed = name.trim();
        if trimmed.chars().count() == 1 {
            return Some(Self::char(trimmed.chars().next().unwrap()));
        }
        let normalized = trimmed.to_ascii_lowercase().replace('-', "_");
        let code = match normalized.as_str() {
            "enter" | "return" => Self::ENTER,
            "tab" => Self::TAB,
            "backspace" | "bs" => Self::BACKSPACE,
            "escape" | "esc" => Self::ESCAPE,
            "space" => Self::SPACE,
            "insert" | "ins" => Self::INSERT,
            "delete" | "del" => Self::DELETE,
            "left" => Self::LEFT,
            "right" => Self::RIGHT,
            "up" => Self::UP,
            "down" => Self::DOWN,
            "pageup" | "page_up" | "pgup" | "prior" => Self::PAGE_UP,
            "pagedown" | "page_down" | "pgdn" | "next" => Self::PAGE_DOWN,
            "home" => Self::HOME,
            "end" => Self::END,
            "capslock" | "caps_lock" => Self::CAPS_LOCK,
            "scrolllock" | "scroll_lock" => Self::SCROLL_LOCK,
            "numlock" | "num_lock" => Self::NUM_LOCK,
            "printscreen" | "print_screen" => Self::PRINT_SCREEN,
            "pause" => Self::PAUSE,
            "menu" => Self::MENU,
            // Keypad values from the protocol table.
            "kp_0" => Self::from_u32(57399),
            "kp_1" => Self::from_u32(57400),
            "kp_2" => Self::from_u32(57401),
            "kp_3" => Self::from_u32(57402),
            "kp_4" => Self::from_u32(57403),
            "kp_5" => Self::from_u32(57404),
            "kp_6" => Self::from_u32(57405),
            "kp_7" => Self::from_u32(57406),
            "kp_8" => Self::from_u32(57407),
            "kp_9" => Self::from_u32(57408),
            "kp_decimal" => Self::from_u32(57409),
            "kp_divide" => Self::from_u32(57410),
            "kp_multiply" => Self::from_u32(57411),
            "kp_subtract" => Self::from_u32(57412),
            "kp_add" => Self::from_u32(57413),
            "kp_enter" => Self::from_u32(57414),
            "kp_equal" => Self::from_u32(57415),
            "kp_separator" => Self::from_u32(57416),
            "kp_left" => Self::from_u32(57417),
            "kp_right" => Self::from_u32(57418),
            "kp_up" => Self::from_u32(57419),
            "kp_down" => Self::from_u32(57420),
            "kp_page_up" => Self::from_u32(57421),
            "kp_page_down" => Self::from_u32(57422),
            "kp_home" => Self::from_u32(57423),
            "kp_end" => Self::from_u32(57424),
            "kp_insert" => Self::from_u32(57425),
            "kp_delete" => Self::from_u32(57426),
            "kp_begin" => Self::from_u32(57427),
            "left_shift" => Self::from_u32(57441),
            "left_control" | "left_ctrl" => Self::from_u32(57442),
            "left_alt" => Self::from_u32(57443),
            "left_super" => Self::from_u32(57444),
            "right_shift" => Self::from_u32(57447),
            "right_control" | "right_ctrl" => Self::from_u32(57448),
            "right_alt" => Self::from_u32(57449),
            "right_super" => Self::from_u32(57450),
            _ => {
                let f = normalized.strip_prefix('f')?;
                let n: u8 = f.parse().ok()?;
                if (1..=35).contains(&n) {
                    Self::function(n)
                } else {
                    return None;
                }
            }
        };
        Some(code)
    }
}

/// The kind of a key event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum KeyEventKind {
    #[default]
    Press,
    Repeat,
    Release,
}

impl KeyEventKind {
    fn protocol_value(self) -> u8 {
        match self {
            Self::Press => 1,
            Self::Repeat => 2,
            Self::Release => 3,
        }
    }
}

/// A toolkit-independent key event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyEvent {
    /// Unshifted protocol key code.
    pub code: KeyCode,
    pub modifiers: Modifiers,
    pub kind: KeyEventKind,
    /// Text produced by the key, if the frontend has it (may be multi-codepoint).
    pub text: Option<String>,
    /// Shifted key value for alternate-key reporting.
    pub shifted_key: Option<char>,
    /// PC-101 base-layout key value for alternate-key reporting.
    pub base_layout_key: Option<char>,
}

impl KeyEvent {
    pub fn new(code: KeyCode, modifiers: Modifiers) -> Self {
        Self {
            code,
            modifiers,
            kind: KeyEventKind::Press,
            text: None,
            shifted_key: None,
            base_layout_key: None,
        }
    }

    pub fn press(code: KeyCode) -> Self {
        Self::new(code, Modifiers::empty())
    }

    pub fn char(c: char) -> Self {
        Self::press(KeyCode::char(c))
    }

    pub fn with_kind(mut self, kind: KeyEventKind) -> Self {
        self.kind = kind;
        self
    }

    pub fn with_text(mut self, text: impl Into<String>) -> Self {
        self.text = Some(text.into());
        self
    }

    pub fn with_shifted_key(mut self, key: char) -> Self {
        self.shifted_key = Some(key);
        self
    }

    pub fn with_base_layout_key(mut self, key: char) -> Self {
        self.base_layout_key = Some(key);
        self
    }
}

/// Encode one key event under `flags`.
///
/// With no flags this returns the legacy VT100/xterm encoding. Under flag 1
/// the Enter/Tab/Backspace exceptions and the Esc/modified-key rules follow
/// the Kitty protocol; flag 8 additionally reports text-producing keys. The
/// returned bytes may be empty for an event that the active flags cannot
/// represent (for example, a release event for a text key without flag 8).
pub fn encode_key(event: &KeyEvent, flags: KeyboardFlags) -> Vec<u8> {
    match event.code.code() {
        // Enter, Tab, Backspace, Escape.
        13 | 9 | 127 | 27 => encode_control_key(event, flags),
        // Kitty functional keys (arrows, navigation, F-keys, keypad, media).
        57348..=63743 => encode_functional_key(event, flags),
        _ => encode_text_key(event, flags),
    }
}

fn encode_text_key(event: &KeyEvent, flags: KeyboardFlags) -> Vec<u8> {
    let all_keys = flags.contains(KeyboardFlags::REPORT_ALL_KEYS);
    let disambiguate = flags.contains(KeyboardFlags::DISAMBIGUATE);
    let text_mods = event.modifiers.without_locks();
    let has_extra = text_mods.intersects(
        Modifiers::CTRL | Modifiers::ALT | Modifiers::SUPER | Modifiers::HYPER | Modifiers::META,
    );
    let ctrl_shift = text_mods.contains(Modifiers::CTRL | Modifiers::SHIFT);
    let legacy_unrepresentable =
        text_mods.intersects(Modifiers::SUPER | Modifiers::HYPER | Modifiers::META) || ctrl_shift;

    if all_keys || (disambiguate && has_extra) || (!disambiguate && legacy_unrepresentable) {
        return csi_u(event, flags);
    }

    // A requested non-press event cannot be represented for a text key unless
    // every key is reported as an escape code.
    if event.kind != KeyEventKind::Press && flags.contains(KeyboardFlags::REPORT_EVENT_TYPES) {
        return Vec::new();
    }

    if !text_mods.intersects(Modifiers::CTRL | Modifiers::ALT) {
        if let Some(text) = &event.text
            && is_safe_text(text)
        {
            return text.as_bytes().to_vec();
        }
        let ch = event
            .code
            .code()
            .try_into()
            .ok()
            .filter(|c: &char| !c.is_control());
        if let Some(ch) = ch {
            let ch = if text_mods.contains(Modifiers::SHIFT) {
                event.shifted_key.unwrap_or_else(|| shift_char(ch))
            } else {
                ch
            };
            return ch.to_string().into_bytes();
        }
    }

    legacy_text_key(event, text_mods)
}

fn encode_control_key(event: &KeyEvent, flags: KeyboardFlags) -> Vec<u8> {
    let all_keys = flags.contains(KeyboardFlags::REPORT_ALL_KEYS);
    let disambiguate = flags.contains(KeyboardFlags::DISAMBIGUATE);
    let text_mods = event.modifiers.without_locks();
    let is_exception = matches!(event.code.code(), 13 | 9 | 127);
    let escaped = event.code.code() == 27;
    let has_mods = !text_mods.is_empty();
    let ctrl_shift_tab = event.code.code() == 9
        && text_mods.contains(Modifiers::CTRL)
        && text_mods.contains(Modifiers::SHIFT);

    // Enter/Tab/Backspace deliberately keep release events in legacy mode
    // unless report-all-keys is active, so a crashed app can still be reset.
    if is_exception
        && event.kind != KeyEventKind::Press
        && flags.contains(KeyboardFlags::REPORT_EVENT_TYPES)
        && !all_keys
    {
        return Vec::new();
    }

    let csi = all_keys
        || (disambiguate && (has_mods || escaped))
        || ctrl_shift_tab
        || (!disambiguate
            && text_mods.intersects(Modifiers::SUPER | Modifiers::HYPER | Modifiers::META));

    if csi {
        csi_u(event, flags)
    } else {
        legacy_control_key(event.code.code(), text_mods)
    }
}

fn encode_functional_key(event: &KeyEvent, flags: KeyboardFlags) -> Vec<u8> {
    let code = event.code.code();
    let include_locks = flags.contains(KeyboardFlags::REPORT_ALL_KEYS);
    let modifier = modifier_value(event.modifiers, include_locks);
    let event_type = event_type(event, flags);
    let has_fields = modifier != 1 || event_type.is_some();
    let type_suffix = event_type.map(|t| format!(":{}", t)).unwrap_or_default();

    // These keys have canonical legacy CSI/SS3 forms even under flag 8.
    let (number, letter) = match code {
        // CSI 1 ; mods A/B/C/D/H/F
        57350 => (None, Some('D')),
        57351 => (None, Some('C')),
        57352 => (None, Some('A')),
        57353 => (None, Some('B')),
        57356 => (None, Some('H')),
        57357 => (None, Some('F')),
        // F1/F2/F4 use SS3 without modifiers and CSI 1;mods with them.
        57364 => (None, Some('P')),
        57365 => (None, Some('Q')),
        57367 => (None, Some('S')),
        // CSI number ; mods ~
        57348 => (Some(2), None),
        57349 => (Some(3), None),
        57354 => (Some(5), None),
        57355 => (Some(6), None),
        57366 => (Some(13), None), // F3
        57368 => (Some(15), None), // F5
        57369 => (Some(17), None), // F6
        57370 => (Some(18), None), // F7
        57371 => (Some(19), None), // F8
        57372 => (Some(20), None), // F9
        57373 => (Some(21), None), // F10
        57374 => (Some(23), None), // F11
        57375 => (Some(24), None), // F12
        _ => return csi_u(event, flags),
    };

    if let Some(letter) = letter {
        if !has_fields {
            if code == 57364 || code == 57365 || code == 57367 {
                return format!("\x1bO{}", letter).into_bytes();
            }
            return format!("\x1b[{}", letter).into_bytes();
        }
        return format!("\x1b[1;{}{}{}", modifier, type_suffix, letter).into_bytes();
    }

    let number = number.expect("number form");
    if !has_fields {
        format!("\x1b[{}~", number).into_bytes()
    } else {
        format!("\x1b[{};{}{}~", number, modifier, type_suffix).into_bytes()
    }
}

fn csi_u(event: &KeyEvent, flags: KeyboardFlags) -> Vec<u8> {
    let include_locks = flags.contains(KeyboardFlags::REPORT_ALL_KEYS);
    let modifier = modifier_value(event.modifiers, include_locks);
    let event_type = event_type(event, flags);
    let has_fields = modifier != 1 || event_type.is_some();

    let mut out = format!("\x1b[{}", event.code.code());

    if flags.contains(KeyboardFlags::REPORT_ALTERNATE_KEYS) {
        let shifted = if event.modifiers.contains(Modifiers::SHIFT) {
            event.shifted_key
        } else {
            None
        };
        match (shifted, event.base_layout_key) {
            (Some(shifted), Some(base)) => {
                out.push_str(&format!(":{}:{}", shifted as u32, base as u32));
            }
            (Some(shifted), None) => out.push_str(&format!(":{}", shifted as u32)),
            (None, Some(base)) => out.push_str(&format!("::{}", base as u32)),
            (None, None) => {}
        }
    }

    let text = associated_text(event, flags);
    if has_fields {
        out.push(';');
        out.push_str(&modifier.to_string());
        if let Some(t) = event_type {
            out.push(':');
            out.push_str(&t.to_string());
        }
    } else if text.is_some() {
        out.push(';');
    }
    if let Some(text) = text {
        out.push(';');
        out.push_str(&text);
    }
    out.push('u');
    out.into_bytes()
}

fn associated_text(event: &KeyEvent, flags: KeyboardFlags) -> Option<String> {
    if !flags.contains(KeyboardFlags::REPORT_ASSOCIATED_TEXT) {
        return None;
    }
    let text = event.text.as_deref()?;
    if !is_safe_text(text) {
        return None;
    }
    Some(
        text.chars()
            .map(|c| (c as u32).to_string())
            .collect::<Vec<_>>()
            .join(":"),
    )
}

fn event_type(event: &KeyEvent, flags: KeyboardFlags) -> Option<u8> {
    if !flags.contains(KeyboardFlags::REPORT_EVENT_TYPES) || event.kind == KeyEventKind::Press {
        return None;
    }
    Some(event.kind.protocol_value())
}

fn modifier_value(modifiers: Modifiers, include_locks: bool) -> u16 {
    let mut bits = modifiers.bits() & 0x3f; // shift .. meta
    if include_locks {
        bits |= modifiers.bits() & (Modifiers::CAPS_LOCK.bits() | Modifiers::NUM_LOCK.bits());
    }
    1 + bits
}

fn is_safe_text(text: &str) -> bool {
    !text.is_empty()
        && text.chars().all(|c| {
            let code = c as u32;
            code >= 0x20 && !(0x7f..=0x9f).contains(&code)
        })
}

fn shift_char(c: char) -> char {
    match c {
        'a'..='z' => c.to_ascii_uppercase(),
        '0' => ')',
        '1' => '!',
        '2' => '@',
        '3' => '#',
        '4' => '$',
        '5' => '%',
        '6' => '^',
        '7' => '&',
        '8' => '*',
        '9' => '(',
        '-' => '_',
        '=' => '+',
        '[' => '{',
        ']' => '}',
        '\\' => '|',
        ';' => ':',
        '\'' => '"',
        ',' => '<',
        '.' => '>',
        '/' => '?',
        '`' => '~',
        _ => c,
    }
}

fn legacy_text_key(event: &KeyEvent, text_mods: Modifiers) -> Vec<u8> {
    let Some(ch) = char::from_u32(event.code.code()) else {
        return Vec::new();
    };
    let alt = text_mods.contains(Modifiers::ALT);
    let ctrl = text_mods.contains(Modifiers::CTRL);
    let shift = text_mods.contains(Modifiers::SHIFT);
    let shifted = event.shifted_key.unwrap_or_else(|| shift_char(ch));
    let mut out = Vec::new();
    if alt {
        out.push(0x1b);
    }
    if ctrl {
        if let Some(byte) = legacy_ctrl_byte(ch) {
            out.push(byte);
        } else if shift {
            out.extend_from_slice(shifted.to_string().as_bytes());
        } else {
            out.extend_from_slice(ch.to_string().as_bytes());
        }
    } else if shift {
        out.extend_from_slice(shifted.to_string().as_bytes());
    } else {
        out.extend_from_slice(ch.to_string().as_bytes());
    }
    out
}

fn legacy_ctrl_byte(c: char) -> Option<u8> {
    let c = c.to_ascii_lowercase();
    Some(match c {
        ' ' | '@' => 0,
        'a'..='z' => (c as u8) - b'a' + 1,
        '[' => 27,
        '\\' => 28,
        ']' => 29,
        '^' => 30,
        '_' => 31,
        '?' => 127,
        '2' => 0,
        '3' => 27,
        '4' => 28,
        '5' => 29,
        '6' => 30,
        '7' => 31,
        '8' => 127,
        '9' => 57,
        '/' => 31,
        _ => return None,
    })
}

fn legacy_control_key(code: u32, mods: Modifiers) -> Vec<u8> {
    let alt = mods.contains(Modifiers::ALT);
    let ctrl = mods.contains(Modifiers::CTRL);
    let shift = mods.contains(Modifiers::SHIFT);
    let mut out = Vec::new();
    if alt {
        out.push(0x1b);
    }
    match code {
        13 => out.push(0x0d),
        127 => {
            if ctrl {
                out.push(0x08);
            } else {
                out.push(0x7f);
            }
        }
        9 => {
            if ctrl && shift {
                // Ctrl+Shift+Tab is one of the legacy combinations that uses
                // CSI u rather than a legacy byte.
                return format!("\x1b[9;{}u", modifier_value(mods, false)).into_bytes();
            } else if shift {
                out.extend_from_slice(b"\x1b[Z");
            } else {
                out.push(0x09);
            }
        }
        27 => out.push(0x1b),
        _ => {}
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const F1: KeyboardFlags = KeyboardFlags::DISAMBIGUATE;
    const ALL: KeyboardFlags = KeyboardFlags::REPORT_ALL_KEYS;
    const EVENTS: KeyboardFlags = KeyboardFlags::REPORT_EVENT_TYPES;
    const ALTS: KeyboardFlags = KeyboardFlags::REPORT_ALTERNATE_KEYS;
    const TEXT: KeyboardFlags = KeyboardFlags::REPORT_ASSOCIATED_TEXT;

    fn event(code: KeyCode, mods: Modifiers) -> KeyEvent {
        KeyEvent::new(code, mods)
    }

    #[test]
    fn legacy_text_fallback() {
        assert_eq!(
            encode_key(
                &event(KeyCode::char('a'), Modifiers::empty()),
                KeyboardFlags::empty()
            ),
            b"a"
        );
        assert_eq!(
            encode_key(
                &event(KeyCode::char('a'), Modifiers::SHIFT),
                KeyboardFlags::empty()
            ),
            b"A"
        );
        assert_eq!(
            encode_key(
                &event(KeyCode::char('a'), Modifiers::CTRL),
                KeyboardFlags::empty()
            ),
            b"\x01"
        );
        assert_eq!(
            encode_key(
                &event(KeyCode::char('a'), Modifiers::ALT),
                KeyboardFlags::empty()
            ),
            b"\x1ba"
        );
        assert_eq!(
            encode_key(
                &event(KeyCode::char('a'), Modifiers::CTRL | Modifiers::SHIFT),
                KeyboardFlags::empty()
            ),
            b"\x1b[97;6u"
        );
        assert_eq!(
            encode_key(
                &event(KeyCode::char('a'), Modifiers::SUPER),
                KeyboardFlags::empty()
            ),
            b"\x1b[97;9u"
        );
    }

    #[test]
    fn flag1_enter_tab_backspace_exceptions() {
        assert_eq!(
            encode_key(&event(KeyCode::ENTER, Modifiers::empty()), F1),
            b"\r"
        );
        assert_eq!(
            encode_key(&event(KeyCode::TAB, Modifiers::empty()), F1),
            b"\t"
        );
        assert_eq!(
            encode_key(&event(KeyCode::BACKSPACE, Modifiers::empty()), F1),
            b"\x7f"
        );
        assert_eq!(
            encode_key(&event(KeyCode::ENTER, Modifiers::SHIFT), F1),
            b"\x1b[13;2u"
        );
        assert_eq!(
            encode_key(&event(KeyCode::ENTER, Modifiers::ALT), F1),
            b"\x1b[13;3u"
        );
        assert_eq!(
            encode_key(&event(KeyCode::TAB, Modifiers::SHIFT), F1),
            b"\x1b[9;2u"
        );
    }

    #[test]
    fn flag1_escape_and_ctrl_i() {
        assert_eq!(
            encode_key(&event(KeyCode::ESCAPE, Modifiers::empty()), F1),
            b"\x1b[27u"
        );
        assert_eq!(
            encode_key(&event(KeyCode::ESCAPE, Modifiers::ALT), F1),
            b"\x1b[27;3u"
        );
        assert_eq!(
            encode_key(&event(KeyCode::char('a'), Modifiers::ALT), F1),
            b"\x1b[97;3u"
        );
        assert_eq!(
            encode_key(&event(KeyCode::char('i'), Modifiers::CTRL), F1),
            b"\x1b[105;5u"
        );
        assert_eq!(
            encode_key(
                &event(KeyCode::char('i'), Modifiers::CTRL | Modifiers::SHIFT),
                F1
            ),
            b"\x1b[105;6u"
        );
        assert_eq!(
            encode_key(
                &event(KeyCode::char('a'), Modifiers::CTRL | Modifiers::ALT),
                F1
            ),
            b"\x1b[97;7u"
        );
        assert_eq!(
            encode_key(
                &event(KeyCode::char('a'), Modifiers::ALT | Modifiers::SHIFT),
                F1
            ),
            b"\x1b[97;4u"
        );
    }

    #[test]
    fn flag1_plain_text_and_shift_text_stay_text() {
        assert_eq!(
            encode_key(&event(KeyCode::char('a'), Modifiers::empty()), F1),
            b"a"
        );
        assert_eq!(
            encode_key(&event(KeyCode::char('a'), Modifiers::SHIFT), F1),
            b"A"
        );
        assert_eq!(
            encode_key(
                &event(KeyCode::char('a'), Modifiers::SHIFT).with_text("A"),
                F1
            ),
            b"A"
        );
    }

    #[test]
    fn flag8_reports_all_keys() {
        assert_eq!(
            encode_key(&event(KeyCode::char('a'), Modifiers::empty()), ALL),
            b"\x1b[97u"
        );
        assert_eq!(
            encode_key(&event(KeyCode::ENTER, Modifiers::empty()), ALL),
            b"\x1b[13u"
        );
        assert_eq!(
            encode_key(&event(KeyCode::TAB, Modifiers::empty()), ALL),
            b"\x1b[9u"
        );
        assert_eq!(
            encode_key(&event(KeyCode::BACKSPACE, Modifiers::empty()), ALL),
            b"\x1b[127u"
        );
        assert_eq!(
            encode_key(&event(KeyCode::char('a'), Modifiers::SHIFT), ALL),
            b"\x1b[97;2u"
        );
    }

    #[test]
    fn event_types_and_associated_text() {
        let release =
            event(KeyCode::char('a'), Modifiers::empty()).with_kind(KeyEventKind::Release);
        assert_eq!(encode_key(&release, ALL | EVENTS), b"\x1b[97;1:3u");
        assert_eq!(
            encode_key(
                &event(KeyCode::char('a'), Modifiers::CTRL).with_kind(KeyEventKind::Release),
                F1 | EVENTS
            ),
            b"\x1b[97;5:3u"
        );
        assert_eq!(
            encode_key(
                &event(KeyCode::char('a'), Modifiers::SHIFT).with_text("A"),
                ALL | TEXT
            ),
            b"\x1b[97;2;65u"
        );
        assert_eq!(
            encode_key(
                &event(KeyCode::char('a'), Modifiers::SHIFT).with_text("A"),
                F1 | TEXT
            ),
            b"A"
        );
        assert_eq!(encode_key(&release, EVENTS), b"");
    }

    #[test]
    fn alternate_keys() {
        let e = event(KeyCode::char('a'), Modifiers::SHIFT)
            .with_shifted_key('A')
            .with_base_layout_key('a');
        assert_eq!(encode_key(&e, ALL | ALTS), b"\x1b[97:65:97;2u");
        let base_only = event(KeyCode::char('a'), Modifiers::SHIFT).with_base_layout_key('a');
        assert_eq!(encode_key(&base_only, ALL | ALTS), b"\x1b[97::97;2u");
    }

    #[test]
    fn legacy_functional_keys() {
        assert_eq!(
            encode_key(
                &event(KeyCode::LEFT, Modifiers::empty()),
                KeyboardFlags::empty()
            ),
            b"\x1b[D"
        );
        assert_eq!(
            encode_key(
                &event(KeyCode::UP, Modifiers::SHIFT),
                KeyboardFlags::empty()
            ),
            b"\x1b[1;2A"
        );
        assert_eq!(
            encode_key(
                &event(KeyCode::INSERT, Modifiers::empty()),
                KeyboardFlags::empty()
            ),
            b"\x1b[2~"
        );
        assert_eq!(
            encode_key(
                &event(KeyCode::PAGE_DOWN, Modifiers::CTRL),
                KeyboardFlags::empty()
            ),
            b"\x1b[6;5~"
        );
        assert_eq!(
            encode_key(
                &event(KeyCode::function(1), Modifiers::empty()),
                KeyboardFlags::empty()
            ),
            b"\x1bOP"
        );
        assert_eq!(
            encode_key(
                &event(KeyCode::function(5), Modifiers::empty()),
                KeyboardFlags::empty()
            ),
            b"\x1b[15~"
        );
        assert_eq!(
            encode_key(
                &event(KeyCode::function(13), Modifiers::empty()),
                KeyboardFlags::empty()
            ),
            b"\x1b[57376u"
        );
    }

    #[test]
    fn flag1_functional_keys_keep_canonical_forms() {
        assert_eq!(
            encode_key(&event(KeyCode::LEFT, Modifiers::empty()), F1),
            b"\x1b[D"
        );
        assert_eq!(
            encode_key(&event(KeyCode::LEFT, Modifiers::SHIFT), F1),
            b"\x1b[1;2D"
        );
        assert_eq!(
            encode_key(&event(KeyCode::function(1), Modifiers::empty()), F1),
            b"\x1bOP"
        );
        assert_eq!(
            encode_key(&event(KeyCode::function(1), Modifiers::SHIFT), F1),
            b"\x1b[1;2P"
        );
    }

    #[test]
    fn keypad_reports_csi_u() {
        let kp = KeyCode::from_name("kp_1").unwrap();
        assert_eq!(
            encode_key(&event(kp, Modifiers::empty()), KeyboardFlags::empty()),
            b"\x1b[57400u"
        );
    }

    #[test]
    fn tab_legacy_variants() {
        assert_eq!(
            encode_key(
                &event(KeyCode::TAB, Modifiers::SHIFT),
                KeyboardFlags::empty()
            ),
            b"\x1b[Z"
        );
        assert_eq!(
            encode_key(
                &event(KeyCode::TAB, Modifiers::CTRL | Modifiers::SHIFT),
                KeyboardFlags::empty()
            ),
            b"\x1b[9;6u"
        );
        assert_eq!(
            encode_key(
                &event(KeyCode::BACKSPACE, Modifiers::CTRL),
                KeyboardFlags::empty()
            ),
            b"\x08"
        );
        assert_eq!(
            encode_key(
                &event(KeyCode::ENTER, Modifiers::ALT),
                KeyboardFlags::empty()
            ),
            b"\x1b\r"
        );
    }
}
