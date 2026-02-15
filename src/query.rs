use std::path::Path;

use anyhow::Result;

use crate::models::Asset;

/// Search and filter assets via the SQLite catalog.
pub struct QueryEngine {
    _catalog_root: std::path::PathBuf,
}

impl QueryEngine {
    pub fn new(catalog_root: &Path) -> Self {
        Self {
            _catalog_root: catalog_root.to_path_buf(),
        }
    }

    /// Search assets by a free-text query.
    pub fn search(&self, _query: &str) -> Result<Vec<Asset>> {
        anyhow::bail!("not yet implemented")
    }
}
