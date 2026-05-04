//! `volume` section of `AssetService` — methods extracted from the original
//! 8.9-kLOC asset_service.rs into a multi-file `impl AssetService` block.
//!
//! Free functions and result types live in the parent `asset_service` module.

use super::*;

impl AssetService {
    // ═══ VOLUME OPERATIONS ═══

    /// Remove a volume and all its associated data (locations, recipes, orphaned assets/previews).
    /// Report-only by default; `--apply` executes removal.
    pub fn remove_volume(
        &self,
        label: &str,
        apply: bool,
        on_file: impl Fn(&Path, CleanupStatus, Duration),
    ) -> Result<VolumeRemoveResult> {
        let registry = DeviceRegistry::new(&self.catalog_root);
        let catalog = Catalog::open(&self.catalog_root)?;
        let metadata_store = MetadataStore::new(&self.catalog_root);

        let volume = registry.resolve_volume(label)?;
        let vol_id_str = volume.id.to_string();

        let mut result = VolumeRemoveResult {
            volume_label: volume.label.clone(),
            volume_id: vol_id_str.clone(),
            locations: 0,
            locations_removed: 0,
            recipes: 0,
            recipes_removed: 0,
            orphaned_assets: 0,
            removed_assets: 0,
            orphaned_previews: 0,
            removed_previews: 0,
            apply,
            errors: Vec::new(),
        };

        // Gather all locations and recipes on this volume
        let locations = catalog.list_locations_for_volume_under_prefix(&vol_id_str, "")?;
        let recipes = catalog.list_recipes_for_volume_under_prefix(&vol_id_str, "")?;
        result.locations = locations.len();
        result.recipes = recipes.len();

        // Build stale list for report-mode orphan prediction
        let stale_locations: Vec<(String, String, String)> = locations
            .iter()
            .map(|(hash, path)| (hash.clone(), vol_id_str.clone(), path.clone()))
            .collect();

        if apply {
            // Remove all file locations on this volume
            for (content_hash, relative_path) in &locations {
                let file_start = Instant::now();
                if let Err(e) = catalog.delete_file_location(
                    content_hash,
                    &vol_id_str,
                    relative_path,
                ) {
                    result.errors.push(format!(
                        "Failed to remove location {}: {e}", relative_path
                    ));
                } else if let Err(e) = self.remove_sidecar_file_location(
                    &metadata_store,
                    &catalog,
                    content_hash,
                    volume.id,
                    relative_path,
                ) {
                    result.errors.push(format!(
                        "Failed to update sidecar for {}: {e}", relative_path
                    ));
                } else {
                    result.locations_removed += 1;
                    on_file(
                        &PathBuf::from(relative_path),
                        CleanupStatus::Stale,
                        file_start.elapsed(),
                    );
                }
            }

            // Remove all recipes on this volume
            for (recipe_id, _content_hash, variant_hash, relative_path) in &recipes {
                let file_start = Instant::now();
                if let Err(e) = catalog.delete_recipe(recipe_id) {
                    result.errors.push(format!(
                        "Failed to remove recipe {}: {e}", relative_path
                    ));
                } else if let Err(e) = self.remove_sidecar_recipe(
                    &metadata_store,
                    &catalog,
                    variant_hash,
                    volume.id,
                    relative_path,
                ) {
                    result.errors.push(format!(
                        "Failed to update sidecar for recipe {}: {e}", relative_path
                    ));
                } else {
                    result.recipes_removed += 1;
                    on_file(
                        &PathBuf::from(relative_path),
                        CleanupStatus::Stale,
                        file_start.elapsed(),
                    );
                }
            }
        }

        // Orphaned assets
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

                // Remove from stacks, collections, faces, and embeddings
                let _ = stack_store.remove(&[asset_id.clone()]);
                let _ = catalog.delete_collection_memberships_for_asset(asset_id);
                let _ = catalog.conn().execute(
                    "DELETE FROM faces WHERE asset_id = ?1",
                    rusqlite::params![asset_id],
                );
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
                if let Ok(uuid) = uuid::Uuid::parse_str(asset_id) {
                    if let Err(e) = metadata_store.delete(uuid) {
                        result.errors.push(format!(
                            "Failed to delete sidecar for orphaned asset {asset_id}: {e}"
                        ));
                    }
                }

                // Delete derived files
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

        // Orphaned previews and smart previews
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
        scan_orphaned_sharded_files(
            &self.catalog_root.join("smart-previews"),
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

        // Finally, remove the volume itself
        if apply {
            if let Err(e) = catalog.delete_volume(&vol_id_str) {
                result.errors.push(format!("Failed to delete volume from catalog: {e}"));
            }
            if let Err(e) = registry.remove(label) {
                result.errors.push(format!("Failed to remove volume from registry: {e}"));
            }
        }

        Ok(result)
    }

    /// Combine a source volume into a target volume, rewriting paths.
    ///
    /// The source must be a subdirectory of the target (same physical disk,
    /// deeper mount point). All file_locations and recipes are moved from source
    /// to target with a computed path prefix. In apply mode, sidecars are
    /// updated first (source of truth), then SQLite bulk update, then the
    /// source volume is removed.
    pub fn combine_volume(
        &self,
        source_label: &str,
        target_label: &str,
        apply: bool,
        on_asset: impl Fn(&str, Duration),
    ) -> Result<VolumeCombineResult> {
        let registry = DeviceRegistry::new(&self.catalog_root);
        let catalog = Catalog::open(&self.catalog_root)?;
        let metadata_store = MetadataStore::new(&self.catalog_root);

        let source = registry.resolve_volume(source_label)?;
        let target = registry.resolve_volume(target_label)?;

        let source_id = source.id.to_string();
        let target_id = target.id.to_string();

        if source.id == target.id {
            bail!(
                "Source and target are the same volume ('{}').",
                source.label
            );
        }

        // Compute path prefix: source mount must be under target mount
        let prefix = source
            .mount_point
            .strip_prefix(&target.mount_point)
            .map_err(|_| {
                anyhow::anyhow!(
                    "Source volume '{}' ({}) is not a subdirectory of target volume '{}' ({}). \
                     Cannot compute path prefix.",
                    source.label,
                    source.mount_point.display(),
                    target.label,
                    target.mount_point.display(),
                )
            })?;

        let prefix_str = if prefix.as_os_str().is_empty() {
            String::new()
        } else {
            let mut p = prefix.to_string_lossy().to_string();
            if !p.ends_with('/') {
                p.push('/');
            }
            p
        };

        // Count locations and recipes
        let locations = catalog.list_locations_for_volume_under_prefix(&source_id, "")?;
        let recipes = catalog.list_recipes_for_volume_under_prefix(&source_id, "")?;
        let asset_ids = catalog.list_asset_ids_on_volume(&source_id)?;

        let mut result = VolumeCombineResult {
            source_label: source.label.clone(),
            source_id: source_id.clone(),
            target_label: target.label.clone(),
            target_id: target_id.clone(),
            path_prefix: prefix_str.clone(),
            locations: locations.len(),
            locations_moved: 0,
            recipes: recipes.len(),
            recipes_moved: 0,
            assets_affected: asset_ids.len(),
            apply,
            errors: Vec::new(),
        };

        if !apply {
            return Ok(result);
        }

        // --- Apply mode ---

        // 1. Update sidecars (source of truth)
        for asset_id_str in &asset_ids {
            let asset_start = Instant::now();
            let uuid = match asset_id_str.parse::<Uuid>() {
                Ok(u) => u,
                Err(e) => {
                    result
                        .errors
                        .push(format!("Invalid asset UUID {asset_id_str}: {e}"));
                    continue;
                }
            };
            match metadata_store.load(uuid) {
                Ok(mut asset) => {
                    let mut changed = false;

                    // Rewrite variant locations
                    for variant in &mut asset.variants {
                        for loc in &mut variant.locations {
                            if loc.volume_id == source.id {
                                loc.volume_id = target.id;
                                let old_path = loc.relative_path.to_string_lossy().to_string();
                                loc.relative_path =
                                    PathBuf::from(format!("{prefix_str}{old_path}"));
                                changed = true;
                            }
                        }
                    }

                    // Rewrite recipe locations
                    for recipe in &mut asset.recipes {
                        if recipe.location.volume_id == source.id {
                            recipe.location.volume_id = target.id;
                            let old_path =
                                recipe.location.relative_path.to_string_lossy().to_string();
                            recipe.location.relative_path =
                                PathBuf::from(format!("{prefix_str}{old_path}"));
                            changed = true;
                        }
                    }

                    if changed {
                        if let Err(e) = metadata_store.save(&asset) {
                            result.errors.push(format!(
                                "Failed to save sidecar for asset {asset_id_str}: {e}"
                            ));
                        }
                    }
                    on_asset(asset_id_str, asset_start.elapsed());
                }
                Err(e) => {
                    result
                        .errors
                        .push(format!("Failed to load sidecar for asset {asset_id_str}: {e}"));
                }
            }
        }

        // 2. Ensure target volume exists in catalog (it may not if nothing was imported onto it)
        catalog.ensure_volume(&target)?;

        // 3. Bulk SQL update
        match catalog.bulk_move_file_locations(&source_id, &target_id, &prefix_str) {
            Ok(n) => result.locations_moved = n,
            Err(e) => result
                .errors
                .push(format!("Failed to move file locations: {e}")),
        }
        match catalog.bulk_move_recipes(&source_id, &target_id, &prefix_str) {
            Ok(n) => result.recipes_moved = n,
            Err(e) => result.errors.push(format!("Failed to move recipes: {e}")),
        }

        // 4. Remove source volume
        if let Err(e) = catalog.delete_volume(&source_id) {
            result
                .errors
                .push(format!("Failed to delete volume from catalog: {e}"));
        }
        if let Err(e) = registry.remove(source_label) {
            result
                .errors
                .push(format!("Failed to remove volume from registry: {e}"));
        }

        Ok(result)
    }

    /// Split a subdirectory from a volume into a new volume.
    ///
    /// The inverse of `combine_volume`: creates a new volume at the source's
    /// mount_point/path, then moves matching file_locations and recipes from
    /// the source to the new volume, stripping the path prefix. The source
    /// volume is NOT removed (it likely has other assets).
    pub fn split_volume(
        &self,
        source_label: &str,
        new_label: &str,
        path: &str,
        purpose: Option<&str>,
        apply: bool,
        on_asset: impl Fn(&str, Duration),
    ) -> Result<VolumeSplitResult> {
        let registry = DeviceRegistry::new(&self.catalog_root);
        let catalog = Catalog::open(&self.catalog_root)?;
        let metadata_store = MetadataStore::new(&self.catalog_root);

        let source = registry.resolve_volume(source_label)?;
        let source_id = source.id.to_string();

        // Normalize prefix: ensure it ends with '/'
        let prefix = if path.ends_with('/') {
            path.to_string()
        } else {
            format!("{path}/")
        };

        // Count matching locations and recipes
        let locations = catalog.list_locations_for_volume_under_prefix(&source_id, &prefix)?;
        let recipes = catalog.list_recipes_for_volume_under_prefix(&source_id, &prefix)?;
        let asset_ids = catalog.list_asset_ids_on_volume_with_prefix(&source_id, &prefix)?;

        // Compute new mount point
        let new_mount = source.mount_point.join(path);

        let mut result = VolumeSplitResult {
            source_label: source.label.clone(),
            source_id: source_id.clone(),
            new_label: new_label.to_string(),
            new_id: String::new(), // filled after registration
            path_prefix: prefix.clone(),
            locations: locations.len(),
            locations_moved: 0,
            recipes: recipes.len(),
            recipes_moved: 0,
            assets_affected: asset_ids.len(),
            apply,
            errors: Vec::new(),
        };

        if !apply {
            return Ok(result);
        }

        // --- Apply mode ---

        // 1. Register the new volume
        use crate::models::volume::{VolumeType, VolumePurpose};
        let vol_purpose = purpose.and_then(VolumePurpose::parse);
        let new_volume = registry.register(new_label, &new_mount, VolumeType::Local, vol_purpose)?;
        let new_id = new_volume.id.to_string();
        result.new_id = new_id.clone();

        // 2. Update sidecars (source of truth)
        for asset_id_str in &asset_ids {
            let asset_start = std::time::Instant::now();
            let uuid = match asset_id_str.parse::<uuid::Uuid>() {
                Ok(u) => u,
                Err(e) => {
                    result.errors.push(format!("Invalid asset UUID {asset_id_str}: {e}"));
                    continue;
                }
            };
            match metadata_store.load(uuid) {
                Ok(mut asset) => {
                    let mut changed = false;

                    for variant in &mut asset.variants {
                        for loc in &mut variant.locations {
                            if loc.volume_id == source.id {
                                let rel = loc.relative_path.to_string_lossy().to_string();
                                if let Some(stripped) = rel.strip_prefix(&prefix) {
                                    loc.volume_id = new_volume.id;
                                    loc.relative_path = std::path::PathBuf::from(stripped);
                                    changed = true;
                                }
                            }
                        }
                    }

                    for recipe in &mut asset.recipes {
                        if recipe.location.volume_id == source.id {
                            let rel = recipe.location.relative_path.to_string_lossy().to_string();
                            if let Some(stripped) = rel.strip_prefix(&prefix) {
                                recipe.location.volume_id = new_volume.id;
                                recipe.location.relative_path = std::path::PathBuf::from(stripped);
                                changed = true;
                            }
                        }
                    }

                    if changed {
                        if let Err(e) = metadata_store.save(&asset) {
                            result.errors.push(format!("Failed to save sidecar for asset {asset_id_str}: {e}"));
                        }
                    }
                    on_asset(asset_id_str, asset_start.elapsed());
                }
                Err(e) => {
                    result.errors.push(format!("Failed to load sidecar for asset {asset_id_str}: {e}"));
                }
            }
        }

        // 3. Ensure new volume exists in catalog
        catalog.ensure_volume(&new_volume)?;

        // 4. Bulk SQL update: move matching locations and strip prefix
        match catalog.bulk_split_file_locations(&source_id, &new_id, &prefix) {
            Ok(n) => result.locations_moved = n,
            Err(e) => result.errors.push(format!("Failed to move file locations: {e}")),
        }
        match catalog.bulk_split_recipes(&source_id, &new_id, &prefix) {
            Ok(n) => result.recipes_moved = n,
            Err(e) => result.errors.push(format!("Failed to move recipes: {e}")),
        }

        Ok(result)
    }

}
