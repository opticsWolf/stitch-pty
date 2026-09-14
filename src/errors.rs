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

    #[error("PTY EOF: child side closed")]
    Eof,

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
            PtyErrorKind::Eof => {
                // Stable, string-match-free contract for Python: a builtin
                // OSError with errno 0 and a `kind == "eof"` attribute.
                let py_err = PyOSError::new_err((0, err.to_string()));
                Python::attach(|py| {
                    let _ = py_err.value(py).setattr("kind", "eof");
                });
                py_err
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

impl PtyErrorKind {
    /// Classify a `std::io::Error` from a PTY master/pipe **read**.
    ///
    /// Errors that mean "the child side is gone" become [`PtyErrorKind::Eof`]
    /// (surfaced to Python as an `OSError` with `errno == 0` and a stable
    /// `kind == "eof"` attribute); everything else becomes `AsyncIo`.
    ///
    /// Write paths must NOT use this: on Unix, write-side EIO maps to
    /// `BrokenPipe` in the backend instead (see `platform_unix.rs`).
    pub fn from_read_error(err: std::io::Error) -> Self {
        if is_eof_error(&err) {
            PtyErrorKind::Eof
        } else {
            PtyErrorKind::AsyncIo(err.to_string())
        }
    }
}

/// True when an `io::Error` from a PTY read means the child side closed.
///
/// Centralizes what used to be ad-hoc `"os error 5" in str(e)` matching on
/// the Python side: Unix EIO on the master, or ConPTY's broken-pipe errors.
/// (The Unix backend additionally short-circuits EIO to `Ok(0)`; this covers
/// every other read path, e.g. readiness errors and the Windows backend.)
pub fn is_eof_error(err: &std::io::Error) -> bool {
    #[cfg(unix)]
    {
        if err.raw_os_error() == Some(libc::EIO) {
            return true;
        }
    }
    #[cfg(windows)]
    {
        // ERROR_BROKEN_PIPE (109) / ERROR_NO_DATA (232) from the ConPTY pipe.
        const ERROR_BROKEN_PIPE: i32 = 109;
        const ERROR_NO_DATA: i32 = 232;
        if matches!(
            err.raw_os_error(),
            Some(ERROR_BROKEN_PIPE) | Some(ERROR_NO_DATA)
        ) {
            return true;
        }
    }
    false
}

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

    #[test]
    fn test_error_display_eof() {
        assert_eq!(PtyErrorKind::Eof.to_string(), "PTY EOF: child side closed");
    }

    #[cfg(unix)]
    #[test]
    fn test_from_read_error_eio_is_eof() {
        let eio = std::io::Error::from_raw_os_error(libc::EIO);
        assert!(is_eof_error(&eio));
        assert_eq!(
            PtyErrorKind::from_read_error(eio),
            PtyErrorKind::Eof
        );
    }

    #[test]
    fn test_from_read_error_other_is_async_io() {
        let other = std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "refused");
        assert!(!is_eof_error(&other));
        match PtyErrorKind::from_read_error(other) {
            PtyErrorKind::AsyncIo(msg) => assert!(msg.contains("refused")),
            _ => panic!("expected AsyncIo"),
        }
    }

    #[cfg(feature = "python")]
    #[test]
    fn test_eof_maps_to_oserror_with_kind_attr() {
        Python::attach(|py| {
            let py_err: PyErr = PtyErrorKind::Eof.into();
            let value = py_err.value(py);
            assert!(value.is_instance_of::<PyOSError>());
            assert_eq!(value.getattr("errno").unwrap().extract::<i32>().unwrap(), 0);
            assert_eq!(value.getattr("kind").unwrap().extract::<String>().unwrap(), "eof");
        });
    }
}
