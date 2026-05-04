//! `duplicates` section of `Catalog` — methods extracted from the original
//! 9.2-kLOC catalog.rs into a multi-file `impl Catalog` block.
//!
//! Types, helper functions, and the connection ctor live in the parent
//! `catalog` module.

use super::*;

impl Catalog {
    // ═══ DUPLICATES ═══

    /// Find variants that have more than one file location (duplicates).
    pub fn find_duplicates(&self) -> Result<Vec<DuplicateEntry>> {
        self.load_duplicate_entries(
            "SELECT v.content_hash, v.original_filename, v.format, v.file_size, a.name, a.id \
             FROM variants v \
             JOIN assets a ON v.asset_id = a.id \
             WHERE v.content_hash IN ( \
                 SELECT content_hash FROM file_locations \
                 GROUP BY content_hash HAVING COUNT(*) > 1 \
             ) \
             ORDER BY v.file_size DESC",
        )
    }

    /// Find variants with 2+ locations on the **same** volume.
    pub fn find_duplicates_same_volume(&self) -> Result<Vec<DuplicateEntry>> {
        self.load_duplicate_entries(
            "SELECT v.content_hash, v.original_filename, v.format, v.file_size, a.name, a.id \
             FROM variants v \
             JOIN assets a ON v.asset_id = a.id \
             WHERE v.content_hash IN ( \
                 SELECT content_hash FROM file_locations \
                 GROUP BY content_hash, volume_id HAVING COUNT(*) > 1 \
             ) \
             ORDER BY v.file_size DESC",
        )
    }

    /// Find variants with locations on 2+ **different** volumes.
    pub fn find_duplicates_cross_volume(&self) -> Result<Vec<DuplicateEntry>> {
        self.load_duplicate_entries(
            "SELECT v.content_hash, v.original_filename, v.format, v.file_size, a.name, a.id \
             FROM variants v \
             JOIN assets a ON v.asset_id = a.id \
             WHERE v.content_hash IN ( \
                 SELECT content_hash FROM file_locations \
                 GROUP BY content_hash HAVING COUNT(DISTINCT volume_id) > 1 \
             ) \
             ORDER BY v.file_size DESC",
        )
    }

    /// Find duplicates with optional filters for volume, format, and path prefix.
    pub fn find_duplicates_filtered(
        &self,
        mode: &str,
        volume: Option<&str>,
        format: Option<&str>,
        path_prefix: Option<&str>,
    ) -> Result<Vec<DuplicateEntry>> {
        // Build the inner GROUP BY subquery based on mode
        let inner = match mode {
            "same" => {
                "SELECT content_hash FROM file_locations \
                 GROUP BY content_hash, volume_id HAVING COUNT(*) > 1"
            }
            "cross" => {
                "SELECT content_hash FROM file_locations \
                 GROUP BY content_hash HAVING COUNT(DISTINCT volume_id) > 1"
            }
            _ => {
                "SELECT content_hash FROM file_locations \
                 GROUP BY content_hash HAVING COUNT(*) > 1"
            }
        };

        let mut sql = format!(
            "SELECT v.content_hash, v.original_filename, v.format, v.file_size, a.name, a.id \
             FROM variants v \
             JOIN assets a ON v.asset_id = a.id \
             WHERE v.content_hash IN ({inner})"
        );

        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

        if let Some(vol) = volume {
            sql.push_str(
                " AND v.content_hash IN (SELECT content_hash FROM file_locations WHERE volume_id = ?)",
            );
            params.push(Box::new(vol.to_string()));
        }

        if let Some(prefix) = path_prefix {
            let like = path_pattern_to_like(prefix);
            sql.push_str(
                " AND v.content_hash IN (SELECT content_hash FROM file_locations WHERE relative_path LIKE ? ESCAPE '\\')",
            );
            params.push(Box::new(like));
        }

        if let Some(fmt) = format {
            sql.push_str(" AND LOWER(v.format) = ?");
            params.push(Box::new(fmt.to_lowercase()));
        }

        sql.push_str(" ORDER BY v.file_size DESC");

        self.load_duplicate_entries_filtered(&sql, &params)
    }

    /// Like `load_duplicate_entries` but accepts dynamic params.
    fn load_duplicate_entries_filtered(
        &self,
        variant_query: &str,
        params: &[Box<dyn rusqlite::types::ToSql>],
    ) -> Result<Vec<DuplicateEntry>> {
        let mut stmt = self.conn.prepare(variant_query)?;

        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| p.as_ref()).collect();

        let entries: Vec<DuplicateEntry> = stmt
            .query_map(param_refs.as_slice(), |row| {
                Ok(DuplicateEntry {
                    content_hash: row.get(0)?,
                    original_filename: row.get(1)?,
                    format: row.get(2)?,
                    file_size: row.get(3)?,
                    asset_name: row.get(4)?,
                    asset_id: row.get(5)?,
                    locations: Vec::new(),
                    volume_count: 0,
                    same_volume_groups: Vec::new(),
                    preview_url: String::new(),
                })
            })?
            .collect::<std::result::Result<_, _>>()?;

        let mut lstmt = self.conn.prepare(
            "SELECT fl.relative_path, vol.label, vol.id, vol.purpose, fl.verified_at \
             FROM file_locations fl \
             JOIN volumes vol ON fl.volume_id = vol.id \
             WHERE fl.content_hash = ?1",
        )?;

        let entries: Vec<DuplicateEntry> = entries
            .into_iter()
            .map(|mut e| {
                e.locations = Self::load_locations_for_hash(&mut lstmt, &e.content_hash);
                Self::compute_duplicate_stats(&mut e);
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
            anyhow::bail!("no recipe found with id '{recipe_id}'");
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

    /// Get the verified_at timestamp for a file location or recipe at this volume+path.
    pub fn get_location_verified_at(
        &self,
        volume_id: &str,
        relative_path: &str,
    ) -> Result<Option<String>> {
        // Check file_locations first
        let mut stmt = self.conn.prepare(
            "SELECT verified_at FROM file_locations WHERE volume_id = ?1 AND relative_path = ?2 LIMIT 1",
        )?;
        let mut rows = stmt.query(rusqlite::params![volume_id, relative_path])?;
        if let Some(row) = rows.next()? {
            let verified_at: Option<String> = row.get(0)?;
            if verified_at.is_some() {
                return Ok(verified_at);
            }
        }
        // Fall back to recipes table
        let mut stmt = self.conn.prepare(
            "SELECT verified_at FROM recipes WHERE volume_id = ?1 AND relative_path = ?2 LIMIT 1",
        )?;
        let mut rows = stmt.query(rusqlite::params![volume_id, relative_path])?;
        if let Some(row) = rows.next()? {
            return Ok(row.get(0)?);
        }
        Ok(None)
    }

}
