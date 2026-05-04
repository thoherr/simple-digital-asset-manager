//! `export` section of `AssetService` — methods extracted from the original
//! 8.9-kLOC asset_service.rs into a multi-file `impl AssetService` block.
//!
//! Free functions and result types live in the parent `asset_service` module.

use super::*;

impl AssetService {
    // ═══ EXPORT ═══

    /// Export files matching a search query to a target directory.
    ///
    /// Searches the catalog, resolves file locations on online volumes, and copies
    /// (or symlinks) files to the target directory. By default exports only the best
    /// variant per asset; `all_variants` exports every variant. `include_sidecars`
    /// also copies recipe files. `dry_run` reports the plan without writing files.
    /// Build an export plan: resolve assets, find online file locations, compute target paths.
    ///
    /// Returns `(plan, assets_matched, errors)`. The plan entries have `target_path` set
    /// relative to `target_base` (for directory export) or as ZIP entry names.
    pub fn build_export_plan(
        &self,
        asset_ids: &[String],
        target_base: &Path,
        layout: ExportLayout,
        all_variants: bool,
        include_sidecars: bool,
    ) -> Result<(Vec<ExportFilePlan>, usize, Vec<String>)> {
        use crate::catalog::Catalog;
        use crate::models::variant::best_preview_index_details;

        let catalog = Catalog::open(&self.catalog_root)?;
        let registry = DeviceRegistry::new(&self.catalog_root);

        let assets_matched = asset_ids.len();

        // Load volumes for resolving online mount points
        let volumes = registry.list()?;
        let online_volumes = crate::models::Volume::online_map(&volumes);

        let mut involved_volume_ids: HashSet<String> = HashSet::new();
        let mut plan: Vec<ExportFilePlan> = Vec::new();
        let mut planned_hashes: HashSet<String> = HashSet::new();
        let mut flat_seen: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        let mut errors: Vec<String> = Vec::new();

        for asset_id in asset_ids {
            let details = match catalog.load_asset_details(asset_id)? {
                Some(d) => d,
                None => {
                    errors.push(format!("Asset {} not found in catalog", &asset_id[..8]));
                    continue;
                }
            };

            let variant_indices: Vec<usize> = if all_variants {
                (0..details.variants.len()).collect()
            } else {
                match best_preview_index_details(&details.variants) {
                    Some(i) => vec![i],
                    None => {
                        errors.push(format!("Asset {} has no variants", &asset_id[..8]));
                        continue;
                    }
                }
            };

            for vi in &variant_indices {
                let variant = &details.variants[*vi];
                if planned_hashes.contains(&variant.content_hash) {
                    continue;
                }

                let loc = variant.locations.iter().find(|l| {
                    online_volumes.contains_key(&l.volume_id)
                });
                let loc = match loc {
                    Some(l) => l,
                    None => {
                        errors.push(format!(
                            "Asset {} variant {} — all locations offline",
                            &asset_id[..8],
                            &variant.content_hash[..12]
                        ));
                        continue;
                    }
                };

                let vol = online_volumes[&loc.volume_id];
                let source_path = vol.mount_point.join(&loc.relative_path);

                let target_path = match layout {
                    ExportLayout::Flat => {
                        resolve_flat_target(
                            target_base,
                            &variant.original_filename,
                            &variant.content_hash,
                            &mut flat_seen,
                        )
                    }
                    ExportLayout::Mirror => {
                        involved_volume_ids.insert(loc.volume_id.clone());
                        target_base.join(&loc.relative_path)
                    }
                };

                planned_hashes.insert(variant.content_hash.clone());
                plan.push(ExportFilePlan {
                    asset_id: asset_id.clone(),
                    content_hash: variant.content_hash.clone(),
                    source_path,
                    target_path,
                    file_size: variant.file_size,
                    is_sidecar: false,
                });
            }

            if include_sidecars {
                for recipe in &details.recipes {
                    let (vol_id, rel_path) = match (&recipe.volume_id, &recipe.relative_path) {
                        (Some(vid), Some(rp)) => (vid, rp),
                        _ => continue,
                    };

                    if planned_hashes.contains(&recipe.content_hash) {
                        continue;
                    }

                    let vol = match online_volumes.get(vol_id.as_str()) {
                        Some(v) => v,
                        None => continue,
                    };

                    let source_path = vol.mount_point.join(rel_path);
                    let filename = Path::new(rel_path)
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();

                    let target_path = match layout {
                        ExportLayout::Flat => {
                            resolve_flat_target(
                                target_base,
                                &filename,
                                &recipe.content_hash,
                                &mut flat_seen,
                            )
                        }
                        ExportLayout::Mirror => {
                            involved_volume_ids.insert(vol_id.clone());
                            target_base.join(rel_path)
                        }
                    };

                    let file_size = source_path.metadata().map(|m| m.len()).unwrap_or(0);
                    planned_hashes.insert(recipe.content_hash.clone());
                    plan.push(ExportFilePlan {
                        asset_id: asset_id.clone(),
                        content_hash: recipe.content_hash.clone(),
                        source_path,
                        target_path,
                        file_size,
                        is_sidecar: true,
                    });
                }
            }
        }

        // Mirror layout: if multiple volumes involved, prefix with volume label
        if layout == ExportLayout::Mirror && involved_volume_ids.len() > 1 {
            for entry in &mut plan {
                for vol in &volumes {
                    if vol.is_online
                        && entry.source_path.starts_with(&vol.mount_point)
                    {
                        if let Ok(rel) = entry.source_path.strip_prefix(&vol.mount_point) {
                            entry.target_path = target_base.join(&vol.label).join(rel);
                        }
                        break;
                    }
                }
            }
        }

        Ok((plan, assets_matched, errors))
    }

    pub fn export(
        &self,
        query: &str,
        target_dir: &Path,
        layout: ExportLayout,
        symlink: bool,
        all_variants: bool,
        include_sidecars: bool,
        dry_run: bool,
        overwrite: bool,
        on_file: impl Fn(&Path, &ExportStatus, Duration),
    ) -> Result<ExportResult> {
        let engine = crate::query::QueryEngine::new(&self.catalog_root);
        let content_store = ContentStore::new(&self.catalog_root);

        // Phase 1: Search
        let search_results = engine.search(query)?;
        let assets_matched = search_results.len();

        if assets_matched == 0 {
            return Ok(ExportResult {
                dry_run,
                assets_matched: 0,
                files_exported: 0,
                files_skipped: 0,
                sidecars_exported: 0,
                total_bytes: 0,
                errors: Vec::new(),
            });
        }

        // Phase 2: Build plan
        let asset_ids: Vec<String> = search_results.iter().map(|r| r.asset_id.clone()).collect();
        let (plan, _, errors) = self.build_export_plan(&asset_ids, target_dir, layout, all_variants, include_sidecars)?;

        // Phase 3: Execute or dry-run
        let mut result = ExportResult {
            dry_run,
            assets_matched,
            files_exported: 0,
            files_skipped: 0,
            sidecars_exported: 0,
            total_bytes: 0,
            errors,
        };

        for entry in &plan {
            let file_start = Instant::now();

            if dry_run {
                if entry.is_sidecar {
                    result.sidecars_exported += 1;
                } else {
                    result.files_exported += 1;
                }
                result.total_bytes += entry.file_size;
                on_file(&entry.target_path, &ExportStatus::Copied, file_start.elapsed());
                continue;
            }

            // Check if target already exists with matching hash
            if !overwrite && entry.target_path.exists() {
                match content_store.hash_file(&entry.target_path) {
                    Ok(existing_hash) if existing_hash == entry.content_hash => {
                        result.files_skipped += 1;
                        on_file(
                            &entry.target_path,
                            &ExportStatus::Skipped,
                            file_start.elapsed(),
                        );
                        continue;
                    }
                    _ => {} // different hash or error — proceed with copy/overwrite
                }
            }

            // Create parent directories
            if let Some(parent) = entry.target_path.parent() {
                if let Err(e) = std::fs::create_dir_all(parent) {
                    let msg = format!(
                        "{} — failed to create directory: {}",
                        entry.target_path.display(),
                        e
                    );
                    result.errors.push(msg.clone());
                    on_file(&entry.target_path, &ExportStatus::Error(msg), file_start.elapsed());
                    continue;
                }
            }

            if symlink {
                match create_symlink(&entry.source_path, &entry.target_path) {
                    Ok(()) => {
                        if entry.is_sidecar {
                            result.sidecars_exported += 1;
                        } else {
                            result.files_exported += 1;
                        }
                        result.total_bytes += entry.file_size;
                        on_file(
                            &entry.target_path,
                            &ExportStatus::Linked,
                            file_start.elapsed(),
                        );
                    }
                    Err(e) => {
                        let msg = format!(
                            "{} — symlink failed: {}",
                            entry.target_path.display(),
                            e
                        );
                        result.errors.push(msg.clone());
                        on_file(
                            &entry.target_path,
                            &ExportStatus::Error(msg),
                            file_start.elapsed(),
                        );
                    }
                }
            } else {
                match content_store.copy_and_verify(
                    &entry.source_path,
                    &entry.target_path,
                    &entry.content_hash,
                ) {
                    Ok(()) => {
                        if entry.is_sidecar {
                            result.sidecars_exported += 1;
                        } else {
                            result.files_exported += 1;
                        }
                        result.total_bytes += entry.file_size;
                        on_file(
                            &entry.target_path,
                            &ExportStatus::Copied,
                            file_start.elapsed(),
                        );
                    }
                    Err(e) => {
                        let msg = format!(
                            "{} — copy failed: {}",
                            entry.target_path.display(),
                            e
                        );
                        result.errors.push(msg.clone());
                        on_file(
                            &entry.target_path,
                            &ExportStatus::Error(msg),
                            file_start.elapsed(),
                        );
                    }
                }
            }
        }

        Ok(result)
    }

    /// Export matching assets as a ZIP archive.
    pub fn export_zip(
        &self,
        query: &str,
        zip_path: &Path,
        layout: ExportLayout,
        all_variants: bool,
        include_sidecars: bool,
        on_file: impl Fn(&Path, &ExportStatus, Duration),
    ) -> Result<ExportResult> {
        let engine = crate::query::QueryEngine::new(&self.catalog_root);

        let search_results = engine.search(query)?;
        let assets_matched = search_results.len();

        if assets_matched == 0 {
            return Ok(ExportResult {
                dry_run: false,
                assets_matched: 0,
                files_exported: 0,
                files_skipped: 0,
                sidecars_exported: 0,
                total_bytes: 0,
                errors: Vec::new(),
            });
        }

        let asset_ids: Vec<String> = search_results.iter().map(|r| r.asset_id.clone()).collect();
        self.export_zip_for_ids(&asset_ids, zip_path, layout, all_variants, include_sidecars, on_file)
    }

    /// Export specific asset IDs as a ZIP archive.
    pub fn export_zip_for_ids(
        &self,
        asset_ids: &[String],
        zip_path: &Path,
        layout: ExportLayout,
        all_variants: bool,
        include_sidecars: bool,
        on_file: impl Fn(&Path, &ExportStatus, Duration),
    ) -> Result<ExportResult> {
        use std::io::Write;
        use zip::write::{SimpleFileOptions, ZipWriter};

        let dummy_base = Path::new("");
        let (plan, assets_matched, errors) =
            self.build_export_plan(asset_ids, dummy_base, layout, all_variants, include_sidecars)?;

        let mut result = ExportResult {
            dry_run: false,
            assets_matched,
            files_exported: 0,
            files_skipped: 0,
            sidecars_exported: 0,
            total_bytes: 0,
            errors,
        };

        if plan.is_empty() {
            return Ok(result);
        }

        if let Some(parent) = zip_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let file = std::fs::File::create(zip_path)?;
        let writer = std::io::BufWriter::with_capacity(1024 * 1024, file);
        let mut zip = ZipWriter::new(writer);
        let options = SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);

        for entry in &plan {
            let file_start = Instant::now();
            let entry_name = entry.target_path.to_string_lossy().replace('\\', "/");
            let entry_name = entry_name.trim_start_matches('/').trim_start_matches("./");

            if let Err(e) = zip.start_file(entry_name, options) {
                let msg = format!("{entry_name} — zip entry failed: {e}");
                result.errors.push(msg.clone());
                on_file(&entry.target_path, &ExportStatus::Error(msg), file_start.elapsed());
                continue;
            }

            let src = match std::fs::File::open(&entry.source_path) {
                Ok(f) => f,
                Err(e) => {
                    let msg = format!("{entry_name} — open failed: {e}");
                    result.errors.push(msg.clone());
                    on_file(&entry.target_path, &ExportStatus::Error(msg), file_start.elapsed());
                    continue;
                }
            };
            let mut reader = std::io::BufReader::with_capacity(256 * 1024, src);
            let mut buf = vec![0u8; 256 * 1024];
            loop {
                let n = match std::io::Read::read(&mut reader, &mut buf) {
                    Ok(0) => break,
                    Ok(n) => n,
                    Err(_) => break,
                };
                if zip.write_all(&buf[..n]).is_err() {
                    break;
                }
            }

            if entry.is_sidecar {
                result.sidecars_exported += 1;
            } else {
                result.files_exported += 1;
            }
            result.total_bytes += entry.file_size;
            on_file(&entry.target_path, &ExportStatus::Copied, file_start.elapsed());
        }

        zip.finish().map_err(|e| anyhow::anyhow!("failed to finalize ZIP: {e}"))?;
        Ok(result)
    }

}
