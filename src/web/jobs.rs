//! Generic background-job registry shared by long-running web operations.
//!
//! Every long-running operation (import, embed, auto-tag, detect-faces, describe,
//! …) follows the same shape: register a job, run it on a background task, push
//! per-item progress events to subscribers via SSE, let clients re-attach after
//! a page reload, and report aggregate progress to a nav badge poller. Doing
//! that per-route would mean repeating the same ~100 LOC of broadcast channel,
//! ring buffer, atomic counters, and lifecycle plumbing five times.
//!
//! `JobRegistry` lifts the machinery to one place. Each route still owns its
//! work loop and the per-kind details of what "progress" means; the registry
//! provides:
//!
//! - **Identity**: a job ID and start timestamp.
//! - **Pub/sub**: a broadcast channel; live SSE clients subscribe to it.
//! - **Re-attach**: a small ring buffer of recent events replayed on connect.
//! - **Snapshot**: a JSON value the route updates per tick; `/api/jobs` and
//!   `/api/jobs/{id}` serve it directly so badge / status UIs work without
//!   subscribing to SSE.
//!
//! The registry is *generic by JSON, not by trait objects* — each route picks
//! the JSON shape that suits it. Clients that need typed access (the import
//! dialog, the nav badge) read the fields they care about and ignore the rest.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

use serde::Serialize;

/// What kind of work a job is performing.
///
/// Used by the nav badge to label running activity ("Importing…",
/// "Embedding…") and by `/api/jobs` consumers that want to filter or count
/// jobs by category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JobKind {
    Import,
    Embed,
    AutoTag,
    DetectFaces,
    Describe,
    Writeback,
    SyncMetadata,
    Verify,
}

impl JobKind {
    pub fn label(self) -> &'static str {
        match self {
            JobKind::Import => "Import",
            JobKind::Embed => "Embed",
            JobKind::AutoTag => "Auto-tag",
            JobKind::DetectFaces => "Detect faces",
            JobKind::Describe => "Describe",
            JobKind::Writeback => "Writeback",
            JobKind::SyncMetadata => "Sync metadata",
            JobKind::Verify => "Verify",
        }
    }
}

/// Maximum number of recent events kept in each job's ring buffer for SSE
/// re-attachment. Sized so a UI re-connecting mid-job sees the last few seconds
/// of activity (typical job emits ~10–100 events/sec depending on workload).
pub const RECENT_EVENTS_CAP: usize = 100;

/// Maximum number of completed jobs kept in the registry after they finish.
/// Lets clients fetch the final state of a recently-finished job after page
/// reload (otherwise the result phase would be unreachable for re-attached UI).
const COMPLETED_HISTORY_CAP: usize = 16;

/// A single in-flight or recently-completed background job.
///
/// All fields are interior-mutable (broadcast sender, mutexes, atomic-equivalent
/// values) so handles can be cloned freely between the owning task, the SSE
/// reader, and the status endpoint.
pub struct Job {
    pub id: String,
    pub kind: JobKind,
    pub started_at: chrono::DateTime<chrono::Utc>,
    /// Broadcast sender for live SSE events. Subscribers receive each event as
    /// a JSON string (already serialised by the producer).
    pub sender: tokio::sync::broadcast::Sender<String>,
    /// Last `RECENT_EVENTS_CAP` events emitted, for SSE re-attach replay.
    /// Each entry is a JSON string ready to be re-broadcast.
    pub recent_events: Mutex<VecDeque<String>>,
    /// Most-recent progress snapshot. The status endpoint serves this directly,
    /// so the badge / re-attached UI sees current totals without subscribing
    /// to the SSE stream. Routes update it on each meaningful tick.
    pub progress: Mutex<serde_json::Value>,
    /// `false` while the job is running; `true` once it has emitted its
    /// terminal event. Completed jobs stay in the registry for a short window
    /// (see `COMPLETED_HISTORY_CAP`) so re-attached clients still see the
    /// final state — without that, a page reload mid-completion would lose it.
    pub completed: std::sync::atomic::AtomicBool,
}

impl Job {
    /// Push an event to every live SSE subscriber and into the replay ring.
    /// Also updates `progress` so polling clients see the same snapshot.
    pub fn emit(&self, event: &serde_json::Value) {
        let s = event.to_string();
        if let Ok(mut buf) = self.recent_events.lock() {
            if buf.len() >= RECENT_EVENTS_CAP {
                buf.pop_front();
            }
            buf.push_back(s.clone());
        }
        if let Ok(mut p) = self.progress.lock() {
            *p = event.clone();
        }
        let _ = self.sender.send(s);
    }

    /// Update the progress snapshot without emitting an SSE event. Useful for
    /// background ticks the SSE clients don't need to see (e.g. heartbeats).
    #[allow(dead_code)]
    pub fn update_progress(&self, snapshot: serde_json::Value) {
        if let Ok(mut p) = self.progress.lock() {
            *p = snapshot;
        }
    }

    /// Mark the job complete and emit a final `done: true` event with the
    /// supplied terminal payload (counts, errors, etc.).
    pub fn finish(&self, terminal: serde_json::Value) {
        self.completed
            .store(true, std::sync::atomic::Ordering::Release);
        // The terminal event always carries `done: true` regardless of the
        // payload the caller passed — clients close their EventSource on it.
        let mut payload = terminal;
        if let Some(obj) = payload.as_object_mut() {
            obj.insert("done".to_string(), serde_json::json!(true));
        }
        self.emit(&payload);
    }

    pub fn is_completed(&self) -> bool {
        self.completed.load(std::sync::atomic::Ordering::Acquire)
    }
}

/// Registry of running and recently-completed jobs.
///
/// Holds every job in a single `HashMap<JobId, Arc<Job>>` so any handler can
/// look up a job by ID. Multiple jobs of the same kind can run concurrently —
/// the registry imposes no per-kind exclusivity. Routes that want
/// "at-most-one" semantics enforce it themselves before calling `start`.
pub struct JobRegistry {
    inner: Mutex<JobRegistryInner>,
}

struct JobRegistryInner {
    /// All jobs, keyed by job ID.
    jobs: HashMap<String, std::sync::Arc<Job>>,
    /// IDs of completed jobs in the order they finished. Bounded by
    /// `COMPLETED_HISTORY_CAP`; oldest entries are evicted when full.
    completed_order: VecDeque<String>,
}

impl JobRegistry {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(JobRegistryInner {
                jobs: HashMap::new(),
                completed_order: VecDeque::with_capacity(COMPLETED_HISTORY_CAP),
            }),
        }
    }

    /// Register a new job and return a handle. The caller is responsible for
    /// spawning the work and calling `Job::emit` / `Job::finish` as it runs.
    pub fn start(&self, kind: JobKind) -> std::sync::Arc<Job> {
        let id = uuid::Uuid::new_v4().to_string();
        let (sender, _rx) = tokio::sync::broadcast::channel::<String>(512);
        let job = std::sync::Arc::new(Job {
            id: id.clone(),
            kind,
            started_at: chrono::Utc::now(),
            sender,
            recent_events: Mutex::new(VecDeque::with_capacity(RECENT_EVENTS_CAP)),
            progress: Mutex::new(serde_json::json!({})),
            completed: std::sync::atomic::AtomicBool::new(false),
        });
        let mut inner = self.inner.lock().unwrap();
        inner.jobs.insert(id, job.clone());
        job
    }

    pub fn get(&self, id: &str) -> Option<std::sync::Arc<Job>> {
        self.inner.lock().unwrap().jobs.get(id).cloned()
    }

    /// All currently-tracked jobs (running + recently completed), most-recently
    /// started first. Used by `GET /api/jobs` and the nav badge poller.
    pub fn list(&self) -> Vec<std::sync::Arc<Job>> {
        let inner = self.inner.lock().unwrap();
        let mut v: Vec<_> = inner.jobs.values().cloned().collect();
        // Newest first — tied to wall clock, monotonic enough for UI ordering.
        v.sort_by(|a, b| b.started_at.cmp(&a.started_at));
        v
    }

    /// Most recent job of the given kind (running or completed). Used by the
    /// import dialog to find "the import job" without tracking IDs across
    /// page reloads, and by re-attach flows that key on kind rather than ID.
    pub fn latest(&self, kind: JobKind) -> Option<std::sync::Arc<Job>> {
        let inner = self.inner.lock().unwrap();
        inner
            .jobs
            .values()
            .filter(|j| j.kind == kind)
            .max_by_key(|j| j.started_at)
            .cloned()
    }

    /// Move a job to the "completed" history. Called by the spawning task
    /// after `Job::finish`, once the SSE channel has drained the terminal
    /// event. Evicts the oldest completed job if `COMPLETED_HISTORY_CAP` is
    /// reached.
    pub fn mark_done(&self, id: &str) {
        let mut inner = self.inner.lock().unwrap();
        if !inner.jobs.contains_key(id) {
            return;
        }
        inner.completed_order.push_back(id.to_string());
        while inner.completed_order.len() > COMPLETED_HISTORY_CAP {
            if let Some(old) = inner.completed_order.pop_front() {
                inner.jobs.remove(&old);
            }
        }
    }

    /// Snapshot all jobs as JSON for the badge poller.
    /// Returns `{running: <count>, jobs: [...]}` where each entry includes
    /// id, kind, started_at, completed flag, and the most recent progress.
    pub fn snapshot(&self) -> serde_json::Value {
        let jobs = self.list();
        let running = jobs.iter().filter(|j| !j.is_completed()).count();
        let entries: Vec<serde_json::Value> = jobs
            .iter()
            .map(|j| {
                serde_json::json!({
                    "id": j.id,
                    "kind": j.kind,
                    "kind_label": j.kind.label(),
                    "started_at": j.started_at.to_rfc3339(),
                    "completed": j.is_completed(),
                    "progress": j.progress.lock()
                        .map(|p| p.clone())
                        .unwrap_or(serde_json::json!({})),
                })
            })
            .collect();
        serde_json::json!({
            "running": running,
            "jobs": entries,
        })
    }
}

impl Default for JobRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_creates_unique_ids() {
        let reg = JobRegistry::new();
        let a = reg.start(JobKind::Import);
        let b = reg.start(JobKind::Import);
        assert_ne!(a.id, b.id);
        assert_eq!(reg.list().len(), 2);
    }

    #[test]
    fn emit_updates_progress_and_ring() {
        let reg = JobRegistry::new();
        let job = reg.start(JobKind::Embed);
        let evt = serde_json::json!({"processed": 1, "total": 10});
        job.emit(&evt);
        let snap = job.progress.lock().unwrap();
        assert_eq!(snap["processed"], 1);
        let buf = job.recent_events.lock().unwrap();
        assert_eq!(buf.len(), 1);
    }

    #[test]
    fn ring_evicts_oldest_at_capacity() {
        let reg = JobRegistry::new();
        let job = reg.start(JobKind::Embed);
        for i in 0..(RECENT_EVENTS_CAP + 5) {
            job.emit(&serde_json::json!({"i": i}));
        }
        let buf = job.recent_events.lock().unwrap();
        assert_eq!(buf.len(), RECENT_EVENTS_CAP);
        // Oldest five should have been dropped — first remaining is i=5.
        let first: serde_json::Value = serde_json::from_str(&buf[0]).unwrap();
        assert_eq!(first["i"], 5);
    }

    #[test]
    fn finish_sets_completed_and_marks_done() {
        let reg = JobRegistry::new();
        let job = reg.start(JobKind::Embed);
        assert!(!job.is_completed());
        job.finish(serde_json::json!({"embedded": 5}));
        assert!(job.is_completed());
        let snap = job.progress.lock().unwrap();
        assert_eq!(snap["embedded"], 5);
        assert_eq!(snap["done"], true);
    }

    #[test]
    fn mark_done_evicts_oldest_when_history_full() {
        let reg = JobRegistry::new();
        let mut ids = Vec::new();
        for _ in 0..(COMPLETED_HISTORY_CAP + 3) {
            let j = reg.start(JobKind::Embed);
            j.finish(serde_json::json!({}));
            ids.push(j.id.clone());
            reg.mark_done(&j.id);
        }
        // Three oldest evicted — get() returns None for them.
        for id in &ids[..3] {
            assert!(reg.get(id).is_none(), "expected {} evicted", id);
        }
        // Most recent still present.
        for id in &ids[3..] {
            assert!(reg.get(id).is_some(), "expected {} retained", id);
        }
    }

    #[test]
    fn latest_picks_most_recent_of_kind() {
        let reg = JobRegistry::new();
        let _other = reg.start(JobKind::Import);
        std::thread::sleep(std::time::Duration::from_millis(2));
        let target = reg.start(JobKind::Embed);
        std::thread::sleep(std::time::Duration::from_millis(2));
        let _newer_other = reg.start(JobKind::Import);
        let found = reg.latest(JobKind::Embed).unwrap();
        assert_eq!(found.id, target.id);
    }

    #[test]
    fn snapshot_counts_running_only() {
        let reg = JobRegistry::new();
        let a = reg.start(JobKind::Import);
        let b = reg.start(JobKind::Embed);
        b.finish(serde_json::json!({}));
        let snap = reg.snapshot();
        assert_eq!(snap["running"], 1); // a still running, b completed
        assert_eq!(snap["jobs"].as_array().unwrap().len(), 2);
        let _ = a;
    }
}
