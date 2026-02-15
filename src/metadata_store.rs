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
    metadata_dir: std::path::PathBuf,
}

impl MetadataStore {
    pub fn new(catalog_root: &Path) -> Self {
        Self {
            metadata_dir: catalog_root.join("metadata"),
        }
    }

    /// Shard directory: first 2 chars of UUID hex.
    fn shard_dir(&self, asset_id: Uuid) -> std::path::PathBuf {
        let hex = asset_id.to_string();
        let prefix = &hex[..2];
        self.metadata_dir.join(prefix)
    }

    fn sidecar_path(&self, asset_id: Uuid) -> std::path::PathBuf {
        self.shard_dir(asset_id).join(format!("{}.yaml", asset_id))
    }

    /// Write/update sidecar YAML for an asset.
    pub fn save(&self, asset: &Asset) -> Result<()> {
        let dir = self.shard_dir(asset.id);
        std::fs::create_dir_all(&dir)?;
        let yaml = serde_yaml::to_string(asset)?;
        std::fs::write(self.sidecar_path(asset.id), yaml)?;
        Ok(())
    }

    /// Read sidecar YAML and return the asset.
    pub fn load(&self, asset_id: Uuid) -> Result<Asset> {
        let path = self.sidecar_path(asset_id);
        let contents = std::fs::read_to_string(&path)?;
        let asset: Asset = serde_yaml::from_str(&contents)?;
        Ok(asset)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::AssetType;

    #[test]
    fn save_and_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let store = MetadataStore::new(dir.path());

        let asset = Asset::new(AssetType::Image);
        let id = asset.id;

        store.save(&asset).unwrap();
        let loaded = store.load(id).unwrap();

        assert_eq!(loaded.id, id);
        assert_eq!(loaded.asset_type, AssetType::Image);
    }
}
