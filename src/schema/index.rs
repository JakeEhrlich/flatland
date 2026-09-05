use super::FileRef;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

pub const INDEX_SCHEMA: &str = "pcb-component-index/1";

/// A component index: a shareable catalogue mapping component names to
/// component files. Indexes are meant to be published as directories/zips/
/// git repos and mirrored; every entry carries a URL and hash.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ComponentIndex {
    pub schema: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub components: IndexMap<String, IndexEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IndexEntry {
    #[serde(flatten)]
    pub file: FileRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl ComponentIndex {
    pub fn new(name: &str) -> Self {
        ComponentIndex { schema: INDEX_SCHEMA.into(), name: name.into(), description: None, components: IndexMap::new() }
    }
}
