//! Ordered terminal event pipeline (drained per frame).
//!
//! Low-frequency signals (bell, title, cwd, alt-screen switches, scrollback
//! growth) accumulate here in parser order while high-frequency row changes
//! go to the dirty-row set. Consumers drain both once per frame:
//! `take_dirty_rows()` for *what to repaint*, `take_events()` for *what
//! happened*. Events are a log, not a set — ordering is the point — while
//! `take_bell()` stays as the coalescing one-bit shortcut for consumers that
//! only care about the bell. The two bell paths are unified: draining either
//! one consumes the pending bell for both (see `Screen::take_bell` and
//! `Screen::take_events`).

/// A low-frequency terminal signal, in the order the parser produced it.
///
/// `ScrollbackGrew` is the exception: it is a per-drain summary appended
/// trailing by `HistoryScreen::take_events`, since scrollback arrivals are
/// collected after each feed rather than during parsing.
///
/// **Bounded log.** The event vec is capped at [`MAX_EVENTS`] entries with
/// drop-oldest semantics (enforced by `Screen::push_event`). A consumer that
/// drains per frame never holds more than a handful of events — even a 64 KiB
/// feed of max-size OSC titles yields only dozens — while a consumer that
/// never drains (e.g. one that only calls `styled_range`) leaks at most ~1 MiB
/// instead of growing without bound, one `TitleChanged` per shell prompt.
/// Ordering among retained events is always parser order; only the oldest
/// prefix is ever discarded.
pub const MAX_EVENTS: usize = 1024;
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TermEvent {
    /// A lone BEL (0x07). OSC BEL terminators never produce this.
    Bell,
    /// OSC 0/2 window title.
    TitleChanged(String),
    /// OSC 0/1 icon name.
    IconChanged(String),
    /// OSC 7 / OSC 9;9 shell working directory.
    CwdChanged(String),
    /// Alternate-screen buffer entered (`true`) or exited (`false`).
    AltScreen { entered: bool },
    /// Lines that entered scrollback history since the last drain.
    ScrollbackGrew(u64),
}

impl TermEvent {
    /// Stable string tag for the Python `poll_events()` tuples, so the
    /// Python side needs no new enum class.
    pub fn tag(&self) -> &'static str {
        match self {
            TermEvent::Bell => "bell",
            TermEvent::TitleChanged(_) => "title",
            TermEvent::IconChanged(_) => "icon",
            TermEvent::CwdChanged(_) => "cwd",
            TermEvent::AltScreen { .. } => "altscreen",
            TermEvent::ScrollbackGrew(_) => "scrollback_grew",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tags_are_stable() {
        assert_eq!(TermEvent::Bell.tag(), "bell");
        assert_eq!(TermEvent::TitleChanged("t".into()).tag(), "title");
        assert_eq!(TermEvent::IconChanged("i".into()).tag(), "icon");
        assert_eq!(TermEvent::CwdChanged("/x".into()).tag(), "cwd");
        assert_eq!(TermEvent::AltScreen { entered: true }.tag(), "altscreen");
        assert_eq!(TermEvent::ScrollbackGrew(3).tag(), "scrollback_grew");
    }
}
