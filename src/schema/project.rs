use super::drc::DrcConfig;
use super::FileRef;
use crate::geom::Edge;
use crate::units::{Length, Point};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

pub const PROJECT_SCHEMA: &str = "pcb-project/1";
pub const PROJECT_FILENAME: &str = "pcb.json";

/// One printed circuit board.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Project {
    pub schema: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Component indexes this board draws parts from, searched in order.
    #[serde(default)]
    pub component_indexes: Vec<FileRef>,
    #[serde(default)]
    pub stackup: Stackup,
    #[serde(default)]
    pub design_rules: DesignRules,
    /// Board outline as a closed chain of edges.
    #[serde(default)]
    pub outline: Vec<Edge>,
    /// Component instances keyed by reference designator.
    #[serde(default)]
    pub components: IndexMap<String, Instance>,
    /// Nets keyed by name. A pin appears in at most one net.
    #[serde(default)]
    pub nets: IndexMap<String, Net>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub holes: Vec<Hole>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pours: Vec<Pour>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub traces: Vec<Trace>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub vias: Vec<Via>,
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub simulations: IndexMap<String, Simulation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub routing: Option<RoutingConfig>,
    /// Design-rule sets, project rules and waivers (`pcb drc`).
    #[serde(default, skip_serializing_if = "DrcConfig::is_empty")]
    pub drc: DrcConfig,
    /// Counter used to generate unique auto net names (`N$1`, `N$2`, ...).
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub next_net_id: u32,
}

fn is_zero_u32(v: &u32) -> bool {
    *v == 0
}

impl Project {
    pub fn new(name: &str) -> Project {
        Project {
            schema: PROJECT_SCHEMA.into(),
            name: name.into(),
            description: None,
            component_indexes: vec![],
            stackup: Stackup::default(),
            design_rules: DesignRules::default(),
            outline: vec![],
            components: IndexMap::new(),
            nets: IndexMap::new(),
            holes: vec![],
            pours: vec![],
            traces: vec![],
            vias: vec![],
            simulations: IndexMap::new(),
            routing: None,
            drc: DrcConfig::default(),
            next_net_id: 0,
        }
    }
}

/// Copper layer stack, listed top to bottom. A one-entry list is a
/// single-sided board (the layer is reachable from both sides).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Stackup {
    pub copper_layers: Vec<String>,
    #[serde(default = "default_board_thickness")]
    pub board_thickness: Length,
    #[serde(default = "default_copper_thickness")]
    pub copper_thickness: Length,
}

fn default_board_thickness() -> Length {
    Length::from_mm(1.6)
}
fn default_copper_thickness() -> Length {
    Length::from_mm(0.035)
}

impl Default for Stackup {
    fn default() -> Self {
        Stackup {
            copper_layers: vec!["F.Cu".into(), "B.Cu".into()],
            board_thickness: default_board_thickness(),
            copper_thickness: default_copper_thickness(),
        }
    }
}

impl Stackup {
    pub fn top(&self) -> &str {
        self.copper_layers.first().map(|s| s.as_str()).unwrap_or("F.Cu")
    }
    pub fn bottom(&self) -> &str {
        self.copper_layers.last().map(|s| s.as_str()).unwrap_or("B.Cu")
    }
    pub fn layer_for_side(&self, side: Side) -> &str {
        match side {
            Side::Top => self.top(),
            Side::Bottom => self.bottom(),
        }
    }
    pub fn has_layer(&self, name: &str) -> bool {
        self.copper_layers.iter().any(|l| l == name)
    }
    pub fn layer_index(&self, name: &str) -> Option<usize> {
        self.copper_layers.iter().position(|l| l == name)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DesignRules {
    pub trace_width: Length,
    pub clearance: Length,
    pub via_drill: Length,
    pub via_diameter: Length,
    /// Copper-to-copper clearance for pours (defaults to `clearance`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pour_clearance: Option<Length>,
    pub mask_expansion: Length,
    pub paste_shrink: Length,
    pub silk_width: Length,
    pub silk_text_size: Length,
    /// Minimum distance from copper to the board edge.
    pub edge_clearance: Length,
    /// Annular ring for plated holes with no explicit diameter.
    pub hole_annular_ring: Length,
    /// Thermal relief gap between a pad and the pour of its net (defaults to `pour_clearance`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thermal_gap: Option<Length>,
    /// Narrowest sliver of pour fill worth keeping (defaults to `trace_width`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pour_min_width: Option<Length>,
    /// Width of the four thermal relief spokes.
    #[serde(default = "default_spoke_width")]
    pub thermal_spoke_width: Length,
    /// Net classes with their own widths/clearances.
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub net_classes: IndexMap<String, NetClass>,
}

fn default_spoke_width() -> Length {
    Length::from_mm(0.3)
}

impl Default for DesignRules {
    fn default() -> Self {
        DesignRules {
            trace_width: Length::from_mm(0.25),
            clearance: Length::from_mm(0.2),
            via_drill: Length::from_mm(0.4),
            via_diameter: Length::from_mm(0.8),
            pour_clearance: None,
            mask_expansion: Length::from_mm(0.05),
            paste_shrink: Length::from_mm(0.0),
            silk_width: Length::from_mm(0.15),
            silk_text_size: Length::from_mm(1.0),
            edge_clearance: Length::from_mm(0.3),
            hole_annular_ring: Length::from_mm(0.5),
            thermal_gap: None,
            pour_min_width: None,
            thermal_spoke_width: default_spoke_width(),
            net_classes: IndexMap::new(),
        }
    }
}

impl DesignRules {
    pub fn pour_clearance(&self) -> Length {
        self.pour_clearance.unwrap_or(self.clearance)
    }
    pub fn thermal_gap(&self) -> Length {
        self.thermal_gap.unwrap_or_else(|| self.pour_clearance())
    }
    pub fn pour_min_width(&self) -> Length {
        self.pour_min_width.unwrap_or(self.trace_width)
    }
    pub fn class(&self, name: Option<&str>) -> (Length, Length) {
        match name.and_then(|n| self.net_classes.get(n)) {
            Some(c) => (c.trace_width.unwrap_or(self.trace_width), c.clearance.unwrap_or(self.clearance)),
            None => (self.trace_width, self.clearance),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct NetClass {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace_width: Option<Length>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clearance: Option<Length>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via_drill: Option<Length>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via_diameter: Option<Length>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default, clap::ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    #[default]
    Top,
    Bottom,
}

impl std::fmt::Display for Side {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self { Side::Top => "top", Side::Bottom => "bottom" })
    }
}

/// A component instance on the board.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Instance {
    /// Component name, optionally qualified with an index: `basic:resistor-0603`.
    pub component: String,
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub parameters: IndexMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placement: Option<Placement>,
    /// Optional human-readable value shown on visualisations (`10k`, `100nF`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Placement {
    pub at: Point,
    /// Counter-clockwise rotation in degrees.
    #[serde(default)]
    pub rotation: f64,
    #[serde(default)]
    pub side: Side,
    /// Locked placements are kept fixed by the router.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub locked: bool,
    /// Reference-designator label centre in the footprint frame (rotates and
    /// mirrors with the part); overrides the footprint's `label_at`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label_at: Option<Point>,
    /// Label text height, overriding the `silk_text_size` rule.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label_size: Option<Length>,
    /// Leave the label off the silkscreen entirely.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub label_hidden: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct Net {
    /// Pin references of the form `<instance>.<pin>`.
    #[serde(default)]
    pub pins: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Hole {
    pub at: Point,
    pub drill: Length,
    #[serde(default)]
    pub plated: bool,
    /// Copper ring outer diameter for plated holes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diameter: Option<Length>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub net: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Pour {
    pub name: String,
    pub layer: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub net: Option<String>,
    #[serde(default)]
    pub edges: Vec<Edge>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clearance: Option<Length>,
    /// Higher priority pours are cut out of lower priority ones.
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    pub priority: i32,
    /// Connect same-net pads with thermal reliefs (default) or solidly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thermal: Option<bool>,
}

fn is_zero_i32(v: &i32) -> bool {
    *v == 0
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Trace {
    pub layer: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub net: Option<String>,
    pub width: Length,
    pub points: Vec<Point>,
    /// Set on traces produced by the autorouter (cleared by `route --clear`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub routed: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Via {
    pub at: Point,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub net: Option<String>,
    pub drill: Length,
    pub diameter: Length,
    /// Layers spanned; empty means all copper layers.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub layers: Vec<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub routed: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct RoutingConfig {
    /// Path to freerouting's executable jar or app launcher.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub freerouting: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub java: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_passes: Option<u32>,
}

// ---------------------------------------------------------------------------
// Simulations

/// A named simulation set-up, like a Fusion "study": rerunnable, with a
/// recorded input hash so stale results can be detected.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Simulation {
    pub analysis: Analysis,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Circuit temperature in °C (SPICE `.temp`). Default 27.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    /// Nominal temperature at which model parameters are specified (`tnom`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nominal_temperature: Option<f64>,
    /// Vectors to save and plot, e.g. `v(VOUT)`, `i(V1)`, `@R1[p]`.
    /// Empty means all node voltages (and source currents).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub probes: Vec<String>,
    /// Instance parameter overrides for this study: `"R1.value": "1k"`.
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub parameters: IndexMap<String, String>,
    /// Extra SPICE lines (stimuli, `.ic`, `.nodeset`, ...).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra: Vec<String>,
    /// `.options` key/values.
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub options: IndexMap<String, String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Analysis {
    /// DC operating point.
    Op,
    /// Transient: `.tran step stop [start]`.
    Tran {
        step: String,
        stop: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        start: Option<String>,
    },
    /// DC sweep of a source (or `temp`): `.dc SRC start stop step`.
    Dc { source: String, start: String, stop: String, step: String },
    /// Small-signal AC sweep: `.ac dec|oct|lin N fstart fstop`.
    Ac {
        #[serde(default = "default_dec")]
        sweep: String,
        points: u32,
        start: String,
        stop: String,
    },
    /// Temperature sweep of the operating point (`.dc temp start stop step`).
    TempSweep { start: f64, stop: f64, step: f64 },
}

fn default_dec() -> String {
    "dec".into()
}

impl Analysis {
    pub fn spice_line(&self) -> String {
        match self {
            Analysis::Op => ".op".into(),
            Analysis::Tran { step, stop, start } => match start {
                Some(s) => format!(".tran {step} {stop} {s}"),
                None => format!(".tran {step} {stop}"),
            },
            Analysis::Dc { source, start, stop, step } => format!(".dc {source} {start} {stop} {step}"),
            Analysis::Ac { sweep, points, start, stop } => format!(".ac {sweep} {points} {start} {stop}"),
            Analysis::TempSweep { start, stop, step } => format!(".dc temp {start} {stop} {step}"),
        }
    }
    pub fn kind(&self) -> &'static str {
        match self {
            Analysis::Op => "op",
            Analysis::Tran { .. } => "tran",
            Analysis::Dc { .. } => "dc",
            Analysis::Ac { .. } => "ac",
            Analysis::TempSweep { .. } => "temp_sweep",
        }
    }
}
