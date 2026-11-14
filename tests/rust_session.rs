//! Integration tests for the high-level Rust [`stitch_pty::PtySession`] API.
//!
//! These run the real platform backend (openpty/fork on Unix, ConPTY on
//! Windows); CI executes them on all three platforms.

use std::time::Duration;
use stitch_pty::platform::open_pty_platform;
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

/// Read until `needle` appears or `timeout` elapses.
///
/// Deliberately does not wait for EOF: on Windows the ConPTY output pipe can
/// stay open after the child exits, so EOF may not arrive at all.
async fn read_until_contains(session: &PtySession, needle: &str, timeout: Duration) -> String {
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

/// Read until the Unix PTY reports EOF. Unix-only: Windows ConPTY can keep
/// the output pipe open after the child exits.
#[cfg(unix)]
async fn read_to_eof(session: &PtySession) -> String {
    let mut out = String::new();
    let mut buf = [0u8; 4096];
    loop {
        match session.read_timeout(&mut buf, Duration::from_secs(5)).await {
            Ok(0) => break,
            Ok(n) => out.push_str(&String::from_utf8_lossy(&buf[..n])),
            Err(err) => panic!("unexpected read error: {err:?}"),
        }
    }
    out
}

/// Wait briefly for the platform reaper/watcher to publish the child exit.
///
/// `is_alive()` reflects the cached reaper state, which can lag the OS exit by
/// a scheduling quantum; close itself already waits, but macOS CI occasionally
/// observes the reaper one tick late.
async fn assert_child_reaped(session: &PtySession, message: &str) {
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while session.is_alive() && std::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(!session.is_alive(), "{message}");
}

#[tokio::test]
async fn spawn_read_wait_reports_exit_status() {
    let (program, args) = shell("echo stitch-session-ok");
    let session = PtySession::spawn(program, &args, size(), None)
        .await
        .expect("spawn");

    let text = read_until_contains(&session, "stitch-session-ok", Duration::from_secs(5)).await;
    assert!(
        text.contains("stitch-session-ok"),
        "child output missing from PTY; got {text:?}"
    );

    let status = session.wait().await.expect("child should be reaped");
    assert!(status.success(), "unexpected exit status: {status}");
}

#[tokio::test]
async fn wait_is_repeatable_after_exit() {
    let (program, args) = shell("exit 0");
    let session = PtySession::spawn(program, &args, size(), None)
        .await
        .expect("spawn");

    let first = session.wait().await.expect("first wait");
    let second = session.wait().await.expect("second wait");
    assert_eq!(first.exit_code(), second.exit_code());
    assert_eq!(first.success(), second.success());
}

#[cfg(unix)]
#[tokio::test]
async fn wait_reports_signal_name_after_kill() {
    let session = PtySession::spawn("sleep", &["30".into()], size(), None)
        .await
        .expect("spawn");

    session.kill().expect("kill");
    // The Unix reaper polls every 50 ms; allow for scheduler lag on loaded CI.
    let status = tokio::time::timeout(Duration::from_secs(5), session.wait())
        .await
        .expect("child should exit after SIGKILL")
        .expect("status");
    assert!(!status.success());
    assert_eq!(status.signal(), Some("SIGKILL"));
}

#[cfg(unix)]
#[tokio::test]
async fn read_reports_eof_after_child_exit_on_unix() {
    // The Unix backend maps EIO on the master (slave closed) to Ok(0).
    // Windows is intentionally not asserted here: ConPTY can keep the output
    // pipe open after the child exits, so reads time out instead.
    let (program, args) = shell("echo eof-marker");
    let session = PtySession::spawn(program, &args, size(), None)
        .await
        .expect("spawn");

    let mut buf = [0u8; 4096];
    let mut saw_eof = false;
    for _ in 0..20 {
        match session.read_timeout(&mut buf, Duration::from_secs(2)).await {
            Ok(0) => {
                saw_eof = true;
                break;
            }
            Ok(_) => continue,
            Err(err) => panic!("unexpected read error before EOF: {err:?}"),
        }
    }
    assert!(saw_eof, "Unix PTY never reported EOF after child exit");
    assert!(session.wait().await.is_some());
}

#[cfg(unix)]
#[tokio::test]
async fn spawn_inherits_parent_environment() {
    // `PtySession::spawn` passes `None`, and the Unix backend builds `envp`
    // from `vars_os()`. Use /usr/bin/env rather than a shell: POSIX shells
    // inject a default PATH when it is missing, which would mask the bug.
    let session = PtySession::spawn("/usr/bin/env", &[], size(), None)
        .await
        .expect("spawn");
    let out = read_to_eof(&session).await;
    let status = session.wait().await.expect("child should be reaped");
    assert!(status.success(), "env failed: {status}");
    assert!(
        out.lines().any(|line| line.starts_with("PATH=")),
        "PATH was not inherited from the parent; env: {out:?}"
    );
}

#[cfg(windows)]
#[tokio::test]
async fn spawn_inherits_parent_environment() {
    let (program, args) = (
        "cmd.exe",
        vec!["/C".into(), "if defined PATH (exit 0) else (exit 1)".into()],
    );
    let session = PtySession::spawn(program, &args, size(), None)
        .await
        .expect("spawn");
    let status = session.wait().await.expect("child should be reaped");
    assert!(
        status.success(),
        "child did not inherit PATH from the parent: {status}"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn explicit_empty_env_is_empty() {
    // `Some(&[])` must be an empty environment, not an accidental "inherit".
    let session = PtySession::spawn_with_env("/usr/bin/env", &[], &[], size(), None)
        .await
        .expect("spawn");
    let out = read_to_eof(&session).await;
    let status = session.wait().await.expect("child should be reaped");
    assert!(status.success(), "env failed: {status}");
    assert_eq!(out, "", "empty env leaked variables: {out:?}");
}

#[cfg(windows)]
#[tokio::test]
async fn explicit_empty_env_is_empty() {
    // Windows needs an explicit `\0\0` block for an empty environment.
    let (program, args) = (
        "cmd.exe",
        vec!["/C".into(), "if defined PATH (exit 1) else (exit 0)".into()],
    );
    let session = PtySession::spawn_with_env(program, &args, &[], size(), None)
        .await
        .expect("spawn");
    let status = session.wait().await.expect("child should be reaped");
    assert!(status.success(), "empty env leaked PATH: {status}");
}

#[cfg(unix)]
#[tokio::test]
async fn explicit_env_replaces_parent_environment() {
    // A supplied env is the complete environment: inherited variables such as
    // PATH must be gone, and the supplied variable must be present.
    let env = vec![("STITCH_TEST".to_string(), "1".to_string())];
    let session = PtySession::spawn_with_env("/usr/bin/env", &[], &env, size(), None)
        .await
        .expect("spawn");
    let out = read_to_eof(&session).await;
    let status = session.wait().await.expect("child should be reaped");
    assert!(status.success(), "env failed: {status}");
    assert!(
        out.lines().any(|line| line == "STITCH_TEST=1"),
        "supplied variable missing; env: {out:?}"
    );
    assert!(
        !out.lines().any(|line| line.starts_with("PATH=")),
        "explicit env was merged with the parent; env: {out:?}"
    );
}

#[cfg(windows)]
#[tokio::test]
async fn explicit_env_dedupes_case_insensitive_keys() {
    // Windows environment names are case-insensitive. The last explicit entry
    // must win, so the child sees one PATH value rather than an ambiguous block.
    let env = vec![
        ("Path".to_string(), "first".to_string()),
        ("PATH".to_string(), "second".to_string()),
    ];
    let (program, args) = (
        "cmd.exe",
        vec!["/C".into(), "echo stitch-env=%PATH%".into()],
    );
    let session = PtySession::spawn_with_env(program, &args, &env, size(), None)
        .await
        .expect("spawn");
    let out = read_until_contains(&session, "stitch-env=", Duration::from_secs(5)).await;
    let status = session.wait().await.expect("child should be reaped");

    assert!(status.success(), "dedupe env command failed: {status}");
    assert!(
        out.contains("stitch-env=second"),
        "last case-insensitive PATH did not win; got {out:?}"
    );
    assert!(
        !out.contains("stitch-env=first"),
        "duplicate PATH leaked through; got {out:?}"
    );
}

#[cfg(windows)]
#[tokio::test]
async fn explicit_env_replaces_parent_environment() {
    let env = vec![("STITCH_TEST".to_string(), "1".to_string())];
    let (program, args) = (
        "cmd.exe",
        vec![
            "/C".into(),
            "if defined PATH (exit 1) & if \"%STITCH_TEST%\"==\"1\" (exit 0) & exit 2".into(),
        ],
    );
    let session = PtySession::spawn_with_env(program, &args, &env, size(), None)
        .await
        .expect("spawn");
    let status = session.wait().await.expect("child should be reaped");
    assert!(
        status.success(),
        "explicit env was merged with the parent: {status}"
    );
}

#[tokio::test]
async fn default_winsize_is_24x80() {
    let (program, args) = shell("exit 0");
    let session = PtySession::spawn(program, &args, None, None)
        .await
        .expect("spawn");
    let ws = session.get_winsize().expect("get_winsize");
    assert_eq!((ws.rows, ws.cols), (24, 80));
    assert!(session.wait().await.is_some());
}

#[tokio::test]
async fn open_pty_defaults_to_24x80() {
    let pty = open_pty_platform(None).await.expect("open_pty");
    let ws = pty.get_winsize().expect("get_winsize");
    assert_eq!((ws.rows, ws.cols), (24, 80));
}

#[tokio::test]
async fn resize_updates_pty_winsize() {
    // Use a non-interactive child: resize is a PTY-level operation, and
    // interactive shells on macOS keep job-control state that makes the
    // post-terminate liveness assertion timing-dependent.
    let (program, args) = quiet_long_running();
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
    // ConPTY emits a few setup bytes before the child is attached, and the
    // delivery can be delayed past the first quiet read. Wait for a sustained
    // quiet window instead of breaking on the first timeout, then require a
    // real timeout from the now-silent session.
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    let mut quiet_reads = 0;
    while std::time::Instant::now() < deadline {
        match session
            .read_timeout(&mut buf, Duration::from_millis(100))
            .await
        {
            Ok(0) => panic!("child exited before the timeout check"),
            Ok(_) => quiet_reads = 0,
            Err(stitch_pty::errors::PtyErrorKind::Timeout(_)) => {
                quiet_reads += 1;
                if quiet_reads >= 5 {
                    break;
                }
            }
            Err(err) => panic!("unexpected read error: {err:?}"),
        }
    }
    assert!(
        quiet_reads >= 5,
        "session never produced a sustained quiet window"
    );

    let result = session
        .read_timeout(&mut buf, Duration::from_millis(250))
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

#[cfg(unix)]
#[tokio::test]
async fn terminate_escalates_when_sigterm_is_ignored() {
    // The shell ignores SIGTERM (and `sleep` inherits SIG_IGN), so terminate
    // must exhaust the grace period and escalate to SIGKILL.
    let session = PtySession::spawn(
        "sh",
        &[
            "-c".into(),
            "trap '' TERM; while :; do sleep 1; done".into(),
        ],
        size(),
        None,
    )
    .await
    .expect("spawn");

    let started = std::time::Instant::now();
    session
        .terminate(Duration::from_millis(200))
        .await
        .expect("terminate");

    assert!(
        !session.is_alive(),
        "SIGTERM-ignoring child survived terminate()"
    );
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "terminate() took {:?}",
        started.elapsed()
    );
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

#[cfg(windows)]
#[tokio::test]
async fn interrupt_is_explicitly_unsupported_on_windows() {
    let (program, args) = quiet_long_running();
    let session = PtySession::spawn(program, &args, size(), None)
        .await
        .expect("spawn");

    let err = session
        .interrupt()
        .expect_err("Windows interrupt() must not silently succeed");
    match err {
        stitch_pty::errors::PtyErrorKind::Unsupported(msg) => {
            assert!(msg.contains("Windows ConPTY"), "unexpected message: {msg}");
        }
        other => panic!("expected Unsupported, got {other:?}"),
    }

    let _ = session.kill();
    let _ = session.wait().await;
}

#[tokio::test]
async fn read_until_exit_returns_output_and_status() {
    let (program, args) = shell("echo stitch-read-until-exit");
    let session = PtySession::spawn(program, &args, size(), None)
        .await
        .expect("spawn");

    let (output, status) = session
        .read_until_exit(Duration::from_secs(2))
        .await
        .expect("read_until_exit");

    let text = String::from_utf8_lossy(&output);
    assert!(
        text.contains("stitch-read-until-exit"),
        "missing child output; got {text:?}"
    );
    assert!(status.expect("status should be available").success());
}

#[tokio::test]
async fn read_until_exit_drains_large_output() {
    let command = if cfg!(windows) {
        "for /L %i in (1,1,1000) do @echo 0123456789"
    } else {
        "for i in $(seq 1 1000); do echo 0123456789; done"
    };
    let (program, args) = shell(command);
    let session = PtySession::spawn(program, &args, size(), None)
        .await
        .expect("spawn");

    let (output, status) = session
        .read_until_exit(Duration::from_secs(5))
        .await
        .expect("read_until_exit");

    assert!(output.len() > 10_000, "drained only {} bytes", output.len());
    assert!(status.expect("status should be available").success());
}

#[tokio::test]
async fn close_is_idempotent_and_does_not_hang_with_unread_output() {
    let (program, args) = quiet_long_running();
    let session = PtySession::spawn(program, &args, size(), None)
        .await
        .expect("spawn");

    let started = std::time::Instant::now();
    session.close().await.expect("first close");
    session.close().await.expect("second close");
    assert!(
        started.elapsed() < Duration::from_secs(4),
        "close took {:?}",
        started.elapsed()
    );
    assert_child_reaped(&session, "close left the child running").await;
}

#[cfg(unix)]
#[tokio::test]
async fn shutdown_write_sends_eof_without_closing_output() {
    let session = PtySession::spawn("cat", &[], size(), None)
        .await
        .expect("spawn");

    session.shutdown_write().await.expect("shutdown_write");
    let status = tokio::time::timeout(Duration::from_secs(2), session.wait())
        .await
        .expect("cat should exit after VEOF")
        .expect("status");
    assert!(status.success());
}

#[tokio::test]
async fn close_with_unread_output_does_not_hang() {
    let (program, args) = shell("echo stitch-close-unread");
    let session = PtySession::spawn(program, &args, size(), None)
        .await
        .expect("spawn");

    let started = std::time::Instant::now();
    session.close().await.expect("close");
    assert!(
        started.elapsed() < Duration::from_secs(4),
        "close with unread output took {:?}",
        started.elapsed()
    );
    assert_child_reaped(&session, "close with unread output left the child running").await;
}

#[tokio::test]
async fn read_after_read_until_exit_returns_eof() {
    let (program, args) = shell("echo stitch-eof");
    let session = PtySession::spawn(program, &args, size(), None)
        .await
        .expect("spawn");

    let (output, status) = session
        .read_until_exit(Duration::from_secs(2))
        .await
        .expect("read_until_exit");
    assert!(String::from_utf8_lossy(&output).contains("stitch-eof"));
    assert!(status.expect("status").success());

    let mut buf = [0u8; 64];
    let eof = session
        .read_timeout(&mut buf, Duration::from_millis(250))
        .await;
    assert_eq!(eof, Ok(0), "expected EOF after read_until_exit");
}
