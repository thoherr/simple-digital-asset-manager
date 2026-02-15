use std::path::Path;

use anyhow::Result;

/// SQLite-backed local catalog for fast queries. This is a derived cache,
/// not the source of truth (sidecar files are).
pub struct Catalog {
    _db_path: std::path::PathBuf,
}

impl Catalog {
    pub fn open(catalog_root: &Path) -> Result<Self> {
        Ok(Self {
            _db_path: catalog_root.join("catalog.db"),
        })
    }

    /// Initialize the database schema.
    pub fn initialize(&self) -> Result<()> {
        anyhow::bail!("not yet implemented")
    }

    /// Rebuild the entire catalog from sidecar files.
    pub fn rebuild(&self) -> Result<()> {
        anyhow::bail!("not yet implemented")
    }
}
