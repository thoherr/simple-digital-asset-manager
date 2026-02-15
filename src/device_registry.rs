use std::path::{Path, PathBuf};

use anyhow::Result;
use uuid::Uuid;

use crate::models::{Volume, VolumeType};

/// Manages volume registration and online/offline detection.
pub struct DeviceRegistry {
    catalog_root: PathBuf,
}

impl DeviceRegistry {
    pub fn new(catalog_root: &Path) -> Self {
        Self {
            catalog_root: catalog_root.to_path_buf(),
        }
    }

    /// Create an empty volumes.yaml file.
    pub fn init(catalog_root: &Path) -> Result<()> {
        let path = catalog_root.join("volumes.yaml");
        std::fs::write(path, "[]\n")?;
        Ok(())
    }

    fn volumes_path(&self) -> PathBuf {
        self.catalog_root.join("volumes.yaml")
    }

    fn load(&self) -> Result<Vec<Volume>> {
        let contents = std::fs::read_to_string(self.volumes_path())?;
        let volumes: Vec<Volume> = serde_yaml::from_str(&contents)?;
        Ok(volumes)
    }

    fn save(&self, volumes: &[Volume]) -> Result<()> {
        let yaml = serde_yaml::to_string(volumes)?;
        std::fs::write(self.volumes_path(), yaml)?;
        Ok(())
    }

    /// Register a new volume.
    pub fn register(
        &self,
        label: &str,
        mount_point: &Path,
        volume_type: VolumeType,
    ) -> Result<Volume> {
        let mut volumes = self.load()?;

        if volumes.iter().any(|v| v.label == label) {
            anyhow::bail!("A volume with label '{}' already exists", label);
        }

        let volume = Volume::new(label.to_string(), mount_point.to_path_buf(), volume_type);
        volumes.push(volume.clone());
        self.save(&volumes)?;

        Ok(volume)
    }

    /// List all volumes with online/offline status.
    pub fn list(&self) -> Result<Vec<Volume>> {
        let mut volumes = self.load()?;
        for v in &mut volumes {
            v.is_online = v.mount_point.exists();
        }
        Ok(volumes)
    }

    /// Find the volume whose mount_point is a prefix of the given path.
    /// Uses longest prefix match if multiple volumes match.
    pub fn find_volume_for_path(&self, path: &Path) -> Result<Volume> {
        let volumes = self.list()?;
        let mut best: Option<&Volume> = None;
        let mut best_len = 0;

        for v in &volumes {
            if path.starts_with(&v.mount_point) {
                let len = v.mount_point.as_os_str().len();
                if len > best_len {
                    best = Some(v);
                    best_len = len;
                }
            }
        }

        best.cloned().ok_or_else(|| {
            anyhow::anyhow!(
                "No registered volume contains path: {}",
                path.display()
            )
        })
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn setup() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        DeviceRegistry::init(dir.path()).unwrap();
        dir
    }

    #[test]
    fn register_creates_volume_and_persists() {
        let dir = setup();
        let registry = DeviceRegistry::new(dir.path());

        let vol = registry
            .register("Photos", Path::new("/mnt/photos"), VolumeType::Local)
            .unwrap();

        assert_eq!(vol.label, "Photos");
        assert_eq!(vol.mount_point, Path::new("/mnt/photos"));
        assert_eq!(vol.volume_type, VolumeType::Local);

        // Verify it was persisted by loading from disk
        let volumes = registry.load().unwrap();
        assert_eq!(volumes.len(), 1);
        assert_eq!(volumes[0].id, vol.id);
    }

    #[test]
    fn register_rejects_duplicate_label() {
        let dir = setup();
        let registry = DeviceRegistry::new(dir.path());

        registry
            .register("Backup", Path::new("/mnt/backup"), VolumeType::External)
            .unwrap();

        let err = registry
            .register("Backup", Path::new("/mnt/other"), VolumeType::Local)
            .unwrap_err();

        assert!(err.to_string().contains("already exists"));
    }

    #[test]
    fn register_multiple_volumes() {
        let dir = setup();
        let registry = DeviceRegistry::new(dir.path());

        let v1 = registry
            .register("Drive A", Path::new("/mnt/a"), VolumeType::Local)
            .unwrap();
        let v2 = registry
            .register("Drive B", Path::new("/mnt/b"), VolumeType::External)
            .unwrap();

        assert_ne!(v1.id, v2.id);

        let volumes = registry.load().unwrap();
        assert_eq!(volumes.len(), 2);
    }

    #[test]
    fn list_detects_online_offline() {
        let dir = setup();
        let registry = DeviceRegistry::new(dir.path());

        // Register a volume pointing at a path inside the temp dir (exists)
        let online_path = dir.path().join("online-vol");
        std::fs::create_dir(&online_path).unwrap();
        registry
            .register("Online", &online_path, VolumeType::Local)
            .unwrap();

        // Register a volume pointing at a path that doesn't exist
        registry
            .register("Offline", Path::new("/nonexistent/volume"), VolumeType::External)
            .unwrap();

        let volumes = registry.list().unwrap();
        let online = volumes.iter().find(|v| v.label == "Online").unwrap();
        let offline = volumes.iter().find(|v| v.label == "Offline").unwrap();

        assert!(online.is_online);
        assert!(!offline.is_online);
    }
}
