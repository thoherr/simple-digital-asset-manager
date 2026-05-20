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

/// POST /api/import — start an import job. Honours `dry_run` by passing
/// the flag through to the workflow; the job machinery (progress SSE,
/// re-attach via `/api/jobs`, minimize-to-toast) treats dry-runs and
/// live imports identically. The terminal payload carries `dry_run`
/// so the client can label the summary "Would import N" vs
/// "Imported N".
pub async fn start_import_api(
    State(state): State<Arc<AppState>>,
    Json(req): Json<StartImportRequest>,
) -> Response {
    // At-most-one running import — applies to dry-runs too, since both
    // hold a catalog handle and traverse the same workflow. The
    // registry keeps completed jobs around for re-attach, so a
    // dry-run finishing doesn't block the next live import.
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


/// Build the workflow request shared by the dry-run and live-progress paths.
fn build_workflow_request(
    state: &AppState,
    req: &StartImportRequest,
) -> anyhow::Result<crate::asset_service::ImportRequest> {
    let registry = crate::device_registry::DeviceRegistry::new(&state.catalog_root);
    let volume = registry.resolve_volume(&req.volume_id)?;

    let mut import_path = volume.mount_point.clone();
    if let Some(ref sub) = req.subfolder {
        if !sub.is_empty() {
            import_path = import_path.join(sub);
        }
    }
    if !import_path.exists() {
        anyhow::bail!("path does not exist: {}", import_path.display());
    }

    Ok(crate::asset_service::ImportRequest {
        paths: vec![import_path],
        // Web always passes the resolved volume ID directly — workflow's
        // resolve_volume accepts ID or label.
        volume_label: Some(req.volume_id.clone()),
        profile: req.profile.clone(),
        include: Vec::new(),
        skip: Vec::new(),
        add_tags: req.tags.clone().unwrap_or_default(),
        dry_run: req.dry_run.unwrap_or(false),
        smart: req.smart.unwrap_or(false),
        auto_group: req.auto_group.unwrap_or(false),
        #[cfg(feature = "ai")]
        embed: req.embed.unwrap_or(false),
        #[cfg(not(feature = "ai"))]
        embed: false,
        #[cfg(feature = "pro")]
        describe: req.describe.unwrap_or(false),
        #[cfg(not(feature = "pro"))]
        describe: false,
    })
}

fn run_import_with_progress(
    state: &AppState,
    req: &StartImportRequest,
    job: &Arc<Job>,
) -> anyhow::Result<serde_json::Value> {
    use crate::asset_service::ImportEvent;

    let wf_req = build_workflow_request(state, req)?;
    let config = crate::config::CatalogConfig::load(&state.catalog_root).unwrap_or_default();
    let service = state.asset_service();

    // Per-job atomic counters. The workflow emits events sequentially within
    // each phase; describe runs concurrently but the workflow wraps the
    // callback in a Mutex so increments are still well-ordered.
    let summary = Arc::new(ImportJobSummary::default());

    let summary_for_cb = summary.clone();
    let job_for_cb = job.clone();
    let r = service.import_workflow(&wf_req, &config, move |evt| {
        match evt {
            ImportEvent::PhaseStarted(_) | ImportEvent::PhaseSkipped { .. } => {
                // Phase boundaries: surface them as low-priority events so
                // the toast/dialog can update its status text. PhaseSkipped
                // carries the reason for transparency.
                let payload = match evt {
                    ImportEvent::PhaseStarted(p) => serde_json::json!({
                        "phase": p.label(),
                        "done": false,
                        "status": "phase_started",
                    }),
                    ImportEvent::PhaseSkipped { phase, reason } => serde_json::json!({
                        "phase": phase.label(),
                        "done": false,
                        "status": "phase_skipped",
                        "message": reason,
                    }),
                    _ => unreachable!(),
                };
                job_for_cb.emit(&payload);
            }
            ImportEvent::File { path, status, elapsed: _ } => {
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
                job_for_cb.emit(&serde_json::json!({
                    "phase": "import",
                    "done": false,
                    "file": file,
                    "status": label,
                    "imported": summary_for_cb.imported.load(Relaxed),
                    "skipped": summary_for_cb.skipped.load(Relaxed),
                    "locations_added": summary_for_cb.locations_added.load(Relaxed),
                    "recipes": summary_for_cb.recipes.load(Relaxed),
                }));
            }
            #[cfg(feature = "ai")]
            ImportEvent::Embed { asset_id, status } => {
                let label = match status {
                    crate::asset_service::EmbedStatus::Embedded => {
                        summary_for_cb.embedded.fetch_add(1, Relaxed);
                        "embedded"
                    }
                    crate::asset_service::EmbedStatus::Skipped(_) => "skipped",
                    crate::asset_service::EmbedStatus::Error(_) => "error",
                };
                let short = &asset_id[..8.min(asset_id.len())];
                job_for_cb.emit(&serde_json::json!({
                    "phase": "embed",
                    "done": false,
                    "status": label,
                    "asset": short,
                    "embedded": summary_for_cb.embedded.load(Relaxed),
                }));
            }
            #[cfg(feature = "pro")]
            ImportEvent::Describe { asset_id, status, elapsed: _ } => {
                let label = match status {
                    crate::vlm::DescribeStatus::Described => {
                        summary_for_cb.described.fetch_add(1, Relaxed);
                        "described"
                    }
                    crate::vlm::DescribeStatus::Skipped(_) => "skipped",
                    crate::vlm::DescribeStatus::Error(_) => "error",
                };
                let short = &asset_id[..8.min(asset_id.len())];
                job_for_cb.emit(&serde_json::json!({
                    "phase": "describe",
                    "done": false,
                    "status": label,
                    "asset": short,
                    "described": summary_for_cb.described.load(Relaxed),
                }));
            }
        }
    })?;

    // Build the terminal payload from the workflow result. `dry_run`
    // is propagated so the client can label the summary "Would import
    // N" vs "Imported N" — same job, same SSE feed, only the framing
    // differs.
    #[allow(unused_mut)]
    let mut out = serde_json::json!({
        "dry_run": wf_req.dry_run,
        "imported": r.import.imported,
        "locations_added": r.import.locations_added,
        "skipped": r.import.skipped,
        "recipes_attached": r.import.recipes_attached,
        "recipes_updated": r.import.recipes_updated,
        "previews_generated": r.import.previews_generated,
        "new_asset_ids": r.import.new_asset_ids,
    });
    #[cfg(feature = "ai")]
    if let Some(ref er) = r.embed {
        out["embedded"] = serde_json::json!(er.embedded);
        out["embeddings_skipped"] = serde_json::json!(er.skipped);
    }
    #[cfg(feature = "pro")]
    if let Some(ref dr) = r.describe {
        out["described"] = serde_json::json!(dr.described);
        out["descriptions_skipped"] = serde_json::json!(dr.skipped);
    }
    Ok(out)
}


/// GET /api/build-info — report which optional features were compiled in,
/// plus a few user-config values the JS layer needs at runtime
/// (slideshow defaults, etc.). Used by the import dialog to gate the
/// Embeddings (ai) / Descriptions (pro) checkboxes; the lightbox JS uses
/// the slideshow values as the initial cadence + loop state.
pub async fn build_info_api(
    State(state): State<Arc<AppState>>,
) -> Response {
    let ai = cfg!(feature = "ai");
    let pro = cfg!(feature = "pro");
    Json(serde_json::json!({
        "ai": ai,
        "pro": pro,
        "slideshow_seconds": state.slideshow_seconds,
        "slideshow_loop": state.slideshow_loop,
        "remember_latest_filter": state.remember_latest_filter,
    })).into_response()
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
