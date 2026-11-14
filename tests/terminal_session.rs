//! Integration tests for the Rust `TerminalSession` (PTY + owned screen).

use std::time::Duration;
use stitch_pty::{TerminalSession, Winsize};

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

async fn read_until_contains(
    session: &mut TerminalSession,
    needle: &str,
    timeout: Duration,
) -> String {
    let deadline = std::time::Instant::now() + timeout;
    let mut out = String::new();
    let mut buf = [0u8; 4096];
    while std::time::Instant::now() < deadline {
        match session
            .read_timeout(&mut buf, Duration::from_millis(250))
            .await
        {
            Ok(0) => break,
            Ok(n) => {
                out.push_str(&String::from_utf8_lossy(&buf[..n]));
                if out.contains(needle) {
                    break;
                }
            }
            Err(stitch_pty::errors::PtyErrorKind::Timeout(_)) => continue,
            Err(err) => panic!("unexpected read error: {err:?}"),
        }
    }
    out
}

#[tokio::test]
async fn read_feeds_screen_and_exposes_dirty_rows() {
    let (program, args) = shell("echo terminal-session-ok");
    let mut session = TerminalSession::spawn(program, &args, size(), None, 100)
        .await
        .expect("spawn");

    let text =
        read_until_contains(&mut session, "terminal-session-ok", Duration::from_secs(5)).await;
    assert!(text.contains("terminal-session-ok"), "got {text:?}");
    assert!(
        session
            .display()
            .iter()
            .any(|line| line.contains("terminal-session-ok")),
        "screen did not receive the output: {:?}",
        session.display()
    );
    assert!(!session.take_dirty_rows().is_empty());
    assert!(session.take_dirty_rows().is_empty());
    assert!(session.has_fed());
    assert!(session.wait().await.expect("status").success());
}

#[tokio::test]
async fn resize_keeps_pty_and_screen_in_sync() {
    let (program, args) = quiet_long_running();
    let mut session = TerminalSession::spawn(program, &args, size(), None, 100)
        .await
        .expect("spawn");

    session.resize(30, 100).expect("resize");
    assert_eq!(session.screen().lines(), 30);
    assert_eq!(session.screen().columns(), 100);
    let winsize = session.get_winsize().expect("winsize");
    assert_eq!((winsize.rows, winsize.cols), (30, 100));

    let full = Winsize::new(35, 110, 0, 0);
    session.set_winsize(full).expect("set_winsize");
    assert_eq!(session.screen().lines(), 35);
    assert_eq!(session.screen().columns(), 110);
    session.kill().expect("kill");
}

#[cfg(unix)]
#[tokio::test]
async fn scrollback_is_owned_by_terminal_session() {
    let (program, args) = shell("i=1; while [ $i -le 40 ]; do echo line-$i; i=$((i+1)); done");
    let mut session = TerminalSession::spawn(program, &args, size(), None, 100)
        .await
        .expect("spawn");

    let mut buf = [0u8; 4096];
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        match session
            .read_timeout(&mut buf, Duration::from_millis(250))
            .await
        {
            Ok(0) => break,
            Ok(_) => continue,
            Err(stitch_pty::errors::PtyErrorKind::Timeout(_)) => continue,
            Err(err) => panic!("unexpected read error: {err:?}"),
        }
    }
    assert!(
        session.screen().history_size() > 0,
        "expected scrollback, got {:?}",
        session.full_display()
    );
    session.wait().await.expect("status");
}
