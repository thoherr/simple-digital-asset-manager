//! `verify` section of `AssetService` — methods extracted from the original
//! 8.9-kLOC asset_service.rs into a multi-file `impl AssetService` block.
//!
//! Free functions and result types live in the parent `asset_service` module.

use super::*;

impl AssetService {
    // ═══ VERIFY ═══

    /// Verify file integrity by re-hashing files and comparing against stored content hashes.
    ///
    /// Two modes:
    /// - **Path mode** (`paths` non-empty): verify specific files/directories on disk.
    /// - **Catalog mode** (`paths` empty): verify all known file locations, optionally
    ///   filtered by `volume_filter` or `asset_filter`.
    pub fn verify(
        &self,
        paths: &[PathBuf],
        volume_filter: Option<&str>,
        asset_filter: Option<&str>,
        filter: &FileTypeFilter,
        max_age_days: Option<u64>,
        on_file: impl Fn(&Path, VerifyStatus, Duration),
    ) -> Result<VerifyResult> {
        let content_store = ContentStore::new(&self.catalog_root);
        let metadata_store = MetadataStore::new(&self.catalog_root);
        let catalog = Catalog::open(&self.catalog_root)?;
        let registry = DeviceRegistry::new(&self.catalog_root);

        let mut result = VerifyResult {
            verified: 0,
            failed: 0,
            modified: 0,
            skipped: 0,
            skipped_recent: 0,
            errors: Vec::new(),
        };

        if !paths.is_empty() {
            // Path mode
            let files = resolve_files(paths, &[]);
            let volumes = registry.list()?;

            for file_path in &files {
                let file_start = std::time::Instant::now();

                // Skip files whose extension isn't in an enabled type group
                let ext = file_path
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("");
                if !ext.is_empty() && !filter.is_importable(ext) {
                    continue;
                }

                // Find which volume this file is on
                let volume = volumes.iter().find(|v| file_path.starts_with(&v.mount_point));
                let volume = match volume {
                    Some(v) => v,
                    None => {
                        result.skipped += 1;
                        result.errors.push(format!(
                            "no volume found for {}",
                            file_path.display()
                        ));
                        on_file(file_path, VerifyStatus::Skipped, file_start.elapsed());
                        continue;
                    }
                };

                let relative_path = file_path
                    .strip_prefix(&volume.mount_point)
                    .unwrap_or(file_path);

                // Skip recently verified files
                if let Some(days) = max_age_days {
                    let verified_at = catalog.get_location_verified_at(
                        &volume.id.to_string(),
                        &relative_path.to_string_lossy(),
                    )?;
                    if is_recently_verified(verified_at.as_deref(), days) {
                        result.skipped_recent += 1;
                        on_file(file_path, VerifyStatus::SkippedRecent, file_start.elapsed());
                        continue;
                    }
                }

                // Hash the file
                let hash = match content_store.hash_file(file_path) {
                    Ok(h) => h,
                    Err(e) => {
                        result.skipped += 1;
                        result.errors.push(format!(
                            "{}: {}",
                            file_path.display(),
                            e
                        ));
                        on_file(file_path, VerifyStatus::Missing, file_start.elapsed());
                        continue;
                    }
                };

                // Look up variant by hash
                match catalog.find_asset_id_by_variant(&hash)? {
                    Some(_) => {
                        // File matches a known variant — verified
                        result.verified += 1;
                        catalog.update_verified_at(
                            &hash,
                            &volume.id.to_string(),
                            &relative_path.to_string_lossy(),
                        )?;
                        // Also update sidecar verified_at
                        self.update_sidecar_verified_at(
                            &metadata_store,
                            &catalog,
                            &hash,
                            volume.id,
                            relative_path,
                        )?;
                        on_file(file_path, VerifyStatus::Ok, file_start.elapsed());
                    }
                    None => {
                        // Not a variant — check if it's a known recipe file by hash
                        if catalog.has_recipe_by_content_hash(&hash)? {
                            result.verified += 1;
                            catalog.update_recipe_verified_at(
                                &hash,
                                &volume.id.to_string(),
                                &relative_path.to_string_lossy(),
                            )?;
                            on_file(file_path, VerifyStatus::Ok, file_start.elapsed());
                        } else if let Some((recipe_id, _old_hash, variant_hash)) =
                            catalog.find_recipe_by_volume_and_path(
                                &volume.id.to_string(),
                                &relative_path.to_string_lossy(),
                            )?
                        {
                            // Recipe at this location has a different hash — modified
                            catalog.update_recipe_content_hash(&recipe_id, &hash)?;

                            // Update the sidecar via the variant's owning asset
                            if let Some(asset_id_str) = catalog.find_asset_id_by_variant(&variant_hash)? {
                                let asset_uuid: Uuid = asset_id_str.parse()?;
                                let mut asset = metadata_store.load(asset_uuid)?;
                                if let Some(recipe) = asset.recipes.iter_mut().find(|r| {
                                    r.location.volume_id == volume.id
                                        && r.location.relative_path == relative_path
                                }) {
                                    recipe.content_hash = hash.clone();

                                    let ext = relative_path.extension()
                                        .and_then(|e| e.to_str())
                                        .unwrap_or("");
                                    if ext.eq_ignore_ascii_case("xmp") {
                                        let xmp = crate::xmp_reader::extract(file_path);
                                        reapply_xmp_data(&xmp, &mut asset, &variant_hash);
                                        catalog.insert_asset(&asset)?;
                                        if let Some(v) = asset.variants.iter().find(|v| v.content_hash == variant_hash) {
                                            catalog.insert_variant(v)?;
                                        }
                                    }

                                    metadata_store.save(&asset)?;
                                }
                            }

                            result.modified += 1;
                            on_file(file_path, VerifyStatus::Modified, file_start.elapsed());
                        } else {
                            result.skipped += 1;
                            result.errors.push(format!(
                                "Untracked: {}",
                                file_path.display()
                            ));
                            on_file(file_path, VerifyStatus::Untracked, file_start.elapsed());
                        }
                    }
                }
            }
        } else {
            // Catalog mode
            let volume_filter_resolved = match volume_filter {
                Some(label) => Some(registry.resolve_volume(label)?),
                None => None,
            };

            let volumes = registry.list()?;

            let assets = if let Some(asset_id) = asset_filter {
                let full_id = catalog
                    .resolve_asset_id(asset_id)?
                    .ok_or_else(|| anyhow::anyhow!("no asset found matching '{asset_id}'"))?;
                let uuid: Uuid = full_id.parse()?;
                vec![metadata_store.load(uuid)?]
            } else if let Some(days) = max_age_days {
                // Fast path: query SQLite for asset IDs with stale locations,
                // then load only those sidecars. Avoids loading all 260k+ YAMLs.
                let vol_filter = volume_filter_resolved.as_ref().map(|v| v.id.to_string());
                let stale_ids = catalog.find_assets_with_stale_locations(days, vol_filter.as_deref())?;
                // Count skipped locations: total (file_locations + recipes) minus stale
                let total_fl = catalog.count_file_locations(vol_filter.as_deref())?;
                let stale_fl = catalog.count_stale_locations(days, vol_filter.as_deref())?;
                let total_recipes: usize = if let Some(vid) = vol_filter.as_deref() {
                    catalog.conn().query_row(
                        "SELECT COUNT(*) FROM recipes WHERE volume_id = ?1",
                        rusqlite::params![vid], |r| r.get(0)
                    ).unwrap_or(0)
                } else {
                    catalog.conn().query_row("SELECT COUNT(*) FROM recipes", [], |r| r.get(0)).unwrap_or(0)
                };
                // File locations that are recent + all recipes for recently-verified assets
                let stale_asset_count = stale_ids.len();
                let total_assets = catalog.conn().query_row("SELECT COUNT(*) FROM assets", [], |r| r.get::<_, usize>(0)).unwrap_or(0);
                let recent_recipes = if total_assets > 0 && stale_asset_count < total_assets {
                    // Proportional estimate of recipe locations for recently-verified assets
                    total_recipes.saturating_sub(
                        (total_recipes as f64 * stale_asset_count as f64 / total_assets as f64) as usize
                    )
                } else {
                    0
                };
                result.skipped_recent = total_fl.saturating_sub(stale_fl) + recent_recipes;
                if self.verbosity.verbose {
                    eprintln!("  Verify: {} asset(s) with stale locations, {} location(s) skipped as recent",
                        stale_ids.len(), result.skipped_recent);
                }
                stale_ids
                    .iter()
                    .filter_map(|id| id.parse::<Uuid>().ok())
                    .filter_map(|uuid| metadata_store.load(uuid).ok())
                    .collect()
            } else {
                let summaries = metadata_store.list()?;
                summaries
                    .iter()
                    .map(|s| metadata_store.load(s.id))
                    .collect::<Result<Vec<_>>>()?
            };

            for asset in &assets {
                // Verify variant file locations
                for variant in &asset.variants {
                    for loc in &variant.locations {
                        self.verify_location(
                            &content_store,
                            &catalog,
                            &metadata_store,
                            &volumes,
                            volume_filter_resolved.as_ref(),
                            &variant.content_hash,
                            loc,
                            None,
                            max_age_days,
                            &mut result,
                            &on_file,
                        )?;
                    }
                }

                // Verify recipe file locations
                for recipe in &asset.recipes {
                    self.verify_location(
                        &content_store,
                        &catalog,
                        &metadata_store,
                        &volumes,
                        volume_filter_resolved.as_ref(),
                        &recipe.content_hash,
                        &recipe.location,
                        Some(&recipe.variant_hash),
                        max_age_days,
                        &mut result,
                        &on_file,
                    )?;
                }
            }
        }

        Ok(result)
    }

    /// Verify a single file location (used by catalog mode).
    #[allow(clippy::too_many_arguments)]
    fn verify_location(
        &self,
        content_store: &ContentStore,
        catalog: &Catalog,
        metadata_store: &MetadataStore,
        volumes: &[Volume],
        volume_filter: Option<&Volume>,
        content_hash: &str,
        loc: &FileLocation,
        recipe_variant_hash: Option<&str>,
        max_age_days: Option<u64>,
        result: &mut VerifyResult,
        on_file: &impl Fn(&Path, VerifyStatus, Duration),
    ) -> Result<()> {
        let file_start = std::time::Instant::now();

        // Apply volume filter
        if let Some(filter_vol) = volume_filter {
            if loc.volume_id != filter_vol.id {
                return Ok(());
            }
        }

        // Skip recently verified files
        if let Some(days) = max_age_days {
            if let Some(ref verified_at) = loc.verified_at {
                let age = chrono::Utc::now() - *verified_at;
                if age.num_days() < days as i64 {
                    result.skipped_recent += 1;
                    on_file(&loc.relative_path, VerifyStatus::SkippedRecent, file_start.elapsed());
                    return Ok(());
                }
            }
        }

        // Find the volume
        let volume = match volumes.iter().find(|v| v.id == loc.volume_id) {
            Some(v) => v,
            None => {
                result.skipped += 1;
                result.errors.push(format!(
                    "Volume {} not found for {}",
                    loc.volume_id,
                    loc.relative_path.display()
                ));
                on_file(&loc.relative_path, VerifyStatus::Skipped, file_start.elapsed());
                return Ok(());
            }
        };

        // Skip offline volumes
        if !volume.is_online {
            result.skipped += 1;
            on_file(&loc.relative_path, VerifyStatus::Skipped, file_start.elapsed());
            return Ok(());
        }

        let full_path = volume.mount_point.join(&loc.relative_path);

        if !full_path.exists() {
            result.skipped += 1;
            result.errors.push(format!(
                "Missing: {} ({}:{})",
                full_path.display(),
                volume.label,
                loc.relative_path.display()
            ));
            on_file(&full_path, VerifyStatus::Missing, file_start.elapsed());
            return Ok(());
        }

        match content_store.verify(content_hash, &full_path) {
            Ok(true) => {
                result.verified += 1;
                if let Some(variant_hash) = recipe_variant_hash {
                    catalog.update_recipe_verified_at(
                        variant_hash,
                        &volume.id.to_string(),
                        &loc.relative_path.to_string_lossy(),
                    )?;
                    self.update_sidecar_recipe_verified_at(
                        metadata_store,
                        catalog,
                        variant_hash,
                        loc.volume_id,
                        &loc.relative_path,
                    )?;
                } else {
                    catalog.update_verified_at(
                        content_hash,
                        &volume.id.to_string(),
                        &loc.relative_path.to_string_lossy(),
                    )?;
                    self.update_sidecar_verified_at(
                        metadata_store,
                        catalog,
                        content_hash,
                        volume.id,
                        &loc.relative_path,
                    )?;
                }
                on_file(&full_path, VerifyStatus::Ok, file_start.elapsed());
            }
            Ok(false) => {
                if let Some(variant_hash) = recipe_variant_hash {
                    // Recipe files are expected to change — report as modified, not failed
                    let new_hash = content_store.hash_file(&full_path)?;

                    // Update the recipe's stored hash in the catalog
                    if let Some((recipe_id, _, _)) = catalog.find_recipe_by_volume_and_path(
                        &volume.id.to_string(),
                        &loc.relative_path.to_string_lossy(),
                    )? {
                        catalog.update_recipe_content_hash(&recipe_id, &new_hash)?;
                    }

                    // Update the sidecar file
                    if let Some(asset_id) = catalog.find_asset_id_by_variant(variant_hash)? {
                        let uuid: Uuid = asset_id.parse()?;
                        let mut asset = metadata_store.load(uuid)?;
                        if let Some(recipe) = asset.recipes.iter_mut().find(|r| {
                            r.location.volume_id == loc.volume_id
                                && r.location.relative_path == loc.relative_path
                        }) {
                            recipe.content_hash = new_hash.clone();

                            // Re-extract XMP data if applicable
                            let ext = loc.relative_path.extension()
                                .and_then(|e| e.to_str())
                                .unwrap_or("");
                            if ext.eq_ignore_ascii_case("xmp") {
                                let xmp = crate::xmp_reader::extract(&full_path);
                                reapply_xmp_data(&xmp, &mut asset, variant_hash);
                                catalog.insert_asset(&asset)?;
                                if let Some(v) = asset.variants.iter().find(|v| v.content_hash == variant_hash) {
                                    catalog.insert_variant(v)?;
                                }
                            }

                            metadata_store.save(&asset)?;
                        }
                    }

                    result.modified += 1;
                    on_file(&full_path, VerifyStatus::Modified, file_start.elapsed());
                } else {
                    result.failed += 1;
                    result.errors.push(format!(
                        "FAILED: {} ({}:{})",
                        full_path.display(),
                        volume.label,
                        loc.relative_path.display()
                    ));
                    on_file(&full_path, VerifyStatus::Mismatch, file_start.elapsed());
                }
            }
            Err(e) => {
                result.skipped += 1;
                result.errors.push(format!(
                    "Error reading {}: {}",
                    full_path.display(),
                    e
                ));
                on_file(&full_path, VerifyStatus::Missing, file_start.elapsed());
            }
        }

        Ok(())
    }

    /// Update the `verified_at` timestamp in the sidecar YAML for a specific file location.
    fn update_sidecar_verified_at(
        &self,
        metadata_store: &MetadataStore,
        catalog: &Catalog,
        content_hash: &str,
        volume_id: Uuid,
        relative_path: &Path,
    ) -> Result<()> {
        let asset_id = match catalog.find_asset_id_by_variant(content_hash)? {
            Some(id) => id,
            None => return Ok(()),
        };
        let uuid: Uuid = asset_id.parse()?;
        let mut asset = metadata_store.load(uuid)?;
        let now = chrono::Utc::now();

        let mut changed = false;
        for variant in &mut asset.variants {
            if variant.content_hash == content_hash {
                for loc in &mut variant.locations {
                    if loc.volume_id == volume_id && loc.relative_path == relative_path {
                        loc.verified_at = Some(now);
                        changed = true;
                    }
                }
            }
        }

        if changed {
            metadata_store.save(&asset)?;
        }

        Ok(())
    }

    /// Update the sidecar YAML with a recipe's verified_at timestamp.
    fn update_sidecar_recipe_verified_at(
        &self,
        metadata_store: &MetadataStore,
        catalog: &Catalog,
        variant_hash: &str,
        volume_id: Uuid,
        relative_path: &Path,
    ) -> Result<()> {
        let asset_id = match catalog.find_asset_id_by_variant(variant_hash)? {
            Some(id) => id,
            None => return Ok(()),
        };
        let uuid: Uuid = asset_id.parse()?;
        let mut asset = metadata_store.load(uuid)?;
        let now = chrono::Utc::now();

        let mut changed = false;
        for recipe in &mut asset.recipes {
            if recipe.location.volume_id == volume_id
                && recipe.location.relative_path == relative_path
            {
                recipe.location.verified_at = Some(now);
                changed = true;
            }
        }

        if changed {
            metadata_store.save(&asset)?;
        }

        Ok(())
    }

}
