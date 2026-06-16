//! Escape sequence designators.

pub const RIS: char = 'c';
pub const IND: char = 'D';
pub const NEL: char = 'E';
pub const HTS: char = 'H';
pub const RI: char = 'M';
pub const DECSC: char = '7';
pub const DECRC: char = '8';
pub const DECALN: char = '8';
pub const ICH: char = '@';
pub const CUU: char = 'A';
pub const CUD: char = 'B';
pub const CUF: char = 'C';
pub const CUB: char = 'D';
pub const CNL: char = 'E';
pub const CPL: char = 'F';
pub const CHA: char = 'G';
pub const CUP: char = 'H';
pub const ED: char = 'J';
pub const EL: char = 'K';
pub const IL: char = 'L';
pub const DL: char = 'M';
pub const DCH: char = 'P';
pub const ECH: char = 'X';
pub const DA: char = 'c';
pub const DSR: char = 'n';
pub const DECSTBM: char = 'r';
pub const SGR: char = 'm';
pub const TBC: char = 'g';
pub const SM: char = 'h';
pub const RM: char = 'l';

#[inline] pub fn is_csi_final(c: char) -> bool { matches!(c as u32, 0x40..=0x7E) }
#[inline] pub fn is_csi_param(c: char) -> bool { matches!(c as u32, 0x30..=0x39) }
#[inline] pub fn is_csi_intermediate(c: char) -> bool { matches!(c as u32, 0x20..=0x2F) }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_escape_designators() {
        assert_eq!(RIS, 'c');
        assert_eq!(IND, 'D');
        assert_eq!(NEL, 'E');
        assert_eq!(HTS, 'H');
        assert_eq!(RI, 'M');
        assert_eq!(DECSC, '7');
        assert_eq!(DECRC, '8');
        assert_eq!(DECALN, '8');
        assert_eq!(ICH, '@');
        assert_eq!(CUU, 'A');
        assert_eq!(CUD, 'B');
        assert_eq!(CUF, 'C');
        assert_eq!(CUB, 'D');
        assert_eq!(CNL, 'E');
        assert_eq!(CPL, 'F');
        assert_eq!(CHA, 'G');
        assert_eq!(CUP, 'H');
        assert_eq!(ED, 'J');
        assert_eq!(EL, 'K');
        assert_eq!(IL, 'L');
        assert_eq!(DL, 'M');
        assert_eq!(DCH, 'P');
        assert_eq!(ECH, 'X');
        assert_eq!(DA, 'c');
        assert_eq!(DSR, 'n');
        assert_eq!(DECSTBM, 'r');
        assert_eq!(SGR, 'm');
        assert_eq!(TBC, 'g');
        assert_eq!(SM, 'h');
        assert_eq!(RM, 'l');
    }

    #[test]
    fn test_is_csi_final_basic() {
        assert!(is_csi_final('A'));  // CUU
        assert!(is_csi_final('m'));  // SGR
        assert!(is_csi_final('J'));  // ED
        assert!(is_csi_final('K'));  // EL
        assert!(is_csi_final('c'));  // DA
        assert!(is_csi_final('z'));
    }

    #[test]
    fn test_is_csi_final_boundary() {
        assert!(is_csi_final('@'));  // 0x40 lower bound
        assert!(is_csi_final('~'));  // 0x7E upper bound
        assert!(!is_csi_final(' '));  // 0x20 not in range
        assert!(!is_csi_final('0'));  // 0x30 not in range
        assert!(!is_csi_final('!'));  // 0x21 not in range
    }

    #[test]
    fn test_is_csi_param_basic() {
        assert!(is_csi_param('0'));
        assert!(is_csi_param('1'));
        assert!(is_csi_param('5'));
        assert!(is_csi_param('9'));
    }

    #[test]
    fn test_is_csi_param_boundary() {
        assert!(is_csi_param('0'));  // 0x30 lower bound
        assert!(is_csi_param('9'));  // 0x39 upper bound
        assert!(!is_csi_param(' '));  // 0x20 not in range
        assert!(!is_csi_param(':'));  // 0x3A not in range
    }

    #[test]
    fn test_is_csi_intermediate_basic() {
        assert!(is_csi_intermediate(' '));  // 0x20 lower bound
        assert!(is_csi_intermediate('/'));  // 0x2F upper bound
        assert!(is_csi_intermediate('#'));  // middle of range
    }

    #[test]
    fn test_is_csi_intermediate_boundary() {
        assert!(is_csi_intermediate(' '));  // 0x20 lower bound
        assert!(is_csi_intermediate('/'));  // 0x2F upper bound
        assert!(!is_csi_intermediate('0'));  // 0x30 not in range
        assert!(!is_csi_intermediate('@'));  // 0x40 not in range
    }

    #[test]
    fn test_is_csi_intermediate_all() {
        for i in 0x20u8..=0x2Fu8 {
            assert!(is_csi_intermediate(i as char));
        }
    }
}
