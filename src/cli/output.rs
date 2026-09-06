//! Visualisation, routing and fabrication outputs.

use super::Ctx;
use crate::error::Result;
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
            let opts = crate::viz::pcb::Options {
                layers: a.layers,
                from_bottom: a.from_bottom,
                ratsnest: !a.no_ratsnest,
                labels: !a.no_labels,
                grid: a.grid,
            };
            let svg = crate::viz::pcb::render(&board, &opts)?;
            crate::viz::write_svg_png(&svg, &png, a.width)?;
            Ok(())
        }
    }
}

#[derive(Args)]
pub struct RouteArgs {
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
    crate::route::run(ctx, a)
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
