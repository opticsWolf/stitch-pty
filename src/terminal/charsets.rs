//! Character set definitions and mappings.
//!
//! Ported from `pyte.charsets` module. Provides VT100-compatible character
//! set designation maps (G0–G3) and the DEC Special Character and Line
//! Drawing set.

pub const ASCII: CharsetRef = CharsetRef::Ascii;
pub const DEC_SPECIAL: &str = "0";
pub const ASCII_CODE: &str = "B";

pub fn get_charset_map(code: &str) -> Option<&'static [(u8, char)]> {
    match code {
        "0" => Some(&DEC_SPECIAL_MAP),
        "A" | "B" | "4" | "5" | "6" | "7" | "8" | "9"
        | "<" | "=" | ">" | "?"
        | "C" | "D" | "E" | "F" | "G" | "H" | "I" | "J"
        | "K" | "L" | "M" | "N" | "O" | "P" | "Q" | "R" | "S" => Some(&[]),
        _ => None,
    }
}

pub const KNOWN_CHARSETS: &[&str] = &[
    "0", "A", "B", "4", "5", "6", "7", "8", "9",
    "<", "=", ">", "?",
    "C", "D", "E", "F", "G", "H", "I", "J",
    "K", "L", "M", "N", "O", "P", "Q", "R", "S",
];

const DEC_SPECIAL_MAP: [(u8, char); 32] = [
    (b'`', '\u{25c6}'), (b'a', '\u{2592}'), (b'b', '\u{2409}'), (b'c', '\u{240c}'),
    (b'd', '\u{240d}'), (b'e', '\u{240a}'), (b'f', '\u{00b0}'), (b'g', '\u{00b1}'),
    (b'h', '\u{2424}'), (b'i', '\u{240b}'), (b'j', '\u{2518}'), (b'k', '\u{2510}'),
    (b'l', '\u{250c}'), (b'm', '\u{2514}'), (b'n', '\u{253c}'), (b'o', '\u{23ba}'),
    (b'p', '\u{23bb}'), (b'q', '\u{2500}'), (b'r', '\u{23bc}'), (b's', '\u{23bd}'),
    (b't', '\u{251c}'), (b'u', '\u{2524}'), (b'v', '\u{2534}'), (b'w', '\u{252c}'),
    (b'x', '\u{2502}'), (b'y', '\u{2264}'), (b'z', '\u{2265}'), (b'{', '\u{03c0}'),
    (b'|', '\u{2260}'), (b'}', '\u{00a3}'), (b'~', '\u{00b7}'), (b'_', ' '),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum CharsetRef {
    #[default]
    Ascii,
    DecSpecial,
}

impl CharsetRef {
    pub fn map(self, byte: u8) -> char {
        match self {
            CharsetRef::Ascii => byte as char,
            CharsetRef::DecSpecial => {
                for &(b, c) in &DEC_SPECIAL_MAP {
                    if b == byte { return c; }
                }
                byte as char
            }
        }
    }

    pub fn from_designation(code: char) -> Option<Self> {
        match code {
            'B' | '(' => Some(CharsetRef::Ascii),
            '0' => Some(CharsetRef::DecSpecial),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ascii_map() {
        assert_eq!(CharsetRef::Ascii.map(b'A'), 'A');
        assert_eq!(CharsetRef::Ascii.map(b' '), ' ');
        assert_eq!(CharsetRef::Ascii.map(b'~'), '~');
    }

    #[test]
    fn test_dec_special_line_drawing() {
        let cs = CharsetRef::DecSpecial;
        assert_eq!(cs.map(b'q'), '\u{2500}'); // horizontal
        assert_eq!(cs.map(b'x'), '\u{2502}'); // vertical
        assert_eq!(cs.map(b'n'), '\u{253c}'); // cross
        assert_eq!(cs.map(b'j'), '\u{2518}'); // corner
    }

    #[test]
    fn test_dec_special_symbols() {
        let cs = CharsetRef::DecSpecial;
        assert_eq!(cs.map(b'`'), '\u{25c6}'); // diamond
        assert_eq!(cs.map(b'g'), '\u{00b1}'); // ±
        assert_eq!(cs.map(b'f'), '\u{00b0}'); // °
        assert_eq!(cs.map(b'{'), '\u{03c0}'); // π
        assert_eq!(cs.map(b'|'), '\u{2260}'); // ≠
        assert_eq!(cs.map(b'_'), ' ');        // space
    }

    #[test]
    fn test_dec_special_fallback() {
        // Bytes not in the map fall back to direct mapping
        let cs = CharsetRef::DecSpecial;
        assert_eq!(cs.map(b'A'), 'A');
        assert_eq!(cs.map(b'1'), '1');
    }

    #[test]
    fn test_from_designation() {
        assert_eq!(CharsetRef::from_designation('B'), Some(CharsetRef::Ascii));
        assert_eq!(CharsetRef::from_designation('0'), Some(CharsetRef::DecSpecial));
        assert_eq!(CharsetRef::from_designation('Z'), None);
    }

    #[test]
    fn test_get_charset_map_known() {
        assert!(get_charset_map("0").is_some());
        assert!(get_charset_map("B").is_some());
        assert!(get_charset_map("A").is_some());
        assert!(get_charset_map("Z").is_none()); // unknown charset
    }

    #[test]
    fn test_known_charsets() {
        for code in KNOWN_CHARSETS {
            assert!(get_charset_map(code).is_some(), "Charset '{}' should be known", code);
        }
    }
}
