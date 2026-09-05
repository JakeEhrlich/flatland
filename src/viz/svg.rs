//! A tiny SVG writer working in millimetres with a y-up board frame.

use crate::geom::Ring;
use crate::units::{fmt_mm, Point};
use std::fmt::Write;

pub struct Svg {
    body: String,
    /// Board-frame y that maps to SVG y = 0 (so y is flipped).
    pub y_top: f64,
    pub mirror_x: bool,
    pub x_right: f64,
    pub width: f64,
    pub height: f64,
    pub background: String,
    defs: String,
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

impl Svg {
    /// Viewport covering board coordinates [x0,x1] x [y0,y1] (mm).
    pub fn new(x0: f64, y0: f64, x1: f64, y1: f64, mirror_x: bool) -> Svg {
        Svg {
            body: String::new(),
            y_top: y1,
            mirror_x,
            x_right: x1,
            width: x1 - x0,
            height: y1 - y0,
            background: "#ffffff".into(),
            defs: String::new(),
        }
        .with_origin(x0)
    }
    fn with_origin(mut self, x0: f64) -> Svg {
        // Store x0 in x_right when mirrored: sx = x_right - x; else sx = x - x0.
        if !self.mirror_x {
            self.x_right = x0;
        }
        self
    }
    pub fn sx(&self, x: f64) -> f64 {
        if self.mirror_x { self.x_right - x } else { x - self.x_right }
    }
    pub fn sy(&self, y: f64) -> f64 {
        self.y_top - y
    }
    pub fn pt(&self, p: Point) -> (f64, f64) {
        (self.sx(p.x.mm()), self.sy(p.y.mm()))
    }
    pub fn raw(&mut self, s: &str) {
        self.body.push_str(s);
        self.body.push('\n');
    }
    pub fn def(&mut self, s: &str) {
        self.defs.push_str(s);
        self.defs.push('\n');
    }
    pub fn group(&mut self, attrs: &str) {
        let _ = writeln!(self.body, "<g {attrs}>");
    }
    pub fn end_group(&mut self) {
        self.body.push_str("</g>\n");
    }

    fn path_d(&self, ring: &[Point], close: bool) -> String {
        let mut d = String::new();
        for (i, p) in ring.iter().enumerate() {
            let (x, y) = self.pt(*p);
            let _ = write!(d, "{}{} {}", if i == 0 { "M" } else { "L" }, fmt_mm(x), fmt_mm(y));
        }
        if close {
            d.push('Z');
        }
        d
    }

    /// Filled polygon set (evenodd so holes render).
    pub fn polygons(&mut self, rings: &[Ring], fill: &str, opacity: f64, extra: &str) {
        if rings.is_empty() {
            return;
        }
        let d: Vec<String> = rings.iter().map(|r| self.path_d(r, true)).collect();
        let stroke = if extra.contains("stroke=") { "" } else { "stroke=\"none\"" };
        let _ = writeln!(
            self.body,
            "<path d=\"{}\" fill=\"{fill}\" fill-opacity=\"{opacity}\" fill-rule=\"evenodd\" {stroke} {extra}/>",
            d.join(" ")
        );
    }
    pub fn polygon(&mut self, ring: &[Point], fill: &str, opacity: f64, extra: &str) {
        self.polygons(&[ring.to_vec()], fill, opacity, extra);
    }
    pub fn polyline(&mut self, pts: &[Point], closed: bool, stroke: &str, width_mm: f64, extra: &str) {
        self.polyline_caps(pts, closed, stroke, width_mm, "round", extra);
    }
    pub fn polyline_caps(&mut self, pts: &[Point], closed: bool, stroke: &str, width_mm: f64, cap: &str, extra: &str) {
        if pts.is_empty() {
            return;
        }
        let d = self.path_d(pts, closed);
        let _ = writeln!(
            self.body,
            "<path d=\"{d}\" fill=\"none\" stroke=\"{stroke}\" stroke-width=\"{}\" stroke-linecap=\"{cap}\" stroke-linejoin=\"round\" {extra}/>",
            fmt_mm(width_mm)
        );
    }
    pub fn line(&mut self, a: Point, b: Point, stroke: &str, width_mm: f64, extra: &str) {
        self.polyline(&[a, b], false, stroke, width_mm, extra);
    }
    pub fn circle(&mut self, c: Point, diameter_mm: f64, fill: &str, stroke: &str, stroke_w: f64, extra: &str) {
        let (x, y) = self.pt(c);
        let _ = writeln!(
            self.body,
            "<circle cx=\"{}\" cy=\"{}\" r=\"{}\" fill=\"{fill}\" stroke=\"{stroke}\" stroke-width=\"{}\" {extra}/>",
            fmt_mm(x),
            fmt_mm(y),
            fmt_mm(diameter_mm / 2.0),
            fmt_mm(stroke_w)
        );
    }
    /// Text in mm units. `anchor` is start/middle/end.
    pub fn text(&mut self, at: Point, s: &str, size_mm: f64, fill: &str, anchor: &str, extra: &str) {
        let (x, y) = self.pt(at);
        let _ = writeln!(
            self.body,
            "<text x=\"{}\" y=\"{}\" font-size=\"{}\" fill=\"{fill}\" text-anchor=\"{anchor}\" font-family=\"Helvetica, Arial, sans-serif\" dominant-baseline=\"middle\" {extra}>{}</text>",
            fmt_mm(x),
            fmt_mm(y),
            fmt_mm(size_mm),
            esc(s)
        );
    }
    /// Text placed in raw SVG coordinates (for rulers/legends).
    pub fn text_raw(&mut self, x: f64, y: f64, s: &str, size_mm: f64, fill: &str, anchor: &str, extra: &str) {
        let _ = writeln!(
            self.body,
            "<text x=\"{}\" y=\"{}\" font-size=\"{}\" fill=\"{fill}\" text-anchor=\"{anchor}\" font-family=\"Helvetica, Arial, sans-serif\" dominant-baseline=\"middle\" {extra}>{}</text>",
            fmt_mm(x),
            fmt_mm(y),
            fmt_mm(size_mm),
            esc(s)
        );
    }
    pub fn rect_raw(&mut self, x: f64, y: f64, w: f64, h: f64, fill: &str, stroke: &str, sw: f64, extra: &str) {
        let _ = writeln!(
            self.body,
            "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"{fill}\" stroke=\"{stroke}\" stroke-width=\"{}\" {extra}/>",
            fmt_mm(x),
            fmt_mm(y),
            fmt_mm(w),
            fmt_mm(h),
            fmt_mm(sw)
        );
    }
    pub fn line_raw(&mut self, x0: f64, y0: f64, x1: f64, y1: f64, stroke: &str, w: f64, extra: &str) {
        let _ = writeln!(
            self.body,
            "<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke=\"{stroke}\" stroke-width=\"{}\" {extra}/>",
            fmt_mm(x0),
            fmt_mm(y0),
            fmt_mm(x1),
            fmt_mm(y1),
            fmt_mm(w)
        );
    }

    /// Finish: the viewBox is the full viewport; `margin` mm are added on
    /// every side (negative coordinates are fine in SVG).
    pub fn finish(&self, margin: f64) -> String {
        let (x, y, w, h) = (-margin, -margin, self.width + 2.0 * margin, self.height + 2.0 * margin);
        format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"{} {} {} {}\" width=\"{}mm\" height=\"{}mm\">\n<defs>\n{}</defs>\n<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"{}\"/>\n{}</svg>\n",
            fmt_mm(x), fmt_mm(y), fmt_mm(w), fmt_mm(h), fmt_mm(w), fmt_mm(h), self.defs,
            fmt_mm(x), fmt_mm(y), fmt_mm(w), fmt_mm(h), self.background, self.body
        )
    }
}
