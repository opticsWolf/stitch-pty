//! H2 regression: a child must use ConPTY, not the parent's redirected stdio.
//!
//! This test lives in its own integration-test binary so the process-wide
//! `SetStdHandle` redirection cannot perturb tests running in parallel.

#![cfg(windows)]

use std::os::windows::io::AsRawHandle;
use std::time::Duration;

use stitch_pty::{PtySession, Winsize};
use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::Console::{
    GetStdHandle, STD_ERROR_HANDLE, STD_OUTPUT_HANDLE, SetStdHandle,
};

fn size() -> Option<Winsize> {
    Some(Winsize::new(24, 80, 0, 0))
}

struct RedirectGuard {
    original_out: HANDLE,
    original_err: HANDLE,
    file: Option<std::fs::File>,
    restored: bool,
}

impl RedirectGuard {
    fn new(path: &std::path::Path) -> Self {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(path)
            .expect("create redirect target");
        let original_out = unsafe { GetStdHandle(STD_OUTPUT_HANDLE) }.expect("stdout handle");
        let original_err = unsafe { GetStdHandle(STD_ERROR_HANDLE) }.expect("stderr handle");
        let handle = HANDLE(file.as_raw_handle() as _);
        unsafe {
            SetStdHandle(STD_OUTPUT_HANDLE, handle).expect("redirect stdout");
            SetStdHandle(STD_ERROR_HANDLE, handle).expect("redirect stderr");
        }
        Self {
            original_out,
            original_err,
            file: Some(file),
            restored: false,
        }
    }

    fn restore(&mut self) {
        if self.restored {
            return;
        }
        unsafe {
            SetStdHandle(STD_OUTPUT_HANDLE, self.original_out).expect("restore stdout");
            SetStdHandle(STD_ERROR_HANDLE, self.original_err).expect("restore stderr");
        }
        self.file.take();
        self.restored = true;
    }
}

impl Drop for RedirectGuard {
    fn drop(&mut self) {
        self.restore();
    }
}

async fn read_until_contains(session: &PtySession, needle: &str) -> String {
    let mut out = String::new();
    let mut buf = [0u8; 4096];
    for _ in 0..20 {
        match session.read_timeout(&mut buf, Duration::from_secs(1)).await {
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
async fn redirected_parent_stdio_is_not_inherited() {
    let path = std::env::temp_dir().join(format!(
        "stitch-pty-redirect-{}-{}.log",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    let mut guard = RedirectGuard::new(&path);

    let args = vec!["/C".to_string(), "echo stitch-redirect-leak".to_string()];
    let session = PtySession::spawn("cmd.exe", &args, size(), None)
        .await
        .expect("spawn");
    let out = read_until_contains(&session, "stitch-redirect-leak").await;
    let status = session.wait().await.expect("child should be reaped");
    guard.restore();

    assert!(status.success(), "child failed: {status}");
    assert!(
        out.contains("stitch-redirect-leak"),
        "child output did not reach the PTY: {out:?}"
    );
    let leaked = std::fs::read_to_string(&path).unwrap_or_default();
    let _ = std::fs::remove_file(&path);
    assert!(
        !leaked.contains("stitch-redirect-leak"),
        "child inherited the parent's redirected stdout: {leaked:?}"
    );
}
