//! `recipe_query` section of `Catalog` — methods extracted from the original
//! 9.2-kLOC catalog.rs into a multi-file `impl Catalog` block.
//!
//! Types, helper functions, and the connection ctor live in the parent
//! `catalog` module.

use super::*;

impl Catalog {
    // ═══ LOCATION & RECIPE QUERIES ═══

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
            anyhow::bail!("no recipe found with id '{recipe_id}'");
        }
        Ok(())
    }

    /// Mark a recipe as needing XMP write-back (e.g. volume was offline during edit).
    pub fn mark_pending_writeback(&self, recipe_id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE recipes SET pending_writeback = 1 WHERE id = ?1",
            rusqlite::params![recipe_id],
        )?;
        Ok(())
    }

    /// Clear the pending write-back flag (after successful XMP write).
    pub fn clear_pending_writeback(&self, recipe_id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE recipes SET pending_writeback = 0 WHERE id = ?1",
            rusqlite::params![recipe_id],
        )?;
        Ok(())
    }

    /// List recipes with pending write-back, optionally filtered by volume.
    /// Returns `(recipe_id, asset_id, volume_id, relative_path)`.
    pub fn list_pending_writeback_recipes(
        &self,
        volume_id: Option<&str>,
    ) -> Result<Vec<(String, String, String, String)>> {
        let (sql, params): (String, Vec<Box<dyn rusqlite::types::ToSql>>) = if let Some(vid) = volume_id {
            (
                "SELECT r.id, v.asset_id, r.volume_id, r.relative_path \
                 FROM recipes r \
                 JOIN variants v ON r.variant_hash = v.content_hash \
                 WHERE r.pending_writeback = 1 AND r.volume_id = ?1"
                    .to_string(),
                vec![Box::new(vid.to_string())],
            )
        } else {
            (
                "SELECT r.id, v.asset_id, r.volume_id, r.relative_path \
                 FROM recipes r \
                 JOIN variants v ON r.variant_hash = v.content_hash \
                 WHERE r.pending_writeback = 1"
                    .to_string(),
                vec![],
            )
        };
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(params.iter()), |row| {
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
    /// Optionally excludes a specific asset ID from results (used by fix-recipes
    /// to avoid self-matching the standalone recipe asset).
    pub fn find_variant_hash_by_stem_and_directory(
        &self,
        stem: &str,
        directory_prefix: &str,
        volume_id: &str,
        exclude_asset_id: Option<&str>,
    ) -> Result<Option<(String, String)>> {
        // Match file_locations where: same volume, path starts with directory_prefix,
        // and the filename (without extension) matches the stem.
        let path_pattern = if directory_prefix.is_empty() {
            format!("{stem}.%")
        } else {
            format!("{directory_prefix}/{stem}.%")
        };
        let (sql, params): (String, Vec<Box<dyn rusqlite::types::ToSql>>) = if let Some(exclude) = exclude_asset_id {
            (
                "SELECT fl.content_hash, v.asset_id FROM file_locations fl \
                 JOIN variants v ON fl.content_hash = v.content_hash \
                 WHERE fl.volume_id = ?1 AND fl.relative_path LIKE ?2 AND v.asset_id != ?3 \
                 LIMIT 1".to_string(),
                vec![
                    Box::new(volume_id.to_string()),
                    Box::new(path_pattern),
                    Box::new(exclude.to_string()),
                ],
            )
        } else {
            (
                "SELECT fl.content_hash, v.asset_id FROM file_locations fl \
                 JOIN variants v ON fl.content_hash = v.content_hash \
                 WHERE fl.volume_id = ?1 AND fl.relative_path LIKE ?2 \
                 LIMIT 1".to_string(),
                vec![
                    Box::new(volume_id.to_string()),
                    Box::new(path_pattern),
                ],
            )
        };
        let mut stmt = self.conn.prepare(&sql)?;
        let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
        let mut rows = stmt.query(param_refs.as_slice())?;
        match rows.next()? {
            Some(row) => Ok(Some((row.get(0)?, row.get(1)?))),
            None => Ok(None),
        }
    }

    /// List assets that have exactly one variant whose format is a recipe extension
    /// (xmp, cos, cot, cop, pp3, dop, on1) and asset_type = 'other'.
    /// Returns `(asset_id, content_hash, format)` for each match.
    /// Optionally scoped by volume or asset ID.
    pub fn list_recipe_only_assets(
        &self,
        volume_id: Option<&str>,
        asset_id: Option<&str>,
    ) -> Result<Vec<(String, String, String)>> {
        let recipe_extensions: &[&str] = &["xmp", "cos", "cot", "cop", "pp3", "dop", "on1"];

        let mut sql = String::from(
            "SELECT a.id, v.content_hash, v.format \
             FROM assets a \
             JOIN variants v ON v.asset_id = a.id \
             WHERE a.asset_type = 'other'",
        );
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        let mut param_idx = 1;

        if let Some(aid) = asset_id {
            sql.push_str(&format!(" AND a.id = ?{param_idx}"));
            params.push(Box::new(aid.to_string()));
            param_idx += 1;
        }

        if let Some(vid) = volume_id {
            sql.push_str(&format!(
                " AND v.content_hash IN (SELECT content_hash FROM file_locations WHERE volume_id = ?{param_idx})"
            ));
            params.push(Box::new(vid.to_string()));
            param_idx += 1;
        }
        let _ = param_idx;

        sql.push_str(" GROUP BY a.id HAVING COUNT(v.content_hash) = 1");

        let mut stmt = self.conn.prepare(&sql)?;
        let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
        let rows = stmt.query_map(param_refs.as_slice(), |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;

        let mut results = Vec::new();
        for row in rows {
            let (aid, hash, fmt) = row?;
            if recipe_extensions.contains(&fmt.to_lowercase().as_str()) {
                results.push((aid, hash, fmt));
            }
        }
        Ok(results)
    }

    /// Find all asset IDs that have file locations on a given volume under any of
    /// the given path prefixes. Used by `import --auto-group` to scope auto-grouping
    /// to the "neighborhood" of imported files.
    pub fn find_asset_ids_by_volume_and_path_prefixes(
        &self,
        volume_id: &str,
        prefixes: &[String],
    ) -> Result<Vec<String>> {
        if prefixes.is_empty() {
            return Ok(Vec::new());
        }

        // Build dynamic OR clause: one `fl.relative_path LIKE ?N` per prefix
        let mut conditions = Vec::new();
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        params.push(Box::new(volume_id.to_string()));

        for (i, prefix) in prefixes.iter().enumerate() {
            conditions.push(format!("fl.relative_path LIKE ?{}", i + 2));
            if prefix.is_empty() {
                params.push(Box::new("%".to_string()));
            } else {
                params.push(Box::new(format!("{prefix}/%")));
            }
        }

        let sql = format!(
            "SELECT DISTINCT v.asset_id FROM variants v \
             JOIN file_locations fl ON v.content_hash = fl.content_hash \
             WHERE fl.volume_id = ?1 AND ({})",
            conditions.join(" OR ")
        );

        let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(param_refs.as_slice(), |row| row.get::<_, String>(0))?;

        let mut ids = Vec::new();
        for row in rows {
            ids.push(row?);
        }
        Ok(ids)
    }

}
