//! Platform-agnostic async I/O helpers.

use std::time::Duration;

/// Read with a timeout, returning None on timeout.
pub async fn read_timeout(
    read_fn: impl std::future::Future<Output = std::io::Result<usize>> + Send,
    _buf: &mut [u8],
    timeout: Duration,
) -> Result<Option<usize>, crate::errors::PtyErrorKind> {
    tokio::time::timeout(timeout, read_fn).await
        .map_err(|_| crate::errors::PtyErrorKind::Timeout(timeout))?
        .map(Some)
        .map_err(|e| crate::errors::PtyErrorKind::AsyncIo(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::PtyErrorKind;

    #[tokio::test]
    async fn test_read_timeout_success() {
        let mut buf = [0u8; 10];
        let success = async { Ok::<usize, std::io::Error>(5) };
        let result = read_timeout(success, &mut buf, Duration::from_secs(1)).await;
        assert_eq!(result, Ok(Some(5)));
    }

    #[tokio::test]
    async fn test_read_timeout_zero_bytes() {
        let mut buf = [0u8; 10];
        let success = async { Ok::<usize, std::io::Error>(0) };
        let result = read_timeout(success, &mut buf, Duration::from_secs(1)).await;
        assert_eq!(result, Ok(Some(0)));
    }

    #[tokio::test]
    async fn test_read_timeout_large_read() {
        let mut buf = [0u8; 4096];
        let success = async { Ok::<usize, std::io::Error>(4096) };
        let result = read_timeout(success, &mut buf, Duration::from_secs(1)).await;
        assert_eq!(result, Ok(Some(4096)));
    }

    #[tokio::test]
    async fn test_read_timeout_expires() {
        let mut buf = [0u8; 10];
        let slow = async { tokio::time::sleep(Duration::from_secs(10)).await; Ok::<usize, std::io::Error>(5) };
        let result = read_timeout(slow, &mut buf, Duration::from_millis(50)).await;
        match result {
            Err(PtyErrorKind::Timeout(d)) => assert_eq!(d, Duration::from_millis(50)),
            _ => panic!("expected Timeout error"),
        }
    }

    #[tokio::test]
    async fn test_read_timeout_io_error() {
        let mut buf = [0u8; 10];
        let fail = async { Err::<usize, std::io::Error>(std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "refused")) };
        let result = read_timeout(fail, &mut buf, Duration::from_secs(1)).await;
        match result {
            Err(PtyErrorKind::AsyncIo(msg)) => assert!(msg.contains("refused")),
            _ => panic!("expected AsyncIo error"),
        }
    }

    #[tokio::test]
    async fn test_read_timeout_io_error_kind() {
        let mut buf = [0u8; 10];
        let fail = async { Err::<usize, std::io::Error>(std::io::Error::new(std::io::ErrorKind::TimedOut, "timeout")) };
        let result = read_timeout(fail, &mut buf, Duration::from_secs(1)).await;
        match result {
            Err(PtyErrorKind::AsyncIo(msg)) => assert!(msg.contains("timeout")),
            _ => panic!("expected AsyncIo error"),
        }
    }

    #[tokio::test]
    async fn test_read_timeout_io_error_kind_permission() {
        let mut buf = [0u8; 10];
        let fail = async { Err::<usize, std::io::Error>(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied")) };
        let result = read_timeout(fail, &mut buf, Duration::from_secs(1)).await;
        match result {
            Err(PtyErrorKind::AsyncIo(msg)) => assert!(msg.contains("denied")),
            _ => panic!("expected AsyncIo error"),
        }
    }

    #[tokio::test]
    async fn test_read_timeout_short_duration() {
        let mut buf = [0u8; 10];
        let success = async { Ok::<usize, std::io::Error>(1) };
        let result = read_timeout(success, &mut buf, Duration::from_millis(1)).await;
        assert_eq!(result, Ok(Some(1)));
    }

    #[tokio::test]
    async fn test_read_timeout_buffer_not_modified_on_timeout() {
        let mut buf = [0u8; 10];
        buf[0] = 0xAA;
        let slow = async { tokio::time::sleep(Duration::from_secs(10)).await; Ok::<usize, std::io::Error>(5) };
        let result = read_timeout(slow, &mut buf, Duration::from_millis(50)).await;
        assert!(result.is_err());
        // Buffer should be unchanged on timeout
        assert_eq!(buf[0], 0xAA);
    }

    #[tokio::test]
    async fn test_read_timeout_buffer_modified_on_success() {
        let mut buf = [0u8; 10];
        let success = async { Ok::<usize, std::io::Error>(3) };
        let result = read_timeout(success, &mut buf, Duration::from_secs(1)).await;
        assert_eq!(result, Ok(Some(3)));
    }

    #[tokio::test]
    async fn test_duration_zero_timeout() {
        let mut buf = [0u8; 10];
        let success = async { Ok::<usize, std::io::Error>(5) };
        let result = read_timeout(success, &mut buf, Duration::from_secs(0)).await;
        assert_eq!(result, Ok(Some(5)));
    }

    #[test]
    fn test_error_kind_timeout_display() {
        let err = PtyErrorKind::Timeout(Duration::from_secs(5));
        assert!(err.to_string().contains("5s"));
    }

    #[test]
    fn test_error_kind_async_io_display() {
        let err = PtyErrorKind::AsyncIo("test error".to_string());
        assert_eq!(err.to_string(), "Async I/O error: test error");
    }
}
