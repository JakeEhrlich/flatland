//! Write a Specctra DSN design file for freerouting.
//!
//! Units: micrometres with 0.1 µm resolution. Coordinates are our board
//! frame (y up), which is what freerouting expects (KiCad negates its
//! y-down coordinates on export).

use super::sexpr::quote;
use crate::error::{Error, Result};
use crate::geom::{self, Ring};
use crate::model::{Board, BoardPad};
use crate::schema::{PadType, Side};
use crate::units::{Length, Point};
use indexmap::IndexMap;
use std::fmt::Write;

fn um(l: Length) -> String {
    let v = l.um();
    let s = format!("{v:.1}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}
fn xy(p: Point) -> String {
    format!("{} {}", um(p.x), um(p.y))
}
fn ring_coords(r: &Ring) -> String {
    let mut s = String::new();
    for p in r {
        let _ = write!(s, " {}", xy(*p));
    }
    // Close the path explicitly.
    if let Some(f) = r.first() {
        let _ = write!(s, " {}", xy(*f));
    }
    s
}

pub fn via_name(diameter: Length, drill: Length, n_layers: usize) -> String {
    format!("Via[0-{}]_{}:{}_um", n_layers.saturating_sub(1), um(diameter), um(drill))
}

/// Parse `diameter:drill` back out of a via padstack name.
pub fn parse_via_name(name: &str) -> Option<(Length, Length)> {
    let rest = name.split('_').nth(1)?;
    let (d, h) = rest.split_once(':')?;
    Some((Length::from_mm(d.parse::<f64>().ok()? / 1000.0), Length::from_mm(h.parse::<f64>().ok()? / 1000.0)))
}

struct Image {
    name: String,
    body: String,
}

/// Padstack name for a pad in the image frame (unrotated by placement).
fn padstack_key(pad: &BoardPad, local_ring: &Ring) -> String {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    use std::hash::{Hash, Hasher};
    for p in local_ring {
        p.x.nm().hash(&mut h);
        p.y.nm().hash(&mut h);
    }
    let kind = match pad.pad_type {
        PadType::Smd => "T",
        PadType::ThroughHole => "A",
    };
    let shape = match pad.shape {
        crate::schema::PadShape::Circle => "Round",
        crate::schema::PadShape::Rect => "Rect",
        crate::schema::PadShape::RoundRect => "RoundRect",
        crate::schema::PadShape::Oval => "Oval",
    };
    format!("{shape}[{kind}]Pad_{}x{}_{:08x}_um", um(pad.size[0]), um(pad.size[1]), h.finish() as u32)
}

pub fn emit(board: &Board, planes: bool) -> Result<String> {
    let outline = board
        .outline
        .as_ref()
        .ok_or_else(|| Error::with_help("cannot route without a board outline", "define one with `pcb outline rect W H`"))?;
    let layers = board.layers();
    let rules = board.rules();
    let n = layers.len();
    let mut s = String::new();
    let _ = writeln!(s, "(pcb {}", quote(&format!("{}.dsn", board.project.name)));
    s.push_str("  (parser\n    (string_quote \")\n    (space_in_quoted_tokens on)\n    (host_cad \"flatland\")\n    (host_version \"0.1\")\n  )\n");
    s.push_str("  (resolution um 10)\n  (unit um)\n");

    // ---- structure
    s.push_str("  (structure\n");
    // Route with 45° corners rather than free angles.
    s.push_str("    (snap_angle fortyfive_degree)\n");
    // With planes, a layer whose netted pour follows the outline is a power
    // layer: freerouting keeps signal traces off it (otherwise it routes across
    // the plane and carves the fill into islands).
    // A layer whose only copper is one outline-following netted pour is a
    // plane layer whatever the flags: the router must not run signals across it.
    let plane_layers: Vec<String> = board.plane_layers()?.into_iter().map(|(l, _)| l).collect();
    for (i, l) in layers.iter().enumerate() {
        let kind = if plane_layers.contains(l) { "power" } else { "signal" };
        let _ = writeln!(s, "    (layer {} (type {kind}) (property (index {i})))", quote(l));
    }
    let _ = writeln!(s, "    (boundary (path pcb 0{}))", ring_coords(outline));
    // Non-plated holes and unassigned plated holes are keepouts.
    for h in &board.holes {
        if h.hole.net.is_none() {
            let ring = h.copper.clone().unwrap_or_else(|| h.ring.clone());
            let grown = geom::offset(&[ring], rules.clearance);
            for r in grown {
                for l in layers {
                    let _ = writeln!(s, "    (keepout \"\" (polygon {} 0{}))", quote(l), ring_coords(&r));
                }
            }
        }
    }
    // The router keeps its own clearance from the boundary, not the project's
    // `edge_clearance`: a keepout ring between the outline and the outline
    // inset by that clearance, on every layer, makes it honour ours.
    if rules.edge_clearance.nm() > 0 {
        let inset = geom::offset(&[outline.clone()], -rules.edge_clearance);
        let ring = geom::difference(&[outline.clone()], &inset)?;
        for r in geom::fracture(&ring) {
            for l in layers {
                let _ = writeln!(s, "    (keepout \"\" (polygon {} 0{}))", quote(l), ring_coords(&r));
            }
        }
    }
    // Copper text belongs to no net: keep the router off it.
    for t in board.project.texts.iter().filter(|t| layers.iter().any(|l| *l == t.layer)) {
        let grown = geom::offset(&board.text_copper(t), rules.clearance);
        for r in grown {
            let _ = writeln!(s, "    (keepout \"\" (polygon {} 0{}))", quote(&t.layer), ring_coords(&r));
        }
    }
    // A pour's net is an ordinary net to the router unless planes are asked
    // for: a `plane` tells the router the net is solid copper across the
    // layer, so it never draws that net and freely walls a pad of it in with
    // other traces (a built board shipped with its timing capacitor's ground
    // pad in such a pocket). Routed as a net, every pad gets a trace or a via,
    // and the pour then swallows whatever it covers. Planes are still an
    // option on dense multi-layer boards where routing the ground net is too
    // expensive.
    // Plane layers are always planes (their nets reach SMD pads through the
    // fanout vias `pcb route` draws first); with --planes every other netted
    // pour is one too.
    for p in board.pours()? {
        if let Some(net) = &p.pour.net {
            if plane_layers.contains(&p.pour.layer) || (planes && n > 1) {
                let _ = writeln!(s, "    (plane {} (polygon {} 0{}))", quote(net), quote(&p.pour.layer), ring_coords(&p.outline));
            }
        }
    }
    let via = via_name(rules.via_diameter, rules.via_drill, n);
    let mut via_names: IndexMap<String, (Length, Length)> = IndexMap::new();
    via_names.insert(via.clone(), (rules.via_diameter, rules.via_drill));
    for c in rules.net_classes.values() {
        let d = c.via_diameter.unwrap_or(rules.via_diameter);
        let h = c.via_drill.unwrap_or(rules.via_drill);
        via_names.insert(via_name(d, h, n), (d, h));
    }
    // Every via size the board already uses needs a padstack too: protected
    // wiring that names an undeclared padstack makes freerouting reject the file.
    for v in &board.project.vias {
        via_names.insert(via_name(v.diameter, v.drill, n), (v.diameter, v.drill));
    }
    // Freerouting needs a via rule even on a single-sided board (it cannot use it there).
    let _ = writeln!(s, "    (via {})", via_names.keys().map(|v| quote(v)).collect::<Vec<_>>().join(" "));
    let _ = writeln!(
        s,
        "    (rule (width {}) (clearance {}) (clearance {} (type default_smd)) (clearance {} (type smd_smd)))",
        um(rules.trace_width),
        um(rules.clearance),
        um(rules.clearance),
        um(rules.clearance)
    );
    s.push_str("  )\n");

    // ---- library images & padstacks (built while walking placements)
    let mut images: IndexMap<String, Image> = IndexMap::new();
    let mut image_keys: IndexMap<String, String> = IndexMap::new();
    let mut padstacks: IndexMap<String, String> = IndexMap::new();
    let mut placements: IndexMap<String, Vec<String>> = IndexMap::new();
    for inst in &board.instances {
        let (Some(fp), Some(pl)) = (&inst.component.footprint, &inst.instance.placement) else { continue };
        let image_name = {
            let key = fp.path.display().to_string();
            let base = fp.footprint.name.clone();
            // Footprints with the same name but from different files get a suffix.
            let mut name = base.clone();
            let mut k = 1;
            while image_keys.get(&name).map_or(false, |existing| existing != &key) {
                k += 1;
                name = format!("{base}~{k}");
            }
            if !images.contains_key(&name) {
                let mut body = String::new();
                // Pads in the footprint frame. A pin with several solder tabs
                // becomes one DSN pin whose padstack holds all the tabs, so the
                // router knows they are already connected.
                let mut secondary: std::collections::HashMap<&str, (&str, Point)> = std::collections::HashMap::new();
                let mut extra_shapes: std::collections::HashMap<&str, Vec<Ring>> = std::collections::HashMap::new();
                for pin in &inst.component.component.pins {
                    let names = pin.pad_names();
                    if names.len() > 1 {
                        let primary = fp.footprint.pads.iter().find(|p| p.name == names[0]);
                        for extra in &names[1..] {
                            if let (Some(pp), Some(ep)) = (primary, fp.footprint.pads.iter().find(|p| p.name == *extra)) {
                                secondary.insert(ep.name.as_str(), (pp.name.as_str(), pp.at));
                                let ring = local_pad_ring(ep);
                                let shifted: Ring = ring.iter().map(|q| *q + ep.at - pp.at).collect();
                                extra_shapes.entry(pp.name.as_str()).or_default().push(shifted);
                            }
                        }
                    }
                }
                for pad in &fp.footprint.pads {
                    if secondary.contains_key(pad.name.as_str()) {
                        continue;
                    }
                    let mut local = vec![local_pad_ring(pad)];
                    if let Some(extra) = extra_shapes.get(pad.name.as_str()) {
                        local.extend(extra.iter().cloned());
                    }
                    let bp = inst.pads.iter().find(|p| p.pad_name == pad.name).unwrap();
                    let ps = padstack_key(bp, &local.concat());
                    padstacks.entry(ps.clone()).or_insert_with(|| padstack_body(&ps, bp, &local, layers));
                    let _ = writeln!(body, "      (pin {} {} {})", quote(&ps), quote(&pad.name), xy(pad.at));
                }
                for g in &fp.footprint.silkscreen {
                    for (pts, closed) in g.polylines() {
                        let mut pts = pts;
                        if closed {
                            if let Some(f) = pts.first().copied() {
                                pts.push(f);
                            }
                        }
                        let mut c = String::new();
                        for p in &pts {
                            let _ = write!(c, " {}", xy(*p));
                        }
                        let _ = writeln!(body, "      (outline (path signal {}{}))", um(g.width().unwrap_or(rules.silk_width)), c);
                    }
                }
                images.insert(name.clone(), Image { name: name.clone(), body });
                image_keys.insert(name.clone(), key);
            }
            name
        };
        let side = match pl.side {
            Side::Top => "front",
            Side::Bottom => "back",
        };
        let mut place = format!("      (place {} {} {} {}", quote(&inst.refdes), xy(pl.at), side, fmt_angle(pl.rotation));
        if let Some(v) = inst.instance.parameters.get("value") {
            let _ = write!(place, " (PN {})", quote(v));
        }
        if pl.locked {
            place.push_str(" (lock_type position)");
        }
        place.push(')');
        placements.entry(image_name).or_default().push(place);
    }
    // Plated holes with nets become single-pin virtual components.
    let mut hole_places = Vec::new();
    for (i, h) in board.holes.iter().enumerate() {
        if let (Some(_net), Some(c)) = (&h.hole.net, &h.copper) {
            let name = format!("HOLE{}", i + 1);
            let local: Ring = c.iter().map(|p| *p - h.hole.at).collect();
            let ps = format!("Round[A]Hole_{}_um", um(Length::from_mm(geom::signed_area(&local).abs().sqrt() / 1e6)));
            let mut body = String::new();
            for l in layers {
                let _ = writeln!(body, "      (shape (polygon {} 0{}))", quote(l), ring_coords(&local));
            }
            padstacks.entry(ps.clone()).or_insert(format!("    (padstack {}\n{}      (attach off)\n    )\n", quote(&ps), body));
            images.insert(name.clone(), Image { name: name.clone(), body: format!("      (pin {} 1 0 0)\n", quote(&ps)) });
            hole_places.push((name.clone(), format!("      (place {} {} front 0 (lock_type position))", quote(&name), xy(h.hole.at))));
        }
    }

    // ---- placement
    s.push_str("  (placement\n");
    for (img, places) in &placements {
        let _ = writeln!(s, "    (component {}", quote(img));
        for p in places {
            s.push_str(p);
            s.push('\n');
        }
        s.push_str("    )\n");
    }
    for (img, place) in &hole_places {
        let _ = writeln!(s, "    (component {}\n{}\n    )", quote(img), place);
    }
    s.push_str("  )\n");

    // ---- library
    s.push_str("  (library\n");
    for im in images.values() {
        let _ = writeln!(s, "    (image {}\n{}    )", quote(&im.name), im.body);
    }
    for body in padstacks.values() {
        s.push_str(body);
    }
    for (name, (d, _)) in via_names.iter() {
        let _ = writeln!(s, "    (padstack {}", quote(name));
        for l in layers {
            let _ = writeln!(s, "      (shape (circle {} {}))", quote(l), um(*d));
        }
        s.push_str("      (attach off)\n    )\n");
    }
    s.push_str("  )\n");

    // ---- network
    s.push_str("  (network\n");
    let mut class_nets: IndexMap<String, Vec<String>> = IndexMap::new();
    for net in &board.nets {
        let mut pins: Vec<String> = Vec::new();
        for (r, pin) in &net.pins {
            // Only the primary pad: secondary tabs live inside its padstack.
            if let Ok(pad) = board.pad(r, pin) {
                pins.push(quote(&format!("{}-{}", pad.refdes, pad.pad_name)));
            }
        }
        for (i, h) in board.holes.iter().enumerate() {
            if h.hole.net.as_deref() == Some(&net.name) && h.copper.is_some() {
                pins.push(format!("HOLE{}-1", i + 1));
            }
        }
        if pins.is_empty() {
            continue;
        }
        let _ = writeln!(s, "    (net {} (pins {}))", quote(&net.name), pins.join(" "));
        class_nets.entry(net.class.clone().unwrap_or_else(|| "default".into())).or_default().push(quote(&net.name));
    }
    for (class, nets) in &class_nets {
        let (w, c) = rules.class(if class == "default" { None } else { Some(class) });
        let (vd, vh) = match rules.net_classes.get(class) {
            Some(nc) => (nc.via_diameter.unwrap_or(rules.via_diameter), nc.via_drill.unwrap_or(rules.via_drill)),
            None => (rules.via_diameter, rules.via_drill),
        };
        let circuit = format!("\n      (circuit (use_via {}))", quote(&via_name(vd, vh, n)));
        let _ = writeln!(
            s,
            "    (class {} {}{}\n      (rule (width {}) (clearance {}))\n    )",
            quote(class),
            nets.join(" "),
            circuit,
            um(w),
            um(c)
        );
    }
    s.push_str("  )\n");

    // ---- existing hand-drawn wiring, protected.
    // Everything present when the DSN is written is wiring to keep: `pcb route`
    // has already removed the router's previous output unless asked to keep it,
    // and the fanout vias it drew for plane nets must survive the router.
    let fixed_traces: Vec<_> = board.project.traces.iter().collect();
    let fixed_vias: Vec<_> = board.project.vias.iter().collect();
    if !fixed_traces.is_empty() || !fixed_vias.is_empty() {
        s.push_str("  (wiring\n");
        for t in fixed_traces {
            let mut c = String::new();
            for p in &t.points {
                let _ = write!(c, " {}", xy(*p));
            }
            let _ = writeln!(
                s,
                "    (wire (path {} {}{}){} (type protect))",
                quote(&t.layer),
                um(t.width),
                c,
                t.net.as_ref().map(|n| format!(" (net {})", quote(n))).unwrap_or_default()
            );
        }
        for v in fixed_vias {
            let _ = writeln!(
                s,
                "    (via {} {}{} (type protect))",
                quote(&via_name(v.diameter, v.drill, n)),
                xy(v.at),
                v.net.as_ref().map(|n| format!(" (net {})", quote(n))).unwrap_or_default()
            );
        }
        s.push_str("  )\n");
    }
    s.push_str(")\n");
    Ok(s)
}

fn fmt_angle(deg: f64) -> String {
    let d = deg.rem_euclid(360.0);
    if d.fract() == 0.0 { format!("{}", d as i64) } else { format!("{d}") }
}

/// Pad copper outline in the footprint frame (no placement transform).
fn local_pad_ring(pad: &crate::schema::Pad) -> Ring {
    use crate::schema::PadShape;
    let base = match pad.shape {
        PadShape::Circle => geom::circle(Point::ORIGIN, pad.size[0]),
        PadShape::Rect => geom::rect(Point::ORIGIN, pad.size[0], pad.size[1]),
        PadShape::RoundRect => geom::round_rect(Point::ORIGIN, pad.size[0], pad.size[1], pad.corner_radius.unwrap_or(Length(pad.size[0].nm().min(pad.size[1].nm()) / 4))),
        PadShape::Oval => geom::oval(Point::ORIGIN, pad.size[0], pad.size[1]),
    };
    geom::Transform { mirror_x: false, degrees: pad.rotation, translate: Point::ORIGIN }.apply_ring(&base)
}

fn padstack_body(name: &str, pad: &BoardPad, shapes: &[Ring], layers: &[String]) -> String {
    let mut s = format!("    (padstack {}\n", quote(name));
    let on: Vec<&String> = match pad.pad_type {
        // SMD padstacks are defined on the top layer; freerouting mirrors them for back-side parts.
        PadType::Smd => vec![&layers[0]],
        PadType::ThroughHole => layers.iter().collect(),
    };
    for l in on {
        if pad.shape == crate::schema::PadShape::Circle && shapes.len() == 1 {
            let _ = writeln!(s, "      (shape (circle {} {}))", quote(l), um(pad.size[0]));
        } else {
            for ring in shapes {
                let _ = writeln!(s, "      (shape (polygon {} 0{}))", quote(l), ring_coords(ring));
            }
        }
    }
    s.push_str("      (attach off)\n    )\n");
    s
}
