//! "Maintain" dialog endpoints: long-running maintenance operations launched
//! from the Maintain tabbed modal in the web UI. Each handler spawns a
//! background job, returns `{job_id}` immediately, and streams per-file
//! progress through the generic `JobRegistry` SSE pipeline so the toast and
//! re-attach flows work identically to the import dialog.
//!
//! Three operations are exposed as parallel `POST /api/maintain/<op>` routes:
//!
//! - `writeback`     — flush pending XMP edits (or all, with `--mirror-tags`)
//! - `sync-metadata` — bidirectional XMP ↔ catalog sync with conflict report
//! - `verify`        — content-hash check for media + recipes on disk
//!
//! Concurrency: at most one job per kind at a time (matches the import
//! pattern). Re-running the same kind while one is in flight returns 409.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::time::Duration;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;

use super::super::AppState;
use crate::web::jobs::JobKind;

// ════════════════════════════════════════════════════════════════════════
// Writeback
// ════════════════════════════════════════════════════════════════════════

#[derive(Debug, serde::Deserialize)]
pub struct StartWritebackRequest {
    /// Search query (same syntax as `maki search`). Resolved to an asset ID set.
    pub query: Option<String>,
    /// Volume label.
    pub volume: Option<String>,
    /// `--all` (writes every XMP in the matching set, not just pending). Required when `mirror_tags` is true.
    #[serde(default)]
    pub all: bool,
    /// `--mirror-tags` (reconcile XMP keyword lists against catalog). Requires `all`.
    #[serde(default)]
    pub mirror_tags: bool,
    /// `--dry-run` (preview without modifying files).
    #[serde(default)]
    pub dry_run: bool,
}

/// POST /api/maintain/writeback — launch a writeback job.
pub async fn start_writeback_api(
    State(state): State<Arc<AppState>>,
    Json(req): Json<StartWritebackRequest>,
) -> Response {
    if req.mirror_tags && !req.all {
        return (StatusCode::BAD_REQUEST, "--mirror-tags requires --all").into_response();
    }

    if let Some(latest) = state.jobs.latest(JobKind::Writeback) {
        if !latest.is_completed() {
            return (StatusCode::CONFLICT, "A writeback is already running").into_response();
        }
    }

    let job = state.jobs.start(JobKind::Writeback);
    let job_id = job.id.clone();
    let state2 = state.clone();
    let job_for_task = job.clone();

    tokio::spawn(async move {
        let state3 = state2.clone();
        let job_inner = job_for_task.clone();
        let result = tokio::task::spawn_blocking(move || {
            run_writeback(&state3, &req, &job_inner)
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

fn run_writeback(
    state: &AppState,
    req: &StartWritebackRequest,
    job: &Arc<crate::web::jobs::Job>,
) -> anyhow::Result<serde_json::Value> {
    let engine = state.query_engine();
    let asset_id_set = engine
        .resolve_scope(req.query.as_deref(), None, &[])
        .ok()
        .flatten();

    let counter = AtomicUsize::new(0);
    let job_cb = job.clone();
    let callback = move |_asset_id: &str, status: &str| {
        let n = counter.fetch_add(1, Relaxed) + 1;
        job_cb.emit(&serde_json::json!({
            "processed": n,
            "status": status,
        }));
    };

    let result = engine.writeback(
        req.volume.as_deref(),
        None,
        asset_id_set.as_ref(),
        req.all,
        req.mirror_tags,
        req.dry_run,
        false,
        Some(&callback),
    )?;

    Ok(serde_json::json!({
        "written": result.written,
        "skipped": result.skipped,
        "failed": result.failed,
        "errors": result.errors,
        "dry_run": result.dry_run,
    }))
}

// ════════════════════════════════════════════════════════════════════════
// Sync-metadata
// ════════════════════════════════════════════════════════════════════════

#[derive(Debug, serde::Deserialize)]
pub struct StartSyncMetadataRequest {
    /// Volume label (optional — without it, every online volume is processed).
    pub volume: Option<String>,
    /// Re-extract embedded XMP from media files (slow). Maps to `--media`.
    #[serde(default)]
    pub media: bool,
    /// Preview without writing.
    #[serde(default)]
    pub dry_run: bool,
}

/// POST /api/maintain/sync-metadata — launch a sync-metadata job.
pub async fn start_sync_metadata_api(
    State(state): State<Arc<AppState>>,
    Json(req): Json<StartSyncMetadataRequest>,
) -> Response {
    if let Some(latest) = state.jobs.latest(JobKind::SyncMetadata) {
        if !latest.is_completed() {
            return (StatusCode::CONFLICT, "A sync-metadata job is already running").into_response();
        }
    }

    let job = state.jobs.start(JobKind::SyncMetadata);
    let job_id = job.id.clone();
    let state2 = state.clone();
    let job_for_task = job.clone();

    tokio::spawn(async move {
        let state3 = state2.clone();
        let job_inner = job_for_task.clone();
        let result = tokio::task::spawn_blocking(move || {
            run_sync_metadata(&state3, &req, &job_inner)
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

fn run_sync_metadata(
    state: &AppState,
    req: &StartSyncMetadataRequest,
    job: &Arc<crate::web::jobs::Job>,
) -> anyhow::Result<serde_json::Value> {
    let service = state.asset_service();

    // Resolve the volume label to the actual `Volume` if given. Sync-metadata's
    // engine method takes `Option<&Volume>` (not a label string).
    let registry = crate::device_registry::DeviceRegistry::new(&state.catalog_root);
    let volume_obj = match req.volume.as_deref() {
        Some(label) if !label.is_empty() => {
            let volumes = registry.list()?;
            let v = volumes.iter()
                .find(|v| v.label == label)
                .ok_or_else(|| anyhow::anyhow!("unknown volume: {label}"))?
                .clone();
            Some(v)
        }
        _ => None,
    };

    let counter = AtomicUsize::new(0);
    let job_cb = job.clone();
    let on_file = move |path: &std::path::Path, status: crate::asset_service::SyncMetadataStatus, _: Duration| {
        let n = counter.fetch_add(1, Relaxed) + 1;
        let status_str = match status {
            crate::asset_service::SyncMetadataStatus::Inbound => "inbound",
            crate::asset_service::SyncMetadataStatus::Outbound => "outbound",
            crate::asset_service::SyncMetadataStatus::Unchanged => "unchanged",
            crate::asset_service::SyncMetadataStatus::Missing => "missing",
            crate::asset_service::SyncMetadataStatus::Offline => "offline",
            crate::asset_service::SyncMetadataStatus::Conflict => "conflict",
            crate::asset_service::SyncMetadataStatus::Error => "error",
        };
        let asset = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        job_cb.emit(&serde_json::json!({
            "processed": n,
            "status": status_str,
            "asset": asset,
        }));
    };

    let result = service.sync_metadata(
        volume_obj.as_ref(),
        None,           // asset_id (not exposed in dialog)
        req.dry_run,
        req.media,
        &[],            // exclude_patterns
        on_file,
    )?;

    Ok(serde_json::json!({
        "inbound": result.inbound,
        "outbound": result.outbound,
        "unchanged": result.unchanged,
        "skipped": result.skipped,
        "conflicts": result.conflicts,
        "media_refreshed": result.media_refreshed,
        "errors": result.errors,
        "dry_run": result.dry_run,
    }))
}

// ════════════════════════════════════════════════════════════════════════
// Verify
// ════════════════════════════════════════════════════════════════════════

#[derive(Debug, serde::Deserialize)]
pub struct StartVerifyRequest {
    /// Volume label.
    pub volume: Option<String>,
    /// Skip files verified within this many days. Mirrors `[verify] max_age_days`.
    pub max_age_days: Option<u64>,
}

/// POST /api/maintain/verify — launch a verify job.
pub async fn start_verify_api(
    State(state): State<Arc<AppState>>,
    Json(req): Json<StartVerifyRequest>,
) -> Response {
    if let Some(latest) = state.jobs.latest(JobKind::Verify) {
        if !latest.is_completed() {
            return (StatusCode::CONFLICT, "A verify job is already running").into_response();
        }
    }

    let job = state.jobs.start(JobKind::Verify);
    let job_id = job.id.clone();
    let state2 = state.clone();
    let job_for_task = job.clone();

    tokio::spawn(async move {
        let state3 = state2.clone();
        let job_inner = job_for_task.clone();
        let result = tokio::task::spawn_blocking(move || {
            run_verify(&state3, &req, &job_inner)
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

fn run_verify(
    state: &AppState,
    req: &StartVerifyRequest,
    job: &Arc<crate::web::jobs::Job>,
) -> anyhow::Result<serde_json::Value> {
    let service = state.asset_service();

    let counter = AtomicUsize::new(0);
    let job_cb = job.clone();
    let on_file = move |path: &std::path::Path, status: crate::asset_service::VerifyStatus, _: Duration| {
        let n = counter.fetch_add(1, Relaxed) + 1;
        let status_str = match status {
            crate::asset_service::VerifyStatus::Ok => "ok",
            crate::asset_service::VerifyStatus::Mismatch => "mismatch",
            crate::asset_service::VerifyStatus::Modified => "modified",
            crate::asset_service::VerifyStatus::Missing => "missing",
            crate::asset_service::VerifyStatus::Skipped => "skipped",
            crate::asset_service::VerifyStatus::SkippedRecent => "skipped (recent)",
            crate::asset_service::VerifyStatus::Untracked => "untracked",
        };
        let asset = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        job_cb.emit(&serde_json::json!({
            "processed": n,
            "status": status_str,
            "asset": asset,
        }));
    };

    let paths: Vec<PathBuf> = Vec::new();
    let filter = crate::asset_service::FileTypeFilter::default();
    let result = service.verify(
        &paths,
        req.volume.as_deref(),
        None,
        &filter,
        req.max_age_days,
        on_file,
    )?;

    Ok(serde_json::json!({
        "verified": result.verified,
        "failed": result.failed,
        "modified": result.modified,
        "skipped": result.skipped,
        "skipped_recent": result.skipped_recent,
        "errors": result.errors,
    }))
}
