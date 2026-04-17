//! Browse / search / asset-page / facets route handlers.
//!
//! Shared helpers (build_parsed_search, merge_search_params, resolve_collection_ids,
//! intersect_name_groups, resolve_best_variant_idx, resolve_similar_filter,
//! BrowseFilters, SearchParams) live in the parent routes module so sibling
//! submodules (calendar_map, media, assets, ai) can reuse them.

use std::sync::Arc;

use askama::Template;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{Html, IntoResponse, Response};
use axum::Json;

use crate::catalog::SearchSort;
use crate::device_registry::DeviceRegistry;
use crate::web::templates::{
    link_cards, AssetCard, AssetPage, BrowsePage, CollectionOption, FaceRow,
    PersonOption, ResultsPartial, SavedSearchChip, StackMemberCard, TagOption, VolumeOption,
};
use crate::web::AppState;

use super::stats::build_format_groups;
use super::{
    build_parsed_search, resolve_best_variant_idx, resolve_collection_ids, SearchParams,
};
#[cfg(feature = "ai")]
use super::intersect_name_groups;
#[cfg(feature = "ai")]
use super::resolve_similar_filter;

/// GET / — browse page with initial results (full page for browser, partial for htmx).
pub async fn browse_page(
    State(state): State<Arc<AppState>>,
    Query(params): Query<SearchParams>,
    headers: HeaderMap,
) -> Response {
    let is_htmx = headers.get("HX-Request").is_some();
    let preview_ext = state.preview_ext.clone();
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let catalog = state.catalog()?;

        let bf = build_parsed_search(&params, &state);
        let parsed = bf.parsed;
        let volume = bf.volume;
        let path_volume_id = bf.path_volume_id;
        let sort_str = bf.sort_str;
        let page = bf.page;
        let collapse_stacks = bf.collapse_stacks;
        let query = bf.query;
        let asset_type = bf.asset_type;
        let tag = bf.tag;
        let format_filter = bf.format_filter;
        let rating_str = bf.rating;
        let label_str = bf.label;
        let collection_str = bf.collection;
        let path_str = bf.path;
        let person_str = bf.person;
        let nodefault = bf.nodefault;

        let mut opts = parsed.to_search_options();
        if !volume.is_empty() {
            opts.volume = Some(&volume);
        }
        if let Some(ref vid) = path_volume_id {
            if opts.volume.is_none() {
                opts.volume = Some(vid);
            }
        }

        let collection_ids;
        if !parsed.collections.is_empty() {
            collection_ids = resolve_collection_ids(&parsed.collections, catalog.conn());
            opts.collection_asset_ids = Some(&collection_ids);
        }

        let collection_exclude_ids;
        if !parsed.collections_exclude.is_empty() {
            collection_exclude_ids = resolve_collection_ids(&parsed.collections_exclude, catalog.conn());
            opts.collection_exclude_ids = Some(&collection_exclude_ids);
        }

        let person_ids;
        if !parsed.persons.is_empty() {
            #[cfg(feature = "ai")]
            {
                let face_store = crate::face_store::FaceStore::new(catalog.conn());
                person_ids = intersect_name_groups(&parsed.persons, |name| {
                    face_store.find_person_asset_ids(name).unwrap_or_default()
                });
                opts.person_asset_ids = Some(&person_ids);
            }
            #[cfg(not(feature = "ai"))]
            {
                person_ids = Vec::<String>::new();
                opts.person_asset_ids = Some(&person_ids);
            }
        }

        #[cfg(feature = "ai")]
        let text_query_ids;
        #[cfg(feature = "ai")]
        if let Some(ref text_q) = parsed.text_query {
            let model_id = &state.ai_config.model;
            let spec = crate::ai::get_model_spec(model_id);
            if let Some(spec) = spec {
                let model_dir = super::ai::resolve_model_dir(&state.ai_config);
                let mut model_guard = state.ai_model.blocking_lock();
                if model_guard.is_none() {
                    if let Ok(m) = crate::ai::SigLipModel::load_with_provider(
                        &model_dir, model_id, state.verbosity, &state.ai_config.execution_provider,
                    ) {
                        *model_guard = Some(m);
                    }
                }
                if let Some(ref mut model) = *model_guard {
                    if let Ok(embs) = model.encode_texts(&[text_q.clone()]) {
                        let query_emb = &embs[0];
                        let needs_load = state.ai_embedding_index.read().unwrap().is_none();
                        if needs_load {
                            if let Ok(index) = crate::embedding_store::EmbeddingIndex::load(
                                catalog.conn(), model_id, spec.embedding_dim,
                            ) {
                                *state.ai_embedding_index.write().unwrap() = Some(index);
                            }
                        }
                        let results = {
                            let idx_guard = state.ai_embedding_index.read().unwrap();
                            if let Some(ref idx) = *idx_guard {
                                idx.search(query_emb, parsed.text_query_limit.unwrap_or(state.ai_config.text_limit), None)
                            } else {
                                Vec::new()
                            }
                        };
                        text_query_ids = results.into_iter().map(|(id, _)| id).collect::<Vec<_>>();
                        opts.text_search_ids = Some(&text_query_ids);
                    }
                }
            }
        }

        #[cfg(feature = "ai")]
        let similar_ids;
        #[cfg(feature = "ai")]
        let similarity_scores: std::collections::HashMap<String, f32>;
        #[cfg(feature = "ai")]
        {
            let (ids, scores) = resolve_similar_filter(&catalog, &state, &parsed)?;
            similar_ids = ids;
            similarity_scores = scores;
            if !similar_ids.is_empty() {
                opts.similar_asset_ids = Some(&similar_ids);
            }
        }

        let has_similarity;
        #[cfg(feature = "ai")]
        { has_similarity = parsed.similar.is_some() && !similarity_scores.is_empty(); }
        #[cfg(not(feature = "ai"))]
        { has_similarity = false; }

        let per_page = if has_similarity { u32::MAX } else { state.per_page };
        let effective_sort = if has_similarity && params.sort.is_none() { "similarity_desc" } else { &sort_str };
        opts.sort = SearchSort::from_str(effective_sort);
        opts.page = if has_similarity { 1 } else { page };
        opts.per_page = per_page;
        opts.collapse_stacks = collapse_stacks;

        let (rows, total) = catalog.search_paginated_with_count(&opts)?;
        let display_per_page = state.per_page;
        let total_pages = if has_similarity { 1 } else { ((total as f64) / display_per_page as f64).ceil() as u32 };
        let mut cards: Vec<AssetCard> = rows.iter().map(|r| AssetCard::from_row(r, &preview_ext)).collect();

        #[cfg(feature = "ai")]
        if !similarity_scores.is_empty() {
            for card in &mut cards {
                card.similarity = similarity_scores.get(&card.asset_id).copied();
            }
            if matches!(opts.sort, SearchSort::SimilarityDesc | SearchSort::SimilarityAsc) {
                cards.sort_by(|a, b| {
                    let sa = a.similarity.unwrap_or(0.0);
                    let sb = b.similarity.unwrap_or(0.0);
                    if matches!(opts.sort, SearchSort::SimilarityAsc) {
                        sa.partial_cmp(&sb).unwrap_or(std::cmp::Ordering::Equal)
                    } else {
                        sb.partial_cmp(&sa).unwrap_or(std::cmp::Ordering::Equal)
                    }
                });
            }
        }

        link_cards(&mut cards);

        if is_htmx {
            let tmpl = ResultsPartial {
                query: query.to_string(),
                asset_type: asset_type.to_string(),
                tag: tag.to_string(),
                format_filter: format_filter.to_string(),
                volume: volume.to_string(),
                rating: rating_str.to_string(),
                label: label_str.to_string(),
                collection: collection_str.to_string(),
                person: person_str.to_string(),
                path: path_str.to_string(),
                sort: effective_sort.to_string(),
                cards,
                total,
                page,
                per_page,
                total_pages,
                collapse_stacks,
                has_similarity,
            };
            return Ok::<_, anyhow::Error>(tmpl.render()?);
        }

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

        #[cfg(feature = "ai")]
        let all_people: Vec<PersonOption> = state.dropdown_cache.get_people(&catalog)
            .into_iter()
            .map(|(id, name)| PersonOption { id, name })
            .collect();
        #[cfg(not(feature = "ai"))]
        let all_people: Vec<PersonOption> = Vec::new();

        let saved_searches = crate::saved_search::load(&state.catalog_root)
            .unwrap_or_default()
            .searches
            .into_iter()
            .filter(|ss| ss.favorite)
            .map(|ss| {
                let url_params = ss.to_url_params();
                SavedSearchChip {
                    name: ss.name,
                    url_params,
                }
            })
            .collect();

        let tmpl = BrowsePage {
            query: query.to_string(),
            asset_type: asset_type.to_string(),
            tag: tag.to_string(),
            format_filter: format_filter.to_string(),
            volume: volume.to_string(),
            rating: rating_str.to_string(),
            label: label_str.to_string(),
            sort: effective_sort.to_string(),
            cards,
            total,
            page,
            per_page,
            total_pages,
            all_tags,
            format_groups,
            all_volumes,
            all_collections,
            all_people,
            collection: collection_str.to_string(),
            path: path_str.to_string(),
            person: person_str.to_string(),
            saved_searches,
            collapse_stacks,
            ai_enabled: state.ai_enabled,
            vlm_enabled: state.vlm_enabled,
            vlm_models: state.vlm_config.available_models(),
            default_filter: state.default_filter.clone().unwrap_or_default(),
            default_filter_active: !nodefault && state.default_filter.is_some(),
            has_similarity,
        };
        Ok::<_, anyhow::Error>(tmpl.render()?)
    })
    .await;

    match result {
        Ok(Ok(html)) => {
            let mut resp = Html(html).into_response();
            resp.headers_mut().insert(
                axum::http::header::CACHE_CONTROL,
                axum::http::HeaderValue::from_static("no-store"),
            );
            resp
        }
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e:#}")).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
    }
}

/// GET /api/search — redirects non-htmx requests, returns partial for htmx.
pub async fn search_api(
    State(state): State<Arc<AppState>>,
    Query(params): Query<SearchParams>,
    headers: HeaderMap,
    uri: Uri,
) -> Response {
    if headers.get("HX-Request").is_none() {
        let query_string = uri.query().unwrap_or("");
        let redirect_url = if query_string.is_empty() {
            "/".to_string()
        } else {
            format!("/?{query_string}")
        };
        return axum::response::Redirect::to(&redirect_url).into_response();
    }

    let preview_ext = state.preview_ext.clone();
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let catalog = state.catalog()?;

        let bf = build_parsed_search(&params, &state);
        let parsed = bf.parsed;
        let volume = bf.volume;
        let path_volume_id = bf.path_volume_id;
        let sort_str = bf.sort_str;
        let page = bf.page;
        let collapse_stacks = bf.collapse_stacks;
        let query = bf.query;
        let asset_type = bf.asset_type;
        let tag = bf.tag;
        let format_filter = bf.format_filter;
        let rating_str = bf.rating;
        let label_str = bf.label;
        let collection_str = bf.collection;
        let person_str = bf.person;
        let path_str = bf.path;

        let mut opts = parsed.to_search_options();
        if !volume.is_empty() {
            opts.volume = Some(&volume);
        }
        if let Some(ref vid) = path_volume_id {
            if opts.volume.is_none() {
                opts.volume = Some(vid);
            }
        }

        let collection_ids;
        if !parsed.collections.is_empty() {
            collection_ids = resolve_collection_ids(&parsed.collections, catalog.conn());
            opts.collection_asset_ids = Some(&collection_ids);
        }

        let collection_exclude_ids;
        if !parsed.collections_exclude.is_empty() {
            collection_exclude_ids = resolve_collection_ids(&parsed.collections_exclude, catalog.conn());
            opts.collection_exclude_ids = Some(&collection_exclude_ids);
        }

        let person_ids;
        if !parsed.persons.is_empty() {
            #[cfg(feature = "ai")]
            {
                let face_store = crate::face_store::FaceStore::new(catalog.conn());
                person_ids = intersect_name_groups(&parsed.persons, |name| {
                    face_store.find_person_asset_ids(name).unwrap_or_default()
                });
                opts.person_asset_ids = Some(&person_ids);
            }
            #[cfg(not(feature = "ai"))]
            {
                person_ids = Vec::<String>::new();
                opts.person_asset_ids = Some(&person_ids);
            }
        }

        #[cfg(feature = "ai")]
        let text_query_ids;
        #[cfg(feature = "ai")]
        if let Some(ref text_q) = parsed.text_query {
            let model_id = &state.ai_config.model;
            let spec = crate::ai::get_model_spec(model_id);
            if let Some(spec) = spec {
                let model_dir = super::ai::resolve_model_dir(&state.ai_config);
                let mut model_guard = state.ai_model.blocking_lock();
                if model_guard.is_none() {
                    if let Ok(m) = crate::ai::SigLipModel::load_with_provider(
                        &model_dir, model_id, state.verbosity, &state.ai_config.execution_provider,
                    ) {
                        *model_guard = Some(m);
                    }
                }
                if let Some(ref mut model) = *model_guard {
                    if let Ok(embs) = model.encode_texts(&[text_q.clone()]) {
                        let query_emb = &embs[0];
                        let needs_load = state.ai_embedding_index.read().unwrap().is_none();
                        if needs_load {
                            if let Ok(index) = crate::embedding_store::EmbeddingIndex::load(
                                catalog.conn(), model_id, spec.embedding_dim,
                            ) {
                                *state.ai_embedding_index.write().unwrap() = Some(index);
                            }
                        }
                        let results = {
                            let idx_guard = state.ai_embedding_index.read().unwrap();
                            if let Some(ref idx) = *idx_guard {
                                idx.search(query_emb, parsed.text_query_limit.unwrap_or(state.ai_config.text_limit), None)
                            } else {
                                Vec::new()
                            }
                        };
                        text_query_ids = results.into_iter().map(|(id, _)| id).collect::<Vec<_>>();
                        opts.text_search_ids = Some(&text_query_ids);
                    }
                }
            }
        }

        #[cfg(feature = "ai")]
        let similar_ids;
        #[cfg(feature = "ai")]
        let similarity_scores: std::collections::HashMap<String, f32>;
        #[cfg(feature = "ai")]
        {
            let (ids, scores) = resolve_similar_filter(&catalog, &state, &parsed)?;
            similar_ids = ids;
            similarity_scores = scores;
            if !similar_ids.is_empty() {
                opts.similar_asset_ids = Some(&similar_ids);
            }
        }

        let has_similarity;
        #[cfg(feature = "ai")]
        { has_similarity = parsed.similar.is_some() && !similarity_scores.is_empty(); }
        #[cfg(not(feature = "ai"))]
        { has_similarity = false; }

        let per_page = if has_similarity { u32::MAX } else { state.per_page };
        let effective_sort = if has_similarity && params.sort.is_none() { "similarity_desc" } else { &sort_str };
        opts.sort = SearchSort::from_str(effective_sort);
        opts.page = if has_similarity { 1 } else { page };
        opts.per_page = per_page;
        opts.collapse_stacks = collapse_stacks;

        let (rows, total) = catalog.search_paginated_with_count(&opts)?;
        let display_per_page = state.per_page;
        let total_pages = if has_similarity { 1 } else { ((total as f64) / display_per_page as f64).ceil() as u32 };
        let mut cards: Vec<AssetCard> = rows.iter().map(|r| AssetCard::from_row(r, &preview_ext)).collect();

        #[cfg(feature = "ai")]
        if !similarity_scores.is_empty() {
            for card in &mut cards {
                card.similarity = similarity_scores.get(&card.asset_id).copied();
            }
            if matches!(opts.sort, SearchSort::SimilarityDesc | SearchSort::SimilarityAsc) {
                cards.sort_by(|a, b| {
                    let sa = a.similarity.unwrap_or(0.0);
                    let sb = b.similarity.unwrap_or(0.0);
                    if matches!(opts.sort, SearchSort::SimilarityAsc) {
                        sa.partial_cmp(&sb).unwrap_or(std::cmp::Ordering::Equal)
                    } else {
                        sb.partial_cmp(&sa).unwrap_or(std::cmp::Ordering::Equal)
                    }
                });
            }
        }

        link_cards(&mut cards);

        let tmpl = ResultsPartial {
            query,
            asset_type,
            tag,
            format_filter,
            volume: volume.to_string(),
            rating: rating_str,
            label: label_str,
            collection: collection_str,
            person: person_str,
            path: path_str,
            sort: effective_sort.to_string(),
            cards,
            total,
            page,
            per_page,
            total_pages,
            collapse_stacks,
            has_similarity,
        };
        Ok::<_, anyhow::Error>(tmpl.render()?)
    })
    .await;

    match result {
        Ok(Ok(html)) => Html(html).into_response(),
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e:#}")).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
    }
}

/// GET /api/page-ids — returns asset IDs for a given page.
pub async fn page_ids_api(
    State(state): State<Arc<AppState>>,
    Query(params): Query<SearchParams>,
) -> Response {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let catalog = state.catalog()?;

        let bf = build_parsed_search(&params, &state);
        let parsed = bf.parsed;
        let volume = bf.volume;
        let path_volume_id = bf.path_volume_id;
        let sort_str = bf.sort_str;
        let page = bf.page;
        let collapse_stacks = bf.collapse_stacks;

        let mut opts = parsed.to_search_options();
        if !volume.is_empty() {
            opts.volume = Some(&volume);
        }
        if let Some(ref vid) = path_volume_id {
            if opts.volume.is_none() {
                opts.volume = Some(vid);
            }
        }

        let collection_ids;
        if !parsed.collections.is_empty() {
            collection_ids = resolve_collection_ids(&parsed.collections, catalog.conn());
            opts.collection_asset_ids = Some(&collection_ids);
        }

        let collection_exclude_ids;
        if !parsed.collections_exclude.is_empty() {
            collection_exclude_ids = resolve_collection_ids(&parsed.collections_exclude, catalog.conn());
            opts.collection_exclude_ids = Some(&collection_exclude_ids);
        }

        let person_ids;
        if !parsed.persons.is_empty() {
            #[cfg(feature = "ai")]
            {
                let face_store = crate::face_store::FaceStore::new(catalog.conn());
                person_ids = intersect_name_groups(&parsed.persons, |name| {
                    face_store.find_person_asset_ids(name).unwrap_or_default()
                });
                opts.person_asset_ids = Some(&person_ids);
            }
            #[cfg(not(feature = "ai"))]
            {
                person_ids = Vec::<String>::new();
                opts.person_asset_ids = Some(&person_ids);
            }
        }

        let per_page = state.per_page;
        opts.sort = SearchSort::from_str(&sort_str);
        opts.page = page;
        opts.per_page = per_page;
        opts.collapse_stacks = collapse_stacks;

        let total = catalog.search_count(&opts)?;
        let total_pages = ((total as f64) / per_page as f64).ceil() as u32;
        let rows = catalog.search_paginated(&opts)?;
        let ids: Vec<String> = rows.iter().map(|r| r.asset_id.clone()).collect();

        Ok::<_, anyhow::Error>(serde_json::json!({
            "ids": ids,
            "page": page,
            "total_pages": total_pages,
        }))
    })
    .await;

    match result {
        Ok(Ok(json)) => Json(json).into_response(),
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e:#}")).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
    }
}

#[derive(Debug, serde::Deserialize)]
pub struct AssetPageParams {
    pub prev: Option<String>,
    pub next: Option<String>,
}

/// GET /asset/{id} — asset detail page.
pub async fn asset_page(
    State(state): State<Arc<AppState>>,
    Path(asset_id): Path<String>,
    Query(nav_params): Query<AssetPageParams>,
) -> Response {
    let preview_ext = state.preview_ext.clone();
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let catalog = state.catalog()?;

        let full_id = catalog
            .resolve_asset_id(&asset_id)?
            .ok_or_else(|| anyhow::anyhow!("no asset found matching '{asset_id}'"))?;
        let details = catalog
            .load_asset_details(&full_id)?
            .ok_or_else(|| anyhow::anyhow!("asset '{full_id}' not found in catalog"))?;

        let preview_gen = state.preview_generator();
        let best_hash = resolve_best_variant_idx(&catalog, &full_id, &details.variants)
            .ok()
            .map(|i| details.variants[i].content_hash.clone());
        let preview_url = best_hash.as_ref().and_then(|h| {
            if preview_gen.has_preview(h) {
                Some(crate::web::templates::preview_url(h, &preview_ext))
            } else {
                None
            }
        });
        let has_smart_preview = best_hash.as_ref().map_or(false, |h| preview_gen.has_smart_preview(h));
        let smart_preview_url = best_hash.as_ref().and_then(|h| {
            if has_smart_preview {
                Some(crate::web::templates::smart_preview_url(h, &preview_ext))
            } else {
                None
            }
        });

        let col_store = crate::collection::CollectionStore::new(catalog.conn());
        let collections = col_store.collections_for_asset(&full_id).unwrap_or_default();

        let stack_store = crate::stack::StackStore::new(catalog.conn());
        let (stack_members, is_stack_pick) = match stack_store.stack_for_asset(&full_id).unwrap_or(None) {
            Some((_sid, member_ids)) => {
                let is_pick = member_ids.first().map_or(false, |id| id == &full_id);
                let mut cards = Vec::new();
                for (i, mid) in member_ids.iter().enumerate() {
                    let name = catalog.get_asset_name(mid).unwrap_or(None)
                        .unwrap_or_else(|| mid[..8.min(mid.len())].to_string());
                    let hash = catalog.get_asset_best_variant_hash(mid).unwrap_or(None);
                    let purl = hash.map(|h| crate::web::templates::preview_url(&h, &preview_ext))
                        .unwrap_or_default();
                    cards.push(StackMemberCard {
                        asset_id: mid.clone(),
                        display_name: name,
                        preview_url: purl,
                        is_pick: i == 0,
                    });
                }
                (cards, is_pick)
            }
            None => (Vec::new(), false),
        };

        let registry = DeviceRegistry::new(&state.catalog_root);
        let volume_online: std::collections::HashMap<String, bool> = registry
            .list()
            .unwrap_or_default()
            .iter()
            .map(|v| (v.id.to_string(), v.is_online))
            .collect();

        let (faces, all_people_detail) = {
            #[cfg(feature = "ai")]
            {
                let face_store = crate::face_store::FaceStore::new(catalog.conn());
                let stored_faces = face_store.faces_for_asset(&full_id).unwrap_or_default();
                let face_rows: Vec<FaceRow> = stored_faces.iter().map(|f| {
                    let crop_url = if crate::face::face_crop_exists(&f.id, &state.catalog_root) {
                        Some(format!("/face/{}/{}.jpg", &f.id[..2.min(f.id.len())], f.id))
                    } else {
                        None
                    };
                    let person_name = f.person_id.as_ref().and_then(|pid| {
                        face_store.get_person(pid).ok().flatten().map(|p| {
                            p.name.unwrap_or_else(|| format!("Unknown ({})", &pid[..8.min(pid.len())]))
                        })
                    });
                    FaceRow {
                        face_id: f.id.clone(),
                        crop_url,
                        confidence_pct: (f.confidence * 100.0) as u32,
                        person_name,
                        person_id: f.person_id.clone(),
                    }
                }).collect();
                let people: Vec<PersonOption> = state.dropdown_cache.get_people(&catalog)
                    .into_iter()
                    .map(|(id, name)| PersonOption { id, name })
                    .collect();
                (face_rows, people)
            }
            #[cfg(not(feature = "ai"))]
            {
                (Vec::<FaceRow>::new(), Vec::<PersonOption>::new())
            }
        };

        let mut tmpl = AssetPage::from_details(details, preview_url, smart_preview_url, has_smart_preview, collections, stack_members, is_stack_pick, &volume_online, best_hash.unwrap_or_default());
        tmpl.prev_id = nav_params.prev;
        tmpl.next_id = nav_params.next;
        tmpl.ai_enabled = state.ai_enabled;
        tmpl.vlm_enabled = state.vlm_enabled;
        tmpl.vlm_models = state.vlm_config.available_models();
        tmpl.faces = faces;
        tmpl.all_people = all_people_detail;
        Ok::<_, anyhow::Error>(tmpl.render()?)
    })
    .await;

    match result {
        Ok(Ok(html)) => Html(html).into_response(),
        Ok(Err(e)) => {
            let msg = format!("{e:#}");
            if msg.contains("no asset found") {
                (
                    StatusCode::NOT_FOUND,
                    Html(format!("<h1>Not Found</h1><p>{msg}</p>")),
                )
                    .into_response()
            } else {
                (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {msg}")).into_response()
            }
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
    }
}

#[derive(Debug, serde::Deserialize)]
pub struct FacetParams {
    pub q: Option<String>,
    #[serde(rename = "type")]
    pub asset_type: Option<String>,
    pub tag: Option<String>,
    pub format: Option<String>,
    pub volume: Option<String>,
    pub rating: Option<String>,
    pub label: Option<String>,
    pub collection: Option<String>,
    pub path: Option<String>,
    pub stacks: Option<String>,
    pub person: Option<String>,
    pub nodefault: Option<String>,
}

/// GET /api/facets — facet counts for the browse sidebar.
pub async fn facets_api(
    State(state): State<Arc<AppState>>,
    Query(params): Query<FacetParams>,
) -> Response {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let catalog = state.catalog()?;

        let search_params = SearchParams {
            q: params.q.clone(),
            asset_type: params.asset_type.clone(),
            tag: params.tag.clone(),
            format: params.format.clone(),
            volume: params.volume.clone(),
            rating: params.rating.clone(),
            label: params.label.clone(),
            collection: params.collection.clone(),
            path: params.path.clone(),
            person: params.person.clone(),
            sort: None,
            page: None,
            stacks: params.stacks.clone(),
            nodefault: params.nodefault.clone(),
        };
        let bf = build_parsed_search(&search_params, &state);
        let parsed = bf.parsed;
        let volume = bf.volume;
        let path_volume_id = bf.path_volume_id;
        let collapse_stacks = bf.collapse_stacks;

        let mut opts = parsed.to_search_options();
        if !volume.is_empty() {
            opts.volume = Some(&volume);
        }
        if let Some(ref vid) = path_volume_id {
            if opts.volume.is_none() {
                opts.volume = Some(vid);
            }
        }
        opts.collapse_stacks = collapse_stacks;

        let collection_ids;
        if !parsed.collections.is_empty() {
            collection_ids = resolve_collection_ids(&parsed.collections, catalog.conn());
            opts.collection_asset_ids = Some(&collection_ids);
        }

        let collection_exclude_ids;
        if !parsed.collections_exclude.is_empty() {
            collection_exclude_ids = resolve_collection_ids(&parsed.collections_exclude, catalog.conn());
            opts.collection_exclude_ids = Some(&collection_exclude_ids);
        }

        let person_ids;
        if !parsed.persons.is_empty() {
            #[cfg(feature = "ai")]
            {
                let face_store = crate::face_store::FaceStore::new(catalog.conn());
                person_ids = intersect_name_groups(&parsed.persons, |name| {
                    face_store.find_person_asset_ids(name).unwrap_or_default()
                });
                opts.person_asset_ids = Some(&person_ids);
            }
            #[cfg(not(feature = "ai"))]
            {
                person_ids = Vec::<String>::new();
                opts.person_asset_ids = Some(&person_ids);
            }
        }

        let facets = catalog.facet_counts(&opts)?;
        Ok::<_, anyhow::Error>(serde_json::json!(facets))
    })
    .await;

    match result {
        Ok(Ok(json)) => Json(json).into_response(),
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e:#}")).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
    }
}
