//! Semantic comparison of two project files: the same board written in a
//! different order, with traces drawn the other way round or marked as
//! routed, compares equal. Everything else is reported.

use crate::schema::{Project, Trace, Via};
use crate::units::Point;
use std::collections::BTreeMap;

fn canon_points(points: &[Point]) -> Vec<Point> {
    let fwd: Vec<(i64, i64)> = points.iter().map(|p| (p.x.nm(), p.y.nm())).collect();
    let rev: Vec<(i64, i64)> = fwd.iter().rev().copied().collect();
    if rev < fwd {
        points.iter().rev().copied().collect()
    } else {
        points.to_vec()
    }
}

fn trace_key(t: &Trace) -> String {
    let pts = canon_points(&t.points).iter().map(|p| format!("{},{}", p.x.nm(), p.y.nm())).collect::<Vec<_>>().join(";");
    format!("{}|{}|{}|{}", t.layer, t.net.as_deref().unwrap_or(""), t.width.nm(), pts)
}

fn trace_desc(t: &Trace) -> String {
    format!("{} trace on {} w {}: {}", t.net.as_deref().unwrap_or("(no net)"), t.layer, t.width, crate::changes::pts(&t.points))
}

fn via_key(v: &Via) -> String {
    let mut layers = v.layers.clone();
    layers.sort();
    format!("{},{}|{}|{}|{}|{}", v.at.x.nm(), v.at.y.nm(), v.net.as_deref().unwrap_or(""), v.drill.nm(), v.diameter.nm(), layers.join("+"))
}

fn via_desc(v: &Via) -> String {
    format!("via {} at {} drill {} dia {}", v.net.as_deref().unwrap_or("(no net)"), v.at, v.drill, v.diameter)
}

fn multiset<'a, T>(items: &'a [T], key: impl Fn(&T) -> String) -> BTreeMap<String, Vec<&'a T>> {
    let mut m: BTreeMap<String, Vec<&T>> = BTreeMap::new();
    for it in items {
        m.entry(key(it)).or_default().push(it);
    }
    m
}

fn json(v: &impl serde::Serialize) -> serde_json::Value {
    serde_json::to_value(v).unwrap_or(serde_json::Value::Null)
}

/// Differences from `a` to `b`, one line each; empty when equivalent.
pub fn diff(a: &Project, b: &Project, a_name: &str, b_name: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if a.name != b.name {
        out.push(format!("name: `{}` vs `{}`", a.name, b.name));
    }
    for (what, va, vb) in [
        ("stackup", json(&a.stackup), json(&b.stackup)),
        ("design rules", json(&a.design_rules), json(&b.design_rules)),
        ("outline", json(&a.outline), json(&b.outline)),
        ("component indexes", json(&a.component_indexes), json(&b.component_indexes)),
        ("drc config", json(&a.drc), json(&b.drc)),
        ("routing config", json(&a.routing), json(&b.routing)),
        ("simulations", json(&a.simulations), json(&b.simulations)),
    ] {
        if va != vb {
            match (va.as_object(), vb.as_object()) {
                (Some(oa), Some(ob)) => {
                    let mut keys: Vec<&String> = oa.keys().chain(ob.keys()).collect();
                    keys.sort();
                    keys.dedup();
                    for k in keys {
                        if oa.get(k) != ob.get(k) {
                            out.push(format!("{what}.{k}: {} vs {}", oa.get(k).map(|v| v.to_string()).unwrap_or_else(|| "(absent)".into()), ob.get(k).map(|v| v.to_string()).unwrap_or_else(|| "(absent)".into())));
                        }
                    }
                }
                _ => out.push(format!("{what} differ")),
            }
        }
    }
    // Components.
    let mut refs: Vec<&String> = a.components.keys().chain(b.components.keys()).collect();
    refs.sort();
    refs.dedup();
    for r in refs {
        match (a.components.get(r), b.components.get(r)) {
            (Some(_), None) => out.push(format!("part {r}: only in {a_name}")),
            (None, Some(_)) => out.push(format!("part {r}: only in {b_name}")),
            (Some(ia), Some(ib)) => {
                if ia.component != ib.component {
                    out.push(format!("part {r}: component `{}` vs `{}`", ia.component, ib.component));
                }
                if ia.parameters != ib.parameters {
                    out.push(format!("part {r}: parameters {} vs {}", json(&ia.parameters), json(&ib.parameters)));
                }
                let pa = ia.placement.as_ref().map(|p| (p.at, p.rotation.rem_euclid(360.0), p.side, p.label_at, p.label_size, p.label_hidden));
                let pb = ib.placement.as_ref().map(|p| (p.at, p.rotation.rem_euclid(360.0), p.side, p.label_at, p.label_size, p.label_hidden));
                if pa != pb {
                    let show = |p: &Option<crate::schema::Placement>| p.as_ref().map(|p| format!("{} rot {} {}", p.at, p.rotation, p.side)).unwrap_or_else(|| "unplaced".into());
                    out.push(format!("part {r}: {} vs {}", show(&ia.placement), show(&ib.placement)));
                }
            }
            (None, None) => {}
        }
    }
    // Nets: by name, pin sets.
    let mut names: Vec<&String> = a.nets.keys().chain(b.nets.keys()).collect();
    names.sort();
    names.dedup();
    for n in names {
        match (a.nets.get(n), b.nets.get(n)) {
            (Some(_), None) => out.push(format!("net {n}: only in {a_name}")),
            (None, Some(_)) => out.push(format!("net {n}: only in {b_name}")),
            (Some(na), Some(nb)) => {
                let mut pa = na.pins.clone();
                let mut pb = nb.pins.clone();
                pa.sort();
                pb.sort();
                if pa != pb {
                    let only_a: Vec<&String> = pa.iter().filter(|p| !pb.contains(p)).collect();
                    let only_b: Vec<&String> = pb.iter().filter(|p| !pa.contains(p)).collect();
                    out.push(format!("net {n}: pins differ (only {a_name}: {:?}; only {b_name}: {:?})", only_a, only_b));
                }
                if na.class != nb.class {
                    out.push(format!("net {n}: class {:?} vs {:?}", na.class, nb.class));
                }
            }
            (None, None) => {}
        }
    }
    // Holes, texts, pours: as sorted JSON.
    for (what, la, lb) in [
        ("hole", a.holes.iter().map(json).collect::<Vec<_>>(), b.holes.iter().map(json).collect::<Vec<_>>()),
        ("text", a.texts.iter().map(json).collect::<Vec<_>>(), b.texts.iter().map(json).collect::<Vec<_>>()),
        ("pour", a.pours.iter().map(json).collect::<Vec<_>>(), b.pours.iter().map(json).collect::<Vec<_>>()),
    ] {
        let ka: Vec<String> = la.iter().map(|v| v.to_string()).collect();
        let kb: Vec<String> = lb.iter().map(|v| v.to_string()).collect();
        for k in &ka {
            if !kb.contains(k) {
                out.push(format!("{what} only in {a_name}: {k}"));
            }
        }
        for k in &kb {
            if !ka.contains(k) {
                out.push(format!("{what} only in {b_name}: {k}"));
            }
        }
    }
    // Traces and vias as multisets.
    let ta = multiset(&a.traces, trace_key);
    let tb = multiset(&b.traces, trace_key);
    for (k, items) in &ta {
        let nb = tb.get(k).map_or(0, |v| v.len());
        for t in items.iter().skip(nb) {
            out.push(format!("trace only in {a_name}: {}", trace_desc(t)));
        }
    }
    for (k, items) in &tb {
        let na = ta.get(k).map_or(0, |v| v.len());
        for t in items.iter().skip(na) {
            out.push(format!("trace only in {b_name}: {}", trace_desc(t)));
        }
    }
    let va = multiset(&a.vias, via_key);
    let vb = multiset(&b.vias, via_key);
    for (k, items) in &va {
        let nb = vb.get(k).map_or(0, |v| v.len());
        for v in items.iter().skip(nb) {
            out.push(format!("via only in {a_name}: {}", via_desc(v)));
        }
    }
    for (k, items) in &vb {
        let na = va.get(k).map_or(0, |v| v.len());
        for v in items.iter().skip(na) {
            out.push(format!("via only in {b_name}: {}", via_desc(v)));
        }
    }
    out
}
