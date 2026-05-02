//! Import job routes (start, dry-run, profiles, build-info).
//!
//! Live progress, status, and SSE re-attach are served by the generic
//! job-registry endpoints in `routes::jobs` (`GET /api/jobs`, `GET /api/jobs/{id}/progress`).
//! The import dialog and nav badge use those directly; this module only
//! handles the import-specific control surfaces.

use std::sync::Arc;
use std::sync::atomic::Ordering::Relaxed;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;

use super::super::AppState;
use crate::web::jobs::{Job, JobKind};
use crate::web::ImportJobSummary;

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
            run_import_dry(&state, &req)
        })
        .await;

        return match result {
            Ok(Ok(json)) => Json(json).into_response(),
            Ok(Err(e)) => (StatusCode::BAD_REQUEST, format!("{e:#}")).into_response(),
            Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
        };
    }

    // At-most-one running import. Still allowed to start once the most recent
    // one has finished — the registry keeps completed jobs around for re-attach.
    if let Some(latest) = state.jobs.latest(JobKind::Import) {
        if !latest.is_completed() {
            return (StatusCode::CONFLICT, "An import is already running").into_response();
        }
    }

    let job = state.jobs.start(JobKind::Import);
    let job_id = job.id.clone();

    let state2 = state.clone();
    let job_for_task = job.clone();
    tokio::spawn(async move {
        let state3 = state2.clone();
        let job_inner = job_for_task.clone();
        let result = tokio::task::spawn_blocking(move || {
            run_import_with_progress(&state3, &req, &job_inner)
        })
        .await;

        let terminal = match result {
            Ok(Ok(json)) => json,
            Ok(Err(e)) => serde_json::json!({"error": format!("{e:#}")}),
            Err(e) => serde_json::json!({"error": format!("{e}")}),
        };
        job_for_task.finish(terminal);
        state2.jobs.mark_done(&job_for_task.id);
    });

    Json(serde_json::json!({"job_id": job_id, "status": "started"})).into_response()
}

/// Synchronous dry-run path. Reports counts without emitting progress.
fn run_import_dry(
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
        true, // dry_run
        smart,
        |_, _, _| {},
    )?;

    Ok(serde_json::json!({
        "dry_run": true,
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
    job: &Arc<Job>,
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

    // Per-import counters owned by this task. The job registry's progress
    // snapshot is updated from here on every event, so SSE clients and the
    // status endpoint see the same totals.
    let summary = Arc::new(ImportJobSummary::default());

    let summary_for_cb = summary.clone();
    let job_for_cb = job.clone();
    let result = service.import_with_callback(
        &[import_path],
        &volume,
        &filter,
        &import_config.exclude,
        &tags,
        false,
        smart,
        move |path, status, _elapsed| {
            let label = match status {
                crate::asset_service::FileStatus::Imported => {
                    summary_for_cb.imported.fetch_add(1, Relaxed);
                    "imported"
                }
                crate::asset_service::FileStatus::LocationAdded => {
                    summary_for_cb.locations_added.fetch_add(1, Relaxed);
                    "location"
                }
                crate::asset_service::FileStatus::Skipped => {
                    summary_for_cb.skipped.fetch_add(1, Relaxed);
                    "skipped"
                }
                crate::asset_service::FileStatus::RecipeAttached
                | crate::asset_service::FileStatus::RecipeLocationAdded
                | crate::asset_service::FileStatus::RecipeUpdated => {
                    summary_for_cb.recipes.fetch_add(1, Relaxed);
                    "recipe"
                }
            };
            let file = path
                .file_name()
                .map(|f| f.to_string_lossy().to_string())
                .unwrap_or_default();
            let evt = serde_json::json!({
                "phase": "import",
                "done": false,
                "file": file,
                "status": label,
                "imported": summary_for_cb.imported.load(Relaxed),
                "skipped": summary_for_cb.skipped.load(Relaxed),
                "locations_added": summary_for_cb.locations_added.load(Relaxed),
                "recipes": summary_for_cb.recipes.load(Relaxed),
            });
            job_for_cb.emit(&evt);
        },
    )?;

    if req.auto_group.unwrap_or(false) && (result.imported > 0 || result.locations_added > 0) {
        let engine = crate::query::QueryEngine::new(&state.catalog_root);
        let _ = engine.auto_group(&result.new_asset_ids, false);
    }

    // Post-import embed phase (AI feature).
    #[cfg(feature = "ai")]
    let embed_summary = run_post_import_embed(state, &config, req, &result.new_asset_ids, job, &summary);

    // Post-import describe phase (Pro feature).
    #[cfg(feature = "pro")]
    let describe_summary = run_post_import_describe(state, &config, req, &result.new_asset_ids, job, &summary);

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
    job: &Arc<Job>,
    summary: &Arc<ImportJobSummary>,
) -> Option<(u32, u32)> {
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
        job.emit(&serde_json::json!({
            "phase": "embed",
            "done": false,
            "status": "skipped",
            "message": "model not downloaded",
        }));
        return None;
    }

    let service = crate::asset_service::AssetService::new(&state.catalog_root, state.verbosity, &config.preview);
    let summary_for_cb = summary.clone();
    let job_for_cb = job.clone();
    let r = service.embed_assets(
        new_asset_ids,
        &model_dir,
        model_id,
        &config.ai.execution_provider,
        false,
        move |aid, status, _elapsed| {
            let label = match status {
                crate::asset_service::EmbedStatus::Embedded => {
                    summary_for_cb.embedded.fetch_add(1, Relaxed);
                    "embedded"
                }
                crate::asset_service::EmbedStatus::Skipped(_) => "skipped",
                crate::asset_service::EmbedStatus::Error(_) => "error",
            };
            let short = &aid[..8.min(aid.len())];
            let evt = serde_json::json!({
                "phase": "embed",
                "done": false,
                "status": label,
                "asset": short,
                "embedded": summary_for_cb.embedded.load(Relaxed),
            });
            job_for_cb.emit(&evt);
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
    job: &Arc<Job>,
    summary: &Arc<ImportJobSummary>,
) -> Option<(u32, u32)> {
    let opted_in = req.describe.unwrap_or(config.import.descriptions);
    if !opted_in || new_asset_ids.is_empty() {
        return None;
    }

    let endpoint = &config.vlm.endpoint;
    let vlm_model = &config.vlm.model;
    if crate::vlm::check_endpoint(endpoint, 5, state.verbosity).is_err() {
        job.emit(&serde_json::json!({
            "phase": "describe",
            "done": false,
            "status": "skipped",
            "message": format!("VLM endpoint unavailable at {endpoint}"),
        }));
        return None;
    }

    let mode = crate::vlm::DescribeMode::from_str(&config.vlm.mode)
        .unwrap_or(crate::vlm::DescribeMode::Describe);
    let params = config.vlm.params_for_model(vlm_model);
    let service = crate::asset_service::AssetService::new(&state.catalog_root, state.verbosity, &config.preview);
    let summary_for_cb = summary.clone();
    let job_for_cb = job.clone();
    let r = service.describe_assets(
        new_asset_ids,
        endpoint,
        vlm_model,
        &params,
        mode,
        false, // force
        false, // dry_run
        config.vlm.concurrency,
        move |aid, status, _elapsed| {
            let label = match status {
                crate::vlm::DescribeStatus::Described => {
                    summary_for_cb.described.fetch_add(1, Relaxed);
                    "described"
                }
                crate::vlm::DescribeStatus::Skipped(_) => "skipped",
                crate::vlm::DescribeStatus::Error(_) => "error",
            };
            let short = &aid[..8.min(aid.len())];
            let evt = serde_json::json!({
                "phase": "describe",
                "done": false,
                "status": label,
                "asset": short,
                "described": summary_for_cb.described.load(Relaxed),
            });
            job_for_cb.emit(&evt);
        },
    ).ok()?;
    Some((r.described as u32, r.skipped as u32))
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
