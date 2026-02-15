use std::path::Path;

use anyhow::Result;

use crate::catalog::{Catalog, SearchRow};

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
}
