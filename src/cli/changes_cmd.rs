//! `pcb changes` and `pcb diff`.

use super::Ctx;
use crate::changes::{self, Change};
use crate::error::{Error, Result};
use crate::store;
use clap::{Args, Subcommand};
use std::path::PathBuf;

#[derive(Args)]
pub struct ChangesArgs {
    #[command(subcommand)]
    pub sub: Option<ChangesSub>,
}

#[derive(Subcommand)]
pub enum ChangesSub {
    /// List the pending changes as pcb commands and Python calls (the default).
    List {
        /// Print the change list file as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Apply the changes to the project and write the result as `pcb-target.json`
    /// (the project file itself is not touched).
    Apply {
        /// Output file (default `pcb-target.json` next to the project).
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Apply even if the project file changed since the changes were recorded.
        #[arg(long)]
        force: bool,
    },
    /// Delete the change list.
    Clear,
}

pub fn changes(ctx: &Ctx, a: ChangesArgs) -> Result<()> {
    let loaded = ctx.load()?;
    let path = loaded.path.clone();
    match a.sub.unwrap_or(ChangesSub::List { json: false }) {
        ChangesSub::List { json } => {
            let Some(c) = changes::load(&path)? else {
                println!("no pending changes ({} not present)", changes::changes_path(&path).display());
                return Ok(());
            };
            if json {
                println!("{}", serde_json::to_string_pretty(&c).unwrap());
                return Ok(());
            }
            let current = changes::is_current(&path, &c)?;
            println!(
                "{} change(s) recorded against {} (blake3 {}…){}",
                c.changes.len(),
                c.base,
                &c.base_blake3[..12],
                if current { "" } else { " — STALE: the project file changed since; `pcb changes clear` or apply with --force" }
            );
            // Replay the list so an entry without a recorded `was` (a hand-written
            // file) still says what it replaces.
            let mut replay = loaded.project.clone();
            for (i, e) in c.changes.iter().enumerate() {
                let (cli, py) = changes::as_commands(&e.change);
                let was = e.was.clone().or_else(|| if current { changes::describe_before(&replay, &e.change) } else { None });
                let _ = changes::apply_one(&mut replay, &e.change);
                println!("\n{}. {}", i + 1, changes::summary(&e.change));
                if let Some(w) = was {
                    println!("   was: {w}");
                }
                println!("   pcb:    {cli}");
                println!("   python: {py}");
            }
            if !c.changes.is_empty() {
                println!("\n`pcb changes apply` writes these into {}; `pcb diff` compares the project against it.", changes::TARGET);
            }
            Ok(())
        }
        ChangesSub::Apply { output, force } => {
            let Some(c) = changes::load(&path)? else {
                return Err(Error::msg("no pending changes"));
            };
            if !changes::is_current(&path, &c)? && !force {
                return Err(Error::with_help(
                    format!("the changes were recorded against a different {} (blake3 {}…)", c.base, &c.base_blake3[..12]),
                    "the project file changed since; review them with `pcb changes`, then `pcb changes apply --force` or `pcb changes clear`",
                ));
            }
            let target = changes::apply_all(&loaded.project, &c)?;
            let out = output.unwrap_or_else(|| changes::target_path(&path));
            store::write_json(&out, &target)?;
            let notes = c.changes.iter().filter(|e| matches!(e.change, Change::Note { .. })).count();
            println!("wrote {} with {} change(s) applied{}", out.display(), c.changes.len() - notes, if notes > 0 { format!(" ({notes} note(s) are for you to read: `pcb changes`)") } else { String::new() });
            println!("rebuild the project and run `pcb diff` until it reports no differences");
            Ok(())
        }
        ChangesSub::Clear => {
            let p = changes::changes_path(&path);
            if p.exists() {
                std::fs::remove_file(&p).map_err(|e| Error::io(format!("could not remove `{}`", p.display()), e))?;
                println!("removed {}", p.display());
            } else {
                println!("no pending changes");
            }
            Ok(())
        }
    }
}

#[derive(Args)]
pub struct ServeArgs {
    /// Port to listen on (0 picks a free one).
    #[arg(long, default_value = "7350")]
    pub port: u16,
    /// Open the page in the default browser.
    #[arg(long)]
    pub open: bool,
}

pub fn serve(ctx: &Ctx, a: ServeArgs) -> Result<()> {
    let path = ctx.load()?.path;
    crate::serve::serve(&path, a.port, a.open)
}

#[derive(Args)]
pub struct DiffArgs {
    /// First project file (default: the project).
    pub a: Option<PathBuf>,
    /// Second project file (default: `pcb-target.json` next to the project).
    pub b: Option<PathBuf>,
}

pub fn diff(ctx: &Ctx, a: DiffArgs) -> Result<()> {
    let (pa, pb) = match (a.a, a.b) {
        (Some(x), Some(y)) => (x, y),
        (Some(x), None) => (ctx.load()?.path, x),
        (None, _) => {
            let p = ctx.load()?.path;
            let t = changes::target_path(&p);
            (p, t)
        }
    };
    if !pb.exists() {
        return Err(Error::with_help(format!("`{}` does not exist", pb.display()), "`pcb changes apply` writes it"));
    }
    let proj_a = store::load_project(&pa)?;
    let proj_b = store::load_project(&pb)?;
    let (na, nb) = (pa.file_name().unwrap().to_string_lossy().to_string(), pb.file_name().unwrap().to_string_lossy().to_string());
    let lines = crate::diff::diff(&proj_a, &proj_b, &na, &nb);
    if lines.is_empty() {
        println!("{na} and {nb} describe the same board");
        return Ok(());
    }
    for l in &lines {
        println!("{l}");
    }
    Err(Error::msg(format!("{} difference(s) between {na} and {nb}", lines.len())))
}
