//! `sync` section of `AssetService` — methods extracted from the original
//! 8.9-kLOC asset_service.rs into a multi-file `impl AssetService` block.
//!
//! Free functions and result types live in the parent `asset_service` module.

use super::*;

impl AssetService {
    // ═══ SYNC ═══

    /// Scan directories and reconcile the catalog with disk reality.
    ///
    /// Detects moved files, new files, modified recipes, and missing files.
    /// Without `apply`, runs in report-only mode. With `apply`, updates the catalog
    /// and sidecar files. `remove_stale` (requires `apply`) removes catalog locations
    /// for confirmed-missing files.
    pub fn sync(
        &self,
        paths: &[PathBuf],
        volume: &Volume,
        apply: bool,
        remove_stale: bool,
        exclude_patterns: &[String],
        on_file: impl Fn(&Path, SyncStatus, Duration),
    ) -> Result<SyncResult> {
        use std::collections::{HashMap, HashSet};

        let content_store = ContentStore::new(&self.catalog_root);
        let metadata_store = MetadataStore::new(&self.catalog_root);
        let catalog = Catalog::open(&self.catalog_root)?;
        let filter = FileTypeFilter::default();

        let mut result = SyncResult {
            unchanged: 0,
            moved: 0,
            new_files: 0,
            modified: 0,
            missing: 0,
            stale_removed: 0,
            orphaned_cleaned: 0,
            locationless_after: 0,
            errors: Vec::new(),
        };

        let vol_id = volume.id.to_string();

        // Collect all files on disk
        let files = resolve_files(paths, exclude_patterns);

        // Track paths seen on disk (relative to volume mount)
        let mut disk_media_paths: HashSet<String> = HashSet::new();
        let mut disk_recipe_paths: HashSet<String> = HashSet::new();

        // Maps for move detection: content_hash -> new_relative_path
        let mut media_hash_to_new_path: HashMap<String, (String, PathBuf)> = HashMap::new();
        // recipe: content_hash -> (new_relative_path, full_path)
        let mut recipe_hash_to_new_path: HashMap<String, (String, PathBuf)> = HashMap::new();

        // ── Pass 1: Scan disk files ──────────────────────────────────
        for file_path in &files {
            let file_start = Instant::now();

            let ext = file_path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("");

            // Skip files not in any known type group
            if !ext.is_empty() && !filter.is_importable(ext) {
                continue;
            }

            let relative_path = match file_path.strip_prefix(&volume.mount_point) {
                Ok(rp) => rp.to_string_lossy().to_string(),
                Err(_) => {
                    result.errors.push(format!(
                        "File {} is not under volume mount point {}",
                        file_path.display(),
                        volume.mount_point.display()
                    ));
                    continue;
                }
            };

            let hash = match content_store.hash_file(file_path) {
                Ok(h) => h,
                Err(e) => {
                    result.errors.push(format!("{}: {}", file_path.display(), e));
                    continue;
                }
            };

            let is_recipe = filter.is_recipe(ext);

            if is_recipe {
                disk_recipe_paths.insert(relative_path.clone());

                // Look up recipe by location
                match catalog.find_recipe_by_volume_and_path(&vol_id, &relative_path)? {
                    Some((_recipe_id, stored_hash, _variant_hash)) => {
                        if stored_hash == hash {
                            // Unchanged recipe
                            result.unchanged += 1;
                            on_file(file_path, SyncStatus::Unchanged, file_start.elapsed());
                        } else {
                            // Modified recipe (content changed at same path)
                            result.modified += 1;
                            if apply {
                                self.apply_modified_recipe(
                                    &catalog,
                                    &metadata_store,
                                    &_recipe_id,
                                    &hash,
                                    &_variant_hash,
                                    volume,
                                    file_path,
                                    &relative_path,
                                )?;
                            }
                            on_file(file_path, SyncStatus::Modified, file_start.elapsed());
                        }
                    }
                    None => {
                        // Not at this location — could be moved or new
                        if catalog.has_recipe_by_content_hash(&hash)? {
                            // Known hash at different location → potential move
                            recipe_hash_to_new_path.insert(
                                hash,
                                (relative_path, file_path.clone()),
                            );
                        } else {
                            // Completely new recipe file
                            result.new_files += 1;
                            on_file(file_path, SyncStatus::New, file_start.elapsed());
                        }
                    }
                }
            } else {
                disk_media_paths.insert(relative_path.clone());

                // Look up media file by location
                match catalog.find_variant_by_volume_and_path(&vol_id, &relative_path)? {
                    Some((stored_hash, _format)) => {
                        if stored_hash == hash {
                            // Unchanged — optionally update verified_at
                            result.unchanged += 1;
                            if apply {
                                catalog.update_verified_at(&hash, &vol_id, &relative_path)?;
                            }
                            on_file(file_path, SyncStatus::Unchanged, file_start.elapsed());
                        } else {
                            // Content-addressed file changed — this shouldn't happen
                            result.errors.push(format!(
                                "Hash mismatch at {}: expected {}, got {}",
                                relative_path, stored_hash, hash
                            ));
                        }
                    }
                    None => {
                        // Not at this location — could be moved or new
                        if catalog.has_variant(&hash)? {
                            // Known hash at different location → potential move
                            media_hash_to_new_path.insert(
                                hash,
                                (relative_path, file_path.clone()),
                            );
                        } else {
                            // Completely new file
                            result.new_files += 1;
                            on_file(file_path, SyncStatus::New, file_start.elapsed());
                        }
                    }
                }
            }
        }

        // ── Pass 2: Detect missing/moved ─────────────────────────────
        // Compute directory prefixes from scanned paths
        let prefixes = compute_prefixes(paths, &volume.mount_point);

        // Check media file locations
        for prefix in &prefixes {
            let catalog_locations =
                catalog.list_locations_for_volume_under_prefix(&vol_id, prefix)?;

            for (content_hash, cat_path) in &catalog_locations {
                if disk_media_paths.contains(cat_path.as_str()) {
                    continue; // Already handled in Pass 1
                }

                let file_start = Instant::now();

                if let Some((new_path, full_path)) = media_hash_to_new_path.remove(content_hash) {
                    // File was moved
                    result.moved += 1;
                    if apply {
                        catalog.update_file_location_path(
                            content_hash,
                            &vol_id,
                            cat_path,
                            &new_path,
                        )?;
                        // Update sidecar
                        self.update_sidecar_file_location_path(
                            &metadata_store,
                            &catalog,
                            content_hash,
                            volume.id,
                            cat_path,
                            &new_path,
                        )?;
                    }
                    on_file(&full_path, SyncStatus::Moved, file_start.elapsed());
                } else {
                    // File is missing from disk
                    result.missing += 1;
                    let full_path = volume.mount_point.join(cat_path);
                    if apply && remove_stale {
                        catalog.delete_file_location(content_hash, &vol_id, cat_path)?;
                        self.remove_sidecar_file_location(
                            &metadata_store,
                            &catalog,
                            content_hash,
                            volume.id,
                            cat_path,
                        )?;
                        result.stale_removed += 1;
                    }
                    on_file(&full_path, SyncStatus::Missing, file_start.elapsed());
                }
            }
        }

        // Check recipe locations
        for prefix in &prefixes {
            let catalog_recipes =
                catalog.list_recipes_for_volume_under_prefix(&vol_id, prefix)?;

            for (recipe_id, content_hash, variant_hash, cat_path) in &catalog_recipes {
                if disk_recipe_paths.contains(cat_path.as_str()) {
                    continue; // Already handled in Pass 1
                }

                let file_start = Instant::now();

                if let Some((new_path, full_path)) = recipe_hash_to_new_path.remove(&*content_hash) {
                    // Recipe was moved
                    result.moved += 1;
                    if apply {
                        catalog.update_recipe_relative_path(recipe_id, &new_path)?;
                        // Update sidecar
                        self.update_sidecar_recipe_path(
                            &metadata_store,
                            &catalog,
                            variant_hash,
                            volume.id,
                            cat_path,
                            &new_path,
                        )?;
                    }
                    on_file(&full_path, SyncStatus::Moved, file_start.elapsed());
                } else {
                    // Recipe is missing from disk
                    result.missing += 1;
                    let full_path = volume.mount_point.join(cat_path);
                    if apply && remove_stale {
                        if let Err(e) = catalog.delete_recipe(recipe_id) {
                            result.errors.push(format!(
                                "Failed to delete stale recipe {cat_path}: {e}"
                            ));
                        } else if let Err(e) = self.remove_sidecar_recipe(
                            &metadata_store,
                            &catalog,
                            variant_hash,
                            volume.id,
                            cat_path,
                        ) {
                            result.errors.push(format!(
                                "Failed to remove recipe from sidecar for {cat_path}: {e}"
                            ));
                        } else {
                            result.stale_removed += 1;
                        }
                    }
                    on_file(&full_path, SyncStatus::Missing, file_start.elapsed());
                }
            }
        }

        // Any remaining entries in hash_to_new_path are files that matched a hash
        // but whose old location wasn't in our scanned prefixes — report as new
        for (_hash, (_path, full_path)) in &media_hash_to_new_path {
            let file_start = Instant::now();
            result.new_files += 1;
            on_file(full_path, SyncStatus::New, file_start.elapsed());
        }
        for (_hash, (_path, full_path)) in &recipe_hash_to_new_path {
            let file_start = Instant::now();
            result.new_files += 1;
            on_file(full_path, SyncStatus::New, file_start.elapsed());
        }

        // Clean up assets that became locationless after stale removal
        if apply && remove_stale && result.stale_removed > 0 {
            let orphaned = catalog.list_orphaned_asset_ids()?;
            for asset_id in &orphaned {
                if let Ok(uuid) = asset_id.parse::<uuid::Uuid>() {
                    // Delete sidecar
                    let _ = metadata_store.delete(uuid);
                    // Delete from catalog (variants, recipes, etc.)
                    let _ = catalog.delete_recipes_for_asset(asset_id);
                    let _ = catalog.delete_asset(asset_id);
                    result.orphaned_cleaned += 1;
                }
            }
        }

        // After sync: count variants that have lost all their locations but
        // whose asset still has other variants (so it wasn't removed above).
        // These linger in the catalog — including, often, as the *selected*
        // best variant for preview — until `maki cleanup --apply` removes
        // them. The CLI uses this count to surface a next-step hint.
        //
        // Always computed (apply or dry-run) so dry-runs can hint about
        // pre-existing locationless variants too.
        if let Ok(locationless) = catalog.list_locationless_variants() {
            result.locationless_after = locationless.len();
        }

        Ok(result)
    }

    /// Apply a modified recipe: update catalog hash, re-extract XMP if applicable, update sidecar.
    pub(super) fn apply_modified_recipe(
        &self,
        catalog: &Catalog,
        metadata_store: &MetadataStore,
        recipe_id: &str,
        new_hash: &str,
        variant_hash: &str,
        volume: &Volume,
        file_path: &Path,
        relative_path: &str,
    ) -> Result<()> {
        catalog.update_recipe_content_hash(recipe_id, new_hash)?;

        if let Some(asset_id_str) = catalog.find_asset_id_by_variant(variant_hash)? {
            let asset_uuid: Uuid = asset_id_str.parse()?;
            let mut asset = metadata_store.load(asset_uuid)?;
            if let Some(recipe) = asset.recipes.iter_mut().find(|r| {
                r.location.volume_id == volume.id
                    && r.location.relative_path.to_string_lossy() == relative_path
            }) {
                recipe.content_hash = new_hash.to_string();

                let ext = file_path
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("");
                if ext.eq_ignore_ascii_case("xmp") {
                    let xmp = crate::xmp_reader::extract(file_path);
                    reapply_xmp_data(&xmp, &mut asset, variant_hash);
                    catalog.insert_asset(&asset)?;
                    if let Some(v) = asset.variants.iter().find(|v| v.content_hash == variant_hash)
                    {
                        catalog.insert_variant(v)?;
                    }
                }

                metadata_store.save(&asset)?;
            }
        }
        Ok(())
    }

    /// Update a file location's relative_path in the sidecar YAML.
    pub(super) fn update_sidecar_file_location_path(
        &self,
        metadata_store: &MetadataStore,
        catalog: &Catalog,
        content_hash: &str,
        volume_id: Uuid,
        old_path: &str,
        new_path: &str,
    ) -> Result<()> {
        let asset_id = match catalog.find_asset_id_by_variant(content_hash)? {
            Some(id) => id,
            None => return Ok(()),
        };
        let uuid: Uuid = asset_id.parse()?;
        let mut asset = metadata_store.load(uuid)?;

        let mut changed = false;
        for variant in &mut asset.variants {
            if variant.content_hash == content_hash {
                for loc in &mut variant.locations {
                    if loc.volume_id == volume_id
                        && loc.relative_path.to_string_lossy() == old_path
                    {
                        loc.relative_path = PathBuf::from(new_path);
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

    /// Remove a file location from the sidecar YAML.
    pub fn remove_sidecar_file_location(
        &self,
        metadata_store: &MetadataStore,
        catalog: &Catalog,
        content_hash: &str,
        volume_id: Uuid,
        relative_path: &str,
    ) -> Result<()> {
        let asset_id = match catalog.find_asset_id_by_variant(content_hash)? {
            Some(id) => id,
            None => return Ok(()),
        };
        let uuid: Uuid = asset_id.parse()?;
        let mut asset = metadata_store.load(uuid)?;

        let mut changed = false;
        for variant in &mut asset.variants {
            if variant.content_hash == content_hash {
                let before = variant.locations.len();
                variant.locations.retain(|loc| {
                    !(loc.volume_id == volume_id
                        && loc.relative_path.to_string_lossy() == relative_path)
                });
                if variant.locations.len() != before {
                    changed = true;
                }
            }
        }

        if changed {
            metadata_store.save(&asset)?;
        }
        Ok(())
    }

    /// Update a recipe's relative_path in the sidecar YAML.
    pub(super) fn update_sidecar_recipe_path(
        &self,
        metadata_store: &MetadataStore,
        catalog: &Catalog,
        variant_hash: &str,
        volume_id: Uuid,
        old_path: &str,
        new_path: &str,
    ) -> Result<()> {
        let asset_id = match catalog.find_asset_id_by_variant(variant_hash)? {
            Some(id) => id,
            None => return Ok(()),
        };
        let uuid: Uuid = asset_id.parse()?;
        let mut asset = metadata_store.load(uuid)?;

        let mut changed = false;
        for recipe in &mut asset.recipes {
            if recipe.location.volume_id == volume_id
                && recipe.location.relative_path.to_string_lossy() == old_path
            {
                recipe.location.relative_path = PathBuf::from(new_path);
                changed = true;
            }
        }

        if changed {
            metadata_store.save(&asset)?;
        }
        Ok(())
    }

    /// Remove a recipe from the sidecar YAML by matching volume_id + relative_path.
    pub fn remove_sidecar_recipe(
        &self,
        metadata_store: &MetadataStore,
        catalog: &Catalog,
        variant_hash: &str,
        volume_id: Uuid,
        relative_path: &str,
    ) -> Result<()> {
        let asset_id = match catalog.find_asset_id_by_variant(variant_hash)? {
            Some(id) => id,
            None => return Ok(()),
        };
        let uuid: Uuid = asset_id.parse()?;
        let mut asset = metadata_store.load(uuid)?;

        let before = asset.recipes.len();
        asset.recipes.retain(|r| {
            !(r.location.volume_id == volume_id
                && r.location.relative_path.to_string_lossy() == relative_path)
        });

        if asset.recipes.len() != before {
            metadata_store.save(&asset)?;
        }
        Ok(())
    }

}
