//! `analytics` section of `Catalog` — methods extracted from the original
//! 9.2-kLOC catalog.rs into a multi-file `impl Catalog` block.
//!
//! Types, helper functions, and the connection ctor live in the parent
//! `catalog` module.

use super::*;

impl Catalog {
    // ═══ ANALYTICS ═══

    /// Build analytics data for the dashboard page.
    pub fn build_analytics(&self, limit: usize) -> Result<AnalyticsData> {
        // Camera usage (top N)
        let mut stmt = self.conn.prepare(
            "SELECT camera_model, COUNT(*) as cnt FROM variants
             WHERE camera_model IS NOT NULL AND camera_model != ''
             GROUP BY camera_model ORDER BY cnt DESC LIMIT ?1"
        )?;
        let camera_usage: Vec<NameCount> = stmt.query_map([limit as i64], |row| {
            Ok(NameCount {
                name: row.get(0)?,
                count: row.get::<_, i64>(1)? as u64,
            })
        })?.filter_map(|r| r.ok()).collect();

        // Lens usage (top N)
        let mut stmt = self.conn.prepare(
            "SELECT lens_model, COUNT(*) as cnt FROM variants
             WHERE lens_model IS NOT NULL AND lens_model != ''
             GROUP BY lens_model ORDER BY cnt DESC LIMIT ?1"
        )?;
        let lens_usage: Vec<NameCount> = stmt.query_map([limit as i64], |row| {
            Ok(NameCount {
                name: row.get(0)?,
                count: row.get::<_, i64>(1)? as u64,
            })
        })?.filter_map(|r| r.ok()).collect();

        // Rating distribution (0=unrated, 1-5)
        let mut rating_distribution = Vec::new();
        let unrated: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM assets WHERE rating IS NULL", [], |r| r.get(0)
        )?;
        rating_distribution.push(RatingCount { rating: 0, count: unrated as u64 });
        for r in 1..=5u8 {
            let cnt: i64 = self.conn.query_row(
                "SELECT COUNT(*) FROM assets WHERE rating = ?1", [r], |row| row.get(0)
            )?;
            rating_distribution.push(RatingCount { rating: r, count: cnt as u64 });
        }

        // Format distribution (top N by primary_variant_format)
        let mut stmt = self.conn.prepare(
            "SELECT primary_variant_format, COUNT(*) as cnt FROM assets
             WHERE primary_variant_format IS NOT NULL AND primary_variant_format != ''
             GROUP BY primary_variant_format ORDER BY cnt DESC LIMIT ?1"
        )?;
        let format_distribution: Vec<NameCount> = stmt.query_map([limit as i64], |row| {
            Ok(NameCount {
                name: row.get(0)?,
                count: row.get::<_, i64>(1)? as u64,
            })
        })?.filter_map(|r| r.ok()).collect();

        // Monthly imports (last 24 months, by created_at)
        let mut stmt = self.conn.prepare(
            "SELECT strftime('%Y-%m', created_at) as month, COUNT(*) as cnt FROM assets
             WHERE created_at IS NOT NULL
             GROUP BY month ORDER BY month DESC LIMIT 24"
        )?;
        let mut monthly_imports: Vec<MonthCount> = stmt.query_map([], |row| {
            Ok(MonthCount {
                month: row.get(0)?,
                count: row.get::<_, i64>(1)? as u64,
            })
        })?.filter_map(|r| r.ok()).collect();
        monthly_imports.reverse(); // chronological order

        // Storage by volume
        let mut stmt = self.conn.prepare(
            "SELECT v.label, COALESCE(SUM(var.file_size), 0) as total_size
             FROM volumes v
             JOIN file_locations fl ON fl.volume_id = v.id
             JOIN variants var ON var.content_hash = fl.content_hash
             GROUP BY v.id ORDER BY total_size DESC"
        )?;
        let storage_by_volume: Vec<VolumeSize> = stmt.query_map([], |row| {
            Ok(VolumeSize {
                label: row.get(0)?,
                size: row.get::<_, i64>(1)? as u64,
            })
        })?.filter_map(|r| r.ok()).collect();

        // Yearly asset counts (by created_at)
        let mut stmt = self.conn.prepare(
            "SELECT strftime('%Y', created_at) as year, COUNT(*) as cnt FROM assets
             WHERE created_at IS NOT NULL
             GROUP BY year ORDER BY year"
        )?;
        let yearly_counts: Vec<YearCount> = stmt.query_map([], |row| {
            Ok(YearCount {
                year: row.get(0)?,
                count: row.get::<_, i64>(1)? as u64,
            })
        })?.filter_map(|r| r.ok()).collect();

        Ok(AnalyticsData {
            camera_usage,
            lens_usage,
            rating_distribution,
            format_distribution,
            monthly_imports,
            storage_by_volume,
            yearly_counts,
        })
    }

}
