//! `ai` section of `AssetService` — methods extracted from the original
//! 8.9-kLOC asset_service.rs into a multi-file `impl AssetService` block.
//!
//! Free functions and result types live in the parent `asset_service` module.

use super::*;

impl AssetService {
    // ═══ AI & FACES ═══

    /// Auto-tag assets using SigLIP zero-shot classification.
    #[cfg(feature = "ai")]
    pub fn auto_tag(
        &self,
        query: Option<&str>,
        asset_id: Option<&str>,
        volume: Option<&str>,
        threshold: f32,
        labels: &[String],
        prompt_template: &str,
        apply: bool,
        model_dir: &std::path::Path,
        model_id: &str,
        execution_provider: &str,
        on_asset: impl Fn(&str, &crate::ai::AutoTagStatus, Duration),
    ) -> Result<crate::ai::AutoTagResult> {
        use crate::ai::{self, AutoTagResult, AutoTagStatus, AssetSuggestions, SigLipModel};
        use crate::catalog::Catalog;
        use crate::embedding_store::EmbeddingStore;
        use crate::preview::PreviewGenerator;

        let catalog = Catalog::open(&self.catalog_root)?;
        let engine = crate::query::QueryEngine::new(&self.catalog_root);
        let preview_gen = PreviewGenerator::new(&self.catalog_root, self.verbosity, &self.preview_config);
        let registry = DeviceRegistry::new(&self.catalog_root);
        let volumes = registry.list()?;
        let online_volumes = crate::models::Volume::online_map(&volumes);

        // Load model
        let mut model = SigLipModel::load_with_provider(model_dir, model_id, self.verbosity, execution_provider)?;

        // Prepare label texts with prompt template
        let prompted_labels: Vec<String> = labels
            .iter()
            .map(|l| ai::apply_prompt_template(prompt_template, l))
            .collect();

        // Pre-encode all label texts
        let label_embs = model.encode_texts(&prompted_labels)?;

        // Resolve target assets
        let asset_ids: Vec<String> = if let Some(id) = asset_id {
            let full_id = catalog
                .resolve_asset_id(id)?
                .ok_or_else(|| anyhow::anyhow!("no asset found matching '{id}'"))?;
            vec![full_id]
        } else {
            let q = if let Some(query) = query {
                let volume_part = volume.map(|v| format!(" volume:{v}")).unwrap_or_default();
                format!("{query}{volume_part}")
            } else if let Some(v) = volume {
                format!("volume:{v}")
            } else {
                "*".to_string()
            };
            let results = engine.search(&q)?;
            results.into_iter().map(|r| r.asset_id).collect()
        };

        let mut result = AutoTagResult {
            assets_processed: 0,
            assets_skipped: 0,
            tags_suggested: 0,
            tags_applied: 0,
            errors: Vec::new(),
            dry_run: !apply,
            suggestions: Vec::new(),
        };

        // Initialize embedding store
        let _ = EmbeddingStore::initialize(catalog.conn());
        let emb_store = EmbeddingStore::new(catalog.conn());

        for aid in &asset_ids {
            let asset_start = Instant::now();

            // Load asset details to find preview/image file
            let details = match catalog.load_asset_details(aid)? {
                Some(d) => d,
                None => {
                    let msg = format!("Asset {} not found", &aid[..8.min(aid.len())]);
                    result.errors.push(msg.clone());
                    on_asset(aid, &AutoTagStatus::Error(msg), asset_start.elapsed());
                    continue;
                }
            };

            // Find an image to process: smart preview > regular preview > original on online volume
            let image_path = self.find_image_for_ai(&details, &preview_gen, &online_volumes);

            let image_path = match image_path {
                Some(p) => p,
                None => {
                    let msg = format!(
                        "No processable image for asset {}",
                        &aid[..8.min(aid.len())]
                    );
                    result.assets_skipped += 1;
                    on_asset(aid, &AutoTagStatus::Skipped(msg), asset_start.elapsed());
                    continue;
                }
            };

            // Check if the image format is supported
            let ext = image_path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("");
            if !ai::is_supported_image(ext) {
                let msg = format!(
                    "Unsupported format '{}' for asset {}",
                    ext,
                    &aid[..8.min(aid.len())]
                );
                result.assets_skipped += 1;
                on_asset(aid, &AutoTagStatus::Skipped(msg), asset_start.elapsed());
                continue;
            }

            // Encode image
            let image_emb = match model.encode_image(&image_path) {
                Ok(emb) => emb,
                Err(e) => {
                    let msg = format!(
                        "Failed to encode image for {}: {e}",
                        &aid[..8.min(aid.len())]
                    );
                    result.errors.push(msg.clone());
                    on_asset(aid, &AutoTagStatus::Error(msg), asset_start.elapsed());
                    continue;
                }
            };

            // Store embedding (SQLite + binary file)
            if let Err(e) = emb_store.store(aid, &image_emb, model_id) {
                eprintln!("Warning: failed to store embedding for {}: {e}", &aid[..8.min(aid.len())]);
            }
            let _ = crate::embedding_store::write_embedding_binary(&self.catalog_root, model_id, aid, &image_emb);

            // Classify
            let suggestions = if self.verbosity.debug {
                eprintln!("  [debug] asset {} — image: {}", &aid[..8.min(aid.len())], image_path.display());
                let norm: f32 = image_emb.iter().map(|x| x * x).sum::<f32>().sqrt();
                eprintln!("  [debug] embedding norm: {norm:.6} (expected ~1.0 for L2-normalized)");
                model.classify_debug(&image_emb, labels, &label_embs, threshold)
            } else {
                model.classify(&image_emb, labels, &label_embs, threshold)
            };

            // Filter out tags already on the asset
            let existing_tags: HashSet<String> = details
                .tags
                .iter()
                .map(|t| t.to_lowercase())
                .collect();
            let new_suggestions: Vec<_> = suggestions
                .into_iter()
                .filter(|s| {
                    let dominated = existing_tags.contains(&s.tag.to_lowercase());
                    if dominated && self.verbosity.debug {
                        eprintln!("  [debug] skipping '{}' ({:.2}%) — tag already exists on asset", s.tag, s.confidence * 100.0);
                    }
                    !dominated
                })
                .collect();

            result.tags_suggested += new_suggestions.len();

            if apply && !new_suggestions.is_empty() {
                let new_tags: Vec<String> = new_suggestions.iter().map(|s| s.tag.clone()).collect();
                match engine.tag(aid, &new_tags, false) {
                    Ok(_) => {
                        result.tags_applied += new_tags.len();
                    }
                    Err(e) => {
                        let msg = format!(
                            "Failed to apply tags to {}: {e}",
                            &aid[..8.min(aid.len())]
                        );
                        result.errors.push(msg.clone());
                    }
                }
            }

            result.assets_processed += 1;
            result.suggestions.push(AssetSuggestions {
                asset_id: aid.clone(),
                suggested_tags: new_suggestions.clone(),
                applied: apply,
            });

            let status = if apply {
                AutoTagStatus::Applied(new_suggestions)
            } else {
                AutoTagStatus::Suggested(new_suggestions)
            };
            on_asset(aid, &status, asset_start.elapsed());
        }

        Ok(result)
    }

    /// Generate SigLIP embeddings for the given asset IDs.
    ///
    /// Reused by `maki embed` and the post-import embedding phase (CLI + web).
    /// Skips assets that already have an embedding for `model_id` unless `force`.
    /// `on_asset` reports per-asset progress: status enum + elapsed time.
    #[cfg(feature = "ai")]
    pub fn embed_assets(
        &self,
        asset_ids: &[String],
        model_dir: &std::path::Path,
        model_id: &str,
        execution_provider: &str,
        force: bool,
        on_asset: impl Fn(&str, &EmbedStatus, std::time::Duration),
    ) -> Result<EmbedAssetsResult> {
        use crate::ai::SigLipModel;
        use crate::catalog::Catalog;
        use crate::embedding_store::EmbeddingStore;
        use crate::preview::PreviewGenerator;

        let catalog = Catalog::open(&self.catalog_root)?;
        let _ = EmbeddingStore::initialize(catalog.conn());
        let emb_store = EmbeddingStore::new(catalog.conn());
        let preview_gen = PreviewGenerator::new(&self.catalog_root, self.verbosity, &self.preview_config);
        let registry = DeviceRegistry::new(&self.catalog_root);
        let volumes = registry.list()?;
        let online_volumes = crate::models::Volume::online_map(&volumes);

        let mut model = SigLipModel::load_with_provider(
            model_dir,
            model_id,
            self.verbosity,
            execution_provider,
        )?;

        let mut result = EmbedAssetsResult {
            embedded: 0,
            skipped: 0,
            errors: Vec::new(),
        };

        for aid in asset_ids {
            let asset_start = Instant::now();

            if !force && emb_store.has_embedding(aid, model_id) {
                result.skipped += 1;
                on_asset(aid, &EmbedStatus::Skipped("already exists"), asset_start.elapsed());
                continue;
            }

            let details = match catalog.load_asset_details(aid)? {
                Some(d) => d,
                None => {
                    let msg = "asset not found";
                    result.errors.push(format!("{}: {msg}", &aid[..8.min(aid.len())]));
                    on_asset(aid, &EmbedStatus::Error(msg.to_string()), asset_start.elapsed());
                    continue;
                }
            };

            let image_path = match self.find_image_for_ai(&details, &preview_gen, &online_volumes) {
                Some(p) => p,
                None => {
                    result.skipped += 1;
                    on_asset(aid, &EmbedStatus::Skipped("no processable image"), asset_start.elapsed());
                    continue;
                }
            };

            let ext = image_path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if !crate::ai::is_supported_image(ext) {
                result.skipped += 1;
                on_asset(aid, &EmbedStatus::Skipped("unsupported format"), asset_start.elapsed());
                continue;
            }

            match model.encode_image(&image_path) {
                Ok(emb) => {
                    if let Err(e) = emb_store.store(aid, &emb, model_id) {
                        let msg = format!("failed to store: {e}");
                        result.errors.push(format!("{}: {msg}", &aid[..8.min(aid.len())]));
                        on_asset(aid, &EmbedStatus::Error(msg), asset_start.elapsed());
                        continue;
                    }
                    let _ = crate::embedding_store::write_embedding_binary(
                        &self.catalog_root,
                        model_id,
                        aid,
                        &emb,
                    );
                    result.embedded += 1;
                    on_asset(aid, &EmbedStatus::Embedded, asset_start.elapsed());
                }
                Err(e) => {
                    let msg = format!("{e:#}");
                    result.errors.push(format!("{}: {msg}", &aid[..8.min(aid.len())]));
                    on_asset(aid, &EmbedStatus::Error(msg), asset_start.elapsed());
                }
            }
        }

        Ok(result)
    }

    /// Find the best image file for processing.
    /// Priority: smart preview > regular preview > original on online volume.
    /// The `is_supported` predicate controls which original file extensions are accepted.
    fn find_image_for_processing(
        &self,
        details: &crate::catalog::AssetDetails,
        preview_gen: &crate::preview::PreviewGenerator,
        online_volumes: &std::collections::HashMap<String, &crate::models::Volume>,
        is_supported: impl Fn(&str) -> bool,
    ) -> Option<PathBuf> {
        // Try smart preview of best variant
        if let Some(best) = crate::models::variant::best_preview_index_details(&details.variants) {
            let variant = &details.variants[best];
            let smart_path = preview_gen.smart_preview_path(&variant.content_hash);
            if smart_path.exists() {
                return Some(smart_path);
            }
            let preview_path = preview_gen.preview_path(&variant.content_hash);
            if preview_path.exists() {
                return Some(preview_path);
            }
        }

        // Fall back to any preview we can find
        for variant in &details.variants {
            let smart_path = preview_gen.smart_preview_path(&variant.content_hash);
            if smart_path.exists() {
                return Some(smart_path);
            }
            let preview_path = preview_gen.preview_path(&variant.content_hash);
            if preview_path.exists() {
                return Some(preview_path);
            }
        }

        // Fall back to original file on an online volume
        for variant in &details.variants {
            for loc in &variant.locations {
                if let Some(vol) = online_volumes.get(&loc.volume_id) {
                    let full_path = vol.mount_point.join(&loc.relative_path);
                    if full_path.exists() {
                        let ext = full_path
                            .extension()
                            .and_then(|e| e.to_str())
                            .unwrap_or("");
                        if is_supported(ext) {
                            return Some(full_path);
                        }
                    }
                }
            }
        }

        None
    }

    /// Find the best image file for AI embedding/detection.
    #[cfg(feature = "ai")]
    pub fn find_image_for_ai(
        &self,
        details: &crate::catalog::AssetDetails,
        preview_gen: &crate::preview::PreviewGenerator,
        online_volumes: &std::collections::HashMap<String, &crate::models::Volume>,
    ) -> Option<PathBuf> {
        self.find_image_for_processing(details, preview_gen, online_volumes, |ext| {
            crate::ai::is_supported_image(ext)
        })
    }

    /// Find the best image file for VLM processing.
    pub fn find_image_for_vlm(
        &self,
        details: &crate::catalog::AssetDetails,
        preview_gen: &crate::preview::PreviewGenerator,
        online_volumes: &std::collections::HashMap<String, &crate::models::Volume>,
    ) -> Option<PathBuf> {
        self.find_image_for_processing(details, preview_gen, online_volumes, |ext| {
            matches!(
                ext.to_lowercase().as_str(),
                "jpg" | "jpeg" | "png" | "webp" | "tif" | "tiff" | "bmp" | "gif"
            )
        })
    }

    /// Detect faces in a batch of assets.
    ///
    /// Shared implementation used by both CLI `maki faces detect` and web batch detect.
    /// `force`: if true, clears existing faces before re-detecting; if false, skips assets
    /// that already have faces.
    #[cfg(feature = "ai")]
    pub fn detect_faces(
        &self,
        asset_ids: &[String],
        detector: &mut crate::face::FaceDetector,
        min_confidence: f32,
        force: bool,
        apply: bool,
        on_asset: impl Fn(&str, u32, std::time::Duration),
    ) -> Result<DetectFacesResult> {
        let catalog = crate::catalog::Catalog::open(&self.catalog_root)?;
        let _ = crate::face_store::FaceStore::initialize(catalog.conn());
        let face_store = crate::face_store::FaceStore::new(catalog.conn());
        let engine = crate::query::QueryEngine::new(&self.catalog_root);
        let metadata_store = crate::metadata_store::MetadataStore::new(&self.catalog_root);
        let preview_gen = crate::preview::PreviewGenerator::new(&self.catalog_root, crate::Verbosity::quiet(), &self.preview_config);
        let registry = crate::device_registry::DeviceRegistry::new(&self.catalog_root);
        let volumes = registry.list()?;
        let online_volumes = crate::models::Volume::online_map(&volumes);

        let mut result = DetectFacesResult {
            assets_processed: 0,
            assets_skipped: 0,
            faces_detected: 0,
            errors: Vec::new(),
        };

        for aid in asset_ids {
            let t0 = std::time::Instant::now();
            let short_id = &aid[..8.min(aid.len())];

            // Skip if this asset has already been scanned (regardless of whether
            // any face was found). Without this, every landscape / product shot /
            // document in the catalog gets re-scanned on every `detect` run —
            // waste of compute proportional to catalog size.
            //
            // `--force` bypasses the check entirely. A secondary fallback on
            // `has_faces()` covers pre-v7 catalogs whose face_scan_status column
            // wasn't populated until migration (assets with faces get backfilled
            // during migration, but this is belt-and-braces for partial upgrades).
            if !force && (catalog.is_face_scan_done(aid) || face_store.has_faces(aid)) {
                result.assets_skipped += 1;
                on_asset(aid, 0, t0.elapsed());
                continue;
            }

            let details = match engine.show(aid) {
                Ok(d) => d,
                Err(e) => {
                    result.errors.push(format!("{short_id}: {e:#}"));
                    continue;
                }
            };

            let image_path = match self.find_image_for_ai(&details, &preview_gen, &online_volumes) {
                Some(p) => p,
                None => {
                    // No accessible image — don't mark as scanned, so a later run
                    // with the volume online will actually process it.
                    result.assets_skipped += 1;
                    continue;
                }
            };

            match detector.detect_and_embed(&image_path, min_confidence) {
                Ok(face_results) => {
                    let n = face_results.len() as u32;
                    if apply {
                        if force {
                            let _ = face_store.delete_faces_for_asset(aid);
                        }
                        for (face, embedding) in &face_results {
                            let face_id = uuid::Uuid::new_v4().to_string();
                            if let Err(e) = face_store.store_face(
                                &face_id, aid, face.bbox_x, face.bbox_y, face.bbox_w, face.bbox_h,
                                embedding, face.confidence,
                                crate::face::RECOGNITION_MODEL.id,
                            ) {
                                result.errors.push(format!("{short_id}: store error: {e:#}"));
                            } else {
                                let _ = crate::face::save_face_crop(&image_path, face, &face_id, &self.catalog_root);
                                let _ = crate::face_store::write_arcface_binary(&self.catalog_root, &face_id, embedding);
                            }
                        }
                        let _ = catalog.update_face_count(aid);
                        // Mark the asset as scanned regardless of whether any faces
                        // were found. This is the key optimization: without it, a
                        // 100k-asset catalog with mostly landscapes gets re-scanned
                        // in full on every `faces detect` run.
                        //
                        // Persist to both SQLite (fast filtering) AND the YAML sidecar
                        // (source of truth for rebuild-catalog). If only SQLite had it,
                        // a rebuild would lose the "scanned, no face" knowledge and the
                        // whole catalog would need re-scanning on the first detect run
                        // post-rebuild.
                        let _ = catalog.mark_face_scan_done(aid);
                        if let Ok(uid) = aid.parse::<uuid::Uuid>() {
                            if let Ok(mut asset) = metadata_store.load(uid) {
                                if asset.face_scan_status.as_deref() != Some("done") {
                                    asset.face_scan_status = Some("done".to_string());
                                    let _ = metadata_store.save(&asset);
                                }
                            }
                        }
                    }
                    result.faces_detected += n;
                    result.assets_processed += 1;
                    on_asset(aid, n, t0.elapsed());
                }
                Err(e) => {
                    // Detection failed (I/O error, decode error, etc.) — don't
                    // mark as scanned so a future run can retry.
                    result.errors.push(format!("{short_id}: {e:#}"));
                }
            }
        }

        // Persist faces/people YAML after all detections
        if apply && result.faces_detected > 0 {
            if let Err(e) = face_store.save_all_yaml(&self.catalog_root) {
                result.errors.push(format!("Failed to save faces/people YAML: {e:#}"));
            }
        }

        Ok(result)
    }

}
