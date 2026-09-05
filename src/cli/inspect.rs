//! Read-only commands: status, check, schema docs.

use super::Ctx;
use crate::error::{Error, Result};
use crate::geom;
use clap::Args;

#[derive(Args)]
pub struct StatusArgs {
    /// Print as JSON (for agents).
    #[arg(long)]
    pub json: bool,
}

pub fn status(ctx: &Ctx, a: StatusArgs) -> Result<()> {
    let loaded = ctx.load()?;
    let board = ctx.board(&loaded)?;
    let conn = board.connectivity()?;
    let unrouted: usize = conn.iter().map(|(_, islands)| islands.len().saturating_sub(1)).sum();
    let placed = board.instances.iter().filter(|i| i.is_placed()).count();
    let virtuals = board.instances.iter().filter(|i| i.component.is_virtual()).count();
    let unplaced: Vec<&str> = board.instances.iter().filter(|i| !i.is_placed() && !i.component.is_virtual()).map(|i| i.refdes.as_str()).collect();
    if a.json {
        let v = serde_json::json!({
            "name": board.project.name,
            "path": board.path,
            "layers": board.layers(),
            "outline": board.outline.as_ref().map(|o| {
                let (lo, hi) = geom::bounds(&[o.clone()]).unwrap();
                serde_json::json!({"width_mm": (hi.x - lo.x).mm(), "height_mm": (hi.y - lo.y).mm(), "min": lo, "max": hi})
            }),
            "components": board.instances.iter().map(|i| serde_json::json!({
                "refdes": i.refdes, "component": i.component.name, "placed": i.is_placed(), "virtual": i.component.is_virtual(),
                "placement": i.instance.placement, "parameters": i.instance.parameters,
            })).collect::<Vec<_>>(),
            "nets": conn.iter().map(|(n, islands)| serde_json::json!({"name": n, "pins": board.net(n).map(|x| x.pins.len()).unwrap_or(0), "islands": islands, "unrouted": islands.len().saturating_sub(1)})).collect::<Vec<_>>(),
            "unrouted_connections": unrouted,
            "traces": board.project.traces.len(),
            "vias": board.project.vias.len(),
            "pours": board.project.pours.len(),
            "holes": board.project.holes.len(),
            "simulations": board.project.simulations.keys().collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string_pretty(&v).unwrap());
        return Ok(());
    }
    println!("project {} ({})", board.project.name, board.path.display());
    println!("layers: {}", board.layers().join(", "));
    match &board.outline {
        Some(o) => {
            let (lo, hi) = geom::bounds(&[o.clone()]).unwrap();
            println!("outline: {} x {} ({} edges, area {:.1} mm²)", hi.x - lo.x, hi.y - lo.y, board.project.outline.len(), geom::signed_area(o).abs() / 1e12);
        }
        None => println!("outline: none (set one with `pcb outline rect W H`)"),
    }
    println!("components: {} ({} placed, {} virtual)", board.instances.len(), placed, virtuals);
    for i in &board.instances {
        let params: Vec<String> = i.instance.parameters.iter().map(|(k, v)| format!("{k}={v}")).collect();
        let where_ = match &i.instance.placement {
            Some(p) => format!("at {} rot {} {}", p.at, p.rotation, p.side),
            None if i.component.is_virtual() => "virtual".into(),
            None => "UNPLACED".into(),
        };
        println!("  {:<6} {:<24} {:<28} {}", i.refdes, i.component.name, params.join(" "), where_);
    }
    println!("nets: {} ({} unrouted connection(s))", board.nets.len(), unrouted);
    for (n, islands) in &conn {
        let net = board.net(n)?;
        let pins: Vec<String> = net.pins.iter().map(|(r, p)| format!("{r}.{p}")).collect();
        let state = if islands.len() <= 1 { "routed".to_string() } else { format!("{} islands", islands.len()) };
        println!("  {:<12} {:<10} {}", n, state, pins.join(" "));
    }
    println!("traces: {}, vias: {}, pours: {}, holes: {}", board.project.traces.len(), board.project.vias.len(), board.project.pours.len(), board.project.holes.len());
    if !board.project.simulations.is_empty() {
        println!("simulations: {}", board.project.simulations.keys().cloned().collect::<Vec<_>>().join(", "));
    }
    if !unplaced.is_empty() {
        println!("unplaced: {}", unplaced.join(", "));
    }
    Ok(())
}

#[derive(Args)]
pub struct CheckArgs {
    /// Treat warnings as errors.
    #[arg(long)]
    pub strict: bool,
}

pub fn check(ctx: &Ctx, a: CheckArgs) -> Result<()> {
    let loaded = ctx.load()?;
    let board = ctx.board(&loaded)?;
    let mut errors: Vec<String> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();

    if board.outline.is_none() {
        errors.push("no board outline".into());
    }
    for i in &board.instances {
        if !i.is_placed() && !i.component.is_virtual() {
            errors.push(format!("{} ({}) is not placed", i.refdes, i.component.name));
        }
    }
    // Pads outside the outline.
    if let Some(o) = &board.outline {
        for pad in board.all_pads() {
            if !geom::contains(o, pad.center) {
                errors.push(format!("pad {}.{} at {} is outside the board outline", pad.refdes, pad.pad_name, pad.center));
            }
        }
        for h in &board.holes {
            if !geom::contains(o, h.hole.at) {
                errors.push(format!("hole at {} is outside the board outline", h.hole.at));
            }
        }
    }
    // Unconnected pins.
    for i in &board.instances {
        for pin in &i.component.component.pins {
            if board.pin_net(&i.refdes, &pin.name).is_none() {
                warnings.push(format!("{}.{} is not connected to any net", i.refdes, pin.name));
            }
        }
    }
    // Single-pin nets.
    for n in &board.nets {
        if n.pins.len() < 2 {
            warnings.push(format!("net {} has only {} pin(s)", n.name, n.pins.len()));
        }
    }
    // Connectivity.
    for (n, islands) in board.connectivity()? {
        if islands.len() > 1 {
            let desc: Vec<String> = islands.iter().map(|i| i.join("+")).collect();
            warnings.push(format!("net {n} is not fully routed: {} islands ({})", islands.len(), desc.join(" | ")));
        }
    }
    // Clearance between different nets on each layer (pads/traces/vias/pours).
    let clearance = board.rules().clearance;
    for layer in board.layers() {
        let mut items: Vec<(Option<String>, geom::Rings, String)> = board
            .copper_on_layer(layer)
            .into_iter()
            .map(|(n, r)| {
                let c = geom::bounds(&[r.clone()]).map(|(lo, hi)| Point_mid(lo, hi)).unwrap_or_default();
                (n, vec![r], c)
            })
            .collect();
        for p in &board.pours {
            if &p.pour.layer == layer {
                // A pour is one polygon set (outer rings plus clearance holes).
                items.push((p.pour.net.clone(), p.copper.clone(), format!("pour {}", p.pour.name)));
            }
        }
        for i in 0..items.len() {
            for j in (i + 1)..items.len() {
                let (na, ra, da) = &items[i];
                let (nb, rb, db) = &items[j];
                if na.is_some() && na == nb {
                    continue;
                }
                if !bbox_near(ra, rb, clearance) {
                    continue;
                }
                // Grow by slightly less than the clearance so copper placed
                // exactly at the rule (e.g. pour edges) does not trip it.
                let grown = geom::offset(ra, clearance - crate::units::Length(2_000));
                if geom::overlaps(&grown, rb) {
                    let touching = geom::overlaps(ra, rb);
                    let why = if touching { "overlap (short circuit)".to_string() } else { format!("are closer than the {clearance} clearance") };
                    errors.push(format!(
                        "{layer}: {} ({}) and {} ({}) {}",
                        da,
                        na.as_deref().unwrap_or("no net"),
                        db,
                        nb.as_deref().unwrap_or("no net"),
                        why
                    ));
                }
            }
        }
    }
    for e in &errors {
        println!("error: {e}");
    }
    for w in &warnings {
        println!("warning: {w}");
    }
    println!("{} error(s), {} warning(s)", errors.len(), warnings.len());
    if !errors.is_empty() || (a.strict && !warnings.is_empty()) {
        return Err(Error::msg("design check failed"));
    }
    Ok(())
}

#[allow(non_snake_case)]
fn Point_mid(lo: crate::units::Point, hi: crate::units::Point) -> String {
    let m = crate::units::Point::nm((lo.x.nm() + hi.x.nm()) / 2, (lo.y.nm() + hi.y.nm()) / 2);
    format!("copper near {m}")
}

fn bbox_near(a: &geom::Rings, b: &geom::Rings, d: crate::units::Length) -> bool {
    let (Some((alo, ahi)), Some((blo, bhi))) = (geom::bounds(a), geom::bounds(b)) else { return false };
    !(ahi.x + d < blo.x || bhi.x + d < alo.x || ahi.y + d < blo.y || bhi.y + d < alo.y)
}

#[derive(Args)]
pub struct SchemaArgs {
    /// Which file kind: project, index, component, footprint (default: all).
    pub kind: Option<String>,
}

pub fn schema(_ctx: &Ctx, a: SchemaArgs) -> Result<()> {
    let text = include_str!("../../docs/SCHEMA.md");
    match a.kind.as_deref() {
        None => println!("{text}"),
        Some(k) => {
            let heading = format!("## {}", k.to_lowercase());
            let mut printing = false;
            let mut any = false;
            for line in text.lines() {
                if line.starts_with("## ") {
                    printing = line.to_lowercase().starts_with(&heading);
                }
                if printing {
                    any = true;
                    println!("{line}");
                }
            }
            if !any {
                return Err(Error::with_help(format!("no schema section `{k}`"), "sections: project, index, component, footprint, simulation"));
            }
        }
    }
    Ok(())
}

/// Manual pages embedded in the binary: (page name, commands it covers, text).
const PAGES: &[(&str, &[&str], &str)] = &[
    ("pcb", &["overview", "units", "coordinates", "workflow", "environment"], include_str!("../../docs/commands/pcb.md")),
    ("pcb-project", &["init", "status", "check", "pads", "schema", "docs"], include_str!("../../docs/commands/pcb-project.md")),
    ("pcb-index", &["index", "component", "library", "footprint"], include_str!("../../docs/commands/pcb-index.md")),
    ("pcb-netlist", &["add", "remove", "set", "connect", "disconnect", "net", "netlist"], include_str!("../../docs/commands/pcb-netlist.md")),
    ("pcb-board", &["outline", "hole", "stackup", "rules", "board"], include_str!("../../docs/commands/pcb-board.md")),
    ("pcb-place", &["place", "unplace", "label"], include_str!("../../docs/commands/pcb-place.md")),
    ("pcb-copper", &["pour", "trace", "via", "copper"], include_str!("../../docs/commands/pcb-copper.md")),
    ("pcb-visualize", &["visualize", "png", "svg", "render"], include_str!("../../docs/commands/pcb-visualize.md")),
    ("pcb-route", &["route", "freerouting", "dsn", "ses"], include_str!("../../docs/commands/pcb-route.md")),
    ("pcb-gerbers", &["gerbers", "gerber", "drill", "excellon", "fab"], include_str!("../../docs/commands/pcb-gerbers.md")),
    ("pcb-assembly", &["bom", "pnp", "assembly", "jlcpcb", "lcsc", "cpl", "pick-and-place"], include_str!("../../docs/commands/pcb-assembly.md")),
    ("pcb-sim", &["sim", "simulate", "spice", "ngspice"], include_str!("../../docs/commands/pcb-sim.md")),
];

#[derive(Args)]
pub struct DocsArgs {
    /// Page or command name, e.g. `route`, `sim`, `connect`, `pour` (default: list pages).
    pub topic: Option<String>,
}

pub fn docs(_ctx: &Ctx, a: DocsArgs) -> Result<()> {
    let Some(topic) = a.topic else {
        println!("Manual pages (`pcb docs <page or command>`; also in docs/commands/*.md):\n");
        for (name, cmds, _) in PAGES {
            println!("  {:<14} {}", name, cmds.join(", "));
        }
        println!("\nFile formats: `pcb schema`.");
        return Ok(());
    };
    let t = topic.trim().trim_start_matches("pcb-").to_lowercase();
    let page = PAGES
        .iter()
        .find(|(name, _, _)| name.trim_start_matches("pcb-") == t || *name == t)
        .or_else(|| PAGES.iter().find(|(_, cmds, _)| cmds.iter().any(|c| *c == t)))
        .or_else(|| PAGES.iter().find(|(_, cmds, _)| cmds.iter().any(|c| c.starts_with(&t))));
    match page {
        Some((_, _, text)) => {
            print!("{text}");
            Ok(())
        }
        None => {
            let names: Vec<&str> = PAGES.iter().flat_map(|(n, c, _)| std::iter::once(*n).chain(c.iter().copied())).collect();
            let mut e = Error::msg(format!("no manual page for `{topic}`; pages are {}", crate::error::list_names(PAGES.iter().map(|p| p.0))));
            if let Some(s) = crate::error::suggest(&t, names.iter().copied()) {
                e = e.help(s);
            }
            Err(e)
        }
    }
}

#[derive(Args)]
pub struct PadsArgs {
    /// Instances to list (default: every placed instance).
    pub refdes: Vec<String>,
    /// Print as JSON.
    #[arg(long)]
    pub json: bool,
}

/// List pad positions (board frame, mm) with their pins and nets — the
/// numbers you need for `pcb trace add` when routing by hand.
pub fn pads(ctx: &Ctx, a: PadsArgs) -> Result<()> {
    let loaded = ctx.load()?;
    let board = ctx.board(&loaded)?;
    let mut rows = Vec::new();
    for inst in &board.instances {
        if !a.refdes.is_empty() && !a.refdes.iter().any(|r| r == &inst.refdes) {
            continue;
        }
        if !inst.is_placed() {
            continue;
        }
        for pad in &inst.pads {
            let (lo, hi) = geom::bounds(&[pad.copper.clone()]).unwrap();
            rows.push(serde_json::json!({
                "refdes": inst.refdes, "pad": pad.pad_name, "pins": pad.pins, "net": pad.net,
                "x": pad.center.x.mm(), "y": pad.center.y.mm(),
                "min": [lo.x.mm(), lo.y.mm()], "max": [hi.x.mm(), hi.y.mm()],
                "layers": pad.layers,
            }));
        }
    }
    for r in &a.refdes {
        board.instance(r)?;
    }
    if a.json {
        println!("{}", serde_json::to_string_pretty(&rows).unwrap());
        return Ok(());
    }
    println!("{:<6} {:<5} {:<8} {:<10} {:>8} {:>8}   extent (min .. max)", "ref", "pad", "pin", "net", "x", "y");
    for r in &rows {
        println!(
            "{:<6} {:<5} {:<8} {:<10} {:>8.3} {:>8.3}   {:.3},{:.3} .. {:.3},{:.3}",
            r["refdes"].as_str().unwrap(), r["pad"].as_str().unwrap(),
            r["pins"].as_array().map(|v| v.iter().map(|x| x.as_str().unwrap()).collect::<Vec<_>>().join("/")).unwrap_or_default(),
            r["net"].as_str().unwrap_or("-"),
            r["x"].as_f64().unwrap(), r["y"].as_f64().unwrap(),
            r["min"][0].as_f64().unwrap(), r["min"][1].as_f64().unwrap(), r["max"][0].as_f64().unwrap(), r["max"][1].as_f64().unwrap()
        );
    }
    Ok(())
}
