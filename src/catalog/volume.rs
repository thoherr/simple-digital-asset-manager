//! `volume` section of `Catalog` — methods extracted from the original
//! 9.2-kLOC catalog.rs into a multi-file `impl Catalog` block.
//!
//! Types, helper functions, and the connection ctor live in the parent
//! `catalog` module.

use super::*;

impl Catalog {
    // ═══ VOLUME OPERATIONS ═══

    /// Ensure a volume record exists in the SQLite cache.
    pub fn ensure_volume(&self, volume: &crate::models::Volume) -> Result<()> {
        self.conn.execute(
            "INSERT INTO volumes (id, label, mount_point, volume_type, purpose) \
             VALUES (?1, ?2, ?3, ?4, ?5) \
             ON CONFLICT(id) DO UPDATE SET purpose = excluded.purpose",
            rusqlite::params![
                volume.id.to_string(),
                volume.label,
                volume.mount_point.to_string_lossy().to_string(),
                format!("{:?}", volume.volume_type).to_lowercase(),
                volume.purpose.as_ref().map(|p| p.as_str()),
            ],
        )?;
        Ok(())
    }

    /// Delete a volume row from the catalog.
    pub fn delete_volume(&self, volume_id: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM volumes WHERE id = ?1",
            rusqlite::params![volume_id],
        )?;
        Ok(())
    }

    /// Rename a volume in the catalog.
    pub fn rename_volume(&self, volume_id: &str, new_label: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE volumes SET label = ?1 WHERE id = ?2",
            rusqlite::params![new_label, volume_id],
        )?;
        Ok(())
    }

    /// Count file_location rows on a given volume.
    pub fn count_locations_for_volume(&self, volume_id: &str) -> Result<usize> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM file_locations WHERE volume_id = ?1",
            rusqlite::params![volume_id],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    /// Count recipe rows on a given volume.
    pub fn count_recipes_for_volume(&self, volume_id: &str) -> Result<usize> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM recipes WHERE volume_id = ?1",
            rusqlite::params![volume_id],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    /// List distinct asset IDs that have file_locations or recipes on a given volume.
    pub fn list_asset_ids_on_volume(&self, volume_id: &str) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT v.asset_id FROM file_locations fl \
             JOIN variants v ON v.content_hash = fl.content_hash \
             WHERE fl.volume_id = ?1 \
             UNION \
             SELECT DISTINCT v.asset_id FROM recipes r \
             JOIN variants v ON v.content_hash = r.variant_hash \
             WHERE r.volume_id = ?1",
        )?;
        let rows = stmt.query_map(rusqlite::params![volume_id], |row| {
            row.get::<_, String>(0)
        })?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }

    /// Bulk-move all file_locations from one volume to another, prepending a path prefix.
    /// Returns the number of rows updated.
    pub fn bulk_move_file_locations(
        &self,
        source_volume_id: &str,
        target_volume_id: &str,
        prefix: &str,
    ) -> Result<usize> {
        let changed = self.conn.execute(
            "UPDATE file_locations SET volume_id = ?1, relative_path = ?2 || relative_path \
             WHERE volume_id = ?3",
            rusqlite::params![target_volume_id, prefix, source_volume_id],
        )?;
        Ok(changed)
    }

    /// Bulk-move all recipes from one volume to another, prepending a path prefix.
    /// Returns the number of rows updated.
    pub fn bulk_move_recipes(
        &self,
        source_volume_id: &str,
        target_volume_id: &str,
        prefix: &str,
    ) -> Result<usize> {
        let changed = self.conn.execute(
            "UPDATE recipes SET volume_id = ?1, relative_path = ?2 || relative_path \
             WHERE volume_id = ?3",
            rusqlite::params![target_volume_id, prefix, source_volume_id],
        )?;
        Ok(changed)
    }

    /// List asset IDs on a volume whose file locations match a path prefix.
    pub fn list_asset_ids_on_volume_with_prefix(&self, volume_id: &str, prefix: &str) -> Result<Vec<String>> {
        let pattern = format!("{prefix}%");
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT v.asset_id FROM file_locations fl \
             JOIN variants v ON v.content_hash = fl.content_hash \
             WHERE fl.volume_id = ?1 AND fl.relative_path LIKE ?2 \
             UNION \
             SELECT DISTINCT v.asset_id FROM recipes r \
             JOIN variants v ON v.content_hash = r.variant_hash \
             WHERE r.volume_id = ?1 AND r.relative_path LIKE ?2",
        )?;
        let rows = stmt.query_map(rusqlite::params![volume_id, pattern], |row| {
            row.get::<_, String>(0)
        })?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }

    /// Bulk-move file_locations matching a prefix from source to target volume, stripping the prefix.
    /// Returns the number of rows updated.
    pub fn bulk_split_file_locations(
        &self,
        source_volume_id: &str,
        target_volume_id: &str,
        prefix: &str,
    ) -> Result<usize> {
        let changed = self.conn.execute(
            "UPDATE file_locations SET volume_id = ?1, relative_path = SUBSTR(relative_path, ?2) \
             WHERE volume_id = ?3 AND relative_path LIKE ?4",
            rusqlite::params![target_volume_id, prefix.len() + 1, source_volume_id, format!("{prefix}%")],
        )?;
        Ok(changed)
    }

    /// Bulk-move recipes matching a prefix from source to target volume, stripping the prefix.
    /// Returns the number of rows updated.
    pub fn bulk_split_recipes(
        &self,
        source_volume_id: &str,
        target_volume_id: &str,
        prefix: &str,
    ) -> Result<usize> {
        let changed = self.conn.execute(
            "UPDATE recipes SET volume_id = ?1, relative_path = SUBSTR(relative_path, ?2) \
             WHERE volume_id = ?3 AND relative_path LIKE ?4",
            rusqlite::params![target_volume_id, prefix.len() + 1, source_volume_id, format!("{prefix}%")],
        )?;
        Ok(changed)
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

    /// Look up a variant's format by its content hash.
    pub fn get_variant_format(&self, content_hash: &str) -> Result<Option<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT format FROM variants WHERE content_hash = ?1 LIMIT 1",
        )?;
        let mut rows = stmt.query(rusqlite::params![content_hash])?;
        match rows.next()? {
            Some(row) => Ok(Some(row.get(0)?)),
            None => Ok(None),
        }
    }

    /// Look up file locations for a variant by content hash.
    /// Returns (volume_id, relative_path) pairs.
    pub fn get_variant_file_locations(&self, content_hash: &str) -> Result<Vec<(String, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT volume_id, relative_path FROM file_locations WHERE content_hash = ?1",
        )?;
        let rows = stmt.query_map(rusqlite::params![content_hash], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Open an in-memory catalog (for testing).
    #[cfg(test)]
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        Ok(Self { conn })
    }

}
