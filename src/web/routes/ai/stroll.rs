//! Stroll — the visual exploration page. Walks the embedding space from
//! a starting asset and surfaces close neighbours, optionally crossing
//! session/event boundaries.

use std::sync::Arc;

use askama::Template;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use axum::Json;

use crate::web::templates::{
    CollectionOption, PersonOption, StrollCenter, StrollNeighbor, StrollPage, TagOption,
    VolumeOption,
};
use crate::web::AppState;

use super::super::stats::build_format_groups;

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
        super::super::resolve_asset_id_or_err(&catalog, id_prefix).map_err(|e| format!("{e:#}"))?
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
