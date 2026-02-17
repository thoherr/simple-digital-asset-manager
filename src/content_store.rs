use std::fs::File;
use std::io::Read;
use std::path::Path;

use anyhow::Result;
use sha2::{Digest, Sha256};

use crate::models::Volume;

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
        self.hash_file(path)
    }

    /// Hash a file and return the SHA-256 content hash as "sha256:<hex>".
    pub fn hash_file(&self, path: &Path) -> Result<String> {
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

    /// Copy a file from source to dest, then verify the copy matches the expected hash.
    /// Creates parent directories as needed. On hash mismatch, deletes the bad copy.
    pub fn copy_and_verify(&self, source: &Path, dest: &Path, expected_hash: &str) -> Result<()> {
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }

        std::fs::copy(source, dest)?;

        let actual_hash = self.hash_file(dest)?;
        if actual_hash != expected_hash {
            let _ = std::fs::remove_file(dest);
            anyhow::bail!(
                "Integrity check failed for {}: expected {}, got {}",
                dest.display(),
                expected_hash,
                actual_hash
            );
        }

        Ok(())
    }

    /// Re-hash file at location and confirm integrity.
    pub fn verify(&self, _content_hash: &str, _location: &crate::models::FileLocation) -> Result<bool> {
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

    #[test]
    fn hash_file_returns_correct_sha256() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("hash_test.txt");
        std::fs::write(&file_path, "hello world").unwrap();

        let store = ContentStore::new(dir.path());
        let hash = store.hash_file(&file_path).unwrap();
        assert_eq!(
            hash,
            "sha256:b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
    }

    #[test]
    fn copy_and_verify_succeeds() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.txt");
        std::fs::write(&source, "copy me").unwrap();

        let store = ContentStore::new(dir.path());
        let hash = store.hash_file(&source).unwrap();

        let dest = dir.path().join("dest.txt");
        store.copy_and_verify(&source, &dest, &hash).unwrap();

        assert!(dest.exists());
        assert_eq!(std::fs::read_to_string(&dest).unwrap(), "copy me");
    }

    #[test]
    fn copy_and_verify_creates_parent_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.txt");
        std::fs::write(&source, "nested copy").unwrap();

        let store = ContentStore::new(dir.path());
        let hash = store.hash_file(&source).unwrap();

        let dest = dir.path().join("a/b/c/dest.txt");
        store.copy_and_verify(&source, &dest, &hash).unwrap();

        assert!(dest.exists());
        assert_eq!(std::fs::read_to_string(&dest).unwrap(), "nested copy");
    }

    #[test]
    fn copy_and_verify_fails_on_hash_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.txt");
        std::fs::write(&source, "some content").unwrap();

        let store = ContentStore::new(dir.path());
        let dest = dir.path().join("dest.txt");
        let err = store
            .copy_and_verify(&source, &dest, "sha256:0000000000000000")
            .unwrap_err();

        assert!(err.to_string().contains("Integrity check failed"));
        // Bad copy should be cleaned up
        assert!(!dest.exists());
    }
}
