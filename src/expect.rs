//! Byte-oriented expect-style matching for [`crate::PtySession`].
//!
//! The Python API has always offered `expect()`. This module gives Rust
//! callers the same byte-first primitive without requiring valid UTF-8 or
//! line boundaries. Patterns are checked in list order after every read, so a
//! match can span arbitrary read chunks.
//!
//! The accumuated buffer is local to one `expect()` call; the returned
//! `after` bytes are the unconsumed tail after the match. A caller that wants
//! to keep reading beyond the match should inspect `after` or issue a new
//! `expect()` (bytes already consumed from the PTY are not pushed back).

use crate::errors::PtyErrorKind;
use crate::session::PtySession;
use std::time::{Duration, Instant};

/// Default cap for one `expect()` call's accumulated output.
pub const DEFAULT_MAX_EXPECT_BUFFER: usize = 4 * 1024 * 1024;

/// A pattern accepted by [`PtySession::expect`].
#[derive(Clone, Debug)]
pub enum ExpectPattern {
    /// Literal bytes.
    Bytes(Vec<u8>),
    /// UTF-8 string, matched as its encoded bytes.
    Str(String),
    /// Byte-oriented regular expression (requires the `regex` feature).
    #[cfg(feature = "regex")]
    Regex(regex::bytes::Regex),
}

impl ExpectPattern {
    pub fn bytes(bytes: impl Into<Vec<u8>>) -> Self {
        Self::Bytes(bytes.into())
    }

    pub fn string(text: impl Into<String>) -> Self {
        Self::Str(text.into())
    }

    /// Compile a byte-oriented regex. Requires the `regex` feature.
    #[cfg(feature = "regex")]
    pub fn regex(pattern: &str) -> Result<Self, regex::Error> {
        Ok(Self::Regex(regex::bytes::Regex::new(pattern)?))
    }

    /// Find the first match in `buffer`.
    pub fn find(&self, buffer: &[u8]) -> Option<std::ops::Range<usize>> {
        match self {
            Self::Bytes(needle) => find_subslice(buffer, needle),
            Self::Str(text) => find_subslice(buffer, text.as_bytes()),
            #[cfg(feature = "regex")]
            Self::Regex(regex) => regex.find(buffer).map(|m| m.range()),
        }
    }
}

impl From<&[u8]> for ExpectPattern {
    fn from(value: &[u8]) -> Self {
        Self::Bytes(value.to_vec())
    }
}

impl From<Vec<u8>> for ExpectPattern {
    fn from(value: Vec<u8>) -> Self {
        Self::Bytes(value)
    }
}

impl From<&str> for ExpectPattern {
    fn from(value: &str) -> Self {
        Self::Str(value.to_string())
    }
}

impl From<String> for ExpectPattern {
    fn from(value: String) -> Self {
        Self::Str(value)
    }
}

#[cfg(feature = "regex")]
impl From<regex::bytes::Regex> for ExpectPattern {
    fn from(value: regex::bytes::Regex) -> Self {
        Self::Regex(value)
    }
}

/// Options for [`PtySession::expect_with_options`].
#[derive(Clone, Copy, Debug)]
pub struct ExpectOptions {
    /// Maximum accumulated bytes. `0` means unlimited.
    pub max_buffer: usize,
}

impl Default for ExpectOptions {
    fn default() -> Self {
        Self {
            max_buffer: DEFAULT_MAX_EXPECT_BUFFER,
        }
    }
}

/// A successful match.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExpectResult {
    /// Index into the pattern list (pattern order wins, mirroring Python).
    pub index: usize,
    /// The matched bytes.
    pub matched: Vec<u8>,
    /// Bytes before the match in this call's buffer.
    pub before: Vec<u8>,
    /// Bytes after the match in this call's buffer (the retained tail).
    pub after: Vec<u8>,
    /// All bytes read during this call, through the matching chunk.
    pub buffer: Vec<u8>,
    /// Byte range of the match in `buffer`.
    pub span: std::ops::Range<usize>,
}

/// Failure from [`PtySession::expect`].
#[derive(Debug)]
pub enum ExpectError {
    /// No pattern matched before the caller's timeout.
    Timeout { buffer: Vec<u8> },
    /// The child closed before a pattern matched.
    Eof { buffer: Vec<u8> },
    /// The accumulated buffer exceeded [`ExpectOptions::max_buffer`].
    BufferLimit { buffer: Vec<u8>, limit: usize },
    /// An empty pattern list was supplied.
    NoPatterns,
    /// A PTY I/O error other than timeout/EOF.
    Io { message: String, buffer: Vec<u8> },
}

impl std::fmt::Display for ExpectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Timeout { buffer } => {
                write!(f, "expect timed out after {} buffered bytes", buffer.len())
            }
            Self::Eof { buffer } => {
                write!(
                    f,
                    "expect reached EOF after {} buffered bytes",
                    buffer.len()
                )
            }
            Self::BufferLimit { limit, buffer } => write!(
                f,
                "expect buffer exceeded {limit} bytes ({} buffered)",
                buffer.len()
            ),
            Self::NoPatterns => f.write_str("expect requires at least one pattern"),
            Self::Io { message, buffer } => {
                write!(
                    f,
                    "expect I/O error after {} bytes: {message}",
                    buffer.len()
                )
            }
        }
    }
}

impl std::error::Error for ExpectError {}

impl PtySession {
    /// Read until one of `patterns` appears or `timeout` elapses.
    ///
    /// Patterns are checked in list order after each read; a literal/regex
    /// match may span read chunks and does not require UTF-8 or line
    /// boundaries. The returned [`ExpectResult::after`] is the unconsumed tail
    /// of the bytes read during this call. On timeout/EOF the buffered bytes
    /// are attached to the error; the session is not killed.
    pub async fn expect(
        &self,
        patterns: &[ExpectPattern],
        timeout: Duration,
    ) -> Result<ExpectResult, ExpectError> {
        self.expect_with_options(patterns, timeout, ExpectOptions::default())
            .await
    }

    /// Like [`Self::expect`] with an explicit buffer cap.
    pub async fn expect_with_options(
        &self,
        patterns: &[ExpectPattern],
        timeout: Duration,
        options: ExpectOptions,
    ) -> Result<ExpectResult, ExpectError> {
        if patterns.is_empty() {
            return Err(ExpectError::NoPatterns);
        }
        let deadline = Instant::now() + timeout;
        let mut buffer = Vec::new();
        let mut chunk = [0u8; 4096];

        loop {
            for (index, pattern) in patterns.iter().enumerate() {
                if let Some(span) = pattern.find(&buffer) {
                    let matched = buffer[span.clone()].to_vec();
                    let before = buffer[..span.start].to_vec();
                    let after = buffer[span.end..].to_vec();
                    return Ok(ExpectResult {
                        index,
                        matched,
                        before,
                        after,
                        buffer,
                        span,
                    });
                }
            }

            if options.max_buffer != 0 && buffer.len() > options.max_buffer {
                return Err(ExpectError::BufferLimit {
                    buffer,
                    limit: options.max_buffer,
                });
            }

            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(ExpectError::Timeout { buffer });
            }

            match self.read_timeout(&mut chunk, remaining).await {
                Ok(0) => return Err(ExpectError::Eof { buffer }),
                Ok(n) => buffer.extend_from_slice(&chunk[..n]),
                Err(PtyErrorKind::Timeout(_)) => return Err(ExpectError::Timeout { buffer }),
                Err(other) => {
                    return Err(ExpectError::Io {
                        message: other.to_string(),
                        buffer,
                    });
                }
            }
        }
    }
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<std::ops::Range<usize>> {
    if needle.is_empty() {
        return Some(0..0);
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
        .map(|start| start..start + needle.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_literal_across_buffer_bytes() {
        let pattern = ExpectPattern::from("needle");
        assert_eq!(pattern.find(b"xxneedlexx"), Some(2..8));
        assert_eq!(pattern.find(b"no match"), None);
        assert_eq!(ExpectPattern::from(&b"ab"[..]).find(b"xxab"), Some(2..4));
    }

    #[test]
    fn empty_pattern_matches_at_start() {
        assert_eq!(ExpectPattern::from(&b""[..]).find(b"abc"), Some(0..0));
    }

    #[cfg(feature = "regex")]
    #[test]
    fn regex_matches_arbitrary_bytes() {
        let pattern = ExpectPattern::regex(r"(?-u:\xff)+").unwrap();
        assert_eq!(pattern.find(&[0x00, 0xff, 0xff, 0x01]), Some(1..3));
    }
}
