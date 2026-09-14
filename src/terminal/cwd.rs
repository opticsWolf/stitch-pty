//! Shell working-directory tracking (OSC 7 / OSC 9;9).
//!
//! Terminals report the shell's cwd so front-ends can show it in tab
//! tooltips or use it as the default directory for new tabs:
//! - `OSC 7 ; file://host/path ST|BEL` (macOS/Linux shells, Windows Terminal)
//! - `OSC 9;9 ; path ST` (ConPTY: raw OS path, no URI wrapper)
//!
//! All input here is untrusted shell output: parsing is total (returns
//! `Option`), never panics, never allocates on failure paths more than
//! necessary, and rejects anything that is not clean UTF-8.

/// Parse an OSC 7 payload (`file://host/path`) into a local path.
///
/// Returns `None` for malformed input: missing scheme, missing path,
/// bad `%XX` sequences, non-UTF-8 bytes, or embedded NULs.
/// A leading `/C:/…` drive prefix is normalized to `C:/…`.
pub fn parse_osc7(payload: &[u8]) -> Option<String> {
    const SCHEME: &[u8] = b"file://";
    if payload.len() < SCHEME.len() || !payload[..SCHEME.len()].eq_ignore_ascii_case(SCHEME) {
        return None;
    }
    let rest = &payload[SCHEME.len()..];
    // Split authority (host) from path at the first '/'.
    let slash = rest.iter().position(|&b| b == b'/')?;
    let path_enc = &rest[slash..];
    if path_enc.is_empty() {
        return None;
    }
    let path = percent_decode(path_enc)?;
    if path.is_empty() || path.contains('\0') {
        return None;
    }
    // `file:///C:/Users/x` → `C:/Users/x`.
    let bytes = path.as_bytes();
    if bytes.len() >= 3
        && bytes[0] == b'/'
        && bytes[1].is_ascii_alphabetic()
        && bytes[2] == b':'
    {
        return Some(path[1..].to_string());
    }
    Some(path)
}

/// Parse an OSC 9;9 payload (raw OS path, e.g. `C:\Users\Main`).
///
/// Strict UTF-8; rejects empty input and embedded NULs.
pub fn parse_osc9_9(payload: &[u8]) -> Option<String> {
    if payload.is_empty() {
        return None;
    }
    let path = std::str::from_utf8(payload).ok()?;
    if path.is_empty() || path.contains('\0') {
        return None;
    }
    Some(path.to_string())
}

/// Percent-decode `%XX` sequences; `None` on any malformed sequence
/// (lone `%`, truncated tail, non-hex digits) or non-UTF-8 result.
fn percent_decode(input: &[u8]) -> Option<String> {
    let mut out: Vec<u8> = Vec::with_capacity(input.len());
    let mut i = 0;
    while i < input.len() {
        if input[i] == b'%' {
            if i + 2 >= input.len() {
                return None;
            }
            let hi = hex_val(input[i + 1])?;
            let lo = hex_val(input[i + 2])?;
            out.push((hi << 4) | lo);
            i += 3;
        } else {
            out.push(input[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_osc7_basic_unix_path() {
        assert_eq!(
            parse_osc7(b"file://myhost/home/user"),
            Some("/home/user".to_string())
        );
    }

    #[test]
    fn test_osc7_localhost_empty_host() {
        assert_eq!(
            parse_osc7(b"file:///etc/hostname"),
            Some("/etc/hostname".to_string())
        );
    }

    #[test]
    fn test_osc7_percent_decoding() {
        assert_eq!(
            parse_osc7(b"file://myhost/home/user%20name/docs"),
            Some("/home/user name/docs".to_string())
        );
    }

    #[test]
    fn test_osc7_percent_encoded_utf8() {
        // `ü` = U+00FC = 0xC3 0xBC.
        assert_eq!(
            parse_osc7(b"file://h/m%C3%BCnchen"),
            Some("/münchen".to_string())
        );
    }

    #[test]
    fn test_osc7_windows_drive() {
        assert_eq!(
            parse_osc7(b"file:///C:/Users/Main"),
            Some("C:/Users/Main".to_string())
        );
    }

    #[test]
    fn test_osc7_rejects_garbage() {
        assert_eq!(parse_osc7(b""), None);
        assert_eq!(parse_osc7(b"not a uri"), None);
        assert_eq!(parse_osc7(b"http://host/path"), None);
        assert_eq!(parse_osc7(b"file://host"), None); // no path
        assert_eq!(parse_osc7(b"file://host/"), Some("/".to_string()));
        assert_eq!(parse_osc7(b"file://h/a%zz"), None); // bad hex
        assert_eq!(parse_osc7(b"file://h/a%2"), None); // truncated
        assert_eq!(parse_osc7(b"file://h/a%"), None); // lone %
        assert_eq!(parse_osc7(b"file://h/\xff\xfe"), None); // non-UTF-8
        assert_eq!(parse_osc7(b"file://h/a\x00b"), None); // NUL
    }

    #[test]
    fn test_osc9_9_windows_path() {
        assert_eq!(
            parse_osc9_9(r"C:\Users\Main".as_bytes()),
            Some(r"C:\Users\Main".to_string())
        );
    }

    #[test]
    fn test_osc9_9_rejects_garbage() {
        assert_eq!(parse_osc9_9(b""), None);
        assert_eq!(parse_osc9_9(b"\xff\xfe"), None);
        assert_eq!(parse_osc9_9(b"C:\\a\x00b"), None);
    }
}
