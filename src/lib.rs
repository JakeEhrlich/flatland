//! flatland: agent-oriented PCB design tooling.
//!
//! A single project JSON describes one board (netlist, placements, outline,
//! traces, pours, simulations). Component indexes are shareable JSON files
//! mapping component names to component files; component files reference a
//! footprint, a datasheet and an optional SPICE model. Everything is addressed
//! by URL (relative paths today, http/s3 mirrors later) with optional blake3
//! hashes so libraries can be pinned and mirrored.

#![allow(clippy::result_large_err, clippy::too_many_arguments)]

pub mod error;
pub mod units;
pub mod geom;
pub mod schema;
pub mod store;
pub mod model;
pub mod viz;
pub mod dsn;
pub mod route;
pub mod gerber;
pub mod sim;
pub mod cli;

pub use error::{Error, Result};
