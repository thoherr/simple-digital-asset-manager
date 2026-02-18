use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::recipe::Recipe;
use super::variant::Variant;

/// Fixed namespace UUID for deriving content-addressable asset IDs via UUID v5.
/// Generated once; must never change (doing so would break all existing asset IDs).
const DAM_NAMESPACE: Uuid = Uuid::from_bytes([
    0x8a, 0x3b, 0x7e, 0x01, 0x4f, 0xd2, 0x4a, 0x6b, 0x9c, 0x1d, 0xe7, 0x5a, 0x0b, 0xf3, 0x28,
    0x4c,
]);

/// The type of digital asset.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetType {
    Image,
    Video,
    Audio,
    Document,
    Other,
}

/// The central entity. Represents a logical asset (e.g. "photo of sunset at beach").
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Asset {
    pub id: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub created_at: DateTime<Utc>,
    pub asset_type: AssetType,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rating: Option<u8>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub variants: Vec<Variant>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recipes: Vec<Recipe>,
}

impl Asset {
    /// Create a new asset with a deterministic ID derived from the content hash.
    /// Same content hash always produces the same asset ID.
    pub fn new(asset_type: AssetType, content_hash: &str) -> Self {
        Self {
            id: Uuid::new_v5(&DAM_NAMESPACE, content_hash.as_bytes()),
            name: None,
            created_at: Utc::now(),
            asset_type,
            tags: Vec::new(),
            description: None,
            rating: None,
            variants: Vec::new(),
            recipes: Vec::new(),
        }
    }
}
