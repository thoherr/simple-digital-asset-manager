//! `search_exec` section of `Catalog` — methods extracted from the original
//! 9.2-kLOC catalog.rs into a multi-file `impl Catalog` block.
//!
//! Types, helper functions, and the connection ctor live in the parent
//! `catalog` module.

use super::*;

impl Catalog {
    // ═══ SEARCH EXECUTION ═══

    /// Paginated search with dynamic filters and sorting.
    /// Uses a separate COUNT query + paginated data query (faster than COUNT(*) OVER()
    /// which forces SQLite to materialize the entire result set).
    pub fn search_paginated_with_count(&self, opts: &SearchOptions) -> Result<(Vec<SearchRow>, u64)> {
        let (where_clause, params, needs_fl_join, needs_v_join) = Self::build_search_where(opts);

        // --- Step 1: Count total matches ---
        let total_count = {
            let count_sql = if needs_v_join {
                let mut sql = String::from(
                    "SELECT COUNT(DISTINCT a.id) FROM assets a \
                     JOIN variants bv ON bv.content_hash = a.best_variant_hash \
                     JOIN variants v ON v.asset_id = a.id",
                );
                if needs_fl_join {
                    sql.push_str(" JOIN file_locations fl ON v.content_hash = fl.content_hash");
                }
                sql.push_str(&where_clause);
                sql
            } else if needs_fl_join {
                let mut sql = String::from(
                    "SELECT COUNT(*) FROM assets a \
                     JOIN variants bv ON bv.content_hash = a.best_variant_hash \
                     JOIN file_locations fl ON bv.content_hash = fl.content_hash",
                );
                sql.push_str(&where_clause);
                sql
            } else {
                // Use same bv JOIN as data query so assets with NULL best_variant_hash
                // are excluded from count (matching the data query behavior)
                let mut sql = String::from(
                    "SELECT COUNT(*) FROM assets a \
                     JOIN variants bv ON bv.content_hash = a.best_variant_hash",
                );
                sql.push_str(&where_clause);
                sql
            };
            let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                params.iter().map(|p| p.as_ref()).collect();
            self.conn.query_row(&count_sql, param_refs.as_slice(), |row| row.get::<_, u64>(0))?
        };

        if total_count == 0 {
            return Ok((Vec::new(), 0));
        }

        // --- Step 2: Fetch one page of results ---
        let (data_params, data_sql) = {
            let mut p = params;
            let page = opts.page.max(1);
            let offset = (page - 1) as u64 * opts.per_page as u64;

            let sql = if needs_v_join {
                let mut inner = String::from(
                    "WITH matched AS (SELECT DISTINCT a.id \
                     FROM assets a \
                     JOIN variants bv ON bv.content_hash = a.best_variant_hash \
                     JOIN variants v ON v.asset_id = a.id",
                );
                if needs_fl_join {
                    inner.push_str(" JOIN file_locations fl ON v.content_hash = fl.content_hash");
                }
                inner.push_str(&where_clause);
                inner.push_str(") SELECT a.id, a.name, a.asset_type, a.created_at, bv.original_filename, bv.format, \
                     a.tags, a.description, bv.content_hash, a.rating, a.color_label, \
                     a.primary_variant_format, a.variant_count, a.stack_id, s.member_count, \
                     a.preview_rotation, a.face_count, a.video_duration \
                     FROM matched m \
                     JOIN assets a ON a.id = m.id \
                     JOIN variants bv ON bv.content_hash = a.best_variant_hash \
                     LEFT JOIN stacks s ON s.id = a.stack_id");
                inner.push_str(&format!(" ORDER BY {}", opts.sort.to_sql()));
                inner.push_str(" LIMIT ? OFFSET ?");
                p.push(Box::new(opts.per_page as u64));
                p.push(Box::new(offset));
                inner
            } else {
                let mut sql = String::from(
                    "SELECT a.id, a.name, a.asset_type, a.created_at, bv.original_filename, bv.format, \
                     a.tags, a.description, bv.content_hash, a.rating, a.color_label, \
                     a.primary_variant_format, a.variant_count, a.stack_id, s.member_count, \
                     a.preview_rotation, a.face_count, a.video_duration \
                     FROM assets a \
                     JOIN variants bv ON bv.content_hash = a.best_variant_hash \
                     LEFT JOIN stacks s ON s.id = a.stack_id",
                );
                sql.push_str(&where_clause);
                sql.push_str(&format!(" ORDER BY {}", opts.sort.to_sql()));
                sql.push_str(" LIMIT ? OFFSET ?");
                p.push(Box::new(opts.per_page as u64));
                p.push(Box::new(offset));
                sql
            };
            (p, sql)
        };

        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            data_params.iter().map(|p| p.as_ref()).collect();
        let mut stmt = self.conn.prepare(&data_sql)?;
        let rows = stmt.query_map(param_refs.as_slice(), |row| {
            let tags_json: String = row.get(6)?;
            let tags: Vec<String> = serde_json::from_str(&tags_json).unwrap_or_default();
            let rating_val: Option<i64> = row.get(9)?;
            let variant_count_val: i64 = row.get(12)?;
            let stack_member_count: Option<i64> = row.get(14)?;
            let rotation_val: Option<i64> = row.get(15)?;
            let face_count_val: i64 = row.get::<_, Option<i64>>(16)?.unwrap_or(0);
            let video_duration: Option<f64> = row.get(17)?;
            Ok(SearchRow {
                asset_id: row.get(0)?,
                name: row.get(1)?,
                asset_type: row.get(2)?,
                created_at: row.get(3)?,
                original_filename: row.get(4)?,
                format: row.get(5)?,
                tags,
                description: row.get(7)?,
                content_hash: row.get(8)?,
                rating: rating_val.map(|r| r as u8),
                color_label: row.get(10)?,
                primary_format: row.get(11)?,
                variant_count: variant_count_val as u32,
                stack_id: row.get(13)?,
                stack_count: stack_member_count.map(|n| n as u32),
                preview_rotation: rotation_val.map(|r| r as u16),
                face_count: face_count_val as u32,
                video_duration,
            })
        })?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok((results, total_count))
    }

    /// Paginated search with dynamic filters and sorting.
    pub fn search_paginated(&self, opts: &SearchOptions) -> Result<Vec<SearchRow>> {
        let (rows, _total) = self.search_paginated_with_count(opts)?;
        Ok(rows)
    }

    /// Fetch a single asset as a SearchRow by asset ID.
    pub fn get_search_row(&self, asset_id: &str) -> Result<Option<SearchRow>> {
        let sql = "SELECT a.id, a.name, a.asset_type, a.created_at, bv.original_filename, bv.format, \
                   a.tags, a.description, bv.content_hash, a.rating, a.color_label, \
                   a.primary_variant_format, a.variant_count, a.stack_id, s.member_count, \
                   a.preview_rotation, a.face_count, a.video_duration \
                   FROM assets a \
                   JOIN variants bv ON bv.content_hash = a.best_variant_hash \
                   LEFT JOIN stacks s ON s.id = a.stack_id \
                   WHERE a.id = ?1";
        let result = self.conn.query_row(sql, rusqlite::params![asset_id], |row| {
            let tags_json: String = row.get(6)?;
            let tags: Vec<String> = serde_json::from_str(&tags_json).unwrap_or_default();
            let rating_val: Option<i64> = row.get(9)?;
            let variant_count_val: i64 = row.get(12)?;
            let stack_member_count: Option<i64> = row.get(14)?;
            let rotation_val: Option<i64> = row.get(15)?;
            let face_count_val: i64 = row.get::<_, Option<i64>>(16)?.unwrap_or(0);
            let video_duration: Option<f64> = row.get(17)?;
            Ok(SearchRow {
                asset_id: row.get(0)?,
                name: row.get(1)?,
                asset_type: row.get(2)?,
                created_at: row.get(3)?,
                original_filename: row.get(4)?,
                format: row.get(5)?,
                tags,
                description: row.get(7)?,
                content_hash: row.get(8)?,
                rating: rating_val.map(|r| r as u8),
                color_label: row.get(10)?,
                primary_format: row.get(11)?,
                variant_count: variant_count_val as u32,
                stack_id: row.get(13)?,
                stack_count: stack_member_count.map(|n| n as u32),
                preview_rotation: rotation_val.map(|r| r as u16),
                face_count: face_count_val as u32,
                video_duration,
            })
        });
        match result {
            Ok(row) => Ok(Some(row)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Count total results matching the same filters as search_paginated (without LIMIT/OFFSET).
    pub fn search_count(&self, opts: &SearchOptions) -> Result<u64> {
        let (where_clause, params, needs_fl_join, needs_v_join) = Self::build_search_where(opts);

        let count_expr = if needs_v_join { "COUNT(DISTINCT a.id)" } else { "COUNT(*)" };
        let mut sql = format!(
            "SELECT {} FROM assets a \
             JOIN variants bv ON bv.content_hash = a.best_variant_hash",
            count_expr
        );

        if needs_v_join {
            sql.push_str(" JOIN variants v ON v.asset_id = a.id");
        }
        if needs_fl_join {
            sql.push_str(" JOIN file_locations fl ON v.content_hash = fl.content_hash");
        }

        sql.push_str(&where_clause);

        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| p.as_ref()).collect();
        let count: u64 = self.conn.query_row(&sql, param_refs.as_slice(), |r| r.get(0))?;
        Ok(count)
    }

}
