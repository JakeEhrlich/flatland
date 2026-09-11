//! In-process use of the tool: a project held in memory, commands run as
//! functions with their output captured. The Python bindings sit on this.

use crate::cli::{Cli, Ctx, MemStore};
use crate::error::{Error, Result};
use crate::schema::Project;
use crate::store;
use clap::Parser;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

pub struct Session {
    store: Rc<RefCell<MemStore>>,
}

impl Session {
    /// A session whose project would live at `path`. Nothing is read or written
    /// until `open`/`save`; relative URLs resolve against `path`'s directory.
    pub fn new(path: impl Into<PathBuf>) -> Session {
        let mut path: PathBuf = path.into();
        if path.is_dir() {
            path = path.join(crate::schema::PROJECT_FILENAME);
        }
        let path = store::absolute(&path);
        Session { store: Rc::new(RefCell::new(MemStore::new(path))) }
    }
    pub fn open(path: impl Into<PathBuf>) -> Result<Session> {
        let s = Session::new(path);
        let p = s.path();
        let project = store::load_project(&p)?;
        s.store.borrow_mut().project = Some(project);
        Ok(s)
    }
    pub fn path(&self) -> PathBuf {
        self.store.borrow().path.clone()
    }
    pub fn has_project(&self) -> bool {
        self.store.borrow().project.is_some()
    }
    fn ctx(&self) -> Ctx {
        Ctx::in_memory(self.store.clone())
    }
    /// Run one command (the words after `pcb`) and return everything it printed
    /// to stdout and stderr, in order.
    pub fn run(&self, argv: &[String]) -> Result<String> {
        let mut full = vec!["pcb".to_string()];
        full.extend(argv.iter().cloned());
        let cli = Cli::try_parse_from(&full).map_err(|e| Error::msg(e.to_string().trim_end().to_string()))?;
        let ctx = self.ctx();
        let (result, output) = capture(|| crate::cli::run_command(&ctx, cli.command));
        match result {
            Ok(()) => Ok(output),
            Err(e) => {
                if output.trim().is_empty() {
                    Err(e)
                } else {
                    // Keep what the command printed before failing.
                    Err(e.help(format!("output before the error:\n{}", output.trim_end())))
                }
            }
        }
    }
    pub fn project(&self) -> Result<Project> {
        self.store.borrow().project.clone().ok_or_else(|| Error::with_help("no project in this session yet", "run `init <name>` first, or open a project file"))
    }
    pub fn project_json(&self) -> Result<String> {
        serde_json::to_string_pretty(&self.project()?).map_err(|e| Error::msg(format!("could not serialise the project: {e}")))
    }
    pub fn set_project_json(&self, json: &str) -> Result<()> {
        let project: Project = serde_json::from_str(json).map_err(|e| Error::with_help(format!("invalid project JSON: {e}"), "see `pcb schema project`"))?;
        let mut m = self.store.borrow_mut();
        m.project = Some(project);
        Ok(())
    }
    /// Write the project to its path, or to `path` (which becomes its path).
    pub fn save(&self, path: Option<&Path>) -> Result<PathBuf> {
        let project = self.project()?;
        let target = match path {
            Some(p) => {
                let p = if p.is_dir() { p.join(crate::schema::PROJECT_FILENAME) } else { p.to_path_buf() };
                store::absolute(&p)
            }
            None => self.path(),
        };
        if let Some(dir) = target.parent() {
            std::fs::create_dir_all(dir).map_err(|e| Error::io(format!("could not create `{}`", dir.display()), e))?;
        }
        store::write_json(&target, &project)?;
        self.store.borrow_mut().path = target.clone();
        Ok(target)
    }
}

/// Render an error the way the CLI would, without colour.
pub fn render_error(e: &Error) -> String {
    let handler = miette::GraphicalReportHandler::new_themed(miette::GraphicalTheme::unicode_nocolor()).with_width(100);
    let mut s = String::new();
    if handler.render_report(&mut s, e).is_err() {
        s = e.to_string();
    }
    s.trim_end().to_string()
}

/// Run `f` with the process's stdout and stderr redirected into a temporary
/// file, and return what was written. File-descriptor level, so it catches
/// `println!` from anywhere in the crate.
#[cfg(unix)]
fn capture<T>(f: impl FnOnce() -> T) -> (T, String) {
    use std::io::{Read, Seek, Write};
    use std::os::unix::io::AsRawFd;
    let _ = std::io::stdout().flush();
    let _ = std::io::stderr().flush();
    let Ok(mut tmp) = tempfile_in_scratch() else { return (f(), String::new()) };
    let fd = tmp.as_raw_fd();
    let (saved_out, saved_err) = unsafe { (libc::dup(1), libc::dup(2)) };
    unsafe {
        libc::dup2(fd, 1);
        libc::dup2(fd, 2);
    }
    let result = f();
    let _ = std::io::stdout().flush();
    let _ = std::io::stderr().flush();
    unsafe {
        libc::dup2(saved_out, 1);
        libc::dup2(saved_err, 2);
        libc::close(saved_out);
        libc::close(saved_err);
    }
    let mut text = String::new();
    let _ = tmp.seek(std::io::SeekFrom::Start(0));
    let _ = tmp.read_to_string(&mut text);
    (result, text)
}

#[cfg(not(unix))]
fn capture<T>(f: impl FnOnce() -> T) -> (T, String) {
    (f(), String::new())
}

#[cfg(unix)]
fn tempfile_in_scratch() -> std::io::Result<std::fs::File> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir();
    let name = format!("flatland-capture-{}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
    let path = dir.join(name);
    let f = std::fs::OpenOptions::new().read(true).write(true).create_new(true).open(&path)?;
    let _ = std::fs::remove_file(&path); // unlinked: vanishes when closed
    Ok(f)
}
