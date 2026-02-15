use std::path::Path;

use anyhow::Result;
use rusqlite::Connection;

use crate::models::{Asset, FileLocation, Variant};

/// SQLite-backed local catalog for fast queries. This is a derived cache,
/// not the source of truth (sidecar files are).
pub struct Catalog {
    conn: Connection,
}

impl Catalog {
    pub fn open(catalog_root: &Path) -> Result<Self> {
        let db_path = catalog_root.join("catalog.db");
        let conn = Connection::open(&db_path)?;
        Ok(Self { conn })
    }

    /// Initialize the database schema.
    pub fn initialize(&self) -> Result<()> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS assets (
                id TEXT PRIMARY KEY,
                name TEXT,
                created_at TEXT NOT NULL,
                asset_type TEXT NOT NULL,
                tags TEXT NOT NULL DEFAULT '[]',
                description TEXT
            );

            CREATE TABLE IF NOT EXISTS variants (
                content_hash TEXT PRIMARY KEY,
                asset_id TEXT NOT NULL REFERENCES assets(id),
                role TEXT NOT NULL,
                format TEXT NOT NULL,
                file_size INTEGER NOT NULL,
                original_filename TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS file_locations (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                content_hash TEXT NOT NULL REFERENCES variants(content_hash),
                volume_id TEXT NOT NULL REFERENCES volumes(id),
                relative_path TEXT NOT NULL,
                verified_at TEXT
            );

            CREATE TABLE IF NOT EXISTS volumes (
                id TEXT PRIMARY KEY,
                label TEXT NOT NULL,
                mount_point TEXT NOT NULL,
                volume_type TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS recipes (
                id TEXT PRIMARY KEY,
                variant_hash TEXT NOT NULL REFERENCES variants(content_hash),
                software TEXT NOT NULL,
                recipe_type TEXT NOT NULL,
                content_hash TEXT NOT NULL,
                volume_id TEXT,
                relative_path TEXT
            );",
        )?;
        Ok(())
    }

    /// Insert an asset into the catalog.
    pub fn insert_asset(&self, asset: &Asset) -> Result<()> {
        let tags_json = serde_json::to_string(&asset.tags)?;
        self.conn.execute(
            "INSERT OR REPLACE INTO assets (id, name, created_at, asset_type, tags, description) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                asset.id.to_string(),
                asset.name,
                asset.created_at.to_rfc3339(),
                format!("{:?}", asset.asset_type).to_lowercase(),
                tags_json,
                asset.description,
            ],
        )?;
        Ok(())
    }

    /// Insert a variant into the catalog.
    pub fn insert_variant(&self, variant: &Variant) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO variants (content_hash, asset_id, role, format, file_size, original_filename) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                variant.content_hash,
                variant.asset_id.to_string(),
                format!("{:?}", variant.role).to_lowercase(),
                variant.format,
                variant.file_size,
                variant.original_filename,
            ],
        )?;
        Ok(())
    }

    /// Insert a file location for a variant.
    pub fn insert_file_location(&self, content_hash: &str, loc: &FileLocation) -> Result<()> {
        self.conn.execute(
            "INSERT INTO file_locations (content_hash, volume_id, relative_path, verified_at) \
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![
                content_hash,
                loc.volume_id.to_string(),
                loc.relative_path.to_string_lossy().to_string(),
                loc.verified_at.map(|t| t.to_rfc3339()),
            ],
        )?;
        Ok(())
    }

    /// Ensure a volume record exists in the SQLite cache.
    pub fn ensure_volume(&self, volume: &crate::models::Volume) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO volumes (id, label, mount_point, volume_type) \
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![
                volume.id.to_string(),
                volume.label,
                volume.mount_point.to_string_lossy().to_string(),
                format!("{:?}", volume.volume_type).to_lowercase(),
            ],
        )?;
        Ok(())
    }

    /// Check if a variant with the given content hash already exists.
    pub fn has_variant(&self, content_hash: &str) -> Result<bool> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM variants WHERE content_hash = ?1",
            rusqlite::params![content_hash],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    /// Open an in-memory catalog (for testing).
    #[cfg(test)]
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        Ok(Self { conn })
    }

    /// Rebuild the entire catalog from sidecar files.
    pub fn rebuild(&self) -> Result<()> {
        anyhow::bail!("not yet implemented")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initialize_creates_all_tables() {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();

        let tables: Vec<String> = catalog
            .conn
            .prepare(
                "SELECT name FROM sqlite_master \
                 WHERE type='table' AND name NOT LIKE 'sqlite_%' \
                 ORDER BY name",
            )
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();

        assert_eq!(
            tables,
            vec!["assets", "file_locations", "recipes", "variants", "volumes"]
        );
    }

    #[test]
    fn initialize_is_idempotent() {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();
        catalog.initialize().unwrap(); // should not error
    }

    #[test]
    fn insert_and_query_asset() {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();

        let asset = crate::models::Asset::new(crate::models::AssetType::Image);
        catalog.insert_asset(&asset).unwrap();

        let count: i64 = catalog
            .conn
            .query_row("SELECT COUNT(*) FROM assets", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn has_variant_returns_false_when_empty() {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();
        assert!(!catalog.has_variant("sha256:abc123").unwrap());
    }

    #[test]
    fn has_variant_returns_true_after_insert() {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();

        let asset = crate::models::Asset::new(crate::models::AssetType::Image);
        catalog.insert_asset(&asset).unwrap();

        let variant = crate::models::Variant {
            content_hash: "sha256:abc123".to_string(),
            asset_id: asset.id,
            role: crate::models::VariantRole::Original,
            format: "txt".to_string(),
            file_size: 100,
            original_filename: "test.txt".to_string(),
            source_metadata: Default::default(),
            locations: vec![],
        };
        catalog.insert_variant(&variant).unwrap();

        assert!(catalog.has_variant("sha256:abc123").unwrap());
    }
}
