//! Standalone embed endpoints — generate SigLIP embeddings without applying
//! any tags.
//!
//! Auto-tag generates embeddings as a side-effect of classification, but
//! users who only want similarity coverage need a path that doesn't apply
//! tags. Per-asset and batch flavours; the batch endpoint uses the
//! JobRegistry for live progress.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;

use crate::web::AppState;

// --- Standalone embed endpoints ---
//
// Auto-tag generates embeddings as a side-effect of classification, but users
// who only want similarity search (without tag suggestions) need a way to
// build the embedding without applying any tags. These endpoints expose
// `AssetService::embed_assets` directly for one asset or a batch.

#[derive(Debug, serde::Deserialize)]
pub struct BatchEmbedRequest {
    pub asset_ids: Vec<String>,
}


/// POST /api/batch/embed — start an embedding job for selected assets.
///
/// Returns `{job_id}` immediately. Progress flows through the generic
/// `/api/jobs/{id}/progress` SSE stream; the final terminal event carries
/// `{embedded, skipped, errors, done: true}`.
pub async fn batch_embed(
    State(state): State<Arc<AppState>>,
    Json(req): Json<BatchEmbedRequest>,
) -> Response {
    spawn_embed_job(state, req.asset_ids).await
}

/// POST /api/asset/{id}/embed — generate the SigLIP embedding for one asset.
///
/// Synchronous: a single image is fast (a few hundred ms) and the asset
/// detail UI expects inline counts in the response. Batch operations go
/// through the job registry instead.
pub async fn embed_asset(
    State(state): State<Arc<AppState>>,
    Path(asset_id): Path<String>,
) -> Response {
    let model_dir = super::resolve_model_dir(&state.ai_config);
    let model_id = state.ai_config.model.clone();
    let exec = state.ai_config.execution_provider.clone();
    let mgr = match crate::model_manager::ModelManager::new(&model_dir, &model_id) {
        Ok(m) => m,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")).into_response(),
    };
    if !mgr.model_exists() {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Model '{model_id}' is not downloaded."),
        )
            .into_response();
    }

    let state2 = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let service = state2.asset_service();
        service.embed_assets(
            &[asset_id],
            &model_dir,
            &model_id,
            &exec,
            false,
            |_, _, _| {},
        )
    })
    .await;

    match result {
        Ok(Ok(r)) => Json(serde_json::json!({
            "embedded": r.embedded,
            "skipped": r.skipped,
            "errors": r.errors,
        }))
        .into_response(),
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
    }
}

async fn spawn_embed_job(state: Arc<AppState>, asset_ids: Vec<String>) -> Response {
    use crate::web::jobs::JobKind;

    // Pre-flight: bail early if the model isn't on disk. The HTTP client gets
    // a 500 with a clear message rather than a job that immediately fails.
    let model_dir = super::resolve_model_dir(&state.ai_config);
    let model_id = state.ai_config.model.clone();
    let exec_provider = state.ai_config.execution_provider.clone();
    let mgr = match crate::model_manager::ModelManager::new(&model_dir, &model_id) {
        Ok(m) => m,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")).into_response(),
    };
    if !mgr.model_exists() {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Model '{model_id}' is not downloaded. Run 'maki auto-tag --download' first."),
        )
            .into_response();
    }

    let job = state.jobs.start(JobKind::Embed);
    let job_id = job.id.clone();
    let total = asset_ids.len();
    job.emit(&serde_json::json!({
        "phase": "embed",
        "done": false,
        "processed": 0,
        "total": total,
        "status": "starting",
    }));

    let state2 = state.clone();
    let job_for_task = job.clone();
    tokio::spawn(async move {
        let job_inner = job_for_task.clone();
        let service = state2.asset_service();
        let log = state2.log_requests;
        let processed = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let processed_for_cb = processed.clone();
        let job_for_cb = job_inner.clone();

        let result = tokio::task::spawn_blocking(move || {
            service.embed_assets(
                &asset_ids,
                &model_dir,
                &model_id,
                &exec_provider,
                false,
                move |aid, status, _elapsed| {
                    use std::sync::atomic::Ordering::Relaxed;
                    let n = processed_for_cb.fetch_add(1, Relaxed) + 1;
                    let label = match status {
                        crate::asset_service::EmbedStatus::Embedded => "embedded",
                        crate::asset_service::EmbedStatus::Skipped(_) => "skipped",
                        crate::asset_service::EmbedStatus::Error(_) => "error",
                    };
                    let short = &aid[..8.min(aid.len())];
                    job_for_cb.emit(&serde_json::json!({
                        "phase": "embed",
                        "done": false,
                        "processed": n,
                        "total": total,
                        "status": label,
                        "asset": short,
                    }));
                },
            )
        })
        .await;

        let terminal = match result {
            Ok(Ok(r)) => {
                if log {
                    eprintln!(
                        "batch_embed: {} assets ({} embedded, {} skipped, {} errors)",
                        total, r.embedded, r.skipped, r.errors.len()
                    );
                }
                serde_json::json!({
                    "phase": "embed",
                    "embedded": r.embedded,
                    "skipped": r.skipped,
                    "errors": r.errors,
                })
            }
            Ok(Err(e)) => serde_json::json!({"phase": "embed", "error": format!("{e:#}")}),
            Err(e) => serde_json::json!({"phase": "embed", "error": format!("{e}")}),
        };
        job_for_task.finish(terminal);
        state2.jobs.mark_done(&job_for_task.id);
    });

    Json(serde_json::json!({"job_id": job_id, "status": "started"})).into_response()
}
