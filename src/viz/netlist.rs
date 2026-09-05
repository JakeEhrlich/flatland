//! Netlist as a schematic-style drawing: every component is a box with
//! named pin stubs on its left and right sides; nets are wires between
//! stubs (with a junction dot for three-pin nets), ground gets a ground
//! symbol at each pin, and busy nets (four or more pins) are shown as
//! net-label flags at each pin instead of wires, as on a real schematic.
//! Box positions come from layout-rs's layered layout; everything else is
//! drawn here.

use crate::error::Result;
use crate::model::Board;
use crate::sim::netlist::node_name;
use layout::core::base::Orientation;
use layout::core::format::{ClipHandle, RenderBackend};
use layout::core::geometry::Point as LPoint;
use layout::core::style::StyleAttr;
use layout::std_shapes::shapes::{Arrow, Element, ShapeKind};
use layout::topo::layout::VisualGraph;
use std::collections::HashMap;
use std::fmt::Write;

const PIN_PITCH: f64 = 22.0;
const STUB: f64 = 26.0;
const HEADER: f64 = 30.0;
const FONT: f64 = 12.0;
const WIRE: &str = "#1f5fbf";
const INK: &str = "#202020";

struct NullBackend;
impl RenderBackend for NullBackend {
    fn draw_rect(&mut self, _: LPoint, _: LPoint, _: &StyleAttr, _: Option<String>, _: Option<ClipHandle>) {}
    fn draw_line(&mut self, _: LPoint, _: LPoint, _: &StyleAttr, _: Option<String>) {}
    fn draw_circle(&mut self, _: LPoint, _: LPoint, _: &StyleAttr, _: Option<String>) {}
    fn draw_text(&mut self, _: LPoint, _: &str, _: &StyleAttr) {}
    fn draw_arrow(&mut self, _: &[(LPoint, LPoint)], _: bool, _: (bool, bool), _: &StyleAttr, _: Option<String>, _: &str) {}
    fn create_clip(&mut self, _: LPoint, _: LPoint, _: usize) -> ClipHandle {
        0
    }
}

#[derive(Clone, Copy, PartialEq)]
enum NetStyle {
    Ground,
    Label,
    Wire,
}

struct Comp {
    refdes: String,
    title: String,
    subtitle: String,
    pins: Vec<String>,
    w: f64,
    h: f64,
    cx: f64,
    cy: f64,
    /// pin -> (side is right?, index on that side)
    slots: HashMap<String, (bool, usize)>,
    left: Vec<String>,
    right: Vec<String>,
}

impl Comp {
    fn pin_point(&self, pin: &str) -> Option<(f64, f64, bool)> {
        let (right, i) = *self.slots.get(pin)?;
        let y = self.cy - self.h / 2.0 + HEADER + PIN_PITCH * (i as f64 + 0.5);
        let x = if right { self.cx + self.w / 2.0 + STUB } else { self.cx - self.w / 2.0 - STUB };
        Some((x, y, right))
    }
}

fn text_w(s: &str, size: f64) -> f64 {
    s.chars().count() as f64 * size * 0.6
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

pub fn render(board: &Board) -> Result<String> {
    Ok(std::panic::catch_unwind(|| draw(board, true)).unwrap_or_else(|_| draw(board, false)))
}

fn net_style(name: &str, pins: usize) -> NetStyle {
    if node_name(name) == "0" {
        NetStyle::Ground
    } else if pins >= 4 {
        NetStyle::Label
    } else {
        NetStyle::Wire
    }
}

fn draw(board: &Board, use_layout: bool) -> String {
    // ---- components
    let mut comps: Vec<Comp> = board
        .instances
        .iter()
        .map(|inst| {
            let pins: Vec<String> = inst.component.component.pins.iter().map(|p| p.name.clone()).collect();
            let subtitle = format!(
                "{}{}",
                inst.component.name,
                inst.instance.note.as_deref().or_else(|| inst.instance.parameters.get("value").map(|s| s.as_str())).map(|v| format!("  {v}")).unwrap_or_default()
            );
            let pin_w = pins.iter().map(|p| text_w(p, FONT)).fold(0.0, f64::max);
            let w = (text_w(&subtitle, FONT).max(text_w(&inst.refdes, FONT * 1.2)) + 20.0).max(2.0 * pin_w + 50.0).max(110.0);
            let rows = pins.len().div_ceil(2).max(1);
            let h = HEADER + rows as f64 * PIN_PITCH + 8.0;
            Comp { refdes: inst.refdes.clone(), title: inst.refdes.clone(), subtitle, pins, w, h, cx: 0.0, cy: 0.0, slots: HashMap::new(), left: vec![], right: vec![] }
        })
        .collect();
    let index: HashMap<String, usize> = comps.iter().enumerate().map(|(i, c)| (c.refdes.clone(), i)).collect();

    // ---- nets
    struct NetDraw {
        name: String,
        style: NetStyle,
        pins: Vec<(usize, String)>,
        junction: Option<(f64, f64)>,
    }
    let mut nets: Vec<NetDraw> = board
        .nets
        .iter()
        .map(|n| {
            let pins: Vec<(usize, String)> = n.pins.iter().filter_map(|(r, p)| index.get(r).map(|i| (*i, p.clone()))).collect();
            NetDraw { name: n.name.clone(), style: net_style(&n.name, pins.len()), pins, junction: None }
        })
        .collect();

    // ---- positions
    if use_layout && !comps.is_empty() {
        let mut vg = VisualGraph::new(Orientation::LeftToRight);
        let style = StyleAttr::simple();
        let handles: Vec<_> = comps
            .iter()
            .map(|c| vg.add_node(Element::create(ShapeKind::new_box(&c.refdes), style.clone(), Orientation::LeftToRight, LPoint::new(c.w + 2.0 * STUB, c.h))))
            .collect();
        let mut junctions: Vec<Option<_>> = vec![None; nets.len()];
        for (ni, n) in nets.iter().enumerate() {
            if n.style != NetStyle::Wire {
                continue;
            }
            if n.pins.len() == 2 {
                if n.pins[0].0 != n.pins[1].0 {
                    vg.add_edge(Arrow::invisible(), handles[n.pins[0].0], handles[n.pins[1].0]);
                }
            } else if n.pins.len() >= 3 {
                let j = vg.add_node(Element::create(ShapeKind::new_circle(&n.name), style.clone(), Orientation::LeftToRight, LPoint::new(24.0, 24.0)));
                for (ci, _) in &n.pins {
                    vg.add_edge(Arrow::invisible(), handles[*ci], j);
                }
                junctions[ni] = Some(j);
            }
        }
        vg.do_it(false, false, false, &mut NullBackend);
        for (c, h) in comps.iter_mut().zip(handles.iter()) {
            let m = vg.pos(*h).middle();
            c.cx = m.x;
            c.cy = m.y;
        }
        for (ni, j) in junctions.iter().enumerate() {
            if let Some(j) = j {
                let m = vg.pos(*j).middle();
                nets[ni].junction = Some((m.x, m.y));
            }
        }
    } else {
        // Simple grid fallback.
        let cols = (comps.len() as f64).sqrt().ceil().max(1.0) as usize;
        for (i, c) in comps.iter_mut().enumerate() {
            c.cx = 120.0 + (i % cols) as f64 * 260.0;
            c.cy = 80.0 + (i / cols) as f64 * 160.0;
        }
        for n in nets.iter_mut() {
            if n.style == NetStyle::Wire && n.pins.len() >= 3 {
                let xs: f64 = n.pins.iter().map(|(ci, _)| comps[*ci].cx).sum::<f64>() / n.pins.len() as f64;
                let ys: f64 = n.pins.iter().map(|(ci, _)| comps[*ci].cy).sum::<f64>() / n.pins.len() as f64;
                n.junction = Some((xs, ys + 90.0));
            }
        }
    }

    // ---- pin sides: face the wire's other end; labels/ground go to the emptier side.
    let mut targets: HashMap<(usize, String), (f64, f64)> = HashMap::new();
    for n in &nets {
        if n.style != NetStyle::Wire {
            continue;
        }
        for (ci, pin) in &n.pins {
            let others: Vec<(f64, f64)> = match n.junction {
                Some(j) => vec![j],
                None => n.pins.iter().filter(|(c, p)| !(c == ci && p == pin)).map(|(c, _)| (comps[*c].cx, comps[*c].cy)).collect(),
            };
            if !others.is_empty() {
                let tx = others.iter().map(|o| o.0).sum::<f64>() / others.len() as f64;
                let ty = others.iter().map(|o| o.1).sum::<f64>() / others.len() as f64;
                targets.insert((*ci, pin.clone()), (tx, ty));
            }
        }
    }
    for (ci, c) in comps.iter_mut().enumerate() {
        let mut left: Vec<(f64, String)> = vec![];
        let mut right: Vec<(f64, String)> = vec![];
        let mut free: Vec<String> = vec![];
        for p in &c.pins {
            match targets.get(&(ci, p.clone())) {
                Some((tx, ty)) => {
                    if *tx < c.cx { left.push((*ty, p.clone())) } else { right.push((*ty, p.clone())) }
                }
                None => free.push(p.clone()),
            }
        }
        left.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        right.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        let (mut l, mut r): (Vec<String>, Vec<String>) = (left.into_iter().map(|x| x.1).collect(), right.into_iter().map(|x| x.1).collect());
        for p in free {
            if l.len() <= r.len() { l.push(p) } else { r.push(p) }
        }
        let rows = l.len().max(r.len()).max(1);
        c.h = HEADER + rows as f64 * PIN_PITCH + 8.0;
        for (i, p) in l.iter().enumerate() {
            c.slots.insert(p.clone(), (false, i));
        }
        for (i, p) in r.iter().enumerate() {
            c.slots.insert(p.clone(), (true, i));
        }
        c.left = l;
        c.right = r;
    }

    // ---- extents
    let (mut x0, mut y0, mut x1, mut y1) = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    for c in &comps {
        x0 = x0.min(c.cx - c.w / 2.0 - STUB - 60.0);
        x1 = x1.max(c.cx + c.w / 2.0 + STUB + 60.0);
        y0 = y0.min(c.cy - c.h / 2.0 - 30.0);
        y1 = y1.max(c.cy + c.h / 2.0 + 40.0);
    }
    if comps.is_empty() {
        (x0, y0, x1, y1) = (0.0, 0.0, 400.0, 100.0);
    }
    let margin = 30.0;
    let (vx, vy, vw, vh) = (x0 - margin, y0 - margin - 30.0, x1 - x0 + 2.0 * margin, y1 - y0 + 2.0 * margin + 30.0);

    let mut s = String::new();
    let _ = write!(s, "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"{vx} {vy} {vw} {vh}\" width=\"{vw}\" height=\"{vh}\" font-family=\"Helvetica, Arial, sans-serif\">");
    let _ = write!(s, "<rect x=\"{vx}\" y=\"{vy}\" width=\"{vw}\" height=\"{vh}\" fill=\"white\"/>");
    let _ = write!(s, "<text x=\"{}\" y=\"{}\" font-size=\"16\" font-weight=\"bold\" fill=\"{INK}\">{} — netlist</text>", x0, y0 - 12.0, esc(&board.project.name));

    // ---- wires first (under the boxes)
    // Boxes (with their stubs) that vertical wire runs should avoid.
    let boxes: Vec<(f64, f64, f64, f64)> = comps.iter().map(|c| (c.cx - c.w / 2.0 - STUB + 4.0, c.cy - c.h / 2.0, c.cx + c.w / 2.0 + STUB - 4.0, c.cy + c.h / 2.0)).collect();
    let clear_x = |mut xm: f64, y0: f64, y1: f64| -> f64 {
        let (ylo, yhi) = (y0.min(y1), y0.max(y1));
        for _ in 0..6 {
            let hit = boxes.iter().find(|(bx0, by0, bx1, by1)| xm > *bx0 && xm < *bx1 && yhi > *by0 && ylo < *by1);
            match hit {
                None => break,
                Some((bx0, _, bx1, _)) => {
                    // Step to the nearer side of the box.
                    xm = if xm - bx0 < bx1 - xm { bx0 - 12.0 } else { bx1 + 12.0 };
                }
            }
        }
        xm
    };
    let wire_path = |s: &mut String, a: (f64, f64, bool), b: (f64, f64), lane: f64| {
        // Leave the stub in its facing direction, run vertically (in a
        // per-net lane so parallel nets stay apart), then arrive.
        let dir = if a.2 { 1.0 } else { -1.0 };
        let xm = if (b.0 - a.0) * dir > 40.0 { (a.0 + b.0) / 2.0 } else { a.0 + dir * 30.0 };
        let xm = clear_x(xm + lane, a.1, b.1);
        let _ = write!(
            s,
            "<polyline points=\"{:.1},{:.1} {:.1},{:.1} {:.1},{:.1} {:.1},{:.1}\" fill=\"none\" stroke=\"{WIRE}\" stroke-width=\"1.8\" stroke-linejoin=\"round\"/>",
            a.0, a.1, xm, a.1, xm, b.1, b.0, b.1
        );
        (xm, (a.1 + b.1) / 2.0)
    };
    for (ni, n) in nets.iter().enumerate() {
        if n.style != NetStyle::Wire || n.pins.is_empty() {
            continue;
        }
        let lane = ((ni % 5) as f64 - 2.0) * 9.0;
        let ends: Vec<(f64, f64, bool)> = n.pins.iter().filter_map(|(ci, p)| comps[*ci].pin_point(p)).collect();
        match (n.junction, ends.len()) {
            (_, 0) => {}
            (None, 1) => {
                // Dangling: short stub with the name.
                let e = ends[0];
                let _ = write!(s, "<text x=\"{:.1}\" y=\"{:.1}\" font-size=\"{}\" fill=\"{WIRE}\" text-anchor=\"{}\">{}</text>", e.0 + if e.2 { 4.0 } else { -4.0 }, e.1 + 4.0, FONT * 0.9, if e.2 { "start" } else { "end" }, esc(&n.name));
            }
            (None, _) => {
                let (lx, ly) = wire_path(&mut s, ends[0], (ends[1].0, ends[1].1), lane);
                let _ = write!(s, "<text x=\"{:.1}\" y=\"{:.1}\" font-size=\"{}\" fill=\"{WIRE}\" text-anchor=\"middle\">{}</text>", lx, ly - 5.0, FONT * 0.9, esc(&n.name));
                for e in ends.iter().skip(2) {
                    wire_path(&mut s, *e, (ends[1].0, ends[1].1), lane);
                }
            }
            (Some(j), _) => {
                for e in &ends {
                    wire_path(&mut s, *e, j, lane);
                }
                let _ = write!(s, "<circle cx=\"{:.1}\" cy=\"{:.1}\" r=\"4\" fill=\"{WIRE}\"/>", j.0, j.1);
                let _ = write!(s, "<text x=\"{:.1}\" y=\"{:.1}\" font-size=\"{}\" fill=\"{WIRE}\" text-anchor=\"middle\">{}</text>", j.0, j.1 - 9.0, FONT * 0.9, esc(&n.name));
            }
        }
    }

    // ---- boxes, pins, stubs
    for c in &comps {
        let (bx, by) = (c.cx - c.w / 2.0, c.cy - c.h / 2.0);
        let _ = write!(s, "<rect x=\"{bx:.1}\" y=\"{by:.1}\" width=\"{:.1}\" height=\"{:.1}\" rx=\"4\" fill=\"#fbfbfb\" stroke=\"{INK}\" stroke-width=\"1.5\"/>", c.w, c.h);
        let _ = write!(s, "<text x=\"{:.1}\" y=\"{:.1}\" font-size=\"{}\" font-weight=\"bold\" text-anchor=\"middle\" fill=\"{INK}\">{}</text>", c.cx, by + 14.0, FONT * 1.2, esc(&c.title));
        let _ = write!(s, "<text x=\"{:.1}\" y=\"{:.1}\" font-size=\"{}\" text-anchor=\"middle\" fill=\"#555\">{}</text>", c.cx, by + 27.0, FONT * 0.85, esc(&c.subtitle));
        let _ = write!(s, "<line x1=\"{bx:.1}\" y1=\"{:.1}\" x2=\"{:.1}\" y2=\"{:.1}\" stroke=\"#bbb\" stroke-width=\"1\"/>", by + HEADER, bx + c.w, by + HEADER);
        for p in &c.pins {
            let Some((px, py, right)) = c.pin_point(p) else { continue };
            let edge_x = if right { bx + c.w } else { bx };
            let _ = write!(s, "<line x1=\"{edge_x:.1}\" y1=\"{py:.1}\" x2=\"{px:.1}\" y2=\"{py:.1}\" stroke=\"{INK}\" stroke-width=\"1.5\"/>");
            let _ = write!(s, "<circle cx=\"{px:.1}\" cy=\"{py:.1}\" r=\"2.5\" fill=\"{INK}\"/>");
            let (tx, anchor) = if right { (edge_x - 5.0, "end") } else { (edge_x + 5.0, "start") };
            let _ = write!(s, "<text x=\"{tx:.1}\" y=\"{:.1}\" font-size=\"{FONT}\" text-anchor=\"{anchor}\" fill=\"{INK}\">{}</text>", py + 4.0, esc(p));
        }
    }

    // ---- ground symbols and net-label flags at pins
    for n in &nets {
        match n.style {
            NetStyle::Wire => {}
            NetStyle::Ground => {
                for (ci, p) in &n.pins {
                    let Some((px, py, right)) = comps[*ci].pin_point(p) else { continue };
                    let dir = if right { 1.0 } else { -1.0 };
                    let gx = px + dir * 14.0;
                    let _ = write!(s, "<path d=\"M{px:.1} {py:.1} H{gx:.1} V{:.1} M{:.1} {:.1} H{:.1} M{:.1} {:.1} H{:.1} M{:.1} {:.1} H{:.1}\" fill=\"none\" stroke=\"{WIRE}\" stroke-width=\"1.8\"/>",
                        py + 12.0, gx - 9.0, py + 12.0, gx + 9.0, gx - 6.0, py + 16.0, gx + 6.0, gx - 3.0, py + 20.0, gx + 3.0);
                }
            }
            NetStyle::Label => {
                for (ci, p) in &n.pins {
                    let Some((px, py, right)) = comps[*ci].pin_point(p) else { continue };
                    let w = text_w(&n.name, FONT * 0.9) + 14.0;
                    let (fx, tx) = if right { (px + 4.0, px + 11.0) } else { (px - 4.0 - w, px - 11.0) };
                    let _ = write!(s, "<rect x=\"{fx:.1}\" y=\"{:.1}\" width=\"{w:.1}\" height=\"18\" rx=\"9\" fill=\"#eef3ff\" stroke=\"{WIRE}\" stroke-width=\"1.2\"/>", py - 9.0);
                    let _ = write!(s, "<text x=\"{tx:.1}\" y=\"{:.1}\" font-size=\"{}\" fill=\"{WIRE}\" text-anchor=\"{}\">{}</text>", py + 4.0, FONT * 0.9, if right { "start" } else { "end" }, esc(&n.name));
                }
            }
        }
    }
    s.push_str("</svg>");
    s
}
