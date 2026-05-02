//! Import job routes (start, SSE progress, status).

use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;

use super::super::AppState;

#[derive(Debug, serde::Deserialize)]
pub struct StartImportRequest {
    pub volume_id: String,
    pub subfolder: Option<String>,
    pub profile: Option<String>,
    pub tags: Option<Vec<String>>,
    pub auto_group: Option<bool>,
    pub smart: Option<bool>,
    /// Generate embeddings after import. Only honored on `ai` builds; silently
    /// ignored otherwise so the JSON shape stays the same across feature sets.
    /// `None` falls back to `[import] embeddings` from config.
    #[allow(dead_code)]
    pub embed: Option<bool>,
    /// Generate VLM descriptions after import. Only honored on `pro` builds;
    /// silently ignored otherwise. `None` falls back to `[import] descriptions`.
    #[allow(dead_code)]
    pub describe: Option<bool>,
    pub dry_run: Option<bool>,
}

/// POST /api/import — start an import job (or run dry-run synchronously).
pub async fn start_import_api(
    State(state): State<Arc<AppState>>,
    Json(req): Json<StartImportRequest>,
) -> Response {
    let dry_run = req.dry_run.unwrap_or(false);

    if dry_run {
        let state = state.clone();
        let result = tokio::task::spawn_blocking(move || {
            run_import(&state, &req)
        })
        .await;

        return match result {
            Ok(Ok(json)) => Json(json).into_response(),
            Ok(Err(e)) => (StatusCode::BAD_REQUEST, format!("{e:#}")).into_response(),
            Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
        };
    }

    {
        let lock = state.import_job.lock().unwrap();
        if lock.is_some() {
            return (StatusCode::CONFLICT, "An import is already running").into_response();
        }
    }

    let (tx, _rx) = tokio::sync::broadcast::channel::<String>(512);
    let job_id = uuid::Uuid::new_v4().to_string();

    {
        let mut lock = state.import_job.lock().unwrap();
        *lock = Some(crate::web::ImportJob {
            job_id: job_id.clone(),
            sender: tx.clone(),
            started_at: chrono::Utc::now(),
            recent_events: std::sync::Mutex::new(std::collections::VecDeque::with_capacity(
                crate::web::IMPORT_RECENT_EVENTS_CAP,
            )),
            summary: crate::web::ImportJobSummary::default(),
        });
    }

    let state2 = state.clone();
    tokio::spawn(async move {
        let tx2 = tx.clone();
        let state3 = state2.clone();
        let result = tokio::task::spawn_blocking(move || {
            run_import_with_progress(&state3, &req, &tx2)
        })
        .await;

        let done_event = match &result {
            Ok(Ok(json)) => {
                let mut obj = json.clone();
                obj.as_object_mut().unwrap().insert("done".to_string(), serde_json::json!(true));
                serde_json::to_string(&obj).unwrap_or_default()
            }
            Ok(Err(e)) => serde_json::json!({"done": true, "error": format!("{e:#}")}).to_string(),
            Err(e) => serde_json::json!({"done": true, "error": format!("{e}")}).to_string(),
        };
        let _ = tx.send(done_event);

        let mut lock = state2.import_job.lock().unwrap();
        *lock = None;
    });

    Json(serde_json::json!({"job_id": job_id, "status": "started"})).into_response()
}

fn run_import(
    state: &AppState,
    req: &StartImportRequest,
) -> anyhow::Result<serde_json::Value> {
    let registry = crate::device_registry::DeviceRegistry::new(&state.catalog_root);
    let volume = registry.resolve_volume(&req.volume_id)?;
    let config = crate::config::CatalogConfig::load(&state.catalog_root).unwrap_or_default();

    let import_config = if let Some(ref profile_name) = req.profile {
        config.import.resolve_profile(profile_name)
            .ok_or_else(|| anyhow::anyhow!("unknown import profile: {profile_name}"))?
    } else {
        config.import.clone()
    };

    let filter = crate::asset_service::FileTypeFilter::default();
    let mut tags: Vec<String> = import_config.auto_tags.clone();
    if let Some(ref extra) = req.tags {
        tags.extend(extra.iter().cloned());
    }
    tags.sort();
    tags.dedup();

    let dry_run = req.dry_run.unwrap_or(false);
    let smart = req.smart.unwrap_or(import_config.smart_previews);

    let mut import_path = volume.mount_point.clone();
    if let Some(ref sub) = req.subfolder {
        if !sub.is_empty() {
            import_path = import_path.join(sub);
        }
    }
    if !import_path.exists() {
        anyhow::bail!("path does not exist: {}", import_path.display());
    }

    let service = state.asset_service();
    let result = service.import_with_callback(
        &[import_path],
        &volume,
        &filter,
        &import_config.exclude,
        &tags,
        dry_run,
        smart,
        |_, _, _| {},
    )?;

    Ok(serde_json::json!({
        "dry_run": dry_run,
        "imported": result.imported,
        "locations_added": result.locations_added,
        "skipped": result.skipped,
        "recipes_attached": result.recipes_attached,
        "recipes_updated": result.recipes_updated,
        "previews_generated": result.previews_generated,
        "new_asset_ids": result.new_asset_ids,
    }))
}

fn run_import_with_progress(
    state: &AppState,
    req: &StartImportRequest,
    tx: &tokio::sync::broadcast::Sender<String>,
) -> anyhow::Result<serde_json::Value> {
    let registry = crate::device_registry::DeviceRegistry::new(&state.catalog_root);
    let volume = registry.resolve_volume(&req.volume_id)?;
    let config = crate::config::CatalogConfig::load(&state.catalog_root).unwrap_or_default();

    let import_config = if let Some(ref profile_name) = req.profile {
        config.import.resolve_profile(profile_name)
            .ok_or_else(|| anyhow::anyhow!("unknown import profile: {profile_name}"))?
    } else {
        config.import.clone()
    };

    let filter = crate::asset_service::FileTypeFilter::default();
    let mut tags: Vec<String> = import_config.auto_tags.clone();
    if let Some(ref extra) = req.tags {
        tags.extend(extra.iter().cloned());
    }
    tags.sort();
    tags.dedup();

    let smart = req.smart.unwrap_or(import_config.smart_previews);

    let mut import_path = volume.mount_point.clone();
    if let Some(ref sub) = req.subfolder {
        if !sub.is_empty() {
            import_path = import_path.join(sub);
        }
    }
    if !import_path.exists() {
        anyhow::bail!("path does not exist: {}", import_path.display());
    }

    let service = state.asset_service();

    let result = service.import_with_callback(
        &[import_path],
        &volume,
        &filter,
        &import_config.exclude,
        &tags,
        false,
        smart,
        |path, status, _elapsed| {
            // Update the shared summary + ring buffer under a brief lock so
            // the status endpoint and re-attaching SSE clients see consistent
            // counts. Lock is dropped before broadcast.send to avoid blocking
            // subscribers on producer work.
            use std::sync::atomic::Ordering::Relaxed;
            let label;
            let evt_json: String = {
                let lock = state.import_job.lock().unwrap();
                let job = match lock.as_ref() {
                    Some(j) => j,
                    None => return, // job was cleared somehow; skip event
                };
                label = match status {
                    crate::asset_service::FileStatus::Imported => {
                        job.summary.imported.fetch_add(1, Relaxed);
                        "imported"
                    }
                    crate::asset_service::FileStatus::LocationAdded => {
                        job.summary.locations_added.fetch_add(1, Relaxed);
                        "location"
                    }
                    crate::asset_service::FileStatus::Skipped => {
                        job.summary.skipped.fetch_add(1, Relaxed);
                        "skipped"
                    }
                    crate::asset_service::FileStatus::RecipeAttached |
                    crate::asset_service::FileStatus::RecipeLocationAdded |
                    crate::asset_service::FileStatus::RecipeUpdated => {
                        job.summary.recipes.fetch_add(1, Relaxed);
                        "recipe"
                    }
                };
                let file = path
                    .file_name()
                    .map(|f| f.to_string_lossy().to_string())
                    .unwrap_or_default();
                let evt = serde_json::json!({
                    "done": false,
                    "file": file,
                    "status": label,
                    "imported": job.summary.imported.load(Relaxed),
                    "skipped": job.summary.skipped.load(Relaxed),
                    "locations_added": job.summary.locations_added.load(Relaxed),
                    "recipes": job.summary.recipes.load(Relaxed),
                });
                let evt_str = evt.to_string();

                // Push to ring buffer, evict oldest if at capacity.
                if let Ok(mut buf) = job.recent_events.lock() {
                    if buf.len() >= crate::web::IMPORT_RECENT_EVENTS_CAP {
                        buf.pop_front();
                    }
                    buf.push_back(evt_str.clone());
                }
                evt_str
            };
            let _ = tx.send(evt_json);
        },
    )?;

    if req.auto_group.unwrap_or(false) && (result.imported > 0 || result.locations_added > 0) {
        let engine = crate::query::QueryEngine::new(&state.catalog_root);
        let _ = engine.auto_group(&result.new_asset_ids, false);
    }

    // Post-import embed phase (AI feature). Same trigger semantics as the CLI:
    // explicit `req.embed = true` OR config default `[import] embeddings = true`.
    #[cfg(feature = "ai")]
    let embed_summary = run_post_import_embed(state, &config, req, &result.new_asset_ids, tx);

    // Post-import describe phase (Pro feature).
    #[cfg(feature = "pro")]
    let describe_summary = run_post_import_describe(state, &config, req, &result.new_asset_ids, tx);

    #[allow(unused_mut)]
    let mut out = serde_json::json!({
        "imported": result.imported,
        "locations_added": result.locations_added,
        "skipped": result.skipped,
        "recipes_attached": result.recipes_attached,
        "recipes_updated": result.recipes_updated,
        "previews_generated": result.previews_generated,
        "new_asset_ids": result.new_asset_ids,
    });
    #[cfg(feature = "ai")]
    if let Some((embedded, embed_skipped)) = embed_summary {
        out["embedded"] = serde_json::json!(embedded);
        out["embeddings_skipped"] = serde_json::json!(embed_skipped);
    }
    #[cfg(feature = "pro")]
    if let Some((described, describe_skipped)) = describe_summary {
        out["described"] = serde_json::json!(described);
        out["descriptions_skipped"] = serde_json::json!(describe_skipped);
    }
    Ok(out)
}

/// Run the post-import embedding phase. Returns `Some((embedded, skipped))` when
/// the phase ran (model present, opted in), or `None` when skipped.
#[cfg(feature = "ai")]
fn run_post_import_embed(
    state: &AppState,
    config: &crate::config::CatalogConfig,
    req: &StartImportRequest,
    new_asset_ids: &[String],
    tx: &tokio::sync::broadcast::Sender<String>,
) -> Option<(u32, u32)> {
    use std::sync::atomic::Ordering::Relaxed;
    let opted_in = req.embed.unwrap_or(config.import.embeddings);
    if !opted_in || new_asset_ids.is_empty() {
        return None;
    }

    let model_id = &config.ai.model;
    let model_dir_str = &config.ai.model_dir;
    let model_base = if model_dir_str.starts_with("~/") {
        let home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).ok()?;
        std::path::PathBuf::from(home).join(&model_dir_str[2..])
    } else {
        std::path::PathBuf::from(model_dir_str)
    };
    let model_dir = model_base.join(model_id);
    let mgr = match crate::model_manager::ModelManager::new(&model_dir, model_id) {
        Ok(m) => m,
        Err(_) => return None,
    };
    if !mgr.model_exists() {
        let evt = serde_json::json!({
            "done": false,
            "phase": "embed",
            "status": "skipped",
            "message": "model not downloaded",
        });
        let _ = tx.send(evt.to_string());
        return None;
    }

    let service = crate::asset_service::AssetService::new(&state.catalog_root, state.verbosity, &config.preview);
    let r = service.embed_assets(
        new_asset_ids,
        &model_dir,
        model_id,
        &config.ai.execution_provider,
        false,
        |aid, status, _elapsed| {
            let lock = state.import_job.lock().unwrap();
            let job = match lock.as_ref() { Some(j) => j, None => return };
            let label = match status {
                crate::asset_service::EmbedStatus::Embedded => {
                    job.summary.embedded.fetch_add(1, Relaxed);
                    "embedded"
                }
                crate::asset_service::EmbedStatus::Skipped(_) => "skipped",
                crate::asset_service::EmbedStatus::Error(_) => "error",
            };
            let short = &aid[..8.min(aid.len())];
            let evt = serde_json::json!({
                "done": false,
                "phase": "embed",
                "status": label,
                "asset": short,
                "embedded": job.summary.embedded.load(Relaxed),
            });
            let evt_str = evt.to_string();
            if let Ok(mut buf) = job.recent_events.lock() {
                if buf.len() >= crate::web::IMPORT_RECENT_EVENTS_CAP {
                    buf.pop_front();
                }
                buf.push_back(evt_str.clone());
            }
            drop(lock);
            let _ = tx.send(evt_str);
        },
    ).ok()?;
    Some((r.embedded, r.skipped))
}

/// Run the post-import VLM describe phase. Returns `Some((described, skipped))`
/// when the phase ran (endpoint reachable, opted in), or `None` when skipped.
#[cfg(feature = "pro")]
fn run_post_import_describe(
    state: &AppState,
    config: &crate::config::CatalogConfig,
    req: &StartImportRequest,
    new_asset_ids: &[String],
    tx: &tokio::sync::broadcast::Sender<String>,
) -> Option<(u32, u32)> {
    use std::sync::atomic::Ordering::Relaxed;
    let opted_in = req.describe.unwrap_or(config.import.descriptions);
    if !opted_in || new_asset_ids.is_empty() {
        return None;
    }

    let endpoint = &config.vlm.endpoint;
    let vlm_model = &config.vlm.model;
    if crate::vlm::check_endpoint(endpoint, 5, state.verbosity).is_err() {
        let evt = serde_json::json!({
            "done": false,
            "phase": "describe",
            "status": "skipped",
            "message": format!("VLM endpoint unavailable at {endpoint}"),
        });
        let _ = tx.send(evt.to_string());
        return None;
    }

    let mode = crate::vlm::DescribeMode::from_str(&config.vlm.mode)
        .unwrap_or(crate::vlm::DescribeMode::Describe);
    let params = config.vlm.params_for_model(vlm_model);
    let service = crate::asset_service::AssetService::new(&state.catalog_root, state.verbosity, &config.preview);
    let r = service.describe_assets(
        new_asset_ids,
        endpoint,
        vlm_model,
        &params,
        mode,
        false, // force
        false, // dry_run
        config.vlm.concurrency,
        |aid, status, _elapsed| {
            let lock = state.import_job.lock().unwrap();
            let job = match lock.as_ref() { Some(j) => j, None => return };
            let label = match status {
                crate::vlm::DescribeStatus::Described => {
                    job.summary.described.fetch_add(1, Relaxed);
                    "described"
                }
                crate::vlm::DescribeStatus::Skipped(_) => "skipped",
                crate::vlm::DescribeStatus::Error(_) => "error",
            };
            let short = &aid[..8.min(aid.len())];
            let evt = serde_json::json!({
                "done": false,
                "phase": "describe",
                "status": label,
                "asset": short,
                "described": job.summary.described.load(Relaxed),
            });
            let evt_str = evt.to_string();
            if let Ok(mut buf) = job.recent_events.lock() {
                if buf.len() >= crate::web::IMPORT_RECENT_EVENTS_CAP {
                    buf.pop_front();
                }
                buf.push_back(evt_str.clone());
            }
            drop(lock);
            let _ = tx.send(evt_str);
        },
    ).ok()?;
    Some((r.described as u32, r.skipped as u32))
}

/// GET /api/import/progress — SSE stream of import progress events.
///
/// On connect: replays the ring buffer (up to IMPORT_RECENT_EVENTS_CAP recent
/// events) so a re-attaching client sees what it missed, then continues with
/// the live broadcast. Subscribe-before-snapshot ordering means a producer
/// event landing in that window arrives via broadcast (the small chance of a
/// duplicate is preferable to a missed event for the user).
pub async fn import_progress_sse(
    State(state): State<Arc<AppState>>,
) -> Response {
    use axum::response::sse::{Event, KeepAlive, Sse};
    use tokio_stream::StreamExt;

    let (rx, replay) = {
        let lock = state.import_job.lock().unwrap();
        match lock.as_ref() {
            Some(job) => {
                // Subscribe first so any event emitted during snapshotting
                // still reaches us via broadcast.
                let rx = job.sender.subscribe();
                let snapshot: Vec<String> = job
                    .recent_events
                    .lock()
                    .map(|buf| buf.iter().cloned().collect())
                    .unwrap_or_default();
                (rx, snapshot)
            }
            None => {
                return (StatusCode::NOT_FOUND, "No import running").into_response();
            }
        }
    };

    let replay_stream = tokio_stream::iter(replay.into_iter().map(|d| Event::default().data(d)));
    let live_stream = tokio_stream::wrappers::BroadcastStream::new(rx)
        .filter_map(|msg| match msg {
            Ok(data) => Some(Event::default().data(data)),
            Err(_) => None,
        });
    let stream = replay_stream.chain(live_stream).map(Ok::<_, std::convert::Infallible>);

    Sse::new(stream).keep_alive(KeepAlive::default()).into_response()
}

/// GET /api/import/status — check if an import is running and report progress.
///
/// Returns `{running, job_id, started_at, imported, skipped, locations_added,
/// recipes}`. The nav badge polls this and re-attaching UI uses the counters
/// to seed initial state before the SSE stream arrives.
pub async fn import_status_api(
    State(state): State<Arc<AppState>>,
) -> Response {
    use std::sync::atomic::Ordering::Relaxed;
    let lock = state.import_job.lock().unwrap();
    match lock.as_ref() {
        Some(job) => Json(serde_json::json!({
            "running": true,
            "job_id": job.job_id,
            "started_at": job.started_at.to_rfc3339(),
            "imported": job.summary.imported.load(Relaxed),
            "skipped": job.summary.skipped.load(Relaxed),
            "locations_added": job.summary.locations_added.load(Relaxed),
            "recipes": job.summary.recipes.load(Relaxed),
            "embedded": job.summary.embedded.load(Relaxed),
            "described": job.summary.described.load(Relaxed),
        }))
        .into_response(),
        None => Json(serde_json::json!({"running": false})).into_response(),
    }
}

/// GET /api/build-info — report which optional features were compiled in.
///
/// Used by the import dialog to hide the Embeddings (ai) and Descriptions
/// (pro) checkboxes when the running binary doesn't support those phases.
pub async fn build_info_api() -> Response {
    let ai = cfg!(feature = "ai");
    let pro = cfg!(feature = "pro");
    Json(serde_json::json!({"ai": ai, "pro": pro})).into_response()
}

/// GET /api/import/profiles — list named import profiles from `[import.profiles.*]`.
///
/// Returns `{"profiles": [...]}`. The global import dialog (mounted in base.html
/// and reachable from every page) populates its profile dropdown from this rather
/// than from a template variable, so no per-page wiring is needed.
pub async fn import_profiles_api(
    State(state): State<Arc<AppState>>,
) -> Response {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let config = crate::config::CatalogConfig::load(&state.catalog_root).unwrap_or_default();
        let mut profiles: Vec<String> = config.import.profiles.keys().cloned().collect();
        profiles.sort();
        Ok::<_, anyhow::Error>(serde_json::json!({"profiles": profiles}))
    })
    .await;
    match result {
        Ok(Ok(json)) => Json(json).into_response(),
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e:#}")).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
    }
}
