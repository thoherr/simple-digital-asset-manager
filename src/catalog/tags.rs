//! `tags` section of `Catalog` — methods extracted from the original
//! 9.2-kLOC catalog.rs into a multi-file `impl Catalog` block.
//!
//! Types, helper functions, and the connection ctor live in the parent
//! `catalog` module.

use super::*;

impl Catalog {
    // ═══ TAG & FORMAT QUERIES ═══

    /// List all unique tags with their usage counts, sorted by count descending.
    pub fn list_all_tags(&self) -> Result<Vec<(String, u64)>> {
        // Use json_each() for SQL-side aggregation — avoids loading all 150k+ tag JSON blobs
        let mut stmt = self.conn.prepare(
            "SELECT je.value, COUNT(*) as cnt \
             FROM assets, json_each(assets.tags) AS je \
             WHERE assets.tags != '[]' \
             GROUP BY je.value \
             ORDER BY cnt DESC, je.value ASC",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, u64>(1)?))
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// For each tag value present in the catalogue, count assets where that
    /// tag is a *leaf* — i.e. the asset has the tag but no other tag of the
    /// form `<tag>|...` on the same asset.
    ///
    /// This is the count that matches the browse chip's `/tag` (leaf-only)
    /// search semantic, and is meaningful in isolation from the
    /// auto-expanded `own_count` returned by `list_all_tags`. For a parent
    /// node like `location`, leaf-count equals "assets tagged at exactly
    /// that level" — typically a small number that surfaces lazily-tagged
    /// assets (parent-tagged but not specialised into a child).
    ///
    /// Pure SQL via the same `json_each` engine `list_all_tags` uses,
    /// with a NOT EXISTS subquery checking for any descendant on the same
    /// asset's tag list.
    pub fn list_leaf_tag_counts(&self) -> Result<std::collections::HashMap<String, u64>> {
        let mut stmt = self.conn.prepare(
            "SELECT je.value, COUNT(*) as cnt \
             FROM assets a, json_each(a.tags) AS je \
             WHERE a.tags != '[]' \
               AND NOT EXISTS ( \
                   SELECT 1 FROM json_each(a.tags) AS je2 \
                   WHERE je2.value LIKE je.value || '|%' \
               ) \
             GROUP BY je.value",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, u64>(1)?))
        })?;
        let mut map = std::collections::HashMap::new();
        for row in rows {
            let (k, v) = row?;
            map.insert(k, v);
        }
        Ok(map)
    }

    /// Find assets with a specific exact tag, returning (asset_id, stack_id) pairs.
    /// Ordered by created_at ASC so the oldest asset comes first.
    pub fn assets_with_exact_tag(&self, tag: &str) -> Result<Vec<(String, Option<String>)>> {
        let mut stmt = self.conn.prepare(
            "SELECT a.id, a.stack_id \
             FROM assets a, json_each(a.tags) AS je \
             WHERE je.value = ?1 COLLATE NOCASE \
             ORDER BY a.created_at ASC",
        )?;
        let rows = stmt.query_map(rusqlite::params![tag], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Find assets that have a tag matching exactly or starting with `tag|` (prefix match).
    /// Used by tag rename to cascade renames to descendant tags.
    /// Find assets whose tag set contains `tag`, optionally including descendants
    /// (`tag|child`) and optionally case-sensitive.
    ///
    /// - `case_sensitive = false` (default): uses `COLLATE NOCASE` for the equality
    ///   check and `LIKE … COLLATE NOCASE` for the prefix check.
    /// - `case_sensitive = true`: uses byte-exact equality and `GLOB` for the
    ///   prefix check (GLOB is case-sensitive in SQLite).
    /// - `exact_only = true`: skips the descendant prefix check, returning only
    ///   assets tagged at exactly this level.
    pub fn assets_with_tag_or_prefix(
        &self,
        tag: &str,
        case_sensitive: bool,
        exact_only: bool,
    ) -> Result<Vec<(String, Option<String>)>> {
        let (sql, params): (String, Vec<Box<dyn rusqlite::types::ToSql>>) = match (case_sensitive, exact_only) {
            (false, false) => {
                // Default: case-insensitive, include descendants
                (
                    "SELECT DISTINCT a.id, a.stack_id \
                     FROM assets a, json_each(a.tags) AS je \
                     WHERE je.value = ?1 COLLATE NOCASE \
                        OR je.value LIKE ?2 COLLATE NOCASE \
                     ORDER BY a.created_at ASC".to_string(),
                    vec![Box::new(tag.to_string()), Box::new(format!("{}|%", tag))],
                )
            }
            (false, true) => {
                // Case-insensitive, exact level only (no descendants)
                (
                    "SELECT DISTINCT a.id, a.stack_id \
                     FROM assets a, json_each(a.tags) AS je \
                     WHERE je.value = ?1 COLLATE NOCASE \
                     ORDER BY a.created_at ASC".to_string(),
                    vec![Box::new(tag.to_string())],
                )
            }
            (true, false) => {
                // Case-sensitive, include descendants. GLOB uses `*` as wildcard.
                (
                    "SELECT DISTINCT a.id, a.stack_id \
                     FROM assets a, json_each(a.tags) AS je \
                     WHERE je.value = ?1 \
                        OR je.value GLOB ?2 \
                     ORDER BY a.created_at ASC".to_string(),
                    vec![Box::new(tag.to_string()), Box::new(format!("{}|*", tag))],
                )
            }
            (true, true) => {
                // Case-sensitive, exact level only
                (
                    "SELECT DISTINCT a.id, a.stack_id \
                     FROM assets a, json_each(a.tags) AS je \
                     WHERE je.value = ?1 \
                     ORDER BY a.created_at ASC".to_string(),
                    vec![Box::new(tag.to_string())],
                )
            }
        };
        let mut stmt = self.conn.prepare(&sql)?;
        let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
        let rows = stmt.query_map(param_refs.as_slice(), |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// List all distinct variant formats.
    pub fn list_all_formats(&self) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT format FROM variants ORDER BY format",
        )?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        rows.collect::<std::result::Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// List all variant formats with their counts (for grouped format filter).
    pub fn list_all_format_counts(&self) -> Result<Vec<(String, u64)>> {
        let mut stmt = self.conn.prepare(
            "SELECT format, COUNT(*) as cnt FROM variants GROUP BY format ORDER BY cnt DESC",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, u64>(1)?))
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// List all volumes from the catalog's volumes table.
    pub fn list_volumes(&self) -> Result<Vec<(String, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, label FROM volumes ORDER BY label",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect::<std::result::Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Per-volume statistics (before merging device registry).
    pub(super) fn stats_per_volume(&self) -> Result<Vec<VolumeStatsRaw>> {
        // Combined query: core counts + variant counts + verification — single pass over file_locations
        #[allow(clippy::type_complexity)]
        let mut core: HashMap<String, (String, u64, u64, u64, u64, u64, Option<String>)> = HashMap::new();
        {
            let mut stmt = self.conn.prepare(
                "SELECT fl.volume_id, v.label, \
                 COUNT(*) AS loc_count, \
                 COUNT(DISTINCT va.asset_id) AS asset_count, \
                 COALESCE(SUM(va.file_size), 0) AS total_size, \
                 COUNT(DISTINCT fl.content_hash) AS variant_count, \
                 SUM(CASE WHEN fl.verified_at IS NOT NULL THEN 1 ELSE 0 END) AS verified_count, \
                 MIN(fl.verified_at) AS oldest_verified \
                 FROM file_locations fl \
                 JOIN volumes v ON fl.volume_id = v.id \
                 JOIN variants va ON fl.content_hash = va.content_hash \
                 GROUP BY fl.volume_id",
            )?;
            let rows = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, u64>(2)?,
                    r.get::<_, u64>(3)?,
                    r.get::<_, u64>(4)?,
                    r.get::<_, u64>(5)?,
                    r.get::<_, u64>(6)?,
                    r.get::<_, Option<String>>(7)?,
                ))
            })?;
            for row in rows {
                let (vid, label, loc_count, asset_count, size, variants, verified, oldest) = row?;
                core.insert(vid, (label, loc_count, asset_count, size, variants, verified, oldest));
            }
        }

        // Directory counting — SQL-side using RTRIM trick for parent path extraction
        let mut dirs_per_vol: HashMap<String, u64> = HashMap::new();
        {
            let mut stmt = self.conn.prepare(
                "SELECT volume_id, COUNT(*) FROM ( \
                    SELECT DISTINCT volume_id, \
                        RTRIM(RTRIM(relative_path, REPLACE(relative_path, '/', '')), '/') AS parent_dir \
                    FROM file_locations \
                 ) GROUP BY volume_id",
            )?;
            let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, u64>(1)?)))?;
            for row in rows {
                let (vid, count) = row?;
                dirs_per_vol.insert(vid, count);
            }
        }

        // Formats per volume
        let mut formats_per_vol: HashMap<String, Vec<String>> = HashMap::new();
        {
            let mut stmt = self.conn.prepare(
                "SELECT DISTINCT fl.volume_id, va.format \
                 FROM file_locations fl \
                 JOIN variants va ON fl.content_hash = va.content_hash \
                 ORDER BY fl.volume_id, va.format",
            )?;
            let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
            for row in rows {
                let (vid, fmt) = row?;
                formats_per_vol.entry(vid).or_default().push(fmt);
            }
        }

        // Recipe count per volume
        let mut recipes_per_vol: HashMap<String, u64> = HashMap::new();
        {
            let mut stmt = self.conn.prepare(
                "SELECT volume_id, COUNT(*) FROM recipes WHERE volume_id IS NOT NULL GROUP BY volume_id",
            )?;
            let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, u64>(1)?)))?;
            for row in rows {
                let (vid, count) = row?;
                recipes_per_vol.insert(vid, count);
            }
        }

        // All volumes (including those with no file_locations)
        let mut all_volume_ids: HashMap<String, String> = HashMap::new();
        {
            let mut stmt = self.conn.prepare("SELECT id, label FROM volumes")?;
            let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
            for row in rows {
                let (vid, label) = row?;
                all_volume_ids.insert(vid, label);
            }
        }

        // Merge all data
        let mut result = Vec::new();
        for (vid, label) in &all_volume_ids {
            let (_, loc_count, asset_count, size, variants, verified, oldest) = core
                .get(vid)
                .cloned()
                .unwrap_or_else(|| (label.clone(), 0, 0, 0, 0, 0, None));
            let dirs = *dirs_per_vol.get(vid).unwrap_or(&0);
            let formats = formats_per_vol.remove(vid).unwrap_or_default();
            let recipes = *recipes_per_vol.get(vid).unwrap_or(&0);

            result.push(VolumeStatsRaw {
                volume_id: vid.clone(),
                label: label.clone(),
                assets: asset_count,
                variants,
                recipes,
                formats,
                directories: dirs,
                size,
                verified_count: verified,
                total_locations: loc_count,
                oldest_verified_at: oldest,
            });
        }

        result.sort_by(|a, b| a.label.cmp(&b.label));
        Ok(result)
    }

    /// Tag frequency counts: uses json_each() to expand and aggregate in SQL.
    /// Returns Vec<(tag, count)> sorted by count descending.
    pub fn stats_tag_frequencies(&self, limit: usize) -> Result<Vec<(String, u64)>> {
        let mut stmt = self.conn.prepare(
            "SELECT je.value, COUNT(*) as cnt \
             FROM assets, json_each(assets.tags) AS je \
             WHERE assets.tags != '[]' \
             GROUP BY je.value \
             ORDER BY cnt DESC, je.value ASC \
             LIMIT ?1",
        )?;
        let rows = stmt.query_map(rusqlite::params![limit as u64], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, u64>(1)?))
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Count of unique tags (uses json_each for SQL-side aggregation).
    pub fn stats_unique_tag_count(&self) -> Result<u64> {
        self.conn.query_row(
            "SELECT COUNT(DISTINCT je.value) \
             FROM assets, json_each(assets.tags) AS je \
             WHERE assets.tags != '[]'",
            [],
            |r| r.get(0),
        ).map_err(Into::into)
    }

    /// Tag coverage: (tagged_count, untagged_count).
    pub fn stats_tag_coverage(&self) -> Result<(u64, u64)> {
        self.conn.query_row(
            "SELECT \
                COALESCE(SUM(CASE WHEN tags != '[]' THEN 1 ELSE 0 END), 0), \
                COALESCE(SUM(CASE WHEN tags = '[]' THEN 1 ELSE 0 END), 0) \
             FROM assets",
            [],
            |r| Ok((r.get::<_, u64>(0)?, r.get::<_, u64>(1)?)),
        ).map_err(Into::into)
    }

    /// Verification overview for file_locations:
    /// (total, verified, oldest_verified_at, newest_verified_at).
    pub fn stats_verification_overview(&self) -> Result<(u64, u64, Option<String>, Option<String>)> {
        self.conn.query_row(
            "SELECT COUNT(*), \
                COALESCE(SUM(CASE WHEN verified_at IS NOT NULL THEN 1 ELSE 0 END), 0), \
                MIN(verified_at), \
                MAX(verified_at) \
             FROM file_locations",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        ).map_err(Into::into)
    }

    /// Verification counts for recipes: (total, verified).
    pub fn stats_recipe_verification(&self) -> Result<(u64, u64)> {
        let total: u64 = self.conn.query_row(
            "SELECT COUNT(*) FROM recipes", [], |r| r.get(0),
        )?;
        let verified: u64 = self.conn.query_row(
            "SELECT COUNT(*) FROM recipes WHERE verified_at IS NOT NULL", [], |r| r.get(0),
        )?;
        Ok((total, verified))
    }

    /// Per-volume verification: Vec<(label, volume_id, total, verified, oldest_verified_at)>.
    pub fn stats_verification_per_volume(&self) -> Result<Vec<(String, String, u64, u64, Option<String>)>> {
        let mut stmt = self.conn.prepare(
            "SELECT v.label, fl.volume_id, \
             COUNT(*) AS total, \
             SUM(CASE WHEN fl.verified_at IS NOT NULL THEN 1 ELSE 0 END) AS verified, \
             MIN(fl.verified_at) AS oldest \
             FROM file_locations fl \
             JOIN volumes v ON fl.volume_id = v.id \
             GROUP BY fl.volume_id \
             ORDER BY v.label",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, u64>(2)?,
                r.get::<_, u64>(3)?,
                r.get::<_, Option<String>>(4)?,
            ))
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Build full CatalogStats with optional sections.
    pub fn build_stats(
        &self,
        volumes_info: &[(String, String, bool, Option<String>)], // (label, volume_id, is_online, purpose)
        show_types: bool,
        show_volumes: bool,
        show_tags: bool,
        show_verified: bool,
        limit: usize,
    ) -> Result<CatalogStats> {
        let (assets, variants, recipes, total_size, file_locations) = self.stats_overview()?;
        let (_, unique_recipes) = self.stats_recipe_counts()?;

        let volumes_total = volumes_info.len() as u64;
        let volumes_online = volumes_info.iter().filter(|v| v.2).count() as u64;
        let volumes_offline = volumes_total - volumes_online;

        let overview = OverviewStats {
            assets,
            variants,
            recipes,
            file_locations,
            unique_recipes,
            volumes_total,
            volumes_online,
            volumes_offline,
            total_size,
        };

        let types = if show_types {
            let asset_types_raw = self.stats_asset_types()?;
            let total_assets = assets.max(1) as f64;
            let asset_types: Vec<TypeCount> = asset_types_raw
                .into_iter()
                .map(|(t, c)| TypeCount {
                    asset_type: t,
                    count: c,
                    percentage: (c as f64 / total_assets) * 100.0,
                })
                .collect();

            let variant_formats: Vec<FormatCount> = self
                .stats_variant_formats(limit)?
                .into_iter()
                .map(|(f, c)| FormatCount { format: f, count: c })
                .collect();

            let recipe_formats: Vec<FormatCount> = self
                .stats_recipe_formats(limit)?
                .into_iter()
                .map(|(f, c)| FormatCount { format: f, count: c })
                .collect();

            Some(TypeStats {
                asset_types,
                variant_formats,
                recipe_formats,
            })
        } else {
            None
        };

        let volumes = if show_volumes {
            let raw = self.stats_per_volume()?;
            let vol_stats: Vec<VolumeStats> = raw
                .into_iter()
                .map(|r| {
                    let vol_info = volumes_info
                        .iter()
                        .find(|v| v.1 == r.volume_id);
                    let is_online = vol_info.map(|v| v.2).unwrap_or(false);
                    let purpose = vol_info.and_then(|v| v.3.clone());
                    let verification_pct = if r.total_locations > 0 {
                        (r.verified_count as f64 / r.total_locations as f64) * 100.0
                    } else {
                        0.0
                    };
                    VolumeStats {
                        label: r.label,
                        volume_id: r.volume_id,
                        is_online,
                        purpose,
                        assets: r.assets,
                        variants: r.variants,
                        recipes: r.recipes,
                        formats: r.formats,
                        directories: r.directories,
                        size: r.size,
                        verified_count: r.verified_count,
                        total_locations: r.total_locations,
                        verification_pct,
                        oldest_verified_at: r.oldest_verified_at,
                    }
                })
                .collect();
            Some(vol_stats)
        } else {
            None
        };

        let tags = if show_tags {
            let (tagged, untagged) = self.stats_tag_coverage()?;
            let unique_tags = self.stats_unique_tag_count()?;
            let top_tags: Vec<TagCount> = self
                .stats_tag_frequencies(limit)?
                .into_iter()
                .map(|(tag, count)| TagCount { tag, count })
                .collect();

            Some(TagStats {
                unique_tags,
                tagged_assets: tagged,
                untagged_assets: untagged,
                top_tags,
            })
        } else {
            None
        };

        let verified = if show_verified {
            let (total, verified_count, oldest, newest) = self.stats_verification_overview()?;
            let coverage_pct = if total > 0 {
                (verified_count as f64 / total as f64) * 100.0
            } else {
                0.0
            };

            let per_volume_raw = self.stats_verification_per_volume()?;
            let per_volume: Vec<VolumeVerificationStats> = per_volume_raw
                .into_iter()
                .map(|(label, vid, total, verified, oldest)| {
                    let vol_info = volumes_info
                        .iter()
                        .find(|v| v.1 == vid);
                    let is_online = vol_info.map(|v| v.2).unwrap_or(false);
                    let purpose = vol_info.and_then(|v| v.3.clone());
                    let cov = if total > 0 {
                        (verified as f64 / total as f64) * 100.0
                    } else {
                        0.0
                    };
                    VolumeVerificationStats {
                        label,
                        volume_id: vid,
                        is_online,
                        purpose,
                        locations: total,
                        verified,
                        coverage_pct: cov,
                        oldest_verified_at: oldest,
                    }
                })
                .collect();

            Some(VerificationStats {
                total_locations: total,
                verified_locations: verified_count,
                unverified_locations: total - verified_count,
                coverage_pct,
                oldest_verified_at: oldest,
                newest_verified_at: newest,
                per_volume,
            })
        } else {
            None
        };

        Ok(CatalogStats {
            overview,
            types,
            volumes,
            tags,
            verified,
        })
    }

}
