//! Line plots as SVG (rasterised by the shared SVG→PNG pipeline).

use super::ngspice::Vector;
use std::fmt::Write;

const COLORS: &[&str] = &["#d9534f", "#337ab7", "#5cb85c", "#f0ad4e", "#9b59b6", "#1abc9c", "#e67e22", "#34495e"];

fn nice_ticks(lo: f64, hi: f64, n: usize) -> Vec<f64> {
    if !(hi > lo) {
        return vec![lo];
    }
    let raw = (hi - lo) / n as f64;
    let p = 10f64.powf(raw.log10().floor());
    let m = raw / p;
    let step = if m < 1.5 { 1.0 } else if m < 3.5 { 2.0 } else if m < 7.5 { 5.0 } else { 10.0 } * p;
    let mut t = (lo / step).ceil() * step;
    let mut out = Vec::new();
    while t <= hi + step * 1e-9 {
        out.push(t);
        t += step;
    }
    out
}

/// Pick an SI prefix so the largest magnitude reads as 1..1000.
fn si_prefix(max_abs: f64) -> (f64, &'static str) {
    if !(max_abs > 0.0) || !max_abs.is_finite() {
        return (1.0, "");
    }
    let table: [(f64, &str); 9] = [(1e9, "G"), (1e6, "M"), (1e3, "k"), (1.0, ""), (1e-3, "m"), (1e-6, "µ"), (1e-9, "n"), (1e-12, "p"), (1e-15, "f")];
    for (scale, p) in table {
        if max_abs >= scale {
            return (scale, p);
        }
    }
    (1e-15, "f")
}

fn axis_label(name: &str, unit: &str, prefix: &str) -> String {
    if unit.is_empty() {
        name.to_string()
    } else {
        format!("{name} ({prefix}{unit})")
    }
}

fn fmt(v: f64) -> String {
    if v == 0.0 {
        return "0".into();
    }
    let a = v.abs();
    if a >= 1e5 || a < 1e-3 {
        format!("{v:.2e}")
    } else {
        let s = format!("{v:.4}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

/// Series whose magnitudes differ wildly get their own panels so each
/// remains readable; otherwise everything shares one set of axes.
pub fn plot(title: &str, x: &Vector, series: &[&Vector], log_x: bool, magnitude_db: bool) -> String {
    let groups = group_series(series, magnitude_db);
    let panel_h = if groups.len() == 1 { 700.0 } else { 380.0 };
    let w = 1200.0;
    let h = 40.0 + panel_h * groups.len() as f64;
    let mut s = String::new();
    let _ = write!(s, "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {w} {h}\" width=\"{w}\" height=\"{h}\" font-family=\"Helvetica, Arial, sans-serif\">");
    s.push_str("<rect width=\"100%\" height=\"100%\" fill=\"white\"/>");
    let _ = write!(s, "<text x=\"{}\" y=\"28\" font-size=\"18\" text-anchor=\"middle\" font-weight=\"bold\">{}</text>", w / 2.0, esc(title));
    let mut color_index = 0;
    for (gi, g) in groups.iter().enumerate() {
        let top = 40.0 + gi as f64 * panel_h;
        panel(&mut s, x, g, log_x, magnitude_db, 0.0, top, w, panel_h, &mut color_index);
    }
    s.push_str("</svg>");
    s
}

fn range_of(v: &Vector, magnitude_db: bool) -> f64 {
    let vals: Vec<f64> = if v.is_complex {
        v.values.iter().zip(v.imag.iter()).map(|(r, i)| { let m = (r * r + i * i).sqrt(); if magnitude_db { 20.0 * m.max(1e-30).log10() } else { m } }).collect()
    } else {
        v.values.clone()
    };
    vals.iter().cloned().filter(|x| x.is_finite()).fold(0.0f64, |a, b| a.max(b.abs()))
}

fn group_series<'a>(series: &[&'a Vector], magnitude_db: bool) -> Vec<Vec<&'a Vector>> {
    // A panel holds one unit; within a unit, series of similar magnitude share axes.
    let mut groups: Vec<(String, f64, Vec<&'a Vector>)> = Vec::new();
    for v in series {
        let r = range_of(v, magnitude_db);
        let unit = if v.is_complex && magnitude_db { "dB".to_string() } else { v.unit.clone() };
        match groups.iter_mut().find(|(gu, gr, _)| *gu == unit && { let (a, b) = (gr.max(1e-30), r.max(1e-30)); (a / b).max(b / a) < 20.0 }) {
            Some((_, _, g)) => g.push(v),
            None => groups.push((unit, r, vec![v])),
        }
    }
    if groups.len() > 6 {
        // Too many panels: fall back to one.
        return vec![series.to_vec()];
    }
    groups.into_iter().map(|(_, _, g)| g).collect()
}

fn panel(s: &mut String, x: &Vector, series: &[&Vector], log_x: bool, magnitude_db: bool, x0: f64, y0: f64, w: f64, h: f64, color_index: &mut usize) {
    let (ml, mr, mt, mb) = (90.0, 30.0, 20.0, 60.0);
    let (pw, ph) = (w - ml - mr, h - mt - mb);
    let (ml, mt) = (x0 + ml, y0 + mt);
    let (xscale, xprefix) = if log_x { (1.0, "") } else { si_prefix(x.values.iter().cloned().fold(0.0f64, |a, b| a.max(b.abs()))) };
    let xs: Vec<f64> = x.values.iter().map(|v| if log_x { v.max(1e-30).log10() } else { *v / xscale }).collect();
    let ys_raw: Vec<Vec<f64>> = series
        .iter()
        .map(|s| {
            if s.is_complex {
                s.values
                    .iter()
                    .zip(s.imag.iter())
                    .map(|(r, i)| {
                        let m = (r * r + i * i).sqrt();
                        if magnitude_db { 20.0 * m.max(1e-30).log10() } else { m }
                    })
                    .collect()
            } else {
                s.values.clone()
            }
        })
        .collect();
    // One unit per panel (series are grouped by magnitude, so mixed units are rare);
    // scale the axis by a common SI prefix.
    let units: Vec<&str> = series.iter().map(|v| if v.is_complex && magnitude_db { "dB" } else { v.unit.as_str() }).collect();
    let unit = if units.iter().all(|u| *u == units[0]) { units[0].to_string() } else { units.iter().collect::<std::collections::BTreeSet<_>>().iter().map(|u| u.to_string()).collect::<Vec<_>>().join(" / ") };
    let ymax_abs = ys_raw.iter().flatten().cloned().filter(|v| v.is_finite()).fold(0.0f64, |a, b| a.max(b.abs()));
    let (yscale, yprefix) = if unit == "dB" || unit == "°C" || unit == "°" || unit.is_empty() { (1.0, "") } else { si_prefix(ymax_abs) };
    let ys: Vec<Vec<f64>> = ys_raw.iter().map(|v| v.iter().map(|y| y / yscale).collect()).collect();
    let xlo = xs.iter().cloned().fold(f64::INFINITY, f64::min);
    let xhi = xs.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let mut ylo = ys.iter().flatten().cloned().filter(|v| v.is_finite()).fold(f64::INFINITY, f64::min);
    let mut yhi = ys.iter().flatten().cloned().filter(|v| v.is_finite()).fold(f64::NEG_INFINITY, f64::max);
    if !(yhi > ylo) {
        ylo -= 1.0;
        yhi += 1.0;
    }
    let pad = (yhi - ylo) * 0.05;
    ylo -= pad;
    yhi += pad;
    let sx = |v: f64| ml + (v - xlo) / (xhi - xlo).max(1e-300) * pw;
    let sy = |v: f64| mt + ph - (v - ylo) / (yhi - ylo) * ph;

    // Grid & ticks
    let xt = if log_x { (xlo.floor() as i64..=xhi.ceil() as i64).map(|e| e as f64).collect() } else { nice_ticks(xlo, xhi, 8) };
    for t in &xt {
        if *t < xlo || *t > xhi {
            continue;
        }
        let px = sx(*t);
        let _ = write!(s, "<line x1=\"{px}\" y1=\"{mt}\" x2=\"{px}\" y2=\"{}\" stroke=\"#e0e0e0\"/>", mt + ph);
        let label = if log_x { fmt(10f64.powf(*t)) } else { fmt(*t) };
        let _ = write!(s, "<text x=\"{px}\" y=\"{}\" font-size=\"12\" text-anchor=\"middle\">{label}</text>", mt + ph + 18.0);
    }
    for t in nice_ticks(ylo, yhi, 8) {
        let py = sy(t);
        let _ = write!(s, "<line x1=\"{ml}\" y1=\"{py}\" x2=\"{}\" y2=\"{py}\" stroke=\"#e0e0e0\"/>", ml + pw);
        let _ = write!(s, "<text x=\"{}\" y=\"{}\" font-size=\"12\" text-anchor=\"end\">{}</text>", ml - 6.0, py + 4.0, fmt(t));
    }
    let _ = write!(s, "<rect x=\"{ml}\" y=\"{mt}\" width=\"{pw}\" height=\"{ph}\" fill=\"none\" stroke=\"#333\"/>");
    let xlabel = if log_x { format!("{} ({}, log)", esc(&x.name), if x.unit.is_empty() { "log" } else { &x.unit }) } else { esc(&axis_label(&x.name, &x.unit, xprefix)) };
    let _ = write!(s, "<text x=\"{}\" y=\"{}\" font-size=\"13\" text-anchor=\"middle\">{}</text>", ml + pw / 2.0, mt + ph + 40.0, xlabel);
    let ylabel = if series.len() == 1 { axis_label(&series[0].name, &unit, yprefix) } else if unit.is_empty() { String::new() } else { format!("{yprefix}{unit}") };
    if !ylabel.is_empty() {
        let _ = write!(s, "<text x=\"{}\" y=\"{}\" font-size=\"13\" text-anchor=\"middle\" transform=\"rotate(-90 {} {})\">{}</text>", x0 + 22.0, mt + ph / 2.0, x0 + 22.0, mt + ph / 2.0, esc(&ylabel));
    }
    // Series
    for (i, (v, y)) in series.iter().zip(ys.iter()).enumerate() {
        let color = COLORS[*color_index % COLORS.len()];
        *color_index += 1;
        let mut d = String::new();
        let mut first = true;
        for (px, py) in xs.iter().zip(y.iter()) {
            if !py.is_finite() {
                first = true;
                continue;
            }
            let _ = write!(d, "{}{:.2} {:.2} ", if first { "M" } else { "L" }, sx(*px), sy(*py));
            first = false;
        }
        let _ = write!(s, "<path d=\"{d}\" fill=\"none\" stroke=\"{color}\" stroke-width=\"2\"/>");
        let ly = mt + 16.0 + i as f64 * 18.0;
        let _ = write!(s, "<rect x=\"{}\" y=\"{}\" width=\"14\" height=\"4\" fill=\"{color}\"/>", ml + pw - 170.0, ly - 2.0);
        let legend = if v.unit.is_empty() && !(v.is_complex && magnitude_db) { v.name.clone() } else { format!("{} ({})", v.name, if v.is_complex && magnitude_db { "dB" } else { &v.unit }) };
        let _ = write!(s, "<text x=\"{}\" y=\"{}\" font-size=\"12\">{}</text>", ml + pw - 150.0, ly + 3.0, esc(&legend));
    }
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}
