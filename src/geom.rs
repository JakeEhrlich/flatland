//! Board geometry: polygons in integer nanometres, arc discretisation,
//! pad shapes, rigid transforms, and thin wrappers over Clipper2 for
//! boolean operations and offsetting.

use crate::error::{Error, Result};
use crate::units::{Length, Point};
use clipper2::{Clipper, EndType, FillRule, JoinType, One, Path as CPath, Paths as CPaths, PointScaler};
use serde::{Deserialize, Serialize};

/// A closed ring of points (implicitly closed; last point != first).
pub type Ring = Vec<Point>;
/// A set of rings. Positive (CCW in y-up) rings are outers, negative are holes.
pub type Rings = Vec<Ring>;

/// Chord error tolerance used when discretising arcs and circles.
pub const ARC_TOLERANCE: Length = Length(5_000); // 5 µm

/// One edge of an outline: a straight line or a circular arc.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Edge {
    Line { from: Point, to: Point },
    Arc { from: Point, to: Point, center: Point, #[serde(default)] clockwise: bool },
}

impl Edge {
    pub fn from(&self) -> Point {
        match self { Edge::Line { from, .. } | Edge::Arc { from, .. } => *from }
    }
    pub fn to(&self) -> Point {
        match self { Edge::Line { to, .. } | Edge::Arc { to, .. } => *to }
    }
    pub fn describe(&self) -> String {
        match self {
            Edge::Line { from, to } => format!("line {from} -> {to}"),
            Edge::Arc { from, to, center, clockwise } => {
                format!("arc {from} -> {to} about {center} ({})", if *clockwise { "cw" } else { "ccw" })
            }
        }
    }
    /// Points along the edge excluding the end point.
    pub fn discretize(&self, out: &mut Vec<Point>) {
        match self {
            Edge::Line { from, .. } => out.push(*from),
            Edge::Arc { from, to, center, clockwise } => {
                let (cx, cy) = (center.x.nm() as f64, center.y.nm() as f64);
                let r0 = ((from.x.nm() as f64 - cx).powi(2) + (from.y.nm() as f64 - cy).powi(2)).sqrt();
                let r1 = ((to.x.nm() as f64 - cx).powi(2) + (to.y.nm() as f64 - cy).powi(2)).sqrt();
                let r = (r0 + r1) / 2.0;
                let a0 = (from.y.nm() as f64 - cy).atan2(from.x.nm() as f64 - cx);
                let a1 = (to.y.nm() as f64 - cy).atan2(to.x.nm() as f64 - cx);
                let mut sweep = a1 - a0;
                if *clockwise {
                    if sweep > 0.0 { sweep -= std::f64::consts::TAU; }
                    if sweep == 0.0 { sweep = -std::f64::consts::TAU; }
                } else {
                    if sweep < 0.0 { sweep += std::f64::consts::TAU; }
                    if sweep == 0.0 { sweep = std::f64::consts::TAU; }
                }
                let n = arc_segments(r, sweep.abs());
                for i in 0..n {
                    let t = i as f64 / n as f64;
                    let a = a0 + sweep * t;
                    // Interpolate radius so slightly inconsistent arcs still close.
                    let rr = r0 + (r1 - r0) * t;
                    out.push(Point::nm((cx + rr * a.cos()).round() as i64, (cy + rr * a.sin()).round() as i64));
                }
            }
        }
    }
}

/// Number of segments for an arc of radius `r` (nm) and angle `sweep` (rad).
pub fn arc_segments(r: f64, sweep: f64) -> usize {
    let tol = ARC_TOLERANCE.nm() as f64;
    if r <= tol {
        return 1.max((sweep / (std::f64::consts::PI / 4.0)).ceil() as usize);
    }
    let max_step = 2.0 * (1.0 - tol / r).clamp(-1.0, 1.0).acos();
    let n = (sweep / max_step).ceil() as usize;
    n.max(1)
}

/// Validate that edges form one closed loop and return its polygon.
/// `what` names the thing for error messages ("board outline", "pour `gnd`").
pub fn edges_to_ring(edges: &[Edge], what: &str) -> Result<Ring> {
    if edges.is_empty() {
        return Err(Error::with_help(
            format!("{what} has no edges"),
            "add edges with `pcb outline add line x1,y1 x2,y2` / `pcb outline add arc ...` or `pcb outline rect`",
        ));
    }
    let tol = Length(1_000); // 1 µm
    for i in 0..edges.len() {
        let next = &edges[(i + 1) % edges.len()];
        let gap = edges[i].to().distance(next.from());
        if gap > tol {
            let msg = if i + 1 == edges.len() {
                format!(
                    "{what} is not closed: last edge ends at {} but first edge starts at {} ({} apart)",
                    edges[i].to(), next.from(), gap
                )
            } else {
                format!(
                    "{what} has a gap between edge {} ({}) and edge {} ({}): {} apart",
                    i, edges[i].describe(), i + 1, next.describe(), gap
                )
            };
            return Err(Error::with_help(msg, "edges must be listed in order, each starting where the previous one ends"));
        }
    }
    let mut ring = Vec::new();
    for e in edges {
        e.discretize(&mut ring);
    }
    if ring.len() < 3 {
        return Err(Error::msg(format!("{what} has fewer than 3 points")));
    }
    Ok(ring)
}

/// Signed area (nm², positive = counter-clockwise in a y-up frame).
pub fn signed_area(ring: &[Point]) -> f64 {
    let mut a = 0.0;
    for i in 0..ring.len() {
        let p = ring[i];
        let q = ring[(i + 1) % ring.len()];
        a += p.x.nm() as f64 * q.y.nm() as f64 - q.x.nm() as f64 * p.y.nm() as f64;
    }
    a / 2.0
}

pub fn bounds(rings: &[Ring]) -> Option<(Point, Point)> {
    let mut it = rings.iter().flatten();
    let first = *it.next()?;
    let (mut lo, mut hi) = (first, first);
    for p in it {
        lo.x = lo.x.min(p.x);
        lo.y = lo.y.min(p.y);
        hi.x = hi.x.max(p.x);
        hi.y = hi.y.max(p.y);
    }
    Some((lo, hi))
}

// ---------------------------------------------------------------------------
// Shapes

pub fn circle(center: Point, diameter: Length) -> Ring {
    let r = diameter.nm() as f64 / 2.0;
    let n = arc_segments(r, std::f64::consts::TAU).max(8);
    (0..n)
        .map(|i| {
            let a = std::f64::consts::TAU * i as f64 / n as f64;
            Point::nm(
                center.x.nm() + (r * a.cos()).round() as i64,
                center.y.nm() + (r * a.sin()).round() as i64,
            )
        })
        .collect()
}

pub fn rect(center: Point, w: Length, h: Length) -> Ring {
    let (hw, hh) = (w.nm() / 2, h.nm() / 2);
    let (cx, cy) = (center.x.nm(), center.y.nm());
    vec![
        Point::nm(cx - hw, cy - hh),
        Point::nm(cx + hw, cy - hh),
        Point::nm(cx + hw, cy + hh),
        Point::nm(cx - hw, cy + hh),
    ]
}

/// Rectangle with rounded corners of radius `r` (clamped to half the short side).
pub fn round_rect(center: Point, w: Length, h: Length, r: Length) -> Ring {
    let r = r.nm().min(w.nm() / 2).min(h.nm() / 2);
    if r <= 0 {
        return rect(center, w, h);
    }
    let (hw, hh) = (w.nm() / 2, h.nm() / 2);
    let (cx, cy) = (center.x.nm(), center.y.nm());
    let corners = [
        (cx + hw - r, cy + hh - r, 0.0),
        (cx - hw + r, cy + hh - r, 90.0),
        (cx - hw + r, cy - hh + r, 180.0),
        (cx + hw - r, cy - hh + r, 270.0),
    ];
    let n = arc_segments(r as f64, std::f64::consts::FRAC_PI_2).max(3);
    let mut ring = Vec::new();
    for (ox, oy, start) in corners {
        for i in 0..=n {
            let a = (start as f64 + 90.0 * i as f64 / n as f64).to_radians();
            ring.push(Point::nm(ox + (r as f64 * a.cos()).round() as i64, oy + (r as f64 * a.sin()).round() as i64));
        }
    }
    ring
}

/// Stadium / oblong: a rectangle whose short sides are full semicircles.
pub fn oval(center: Point, w: Length, h: Length) -> Ring {
    round_rect(center, w, h, Length(w.nm().min(h.nm()) / 2))
}

// ---------------------------------------------------------------------------
// Transforms

/// A rigid transform: optional mirror across the Y axis (x -> -x), then
/// rotation by `degrees` counter-clockwise, then translation.
#[derive(Clone, Copy, Debug)]
pub struct Transform {
    pub mirror_x: bool,
    pub degrees: f64,
    pub translate: Point,
}

impl Transform {
    pub fn identity() -> Transform {
        Transform { mirror_x: false, degrees: 0.0, translate: Point::ORIGIN }
    }
    pub fn apply(&self, p: Point) -> Point {
        let mut x = p.x.nm() as f64;
        let y = p.y.nm() as f64;
        if self.mirror_x {
            x = -x;
        }
        let (s, c) = self.degrees.to_radians().sin_cos();
        let (rx, ry) = (x * c - y * s, x * s + y * c);
        Point::nm(
            self.translate.x.nm() + rx.round() as i64,
            self.translate.y.nm() + ry.round() as i64,
        )
    }
    /// Inverse of `apply`: board coordinates back into the footprint frame.
    pub fn unapply(&self, p: Point) -> Point {
        let x = (p.x.nm() - self.translate.x.nm()) as f64;
        let y = (p.y.nm() - self.translate.y.nm()) as f64;
        let (s, c) = (-self.degrees).to_radians().sin_cos();
        let (mut rx, ry) = (x * c - y * s, x * s + y * c);
        if self.mirror_x {
            rx = -rx;
        }
        Point::nm(rx.round() as i64, ry.round() as i64)
    }
    pub fn apply_ring(&self, ring: &[Point]) -> Ring {
        let mut r: Ring = ring.iter().map(|p| self.apply(*p)).collect();
        if self.mirror_x {
            r.reverse(); // keep orientation
        }
        r
    }
    /// Rotation of a sub-feature after this transform (degrees).
    pub fn rotate_angle(&self, deg: f64) -> f64 {
        if self.mirror_x { self.degrees - deg } else { self.degrees + deg }
    }
}

// ---------------------------------------------------------------------------
// Clipper2 wrappers. Coordinates are passed as raw nanometres (scaler `One`).

fn to_cpath(ring: &[Point]) -> CPath<One> {
    ring.iter().map(|p| clipper2::Point::<One>::new(p.x.nm() as f64, p.y.nm() as f64)).collect()
}
fn to_cpaths(rings: &[Ring]) -> CPaths<One> {
    rings.iter().map(|r| to_cpath(r)).collect()
}
fn from_cpaths(paths: CPaths<One>) -> Rings {
    paths
        .iter()
        .map(|p| p.iter().map(|q| Point::nm(q.x_scaled(), q.y_scaled())).collect::<Ring>())
        .filter(|r: &Ring| r.len() >= 3)
        .collect()
}

fn clip_err(e: clipper2::ClipperError) -> Error {
    Error::msg(format!("geometry operation failed: {e:?}"))
}

/// Union of all rings (non-zero fill).
pub fn union(rings: &[Ring]) -> Result<Rings> {
    if rings.is_empty() {
        return Ok(vec![]);
    }
    let empty: CPaths<One> = CPaths::new(vec![]);
    let r = Clipper::<_, One>::new().add_subject(to_cpaths(rings)).add_clip(empty).union(FillRule::NonZero).map_err(clip_err)?;
    Ok(from_cpaths(r))
}

pub fn difference(subject: &[Ring], clip: &[Ring]) -> Result<Rings> {
    if subject.is_empty() {
        return Ok(vec![]);
    }
    if clip.is_empty() {
        return union(subject);
    }
    let r = Clipper::<_, One>::new()
        .add_subject(to_cpaths(subject))
        .add_clip(to_cpaths(clip))
        .difference(FillRule::NonZero)
        .map_err(clip_err)?;
    Ok(from_cpaths(r))
}

pub fn intersection(subject: &[Ring], clip: &[Ring]) -> Result<Rings> {
    if subject.is_empty() || clip.is_empty() {
        return Ok(vec![]);
    }
    let r = Clipper::<_, One>::new()
        .add_subject(to_cpaths(subject))
        .add_clip(to_cpaths(clip))
        .intersect(FillRule::NonZero)
        .map_err(clip_err)?;
    Ok(from_cpaths(r))
}

/// Offset closed polygons outward (positive) or inward (negative) with round joins.
pub fn offset(rings: &[Ring], delta: Length) -> Rings {
    if rings.is_empty() {
        return vec![];
    }
    // Union first so overlapping inputs offset as one region.
    let u = union(rings).unwrap_or_else(|_| rings.to_vec());
    let p = to_cpaths(&u).inflate(delta.nm() as f64, JoinType::Round, EndType::Polygon, 2.0);
    from_cpaths(p)
}

/// Morphological opening: remove every feature of `rings` narrower than
/// `width` (erode by half, dilate by half, clip to the original so the
/// arc approximation never grows the region).
pub fn open(rings: &[Ring], width: Length) -> Result<Rings> {
    if rings.is_empty() || width.nm() <= 0 {
        return Ok(rings.to_vec());
    }
    let half = Length::from_nm(width.nm() / 2);
    let eroded = offset(rings, -half);
    if eroded.is_empty() {
        return Ok(vec![]);
    }
    // A hair more than half on the way back so that features exactly at the
    // limit survive rounding.
    let dilated = offset(&eroded, Length::from_nm(half.nm() + 1000));
    intersection(&dilated, rings)
}

/// Thicken an open polyline into a polygon with round caps and joins.
pub fn stroke(points: &[Point], width: Length) -> Rings {
    if points.len() < 2 {
        if let Some(p) = points.first() {
            return vec![circle(*p, width)];
        }
        return vec![];
    }
    let p = CPaths::<One>::from(to_cpath(points)).inflate(width.nm() as f64 / 2.0, JoinType::Round, EndType::Round, 2.0);
    from_cpaths(p)
}

/// Thicken an open polyline with round joins but flat ends: copper traces
/// end inside a pad, and a round cap wider than the pad would poke out of it.
pub fn stroke_flat(points: &[Point], width: Length) -> Rings {
    if points.len() < 2 {
        if let Some(p) = points.first() {
            return vec![circle(*p, width)];
        }
        return vec![];
    }
    let p = CPaths::<One>::from(to_cpath(points)).inflate(width.nm() as f64 / 2.0, JoinType::Round, EndType::Butt, 2.0);
    from_cpaths(p)
}

/// Do any of `a` overlap any of `b`?
pub fn overlaps(a: &[Ring], b: &[Ring]) -> bool {
    intersection(a, b).map(|r| r.iter().any(|ring| signed_area(ring).abs() > 0.0)).unwrap_or(false)
}

pub fn area(rings: &[Ring]) -> f64 {
    rings.iter().map(|r| signed_area(r)).sum()
}

/// Does `ring` contain `p` (edges count as inside)?
pub fn contains(ring: &[Point], p: Point) -> bool {
    let cp = clipper2::Point::<One>::new(p.x.nm() as f64, p.y.nm() as f64);
    !matches!(to_cpath(ring).is_point_inside(cp), clipper2::PointInPolygonResult::IsOutside)
}

#[allow(dead_code)]
fn _assert_scaler() {
    fn f<P: PointScaler>() {}
    f::<One>();
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn square_area() {
        let r = rect(Point::ORIGIN, Length::from_mm(2.0), Length::from_mm(2.0));
        assert!((signed_area(&r) - 4e12).abs() < 1.0);
    }
    #[test]
    fn opening_keeps_holes_and_wide_channels() {
        // 20x20 square with a 10x10 hole: an opening must not touch it.
        let outer = rect(Point::ORIGIN, Length::from_mm(20.0), Length::from_mm(20.0));
        let hole = rect(Point::ORIGIN, Length::from_mm(10.0), Length::from_mm(10.0));
        let annulus = difference(&[outer], &[hole]).unwrap();
        let before: f64 = annulus.iter().map(|r| signed_area(r)).sum::<f64>() / 1e12;
        let after = open(&annulus, Length::from_mm(0.4)).unwrap();
        let a: f64 = after.iter().map(|r| signed_area(r)).sum::<f64>() / 1e12;
        assert!((before - 300.0).abs() < 0.01 && (a - before).abs() < 0.05, "annulus {before} -> {a}");
        // Two squares joined by a 0.8 mm channel survive a 0.4 mm opening as one piece;
        // a 0.2 mm channel is removed and leaves two pieces.
        for (w, pieces) in [(0.8, 1usize), (0.2, 2usize)] {
            let a = rect(Point::mm(0.0, 0.0), Length::from_mm(10.0), Length::from_mm(10.0));
            let b = rect(Point::mm(20.0, 0.0), Length::from_mm(10.0), Length::from_mm(10.0));
            let neck = rect(Point::mm(10.0, 0.0), Length::from_mm(10.2), Length::from_mm(w));
            let joined = union(&[a, b, neck]).unwrap();
            assert_eq!(joined.iter().filter(|r| signed_area(r) > 0.0).count(), 1);
            let opened = open(&joined, Length::from_mm(0.4)).unwrap();
            let outers = opened.iter().filter(|r| signed_area(r) > 0.0).count();
            assert_eq!(outers, pieces, "channel {w} mm");
            let area: f64 = opened.iter().map(|r| signed_area(r)).sum::<f64>() / 1e12;
            let expect = if pieces == 1 { 200.0 + 10.0 * w } else { 200.0 };
            assert!((area - expect).abs() < 0.2, "channel {w}: area {area}, expected {expect}");
        }
    }
    #[test]
    fn circle_is_round() {
        let c = circle(Point::ORIGIN, Length::from_mm(1.0));
        let a = signed_area(&c) / 1e12;
        assert!((a - std::f64::consts::PI / 4.0).abs() < 0.01, "{a}");
    }
    #[test]
    fn closes_outline() {
        let e = vec![
            Edge::Line { from: Point::mm(0.0, 0.0), to: Point::mm(10.0, 0.0) },
            Edge::Arc { from: Point::mm(10.0, 0.0), to: Point::mm(10.0, 10.0), center: Point::mm(10.0, 5.0), clockwise: false },
            Edge::Line { from: Point::mm(10.0, 10.0), to: Point::mm(0.0, 10.0) },
            Edge::Line { from: Point::mm(0.0, 10.0), to: Point::mm(0.0, 0.0) },
        ];
        let ring = edges_to_ring(&e, "test").unwrap();
        let a = signed_area(&ring) / 1e12;
        // The ccw arc from (10,0) to (10,10) about (10,5) sweeps through 0°,
        // so it bulges outward (x > 10): square plus a half disc.
        assert!((a - (100.0 + std::f64::consts::PI * 25.0 / 2.0)).abs() < 0.05, "{a}");
        let bad = vec![e[0].clone(), e[2].clone()];
        assert!(edges_to_ring(&bad, "test").is_err());
    }
    #[test]
    fn boolean_ops() {
        let a = rect(Point::ORIGIN, Length::from_mm(4.0), Length::from_mm(4.0));
        let b = rect(Point::mm(2.0, 0.0), Length::from_mm(4.0), Length::from_mm(4.0));
        let u = union(&[a.clone(), b.clone()]).unwrap();
        assert!((area(&u) / 1e12 - 24.0).abs() < 0.01);
        let d = difference(&[a.clone()], &[b]).unwrap();
        assert!((area(&d) / 1e12 - 8.0).abs() < 0.01);
        let s = stroke(&[Point::mm(0.0, 0.0), Point::mm(10.0, 0.0)], Length::from_mm(1.0));
        let sa = area(&s) / 1e12;
        assert!((sa - (10.0 + std::f64::consts::PI / 4.0)).abs() < 0.05, "{sa}");
    }
}
