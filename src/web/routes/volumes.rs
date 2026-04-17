//! Volume management routes.

use std::sync::Arc;

use askama::Template;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use axum::Json;

use super::super::AppState;

/// GET /volumes — render volumes page.
pub async fn volumes_page(State(state): State<Arc<AppState>>) -> Response {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let registry = crate::device_registry::DeviceRegistry::new(&state.catalog_root);
        let volumes = registry.list()?;
        let rows: Vec<crate::web::templates::VolumeRow> = volumes
            .iter()
            .map(|v| crate::web::templates::VolumeRow {
                id: v.id.to_string(),
                label: v.label.clone(),
                mount_point: v.mount_point.to_string_lossy().to_string(),
                volume_type: format!("{:?}", v.volume_type).to_lowercase(),
                purpose: v.purpose.as_ref().map(|p| p.as_str().to_string()),
                is_online: v.is_online,
            })
            .collect();
        let config = crate::config::CatalogConfig::load(&state.catalog_root).unwrap_or_default();
        let profile_names: Vec<String> = config.import.profiles.keys().cloned().collect();
        let tmpl = crate::web::templates::VolumesPage {
            volumes: rows,
            profile_names,
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

/// GET /api/volumes — list volumes as JSON.
pub async fn list_volumes_api(State(state): State<Arc<AppState>>) -> Response {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let registry = crate::device_registry::DeviceRegistry::new(&state.catalog_root);
        let volumes = registry.list()?;
        let json: Vec<serde_json::Value> = volumes
            .iter()
            .map(|v| {
                serde_json::json!({
                    "id": v.id.to_string(),
                    "label": v.label,
                    "mount_point": v.mount_point.to_string_lossy(),
                    "volume_type": format!("{:?}", v.volume_type).to_lowercase(),
                    "purpose": v.purpose.as_ref().map(|p| p.as_str()),
                    "is_online": v.is_online,
                })
            })
            .collect();
        Ok::<_, anyhow::Error>(serde_json::json!(json))
    })
    .await;

    match result {
        Ok(Ok(json)) => Json(json).into_response(),
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e:#}")).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
    }
}

#[derive(Debug, serde::Deserialize)]
pub struct RegisterVolumeRequest {
    pub path: String,
    pub label: String,
    pub purpose: Option<String>,
}

/// POST /api/volumes — register a new volume.
pub async fn register_volume_api(
    State(state): State<Arc<AppState>>,
    Json(req): Json<RegisterVolumeRequest>,
) -> Response {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let path = std::path::PathBuf::from(&req.path);
        if !path.exists() {
            anyhow::bail!("path does not exist: {}", req.path);
        }
        let label = req.label.trim().to_string();
        if label.is_empty() {
            anyhow::bail!("label cannot be empty");
        }
        let purpose = req
            .purpose
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(|s| {
                crate::models::VolumePurpose::parse(s)
                    .ok_or_else(|| anyhow::anyhow!("invalid purpose: {s}"))
            })
            .transpose()?;
        let registry = crate::device_registry::DeviceRegistry::new(&state.catalog_root);
        let volume = registry.register(&label, &path, crate::models::VolumeType::External, purpose)?;
        Ok::<_, anyhow::Error>(serde_json::json!({
            "id": volume.id.to_string(),
            "label": volume.label,
        }))
    })
    .await;

    match result {
        Ok(Ok(json)) => (StatusCode::CREATED, Json(json)).into_response(),
        Ok(Err(e)) => {
            let msg = format!("{e:#}");
            let status = if msg.contains("already") {
                StatusCode::CONFLICT
            } else {
                StatusCode::BAD_REQUEST
            };
            (status, msg).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
    }
}

#[derive(Debug, serde::Deserialize)]
pub struct RenameVolumeRequest {
    pub label: String,
}

/// PUT /api/volumes/{id}/rename — rename a volume.
pub async fn rename_volume_api(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<RenameVolumeRequest>,
) -> Response {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let label = req.label.trim().to_string();
        if label.is_empty() {
            anyhow::bail!("label cannot be empty");
        }
        let registry = crate::device_registry::DeviceRegistry::new(&state.catalog_root);
        registry.rename(&id, &label)?;
        Ok::<_, anyhow::Error>(serde_json::json!({"ok": true}))
    })
    .await;

    match result {
        Ok(Ok(json)) => Json(json).into_response(),
        Ok(Err(e)) => (StatusCode::BAD_REQUEST, format!("{e:#}")).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
    }
}

#[derive(Debug, serde::Deserialize)]
pub struct SetPurposeRequest {
    pub purpose: Option<String>,
}

/// PUT /api/volumes/{id}/purpose — set or clear volume purpose.
pub async fn set_volume_purpose_api(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<SetPurposeRequest>,
) -> Response {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let purpose = req
            .purpose
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(|s| {
                crate::models::VolumePurpose::parse(s)
                    .ok_or_else(|| anyhow::anyhow!("invalid purpose: {s}"))
            })
            .transpose()?;
        let registry = crate::device_registry::DeviceRegistry::new(&state.catalog_root);
        registry.set_purpose(&id, purpose)?;
        Ok::<_, anyhow::Error>(serde_json::json!({"ok": true}))
    })
    .await;

    match result {
        Ok(Ok(json)) => Json(json).into_response(),
        Ok(Err(e)) => (StatusCode::BAD_REQUEST, format!("{e:#}")).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
    }
}

/// DELETE /api/volumes/{id} — remove a volume.
pub async fn remove_volume_api(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let service = state.asset_service();
        let result = service.remove_volume(&id, true, |_, _, _| {})?;
        Ok::<_, anyhow::Error>(serde_json::json!({
            "label": result.volume_label,
            "locations_removed": result.locations_removed,
            "recipes_removed": result.recipes_removed,
            "assets_removed": result.removed_assets,
        }))
    })
    .await;

    match result {
        Ok(Ok(json)) => Json(json).into_response(),
        Ok(Err(e)) => (StatusCode::BAD_REQUEST, format!("{e:#}")).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
    }
}
