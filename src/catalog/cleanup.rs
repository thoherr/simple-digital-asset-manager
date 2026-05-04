//! `cleanup` section of `Catalog` — methods extracted from the original
//! 9.2-kLOC catalog.rs into a multi-file `impl Catalog` block.
//!
//! Types, helper functions, and the connection ctor live in the parent
//! `catalog` module.

use super::*;

impl Catalog {
    // ═══ CLEANUP QUERIES ═══

    /// Return asset IDs where all variants have zero file_locations.
    /// Find asset IDs that have at least one file location with stale or missing verification.
    /// Used by verify --max-age to skip loading sidecars for fully-verified assets.
    /// Count total file locations, optionally filtered by volume.
    pub fn count_file_locations(&self, volume_id: Option<&str>) -> Result<usize> {
        let (sql, params): (String, Vec<Box<dyn rusqlite::types::ToSql>>) = if let Some(vid) = volume_id {
            ("SELECT COUNT(*) FROM file_locations WHERE volume_id = ?1".to_string(),
             vec![Box::new(vid.to_string())])
        } else {
            ("SELECT COUNT(*) FROM file_locations".to_string(), vec![])
        };
        let count: usize = self.conn.query_row(&sql, rusqlite::params_from_iter(params.iter()), |r| r.get(0))?;
        Ok(count)
    }

    /// Count file locations with stale or missing verification.
    pub fn count_stale_locations(&self, max_age_days: u64, volume_id: Option<&str>) -> Result<usize> {
        let cutoff = (chrono::Utc::now() - chrono::Duration::days(max_age_days as i64)).to_rfc3339();
        let (sql, params): (String, Vec<Box<dyn rusqlite::types::ToSql>>) = if let Some(vid) = volume_id {
            ("SELECT COUNT(*) FROM file_locations WHERE volume_id = ?1 AND (verified_at IS NULL OR verified_at < ?2)".to_string(),
             vec![Box::new(vid.to_string()), Box::new(cutoff)])
        } else {
            ("SELECT COUNT(*) FROM file_locations WHERE verified_at IS NULL OR verified_at < ?1".to_string(),
             vec![Box::new(cutoff)])
        };
        let count: usize = self.conn.query_row(&sql, rusqlite::params_from_iter(params.iter()), |r| r.get(0))?;
        Ok(count)
    }

    pub fn find_assets_with_stale_locations(&self, max_age_days: u64, volume_id: Option<&str>) -> Result<Vec<String>> {
        let cutoff = (chrono::Utc::now() - chrono::Duration::days(max_age_days as i64)).to_rfc3339();
        let sql = if let Some(vid) = volume_id {
            format!(
                "SELECT DISTINCT v.asset_id FROM file_locations fl \
                 JOIN variants v ON fl.content_hash = v.content_hash \
                 WHERE fl.volume_id = '{}' AND (fl.verified_at IS NULL OR fl.verified_at < ?1)",
                vid.replace('\'', "''")
            )
        } else {
            "SELECT DISTINCT v.asset_id FROM file_locations fl \
             JOIN variants v ON fl.content_hash = v.content_hash \
             WHERE fl.verified_at IS NULL OR fl.verified_at < ?1".to_string()
        };
        let mut stmt = self.conn.prepare(&sql)?;
        let ids = stmt
            .query_map(rusqlite::params![cutoff], |r| r.get(0))?
            .collect::<std::result::Result<Vec<String>, _>>()?;
        Ok(ids)
    }

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

    /// Delete all collection memberships for an asset.
    pub fn delete_collection_memberships_for_asset(&self, asset_id: &str) -> Result<usize> {
        let changed = self.conn.execute(
            "DELETE FROM collection_assets WHERE asset_id = ?1",
            rusqlite::params![asset_id],
        )?;
        Ok(changed)
    }

    /// List all variant content hashes for an asset.
    pub fn list_variant_hashes_for_asset(&self, asset_id: &str) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT content_hash FROM variants WHERE asset_id = ?1",
        )?;
        let rows = stmt.query_map(rusqlite::params![asset_id], |row| {
            row.get::<_, String>(0)
        })?;
        let mut hashes = Vec::new();
        for row in rows {
            hashes.push(row?);
        }
        Ok(hashes)
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

    /// List variants that have zero file_locations, returning (asset_id, content_hash) pairs.
    /// Only returns variants belonging to assets that still have at least one other variant with locations.
    pub fn list_locationless_variants(&self) -> Result<Vec<(String, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT v.asset_id, v.content_hash FROM variants v \
             WHERE NOT EXISTS (SELECT 1 FROM file_locations fl WHERE fl.content_hash = v.content_hash) \
             AND EXISTS ( \
                 SELECT 1 FROM variants v2 \
                 JOIN file_locations fl2 ON fl2.content_hash = v2.content_hash \
                 WHERE v2.asset_id = v.asset_id AND v2.content_hash != v.content_hash \
             )",
        )?;
        let pairs = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<std::result::Result<Vec<(String, String)>, _>>()?;
        Ok(pairs)
    }

    /// Predict locationless variants after a set of stale locations would be removed.
    pub fn list_would_be_locationless_variants(
        &self,
        stale_locations: &[(String, String, String)],
    ) -> Result<Vec<(String, String)>> {
        if stale_locations.is_empty() {
            return self.list_locationless_variants();
        }

        self.conn.execute_batch(
            "CREATE TEMP TABLE IF NOT EXISTS _stale_locs2 (content_hash TEXT, volume_id TEXT, relative_path TEXT)",
        )?;
        self.conn.execute("DELETE FROM _stale_locs2", [])?;

        let mut insert = self.conn.prepare(
            "INSERT INTO _stale_locs2 (content_hash, volume_id, relative_path) VALUES (?1, ?2, ?3)",
        )?;
        for (hash, vol, path) in stale_locations {
            insert.execute(rusqlite::params![hash, vol, path])?;
        }
        drop(insert);

        // A variant is "locationless" if all its remaining locations are in the stale set
        // AND the asset has at least one other variant that still has non-stale locations
        let mut stmt = self.conn.prepare(
            "SELECT v.asset_id, v.content_hash FROM variants v \
             WHERE NOT EXISTS ( \
                 SELECT 1 FROM file_locations fl WHERE fl.content_hash = v.content_hash \
                 AND NOT EXISTS ( \
                     SELECT 1 FROM _stale_locs2 sl \
                     WHERE sl.content_hash = fl.content_hash \
                     AND sl.volume_id = fl.volume_id \
                     AND sl.relative_path = fl.relative_path \
                 ) \
             ) \
             AND EXISTS ( \
                 SELECT 1 FROM variants v2 \
                 JOIN file_locations fl2 ON fl2.content_hash = v2.content_hash \
                 WHERE v2.asset_id = v.asset_id AND v2.content_hash != v.content_hash \
                 AND NOT EXISTS ( \
                     SELECT 1 FROM _stale_locs2 sl2 \
                     WHERE sl2.content_hash = fl2.content_hash \
                     AND sl2.volume_id = fl2.volume_id \
                     AND sl2.relative_path = fl2.relative_path \
                 ) \
             )",
        )?;
        let pairs = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<std::result::Result<Vec<(String, String)>, _>>()?;

        self.conn.execute("DROP TABLE IF EXISTS _stale_locs2", [])?;

        Ok(pairs)
    }

    /// Delete a single variant by content_hash. Also deletes its file_locations and recipes.
    pub fn delete_variant(&self, content_hash: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM recipes WHERE variant_hash = ?1",
            rusqlite::params![content_hash],
        )?;
        self.conn.execute(
            "DELETE FROM file_locations WHERE content_hash = ?1",
            rusqlite::params![content_hash],
        )?;
        self.conn.execute(
            "DELETE FROM embeddings WHERE asset_id IN (SELECT asset_id FROM variants WHERE content_hash = ?1)",
            rusqlite::params![content_hash],
        )?;
        self.conn.execute(
            "DELETE FROM variants WHERE content_hash = ?1",
            rusqlite::params![content_hash],
        )?;
        Ok(())
    }

    /// Return all variant content hashes in the catalog.
    pub fn list_all_variant_hashes(&self) -> Result<std::collections::HashSet<String>> {
        let mut stmt = self.conn.prepare("SELECT content_hash FROM variants")?;
        let hashes = stmt
            .query_map([], |row| row.get(0))?
            .collect::<std::result::Result<std::collections::HashSet<String>, _>>()?;
        Ok(hashes)
    }

    pub fn list_all_asset_ids(&self) -> Result<std::collections::HashSet<String>> {
        let mut stmt = self.conn.prepare("SELECT id FROM assets")?;
        let ids = stmt
            .query_map([], |row| row.get(0))?
            .collect::<std::result::Result<std::collections::HashSet<String>, _>>()?;
        Ok(ids)
    }
}
