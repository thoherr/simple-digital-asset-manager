use std::sync::Arc;

use askama::Template;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{Html, IntoResponse, Response};
use axum::{Form, Json};

use crate::catalog::SearchSort;
use crate::query::parse_search_query;

use crate::device_registry::DeviceRegistry;

use super::templates::{
    format_size, AssetCard, AssetPage, BrowsePage, CollectionOption, DescriptionFragment,
    FormatOption, LabelFragment, NameFragment, PreviewFragment, RatingFragment, ResultsPartial,
    SavedSearchChip, StatsPage, TagOption, TagPageEntry, TagsFragment, TagsPage, VolumeOption,
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
    pub sort: Option<String>,
    pub page: Option<u32>,
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

        let parsed = merge_search_params(query, asset_type, tag, format, rating_str, label_str);
        let mut opts = parsed.to_search_options();
        if !volume.is_empty() {
            opts.volume = Some(volume);
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

        let total = catalog.search_count(&opts)?;
        let rows = catalog.search_paginated(&opts)?;
        let total_pages = ((total as f64) / 60.0).ceil() as u32;
        let cards: Vec<AssetCard> = rows.iter().map(|r| AssetCard::from_row(r, &preview_ext)).collect();

        if is_htmx {
            let tmpl = ResultsPartial {
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
            };
            return Ok::<_, anyhow::Error>(tmpl.render()?);
        }

        let all_tags: Vec<TagOption> = catalog
            .list_all_tags()?
            .into_iter()
            .map(|(name, count)| TagOption { name, count })
            .collect();
        let all_formats: Vec<FormatOption> = catalog
            .list_all_formats()?
            .into_iter()
            .map(|name| FormatOption { name })
            .collect();
        let all_volumes: Vec<VolumeOption> = catalog
            .list_volumes()?
            .into_iter()
            .map(|(id, label)| VolumeOption { id, label })
            .collect();

        let all_collections: Vec<CollectionOption> = {
            let col_store = crate::collection::CollectionStore::new(catalog.conn());
            col_store.list()
                .unwrap_or_default()
                .into_iter()
                .map(|c| CollectionOption { name: c.name })
                .collect()
        };

        let saved_searches = crate::saved_search::load(&state.catalog_root)
            .unwrap_or_default()
            .searches
            .into_iter()
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
            saved_searches,
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

        let parsed = merge_search_params(query, asset_type, tag, format, rating_str, label_str);
        let mut opts = parsed.to_search_options();
        if !volume.is_empty() {
            opts.volume = Some(volume);
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

        let total = catalog.search_count(&opts)?;
        let rows = catalog.search_paginated(&opts)?;
        let total_pages = ((total as f64) / 60.0).ceil() as u32;
        let cards: Vec<AssetCard> = rows.iter().map(|r| AssetCard::from_row(r, &preview_ext)).collect();

        let tmpl = ResultsPartial {
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

/// GET /asset/{id} — asset detail page.
pub async fn asset_page(
    State(state): State<Arc<AppState>>,
    Path(asset_id): Path<String>,
) -> Response {
    let preview_ext = state.preview_ext.clone();
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let engine = state.query_engine();
        let details = engine.show(&asset_id)?;

        let preview_gen = state.preview_generator();
        let preview_url = details.variants.first().and_then(|primary| {
            if preview_gen.has_preview(&primary.content_hash) {
                Some(super::templates::preview_url(&primary.content_hash, &preview_ext))
            } else {
                None
            }
        });

        // Load collections this asset belongs to
        let collections = {
            let catalog = state.catalog()?;
            let col_store = crate::collection::CollectionStore::new(catalog.conn());
            col_store.collections_for_asset(&asset_id).unwrap_or_default()
        };

        let tmpl = AssetPage::from_details(details, preview_url, collections);
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
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .collect();
        let result = engine.tag(&asset_id, &tags, false)?;
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

/// DELETE /api/asset/{id}/tags/{tag} — remove tag, return tags fragment.
pub async fn remove_tag(
    State(state): State<Arc<AppState>>,
    Path((asset_id, tag)): Path<(String, String)>,
) -> Response {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let engine = state.query_engine();
        let result = engine.tag(&asset_id, &[tag], true)?;
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
        let tags = catalog.list_all_tags()?;
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

/// GET /tags — tags HTML page.
pub async fn tags_page(State(state): State<Arc<AppState>>) -> Response {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let catalog = state.catalog()?;
        let tags = catalog.list_all_tags()?;
        let total_tags = tags.len() as u64;
        let entries: Vec<TagPageEntry> = tags
            .into_iter()
            .map(|(name, count)| TagPageEntry { name, count })
            .collect();
        let tmpl = TagsPage {
            tags: entries,
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
        let volumes_info: Vec<(String, String, bool)> = vol_list
            .iter()
            .map(|v| (v.label.clone(), v.id.to_string(), v.is_online))
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
        let mut succeeded = 0u32;
        let mut errors = Vec::new();
        for id in &req.asset_ids {
            match engine.tag(id, &req.tags, req.remove) {
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

        let tmpl = PreviewFragment {
            asset_id,
            primary_preview_url: preview_url,
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
