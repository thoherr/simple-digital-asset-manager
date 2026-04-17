//! Stack operations: split, create, add, unstack, delete, pick, dissolve, similarity-based stacking.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;

use crate::web::AppState;

// --- Split operation ---

#[derive(serde::Deserialize)]
pub struct SplitRequest {
    pub variant_hashes: Vec<String>,
}

/// POST /api/asset/{id}/split — extract variants into new assets.
pub async fn split_asset(
    State(state): State<Arc<AppState>>,
    Path(asset_id): Path<String>,
    Json(req): Json<SplitRequest>,
) -> Response {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let engine = state.query_engine();
        let result = engine.split(&asset_id, &req.variant_hashes)?;
        Ok::<_, anyhow::Error>(serde_json::json!({
            "source_id": result.source_id,
            "new_assets": result.new_assets,
        }))
    })
    .await;

    match result {
        Ok(Ok(json)) => Json(json).into_response(),
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e:#}")).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
    }
}

// --- Stack batch operations ---

#[derive(serde::Deserialize)]
pub struct BatchStackRequest {
    pub asset_ids: Vec<String>,
}

/// POST /api/batch/stack — create a stack from selected assets.
pub async fn batch_create_stack(
    State(state): State<Arc<AppState>>,
    Json(req): Json<BatchStackRequest>,
) -> Response {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let catalog = state.catalog()?;
        let store = crate::stack::StackStore::new(catalog.conn());
        let stack = store.create(&req.asset_ids)?;
        let yaml = store.export_all()?;
        crate::stack::save_yaml(&state.catalog_root, &yaml)?;
        Ok::<_, anyhow::Error>(serde_json::json!({
            "stack_id": stack.id.to_string(),
            "member_count": stack.asset_ids.len(),
        }))
    })
    .await;

    match result {
        Ok(Ok(json)) => Json(json).into_response(),
        Ok(Err(e)) => (StatusCode::BAD_REQUEST, format!("Error: {e:#}")).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
    }
}

/// POST /api/asset/{id}/stack-add — add this asset to an existing stack.
pub async fn add_to_stack(
    State(state): State<Arc<AppState>>,
    Path(reference_id): Path<String>,
    Json(req): Json<BatchStackRequest>,
) -> Response {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let catalog = state.catalog()?;
        let ref_full = catalog
            .resolve_asset_id(&reference_id)?
            .ok_or_else(|| anyhow::anyhow!("no asset found matching '{reference_id}'"))?;
        let store = crate::stack::StackStore::new(catalog.conn());
        let added = store.add(&ref_full, &req.asset_ids)?;
        let yaml = store.export_all()?;
        crate::stack::save_yaml(&state.catalog_root, &yaml)?;
        Ok::<_, anyhow::Error>(serde_json::json!({
            "added": added,
        }))
    })
    .await;

    match result {
        Ok(Ok(json)) => Json(json).into_response(),
        Ok(Err(e)) => (StatusCode::BAD_REQUEST, format!("Error: {e:#}")).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
    }
}

/// DELETE /api/batch/stack — unstack selected assets.
pub async fn batch_unstack(
    State(state): State<Arc<AppState>>,
    Json(req): Json<BatchStackRequest>,
) -> Response {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let catalog = state.catalog()?;
        let store = crate::stack::StackStore::new(catalog.conn());
        let removed = store.remove(&req.asset_ids)?;
        let yaml = store.export_all()?;
        crate::stack::save_yaml(&state.catalog_root, &yaml)?;
        Ok::<_, anyhow::Error>(serde_json::json!({
            "removed": removed,
        }))
    })
    .await;

    match result {
        Ok(Ok(json)) => Json(json).into_response(),
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e:#}")).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
    }
}

/// POST /api/batch/delete — delete selected assets from catalog.
#[derive(serde::Deserialize)]
pub struct BatchDeleteRequest {
    pub asset_ids: Vec<String>,
    #[serde(default)]
    pub remove_files: bool,
}

pub async fn batch_delete(
    State(state): State<Arc<AppState>>,
    Json(req): Json<BatchDeleteRequest>,
) -> Response {
    let catalog_root = state.catalog_root.clone();
    let preview_config = state.preview_config.clone();
    let result = tokio::task::spawn_blocking(move || {
        let service = crate::asset_service::AssetService::new(&catalog_root, state.verbosity, &preview_config);
        let result = service.delete_assets(&req.asset_ids, true, req.remove_files, |_id, _status, _elapsed| {})?;
        Ok::<_, anyhow::Error>(serde_json::json!({
            "deleted": result.deleted,
            "files_removed": result.files_removed,
            "previews_removed": result.previews_removed,
            "not_found": result.not_found,
            "errors": result.errors,
        }))
    })
    .await;

    match result {
        Ok(Ok(json)) => Json(json).into_response(),
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e:#}")).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
    }
}

/// PUT /api/asset/{id}/stack-pick — set this asset as the stack pick.
pub async fn set_stack_pick(
    State(state): State<Arc<AppState>>,
    Path(asset_id): Path<String>,
) -> Response {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let catalog = state.catalog()?;
        let store = crate::stack::StackStore::new(catalog.conn());
        store.set_pick(&asset_id)?;
        let yaml = store.export_all()?;
        crate::stack::save_yaml(&state.catalog_root, &yaml)?;
        Ok::<_, anyhow::Error>(serde_json::json!({ "pick": asset_id }))
    })
    .await;

    match result {
        Ok(Ok(json)) => Json(json).into_response(),
        Ok(Err(e)) => (StatusCode::BAD_REQUEST, format!("Error: {e:#}")).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
    }
}

/// DELETE /api/asset/{id}/stack — dissolve the stack this asset belongs to.
pub async fn dissolve_stack(
    State(state): State<Arc<AppState>>,
    Path(asset_id): Path<String>,
) -> Response {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let catalog = state.catalog()?;
        let store = crate::stack::StackStore::new(catalog.conn());
        store.dissolve(&asset_id)?;
        let yaml = store.export_all()?;
        crate::stack::save_yaml(&state.catalog_root, &yaml)?;
        Ok::<_, anyhow::Error>(serde_json::json!({ "status": "dissolved" }))
    })
    .await;

    match result {
        Ok(Ok(json)) => Json(json).into_response(),
        Ok(Err(e)) => (StatusCode::BAD_REQUEST, format!("Error: {e:#}")).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
    }
}

/// POST /api/asset/{id}/stack-similar — stack visually similar assets (AI).
#[cfg(feature = "ai")]
pub async fn stack_by_similarity(
    State(state): State<Arc<AppState>>,
    Path(asset_id): Path<String>,
    Json(params): Json<StackSimilarRequest>,
) -> Response {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let catalog = state.catalog()?;
        let full_id = catalog
            .resolve_asset_id(&asset_id)?
            .ok_or_else(|| anyhow::anyhow!("no asset found matching '{asset_id}'"))?;

        let store = crate::stack::StackStore::new(catalog.conn());
        if store.stack_for_asset(&full_id)?.is_some() {
            anyhow::bail!("asset is already in a stack. Dissolve it first.");
        }

        let model_id = &state.ai_config.model;
        let spec = crate::ai::get_model_spec(model_id)
            .ok_or_else(|| anyhow::anyhow!("aI model not configured"))?;

        let emb_store = crate::embedding_store::EmbeddingStore::new(catalog.conn());
        let query_emb = emb_store
            .get(&full_id, model_id)?
            .ok_or_else(|| anyhow::anyhow!(
                "No embedding for this asset. Run `maki embed --asset {}` first.", &full_id
            ))?;

        let needs_load = state.ai_embedding_index.read().unwrap().is_none();
        if needs_load {
            if let Ok(index) = crate::embedding_store::EmbeddingIndex::load(
                catalog.conn(), model_id, spec.embedding_dim,
            ) {
                *state.ai_embedding_index.write().unwrap() = Some(index);
            }
        }

        let threshold = params.threshold.unwrap_or(85.0).clamp(0.0, 100.0) / 100.0;
        let limit = params.limit.unwrap_or(40);

        let results = {
            let idx_guard = state.ai_embedding_index.read().unwrap();
            if let Some(ref idx) = *idx_guard {
                idx.search(&query_emb, limit, Some(&full_id))
            } else {
                return Err(anyhow::anyhow!("embedding index not available"));
            }
        };

        let mut stack_ids: Vec<String> = vec![full_id.clone()];
        for (id, sim) in &results {
            if *sim >= threshold {
                if store.stack_for_asset(id)?.is_none() {
                    stack_ids.push(id.clone());
                }
            }
        }

        if stack_ids.len() < 2 {
            anyhow::bail!("no similar assets found above {}% threshold", (threshold * 100.0) as u32);
        }

        let stack = store.create(&stack_ids)?;
        let yaml = store.export_all()?;
        crate::stack::save_yaml(&state.catalog_root, &yaml)?;

        Ok(serde_json::json!({
            "stack_id": stack.id.to_string(),
            "member_count": stack_ids.len(),
            "threshold": (threshold * 100.0) as u32,
        }))
    })
    .await;

    match result {
        Ok(Ok(json)) => Json(json).into_response(),
        Ok(Err(e)) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": format!("{e:#}") }))).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
    }
}

#[cfg(feature = "ai")]
#[derive(Debug, serde::Deserialize)]
pub struct StackSimilarRequest {
    pub threshold: Option<f32>,
    pub limit: Option<usize>,
}

/// GET /api/stack/{id}/members — return stack member cards as JSON.
pub async fn stack_members_api(
    State(state): State<Arc<AppState>>,
    Path(stack_id): Path<String>,
) -> Response {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let catalog = state.catalog()?;
        let store = crate::stack::StackStore::new(catalog.conn());
        let member_ids = store.ordered_members(&stack_id)?;
        let preview_ext = &state.preview_ext;

        let mut members = Vec::new();
        for mid in &member_ids {
            let row = catalog.get_search_row(mid);
            if let Ok(Some(r)) = row {
                members.push(serde_json::json!({
                    "asset_id": r.asset_id,
                    "name": r.name.as_deref().unwrap_or(&r.original_filename),
                    "asset_type": r.asset_type,
                    "format": r.primary_format.as_deref().unwrap_or(&r.format),
                    "date": crate::web::templates::format_date(&r.created_at),
                    "preview_url": crate::web::templates::preview_url(&r.content_hash, preview_ext),
                    "rating": r.rating.unwrap_or(0),
                    "label": r.color_label.as_deref().unwrap_or(""),
                    "variant_count": r.variant_count,
                    "stack_id": r.stack_id,
                    "stack_count": r.stack_count,
                    "preview_rotation": r.preview_rotation.unwrap_or(0),
                    "face_count": r.face_count,
                }));
            }
        }
        Ok::<_, anyhow::Error>(serde_json::json!(members))
    })
    .await;

    match result {
        Ok(Ok(json)) => Json(json).into_response(),
        Ok(Err(e)) => (StatusCode::BAD_REQUEST, format!("Error: {e:#}")).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
    }
}
