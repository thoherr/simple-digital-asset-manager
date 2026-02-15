use std::path::Path;

use anyhow::Result;
use uuid::Uuid;

use crate::models::{Volume, VolumeType};

/// Manages volume registration and online/offline detection.
pub struct DeviceRegistry {
    _catalog_root: std::path::PathBuf,
}

impl DeviceRegistry {
    pub fn new(catalog_root: &Path) -> Self {
        Self {
            _catalog_root: catalog_root.to_path_buf(),
        }
    }

    /// Register a new volume.
    pub fn register(
        &self,
        _label: &str,
        _mount_point: &Path,
        _volume_type: VolumeType,
    ) -> Result<Volume> {
        anyhow::bail!("not yet implemented")
    }

    /// List all volumes with online/offline status.
    pub fn list(&self) -> Result<Vec<Volume>> {
        anyhow::bail!("not yet implemented")
    }

    /// Check which mount points are currently available.
    pub fn detect_online(&self) -> Result<()> {
        anyhow::bail!("not yet implemented")
    }

    /// Scan a volume for new/changed/deleted files.
    pub fn scan(&self, _volume_id: Uuid) -> Result<()> {
        anyhow::bail!("not yet implemented")
    }
}
