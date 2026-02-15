use std::path::Path;

use anyhow::Result;

/// Creates and caches thumbnails for browsing.
pub struct PreviewGenerator {
    _preview_dir: std::path::PathBuf,
}

impl PreviewGenerator {
    pub fn new(catalog_root: &Path) -> Self {
        Self {
            _preview_dir: catalog_root.join("previews"),
        }
    }

    /// Generate a preview for a file, returning the path to the preview image.
    pub fn generate(&self, _content_hash: &str, _source_path: &Path) -> Result<std::path::PathBuf> {
        anyhow::bail!("not yet implemented")
    }
}
