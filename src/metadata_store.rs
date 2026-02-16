use std::path::Path;

use anyhow::Result;
use uuid::Uuid;

use crate::models::Asset;

/// Result of syncing sidecar files to the catalog.
pub struct SyncResult {
    pub synced: u64,
    pub errors: u64,
}

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

    /// Delete the sidecar YAML file for an asset.
    pub fn delete(&self, asset_id: Uuid) -> Result<()> {
        let path = self.sidecar_path(asset_id);
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
        Ok(())
    }

    /// Enumerate all known assets by walking sidecar YAML files.
    pub fn list(&self) -> Result<Vec<AssetSummary>> {
        let mut summaries = Vec::new();

        if !self.metadata_dir.exists() {
            return Ok(summaries);
        }

        for shard_entry in std::fs::read_dir(&self.metadata_dir)? {
            let shard_entry = shard_entry?;
            if !shard_entry.file_type()?.is_dir() {
                continue;
            }
            for file_entry in std::fs::read_dir(shard_entry.path())? {
                let file_entry = file_entry?;
                let path = file_entry.path();
                let ext = path.extension().and_then(|e| e.to_str());
                if ext != Some("yaml") {
                    continue;
                }
                let stem = match path.file_stem().and_then(|s| s.to_str()) {
                    Some(s) => s,
                    None => continue,
                };
                let id = match uuid::Uuid::parse_str(stem) {
                    Ok(id) => id,
                    Err(_) => continue,
                };
                match self.load(id) {
                    Ok(asset) => {
                        summaries.push(AssetSummary {
                            id: asset.id,
                            name: asset.name.clone(),
                            asset_type: asset.asset_type.clone(),
                            variant_count: asset.variants.len(),
                        });
                    }
                    Err(e) => {
                        eprintln!("Warning: failed to load sidecar {}: {e}", path.display());
                    }
                }
            }
        }

        Ok(summaries)
    }

    /// Rebuild SQLite catalog from sidecar files.
    pub fn sync_to_catalog(&self, catalog: &crate::catalog::Catalog) -> Result<SyncResult> {
        let summaries = self.list()?;
        let mut synced = 0u64;
        let mut errors = 0u64;

        for summary in &summaries {
            match self.load(summary.id) {
                Ok(asset) => {
                    if let Err(e) = catalog.insert_asset(&asset) {
                        eprintln!("Error inserting asset {}: {e}", summary.id);
                        errors += 1;
                        continue;
                    }
                    for variant in &asset.variants {
                        if let Err(e) = catalog.insert_variant(variant) {
                            eprintln!(
                                "Error inserting variant {} for asset {}: {e}",
                                variant.content_hash, summary.id
                            );
                            errors += 1;
                            continue;
                        }
                        for loc in &variant.locations {
                            if let Err(e) = catalog.insert_file_location(&variant.content_hash, loc)
                            {
                                eprintln!(
                                    "Error inserting location for variant {}: {e}",
                                    variant.content_hash
                                );
                                errors += 1;
                            }
                        }
                    }
                    for recipe in &asset.recipes {
                        if let Err(e) = catalog.insert_recipe(recipe) {
                            eprintln!(
                                "Error inserting recipe {} for asset {}: {e}",
                                recipe.id, summary.id
                            );
                            errors += 1;
                        }
                    }
                    synced += 1;
                }
                Err(e) => {
                    eprintln!("Error loading asset {}: {e}", summary.id);
                    errors += 1;
                }
            }
        }

        Ok(SyncResult { synced, errors })
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

        let asset = Asset::new(AssetType::Image, "sha256:meta_test1");
        let id = asset.id;

        store.save(&asset).unwrap();
        let loaded = store.load(id).unwrap();

        assert_eq!(loaded.id, id);
        assert_eq!(loaded.asset_type, AssetType::Image);
    }

    #[test]
    fn list_returns_saved_assets() {
        let dir = tempfile::tempdir().unwrap();
        let store = MetadataStore::new(dir.path());

        let mut a1 = Asset::new(AssetType::Image, "sha256:list1");
        a1.name = Some("First".to_string());
        let mut a2 = Asset::new(AssetType::Video, "sha256:list2");
        a2.name = Some("Second".to_string());

        store.save(&a1).unwrap();
        store.save(&a2).unwrap();

        let summaries = store.list().unwrap();
        assert_eq!(summaries.len(), 2);

        let mut ids: Vec<_> = summaries.iter().map(|s| s.id).collect();
        ids.sort();
        let mut expected = vec![a1.id, a2.id];
        expected.sort();
        assert_eq!(ids, expected);
    }

    #[test]
    fn list_empty_returns_empty() {
        let dir = tempfile::tempdir().unwrap();
        let store = MetadataStore::new(dir.path());
        let summaries = store.list().unwrap();
        assert!(summaries.is_empty());
    }

    #[test]
    fn sync_to_catalog_inserts_assets_and_variants() {
        use crate::catalog::Catalog;
        use crate::models::{FileLocation, Variant, VariantRole, Volume, VolumeType};

        let dir = tempfile::tempdir().unwrap();
        let store = MetadataStore::new(dir.path());
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();

        // Create a volume so FK references work
        let volume = Volume::new(
            "test-vol".to_string(),
            std::path::PathBuf::from("/mnt/test"),
            VolumeType::Local,
        );
        catalog.ensure_volume(&volume).unwrap();

        // Create an asset with a variant and location
        let mut asset = Asset::new(AssetType::Image, "sha256:sync1");
        asset.name = Some("synced".to_string());
        let variant = Variant {
            content_hash: "sha256:sync1".to_string(),
            asset_id: asset.id,
            role: VariantRole::Original,
            format: "jpg".to_string(),
            file_size: 1024,
            original_filename: "photo.jpg".to_string(),
            source_metadata: Default::default(),
            locations: vec![FileLocation {
                volume_id: volume.id,
                relative_path: std::path::PathBuf::from("photos/photo.jpg"),
                verified_at: None,
            }],
        };
        asset.variants.push(variant);
        store.save(&asset).unwrap();

        let result = store.sync_to_catalog(&catalog).unwrap();
        assert_eq!(result.synced, 1);
        assert_eq!(result.errors, 0);

        // Verify asset is in the catalog
        let details = catalog.load_asset_details(&asset.id.to_string()).unwrap().unwrap();
        assert_eq!(details.name.as_deref(), Some("synced"));
        assert_eq!(details.variants.len(), 1);
        assert_eq!(details.variants[0].content_hash, "sha256:sync1");
        assert_eq!(details.variants[0].locations.len(), 1);
        assert_eq!(details.variants[0].locations[0].relative_path, "photos/photo.jpg");
    }
}
