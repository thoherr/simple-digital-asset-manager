//! Saved-search API and management routes.

use std::sync::Arc;

use askama::Template;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use axum::Json;

use crate::web::templates::{SavedSearchEntry, SavedSearchesPage};
use crate::web::AppState;

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
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e:#}")).into_response(),
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
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e:#}")).into_response(),
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
            anyhow::bail!("no saved search named '{name}'");
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
        let tmpl = SavedSearchesPage { searches, ai_enabled: state.ai_enabled, vlm_enabled: state.vlm_enabled };
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
            .ok_or_else(|| anyhow::anyhow!("no saved search named '{name}'"))?;
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
            anyhow::bail!("name cannot be empty");
        }
        let mut file = crate::saved_search::load(&state.catalog_root)?;
        if file.searches.iter().any(|s| s.name == new_name) {
            anyhow::bail!("a saved search named '{new_name}' already exists");
        }
        let entry = file
            .searches
            .iter_mut()
            .find(|s| s.name == name)
            .ok_or_else(|| anyhow::anyhow!("no saved search named '{name}'"))?;
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
