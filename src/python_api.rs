//! PyO3 Python API bindings (cross-platform)
//!
//! Pattern: use `py: Python<'py>` + `future_into_py`. The async block returns plain Rust values;
//! future_into_py re-acquires the GIL to convert them to Python objects.

use crate::errors::PtyErrorKind;
use crate::platform::{open_pty_platform, spawn_platform, ChildBackend, PtyBackend};
use crate::winsize::Winsize;
use pyo3::prelude::*;
use std::time::Duration;

#[pyclass(skip_from_py_object)]
pub struct PtyMaster {
    inner: std::sync::Arc<dyn PtyBackend>,
}

#[pymethods]
impl PtyMaster {
    fn read<'py>(&self, py: Python<'py>, size: usize) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mut buf = vec![0u8; size];
            let n = inner.read(&mut buf).await
                .map_err(|e| PtyErrorKind::AsyncIo(e.to_string()))?;
            buf.truncate(n);
            Ok(buf)  // Vec<u8> → PyBytes
        })
    }

    fn read_timeout<'py>(&self, py: Python<'py>, size: usize, timeout_secs: f64) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mut buf = vec![0u8; size];
            let duration = Duration::from_secs_f64(timeout_secs);
            let timeout_result = tokio::time::timeout(duration, inner.read(&mut buf)).await;
            let n = match timeout_result {
                Ok(Ok(n)) => n,
                Ok(Err(e)) => return Err(PtyErrorKind::AsyncIo(e.to_string()).into()),
                Err(_) => return Err(PtyErrorKind::Timeout(duration).into()),
            };
            buf.truncate(n);
            Ok(buf)
        })
    }

    fn write<'py>(&self, py: Python<'py>, data: Vec<u8>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let n = inner.write(&data).await
                .map_err(|e| PtyErrorKind::AsyncIo(e.to_string()))?;
            Ok(n)  // usize → int
        })
    }

    fn write_all<'py>(&self, py: Python<'py>, data: Vec<u8>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mut remaining = &data[..];
            while !remaining.is_empty() {
                let n = inner.write(remaining).await
                    .map_err(|e| PtyErrorKind::AsyncIo(e.to_string()))?;
                remaining = &remaining[n..];
            }
            Ok(())  // () → None
        })
    }

    fn set_winsize(&self, winsize: &Winsize) -> PyResult<()> {
        self.inner.set_winsize(*winsize).map_err(PyErr::from)
    }

    fn get_winsize(&self) -> PyResult<Winsize> {
        self.inner.get_winsize().map_err(PyErr::from)
    }

    fn raw_fd(&self) -> i32 {
        #[cfg(unix)] { self.inner.raw_handle() as i32 }
        #[cfg(windows)] { -1 }
    }

    fn is_open(&self) -> bool {
        self.inner.is_open()
    }

    fn __repr__(&self) -> String {
        format!("PtyMaster(platform={}, open={})",
            if cfg!(unix) { "unix" } else { "windows" },
            self.is_open()
        )
    }
}

#[pyclass(skip_from_py_object)]
pub struct PtyChild {
    inner: std::sync::Arc<dyn ChildBackend>,
}

#[pymethods]
impl PtyChild {
    #[getter]
    fn pid(&self) -> u32 {
        self.inner.pid()
    }

    #[getter]
    fn is_running(&self) -> bool {
        self.inner.is_running()
    }

    fn wait<'py>(&mut self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let exit = inner.wait().await;
            match exit {
                Some(exit) => {
                    Ok((exit.pid, exit.exit_code, exit.signal, false))
                }
                None => {
                    Ok((0u32, None::<i32>, None::<i32>, false))
                }
            }
        })
    }

    fn terminate<'py>(&mut self, py: Python<'py>, grace_period_secs: f64) -> PyResult<Bound<'py, PyAny>> {
        let grace = Duration::from_secs_f64(grace_period_secs);
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let _ = inner.signal(15);
            let timeout_result = tokio::time::timeout(grace, async {
                inner.wait().await
            }).await;
            match timeout_result {
                Ok(Some(_)) | Ok(None) => Ok(()),
                Err(_) => {
                    let _ = inner.kill();
                    let _ = inner.wait().await;
                    Ok(())
                }
            }
        })
    }

    fn kill(&self) -> PyResult<()> {
        self.inner.kill().map_err(PyErr::from)
    }

    fn interrupt(&self) -> PyResult<()> {
        self.inner.signal(2).map_err(PyErr::from)
    }

    fn send_signal(&self, signal_num: i32) -> PyResult<()> {
        self.inner.signal(signal_num).map_err(PyErr::from)
    }

    fn __repr__(&self) -> String {
        format!("PtyChild(pid={}, running={})", self.inner.pid(), self.inner.is_running())
    }
}

#[pyclass(skip_from_py_object)]
pub struct PtySession {
    master: PtyMaster,
    child: PtyChild,
}

#[pymethods]
impl PtySession {
    fn read<'py>(&self, py: Python<'py>, size: usize) -> PyResult<Bound<'py, PyAny>> { self.master.read(py, size) }
    fn read_timeout<'py>(&self, py: Python<'py>, size: usize, timeout_secs: f64) -> PyResult<Bound<'py, PyAny>> { self.master.read_timeout(py, size, timeout_secs) }
    fn write<'py>(&self, py: Python<'py>, data: Vec<u8>) -> PyResult<Bound<'py, PyAny>> { self.master.write(py, data) }
    fn write_all<'py>(&self, py: Python<'py>, data: Vec<u8>) -> PyResult<Bound<'py, PyAny>> { self.master.write_all(py, data) }
    fn set_winsize(&self, winsize: &Winsize) -> PyResult<()> { self.master.set_winsize(winsize) }
    fn get_winsize(&self) -> PyResult<Winsize> { self.master.get_winsize() }

    fn resize(&self, rows: u16, cols: u16) -> PyResult<()> {
        self.master.set_winsize(&Winsize { rows, cols, xpixel: 0, ypixel: 0 })
    }

    fn wait<'py>(&mut self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> { self.child.wait(py) }
    fn terminate<'py>(&mut self, py: Python<'py>, grace_period_secs: f64) -> PyResult<Bound<'py, PyAny>> { self.child.terminate(py, grace_period_secs) }
    fn kill(&self) -> PyResult<()> { self.child.kill() }
    fn interrupt(&self) -> PyResult<()> { self.child.interrupt() }
    fn send_signal(&self, signal_num: i32) -> PyResult<()> { self.child.send_signal(signal_num) }

    #[getter]
    fn is_alive(&self) -> bool { self.child.is_running() }

    fn __repr__(&self) -> String {
        format!("PtySession(master={}, child={})",
            self.master.__repr__(), self.child.__repr__()
        )
    }
}

#[pyfunction]
#[pyo3(signature = (winsize=None))]
pub fn open_pty(py: Python<'_>, winsize: Option<Winsize>) -> PyResult<Bound<'_, PyAny>> {
    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        let backend = open_pty_platform(winsize)
            .await
            .map_err(PyErr::from)?;
        Ok(PtyMaster { inner: backend })
    })
}

#[pyfunction]
#[pyo3(signature = (program, args=None, env=None, winsize=None))]
pub fn spawn<'py>(
    py: Python<'py>,
    program: &str,
    args: Option<Vec<String>>,
    env: Option<std::collections::HashMap<String, String>>,
    winsize: Option<Winsize>,
) -> PyResult<Bound<'py, PyAny>> {
    let program = program.to_string();
    let args = args.unwrap_or_default();
    let env: Vec<(String, String)> = env.unwrap_or_default().into_iter().collect();
    let winsize = winsize;
    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        let (pty_backend, child_backend) = spawn_platform(&program, &args, &env, winsize)
            .await
            .map_err(|e| PtyErrorKind::ForkFailed(e.to_string()))?;
        let session = PtySession {
            master: PtyMaster { inner: pty_backend },
            child: PtyChild { inner: child_backend },
        };
        Ok(session)  // PtySession → Python object
    })
}
