//! Project-editing commands.

use super::{validate, Ctx, Loaded};
use crate::error::{list_names, suggest, Error, Result};
use crate::geom;
use crate::geom::{Edge, Ring, Transform};
use crate::model::parse_pin_ref;
use crate::schema::*;
use crate::store;
use crate::units::{parse_degrees, Length, Point};
use clap::{Args, Subcommand};
use indexmap::IndexMap;
use std::path::PathBuf;

fn point(s: &str) -> std::result::Result<Point, Error> {
    Point::parse(s)
}
fn length(s: &str) -> std::result::Result<Length, Error> {
    Length::parse(s)
}
fn degrees(s: &str) -> std::result::Result<f64, Error> {
    parse_degrees(s)
}

fn parse_kv(s: &str) -> std::result::Result<(String, String), Error> {
    match s.split_once('=') {
        Some((k, v)) if !k.trim().is_empty() => Ok((k.trim().to_string(), v.trim().to_string())),
        _ => Err(Error::with_help(format!("`{s}` is not a `key=value` pair"), "e.g. `value=10k`")),
    }
}

// ---------------------------------------------------------------------------

#[derive(Args)]
pub struct InitArgs {
    /// Board name.
    pub name: String,
    /// Directory to create the project in (default: current directory).
    #[arg(short, long)]
    pub dir: Option<PathBuf>,
    /// Copper layers, top to bottom (default `F.Cu,B.Cu`; use `B.Cu` for single-sided).
    #[arg(short, long, value_delimiter = ',')]
    pub layers: Option<Vec<String>>,
    /// Component index files to use (repeatable).
    #[arg(short, long = "index")]
    pub indexes: Vec<PathBuf>,
    /// Overwrite an existing project file.
    #[arg(long)]
    pub force: bool,
}

pub fn init(_ctx: &Ctx, a: InitArgs) -> Result<()> {
    let dir = a.dir.unwrap_or_else(|| PathBuf::from("."));
    std::fs::create_dir_all(&dir).map_err(|e| Error::io(format!("could not create `{}`", dir.display()), e))?;
    let path = dir.join(PROJECT_FILENAME);
    if path.exists() && !a.force {
        return Err(Error::with_help(
            format!("`{}` already exists", path.display()),
            "pass --force to overwrite it, or choose another --dir",
        ));
    }
    let mut project = Project::new(&a.name);
    if let Some(layers) = a.layers {
        let layers: Vec<String> = layers.into_iter().filter(|l| !l.trim().is_empty()).collect();
        if layers.is_empty() {
            return Err(Error::msg("--layers needs at least one layer name"));
        }
        project.stackup.copper_layers = layers;
    }
    for ix in &a.indexes {
        let ip = if ix.is_dir() { ix.join("index.json") } else { ix.clone() };
        let _: ComponentIndex = store::read_json(&ip, "component index")?;
        project.component_indexes.push(FileRef::new(store::relative_url(&ip, &path)));
    }
    store::write_json(&path, &project)?;
    println!("created {} ({} copper layer(s): {})", path.display(), project.stackup.copper_layers.len(), project.stackup.copper_layers.join(", "));
    if project.component_indexes.is_empty() {
        println!("next: add a component index with `pcb index add <index.json>` or create one with `pcb index new <dir>`");
    }
    Ok(())
}

// ---------------------------------------------------------------------------

#[derive(Args)]
pub struct AddArgs {
    /// Instance name (reference designator), e.g. `R1`.
    pub refdes: String,
    /// Component name, optionally qualified: `resistor-0603` or `basic:resistor-0603`.
    pub component: String,
    /// Instance parameters, e.g. `value=10k` (repeatable).
    #[arg(short = 'P', long = "param", value_parser = parse_kv)]
    pub params: Vec<(String, String)>,
    /// Place immediately at `x,y`.
    #[arg(long, value_parser = point)]
    pub at: Option<Point>,
    #[arg(short, long, value_parser = degrees, default_value = "0", allow_negative_numbers = true)]
    pub rotation: f64,
    #[arg(short, long, value_enum, default_value_t = Side::Top)]
    pub side: Side,
    /// Free-form note shown in visualisations.
    #[arg(long)]
    pub note: Option<String>,
}

fn check_refdes(name: &str) -> Result<()> {
    if name.is_empty() || name.contains('.') || name.contains(char::is_whitespace) || name.contains(':') {
        return Err(Error::with_help(
            format!("`{name}` is not a valid instance name"),
            "instance names must be non-empty with no `.`, `:` or spaces, e.g. `R1`, `U3`, `J_PWR`",
        ));
    }
    Ok(())
}

pub fn add(ctx: &Ctx, a: AddArgs) -> Result<()> {
    check_refdes(&a.refdes)?;
    let mut loaded = ctx.load()?;
    if loaded.project.components.contains_key(&a.refdes) {
        return Err(Error::with_help(
            format!("an instance named `{}` already exists", a.refdes),
            format!("use another name, `pcb set {} ...` to change it, or `pcb remove {}` first", a.refdes, a.refdes),
        ));
    }
    let lib = ctx.library(&loaded)?;
    let comp = lib.component(&a.component)?;
    let mut parameters = IndexMap::new();
    for (k, v) in a.params {
        if !comp.component.parameters.contains_key(&k) {
            let names: Vec<&str> = comp.component.parameters.keys().map(|s| s.as_str()).collect();
            let mut e = Error::msg(format!(
                "component `{}` has no parameter `{k}`; its parameters are {}",
                comp.name,
                list_names(names.iter().copied())
            ));
            if let Some(s) = suggest(&k, names.iter().copied()) {
                e = e.help(s);
            }
            return Err(e);
        }
        parameters.insert(k, v);
    }
    // Parameters without a default must be supplied.
    let missing: Vec<&str> = comp
        .component
        .parameters
        .iter()
        .filter(|(k, p)| p.default.is_none() && !parameters.contains_key(*k))
        .map(|(k, _)| k.as_str())
        .collect();
    if !missing.is_empty() {
        return Err(Error::with_help(
            format!("component `{}` requires parameter(s) {}", comp.name, list_names(missing.iter().copied())),
            format!("pass them with `--param {}=<value>`", missing[0]),
        ));
    }
    let placement = a.at.map(|at| Placement { at, rotation: a.rotation, side: a.side, locked: false, label_at: None, label_size: None, label_hidden: false });
    loaded.project.components.insert(
        a.refdes.clone(),
        Instance { component: a.component.clone(), parameters, placement, note: a.note },
    );
    validate(ctx, &loaded)?;
    loaded.save()?;
    let pins: Vec<&str> = comp.component.pins.iter().map(|p| p.name.as_str()).collect();
    println!(
        "added {} = {} ({}){}",
        a.refdes,
        comp.name,
        if comp.is_virtual() { "virtual, no footprint".to_string() } else { format!("footprint {}", comp.footprint.as_ref().unwrap().footprint.name) },
        if pins.is_empty() { String::new() } else { format!("; pins: {}", pins.join(", ")) }
    );
    Ok(())
}

#[derive(Args)]
pub struct RemoveArgs {
    pub refdes: String,
}

pub fn remove(ctx: &Ctx, a: RemoveArgs) -> Result<()> {
    let mut loaded = ctx.load()?;
    if loaded.project.components.shift_remove(&a.refdes).is_none() {
        return Err(unknown_instance(&loaded.project, &a.refdes));
    }
    let prefix = format!("{}.", a.refdes);
    let mut emptied = Vec::new();
    for (name, net) in loaded.project.nets.iter_mut() {
        net.pins.retain(|p| !p.starts_with(&prefix));
        if net.pins.is_empty() {
            emptied.push(name.clone());
        }
    }
    for n in &emptied {
        loaded.project.nets.shift_remove(n);
    }
    loaded.save()?;
    println!("removed {}{}", a.refdes, if emptied.is_empty() { String::new() } else { format!("; dropped now-empty nets {}", emptied.join(", ")) });
    Ok(())
}

fn unknown_instance(project: &Project, refdes: &str) -> Error {
    let names: Vec<&str> = project.components.keys().map(|s| s.as_str()).collect();
    let mut e = Error::msg(format!("no instance named `{refdes}`; instances are {}", list_names(names.iter().copied())));
    e = match suggest(refdes, names.iter().copied()) {
        Some(s) => e.help(s),
        None => e.help("add one with `pcb add <refdes> <component>`"),
    };
    e
}

#[derive(Args)]
pub struct SetArgs {
    pub refdes: String,
    /// `param=value` pairs; `note=...` sets the display note; `component=...` swaps the part.
    #[arg(required = true, value_parser = parse_kv)]
    pub assignments: Vec<(String, String)>,
}

pub fn set(ctx: &Ctx, a: SetArgs) -> Result<()> {
    let mut loaded = ctx.load()?;
    if !loaded.project.components.contains_key(&a.refdes) {
        return Err(unknown_instance(&loaded.project, &a.refdes));
    }
    let lib = ctx.library(&loaded)?;
    for (k, v) in a.assignments {
        let inst = loaded.project.components.get_mut(&a.refdes).unwrap();
        match k.as_str() {
            "note" => inst.note = if v.is_empty() { None } else { Some(v) },
            "component" => {
                lib.component(&v)?;
                inst.component = v;
            }
            _ => {
                let comp = lib.component(&inst.component)?;
                if !comp.component.parameters.contains_key(&k) {
                    let names: Vec<&str> = comp.component.parameters.keys().map(|s| s.as_str()).collect();
                    let mut e = Error::msg(format!(
                        "component `{}` has no parameter `{k}`; its parameters are {} (plus `note` and `component`)",
                        comp.name,
                        list_names(names.iter().copied())
                    ));
                    if let Some(s) = suggest(&k, names.iter().copied()) {
                        e = e.help(s);
                    }
                    return Err(e);
                }
                inst.parameters.insert(k, v);
            }
        }
    }
    validate(ctx, &loaded)?;
    loaded.save()?;
    println!("updated {}", a.refdes);
    Ok(())
}

// ---------------------------------------------------------------------------
// Nets

#[derive(Args)]
pub struct ConnectArgs {
    /// Pins to join, `<instance>.<pin>` (at least two, or one with --net).
    #[arg(required = true)]
    pub pins: Vec<String>,
    /// Name for the resulting net (default: reuse an existing name or auto-generate `N$<n>`).
    #[arg(short, long)]
    pub net: Option<String>,
    /// Allow joining two differently named nets.
    #[arg(long)]
    pub merge: bool,
}

fn check_net_name(name: &str) -> Result<()> {
    if name.is_empty() || name.contains(char::is_whitespace) || name.contains('"') || name.contains('(') || name.contains(')') {
        return Err(Error::with_help(
            format!("`{name}` is not a valid net name"),
            "net names must be non-empty with no spaces, quotes or parentheses, e.g. `GND`, `VCC_3V3`",
        ));
    }
    Ok(())
}

fn resolve_pin(loaded: &Loaded, lib: &store::Library, pin_ref: &str) -> Result<String> {
    let (refdes, pin) = parse_pin_ref(pin_ref)?;
    let inst = loaded.project.components.get(refdes).ok_or_else(|| unknown_instance(&loaded.project, refdes))?;
    let comp = lib.component(&inst.component)?;
    comp.pin(pin).map_err(|e| e.help(format!("(pin reference `{pin_ref}`)")))?;
    Ok(format!("{refdes}.{pin}"))
}

fn fresh_net_name(project: &mut Project) -> String {
    loop {
        project.next_net_id += 1;
        let name = format!("N${}", project.next_net_id);
        if !project.nets.contains_key(&name) {
            return name;
        }
    }
}

pub fn connect(ctx: &Ctx, a: ConnectArgs) -> Result<()> {
    let mut loaded = ctx.load()?;
    let lib = ctx.library(&loaded)?;
    if a.pins.len() < 2 && a.net.is_none() {
        return Err(Error::with_help("connect needs at least two pins", "e.g. `pcb connect R1.1 U1.VCC` (or one pin with `--net GND`)"));
    }
    let mut pins = Vec::new();
    for p in &a.pins {
        pins.push(resolve_pin(&loaded, &lib, p)?);
    }
    if let Some(n) = &a.net {
        check_net_name(n)?;
    }
    // Nets the pins already belong to.
    let mut existing: Vec<String> = Vec::new();
    for p in &pins {
        for (name, net) in &loaded.project.nets {
            if net.pins.contains(p) && !existing.contains(name) {
                existing.push(name.clone());
            }
        }
    }
    if let Some(n) = &a.net {
        if loaded.project.nets.contains_key(n) && !existing.contains(n) {
            existing.push(n.clone());
        }
    }
    let named: Vec<&String> = existing.iter().filter(|n| !n.starts_with("N$")).collect();
    if named.len() > 1 && !a.merge {
        return Err(Error::with_help(
            format!("these pins span different named nets: {}", list_names(named.iter().map(|s| s.as_str()))),
            format!("pass --merge to join them (the result is called `{}`), or check the pin references", a.net.as_deref().unwrap_or(named[0])),
        ));
    }
    let target = match (&a.net, named.first()) {
        (Some(n), _) => n.clone(),
        (None, Some(n)) => (*n).clone(),
        (None, None) => match existing.first() {
            Some(n) => n.clone(),
            None => fresh_net_name(&mut loaded.project),
        },
    };
    // Merge everything into target.
    let mut all_pins: Vec<String> = Vec::new();
    for n in &existing {
        if let Some(net) = loaded.project.nets.get(n) {
            for p in &net.pins {
                if !all_pins.contains(p) {
                    all_pins.push(p.clone());
                }
            }
        }
    }
    let mut class = None;
    for n in &existing {
        if let Some(net) = loaded.project.nets.get(n) {
            if net.class.is_some() {
                class = net.class.clone();
            }
        }
        if *n != target {
            loaded.project.nets.shift_remove(n);
            rename_net_refs(&mut loaded.project, n, &target);
        }
    }
    for p in &pins {
        if !all_pins.contains(p) {
            all_pins.push(p.clone());
        }
    }
    let entry = loaded.project.nets.entry(target.clone()).or_default();
    entry.pins = all_pins;
    if class.is_some() {
        entry.class = class;
    }
    let count = entry.pins.len();
    validate(ctx, &loaded)?;
    loaded.save()?;
    let merged: Vec<&String> = existing.iter().filter(|n| **n != target).collect();
    println!(
        "net {target}: {} pin(s){}",
        count,
        if merged.is_empty() { String::new() } else { format!(" (merged {})", merged.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")) }
    );
    Ok(())
}

fn rename_net_refs(project: &mut Project, old: &str, new: &str) {
    for t in &mut project.traces {
        if t.net.as_deref() == Some(old) {
            t.net = Some(new.to_string());
        }
    }
    for v in &mut project.vias {
        if v.net.as_deref() == Some(old) {
            v.net = Some(new.to_string());
        }
    }
    for p in &mut project.pours {
        if p.net.as_deref() == Some(old) {
            p.net = Some(new.to_string());
        }
    }
    for h in &mut project.holes {
        if h.net.as_deref() == Some(old) {
            h.net = Some(new.to_string());
        }
    }
}

#[derive(Args)]
pub struct DisconnectArgs {
    #[arg(required = true)]
    pub pins: Vec<String>,
}

pub fn disconnect(ctx: &Ctx, a: DisconnectArgs) -> Result<()> {
    let mut loaded = ctx.load()?;
    for p in &a.pins {
        parse_pin_ref(p)?;
        let mut found = None;
        for (name, net) in loaded.project.nets.iter_mut() {
            if let Some(i) = net.pins.iter().position(|x| x == p) {
                net.pins.remove(i);
                found = Some(name.clone());
            }
        }
        match found {
            Some(n) => {
                if loaded.project.nets[&n].pins.is_empty() {
                    loaded.project.nets.shift_remove(&n);
                    println!("removed {p} from {n} (net is now empty and was dropped)");
                } else {
                    println!("removed {p} from {n}");
                }
            }
            None => return Err(Error::msg(format!("pin `{p}` is not connected to any net"))),
        }
    }
    loaded.save()
}

#[derive(Subcommand)]
pub enum NetCmd {
    /// List nets and their pins.
    List,
    /// Show one net.
    Show { name: String },
    /// Rename a net.
    Rename { old: String, new: String },
    /// Assign a net class (`-` to clear).
    Class { net: String, class: String },
    /// Delete a net (pins become unconnected).
    Remove { name: String },
}

pub fn run_net(ctx: &Ctx, c: NetCmd) -> Result<()> {
    let mut loaded = ctx.load()?;
    match c {
        NetCmd::List => {
            if loaded.project.nets.is_empty() {
                println!("no nets yet; create them with `pcb connect <pin> <pin>`");
            }
            for (name, net) in &loaded.project.nets {
                println!(
                    "{name}{}: {}",
                    net.class.as_ref().map(|c| format!(" [{c}]")).unwrap_or_default(),
                    net.pins.join(" ")
                );
            }
            Ok(())
        }
        NetCmd::Show { name } => {
            let net = loaded.project.nets.get(&name).ok_or_else(|| unknown_net(&loaded.project, &name))?;
            println!("{name}: {} pin(s)", net.pins.len());
            for p in &net.pins {
                println!("  {p}");
            }
            let traces = loaded.project.traces.iter().filter(|t| t.net.as_deref() == Some(&name)).count();
            let vias = loaded.project.vias.iter().filter(|v| v.net.as_deref() == Some(&name)).count();
            let pours: Vec<&str> = loaded.project.pours.iter().filter(|p| p.net.as_deref() == Some(&name)).map(|p| p.name.as_str()).collect();
            println!("  traces: {traces}, vias: {vias}, pours: {}", if pours.is_empty() { "-".into() } else { pours.join(", ") });
            Ok(())
        }
        NetCmd::Rename { old, new } => {
            check_net_name(&new)?;
            if loaded.project.nets.contains_key(&new) {
                return Err(Error::with_help(format!("a net named `{new}` already exists"), "use `pcb connect --merge` to join nets"));
            }
            let idx = loaded.project.nets.get_index_of(&old).ok_or_else(|| unknown_net(&loaded.project, &old))?;
            let (_, net) = loaded.project.nets.shift_remove_index(idx).unwrap();
            loaded.project.nets.shift_insert(idx, new.clone(), net);
            rename_net_refs(&mut loaded.project, &old, &new);
            loaded.save()?;
            println!("renamed {old} -> {new}");
            Ok(())
        }
        NetCmd::Class { net, class } => {
            if !loaded.project.nets.contains_key(&net) {
                return Err(unknown_net(&loaded.project, &net));
            }
            if class != "-" && !loaded.project.design_rules.net_classes.contains_key(&class) {
                let names: Vec<&str> = loaded.project.design_rules.net_classes.keys().map(|s| s.as_str()).collect();
                return Err(Error::with_help(
                    format!("no net class `{class}`; classes are {}", list_names(names.iter().copied())),
                    format!("define it with `pcb rules class {class} --width <w> --clearance <c>`"),
                ));
            }
            loaded.project.nets[&net].class = if class == "-" { None } else { Some(class) };
            loaded.save()
        }
        NetCmd::Remove { name } => {
            if loaded.project.nets.shift_remove(&name).is_none() {
                return Err(unknown_net(&loaded.project, &name));
            }
            loaded.project.traces.retain(|t| t.net.as_deref() != Some(&name));
            loaded.project.vias.retain(|v| v.net.as_deref() != Some(&name));
            for p in &mut loaded.project.pours {
                if p.net.as_deref() == Some(&name) {
                    p.net = None;
                }
            }
            loaded.save()?;
            println!("removed net {name}");
            Ok(())
        }
    }
}

fn unknown_net(project: &Project, name: &str) -> Error {
    let names: Vec<&str> = project.nets.keys().map(|s| s.as_str()).collect();
    let mut e = Error::msg(format!("no net named `{name}`; nets are {}", list_names(names.iter().copied())));
    if let Some(s) = suggest(name, names.iter().copied()) {
        e = e.help(s);
    }
    e
}

// ---------------------------------------------------------------------------
// Outline / pour edges

#[derive(Subcommand)]
pub enum EdgeCmd {
    /// Straight edge from `x1,y1` to `x2,y2`.
    Line {
        #[arg(value_parser = point)]
        from: Point,
        #[arg(value_parser = point)]
        to: Point,
    },
    /// Circular arc from `x1,y1` to `x2,y2`; give `--center` (with `--cw` for
    /// clockwise) or `--through x,y` for a point on the arc.
    Arc {
        #[arg(value_parser = point)]
        from: Point,
        #[arg(value_parser = point)]
        to: Point,
        #[arg(long, value_parser = point, conflicts_with = "through")]
        center: Option<Point>,
        #[arg(long, value_parser = point)]
        through: Option<Point>,
        /// Clockwise (only with --center).
        #[arg(long)]
        cw: bool,
    },
}

impl EdgeCmd {
    pub fn to_edge(&self) -> Result<Edge> {
        match self {
            EdgeCmd::Line { from, to } => {
                if from == to {
                    return Err(Error::msg("a line edge needs two distinct points"));
                }
                Ok(Edge::Line { from: *from, to: *to })
            }
            EdgeCmd::Arc { from, to, center, through, cw } => {
                if let Some(c) = center {
                    let r0 = from.distance(*c);
                    let r1 = to.distance(*c);
                    if (r0 - r1).abs() > Length::from_mm(0.01) {
                        return Err(Error::with_help(
                            format!("arc endpoints are not equidistant from the center ({r0} vs {r1})"),
                            "check the center, or use `--through x,y` to specify a point on the arc instead",
                        ));
                    }
                    return Ok(Edge::Arc { from: *from, to: *to, center: *c, clockwise: *cw });
                }
                let m = through.ok_or_else(|| Error::with_help("an arc needs `--center x,y` or `--through x,y`", "e.g. `pcb outline add arc 0,0 10,0 --through 5,5`"))?;
                arc_from_three_points(*from, m, *to)
            }
        }
    }
}

/// Circumcircle of three points, orientation from the middle point.
pub fn arc_from_three_points(a: Point, m: Point, b: Point) -> Result<Edge> {
    let (ax, ay) = (a.x.nm() as f64, a.y.nm() as f64);
    let (mx, my) = (m.x.nm() as f64, m.y.nm() as f64);
    let (bx, by) = (b.x.nm() as f64, b.y.nm() as f64);
    let d = 2.0 * (ax * (my - by) + mx * (by - ay) + bx * (ay - my));
    if d.abs() < 1e-3 {
        return Err(Error::msg(format!("points {a}, {m}, {b} are collinear; they do not define an arc")));
    }
    let ux = ((ax * ax + ay * ay) * (my - by) + (mx * mx + my * my) * (by - ay) + (bx * bx + by * by) * (ay - my)) / d;
    let uy = ((ax * ax + ay * ay) * (bx - mx) + (mx * mx + my * my) * (ax - bx) + (bx * bx + by * by) * (mx - ax)) / d;
    let center = Point::nm(ux.round() as i64, uy.round() as i64);
    // Orientation: cross product of (m-a) x (b-a) > 0 means ccw.
    let cross = (mx - ax) * (by - ay) - (my - ay) * (bx - ax);
    Ok(Edge::Arc { from: a, to: b, center, clockwise: cross < 0.0 })
}

#[derive(Subcommand)]
pub enum OutlineCmd {
    /// Append an edge (`line` or `arc`) to the outline.
    #[command(subcommand)]
    Add(EdgeCmd),
    /// Replace the outline with a rectangle of the given size.
    Rect {
        #[arg(value_parser = length)]
        width: Length,
        #[arg(value_parser = length)]
        height: Length,
        /// Bottom-left corner (default `0,0`).
        #[arg(long, value_parser = point, default_value = "0,0")]
        at: Point,
        /// Corner radius for rounded rectangles.
        #[arg(long, value_parser = length)]
        radius: Option<Length>,
    },
    /// Replace the outline with a circle.
    Circle {
        #[arg(value_parser = length)]
        diameter: Length,
        #[arg(long, value_parser = point, default_value = "0,0")]
        center: Point,
    },
    /// Remove the last edge.
    Pop,
    /// Remove all edges.
    Clear,
    /// Print the edges.
    Show,
}

/// Edges of a (rounded) rectangle, counter-clockwise from the bottom-left.
pub fn rect_edges(at: Point, w: Length, h: Length, radius: Option<Length>) -> Result<Vec<Edge>> {
    if w.0 <= 0 || h.0 <= 0 {
        return Err(Error::msg("width and height must be positive"));
    }
    let (x0, y0, x1, y1) = (at.x, at.y, at.x + w, at.y + h);
    let r = radius.unwrap_or(Length::ZERO);
    if r.0 <= 0 {
        return Ok(vec![
            Edge::Line { from: Point::new(x0, y0), to: Point::new(x1, y0) },
            Edge::Line { from: Point::new(x1, y0), to: Point::new(x1, y1) },
            Edge::Line { from: Point::new(x1, y1), to: Point::new(x0, y1) },
            Edge::Line { from: Point::new(x0, y1), to: Point::new(x0, y0) },
        ]);
    }
    if r * 2 > w || r * 2 > h {
        return Err(Error::msg(format!("corner radius {r} is too large for a {w} x {h} rectangle")));
    }
    Ok(vec![
        Edge::Line { from: Point::new(x0 + r, y0), to: Point::new(x1 - r, y0) },
        Edge::Arc { from: Point::new(x1 - r, y0), to: Point::new(x1, y0 + r), center: Point::new(x1 - r, y0 + r), clockwise: false },
        Edge::Line { from: Point::new(x1, y0 + r), to: Point::new(x1, y1 - r) },
        Edge::Arc { from: Point::new(x1, y1 - r), to: Point::new(x1 - r, y1), center: Point::new(x1 - r, y1 - r), clockwise: false },
        Edge::Line { from: Point::new(x1 - r, y1), to: Point::new(x0 + r, y1) },
        Edge::Arc { from: Point::new(x0 + r, y1), to: Point::new(x0, y1 - r), center: Point::new(x0 + r, y1 - r), clockwise: false },
        Edge::Line { from: Point::new(x0, y1 - r), to: Point::new(x0, y0 + r) },
        Edge::Arc { from: Point::new(x0, y0 + r), to: Point::new(x0 + r, y0), center: Point::new(x0 + r, y0 + r), clockwise: false },
    ])
}

pub fn circle_edges(center: Point, d: Length) -> Vec<Edge> {
    let r = d / 2;
    let a = Point::new(center.x + r, center.y);
    let b = Point::new(center.x - r, center.y);
    vec![
        Edge::Arc { from: a, to: b, center, clockwise: false },
        Edge::Arc { from: b, to: a, center, clockwise: false },
    ]
}

fn print_edges(edges: &[Edge]) {
    if edges.is_empty() {
        println!("(no edges)");
    }
    for (i, e) in edges.iter().enumerate() {
        println!("{i:3}: {}", e.describe());
    }
}

pub fn run_outline(ctx: &Ctx, c: OutlineCmd) -> Result<()> {
    let mut loaded = ctx.load()?;
    match c {
        OutlineCmd::Add(e) => {
            let edge = e.to_edge()?;
            if let Some(last) = loaded.project.outline.last() {
                if last.to().distance(edge.from()) > Length::from_mm(0.001) {
                    return Err(Error::with_help(
                        format!("edge starts at {} but the previous edge ends at {}", edge.from(), last.to()),
                        "outline edges must chain end-to-start; use `pcb outline show` to see them",
                    ));
                }
            }
            loaded.project.outline.push(edge);
            let closed = loaded.project.outline.first().unwrap().from().distance(loaded.project.outline.last().unwrap().to()) <= Length::from_mm(0.001);
            loaded.save()?;
            println!("outline: {} edge(s), {}", loaded.project.outline.len(), if closed { "closed" } else { "open" });
            Ok(())
        }
        OutlineCmd::Rect { width, height, at, radius } => {
            loaded.project.outline = rect_edges(at, width, height, radius)?;
            loaded.save()?;
            println!("outline: {width} x {height} rectangle at {at}");
            Ok(())
        }
        OutlineCmd::Circle { diameter, center } => {
            loaded.project.outline = circle_edges(center, diameter);
            loaded.save()?;
            println!("outline: {diameter} circle at {center}");
            Ok(())
        }
        OutlineCmd::Pop => {
            loaded.project.outline.pop().ok_or_else(|| Error::msg("the outline has no edges"))?;
            loaded.save()
        }
        OutlineCmd::Clear => {
            loaded.project.outline.clear();
            loaded.save()
        }
        OutlineCmd::Show => {
            print_edges(&loaded.project.outline);
            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// Placement

#[derive(Args)]
pub struct PlaceArgs {
    pub refdes: String,
    /// Position of the footprint origin, `x,y`.
    #[arg(value_parser = point)]
    pub at: Option<Point>,
    #[arg(short, long, value_parser = degrees, allow_negative_numbers = true)]
    pub rotation: Option<f64>,
    #[arg(short, long, value_enum)]
    pub side: Option<Side>,
    /// Keep fixed during autorouting/placement passes.
    #[arg(long)]
    pub lock: bool,
    #[arg(long, conflicts_with = "lock")]
    pub unlock: bool,
}

pub fn place(ctx: &Ctx, a: PlaceArgs) -> Result<()> {
    let mut loaded = ctx.load()?;
    if !loaded.project.components.contains_key(&a.refdes) {
        return Err(unknown_instance(&loaded.project, &a.refdes));
    }
    let inst = loaded.project.components.get_mut(&a.refdes).unwrap();
    let mut placement = inst.placement.clone().unwrap_or(Placement { at: Point::ORIGIN, rotation: 0.0, side: Side::Top, locked: false, label_at: None, label_size: None, label_hidden: false });
    if a.at.is_none() && a.rotation.is_none() && a.side.is_none() && !a.lock && !a.unlock {
        return Err(Error::with_help("nothing to change", "give a position `x,y`, `--rotation`, `--side`, `--lock` or `--unlock`"));
    }
    if let Some(at) = a.at {
        placement.at = at;
    } else if inst.placement.is_none() {
        return Err(Error::with_help(format!("`{}` is not placed yet", a.refdes), "give a position: `pcb place R1 10,20`"));
    }
    if let Some(r) = a.rotation {
        placement.rotation = r;
    }
    if let Some(s) = a.side {
        placement.side = s;
    }
    if a.lock {
        placement.locked = true;
    }
    if a.unlock {
        placement.locked = false;
    }
    inst.placement = Some(placement.clone());
    let lib = ctx.library(&loaded)?;
    let comp = lib.component(&loaded.project.components[&a.refdes].component)?;
    if comp.is_virtual() {
        return Err(Error::with_help(
            format!("`{}` is a virtual component ({}) with no footprint, so it cannot be placed", a.refdes, comp.name),
            "virtual components only exist in the netlist and simulations",
        ));
    }
    validate(ctx, &loaded)?;
    loaded.save()?;
    println!("{} at {} rot {} side {}{}", a.refdes, placement.at, placement.rotation, placement.side, if placement.locked { " (locked)" } else { "" });
    Ok(())
}

#[derive(Args)]
pub struct UnplaceArgs {
    pub refdes: String,
}

pub fn unplace(ctx: &Ctx, a: UnplaceArgs) -> Result<()> {
    let mut loaded = ctx.load()?;
    if !loaded.project.components.contains_key(&a.refdes) {
        return Err(unknown_instance(&loaded.project, &a.refdes));
    }
    loaded.project.components.get_mut(&a.refdes).unwrap().placement = None;
    loaded.save()
}

#[derive(Args)]
pub struct LabelArgs {
    /// One or more placed parts.
    #[arg(required = true)]
    pub refdes: Vec<String>,
    /// Label centre `x,y` as an offset from the footprint origin (rotates with
    /// the part); with `--absolute`, board coordinates instead.
    #[arg(long, value_parser = point, allow_negative_numbers = true)]
    pub at: Option<Point>,
    #[arg(long, requires = "at")]
    pub absolute: bool,
    /// Text height in mm (overrides the `silk_text_size` rule).
    #[arg(long, value_parser = length)]
    pub size: Option<Length>,
    /// Leave the label off the silkscreen.
    #[arg(long, conflicts_with = "show")]
    pub hide: bool,
    /// Put a hidden label back.
    #[arg(long)]
    pub show: bool,
    /// Forget every override and use the footprint default again.
    #[arg(long, conflicts_with_all = ["at", "size", "hide", "show"])]
    pub reset: bool,
}

pub fn label(ctx: &Ctx, a: LabelArgs) -> Result<()> {
    let mut loaded = ctx.load()?;
    if a.at.is_none() && a.size.is_none() && !a.hide && !a.show && !a.reset {
        return Err(Error::with_help("nothing to change", "give `--at x,y`, `--size mm`, `--hide`, `--show` or `--reset`"));
    }
    for refdes in &a.refdes {
        if !loaded.project.components.contains_key(refdes) {
            return Err(unknown_instance(&loaded.project, refdes));
        }
        let inst = loaded.project.components.get_mut(refdes).unwrap();
        let Some(p) = inst.placement.as_mut() else {
            return Err(Error::with_help(format!("`{refdes}` is not placed"), "labels belong to placements; run `pcb place` first"));
        };
        if a.reset {
            p.label_at = None;
            p.label_size = None;
            p.label_hidden = false;
        }
        if let Some(at) = a.at {
            p.label_at = Some(if a.absolute {
                // Undo the placement transform so the stored offset follows the part.
                let t = Transform { mirror_x: p.side == Side::Bottom, degrees: p.rotation, translate: p.at };
                t.unapply(at)
            } else {
                at
            });
        }
        if let Some(sz) = a.size {
            p.label_size = Some(sz);
        }
        if a.hide {
            p.label_hidden = true;
        }
        if a.show {
            p.label_hidden = false;
        }
    }
    validate(ctx, &loaded)?;
    let board = ctx.board(&loaded)?;
    loaded.save()?;
    for refdes in &a.refdes {
        let inst = board.instances.iter().find(|i| &i.refdes == refdes).unwrap();
        match inst.label_at {
            Some(at) => println!("{refdes} label at {at} size {}", inst.label_size(board.rules())),
            None => println!("{refdes} label hidden"),
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Holes

#[derive(Subcommand)]
pub enum HoleCmd {
    /// Drill a hole at `x,y`.
    Add {
        #[arg(value_parser = point)]
        at: Point,
        #[arg(short, long, value_parser = length)]
        drill: Length,
        /// Plate the hole (adds a copper ring on every layer).
        #[arg(long)]
        plated: bool,
        /// Outer copper diameter for plated holes.
        #[arg(long, value_parser = length)]
        diameter: Option<Length>,
        /// Connect the plated ring to a net.
        #[arg(long)]
        net: Option<String>,
    },
    /// Remove the hole with the given index (see `list`).
    Remove { index: usize },
    List,
}

pub fn run_hole(ctx: &Ctx, c: HoleCmd) -> Result<()> {
    let mut loaded = ctx.load()?;
    match c {
        HoleCmd::Add { at, drill, plated, diameter, net } => {
            if drill.0 <= 0 {
                return Err(Error::msg("drill diameter must be positive"));
            }
            if let Some(n) = &net {
                if !loaded.project.nets.contains_key(n) {
                    return Err(unknown_net(&loaded.project, n));
                }
            }
            let plated = plated || net.is_some() || diameter.is_some();
            loaded.project.holes.push(Hole { at, drill, plated, diameter, net });
            validate(ctx, &loaded)?;
            loaded.save()?;
            println!("hole #{} at {at}, drill {drill}{}", loaded.project.holes.len() - 1, if plated { ", plated" } else { "" });
            Ok(())
        }
        HoleCmd::Remove { index } => {
            if index >= loaded.project.holes.len() {
                return Err(Error::msg(format!("no hole #{index}; there are {} holes", loaded.project.holes.len())));
            }
            loaded.project.holes.remove(index);
            loaded.save()
        }
        HoleCmd::List => {
            for (i, h) in loaded.project.holes.iter().enumerate() {
                println!(
                    "#{i}: at {} drill {}{}{}",
                    h.at,
                    h.drill,
                    if h.plated { " plated" } else { "" },
                    h.net.as_ref().map(|n| format!(" net {n}")).unwrap_or_default()
                );
            }
            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// Pours

#[derive(Subcommand)]
pub enum PourCmd {
    /// Create a pour. Give its shape with `--rect`, `--circle`, or
    /// `--follow-outline`, or add edges afterwards with `pcb pour add`.
    New {
        name: String,
        #[arg(short, long)]
        layer: String,
        #[arg(short, long)]
        net: Option<String>,
        /// Rectangle: `x,y WxH` i.e. `--rect 0,0 --size 50,30`.
        #[arg(long, value_parser = point, requires = "size")]
        rect: Option<Point>,
        #[arg(long, value_parser = point)]
        size: Option<Point>,
        /// Use the board outline as the pour shape.
        #[arg(long)]
        follow_outline: bool,
        #[arg(long, value_parser = length)]
        clearance: Option<Length>,
        #[arg(long, default_value = "0")]
        priority: i32,
        /// Flood same-net pads solidly instead of with thermal reliefs.
        #[arg(long)]
        solid: bool,
    },
    /// Append an edge to a pour's boundary.
    Add {
        name: String,
        #[command(subcommand)]
        edge: EdgeCmd,
    },
    Remove { name: String },
    List,
}

pub fn run_pour(ctx: &Ctx, c: PourCmd) -> Result<()> {
    let mut loaded = ctx.load()?;
    match c {
        PourCmd::New { name, layer, net, rect, size, follow_outline, clearance, priority, solid } => {
            if loaded.project.pours.iter().any(|p| p.name == name) {
                return Err(Error::msg(format!("a pour named `{name}` already exists")));
            }
            if let Some(n) = &net {
                if !loaded.project.nets.contains_key(n) {
                    return Err(unknown_net(&loaded.project, n));
                }
            }
            let edges = if let (Some(at), Some(sz)) = (rect, size) {
                rect_edges(at, sz.x, sz.y, None)?
            } else if follow_outline {
                if loaded.project.outline.is_empty() {
                    return Err(Error::with_help("the board has no outline to follow", "define one with `pcb outline rect W H`"));
                }
                loaded.project.outline.clone()
            } else {
                vec![]
            };
            loaded.project.pours.push(Pour { name: name.clone(), layer, net, edges, clearance, priority, thermal: if solid { Some(false) } else { None } });
            validate(ctx, &loaded).or_else(|e| if loaded.project.pours.last().unwrap().edges.is_empty() { Ok(()) } else { Err(e) })?;
            // An edgeless pour is allowed to exist until edges are added; validation ignores it.
            loaded.save()?;
            println!("pour {name} created{}", if loaded.project.pours.last().unwrap().edges.is_empty() { "; add edges with `pcb pour add <name> line ...`" } else { "" });
            Ok(())
        }
        PourCmd::Add { name, edge } => {
            let edge = edge.to_edge()?;
            let pour = loaded.project.pours.iter_mut().find(|p| p.name == name).ok_or_else(|| Error::msg(format!("no pour named `{name}`")))?;
            if let Some(last) = pour.edges.last() {
                if last.to().distance(edge.from()) > Length::from_mm(0.001) {
                    return Err(Error::msg(format!("edge starts at {} but the previous edge ends at {}", edge.from(), last.to())));
                }
            }
            pour.edges.push(edge);
            let n = pour.edges.len();
            let closed = pour.edges.first().unwrap().from().distance(pour.edges.last().unwrap().to()) <= Length::from_mm(0.001);
            if closed {
                validate(ctx, &loaded)?;
            }
            loaded.save()?;
            println!("pour {name}: {n} edge(s), {}", if closed { "closed" } else { "open" });
            Ok(())
        }
        PourCmd::Remove { name } => {
            let before = loaded.project.pours.len();
            loaded.project.pours.retain(|p| p.name != name);
            if loaded.project.pours.len() == before {
                return Err(Error::msg(format!("no pour named `{name}`")));
            }
            loaded.save()
        }
        PourCmd::List => {
            // The computed fill, piece by piece: area and which same-net pads each
            // piece reaches (through its thermal spokes or solidly). A pour that
            // comes out in several pieces is not necessarily wrong, but a piece
            // that reaches no pad is floating copper and a net whose pads are
            // spread over several pieces is relying on traces for the rest.
            let board = ctx.board(&loaded)?;
            for pr in &board.pours {
                let p = &pr.pour;
                let outers: Vec<&Ring> = pr.copper.iter().filter(|r| geom::signed_area(r) > 0.0).collect();
                let area: f64 = pr.copper.iter().map(|r| geom::signed_area(r)).sum::<f64>() / 1e12;
                println!(
                    "{}: layer {} net {} {} edge(s) priority {}{} — {:.1} mm² in {} piece(s)",
                    p.name, p.layer, p.net.as_deref().unwrap_or("-"), p.edges.len(), p.priority, if p.thermal == Some(false) { " solid" } else { "" }, area, outers.len()
                );
                if p.net.is_some() {
                    let holes: Vec<&Ring> = pr.copper.iter().filter(|r| geom::signed_area(r) <= 0.0).collect();
                    for o in outers {
                        let piece_area = geom::signed_area(o) / 1e12
                            - holes.iter().filter(|h| h.first().map_or(false, |q| geom::contains(o, *q))).map(|h| -geom::signed_area(h)).sum::<f64>() / 1e12;
                        let mut reached: Vec<String> = Vec::new();
                        for pad in board.all_pads() {
                            if pad.net == p.net && pad.on_layer(&p.layer) && geom::overlaps(&[o.clone()], &[pad.copper.clone()]) {
                                reached.push(format!("{}.{}", pad.refdes, pad.pad_name));
                            }
                        }
                        println!("  piece {:.1} mm²: {}", piece_area, if reached.is_empty() { "reaches no pad (floating)".to_string() } else { reached.join(" ") });
                    }
                }
            }
            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// Traces & vias

/// Replace each corner that turns by roughly 90° with a 45° cut of length
/// `c` (shortened where a leg is too short). Gentle bends and the endpoints
/// are left alone.
pub fn chamfer_corners(points: &[Point], c: Length) -> Vec<Point> {
    if points.len() < 3 || c.nm() <= 0 {
        return points.to_vec();
    }
    let mut out = vec![points[0]];
    for i in 1..points.len() - 1 {
        let (a, p, b) = (points[i - 1], points[i], points[i + 1]);
        let (ux, uy) = ((p.x.nm() - a.x.nm()) as f64, (p.y.nm() - a.y.nm()) as f64);
        let (vx, vy) = ((b.x.nm() - p.x.nm()) as f64, (b.y.nm() - p.y.nm()) as f64);
        let (lu, lv) = (ux.hypot(uy), vx.hypot(vy));
        if lu == 0.0 || lv == 0.0 {
            out.push(p);
            continue;
        }
        let cos = (ux * vx + uy * vy) / (lu * lv);
        if cos.abs() > 0.5 {
            // Not a right angle (straight-ish or a hairpin): keep the vertex.
            out.push(p);
            continue;
        }
        let cut = (c.nm() as f64).min(lu / 2.0).min(lv / 2.0);
        out.push(Point::nm(p.x.nm() - (ux / lu * cut).round() as i64, p.y.nm() - (uy / lu * cut).round() as i64));
        out.push(Point::nm(p.x.nm() + (vx / lv * cut).round() as i64, p.y.nm() + (vy / lv * cut).round() as i64));
    }
    out.push(*points.last().unwrap());
    out
}

#[derive(Subcommand)]
pub enum TraceCmd {
    /// Draw a trace through the given points.
    Add {
        #[arg(short, long)]
        layer: String,
        /// Net (inferred from a pad under the first/last point if omitted).
        #[arg(short, long)]
        net: Option<String>,
        #[arg(short, long, value_parser = length)]
        width: Option<Length>,
        /// Cut every right-angle corner with a 45° segment of this length (mm).
        #[arg(long, value_parser = length)]
        chamfer: Option<Length>,
        #[arg(required = true, num_args = 2.., value_parser = point)]
        points: Vec<Point>,
    },
    /// Delete traces (all, or only autorouted ones).
    Clear {
        #[arg(long)]
        routed_only: bool,
        #[arg(long)]
        net: Option<String>,
    },
    List,
}

pub fn run_trace(ctx: &Ctx, c: TraceCmd) -> Result<()> {
    let mut loaded = ctx.load()?;
    match c {
        TraceCmd::Add { layer, net, width, chamfer, points } => {
            let board = ctx.board(&loaded)?;
            let points = match chamfer {
                Some(c) => chamfer_corners(&points, c),
                None => points,
            };
            let net = match net {
                Some(n) => {
                    if !loaded.project.nets.contains_key(&n) {
                        return Err(unknown_net(&loaded.project, &n));
                    }
                    Some(n)
                }
                None => {
                    // Infer from pads touched by the endpoints.
                    let mut found: Option<String> = None;
                    for p in [points[0], *points.last().unwrap()] {
                        for pad in board.all_pads() {
                            if pad.on_layer(&layer) && crate::geom::contains(&pad.copper, p) {
                                if let Some(n) = &pad.net {
                                    found = Some(n.clone());
                                }
                            }
                        }
                    }
                    found
                }
            };
            let (class_width, _) = board.rules().class(net.as_deref().and_then(|n| loaded.project.nets.get(n)).and_then(|n| n.class.as_deref()));
            let width = width.unwrap_or(class_width);
            loaded.project.traces.push(Trace { layer: layer.clone(), net: net.clone(), width, points: points.clone(), routed: false });
            validate(ctx, &loaded)?;
            loaded.save()?;
            println!("trace on {layer} net {} width {width}, {} points", net.as_deref().unwrap_or("(none)"), points.len());
            Ok(())
        }
        TraceCmd::Clear { routed_only, net } => {
            let before = loaded.project.traces.len() + loaded.project.vias.len();
            loaded.project.traces.retain(|t| (routed_only && !t.routed) || (net.is_some() && t.net != net));
            loaded.project.vias.retain(|v| (routed_only && !v.routed) || (net.is_some() && v.net != net));
            let after = loaded.project.traces.len() + loaded.project.vias.len();
            loaded.save()?;
            println!("removed {} trace(s)/via(s)", before - after);
            Ok(())
        }
        TraceCmd::List => {
            for (i, t) in loaded.project.traces.iter().enumerate() {
                let pts: Vec<String> = t.points.iter().map(|p| p.to_string()).collect();
                println!("#{i}: {} net {} w {} {}{}", t.layer, t.net.as_deref().unwrap_or("-"), t.width, pts.join(" -> "), if t.routed { " (routed)" } else { "" });
            }
            Ok(())
        }
    }
}

#[derive(Subcommand)]
pub enum ViaCmd {
    Add {
        #[arg(value_parser = point)]
        at: Point,
        #[arg(short, long)]
        net: Option<String>,
        #[arg(long, value_parser = length)]
        drill: Option<Length>,
        #[arg(long, value_parser = length)]
        diameter: Option<Length>,
    },
    Remove { index: usize },
    List,
}

pub fn run_via(ctx: &Ctx, c: ViaCmd) -> Result<()> {
    let mut loaded = ctx.load()?;
    match c {
        ViaCmd::Add { at, net, drill, diameter } => {
            if loaded.project.stackup.copper_layers.len() < 2 {
                return Err(Error::with_help("vias need at least two copper layers", "this board is single-sided"));
            }
            if let Some(n) = &net {
                if !loaded.project.nets.contains_key(n) {
                    return Err(unknown_net(&loaded.project, n));
                }
            }
            let r = &loaded.project.design_rules;
            loaded.project.vias.push(Via {
                at,
                net,
                drill: drill.unwrap_or(r.via_drill),
                diameter: diameter.unwrap_or(r.via_diameter),
                layers: vec![],
                routed: false,
            });
            validate(ctx, &loaded)?;
            loaded.save()
        }
        ViaCmd::Remove { index } => {
            if index >= loaded.project.vias.len() {
                return Err(Error::msg(format!("no via #{index}")));
            }
            loaded.project.vias.remove(index);
            loaded.save()
        }
        ViaCmd::List => {
            for (i, v) in loaded.project.vias.iter().enumerate() {
                println!("#{i}: at {} net {} drill {} dia {}", v.at, v.net.as_deref().unwrap_or("-"), v.drill, v.diameter);
            }
            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// Stackup & rules

#[derive(Subcommand)]
pub enum StackupCmd {
    /// Set the copper layers, top to bottom (comma separated).
    Set {
        #[arg(value_delimiter = ',')]
        layers: Vec<String>,
        #[arg(long, value_parser = length)]
        board_thickness: Option<Length>,
        #[arg(long, value_parser = length)]
        copper_thickness: Option<Length>,
    },
    Show,
}

pub fn run_stackup(ctx: &Ctx, c: StackupCmd) -> Result<()> {
    let mut loaded = ctx.load()?;
    match c {
        StackupCmd::Set { layers, board_thickness, copper_thickness } => {
            if !layers.is_empty() {
                loaded.project.stackup.copper_layers = layers;
            }
            if let Some(t) = board_thickness {
                loaded.project.stackup.board_thickness = t;
            }
            if let Some(t) = copper_thickness {
                loaded.project.stackup.copper_thickness = t;
            }
            validate(ctx, &loaded)?;
            loaded.save()
        }
        StackupCmd::Show => {
            let s = &loaded.project.stackup;
            println!("copper layers (top to bottom): {}", s.copper_layers.join(", "));
            println!("board thickness {}, copper thickness {}", s.board_thickness, s.copper_thickness);
            Ok(())
        }
    }
}

#[derive(Subcommand)]
pub enum RulesCmd {
    /// Set rules: `trace_width=0.3 clearance=0.2 via_drill=0.4 ...`.
    Set {
        #[arg(required = true, value_parser = parse_kv)]
        assignments: Vec<(String, String)>,
    },
    /// Define or update a net class.
    Class {
        name: String,
        #[arg(long, value_parser = length)]
        width: Option<Length>,
        #[arg(long, value_parser = length)]
        clearance: Option<Length>,
        #[arg(long, value_parser = length)]
        via_drill: Option<Length>,
        #[arg(long, value_parser = length)]
        via_diameter: Option<Length>,
    },
    Show,
}

pub fn run_rules(ctx: &Ctx, c: RulesCmd) -> Result<()> {
    let mut loaded = ctx.load()?;
    match c {
        RulesCmd::Set { assignments } => {
            for (k, v) in assignments {
                let r = &mut loaded.project.design_rules;
                let val = Length::parse(&v)?;
                match k.as_str() {
                    "trace_width" => r.trace_width = val,
                    "clearance" => r.clearance = val,
                    "via_drill" => r.via_drill = val,
                    "via_diameter" => r.via_diameter = val,
                    "pour_clearance" => r.pour_clearance = Some(val),
                    "mask_expansion" => r.mask_expansion = val,
                    "paste_shrink" => r.paste_shrink = val,
                    "silk_width" => r.silk_width = val,
                    "silk_text_size" => r.silk_text_size = val,
                    "edge_clearance" => r.edge_clearance = val,
                    "hole_annular_ring" => r.hole_annular_ring = val,
                    "thermal_gap" => r.thermal_gap = Some(val),
                    "pour_min_width" => r.pour_min_width = Some(val),
                    "thermal_spoke_width" => r.thermal_spoke_width = val,
                    other => {
                        let names = ["trace_width", "clearance", "via_drill", "via_diameter", "pour_clearance", "mask_expansion", "paste_shrink", "silk_width", "silk_text_size", "edge_clearance", "hole_annular_ring", "thermal_gap", "thermal_spoke_width", "pour_min_width"];
                        let mut e = Error::msg(format!("unknown design rule `{other}`; rules are {}", list_names(names)));
                        if let Some(s) = suggest(other, names) {
                            e = e.help(s);
                        }
                        return Err(e);
                    }
                }
            }
            validate(ctx, &loaded)?;
            loaded.save()
        }
        RulesCmd::Class { name, width, clearance, via_drill, via_diameter } => {
            let c = loaded.project.design_rules.net_classes.entry(name).or_default();
            if width.is_some() { c.trace_width = width; }
            if clearance.is_some() { c.clearance = clearance; }
            if via_drill.is_some() { c.via_drill = via_drill; }
            if via_diameter.is_some() { c.via_diameter = via_diameter; }
            loaded.save()
        }
        RulesCmd::Show => {
            let r = &loaded.project.design_rules;
            println!("trace_width {}\nclearance {}\nvia_drill {}\nvia_diameter {}\npour_clearance {}\nmask_expansion {}\npaste_shrink {}\nsilk_width {}\nsilk_text_size {}\nedge_clearance {}\nhole_annular_ring {}\nthermal_gap {}\nthermal_spoke_width {}\npour_min_width {}",
                r.trace_width, r.clearance, r.via_drill, r.via_diameter, r.pour_clearance(), r.mask_expansion, r.paste_shrink, r.silk_width, r.silk_text_size, r.edge_clearance, r.hole_annular_ring, r.thermal_gap(), r.thermal_spoke_width, r.pour_min_width());
            for (n, c) in &r.net_classes {
                println!("class {n}: width {} clearance {}", c.trace_width.map(|l| l.to_string()).unwrap_or("default".into()), c.clearance.map(|l| l.to_string()).unwrap_or("default".into()));
            }
            Ok(())
        }
    }
}
