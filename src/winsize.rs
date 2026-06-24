use pyo3::prelude::*;

#[pyclass(from_py_object)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Winsize {
    #[pyo3(get, set)] pub rows: u16,
    #[pyo3(get, set)] pub cols: u16,
    #[pyo3(get, set)] pub xpixel: u16,
    #[pyo3(get, set)] pub ypixel: u16,
}

#[pymethods]
impl Winsize {
    #[new]
    #[pyo3(signature = (rows, cols, xpixel = 0, ypixel = 0))]
    fn new(rows: u16, cols: u16, xpixel: u16, ypixel: u16) -> Self {
        Winsize { rows, cols, xpixel, ypixel }
    }

    fn __repr__(&self) -> String {
        format!("Winsize(rows={}, cols={}, xpixel={}, ypixel={})",
            self.rows, self.cols, self.xpixel, self.ypixel)
    }

    fn __eq__(&self, other: &Winsize) -> bool {
        self.rows == other.rows && self.cols == other.cols
            && self.xpixel == other.xpixel && self.ypixel == other.ypixel
    }
}

#[cfg(unix)]
impl From<Winsize> for nix::pty::Winsize {
    fn from(w: Winsize) -> Self {
        nix::pty::Winsize {
            ws_row: w.rows, ws_col: w.cols,
            ws_xpixel: w.xpixel, ws_ypixel: w.ypixel,
        }
    }
}

#[cfg(unix)]
impl From<nix::pty::Winsize> for Winsize {
    fn from(w: nix::pty::Winsize) -> Self {
        Winsize {
            rows: w.ws_row, cols: w.ws_col,
            xpixel: w.ws_xpixel, ypixel: w.ws_ypixel,
        }
    }
}

#[cfg(windows)]
impl From<Winsize> for windows::Win32::System::Console::COORD {
    fn from(w: Winsize) -> Self {
        windows::Win32::System::Console::COORD {
            X: w.cols as i16, Y: w.rows as i16,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_winsize_new() {
        let ws = Winsize::new(24, 80, 0, 0);
        assert_eq!(ws.rows, 24);
        assert_eq!(ws.cols, 80);
        assert_eq!(ws.xpixel, 0);
        assert_eq!(ws.ypixel, 0);
    }

    #[test]
    fn test_winsize_default() {
        let ws = Winsize::default();
        assert_eq!(ws.rows, 0);
        assert_eq!(ws.cols, 0);
        assert_eq!(ws.xpixel, 0);
        assert_eq!(ws.ypixel, 0);
    }

    #[test]
    fn test_winsize_equality_equal() {
        let ws1 = Winsize::new(24, 80, 100, 200);
        let ws2 = Winsize::new(24, 80, 100, 200);
        assert_eq!(ws1, ws2);
    }

    #[test]
    fn test_winsize_equality_different_rows() {
        let ws1 = Winsize::new(24, 80, 0, 0);
        let ws2 = Winsize::new(25, 80, 0, 0);
        assert_ne!(ws1, ws2);
    }

    #[test]
    fn test_winsize_equality_different_cols() {
        let ws1 = Winsize::new(24, 80, 0, 0);
        let ws2 = Winsize::new(24, 120, 0, 0);
        assert_ne!(ws1, ws2);
    }

    #[test]
    fn test_winsize_equality_different_pixels() {
        let ws1 = Winsize::new(24, 80, 100, 200);
        let ws2 = Winsize::new(24, 80, 200, 300);
        assert_ne!(ws1, ws2);
    }

    #[test]
    fn test_winsize_repr() {
        let ws = Winsize::new(24, 80, 100, 200);
        let repr = ws.__repr__();
        assert!(repr.contains("rows=24"));
        assert!(repr.contains("cols=80"));
        assert!(repr.contains("xpixel=100"));
        assert!(repr.contains("ypixel=200"));
    }

    #[test]
    fn test_winsize_repr_default() {
        let ws = Winsize::default();
        let repr = ws.__repr__();
        assert!(repr.contains("rows=0"));
        assert!(repr.contains("cols=0"));
    }

    #[test]
    fn test_winsize_clone() {
        let ws1 = Winsize::new(24, 80, 100, 200);
        let ws2 = ws1.clone();
        assert_eq!(ws1, ws2);
    }

    #[test]
    fn test_winsize_debug() {
        let ws = Winsize::new(24, 80, 0, 0);
        let debug = format!("{:?}", ws);
        assert!(debug.contains("Winsize"));
        assert!(debug.contains("24"));
        assert!(debug.contains("80"));
    }

    #[test]
    fn test_winsize_max_values() {
        let ws = Winsize::new(u16::MAX, u16::MAX, u16::MAX, u16::MAX);
        assert_eq!(ws.rows, u16::MAX);
        assert_eq!(ws.cols, u16::MAX);
    }

    #[test]
    fn test_winsize_copy_trait() {
        // Verify Winsize is Copy
        let ws1 = Winsize::new(24, 80, 0, 0);
        let ws2 = ws1; // should copy, not move
        let ws3 = ws1; // should also work
        assert_eq!(ws1, ws2);
        assert_eq!(ws2, ws3);
    }

    #[test]
    fn test_winsize_from_py_object_default() {
        // Verify Winsize can be created from default (from_py_object)
        let ws = Winsize::default();
        assert_eq!(ws.rows, 0);
        assert_eq!(ws.cols, 0);
    }
}
