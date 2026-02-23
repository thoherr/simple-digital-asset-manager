use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;
use rusqlite::Connection;

use crate::models::{Asset, FileLocation, Recipe, Variant};

/// A row returned from a search query.
#[derive(Debug, serde::Serialize)]
pub struct SearchRow {
    pub asset_id: String,
    pub name: Option<String>,
    pub asset_type: String,
    pub created_at: String,
    pub original_filename: String,
    pub format: String,
    pub tags: Vec<String>,
    pub description: Option<String>,
    pub content_hash: String,
    pub rating: Option<u8>,
    pub color_label: Option<String>,
}

/// Full asset details returned by `load_asset_details`.
#[derive(Debug, serde::Serialize)]
pub struct AssetDetails {
    pub id: String,
    pub name: Option<String>,
    pub asset_type: String,
    pub created_at: String,
    pub tags: Vec<String>,
    pub description: Option<String>,
    pub rating: Option<u8>,
    pub color_label: Option<String>,
    pub variants: Vec<VariantDetails>,
    pub recipes: Vec<RecipeDetails>,
}

/// Variant details within an `AssetDetails`.
#[derive(Debug, serde::Serialize)]
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
#[derive(Debug, serde::Serialize)]
pub struct LocationDetails {
    pub volume_label: String,
    pub relative_path: String,
}

/// A variant that exists in multiple file locations.
#[derive(Debug, serde::Serialize)]
pub struct DuplicateEntry {
    pub content_hash: String,
    pub original_filename: String,
    pub format: String,
    pub file_size: u64,
    pub asset_name: Option<String>,
    pub locations: Vec<LocationDetails>,
}

/// Recipe details within an `AssetDetails`.
#[derive(Debug, serde::Serialize)]
pub struct RecipeDetails {
    pub software: String,
    pub recipe_type: String,
    pub content_hash: String,
    pub relative_path: Option<String>,
}

/// Top-level container for catalog statistics.
#[derive(Debug, serde::Serialize)]
pub struct CatalogStats {
    pub overview: OverviewStats,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub types: Option<TypeStats>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volumes: Option<Vec<VolumeStats>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<TagStats>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verified: Option<VerificationStats>,
}

/// Overview counts for the entire catalog.
#[derive(Debug, serde::Serialize)]
pub struct OverviewStats {
    pub assets: u64,
    pub variants: u64,
    pub recipes: u64,
    pub volumes_total: u64,
    pub volumes_online: u64,
    pub volumes_offline: u64,
    pub total_size: u64,
}

/// Breakdown by asset type and file formats.
#[derive(Debug, serde::Serialize)]
pub struct TypeStats {
    pub asset_types: Vec<TypeCount>,
    pub variant_formats: Vec<FormatCount>,
    pub recipe_formats: Vec<FormatCount>,
}

/// A single asset-type count entry.
#[derive(Debug, serde::Serialize)]
pub struct TypeCount {
    pub asset_type: String,
    pub count: u64,
    pub percentage: f64,
}

/// A single format count entry.
#[derive(Debug, serde::Serialize)]
pub struct FormatCount {
    pub format: String,
    pub count: u64,
}

/// Per-volume statistics.
#[derive(Debug, serde::Serialize)]
pub struct VolumeStats {
    pub label: String,
    pub volume_id: String,
    pub is_online: bool,
    pub assets: u64,
    pub variants: u64,
    pub recipes: u64,
    pub formats: Vec<String>,
    pub directories: u64,
    pub size: u64,
    pub verified_count: u64,
    pub total_locations: u64,
    pub verification_pct: f64,
    pub oldest_verified_at: Option<String>,
}

/// Internal helper for per-volume data before merging with device registry.
struct VolumeStatsRaw {
    volume_id: String,
    label: String,
    assets: u64,
    variants: u64,
    recipes: u64,
    formats: Vec<String>,
    directories: u64,
    size: u64,
    verified_count: u64,
    total_locations: u64,
    oldest_verified_at: Option<String>,
}

/// Tag usage statistics.
#[derive(Debug, serde::Serialize)]
pub struct TagStats {
    pub unique_tags: u64,
    pub tagged_assets: u64,
    pub untagged_assets: u64,
    pub top_tags: Vec<TagCount>,
}

/// A single tag frequency entry.
#[derive(Debug, serde::Serialize)]
pub struct TagCount {
    pub tag: String,
    pub count: u64,
}

/// Verification health statistics.
#[derive(Debug, serde::Serialize)]
pub struct VerificationStats {
    pub total_locations: u64,
    pub verified_locations: u64,
    pub unverified_locations: u64,
    pub coverage_pct: f64,
    pub oldest_verified_at: Option<String>,
    pub newest_verified_at: Option<String>,
    pub per_volume: Vec<VolumeVerificationStats>,
}

/// Per-volume verification summary.
#[derive(Debug, serde::Serialize)]
pub struct VolumeVerificationStats {
    pub label: String,
    pub volume_id: String,
    pub is_online: bool,
    pub locations: u64,
    pub verified: u64,
    pub coverage_pct: f64,
    pub oldest_verified_at: Option<String>,
}

/// Sort order for paginated search.
#[derive(Debug, Clone, Copy)]
pub enum SearchSort {
    DateDesc,
    DateAsc,
    NameAsc,
    NameDesc,
    SizeDesc,
    SizeAsc,
}

impl SearchSort {
    fn to_sql(&self) -> &'static str {
        match self {
            SearchSort::DateDesc => "a.created_at DESC",
            SearchSort::DateAsc => "a.created_at ASC",
            SearchSort::NameAsc => "COALESCE(a.name, bv.original_filename) ASC",
            SearchSort::NameDesc => "COALESCE(a.name, bv.original_filename) DESC",
            SearchSort::SizeDesc => "bv.file_size DESC",
            SearchSort::SizeAsc => "bv.file_size ASC",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "date_asc" => SearchSort::DateAsc,
            "name_asc" => SearchSort::NameAsc,
            "name_desc" => SearchSort::NameDesc,
            "size_desc" => SearchSort::SizeDesc,
            "size_asc" => SearchSort::SizeAsc,
            _ => SearchSort::DateDesc,
        }
    }
}

/// Options for paginated search.
pub struct SearchOptions<'a> {
    pub text: Option<&'a str>,
    pub asset_type: Option<&'a str>,
    pub tag: Option<&'a str>,
    pub format: Option<&'a str>,
    pub volume: Option<&'a str>,
    pub rating_min: Option<u8>,
    pub rating_exact: Option<u8>,
    pub camera: Option<&'a str>,
    pub lens: Option<&'a str>,
    pub iso_min: Option<i64>,
    pub iso_max: Option<i64>,
    pub focal_min: Option<f64>,
    pub focal_max: Option<f64>,
    pub f_min: Option<f64>,
    pub f_max: Option<f64>,
    pub width_min: Option<i64>,
    pub height_min: Option<i64>,
    pub meta_filters: Vec<(&'a str, &'a str)>,
    pub orphan: bool,
    pub stale_days: Option<u64>,
    pub missing_asset_ids: Option<&'a [String]>,
    pub no_online_locations: Option<&'a [String]>,
    pub color_label: Option<&'a str>,
    pub path_prefix: Option<&'a str>,
    pub collection_asset_ids: Option<&'a [String]>,
    pub sort: SearchSort,
    pub page: u32,
    pub per_page: u32,
}

impl<'a> Default for SearchOptions<'a> {
    fn default() -> Self {
        Self {
            text: None,
            asset_type: None,
            tag: None,
            format: None,
            volume: None,
            rating_min: None,
            rating_exact: None,
            camera: None,
            lens: None,
            iso_min: None,
            iso_max: None,
            focal_min: None,
            focal_max: None,
            f_min: None,
            f_max: None,
            width_min: None,
            height_min: None,
            meta_filters: Vec::new(),
            orphan: false,
            stale_days: None,
            missing_asset_ids: None,
            no_online_locations: None,
            color_label: None,
            path_prefix: None,
            collection_asset_ids: None,
            sort: SearchSort::DateDesc,
            page: 1,
            per_page: 60,
        }
    }
}

/// A page of search results with pagination metadata.
#[derive(Debug, serde::Serialize)]
pub struct SearchPage {
    pub rows: Vec<SearchRow>,
    pub total: u64,
    pub page: u32,
    pub per_page: u32,
    pub total_pages: u32,
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
        let catalog = Self { conn };
        catalog.run_migrations();
        Ok(catalog)
    }

    /// Open without running migrations — for hot paths where migrations
    /// have already been applied (e.g. per-request in the web server).
    pub fn open_fast(catalog_root: &Path) -> Result<Self> {
        let db_path = catalog_root.join("catalog.db");
        let conn = Connection::open(&db_path)?;
        Ok(Self { conn })
    }

    /// Access the underlying SQLite connection.
    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    /// Run lightweight schema migrations (ADD COLUMN + CREATE INDEX).
    ///
    /// Should be called once at startup (e.g. server init) so that existing
    /// catalogs pick up new columns without requiring `dam init` or
    /// `dam rebuild-catalog`. Each ALTER TABLE silently ignores
    /// "duplicate column" errors.
    pub fn run_migrations(&self) {
        let _ = self.conn.execute_batch("ALTER TABLE assets ADD COLUMN rating INTEGER");
        let _ = self.conn.execute_batch("ALTER TABLE assets ADD COLUMN color_label TEXT");
        let _ = self.conn.execute_batch("ALTER TABLE variants ADD COLUMN camera_model TEXT");
        let _ = self.conn.execute_batch("ALTER TABLE variants ADD COLUMN lens_model TEXT");
        let _ = self.conn.execute_batch("ALTER TABLE variants ADD COLUMN focal_length_mm REAL");
        let _ = self.conn.execute_batch("ALTER TABLE variants ADD COLUMN f_number REAL");
        let _ = self.conn.execute_batch("ALTER TABLE variants ADD COLUMN iso INTEGER");
        let _ = self.conn.execute_batch("ALTER TABLE variants ADD COLUMN image_width INTEGER");
        let _ = self.conn.execute_batch("ALTER TABLE variants ADD COLUMN image_height INTEGER");
        let _ = self.conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_variants_camera ON variants(camera_model);
             CREATE INDEX IF NOT EXISTS idx_variants_lens ON variants(lens_model);
             CREATE INDEX IF NOT EXISTS idx_variants_iso ON variants(iso);
             CREATE INDEX IF NOT EXISTS idx_variants_focal ON variants(focal_length_mm);",
        );
        // best_variant_hash denormalization
        let _ = self.conn.execute_batch("ALTER TABLE assets ADD COLUMN best_variant_hash TEXT");
        let _ = self.conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_variants_asset_id ON variants(asset_id)",
        );
        // Backfill best_variant_hash for existing rows (runs once, idempotent)
        let _ = self.conn.execute_batch(
            "UPDATE assets SET best_variant_hash = (
                SELECT content_hash FROM variants WHERE asset_id = assets.id
                ORDER BY
                    CASE role WHEN 'export' THEN 300 WHEN 'processed' THEN 200
                        WHEN 'original' THEN 100 ELSE 0 END +
                    CASE WHEN LOWER(format) IN ('jpg','jpeg','png','tiff','tif','webp')
                        THEN 50 ELSE 0 END +
                    MIN(file_size / 1000000, 49)
                DESC LIMIT 1
            ) WHERE best_variant_hash IS NULL",
        );
        // Collection tables
        let _ = crate::collection::CollectionStore::initialize(&self.conn);
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
                description TEXT,
                best_variant_hash TEXT
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
                relative_path TEXT,
                verified_at TEXT
            );",
        )?;
        // Migration: add rating column to existing catalogs (ignored if already present)
        let _ = self.conn.execute_batch("ALTER TABLE assets ADD COLUMN rating INTEGER");
        // Migration: add color_label column to existing catalogs (ignored if already present)
        let _ = self.conn.execute_batch("ALTER TABLE assets ADD COLUMN color_label TEXT");

        // Migration: add indexed metadata columns to variants
        let _ = self.conn.execute_batch("ALTER TABLE variants ADD COLUMN camera_model TEXT");
        let _ = self.conn.execute_batch("ALTER TABLE variants ADD COLUMN lens_model TEXT");
        let _ = self.conn.execute_batch("ALTER TABLE variants ADD COLUMN focal_length_mm REAL");
        let _ = self.conn.execute_batch("ALTER TABLE variants ADD COLUMN f_number REAL");
        let _ = self.conn.execute_batch("ALTER TABLE variants ADD COLUMN iso INTEGER");
        let _ = self.conn.execute_batch("ALTER TABLE variants ADD COLUMN image_width INTEGER");
        let _ = self.conn.execute_batch("ALTER TABLE variants ADD COLUMN image_height INTEGER");

        self.conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_variants_camera ON variants(camera_model);
             CREATE INDEX IF NOT EXISTS idx_variants_lens ON variants(lens_model);
             CREATE INDEX IF NOT EXISTS idx_variants_iso ON variants(iso);
             CREATE INDEX IF NOT EXISTS idx_variants_focal ON variants(focal_length_mm);
             CREATE INDEX IF NOT EXISTS idx_variants_asset_id ON variants(asset_id);",
        )?;

        // Migration: best_variant_hash denormalization
        let _ = self.conn.execute_batch("ALTER TABLE assets ADD COLUMN best_variant_hash TEXT");

        // Collection tables
        crate::collection::CollectionStore::initialize(&self.conn)?;

        // Backfill metadata columns from existing JSON (only rows not yet populated)
        let _ = self.conn.execute_batch(
            "UPDATE variants SET
                camera_model = json_extract(source_metadata, '$.camera_model'),
                lens_model = json_extract(source_metadata, '$.lens_model'),
                iso = CAST(json_extract(source_metadata, '$.iso') AS INTEGER),
                focal_length_mm = CAST(REPLACE(json_extract(source_metadata, '$.focal_length'), ' mm', '') AS REAL),
                f_number = CAST(json_extract(source_metadata, '$.f_number') AS REAL),
                image_width = CAST(json_extract(source_metadata, '$.image_width') AS INTEGER),
                image_height = CAST(json_extract(source_metadata, '$.image_height') AS INTEGER)
            WHERE camera_model IS NULL AND source_metadata != '{}'"
        );

        // Backfill best_variant_hash for existing rows (runs once, idempotent)
        let _ = self.conn.execute_batch(
            "UPDATE assets SET best_variant_hash = (
                SELECT content_hash FROM variants WHERE asset_id = assets.id
                ORDER BY
                    CASE role WHEN 'export' THEN 300 WHEN 'processed' THEN 200
                        WHEN 'original' THEN 100 ELSE 0 END +
                    CASE WHEN LOWER(format) IN ('jpg','jpeg','png','tiff','tif','webp')
                        THEN 50 ELSE 0 END +
                    MIN(file_size / 1000000, 49)
                DESC LIMIT 1
            ) WHERE best_variant_hash IS NULL",
        );

        Ok(())
    }

    /// Insert an asset into the catalog.
    pub fn insert_asset(&self, asset: &Asset) -> Result<()> {
        let tags_json = serde_json::to_string(&asset.tags)?;
        let best_hash = crate::models::variant::compute_best_variant_hash(&asset.variants);
        self.conn.execute(
            "INSERT OR REPLACE INTO assets (id, name, created_at, asset_type, tags, description, rating, color_label, best_variant_hash) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            rusqlite::params![
                asset.id.to_string(),
                asset.name,
                asset.created_at.to_rfc3339(),
                format!("{:?}", asset.asset_type).to_lowercase(),
                tags_json,
                asset.description,
                asset.rating.map(|r| r as i64),
                asset.color_label,
                best_hash,
            ],
        )?;
        Ok(())
    }

    /// Update just the rating for an asset in the catalog.
    pub fn update_asset_rating(&self, asset_id: &str, rating: Option<u8>) -> Result<()> {
        self.conn.execute(
            "UPDATE assets SET rating = ?1 WHERE id = ?2",
            rusqlite::params![rating.map(|r| r as i64), asset_id],
        )?;
        Ok(())
    }

    /// Update just the color label for an asset in the catalog.
    pub fn update_asset_color_label(&self, asset_id: &str, color_label: Option<&str>) -> Result<()> {
        self.conn.execute(
            "UPDATE assets SET color_label = ?1 WHERE id = ?2",
            rusqlite::params![color_label, asset_id],
        )?;
        Ok(())
    }

    /// Update the denormalized best_variant_hash for an asset.
    pub fn update_best_variant_hash(&self, asset_id: &str, hash: Option<&str>) -> Result<()> {
        self.conn.execute(
            "UPDATE assets SET best_variant_hash = ?1 WHERE id = ?2",
            rusqlite::params![hash, asset_id],
        )?;
        Ok(())
    }

    /// Insert a variant into the catalog.
    pub fn insert_variant(&self, variant: &Variant) -> Result<()> {
        let meta = &variant.source_metadata;
        let meta_json = serde_json::to_string(meta)?;

        let camera_model = meta.get("camera_model").cloned();
        let lens_model = meta.get("lens_model").cloned();
        let focal_length_mm: Option<f64> = meta
            .get("focal_length")
            .and_then(|v| v.trim_end_matches(" mm").parse().ok());
        let f_number: Option<f64> = meta.get("f_number").and_then(|v| v.parse().ok());
        let iso: Option<i64> = meta.get("iso").and_then(|v| v.parse().ok());
        let image_width: Option<i64> = meta.get("image_width").and_then(|v| v.parse().ok());
        let image_height: Option<i64> = meta.get("image_height").and_then(|v| v.parse().ok());

        self.conn.execute(
            "INSERT OR REPLACE INTO variants (content_hash, asset_id, role, format, file_size, original_filename, source_metadata, \
             camera_model, lens_model, focal_length_mm, f_number, iso, image_width, image_height) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            rusqlite::params![
                variant.content_hash,
                variant.asset_id.to_string(),
                format!("{:?}", variant.role).to_lowercase(),
                variant.format,
                variant.file_size,
                variant.original_filename,
                meta_json,
                camera_model,
                lens_model,
                focal_length_mm,
                f_number,
                iso,
                image_width,
                image_height,
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

    /// List all file locations with their associated asset IDs.
    /// Returns `(asset_id, volume_id, relative_path)` tuples.
    pub fn list_all_locations_with_assets(&self) -> Result<Vec<(String, String, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT v.asset_id, fl.volume_id, fl.relative_path \
             FROM file_locations fl \
             JOIN variants v ON fl.content_hash = v.content_hash",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    }

    /// Insert a recipe into the catalog.
    pub fn insert_recipe(&self, recipe: &Recipe) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO recipes (id, variant_hash, software, recipe_type, content_hash, volume_id, relative_path) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                recipe.id.to_string(),
                recipe.variant_hash,
                recipe.software,
                format!("{:?}", recipe.recipe_type).to_lowercase(),
                recipe.content_hash,
                recipe.location.volume_id.to_string(),
                recipe.location.relative_path.to_string_lossy().to_string(),
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
        rating_min: Option<u8>,
        rating_exact: Option<u8>,
    ) -> Result<Vec<SearchRow>> {
        let opts = SearchOptions {
            text,
            asset_type,
            tag,
            format: format,
            rating_min,
            rating_exact,
            per_page: u32::MAX,
            ..Default::default()
        };
        self.search_paginated(&opts)
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
            "SELECT id, name, asset_type, created_at, tags, description, rating, color_label \
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
        let rating_val: Option<i64> = row.get(6)?;
        let rating = rating_val.map(|r| r as u8);
        let color_label: Option<String> = row.get(7)?;
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

        // Load recipes linked to any variant of this asset
        let mut rstmt = self.conn.prepare(
            "SELECT r.software, r.recipe_type, r.content_hash, r.relative_path \
             FROM recipes r \
             JOIN variants v ON r.variant_hash = v.content_hash \
             WHERE v.asset_id = ?1",
        )?;
        let recipes: Vec<RecipeDetails> = rstmt
            .query_map(rusqlite::params![asset_id], |rrow| {
                Ok(RecipeDetails {
                    software: rrow.get(0)?,
                    recipe_type: rrow.get(1)?,
                    content_hash: rrow.get(2)?,
                    relative_path: rrow.get(3)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        Ok(Some(AssetDetails {
            id,
            name,
            asset_type,
            created_at,
            tags,
            description,
            rating,
            color_label,
            variants,
            recipes,
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

    /// Update a variant's role in the catalog.
    pub fn update_variant_role(&self, content_hash: &str, role: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE variants SET role = ?1 WHERE content_hash = ?2",
            rusqlite::params![role, content_hash],
        )?;
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

    /// Find variants that have more than one file location (duplicates).
    pub fn find_duplicates(&self) -> Result<Vec<DuplicateEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT v.content_hash, v.original_filename, v.format, v.file_size, a.name \
             FROM variants v \
             JOIN assets a ON v.asset_id = a.id \
             WHERE v.content_hash IN ( \
                 SELECT content_hash FROM file_locations \
                 GROUP BY content_hash HAVING COUNT(*) > 1 \
             ) \
             ORDER BY v.file_size DESC",
        )?;

        let entries: Vec<DuplicateEntry> = stmt
            .query_map([], |row| {
                Ok(DuplicateEntry {
                    content_hash: row.get(0)?,
                    original_filename: row.get(1)?,
                    format: row.get(2)?,
                    file_size: row.get(3)?,
                    asset_name: row.get(4)?,
                    locations: Vec::new(),
                })
            })?
            .collect::<std::result::Result<_, _>>()?;

        // Load locations for each duplicate
        let mut lstmt = self.conn.prepare(
            "SELECT fl.relative_path, vol.label \
             FROM file_locations fl \
             JOIN volumes vol ON fl.volume_id = vol.id \
             WHERE fl.content_hash = ?1",
        )?;

        let entries: Vec<DuplicateEntry> = entries
            .into_iter()
            .map(|mut e| {
                let locs: Vec<LocationDetails> = lstmt
                    .query_map(rusqlite::params![e.content_hash], |lrow| {
                        Ok(LocationDetails {
                            volume_label: lrow.get(1)?,
                            relative_path: lrow.get(0)?,
                        })
                    })
                    .unwrap()
                    .filter_map(|r| r.ok())
                    .collect();
                e.locations = locs;
                e
            })
            .collect();

        Ok(entries)
    }

    /// Delete a specific file location row. Returns true if a row was deleted.
    pub fn delete_file_location(
        &self,
        content_hash: &str,
        volume_id: &str,
        relative_path: &str,
    ) -> Result<bool> {
        let changed = self.conn.execute(
            "DELETE FROM file_locations WHERE content_hash = ?1 AND volume_id = ?2 AND relative_path = ?3",
            rusqlite::params![content_hash, volume_id, relative_path],
        )?;
        Ok(changed > 0)
    }

    /// Delete a recipe record by ID. Returns true if a row was deleted.
    pub fn delete_recipe(&self, recipe_id: &str) -> Result<bool> {
        let changed = self.conn.execute(
            "DELETE FROM recipes WHERE id = ?1",
            rusqlite::params![recipe_id],
        )?;
        Ok(changed > 0)
    }

    /// Update the volume and path for a recipe.
    pub fn update_recipe_location(
        &self,
        recipe_id: &str,
        volume_id: &str,
        relative_path: &str,
    ) -> Result<()> {
        let changed = self.conn.execute(
            "UPDATE recipes SET volume_id = ?1, relative_path = ?2 WHERE id = ?3",
            rusqlite::params![volume_id, relative_path, recipe_id],
        )?;
        if changed == 0 {
            anyhow::bail!("No recipe found with id '{recipe_id}'");
        }
        Ok(())
    }

    /// Update the `verified_at` timestamp for a file location.
    pub fn update_verified_at(
        &self,
        content_hash: &str,
        volume_id: &str,
        relative_path: &str,
    ) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        self.conn.execute(
            "UPDATE file_locations SET verified_at = ?1 \
             WHERE content_hash = ?2 AND volume_id = ?3 AND relative_path = ?4",
            rusqlite::params![now, content_hash, volume_id, relative_path],
        )?;
        Ok(())
    }

    /// Check if a recipe with the given content hash exists.
    pub fn has_recipe_by_content_hash(&self, content_hash: &str) -> Result<bool> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM recipes WHERE content_hash = ?1",
            rusqlite::params![content_hash],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    /// Find a recipe by its location (variant_hash, volume_id, relative_path).
    /// Returns `(recipe_id, content_hash)` if found.
    pub fn find_recipe_by_location(
        &self,
        variant_hash: &str,
        volume_id: &str,
        relative_path: &str,
    ) -> Result<Option<(String, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, content_hash FROM recipes \
             WHERE variant_hash = ?1 AND volume_id = ?2 AND relative_path = ?3",
        )?;
        let mut rows = stmt.query(rusqlite::params![variant_hash, volume_id, relative_path])?;
        match rows.next()? {
            Some(row) => Ok(Some((row.get(0)?, row.get(1)?))),
            None => Ok(None),
        }
    }

    /// Update a recipe's content hash (used when a recipe file changes on re-import).
    pub fn update_recipe_content_hash(&self, recipe_id: &str, new_content_hash: &str) -> Result<()> {
        let changed = self.conn.execute(
            "UPDATE recipes SET content_hash = ?1 WHERE id = ?2",
            rusqlite::params![new_content_hash, recipe_id],
        )?;
        if changed == 0 {
            anyhow::bail!("No recipe found with id '{recipe_id}'");
        }
        Ok(())
    }

    /// Find a recipe by volume and path (ignoring variant_hash).
    /// Returns `(recipe_id, content_hash, variant_hash)` if found.
    pub fn find_recipe_by_volume_and_path(
        &self,
        volume_id: &str,
        relative_path: &str,
    ) -> Result<Option<(String, String, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, content_hash, variant_hash FROM recipes \
             WHERE volume_id = ?1 AND relative_path = ?2",
        )?;
        let mut rows = stmt.query(rusqlite::params![volume_id, relative_path])?;
        match rows.next()? {
            Some(row) => Ok(Some((row.get(0)?, row.get(1)?, row.get(2)?))),
            None => Ok(None),
        }
    }

    /// Update `verified_at` timestamp on a recipe by its location.
    pub fn update_recipe_verified_at(
        &self,
        variant_hash: &str,
        volume_id: &str,
        relative_path: &str,
    ) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        self.conn.execute(
            "UPDATE recipes SET verified_at = ?1 \
             WHERE variant_hash = ?2 AND volume_id = ?3 AND relative_path = ?4",
            rusqlite::params![now, variant_hash, volume_id, relative_path],
        )?;
        Ok(())
    }

    /// Find a variant by its exact volume + relative_path.
    /// Returns `(content_hash, format)`.
    pub fn find_variant_by_volume_and_path(
        &self,
        volume_id: &str,
        relative_path: &str,
    ) -> Result<Option<(String, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT v.content_hash, v.format FROM file_locations fl \
             JOIN variants v ON fl.content_hash = v.content_hash \
             WHERE fl.volume_id = ?1 AND fl.relative_path = ?2 \
             LIMIT 1",
        )?;
        let mut rows = stmt.query(rusqlite::params![volume_id, relative_path])?;
        match rows.next()? {
            Some(row) => Ok(Some((row.get(0)?, row.get(1)?))),
            None => Ok(None),
        }
    }

    /// Find a variant whose file location on the given volume shares the same
    /// directory prefix and filename stem. Returns `(content_hash, asset_id)`.
    pub fn find_variant_hash_by_stem_and_directory(
        &self,
        stem: &str,
        directory_prefix: &str,
        volume_id: &str,
    ) -> Result<Option<(String, String)>> {
        // Match file_locations where: same volume, path starts with directory_prefix,
        // and the filename (without extension) matches the stem.
        let path_pattern = if directory_prefix.is_empty() {
            format!("{stem}.%")
        } else {
            format!("{directory_prefix}/{stem}.%")
        };
        let mut stmt = self.conn.prepare(
            "SELECT fl.content_hash, v.asset_id FROM file_locations fl \
             JOIN variants v ON fl.content_hash = v.content_hash \
             WHERE fl.volume_id = ?1 AND fl.relative_path LIKE ?2 \
             LIMIT 1",
        )?;
        let mut rows = stmt.query(rusqlite::params![volume_id, path_pattern])?;
        match rows.next()? {
            Some(row) => Ok(Some((row.get(0)?, row.get(1)?))),
            None => Ok(None),
        }
    }

    /// Drop and recreate data tables (assets, variants, file_locations, recipes).
    /// Keeps the volumes table intact. Ensures the schema is up to date.
    pub fn rebuild(&self) -> Result<()> {
        self.conn.execute_batch(
            "DROP TABLE IF EXISTS collection_assets;
             DROP TABLE IF EXISTS collections;
             DROP TABLE IF EXISTS file_locations;
             DROP TABLE IF EXISTS recipes;
             DROP TABLE IF EXISTS variants;
             DROP TABLE IF EXISTS assets;",
        )?;
        self.initialize()?;
        Ok(())
    }

    // ── Sync queries ───────────────────────────────────────────────

    /// List all file locations on a volume whose path starts with the given prefix.
    /// Returns `(content_hash, relative_path)` pairs.
    pub fn list_locations_for_volume_under_prefix(
        &self,
        volume_id: &str,
        prefix: &str,
    ) -> Result<Vec<(String, String)>> {
        let pattern = if prefix.is_empty() {
            "%".to_string()
        } else {
            format!("{prefix}%")
        };
        let mut stmt = self.conn.prepare(
            "SELECT content_hash, relative_path FROM file_locations \
             WHERE volume_id = ?1 AND relative_path LIKE ?2",
        )?;
        let rows = stmt.query_map(rusqlite::params![volume_id, pattern], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }

    /// List all recipes on a volume whose path starts with the given prefix.
    /// Returns `(recipe_id, content_hash, variant_hash, relative_path)` tuples.
    pub fn list_recipes_for_volume_under_prefix(
        &self,
        volume_id: &str,
        prefix: &str,
    ) -> Result<Vec<(String, String, String, String)>> {
        let pattern = if prefix.is_empty() {
            "%".to_string()
        } else {
            format!("{prefix}%")
        };
        let mut stmt = self.conn.prepare(
            "SELECT id, content_hash, variant_hash, relative_path FROM recipes \
             WHERE volume_id = ?1 AND relative_path LIKE ?2",
        )?;
        let rows = stmt.query_map(rusqlite::params![volume_id, pattern], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }

    /// List all recipes for a given asset (across all volumes).
    /// Returns `(recipe_id, content_hash, variant_hash, relative_path, volume_id)` tuples.
    pub fn list_recipes_for_asset(
        &self,
        asset_id: &str,
    ) -> Result<Vec<(String, String, String, String, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT r.id, r.content_hash, r.variant_hash, r.relative_path, r.volume_id \
             FROM recipes r \
             JOIN variants v ON r.variant_hash = v.content_hash \
             WHERE v.asset_id = ?1",
        )?;
        let rows = stmt.query_map(rusqlite::params![asset_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }

    /// List all file locations for an asset's variants.
    /// Returns (content_hash, relative_path, volume_id).
    pub fn list_file_locations_for_asset(
        &self,
        asset_id: &str,
    ) -> Result<Vec<(String, String, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT fl.content_hash, fl.relative_path, fl.volume_id \
             FROM file_locations fl \
             JOIN variants v ON fl.content_hash = v.content_hash \
             WHERE v.asset_id = ?1",
        )?;
        let rows = stmt.query_map(rusqlite::params![asset_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }

    /// Update the relative_path for a file location (variant moved on disk).
    pub fn update_file_location_path(
        &self,
        content_hash: &str,
        volume_id: &str,
        old_path: &str,
        new_path: &str,
    ) -> Result<()> {
        let changed = self.conn.execute(
            "UPDATE file_locations SET relative_path = ?1 \
             WHERE content_hash = ?2 AND volume_id = ?3 AND relative_path = ?4",
            rusqlite::params![new_path, content_hash, volume_id, old_path],
        )?;
        if changed == 0 {
            anyhow::bail!(
                "No file location found for hash '{content_hash}' at '{old_path}'"
            );
        }
        Ok(())
    }

    /// Update the relative_path for a recipe (recipe file moved on disk).
    pub fn update_recipe_relative_path(
        &self,
        recipe_id: &str,
        new_path: &str,
    ) -> Result<()> {
        let changed = self.conn.execute(
            "UPDATE recipes SET relative_path = ?1 WHERE id = ?2",
            rusqlite::params![new_path, recipe_id],
        )?;
        if changed == 0 {
            anyhow::bail!("No recipe found with id '{recipe_id}'");
        }
        Ok(())
    }

    // ── Stats queries ──────────────────────────────────────────────

    /// Core overview counts: (assets, variants, recipes, total_size).
    pub fn stats_overview(&self) -> Result<(u64, u64, u64, u64)> {
        let assets: u64 = self.conn.query_row(
            "SELECT COUNT(*) FROM assets", [], |r| r.get(0),
        )?;
        let variants: u64 = self.conn.query_row(
            "SELECT COUNT(*) FROM variants", [], |r| r.get(0),
        )?;
        let recipes: u64 = self.conn.query_row(
            "SELECT COUNT(*) FROM recipes", [], |r| r.get(0),
        )?;
        let total_size: u64 = self.conn.query_row(
            "SELECT COALESCE(SUM(file_size), 0) FROM variants", [], |r| r.get(0),
        )?;
        Ok((assets, variants, recipes, total_size))
    }

    /// Asset type breakdown: Vec<(type_name, count)>.
    pub fn stats_asset_types(&self) -> Result<Vec<(String, u64)>> {
        let mut stmt = self.conn.prepare(
            "SELECT asset_type, COUNT(*) FROM assets GROUP BY asset_type ORDER BY COUNT(*) DESC",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect::<std::result::Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Top variant formats: Vec<(format, count)>.
    pub fn stats_variant_formats(&self, limit: usize) -> Result<Vec<(String, u64)>> {
        let mut stmt = self.conn.prepare(
            "SELECT format, COUNT(*) FROM variants GROUP BY format ORDER BY COUNT(*) DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(rusqlite::params![limit as u64], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect::<std::result::Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// All recipe relative paths (for extension extraction in Rust).
    pub fn stats_recipe_paths(&self) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT relative_path FROM recipes WHERE relative_path IS NOT NULL",
        )?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        rows.collect::<std::result::Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Build the WHERE clause and parameters for search queries.
    /// Returns (where_clause, params, needs_fl_join, needs_v_join).
    /// `needs_v_join`: true when any filter references the `v` (variants) table directly.
    /// `needs_fl_join`: true when any filter references `fl` (file_locations); implies `needs_v_join`.
    fn build_search_where(opts: &SearchOptions) -> (String, Vec<Box<dyn rusqlite::types::ToSql>>, bool, bool) {
        let mut clauses = Vec::new();
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        let mut needs_fl_join = opts.volume.is_some();
        let mut needs_v_join = false;

        if let Some(text) = opts.text {
            if !text.is_empty() {
                // Text search uses bv (best variant) — no v join needed
                clauses.push(
                    "(a.name LIKE ? OR bv.original_filename LIKE ? OR a.description LIKE ? OR bv.source_metadata LIKE ?)".to_string(),
                );
                let pattern = format!("%{text}%");
                params.push(Box::new(pattern.clone()));
                params.push(Box::new(pattern.clone()));
                params.push(Box::new(pattern.clone()));
                params.push(Box::new(pattern));
            }
        }
        if let Some(asset_type) = opts.asset_type {
            if !asset_type.is_empty() {
                clauses.push("a.asset_type = ?".to_string());
                params.push(Box::new(asset_type.to_lowercase()));
            }
        }
        if let Some(tag) = opts.tag {
            if !tag.is_empty() {
                for t in tag.split(',') {
                    let t = t.trim();
                    if !t.is_empty() {
                        clauses.push("a.tags LIKE ?".to_string());
                        params.push(Box::new(format!("%\"{t}\"%")));
                    }
                }
            }
        }
        if let Some(format_filter) = opts.format {
            if !format_filter.is_empty() {
                clauses.push("v.format = ?".to_string());
                params.push(Box::new(format_filter.to_lowercase()));
                needs_v_join = true;
            }
        }
        if let Some(volume) = opts.volume {
            if !volume.is_empty() {
                clauses.push("fl.volume_id = ?".to_string());
                params.push(Box::new(volume.to_string()));
            }
        }
        if let Some(min) = opts.rating_min {
            clauses.push("a.rating >= ?".to_string());
            params.push(Box::new(min as i64));
        }
        if let Some(exact) = opts.rating_exact {
            clauses.push("a.rating = ?".to_string());
            params.push(Box::new(exact as i64));
        }
        if let Some(label) = opts.color_label {
            if !label.is_empty() {
                clauses.push("a.color_label = ?".to_string());
                params.push(Box::new(label.to_string()));
            }
        }
        if let Some(prefix) = opts.path_prefix {
            if !prefix.is_empty() {
                clauses.push("fl.relative_path LIKE ?".to_string());
                params.push(Box::new(format!("{prefix}%")));
                needs_fl_join = true;
            }
        }

        // Metadata column filters — these reference v.* so need v join
        if let Some(camera) = opts.camera {
            if !camera.is_empty() {
                clauses.push("v.camera_model LIKE ?".to_string());
                params.push(Box::new(format!("%{camera}%")));
                needs_v_join = true;
            }
        }
        if let Some(lens) = opts.lens {
            if !lens.is_empty() {
                clauses.push("v.lens_model LIKE ?".to_string());
                params.push(Box::new(format!("%{lens}%")));
                needs_v_join = true;
            }
        }
        if let Some(min) = opts.iso_min {
            clauses.push("v.iso >= ?".to_string());
            params.push(Box::new(min));
            needs_v_join = true;
        }
        if let Some(max) = opts.iso_max {
            clauses.push("v.iso <= ?".to_string());
            params.push(Box::new(max));
            needs_v_join = true;
        }
        if let Some(min) = opts.focal_min {
            clauses.push("v.focal_length_mm >= ?".to_string());
            params.push(Box::new(min));
            needs_v_join = true;
        }
        if let Some(max) = opts.focal_max {
            clauses.push("v.focal_length_mm <= ?".to_string());
            params.push(Box::new(max));
            needs_v_join = true;
        }
        if let Some(min) = opts.f_min {
            clauses.push("v.f_number >= ?".to_string());
            params.push(Box::new(min));
            needs_v_join = true;
        }
        if let Some(max) = opts.f_max {
            clauses.push("v.f_number <= ?".to_string());
            params.push(Box::new(max));
            needs_v_join = true;
        }
        if let Some(min) = opts.width_min {
            clauses.push("v.image_width >= ?".to_string());
            params.push(Box::new(min));
            needs_v_join = true;
        }
        if let Some(min) = opts.height_min {
            clauses.push("v.image_height >= ?".to_string());
            params.push(Box::new(min));
            needs_v_join = true;
        }

        // JSON fallback filters (meta:key=value)
        for (key, value) in &opts.meta_filters {
            clauses.push(format!("json_extract(v.source_metadata, '$.{key}') LIKE ?"));
            params.push(Box::new(format!("%{value}%")));
            needs_v_join = true;
        }

        // Location health filters
        if opts.orphan {
            // Use subquery referencing variants table directly (no v alias needed)
            clauses.push(
                "NOT EXISTS (SELECT 1 FROM file_locations fl2 JOIN variants v2 ON fl2.content_hash = v2.content_hash WHERE v2.asset_id = a.id)"
                    .to_string(),
            );
        }
        if let Some(days) = opts.stale_days {
            clauses.push(format!(
                "EXISTS (SELECT 1 FROM file_locations fl2 \
                 JOIN variants v2 ON fl2.content_hash = v2.content_hash \
                 WHERE v2.asset_id = a.id AND \
                 (fl2.verified_at IS NULL OR fl2.verified_at < datetime('now', '-{} days')))",
                days
            ));
        }
        if let Some(ids) = opts.missing_asset_ids {
            if ids.is_empty() {
                clauses.push("0".to_string()); // no matches
            } else {
                let placeholders: Vec<&str> = ids.iter().map(|_| "?").collect();
                clauses.push(format!("a.id IN ({})", placeholders.join(",")));
                for id in ids {
                    params.push(Box::new(id.clone()));
                }
            }
        }
        if let Some(online_ids) = opts.no_online_locations {
            if !online_ids.is_empty() {
                // Use subquery — no v alias needed
                let placeholders: Vec<&str> = online_ids.iter().map(|_| "?").collect();
                clauses.push(format!(
                    "NOT EXISTS (SELECT 1 FROM file_locations fl2 \
                     JOIN variants v2 ON fl2.content_hash = v2.content_hash \
                     WHERE v2.asset_id = a.id AND fl2.volume_id IN ({}))",
                    placeholders.join(",")
                ));
                for id in online_ids {
                    params.push(Box::new(id.clone()));
                }
            }
            // If online_ids is empty, every asset matches volume:none (no clause needed)
        }

        // Collection filter: restrict to a pre-computed set of asset IDs
        if let Some(ids) = opts.collection_asset_ids {
            if ids.is_empty() {
                clauses.push("0".to_string()); // no matches
            } else {
                let placeholders: Vec<&str> = ids.iter().map(|_| "?").collect();
                clauses.push(format!("a.id IN ({})", placeholders.join(",")));
                for id in ids {
                    params.push(Box::new(id.clone()));
                }
            }
        }

        let where_clause = if clauses.is_empty() {
            " WHERE 1=1".to_string()
        } else {
            format!(" WHERE {}", clauses.join(" AND "))
        };

        // fl join implies v join (fl joins through v)
        if needs_fl_join {
            needs_v_join = true;
        }

        (where_clause, params, needs_fl_join, needs_v_join)
    }

    /// Paginated search with dynamic filters and sorting.
    pub fn search_paginated(&self, opts: &SearchOptions) -> Result<Vec<SearchRow>> {
        let (where_clause, mut params, needs_fl_join, needs_v_join) = Self::build_search_where(opts);

        let mut sql = String::from(
            "SELECT a.id, a.name, a.asset_type, a.created_at, bv.original_filename, bv.format, \
             a.tags, a.description, bv.content_hash, a.rating, a.color_label \
             FROM assets a \
             JOIN variants bv ON bv.content_hash = a.best_variant_hash",
        );

        if needs_v_join {
            sql.push_str(" JOIN variants v ON v.asset_id = a.id");
        }
        if needs_fl_join {
            sql.push_str(" JOIN file_locations fl ON v.content_hash = fl.content_hash");
        }

        sql.push_str(&where_clause);

        if needs_v_join {
            sql.push_str(" GROUP BY a.id");
        }

        sql.push_str(&format!(" ORDER BY {}", opts.sort.to_sql()));

        let page = opts.page.max(1);
        let offset = (page - 1) as u64 * opts.per_page as u64;
        sql.push_str(" LIMIT ? OFFSET ?");
        params.push(Box::new(opts.per_page as u64));
        params.push(Box::new(offset));

        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| p.as_ref()).collect();
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(param_refs.as_slice(), |row| {
            let tags_json: String = row.get(6)?;
            let tags: Vec<String> = serde_json::from_str(&tags_json).unwrap_or_default();
            let rating_val: Option<i64> = row.get(9)?;
            Ok(SearchRow {
                asset_id: row.get(0)?,
                name: row.get(1)?,
                asset_type: row.get(2)?,
                created_at: row.get(3)?,
                original_filename: row.get(4)?,
                format: row.get(5)?,
                tags,
                description: row.get(7)?,
                content_hash: row.get(8)?,
                rating: rating_val.map(|r| r as u8),
                color_label: row.get(10)?,
            })
        })?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    }

    /// Count total results matching the same filters as search_paginated (without LIMIT/OFFSET).
    pub fn search_count(&self, opts: &SearchOptions) -> Result<u64> {
        let (where_clause, params, needs_fl_join, needs_v_join) = Self::build_search_where(opts);

        let count_expr = if needs_v_join { "COUNT(DISTINCT a.id)" } else { "COUNT(*)" };
        let mut sql = format!(
            "SELECT {} FROM assets a \
             JOIN variants bv ON bv.content_hash = a.best_variant_hash",
            count_expr
        );

        if needs_v_join {
            sql.push_str(" JOIN variants v ON v.asset_id = a.id");
        }
        if needs_fl_join {
            sql.push_str(" JOIN file_locations fl ON v.content_hash = fl.content_hash");
        }

        sql.push_str(&where_clause);

        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| p.as_ref()).collect();
        let count: u64 = self.conn.query_row(&sql, param_refs.as_slice(), |r| r.get(0))?;
        Ok(count)
    }

    /// List all unique tags with their usage counts, sorted by count descending.
    pub fn list_all_tags(&self) -> Result<Vec<(String, u64)>> {
        let all_tags_json = self.stats_all_tags_json()?;
        let mut tag_freq: HashMap<String, u64> = HashMap::new();
        for tags_str in &all_tags_json {
            if let Ok(tags) = serde_json::from_str::<Vec<String>>(tags_str) {
                for tag in tags {
                    *tag_freq.entry(tag).or_default() += 1;
                }
            }
        }
        let mut sorted: Vec<(String, u64)> = tag_freq.into_iter().collect();
        sorted.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        Ok(sorted)
    }

    /// List all distinct variant formats.
    pub fn list_all_formats(&self) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT format FROM variants ORDER BY format",
        )?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        rows.collect::<std::result::Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// List all volumes from the catalog's volumes table.
    pub fn list_volumes(&self) -> Result<Vec<(String, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, label FROM volumes ORDER BY label",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect::<std::result::Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Per-volume statistics (before merging device registry).
    fn stats_per_volume(&self) -> Result<Vec<VolumeStatsRaw>> {
        // 1. Core counts per volume
        let mut core: HashMap<String, (String, u64, u64, u64)> = HashMap::new();
        {
            let mut stmt = self.conn.prepare(
                "SELECT fl.volume_id, v.label, COUNT(*) AS loc_count, \
                 COUNT(DISTINCT va.asset_id) AS asset_count, \
                 COALESCE(SUM(va.file_size), 0) AS total_size \
                 FROM file_locations fl \
                 JOIN volumes v ON fl.volume_id = v.id \
                 JOIN variants va ON fl.content_hash = va.content_hash \
                 GROUP BY fl.volume_id",
            )?;
            let rows = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, u64>(2)?,
                    r.get::<_, u64>(3)?,
                    r.get::<_, u64>(4)?,
                ))
            })?;
            for row in rows {
                let (vid, label, loc_count, asset_count, size) = row?;
                core.insert(vid, (label, loc_count, asset_count, size));
            }
        }

        // 2. Unique variant count per volume
        let mut variant_counts: HashMap<String, u64> = HashMap::new();
        {
            let mut stmt = self.conn.prepare(
                "SELECT volume_id, COUNT(DISTINCT content_hash) FROM file_locations GROUP BY volume_id",
            )?;
            let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, u64>(1)?)))?;
            for row in rows {
                let (vid, count) = row?;
                variant_counts.insert(vid, count);
            }
        }

        // 3. Paths per volume (for directory counting)
        let mut dirs_per_vol: HashMap<String, u64> = HashMap::new();
        {
            let mut stmt = self.conn.prepare(
                "SELECT volume_id, relative_path FROM file_locations",
            )?;
            let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
            let mut vol_dirs: HashMap<String, std::collections::HashSet<String>> = HashMap::new();
            for row in rows {
                let (vid, path) = row?;
                let parent = std::path::Path::new(&path)
                    .parent()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_default();
                vol_dirs.entry(vid).or_default().insert(parent);
            }
            for (vid, dirs) in vol_dirs {
                dirs_per_vol.insert(vid, dirs.len() as u64);
            }
        }

        // 4. Formats per volume
        let mut formats_per_vol: HashMap<String, Vec<String>> = HashMap::new();
        {
            let mut stmt = self.conn.prepare(
                "SELECT DISTINCT fl.volume_id, va.format \
                 FROM file_locations fl \
                 JOIN variants va ON fl.content_hash = va.content_hash \
                 ORDER BY fl.volume_id, va.format",
            )?;
            let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
            for row in rows {
                let (vid, fmt) = row?;
                formats_per_vol.entry(vid).or_default().push(fmt);
            }
        }

        // 5. Recipe count per volume
        let mut recipes_per_vol: HashMap<String, u64> = HashMap::new();
        {
            let mut stmt = self.conn.prepare(
                "SELECT volume_id, COUNT(*) FROM recipes WHERE volume_id IS NOT NULL GROUP BY volume_id",
            )?;
            let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, u64>(1)?)))?;
            for row in rows {
                let (vid, count) = row?;
                recipes_per_vol.insert(vid, count);
            }
        }

        // 6. Verification per volume
        let mut verif_per_vol: HashMap<String, (u64, u64, Option<String>)> = HashMap::new();
        {
            let mut stmt = self.conn.prepare(
                "SELECT volume_id, \
                 COUNT(*) AS total, \
                 SUM(CASE WHEN verified_at IS NOT NULL THEN 1 ELSE 0 END) AS verified, \
                 MIN(verified_at) AS oldest \
                 FROM file_locations GROUP BY volume_id",
            )?;
            let rows = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, u64>(1)?,
                    r.get::<_, u64>(2)?,
                    r.get::<_, Option<String>>(3)?,
                ))
            })?;
            for row in rows {
                let (vid, total, verified, oldest) = row?;
                verif_per_vol.insert(vid, (total, verified, oldest));
            }
        }

        // Also include volumes from the volumes table that have no file_locations
        let mut all_volume_ids: HashMap<String, String> = HashMap::new();
        {
            let mut stmt = self.conn.prepare("SELECT id, label FROM volumes")?;
            let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
            for row in rows {
                let (vid, label) = row?;
                all_volume_ids.insert(vid, label);
            }
        }

        // Merge all data
        let mut result = Vec::new();
        for (vid, label) in &all_volume_ids {
            let (core_label, loc_count, asset_count, size) = core
                .get(vid)
                .cloned()
                .unwrap_or_else(|| (label.clone(), 0, 0, 0));
            let _ = core_label; // use label from volumes table
            let variants = *variant_counts.get(vid).unwrap_or(&0);
            let dirs = *dirs_per_vol.get(vid).unwrap_or(&0);
            let formats = formats_per_vol.remove(vid).unwrap_or_default();
            let recipes = *recipes_per_vol.get(vid).unwrap_or(&0);
            let (total_locs, verified, oldest) = verif_per_vol
                .get(vid)
                .cloned()
                .unwrap_or((0, 0, None));

            result.push(VolumeStatsRaw {
                volume_id: vid.clone(),
                label: label.clone(),
                assets: asset_count,
                variants,
                recipes,
                formats,
                directories: dirs,
                size,
                verified_count: verified,
                total_locations: total_locs.max(loc_count),
                oldest_verified_at: oldest,
            });
        }

        result.sort_by(|a, b| a.label.cmp(&b.label));
        Ok(result)
    }

    /// All tags JSON strings from assets table (for parsing + counting in Rust).
    pub fn stats_all_tags_json(&self) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare("SELECT tags FROM assets")?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        rows.collect::<std::result::Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Tag coverage: (tagged_count, untagged_count).
    pub fn stats_tag_coverage(&self) -> Result<(u64, u64)> {
        let tagged: u64 = self.conn.query_row(
            "SELECT COUNT(*) FROM assets WHERE tags != '[]'", [], |r| r.get(0),
        )?;
        let untagged: u64 = self.conn.query_row(
            "SELECT COUNT(*) FROM assets WHERE tags = '[]'", [], |r| r.get(0),
        )?;
        Ok((tagged, untagged))
    }

    /// Verification overview for file_locations:
    /// (total, verified, oldest_verified_at, newest_verified_at).
    pub fn stats_verification_overview(&self) -> Result<(u64, u64, Option<String>, Option<String>)> {
        let total: u64 = self.conn.query_row(
            "SELECT COUNT(*) FROM file_locations", [], |r| r.get(0),
        )?;
        let verified: u64 = self.conn.query_row(
            "SELECT COUNT(*) FROM file_locations WHERE verified_at IS NOT NULL", [], |r| r.get(0),
        )?;
        let oldest: Option<String> = self.conn.query_row(
            "SELECT MIN(verified_at) FROM file_locations WHERE verified_at IS NOT NULL", [], |r| r.get(0),
        )?;
        let newest: Option<String> = self.conn.query_row(
            "SELECT MAX(verified_at) FROM file_locations WHERE verified_at IS NOT NULL", [], |r| r.get(0),
        )?;
        Ok((total, verified, oldest, newest))
    }

    /// Verification counts for recipes: (total, verified).
    pub fn stats_recipe_verification(&self) -> Result<(u64, u64)> {
        let total: u64 = self.conn.query_row(
            "SELECT COUNT(*) FROM recipes", [], |r| r.get(0),
        )?;
        let verified: u64 = self.conn.query_row(
            "SELECT COUNT(*) FROM recipes WHERE verified_at IS NOT NULL", [], |r| r.get(0),
        )?;
        Ok((total, verified))
    }

    /// Per-volume verification: Vec<(label, volume_id, total, verified, oldest_verified_at)>.
    pub fn stats_verification_per_volume(&self) -> Result<Vec<(String, String, u64, u64, Option<String>)>> {
        let mut stmt = self.conn.prepare(
            "SELECT v.label, fl.volume_id, \
             COUNT(*) AS total, \
             SUM(CASE WHEN fl.verified_at IS NOT NULL THEN 1 ELSE 0 END) AS verified, \
             MIN(fl.verified_at) AS oldest \
             FROM file_locations fl \
             JOIN volumes v ON fl.volume_id = v.id \
             GROUP BY fl.volume_id \
             ORDER BY v.label",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, u64>(2)?,
                r.get::<_, u64>(3)?,
                r.get::<_, Option<String>>(4)?,
            ))
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Build full CatalogStats with optional sections.
    pub fn build_stats(
        &self,
        volumes_info: &[(String, String, bool)], // (label, volume_id, is_online)
        show_types: bool,
        show_volumes: bool,
        show_tags: bool,
        show_verified: bool,
        limit: usize,
    ) -> Result<CatalogStats> {
        let (assets, variants, recipes, total_size) = self.stats_overview()?;

        let volumes_total = volumes_info.len() as u64;
        let volumes_online = volumes_info.iter().filter(|v| v.2).count() as u64;
        let volumes_offline = volumes_total - volumes_online;

        let overview = OverviewStats {
            assets,
            variants,
            recipes,
            volumes_total,
            volumes_online,
            volumes_offline,
            total_size,
        };

        let types = if show_types {
            let asset_types_raw = self.stats_asset_types()?;
            let total_assets = assets.max(1) as f64;
            let asset_types: Vec<TypeCount> = asset_types_raw
                .into_iter()
                .map(|(t, c)| TypeCount {
                    asset_type: t,
                    count: c,
                    percentage: (c as f64 / total_assets) * 100.0,
                })
                .collect();

            let variant_formats: Vec<FormatCount> = self
                .stats_variant_formats(limit)?
                .into_iter()
                .map(|(f, c)| FormatCount { format: f, count: c })
                .collect();

            let recipe_paths = self.stats_recipe_paths()?;
            let mut recipe_ext_counts: HashMap<String, u64> = HashMap::new();
            for path in &recipe_paths {
                let ext = std::path::Path::new(path)
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("unknown")
                    .to_lowercase();
                *recipe_ext_counts.entry(ext).or_default() += 1;
            }
            let mut recipe_formats: Vec<FormatCount> = recipe_ext_counts
                .into_iter()
                .map(|(f, c)| FormatCount { format: f, count: c })
                .collect();
            recipe_formats.sort_by(|a, b| b.count.cmp(&a.count));
            if recipe_formats.len() > limit {
                recipe_formats.truncate(limit);
            }

            Some(TypeStats {
                asset_types,
                variant_formats,
                recipe_formats,
            })
        } else {
            None
        };

        let volumes = if show_volumes {
            let raw = self.stats_per_volume()?;
            let vol_stats: Vec<VolumeStats> = raw
                .into_iter()
                .map(|r| {
                    let is_online = volumes_info
                        .iter()
                        .find(|v| v.1 == r.volume_id)
                        .map(|v| v.2)
                        .unwrap_or(false);
                    let verification_pct = if r.total_locations > 0 {
                        (r.verified_count as f64 / r.total_locations as f64) * 100.0
                    } else {
                        0.0
                    };
                    VolumeStats {
                        label: r.label,
                        volume_id: r.volume_id,
                        is_online,
                        assets: r.assets,
                        variants: r.variants,
                        recipes: r.recipes,
                        formats: r.formats,
                        directories: r.directories,
                        size: r.size,
                        verified_count: r.verified_count,
                        total_locations: r.total_locations,
                        verification_pct,
                        oldest_verified_at: r.oldest_verified_at,
                    }
                })
                .collect();
            Some(vol_stats)
        } else {
            None
        };

        let tags = if show_tags {
            let (tagged, untagged) = self.stats_tag_coverage()?;
            let all_tags_json = self.stats_all_tags_json()?;
            let mut tag_freq: HashMap<String, u64> = HashMap::new();
            for tags_str in &all_tags_json {
                if let Ok(tags) = serde_json::from_str::<Vec<String>>(tags_str) {
                    for tag in tags {
                        *tag_freq.entry(tag).or_default() += 1;
                    }
                }
            }
            let unique_tags = tag_freq.len() as u64;
            let mut sorted: Vec<(String, u64)> = tag_freq.into_iter().collect();
            sorted.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
            if sorted.len() > limit {
                sorted.truncate(limit);
            }
            let top_tags: Vec<TagCount> = sorted
                .into_iter()
                .map(|(tag, count)| TagCount { tag, count })
                .collect();

            Some(TagStats {
                unique_tags,
                tagged_assets: tagged,
                untagged_assets: untagged,
                top_tags,
            })
        } else {
            None
        };

        let verified = if show_verified {
            let (total, verified_count, oldest, newest) = self.stats_verification_overview()?;
            let coverage_pct = if total > 0 {
                (verified_count as f64 / total as f64) * 100.0
            } else {
                0.0
            };

            let per_volume_raw = self.stats_verification_per_volume()?;
            let per_volume: Vec<VolumeVerificationStats> = per_volume_raw
                .into_iter()
                .map(|(label, vid, total, verified, oldest)| {
                    let is_online = volumes_info
                        .iter()
                        .find(|v| v.1 == vid)
                        .map(|v| v.2)
                        .unwrap_or(false);
                    let cov = if total > 0 {
                        (verified as f64 / total as f64) * 100.0
                    } else {
                        0.0
                    };
                    VolumeVerificationStats {
                        label,
                        volume_id: vid,
                        is_online,
                        locations: total,
                        verified,
                        coverage_pct: cov,
                        oldest_verified_at: oldest,
                    }
                })
                .collect();

            Some(VerificationStats {
                total_locations: total,
                verified_locations: verified_count,
                unverified_locations: total - verified_count,
                coverage_pct,
                oldest_verified_at: oldest,
                newest_verified_at: newest,
                per_volume,
            })
        } else {
            None
        };

        Ok(CatalogStats {
            overview,
            types,
            volumes,
            tags,
            verified,
        })
    }

    /// Return asset IDs where all variants have zero file_locations.
    pub fn list_orphaned_asset_ids(&self) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT a.id FROM assets a WHERE NOT EXISTS ( \
                 SELECT 1 FROM variants v JOIN file_locations fl ON fl.content_hash = v.content_hash \
                 WHERE v.asset_id = a.id \
             )",
        )?;
        let ids = stmt
            .query_map([], |row| row.get(0))?
            .collect::<std::result::Result<Vec<String>, _>>()?;
        Ok(ids)
    }

    /// Delete all recipes attached to variants of an asset.
    pub fn delete_recipes_for_asset(&self, asset_id: &str) -> Result<usize> {
        let changed = self.conn.execute(
            "DELETE FROM recipes WHERE variant_hash IN (SELECT content_hash FROM variants WHERE asset_id = ?1)",
            rusqlite::params![asset_id],
        )?;
        Ok(changed)
    }

    /// Delete all file_locations for variants of an asset (safety net for true orphans).
    pub fn delete_file_locations_for_asset(&self, asset_id: &str) -> Result<usize> {
        let changed = self.conn.execute(
            "DELETE FROM file_locations WHERE content_hash IN (SELECT content_hash FROM variants WHERE asset_id = ?1)",
            rusqlite::params![asset_id],
        )?;
        Ok(changed)
    }

    /// Delete all variants belonging to an asset.
    pub fn delete_variants_for_asset(&self, asset_id: &str) -> Result<usize> {
        let changed = self.conn.execute(
            "DELETE FROM variants WHERE asset_id = ?1",
            rusqlite::params![asset_id],
        )?;
        Ok(changed)
    }

    /// Return asset IDs where all variants would have zero file_locations
    /// if the given set of stale locations were removed.
    /// Each stale location is `(content_hash, volume_id, relative_path)`.
    pub fn list_would_be_orphaned_asset_ids(
        &self,
        stale_locations: &[(String, String, String)],
    ) -> Result<Vec<String>> {
        if stale_locations.is_empty() {
            return self.list_orphaned_asset_ids();
        }

        // Create a temp table with stale locations to exclude
        self.conn.execute_batch(
            "CREATE TEMP TABLE IF NOT EXISTS _stale_locs (content_hash TEXT, volume_id TEXT, relative_path TEXT)",
        )?;
        self.conn.execute("DELETE FROM _stale_locs", [])?;

        let mut insert = self.conn.prepare(
            "INSERT INTO _stale_locs (content_hash, volume_id, relative_path) VALUES (?1, ?2, ?3)",
        )?;
        for (hash, vol, path) in stale_locations {
            insert.execute(rusqlite::params![hash, vol, path])?;
        }
        drop(insert);

        let mut stmt = self.conn.prepare(
            "SELECT a.id FROM assets a WHERE NOT EXISTS ( \
                 SELECT 1 FROM variants v \
                 JOIN file_locations fl ON fl.content_hash = v.content_hash \
                 WHERE v.asset_id = a.id \
                 AND NOT EXISTS ( \
                     SELECT 1 FROM _stale_locs sl \
                     WHERE sl.content_hash = fl.content_hash \
                     AND sl.volume_id = fl.volume_id \
                     AND sl.relative_path = fl.relative_path \
                 ) \
             )",
        )?;
        let ids = stmt
            .query_map([], |row| row.get(0))?
            .collect::<std::result::Result<Vec<String>, _>>()?;

        self.conn.execute("DROP TABLE IF EXISTS _stale_locs", [])?;

        Ok(ids)
    }

    /// Return all variant content hashes in the catalog.
    pub fn list_all_variant_hashes(&self) -> Result<std::collections::HashSet<String>> {
        let mut stmt = self.conn.prepare("SELECT content_hash FROM variants")?;
        let hashes = stmt
            .query_map([], |row| row.get(0))?
            .collect::<std::result::Result<std::collections::HashSet<String>, _>>()?;
        Ok(hashes)
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
            vec!["assets", "collection_assets", "collections", "file_locations", "recipes", "variants", "volumes"]
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

        let asset = crate::models::Asset::new(crate::models::AssetType::Image, "sha256:test1");
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

        let asset = crate::models::Asset::new(crate::models::AssetType::Image, "sha256:test2");
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

        let mut asset = crate::models::Asset::new(crate::models::AssetType::Image, "sha256:search1");
        asset.name = Some("sunset photo".to_string());
        asset.description = Some("A beautiful sunset over the ocean".to_string());
        asset.tags = vec!["landscape".to_string(), "nature".to_string()];

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
        asset.variants.push(variant.clone());
        catalog.insert_asset(&asset).unwrap();
        catalog.insert_variant(&variant).unwrap();

        // Add a second asset of different type
        let mut asset2 = crate::models::Asset::new(crate::models::AssetType::Video, "sha256:search2");
        asset2.name = Some("holiday clip".to_string());

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
        asset2.variants.push(variant2.clone());
        catalog.insert_asset(&asset2).unwrap();
        catalog.insert_variant(&variant2).unwrap();

        catalog
    }

    #[test]
    fn search_by_text() {
        let catalog = setup_search_catalog();
        let results = catalog.search_assets(Some("sunset"), None, None, None, None, None).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name.as_deref(), Some("sunset photo"));
    }

    #[test]
    fn search_by_type() {
        let catalog = setup_search_catalog();
        let results = catalog.search_assets(None, Some("video"), None, None, None, None).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].format, "mp4");
    }

    #[test]
    fn search_by_tag() {
        let catalog = setup_search_catalog();
        let results = catalog.search_assets(None, None, Some("landscape"), None, None, None).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name.as_deref(), Some("sunset photo"));
    }

    #[test]
    fn search_by_format() {
        let catalog = setup_search_catalog();
        let results = catalog.search_assets(None, None, None, Some("jpg"), None, None).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].original_filename, "sunset_beach.jpg");
    }

    #[test]
    fn search_no_results() {
        let catalog = setup_search_catalog();
        let results = catalog.search_assets(Some("nonexistent"), None, None, None, None, None).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn search_combined_filters() {
        let catalog = setup_search_catalog();
        let results = catalog
            .search_assets(Some("sunset"), Some("image"), Some("landscape"), Some("jpg"), None, None)
            .unwrap();
        assert_eq!(results.len(), 1);
        // Combining mismatched filters yields nothing
        let results = catalog
            .search_assets(Some("sunset"), Some("video"), None, None, None, None)
            .unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn resolve_asset_id_full_match() {
        let catalog = setup_search_catalog();
        let results = catalog.search_assets(None, None, None, None, None, None).unwrap();
        let full_id = &results[0].asset_id;
        let resolved = catalog.resolve_asset_id(full_id).unwrap();
        assert_eq!(resolved.as_deref(), Some(full_id.as_str()));
    }

    #[test]
    fn resolve_asset_id_prefix_match() {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();

        let asset = crate::models::Asset::new(crate::models::AssetType::Image, "sha256:test3");
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
        let a1 = crate::models::Asset::new(crate::models::AssetType::Image, "sha256:ambig1");
        let a2 = crate::models::Asset::new(crate::models::AssetType::Video, "sha256:ambig2");
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
        let results = catalog.search_assets(Some("sunset"), None, None, None, None, None).unwrap();
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

        let mut asset = crate::models::Asset::new(crate::models::AssetType::Image, "sha256:loc1");
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

        let asset = crate::models::Asset::new(crate::models::AssetType::Image, "sha256:findme");
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
    fn list_file_locations_for_asset_works() {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();

        let asset = crate::models::Asset::new(crate::models::AssetType::Image, "sha256:loctest");
        let asset_id = asset.id.to_string();
        catalog.insert_asset(&asset).unwrap();

        let variant = crate::models::Variant {
            content_hash: "sha256:loctest".to_string(),
            asset_id: asset.id,
            role: crate::models::VariantRole::Original,
            format: "jpg".to_string(),
            file_size: 1000,
            original_filename: "test.jpg".to_string(),
            source_metadata: Default::default(),
            locations: vec![],
        };
        catalog.insert_variant(&variant).unwrap();

        let volume = crate::models::Volume::new(
            "test-vol".to_string(),
            std::path::PathBuf::from("/mnt/test"),
            crate::models::VolumeType::Local,
        );
        catalog.ensure_volume(&volume).unwrap();

        let loc = crate::models::FileLocation {
            volume_id: volume.id,
            relative_path: std::path::PathBuf::from("photos/test.jpg"),
            verified_at: None,
        };
        catalog
            .insert_file_location("sha256:loctest", &loc)
            .unwrap();

        let locs = catalog.list_file_locations_for_asset(&asset_id).unwrap();
        assert_eq!(locs.len(), 1);
        assert_eq!(locs[0].0, "sha256:loctest");
        assert_eq!(locs[0].1, "photos/test.jpg");
        assert_eq!(locs[0].2, volume.id.to_string());

        // Non-existent asset returns empty
        let empty = catalog
            .list_file_locations_for_asset("nonexistent")
            .unwrap();
        assert!(empty.is_empty());
    }

    #[test]
    fn delete_asset_removes_row() {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();

        let asset = crate::models::Asset::new(crate::models::AssetType::Image, "sha256:delete1");
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

        let asset1 = crate::models::Asset::new(crate::models::AssetType::Image, "sha256:move1");
        let asset2 = crate::models::Asset::new(crate::models::AssetType::Image, "sha256:move2");
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

    #[test]
    fn rebuild_clears_data_rows() {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();

        // Insert asset + variant + location
        let asset = crate::models::Asset::new(crate::models::AssetType::Image, "sha256:rebuild1");
        catalog.insert_asset(&asset).unwrap();

        let volume = crate::models::Volume::new(
            "vol".to_string(),
            std::path::PathBuf::from("/mnt/vol"),
            crate::models::VolumeType::Local,
        );
        catalog.ensure_volume(&volume).unwrap();

        let variant = crate::models::Variant {
            content_hash: "sha256:rebuild1".to_string(),
            asset_id: asset.id,
            role: crate::models::VariantRole::Original,
            format: "png".to_string(),
            file_size: 100,
            original_filename: "test.png".to_string(),
            source_metadata: Default::default(),
            locations: vec![],
        };
        catalog.insert_variant(&variant).unwrap();

        let loc = crate::models::FileLocation {
            volume_id: volume.id,
            relative_path: std::path::PathBuf::from("test.png"),
            verified_at: None,
        };
        catalog.insert_file_location(&variant.content_hash, &loc).unwrap();

        // Rebuild should clear data rows
        catalog.rebuild().unwrap();

        let count = |table: &str| -> i64 {
            catalog
                .conn
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| row.get(0))
                .unwrap()
        };

        assert_eq!(count("assets"), 0);
        assert_eq!(count("variants"), 0);
        assert_eq!(count("file_locations"), 0);
        // Volumes should be preserved
        assert_eq!(count("volumes"), 1);
    }

    #[test]
    fn find_duplicates_returns_entries_with_multiple_locations() {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();

        let vol1 = crate::models::Volume::new(
            "vol-a".to_string(),
            std::path::PathBuf::from("/mnt/a"),
            crate::models::VolumeType::Local,
        );
        let vol2 = crate::models::Volume::new(
            "vol-b".to_string(),
            std::path::PathBuf::from("/mnt/b"),
            crate::models::VolumeType::Local,
        );
        catalog.ensure_volume(&vol1).unwrap();
        catalog.ensure_volume(&vol2).unwrap();

        // Asset with a variant that has two locations (duplicate)
        let asset = crate::models::Asset::new(crate::models::AssetType::Image, "sha256:dup1");
        catalog.insert_asset(&asset).unwrap();

        let variant = crate::models::Variant {
            content_hash: "sha256:dup1".to_string(),
            asset_id: asset.id,
            role: crate::models::VariantRole::Original,
            format: "jpg".to_string(),
            file_size: 5000,
            original_filename: "photo.jpg".to_string(),
            source_metadata: Default::default(),
            locations: vec![],
        };
        catalog.insert_variant(&variant).unwrap();

        let loc1 = crate::models::FileLocation {
            volume_id: vol1.id,
            relative_path: std::path::PathBuf::from("photos/photo.jpg"),
            verified_at: None,
        };
        let loc2 = crate::models::FileLocation {
            volume_id: vol2.id,
            relative_path: std::path::PathBuf::from("backup/photo.jpg"),
            verified_at: None,
        };
        catalog.insert_file_location(&variant.content_hash, &loc1).unwrap();
        catalog.insert_file_location(&variant.content_hash, &loc2).unwrap();

        // Asset with only one location (not a duplicate)
        let asset2 = crate::models::Asset::new(crate::models::AssetType::Image, "sha256:single1");
        catalog.insert_asset(&asset2).unwrap();

        let variant2 = crate::models::Variant {
            content_hash: "sha256:single1".to_string(),
            asset_id: asset2.id,
            role: crate::models::VariantRole::Original,
            format: "png".to_string(),
            file_size: 1000,
            original_filename: "unique.png".to_string(),
            source_metadata: Default::default(),
            locations: vec![],
        };
        catalog.insert_variant(&variant2).unwrap();

        let loc3 = crate::models::FileLocation {
            volume_id: vol1.id,
            relative_path: std::path::PathBuf::from("photos/unique.png"),
            verified_at: None,
        };
        catalog.insert_file_location(&variant2.content_hash, &loc3).unwrap();

        let dupes = catalog.find_duplicates().unwrap();
        assert_eq!(dupes.len(), 1);
        assert_eq!(dupes[0].content_hash, "sha256:dup1");
        assert_eq!(dupes[0].original_filename, "photo.jpg");
        assert_eq!(dupes[0].locations.len(), 2);
    }

    #[test]
    fn find_duplicates_empty_when_no_duplicates() {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();
        let dupes = catalog.find_duplicates().unwrap();
        assert!(dupes.is_empty());
    }

    #[test]
    fn delete_file_location_removes_row() {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();

        let volume = crate::models::Volume::new(
            "vol".to_string(),
            std::path::PathBuf::from("/mnt/vol"),
            crate::models::VolumeType::Local,
        );
        catalog.ensure_volume(&volume).unwrap();

        let asset = crate::models::Asset::new(crate::models::AssetType::Image, "sha256:delloc1");
        catalog.insert_asset(&asset).unwrap();

        let variant = crate::models::Variant {
            content_hash: "sha256:delloc1".to_string(),
            asset_id: asset.id,
            role: crate::models::VariantRole::Original,
            format: "jpg".to_string(),
            file_size: 100,
            original_filename: "photo.jpg".to_string(),
            source_metadata: Default::default(),
            locations: vec![],
        };
        catalog.insert_variant(&variant).unwrap();

        let loc = crate::models::FileLocation {
            volume_id: volume.id,
            relative_path: std::path::PathBuf::from("photos/photo.jpg"),
            verified_at: None,
        };
        catalog
            .insert_file_location(&variant.content_hash, &loc)
            .unwrap();

        let deleted = catalog
            .delete_file_location("sha256:delloc1", &volume.id.to_string(), "photos/photo.jpg")
            .unwrap();
        assert!(deleted);

        // Verify location is gone
        let details = catalog
            .load_asset_details(&asset.id.to_string())
            .unwrap()
            .unwrap();
        assert!(details.variants[0].locations.is_empty());
    }

    #[test]
    fn delete_file_location_returns_false_for_missing() {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();

        let deleted = catalog
            .delete_file_location("sha256:nope", "some-vol", "some/path.jpg")
            .unwrap();
        assert!(!deleted);
    }

    #[test]
    fn update_recipe_location_changes_values() {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();

        let vol1 = crate::models::Volume::new(
            "vol1".to_string(),
            std::path::PathBuf::from("/mnt/vol1"),
            crate::models::VolumeType::Local,
        );
        let vol2 = crate::models::Volume::new(
            "vol2".to_string(),
            std::path::PathBuf::from("/mnt/vol2"),
            crate::models::VolumeType::Local,
        );
        catalog.ensure_volume(&vol1).unwrap();
        catalog.ensure_volume(&vol2).unwrap();

        let asset = crate::models::Asset::new(crate::models::AssetType::Image, "sha256:rec1");
        catalog.insert_asset(&asset).unwrap();

        let variant = crate::models::Variant {
            content_hash: "sha256:rec1".to_string(),
            asset_id: asset.id,
            role: crate::models::VariantRole::Original,
            format: "nef".to_string(),
            file_size: 500,
            original_filename: "photo.nef".to_string(),
            source_metadata: Default::default(),
            locations: vec![],
        };
        catalog.insert_variant(&variant).unwrap();

        let recipe_id = uuid::Uuid::new_v4();
        let recipe = crate::models::Recipe {
            id: recipe_id,
            variant_hash: "sha256:rec1".to_string(),
            software: "Adobe".to_string(),
            recipe_type: crate::models::RecipeType::Sidecar,
            content_hash: "sha256:recipe_hash".to_string(),
            location: crate::models::FileLocation {
                volume_id: vol1.id,
                relative_path: std::path::PathBuf::from("photos/photo.xmp"),
                verified_at: None,
            },
        };
        catalog.insert_recipe(&recipe).unwrap();

        catalog
            .update_recipe_location(
                &recipe_id.to_string(),
                &vol2.id.to_string(),
                "backup/photo.xmp",
            )
            .unwrap();

        // Verify by querying the recipe
        let row: (String, String) = catalog
            .conn
            .query_row(
                "SELECT volume_id, relative_path FROM recipes WHERE id = ?1",
                rusqlite::params![recipe_id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(row.0, vol2.id.to_string());
        assert_eq!(row.1, "backup/photo.xmp");
    }

    #[test]
    fn update_verified_at_sets_timestamp() {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();

        let volume = crate::models::Volume::new(
            "vol".to_string(),
            std::path::PathBuf::from("/mnt/vol"),
            crate::models::VolumeType::Local,
        );
        catalog.ensure_volume(&volume).unwrap();

        let asset = crate::models::Asset::new(crate::models::AssetType::Image, "sha256:ver1");
        catalog.insert_asset(&asset).unwrap();

        let variant = crate::models::Variant {
            content_hash: "sha256:ver1".to_string(),
            asset_id: asset.id,
            role: crate::models::VariantRole::Original,
            format: "jpg".to_string(),
            file_size: 100,
            original_filename: "photo.jpg".to_string(),
            source_metadata: Default::default(),
            locations: vec![],
        };
        catalog.insert_variant(&variant).unwrap();

        let loc = crate::models::FileLocation {
            volume_id: volume.id,
            relative_path: std::path::PathBuf::from("photos/photo.jpg"),
            verified_at: None,
        };
        catalog.insert_file_location(&variant.content_hash, &loc).unwrap();

        // Initially null
        let before: Option<String> = catalog
            .conn
            .query_row(
                "SELECT verified_at FROM file_locations WHERE content_hash = ?1",
                rusqlite::params!["sha256:ver1"],
                |row| row.get(0),
            )
            .unwrap();
        assert!(before.is_none());

        // Update
        catalog
            .update_verified_at("sha256:ver1", &volume.id.to_string(), "photos/photo.jpg")
            .unwrap();

        let after: Option<String> = catalog
            .conn
            .query_row(
                "SELECT verified_at FROM file_locations WHERE content_hash = ?1",
                rusqlite::params!["sha256:ver1"],
                |row| row.get(0),
            )
            .unwrap();
        assert!(after.is_some());
    }

    #[test]
    fn update_verified_at_noop_for_missing() {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();
        // Should not error, just update 0 rows
        catalog
            .update_verified_at("sha256:nope", "some-vol", "some/path.jpg")
            .unwrap();
    }

    #[test]
    fn update_recipe_location_errors_on_missing() {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();
        let err = catalog
            .update_recipe_location("nonexistent-id", "vol", "path")
            .unwrap_err();
        assert!(err.to_string().contains("No recipe found"));
    }

    /// Helper to set up a catalog with a volume, asset, variant, and recipe for recipe tests.
    fn setup_recipe_catalog() -> (Catalog, crate::models::Volume, crate::models::Asset, String) {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();

        let volume = crate::models::Volume::new(
            "vol".to_string(),
            std::path::PathBuf::from("/mnt/vol"),
            crate::models::VolumeType::Local,
        );
        catalog.ensure_volume(&volume).unwrap();

        let asset = crate::models::Asset::new(crate::models::AssetType::Image, "sha256:rectest");
        catalog.insert_asset(&asset).unwrap();

        let variant = crate::models::Variant {
            content_hash: "sha256:rectest".to_string(),
            asset_id: asset.id,
            role: crate::models::VariantRole::Original,
            format: "nef".to_string(),
            file_size: 1000,
            original_filename: "photo.nef".to_string(),
            source_metadata: Default::default(),
            locations: vec![],
        };
        catalog.insert_variant(&variant).unwrap();

        let loc = crate::models::FileLocation {
            volume_id: volume.id,
            relative_path: std::path::PathBuf::from("photos/photo.nef"),
            verified_at: None,
        };
        catalog.insert_file_location("sha256:rectest", &loc).unwrap();

        let recipe_id = uuid::Uuid::new_v4();
        let recipe = crate::models::Recipe {
            id: recipe_id,
            variant_hash: "sha256:rectest".to_string(),
            software: "Adobe/CaptureOne".to_string(),
            recipe_type: crate::models::RecipeType::Sidecar,
            content_hash: "sha256:recipe_old".to_string(),
            location: crate::models::FileLocation {
                volume_id: volume.id,
                relative_path: std::path::PathBuf::from("photos/photo.xmp"),
                verified_at: None,
            },
        };
        catalog.insert_recipe(&recipe).unwrap();

        (catalog, volume, asset, recipe_id.to_string())
    }

    #[test]
    fn find_recipe_by_location_returns_match() {
        let (catalog, volume, _, recipe_id) = setup_recipe_catalog();
        let result = catalog
            .find_recipe_by_location(
                "sha256:rectest",
                &volume.id.to_string(),
                "photos/photo.xmp",
            )
            .unwrap();
        assert!(result.is_some());
        let (id, hash) = result.unwrap();
        assert_eq!(id, recipe_id);
        assert_eq!(hash, "sha256:recipe_old");
    }

    #[test]
    fn find_recipe_by_location_returns_none() {
        let (catalog, volume, _, _) = setup_recipe_catalog();
        let result = catalog
            .find_recipe_by_location(
                "sha256:rectest",
                &volume.id.to_string(),
                "photos/other.xmp",
            )
            .unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn update_recipe_content_hash_works() {
        let (catalog, volume, _, recipe_id) = setup_recipe_catalog();
        catalog
            .update_recipe_content_hash(&recipe_id, "sha256:recipe_new")
            .unwrap();

        let result = catalog
            .find_recipe_by_location(
                "sha256:rectest",
                &volume.id.to_string(),
                "photos/photo.xmp",
            )
            .unwrap()
            .unwrap();
        assert_eq!(result.1, "sha256:recipe_new");
    }

    #[test]
    fn find_recipe_by_volume_and_path_works() {
        let (catalog, volume, _, recipe_id) = setup_recipe_catalog();
        let result = catalog
            .find_recipe_by_volume_and_path(
                &volume.id.to_string(),
                "photos/photo.xmp",
            )
            .unwrap();
        assert!(result.is_some());
        let (id, hash, variant_hash) = result.unwrap();
        assert_eq!(id, recipe_id);
        assert_eq!(hash, "sha256:recipe_old");
        assert_eq!(variant_hash, "sha256:rectest");

        // Non-existent path returns None
        let result = catalog
            .find_recipe_by_volume_and_path(
                &volume.id.to_string(),
                "photos/nonexistent.xmp",
            )
            .unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn update_recipe_verified_at_works() {
        let (catalog, volume, _, _) = setup_recipe_catalog();
        catalog
            .update_recipe_verified_at(
                "sha256:rectest",
                &volume.id.to_string(),
                "photos/photo.xmp",
            )
            .unwrap();

        let verified: Option<String> = catalog
            .conn
            .query_row(
                "SELECT verified_at FROM recipes WHERE variant_hash = ?1 AND volume_id = ?2 AND relative_path = ?3",
                rusqlite::params!["sha256:rectest", volume.id.to_string(), "photos/photo.xmp"],
                |row| row.get(0),
            )
            .unwrap();
        assert!(verified.is_some());
    }

    #[test]
    fn find_variant_hash_by_stem_and_directory_works() {
        let (catalog, volume, _, _) = setup_recipe_catalog();

        // "photo" stem in "photos" directory should match "photos/photo.nef"
        let result = catalog
            .find_variant_hash_by_stem_and_directory(
                "photo",
                "photos",
                &volume.id.to_string(),
            )
            .unwrap();
        assert!(result.is_some());
        let (hash, _asset_id) = result.unwrap();
        assert_eq!(hash, "sha256:rectest");

        // Wrong stem returns None
        let result = catalog
            .find_variant_hash_by_stem_and_directory(
                "other",
                "photos",
                &volume.id.to_string(),
            )
            .unwrap();
        assert!(result.is_none());

        // Wrong directory returns None
        let result = catalog
            .find_variant_hash_by_stem_and_directory(
                "photo",
                "other_dir",
                &volume.id.to_string(),
            )
            .unwrap();
        assert!(result.is_none());
    }

    // ── Stats tests ──────────────────────────────────────────────

    #[test]
    fn stats_overview_empty_catalog() {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();

        let (assets, variants, recipes, size) = catalog.stats_overview().unwrap();
        assert_eq!(assets, 0);
        assert_eq!(variants, 0);
        assert_eq!(recipes, 0);
        assert_eq!(size, 0);
    }

    #[test]
    fn stats_overview_with_data() {
        let catalog = setup_search_catalog();

        let (assets, variants, recipes, size) = catalog.stats_overview().unwrap();
        assert_eq!(assets, 2);
        assert_eq!(variants, 2);
        assert_eq!(recipes, 0);
        assert_eq!(size, 105_000); // 5000 + 100_000
    }

    #[test]
    fn stats_asset_types_groups_correctly() {
        let catalog = setup_search_catalog();

        let types = catalog.stats_asset_types().unwrap();
        assert_eq!(types.len(), 2);
        // Both should be present (image, video)
        let type_names: Vec<&str> = types.iter().map(|t| t.0.as_str()).collect();
        assert!(type_names.contains(&"image"));
        assert!(type_names.contains(&"video"));
    }

    #[test]
    fn stats_variant_formats_respects_limit() {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();

        // Insert 3 variants with different formats
        for (i, fmt) in ["jpg", "png", "tiff"].iter().enumerate() {
            let hash = format!("sha256:limit{i}");
            let asset = crate::models::Asset::new(crate::models::AssetType::Image, &hash);
            catalog.insert_asset(&asset).unwrap();
            let variant = crate::models::Variant {
                content_hash: hash,
                asset_id: asset.id,
                role: crate::models::VariantRole::Original,
                format: fmt.to_string(),
                file_size: 100,
                original_filename: format!("file.{fmt}"),
                source_metadata: Default::default(),
                locations: vec![],
            };
            catalog.insert_variant(&variant).unwrap();
        }

        let formats = catalog.stats_variant_formats(2).unwrap();
        assert_eq!(formats.len(), 2); // limited to 2
    }

    #[test]
    fn stats_tag_coverage_counts_correctly() {
        let catalog = setup_search_catalog();

        let (tagged, untagged) = catalog.stats_tag_coverage().unwrap();
        // setup_search_catalog: asset1 has tags, asset2 has empty tags
        assert_eq!(tagged, 1);
        assert_eq!(untagged, 1);
    }

    #[test]
    fn stats_per_volume_computes_directories() {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();

        let volume = crate::models::Volume::new(
            "vol".to_string(),
            std::path::PathBuf::from("/mnt/vol"),
            crate::models::VolumeType::Local,
        );
        catalog.ensure_volume(&volume).unwrap();

        let asset = crate::models::Asset::new(crate::models::AssetType::Image, "sha256:dirs1");
        catalog.insert_asset(&asset).unwrap();

        // Two variants in different directories
        for (i, path) in ["photos/a.jpg", "archive/b.jpg"].iter().enumerate() {
            let hash = format!("sha256:dirs{}", i + 1);
            if i > 0 {
                let a2 = crate::models::Asset::new(crate::models::AssetType::Image, &hash);
                catalog.insert_asset(&a2).unwrap();
                let v = crate::models::Variant {
                    content_hash: hash.clone(),
                    asset_id: a2.id,
                    role: crate::models::VariantRole::Original,
                    format: "jpg".to_string(),
                    file_size: 100,
                    original_filename: "b.jpg".to_string(),
                    source_metadata: Default::default(),
                    locations: vec![],
                };
                catalog.insert_variant(&v).unwrap();
            } else {
                let v = crate::models::Variant {
                    content_hash: hash.clone(),
                    asset_id: asset.id,
                    role: crate::models::VariantRole::Original,
                    format: "jpg".to_string(),
                    file_size: 100,
                    original_filename: "a.jpg".to_string(),
                    source_metadata: Default::default(),
                    locations: vec![],
                };
                catalog.insert_variant(&v).unwrap();
            }
            let loc = crate::models::FileLocation {
                volume_id: volume.id,
                relative_path: std::path::PathBuf::from(path),
                verified_at: None,
            };
            catalog.insert_file_location(&format!("sha256:dirs{}", i + 1), &loc).unwrap();
        }

        let raw = catalog.stats_per_volume().unwrap();
        assert_eq!(raw.len(), 1);
        assert_eq!(raw[0].directories, 2); // "photos" and "archive"
        assert_eq!(raw[0].total_locations, 2);
    }

    #[test]
    fn stats_verification_overview_correct() {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();

        let volume = crate::models::Volume::new(
            "vol".to_string(),
            std::path::PathBuf::from("/mnt/vol"),
            crate::models::VolumeType::Local,
        );
        catalog.ensure_volume(&volume).unwrap();

        // Two variants, one with verified_at set
        let a1 = crate::models::Asset::new(crate::models::AssetType::Image, "sha256:v1");
        catalog.insert_asset(&a1).unwrap();
        let v1 = crate::models::Variant {
            content_hash: "sha256:v1".to_string(),
            asset_id: a1.id,
            role: crate::models::VariantRole::Original,
            format: "jpg".to_string(),
            file_size: 100,
            original_filename: "a.jpg".to_string(),
            source_metadata: Default::default(),
            locations: vec![],
        };
        catalog.insert_variant(&v1).unwrap();
        let loc1 = crate::models::FileLocation {
            volume_id: volume.id,
            relative_path: std::path::PathBuf::from("a.jpg"),
            verified_at: Some(chrono::Utc::now()),
        };
        catalog.insert_file_location("sha256:v1", &loc1).unwrap();

        let a2 = crate::models::Asset::new(crate::models::AssetType::Image, "sha256:v2");
        catalog.insert_asset(&a2).unwrap();
        let v2 = crate::models::Variant {
            content_hash: "sha256:v2".to_string(),
            asset_id: a2.id,
            role: crate::models::VariantRole::Original,
            format: "jpg".to_string(),
            file_size: 200,
            original_filename: "b.jpg".to_string(),
            source_metadata: Default::default(),
            locations: vec![],
        };
        catalog.insert_variant(&v2).unwrap();
        let loc2 = crate::models::FileLocation {
            volume_id: volume.id,
            relative_path: std::path::PathBuf::from("b.jpg"),
            verified_at: None,
        };
        catalog.insert_file_location("sha256:v2", &loc2).unwrap();

        let (total, verified, oldest, newest) = catalog.stats_verification_overview().unwrap();
        assert_eq!(total, 2);
        assert_eq!(verified, 1);
        assert!(oldest.is_some());
        assert!(newest.is_some());
    }

    #[test]
    fn build_stats_empty_catalog() {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();

        let stats = catalog.build_stats(&[], true, true, true, true, 20).unwrap();
        assert_eq!(stats.overview.assets, 0);
        assert_eq!(stats.overview.variants, 0);
        assert_eq!(stats.overview.recipes, 0);
        assert_eq!(stats.overview.total_size, 0);
        assert_eq!(stats.overview.volumes_total, 0);
        assert!(stats.types.unwrap().asset_types.is_empty());
        assert!(stats.volumes.unwrap().is_empty());
        assert_eq!(stats.tags.as_ref().unwrap().unique_tags, 0);
        assert_eq!(stats.verified.as_ref().unwrap().total_locations, 0);
    }

    /// Helper to create a catalog with metadata-rich variants for metadata search tests.
    fn setup_metadata_catalog() -> Catalog {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();

        // Asset 1: Fuji camera
        let mut asset1 = crate::models::Asset::new(crate::models::AssetType::Image, "sha256:meta1");

        let mut meta1 = HashMap::new();
        meta1.insert("camera_model".to_string(), "X-T5".to_string());
        meta1.insert("lens_model".to_string(), "XF56mmF1.2 R".to_string());
        meta1.insert("focal_length".to_string(), "56 mm".to_string());
        meta1.insert("f_number".to_string(), "1.2".to_string());
        meta1.insert("iso".to_string(), "400".to_string());
        meta1.insert("image_width".to_string(), "6240".to_string());
        meta1.insert("image_height".to_string(), "4160".to_string());
        meta1.insert("camera_make".to_string(), "FUJIFILM".to_string());

        let variant1 = crate::models::Variant {
            content_hash: "sha256:meta1".to_string(),
            asset_id: asset1.id,
            role: crate::models::VariantRole::Original,
            format: "raf".to_string(),
            file_size: 50_000_000,
            original_filename: "DSCF0001.RAF".to_string(),
            source_metadata: meta1,
            locations: vec![],
        };
        asset1.variants.push(variant1.clone());
        catalog.insert_asset(&asset1).unwrap();
        catalog.insert_variant(&variant1).unwrap();

        // Asset 2: Nikon camera
        let mut asset2 = crate::models::Asset::new(crate::models::AssetType::Image, "sha256:meta2");

        let mut meta2 = HashMap::new();
        meta2.insert("camera_model".to_string(), "Z 6II".to_string());
        meta2.insert("lens_model".to_string(), "NIKKOR Z 24-70mm f/4 S".to_string());
        meta2.insert("focal_length".to_string(), "50 mm".to_string());
        meta2.insert("f_number".to_string(), "4.0".to_string());
        meta2.insert("iso".to_string(), "3200".to_string());
        meta2.insert("image_width".to_string(), "6048".to_string());
        meta2.insert("image_height".to_string(), "4024".to_string());
        meta2.insert("camera_make".to_string(), "NIKON CORPORATION".to_string());
        meta2.insert("label".to_string(), "Red".to_string());

        let variant2 = crate::models::Variant {
            content_hash: "sha256:meta2".to_string(),
            asset_id: asset2.id,
            role: crate::models::VariantRole::Original,
            format: "nef".to_string(),
            file_size: 40_000_000,
            original_filename: "DSC_0001.NEF".to_string(),
            source_metadata: meta2,
            locations: vec![],
        };
        asset2.variants.push(variant2.clone());
        catalog.insert_asset(&asset2).unwrap();
        catalog.insert_variant(&variant2).unwrap();

        catalog
    }

    #[test]
    fn insert_variant_populates_metadata_columns() {
        let catalog = setup_metadata_catalog();

        let row: (Option<String>, Option<String>, Option<f64>, Option<f64>, Option<i64>, Option<i64>, Option<i64>) =
            catalog.conn.query_row(
                "SELECT camera_model, lens_model, focal_length_mm, f_number, iso, image_width, image_height \
                 FROM variants WHERE content_hash = 'sha256:meta1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?)),
            ).unwrap();

        assert_eq!(row.0.as_deref(), Some("X-T5"));
        assert_eq!(row.1.as_deref(), Some("XF56mmF1.2 R"));
        assert!((row.2.unwrap() - 56.0).abs() < 0.01);
        assert!((row.3.unwrap() - 1.2).abs() < 0.01);
        assert_eq!(row.4, Some(400));
        assert_eq!(row.5, Some(6240));
        assert_eq!(row.6, Some(4160));
    }

    #[test]
    fn backfill_metadata_from_json() {
        let catalog = Catalog::open_in_memory().unwrap();
        // First initialize without metadata columns by creating a minimal schema
        catalog.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS assets (
                id TEXT PRIMARY KEY,
                name TEXT,
                created_at TEXT NOT NULL,
                asset_type TEXT NOT NULL,
                tags TEXT NOT NULL DEFAULT '[]',
                description TEXT,
                rating INTEGER
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
                volume_id TEXT NOT NULL,
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
                relative_path TEXT,
                verified_at TEXT
            );"
        ).unwrap();

        // Insert an asset and variant using old schema (no metadata columns)
        let asset = crate::models::Asset::new(crate::models::AssetType::Image, "sha256:old1");
        catalog.conn.execute(
            "INSERT INTO assets (id, name, created_at, asset_type, tags) VALUES (?1, NULL, ?2, 'image', '[]')",
            rusqlite::params![asset.id.to_string(), asset.created_at.to_rfc3339()],
        ).unwrap();

        catalog.conn.execute(
            "INSERT INTO variants (content_hash, asset_id, role, format, file_size, original_filename, source_metadata) \
             VALUES ('sha256:old1', ?1, 'original', 'nef', 30000000, 'OLD.NEF', \
             '{\"camera_model\":\"D850\",\"iso\":\"800\",\"focal_length\":\"70 mm\",\"f_number\":\"2.8\"}')",
            rusqlite::params![asset.id.to_string()],
        ).unwrap();

        // Now run initialize() which should add columns and backfill
        catalog.initialize().unwrap();

        let row: (Option<String>, Option<i64>, Option<f64>, Option<f64>) =
            catalog.conn.query_row(
                "SELECT camera_model, iso, focal_length_mm, f_number FROM variants WHERE content_hash = 'sha256:old1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            ).unwrap();

        assert_eq!(row.0.as_deref(), Some("D850"));
        assert_eq!(row.1, Some(800));
        assert!((row.2.unwrap() - 70.0).abs() < 0.01);
        assert!((row.3.unwrap() - 2.8).abs() < 0.01);
    }

    #[test]
    fn search_by_camera() {
        let catalog = setup_metadata_catalog();
        let opts = SearchOptions {
            camera: Some("X-T5"),
            per_page: u32::MAX,
            ..Default::default()
        };
        let results = catalog.search_paginated(&opts).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].original_filename, "DSCF0001.RAF");
    }

    #[test]
    fn search_by_camera_partial() {
        let catalog = setup_metadata_catalog();
        let opts = SearchOptions {
            camera: Some("Z 6"),
            per_page: u32::MAX,
            ..Default::default()
        };
        let results = catalog.search_paginated(&opts).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].original_filename, "DSC_0001.NEF");
    }

    #[test]
    fn search_by_iso_exact_and_range() {
        let catalog = setup_metadata_catalog();

        // Exact ISO
        let opts = SearchOptions {
            iso_min: Some(400),
            iso_max: Some(400),
            per_page: u32::MAX,
            ..Default::default()
        };
        let results = catalog.search_paginated(&opts).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].original_filename, "DSCF0001.RAF");

        // ISO range 100-800: should match Fuji (400) only
        let opts = SearchOptions {
            iso_min: Some(100),
            iso_max: Some(800),
            per_page: u32::MAX,
            ..Default::default()
        };
        let results = catalog.search_paginated(&opts).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].original_filename, "DSCF0001.RAF");

        // ISO min 1000+: should match Nikon (3200) only
        let opts = SearchOptions {
            iso_min: Some(1000),
            per_page: u32::MAX,
            ..Default::default()
        };
        let results = catalog.search_paginated(&opts).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].original_filename, "DSC_0001.NEF");
    }

    #[test]
    fn search_by_focal_range() {
        let catalog = setup_metadata_catalog();

        // focal 50-56: should match both
        let opts = SearchOptions {
            focal_min: Some(50.0),
            focal_max: Some(56.0),
            per_page: u32::MAX,
            ..Default::default()
        };
        let results = catalog.search_paginated(&opts).unwrap();
        assert_eq!(results.len(), 2);

        // focal 55-60: should match Fuji (56mm) only
        let opts = SearchOptions {
            focal_min: Some(55.0),
            focal_max: Some(60.0),
            per_page: u32::MAX,
            ..Default::default()
        };
        let results = catalog.search_paginated(&opts).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].original_filename, "DSCF0001.RAF");
    }

    #[test]
    fn search_text_includes_metadata() {
        let catalog = setup_metadata_catalog();

        // "FUJIFILM" exists in source_metadata JSON but not in name/filename/description
        let opts = SearchOptions {
            text: Some("FUJIFILM"),
            per_page: u32::MAX,
            ..Default::default()
        };
        let results = catalog.search_paginated(&opts).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].original_filename, "DSCF0001.RAF");
    }

    #[test]
    fn search_meta_json_extract() {
        let catalog = setup_metadata_catalog();

        // meta:label=Red should match the Nikon variant
        let opts = SearchOptions {
            meta_filters: vec![("label", "Red")],
            per_page: u32::MAX,
            ..Default::default()
        };
        let results = catalog.search_paginated(&opts).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].original_filename, "DSC_0001.NEF");
    }
}
