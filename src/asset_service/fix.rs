//! `fix` section of `AssetService` — methods extracted from the original
//! 8.9-kLOC asset_service.rs into a multi-file `impl AssetService` block.
//!
//! Free functions and result types live in the parent `asset_service` module.

use super::*;

impl AssetService {
    // ═══ FIX COMMANDS ═══

    /// Fix variant roles: re-role non-RAW variants to Export in assets that have a RAW variant.
    pub fn fix_roles(
        &self,
        paths: &[PathBuf],
        volume_filter: Option<&str>,
        asset_filter: Option<&str>,
        apply: bool,
        on_asset: impl Fn(&str, FixRolesStatus),
    ) -> Result<FixRolesResult> {
        let metadata_store = MetadataStore::new(&self.catalog_root);
        let catalog = Catalog::open(&self.catalog_root)?;
        let registry = DeviceRegistry::new(&self.catalog_root);

        let mut result = FixRolesResult {
            checked: 0,
            fixed: 0,
            variants_fixed: 0,
            already_correct: 0,
            dry_run: !apply,
            errors: Vec::new(),
        };

        // Resolve asset list
        let assets = if let Some(asset_id) = asset_filter {
            let full_id = catalog
                .resolve_asset_id(asset_id)?
                .ok_or_else(|| anyhow::anyhow!("no asset found matching '{asset_id}'"))?;
            let uuid: Uuid = full_id.parse()?;
            vec![metadata_store.load(uuid)?]
        } else if !paths.is_empty() {
            // Path mode: resolve files, find their assets
            let files = resolve_files(paths, &[]);
            let volumes = registry.list()?;
            let content_store = ContentStore::new(&self.catalog_root);
            let mut asset_ids: HashSet<String> = HashSet::new();

            for file_path in &files {
                if !volumes.iter().any(|v| file_path.starts_with(&v.mount_point)) {
                    continue;
                }
                let hash = match content_store.hash_file(file_path) {
                    Ok(h) => h,
                    Err(_) => continue,
                };
                if let Some(aid) = catalog.find_asset_id_by_variant(&hash)? {
                    asset_ids.insert(aid);
                }
            }

            let mut assets = Vec::new();
            for aid in &asset_ids {
                let uuid: Uuid = aid.parse()?;
                assets.push(metadata_store.load(uuid)?);
            }
            assets
        } else {
            // Catalog mode: load all assets
            let summaries = metadata_store.list()?;
            let mut assets = Vec::new();
            for s in &summaries {
                assets.push(metadata_store.load(s.id)?);
            }
            assets
        };

        // Optional volume filter: keep only assets with at least one variant location on that volume
        let volume_filter_resolved = match volume_filter {
            Some(label) => Some(registry.resolve_volume(label)?),
            None => None,
        };

        for mut asset in assets {
            // Volume filter: skip assets without a location on the target volume
            if let Some(ref vol) = volume_filter_resolved {
                let has_location = asset.variants.iter().any(|v| {
                    v.locations.iter().any(|loc| loc.volume_id == vol.id)
                });
                if !has_location {
                    continue;
                }
            }

            result.checked += 1;

            // Skip single-variant assets
            if asset.variants.len() < 2 {
                result.already_correct += 1;
                on_asset(
                    asset.name.as_deref().unwrap_or(&asset.id.to_string()),
                    FixRolesStatus::AlreadyCorrect,
                );
                continue;
            }

            // Check if any variant is RAW
            let has_raw = asset.variants.iter().any(|v| is_raw_extension(&v.format));
            if !has_raw {
                result.already_correct += 1;
                on_asset(
                    asset.name.as_deref().unwrap_or(&asset.id.to_string()),
                    FixRolesStatus::AlreadyCorrect,
                );
                continue;
            }

            // Find non-RAW variants that should be Export but aren't
            let fixable: Vec<usize> = asset
                .variants
                .iter()
                .enumerate()
                .filter(|(_, v)| {
                    !is_raw_extension(&v.format)
                        && v.role != VariantRole::Export
                        && v.role != VariantRole::Processed
                        && v.role != VariantRole::Sidecar
                })
                .map(|(i, _)| i)
                .collect();

            if fixable.is_empty() {
                result.already_correct += 1;
                on_asset(
                    asset.name.as_deref().unwrap_or(&asset.id.to_string()),
                    FixRolesStatus::AlreadyCorrect,
                );
                continue;
            }

            if apply {
                for &idx in &fixable {
                    asset.variants[idx].role = VariantRole::Export;
                    catalog.update_variant_role(
                        &asset.variants[idx].content_hash,
                        "export",
                    )?;
                }
                metadata_store.save(&asset)?;
                catalog.update_denormalized_variant_columns(&asset)?;
            }

            result.fixed += 1;
            result.variants_fixed += fixable.len();
            on_asset(
                asset.name.as_deref().unwrap_or(&asset.id.to_string()),
                FixRolesStatus::Fixed,
            );
        }

        Ok(result)
    }

    /// Fix asset dates by examining variant metadata and file modification times.
    ///
    /// For each asset, finds the oldest plausible date from:
    /// 1. EXIF DateTimeOriginal stored in variant `source_metadata["date_taken"]`
    /// 2. Re-extracted EXIF from files on disk (for assets imported before date_taken was stored)
    /// 3. File modification time on disk
    ///
    /// Sources 2 and 3 require the volume to be online. Assets whose only locations
    /// are on offline volumes are counted as `skipped_offline`.
    ///
    /// When applying, also backfills `date_taken` into variant source_metadata so
    /// future runs work from metadata alone without needing the volume online.
    ///
    /// Report-only by default; pass `apply=true` to update sidecars and catalog.
    pub fn fix_dates(
        &self,
        volume_filter: Option<&str>,
        asset_filter: Option<&str>,
        apply: bool,
        on_asset: impl Fn(&str, FixDatesStatus, Option<&str>),
    ) -> Result<FixDatesResult> {
        use chrono::{DateTime, NaiveDateTime, Utc};

        let metadata_store = MetadataStore::new(&self.catalog_root);
        let catalog = Catalog::open(&self.catalog_root)?;
        let registry = DeviceRegistry::new(&self.catalog_root);
        let volumes = registry.list()?;

        // Collect offline volume labels for warnings
        let offline_volumes: Vec<String> = volumes.iter()
            .filter(|v| !v.is_online)
            .map(|v| v.label.clone())
            .collect();

        let mut result = FixDatesResult {
            checked: 0,
            fixed: 0,
            already_correct: 0,
            no_date: 0,
            skipped_offline: 0,
            dry_run: !apply,
            offline_volumes: offline_volumes.clone(),
            errors: Vec::new(),
        };

        // Resolve asset list
        let assets = if let Some(asset_id) = asset_filter {
            let full_id = catalog
                .resolve_asset_id(asset_id)?
                .ok_or_else(|| anyhow::anyhow!("no asset found matching '{asset_id}'"))?;
            let uuid: Uuid = full_id.parse()?;
            vec![metadata_store.load(uuid)?]
        } else {
            let summaries = metadata_store.list()?;
            let mut assets = Vec::new();
            for s in &summaries {
                assets.push(metadata_store.load(s.id)?);
            }
            assets
        };

        // Optional volume filter
        let volume_filter_resolved = match volume_filter {
            Some(label) => Some(registry.resolve_volume(label)?),
            None => None,
        };

        for mut asset in assets {
            // Volume filter: skip assets without a location on the target volume
            if let Some(ref vol) = volume_filter_resolved {
                let has_location = asset.variants.iter().any(|v| {
                    v.locations.iter().any(|loc| loc.volume_id == vol.id)
                });
                if !has_location {
                    continue;
                }
            }

            result.checked += 1;
            let asset_name = asset.name.clone()
                .unwrap_or_else(|| asset.variants.first()
                    .map(|v| v.original_filename.clone())
                    .unwrap_or_else(|| asset.id.to_string()));

            // Collect candidate dates from all variants
            let mut candidates: Vec<DateTime<Utc>> = Vec::new();
            let mut has_metadata_date = false;
            let mut all_offline = true;
            let mut backfill_dates: Vec<(usize, DateTime<Utc>)> = Vec::new();

            for (vi, variant) in asset.variants.iter().enumerate() {
                // 1. Check source_metadata for stored date_taken
                if let Some(date_str) = variant.source_metadata.get("date_taken") {
                    if let Ok(dt) = DateTime::parse_from_rfc3339(date_str) {
                        candidates.push(dt.with_timezone(&Utc));
                        has_metadata_date = true;
                    } else {
                        let s = date_str.trim_matches('"');
                        if let Ok(ndt) = NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S")
                            .or_else(|_| NaiveDateTime::parse_from_str(s, "%Y:%m:%d %H:%M:%S"))
                        {
                            candidates.push(ndt.and_utc());
                            has_metadata_date = true;
                        }
                    }
                }

                // 2. Check files on disk for online volumes
                for loc in &variant.locations {
                    if let Some(vol) = volumes.iter().find(|v| v.id == loc.volume_id) {
                        if vol.is_online {
                            all_offline = false;
                            let full_path = vol.mount_point.join(&loc.relative_path);

                            // Re-extract EXIF if no date_taken in metadata
                            if !has_metadata_date {
                                let exif_data = crate::exif_reader::extract(&full_path);
                                if let Some(dt) = exif_data.date_taken {
                                    candidates.push(dt);
                                    // Remember to backfill this date into source_metadata
                                    backfill_dates.push((vi, dt));
                                }
                            }

                            // File mtime as fallback
                            if let Some(mtime) = file_mtime(&full_path) {
                                candidates.push(mtime);
                            }
                        }
                    }
                }
            }

            // If no metadata date and all locations are offline, skip with specific status
            if candidates.is_empty() && all_offline && !asset.variants.is_empty() {
                // Check if the asset actually has locations on offline volumes
                let has_offline_locations = asset.variants.iter().any(|v| {
                    v.locations.iter().any(|loc| {
                        volumes.iter().any(|vol| vol.id == loc.volume_id && !vol.is_online)
                    })
                });
                if has_offline_locations {
                    result.skipped_offline += 1;
                    on_asset(&asset_name, FixDatesStatus::SkippedOffline, None);
                    continue;
                }
            }

            if candidates.is_empty() {
                result.no_date += 1;
                on_asset(&asset_name, FixDatesStatus::NoDate, None);
                continue;
            }

            // Pick the oldest date
            let oldest = candidates.into_iter().min().expect("candidates non-empty after date extraction");

            // Compare with current created_at (allow 1 second tolerance for rounding)
            let diff = (asset.created_at - oldest).num_seconds().abs();
            if diff <= 1 {
                // Even if date is correct, backfill date_taken into source_metadata if missing
                if apply && !backfill_dates.is_empty() {
                    for (vi, dt) in &backfill_dates {
                        asset.variants[*vi].source_metadata.insert(
                            "date_taken".to_string(),
                            dt.to_rfc3339(),
                        );
                    }
                    metadata_store.save(&asset)?;
                    catalog.insert_asset(&asset)?;
                }
                result.already_correct += 1;
                on_asset(&asset_name, FixDatesStatus::AlreadyCorrect, None);
                continue;
            }

            let old_date = asset.created_at.format("%Y-%m-%d %H:%M:%S").to_string();
            let new_date = oldest.format("%Y-%m-%d %H:%M:%S").to_string();
            let detail = format!("{old_date} → {new_date}");

            if apply {
                asset.created_at = oldest;
                // Backfill date_taken into source_metadata
                for (vi, dt) in &backfill_dates {
                    asset.variants[*vi].source_metadata.insert(
                        "date_taken".to_string(),
                        dt.to_rfc3339(),
                    );
                }
                metadata_store.save(&asset)?;
                catalog.update_asset_created_at(&asset.id.to_string(), &oldest)?;
                // Also update catalog variant metadata if we backfilled
                if !backfill_dates.is_empty() {
                    catalog.insert_asset(&asset)?;
                }
            }

            result.fixed += 1;
            on_asset(&asset_name, FixDatesStatus::Fixed, Some(&detail));
        }

        Ok(result)
    }

    /// Re-attach recipe files that were imported as standalone assets.
    /// Finds single-variant assets with recipe extensions, tries to match them
    /// to a parent variant by stem + directory, and converts them to Recipe records.
    pub fn fix_recipes(
        &self,
        volume_filter: Option<&str>,
        asset_filter: Option<&str>,
        apply: bool,
        on_asset: impl Fn(&str, FixRecipesStatus),
    ) -> Result<FixRecipesResult> {
        let metadata_store = MetadataStore::new(&self.catalog_root);
        let catalog = Catalog::open(&self.catalog_root)?;
        let registry = DeviceRegistry::new(&self.catalog_root);

        let mut result = FixRecipesResult {
            checked: 0,
            reattached: 0,
            no_parent: 0,
            skipped: 0,
            dry_run: !apply,
            errors: Vec::new(),
        };

        // Resolve optional volume filter
        let volume_id = match volume_filter {
            Some(label) => Some(registry.resolve_volume(label)?.id.to_string()),
            None => None,
        };

        // Resolve optional asset filter
        let asset_id = match asset_filter {
            Some(prefix) => Some(
                catalog
                    .resolve_asset_id(prefix)?
                    .ok_or_else(|| anyhow::anyhow!("no asset found matching '{prefix}'"))?,
            ),
            None => None,
        };

        let candidates = catalog.list_recipe_only_assets(
            volume_id.as_deref(),
            asset_id.as_deref(),
        )?;

        for (standalone_id, content_hash, format) in &candidates {
            result.checked += 1;

            // Load the standalone asset
            let standalone_uuid: Uuid = standalone_id.parse()?;
            let standalone = match metadata_store.load(standalone_uuid) {
                Ok(a) => a,
                Err(e) => {
                    result.errors.push(format!("{standalone_id}: {e}"));
                    continue;
                }
            };

            let asset_name = standalone
                .name
                .clone()
                .unwrap_or_else(|| {
                    standalone
                        .variants
                        .first()
                        .map(|v| v.original_filename.clone())
                        .unwrap_or_else(|| standalone_id.clone())
                });

            // Get the variant's file location to determine stem + directory
            let variant = match standalone.variants.first() {
                Some(v) => v,
                None => {
                    result.skipped += 1;
                    on_asset(&asset_name, FixRecipesStatus::Skipped);
                    continue;
                }
            };

            let location = match variant.locations.first() {
                Some(l) => l,
                None => {
                    // No file location — can't determine stem/directory
                    result.skipped += 1;
                    on_asset(&asset_name, FixRecipesStatus::Skipped);
                    continue;
                }
            };

            let rel_path = &location.relative_path;
            let stem = rel_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("");
            let dir_prefix = rel_path
                .parent()
                .unwrap_or_else(|| Path::new(""))
                .to_string_lossy();
            let vol_id_str = location.volume_id.to_string();

            // Try to find parent variant by stem + directory (exclude self)
            let exclude = Some(standalone_id.as_str());
            let mut parent = catalog.find_variant_hash_by_stem_and_directory(
                stem,
                &dir_prefix,
                &vol_id_str,
                exclude,
            )?;

            // If not found and stem contains a dot (compound extension like DSC_001.NRW.xmp),
            // strip the last extension and retry
            if parent.is_none() {
                if let Some(dot_pos) = stem.rfind('.') {
                    let stripped_stem = &stem[..dot_pos];
                    parent = catalog.find_variant_hash_by_stem_and_directory(
                        stripped_stem,
                        &dir_prefix,
                        &vol_id_str,
                        exclude,
                    )?;
                }
            }

            let (parent_hash, parent_asset_id) = match parent {
                Some(p) => p,
                None => {
                    result.no_parent += 1;
                    on_asset(&asset_name, FixRecipesStatus::NoParentFound);
                    continue;
                }
            };

            if apply {
                // Load parent asset
                let parent_uuid: Uuid = parent_asset_id.parse()?;
                let mut parent_asset = metadata_store.load(parent_uuid)?;

                // Create recipe record
                let recipe = Recipe {
                    id: Uuid::new_v4(),
                    variant_hash: parent_hash.clone(),
                    software: determine_recipe_software(format).to_string(),
                    recipe_type: RecipeType::Sidecar,
                    content_hash: content_hash.clone(),
                    location: location.clone(),
                    pending_writeback: false,
                };

                // Apply XMP metadata if this is an XMP file
                if format.eq_ignore_ascii_case("xmp") {
                    // Find the file on disk to extract XMP
                    let volumes = registry.list()?;
                    let vol = volumes.iter().find(|v| v.id == location.volume_id);
                    if let Some(vol) = vol {
                        if vol.is_online {
                            let file_path = vol.mount_point.join(rel_path);
                            if file_path.exists() {
                                let xmp = crate::xmp_reader::extract(&file_path);
                                apply_xmp_data(&xmp, &mut parent_asset, &parent_hash);
                            }
                        }
                    }
                }

                parent_asset.recipes.push(recipe.clone());
                metadata_store.save(&parent_asset)?;
                catalog.insert_asset(&parent_asset)?;
                if let Some(v) = parent_asset
                    .variants
                    .iter()
                    .find(|v| v.content_hash == parent_hash)
                {
                    catalog.insert_variant(v)?;
                }
                catalog.insert_recipe(&recipe)?;
                catalog.update_denormalized_variant_columns(&parent_asset)?;

                // Delete standalone asset: recipes → locations → variants → asset → sidecar
                let id_str = standalone_id.as_str();
                catalog.delete_recipes_for_asset(id_str)?;
                catalog.delete_file_locations_for_asset(id_str)?;
                catalog.delete_variants_for_asset(id_str)?;
                catalog.delete_asset(id_str)?;
                metadata_store.delete(standalone_uuid)?;
            }

            result.reattached += 1;
            on_asset(&asset_name, FixRecipesStatus::Reattached);
        }

        Ok(result)
    }

}
