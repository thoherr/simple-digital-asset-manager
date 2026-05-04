//! `asset_crud` section of `Catalog` — methods extracted from the original
//! 9.2-kLOC catalog.rs into a multi-file `impl Catalog` block.
//!
//! Types, helper functions, and the connection ctor live in the parent
//! `catalog` module.

use super::*;

impl Catalog {
    // ═══ ASSET CRUD ═══

    /// Insert an asset into the catalog.
    pub fn insert_asset(&self, asset: &Asset) -> Result<()> {
        let tags_json = serde_json::to_string(&asset.tags)?;
        let best_hash = crate::models::variant::compute_best_variant_hash_with_override(
            &asset.variants,
            asset.preview_variant.as_deref(),
        );
        let primary_format = crate::models::variant::compute_primary_format(&asset.variants);
        let variant_count = asset.variants.len() as i64;
        let (latitude, longitude) = crate::models::variant::compute_gps_from_variants(&asset.variants);
        // Compute video duration from first variant that has it
        let video_duration: Option<f64> = asset.variants.iter()
            .find_map(|v| v.source_metadata.get("video_duration")?.parse::<f64>().ok());
        let video_codec: Option<String> = asset.variants.iter()
            .find_map(|v| v.source_metadata.get("video_codec").cloned());
        // Leaf tag count — denormalised for the `tagcount:` search filter.
        // Computed from the current tags list, so any call that saves the
        // asset (tag add/remove/rename/split/clear/reimport) picks up the
        // new count for free.
        let leaf_tag_count = crate::tag_util::leaf_tag_count(&asset.tags) as i64;
        // Use ON CONFLICT UPDATE instead of INSERT OR REPLACE to avoid
        // intermediate DELETE that triggers FK constraint violations on
        // variants/faces/collection_assets referencing this asset.
        self.conn.execute(
            "INSERT INTO assets (id, name, created_at, asset_type, tags, description, rating, color_label, best_variant_hash, primary_variant_format, variant_count, latitude, longitude, preview_rotation, preview_variant, video_duration, video_codec, face_scan_status, leaf_tag_count) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19) \
             ON CONFLICT(id) DO UPDATE SET \
               name = excluded.name, \
               created_at = excluded.created_at, \
               asset_type = excluded.asset_type, \
               tags = excluded.tags, \
               description = excluded.description, \
               rating = excluded.rating, \
               color_label = excluded.color_label, \
               best_variant_hash = excluded.best_variant_hash, \
               primary_variant_format = excluded.primary_variant_format, \
               variant_count = excluded.variant_count, \
               latitude = excluded.latitude, \
               longitude = excluded.longitude, \
               preview_rotation = excluded.preview_rotation, \
               preview_variant = excluded.preview_variant, \
               video_duration = excluded.video_duration, \
               video_codec = excluded.video_codec, \
               face_scan_status = excluded.face_scan_status, \
               leaf_tag_count = excluded.leaf_tag_count",
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
                primary_format,
                variant_count,
                latitude,
                longitude,
                asset.preview_rotation.map(|r| r as i64),
                asset.preview_variant,
                video_duration,
                video_codec,
                asset.face_scan_status.as_deref(),
                leaf_tag_count,
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

    /// Update just the preview rotation for an asset in the catalog.
    pub fn update_asset_preview_rotation(&self, asset_id: &str, rotation: Option<u16>) -> Result<()> {
        self.conn.execute(
            "UPDATE assets SET preview_rotation = ?1 WHERE id = ?2",
            rusqlite::params![rotation.map(|r| r as i64), asset_id],
        )?;
        Ok(())
    }

    /// Update the preview variant override and recompute best_variant_hash.
    pub fn update_asset_preview_variant(
        &self,
        asset_id: &str,
        preview_variant: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE assets SET preview_variant = ?1 WHERE id = ?2",
            rusqlite::params![preview_variant, asset_id],
        )?;
        // Recompute best_variant_hash: if override set, use it; else fall back to scoring
        if let Some(hash) = preview_variant {
            self.conn.execute(
                "UPDATE assets SET best_variant_hash = ?1 WHERE id = ?2 AND EXISTS (SELECT 1 FROM variants WHERE content_hash = ?1 AND asset_id = ?2)",
                rusqlite::params![hash, asset_id],
            )?;
        } else {
            // Clear override — recompute from scoring via SQL
            self.conn.execute(
                "UPDATE assets SET best_variant_hash = (
                    SELECT content_hash FROM variants WHERE asset_id = ?1
                    ORDER BY
                        CASE role WHEN 'export' THEN 300 WHEN 'processed' THEN 200
                            WHEN 'original' THEN 100 ELSE 0 END +
                        CASE WHEN LOWER(format) IN ('jpg','jpeg','png','tiff','tif','webp')
                            THEN 50 ELSE 0 END +
                        MIN(file_size / 1000000, 49)
                    DESC LIMIT 1
                ) WHERE id = ?1",
                rusqlite::params![asset_id],
            )?;
        }
        Ok(())
    }

    /// Update the denormalized face_count for an asset.
    /// Recomputes from the faces table (requires faces table to exist).
    pub fn update_face_count(&self, asset_id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE assets SET face_count = (SELECT COUNT(*) FROM faces WHERE asset_id = ?1) WHERE id = ?1",
            rusqlite::params![asset_id],
        )?;
        Ok(())
    }

    /// Mark an asset as having been scanned for faces (regardless of face count).
    /// Used by `maki faces detect` to avoid re-scanning zero-face assets on every run.
    pub fn mark_face_scan_done(&self, asset_id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE assets SET face_scan_status = 'done' WHERE id = ?1",
            rusqlite::params![asset_id],
        )?;
        Ok(())
    }

    /// Clear the face-scan-done flag for an asset. Used by `--force` to ensure
    /// the asset gets re-scanned even if it had been marked done previously.
    pub fn clear_face_scan_status(&self, asset_id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE assets SET face_scan_status = NULL WHERE id = ?1",
            rusqlite::params![asset_id],
        )?;
        Ok(())
    }

    /// Check whether an asset has been scanned for faces (regardless of whether
    /// any were found). Returns true if `face_scan_status = 'done'`.
    pub fn is_face_scan_done(&self, asset_id: &str) -> bool {
        self.conn
            .query_row(
                "SELECT 1 FROM assets WHERE id = ?1 AND face_scan_status = 'done'",
                rusqlite::params![asset_id],
                |_| Ok(()),
            )
            .is_ok()
    }

    /// Update just the created_at date for an asset in the catalog.
    pub fn update_asset_created_at(&self, asset_id: &str, created_at: &chrono::DateTime<chrono::Utc>) -> Result<()> {
        self.conn.execute(
            "UPDATE assets SET created_at = ?1 WHERE id = ?2",
            rusqlite::params![created_at.to_rfc3339(), asset_id],
        )?;
        Ok(())
    }

    /// Update denormalized variant columns for an asset.
    pub fn update_best_variant_hash(&self, asset_id: &str, hash: Option<&str>) -> Result<()> {
        self.conn.execute(
            "UPDATE assets SET best_variant_hash = ?1 WHERE id = ?2",
            rusqlite::params![hash, asset_id],
        )?;
        Ok(())
    }

    /// Update all denormalized variant columns from an asset's variants.
    pub fn update_denormalized_variant_columns(&self, asset: &Asset) -> Result<()> {
        let best_hash = crate::models::variant::compute_best_variant_hash_with_override(
            &asset.variants,
            asset.preview_variant.as_deref(),
        );
        let primary_format = crate::models::variant::compute_primary_format(&asset.variants);
        let variant_count = asset.variants.len() as i64;
        let (latitude, longitude) = crate::models::variant::compute_gps_from_variants(&asset.variants);
        self.conn.execute(
            "UPDATE assets SET best_variant_hash = ?1, primary_variant_format = ?2, variant_count = ?3, latitude = ?4, longitude = ?5 WHERE id = ?6",
            rusqlite::params![best_hash, primary_format, variant_count, latitude, longitude, asset.id.to_string()],
        )?;
        Ok(())
    }

}
