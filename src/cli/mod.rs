//! Command line interface. Every command loads the project, mutates or
//! inspects it, and (for mutations) writes it back atomically.

mod assembly;
mod edit;
mod index_cmd;
mod inspect;
pub mod output;
mod sim_cmd;

use crate::error::Result;
use crate::schema::Project;
use crate::store::{self, Library};
use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(name = "pcb", version, about = "Agent-oriented PCB design tools", long_about = None)]
#[command(propagate_version = true)]
pub struct Cli {
    /// Project file (defaults to the nearest `pcb.json`, or `$PCB_PROJECT`).
    #[arg(short, long, global = true, value_name = "PATH")]
    pub project: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Create a new project (`pcb.json`) in a directory.
    Init(edit::InitArgs),
    /// Manage component indexes (create, add to project, list, verify, update hashes).
    #[command(subcommand)]
    Index(index_cmd::IndexCmd),
    /// Inspect components available from the project's indexes.
    #[command(subcommand)]
    Component(index_cmd::ComponentCmd),
    /// Add a component instance to the board.
    Add(edit::AddArgs),
    /// Remove a component instance (and its pins from nets).
    Remove(edit::RemoveArgs),
    /// Set an instance parameter (`pcb set R1 value=10k`).
    Set(edit::SetArgs),
    /// Connect two or more pins (`R1.1 U1.VCC ...`) into one net.
    Connect(edit::ConnectArgs),
    /// Remove a pin from its net.
    Disconnect(edit::DisconnectArgs),
    /// Manage nets (list, rename, show, set class).
    #[command(subcommand)]
    Net(edit::NetCmd),
    /// Manage the board outline (lines and arcs).
    #[command(subcommand)]
    Outline(edit::OutlineCmd),
    /// Place (or move/rotate/flip) a component on the board.
    Place(edit::PlaceArgs),
    /// Remove a component's placement.
    Unplace(edit::UnplaceArgs),
    /// Move, resize or hide a part's silkscreen reference label.
    Label(edit::LabelArgs),
    /// Manage drilled holes that are not part of a footprint.
    #[command(subcommand)]
    Hole(edit::HoleCmd),
    /// Manage copper pours (filled zones).
    #[command(subcommand)]
    Pour(edit::PourCmd),
    /// Manage hand-drawn traces and vias.
    #[command(subcommand)]
    Trace(edit::TraceCmd),
    /// Manage vias.
    #[command(subcommand)]
    Via(edit::ViaCmd),
    /// Manage the layer stackup.
    #[command(subcommand)]
    Stackup(edit::StackupCmd),
    /// Manage design rules and net classes.
    #[command(subcommand)]
    Rules(edit::RulesCmd),
    /// Summarise the project: components, nets, placement and routing status.
    Status(inspect::StatusArgs),
    /// Check the design: connectivity, clearances, outline, unplaced parts.
    Check(inspect::CheckArgs),
    /// List pad positions and nets of placed parts (for hand routing).
    Pads(inspect::PadsArgs),
    /// Render the netlist (schematic-style graph) or the board to PNG/SVG.
    #[command(subcommand)]
    Visualize(output::VisualizeCmd),
    /// Autoroute with freerouting (writes a .dsn, runs the router, imports the .ses).
    Route(output::RouteArgs),
    /// Emit fabrication outputs: gerbers, drill files and a zip.
    Gerbers(output::GerbersArgs),
    /// Write the bill of materials (JLCPCB format by default).
    Bom(assembly::BomArgs),
    /// Write the pick-and-place / component placement list (JLCPCB CPL format by default).
    Pnp(assembly::PnpArgs),
    /// Define and run SPICE simulations.
    #[command(subcommand)]
    Sim(sim_cmd::SimCmd),
    /// Print the JSON schema conventions used by project/index/component/footprint files.
    Schema(inspect::SchemaArgs),
    /// Print the manual page for a command family (`pcb docs` lists them).
    Docs(inspect::DocsArgs),
}

pub fn run() -> Result<()> {
    let cli = Cli::parse();
    let ctx = Ctx { project_arg: cli.project.clone() };
    match cli.command {
        Command::Init(a) => edit::init(&ctx, a),
        Command::Index(c) => index_cmd::run_index(&ctx, c),
        Command::Component(c) => index_cmd::run_component(&ctx, c),
        Command::Add(a) => edit::add(&ctx, a),
        Command::Remove(a) => edit::remove(&ctx, a),
        Command::Set(a) => edit::set(&ctx, a),
        Command::Connect(a) => edit::connect(&ctx, a),
        Command::Disconnect(a) => edit::disconnect(&ctx, a),
        Command::Net(c) => edit::run_net(&ctx, c),
        Command::Outline(c) => edit::run_outline(&ctx, c),
        Command::Place(a) => edit::place(&ctx, a),
        Command::Unplace(a) => edit::unplace(&ctx, a),
        Command::Label(a) => edit::label(&ctx, a),
        Command::Hole(c) => edit::run_hole(&ctx, c),
        Command::Pour(c) => edit::run_pour(&ctx, c),
        Command::Trace(c) => edit::run_trace(&ctx, c),
        Command::Via(c) => edit::run_via(&ctx, c),
        Command::Stackup(c) => edit::run_stackup(&ctx, c),
        Command::Rules(c) => edit::run_rules(&ctx, c),
        Command::Status(a) => inspect::status(&ctx, a),
        Command::Check(a) => inspect::check(&ctx, a),
        Command::Pads(a) => inspect::pads(&ctx, a),
        Command::Visualize(c) => output::run_visualize(&ctx, c),
        Command::Route(a) => output::route(&ctx, a),
        Command::Gerbers(a) => output::gerbers(&ctx, a),
        Command::Bom(a) => assembly::bom(&ctx, a),
        Command::Pnp(a) => assembly::pnp(&ctx, a),
        Command::Sim(c) => sim_cmd::run_sim(&ctx, c),
        Command::Schema(a) => inspect::schema(&ctx, a),
        Command::Docs(a) => inspect::docs(&ctx, a),
    }
}

pub struct Ctx {
    pub project_arg: Option<PathBuf>,
}

/// A loaded project plus where it lives.
pub struct Loaded {
    pub path: PathBuf,
    pub project: Project,
}

impl Ctx {
    pub fn load(&self) -> Result<Loaded> {
        let path = store::find_project(self.project_arg.as_deref())?;
        let project = store::load_project(&path)?;
        Ok(Loaded { path, project })
    }
    pub fn library(&self, loaded: &Loaded) -> Result<Library> {
        Library::load(&loaded.project, &loaded.path)
    }
    pub fn board(&self, loaded: &Loaded) -> Result<crate::model::Board> {
        let lib = self.library(loaded)?;
        crate::model::Board::build(loaded.project.clone(), &loaded.path, &lib)
    }
}

impl Loaded {
    pub fn save(&self) -> Result<()> {
        store::write_json(&self.path, &self.project)
    }
    pub fn dir(&self) -> &Path {
        self.path.parent().unwrap_or(Path::new("."))
    }
    /// Output directory for generated files (`build/` next to the project).
    pub fn build_dir(&self) -> PathBuf {
        self.dir().join("build")
    }
}

/// Validate the whole project after an edit; on failure the edit is *not*
/// saved and the error explains why.
pub fn validate(ctx: &Ctx, loaded: &Loaded) -> Result<()> {
    ctx.board(loaded).map(|_| ())
}
