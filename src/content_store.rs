use std::fs::File;
use std::io::Read;
use std::path::Path;

use anyhow::Result;
use sha2::{Digest, Sha256};

use crate::models::{FileLocation, Volume};

/// Manages file identity, deduplication, and physical location tracking.
pub struct ContentStore {
    catalog_root: std::path::PathBuf,
}

impl ContentStore {
    pub fn new(catalog_root: &Path) -> Self {
        Self {
            catalog_root: catalog_root.to_path_buf(),
        }
    }

    /// Hash a file and return the SHA-256 content hash as "sha256:<hex>".
    /// Referenced mode: no file copying is performed.
    pub fn ingest(&self, path: &Path, _volume: &Volume) -> Result<String> {
        let mut file = File::open(path)?;
        let mut hasher = Sha256::new();
        let mut buffer = [0u8; 8192];
        loop {
            let bytes_read = file.read(&mut buffer)?;
            if bytes_read == 0 {
                break;
            }
            hasher.update(&buffer[..bytes_read]);
        }
        let hash = hasher.finalize();
        Ok(format!("sha256:{:x}", hash))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ingest_returns_sha256_hash() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.txt");
        std::fs::write(&file_path, "hello world").unwrap();

        let store = ContentStore::new(dir.path());
        let volume = Volume::new(
            "test".to_string(),
            dir.path().to_path_buf(),
            crate::models::VolumeType::Local,
        );
        let hash = store.ingest(&file_path, &volume).unwrap();
        assert!(hash.starts_with("sha256:"));
        // Known SHA-256 of "hello world"
        assert_eq!(
            hash,
            "sha256:b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
    }
}
