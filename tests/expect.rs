//! Integration tests for byte-oriented Rust `PtySession::expect`.

use std::time::Duration;
use stitch_pty::{ExpectError, ExpectPattern, PtySession, Winsize};

fn size() -> Option<Winsize> {
    Some(Winsize::new(24, 80, 0, 0))
}

fn shell(command: &str) -> (&'static str, Vec<String>) {
    if cfg!(windows) {
        ("cmd.exe", vec!["/C".into(), command.into()])
    } else {
        ("sh", vec!["-c".into(), command.into()])
    }
}

fn quiet_long_running() -> (&'static str, Vec<String>) {
    if cfg!(windows) {
        (
            "cmd.exe",
            vec!["/C".into(), "ping -n 30 127.0.0.1 > NUL".into()],
        )
    } else {
        ("sh", vec!["-c".into(), "sleep 30".into()])
    }
}

#[cfg(unix)]
#[tokio::test]
async fn matches_across_read_chunks() {
    let (program, args) = shell("printf part1; sleep 0.2; printf part2");
    let session = PtySession::spawn(program, &args, size(), None)
        .await
        .expect("spawn");
    let result = session
        .expect(&[ExpectPattern::from("part1part2")], Duration::from_secs(5))
        .await
        .expect("expected split match");
    assert_eq!(result.index, 0);
    assert_eq!(result.matched, b"part1part2");
    assert!(result.buffer.starts_with(b"part1part2"));
    let _ = session.kill();
}

#[tokio::test]
async fn multiple_patterns_use_input_order() {
    let (program, args) = shell("echo foo bar");
    let session = PtySession::spawn(program, &args, size(), None)
        .await
        .expect("spawn");
    let result = session
        .expect(
            &[ExpectPattern::from("bar"), ExpectPattern::from("foo")],
            Duration::from_secs(5),
        )
        .await
        .expect("expected match");
    assert_eq!(result.index, 0, "pattern order should win");
    assert_eq!(result.matched, b"bar");
    assert!(result.before.windows(3).any(|w| w == b"foo"));
    let _ = session.kill();
}

#[tokio::test]
async fn timeout_returns_buffer_without_killing_session() {
    let (program, args) = quiet_long_running();
    let session = PtySession::spawn(program, &args, size(), None)
        .await
        .expect("spawn");
    let error = session
        .expect(
            &[ExpectPattern::from("never-appears")],
            Duration::from_millis(250),
        )
        .await
        .expect_err("expected timeout");
    match error {
        ExpectError::Timeout { buffer } => {
            // ConPTY/POSIX startup may emit bytes, but the point is that the
            // buffer is preserved and the session remains usable.
            let _ = buffer;
        }
        other => panic!("expected timeout, got {other:?}"),
    }
    assert!(session.is_alive(), "timeout must not kill the child");
    let _ = session.kill();
}

#[cfg(unix)]
#[tokio::test]
async fn matches_non_utf8_bytes() {
    let (program, args) = shell(r"printf '\377\376token'");
    let session = PtySession::spawn(program, &args, size(), None)
        .await
        .expect("spawn");
    let result = session
        .expect(
            &[ExpectPattern::from(&b"\xff\xfe"[..])],
            Duration::from_secs(5),
        )
        .await
        .expect("expected non-UTF-8 match");
    assert_eq!(result.matched, b"\xff\xfe");
    assert!(result.after.starts_with(b"token"));
    let _ = session.kill();
}

#[cfg(all(unix, feature = "regex"))]
#[tokio::test]
async fn regex_patterns_are_optional_and_byte_oriented() {
    let (program, args) = shell("printf 'abc123'");
    let session = PtySession::spawn(program, &args, size(), None)
        .await
        .expect("spawn");
    let result = session
        .expect(
            &[ExpectPattern::regex(r"[0-9]+").unwrap()],
            Duration::from_secs(5),
        )
        .await
        .expect("expected regex match");
    assert_eq!(result.matched, b"123");
    let _ = session.kill();
}
