use std::collections::HashSet;
use std::path::Path;

use anyhow::Result;

use crate::catalog::{AssetDetails, Catalog, SearchRow};
use crate::metadata_store::MetadataStore;

/// Result of a group operation.
#[derive(Debug)]
pub struct GroupResult {
    /// The asset ID that all variants were merged into.
    pub target_id: String,
    /// Number of variants moved from donor assets.
    pub variants_moved: usize,
    /// Number of donor assets removed.
    pub donors_removed: usize,
}

/// Result of a tag add/remove operation.
pub struct TagResult {
    /// Tags that were actually added or removed.
    pub changed: Vec<String>,
    /// The full set of tags after the operation.
    pub current_tags: Vec<String>,
}

/// Search and filter assets via the SQLite catalog.
pub struct QueryEngine {
    catalog_root: std::path::PathBuf,
}

impl QueryEngine {
    pub fn new(catalog_root: &Path) -> Self {
        Self {
            catalog_root: catalog_root.to_path_buf(),
        }
    }

    /// Search assets by a free-text query string.
    ///
    /// Supports prefix filters: `type:image`, `tag:landscape`, `format:jpg`.
    /// Remaining tokens are joined as free-text search against name/filename/description.
    /// Multiple tokens are AND-ed.
    pub fn search(&self, query: &str) -> Result<Vec<SearchRow>> {
        let mut text_parts = Vec::new();
        let mut asset_type = None;
        let mut tag = None;
        let mut format = None;
        let mut rating_min = None;
        let mut rating_exact = None;

        for token in query.split_whitespace() {
            if let Some(value) = token.strip_prefix("type:") {
                asset_type = Some(value.to_string());
            } else if let Some(value) = token.strip_prefix("tag:") {
                tag = Some(value.to_string());
            } else if let Some(value) = token.strip_prefix("format:") {
                format = Some(value.to_string());
            } else if let Some(value) = token.strip_prefix("rating:") {
                if let Some(num_str) = value.strip_suffix('+') {
                    if let Ok(n) = num_str.parse::<u8>() {
                        rating_min = Some(n);
                    }
                } else if let Ok(n) = value.parse::<u8>() {
                    rating_exact = Some(n);
                }
            } else {
                text_parts.push(token);
            }
        }

        let text = if text_parts.is_empty() {
            None
        } else {
            Some(text_parts.join(" "))
        };

        let catalog = Catalog::open(&self.catalog_root)?;
        catalog.search_assets(
            text.as_deref(),
            asset_type.as_deref(),
            tag.as_deref(),
            format.as_deref(),
            rating_min,
            rating_exact,
        )
    }

    /// Look up a single asset by its full ID or a unique prefix.
    pub fn show(&self, asset_id_prefix: &str) -> Result<AssetDetails> {
        let catalog = Catalog::open(&self.catalog_root)?;
        let full_id = catalog
            .resolve_asset_id(asset_id_prefix)?
            .ok_or_else(|| anyhow::anyhow!("No asset found matching '{asset_id_prefix}'"))?;
        catalog
            .load_asset_details(&full_id)?
            .ok_or_else(|| anyhow::anyhow!("Asset '{full_id}' not found in catalog"))
    }

    /// Group variants (identified by content hashes) into a single asset.
    ///
    /// Picks the oldest asset as the target, moves all other variants into it,
    /// merges tags, and deletes donor assets.
    pub fn group(&self, variant_hashes: &[String]) -> Result<GroupResult> {
        if variant_hashes.is_empty() {
            anyhow::bail!("No variant hashes provided");
        }

        let catalog = Catalog::open(&self.catalog_root)?;
        let store = MetadataStore::new(&self.catalog_root);

        // Step 1: Look up owning asset for each hash
        let mut asset_ids = Vec::new();
        for hash in variant_hashes {
            let asset_id = catalog
                .find_asset_id_by_variant(hash)?
                .ok_or_else(|| anyhow::anyhow!("No variant found with hash '{hash}'"))?;
            asset_ids.push(asset_id);
        }

        // Step 2: Collect unique asset IDs
        let unique_ids: Vec<String> = {
            let mut seen = HashSet::new();
            asset_ids
                .iter()
                .filter(|id| seen.insert((*id).clone()))
                .cloned()
                .collect()
        };

        if unique_ids.len() == 1 {
            return Ok(GroupResult {
                target_id: unique_ids.into_iter().next().unwrap(),
                variants_moved: 0,
                donors_removed: 0,
            });
        }

        // Step 3: Load all assets from sidecar, pick oldest as target
        let mut assets: Vec<crate::models::Asset> = unique_ids
            .iter()
            .map(|id| {
                let uuid: uuid::Uuid = id.parse()?;
                store.load(uuid)
            })
            .collect::<Result<_>>()?;

        assets.sort_by_key(|a| a.created_at);
        let target_id = assets[0].id;
        let mut target = assets.remove(0);
        let donors = assets; // remaining are donors

        // Step 4: Merge variants and tags from donors into target
        let mut variants_moved = 0;
        let existing_tags: HashSet<String> = target.tags.iter().cloned().collect();
        let mut all_tags = existing_tags;

        for donor in &donors {
            for variant in &donor.variants {
                let mut moved_variant = variant.clone();
                moved_variant.asset_id = target_id;
                target.variants.push(moved_variant);
                variants_moved += 1;
            }
            for tag in &donor.tags {
                if all_tags.insert(tag.clone()) {
                    target.tags.push(tag.clone());
                }
            }
        }

        // Step 5: Save target sidecar and update catalog
        store.save(&target)?;
        catalog.insert_asset(&target)?;

        // Step 6: Update variant rows in catalog and clean up donors
        for donor in &donors {
            for variant in &donor.variants {
                catalog.update_variant_asset_id(
                    &variant.content_hash,
                    &target_id.to_string(),
                )?;
            }
            store.delete(donor.id)?;
            catalog.delete_asset(&donor.id.to_string())?;
        }

        let donors_removed = donors.len();

        Ok(GroupResult {
            target_id: target_id.to_string(),
            variants_moved,
            donors_removed,
        })
    }

    /// Add or remove tags on an asset. Updates both sidecar YAML and SQLite catalog.
    pub fn tag(&self, asset_id_prefix: &str, tags: &[String], remove: bool) -> Result<TagResult> {
        let catalog = Catalog::open(&self.catalog_root)?;
        let full_id = catalog
            .resolve_asset_id(asset_id_prefix)?
            .ok_or_else(|| anyhow::anyhow!("No asset found matching '{asset_id_prefix}'"))?;

        let uuid: uuid::Uuid = full_id.parse()?;
        let store = MetadataStore::new(&self.catalog_root);
        let mut asset = store.load(uuid)?;

        let changed;
        if remove {
            let to_remove: std::collections::HashSet<&str> =
                tags.iter().map(|s| s.as_str()).collect();
            let mut actually_removed = Vec::new();
            asset.tags.retain(|t| {
                if to_remove.contains(t.as_str()) {
                    actually_removed.push(t.clone());
                    false
                } else {
                    true
                }
            });
            changed = actually_removed;
        } else {
            let existing: std::collections::HashSet<String> =
                asset.tags.iter().cloned().collect();
            let mut added = Vec::new();
            for tag in tags {
                if !existing.contains(tag) {
                    asset.tags.push(tag.clone());
                    added.push(tag.clone());
                }
            }
            changed = added;
        }

        store.save(&asset)?;
        catalog.insert_asset(&asset)?;

        Ok(TagResult {
            changed,
            current_tags: asset.tags.clone(),
        })
    }

    /// Set the rating on an asset. Updates both sidecar YAML and SQLite catalog.
    /// Returns the new rating value.
    pub fn set_rating(&self, asset_id_prefix: &str, rating: Option<u8>) -> Result<Option<u8>> {
        let catalog = Catalog::open(&self.catalog_root)?;
        let full_id = catalog
            .resolve_asset_id(asset_id_prefix)?
            .ok_or_else(|| anyhow::anyhow!("No asset found matching '{asset_id_prefix}'"))?;

        let uuid: uuid::Uuid = full_id.parse()?;
        let store = MetadataStore::new(&self.catalog_root);
        let mut asset = store.load(uuid)?;

        asset.rating = rating;
        store.save(&asset)?;
        catalog.update_asset_rating(&full_id, rating)?;

        Ok(rating)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Catalog;
    use crate::models::{Asset, AssetType};

    /// Set up a temp catalog with one asset and its sidecar, returning (dir, asset_id).
    fn setup_tag_env() -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        let catalog_root = dir.path();

        // Init catalog
        let catalog = Catalog::open(catalog_root).unwrap();
        catalog.initialize().unwrap();

        // Create and persist an asset
        let mut asset = Asset::new(AssetType::Image, "sha256:tag_env");
        asset.tags = vec!["existing".to_string()];
        catalog.insert_asset(&asset).unwrap();

        let store = MetadataStore::new(catalog_root);
        store.save(&asset).unwrap();

        (dir, asset.id.to_string())
    }

    use crate::models::{Variant, VariantRole};

    /// Set up a temp catalog with two assets, each with one variant, for group tests.
    /// Returns (dir, hash1, hash2, asset_id1, asset_id2).
    fn setup_group_env() -> (tempfile::TempDir, String, String, String, String) {
        let dir = tempfile::tempdir().unwrap();
        let catalog_root = dir.path();

        let catalog = Catalog::open(catalog_root).unwrap();
        catalog.initialize().unwrap();
        let store = MetadataStore::new(catalog_root);

        // Create first asset (older)
        let mut asset1 = Asset::new(AssetType::Image, "sha256:hash1");
        asset1.created_at = chrono::Utc::now() - chrono::Duration::hours(2);
        asset1.tags = vec!["landscape".to_string()];
        let variant1 = Variant {
            content_hash: "sha256:hash1".to_string(),
            asset_id: asset1.id,
            role: VariantRole::Original,
            format: "arw".to_string(),
            file_size: 25_000_000,
            original_filename: "DSC_001.ARW".to_string(),
            source_metadata: Default::default(),
            locations: vec![],
        };
        asset1.variants.push(variant1.clone());
        catalog.insert_asset(&asset1).unwrap();
        catalog.insert_variant(&variant1).unwrap();
        store.save(&asset1).unwrap();

        // Create second asset (newer)
        let mut asset2 = Asset::new(AssetType::Image, "sha256:hash2");
        asset2.tags = vec!["nature".to_string()];
        let variant2 = Variant {
            content_hash: "sha256:hash2".to_string(),
            asset_id: asset2.id,
            role: VariantRole::Original,
            format: "jpg".to_string(),
            file_size: 5_000_000,
            original_filename: "DSC_001.JPG".to_string(),
            source_metadata: Default::default(),
            locations: vec![],
        };
        asset2.variants.push(variant2.clone());
        catalog.insert_asset(&asset2).unwrap();
        catalog.insert_variant(&variant2).unwrap();
        store.save(&asset2).unwrap();

        let id1 = asset1.id.to_string();
        let id2 = asset2.id.to_string();
        (dir, "sha256:hash1".to_string(), "sha256:hash2".to_string(), id1, id2)
    }

    #[test]
    fn group_two_variants_from_two_assets() {
        let (dir, hash1, hash2, id1, id2) = setup_group_env();
        let engine = QueryEngine::new(dir.path());

        let result = engine.group(&[hash1, hash2]).unwrap();

        // Target should be the older asset (asset1)
        assert_eq!(result.target_id, id1);
        assert_eq!(result.variants_moved, 1);
        assert_eq!(result.donors_removed, 1);

        // Target should now have both variants
        let details = engine.show(&id1).unwrap();
        assert_eq!(details.variants.len(), 2);

        // Donor should be gone
        assert!(engine.show(&id2).is_err());
    }

    #[test]
    fn group_already_same_asset_is_noop() {
        let (dir, hash1, _, id1, _) = setup_group_env();
        let engine = QueryEngine::new(dir.path());

        let result = engine.group(&[hash1.clone(), hash1]).unwrap();

        assert_eq!(result.target_id, id1);
        assert_eq!(result.variants_moved, 0);
        assert_eq!(result.donors_removed, 0);
    }

    #[test]
    fn group_nonexistent_hash_errors() {
        let (dir, _, _, _, _) = setup_group_env();
        let engine = QueryEngine::new(dir.path());

        let result = engine.group(&["sha256:bogus".to_string()]);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("No variant found"));
    }

    #[test]
    fn group_merges_tags() {
        let (dir, hash1, hash2, id1, _) = setup_group_env();
        let engine = QueryEngine::new(dir.path());

        engine.group(&[hash1, hash2]).unwrap();

        let details = engine.show(&id1).unwrap();
        assert!(details.tags.contains(&"landscape".to_string()));
        assert!(details.tags.contains(&"nature".to_string()));
    }

    #[test]
    fn tag_add_new() {
        let (dir, id) = setup_tag_env();
        let engine = QueryEngine::new(dir.path());

        let result = engine
            .tag(&id, &["landscape".to_string(), "nature".to_string()], false)
            .unwrap();

        assert_eq!(result.changed, vec!["landscape", "nature"]);
        assert_eq!(result.current_tags, vec!["existing", "landscape", "nature"]);
    }

    #[test]
    fn tag_add_duplicate_is_noop() {
        let (dir, id) = setup_tag_env();
        let engine = QueryEngine::new(dir.path());

        let result = engine.tag(&id, &["existing".to_string()], false).unwrap();

        assert!(result.changed.is_empty());
        assert_eq!(result.current_tags, vec!["existing"]);
    }

    #[test]
    fn tag_remove_existing() {
        let (dir, id) = setup_tag_env();
        let engine = QueryEngine::new(dir.path());

        let result = engine.tag(&id, &["existing".to_string()], true).unwrap();

        assert_eq!(result.changed, vec!["existing"]);
        assert!(result.current_tags.is_empty());
    }

    #[test]
    fn tag_remove_nonexistent_is_noop() {
        let (dir, id) = setup_tag_env();
        let engine = QueryEngine::new(dir.path());

        let result = engine.tag(&id, &["nope".to_string()], true).unwrap();

        assert!(result.changed.is_empty());
        assert_eq!(result.current_tags, vec!["existing"]);
    }

    #[test]
    fn tag_persists_to_sidecar_and_catalog() {
        let (dir, id) = setup_tag_env();
        let engine = QueryEngine::new(dir.path());

        engine.tag(&id, &["new_tag".to_string()], false).unwrap();

        // Verify sidecar
        let uuid: uuid::Uuid = id.parse().unwrap();
        let store = MetadataStore::new(dir.path());
        let asset = store.load(uuid).unwrap();
        assert!(asset.tags.contains(&"new_tag".to_string()));

        // Verify catalog
        let details = engine.show(&id).unwrap();
        assert!(details.tags.contains(&"new_tag".to_string()));
    }
}
