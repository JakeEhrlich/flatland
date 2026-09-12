//! Design-rule checking: turn a board into a list of features, run every
//! rule of the active rule sets against them, apply waivers.

use crate::error::Result;
use crate::geom::{self, Ring, Rings};
use crate::model::Board;
use crate::schema::PadType;
use crate::schema::*;
use crate::store;
use crate::units::{Length, Point};
use indexmap::IndexMap;
use std::path::Path;

pub const BUILTIN_BASIC: &str = include_str!("../drc/basic.json");

/// One checkable thing on the board.
#[derive(Clone, Debug)]
pub struct Item {
    pub kind: Feature,
    /// Human-readable, stable enough to waive: `R1.2`, `trace VIN at 4,6.5`.
    pub id: String,
    pub refdes: Option<String>,
    pub net: Option<String>,
    /// Copper layers occupied, or `top`/`bottom` for side features; empty = all.
    pub layers: Vec<String>,
    pub geom: Rings,
    /// Scalar attributes in mm (or counts): width, drill, diameter, annular_ring, ...
    pub attrs: IndexMap<&'static str, f64>,
    pub plated: Option<bool>,
    pub pad_kind: Option<&'static str>,
    pub footprint: Option<String>,
    pub component: Option<String>,
    pub class: Option<String>,
    pub center: Point,
    pub extra: Vec<Point>,
    /// Big polygons (pours) cut into grid tiles so a small feature is only
    /// tested against the pieces near it: (bbox, rings).
    pub tiles: Option<Vec<((Point, Point), Rings)>>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct Finding {
    pub rule: String,
    pub severity: Severity,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at: Option<Point>,
    /// Ids a waiver can name: the feature id(s), reference designator(s), net(s).
    pub features: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub waived: Option<String>,
}

pub struct Report {
    pub findings: Vec<Finding>,
    pub rules_run: usize,
    pub sets: Vec<String>,
    /// (stage or rule name, seconds) for `pcb check --timing`.
    pub timing: Vec<(String, f64)>,
}

impl Report {
    pub fn count(&self, s: Severity) -> usize {
        self.findings.iter().filter(|f| f.severity == s && f.waived.is_none()).count()
    }
    pub fn waived(&self) -> usize {
        self.findings.iter().filter(|f| f.waived.is_some()).count()
    }
}

/// Load every active rule set: the built-in basic set, the project's
/// referenced sets (hash-verified), and the project's own rules.
pub fn active_rules(project: &Project, project_path: &Path) -> Result<(Vec<(String, RuleSet)>, Vec<Rule>)> {
    let mut sets = vec![("basic".to_string(), serde_json::from_str::<RuleSet>(BUILTIN_BASIC).expect("built-in rule set parses"))];
    for r in &project.drc.rulesets {
        let path = store::local_path(&r.url, project_path, "rule set")?;
        store::verify_hash(&path, r.blake3.as_deref(), "rule set")?;
        let set: RuleSet = store::read_json(&path, "rule set")?;
        if set.schema != DRC_SCHEMA {
            return Err(crate::error::Error::with_help(
                format!("{} is `{}`, not a `{DRC_SCHEMA}` rule set", path.display(), set.schema),
                "see `pcb schema drc`",
            ));
        }
        sets.push((set.name.clone(), set));
    }
    Ok((sets, project.drc.rules.clone()))
}

pub fn run(board: &Board, project_path: &Path) -> Result<Report> {
    let (sets, own) = active_rules(&board.project, project_path)?;
    let mut timing: Vec<(String, f64)> = Vec::new();
    let t0 = std::time::Instant::now();
    let items = collect(board)?;
    timing.push(("collect features".into(), t0.elapsed().as_secs_f64()));
    let mut findings = Vec::new();
    let layer_count = board.layers().len();
    for (name, set) in &sets {
        if let Some(a) = &set.applies {
            if !a.layers.is_empty() && !a.layers.contains(&layer_count) {
                findings.push(Finding {
                    rule: format!("{name}:applies"),
                    severity: Severity::Warning,
                    message: format!(
                        "rule set `{name}` is for {}-layer boards, this board has {layer_count}",
                        a.layers.iter().map(|l| l.to_string()).collect::<Vec<_>>().join("/")
                    ),
                    at: None,
                    features: vec![],
                    waived: None,
                });
            }
        }
    }
    // Rules are independent: run them across all cores.
    let jobs: Vec<(String, &Rule)> = sets
        .iter()
        .flat_map(|(name, set)| set.rules.iter().map(move |r| (format!("{name}:{}", r.name), r)))
        .chain(own.iter().map(|r| (format!("project:{}", r.name), r)))
        .collect();
    let rules_run = jobs.len();
    use rayon::prelude::*;
    let results: Vec<Result<(Vec<Finding>, String, f64)>> = jobs
        .par_iter()
        .map(|(label, rule)| {
            let t = std::time::Instant::now();
            let mut f = Vec::new();
            eval(board, &items, rule, &mut f)?;
            Ok((f, label.clone(), t.elapsed().as_secs_f64()))
        })
        .collect();
    for r in results {
        let (f, label, secs) = r?;
        findings.extend(f);
        timing.push((label, secs));
    }
    // Waivers.
    for f in &mut findings {
        for w in &board.project.drc.waivers {
            if w.rule != f.rule {
                continue;
            }
            if w.features.is_empty() || w.features.iter().any(|x| f.features.iter().any(|y| y == x)) {
                f.waived = Some(w.reason.clone());
                break;
            }
        }
    }
    findings.sort_by(|a, b| b.severity.cmp(&a.severity).then_with(|| a.rule.cmp(&b.rule)));
    Ok(Report { findings, rules_run, sets: sets.iter().map(|(n, _)| n.clone()).collect(), timing })
}

// ---------------------------------------------------------------------------
// Feature extraction

fn side_name(s: Side) -> &'static str {
    match s {
        Side::Top => "top",
        Side::Bottom => "bottom",
    }
}

fn centre(rings: &[Ring]) -> Point {
    geom::bounds(rings).map(|(lo, hi)| Point::nm((lo.x.nm() + hi.x.nm()) / 2, (lo.y.nm() + hi.y.nm()) / 2)).unwrap_or(Point::ORIGIN)
}

fn item(kind: Feature, id: String, geom: Rings) -> Item {
    let center = centre(&geom);
    Item {
        kind,
        id,
        refdes: None,
        net: None,
        layers: vec![],
        geom,
        attrs: IndexMap::new(),
        plated: None,
        pad_kind: None,
        footprint: None,
        component: None,
        class: None,
        center,
        extra: vec![],
        tiles: None,
    }
}

const TILE_MM: f64 = 6.0;
/// Offsets simplify outlines by up to 2 µm; morphological differences ignore
/// anything thinner than this.
const GEOM_TOLERANCE: Length = Length(5_000);
const TILE_MIN_VERTICES: usize = 400;

/// Cut a big polygon set into grid tiles (done once per feature).
fn tile(rings: &Rings) -> Option<Vec<((Point, Point), Rings)>> {
    let n: usize = rings.iter().map(|r| r.len()).sum();
    if n < TILE_MIN_VERTICES {
        return None;
    }
    let (lo, hi) = geom::bounds(rings)?;
    let step = Length::from_mm(TILE_MM);
    let mut rects: Vec<Ring> = Vec::new();
    let mut y = lo.y;
    while y < hi.y {
        let mut x = lo.x;
        let y1 = (y + step).min(hi.y);
        while x < hi.x {
            let x1 = (x + step).min(hi.x);
            rects.push(vec![Point::nm(x.nm(), y.nm()), Point::nm(x1.nm(), y.nm()), Point::nm(x1.nm(), y1.nm()), Point::nm(x.nm(), y1.nm())]);
            x = x1;
        }
        y = y1;
    }
    // Only the rings whose bounding box touches a tile can shape it: the outer
    // ring(s) and the few holes inside. Passing all of a pour's cut-outs to every
    // tile made tiling cost more than the checks it was speeding up.
    let boxes: Vec<(Point, Point)> = rings.iter().map(|r| geom::bounds(&[r.clone()]).unwrap_or((Point::ORIGIN, Point::ORIGIN))).collect();
    use rayon::prelude::*;
    let tiles: Vec<((Point, Point), Rings)> = rects
        .par_iter()
        .filter_map(|rect| {
            let (rlo, rhi) = (rect[0], rect[2]);
            let subset: Rings = rings
                .iter()
                .zip(&boxes)
                .filter(|(_, (blo, bhi))| !(bhi.x < rlo.x || rhi.x < blo.x || bhi.y < rlo.y || rhi.y < blo.y))
                .map(|(r, _)| r.clone())
                .collect();
            if subset.is_empty() {
                return None;
            }
            let piece = geom::intersection(&subset, &[rect.clone()]).ok()?;
            if piece.is_empty() {
                return None;
            }
            let bb = geom::bounds(&piece)?;
            Some((bb, piece))
        })
        .collect();
    Some(tiles)
}

/// Does `rings` overlap the item's geometry? Uses the item's tiles when it has them.
fn overlaps_item(rings: &Rings, it: &Item) -> bool {
    match &it.tiles {
        Some(tiles) => {
            let Some((lo, hi)) = geom::bounds(rings) else { return false };
            tiles.iter().any(|((tlo, thi), piece)| {
                !(hi.x < tlo.x || thi.x < lo.x || hi.y < tlo.y || thi.y < lo.y) && geom::overlaps(rings, piece)
            })
        }
        None => geom::overlaps(rings, &it.geom),
    }
}

pub fn collect(board: &Board) -> Result<Vec<Item>> {
    let project = &board.project;
    let rules = board.rules();
    let class_of = |net: &Option<String>| net.as_ref().and_then(|n| project.nets.get(n)).and_then(|n| n.class.clone());
    let all_layers: Vec<String> = board.layers().to_vec();
    let mut items: Vec<Item> = Vec::new();

    // Traces.
    for t in &project.traces {
        let mut it = item(Feature::Trace, format!("trace {} at {}", t.net.as_deref().unwrap_or("-"), t.points[0]), geom::stroke_flat(&t.points, t.width));
        it.net = t.net.clone();
        it.layers = vec![t.layer.clone()];
        it.attrs.insert("width", t.width.mm());
        it.class = class_of(&t.net);
        it.extra = vec![t.points[0], *t.points.last().unwrap()];
        items.push(it);
    }
    // Vias: copper disc, plus the hole.
    for (vi, v) in project.vias.iter().enumerate() {
        let layers = if v.layers.is_empty() { all_layers.clone() } else { v.layers.clone() };
        let mut it = item(Feature::Via, format!("via #{vi} {} at {}", v.net.as_deref().unwrap_or("-"), v.at), vec![geom::circle(v.at, v.diameter)]);
        it.net = v.net.clone();
        it.layers = layers.clone();
        it.attrs.insert("drill", v.drill.mm());
        it.attrs.insert("diameter", v.diameter.mm());
        it.attrs.insert("annular_ring", (v.diameter.mm() - v.drill.mm()) / 2.0);
        it.class = class_of(&v.net);
        it.plated = Some(true);
        items.push(it);
        let mut h = item(Feature::Hole, format!("hole of via #{vi} at {}", v.at), vec![geom::circle(v.at, v.drill)]);
        h.net = v.net.clone();
        h.layers = layers;
        h.attrs.insert("drill", v.drill.mm());
        h.plated = Some(true);
        items.push(h);
    }
    // Pads (and the holes of through-hole pads).
    for pad in board.all_pads() {
        let inst = board.instances.iter().find(|i| i.refdes == pad.refdes);
        let mut it = item(Feature::Pad, format!("{}.{}", pad.refdes, pad.pad_name), vec![pad.copper.clone()]);
        it.refdes = Some(pad.refdes.clone());
        it.net = pad.net.clone();
        it.layers = pad.layers.clone();
        it.attrs.insert("width", pad.size[0].mm());
        it.attrs.insert("height", pad.size[1].mm());
        if let Some(d) = pad.drill {
            it.attrs.insert("drill", d.mm());
            if pad.plated {
                it.attrs.insert("annular_ring", (pad.size[0].mm().min(pad.size[1].mm()) - d.mm()) / 2.0);
            }
        }
        it.plated = Some(pad.plated);
        it.pad_kind = Some(match pad.pad_type {
            PadType::Smd => "smd",
            PadType::ThroughHole => "through_hole",
        });
        it.footprint = inst.and_then(|i| i.component.footprint.as_ref().map(|f| f.footprint.name.clone()));
        it.component = inst.map(|i| i.component.name.clone());
        it.class = class_of(&pad.net);
        it.center = pad.center;
        items.push(it);
        if let Some(d) = pad.drill {
            let mut h = item(Feature::Hole, format!("hole of {}.{}", pad.refdes, pad.pad_name), vec![geom::circle(pad.center, d)]);
            h.refdes = Some(pad.refdes.clone());
            h.net = pad.net.clone();
            h.layers = all_layers.clone();
            h.attrs.insert("drill", d.mm());
            h.plated = Some(pad.plated);
            h.center = pad.center;
            items.push(h);
        }
    }
    // Free holes (and the copper ring of plated ones).
    for (hi, h) in board.holes.iter().enumerate() {
        let mut it = item(Feature::Hole, format!("hole #{hi} at {}", h.hole.at), vec![h.ring.clone()]);
        it.net = h.hole.net.clone();
        it.layers = all_layers.clone();
        it.attrs.insert("drill", h.hole.drill.mm());
        it.plated = Some(h.hole.plated);
        it.center = h.hole.at;
        items.push(it);
        if let Some(c) = &h.copper {
            let mut r = item(Feature::Pad, format!("ring of hole at {}", h.hole.at), vec![c.clone()]);
            r.net = h.hole.net.clone();
            r.layers = all_layers.clone();
            let d = h.hole.diameter.unwrap_or(h.hole.drill + rules.hole_annular_ring * 2);
            r.attrs.insert("drill", h.hole.drill.mm());
            r.attrs.insert("annular_ring", (d.mm() - h.hole.drill.mm()) / 2.0);
            r.plated = Some(true);
            r.pad_kind = Some("through_hole");
            r.center = h.hole.at;
            items.push(r);
        }
    }
    // Pours and their pieces.
    for p in board.pours()? {
        let mut it = item(Feature::Pour, format!("pour {}", p.pour.name), p.copper.clone());
        it.tiles = tile(&it.geom);
        it.net = p.pour.net.clone();
        it.layers = vec![p.pour.layer.clone()];
        it.attrs.insert("area", p.copper.iter().map(|r| geom::signed_area(r)).sum::<f64>() / 1e12);
        it.class = class_of(&p.pour.net);
        items.push(it);
        let holes: Vec<&Ring> = p.copper.iter().filter(|r| geom::signed_area(r) <= 0.0).collect();
        for (i, o) in p.copper.iter().filter(|r| geom::signed_area(r) > 0.0).enumerate() {
            let mut rings = vec![o.clone()];
            rings.extend(holes.iter().filter(|h| h.first().map_or(false, |q| geom::contains(o, *q))).map(|h| (*h).clone()));
            let mut piece = item(Feature::PourPiece, format!("pour {} piece {}", p.pour.name, i + 1), rings);
            piece.net = p.pour.net.clone();
            piece.layers = vec![p.pour.layer.clone()];
            piece.attrs.insert("area", piece.geom.iter().map(|r| geom::signed_area(r)).sum::<f64>() / 1e12);
            items.push(piece);
        }
    }
    // Mask openings, silk, courtyards per side.
    for side in [Side::Top, Side::Bottom] {
        let layer = board.stackup().layer_for_side(side).to_string();
        for pad in board.all_pads() {
            if !(pad.pad_type == PadType::ThroughHole || pad.on_layer(&layer)) {
                continue;
            }
            let mut it = item(Feature::MaskOpening, format!("mask of {}.{} ({})", pad.refdes, pad.pad_name, side_name(side)), pad.mask_ring());
            it.refdes = Some(pad.refdes.clone());
            it.net = pad.net.clone();
            it.layers = vec![side_name(side).to_string()];
            it.center = pad.center;
            items.push(it);
        }
        for v in &project.vias {
            let mut it = item(Feature::MaskOpening, format!("mask of via at {} ({})", v.at, side_name(side)), vec![geom::circle(v.at, v.diameter + rules.mask_expansion * 2)]);
            it.net = v.net.clone();
            it.layers = vec![side_name(side).to_string()];
            items.push(it);
        }
        for h in &board.holes {
            if let Some(c) = &h.copper {
                let mut it = item(Feature::MaskOpening, format!("mask of hole at {} ({})", h.hole.at, side_name(side)), geom::offset(&[c.clone()], rules.mask_expansion));
                it.net = h.hole.net.clone();
                it.layers = vec![side_name(side).to_string()];
                items.push(it);
            }
        }
        for inst in board.instances.iter().filter(|i| i.side() == Some(side)) {
            for (k, (pts, closed, w)) in inst.silk_lines.iter().enumerate() {
                let mut pts = pts.clone();
                if *closed {
                    if let Some(f) = pts.first().copied() {
                        pts.push(f);
                    }
                }
                let mut it = item(Feature::Silk, format!("silk {} of {}", k + 1, inst.refdes), geom::stroke(&pts, *w));
                it.refdes = Some(inst.refdes.clone());
                it.layers = vec![side_name(side).to_string()];
                it.attrs.insert("width", w.mm());
                items.push(it);
            }
            if let Some(at) = inst.label_at {
                let size = inst.label_size(rules);
                let mut strokes: Rings = Vec::new();
                for s in crate::gerber::font::render(&inst.refdes, at, size, side == Side::Bottom) {
                    strokes.extend(geom::stroke(&s, rules.silk_width));
                }
                let mut it = item(Feature::SilkText, format!("label of {}", inst.refdes), strokes);
                it.refdes = Some(inst.refdes.clone());
                it.layers = vec![side_name(side).to_string()];
                it.attrs.insert("height", size.mm());
                it.attrs.insert("width", rules.silk_width.mm());
                it.center = at;
                items.push(it);
            }
            let mut rings: Rings = Vec::new();
            for (pts, _) in &inst.courtyard {
                if pts.len() >= 3 {
                    rings.push(pts.clone());
                }
            }
            if !rings.is_empty() {
                let mut it = item(Feature::Courtyard, format!("courtyard of {}", inst.refdes), rings);
                it.refdes = Some(inst.refdes.clone());
                it.layers = vec![side_name(side).to_string()];
                it.footprint = inst.component.footprint.as_ref().map(|f| f.footprint.name.clone());
                it.component = Some(inst.component.name.clone());
                items.push(it);
            }
        }
    }
    // Free text: copper text counts as copper, silk text as silk_text.
    for t in &project.texts {
        let on_copper = all_layers.contains(&t.layer);
        let (layers, geom_rings, width) = if on_copper {
            (vec![t.layer.clone()], board.text_copper(t), t.width.unwrap_or(rules.trace_width))
        } else {
            let side = t.silk_side().unwrap_or(Side::Top);
            let w = t.width.unwrap_or(rules.silk_width);
            let mut rings: Rings = Vec::new();
            for st in crate::gerber::font::render_rotated(&t.text, t.at, t.size, t.rotation, side == Side::Bottom) {
                rings.extend(geom::stroke(&st, w));
            }
            (vec![side_name(side).to_string()], rings, w)
        };
        let mut it = item(Feature::Text, format!("text \"{}\" at {}", t.text, t.at), geom_rings);
        it.layers = layers;
        it.attrs.insert("height", t.size.mm());
        it.attrs.insert("width", width.mm());
        it.center = t.at;
        it.extra = if on_copper { vec![t.at] } else { vec![] }; // marker: copper text
        items.push(it);
    }
    // Outline and board.
    if let Some(o) = &board.outline {
        items.push(item(Feature::Outline, "board outline".into(), vec![o.clone()]));
        let mut b = item(Feature::Board, "board".into(), vec![]);
        if let Some((lo, hi)) = geom::bounds(&[o.clone()]) {
            b.attrs.insert("width", (hi.x - lo.x).mm());
            b.attrs.insert("height", (hi.y - lo.y).mm());
        }
        b.attrs.insert("layers", all_layers.len() as f64);
        items.push(b);
    } else {
        let mut b = item(Feature::Board, "board".into(), vec![]);
        b.attrs.insert("layers", all_layers.len() as f64);
        items.push(b);
    }
    // Nets, pins, parts.
    for n in &board.nets {
        let mut it = item(Feature::Net, format!("net {}", n.name), vec![]);
        it.net = Some(n.name.clone());
        it.class = n.class.clone();
        it.attrs.insert("pins", n.pins.len() as f64);
        items.push(it);
    }
    for i in &board.instances {
        for pin in &i.component.component.pins {
            let mut it = item(Feature::Pin, format!("{}.{}", i.refdes, pin.name), vec![]);
            it.refdes = Some(i.refdes.clone());
            it.net = board.pin_net(&i.refdes, &pin.name).map(|s| s.to_string());
            it.component = Some(i.component.name.clone());
            items.push(it);
        }
        let mut it = item(Feature::Part, i.refdes.clone(), vec![]);
        it.refdes = Some(i.refdes.clone());
        it.component = Some(i.component.name.clone());
        it.footprint = i.component.footprint.as_ref().map(|f| f.footprint.name.clone());
        it.attrs.insert("placed", if i.is_placed() { 1.0 } else { 0.0 });
        it.attrs.insert("virtual", if i.component.is_virtual() { 1.0 } else { 0.0 });
        items.push(it);
    }
    Ok(items)
}

// ---------------------------------------------------------------------------
// Evaluation

fn is_copper(k: Feature) -> bool {
    matches!(k, Feature::Trace | Feature::Via | Feature::Pad | Feature::Pour)
}

fn is_copper_item(it: &Item) -> bool {
    is_copper(it.kind) || (it.kind == Feature::Text && !it.extra.is_empty())
}

fn selects(f: Feature, it: &Item) -> bool {
    match f {
        Feature::Copper => is_copper_item(it),
        Feature::SilkText => it.kind == Feature::SilkText || (it.kind == Feature::Text && it.extra.is_empty()),
        other => it.kind == other,
    }
}

fn matches_where(w: &Option<Where>, it: &Item) -> bool {
    let Some(w) = w else { return true };
    if let Some(l) = &w.layer {
        if !it.layers.is_empty() && !it.layers.iter().any(|x| x == l) {
            return false;
        }
    }
    if let Some(n) = &w.net {
        if it.net.as_deref() != Some(n.as_str()) {
            return false;
        }
    }
    if let Some(c) = &w.class {
        if it.class.as_deref() != Some(c.as_str()) {
            return false;
        }
    }
    if let Some(p) = w.plated {
        if it.plated != Some(p) {
            return false;
        }
    }
    if let Some(k) = &w.kind {
        if it.pad_kind != Some(k.as_str()) {
            return false;
        }
    }
    if let Some(r) = &w.refdes {
        if it.refdes.as_deref() != Some(r.as_str()) {
            return false;
        }
    }
    if let Some(f) = &w.footprint {
        if it.footprint.as_deref() != Some(f.as_str()) {
            return false;
        }
    }
    if let Some(c) = &w.component {
        if it.component.as_deref() != Some(c.as_str()) {
            return false;
        }
    }
    true
}

fn share_layer(a: &Item, b: &Item) -> bool {
    a.layers.is_empty() || b.layers.is_empty() || a.layers.iter().any(|l| b.layers.contains(l))
}

fn related(rel: Relation, a: &Item, b: &Item) -> bool {
    match rel {
        Relation::DifferentNet => !(a.net.is_some() && a.net == b.net),
        Relation::Any => true,
        Relation::DifferentPart => !(a.refdes.is_some() && a.refdes == b.refdes),
        Relation::SameNet => a.net.is_some() && a.net == b.net,
    }
}

fn bbox_near(a: &Rings, b: &Rings, d: Length) -> bool {
    let (Some((alo, ahi)), Some((blo, bhi))) = (geom::bounds(a), geom::bounds(b)) else { return false };
    !(ahi.x + d < blo.x || bhi.x + d < alo.x || ahi.y + d < blo.y || bhi.y + d < alo.y)
}

fn resolve(limit: &Limit, rules: &DesignRules, ctx: &str) -> Length {
    match limit {
        Limit::Mm(v) => Length::from_mm(*v),
        Limit::Named(_) => match ctx {
            "edge" => rules.edge_clearance,
            "width" => rules.trace_width,
            _ => rules.clearance,
        },
    }
}

fn features_of(it: &Item) -> Vec<String> {
    let mut v = vec![it.id.clone()];
    if let Some(r) = &it.refdes {
        v.push(r.clone());
    }
    if let Some(n) = &it.net {
        v.push(n.clone());
    }
    v
}

fn push(findings: &mut Vec<Finding>, rule: &Rule, message: String, at: Option<Point>, features: Vec<String>) {
    findings.push(Finding { rule: rule.name.clone(), severity: rule.severity, message, at, features, waived: None });
}

/// Pieces of `diff` big enough to be a real sliver/gap rather than an arc
/// approximation artefact. A piece is an outer ring minus the holes inside
/// it (a closing that merely re-rounds a pad's corners yields a hair-thin
/// annulus: big outer, big hole, negligible net area). Real slivers and gaps
/// are longer than twice the limit and at least `limit²` in area.
fn significant(diff: Rings, limit: Length) -> Vec<(Point, f64)> {
    let mut out = Vec::new();
    let lim = limit.mm();
    let holes: Vec<&Ring> = diff.iter().filter(|r| geom::signed_area(r) <= 0.0).collect();
    for r in diff.iter().filter(|r| geom::signed_area(r) > 0.0) {
        let Some((lo, hi)) = geom::bounds(&[r.clone()]) else { continue };
        let long = (hi.x - lo.x).max(hi.y - lo.y);
        let holes_in: f64 = holes.iter().filter(|h| h.first().map_or(false, |q| geom::contains(r, *q))).map(|h| -geom::signed_area(h)).sum();
        let area = (geom::signed_area(r) - holes_in) / 1e12;
        if long.nm() >= limit.nm() * 2 && area >= lim * lim {
            out.push((centre(&[r.clone()]), area));
        }
    }
    out
}

fn eval(board: &Board, items: &[Item], rule: &Rule, findings: &mut Vec<Finding>) -> Result<()> {
    let rules = board.rules();
    let subjects: Vec<&Item> = items.iter().filter(|it| selects(rule.for_, it) && matches_where(&rule.where_, it)).collect();
    match &rule.check {
        Check::Clearance { to, relation, min } => {
            if *to == Feature::Outline {
                let Some(o) = &board.outline else { return Ok(()) };
                let d = resolve(min, rules, "edge");
                for s in &subjects {
                    if s.geom.is_empty() {
                        continue;
                    }
                    // A pour is clipped to the outline minus `edge_clearance` when it is
                    // filled; only a tighter rule needs the (expensive) geometry test.
                    if s.kind == Feature::Pour && rules.edge_clearance.nm() >= d.nm() {
                        continue;
                    }
                    let grown = geom::offset(&s.geom, d - Length::from_nm(2_000));
                    let outside = geom::difference(&grown, &[o.clone()])?;
                    if outside.iter().any(|r| geom::signed_area(r) > 1e6) {
                        push(findings, rule, format!("{} is closer than {d} to the board edge", s.id), Some(s.center), features_of(s));
                    }
                }
                return Ok(());
            }
            let d = resolve(min, rules, "clearance");
            let targets: Vec<&Item> = items.iter().filter(|it| selects(*to, it) && !it.geom.is_empty()).collect();
            let is_subject = |it: &Item| selects(rule.for_, it) && matches_where(&rule.where_, it);
            // Each unordered pair is tested once, by exactly one side: the untiled
            // side when only one is a pour (it grows itself and probes the pour's
            // tiles), otherwise the side with the smaller id. A target that is not
            // itself a subject is always tested from the subject.
            let owns = |s: &Item, t: &Item| -> bool {
                if !is_subject(t) {
                    return true;
                }
                match (s.tiles.is_some(), t.tiles.is_some()) {
                    (true, false) => false,
                    (false, true) => true,
                    _ => s.id < t.id,
                }
            };
            use rayon::prelude::*;
            let found: Vec<Finding> = subjects
                .par_iter()
                .flat_map_iter(|s| {
                    let mut out = Vec::new();
                    if s.geom.is_empty() {
                        return out;
                    }
                    // Growing a pour is expensive: do it only if some pair needs it.
                    let mut grown: Option<Rings> = None;
                    for t in &targets {
                        if std::ptr::eq(*s, *t) || s.id == t.id || !share_layer(s, t) || !related(*relation, s, t) {
                            continue;
                        }
                        // Pads and the holes/mask of the same pad are one thing.
                        if (s.kind == Feature::Hole || t.kind == Feature::Hole) && s.refdes.is_some() && s.refdes == t.refdes && s.center == t.center {
                            continue;
                        }
                        if !bbox_near(&s.geom, &t.geom, d) || !owns(s, t) {
                            continue;
                        }
                        let g = grown.get_or_insert_with(|| geom::offset(&s.geom, d - Length::from_nm(2_000)));
                        if overlaps_item(g, t) {
                            let touching = geom::overlaps(&s.geom, &t.geom);
                            let msg = if d.nm() <= 0 || touching {
                                format!("{} and {} overlap", s.id, t.id)
                            } else {
                                format!("{} and {} are closer than {d}", s.id, t.id)
                            };
                            let mut feats = features_of(s);
                            feats.extend(features_of(t));
                            out.push(Finding { rule: rule.name.clone(), severity: rule.severity, message: msg, at: Some(s.center), features: feats, waived: None });
                        }
                    }
                    out
                })
                .collect();
            findings.extend(found);
        }
        Check::MinWidth(limit) => {
            let w = resolve(limit, rules, "width");
            for s in &subjects {
                if let Some(v) = s.attrs.get("width").copied() {
                    let v = match s.kind {
                        Feature::Pad => v.min(s.attrs.get("height").copied().unwrap_or(v)),
                        _ => v,
                    };
                    if v + 1e-6 < w.mm() {
                        push(findings, rule, format!("{} is {:.3} mm wide, minimum {w}", s.id, v), Some(s.center), features_of(s));
                    }
                    continue;
                }
                if s.geom.is_empty() {
                    continue;
                }
                // Pour fill narrower than `pour_min_width` is removed when the pour is
                // filled, so the opening is only worth running for a tighter rule.
                if matches!(s.kind, Feature::Pour | Feature::PourPiece) && rules.pour_min_width().nm() >= w.nm() {
                    continue;
                }
                let opened = geom::open(&s.geom, w)?;
                // The original shrunk by a few µm: offsets simplify their outlines by
                // up to 2 µm, and without this a hairline along every edge would
                // register as a sliver.
                let core = geom::offset(&s.geom, -GEOM_TOLERANCE);
                for (at, area) in significant(geom::difference(&core, &opened)?, w) {
                    push(findings, rule, format!("{} has a sliver narrower than {w} near {at} ({area:.2} mm²)", s.id), Some(at), features_of(s));
                }
            }
        }
        Check::MinGap(limit) => {
            let g = resolve(limit, rules, "clearance");
            // Group: copper by (layer, net); side features by side; else alone.
            let mut groups: IndexMap<String, (Rings, Vec<String>, Point)> = IndexMap::new();
            for s in &subjects {
                if s.geom.is_empty() {
                    continue;
                }
                // A pour's internal gaps are its own clearance cut-outs (at least
                // `pour_clearance` wide) and it joins same-net copper solidly, so it
                // cannot hold a gap narrower than the rule; leave it out of the group.
                if matches!(s.kind, Feature::Pour | Feature::PourPiece) {
                    continue;
                }
                let key = if is_copper_item(s) {
                    format!("{}|{}", s.layers.join(","), s.net.as_deref().unwrap_or("-"))
                } else if matches!(s.kind, Feature::MaskOpening | Feature::Silk | Feature::SilkText | Feature::Courtyard) {
                    s.layers.join(",")
                } else {
                    s.id.clone()
                };
                let e = groups.entry(key).or_insert_with(|| (Vec::new(), Vec::new(), s.center));
                e.0.extend(s.geom.iter().cloned());
                e.1.extend(features_of(s));
            }
            for (key, (rings, feats, _)) in groups {
                // Only rings with a neighbour within the gap can form one: drop the
                // rest before the (costly) closing, and skip groups with nothing left.
                let boxes: Vec<(Point, Point)> = rings.iter().filter_map(|r| geom::bounds(&[r.clone()])).collect();
                let near = |i: usize| boxes.iter().enumerate().any(|(j, b)| j != i && !(boxes[i].1.x + g < b.0.x || b.1.x + g < boxes[i].0.x || boxes[i].1.y + g < b.0.y || b.1.y + g < boxes[i].0.y));
                let rings: Rings = rings.iter().enumerate().filter(|(i, _)| near(*i)).map(|(_, r)| r.clone()).collect();
                if rings.is_empty() {
                    continue;
                }
                let u = geom::union(&rings)?;
                // Dilate then erode by exactly half; union with the original so the
                // result is a superset of it and the difference is only what got
                // filled in (a slop on the way back would leave a µm-thin annulus
                // around every feature, which adds up on long perimeters).
                let half = Length::from_nm(g.nm() / 2);
                let closed = geom::offset(&geom::offset(&u, half), -half);
                let closed = geom::union(&[closed, u.clone()].concat())?;
                // Subtract the original grown by a few µm (see min_width).
                let diff = geom::difference(&closed, &geom::offset(&u, GEOM_TOLERANCE))?;
                for (at, area) in significant(diff, g) {
                    push(findings, rule, format!("gap narrower than {g} in {} near {at} ({area:.2} mm²)", key.replace('|', " net ")), Some(at), feats.clone());
                }
            }
        }
        Check::Min(limits) | Check::Max(limits) => {
            let is_min = matches!(rule.check, Check::Min(_));
            for s in &subjects {
                for (k, v) in limits {
                    let Some(have) = s.attrs.get(k.as_str()).copied() else { continue };
                    let bad = if is_min { have + 1e-6 < *v } else { have - 1e-6 > *v };
                    if bad {
                        push(
                            findings,
                            rule,
                            format!("{} has {k} {have:.3}, {} {v}", s.id, if is_min { "minimum" } else { "maximum" }),
                            Some(s.center),
                            features_of(s),
                        );
                    }
                }
            }
        }
        Check::Inside(container) => {
            let region: Rings = items.iter().filter(|it| selects(*container, it)).flat_map(|it| it.geom.iter().cloned()).collect();
            if region.is_empty() {
                return Ok(());
            }
            let region = geom::union(&region)?;
            for s in &subjects {
                if s.geom.is_empty() {
                    continue;
                }
                let shrunk = geom::offset(&s.geom, Length::from_nm(-2_000));
                let outside = geom::difference(&shrunk, &region)?;
                if outside.iter().any(|r| geom::signed_area(r) > 1e6) {
                    push(findings, rule, format!("{} is not entirely inside the {container}", s.id), Some(s.center), features_of(s));
                }
            }
        }
        Check::Connected(want) => {
            match rule.for_ {
                Feature::Net => {
                    let conn = board.connectivity()?;
                    for s in &subjects {
                        let Some((_, islands)) = conn.iter().find(|(n, _)| Some(n.as_str()) == s.net.as_deref()) else { continue };
                        let ok = islands.len() <= 1;
                        if ok != *want {
                            let desc: Vec<String> = islands.iter().map(|i| i.join("+")).collect();
                            push(findings, rule, format!("{} is not fully routed: {} islands ({})", s.id, islands.len(), desc.join(" | ")), None, features_of(s));
                        }
                    }
                }
                Feature::Pin => {
                    for s in &subjects {
                        if s.net.is_some() != *want {
                            push(findings, rule, format!("{} is not connected to any net", s.id), None, features_of(s));
                        }
                    }
                }
                Feature::PourPiece => {
                    for s in &subjects {
                        // Reaching a pad, via or trace of its net on this layer is enough
                        // (a via carries the piece to the other layers).
                        let reaches = items.iter().any(|p| {
                            matches!(p.kind, Feature::Pad | Feature::Via | Feature::Trace) && p.net.is_some() && p.net == s.net && share_layer(p, s) && geom::overlaps(&s.geom, &p.geom)
                        });
                        if reaches != *want {
                            push(findings, rule, format!("{} ({:.1} mm²) reaches no copper of its net: floating", s.id, s.attrs.get("area").copied().unwrap_or(0.0)), Some(s.center), features_of(s));
                        }
                    }
                }
                Feature::Trace | Feature::Copper => {
                    for s in &subjects {
                        if s.kind != Feature::Trace {
                            continue;
                        }
                        for end in &s.extra {
                            let touched = items.iter().any(|o| {
                                is_copper(o.kind) && o.id != s.id && o.net == s.net && share_layer(o, s) && o.geom.iter().filter(|r| geom::signed_area(r) > 0.0).any(|r| geom::contains(r, *end))
                            });
                            if touched != *want {
                                push(findings, rule, format!("{} ends at {end} on nothing of its net (dangling)", s.id), Some(*end), features_of(s));
                            }
                        }
                    }
                }
                Feature::Pad => {
                    for s in &subjects {
                        let Some(net) = &s.net else { continue };
                        let pins = items.iter().find(|n| n.kind == Feature::Net && n.net.as_deref() == Some(net.as_str())).and_then(|n| n.attrs.get("pins").copied()).unwrap_or(0.0);
                        if pins < 2.0 {
                            continue;
                        }
                        let touched = items.iter().any(|o| is_copper(o.kind) && o.id != s.id && o.net == s.net && share_layer(o, s) && geom::overlaps(&o.geom, &s.geom));
                        if touched != *want {
                            push(findings, rule, format!("{} is not connected to anything on net {net}", s.id), Some(s.center), features_of(s));
                        }
                    }
                }
                _ => {}
            }
        }
        Check::Exists(want) => {
            if *want && subjects.is_empty() {
                push(findings, rule, format!("no {}", rule.for_), None, vec![]);
            }
            if !*want {
                for s in &subjects {
                    push(findings, rule, format!("{} is not allowed here", s.id), Some(s.center), features_of(s));
                }
            }
        }
        Check::Placed(want) => {
            for s in &subjects {
                if s.attrs.get("virtual").copied().unwrap_or(0.0) > 0.0 {
                    continue;
                }
                let placed = s.attrs.get("placed").copied().unwrap_or(0.0) > 0.0;
                if placed != *want {
                    push(findings, rule, format!("{} ({}) is not placed", s.id, s.component.as_deref().unwrap_or("?")), None, features_of(s));
                }
            }
        }
        Check::DesignRules(mins) => {
            for (k, v) in mins {
                let have = match k.as_str() {
                    "trace_width" => rules.trace_width,
                    "clearance" => rules.clearance,
                    "via_drill" => rules.via_drill,
                    "via_diameter" => rules.via_diameter,
                    "pour_clearance" => rules.pour_clearance(),
                    "edge_clearance" => rules.edge_clearance,
                    "silk_width" => rules.silk_width,
                    "silk_text_size" => rules.silk_text_size,
                    "mask_expansion" => rules.mask_expansion,
                    "hole_annular_ring" => rules.hole_annular_ring,
                    "thermal_spoke_width" => rules.thermal_spoke_width,
                    "pour_min_width" => rules.pour_min_width(),
                    "hole_clearance" => rules.hole_clearance(),
                    _ => continue,
                };
                if have.mm() + 1e-6 < *v {
                    push(findings, rule, format!("design rule {k} is {have}, the process needs at least {v} mm"), None, vec![k.clone()]);
                }
            }
        }
    }
    Ok(())
}
