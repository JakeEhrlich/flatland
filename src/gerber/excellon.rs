//! Excellon drill file writer (metric, decimal coordinates).

use crate::units::{Length, Point};
use indexmap::IndexMap;
use std::fmt::Write;

pub struct Drill {
    plated: bool,
    /// tool diameter (nm) -> list of (hole | slot)
    tools: IndexMap<i64, Vec<Op>>,
}

enum Op {
    Hole(Point),
    Slot(Point, Point),
}

impl Drill {
    pub fn new(plated: bool) -> Drill {
        Drill { plated, tools: IndexMap::new() }
    }
    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }
    pub fn hole(&mut self, at: Point, diameter: Length) {
        self.tools.entry(diameter.nm()).or_default().push(Op::Hole(at));
    }
    /// Slot of width `diameter` and total length `length` along `rotation` degrees.
    pub fn slot(&mut self, center: Point, diameter: Length, length: Length, rotation: f64) {
        let half = (length.nm() - diameter.nm()) as f64 / 2.0;
        let (s, c) = rotation.to_radians().sin_cos();
        let a = Point::nm(center.x.nm() - (half * c).round() as i64, center.y.nm() - (half * s).round() as i64);
        let b = Point::nm(center.x.nm() + (half * c).round() as i64, center.y.nm() + (half * s).round() as i64);
        self.tools.entry(diameter.nm()).or_default().push(Op::Slot(a, b));
    }
    fn coord(p: Point) -> String {
        format!("X{:.3}Y{:.3}", p.x.mm(), p.y.mm())
    }
    pub fn finish(self) -> String {
        let mut s = String::new();
        s.push_str("M48\n");
        s.push_str("; DRILL file {flatland} \n");
        let _ = writeln!(s, "; #@! TF.FileFunction,{}", if self.plated { "Plated,1,2,PTH" } else { "NonPlated,1,2,NPTH" });
        s.push_str("FMAT,2\nMETRIC\n");
        let mut sorted: Vec<(&i64, &Vec<Op>)> = self.tools.iter().collect();
        sorted.sort_by_key(|(d, _)| **d);
        for (i, (d, _)) in sorted.iter().enumerate() {
            let _ = writeln!(s, "T{}C{:.3}", i + 1, **d as f64 / 1e6);
        }
        s.push_str("%\nG90\nG05\n");
        for (i, (_, ops)) in sorted.iter().enumerate() {
            let _ = writeln!(s, "T{}", i + 1);
            for op in ops.iter() {
                match op {
                    Op::Hole(p) => {
                        let _ = writeln!(s, "{}", Self::coord(*p));
                    }
                    Op::Slot(a, b) => {
                        let _ = writeln!(s, "{}G85{}", Self::coord(*a), Self::coord(*b));
                    }
                }
            }
        }
        s.push_str("T0\nM30\n");
        s
    }
}
