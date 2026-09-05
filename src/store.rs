//! Loading project/index/component/footprint files, resolving the URLs that
//! link them, and verifying hashes.
//!
//! URL forms accepted today:
//! * `relative/path.json` — relative to the directory of the referencing file;
//! * `/absolute/path.json`, `file:///absolute/path.json`;
//! * `~/path` — home-relative;
//! * `http(s)://…`, `s3://…`, `gs://…` — recognised but not fetched yet; the
//!   error explains how to mirror them locally.

use crate::error::{list_names, suggest, Error, Result};
use crate::schema::*;
use indexmap::IndexMap;
use serde::de::DeserializeOwned;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Files & hashes

pub fn read_text(path: &Path) -> Result<String> {
    std::fs::read_to_string(path).map_err(|e| {
        let help = if e.kind() == std::io::ErrorKind::NotFound {
            Some(format!("expected a file at `{}`", path.display()))
        } else {
            None
        };
        let err = Error::io(format!("could not read `{}`", path.display()), e);
        match help { Some(h) => err.help(h), None => err }
    })
}

pub fn blake3_hex(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

pub fn file_blake3(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path).map_err(|e| Error::io(format!("could not read `{}`", path.display()), e))?;
    Ok(blake3_hex(&bytes))
}

/// Parse JSON with an error that points at the offending line.
pub fn parse_json<T: DeserializeOwned>(path: &Path, text: &str, what: &str) -> Result<T> {
    serde_json::from_str::<T>(text).map_err(|e| {
        Error::at_line_col(path, text, e.line(), e.column(), format!("invalid {what} file: {e}"), "here")
            .help(format!("fix the JSON in `{}`", path.display()))
    })
}

pub fn read_json<T: DeserializeOwned>(path: &Path, what: &str) -> Result<T> {
    let text = read_text(path)?;
    parse_json(path, &text, what)
}

pub fn write_json<T: serde::Serialize>(path: &Path, value: &T) -> Result<()> {
    let mut text = serde_json::to_string_pretty(value).map_err(|e| Error::msg(format!("could not serialise: {e}")))?;
    text.push('\n');
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|e| Error::io(format!("could not create directory `{}`", parent.display()), e))?;
        }
    }
    // Write to a temp file then rename so a crash never leaves a half-written project.
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text).map_err(|e| Error::io(format!("could not write `{}`", tmp.display()), e))?;
    std::fs::rename(&tmp, path).map_err(|e| Error::io(format!("could not write `{}`", path.display()), e))
}

fn check_schema(path: &Path, got: &str, want: &str, what: &str) -> Result<()> {
    if got != want {
        return Err(Error::with_help(
            format!("`{}` is not a {what} file: its `schema` is `{got}`, expected `{want}`", path.display()),
            "check that the URL points at the right kind of file",
        ));
    }
    Ok(())
}

/// Verify a file against an expected blake3 (if any).
pub fn verify_hash(path: &Path, expected: Option<&str>, what: &str) -> Result<()> {
    if let Some(expected) = expected {
        let actual = file_blake3(path)?;
        if !actual.eq_ignore_ascii_case(expected) {
            return Err(Error::with_help(
                format!(
                    "hash mismatch for {what} `{}`\n  expected blake3 {expected}\n  actual   blake3 {actual}",
                    path.display()
                ),
                "the file changed since it was indexed; if that is intended run `pcb index update` \
                 (or `pcb index verify` to list every mismatch)",
            ));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// URLs

/// Where a URL points.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Location {
    Local(PathBuf),
    Remote(String),
}

/// Resolve `url` relative to the file that contains it.
pub fn resolve_url(url: &str, referrer: &Path) -> Location {
    let base = referrer.parent().unwrap_or(Path::new("."));
    if let Some(rest) = url.strip_prefix("file://") {
        return Location::Local(PathBuf::from(rest));
    }
    if let Some(rest) = url.strip_prefix("file:") {
        return Location::Local(base.join(rest));
    }
    if url.contains("://") {
        return Location::Remote(url.to_string());
    }
    if let Some(rest) = url.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return Location::Local(PathBuf::from(home).join(rest));
        }
    }
    let p = Path::new(url);
    if p.is_absolute() {
        Location::Local(p.to_path_buf())
    } else {
        Location::Local(base.join(p))
    }
}

/// Resolve to a local path or explain why we can't.
pub fn local_path(url: &str, referrer: &Path, what: &str) -> Result<PathBuf> {
    match resolve_url(url, referrer) {
        Location::Local(p) => Ok(normalize(&p)),
        Location::Remote(u) => Err(Error::with_help(
            format!("{what} `{u}` is a remote URL, which this version cannot fetch (referenced from `{}`)", referrer.display()),
            "mirror the index locally (e.g. clone it or download the zip) and point the URL at the local copy; \
             remote fetching with hash verification is planned",
        )),
    }
}

/// Collapse `.` and `..` without touching the filesystem.
pub fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                // `..` cancels a preceding real segment; on top of another `..`
                // (or nothing) it must stack, not vanish.
                let top_is_parent = matches!(out.components().next_back(), Some(std::path::Component::ParentDir));
                if top_is_parent || !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other),
        }
    }
    out
}

/// Make `target` relative to the directory of `from_file`, for storing URLs.
pub fn relative_url(target: &Path, from_file: &Path) -> String {
    let target = absolute(target);
    let base = absolute(from_file.parent().unwrap_or(Path::new(".")));
    let t: Vec<_> = target.components().collect();
    let b: Vec<_> = base.components().collect();
    let common = t.iter().zip(b.iter()).take_while(|(a, b)| a == b).count();
    let mut rel = PathBuf::new();
    for _ in common..b.len() {
        rel.push("..");
    }
    for c in &t[common..] {
        rel.push(c);
    }
    let s = rel.to_string_lossy().replace('\\', "/");
    if s.is_empty() { ".".into() } else { s }
}

pub fn absolute(p: &Path) -> PathBuf {
    if p.is_absolute() {
        normalize(p)
    } else {
        normalize(&std::env::current_dir().map(|d| d.join(p)).unwrap_or_else(|_| p.to_path_buf()))
    }
}

// ---------------------------------------------------------------------------
// Project discovery

/// Find the project file: explicit path, `$PCB_PROJECT`, or the nearest
/// `pcb.json` in this or a parent directory.
pub fn find_project(explicit: Option<&Path>) -> Result<PathBuf> {
    if let Some(p) = explicit {
        let p = if p.is_dir() { p.join(PROJECT_FILENAME) } else { p.to_path_buf() };
        if !p.exists() {
            return Err(Error::with_help(
                format!("project file `{}` does not exist", p.display()),
                "run `pcb init` to create one",
            ));
        }
        return Ok(p);
    }
    if let Some(env) = std::env::var_os("PCB_PROJECT") {
        return find_project(Some(Path::new(&env)));
    }
    let cwd = std::env::current_dir().map_err(|e| Error::io("could not determine the current directory", e))?;
    let mut dir = cwd.as_path();
    loop {
        let candidate = dir.join(PROJECT_FILENAME);
        if candidate.is_file() {
            return Ok(candidate);
        }
        match dir.parent() {
            Some(p) => dir = p,
            None => break,
        }
    }
    Err(Error::with_help(
        format!("no `{PROJECT_FILENAME}` found in `{}` or any parent directory", cwd.display()),
        "run `pcb init <name>` to start a project here, or pass `--project <path>`",
    ))
}

pub fn load_project(path: &Path) -> Result<Project> {
    let project: Project = read_json(path, "project")?;
    check_schema(path, &project.schema, PROJECT_SCHEMA, "project")?;
    Ok(project)
}

// ---------------------------------------------------------------------------
// Library: the union of all component indexes a project uses.

#[derive(Clone, Debug)]
pub struct LoadedIndex {
    pub path: PathBuf,
    pub index: ComponentIndex,
}

#[derive(Clone, Debug)]
pub struct LoadedComponent {
    /// Index the component came from.
    pub index_name: String,
    pub name: String,
    pub path: PathBuf,
    pub component: Component,
    pub footprint: Option<LoadedFootprint>,
}

#[derive(Clone, Debug)]
pub struct LoadedFootprint {
    pub path: PathBuf,
    pub footprint: Footprint,
}

impl LoadedComponent {
    pub fn pin(&self, name: &str) -> Result<&Pin> {
        self.component.pins.iter().find(|p| p.name == name).ok_or_else(|| {
            let names: Vec<&str> = self.component.pins.iter().map(|p| p.name.as_str()).collect();
            let mut e = Error::msg(format!(
                "component `{}` has no pin `{name}`; its pins are {}",
                self.name,
                list_names(names.iter().copied())
            ));
            if let Some(s) = suggest(name, names.iter().copied()) {
                e = e.help(s);
            }
            e
        })
    }
    pub fn is_virtual(&self) -> bool {
        self.footprint.is_none()
    }
}

#[derive(Debug, Default)]
pub struct Library {
    pub indexes: Vec<LoadedIndex>,
    cache: std::cell::RefCell<IndexMap<String, LoadedComponent>>,
}

impl Library {
    /// Load every index the project references.
    pub fn load(project: &Project, project_path: &Path) -> Result<Library> {
        let mut lib = Library::default();
        for r in &project.component_indexes {
            let path = local_path(&r.url, project_path, "component index")?;
            let text = read_text(&path).map_err(|e| {
                e.help(format!(
                    "the project lists this index as `{}`; fix the URL in `{}` or remove it with `pcb index remove`",
                    r.url,
                    project_path.display()
                ))
            })?;
            let index: ComponentIndex = parse_json(&path, &text, "component index")?;
            check_schema(&path, &index.schema, INDEX_SCHEMA, "component index")?;
            verify_hash(&path, r.blake3.as_deref(), "component index")?;
            lib.indexes.push(LoadedIndex { path, index });
        }
        Ok(lib)
    }

    pub fn all_component_names(&self) -> Vec<String> {
        let mut v = Vec::new();
        for ix in &self.indexes {
            for name in ix.index.components.keys() {
                if self.indexes.iter().filter(|i| i.index.components.contains_key(name)).count() > 1 {
                    v.push(format!("{}:{}", ix.index.name, name));
                } else {
                    v.push(name.clone());
                }
            }
        }
        v
    }

    /// Resolve `name` or `index:name` to a component.
    pub fn component(&self, reference: &str) -> Result<LoadedComponent> {
        if let Some(c) = self.cache.borrow().get(reference) {
            return Ok(c.clone());
        }
        let c = self.load_component(reference)?;
        self.cache.borrow_mut().insert(reference.to_string(), c.clone());
        Ok(c)
    }

    fn load_component(&self, reference: &str) -> Result<LoadedComponent> {
        if self.indexes.is_empty() {
            return Err(Error::with_help(
                format!("cannot look up component `{reference}`: the project has no component indexes"),
                "add one with `pcb index add <path-to-index.json>` (or create one with `pcb index new`)",
            ));
        }
        let (index_filter, name) = match reference.split_once(':') {
            Some((ix, n)) => (Some(ix), n),
            None => (None, reference),
        };
        let mut hits: Vec<&LoadedIndex> = self
            .indexes
            .iter()
            .filter(|ix| index_filter.map_or(true, |f| ix.index.name == f))
            .filter(|ix| ix.index.components.contains_key(name))
            .collect();
        if let Some(f) = index_filter {
            if !self.indexes.iter().any(|ix| ix.index.name == f) {
                let names: Vec<&str> = self.indexes.iter().map(|i| i.index.name.as_str()).collect();
                let mut e = Error::msg(format!(
                    "no component index named `{f}` (looking for `{reference}`); indexes are {}",
                    list_names(names.iter().copied())
                ));
                if let Some(s) = suggest(f, names.iter().copied()) {
                    e = e.help(s);
                }
                return Err(e);
            }
        }
        if hits.is_empty() {
            let all = self.all_component_names();
            let mut e = Error::msg(format!(
                "no component named `{name}` in {}; available components: {}",
                if let Some(f) = index_filter { format!("index `{f}`") } else { "any index".into() },
                list_names(all.iter().map(|s| s.as_str()))
            ));
            e = match suggest(name, all.iter().map(|s| s.as_str())) {
                Some(s) => e.help(s),
                None => e.help("run `pcb component list` to see everything available"),
            };
            return Err(e);
        }
        if hits.len() > 1 {
            let names: Vec<String> = hits.iter().map(|h| format!("`{}:{name}`", h.index.name)).collect();
            return Err(Error::with_help(
                format!("component `{name}` exists in several indexes: {}", names.join(", ")),
                "qualify it with the index name, e.g. `pcb add R1 basic:resistor-0603`",
            ));
        }
        let ix = hits.remove(0);
        let entry = &ix.index.components[name];
        let path = local_path(&entry.file.url, &ix.path, "component file")?;
        let text = read_text(&path).map_err(|e| {
            e.help(format!("index `{}` (`{}`) lists `{name}` at `{}`", ix.index.name, ix.path.display(), entry.file.url))
        })?;
        let component: Component = parse_json(&path, &text, "component")?;
        check_schema(&path, &component.schema, COMPONENT_SCHEMA, "component")?;
        verify_hash(&path, entry.file.blake3.as_deref(), "component file")?;
        validate_component(&component, &path)?;

        let footprint = match &component.footprint {
            Some(fr) => {
                let fp_path = local_path(&fr.url, &path, "footprint file")?;
                let text = read_text(&fp_path)
                    .map_err(|e| e.help(format!("component `{name}` (`{}`) references footprint `{}`", path.display(), fr.url)))?;
                let footprint: Footprint = parse_json(&fp_path, &text, "footprint")?;
                check_schema(&fp_path, &footprint.schema, FOOTPRINT_SCHEMA, "footprint")?;
                verify_hash(&fp_path, fr.blake3.as_deref(), "footprint file")?;
                validate_footprint(&footprint, &fp_path)?;
                // Every pin must land on a real pad.
                for pin in &component.pins {
                    for pad_name in pin.pad_names() {
                        if !footprint.pads.iter().any(|p| p.name == pad_name) {
                            let pads: Vec<&str> = footprint.pads.iter().map(|p| p.name.as_str()).collect();
                            return Err(Error::with_help(
                                format!(
                                    "pin `{}` of component `{name}` maps to pad `{}` but footprint `{}` has pads {}",
                                    pin.name,
                                    pad_name,
                                    footprint.name,
                                    list_names(pads.iter().copied())
                                ),
                                format!("fix the `pad` of that pin in `{}` or the pad names in `{}`", path.display(), fp_path.display()),
                            ));
                        }
                    }
                }
                Some(LoadedFootprint { path: fp_path, footprint })
            }
            None => None,
        };
        Ok(LoadedComponent { index_name: ix.index.name.clone(), name: name.to_string(), path, component, footprint })
    }
}

pub fn validate_component(c: &Component, path: &Path) -> Result<()> {
    let mut seen = std::collections::HashSet::new();
    for pin in &c.pins {
        if pin.name.is_empty() || pin.name.contains('.') || pin.name.contains(char::is_whitespace) {
            return Err(Error::msg(format!(
                "component `{}` (`{}`) has an invalid pin name `{}`: pin names must be non-empty and contain no `.` or spaces",
                c.name, path.display(), pin.name
            )));
        }
        if !seen.insert(pin.name.as_str()) {
            return Err(Error::msg(format!("component `{}` (`{}`) lists pin `{}` twice", c.name, path.display(), pin.name)));
        }
    }
    if let Some(sp) = &c.spice {
        if sp.template.trim().is_empty() {
            return Err(Error::msg(format!("component `{}` (`{}`) has an empty spice template", c.name, path.display())));
        }
    }
    Ok(())
}

fn validate_footprint(f: &Footprint, path: &Path) -> Result<()> {
    for pad in &f.pads {
        if pad.pad_type == PadType::ThroughHole && pad.drill.is_none() {
            return Err(Error::msg(format!(
                "footprint `{}` (`{}`): through-hole pad `{}` has no `drill` diameter",
                f.name, path.display(), pad.name
            )));
        }
        if pad.size[0].0 <= 0 || pad.size[1].0 <= 0 {
            return Err(Error::msg(format!(
                "footprint `{}` (`{}`): pad `{}` has a non-positive size",
                f.name, path.display(), pad.name
            )));
        }
        if let Some(d) = pad.drill {
            if d >= pad.size[0] && d >= pad.size[1] && pad.is_plated() {
                return Err(Error::msg(format!(
                    "footprint `{}` (`{}`): plated pad `{}` drill {} is not smaller than its copper size {}x{}",
                    f.name, path.display(), pad.name, d, pad.size[0], pad.size[1]
                )));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn urls() {
        let r = Path::new("/a/b/index.json");
        assert_eq!(resolve_url("c/d.json", r), Location::Local(PathBuf::from("/a/b/c/d.json")));
        assert_eq!(resolve_url("../x.json", r), Location::Local(PathBuf::from("/a/b/../x.json")));
        assert_eq!(resolve_url("file:///tmp/x.json", r), Location::Local(PathBuf::from("/tmp/x.json")));
        assert!(matches!(resolve_url("https://x/y.json", r), Location::Remote(_)));
        assert_eq!(relative_url(Path::new("/a/b/c/d.json"), Path::new("/a/b/index.json")), "c/d.json");
        assert_eq!(relative_url(Path::new("/a/x.json"), Path::new("/a/b/index.json")), "../x.json");
    }
}
