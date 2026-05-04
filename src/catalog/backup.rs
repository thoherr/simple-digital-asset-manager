//! `backup` section of `Catalog` — methods extracted from the original
//! 9.2-kLOC catalog.rs into a multi-file `impl Catalog` block.
//!
//! Types, helper functions, and the connection ctor live in the parent
//! `catalog` module.

use super::*;

impl Catalog {
    // ═══ BACKUP STATUS ═══

    /// Build a backup-status overview for the given scope of assets.
    ///
    /// - `scope_ids`: `None` = all assets, `Some(ids)` = specific assets
    /// - `volumes_info`: `(label, volume_id, is_online, purpose)` from DeviceRegistry
    /// - `min_copies`: threshold for "at risk"
    /// - `target_volume_id`: optional volume to compute `VolumeGapDetail` for
    pub fn backup_status_overview(
        &self,
        scope_ids: Option<&[String]>,
        volumes_info: &[(String, String, bool, Option<String>)],
        min_copies: u64,
        target_volume_id: Option<&str>,
    ) -> Result<BackupStatusResult> {
        let scoped = scope_ids.is_some();

        // Create temp table for scoped queries
        if let Some(ids) = scope_ids {
            self.conn.execute_batch("CREATE TEMP TABLE IF NOT EXISTS _bs_scope (asset_id TEXT PRIMARY KEY)")?;
            self.conn.execute_batch("DELETE FROM _bs_scope")?;

            // Batch insert in chunks of 500
            for chunk in ids.chunks(500) {
                let placeholders: Vec<&str> = chunk.iter().map(|_| "(?)").collect();
                let sql = format!("INSERT OR IGNORE INTO _bs_scope (asset_id) VALUES {}", placeholders.join(","));
                let params: Vec<&dyn rusqlite::types::ToSql> = chunk.iter().map(|id| id as &dyn rusqlite::types::ToSql).collect();
                self.conn.execute(&sql, params.as_slice())?;
            }
        }

        let scope_filter = if scoped {
            "JOIN _bs_scope bs ON bs.asset_id = a.id"
        } else {
            ""
        };
        let scope_filter_v = if scoped {
            "JOIN _bs_scope bs ON bs.asset_id = v.asset_id"
        } else {
            ""
        };

        // Combined counts: total assets, variants, file_locations in one query
        let (total_assets, total_variants, total_file_locations): (u64, u64, u64) = self.conn.query_row(
            &format!(
                "SELECT \
                    (SELECT COUNT(*) FROM assets a {}), \
                    (SELECT COUNT(*) FROM variants v {}), \
                    (SELECT COUNT(*) FROM file_locations fl \
                        JOIN variants v ON fl.content_hash = v.content_hash {})",
                scope_filter, scope_filter_v, scope_filter_v
            ),
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;

        // Volume distribution + at-risk count in a single pass
        // Computes per-asset volume count, then buckets and counts at-risk
        let mut stmt = self.conn.prepare(&format!(
            "SELECT vol_count, COUNT(*) FROM ( \
                SELECT a.id, COUNT(DISTINCT fl.volume_id) as vol_count \
                FROM assets a {} \
                LEFT JOIN variants v2 ON v2.asset_id = a.id \
                LEFT JOIN file_locations fl ON fl.content_hash = v2.content_hash \
                GROUP BY a.id \
            ) GROUP BY vol_count ORDER BY vol_count",
            scope_filter,
        ))?;
        let mut buckets = [0u64; 4]; // [0, 1, 2, 3+]
        let mut at_risk_count = 0u64;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, u64>(0)?, r.get::<_, u64>(1)?)))?;
        for row in rows {
            let (vol_count, asset_count) = row?;
            match vol_count {
                0 => buckets[0] += asset_count,
                1 => buckets[1] += asset_count,
                2 => buckets[2] += asset_count,
                _ => buckets[3] += asset_count,
            }
            if vol_count < min_copies {
                at_risk_count += asset_count;
            }
        }
        let location_distribution = vec![
            LocationBucket { volume_count: "0".to_string(), asset_count: buckets[0] },
            LocationBucket { volume_count: "1".to_string(), asset_count: buckets[1] },
            LocationBucket { volume_count: "2".to_string(), asset_count: buckets[2] },
            LocationBucket { volume_count: "3+".to_string(), asset_count: buckets[3] },
        ];

        // Purpose coverage + volume gaps in batch queries (one each instead of per-item)
        let mut purpose_groups: HashMap<String, Vec<(String, String, Option<String>)>> = HashMap::new();
        for (label, vid, _online, purpose) in volumes_info {
            let purpose_str = purpose.as_deref().unwrap_or("(none)");
            purpose_groups.entry(purpose_str.to_string()).or_default()
                .push((vid.clone(), label.clone(), purpose.clone()));
        }

        // Single query: asset count per volume (reused for both purpose coverage and volume gaps)
        let mut assets_per_volume: HashMap<String, u64> = HashMap::new();
        {
            let mut stmt = self.conn.prepare(&format!(
                "SELECT fl.volume_id, COUNT(DISTINCT v.asset_id) \
                 FROM file_locations fl \
                 JOIN variants v ON fl.content_hash = v.content_hash \
                 {} \
                 GROUP BY fl.volume_id",
                scope_filter_v,
            ))?;
            let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, u64>(1)?)))?;
            for row in rows {
                let (vid, count) = row?;
                assets_per_volume.insert(vid, count);
            }
        }

        // Build purpose coverage from the per-volume counts
        let mut purpose_coverage = Vec::new();
        for (purpose_str, vol_entries) in &purpose_groups {
            // For purpose coverage we need assets on ANY volume of this purpose (distinct)
            let vol_ids: Vec<&str> = vol_entries.iter().map(|(vid, _, _)| vid.as_str()).collect();
            if vol_ids.is_empty() { continue; }
            let placeholders: Vec<String> = vol_ids.iter().enumerate().map(|(i, _)| format!("?{}", i + 1)).collect();
            let sql = format!(
                "SELECT COUNT(DISTINCT v.asset_id) FROM file_locations fl \
                 JOIN variants v ON fl.content_hash = v.content_hash \
                 {} \
                 WHERE fl.volume_id IN ({})",
                scope_filter_v,
                placeholders.join(","),
            );
            let params: Vec<&dyn rusqlite::types::ToSql> = vol_ids.iter().map(|id| id as &dyn rusqlite::types::ToSql).collect();
            let asset_count: u64 = self.conn.query_row(&sql, params.as_slice(), |r| r.get(0))?;
            let pct = if total_assets > 0 { (asset_count as f64 / total_assets as f64) * 100.0 } else { 0.0 };
            purpose_coverage.push(PurposeCoverage {
                purpose: purpose_str.clone(),
                volume_count: vol_entries.len() as u64,
                asset_count,
                asset_percentage: pct,
            });
        }
        purpose_coverage.sort_by(|a, b| b.asset_count.cmp(&a.asset_count));

        // Build volume gaps from the per-volume counts (no extra queries)
        let mut volume_gaps = Vec::new();
        for (label, vid, _online, purpose) in volumes_info {
            let present = *assets_per_volume.get(vid).unwrap_or(&0);
            let missing = total_assets.saturating_sub(present);
            if missing > 0 {
                volume_gaps.push(VolumeGap {
                    volume_label: label.clone(),
                    volume_id: vid.clone(),
                    purpose: purpose.clone(),
                    missing_count: missing,
                });
            }
        }
        volume_gaps.sort_by(|a, b| a.missing_count.cmp(&b.missing_count));

        // Volume detail: for --volume target
        let volume_detail = if let Some(target_vid) = target_volume_id {
            let present: u64 = self.conn.query_row(
                &format!(
                    "SELECT COUNT(DISTINCT v.asset_id) FROM file_locations fl \
                     JOIN variants v ON fl.content_hash = v.content_hash \
                     {} \
                     WHERE fl.volume_id = ?1",
                    scope_filter_v,
                ),
                [target_vid],
                |r| r.get(0),
            )?;
            let missing = total_assets.saturating_sub(present);
            let pct = if total_assets > 0 { (present as f64 / total_assets as f64) * 100.0 } else { 0.0 };
            let vol_info = volumes_info.iter().find(|v| v.1 == target_vid);
            Some(VolumeGapDetail {
                volume_label: vol_info.map(|v| v.0.clone()).unwrap_or_default(),
                volume_id: target_vid.to_string(),
                purpose: vol_info.and_then(|v| v.3.clone()),
                present_count: present,
                missing_count: missing,
                total_scoped: total_assets,
                coverage_pct: pct,
            })
        } else {
            None
        };

        let scope = if scoped { "filtered" } else { "all assets" }.to_string();

        // Cleanup temp table
        if scoped {
            let _ = self.conn.execute_batch("DROP TABLE IF EXISTS _bs_scope");
        }

        Ok(BackupStatusResult {
            scope,
            total_assets,
            total_variants,
            total_file_locations,
            min_copies,
            at_risk_count,
            purpose_coverage,
            location_distribution,
            volume_gaps,
            volume_detail,
        })
    }

    /// Return asset IDs on fewer than `min_copies` distinct volumes.
    pub fn backup_status_at_risk_ids(
        &self,
        scope_ids: Option<&[String]>,
        min_copies: u64,
    ) -> Result<Vec<String>> {
        let scoped = scope_ids.is_some();

        if let Some(ids) = scope_ids {
            self.conn.execute_batch("CREATE TEMP TABLE IF NOT EXISTS _bs_scope (asset_id TEXT PRIMARY KEY)")?;
            self.conn.execute_batch("DELETE FROM _bs_scope")?;
            for chunk in ids.chunks(500) {
                let placeholders: Vec<&str> = chunk.iter().map(|_| "(?)").collect();
                let sql = format!("INSERT OR IGNORE INTO _bs_scope (asset_id) VALUES {}", placeholders.join(","));
                let params: Vec<&dyn rusqlite::types::ToSql> = chunk.iter().map(|id| id as &dyn rusqlite::types::ToSql).collect();
                self.conn.execute(&sql, params.as_slice())?;
            }
        }

        let scope_filter = if scoped {
            "JOIN _bs_scope bs ON bs.asset_id = a.id"
        } else {
            ""
        };

        let mut stmt = self.conn.prepare(&format!(
            "SELECT a.id FROM assets a {} \
             LEFT JOIN variants v ON v.asset_id = a.id \
             LEFT JOIN file_locations fl ON fl.content_hash = v.content_hash \
             GROUP BY a.id HAVING COUNT(DISTINCT fl.volume_id) < ?1",
            scope_filter,
        ))?;
        let rows = stmt.query_map([min_copies], |r| r.get::<_, String>(0))?;
        let mut ids = Vec::new();
        for row in rows {
            ids.push(row?);
        }

        if scoped {
            let _ = self.conn.execute_batch("DROP TABLE IF EXISTS _bs_scope");
        }

        Ok(ids)
    }

    /// Return asset IDs that have no file_location on the given volume.
    pub fn backup_status_missing_from_volume(
        &self,
        scope_ids: Option<&[String]>,
        volume_id: &str,
    ) -> Result<Vec<String>> {
        let scoped = scope_ids.is_some();

        if let Some(ids) = scope_ids {
            self.conn.execute_batch("CREATE TEMP TABLE IF NOT EXISTS _bs_scope (asset_id TEXT PRIMARY KEY)")?;
            self.conn.execute_batch("DELETE FROM _bs_scope")?;
            for chunk in ids.chunks(500) {
                let placeholders: Vec<&str> = chunk.iter().map(|_| "(?)").collect();
                let sql = format!("INSERT OR IGNORE INTO _bs_scope (asset_id) VALUES {}", placeholders.join(","));
                let params: Vec<&dyn rusqlite::types::ToSql> = chunk.iter().map(|id| id as &dyn rusqlite::types::ToSql).collect();
                self.conn.execute(&sql, params.as_slice())?;
            }
        }

        let scope_filter = if scoped {
            "JOIN _bs_scope bs ON bs.asset_id = a.id"
        } else {
            ""
        };

        let mut stmt = self.conn.prepare(&format!(
            "SELECT a.id FROM assets a {} \
             WHERE NOT EXISTS ( \
                SELECT 1 FROM variants v \
                JOIN file_locations fl ON fl.content_hash = v.content_hash \
                WHERE v.asset_id = a.id AND fl.volume_id = ?1 \
             )",
            scope_filter,
        ))?;
        let rows = stmt.query_map([volume_id], |r| r.get::<_, String>(0))?;
        let mut ids = Vec::new();
        for row in rows {
            ids.push(row?);
        }

        if scoped {
            let _ = self.conn.execute_batch("DROP TABLE IF EXISTS _bs_scope");
        }

        Ok(ids)
    }

}
