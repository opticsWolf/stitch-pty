//! Graphic-related constants and color lookup functions for SGR sequences.

use std::sync::OnceLock;

pub fn text_attr(code: i32) -> Option<TextAttr> {
    match code {
        1 => Some(TextAttr::SetBold), 3 => Some(TextAttr::SetItalics),
        4 => Some(TextAttr::SetUnderline), 5 => Some(TextAttr::SetBlink),
        7 => Some(TextAttr::SetReverse), 9 => Some(TextAttr::SetStrikethrough),
        22 => Some(TextAttr::ResetBold), 23 => Some(TextAttr::ResetItalics),
        24 => Some(TextAttr::ResetUnderline), 25 => Some(TextAttr::ResetBlink),
        27 => Some(TextAttr::ResetReverse), 29 => Some(TextAttr::ResetStrikethrough),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextAttr {
    SetBold, ResetBold, SetItalics, ResetItalics,
    SetUnderline, ResetUnderline, SetBlink, ResetBlink,
    SetReverse, ResetReverse, SetStrikethrough, ResetStrikethrough,
}

impl TextAttr {
    pub fn is_set(&self) -> bool {
        matches!(self, TextAttr::SetBold | TextAttr::SetItalics | TextAttr::SetUnderline
            | TextAttr::SetBlink | TextAttr::SetReverse | TextAttr::SetStrikethrough)
    }
}

pub fn fg_ansi(code: i32) -> Option<&'static str> {
    match code {
        30 => Some("black"), 31 => Some("red"), 32 => Some("green"),
        33 => Some("brown"), 34 => Some("blue"), 35 => Some("magenta"),
        36 => Some("cyan"), 37 => Some("white"), 39 => Some("default"),
        _ => None,
    }
}

pub fn fg_aixterm(code: i32) -> Option<&'static str> {
    match code {
        90 => Some("brightblack"), 91 => Some("brightred"), 92 => Some("brightgreen"),
        93 => Some("brightbrown"), 94 => Some("brightblue"), 95 => Some("brightmagenta"),
        96 => Some("brightcyan"), 97 => Some("brightwhite"),
        _ => None,
    }
}

pub fn bg_ansi(code: i32) -> Option<&'static str> {
    match code {
        40 => Some("black"), 41 => Some("red"), 42 => Some("green"),
        43 => Some("brown"), 44 => Some("blue"), 45 => Some("magenta"),
        46 => Some("cyan"), 47 => Some("white"), 49 => Some("default"),
        _ => None,
    }
}

pub fn bg_aixterm(code: i32) -> Option<&'static str> {
    match code {
        100 => Some("brightblack"), 101 => Some("brightred"), 102 => Some("brightgreen"),
        103 => Some("brightbrown"), 104 => Some("brightblue"), 105 => Some("brightmagenta"),
        106 => Some("brightcyan"), 107 => Some("brightwhite"),
        _ => None,
    }
}

pub const FG_256: u16 = 38;
pub const BG_256: u16 = 48;

fn get_256_palette() -> &'static [String] {
    static PALETTE: OnceLock<Vec<String>> = OnceLock::new();
    PALETTE.get_or_init(|| {
        let mut palette = Vec::with_capacity(256);
        let standard = [
            (0x00u8, 0x00u8, 0x00u8), (0xcd, 0x00, 0x00), (0x00, 0xcd, 0x00),
            (0xcd, 0xcd, 0x00), (0x00, 0x00, 0xee), (0xcd, 0x00, 0xcd),
            (0x00, 0xcd, 0xcd), (0xe5, 0xe5, 0xe5), (0x7f, 0x7f, 0x7f),
            (0xff, 0x00, 0x00), (0x00, 0xff, 0x00), (0xff, 0xff, 0x00),
            (0x5c, 0x5c, 0xff), (0xff, 0x00, 0xff), (0x00, 0xff, 0xff),
            (0xff, 0xff, 0xff),
        ];
        for &(r, g, b) in &standard {
            palette.push(format!("{:02x}{:02x}{:02x}", r, g, b));
        }
        let values = [0x00u8, 0x5f, 0x87, 0xaf, 0xd7, 0xff];
        for &r in &values {
            for &g in &values {
                for &b in &values {
                    palette.push(format!("{:02x}{:02x}{:02x}", r, g, b));
                }
            }
        }
        for i in 0..24u8 {
            let v = 8 + i as u16 * 10;
            palette.push(format!("{:02x}{:02x}{:02x}", v, v, v));
        }
        palette
    })
}

pub fn fg_bg_256() -> &'static [String] {
    get_256_palette()
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Color {
    Default,
    Named(&'static str),
    Indexed(u8),
    Rgb(String),
}

impl Default for Color {
    fn default() -> Self { Color::Default }
}

impl Color {
    pub fn is_default(&self) -> bool { matches!(self, Color::Default) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_text_attr_setters() {
        assert_eq!(text_attr(1), Some(TextAttr::SetBold));
        assert_eq!(text_attr(3), Some(TextAttr::SetItalics));
        assert_eq!(text_attr(4), Some(TextAttr::SetUnderline));
        assert_eq!(text_attr(5), Some(TextAttr::SetBlink));
        assert_eq!(text_attr(7), Some(TextAttr::SetReverse));
        assert_eq!(text_attr(9), Some(TextAttr::SetStrikethrough));
    }

    #[test]
    fn test_text_attr_resetters() {
        assert_eq!(text_attr(22), Some(TextAttr::ResetBold));
        assert_eq!(text_attr(23), Some(TextAttr::ResetItalics));
        assert_eq!(text_attr(24), Some(TextAttr::ResetUnderline));
        assert_eq!(text_attr(25), Some(TextAttr::ResetBlink));
        assert_eq!(text_attr(27), Some(TextAttr::ResetReverse));
        assert_eq!(text_attr(29), Some(TextAttr::ResetStrikethrough));
    }

    #[test]
    fn test_text_attr_unknown() {
        assert_eq!(text_attr(0), None);
        assert_eq!(text_attr(2), None);
        assert_eq!(text_attr(6), None);
        assert_eq!(text_attr(8), None);
        assert_eq!(text_attr(10), None);
        assert_eq!(text_attr(-1), None);
        assert_eq!(text_attr(100), None);
    }

    #[test]
    fn test_text_attr_is_set() {
        assert!(TextAttr::SetBold.is_set());
        assert!(TextAttr::SetItalics.is_set());
        assert!(TextAttr::SetUnderline.is_set());
        assert!(TextAttr::SetBlink.is_set());
        assert!(TextAttr::SetReverse.is_set());
        assert!(TextAttr::SetStrikethrough.is_set());
    }

    #[test]
    fn test_text_attr_is_reset() {
        assert!(!TextAttr::ResetBold.is_set());
        assert!(!TextAttr::ResetItalics.is_set());
        assert!(!TextAttr::ResetUnderline.is_set());
        assert!(!TextAttr::ResetBlink.is_set());
        assert!(!TextAttr::ResetReverse.is_set());
        assert!(!TextAttr::ResetStrikethrough.is_set());
    }

    #[test]
    fn test_fg_ansi() {
        assert_eq!(fg_ansi(30), Some("black"));
        assert_eq!(fg_ansi(31), Some("red"));
        assert_eq!(fg_ansi(32), Some("green"));
        assert_eq!(fg_ansi(33), Some("brown"));
        assert_eq!(fg_ansi(34), Some("blue"));
        assert_eq!(fg_ansi(35), Some("magenta"));
        assert_eq!(fg_ansi(36), Some("cyan"));
        assert_eq!(fg_ansi(37), Some("white"));
        assert_eq!(fg_ansi(39), Some("default"));
    }

    #[test]
    fn test_fg_ansi_unknown() {
        assert_eq!(fg_ansi(0), None);
        assert_eq!(fg_ansi(1), None);
        assert_eq!(fg_ansi(29), None);
        assert_eq!(fg_ansi(40), None);
        assert_eq!(fg_ansi(100), None);
    }

    #[test]
    fn test_fg_aixterm() {
        assert_eq!(fg_aixterm(90), Some("brightblack"));
        assert_eq!(fg_aixterm(91), Some("brightred"));
        assert_eq!(fg_aixterm(92), Some("brightgreen"));
        assert_eq!(fg_aixterm(93), Some("brightbrown"));
        assert_eq!(fg_aixterm(94), Some("brightblue"));
        assert_eq!(fg_aixterm(95), Some("brightmagenta"));
        assert_eq!(fg_aixterm(96), Some("brightcyan"));
        assert_eq!(fg_aixterm(97), Some("brightwhite"));
    }

    #[test]
    fn test_fg_aixterm_unknown() {
        assert_eq!(fg_aixterm(30), None);
        assert_eq!(fg_aixterm(0), None);
        assert_eq!(fg_aixterm(99), None);
    }

    #[test]
    fn test_bg_ansi() {
        assert_eq!(bg_ansi(40), Some("black"));
        assert_eq!(bg_ansi(41), Some("red"));
        assert_eq!(bg_ansi(42), Some("green"));
        assert_eq!(bg_ansi(43), Some("brown"));
        assert_eq!(bg_ansi(44), Some("blue"));
        assert_eq!(bg_ansi(45), Some("magenta"));
        assert_eq!(bg_ansi(46), Some("cyan"));
        assert_eq!(bg_ansi(47), Some("white"));
        assert_eq!(bg_ansi(49), Some("default"));
    }

    #[test]
    fn test_bg_ansi_unknown() {
        assert_eq!(bg_ansi(0), None);
        assert_eq!(bg_ansi(30), None);
        assert_eq!(bg_ansi(100), None);
    }

    #[test]
    fn test_bg_aixterm() {
        assert_eq!(bg_aixterm(100), Some("brightblack"));
        assert_eq!(bg_aixterm(101), Some("brightred"));
        assert_eq!(bg_aixterm(102), Some("brightgreen"));
        assert_eq!(bg_aixterm(103), Some("brightbrown"));
        assert_eq!(bg_aixterm(104), Some("brightblue"));
        assert_eq!(bg_aixterm(105), Some("brightmagenta"));
        assert_eq!(bg_aixterm(106), Some("brightcyan"));
        assert_eq!(bg_aixterm(107), Some("brightwhite"));
    }

    #[test]
    fn test_bg_aixterm_unknown() {
        assert_eq!(bg_aixterm(40), None);
        assert_eq!(bg_aixterm(0), None);
        assert_eq!(bg_aixterm(108), None);
    }

    #[test]
    fn test_fg_bg_256_constants() {
        assert_eq!(FG_256, 38);
        assert_eq!(BG_256, 48);
    }

    #[test]
    fn test_fg_bg_256_palette() {
        let palette = fg_bg_256();
        assert_eq!(palette.len(), 256);
        // First 16 are standard colors
        assert_eq!(palette[0], "000000");
        assert_eq!(palette[1], "cd0000");
        assert_eq!(palette[7], "e5e5e5");
        assert_eq!(palette[8], "7f7f7f");
        assert_eq!(palette[15], "ffffff");
    }

    #[test]
    fn test_color_default() {
        let c = Color::default();
        assert!(c.is_default());
    }

    #[test]
    fn test_color_named() {
        let c = Color::Named("red");
        assert!(!c.is_default());
    }

    #[test]
    fn test_color_indexed() {
        let c = Color::Indexed(42);
        assert!(!c.is_default());
    }

    #[test]
    fn test_color_rgb() {
        let c = Color::Rgb("ff0000".to_string());
        assert!(!c.is_default());
    }

    #[test]
    fn test_color_equality() {
        let c1 = Color::Named("red");
        let c2 = Color::Named("red");
        let c3 = Color::Named("blue");
        assert_eq!(c1, c2);
        assert_ne!(c1, c3);
    }

    #[test]
    fn test_color_hash() {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let c1 = Color::Named("red");
        let c2 = Color::Named("red");
        let mut h1: Box<dyn Hasher> = Box::new(DefaultHasher::new());
        let mut h2: Box<dyn Hasher> = Box::new(DefaultHasher::new());
        c1.hash(&mut h1);
        c2.hash(&mut h2);
        assert_eq!(c1, c2);
    }

    #[test]
    fn test_text_attr_debug() {
        let attr = TextAttr::SetBold;
        let s = format!("{:?}", attr);
        assert!(s.contains("SetBold"));
    }

    #[test]
    fn test_color_clone() {
        let c1 = Color::Rgb("ff0000".to_string());
        let c2 = c1.clone();
        assert_eq!(c1, c2);
    }

    #[test]
    fn test_text_attr_clone() {
        let a1 = TextAttr::SetBold;
        let a2 = a1.clone();
        assert_eq!(a1, a2);
    }
}
