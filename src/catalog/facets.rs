//! `facets` section of `Catalog` — methods extracted from the original
//! 9.2-kLOC catalog.rs into a multi-file `impl Catalog` block.
//!
//! Types, helper functions, and the connection ctor live in the parent
//! `catalog` module.

use super::*;

impl Catalog {
    // ═══ CALENDAR & FACETS ═══

    /// Get asset counts per day for a given year, respecting search filters.
    ///
    /// Returns a map of `"YYYY-MM-DD"` → count. Reuses `build_search_where()`
    /// for filter consistency, then adds a year constraint and groups by day.
    pub fn calendar_counts(&self, year: i32, opts: &SearchOptions) -> Result<HashMap<String, u64>> {
        let (where_clause, mut params, needs_fl_join, needs_v_join) = Self::build_search_where(opts);

        let mut sql = String::from(
            "SELECT substr(a.created_at, 1, 10) as day, COUNT(DISTINCT a.id) \
             FROM assets a \
             JOIN variants bv ON bv.content_hash = a.best_variant_hash",
        );

        if needs_v_join {
            sql.push_str(" JOIN variants v ON v.asset_id = a.id");
        }
        if needs_fl_join {
            sql.push_str(" JOIN file_locations fl ON v.content_hash = fl.content_hash");
        }

        sql.push_str(&where_clause);

        // Add year constraint
        sql.push_str(" AND a.created_at >= ? AND a.created_at < ?");
        params.push(Box::new(format!("{year:04}-01-01")));
        params.push(Box::new(format!("{:04}-01-01", year + 1)));

        sql.push_str(" GROUP BY day");

        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| p.as_ref()).collect();
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(param_refs.as_slice(), |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?))
        })?;

        let mut counts = HashMap::new();
        for row in rows {
            let (day, count) = row?;
            counts.insert(day, count);
        }
        Ok(counts)
    }

    /// Get all distinct years that have assets.
    pub fn calendar_years(&self) -> Result<Vec<i32>> {
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT CAST(substr(created_at, 1, 4) AS INTEGER) \
             FROM assets \
             WHERE created_at IS NOT NULL \
             ORDER BY 1",
        )?;
        let rows = stmt.query_map([], |row| row.get::<_, i32>(0))?;
        let mut years = Vec::new();
        for row in rows {
            years.push(row?);
        }
        Ok(years)
    }

    /// Get facet counts for the browse sidebar, respecting search filters.
    ///
    /// Runs 8 aggregate queries sharing the same WHERE clause from `build_search_where()`.
    /// Returns counts grouped by rating, label, format, volume, tag, year, and geo.
    pub fn facet_counts(&self, opts: &SearchOptions) -> Result<FacetCounts> {
        let (where_clause, params, needs_fl_join, needs_v_join) = Self::build_search_where(opts);

        // Helper: build the FROM/JOIN prefix used by most queries
        let mut base_from = String::from(
            "FROM assets a \
             JOIN variants bv ON bv.content_hash = a.best_variant_hash",
        );
        if needs_v_join {
            base_from.push_str(" JOIN variants v ON v.asset_id = a.id");
        }
        if needs_fl_join {
            base_from.push_str(" JOIN file_locations fl ON v.content_hash = fl.content_hash");
        }

        // Macro to build param refs from the shared params vec
        macro_rules! prefs {
            ($p:expr) => {
                {
                    let refs: Vec<&dyn rusqlite::types::ToSql> = $p.iter().map(|b| b.as_ref()).collect();
                    refs
                }
            }
        }

        // 1. Total count
        let total: u64 = self.conn.query_row(
            &format!("SELECT COUNT(DISTINCT a.id) {base_from}{where_clause}"),
            prefs!(params).as_slice(),
            |r| r.get(0),
        )?;

        // 2. Rating distribution
        let mut ratings = Vec::new();
        {
            let sql = format!(
                "SELECT a.rating, COUNT(DISTINCT a.id) AS cnt {base_from}{where_clause} GROUP BY a.rating ORDER BY a.rating"
            );
            let mut stmt = self.conn.prepare(&sql)?;
            let rows = stmt.query_map(prefs!(params).as_slice(), |row| {
                Ok((row.get::<_, Option<u8>>(0)?, row.get::<_, u64>(1)?))
            })?;
            for row in rows {
                ratings.push(row?);
            }
        }

        // 3. Label distribution
        let mut labels = Vec::new();
        {
            let sql = format!(
                "SELECT a.color_label, COUNT(DISTINCT a.id) AS cnt {base_from}{where_clause} GROUP BY a.color_label ORDER BY a.color_label"
            );
            let mut stmt = self.conn.prepare(&sql)?;
            let rows = stmt.query_map(prefs!(params).as_slice(), |row| {
                Ok((row.get::<_, Option<String>>(0)?, row.get::<_, u64>(1)?))
            })?;
            for row in rows {
                labels.push(row?);
            }
        }

        // 4. Format distribution (uses denormalized primary_variant_format)
        let mut formats = Vec::new();
        {
            let sql = format!(
                "SELECT COALESCE(a.primary_variant_format, 'unknown') AS fmt, COUNT(DISTINCT a.id) AS cnt \
                 {base_from}{where_clause} GROUP BY fmt ORDER BY cnt DESC LIMIT 30"
            );
            let mut stmt = self.conn.prepare(&sql)?;
            let rows = stmt.query_map(prefs!(params).as_slice(), |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?))
            })?;
            for row in rows {
                formats.push(row?);
            }
        }

        // 5. Volume distribution — always needs fl JOIN for volume_id
        let mut volumes = Vec::new();
        {
            let mut vol_from = String::from(
                "FROM assets a \
                 JOIN variants bv ON bv.content_hash = a.best_variant_hash",
            );
            if needs_v_join {
                vol_from.push_str(" JOIN variants v ON v.asset_id = a.id");
            }
            // Always join file_locations for volume query
            if needs_fl_join {
                vol_from.push_str(" JOIN file_locations fl ON v.content_hash = fl.content_hash");
            } else if needs_v_join {
                vol_from.push_str(" JOIN file_locations fl ON v.content_hash = fl.content_hash");
            } else {
                // Need both v and fl joins
                vol_from.push_str(" JOIN variants v ON v.asset_id = a.id");
                vol_from.push_str(" JOIN file_locations fl ON v.content_hash = fl.content_hash");
            }
            vol_from.push_str(" JOIN volumes vol ON vol.id = fl.volume_id");

            let sql = format!(
                "SELECT fl.volume_id, vol.label, COUNT(DISTINCT a.id) AS cnt \
                 {vol_from}{where_clause} GROUP BY fl.volume_id ORDER BY cnt DESC"
            );
            let mut stmt = self.conn.prepare(&sql)?;
            let rows = stmt.query_map(prefs!(params).as_slice(), |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, u64>(2)?))
            })?;
            for row in rows {
                volumes.push(row?);
            }
        }

        // 6. Tag distribution (JSON expansion of a.tags). Returns every tag
        // present in the matching set, not just a top-N: the facet sidebar
        // renders these as a hierarchy, and an arbitrary cap chops off
        // lower-frequency siblings — with a knock-on effect of leaving
        // descendants without their parent rows (the JS tree-build then
        // synthesises a count=0 parent, which is visually wrong). The data
        // volume is bounded by the user's actual tag vocabulary; the 5000-
        // row cap is generous (real catalogues we've seen mid-restructure
        // were at ~4500 distinct tags total — far more than would appear
        // in any single filtered result set), but cheap: short strings,
        // SQLite GROUP BY on a denormalised JSON expansion.
        let mut tags = Vec::new();
        {
            let sql = format!(
                "SELECT je.value, COUNT(DISTINCT a.id) AS cnt \
                 {base_from}, json_each(a.tags) AS je{where_clause} \
                 GROUP BY je.value ORDER BY cnt DESC LIMIT 5000"
            );
            let mut stmt = self.conn.prepare(&sql)?;
            let rows = stmt.query_map(prefs!(params).as_slice(), |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?))
            })?;
            for row in rows {
                let (tag_name, count) = row?;
                tags.push((tag_name, count));
            }
        }

        // 7. Year distribution
        let mut years = Vec::new();
        {
            let sql = format!(
                "SELECT substr(a.created_at, 1, 4) AS year, COUNT(DISTINCT a.id) AS cnt \
                 {base_from}{where_clause} AND a.created_at IS NOT NULL \
                 GROUP BY year ORDER BY year DESC"
            );
            let mut stmt = self.conn.prepare(&sql)?;
            let rows = stmt.query_map(prefs!(params).as_slice(), |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?))
            })?;
            for row in rows {
                years.push(row?);
            }
        }

        // 8. Geotagged count
        let geotagged: u64 = self.conn.query_row(
            &format!(
                "SELECT COUNT(DISTINCT a.id) {base_from}{where_clause} \
                 AND a.latitude IS NOT NULL"
            ),
            prefs!(params).as_slice(),
            |r| r.get(0),
        )?;

        Ok(FacetCounts {
            total,
            ratings,
            labels,
            formats,
            volumes,
            tags,
            years,
            geotagged,
        })
    }

    /// Backfill GPS latitude/longitude on assets from variant source_metadata.
    /// Called from migrations, idempotent via `WHERE a.latitude IS NULL`.
    pub(super) fn backfill_gps_columns(&self) {
        // Try gps_latitude_decimal first, fall back to parsing DMS strings
        let _ = self.conn.execute_batch(
            "UPDATE assets SET
                latitude = (
                    SELECT COALESCE(
                        CAST(json_extract(v.source_metadata, '$.gps_latitude_decimal') AS REAL),
                        NULL
                    )
                    FROM variants v WHERE v.asset_id = assets.id
                    AND json_extract(v.source_metadata, '$.gps_latitude_decimal') IS NOT NULL
                    ORDER BY CASE v.role WHEN 'original' THEN 0 ELSE 1 END LIMIT 1
                ),
                longitude = (
                    SELECT COALESCE(
                        CAST(json_extract(v.source_metadata, '$.gps_longitude_decimal') AS REAL),
                        NULL
                    )
                    FROM variants v WHERE v.asset_id = assets.id
                    AND json_extract(v.source_metadata, '$.gps_longitude_decimal') IS NOT NULL
                    ORDER BY CASE v.role WHEN 'original' THEN 0 ELSE 1 END LIMIT 1
                )
            WHERE assets.latitude IS NULL
            AND EXISTS (
                SELECT 1 FROM variants v2
                WHERE v2.asset_id = assets.id
                AND json_extract(v2.source_metadata, '$.gps_latitude_decimal') IS NOT NULL
            )"
        );

        // Fallback: parse DMS strings for rows still NULL
        // This needs Rust-side parsing, so we query and update individually
        let rows: Vec<(String, String, String)> = if let Ok(mut stmt) = self.conn.prepare(
            "SELECT a.id, json_extract(v.source_metadata, '$.gps_latitude'),
                    json_extract(v.source_metadata, '$.gps_longitude')
             FROM assets a
             JOIN variants v ON v.asset_id = a.id
             WHERE a.latitude IS NULL
             AND json_extract(v.source_metadata, '$.gps_latitude') IS NOT NULL
             AND json_extract(v.source_metadata, '$.gps_longitude') IS NOT NULL
             GROUP BY a.id"
        ) {
            stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?))
            }).map(|rows| rows.filter_map(|r| r.ok()).collect()).unwrap_or_default()
        } else {
            Vec::new()
        };

        for (id, lat_str, lon_str) in &rows {
            if let (Some(lat), Some(lon)) = (
                crate::exif_reader::parse_dms_string(lat_str),
                crate::exif_reader::parse_dms_string(lon_str),
            ) {
                let _ = self.conn.execute(
                    "UPDATE assets SET latitude = ?1, longitude = ?2 WHERE id = ?3",
                    rusqlite::params![lat, lon, id],
                );
            }
        }
    }

    /// Get map markers (geotagged assets) matching search filters.
    pub fn map_markers(&self, opts: &SearchOptions, limit: u32) -> Result<(Vec<MapMarker>, u64)> {
        let (where_clause, mut params, needs_fl_join, needs_v_join) = Self::build_search_where(opts);

        // Count total geotagged assets matching filters
        let mut count_sql = String::from(
            "SELECT COUNT(DISTINCT a.id) FROM assets a \
             JOIN variants bv ON bv.content_hash = a.best_variant_hash",
        );
        if needs_v_join {
            count_sql.push_str(" JOIN variants v ON v.asset_id = a.id");
        }
        if needs_fl_join {
            count_sql.push_str(" JOIN file_locations fl ON v.content_hash = fl.content_hash");
        }
        count_sql.push_str(&where_clause);
        count_sql.push_str(" AND a.latitude IS NOT NULL AND a.longitude IS NOT NULL");

        let count_param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| p.as_ref()).collect();
        let total: u64 = self.conn.query_row(&count_sql, count_param_refs.as_slice(), |r| r.get(0))?;

        // Fetch markers
        let mut sql = String::from(
            "SELECT DISTINCT a.id, a.latitude, a.longitude, a.best_variant_hash, \
             COALESCE(a.name, bv.original_filename) as display_name, a.rating, a.color_label \
             FROM assets a \
             JOIN variants bv ON bv.content_hash = a.best_variant_hash",
        );
        if needs_v_join {
            sql.push_str(" JOIN variants v ON v.asset_id = a.id");
        }
        if needs_fl_join {
            sql.push_str(" JOIN file_locations fl ON v.content_hash = fl.content_hash");
        }
        sql.push_str(&where_clause);
        sql.push_str(" AND a.latitude IS NOT NULL AND a.longitude IS NOT NULL");
        sql.push_str(" LIMIT ?");
        params.push(Box::new(limit as u64));

        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| p.as_ref()).collect();
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(param_refs.as_slice(), |row| {
            Ok(MapMarker {
                id: row.get(0)?,
                lat: row.get(1)?,
                lng: row.get(2)?,
                preview: row.get(3)?,
                name: row.get(4)?,
                rating: row.get::<_, Option<i64>>(5)?.map(|r| r as u8),
                label: row.get(6)?,
            })
        })?;

        let mut markers = Vec::new();
        for row in rows {
            markers.push(row?);
        }
        Ok((markers, total))
    }

}
