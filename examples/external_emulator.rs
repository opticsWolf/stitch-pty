//! Feed a `PtySession` into an external terminal emulator crate.
//!
//! Run with:
//! `cargo run --example external_emulator --no-default-features`
//!
//! The session is transport-only: it owns the PTY and child, while `vt100`
//! owns the screen model. The reader/writer handles share the session's EOF
//! and close state without reimplementing its locking.

use std::error::Error;
use std::sync::Arc;
use std::time::Duration;

use stitch_pty::{PtySession, Winsize};
use vt100::Parser;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let (program, args) = if cfg!(windows) {
        (
            "cmd.exe",
            vec!["/C".to_string(), "echo external-emulator-ok".to_string()],
        )
    } else {
        (
            "sh",
            vec![
                "-c".to_string(),
                "printf 'external-emulator-ok\n'".to_string(),
            ],
        )
    };

    let session =
        Arc::new(PtySession::spawn(program, &args, Some(Winsize::new(24, 80, 0, 0)), None).await?);
    let reader = session.reader();
    let mut parser = Parser::default();

    let mut buf = [0u8; 4096];
    loop {
        match reader.read_timeout(&mut buf, Duration::from_secs(2)).await {
            Ok(0) => break,
            Ok(n) => parser.process(&buf[..n]),
            Err(stitch_pty::errors::PtyErrorKind::Timeout(_)) => break,
            Err(err) => return Err(err.into()),
        }
    }
    let text = parser.screen().contents();
    println!("{text}");
    if !text.contains("external-emulator-ok") {
        return Err("external emulator did not receive child output".into());
    }
    Ok(())
}
