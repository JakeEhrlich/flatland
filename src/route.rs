//! Drive freerouting: write a DSN, run the router headlessly, import the SES.

use crate::cli::output::RouteArgs;
use crate::cli::{Ctx, Loaded};
use crate::dsn;
use crate::error::{Error, Result};
use crate::store;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const CANDIDATE_LAUNCHERS: &[&str] = &[
    "/Applications/freerouting.app/Contents/MacOS/freerouting",
    "~/freerouting/freerouting.jar",
    "~/freerouting.jar",
    "/usr/local/share/freerouting/freerouting.jar",
    "/opt/freerouting/freerouting.jar",
];
const CANDIDATE_JAVAS: &[&str] = &[
    "/opt/homebrew/opt/openjdk/bin/java",
    "/opt/homebrew/opt/openjdk@25/bin/java",
    "/opt/homebrew/opt/openjdk@21/bin/java",
    "/usr/local/opt/openjdk/bin/java",
    "/usr/lib/jvm/default-java/bin/java",
];

fn expand(p: &str) -> PathBuf {
    if let Some(rest) = p.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(p)
}

fn find_freerouting(explicit: Option<&Path>, configured: Option<&str>) -> Result<PathBuf> {
    if let Some(p) = explicit {
        if p.exists() {
            return Ok(p.to_path_buf());
        }
        return Err(Error::msg(format!("freerouting not found at `{}`", p.display())));
    }
    if let Some(c) = configured {
        let p = expand(c);
        if p.exists() {
            return Ok(p);
        }
        return Err(Error::with_help(
            format!("freerouting not found at `{}` (from pcb.json `routing.freerouting`)", p.display()),
            "fix the path in pcb.json or pass --freerouting",
        ));
    }
    if let Some(env) = std::env::var_os("FREEROUTING") {
        let p = PathBuf::from(env);
        if p.exists() {
            return Ok(p);
        }
        return Err(Error::msg(format!("$FREEROUTING points at `{}` which does not exist", p.display())));
    }
    for c in CANDIDATE_LAUNCHERS {
        let p = expand(c);
        if p.exists() {
            return Ok(p);
        }
    }
    Err(Error::with_help(
        "freerouting was not found",
        "install it from https://github.com/freerouting/freerouting/releases, then pass `--freerouting <path-to-jar-or-app-launcher>`, \
         set $FREEROUTING, or put the path in pcb.json under `routing.freerouting`",
    ))
}

fn find_java(explicit: Option<&Path>, configured: Option<&str>) -> Result<PathBuf> {
    if let Some(p) = explicit {
        return Ok(p.to_path_buf());
    }
    if let Some(c) = configured {
        return Ok(expand(c));
    }
    if let Some(home) = std::env::var_os("JAVA_HOME") {
        let p = PathBuf::from(home).join("bin/java");
        if p.exists() {
            return Ok(p);
        }
    }
    for c in CANDIDATE_JAVAS {
        let p = expand(c);
        if p.exists() {
            return Ok(p);
        }
    }
    Ok(PathBuf::from("java"))
}

pub fn run(ctx: &Ctx, a: RouteArgs) -> Result<()> {
    let mut loaded = ctx.load()?;
    if !a.keep && a.import.is_none() {
        // Routed traces are regenerated; hand-drawn ones are kept and protected.
        loaded.project.traces.retain(|t| !t.routed);
        loaded.project.vias.retain(|v| !v.routed);
    }
    let board = ctx.board(&loaded)?;
    let build = loaded.build_dir();
    std::fs::create_dir_all(&build).map_err(|e| Error::io(format!("could not create `{}`", build.display()), e))?;
    let dsn_path = build.join(format!("{}.dsn", board.project.name));
    let ses_path = build.join(format!("{}.ses", board.project.name));

    let ses_to_import: PathBuf = if let Some(p) = &a.import {
        p.clone()
    } else {
        let text = dsn::emit::emit(&board)?;
        std::fs::write(&dsn_path, &text).map_err(|e| Error::io(format!("could not write `{}`", dsn_path.display()), e))?;
        println!("wrote {}", dsn_path.display());
        if a.dsn_only {
            println!("route it in freerouting and import the session with `pcb route --import <file.ses>`");
            return Ok(());
        }
        let unrouted: usize = board.connectivity()?.iter().map(|(_, i)| i.len().saturating_sub(1)).sum();
        if unrouted == 0 {
            println!("nothing to route: every net is already connected");
            return Ok(());
        }
        let cfg = board.project.routing.clone().unwrap_or_default();
        let launcher = find_freerouting(a.freerouting.as_deref(), cfg.freerouting.as_deref())?;
        let _ = std::fs::remove_file(&ses_path);
        let passes = a.passes.or(cfg.max_passes).unwrap_or(20);
        let mut cmd = if launcher.extension().map_or(false, |e| e == "jar") {
            let java = find_java(a.java.as_deref(), cfg.java.as_deref())?;
            let mut c = Command::new(&java);
            c.arg("-jar").arg(&launcher);
            c
        } else {
            Command::new(&launcher)
        };
        // Headless batch mode: design in, session out, no confirmation dialogs.
        cmd.arg("-de").arg(&dsn_path).arg("-do").arg(&ses_path).arg("-mp").arg(passes.to_string()).arg("-dct").arg("0").arg("-da").arg("-dl");
        cmd.arg("--gui.enabled=false");
        cmd.env("JAVA_TOOL_OPTIONS", "-Djava.awt.headless=true");
        cmd.current_dir(&build);
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).stdin(Stdio::null());
        println!("running freerouting ({}) with up to {passes} passes, {} unrouted connection(s)…", launcher.display(), unrouted);
        let log_path = build.join("freerouting.log");
        let start = Instant::now();
        let mut child = cmd.spawn().map_err(|e| {
            Error::io(format!("could not start `{}`", launcher.display()), e)
                .help("if this is a jar, pass --java <path to a Java 21+ executable>")
        })?;
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let out_thread = std::thread::spawn(move || read_all(stdout));
        let err_thread = std::thread::spawn(move || read_all(stderr));
        let status = loop {
            match child.try_wait() {
                Ok(Some(st)) => break st,
                Ok(None) => {
                    if start.elapsed() > Duration::from_secs(a.timeout) {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err(Error::with_help(
                            format!("freerouting did not finish within {} s", a.timeout),
                            format!("pass a larger `--timeout`, fewer `--passes`, or route by hand from {}", dsn_path.display()),
                        ));
                    }
                    std::thread::sleep(Duration::from_millis(200));
                }
                Err(e) => return Err(Error::io("waiting for freerouting failed", e)),
            }
        };
        let out = out_thread.join().unwrap_or_default();
        let err = err_thread.join().unwrap_or_default();
        let log = format!("{out}\n{err}");
        let _ = std::fs::write(&log_path, &log);
        if log.contains("UnsupportedClassVersionError") {
            return Err(Error::with_help(
                "freerouting needs a newer Java than the one used",
                format!("install a recent JDK (e.g. `brew install openjdk`) and pass `--java /opt/homebrew/opt/openjdk/bin/java`; see {}", log_path.display()),
            ));
        }
        if !ses_path.exists() {
            let tail: Vec<&str> = log.lines().rev().take(15).collect::<Vec<_>>().into_iter().rev().collect();
            return Err(Error::with_help(
                format!("freerouting exited with {status} without writing {}", ses_path.display()),
                format!("last log lines:\n{}\n(full log: {})", tail.join("\n"), log_path.display()),
            ));
        }
        println!("freerouting finished in {:.1} s; log at {}", start.elapsed().as_secs_f64(), log_path.display());
        ses_path.clone()
    };

    let text = store::read_text(&ses_to_import)?;
    let session = dsn::ses::parse_session(&ses_to_import, &text, board.layers())?;
    // freerouting echoes protected (hand-drawn) wiring back in the session;
    // do not store those again as routed copies.
    let hand: Vec<&crate::schema::Trace> = loaded.project.traces.iter().filter(|t| !t.routed).collect();
    let same = |a: &crate::schema::Trace, b: &crate::schema::Trace| {
        a.layer == b.layer && a.net == b.net && a.width == b.width && (a.points == b.points || a.points.iter().rev().eq(b.points.iter()))
    };
    let new_traces: Vec<crate::schema::Trace> = session.traces.into_iter().filter(|t| !hand.iter().any(|h| same(h, t))).collect();
    // The same for vias the router echoes back.
    let hand_vias: Vec<crate::schema::Via> = loaded.project.vias.iter().filter(|v| !v.routed).cloned().collect();
    let new_vias: Vec<crate::schema::Via> = session.vias.into_iter().filter(|v| !hand_vias.iter().any(|h| h.at == v.at && h.net == v.net)).collect();
    let (nt, nv) = (new_traces.len(), new_vias.len());
    // freerouting models wire ends as round caps and may stop a wide wire just
    // short of a narrow pad (the cap reaches it, the flat end does not) or
    // approach at an angle. Finish such ends at the pad centre.
    let finished = finish_wire_ends(&board, new_traces);
    let extended = finished.1;
    // Wires meeting end to end: merge equal widths into one polyline, extend a
    // narrower one into the wider so the copper overlaps (a shared end line is
    // no joint).
    let (welded, merges, welds) = weld_junctions(finished.0);
    loaded.project.traces.extend(welded);
    loaded.project.vias.extend(new_vias);
    let board = ctx.board(&loaded)?;
    let conn = board.connectivity()?;
    let unrouted: Vec<String> = conn.iter().filter(|(_, i)| i.len() > 1).map(|(n, i)| format!("{n} ({} islands)", i.len())).collect();
    println!("imported {nt} trace segment(s) and {nv} via(s) from {}", ses_to_import.display());
    if extended > 0 {
        println!("extended {extended} wire end(s) to the centre of the pad they stop short of");
    }
    if merges + welds > 0 {
        println!("joined {merges} wire(s) end to end and welded {welds} width change(s)");
    }
    if !a.keep_redundant {
        let dropped = prune_redundant_traces(ctx, &mut loaded, &conn)?;
        if !dropped.is_empty() {
            let total: usize = dropped.iter().map(|(_, n)| n).sum();
            let per: Vec<String> = dropped.iter().map(|(n, c)| format!("{n} {c}")).collect();
            println!("dropped {total} redundant router segment(s) ({}): pours, hand wiring or multi-tab pins already make those connections (keep them with --keep-redundant)", per.join(", "));
        }
    }
    loaded.save()?;
    if unrouted.is_empty() {
        println!("all nets routed");
    } else {
        println!("still unrouted: {}", unrouted.join(", "));
    }
    Ok(())
}

/// For each end of each imported wire: if a same-net pad lies within half the
/// wire width (so the router's round cap would reach it) but the end point is
/// outside the pad copper, append the pad centre. Returns the wires and how
/// many ends were extended.
fn finish_wire_ends(board: &crate::model::Board, wires: Vec<crate::schema::Trace>) -> (Vec<crate::schema::Trace>, usize) {
    use crate::geom;
    let mut n = 0;
    let out = wires
        .into_iter()
        .map(|mut t| {
            let Some(net) = t.net.clone() else { return t };
            let half = crate::units::Length::from_nm(t.width.nm() / 2);
            for end in [true, false] {
                let pt = if end { *t.points.last().unwrap() } else { t.points[0] };
                let target = board.all_pads().find(|p| {
                    p.net.as_deref() == Some(net.as_str())
                        && p.on_layer(&t.layer)
                        && !geom::contains(&p.copper, pt)
                        && geom::overlaps(&[geom::circle(pt, crate::units::Length::from_nm(half.nm() * 2))], &[p.copper.clone()])
                });
                if let Some(p) = target {
                    if end {
                        t.points.push(p.center);
                    } else {
                        t.points.insert(0, p.center);
                    }
                    n += 1;
                }
            }
            t
        })
        .collect();
    (out, n)
}

/// Join wires of one net that meet end to end. Equal widths become one
/// polyline; where the width changes, the narrower wire is extended into the
/// wider one by the wider one's half width so their copper overlaps.
fn weld_junctions(mut wires: Vec<crate::schema::Trace>) -> (Vec<crate::schema::Trace>, usize, usize) {
    use crate::units::Point;
    let tol = 10_000i64;
    let same = |a: Point, b: Point| (a.x.nm() - b.x.nm()).abs() <= tol && (a.y.nm() - b.y.nm()).abs() <= tol;
    let (mut merges, mut welds) = (0usize, 0usize);
    for w in &mut wires {
        w.points.dedup();
    }
    wires.retain(|w| w.points.len() >= 2);
    // Merge pass.
    let mut changed = true;
    while changed {
        changed = false;
        'outer: for i in 0..wires.len() {
            for j in 0..wires.len() {
                if i == j || wires[i].net != wires[j].net || wires[i].layer != wires[j].layer || wires[i].width != wires[j].width {
                    continue;
                }
                let (a_end, b_start, b_end) = (*wires[i].points.last().unwrap(), wires[j].points[0], *wires[j].points.last().unwrap());
                if same(a_end, b_start) || same(a_end, b_end) {
                    let mut b = wires.remove(j);
                    let i2 = if j < i { i - 1 } else { i };
                    if same(*wires[i2].points.last().unwrap(), *b.points.last().unwrap()) {
                        b.points.reverse();
                    }
                    wires[i2].points.extend(b.points.into_iter().skip(1));
                    merges += 1;
                    changed = true;
                    break 'outer;
                }
            }
        }
    }
    // Weld pass: different widths meeting end to end.
    let n = wires.len();
    for i in 0..n {
        for j in 0..n {
            if i == j || wires[i].net != wires[j].net || wires[i].layer != wires[j].layer || wires[i].width >= wires[j].width {
                continue;
            }
            let wide_half = wires[j].width.nm() / 2;
            let ends_j = [wires[j].points[0], *wires[j].points.last().unwrap()];
            for end_of_i in [true, false] {
                let (p_end, p_prev) = if end_of_i {
                    let k = wires[i].points.len();
                    (wires[i].points[k - 1], wires[i].points[k - 2])
                } else {
                    (wires[i].points[0], wires[i].points[1])
                };
                if !ends_j.iter().any(|e| same(*e, p_end)) {
                    continue;
                }
                let (dx, dy) = ((p_end.x.nm() - p_prev.x.nm()) as f64, (p_end.y.nm() - p_prev.y.nm()) as f64);
                let len = dx.hypot(dy);
                if len <= 0.0 {
                    continue;
                }
                let ext = Point::nm(p_end.x.nm() + (dx / len * wide_half as f64).round() as i64, p_end.y.nm() + (dy / len * wide_half as f64).round() as i64);
                let k = wires[i].points.len();
                if end_of_i {
                    wires[i].points[k - 1] = ext;
                } else {
                    wires[i].points[0] = ext;
                }
                welds += 1;
            }
        }
    }
    (wires, merges, welds)
}

/// Remove router segments that add nothing: on boards without planes (one
/// copper layer) freerouting links the pads of a pour's net, which the pour
/// joins again through thermal reliefs; it also re-draws pieces of protected
/// hand wiring and links the tabs of a multi-tab pin. Each routed segment is
/// removed in turn and kept only if the net would otherwise fall apart.
/// Returns per-net counts of removed segments.
fn prune_redundant_traces(ctx: &Ctx, loaded: &mut Loaded, before: &[(String, Vec<Vec<String>>)]) -> Result<Vec<(String, usize)>> {
    let islands_before = |net: &str| before.iter().find(|(n, _)| n == net).map_or(1, |(_, i)| i.len());
    let candidates: Vec<usize> = loaded.project.traces.iter().enumerate().filter(|(_, t)| t.routed && t.net.is_some()).map(|(i, _)| i).collect();
    let mut dropped: Vec<(String, usize)> = Vec::new();
    for i in candidates.into_iter().rev() {
        let t = loaded.project.traces.remove(i);
        let net = t.net.clone().unwrap();
        let board = ctx.board(loaded)?;
        let after = board.connectivity()?.into_iter().find(|(n, _)| *n == net).map_or(1, |(_, i)| i.len());
        if after <= islands_before(&net) {
            match dropped.iter_mut().find(|(n, _)| *n == net) {
                Some(e) => e.1 += 1,
                None => dropped.push((net, 1)),
            }
        } else {
            loaded.project.traces.insert(i, t);
        }
    }
    Ok(dropped)
}

fn read_all(mut r: impl std::io::Read) -> String {
    let mut s = String::new();
    let _ = r.read_to_string(&mut s);
    s
}
