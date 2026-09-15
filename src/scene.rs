//! Board geometry for the UI (`pcb serve`), as JSON in millimetres, y up.

use crate::error::Result;
use crate::geom::Ring;
use crate::model::Board;
use crate::schema::Side;
use crate::units::Point;
use serde_json::{json, Value};
use std::path::Path;

fn pt(p: Point) -> Value {
    json!([round(p.x.mm()), round(p.y.mm())])
}

fn round(v: f64) -> f64 {
    (v * 1e4).round() / 1e4
}

fn ring(r: &Ring) -> Value {
    Value::Array(r.iter().map(|p| pt(*p)).collect())
}

fn rings(rs: &[Ring]) -> Value {
    Value::Array(rs.iter().map(ring).collect())
}

fn pts(ps: &[Point]) -> Value {
    Value::Array(ps.iter().map(|p| pt(*p)).collect())
}

/// Everything the page draws and edits, plus the check findings and airwires.
pub fn scene(board: &Board, project_path: &Path) -> Result<Value> {
    let rules = board.rules();
    let mut parts = Vec::new();
    for inst in &board.instances {
        let placement = inst.instance.placement.as_ref();
        let pads: Vec<Value> = inst
            .pads
            .iter()
            .map(|p| {
                json!({
                    "name": p.pad_name,
                    "pins": p.pins,
                    "net": p.net,
                    "layers": p.layers,
                    "ring": ring(&p.copper),
                    "center": pt(p.center),
                    "drill": p.drill.map(|d| round(d.mm())),
                    "plated": p.plated,
                })
            })
            .collect();
        let silk: Vec<Value> = inst.silk_lines.iter().map(|(pts_, closed, w)| json!({"points": pts(pts_), "closed": closed, "width": round(w.mm())})).collect();
        parts.push(json!({
            "refdes": inst.refdes,
            "component": inst.component.name,
            "footprint": inst.component.footprint.as_ref().map(|f| f.footprint.name.clone()),
            "value": inst.instance.parameters.get("value"),
            "note": inst.instance.note,
            "placed": inst.is_placed(),
            "at": placement.map(|p| pt(p.at)),
            "rotation": placement.map(|p| p.rotation).unwrap_or(0.0),
            "side": placement.map(|p| p.side.to_string()).unwrap_or_else(|| "top".into()),
            "locked": placement.map(|p| p.locked).unwrap_or(false),
            "label": inst.label_at.map(|at| json!({"at": pt(at), "size": round(inst.label_size(rules).mm()), "hidden": placement.map(|p| p.label_hidden).unwrap_or(false)})),
            "pads": pads,
            "silk": silk,
            "silk_polys": rings(&inst.silk_polys),
            "courtyard": inst.courtyard.iter().map(|(p, closed)| json!({"points": pts(p), "closed": closed})).collect::<Vec<_>>(),
        }));
    }
    let traces: Vec<Value> = board
        .project
        .traces
        .iter()
        .enumerate()
        .map(|(i, t)| json!({"index": i, "layer": t.layer, "net": t.net, "width": round(t.width.mm()), "points": pts(&t.points), "routed": t.routed}))
        .collect();
    let vias: Vec<Value> = board
        .project
        .vias
        .iter()
        .enumerate()
        .map(|(i, v)| json!({"index": i, "at": pt(v.at), "net": v.net, "diameter": round(v.diameter.mm()), "drill": round(v.drill.mm()), "layers": v.layers}))
        .collect();
    let pours: Vec<Value> = board.pours()?.iter().map(|p| json!({"name": p.pour.name, "layer": p.pour.layer, "net": p.pour.net, "rings": rings(&p.copper)})).collect();
    let texts: Vec<Value> = board
        .project
        .texts
        .iter()
        .enumerate()
        .map(|(i, t)| json!({"index": i, "text": t.text, "at": pt(t.at), "layer": t.layer, "size": round(t.size.mm()), "rotation": t.rotation, "rings": rings(&board.text_copper(t))}))
        .collect();
    let holes: Vec<Value> = board.holes.iter().map(|h| json!({"at": pt(h.hole.at), "drill": round(h.hole.drill.mm()), "plated": h.hole.plated, "net": h.hole.net, "ring": h.copper.as_ref().map(ring)})).collect();
    let report = crate::drc::run(board, project_path)?;
    let findings: Vec<Value> = report
        .findings
        .iter()
        .filter(|f| f.waived.is_none())
        .map(|f| json!({"rule": f.rule, "severity": f.severity.to_string(), "message": f.message, "at": f.at.map(pt), "features": f.features}))
        .collect();
    let unrouted: Vec<Value> = board.ratsnest()?.iter().map(|(n, a, b)| json!({"net": n, "from": pt(*a), "to": pt(*b)})).collect();
    let (lo, hi) = board.bbox().unwrap_or((Point::mm(0.0, 0.0), Point::mm(50.0, 30.0)));
    let mut nets: Vec<&String> = board.project.nets.keys().collect();
    nets.sort();
    Ok(json!({
        "name": board.project.name,
        "layers": board.layers(),
        "top": board.stackup().layer_for_side(Side::Top),
        "bottom": board.stackup().layer_for_side(Side::Bottom),
        "outline": board.outline.as_ref().map(ring),
        "bbox": [round(lo.x.mm()), round(lo.y.mm()), round(hi.x.mm()), round(hi.y.mm())],
        "rules": {"trace_width": round(rules.trace_width.mm()), "clearance": round(rules.clearance.mm()), "via_drill": round(rules.via_drill.mm()), "via_diameter": round(rules.via_diameter.mm()),
                  "classes": rules.net_classes.iter().map(|(k, c)| (k.clone(), json!({"width": c.trace_width.map(|w| round(w.mm()))}))).collect::<serde_json::Map<String, Value>>()},
        "nets": nets,
        "parts": parts,
        "traces": traces,
        "vias": vias,
        "pours": pours,
        "texts": texts,
        "holes": holes,
        "findings": findings,
        "unrouted": unrouted,
    }))
}
