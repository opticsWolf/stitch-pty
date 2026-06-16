//! C0 and C1 control character constants.

pub const SP: char = ' ';
pub const NUL: char = '\x00';
pub const BEL: char = '\x07';
pub const BS: char = '\x08';
pub const HT: char = '\x09';
pub const LF: char = '\n';
pub const VT: char = '\x0b';
pub const FF: char = '\x0c';
pub const CR: char = '\r';
pub const SO: char = '\x0e';
pub const SI: char = '\x0f';
pub const CAN: char = '\x18';
pub const SUB: char = '\x1a';
pub const ESC: char = '\x1b';
pub const DEL: char = '\x7f';
pub const CSI_C0: &str = "\x1b[";
pub const CSI_C1: u8 = 0x9b;
pub const CSI: &str = CSI_C0;
pub const ST_C0: &str = "\x1b\\";
pub const ST_C1: u8 = 0x9c;
pub const ST: &str = ST_C0;
pub const OSC_C0: &str = "\x1b]";
pub const OSC_C1: u8 = 0x9d;
pub const OSC: &str = OSC_C0;

pub const C0: [char; 32] = [
    '\x00', '\x01', '\x02', '\x03', '\x04', '\x05', '\x06', '\x07',
    '\x08', '\x09', '\x0a', '\x0b', '\x0c', '\x0d', '\x0e', '\x0f',
    '\x10', '\x11', '\x12', '\x13', '\x14', '\x15', '\x16', '\x17',
    '\x18', '\x19', '\x1a', '\x1b', '\x1c', '\x1d', '\x1e', '\x1f',
];

pub const C1: [u8; 32] = [
    0x80, 0x81, 0x82, 0x83, 0x84, 0x85, 0x86, 0x87,
    0x88, 0x89, 0x8a, 0x8b, 0x8c, 0x8d, 0x8e, 0x8f,
    0x90, 0x91, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97,
    0x98, 0x99, 0x9a, 0x9b, 0x9c, 0x9d, 0x9e, 0x9f,
];

#[inline] pub fn is_c0(c: char) -> bool { (c as u32) < 0x20 }
#[inline] pub fn is_c1(c: char) -> bool { let code = c as u32; code >= 0x80 && code <= 0x9F }
#[inline] pub fn is_control(c: char) -> bool { is_c0(c) || is_c1(c) }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_c0_constants() {
        assert_eq!(NUL, '\x00');
        assert_eq!(BEL, '\x07');
        assert_eq!(BS, '\x08');
        assert_eq!(HT, '\x09');
        assert_eq!(LF, '\n');
        assert_eq!(VT, '\x0b');
        assert_eq!(FF, '\x0c');
        assert_eq!(CR, '\r');
        assert_eq!(SO, '\x0e');
        assert_eq!(SI, '\x0f');
        assert_eq!(CAN, '\x18');
        assert_eq!(SUB, '\x1a');
        assert_eq!(ESC, '\x1b');
        assert_eq!(DEL, '\x7f');
    }

    #[test]
    fn test_c1_constants() {
        assert_eq!(CSI_C1, 0x9b);
        assert_eq!(ST_C1, 0x9c);
        assert_eq!(OSC_C1, 0x9d);
    }

    #[test]
    fn test_escape_sequences_c0() {
        assert_eq!(CSI, "\x1b[");
        assert_eq!(ST, "\x1b\\");
        assert_eq!(OSC, "\x1b]");
        assert_eq!(CSI_C0, CSI);
        assert_eq!(ST_C0, ST);
        assert_eq!(OSC_C0, OSC);
    }

    #[test]
    fn test_c0_array() {
        assert_eq!(C0.len(), 32);
        assert_eq!(C0[0], '\x00');
        assert_eq!(C0[31], '\x1f');
    }

    #[test]
    fn test_c1_array() {
        assert_eq!(C1.len(), 32);
        assert_eq!(C1[0], 0x80);
        assert_eq!(C1[31], 0x9f);
    }

    #[test]
    fn test_is_c0_basic() {
        assert!(is_c0('\x00'));
        assert!(is_c0('\x07'));
        assert!(is_c0('\x1f'));
    }

    #[test]
    fn test_is_c0_boundary() {
        assert!(!is_c0('\x20'));  // space is not control
        assert!(!is_c0('a'));
        assert!(!is_c0('Z'));
        assert!(!is_c0('\x7f'));
    }

    #[test]
    fn test_is_c1_basic() {
        assert!(is_c1('\u{80}'));
        assert!(is_c1('\u{90}'));
        assert!(is_c1('\u{9F}'));
    }

    #[test]
    fn test_is_c1_boundary() {
        assert!(!is_c1('\x7f'));
        assert!(!is_c1('\u{A0}'));
        assert!(!is_c1('a'));
    }

    #[test]
    fn test_is_control_c0() {
        assert!(is_control('\x00'));
        assert!(is_control('\x1f'));
        assert!(is_control(ESC));
    }

    #[test]
    fn test_is_control_c1() {
        assert!(is_control('\u{80}'));
        assert!(is_control('\u{9F}'));
    }

    #[test]
    fn test_is_control_non_control() {
        assert!(!is_control(' '));
        assert!(!is_control('a'));
        assert!(!is_control('Z'));
        assert!(!is_control('\x7f'));
        assert!(!is_control('\u{A0}'));
    }
}
