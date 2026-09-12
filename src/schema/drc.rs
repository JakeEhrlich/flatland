//! Design-rule sets: `pcb-drc/1` files (importable like component indexes)
//! and the per-project additions in `pcb.json`.
//!
//! A rule selects a kind of feature, optionally filters it, and applies one
//! check at a severity. See `docs/commands/pcb-drc.md` for the grammar.

use super::FileRef;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

pub const DRC_SCHEMA: &str = "pcb-drc/1";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RuleSet {
    pub schema: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Which boards the set is meant for; a mismatch is reported, not fatal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub applies: Option<Applies>,
    #[serde(default)]
    pub rules: Vec<Rule>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct Applies {
    /// Copper layer counts this set covers (empty: any).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub layers: Vec<usize>,
    /// Informational: `fr4`, `aluminium`, ...
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material: Option<String>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Default, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Warning,
    #[default]
    Error,
}

impl std::fmt::Display for Severity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Severity::Info => "info",
            Severity::Warning => "warning",
            Severity::Error => "error",
        })
    }
}

/// What a rule selects. `copper` stands for traces, vias, pads, plated rings
/// and pours together.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash, clap::ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum Feature {
    Trace,
    Via,
    Pad,
    Hole,
    Pour,
    PourPiece,
    Copper,
    MaskOpening,
    Silk,
    SilkText,
    /// Free text (`pcb text`), on silk or copper.
    Text,
    Courtyard,
    Outline,
    Board,
    Net,
    Pin,
    Part,
}

impl std::fmt::Display for Feature {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Feature::Trace => "trace",
            Feature::Via => "via",
            Feature::Pad => "pad",
            Feature::Hole => "hole",
            Feature::Pour => "pour",
            Feature::PourPiece => "pour_piece",
            Feature::Copper => "copper",
            Feature::MaskOpening => "mask_opening",
            Feature::Silk => "silk",
            Feature::SilkText => "silk_text",
            Feature::Text => "text",
            Feature::Courtyard => "courtyard",
            Feature::Outline => "outline",
            Feature::Board => "board",
            Feature::Net => "net",
            Feature::Pin => "pin",
            Feature::Part => "part",
        })
    }
}

/// Optional filter on the selected features. Every given field must match.
#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq)]
pub struct Where {
    /// Copper layer name, or `top`/`bottom` for silk, mask and courtyards.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub net: Option<String>,
    /// Net class name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plated: Option<bool>,
    /// `smd` or `through_hole` (pads).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refdes: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub footprint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub component: Option<String>,
}

/// Which other features a clearance check is measured against.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Relation {
    /// Different nets (copper with no net counts as different from everything).
    #[default]
    DifferentNet,
    /// Everything except the feature itself.
    Any,
    /// Features belonging to a different part (courtyards, mask openings).
    DifferentPart,
    /// Same net only.
    SameNet,
}

/// A distance: millimetres, or `"design"` for the project's own rule value
/// (clearance, edge clearance or trace width depending on the check).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum Limit {
    Mm(f64),
    Named(String),
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Check {
    /// Grown by `min`, the feature must not touch `to`.
    Clearance {
        to: Feature,
        #[serde(default)]
        relation: Relation,
        min: Limit,
    },
    /// No part of the feature narrower than this (attribute for traces and
    /// pads, morphological opening for pours and other geometry).
    MinWidth(Limit),
    /// No gap inside the feature (or between same-net/same-side features)
    /// narrower than this (morphological closing).
    MinGap(Limit),
    /// Attribute lower bounds, in millimetres (or counts).
    Min(IndexMap<String, f64>),
    /// Attribute upper bounds.
    Max(IndexMap<String, f64>),
    /// The feature must lie entirely within the named feature kind.
    Inside(Feature),
    /// Nets: one island. Pins: on a net. Pour pieces: reach a pad. Traces:
    /// both ends on copper of their net. Pads: touched by copper of their net.
    Connected(bool),
    /// Whether any such feature may exist at all.
    Exists(bool),
    /// Parts must be placed.
    Placed(bool),
    /// The project's design rules must be at least these values.
    DesignRules(IndexMap<String, f64>),
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Rule {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub severity: Severity,
    #[serde(rename = "for")]
    pub for_: Feature,
    #[serde(rename = "where", default, skip_serializing_if = "Option::is_none")]
    pub where_: Option<Where>,
    pub check: Check,
}

/// A silenced finding: the rule, which features (ids, reference designators
/// or net names; empty means all), and why.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Waiver {
    pub rule: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub features: Vec<String>,
    pub reason: String,
}

/// The `drc` section of `pcb.json`.
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct DrcConfig {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rulesets: Vec<FileRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<Rule>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub waivers: Vec<Waiver>,
}

impl DrcConfig {
    pub fn is_empty(&self) -> bool {
        self.rulesets.is_empty() && self.rules.is_empty() && self.waivers.is_empty()
    }
}
