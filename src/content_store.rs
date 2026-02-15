use std::path::Path;

use anyhow::Result;

use crate::models::{FileLocation, Volume};

/// Manages file identity, deduplication, and physical location tracking.
pub struct ContentStore {
    _catalog_root: std::path::PathBuf,
}

impl ContentStore {
    pub fn new(catalog_root: &Path) -> Self {
        Self {
            _catalog_root: catalog_root.to_path_buf(),
        }
    }

    /// Hash a file and register it. Returns the SHA-256 content hash.
    pub fn ingest(&self, _path: &Path, _volume: &Volume) -> Result<String> {
        anyhow::bail!("not yet implemented")
    }

    /// Find all known locations of a file by its content hash.
    pub fn locate(&self, _content_hash: &str) -> Result<Vec<FileLocation>> {
        anyhow::bail!("not yet implemented")
    }

    /// Move/copy a file between volumes, updating locations.
    pub fn relocate(
        &self,
        _content_hash: &str,
        _from_volume: &Volume,
        _to_volume: &Volume,
    ) -> Result<()> {
        anyhow::bail!("not yet implemented")
    }

    /// Re-hash file at location and confirm integrity.
    pub fn verify(&self, _content_hash: &str, _location: &FileLocation) -> Result<bool> {
        anyhow::bail!("not yet implemented")
    }

    /// Unregister a location (file moved/deleted externally).
    pub fn remove_location(
        &self,
        _content_hash: &str,
        _location: &FileLocation,
    ) -> Result<()> {
        anyhow::bail!("not yet implemented")
    }
}
