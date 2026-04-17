//! Tag CRUD routes: add, remove, clear, rename, list, tag page, batch.

use std::sync::Arc;

use askama::Template;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use axum::{Form, Json};

use crate::web::templates::{TagTreeEntry, TagsFragment, TagsPage};
use crate::web::AppState;

use super::{BatchError, BatchResult};

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
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e:#}")).into_response(),
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
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e:#}")).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
    }
}

/// POST /api/asset/{id}/tags/clear — remove all tags, return tags fragment.
pub async fn clear_tags(
    State(state): State<Arc<AppState>>,
    Path(asset_id): Path<String>,
) -> Response {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let engine = state.query_engine();
        let details = engine.show(&asset_id)?;
        if details.tags.is_empty() {
            let tmpl = TagsFragment {
                asset_id,
                tags: vec![],
            };
            return Ok::<_, anyhow::Error>(tmpl.render()?);
        }
        let result = engine.tag(&asset_id, &details.tags, true)?;
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
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e:#}")).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
    }
}

#[derive(Debug, serde::Deserialize)]
pub struct RenameTagRequest {
    pub old_tag: String,
    pub new_tag: String,
    #[serde(default)]
    pub apply: bool,
}

/// POST /api/tag/rename — rename a tag across all assets.
pub async fn rename_tag_api(
    State(state): State<Arc<AppState>>,
    Json(req): Json<RenameTagRequest>,
) -> Response {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let old_storage = crate::tag_util::tag_input_to_storage(&req.old_tag);
        let new_storage = crate::tag_util::tag_input_to_storage(&req.new_tag);
        if new_storage.is_empty() {
            anyhow::bail!("new tag name cannot be empty");
        }
        let engine = state.query_engine();
        let result = engine.tag_rename(&old_storage, &new_storage, req.apply, |_, _| {})?;
        if req.apply {
            state.dropdown_cache.invalidate_tags();
        }
        Ok::<_, anyhow::Error>(serde_json::json!({
            "matched": result.matched,
            "renamed": result.renamed,
            "removed": result.removed,
            "skipped": result.skipped,
            "dry_run": result.dry_run,
        }))
    })
    .await;

    match result {
        Ok(Ok(json)) => Json(json).into_response(),
        Ok(Err(e)) => (StatusCode::BAD_REQUEST, format!("{e:#}")).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
    }
}

/// GET /api/tags — all tags as JSON (for autocomplete).
pub async fn tags_api(State(state): State<Arc<AppState>>) -> Response {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let catalog = state.catalog()?;
        let mut tags = catalog.list_all_tags().unwrap_or_default();
        let vocab = crate::vocabulary::load_vocabulary(&state.catalog_root);
        let existing: std::collections::HashSet<String> = tags.iter().map(|(name, _)| name.clone()).collect();
        for vt in vocab {
            if !existing.contains(&vt) {
                tags.push((vt, 0));
            }
        }
        tags.sort_by(|a, b| a.0.cmp(&b.0));
        Ok::<_, anyhow::Error>(tags)
    })
    .await;

    match result {
        Ok(Ok(tags)) => axum::Json(tags).into_response(),
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e:#}")).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
    }
}

/// Build a tree of tag entries from a flat list of (name, count) pairs.
fn build_tag_tree(flat_tags: &[(String, u64)]) -> Vec<TagTreeEntry> {
    use std::collections::BTreeMap;

    let mut own_counts: BTreeMap<String, u64> = BTreeMap::new();
    for (name, count) in flat_tags {
        own_counts.insert(name.clone(), *count);
    }

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

    let sorted_names: Vec<String> = own_counts.keys().cloned().collect();
    let mut total_counts: BTreeMap<String, u64> = BTreeMap::new();
    for name in &sorted_names {
        let own = own_counts[name];
        total_counts.insert(name.clone(), own);
    }
    for name in sorted_names.iter().rev() {
        let total = total_counts[name];
        if let Some(pipe_pos) = name.rfind('|') {
            let parent = &name[..pipe_pos];
            if let Some(parent_total) = total_counts.get_mut(parent) {
                *parent_total += total;
            }
        }
    }

    let mut has_children_set: std::collections::HashSet<String> = std::collections::HashSet::new();
    for name in &sorted_names {
        if let Some(pipe_pos) = name.rfind('|') {
            has_children_set.insert(name[..pipe_pos].to_string());
        }
    }

    sorted_names
        .iter()
        .map(|name| {
            let depth = name.matches('|').count() as u32;
            let display = name
                .rsplit('|')
                .next()
                .unwrap_or(name)
                .to_string();
            let display_name = name.clone();
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
            ai_enabled: state.ai_enabled,
            vlm_enabled: state.vlm_enabled,
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

#[derive(Debug, serde::Deserialize)]
pub struct BatchTagRequest {
    pub asset_ids: Vec<String>,
    pub tags: Vec<String>,
    pub remove: bool,
}

/// POST /api/batch/tags — add or remove tags on multiple assets.
pub async fn batch_tags(
    State(state): State<Arc<AppState>>,
    Json(req): Json<BatchTagRequest>,
) -> Response {
    let log = state.log_requests;
    let count = req.asset_ids.len();
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let start = std::time::Instant::now();
        let engine = state.query_engine();
        let storage_tags: Vec<String> = req.tags.iter()
            .map(|t| crate::tag_util::tag_input_to_storage(t))
            .collect();
        let results = engine.batch_tag(&req.asset_ids, &storage_tags, req.remove);
        let mut succeeded = 0u32;
        let mut errors = Vec::new();
        for (i, r) in results.into_iter().enumerate() {
            match r {
                Ok(_) => succeeded += 1,
                Err(e) => errors.push(BatchError {
                    asset_id: req.asset_ids[i].clone(),
                    error: format!("{e:#}"),
                }),
            }
        }
        if succeeded > 0 {
            state.dropdown_cache.invalidate_tags();
        }
        let failed = errors.len() as u32;
        if log {
            eprintln!("batch_tag: {} assets in {:.1?} ({} ok, {} err)", count, start.elapsed(), succeeded, failed);
        }
        Ok::<_, anyhow::Error>(BatchResult {
            succeeded,
            failed,
            errors,
        })
    })
    .await;

    match result {
        Ok(Ok(batch)) => Json(batch).into_response(),
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e:#}")).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
    }
}
