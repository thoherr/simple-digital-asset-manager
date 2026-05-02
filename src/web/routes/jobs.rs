//! Generic job-registry HTTP endpoints.
//!
//! Backs the nav badge poller and the SSE re-attach flow for every
//! long-running operation (import, embed, auto-tag, …). Per-kind start
//! endpoints live in their own modules; everything observed by the dialog UI
//! flows through here.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;

use super::super::AppState;

/// GET /api/jobs — snapshot of running and recently-completed jobs.
///
/// Used by the nav badge poller and the import-dialog re-attach probe.
/// Returns `{running: <count>, jobs: [{id, kind, kind_label, started_at,
/// completed, progress}]}`. Newest first.
pub async fn jobs_list_api(
    State(state): State<Arc<AppState>>,
) -> Response {
    Json(state.jobs.snapshot()).into_response()
}

/// GET /api/jobs/{id} — single job's current status snapshot.
///
/// Returns the same per-job shape as the entries in `/api/jobs`. 404 if no
/// job with this ID is in the registry (running or recently-completed).
pub async fn job_status_api(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    let job = match state.jobs.get(&id) {
        Some(j) => j,
        None => return (StatusCode::NOT_FOUND, "job not found").into_response(),
    };
    let progress = job.progress.lock()
        .map(|p| p.clone())
        .unwrap_or(serde_json::json!({}));
    Json(serde_json::json!({
        "id": job.id,
        "kind": job.kind,
        "kind_label": job.kind.label(),
        "started_at": job.started_at.to_rfc3339(),
        "completed": job.is_completed(),
        "progress": progress,
    }))
    .into_response()
}

/// GET /api/jobs/{id}/progress — SSE stream of per-job progress events.
///
/// On connect: replays the ring buffer (up to `RECENT_EVENTS_CAP` recent
/// events) so a re-attaching client sees what it missed, then continues with
/// the live broadcast. Subscribe-before-snapshot ordering means a producer
/// event landing in that window arrives via broadcast (a small chance of a
/// duplicate is preferable to a missed event for the user).
pub async fn job_progress_sse(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    use axum::response::sse::{Event, KeepAlive, Sse};
    use tokio_stream::StreamExt;

    let job = match state.jobs.get(&id) {
        Some(j) => j,
        None => return (StatusCode::NOT_FOUND, "job not found").into_response(),
    };

    // Subscribe first so any event emitted during snapshotting still reaches
    // us via broadcast.
    let rx = job.sender.subscribe();
    let snapshot: Vec<String> = job
        .recent_events
        .lock()
        .map(|buf| buf.iter().cloned().collect())
        .unwrap_or_default();

    let replay_stream = tokio_stream::iter(snapshot.into_iter().map(|d| Event::default().data(d)));
    let live_stream = tokio_stream::wrappers::BroadcastStream::new(rx)
        .filter_map(|msg| match msg {
            Ok(data) => Some(Event::default().data(data)),
            Err(_) => None,
        });
    let stream = replay_stream.chain(live_stream).map(Ok::<_, std::convert::Infallible>);

    Sse::new(stream).keep_alive(KeepAlive::default()).into_response()
}
