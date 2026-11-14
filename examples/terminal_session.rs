//! Spawn a child with an owned `TerminalSession` and render its screen.
//!
//! `cargo run --example terminal_session --no-default-features`

use std::error::Error;
use std::time::Duration;

use stitch_pty::{TerminalSession, Winsize};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let (program, args) = if cfg!(windows) {
        (
            "cmd.exe",
            vec![
                "/C".to_string(),
                "echo terminal-session-example".to_string(),
            ],
        )
    } else {
        (
            "sh",
            vec![
                "-c".to_string(),
                "printf 'terminal-session-example\n'".to_string(),
            ],
        )
    };

    let mut session =
        TerminalSession::spawn(program, &args, Some(Winsize::new(24, 80, 0, 0)), None, 1000)
            .await?;

    let mut buf = [0u8; 4096];
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        match session
            .read_timeout(&mut buf, Duration::from_millis(250))
            .await
        {
            Ok(0) => {
                if !session.is_alive() {
                    break;
                }
            }
            Ok(_) => {
                if session
                    .display()
                    .iter()
                    .any(|line| line.contains("terminal-session-example"))
                {
                    break;
                }
            }
            Err(stitch_pty::errors::PtyErrorKind::Timeout(_)) => continue,
            Err(err) => return Err(err.into()),
        }
    }

    for line in session.display() {
        println!("{line}");
    }
    session.wait().await;
    Ok(())
}
