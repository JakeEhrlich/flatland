//! Simulation studies.

use super::Ctx;
use crate::error::{list_names, suggest, Error, Result};
use crate::schema::*;
use clap::{Args, Subcommand};
use indexmap::IndexMap;

#[derive(Subcommand)]
pub enum SimCmd {
    /// Define (or redefine) a named simulation.
    Add(SimAddArgs),
    /// Run a simulation (all of them if no name is given).
    Run {
        name: Option<String>,
        /// Re-run even if results are up to date.
        #[arg(long)]
        force: bool,
        /// Ambient temperature override in °C.
        #[arg(long)]
        temperature: Option<f64>,
    },
    /// List simulations and whether their results are current.
    List,
    /// Show a simulation's definition and last results summary.
    Show { name: String },
    /// Delete a simulation.
    Remove { name: String },
    /// Print the SPICE netlist that would be simulated.
    Netlist { name: Option<String> },
}

#[derive(Args)]
pub struct SimAddArgs {
    pub name: String,
    /// Analysis: `op`, `tran STEP STOP`, `dc SRC START STOP STEP`, `ac dec N FSTART FSTOP`, `temp START STOP STEP`.
    #[arg(required = true, num_args = 1.., allow_negative_numbers = true)]
    pub analysis: Vec<String>,
    /// Circuit temperature in °C.
    #[arg(long)]
    pub temperature: Option<f64>,
    /// Nominal model temperature (tnom) in °C.
    #[arg(long)]
    pub tnom: Option<f64>,
    /// Vectors to save/plot, e.g. `v(OUT)`, `i(V1)`, `@R1[p]` (repeatable).
    #[arg(long = "probe")]
    pub probes: Vec<String>,
    /// Instance parameter override `R1.value=1k` (repeatable).
    #[arg(short = 'P', long = "param")]
    pub params: Vec<String>,
    /// Extra SPICE line (repeatable), e.g. `.ic v(OUT)=0`.
    #[arg(short = 'x', long = "extra")]
    pub extra: Vec<String>,
    /// `.options` entry `key=value` (repeatable).
    #[arg(short = 'o', long = "option")]
    pub options: Vec<String>,
    #[arg(long)]
    pub description: Option<String>,
}

fn parse_analysis(words: &[String]) -> Result<Analysis> {
    let kind = words[0].to_lowercase();
    let need = |n: usize, usage: &str| -> Result<()> {
        if words.len() != n + 1 {
            return Err(Error::with_help(format!("`{kind}` analysis takes {n} argument(s)"), format!("usage: {usage}")));
        }
        Ok(())
    };
    Ok(match kind.as_str() {
        "op" => {
            need(0, "op")?;
            Analysis::Op
        }
        "tran" => {
            if words.len() < 3 || words.len() > 4 {
                return Err(Error::with_help("`tran` takes STEP STOP [START]", "e.g. `tran 1us 10ms`"));
            }
            Analysis::Tran { step: words[1].clone(), stop: words[2].clone(), start: words.get(3).cloned() }
        }
        "dc" => {
            need(4, "dc SOURCE START STOP STEP, e.g. `dc V1 0 5 0.1`")?;
            Analysis::Dc { source: words[1].clone(), start: words[2].clone(), stop: words[3].clone(), step: words[4].clone() }
        }
        "ac" => {
            need(4, "ac dec|oct|lin POINTS FSTART FSTOP, e.g. `ac dec 10 1 1meg`")?;
            let points = words[2].parse().map_err(|_| Error::msg(format!("`{}` is not a point count", words[2])))?;
            Analysis::Ac { sweep: words[1].to_lowercase(), points, start: words[3].clone(), stop: words[4].clone() }
        }
        "temp" | "temp_sweep" => {
            need(3, "temp START STOP STEP (°C), e.g. `temp -40 125 5`")?;
            let f = |s: &str| s.parse::<f64>().map_err(|_| Error::msg(format!("`{s}` is not a temperature")));
            Analysis::TempSweep { start: f(&words[1])?, stop: f(&words[2])?, step: f(&words[3])? }
        }
        other => {
            let kinds = ["op", "tran", "dc", "ac", "temp"];
            let mut e = Error::msg(format!("unknown analysis `{other}`; analyses are {}", list_names(kinds)));
            if let Some(s) = suggest(other, kinds) {
                e = e.help(s);
            }
            return Err(e);
        }
    })
}

fn kv_map(items: &[String]) -> Result<IndexMap<String, String>> {
    let mut m = IndexMap::new();
    for it in items {
        let (k, v) = it.split_once('=').ok_or_else(|| Error::msg(format!("`{it}` is not `key=value`")))?;
        m.insert(k.trim().to_string(), v.trim().to_string());
    }
    Ok(m)
}

pub fn run_sim(ctx: &Ctx, c: SimCmd) -> Result<()> {
    match c {
        SimCmd::Add(a) => {
            let mut loaded = ctx.load()?;
            let analysis = parse_analysis(&a.analysis)?;
            let sim = Simulation {
                analysis,
                description: a.description,
                temperature: a.temperature,
                nominal_temperature: a.tnom,
                probes: a.probes,
                parameters: kv_map(&a.params)?,
                extra: a.extra,
                options: kv_map(&a.options)?,
            };
            // Validate by generating the netlist.
            let board = ctx.board(&loaded)?;
            crate::sim::netlist::generate(&board, &sim, &a.name)?;
            let replaced = loaded.project.simulations.insert(a.name.clone(), sim).is_some();
            loaded.save()?;
            println!("{} simulation `{}`; run it with `pcb sim run {}`", if replaced { "updated" } else { "added" }, a.name, a.name);
            Ok(())
        }
        SimCmd::Run { name, force, temperature } => {
            let loaded = ctx.load()?;
            let board = ctx.board(&loaded)?;
            let names: Vec<String> = match name {
                Some(n) => {
                    sim_exists(&board.project, &n)?;
                    vec![n]
                }
                None => board.project.simulations.keys().cloned().collect(),
            };
            if names.is_empty() {
                return Err(Error::with_help("no simulations defined", "add one with `pcb sim add <name> op` (or tran/dc/ac/temp)"));
            }
            for n in names {
                crate::sim::run(&board, &loaded.build_dir(), &n, force, temperature)?;
            }
            Ok(())
        }
        SimCmd::List => {
            let loaded = ctx.load()?;
            let board = ctx.board(&loaded)?;
            if board.project.simulations.is_empty() {
                println!("no simulations; add one with `pcb sim add <name> op|tran|dc|ac|temp ...`");
            }
            for (n, s) in &board.project.simulations {
                let state = crate::sim::status(&board, &loaded.build_dir(), n)?;
                println!("{n}: {} — {state}", s.analysis.spice_line());
            }
            Ok(())
        }
        SimCmd::Show { name } => {
            let loaded = ctx.load()?;
            let board = ctx.board(&loaded)?;
            sim_exists(&board.project, &name)?;
            let s = &board.project.simulations[&name];
            println!("{}", serde_json::to_string_pretty(s).unwrap());
            println!("status: {}", crate::sim::status(&board, &loaded.build_dir(), &name)?);
            if let Some(summary) = crate::sim::last_summary(&loaded.build_dir(), &name) {
                println!("{summary}");
            }
            Ok(())
        }
        SimCmd::Remove { name } => {
            let mut loaded = ctx.load()?;
            sim_exists(&loaded.project, &name)?;
            loaded.project.simulations.shift_remove(&name);
            loaded.save()
        }
        SimCmd::Netlist { name } => {
            let loaded = ctx.load()?;
            let board = ctx.board(&loaded)?;
            let sim = match &name {
                Some(n) => {
                    sim_exists(&board.project, n)?;
                    board.project.simulations[n].clone()
                }
                None => Simulation { analysis: Analysis::Op, description: None, temperature: None, nominal_temperature: None, probes: vec![], parameters: Default::default(), extra: vec![], options: Default::default() },
            };
            let text = crate::sim::netlist::generate(&board, &sim, name.as_deref().unwrap_or("netlist"))?;
            print!("{text}");
            Ok(())
        }
    }
}

fn sim_exists(project: &Project, name: &str) -> Result<()> {
    if project.simulations.contains_key(name) {
        return Ok(());
    }
    let names: Vec<&str> = project.simulations.keys().map(|s| s.as_str()).collect();
    let mut e = Error::msg(format!("no simulation named `{name}`; simulations are {}", list_names(names.iter().copied())));
    if let Some(s) = suggest(name, names.iter().copied()) {
        e = e.help(s);
    }
    Err(e)
}
