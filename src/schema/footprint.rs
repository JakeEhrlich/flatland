use crate::geom::Edge;
use crate::units::{Length, Point};
use serde::{Deserialize, Serialize};

pub const FOOTPRINT_SCHEMA: &str = "pcb-footprint/1";

/// A footprint: pads plus silkscreen/courtyard graphics, in the footprint's
/// own frame (origin at the component centre, y up, as if on the top side).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Footprint {
    pub schema: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub pads: Vec<Pad>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub silkscreen: Vec<Graphic>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub courtyard: Vec<Graphic>,
    /// Where to draw the reference designator (footprint frame).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label_at: Option<Point>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PadType {
    Smd,
    ThroughHole,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PadShape {
    Circle,
    Rect,
    RoundRect,
    Oval,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Pad {
    pub name: String,
    #[serde(rename = "type")]
    pub pad_type: PadType,
    pub shape: PadShape,
    pub at: Point,
    /// Width and height of the copper.
    pub size: [Length; 2],
    #[serde(default, skip_serializing_if = "is_zero_f64")]
    pub rotation: f64,
    /// Corner radius for `round_rect`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub corner_radius: Option<Length>,
    /// Drill diameter for through-hole pads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drill: Option<Length>,
    /// Slot drill: `drill` is the width, `drill_length` the length along x.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drill_length: Option<Length>,
    /// Through-hole pads are plated by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plated: Option<bool>,
    /// Emit solder paste (SMD only, default true).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paste: Option<bool>,
    /// Override the design-rule solder mask expansion.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mask_expansion: Option<Length>,
    /// Connect to same-net pours through a thermal relief (default true);
    /// false for pads that should be flooded solid (wire pads, heat sinks).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thermal_relief: Option<bool>,
}

fn is_zero_f64(v: &f64) -> bool {
    *v == 0.0
}

impl Pad {
    pub fn is_plated(&self) -> bool {
        self.plated.unwrap_or(true)
    }
}

/// Silkscreen / courtyard graphics.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Graphic {
    Line { from: Point, to: Point, #[serde(default, skip_serializing_if = "Option::is_none")] width: Option<Length> },
    Arc {
        from: Point,
        to: Point,
        center: Point,
        #[serde(default)]
        clockwise: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        width: Option<Length>,
    },
    Circle { center: Point, diameter: Length, #[serde(default, skip_serializing_if = "Option::is_none")] width: Option<Length> },
    /// Closed polyline (outline only).
    Polyline { points: Vec<Point>, #[serde(default, skip_serializing_if = "Option::is_none")] width: Option<Length> },
    /// Filled polygon.
    Polygon { points: Vec<Point> },
    Text { at: Point, text: String, #[serde(default, skip_serializing_if = "Option::is_none")] size: Option<Length> },
}

impl Graphic {
    /// Convert stroked graphics to their centreline polylines (closed flag).
    pub fn polylines(&self) -> Vec<(Vec<Point>, bool)> {
        match self {
            Graphic::Line { from, to, .. } => vec![(vec![*from, *to], false)],
            Graphic::Arc { from, to, center, clockwise, .. } => {
                let mut pts = Vec::new();
                Edge::Arc { from: *from, to: *to, center: *center, clockwise: *clockwise }.discretize(&mut pts);
                pts.push(*to);
                vec![(pts, false)]
            }
            Graphic::Circle { center, diameter, .. } => vec![(crate::geom::circle(*center, *diameter), true)],
            Graphic::Polyline { points, .. } => vec![(points.clone(), true)],
            Graphic::Polygon { .. } | Graphic::Text { .. } => vec![],
        }
    }
    pub fn width(&self) -> Option<Length> {
        match self {
            Graphic::Line { width, .. }
            | Graphic::Arc { width, .. }
            | Graphic::Circle { width, .. }
            | Graphic::Polyline { width, .. } => *width,
            _ => None,
        }
    }
}
