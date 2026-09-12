//! Export the board as a KiCad `.kicad_pcb` (plus a `.kicad_pro` carrying the
//! design rules), so KiCad's own DRC can be run on it as an independent check.
//! KiCad's y axis points down: every y is negated on the way out.

use crate::error::{Error, Result};
use crate::geom::Edge;
use crate::model::Board;
use crate::schema::*;
use crate::units::{Length, Point};
use std::fmt::Write;
use std::path::{Path, PathBuf};

fn mm(l: Length) -> String {
    let v = l.mm();
    let s = format!("{v:.4}");
    let s = s.trim_end_matches('0').trim_end_matches('.').to_string();
    if s == "-0" { "0".into() } else { s }
}
fn xy(p: Point) -> String {
    format!("{} {}", mm(p.x), mm(Length::from_nm(-p.y.nm())))
}
fn q(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

struct Uuids(u64);
impl Uuids {
    fn next(&mut self) -> String {
        self.0 += 1;
        let n = self.0;
        format!("f1a71a4d-{:04x}-4{:03x}-8{:03x}-{:012x}", (n >> 32) & 0xffff, (n >> 20) & 0xfff, (n >> 8) & 0xfff, n & 0xffff_ffff_ffff)
    }
}

fn silk_layer(side: Side) -> &'static str {
    match side {
        Side::Top => "F.SilkS",
        Side::Bottom => "B.SilkS",
    }
}
fn crtyd_layer(side: Side) -> &'static str {
    match side {
        Side::Top => "F.CrtYd",
        Side::Bottom => "B.CrtYd",
    }
}

/// Copper layer name as KiCad wants it. Our names already match (`F.Cu`, `B.Cu`).
fn cu_layer(name: &str) -> String {
    name.to_string()
}

pub struct Export {
    pub pcb: PathBuf,
    pub pro: PathBuf,
}

pub fn write(board: &Board, dir: &Path) -> Result<Export> {
    std::fs::create_dir_all(dir).map_err(|e| Error::io(format!("could not create `{}`", dir.display()), e))?;
    let name = board.project.name.replace(' ', "_");
    let pcb_path = dir.join(format!("{name}.kicad_pcb"));
    let pro_path = dir.join(format!("{name}.kicad_pro"));
    let mut ids = Uuids(0);
    let rules = board.rules();
    let mut s = String::new();
    let thickness = board.stackup().board_thickness;
    let _ = writeln!(s, "(kicad_pcb (version 20240108) (generator \"flatland\") (generator_version \"0.1\")");
    let _ = writeln!(s, "  (general (thickness {}) (legacy_teardrops no))", mm(thickness));
    let _ = writeln!(s, "  (paper \"A4\")");
    // Layers: KiCad boards have at least F.Cu and B.Cu.
    let copper: Vec<String> = board.layers().to_vec();
    let mut layer_lines = vec!["    (0 \"F.Cu\" signal)".to_string()];
    for (i, l) in copper.iter().enumerate() {
        if l != "F.Cu" && l != "B.Cu" {
            layer_lines.push(format!("    ({} {} signal)", i, q(l)));
        }
    }
    layer_lines.push("    (31 \"B.Cu\" signal)".into());
    for (n, name, kind, desc) in [
        (32, "B.Adhes", "user", "B.Adhesive"), (33, "F.Adhes", "user", "F.Adhesive"), (34, "B.Paste", "user", ""), (35, "F.Paste", "user", ""),
        (36, "B.SilkS", "user", "B.Silkscreen"), (37, "F.SilkS", "user", "F.Silkscreen"), (38, "B.Mask", "user", ""), (39, "F.Mask", "user", ""),
        (40, "Dwgs.User", "user", "User.Drawings"), (41, "Cmts.User", "user", "User.Comments"), (44, "Edge.Cuts", "user", ""), (45, "Margin", "user", ""),
        (46, "B.CrtYd", "user", "B.Courtyard"), (47, "F.CrtYd", "user", "F.Courtyard"), (48, "B.Fab", "user", ""), (49, "F.Fab", "user", ""),
    ] {
        layer_lines.push(if desc.is_empty() { format!("    ({n} {} {kind})", q(name)) } else { format!("    ({n} {} {kind} {})", q(name), q(desc)) });
    }
    let _ = writeln!(s, "  (layers\n{}\n  )", layer_lines.join("\n"));
    let _ = writeln!(
        s,
        "  (setup (pad_to_mask_clearance {}) (allow_soldermask_bridges_in_footprints no) (pcbplotparams))",
        mm(rules.mask_expansion)
    );
    // Nets.
    let mut net_ids: Vec<(String, usize)> = Vec::new();
    let _ = writeln!(s, "  (net 0 \"\")");
    for (i, n) in board.nets.iter().enumerate() {
        net_ids.push((n.name.clone(), i + 1));
        let _ = writeln!(s, "  (net {} {})", i + 1, q(&n.name));
    }
    let net_id = |n: &Option<String>| -> Option<usize> { n.as_ref().and_then(|n| net_ids.iter().find(|(m, _)| m == n).map(|(_, i)| *i)) };
    // Pads name the net; segments and vias only number it.
    let net_clause = |n: &Option<String>| -> String {
        match (net_id(n), n) {
            (Some(i), Some(name)) => format!(" (net {i} {})", q(name)),
            _ => String::new(),
        }
    };
    let net_num = |n: &Option<String>| -> String {
        match net_id(n) {
            Some(i) => format!(" (net {i})"),
            None => String::new(),
        }
    };
    // Footprints: emitted at rotation 0 with every pad in its final position, so
    // no assumption about KiCad's rotation or flip conventions is needed.
    for inst in board.instances.iter().filter(|i| i.is_placed()) {
        let pl = inst.instance.placement.as_ref().unwrap();
        let side = pl.side;
        let fp_name = inst.component.footprint.as_ref().map(|f| f.footprint.name.clone()).unwrap_or_else(|| "virtual".into());
        let origin = pl.at;
        let local = |p: Point| Point::nm(p.x.nm() - origin.x.nm(), p.y.nm() - origin.y.nm());
        let _ = writeln!(s, "  (footprint {} (layer {}) (uuid {}) (at {} 0)", q(&format!("flatland:{fp_name}")), q(if side == Side::Top { "F.Cu" } else { "B.Cu" }), q(&ids.next()), xy(origin));
        let label_at = inst.label_at.map(local).unwrap_or(Point::ORIGIN);
        let size = inst.label_size(rules);
        let _ = writeln!(
            s,
            "    (property \"Reference\" {} (at {} 0) (layer {}) {}(uuid {}) (effects (font (size {} {}) (thickness {})){}))",
            q(&inst.refdes),
            xy(label_at),
            q(silk_layer(side)),
            if inst.label_at.is_none() { "(hide yes) " } else { "" },
            q(&ids.next()),
            mm(size),
            mm(size),
            mm(rules.silk_width),
            if side == Side::Bottom { " (justify mirror)" } else { "" }
        );
        let _ = writeln!(s, "    (property \"Value\" {} (at 0 0 0) (layer {}) (hide yes) (uuid {}) (effects (font (size 1 1) (thickness 0.15))))", q(&inst.component.name), q(if side == Side::Top { "F.Fab" } else { "B.Fab" }), q(&ids.next()));
        let any_tht = inst.pads.iter().any(|p| p.pad_type == PadType::ThroughHole);
        let _ = writeln!(s, "    (attr {})", if any_tht { "through_hole" } else { "smd" });
        // A pin with several solder tabs: the tabs share a pad number, and this tells
        // KiCad they are joined inside the part rather than needing copper.
        let multi_tab = inst.pads.iter().any(|p| p.pins.len() == 1 && inst.pads.iter().filter(|o| o.pins == p.pins).count() > 1);
        if multi_tab {
            let _ = writeln!(s, "    (duplicate_pad_numbers_are_jumpers yes)");
        }
        for (pts, closed, w) in &inst.silk_lines {
            let mut pts = pts.clone();
            if *closed {
                if let Some(f) = pts.first().copied() {
                    pts.push(f);
                }
            }
            for seg in pts.windows(2) {
                let _ = writeln!(s, "    (fp_line (start {}) (end {}) (stroke (width {}) (type solid)) (layer {}) (uuid {}))", xy(local(seg[0])), xy(local(seg[1])), mm(*w), q(silk_layer(side)), q(&ids.next()));
            }
        }
        for (pts, _) in &inst.courtyard {
            if pts.len() < 2 {
                continue;
            }
            let mut ring = pts.clone();
            ring.push(pts[0]);
            for seg in ring.windows(2) {
                let _ = writeln!(s, "    (fp_line (start {}) (end {}) (stroke (width 0.05) (type solid)) (layer {}) (uuid {}))", xy(local(seg[0])), xy(local(seg[1])), q(crtyd_layer(side)), q(&ids.next()));
            }
        }
        for pad in &inst.pads {
            // KiCad treats pads with the same number as connected inside the part:
            // a pin with several solder tabs gets its pin name on every tab.
            let number = if pad.pins.len() == 1 && inst.pads.iter().filter(|o| o.pins == pad.pins).count() > 1 { pad.pins[0].clone() } else { pad.pad_name.clone() };
            let at = local(pad.center);
            let (w, h) = (pad.size[0], pad.size[1]);
            let shape = match pad.shape {
                PadShape::Circle => "circle",
                PadShape::Rect => "rect",
                PadShape::RoundRect => "roundrect",
                PadShape::Oval => "oval",
            };
            let rr = match pad.shape {
                PadShape::RoundRect => {
                    let r = pad.corner_radius.map(|r| r.mm()).unwrap_or(0.0);
                    let m = w.mm().min(h.mm());
                    format!(" (roundrect_rratio {:.4})", if m > 0.0 { (r / m).min(0.5) } else { 0.25 })
                }
                _ => String::new(),
            };
            let zone_connect = if !pad.thermal_relief { " (zone_connect 2)" } else { "" };
            match pad.pad_type {
                PadType::Smd => {
                    let side_l = if pad.on_layer("F.Cu") || (copper.len() == 1 && side == Side::Top) { "F" } else { "B" };
                    let layers = format!("{}.Cu {}{}.Mask", side_l, if pad.paste { format!("{side_l}.Paste ") } else { String::new() }, side_l);
                    let _ = writeln!(
                        s,
                        "    (pad {} smd {shape} (at {} {}) (size {} {}) (layers {}){rr}{}{zone_connect} (uuid {}))",
                        q(&number),
                        xy(at),
                        fmt_angle(pad.rotation),
                        mm(w),
                        mm(h),
                        layers.split(' ').map(q).collect::<Vec<_>>().join(" "),
                        net_clause(&pad.net),
                        q(&ids.next())
                    );
                }
                PadType::ThroughHole => {
                    let drill = pad.drill.unwrap_or(Length::from_mm(0.8));
                    let drill_clause = match pad.drill_length {
                        Some(len) => format!("(drill oval {} {})", mm(drill), mm(len)),
                        None => format!("(drill {})", mm(drill)),
                    };
                    if pad.plated {
                        let _ = writeln!(
                            s,
                            "    (pad {} thru_hole {shape} (at {} {}) (size {} {}) {drill_clause} (layers \"*.Cu\" \"*.Mask\"){rr}{}{zone_connect} (uuid {}))",
                            q(&number),
                            xy(at),
                            fmt_angle(pad.rotation),
                            mm(w),
                            mm(h),
                            net_clause(&pad.net),
                            q(&ids.next())
                        );
                    } else {
                        let _ = writeln!(s, "    (pad \"\" np_thru_hole circle (at {}) (size {} {}) {drill_clause} (layers \"*.Cu\" \"*.Mask\") (uuid {}))", xy(at), mm(drill), mm(drill), q(&ids.next()));
                    }
                }
            }
        }
        let _ = writeln!(s, "  )");
    }
    // Free holes as one-pad footprints.
    for (i, h) in board.holes.iter().enumerate() {
        let _ = writeln!(s, "  (footprint \"flatland:hole\" (layer \"F.Cu\") (uuid {}) (at {} 0)", q(&ids.next()), xy(h.hole.at));
        let _ = writeln!(s, "    (property \"Reference\" \"H{}\" (at 0 0 0) (layer \"F.SilkS\") (hide yes) (uuid {}) (effects (font (size 1 1) (thickness 0.15))))", i + 1, q(&ids.next()));
        let _ = writeln!(s, "    (property \"Value\" \"hole\" (at 0 0 0) (layer \"F.Fab\") (hide yes) (uuid {}) (effects (font (size 1 1) (thickness 0.15))))", q(&ids.next()));
        let _ = writeln!(s, "    (attr through_hole)");
        if h.hole.plated {
            let d = h.hole.diameter.unwrap_or(h.hole.drill + rules.hole_annular_ring * 2);
            let _ = writeln!(s, "    (pad \"1\" thru_hole circle (at 0 0) (size {} {}) (drill {}) (layers \"*.Cu\" \"*.Mask\"){} (uuid {}))", mm(d), mm(d), mm(h.hole.drill), net_clause(&h.hole.net), q(&ids.next()));
        } else {
            let _ = writeln!(s, "    (pad \"\" np_thru_hole circle (at 0 0) (size {} {}) (drill {}) (layers \"*.Cu\" \"*.Mask\") (uuid {}))", mm(h.hole.drill), mm(h.hole.drill), mm(h.hole.drill), q(&ids.next()));
        }
        let _ = writeln!(s, "  )");
    }
    // Outline.
    for e in &board.project.outline {
        match e {
            Edge::Line { from, to } => {
                let _ = writeln!(s, "  (gr_line (start {}) (end {}) (stroke (width 0.05) (type default)) (layer \"Edge.Cuts\") (uuid {}))", xy(*from), xy(*to), q(&ids.next()));
            }
            Edge::Arc { from, to, center, clockwise } => {
                let mid = arc_mid(*from, *to, *center, *clockwise);
                let _ = writeln!(s, "  (gr_arc (start {}) (mid {}) (end {}) (stroke (width 0.05) (type default)) (layer \"Edge.Cuts\") (uuid {}))", xy(*from), xy(mid), xy(*to), q(&ids.next()));
            }
        }
    }
    // Free text.
    for t in &board.project.texts {
        let layer = match t.layer.as_str() {
            SILK_TOP => "F.SilkS".to_string(),
            SILK_BOTTOM => "B.SilkS".to_string(),
            other => cu_layer(other),
        };
        let w = t.width.unwrap_or(if t.silk_side().is_some() { rules.silk_width } else { rules.trace_width });
        let _ = writeln!(s, "  (gr_text {} (at {} {}) (layer {}) (uuid {}) (effects (font (size {} {}) (thickness {}))))", q(&t.text), xy(t.at), fmt_angle(t.rotation), q(&layer), q(&ids.next()), mm(t.size), mm(t.size), mm(w));
    }
    // Traces and vias.
    // KiCad tracks are round-ended; ours end flat. Pull each trace's two ends back
    // by half the width so the round cap reaches where the flat end stopped.
    for t in &board.project.traces {
        let pts = pull_back(&t.points, t.width);
        for seg in pts.windows(2) {
            let _ = writeln!(s, "  (segment (start {}) (end {}) (width {}) (layer {}){} (uuid {}))", xy(seg[0]), xy(seg[1]), mm(t.width), q(&cu_layer(&t.layer)), net_num(&t.net), q(&ids.next()));
        }
    }
    for v in &board.project.vias {
        let _ = writeln!(s, "  (via (at {}) (size {}) (drill {}) (layers \"F.Cu\" \"B.Cu\"){} (uuid {}))", xy(v.at), mm(v.diameter), mm(v.drill), net_num(&v.net), q(&ids.next()));
    }
    // Pours as zones; KiCad fills them itself (`kicad-cli pcb drc --refill-zones`).
    for pr in board.pours()? {
        let p = &pr.pour;
        let clearance = p.clearance.unwrap_or(rules.pour_clearance());
        let gap = p.clearance.unwrap_or(rules.thermal_gap());
        let connect = if p.thermal == Some(false) { format!("(connect_pads yes (clearance {}))", mm(clearance)) } else { format!("(connect_pads (clearance {}))", mm(clearance)) };
        let net_name = p.net.clone().unwrap_or_default();
        let nid = net_id(&p.net).unwrap_or(0);
        let pts: Vec<String> = pr.outline.iter().map(|q| format!("(xy {})", xy(*q))).collect();
        let _ = writeln!(
            s,
            "  (zone (net {nid}) (net_name {}) (layer {}) (uuid {}) (name {}) (hatch edge 0.5) (priority {}) {connect} (min_thickness {}) (filled_areas_thickness no) (fill yes (thermal_gap {}) (thermal_bridge_width {}) (island_removal_mode 0) (island_area_min 10)) (polygon (pts {})))",
            q(&net_name),
            q(&cu_layer(&p.layer)),
            q(&ids.next()),
            q(&p.name),
            p.priority,
            mm(rules.pour_min_width()),
            mm(gap),
            mm(rules.thermal_spoke_width),
            pts.join(" ")
        );
    }
    let _ = writeln!(s, ")");
    std::fs::write(&pcb_path, s).map_err(|e| Error::io(format!("could not write `{}`", pcb_path.display()), e))?;

    // Project file: design rules and net classes so KiCad's DRC has the same limits.
    let mut classes = vec![serde_json::json!({
        "name": "Default", "clearance": rules.clearance.mm(), "track_width": rules.trace_width.mm(),
        "via_diameter": rules.via_diameter.mm(), "via_drill": rules.via_drill.mm(),
        "microvia_diameter": 0.3, "microvia_drill": 0.1, "diff_pair_width": rules.trace_width.mm(), "diff_pair_gap": rules.clearance.mm(),
        "wire_width": 6, "bus_width": 12, "pcb_color": "rgba(0, 0, 0, 0.000)", "schematic_color": "rgba(0, 0, 0, 0.000)", "line_style": 0
    })];
    let mut assignments: Vec<serde_json::Value> = Vec::new();
    for (cname, c) in &rules.net_classes {
        classes.push(serde_json::json!({
            "name": cname, "clearance": c.clearance.unwrap_or(rules.clearance).mm(), "track_width": c.trace_width.unwrap_or(rules.trace_width).mm(),
            "via_diameter": c.via_diameter.unwrap_or(rules.via_diameter).mm(), "via_drill": c.via_drill.unwrap_or(rules.via_drill).mm(),
            "microvia_diameter": 0.3, "microvia_drill": 0.1, "diff_pair_width": rules.trace_width.mm(), "diff_pair_gap": rules.clearance.mm(),
            "wire_width": 6, "bus_width": 12, "pcb_color": "rgba(0, 0, 0, 0.000)", "schematic_color": "rgba(0, 0, 0, 0.000)", "line_style": 0
        }));
        for n in board.nets.iter().filter(|n| n.class.as_deref() == Some(cname.as_str())) {
            assignments.push(serde_json::json!({ "pattern": n.name, "netclass": cname }));
        }
    }
    let pro = serde_json::json!({
        "meta": { "filename": pro_path.file_name().unwrap().to_string_lossy(), "version": 1 },
        "board": {
            "design_settings": {
                "rules": {
                    "min_clearance": rules.clearance.mm(),
                    "min_track_width": rules.trace_width.mm().min(0.1),
                    "min_via_diameter": rules.via_diameter.mm(),
                    "min_via_annular_width": ((rules.via_diameter.mm() - rules.via_drill.mm()) / 2.0).min(0.13),
                    "min_through_hole_diameter": 0.15,
                    "min_hole_clearance": rules.hole_clearance().mm(),
                    "min_hole_to_hole": 0.25,
                    "min_copper_edge_clearance": rules.edge_clearance.mm(),
                    "min_silk_clearance": 0.0,
                    "min_text_height": 0.5,
                    "min_text_thickness": 0.08,
                    "min_connection": 0.0,
                    "min_microvia_diameter": 0.2,
                    "min_microvia_drill": 0.1,
                    "min_resolved_spokes": 2,
                    "solder_mask_clearance": rules.mask_expansion.mm(),
                    "solder_mask_min_width": 0.0,
                    "solder_paste_clearance": -rules.paste_shrink.mm(),
                    "solder_paste_margin_ratio": 0.0
                },
                "rule_severities": {
                    "silk_over_copper": "warning", "silk_overlap": "warning", "silk_edge_clearance": "ignore",
                    "lib_footprint_issues": "ignore", "lib_footprint_mismatch": "ignore", "footprint_type_mismatch": "ignore",
                    "unresolved_variable": "ignore", "isolated_copper": "warning", "starved_thermal": "warning"
                }
            }
        },
        "net_settings": { "classes": classes, "meta": { "version": 4 }, "netclass_patterns": assignments },
        "pcbnew": { "last_paths": {}, "page_layout_descr_file": "" },
        "schematic": { "legacy_lib_dir": "", "legacy_lib_list": [] }
    });
    std::fs::write(&pro_path, serde_json::to_string_pretty(&pro).unwrap()).map_err(|e| Error::io(format!("could not write `{}`", pro_path.display()), e))?;
    Ok(Export { pcb: pcb_path, pro: pro_path })
}

fn pull_back(points: &[Point], width: Length) -> Vec<Point> {
    let mut pts = points.to_vec();
    let r = width.nm() as f64 / 2.0;
    let shorten = |a: Point, b: Point| -> Point {
        // Move `a` toward `b` by min(r, half the segment).
        let (dx, dy) = ((b.x.nm() - a.x.nm()) as f64, (b.y.nm() - a.y.nm()) as f64);
        let len = dx.hypot(dy);
        if len <= 0.0 {
            return a;
        }
        let d = r.min(len / 2.0);
        Point::nm(a.x.nm() + (dx / len * d).round() as i64, a.y.nm() + (dy / len * d).round() as i64)
    };
    if pts.len() >= 2 {
        pts[0] = shorten(pts[0], pts[1]);
        let n = pts.len();
        pts[n - 1] = shorten(pts[n - 1], pts[n - 2]);
    }
    pts
}

fn fmt_angle(deg: f64) -> String {
    let d = deg.rem_euclid(360.0);
    let s = format!("{d:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

fn arc_mid(from: Point, to: Point, center: Point, clockwise: bool) -> Point {
    let (cx, cy) = (center.x.nm() as f64, center.y.nm() as f64);
    let r = ((from.x.nm() as f64 - cx).hypot(from.y.nm() as f64 - cy) + (to.x.nm() as f64 - cx).hypot(to.y.nm() as f64 - cy)) / 2.0;
    let a0 = (from.y.nm() as f64 - cy).atan2(from.x.nm() as f64 - cx);
    let a1 = (to.y.nm() as f64 - cy).atan2(to.x.nm() as f64 - cx);
    let mut sweep = a1 - a0;
    if clockwise {
        if sweep > 0.0 {
            sweep -= std::f64::consts::TAU;
        }
    } else if sweep < 0.0 {
        sweep += std::f64::consts::TAU;
    }
    let a = a0 + sweep / 2.0;
    Point::nm((cx + r * a.cos()).round() as i64, (cy + r * a.sin()).round() as i64)
}

/// Where `kicad-cli` lives, if anywhere.
pub fn find_kicad_cli() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("KICAD_CLI") {
        return Some(PathBuf::from(p));
    }
    for c in ["/Applications/KiCad/KiCad.app/Contents/MacOS/kicad-cli", "/usr/bin/kicad-cli", "/usr/local/bin/kicad-cli", "/opt/homebrew/bin/kicad-cli"] {
        if Path::new(c).exists() {
            return Some(PathBuf::from(c));
        }
    }
    if let Ok(path) = std::env::var("PATH") {
        for dir in path.split(':') {
            let p = Path::new(dir).join("kicad-cli");
            if p.exists() {
                return Some(p);
            }
        }
    }
    None
}

#[derive(Debug, serde::Deserialize)]
pub struct DrcReport {
    #[serde(default)]
    pub violations: Vec<Violation>,
    #[serde(default)]
    pub unconnected_items: Vec<Violation>,
    #[serde(default)]
    pub schematic_parity: Vec<Violation>,
}
#[derive(Debug, serde::Deserialize)]
pub struct Violation {
    #[serde(rename = "type")]
    pub kind: String,
    pub description: String,
    pub severity: String,
    #[serde(default)]
    pub items: Vec<serde_json::Value>,
}

/// Run KiCad's DRC on an exported board; returns the parsed JSON report.
pub fn run_drc(pcb: &Path) -> Result<DrcReport> {
    let cli = find_kicad_cli().ok_or_else(|| Error::with_help("kicad-cli not found", "install KiCad, or set KICAD_CLI to the executable"))?;
    let report = pcb.with_extension("drc.json");
    let out = std::process::Command::new(&cli)
        .args(["pcb", "drc", "--refill-zones", "--format", "json", "--severity-all", "--units", "mm", "-o"])
        .arg(&report)
        .arg(pcb)
        .output()
        .map_err(|e| Error::io(format!("could not run `{}`", cli.display()), e))?;
    if !report.exists() {
        return Err(Error::with_help(
            format!("kicad-cli did not write a report (exit {:?})", out.status.code()),
            format!("stdout:\n{}\nstderr:\n{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)),
        ));
    }
    let text = std::fs::read_to_string(&report).map_err(|e| Error::io("could not read the DRC report", e))?;
    serde_json::from_str(&text).map_err(|e| Error::msg(format!("could not parse the DRC report: {e}")))
}
