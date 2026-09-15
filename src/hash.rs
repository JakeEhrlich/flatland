//! A semantic hash of a board: what the fab makes and what the assembler
//! places, and nothing else. Silkscreen, solder mask, labels, notes,
//! simulations, rule sets and file layout do not count; the order in
//! which traces, vias, holes or pours were written does not count, nor the
//! direction a trace was drawn in or whether a router drew it. Two project
//! files with the same hash describe the same copper, drills, outline and
//! parts, so the hash names a revision.
//!
//! The canonical text is deterministic and readable (`pcb hash --canonical`)
//! and the hash is blake3 over it, so collisions are as rare as blake3's.

use crate::model::Board;
use crate::units::{Length, Point};
use std::fmt::Write;

fn nm(l: Length) -> i64 {
    l.nm()
}

fn pt(p: Point) -> String {
    format!("{},{}", p.x.nm(), p.y.nm())
}

fn pts(points: &[Point]) -> String {
    points.iter().map(|p| pt(*p)).collect::<Vec<_>>().join(";")
}

fn canon_points(points: &[Point]) -> Vec<Point> {
    let fwd: Vec<(i64, i64)> = points.iter().map(|p| (p.x.nm(), p.y.nm())).collect();
    let rev: Vec<(i64, i64)> = fwd.iter().rev().copied().collect();
    if rev < fwd {
        points.iter().rev().copied().collect()
    } else {
        points.to_vec()
    }
}

/// Rotate a closed ring so it starts at its lexicographically smallest
/// vertex, winding with positive area: the same polygon written from any
/// vertex in either direction canonicalizes to one string.
fn canon_ring(ring: &[Point]) -> Vec<Point> {
    if ring.is_empty() {
        return vec![];
    }
    let mut r: Vec<Point> = ring.to_vec();
    if crate::geom::signed_area(&r) < 0.0 {
        r.reverse();
    }
    let start = (0..r.len()).min_by_key(|&i| (r[i].x.nm(), r[i].y.nm())).unwrap_or(0);
    r.rotate_left(start);
    r
}

fn sorted(mut v: Vec<String>) -> Vec<String> {
    v.sort();
    v
}

/// The canonical text and its blake3 hex.
pub fn semantic(board: &Board) -> (String, String) {
    let p = &board.project;
    let mut s = String::new();
    let st = board.stackup();
    let _ = writeln!(s, "stackup {} thickness {} copper {}", st.copper_layers.join(","), nm(st.board_thickness), nm(st.copper_thickness));
    // Rules that shape copper; mask, paste and silk rules do not.
    let r = board.rules();
    let _ = writeln!(
        s,
        "rules trace_width {} clearance {} via {}/{} pour_clearance {} thermal_gap {} spoke {} pour_min_width {} hole_clearance {} hole_ring {}",
        nm(r.trace_width),
        nm(r.clearance),
        nm(r.via_drill),
        nm(r.via_diameter),
        r.pour_clearance.map(nm).unwrap_or(-1),
        r.thermal_gap.map(nm).unwrap_or(-1),
        nm(r.thermal_spoke_width),
        r.pour_min_width.map(nm).unwrap_or(-1),
        r.hole_clearance.map(nm).unwrap_or(-1),
        nm(r.hole_annular_ring)
    );
    let mut classes: Vec<String> = r
        .net_classes
        .iter()
        .map(|(k, c)| format!("class {k} width {} clearance {} via {}/{}", c.trace_width.map(nm).unwrap_or(-1), c.clearance.map(nm).unwrap_or(-1), c.via_drill.map(nm).unwrap_or(-1), c.via_diameter.map(nm).unwrap_or(-1)))
        .collect();
    classes.sort();
    for c in classes {
        let _ = writeln!(s, "{c}");
    }
    match &board.outline {
        Some(o) => {
            let _ = writeln!(s, "outline {}", pts(&canon_ring(o)));
        }
        None => {
            let _ = writeln!(s, "outline none");
        }
    }
    for h in sorted(p.holes.iter().map(|h| format!("hole {} drill {} plated {} ring {} net {}", pt(h.at), nm(h.drill), h.plated, h.diameter.map(nm).unwrap_or(-1), h.net.as_deref().unwrap_or("-"))).collect()) {
        let _ = writeln!(s, "{h}");
    }
    // Parts: identity, parameters, placement and the resolved pad copper (so a
    // library footprint change counts). Labels, notes and locks do not.
    let mut parts: Vec<String> = Vec::new();
    for inst in &board.instances {
        if inst.component.is_virtual() {
            continue;
        }
        let mut params: Vec<String> = inst.instance.parameters.iter().map(|(k, v)| format!("{k}={v}")).collect();
        params.sort();
        let placement = match &inst.instance.placement {
            Some(pl) => format!("at {} rot {} side {}", pt(pl.at), pl.rotation.rem_euclid(360.0), pl.side),
            None => "unplaced".into(),
        };
        let mut pads: Vec<String> = inst
            .pads
            .iter()
            .map(|pad| {
                let mut layers = pad.layers.clone();
                layers.sort();
                format!(
                    "  pad {} pins {} net {} layers {} plated {} drill {} slot {} ring {}",
                    pad.pad_name,
                    pad.pins.join("+"),
                    pad.net.as_deref().unwrap_or("-"),
                    layers.join("+"),
                    pad.plated,
                    pad.drill.map(nm).unwrap_or(-1),
                    pad.drill_length.map(nm).unwrap_or(-1),
                    pts(&canon_ring(&pad.copper))
                )
            })
            .collect();
        pads.sort();
        parts.push(format!("part {} component {} params [{}] {}\n{}", inst.refdes, inst.component.name, params.join(","), placement, pads.join("\n")));
    }
    parts.sort();
    for part in parts {
        let _ = writeln!(s, "{part}");
    }
    let mut nets: Vec<String> = p
        .nets
        .iter()
        .map(|(name, n)| {
            let mut pins = n.pins.clone();
            pins.sort();
            format!("net {name} class {} pins {}", n.class.as_deref().unwrap_or("-"), pins.join(","))
        })
        .collect();
    nets.sort();
    for n in nets {
        let _ = writeln!(s, "{n}");
    }
    // Copper drawn by hand or by a router: a union, so a multiset.
    for t in sorted(p.traces.iter().map(|t| format!("trace {} net {} width {} {}", t.layer, t.net.as_deref().unwrap_or("-"), nm(t.width), pts(&canon_points(&t.points)))).collect()) {
        let _ = writeln!(s, "{t}");
    }
    for v in sorted(
        p.vias
            .iter()
            .map(|v| {
                let mut layers = v.layers.clone();
                layers.sort();
                format!("via {} net {} drill {} dia {} layers {}", pt(v.at), v.net.as_deref().unwrap_or("-"), nm(v.drill), nm(v.diameter), if layers.is_empty() { "all".to_string() } else { layers.join("+") })
            })
            .collect(),
    ) {
        let _ = writeln!(s, "{v}");
    }
    for pr in sorted(
        p.pours
            .iter()
            .map(|pr| {
                let outline = crate::geom::edges_to_ring(&pr.edges, "pour").map(|r| pts(&canon_ring(&r))).unwrap_or_else(|_| format!("{:?}", serde_json::to_string(&pr.edges).unwrap_or_default()));
                format!("pour {} net {} priority {} clearance {} thermal {} {}", pr.layer, pr.net.as_deref().unwrap_or("-"), pr.priority, pr.clearance.map(nm).unwrap_or(-1), pr.thermal.map(|b| b.to_string()).unwrap_or_else(|| "default".into()), outline)
            })
            .collect(),
    ) {
        let _ = writeln!(s, "{pr}");
    }
    // Text is copper only when it sits on a copper layer.
    let copper_layers = board.layers();
    for t in sorted(p.texts.iter().filter(|t| copper_layers.iter().any(|l| *l == t.layer)).map(|t| format!("text {} at {} size {} rot {} width {} {:?}", t.layer, pt(t.at), nm(t.size), t.rotation, t.width.map(nm).unwrap_or(-1), t.text)).collect()) {
        let _ = writeln!(s, "{t}");
    }
    let hex = crate::store::blake3_hex(s.as_bytes());
    (s, hex)
}
