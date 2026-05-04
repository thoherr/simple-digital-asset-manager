//! `video` section of `AssetService` — methods extracted from the original
//! 8.9-kLOC asset_service.rs into a multi-file `impl AssetService` block.
//!
//! Free functions and result types live in the parent `asset_service` module.

use super::*;

impl AssetService {
    // ═══ VIDEO METADATA ═══

    /// Backfill video metadata (duration, codec, resolution, framerate) via ffprobe.
    ///
    /// Updates variant source_metadata in both the YAML sidecar and SQLite catalog.
    /// Returns true if metadata was added.
    pub fn backfill_video_metadata(
        &self,
        asset_id: &str,
        variant_hash: &str,
        source_path: &std::path::Path,
    ) -> bool {
        let video_meta = crate::preview::extract_video_metadata(source_path);
        if video_meta.is_empty() {
            return false;
        }
        let store = MetadataStore::new(&self.catalog_root);
        let uuid = match asset_id.parse::<uuid::Uuid>() {
            Ok(u) => u,
            Err(_) => return false,
        };
        if let Ok(mut asset) = store.load(uuid) {
            if let Some(v) = asset.variants.iter_mut().find(|v| v.content_hash == variant_hash) {
                v.source_metadata.extend(video_meta);
                if let Ok(catalog) = crate::catalog::Catalog::open(&self.catalog_root) {
                    catalog.insert_variant(v).ok();
                    catalog.insert_asset(&asset).ok();
                }
            }
            let _ = store.save(&asset);
            true
        } else {
            false
        }
    }

    /// Batch-describe assets using a VLM endpoint.
    pub fn describe(
        &self,
        query: Option<&str>,
        asset_id: Option<&str>,
        volume: Option<&str>,
        endpoint: &str,
        model: &str,
        params: &crate::vlm::VlmParams,
        mode: crate::vlm::DescribeMode,
        apply: bool,
        force: bool,
        dry_run: bool,
        concurrency: u32,
        on_asset: impl Fn(&str, &crate::vlm::DescribeStatus, std::time::Duration) + Sync,
    ) -> Result<crate::vlm::BatchDescribeResult> {
        self.describe_inner(query, asset_id, volume, None, endpoint, model, params, mode, apply, force, dry_run, concurrency, on_asset)
    }

    /// Describe specific assets by ID (for post-import phase).
    pub fn describe_assets(
        &self,
        asset_ids: &[String],
        endpoint: &str,
        model: &str,
        params: &crate::vlm::VlmParams,
        mode: crate::vlm::DescribeMode,
        force: bool,
        dry_run: bool,
        concurrency: u32,
        on_asset: impl Fn(&str, &crate::vlm::DescribeStatus, std::time::Duration) + Sync,
    ) -> Result<crate::vlm::BatchDescribeResult> {
        self.describe_inner(None, None, None, Some(asset_ids), endpoint, model, params, mode, true, force, dry_run, concurrency, on_asset)
    }

    fn describe_inner(
        &self,
        query: Option<&str>,
        asset_id: Option<&str>,
        volume: Option<&str>,
        explicit_ids: Option<&[String]>,
        endpoint: &str,
        model: &str,
        params: &crate::vlm::VlmParams,
        mode: crate::vlm::DescribeMode,
        apply: bool,
        force: bool,
        dry_run: bool,
        concurrency: u32,
        on_asset: impl Fn(&str, &crate::vlm::DescribeStatus, std::time::Duration) + Sync,
    ) -> Result<crate::vlm::BatchDescribeResult> {
        use crate::vlm::{self, BatchDescribeResult, DescribeMode, DescribeResult, DescribeStatus};

        let catalog = crate::catalog::Catalog::open(&self.catalog_root)?;
        let engine = crate::query::QueryEngine::new(&self.catalog_root);
        let preview_gen =
            crate::preview::PreviewGenerator::new(&self.catalog_root, self.verbosity, &self.preview_config);
        let registry = DeviceRegistry::new(&self.catalog_root);
        let volumes = registry.list()?;
        let online_volumes: HashMap<String, &crate::models::Volume> = volumes
            .iter()
            .filter(|v| v.is_online)
            .map(|v| (v.id.to_string(), v))
            .collect();

        // Resolve target assets
        let asset_ids: Vec<String> = if let Some(ids) = explicit_ids {
            ids.to_vec()
        } else if let Some(id) = asset_id {
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

        let wants_description = mode == DescribeMode::Describe || mode == DescribeMode::Both;
        let concurrency = (concurrency.max(1)) as usize;

        if self.verbosity.verbose {
            eprintln!("  Describe: {} candidate asset(s), concurrency={concurrency}", asset_ids.len());
        }

        let mut result = BatchDescribeResult {
            described: 0,
            skipped: 0,
            failed: 0,
            tags_applied: 0,
            errors: Vec::new(),
            dry_run: !apply || dry_run,
            mode: mode.to_string(),
            results: Vec::new(),
        };

        // Phase 1: Prepare work items (sequential — needs catalog reads)
        struct WorkItem {
            asset_id: String,
            image_path: std::path::PathBuf,
            existing_tags: HashSet<String>,
        }
        let mut work_items: Vec<WorkItem> = Vec::new();

        for aid in &asset_ids {
            let asset_start = std::time::Instant::now();
            let short_id = &aid[..8.min(aid.len())];

            // Load asset details
            let details = match catalog.load_asset_details(aid)? {
                Some(d) => d,
                None => {
                    let msg = format!("Asset {short_id} not found");
                    result.errors.push(msg.clone());
                    result.failed += 1;
                    result.results.push(DescribeResult {
                        asset_id: aid.clone(),
                        description: None,
                        tags: Vec::new(),
                        status: DescribeStatus::Error(msg.clone()),
                    });
                    on_asset(aid, &DescribeStatus::Error(msg), asset_start.elapsed());
                    continue;
                }
            };

            // In describe/both modes, skip if description exists and --force not set
            if wants_description && !force {
                if let Some(ref desc) = details.description {
                    if !desc.is_empty() {
                        let msg = "already has description".to_string();
                        result.skipped += 1;
                        result.results.push(DescribeResult {
                            asset_id: aid.clone(),
                            description: Some(desc.clone()),
                            tags: Vec::new(),
                            status: DescribeStatus::Skipped(msg.clone()),
                        });
                        on_asset(aid, &DescribeStatus::Skipped(msg), asset_start.elapsed());
                        continue;
                    }
                }
            }

            // Find image
            let image_path = self.find_image_for_vlm(&details, &preview_gen, &online_volumes);
            let image_path = match image_path {
                Some(p) => p,
                None => {
                    let msg = format!("No preview/image for asset {short_id}. Run `maki generate-previews` first.");
                    result.skipped += 1;
                    result.results.push(DescribeResult {
                        asset_id: aid.clone(),
                        description: None,
                        tags: Vec::new(),
                        status: DescribeStatus::Skipped(msg.clone()),
                    });
                    on_asset(aid, &DescribeStatus::Skipped(msg), asset_start.elapsed());
                    continue;
                }
            };

            if dry_run {
                let msg = format!("would process (image: {})", image_path.display());
                result.described += 1;
                result.results.push(DescribeResult {
                    asset_id: aid.clone(),
                    description: None,
                    tags: Vec::new(),
                    status: DescribeStatus::Described,
                });
                on_asset(aid, &DescribeStatus::Skipped(msg), asset_start.elapsed());
                continue;
            }

            let existing_tags: HashSet<String> = details
                .tags
                .iter()
                .map(|t| t.to_lowercase())
                .collect();

            work_items.push(WorkItem {
                asset_id: aid.clone(),
                image_path,
                existing_tags,
            });
        }

        // Phase 2: VLM calls in parallel batches
        let verbosity = self.verbosity;
        for chunk in work_items.chunks(concurrency) {
            // Each chunk runs concurrently using scoped threads
            let vlm_results: Vec<(String, HashSet<String>, std::time::Duration, Result<vlm::VlmOutput, String>)> =
                std::thread::scope(|s| {
                    let handles: Vec<_> = chunk
                        .iter()
                        .map(|item| {
                            let aid = &item.asset_id;
                            let image_path = &item.image_path;
                            s.spawn(move || {
                                let start = std::time::Instant::now();
                                let short_id = &aid[..8.min(aid.len())];

                                // Encode image to base64
                                let vlm_max_edge = if params.max_image_edge > 0 { Some(params.max_image_edge) } else { None };
                                let image_base64 = match vlm::encode_image_base64(image_path, vlm_max_edge) {
                                    Ok(b) => b,
                                    Err(e) => {
                                        return (
                                            aid.clone(),
                                            start.elapsed(),
                                            Err(format!("Failed to read image for {short_id}: {e}")),
                                        );
                                    }
                                };

                                // Call VLM
                                let prompt = params.prompt.as_deref()
                                    .unwrap_or_else(|| vlm::default_prompt_for_mode(mode));
                                match vlm::call_vlm_with_mode(
                                    endpoint, model, &image_base64, prompt,
                                    params, mode, verbosity,
                                ) {
                                    Ok(output) => {
                                        if output.description.as_ref().map_or(true, |d| d.is_empty())
                                            && output.tags.is_empty()
                                        {
                                            (
                                                aid.clone(),
                                                start.elapsed(),
                                                Err(format!(
                                                    "VLM returned empty response for {short_id} — \
                                                     model \"{model}\" may not support vision or failed to load"
                                                )),
                                            )
                                        } else {
                                            (aid.clone(), start.elapsed(), Ok(output))
                                        }
                                    }
                                    Err(e) => (
                                        aid.clone(),
                                        start.elapsed(),
                                        Err(format!("VLM failed for {short_id}: {e}")),
                                    ),
                                }
                            })
                        })
                        .collect();

                    handles
                        .into_iter()
                        .zip(chunk.iter())
                        .map(|(h, item)| {
                            let (aid, elapsed, vlm_result) = h.join().expect("VLM processing thread should not panic");
                            (aid, item.existing_tags.clone(), elapsed, vlm_result)
                        })
                        .collect()
                });

            // Phase 3: Apply results sequentially (catalog writes not thread-safe)
            for (aid, existing_tags, elapsed, vlm_result) in vlm_results {
                let short_id = &aid[..8.min(aid.len())];

                match vlm_result {
                    Err(msg) => {
                        result.errors.push(msg.clone());
                        result.failed += 1;
                        result.results.push(DescribeResult {
                            asset_id: aid.clone(),
                            description: None,
                            tags: Vec::new(),
                            status: DescribeStatus::Error(msg.clone()),
                        });
                        on_asset(&aid, &DescribeStatus::Error(msg), elapsed);
                    }
                    Ok(output) => {
                        if apply {
                            // Apply description
                            if let Some(ref desc) = output.description {
                                if !desc.is_empty() {
                                    let edit_fields = crate::query::EditFields {
                                        name: None,
                                        description: Some(Some(desc.clone())),
                                        rating: None,
                                        color_label: None,
                                        created_at: None,
                                    };
                                    if let Err(e) = engine.edit(&aid, edit_fields) {
                                        let msg = format!("Failed to save description for {short_id}: {e}");
                                        result.errors.push(msg.clone());
                                        result.failed += 1;
                                        result.results.push(DescribeResult {
                                            asset_id: aid.clone(),
                                            description: output.description,
                                            tags: output.tags,
                                            status: DescribeStatus::Error(msg.clone()),
                                        });
                                        on_asset(&aid, &DescribeStatus::Error(msg), elapsed);
                                        continue;
                                    }
                                }
                            }

                            // Apply tags — deduplicated against existing tags
                            if !output.tags.is_empty() {
                                let new_tags: Vec<String> = output
                                    .tags
                                    .iter()
                                    .filter(|t| !existing_tags.contains(&t.to_lowercase()))
                                    .cloned()
                                    .collect();

                                if !new_tags.is_empty() {
                                    match engine.tag(&aid, &new_tags, false) {
                                        Ok(_) => {
                                            result.tags_applied += new_tags.len();
                                        }
                                        Err(e) => {
                                            let msg = format!("Failed to apply tags for {short_id}: {e}");
                                            result.errors.push(msg.clone());
                                        }
                                    }
                                }
                            }
                        }

                        result.described += 1;
                        result.results.push(DescribeResult {
                            asset_id: aid.clone(),
                            description: output.description.clone(),
                            tags: output.tags.clone(),
                            status: DescribeStatus::Described,
                        });
                        on_asset(&aid, &DescribeStatus::Described, elapsed);
                    }
                }
            }
        }

        Ok(result)
    }
}
