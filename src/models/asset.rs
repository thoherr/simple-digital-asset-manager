use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::recipe::Recipe;
use super::variant::Variant;

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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub variants: Vec<Variant>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recipes: Vec<Recipe>,
}

impl Asset {
    pub fn new(asset_type: AssetType) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: None,
            created_at: Utc::now(),
            asset_type,
            tags: Vec::new(),
            description: None,
            variants: Vec::new(),
            recipes: Vec::new(),
        }
    }
}
