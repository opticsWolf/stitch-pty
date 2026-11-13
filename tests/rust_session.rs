//! Integration tests for the high-level Rust [`stitch_pty::PtySession`] API.
//!
//! These run the real platform backend (openpty/fork on Unix, ConPTY on
//! Windows); CI executes them on all three platforms.

use std::time::Duration;
use stitch_pty::{PtySession, Winsize};

fn size() -> Option<Winsize> {
    Some(Winsize::new(24, 80, 0, 0))
}

/// `(program, args)` for a one-shot shell command.
fn shell(command: &str) -> (&'static str, Vec<String>) {
    if cfg!(windows) {
        ("cmd.exe", vec!["/C".into(), command.into()])
    } else {
        ("sh", vec!["-c".into(), command.into()])
    }
}

/// `(program, args)` for a long-running child that produces no output.
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

/// `(program, args)` for an interactive shell.
fn interactive_shell() -> (&'static str, Vec<String>) {
    if cfg!(windows) {
        ("cmd.exe", vec![])
    } else {
        ("sh", vec!["-i".into()])
    }
}

async fn drain_to_eof(session: &PtySession) -> Vec<u8> {
    let mut out = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        match session.read_timeout(&mut buf, Duration::from_secs(5)).await {
            Ok(0) => break,
            Ok(n) => out.extend_from_slice(&buf[..n]),
            Err(_) => break,
        }
    }
    out
}

#[tokio::test]
async fn spawn_read_wait_reports_exit_status() {
    let (program, args) = shell("echo stitch-session-ok");
    let session = PtySession::spawn(program, &args, size(), None)
        .await
        .expect("spawn");

    let out = drain_to_eof(&session).await;
    let text = String::from_utf8_lossy(&out);
    assert!(
        text.contains("stitch-session-ok"),
        "child output missing from PTY; got {text:?}"
    );

    let status = session.wait().await.expect("child should be reaped");
    assert!(status.success(), "unexpected exit status: {status}");
}

#[tokio::test]
async fn empty_env_inherits_parent_environment() {
    // Regression test: on Unix, `spawn_platform(..., env = &[])` used to pass
    // an empty envp, so the child lost PATH/HOME/TERM. Windows already
    // inherited. Both must now inherit.
    let (program, args) = if cfg!(windows) {
        (
            "cmd.exe",
            vec!["/C".into(), "if defined PATH (exit 0) else (exit 1)".into()],
        )
    } else {
        ("sh", vec!["-c".into(), "test -n \"$PATH\"".into()])
    };

    let session = PtySession::spawn(program, &args, size(), None)
        .await
        .expect("spawn");
    let status = session.wait().await.expect("child should be reaped");
    assert!(
        status.success(),
        "child did not inherit PATH from the parent: {status}"
    );
}

#[tokio::test]
async fn resize_updates_pty_winsize() {
    let (program, args) = interactive_shell();
    let session = PtySession::spawn(program, &args, size(), None)
        .await
        .expect("spawn");

    session.resize(30, 100).expect("resize");
    let ws = session.get_winsize().expect("get_winsize");
    assert_eq!((ws.rows, ws.cols), (30, 100));

    session
        .terminate(Duration::from_secs(2))
        .await
        .expect("terminate");
    assert!(!session.is_alive());
}

#[tokio::test]
async fn write_all_reaches_the_child() {
    let (program, args) = interactive_shell();
    let session = PtySession::spawn(program, &args, size(), None)
        .await
        .expect("spawn");

    session
        .write_all(b"echo stitch-write-ok\r\n")
        .await
        .expect("write_all");

    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let mut seen = String::new();
    let mut buf = [0u8; 4096];
    while std::time::Instant::now() < deadline {
        match session
            .read_timeout(&mut buf, Duration::from_millis(300))
            .await
        {
            Ok(0) => break,
            Ok(n) => {
                seen.push_str(&String::from_utf8_lossy(&buf[..n]));
                // The shell echoes the command and then the command's output.
                if seen.matches("stitch-write-ok").count() >= 2 {
                    break;
                }
            }
            Err(_) => continue,
        }
    }
    assert!(
        seen.contains("stitch-write-ok"),
        "written command never reached the child; got {seen:?}"
    );

    session
        .terminate(Duration::from_secs(2))
        .await
        .expect("terminate");
}

#[tokio::test]
async fn read_timeout_returns_timeout_without_killing_the_session() {
    let (program, args) = quiet_long_running();
    let session = PtySession::spawn(program, &args, size(), None)
        .await
        .expect("spawn");

    let mut buf = [0u8; 64];
    // ConPTY emits a few setup bytes before the child is attached; drain
    // them, then require a real timeout from the now-silent session.
    for _ in 0..40 {
        match session
            .read_timeout(&mut buf, Duration::from_millis(50))
            .await
        {
            Ok(0) => panic!("child exited before the timeout check"),
            Ok(_) => continue,
            Err(stitch_pty::errors::PtyErrorKind::Timeout(_)) => break,
            Err(err) => panic!("unexpected read error: {err:?}"),
        }
    }

    let result = session
        .read_timeout(&mut buf, Duration::from_millis(150))
        .await;
    assert!(
        matches!(result, Err(stitch_pty::errors::PtyErrorKind::Timeout(_))),
        "expected Timeout, got {result:?}"
    );
    assert!(session.is_alive(), "timeout must not tear down the session");

    session
        .terminate(Duration::from_secs(2))
        .await
        .expect("terminate");
}

#[tokio::test]
async fn terminate_escalates_and_reaps() {
    let (program, args) = quiet_long_running();
    let session = PtySession::spawn(program, &args, size(), None)
        .await
        .expect("spawn");

    let started = std::time::Instant::now();
    session
        .terminate(Duration::from_millis(250))
        .await
        .expect("terminate");

    assert!(!session.is_alive(), "child survived terminate()");
    assert!(
        started.elapsed() < Duration::from_secs(6),
        "terminate() took {:?}",
        started.elapsed()
    );
}
