//! Board renderer: a top-down (or bottom-up) view with rulers, layer legend
//! and airwires, meant to be read by a model as a PNG.

use super::svg::Svg;
use crate::error::Result;
use crate::geom::{self, Ring};
use crate::model::Board;
use crate::schema::{PadType, Side};
use crate::units::{fmt_mm, Length, Point};

pub struct Options {
    pub layers: Option<Vec<String>>,
    pub from_bottom: bool,
    pub ratsnest: bool,
    pub labels: bool,
    pub grid: bool,
}

const BG: &str = "#1c1c1e";
const OUTLINE: &str = "#e8d44d";
const GOLD: &str = "#d9b44a";
const VIA: &str = "#b0b0b0";
const SILK_TOP: &str = "#f2f2f2";
const SILK_BOTTOM: &str = "#e39be3";
const RATSNEST: &str = "#ff6ef0";
const RULER: &str = "#9a9a9a";

pub fn layer_color(board: &Board, layer: &str) -> &'static str {
    let n = board.layers().len();
    let i = board.stackup().layer_index(layer).unwrap_or(0);
    if n == 1 {
        return "#d9534f";
    }
    if i == 0 {
        "#d9534f" // top: red
    } else if i == n - 1 {
        "#4d8ee0" // bottom: blue
    } else {
        ["#4fc26a", "#e0a34d", "#b56de0", "#4dd6d9"][(i - 1) % 4]
    }
}

pub fn render(board: &Board, opts: &Options) -> Result<String> {
    // Extent.
    let (lo, hi) = board.bbox().unwrap_or((Point::mm(0.0, 0.0), Point::mm(50.0, 30.0)));
    let (x0, y0, x1, y1) = (lo.x.mm(), lo.y.mm(), hi.x.mm(), hi.y.mm());
    let extent = (x1 - x0).max(y1 - y0).max(10.0);
    let pad = extent * 0.06 + 2.0;
    let (vx0, vy0, vx1, vy1) = (x0 - pad, y0 - pad, x1 + pad, y1 + pad);
    let mut svg = Svg::new(vx0, vy0, vx1, vy1, opts.from_bottom);
    svg.background = BG.into();
    let text = (extent / 60.0).max(0.7);
    let thin = (extent / 800.0).max(0.04);

    let layers: Vec<String> = board.layers().to_vec();
    let visible = |l: &str| opts.layers.as_ref().map_or(true, |v| v.iter().any(|x| x == l));
    // Draw order: far side first.
    let mut order: Vec<&String> = layers.iter().collect();
    if !opts.from_bottom {
        order.reverse();
    }

    if opts.grid {
        let step = nice_step(extent / 20.0);
        let mut x = (x0 / step).floor() * step;
        while x <= x1 {
            svg.line(Point::mm(x, y0), Point::mm(x, y1), "#3a3a3a", thin, "");
            x += step;
        }
        let mut y = (y0 / step).floor() * step;
        while y <= y1 {
            svg.line(Point::mm(x0, y), Point::mm(x1, y), "#3a3a3a", thin, "");
            y += step;
        }
    }

    // Board substrate.
    if let Some(o) = &board.outline {
        svg.polygon(o, "#2e3b2e", 1.0, "");
    }

    // Pours.
    for l in &order {
        if !visible(l) {
            continue;
        }
        for p in board.pours()? {
            if &p.pour.layer == *l {
                svg.polygon(&p.outline, "none", 0.0, &format!("stroke=\"{}\" stroke-width=\"{}\" stroke-dasharray=\"{} {}\" stroke-opacity=\"0.6\"", layer_color(board, l), fmt_mm(thin), fmt_mm(thin * 8.0), fmt_mm(thin * 4.0)));
                svg.polygons(&p.copper, layer_color(board, l), 0.45, "");
            }
        }
    }
    // Traces.
    for l in &order {
        if !visible(l) {
            continue;
        }
        for t in board.project.traces.iter().filter(|t| &t.layer == *l) {
            let pts: Vec<Point> = t.points.clone();
            svg.polyline_caps(&pts, false, layer_color(board, l), t.width.mm(), "butt", "stroke-opacity=\"0.9\"");
        }
        for t in board.project.texts.iter().filter(|t| &t.layer == *l) {
            svg.polygons(&board.text_copper(t), layer_color(board, l), 0.9, "");
        }
    }
    // Pads, far side first, then through-hole.
    for l in &order {
        if !visible(l) {
            continue;
        }
        for pad in board.all_pads() {
            if pad.pad_type == PadType::Smd && pad.on_layer(l) {
                svg.polygon(&pad.copper, layer_color(board, l), 0.95, "");
            }
        }
    }
    for pad in board.all_pads() {
        if pad.pad_type == PadType::ThroughHole {
            svg.polygon(&pad.copper, GOLD, 0.95, "");
            if let Some(d) = pad.drill_ring() {
                svg.polygon(&d, BG, 1.0, "");
            }
        }
    }
    // Vias & holes.
    for v in &board.project.vias {
        svg.circle(v.at, v.diameter.mm(), VIA, "none", 0.0, "");
        svg.circle(v.at, v.drill.mm(), BG, "none", 0.0, "");
    }
    for h in &board.holes {
        if let Some(c) = &h.copper {
            svg.polygon(c, GOLD, 0.95, "");
        }
        svg.circle(h.hole.at, h.hole.drill.mm(), BG, "#777", thin, "");
    }
    // Silkscreen (clipped away from copper, as it will be fabricated) & courtyards.
    for side in [Side::Bottom, Side::Top] {
        let color = if side == Side::Top { SILK_TOP } else { SILK_BOTTOM };
        svg.polygons(&board.silkscreen(side)?, color, 1.0, "");
    }
    for inst in &board.instances {
        for (pts, closed) in &inst.courtyard {
            svg.polyline(pts, *closed, "#888", thin, &format!("stroke-dasharray=\"{} {}\"", fmt_mm(thin * 4.0), fmt_mm(thin * 3.0)));
        }
    }
    // Pad numbers (only when large enough to read).
    for pad in board.all_pads() {
        let (w, h) = (pad.size[0].mm(), pad.size[1].mm());
        let sz = (w.min(h) * 0.6).min(text * 0.8);
        if sz >= text * 0.35 {
            let label = pad.pins.first().cloned().unwrap_or_else(|| pad.pad_name.clone());
            svg.text(pad.center, &label, sz, "#111", "middle", "");
        }
    }
    // Reference designators.
    if opts.labels {
        for inst in &board.instances {
            if let Some(at) = inst.label_at {
                let side = inst.side().unwrap_or(Side::Top);
                let color = if side == Side::Top { "#ffffff" } else { SILK_BOTTOM };
                // The refdes itself is on the silkscreen; add the value/note beneath it.
                if let Some(v) = inst.instance.note.as_deref().or_else(|| inst.instance.parameters.get("value").map(|s| s.as_str())) {
                    let below = Point::new(at.x, at.y - inst.label_size(board.rules()));
                    svg.text(below, v, text * 0.8, color, "middle", "");
                }
            }
        }
    }
    // Ratsnest.
    if opts.ratsnest {
        for (_, a, b) in board.ratsnest()? {
            svg.line(a, b, RATSNEST, thin * 1.5, &format!("stroke-dasharray=\"{} {}\" stroke-opacity=\"0.9\"", fmt_mm(thin * 6.0), fmt_mm(thin * 4.0)));
        }
    }
    // Outline.
    if let Some(o) = &board.outline {
        svg.polygon(o, "none", 0.0, &format!("stroke=\"{OUTLINE}\" stroke-width=\"{}\"", fmt_mm(thin * 2.5)));
    } else {
        svg.text_raw(svg.width / 2.0, 1.0, "no board outline", text, OUTLINE, "middle", "");
    }

    // Rulers along the bottom and left edges of the viewport.
    draw_rulers(&mut svg, x0, y0, x1, y1, text, thin, opts.from_bottom);
    // Legend & title.
    draw_legend(&mut svg, board, &layers, text, thin, opts);

    Ok(svg.finish(0.0))
}

fn nice_step(raw: f64) -> f64 {
    let p = 10f64.powf(raw.log10().floor());
    let m = raw / p;
    let n = if m < 1.5 { 1.0 } else if m < 3.5 { 2.0 } else if m < 7.5 { 5.0 } else { 10.0 };
    n * p
}

fn draw_rulers(svg: &mut Svg, x0: f64, y0: f64, x1: f64, y1: f64, text: f64, thin: f64, mirrored: bool) {
    let step = nice_step((x1 - x0).max(y1 - y0) / 12.0);
    let ts = text * 0.75;
    // Baselines just outside the content bbox.
    let yb = svg.sy(y0) + text * 0.9;
    let xl = svg.sx(if mirrored { x1 } else { x0 }) - text * 0.9;
    let mut x = (x0 / step).ceil() * step;
    while x <= x1 + 1e-9 {
        let sx = svg.sx(x);
        svg.line_raw(sx, yb - text * 0.3, sx, yb, RULER, thin, "");
        svg.text_raw(sx, yb + ts * 0.8, &fmt_mm(x), ts, RULER, "middle", "");
        x += step;
    }
    let mut y = (y0 / step).ceil() * step;
    while y <= y1 + 1e-9 {
        let sy = svg.sy(y);
        svg.line_raw(xl, sy, xl + text * 0.3, sy, RULER, thin, "");
        svg.text_raw(xl - ts * 0.3, sy, &fmt_mm(y), ts, RULER, "end", "");
        y += step;
    }
    svg.text_raw(svg.sx(x1) + text * 0.5, yb + ts * 0.8, "mm", ts, RULER, "start", "");
}

fn draw_legend(svg: &mut Svg, board: &Board, layers: &[String], text: f64, thin: f64, opts: &Options) {
    let ts = text * 0.8;
    let mut x = 0.5;
    let y = 0.3 + ts * 0.6;
    let title = format!(
        "{} — {} view",
        board.project.name,
        if opts.from_bottom { "bottom (mirrored)" } else { "top" }
    );
    svg.text_raw(x, y, &title, ts, "#ffffff", "start", "font-weight=\"bold\"");
    x += title.len() as f64 * ts * 0.6 + ts;
    for l in layers {
        let c = layer_color(board, l);
        svg.rect_raw(x, y - ts * 0.4, ts * 0.8, ts * 0.8, c, "none", 0.0, "");
        svg.text_raw(x + ts, y, l, ts, "#dddddd", "start", "");
        x += ts * 1.2 + l.len() as f64 * ts * 0.6 + ts * 0.8;
    }
    let items = [(GOLD, "through-hole"), (VIA, "via"), (RATSNEST, "unrouted")];
    for (c, name) in items {
        svg.rect_raw(x, y - ts * 0.4, ts * 0.8, ts * 0.8, c, "none", 0.0, "");
        svg.text_raw(x + ts, y, name, ts, "#dddddd", "start", "");
        x += ts * 1.2 + name.len() as f64 * ts * 0.6 + ts * 0.8;
    }
    let _ = thin;
    // Unplaced parts note.
    let unplaced: Vec<&str> = board.instances.iter().filter(|i| !i.is_placed() && !i.component.is_virtual()).map(|i| i.refdes.as_str()).collect();
    if !unplaced.is_empty() {
        svg.text_raw(0.5, y + ts * 1.4, &format!("unplaced: {}", unplaced.join(", ")), ts, RATSNEST, "start", "");
    }
}

#[allow(dead_code)]
fn ring_len(r: &Ring) -> Length {
    Length::from_mm(geom::signed_area(r).abs())
}
