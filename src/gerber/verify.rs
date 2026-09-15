//! Verify written gerbers against the netlist, from the files alone.
//!
//! The copper layers and the plated drill file are parsed back, every
//! flash, region and stroke becomes a polygon, the polygons of each layer
//! are unioned into islands, plated drills join the islands they pass
//! through, and the `TO.P` attributes on pad regions say which pad lies in
//! which island. The pads are then grouped by island and compared with the
//! project's nets: a net whose pads land in several groups is open in the
//! fab files; a group holding pads of several nets is a short. This is an
//! independent path from the model's own connectivity (the files, not the
//! rings that produced them), so a writer bug shows up here.

use crate::error::{Error, Result};
use crate::geom::{self, Ring, Rings};
use crate::model::Board;
use crate::units::{Length, Point};
use std::collections::HashMap;
use std::path::Path;

/// One dark object with the pad and net attributes in force when it was drawn.
struct Object {
    rings: Rings,
    pad: Option<(String, String)>,
}

struct Layer {
    index: usize,
    objects: Vec<Object>,
}

struct GerberLayerFile {
    layer: Option<usize>,
    copper: bool,
}

fn file_function(text: &str) -> GerberLayerFile {
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("%TF.FileFunction,") {
            let v = rest.trim_end_matches("*%");
            let parts: Vec<&str> = v.split(',').collect();
            if parts.first() == Some(&"Copper") {
                let n = parts.get(1).and_then(|s| s.strip_prefix('L')).and_then(|s| s.parse::<usize>().ok());
                return GerberLayerFile { layer: n, copper: true };
            }
            return GerberLayerFile { layer: None, copper: false };
        }
    }
    GerberLayerFile { layer: None, copper: false }
}

/// Parse the subset of RS-274X that `pcb gerbers` writes: circle apertures,
/// flashes, linear strokes and G36/G37 regions, with X2 object attributes.
fn parse_gerber(path: &Path, text: &str) -> Result<Vec<Object>> {
    let fail = |msg: String| Error::msg(format!("{}: {msg}", path.display()));
    let mut scale_x = 1i64; // multiplier from file integer to nm
    let mut scale_y = 1i64;
    let mut apertures: HashMap<u32, Length> = HashMap::new();
    let mut current: Option<u32> = None;
    let mut pad: Option<(String, String)> = None;
    let mut objects: Vec<Object> = Vec::new();
    let mut in_region = false;
    let mut region: Vec<Point> = Vec::new();
    let mut pos = Point::nm(0, 0);
    let mut stroke: Vec<Point> = Vec::new();
    let mut stroke_width: Option<Length> = None;
    let mut polarity_dark = true;
    let flush_stroke = |stroke: &mut Vec<Point>, width: Option<Length>, objects: &mut Vec<Object>, pad: &Option<(String, String)>| {
        if stroke.len() >= 2 {
            if let Some(w) = width {
                objects.push(Object { rings: geom::stroke(stroke, w), pad: pad.clone() });
            }
        }
        stroke.clear();
    };
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(ext) = line.strip_prefix('%').and_then(|s| s.strip_suffix("*%")) {
            if let Some(fs) = ext.strip_prefix("FSLAX") {
                // e.g. 46Y46
                let xd = fs.chars().nth(1).and_then(|c| c.to_digit(10)).ok_or_else(|| fail("bad FS".into()))? as u32;
                let yd = fs.split('Y').nth(1).and_then(|s| s.chars().nth(1)).and_then(|c| c.to_digit(10)).ok_or_else(|| fail("bad FS".into()))? as u32;
                scale_x = 10i64.pow(6 - xd.min(6));
                scale_y = 10i64.pow(6 - yd.min(6));
            } else if ext == "MOIN" {
                return Err(fail("inch units are not supported by the verifier".into()));
            } else if let Some(ad) = ext.strip_prefix("ADD") {
                // ADD10C,0.600000
                let (code, rest) = ad.split_at(ad.find(|c: char| !c.is_ascii_digit()).unwrap_or(ad.len()));
                let code: u32 = code.parse().map_err(|_| fail(format!("bad aperture `{ad}`")))?;
                if let Some(d) = rest.strip_prefix("C,") {
                    let mm: f64 = d.parse().map_err(|_| fail(format!("bad aperture `{ad}`")))?;
                    apertures.insert(code, Length::from_nm((mm * 1e6).round() as i64));
                } else {
                    return Err(fail(format!("aperture `{ad}` is not a circle; the verifier reads only what pcb writes")));
                }
            } else if let Some(p) = ext.strip_prefix("TO.P,") {
                let mut it = p.splitn(2, ',');
                let r = it.next().unwrap_or("").to_string();
                let n = it.next().unwrap_or("").to_string();
                pad = Some((r, n));
            } else if ext == "TD" || ext.starts_with("TD,") {
                pad = None;
            } else if ext == "LPC" {
                polarity_dark = false;
            } else if ext == "LPD" {
                polarity_dark = true;
            }
            continue;
        }
        let body = line.trim_end_matches('*');
        if body == "G36" {
            in_region = true;
            region.clear();
            continue;
        }
        if body == "G37" {
            in_region = false;
            if region.len() >= 3 && polarity_dark {
                if region.first() == region.last() {
                    region.pop();
                }
                let ring: Ring = region.clone();
                if geom::signed_area(&ring).abs() > 0.0 {
                    let ring = if geom::signed_area(&ring) < 0.0 { ring.into_iter().rev().collect() } else { ring };
                    objects.push(Object { rings: vec![ring], pad: pad.clone() });
                }
            }
            region.clear();
            continue;
        }
        if body == "M02" {
            break;
        }
        if let Some(d) = body.strip_prefix('D').filter(|s| s.chars().all(|c| c.is_ascii_digit())) {
            flush_stroke(&mut stroke, stroke_width, &mut objects, &pad);
            current = Some(d.parse().map_err(|_| fail(format!("bad D code `{body}`")))?);
            continue;
        }
        if body.starts_with('G') && !body.contains('X') && !body.contains('Y') {
            continue; // G01, G04 comments, G75...
        }
        // Coordinate word: X..Y..Dnn
        let mut x = pos.x.nm();
        let mut y = pos.y.nm();
        let mut op: Option<u32> = None;
        let mut rest = body;
        while !rest.is_empty() {
            let c = rest.chars().next().unwrap();
            let num_end = rest[1..].find(|ch: char| !(ch.is_ascii_digit() || ch == '-' || ch == '+')).map(|i| i + 1).unwrap_or(rest.len());
            let num = &rest[1..num_end];
            match c {
                'X' => x = num.parse::<i64>().map_err(|_| fail(format!("bad coordinate `{body}`")))? * scale_x,
                'Y' => y = num.parse::<i64>().map_err(|_| fail(format!("bad coordinate `{body}`")))? * scale_y,
                'D' => op = Some(num.parse().map_err(|_| fail(format!("bad op `{body}`")))?),
                'G' | 'I' | 'J' => {}
                _ => return Err(fail(format!("unexpected `{body}`"))),
            }
            rest = &rest[num_end..];
        }
        let p = Point::nm(x, y);
        match op {
            Some(1) => {
                if in_region {
                    region.push(p);
                } else {
                    if stroke.is_empty() {
                        stroke.push(pos);
                    }
                    stroke.push(p);
                    stroke_width = current.and_then(|c| apertures.get(&c).copied());
                }
            }
            Some(2) => {
                if in_region {
                    region.clear();
                    region.push(p);
                } else {
                    flush_stroke(&mut stroke, stroke_width, &mut objects, &pad);
                }
            }
            Some(3) => {
                flush_stroke(&mut stroke, stroke_width, &mut objects, &pad);
                let d = current.and_then(|c| apertures.get(&c).copied()).ok_or_else(|| fail("flash without an aperture".into()))?;
                if polarity_dark {
                    objects.push(Object { rings: vec![geom::circle(p, d)], pad: pad.clone() });
                }
            }
            _ => {}
        }
        pos = p;
    }
    flush_stroke(&mut stroke, stroke_width, &mut objects, &pad);
    Ok(objects)
}

/// Plated drill hits: (point, layer span) from an Excellon file.
fn parse_excellon(path: &Path, text: &str) -> Result<(bool, Vec<Point>)> {
    let fail = |msg: String| Error::msg(format!("{}: {msg}", path.display()));
    let mut plated = true;
    let mut hits: Vec<Point> = Vec::new();
    let mut header = true;
    for raw in text.lines() {
        let line = raw.trim();
        if line.starts_with(';') {
            if line.contains("NonPlated") {
                plated = false;
            }
            continue;
        }
        if line == "%" {
            header = false;
            continue;
        }
        if header || line.is_empty() {
            continue;
        }
        if line.starts_with('X') || line.starts_with('Y') {
            let parse_xy = |s: &str| -> Result<Point> {
                let mut x = 0.0f64;
                let mut y = 0.0f64;
                let mut rest = s;
                while !rest.is_empty() {
                    let c = rest.chars().next().unwrap();
                    let end = rest[1..].find(|ch: char| !(ch.is_ascii_digit() || ch == '.' || ch == '-')).map(|i| i + 1).unwrap_or(rest.len());
                    let v: f64 = rest[1..end].parse().map_err(|_| fail(format!("bad drill line `{s}`")))?;
                    match c {
                        'X' => x = v,
                        'Y' => y = v,
                        _ => return Err(fail(format!("bad drill line `{s}`"))),
                    }
                    rest = &rest[end..];
                }
                Ok(Point::nm((x * 1e6).round() as i64, (y * 1e6).round() as i64))
            };
            // A slot `X..Y..G85X..Y..` is joined at both ends.
            for part in line.split("G85") {
                hits.push(parse_xy(part)?);
            }
        }
    }
    Ok((plated, hits))
}

fn inside(rings: &[Ring], p: Point) -> bool {
    rings.iter().filter(|r| geom::contains(r, p)).count() % 2 == 1
}

fn find(parent: &mut Vec<usize>, i: usize) -> usize {
    let mut r = i;
    while parent[r] != r {
        r = parent[r];
    }
    let mut c = i;
    while parent[c] != r {
        let n = parent[c];
        parent[c] = r;
        c = n;
    }
    r
}

/// Read every copper gerber and drill file in `dir` and check the copper
/// against the board's netlist. Returns a one-line summary on success.
pub fn verify_dir(board: &Board, dir: &Path) -> Result<String> {
    let mut layers: Vec<Layer> = Vec::new();
    let mut drills: Vec<Point> = Vec::new();
    let mut drill_files = 0;
    let entries = std::fs::read_dir(dir).map_err(|e| Error::io(format!("could not read `{}`", dir.display()), e))?;
    for entry in entries {
        let path = entry.map_err(|e| Error::io("reading gerber directory", e))?.path();
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
        if ext == "drl" {
            let text = std::fs::read_to_string(&path).map_err(|e| Error::io(format!("could not read `{}`", path.display()), e))?;
            let (plated, hits) = parse_excellon(&path, &text)?;
            if plated {
                drills.extend(hits);
                drill_files += 1;
            }
            continue;
        }
        if !matches!(ext.as_str(), "gtl" | "gbl") && !(ext.starts_with('g') && ext[1..].chars().all(|c| c.is_ascii_digit()) && ext.len() > 1) {
            continue;
        }
        let text = std::fs::read_to_string(&path).map_err(|e| Error::io(format!("could not read `{}`", path.display()), e))?;
        let f = file_function(&text);
        if !f.copper {
            continue;
        }
        let index = f.layer.ok_or_else(|| Error::msg(format!("{}: copper file without a layer number", path.display())))?;
        layers.push(Layer { index, objects: parse_gerber(&path, &text)? });
    }
    if layers.is_empty() {
        return Err(Error::with_help(format!("no copper gerbers in `{}`", dir.display()), "run `pcb gerbers` first"));
    }
    layers.sort_by_key(|l| l.index);
    let _ = drill_files;

    // Islands per layer.
    struct Island {
        rings: Rings,
    }
    let mut islands: Vec<Island> = Vec::new();
    let mut pad_nodes: HashMap<(String, String), Vec<usize>> = HashMap::new();
    for l in &layers {
        let all: Rings = l.objects.iter().flat_map(|o| o.rings.iter().cloned()).collect();
        let eroded = geom::erode_each(&all, Length::from_nm(1_000));
        let unioned = geom::union(&eroded)?;
        let outers: Vec<Ring> = unioned.iter().filter(|r| geom::signed_area(r) > 0.0).cloned().collect();
        let holes: Vec<Ring> = unioned.iter().filter(|r| geom::signed_area(r) < 0.0).cloned().collect();
        let first = islands.len();
        for o in outers {
            let mut rings = vec![o.clone()];
            for h in &holes {
                if h.first().map_or(false, |p| geom::contains(&o, *p)) {
                    rings.push(h.clone());
                }
            }
            islands.push(Island { rings });
        }
        // Which island holds each pad object: the centroid lies inside for every
        // pad shape pcb writes; a few vertices are tried as well for safety.
        for o in &l.objects {
            let Some(pad) = &o.pad else { continue };
            let c = centroid(&o.rings[0]);
            let mut found = None;
            for (i, isl) in islands.iter().enumerate().skip(first) {
                if inside(&isl.rings, c) {
                    found = Some(i);
                    break;
                }
            }
            if found.is_none() {
                // The pad's copper eroded away or the centroid fell in a hole: try vertices.
                'outer: for v in o.rings[0].iter().step_by((o.rings[0].len() / 8).max(1)) {
                    for (i, isl) in islands.iter().enumerate().skip(first) {
                        if inside(&isl.rings, *v) {
                            found = Some(i);
                            break 'outer;
                        }
                    }
                }
            }
            if let Some(i) = found {
                pad_nodes.entry(pad.clone()).or_default().push(i);
            }
        }
    }

    // Union-find over islands; plated drills join every island they pierce.
    let mut parent: Vec<usize> = (0..islands.len()).collect();
    let mut drill_joins = 0usize;
    for d in &drills {
        let touched: Vec<usize> = islands.iter().enumerate().filter(|(_, isl)| inside(&isl.rings, *d)).map(|(i, _)| i).collect();
        for w in touched.windows(2) {
            let (a, b) = (find(&mut parent, w[0]), find(&mut parent, w[1]));
            if a != b {
                parent[a] = b;
                drill_joins += 1;
            }
        }
    }
    let _ = drill_joins;

    // The solder tabs of one pin (a connector with two tabs per contact) are
    // joined inside the part: union them before reading the groups.
    let mut tabs: HashMap<(String, String), Vec<usize>> = HashMap::new();
    for p in board.all_pads() {
        if let Some(nodes) = pad_nodes.get(&(p.refdes.clone(), p.pad_name.clone())) {
            for pin in &p.pins {
                tabs.entry((p.refdes.clone(), pin.clone())).or_default().extend(nodes.iter().copied());
            }
        }
    }
    for (_, nodes) in tabs {
        for w in nodes.windows(2) {
            let (a, b) = (find(&mut parent, w[0]), find(&mut parent, w[1]));
            if a != b {
                parent[a] = b;
            }
        }
    }

    // Group pads by root.
    let mut problems: Vec<String> = Vec::new();
    let mut pad_root: HashMap<(String, String), usize> = HashMap::new();
    for (pad, nodes) in &pad_nodes {
        let roots: Vec<usize> = {
            let mut v: Vec<usize> = nodes.iter().map(|n| find(&mut parent, *n)).collect();
            v.sort_unstable();
            v.dedup();
            v
        };
        if roots.len() > 1 {
            problems.push(format!("pad {}.{} is copper on {} layers that nothing joins (no plated drill through it in the drill file)", pad.0, pad.1, roots.len()));
        }
        pad_root.insert(pad.clone(), roots[0]);
    }

    // Expected: every pad of the board with its net.
    let mut expected: Vec<((String, String), Option<String>)> = Vec::new();
    for p in board.all_pads() {
        if p.plated {
            expected.push(((p.refdes.clone(), p.pad_name.clone()), p.net.clone()));
        }
    }
    for (i, h) in board.holes.iter().enumerate() {
        if h.copper.is_some() {
            expected.push((("HOLE".to_string(), i.to_string()), h.hole.net.clone()));
        }
    }
    let mut net_roots: HashMap<String, Vec<(usize, String)>> = HashMap::new();
    let mut root_nets: HashMap<usize, Vec<(String, String)>> = HashMap::new();
    let mut missing = 0usize;
    for (pad, net) in &expected {
        let Some(root) = pad_root.get(pad) else {
            missing += 1;
            problems.push(format!("pad {}.{} has no copper in the gerbers", pad.0, pad.1));
            continue;
        };
        let label = format!("{}.{}", pad.0, pad.1);
        if let Some(n) = net {
            net_roots.entry(n.clone()).or_default().push((*root, label.clone()));
            root_nets.entry(*root).or_default().push((n.clone(), label));
        } else {
            root_nets.entry(*root).or_default().push((String::new(), label));
        }
    }
    let mut opens = 0usize;
    let mut nets: Vec<&String> = net_roots.keys().collect();
    nets.sort();
    for n in nets {
        let pads = &net_roots[n];
        let mut groups: Vec<(usize, Vec<&str>)> = Vec::new();
        for (r, l) in pads {
            match groups.iter_mut().find(|(g, _)| g == r) {
                Some(g) => g.1.push(l),
                None => groups.push((*r, vec![l])),
            }
        }
        if groups.len() > 1 {
            opens += 1;
            let parts: Vec<String> = groups.iter().map(|(_, ls)| ls.join("+")).collect();
            problems.push(format!("open: net {n} is {} separate pieces of copper in the gerbers: {}", groups.len(), parts.join(" | ")));
        }
    }
    let mut shorts = 0usize;
    let mut roots: Vec<&usize> = root_nets.keys().collect();
    roots.sort();
    for r in roots {
        let pads = &root_nets[r];
        let mut names: Vec<&str> = pads.iter().map(|(n, _)| n.as_str()).filter(|n| !n.is_empty()).collect();
        names.sort_unstable();
        names.dedup();
        let unnamed: Vec<&str> = pads.iter().filter(|(n, _)| n.is_empty()).map(|(_, l)| l.as_str()).collect();
        if names.len() > 1 || (names.len() == 1 && !unnamed.is_empty()) {
            shorts += 1;
            let mut desc: Vec<String> = names.iter().map(|n| format!("{n} ({})", pads.iter().filter(|(m, _)| m == n).map(|(_, l)| l.as_str()).collect::<Vec<_>>().join(", "))).collect();
            if !unnamed.is_empty() {
                desc.push(format!("unconnected pads ({})", unnamed.join(", ")));
            }
            problems.push(format!("short: one piece of copper in the gerbers joins {}", desc.join(" and ")));
        }
    }
    if !problems.is_empty() {
        for p in &problems {
            eprintln!("error: gerber netlist: {p}");
        }
        return Err(Error::with_help(
            format!("gerbers do not match the netlist: {opens} open net(s), {shorts} short(s), {missing} missing pad(s)"),
            "the copper in the written files was traced back to pads by contact alone; compare with `pcb status` and `pcb check`",
        ));
    }
    Ok(format!(
        "gerbers verified against the netlist: {} pad(s) on {} copper layer(s) trace back to {} net(s) by contact alone, {} plated drill(s) joining layers",
        expected.len(),
        layers.len(),
        net_roots.len(),
        drills.len()
    ))
}

fn centroid(ring: &Ring) -> Point {
    let n = ring.len() as f64;
    let (sx, sy) = ring.iter().fold((0.0f64, 0.0f64), |(a, b), p| (a + p.x.nm() as f64, b + p.y.nm() as f64));
    Point::nm((sx / n).round() as i64, (sy / n).round() as i64)
}
