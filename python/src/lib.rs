//! `flatland._native`: the Rust engine behind the `flatland` Python package.

use pyo3::prelude::*;
use std::path::PathBuf;

pyo3::create_exception!(_native, PcbError, pyo3::exceptions::PyException, "A flatland command failed; the message carries the tool's diagnostic and help text.");

fn to_py(e: flatland::error::Error) -> PyErr {
    PcbError::new_err(flatland::session::render_error(&e))
}

/// A project held in memory. Commands run in-process; nothing touches the
/// project file until `save()`.
#[pyclass(unsendable, module = "flatland._native")]
struct Session {
    inner: flatland::session::Session,
}

#[pymethods]
impl Session {
    /// A session whose project (once created with `run(["init", ...])`) would live at `path`;
    /// relative library and rule-set URLs resolve against that location.
    #[new]
    fn new(path: PathBuf) -> Self {
        Session { inner: flatland::session::Session::new(path) }
    }
    /// Load an existing project file into a new session.
    #[staticmethod]
    fn open(path: PathBuf) -> PyResult<Self> {
        Ok(Session { inner: flatland::session::Session::open(path).map_err(to_py)? })
    }
    /// Run one `pcb` command (the arguments after `pcb`) and return what it printed.
    fn run(&self, argv: Vec<String>) -> PyResult<String> {
        self.inner.run(&argv).map_err(to_py)
    }
    /// The project as JSON text (`pcb.json` contents).
    fn project_json(&self) -> PyResult<String> {
        self.inner.project_json().map_err(to_py)
    }
    /// Replace the project from JSON text.
    fn set_project_json(&self, json: &str) -> PyResult<()> {
        self.inner.set_project_json(json).map_err(to_py)
    }
    /// Write the project to its path (or another one, which then becomes its path).
    #[pyo3(signature = (path=None))]
    fn save(&self, path: Option<PathBuf>) -> PyResult<PathBuf> {
        self.inner.save(path.as_deref()).map_err(to_py)
    }
    #[getter]
    fn path(&self) -> PathBuf {
        self.inner.path()
    }
    #[getter]
    fn has_project(&self) -> bool {
        self.inner.has_project()
    }
}

#[pymodule]
fn _native(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<Session>()?;
    m.add("PcbError", m.py().get_type::<PcbError>())?;
    Ok(())
}
