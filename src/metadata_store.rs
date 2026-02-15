use std::path::Path;

use anyhow::Result;
use uuid::Uuid;

use crate::models::Asset;

/// Summary of an asset for listing purposes.
pub struct AssetSummary {
    pub id: Uuid,
    pub name: Option<String>,
    pub asset_type: crate::models::AssetType,
    pub variant_count: usize,
}

/// Persists and retrieves all asset metadata as YAML sidecar files.
pub struct MetadataStore {
    _metadata_dir: std::path::PathBuf,
}

impl MetadataStore {
    pub fn new(catalog_root: &Path) -> Self {
        Self {
            _metadata_dir: catalog_root.join("metadata"),
        }
    }

    /// Write/update sidecar YAML for an asset.
    pub fn save(&self, _asset: &Asset) -> Result<()> {
        anyhow::bail!("not yet implemented")
    }

    /// Read sidecar YAML and return the asset.
    pub fn load(&self, _asset_id: Uuid) -> Result<Asset> {
        anyhow::bail!("not yet implemented")
    }

    /// Enumerate all known assets.
    pub fn list(&self) -> Result<Vec<AssetSummary>> {
        anyhow::bail!("not yet implemented")
    }

    /// Rebuild SQLite catalog from sidecar files.
    pub fn sync_to_catalog(&self, _catalog: &crate::catalog::Catalog) -> Result<()> {
        anyhow::bail!("not yet implemented")
    }
}
