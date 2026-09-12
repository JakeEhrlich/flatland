//! Read-only commands: status, check, schema docs.

use crate::schema::Severity;
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
    /// Findings as a JSON array.
    #[arg(long)]
    pub json: bool,
    /// Only run this rule.
    #[arg(long)]
    pub rule: Option<String>,
    /// Show waived findings too.
    #[arg(long)]
    pub waived: bool,
    /// List info-level findings (they are only counted otherwise).
    #[arg(long)]
    pub info: bool,
}

pub fn check(ctx: &Ctx, a: CheckArgs) -> Result<()> {
    let loaded = ctx.load()?;
    let board = ctx.board(&loaded)?;
    let mut report = crate::drc::run(&board, &loaded.path)?;
    if let Some(r) = &a.rule {
        report.findings.retain(|f| &f.rule == r);
    }
    if a.json {
        println!("{}", serde_json::to_string_pretty(&report.findings).unwrap());
    } else {
        // Long lists of one rule's findings are summarised after a few lines;
        // `--rule NAME` shows them all.
        let cap = if a.rule.is_some() { usize::MAX } else { 6 };
        let mut shown: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
        let mut hidden: Vec<(String, usize)> = Vec::new();
        for f in &report.findings {
            if f.waived.is_some() && !a.waived {
                continue;
            }
            if f.severity == Severity::Info && !a.info && a.rule.is_none() {
                continue;
            }
            let n = shown.entry(f.rule.as_str()).or_insert(0);
            *n += 1;
            if *n > cap {
                match hidden.iter_mut().find(|(r, _)| *r == f.rule) {
                    Some(e) => e.1 += 1,
                    None => hidden.push((f.rule.clone(), 1)),
                }
                continue;
            }
            match &f.waived {
                Some(reason) => println!("waived: {}: {} (waived: {reason})", f.rule, f.message),
                None => println!("{}: {}: {}", f.severity, f.rule, f.message),
            }
        }
        for (rule, n) in hidden {
            println!("… {n} more {rule} finding(s) (`pcb check --rule {rule}` lists them)");
        }
        let (e, w, i) = (report.count(Severity::Error), report.count(Severity::Warning), report.count(Severity::Info));
        let waived = report.waived();
        println!(
            "{e} error(s), {w} warning(s){}{} — {} rule(s) from {}",
            if i > 0 { format!(", {i} info (--info lists them)") } else { String::new() },
            if waived > 0 { format!(", {waived} waived") } else { String::new() },
            report.rules_run,
            report.sets.join(", ")
        );
    }
    let errors = report.count(Severity::Error);
    let warnings = report.count(Severity::Warning);
    if errors > 0 || (a.strict && warnings > 0) {
        return Err(Error::with_help("design check failed", "`pcb drc explain <rule>` describes a rule; `pcb drc waive <rule> <feature> --reason ...` silences a finding you have judged acceptable"));
    }
    Ok(())
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
                return Err(Error::with_help(format!("no schema section `{k}`"), "sections: project, index, component, footprint, simulation, drc"));
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
    ("pcb-board", &["outline", "hole", "text", "stackup", "rules", "board"], include_str!("../../docs/commands/pcb-board.md")),
    ("pcb-place", &["place", "unplace", "label"], include_str!("../../docs/commands/pcb-place.md")),
    ("pcb-copper", &["pour", "trace", "via", "copper"], include_str!("../../docs/commands/pcb-copper.md")),
    ("pcb-visualize", &["visualize", "png", "svg", "render"], include_str!("../../docs/commands/pcb-visualize.md")),
    ("pcb-route", &["route", "freerouting", "dsn", "ses"], include_str!("../../docs/commands/pcb-route.md")),
    ("pcb-gerbers", &["gerbers", "gerber", "drill", "excellon", "fab"], include_str!("../../docs/commands/pcb-gerbers.md")),
    ("pcb-drc", &["drc", "rules-check", "design-rules", "waive", "profiles"], include_str!("../../docs/commands/pcb-drc.md")),
    ("pcb-assembly", &["bom", "pnp", "assembly", "jlcpcb", "lcsc", "cpl", "pick-and-place"], include_str!("../../docs/commands/pcb-assembly.md")),
    ("pcb-sim", &["sim", "simulate", "spice", "ngspice"], include_str!("../../docs/commands/pcb-sim.md")),
    ("pcb-python", &["python", "bindings", "api", "library", "in-memory"], include_str!("../../docs/commands/pcb-python.md")),
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
