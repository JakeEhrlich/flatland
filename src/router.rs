//! Pin-to-pin routing: one trace between two pins, fast enough to run inside
//! an agent's edit loop. A* over a rasterised cost field per copper layer;
//! same-net copper is free (and is where the route starts and ends), other
//! copper is an obstacle inflated by clearance and half the trace width; the
//! cell path is pulled into 0/45/90° segments and verified against real
//! geometry. Waypoints steer the route; a blocked route reports where it got
//! closest and what stood in the way.

use crate::error::{Error, Result};
use crate::geom::{self, Ring, Rings};
use crate::model::{Board, BoardPad};
use crate::schema::*;
use crate::units::{Length, Point};
use std::collections::BinaryHeap;

pub struct Request {
    pub from: String,
    pub to: String,
    pub layer: Option<String>,
    pub width: Option<Length>,
    pub waypoints: Vec<Point>,
    /// Cost of a via in trace-length equivalents (mm).
    pub via_cost_mm: f64,
}

pub enum Outcome {
    Routed { traces: Vec<Trace>, vias: Vec<Via>, length_mm: f64 },
    Blocked { layer: String, closest: Point, distance_mm: f64, blockers: Vec<String> },
}

// ---------------------------------------------------------------------------
// Grid

struct Layer {
    name: String,
    /// Obstacle owner per cell (0 = none), before inflation.
    owner: Vec<u16>,
    /// Distance to the nearest obstacle cell, in cells.
    dist: Vec<f32>,
    /// Cells the route may occupy.
    free: Vec<bool>,
    /// Cells where a via may sit (all layers clear).
    via_ok: Vec<bool>,
}

struct Grid {
    w: usize,
    h: usize,
    cell: Length,
    origin: Point,
    layers: Vec<Layer>,
    names: Vec<String>,
    /// Obstacle inflation radius in cells (clearance + half width).
    r_cells: f32,
}

impl Grid {
    fn idx(&self, x: usize, y: usize) -> usize {
        y * self.w + x
    }
    fn cell_of(&self, p: Point) -> Option<(usize, usize)> {
        let x = (p.x.nm() - self.origin.x.nm()) as f64 / self.cell.nm() as f64;
        let y = (p.y.nm() - self.origin.y.nm()) as f64 / self.cell.nm() as f64;
        if x < 0.0 || y < 0.0 {
            return None;
        }
        let (x, y) = (x.floor() as usize, y.floor() as usize);
        if x >= self.w || y >= self.h {
            return None;
        }
        Some((x, y))
    }
    fn centre(&self, x: usize, y: usize) -> Point {
        Point::nm(self.origin.x.nm() + (x as i64 * 2 + 1) * self.cell.nm() / 2, self.origin.y.nm() + (y as i64 * 2 + 1) * self.cell.nm() / 2)
    }
}

/// Mark every cell whose centre lies inside `ring` (scanline, even-odd).
fn rasterise(grid: &Grid, ring: &Ring, mark: &mut dyn FnMut(usize)) {
    if ring.len() < 3 {
        return;
    }
    let Some((lo, hi)) = geom::bounds(&[ring.clone()]) else { return };
    let (Some((x0, y0)), Some((x1, y1))) = (grid.cell_of(lo).or(Some((0, 0))), grid.cell_of(hi).or(Some((grid.w - 1, grid.h - 1)))) else { return };
    let (x0, x1) = (x0.min(grid.w - 1), x1.min(grid.w - 1));
    let (y0, y1) = (y0.min(grid.h - 1), y1.min(grid.h - 1));
    let n = ring.len();
    let mut xs: Vec<f64> = Vec::new();
    for y in y0..=y1 {
        let cy = grid.centre(0, y).y.nm() as f64;
        xs.clear();
        for i in 0..n {
            let (a, b) = (ring[i], ring[(i + 1) % n]);
            let (ay, by) = (a.y.nm() as f64, b.y.nm() as f64);
            if (ay <= cy) != (by <= cy) {
                let t = (cy - ay) / (by - ay);
                xs.push(a.x.nm() as f64 + t * (b.x.nm() - a.x.nm()) as f64);
            }
        }
        if xs.len() < 2 {
            continue;
        }
        xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
        for pair in xs.chunks(2) {
            if pair.len() < 2 {
                break;
            }
            let cell = grid.cell.nm() as f64;
            let ox = grid.origin.x.nm() as f64;
            let xa = ((pair[0] - ox) / cell - 0.5).ceil().max(x0 as f64) as usize;
            let xb = ((pair[1] - ox) / cell - 0.5).floor().min(x1 as f64);
            if xb < 0.0 {
                continue;
            }
            for x in xa..=(xb as usize) {
                mark(grid.idx(x, y));
            }
        }
    }
}

/// Squared Euclidean distance transform (Felzenszwalb), in cells.
fn distance_transform(w: usize, h: usize, obstacle: &[bool]) -> Vec<f32> {
    const INF: f64 = 1e12;
    let mut f: Vec<f64> = obstacle.iter().map(|&o| if o { 0.0 } else { INF }).collect();
    let mut tmp = vec![0.0f64; w.max(h)];
    let mut v = vec![0usize; w.max(h)];
    let mut z = vec![0.0f64; w.max(h) + 1];
    let dt1 = |f: &mut [f64], n: usize, stride: usize, tmp: &mut [f64], v: &mut [usize], z: &mut [f64]| {
        // 1-D squared distance transform of f[0..n] with the given stride.
        let g = |i: usize| f[i * stride];
        let mut k = 0usize;
        v[0] = 0;
        z[0] = -INF;
        z[1] = INF;
        for q in 1..n {
            let mut s;
            loop {
                let vk = v[k];
                s = ((g(q) + (q * q) as f64) - (g(vk) + (vk * vk) as f64)) / (2.0 * (q as f64 - vk as f64));
                if s <= z[k] && k > 0 {
                    k -= 1;
                } else {
                    break;
                }
            }
            k += 1;
            v[k] = q;
            z[k] = s;
            z[k + 1] = INF;
        }
        k = 0;
        for q in 0..n {
            while z[k + 1] < q as f64 {
                k += 1;
            }
            let vk = v[k];
            tmp[q] = (q as f64 - vk as f64).powi(2) + g(vk);
        }
        for q in 0..n {
            f[q * stride] = tmp[q];
        }
    };
    for y in 0..h {
        dt1(&mut f[y * w..(y + 1) * w], w, 1, &mut tmp, &mut v, &mut z);
    }
    for x in 0..w {
        dt1(&mut f[x..], h, w, &mut tmp, &mut v, &mut z);
    }
    f.iter().map(|d| (d.min(INF)).sqrt() as f32).collect()
}

struct Target {
    /// Island geometry per layer name.
    per_layer: Vec<(String, Rings)>,
}

/// Same-net copper reachable from a pad: unions per layer, joined by vias
/// and through-hole pads.
fn island(board: &Board, pad: &BoardPad, net: &str) -> Result<Target> {
    let layers = board.layers().to_vec();
    // Union of the net's copper per layer (pads, traces, vias).
    let mut per: Vec<(String, Vec<Ring>)> = Vec::new();
    for l in &layers {
        let rings: Rings = board.copper_on_layer(l).into_iter().filter(|(n, _)| n.as_deref() == Some(net)).map(|(_, r)| r).collect();
        let u = if rings.is_empty() { vec![] } else { geom::union(&rings)? };
        per.push((l.clone(), u.into_iter().filter(|r| geom::signed_area(r) > 0.0).collect()));
    }
    // Seeds: the pad's centre on each of its layers; grow across vias/through-holes.
    let mut chosen: Vec<(usize, usize)> = Vec::new(); // (layer idx, ring idx)
    let mut seeds: Vec<(usize, Point)> = layers.iter().enumerate().filter(|(_, l)| pad.on_layer(l)).map(|(i, _)| (i, pad.center)).collect();
    let mut guard = 0;
    while let Some((li, p)) = seeds.pop() {
        guard += 1;
        if guard > 10_000 {
            break;
        }
        let Some(ri) = per[li].1.iter().position(|r| geom::contains(r, p)) else { continue };
        if chosen.contains(&(li, ri)) {
            continue;
        }
        chosen.push((li, ri));
        // Anything crossing layers inside this ring seeds the others.
        let ring = per[li].1[ri].clone();
        for v in board.project.vias.iter().filter(|v| v.net.as_deref() == Some(net)) {
            if geom::contains(&ring, v.at) {
                for (lj, l) in layers.iter().enumerate() {
                    if lj != li && (v.layers.is_empty() || v.layers.contains(l)) {
                        seeds.push((lj, v.at));
                    }
                }
            }
        }
        for tp in board.all_pads().filter(|p| p.net.as_deref() == Some(net) && p.pad_type == PadType::ThroughHole) {
            if geom::contains(&ring, tp.center) {
                for lj in 0..layers.len() {
                    if lj != li {
                        seeds.push((lj, tp.center));
                    }
                }
            }
        }
    }
    let mut out = Vec::new();
    for (li, l) in layers.iter().enumerate() {
        let rings: Rings = chosen.iter().filter(|(a, _)| *a == li).map(|(_, ri)| per[li].1[*ri].clone()).collect();
        if !rings.is_empty() {
            out.push((l.clone(), rings));
        }
    }
    Ok(Target { per_layer: out })
}

fn build_grid(board: &Board, net: &str, width: Length, clearance: Length, layers_wanted: &[String], via_d: Length) -> Result<Grid> {
    let rules = board.rules();
    let outline = board.outline.clone().ok_or_else(|| Error::with_help("no board outline", "routing needs one: `pcb outline rect W H`"))?;
    let (lo, hi) = geom::bounds(&[outline.clone()]).unwrap();
    let cell = Length::from_nm((width.min(clearance).nm() / 2).clamp(50_000, 250_000));
    let margin = width + clearance;
    let origin = Point::nm(lo.x.nm() - margin.nm(), lo.y.nm() - margin.nm());
    let w = ((hi.x.nm() - lo.x.nm() + 2 * margin.nm()) / cell.nm()) as usize + 1;
    let h = ((hi.y.nm() - lo.y.nm() + 2 * margin.nm()) / cell.nm()) as usize + 1;
    let mut grid = Grid { w, h, cell, origin, layers: vec![], names: vec!["board edge".into()], r_cells: 0.0 };
    // The keep-in: inside the outline shrunk by whatever `edge_clearance` exceeds `clearance`
    // (the common inflation below adds `clearance` back).
    let extra_edge = Length::from_nm((rules.edge_clearance.nm() - clearance.nm()).max(0));
    let keepin: Rings = if extra_edge.nm() > 0 { geom::offset(&[outline.clone()], -extra_edge) } else { vec![outline.clone()] };
    let r_cells = (clearance.nm() + width.nm() / 2) as f32 / cell.nm() as f32 + 1.0;
    grid.r_cells = r_cells;
    let via_r_cells = (clearance.nm() + via_d.nm() / 2) as f32 / cell.nm() as f32 + 1.0;
    let mut obstacle_maps: Vec<Vec<bool>> = Vec::new();
    for l in layers_wanted {
        let mut owner = vec![0u16; w * h];
        let mut inside = vec![false; w * h];
        for r in &keepin {
            if geom::signed_area(r) > 0.0 {
                rasterise(&grid, r, &mut |i| inside[i] = true);
            }
        }
        for r in &keepin {
            if geom::signed_area(r) <= 0.0 {
                rasterise(&grid, r, &mut |i| inside[i] = false);
            }
        }
        for i in 0..w * h {
            if !inside[i] {
                owner[i] = 1; // board edge
            }
        }
        // Other-net copper (pads, traces, vias, plated rings, copper text).
        for pad in board.all_pads().filter(|p| p.on_layer(l) && p.net.as_deref() != Some(net)) {
            grid.names.push(format!("{}.{} ({})", pad.refdes, pad.pad_name, pad.net.as_deref().unwrap_or("no net")));
            let id = (grid.names.len() - 1) as u16;
            rasterise(&grid, &pad.copper, &mut |i| owner[i] = id);
        }
        for t in board.project.traces.iter().filter(|t| &t.layer == l && t.net.as_deref() != Some(net)) {
            grid.names.push(format!("trace {}", t.net.as_deref().unwrap_or("no net")));
            let id = (grid.names.len() - 1) as u16;
            for r in geom::stroke_flat(&t.points, t.width) {
                rasterise(&grid, &r, &mut |i| owner[i] = id);
            }
        }
        for v in board.project.vias.iter().filter(|v| v.net.as_deref() != Some(net) && (v.layers.is_empty() || v.layers.contains(l))) {
            grid.names.push(format!("via {}", v.net.as_deref().unwrap_or("no net")));
            let id = (grid.names.len() - 1) as u16;
            rasterise(&grid, &geom::circle(v.at, v.diameter), &mut |i| owner[i] = id);
        }
        for hh in &board.holes {
            if hh.hole.net.as_deref() != Some(net) {
                grid.names.push(format!("hole at {}", hh.hole.at));
                let id = (grid.names.len() - 1) as u16;
                let ring = hh.copper.clone().unwrap_or_else(|| hh.ring.clone());
                rasterise(&grid, &ring, &mut |i| owner[i] = id);
            }
        }
        for pad in board.all_pads().filter(|p| p.drill.is_some() && (p.net.as_deref() != Some(net) || !p.plated)) {
            if let Some(d) = pad.drill_ring() {
                grid.names.push(format!("hole of {}.{}", pad.refdes, pad.pad_name));
                let id = (grid.names.len() - 1) as u16;
                rasterise(&grid, &d, &mut |i| owner[i] = id);
            }
        }
        for t in board.project.texts.iter().filter(|t| &t.layer == l) {
            grid.names.push(format!("text \"{}\"", t.text));
            let id = (grid.names.len() - 1) as u16;
            for r in board.text_copper(t) {
                rasterise(&grid, &r, &mut |i| owner[i] = id);
            }
        }
        let obstacle: Vec<bool> = owner.iter().map(|o| *o != 0).collect();
        let dist = distance_transform(w, h, &obstacle);
        let free: Vec<bool> = dist.iter().map(|d| *d >= r_cells).collect();
        obstacle_maps.push(obstacle);
        grid.layers.push(Layer { name: l.clone(), owner, dist, free, via_ok: vec![] });
    }
    let n_layers = grid.layers.len();
    let via_ok: Vec<bool> = (0..w * h).map(|i| n_layers > 1 && grid.layers.iter().all(|l| l.dist[i] >= via_r_cells)).collect();
    for l in &mut grid.layers {
        l.via_ok = via_ok.clone();
    }
    Ok(grid)
}

// ---------------------------------------------------------------------------
// Search

#[derive(Copy, Clone, Eq, PartialEq)]
struct Node {
    f: i64,
    idx: u32,
}
impl Ord for Node {
    fn cmp(&self, o: &Self) -> std::cmp::Ordering {
        o.f.cmp(&self.f).then_with(|| o.idx.cmp(&self.idx))
    }
}
impl PartialOrd for Node {
    fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(o))
    }
}

const DIRS: [(i32, i32, i64); 8] = [(1, 0, 10), (-1, 0, 10), (0, 1, 10), (0, -1, 10), (1, 1, 14), (-1, 1, 14), (1, -1, 14), (-1, -1, 14)];

struct Search<'a> {
    grid: &'a Grid,
    via_cost: i64,
}

struct Found {
    /// (layer, x, y) from start to goal.
    path: Vec<(usize, usize, usize)>,
}

struct Miss {
    layer: usize,
    x: usize,
    y: usize,
    h_cells: f64,
}

impl<'a> Search<'a> {
    /// Multi-source A*; goals are a cell set (or a bbox heuristic to it).
    fn run(&self, starts: &[(usize, usize, usize)], goal: &[bool], goal_bbox: (usize, usize, usize, usize)) -> std::result::Result<Found, Miss> {
        let g = self.grid;
        let (w, h) = (g.w, g.h);
        let per = w * h;
        let n = per * g.layers.len();
        let mut best = vec![i64::MAX; n];
        let mut parent = vec![u32::MAX; n];
        let mut heap = BinaryHeap::new();
        let heur = |x: usize, y: usize| -> i64 {
            let (bx0, by0, bx1, by1) = goal_bbox;
            let dx = if x < bx0 { bx0 - x } else if x > bx1 { x - bx1 } else { 0 } as i64;
            let dy = if y < by0 { by0 - y } else if y > by1 { y - by1 } else { 0 } as i64;
            let (mn, mx) = (dx.min(dy), dx.max(dy));
            14 * mn + 10 * (mx - mn)
        };
        for &(l, x, y) in starts {
            let i = l * per + y * w + x;
            best[i] = 0;
            heap.push(Node { f: heur(x, y), idx: i as u32 });
        }
        let mut closest: Option<(usize, i64)> = None;
        while let Some(Node { f, idx }) = heap.pop() {
            let i = idx as usize;
            let gi = best[i];
            if f - gi > heur(i % per % w, (i % per) / w) {
                // stale entry
                continue;
            }
            let (l, rem) = (i / per, i % per);
            let (x, y) = (rem % w, rem / w);
            if goal[i] {
                let mut path = vec![(l, x, y)];
                let mut cur = i;
                while parent[cur] != u32::MAX {
                    cur = parent[cur] as usize;
                    let (cl, cr) = (cur / per, cur % per);
                    path.push((cl, cr % w, cr / w));
                }
                path.reverse();
                return Ok(Found { path });
            }
            let hc = heur(x, y);
            if closest.map_or(true, |(_, bh)| hc < bh) {
                closest = Some((i, hc));
            }
            let layer = &g.layers[l];
            // Parent direction, for the bend penalty.
            let pdir = if parent[i] != u32::MAX {
                let p = parent[i] as usize;
                if p / per == l {
                    let pr = p % per;
                    Some(((x as i32) - (pr % w) as i32, (y as i32) - (pr / w) as i32))
                } else {
                    None
                }
            } else {
                None
            };
            for &(dx, dy, step) in &DIRS {
                let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                if nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 {
                    continue;
                }
                let (nx, ny) = (nx as usize, ny as usize);
                let ni_rem = ny * w + nx;
                if !layer.free[ni_rem] {
                    continue;
                }
                // Diagonal moves must not cut a corner between two blocked cells.
                if dx != 0 && dy != 0 && (!layer.free[y * w + nx] || !layer.free[ny * w + x]) {
                    continue;
                }
                let mut cost = step;
                if let Some((px, py)) = pdir {
                    if (px, py) != (dx, dy) {
                        cost += 6;
                    }
                }
                let d = layer.dist[ni_rem];
                let r = (g.layers[l].free.len() as f32).min(0.0); // placeholder to keep types simple
                let _ = r;
                cost += proximity_penalty(d, self.grid);
                let ni = l * per + ni_rem;
                let ng = gi + cost;
                if ng < best[ni] {
                    best[ni] = ng;
                    parent[ni] = i as u32;
                    heap.push(Node { f: ng + heur(nx, ny), idx: ni as u32 });
                }
            }
            // Via: same cell, other layers.
            if g.layers.len() > 1 && layer.via_ok[rem] {
                for l2 in 0..g.layers.len() {
                    if l2 == l || !g.layers[l2].free[rem] {
                        continue;
                    }
                    let ni = l2 * per + rem;
                    let ng = gi + self.via_cost;
                    if ng < best[ni] {
                        best[ni] = ng;
                        parent[ni] = i as u32;
                        heap.push(Node { f: ng + hc, idx: ni as u32 });
                    }
                }
            }
        }
        let (ci, ch) = closest.unwrap_or((starts.first().map(|&(l, x, y)| l * per + y * w + x).unwrap_or(0), 0));
        let (l, rem) = (ci / per, ci % per);
        Err(Miss { layer: l, x: rem % w, y: rem / w, h_cells: ch as f64 / 10.0 })
    }
}

/// Small extra cost for running close to obstacles: leaves room for the next trace.
fn proximity_penalty(dist_cells: f32, grid: &Grid) -> i64 {
    // `dist` is measured to the raw obstacle; the free threshold already includes
    // clearance + half width. Penalise the next two cells beyond that.
    let cell_mm = grid.cell.mm();
    let extra = dist_cells as f64 * cell_mm; // mm from the obstacle
    if extra < 0.8 {
        3
    } else if extra < 1.5 {
        1
    } else {
        0
    }
}

// ---------------------------------------------------------------------------
// Path -> polyline

/// Merge collinear moves, then replace stair-steps by one diagonal plus one
/// straight segment wherever the corridor is free.
fn pull(grid: &Grid, layer: &Layer, cells: &[(usize, usize)]) -> Vec<(usize, usize)> {
    if cells.len() <= 2 {
        return cells.to_vec();
    }
    let free_line = |a: (usize, usize), b: (usize, usize)| -> bool {
        let (dx, dy) = (b.0 as i64 - a.0 as i64, b.1 as i64 - a.1 as i64);
        let n = dx.abs().max(dy.abs()).max(1);
        for k in 0..=n {
            let x = a.0 as i64 + dx * k / n;
            let y = a.1 as i64 + dy * k / n;
            if !layer.free[grid.idx(x as usize, y as usize)] {
                return false;
            }
        }
        true
    };
    // A two-segment 45°/90° path from a to b: diagonal first or straight first.
    let corner_paths = |a: (usize, usize), b: (usize, usize)| -> Vec<(usize, usize)> {
        let (dx, dy) = (b.0 as i64 - a.0 as i64, b.1 as i64 - a.1 as i64);
        let m = dx.abs().min(dy.abs());
        let (sx, sy) = (dx.signum(), dy.signum());
        let c1 = ((a.0 as i64 + sx * m) as usize, (a.1 as i64 + sy * m) as usize); // diagonal first
        let c2 = ((b.0 as i64 - sx * m) as usize, (b.1 as i64 - sy * m) as usize); // straight first
        let mut out = Vec::new();
        if free_line(a, c1) && free_line(c1, b) {
            out.push(c1);
        }
        if free_line(a, c2) && free_line(c2, b) {
            out.push(c2);
        }
        out
    };
    let mut out = vec![cells[0]];
    let mut i = 0;
    while i < cells.len() - 1 {
        // Farthest j reachable with at most one corner.
        let mut best_j = i + 1;
        let mut best_corner: Option<(usize, usize)> = None;
        for j in (i + 1..cells.len()).rev() {
            let (a, b) = (cells[i], cells[j]);
            let (dx, dy) = (b.0 as i64 - a.0 as i64, b.1 as i64 - a.1 as i64);
            let straight = dx == 0 || dy == 0 || dx.abs() == dy.abs();
            if straight && free_line(a, b) {
                best_j = j;
                best_corner = None;
                break;
            }
            let cs = corner_paths(a, b);
            if let Some(c) = cs.first() {
                best_j = j;
                best_corner = Some(*c);
                break;
            }
        }
        if let Some(c) = best_corner {
            if c != cells[i] && c != cells[best_j] {
                out.push(c);
            }
        }
        out.push(cells[best_j]);
        i = best_j;
    }
    out
}

fn polyline_len(pts: &[Point]) -> f64 {
    pts.windows(2).map(|w| ((w[1].x.nm() - w[0].x.nm()) as f64).hypot((w[1].y.nm() - w[0].y.nm()) as f64)).sum::<f64>() / 1e6
}

/// Route one connection. `board` is the current state; nothing is modified.
pub fn route_pin(board: &Board, req: &Request) -> Result<Outcome> {
    let (fr, fp) = crate::model::parse_pin_ref(&req.from)?;
    let (tr, tp) = crate::model::parse_pin_ref(&req.to)?;
    let from = board.pad(&fr, &fp)?;
    let to = board.pad(&tr, &tp)?;
    let net = match (&from.net, &to.net) {
        (Some(a), Some(b)) if a == b => a.clone(),
        (Some(a), Some(b)) => return Err(Error::with_help(format!("{} is on net {a} but {} is on net {b}", req.from, req.to), "a trace joins pins of one net; `pcb connect` them first")),
        _ => return Err(Error::with_help("both pins must be on a net", format!("`pcb connect {} {} --net <name>`", req.from, req.to))),
    };
    let rules = board.rules();
    let class = board.project.nets.get(&net).and_then(|n| n.class.as_deref());
    let (class_w, clearance) = rules.class(class);
    let width = req.width.unwrap_or(class_w);
    let layers: Vec<String> = match &req.layer {
        Some(l) => {
            if !board.layers().contains(l) {
                return Err(Error::with_help(format!("no copper layer `{l}`"), format!("layers are {}", board.layers().join(", "))));
            }
            vec![l.clone()]
        }
        None => board.layers().to_vec(),
    };
    let via_d = rules.via_diameter;
    let start = island(board, from, &net)?;
    let goal = island(board, to, &net)?;
    // Already connected?
    let start_has_goal = start.per_layer.iter().any(|(l, rings)| goal.per_layer.iter().any(|(gl, grings)| l == gl && geom::overlaps(rings, grings)));
    if start_has_goal {
        return Err(Error::msg(format!("{} and {} are already connected", req.from, req.to)));
    }

    {
        let mut grid = build_grid(board, &net, width, clearance, &layers, via_d)?;
        let per = grid.w * grid.h;
        // Start/goal cells: rasterise the islands (only on wanted layers).
        let n_layers = grid.layers.len();
        let mut start_cells: Vec<(usize, usize, usize)> = Vec::new();
        let mut goal_mask = vec![false; per * n_layers];
        let mut goal_cells: Vec<(usize, usize, usize)> = Vec::new();
        for (li, layer) in grid.layers.iter().enumerate() {
            if let Some((_, rings)) = start.per_layer.iter().find(|(l, _)| *l == layer.name) {
                for r in rings {
                    rasterise(&grid, r, &mut |i| {
                        start_cells.push((li, i % grid.w, i / grid.w));
                    });
                }
            }
            if let Some((_, rings)) = goal.per_layer.iter().find(|(l, _)| *l == layer.name) {
                for r in rings {
                    rasterise(&grid, r, &mut |i| {
                        goal_mask[li * per + i] = true;
                        goal_cells.push((li, i % grid.w, i / grid.w));
                    });
                }
            }
        }
        // Island cells sit on same-net copper: make them free even if a neighbouring
        // other-net feature's inflation reaches them, and reject cells too tight to leave.
        for (li, l) in grid.layers.iter_mut().enumerate() {
            for &(sl, x, y) in &start_cells {
                if sl == li {
                    l.free[y * grid.w + x] = true;
                }
            }
            for &(gl, x, y) in &goal_cells {
                if gl == li {
                    l.free[y * grid.w + x] = true;
                }
            }
        }
        if start_cells.is_empty() {
            return Err(Error::msg(format!("{} has no copper on the routable layer(s)", req.from)));
        }
        if goal_cells.is_empty() {
            return Err(Error::msg(format!("{} has no copper on the routable layer(s)", req.to)));
        }
        let goal_bbox = {
            let xs = goal_cells.iter().map(|c| c.1);
            let ys = goal_cells.iter().map(|c| c.2);
            (xs.clone().min().unwrap(), ys.clone().min().unwrap(), xs.max().unwrap(), ys.max().unwrap())
        };
        let via_cost = (req.via_cost_mm / grid.cell.mm() * 10.0) as i64;
        let search = Search { grid: &grid, via_cost };
        // Legs: start -> waypoints... -> goal.
        let mut legs: Vec<Vec<(usize, usize, usize)>> = Vec::new();
        let mut cur_starts = start_cells.clone();
        for wp in &req.waypoints {
            let Some((wx, wy)) = grid.cell_of(*wp) else { return Err(Error::msg(format!("waypoint {wp} is off the board"))) };
            let mut mask = vec![false; per * n_layers];
            for li in 0..n_layers {
                mask[li * per + wy * grid.w + wx] = true;
            }
            let free_somewhere = grid.layers.iter().any(|l| l.free[wy * grid.w + wx]);
            if !free_somewhere {
                return Err(Error::with_help(format!("waypoint {wp} is not routable (too close to other copper or the edge)"), "move it a little"));
            }
            match search.run(&cur_starts, &mask, (wx, wy, wx, wy)) {
                Ok(f) => {
                    let last = *f.path.last().unwrap();
                    cur_starts = vec![last];
                    legs.push(f.path);
                }
                Err(m) => return Ok(blocked(&grid, &m)),
            }
        }
        let last_leg = match search.run(&cur_starts, &goal_mask, goal_bbox) {
            Ok(f) => f.path,
            Err(m) => return Ok(blocked(&grid, &m)),
        };
        legs.push(last_leg);
        // Each leg is pulled straight on its own so waypoints stay where they were
        // asked for; within a leg, a layer change splits the run and places a via.
        let mut pieces: Vec<(usize, Vec<Point>)> = Vec::new();
        let mut vias: Vec<Via> = Vec::new();
        let n_legs = legs.len();
        for (leg_i, leg) in legs.iter().enumerate() {
            if leg.is_empty() {
                continue;
            }
            // Pull from the pad centres when the route begins/ends inside the pads, so
            // the 45° decomposition starts there rather than at an arbitrary pad cell.
            let mut leg: Vec<(usize, usize, usize)> = leg.clone();
            if leg_i == 0 {
                let (l, x, y) = leg[0];
                if geom::contains(&from.copper, grid.centre(x, y)) {
                    if let Some((cx, cy)) = grid.cell_of(from.center) {
                        if (cx, cy) != (x, y) && grid.layers[l].free[cy * grid.w + cx] {
                            leg.insert(0, (l, cx, cy));
                        }
                    }
                }
            }
            if leg_i == n_legs - 1 {
                let (l, x, y) = *leg.last().unwrap();
                if geom::contains(&to.copper, grid.centre(x, y)) {
                    if let Some((cx, cy)) = grid.cell_of(to.center) {
                        if (cx, cy) != (x, y) && grid.layers[l].free[cy * grid.w + cx] {
                            leg.push((l, cx, cy));
                        }
                    }
                }
            }
            let mut run: Vec<(usize, usize)> = Vec::new();
            let mut run_layer = leg[0].0;
            for (k, &(l, x, y)) in leg.iter().enumerate() {
                if l != run_layer {
                    let pulled = pull(&grid, &grid.layers[run_layer], &run);
                    pieces.push((run_layer, pulled.iter().map(|&(x, y)| grid.centre(x, y)).collect()));
                    let (px, py) = *run.last().unwrap();
                    vias.push(Via { at: grid.centre(px, py), net: Some(net.clone()), drill: rules.via_drill, diameter: rules.via_diameter, layers: vec![], routed: false });
                    run = vec![(x, y)];
                    run_layer = l;
                } else {
                    run.push((x, y));
                }
                if k == leg.len() - 1 {
                    let pulled = pull(&grid, &grid.layers[run_layer], &run);
                    pieces.push((run_layer, pulled.iter().map(|&(x, y)| grid.centre(x, y)).collect()));
                }
            }
        }
        // Join consecutive pieces on one layer (leg boundaries) into single traces.
        let mut traces: Vec<Trace> = Vec::new();
        for (li, pts) in pieces {
            let layer_name = grid.layers[li].name.clone();
            match traces.last_mut() {
                Some(t) if t.layer == layer_name && t.points.last() == pts.first() => t.points.extend(pts.into_iter().skip(1)),
                _ => traces.push(Trace { layer: layer_name, net: Some(net.clone()), width, points: pts, routed: false }),
            }
        }
        // Land on the pad centres when the route starts/ends inside the pads themselves.
        if let Some(t) = traces.first_mut() {
            if geom::contains(&from.copper, t.points[0]) {
                t.points[0] = from.center;
            }
        }
        if let Some(t) = traces.last_mut() {
            let n = t.points.len();
            if geom::contains(&to.copper, t.points[n - 1]) {
                t.points[n - 1] = to.center;
            }
        }
        for t in &mut traces {
            t.points.dedup();
        }
        traces.retain(|t| t.points.len() >= 2);
        // Verify against real geometry.
        let violations = verify(board, &net, &traces, &vias, clearance);
        if violations.is_empty() {
            let length_mm = traces.iter().map(|t| polyline_len(&t.points)).sum();
            return Ok(Outcome::Routed { traces, vias, length_mm });
        }
        Err(Error::with_help(
            format!("the route found clears the grid but not the geometry: {}", violations.join("; ")),
            "try a waypoint (`--via x,y`) or a narrower width",
        ))
    }
}

fn blocked(grid: &Grid, m: &Miss) -> Outcome {
    let layer = &grid.layers[m.layer];
    let (w, h) = (grid.w, grid.h);
    // Look as far as the inflation reaches: that is what actually blocked the cell.
    let r = grid.r_cells.ceil() as i64 + 2;
    let mut names: Vec<String> = Vec::new();
    for dy in -r..=r {
        for dx in -r..=r {
            let (x, y) = (m.x as i64 + dx, m.y as i64 + dy);
            if x < 0 || y < 0 || x >= w as i64 || y >= h as i64 {
                continue;
            }
            let o = layer.owner[y as usize * w + x as usize];
            if o != 0 {
                let name = grid.names[o as usize].clone();
                if !names.contains(&name) {
                    names.push(name);
                }
            }
        }
    }
    Outcome::Blocked { layer: layer.name.clone(), closest: grid.centre(m.x, m.y), distance_mm: m.h_cells * grid.cell.mm(), blockers: names }
}

/// Real-geometry clearance check of the proposed copper against other nets.
fn verify(board: &Board, net: &str, traces: &[Trace], vias: &[Via], clearance: Length) -> Vec<String> {
    let mut out = Vec::new();
    let eps = Length::from_nm(2_000);
    for t in traces {
        let mine = geom::stroke_flat(&t.points, t.width);
        let grown = geom::offset(&mine, clearance - eps);
        for (n, ring) in board.copper_on_layer(&t.layer) {
            if n.as_deref() == Some(net) {
                continue;
            }
            if geom::overlaps(&grown, &[ring.clone()]) {
                out.push(format!("trace on {} too close to {} copper", t.layer, n.as_deref().unwrap_or("unassigned")));
                break;
            }
        }
        for h in &board.holes {
            if h.hole.net.as_deref() != Some(net) && geom::overlaps(&grown, &[h.ring.clone()]) {
                out.push(format!("trace on {} too close to the hole at {}", t.layer, h.hole.at));
            }
        }
    }
    for v in vias {
        let disc = geom::offset(&[geom::circle(v.at, v.diameter)], clearance - eps);
        for l in board.layers() {
            for (n, ring) in board.copper_on_layer(l) {
                if n.as_deref() != Some(net) && geom::overlaps(&disc, &[ring]) {
                    out.push(format!("via at {} too close to {} copper on {l}", v.at, n.as_deref().unwrap_or("unassigned")));
                    break;
                }
            }
        }
    }
    out
}
