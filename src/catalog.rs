use std::path::Path;

use anyhow::Result;
use rusqlite::Connection;

use crate::models::{Asset, FileLocation, Variant};

/// A row returned from a search query.
#[derive(Debug)]
pub struct SearchRow {
    pub asset_id: String,
    pub name: Option<String>,
    pub asset_type: String,
    pub created_at: String,
    pub original_filename: String,
    pub format: String,
}

/// Full asset details returned by `load_asset_details`.
#[derive(Debug)]
pub struct AssetDetails {
    pub id: String,
    pub name: Option<String>,
    pub asset_type: String,
    pub created_at: String,
    pub tags: Vec<String>,
    pub description: Option<String>,
    pub variants: Vec<VariantDetails>,
}

/// Variant details within an `AssetDetails`.
#[derive(Debug)]
pub struct VariantDetails {
    pub content_hash: String,
    pub role: String,
    pub format: String,
    pub file_size: u64,
    pub original_filename: String,
    pub source_metadata: std::collections::HashMap<String, String>,
    pub locations: Vec<LocationDetails>,
}

/// File location details within a `VariantDetails`.
#[derive(Debug)]
pub struct LocationDetails {
    pub volume_label: String,
    pub relative_path: String,
}

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
                original_filename TEXT NOT NULL,
                source_metadata TEXT NOT NULL DEFAULT '{}'
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
        let meta_json = serde_json::to_string(&variant.source_metadata)?;
        self.conn.execute(
            "INSERT OR REPLACE INTO variants (content_hash, asset_id, role, format, file_size, original_filename, source_metadata) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                variant.content_hash,
                variant.asset_id.to_string(),
                format!("{:?}", variant.role).to_lowercase(),
                variant.format,
                variant.file_size,
                variant.original_filename,
                meta_json,
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

    /// Search assets by optional filters. Results join assets with variants.
    pub fn search_assets(
        &self,
        text: Option<&str>,
        asset_type: Option<&str>,
        tag: Option<&str>,
        format: Option<&str>,
    ) -> Result<Vec<SearchRow>> {
        let mut sql = String::from(
            "SELECT a.id, a.name, a.asset_type, a.created_at, v.original_filename, v.format \
             FROM assets a JOIN variants v ON a.id = v.asset_id WHERE 1=1",
        );
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

        if let Some(text) = text {
            sql.push_str(" AND (a.name LIKE ? OR v.original_filename LIKE ? OR a.description LIKE ?)");
            let pattern = format!("%{text}%");
            params.push(Box::new(pattern.clone()));
            params.push(Box::new(pattern.clone()));
            params.push(Box::new(pattern));
        }
        if let Some(asset_type) = asset_type {
            sql.push_str(" AND a.asset_type = ?");
            params.push(Box::new(asset_type.to_lowercase()));
        }
        if let Some(tag) = tag {
            sql.push_str(" AND a.tags LIKE ?");
            params.push(Box::new(format!("%{tag}%")));
        }
        if let Some(format_filter) = format {
            sql.push_str(" AND v.format = ?");
            params.push(Box::new(format_filter.to_lowercase()));
        }

        sql.push_str(" ORDER BY a.created_at DESC");

        let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(param_refs.as_slice(), |row| {
            Ok(SearchRow {
                asset_id: row.get(0)?,
                name: row.get(1)?,
                asset_type: row.get(2)?,
                created_at: row.get(3)?,
                original_filename: row.get(4)?,
                format: row.get(5)?,
            })
        })?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    }

    /// Resolve a short asset ID prefix to a full UUID string.
    ///
    /// Returns `Ok(Some(id))` if exactly one match, `Ok(None)` if no match,
    /// or an error if the prefix is ambiguous (multiple matches).
    pub fn resolve_asset_id(&self, prefix: &str) -> Result<Option<String>> {
        let pattern = format!("{prefix}%");
        let mut stmt = self.conn.prepare(
            "SELECT id FROM assets WHERE id LIKE ?1",
        )?;
        let ids: Vec<String> = stmt
            .query_map(rusqlite::params![pattern], |row| row.get(0))?
            .collect::<std::result::Result<_, _>>()?;

        match ids.len() {
            0 => Ok(None),
            1 => Ok(Some(ids.into_iter().next().unwrap())),
            n => anyhow::bail!(
                "Ambiguous asset ID prefix '{prefix}': matches {n} assets"
            ),
        }
    }

    /// Load full asset details from the catalog (variants + locations).
    pub fn load_asset_details(&self, asset_id: &str) -> Result<Option<AssetDetails>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, asset_type, created_at, tags, description \
             FROM assets WHERE id = ?1",
        )?;
        let mut rows = stmt.query(rusqlite::params![asset_id])?;
        let row = match rows.next()? {
            Some(r) => r,
            None => return Ok(None),
        };

        let id: String = row.get(0)?;
        let name: Option<String> = row.get(1)?;
        let asset_type: String = row.get(2)?;
        let created_at: String = row.get(3)?;
        let tags_json: String = row.get(4)?;
        let description: Option<String> = row.get(5)?;
        let tags: Vec<String> = serde_json::from_str(&tags_json).unwrap_or_default();

        // Load variants
        let mut vstmt = self.conn.prepare(
            "SELECT content_hash, role, format, file_size, original_filename, source_metadata \
             FROM variants WHERE asset_id = ?1",
        )?;
        let variants: Vec<VariantDetails> = vstmt
            .query_map(rusqlite::params![asset_id], |vrow| {
                let meta_json: String = vrow.get(5)?;
                let source_metadata: std::collections::HashMap<String, String> =
                    serde_json::from_str(&meta_json).unwrap_or_default();
                Ok(VariantDetails {
                    content_hash: vrow.get(0)?,
                    role: vrow.get(1)?,
                    format: vrow.get(2)?,
                    file_size: vrow.get(3)?,
                    original_filename: vrow.get(4)?,
                    source_metadata,
                    locations: Vec::new(), // filled below
                })
            })?
            .collect::<std::result::Result<_, _>>()?;

        // Load locations for each variant
        let mut lstmt = self.conn.prepare(
            "SELECT fl.relative_path, v.label \
             FROM file_locations fl \
             JOIN volumes v ON fl.volume_id = v.id \
             WHERE fl.content_hash = ?1",
        )?;

        let variants: Vec<VariantDetails> = variants
            .into_iter()
            .map(|mut v| {
                let locs: Vec<LocationDetails> = lstmt
                    .query_map(rusqlite::params![v.content_hash], |lrow| {
                        Ok(LocationDetails {
                            volume_label: lrow.get(1)?,
                            relative_path: lrow.get(0)?,
                        })
                    })
                    .unwrap_or_else(|_| {
                        // Return an empty iterator wrapper on error
                        panic!("failed to query locations")
                    })
                    .filter_map(|r| r.ok())
                    .collect();
                v.locations = locs;
                v
            })
            .collect();

        Ok(Some(AssetDetails {
            id,
            name,
            asset_type,
            created_at,
            tags,
            description,
            variants,
        }))
    }

    /// Find which asset owns a variant by its content hash.
    pub fn find_asset_id_by_variant(&self, content_hash: &str) -> Result<Option<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT asset_id FROM variants WHERE content_hash = ?1",
        )?;
        let mut rows = stmt.query(rusqlite::params![content_hash])?;
        match rows.next()? {
            Some(row) => Ok(Some(row.get(0)?)),
            None => Ok(None),
        }
    }

    /// Reassign a variant to a different asset in the catalog.
    pub fn update_variant_asset_id(&self, content_hash: &str, new_asset_id: &str) -> Result<()> {
        let changed = self.conn.execute(
            "UPDATE variants SET asset_id = ?1 WHERE content_hash = ?2",
            rusqlite::params![new_asset_id, content_hash],
        )?;
        if changed == 0 {
            anyhow::bail!("No variant found with hash '{content_hash}'");
        }
        Ok(())
    }

    /// Delete an asset row from the catalog.
    pub fn delete_asset(&self, asset_id: &str) -> Result<()> {
        let changed = self.conn.execute(
            "DELETE FROM assets WHERE id = ?1",
            rusqlite::params![asset_id],
        )?;
        if changed == 0 {
            anyhow::bail!("No asset found with id '{asset_id}'");
        }
        Ok(())
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

    /// Helper to set up a catalog with one asset and variant for search tests.
    fn setup_search_catalog() -> Catalog {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();

        let mut asset = crate::models::Asset::new(crate::models::AssetType::Image);
        asset.name = Some("sunset photo".to_string());
        asset.description = Some("A beautiful sunset over the ocean".to_string());
        asset.tags = vec!["landscape".to_string(), "nature".to_string()];
        catalog.insert_asset(&asset).unwrap();

        let variant = crate::models::Variant {
            content_hash: "sha256:search1".to_string(),
            asset_id: asset.id.clone(),
            role: crate::models::VariantRole::Original,
            format: "jpg".to_string(),
            file_size: 5000,
            original_filename: "sunset_beach.jpg".to_string(),
            source_metadata: Default::default(),
            locations: vec![],
        };
        catalog.insert_variant(&variant).unwrap();

        // Add a second asset of different type
        let mut asset2 = crate::models::Asset::new(crate::models::AssetType::Video);
        asset2.name = Some("holiday clip".to_string());
        catalog.insert_asset(&asset2).unwrap();

        let variant2 = crate::models::Variant {
            content_hash: "sha256:search2".to_string(),
            asset_id: asset2.id,
            role: crate::models::VariantRole::Original,
            format: "mp4".to_string(),
            file_size: 100_000,
            original_filename: "holiday.mp4".to_string(),
            source_metadata: Default::default(),
            locations: vec![],
        };
        catalog.insert_variant(&variant2).unwrap();

        catalog
    }

    #[test]
    fn search_by_text() {
        let catalog = setup_search_catalog();
        let results = catalog.search_assets(Some("sunset"), None, None, None).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name.as_deref(), Some("sunset photo"));
    }

    #[test]
    fn search_by_type() {
        let catalog = setup_search_catalog();
        let results = catalog.search_assets(None, Some("video"), None, None).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].format, "mp4");
    }

    #[test]
    fn search_by_tag() {
        let catalog = setup_search_catalog();
        let results = catalog.search_assets(None, None, Some("landscape"), None).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name.as_deref(), Some("sunset photo"));
    }

    #[test]
    fn search_by_format() {
        let catalog = setup_search_catalog();
        let results = catalog.search_assets(None, None, None, Some("jpg")).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].original_filename, "sunset_beach.jpg");
    }

    #[test]
    fn search_no_results() {
        let catalog = setup_search_catalog();
        let results = catalog.search_assets(Some("nonexistent"), None, None, None).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn search_combined_filters() {
        let catalog = setup_search_catalog();
        let results = catalog
            .search_assets(Some("sunset"), Some("image"), Some("landscape"), Some("jpg"))
            .unwrap();
        assert_eq!(results.len(), 1);
        // Combining mismatched filters yields nothing
        let results = catalog
            .search_assets(Some("sunset"), Some("video"), None, None)
            .unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn resolve_asset_id_full_match() {
        let catalog = setup_search_catalog();
        let results = catalog.search_assets(None, None, None, None).unwrap();
        let full_id = &results[0].asset_id;
        let resolved = catalog.resolve_asset_id(full_id).unwrap();
        assert_eq!(resolved.as_deref(), Some(full_id.as_str()));
    }

    #[test]
    fn resolve_asset_id_prefix_match() {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();

        let asset = crate::models::Asset::new(crate::models::AssetType::Image);
        let full_id = asset.id.to_string();
        catalog.insert_asset(&asset).unwrap();

        let prefix = &full_id[..8];
        let resolved = catalog.resolve_asset_id(prefix).unwrap();
        assert_eq!(resolved.as_deref(), Some(full_id.as_str()));
    }

    #[test]
    fn resolve_asset_id_no_match() {
        let catalog = setup_search_catalog();
        let resolved = catalog.resolve_asset_id("zzzzzzzz").unwrap();
        assert!(resolved.is_none());
    }

    #[test]
    fn resolve_asset_id_ambiguous() {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();

        // Insert two assets and use an empty prefix to match both
        let a1 = crate::models::Asset::new(crate::models::AssetType::Image);
        let a2 = crate::models::Asset::new(crate::models::AssetType::Video);
        catalog.insert_asset(&a1).unwrap();
        catalog.insert_asset(&a2).unwrap();

        let result = catalog.resolve_asset_id("");
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("Ambiguous"), "expected ambiguous error, got: {msg}");
    }

    #[test]
    fn load_asset_details_returns_none_for_missing() {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();
        let details = catalog.load_asset_details("nonexistent-id").unwrap();
        assert!(details.is_none());
    }

    #[test]
    fn load_asset_details_returns_full_info() {
        let catalog = setup_search_catalog();
        let results = catalog.search_assets(Some("sunset"), None, None, None).unwrap();
        let asset_id = &results[0].asset_id;

        let details = catalog.load_asset_details(asset_id).unwrap().unwrap();
        assert_eq!(details.id, *asset_id);
        assert_eq!(details.name.as_deref(), Some("sunset photo"));
        assert_eq!(details.asset_type, "image");
        assert_eq!(details.tags, vec!["landscape", "nature"]);
        assert_eq!(details.description.as_deref(), Some("A beautiful sunset over the ocean"));
        assert_eq!(details.variants.len(), 1);
        assert_eq!(details.variants[0].role, "original");
        assert_eq!(details.variants[0].format, "jpg");
        assert_eq!(details.variants[0].file_size, 5000);
        assert_eq!(details.variants[0].original_filename, "sunset_beach.jpg");
    }

    #[test]
    fn load_asset_details_includes_locations() {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();

        let mut asset = crate::models::Asset::new(crate::models::AssetType::Image);
        asset.name = Some("located asset".to_string());
        catalog.insert_asset(&asset).unwrap();

        let volume = crate::models::Volume::new(
            "test-vol".to_string(),
            std::path::PathBuf::from("/mnt/test"),
            crate::models::VolumeType::Local,
        );
        catalog.ensure_volume(&volume).unwrap();

        let variant = crate::models::Variant {
            content_hash: "sha256:loc1".to_string(),
            asset_id: asset.id,
            role: crate::models::VariantRole::Original,
            format: "png".to_string(),
            file_size: 2048,
            original_filename: "photo.png".to_string(),
            source_metadata: Default::default(),
            locations: vec![],
        };
        catalog.insert_variant(&variant).unwrap();

        let loc = crate::models::FileLocation {
            volume_id: volume.id,
            relative_path: std::path::PathBuf::from("photos/photo.png"),
            verified_at: None,
        };
        catalog.insert_file_location(&variant.content_hash, &loc).unwrap();

        let details = catalog.load_asset_details(&asset.id.to_string()).unwrap().unwrap();
        assert_eq!(details.variants.len(), 1);
        assert_eq!(details.variants[0].locations.len(), 1);
        assert_eq!(details.variants[0].locations[0].volume_label, "test-vol");
        assert_eq!(details.variants[0].locations[0].relative_path, "photos/photo.png");
    }

    #[test]
    fn find_asset_id_by_variant_returns_correct_asset() {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();

        let asset = crate::models::Asset::new(crate::models::AssetType::Image);
        let asset_id = asset.id.to_string();
        catalog.insert_asset(&asset).unwrap();

        let variant = crate::models::Variant {
            content_hash: "sha256:findme".to_string(),
            asset_id: asset.id,
            role: crate::models::VariantRole::Original,
            format: "jpg".to_string(),
            file_size: 1000,
            original_filename: "test.jpg".to_string(),
            source_metadata: Default::default(),
            locations: vec![],
        };
        catalog.insert_variant(&variant).unwrap();

        let found = catalog.find_asset_id_by_variant("sha256:findme").unwrap();
        assert_eq!(found, Some(asset_id));

        let missing = catalog.find_asset_id_by_variant("sha256:nope").unwrap();
        assert!(missing.is_none());
    }

    #[test]
    fn delete_asset_removes_row() {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();

        let asset = crate::models::Asset::new(crate::models::AssetType::Image);
        let asset_id = asset.id.to_string();
        catalog.insert_asset(&asset).unwrap();

        catalog.delete_asset(&asset_id).unwrap();

        let count: i64 = catalog
            .conn
            .query_row("SELECT COUNT(*) FROM assets WHERE id = ?1", rusqlite::params![asset_id], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn delete_asset_errors_on_missing() {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();
        assert!(catalog.delete_asset("nonexistent").is_err());
    }

    #[test]
    fn update_variant_asset_id_changes_fk() {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();

        let asset1 = crate::models::Asset::new(crate::models::AssetType::Image);
        let asset2 = crate::models::Asset::new(crate::models::AssetType::Image);
        catalog.insert_asset(&asset1).unwrap();
        catalog.insert_asset(&asset2).unwrap();

        let variant = crate::models::Variant {
            content_hash: "sha256:moveme".to_string(),
            asset_id: asset1.id,
            role: crate::models::VariantRole::Original,
            format: "jpg".to_string(),
            file_size: 500,
            original_filename: "move.jpg".to_string(),
            source_metadata: Default::default(),
            locations: vec![],
        };
        catalog.insert_variant(&variant).unwrap();

        catalog
            .update_variant_asset_id("sha256:moveme", &asset2.id.to_string())
            .unwrap();

        let new_owner = catalog.find_asset_id_by_variant("sha256:moveme").unwrap();
        assert_eq!(new_owner, Some(asset2.id.to_string()));
    }
}
