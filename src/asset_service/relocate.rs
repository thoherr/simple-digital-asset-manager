//! `relocate` section of `AssetService` — methods extracted from the original
//! 8.9-kLOC asset_service.rs into a multi-file `impl AssetService` block.
//!
//! Free functions and result types live in the parent `asset_service` module.

use super::*;

impl AssetService {
    // ═══ RELOCATE & UPDATE LOCATION ═══

    /// Relocate all files of an asset to a target volume.
    ///
    /// Copies variant files and recipe files, verifies integrity, updates metadata.
    /// With `remove_source`, deletes source files after successful copy.
    /// With `dry_run`, only reports what would happen.
    pub fn relocate(
        &self,
        asset_id: &str,
        target_volume_label: &str,
        remove_source: bool,
        create_sidecars: bool,
        dry_run: bool,
    ) -> Result<RelocateResult> {
        let catalog = Catalog::open(&self.catalog_root)?;
        let metadata_store = MetadataStore::new(&self.catalog_root);
        let content_store = ContentStore::new(&self.catalog_root);
        let registry = DeviceRegistry::new(&self.catalog_root);

        // Resolve asset
        let full_id = catalog
            .resolve_asset_id(asset_id)?
            .ok_or_else(|| anyhow::anyhow!("no asset found matching '{asset_id}'"))?;
        let asset_uuid: Uuid = full_id.parse()?;
        let asset = metadata_store.load(asset_uuid)?;

        // Resolve target volume
        let target_volume = registry.resolve_volume(target_volume_label)?;
        if !target_volume.mount_point.exists() {
            bail!("target volume '{}' is offline (mount point {} not found)",
                target_volume.label, target_volume.mount_point.display());
        }

        // Get all volumes for resolving source paths
        let volumes = registry.list()?;
        let find_volume = |vol_id: Uuid| -> Option<Volume> {
            volumes.iter().find(|v| v.id == vol_id).cloned()
        };

        // Build copy plan
        let mut plan: Vec<FileCopyPlan> = Vec::new();

        // Plan variant file copies
        for variant in &asset.variants {
            for loc in &variant.locations {
                if loc.volume_id == target_volume.id {
                    continue; // Already on target
                }
                let source_vol = find_volume(loc.volume_id)
                    .ok_or_else(|| anyhow::anyhow!(
                        "Source volume {} not found in registry", loc.volume_id
                    ))?;
                if !source_vol.mount_point.exists() {
                    bail!("source volume '{}' is offline (mount point {} not found)",
                        source_vol.label, source_vol.mount_point.display());
                }

                let source_path = source_vol.mount_point.join(&loc.relative_path);
                let target_path = target_volume.mount_point.join(&loc.relative_path);

                plan.push(FileCopyPlan {
                    content_hash: variant.content_hash.clone(),
                    source_path,
                    target_path,
                    kind: FileCopyKind::Variant,
                    source_volume_id: loc.volume_id,
                    source_relative_path: loc.relative_path.clone(),
                });
            }
        }

        // Plan recipe file copies
        for recipe in &asset.recipes {
            if recipe.location.volume_id == target_volume.id {
                continue; // Already on target
            }
            let source_vol = find_volume(recipe.location.volume_id)
                .ok_or_else(|| anyhow::anyhow!(
                    "Source volume {} not found in registry", recipe.location.volume_id
                ))?;
            if !source_vol.mount_point.exists() {
                bail!("source volume '{}' is offline (mount point {} not found)",
                    source_vol.label, source_vol.mount_point.display());
            }

            let source_path = source_vol.mount_point.join(&recipe.location.relative_path);
            let target_path = target_volume.mount_point.join(&recipe.location.relative_path);

            plan.push(FileCopyPlan {
                content_hash: recipe.content_hash.clone(),
                source_path,
                target_path,
                kind: FileCopyKind::Recipe { recipe_id: recipe.id },
                source_volume_id: recipe.location.volume_id,
                source_relative_path: recipe.location.relative_path.clone(),
            });
        }

        // Early return if nothing to do
        if plan.is_empty() {
            return Ok(RelocateResult {
                copied: 0,
                skipped: 0,
                removed: 0,
                actions: vec!["All files already on target volume".to_string()],
            });
        }

        // Dry run: report what would happen
        if dry_run {
            let mut actions = Vec::new();
            let mut would_copy = 0usize;
            let mut would_skip = 0usize;

            for entry in &plan {
                if entry.target_path.exists() {
                    let existing_hash = content_store.hash_file(&entry.target_path)?;
                    if existing_hash == entry.content_hash {
                        actions.push(format!(
                            "SKIP {} (already exists with matching hash)",
                            entry.source_relative_path.display()
                        ));
                        would_skip += 1;
                        continue;
                    }
                }
                let verb = if remove_source { "MOVE" } else { "COPY" };
                actions.push(format!(
                    "{} {} -> {}",
                    verb,
                    entry.source_path.display(),
                    entry.target_path.display()
                ));
                would_copy += 1;
            }

            return Ok(RelocateResult {
                copied: would_copy,
                skipped: would_skip,
                removed: if remove_source { would_copy } else { 0 },
                actions,
            });
        }

        // Phase 1: Copy all files (no metadata changes yet)
        let mut copied = 0usize;
        let mut skipped = 0usize;
        let mut actions = Vec::new();

        for entry in &plan {
            if entry.target_path.exists() {
                let existing_hash = content_store.hash_file(&entry.target_path)?;
                if existing_hash == entry.content_hash {
                    actions.push(format!(
                        "Skipped {} (already exists on target)",
                        entry.source_relative_path.display()
                    ));
                    skipped += 1;
                    continue;
                }
            }
            content_store
                .copy_and_verify(&entry.source_path, &entry.target_path, &entry.content_hash)
                .with_context(|| format!(
                    "Failed to copy {} to {}",
                    entry.source_path.display(),
                    entry.target_path.display()
                ))?;
            actions.push(format!(
                "Copied {} -> {}",
                entry.source_relative_path.display(),
                target_volume.label
            ));
            copied += 1;
        }

        // Phase 2: Update metadata
        let mut asset = metadata_store.load(asset_uuid)?;
        catalog.ensure_volume(&target_volume)?;

        for entry in &plan {
            match &entry.kind {
                FileCopyKind::Variant => {
                    // Add new location to the variant
                    let new_loc = FileLocation {
                        volume_id: target_volume.id,
                        relative_path: entry.source_relative_path.clone(),
                        verified_at: None,
                    };
                    if let Some(variant) = asset.variants.iter_mut().find(|v| v.content_hash == entry.content_hash) {
                        let already_has = variant.locations.iter().any(|l| {
                            l.volume_id == target_volume.id
                                && l.relative_path == entry.source_relative_path
                        });
                        if !already_has {
                            variant.locations.push(new_loc.clone());
                            catalog.insert_file_location(&entry.content_hash, &new_loc)?;
                        }
                    }
                }
                FileCopyKind::Recipe { recipe_id } => {
                    // Update recipe location to target volume
                    if let Some(recipe) = asset.recipes.iter_mut().find(|r| r.id == *recipe_id) {
                        recipe.location.volume_id = target_volume.id;
                        recipe.location.relative_path = entry.source_relative_path.clone();
                    }
                    catalog.update_recipe_location(
                        &recipe_id.to_string(),
                        &target_volume.id.to_string(),
                        &entry.source_relative_path.to_string_lossy(),
                    )?;
                }
            }
        }

        // Phase 2.5: Create XMP sidecars for variants without recipes (if requested)
        if create_sidecars {
            let has_metadata = !asset.tags.is_empty()
                || asset.rating.is_some()
                || asset.color_label.is_some()
                || asset.description.is_some();

            if has_metadata {
                for variant in &asset.variants {
                    // Skip if variant already has an XMP recipe on the target volume
                    let has_xmp_on_target = asset.recipes.iter().any(|r| {
                        r.variant_hash == variant.content_hash
                            && r.location.volume_id == target_volume.id
                            && r.recipe_type == crate::models::recipe::RecipeType::Sidecar
                    });
                    if has_xmp_on_target {
                        continue;
                    }

                    // Check if variant has a location on the target volume
                    let target_loc = match variant.locations.iter().find(|l| l.volume_id == target_volume.id) {
                        Some(loc) => loc,
                        None => continue,
                    };

                    // Build XMP sidecar path: variant filename + .xmp extension
                    let variant_relative = &target_loc.relative_path;
                    let xmp_relative = variant_relative.with_extension(
                        format!("{}.xmp", variant_relative.extension().unwrap_or_default().to_string_lossy())
                    );
                    let xmp_path = target_volume.mount_point.join(&xmp_relative);

                    if let Some(parent) = xmp_path.parent() {
                        std::fs::create_dir_all(parent)?;
                    }

                    let xmp_content = crate::xmp_reader::create_xmp(
                        &asset.tags,
                        asset.rating,
                        asset.color_label.as_deref(),
                        asset.description.as_deref(),
                    );

                    std::fs::write(&xmp_path, &xmp_content)?;
                    let xmp_hash = content_store.hash_file(&xmp_path)?;

                    let recipe = crate::models::recipe::Recipe {
                        id: Uuid::new_v4(),
                        variant_hash: variant.content_hash.clone(),
                        software: "MAKI".to_string(),
                        recipe_type: crate::models::recipe::RecipeType::Sidecar,
                        content_hash: xmp_hash,
                        location: crate::models::FileLocation {
                            volume_id: target_volume.id,
                            relative_path: xmp_relative.clone(),
                            verified_at: None,
                        },
                        pending_writeback: false,
                    };
                    catalog.insert_recipe(&recipe)?;
                    asset.recipes.push(recipe);

                    actions.push(format!("Created sidecar {}", xmp_relative.display()));
                    copied += 1;
                }
            }
        }

        metadata_store.save(&asset)?;

        // Phase 3: Remove sources (only if --remove-source)
        let mut removed = 0usize;
        if remove_source {
            for entry in &plan {
                // Delete source file
                if entry.source_path.exists() {
                    std::fs::remove_file(&entry.source_path)
                        .with_context(|| format!(
                            "Failed to remove source file {}",
                            entry.source_path.display()
                        ))?;
                }

                match &entry.kind {
                    FileCopyKind::Variant => {
                        // Remove old location from variant
                        if let Some(variant) = asset.variants.iter_mut().find(|v| v.content_hash == entry.content_hash) {
                            variant.locations.retain(|l| {
                                !(l.volume_id == entry.source_volume_id
                                    && l.relative_path == entry.source_relative_path)
                            });
                        }
                        catalog.delete_file_location(
                            &entry.content_hash,
                            &entry.source_volume_id.to_string(),
                            &entry.source_relative_path.to_string_lossy(),
                        )?;
                    }
                    FileCopyKind::Recipe { .. } => {
                        // Recipe location already updated to target in Phase 2
                    }
                }

                removed += 1;
            }

            // Save again after removals
            metadata_store.save(&asset)?;

            // Update action messages
            actions = actions
                .into_iter()
                .map(|a| a.replace("Copied", "Moved"))
                .collect();
        }

        Ok(RelocateResult {
            copied,
            skipped,
            removed,
            actions,
        })
    }

    /// Update a file's location in the catalog after it was moved on disk.
    ///
    /// Looks up the old path as a variant file location or recipe, verifies the
    /// file at `to_path` has the same content hash, and updates catalog + sidecar.
    pub fn update_location(
        &self,
        asset_id: &str,
        from_path: &str,
        to_path: &Path,
        volume_label: Option<&str>,
    ) -> Result<UpdateLocationResult> {
        let catalog = Catalog::open(&self.catalog_root)?;
        let metadata_store = MetadataStore::new(&self.catalog_root);
        let content_store = ContentStore::new(&self.catalog_root);
        let registry = DeviceRegistry::new(&self.catalog_root);

        // Resolve volume from --to path or explicit --volume
        let volume = if let Some(label) = volume_label {
            registry.resolve_volume(label)?
        } else {
            registry.find_volume_for_path(to_path)?
        };
        let volume_id_str = volume.id.to_string();

        // Convert to_path to volume-relative
        let new_relative = to_path
            .strip_prefix(&volume.mount_point)
            .with_context(|| {
                format!(
                    "Path '{}' is not under volume '{}' ({})",
                    to_path.display(),
                    volume.label,
                    volume.mount_point.display()
                )
            })?;
        let new_relative_str = new_relative.to_string_lossy().replace('\\', "/");

        // Convert from_path to volume-relative (strip mount point if absolute)
        let from = Path::new(from_path);
        let old_relative_str = if from.is_absolute() {
            from.strip_prefix(&volume.mount_point)
                .with_context(|| {
                    format!(
                        "Path '{}' is not under volume '{}' ({})",
                        from_path,
                        volume.label,
                        volume.mount_point.display()
                    )
                })?
                .to_string_lossy()
                .replace('\\', "/")
        } else {
            from_path.to_string()
        };

        // Resolve asset ID
        let full_id = catalog
            .resolve_asset_id(asset_id)?
            .ok_or_else(|| anyhow::anyhow!("no asset found matching '{asset_id}'"))?;

        // Try as variant file location first, then recipe
        if let Some((content_hash, _format)) =
            catalog.find_variant_by_volume_and_path(&volume_id_str, &old_relative_str)?
        {
            // Verify this variant belongs to the resolved asset
            let variant_asset_id = catalog.find_asset_id_by_variant(&content_hash)?;
            if variant_asset_id.as_deref() != Some(&full_id) {
                bail!(
                    "File at '{}' belongs to asset {}, not {}",
                    old_relative_str,
                    variant_asset_id.unwrap_or_else(|| "(unknown)".to_string()),
                    &full_id[..8]
                );
            }

            // Verify file exists at new path
            if !to_path.exists() {
                bail!("file not found at '{}'", to_path.display());
            }

            // Verify content hash matches
            let actual_hash = content_store.hash_file(to_path)?;
            if actual_hash != content_hash {
                bail!(
                    "Hash mismatch: file at '{}' has hash {} but catalog expects {}",
                    to_path.display(),
                    &actual_hash[..16],
                    &content_hash[..16]
                );
            }

            // Update catalog
            catalog.update_file_location_path(
                &content_hash,
                &volume_id_str,
                &old_relative_str,
                &new_relative_str,
            )?;

            // Update sidecar
            self.update_sidecar_file_location_path(
                &metadata_store,
                &catalog,
                &content_hash,
                volume.id,
                &old_relative_str,
                &new_relative_str,
            )?;

            Ok(UpdateLocationResult {
                asset_id: full_id,
                file_type: "variant".to_string(),
                content_hash,
                old_path: old_relative_str,
                new_path: new_relative_str,
                volume_label: volume.label,
            })
        } else if let Some((recipe_id, content_hash, variant_hash)) =
            catalog.find_recipe_by_volume_and_path(&volume_id_str, &old_relative_str)?
        {
            // Verify the recipe's variant belongs to the resolved asset
            let variant_asset_id = catalog.find_asset_id_by_variant(&variant_hash)?;
            if variant_asset_id.as_deref() != Some(&full_id) {
                bail!(
                    "Recipe at '{}' belongs to asset {}, not {}",
                    old_relative_str,
                    variant_asset_id.unwrap_or_else(|| "(unknown)".to_string()),
                    &full_id[..8]
                );
            }

            // Verify file exists at new path
            if !to_path.exists() {
                bail!("file not found at '{}'", to_path.display());
            }

            // Verify content hash matches
            let actual_hash = content_store.hash_file(to_path)?;
            if actual_hash != content_hash {
                bail!(
                    "Hash mismatch: file at '{}' has hash {} but catalog expects {}",
                    to_path.display(),
                    &actual_hash[..16],
                    &content_hash[..16]
                );
            }

            // Update catalog
            catalog.update_recipe_relative_path(&recipe_id, &new_relative_str)?;

            // Update sidecar
            self.update_sidecar_recipe_path(
                &metadata_store,
                &catalog,
                &variant_hash,
                volume.id,
                &old_relative_str,
                &new_relative_str,
            )?;

            Ok(UpdateLocationResult {
                asset_id: full_id,
                file_type: "recipe".to_string(),
                content_hash,
                old_path: old_relative_str,
                new_path: new_relative_str,
                volume_label: volume.label,
            })
        } else {
            bail!(
                "No variant or recipe found at '{}' on volume '{}'",
                old_relative_str,
                volume.label
            );
        }
    }

}
