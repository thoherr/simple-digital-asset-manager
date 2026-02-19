use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Preview output format.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PreviewFormat {
    Jpeg,
    Webp,
}

impl Default for PreviewFormat {
    fn default() -> Self {
        Self::Jpeg
    }
}

impl PreviewFormat {
    /// File extension for this format.
    pub fn extension(&self) -> &'static str {
        match self {
            PreviewFormat::Jpeg => "jpg",
            PreviewFormat::Webp => "webp",
        }
    }
}

/// Preview generation configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PreviewConfig {
    #[serde(default = "default_max_edge")]
    pub max_edge: u32,
    #[serde(default)]
    pub format: PreviewFormat,
    #[serde(default = "default_quality")]
    pub quality: u8,
}

fn default_max_edge() -> u32 {
    800
}

fn default_quality() -> u8 {
    85
}

impl Default for PreviewConfig {
    fn default() -> Self {
        Self {
            max_edge: 800,
            format: PreviewFormat::Jpeg,
            quality: 85,
        }
    }
}

/// Web server configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ServeConfig {
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default = "default_bind")]
    pub bind: String,
}

fn default_port() -> u16 {
    8080
}

fn default_bind() -> String {
    "127.0.0.1".to_string()
}

impl Default for ServeConfig {
    fn default() -> Self {
        Self {
            port: 8080,
            bind: "127.0.0.1".to_string(),
        }
    }
}

/// Import behavior configuration.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ImportConfig {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub auto_tags: Vec<String>,
}

fn is_default_preview(p: &PreviewConfig) -> bool {
    *p == PreviewConfig::default()
}

fn is_default_serve(s: &ServeConfig) -> bool {
    *s == ServeConfig::default()
}

fn is_default_import(i: &ImportConfig) -> bool {
    *i == ImportConfig::default()
}

/// Catalog configuration stored in dam.toml.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_volume: Option<Uuid>,
    #[serde(default, skip_serializing_if = "is_default_preview")]
    pub preview: PreviewConfig,
    #[serde(default, skip_serializing_if = "is_default_serve")]
    pub serve: ServeConfig,
    #[serde(default, skip_serializing_if = "is_default_import")]
    pub import: ImportConfig,
}

impl Default for CatalogConfig {
    fn default() -> Self {
        Self {
            default_volume: None,
            preview: PreviewConfig::default(),
            serve: ServeConfig::default(),
            import: ImportConfig::default(),
        }
    }
}

impl CatalogConfig {
    /// Load configuration from a dam.toml file.
    pub fn load(catalog_root: &Path) -> Result<Self> {
        let path = catalog_root.join("dam.toml");
        if path.exists() {
            let contents = std::fs::read_to_string(&path)?;
            let config: Self = toml::from_str(&contents)?;
            config.validate()?;
            Ok(config)
        } else {
            Ok(Self::default())
        }
    }

    /// Save configuration to a dam.toml file.
    pub fn save(&self, catalog_root: &Path) -> Result<()> {
        let path = catalog_root.join("dam.toml");
        let contents = toml::to_string_pretty(self)?;
        std::fs::write(path, contents)?;
        Ok(())
    }

    /// Validate configuration values.
    pub fn validate(&self) -> Result<()> {
        if self.preview.max_edge == 0 {
            anyhow::bail!("preview.max_edge must be greater than 0");
        }
        if self.preview.quality == 0 || self.preview.quality > 100 {
            anyhow::bail!("preview.quality must be between 1 and 100");
        }
        Ok(())
    }
}

/// Find the catalog root by looking for dam.toml in current and parent directories.
pub fn find_catalog_root() -> Result<PathBuf> {
    let mut dir = std::env::current_dir()?;
    loop {
        if dir.join("dam.toml").exists() {
            return Ok(dir);
        }
        if !dir.pop() {
            anyhow::bail!("No dam catalog found. Run `dam init` to create one.");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serialize_default_config() {
        let config = CatalogConfig::default();
        let toml_str = toml::to_string_pretty(&config).unwrap();
        // Default config should be empty (all fields skipped)
        assert!(toml_str.trim().is_empty(), "got: {toml_str}");
    }

    #[test]
    fn serialize_with_default_volume() {
        let uuid = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let config = CatalogConfig {
            default_volume: Some(uuid),
            ..Default::default()
        };
        let toml_str = toml::to_string_pretty(&config).unwrap();
        assert!(toml_str.contains("default_volume = \"550e8400-e29b-41d4-a716-446655440000\""));
    }

    #[test]
    fn parse_empty_config() {
        let config: CatalogConfig = toml::from_str("").unwrap();
        assert!(config.default_volume.is_none());
        assert_eq!(config.preview, PreviewConfig::default());
        assert_eq!(config.serve, ServeConfig::default());
        assert_eq!(config.import, ImportConfig::default());
    }

    #[test]
    fn parse_comment_only() {
        let config: CatalogConfig = toml::from_str("# dam catalog configuration\n").unwrap();
        assert!(config.default_volume.is_none());
    }

    #[test]
    fn parse_with_default_volume() {
        let input = "default_volume = \"550e8400-e29b-41d4-a716-446655440000\"\n";
        let config: CatalogConfig = toml::from_str(input).unwrap();
        assert_eq!(
            config.default_volume.unwrap().to_string(),
            "550e8400-e29b-41d4-a716-446655440000"
        );
    }

    #[test]
    fn parse_default_volume_only_backward_compat() {
        // Old dam.toml files that only had default_volume should still parse
        let input = "default_volume = \"550e8400-e29b-41d4-a716-446655440000\"\n";
        let config: CatalogConfig = toml::from_str(input).unwrap();
        assert!(config.default_volume.is_some());
        assert_eq!(config.preview, PreviewConfig::default());
    }

    #[test]
    fn round_trip() {
        let uuid = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let original = CatalogConfig {
            default_volume: Some(uuid),
            ..Default::default()
        };
        let toml_str = toml::to_string_pretty(&original).unwrap();
        let parsed: CatalogConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(original.default_volume, parsed.default_volume);
    }

    #[test]
    fn round_trip_none() {
        let original = CatalogConfig::default();
        let toml_str = toml::to_string_pretty(&original).unwrap();
        let parsed: CatalogConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(original.default_volume, parsed.default_volume);
    }

    #[test]
    fn parse_invalid_uuid_errors() {
        let input = "default_volume = \"not-a-uuid\"\n";
        assert!(toml::from_str::<CatalogConfig>(input).is_err());
    }

    #[test]
    fn parse_full_config() {
        let input = r#"
default_volume = "550e8400-e29b-41d4-a716-446655440000"

[preview]
max_edge = 1200
format = "webp"
quality = 90

[serve]
port = 9090
bind = "0.0.0.0"

[import]
exclude = ["Thumbs.db", "*.tmp", ".DS_Store"]
auto_tags = ["inbox", "unreviewed"]
"#;
        let config: CatalogConfig = toml::from_str(input).unwrap();
        assert!(config.default_volume.is_some());
        assert_eq!(config.preview.max_edge, 1200);
        assert_eq!(config.preview.format, PreviewFormat::Webp);
        assert_eq!(config.preview.quality, 90);
        assert_eq!(config.serve.port, 9090);
        assert_eq!(config.serve.bind, "0.0.0.0");
        assert_eq!(config.import.exclude, vec!["Thumbs.db", "*.tmp", ".DS_Store"]);
        assert_eq!(config.import.auto_tags, vec!["inbox", "unreviewed"]);
    }

    #[test]
    fn parse_partial_config_missing_sections() {
        let input = r#"
[preview]
max_edge = 1000
"#;
        let config: CatalogConfig = toml::from_str(input).unwrap();
        assert!(config.default_volume.is_none());
        assert_eq!(config.preview.max_edge, 1000);
        assert_eq!(config.preview.format, PreviewFormat::Jpeg); // default
        assert_eq!(config.preview.quality, 85); // default
        assert_eq!(config.serve, ServeConfig::default());
        assert_eq!(config.import, ImportConfig::default());
    }

    #[test]
    fn full_round_trip() {
        let original = CatalogConfig {
            default_volume: None,
            preview: PreviewConfig {
                max_edge: 1200,
                format: PreviewFormat::Webp,
                quality: 90,
            },
            serve: ServeConfig {
                port: 9090,
                bind: "0.0.0.0".to_string(),
            },
            import: ImportConfig {
                exclude: vec!["*.tmp".to_string()],
                auto_tags: vec!["test".to_string()],
            },
        };
        let toml_str = toml::to_string_pretty(&original).unwrap();
        let parsed: CatalogConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.preview, original.preview);
        assert_eq!(parsed.serve, original.serve);
        assert_eq!(parsed.import, original.import);
    }

    #[test]
    fn validate_zero_max_edge_errors() {
        let config = CatalogConfig {
            preview: PreviewConfig {
                max_edge: 0,
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn validate_zero_quality_errors() {
        let config = CatalogConfig {
            preview: PreviewConfig {
                quality: 0,
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn validate_quality_over_100_errors() {
        let config = CatalogConfig {
            preview: PreviewConfig {
                quality: 101,
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn validate_default_passes() {
        CatalogConfig::default().validate().unwrap();
    }

    #[test]
    fn preview_format_extension() {
        assert_eq!(PreviewFormat::Jpeg.extension(), "jpg");
        assert_eq!(PreviewFormat::Webp.extension(), "webp");
    }

    #[test]
    fn load_creates_default_when_no_file() {
        let dir = tempfile::tempdir().unwrap();
        let config = CatalogConfig::load(dir.path()).unwrap();
        assert!(config.default_volume.is_none());
        assert_eq!(config.preview, PreviewConfig::default());
    }

    #[test]
    fn save_and_load_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let original = CatalogConfig {
            default_volume: None,
            preview: PreviewConfig {
                max_edge: 1200,
                format: PreviewFormat::Webp,
                quality: 90,
            },
            serve: ServeConfig::default(),
            import: ImportConfig {
                exclude: vec!["*.tmp".to_string()],
                auto_tags: vec![],
            },
        };
        original.save(dir.path()).unwrap();
        let loaded = CatalogConfig::load(dir.path()).unwrap();
        assert_eq!(loaded.preview, original.preview);
        assert_eq!(loaded.import.exclude, original.import.exclude);
    }
}
