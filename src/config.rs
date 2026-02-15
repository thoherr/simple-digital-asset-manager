use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Catalog configuration stored in dam.toml.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_volume: Option<Uuid>,
}

impl Default for CatalogConfig {
    fn default() -> Self {
        Self {
            default_volume: None,
        }
    }
}

impl CatalogConfig {
    /// Load configuration from a dam.toml file.
    pub fn load(catalog_root: &Path) -> Result<Self> {
        let path = catalog_root.join("dam.toml");
        if path.exists() {
            let contents = std::fs::read_to_string(&path)?;
            let config: Self = toml_minimal_parse(&contents)?;
            Ok(config)
        } else {
            Ok(Self::default())
        }
    }

    /// Save configuration to a dam.toml file.
    pub fn save(&self, catalog_root: &Path) -> Result<()> {
        let path = catalog_root.join("dam.toml");
        let contents = toml_minimal_serialize(self)?;
        std::fs::write(path, contents)?;
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

// Simple TOML handling without a full toml crate dependency.
// We only have one optional field, so hand-written parsing is fine.
fn toml_minimal_parse(contents: &str) -> Result<CatalogConfig> {
    let mut default_volume = None;
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(rest) = line.strip_prefix("default_volume") {
            let rest = rest.trim();
            if let Some(value) = rest.strip_prefix('=') {
                let value = value.trim().trim_matches('"');
                default_volume = Some(value.parse::<Uuid>()?);
            }
        }
    }
    Ok(CatalogConfig { default_volume })
}

fn toml_minimal_serialize(config: &CatalogConfig) -> Result<String> {
    let mut out = String::from("# dam catalog configuration\n");
    if let Some(vol) = &config.default_volume {
        out.push_str(&format!("default_volume = \"{vol}\"\n"));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serialize_default_config() {
        let config = CatalogConfig::default();
        let toml = toml_minimal_serialize(&config).unwrap();
        assert_eq!(toml, "# dam catalog configuration\n");
    }

    #[test]
    fn serialize_with_default_volume() {
        let uuid = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let config = CatalogConfig {
            default_volume: Some(uuid),
        };
        let toml = toml_minimal_serialize(&config).unwrap();
        assert!(toml.contains("default_volume = \"550e8400-e29b-41d4-a716-446655440000\""));
    }

    #[test]
    fn parse_empty_config() {
        let config = toml_minimal_parse("").unwrap();
        assert!(config.default_volume.is_none());
    }

    #[test]
    fn parse_comment_only() {
        let config = toml_minimal_parse("# dam catalog configuration\n").unwrap();
        assert!(config.default_volume.is_none());
    }

    #[test]
    fn parse_with_default_volume() {
        let input = "default_volume = \"550e8400-e29b-41d4-a716-446655440000\"\n";
        let config = toml_minimal_parse(input).unwrap();
        assert_eq!(
            config.default_volume.unwrap().to_string(),
            "550e8400-e29b-41d4-a716-446655440000"
        );
    }

    #[test]
    fn parse_with_whitespace() {
        let input = "  default_volume  =  \"550e8400-e29b-41d4-a716-446655440000\"  \n";
        let config = toml_minimal_parse(input).unwrap();
        assert!(config.default_volume.is_some());
    }

    #[test]
    fn round_trip() {
        let uuid = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let original = CatalogConfig {
            default_volume: Some(uuid),
        };
        let toml = toml_minimal_serialize(&original).unwrap();
        let parsed = toml_minimal_parse(&toml).unwrap();
        assert_eq!(original.default_volume, parsed.default_volume);
    }

    #[test]
    fn round_trip_none() {
        let original = CatalogConfig::default();
        let toml = toml_minimal_serialize(&original).unwrap();
        let parsed = toml_minimal_parse(&toml).unwrap();
        assert_eq!(original.default_volume, parsed.default_volume);
    }

    #[test]
    fn parse_invalid_uuid_errors() {
        let input = "default_volume = \"not-a-uuid\"\n";
        assert!(toml_minimal_parse(input).is_err());
    }
}
