//! Error types and Python exception mapping (cross-platform)

#[cfg(feature = "python")]
use pyo3::create_exception;
#[cfg(feature = "python")]
use pyo3::exceptions::{PyException, PyIOError, PyOSError};
#[cfg(feature = "python")]
use pyo3::prelude::*;

// ── Python Exception Types ───────────────────────────────────────

#[cfg(feature = "python")]
create_exception!(stitch_pty, PtyError, PyException);
#[cfg(feature = "python")]
create_exception!(stitch_pty, ProcessError, PtyError);
#[cfg(feature = "python")]
create_exception!(stitch_pty, IOError, PtyError);

// ── Rust Error Types ───────────────────────────────────────────

#[derive(thiserror::Error, Debug, PartialEq, Eq)]
pub enum PtyErrorKind {
    #[error("PTY open failed: {0}")]
    OpenFailed(String),

    #[error("PTY operation failed: {0}")]
    OperationFailed(String),

    #[error("Process spawn failed: {0}")]
    ForkFailed(String),

    #[error("Process not running")]
    ProcessNotRunning,

    #[error("Invalid handle/FD")]
    InvalidHandle,

    #[error("Window size operation failed: {0}")]
    WinsizeFailed(String),

    #[error("Signal handling error: {0}")]
    SignalError(String),

    #[error("Async I/O error: {0}")]
    AsyncIo(String),

    #[error("Buffer overflow: requested {requested}, max {max}")]
    BufferOverflow { requested: usize, max: usize },

    #[error("PTY is closed")]
    Closed,

    #[error("Timeout after {0:?}")]
    Timeout(std::time::Duration),

    #[error("Platform not supported: {0}")]
    PlatformNotSupported(String),

    #[error("Windows API error: {0}")]
    WindowsError(String),
}

// ── IntoPy<PyErr> Implementation ──────────────────────────────

#[cfg(feature = "python")]
impl From<PtyErrorKind> for PyErr {
    fn from(err: PtyErrorKind) -> PyErr {
        match &err {
            PtyErrorKind::OpenFailed(_)
            | PtyErrorKind::OperationFailed(_)
            | PtyErrorKind::Closed
            | PtyErrorKind::PlatformNotSupported(_) => {
                PtyError::new_err(err.to_string())
            }
            PtyErrorKind::ForkFailed(_)
            | PtyErrorKind::ProcessNotRunning => {
                ProcessError::new_err(err.to_string())
            }
            PtyErrorKind::InvalidHandle
            | PtyErrorKind::WinsizeFailed(_)
            | PtyErrorKind::BufferOverflow { .. } => {
                IOError::new_err(err.to_string())
            }
            PtyErrorKind::Timeout(_) => {
                PtyError::new_err(err.to_string())
            }
            PtyErrorKind::SignalError(_)
            | PtyErrorKind::WindowsError(_) => {
                PyOSError::new_err(err.to_string())
            }
            PtyErrorKind::AsyncIo(_) => {
                PyIOError::new_err(err.to_string())
            }
        }
    }
}

// ── Conversion from platform errors ───────────────────────────

#[cfg(unix)]
impl From<nix::Error> for PtyErrorKind {
    fn from(err: nix::Error) -> Self {
        PtyErrorKind::OperationFailed(format!("{} (errno: {:?})", err, err))
    }
}

impl From<std::io::Error> for PtyErrorKind {
    fn from(err: std::io::Error) -> Self {
        PtyErrorKind::AsyncIo(err.to_string())
    }
}

#[cfg(windows)]
impl From<windows::core::Error> for PtyErrorKind {
    fn from(err: windows::core::Error) -> Self {
        PtyErrorKind::WindowsError(format!("{:?}", err))
    }
}

// ── Result Type Alias ─────────────────────────────────────────

pub type PtyResult<T> = Result<T, PtyErrorKind>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display_open_failed() {
        let err = PtyErrorKind::OpenFailed("device busy".to_string());
        assert_eq!(err.to_string(), "PTY open failed: device busy");
    }

    #[test]
    fn test_error_display_operation_failed() {
        let err = PtyErrorKind::OperationFailed("read error".to_string());
        assert_eq!(err.to_string(), "PTY operation failed: read error");
    }

    #[test]
    fn test_error_display_fork_failed() {
        let err = PtyErrorKind::ForkFailed("no memory".to_string());
        assert_eq!(err.to_string(), "Process spawn failed: no memory");
    }

    #[test]
    fn test_error_display_process_not_running() {
        let err = PtyErrorKind::ProcessNotRunning;
        assert_eq!(err.to_string(), "Process not running");
    }

    #[test]
    fn test_error_display_invalid_handle() {
        let err = PtyErrorKind::InvalidHandle;
        assert_eq!(err.to_string(), "Invalid handle/FD");
    }

    #[test]
    fn test_error_display_winsize_failed() {
        let err = PtyErrorKind::WinsizeFailed("ioctl failed".to_string());
        assert_eq!(err.to_string(), "Window size operation failed: ioctl failed");
    }

    #[test]
    fn test_error_display_signal_error() {
        let err = PtyErrorKind::SignalError("bad signal".to_string());
        assert_eq!(err.to_string(), "Signal handling error: bad signal");
    }

    #[test]
    fn test_error_display_async_io() {
        let err = PtyErrorKind::AsyncIo("connection reset".to_string());
        assert_eq!(err.to_string(), "Async I/O error: connection reset");
    }

    #[test]
    fn test_error_display_buffer_overflow() {
        let err = PtyErrorKind::BufferOverflow { requested: 1024, max: 512 };
        assert_eq!(err.to_string(), "Buffer overflow: requested 1024, max 512");
    }

    #[test]
    fn test_error_display_closed() {
        let err = PtyErrorKind::Closed;
        assert_eq!(err.to_string(), "PTY is closed");
    }

    #[test]
    fn test_error_display_timeout() {
        let err = PtyErrorKind::Timeout(std::time::Duration::from_secs(5));
        assert!(err.to_string().contains("5s"));
    }

    #[test]
    fn test_error_display_platform_not_supported() {
        let err = PtyErrorKind::PlatformNotSupported("darwin arm64".to_string());
        assert_eq!(err.to_string(), "Platform not supported: darwin arm64");
    }

    #[test]
    fn test_error_display_windows_error() {
        let err = PtyErrorKind::WindowsError("test error".to_string());
        assert_eq!(err.to_string(), "Windows API error: test error");
    }

    #[test]
    fn test_io_error_from_std_io_error() {
        let std_err = std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "refused");
        let pty_err: PtyErrorKind = PtyErrorKind::from(std_err);
        match pty_err {
            PtyErrorKind::AsyncIo(msg) => assert!(msg.contains("refused")),
            _ => panic!("expected AsyncIo"),
        }
    }

    #[test]
    fn test_result_type_alias() {
        let ok: PtyResult<i32> = Ok(42);
        assert_eq!(ok.unwrap(), 42);
    }
}
