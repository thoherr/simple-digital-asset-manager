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

    // `[writeback] mirror_tags` in maki.toml turns mirror-tags on by
    // default for every writeback. The dialog's `--mirror-tags`
    // checkbox still takes effect on top (OR semantics).
    let cfg = crate::config::CatalogConfig::load(&state.catalog_root).unwrap_or_default();
    let effective_mirror_tags = req.mirror_tags || cfg.writeback.mirror_tags;

    let result = engine.writeback(
        req.volume.as_deref(),
        None,
        asset_id_set.as_ref(),
        req.all,
        effective_mirror_tags,
        req.dry_run,
        false,
        Some(&callback),
    )?;

    let skipped_offline: Vec<String> = result
        .skipped_offline_volumes
        .iter()
        .cloned()
        .collect();
    Ok(serde_json::json!({
        "written": result.written,
        "already_in_sync": result.already_in_sync,
        "skipped": result.skipped,
        "failed": result.failed,
        "errors": result.errors,
        "dry_run": result.dry_run,
        "skipped_offline_volumes": skipped_offline,
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

// ════════════════════════════════════════════════════════════════════════
// Generate previews
// ════════════════════════════════════════════════════════════════════════

#[derive(Debug, serde::Deserialize)]
pub struct StartGeneratePreviewsRequest {
    pub volume: Option<String>,
    /// Asset ID prefix or full ID — restricts to a single asset. Mirrors `--asset` on the CLI.
    pub asset: Option<String>,
    /// Also generate the 2560px smart preview alongside the regular thumbnail.
    #[serde(default)]
    pub smart: bool,
    /// Regenerate every preview, even ones that already exist.
    #[serde(default)]
    pub force: bool,
    /// Skip assets where the best-preview variant is already at index 0
    /// (i.e. nothing changed since previous generation). Useful after a
    /// `fix-roles --apply` that moved a different variant into the best
    /// slot — only those assets need a new preview.
    #[serde(default)]
    pub upgrade: bool,
}

/// POST /api/maintain/generate-previews — kick off a preview-regeneration job.
pub async fn start_generate_previews_api(
    State(state): State<Arc<AppState>>,
    Json(req): Json<StartGeneratePreviewsRequest>,
) -> Response {
    if let Some(latest) = state.jobs.latest(JobKind::GeneratePreviews) {
        if !latest.is_completed() {
            return (StatusCode::CONFLICT, "A generate-previews job is already running").into_response();
        }
    }

    let job = state.jobs.start(JobKind::GeneratePreviews);
    let job_id = job.id.clone();
    let state2 = state.clone();
    let job_for_task = job.clone();

    tokio::spawn(async move {
        let state3 = state2.clone();
        let job_inner = job_for_task.clone();
        let result = tokio::task::spawn_blocking(move || {
            run_generate_previews(&state3, &req, &job_inner)
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

/// Catalog-mode preview generation: walks every asset (or one volume's
/// worth, or one specific asset) and regenerates the missing/forced/upgrade
/// previews. The CLI's PATHS-mode (regenerate previews for specific files
/// just touched on disk) stays CLI-only — the web user picks scope from
/// the catalog rather than from the filesystem.
fn run_generate_previews(
    state: &AppState,
    req: &StartGeneratePreviewsRequest,
    job: &Arc<crate::web::jobs::Job>,
) -> anyhow::Result<serde_json::Value> {
    let (catalog_root, config) = crate::config::load_config()?;
    let preview_gen = crate::preview::PreviewGenerator::new(
        &catalog_root,
        crate::Verbosity::new(false, false),
        &config.preview,
    );
    let metadata_store = crate::metadata_store::MetadataStore::new(&catalog_root);
    let registry = crate::device_registry::DeviceRegistry::new(&catalog_root);
    let volumes = registry.list()?;

    let volume_filter = match req.volume.as_deref() {
        Some(label) if !label.is_empty() => Some(
            volumes
                .iter()
                .find(|v| v.label == label)
                .ok_or_else(|| anyhow::anyhow!("unknown volume: {label}"))?
                .clone(),
        ),
        _ => None,
    };

    // Collect asset IDs to process.
    let asset_ids: Vec<uuid::Uuid> = if let Some(asset_str) = req.asset.as_deref() {
        let engine = state.query_engine();
        let details = engine.show(asset_str)?;
        vec![details.id.parse()?]
    } else {
        metadata_store.list()?.into_iter().map(|s| s.id).collect()
    };

    let mut generated: usize = 0;
    let mut skipped: usize = 0;
    let mut failed: usize = 0;
    let mut upgraded: usize = 0;
    let mut errors: Vec<String> = Vec::new();
    let mut offline_blockers: std::collections::HashSet<String> = std::collections::HashSet::new();

    let total = asset_ids.len();
    for (idx, aid) in asset_ids.iter().enumerate() {
        let asset_data = match metadata_store.load(*aid) {
            Ok(a) => a,
            Err(_) => { skipped += 1; continue; }
        };

        // Pick the asset's best-preview variant (same selection as CLI).
        let pidx = asset_data
            .preview_variant
            .as_ref()
            .and_then(|h| asset_data.variants.iter().position(|v| &v.content_hash == h))
            .or_else(|| crate::models::variant::best_preview_index(&asset_data.variants))
            .unwrap_or(0);
        let variant = match asset_data.variants.get(pidx) {
            Some(v) => v,
            None => { skipped += 1; continue; }
        };

        if req.upgrade && pidx == 0 { skipped += 1; continue; }

        // Find a reachable file for this variant (respecting volume filter if any).
        let source_path = variant.locations.iter().find_map(|loc| {
            if let Some(ref vf) = volume_filter {
                if loc.volume_id != vf.id { return None; }
            }
            volumes.iter().find_map(|v| {
                if v.id == loc.volume_id && v.is_online {
                    let full = v.mount_point.join(&loc.relative_path);
                    if full.exists() { Some(full) } else { None }
                } else { None }
            })
        });

        if source_path.is_none() {
            for loc in &variant.locations {
                if let Some(v) = volumes.iter().find(|v| v.id == loc.volume_id) {
                    if !v.is_online { offline_blockers.insert(v.label.clone()); }
                }
            }
            skipped += 1;
            continue;
        }
        let path = source_path.unwrap();

        let rotation = asset_data.preview_rotation;
        let result = if req.force || req.upgrade {
            preview_gen.regenerate_with_rotation(&variant.content_hash, &path, &variant.format, rotation)
        } else {
            preview_gen.generate(&variant.content_hash, &path, &variant.format)
        };
        if req.smart {
            let _ = if req.force || req.upgrade {
                preview_gen.regenerate_smart_with_rotation(&variant.content_hash, &path, &variant.format, rotation)
            } else {
                preview_gen.generate_smart(&variant.content_hash, &path, &variant.format)
            };
        }
        match result {
            Ok(Some(_)) => {
                generated += 1;
                if req.upgrade { upgraded += 1; }
            }
            Ok(None) => skipped += 1,
            Err(e) => { failed += 1; errors.push(format!("{}: {e:#}", path.display())); }
        }

        let asset_short = aid.to_string()[..8.min(aid.to_string().len())].to_string();
        job.emit(&serde_json::json!({
            "processed": idx + 1,
            "total": total,
            "asset": asset_short,
        }));
    }

    Ok(serde_json::json!({
        "generated": generated,
        "skipped": skipped,
        "failed": failed,
        "upgraded": upgraded,
        "errors": errors,
        "offline_blockers": offline_blockers.into_iter().collect::<Vec<_>>(),
    }))
}

// ════════════════════════════════════════════════════════════════════════
// Sync (file-layer reconciliation)
// ════════════════════════════════════════════════════════════════════════

#[derive(Debug, serde::Deserialize)]
pub struct StartSyncRequest {
    /// Volume label (required for sync — the engine method takes a `Volume`,
    /// and the natural "scan the whole catalog" web mode is "scan this volume").
    pub volume: String,
    /// Optional volume-relative subpath to scope the scan (e.g. "2024/wedding").
    /// Without it, the whole volume is scanned. Joined with the volume's mount
    /// point on the server before being passed to the engine.
    pub path: Option<String>,
    /// Apply changes (move locations, mark missing). Without this, dry-run.
    #[serde(default)]
    pub apply: bool,
    /// Drop catalog locations for files confirmed missing. Requires `apply`.
    #[serde(default)]
    pub remove_stale: bool,
}

/// POST /api/maintain/sync — kick off a sync job for one volume.
pub async fn start_sync_api(
    State(state): State<Arc<AppState>>,
    Json(req): Json<StartSyncRequest>,
) -> Response {
    if req.remove_stale && !req.apply {
        return (StatusCode::BAD_REQUEST, "remove_stale requires apply").into_response();
    }

    if let Some(latest) = state.jobs.latest(JobKind::Sync) {
        if !latest.is_completed() {
            return (StatusCode::CONFLICT, "A sync job is already running").into_response();
        }
    }

    let job = state.jobs.start(JobKind::Sync);
    let job_id = job.id.clone();
    let state2 = state.clone();
    let job_for_task = job.clone();

    tokio::spawn(async move {
        let state3 = state2.clone();
        let job_inner = job_for_task.clone();
        let result = tokio::task::spawn_blocking(move || {
            run_sync(&state3, &req, &job_inner)
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

fn run_sync(
    state: &AppState,
    req: &StartSyncRequest,
    job: &Arc<crate::web::jobs::Job>,
) -> anyhow::Result<serde_json::Value> {
    let service = state.asset_service();
    let registry = crate::device_registry::DeviceRegistry::new(&state.catalog_root);
    let volumes = registry.list()?;
    let volume = volumes
        .iter()
        .find(|v| v.label == req.volume)
        .ok_or_else(|| anyhow::anyhow!("unknown volume: {}", req.volume))?
        .clone();

    let counter = AtomicUsize::new(0);
    let job_cb = job.clone();
    let on_file = move |path: &std::path::Path, status: crate::asset_service::SyncStatus, _: Duration| {
        let n = counter.fetch_add(1, Relaxed) + 1;
        let status_str = match status {
            crate::asset_service::SyncStatus::Unchanged => "unchanged",
            crate::asset_service::SyncStatus::Moved => "moved",
            crate::asset_service::SyncStatus::New => "new",
            crate::asset_service::SyncStatus::Modified => "modified",
            crate::asset_service::SyncStatus::Missing => "missing",
        };
        let asset = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        job_cb.emit(&serde_json::json!({
            "processed": n,
            "status": status_str,
            "asset": asset,
        }));
    };

    // Sync's "paths" parameter scopes the scan. Default: the whole volume
    // (its mount point). With an explicit subpath we join — but reject any
    // value that escapes the mount via `..` so a malicious / mistyped value
    // can't sweep the user's home directory. The engine itself is volume-
    // scoped after this anyway, but the path is handed to a filesystem walk
    // before that, so the boundary check belongs here.
    let scan_path = match req.path.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(rel) => {
            let candidate = volume.mount_point.join(rel.trim_start_matches('/'));
            let canon_mount = std::fs::canonicalize(&volume.mount_point)
                .unwrap_or_else(|_| volume.mount_point.clone());
            let canon_candidate = std::fs::canonicalize(&candidate)
                .unwrap_or_else(|_| candidate.clone());
            if !canon_candidate.starts_with(&canon_mount) {
                anyhow::bail!("path escapes the volume mount: {}", rel);
            }
            canon_candidate
        }
        None => volume.mount_point.clone(),
    };
    let paths = vec![scan_path];
    let result = service.sync(&paths, &volume, req.apply, req.remove_stale, &[], on_file)?;

    Ok(serde_json::json!({
        "unchanged": result.unchanged,
        "moved": result.moved,
        "new_files": result.new_files,
        "modified": result.modified,
        "missing": result.missing,
        "stale_removed": result.stale_removed,
        "orphaned_cleaned": result.orphaned_cleaned,
        "locationless_after": result.locationless_after,
        "errors": result.errors,
    }))
}

// ════════════════════════════════════════════════════════════════════════
// Refresh (re-read sidecar / recipe metadata)
// ════════════════════════════════════════════════════════════════════════

#[derive(Debug, serde::Deserialize)]
pub struct StartRefreshRequest {
    pub volume: Option<String>,
    /// Re-extract embedded XMP from JPEG/TIFF files too (slow).
    #[serde(default)]
    pub media: bool,
    #[serde(default)]
    pub dry_run: bool,
}

/// POST /api/maintain/refresh — kick off a refresh job.
pub async fn start_refresh_api(
    State(state): State<Arc<AppState>>,
    Json(req): Json<StartRefreshRequest>,
) -> Response {
    if let Some(latest) = state.jobs.latest(JobKind::Refresh) {
        if !latest.is_completed() {
            return (StatusCode::CONFLICT, "A refresh job is already running").into_response();
        }
    }

    let job = state.jobs.start(JobKind::Refresh);
    let job_id = job.id.clone();
    let state2 = state.clone();
    let job_for_task = job.clone();

    tokio::spawn(async move {
        let state3 = state2.clone();
        let job_inner = job_for_task.clone();
        let result = tokio::task::spawn_blocking(move || {
            run_refresh(&state3, &req, &job_inner)
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

fn run_refresh(
    state: &AppState,
    req: &StartRefreshRequest,
    job: &Arc<crate::web::jobs::Job>,
) -> anyhow::Result<serde_json::Value> {
    let service = state.asset_service();
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
    let on_file = move |path: &std::path::Path, status: crate::asset_service::RefreshStatus, _: Duration| {
        let n = counter.fetch_add(1, Relaxed) + 1;
        let status_str = match status {
            crate::asset_service::RefreshStatus::Unchanged => "unchanged",
            crate::asset_service::RefreshStatus::Refreshed => "refreshed",
            crate::asset_service::RefreshStatus::Missing => "missing",
            crate::asset_service::RefreshStatus::Offline => "offline",
            crate::asset_service::RefreshStatus::SidecarPresent => "sidecar-present",
        };
        let asset = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        job_cb.emit(&serde_json::json!({
            "processed": n,
            "status": status_str,
            "asset": asset,
        }));
    };

    let paths: Vec<PathBuf> = Vec::new();
    let result = service.refresh(
        &paths,
        volume_obj.as_ref(),
        None,
        req.dry_run,
        req.media,
        &[],
        on_file,
    )?;

    Ok(serde_json::json!({
        "unchanged": result.unchanged,
        "refreshed": result.refreshed,
        "missing": result.missing,
        "skipped": result.skipped,
        "errors": result.errors,
        "dry_run": req.dry_run,
    }))
}

// ════════════════════════════════════════════════════════════════════════
// Cleanup (orphan removal)
// ════════════════════════════════════════════════════════════════════════

#[derive(Debug, serde::Deserialize)]
pub struct StartCleanupRequest {
    pub volume: Option<String>,
    /// Catalog-relative path prefix (forward slashes); restricts to that subtree.
    pub path: Option<String>,
    #[serde(default)]
    pub apply: bool,
}

/// POST /api/maintain/cleanup — kick off a cleanup job.
pub async fn start_cleanup_api(
    State(state): State<Arc<AppState>>,
    Json(req): Json<StartCleanupRequest>,
) -> Response {
    if let Some(latest) = state.jobs.latest(JobKind::Cleanup) {
        if !latest.is_completed() {
            return (StatusCode::CONFLICT, "A cleanup job is already running").into_response();
        }
    }

    let job = state.jobs.start(JobKind::Cleanup);
    let job_id = job.id.clone();
    let state2 = state.clone();
    let job_for_task = job.clone();

    tokio::spawn(async move {
        let state3 = state2.clone();
        let job_inner = job_for_task.clone();
        let result = tokio::task::spawn_blocking(move || {
            run_cleanup(&state3, &req, &job_inner)
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

fn run_cleanup(
    state: &AppState,
    req: &StartCleanupRequest,
    job: &Arc<crate::web::jobs::Job>,
) -> anyhow::Result<serde_json::Value> {
    let service = state.asset_service();

    let counter = AtomicUsize::new(0);
    let job_cb = job.clone();
    let on_file = move |path: &std::path::Path, status: crate::asset_service::CleanupStatus, _: Duration| {
        let n = counter.fetch_add(1, Relaxed) + 1;
        let status_str = match status {
            crate::asset_service::CleanupStatus::Ok => "ok",
            crate::asset_service::CleanupStatus::Stale => "stale",
            crate::asset_service::CleanupStatus::Offline => "offline",
            crate::asset_service::CleanupStatus::LocationlessVariant => "locationless",
            crate::asset_service::CleanupStatus::OrphanedAsset => "orphaned asset",
            crate::asset_service::CleanupStatus::OrphanedFile => "orphaned file",
        };
        let asset = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        job_cb.emit(&serde_json::json!({
            "processed": n,
            "status": status_str,
            "asset": asset,
        }));
    };

    let result = service.cleanup(
        req.volume.as_deref(),
        req.path.as_deref(),
        req.apply,
        on_file,
    )?;

    Ok(serde_json::json!({
        "checked": result.checked,
        "stale": result.stale,
        "removed": result.removed,
        "skipped_offline": result.skipped_offline,
        "locationless_variants": result.locationless_variants,
        "removed_variants": result.removed_variants,
        "orphaned_assets": result.orphaned_assets,
        "removed_assets": result.removed_assets,
        "orphaned_previews": result.orphaned_previews,
        "removed_previews": result.removed_previews,
        "orphaned_smart_previews": result.orphaned_smart_previews,
        "removed_smart_previews": result.removed_smart_previews,
        "orphaned_embeddings": result.orphaned_embeddings,
        "removed_embeddings": result.removed_embeddings,
        "orphaned_face_files": result.orphaned_face_files,
        "removed_face_files": result.removed_face_files,
        "errors": result.errors,
        "skipped_global_passes": result.skipped_global_passes,
        "applied": req.apply,
    }))
}
