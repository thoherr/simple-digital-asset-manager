//! `cleanup` section of `AssetService` — methods extracted from the original
//! 8.9-kLOC asset_service.rs into a multi-file `impl AssetService` block.
//!
//! Free functions and result types live in the parent `asset_service` module.

use super::*;

impl AssetService {
    // ═══ CLEANUP & DELETE ═══

    /// Scan all file locations and recipes across online volumes, checking for files
    /// that no longer exist on disk. Optionally remove stale records.
    ///
    /// Also scans for orphaned derived files (previews, smart previews, embeddings,
    /// face crops) and removes them.
    pub fn cleanup(
        &self,
        volume_filter: Option<&str>,
        path_prefix: Option<&str>,
        apply: bool,
        on_file: impl Fn(&Path, CleanupStatus, Duration),
    ) -> Result<CleanupResult> {
        let registry = DeviceRegistry::new(&self.catalog_root);
        let catalog = Catalog::open(&self.catalog_root)?;
        let metadata_store = MetadataStore::new(&self.catalog_root);

        let mut result = CleanupResult {
            checked: 0,
            stale: 0,
            removed: 0,
            skipped_offline: 0,
            locationless_variants: 0,
            removed_variants: 0,
            orphaned_assets: 0,
            removed_assets: 0,
            orphaned_previews: 0,
            removed_previews: 0,
            orphaned_smart_previews: 0,
            removed_smart_previews: 0,
            orphaned_embeddings: 0,
            removed_embeddings: 0,
            orphaned_face_files: 0,
            removed_face_files: 0,
            errors: Vec::new(),
            skipped_global_passes: false,
        };

        let volumes = if let Some(label) = volume_filter {
            vec![registry.resolve_volume(label)?]
        } else {
            registry.list()?
        };

        // Collect stale locations for report-mode orphan prediction
        let mut stale_locations: Vec<(String, String, String)> = Vec::new();

        for volume in &volumes {
            if !volume.is_online {
                result.skipped_offline += 1;
                on_file(&volume.mount_point, CleanupStatus::Offline, Duration::ZERO);
                continue;
            }

            let vol_id_str = volume.id.to_string();

            // Check variant file locations
            let prefix = path_prefix.unwrap_or("");
            let locations = catalog.list_locations_for_volume_under_prefix(&vol_id_str, prefix)?;
            for (content_hash, relative_path) in &locations {
                let file_start = Instant::now();
                let full_path = volume.mount_point.join(relative_path);

                if full_path.exists() {
                    result.checked += 1;
                    on_file(&full_path, CleanupStatus::Ok, file_start.elapsed());
                } else {
                    result.stale += 1;
                    on_file(&full_path, CleanupStatus::Stale, file_start.elapsed());
                    if apply {
                        if let Err(e) = catalog.delete_file_location(
                            content_hash,
                            &vol_id_str,
                            relative_path,
                        ) {
                            result.errors.push(format!(
                                "Failed to remove location {}: {e}",
                                relative_path
                            ));
                        } else if let Err(e) = self.remove_sidecar_file_location(
                            &metadata_store,
                            &catalog,
                            content_hash,
                            volume.id,
                            relative_path,
                        ) {
                            result.errors.push(format!(
                                "Failed to update sidecar for {}: {e}",
                                relative_path
                            ));
                        } else {
                            result.removed += 1;
                        }
                    } else {
                        stale_locations.push((
                            content_hash.clone(),
                            vol_id_str.clone(),
                            relative_path.clone(),
                        ));
                    }
                }
            }

            // Check recipe file locations
            let recipes =
                catalog.list_recipes_for_volume_under_prefix(&vol_id_str, prefix)?;
            for (recipe_id, _content_hash, variant_hash, relative_path) in &recipes {
                let file_start = Instant::now();
                let full_path = volume.mount_point.join(relative_path);

                if full_path.exists() {
                    result.checked += 1;
                    on_file(&full_path, CleanupStatus::Ok, file_start.elapsed());
                } else {
                    result.stale += 1;
                    on_file(&full_path, CleanupStatus::Stale, file_start.elapsed());
                    if apply {
                        if let Err(e) = catalog.delete_recipe(recipe_id) {
                            result.errors.push(format!(
                                "Failed to remove recipe {}: {e}",
                                relative_path
                            ));
                        } else if let Err(e) = self.remove_sidecar_recipe(
                            &metadata_store,
                            &catalog,
                            variant_hash,
                            volume.id,
                            relative_path,
                        ) {
                            result.errors.push(format!(
                                "Failed to update sidecar for recipe {}: {e}",
                                relative_path
                            ));
                        } else {
                            result.removed += 1;
                        }
                    }
                }
            }
        }

        // Pass 2: Locationless variants (variant has no locations but asset has other located variants)
        let locationless = if apply {
            catalog.list_locationless_variants()?
        } else {
            catalog.list_would_be_locationless_variants(&stale_locations)?
        };
        result.locationless_variants = locationless.len();

        if apply {
            let preview_gen2 = crate::preview::PreviewGenerator::new(
                &self.catalog_root,
                self.verbosity,
                &self.preview_config,
            );
            // Group by asset_id so we can update sidecars and denormalized columns once per asset
            let mut by_asset: std::collections::HashMap<String, Vec<String>> =
                std::collections::HashMap::new();
            for (asset_id, content_hash) in &locationless {
                by_asset
                    .entry(asset_id.clone())
                    .or_default()
                    .push(content_hash.clone());
            }

            for (asset_id, hashes) in &by_asset {
                for hash in hashes {
                    let file_start = Instant::now();
                    let hash_path = PathBuf::from(hash);

                    // Delete variant from catalog (cascades to file_locations, recipes, embeddings)
                    if let Err(e) = catalog.delete_variant(hash) {
                        result.errors.push(format!(
                            "Failed to delete locationless variant {}: {e}",
                            &hash[..16.min(hash.len())]
                        ));
                        continue;
                    }

                    // Delete derived files
                    let _ = std::fs::remove_file(preview_gen2.preview_path(hash));
                    let _ = std::fs::remove_file(preview_gen2.smart_preview_path(hash));

                    result.removed_variants += 1;
                    on_file(&hash_path, CleanupStatus::LocationlessVariant, file_start.elapsed());
                }

                // Update sidecar: remove the variant(s) from YAML
                if let Ok(uuid) = uuid::Uuid::parse_str(asset_id) {
                    if let Ok(mut asset) = metadata_store.load(uuid) {
                        let hash_set: std::collections::HashSet<&str> =
                            hashes.iter().map(|h| h.as_str()).collect();
                        asset.variants.retain(|v| !hash_set.contains(v.content_hash.as_str()));
                        asset.recipes.retain(|r| !hash_set.contains(r.variant_hash.as_str()));
                        let _ = metadata_store.save(&asset);
                        // Update denormalized columns
                        let _ = catalog.update_denormalized_variant_columns(&asset);
                    }
                }
            }
        }

        // Pass 3: Orphaned assets (all variants have zero file_locations)
        // In apply mode, locations were already removed so we query directly.
        // In report mode, we predict which assets would become orphaned.
        let orphaned_ids = if apply {
            catalog.list_orphaned_asset_ids()?
        } else {
            catalog.list_would_be_orphaned_asset_ids(&stale_locations)?
        };
        result.orphaned_assets = orphaned_ids.len();

        if apply {
            let stack_store = crate::stack::StackStore::new(catalog.conn());
            let preview_gen = crate::preview::PreviewGenerator::new(
                &self.catalog_root,
                self.verbosity,
                &self.preview_config,
            );
            for asset_id in &orphaned_ids {
                let file_start = Instant::now();
                let asset_id_path = PathBuf::from(asset_id);

                // Collect variant hashes and face IDs before deleting DB records
                let variant_hashes: Vec<String> = catalog.conn()
                    .prepare("SELECT content_hash FROM variants WHERE asset_id = ?1")
                    .and_then(|mut s| s.query_map(rusqlite::params![asset_id], |r| r.get(0))
                        .and_then(|rows| rows.collect()))
                    .unwrap_or_default();
                let face_ids: Vec<String> = catalog.conn()
                    .prepare("SELECT id FROM faces WHERE asset_id = ?1")
                    .and_then(|mut s| s.query_map(rusqlite::params![asset_id], |r| r.get(0))
                        .and_then(|rows| rows.collect()))
                    .unwrap_or_default();

                // Remove from stacks, collections, and faces before deleting the asset
                let _ = stack_store.remove(&[asset_id.clone()]);
                let _ = catalog.delete_collection_memberships_for_asset(asset_id);
                let _ = catalog.conn().execute(
                    "DELETE FROM faces WHERE asset_id = ?1",
                    rusqlite::params![asset_id],
                );
                // Delete embedding DB records
                let _ = catalog.conn().execute(
                    "DELETE FROM embeddings WHERE asset_id = ?1",
                    rusqlite::params![asset_id],
                );

                if let Err(e) = catalog.delete_recipes_for_asset(asset_id) {
                    result.errors.push(format!(
                        "Failed to delete recipes for orphaned asset {asset_id}: {e}"
                    ));
                    continue;
                }
                if let Err(e) = catalog.delete_file_locations_for_asset(asset_id) {
                    result.errors.push(format!(
                        "Failed to delete locations for orphaned asset {asset_id}: {e}"
                    ));
                    continue;
                }
                if let Err(e) = catalog.delete_variants_for_asset(asset_id) {
                    result.errors.push(format!(
                        "Failed to delete variants for orphaned asset {asset_id}: {e}"
                    ));
                    continue;
                }
                if let Err(e) = catalog.delete_asset(asset_id) {
                    result.errors.push(format!(
                        "Failed to delete orphaned asset {asset_id}: {e}"
                    ));
                    continue;
                }

                // Delete sidecar YAML
                if let Ok(uuid) = uuid::Uuid::parse_str(asset_id) {
                    if let Err(e) = metadata_store.delete(uuid) {
                        result.errors.push(format!(
                            "Failed to delete sidecar for orphaned asset {asset_id}: {e}"
                        ));
                    }
                }

                // Delete derived files: previews, smart previews, embeddings, face crops
                for hash in &variant_hashes {
                    let _ = std::fs::remove_file(preview_gen.preview_path(hash));
                    let _ = std::fs::remove_file(preview_gen.smart_preview_path(hash));
                }
                for face_id in &face_ids {
                    let prefix = &face_id[..2.min(face_id.len())];
                    let _ = std::fs::remove_file(
                        self.catalog_root.join("faces").join(prefix).join(format!("{face_id}.jpg")),
                    );
                    let _ = std::fs::remove_file(
                        self.catalog_root.join("embeddings").join("arcface").join(prefix).join(format!("{face_id}.bin")),
                    );
                }
                // Delete SigLIP embedding binaries
                for model in &["siglip-vit-b16-256", "siglip-vit-l16-256"] {
                    let prefix = &asset_id[..2.min(asset_id.len())];
                    let _ = std::fs::remove_file(
                        self.catalog_root.join("embeddings").join(model).join(prefix).join(format!("{asset_id}.bin")),
                    );
                }

                result.removed_assets += 1;
                on_file(&asset_id_path, CleanupStatus::OrphanedAsset, file_start.elapsed());
            }
        }

        // Passes 4-7 scan all files under `<catalog_root>/{previews,smart-previews,
        // embeddings,faces}` against the entire catalog — they cannot be meaningfully
        // restricted to a single volume or path prefix, since orphaned derived files
        // live in shared catalog directories, not under a specific volume mount.
        // When the user scoped the run with `--volume` or `--path`, skip the global
        // passes so counts in the summary don't mix path-scoped and catalog-wide numbers.
        let scope_restricted = volume_filter.is_some() || path_prefix.is_some();
        if scope_restricted {
            result.skipped_global_passes = true;
            return Ok(result);
        }

        // Pass 4: Orphaned previews (preview files with no matching variant)
        let variant_hashes = catalog.list_all_variant_hashes()?;
        scan_orphaned_sharded_files(
            &self.catalog_root.join("previews"),
            |stem| {
                let content_hash = format!("sha256:{stem}");
                variant_hashes.contains(&content_hash)
            },
            apply,
            &mut result.orphaned_previews,
            &mut result.removed_previews,
            &mut result.errors,
            &on_file,
        );

        // Pass 5: Orphaned smart previews (same logic, different directory)
        scan_orphaned_sharded_files(
            &self.catalog_root.join("smart-previews"),
            |stem| {
                let content_hash = format!("sha256:{stem}");
                variant_hashes.contains(&content_hash)
            },
            apply,
            &mut result.orphaned_smart_previews,
            &mut result.removed_smart_previews,
            &mut result.errors,
            &on_file,
        );

        // Pass 6: Orphaned embedding binaries (asset_id.bin under embeddings/<model>/)
        let asset_ids_set: HashSet<String> = catalog.list_all_asset_ids()?;
        let emb_base = self.catalog_root.join("embeddings");
        if emb_base.is_dir() {
            if let Ok(model_entries) = std::fs::read_dir(&emb_base) {
                for model_entry in model_entries.flatten() {
                    if !model_entry.path().is_dir() {
                        continue;
                    }
                    let model_name = model_entry.file_name().to_string_lossy().to_string();
                    if model_name == "arcface" {
                        continue; // handled separately in pass 7
                    }
                    scan_orphaned_sharded_files(
                        &model_entry.path(),
                        |stem| asset_ids_set.contains(stem),
                        apply,
                        &mut result.orphaned_embeddings,
                        &mut result.removed_embeddings,
                        &mut result.errors,
                        &on_file,
                    );
                }
            }
        }

        // Pass 7: Orphaned face crop thumbnails (face_id.jpg under faces/)
        let face_ids_set: HashSet<String> = catalog.conn()
            .prepare("SELECT id FROM faces")
            .and_then(|mut s| s.query_map([], |r| r.get(0))
                .and_then(|rows| rows.collect()))
            .unwrap_or_default();
        scan_orphaned_sharded_files(
            &self.catalog_root.join("faces"),
            |stem| face_ids_set.contains(stem),
            apply,
            &mut result.orphaned_face_files,
            &mut result.removed_face_files,
            &mut result.errors,
            &on_file,
        );

        // Pass 7: Orphaned ArcFace embedding binaries (face_id.bin under embeddings/arcface/)
        scan_orphaned_sharded_files(
            &self.catalog_root.join("embeddings").join("arcface"),
            |stem| face_ids_set.contains(stem),
            apply,
            &mut result.orphaned_embeddings,
            &mut result.removed_embeddings,
            &mut result.errors,
            &on_file,
        );

        Ok(result)
    }

    /// Delete assets from the catalog. Report-only by default; `apply` executes deletion.
    /// `remove_files` (requires `apply`) also deletes physical media and recipe files from disk.
    pub fn delete_assets(
        &self,
        asset_ids: &[String],
        apply: bool,
        remove_files: bool,
        on_asset: impl Fn(&str, &DeleteStatus, Duration),
    ) -> Result<DeleteResult> {
        let catalog = Catalog::open(&self.catalog_root)?;
        let metadata_store = MetadataStore::new(&self.catalog_root);
        let registry = DeviceRegistry::new(&self.catalog_root);
        let preview_gen = crate::preview::PreviewGenerator::new(
            &self.catalog_root,
            self.verbosity,
            &self.preview_config,
        );

        // Build volume lookup for file deletion
        let volumes = registry.list().unwrap_or_default();
        let volume_map: std::collections::HashMap<String, &Volume> = volumes
            .iter()
            .map(|v| (v.id.to_string(), v))
            .collect();

        let stack_store = crate::stack::StackStore::new(catalog.conn());
        let mut stacks_changed = false;
        let mut collections_changed = false;

        let mut result = DeleteResult {
            deleted: 0,
            not_found: Vec::new(),
            files_removed: 0,
            previews_removed: 0,
            dry_run: !apply,
            errors: Vec::new(),
        };

        for raw_id in asset_ids {
            let asset_start = Instant::now();

            // 1. Resolve ID (prefix match)
            let asset_id = match catalog.resolve_asset_id(raw_id) {
                Ok(Some(id)) => id,
                Ok(None) => {
                    result.not_found.push(raw_id.clone());
                    on_asset(raw_id, &DeleteStatus::NotFound, asset_start.elapsed());
                    continue;
                }
                Err(e) => {
                    let msg = format!("{raw_id}: {e}");
                    result.errors.push(msg.clone());
                    on_asset(raw_id, &DeleteStatus::Error(msg), asset_start.elapsed());
                    continue;
                }
            };

            // 2. Gather variant hashes (before deleting variants)
            let variant_hashes = catalog.list_variant_hashes_for_asset(&asset_id)
                .unwrap_or_default();

            // 3. Gather file + recipe locations (for --remove-files and report)
            let file_locations = catalog.list_file_locations_for_asset(&asset_id)
                .unwrap_or_default();
            let recipe_locations = catalog.list_recipes_for_asset(&asset_id)
                .unwrap_or_default();

            if apply {
                // 4a. Delete physical files (only if remove_files)
                if remove_files {
                    for (_hash, rel_path, vol_id) in &file_locations {
                        if let Some(vol) = volume_map.get(vol_id.as_str()) {
                            if vol.is_online {
                                let full_path = vol.mount_point.join(rel_path);
                                if full_path.exists() {
                                    if let Err(e) = std::fs::remove_file(&full_path) {
                                        result.errors.push(format!(
                                            "Failed to remove file {}: {e}",
                                            full_path.display()
                                        ));
                                    } else {
                                        result.files_removed += 1;
                                    }
                                }
                            } else {
                                eprintln!(
                                    "  Warning: volume '{}' is offline, skipping file {}",
                                    vol.label, rel_path
                                );
                            }
                        }
                    }
                    for (_recipe_id, _content_hash, _variant_hash, rel_path, vol_id) in &recipe_locations {
                        if let Some(vol) = volume_map.get(vol_id.as_str()) {
                            if vol.is_online {
                                let full_path = vol.mount_point.join(rel_path);
                                if full_path.exists() {
                                    if let Err(e) = std::fs::remove_file(&full_path) {
                                        result.errors.push(format!(
                                            "Failed to remove recipe file {}: {e}",
                                            full_path.display()
                                        ));
                                    } else {
                                        result.files_removed += 1;
                                    }
                                }
                            }
                        }
                    }
                }

                // 4b. Remove from stacks
                if stack_store.remove(&[asset_id.clone()]).unwrap_or(0) > 0 {
                    stacks_changed = true;
                }

                // 4c. Remove collection memberships
                if catalog.delete_collection_memberships_for_asset(&asset_id).unwrap_or(0) > 0 {
                    collections_changed = true;
                }

                // 4c2. Delete faces and their derived files
                let face_ids: Vec<String> = catalog.conn()
                    .prepare("SELECT id FROM faces WHERE asset_id = ?1")
                    .and_then(|mut s| s.query_map(rusqlite::params![&asset_id], |r| r.get(0))
                        .and_then(|rows| rows.collect()))
                    .unwrap_or_default();
                let _ = catalog.conn().execute(
                    "DELETE FROM faces WHERE asset_id = ?1",
                    rusqlite::params![&asset_id],
                );
                for face_id in &face_ids {
                    let prefix = &face_id[..2.min(face_id.len())];
                    let _ = std::fs::remove_file(
                        self.catalog_root.join("faces").join(prefix).join(format!("{face_id}.jpg")),
                    );
                    let _ = std::fs::remove_file(
                        self.catalog_root.join("embeddings").join("arcface").join(prefix).join(format!("{face_id}.bin")),
                    );
                }

                // 4c3. Delete embeddings (DB records + binary files)
                let _ = catalog.conn().execute(
                    "DELETE FROM embeddings WHERE asset_id = ?1",
                    rusqlite::params![&asset_id],
                );
                for model in &["siglip-vit-b16-256", "siglip-vit-l16-256"] {
                    let prefix = &asset_id[..2.min(asset_id.len())];
                    let _ = std::fs::remove_file(
                        self.catalog_root.join("embeddings").join(model).join(prefix).join(format!("{asset_id}.bin")),
                    );
                }

                // 4d. Delete recipes
                if let Err(e) = catalog.delete_recipes_for_asset(&asset_id) {
                    result.errors.push(format!("{asset_id}: failed to delete recipes: {e}"));
                    on_asset(&asset_id, &DeleteStatus::Error(e.to_string()), asset_start.elapsed());
                    continue;
                }

                // 4e. Delete file locations
                if let Err(e) = catalog.delete_file_locations_for_asset(&asset_id) {
                    result.errors.push(format!("{asset_id}: failed to delete locations: {e}"));
                    on_asset(&asset_id, &DeleteStatus::Error(e.to_string()), asset_start.elapsed());
                    continue;
                }

                // 4f. Delete variants
                if let Err(e) = catalog.delete_variants_for_asset(&asset_id) {
                    result.errors.push(format!("{asset_id}: failed to delete variants: {e}"));
                    on_asset(&asset_id, &DeleteStatus::Error(e.to_string()), asset_start.elapsed());
                    continue;
                }

                // 4g. Delete asset
                if let Err(e) = catalog.delete_asset(&asset_id) {
                    result.errors.push(format!("{asset_id}: failed to delete asset: {e}"));
                    on_asset(&asset_id, &DeleteStatus::Error(e.to_string()), asset_start.elapsed());
                    continue;
                }

                // 4h. Delete sidecar YAML
                if let Ok(uuid) = Uuid::parse_str(&asset_id) {
                    if let Err(e) = metadata_store.delete(uuid) {
                        result.errors.push(format!("{asset_id}: failed to delete sidecar: {e}"));
                    }
                }

                // 4i. Delete previews
                for hash in &variant_hashes {
                    let preview_path = preview_gen.preview_path(hash);
                    if preview_path.exists() {
                        if std::fs::remove_file(&preview_path).is_ok() {
                            result.previews_removed += 1;
                        }
                    }
                    let smart_path = preview_gen.smart_preview_path(hash);
                    if smart_path.exists() {
                        if std::fs::remove_file(&smart_path).is_ok() {
                            result.previews_removed += 1;
                        }
                    }
                }

                result.deleted += 1;
                on_asset(&asset_id, &DeleteStatus::Deleted, asset_start.elapsed());
            } else {
                // Report mode: count what would be affected
                result.deleted += 1;
                on_asset(&asset_id, &DeleteStatus::Deleted, asset_start.elapsed());
            }
        }

        // Persist stack/collection changes
        if apply && stacks_changed {
            if let Ok(yaml) = stack_store.export_all() {
                let _ = crate::stack::save_yaml(&self.catalog_root, &yaml);
            }
        }
        if apply && collections_changed {
            let col_store = crate::collection::CollectionStore::new(catalog.conn());
            if let Ok(yaml) = col_store.export_all() {
                let _ = crate::collection::save_yaml(&self.catalog_root, &yaml);
            }
        }

        Ok(result)
    }

}
