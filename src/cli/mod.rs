//! Command line interface. Every command loads the project, mutates or
//! inspects it, and (for mutations) writes it back atomically.

mod assembly;
mod drc_cmd;
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
    /// Free text on silkscreen or copper.
    #[command(subcommand)]
    Text(edit::TextCmd),
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
    /// Design-rule sets, project rules and waivers.
    #[command(subcommand)]
    Drc(drc_cmd::DrcCmd),
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
    let ctx = Ctx::files(cli.project.clone());
    run_command(&ctx, cli.command)
}

/// Dispatch one parsed command against a context (file-backed or in-memory).
pub fn run_command(ctx: &Ctx, command: Command) -> Result<()> {
    let ctx = ctx;
    match command {
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
        Command::Text(c) => edit::run_text(&ctx, c),
        Command::Pour(c) => edit::run_pour(&ctx, c),
        Command::Trace(c) => edit::run_trace(&ctx, c),
        Command::Via(c) => edit::run_via(&ctx, c),
        Command::Stackup(c) => edit::run_stackup(&ctx, c),
        Command::Rules(c) => edit::run_rules(&ctx, c),
        Command::Status(a) => inspect::status(&ctx, a),
        Command::Check(a) => inspect::check(&ctx, a),
        Command::Drc(c) => drc_cmd::run_drc(&ctx, c),
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
    /// When set, the project lives here instead of on disk: `load` clones it,
    /// `save` stores it back, and the component library is cached across
    /// commands. Used by the in-process API (`crate::session`).
    pub memory: Option<std::rc::Rc<std::cell::RefCell<MemStore>>>,
}

/// In-memory project store for a session.
pub struct MemStore {
    /// Where the project would live; relative URLs resolve against it.
    pub path: PathBuf,
    pub project: Option<Project>,
    lib_cache: Option<(String, std::rc::Rc<Library>)>,
}

impl MemStore {
    pub fn new(path: PathBuf) -> MemStore {
        MemStore { path, project: None, lib_cache: None }
    }
}

/// A loaded project plus where it lives.
pub struct Loaded {
    pub path: PathBuf,
    pub project: Project,
    sink: Option<std::rc::Rc<std::cell::RefCell<MemStore>>>,
}

impl Ctx {
    pub fn files(project_arg: Option<PathBuf>) -> Ctx {
        Ctx { project_arg, memory: None }
    }
    pub fn in_memory(store: std::rc::Rc<std::cell::RefCell<MemStore>>) -> Ctx {
        Ctx { project_arg: None, memory: Some(store) }
    }
    pub fn load(&self) -> Result<Loaded> {
        if let Some(m) = &self.memory {
            let m_ref = m.borrow();
            let project = m_ref.project.clone().ok_or_else(|| crate::error::Error::with_help("no project in this session yet", "run `init <name>` first, or open an existing project file"))?;
            return Ok(Loaded { path: m_ref.path.clone(), project, sink: Some(m.clone()) });
        }
        let path = store::find_project(self.project_arg.as_deref())?;
        let project = store::load_project(&path)?;
        Ok(Loaded { path, project, sink: None })
    }
    /// Create the project (`pcb init`): written to `path`, or stored in memory.
    pub fn create(&self, path: &Path, project: &Project, force: bool) -> Result<PathBuf> {
        if let Some(m) = &self.memory {
            let mut m = m.borrow_mut();
            m.project = Some(project.clone());
            m.lib_cache = None;
            return Ok(m.path.clone());
        }
        if path.exists() && !force {
            return Err(crate::error::Error::with_help(format!("`{}` already exists", path.display()), "pass --force to overwrite it, or choose another --dir"));
        }
        store::write_json(path, project)?;
        Ok(path.to_path_buf())
    }
    pub fn library(&self, loaded: &Loaded) -> Result<std::rc::Rc<Library>> {
        if let Some(m) = &self.memory {
            let key = serde_json::to_string(&loaded.project.component_indexes).unwrap_or_default();
            if let Some((k, lib)) = &m.borrow().lib_cache {
                if *k == key {
                    return Ok(lib.clone());
                }
            }
            let lib = std::rc::Rc::new(Library::load(&loaded.project, &loaded.path)?);
            m.borrow_mut().lib_cache = Some((key, lib.clone()));
            return Ok(lib);
        }
        Ok(std::rc::Rc::new(Library::load(&loaded.project, &loaded.path)?))
    }
    pub fn board(&self, loaded: &Loaded) -> Result<crate::model::Board> {
        let lib = self.library(loaded)?;
        crate::model::Board::build(loaded.project.clone(), &loaded.path, &lib)
    }
}

impl Loaded {
    pub fn save(&self) -> Result<()> {
        if let Some(sink) = &self.sink {
            sink.borrow_mut().project = Some(self.project.clone());
            return Ok(());
        }
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

pub fn validate(ctx: &Ctx, loaded: &Loaded) -> Result<()> {
    ctx.board(loaded).map(|_| ())
}
