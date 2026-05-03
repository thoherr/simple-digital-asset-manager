//! `suggest-tags` and batch `auto-tag` route handlers.
//!
//! Both rely on the SigLIP model loaded into AppState's cached slot, with
//! the active label list (config-defined or `DEFAULT_LABELS`) embedded once
//! and reused across requests.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;

use crate::web::AppState;

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

    let model_dir = super::resolve_model_dir(&state.ai_config);
    let model_id = &state.ai_config.model;
    let model_guard = state.ai_model.blocking_lock();
    let mut model_opt = model_guard;
    if model_opt.is_none() {
        let m = ai::SigLipModel::load_with_provider(&model_dir, model_id, state.verbosity, &state.ai_config.execution_provider)
            .map_err(|e| format!("Failed to load AI model: {e:#}"))?;
        *model_opt = Some(m);
    }
    let model = model_opt.as_mut().unwrap();

    let labels = super::resolve_labels(&state.ai_config)?;
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

    let model_dir = super::resolve_model_dir(&state.ai_config);
    let model_id = &state.ai_config.model;
    let mut model_guard = state.ai_model.blocking_lock();
    if model_guard.is_none() {
        let m = ai::SigLipModel::load_with_provider(&model_dir, model_id, state.verbosity, &state.ai_config.execution_provider)
            .map_err(|e| format!("Failed to load AI model: {e:#}"))?;
        *model_guard = Some(m);
    }
    let model = model_guard.as_mut().unwrap();

    let labels = super::resolve_labels(&state.ai_config)?;
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
