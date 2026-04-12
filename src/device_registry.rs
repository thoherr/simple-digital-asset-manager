use std::path::{Path, PathBuf};

use anyhow::Result;
use uuid::Uuid;

use crate::models::{Volume, VolumePurpose, VolumeType};

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
        purpose: Option<VolumePurpose>,
    ) -> Result<Volume> {
        let mut volumes = self.load()?;

        if volumes.iter().any(|v| v.label == label) {
            anyhow::bail!("a volume with label '{}' already exists", label);
        }

        let mut volume = Volume::new(label.to_string(), mount_point.to_path_buf(), volume_type);
        volume.purpose = purpose;
        volumes.push(volume.clone());
        self.save(&volumes)?;

        Ok(volume)
    }

    /// Set or clear the purpose of an existing volume.
    pub fn set_purpose(&self, label_or_id: &str, purpose: Option<VolumePurpose>) -> Result<Volume> {
        let mut volumes = self.load()?;

        let vol = volumes.iter_mut().find(|v| {
            v.label == label_or_id
                || uuid::Uuid::parse_str(label_or_id)
                    .map(|u| v.id == u)
                    .unwrap_or(false)
        });

        match vol {
            Some(v) => {
                v.purpose = purpose;
                let result = v.clone();
                self.save(&volumes)?;
                Ok(result)
            }
            None => anyhow::bail!("no volume found matching '{}'", label_or_id),
        }
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

    /// Find a volume by label or UUID string.
    pub fn resolve_volume(&self, label_or_id: &str) -> Result<Volume> {
        let volumes = self.list()?;

        // Try UUID match first
        if let Ok(uuid) = uuid::Uuid::parse_str(label_or_id) {
            if let Some(v) = volumes.iter().find(|v| v.id == uuid) {
                return Ok(v.clone());
            }
        }

        // Fall back to label match
        if let Some(v) = volumes.iter().find(|v| v.label == label_or_id) {
            return Ok(v.clone());
        }

        let labels: Vec<&str> = volumes.iter().map(|v| v.label.as_str()).collect();
        anyhow::bail!(
            "no volume found matching '{}'. Known volumes: {}",
            label_or_id,
            if labels.is_empty() {
                "(none)".to_string()
            } else {
                labels.join(", ")
            }
        )
    }

    /// Remove a volume by label or UUID. Returns the removed volume.
    pub fn remove(&self, label_or_id: &str) -> Result<Volume> {
        let mut volumes = self.load()?;

        let idx = volumes.iter().position(|v| {
            v.label == label_or_id
                || uuid::Uuid::parse_str(label_or_id)
                    .map(|u| v.id == u)
                    .unwrap_or(false)
        });

        match idx {
            Some(i) => {
                let removed = volumes.remove(i);
                self.save(&volumes)?;
                Ok(removed)
            }
            None => anyhow::bail!("no volume found matching '{}'", label_or_id),
        }
    }

    /// Rename a volume by label or UUID.
    pub fn rename(&self, label_or_id: &str, new_label: &str) -> Result<()> {
        let mut volumes = self.load()?;

        let vol = volumes.iter_mut().find(|v| {
            v.label == label_or_id
                || uuid::Uuid::parse_str(label_or_id)
                    .map(|u| v.id == u)
                    .unwrap_or(false)
        });

        match vol {
            Some(v) => {
                v.label = new_label.to_string();
                self.save(&volumes)?;
                Ok(())
            }
            None => anyhow::bail!("no volume found matching '{}'", label_or_id),
        }
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
            .register("Photos", Path::new("/mnt/photos"), VolumeType::Local, None)
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
            .register("Backup", Path::new("/mnt/backup"), VolumeType::External, None)
            .unwrap();

        let err = registry
            .register("Backup", Path::new("/mnt/other"), VolumeType::Local, None)
            .unwrap_err();

        assert!(err.to_string().contains("already exists"));
    }

    #[test]
    fn register_multiple_volumes() {
        let dir = setup();
        let registry = DeviceRegistry::new(dir.path());

        let v1 = registry
            .register("Drive A", Path::new("/mnt/a"), VolumeType::Local, None)
            .unwrap();
        let v2 = registry
            .register("Drive B", Path::new("/mnt/b"), VolumeType::External, None)
            .unwrap();

        assert_ne!(v1.id, v2.id);

        let volumes = registry.load().unwrap();
        assert_eq!(volumes.len(), 2);
    }

    #[test]
    fn resolve_volume_by_label() {
        let dir = setup();
        let registry = DeviceRegistry::new(dir.path());
        registry
            .register("Photos", Path::new("/mnt/photos"), VolumeType::Local, None)
            .unwrap();

        let vol = registry.resolve_volume("Photos").unwrap();
        assert_eq!(vol.label, "Photos");
    }

    #[test]
    fn resolve_volume_by_uuid() {
        let dir = setup();
        let registry = DeviceRegistry::new(dir.path());
        let registered = registry
            .register("Photos", Path::new("/mnt/photos"), VolumeType::Local, None)
            .unwrap();

        let vol = registry
            .resolve_volume(&registered.id.to_string())
            .unwrap();
        assert_eq!(vol.id, registered.id);
        assert_eq!(vol.label, "Photos");
    }

    #[test]
    fn resolve_volume_unknown_errors() {
        let dir = setup();
        let registry = DeviceRegistry::new(dir.path());
        registry
            .register("Photos", Path::new("/mnt/photos"), VolumeType::Local, None)
            .unwrap();

        let err = registry.resolve_volume("Nonexistent").unwrap_err();
        assert!(err.to_string().contains("no volume found"));
    }

    #[test]
    fn list_detects_online_offline() {
        let dir = setup();
        let registry = DeviceRegistry::new(dir.path());

        // Register a volume pointing at a path inside the temp dir (exists)
        let online_path = dir.path().join("online-vol");
        std::fs::create_dir(&online_path).unwrap();
        registry
            .register("Online", &online_path, VolumeType::Local, None)
            .unwrap();

        // Register a volume pointing at a path that doesn't exist
        registry
            .register("Offline", Path::new("/nonexistent/volume"), VolumeType::External, None)
            .unwrap();

        let volumes = registry.list().unwrap();
        let online = volumes.iter().find(|v| v.label == "Online").unwrap();
        let offline = volumes.iter().find(|v| v.label == "Offline").unwrap();

        assert!(online.is_online);
        assert!(!offline.is_online);
    }

    #[test]
    fn register_with_purpose() {
        let dir = setup();
        let registry = DeviceRegistry::new(dir.path());

        let vol = registry
            .register(
                "Backup",
                Path::new("/mnt/backup"),
                VolumeType::External,
                Some(VolumePurpose::Backup),
            )
            .unwrap();

        assert_eq!(vol.purpose, Some(VolumePurpose::Backup));

        // Verify persisted
        let volumes = registry.load().unwrap();
        assert_eq!(volumes[0].purpose, Some(VolumePurpose::Backup));
    }

    #[test]
    fn register_without_purpose() {
        let dir = setup();
        let registry = DeviceRegistry::new(dir.path());

        let vol = registry
            .register("Local", Path::new("/mnt/local"), VolumeType::Local, None)
            .unwrap();

        assert_eq!(vol.purpose, None);
    }

    #[test]
    fn set_purpose_by_label() {
        let dir = setup();
        let registry = DeviceRegistry::new(dir.path());

        registry
            .register("Photos", Path::new("/mnt/photos"), VolumeType::Local, None)
            .unwrap();

        let vol = registry.set_purpose("Photos", Some(VolumePurpose::Archive)).unwrap();
        assert_eq!(vol.purpose, Some(VolumePurpose::Archive));

        // Verify persisted
        let volumes = registry.load().unwrap();
        assert_eq!(volumes[0].purpose, Some(VolumePurpose::Archive));
    }

    #[test]
    fn set_purpose_clear() {
        let dir = setup();
        let registry = DeviceRegistry::new(dir.path());

        registry
            .register(
                "Backup",
                Path::new("/mnt/backup"),
                VolumeType::External,
                Some(VolumePurpose::Backup),
            )
            .unwrap();

        let vol = registry.set_purpose("Backup", None).unwrap();
        assert_eq!(vol.purpose, None);
    }

    #[test]
    fn set_purpose_unknown_volume_errors() {
        let dir = setup();
        let registry = DeviceRegistry::new(dir.path());

        let err = registry
            .set_purpose("Nonexistent", Some(VolumePurpose::Working))
            .unwrap_err();
        assert!(err.to_string().contains("no volume found"));
    }

    #[test]
    fn volume_purpose_parse() {
        assert_eq!(VolumePurpose::parse("working"), Some(VolumePurpose::Working));
        assert_eq!(VolumePurpose::parse("Archive"), Some(VolumePurpose::Archive));
        assert_eq!(VolumePurpose::parse("BACKUP"), Some(VolumePurpose::Backup));
        assert_eq!(VolumePurpose::parse("cloud"), Some(VolumePurpose::Cloud));
        assert_eq!(VolumePurpose::parse("Media"), Some(VolumePurpose::Media));
        assert_eq!(VolumePurpose::parse("invalid"), None);
    }
}
