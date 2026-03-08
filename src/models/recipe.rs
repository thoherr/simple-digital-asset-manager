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
    /// True when metadata was edited in the DAM but the XMP file could not be
    /// updated (e.g. because the volume was offline). Cleared after successful
    /// write-back.
    #[serde(default, skip_serializing_if = "is_false")]
    pub pending_writeback: bool,
}

fn is_false(v: &bool) -> bool {
    !*v
}
