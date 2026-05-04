//! `rebuild` section of `Catalog` — methods extracted from the original
//! 9.2-kLOC catalog.rs into a multi-file `impl Catalog` block.
//!
//! Types, helper functions, and the connection ctor live in the parent
//! `catalog` module.

use super::*;

impl Catalog {
    // ═══ REBUILD ═══

    /// Drop and recreate data tables (assets, variants, file_locations, recipes).
    /// Keeps the volumes table intact. Ensures the schema is up to date.
    pub fn rebuild(&self) -> Result<()> {
        self.conn.execute_batch(
            "DROP TABLE IF EXISTS schema_version;
             DROP TABLE IF EXISTS faces;
             DROP TABLE IF EXISTS people;
             DROP TABLE IF EXISTS embeddings;
             DROP TABLE IF EXISTS collection_assets;
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

    /// Like `list_recipes_for_volume_under_prefix` but also returns `pending_writeback`.
    /// Returns `(id, content_hash, variant_hash, relative_path, pending_writeback)`.
    pub fn list_recipes_with_pending_for_volume(
        &self,
        volume_id: &str,
    ) -> Result<Vec<(String, String, String, String, bool)>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, content_hash, variant_hash, relative_path, pending_writeback \
             FROM recipes WHERE volume_id = ?1",
        )?;
        let rows = stmt.query_map(rusqlite::params![volume_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, bool>(4)?,
            ))
        })?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }

    /// List recipes for a specific variant on a specific volume.
    /// Returns `(recipe_id, content_hash, relative_path)` tuples.
    pub fn list_recipes_for_variant_on_volume(
        &self,
        variant_hash: &str,
        volume_id: &str,
    ) -> Result<Vec<(String, String, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, content_hash, relative_path FROM recipes \
             WHERE variant_hash = ?1 AND volume_id = ?2",
        )?;
        let rows = stmt.query_map(rusqlite::params![variant_hash, volume_id], |row| {
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

    /// Find all asset IDs that share the same directory (session) as the given asset.
    /// Goes up one directory level from each of the asset's file locations to find
    /// "session roots", then returns all asset IDs with files under those roots.
    pub fn find_same_session_asset_ids(&self, asset_id: &str) -> Result<std::collections::HashSet<String>> {
        let locations = self.list_file_locations_for_asset(asset_id)?;
        let mut session_ids = std::collections::HashSet::new();

        for (_hash, rel_path, volume_id) in &locations {
            // Get the parent directory of this file
            let parent = std::path::Path::new(rel_path)
                .parent()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default();
            if parent.is_empty() {
                continue;
            }
            // Go up one more level to get the "session root" (e.g., Capture/2026-02-22/)
            let session_root = std::path::Path::new(&parent)
                .parent()
                .map(|p| {
                    let s = p.to_string_lossy().to_string();
                    if s.is_empty() { parent.clone() } else { s }
                })
                .unwrap_or_else(|| parent.clone());
            let prefix = format!("{}%", session_root);

            let mut stmt = self.conn.prepare(
                "SELECT DISTINCT v.asset_id FROM variants v \
                 JOIN file_locations fl ON fl.content_hash = v.content_hash \
                 WHERE fl.volume_id = ?1 AND fl.relative_path LIKE ?2",
            )?;
            let rows = stmt.query_map(rusqlite::params![volume_id, prefix], |row| {
                row.get::<_, String>(0)
            })?;
            for row in rows {
                if let Ok(id) = row {
                    session_ids.insert(id);
                }
            }
        }
        Ok(session_ids)
    }

    /// Update the relative_path for a file location (variant moved on disk).
    pub fn update_file_location_path(
        &self,
        content_hash: &str,
        volume_id: &str,
        old_path: &str,
        new_path: &str,
    ) -> Result<()> {
        let new_path_norm = new_path.replace('\\', "/");
        let old_path_norm = old_path.replace('\\', "/");
        let changed = self.conn.execute(
            "UPDATE file_locations SET relative_path = ?1 \
             WHERE content_hash = ?2 AND volume_id = ?3 AND relative_path = ?4",
            rusqlite::params![new_path_norm, content_hash, volume_id, old_path_norm],
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
        let new_path_norm = new_path.replace('\\', "/");
        let changed = self.conn.execute(
            "UPDATE recipes SET relative_path = ?1 WHERE id = ?2",
            rusqlite::params![new_path_norm, recipe_id],
        )?;
        if changed == 0 {
            anyhow::bail!("no recipe found with id '{recipe_id}'");
        }
        Ok(())
    }

}
