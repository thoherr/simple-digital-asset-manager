//! Per-command handlers — one `run_X_command` function per CLI command.
//!
//! Extracted from main.rs (which used to be 9.7 kLOC of CLI parsing +
//! dispatch + handlers all jumbled together) so main.rs can be a
//! near-pure dispatcher + CLI entrypoint. Each handler takes the
//! destructured command fields by value plus `json`, `log`, `verbosity`
//! for the small set of cross-cutting CLI flags it needs.
//!
//! Currently a single file. Per-command splitting (one file per
//! `run_X_command`) is a later cleanup if this file becomes painful to
//! navigate.

use std::path::PathBuf;

use maki::asset_service::AssetService;
use maki::catalog::Catalog;
use maki::cli_output::{format_duration, format_size, item_status};
#[cfg(feature = "ai")]
use maki::config::CatalogConfig;
use maki::device_registry::DeviceRegistry;
use maki::metadata_store::MetadataStore;
use maki::query::QueryEngine;

#[cfg(feature = "ai")]
use crate::FacesCommands;
use crate::{
    CollectionCommands, SavedSearchCommands, StackCommands, TagCommands, VolumeCommands,
};

/// Execute a parsed CLI command. Returns asset IDs produced by the command (for shell _ variable).
#[cfg(feature = "ai")]
pub fn run_faces_command(
    cmd: FacesCommands,
    json: bool,
    log: bool,
    #[allow(unused_variables)] verbosity: maki::Verbosity,
) -> anyhow::Result<()> {
    let catalog_root = maki::config::find_catalog_root()?;
    let config = maki::config::CatalogConfig::load(&catalog_root)?;
    let face_model_dir = maki::face::resolve_face_model_dir(&config.ai);

    // Shadow `cli` with a lightweight struct carrying the two flags the
    // faces subcommands read, so the extracted body can keep referencing
    // `cli.json` and `cli.log` unchanged.
    #[allow(dead_code)]
    struct Ctx { json: bool, log: bool }
    let cli = Ctx { json, log };

    match cmd {
        FacesCommands::Download => {
            maki::face::FaceDetector::download_models(&face_model_dir, |name, i, total| {
                eprintln!("  Downloading {name} ({i}/{total})...");
            })?;
            println!("Face models downloaded to {}", face_model_dir.display());
            Ok(())
        }
        FacesCommands::Status => {
            let exists = maki::face::FaceDetector::models_exist(&face_model_dir);
            let current_model = maki::face::RECOGNITION_MODEL.id;
            println!("Face model directory: {}", face_model_dir.display());
            println!("Models downloaded: {}", if exists { "yes" } else { "no" });
            println!("Current recognition model: {current_model}");

            let catalog = maki::catalog::Catalog::open(&catalog_root)?;
            let _ = maki::face_store::FaceStore::initialize(catalog.conn());
            let store = maki::face_store::FaceStore::new(catalog.conn());
            println!("Total faces detected: {}", store.total_faces());
            println!("Total people: {}", store.total_people());

            let by_model = store.face_counts_by_model().unwrap_or_default();
            if !by_model.is_empty() {
                println!("Faces by recognition model:");
                for (model, count) in &by_model {
                    let marker = if model == current_model { " (current)" } else { "" };
                    println!("  {model}: {count}{marker}");
                }
                let stale: u32 = by_model.iter()
                    .filter(|(m, _)| m != current_model)
                    .map(|(_, c)| *c).sum();
                if stale > 0 {
                    println!("  {stale} face(s) use a different recognition model and will be");
                    println!("  ignored by clustering. Re-run `maki faces detect --force` to update.");
                }
            }

            if cli.json {
                let json = serde_json::json!({
                    "model_dir": face_model_dir.to_string_lossy(),
                    "models_downloaded": exists,
                    "current_recognition_model": current_model,
                    "total_faces": store.total_faces(),
                    "total_people": store.total_people(),
                    "faces_by_model": by_model.iter().map(|(m, c)| serde_json::json!({"model": m, "count": c})).collect::<Vec<_>>(),
                });
                println!("{}", serde_json::to_string_pretty(&json)?);
            }
            Ok(())
        }
        FacesCommands::Detect { query, asset, volume, min_confidence, apply, force } => {
            if !maki::face::FaceDetector::models_exist(&face_model_dir) {
                anyhow::bail!(
                    "Face models not downloaded. Run 'maki faces download' first."
                );
            }

            if query.is_none() && asset.is_none() && volume.is_none() {
                anyhow::bail!(
                    "No scope specified. Use --query, --asset, or --volume to select assets.\n  \
                     Examples:\n    \
                     maki faces detect --query '' --apply     # all assets\n    \
                     maki faces detect --asset <id> --apply   # single asset\n    \
                     maki faces detect --volume <label> --apply"
                );
            }

            let engine = QueryEngine::new(&catalog_root);
            let service = AssetService::new(&catalog_root, verbosity, &config.preview);

            // Resolve target assets
            let catalog = maki::catalog::Catalog::open(&catalog_root)?;
            let asset_ids: Vec<String> = if let Some(ref aid) = asset {
                let full_id = catalog
                    .resolve_asset_id(aid)?
                    .ok_or_else(|| anyhow::anyhow!("no asset found matching '{aid}'"))?;
                vec![full_id]
            } else {
                let q = if let Some(ref query) = query {
                    let volume_part = volume.as_deref().map(|v| format!(" volume:{v}")).unwrap_or_default();
                    format!("{query}{volume_part}")
                } else if let Some(ref v) = volume {
                    format!("volume:{v}")
                } else {
                    "*".to_string()
                };
                let results = engine.search(&q)?;
                let mut seen = std::collections::HashSet::new();
                results.into_iter()
                    .filter(|r| seen.insert(r.asset_id.clone()))
                    .map(|r| r.asset_id)
                    .collect()
            };
            drop(catalog);

            let mut detector = maki::face::FaceDetector::load_with_provider(&face_model_dir, verbosity, &config.ai.execution_provider)?;

            let show_log = cli.log;
            let result = service.detect_faces(
                &asset_ids,
                &mut detector,
                min_confidence,
                force,
                apply,
                |aid, n, elapsed| {
                    if show_log {
                        let short_id = &aid[..8.min(aid.len())];
                        if n == 0 {
                            item_status(short_id, "skipped", Some(elapsed));
                        } else {
                            eprintln!(
                                "  {short_id} — {} face{} detected ({})",
                                n, if n == 1 { "" } else { "s" }, format_duration(elapsed)
                            );
                        }
                    }
                },
            )?;

            let total_faces = result.faces_detected;
            let total_assets = result.assets_processed;
            let total_skipped = result.assets_skipped;
            let errors = result.errors;

            if cli.json {
                let json = serde_json::json!({
                    "assets_processed": total_assets,
                    "assets_skipped": total_skipped,
                    "faces_detected": total_faces,
                    "errors": errors,
                    "dry_run": !apply,
                });
                println!("{}", serde_json::to_string_pretty(&json)?);
            } else {
                for err in &errors {
                    eprintln!("  {err}");
                }
                let mode = if apply { "Face detect" } else { "Face detect (dry run)" };
                let mut parts = vec![
                    format!("{total_assets} assets processed"),
                ];
                if total_skipped > 0 {
                    parts.push(format!("{total_skipped} skipped"));
                }
                parts.push(format!("{total_faces} faces detected"));
                if !errors.is_empty() {
                    parts.push(format!("{} errors", errors.len()));
                }
                println!("{mode}: {}", parts.join(", "));
                if !apply && total_faces > 0 {
                    println!("  Run with --apply to store face detections.");
                }
            }
            Ok(())
        }
        FacesCommands::Cluster { query, asset, volume, threshold, min_confidence, apply } => {
            let catalog = maki::catalog::Catalog::open(&catalog_root)?;
            let _ = maki::face_store::FaceStore::initialize(catalog.conn());
            let face_store = maki::face_store::FaceStore::new(catalog.conn());

            let thresh = threshold.unwrap_or(config.ai.face_cluster_threshold);
            let min_conf = min_confidence.unwrap_or(config.ai.face_min_confidence);

            // Resolve scope to asset IDs (same pattern as maki embed)
            let scoped_ids: Option<Vec<String>> = if query.is_some() || asset.is_some() || volume.is_some() {
                let engine = QueryEngine::new(&catalog_root);
                if let Some(ref a) = asset {
                    let full_id = catalog
                        .resolve_asset_id(a)?
                        .ok_or_else(|| anyhow::anyhow!("no asset found matching '{a}'"))?;
                    Some(vec![full_id])
                } else {
                    let q = if let Some(ref query) = query {
                        let volume_part = volume.as_deref().map(|v| format!(" volume:{v}")).unwrap_or_default();
                        format!("{query}{volume_part}")
                    } else if let Some(ref v) = volume {
                        format!("volume:{v}")
                    } else {
                        "*".to_string()
                    };
                    let rows = engine.search(&q)?;
                    Some(rows.into_iter().map(|r| r.asset_id).collect())
                }
            } else {
                None
            };
            let scope = scoped_ids.as_deref();

            if apply {
                let result = face_store.auto_cluster(thresh, min_conf, scope)?;
                // Persist faces/people YAML
                if let Err(e) = face_store.save_all_yaml(&catalog_root) {
                    eprintln!("  Warning: failed to save faces/people YAML: {e:#}");
                }
                if cli.json {
                    println!("{}", serde_json::to_string_pretty(&result)?);
                } else {
                    println!(
                        "Clustered: {} people created, {} faces assigned, {} singletons skipped",
                        result.people_created, result.faces_assigned, result.singletons_skipped
                    );
                }
            } else {
                let (clusters, _unassigned) = face_store.cluster_faces(thresh, min_conf, scope)?;
                let total_faces: usize = clusters.iter().map(|c| c.len()).sum();
                if cli.json {
                    println!("{}", serde_json::json!({
                        "dry_run": true,
                        "clusters": clusters.len(),
                        "faces_in_clusters": total_faces,
                        "cluster_sizes": clusters.iter().map(|c| c.len()).collect::<Vec<_>>(),
                        "threshold": thresh,
                        "min_confidence": min_conf,
                    }));
                } else {
                    println!("Cluster preview (threshold={thresh:.2}, min_confidence={min_conf:.2}):");
                    for (i, cluster) in clusters.iter().enumerate() {
                        println!("  Cluster {}: {} faces", i + 1, cluster.len());
                    }
                    println!("Total: {} clusters, {} faces", clusters.len(), total_faces);
                    println!("  Run with --apply to create people and assign faces.");
                }
            }
            Ok(())
        }
        FacesCommands::People => {
            let catalog = maki::catalog::Catalog::open(&catalog_root)?;
            let _ = maki::face_store::FaceStore::initialize(catalog.conn());
            let face_store = maki::face_store::FaceStore::new(catalog.conn());

            let people = face_store.list_people()?;
            if cli.json {
                let json_people: Vec<_> = people.iter().map(|(p, count)| {
                    serde_json::json!({
                        "id": p.id,
                        "name": p.name,
                        "representative_face_id": p.representative_face_id,
                        "face_count": count,
                    })
                }).collect();
                println!("{}", serde_json::to_string_pretty(&json_people)?);
            } else {
                if people.is_empty() {
                    println!("No people found. Run 'maki faces cluster --apply' to create people from detected faces.");
                } else {
                    println!("{:<10} {:<30} {}", "ID", "Name", "Faces");
                    for (person, count) in &people {
                        let short_id = &person.id[..8.min(person.id.len())];
                        let name = person.name.as_deref().unwrap_or("(unnamed)");
                        println!("{:<10} {:<30} {}", short_id, name, count);
                    }
                }
            }
            Ok(())
        }
        FacesCommands::Name { person_id, name } => {
            let catalog = maki::catalog::Catalog::open(&catalog_root)?;
            let _ = maki::face_store::FaceStore::initialize(catalog.conn());
            let face_store = maki::face_store::FaceStore::new(catalog.conn());

            // Resolve person ID prefix
            let full_id = resolve_person_id(&face_store, &person_id)?;
            face_store.name_person(&full_id, &name)?;
            let _ = face_store.save_all_yaml(&catalog_root);
            let short = &full_id[..8.min(full_id.len())];
            println!("Named person {short} as \"{name}\"");
            Ok(())
        }
        FacesCommands::Merge { target_id, source_id } => {
            let catalog = maki::catalog::Catalog::open(&catalog_root)?;
            let _ = maki::face_store::FaceStore::initialize(catalog.conn());
            let face_store = maki::face_store::FaceStore::new(catalog.conn());

            let target = resolve_person_id(&face_store, &target_id)?;
            let source = resolve_person_id(&face_store, &source_id)?;
            let moved = face_store.merge_people(&target, &source)?;
            let _ = face_store.save_all_yaml(&catalog_root);
            let short_t = &target[..8.min(target.len())];
            let short_s = &source[..8.min(source.len())];
            println!("Merged {short_s} into {short_t}: {moved} faces moved");
            Ok(())
        }
        FacesCommands::DeletePerson { person_id } => {
            let catalog = maki::catalog::Catalog::open(&catalog_root)?;
            let _ = maki::face_store::FaceStore::initialize(catalog.conn());
            let face_store = maki::face_store::FaceStore::new(catalog.conn());

            let full_id = resolve_person_id(&face_store, &person_id)?;
            face_store.delete_person(&full_id)?;
            let _ = face_store.save_all_yaml(&catalog_root);
            let short = &full_id[..8.min(full_id.len())];
            println!("Deleted person {short} (faces unassigned)");
            Ok(())
        }
        FacesCommands::Unassign { face_id } => {
            let catalog = maki::catalog::Catalog::open(&catalog_root)?;
            let _ = maki::face_store::FaceStore::initialize(catalog.conn());
            let face_store = maki::face_store::FaceStore::new(catalog.conn());

            // Resolve face ID prefix
            let full_id = resolve_face_id(&face_store, &face_id)?;
            face_store.unassign_face(&full_id)?;
            let _ = face_store.save_all_yaml(&catalog_root);
            let short = &full_id[..8.min(full_id.len())];
            println!("Unassigned face {short} from its person");
            Ok(())
        }
        FacesCommands::Clean { apply } => {
            let catalog = maki::catalog::Catalog::open(&catalog_root)?;
            let _ = maki::face_store::FaceStore::initialize(catalog.conn());
            let face_store = maki::face_store::FaceStore::new(catalog.conn());

            let count = face_store.count_unassigned_faces()?;
            if cli.json {
                if apply {
                    let deleted = face_store.delete_unassigned_faces()?;
                    if let Err(e) = face_store.save_all_yaml(&catalog_root) {
                        eprintln!("  Warning: failed to save faces/people YAML: {e:#}");
                    }
                    println!("{}", serde_json::json!({
                        "dry_run": false,
                        "deleted": deleted,
                    }));
                } else {
                    println!("{}", serde_json::json!({
                        "dry_run": true,
                        "would_delete": count,
                    }));
                }
            } else if count == 0 {
                println!("No unassigned faces to delete.");
            } else if apply {
                let deleted = face_store.delete_unassigned_faces()?;
                if let Err(e) = face_store.save_all_yaml(&catalog_root) {
                    eprintln!("  Warning: failed to save faces/people YAML: {e:#}");
                }
                println!("Deleted {deleted} unassigned face record(s).");
            } else {
                println!("Dry run — would delete {count} unassigned face record(s).");
                println!("  Run with --apply to actually delete.");
            }
            Ok(())
        }
        FacesCommands::DumpAligned { query, asset, output, limit } => {
            let catalog = maki::catalog::Catalog::open(&catalog_root)?;
            let engine = QueryEngine::new(&catalog_root);

            // Resolve scope to asset IDs.
            let scoped_ids: Vec<String> = if let Some(ref a) = asset {
                let full_id = catalog
                    .resolve_asset_id(a)?
                    .ok_or_else(|| anyhow::anyhow!("no asset found matching '{a}'"))?;
                vec![full_id]
            } else {
                let q = query.as_deref().unwrap_or("*");
                let rows = engine.search(q)?;
                rows.into_iter().map(|r| r.asset_id).collect()
            };
            eprintln!("Scope: {} asset(s)", scoped_ids.len());

            std::fs::create_dir_all(&output)?;
            // Use the same path-resolution that `faces detect` uses —
            // falls back to the smart/regular preview when the original
            // file is on an offline volume. Matches how embeddings are
            // computed so the aligned crops we save reflect what the
            // model actually sees.
            let volumes = maki::device_registry::DeviceRegistry::new(&catalog_root).list()?;
            let online_volumes = maki::models::Volume::online_map(&volumes);
            let preview_config = config.preview.clone();
            let preview_gen = maki::preview::PreviewGenerator::new(
                &catalog_root, maki::Verbosity::default(), &preview_config,
            );
            let asset_service = maki::asset_service::AssetService::new(
                &catalog_root, maki::Verbosity::default(), &preview_config,
            );
            let face_model_dir = maki::face::resolve_face_model_dir(&config.ai);
            let mut detector = maki::face::FaceDetector::load(&face_model_dir, maki::Verbosity::default())?;

            let mut saved = 0usize;
            let mut skipped_no_image = 0usize;
            let mut skipped_no_faces = 0usize;
            for aid in &scoped_ids {
                if limit > 0 && saved >= limit { break; }
                let details = match engine.show(aid) {
                    Ok(d) => d,
                    Err(_) => continue,
                };
                let full_path = match asset_service.find_image_for_ai(&details, &preview_gen, &online_volumes) {
                    Some(p) => p,
                    None => { skipped_no_image += 1; continue; }
                };

                let detected = detector.detect_faces(&full_path, 0.5).unwrap_or_default();
                if detected.is_empty() { skipped_no_faces += 1; continue; }

                let img = match image::open(&full_path) {
                    Ok(i) => i,
                    Err(_) => continue,
                };

                for (i, df) in detected.iter().enumerate() {
                    if limit > 0 && saved >= limit { break; }
                    let aligned = maki::face::align_face_to_arcface(&img, &df.landmarks);
                    let short_asset = &aid[..8.min(aid.len())];
                    let path = output.join(format!("{short_asset}_{i}.jpg"));
                    aligned.save(&path)?;
                    eprintln!("  saved {} (conf={:.2})", path.display(), df.confidence);
                    saved += 1;
                }
            }
            println!(
                "Saved {saved} aligned face crop(s) to {}",
                output.display()
            );
            if skipped_no_image + skipped_no_faces > 0 {
                println!(
                    "Skipped: {skipped_no_image} no accessible image, {skipped_no_faces} no faces detected"
                );
            }
            Ok(())
        }
        FacesCommands::Similarity { query, asset, volume, min_confidence, top, all } => {
            let catalog = maki::catalog::Catalog::open(&catalog_root)?;
            let _ = maki::face_store::FaceStore::initialize(catalog.conn());
            let face_store = maki::face_store::FaceStore::new(catalog.conn());

            // Resolve scope (same pattern as cluster)
            let scoped_ids: Option<Vec<String>> = if query.is_some() || asset.is_some() || volume.is_some() {
                let engine = QueryEngine::new(&catalog_root);
                if let Some(ref a) = asset {
                    let full_id = catalog
                        .resolve_asset_id(a)?
                        .ok_or_else(|| anyhow::anyhow!("no asset found matching '{a}'"))?;
                    Some(vec![full_id])
                } else {
                    let q = if let Some(ref query) = query {
                        let volume_part = volume.as_deref().map(|v| format!(" volume:{v}")).unwrap_or_default();
                        format!("{query}{volume_part}")
                    } else if let Some(ref v) = volume {
                        format!("volume:{v}")
                    } else {
                        "*".to_string()
                    };
                    let rows = engine.search(&q)?;
                    Some(rows.into_iter().map(|r| r.asset_id).collect())
                }
            } else {
                None
            };

            let faces = face_store.face_embeddings_scoped(scoped_ids.as_deref())?;
            // Filter by assignment + confidence; also look up asset_id for each
            // kept face so the output can point back to a real asset.
            let filtered: Vec<(String, String, String, Vec<f32>, f32)> = faces.into_iter()
                .filter(|(_, pid, _, conf, _)| (all || pid.is_none()) && *conf >= min_confidence)
                .filter_map(|(id, pid, emb, conf, _model)| {
                    let asset_id = face_store.get_face(&id).ok().flatten().map(|f| f.asset_id)?;
                    Some((id, pid.unwrap_or_default(), asset_id, emb, conf))
                })
                .collect();

            let n = filtered.len();
            if n < 2 {
                if cli.json {
                    println!("{}", serde_json::json!({"faces": n, "pairs": 0}));
                } else {
                    println!("Not enough faces to analyze ({n}). Need at least 2.");
                }
                return Ok(());
            }

            // Compute all pairwise similarities (embeddings are at tuple index 3)
            let mut sims: Vec<f32> = Vec::with_capacity(n * (n - 1) / 2);
            let mut per_face: Vec<Vec<(usize, f32)>> = vec![Vec::new(); n];
            for i in 0..n {
                for j in (i + 1)..n {
                    let s = maki::ai::cosine_similarity(&filtered[i].3, &filtered[j].3);
                    sims.push(s);
                    per_face[i].push((j, s));
                    per_face[j].push((i, s));
                }
            }
            sims.sort_by(|a, b| a.partial_cmp(b).unwrap());

            // Stats
            let pct = |p: f32| -> f32 {
                let idx = ((sims.len() as f32 - 1.0) * p).round() as usize;
                sims[idx]
            };
            let mean: f32 = sims.iter().sum::<f32>() / sims.len() as f32;
            let min = sims[0];
            let max = *sims.last().unwrap();
            let p10 = pct(0.10);
            let p25 = pct(0.25);
            let p50 = pct(0.50);
            let p75 = pct(0.75);
            let p90 = pct(0.90);
            let p95 = pct(0.95);
            let p99 = pct(0.99);

            // Histogram: 10 buckets from min to max
            let bucket_count = 10;
            let mut buckets = vec![0u32; bucket_count];
            let range = (max - min).max(1e-6);
            for &s in &sims {
                let mut b = (((s - min) / range) * bucket_count as f32) as usize;
                if b >= bucket_count { b = bucket_count - 1; }
                buckets[b] += 1;
            }

            if cli.json {
                println!("{}", serde_json::json!({
                    "faces": n,
                    "pairs": sims.len(),
                    "stats": {
                        "min": min, "max": max, "mean": mean,
                        "p10": p10, "p25": p25, "p50": p50,
                        "p75": p75, "p90": p90, "p95": p95, "p99": p99,
                    },
                    "histogram": {
                        "min": min, "max": max, "buckets": buckets,
                    },
                }));
            } else {
                let mode = if all { "all faces" } else { "unassigned faces" };
                println!("Face similarity analysis — {n} {mode}, {} pairs (min_confidence={min_confidence:.2})", sims.len());
                println!();
                println!("Pairwise cosine similarity:");
                println!("  min:    {min:.3}");
                println!("  p10:    {p10:.3}");
                println!("  p25:    {p25:.3}");
                println!("  median: {p50:.3}");
                println!("  mean:   {mean:.3}");
                println!("  p75:    {p75:.3}");
                println!("  p90:    {p90:.3}");
                println!("  p95:    {p95:.3}");
                println!("  p99:    {p99:.3}");
                println!("  max:    {max:.3}");
                println!();
                println!("Histogram ({bucket_count} buckets, {min:.2}–{max:.2}):");
                let max_count = *buckets.iter().max().unwrap_or(&1);
                for (i, &count) in buckets.iter().enumerate() {
                    let lo = min + (i as f32) * range / bucket_count as f32;
                    let hi = min + ((i + 1) as f32) * range / bucket_count as f32;
                    let bar_width = (count as f32 * 40.0 / max_count as f32) as usize;
                    let bar: String = "█".repeat(bar_width);
                    println!("  {lo:.3}–{hi:.3}  {count:>6}  {bar}");
                }
                println!();
                println!("Interpretation:");
                println!("  • A bimodal distribution (two humps) means the model separates people well.");
                println!("    Pick a threshold in the gap between humps.");
                println!("  • A single hump or flat distribution means embeddings are not discriminating.");
                println!("    Check face model quality, face crop size, or filter by higher --min-confidence.");

                if top > 0 {
                    println!();
                    println!("Top-{top} nearest neighbors per face (format: face_id/asset_id):");
                    for i in 0..n {
                        per_face[i].sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
                        let short_face = &filtered[i].0[..8.min(filtered[i].0.len())];
                        let short_asset = &filtered[i].2[..8.min(filtered[i].2.len())];
                        let person = if filtered[i].1.is_empty() { "unassigned".to_string() } else { format!("person={}", &filtered[i].1[..8.min(filtered[i].1.len())]) };
                        println!("  {short_face}/{short_asset} (conf={:.2}, {person}):", filtered[i].4);
                        for (j, s) in per_face[i].iter().take(top) {
                            let short_jf = &filtered[*j].0[..8.min(filtered[*j].0.len())];
                            let short_ja = &filtered[*j].2[..8.min(filtered[*j].2.len())];
                            let person_j = if filtered[*j].1.is_empty() { "unassigned".to_string() } else { format!("person={}", &filtered[*j].1[..8.min(filtered[*j].1.len())]) };
                            println!("    → {short_jf}/{short_ja} [{s:.3}] ({person_j})");
                        }
                    }
                }
            }
            Ok(())
        }
        FacesCommands::Export => {
            let catalog = maki::catalog::Catalog::open(&catalog_root)?;
            let _ = maki::face_store::FaceStore::initialize(catalog.conn());
            let face_store = maki::face_store::FaceStore::new(catalog.conn());

            // Export faces + people YAML
            face_store.save_all_yaml(&catalog_root)?;
            let faces_file = face_store.export_all_faces()?;
            let people_file = face_store.export_all_people()?;

            // Export ArcFace embedding binaries
            let mut arcface_count = 0u32;
            for face in &faces_file.faces {
                if let Ok(Some(emb)) = face_store.get_face_embedding(&face.id) {
                    if !emb.is_empty() {
                        if let Err(e) = maki::face_store::write_arcface_binary(&catalog_root, &face.id, &emb) {
                            eprintln!("  Warning: {}: {e:#}", &face.id[..8.min(face.id.len())]);
                        } else {
                            arcface_count += 1;
                        }
                    }
                }
            }

            if cli.json {
                println!("{}", serde_json::json!({
                    "faces": faces_file.faces.len(),
                    "people": people_file.people.len(),
                    "arcface_binaries": arcface_count,
                }));
            } else {
                println!("Exported {} faces, {} people to YAML", faces_file.faces.len(), people_file.people.len());
                println!("Exported {arcface_count} ArcFace embedding binaries");
            }
            Ok(())
        }
    }
}

/// Print a warning to stderr listing tags that were modified during
/// keyword-text sanitization (XML entity decode, comma/semicolon stripping).
/// Helps the user find tags they may want to rename at the source.
pub fn report_sanitized_tags(changes: &[(String, String)]) {
    if changes.is_empty() {
        return;
    }
    eprintln!(
        "\nwarning: {} tag(s) were sanitized for Lightroom/Capture One compatibility.",
        changes.len(),
    );
    eprintln!("         Consider renaming them in your catalog (see `maki tag rename`):");
    for (before, after) in changes {
        if after.is_empty() {
            eprintln!("         - {before:?} -> (skipped: empty after sanitize)");
        } else {
            eprintln!("         - {before:?} -> {after:?}");
        }
    }
    eprintln!();
}

/// Merge trailing asset IDs (from shell variable expansion) into query/asset.
/// Single ID → asset; multiple IDs → `id:xxx id:yyy` query.
#[cfg(any(feature = "pro", feature = "ai"))]
pub fn merge_trailing_ids(
    query: Option<String>,
    asset: Option<String>,
    asset_ids: &[String],
) -> (Option<String>, Option<String>) {
    if asset_ids.is_empty() {
        return (query, asset);
    }
    if asset_ids.len() == 1 {
        return (query, Some(asset_ids[0].clone()));
    }
    let id_query = asset_ids.iter().map(|id| format!("id:{id}")).collect::<Vec<_>>().join(" ");
    let combined = match query {
        Some(q) => format!("{q} {id_query}"),
        None => id_query,
    };
    (Some(combined), asset)
}

/// Resolve a person ID prefix to a full ID.
#[cfg(feature = "ai")]
pub fn resolve_person_id(face_store: &maki::face_store::FaceStore, prefix: &str) -> anyhow::Result<String> {
    let people = face_store.list_people()?;
    let matches: Vec<_> = people
        .iter()
        .filter(|(p, _)| p.id.starts_with(prefix))
        .collect();
    match matches.len() {
        0 => anyhow::bail!("no person found matching '{prefix}'"),
        1 => Ok(matches[0].0.id.clone()),
        _ => anyhow::bail!("ambiguous person ID prefix '{prefix}' — matches {} people", matches.len()),
    }
}

/// Resolve a face ID prefix to a full ID.
#[cfg(feature = "ai")]
pub fn resolve_face_id(face_store: &maki::face_store::FaceStore, prefix: &str) -> anyhow::Result<String> {
    // Try exact match first
    if let Ok(Some(_)) = face_store.get_face(prefix) {
        return Ok(prefix.to_string());
    }
    // Fall back to prefix search via all faces
    let conn = face_store.conn();
    let mut stmt = conn.prepare("SELECT id FROM faces WHERE id LIKE ?1")?;
    let ids: Vec<String> = stmt
        .query_map(rusqlite::params![format!("{prefix}%")], |row| row.get(0))?
        .collect::<Result<Vec<_>, _>>()?;
    match ids.len() {
        0 => anyhow::bail!("no face found matching '{prefix}'"),
        1 => Ok(ids[0].clone()),
        _ => anyhow::bail!("ambiguous face ID prefix '{prefix}' — matches {} faces", ids.len()),
    }
}

pub fn print_stats_human(stats: &maki::catalog::CatalogStats) {
    let o = &stats.overview;
    println!("Catalog Overview");
    println!("  Assets:    {}", o.assets);
    println!("  Variants:  {}", o.variants);
    println!("  Recipes:   {}", o.recipes);
    println!("  Volumes:   {} ({} online, {} offline)", o.volumes_total, o.volumes_online, o.volumes_offline);
    println!("  Total size: {}", format_size(o.total_size));

    if let Some(types) = &stats.types {
        println!("\nAsset Types");
        for t in &types.asset_types {
            println!("  {:<12} {:>6}  ({:.1}%)", t.asset_type, t.count, t.percentage);
        }
        if !types.variant_formats.is_empty() {
            println!("\nVariant Formats");
            for f in &types.variant_formats {
                println!("  {:<12} {:>6}", f.format, f.count);
            }
        }
        if !types.recipe_formats.is_empty() {
            println!("\nRecipe Formats");
            for f in &types.recipe_formats {
                println!("  {:<12} {:>6}", f.format, f.count);
            }
        }
    }

    if let Some(volumes) = &stats.volumes {
        println!("\nVolumes");
        for v in volumes {
            let status = if v.is_online { "online" } else { "offline" };
            if let Some(purpose) = &v.purpose {
                println!("  {} [{}] [{}]", v.label, status, purpose);
            } else {
                println!("  {} [{}]", v.label, status);
            }
            println!("    Assets: {}  Variants: {}  Recipes: {}", v.assets, v.variants, v.recipes);
            println!("    Size: {}  Directories: {}", format_size(v.size), v.directories);
            if !v.formats.is_empty() {
                println!("    Formats: {}", v.formats.join(", "));
            }
            println!("    Verified: {}/{} ({:.1}%)", v.verified_count, v.total_locations, v.verification_pct);
            if let Some(oldest) = &v.oldest_verified_at {
                println!("    Oldest verification: {oldest}");
            }
        }
    }

    if let Some(tags) = &stats.tags {
        println!("\nTags");
        println!("  Unique tags:     {}", tags.unique_tags);
        println!("  Tagged assets:   {}", tags.tagged_assets);
        println!("  Untagged assets: {}", tags.untagged_assets);
        if !tags.top_tags.is_empty() {
            println!("\n  Top Tags");
            for t in &tags.top_tags {
                println!("    {:<20} {:>4}", t.tag, t.count);
            }
        }
    }

    if let Some(v) = &stats.verified {
        println!("\nVerification");
        println!("  Total locations:    {}", v.total_locations);
        println!("  Verified:           {}", v.verified_locations);
        println!("  Unverified:         {}", v.unverified_locations);
        println!("  Coverage:           {:.1}%", v.coverage_pct);
        if let Some(oldest) = &v.oldest_verified_at {
            println!("  Oldest verified:    {oldest}");
        }
        if let Some(newest) = &v.newest_verified_at {
            println!("  Newest verified:    {newest}");
        }
        if !v.per_volume.is_empty() {
            println!("\n  Per Volume");
            for pv in &v.per_volume {
                let status = if pv.is_online { "online" } else { "offline" };
                let purpose_tag = pv.purpose.as_ref().map(|p| format!(" [{}]", p)).unwrap_or_default();
                println!(
                    "    {} [{}]{}: {}/{} ({:.1}%)",
                    pv.label, status, purpose_tag, pv.verified, pv.locations, pv.coverage_pct
                );
            }
        }
    }
}

/// Render a status report as human-readable text.
///
/// Sections roll up from `StatusReport`'s nested structs in this order:
/// Catalog overview → Cleanup needs → Pending work → Backup coverage →
/// Volumes. Each item is prefixed `✓` (clean / ok) or `✗` (action item)
/// with a one-line `→ command` suggestion on every `✗` so the user knows
/// what to run next without consulting docs.
pub fn print_status_human(report: &maki::status::StatusReport) {
    use maki::cli_output::format_size;

    println!("MAKI catalog status — {}", report.catalog_root);

    // ── Catalog overview ─────────────────────────────────
    println!("\nCatalog");
    let schema = if report.catalog.schema_version == report.catalog.schema_current {
        format!("v{} (current)", report.catalog.schema_version)
    } else {
        format!(
            "v{} (run `maki migrate` — current is v{})",
            report.catalog.schema_version, report.catalog.schema_current
        )
    };
    println!("  Schema:   {schema}");
    println!(
        "  Counts:   {} assets · {} variants · {} recipes · {} file locations",
        report.catalog.assets,
        report.catalog.variants,
        report.catalog.recipes,
        report.catalog.file_locations,
    );
    let online = report.volumes.iter().filter(|v| v.is_online).count();
    let offline = report.volumes.len() - online;
    println!(
        "  Storage:  {} across {} volume(s) ({} online, {} offline)",
        format_size(report.catalog.total_bytes),
        report.volumes.len(),
        online,
        offline,
    );

    // ── Cleanup needs ────────────────────────────────────
    println!("\nCleanup");
    let c = &report.cleanup;
    let cleanup_actions = [
        (c.locationless_variants, "locationless variant(s)"),
        (c.orphaned_assets, "orphaned asset(s)"),
        (c.orphaned_previews, "orphaned preview(s) on disk"),
        (c.orphaned_smart_previews, "orphaned smart preview(s) on disk"),
        (c.orphaned_embeddings, "orphaned embedding file(s) on disk"),
        (c.orphaned_face_files, "orphaned face file(s) on disk"),
    ];
    let any_cleanup = cleanup_actions.iter().any(|(n, _)| *n > 0);
    if !any_cleanup {
        println!("  ✓ no cleanup needed");
    } else {
        for (n, label) in &cleanup_actions {
            if *n > 0 {
                println!(
                    "  ✗ {n} {label:<42} → maki cleanup --apply"
                );
            }
        }
    }

    // ── Pending work ─────────────────────────────────────
    println!("\nPending work");
    let p = &report.pending;
    let mut pending_lines = 0;
    if p.pending_writebacks_online > 0 {
        if p.writeback_enabled {
            println!(
                "  ✗ {} pending XMP writeback(s) on online volume(s){:<11} → maki writeback",
                p.pending_writebacks_online, ""
            );
        } else {
            // Auto-flush off (the safety-net default). Manual `maki
            // writeback` runs regardless of the config flag, so the hint
            // points straight at it without a config-change detour.
            println!(
                "  ✗ {} pending XMP writeback(s){:<23} → maki writeback  (auto-flush off; this is the manual flush)",
                p.pending_writebacks_online, ""
            );
        }
        pending_lines += 1;
    }
    if p.pending_writebacks_offline > 0 {
        println!(
            "  ✗ {} pending XMP writeback(s) on offline volume(s){:<6} → mount the volumes, then `maki writeback`",
            p.pending_writebacks_offline, ""
        );
        pending_lines += 1;
    }
    if let Some(n) = p.assets_without_embedding {
        if n > 0 {
            println!(
                "  ✗ {} asset(s) without an embedding{:<23} → maki embed",
                n, ""
            );
            pending_lines += 1;
        }
    }
    if let Some(n) = p.assets_without_face_scan {
        if n > 0 {
            println!(
                "  ✗ {} asset(s) unscanned for faces{:<24} → maki faces detect",
                n, ""
            );
            pending_lines += 1;
        }
    }
    if pending_lines == 0 {
        println!("  ✓ nothing pending");
    }

    // ── Backup coverage ──────────────────────────────────
    println!("\nBackup coverage");
    let b = &report.backup;
    if b.total_assets == 0 {
        println!("  (catalog is empty)");
    } else if b.at_risk == 0 {
        println!(
            "  ✓ all {} asset(s) have ≥{} copies",
            b.total_assets, b.min_copies
        );
    } else {
        let pct = (b.at_risk as f64 / b.total_assets as f64) * 100.0;
        println!(
            "  ✗ {} of {} asset(s) ({:.1}%) have fewer than {} copies → maki backup-status --at-risk",
            b.at_risk, b.total_assets, pct, b.min_copies
        );
    }

    // ── Volumes ──────────────────────────────────────────
    if !report.volumes.is_empty() {
        println!("\nVolumes");
        for v in &report.volumes {
            let dot = if v.is_online { "●" } else { "○" };
            let purpose = v
                .purpose
                .as_deref()
                .map(|p| format!(" [{p}]"))
                .unwrap_or_default();
            let status = if v.is_online { "" } else { " (offline)" };
            println!(
                "  {} {:<18} {:<28} {} asset(s), {}{}{}",
                dot,
                v.label,
                v.mount_point,
                v.asset_count,
                format_size(v.size_bytes),
                purpose,
                status,
            );
        }
    }
}

pub fn print_backup_status_human(result: &maki::catalog::BackupStatusResult) {
    println!("Backup Status ({})", result.scope);
    println!("{}", "=".repeat(40));
    println!();
    println!("Total assets:          {:>8}", result.total_assets);
    println!("Total variants:        {:>8}", result.total_variants);
    println!("Total file locations:  {:>8}", result.total_file_locations);

    if !result.purpose_coverage.is_empty() {
        println!();
        println!("Coverage by volume purpose:");
        for pc in &result.purpose_coverage {
            // Capitalize first letter for display
            let display_purpose = {
                let mut chars = pc.purpose.chars();
                match chars.next() {
                    None => String::new(),
                    Some(c) => c.to_uppercase().to_string() + chars.as_str(),
                }
            };
            println!(
                "  {:<10} ({} volume{}):  {:>6} assets ({:.1}%)",
                display_purpose,
                pc.volume_count,
                if pc.volume_count == 1 { "" } else { "s" },
                pc.asset_count,
                pc.asset_percentage,
            );
        }
    }

    println!();
    println!("Volume distribution:");
    for bucket in &result.location_distribution {
        if bucket.asset_count == 0 {
            continue;
        }
        let label = match bucket.volume_count.as_str() {
            "0" => "0 volumes (orphaned):",
            "1" => "1 volume only:",
            "2" => "2 volumes:",
            _ => "3+ volumes:",
        };
        let at_risk = if bucket.volume_count == "0" || bucket.volume_count == "1" {
            "  <- AT RISK"
        } else {
            ""
        };
        println!("  {:<26} {:>6} assets{}", label, bucket.asset_count, at_risk);
    }

    if result.at_risk_count > 0 {
        println!();
        println!(
            "At-risk assets ({} on fewer than {} volume{}):",
            result.at_risk_count,
            result.min_copies,
            if result.min_copies == 1 { "" } else { "s" },
        );
        println!("  Use 'maki backup-status --at-risk' to list them");
        println!("  Use 'maki backup-status --at-risk -q' for asset IDs (pipeable)");
    } else {
        println!();
        println!(
            "All assets exist on {} or more volume{}. No at-risk assets.",
            result.min_copies,
            if result.min_copies == 1 { "" } else { "s" },
        );
    }

    if let Some(ref detail) = result.volume_detail {
        println!();
        let purpose_tag = detail.purpose.as_ref().map(|p| format!(" [{}]", p)).unwrap_or_default();
        println!("Volume detail: {}{}", detail.volume_label, purpose_tag);
        println!("  Present: {} / {} ({:.1}%)", detail.present_count, detail.total_scoped, detail.coverage_pct);
        println!("  Missing: {}", detail.missing_count);
    }

    if !result.volume_gaps.is_empty() {
        println!();
        println!("Volume gaps:");
        for gap in &result.volume_gaps {
            let purpose_tag = gap.purpose.as_ref().map(|p| format!(" [{}]", p)).unwrap_or_default();
            println!("  {}{}:  missing {} assets", gap.volume_label, purpose_tag, gap.missing_count);
        }
    }
}

/// Extracted body of `Commands::Import`. Kept as a free function so the
/// `run_command` match arm stays readable — the body is identical to the
/// inline version it replaced.
pub fn run_import_command(
    paths: Vec<String>,
    volume: Option<String>,
    profile: Option<String>,
    include: Vec<String>,
    skip: Vec<String>,
    add_tags: Vec<String>,
    dry_run: bool,
    auto_group: bool,
    smart: bool,
    #[cfg(feature = "ai")] embed: bool,
    #[cfg(feature = "pro")] describe: bool,
    json: bool,
    log: bool,
    #[allow(unused_variables)] verbosity: maki::Verbosity,
) -> anyhow::Result<()> {
    use maki::asset_service::{ImportEvent, ImportPhase, ImportRequest};

    let (catalog_root, config) = maki::config::load_config()?;

    // Canonicalize input paths against the working directory. The workflow
    // takes already-canonicalised paths so it can be called from contexts
    // (like the web handler) that don't have a meaningful CWD.
    let canonical_paths: Vec<PathBuf> = paths
        .iter()
        .map(|p| std::fs::canonicalize(p).unwrap_or_else(|_| PathBuf::from(p)))
        .collect();

    let req = ImportRequest {
        paths: canonical_paths,
        volume_label: volume,
        profile: profile.clone(),
        include,
        skip,
        add_tags,
        dry_run,
        smart,
        auto_group,
        #[cfg(feature = "ai")]
        embed,
        #[cfg(not(feature = "ai"))]
        embed: false,
        #[cfg(feature = "pro")]
        describe,
        #[cfg(not(feature = "pro"))]
        describe: false,
    };

    if verbosity.verbose {
        eprintln!(
            "  Import: {} file(s){}",
            req.paths.len(),
            req.volume_label.as_ref().map(|v| format!(" on volume \"{v}\"")).unwrap_or_default()
        );
        if let Some(ref p) = profile {
            eprintln!("  Import: using profile \"{}\"", p);
        }
        if !req.add_tags.is_empty() {
            eprintln!("  Import: extra tags: {}", req.add_tags.join(", "));
        }
        if smart {
            eprintln!("  Import: smart previews enabled");
        }
    }

    let service = AssetService::new(&catalog_root, verbosity, &config.preview);
    let workflow_result = service.import_workflow(&req, &config, |evt| {
        // Per-event progress: only emit per-item lines in --log mode. Phase
        // boundaries print a one-line announcement when --log is on (or
        // verbose); skipped phases always print so the user knows why a
        // phase didn't run.
        match evt {
            ImportEvent::PhaseStarted(phase) => {
                if log && phase != ImportPhase::Import {
                    eprintln!("  Phase: {}", phase.label());
                }
            }
            ImportEvent::PhaseSkipped { phase, reason } => {
                eprintln!("  Skipping {} phase: {reason}", phase.label());
            }
            ImportEvent::File { path, status, elapsed } => {
                if !log { return; }
                use maki::asset_service::FileStatus;
                let label = match status {
                    FileStatus::Imported => "imported",
                    FileStatus::LocationAdded => "location added",
                    FileStatus::Skipped => "skipped",
                    FileStatus::RecipeAttached => "recipe",
                    FileStatus::RecipeLocationAdded => "recipe location added",
                    FileStatus::RecipeUpdated => "recipe updated",
                };
                let name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or_else(|| path.to_str().unwrap_or("?"));
                item_status(name, label, Some(elapsed));
            }
            #[cfg(feature = "ai")]
            ImportEvent::Embed { asset_id, status } => {
                if !log { return; }
                let short = &asset_id[..8.min(asset_id.len())];
                match status {
                    maki::asset_service::EmbedStatus::Embedded => {
                        eprintln!("  {short} — embedded");
                    }
                    maki::asset_service::EmbedStatus::Error(msg) => {
                        eprintln!("  {short} — embed error: {msg}");
                    }
                    maki::asset_service::EmbedStatus::Skipped(_) => {}
                }
            }
            #[cfg(feature = "pro")]
            ImportEvent::Describe { asset_id, status, elapsed } => {
                if !log { return; }
                let short = &asset_id[..8.min(asset_id.len())];
                match status {
                    maki::vlm::DescribeStatus::Described => {
                        item_status(short, "described", Some(elapsed));
                    }
                    maki::vlm::DescribeStatus::Skipped(msg) => {
                        eprintln!("  {short} — skipped: {msg}");
                    }
                    maki::vlm::DescribeStatus::Error(msg) => {
                        eprintln!("  {short} — error: {msg}");
                    }
                }
            }
        }
    })?;

    // Format output. The workflow returns a single bundle; the CLI just
    // unpacks it into JSON or human text. Frontend-specific concern only.
    let result = &workflow_result.import;
    let auto_group_result = workflow_result.auto_group.as_ref();
    #[cfg(feature = "ai")]
    let embed_result = workflow_result.embed.as_ref();
    #[cfg(feature = "pro")]
    let describe_result = workflow_result.describe.as_ref();

    if json {
        #[allow(unused_mut)]
        let mut json_val = serde_json::to_value(result)?;
        if let Some(ag) = auto_group_result {
            json_val["auto_group"] = serde_json::to_value(ag)?;
        }
        #[cfg(feature = "ai")]
        if let Some(er) = embed_result {
            json_val["embeddings_generated"] = serde_json::json!(er.embedded);
            json_val["embeddings_skipped"] = serde_json::json!(er.skipped);
        }
        #[cfg(feature = "pro")]
        if let Some(dr) = describe_result {
            json_val["descriptions_generated"] = serde_json::json!(dr.described);
            json_val["descriptions_skipped"] = serde_json::json!(dr.skipped);
            if dr.tags_applied > 0 {
                json_val["describe_tags_applied"] = serde_json::json!(dr.tags_applied);
            }
        }
        println!("{}", serde_json::to_string_pretty(&json_val)?);
    } else {
        let mut parts: Vec<String> = Vec::new();
        if result.imported > 0          { parts.push(format!("{} imported", result.imported)); }
        if result.skipped > 0           { parts.push(format!("{} skipped", result.skipped)); }
        if result.locations_added > 0   { parts.push(format!("{} location(s) added", result.locations_added)); }
        if result.recipes_attached > 0  { parts.push(format!("{} recipe(s) attached", result.recipes_attached)); }
        if result.recipes_location_added > 0 { parts.push(format!("{} recipe location(s) added", result.recipes_location_added)); }
        if result.recipes_updated > 0   { parts.push(format!("{} recipe(s) updated", result.recipes_updated)); }
        if result.previews_generated > 0       { parts.push(format!("{} preview(s) generated", result.previews_generated)); }
        if result.smart_previews_generated > 0 { parts.push(format!("{} smart preview(s) generated", result.smart_previews_generated)); }
        #[cfg(feature = "ai")]
        if let Some(er) = embed_result {
            if er.embedded > 0 { parts.push(format!("{} embedding(s) generated", er.embedded)); }
        }
        #[cfg(feature = "pro")]
        if let Some(dr) = describe_result {
            if dr.described > 0 { parts.push(format!("{} described", dr.described)); }
        }
        if parts.is_empty() {
            println!("Import: nothing to import");
        } else if dry_run {
            println!("Dry run — would import: {}", parts.join(", "));
        } else {
            println!("Import complete: {}", parts.join(", "));
        }

        if let Some(ag) = auto_group_result {
            if log {
                for group in &ag.groups {
                    let short_id = &group.target_id[..8.min(group.target_id.len())];
                    eprintln!(
                        "  {} — {} asset(s) → target {short_id}",
                        group.stem,
                        group.asset_ids.len(),
                    );
                }
            }
            println!(
                "Auto-group: {} stem group(s), {} donor(s) {}, {} variant(s) moved",
                ag.groups.len(),
                ag.total_donors_merged,
                if dry_run { "would merge" } else { "merged" },
                ag.total_variants_moved,
            );
        }

        // Tier-A hints: tell users about post-import phases they didn't
        // engage. The `*_result.is_none()` check covers both "didn't opt in"
        // and "opted in but the phase was skipped" (model missing, VLM
        // unreachable). Only emit on a real (non-dry) successful import.
        if !dry_run && result.imported > 0 {
            #[cfg(feature = "ai")]
            {
                if !embed && !config.import.embeddings {
                    println!(
                        "  Tip: run 'maki embed' to generate visual-similarity \
                         embeddings (or pass --embed on import / set \
                         [import] embeddings = true in maki.toml)."
                    );
                }
            }
            #[cfg(feature = "pro")]
            {
                if !describe && !config.import.descriptions {
                    println!(
                        "  Tip: run 'maki describe' to generate VLM \
                         descriptions (or pass --describe on import / set \
                         [import] descriptions = true in maki.toml)."
                    );
                }
            }
        }
    }
    Ok(())
}

/// Extracted body of `Commands::AutoTag`. See `run_import_command` for the
/// extraction pattern (Ctx-shadow trick + body kept verbatim).
#[cfg(feature = "ai")]
pub fn run_auto_tag_command(
    query: Option<String>,
    asset: Option<String>,
    volume: Option<String>,
    model: Option<String>,
    threshold: Option<f32>,
    labels: Option<String>,
    apply: bool,
    download: bool,
    remove_model: bool,
    list_models: bool,
    list_labels: bool,
    similar: Option<String>,
    asset_ids: Vec<String>,
    json: bool,
    log: bool,
    #[allow(unused_variables)] verbosity: maki::Verbosity,
) -> anyhow::Result<()> {
    #[allow(dead_code)]
    struct Ctx { json: bool, log: bool }
    let cli = Ctx { json, log };
    let (query, asset) = merge_trailing_ids(query, asset, &asset_ids);
    use maki::model_manager::ModelManager;

    // List labels can work without a catalog (uses defaults)
    if list_labels {
        use maki::ai::{DEFAULT_LABELS, load_labels_from_file};

        let label_list: Vec<String> = if let Some(ref path) = labels {
            load_labels_from_file(std::path::Path::new(path))?
        } else {
            // Try config if catalog exists, fall back to defaults
            let config_labels = maki::config::find_catalog_root()
                .ok()
                .and_then(|root| CatalogConfig::load(&root).ok())
                .and_then(|c| c.ai.labels.clone());
            if let Some(ref path) = config_labels {
                load_labels_from_file(std::path::Path::new(path))?
            } else {
                DEFAULT_LABELS.iter().map(|s| s.to_string()).collect()
            }
        };

        if cli.json {
            println!("{}", serde_json::to_string_pretty(&label_list)?);
        } else {
            for label in &label_list {
                println!("{label}");
            }
            eprintln!("\n{} labels", label_list.len());
        }
        return Ok(());
    }

    let (catalog_root, config) = maki::config::load_config()?;

    // Resolve model ID: CLI --model > config ai.model > default.
    // For --download/--remove-model, also accept the positional `query`
    // as a model id when --model isn't given and the positional looks
    // like a known model (this is what users naturally type).
    let model_id_owned: Option<String> = if (download || remove_model) && model.is_none() {
        query
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty() && maki::ai::get_model_spec(s).is_some())
            .map(|s| s.to_string())
    } else {
        None
    };
    let model_id = model_id_owned
        .as_deref()
        .or(model.as_deref())
        .unwrap_or(&config.ai.model);
    let _spec = maki::ai::get_model_spec(model_id)
        .ok_or_else(|| anyhow::anyhow!(
            "Unknown model: {model_id}. Run 'maki auto-tag --list-models' to see available models."
        ))?;

    let model_dir = maki::config::resolve_model_dir(&config.ai.model_dir, model_id);
    let mgr = ModelManager::new(&model_dir, model_id)?;

    // Model management commands
    if download {
        eprintln!("Downloading {} ...", mgr.spec().display_name);
        mgr.ensure_model(|file, current, total| {
            eprintln!("  [{current}/{total}] {file}");
        })?;
        let total = mgr.total_size();
        if cli.json {
            println!("{}", serde_json::json!({
                "status": "downloaded",
                "model": model_id,
                "model_dir": model_dir.display().to_string(),
                "total_size": total,
            }));
        } else {
            println!("Model downloaded to {}", model_dir.display());
            println!("  Total size: {}", format_size(total));
        }
        return Ok(());
    }

    if remove_model {
        mgr.remove_model()?;
        if cli.json {
            println!("{}", serde_json::json!({
                "status": "removed",
                "model": model_id,
                "model_dir": model_dir.display().to_string(),
            }));
        } else {
            println!("Model removed from {}", model_dir.display());
        }
        return Ok(());
    }

    if list_models {
        // model_dir is `<model_base>/<model_id>`; recover model_base for
        // listing siblings. Defaults to `.` if for some reason the path
        // has no parent (shouldn't happen for normal config).
        let model_base = model_dir.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| PathBuf::from("."));
        let models = ModelManager::list_available_models(&model_base);
        if cli.json {
            let json_models: Vec<serde_json::Value> = models
                .iter()
                .map(|(spec, exists, size)| {
                    serde_json::json!({
                        "id": spec.id,
                        "name": spec.display_name,
                        "downloaded": exists,
                        "size": size,
                        "active": spec.id == model_id,
                        "embedding_dim": spec.embedding_dim,
                    })
                })
                .collect();
            println!("{}", serde_json::json!({
                "model_dir": model_base.display().to_string(),
                "active_model": model_id,
                "models": json_models,
            }));
        } else {
            println!("Available models (directory: {}):", model_base.display());
            for (spec, exists, size) in &models {
                let status = if *exists {
                    format!("downloaded ({})", format_size(*size))
                } else {
                    "not downloaded".to_string()
                };
                let active = if spec.id == model_id { " [active]" } else { "" };
                println!("  {} — {}{active}", spec.display_name, status);
                println!("    ID: {}  Embedding dim: {}  Image size: {}px", spec.id, spec.embedding_dim, spec.image_size);
            }
        }
        return Ok(());
    }

    // Similar search mode
    if let Some(ref similar_id) = similar {
        if !mgr.model_exists() {
            anyhow::bail!(
                "Model not downloaded. Run 'maki auto-tag --download --model {model_id}' first."
            );
        }

        let catalog = maki::catalog::Catalog::open(&catalog_root)?;
        let _ = maki::embedding_store::EmbeddingStore::initialize(catalog.conn());
        let emb_store = maki::embedding_store::EmbeddingStore::new(catalog.conn());

        let full_id = catalog
            .resolve_asset_id(similar_id)?
            .ok_or_else(|| anyhow::anyhow!("no asset found matching '{similar_id}'"))?;

        let query_emb = match emb_store.get(&full_id, model_id)? {
            Some(emb) => emb,
            None => {
                // No stored embedding — encode it now
                let config_preview = &config.preview;
                let service = AssetService::new(&catalog_root, verbosity, config_preview);
                let mut ai_model = maki::ai::SigLipModel::load_with_provider(&model_dir, model_id, verbosity, &config.ai.execution_provider)?;
                let registry = DeviceRegistry::new(&catalog_root);
                let volumes = registry.list()?;
                let online_volumes = maki::models::Volume::online_map(&volumes);
                let preview_gen = maki::preview::PreviewGenerator::new(
                    &catalog_root,
                    verbosity,
                    config_preview,
                );
                let details = catalog
                    .load_asset_details(&full_id)?
                    .ok_or_else(|| anyhow::anyhow!("asset not found"))?;
                let image_path = service
                    .find_image_for_ai(&details, &preview_gen, &online_volumes)
                    .ok_or_else(|| {
                        anyhow::anyhow!("no processable image for asset {}", &full_id[..8])
                    })?;
                let emb = ai_model.encode_image(&image_path)?;
                emb_store.store(&full_id, &emb, model_id)?;
                emb
            }
        };

        let results = emb_store.find_similar(&query_emb, 20, Some(&full_id), model_id)?;

        if cli.json {
            let json_results: Vec<serde_json::Value> = results
                .iter()
                .map(|(id, sim)| {
                    serde_json::json!({
                        "asset_id": id,
                        "similarity": sim,
                    })
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&json_results)?);
        } else if results.is_empty() {
            println!("No similar assets found. Run 'maki auto-tag' on more assets to build embeddings.");
        } else {
            println!(
                "Assets similar to {} ({} results):",
                &full_id[..8],
                results.len()
            );
            for (id, sim) in &results {
                let short_id = &id[..8.min(id.len())];
                println!("  {short_id}  similarity: {sim:.3}");
            }
        }
        return Ok(());
    }

    // Main auto-tag flow — require at least one scope filter
    if query.is_none() && asset.is_none() && volume.is_none() && similar.is_none() {
        anyhow::bail!(
            "No scope specified. Provide a query, --asset, or --volume to select assets.\n  \
             Examples:\n    \
             maki auto-tag ''                    # all assets\n    \
             maki auto-tag --asset <id>          # single asset\n    \
             maki auto-tag --volume <label>      # one volume\n    \
             maki auto-tag 'tag:landscape' --apply"
        );
    }

    if !mgr.model_exists() {
        anyhow::bail!(
            "Model not downloaded. Run 'maki auto-tag --download --model {model_id}' first."
        );
    }

    let threshold = threshold.unwrap_or(config.ai.threshold);

    // Resolve labels
    let label_list: Vec<String> = if let Some(ref labels_path) = labels {
        maki::ai::load_labels_from_file(std::path::Path::new(labels_path))?
    } else if let Some(ref config_labels) = config.ai.labels {
        maki::ai::load_labels_from_file(std::path::Path::new(config_labels))?
    } else {
        maki::ai::DEFAULT_LABELS.iter().map(|s| s.to_string()).collect()
    };

    let prompt = &config.ai.prompt;
    let service = AssetService::new(&catalog_root, verbosity, &config.preview);

    let show_log = cli.log;
    let result = service.auto_tag(
        query.as_deref(),
        asset.as_deref(),
        volume.as_deref(),
        threshold,
        &label_list,
        prompt,
        apply,
        &model_dir,
        model_id,
        &config.ai.execution_provider,
        |id, status, elapsed| {
            if show_log {
                let short_id = &id[..8.min(id.len())];
                match status {
                    maki::ai::AutoTagStatus::Suggested(tags) => {
                        let tag_names: Vec<&str> =
                            tags.iter().map(|t| t.tag.as_str()).collect();
                        eprintln!(
                            "  {short_id} — {} tags suggested: {} ({})",
                            tags.len(),
                            tag_names.join(", "),
                            format_duration(elapsed)
                        );
                    }
                    maki::ai::AutoTagStatus::Applied(tags) => {
                        let tag_names: Vec<&str> =
                            tags.iter().map(|t| t.tag.as_str()).collect();
                        eprintln!(
                            "  {short_id} — {} tags applied: {} ({})",
                            tags.len(),
                            tag_names.join(", "),
                            format_duration(elapsed)
                        );
                    }
                    maki::ai::AutoTagStatus::Skipped(msg) => {
                        eprintln!("  {short_id} — skipped: {msg}");
                    }
                    maki::ai::AutoTagStatus::Error(msg) => {
                        eprintln!("  {short_id} — error: {msg}");
                    }
                }
            }
        },
    )?;

    if cli.json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        for err in &result.errors {
            eprintln!("  {err}");
        }

        let mode = if apply { "Auto-tag" } else { "Auto-tag (dry run)" };
        let mut parts = vec![
            format!("{} processed", result.assets_processed),
        ];
        if result.assets_skipped > 0 {
            parts.push(format!("{} skipped", result.assets_skipped));
        }
        parts.push(format!("{} tags suggested", result.tags_suggested));
        if apply {
            parts.push(format!("{} tags applied", result.tags_applied));
        }
        if !result.errors.is_empty() {
            parts.push(format!("{} errors", result.errors.len()));
        }
        println!("{mode}: {}", parts.join(", "));
        if !apply && result.tags_suggested > 0 {
            println!("  Run with --apply to apply suggested tags.");
        }
    }
    Ok(())
}

/// Extracted body of `Commands::Tag`. See `run_import_command` for the
/// extraction pattern.
pub fn run_tag_command(
    asset_id: Option<String>,
    remove: bool,
    tags: Vec<String>,
    subcmd: Option<TagCommands>,
    json: bool,
    log: bool,
) -> anyhow::Result<()> {
    #[allow(dead_code)]
    struct Ctx { json: bool, log: bool }
    let cli = Ctx { json, log };
    match subcmd {
        Some(TagCommands::Rename { old_tag, new_tag, apply }) => {
            let catalog_root = maki::config::find_catalog_root()?;
            let engine = QueryEngine::new(&catalog_root);
            let old_storage = maki::tag_util::tag_input_to_storage(&old_tag);
            let new_storage = maki::tag_util::tag_input_to_storage(&new_tag);
            let show_log = cli.log;

            use maki::query::TagRenameAction;
            let result = engine.tag_rename(&old_storage, &new_storage, apply, |name, action| {
                if show_log {
                    let verb = match (action, apply) {
                        (TagRenameAction::Renamed, true) => "renamed",
                        (TagRenameAction::Renamed, false) => "would rename",
                        (TagRenameAction::Removed, true) => "removed (already had target)",
                        (TagRenameAction::Removed, false) => "would remove (already has target)",
                        (TagRenameAction::Skipped, _) => "skipped (already correct)",
                    };
                    eprintln!("  {} — {}", name, verb);
                }
            })?;

            if cli.json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                let old_display = maki::tag_util::tag_storage_to_display(&old_storage);
                let new_display = maki::tag_util::tag_storage_to_display(&new_storage);
                if result.matched == 0 {
                    println!("No assets found with tag \"{}\".", old_display);
                } else {
                    if !apply && (result.renamed > 0 || result.removed > 0) {
                        eprint!("Dry run — ");
                    }
                    let mut parts = Vec::new();
                    if result.renamed > 0 {
                        parts.push(format!("{} renamed", result.renamed));
                    }
                    if result.removed > 0 {
                        parts.push(format!("{} removed (merged)", result.removed));
                    }
                    if result.skipped > 0 {
                        parts.push(format!("{} skipped", result.skipped));
                    }
                    println!(
                        "Tag rename: \"{}\" → \"{}\": {}",
                        old_display, new_display, parts.join(", "),
                    );
                    if !apply && (result.renamed > 0 || result.removed > 0) {
                        println!("  Run with --apply to rename tags.");
                    }
                }
            }
            Ok(())
        }
        Some(TagCommands::Split { old_tag, new_tags, keep, apply }) => {
            let catalog_root = maki::config::find_catalog_root()?;
            let engine = QueryEngine::new(&catalog_root);
            let old_storage = maki::tag_util::tag_input_to_storage(&old_tag);
            let new_storage: Vec<String> = new_tags.iter()
                .map(|t| maki::tag_util::tag_input_to_storage(t))
                .collect();
            let show_log = cli.log;

            use maki::query::TagSplitAction;
            let result = engine.tag_split(&old_storage, &new_storage, keep, apply, |name, action| {
                if show_log {
                    let verb = match (action, apply, keep) {
                        (TagSplitAction::Split, true, false) => "split",
                        (TagSplitAction::Split, false, false) => "would split",
                        (TagSplitAction::Split, true, true) => "added targets",
                        (TagSplitAction::Split, false, true) => "would add targets",
                        (TagSplitAction::Skipped, _, _) => "skipped (no change needed)",
                    };
                    eprintln!("  {} — {}", name, verb);
                }
            })?;

            if cli.json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                let old_display = maki::tag_util::tag_storage_to_display(&old_storage);
                let targets_display = new_storage.iter()
                    .map(|t| format!("\"{}\"", maki::tag_util::tag_storage_to_display(t)))
                    .collect::<Vec<_>>()
                    .join(", ");
                if result.matched == 0 {
                    println!("No assets found with tag \"{}\".", old_display);
                } else {
                    if !apply && result.split > 0 {
                        eprint!("Dry run — ");
                    }
                    let verb = if keep { "add" } else { "split" };
                    let mut parts = Vec::new();
                    if result.split > 0 {
                        parts.push(format!("{} {}", result.split, if keep { "augmented" } else { "split" }));
                    }
                    if result.skipped > 0 {
                        parts.push(format!("{} skipped", result.skipped));
                    }
                    println!(
                        "Tag {}: \"{}\" → [{}]: {}",
                        verb, old_display, targets_display, parts.join(", "),
                    );
                    if !apply && result.split > 0 {
                        println!("  Run with --apply to {} tags.", verb);
                    }
                }
            }
            Ok(())
        }
        Some(TagCommands::Delete { tag, apply }) => {
            let catalog_root = maki::config::find_catalog_root()?;
            let engine = QueryEngine::new(&catalog_root);
            let storage_tag = maki::tag_util::tag_input_to_storage(&tag);
            let show_log = cli.log;

            use maki::query::TagDeleteAction;
            let result = engine.tag_delete(&storage_tag, apply, |name, action| {
                if show_log {
                    let verb = match (action, apply) {
                        (TagDeleteAction::Removed, true) => "removed",
                        (TagDeleteAction::Removed, false) => "would remove",
                        (TagDeleteAction::Skipped, _) => "skipped",
                    };
                    eprintln!("  {} — {}", name, verb);
                }
            })?;

            if cli.json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                // Strip markers from the display string for the user-facing message.
                let display = storage_tag
                    .trim_start_matches(|c: char| c == '=' || c == '/' || c == '^')
                    .to_string();
                if result.matched == 0 {
                    println!("No assets found with tag \"{}\".", display);
                } else {
                    if !apply && result.removed > 0 {
                        eprint!("Dry run — ");
                    }
                    let mut parts = Vec::new();
                    if result.removed > 0 {
                        parts.push(format!(
                            "{} {}",
                            result.removed,
                            if apply { "removed" } else { "would remove" }
                        ));
                    }
                    if result.skipped > 0 {
                        parts.push(format!("{} skipped", result.skipped));
                    }
                    println!(
                        "Tag delete \"{}\": {} matched, {}",
                        display,
                        result.matched,
                        parts.join(", "),
                    );
                    if !apply && result.removed > 0 {
                        println!("  Run with --apply to delete the tag.");
                    }
                }
            }
            Ok(())
        }
        Some(TagCommands::FixUnicode { apply }) => {
            let catalog_root = maki::config::find_catalog_root()?;
            let engine = QueryEngine::new(&catalog_root);
            let show_log = cli.log;

            let result = engine.tag_fix_unicode(apply, |name, _changed| {
                if show_log {
                    eprintln!("  {} — {}", name, if apply { "fixed" } else { "would fix" });
                }
            })?;

            if cli.json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                if !apply && result.fixed > 0 {
                    eprint!("Dry run — ");
                }
                if result.fixed == 0 {
                    println!(
                        "Tag fix-unicode: {} asset(s) scanned, all already NFC.",
                        result.scanned
                    );
                } else {
                    let merge_msg = if result.merged > 0 {
                        format!(", {} with merged duplicates", result.merged)
                    } else {
                        String::new()
                    };
                    println!(
                        "Tag fix-unicode: {} scanned, {} {}, {} tag value(s) normalised{}",
                        result.scanned,
                        result.fixed,
                        if apply { "fixed" } else { "would be fixed" },
                        result.tags_normalized,
                        merge_msg,
                    );
                    if !apply && result.fixed > 0 {
                        println!("  Run with --apply to normalise tags.");
                    }
                }
            }
            Ok(())
        }
        Some(TagCommands::Clear { asset_id }) => {
            let catalog_root = maki::config::find_catalog_root()?;
            let engine = QueryEngine::new(&catalog_root);

            // Load asset to get current tags, then remove all
            let catalog = maki::catalog::Catalog::open(&catalog_root)?;
            let full_id = catalog.resolve_asset_id(&asset_id)?
                .ok_or_else(|| anyhow::anyhow!("no asset found matching '{asset_id}'"))?;
            let uuid: uuid::Uuid = full_id.parse()?;
            let metadata_store = maki::metadata_store::MetadataStore::new(&catalog_root);
            let asset = metadata_store.load(uuid)?;

            if asset.tags.is_empty() {
                if cli.json {
                    println!("{}", serde_json::json!({ "changed": [], "tags": [] }));
                } else {
                    println!("Tags: (none)");
                }
                return Ok(());
            }

            let tags_to_remove = asset.tags.clone();
            let result = engine.tag(&asset_id, &tags_to_remove, true)?;

            if cli.json {
                println!("{}", serde_json::json!({
                    "changed": result.changed,
                    "tags": result.current_tags,
                }));
            } else {
                let display_removed: Vec<String> = result.changed.iter()
                    .map(|t| maki::tag_util::tag_storage_to_display(t))
                    .collect();
                println!("Cleared {} tag(s): {}", display_removed.len(), display_removed.join(", "));
            }
            Ok(())
        }
        Some(TagCommands::ExpandAncestors { query, asset, apply, asset_ids }) => {
            let catalog_root = maki::config::find_catalog_root()?;
            let catalog = maki::catalog::Catalog::open(&catalog_root)?;
            let metadata_store = maki::metadata_store::MetadataStore::new(&catalog_root);
            let engine = maki::query::QueryEngine::new(&catalog_root);

            let scope = engine.resolve_scope(query.as_deref(), asset.as_deref(), &asset_ids)?;
            let summaries = metadata_store.list()?;
            let ids: Vec<uuid::Uuid> = match scope {
                Some(set) => set.iter().filter_map(|id| id.parse().ok()).collect(),
                None => summaries.iter().map(|s| s.id).collect(),
            };

            let mut checked = 0usize;
            let mut expanded = 0usize;
            let mut tags_added = 0usize;

            for asset_id in &ids {
                let mut asset_obj = match metadata_store.load(*asset_id) {
                    Ok(a) => a,
                    Err(_) => continue,
                };
                checked += 1;

                let full = maki::tag_util::expand_all_ancestors(&asset_obj.tags);
                let existing: std::collections::HashSet<&str> = asset_obj.tags.iter().map(|s| s.as_str()).collect();
                let missing: Vec<String> = full.iter()
                    .filter(|t| !existing.contains(t.as_str()))
                    .cloned()
                    .collect();

                if missing.is_empty() {
                    continue;
                }

                if cli.log {
                    let id_str = asset_id.to_string();
                    let name = asset_obj.name.as_deref().unwrap_or(&id_str[..8]);
                    if apply {
                        eprintln!("  {} — {} ancestor(s) added", name, missing.len());
                    } else {
                        eprintln!("  {} — {} ancestor(s) would add", name, missing.len());
                    }
                }

                if apply {
                    for tag in &missing {
                        asset_obj.tags.push(tag.clone());
                    }
                    metadata_store.save(&asset_obj)?;
                    catalog.insert_asset(&asset_obj)?;
                }

                expanded += 1;
                tags_added += missing.len();
            }

            if cli.json {
                println!("{}", serde_json::json!({
                    "dry_run": !apply,
                    "checked": checked,
                    "expanded": expanded,
                    "tags_added": tags_added,
                }));
            } else {
                if !apply && expanded > 0 {
                    eprint!("Dry run — ");
                }
                println!("Expand ancestors: {} checked, {} assets with missing ancestors, {} tags {}",
                    checked, expanded, tags_added,
                    if apply { "added" } else { "would add" },
                );
                if !apply && expanded > 0 {
                    println!("  Run with --apply to expand ancestor tags.");
                }
            }
            Ok(())
        }
        Some(TagCommands::ExportVocabulary { output, format, prune, default, counts }) => {
            let catalog_root = maki::config::find_catalog_root()?;

            let format = format.to_lowercase();
            #[derive(Clone, Copy, PartialEq)]
            enum Fmt { Yaml, Text, Json }
            let (fmt, default_filename) = match format.as_str() {
                "yaml" | "yml" => (Fmt::Yaml, "vocabulary.yaml"),
                "text" | "txt" => (Fmt::Text, "vocabulary.txt"),
                "json"         => (Fmt::Json, "vocabulary.json"),
                other => anyhow::bail!("unknown --format '{}': expected 'yaml', 'text', or 'json'", other),
            };

            if default {
                // Export only the built-in default vocabulary. The default
                // tree has no asset counts, so --counts has nothing useful
                // to add (every node would say "0 assets") — silently
                // ignored regardless of format.
                let (content, sanitized) = match fmt {
                    Fmt::Text => {
                        let flat = maki::vocabulary::parse_vocabulary(maki::vocabulary::default_vocabulary());
                        let pairs: Vec<(String, u64)> = flat.into_iter().map(|t| (t, 0)).collect();
                        maki::vocabulary::tags_to_keyword_text(&pairs)
                    }
                    Fmt::Json => {
                        let flat = maki::vocabulary::parse_vocabulary(maki::vocabulary::default_vocabulary());
                        let pairs: Vec<(String, u64)> = flat.into_iter().map(|t| (t, 0)).collect();
                        (maki::vocabulary::tags_to_vocabulary_json(&pairs), Vec::new())
                    }
                    Fmt::Yaml => {
                        // The built-in vocab is itself YAML; emit verbatim.
                        (maki::vocabulary::default_vocabulary().to_string(), Vec::new())
                    }
                };
                let out_path = output.map(std::path::PathBuf::from)
                    .unwrap_or_else(|| catalog_root.join(default_filename));
                std::fs::write(&out_path, content)?;
                report_sanitized_tags(&sanitized);
                println!("Exported default vocabulary to {}", out_path.display());
                return Ok(());
            }

            let catalog = maki::catalog::Catalog::open(&catalog_root)?;
            let catalog_tags = catalog.list_all_tags()?;

            // Merge with existing vocabulary (preserve planned-but-unused entries)
            let mut all_tags = catalog_tags;
            if !prune {
                let vocab = maki::vocabulary::load_vocabulary(&catalog_root);
                let existing: std::collections::HashSet<String> = all_tags.iter().map(|(name, _)| name.clone()).collect();
                for vt in vocab {
                    if !existing.contains(&vt) {
                        all_tags.push((vt, 0));
                    }
                }
            }
            all_tags.sort_by(|a, b| a.0.cmp(&b.0));

            let (content, sanitized) = match fmt {
                Fmt::Text => {
                    // Counts can't go in keyword-text (LR/C1 reject comments).
                    if counts {
                        eprintln!("  Note: --counts is ignored for text format (Lightroom/Capture One don't accept comments).");
                    }
                    maki::vocabulary::tags_to_keyword_text(&all_tags)
                }
                Fmt::Yaml => {
                    let yaml = if counts {
                        maki::vocabulary::tags_to_vocabulary_yaml_with_counts(&all_tags)
                    } else {
                        maki::vocabulary::tags_to_vocabulary_yaml(&all_tags)
                    };
                    (yaml, Vec::new())
                }
                Fmt::Json => (maki::vocabulary::tags_to_vocabulary_json(&all_tags), Vec::new()),
            };
            let out_path = output.map(std::path::PathBuf::from)
                .unwrap_or_else(|| catalog_root.join(default_filename));
            std::fs::write(&out_path, &content)?;

            report_sanitized_tags(&sanitized);

            let used = all_tags.iter().filter(|(_, c)| *c > 0).count();
            let planned = all_tags.len() - used;
            if prune {
                println!("Exported {} tags to {} (pruned, unused entries removed)", used, out_path.display());
            } else {
                println!("Exported {} tags to {} ({} used, {} planned)", all_tags.len(), out_path.display(), used, planned);
            }
            Ok(())
        }
        None => {
            // Direct tag add/remove: maki tag <asset> <tags> [--remove]
            let asset_id = asset_id.map(|s| s.trim().to_string())
                .ok_or_else(|| anyhow::anyhow!("asset ID is required for tag add/remove"))?;
            if tags.is_empty() {
                anyhow::bail!("no tags specified.");
            }
            let catalog_root = maki::config::find_catalog_root()?;
            let engine = QueryEngine::new(&catalog_root);
            let storage_tags: Vec<String> = tags.iter()
                .map(|t| maki::tag_util::tag_input_to_storage(t))
                .collect();
            let result = engine.tag(&asset_id, &storage_tags, remove)?;

            if cli.json {
                println!("{}", serde_json::json!({
                    "changed": result.changed,
                    "tags": result.current_tags,
                }));
            } else {
                let display_changed: Vec<String> = result.changed.iter()
                    .map(|t| maki::tag_util::tag_storage_to_display(t))
                    .collect();
                let display_tags: Vec<String> = result.current_tags.iter()
                    .map(|t| maki::tag_util::tag_storage_to_display(t))
                    .collect();
                if !display_changed.is_empty() {
                    if remove {
                        println!("Removed tags: {}", display_changed.join(", "));
                    } else {
                        println!("Added tags: {}", display_changed.join(", "));
                    }
                }
                if display_tags.is_empty() {
                    println!("Tags: (none)");
                } else {
                    println!("Tags: {}", display_tags.join(", "));
                }
            }
            Ok(())
        }
    }
}

/// Extracted body of `Commands::RebuildCatalog`.
pub fn run_rebuild_catalog_command(
    asset: Option<String>,
    json: bool,
) -> anyhow::Result<()> {
    struct Ctx { json: bool }
    let cli = Ctx { json };
    let catalog_root = maki::config::find_catalog_root()?;

    if let Some(ref asset_id) = asset {
        // Per-asset rebuild: delete and re-insert a single asset from its sidecar
        let catalog = Catalog::open(&catalog_root)?;
        let store = MetadataStore::new(&catalog_root);

        // Resolve asset ID (try as UUID first, then prefix match in catalog)
        let uuid: uuid::Uuid = if let Ok(u) = asset_id.parse() {
            u
        } else if let Some(full) = catalog.resolve_asset_id(asset_id)? {
            full.parse()?
        } else {
            // Not in SQLite — try loading sidecar directly
            anyhow::bail!("asset '{}' not found in catalog. For new assets, use 'maki refresh --reimport --asset {}'", asset_id, asset_id);
        };

        let asset_obj = store.load(uuid)?;
        let id_str = uuid.to_string();

        // Delete all existing rows for this asset (FK checks off for safety)
        let _ = catalog.conn().execute_batch("PRAGMA foreign_keys = OFF");

        // Get all variant hashes (from SQLite, may differ from sidecar)
        let sqlite_hashes: Vec<String> = catalog.conn()
            .prepare("SELECT content_hash FROM variants WHERE asset_id = ?1")?
            .query_map(rusqlite::params![&id_str], |r| r.get(0))?
            .filter_map(|r| r.ok())
            .collect();

        for hash in &sqlite_hashes {
            let _ = catalog.conn().execute("DELETE FROM recipes WHERE variant_hash = ?1", rusqlite::params![hash]);
            let _ = catalog.conn().execute("DELETE FROM file_locations WHERE content_hash = ?1", rusqlite::params![hash]);
        }
        let _ = catalog.conn().execute("DELETE FROM variants WHERE asset_id = ?1", rusqlite::params![&id_str]);
        let _ = catalog.conn().execute("DELETE FROM faces WHERE asset_id = ?1", rusqlite::params![&id_str]);
        let _ = catalog.conn().execute("DELETE FROM embeddings WHERE asset_id = ?1", rusqlite::params![&id_str]);
        let _ = catalog.conn().execute("DELETE FROM collection_assets WHERE asset_id = ?1", rusqlite::params![&id_str]);
        let _ = catalog.conn().execute("DELETE FROM assets WHERE id = ?1", rusqlite::params![&id_str]);

        let _ = catalog.conn().execute_batch("PRAGMA foreign_keys = ON");

        // Re-insert from sidecar
        let registry = DeviceRegistry::new(&catalog_root);
        for volume in registry.list()? {
            catalog.ensure_volume(&volume)?;
        }

        catalog.insert_asset(&asset_obj)?;
        for variant in &asset_obj.variants {
            catalog.insert_variant(variant)?;
            for loc in &variant.locations {
                catalog.insert_file_location(&variant.content_hash, loc)?;
            }
        }
        for recipe in &asset_obj.recipes {
            catalog.insert_recipe(recipe)?;
        }
        catalog.update_denormalized_variant_columns(&asset_obj)?;

        // Restore embedding from binary file if it exists
        #[cfg(feature = "ai")]
        {
            let emb_store = maki::embedding_store::EmbeddingStore::new(catalog.conn());
            let emb_base = catalog_root.join("embeddings");
            if emb_base.exists() {
                if let Ok(entries) = std::fs::read_dir(&emb_base) {
                    for entry in entries.flatten() {
                        let name = entry.file_name().to_string_lossy().to_string();
                        if name == "arcface" || !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                            continue;
                        }
                        let prefix = &id_str[..2];
                        let bin_path = emb_base.join(&name).join(prefix).join(format!("{id_str}.bin"));
                        if bin_path.exists() {
                            if let Ok(data) = std::fs::read(&bin_path) {
                                let embedding: Vec<f32> = data.chunks_exact(4)
                                    .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                                    .collect();
                                let _ = emb_store.store(&id_str, &embedding, &name);
                            }
                        }
                    }
                }
            }

            // Restore faces for this asset
            let face_store = maki::face_store::FaceStore::new(catalog.conn());
            let faces_file = maki::face_store::load_faces_yaml(&catalog_root).unwrap_or_default();
            let asset_face_ids: Vec<String> = faces_file.faces.iter()
                .filter(|f| f.asset_id == id_str)
                .map(|f| f.id.clone())
                .collect();
            if !asset_face_ids.is_empty() {
                let filtered = maki::face_store::FacesFile {
                    faces: faces_file.faces.into_iter().filter(|f| f.asset_id == id_str).collect(),
                };
                let _ = face_store.import_faces_from_yaml(&filtered);
            }
            // Restore ArcFace embeddings for this asset's faces
            let asset_faces = face_store.faces_for_asset(&id_str).unwrap_or_default();
            for face in &asset_faces {
                let prefix = &face.id[..2.min(face.id.len())];
                let bin_path = emb_base.join("arcface").join(prefix).join(format!("{}.bin", face.id));
                if bin_path.exists() {
                    if let Ok(data) = std::fs::read(&bin_path) {
                        let embedding: Vec<f32> = data.chunks_exact(4)
                            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                            .collect();
                        let _ = face_store.import_face_embedding(&face.id, &embedding);
                    }
                }
            }

            // Update face_count
            let _ = catalog.conn().execute(
                "UPDATE assets SET face_count = (SELECT COUNT(*) FROM faces WHERE faces.asset_id = ?1) WHERE id = ?1",
                rusqlite::params![&id_str],
            );
            // Legacy upgrade fallback: see the full-catalog rebuild for context.
            let _ = catalog.conn().execute(
                "UPDATE assets SET face_scan_status = 'done' \
                 WHERE id = ?1 AND face_scan_status IS NULL AND face_count > 0",
                rusqlite::params![&id_str],
            );
        }

        if cli.json {
            println!("{}", serde_json::json!({
                "asset_id": id_str,
                "variants": asset_obj.variants.len(),
                "recipes": asset_obj.recipes.len(),
            }));
        } else {
            println!("Rebuilt asset {}: {} variant(s), {} recipe(s)",
                &id_str[..8], asset_obj.variants.len(), asset_obj.recipes.len());
        }
        return Ok(());
    }

    let catalog = Catalog::open(&catalog_root)?;
    catalog.initialize()?;

    // Ensure volume rows exist so FK references work
    let registry = DeviceRegistry::new(&catalog_root);
    for volume in registry.list()? {
        catalog.ensure_volume(&volume)?;
    }

    // Clear existing data rows
    catalog.rebuild()?;

    // Sync sidecar files into catalog
    let store = MetadataStore::new(&catalog_root);
    let result = store.sync_to_catalog(&catalog)?;

    // Restore collections from YAML
    let collections_restored = {
        let col_file = maki::collection::load_yaml(&catalog_root).unwrap_or_default();
        if !col_file.collections.is_empty() {
            let col_store = maki::collection::CollectionStore::new(catalog.conn());
            col_store.import_from_yaml(&col_file).unwrap_or(0)
        } else {
            0
        }
    };

    // Restore stacks from YAML
    let stacks_restored = {
        let stacks_file = maki::stack::load_yaml(&catalog_root).unwrap_or_default();
        if !stacks_file.stacks.is_empty() {
            let stack_store = maki::stack::StackStore::new(catalog.conn());
            stack_store.import_from_yaml(&stacks_file).unwrap_or(0)
        } else {
            0
        }
    };

    // Restore faces, people, and embeddings from files
    #[cfg(feature = "ai")]
    let (people_restored, faces_restored, face_embeddings_restored, embeddings_restored) = {
        let _ = maki::face_store::FaceStore::initialize(catalog.conn());
        let _ = maki::embedding_store::EmbeddingStore::initialize(catalog.conn());
        let face_store = maki::face_store::FaceStore::new(catalog.conn());

        // Import people first (faces reference people via FK)
        let people_file = maki::face_store::load_people_yaml(&catalog_root).unwrap_or_default();
        let people_restored = if !people_file.people.is_empty() {
            face_store.import_people_from_yaml(&people_file).unwrap_or(0)
        } else {
            0
        };

        // Import faces (with empty embedding placeholder)
        let faces_file = maki::face_store::load_faces_yaml(&catalog_root).unwrap_or_default();
        let faces_restored = if !faces_file.faces.is_empty() {
            face_store.import_faces_from_yaml(&faces_file).unwrap_or(0)
        } else {
            0
        };

        // Restore ArcFace embeddings from binary files
        let mut face_embeddings_restored = 0u32;
        if let Ok(arcface_entries) = maki::face_store::scan_arcface_binaries(&catalog_root) {
            for (face_id, embedding) in &arcface_entries {
                if face_store.import_face_embedding(face_id, embedding).is_ok() {
                    face_embeddings_restored += 1;
                }
            }
        }

        // Restore SigLIP embeddings from binary files
        let mut embeddings_restored = 0u32;
        let emb_store = maki::embedding_store::EmbeddingStore::new(catalog.conn());
        // Scan all model directories under embeddings/ (skip "arcface")
        let emb_base = catalog_root.join("embeddings");
        if emb_base.exists() {
            if let Ok(entries) = std::fs::read_dir(&emb_base) {
                for entry in entries.flatten() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    if name == "arcface" || !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                        continue;
                    }
                    if let Ok(model_entries) = maki::embedding_store::scan_embedding_binaries(&catalog_root, &name) {
                        for (asset_id, embedding) in &model_entries {
                            if emb_store.store(asset_id, embedding, &name).is_ok() {
                                embeddings_restored += 1;
                            }
                        }
                    }
                }
            }
        }

        // Backfill face_count denormalized column
        if faces_restored > 0 {
            let _ = catalog.conn().execute_batch(
                "UPDATE assets SET face_count = (
                    SELECT COUNT(*) FROM faces WHERE faces.asset_id = assets.id
                ) WHERE id IN (SELECT DISTINCT asset_id FROM faces)"
            );
            // Legacy upgrade fallback: if an asset has face records but its
            // YAML sidecar predates the face_scan_status field, mark it as
            // scanned. This matters only for users upgrading from v4.4.2 or
            // earlier — newer writes always put face_scan_status in the
            // sidecar, so this branch is a no-op for fresh catalogs.
            let _ = catalog.conn().execute_batch(
                "UPDATE assets SET face_scan_status = 'done' \
                 WHERE face_scan_status IS NULL AND face_count > 0"
            );
        }

        (people_restored, faces_restored, face_embeddings_restored, embeddings_restored)
    };

    if cli.json {
        #[allow(unused_mut)]
        let mut json = serde_json::json!({
            "synced": result.synced,
            "errors": result.errors,
            "collections_restored": collections_restored,
            "stacks_restored": stacks_restored,
        });
        #[cfg(feature = "ai")]
        {
            json["people_restored"] = serde_json::json!(people_restored);
            json["faces_restored"] = serde_json::json!(faces_restored);
            json["face_embeddings_restored"] = serde_json::json!(face_embeddings_restored);
            json["embeddings_restored"] = serde_json::json!(embeddings_restored);
        }
        println!("{}", json);
    } else {
        println!("Rebuild complete: {} asset(s) synced", result.synced);
        if collections_restored > 0 {
            println!("  {} collection(s) restored", collections_restored);
        }
        if stacks_restored > 0 {
            println!("  {} stack(s) restored", stacks_restored);
        }
        #[cfg(feature = "ai")]
        {
            if people_restored > 0 {
                println!("  {} people restored", people_restored);
            }
            if faces_restored > 0 {
                println!("  {} face(s) restored ({} embeddings)", faces_restored, face_embeddings_restored);
            }
            if embeddings_restored > 0 {
                println!("  {} embedding(s) restored", embeddings_restored);
            }
        }
        if result.errors > 0 {
            println!("  {} error(s) encountered", result.errors);
        }

        // After rebuild, count assets that ended up without AI-derived
        // data — those whose embedding binaries weren't on disk, or
        // that were imported on a build without the AI feature. The
        // user often forgets to re-run `embed` / `faces detect` after
        // a rebuild and only notices much later when similarity search
        // returns empty / face cluster is missing recent assets.
        #[cfg(feature = "ai")]
        {
            let total_assets = catalog.conn().query_row(
                "SELECT COUNT(*) FROM assets", [], |r| r.get::<_, i64>(0)
            ).unwrap_or(0);
            let with_embeddings = catalog.conn().query_row(
                "SELECT COUNT(DISTINCT asset_id) FROM embeddings", [], |r| r.get::<_, i64>(0)
            ).unwrap_or(0);
            let missing_embeddings = (total_assets - with_embeddings).max(0);
            if missing_embeddings > 0 {
                println!(
                    "  Tip: {} asset(s) have no embedding. Run 'maki embed' \
                     for visual similarity / text search.",
                    missing_embeddings
                );
            }
            let unscanned_for_faces = catalog.conn().query_row(
                "SELECT COUNT(*) FROM assets \
                 WHERE (face_scan_status IS NULL OR face_scan_status = 'pending')",
                [], |r| r.get::<_, i64>(0)
            ).unwrap_or(0);
            if unscanned_for_faces > 0 {
                println!(
                    "  Tip: {} asset(s) haven't been scanned for faces. \
                     Run 'maki faces detect' to populate.",
                    unscanned_for_faces
                );
            }
        }
    }
    Ok(())
}

/// Extracted body of `Commands::Volume`. Inner match dispatches to the
/// `VolumeCommands` subcommand variants.
pub fn run_volume_command(
    cmd: VolumeCommands,
    json: bool,
    log: bool,
    #[allow(unused_variables)] verbosity: maki::Verbosity,
) -> anyhow::Result<()> {
    #[allow(dead_code)]
    struct Ctx { json: bool, log: bool }
    let cli = Ctx { json, log };
    let _ = verbosity;
    match cmd {
    VolumeCommands::Add { args, purpose } => {
        // Two positional args: LABEL PATH. One arg: PATH (label derived).
        let (label, path) = if args.len() == 2 {
            (args[0].clone(), args[1].clone())
        } else {
            let path = &args[0];
            let label = std::path::Path::new(path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("Volume")
                .to_string();
            (label, path.clone())
        };

        let catalog_root = maki::config::find_catalog_root()?;
        let registry = DeviceRegistry::new(&catalog_root);
        let parsed_purpose = if let Some(ref p) = purpose {
            Some(maki::models::VolumePurpose::parse(p).ok_or_else(|| {
                anyhow::anyhow!("invalid purpose '{}'. Valid values: media, working, archive, backup, cloud", p)
            })?)
        } else {
            None
        };
        let volume = registry.register(
            &label,
            std::path::Path::new(&path),
            maki::models::VolumeType::Local,
            parsed_purpose,
        )?;
        if cli.json {
            println!("{}", serde_json::json!({
                "id": volume.id.to_string(),
                "label": volume.label,
                "path": volume.mount_point.display().to_string(),
                "purpose": volume.purpose.as_ref().map(|p| p.as_str()),
            }));
        } else {
            println!("Registered volume '{}' ({})", volume.label, volume.id);
            println!("  Path: {}", volume.mount_point.display());
            if let Some(ref p) = volume.purpose {
                println!("  Purpose: {}", p);
            } else {
                eprintln!("  Hint: use --purpose <media|working|archive|backup|cloud> to set the volume's role");
            }
        }
        Ok(())
    }
    VolumeCommands::List { purpose, offline, online } => {
        if offline && online {
            anyhow::bail!("--offline and --online are mutually exclusive");
        }
        let purpose_filter = if let Some(ref p) = purpose {
            Some(maki::models::VolumePurpose::parse(p).ok_or_else(|| {
                anyhow::anyhow!("invalid purpose '{}'. Valid values: media, working, archive, backup, cloud", p)
            })?)
        } else {
            None
        };

        let catalog_root = maki::config::find_catalog_root()?;
        let registry = DeviceRegistry::new(&catalog_root);
        let volumes: Vec<_> = registry.list()?.into_iter().filter(|v| {
            if let Some(ref pf) = purpose_filter {
                if v.purpose.as_ref() != Some(pf) {
                    return false;
                }
            }
            if offline && v.is_online { return false; }
            if online && !v.is_online { return false; }
            true
        }).collect();

        if cli.json {
            let json_volumes: Vec<serde_json::Value> = volumes.iter().map(|v| {
                serde_json::json!({
                    "id": v.id.to_string(),
                    "label": v.label,
                    "path": v.mount_point.display().to_string(),
                    "volume_type": format!("{:?}", v.volume_type).to_lowercase(),
                    "purpose": v.purpose.as_ref().map(|p| p.as_str()),
                    "is_online": v.is_online,
                })
            }).collect();
            println!("{}", serde_json::to_string_pretty(&json_volumes)?);
        } else if volumes.is_empty() {
            if purpose.is_some() || offline || online {
                println!("No matching volumes.");
            } else {
                println!("No volumes registered.");
            }
        } else {
            for v in &volumes {
                let status = if v.is_online { "online" } else { "offline" };
                let purpose_tag = v.purpose.as_ref()
                    .map(|p| format!(" [{}]", p))
                    .unwrap_or_default();
                println!("{} ({}) [{}]{}", v.label, v.id, status, purpose_tag);
                println!("  Path: {}", v.mount_point.display());
            }
        }
        Ok(())
    }
    VolumeCommands::SetPurpose { volume, purpose } => {
        let catalog_root = maki::config::find_catalog_root()?;
        let registry = DeviceRegistry::new(&catalog_root);
        let parsed_purpose = if purpose == "none" || purpose == "clear" {
            None
        } else {
            Some(maki::models::VolumePurpose::parse(&purpose).ok_or_else(|| {
                anyhow::anyhow!("invalid purpose '{}'. Valid values: media, working, archive, backup, cloud, none", purpose)
            })?)
        };
        let vol = registry.set_purpose(&volume, parsed_purpose)?;
        // Update the SQLite cache too
        let catalog = maki::catalog::Catalog::open(&catalog_root)?;
        catalog.ensure_volume(&vol)?;
        if cli.json {
            println!("{}", serde_json::json!({
                "id": vol.id.to_string(),
                "label": vol.label,
                "purpose": vol.purpose.as_ref().map(|p| p.as_str()),
            }));
        } else if let Some(ref p) = vol.purpose {
            println!("Volume '{}' purpose set to: {}", vol.label, p);
        } else {
            println!("Volume '{}' purpose cleared.", vol.label);
        }
        Ok(())
    }
    VolumeCommands::Remove { volume, apply } => {
        let (catalog_root, config) = maki::config::load_config()?;
        let service = AssetService::new(&catalog_root, verbosity, &config.preview);

        let show_log = cli.log;
        let result = if show_log {
            use maki::asset_service::CleanupStatus;
            service.remove_volume(
                &volume,
                apply,
                |path, status, elapsed| {
                    match status {
                        CleanupStatus::Stale => {
                            let name = path.file_name()
                                .and_then(|n| n.to_str())
                                .unwrap_or_else(|| path.to_str().unwrap_or("?"));
                            item_status(name, "removed", Some(elapsed));
                        }
                        CleanupStatus::LocationlessVariant => {
                            let name = path.to_str().unwrap_or("?");
                            item_status(name, "locationless variant removed", Some(elapsed));
                        }
                        CleanupStatus::OrphanedAsset => {
                            let name = path.to_str().unwrap_or("?");
                            item_status(name, "orphaned asset removed", Some(elapsed));
                        }
                        CleanupStatus::OrphanedFile => {
                            let name = path.file_name()
                                .and_then(|n| n.to_str())
                                .unwrap_or_else(|| path.to_str().unwrap_or("?"));
                            item_status(name, "orphaned file removed", Some(elapsed));
                        }
                        _ => {}
                    }
                },
            )?
        } else {
            service.remove_volume(
                &volume,
                apply,
                |_, _, _| {},
            )?
        };

        if cli.json {
            println!("{}", serde_json::to_string_pretty(&result)?);
        } else {
            for err in &result.errors {
                eprintln!("  {err}");
            }

            if apply {
                let mut parts = vec![
                    format!("{} locations removed", result.locations_removed),
                    format!("{} recipes removed", result.recipes_removed),
                ];
                if result.removed_assets > 0 {
                    parts.push(format!("{} orphaned assets removed", result.removed_assets));
                }
                if result.removed_previews > 0 {
                    parts.push(format!("{} orphaned previews removed", result.removed_previews));
                }
                println!("Volume '{}' removed: {}", result.volume_label, parts.join(", "));
            } else {
                let mut parts = vec![
                    format!("{} locations", result.locations),
                    format!("{} recipes", result.recipes),
                ];
                if result.orphaned_assets > 0 {
                    parts.push(format!("{} orphaned assets", result.orphaned_assets));
                }
                if result.orphaned_previews > 0 {
                    parts.push(format!("{} orphaned previews", result.orphaned_previews));
                }
                println!("Volume '{}' would remove: {}", result.volume_label, parts.join(", "));
                if result.locations > 0 || result.recipes > 0 {
                    println!("  Run with --apply to remove.");
                }
            }
        }
        Ok(())
    }
    VolumeCommands::Combine { source, target, apply } => {
        let (catalog_root, config) = maki::config::load_config()?;
        let service = AssetService::new(&catalog_root, verbosity, &config.preview);

        let show_log = cli.log;
        let result = service.combine_volume(
            &source,
            &target,
            apply,
            |asset_id, elapsed| {
                if show_log {
                    item_status(asset_id, "updated", Some(elapsed));
                }
            },
        )?;

        if cli.json {
            println!("{}", serde_json::to_string_pretty(&result)?);
        } else {
            for err in &result.errors {
                eprintln!("  {err}");
            }

            if apply {
                println!(
                    "Volume '{}' combined into '{}': {} locations moved, {} recipes moved ({} assets, prefix '{}')",
                    result.source_label,
                    result.target_label,
                    result.locations_moved,
                    result.recipes_moved,
                    result.assets_affected,
                    result.path_prefix,
                );
            } else {
                println!(
                    "Would combine '{}' into '{}': {} locations, {} recipes ({} assets, prefix '{}')",
                    result.source_label,
                    result.target_label,
                    result.locations,
                    result.recipes,
                    result.assets_affected,
                    result.path_prefix,
                );
                if result.locations > 0 || result.recipes > 0 {
                    println!("  Run with --apply to combine.");
                }
            }
        }
        Ok(())
    }
    VolumeCommands::Split { source, new_label, path, purpose, apply } => {
        let (catalog_root, config) = maki::config::load_config()?;
        let service = AssetService::new(&catalog_root, verbosity, &config.preview);

        let show_log = cli.log;
        let result = service.split_volume(
            &source,
            &new_label,
            &path,
            purpose.as_deref(),
            apply,
            |asset_id, elapsed| {
                if show_log {
                    item_status(asset_id, "updated", Some(elapsed));
                }
            },
        )?;

        if cli.json {
            println!("{}", serde_json::to_string_pretty(&result)?);
        } else {
            for err in &result.errors {
                eprintln!("  {err}");
            }

            if apply {
                println!(
                    "Volume '{}' split: new volume '{}' created with {} locations, {} recipes ({} assets, prefix '{}')",
                    result.source_label,
                    result.new_label,
                    result.locations_moved,
                    result.recipes_moved,
                    result.assets_affected,
                    result.path_prefix,
                );
            } else {
                println!(
                    "Would split '{}': new volume '{}' with {} locations, {} recipes ({} assets, prefix '{}')",
                    result.source_label,
                    result.new_label,
                    result.locations,
                    result.recipes,
                    result.assets_affected,
                    result.path_prefix,
                );
                if result.locations > 0 || result.recipes > 0 {
                    println!("  Run with --apply to split.");
                }
            }
        }
        Ok(())
    }
    VolumeCommands::Rename { volume, new_label } => {
        let catalog_root = maki::config::find_catalog_root()?;
        let registry = DeviceRegistry::new(&catalog_root);
        let vol = registry.resolve_volume(&volume)?;
        let old_label = vol.label.clone();

        registry.rename(&volume, &new_label)?;

        let catalog = maki::catalog::Catalog::open(&catalog_root)?;
        catalog.rename_volume(&vol.id.to_string(), &new_label)?;

        if cli.json {
            println!("{}", serde_json::json!({
                "old_label": old_label,
                "new_label": new_label,
                "volume_id": vol.id.to_string(),
            }));
        } else {
            println!("Volume '{}' renamed to '{}'", old_label, new_label);
        }
        Ok(())
    }
    }
}

/// Extracted body of `Commands::SavedSearch`.
pub fn run_saved_search_command(
    cmd: SavedSearchCommands,
    json: bool,
    log: bool,
    #[allow(unused_variables)] verbosity: maki::Verbosity,
) -> anyhow::Result<()> {
    #[allow(dead_code)]
    struct Ctx { json: bool, log: bool }
    let cli = Ctx { json, log };
    let _ = verbosity;
    let catalog_root = maki::config::find_catalog_root()?;
    match cmd {
        SavedSearchCommands::Save { name, query, sort, favorite } => {
            let mut file = maki::saved_search::load(&catalog_root)?;
            // Replace existing entry with same name, or append
            let entry = maki::saved_search::SavedSearch {
                name: name.clone(),
                query,
                sort,
                favorite,
            };
            if let Some(existing) = file.searches.iter_mut().find(|s| s.name == name) {
                *existing = entry;
            } else {
                file.searches.push(entry);
            }
            maki::saved_search::save(&catalog_root, &file)?;
            if cli.json {
                println!("{}", serde_json::json!({
                    "status": "saved",
                    "name": name,
                }));
            } else {
                println!("Saved search '{name}'");
            }
            Ok(())
        }
        SavedSearchCommands::List => {
            let file = maki::saved_search::load(&catalog_root)?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&file.searches)?);
            } else if file.searches.is_empty() {
                println!("No saved searches.");
            } else {
                for ss in &file.searches {
                    let sort_info = ss.sort.as_deref().unwrap_or("date_desc");
                    let fav = if ss.favorite { " [*]" } else { "" };
                    println!("  {}{} — {} (sort: {})", ss.name, fav, ss.query, sort_info);
                }
            }
            Ok(())
        }
        SavedSearchCommands::Run { name, format } => {
            use maki::format::{self, OutputFormat};

            let file = maki::saved_search::load(&catalog_root)?;
            let ss = maki::saved_search::find_by_name(&file, &name)
                .ok_or_else(|| anyhow::anyhow!("no saved search named '{name}'"))?;

            let engine = QueryEngine::new(&catalog_root);
            let results = engine.search(&ss.query)?;

            let output_format = if let Some(fmt) = &format {
                format::parse_format(fmt).map_err(|e| anyhow::anyhow!(e))?
            } else if cli.json {
                OutputFormat::Json
            } else {
                OutputFormat::Short
            };

            let explicit_format = format.is_some();

            if results.is_empty() {
                match output_format {
                    OutputFormat::Json => println!("[]"),
                    _ => {
                        if !explicit_format {
                            println!("No results found.");
                        }
                    }
                }
            } else {
                match output_format {
                    OutputFormat::Ids => {
                        for row in &results {
                            println!("{}", row.asset_id);
                        }
                    }
                    OutputFormat::Short => {
                        for row in &results {
                            let display_name = row
                                .name
                                .as_deref()
                                .unwrap_or(&row.original_filename);
                            let short_id = &row.asset_id[..8];
                            println!(
                                "{}  {} [{}] ({}) — {}",
                                short_id, display_name, row.asset_type, row.display_format(), row.created_at
                            );
                        }
                        if !explicit_format {
                            println!("\n{} result(s)", results.len());
                        }
                    }
                    OutputFormat::Full => {
                        for row in &results {
                            let display_name = row
                                .name
                                .as_deref()
                                .unwrap_or(&row.original_filename);
                            let short_id = &row.asset_id[..8];
                            let tags = if row.tags.is_empty() {
                                String::new()
                            } else {
                                format!(" tags:{}", row.tags.join(","))
                            };
                            let desc = row.description.as_deref().unwrap_or("");
                            println!(
                                "{}  {} [{}] ({}) — {}{} {}",
                                short_id, display_name, row.asset_type, row.display_format(),
                                row.created_at, tags, desc
                            );
                        }
                        if !explicit_format {
                            println!("\n{} result(s)", results.len());
                        }
                    }
                    OutputFormat::Json => {
                        println!("{}", serde_json::to_string_pretty(&results)?);
                    }
                    OutputFormat::Template(ref tpl) => {
                        for row in &results {
                            let tags_str = row.tags.join(", ");
                            let desc = row.description.as_deref().unwrap_or("");
                            let label = row.color_label.as_deref().unwrap_or("");
                            let values = format::search_row_values(
                                &row.asset_id,
                                row.name.as_deref(),
                                &row.original_filename,
                                &row.asset_type,
                                row.display_format(),
                                &row.created_at,
                                &tags_str,
                                desc,
                                &row.content_hash,
                                label,
                            );
                            println!("{}", format::render_template(tpl, &values));
                        }
                    }
                }
            }
            Ok(())
        }
        SavedSearchCommands::Delete { name } => {
            let mut file = maki::saved_search::load(&catalog_root)?;
            let before = file.searches.len();
            file.searches.retain(|s| s.name != name);
            if file.searches.len() == before {
                anyhow::bail!("no saved search named '{name}'");
            }
            maki::saved_search::save(&catalog_root, &file)?;
            if cli.json {
                println!("{}", serde_json::json!({
                    "status": "deleted",
                    "name": name,
                }));
            } else {
                println!("Deleted saved search '{name}'");
            }
            Ok(())
        }
    }
}

/// Extracted body of `Commands::Stack`.
pub fn run_stack_command(
    cmd: StackCommands,
    json: bool,
    log: bool,
    #[allow(unused_variables)] verbosity: maki::Verbosity,
) -> anyhow::Result<()> {
    #[allow(dead_code)]
    struct Ctx { json: bool, log: bool }
    let cli = Ctx { json, log };
    let _ = verbosity;
    let catalog_root = maki::config::find_catalog_root()?;
    let catalog = Catalog::open(&catalog_root)?;
    let store = maki::stack::StackStore::new(catalog.conn());
    match cmd {
        StackCommands::Create { asset_ids } => {
            if asset_ids.len() < 2 {
                anyhow::bail!("a stack requires at least 2 assets");
            }
            let stack = store.create(&asset_ids)?;
            let yaml = store.export_all()?;
            maki::stack::save_yaml(&catalog_root, &yaml)?;
            if cli.json {
                println!("{}", serde_json::json!({
                    "id": stack.id.to_string(),
                    "member_count": stack.asset_ids.len(),
                    "pick": stack.asset_ids[0],
                }));
            } else {
                println!("Created stack {} ({} assets, pick: {})",
                    &stack.id.to_string()[..8],
                    stack.asset_ids.len(),
                    &stack.asset_ids[0][..8.min(stack.asset_ids[0].len())]);
            }
            Ok(())
        }
        StackCommands::Add { reference, asset_ids } => {
            let added = store.add(&reference, &asset_ids)?;
            let yaml = store.export_all()?;
            maki::stack::save_yaml(&catalog_root, &yaml)?;
            if cli.json {
                println!("{}", serde_json::json!({ "added": added }));
            } else {
                println!("Added {} asset(s) to stack", added);
            }
            Ok(())
        }
        StackCommands::Remove { asset_ids } => {
            if asset_ids.is_empty() {
                anyhow::bail!("no asset IDs specified.");
            }
            let removed = store.remove(&asset_ids)?;
            let yaml = store.export_all()?;
            maki::stack::save_yaml(&catalog_root, &yaml)?;
            if cli.json {
                println!("{}", serde_json::json!({ "removed": removed }));
            } else {
                println!("Removed {} asset(s) from stack(s)", removed);
            }
            Ok(())
        }
        StackCommands::Pick { asset_id } => {
            store.set_pick(&asset_id)?;
            let yaml = store.export_all()?;
            maki::stack::save_yaml(&catalog_root, &yaml)?;
            if cli.json {
                println!("{}", serde_json::json!({ "pick": asset_id }));
            } else {
                println!("Set {} as stack pick", &asset_id[..8.min(asset_id.len())]);
            }
            Ok(())
        }
        StackCommands::Dissolve { asset_id } => {
            store.dissolve(&asset_id)?;
            let yaml = store.export_all()?;
            maki::stack::save_yaml(&catalog_root, &yaml)?;
            if cli.json {
                println!("{}", serde_json::json!({ "status": "dissolved" }));
            } else {
                println!("Stack dissolved");
            }
            Ok(())
        }
        StackCommands::List => {
            let list = store.list()?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&list)?);
            } else if list.is_empty() {
                println!("No stacks.");
            } else {
                for s in &list {
                    let pick = s.pick_asset_id.as_deref().unwrap_or("?");
                    let short_id = &s.id[..8.min(s.id.len())];
                    let short_pick = &pick[..8.min(pick.len())];
                    println!("  {} ({} assets, pick: {})", short_id, s.member_count, short_pick);
                }
            }
            Ok(())
        }
        StackCommands::Show { asset_id, format } => {
            let (stack_id, members) = store.stack_for_asset(&asset_id)?
                .ok_or_else(|| anyhow::anyhow!("asset {asset_id} is not in a stack"))?;
            if cli.json {
                println!("{}", serde_json::json!({
                    "stack_id": stack_id,
                    "members": members,
                    "pick": members.first(),
                }));
            } else if let Some(ref fmt) = format {
                if fmt == "ids" {
                    for id in &members {
                        println!("{}", id);
                    }
                } else {
                    let short_sid = &stack_id[..8.min(stack_id.len())];
                    println!("Stack {}:", short_sid);
                    for (i, id) in members.iter().enumerate() {
                        let marker = if i == 0 { " [pick]" } else { "" };
                        println!("  {}{}", id, marker);
                    }
                }
            } else {
                let short_sid = &stack_id[..8.min(stack_id.len())];
                println!("Stack {}:", short_sid);
                for (i, id) in members.iter().enumerate() {
                    let marker = if i == 0 { " [pick]" } else { "" };
                    println!("  {}{}", id, marker);
                }
            }
            Ok(())
        }
        StackCommands::FromTag { pattern, remove_tags, apply } => {
            let engine = QueryEngine::new(&catalog_root);
            let result = engine.stack_from_tag(&pattern, remove_tags, apply, cli.log)?;

            if cli.json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                let mode = if result.dry_run { " (dry run)" } else { "" };
                println!("Tags matched: {}{}", result.tags_matched, mode);
                println!("Tags skipped: {}", result.tags_skipped);
                println!("Stacks created: {}", result.stacks_created);
                println!("Assets stacked: {}", result.assets_stacked);
                println!("Assets already stacked (skipped): {}", result.assets_skipped);
                if remove_tags {
                    println!("Tags removed: {}", result.tags_removed);
                }
            }
            Ok(())
        }
    }
}

/// Extracted body of `Commands::Collection`.
pub fn run_collection_command(
    cmd: CollectionCommands,
    json: bool,
    log: bool,
    #[allow(unused_variables)] verbosity: maki::Verbosity,
) -> anyhow::Result<()> {
    #[allow(dead_code)]
    struct Ctx { json: bool, log: bool }
    let cli = Ctx { json, log };
    let _ = verbosity;
    let catalog_root = maki::config::find_catalog_root()?;
    let catalog = Catalog::open(&catalog_root)?;
    let store = maki::collection::CollectionStore::new(catalog.conn());
    match cmd {
        CollectionCommands::Create { name, description } => {
            let col = store.create(&name, description.as_deref())?;
            // Persist to YAML
            let yaml = store.export_all()?;
            maki::collection::save_yaml(&catalog_root, &yaml)?;
            if cli.json {
                println!("{}", serde_json::json!({
                    "id": col.id.to_string(),
                    "name": col.name,
                }));
            } else {
                println!("Created collection '{}'", col.name);
            }
            Ok(())
        }
        CollectionCommands::List => {
            let list = store.list()?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&list)?);
            } else if list.is_empty() {
                println!("No collections.");
            } else {
                for c in &list {
                    let desc = c.description.as_deref().unwrap_or("");
                    if desc.is_empty() {
                        println!("  {} ({} assets)", c.name, c.asset_count);
                    } else {
                        println!("  {} ({} assets) — {}", c.name, c.asset_count, desc);
                    }
                }
            }
            Ok(())
        }
        CollectionCommands::Show { name, format } => {
            use maki::format::{self, OutputFormat};

            let col = store.get_by_name(&name)?
                .ok_or_else(|| anyhow::anyhow!("no collection named '{name}'"))?;

            if col.asset_ids.is_empty() {
                if cli.json {
                    println!("{}", serde_json::to_string_pretty(&col)?);
                } else {
                    println!("Collection '{}' is empty.", name);
                }
                return Ok(());
            }

            // Search with collection filter
            let engine = QueryEngine::new(&catalog_root);
            let query_str = format!("collection:{}", name);
            let results = engine.search(&query_str)?;

            let output_format = if let Some(fmt) = &format {
                format::parse_format(fmt).map_err(|e| anyhow::anyhow!(e))?
            } else if cli.json {
                OutputFormat::Json
            } else {
                OutputFormat::Short
            };

            let explicit_format = format.is_some();

            if results.is_empty() {
                match output_format {
                    OutputFormat::Json => println!("[]"),
                    _ => {
                        if !explicit_format {
                            println!("Collection '{}': no matching assets.", name);
                        }
                    }
                }
            } else {
                match output_format {
                    OutputFormat::Ids => {
                        for row in &results {
                            println!("{}", row.asset_id);
                        }
                    }
                    OutputFormat::Short => {
                        if !explicit_format {
                            println!("Collection '{}':", name);
                        }
                        for row in &results {
                            let display_name = row.name.as_deref().unwrap_or(&row.original_filename);
                            let short_id = &row.asset_id[..8];
                            println!("  {}  {} [{}] ({})", short_id, display_name, row.asset_type, row.display_format());
                        }
                        if !explicit_format {
                            println!("\n{} asset(s)", results.len());
                        }
                    }
                    OutputFormat::Full => {
                        if !explicit_format {
                            println!("Collection '{}':", name);
                        }
                        for row in &results {
                            let display_name = row.name.as_deref().unwrap_or(&row.original_filename);
                            let short_id = &row.asset_id[..8];
                            let tags = if row.tags.is_empty() {
                                String::new()
                            } else {
                                format!(" tags:{}", row.tags.join(","))
                            };
                            println!("  {}  {} [{}] ({}){}", short_id, display_name, row.asset_type, row.display_format(), tags);
                        }
                        if !explicit_format {
                            println!("\n{} asset(s)", results.len());
                        }
                    }
                    OutputFormat::Json => {
                        println!("{}", serde_json::to_string_pretty(&results)?);
                    }
                    OutputFormat::Template(ref tpl) => {
                        for row in &results {
                            let tags_str = row.tags.join(", ");
                            let desc = row.description.as_deref().unwrap_or("");
                            let label = row.color_label.as_deref().unwrap_or("");
                            let values = format::search_row_values(
                                &row.asset_id,
                                row.name.as_deref(),
                                &row.original_filename,
                                &row.asset_type,
                                row.display_format(),
                                &row.created_at,
                                &tags_str,
                                desc,
                                &row.content_hash,
                                label,
                            );
                            println!("{}", format::render_template(tpl, &values));
                        }
                    }
                }
            }
            Ok(())
        }
        CollectionCommands::Add { name, asset_ids } => {
            // Read from stdin if no IDs provided
            let ids = if asset_ids.is_empty() {
                use std::io::BufRead;
                std::io::stdin().lock().lines()
                    .filter_map(|l| l.ok())
                    .map(|l| l.trim().to_string())
                    .filter(|l| !l.is_empty())
                    .collect()
            } else {
                asset_ids
            };
            if ids.is_empty() {
                anyhow::bail!("no asset IDs specified.");
            }
            let added = store.add_assets(&name, &ids)?;
            // Persist to YAML
            let yaml = store.export_all()?;
            maki::collection::save_yaml(&catalog_root, &yaml)?;
            if cli.json {
                println!("{}", serde_json::json!({
                    "added": added,
                    "collection": name,
                }));
            } else {
                println!("Added {} asset(s) to '{}'", added, name);
            }
            Ok(())
        }
        CollectionCommands::Remove { name, asset_ids } => {
            if asset_ids.is_empty() {
                anyhow::bail!("no asset IDs specified.");
            }
            let removed = store.remove_assets(&name, &asset_ids)?;
            // Persist to YAML
            let yaml = store.export_all()?;
            maki::collection::save_yaml(&catalog_root, &yaml)?;
            if cli.json {
                println!("{}", serde_json::json!({
                    "removed": removed,
                    "collection": name,
                }));
            } else {
                println!("Removed {} asset(s) from '{}'", removed, name);
            }
            Ok(())
        }
        CollectionCommands::Delete { name } => {
            store.delete(&name)?;
            // Persist to YAML
            let yaml = store.export_all()?;
            maki::collection::save_yaml(&catalog_root, &yaml)?;
            if cli.json {
                println!("{}", serde_json::json!({
                    "status": "deleted",
                    "name": name,
                }));
            } else {
                println!("Deleted collection '{name}'");
            }
            Ok(())
        }
    }
}

/// Extracted body of `Commands::BackupStatus`. See `run_import_command` for the
/// extraction pattern.
pub fn run_backup_status_command(
        query: Option<String>,
        at_risk: bool,
        min_copies: u64,
        volume: Option<String>,
        format: Option<String>,
        quiet: bool,
        json: bool,
        log: bool,
        #[allow(unused_variables)] verbosity: maki::Verbosity,
) -> anyhow::Result<()> {
    #[allow(dead_code)]
    struct Ctx { json: bool, log: bool }
    let cli = Ctx { json, log };
    use maki::format::{self, OutputFormat};

    let catalog_root = maki::config::find_catalog_root()?;
    let catalog = Catalog::open(&catalog_root)?;
    let registry = DeviceRegistry::new(&catalog_root);
    let vol_list = registry.list()?;

    // Exclude media volumes from backup coverage (transient sources like memory cards)
    let volumes_info: Vec<(String, String, bool, Option<String>)> = vol_list
        .iter()
        .filter(|v| v.purpose.as_ref() != Some(&maki::models::VolumePurpose::Media))
        .map(|v| (v.label.clone(), v.id.to_string(), v.is_online, v.purpose.as_ref().map(|p| p.as_str().to_string())))
        .collect();

    // Resolve target volume if specified
    let target_volume = if let Some(ref vol_label) = volume {
        Some(registry.resolve_volume(vol_label)?)
    } else {
        None
    };
    let target_volume_id = target_volume.as_ref().map(|v| v.id.to_string());

    // Scope: optional query → asset IDs
    let scope_ids: Option<Vec<String>> = if let Some(ref q) = query {
        let engine = QueryEngine::new(&catalog_root);
        let results = engine.search(q)?;
        let ids: Vec<String> = results.iter().map(|r| r.asset_id.clone()).collect();
        Some(ids)
    } else {
        None
    };
    let scope_refs = scope_ids.as_deref();

    // Determine mode: at-risk listing vs overview
    let listing_mode = at_risk || quiet || format.is_some();

    if listing_mode {
        // Get at-risk IDs
        let risk_ids = if let Some(ref tvid) = target_volume_id {
            catalog.backup_status_missing_from_volume(scope_refs, tvid)?
        } else {
            catalog.backup_status_at_risk_ids(scope_refs, min_copies)?
        };

        // Fetch full SearchRow data for output formatting
        let results = if risk_ids.is_empty() {
            Vec::new()
        } else {
            let opts = maki::catalog::SearchOptions {
                collection_asset_ids: Some(&risk_ids),
                per_page: u32::MAX,
                ..Default::default()
            };
            catalog.search_paginated(&opts)?
        };

        let output_format = if quiet {
            OutputFormat::Ids
        } else if let Some(fmt) = &format {
            format::parse_format(fmt).map_err(|e| anyhow::anyhow!(e))?
        } else if cli.json {
            OutputFormat::Json
        } else {
            OutputFormat::Short
        };

        let explicit_format = quiet || format.is_some();

        if results.is_empty() {
            match output_format {
                OutputFormat::Json => println!("[]"),
                _ => {
                    if !explicit_format {
                        println!("No at-risk assets found.");
                    }
                }
            }
        } else {
            match output_format {
                OutputFormat::Ids => {
                    for row in &results {
                        println!("{}", row.asset_id);
                    }
                }
                OutputFormat::Short => {
                    for row in &results {
                        let display_name = row.name.as_deref().unwrap_or(&row.original_filename);
                        let short_id = &row.asset_id[..8];
                        println!(
                            "{}  {} [{}] ({}) — {}",
                            short_id, display_name, row.asset_type, row.display_format(), row.created_at
                        );
                    }
                    if !explicit_format {
                        println!("\n{} at-risk asset(s)", results.len());
                    }
                }
                OutputFormat::Full => {
                    for row in &results {
                        let display_name = row.name.as_deref().unwrap_or(&row.original_filename);
                        let short_id = &row.asset_id[..8];
                        let tags = if row.tags.is_empty() {
                            String::new()
                        } else {
                            format!(" tags:{}", row.tags.join(","))
                        };
                        let desc = row.description.as_deref().unwrap_or("");
                        println!(
                            "{}  {} [{}] ({}) — {}{} {}",
                            short_id, display_name, row.asset_type, row.display_format(),
                            row.created_at, tags, desc
                        );
                    }
                    if !explicit_format {
                        println!("\n{} at-risk asset(s)", results.len());
                    }
                }
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&results)?);
                }
                OutputFormat::Template(ref tpl) => {
                    for row in &results {
                        let tags_str = row.tags.join(", ");
                        let desc = row.description.as_deref().unwrap_or("");
                        let label = row.color_label.as_deref().unwrap_or("");
                        let values = format::search_row_values(
                            &row.asset_id,
                            row.name.as_deref(),
                            &row.original_filename,
                            &row.asset_type,
                            row.display_format(),
                            &row.created_at,
                            &tags_str,
                            desc,
                            &row.content_hash,
                            label,
                        );
                        println!("{}", format::render_template(tpl, &values));
                    }
                }
            }
        }
    } else {
        // Overview mode
        let result = catalog.backup_status_overview(
            scope_refs,
            &volumes_info,
            min_copies,
            target_volume_id.as_deref(),
        )?;

        if cli.json {
            println!("{}", serde_json::to_string_pretty(&result)?);
        } else {
            print_backup_status_human(&result);
        }
    }
    Ok(())
}

/// Extracted body of `Commands::Status`. See `run_import_command` for the
/// extraction pattern.
pub fn run_status_command(
        min_copies: u64,
        json: bool,
        log: bool,
        #[allow(unused_variables)] verbosity: maki::Verbosity,
) -> anyhow::Result<()> {
    #[allow(dead_code)]
    struct Ctx { json: bool, log: bool }
    let cli = Ctx { json, log };
    let (catalog_root, config) = maki::config::load_config()?;
    let ai_enabled = cfg!(feature = "ai");
    // The orphan-on-disk scan (cleanup passes 4-7) dominates runtime
    // on real catalogs — easily 30s+ on tens of thousands of files.
    // Emit a one-line "still alive" marker to stderr so the user
    // doesn't wonder whether the command crashed. Suppressed under
    // --json so scripted output stays clean.
    if !cli.json {
        eprintln!("Gathering catalog status (scanning derived files; may take a moment)...");
    }
    let report = maki::status::gather(
        &catalog_root,
        verbosity,
        &config.preview,
        min_copies,
        ai_enabled,
    )?;
    if cli.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print_status_human(&report);
    }
    Ok(())
}

/// Extracted body of `Commands::Stats`. See `run_import_command` for the
/// extraction pattern.
pub fn run_stats_command(
        types: bool,
        volumes: bool,
        tags: bool,
        verified: bool,
        all: bool,
        limit: usize,
        json: bool,
        log: bool,
        #[allow(unused_variables)] verbosity: maki::Verbosity,
) -> anyhow::Result<()> {
    #[allow(dead_code)]
    struct Ctx { json: bool, log: bool }
    let cli = Ctx { json, log };
    let catalog_root = maki::config::find_catalog_root()?;
    let catalog = Catalog::open(&catalog_root)?;
    let registry = DeviceRegistry::new(&catalog_root);
    let vol_list = registry.list()?;

    let volumes_info: Vec<(String, String, bool, Option<String>)> = vol_list
        .iter()
        .map(|v| (v.label.clone(), v.id.to_string(), v.is_online, v.purpose.as_ref().map(|p| p.as_str().to_string())))
        .collect();

    let show_types = types || all;
    let show_volumes = volumes || all;
    let show_tags = tags || all;
    let show_verified = verified || all;

    let stats = catalog.build_stats(
        &volumes_info,
        show_types,
        show_volumes,
        show_tags,
        show_verified,
        limit,
    )?;

    if cli.json {
        println!("{}", serde_json::to_string_pretty(&stats)?);
    } else {
        print_stats_human(&stats);
    }
    Ok(())
}

/// Extracted body of `Commands::Export`. See `run_import_command` for the
/// extraction pattern.
pub fn run_export_command(
        query: String,
        target: String,
        layout: String,
        symlink: bool,
        all_variants: bool,
        include_sidecars: bool,
        dry_run: bool,
        overwrite: bool,
        zip: bool,
        json: bool,
        log: bool,
        #[allow(unused_variables)] verbosity: maki::Verbosity,
) -> anyhow::Result<()> {
    #[allow(dead_code)]
    struct Ctx { json: bool, log: bool }
    let cli = Ctx { json, log };
    use maki::asset_service::{ExportLayout, ExportStatus};

    let export_layout = match layout.as_str() {
        "flat" => ExportLayout::Flat,
        "mirror" => ExportLayout::Mirror,
        _ => anyhow::bail!("unknown layout '{}'. Valid layouts: flat, mirror", layout),
    };

    let (catalog_root, config) = maki::config::load_config()?;
    let service = AssetService::new(&catalog_root, verbosity, &config.preview);

    let show_log = cli.log;
    let log_callback = |path: &std::path::Path, status: &ExportStatus, elapsed: std::time::Duration| {
        if show_log {
            let name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy();
            match status {
                ExportStatus::Copied => {
                    item_status(&name, "added", Some(elapsed));
                }
                ExportStatus::Linked => {
                    item_status(&name, "linked", Some(elapsed));
                }
                ExportStatus::Skipped => {
                    item_status(&name, "skipped", Some(elapsed));
                }
                ExportStatus::Error(msg) => {
                    eprintln!("  {name} — error: {msg}");
                }
            }
        }
    };

    let result = if zip {
        if symlink {
            anyhow::bail!("--symlink cannot be used with --zip");
        }
        let zip_path = PathBuf::from(&target);
        // Append .zip if not already present
        let zip_path = if zip_path.extension().and_then(|e| e.to_str()) != Some("zip") {
            zip_path.with_extension("zip")
        } else {
            zip_path
        };
        service.export_zip(
            &query,
            &zip_path,
            export_layout,
            all_variants,
            include_sidecars,
            log_callback,
        )?
    } else {
        let target_path = PathBuf::from(&target);
        if !dry_run {
            std::fs::create_dir_all(&target_path)?;
        }
        service.export(
            &query,
            &target_path,
            export_layout,
            symlink,
            all_variants,
            include_sidecars,
            dry_run,
            overwrite,
            log_callback,
        )?
    };

    if cli.json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        for err in &result.errors {
            eprintln!("  {err}");
        }

        if dry_run {
            println!("Export (dry run): {} assets matched, {} files would be exported",
                result.assets_matched, result.files_exported);
            if result.sidecars_exported > 0 {
                println!("  {} sidecars would be exported", result.sidecars_exported);
            }
            if result.total_bytes > 0 {
                println!("  Total size: {}", format_size(result.total_bytes));
            }
        } else if result.assets_matched == 0 {
            println!("No assets matched the query.");
        } else if zip {
            let mut parts = vec![
                format!("{} files archived", result.files_exported),
            ];
            if result.sidecars_exported > 0 {
                parts.push(format!("{} sidecars", result.sidecars_exported));
            }
            println!("Export complete: {}", parts.join(", "));
            if result.total_bytes > 0 {
                println!("  Total size: {}", format_size(result.total_bytes));
            }
            println!("  Written to: {target}");
        } else {
            let verb = if symlink { "linked" } else { "copied" };
            let mut parts = vec![
                format!("{} files {verb}", result.files_exported),
            ];
            if result.sidecars_exported > 0 {
                parts.push(format!("{} sidecars", result.sidecars_exported));
            }
            if result.files_skipped > 0 {
                parts.push(format!("{} skipped", result.files_skipped));
            }
            println!("Export complete: {}", parts.join(", "));
            if result.total_bytes > 0 {
                println!("  Total size: {}", format_size(result.total_bytes));
            }
        }
    }

    Ok(())
}

/// Extracted body of `Commands::ContactSheet`. See `run_import_command` for the
/// extraction pattern.
pub fn run_contact_sheet_command(
        query: String,
        output: String,
        layout: String,
        columns: Option<u32>,
        rows: Option<u32>,
        paper: String,
        landscape: bool,
        title: Option<String>,
        fields: Option<String>,
        sort: Option<String>,
        no_smart: bool,
        group_by: Option<String>,
        margin: Option<f32>,
        label_style: Option<String>,
        quality: Option<u8>,
        copyright: Option<String>,
        dry_run: bool,
        json: bool,
        log: bool,
        #[allow(unused_variables)] verbosity: maki::Verbosity,
) -> anyhow::Result<()> {
    #[allow(dead_code)]
    struct Ctx { json: bool, log: bool }
    let cli = Ctx { json, log };
    use maki::contact_sheet::{
        generate_contact_sheet, ContactSheetConfig, ContactSheetLayout,
        ContactSheetStatus, GroupByField, LabelStyle, MetadataField, PaperSize,
    };

    let (catalog_root, config) = maki::config::load_config()?;
    let cs_defaults = &config.contact_sheet;

    let cs_layout: ContactSheetLayout = layout.parse()?;
    let cs_paper: PaperSize = paper.parse()?;

    let cs_fields = if let Some(ref f) = fields {
        let parsed: Vec<MetadataField> = f
            .split(',')
            .map(|s| s.trim().parse::<MetadataField>())
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Some(parsed)
    } else if cs_defaults.fields != "filename,date,rating" {
        let parsed: Vec<MetadataField> = cs_defaults
            .fields
            .split(',')
            .map(|s| s.trim().parse::<MetadataField>())
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Some(parsed)
    } else {
        None // Use layout preset default
    };

    let cs_group_by = group_by
        .map(|g| g.parse::<GroupByField>())
        .transpose()?;

    let cs_label_style: LabelStyle = label_style
        .unwrap_or_else(|| cs_defaults.label_style.clone())
        .parse()?;

    let cs_quality = quality.unwrap_or(cs_defaults.quality);
    let cs_margin = margin.unwrap_or(cs_defaults.margin);

    let cs_config = ContactSheetConfig {
        layout: cs_layout,
        columns,
        rows,
        paper: cs_paper,
        landscape,
        title,
        fields: cs_fields,
        sort,
        use_smart_previews: !no_smart,
        group_by: cs_group_by,
        margin_mm: cs_margin,
        label_style: cs_label_style,
        quality: cs_quality,
        copyright: copyright.or_else(|| cs_defaults.copyright.clone()),
    };

    let output_path = PathBuf::from(&output);
    let show_log = cli.log;

    let result = generate_contact_sheet(
        &catalog_root,
        &query,
        &output_path,
        &cs_config,
        dry_run,
        |msg, status, elapsed| {
            if show_log || matches!(status, ContactSheetStatus::Complete) {
                match status {
                    ContactSheetStatus::Rendering => {
                        eprintln!("  {} ({})", msg, format_duration(elapsed));
                    }
                    ContactSheetStatus::Complete => {
                        if !cli.json {
                            eprintln!("{}", msg);
                        }
                    }
                    ContactSheetStatus::Error => {
                        eprintln!("  Error: {}", msg);
                    }
                }
            }
        },
    )?;

    if cli.json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else if !dry_run {
        println!(
            "Contact sheet: {} assets, {} pages → {}",
            result.assets, result.pages, result.output,
        );
    }

    Ok(())
}

/// Extracted body of `Commands::Serve`. See `run_import_command` for the
/// extraction pattern.
pub fn run_serve_command(
        port: Option<u16>,
        bind: Option<String>,
        json: bool,
        log: bool,
        #[allow(unused_variables)] verbosity: maki::Verbosity,
) -> anyhow::Result<()> {
    #[allow(dead_code)]
    struct Ctx { json: bool, log: bool }
    let cli = Ctx { json, log };
    let (catalog_root, config) = maki::config::load_config()?;
    let port = port.unwrap_or(config.serve.port);
    let bind = bind.unwrap_or_else(|| config.serve.bind.clone());
    let rt = tokio::runtime::Runtime::new()?;
    #[cfg(feature = "ai")]
    rt.block_on(maki::web::serve(catalog_root, &bind, port, config.preview, cli.log, config.dedup.prefer, config.serve.per_page, config.serve.stroll_neighbors, config.serve.stroll_neighbors_max, config.serve.stroll_fanout, config.serve.stroll_fanout_max, config.serve.stroll_discover_pool, config.ai, config.vlm, config.browse.default_filter, verbosity))?;
    #[cfg(not(feature = "ai"))]
    rt.block_on(maki::web::serve(catalog_root, &bind, port, config.preview, cli.log, config.dedup.prefer, config.serve.per_page, config.serve.stroll_neighbors, config.serve.stroll_neighbors_max, config.serve.stroll_fanout, config.serve.stroll_fanout_max, config.serve.stroll_discover_pool, config.vlm, config.browse.default_filter, verbosity))?;
    Ok(())
}

/// Extracted body of `Commands::Doc`. See `run_import_command` for the
/// extraction pattern.
pub fn run_doc_command(
        document: String,
        json: bool,
        log: bool,
        #[allow(unused_variables)] verbosity: maki::Verbosity,
) -> anyhow::Result<()> {
    #[allow(dead_code)]
    struct Ctx { json: bool, log: bool }
    let cli = Ctx { json, log };
    let url = match document.to_lowercase().as_str() {
        "manual" | "man" | "guide" => "https://github.com/thoherr/maki/releases/latest/download/maki-manual.pdf",
        "cheatsheet" | "cheat" | "cs" => "https://github.com/thoherr/maki/releases/latest/download/cheat-sheet.pdf",
        "filters" | "search" | "filter" | "sf" => "https://github.com/thoherr/maki/releases/latest/download/search-filters.pdf",
        "tagging" | "tags" | "tag" => "https://github.com/thoherr/maki/releases/latest/download/tagging.pdf",
        _ => {
            anyhow::bail!("unknown document '{}'. Available: manual, cheatsheet, filters, tagging", document);
        }
    };
    if cli.json {
        println!("{}", serde_json::json!({ "url": url }));
    } else {
        println!("Opening {url}");
        #[cfg(target_os = "macos")]
        { let _ = std::process::Command::new("open").arg(url).spawn(); }
        #[cfg(target_os = "linux")]
        { let _ = std::process::Command::new("xdg-open").arg(url).spawn(); }
        #[cfg(target_os = "windows")]
        { let _ = std::process::Command::new("cmd").args(["/c", "start", url]).spawn(); }
    }
    Ok(())
}

/// Extracted body of `Commands::Licenses`. See `run_import_command` for the
/// extraction pattern.
pub fn run_licenses_command(
        summary: bool,
        json: bool,
        log: bool,
        #[allow(unused_variables)] verbosity: maki::Verbosity,
) -> anyhow::Result<()> {
    #[allow(dead_code)]
    struct Ctx { json: bool, log: bool }
    let cli = Ctx { json, log };
    let third_party_url = "https://github.com/thoherr/maki/releases/latest/download/THIRD_PARTY_LICENSES.md";

    if cli.json {
        println!("{}", serde_json::json!({
            "maki_license": "Apache-2.0",
            "maki_license_url": "https://github.com/thoherr/maki/blob/main/LICENSE",
            "third_party_licenses_url": third_party_url,
            "third_party_summary": "All Rust dependencies use permissive licenses (MIT, Apache-2.0, BSD, ISC, MPL-2.0, NCSA, Unicode, Zlib, BSL-1.0, CC0). See THIRD_PARTY_LICENSES.md in the release archive.",
            "ai_models": {
                "license": "Apache-2.0",
                "source": "Hugging Face (downloaded on demand)",
                "credit": "Google Research (SigLIP, SigLIP 2)",
            },
            "external_tools": "dcraw, libraw, ffmpeg, curl — installed separately by the user under their own licenses; not bundled.",
        }));
        return Ok(());
    }

    println!("MAKI — Media Asset Keeper & Indexer");
    println!("License: Apache-2.0");
    println!("https://github.com/thoherr/maki/blob/main/LICENSE");
    println!();
    println!("Third-party Rust dependencies");
    println!("─────────────────────────────");
    println!("All compiled-in Rust crates use permissive open-source licenses:");
    println!("  Apache-2.0, MIT, BSD-2/3-Clause, ISC, MPL-2.0, NCSA, Unicode, Zlib,");
    println!("  BSL-1.0, CC0, 0BSD.");
    println!();
    println!("The full license text for every dependency is in:");
    println!("  THIRD_PARTY_LICENSES.md (in your MAKI release archive)");
    println!("  {third_party_url}");
    println!();
    println!("AI models (Pro)");
    println!("───────────────");
    println!("SigLIP and SigLIP 2 image-text encoders are downloaded on demand from");
    println!("Hugging Face. Both are released by Google Research under Apache-2.0.");
    println!("Face detection/recognition models (when used) are also Apache-2.0.");
    println!("MAKI does not bundle model weights in the binary.");
    println!();
    println!("External tools");
    println!("──────────────");
    println!("dcraw, libraw, ffmpeg, ffprobe, and curl are called as separate processes");
    println!("when present on the system. They are governed by their own licenses and");
    println!("installed by the user; MAKI does not bundle their code.");

    if !summary {
        println!();
        println!("Run 'maki licenses --summary' for a shorter version, or open the");
        println!("THIRD_PARTY_LICENSES.md file in your release archive for the full");
        println!("text of every dependency's license.");
    }

    Ok(())
}

/// Extracted body of `Commands::CreateSidecars`. See `run_import_command` for the
/// extraction pattern.
pub fn run_create_sidecars_command(
        query: Option<String>,
        volume: Option<String>,
        asset: Option<String>,
        apply: bool,
        asset_ids: Vec<String>,
        json: bool,
        log: bool,
        #[allow(unused_variables)] verbosity: maki::Verbosity,
) -> anyhow::Result<()> {
    #[allow(dead_code)]
    struct Ctx { json: bool, log: bool }
    let cli = Ctx { json, log };
    let catalog_root = maki::config::find_catalog_root()?;
    let catalog = Catalog::open(&catalog_root)?;
    let metadata_store = MetadataStore::new(&catalog_root);
    let content_store = maki::content_store::ContentStore::new(&catalog_root);
    let registry = DeviceRegistry::new(&catalog_root);
    let engine = maki::query::QueryEngine::new(&catalog_root);
    let volumes = registry.list()?;

    // Resolve target volume filter
    let volume_filter = if let Some(ref vol_label) = volume {
        Some(registry.resolve_volume(vol_label)?)
    } else {
        None
    };

    // Resolve asset scope
    let scope = engine.resolve_scope(query.as_deref(), asset.as_deref(), &asset_ids)?;
    let summaries = metadata_store.list()?;
    let ids: Vec<uuid::Uuid> = match scope {
        Some(set) => set.iter().filter_map(|id| id.parse().ok()).collect(),
        None => summaries.iter().map(|s| s.id).collect(),
    };

    let mut created = 0usize;
    let mut skipped = 0usize;
    let mut checked = 0usize;

    for asset_id in &ids {
        let mut asset = match metadata_store.load(*asset_id) {
            Ok(a) => a,
            Err(_) => continue,
        };

        let has_metadata = !asset.tags.is_empty()
            || asset.rating.is_some()
            || asset.color_label.is_some()
            || asset.description.is_some();

        if !has_metadata {
            skipped += 1;
            continue;
        }

        checked += 1;
        let mut asset_changed = false;

        for variant in &asset.variants {
            for loc in &variant.locations {
                // Filter by volume if specified
                if let Some(ref vf) = volume_filter {
                    if loc.volume_id != vf.id {
                        continue;
                    }
                }

                // Check if volume is online
                let vol = match volumes.iter().find(|v| v.id == loc.volume_id) {
                    Some(v) if v.mount_point.exists() => v,
                    _ => continue,
                };

                // Check if this variant already has an XMP recipe on this volume
                let has_xmp = asset.recipes.iter().any(|r| {
                    r.variant_hash == variant.content_hash
                        && r.location.volume_id == loc.volume_id
                        && r.recipe_type == maki::models::recipe::RecipeType::Sidecar
                });
                if has_xmp {
                    continue;
                }

                // Build XMP sidecar path
                let xmp_relative = loc.relative_path.with_extension(
                    format!("{}.xmp", loc.relative_path.extension().unwrap_or_default().to_string_lossy())
                );
                let xmp_path = vol.mount_point.join(&xmp_relative);

                if cli.log {
                    let name = xmp_relative.file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("?");
                    if apply {
                        eprintln!("  {} — created", name);
                    } else {
                        eprintln!("  {} — would create", name);
                    }
                }

                if apply {
                    if let Some(parent) = xmp_path.parent() {
                        std::fs::create_dir_all(parent)?;
                    }

                    let xmp_content = maki::xmp_reader::create_xmp(
                        &asset.tags,
                        asset.rating,
                        asset.color_label.as_deref(),
                        asset.description.as_deref(),
                    );

                    std::fs::write(&xmp_path, &xmp_content)?;
                    let xmp_hash = content_store.hash_file(&xmp_path)?;

                    let recipe = maki::models::recipe::Recipe {
                        id: uuid::Uuid::new_v4(),
                        variant_hash: variant.content_hash.clone(),
                        software: "MAKI".to_string(),
                        recipe_type: maki::models::recipe::RecipeType::Sidecar,
                        content_hash: xmp_hash,
                        location: maki::models::FileLocation {
                            volume_id: loc.volume_id,
                            relative_path: xmp_relative,
                            verified_at: None,
                        },
                        pending_writeback: false,
                    };
                    catalog.insert_recipe(&recipe)?;
                    asset.recipes.push(recipe);
                    asset_changed = true;
                }

                created += 1;
            }
        }

        if asset_changed {
            metadata_store.save(&asset)?;
        }
    }

    if cli.json {
        let result = serde_json::json!({
            "dry_run": !apply,
            "checked": checked,
            "created": created,
            "skipped_no_metadata": skipped,
        });
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        if !apply && created > 0 {
            eprint!("Dry run — ");
        }
        println!("Create-sidecars: {} checked, {} created, {} skipped (no metadata)", checked, created, skipped);
        if !apply && created > 0 {
            println!("  Run with --apply to create XMP files.");
        }
    }

    Ok(())
}

/// Extracted body of `Commands::FixRecipes`. See `run_import_command` for the
/// extraction pattern.
pub fn run_fix_recipes_command(
        query: Option<String>,
        volume: Option<String>,
        asset: Option<String>,
        apply: bool,
        asset_ids: Vec<String>,
        json: bool,
        log: bool,
        #[allow(unused_variables)] verbosity: maki::Verbosity,
) -> anyhow::Result<()> {
    #[allow(dead_code)]
    struct Ctx { json: bool, log: bool }
    let cli = Ctx { json, log };
    let (catalog_root, config) = maki::config::load_config()?;
    let service = AssetService::new(&catalog_root, verbosity, &config.preview);
    let engine = maki::query::QueryEngine::new(&catalog_root);

    // Resolve scope (query/asset/asset_ids) to individual asset IDs
    let scope = engine.resolve_scope(query.as_deref(), asset.as_deref(), &asset_ids)?;
    let asset_id_list: Vec<Option<String>> = match scope {
        Some(set) => set.into_iter().map(Some).collect(),
        None => vec![None], // process all
    };

    let show_log = cli.log;
    let mut result = maki::asset_service::FixRecipesResult { dry_run: !apply, ..Default::default() };
    for aid in &asset_id_list {
        let r = service.fix_recipes(
            volume.as_deref(),
            aid.as_deref(),
            apply,
            |name, status| {
                if show_log {
                    let label = match status {
                        maki::asset_service::FixRecipesStatus::Reattached => {
                            if apply { "reattached" } else { "would reattach" }
                        }
                        maki::asset_service::FixRecipesStatus::NoParentFound => "no parent found",
                        maki::asset_service::FixRecipesStatus::Skipped => "skipped",
                    };
                    eprintln!("  {} — {}", name, label);
                }
            },
        )?;
        result.checked += r.checked;
        result.reattached += r.reattached;
        result.no_parent += r.no_parent;
        result.skipped += r.skipped;
        result.errors.extend(r.errors);
    }

    if cli.json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        for err in &result.errors {
            eprintln!("  {err}");
        }

        if !apply && result.reattached > 0 {
            eprint!("Dry run — ");
        }

        let mut parts = vec![
            format!("{} checked", result.checked),
            format!("{} reattached", result.reattached),
        ];
        if result.no_parent > 0 {
            parts.push(format!("{} no parent found", result.no_parent));
        }
        if result.skipped > 0 {
            parts.push(format!("{} skipped", result.skipped));
        }

        println!("Fix-recipes: {}", parts.join(", "));

        if !apply && result.reattached > 0 {
            println!("  Run with --apply to make changes.");
        }
    }

    Ok(())
}

/// Extracted body of `Commands::FixDates`. See `run_import_command` for the
/// extraction pattern.
pub fn run_fix_dates_command(
        query: Option<String>,
        volume: Option<String>,
        asset: Option<String>,
        apply: bool,
        asset_ids: Vec<String>,
        json: bool,
        log: bool,
        #[allow(unused_variables)] verbosity: maki::Verbosity,
) -> anyhow::Result<()> {
    #[allow(dead_code)]
    struct Ctx { json: bool, log: bool }
    let cli = Ctx { json, log };
    let (catalog_root, config) = maki::config::load_config()?;
    let service = AssetService::new(&catalog_root, verbosity, &config.preview);
    let engine = maki::query::QueryEngine::new(&catalog_root);

    // Resolve scope (query/asset/asset_ids) to individual asset IDs
    let scope = engine.resolve_scope(query.as_deref(), asset.as_deref(), &asset_ids)?;
    let asset_id_list: Vec<Option<String>> = match scope {
        Some(set) => set.into_iter().map(Some).collect(),
        None => vec![None], // process all
    };

    let show_log = cli.log;
    let mut result = maki::asset_service::FixDatesResult { dry_run: !apply, ..Default::default() };
    for aid in &asset_id_list {
        let r = service.fix_dates(
            volume.as_deref(),
            aid.as_deref(),
            apply,
            |name, status, detail| {
                if show_log {
                    let label = match status {
                        maki::asset_service::FixDatesStatus::AlreadyCorrect => "ok".to_string(),
                        maki::asset_service::FixDatesStatus::NoDate => "no date available".to_string(),
                        maki::asset_service::FixDatesStatus::SkippedOffline => "skipped (volume offline)".to_string(),
                        maki::asset_service::FixDatesStatus::Fixed => {
                            let action = if apply { "fixed" } else { "would fix" };
                            if let Some(d) = detail {
                                format!("{action}: {d}")
                            } else {
                                action.to_string()
                            }
                        }
                    };
                    eprintln!("  {} — {}", name, label);
                }
            },
        )?;
        result.checked += r.checked;
        result.fixed += r.fixed;
        result.already_correct += r.already_correct;
        result.skipped_offline += r.skipped_offline;
        result.no_date += r.no_date;
        result.errors.extend(r.errors);
        for v in r.offline_volumes {
            if !result.offline_volumes.contains(&v) {
                result.offline_volumes.push(v);
            }
        }
    }

    if cli.json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        // Print offline volume warnings
        if !result.offline_volumes.is_empty() {
            for vol_label in &result.offline_volumes {
                eprintln!("Warning: volume '{}' is offline — cannot read files for date extraction", vol_label);
            }
        }

        for err in &result.errors {
            eprintln!("  {err}");
        }

        if !apply && result.fixed > 0 {
            eprint!("Dry run — ");
        }

        let mut parts = vec![
            format!("{} checked", result.checked),
            format!("{} fixed", result.fixed),
            format!("{} already correct", result.already_correct),
        ];
        if result.skipped_offline > 0 {
            parts.push(format!("{} skipped (volume offline)", result.skipped_offline));
        }
        if result.no_date > 0 {
            parts.push(format!("{} no date available", result.no_date));
        }

        println!("Fix-dates: {}", parts.join(", "));

        if !apply && result.fixed > 0 {
            println!("  Run with --apply to make changes.");
        }
        if result.skipped_offline > 0 {
            println!("  Mount offline volumes and re-run to fix remaining assets.");
        }
    }

    Ok(())
}

/// Extracted body of `Commands::FixRoles`. See `run_import_command` for the
/// extraction pattern.
pub fn run_fix_roles_command(
        paths: Vec<String>,
        volume: Option<String>,
        asset: Option<String>,
        apply: bool,
        json: bool,
        log: bool,
        #[allow(unused_variables)] verbosity: maki::Verbosity,
) -> anyhow::Result<()> {
    #[allow(dead_code)]
    struct Ctx { json: bool, log: bool }
    let cli = Ctx { json, log };
    let (catalog_root, config) = maki::config::load_config()?;
    let service = AssetService::new(&catalog_root, verbosity, &config.preview);

    let canonical_paths: Vec<PathBuf> = paths
        .iter()
        .map(|p| {
            std::fs::canonicalize(p)
                .unwrap_or_else(|_| PathBuf::from(p))
        })
        .collect();

    let show_log = cli.log;
    let result = service.fix_roles(
        &canonical_paths,
        volume.as_deref(),
        asset.as_deref(),
        apply,
        |name, status| {
            if show_log {
                let label = match status {
                    maki::asset_service::FixRolesStatus::AlreadyCorrect => "ok",
                    maki::asset_service::FixRolesStatus::Fixed => {
                        if apply { "fixed" } else { "would fix" }
                    }
                };
                eprintln!("  {} — {}", name, label);
            }
        },
    )?;

    if cli.json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        for err in &result.errors {
            eprintln!("  {err}");
        }

        if !apply && result.fixed > 0 {
            eprint!("Dry run — ");
        }

        println!(
            "Fix-roles: {} checked, {} fixed ({} variant(s)), {} already correct",
            result.checked, result.fixed, result.variants_fixed, result.already_correct
        );

        if !apply && result.fixed > 0 {
            println!("  Run with --apply to make changes.");
        }
        // Reordering variant roles changes which variant is selected for
        // preview generation. Cached previews still reflect the *old*
        // best variant — `generate-previews --upgrade` regenerates them
        // for assets whose best changed.
        if apply && result.fixed > 0 {
            println!(
                "  Tip: best-preview variant changed for {} asset(s). \
                 Run 'maki generate-previews --upgrade' to refresh their previews.",
                result.fixed
            );
        }
    }

    Ok(())
}

/// Extracted body of `Commands::GeneratePreviews`. See `run_import_command` for the
/// extraction pattern.
pub fn run_generate_previews_command(
        paths: Vec<String>,
        volume: Option<String>,
        asset: Option<String>,
        include: Vec<String>,
        skip: Vec<String>,
        force: bool,
        upgrade: bool,
        smart: bool,
        json: bool,
        log: bool,
        #[allow(unused_variables)] verbosity: maki::Verbosity,
) -> anyhow::Result<()> {
    #[allow(dead_code)]
    struct Ctx { json: bool, log: bool }
    let cli = Ctx { json, log };
    use maki::asset_service::FileTypeFilter;

    let (catalog_root, config) = maki::config::load_config()?;
    let preview_gen = maki::preview::PreviewGenerator::new(&catalog_root, verbosity, &config.preview);
    let metadata_store = MetadataStore::new(&catalog_root);
    let registry = maki::device_registry::DeviceRegistry::new(&catalog_root);
    let catalog = maki::catalog::Catalog::open(&catalog_root)?;
    let volumes = registry.list()?;

    // Build file type filter
    let mut filter = FileTypeFilter::default();
    for group in &include {
        if skip.contains(group) {
            anyhow::bail!(
                "Group '{}' cannot be both included and skipped.",
                group
            );
        }
    }
    for group in &include {
        filter.include(group)?;
    }
    for group in &skip {
        filter.skip(group)?;
    }

    let mut generated = 0usize;
    let mut skipped = 0usize;
    let mut failed = 0usize;
    let mut upgraded = 0usize;
    // Volumes that held the only locations of variants we couldn't
    // process because they're offline. Surfaced at the end so the user
    // knows which disk to mount instead of seeing a silent skip.
    let mut offline_blockers: std::collections::HashSet<String> = std::collections::HashSet::new();

    // Canonicalize input paths
    let canonical_paths: Vec<PathBuf> = paths
        .iter()
        .map(|p| {
            std::fs::canonicalize(p)
                .unwrap_or_else(|_| PathBuf::from(p))
        })
        .collect();

    if !canonical_paths.is_empty() {
        // PATHS mode: resolve files, look up each in catalog
        let files = maki::asset_service::resolve_files(&canonical_paths, &config.import.exclude);
        let content_store = maki::content_store::ContentStore::new(&catalog_root);

        for file_path in &files {
            // Filter by extension
            let ext = file_path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("");
            if !ext.is_empty() && !filter.is_importable(ext) {
                continue;
            }

            // Look up variant in catalog: try volume+path first, fall back to content hash
            let lookup = {
                let vol = volumes.iter().find(|v| file_path.starts_with(&v.mount_point));
                if let Some(v) = vol {
                    let relative_path = file_path
                        .strip_prefix(&v.mount_point)
                        .unwrap_or(file_path);
                    catalog.find_variant_by_volume_and_path(
                        &v.id.to_string(),
                        &relative_path.to_string_lossy(),
                    )?
                } else {
                    None
                }
            };
            // Fall back to hashing the file and looking up by content hash
            let lookup = match lookup {
                Some(v) => Some(v),
                None => {
                    let hash = content_store.hash_file(file_path)?;
                    catalog.get_variant_format(&hash)?.map(|fmt| (hash, fmt))
                }
            };

            if let Some((content_hash, format)) = lookup {
                let file_start = std::time::Instant::now();
                // Generate regular preview (always)
                let result = if force {
                    preview_gen.regenerate(&content_hash, file_path, &format)
                } else {
                    preview_gen.generate(&content_hash, file_path, &format)
                };
                // Also generate smart preview when --smart is set
                if smart {
                    let _ = if force { preview_gen.regenerate_smart(&content_hash, file_path, &format) }
                    else { preview_gen.generate_smart(&content_hash, file_path, &format) };
                }
                let file_elapsed = file_start.elapsed();
                let name = file_path.file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or_else(|| file_path.to_str().unwrap_or("?"));
                match result {
                    Ok(Some(_)) => {
                        generated += 1;
                        if cli.log { item_status(name, "generated", Some(file_elapsed)); }
                    }
                    Ok(None) => {
                        skipped += 1;
                        if cli.log { item_status(name, "skipped", Some(file_elapsed)); }
                    }
                    Err(e) => {
                        eprintln!("  Failed for {}: {e:#} ({})", file_path.display(), format_duration(file_elapsed));
                        failed += 1;
                    }
                }
            }
        }
    } else {
        // Catalog mode: iterate assets
        let volume_filter = match &volume {
            Some(label) => Some(registry.resolve_volume(label)?),
            None => None,
        };

        let assets = if let Some(asset_id) = &asset {
            let engine = QueryEngine::new(&catalog_root);
            let details = engine.show(asset_id)?;
            let uuid: uuid::Uuid = details.id.parse()?;
            vec![metadata_store.load(uuid)?]
        } else {
            let summaries = metadata_store.list()?;
            summaries
                .iter()
                .map(|s| metadata_store.load(s.id))
                .collect::<Result<Vec<_>, _>>()?
        };

        for asset_data in &assets {
            // Select the best variant for preview generation (respects user override)
            let idx = asset_data.preview_variant.as_ref()
                .and_then(|h| asset_data.variants.iter().position(|v| &v.content_hash == h))
                .or_else(|| maki::models::variant::best_preview_index(&asset_data.variants))
                .unwrap_or(0);
            if let Some(variant) = asset_data.variants.get(idx) {
                // In --upgrade mode, skip assets where the best variant is already the first
                if upgrade && idx == 0 {
                    skipped += 1;
                    continue;
                }

                // Apply format filter
                let ext = &variant.format;
                if !ext.is_empty() && !filter.is_importable(ext) {
                    skipped += 1;
                    continue;
                }

                // Try to find a reachable file for this variant
                let source_path = variant.locations.iter().find_map(|loc| {
                    // Apply volume filter
                    if let Some(ref vf) = volume_filter {
                        if loc.volume_id != vf.id {
                            return None;
                        }
                    }
                    volumes.iter().find_map(|v| {
                        if v.id == loc.volume_id && v.is_online {
                            let full = v.mount_point.join(&loc.relative_path);
                            if full.exists() { Some(full) } else { None }
                        } else {
                            None
                        }
                    })
                });

                // If we couldn't reach the file, record any offline volume
                // that held a location — so the end-of-run hint can tell
                // the user which disk to mount.
                if source_path.is_none() {
                    for loc in &variant.locations {
                        if let Some(v) = volumes.iter().find(|v| v.id == loc.volume_id) {
                            if !v.is_online {
                                offline_blockers.insert(v.label.clone());
                            }
                        }
                    }
                }

                if let Some(path) = source_path {
                    // Backfill video metadata if missing
                    if maki::asset_service::determine_asset_type(&variant.format) == maki::models::AssetType::Video
                        && !variant.source_metadata.contains_key("video_duration")
                    {
                        let service = AssetService::new(&catalog_root, verbosity, &config.preview);
                        service.backfill_video_metadata(&asset_data.id.to_string(), &variant.content_hash, &path);
                    }

                    let file_start = std::time::Instant::now();
                    let rotation = asset_data.preview_rotation;
                    // Generate regular preview (always)
                    let result = if force || upgrade {
                        preview_gen.regenerate_with_rotation(&variant.content_hash, &path, &variant.format, rotation)
                    } else {
                        preview_gen.generate(&variant.content_hash, &path, &variant.format)
                    };
                    // Also generate smart preview when --smart is set
                    if smart {
                        let _ = if force || upgrade {
                            preview_gen.regenerate_smart_with_rotation(&variant.content_hash, &path, &variant.format, rotation)
                        } else {
                            preview_gen.generate_smart(&variant.content_hash, &path, &variant.format)
                        };
                    }
                    let file_elapsed = file_start.elapsed();
                    let name = path.file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or_else(|| path.to_str().unwrap_or("?"));
                    match result {
                        Ok(Some(_)) => {
                            generated += 1;
                            if upgrade { upgraded += 1; }
                            if cli.log { item_status(name, if upgrade { "upgraded" } else { "generated" }, Some(file_elapsed)); }
                        }
                        Ok(None) => {
                            skipped += 1;
                            if cli.log { item_status(name, "skipped", Some(file_elapsed)); }
                        }
                        Err(e) => {
                            eprintln!("  Failed for {}: {e:#} ({})", asset_data.id, format_duration(file_elapsed));
                            failed += 1;
                        }
                    }
                } else {
                    skipped += 1;
                }
            } else {
                skipped += 1;
            }
        }
    }

    let preview_label = if smart { "smart preview(s)" } else { "preview(s)" };
    if cli.json {
        let mut result = serde_json::json!({
            "generated": generated,
            "skipped": skipped,
            "failed": failed,
        });
        if upgrade {
            result["upgraded"] = serde_json::json!(upgraded);
        }
        if smart {
            result["smart"] = serde_json::json!(true);
        }
        println!("{result}");
    } else {
        if upgrade && upgraded > 0 {
            println!(
                "Generated {} {} ({} upgraded), {} skipped, {} failed",
                generated, preview_label, upgraded, skipped, failed
            );
        } else {
            println!(
                "Generated {} {}, {} skipped, {} failed",
                generated, preview_label, skipped, failed
            );
        }
        // Tell the user which volumes blocked some skips so they don't
        // wonder why a file count looks low.
        if !offline_blockers.is_empty() {
            let mut labels: Vec<String> = offline_blockers.into_iter().collect();
            labels.sort();
            println!(
                "  Tip: some assets were skipped because their files \
                 live on offline volume(s): {}. Mount and re-run.",
                labels.join(", ")
            );
        }
    }
    Ok(())
}

/// Extracted body of `Commands::Duplicates`. See `run_import_command` for the
/// extraction pattern.
pub fn run_duplicates_command(
        format: Option<String>,
        same_volume: bool,
        cross_volume: bool,
        volume: Option<String>,
        filter_format: Option<String>,
        path: Option<String>,
        json: bool,
        log: bool,
        #[allow(unused_variables)] verbosity: maki::Verbosity,
) -> anyhow::Result<()> {
    #[allow(dead_code)]
    struct Ctx { json: bool, log: bool }
    let cli = Ctx { json, log };
    use maki::format::{self, OutputFormat};

    if same_volume && cross_volume {
        anyhow::bail!("--same-volume and --cross-volume are mutually exclusive");
    }

    let catalog_root = maki::config::find_catalog_root()?;
    let catalog = Catalog::open(&catalog_root)?;

    // Resolve volume label → ID for the SQL filter (unknown volume → empty results)
    let vol_id = if let Some(ref label) = volume {
        let registry = DeviceRegistry::new(&catalog_root);
        match registry.resolve_volume(label) {
            Ok(v) => Some(v.id.to_string()),
            Err(_) => Some("nonexistent".to_string()),
        }
    } else {
        None
    };

    let mode = if same_volume { "same" } else if cross_volume { "cross" } else { "all" };
    let has_filters = vol_id.is_some() || filter_format.is_some() || path.is_some();

    let entries = if has_filters {
        catalog.find_duplicates_filtered(
            mode,
            vol_id.as_deref(),
            filter_format.as_deref(),
            path.as_deref(),
        )?
    } else if same_volume {
        catalog.find_duplicates_same_volume()?
    } else if cross_volume {
        catalog.find_duplicates_cross_volume()?
    } else {
        catalog.find_duplicates()?
    };

    let explicit_format = format.is_some();

    let output_format = if let Some(fmt) = &format {
        format::parse_format(fmt).map_err(|e| anyhow::anyhow!(e))?
    } else if cli.json {
        OutputFormat::Json
    } else {
        OutputFormat::Short
    };

    if entries.is_empty() {
        match output_format {
            OutputFormat::Json => println!("[]"),
            _ => {
                if !explicit_format {
                    if same_volume {
                        println!("No same-volume duplicates found.");
                    } else if cross_volume {
                        println!("No cross-volume copies found.");
                    } else {
                        println!("No duplicates found.");
                    }
                }
            }
        }
    } else {
        match output_format {
            OutputFormat::Ids => {
                for entry in &entries {
                    println!("{}", entry.content_hash);
                }
            }
            OutputFormat::Short | OutputFormat::Full => {
                let is_full = matches!(output_format, OutputFormat::Full);
                for entry in &entries {
                    let display_name = entry
                        .asset_name
                        .as_deref()
                        .unwrap_or(&entry.original_filename);
                    let vol_info = if entry.volume_count > 1 {
                        format!(" [{} volumes]", entry.volume_count)
                    } else {
                        String::new()
                    };
                    println!(
                        "{} ({}, {}){}",
                        display_name,
                        entry.format,
                        format_size(entry.file_size),
                        vol_info,
                    );
                    println!("  Hash: {}", entry.content_hash);
                    for loc in &entry.locations {
                        let purpose = loc
                            .volume_purpose
                            .as_deref()
                            .map(|p| format!(" [{}]", p))
                            .unwrap_or_default();
                        if is_full {
                            let verified = loc
                                .verified_at
                                .as_deref()
                                .unwrap_or("never");
                            println!(
                                "    {}{} \u{2192} {} (verified: {})",
                                loc.volume_label, purpose, loc.relative_path, verified
                            );
                        } else {
                            println!(
                                "    {}{} \u{2192} {}",
                                loc.volume_label, purpose, loc.relative_path
                            );
                        }
                    }
                    if !entry.same_volume_groups.is_empty() {
                        println!(
                            "  \u{26a0} same-volume duplicates on: {}",
                            entry.same_volume_groups.join(", ")
                        );
                    }
                }
                if !explicit_format {
                    let label = if same_volume {
                        "same-volume duplicate(s)"
                    } else if cross_volume {
                        "cross-volume copie(s)"
                    } else {
                        "file(s) with duplicate locations"
                    };
                    println!(
                        "\n{} {}",
                        entries.len(),
                        label,
                    );
                }
            }
            OutputFormat::Json => {
                println!("{}", serde_json::to_string_pretty(&entries)?);
            }
            OutputFormat::Template(ref tpl) => {
                for entry in &entries {
                    let mut values = std::collections::HashMap::new();
                    values.insert("hash", entry.content_hash.clone());
                    values.insert("filename", entry.original_filename.clone());
                    values.insert("format", entry.format.clone());
                    values.insert("size", format_size(entry.file_size));
                    values.insert("name", entry.asset_name.as_deref()
                        .unwrap_or(&entry.original_filename).to_string());
                    let locs: Vec<String> = entry.locations.iter()
                        .map(|l| {
                            let purpose = l.volume_purpose.as_deref()
                                .map(|p| format!("[{}]", p))
                                .unwrap_or_default();
                            format!("{}{}:{}", l.volume_label, purpose, l.relative_path)
                        })
                        .collect();
                    values.insert("locations", locs.join(", "));
                    values.insert("volumes", entry.volume_count.to_string());
                    println!("{}", format::render_template(tpl, &values));
                }
            }
        }
    }
    Ok(())
}

/// Extracted body of `Commands::UpdateLocation`. See `run_import_command` for the
/// extraction pattern.
pub fn run_update_location_command(
        asset_id: String,
        from: String,
        to: String,
        volume: Option<String>,
        json: bool,
        log: bool,
        #[allow(unused_variables)] verbosity: maki::Verbosity,
) -> anyhow::Result<()> {
    #[allow(dead_code)]
    struct Ctx { json: bool, log: bool }
    let cli = Ctx { json, log };
    let (catalog_root, config) = maki::config::load_config()?;
    let service = AssetService::new(&catalog_root, verbosity, &config.preview);

    let to_path = std::fs::canonicalize(&to)
        .unwrap_or_else(|_| PathBuf::from(&to));

    let result = service.update_location(
        &asset_id,
        &from,
        &to_path,
        volume.as_deref(),
    )?;

    if cli.json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        let short_id = &result.asset_id[..8];
        println!(
            "Updated {} location for asset {short_id} on volume '{}'",
            result.file_type, result.volume_label,
        );
        println!("  {} -> {}", result.old_path, result.new_path);
    }
    Ok(())
}

/// Extracted body of `Commands::Dedup`. See `run_import_command` for the
/// extraction pattern.
pub fn run_dedup_command(
        volume: Option<String>,
        prefer: Option<String>,
        filter_format: Option<String>,
        path: Option<String>,
        min_copies: usize,
        apply: bool,
        json: bool,
        log: bool,
        #[allow(unused_variables)] verbosity: maki::Verbosity,
) -> anyhow::Result<()> {
    #[allow(dead_code)]
    struct Ctx { json: bool, log: bool }
    let cli = Ctx { json, log };
    let (catalog_root, config) = maki::config::load_config()?;
    let service = AssetService::new(&catalog_root, verbosity, &config.preview);

    // CLI --prefer overrides config [dedup] prefer
    let effective_prefer = prefer.or(config.dedup.prefer);

    let show_log = cli.log;
    let result = if show_log {
        use maki::asset_service::DedupStatus;
        service.dedup(
            volume.as_deref(),
            filter_format.as_deref(),
            path.as_deref(),
            effective_prefer.as_deref(),
            min_copies,
            apply,
            |filename, path, status, vol_label| {
                match status {
                    DedupStatus::Keep => {
                        eprintln!("  {} — keep ({}, {})", filename, path, vol_label);
                    }
                    DedupStatus::Remove => {
                        eprintln!("  {} — remove ({}, {})", filename, path, vol_label);
                    }
                    DedupStatus::Skipped => {
                        eprintln!("  {} — skipped, min-copies ({}, {})", filename, path, vol_label);
                    }
                }
            },
        )?
    } else {
        service.dedup(
            volume.as_deref(),
            filter_format.as_deref(),
            path.as_deref(),
            effective_prefer.as_deref(),
            min_copies,
            apply,
            |_, _, _, _| {},
        )?
    };

    if cli.json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        for err in &result.errors {
            eprintln!("  {err}");
        }

        if apply {
            let recipe_msg = if result.recipes_removed > 0 {
                format!(", {} recipes removed", result.recipes_removed)
            } else {
                String::new()
            };
            println!(
                "Dedup: {} duplicate groups, {} locations removed, {} files deleted{} ({})",
                result.duplicates_found,
                result.locations_removed,
                result.files_deleted,
                recipe_msg,
                format_size(result.bytes_freed),
            );
            // Same trap as sync: removing redundant locations leaves
            // variants that no longer have any locations. They linger
            // — sometimes as the asset's selected best-preview variant
            // — until `cleanup --apply` removes them.
            if result.locations_removed > 0 {
                if let Ok(catalog) = maki::catalog::Catalog::open(&catalog_root) {
                    if let Ok(locationless) = catalog.list_locationless_variants() {
                        if !locationless.is_empty() {
                            println!(
                                "  Tip: {} variant(s) have no remaining locations. \
                                 Run 'maki cleanup --apply' to remove them and their \
                                 orphaned previews/embeddings/face files.",
                                locationless.len()
                            );
                        }
                    }
                }
            }
        } else {
            let recipe_msg = if result.recipes_removed > 0 {
                format!(", {} recipe files", result.recipes_removed)
            } else {
                String::new()
            };
            println!(
                "Dedup: {} duplicate groups, {} redundant locations{} ({} reclaimable)",
                result.duplicates_found,
                result.locations_to_remove,
                recipe_msg,
                format_size(result.bytes_freed),
            );
            if result.locations_to_remove > 0 {
                println!("  Run with --apply to remove redundant files.");
            }
        }
    }

    Ok(())
}

/// Extracted body of `Commands::Cleanup`. See `run_import_command` for the
/// extraction pattern.
pub fn run_cleanup_command(
        volume: Option<String>,
        path: Option<String>,
        list: bool,
        apply: bool,
        json: bool,
        log: bool,
        #[allow(unused_variables)] verbosity: maki::Verbosity,
) -> anyhow::Result<()> {
    #[allow(dead_code)]
    struct Ctx { json: bool, log: bool }
    let cli = Ctx { json, log };
    let (catalog_root, config) = maki::config::load_config()?;
    let service = AssetService::new(&catalog_root, verbosity, &config.preview);

    // If --path is given without --volume, try to auto-detect the volume
    let volume = if volume.is_none() && path.is_some() {
        let registry = DeviceRegistry::new(&catalog_root);
        let p = std::path::Path::new(path.as_deref().unwrap());
        if p.is_absolute() {
            // Absolute path: find which volume contains it
            match registry.find_volume_for_path(p) {
                Ok(v) => Some(v.label.clone()),
                Err(_) => None, // fall through — cleanup will check all volumes
            }
        } else {
            None
        }
    } else {
        volume
    };

    // Convert absolute --path to relative (strip volume mount point)
    let path_prefix = if let (Some(ref p), Some(ref vol_label)) = (&path, &volume) {
        let abs = std::path::Path::new(p);
        if abs.is_absolute() {
            let registry = DeviceRegistry::new(&catalog_root);
            if let Ok(vol) = registry.resolve_volume(vol_label) {
                abs.strip_prefix(&vol.mount_point)
                    .ok()
                    .and_then(|rel| rel.to_str())
                    .map(|s| s.to_string())
                    .or_else(|| path.clone())
            } else {
                path.clone()
            }
        } else {
            path.clone()
        }
    } else {
        path
    };

    if verbosity.verbose {
        if let Some(ref prefix) = path_prefix {
            eprintln!("  Cleanup: path prefix \"{}\"", prefix);
        }
    }

    let show_log = cli.log;
    let show_list = list;
    let result = if show_log || show_list {
        use maki::asset_service::CleanupStatus;
        service.cleanup(
            volume.as_deref(),
            path_prefix.as_deref(),
            apply,
            |path, status, elapsed| {
                match status {
                    CleanupStatus::Ok if show_log => {
                        let name = path.file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or_else(|| path.to_str().unwrap_or("?"));
                        item_status(name, "ok", Some(elapsed));
                    }
                    CleanupStatus::Stale => {
                        let name = path.file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or_else(|| path.to_str().unwrap_or("?"));
                        item_status(name, "stale", Some(elapsed));
                    }
                    CleanupStatus::Offline => {
                        let name = path.file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or_else(|| path.to_str().unwrap_or("?"));
                        item_status(name, "offline", None);
                    }
                    CleanupStatus::LocationlessVariant => {
                        let name = path.to_str().unwrap_or("?");
                        item_status(name, "locationless variant removed", Some(elapsed));
                    }
                    CleanupStatus::OrphanedAsset => {
                        let name = path.to_str().unwrap_or("?");
                        item_status(name, "orphaned asset removed", Some(elapsed));
                    }
                    CleanupStatus::OrphanedFile => {
                        let name = path.file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or_else(|| path.to_str().unwrap_or("?"));
                        item_status(name, "orphaned file removed", Some(elapsed));
                    }
                    _ => {}
                }
            },
        )?
    } else {
        service.cleanup(
            volume.as_deref(),
            path_prefix.as_deref(),
            apply,
            |_, _, _| {},
        )?
    };

    if cli.json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        for err in &result.errors {
            eprintln!("  {err}");
        }

        if result.skipped_offline > 0 {
            eprintln!(
                "  Skipped {} offline volume(s).",
                result.skipped_offline
            );
        }

        if apply {
            let mut parts = vec![
                format!("{} checked", result.checked),
                format!("{} stale", result.stale),
                format!("{} removed", result.removed),
            ];
            if result.removed_variants > 0 {
                parts.push(format!("{} locationless variants removed", result.removed_variants));
            }
            if result.removed_assets > 0 {
                parts.push(format!("{} orphaned assets removed", result.removed_assets));
            }
            if result.removed_previews > 0 {
                parts.push(format!("{} orphaned previews removed", result.removed_previews));
            }
            if result.removed_smart_previews > 0 {
                parts.push(format!("{} orphaned smart previews removed", result.removed_smart_previews));
            }
            if result.removed_embeddings > 0 {
                parts.push(format!("{} orphaned embeddings removed", result.removed_embeddings));
            }
            if result.removed_face_files > 0 {
                parts.push(format!("{} orphaned face files removed", result.removed_face_files));
            }
            println!("Cleanup complete: {}", parts.join(", "));
        } else {
            let mut parts = vec![
                format!("{} checked", result.checked),
                format!("{} stale", result.stale),
            ];
            if result.locationless_variants > 0 {
                parts.push(format!("{} locationless variants", result.locationless_variants));
            }
            if result.orphaned_assets > 0 {
                parts.push(format!("{} orphaned assets", result.orphaned_assets));
            }
            if result.orphaned_previews > 0 {
                parts.push(format!("{} orphaned previews", result.orphaned_previews));
            }
            if result.orphaned_smart_previews > 0 {
                parts.push(format!("{} orphaned smart previews", result.orphaned_smart_previews));
            }
            if result.orphaned_embeddings > 0 {
                parts.push(format!("{} orphaned embeddings", result.orphaned_embeddings));
            }
            if result.orphaned_face_files > 0 {
                parts.push(format!("{} orphaned face files", result.orphaned_face_files));
            }
            println!("Cleanup complete: {}", parts.join(", "));
            let has_orphans = result.stale > 0
                || result.locationless_variants > 0
                || result.orphaned_assets > 0
                || result.orphaned_previews > 0
                || result.orphaned_smart_previews > 0
                || result.orphaned_embeddings > 0
                || result.orphaned_face_files > 0;
            if has_orphans {
                println!("  Run with --apply to remove stale records and orphaned files.");
            }
        }

        if result.skipped_global_passes {
            println!(
                "  Note: --volume/--path limits the scan to catalog records under that scope;"
            );
            println!(
                "        orphaned previews, embeddings, and face files are catalog-wide —"
            );
            println!(
                "        run `maki cleanup` without --volume/--path to check for those."
            );
        }
    }

    Ok(())
}

/// Extracted body of `Commands::Writeback`. See `run_import_command` for the
/// extraction pattern.
#[cfg(feature = "pro")]
pub fn run_writeback_command(
        query: Option<String>,
        volume: Option<String>,
        asset: Option<String>,
        all: bool,
        dry_run: bool,
        asset_ids: Vec<String>,
        json: bool,
        log: bool,
        #[allow(unused_variables)] verbosity: maki::Verbosity,
) -> anyhow::Result<()> {
    #[allow(dead_code)]
    struct Ctx { json: bool, log: bool }
    let cli = Ctx { json, log };
    let catalog_root = maki::config::find_catalog_root()?;
    let engine = maki::query::QueryEngine::new(&catalog_root);
    let _start = std::time::Instant::now();

    let scope = engine.resolve_scope(query.as_deref(), asset.as_deref(), &asset_ids)?;

    let result = engine.writeback(
        volume.as_deref(),
        None, // asset_filter replaced by scope
        scope.as_ref(),
        all,
        dry_run,
        cli.log,
        None,
    )?;

    if cli.json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        if dry_run {
            eprint!("Dry run: ");
        }
        let mut parts = Vec::new();
        parts.push(format!("{} written", result.written));
        if result.skipped > 0 {
            parts.push(format!("{} skipped", result.skipped));
        }
        if result.failed > 0 {
            parts.push(format!("{} failed", result.failed));
        }
        println!("Writeback: {}", parts.join(", "));
        for e in &result.errors {
            eprintln!("  Error: {e}");
        }
    }

    Ok(())
}

/// Extracted body of `Commands::Refresh`. See `run_import_command` for the
/// extraction pattern.
pub fn run_refresh_command(
        paths: Vec<String>,
        volume: Option<String>,
        asset: Option<String>,
        dry_run: bool,
        media: bool,
        reimport: bool,
        exif_only: bool,
        json: bool,
        log: bool,
        #[allow(unused_variables)] verbosity: maki::Verbosity,
) -> anyhow::Result<()> {
    #[allow(dead_code)]
    struct Ctx { json: bool, log: bool }
    let cli = Ctx { json, log };
    if reimport || exif_only {
        // --reimport: clear and re-extract all metadata from source files
        // --exif-only: re-extract only EXIF, leave tags/description/rating/label
        let catalog_root = maki::config::find_catalog_root()?;
        let engine = QueryEngine::new(&catalog_root);

        if asset.is_none() && paths.is_empty() {
            anyhow::bail!("--reimport/--exif-only requires --asset <ID> or asset IDs as arguments");
        }

        let asset_ids: Vec<String> = if let Some(ref id) = asset {
            vec![id.clone()]
        } else {
            paths.clone()
        };

        let mut reimported = 0usize;
        for id in &asset_ids {
            let result = if exif_only {
                engine.reimport_exif_only(id)
            } else {
                engine.reimport_metadata(id)
            };
            match result {
                Ok(tags) => {
                    reimported += 1;
                    if cli.log {
                        let short = if id.len() > 8 { &id[..8] } else { id };
                        eprintln!("  {} — reimported ({} tags)", short, tags.len());
                    }
                }
                Err(e) => {
                    eprintln!("  {} — error: {e}", if id.len() > 8 { &id[..8] } else { id.as_str() });
                }
            }
        }

        if cli.json {
            println!("{}", serde_json::json!({ "reimported": reimported }));
        } else {
            println!("Reimport metadata: {} asset(s) refreshed", reimported);
        }
        return Ok(());
    }

    let (catalog_root, config) = maki::config::load_config()?;
    let registry = DeviceRegistry::new(&catalog_root);

    let canonical_paths: Vec<PathBuf> = paths
        .iter()
        .map(|p| {
            std::fs::canonicalize(p)
                .unwrap_or_else(|_| PathBuf::from(p))
        })
        .collect();

    // Resolve volume
    let resolved_volume = if let Some(label) = &volume {
        Some(registry.resolve_volume(label)?)
    } else if !canonical_paths.is_empty() {
        Some(registry.find_volume_for_path(&canonical_paths[0])?)
    } else {
        None
    };

    // Resolve asset ID prefix
    let resolved_asset_id = if let Some(prefix) = &asset {
        let catalog = Catalog::open(&catalog_root)?;
        match catalog.resolve_asset_id(prefix)? {
            Some(id) => Some(id),
            None => anyhow::bail!("no asset found matching '{prefix}'"),
        }
    } else {
        None
    };

    let service = AssetService::new(&catalog_root, verbosity, &config.preview);
    let result = if cli.log {
        use maki::asset_service::RefreshStatus;
        service.refresh(
            &canonical_paths,
            resolved_volume.as_ref(),
            resolved_asset_id.as_deref(),
            dry_run,
            media,
            &config.import.exclude,
            |path, status, elapsed| {
                let label = match status {
                    RefreshStatus::Unchanged => "unchanged",
                    RefreshStatus::Refreshed => "refreshed",
                    RefreshStatus::Missing => "missing",
                    RefreshStatus::Offline => "offline",
                };
                let name = path.file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or_else(|| path.to_str().unwrap_or("?"));
                item_status(name, label, Some(elapsed));
            },
        )?
    } else {
        service.refresh(
            &canonical_paths,
            resolved_volume.as_ref(),
            resolved_asset_id.as_deref(),
            dry_run,
            media,
            &config.import.exclude,
            |_, _, _| {},
        )?
    };

    if cli.json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        for err in &result.errors {
            eprintln!("  {err}");
        }

        if dry_run {
            eprint!("Dry run — ");
        }

        let mut parts: Vec<String> = Vec::new();
        if result.refreshed > 0 {
            parts.push(format!("{} refreshed", result.refreshed));
        }
        if result.unchanged > 0 {
            parts.push(format!("{} unchanged", result.unchanged));
        }
        if result.missing > 0 {
            parts.push(format!("{} missing", result.missing));
        }
        if result.skipped > 0 {
            parts.push(format!("{} skipped (offline)", result.skipped));
        }
        if parts.is_empty() {
            println!("Refresh: nothing to check");
        } else {
            println!("Refresh complete: {}", parts.join(", "));
        }
    }

    Ok(())
}

/// Extracted body of `Commands::SyncMetadata`. See `run_import_command` for the
/// extraction pattern.
#[cfg(feature = "pro")]
pub fn run_sync_metadata_command(
        query: Option<String>,
        volume: Option<String>,
        asset: Option<String>,
        dry_run: bool,
        media: bool,
        asset_ids: Vec<String>,
        json: bool,
        log: bool,
        #[allow(unused_variables)] verbosity: maki::Verbosity,
) -> anyhow::Result<()> {
    #[allow(dead_code)]
    struct Ctx { json: bool, log: bool }
    let cli = Ctx { json, log };
    let _start = std::time::Instant::now();
    let (catalog_root, config) = maki::config::load_config()?;
    let registry = DeviceRegistry::new(&catalog_root);
    let engine = maki::query::QueryEngine::new(&catalog_root);

    // Resolve volume
    let resolved_volume = if let Some(label) = &volume {
        Some(registry.resolve_volume(label)?)
    } else {
        None
    };

    // Resolve scope (query/asset/asset_ids) to individual asset IDs
    let scope = engine.resolve_scope(query.as_deref(), asset.as_deref(), &asset_ids)?;
    let asset_id_list: Vec<Option<String>> = match scope {
        Some(set) => set.into_iter().map(Some).collect(),
        None => vec![None], // process all
    };

    let service = AssetService::new(&catalog_root, verbosity, &config.preview);
    let mut result = maki::asset_service::SyncMetadataResult { dry_run, ..Default::default() };
    for aid in &asset_id_list {
        let r = if cli.log {
            use maki::asset_service::SyncMetadataStatus;
            service.sync_metadata(
                resolved_volume.as_ref(),
                aid.as_deref(),
                dry_run,
                media,
                &config.import.exclude,
                |path, status, elapsed| {
                    let label = match status {
                        SyncMetadataStatus::Inbound => "inbound",
                        SyncMetadataStatus::Outbound => "outbound",
                        SyncMetadataStatus::Unchanged => "unchanged",
                        SyncMetadataStatus::Missing => "missing",
                        SyncMetadataStatus::Offline => "offline",
                        SyncMetadataStatus::Conflict => "CONFLICT",
                        SyncMetadataStatus::Error => "error",
                    };
                    let name = path.file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or_else(|| path.to_str().unwrap_or("?"));
                    item_status(name, label, Some(elapsed));
                },
            )?
        } else {
            service.sync_metadata(
                resolved_volume.as_ref(),
                aid.as_deref(),
                dry_run,
                media,
                &config.import.exclude,
                |_, _, _| {},
            )?
        };
        result.inbound += r.inbound;
        result.outbound += r.outbound;
        result.unchanged += r.unchanged;
        result.conflicts += r.conflicts;
        result.skipped += r.skipped;
        result.media_refreshed += r.media_refreshed;
        result.errors.extend(r.errors);
    }

    if cli.json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        for err in &result.errors {
            eprintln!("  {err}");
        }

        if dry_run {
            eprint!("Dry run — ");
        }

        let mut parts: Vec<String> = Vec::new();
        if result.inbound > 0 {
            parts.push(format!("{} read from disk", result.inbound));
        }
        if result.outbound > 0 {
            parts.push(format!("{} written to disk", result.outbound));
        }
        if result.conflicts > 0 {
            parts.push(format!("{} conflicts (skipped)", result.conflicts));
        }
        if result.media_refreshed > 0 {
            parts.push(format!("{} media refreshed", result.media_refreshed));
        }
        if result.unchanged > 0 {
            parts.push(format!("{} unchanged", result.unchanged));
        }
        if result.skipped > 0 {
            parts.push(format!("{} skipped", result.skipped));
        }
        if parts.is_empty() {
            println!("Sync metadata: nothing to do");
        } else {
            println!("Sync metadata: {}", parts.join(", "));
        }

        if result.conflicts > 0 {
            eprintln!("  Tip: resolve conflicts by running 'maki refresh' (accept external) or 'maki writeback' (keep DAM edits).");
        }
    }

    Ok(())
}

/// Extracted body of `Commands::Sync`. See `run_import_command` for the
/// extraction pattern.
pub fn run_sync_command(
        paths: Vec<String>,
        volume: Option<String>,
        apply: bool,
        remove_stale: bool,
        json: bool,
        log: bool,
        #[allow(unused_variables)] verbosity: maki::Verbosity,
) -> anyhow::Result<()> {
    #[allow(dead_code)]
    struct Ctx { json: bool, log: bool }
    let cli = Ctx { json, log };
    if paths.is_empty() {
        anyhow::bail!("no paths specified for sync.");
    }
    if remove_stale && !apply {
        anyhow::bail!("--remove-stale requires --apply.");
    }

    let (catalog_root, config) = maki::config::load_config()?;
    let registry = DeviceRegistry::new(&catalog_root);

    let canonical_paths: Vec<PathBuf> = paths
        .iter()
        .map(|p| {
            std::fs::canonicalize(p)
                .unwrap_or_else(|_| PathBuf::from(p))
        })
        .collect();

    let volume = if let Some(label) = &volume {
        registry.resolve_volume(label)?
    } else {
        registry.find_volume_for_path(&canonical_paths[0])?
    };

    let service = AssetService::new(&catalog_root, verbosity, &config.preview);
    let result = if cli.log {
        use maki::asset_service::SyncStatus;
        service.sync(
            &canonical_paths,
            &volume,
            apply,
            remove_stale,
            &config.import.exclude,
            |path, status, elapsed| {
                let label = match status {
                    SyncStatus::Unchanged => "unchanged",
                    SyncStatus::Moved => "moved",
                    SyncStatus::New => "new",
                    SyncStatus::Modified => "modified",
                    SyncStatus::Missing => "missing",
                };
                let name = path.file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or_else(|| path.to_str().unwrap_or("?"));
                item_status(name, label, Some(elapsed));
            },
        )?
    } else {
        service.sync(
            &canonical_paths,
            &volume,
            apply,
            remove_stale,
            &config.import.exclude,
            |_, _, _| {},
        )?
    };

    if cli.json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        for err in &result.errors {
            eprintln!("  {err}");
        }

        let mut parts: Vec<String> = Vec::new();
        if result.unchanged > 0 {
            parts.push(format!("{} unchanged", result.unchanged));
        }
        if result.moved > 0 {
            parts.push(format!("{} moved", result.moved));
        }
        if result.new_files > 0 {
            parts.push(format!("{} new", result.new_files));
        }
        if result.modified > 0 {
            parts.push(format!("{} modified", result.modified));
        }
        if result.missing > 0 {
            parts.push(format!("{} missing", result.missing));
        }
        if result.stale_removed > 0 {
            parts.push(format!("{} stale removed", result.stale_removed));
        }
        if result.orphaned_cleaned > 0 {
            parts.push(format!("{} orphaned assets cleaned", result.orphaned_cleaned));
        }
        if parts.is_empty() {
            println!("Sync: nothing to sync");
        } else {
            if !apply && (result.moved > 0 || result.modified > 0 || result.missing > 0) {
                eprint!("Dry run — ");
            }
            println!("Sync complete: {}", parts.join(", "));
        }
        if !apply && (result.moved > 0 || result.modified > 0) {
            println!("  Run with --apply to apply changes.");
        }
        if result.missing > 0 && !remove_stale {
            println!("  Run with --apply --remove-stale to remove missing file records.");
        }
        if result.new_files > 0 {
            println!("  Tip: run 'maki import' to import new files.");
        }
        // After sync, variants whose only locations were removed linger
        // in the catalog (often as the asset's selected best-preview
        // variant). They confuse subsequent `preview`/`generate-previews`
        // calls — `maki cleanup --apply` removes them and their derived
        // preview/embedding/face files.
        if result.locationless_after > 0 {
            println!(
                "  Tip: {} variant(s) have no remaining locations. \
                 Run 'maki cleanup --apply' to remove them and their \
                 orphaned previews/embeddings/face files.",
                result.locationless_after
            );
        }
    }

    Ok(())
}

/// Extracted body of `Commands::Verify`. See `run_import_command` for the
/// extraction pattern.
pub fn run_verify_command(
        paths: Vec<String>,
        volume: Option<String>,
        asset: Option<String>,
        include: Vec<String>,
        skip: Vec<String>,
        max_age: Option<u64>,
        force: bool,
        json: bool,
        log: bool,
        #[allow(unused_variables)] verbosity: maki::Verbosity,
) -> anyhow::Result<()> {
    #[allow(dead_code)]
    struct Ctx { json: bool, log: bool }
    let cli = Ctx { json, log };
    use maki::asset_service::FileTypeFilter;

    let (catalog_root, config) = maki::config::load_config()?;
    let service = AssetService::new(&catalog_root, verbosity, &config.preview);

    let max_age_days: Option<u64> = if force {
        None
    } else {
        max_age.or(config.verify.max_age_days)
    };

    // Build file type filter (same logic as import)
    let mut filter = FileTypeFilter::default();
    for group in &include {
        if skip.contains(group) {
            anyhow::bail!(
                "Group '{}' cannot be both included and skipped.",
                group
            );
        }
    }
    for group in &include {
        filter.include(group)?;
    }
    for group in &skip {
        filter.skip(group)?;
    }

    let canonical_paths: Vec<PathBuf> = paths
        .iter()
        .map(|p| {
            std::fs::canonicalize(p)
                .unwrap_or_else(|_| PathBuf::from(p))
        })
        .collect();

    let result = if cli.log {
        use maki::asset_service::VerifyStatus;
        service.verify(
            &canonical_paths,
            volume.as_deref(),
            asset.as_deref(),
            &filter,
            max_age_days,
            |path, status, elapsed| {
                let label = match status {
                    VerifyStatus::Ok => "OK",
                    VerifyStatus::Mismatch => "FAILED",
                    VerifyStatus::Modified => "MODIFIED",
                    VerifyStatus::Missing => "MISSING",
                    VerifyStatus::Skipped => "SKIPPED",
                    VerifyStatus::SkippedRecent => "RECENT",
                    VerifyStatus::Untracked => "UNTRACKED",
                };
                let name = path.file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or_else(|| path.to_str().unwrap_or("?"));
                item_status(name, label, Some(elapsed));
            },
        )?
    } else {
        service.verify(
            &canonical_paths,
            volume.as_deref(),
            asset.as_deref(),
            &filter,
            max_age_days,
            |_, _, _| {},
        )?
    };

    if cli.json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        // Print error details
        for err in &result.errors {
            eprintln!("  {err}");
        }

        // Print summary
        let mut parts: Vec<String> = Vec::new();
        if result.verified > 0 {
            parts.push(format!("{} verified", result.verified));
        }
        if result.modified > 0 {
            parts.push(format!("{} modified", result.modified));
        }
        if result.failed > 0 {
            parts.push(format!("{} FAILED", result.failed));
        }
        if result.skipped_recent > 0 {
            let age_label = max_age_days
                .map(|d| format!("{d} days"))
                .unwrap_or_else(|| "max age".to_string());
            parts.push(format!(
                "{} skipped (verified within {})",
                result.skipped_recent, age_label
            ));
        }
        if result.skipped > 0 {
            parts.push(format!("{} skipped", result.skipped));
        }
        if parts.is_empty() {
            println!("Verify: nothing to verify");
        } else {
            println!("Verify complete: {}", parts.join(", "));
        }
    }

    if result.failed > 0 {
        anyhow::bail!("verification failed for {} file(s)", result.failed);
    }

    Ok(())
}

/// Extracted body of `Commands::Relocate`. See `run_import_command` for the
/// extraction pattern.
pub fn run_relocate_command(
        asset_ids: Vec<String>,
        target: Option<String>,
        query: Option<String>,
        remove_source: bool,
        create_sidecars: bool,
        dry_run: bool,
        json: bool,
        log: bool,
        #[allow(unused_variables)] verbosity: maki::Verbosity,
) -> anyhow::Result<()> {
    #[allow(dead_code)]
    struct Ctx { json: bool, log: bool }
    let cli = Ctx { json, log };
    let (catalog_root, config) = maki::config::load_config()?;
    let service = AssetService::new(&catalog_root, verbosity, &config.preview);

    // Resolve asset IDs: --query, positional args, or stdin
    let ids: Vec<String> = if let Some(ref q) = query {
        let engine = QueryEngine::new(&catalog_root);
        engine.search(q)?.into_iter().map(|r| r.asset_id).collect()
    } else if asset_ids.is_empty() {
        use std::io::BufRead;
        std::io::stdin().lock().lines()
            .filter_map(|l| l.ok())
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect()
    } else {
        asset_ids
    };

    if ids.is_empty() {
        anyhow::bail!("no asset IDs specified. Use --query, positional args, or pipe from stdin.");
    }

    // Determine target volume: --target flag, or second positional arg for single-asset compat
    let target_volume = match target {
        Some(t) => t,
        None => {
            // Backward compat: `maki relocate <asset-id> <volume>`
            if ids.len() == 2 && query.is_none() {
                let vol = ids[1].clone();
                // Treat as single-asset mode: first arg is asset, second is volume
                let single_id = ids[0].clone();
                let result = service.relocate(&single_id, &vol, remove_source, create_sidecars, dry_run)?;

                if cli.json {
                    println!("{}", serde_json::to_string_pretty(&result)?);
                } else {
                    if dry_run {
                        println!("Dry run — no changes made:");
                    }
                    for action in &result.actions {
                        println!("  {action}");
                    }
                    let verb = if remove_source { "moved" } else { "copied" };
                    let mut parts: Vec<String> = Vec::new();
                    if result.copied > 0 {
                        parts.push(format!("{} {verb}", result.copied));
                    }
                    if result.skipped > 0 {
                        parts.push(format!("{} skipped", result.skipped));
                    }
                    if parts.is_empty() {
                        if result.actions.len() == 1 {
                            // The "already on target" message was printed above
                        } else {
                            println!("Relocate: nothing to do");
                        }
                    } else {
                        println!("Relocate complete: {}", parts.join(", "));
                    }
                }
                return Ok(());
            }
            anyhow::bail!("--target <volume> is required for batch relocate");
        }
    };

    // Batch relocate
    let total = ids.len();
    let mut total_copied: usize = 0;
    let mut total_skipped: usize = 0;
    let mut total_removed: usize = 0;
    let mut errors: Vec<String> = Vec::new();

    if dry_run && !cli.json {
        println!("Dry run — no changes will be made:");
    }

    for (i, id) in ids.iter().enumerate() {
        match service.relocate(id, &target_volume, remove_source, create_sidecars, dry_run) {
            Ok(result) => {
                total_copied += result.copied;
                total_skipped += result.skipped;
                total_removed += result.removed;

                if cli.log {
                    let verb = if remove_source { "moved" } else { "copied" };
                    eprintln!("[{}/{}] {} — {} {verb}, {} skipped",
                        i + 1, total, &id[..8.min(id.len())],
                        result.copied, result.skipped);
                }
            }
            Err(e) => {
                let msg = format!("{}: {e:#}", &id[..8.min(id.len())]);
                if cli.log {
                    eprintln!("[{}/{}] ERROR {msg}", i + 1, total);
                }
                errors.push(msg);
            }
        }
    }

    if cli.json {
        println!("{}", serde_json::json!({
            "assets": total,
            "copied": total_copied,
            "skipped": total_skipped,
            "removed": total_removed,
            "errors": errors,
            "dry_run": dry_run,
        }));
    } else {
        let verb = if remove_source { "moved" } else { "copied" };
        let mut parts: Vec<String> = Vec::new();
        parts.push(format!("{total} assets"));
        if total_copied > 0 {
            parts.push(format!("{total_copied} files {verb}"));
        }
        if total_skipped > 0 {
            parts.push(format!("{total_skipped} skipped"));
        }
        if !errors.is_empty() {
            parts.push(format!("{} errors", errors.len()));
            for e in &errors {
                eprintln!("  error: {e}");
            }
        }
        println!("Relocate complete: {}", parts.join(", "));
    }

    Ok(())
}

/// Extracted body of `Commands::AutoGroup`. See `run_import_command` for the
/// extraction pattern.
pub fn run_auto_group_command(
        query: Option<String>,
        apply: bool,
        global: bool,
        json: bool,
        log: bool,
        #[allow(unused_variables)] verbosity: maki::Verbosity,
) -> anyhow::Result<()> {
    #[allow(dead_code)]
    struct Ctx { json: bool, log: bool }
    let cli = Ctx { json, log };
    let catalog_root = maki::config::find_catalog_root()?;
    let engine = QueryEngine::new(&catalog_root);

    // Search to get asset IDs, deduplicate (search returns one row per variant)
    let results = engine.search(query.as_deref().unwrap_or(""))?;
    let asset_ids: Vec<String> = {
        let mut seen = std::collections::HashSet::new();
        results
            .iter()
            .filter(|r| seen.insert(r.asset_id.clone()))
            .map(|r| r.asset_id.clone())
            .collect()
    };

    let show_log = cli.log;
    let result = if global {
        engine.auto_group_global(&asset_ids, !apply)?
    } else {
        engine.auto_group_with_log(&asset_ids, !apply, |stem, count| {
            if show_log {
                eprintln!("  {} — {} asset(s)", stem, count);
            }
        })?
    };

    if cli.json {
        println!("{}", serde_json::to_string(&result)?);
    } else {
        if result.groups.is_empty() {
            eprintln!("No groupable assets found");
        } else {
            println!(
                "{} stem group(s), {} donor(s) {}, {} variant(s) moved",
                result.groups.len(),
                result.total_donors_merged,
                if apply { "merged" } else { "would merge" },
                result.total_variants_moved,
            );
        }
        if !apply {
            eprintln!("Dry run — use --apply to merge");
        }
        // Merging variants into a target reorders variants and may
        // change which one is the best-preview pick. Cached previews
        // for the target still reflect the pre-merge best — refresh
        // with `generate-previews --upgrade`.
        if apply && result.total_donors_merged > 0 {
            println!(
                "  Tip: {} group(s) gained variants. Run \
                 'maki generate-previews --upgrade' to refresh \
                 previews for assets whose best variant changed.",
                result.groups.len()
            );
        }
    }
    Ok(())
}

/// Extracted body of `Commands::Embed`. See `run_import_command` for the
/// extraction pattern.
#[cfg(feature = "ai")]
pub fn run_embed_command(
        query: Option<String>,
        asset: Option<String>,
        volume: Option<String>,
        model: Option<String>,
        force: bool,
        export: bool,
        asset_ids: Vec<String>,
        json: bool,
        log: bool,
        #[allow(unused_variables)] verbosity: maki::Verbosity,
) -> anyhow::Result<()> {
    #[allow(dead_code)]
    struct Ctx { json: bool, log: bool }
    let cli = Ctx { json, log };
    let (query, asset) = merge_trailing_ids(query, asset, &asset_ids);
    use maki::model_manager::ModelManager;

    if export {
        let catalog_root = maki::config::find_catalog_root()?;
        let catalog = maki::catalog::Catalog::open(&catalog_root)?;
        let _ = maki::embedding_store::EmbeddingStore::initialize(catalog.conn());
        let emb_store = maki::embedding_store::EmbeddingStore::new(catalog.conn());

        let mut total = 0u32;
        let models = emb_store.list_models()?;
        for m in &models {
            let embeddings = emb_store.all_embeddings_for_model(m)?;
            for (asset_id, emb) in &embeddings {
                if let Err(e) = maki::embedding_store::write_embedding_binary(&catalog_root, m, asset_id, emb) {
                    eprintln!("  Warning: {}: {e:#}", &asset_id[..8.min(asset_id.len())]);
                } else {
                    total += 1;
                }
            }
            if !embeddings.is_empty() {
                eprintln!("  {}: {} embeddings", m, embeddings.len());
            }
        }
        if cli.json {
            println!("{}", serde_json::json!({"exported": total, "models": models}));
        } else {
            println!("Exported {total} embedding binaries");
        }
        return Ok(());
    }

    if query.is_none() && asset.is_none() && volume.is_none() {
        anyhow::bail!(
            "No scope specified. Provide a query, --asset, or --volume to select assets.\n  \
             Examples:\n    \
             maki embed ''                    # all assets\n    \
             maki embed --asset <id>          # single asset\n    \
             maki embed --volume <label>      # one volume"
        );
    }

    let (catalog_root, config) = maki::config::load_config()?;

    let model_id = model.as_deref().unwrap_or(&config.ai.model);
    let _spec = maki::ai::get_model_spec(model_id)
        .ok_or_else(|| anyhow::anyhow!(
            "Unknown model: {model_id}. Run 'maki auto-tag --list-models' to see available models."
        ))?;

    let model_dir = maki::config::resolve_model_dir(&config.ai.model_dir, model_id);
    let mgr = ModelManager::new(&model_dir, model_id)?;

    if !mgr.model_exists() {
        anyhow::bail!(
            "Model not downloaded. Run 'maki auto-tag --download --model {model_id}' first."
        );
    }

    let catalog = maki::catalog::Catalog::open(&catalog_root)?;
    let engine = QueryEngine::new(&catalog_root);

    // Resolve target assets
    let asset_ids: Vec<String> = if let Some(ref id) = asset {
        let full_id = catalog
            .resolve_asset_id(id)?
            .ok_or_else(|| anyhow::anyhow!("no asset found matching '{id}'"))?;
        vec![full_id]
    } else {
        let q = if let Some(ref query) = query {
            let volume_part = volume.as_deref().map(|v| format!(" volume:{v}")).unwrap_or_default();
            format!("{query}{volume_part}")
        } else if let Some(ref v) = volume {
            format!("volume:{v}")
        } else {
            String::new()
        };
        let results = engine.search(&q)?;
        results.into_iter().map(|r| r.asset_id).collect()
    };

    let service = AssetService::new(&catalog_root, verbosity, &config.preview);
    let log = cli.log;
    let result = service.embed_assets(
        &asset_ids,
        &model_dir,
        model_id,
        &config.ai.execution_provider,
        force,
        |aid, status, elapsed| {
            if !log { return; }
            let short = &aid[..8.min(aid.len())];
            match status {
                maki::asset_service::EmbedStatus::Embedded => {
                    item_status(short, "embedded", Some(elapsed));
                }
                maki::asset_service::EmbedStatus::Skipped(reason) => {
                    item_status(short, &format!("skipped: {reason}"), Some(elapsed));
                }
                maki::asset_service::EmbedStatus::Error(msg) => {
                    eprintln!("  {short} — error: {msg}");
                }
            }
        },
    )?;

    if cli.json {
        println!("{}", serde_json::json!({
            "embedded": result.embedded,
            "skipped": result.skipped,
            "errors": result.errors,
            "model": model_id,
            "force": force,
        }));
    } else {
        for err in &result.errors {
            eprintln!("  {err}");
        }
        let mut parts = vec![
            format!("{} embedded", result.embedded),
            format!("{} skipped", result.skipped),
        ];
        if !result.errors.is_empty() {
            parts.push(format!("{} errors", result.errors.len()));
        }
        println!("Embed: {}", parts.join(", "));
    }
    Ok(())
}

/// Extracted body of `Commands::Describe`. See `run_import_command` for the
/// extraction pattern.
#[cfg(feature = "pro")]
pub fn run_describe_command(
        query: Option<String>,
        asset: Option<String>,
        volume: Option<String>,
        model: Option<String>,
        endpoint: Option<String>,
        prompt: Option<String>,
        max_tokens: Option<u32>,
        timeout: Option<u32>,
        mode: String,
        temperature: Option<f32>,
        num_ctx: Option<u32>,
        top_p: Option<f32>,
        top_k: Option<u32>,
        repeat_penalty: Option<f32>,
        apply: bool,
        force: bool,
        dry_run: bool,
        check: bool,
        asset_ids: Vec<String>,
        json: bool,
        log: bool,
        #[allow(unused_variables)] verbosity: maki::Verbosity,
) -> anyhow::Result<()> {
    #[allow(dead_code)]
    struct Ctx { json: bool, log: bool }
    let cli = Ctx { json, log };
    let (catalog_root, config) = maki::config::load_config()?;

    let endpoint = endpoint.as_deref().unwrap_or(&config.vlm.endpoint);
    let model = model.as_deref().unwrap_or(&config.vlm.model);
    let vlm_mode = maki::vlm::DescribeMode::from_str(&mode)?;

    // Build params: per-model config merged with CLI overrides
    let mut vlm_params = config.vlm.params_for_model(model);
    if let Some(v) = max_tokens { vlm_params.max_tokens = v; }
    if let Some(v) = timeout { vlm_params.timeout = v; }
    if let Some(v) = temperature { vlm_params.temperature = v; }
    if let Some(v) = num_ctx { vlm_params.num_ctx = v; }
    if let Some(v) = top_p { vlm_params.top_p = v; }
    if let Some(v) = top_k { vlm_params.top_k = v; }
    if let Some(v) = repeat_penalty { vlm_params.repeat_penalty = v; }
    if let Some(ref p) = prompt { vlm_params.prompt = Some(p.clone()); }

    if check {
        match maki::vlm::check_endpoint_status(endpoint, vlm_params.timeout, verbosity) {
            Ok(status) => {
                let model_status = if status.available_models.is_empty() {
                    format!("Configured model: {model}")
                } else {
                    match maki::vlm::find_matching_model(model, &status.available_models) {
                        Some(matched) if matched == model => {
                            format!("Model {model} is available")
                        }
                        Some(matched) => {
                            format!("Model {matched} matched (from \"{model}\")")
                        }
                        None => {
                            format!(
                                "WARNING: model \"{model}\" not found. Pull it with `ollama pull {model}` or set [vlm] model in maki.toml"
                            )
                        }
                    }
                };
                if cli.json {
                    let model_ok = status.available_models.is_empty()
                        || maki::vlm::find_matching_model(model, &status.available_models).is_some();
                    println!("{}", serde_json::json!({
                        "status": "ok",
                        "endpoint": endpoint,
                        "model": model,
                        "model_available": model_ok,
                        "available_models": status.available_models,
                        "message": status.message,
                    }));
                } else {
                    println!("{}", status.message);
                    println!("{model_status}");
                }
            }
            Err(e) => {
                if cli.json {
                    println!("{}", serde_json::json!({
                        "status": "error",
                        "endpoint": endpoint,
                        "model": model,
                        "message": format!("{e}"),
                    }));
                } else {
                    eprintln!("{e}");
                }
                anyhow::bail!("vLM endpoint check failed");
            }
        }
        return Ok(());
    }

    // Merge asset_ids from shell variable expansion into asset/query
    let (query, asset) = merge_trailing_ids(query, asset, &asset_ids);

    if query.is_none() && asset.is_none() && volume.is_none() {
        anyhow::bail!(
            "No scope specified. Use a query, --asset, or --volume to select assets.\n  \
             Examples:\n    \
             maki describe ''                    # all assets\n    \
             maki describe --asset <id>          # single asset\n    \
             maki describe 'rating:4+' --apply   # apply to rated assets"
        );
    }

    if verbosity.verbose {
        eprintln!("  VLM: endpoint={endpoint}, model={model}, mode={mode}");
        eprintln!("  VLM: max_tokens={}, timeout={}s, temperature={}, concurrency={}", vlm_params.max_tokens, vlm_params.timeout, vlm_params.temperature, config.vlm.concurrency);
    }

    let service = AssetService::new(&catalog_root, verbosity, &config.preview);

    let show_log = cli.log;
    let result = service.describe(
        query.as_deref(),
        asset.as_deref(),
        volume.as_deref(),
        endpoint,
        model,
        &vlm_params,
        vlm_mode,
        apply,
        force,
        dry_run,
        config.vlm.concurrency,
        |id, status, elapsed| {
            if show_log {
                let short_id = &id[..8.min(id.len())];
                match status {
                    maki::vlm::DescribeStatus::Described => {
                        eprintln!(
                            "  {short_id} — described ({})",
                            format_duration(elapsed)
                        );
                    }
                    maki::vlm::DescribeStatus::Skipped(msg) => {
                        eprintln!("  {short_id} — skipped: {msg}");
                    }
                    maki::vlm::DescribeStatus::Error(msg) => {
                        eprintln!("  {short_id} — error: {msg}");
                    }
                }
            }
        },
    )?;

    if cli.json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        // Print each result
        for r in &result.results {
            let short_id = &r.asset_id[..8.min(r.asset_id.len())];
            match &r.status {
                maki::vlm::DescribeStatus::Described => {
                    if let Some(ref desc) = r.description {
                        println!("{short_id}: {desc}");
                    }
                    if !r.tags.is_empty() {
                        println!("{short_id}: tags: {}", r.tags.join(", "));
                    }
                }
                maki::vlm::DescribeStatus::Skipped(msg) => {
                    if !cli.log {
                        eprintln!("{short_id}: skipped — {msg}");
                    }
                }
                maki::vlm::DescribeStatus::Error(msg) => {
                    if !cli.log {
                        eprintln!("{short_id}: error — {msg}");
                    }
                }
            }
        }

        let label = if dry_run {
            "Describe (dry run)"
        } else if apply {
            "Describe"
        } else {
            "Describe (report only)"
        };
        let mut parts = vec![format!("{} processed", result.described)];
        if result.skipped > 0 {
            parts.push(format!("{} skipped", result.skipped));
        }
        if result.failed > 0 {
            parts.push(format!("{} failed", result.failed));
        }
        if result.tags_applied > 0 {
            parts.push(format!("{} tags applied", result.tags_applied));
        }
        eprintln!("{label}: {}", parts.join(", "));
        if !apply && !dry_run && result.described > 0 {
            eprintln!("  Run with --apply to save changes to assets.");
        }
    }
    Ok(())
}

/// Extracted body of `Commands::Delete`. See `run_import_command` for the
/// extraction pattern.
pub fn run_delete_command(
        asset_ids: Vec<String>,
        apply: bool,
        remove_files: bool,
        json: bool,
        log: bool,
        #[allow(unused_variables)] verbosity: maki::Verbosity,
) -> anyhow::Result<()> {
    #[allow(dead_code)]
    struct Ctx { json: bool, log: bool }
    let cli = Ctx { json, log };
    if remove_files && !apply {
        anyhow::bail!("--remove-files requires --apply");
    }

    // Read from stdin if no IDs provided
    let ids: Vec<String> = if asset_ids.is_empty() {
        use std::io::BufRead;
        std::io::stdin().lock().lines()
            .filter_map(|l| l.ok())
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect()
    } else {
        asset_ids
    };
    if ids.is_empty() {
        anyhow::bail!("no asset IDs specified.");
    }

    let (catalog_root, config) = maki::config::load_config()?;
    let service = AssetService::new(&catalog_root, verbosity, &config.preview);

    // Collect face IDs for deleted assets before deletion (for AI cleanup)
    #[cfg(feature = "ai")]
    let ai_cleanup_info: Vec<(String, Vec<String>)> = if apply {
        let catalog = maki::catalog::Catalog::open(&catalog_root)?;
        let _ = maki::face_store::FaceStore::initialize(catalog.conn());
        let face_store = maki::face_store::FaceStore::new(catalog.conn());
        ids.iter().filter_map(|id| {
            let full_id = catalog.resolve_asset_id(id).ok().flatten()?;
            let faces = face_store.faces_for_asset(&full_id).unwrap_or_default();
            let face_ids: Vec<String> = faces.into_iter().map(|f| f.id).collect();
            Some((full_id, face_ids))
        }).collect()
    } else {
        Vec::new()
    };

    let show_log = cli.log;
    let result = service.delete_assets(
        &ids,
        apply,
        remove_files,
        |id, status, elapsed| {
            if show_log {
                let short_id = &id[..8.min(id.len())];
                match status {
                    maki::asset_service::DeleteStatus::Deleted => {
                        item_status(short_id, "deleted", Some(elapsed));
                    }
                    maki::asset_service::DeleteStatus::NotFound => {
                        item_status(short_id, "not found", None);
                    }
                    maki::asset_service::DeleteStatus::Error(msg) => {
                        eprintln!("  {short_id} — error: {msg}");
                    }
                }
            }
        },
    )?;

    // Clean up AI files for deleted assets
    #[cfg(feature = "ai")]
    if apply && result.deleted > 0 {
        for (asset_id, face_ids) in &ai_cleanup_info {
            // Delete ArcFace binaries for each face
            for face_id in face_ids {
                maki::face_store::delete_arcface_binary(&catalog_root, face_id);
            }
            // Delete SigLIP embedding binary
            maki::embedding_store::delete_embedding_binary(&catalog_root, "siglip-vit-b16-256", asset_id);
            maki::embedding_store::delete_embedding_binary(&catalog_root, "siglip-vit-l16-256", asset_id);
        }
        // Update faces/people YAML
        let catalog = maki::catalog::Catalog::open(&catalog_root)?;
        let _ = maki::face_store::FaceStore::initialize(catalog.conn());
        let face_store = maki::face_store::FaceStore::new(catalog.conn());
        let _ = face_store.save_all_yaml(&catalog_root);
    }

    if cli.json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        for err in &result.errors {
            eprintln!("  {err}");
        }

        if apply {
            let mut parts = vec![
                format!("{} deleted", result.deleted),
            ];
            if !result.not_found.is_empty() {
                parts.push(format!("{} not found", result.not_found.len()));
            }
            if result.files_removed > 0 {
                parts.push(format!("{} files removed", result.files_removed));
            }
            if result.previews_removed > 0 {
                parts.push(format!("{} previews removed", result.previews_removed));
            }
            println!("Delete complete: {}", parts.join(", "));
        } else {
            let mut parts = vec![
                format!("{} would be deleted", result.deleted),
            ];
            if !result.not_found.is_empty() {
                parts.push(format!("{} not found", result.not_found.len()));
            }
            println!("Delete (dry run): {}", parts.join(", "));
            if result.deleted > 0 {
                println!("  Run with --apply to delete.");
            }
        }
    }
    Ok(())
}

/// Extracted body of `Commands::Split`. See `run_import_command` for the
/// extraction pattern.
pub fn run_split_command(
        asset_id: String,
        variant_hashes: Vec<String>,
        json: bool,
        log: bool,
        #[allow(unused_variables)] verbosity: maki::Verbosity,
) -> anyhow::Result<()> {
    #[allow(dead_code)]
    struct Ctx { json: bool, log: bool }
    let cli = Ctx { json, log };
    let catalog_root = maki::config::find_catalog_root()?;
    let engine = QueryEngine::new(&catalog_root);
    let result = engine.split(&asset_id, &variant_hashes)?;

    if cli.json {
        println!("{}", serde_json::to_string(&result)?);
    } else {
        let short_src = &result.source_id[..8];
        println!(
            "Split {} variant(s) from asset {short_src}",
            result.new_assets.len()
        );
        for new_asset in &result.new_assets {
            let short_id = &new_asset.asset_id[..8];
            println!(
                "  → {short_id} ({}, {})",
                new_asset.original_filename, new_asset.variant_hash
            );
        }
    }
    Ok(())
}

/// Extracted body of `Commands::Group`. See `run_import_command` for the
/// extraction pattern.
pub fn run_group_command(
        variant_hashes: Vec<String>,
        json: bool,
        log: bool,
        #[allow(unused_variables)] verbosity: maki::Verbosity,
) -> anyhow::Result<()> {
    #[allow(dead_code)]
    struct Ctx { json: bool, log: bool }
    let cli = Ctx { json, log };
    let catalog_root = maki::config::find_catalog_root()?;
    let engine = QueryEngine::new(&catalog_root);
    let result = engine.group(&variant_hashes)?;

    if cli.json {
        println!("{}", serde_json::json!({
            "target_id": result.target_id,
            "variants_moved": result.variants_moved,
            "donors_removed": result.donors_removed,
        }));
    } else {
        let short_id = &result.target_id[..8];
        println!(
            "Grouped {} variant(s) into asset {short_id}",
            variant_hashes.len()
        );
        if result.donors_removed > 0 {
            println!("  Merged {} donor asset(s)", result.donors_removed);
        } else {
            println!("  Already grouped (no changes)");
        }
    }
    Ok(())
}

/// Extracted body of `Commands::Edit`. See `run_import_command` for the
/// extraction pattern.
pub fn run_edit_command(
        asset_id: String,
        name: Option<String>,
        clear_name: bool,
        description: Option<String>,
        clear_description: bool,
        rating: Option<u8>,
        clear_rating: bool,
        label: Option<String>,
        clear_label: bool,
        clear_tags: bool,
        date: Option<String>,
        clear_date: bool,
        role: Option<String>,
        variant: Option<String>,
        json: bool,
        log: bool,
        #[allow(unused_variables)] verbosity: maki::Verbosity,
) -> anyhow::Result<()> {
    #[allow(dead_code)]
    struct Ctx { json: bool, log: bool }
    let cli = Ctx { json, log };
    use maki::query::{EditFields, parse_date_input};

    // Handle --role --variant separately from asset-level edits
    if role.is_some() || variant.is_some() {
        let role = role.ok_or_else(|| anyhow::anyhow!("--variant requires --role"))?;
        let variant_hash = variant.ok_or_else(|| anyhow::anyhow!("--role requires --variant"))?;

        let catalog_root = maki::config::find_catalog_root()?;
        let engine = QueryEngine::new(&catalog_root);
        engine.set_variant_role(&asset_id, &variant_hash, &role)?;

        if cli.json {
            println!("{}", serde_json::json!({
                "asset_id": asset_id,
                "variant": variant_hash,
                "role": role,
            }));
        } else {
            let short_hash = &variant_hash[..16.min(variant_hash.len())];
            println!("Variant {short_hash}… role set to {role}");
        }
        return Ok(());
    }

    if name.is_none() && !clear_name && description.is_none() && !clear_description && rating.is_none() && !clear_rating && label.is_none() && !clear_label && !clear_tags && date.is_none() && !clear_date {
        anyhow::bail!("no edit flags provided. Use --name, --description, --rating, --label, --date, --role/--variant, or --clear-*.");
    }

    // Validate label if provided
    let label_field = if clear_label {
        Some(None)
    } else if let Some(ref l) = label {
        match maki::models::Asset::validate_color_label(l) {
            Ok(canonical) => Some(canonical),
            Err(e) => anyhow::bail!(e),
        }
    } else {
        None
    };

    // Parse date if provided
    let date_field = if clear_date {
        Some(None)
    } else if let Some(ref d) = date {
        Some(Some(parse_date_input(d)?))
    } else {
        None
    };

    let fields = EditFields {
        name: if clear_name {
            Some(None)
        } else {
            name.map(Some)
        },
        description: if clear_description {
            Some(None)
        } else {
            description.map(Some)
        },
        rating: if clear_rating {
            Some(None)
        } else {
            rating.map(Some)
        },
        color_label: label_field,
        created_at: date_field,
    };

    let catalog_root = maki::config::find_catalog_root()?;
    let engine = QueryEngine::new(&catalog_root);

    // Clear all tags if requested (before edit, so JSON output includes the result)
    let tags_cleared = if clear_tags {
        let details = engine.show(&asset_id)?;
        if !details.tags.is_empty() {
            let tag_result = engine.tag(&asset_id, &details.tags, true)?;
            tag_result.current_tags.is_empty()
        } else {
            true
        }
    } else {
        false
    };

    let result = engine.edit(&asset_id, fields)?;

    if cli.json {
        let mut json = serde_json::to_value(&result)?;
        if clear_tags {
            json["tags_cleared"] = serde_json::json!(tags_cleared);
        }
        println!("{}", serde_json::to_string_pretty(&json)?);
    } else {
        if let Some(name) = &result.name {
            println!("Name: {name}");
        } else {
            println!("Name: (none)");
        }
        if let Some(desc) = &result.description {
            println!("Description: {desc}");
        } else {
            println!("Description: (none)");
        }
        if let Some(r) = result.rating {
            let stars: String = (1..=5).map(|i| if i <= r { '\u{2605}' } else { '\u{2606}' }).collect();
            println!("Rating: {stars} ({r}/5)");
        } else {
            println!("Rating: (none)");
        }
        if let Some(l) = &result.color_label {
            println!("Label: {l}");
        } else {
            println!("Label: (none)");
        }
        if tags_cleared {
            println!("Tags: cleared");
        }
        // Show date (truncate to YYYY-MM-DD)
        let date_display = result.created_at.split('T').next().unwrap_or(&result.created_at);
        println!("Date: {date_display}");
    }
    Ok(())
}

/// Extracted body of `Commands::Preview`. See `run_import_command` for the
/// extraction pattern.
pub fn run_preview_command(
        asset_id: String,
        json: bool,
        log: bool,
        #[allow(unused_variables)] verbosity: maki::Verbosity,
) -> anyhow::Result<()> {
    #[allow(dead_code)]
    struct Ctx { json: bool, log: bool }
    let cli = Ctx { json, log };
    let (catalog_root, config) = maki::config::load_config()?;
    let catalog = maki::catalog::Catalog::open(&catalog_root)?;
    let engine = QueryEngine::new(&catalog_root);
    let details = engine.show(&asset_id)?;
    let full_id = &details.id;
    let preview_gen = maki::preview::PreviewGenerator::new(&catalog_root, verbosity, &config.preview);

    // Find best preview file (smart preview > regular preview)
    let best_hash = catalog.get_asset_best_variant_hash(full_id)
        .unwrap_or(None)
        .or_else(|| {
            maki::models::variant::best_preview_index_details(&details.variants)
                .map(|i| details.variants[i].content_hash.clone())
        });

    let preview_path = best_hash.as_ref().and_then(|h| {
        let smart = preview_gen.smart_preview_path(h);
        if smart.exists() { return Some(smart); }
        let regular = preview_gen.preview_path(h);
        if regular.exists() { return Some(regular); }
        None
    });

    match preview_path {
        Some(path) => {
            maki::preview::open_in_viewer(&path)?;
            if !cli.json {
                let name = details.name.as_deref().unwrap_or(full_id);
                eprintln!("Opened preview for {name}");
            }
            if cli.json {
                println!("{}", serde_json::json!({
                    "id": full_id,
                    "preview": path.display().to_string(),
                }));
            }
        }
        None => {
            let name = details.name.as_deref().unwrap_or(full_id);
            if cli.json {
                println!("{}", serde_json::json!({
                    "id": full_id,
                    "preview": null,
                }));
            } else {
                eprintln!("No preview available for {name}");
            }
        }
    }

    Ok(())
}

/// Extracted body of `Commands::Show`. See `run_import_command` for the
/// extraction pattern.
pub fn run_show_command(
        asset_id: String,
        locations: bool,
        json: bool,
        log: bool,
        #[allow(unused_variables)] verbosity: maki::Verbosity,
) -> anyhow::Result<()> {
    #[allow(dead_code)]
    struct Ctx { json: bool, log: bool }
    let cli = Ctx { json, log };
    let (catalog_root, config) = maki::config::load_config()?;
    let engine = QueryEngine::new(&catalog_root);
    let details = engine.show(&asset_id)?;

    if locations {
        if cli.json {
            let locs: Vec<serde_json::Value> = details.variants.iter()
                .flat_map(|v| v.locations.iter().map(move |loc| {
                    serde_json::json!({
                        "volume": loc.volume_label,
                        "path": loc.relative_path,
                        "variant": v.original_filename,
                        "format": v.format,
                        "role": v.role,
                    })
                }))
                .collect();
            let recipe_locs: Vec<serde_json::Value> = details.recipes.iter()
                .filter_map(|r| {
                    let label = r.volume_label.as_deref()?;
                    let path = r.relative_path.as_deref()?;
                    Some(serde_json::json!({
                        "volume": label,
                        "path": path,
                        "variant": r.software,
                        "format": r.recipe_type,
                        "role": "recipe",
                    }))
                })
                .collect();
            let all: Vec<serde_json::Value> = locs.into_iter().chain(recipe_locs).collect();
            println!("{}", serde_json::to_string_pretty(&all)?);
        } else {
            for v in &details.variants {
                for loc in &v.locations {
                    println!("{}:{}", loc.volume_label, loc.relative_path);
                }
            }
            for r in &details.recipes {
                if let (Some(label), Some(path)) = (&r.volume_label, &r.relative_path) {
                    println!("{label}:{path}");
                }
            }
        }
        return Ok(());
    }

    if cli.json {
        println!("{}", serde_json::to_string_pretty(&details)?);
    } else {
        let preview_gen = maki::preview::PreviewGenerator::new(&catalog_root, verbosity, &config.preview);

        println!("Asset: {}", details.id);
        if let Some(name) = &details.name {
            println!("Name:  {name}");
        }
        println!("Type:  {}", details.asset_type);
        println!("Date:  {}", details.created_at);
        if !details.tags.is_empty() {
            let display_tags: Vec<String> = details.tags.iter()
                .map(|t| maki::tag_util::tag_storage_to_display(t))
                .collect();
            println!("Tags:  {}", display_tags.join(", "));
        }
        if let Some(rating) = details.rating {
            let stars: String = (1..=5).map(|i| if i <= rating { '\u{2605}' } else { '\u{2606}' }).collect();
            println!("Rating: {stars} ({rating}/5)");
        }
        if let Some(label) = &details.color_label {
            println!("Label: {label}");
        }
        if let Some(desc) = &details.description {
            println!("Description: {desc}");
        }

        // Show preview status for the best preview variant
        if let Some(idx) = maki::models::variant::best_preview_index_details(&details.variants) {
            let v = &details.variants[idx];
            let preview_path = preview_gen.preview_path(&v.content_hash);
            if preview_gen.has_preview(&v.content_hash) {
                println!("Preview: {}", preview_path.display());
            } else {
                println!("Preview: (none)");
            }
        }

        if !details.variants.is_empty() {
            println!("\nVariants:");
            for v in &details.variants {
                println!(
                    "  [{}] {} ({}, {})",
                    v.role,
                    v.original_filename,
                    v.format,
                    format_size(v.file_size)
                );
                println!("    Hash: {}", v.content_hash);
                for loc in &v.locations {
                    println!(
                        "    Location: {} \u{2192} {}",
                        loc.volume_label, loc.relative_path
                    );
                }
                if !v.source_metadata.is_empty() {
                    let mut keys: Vec<&String> = v.source_metadata.keys().collect();
                    keys.sort();
                    for key in keys {
                        println!("    {}: {}", key, v.source_metadata[key]);
                    }
                }
            }
        }

        if !details.recipes.is_empty() {
            println!("\nRecipes:");
            for r in &details.recipes {
                let short_variant = &r.variant_hash[r.variant_hash.len().saturating_sub(8)..];
                println!("  [{}] {} → …{} ({})", r.recipe_type, r.software, short_variant, r.content_hash);
                if let Some(path) = &r.relative_path {
                    let label = r.volume_label.as_deref().unwrap_or("?");
                    println!("    Location: {label}:{path}");
                }
            }
        }
    }

    Ok(())
}
