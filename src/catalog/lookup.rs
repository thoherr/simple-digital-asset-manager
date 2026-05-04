//! `lookup` section of `Catalog` — methods extracted from the original
//! 9.2-kLOC catalog.rs into a multi-file `impl Catalog` block.
//!
//! Types, helper functions, and the connection ctor live in the parent
//! `catalog` module.

use super::*;

impl Catalog {
    // ═══ ASSET LOOKUPS ═══

    /// Search assets by optional filters. Results join assets with variants.
    pub fn search_assets(
        &self,
        text: Option<&str>,
        asset_type: Option<&str>,
        tag: Option<&str>,
        format: Option<&str>,
        rating_min: Option<u8>,
        rating_exact: Option<u8>,
    ) -> Result<Vec<SearchRow>> {
        let asset_types_vec;
        let tags_vec;
        let formats_vec;
        let opts = SearchOptions {
            text,
            asset_types: if let Some(t) = asset_type {
                asset_types_vec = vec![t.to_string()];
                &asset_types_vec
            } else {
                &[]
            },
            tags: if let Some(t) = tag {
                tags_vec = vec![t.to_string()];
                &tags_vec
            } else {
                &[]
            },
            formats: if let Some(f) = format {
                formats_vec = vec![f.to_string()];
                &formats_vec
            } else {
                &[]
            },
            rating: if let Some(min) = rating_min {
                Some(NumericFilter::Min(min as f64))
            } else if let Some(exact) = rating_exact {
                Some(NumericFilter::Exact(exact as f64))
            } else {
                None
            },
            per_page: u32::MAX,
            ..Default::default()
        };
        self.search_paginated(&opts)
    }

    /// Resolve a short asset ID prefix to a full UUID string.
    ///
    /// Returns `Ok(Some(id))` if exactly one match, `Ok(None)` if no match,
    /// or an error if the prefix is ambiguous (multiple matches).
    pub fn resolve_asset_id(&self, prefix: &str) -> Result<Option<String>> {
        let prefix = prefix.trim();
        let pattern = format!("{prefix}%");
        let mut stmt = self.conn.prepare(
            "SELECT id FROM assets WHERE id LIKE ?1",
        )?;
        let ids: Vec<String> = stmt
            .query_map(rusqlite::params![pattern], |row| row.get(0))?
            .collect::<std::result::Result<_, _>>()?;

        match ids.len() {
            0 => Ok(None),
            1 => Ok(Some(ids.into_iter().next().unwrap())),
            n => anyhow::bail!(
                "Ambiguous asset ID prefix '{prefix}': matches {n} assets"
            ),
        }
    }

    /// Load full asset details from the catalog (variants + locations).
    pub fn load_asset_details(&self, asset_id: &str) -> Result<Option<AssetDetails>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, asset_type, created_at, tags, description, rating, color_label \
             FROM assets WHERE id = ?1",
        )?;
        let mut rows = stmt.query(rusqlite::params![asset_id])?;
        let row = match rows.next()? {
            Some(r) => r,
            None => return Ok(None),
        };

        let id: String = row.get(0)?;
        let name: Option<String> = row.get(1)?;
        let asset_type: String = row.get(2)?;
        let created_at: String = row.get(3)?;
        let tags_json: String = row.get(4)?;
        let description: Option<String> = row.get(5)?;
        let rating_val: Option<i64> = row.get(6)?;
        let rating = rating_val.map(|r| r as u8);
        let color_label: Option<String> = row.get(7)?;
        let tags: Vec<String> = serde_json::from_str(&tags_json).unwrap_or_default();

        // Load variants
        let mut vstmt = self.conn.prepare(
            "SELECT content_hash, role, format, file_size, original_filename, source_metadata \
             FROM variants WHERE asset_id = ?1",
        )?;
        let variants: Vec<VariantDetails> = vstmt
            .query_map(rusqlite::params![asset_id], |vrow| {
                let meta_json: String = vrow.get(5)?;
                let source_metadata: std::collections::HashMap<String, String> =
                    serde_json::from_str(&meta_json).unwrap_or_default();
                Ok(VariantDetails {
                    content_hash: vrow.get(0)?,
                    role: vrow.get(1)?,
                    format: vrow.get(2)?,
                    file_size: vrow.get(3)?,
                    original_filename: vrow.get(4)?,
                    source_metadata,
                    locations: Vec::new(), // filled below
                })
            })?
            .collect::<std::result::Result<_, _>>()?;

        // Load locations for each variant
        let mut lstmt = self.conn.prepare(
            "SELECT fl.relative_path, vol.label, vol.id, vol.purpose, fl.verified_at \
             FROM file_locations fl \
             JOIN volumes vol ON fl.volume_id = vol.id \
             WHERE fl.content_hash = ?1",
        )?;

        let variants: Vec<VariantDetails> = variants
            .into_iter()
            .map(|mut v| {
                let locs: Vec<LocationDetails> = lstmt
                    .query_map(rusqlite::params![v.content_hash], |lrow| {
                        Ok(LocationDetails {
                            relative_path: lrow.get(0)?,
                            volume_label: lrow.get(1)?,
                            volume_id: lrow.get(2)?,
                            volume_purpose: lrow.get(3)?,
                            verified_at: lrow.get(4)?,
                        })
                    })
                    .unwrap_or_else(|_| {
                        // Return an empty iterator wrapper on error
                        panic!("failed to query locations")
                    })
                    .filter_map(|r| r.ok())
                    .collect();
                v.locations = locs;
                v
            })
            .collect();

        // Load recipes linked to any variant of this asset
        let mut rstmt = self.conn.prepare(
            "SELECT r.variant_hash, r.software, r.recipe_type, r.content_hash, r.volume_id, \
                    vol.label, r.relative_path, r.pending_writeback \
             FROM recipes r \
             JOIN variants v ON r.variant_hash = v.content_hash \
             LEFT JOIN volumes vol ON r.volume_id = vol.id \
             WHERE v.asset_id = ?1",
        )?;
        let recipes: Vec<RecipeDetails> = rstmt
            .query_map(rusqlite::params![asset_id], |rrow| {
                Ok(RecipeDetails {
                    variant_hash: rrow.get(0)?,
                    software: rrow.get(1)?,
                    recipe_type: rrow.get(2)?,
                    content_hash: rrow.get(3)?,
                    volume_id: rrow.get(4)?,
                    volume_label: rrow.get(5)?,
                    relative_path: rrow.get(6)?,
                    pending_writeback: rrow.get::<_, i32>(7).unwrap_or(0) != 0,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        Ok(Some(AssetDetails {
            id,
            name,
            asset_type,
            created_at,
            tags,
            description,
            rating,
            color_label,
            variants,
            recipes,
        }))
    }

    /// Find which asset owns a variant by its content hash.
    pub fn find_asset_id_by_variant(&self, content_hash: &str) -> Result<Option<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT asset_id FROM variants WHERE content_hash = ?1",
        )?;
        let mut rows = stmt.query(rusqlite::params![content_hash])?;
        match rows.next()? {
            Some(row) => Ok(Some(row.get(0)?)),
            None => Ok(None),
        }
    }

    /// Get an asset's name by ID.
    pub fn get_asset_name(&self, asset_id: &str) -> Result<Option<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT COALESCE(a.name, bv.original_filename) FROM assets a \
             LEFT JOIN variants bv ON bv.content_hash = a.best_variant_hash \
             WHERE a.id = ?1",
        )?;
        let mut rows = stmt.query(rusqlite::params![asset_id])?;
        match rows.next()? {
            Some(row) => Ok(row.get(0)?),
            None => Ok(None),
        }
    }

    /// Get an asset's best_variant_hash by ID.
    pub fn get_asset_best_variant_hash(&self, asset_id: &str) -> Result<Option<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT best_variant_hash FROM assets WHERE id = ?1",
        )?;
        let mut rows = stmt.query(rusqlite::params![asset_id])?;
        match rows.next()? {
            Some(row) => Ok(row.get(0)?),
            None => Ok(None),
        }
    }

    /// Reassign a variant to a different asset in the catalog.
    pub fn update_variant_asset_id(&self, content_hash: &str, new_asset_id: &str) -> Result<()> {
        let changed = self.conn.execute(
            "UPDATE variants SET asset_id = ?1 WHERE content_hash = ?2",
            rusqlite::params![new_asset_id, content_hash],
        )?;
        if changed == 0 {
            anyhow::bail!("no variant found with hash '{content_hash}'");
        }
        Ok(())
    }

    /// Update a variant's role in the catalog.
    pub fn update_variant_role(&self, content_hash: &str, role: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE variants SET role = ?1 WHERE content_hash = ?2",
            rusqlite::params![role, content_hash],
        )?;
        Ok(())
    }

    /// Delete an asset row from the catalog.
    pub fn delete_asset(&self, asset_id: &str) -> Result<()> {
        let changed = self.conn.execute(
            "DELETE FROM assets WHERE id = ?1",
            rusqlite::params![asset_id],
        )?;
        if changed == 0 {
            anyhow::bail!("no asset found with id '{asset_id}'");
        }
        Ok(())
    }

    /// Load enriched location details for a variant hash.
    pub(super) fn load_locations_for_hash(
        lstmt: &mut rusqlite::Statement,
        content_hash: &str,
    ) -> Vec<LocationDetails> {
        lstmt
            .query_map(rusqlite::params![content_hash], |lrow| {
                Ok(LocationDetails {
                    relative_path: lrow.get(0)?,
                    volume_label: lrow.get(1)?,
                    volume_id: lrow.get(2)?,
                    volume_purpose: lrow.get(3)?,
                    verified_at: lrow.get(4)?,
                })
            })
            .unwrap()
            .filter_map(|r| r.ok())
            .collect()
    }

    /// Compute `volume_count` and `same_volume_groups` from locations.
    pub(super) fn compute_duplicate_stats(entry: &mut DuplicateEntry) {
        let mut vol_counts: HashMap<String, usize> = HashMap::new();
        for loc in &entry.locations {
            *vol_counts.entry(loc.volume_id.clone()).or_insert(0) += 1;
        }
        entry.volume_count = vol_counts.len();
        // Find volume labels where the same volume has 2+ locations
        let mut same_vol: Vec<String> = Vec::new();
        for loc in &entry.locations {
            let count = vol_counts.get(&loc.volume_id).copied().unwrap_or(0);
            if count > 1 && !same_vol.contains(&loc.volume_label) {
                same_vol.push(loc.volume_label.clone());
            }
        }
        entry.same_volume_groups = same_vol;
    }

    /// Load duplicate entries from a variant query and enrich with locations.
    pub(super) fn load_duplicate_entries(
        &self,
        variant_query: &str,
    ) -> Result<Vec<DuplicateEntry>> {
        let mut stmt = self.conn.prepare(variant_query)?;

        let entries: Vec<DuplicateEntry> = stmt
            .query_map([], |row| {
                Ok(DuplicateEntry {
                    content_hash: row.get(0)?,
                    original_filename: row.get(1)?,
                    format: row.get(2)?,
                    file_size: row.get(3)?,
                    asset_name: row.get(4)?,
                    asset_id: row.get(5)?,
                    locations: Vec::new(),
                    volume_count: 0,
                    same_volume_groups: Vec::new(),
                    preview_url: String::new(),
                })
            })?
            .collect::<std::result::Result<_, _>>()?;

        let mut lstmt = self.conn.prepare(
            "SELECT fl.relative_path, vol.label, vol.id, vol.purpose, fl.verified_at \
             FROM file_locations fl \
             JOIN volumes vol ON fl.volume_id = vol.id \
             WHERE fl.content_hash = ?1",
        )?;

        let entries: Vec<DuplicateEntry> = entries
            .into_iter()
            .map(|mut e| {
                e.locations = Self::load_locations_for_hash(&mut lstmt, &e.content_hash);
                Self::compute_duplicate_stats(&mut e);
                e
            })
            .collect();

        Ok(entries)
    }

}
