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

/// Pair of "would-be-larger if we relaxed this constraint" deltas surfaced
/// next to the result count on the browse page. Each component is the
/// number of *additional* matches that the corresponding URL flag flip
/// would expose; both are 0 when the relaxation wouldn't reveal anything.
///
/// One source of truth for both the field shape (template structs forward
/// these fields verbatim) and the default values (zero across the board
/// when no relaxation applies).
#[derive(Debug, Default, Clone, Copy)]
pub(super) struct CountDeltas {
    /// Extra matches if `&stacks=0` were set (currently-collapsed stacks
    /// would surface their hidden members). 0 when stacks aren't being
    /// collapsed.
    pub in_stacks: u64,
    /// Extra matches if `&nodefault=1` were set (the configured `[browse]
    /// default_filter` is excluding these). 0 when no default filter is
    /// active.
    pub filtered_by_default: u64,
}

/// Inputs the count-delta helper needs that aren't already on `opts`.
/// Bundled into a struct purely to keep the signature legible.
pub(super) struct DeltaContext<'a> {
    pub state: &'a AppState,
    pub params: &'a SearchParams,
    pub collection_ids: &'a [String],
    pub collection_exclude_ids: &'a [String],
    pub person_ids: &'a [String],
    pub volume: &'a str,
    pub path_volume_id: Option<&'a str>,
    pub effective_sort: &'a str,
    pub page: u32,
    pub per_page: u32,
    pub collapse_stacks: bool,
    pub base_total: u64,
    pub default_filter_active: bool,
    pub has_similarity: bool,
}

/// Compute both deltas in one call. The `in_stacks` portion is a cheap
/// `opts.collapse_stacks=false` toggle + `search_count`; the
/// `filtered_by_default` portion re-runs `build_parsed_search` with
/// `nodefault=1` forced (collection/person/volume IDs reused from the
/// live query — typical default filters don't reference those, and
/// re-deriving them here would duplicate ~30 LOC for no gain).
///
/// The opts is mutated and restored — caller should treat it as borrowed
/// across the call but unchanged afterwards.
pub(super) fn compute_count_deltas(
    catalog: &crate::catalog::Catalog,
    opts: &mut crate::catalog::SearchOptions<'_>,
    ctx: &DeltaContext<'_>,
) -> CountDeltas {
    if ctx.has_similarity {
        return CountDeltas::default();
    }
    let in_stacks: u64 = if ctx.collapse_stacks {
        opts.collapse_stacks = false;
        let n = catalog.search_count(opts).unwrap_or(ctx.base_total);
        opts.collapse_stacks = true;
        n.saturating_sub(ctx.base_total)
    } else {
        0
    };
    let filtered_by_default: u64 = if ctx.default_filter_active {
        let mut params_nd = ctx.params.clone();
        params_nd.nodefault = Some("1".to_string());
        let bf_nd = build_parsed_search(&params_nd, ctx.state);
        let parsed_nd = bf_nd.parsed;
        let mut opts_nd = parsed_nd.to_search_options();
        if !parsed_nd.collections.is_empty() {
            opts_nd.collection_asset_ids = Some(ctx.collection_ids);
        }
        if !parsed_nd.collections_exclude.is_empty() {
            opts_nd.collection_exclude_ids = Some(ctx.collection_exclude_ids);
        }
        if !parsed_nd.persons.is_empty() {
            opts_nd.person_asset_ids = Some(ctx.person_ids);
        }
        if !ctx.volume.is_empty() {
            opts_nd.volume = Some(ctx.volume);
        }
        if let Some(vid) = ctx.path_volume_id {
            if opts_nd.volume.is_none() {
                opts_nd.volume = Some(vid);
            }
        }
        opts_nd.sort = SearchSort::from_str(ctx.effective_sort);
        opts_nd.page = ctx.page;
        opts_nd.per_page = ctx.per_page;
        opts_nd.collapse_stacks = ctx.collapse_stacks;
        let n = catalog.search_count(&opts_nd).unwrap_or(ctx.base_total);
        n.saturating_sub(ctx.base_total)
    } else {
        0
    };
    CountDeltas { in_stacks, filtered_by_default }
}

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

        // Default-initialize the lookup vecs so the count-delta helper below
        // can take slices unconditionally. (Pre-existing sparse-init pattern
        // relied on these only being accessed when populated.)
        let collection_ids: Vec<String> = if !parsed.collections.is_empty() {
            resolve_collection_ids(&parsed.collections, catalog.conn())
        } else { Vec::new() };
        if !collection_ids.is_empty() {
            opts.collection_asset_ids = Some(&collection_ids);
        }

        let collection_exclude_ids: Vec<String> = if !parsed.collections_exclude.is_empty() {
            resolve_collection_ids(&parsed.collections_exclude, catalog.conn())
        } else { Vec::new() };
        if !collection_exclude_ids.is_empty() {
            opts.collection_exclude_ids = Some(&collection_exclude_ids);
        }

        #[cfg(feature = "ai")]
        let person_ids: Vec<String> = if !parsed.persons.is_empty() {
            let face_store = crate::face_store::FaceStore::new(catalog.conn());
            intersect_name_groups(&parsed.persons, |name| {
                face_store.find_person_asset_ids(name).unwrap_or_default()
            })
        } else { Vec::new() };
        #[cfg(not(feature = "ai"))]
        let person_ids: Vec<String> = Vec::new();
        if !person_ids.is_empty() {
            opts.person_asset_ids = Some(&person_ids);
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

        // Count-delta hints: surface "more matches exist behind this view"
        // so the rendered count isn't silently smaller than what the tags
        // page would suggest. See `compute_count_deltas` for what each
        // delta means.
        let deltas = compute_count_deltas(&catalog, &mut opts, &DeltaContext {
            state: &state,
            params: &params,
            collection_ids: &collection_ids,
            collection_exclude_ids: &collection_exclude_ids,
            person_ids: &person_ids,
            volume: &volume,
            path_volume_id: path_volume_id.as_deref(),
            effective_sort,
            page,
            per_page,
            collapse_stacks,
            base_total: total,
            default_filter_active: !nodefault && state.default_filter.is_some(),
            has_similarity,
        });

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
                count_in_stacks: deltas.in_stacks,
                count_filtered_by_default: deltas.filtered_by_default,
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
            count_in_stacks: deltas.in_stacks,
            count_filtered_by_default: deltas.filtered_by_default,
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
        let nodefault = bf.nodefault;
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

        // Default-initialize the lookup vecs so the count-delta helper below
        // can take slices unconditionally. (Pre-existing sparse-init pattern
        // relied on these only being accessed when populated.)
        let collection_ids: Vec<String> = if !parsed.collections.is_empty() {
            resolve_collection_ids(&parsed.collections, catalog.conn())
        } else { Vec::new() };
        if !collection_ids.is_empty() {
            opts.collection_asset_ids = Some(&collection_ids);
        }

        let collection_exclude_ids: Vec<String> = if !parsed.collections_exclude.is_empty() {
            resolve_collection_ids(&parsed.collections_exclude, catalog.conn())
        } else { Vec::new() };
        if !collection_exclude_ids.is_empty() {
            opts.collection_exclude_ids = Some(&collection_exclude_ids);
        }

        #[cfg(feature = "ai")]
        let person_ids: Vec<String> = if !parsed.persons.is_empty() {
            let face_store = crate::face_store::FaceStore::new(catalog.conn());
            intersect_name_groups(&parsed.persons, |name| {
                face_store.find_person_asset_ids(name).unwrap_or_default()
            })
        } else { Vec::new() };
        #[cfg(not(feature = "ai"))]
        let person_ids: Vec<String> = Vec::new();
        if !person_ids.is_empty() {
            opts.person_asset_ids = Some(&person_ids);
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

        let deltas = compute_count_deltas(&catalog, &mut opts, &DeltaContext {
            state: &state,
            params: &params,
            collection_ids: &collection_ids,
            collection_exclude_ids: &collection_exclude_ids,
            person_ids: &person_ids,
            volume: &volume,
            path_volume_id: path_volume_id.as_deref(),
            effective_sort,
            page,
            per_page,
            collapse_stacks,
            base_total: total,
            default_filter_active: !nodefault && state.default_filter.is_some(),
            has_similarity,
        });

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
            count_in_stacks: deltas.in_stacks,
            count_filtered_by_default: deltas.filtered_by_default,
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

        // Default-initialize the lookup vecs so the count-delta helper below
        // can take slices unconditionally. (Pre-existing sparse-init pattern
        // relied on these only being accessed when populated.)
        let collection_ids: Vec<String> = if !parsed.collections.is_empty() {
            resolve_collection_ids(&parsed.collections, catalog.conn())
        } else { Vec::new() };
        if !collection_ids.is_empty() {
            opts.collection_asset_ids = Some(&collection_ids);
        }

        let collection_exclude_ids: Vec<String> = if !parsed.collections_exclude.is_empty() {
            resolve_collection_ids(&parsed.collections_exclude, catalog.conn())
        } else { Vec::new() };
        if !collection_exclude_ids.is_empty() {
            opts.collection_exclude_ids = Some(&collection_exclude_ids);
        }

        #[cfg(feature = "ai")]
        let person_ids: Vec<String> = if !parsed.persons.is_empty() {
            let face_store = crate::face_store::FaceStore::new(catalog.conn());
            intersect_name_groups(&parsed.persons, |name| {
                face_store.find_person_asset_ids(name).unwrap_or_default()
            })
        } else { Vec::new() };
        #[cfg(not(feature = "ai"))]
        let person_ids: Vec<String> = Vec::new();
        if !person_ids.is_empty() {
            opts.person_asset_ids = Some(&person_ids);
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

        let full_id = super::resolve_asset_id_or_err(&catalog, &asset_id)?;
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

        // Default-initialize the lookup vecs so the count-delta helper below
        // can take slices unconditionally. (Pre-existing sparse-init pattern
        // relied on these only being accessed when populated.)
        let collection_ids: Vec<String> = if !parsed.collections.is_empty() {
            resolve_collection_ids(&parsed.collections, catalog.conn())
        } else { Vec::new() };
        if !collection_ids.is_empty() {
            opts.collection_asset_ids = Some(&collection_ids);
        }

        let collection_exclude_ids: Vec<String> = if !parsed.collections_exclude.is_empty() {
            resolve_collection_ids(&parsed.collections_exclude, catalog.conn())
        } else { Vec::new() };
        if !collection_exclude_ids.is_empty() {
            opts.collection_exclude_ids = Some(&collection_exclude_ids);
        }

        #[cfg(feature = "ai")]
        let person_ids: Vec<String> = if !parsed.persons.is_empty() {
            let face_store = crate::face_store::FaceStore::new(catalog.conn());
            intersect_name_groups(&parsed.persons, |name| {
                face_store.find_person_asset_ids(name).unwrap_or_default()
            })
        } else { Vec::new() };
        #[cfg(not(feature = "ai"))]
        let person_ids: Vec<String> = Vec::new();
        if !person_ids.is_empty() {
            opts.person_asset_ids = Some(&person_ids);
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

// ─── Path autocomplete ──────────────────────────────────────────────

#[derive(Debug, serde::Deserialize)]
pub struct PathsParams {
    /// Prefix the user has typed so far. Matches against
    /// `file_locations.relative_path`. Empty or missing returns top-level
    /// segments across the catalogue.
    pub q: Option<String>,
    /// Optional volume ID to scope completions (string UUID).
    pub volume: Option<String>,
    /// Max number of completions to return (default 20, max 100).
    pub limit: Option<u32>,
}

/// GET /api/paths — hierarchical path completion for the filter bar.
///
/// Given a prefix `q`, returns the distinct set of next-segment completions
/// found in `file_locations.relative_path`. Directory completions carry a
/// trailing `/`, file completions don't — so the client can keep typing
/// after accepting a directory and have the next query fetch the next
/// level naturally.
///
/// Wildcard patterns (`*` in `q`) short-circuit to an empty list: the
/// filter already understands wildcards, so offering completions for a
/// pattern would be misleading.
pub async fn paths_api(
    State(state): State<Arc<AppState>>,
    Query(params): Query<PathsParams>,
) -> Response {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let raw_prefix = params.q.unwrap_or_default();
        // Wildcard short-circuit: the filter handles `*` patterns on the
        // server, and autocomplete would be nonsensical for them.
        if raw_prefix.contains('*') {
            return Ok::<_, anyhow::Error>(serde_json::json!([] as [String; 0]));
        }

        let limit = params.limit.unwrap_or(20).clamp(1, 100) as usize;
        let catalog = state.catalog()?;

        // Handle absolute paths: if the user typed a path starting with `/`
        // (or a Windows drive letter), try to match it against a volume
        // mount point. On match, strip the mount prefix and pin the
        // autocomplete to that volume. Mirrors what `normalize_path_for_search`
        // does for the search filter itself.
        let registry = crate::device_registry::DeviceRegistry::new(&state.catalog_root);
        let volumes = registry.list().unwrap_or_default();
        let (normalized_prefix, inferred_volume) =
            crate::query::normalize_path_for_search(&raw_prefix, &volumes, None);
        let volume_id = params
            .volume
            .filter(|s| !s.is_empty())
            .or(inferred_volume);

        // If the user typed an absolute path that didn't match any volume
        // mount, we'd only get false-positive matches from the LIKE query.
        // Short-circuit to empty.
        if raw_prefix != normalized_prefix
            && std::path::Path::new(&normalized_prefix).is_absolute()
        {
            return Ok(serde_json::json!([] as [String; 0]));
        }

        // Escape LIKE metacharacters so a path with `%` or `_` in it is
        // matched literally rather than as a pattern.
        let escaped = normalized_prefix
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_");
        let like_pattern = format!("{escaped}%");

        // Character count (not byte count) because SQLite's substr/instr
        // operate on characters for TEXT columns. `\.chars().count()` gives
        // us Unicode scalar values, which matches SQLite's semantics.
        let prefix_len = normalized_prefix.chars().count() as i64;

        let rows = path_completions_sql(
            catalog.conn(),
            &like_pattern,
            prefix_len,
            volume_id.as_deref(),
            limit as i64,
        )?;

        Ok(serde_json::json!(rows))
    })
    .await;

    match result {
        Ok(Ok(json)) => Json(json).into_response(),
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e:#}")).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
    }
}

/// Aggregate next-segment path completions for a prefix, deduping in SQL.
///
/// **Why SQL-side GROUP BY instead of fetch-then-dedupe in Rust**: a
/// row-limited sample would get monopolised by a single directory that
/// holds thousands of files — we'd fill the sample before seeing the
/// sibling directories. GROUP BY on the generated next-segment
/// expression collapses each directory to one row *before* the LIMIT
/// applies, so every sibling shows up regardless of how many files
/// lives under it.
///
/// The `CASE` expression yields either:
/// - the prefix + next `/`-terminated segment (directory completion, e.g.
///   `Pictures/2026/2026-04-18/`), or
/// - the whole path (file leaf, no trailing `/`, e.g.
///   `Pictures/2026/2026-04-18/DSC_0001.jpg`).
///
/// `prefix_len` is in characters (not bytes) because SQLite's
/// `substr()` and `instr()` operate on character positions for TEXT
/// columns. Rust callers pass `prefix.chars().count() as i64`.
pub(super) fn path_completions_sql(
    conn: &rusqlite::Connection,
    like_pattern: &str,
    prefix_len: i64,
    volume_id: Option<&str>,
    limit: i64,
) -> anyhow::Result<Vec<String>> {
    // The CASE expression computes the next-segment string for each row.
    // The inner `instr(substr(relative_path, ?len + 1), '/')` finds the
    // position of the next `/` after the prefix; if there is one, we
    // keep the prefix + everything through that `/`, otherwise the row
    // is a leaf file and we keep the whole path.
    let sql = if volume_id.is_some() {
        "SELECT CASE \
           WHEN instr(substr(relative_path, ?2 + 1), '/') > 0 \
             THEN substr(relative_path, 1, ?2 + instr(substr(relative_path, ?2 + 1), '/')) \
           ELSE relative_path \
         END AS completion \
         FROM file_locations \
         WHERE relative_path LIKE ?1 ESCAPE '\\' AND volume_id = ?3 \
         GROUP BY completion \
         ORDER BY completion \
         LIMIT ?4"
    } else {
        "SELECT CASE \
           WHEN instr(substr(relative_path, ?2 + 1), '/') > 0 \
             THEN substr(relative_path, 1, ?2 + instr(substr(relative_path, ?2 + 1), '/')) \
           ELSE relative_path \
         END AS completion \
         FROM file_locations \
         WHERE relative_path LIKE ?1 ESCAPE '\\' \
         GROUP BY completion \
         ORDER BY completion \
         LIMIT ?3"
    };

    let mut stmt = conn.prepare(sql)?;
    let rows: Vec<String> = if let Some(vol) = volume_id {
        stmt.query_map(
            rusqlite::params![like_pattern, prefix_len, vol, limit],
            |r| r.get::<_, String>(0),
        )?
        .filter_map(Result::ok)
        .collect()
    } else {
        stmt.query_map(
            rusqlite::params![like_pattern, prefix_len, limit],
            |r| r.get::<_, String>(0),
        )?
        .filter_map(Result::ok)
        .collect()
    };
    Ok(rows)
}

#[cfg(test)]
mod path_completion_tests {
    use super::path_completions_sql;

    /// Spin up an in-memory SQLite with the minimal file_locations shape
    /// and seed it with the given paths (all on a fake volume).
    fn seed_conn(paths: &[&str]) -> rusqlite::Connection {
        seed_conn_multi_vol(&paths.iter().map(|p| (*p, "vol-a")).collect::<Vec<_>>())
    }

    fn seed_conn_multi_vol(rows: &[(&str, &str)]) -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute(
            "CREATE TABLE file_locations (\
               content_hash TEXT NOT NULL, \
               volume_id TEXT NOT NULL, \
               relative_path TEXT NOT NULL, \
               verified_at TEXT\
             )",
            [],
        )
        .unwrap();
        for (i, (path, vol)) in rows.iter().enumerate() {
            conn.execute(
                "INSERT INTO file_locations (content_hash, volume_id, relative_path) \
                 VALUES (?1, ?2, ?3)",
                rusqlite::params![format!("sha256:{i}"), vol, path],
            )
            .unwrap();
        }
        conn
    }

    fn run(prefix: &str, conn: &rusqlite::Connection, limit: i64) -> Vec<String> {
        let like = format!("{}%", prefix);
        let plen = prefix.chars().count() as i64;
        path_completions_sql(conn, &like, plen, None, limit).unwrap()
    }

    #[test]
    fn extracts_unique_next_segments() {
        let conn = seed_conn(&[
            "Capture/2024/a.jpg",
            "Capture/2024/b.jpg",
            "Capture/2025/x.jpg",
            "Other/y.jpg",
        ]);
        assert_eq!(run("Capture/", &conn, 10), vec!["Capture/2024/", "Capture/2025/"]);
    }

    #[test]
    fn empty_prefix_returns_top_level() {
        let conn = seed_conn(&["Capture/a.jpg", "Other/b.jpg"]);
        assert_eq!(run("", &conn, 10), vec!["Capture/", "Other/"]);
    }

    #[test]
    fn leaf_files_have_no_trailing_slash() {
        let conn = seed_conn(&["Capture/2024/a.jpg", "Capture/2024/b.jpg"]);
        assert_eq!(
            run("Capture/2024/", &conn, 10),
            vec!["Capture/2024/a.jpg", "Capture/2024/b.jpg"]
        );
    }

    #[test]
    fn dedups_identical_next_segments() {
        // Hundreds of files under the same dir collapse to one dir entry.
        let paths: Vec<String> = (0..50).map(|i| format!("Capture/2024/file{i}.jpg")).collect();
        let slices: Vec<&str> = paths.iter().map(|s| s.as_str()).collect();
        let conn = seed_conn(&slices);
        assert_eq!(run("Capture/", &conn, 10), vec!["Capture/2024/"]);
    }

    /// Regression for the "dense first sibling hides later siblings" bug.
    /// Directory A has 5000 files, directory B has 1; both must show up.
    #[test]
    fn sibling_with_few_files_not_hidden_by_dense_neighbour() {
        let mut paths: Vec<String> = (0..5000).map(|i| format!("root/A/f{i:05}.jpg")).collect();
        paths.push("root/B/only.jpg".to_string());
        let slices: Vec<&str> = paths.iter().map(|s| s.as_str()).collect();
        let conn = seed_conn(&slices);
        assert_eq!(run("root/", &conn, 10), vec!["root/A/", "root/B/"]);
    }

    #[test]
    fn limit_truncates_results() {
        let paths: Vec<String> = (0..50).map(|i| format!("Capture/dir{i:03}/x.jpg")).collect();
        let slices: Vec<&str> = paths.iter().map(|s| s.as_str()).collect();
        let conn = seed_conn(&slices);
        let got = run("Capture/", &conn, 5);
        assert_eq!(got.len(), 5);
        assert_eq!(got[0], "Capture/dir000/");
        assert_eq!(got[4], "Capture/dir004/");
    }

    #[test]
    fn mixed_dirs_and_files_at_same_level() {
        let conn = seed_conn(&[
            "root/index.md",
            "root/assets/x.jpg",
            "root/assets/y.jpg",
            "root/readme.txt",
        ]);
        assert_eq!(
            run("root/", &conn, 10),
            vec!["root/assets/", "root/index.md", "root/readme.txt"]
        );
    }

    #[test]
    fn prefix_collision_does_not_match_sibling_root() {
        // "Capture/..." and "Capture2/..." both pass LIKE 'Capture%', but
        // with the slash in the prefix only the first belongs. The WHERE
        // clause (plus the prefix-based substr position) does the right
        // thing — Capture2 paths go into a different bucket that we skip.
        let conn = seed_conn(&["Capture/a.jpg", "Capture2/b.jpg"]);
        assert_eq!(run("Capture/", &conn, 10), vec!["Capture/a.jpg"]);
    }

    #[test]
    fn volume_scope_filters_completions() {
        let conn = seed_conn_multi_vol(&[
            ("Capture/A/x.jpg", "vol-a"),
            ("Capture/B/y.jpg", "vol-b"),
        ]);
        let plen = "Capture/".chars().count() as i64;
        let got_a = path_completions_sql(&conn, "Capture/%", plen, Some("vol-a"), 10).unwrap();
        let got_b = path_completions_sql(&conn, "Capture/%", plen, Some("vol-b"), 10).unwrap();
        assert_eq!(got_a, vec!["Capture/A/"]);
        assert_eq!(got_b, vec!["Capture/B/"]);
    }

    #[test]
    fn unicode_prefix_with_multibyte_chars() {
        // `München` in the prefix uses multi-byte UTF-8 — the SQL needs
        // character-count positions (via `.chars().count()`), not bytes,
        // or substr/instr would slice mid-codepoint.
        let conn = seed_conn(&[
            "location/München/bridge.jpg",
            "location/München/tower.jpg",
            "location/Köln/dom.jpg",
        ]);
        assert_eq!(
            run("location/München/", &conn, 10),
            vec!["location/München/bridge.jpg", "location/München/tower.jpg"]
        );
    }
}
