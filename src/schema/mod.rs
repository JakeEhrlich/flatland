//! On-disk JSON schemas. Three kinds of files:
//!
//! * a **project** (`pcb.json`): one board — netlist, placements, outline,
//!   traces, pours, simulations, and the list of component indexes it uses;
//! * a **component index**: a shareable map of component names to component
//!   files (URLs + hashes);
//! * a **component** file: pins, a footprint reference, a datasheet URL and
//!   an optional SPICE model; and a **footprint** file: pads and graphics.

pub mod component;
pub mod footprint;
pub mod index;
pub mod project;

pub use component::*;
pub use footprint::*;
pub use index::*;
pub use project::*;

use serde::{Deserialize, Serialize};

/// A reference to another file: a URL (relative path, `file:`, or later
/// `http(s):`/`s3:`) plus an optional blake3 of the file's bytes.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct FileRef {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blake3: Option<String>,
}

impl FileRef {
    pub fn new(url: impl Into<String>) -> FileRef {
        FileRef { url: url.into(), blake3: None }
    }
}
