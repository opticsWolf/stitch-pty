//! Terminal mode constants and mode management.

use bitflags::bitflags;
use std::collections::HashSet;

// Public ANSI Modes
pub const IRM: u16 = 4;
pub const LNM: u16 = 20;

// Private DEC Modes
pub const DECCKM: u16 = 1;
pub const DECCOLM: u16 = 3;
pub const DECOM: u16 = 6;
pub const DECAWM: u16 = 7;
pub const DECTCEM: u16 = 25;
pub const DECSCNM: u16 = 5;
pub const X10_MOUSE: u16 = 1000;
pub const NORMAL_MOUSE: u16 = 1002;
pub const ANY_EVENT_MOUSE: u16 = 1003;
pub const FOCUS_EVENT: u16 = 1004;
pub const SGR_MOUSE: u16 = 1006;
pub const BRACKETED_PASTE: u16 = 2004;

// Alternate screen buffer (tracked as a flag only — no buffer swap is performed)
pub const ALT_SCREEN_1049: u16 = 1049;
pub const ALT_SCREEN_1047: u16 = 1047;
pub const ALT_SCREEN_47: u16 = 47;

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
    pub struct PublicModes: u16 {
        const IRM = 1 << 2;
        const LNM = 1 << 4;
    }
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
    pub struct PrivateModes: u32 {
        const DECCKM = 1 << 1;
        const DECCOLM = 1 << 3;
        const DECOM = 1 << 6;
        const DECAWM = 1 << 7;
        const DECTCEM = 1 << 25;
        const DECSCNM = 1 << 5;
    }
}

#[derive(Debug, Clone, Default)]
pub struct Modes {
    pub public: PublicModes,
    pub private: PrivateModes,
    pub extended: HashSet<u16>,
}

impl Modes {
    pub fn new() -> Self {
        Self {
            public: PublicModes::LNM,
            private: PrivateModes::DECAWM | PrivateModes::DECTCEM,
            extended: HashSet::new(),
        }
    }

    pub fn has_public(&self, mode: u16) -> bool {
        match mode {
            IRM => self.public.contains(PublicModes::IRM),
            LNM => self.public.contains(PublicModes::LNM),
            _ => false,
        }
    }

    pub fn has_private(&self, mode: u16) -> bool {
        match mode {
            DECCKM => self.private.contains(PrivateModes::DECCKM),
            DECCOLM => self.private.contains(PrivateModes::DECCOLM),
            DECOM => self.private.contains(PrivateModes::DECOM),
            DECAWM => self.private.contains(PrivateModes::DECAWM),
            DECTCEM => self.private.contains(PrivateModes::DECTCEM),
            DECSCNM => self.private.contains(PrivateModes::DECSCNM),
            _ => self.extended.contains(&mode),
        }
    }

    pub fn set_public(&mut self, mode: u16) {
        match mode { IRM => self.public.insert(PublicModes::IRM), LNM => self.public.insert(PublicModes::LNM), _ => {} }
    }

    pub fn clear_public(&mut self, mode: u16) {
        match mode { IRM => self.public.remove(PublicModes::IRM), LNM => self.public.remove(PublicModes::LNM), _ => {} }
    }

    pub fn set_private(&mut self, mode: u16) {
        match mode {
            DECCKM => self.private.insert(PrivateModes::DECCKM),
            DECCOLM => self.private.insert(PrivateModes::DECCOLM),
            DECOM => self.private.insert(PrivateModes::DECOM),
            DECAWM => self.private.insert(PrivateModes::DECAWM),
            DECTCEM => self.private.insert(PrivateModes::DECTCEM),
            DECSCNM => self.private.insert(PrivateModes::DECSCNM),
            _ => { self.extended.insert(mode); }
        }
    }

    pub fn clear_private(&mut self, mode: u16) {
        match mode {
            DECCKM => self.private.remove(PrivateModes::DECCKM),
            DECCOLM => self.private.remove(PrivateModes::DECCOLM),
            DECOM => self.private.remove(PrivateModes::DECOM),
            DECAWM => self.private.remove(PrivateModes::DECAWM),
            DECTCEM => self.private.remove(PrivateModes::DECTCEM),
            DECSCNM => self.private.remove(PrivateModes::DECSCNM),
            _ => { self.extended.remove(&mode); }
        }
    }

    // ── Convenience queries (used by the Python binding) ────────────

    /// Highest-precedence active mouse tracking mode (0 = none).
    pub fn mouse_protocol(&self) -> u16 {
        if self.has_private(ANY_EVENT_MOUSE) {
            ANY_EVENT_MOUSE
        } else if self.has_private(NORMAL_MOUSE) {
            NORMAL_MOUSE
        } else if self.has_private(X10_MOUSE) {
            X10_MOUSE
        } else {
            0
        }
    }

    /// Whether SGR (1006) mouse encoding is active.
    pub fn sgr_mouse(&self) -> bool {
        self.has_private(SGR_MOUSE)
    }

    /// Whether any alternate-screen mode is active.
    ///
    /// Note: this is a flag only — the screen buffer is not swapped or
    /// restored. It reports that the application requested the alternate
    /// screen, which is what a front-end needs to decide e.g. wheel behavior.
    pub fn is_alt_screen(&self) -> bool {
        self.has_private(ALT_SCREEN_1049)
            || self.has_private(ALT_SCREEN_1047)
            || self.has_private(ALT_SCREEN_47)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_modes() {
        let modes = Modes::new();
        assert!(modes.has_public(LNM));
        assert!(!modes.has_public(IRM));
        assert!(modes.has_private(DECAWM));
        assert!(modes.has_private(DECTCEM));
        assert!(!modes.has_private(DECCOLM));
    }

    #[test]
    fn test_set_clear_public() {
        let mut modes = Modes::new();
        modes.set_public(IRM);
        assert!(modes.has_public(IRM));
        modes.clear_public(IRM);
        assert!(!modes.has_public(IRM));
        assert!(modes.has_public(LNM));
    }

    #[test]
    fn test_set_clear_private() {
        let mut modes = Modes::new();
        modes.set_private(DECCOLM);
        assert!(modes.has_private(DECCOLM));
        modes.clear_private(DECCOLM);
        assert!(!modes.has_private(DECCOLM));
    }

    #[test]
    fn test_mouse_modes() {
        let mut modes = Modes::new();
        modes.set_private(X10_MOUSE);
        assert!(modes.has_private(X10_MOUSE));
        modes.set_private(NORMAL_MOUSE);
        assert!(modes.has_private(NORMAL_MOUSE));
        modes.set_private(ANY_EVENT_MOUSE);
        assert!(modes.has_private(ANY_EVENT_MOUSE));
        modes.set_private(FOCUS_EVENT);
        assert!(modes.has_private(FOCUS_EVENT));
        modes.set_private(BRACKETED_PASTE);
        assert!(modes.has_private(BRACKETED_PASTE));

        modes.clear_private(X10_MOUSE);
        assert!(!modes.has_private(X10_MOUSE));
        // Others should still be set
        assert!(modes.has_private(NORMAL_MOUSE));
        assert!(modes.has_private(BRACKETED_PASTE));
    }

    #[test]
    fn test_mode_query_helpers() {
        let mut modes = Modes::new();
        assert_eq!(modes.mouse_protocol(), 0);
        assert!(!modes.sgr_mouse());
        assert!(!modes.is_alt_screen());

        modes.set_private(X10_MOUSE);
        assert_eq!(modes.mouse_protocol(), X10_MOUSE);
        modes.set_private(ANY_EVENT_MOUSE);
        assert_eq!(modes.mouse_protocol(), ANY_EVENT_MOUSE); // highest precedence

        modes.set_private(SGR_MOUSE);
        assert!(modes.sgr_mouse());

        modes.set_private(ALT_SCREEN_1049);
        assert!(modes.is_alt_screen());
        modes.clear_private(ALT_SCREEN_1049);
        assert!(!modes.is_alt_screen());
    }

    #[test]
    fn test_unknown_mode_noop() {
        let mut modes = Modes::new();
        modes.set_public(9999);
        modes.set_private(9999);
        modes.clear_public(9999);
        modes.clear_private(9999);
        assert!(modes.has_public(LNM));
        assert!(modes.has_private(DECAWM));
    }

    #[test]
    fn test_mode_constants() {
        assert_eq!(IRM, 4);
        assert_eq!(LNM, 20);
        assert_eq!(DECCOLM, 3);
        assert_eq!(DECOM, 6);
        assert_eq!(DECAWM, 7);
        assert_eq!(DECTCEM, 25);
        assert_eq!(DECSCNM, 5);
    }
}
