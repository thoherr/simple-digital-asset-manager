use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::volume::FileLocation;

/// The type of processing recipe.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecipeType {
    Sidecar,
    EmbeddedExport,
}

/// Processing instructions associated with a variant.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Recipe {
    pub id: Uuid,
    pub variant_hash: String,
    pub software: String,
    pub recipe_type: RecipeType,
    pub content_hash: String,
    pub location: FileLocation,
}
