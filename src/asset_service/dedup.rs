//! `dedup` section of `AssetService` — methods extracted from the original
//! 8.9-kLOC asset_service.rs into a multi-file `impl AssetService` block.
//!
//! Free functions and result types live in the parent `asset_service` module.

use super::*;

impl AssetService {
    // ═══ DEDUP ═══

    /// Remove same-volume duplicate file locations.
    ///
    /// For each variant with 2+ locations on the same volume, keeps the "best"
    /// location and removes the rest. In apply mode, deletes physical files and
    /// removes catalog/sidecar location records.
    pub fn dedup(
        &self,
        volume_filter: Option<&str>,
        format_filter: Option<&str>,
        path_prefix: Option<&str>,
        prefer: Option<&str>,
        min_copies: usize,
        apply: bool,
        on_entry: impl Fn(&str, &str, DedupStatus, &str),
    ) -> Result<DedupResult> {
        let registry = DeviceRegistry::new(&self.catalog_root);
        let catalog = Catalog::open(&self.catalog_root)?;
        let metadata_store = MetadataStore::new(&self.catalog_root);

        let filter_volume_id = if let Some(label) = volume_filter {
            let vol = registry.resolve_volume(label)?;
            Some(vol.id.to_string())
        } else {
            None
        };

        let entries = if format_filter.is_some() || path_prefix.is_some() || filter_volume_id.is_some() {
            catalog.find_duplicates_filtered(
                "same",
                filter_volume_id.as_deref(),
                format_filter,
                path_prefix,
            )?
        } else {
            catalog.find_duplicates_same_volume()?
        };

        let mut result = DedupResult {
            duplicates_found: 0,
            locations_to_remove: 0,
            locations_removed: 0,
            files_deleted: 0,
            recipes_removed: 0,
            bytes_freed: 0,
            dry_run: !apply,
            errors: Vec::new(),
        };

        // Build a map of volumes for resolving mount points
        let volumes = registry.list()?;
        let vol_map: std::collections::HashMap<String, &Volume> = volumes
            .iter()
            .map(|v| (v.id.to_string(), v))
            .collect();

        for entry in &entries {
            // Group locations by volume_id
            let mut by_volume: BTreeMap<String, Vec<&crate::catalog::LocationDetails>> =
                BTreeMap::new();
            for loc in &entry.locations {
                by_volume
                    .entry(loc.volume_id.clone())
                    .or_default()
                    .push(loc);
            }

            // Track how many locations we're removing for this variant (for min-copies)
            let mut entry_removals = 0usize;

            for (vol_id, mut locs) in by_volume {
                if locs.len() < 2 {
                    continue;
                }

                // If volume filter set, skip other volumes
                if let Some(ref fid) = filter_volume_id {
                    if &vol_id != fid {
                        continue;
                    }
                }

                result.duplicates_found += 1;

                // Sort by resolution heuristic (best first = keep)
                locs.sort_by(|a, b| {
                    // 1. Prefer locations matching --prefer substring
                    if let Some(prefix) = prefer {
                        let a_match = a.relative_path.contains(prefix);
                        let b_match = b.relative_path.contains(prefix);
                        if a_match != b_match {
                            return if a_match {
                                std::cmp::Ordering::Less
                            } else {
                                std::cmp::Ordering::Greater
                            };
                        }
                    }

                    // 2. Prefer more recently verified (NULL = oldest)
                    let a_ver = a.verified_at.as_deref().unwrap_or("");
                    let b_ver = b.verified_at.as_deref().unwrap_or("");
                    match b_ver.cmp(a_ver) {
                        std::cmp::Ordering::Equal => {}
                        other => return other,
                    }

                    // 3. Prefer shorter relative paths
                    match a.relative_path.len().cmp(&b.relative_path.len()) {
                        std::cmp::Ordering::Equal => {}
                        other => return other,
                    }

                    // 4. Tiebreak: alphabetical
                    a.relative_path.cmp(&b.relative_path)
                });

                let vol_label = locs
                    .first()
                    .map(|l| l.volume_label.as_str())
                    .unwrap_or("?");

                // Keep the first, mark the rest for removal
                on_entry(
                    &entry.original_filename,
                    &locs[0].relative_path,
                    DedupStatus::Keep,
                    vol_label,
                );

                for loc in &locs[1..] {
                    // Check min-copies constraint: total locations across all volumes
                    let remaining = entry.locations.len() - entry_removals;
                    if remaining <= min_copies {
                        on_entry(
                            &entry.original_filename,
                            &loc.relative_path,
                            DedupStatus::Skipped,
                            vol_label,
                        );
                        continue;
                    }

                    entry_removals += 1;
                    result.locations_to_remove += 1;
                    result.bytes_freed += entry.file_size;

                    on_entry(
                        &entry.original_filename,
                        &loc.relative_path,
                        DedupStatus::Remove,
                        vol_label,
                    );

                    // Find co-located recipes (same variant, same volume, same directory)
                    let loc_dir = std::path::Path::new(&loc.relative_path)
                        .parent()
                        .map(|p| p.to_string_lossy().to_string())
                        .unwrap_or_default();
                    let colocated_recipes = catalog
                        .list_recipes_for_variant_on_volume(&entry.content_hash, &vol_id)
                        .unwrap_or_default()
                        .into_iter()
                        .filter(|(_id, _hash, rpath)| {
                            let rdir = std::path::Path::new(rpath)
                                .parent()
                                .map(|p| p.to_string_lossy().to_string())
                                .unwrap_or_default();
                            rdir == loc_dir
                        })
                        .collect::<Vec<_>>();

                    if apply {
                        // Delete the physical file
                        if let Some(vol) = vol_map.get(&vol_id) {
                            if vol.is_online {
                                let full_path = vol.mount_point.join(&loc.relative_path);
                                match std::fs::remove_file(&full_path) {
                                    Ok(()) => {
                                        result.files_deleted += 1;
                                    }
                                    Err(e) => {
                                        result.errors.push(format!(
                                            "Failed to delete {}: {e}",
                                            full_path.display()
                                        ));
                                        continue;
                                    }
                                }
                            }
                        }

                        // Remove from catalog
                        if let Err(e) = catalog.delete_file_location(
                            &entry.content_hash,
                            &vol_id,
                            &loc.relative_path,
                        ) {
                            result.errors.push(format!(
                                "Failed to remove catalog location {}: {e}",
                                loc.relative_path
                            ));
                        } else if let Err(e) = self.remove_sidecar_file_location(
                            &metadata_store,
                            &catalog,
                            &entry.content_hash,
                            vol_id.parse().unwrap_or_default(),
                            &loc.relative_path,
                        ) {
                            result.errors.push(format!(
                                "Failed to update sidecar for {}: {e}",
                                loc.relative_path
                            ));
                        } else {
                            result.locations_removed += 1;
                        }

                        // Clean up co-located recipe files
                        for (recipe_id, _recipe_hash, recipe_path) in &colocated_recipes {
                            if let Some(vol) = vol_map.get(&vol_id) {
                                if vol.is_online {
                                    let recipe_full = vol.mount_point.join(recipe_path);
                                    let _ = std::fs::remove_file(&recipe_full);
                                }
                            }
                            if let Err(e) = catalog.delete_recipe(recipe_id) {
                                result.errors.push(format!(
                                    "Failed to remove recipe {recipe_path}: {e}"
                                ));
                            } else if let Err(e) = self.remove_sidecar_recipe(
                                &metadata_store,
                                &catalog,
                                &entry.content_hash,
                                vol_id.parse().unwrap_or_default(),
                                recipe_path,
                            ) {
                                result.errors.push(format!(
                                    "Failed to update sidecar for recipe {recipe_path}: {e}"
                                ));
                            } else {
                                result.recipes_removed += 1;
                            }
                        }
                    } else {
                        // Dry-run: just count recipes
                        result.recipes_removed += colocated_recipes.len();
                    }
                }
            }
        }

        Ok(result)
    }

}
