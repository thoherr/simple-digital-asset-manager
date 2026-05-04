//! `recipe_crud` section of `Catalog` — methods extracted from the original
//! 9.2-kLOC catalog.rs into a multi-file `impl Catalog` block.
//!
//! Types, helper functions, and the connection ctor live in the parent
//! `catalog` module.

use super::*;

impl Catalog {
    // ═══ RECIPE CRUD ═══

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
                recipe.location.relative_path_str(),
            ],
        )?;
        Ok(())
    }

}
