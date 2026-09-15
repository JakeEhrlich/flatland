//! RS-274X writer. Coordinates are integer nanometres, which is exactly
//! the 4.6 millimetre format, so no rounding happens on output.

use crate::geom::{self, Ring};
use crate::units::{Length, Point};
use indexmap::IndexMap;
use std::fmt::Write;

pub struct Gerber {
    header: String,
    body: String,
    /// circle aperture diameter (nm) -> D-code
    apertures: IndexMap<i64, u32>,
    next_d: u32,
}

impl Gerber {
    pub fn new(function: &str, polarity: &str) -> Gerber {
        let mut header = String::new();
        let _ = writeln!(header, "%TF.GenerationSoftware,flatland,pcb,0.1*%");
        let now = time::OffsetDateTime::now_utc();
        let _ = writeln!(
            header,
            "%TF.CreationDate,{:04}-{:02}-{:02}T{:02}:{:02}:{:02}+00:00*%",
            now.year(),
            now.month() as u8,
            now.day(),
            now.hour(),
            now.minute(),
            now.second()
        );
        let _ = writeln!(header, "%TF.FileFunction,{function}*%");
        let _ = writeln!(header, "%TF.FilePolarity,{polarity}*%");
        header.push_str("%FSLAX46Y46*%\n%MOMM*%\n");
        Gerber { header, body: String::new(), apertures: IndexMap::new(), next_d: 10 }
    }

    fn aperture(&mut self, diameter: Length) -> u32 {
        let d = diameter.nm().max(1);
        if let Some(code) = self.apertures.get(&d) {
            return *code;
        }
        let code = self.next_d;
        self.next_d += 1;
        self.apertures.insert(d, code);
        code
    }

    fn coord(p: Point) -> String {
        format!("X{}Y{}", p.x.nm(), p.y.nm())
    }

    /// X2 object attributes for what follows: `TO.P` (refdes, pad) names a
    /// pad, `TO.N` its net. CAM tools show them; `pcb gerbers` reads them
    /// back to verify the files against the netlist.
    pub fn object(&mut self, pad: Option<(&str, &str)>, net: Option<&str>) {
        if let Some((r, p)) = pad {
            let _ = writeln!(self.body, "%TO.P,{},{}*%", r.replace(',', "_"), p.replace(',', "_"));
        }
        if let Some(n) = net {
            let _ = writeln!(self.body, "%TO.N,{}*%", n.replace(',', "_"));
        }
    }

    /// End the current object attributes.
    pub fn object_end(&mut self) {
        self.body.push_str("%TD*%\n");
    }

    pub fn flash_circle(&mut self, at: Point, diameter: Length) {
        let d = self.aperture(diameter);
        let _ = writeln!(self.body, "D{d}*\n{}D03*", Self::coord(at));
    }

    pub fn polyline(&mut self, pts: &[Point], width: Length) {
        if pts.len() < 2 {
            return;
        }
        let d = self.aperture(width);
        let _ = writeln!(self.body, "D{d}*");
        let _ = writeln!(self.body, "{}D02*", Self::coord(pts[0]));
        for p in &pts[1..] {
            let _ = writeln!(self.body, "{}D01*", Self::coord(*p));
        }
    }

    /// Filled polygon (outer ring only).
    pub fn region(&mut self, ring: &Ring) {
        if ring.len() < 3 {
            return;
        }
        self.body.push_str("G36*\n");
        let _ = writeln!(self.body, "{}D02*", Self::coord(ring[0]));
        for p in &ring[1..] {
            let _ = writeln!(self.body, "{}D01*", Self::coord(*p));
        }
        let _ = writeln!(self.body, "{}D01*", Self::coord(ring[0]));
        self.body.push_str("G37*\n");
    }

    /// Polygon set from Clipper: positive rings are filled, negative rings
    /// (holes) are cleared with LPC.
    /// Polygons with holes, drawn without polarity clears (a clear would erase
    /// whatever was drawn earlier inside it): holes are keyholed into their outers.
    pub fn regions_fractured(&mut self, rings: &[Ring]) {
        for r in geom::fracture(rings) {
            self.region(&r);
        }
    }

    pub fn regions_with_holes(&mut self, rings: &[Ring]) {
        let outers: Vec<&Ring> = rings.iter().filter(|r| geom::signed_area(r) > 0.0).collect();
        let holes: Vec<&Ring> = rings.iter().filter(|r| geom::signed_area(r) < 0.0).collect();
        for r in outers {
            self.region(r);
        }
        if !holes.is_empty() {
            self.body.push_str("%LPC*%\n");
            for r in holes {
                self.region(r);
            }
            self.body.push_str("%LPD*%\n");
        }
    }

    pub fn finish(mut self) -> String {
        let mut s = std::mem::take(&mut self.header);
        for (d, code) in &self.apertures {
            let _ = writeln!(s, "%ADD{code}C,{:.6}*%", *d as f64 / 1e6);
        }
        s.push_str("%LPD*%\nG01*\n");
        s.push_str(&self.body);
        s.push_str("M02*\n");
        s
    }
}
