//! Drive freerouting: write a DSN, run the router headlessly, import the SES.

use crate::cli::output::RouteArgs;
use crate::cli::{Ctx, Loaded};
use crate::dsn;
use crate::error::{Error, Result};
use crate::store;
use crate::units::{Length, Point};
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
        // Routed traces are regenerated, and so is the plane fanout; hand-drawn
        // copper is kept and protected.
        loaded.project.traces.retain(|t| !t.routed && !t.fanout);
        loaded.project.vias.retain(|v| !v.routed && !v.fanout);
    }
    if a.import.is_none() {
        // Plane nets: freerouting never drops a via from an SMD pad to a plane,
        // so draw those first, as wiring the router must keep.
        let fan = fanout_plane_pads(ctx, &mut loaded, None)?;
        if fan.vias > 0 || !fan.skipped.is_empty() {
            println!("fanout: {} via(s) with stubs for SMD pads on plane net(s) {}", fan.vias, fan.nets.join(", "));
            if !fan.skipped.is_empty() {
                println!("warning: no clear spot for a fanout via at {}; those pads stay unconnected until wired by hand", fan.skipped.join(", "));
            }
            // Keep them even if this run stops at the DSN or the router fails.
            loaded.save()?;
        }
    }
    let board = ctx.board(&loaded)?;
    let build = loaded.build_dir();
    std::fs::create_dir_all(&build).map_err(|e| Error::io(format!("could not create `{}`", build.display()), e))?;
    let dsn_path = build.join(format!("{}.dsn", board.project.name));
    let ses_path = build.join(format!("{}.ses", board.project.name));

    let ses_to_import: PathBuf = if let Some(p) = &a.import {
        p.clone()
    } else {
        let text = dsn::emit::emit(&board, a.planes)?;
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
    // The session's coordinates are rounded to its resolution, so compare within 10 µm.
    let close = |a: Point, b: Point| (a.x.nm() - b.x.nm()).abs() <= 10_000 && (a.y.nm() - b.y.nm()).abs() <= 10_000;
    let hand: Vec<&crate::schema::Trace> = loaded.project.traces.iter().collect();
    let same = |a: &crate::schema::Trace, b: &crate::schema::Trace| {
        a.layer == b.layer
            && a.net == b.net
            && (a.width.nm() - b.width.nm()).abs() <= 10_000
            && a.points.len() == b.points.len()
            && (a.points.iter().zip(&b.points).all(|(p, q)| close(*p, *q)) || a.points.iter().rev().zip(&b.points).all(|(p, q)| close(*p, *q)))
    };
    let new_traces: Vec<crate::schema::Trace> = session.traces.into_iter().filter(|t| !hand.iter().any(|h| same(h, t))).collect();
    // The same for vias the router echoes back.
    let hand_vias: Vec<crate::schema::Via> = loaded.project.vias.iter().cloned().collect();
    let new_vias: Vec<crate::schema::Via> = session.vias.into_iter().filter(|v| !hand_vias.iter().any(|h| close(h.at, v.at) && h.net == v.net)).collect();
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
        let (clipped, removed) = clip_covered_traces(ctx, &mut loaded, true)?;
        if clipped + removed > 0 {
            println!("fill covers {removed} router trace(s) entirely (removed) and stretches of {clipped} more (cut back to what the fill does not cover)");
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

pub struct Fanout {
    pub vias: usize,
    pub nets: Vec<String>,
    pub skipped: Vec<String>,
}

/// Draw a via (with a short stub) from every SMD pad of a plane net that
/// does not already touch a via or trace of its net, so the pad reaches the
/// plane. `nets`: only these (default: the nets of the board's plane layers).
/// The via goes outward from the part's centre, past the pad by the
/// clearance, at alternating distances for neighbouring pads so the mask
/// openings keep a dam, and only where it clears every other net's copper,
/// every other via and the board edge; pads with no clear spot are skipped.
pub fn fanout_plane_pads(ctx: &Ctx, loaded: &mut Loaded, nets: Option<&[String]>) -> Result<Fanout> {
    use crate::geom;
    let board = ctx.board(loaded)?;
    let rules = board.rules().clone();
    let planes = board.plane_layers()?;
    let mut target_nets: Vec<String> = match nets {
        Some(n) => n.to_vec(),
        None => planes.iter().map(|(_, n)| n.clone()).collect(),
    };
    target_nets.sort();
    target_nets.dedup();
    let mut out = Fanout { vias: 0, nets: target_nets.clone(), skipped: vec![] };
    if target_nets.is_empty() {
        return Ok(out);
    }
    let Some(outline) = board.outline.clone() else { return Ok(out) };
    let layers: Vec<String> = board.layers().to_vec();
    // Other-net copper per layer (pours excluded: they re-fill around the via).
    let mut obstacles: Vec<(String, Option<String>, Vec<geom::Ring>)> = Vec::new();
    for layer in &layers {
        for (net, ring) in board.copper_on_layer(layer) {
            obstacles.push((layer.clone(), net, vec![ring]));
        }
    }
    let mut via_circles: Vec<geom::Ring> = board.project.vias.iter().map(|v| geom::circle(v.at, v.diameter)).collect();
    let mask_rings: Vec<(String, String, geom::Ring)> = board.all_pads().filter(|p| p.plated).flat_map(|p| geom::offset(&[p.copper.clone()], rules.mask_expansion).into_iter().map(move |r| (p.refdes.clone(), p.pad_name.clone(), r))).collect();
    let dam = Length::from_mm(0.15);
    let mut new_vias: Vec<crate::schema::Via> = Vec::new();
    let mut new_traces: Vec<crate::schema::Trace> = Vec::new();
    for inst in &board.instances {
        let Some(pl) = inst.instance.placement.as_ref() else { continue };
        let mut fan_index = 0usize;
        for pad in &inst.pads {
            let Some(net) = pad.net.clone() else { continue };
            if pad.pad_type != crate::schema::PadType::Smd || !target_nets.contains(&net) {
                continue;
            }
            let Some(layer) = pad.layers.first().cloned() else { continue };
            // A pad on a layer that carries this net's pour reaches it through the fill.
            if board.pours()?.iter().any(|p| p.pour.layer == layer && p.pour.net.as_deref() == Some(net.as_str())) {
                continue;
            }
            // Already reached by a via or a trace of its net?
            let touched = board.project.vias.iter().chain(new_vias.iter()).any(|v| v.net.as_deref() == Some(net.as_str()) && geom::overlaps(&[geom::circle(v.at, v.diameter)], &[pad.copper.clone()]))
                || board.project.traces.iter().any(|t| t.net.as_deref() == Some(net.as_str()) && t.layer == layer && geom::overlaps(&geom::stroke_flat(&t.points, t.width), &[pad.copper.clone()]));
            if touched {
                continue;
            }
            let (class_w, clearance) = rules.class(board.project.nets.get(&net).and_then(|n| n.class.as_deref()));
            let (via_d, via_h) = match board.project.nets.get(&net).and_then(|n| n.class.as_deref()).and_then(|c| rules.net_classes.get(c)) {
                Some(c) => (c.via_diameter.unwrap_or(rules.via_diameter), c.via_drill.unwrap_or(rules.via_drill)),
                None => (rules.via_diameter, rules.via_drill),
            };
            let width = Length::from_nm(class_w.nm().min(pad.size[0].nm().min(pad.size[1].nm())));
            let (cx, cy) = (pad.center.x.mm(), pad.center.y.mm());
            let (mut ux, mut uy) = (cx - pl.at.x.mm(), cy - pl.at.y.mm());
            let len = (ux * ux + uy * uy).sqrt();
            if len < 1e-6 {
                ux = 1.0;
                uy = 0.0;
            } else {
                ux /= len;
                uy /= len;
            }
            let ext = pad.copper.iter().map(|p| (p.x.mm() - cx) * ux + (p.y.mm() - cy) * uy).fold(0.0f64, f64::max);
            let margin = clearance.mm().max(0.25);
            let mut base = ext + via_d.mm() / 2.0 + margin;
            if fan_index % 2 == 1 {
                base += via_d.mm() + 2.0 * rules.mask_expansion.mm() + dam.mm();
            }
            let step = via_d.mm() / 2.0 + 0.2;
            let edge_keep = geom::offset(&[outline.clone()], -(Length::from_nm(via_d.nm() / 2) + rules.edge_clearance));
            let mut placed = None;
            for k in 0..5 {
                let d = base + k as f64 * step;
                let at = Point::mm(cx + ux * d, cy + uy * d);
                if !edge_keep.iter().any(|r| geom::contains(r, at)) {
                    continue;
                }
                let circle = geom::circle(at, via_d);
                let grown = geom::offset(&[circle.clone()], clearance);
                let stub = geom::stroke_flat(&[pad.center, at], width);
                let stub_grown = geom::offset(&stub, clearance);
                let mut ok = true;
                for (l, onet, rings) in &obstacles {
                    if onet.as_deref() == Some(net.as_str()) {
                        continue;
                    }
                    if geom::overlaps(&grown, rings) || (*l == layer && geom::overlaps(&stub_grown, rings)) {
                        ok = false;
                        break;
                    }
                }
                if ok {
                    let hole_keep = geom::offset(&[circle.clone()], Length::from_nm(clearance.nm().max(300_000)));
                    ok = !via_circles.iter().any(|c| geom::overlaps(&hole_keep, &[c.clone()]));
                }
                if ok {
                    let mask_keep = geom::offset(&[circle.clone()], rules.mask_expansion + dam);
                    ok = !mask_rings.iter().any(|(r, p, ring)| !(*r == inst.refdes && *p == pad.pad_name) && geom::overlaps(&mask_keep, &[ring.clone()]));
                }
                if ok {
                    placed = Some((at, circle));
                    break;
                }
            }
            match placed {
                Some((at, circle)) => {
                    via_circles.push(circle);
                    new_vias.push(crate::schema::Via { at, net: Some(net.clone()), drill: via_h, diameter: via_d, layers: vec![], routed: false, fanout: true });
                    new_traces.push(crate::schema::Trace { layer: layer.clone(), net: Some(net.clone()), width, points: vec![pad.center, at], routed: false, fanout: true });
                    out.vias += 1;
                    fan_index += 1;
                }
                None => out.skipped.push(format!("{}.{}", inst.refdes, pad.pad_name)),
            }
        }
    }
    loaded.project.vias.extend(new_vias);
    loaded.project.traces.extend(new_traces);
    Ok(out)
}

/// Cut every trace back to the stretches its own net's fill does not cover.
/// A trace running through the fill adds no copper (the fill already covers
/// it), but it clutters the design and blocks later routes; the stretches
/// that cross a clearance channel or reach into a pocket are the ones that
/// matter and stay, each extended a full trace width into the fill so the
/// joint is face to face. Only routed traces unless `routed_only` is false.
/// Returns (traces cut, traces removed entirely); if connectivity would
/// suffer (it should not: only copper the fill also covers is removed) the
/// traces are left alone.
pub fn clip_covered_traces(ctx: &Ctx, loaded: &mut Loaded, routed_only: bool) -> Result<(usize, usize)> {
    let board = ctx.board(loaded)?;
    let before = board.connectivity()?;
    let pours = board.pours()?;
    let mut out: Vec<crate::schema::Trace> = Vec::new();
    let mut pieces_made: Vec<usize> = Vec::new();
    let (mut clipped, mut removed) = (0usize, 0usize);
    for t in &loaded.project.traces {
        let Some(net) = t.net.as_deref() else { out.push(t.clone()); continue };
        if (routed_only && !t.routed) || t.points.len() < 2 {
            out.push(t.clone());
            continue;
        }
        let fill: crate::geom::Rings = pours.iter().filter(|p| p.pour.layer == t.layer && p.pour.net.as_deref() == Some(net)).flat_map(|p| p.copper.iter().cloned()).collect();
        if fill.is_empty() {
            out.push(t.clone());
            continue;
        }
        // Where the centreline may run so that the whole cross-section lies in the fill.
        let core = crate::geom::offset(&fill, -Length::from_nm(t.width.nm() / 2 + 5_000));
        match uncovered_pieces(&t.points, t.width, &core) {
            None => out.push(t.clone()),
            Some(pieces) if pieces.is_empty() => removed += 1,
            Some(pieces) => {
                clipped += 1;
                for pts in pieces {
                    let mut t2 = t.clone();
                    t2.points = pts;
                    pieces_made.push(out.len());
                    out.push(t2);
                }
            }
        }
    }
    if clipped + removed == 0 {
        return Ok((0, 0));
    }
    let saved = std::mem::replace(&mut loaded.project.traces, out);
    let board = ctx.board(loaded)?;
    let after = board.connectivity()?;
    let islands = |conn: &[(String, Vec<Vec<String>>)], net: &str| conn.iter().find(|(m, _)| m == net).map_or(1, |(_, j)| j.len());
    let worse = before.iter().any(|(n, i)| islands(&after, n) > i.len());
    if worse {
        loaded.project.traces = saved;
        return Ok((0, 0));
    }
    // A piece left in a pad's thermal ring parallels the spokes; drop every
    // piece the net does not need.
    for i in pieces_made.into_iter().rev() {
        let t = loaded.project.traces.remove(i);
        let net = t.net.clone().unwrap_or_default();
        let conn = ctx.board(loaded)?.connectivity()?;
        if islands(&conn, &net) > islands(&before, &net) {
            loaded.project.traces.insert(i, t);
        }
    }
    Ok((clipped, removed))
}

/// Split a polyline into the stretches whose centreline is not inside `core`
/// (a polygon set), each stretch extended by `width` into the covered part.
/// `None` when nothing is covered; `Some(empty)` when everything is.
fn uncovered_pieces(points: &[Point], width: Length, core: &[crate::geom::Ring]) -> Option<Vec<Vec<Point>>> {
    let inside = |x: f64, y: f64| {
        let p = Point::nm((x * 1e6).round() as i64, (y * 1e6).round() as i64);
        core.iter().filter(|r| crate::geom::contains(r, p)).count() % 2 == 1
    };
    let xy: Vec<(f64, f64)> = points.iter().map(|p| (p.x.mm(), p.y.mm())).collect();
    let mut cum = vec![0.0f64];
    for w in xy.windows(2) {
        cum.push(cum.last().unwrap() + ((w[1].0 - w[0].0).powi(2) + (w[1].1 - w[0].1).powi(2)).sqrt());
    }
    let total = *cum.last().unwrap();
    if total <= 0.0 {
        return None;
    }
    let at = |s: f64| -> (f64, f64) {
        let s = s.clamp(0.0, total);
        let mut i = 0;
        while i + 2 < cum.len() && cum[i + 1] < s {
            i += 1;
        }
        let len = cum[i + 1] - cum[i];
        let f = if len > 0.0 { (s - cum[i]) / len } else { 0.0 };
        (xy[i].0 + (xy[i + 1].0 - xy[i].0) * f, xy[i].1 + (xy[i + 1].1 - xy[i].1) * f)
    };
    let inside_at = |s: f64| {
        let (x, y) = at(s);
        inside(x, y)
    };
    // Sample, then bisect each transition to a micron.
    let step = (width.mm() / 4.0).clamp(0.005, 0.1);
    let mut samples: Vec<f64> = cum.clone();
    let mut s = 0.0;
    while s < total {
        samples.push(s);
        s += step;
    }
    samples.push(total);
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
    samples.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
    let flags: Vec<bool> = samples.iter().map(|s| inside_at(*s)).collect();
    if flags.iter().all(|f| !*f) {
        return None;
    }
    // Covered intervals in arc length.
    let mut covered: Vec<(f64, f64)> = Vec::new();
    let mut start: Option<f64> = if flags[0] { Some(0.0) } else { None };
    for i in 1..samples.len() {
        if flags[i] != flags[i - 1] {
            let (mut lo, mut hi) = (samples[i - 1], samples[i]);
            for _ in 0..24 {
                let mid = 0.5 * (lo + hi);
                if inside_at(mid) == flags[i - 1] {
                    lo = mid;
                } else {
                    hi = mid;
                }
                if hi - lo < 1e-4 {
                    break;
                }
            }
            let x = 0.5 * (lo + hi);
            if flags[i] {
                start = Some(x);
            } else if let Some(a) = start.take() {
                covered.push((a, x));
            }
        }
    }
    if let Some(a) = start {
        covered.push((a, total));
    }
    if covered.is_empty() {
        return None;
    }
    // Complement, extended by a trace width into the fill, merged.
    let margin = width.mm();
    let mut keep: Vec<(f64, f64)> = Vec::new();
    let mut cursor = 0.0;
    for (a, b) in &covered {
        if *a > cursor {
            keep.push(((cursor - margin).max(0.0), (*a + margin).min(total)));
        }
        cursor = *b;
    }
    if cursor < total {
        keep.push(((cursor - margin).max(0.0), total));
    }
    let mut merged: Vec<(f64, f64)> = Vec::new();
    for (a, b) in keep {
        match merged.last_mut() {
            Some(last) if a <= last.1 => last.1 = last.1.max(b),
            _ => merged.push((a, b)),
        }
    }
    let mut pieces: Vec<Vec<Point>> = Vec::new();
    for (a, b) in merged {
        if b - a < 0.001 {
            continue;
        }
        let mut pts: Vec<(f64, f64)> = vec![at(a)];
        for (i, s) in cum.iter().enumerate() {
            if *s > a && *s < b {
                pts.push(xy[i]);
            }
        }
        pts.push(at(b));
        let mut out: Vec<Point> = Vec::new();
        for (x, y) in pts {
            let p = Point::nm((x * 1e6).round() as i64, (y * 1e6).round() as i64);
            if out.last().map_or(true, |q| (q.x.nm() - p.x.nm()).abs() > 500 || (q.y.nm() - p.y.nm()).abs() > 500) {
                out.push(p);
            }
        }
        if out.len() >= 2 {
            pieces.push(out);
        }
    }
    Some(pieces)
}

fn read_all(mut r: impl std::io::Read) -> String {
    let mut s = String::new();
    let _ = r.read_to_string(&mut s);
    s
}
