//! Visual similarity search — the `/api/asset/{id}/similar` endpoint.
//!
//! Resolves an asset's stored SigLIP embedding, queries the in-memory
//! similarity index, and returns scored neighbours.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;

use crate::web::AppState;

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

        let model_dir = super::resolve_model_dir(&state.ai_config);
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
