//! Visualisation, routing and fabrication outputs.

use super::Ctx;
use crate::error::{Error, Result};
use clap::{Args, Subcommand};
use std::path::PathBuf;

#[derive(Subcommand)]
pub enum VisualizeCmd {
    /// Draw the netlist as a graph: components as boxes, nets as edges.
    Netlist(VizArgs),
    /// Draw the board: outline, footprints, pads, traces, pours, airwires.
    Pcb(PcbVizArgs),
}

#[derive(Args)]
pub struct VizArgs {
    /// Output PNG path (default `build/netlist.png`); an SVG is written alongside.
    #[arg(short, long)]
    pub output: Option<PathBuf>,
    /// PNG width in pixels.
    #[arg(long, default_value = "1600")]
    pub width: u32,
}

#[derive(Args)]
pub struct PcbVizArgs {
    /// Render only this region: two corners `x0,y0 x1,y1`.
    #[arg(long, num_args = 2, value_parser = point_arg, allow_negative_numbers = true)]
    pub crop: Vec<crate::units::Point>,
    /// Output PNG path (default `build/pcb.png`); an SVG is written alongside.
    #[arg(short, long)]
    pub output: Option<PathBuf>,
    /// PNG width in pixels.
    #[arg(long, default_value = "1600")]
    pub width: u32,
    /// Only draw these copper layers (comma separated).
    #[arg(long, value_delimiter = ',')]
    pub layers: Option<Vec<String>>,
    /// View from the bottom (mirrored).
    #[arg(long)]
    pub from_bottom: bool,
    /// Hide airwires.
    #[arg(long)]
    pub no_ratsnest: bool,
    /// Hide reference designators.
    #[arg(long)]
    pub no_labels: bool,
    /// Draw a millimetre grid.
    #[arg(long)]
    pub grid: bool,
}

pub fn run_visualize(ctx: &Ctx, c: VisualizeCmd) -> Result<()> {
    let loaded = ctx.load()?;
    let board = ctx.board(&loaded)?;
    match c {
        VisualizeCmd::Netlist(a) => {
            let png = a.output.unwrap_or_else(|| loaded.build_dir().join("netlist.png"));
            let svg = crate::viz::netlist::render(&board)?;
            // Big netlists get a wider PNG so pin names stay legible (unless --width was given).
            let width = if a.width == 1600 {
                let vb_w = svg.split("viewBox=\"").nth(1).and_then(|s| s.split('"').next()).and_then(|v| v.split_whitespace().nth(2)).and_then(|w| w.parse::<f64>().ok()).unwrap_or(1600.0);
                (vb_w.max(1600.0).min(8000.0)) as u32
            } else {
                a.width
            };
            crate::viz::write_svg_png(&svg, &png, width)?;
            Ok(())
        }
        VisualizeCmd::Pcb(a) => {
            let png = a.output.unwrap_or_else(|| loaded.build_dir().join("pcb.png"));
            let crop = match a.crop.as_slice() {
                [lo, hi] => Some((*lo, *hi)),
                [] => None,
                _ => return Err(Error::with_help("--crop takes two points", "e.g. `--crop 2,9 8,24`")),
            };
            let opts = crate::viz::pcb::Options {
                layers: a.layers,
                from_bottom: a.from_bottom,
                ratsnest: !a.no_ratsnest,
                labels: !a.no_labels,
                grid: a.grid,
                crop,
            };
            let svg = crate::viz::pcb::render(&board, &opts)?;
            crate::viz::write_svg_png(&svg, &png, a.width)?;
            Ok(())
        }
    }
}

#[derive(Subcommand)]
pub enum RouteSub {
    /// Route one trace between two pins of a net, fast: A* over a cost field, 45° geometry,
    /// verified against real clearances. Starts/ends on any copper already joined to each pin.
    Pin {
        /// Start pin, e.g. `D8.K`.
        from: String,
        /// End pin on the same net.
        to: String,
        /// Restrict to one copper layer (default: all, with vias on multi-layer boards).
        #[arg(short, long)]
        layer: Option<String>,
        /// Trace width (default: the net class / `trace_width`).
        #[arg(short, long, value_parser = length_arg)]
        width: Option<crate::units::Length>,
        /// Waypoint `x,y` the route must pass through (repeatable, in order).
        #[arg(long = "via", value_parser = point_arg, allow_negative_numbers = true)]
        waypoints: Vec<crate::units::Point>,
        /// Cost of a layer change, in mm of trace it is worth avoiding.
        #[arg(long, default_value = "6")]
        via_cost: f64,
        /// Write build/pcb.png afterwards: `full`, or `crop` (the new trace and its surroundings).
        #[arg(long, value_name = "MODE")]
        png: Option<String>,
        /// Find the route and report it without adding it.
        #[arg(long)]
        dry_run: bool,
    },
}

fn length_arg(s: &str) -> std::result::Result<crate::units::Length, Error> {
    crate::units::Length::parse(s)
}
fn point_arg(s: &str) -> std::result::Result<crate::units::Point, Error> {
    crate::units::Point::parse(s)
}

#[derive(Args)]
#[command(args_conflicts_with_subcommands = true)]
pub struct RouteArgs {
    #[command(subcommand)]
    pub sub: Option<RouteSub>,
    /// Only write the .dsn file (to route by hand in freerouting).
    #[arg(long)]
    pub dsn_only: bool,
    /// Import an existing .ses session file instead of running freerouting.
    #[arg(long)]
    pub import: Option<PathBuf>,
    /// Path to freerouting (jar or app launcher); overrides project config / $FREEROUTING.
    #[arg(long)]
    pub freerouting: Option<PathBuf>,
    /// Java executable to use for a jar.
    #[arg(long)]
    pub java: Option<PathBuf>,
    /// Maximum optimisation passes.
    #[arg(long)]
    pub passes: Option<u32>,
    /// Keep existing autorouted traces (default: they are replaced).
    #[arg(long)]
    pub keep: bool,
    /// Time limit for the router in seconds.
    #[arg(long, default_value = "600")]
    pub timeout: u64,
    /// Keep every router-drawn segment, even ones a pour, hand wiring or a multi-tab pin already makes redundant.
    #[arg(long)]
    pub keep_redundant: bool,
}

pub fn route(ctx: &Ctx, a: RouteArgs) -> Result<()> {
    match a.sub {
        Some(RouteSub::Pin { from, to, layer, width, waypoints, via_cost, png, dry_run }) => route_pin(ctx, from, to, layer, width, waypoints, via_cost, png, dry_run),
        None => crate::route::run(ctx, a),
    }
}

fn net_of(board: &crate::model::Board, pin: &str) -> String {
    crate::model::parse_pin_ref(pin).ok().and_then(|(r, p)| board.pad(r, p).ok()).and_then(|p| p.net.clone()).unwrap_or_default()
}

#[allow(clippy::too_many_arguments)]
fn route_pin(ctx: &Ctx, from: String, to: String, layer: Option<String>, width: Option<crate::units::Length>, waypoints: Vec<crate::units::Point>, via_cost: f64, png: Option<String>, dry_run: bool) -> Result<()> {
    let mut loaded = ctx.load()?;
    let board = ctx.board(&loaded)?;
    let t0 = std::time::Instant::now();
    let req = crate::router::Request { from: from.clone(), to: to.clone(), layer, width, waypoints, via_cost_mm: via_cost };
    let width_used = width.unwrap_or_else(|| {
        let class = board.project.nets.get(&net_of(&board, &from)).and_then(|n| n.class.as_deref());
        board.rules().class(class).0
    });
    match crate::router::route_pin(&board, &req)? {
        crate::router::Outcome::Blocked { layer, closest, distance_mm, blockers } => {
            return Err(Error::with_help(
                format!("no route from {from} to {to}: on {layer} the nearest approach is {distance_mm:.2} mm from the target, at {closest}"),
                if blockers.is_empty() {
                    "nothing copper is in the way; the board edge or the layer choice is".to_string()
                } else {
                    format!("in the way there: {}; the trace is {} wide (try `--width`), or move something, add a waypoint (`--via x,y`) or route on another layer", blockers.join(", "), width_used)
                },
            ));
        }
        crate::router::Outcome::Routed { traces, vias, length_mm } => {
            let segs: usize = traces.iter().map(|t| t.points.len() - 1).sum();
            let layers: Vec<String> = traces.iter().map(|t| t.layer.clone()).collect::<std::collections::BTreeSet<_>>().into_iter().collect();
            println!(
                "{}routed {from} -> {to} on {}: {length_mm:.2} mm, {segs} segment(s), {} via(s) ({:.0} ms)",
                if dry_run { "dry run: " } else { "" },
                layers.join("+"),
                vias.len(),
                t0.elapsed().as_secs_f64() * 1000.0
            );
            for t in &traces {
                let pts: Vec<String> = t.points.iter().map(|p| p.to_string()).collect();
                println!("  {} w {}: {}", t.layer, t.width, pts.join(" -> "));
            }
            for v in &vias {
                println!("  via at {}", v.at);
            }
            if dry_run {
                return Ok(());
            }
            let mut bbox: Option<(crate::units::Point, crate::units::Point)> = None;
            for t in &traces {
                for p in &t.points {
                    bbox = Some(match bbox {
                        None => (*p, *p),
                        Some((lo, hi)) => (crate::units::Point::nm(lo.x.nm().min(p.x.nm()), lo.y.nm().min(p.y.nm())), crate::units::Point::nm(hi.x.nm().max(p.x.nm()), hi.y.nm().max(p.y.nm()))),
                    });
                }
            }
            loaded.project.traces.extend(traces);
            loaded.project.vias.extend(vias);
            loaded.save()?;
            if let Some(mode) = png {
                let board = ctx.board(&loaded)?;
                let crop = match mode.as_str() {
                    "crop" => bbox.map(|(lo, hi)| {
                        let m = crate::units::Length::from_mm(4.0);
                        (crate::units::Point::nm(lo.x.nm() - m.nm(), lo.y.nm() - m.nm()), crate::units::Point::nm(hi.x.nm() + m.nm(), hi.y.nm() + m.nm()))
                    }),
                    "full" => None,
                    other => return Err(Error::with_help(format!("unknown --png mode `{other}`"), "use `full` or `crop`")),
                };
                let opts = crate::viz::pcb::Options { layers: None, from_bottom: false, ratsnest: true, labels: true, grid: false, crop };
                let svg = crate::viz::pcb::render(&board, &opts)?;
                let png_path = loaded.build_dir().join("pcb.png");
                crate::viz::write_svg_png(&svg, &png_path, 1600)?;
                println!("wrote {}{}", png_path.display(), if crop.is_some() { " (cropped to the new trace)" } else { "" });
            }
            Ok(())
        }
    }
}

#[derive(Args)]
pub struct GerbersArgs {
    /// Output directory (default `build/gerbers`).
    #[arg(short, long)]
    pub output: Option<PathBuf>,
    /// Skip the zip archive.
    #[arg(long)]
    pub no_zip: bool,
    /// Write the files even if the design check reports errors.
    #[arg(long)]
    pub force: bool,
}

pub fn gerbers(ctx: &Ctx, a: GerbersArgs) -> Result<()> {
    let loaded = ctx.load()?;
    let board = ctx.board(&loaded)?;
    // Fab output goes through the design check first.
    let report = crate::drc::run(&board, &loaded.path)?;
    let errors = report.count(crate::schema::Severity::Error);
    if errors > 0 {
        for f in report.findings.iter().filter(|f| f.severity == crate::schema::Severity::Error && f.waived.is_none()) {
            eprintln!("error: {}: {}", f.rule, f.message);
        }
        if !a.force {
            return Err(crate::error::Error::with_help(
                format!("design check reports {errors} error(s); not writing gerbers"),
                "fix them (`pcb check`), waive the ones you have judged acceptable (`pcb drc waive`), or pass --force",
            ));
        }
        eprintln!("warning: writing gerbers despite {errors} design-check error(s) (--force)");
    }
    let dir = a.output.unwrap_or_else(|| loaded.build_dir().join("gerbers"));
    let files = crate::gerber::emit(&board, &dir, !a.no_zip)?;
    for f in files {
        println!("{}", f.display());
    }
    Ok(())
}

#[derive(Subcommand)]
pub enum ExportCmd {
    /// Write a KiCad board (`.kicad_pcb` + `.kicad_pro` with the design rules), for KiCad's DRC or editor.
    Kicad {
        /// Output directory (default `build/kicad`).
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Run `kicad-cli pcb drc` on the result and summarise the report.
        #[arg(long)]
        drc: bool,
    },
}

pub fn run_export(ctx: &Ctx, c: ExportCmd) -> Result<()> {
    match c {
        ExportCmd::Kicad { output, drc } => {
            let loaded = ctx.load()?;
            let board = ctx.board(&loaded)?;
            let dir = output.unwrap_or_else(|| loaded.build_dir().join("kicad"));
            let ex = crate::kicad::write(&board, &dir)?;
            println!("{}\n{}", ex.pcb.display(), ex.pro.display());
            if !drc {
                return Ok(());
            }
            let report = crate::kicad::run_drc(&ex.pcb)?;
            let mut all: Vec<&crate::kicad::Violation> = report.violations.iter().chain(report.unconnected_items.iter()).collect();
            all.sort_by(|a, b| b.severity.cmp(&a.severity).then_with(|| a.kind.cmp(&b.kind)));
            let mut counts: indexmap::IndexMap<(String, String), usize> = indexmap::IndexMap::new();
            for v in &all {
                *counts.entry((v.severity.clone(), v.kind.clone())).or_insert(0) += 1;
            }
            for ((sev, kind), n) in &counts {
                println!("{sev}: {kind} x{n}");
                for v in all.iter().filter(|v| v.severity == *sev && v.kind == *kind).take(4) {
                    println!("    {}", v.description);
                }
            }
            let errors = all.iter().filter(|v| v.severity == "error").count();
            let warnings = all.iter().filter(|v| v.severity == "warning").count();
            println!("kicad drc: {errors} error(s), {warnings} warning(s) (report: {})", ex.pcb.with_extension("drc.json").display());
            if errors > 0 {
                return Err(Error::with_help("KiCad's DRC reports errors", "open the .kicad_pcb in KiCad to inspect them, or read the JSON report"));
            }
            Ok(())
        }
    }
}
