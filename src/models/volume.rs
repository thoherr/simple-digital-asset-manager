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

/// A storage device or mount point.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Volume {
    pub id: Uuid,
    pub label: String,
    pub mount_point: PathBuf,
    pub volume_type: VolumeType,
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
