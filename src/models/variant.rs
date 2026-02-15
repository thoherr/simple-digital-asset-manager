use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::volume::FileLocation;

/// The role a variant plays within an asset group.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VariantRole {
    Original,
    Processed,
    Export,
    Sidecar,
}

/// A concrete file belonging to an asset.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Variant {
    pub content_hash: String,
    pub asset_id: Uuid,
    pub role: VariantRole,
    pub format: String,
    pub file_size: u64,
    pub original_filename: String,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub source_metadata: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub locations: Vec<FileLocation>,
}
