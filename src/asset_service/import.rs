//! `import` section of `AssetService` — methods extracted from the original
//! 8.9-kLOC asset_service.rs into a multi-file `impl AssetService` block.
//!
//! Free functions and result types live in the parent `asset_service` module.

use super::*;

impl AssetService {
    // ═══ IMPORT ═══

    /// Import files: hash, deduplicate, create assets/variants, write sidecars, insert into DB.
    pub fn import(
        &self,
        paths: &[PathBuf],
        volume: &Volume,
        filter: &FileTypeFilter,
    ) -> Result<ImportResult> {
        self.import_with_callback(paths, volume, filter, &[], &[], false, false, |_, _, _| {})
    }

    /// Import files with a per-file callback reporting path, status, and elapsed time.
    /// With `dry_run`, reports what would happen without writing to catalog, sidecar, or disk.
    /// With `smart`, generates smart previews (2560px) alongside regular previews.
    pub fn import_with_callback(
        &self,
        paths: &[PathBuf],
        volume: &Volume,
        filter: &FileTypeFilter,
        exclude_patterns: &[String],
        auto_tags: &[String],
        dry_run: bool,
        smart: bool,
        on_file: impl Fn(&Path, FileStatus, Duration),
    ) -> Result<ImportResult> {
        let content_store = ContentStore::new(&self.catalog_root);
        let metadata_store = MetadataStore::new(&self.catalog_root);
        let catalog = Catalog::open(&self.catalog_root)?;
        let preview_gen = crate::preview::PreviewGenerator::new(&self.catalog_root, self.verbosity, &self.preview_config);

        if !dry_run {
            catalog.ensure_volume(volume)?;
        }

        let files = resolve_files(paths, exclude_patterns);
        let groups = group_by_stem(&files, filter);

        if self.verbosity.verbose {
            eprintln!("  Import: {} file(s) resolved, {} group(s)", files.len(), groups.len());
        }

        let mut imported = 0;
        let mut locations_added = 0;
        let mut skipped = 0;
        let mut recipes_attached = 0;
        let mut recipes_location_added = 0;
        let mut recipes_updated = 0;
        let mut previews_generated = 0;
        let mut smart_previews_generated = 0;
        let mut new_asset_ids = Vec::new();
        let mut imported_dirs: std::collections::HashSet<String> = std::collections::HashSet::new();

        for group in &groups {
            // Track the asset created/found for this group's primary variant
            let mut group_asset: Option<Asset> = None;
            let mut primary_variant_hash: Option<String> = None;

            // Pass 1: Process media files (RAW first due to sorting in group_by_stem)
            for file_path in &group.media_files {
                let file_start = Instant::now();

                // Track volume-relative directory for auto-group neighborhood
                if let Ok(rel) = file_path.strip_prefix(&volume.mount_point) {
                    if let Some(parent) = rel.parent() {
                        imported_dirs.insert(parent.to_string_lossy().to_string());
                    }
                }

                let content_hash = content_store
                    .ingest(file_path, volume)
                    .with_context(|| format!("Failed to hash {}", file_path.display()))?;

                if catalog.has_variant(&content_hash)? {
                    // Variant exists — check if we should add a new location
                    let relative_path = file_path
                        .strip_prefix(&volume.mount_point)
                        .with_context(|| {
                            format!(
                                "File {} is not under volume mount point {}",
                                file_path.display(),
                                volume.mount_point.display()
                            )
                        })?;

                    let location = FileLocation {
                        volume_id: volume.id,
                        relative_path: relative_path.to_path_buf(),
                        verified_at: None,
                    };

                    let asset_id = catalog
                        .find_asset_id_by_variant(&content_hash)?
                        .with_context(|| {
                            format!("Variant {} exists but no owning asset found", content_hash)
                        })?;
                    let asset_id: Uuid = asset_id.parse().with_context(|| {
                        format!("Invalid asset UUID: {}", asset_id)
                    })?;
                    let mut asset = metadata_store.load(asset_id)?;

                    // Find the variant and check if this exact location already exists
                    let variant = asset
                        .variants
                        .iter_mut()
                        .find(|v| v.content_hash == content_hash);
                    if let Some(variant) = variant {
                        let already_tracked = variant.locations.iter().any(|l| {
                            l.volume_id == location.volume_id
                                && l.relative_path == location.relative_path
                        });
                        if already_tracked {
                            // Even though skipped, use this as the group asset if needed
                            if group_asset.is_none() {
                                primary_variant_hash = Some(content_hash.clone());
                                group_asset = Some(asset);
                            }
                            skipped += 1;
                            on_file(file_path, FileStatus::Skipped, file_start.elapsed());
                            continue;
                        }
                        if !dry_run {
                            variant.locations.push(location.clone());
                            metadata_store.save(&asset)?;
                            catalog.insert_file_location(&content_hash, &location)?;
                        }
                        if group_asset.is_none() {
                            primary_variant_hash = Some(content_hash.clone());
                            group_asset = Some(asset);
                        }
                        locations_added += 1;
                        on_file(file_path, FileStatus::LocationAdded, file_start.elapsed());
                    } else {
                        if group_asset.is_none() {
                            primary_variant_hash = Some(content_hash.clone());
                            group_asset = Some(asset);
                        }
                        skipped += 1;
                        on_file(file_path, FileStatus::Skipped, file_start.elapsed());
                    }
                    continue;
                }

                // New variant
                let ext = file_path
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("");

                let filename = file_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("unknown")
                    .to_string();

                let file_size = std::fs::metadata(file_path)
                    .with_context(|| {
                        format!("Failed to read metadata for {}", file_path.display())
                    })?
                    .len();

                let relative_path = file_path
                    .strip_prefix(&volume.mount_point)
                    .with_context(|| {
                        format!(
                            "File {} is not under volume mount point {}",
                            file_path.display(),
                            volume.mount_point.display()
                        )
                    })?;

                let location = FileLocation {
                    volume_id: volume.id,
                    relative_path: relative_path.to_path_buf(),
                    verified_at: None,
                };

                if group_asset.is_none() {
                    // First new media file creates the asset
                    let asset_type = determine_asset_type(ext);
                    let mut exif_data = crate::exif_reader::extract(file_path);
                    if asset_type == AssetType::Video {
                        let video_meta = crate::preview::extract_video_metadata(file_path);
                        exif_data.source_metadata.extend(video_meta);
                    }

                    let mut asset = Asset::new(asset_type, &content_hash);
                    // Date fallback chain: EXIF DateTimeOriginal → file mtime → Utc::now()
                    if let Some(date_taken) = exif_data.date_taken {
                        asset.created_at = date_taken;
                    } else if let Some(mtime) = file_mtime(file_path) {
                        asset.created_at = mtime;
                    }
                    asset.name = Some(group.stem.clone());

                    // Apply auto_tags (merge, no duplicates)
                    for tag in auto_tags {
                        if !asset.tags.contains(tag) {
                            asset.tags.push(tag.clone());
                        }
                    }

                    let variant = Variant {
                        content_hash: content_hash.clone(),
                        asset_id: asset.id,
                        role: VariantRole::Original,
                        format: ext.to_lowercase(),
                        file_size,
                        original_filename: filename,
                        source_metadata: exif_data.source_metadata.into_iter().collect(),
                        locations: vec![location.clone()],
                    };

                    asset.variants.push(variant.clone());
                    primary_variant_hash = Some(content_hash.clone());

                    // Extract embedded XMP from JPEG/TIFF
                    let embedded_xmp = crate::embedded_xmp::extract_embedded_xmp(file_path);
                    if !embedded_xmp.keywords.is_empty()
                        || embedded_xmp.description.is_some()
                        || !embedded_xmp.source_metadata.is_empty()
                    {
                        apply_xmp_data(&embedded_xmp, &mut asset, &content_hash);
                    }

                    if !dry_run {
                        // Write sidecar + catalog immediately for first variant
                        metadata_store.save(&asset).with_context(|| {
                            format!("Failed to write sidecar for {}", file_path.display())
                        })?;
                        catalog.insert_asset(&asset)?;
                        catalog.insert_variant(&variant)?;
                        catalog.insert_file_location(&content_hash, &location)?;

                        // Generate preview for the newly imported variant
                        match preview_gen.generate(&content_hash, file_path, ext) {
                            Ok(Some(_)) => previews_generated += 1,
                            Ok(None) => {}
                            Err(e) => eprintln!("  Preview warning: {e:#}"),
                        }
                        if smart {
                            match preview_gen.generate_smart(&content_hash, file_path, ext) {
                                Ok(Some(_)) => smart_previews_generated += 1,
                                Ok(None) => {}
                                Err(e) => eprintln!("  Smart preview warning: {e:#}"),
                            }
                        }
                    }

                    new_asset_ids.push(asset.id.to_string());
                    group_asset = Some(asset);
                } else {
                    // Additional media file → add variant to existing group asset
                    let asset = group_asset.as_mut().expect("group_asset must be Some when processing grouped recipe");
                    let mut exif_data = crate::exif_reader::extract(file_path);
                    if determine_asset_type(ext) == AssetType::Video {
                        let video_meta = crate::preview::extract_video_metadata(file_path);
                        exif_data.source_metadata.extend(video_meta);
                    }

                    // If this variant has an older date, update the asset's created_at
                    let variant_date = exif_data.date_taken.or_else(|| file_mtime(file_path));
                    if let Some(vd) = variant_date {
                        if vd < asset.created_at {
                            asset.created_at = vd;
                        }
                    }

                    // If the primary variant is RAW and this file is not, it's an alternate
                    let primary_is_raw = asset.variants.first()
                        .map(|v| is_raw_extension(&v.format))
                        .unwrap_or(false);
                    let role = if primary_is_raw && !is_raw_extension(ext) {
                        VariantRole::Alternate
                    } else {
                        VariantRole::Original
                    };

                    let variant = Variant {
                        content_hash: content_hash.clone(),
                        asset_id: asset.id,
                        role,
                        format: ext.to_lowercase(),
                        file_size,
                        original_filename: filename,
                        source_metadata: exif_data.source_metadata.into_iter().collect(),
                        locations: vec![location.clone()],
                    };

                    if !dry_run {
                        asset.variants.push(variant.clone());

                        // Extract embedded XMP from JPEG/TIFF
                        let embedded_xmp = crate::embedded_xmp::extract_embedded_xmp(file_path);
                        if !embedded_xmp.keywords.is_empty()
                            || embedded_xmp.description.is_some()
                            || !embedded_xmp.source_metadata.is_empty()
                        {
                            apply_xmp_data(&embedded_xmp, asset, &content_hash);
                        }

                        metadata_store.save(asset).with_context(|| {
                            format!("Failed to write sidecar for {}", file_path.display())
                        })?;
                        catalog.insert_asset(asset)?;
                        catalog.insert_variant(&variant)?;
                        catalog.insert_file_location(&content_hash, &location)?;

                        // Generate preview for the additional variant
                        match preview_gen.generate(&content_hash, file_path, ext) {
                            Ok(Some(_)) => previews_generated += 1,
                            Ok(None) => {}
                            Err(e) => eprintln!("  Preview warning: {e:#}"),
                        }
                        if smart {
                            match preview_gen.generate_smart(&content_hash, file_path, ext) {
                                Ok(Some(_)) => smart_previews_generated += 1,
                                Ok(None) => {}
                                Err(e) => eprintln!("  Smart preview warning: {e:#}"),
                            }
                        }
                    }
                }

                imported += 1;
                on_file(file_path, FileStatus::Imported, file_start.elapsed());
            }

            // Pass 2: Process recipe files
            for file_path in &group.recipe_files {
                let file_start = Instant::now();

                // If no media file was found for this group, treat recipe as standalone media
                if group_asset.is_none() {
                    let content_hash = content_store
                        .ingest(file_path, volume)
                        .with_context(|| format!("Failed to hash {}", file_path.display()))?;

                    if catalog.has_variant(&content_hash)? {
                        // Same dedup logic as media
                        let relative_path = file_path
                            .strip_prefix(&volume.mount_point)
                            .with_context(|| {
                                format!(
                                    "File {} is not under volume mount point {}",
                                    file_path.display(),
                                    volume.mount_point.display()
                                )
                            })?;
                        let location = FileLocation {
                            volume_id: volume.id,
                            relative_path: relative_path.to_path_buf(),
                            verified_at: None,
                        };
                        let asset_id = catalog
                            .find_asset_id_by_variant(&content_hash)?
                            .with_context(|| {
                                format!(
                                    "Variant {} exists but no owning asset found",
                                    content_hash
                                )
                            })?;
                        let asset_id: Uuid = asset_id.parse()?;
                        let mut asset = metadata_store.load(asset_id)?;
                        let variant = asset
                            .variants
                            .iter_mut()
                            .find(|v| v.content_hash == content_hash);
                        if let Some(variant) = variant {
                            let already_tracked = variant.locations.iter().any(|l| {
                                l.volume_id == location.volume_id
                                    && l.relative_path == location.relative_path
                            });
                            if already_tracked {
                                skipped += 1;
                                on_file(file_path, FileStatus::Skipped, file_start.elapsed());
                            } else {
                                if !dry_run {
                                    variant.locations.push(location.clone());
                                    metadata_store.save(&asset)?;
                                    catalog.insert_file_location(&content_hash, &location)?;
                                }
                                locations_added += 1;
                                on_file(
                                    file_path,
                                    FileStatus::LocationAdded,
                                    file_start.elapsed(),
                                );
                            }
                        } else {
                            skipped += 1;
                            on_file(file_path, FileStatus::Skipped, file_start.elapsed());
                        }
                        continue;
                    }

                    // Try to find a parent variant by stem + directory on this volume
                    let ext = file_path
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("");
                    let stem = file_path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("");
                    let relative_path = file_path
                        .strip_prefix(&volume.mount_point)
                        .with_context(|| {
                            format!(
                                "File {} is not under volume mount point {}",
                                file_path.display(),
                                volume.mount_point.display()
                            )
                        })?;
                    let dir_prefix = relative_path
                        .parent()
                        .unwrap_or_else(|| Path::new(""))
                        .to_string_lossy();

                    if let Some((parent_variant_hash, parent_asset_id)) =
                        catalog.find_variant_hash_by_stem_and_directory(
                            stem,
                            &dir_prefix,
                            &volume.id.to_string(),
                            None,
                        )?
                    {
                        // Found parent variant — attach recipe to it
                        let asset_uuid: Uuid = parent_asset_id.parse()?;
                        let mut asset = metadata_store.load(asset_uuid)?;

                        let location = FileLocation {
                            volume_id: volume.id,
                            relative_path: relative_path.to_path_buf(),
                            verified_at: None,
                        };

                        // Location-based dedup on the parent asset
                        let existing_recipe = asset.recipes.iter().find(|r| {
                            r.location.volume_id == volume.id
                                && r.location.relative_path == relative_path
                        });

                        if let Some(existing) = existing_recipe {
                            if existing.content_hash == content_hash {
                                skipped += 1;
                                on_file(file_path, FileStatus::Skipped, file_start.elapsed());
                            } else {
                                if !dry_run {
                                    let recipe_id = existing.id;
                                    let recipe_id_str = recipe_id.to_string();
                                    let recipe_mut = asset.recipes.iter_mut().find(|r| r.id == recipe_id).expect("recipe must exist after location check");
                                    recipe_mut.content_hash = content_hash.clone();
                                    catalog.update_recipe_content_hash(&recipe_id_str, &content_hash)?;
                                    if ext.eq_ignore_ascii_case("xmp") {
                                        let xmp = crate::xmp_reader::extract(file_path);
                                        reapply_xmp_data(&xmp, &mut asset, &parent_variant_hash);
                                        catalog.insert_asset(&asset)?;
                                        if let Some(v) = asset.variants.iter().find(|v| v.content_hash == parent_variant_hash) {
                                            catalog.insert_variant(v)?;
                                        }
                                    }
                                    metadata_store.save(&asset)?;
                                }
                                recipes_updated += 1;
                                on_file(file_path, FileStatus::RecipeUpdated, file_start.elapsed());
                            }
                        } else {
                            // Check if the asset already has a recipe with the same
                            // content hash (on any volume). If so, the XMP content is
                            // identical to one we've already processed — there's nothing
                            // new to merge. Just record the location for backup tracking.
                            // This prevents re-importing old metadata from a backup copy
                            // that was made before tag renames, rating changes, etc.
                            let already_known_content = asset.recipes.iter().any(|r| r.content_hash == content_hash);

                            if !dry_run {
                                // Attach new recipe to parent
                                let recipe = Recipe {
                                    id: Uuid::new_v4(),
                                    variant_hash: parent_variant_hash.clone(),
                                    software: determine_recipe_software(ext).to_string(),
                                    recipe_type: RecipeType::Sidecar,
                                    content_hash: content_hash.clone(),
                                    location,
                                    pending_writeback: false,
                                };
                                asset.recipes.push(recipe.clone());
                                if ext.eq_ignore_ascii_case("xmp") && !already_known_content {
                                    let xmp = crate::xmp_reader::extract(file_path);
                                    apply_xmp_data(&xmp, &mut asset, &parent_variant_hash);
                                    catalog.insert_asset(&asset)?;
                                    if let Some(v) = asset.variants.iter().find(|v| v.content_hash == parent_variant_hash) {
                                        catalog.insert_variant(v)?;
                                    }
                                }
                                metadata_store.save(&asset)?;
                                catalog.insert_recipe(&recipe)?;
                            }
                            if already_known_content {
                                recipes_location_added += 1;
                                on_file(file_path, FileStatus::RecipeLocationAdded, file_start.elapsed());
                            } else {
                                recipes_attached += 1;
                                on_file(file_path, FileStatus::RecipeAttached, file_start.elapsed());
                            }
                        }
                        continue;
                    }

                    // No parent found — import as standalone asset
                    let filename = file_path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("unknown")
                        .to_string();
                    let file_size = std::fs::metadata(file_path)?.len();
                    let location = FileLocation {
                        volume_id: volume.id,
                        relative_path: relative_path.to_path_buf(),
                        verified_at: None,
                    };
                    let mut asset = Asset::new(AssetType::Other, &content_hash);
                    asset.name = Some(filename.clone());
                    for tag in auto_tags {
                        if !asset.tags.contains(tag) {
                            asset.tags.push(tag.clone());
                        }
                    }
                    let variant = Variant {
                        content_hash: content_hash.clone(),
                        asset_id: asset.id,
                        role: VariantRole::Original,
                        format: ext.to_lowercase(),
                        file_size,
                        original_filename: filename,
                        source_metadata: Default::default(),
                        locations: vec![location.clone()],
                    };
                    if !dry_run {
                        asset.variants.push(variant.clone());
                        metadata_store.save(&asset)?;
                        catalog.insert_asset(&asset)?;
                        catalog.insert_variant(&variant)?;
                        catalog.insert_file_location(&content_hash, &location)?;
                    }
                    new_asset_ids.push(asset.id.to_string());
                    imported += 1;
                    on_file(file_path, FileStatus::Imported, file_start.elapsed());
                    continue;
                }

                // Recipe file with a group asset: attach as Recipe
                let content_hash = content_store
                    .ingest(file_path, volume)
                    .with_context(|| format!("Failed to hash {}", file_path.display()))?;

                let ext = file_path
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("");

                let relative_path = file_path
                    .strip_prefix(&volume.mount_point)
                    .with_context(|| {
                        format!(
                            "File {} is not under volume mount point {}",
                            file_path.display(),
                            volume.mount_point.display()
                        )
                    })?;

                let location = FileLocation {
                    volume_id: volume.id,
                    relative_path: relative_path.to_path_buf(),
                    verified_at: None,
                };

                let variant_hash = primary_variant_hash
                    .as_ref()
                    .expect("primary_variant_hash should be set when group_asset is Some");

                let asset = group_asset.as_mut().expect("group_asset must be Some when processing grouped recipe");

                // Location-based recipe dedup: find existing recipe at same location
                let existing_recipe = asset.recipes.iter().find(|r| {
                    r.location.volume_id == volume.id
                        && r.location.relative_path == relative_path
                });

                if let Some(existing) = existing_recipe {
                    if existing.content_hash == content_hash {
                        // Same location, same hash — nothing changed
                        skipped += 1;
                        on_file(file_path, FileStatus::Skipped, file_start.elapsed());
                        continue;
                    }
                    // Same location, different hash — recipe was modified externally
                    if !dry_run {
                        let recipe_id = existing.id;
                        let recipe_id_str = recipe_id.to_string();

                        // Update in-memory
                        let recipe_mut = asset.recipes.iter_mut().find(|r| r.id == recipe_id).expect("recipe must exist after location check");
                        recipe_mut.content_hash = content_hash.clone();

                        // Update catalog
                        catalog.update_recipe_content_hash(&recipe_id_str, &content_hash)?;

                        // Re-extract XMP metadata if applicable
                        if ext.eq_ignore_ascii_case("xmp") {
                            let xmp = crate::xmp_reader::extract(file_path);
                            reapply_xmp_data(&xmp, asset, variant_hash);
                            catalog.insert_asset(asset)?;
                            if let Some(v) = asset.variants.iter().find(|v| v.content_hash == *variant_hash) {
                                catalog.insert_variant(v)?;
                            }
                        }

                        metadata_store.save(asset)?;
                    }
                    recipes_updated += 1;
                    on_file(file_path, FileStatus::RecipeUpdated, file_start.elapsed());
                    continue;
                }

                // Check if the asset already has a recipe with the same content
                // hash (on any volume). If so, skip the metadata merge — the XMP
                // content is identical to one we've already processed.
                let already_known_content = asset.recipes.iter().any(|r| r.content_hash == content_hash);

                if !dry_run {
                    // No existing recipe at this location — attach new recipe
                    let recipe = Recipe {
                        id: Uuid::new_v4(),
                        variant_hash: variant_hash.clone(),
                        software: determine_recipe_software(ext).to_string(),
                        recipe_type: RecipeType::Sidecar,
                        content_hash,
                        location,
                        pending_writeback: false,
                    };

                    asset.recipes.push(recipe.clone());

                    // Extract metadata from XMP sidecars — but only if the content
                    // is genuinely new (different hash from all existing recipes).
                    if ext.eq_ignore_ascii_case("xmp") && !already_known_content {
                        let xmp = crate::xmp_reader::extract(file_path);
                        apply_xmp_data(&xmp, asset, variant_hash);
                        catalog.insert_asset(asset)?;
                        if let Some(v) = asset.variants.iter().find(|v| v.content_hash == *variant_hash) {
                            catalog.insert_variant(v)?;
                        }
                    }

                    metadata_store.save(asset)?;
                    catalog.insert_recipe(&recipe)?;
                }

                if already_known_content {
                    recipes_location_added += 1;
                    on_file(file_path, FileStatus::RecipeLocationAdded, file_start.elapsed());
                } else {
                    recipes_attached += 1;
                    on_file(file_path, FileStatus::RecipeAttached, file_start.elapsed());
                }
            }
        }

        Ok(ImportResult {
            dry_run,
            imported,
            locations_added,
            skipped,
            recipes_attached,
            recipes_location_added,
            recipes_updated,
            previews_generated,
            smart_previews_generated,
            new_asset_ids,
            imported_directories: imported_dirs.into_iter().collect(),
        })
    }

}
