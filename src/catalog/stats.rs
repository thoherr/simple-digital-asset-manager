//! `stats` section of `Catalog` — methods extracted from the original
//! 9.2-kLOC catalog.rs into a multi-file `impl Catalog` block.
//!
//! Types, helper functions, and the connection ctor live in the parent
//! `catalog` module.

use super::*;

impl Catalog {
    // ═══ STATS ═══

    /// Core overview counts: (assets, variants, recipes, total_size).
    pub fn stats_overview(&self) -> Result<(u64, u64, u64, u64, u64)> {
        self.conn.query_row(
            "SELECT \
                (SELECT COUNT(*) FROM assets), \
                (SELECT COUNT(*) FROM variants), \
                (SELECT COUNT(*) FROM recipes), \
                (SELECT COALESCE(SUM(file_size), 0) FROM variants), \
                (SELECT COUNT(*) FROM file_locations)",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        ).map_err(Into::into)
    }

    /// Recipe counts: (total_recipe_rows, unique_content_hashes).
    /// The difference is the number of duplicate recipe locations (e.g. backups).
    pub fn stats_recipe_counts(&self) -> Result<(u64, u64)> {
        self.conn.query_row(
            "SELECT \
                (SELECT COUNT(*) FROM recipes), \
                (SELECT COUNT(DISTINCT content_hash) FROM recipes)",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        ).map_err(Into::into)
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

    /// Recipe format counts: extract file extension in SQL and aggregate.
    pub fn stats_recipe_formats(&self, limit: usize) -> Result<Vec<(String, u64)>> {
        let mut stmt = self.conn.prepare(
            "SELECT LOWER(REPLACE(relative_path, \
                RTRIM(relative_path, REPLACE(relative_path, '.', '')), '')) as ext, \
             COUNT(*) as cnt \
             FROM recipes WHERE relative_path IS NOT NULL \
             GROUP BY ext ORDER BY cnt DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(rusqlite::params![limit as u64], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, u64>(1)?))
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>().map_err(Into::into)
    }

}
