//! A change list: incremental edits made in the UI (`pcb serve`), recorded
//! against a specific `pcb.json` (by hash) so an agent can fold them into
//! its generator script. `pcb changes` prints them, `pcb changes apply`
//! writes the result as `pcb-target.json`, and `pcb diff` tells whether a
//! rebuilt board has reached the target.

use crate::error::{Error, Result};
use crate::schema::{Placement, Project, Side, Trace, Via};
use crate::store;
use crate::units::{Length, Point};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const SCHEMA: &str = "pcb-changes/1";
pub const FILE: &str = "changes.json";
pub const TARGET: &str = "pcb-target.json";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Change {
    /// Place or move a part.
    MovePart {
        refdes: String,
        at: Point,
        rotation: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        side: Option<Side>,
    },
    /// Replace the points (and optionally the width) of trace `trace`.
    SetTracePoints {
        trace: usize,
        points: Vec<Point>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        width: Option<Length>,
    },
    AddTrace {
        layer: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        net: Option<String>,
        width: Length,
        points: Vec<Point>,
    },
    DeleteTrace {
        trace: usize,
    },
    MoveVia {
        via: usize,
        to: Point,
    },
    AddVia {
        at: Point,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        net: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        drill: Option<Length>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        diameter: Option<Length>,
    },
    DeleteVia {
        via: usize,
    },
    /// Free text for the agent, pinned to a point, a part, a net, a trace,
    /// or a sketched path (an intended route).
    Note {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        at: Option<Point>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        refdes: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        net: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        trace: Option<usize>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<Vec<Point>>,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Entry {
    #[serde(flatten)]
    pub change: Change,
    /// What the change replaced, for the reader (`was R3 at 10,8 rot 0`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub was: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Changes {
    pub schema: String,
    /// blake3 of the `pcb.json` these changes were made against.
    pub base_blake3: String,
    /// Its file name, for the reader.
    pub base: String,
    #[serde(default)]
    pub changes: Vec<Entry>,
}

impl Changes {
    pub fn new(project_path: &Path) -> Result<Changes> {
        Ok(Changes {
            schema: SCHEMA.into(),
            base_blake3: store::file_blake3(project_path)?,
            base: project_path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "pcb.json".into()),
            changes: Vec::new(),
        })
    }
}

pub fn changes_path(project_path: &Path) -> PathBuf {
    project_path.parent().unwrap_or(Path::new(".")).join("build").join(FILE)
}

pub fn target_path(project_path: &Path) -> PathBuf {
    project_path.parent().unwrap_or(Path::new(".")).join(TARGET)
}

pub fn load(project_path: &Path) -> Result<Option<Changes>> {
    let p = changes_path(project_path);
    if !p.exists() {
        return Ok(None);
    }
    let c: Changes = store::read_json(&p, "change list")?;
    if c.schema != SCHEMA {
        return Err(Error::msg(format!("{}: schema `{}` is not `{SCHEMA}`", p.display(), c.schema)));
    }
    Ok(Some(c))
}

pub fn save(project_path: &Path, changes: &Changes) -> Result<PathBuf> {
    let p = changes_path(project_path);
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).map_err(|e| Error::io(format!("could not create `{}`", d.display()), e))?;
    }
    store::write_json(&p, changes)?;
    Ok(p)
}

/// Whether `changes` were recorded against the project file as it is now.
pub fn is_current(project_path: &Path, changes: &Changes) -> Result<bool> {
    Ok(store::file_blake3(project_path)? == changes.base_blake3)
}

fn trace_desc(t: &Trace) -> String {
    format!("{} trace on {} w {} {}", t.net.as_deref().unwrap_or("(no net)"), t.layer, t.width, pts(&t.points))
}

pub fn pts(points: &[Point]) -> String {
    points.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(" -> ")
}

/// Describe what a change replaces, before it is applied.
pub fn describe_before(project: &Project, c: &Change) -> Option<String> {
    match c {
        Change::MovePart { refdes, .. } => project.components.get(refdes).map(|i| match &i.placement {
            Some(p) => format!("{refdes} at {} rot {} {}", p.at, p.rotation, p.side),
            None => format!("{refdes} unplaced"),
        }),
        Change::SetTracePoints { trace, .. } | Change::DeleteTrace { trace } => project.traces.get(*trace).map(|t| format!("#{trace}: {}", trace_desc(t))),
        Change::MoveVia { via, .. } | Change::DeleteVia { via } => project.vias.get(*via).map(|v| format!("#{via}: via {} at {}", v.net.as_deref().unwrap_or("(no net)"), v.at)),
        Change::Note { trace: Some(trace), .. } => project.traces.get(*trace).map(|t| format!("#{trace}: {}", trace_desc(t))),
        _ => None,
    }
}

/// Apply one change to the project in place.
pub fn apply_one(project: &mut Project, c: &Change) -> Result<()> {
    match c {
        Change::MovePart { refdes, at, rotation, side } => {
            let inst = project.components.get_mut(refdes).ok_or_else(|| Error::msg(format!("no instance named `{refdes}`")))?;
            let mut p = inst.placement.clone().unwrap_or(Placement { at: Point::ORIGIN, rotation: 0.0, side: Side::Top, locked: false, label_at: None, label_size: None, label_hidden: false });
            p.at = *at;
            p.rotation = *rotation;
            if let Some(s) = side {
                p.side = *s;
            }
            inst.placement = Some(p);
        }
        Change::SetTracePoints { trace, points, width } => {
            let t = project.traces.get_mut(*trace).ok_or_else(|| Error::msg(format!("no trace #{trace}")))?;
            if points.len() < 2 {
                return Err(Error::msg(format!("trace #{trace} needs two points")));
            }
            t.points = points.clone();
            if let Some(w) = width {
                t.width = *w;
            }
        }
        Change::AddTrace { layer, net, width, points } => {
            if points.len() < 2 {
                return Err(Error::msg("a trace needs two points"));
            }
            project.traces.push(Trace { layer: layer.clone(), net: net.clone(), width: *width, points: points.clone(), routed: false });
        }
        Change::DeleteTrace { trace } => {
            if *trace >= project.traces.len() {
                return Err(Error::msg(format!("no trace #{trace}")));
            }
            project.traces.remove(*trace);
        }
        Change::MoveVia { via, to } => {
            let v = project.vias.get_mut(*via).ok_or_else(|| Error::msg(format!("no via #{via}")))?;
            v.at = *to;
        }
        Change::AddVia { at, net, drill, diameter } => {
            let r = &project.design_rules;
            project.vias.push(Via { at: *at, net: net.clone(), drill: drill.unwrap_or(r.via_drill), diameter: diameter.unwrap_or(r.via_diameter), layers: vec![], routed: false });
        }
        Change::DeleteVia { via } => {
            if *via >= project.vias.len() {
                return Err(Error::msg(format!("no via #{via}")));
            }
            project.vias.remove(*via);
        }
        Change::Note { .. } => {}
    }
    Ok(())
}

/// The project with every change applied, in order.
pub fn apply_all(base: &Project, changes: &Changes) -> Result<Project> {
    let mut p = base.clone();
    for (i, e) in changes.changes.iter().enumerate() {
        apply_one(&mut p, &e.change).map_err(|err| Error::msg(format!("change {}: {err}", i + 1)))?;
    }
    Ok(p)
}

fn py_pt(p: Point) -> String {
    format!("({}, {})", p.x.mm(), p.y.mm())
}

fn py_pts(points: &[Point]) -> String {
    format!("[{}]", points.iter().map(|p| py_pt(*p)).collect::<Vec<_>>().join(", "))
}

fn py_str(s: &str) -> String {
    format!("{:?}", s)
}

/// (`pcb` command line, Python call) that reproduce the change on the base
/// board. Trace edits reference the base trace by index and description; an
/// agent maintaining a script replaces the call that created that trace.
pub fn as_commands(c: &Change) -> (String, String) {
    match c {
        Change::MovePart { refdes, at, rotation, side } => {
            let side_cli = side.map(|s| format!(" --side {s}")).unwrap_or_default();
            let side_py = side.map(|s| format!(", side={}", py_str(&s.to_string()))).unwrap_or_default();
            (format!("pcb place {refdes} {at} --rotation {rotation}{side_cli}"), format!("pcb.place({}, {}, rotation={rotation}{side_py})", py_str(refdes), py_pt(*at)))
        }
        Change::SetTracePoints { trace, points, width } => {
            let w_cli = width.map(|w| format!(" --width {}", w.mm())).unwrap_or_default();
            let w_py = width.map(|w| format!(", width={}", w.mm())).unwrap_or_default();
            (format!("# trace #{trace}: replace its points with {}{w_cli}", pts(points)), format!("# trace #{trace}: points={}{w_py}", py_pts(points)))
        }
        Change::AddTrace { layer, net, width, points } => {
            let net_cli = net.as_ref().map(|n| format!(" --net {n}")).unwrap_or_default();
            (
                format!("pcb trace add --layer {layer}{net_cli} --width {} {}", width.mm(), points.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(" ")),
                format!("pcb.trace({}, {}, layer={}, width={})", net.as_deref().map(py_str).unwrap_or_else(|| "None".into()), py_pts(points), py_str(layer), width.mm()),
            )
        }
        Change::DeleteTrace { trace } => (format!("pcb trace remove {trace}"), format!("pcb.run(\"trace\", \"remove\", {trace})")),
        Change::MoveVia { via, to } => (format!("# via #{via}: move to {to}"), format!("# via #{via}: at={}", py_pt(*to))),
        Change::AddVia { at, net, drill, diameter } => {
            let net_cli = net.as_ref().map(|n| format!(" --net {n}")).unwrap_or_default();
            let d_cli = drill.map(|d| format!(" --drill {}", d.mm())).unwrap_or_default();
            let dia_cli = diameter.map(|d| format!(" --diameter {}", d.mm())).unwrap_or_default();
            let net_py = net.as_ref().map(|n| format!(", net={}", py_str(n))).unwrap_or_default();
            let d_py = drill.map(|d| format!(", drill={}", d.mm())).unwrap_or_default();
            let dia_py = diameter.map(|d| format!(", diameter={}", d.mm())).unwrap_or_default();
            (format!("pcb via add{net_cli}{d_cli}{dia_cli} {at}"), format!("pcb.via({}{net_py}{d_py}{dia_py})", py_pt(*at)))
        }
        Change::DeleteVia { via } => (format!("pcb via remove {via}"), format!("pcb.run(\"via\", \"remove\", {via})")),
        Change::Note { text, at, refdes, net, trace, path } => {
            let mut about: Vec<String> = Vec::new();
            if let Some(r) = refdes {
                about.push(format!("part {r}"));
            }
            if let Some(n) = net {
                about.push(format!("net {n}"));
            }
            if let Some(t) = trace {
                about.push(format!("trace #{t}"));
            }
            if let Some(p) = at {
                about.push(format!("at {p}"));
            }
            if let Some(p) = path {
                about.push(format!("along {}", pts(p)));
            }
            let where_ = if about.is_empty() { String::new() } else { format!(" ({})", about.join(", ")) };
            (format!("# note{where_}: {text}"), format!("# note{where_}: {text}"))
        }
    }
}

/// One-line human summary.
pub fn summary(c: &Change) -> String {
    match c {
        Change::MovePart { refdes, at, rotation, side } => format!("move {refdes} to {at} rot {rotation}{}", side.map(|s| format!(" {s}")).unwrap_or_default()),
        Change::SetTracePoints { trace, points, width } => format!("reshape trace #{trace}: {}{}", pts(points), width.map(|w| format!(" w {w}")).unwrap_or_default()),
        Change::AddTrace { layer, net, width, points } => format!("add {} trace on {layer} w {width}: {}", net.as_deref().unwrap_or("(no net)"), pts(points)),
        Change::DeleteTrace { trace } => format!("delete trace #{trace}"),
        Change::MoveVia { via, to } => format!("move via #{via} to {to}"),
        Change::AddVia { at, net, .. } => format!("add via {} at {at}", net.as_deref().unwrap_or("(no net)")),
        Change::DeleteVia { via } => format!("delete via #{via}"),
        Change::Note { text, .. } => format!("note: {text}"),
    }
}
