use std::path::Path;

use anyhow::Result;

use crate::catalog::{AssetDetails, Catalog, SearchRow};
use crate::metadata_store::MetadataStore;

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

        for token in query.split_whitespace() {
            if let Some(value) = token.strip_prefix("type:") {
                asset_type = Some(value.to_string());
            } else if let Some(value) = token.strip_prefix("tag:") {
                tag = Some(value.to_string());
            } else if let Some(value) = token.strip_prefix("format:") {
                format = Some(value.to_string());
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
        let mut asset = Asset::new(AssetType::Image);
        asset.tags = vec!["existing".to_string()];
        catalog.insert_asset(&asset).unwrap();

        let store = MetadataStore::new(catalog_root);
        store.save(&asset).unwrap();

        (dir, asset.id.to_string())
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
