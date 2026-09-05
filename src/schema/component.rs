use super::FileRef;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

pub const COMPONENT_SCHEMA: &str = "pcb-component/1";

/// A component definition, shared between projects via an index.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Component {
    pub schema: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Datasheet (may be a local path or a web URL).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub datasheet: Option<FileRef>,
    /// Footprint file. `None` makes this a *virtual* component: it exists in
    /// the netlist and simulations (e.g. a voltage source) but not on the board.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub footprint: Option<FileRef>,
    #[serde(default)]
    pub pins: Vec<Pin>,
    /// Per-instance parameters (e.g. a resistor's `value`). Substituted into
    /// the SPICE template as `{name}`.
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub parameters: IndexMap<String, Parameter>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spice: Option<SpiceModel>,
    /// Free-form metadata (manufacturer, part number, ...).
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub metadata: IndexMap<String, serde_json::Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Pin {
    /// Name used in `<instance>.<pin>` references, e.g. `1`, `VCC`, `GND`.
    pub name: String,
    /// Footprint pad(s) this pin lands on: a pad name, or a list when one
    /// pin has several solder tabs. Defaults to the pin name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pad: Option<PadRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum PadRef {
    One(String),
    Many(Vec<String>),
}

impl Pin {
    /// The pin's primary pad (used for airwires and pin positions).
    pub fn pad_name(&self) -> &str {
        match &self.pad {
            None => &self.name,
            Some(PadRef::One(p)) => p,
            Some(PadRef::Many(v)) => v.first().map(|s| s.as_str()).unwrap_or(&self.name),
        }
    }
    /// Every pad the pin connects to.
    pub fn pad_names(&self) -> Vec<&str> {
        match &self.pad {
            None => vec![&self.name],
            Some(PadRef::One(p)) => vec![p],
            Some(PadRef::Many(v)) => v.iter().map(|s| s.as_str()).collect(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct Parameter {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// How to emit this component into a SPICE netlist.
///
/// `template` is one or more SPICE lines with placeholders:
/// `{ref}` (instance name), `{pin:NAME}` (net attached to pin NAME),
/// `{PARAM}` (a parameter value). Lines starting with `.` (e.g. `.model`)
/// may be placed in `model`; they are emitted once per distinct component.
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct SpiceModel {
    pub template: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Library files to `.include`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub includes: Vec<FileRef>,
}
