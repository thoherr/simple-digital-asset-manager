//! `recipe_crud` section of `Catalog` — methods extracted from the original
//! 9.2-kLOC catalog.rs into a multi-file `impl Catalog` block.
//!
//! Types, helper functions, and the connection ctor live in the parent
//! `catalog` module.

use super::*;

impl Catalog {
    // ═══ RECIPE CRUD ═══

    /// Insert a recipe into the catalog.
    ///
    /// `pending_writeback` is included in the column list. The schema
    /// declares it `NOT NULL DEFAULT 0`, and an `INSERT OR REPLACE` that
    /// omits the column would silently reset an existing row's flag back
    /// to 0 on every catalog rebuild / reimport — causing YAML (source
    /// of truth, which carried `pending_writeback: true`) to diverge
    /// from SQLite (clobbered to 0). With the flag in the column list,
    /// state survives every catalog write.
    pub fn insert_recipe(&self, recipe: &Recipe) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO recipes (id, variant_hash, software, recipe_type, content_hash, volume_id, relative_path, pending_writeback) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                recipe.id.to_string(),
                recipe.variant_hash,
                recipe.software,
                format!("{:?}", recipe.recipe_type).to_lowercase(),
                recipe.content_hash,
                recipe.location.volume_id.to_string(),
                recipe.location.relative_path_str(),
                if recipe.pending_writeback { 1 } else { 0 },
            ],
        )?;
        Ok(())
    }

}
