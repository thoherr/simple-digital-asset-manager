use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The type of storage volume.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VolumeType {
    Local,
    External,
    Network,
}

/// The logical purpose of a volume in the storage hierarchy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VolumePurpose {
    Working,
    Archive,
    Backup,
    Cloud,
}

impl VolumePurpose {
    /// Parse a purpose string (case-insensitive).
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "working" => Some(Self::Working),
            "archive" => Some(Self::Archive),
            "backup" => Some(Self::Backup),
            "cloud" => Some(Self::Cloud),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Working => "working",
            Self::Archive => "archive",
            Self::Backup => "backup",
            Self::Cloud => "cloud",
        }
    }
}

impl std::fmt::Display for VolumePurpose {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A storage device or mount point.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Volume {
    pub id: Uuid,
    pub label: String,
    pub mount_point: PathBuf,
    pub volume_type: VolumeType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub purpose: Option<VolumePurpose>,
    #[serde(skip)]
    pub is_online: bool,
}

impl Volume {
    pub fn new(label: String, mount_point: PathBuf, volume_type: VolumeType) -> Self {
        Self {
            id: Uuid::new_v4(),
            label,
            mount_point,
            volume_type,
            purpose: None,
            is_online: false,
        }
    }
}

/// A physical location of a variant on a specific volume.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileLocation {
    pub volume_id: Uuid,
    pub relative_path: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verified_at: Option<DateTime<Utc>>,
}
