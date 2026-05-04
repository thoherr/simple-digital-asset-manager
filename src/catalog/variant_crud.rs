//! `variant_crud` section of `Catalog` — methods extracted from the original
//! 9.2-kLOC catalog.rs into a multi-file `impl Catalog` block.
//!
//! Types, helper functions, and the connection ctor live in the parent
//! `catalog` module.

use super::*;

impl Catalog {
    // ═══ VARIANT & LOCATION CRUD ═══

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
        // Check if this exact location already exists (no unique constraint on table)
        let exists: bool = self.conn.query_row(
            "SELECT COUNT(*) > 0 FROM file_locations WHERE content_hash = ?1 AND volume_id = ?2 AND relative_path = ?3",
            rusqlite::params![content_hash, loc.volume_id.to_string(), loc.relative_path_str()],
            |r| r.get(0),
        )?;
        if exists {
            return Ok(());
        }
        self.conn.execute(
            "INSERT INTO file_locations (content_hash, volume_id, relative_path, verified_at) \
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![
                content_hash,
                loc.volume_id.to_string(),
                loc.relative_path_str(),
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

}
