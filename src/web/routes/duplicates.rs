//! Duplicate file handling routes.

use std::sync::Arc;

use askama::Template;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use axum::Json;

use crate::device_registry::DeviceRegistry;

use super::super::templates::{DuplicatesPage, FormatOption, VolumeOption};
use super::super::AppState;

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
                crate::web::templates::preview_url(&entry.content_hash, &preview_ext);
        }

        let total_groups = entries.len();

        let mut total_wasted: u64 = 0;
        let mut same_volume_count: usize = 0;
        for entry in &entries {
            if !entry.same_volume_groups.is_empty() {
                same_volume_count += 1;
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
            .into_iter().map(|(name, count)| FormatOption { name, count }).collect();
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
            ai_enabled: state.ai_enabled,
            vlm_enabled: state.vlm_enabled,
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

        let location_count: u64 = catalog.conn().query_row(
            "SELECT COUNT(*) FROM file_locations WHERE content_hash = ?1",
            rusqlite::params![req.content_hash],
            |row| row.get(0),
        )?;
        if location_count <= 1 {
            anyhow::bail!("cannot remove the last copy of a file");
        }

        let registry = DeviceRegistry::new(&state.catalog_root);
        let volumes = registry.list()?;
        let vol = volumes
            .iter()
            .find(|v| v.id.to_string() == req.volume_id)
            .ok_or_else(|| anyhow::anyhow!("volume not found: {}", req.volume_id))?;
        if !vol.is_online {
            anyhow::bail!("volume '{}' is offline", vol.label);
        }

        let full_path = vol.mount_point.join(&req.relative_path);
        if full_path.exists() {
            std::fs::remove_file(&full_path).map_err(|e| {
                anyhow::anyhow!("failed to delete {}: {e}", full_path.display())
            })?;
        }

        catalog.delete_file_location(&req.content_hash, &req.volume_id, &req.relative_path)?;

        let service = state.asset_service();
        let metadata_store = crate::metadata_store::MetadataStore::new(&state.catalog_root);
        let vol_uuid: uuid::Uuid = req.volume_id.parse().map_err(|e| {
            anyhow::anyhow!("invalid volume ID '{}': {e}", req.volume_id)
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
