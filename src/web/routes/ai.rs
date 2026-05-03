//! AI-gated route handlers: suggest-tags, auto-tag, find-similar, faces/people, stroll.
//!
//! All items are `#[cfg(feature = "ai")]` — this module is only compiled when the
//! `ai` feature is enabled. The parent `routes::mod` declares it behind the same
//! feature gate.

use std::sync::Arc;

use askama::Template;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use axum::Json;

use crate::web::templates::{
    CollectionOption, PeoplePage, PersonCard, PersonOption, StrollCenter, StrollNeighbor,
    StrollPage, TagOption, VolumeOption,
};
use crate::web::AppState;

use super::stats::build_format_groups;

// --- AI auto-tag endpoints ---

#[derive(Debug, serde::Serialize)]
pub struct SuggestTagsResponse {
    pub tag: String,
    pub confidence: f32,
    pub existing: bool,
}

/// POST /api/asset/{id}/suggest-tags — suggest tags for an asset using AI.
pub async fn suggest_tags(
    State(state): State<Arc<AppState>>,
    Path(asset_id): Path<String>,
) -> Response {
    let state = state.clone();
    let result: Result<Result<Vec<SuggestTagsResponse>, String>, _> =
        tokio::task::spawn_blocking(move || {
            suggest_tags_inner(&state, &asset_id)
        })
        .await;

    match result {
        Ok(Ok(suggestions)) => Json(suggestions).into_response(),
        Ok(Err(msg)) => (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
    }
}

pub(super) fn resolve_model_dir(config: &crate::config::AiConfig) -> std::path::PathBuf {
    crate::config::resolve_model_dir(&config.model_dir, &config.model)
}

fn resolve_labels(config: &crate::config::AiConfig) -> Result<Vec<String>, String> {
    if let Some(ref labels_path) = config.labels {
        crate::ai::load_labels_from_file(std::path::Path::new(labels_path))
            .map_err(|e| format!("Failed to load labels: {e}"))
    } else {
        Ok(crate::ai::DEFAULT_LABELS.iter().map(|s| s.to_string()).collect())
    }
}

fn suggest_tags_inner(
    state: &AppState,
    asset_id: &str,
) -> Result<Vec<SuggestTagsResponse>, String> {
    use crate::ai;
    use crate::device_registry::DeviceRegistry;

    let engine = state.query_engine();
    let details = engine.show(asset_id).map_err(|e| format!("{e:#}"))?;

    let preview_gen = state.preview_generator();
    let registry = DeviceRegistry::new(&state.catalog_root);
    let volumes = registry.list().map_err(|e| format!("{e:#}"))?;
    let online_volumes: std::collections::HashMap<String, &crate::models::Volume> = volumes
        .iter()
        .filter(|v| v.is_online)
        .map(|v| (v.id.to_string(), v))
        .collect();

    let service = state.asset_service();
    let image_path = service
        .find_image_for_ai(&details, &preview_gen, &online_volumes)
        .ok_or_else(|| "No processable image found for this asset".to_string())?;

    let ext = image_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    if !ai::is_supported_image(ext) {
        return Err(format!("Unsupported image format: {ext}"));
    }

    let model_dir = resolve_model_dir(&state.ai_config);
    let model_id = &state.ai_config.model;
    let model_guard = state.ai_model.blocking_lock();
    let mut model_opt = model_guard;
    if model_opt.is_none() {
        let m = ai::SigLipModel::load_with_provider(&model_dir, model_id, state.verbosity, &state.ai_config.execution_provider)
            .map_err(|e| format!("Failed to load AI model: {e:#}"))?;
        *model_opt = Some(m);
    }
    let model = model_opt.as_mut().unwrap();

    let labels = resolve_labels(&state.ai_config)?;
    let label_cache_read = state.ai_label_cache.blocking_read();
    let cached = label_cache_read.is_some();
    drop(label_cache_read);

    let (label_list, label_embs) = if cached {
        let guard = state.ai_label_cache.blocking_read();
        let (l, e) = guard.as_ref().unwrap();
        (l.clone(), e.clone())
    } else {
        let prompt_template = &state.ai_config.prompt;
        let prompted: Vec<String> = labels
            .iter()
            .map(|l| ai::apply_prompt_template(prompt_template, l))
            .collect();
        let embs = model
            .encode_texts(&prompted)
            .map_err(|e| format!("Failed to encode labels: {e:#}"))?;
        let mut guard = state.ai_label_cache.blocking_write();
        *guard = Some((labels.clone(), embs.clone()));
        (labels, embs)
    };

    let image_emb = model
        .encode_image(&image_path)
        .map_err(|e| format!("Failed to encode image: {e:#}"))?;

    {
        let catalog = crate::catalog::Catalog::open_fast(&state.catalog_root);
        if let Ok(catalog) = catalog {
            let _ = crate::embedding_store::EmbeddingStore::initialize(catalog.conn());
            let emb_store = crate::embedding_store::EmbeddingStore::new(catalog.conn());
            let _ = emb_store.store(asset_id, &image_emb, model_id);
        }
        let _ = crate::embedding_store::write_embedding_binary(&state.catalog_root, model_id, asset_id, &image_emb);
        if let Ok(mut idx_guard) = state.ai_embedding_index.write() {
            if let Some(ref mut idx) = *idx_guard {
                idx.upsert(asset_id, &image_emb);
            }
        }
    }

    let threshold = state.ai_config.threshold;
    let suggestions = model.classify(&image_emb, &label_list, &label_embs, threshold);

    let existing: std::collections::HashSet<String> = details
        .tags
        .iter()
        .map(|t| t.to_lowercase())
        .collect();

    let result: Vec<SuggestTagsResponse> = suggestions
        .into_iter()
        .map(|s| {
            let is_existing = existing.contains(&s.tag.to_lowercase());
            SuggestTagsResponse {
                tag: s.tag,
                confidence: s.confidence,
                existing: is_existing,
            }
        })
        .collect();

    Ok(result)
}

#[derive(Debug, serde::Deserialize)]
pub struct BatchAutoTagRequest {
    pub asset_ids: Vec<String>,
}

#[derive(Debug, serde::Serialize)]
pub struct BatchAutoTagResponse {
    pub succeeded: u32,
    pub failed: u32,
    pub tags_applied: u32,
    pub errors: Vec<String>,
}

/// POST /api/batch/auto-tag — start an auto-tag job for selected assets.
///
/// Returns `{job_id}` immediately. Progress flows through `/api/jobs/{id}/progress`;
/// the terminal event carries `{succeeded, failed, tags_applied, errors, done: true}`.
pub async fn batch_auto_tag(
    State(state): State<Arc<AppState>>,
    Json(req): Json<BatchAutoTagRequest>,
) -> Response {
    use crate::web::jobs::JobKind;

    let job = state.jobs.start(JobKind::AutoTag);
    let job_id = job.id.clone();
    let total = req.asset_ids.len();
    job.emit(&serde_json::json!({
        "phase": "auto_tag",
        "done": false,
        "processed": 0,
        "total": total,
        "status": "starting",
    }));

    let state2 = state.clone();
    let job_for_task = job.clone();
    tokio::spawn(async move {
        let log = state2.log_requests;
        let job_inner = job_for_task.clone();
        let state_for_blocking = state2.clone();
        let result = tokio::task::spawn_blocking(move || {
            batch_auto_tag_inner(&state_for_blocking, req.asset_ids, &job_inner, total)
        })
        .await;

        let terminal = match result {
            Ok(Ok(resp)) => {
                if log {
                    eprintln!(
                        "batch_auto_tag: {} assets ({} ok, {} err, {} tags)",
                        total, resp.succeeded, resp.failed, resp.tags_applied
                    );
                }
                if resp.succeeded > 0 {
                    state2.dropdown_cache.invalidate_tags();
                }
                serde_json::json!({
                    "phase": "auto_tag",
                    "succeeded": resp.succeeded,
                    "failed": resp.failed,
                    "tags_applied": resp.tags_applied,
                    "errors": resp.errors,
                })
            }
            Ok(Err(msg)) => serde_json::json!({"phase": "auto_tag", "error": msg}),
            Err(e) => serde_json::json!({"phase": "auto_tag", "error": format!("{e}")}),
        };
        job_for_task.finish(terminal);
        state2.jobs.mark_done(&job_for_task.id);
    });

    Json(serde_json::json!({"job_id": job_id, "status": "started"})).into_response()
}

fn batch_auto_tag_inner(
    state: &AppState,
    asset_ids: Vec<String>,
    job: &std::sync::Arc<crate::web::jobs::Job>,
    total: usize,
) -> Result<BatchAutoTagResponse, String> {
    use crate::ai;
    use crate::device_registry::DeviceRegistry;

    let engine = state.query_engine();
    let preview_gen = state.preview_generator();
    let registry = DeviceRegistry::new(&state.catalog_root);
    let volumes = registry.list().map_err(|e| format!("{e:#}"))?;
    let online_volumes: std::collections::HashMap<String, &crate::models::Volume> = volumes
        .iter()
        .filter(|v| v.is_online)
        .map(|v| (v.id.to_string(), v))
        .collect();

    let model_dir = resolve_model_dir(&state.ai_config);
    let model_id = &state.ai_config.model;
    let mut model_guard = state.ai_model.blocking_lock();
    if model_guard.is_none() {
        let m = ai::SigLipModel::load_with_provider(&model_dir, model_id, state.verbosity, &state.ai_config.execution_provider)
            .map_err(|e| format!("Failed to load AI model: {e:#}"))?;
        *model_guard = Some(m);
    }
    let model = model_guard.as_mut().unwrap();

    let labels = resolve_labels(&state.ai_config)?;
    let label_cache_read = state.ai_label_cache.blocking_read();
    let cached = label_cache_read.is_some();
    drop(label_cache_read);

    let (label_list, label_embs) = if cached {
        let guard = state.ai_label_cache.blocking_read();
        let (l, e) = guard.as_ref().unwrap();
        (l.clone(), e.clone())
    } else {
        let prompt_template = &state.ai_config.prompt;
        let prompted: Vec<String> = labels
            .iter()
            .map(|l| ai::apply_prompt_template(prompt_template, l))
            .collect();
        let embs = model
            .encode_texts(&prompted)
            .map_err(|e| format!("Failed to encode labels: {e:#}"))?;
        let mut guard = state.ai_label_cache.blocking_write();
        *guard = Some((labels.clone(), embs.clone()));
        (labels, embs)
    };

    let threshold = state.ai_config.threshold;
    let service = state.asset_service();
    let mut resp = BatchAutoTagResponse {
        succeeded: 0,
        failed: 0,
        tags_applied: 0,
        errors: Vec::new(),
    };

    let mut processed: usize = 0;
    let emit_progress = |processed: usize, aid: &str, status: &str, tags_applied: u32| {
        let short = &aid[..8.min(aid.len())];
        job.emit(&serde_json::json!({
            "phase": "auto_tag",
            "done": false,
            "processed": processed,
            "total": total,
            "asset": short,
            "status": status,
            "tags_applied": tags_applied,
        }));
    };

    for aid in &asset_ids {
        processed += 1;
        let details = match engine.show(aid) {
            Ok(d) => d,
            Err(e) => {
                resp.failed += 1;
                resp.errors.push(format!("{}: {e:#}", &aid[..8.min(aid.len())]));
                emit_progress(processed, aid, "error", resp.tags_applied);
                continue;
            }
        };

        let image_path = match service.find_image_for_ai(&details, &preview_gen, &online_volumes) {
            Some(p) => p,
            None => {
                emit_progress(processed, aid, "skipped", resp.tags_applied);
                continue;
            }
        };

        let ext = image_path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");
        if !ai::is_supported_image(ext) {
            emit_progress(processed, aid, "skipped", resp.tags_applied);
            continue;
        }

        let image_emb = match model.encode_image(&image_path) {
            Ok(emb) => emb,
            Err(e) => {
                resp.failed += 1;
                resp.errors.push(format!("{}: {e:#}", &aid[..8.min(aid.len())]));
                emit_progress(processed, aid, "error", resp.tags_applied);
                continue;
            }
        };

        {
            let cat = crate::catalog::Catalog::open_fast(&state.catalog_root);
            if let Ok(cat) = cat {
                let _ = crate::embedding_store::EmbeddingStore::initialize(cat.conn());
                let es = crate::embedding_store::EmbeddingStore::new(cat.conn());
                let _ = es.store(aid, &image_emb, model_id);
            }
            let _ = crate::embedding_store::write_embedding_binary(&state.catalog_root, model_id, aid, &image_emb);
            if let Ok(mut idx_guard) = state.ai_embedding_index.write() {
                if let Some(ref mut idx) = *idx_guard {
                    idx.upsert(aid, &image_emb);
                }
            }
        }

        let suggestions = model.classify(&image_emb, &label_list, &label_embs, threshold);

        let existing: std::collections::HashSet<String> = details
            .tags
            .iter()
            .map(|t| t.to_lowercase())
            .collect();

        let new_tags: Vec<String> = suggestions
            .into_iter()
            .filter(|s| !existing.contains(&s.tag.to_lowercase()))
            .map(|s| s.tag)
            .collect();

        if new_tags.is_empty() {
            resp.succeeded += 1;
            emit_progress(processed, aid, "no-new-tags", resp.tags_applied);
            continue;
        }

        match engine.tag(aid, &new_tags, false) {
            Ok(_) => {
                resp.tags_applied += new_tags.len() as u32;
                resp.succeeded += 1;
                emit_progress(processed, aid, "tagged", resp.tags_applied);
            }
            Err(e) => {
                resp.failed += 1;
                resp.errors.push(format!("{}: {e:#}", &aid[..8.min(aid.len())]));
                emit_progress(processed, aid, "error", resp.tags_applied);
            }
        }
    }

    Ok(resp)
}

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
    let model_dir = resolve_model_dir(&state.ai_config);
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
    let model_dir = resolve_model_dir(&state.ai_config);
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

// --- Visual similarity search endpoint ---

#[derive(Debug, serde::Serialize)]
pub struct SimilarAssetResponse {
    pub asset_id: String,
    pub similarity: f32,
    pub preview_url: Option<String>,
    pub name: String,
}

/// POST /api/asset/{id}/similar — find visually similar assets.
pub async fn find_similar(
    State(state): State<Arc<AppState>>,
    Path(asset_id): Path<String>,
) -> Response {
    let state = state.clone();
    let result: Result<Result<Vec<SimilarAssetResponse>, String>, _> =
        tokio::task::spawn_blocking(move || find_similar_inner(&state, &asset_id))
            .await;

    match result {
        Ok(Ok(results)) => Json(results).into_response(),
        Ok(Err(msg)) => (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
    }
}

fn find_similar_inner(
    state: &AppState,
    asset_id: &str,
) -> Result<Vec<SimilarAssetResponse>, String> {
    use crate::ai;
    use crate::device_registry::DeviceRegistry;
    use crate::embedding_store::EmbeddingStore;

    let model_id = &state.ai_config.model;
    let spec = crate::ai::get_model_spec(model_id)
        .ok_or_else(|| format!("Unknown model: {model_id}"))?;

    let catalog = state.catalog().map_err(|e| format!("{e:#}"))?;
    let _ = EmbeddingStore::initialize(catalog.conn());
    let emb_store = EmbeddingStore::new(catalog.conn());

    let stored_emb = emb_store.get(asset_id, model_id).map_err(|e| format!("{e:#}"))?;

    let query_emb = if let Some(emb) = stored_emb {
        emb
    } else {
        let engine = state.query_engine();
        let details = engine.show(asset_id).map_err(|e| format!("{e:#}"))?;

        let preview_gen = state.preview_generator();
        let registry = DeviceRegistry::new(&state.catalog_root);
        let volumes = registry.list().map_err(|e| format!("{e:#}"))?;
        let online_volumes: std::collections::HashMap<String, &crate::models::Volume> = volumes
            .iter()
            .filter(|v| v.is_online)
            .map(|v| (v.id.to_string(), v))
            .collect();

        let service = state.asset_service();
        let image_path = service
            .find_image_for_ai(&details, &preview_gen, &online_volumes)
            .ok_or_else(|| "No processable image found for this asset".to_string())?;

        let ext = image_path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");
        if !ai::is_supported_image(ext) {
            return Err(format!("Unsupported image format: {ext}"));
        }

        let model_dir = resolve_model_dir(&state.ai_config);
        let mut model_guard = state.ai_model.blocking_lock();
        if model_guard.is_none() {
            let m = ai::SigLipModel::load(&model_dir, model_id)
                .map_err(|e| format!("Failed to load AI model: {e:#}"))?;
            *model_guard = Some(m);
        }
        let model = model_guard.as_mut().unwrap();

        let emb = model
            .encode_image(&image_path)
            .map_err(|e| format!("Failed to encode image: {e:#}"))?;

        drop(model_guard);

        emb_store
            .store(asset_id, &emb, model_id)
            .map_err(|e| format!("Failed to store embedding: {e:#}"))?;
        let _ = crate::embedding_store::write_embedding_binary(&state.catalog_root, model_id, asset_id, &emb);
        emb
    };

    {
        let needs_load = state.ai_embedding_index.read().unwrap().is_none();
        if needs_load {
            let index = crate::embedding_store::EmbeddingIndex::load(
                catalog.conn(),
                model_id,
                spec.embedding_dim,
            ).map_err(|e| format!("Failed to load embedding index: {e:#}"))?;
            *state.ai_embedding_index.write().unwrap() = Some(index);
        }
    }

    {
        let mut idx_guard = state.ai_embedding_index.write().unwrap();
        if let Some(ref mut idx) = *idx_guard {
            idx.upsert(asset_id, &query_emb);
        }
    }

    let results = {
        let idx_guard = state.ai_embedding_index.read().unwrap();
        let idx = idx_guard.as_ref().unwrap();
        idx.search(&query_emb, 20, Some(asset_id))
    };

    let preview_gen = state.preview_generator();
    let preview_ext = &state.preview_ext;
    let response: Vec<SimilarAssetResponse> = results
        .into_iter()
        .filter_map(|(id, similarity)| {
            let cat = state.catalog().ok()?;
            let d = cat.load_asset_details(&id).ok()??;
            let name = d
                .name
                .clone()
                .unwrap_or_else(|| {
                    d.variants
                        .first()
                        .and_then(|v| {
                            v.locations
                                .first()
                                .map(|fl| {
                                    std::path::Path::new(&fl.relative_path)
                                        .file_name()
                                        .unwrap_or_default()
                                        .to_string_lossy()
                                        .to_string()
                                })
                        })
                        .unwrap_or_else(|| id[..8.min(id.len())].to_string())
                });

            let preview_url = {
                let best_idx = crate::models::variant::best_preview_index_details(&d.variants);
                best_idx.and_then(|i| {
                    let v = &d.variants[i];
                    if preview_gen.has_preview(&v.content_hash) {
                        Some(crate::web::templates::preview_url(&v.content_hash, preview_ext))
                    } else {
                        None
                    }
                })
            };

            Some(SimilarAssetResponse {
                asset_id: id,
                similarity,
                preview_url,
                name,
            })
        })
        .collect();

    Ok(response)
}

// --- Face recognition handlers ---

/// GET /api/asset/{id}/faces — list faces for an asset.
pub async fn asset_faces(
    State(state): State<Arc<AppState>>,
    Path(asset_id): Path<String>,
) -> Response {
    let result = tokio::task::spawn_blocking(move || {
        let catalog = state.catalog()?;
        let face_store = crate::face_store::FaceStore::new(catalog.conn());
        let faces = face_store.faces_for_asset(&asset_id)?;
        let result: Vec<serde_json::Value> = faces.iter().map(|f| {
            let crop_url = if crate::face::face_crop_exists(&f.id, &state.catalog_root) {
                Some(format!("/face/{}/{}.jpg", &f.id[..2.min(f.id.len())], f.id))
            } else {
                None
            };
            let person_name = f.person_id.as_ref().and_then(|pid| {
                face_store.get_person(pid).ok().flatten().and_then(|p| p.name)
            });
            serde_json::json!({
                "face_id": f.id,
                "confidence": f.confidence,
                "bbox": [f.bbox_x, f.bbox_y, f.bbox_w, f.bbox_h],
                "person_id": f.person_id,
                "person_name": person_name,
                "crop_url": crop_url,
            })
        }).collect();
        Ok::<_, anyhow::Error>(result)
    }).await;

    match result {
        Ok(Ok(faces)) => Json(faces).into_response(),
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e}")).into_response(),
    }
}

/// POST /api/asset/{id}/detect-faces — detect faces for a single asset.
pub async fn detect_faces_for_asset(
    State(state): State<Arc<AppState>>,
    Path(asset_id): Path<String>,
) -> Response {
    let state2 = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        detect_faces_inner(&state2, &[asset_id])
    }).await;

    match result {
        Ok(Ok(json)) => Json(json).into_response(),
        Ok(Err(msg)) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": msg}))).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": format!("{e}")}))).into_response(),
    }
}

/// POST /api/batch/detect-faces — start a face-detection job for selected assets.
///
/// Returns `{job_id}` immediately. Progress flows through `/api/jobs/{id}/progress`;
/// the terminal event carries `{succeeded, faces_detected, errors, done: true}`.
pub async fn batch_detect_faces(
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    use crate::web::jobs::JobKind;

    let asset_ids: Vec<String> = body.get("asset_ids")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    // Pre-flight: face models present?
    let face_model_dir = crate::face::resolve_face_model_dir(&state.ai_config);
    if !crate::face::FaceDetector::models_exist(&face_model_dir) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Face models not downloaded. Run 'maki faces download' first.",
        )
            .into_response();
    }

    let job = state.jobs.start(JobKind::DetectFaces);
    let job_id = job.id.clone();
    let total = asset_ids.len();
    job.emit(&serde_json::json!({
        "phase": "detect_faces",
        "done": false,
        "processed": 0,
        "total": total,
        "status": "starting",
    }));

    let state2 = state.clone();
    let job_for_task = job.clone();
    let exec_provider = state.ai_config.execution_provider.clone();
    let min_conf = state.ai_config.face_min_confidence;

    tokio::spawn(async move {
        let job_inner = job_for_task.clone();
        let processed = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let processed_for_cb = processed.clone();
        let job_for_cb = job_inner.clone();
        let state_for_blocking = state2.clone();

        let result = tokio::task::spawn_blocking(move || {
            let mut detector = match crate::face::FaceDetector::load_with_provider(
                &face_model_dir,
                state_for_blocking.verbosity,
                &exec_provider,
            ) {
                Ok(d) => d,
                Err(e) => return Err(format!("Failed to load face detector: {e:#}")),
            };
            let service = state_for_blocking.asset_service();
            service
                .detect_faces(
                    &asset_ids,
                    &mut detector,
                    min_conf,
                    true,
                    true,
                    move |aid, faces, _elapsed| {
                        use std::sync::atomic::Ordering::Relaxed;
                        let n = processed_for_cb.fetch_add(1, Relaxed) + 1;
                        let short = &aid[..8.min(aid.len())];
                        job_for_cb.emit(&serde_json::json!({
                            "phase": "detect_faces",
                            "done": false,
                            "processed": n,
                            "total": total,
                            "asset": short,
                            "faces": faces,
                        }));
                    },
                )
                .map_err(|e| format!("{e:#}"))
        })
        .await;

        let terminal = match result {
            Ok(Ok(r)) => serde_json::json!({
                "phase": "detect_faces",
                "succeeded": r.assets_processed,
                "faces_detected": r.faces_detected,
                "errors": r.errors,
            }),
            Ok(Err(e)) => serde_json::json!({"phase": "detect_faces", "error": e}),
            Err(e) => serde_json::json!({"phase": "detect_faces", "error": format!("{e}")}),
        };
        job_for_task.finish(terminal);
        state2.jobs.mark_done(&job_for_task.id);
    });

    Json(serde_json::json!({"job_id": job_id, "status": "started"})).into_response()
}

fn detect_faces_inner(state: &AppState, asset_ids: &[String]) -> Result<serde_json::Value, String> {
    let face_model_dir = crate::face::resolve_face_model_dir(&state.ai_config);
    if !crate::face::FaceDetector::models_exist(&face_model_dir) {
        return Err("Face models not downloaded. Run 'maki faces download' first.".to_string());
    }

    let mut detector = crate::face::FaceDetector::load_with_provider(&face_model_dir, state.verbosity, &state.ai_config.execution_provider)
        .map_err(|e| format!("Failed to load face detector: {e:#}"))?;

    let service = state.asset_service();
    let result = service.detect_faces(
        asset_ids,
        &mut detector,
        state.ai_config.face_min_confidence,
        true,
        true,
        |_, _, _| {},
    ).map_err(|e| format!("{e:#}"))?;

    Ok(serde_json::json!({
        "succeeded": result.assets_processed,
        "faces_detected": result.faces_detected,
        "errors": result.errors,
    }))
}

/// PUT /api/faces/{face_id}/assign — assign a face to a person.
pub async fn assign_face(
    State(state): State<Arc<AppState>>,
    Path(face_id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    let person_id: String = match body.get("person_id").and_then(|v| v.as_str()) {
        Some(pid) => pid.to_string(),
        None => return (StatusCode::BAD_REQUEST, "Missing person_id").into_response(),
    };

    match super::spawn_catalog_blocking(move || {
        let catalog = state.catalog()?;
        let face_store = crate::face_store::FaceStore::new(catalog.conn());
        face_store.assign_face_to_person(&face_id, &person_id)?;
        let _ = face_store.save_all_yaml(&state.catalog_root);
        state.dropdown_cache.invalidate_people();
        Ok(())
    }).await {
        Ok(()) => Json(serde_json::json!({"ok": true})).into_response(),
        Err(resp) => resp,
    }
}

/// DELETE /api/faces/{face_id}/unassign — unassign a face from its person.
pub async fn unassign_face_api(
    State(state): State<Arc<AppState>>,
    Path(face_id): Path<String>,
) -> Response {
    match super::spawn_catalog_blocking(move || {
        let catalog = state.catalog()?;
        let face_store = crate::face_store::FaceStore::new(catalog.conn());
        face_store.unassign_face(&face_id)?;
        let _ = face_store.save_all_yaml(&state.catalog_root);
        state.dropdown_cache.invalidate_people();
        Ok(())
    }).await {
        Ok(()) => Json(serde_json::json!({"ok": true})).into_response(),
        Err(resp) => resp,
    }
}

/// DELETE /api/faces/{face_id} — delete a face detection (e.g., false positive).
pub async fn delete_face_api(
    State(state): State<Arc<AppState>>,
    Path(face_id): Path<String>,
) -> Response {
    let catalog_root = state.catalog_root.clone();
    let result = tokio::task::spawn_blocking(move || {
        let catalog = state.catalog()?;
        let face_store = crate::face_store::FaceStore::new(catalog.conn());
        if let Some(asset_id) = face_store.delete_face(&face_id)? {
            catalog.update_face_count(&asset_id)?;
            let prefix = &face_id[..2.min(face_id.len())];
            let crop = catalog_root.join("faces").join(prefix).join(format!("{face_id}.jpg"));
            let _ = std::fs::remove_file(crop);
            crate::face_store::delete_arcface_binary(&catalog_root, &face_id);
        }
        let _ = face_store.save_all_yaml(&catalog_root);
        Ok::<_, anyhow::Error>(())
    }).await;

    match result {
        Ok(Ok(())) => Json(serde_json::json!({"ok": true})).into_response(),
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": format!("{e:#}")}))).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": format!("{e}")}))).into_response(),
    }
}

/// GET /people — people gallery page.
pub async fn people_page(
    State(state): State<Arc<AppState>>,
) -> Response {
    let result = tokio::task::spawn_blocking(move || {
        let catalog = state.catalog()?;
        let face_store = crate::face_store::FaceStore::new(catalog.conn());
        let people_list = face_store.list_people()?;

        let people: Vec<PersonCard> = people_list.into_iter().map(|(p, count)| {
            let crop_url = p.representative_face_id.as_ref().and_then(|fid| {
                if crate::face::face_crop_exists(fid, &state.catalog_root) {
                    Some(format!("/face/{}/{}.jpg", &fid[..2.min(fid.len())], fid))
                } else {
                    None
                }
            });
            PersonCard {
                name: p.name.unwrap_or_else(|| format!("Unknown ({})", &p.id[..8.min(p.id.len())])),
                id: p.id,
                face_count: count,
                crop_url,
            }
        }).collect();

        let tmpl = PeoplePage {
            people,
            ai_enabled: true,
            vlm_enabled: state.vlm_enabled,
        };
        Ok::<_, anyhow::Error>(tmpl.render()?)
    }).await;

    match result {
        Ok(Ok(html)) => Html(html).into_response(),
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e}")).into_response(),
    }
}

/// GET /api/people — JSON list of people (for dropdown).
pub async fn list_people_api(
    State(state): State<Arc<AppState>>,
) -> Response {
    let result = tokio::task::spawn_blocking(move || {
        let catalog = state.catalog()?;
        let face_store = crate::face_store::FaceStore::new(catalog.conn());
        let people = face_store.list_people()?;
        let json: Vec<serde_json::Value> = people.into_iter().map(|(p, count)| {
            serde_json::json!({
                "id": p.id,
                "name": p.name,
                "face_count": count,
                "representative_face_id": p.representative_face_id,
            })
        }).collect();
        Ok::<_, anyhow::Error>(json)
    }).await;

    match result {
        Ok(Ok(json)) => Json(json).into_response(),
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e}")).into_response(),
    }
}

/// POST /api/people — create a new person.
pub async fn create_person_api(
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    let name = body.get("name").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
    if name.is_empty() {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": "name is required"}))).into_response();
    }
    let result = tokio::task::spawn_blocking(move || {
        let catalog = state.catalog()?;
        let face_store = crate::face_store::FaceStore::new(catalog.conn());
        let id = face_store.create_person(Some(&name))?;
        let _ = face_store.save_all_yaml(&state.catalog_root);
        Ok::<_, anyhow::Error>(serde_json::json!({"id": id, "name": name}))
    }).await;

    match result {
        Ok(Ok(json)) => Json(json).into_response(),
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": format!("{e:#}")}))).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": format!("{e}")}))).into_response(),
    }
}

/// PUT /api/people/{id}/name — rename a person.
pub async fn name_person_api(
    State(state): State<Arc<AppState>>,
    Path(person_id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    let name: String = match body.get("name").and_then(|v| v.as_str()) {
        Some(n) => n.to_string(),
        None => return (StatusCode::BAD_REQUEST, "Missing name").into_response(),
    };

    let result = tokio::task::spawn_blocking(move || {
        let catalog = state.catalog()?;
        let face_store = crate::face_store::FaceStore::new(catalog.conn());
        face_store.name_person(&person_id, &name)?;
        let _ = face_store.save_all_yaml(&state.catalog_root);
        state.dropdown_cache.invalidate_people();
        Ok::<_, anyhow::Error>(())
    }).await;

    match result {
        Ok(Ok(())) => Json(serde_json::json!({"ok": true})).into_response(),
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e}")).into_response(),
    }
}

/// POST /api/people/{id}/merge — merge source people into target.
pub async fn merge_person_api(
    State(state): State<Arc<AppState>>,
    Path(target_id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    let source_ids: Vec<String> = if let Some(arr) = body.get("source_ids").and_then(|v| v.as_array()) {
        arr.iter().filter_map(|v| v.as_str().map(String::from)).collect()
    } else if let Some(s) = body.get("source_id").and_then(|v| v.as_str()) {
        vec![s.to_string()]
    } else {
        return (StatusCode::BAD_REQUEST, "Missing source_id or source_ids").into_response();
    };
    if source_ids.is_empty() {
        return (StatusCode::BAD_REQUEST, "Empty source_ids").into_response();
    }

    let result = tokio::task::spawn_blocking(move || {
        let catalog = state.catalog()?;
        let face_store = crate::face_store::FaceStore::new(catalog.conn());
        let moved = face_store.merge_people_batch(&target_id, &source_ids)?;
        let _ = face_store.save_all_yaml(&state.catalog_root);
        state.dropdown_cache.invalidate_people();
        Ok::<_, anyhow::Error>((moved, source_ids.len()))
    }).await;

    match result {
        Ok(Ok((moved, n))) => Json(serde_json::json!({"ok": true, "faces_moved": moved, "people_merged": n})).into_response(),
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e}")).into_response(),
    }
}

/// DELETE /api/people/{id} — delete a person.
pub async fn delete_person_api(
    State(state): State<Arc<AppState>>,
    Path(person_id): Path<String>,
) -> Response {
    let result = tokio::task::spawn_blocking(move || {
        let catalog = state.catalog()?;
        let face_store = crate::face_store::FaceStore::new(catalog.conn());
        face_store.delete_person(&person_id)?;
        let _ = face_store.save_all_yaml(&state.catalog_root);
        state.dropdown_cache.invalidate_people();
        Ok::<_, anyhow::Error>(())
    }).await;

    match result {
        Ok(Ok(())) => Json(serde_json::json!({"ok": true})).into_response(),
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e}")).into_response(),
    }
}

/// GET /api/people/merge-suggestions — find pairs likely to be the same person.
pub async fn merge_suggestions_api(
    State(state): State<Arc<AppState>>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Response {
    let threshold: f32 = params.get("threshold").and_then(|s| s.parse().ok()).unwrap_or(0.4);
    let limit: usize = params.get("limit").and_then(|s| s.parse().ok()).unwrap_or(20);

    let result = tokio::task::spawn_blocking(move || {
        let catalog = state.catalog()?;
        let face_store = crate::face_store::FaceStore::new(catalog.conn());
        let pairs = face_store.suggest_person_merges(threshold, limit)?;

        let people = face_store.list_people()?;
        let lookup: std::collections::HashMap<String, (Option<String>, usize, Option<String>)> =
            people.into_iter().map(|(p, count)| (p.id.clone(), (p.name, count, p.representative_face_id))).collect();

        let items: Vec<serde_json::Value> = pairs.into_iter().filter_map(|(a, b, sim)| {
            let info_a = lookup.get(&a)?;
            let info_b = lookup.get(&b)?;
            let crop_for = |fid: &Option<String>| -> Option<String> {
                fid.as_ref().and_then(|f| {
                    if crate::face::face_crop_exists(f, &state.catalog_root) {
                        Some(format!("/face/{}/{}.jpg", &f[..2.min(f.len())], f))
                    } else { None }
                })
            };
            Some(serde_json::json!({
                "similarity": sim,
                "a": {
                    "id": a,
                    "name": info_a.0.clone().unwrap_or_else(|| format!("Unknown ({})", &a[..8.min(a.len())])),
                    "face_count": info_a.1,
                    "crop_url": crop_for(&info_a.2),
                    "named": info_a.0.is_some(),
                },
                "b": {
                    "id": b,
                    "name": info_b.0.clone().unwrap_or_else(|| format!("Unknown ({})", &b[..8.min(b.len())])),
                    "face_count": info_b.1,
                    "crop_url": crop_for(&info_b.2),
                    "named": info_b.0.is_some(),
                },
            }))
        }).collect();

        Ok::<_, anyhow::Error>(items)
    }).await;

    match result {
        Ok(Ok(items)) => Json(serde_json::json!({"suggestions": items})).into_response(),
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e}")).into_response(),
    }
}

/// POST /api/faces/cluster — run auto-clustering.
pub async fn cluster_faces_api(
    State(state): State<Arc<AppState>>,
) -> Response {
    let result = tokio::task::spawn_blocking(move || {
        let catalog = state.catalog()?;
        let _ = crate::face_store::FaceStore::initialize(catalog.conn());
        let face_store = crate::face_store::FaceStore::new(catalog.conn());
        let threshold = state.ai_config.face_cluster_threshold;
        let min_confidence = state.ai_config.face_min_confidence;
        let result = face_store.auto_cluster(threshold, min_confidence, None)?;
        let _ = face_store.save_all_yaml(&state.catalog_root);
        state.dropdown_cache.invalidate_people();
        Ok::<_, anyhow::Error>(result)
    }).await;

    match result {
        Ok(Ok(result)) => Json(serde_json::json!({
            "people_created": result.people_created,
            "faces_assigned": result.faces_assigned,
            "singletons_skipped": result.singletons_skipped,
        })).into_response(),
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e}")).into_response(),
    }
}

// --- Stroll page (visual exploration) ---

#[derive(Debug, serde::Deserialize)]
pub struct StrollParams {
    pub id: Option<String>,
    pub q: Option<String>,
    pub n: Option<u32>,
    pub mode: Option<String>,
    pub skip: Option<u32>,
    pub cross_session: Option<bool>,
}

/// GET /stroll — visual exploration page.
pub async fn stroll_page(
    State(state): State<Arc<AppState>>,
    Query(params): Query<StrollParams>,
) -> Response {
    let state = state.clone();
    let result: Result<Result<StrollPage, String>, _> =
        tokio::task::spawn_blocking(move || {
            let default_n = state.stroll_neighbors;
            let max_n = state.stroll_neighbors_max;
            let n = params.n.unwrap_or(default_n).clamp(5, max_n);
            let mode = params.mode.as_deref().unwrap_or("nearest");
            let skip = params.skip.unwrap_or(0);
            let cross_session = params.cross_session.unwrap_or(false);
            stroll_page_inner(&state, params.id.as_deref(), params.q.as_deref(), n, mode, skip, cross_session)
        }).await;

    match result {
        Ok(Ok(page)) => Html(page.render().unwrap_or_default()).into_response(),
        Ok(Err(msg)) => (StatusCode::NOT_FOUND, msg).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e}")).into_response(),
    }
}

fn stroll_page_inner(
    state: &AppState,
    asset_id: Option<&str>,
    query: Option<&str>,
    neighbor_count: u32,
    mode: &str,
    skip: u32,
    cross_session: bool,
) -> Result<StrollPage, String> {
    let catalog = state.catalog().map_err(|e| format!("{e:#}"))?;
    let preview_gen = state.preview_generator();
    let preview_ext = &state.preview_ext;
    let model_id = &state.ai_config.model;

    let _ = crate::embedding_store::EmbeddingStore::initialize(catalog.conn());
    let emb_store = crate::embedding_store::EmbeddingStore::new(catalog.conn());

    let center_id = if let Some(id_prefix) = asset_id {
        super::resolve_asset_id_or_err(&catalog, id_prefix).map_err(|e| format!("{e:#}"))?
    } else {
        let all = emb_store.all_embeddings_for_model(model_id).map_err(|e| format!("{e:#}"))?;
        if all.is_empty() {
            return Err("No embeddings found. Run `maki embed` first.".into());
        }
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        std::time::SystemTime::now().hash(&mut hasher);
        let idx = (hasher.finish() as usize) % all.len();
        all[idx].0.clone()
    };

    let effective_query = match (query, &state.default_filter) {
        (Some(q), Some(df)) if !q.trim().is_empty() => Some(format!("{} {}", q, df)),
        (Some(q), _) if !q.trim().is_empty() => Some(q.to_string()),
        (_, Some(df)) if !df.is_empty() => Some(df.clone()),
        _ => None,
    };
    let filter_ids: Option<std::collections::HashSet<String>> = if let Some(ref eq) = effective_query {
        let engine = state.query_engine();
        let results = engine.search(eq)
            .map_err(|e| format!("{e:#}"))?;
        Some(results.into_iter().map(|r| r.asset_id).collect())
    } else {
        None
    };

    let exclude_session: Option<std::collections::HashSet<String>> = if cross_session {
        catalog.find_same_session_asset_ids(&center_id)
            .ok()
            .filter(|ids| ids.len() > 1)
    } else {
        None
    };

    let details = catalog
        .load_asset_details(&center_id)
        .map_err(|e| format!("{e:#}"))?
        .ok_or_else(|| format!("Asset '{center_id}' not found"))?;

    let center_preview = best_preview_for_details(&details, &preview_gen, preview_ext);
    let center_smart = best_smart_preview_for_details(&details, &preview_gen, preview_ext);

    let center = StrollCenter {
        asset_id: center_id.clone(),
        name: details.name.clone().unwrap_or_else(|| {
            details.variants.first()
                .and_then(|v| v.locations.first().map(|fl| {
                    std::path::Path::new(&fl.relative_path)
                        .file_name().unwrap_or_default()
                        .to_string_lossy().to_string()
                }))
                .unwrap_or_else(|| center_id[..8.min(center_id.len())].to_string())
        }),
        preview_url: center_preview.unwrap_or_default(),
        smart_preview_url: center_smart,
        rating: details.rating.map(|r| r.min(5)),
        color_label: details.color_label.clone(),
        format: details.variants.first().map(|v| v.format.clone()).unwrap_or_default(),
        created_at: details.created_at.clone(),
    };

    let query_emb = emb_store.get(&center_id, model_id).map_err(|e| format!("{e:#}"))?;
    let base_limit = match mode {
        "discover" => (state.stroll_discover_pool as usize).max(neighbor_count as usize * 4),
        "explore" => (skip as usize) + (neighbor_count as usize),
        _ => neighbor_count as usize,
    };
    let has_filters = filter_ids.is_some() || exclude_session.is_some();
    let fetch_limit = if has_filters { base_limit * 4 } else { base_limit };
    let neighbors = if let Some(emb) = query_emb {
        let spec = crate::ai::get_model_spec(model_id)
            .ok_or_else(|| format!("Unknown model: {model_id}"))?;

        {
            let needs_load = state.ai_embedding_index.read().unwrap().is_none();
            if needs_load {
                let index = crate::embedding_store::EmbeddingIndex::load(
                    catalog.conn(), model_id, spec.embedding_dim,
                ).map_err(|e| format!("{e:#}"))?;
                *state.ai_embedding_index.write().unwrap() = Some(index);
            }
        }
        {
            let mut idx_guard = state.ai_embedding_index.write().unwrap();
            if let Some(ref mut idx) = *idx_guard {
                idx.upsert(&center_id, &emb);
            }
        }
        let results = {
            let idx_guard = state.ai_embedding_index.read().unwrap();
            let idx = idx_guard.as_ref().unwrap();
            idx.search(&emb, fetch_limit, Some(&center_id))
        };

        let filtered_results: Vec<(String, f32)> = results.into_iter().filter(|(id, _)| {
            if let Some(ref fids) = filter_ids {
                if !fids.contains(id) { return false; }
            }
            if let Some(ref exc) = exclude_session {
                if exc.contains(id) { return false; }
            }
            true
        }).collect();

        let selected: Vec<(String, f32)> = match mode {
            "discover" => {
                let mut pool = filtered_results;
                let seed = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos() as u64;
                let mut rng = seed;
                for i in (1..pool.len()).rev() {
                    rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                    let j = (rng >> 33) as usize % (i + 1);
                    pool.swap(i, j);
                }
                pool.truncate(neighbor_count as usize);
                pool
            }
            "explore" => {
                let skip_n = (skip as usize).min(filtered_results.len());
                filtered_results.into_iter().skip(skip_n).take(neighbor_count as usize).collect()
            }
            _ => {
                filtered_results.into_iter().take(neighbor_count as usize).collect()
            }
        };

        let mut neighbors: Vec<StrollNeighbor> = selected.into_iter().filter_map(|(id, similarity)| {
            let cat = state.catalog().ok()?;
            let d = cat.load_asset_details(&id).ok()??;
            let name = d.name.clone().unwrap_or_else(|| {
                d.variants.first()
                    .and_then(|v| v.locations.first().map(|fl| {
                        std::path::Path::new(&fl.relative_path)
                            .file_name().unwrap_or_default()
                            .to_string_lossy().to_string()
                    }))
                    .unwrap_or_else(|| id[..8.min(id.len())].to_string())
            });
            let purl = best_preview_for_details(&d, &preview_gen, preview_ext)?;
            Some(StrollNeighbor {
                asset_id: id,
                name,
                preview_url: purl,
                similarity,
                similarity_pct: (similarity * 100.0) as u32,
                rating: d.rating.map(|r| r.min(5)),
                color_label: d.color_label.clone(),
            })
        }).collect();
        neighbors.truncate(neighbor_count as usize);
        neighbors
    } else {
        Vec::new()
    };

    let all_tags: Vec<TagOption> = state.dropdown_cache.get_tags(&catalog)
        .into_iter()
        .map(|(name, count)| TagOption { name, count })
        .collect();
    let format_groups = build_format_groups(state.dropdown_cache.get_formats(&catalog));
    let all_volumes: Vec<VolumeOption> = state.dropdown_cache.get_volumes(&catalog)
        .into_iter()
        .map(|(id, label)| VolumeOption { id, label })
        .collect();
    let all_collections: Vec<CollectionOption> = state.dropdown_cache.get_collections(&catalog)
        .into_iter()
        .map(|name| CollectionOption { name })
        .collect();
    let all_people: Vec<PersonOption> = state.dropdown_cache.get_people(&catalog)
        .into_iter()
        .map(|(id, name)| PersonOption { id, name })
        .collect();

    Ok(StrollPage {
        center,
        neighbors,
        query: query.unwrap_or("").to_string(),
        neighbor_count,
        stroll_neighbors_max: state.stroll_neighbors_max,
        stroll_fanout: state.stroll_fanout,
        stroll_fanout_max: state.stroll_fanout_max,
        ai_enabled: state.ai_enabled,
        vlm_enabled: state.vlm_enabled,
        tag: String::new(),
        rating: String::new(),
        label: String::new(),
        asset_type: String::new(),
        format_filter: String::new(),
        format_groups,
        all_tags,
        all_volumes,
        all_collections,
        all_people,
        volume: String::new(),
        collection: String::new(),
        path: String::new(),
        person: String::new(),
        default_filter: state.default_filter.clone().unwrap_or_default(),
        default_filter_active: state.default_filter.is_some(),
    })
}

fn best_preview_for_details(
    details: &crate::catalog::AssetDetails,
    preview_gen: &crate::preview::PreviewGenerator,
    ext: &str,
) -> Option<String> {
    let idx = crate::models::variant::best_preview_index_details(&details.variants)?;
    let v = &details.variants[idx];
    if preview_gen.has_preview(&v.content_hash) {
        Some(crate::web::templates::preview_url(&v.content_hash, ext))
    } else {
        None
    }
}

fn best_smart_preview_for_details(
    details: &crate::catalog::AssetDetails,
    preview_gen: &crate::preview::PreviewGenerator,
    ext: &str,
) -> Option<String> {
    let idx = crate::models::variant::best_preview_index_details(&details.variants)?;
    let v = &details.variants[idx];
    if preview_gen.has_smart_preview(&v.content_hash) {
        Some(crate::web::templates::smart_preview_url(&v.content_hash, ext))
    } else {
        None
    }
}

/// GET /api/stroll/neighbors — JSON neighbor data for navigation.
pub async fn stroll_neighbors_api(
    State(state): State<Arc<AppState>>,
    Query(params): Query<StrollParams>,
) -> Response {
    let asset_id = match params.id {
        Some(id) => id,
        None => return (StatusCode::BAD_REQUEST, "Missing id parameter").into_response(),
    };
    let state = state.clone();
    let q = params.q;
    let mode = params.mode.unwrap_or_default();
    let skip = params.skip.unwrap_or(0);
    let cross_session = params.cross_session.unwrap_or(false);
    let default_n = state.stroll_neighbors;
    let max_n = state.stroll_neighbors_max;
    let n = params.n.unwrap_or(default_n).clamp(5, max_n);
    let result: Result<Result<serde_json::Value, String>, _> =
        tokio::task::spawn_blocking(move || {
            let m = if mode.is_empty() { "nearest" } else { &mode };
            let page = stroll_page_inner(&state, Some(&asset_id), q.as_deref(), n, m, skip, cross_session)?;
            Ok(serde_json::json!({
                "center": {
                    "asset_id": page.center.asset_id,
                    "name": page.center.name,
                    "preview_url": page.center.preview_url,
                    "smart_preview_url": page.center.smart_preview_url,
                    "rating": page.center.rating,
                    "color_label": page.center.color_label,
                    "format": page.center.format,
                    "created_at": page.center.created_at,
                },
                "neighbors": page.neighbors.iter().map(|n| serde_json::json!({
                    "asset_id": n.asset_id,
                    "name": n.name,
                    "preview_url": n.preview_url,
                    "similarity": n.similarity,
                    "rating": n.rating,
                    "color_label": n.color_label,
                })).collect::<Vec<_>>(),
            }))
        }).await;

    match result {
        Ok(Ok(json)) => Json(json).into_response(),
        Ok(Err(msg)) => (StatusCode::NOT_FOUND, msg).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e}")).into_response(),
    }
}
