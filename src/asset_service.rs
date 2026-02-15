use std::path::Path;

use anyhow::Result;
use uuid::Uuid;

use crate::models::Asset;

/// A group of variants that share the same content hash.
pub struct DuplicateGroup {
    pub content_hash: String,
    pub locations: Vec<crate::models::FileLocation>,
}

/// An integrity issue found during verification.
pub struct IntegrityIssue {
    pub content_hash: String,
    pub location: crate::models::FileLocation,
    pub issue: String,
}

/// High-level operations that orchestrate the other components.
pub struct AssetService {
    _catalog_root: std::path::PathBuf,
}

impl AssetService {
    pub fn new(catalog_root: &Path) -> Self {
        Self {
            _catalog_root: catalog_root.to_path_buf(),
        }
    }

    /// Import files: hash, extract metadata, create assets/variants, write sidecars.
    pub fn import(&self, _paths: &[&Path], _volume_id: Uuid) -> Result<Vec<Asset>> {
        anyhow::bail!("not yet implemented")
    }

    /// Manually group variants into one asset.
    pub fn group(&self, _variant_hashes: &[&str]) -> Result<Asset> {
        anyhow::bail!("not yet implemented")
    }

    /// Remove a variant from a group.
    pub fn ungroup(&self, _asset_id: Uuid, _variant_hash: &str) -> Result<()> {
        anyhow::bail!("not yet implemented")
    }

    /// Add tags to an asset.
    pub fn tag(&self, _asset_id: Uuid, _tags: &[String]) -> Result<()> {
        anyhow::bail!("not yet implemented")
    }

    /// Move all variants of an asset to another volume.
    pub fn relocate(&self, _asset_id: Uuid, _target_volume: Uuid) -> Result<()> {
        anyhow::bail!("not yet implemented")
    }

    /// Find variants with the same hash on multiple locations.
    pub fn find_duplicates(&self) -> Result<Vec<DuplicateGroup>> {
        anyhow::bail!("not yet implemented")
    }

    /// Verify hashes for a volume or all online volumes.
    pub fn check_integrity(&self, _volume_id: Option<Uuid>) -> Result<Vec<IntegrityIssue>> {
        anyhow::bail!("not yet implemented")
    }
}
