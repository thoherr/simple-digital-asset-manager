//! `refresh` section of `AssetService` — methods extracted from the original
//! 8.9-kLOC asset_service.rs into a multi-file `impl AssetService` block.
//!
//! Free functions and result types live in the parent `asset_service` module.

use super::*;

impl AssetService {
    // ═══ REFRESH & SYNC METADATA ═══

    /// Re-read metadata from changed recipe/sidecar files, and optionally
    /// re-extract embedded XMP from JPEG/TIFF media files (`--media`).
    pub fn refresh(
        &self,
        paths: &[PathBuf],
        volume: Option<&Volume>,
        asset_id: Option<&str>,
        dry_run: bool,
        media: bool,
        exclude_patterns: &[String],
        on_file: impl Fn(&Path, RefreshStatus, Duration),
    ) -> Result<RefreshResult> {
        let content_store = ContentStore::new(&self.catalog_root);
        let metadata_store = MetadataStore::new(&self.catalog_root);
        let catalog = Catalog::open(&self.catalog_root)?;
        let registry = DeviceRegistry::new(&self.catalog_root);

        let mut result = RefreshResult {
            unchanged: 0,
            refreshed: 0,
            missing: 0,
            skipped: 0,
            errors: Vec::new(),
        };

        // Collect recipe locations to check: (recipe_id, content_hash, variant_hash, relative_path, volume_id_str)
        let recipe_entries: Vec<(String, String, String, String, String)>;

        if let Some(aid) = asset_id {
            // Asset mode: all recipes for a specific asset
            recipe_entries = catalog.list_recipes_for_asset(aid)?;
        } else if !paths.is_empty() {
            // Path mode: scan files under given paths, filter to recipes, look up each
            let files = resolve_files(paths, exclude_patterns);
            let filter = FileTypeFilter::default();
            let vol = volume.ok_or_else(|| anyhow::anyhow!("no volume resolved for path mode"))?;
            let vol_id = vol.id.to_string();

            let mut entries = Vec::new();
            for file_path in &files {
                let ext = file_path
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("");
                if !filter.is_recipe(ext) {
                    continue;
                }
                let relative_path = match file_path.strip_prefix(&vol.mount_point) {
                    Ok(rp) => rp.to_string_lossy().to_string(),
                    Err(_) => continue,
                };
                if let Some((recipe_id, content_hash, variant_hash)) =
                    catalog.find_recipe_by_volume_and_path(&vol_id, &relative_path)?
                {
                    entries.push((recipe_id, content_hash, variant_hash, relative_path, vol_id.clone()));
                }
            }
            recipe_entries = entries;
        } else if let Some(vol) = volume {
            // Volume mode: all recipes on the specified volume
            let vol_id = vol.id.to_string();
            recipe_entries = catalog
                .list_recipes_for_volume_under_prefix(&vol_id, "")?
                .into_iter()
                .map(|(rid, ch, vh, rp)| (rid, ch, vh, rp, vol_id.clone()))
                .collect();
        } else {
            // All mode: iterate all online volumes
            let volumes = registry.list()?;
            let mut entries = Vec::new();
            for vol in &volumes {
                if !vol.is_online {
                    continue;
                }
                let vol_id = vol.id.to_string();
                for (rid, ch, vh, rp) in
                    catalog.list_recipes_for_volume_under_prefix(&vol_id, "")?
                {
                    entries.push((rid, ch, vh, rp, vol_id.clone()));
                }
            }
            recipe_entries = entries;
        }

        // Resolve volumes for lookup
        let all_volumes = registry.list()?;

        // Process each recipe
        for (recipe_id, stored_hash, variant_hash, relative_path, volume_id_str) in &recipe_entries {
            let file_start = Instant::now();

            // Find the volume
            let vol = match all_volumes.iter().find(|v| v.id.to_string() == *volume_id_str) {
                Some(v) => v,
                None => {
                    result.skipped += 1;
                    on_file(Path::new(&relative_path), RefreshStatus::Offline, file_start.elapsed());
                    continue;
                }
            };

            if !vol.is_online {
                result.skipped += 1;
                on_file(
                    &vol.mount_point.join(relative_path),
                    RefreshStatus::Offline,
                    file_start.elapsed(),
                );
                continue;
            }

            let full_path = vol.mount_point.join(relative_path);

            if !full_path.exists() {
                result.missing += 1;
                on_file(&full_path, RefreshStatus::Missing, file_start.elapsed());
                continue;
            }

            let new_hash = match content_store.hash_file(&full_path) {
                Ok(h) => h,
                Err(e) => {
                    result.errors.push(format!("{}: {}", full_path.display(), e));
                    continue;
                }
            };

            if new_hash == *stored_hash {
                result.unchanged += 1;
                on_file(&full_path, RefreshStatus::Unchanged, file_start.elapsed());
            } else {
                if !dry_run {
                    if let Err(e) = self.apply_modified_recipe(
                        &catalog,
                        &metadata_store,
                        recipe_id,
                        &new_hash,
                        variant_hash,
                        vol,
                        &full_path,
                        relative_path,
                    ) {
                        result.errors.push(format!("{}: {}", full_path.display(), e));
                        continue;
                    }
                }
                result.refreshed += 1;
                on_file(&full_path, RefreshStatus::Refreshed, file_start.elapsed());
            }
        }

        // --- Media file processing (embedded XMP re-extraction) ---
        if media {
            // Collect media file locations: (content_hash, relative_path, volume_id)
            let media_entries: Vec<(String, String, String)>;

            if let Some(aid) = asset_id {
                media_entries = catalog.list_file_locations_for_asset(aid)?;
            } else if !paths.is_empty() {
                let files = resolve_files(paths, exclude_patterns);
                let vol = volume.ok_or_else(|| anyhow::anyhow!("no volume resolved for path mode"))?;
                let vol_id = vol.id.to_string();

                let mut entries = Vec::new();
                for file_path in &files {
                    let ext = file_path
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("");
                    if !is_embedded_xmp_extension(ext) {
                        continue;
                    }
                    let relative_path = match file_path.strip_prefix(&vol.mount_point) {
                        Ok(rp) => rp.to_string_lossy().to_string(),
                        Err(_) => continue,
                    };
                    if let Some((content_hash, _format)) =
                        catalog.find_variant_by_volume_and_path(&vol_id, &relative_path)?
                    {
                        entries.push((content_hash, relative_path, vol_id.clone()));
                    }
                }
                media_entries = entries;
            } else if let Some(vol) = volume {
                let vol_id = vol.id.to_string();
                media_entries = catalog
                    .list_locations_for_volume_under_prefix(&vol_id, "")?
                    .into_iter()
                    .map(|(ch, rp)| (ch, rp, vol_id.clone()))
                    .collect();
            } else {
                let volumes = registry.list()?;
                let mut entries = Vec::new();
                for vol in &volumes {
                    if !vol.is_online {
                        continue;
                    }
                    let vol_id = vol.id.to_string();
                    for (ch, rp) in
                        catalog.list_locations_for_volume_under_prefix(&vol_id, "")?
                    {
                        entries.push((ch, rp, vol_id.clone()));
                    }
                }
                media_entries = entries;
            }

            for (content_hash, relative_path, volume_id_str) in &media_entries {
                // Filter to JPEG/TIFF only
                let ext = Path::new(relative_path)
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("");
                if !is_embedded_xmp_extension(ext) {
                    continue;
                }

                let file_start = Instant::now();

                // Find the volume
                let vol = match all_volumes.iter().find(|v| v.id.to_string() == *volume_id_str) {
                    Some(v) => v,
                    None => {
                        result.skipped += 1;
                        on_file(Path::new(&relative_path), RefreshStatus::Offline, file_start.elapsed());
                        continue;
                    }
                };

                if !vol.is_online {
                    result.skipped += 1;
                    on_file(
                        &vol.mount_point.join(relative_path),
                        RefreshStatus::Offline,
                        file_start.elapsed(),
                    );
                    continue;
                }

                let full_path = vol.mount_point.join(relative_path);

                if !full_path.exists() {
                    result.missing += 1;
                    on_file(&full_path, RefreshStatus::Missing, file_start.elapsed());
                    continue;
                }

                let embedded_xmp = crate::embedded_xmp::extract_embedded_xmp(&full_path);

                // Check if XMP data is non-empty
                if embedded_xmp.keywords.is_empty()
                    && embedded_xmp.description.is_none()
                    && embedded_xmp.source_metadata.is_empty()
                {
                    result.unchanged += 1;
                    on_file(&full_path, RefreshStatus::Unchanged, file_start.elapsed());
                    continue;
                }

                // Load asset and re-apply embedded XMP
                let asset_id_str = match catalog.find_asset_id_by_variant(content_hash)? {
                    Some(id) => id,
                    None => {
                        result.errors.push(format!(
                            "{}: no asset found for variant {}",
                            full_path.display(),
                            content_hash
                        ));
                        continue;
                    }
                };

                let uuid: Uuid = asset_id_str.parse()?;
                let mut asset = metadata_store.load(uuid)?;

                reapply_xmp_data(&embedded_xmp, &mut asset, content_hash);

                if !dry_run {
                    metadata_store.save(&asset)?;
                    catalog.insert_asset(&asset)?;
                    if let Some(v) = asset.variants.iter().find(|v| v.content_hash == *content_hash) {
                        catalog.insert_variant(v)?;
                    }
                }

                result.refreshed += 1;
                on_file(&full_path, RefreshStatus::Refreshed, file_start.elapsed());
            }
        }

        Ok(result)
    }

    /// Bidirectional metadata sync: reads external XMP changes (inbound) and writes pending
    /// DAM changes back (outbound). Detects conflicts where both sides changed.
    ///
    /// Phase 1 (Inbound): For each XMP recipe on online volumes, hash the file. If the hash
    /// differs from stored AND the recipe has no pending_writeback, read external changes.
    /// If both changed, report as conflict.
    ///
    /// Phase 2 (Outbound): Write pending DAM metadata to XMP recipes that weren't conflicting.
    ///
    /// Phase 3 (Media, optional): Re-extract embedded XMP from JPEG/TIFF files.
    pub fn sync_metadata(
        &self,
        volume: Option<&Volume>,
        asset_id: Option<&str>,
        dry_run: bool,
        media: bool,
        _exclude_patterns: &[String],
        on_file: impl Fn(&Path, SyncMetadataStatus, Duration),
    ) -> Result<SyncMetadataResult> {
        let content_store = ContentStore::new(&self.catalog_root);
        let metadata_store = MetadataStore::new(&self.catalog_root);
        let catalog = Catalog::open(&self.catalog_root)?;
        let registry = DeviceRegistry::new(&self.catalog_root);
        let all_volumes = registry.list()?;

        let mut result = SyncMetadataResult {
            inbound: 0,
            outbound: 0,
            unchanged: 0,
            skipped: 0,
            conflicts: 0,
            media_refreshed: 0,
            dry_run,
            errors: Vec::new(),
        };

        // Collect XMP recipes from online volumes
        // Each entry: (recipe_id, content_hash, variant_hash, relative_path, pending_writeback, volume)
        struct RecipeEntry<'a> {
            recipe_id: String,
            stored_hash: String,
            variant_hash: String,
            relative_path: String,
            pending: bool,
            vol: &'a Volume,
        }

        let mut recipes: Vec<RecipeEntry> = Vec::new();

        // Determine which volumes to scan
        let target_volumes: Vec<&Volume> = if let Some(v) = volume {
            vec![v]
        } else {
            all_volumes.iter().filter(|v| v.is_online).collect()
        };

        for vol in &target_volumes {
            if !vol.is_online {
                continue;
            }
            let vol_id = vol.id.to_string();
            let entries = catalog.list_recipes_with_pending_for_volume(&vol_id)?;

            for (rid, ch, vh, rp, pending) in entries {
                // Only XMP files participate in metadata sync
                let is_xmp = rp.to_lowercase().ends_with(".xmp");
                if !is_xmp {
                    continue;
                }

                // Filter by asset if requested
                if let Some(aid) = asset_id {
                    // Look up the asset for this recipe's variant
                    if let Ok(Some(recipe_asset_id)) = catalog.find_asset_id_by_variant(&vh) {
                        if !recipe_asset_id.starts_with(aid) {
                            continue;
                        }
                    } else {
                        continue;
                    }
                }

                recipes.push(RecipeEntry {
                    recipe_id: rid,
                    stored_hash: ch,
                    variant_hash: vh,
                    relative_path: rp,
                    pending,
                    vol,
                });
            }
        }

        // Phase 1: Inbound — read external XMP changes; collect pending recipes for Phase 2
        let mut pending_for_writeback: Vec<(String, String, String, String)> = Vec::new();

        for entry in &recipes {
            let file_start = Instant::now();
            let full_path = entry.vol.mount_point.join(&entry.relative_path);

            if !full_path.exists() {
                result.skipped += 1;
                on_file(&full_path, SyncMetadataStatus::Missing, file_start.elapsed());
                continue;
            }

            let new_hash = match content_store.hash_file(&full_path) {
                Ok(h) => h,
                Err(e) => {
                    result.errors.push(format!("{}: {}", full_path.display(), e));
                    on_file(&full_path, SyncMetadataStatus::Error, file_start.elapsed());
                    continue;
                }
            };

            let disk_changed = new_hash != entry.stored_hash;

            match (disk_changed, entry.pending) {
                (false, false) => {
                    // Nothing to do
                    result.unchanged += 1;
                    on_file(&full_path, SyncMetadataStatus::Unchanged, file_start.elapsed());
                }
                (true, false) => {
                    // External change, no pending DAM edits → inbound (refresh)
                    if !dry_run {
                        if let Err(e) = self.apply_modified_recipe(
                            &catalog,
                            &metadata_store,
                            &entry.recipe_id,
                            &new_hash,
                            &entry.variant_hash,
                            entry.vol,
                            &full_path,
                            &entry.relative_path,
                        ) {
                            result.errors.push(format!("{}: {}", full_path.display(), e));
                            on_file(&full_path, SyncMetadataStatus::Error, file_start.elapsed());
                            continue;
                        }
                    }
                    result.inbound += 1;
                    on_file(&full_path, SyncMetadataStatus::Inbound, file_start.elapsed());
                }
                (false, true) => {
                    // No external change, pending DAM edits → outbound (writeback)
                    // Look up asset_id for the writeback process
                    if let Ok(Some(aid)) = catalog.find_asset_id_by_variant(&entry.variant_hash) {
                        pending_for_writeback.push((
                            entry.recipe_id.clone(),
                            aid,
                            entry.vol.id.to_string(),
                            entry.relative_path.clone(),
                        ));
                    }
                    // Don't count here — will be counted by writeback_process
                }
                (true, true) => {
                    // Both sides changed → conflict
                    result.conflicts += 1;
                    on_file(&full_path, SyncMetadataStatus::Conflict, file_start.elapsed());
                }
            }
        }

        // Phase 2: Outbound — write pending DAM metadata via writeback
        if !pending_for_writeback.is_empty() {
            let engine = crate::query::QueryEngine::new(&self.catalog_root);
            let online: HashMap<uuid::Uuid, PathBuf> = all_volumes
                .iter()
                .filter(|v| v.is_online)
                .map(|v| (v.id, v.mount_point.clone()))
                .collect();

            let wb_result = engine.writeback_process(
                pending_for_writeback,
                &catalog,
                &metadata_store,
                &online,
                &content_store,
                None, // no additional asset filter, already filtered above
                None, // no asset ID set filter
                dry_run,
                false, // log handled by our callback
                None,
            )?;

            result.outbound += wb_result.written as usize;
            result.skipped += wb_result.skipped as usize;
            result.errors.extend(wb_result.errors);
        }

        // Phase 3: Media — re-extract embedded XMP from JPEG/TIFF files (same as refresh --media)
        if media {
            let media_entries: Vec<(String, String, String)>;

            if let Some(aid) = asset_id {
                media_entries = catalog.list_file_locations_for_asset(aid)?;
            } else if let Some(vol) = volume {
                let vol_id = vol.id.to_string();
                media_entries = catalog
                    .list_locations_for_volume_under_prefix(&vol_id, "")?
                    .into_iter()
                    .map(|(ch, rp)| (ch, rp, vol_id.clone()))
                    .collect();
            } else {
                let mut entries = Vec::new();
                for vol in &all_volumes {
                    if !vol.is_online {
                        continue;
                    }
                    let vol_id = vol.id.to_string();
                    for (ch, rp) in catalog.list_locations_for_volume_under_prefix(&vol_id, "")? {
                        entries.push((ch, rp, vol_id.clone()));
                    }
                }
                media_entries = entries;
            }

            for (content_hash, relative_path, volume_id_str) in &media_entries {
                let ext = Path::new(relative_path)
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("");
                if !is_embedded_xmp_extension(ext) {
                    continue;
                }

                let vol = match all_volumes.iter().find(|v| v.id.to_string() == *volume_id_str) {
                    Some(v) if v.is_online => v,
                    _ => continue,
                };

                let full_path = vol.mount_point.join(relative_path);
                if !full_path.exists() {
                    continue;
                }

                let embedded_xmp = crate::embedded_xmp::extract_embedded_xmp(&full_path);

                if embedded_xmp.keywords.is_empty()
                    && embedded_xmp.description.is_none()
                    && embedded_xmp.source_metadata.is_empty()
                {
                    continue;
                }

                let asset_id_str = match catalog.find_asset_id_by_variant(content_hash)? {
                    Some(id) => id,
                    None => continue,
                };

                let uuid: Uuid = asset_id_str.parse()?;
                let mut asset = metadata_store.load(uuid)?;

                reapply_xmp_data(&embedded_xmp, &mut asset, content_hash);

                if !dry_run {
                    metadata_store.save(&asset)?;
                    catalog.insert_asset(&asset)?;
                    if let Some(v) = asset.variants.iter().find(|v| v.content_hash == *content_hash) {
                        catalog.insert_variant(v)?;
                    }
                }

                result.media_refreshed += 1;
            }
        }

        Ok(result)
    }

}
