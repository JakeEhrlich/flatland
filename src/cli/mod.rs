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
    /// Restore the project as it was before the last change (up to 20 steps).
    Undo,
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
        Command::Undo => undo(ctx),
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
    /// Previous states, newest last (`pcb undo`).
    pub undo: Vec<Project>,
}

impl MemStore {
    pub fn new(path: PathBuf) -> MemStore {
        MemStore { path, project: None, lib_cache: None, undo: Vec::new() }
    }
}

pub const UNDO_DEPTH: usize = 20;

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
    /// Persist the project, keeping the previous state for `pcb undo`.
    pub fn save(&self) -> Result<()> {
        if let Some(sink) = &self.sink {
            let mut m = sink.borrow_mut();
            if let Some(prev) = m.project.take() {
                m.undo.push(prev);
                if m.undo.len() > UNDO_DEPTH {
                    m.undo.remove(0);
                }
            }
            m.project = Some(self.project.clone());
            return Ok(());
        }
        undo_snapshot(&self.path)?;
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

/// Copy the current project file into `build/undo/` before it is overwritten.
fn undo_snapshot(path: &Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let dir = path.parent().unwrap_or(Path::new(".")).join("build").join("undo");
    std::fs::create_dir_all(&dir).map_err(|e| crate::error::Error::io(format!("could not create `{}`", dir.display()), e))?;
    let existing = undo_files(&dir);
    let next = existing.last().map(|(n, _)| n + 1).unwrap_or(1);
    std::fs::copy(path, dir.join(format!("{next:06}.json"))).map_err(|e| crate::error::Error::io("could not snapshot the project for undo", e))?;
    let mut all = undo_files(&dir);
    while all.len() > UNDO_DEPTH {
        let (_, p) = all.remove(0);
        let _ = std::fs::remove_file(p);
    }
    Ok(())
}

/// (index, path) of the undo snapshots, oldest first.
pub fn undo_files(dir: &Path) -> Vec<(u64, PathBuf)> {
    let mut v: Vec<(u64, PathBuf)> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter_map(|e| {
                    let p = e.path();
                    let n = p.file_stem()?.to_str()?.parse::<u64>().ok()?;
                    Some((n, p))
                })
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

/// `pcb undo`: restore the state before the last change.
pub fn undo(ctx: &Ctx) -> Result<()> {
    if let Some(m) = &ctx.memory {
        let mut m = m.borrow_mut();
        match m.undo.pop() {
            Some(prev) => {
                m.project = Some(prev);
                println!("restored the project as it was before the last change ({} more undo step(s))", m.undo.len());
                return Ok(());
            }
            None => return Err(crate::error::Error::msg("nothing to undo")),
        }
    }
    let path = store::find_project(ctx.project_arg.as_deref())?;
    let dir = path.parent().unwrap_or(Path::new(".")).join("build").join("undo");
    let files = undo_files(&dir);
    let Some((_, last)) = files.last() else {
        return Err(crate::error::Error::with_help("nothing to undo", "snapshots are kept in build/undo/ from the second change on"));
    };
    std::fs::copy(last, &path).map_err(|e| crate::error::Error::io("could not restore the snapshot", e))?;
    std::fs::remove_file(last).ok();
    println!("restored the project as it was before the last change ({} more undo step(s))", files.len() - 1);
    Ok(())
}

pub fn validate(ctx: &Ctx, loaded: &Loaded) -> Result<()> {
    ctx.board(loaded).map(|_| ())
}
