use std::sync::Arc;

use askama::Template;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{Html, IntoResponse, Response};
use axum::{Form, Json};

use crate::catalog::SearchSort;
use crate::query::{normalize_path_for_search, parse_search_query};

use crate::device_registry::DeviceRegistry;

use super::templates::{
    format_size, link_cards, AssetCard, AssetPage, BackupPage, BrowsePage, CollectionOption,
    CompareAsset, ComparePage, DescriptionFragment, DuplicatesPage, FormatOption, LabelFragment,
    NameFragment, PreviewFragment, RatingFragment, ResultsPartial, SavedSearchChip,
    SavedSearchEntry, SavedSearchesPage, StackMemberCard, StatsPage, TagOption, TagTreeEntry,
    TagsFragment, TagsPage, VolumeOption,
};
use super::AppState;

#[derive(Debug, serde::Deserialize)]
pub struct SearchParams {
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
    pub sort: Option<String>,
    pub page: Option<u32>,
    pub stacks: Option<String>,
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

        let query = params.q.as_deref().unwrap_or("");
        let asset_type = params.asset_type.as_deref().unwrap_or("");
        let tag = params.tag.as_deref().unwrap_or("");
        let format = params.format.as_deref().unwrap_or("");
        let volume = params.volume.as_deref().unwrap_or("");
        let rating_str = params.rating.as_deref().unwrap_or("");
        let label_str = params.label.as_deref().unwrap_or("");
        let sort_str = params.sort.as_deref().unwrap_or("date_desc");
        let page = params.page.unwrap_or(1).max(1);

        let collection_str = params.collection.as_deref().unwrap_or("");
        let path_str = params.path.as_deref().unwrap_or("");
        let collapse_stacks = params.stacks.as_deref().unwrap_or("1") == "1";

        // Normalize absolute path → volume-relative + implicit volume filter
        let (normalized_path, path_volume_id) = if !path_str.is_empty() {
            let registry = DeviceRegistry::new(&state.catalog_root);
            let vols = registry.list().unwrap_or_default();
            normalize_path_for_search(path_str, &vols, None)
        } else {
            (String::new(), None)
        };

        let parsed = merge_search_params(query, asset_type, tag, format, rating_str, label_str);
        let mut opts = parsed.to_search_options();
        if !volume.is_empty() {
            opts.volume = Some(volume);
        }
        if let Some(ref vid) = path_volume_id {
            if opts.volume.is_none() {
                opts.volume = Some(vid);
            }
        }
        if !normalized_path.is_empty() {
            opts.path_prefix = Some(&normalized_path);
        }

        // Resolve collection filter
        let collection_ids;
        if !collection_str.is_empty() {
            let col_store = crate::collection::CollectionStore::new(catalog.conn());
            collection_ids = col_store.asset_ids_for_collection(collection_str)
                .unwrap_or_default();
            opts.collection_asset_ids = Some(&collection_ids);
        }

        opts.sort = SearchSort::from_str(sort_str);
        opts.page = page;
        opts.per_page = 60;
        opts.collapse_stacks = collapse_stacks;

        let total = catalog.search_count(&opts)?;
        let rows = catalog.search_paginated(&opts)?;
        let total_pages = ((total as f64) / 60.0).ceil() as u32;
        let mut cards: Vec<AssetCard> = rows.iter().map(|r| AssetCard::from_row(r, &preview_ext)).collect();
        link_cards(&mut cards);

        if is_htmx {
            let tmpl = ResultsPartial {
                query: query.to_string(),
                asset_type: asset_type.to_string(),
                tag: tag.to_string(),
                format_filter: format.to_string(),
                volume: volume.to_string(),
                rating: rating_str.to_string(),
                label: label_str.to_string(),
                collection: collection_str.to_string(),
                path: path_str.to_string(),
                sort: sort_str.to_string(),
                cards,
                total,
                page,
                per_page: 60,
                total_pages,
                collapse_stacks,
            };
            return Ok::<_, anyhow::Error>(tmpl.render()?);
        }

        let all_tags: Vec<TagOption> = state.dropdown_cache.get_tags(&catalog)
            .into_iter()
            .map(|(name, count)| TagOption { name, count })
            .collect();
        let all_formats: Vec<FormatOption> = state.dropdown_cache.get_formats(&catalog)
            .into_iter()
            .map(|name| FormatOption { name })
            .collect();
        let all_volumes: Vec<VolumeOption> = state.dropdown_cache.get_volumes(&catalog)
            .into_iter()
            .map(|(id, label)| VolumeOption { id, label })
            .collect();
        let all_collections: Vec<CollectionOption> = state.dropdown_cache.get_collections(&catalog)
            .into_iter()
            .map(|name| CollectionOption { name })
            .collect();

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
            format_filter: format.to_string(),
            volume: volume.to_string(),
            rating: rating_str.to_string(),
            label: label_str.to_string(),
            sort: sort_str.to_string(),
            cards,
            total,
            page,
            per_page: 60,
            total_pages,
            all_tags,
            all_formats,
            all_volumes,
            all_collections,
            collection: collection_str.to_string(),
            path: path_str.to_string(),
            saved_searches,
            collapse_stacks,
        };
        Ok::<_, anyhow::Error>(tmpl.render()?)
    })
    .await;

    match result {
        Ok(Ok(html)) => Html(html).into_response(),
        Ok(Err(e)) => {
            (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e:#}")).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
    }
}

/// GET /api/search — redirects non-htmx requests to browse page, returns partial for htmx.
pub async fn search_api(
    State(state): State<Arc<AppState>>,
    Query(params): Query<SearchParams>,
    headers: HeaderMap,
    uri: Uri,
) -> Response {
    // Non-htmx requests (direct browser load, reload, back button) get redirected
    // to the browse page which renders the full HTML with layout and CSS.
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

        let query = params.q.as_deref().unwrap_or("");
        let asset_type = params.asset_type.as_deref().unwrap_or("");
        let tag = params.tag.as_deref().unwrap_or("");
        let format = params.format.as_deref().unwrap_or("");
        let volume = params.volume.as_deref().unwrap_or("");
        let rating_str = params.rating.as_deref().unwrap_or("");
        let label_str = params.label.as_deref().unwrap_or("");
        let sort_str = params.sort.as_deref().unwrap_or("date_desc");
        let page = params.page.unwrap_or(1).max(1);

        let collection_str = params.collection.as_deref().unwrap_or("");
        let path_str = params.path.as_deref().unwrap_or("");
        let collapse_stacks = params.stacks.as_deref().unwrap_or("1") == "1";

        // Normalize absolute path → volume-relative + implicit volume filter
        let (normalized_path, path_volume_id) = if !path_str.is_empty() {
            let registry = DeviceRegistry::new(&state.catalog_root);
            let vols = registry.list().unwrap_or_default();
            normalize_path_for_search(path_str, &vols, None)
        } else {
            (String::new(), None)
        };

        let parsed = merge_search_params(query, asset_type, tag, format, rating_str, label_str);
        let mut opts = parsed.to_search_options();
        if !volume.is_empty() {
            opts.volume = Some(volume);
        }
        if let Some(ref vid) = path_volume_id {
            if opts.volume.is_none() {
                opts.volume = Some(vid);
            }
        }
        if !normalized_path.is_empty() {
            opts.path_prefix = Some(&normalized_path);
        }

        // Resolve collection filter
        let collection_ids;
        if !collection_str.is_empty() {
            let col_store = crate::collection::CollectionStore::new(catalog.conn());
            collection_ids = col_store.asset_ids_for_collection(collection_str)
                .unwrap_or_default();
            opts.collection_asset_ids = Some(&collection_ids);
        }

        opts.sort = SearchSort::from_str(sort_str);
        opts.page = page;
        opts.per_page = 60;
        opts.collapse_stacks = collapse_stacks;

        let total = catalog.search_count(&opts)?;
        let rows = catalog.search_paginated(&opts)?;
        let total_pages = ((total as f64) / 60.0).ceil() as u32;
        let mut cards: Vec<AssetCard> = rows.iter().map(|r| AssetCard::from_row(r, &preview_ext)).collect();
        link_cards(&mut cards);

        let tmpl = ResultsPartial {
            query: query.to_string(),
            asset_type: asset_type.to_string(),
            tag: tag.to_string(),
            format_filter: format.to_string(),
            volume: volume.to_string(),
            rating: rating_str.to_string(),
            label: label_str.to_string(),
            collection: collection_str.to_string(),
            path: path_str.to_string(),
            sort: sort_str.to_string(),
            cards,
            total,
            page,
            per_page: 60,
            total_pages,
            collapse_stacks,
        };
        Ok::<_, anyhow::Error>(tmpl.render()?)
    })
    .await;

    match result {
        Ok(Ok(html)) => Html(html).into_response(),
        Ok(Err(e)) => {
            (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e:#}")).into_response()
        }
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
        let engine = state.query_engine();
        let details = engine.show(&asset_id)?;

        let preview_gen = state.preview_generator();
        let best = crate::models::variant::best_preview_index_details(&details.variants);
        let preview_url = best.and_then(|i| {
            let v = &details.variants[i];
            if preview_gen.has_preview(&v.content_hash) {
                Some(super::templates::preview_url(&v.content_hash, &preview_ext))
            } else {
                None
            }
        });
        let best_hash = best.map(|i| details.variants[i].content_hash.clone());
        let has_smart_preview = best_hash.as_ref().map_or(false, |h| preview_gen.has_smart_preview(h));
        let smart_preview_url = best_hash.as_ref().and_then(|h| {
            if has_smart_preview {
                Some(super::templates::smart_preview_url(h, &preview_ext))
            } else {
                None
            }
        });

        // Load collections this asset belongs to
        let (collections, stack_members, is_stack_pick) = {
            let catalog = state.catalog()?;
            let col_store = crate::collection::CollectionStore::new(catalog.conn());
            let cols = col_store.collections_for_asset(&asset_id).unwrap_or_default();

            let stack_store = crate::stack::StackStore::new(catalog.conn());
            let (members, is_pick) = match stack_store.stack_for_asset(&asset_id).unwrap_or(None) {
                Some((_sid, member_ids)) => {
                    let is_pick = member_ids.first().map_or(false, |id| id == &asset_id);
                    let mut cards = Vec::new();
                    for (i, mid) in member_ids.iter().enumerate() {
                        if mid == &asset_id { continue; }
                        // Load minimal info for stack member
                        let name = catalog.get_asset_name(mid).unwrap_or(None)
                            .unwrap_or_else(|| mid[..8.min(mid.len())].to_string());
                        let hash = catalog.get_asset_best_variant_hash(mid).unwrap_or(None);
                        let purl = hash.map(|h| super::templates::preview_url(&h, &preview_ext))
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

            (cols, members, is_pick)
        };

        let mut tmpl = AssetPage::from_details(details, preview_url, smart_preview_url, has_smart_preview, collections, stack_members, is_stack_pick);
        tmpl.prev_id = nav_params.prev;
        tmpl.next_id = nav_params.next;
        Ok::<_, anyhow::Error>(tmpl.render()?)
    })
    .await;

    match result {
        Ok(Ok(html)) => Html(html).into_response(),
        Ok(Err(e)) => {
            let msg = format!("{e:#}");
            if msg.contains("No asset found") {
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
pub struct TagForm {
    pub tags: String,
}

#[derive(Debug, serde::Deserialize)]
pub struct RemoveTagQuery {
    pub tag: String,
}

/// POST /api/asset/{id}/tags — add tags, return tags fragment.
pub async fn add_tags(
    State(state): State<Arc<AppState>>,
    Path(asset_id): Path<String>,
    Form(form): Form<TagForm>,
) -> Response {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let engine = state.query_engine();
        let tags: Vec<String> = form
            .tags
            .split(',')
            .map(|t| crate::tag_util::tag_input_to_storage(t.trim()))
            .filter(|t| !t.is_empty())
            .collect();
        let result = engine.tag(&asset_id, &tags, false)?;
        state.dropdown_cache.invalidate_tags();
        let tmpl = TagsFragment {
            asset_id,
            tags: result.current_tags,
        };
        Ok::<_, anyhow::Error>(tmpl.render()?)
    })
    .await;

    match result {
        Ok(Ok(html)) => Html(html).into_response(),
        Ok(Err(e)) => {
            (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e:#}")).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
    }
}

/// DELETE /api/asset/{id}/tags?tag=... — remove tag, return tags fragment.
pub async fn remove_tag(
    State(state): State<Arc<AppState>>,
    Path(asset_id): Path<String>,
    Query(query): Query<RemoveTagQuery>,
) -> Response {
    let state = state.clone();
    let tag = query.tag;
    let result = tokio::task::spawn_blocking(move || {
        let engine = state.query_engine();
        let result = engine.tag(&asset_id, &[tag], true)?;
        state.dropdown_cache.invalidate_tags();
        let tmpl = TagsFragment {
            asset_id,
            tags: result.current_tags,
        };
        Ok::<_, anyhow::Error>(tmpl.render()?)
    })
    .await;

    match result {
        Ok(Ok(html)) => Html(html).into_response(),
        Ok(Err(e)) => {
            (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e:#}")).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
    }
}

/// GET /api/tags — all tags as JSON (for autocomplete).
pub async fn tags_api(State(state): State<Arc<AppState>>) -> Response {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let catalog = state.catalog()?;
        let tags = state.dropdown_cache.get_tags(&catalog);
        Ok::<_, anyhow::Error>(tags)
    })
    .await;

    match result {
        Ok(Ok(tags)) => axum::Json(tags).into_response(),
        Ok(Err(e)) => {
            (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e:#}")).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
    }
}

/// GET /api/stats — catalog stats as JSON.
pub async fn stats_api(State(state): State<Arc<AppState>>) -> Response {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let catalog = state.catalog()?;
        let (assets, variants, recipes, total_size) = catalog.stats_overview()?;
        Ok::<_, anyhow::Error>(serde_json::json!({
            "assets": assets,
            "variants": variants,
            "recipes": recipes,
            "total_size": total_size,
        }))
    })
    .await;

    match result {
        Ok(Ok(stats)) => axum::Json(stats).into_response(),
        Ok(Err(e)) => {
            (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e:#}")).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
    }
}

/// Build a tree of tag entries from a flat list of (name, count) pairs.
/// Ensures ancestor paths exist and computes total counts including descendants.
fn build_tag_tree(flat_tags: &[(String, u64)]) -> Vec<TagTreeEntry> {
    use std::collections::BTreeMap;

    // Collect own counts into a sorted map
    let mut own_counts: BTreeMap<String, u64> = BTreeMap::new();
    for (name, count) in flat_tags {
        own_counts.insert(name.clone(), *count);
    }

    // Ensure ancestor paths exist (e.g. "animals|birds|eagles" creates "animals" and "animals|birds")
    let names: Vec<String> = own_counts.keys().cloned().collect();
    for name in &names {
        if name.contains('|') {
            let mut prefix = String::new();
            for part in name.split('|') {
                if !prefix.is_empty() {
                    prefix.push('|');
                }
                prefix.push_str(part);
                if prefix != *name {
                    own_counts.entry(prefix.clone()).or_insert(0);
                }
            }
        }
    }

    // Compute total counts (own + all descendants)
    let sorted_names: Vec<String> = own_counts.keys().cloned().collect();
    let mut total_counts: BTreeMap<String, u64> = BTreeMap::new();
    for name in &sorted_names {
        let own = own_counts[name];
        total_counts.insert(name.clone(), own);
    }
    // Accumulate child counts into parents
    for name in sorted_names.iter().rev() {
        let total = total_counts[name];
        if let Some(pipe_pos) = name.rfind('|') {
            let parent = &name[..pipe_pos];
            if let Some(parent_total) = total_counts.get_mut(parent) {
                *parent_total += total;
            }
        }
    }

    // Determine which entries have children
    let mut has_children_set: std::collections::HashSet<String> = std::collections::HashSet::new();
    for name in &sorted_names {
        if let Some(pipe_pos) = name.rfind('|') {
            has_children_set.insert(name[..pipe_pos].to_string());
        }
    }

    // Flatten to Vec with depth; display uses `/` for hierarchy separator
    sorted_names
        .iter()
        .map(|name| {
            let depth = name.matches('|').count() as u32;
            let display = name
                .rsplit('|')
                .next()
                .unwrap_or(name)
                .to_string();
            let display_name = name.replace('|', "/");
            TagTreeEntry {
                name: name.clone(),
                display_name,
                display,
                depth,
                own_count: own_counts[name],
                total_count: total_counts[name],
                has_children: has_children_set.contains(name.as_str()),
            }
        })
        .collect()
}

/// GET /tags — tags HTML page.
pub async fn tags_page(State(state): State<Arc<AppState>>) -> Response {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let catalog = state.catalog()?;
        let tags = catalog.list_all_tags()?;
        let total_tags = tags.len() as u64;
        let tree = build_tag_tree(&tags);
        let tmpl = TagsPage {
            tags: tree,
            total_tags,
        };
        Ok::<_, anyhow::Error>(tmpl.render()?)
    })
    .await;

    match result {
        Ok(Ok(html)) => Html(html).into_response(),
        Ok(Err(e)) => {
            (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e:#}")).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
    }
}

/// GET /stats — stats HTML page.
pub async fn stats_page(State(state): State<Arc<AppState>>) -> Response {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let catalog = state.catalog()?;
        let registry = DeviceRegistry::new(&state.catalog_root);
        let vol_list = registry.list()?;
        let volumes_info: Vec<(String, String, bool, Option<String>)> = vol_list
            .iter()
            .map(|v| (v.label.clone(), v.id.to_string(), v.is_online, v.purpose.as_ref().map(|p| p.as_str().to_string())))
            .collect();

        let stats = catalog.build_stats(&volumes_info, true, true, true, true, 20)?;
        let total_size_fmt = format_size(stats.overview.total_size);

        let tmpl = StatsPage {
            stats,
            total_size_fmt,
        };
        Ok::<_, anyhow::Error>(tmpl.render()?)
    })
    .await;

    match result {
        Ok(Ok(html)) => Html(html).into_response(),
        Ok(Err(e)) => {
            (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e:#}")).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
    }
}

/// GET /backup — backup status dashboard.
pub async fn backup_page(State(state): State<Arc<AppState>>) -> Response {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let catalog = state.catalog()?;
        let registry = DeviceRegistry::new(&state.catalog_root);
        let vol_list = registry.list()?;
        let volumes_info: Vec<(String, String, bool, Option<String>)> = vol_list
            .iter()
            .map(|v| {
                (
                    v.label.clone(),
                    v.id.to_string(),
                    v.is_online,
                    v.purpose.as_ref().map(|p| p.as_str().to_string()),
                )
            })
            .collect();

        let backup = catalog.backup_status_overview(None, &volumes_info, 2, None)?;
        let total_assets_fmt = format!("{}", backup.total_assets);

        let tmpl = BackupPage {
            result: backup,
            total_assets_fmt,
        };
        Ok::<_, anyhow::Error>(tmpl.render()?)
    })
    .await;

    match result {
        Ok(Ok(html)) => Html(html).into_response(),
        Ok(Err(e)) => {
            (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e:#}")).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
    }
}

use crate::query::ParsedSearch;

/// Parse the `q` param through `parse_search_query` and overlay explicit dropdown params.
/// Returns a `ParsedSearch` (owned) that can be converted to `SearchOptions` by the caller.
fn merge_search_params(
    query: &str,
    asset_type: &str,
    tag: &str,
    format: &str,
    rating_str: &str,
    label: &str,
) -> ParsedSearch {
    let mut parsed = parse_search_query(query);

    // Explicit dropdown params override parsed values
    if !asset_type.is_empty() {
        parsed.asset_type = Some(asset_type.to_string());
    }
    if !tag.is_empty() {
        parsed.tag = Some(tag.to_string());
    }
    if !format.is_empty() {
        parsed.format = Some(format.to_string());
    }
    if !rating_str.is_empty() {
        let (rating_min, rating_exact) = parse_rating_filter(rating_str);
        if rating_min.is_some() {
            parsed.rating_min = rating_min;
            parsed.rating_exact = None;
        }
        if rating_exact.is_some() {
            parsed.rating_exact = rating_exact;
            parsed.rating_min = None;
        }
    }
    if !label.is_empty() {
        parsed.color_label = Some(label.to_string());
    }

    parsed
}

/// Parse a rating filter string into (rating_min, rating_exact).
/// "3+" → (Some(3), None), "5" → (None, Some(5)), "" → (None, None)
fn parse_rating_filter(s: &str) -> (Option<u8>, Option<u8>) {
    if s.is_empty() {
        return (None, None);
    }
    if let Some(num_str) = s.strip_suffix('+') {
        if let Ok(n) = num_str.parse::<u8>() {
            return (Some(n), None);
        }
    }
    if let Ok(n) = s.parse::<u8>() {
        return (None, Some(n));
    }
    (None, None)
}

#[derive(Debug, serde::Deserialize)]
pub struct RatingForm {
    pub rating: Option<u8>,
}

/// PUT /api/asset/{id}/rating — set rating, return rating fragment.
pub async fn set_rating(
    State(state): State<Arc<AppState>>,
    Path(asset_id): Path<String>,
    Form(form): Form<RatingForm>,
) -> Response {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let engine = state.query_engine();
        // Treat 0 as "clear rating"
        let rating = form.rating.filter(|&r| r > 0);
        let new_rating = engine.set_rating(&asset_id, rating)?;
        let tmpl = RatingFragment {
            asset_id,
            rating: new_rating,
        };
        Ok::<_, anyhow::Error>(tmpl.render()?)
    })
    .await;

    match result {
        Ok(Ok(html)) => Html(html).into_response(),
        Ok(Err(e)) => {
            (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e:#}")).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
    }
}

#[derive(Debug, serde::Deserialize)]
pub struct DescriptionForm {
    pub description: Option<String>,
}

/// PUT /api/asset/{id}/description — set description, return description fragment.
pub async fn set_description(
    State(state): State<Arc<AppState>>,
    Path(asset_id): Path<String>,
    Form(form): Form<DescriptionForm>,
) -> Response {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let engine = state.query_engine();
        // Treat empty string as "clear description"
        let description = form.description.filter(|s| !s.trim().is_empty());
        let new_desc = engine.set_description(&asset_id, description)?;
        let tmpl = DescriptionFragment {
            asset_id,
            description: new_desc,
        };
        Ok::<_, anyhow::Error>(tmpl.render()?)
    })
    .await;

    match result {
        Ok(Ok(html)) => Html(html).into_response(),
        Ok(Err(e)) => {
            (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e:#}")).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
    }
}

#[derive(Debug, serde::Deserialize)]
pub struct NameForm {
    pub name: Option<String>,
}

/// PUT /api/asset/{id}/name — set name, return name fragment.
pub async fn set_name(
    State(state): State<Arc<AppState>>,
    Path(asset_id): Path<String>,
    Form(form): Form<NameForm>,
) -> Response {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let engine = state.query_engine();
        // Treat empty string as "clear name"
        let name = form.name.filter(|s| !s.trim().is_empty());
        let new_name = engine.set_name(&asset_id, name)?;

        // Load fallback name from primary variant's filename
        let details = engine.show(&asset_id)?;
        let fallback_name = details
            .variants
            .first()
            .map(|v| v.original_filename.clone())
            .unwrap_or_else(|| "Untitled".to_string());

        let tmpl = NameFragment {
            asset_id,
            name: new_name,
            fallback_name,
        };
        Ok::<_, anyhow::Error>(tmpl.render()?)
    })
    .await;

    match result {
        Ok(Ok(html)) => Html(html).into_response(),
        Ok(Err(e)) => {
            (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e:#}")).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
    }
}

// --- Batch operations ---

#[derive(Debug, serde::Deserialize)]
pub struct BatchRatingRequest {
    pub asset_ids: Vec<String>,
    pub rating: Option<u8>,
}

#[derive(Debug, serde::Deserialize)]
pub struct BatchTagRequest {
    pub asset_ids: Vec<String>,
    pub tags: Vec<String>,
    pub remove: bool,
}

#[derive(Debug, serde::Serialize)]
pub struct BatchResult {
    pub succeeded: u32,
    pub failed: u32,
    pub errors: Vec<BatchError>,
}

#[derive(Debug, serde::Serialize)]
pub struct BatchError {
    pub asset_id: String,
    pub error: String,
}

/// PUT /api/batch/rating — set rating on multiple assets.
pub async fn batch_set_rating(
    State(state): State<Arc<AppState>>,
    Json(req): Json<BatchRatingRequest>,
) -> Response {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let engine = state.query_engine();
        let rating = req.rating.filter(|&r| r > 0);
        let mut succeeded = 0u32;
        let mut errors = Vec::new();
        for id in &req.asset_ids {
            match engine.set_rating(id, rating) {
                Ok(_) => succeeded += 1,
                Err(e) => errors.push(BatchError {
                    asset_id: id.clone(),
                    error: format!("{e:#}"),
                }),
            }
        }
        let failed = errors.len() as u32;
        Ok::<_, anyhow::Error>(BatchResult {
            succeeded,
            failed,
            errors,
        })
    })
    .await;

    match result {
        Ok(Ok(batch)) => Json(batch).into_response(),
        Ok(Err(e)) => {
            (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e:#}")).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
    }
}

/// POST /api/batch/tags — add or remove tags on multiple assets.
pub async fn batch_tags(
    State(state): State<Arc<AppState>>,
    Json(req): Json<BatchTagRequest>,
) -> Response {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let engine = state.query_engine();
        // Convert user-facing tag input to storage form
        let storage_tags: Vec<String> = req.tags.iter()
            .map(|t| crate::tag_util::tag_input_to_storage(t))
            .collect();
        let mut succeeded = 0u32;
        let mut errors = Vec::new();
        for id in &req.asset_ids {
            match engine.tag(id, &storage_tags, req.remove) {
                Ok(_) => succeeded += 1,
                Err(e) => errors.push(BatchError {
                    asset_id: id.clone(),
                    error: format!("{e:#}"),
                }),
            }
        }
        if succeeded > 0 {
            state.dropdown_cache.invalidate_tags();
        }
        let failed = errors.len() as u32;
        Ok::<_, anyhow::Error>(BatchResult {
            succeeded,
            failed,
            errors,
        })
    })
    .await;

    match result {
        Ok(Ok(batch)) => Json(batch).into_response(),
        Ok(Err(e)) => {
            (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e:#}")).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
    }
}

/// POST /api/asset/{id}/preview — generate preview, return preview fragment.
pub async fn generate_preview(
    State(state): State<Arc<AppState>>,
    Path(asset_id): Path<String>,
) -> Response {
    let preview_ext = state.preview_ext.clone();
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let engine = state.query_engine();
        let details = engine.show(&asset_id)?;

        let primary = details
            .variants
            .first()
            .ok_or_else(|| anyhow::anyhow!("Asset has no variants"))?;

        let content_hash = &primary.content_hash;
        let format = &primary.format;

        // Resolve the source file path from the first online location
        let registry = DeviceRegistry::new(&state.catalog_root);
        let volumes = registry.list()?;

        let source_path = primary
            .locations
            .iter()
            .find_map(|loc| {
                let vol = volumes.iter().find(|v| v.label == loc.volume_label)?;
                if !vol.is_online {
                    return None;
                }
                let path = vol.mount_point.join(&loc.relative_path);
                if path.exists() {
                    Some(path)
                } else {
                    None
                }
            })
            .ok_or_else(|| {
                anyhow::anyhow!("No online file location found for primary variant")
            })?;

        let preview_gen = state.preview_generator();
        preview_gen.regenerate(content_hash, &source_path, format)?;

        let preview_url = if preview_gen.has_preview(content_hash) {
            Some(super::templates::preview_url(content_hash, &preview_ext))
        } else {
            None
        };

        let best = crate::models::variant::best_preview_index_details(&details.variants);
        let best_hash = best.map(|i| details.variants[i].content_hash.clone());
        let has_smart = best_hash.as_ref().map_or(false, |h| preview_gen.has_smart_preview(h));
        let smart_url = best_hash.as_ref().and_then(|h| {
            if has_smart {
                Some(super::templates::smart_preview_url(h, &preview_ext))
            } else {
                None
            }
        });

        let tmpl = PreviewFragment {
            asset_id,
            primary_preview_url: preview_url,
            smart_preview_url: smart_url,
            has_smart_preview: has_smart,
        };
        Ok::<_, anyhow::Error>(tmpl.render()?)
    })
    .await;

    match result {
        Ok(Ok(html)) => Html(html).into_response(),
        Ok(Err(e)) => {
            (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e:#}")).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
    }
}

/// POST /api/asset/{id}/smart-preview — generate smart preview, return preview fragment.
pub async fn generate_smart_preview(
    State(state): State<Arc<AppState>>,
    Path(asset_id): Path<String>,
) -> Response {
    let preview_ext = state.preview_ext.clone();
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let engine = state.query_engine();
        let details = engine.show(&asset_id)?;

        let best_idx = crate::models::variant::best_preview_index_details(&details.variants)
            .ok_or_else(|| anyhow::anyhow!("Asset has no variants"))?;
        let variant = &details.variants[best_idx];
        let content_hash = &variant.content_hash;
        let format = &variant.format;

        // Resolve the source file path from the first online location
        let registry = DeviceRegistry::new(&state.catalog_root);
        let volumes = registry.list()?;

        let source_path = variant
            .locations
            .iter()
            .find_map(|loc| {
                let vol = volumes.iter().find(|v| v.label == loc.volume_label)?;
                if !vol.is_online {
                    return None;
                }
                let path = vol.mount_point.join(&loc.relative_path);
                if path.exists() {
                    Some(path)
                } else {
                    None
                }
            })
            .ok_or_else(|| {
                anyhow::anyhow!("No online file location found for variant")
            })?;

        let preview_gen = state.preview_generator();
        preview_gen.regenerate_smart(content_hash, &source_path, format)?;

        let preview_url = if preview_gen.has_preview(content_hash) {
            Some(super::templates::preview_url(content_hash, &preview_ext))
        } else {
            None
        };

        let has_smart = preview_gen.has_smart_preview(content_hash);
        let smart_url = if has_smart {
            Some(super::templates::smart_preview_url(content_hash, &preview_ext))
        } else {
            None
        };

        let tmpl = PreviewFragment {
            asset_id,
            primary_preview_url: preview_url,
            smart_preview_url: smart_url,
            has_smart_preview: has_smart,
        };
        Ok::<_, anyhow::Error>(tmpl.render()?)
    })
    .await;

    match result {
        Ok(Ok(html)) => Html(html).into_response(),
        Ok(Err(e)) => {
            (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e:#}")).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
    }
}

#[derive(Debug, serde::Deserialize)]
pub struct LabelForm {
    pub label: Option<String>,
}

/// PUT /api/asset/{id}/label — set color label, return label fragment.
pub async fn set_label(
    State(state): State<Arc<AppState>>,
    Path(asset_id): Path<String>,
    Form(form): Form<LabelForm>,
) -> Response {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let engine = state.query_engine();
        // Treat empty string as "clear label"
        let label_str = form.label.filter(|s| !s.trim().is_empty());
        let validated = match label_str {
            Some(ref s) => match crate::models::Asset::validate_color_label(s) {
                Ok(canonical) => canonical,
                Err(e) => return Err(anyhow::anyhow!(e)),
            },
            None => None,
        };
        let new_label = engine.set_color_label(&asset_id, validated)?;
        let tmpl = LabelFragment {
            asset_id,
            color_label: new_label,
        };
        Ok::<_, anyhow::Error>(tmpl.render()?)
    })
    .await;

    match result {
        Ok(Ok(html)) => Html(html).into_response(),
        Ok(Err(e)) => {
            (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e:#}")).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
    }
}

#[derive(Debug, serde::Deserialize)]
pub struct BatchLabelRequest {
    pub asset_ids: Vec<String>,
    pub label: Option<String>,
}

/// PUT /api/batch/label — set color label on multiple assets.
pub async fn batch_set_label(
    State(state): State<Arc<AppState>>,
    Json(req): Json<BatchLabelRequest>,
) -> Response {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let engine = state.query_engine();
        let label_str = req.label.filter(|s| !s.trim().is_empty());
        let validated = match label_str {
            Some(ref s) => match crate::models::Asset::validate_color_label(s) {
                Ok(canonical) => canonical,
                Err(e) => return Err(anyhow::anyhow!(e)),
            },
            None => None,
        };
        let mut succeeded = 0u32;
        let mut errors = Vec::new();
        for id in &req.asset_ids {
            match engine.set_color_label(id, validated.clone()) {
                Ok(_) => succeeded += 1,
                Err(e) => errors.push(BatchError {
                    asset_id: id.clone(),
                    error: format!("{e:#}"),
                }),
            }
        }
        let failed = errors.len() as u32;
        Ok::<_, anyhow::Error>(BatchResult {
            succeeded,
            failed,
            errors,
        })
    })
    .await;

    match result {
        Ok(Ok(batch)) => Json(batch).into_response(),
        Ok(Err(e)) => {
            (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e:#}")).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
    }
}

// --- Saved search API ---

/// GET /api/saved-searches — list all saved searches as JSON.
pub async fn list_saved_searches(State(state): State<Arc<AppState>>) -> Response {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let file = crate::saved_search::load(&state.catalog_root)?;
        Ok::<_, anyhow::Error>(file.searches)
    })
    .await;

    match result {
        Ok(Ok(searches)) => Json(searches).into_response(),
        Ok(Err(e)) => {
            (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e:#}")).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
    }
}

#[derive(Debug, serde::Deserialize)]
pub struct CreateSavedSearchRequest {
    pub name: String,
    pub query: String,
    pub sort: Option<String>,
    #[serde(default)]
    pub favorite: bool,
}

/// POST /api/saved-searches — create or update a saved search.
pub async fn create_saved_search(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateSavedSearchRequest>,
) -> Response {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let mut file = crate::saved_search::load(&state.catalog_root)?;
        let entry = crate::saved_search::SavedSearch {
            name: req.name.clone(),
            query: req.query,
            sort: req.sort,
            favorite: req.favorite,
        };
        if let Some(existing) = file.searches.iter_mut().find(|s| s.name == req.name) {
            *existing = entry;
        } else {
            file.searches.push(entry);
        }
        crate::saved_search::save(&state.catalog_root, &file)?;
        Ok::<_, anyhow::Error>(serde_json::json!({"status": "saved", "name": req.name}))
    })
    .await;

    match result {
        Ok(Ok(json)) => Json(json).into_response(),
        Ok(Err(e)) => {
            (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e:#}")).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
    }
}

/// DELETE /api/saved-searches/{name} — delete a saved search.
pub async fn delete_saved_search(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Response {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let mut file = crate::saved_search::load(&state.catalog_root)?;
        let before = file.searches.len();
        file.searches.retain(|s| s.name != name);
        if file.searches.len() == before {
            anyhow::bail!("No saved search named '{name}'");
        }
        crate::saved_search::save(&state.catalog_root, &file)?;
        Ok::<_, anyhow::Error>(serde_json::json!({"status": "deleted", "name": name}))
    })
    .await;

    match result {
        Ok(Ok(json)) => Json(json).into_response(),
        Ok(Err(e)) => {
            if format!("{e:#}").contains("No saved search") {
                (StatusCode::NOT_FOUND, format!("{e:#}")).into_response()
            } else {
                (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e:#}")).into_response()
            }
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
    }
}

// --- Collections ---

/// GET /collections — collections HTML page.
pub async fn collections_page(State(state): State<Arc<AppState>>) -> Response {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let catalog = state.catalog()?;
        let col_store = crate::collection::CollectionStore::new(catalog.conn());
        let collections = col_store.list()?;
        let tmpl = super::templates::CollectionsPage { collections };
        Ok::<_, anyhow::Error>(tmpl.render()?)
    })
    .await;

    match result {
        Ok(Ok(html)) => Html(html).into_response(),
        Ok(Err(e)) => {
            (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e:#}")).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
    }
}

#[derive(Debug, serde::Deserialize)]
pub struct CreateCollectionRequest {
    pub name: String,
    pub description: Option<String>,
}

/// POST /api/collections — create a new collection.
pub async fn create_collection_api(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateCollectionRequest>,
) -> Response {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let name = req.name.trim().to_string();
        if name.is_empty() {
            anyhow::bail!("Collection name cannot be empty");
        }
        let description = req.description.as_deref().map(|s| s.trim()).filter(|s| !s.is_empty());
        let catalog = state.catalog()?;
        let col_store = crate::collection::CollectionStore::new(catalog.conn());
        let collection = col_store.create(&name, description)?;
        // Persist to YAML
        let yaml = col_store.export_all()?;
        crate::collection::save_yaml(&state.catalog_root, &yaml)?;
        state.dropdown_cache.invalidate_collections();
        Ok::<_, anyhow::Error>(serde_json::json!({
            "id": collection.id.to_string(),
            "name": collection.name,
            "description": collection.description,
        }))
    })
    .await;

    match result {
        Ok(Ok(json)) => (StatusCode::CREATED, Json(json)).into_response(),
        Ok(Err(e)) => {
            let msg = format!("{e:#}");
            if msg.contains("UNIQUE constraint") || msg.contains("already exists") {
                (StatusCode::CONFLICT, format!("Collection already exists: {msg}")).into_response()
            } else {
                (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {msg}")).into_response()
            }
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
    }
}

/// GET /api/collections — list all collections as JSON.
pub async fn list_collections_api(State(state): State<Arc<AppState>>) -> Response {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let catalog = state.catalog()?;
        let col_store = crate::collection::CollectionStore::new(catalog.conn());
        let collections = col_store.list()?;
        Ok::<_, anyhow::Error>(collections)
    })
    .await;

    match result {
        Ok(Ok(collections)) => Json(collections).into_response(),
        Ok(Err(e)) => {
            (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e:#}")).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
    }
}

#[derive(Debug, serde::Deserialize)]
pub struct BatchCollectionRequest {
    pub asset_ids: Vec<String>,
    pub collection: String,
}

/// DELETE /api/batch/collection — remove assets from a collection.
pub async fn batch_remove_from_collection(
    State(state): State<Arc<AppState>>,
    Json(req): Json<BatchCollectionRequest>,
) -> Response {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let catalog = state.catalog()?;
        let col_store = crate::collection::CollectionStore::new(catalog.conn());
        let removed = col_store.remove_assets(&req.collection, &req.asset_ids)?;
        // Persist to YAML
        let yaml = col_store.export_all()?;
        crate::collection::save_yaml(&state.catalog_root, &yaml)?;
        state.dropdown_cache.invalidate_collections();
        Ok::<_, anyhow::Error>(serde_json::json!({
            "removed": removed,
            "collection": req.collection,
        }))
    })
    .await;

    match result {
        Ok(Ok(json)) => Json(json).into_response(),
        Ok(Err(e)) => {
            (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e:#}")).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
    }
}

/// POST /api/batch/collection — add assets to a collection.
pub async fn batch_add_to_collection(
    State(state): State<Arc<AppState>>,
    Json(req): Json<BatchCollectionRequest>,
) -> Response {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let catalog = state.catalog()?;
        let col_store = crate::collection::CollectionStore::new(catalog.conn());
        let added = col_store.add_assets(&req.collection, &req.asset_ids)?;
        // Persist to YAML
        let yaml = col_store.export_all()?;
        crate::collection::save_yaml(&state.catalog_root, &yaml)?;
        state.dropdown_cache.invalidate_collections();
        Ok::<_, anyhow::Error>(serde_json::json!({
            "added": added,
            "collection": req.collection,
        }))
    })
    .await;

    match result {
        Ok(Ok(json)) => Json(json).into_response(),
        Ok(Err(e)) => {
            (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e:#}")).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
    }
}

#[derive(serde::Deserialize)]
pub struct BatchAutoGroupRequest {
    pub asset_ids: Vec<String>,
}

/// POST /api/batch/auto-group — auto-group selected assets by filename stem.
pub async fn batch_auto_group(
    State(state): State<Arc<AppState>>,
    Json(req): Json<BatchAutoGroupRequest>,
) -> Response {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let engine = state.query_engine();
        let result = engine.auto_group(&req.asset_ids, false)?;
        Ok::<_, anyhow::Error>(serde_json::json!({
            "groups_merged": result.groups.len(),
            "donors_removed": result.total_donors_merged,
            "variants_moved": result.total_variants_moved,
        }))
    })
    .await;

    match result {
        Ok(Ok(json)) => Json(json).into_response(),
        Ok(Err(e)) => {
            (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e:#}")).into_response()
        }
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
        Ok(Err(e)) => {
            (StatusCode::BAD_REQUEST, format!("Error: {e:#}")).into_response()
        }
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
        Ok(Err(e)) => {
            (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e:#}")).into_response()
        }
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
        Ok(Err(e)) => {
            (StatusCode::BAD_REQUEST, format!("Error: {e:#}")).into_response()
        }
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
        Ok(Err(e)) => {
            (StatusCode::BAD_REQUEST, format!("Error: {e:#}")).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
    }
}

// --- Saved searches management ---

/// GET /saved-searches — saved searches management page.
pub async fn saved_searches_page(State(state): State<Arc<AppState>>) -> Response {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let file = crate::saved_search::load(&state.catalog_root)?;
        let searches: Vec<SavedSearchEntry> = file
            .searches
            .into_iter()
            .map(|ss| {
                let url_params = ss.to_url_params();
                let sort = ss.sort.as_deref().unwrap_or("date_desc").to_string();
                SavedSearchEntry {
                    name: ss.name,
                    query: ss.query,
                    sort,
                    favorite: ss.favorite,
                    url_params,
                }
            })
            .collect();
        let tmpl = SavedSearchesPage { searches };
        Ok::<_, anyhow::Error>(tmpl.render()?)
    })
    .await;

    match result {
        Ok(Ok(html)) => Html(html).into_response(),
        Ok(Err(e)) => {
            (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e:#}")).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
    }
}

#[derive(Debug, serde::Deserialize)]
pub struct FavoriteRequest {
    pub favorite: bool,
}

/// PUT /api/saved-searches/{name}/favorite — toggle favorite status.
pub async fn toggle_saved_search_favorite(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(req): Json<FavoriteRequest>,
) -> Response {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let mut file = crate::saved_search::load(&state.catalog_root)?;
        let entry = file
            .searches
            .iter_mut()
            .find(|s| s.name == name)
            .ok_or_else(|| anyhow::anyhow!("No saved search named '{name}'"))?;
        entry.favorite = req.favorite;
        crate::saved_search::save(&state.catalog_root, &file)?;
        Ok::<_, anyhow::Error>(serde_json::json!({"status": "updated", "name": name, "favorite": req.favorite}))
    })
    .await;

    match result {
        Ok(Ok(json)) => Json(json).into_response(),
        Ok(Err(e)) => {
            let msg = format!("{e:#}");
            if msg.contains("No saved search") {
                (StatusCode::NOT_FOUND, msg).into_response()
            } else {
                (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {msg}")).into_response()
            }
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
    }
}

#[derive(Debug, serde::Deserialize)]
pub struct RenameRequest {
    pub new_name: String,
}

/// PUT /api/saved-searches/{name}/rename — rename a saved search.
pub async fn rename_saved_search(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(req): Json<RenameRequest>,
) -> Response {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let new_name = req.new_name.trim().to_string();
        if new_name.is_empty() {
            anyhow::bail!("Name cannot be empty");
        }
        let mut file = crate::saved_search::load(&state.catalog_root)?;
        // Check for name collision
        if file.searches.iter().any(|s| s.name == new_name) {
            anyhow::bail!("A saved search named '{new_name}' already exists");
        }
        let entry = file
            .searches
            .iter_mut()
            .find(|s| s.name == name)
            .ok_or_else(|| anyhow::anyhow!("No saved search named '{name}'"))?;
        entry.name = new_name.clone();
        crate::saved_search::save(&state.catalog_root, &file)?;
        Ok::<_, anyhow::Error>(serde_json::json!({"status": "renamed", "old_name": name, "new_name": new_name}))
    })
    .await;

    match result {
        Ok(Ok(json)) => Json(json).into_response(),
        Ok(Err(e)) => {
            let msg = format!("{e:#}");
            if msg.contains("No saved search") {
                (StatusCode::NOT_FOUND, msg).into_response()
            } else if msg.contains("already exists") {
                (StatusCode::CONFLICT, msg).into_response()
            } else {
                (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {msg}")).into_response()
            }
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
    }
}

// --- Calendar heatmap ---

#[derive(Debug, serde::Deserialize)]
pub struct CalendarParams {
    pub year: Option<i32>,
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
}

/// GET /api/calendar — calendar heatmap data.
pub async fn calendar_api(
    State(state): State<Arc<AppState>>,
    Query(params): Query<CalendarParams>,
) -> Response {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let catalog = state.catalog()?;

        let year = params.year.unwrap_or_else(|| {
            chrono::Utc::now().format("%Y").to_string().parse::<i32>().unwrap_or(2026)
        });

        let query = params.q.as_deref().unwrap_or("");
        let asset_type = params.asset_type.as_deref().unwrap_or("");
        let tag = params.tag.as_deref().unwrap_or("");
        let format = params.format.as_deref().unwrap_or("");
        let volume = params.volume.as_deref().unwrap_or("");
        let rating_str = params.rating.as_deref().unwrap_or("");
        let label_str = params.label.as_deref().unwrap_or("");
        let collection_str = params.collection.as_deref().unwrap_or("");
        let path_str = params.path.as_deref().unwrap_or("");

        // Normalize path
        let (normalized_path, path_volume_id) = if !path_str.is_empty() {
            let registry = DeviceRegistry::new(&state.catalog_root);
            let vols = registry.list().unwrap_or_default();
            normalize_path_for_search(path_str, &vols, None)
        } else {
            (String::new(), None)
        };

        let parsed = merge_search_params(query, asset_type, tag, format, rating_str, label_str);
        let mut opts = parsed.to_search_options();
        if !volume.is_empty() {
            opts.volume = Some(volume);
        }
        if let Some(ref vid) = path_volume_id {
            if opts.volume.is_none() {
                opts.volume = Some(vid);
            }
        }
        if !normalized_path.is_empty() {
            opts.path_prefix = Some(&normalized_path);
        }

        // Collapse stacks (default: yes)
        let collapse_stacks = params.stacks.as_deref().unwrap_or("1") == "1";
        opts.collapse_stacks = collapse_stacks;

        // Resolve collection filter
        let collection_ids;
        if !collection_str.is_empty() {
            let col_store = crate::collection::CollectionStore::new(catalog.conn());
            collection_ids = col_store.asset_ids_for_collection(collection_str)
                .unwrap_or_default();
            opts.collection_asset_ids = Some(&collection_ids);
        }

        let counts = catalog.calendar_counts(year, &opts)?;
        let years = catalog.calendar_years()?;

        Ok::<_, anyhow::Error>(serde_json::json!({
            "year": year,
            "counts": counts,
            "years": years,
        }))
    })
    .await;

    match result {
        Ok(Ok(json)) => Json(json).into_response(),
        Ok(Err(e)) => {
            (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e:#}")).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
    }
}

// --- Map ---

#[derive(Debug, serde::Deserialize)]
pub struct MapParams {
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
    pub limit: Option<u32>,
}

/// GET /api/map — map markers for geotagged assets.
pub async fn map_api(
    State(state): State<Arc<AppState>>,
    Query(params): Query<MapParams>,
) -> Response {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let catalog = state.catalog()?;

        let query = params.q.as_deref().unwrap_or("");
        let asset_type = params.asset_type.as_deref().unwrap_or("");
        let tag = params.tag.as_deref().unwrap_or("");
        let format = params.format.as_deref().unwrap_or("");
        let volume = params.volume.as_deref().unwrap_or("");
        let rating_str = params.rating.as_deref().unwrap_or("");
        let label_str = params.label.as_deref().unwrap_or("");
        let collection_str = params.collection.as_deref().unwrap_or("");
        let path_str = params.path.as_deref().unwrap_or("");
        let limit = params.limit.unwrap_or(10_000);

        // Normalize path
        let (normalized_path, path_volume_id) = if !path_str.is_empty() {
            let registry = DeviceRegistry::new(&state.catalog_root);
            let vols = registry.list().unwrap_or_default();
            normalize_path_for_search(path_str, &vols, None)
        } else {
            (String::new(), None)
        };

        let parsed = merge_search_params(query, asset_type, tag, format, rating_str, label_str);
        let mut opts = parsed.to_search_options();
        if !volume.is_empty() {
            opts.volume = Some(volume);
        }
        if let Some(ref vid) = path_volume_id {
            if opts.volume.is_none() {
                opts.volume = Some(vid);
            }
        }
        if !normalized_path.is_empty() {
            opts.path_prefix = Some(&normalized_path);
        }

        // Collapse stacks (default: yes)
        let collapse_stacks = params.stacks.as_deref().unwrap_or("1") == "1";
        opts.collapse_stacks = collapse_stacks;

        // Resolve collection filter
        let collection_ids;
        if !collection_str.is_empty() {
            let col_store = crate::collection::CollectionStore::new(catalog.conn());
            collection_ids = col_store.asset_ids_for_collection(collection_str)
                .unwrap_or_default();
            opts.collection_asset_ids = Some(&collection_ids);
        }

        let preview_ext = &state.preview_ext;
        let (markers, total) = catalog.map_markers(&opts, limit)?;

        // Transform preview hashes to URLs
        let markers_json: Vec<serde_json::Value> = markers.iter().map(|m| {
            let preview_url = m.preview.as_ref().map(|h| {
                super::templates::preview_url(h, preview_ext)
            });
            serde_json::json!({
                "id": m.id,
                "lat": m.lat,
                "lng": m.lng,
                "preview": preview_url,
                "name": m.name,
                "rating": m.rating,
                "label": m.label,
            })
        }).collect();

        Ok::<_, anyhow::Error>(serde_json::json!({
            "markers": markers_json,
            "total": total,
            "truncated": total > limit as u64,
        }))
    })
    .await;

    match result {
        Ok(Ok(json)) => Json(json).into_response(),
        Ok(Err(e)) => {
            (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e:#}")).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
    }
}

// --- Facets ---

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
}

/// GET /api/facets — facet counts for the browse sidebar.
pub async fn facets_api(
    State(state): State<Arc<AppState>>,
    Query(params): Query<FacetParams>,
) -> Response {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let catalog = state.catalog()?;

        let query = params.q.as_deref().unwrap_or("");
        let asset_type = params.asset_type.as_deref().unwrap_or("");
        let tag = params.tag.as_deref().unwrap_or("");
        let format = params.format.as_deref().unwrap_or("");
        let volume = params.volume.as_deref().unwrap_or("");
        let rating_str = params.rating.as_deref().unwrap_or("");
        let label_str = params.label.as_deref().unwrap_or("");
        let collection_str = params.collection.as_deref().unwrap_or("");
        let path_str = params.path.as_deref().unwrap_or("");

        // Normalize path
        let (normalized_path, path_volume_id) = if !path_str.is_empty() {
            let registry = DeviceRegistry::new(&state.catalog_root);
            let vols = registry.list().unwrap_or_default();
            normalize_path_for_search(path_str, &vols, None)
        } else {
            (String::new(), None)
        };

        let parsed = merge_search_params(query, asset_type, tag, format, rating_str, label_str);
        let mut opts = parsed.to_search_options();
        if !volume.is_empty() {
            opts.volume = Some(volume);
        }
        if let Some(ref vid) = path_volume_id {
            if opts.volume.is_none() {
                opts.volume = Some(vid);
            }
        }
        if !normalized_path.is_empty() {
            opts.path_prefix = Some(&normalized_path);
        }

        // Collapse stacks (default: yes)
        let collapse_stacks = params.stacks.as_deref().unwrap_or("1") == "1";
        opts.collapse_stacks = collapse_stacks;

        // Resolve collection filter
        let collection_ids;
        if !collection_str.is_empty() {
            let col_store = crate::collection::CollectionStore::new(catalog.conn());
            collection_ids = col_store.asset_ids_for_collection(collection_str)
                .unwrap_or_default();
            opts.collection_asset_ids = Some(&collection_ids);
        }

        let facets = catalog.facet_counts(&opts)?;
        Ok::<_, anyhow::Error>(serde_json::json!(facets))
    })
    .await;

    match result {
        Ok(Ok(json)) => Json(json).into_response(),
        Ok(Err(e)) => {
            (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e:#}")).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
    }
}

// --- Duplicates ---

#[derive(Debug, serde::Deserialize)]
pub struct DuplicatesParams {
    pub mode: Option<String>,
    pub volume: Option<String>,
    pub format: Option<String>,
    pub path: Option<String>,
}

/// GET /duplicates — duplicates page showing duplicate file groups.
pub async fn duplicates_page(
    State(state): State<Arc<AppState>>,
    Query(params): Query<DuplicatesParams>,
) -> Response {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let catalog = state.catalog()?;
        let mode = params.mode.as_deref().unwrap_or("all");

        let vol_filter = params.volume.as_deref().filter(|s| !s.is_empty());
        let fmt_filter = params.format.as_deref().filter(|s| !s.is_empty());
        let path_filter = params.path.as_deref().filter(|s| !s.is_empty());
        let has_filters = vol_filter.is_some() || fmt_filter.is_some() || path_filter.is_some();

        let mut entries = if has_filters {
            catalog.find_duplicates_filtered(mode, vol_filter, fmt_filter, path_filter)?
        } else {
            match mode {
                "same" => catalog.find_duplicates_same_volume()?,
                "cross" => catalog.find_duplicates_cross_volume()?,
                _ => catalog.find_duplicates()?,
            }
        };

        let preview_ext = state.preview_ext.clone();
        for entry in &mut entries {
            entry.preview_url =
                super::templates::preview_url(&entry.content_hash, &preview_ext);
        }

        let total_groups = entries.len();

        // Compute wasted space: for groups with same-volume dupes,
        // count file_size * (extra copies on same volume) per volume group
        let mut total_wasted: u64 = 0;
        let mut same_volume_count: usize = 0;
        for entry in &entries {
            if !entry.same_volume_groups.is_empty() {
                same_volume_count += 1;
                // Count extra same-volume locations
                let mut vol_counts: std::collections::HashMap<&str, usize> =
                    std::collections::HashMap::new();
                for loc in &entry.locations {
                    *vol_counts.entry(&loc.volume_id).or_insert(0) += 1;
                }
                for (_, count) in &vol_counts {
                    if *count > 1 {
                        total_wasted += entry.file_size * (*count as u64 - 1);
                    }
                }
            }
        }

        let all_formats: Vec<FormatOption> = state.dropdown_cache.get_formats(&catalog)
            .into_iter().map(|name| FormatOption { name }).collect();
        let all_volumes: Vec<VolumeOption> = state.dropdown_cache.get_volumes(&catalog)
            .into_iter().map(|(id, label)| VolumeOption { id, label }).collect();

        let dedup_prefer = state.dedup_prefer.clone().unwrap_or_default();

        let tmpl = DuplicatesPage {
            entries,
            mode: mode.to_string(),
            total_groups,
            total_wasted,
            same_volume_count,
            volume: params.volume.unwrap_or_default(),
            format_filter: params.format.unwrap_or_default(),
            path: params.path.unwrap_or_default(),
            all_volumes,
            all_formats,
            dedup_prefer,
        };
        Ok::<_, anyhow::Error>(tmpl.render()?)
    })
    .await;

    match result {
        Ok(Ok(html)) => Html(html).into_response(),
        Ok(Err(e)) => {
            (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e:#}")).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
    }
}

#[derive(Debug, serde::Deserialize)]
pub struct DedupResolveRequest {
    pub min_copies: Option<usize>,
    pub volume: Option<String>,
    pub format: Option<String>,
    pub path: Option<String>,
    pub prefer: Option<String>,
    pub dry_run: Option<bool>,
}

/// POST /api/dedup/resolve — auto-resolve same-volume duplicates (with optional filters and dry-run).
pub async fn dedup_resolve_api(
    State(state): State<Arc<AppState>>,
    Json(req): Json<DedupResolveRequest>,
) -> Response {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let service = state.asset_service();
        let min_copies = req.min_copies.unwrap_or(1);
        let dry_run = req.dry_run.unwrap_or(false);
        let apply = !dry_run;
        let volume = req.volume.filter(|s| !s.is_empty());
        let format = req.format.filter(|s| !s.is_empty());
        let path = req.path.filter(|s| !s.is_empty());
        let prefer = req.prefer.filter(|s| !s.is_empty())
            .or_else(|| state.dedup_prefer.clone());
        let dedup_result = service.dedup(
            volume.as_deref(),
            format.as_deref(),
            path.as_deref(),
            prefer.as_deref(),
            min_copies,
            apply,
            |_, _, _, _| {},
        )?;
        Ok::<_, anyhow::Error>(dedup_result)
    })
    .await;

    match result {
        Ok(Ok(dedup)) => Json(dedup).into_response(),
        Ok(Err(e)) => {
            (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e:#}")).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
    }
}

#[derive(Debug, serde::Deserialize)]
pub struct RemoveLocationRequest {
    pub content_hash: String,
    pub volume_id: String,
    pub relative_path: String,
}

/// DELETE /api/dedup/location — remove a specific file copy.
pub async fn dedup_remove_location_api(
    State(state): State<Arc<AppState>>,
    Json(req): Json<RemoveLocationRequest>,
) -> Response {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let catalog = state.catalog()?;

        // Safety check: count remaining locations for this hash
        let location_count: u64 = catalog.conn().query_row(
            "SELECT COUNT(*) FROM file_locations WHERE content_hash = ?1",
            rusqlite::params![req.content_hash],
            |row| row.get(0),
        )?;
        if location_count <= 1 {
            anyhow::bail!("Cannot remove the last copy of a file");
        }

        // Resolve volume and check it's online
        let registry = DeviceRegistry::new(&state.catalog_root);
        let volumes = registry.list()?;
        let vol = volumes
            .iter()
            .find(|v| v.id.to_string() == req.volume_id)
            .ok_or_else(|| anyhow::anyhow!("Volume not found: {}", req.volume_id))?;
        if !vol.is_online {
            anyhow::bail!("Volume '{}' is offline", vol.label);
        }

        // Delete the physical file
        let full_path = vol.mount_point.join(&req.relative_path);
        if full_path.exists() {
            std::fs::remove_file(&full_path).map_err(|e| {
                anyhow::anyhow!("Failed to delete {}: {e}", full_path.display())
            })?;
        }

        // Remove from catalog
        catalog.delete_file_location(&req.content_hash, &req.volume_id, &req.relative_path)?;

        // Update sidecar YAML
        let service = state.asset_service();
        let metadata_store = crate::metadata_store::MetadataStore::new(&state.catalog_root);
        let vol_uuid: uuid::Uuid = req.volume_id.parse().map_err(|e| {
            anyhow::anyhow!("Invalid volume ID '{}': {e}", req.volume_id)
        })?;
        if let Err(e) = service.remove_sidecar_file_location(
            &metadata_store,
            &catalog,
            &req.content_hash,
            vol_uuid,
            &req.relative_path,
        ) {
            eprintln!("Warning: failed to update sidecar: {e}");
        }

        // Clean up co-located recipe files (same variant, same volume, same directory)
        let loc_dir = std::path::Path::new(&req.relative_path)
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        let mut recipes_removed = 0usize;
        if let Ok(recipes) = catalog.list_recipes_for_variant_on_volume(&req.content_hash, &req.volume_id) {
            for (recipe_id, _recipe_hash, recipe_path) in &recipes {
                let rdir = std::path::Path::new(recipe_path.as_str())
                    .parent()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_default();
                if rdir != loc_dir {
                    continue;
                }
                let recipe_full = vol.mount_point.join(recipe_path);
                let _ = std::fs::remove_file(&recipe_full);
                if let Err(e) = catalog.delete_recipe(recipe_id) {
                    eprintln!("Warning: failed to remove recipe {recipe_path}: {e}");
                } else if let Err(e) = service.remove_sidecar_recipe(
                    &metadata_store,
                    &catalog,
                    &req.content_hash,
                    vol_uuid,
                    recipe_path,
                ) {
                    eprintln!("Warning: failed to update sidecar for recipe {recipe_path}: {e}");
                } else {
                    recipes_removed += 1;
                }
            }
        }

        Ok::<_, anyhow::Error>(serde_json::json!({"removed": true, "recipes_removed": recipes_removed}))
    })
    .await;

    match result {
        Ok(Ok(json)) => Json(json).into_response(),
        Ok(Err(e)) => {
            let msg = format!("{e:#}");
            let status = if msg.contains("Cannot remove") || msg.contains("offline") {
                StatusCode::BAD_REQUEST
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            };
            (status, msg).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
    }
}

#[derive(Debug, serde::Deserialize)]
pub struct CompareParams {
    pub ids: Option<String>,
}

/// GET /compare?ids=id1,id2,... — side-by-side compare page.
pub async fn compare_page(
    State(state): State<Arc<AppState>>,
    Query(params): Query<CompareParams>,
) -> Response {
    let ids_str = params.ids.unwrap_or_default();
    let ids: Vec<&str> = ids_str.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()).collect();

    if ids.len() < 2 || ids.len() > 4 {
        return (
            StatusCode::BAD_REQUEST,
            Html("<h1>Compare</h1><p>Select 2\u{2013}4 assets to compare.</p><p><a href=\"/\">Back to browse</a></p>".to_string()),
        )
            .into_response();
    }

    let preview_ext = state.preview_ext.clone();
    let ids_owned: Vec<String> = ids.iter().map(|s| s.to_string()).collect();
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let engine = state.query_engine();
        let preview_gen = state.preview_generator();
        let mut assets = Vec::new();

        for id in &ids_owned {
            let details = engine.show(id)?;
            let best = crate::models::variant::best_preview_index_details(&details.variants);
            let purl = best
                .and_then(|i| {
                    let v = &details.variants[i];
                    if preview_gen.has_preview(&v.content_hash) {
                        Some(super::templates::preview_url(&v.content_hash, &preview_ext))
                    } else {
                        None
                    }
                })
                .unwrap_or_default();

            assets.push(CompareAsset::from_details(&details, purl));
        }

        let tmpl = ComparePage { assets };
        Ok::<_, anyhow::Error>(tmpl.render()?)
    })
    .await;

    match result {
        Ok(Ok(html)) => Html(html).into_response(),
        Ok(Err(e)) => {
            let msg = format!("{e:#}");
            if msg.contains("No asset found") {
                (
                    StatusCode::NOT_FOUND,
                    Html(format!("<h1>Not Found</h1><p>{msg}</p><p><a href=\"/\">Back to browse</a></p>")),
                )
                    .into_response()
            } else {
                (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {msg}")).into_response()
            }
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
    }
}

/// GET /smart-preview/{prefix}/{file} — serve smart preview, generating on-demand if configured.
pub async fn serve_smart_preview(
    State(state): State<Arc<AppState>>,
    Path((prefix, file)): Path<(String, String)>,
) -> Response {
    let smart_dir = state.catalog_root.join("smart-previews");
    let file_path = smart_dir.join(&prefix).join(&file);

    // If the file already exists, serve it directly
    if file_path.exists() {
        return serve_smart_file(&file_path, &file).await;
    }

    // If on-demand generation is disabled, return 404
    if !state.smart_on_demand {
        return StatusCode::NOT_FOUND.into_response();
    }

    // Extract content hash from filename (strip extension, prepend sha256:)
    let hash_hex = match file.rsplit_once('.') {
        Some((stem, _ext)) => stem,
        None => &file,
    };
    let content_hash = format!("sha256:{hash_hex}");

    let state = state.clone();
    let file_path_clone = file_path.clone();
    let file_name = file.clone();
    let result = tokio::task::spawn_blocking(move || {
        let catalog = state.catalog()?;

        // Look up variant format
        let format = catalog
            .get_variant_format(&content_hash)?
            .ok_or_else(|| anyhow::anyhow!("Variant not found"))?;

        // Find an online source file
        let locations = catalog.get_variant_file_locations(&content_hash)?;
        let registry = DeviceRegistry::new(&state.catalog_root);
        let volumes = registry.list()?;

        let source_path = locations
            .iter()
            .find_map(|(vol_id, rel_path)| {
                let vol = volumes.iter().find(|v| v.id.to_string() == *vol_id)?;
                if !vol.is_online {
                    return None;
                }
                let path = vol.mount_point.join(rel_path);
                if path.exists() {
                    Some(path)
                } else {
                    None
                }
            })
            .ok_or_else(|| anyhow::anyhow!("No online source file"))?;

        // Generate the smart preview
        let preview_gen = state.preview_generator();
        preview_gen.generate_smart(&content_hash, &source_path, &format)?;

        Ok::<_, anyhow::Error>(())
    })
    .await;

    match result {
        Ok(Ok(())) if file_path_clone.exists() => {
            serve_smart_file(&file_path_clone, &file_name).await
        }
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn serve_smart_file(path: &std::path::Path, filename: &str) -> Response {
    match tokio::fs::read(path).await {
        Ok(bytes) => {
            let content_type = if filename.ends_with(".webp") {
                "image/webp"
            } else {
                "image/jpeg"
            };
            (
                StatusCode::OK,
                [
                    (axum::http::header::CONTENT_TYPE, content_type),
                    (axum::http::header::CACHE_CONTROL, "public, max-age=86400"),
                ],
                bytes,
            )
                .into_response()
        }
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}
