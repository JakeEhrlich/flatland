//! SPICE simulation studies via ngspice's shared library.

pub mod netlist;
pub mod ngspice;
pub mod plot;

use crate::error::{Error, Result};
use crate::model::Board;
use crate::schema::Analysis;
use crate::store;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize)]
pub struct Manifest {
    pub name: String,
    pub input_hash: String,
    pub analysis: String,
    pub finished_at: String,
    pub scale: Option<String>,
    pub vectors: Vec<String>,
    pub points: usize,
    pub files: Vec<String>,
    pub summary: String,
}

pub fn sim_dir(build: &Path, name: &str) -> PathBuf {
    build.join("sim").join(name)
}

fn manifest(build: &Path, name: &str) -> Option<Manifest> {
    let p = sim_dir(build, name).join("manifest.json");
    let text = std::fs::read_to_string(p).ok()?;
    serde_json::from_str(&text).ok()
}

/// "not run" / "up to date" / "stale".
pub fn status(board: &Board, build: &Path, name: &str) -> Result<String> {
    let sim = &board.project.simulations[name];
    let text = netlist::generate(board, sim, name)?;
    let hash = store::blake3_hex(text.as_bytes());
    Ok(match manifest(build, name) {
        None => "not run".into(),
        Some(m) if m.input_hash == hash => format!("up to date ({})", m.finished_at),
        Some(m) => format!("stale — inputs changed since {} (rerun with `pcb sim run {name}`)", m.finished_at),
    })
}

pub fn last_summary(build: &Path, name: &str) -> Option<String> {
    manifest(build, name).map(|m| m.summary)
}

pub fn run(board: &Board, build: &Path, name: &str, force: bool, temperature: Option<f64>) -> Result<()> {
    let mut sim = board.project.simulations[name].clone();
    if let Some(t) = temperature {
        sim.temperature = Some(t);
    }
    let text = netlist::generate(board, &sim, name)?;
    let hash = store::blake3_hex(text.as_bytes());
    let dir = sim_dir(build, name);
    if !force && temperature.is_none() {
        if let Some(m) = manifest(build, name) {
            if m.input_hash == hash {
                println!("{name}: results are up to date ({}); pass --force to rerun", dir.display());
                println!("{}", m.summary);
                return Ok(());
            }
        }
    }
    std::fs::create_dir_all(&dir).map_err(|e| Error::io(format!("could not create `{}`", dir.display()), e))?;
    let cir = dir.join("netlist.cir");
    std::fs::write(&cir, &text).map_err(|e| Error::io(format!("could not write `{}`", cir.display()), e))?;

    for w in text.lines().filter_map(|l| l.strip_prefix("* warning: ")) {
        eprintln!("warning: {w}");
    }
    if let Some(skipped) = text.lines().find_map(|l| l.strip_prefix("* not simulated (no spice model): ")) {
        eprintln!("note: not simulated (no spice model): {skipped}");
    }
    println!("{name}: running {} …", sim.analysis.spice_line());
    let mut engine = ngspice::Ngspice::load()?;
    let result = engine.run(&text, matches!(sim.analysis, Analysis::Op))?;
    std::fs::write(dir.join("ngspice.log"), &result.log).ok();

    if result.vectors.is_empty() {
        return Err(Error::with_help(
            format!("ngspice produced no data for `{name}`"),
            format!("see {} for the simulator output", dir.join("ngspice.log").display()),
        ));
    }
    // Select vectors: probes if given, else everything sensible.
    let selected = select_vectors(&result, &sim.probes)?;
    let scale_name = result.scale.clone();
    let mut files = vec!["netlist.cir".to_string(), "ngspice.log".to_string()];

    // CSV
    let csv_path = dir.join("results.csv");
    let mut csv = String::new();
    let cols: Vec<&ngspice::Vector> = {
        let mut v = Vec::new();
        if let Some(s) = &scale_name {
            if let Some(sv) = result.vectors.iter().find(|x| &x.name == s) {
                v.push(sv);
            }
        }
        for s in &selected {
            if Some(&s.name) != scale_name.as_ref() {
                v.push(s);
            }
        }
        v
    };
    csv.push_str(&cols.iter().map(|c| if c.unit.is_empty() { c.name.clone() } else { format!("{} ({})", c.name, c.unit) }).collect::<Vec<_>>().join(","));
    csv.push('\n');
    let n = cols.iter().map(|c| c.values.len()).max().unwrap_or(0);
    for i in 0..n {
        let row: Vec<String> = cols.iter().map(|c| c.values.get(i).map(|v| format_val(*v, c.is_complex, c.imag.get(i).copied())).unwrap_or_default()).collect();
        csv.push_str(&row.join(","));
        csv.push('\n');
    }
    std::fs::write(&csv_path, &csv).map_err(|e| Error::io(format!("could not write `{}`", csv_path.display()), e))?;
    files.push("results.csv".into());

    // Summary & plot
    let summary = summarize(&sim.analysis, &result, &selected);
    println!("{summary}");
    if n > 1 {
        if let Some(scale) = scale_name.as_ref().and_then(|s| result.vectors.iter().find(|x| &x.name == s)) {
            let log_x = matches!(sim.analysis, Analysis::Ac { .. });
            let title = format!("{} — {} ({})", board.project.name, name, sim.analysis.spice_line());
            let svg = plot::plot(&title, scale, &selected, log_x, matches!(sim.analysis, Analysis::Ac { .. }));
            let png = dir.join("plot.png");
            crate::viz::write_svg_png(&svg, &png, 1600)?;
            files.push("plot.png".into());
            files.push("plot.svg".into());
        }
    }
    let now = time::OffsetDateTime::now_utc();
    let m = Manifest {
        name: name.to_string(),
        input_hash: hash,
        analysis: sim.analysis.spice_line(),
        finished_at: format!("{:04}-{:02}-{:02} {:02}:{:02} UTC", now.year(), now.month() as u8, now.day(), now.hour(), now.minute()),
        scale: scale_name,
        vectors: selected.iter().map(|v| v.name.clone()).collect(),
        points: n,
        files,
        summary,
    };
    store::write_json(&dir.join("manifest.json"), &m)?;
    println!("results in {}", dir.display());
    Ok(())
}

fn format_val(v: f64, complex: bool, im: Option<f64>) -> String {
    if complex {
        format!("{v:.6e}{:+.6e}j", im.unwrap_or(0.0))
    } else {
        format!("{v:.6e}")
    }
}

/// Map a probe like `v(OUT)`, `i(V1)`, `@R1[p]` to ngspice vector names.
fn probe_matches(probe: &str, vec_name: &str) -> bool {
    let p = probe.trim().to_lowercase();
    let v = vec_name.to_lowercase();
    if p == v {
        return true;
    }
    if let Some(inner) = p.strip_prefix("v(").and_then(|s| s.strip_suffix(')')) {
        return v == inner || v == format!("v({inner})");
    }
    if let Some(inner) = p.strip_prefix("i(").and_then(|s| s.strip_suffix(')')) {
        return v == format!("{inner}#branch") || v == format!("i({inner})");
    }
    false
}

fn select_vectors<'a>(result: &'a ngspice::RunResult, probes: &[String]) -> Result<Vec<&'a ngspice::Vector>> {
    if probes.is_empty() {
        return Ok(result
            .vectors
            .iter()
            .filter(|v| Some(&v.name) != result.scale.as_ref())
            .collect());
    }
    let mut out = Vec::new();
    for p in probes {
        match result.vectors.iter().find(|v| probe_matches(p, &v.name)) {
            Some(v) => out.push(v),
            None => {
                let names: Vec<&str> = result.vectors.iter().map(|v| v.name.as_str()).collect();
                return Err(Error::with_help(
                    format!("probe `{p}` matched no simulation vector; available vectors: {}", crate::error::list_names(names.iter().copied())),
                    "probes look like `v(NET)`, `i(V1)` or `@r1[p]` (net names are case-insensitive)",
                ));
            }
        }
    }
    Ok(out)
}

fn si(v: f64) -> String {
    let a = v.abs();
    let (scale, suffix) = if a == 0.0 {
        (1.0, "")
    } else if a >= 1e9 {
        (1e9, "G")
    } else if a >= 1e6 {
        (1e6, "M")
    } else if a >= 1e3 {
        (1e3, "k")
    } else if a >= 1.0 {
        (1.0, "")
    } else if a >= 1e-3 {
        (1e-3, "m")
    } else if a >= 1e-6 {
        (1e-6, "u")
    } else if a >= 1e-9 {
        (1e-9, "n")
    } else if a >= 1e-12 {
        (1e-12, "p")
    } else {
        (1e-15, "f")
    };
    let s = format!("{:.4}", v / scale);
    let s = s.trim_end_matches('0').trim_end_matches('.');
    format!("{s}{suffix}")
}

fn summarize(analysis: &Analysis, result: &ngspice::RunResult, selected: &[&ngspice::Vector]) -> String {
    let mut s = String::new();
    match analysis {
        Analysis::Op => {
            s.push_str("operating point:\n");
            for v in selected {
                if let Some(x) = v.values.first() {
                    s.push_str(&format!("  {:<20} {}{}\n", v.name, si(*x), v.unit));
                }
            }
        }
        _ => {
            let scale = result.scale.as_deref().unwrap_or("index");
            let n = selected.first().map(|v| v.values.len()).unwrap_or(0);
            s.push_str(&format!("{n} points over {scale}"));
            if let Some(sv) = result.vectors.iter().find(|v| Some(&v.name) == result.scale.as_ref()) {
                if let (Some(a), Some(b)) = (sv.values.first(), sv.values.last()) {
                    s.push_str(&format!(" from {}{u} to {}{u}", si(*a), si(*b), u = sv.unit));
                }
            }
            s.push_str(":\n");
            for v in selected {
                if v.values.is_empty() {
                    continue;
                }
                let min = v.values.iter().cloned().fold(f64::INFINITY, f64::min);
                let max = v.values.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
                let last = v.values.last().copied().unwrap_or(0.0);
                if v.is_complex {
                    let mags: Vec<f64> = v.values.iter().zip(v.imag.iter()).map(|(r, i)| (r * r + i * i).sqrt()).collect();
                    let mmax = mags.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
                    let mmin = mags.iter().cloned().fold(f64::INFINITY, f64::min);
                    s.push_str(&format!("  {:<20} |min| {}{u}  |max| {}{u}\n", v.name, si(mmin), si(mmax), u = v.unit));
                } else {
                    s.push_str(&format!("  {:<20} min {}{u}  max {}{u}  final {}{u}\n", v.name, si(min), si(max), si(last), u = v.unit));
                }
            }
        }
    }
    s.trim_end().to_string()
}
