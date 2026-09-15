//! `pcb serve`: a local web UI for looking at a board and recording
//! incremental changes (moved parts, reshaped or new traces, moved vias,
//! notes) against it. The project file is never written; edits go to the
//! change list (`build/changes.json`, see `pcb changes`) and are applied to
//! an in-memory copy so the page shows the result with live check findings.
//!
//! Plain HTTP over std, one request at a time: the board model is not
//! thread-safe and a local single-user UI does not need more.

use crate::changes::{self, Change, Changes, Entry};
use crate::cli::{Ctx, Loaded, MemStore};
use crate::error::{Error, Result};
use crate::schema::Project;
use crate::store;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::rc::Rc;

const APP: &str = include_str!("serve/app.html");

struct State {
    project_path: PathBuf,
    base: Project,
    base_hash: String,
    changes: Changes,
    /// The changes were recorded against a different project file.
    stale: bool,
    scratch: Project,
    store: Rc<std::cell::RefCell<MemStore>>,
    cached: Option<Value>,
}

impl State {
    fn open(project_path: PathBuf) -> Result<State> {
        let base = store::load_project(&project_path)?;
        let base_hash = store::file_blake3(&project_path)?;
        let changes = changes::load(&project_path)?.unwrap_or(Changes::new(&project_path)?);
        let stale = changes.base_blake3 != base_hash && !changes.changes.is_empty();
        let store = Rc::new(std::cell::RefCell::new(MemStore::new(project_path.clone())));
        let mut s = State { project_path, base, base_hash, changes, stale, scratch: Project::default_like(), store, cached: None };
        s.rebuild_scratch()?;
        Ok(s)
    }

    fn rebuild_scratch(&mut self) -> Result<()> {
        self.scratch = if self.stale { self.base.clone() } else { changes::apply_all(&self.base, &self.changes)? };
        self.cached = None;
        Ok(())
    }

    /// Pick up a project file rewritten on disk (the agent rebuilt the board).
    fn refresh(&mut self) -> Result<bool> {
        let hash = store::file_blake3(&self.project_path)?;
        if hash == self.base_hash {
            return Ok(false);
        }
        self.base = store::load_project(&self.project_path)?;
        self.base_hash = hash;
        // A change list on disk may have been cleared or rewritten meanwhile.
        self.changes = changes::load(&self.project_path)?.unwrap_or(Changes::new(&self.project_path)?);
        self.stale = self.changes.base_blake3 != self.base_hash && !self.changes.changes.is_empty();
        self.rebuild_scratch()?;
        Ok(true)
    }

    fn loaded(&self) -> Result<(Ctx, Loaded)> {
        self.store.borrow_mut().project = Some(self.scratch.clone());
        let ctx = Ctx::in_memory(self.store.clone());
        let loaded = ctx.load()?;
        Ok((ctx, loaded))
    }

    fn payload(&mut self) -> Result<Value> {
        if self.cached.is_none() {
            let (ctx, loaded) = self.loaded()?;
            let board = ctx.board(&loaded)?;
            let scene = crate::scene::scene(&board, &self.project_path)?;
            self.cached = Some(scene);
        }
        let entries: Vec<Value> = self
            .changes
            .changes
            .iter()
            .map(|e| {
                let (cli, py) = changes::as_commands(&e.change);
                json!({"summary": changes::summary(&e.change), "was": e.was, "pcb": cli, "python": py, "change": e.change})
            })
            .collect();
        Ok(json!({
            "hash": self.base_hash,
            "project": self.project_path.display().to_string(),
            "stale": self.stale,
            "changes": entries,
            "scene": self.cached.clone().unwrap(),
        }))
    }

    fn version(&self) -> Value {
        json!({"hash": self.base_hash, "changes": self.changes.changes.len(), "stale": self.stale})
    }

    fn add_change(&mut self, change: Change) -> Result<()> {
        if self.stale {
            return Err(Error::with_help("the pending changes were recorded against an older project file", "clear them first"));
        }
        let was = changes::describe_before(&self.scratch, &change);
        let mut trial = self.scratch.clone();
        changes::apply_one(&mut trial, &change)?;
        // Validate: the board must still build (a trace on a missing layer, say).
        {
            self.store.borrow_mut().project = Some(trial.clone());
            let ctx = Ctx::in_memory(self.store.clone());
            let loaded = ctx.load()?;
            ctx.board(&loaded)?;
        }
        if self.changes.changes.is_empty() {
            self.changes = Changes::new(&self.project_path)?;
        }
        let now = time::OffsetDateTime::now_utc();
        let when = format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", now.year(), now.month() as u8, now.day(), now.hour(), now.minute(), now.second());
        self.changes.changes.push(Entry { change, was, when: Some(when) });
        self.scratch = trial;
        self.cached = None;
        changes::save(&self.project_path, &self.changes)?;
        Ok(())
    }

    fn undo(&mut self) -> Result<()> {
        if self.changes.changes.pop().is_some() {
            if self.changes.changes.is_empty() {
                let p = changes::changes_path(&self.project_path);
                let _ = std::fs::remove_file(p);
            } else {
                changes::save(&self.project_path, &self.changes)?;
            }
        }
        self.rebuild_scratch()
    }

    fn clear(&mut self) -> Result<()> {
        self.changes = Changes::new(&self.project_path)?;
        self.stale = false;
        let p = changes::changes_path(&self.project_path);
        let _ = std::fs::remove_file(p);
        self.rebuild_scratch()
    }

    fn commands_text(&self) -> String {
        let mut s = String::new();
        for (i, e) in self.changes.changes.iter().enumerate() {
            let (cli, py) = changes::as_commands(&e.change);
            s.push_str(&format!("{}. {}\n", i + 1, changes::summary(&e.change)));
            if let Some(w) = &e.was {
                s.push_str(&format!("   was: {w}\n"));
            }
            s.push_str(&format!("   pcb:    {cli}\n   python: {py}\n"));
        }
        s
    }
}

trait DefaultLike {
    fn default_like() -> Project;
}
impl DefaultLike for Project {
    fn default_like() -> Project {
        serde_json::from_value(json!({"schema": "pcb-project/1", "name": ""})).expect("empty project")
    }
}

struct Request {
    method: String,
    path: String,
    body: Vec<u8>,
}

fn read_request(stream: &mut TcpStream) -> std::io::Result<Option<Request>> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    if reader.read_line(&mut line)? == 0 {
        return Ok(None);
    }
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let path = parts.next().unwrap_or("/").to_string();
    let mut content_length = 0usize;
    loop {
        let mut h = String::new();
        if reader.read_line(&mut h)? == 0 {
            break;
        }
        let h = h.trim_end();
        if h.is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            if k.eq_ignore_ascii_case("content-length") {
                content_length = v.trim().parse().unwrap_or(0);
            }
        }
    }
    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        reader.read_exact(&mut body)?;
    }
    Ok(Some(Request { method, path, body }))
}

fn respond(stream: &mut TcpStream, status: &str, content_type: &str, body: &[u8]) {
    let head = format!("HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n", body.len());
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body);
    let _ = stream.flush();
}

fn json_response(stream: &mut TcpStream, r: Result<Value>) {
    match r {
        Ok(v) => respond(stream, "200 OK", "application/json", v.to_string().as_bytes()),
        Err(e) => respond(stream, "400 Bad Request", "application/json", json!({"error": crate::session::render_error(&e)}).to_string().as_bytes()),
    }
}

/// Serve the project at `project_path` on `port` until the process is killed.
pub fn serve(project_path: &Path, port: u16, open: bool) -> Result<()> {
    serve_with(project_path, port, open, |_| {})
}

/// As `serve`, calling `on_ready` with the bound port once listening.
pub fn serve_with(project_path: &Path, port: u16, open: bool, on_ready: impl FnOnce(u16)) -> Result<()> {
    let mut state = State::open(project_path.to_path_buf())?;
    let listener = TcpListener::bind(("127.0.0.1", port)).map_err(|e| Error::io(format!("could not listen on 127.0.0.1:{port}"), e))?;
    let port = listener.local_addr().map(|a| a.port()).unwrap_or(port);
    on_ready(port);
    let url = format!("http://127.0.0.1:{port}/");
    println!("serving {} at {url}", project_path.display());
    println!("edits are recorded in {} (`pcb changes`); the project file is never written", changes::changes_path(project_path).display());
    println!("press Ctrl-C to stop");
    if open {
        let _ = std::process::Command::new(if cfg!(target_os = "macos") { "open" } else { "xdg-open" }).arg(&url).spawn();
    }
    for stream in listener.incoming() {
        let mut stream = match stream {
            Ok(s) => s,
            Err(_) => continue,
        };
        let req = match read_request(&mut stream) {
            Ok(Some(r)) => r,
            _ => continue,
        };
        let path = req.path.split('?').next().unwrap_or("/").to_string();
        match (req.method.as_str(), path.as_str()) {
            ("GET", "/") | ("GET", "/index.html") => {
                // The first board payload rides along in the page: no round trip before the first paint.
                let page = match state.refresh().and_then(|_| state.payload()) {
                    Ok(v) => APP.replacen("<script>", &format!("<script>window.__BOARD__ = {};</script>\n<script>", v.to_string().replace("</", "<\\/")), 1),
                    Err(_) => APP.to_string(),
                };
                respond(&mut stream, "200 OK", "text/html; charset=utf-8", page.as_bytes());
            }
            ("GET", "/api/board") => {
                let r = state.refresh().and_then(|_| state.payload());
                json_response(&mut stream, r);
            }
            ("GET", "/api/version") => {
                let r = state.refresh().map(|_| state.version());
                json_response(&mut stream, r);
            }
            ("GET", "/api/commands") => {
                respond(&mut stream, "200 OK", "text/plain; charset=utf-8", state.commands_text().as_bytes());
            }
            ("POST", "/api/change") => {
                let r = serde_json::from_slice::<Change>(&req.body)
                    .map_err(|e| Error::msg(format!("bad change: {e}")))
                    .and_then(|c| {
                        state.refresh()?;
                        state.add_change(c)
                    })
                    .and_then(|_| state.payload());
                json_response(&mut stream, r);
            }
            ("POST", "/api/undo") => {
                let r = state.refresh().and_then(|_| state.undo()).and_then(|_| state.payload());
                json_response(&mut stream, r);
            }
            ("POST", "/api/clear") => {
                let r = state.refresh().and_then(|_| state.clear()).and_then(|_| state.payload());
                json_response(&mut stream, r);
            }
            _ => respond(&mut stream, "404 Not Found", "text/plain", b"not found"),
        }
    }
    Ok(())
}
