//! Per-asset mutation routes: rating, description, name, date, label, rotation,
//! preview-variant, variant-role, reimport-metadata, generate-preview, and batch variants.

use std::sync::Arc;

use askama::Template;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use axum::{Form, Json};

use crate::device_registry::DeviceRegistry;
use crate::web::templates::{
    DateFragment, DescriptionFragment, LabelFragment, NameFragment, PreviewFragment,
    RatingFragment, TagsFragment,
};
use crate::web::AppState;

use super::resolve_best_variant_idx;

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
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e:#}")).into_response(),
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
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e:#}")).into_response(),
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
        let name = form.name.filter(|s| !s.trim().is_empty());
        let new_name = engine.set_name(&asset_id, name)?;

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
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e:#}")).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
    }
}

#[derive(Debug, serde::Deserialize)]
pub struct DateForm {
    pub date: String,
}

/// PUT /api/asset/{id}/date — set date, return date fragment.
pub async fn set_date(
    State(state): State<Arc<AppState>>,
    Path(asset_id): Path<String>,
    Form(form): Form<DateForm>,
) -> Response {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let engine = state.query_engine();
        let dt = crate::query::parse_date_input(&form.date)?;
        let date_str = engine.set_date(&asset_id, dt)?;
        let tmpl = DateFragment {
            asset_id,
            created_at: crate::web::templates::format_date(&date_str),
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

// --- Batch operations ---

#[derive(Debug, serde::Deserialize)]
pub struct BatchRatingRequest {
    pub asset_ids: Vec<String>,
    pub rating: Option<u8>,
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
    let log = state.log_requests;
    let count = req.asset_ids.len();
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let start = std::time::Instant::now();
        let engine = state.query_engine();
        let rating = req.rating.filter(|&r| r > 0);
        let results = engine.batch_set_rating(&req.asset_ids, rating);
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
        let failed = errors.len() as u32;
        if log {
            eprintln!("batch_rating: {} assets in {:.1?} ({} ok, {} err)", count, start.elapsed(), succeeded, failed);
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

/// POST /api/asset/{id}/preview — regenerate preview + smart preview.
pub async fn generate_preview(
    State(state): State<Arc<AppState>>,
    Path(asset_id): Path<String>,
) -> Response {
    let preview_ext = state.preview_ext.clone();
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let catalog = state.catalog()?;
        let engine = state.query_engine();
        let details = engine.show(&asset_id)?;

        let best_idx = resolve_best_variant_idx(&catalog, &asset_id, &details.variants)?;
        let variant = &details.variants[best_idx];
        let content_hash = &variant.content_hash;
        let format = &variant.format;

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
                if path.exists() { Some(path) } else { None }
            });

        let preview_gen = state.preview_generator();
        let existing_preview_url = if preview_gen.has_preview(content_hash) {
            Some(crate::web::templates::preview_url(content_hash, &preview_ext))
        } else {
            None
        };
        let existing_smart_url = if preview_gen.has_smart_preview(content_hash) {
            Some(crate::web::templates::smart_preview_url(content_hash, &preview_ext))
        } else {
            None
        };

        let has_existing_smart = existing_smart_url.is_some();
        let source_path = match source_path {
            Some(p) => p,
            None => {
                let tmpl = PreviewFragment {
                    asset_id,
                    primary_preview_url: existing_preview_url,
                    smart_preview_url: existing_smart_url,
                    has_smart_preview: has_existing_smart,
                    has_online_source: false,
                    error: Some("Source files are offline — cannot regenerate previews.".to_string()),
                    is_video: false,
                    video_url: None,
                };
                return Ok::<_, anyhow::Error>(tmpl.render()?);
            }
        };

        preview_gen.regenerate(content_hash, &source_path, format)?;
        preview_gen.regenerate_smart(content_hash, &source_path, format)?;

        let is_video = details.asset_type == "video";
        if is_video {
            let has_duration = details.variants.get(best_idx)
                .map(|v| v.source_metadata.contains_key("video_duration"))
                .unwrap_or(false);
            if !has_duration {
                let service = state.asset_service();
                service.backfill_video_metadata(&details.id, content_hash, &source_path);
            }
        }

        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let preview_url = if preview_gen.has_preview(content_hash) {
            let url = crate::web::templates::preview_url(content_hash, &preview_ext);
            Some(format!("{url}?t={ts}"))
        } else {
            None
        };

        let has_smart = preview_gen.has_smart_preview(content_hash);
        let smart_url = if has_smart {
            let url = crate::web::templates::smart_preview_url(content_hash, &preview_ext);
            Some(format!("{url}?t={ts}"))
        } else {
            None
        };

        let tmpl = PreviewFragment {
            asset_id,
            primary_preview_url: preview_url,
            smart_preview_url: smart_url,
            has_smart_preview: has_smart,
            has_online_source: true,
            error: None,
            is_video: details.asset_type == "video",
            video_url: if details.asset_type == "video" {
                Some(crate::web::templates::video_url(content_hash))
            } else {
                None
            },
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

/// POST /api/asset/{id}/rotate — cycle preview rotation 90° CW.
pub async fn set_rotation(
    State(state): State<Arc<AppState>>,
    Path(asset_id): Path<String>,
) -> Response {
    let preview_ext = state.preview_ext.clone();
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let engine = state.query_engine();
        let catalog = state.catalog()?;

        let details = engine.show(&asset_id)?;
        let current: Option<u16> = catalog.conn().query_row(
            "SELECT preview_rotation FROM assets WHERE id = ?1",
            [&details.id],
            |r| {
                let val: Option<i64> = r.get(0)?;
                Ok(val.map(|v| v as u16))
            },
        ).unwrap_or(None);

        let new_rotation = match current {
            None | Some(0) => Some(90u16),
            Some(90) => Some(180),
            Some(180) => Some(270),
            Some(270) => None,
            Some(_) => Some(90),
        };

        engine.set_preview_rotation(&asset_id, new_rotation)?;

        let best_idx = resolve_best_variant_idx(&catalog, &asset_id, &details.variants)?;
        let variant = &details.variants[best_idx];
        let content_hash = &variant.content_hash;
        let format = &variant.format;

        let registry = crate::device_registry::DeviceRegistry::new(&state.catalog_root);
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
                if path.exists() { Some(path) } else { None }
            });

        let preview_gen = state.preview_generator();
        let existing_preview_url = if preview_gen.has_preview(content_hash) {
            Some(crate::web::templates::preview_url(content_hash, &preview_ext))
        } else {
            None
        };
        let existing_smart_url = if preview_gen.has_smart_preview(content_hash) {
            Some(crate::web::templates::smart_preview_url(content_hash, &preview_ext))
        } else {
            None
        };

        let has_existing_smart = existing_smart_url.is_some();
        let source_path = match source_path {
            Some(p) => p,
            None => {
                let tmpl = PreviewFragment {
                    asset_id,
                    primary_preview_url: existing_preview_url,
                    smart_preview_url: existing_smart_url,
                    has_smart_preview: has_existing_smart,
                    has_online_source: false,
                    error: Some("Source files are offline — cannot rotate.".to_string()),
                    is_video: false,
                    video_url: None,
                };
                return Ok::<_, anyhow::Error>(tmpl.render()?);
            }
        };

        preview_gen.regenerate_with_rotation(content_hash, &source_path, format, new_rotation)?;
        if preview_gen.has_smart_preview(content_hash) {
            preview_gen.regenerate_smart_with_rotation(content_hash, &source_path, format, new_rotation)?;
        }

        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let preview_url = if preview_gen.has_preview(content_hash) {
            let url = crate::web::templates::preview_url(content_hash, &preview_ext);
            Some(format!("{url}?t={ts}"))
        } else {
            None
        };

        let has_smart = preview_gen.has_smart_preview(content_hash);
        let smart_url = if has_smart {
            let url = crate::web::templates::smart_preview_url(content_hash, &preview_ext);
            Some(format!("{url}?t={ts}"))
        } else {
            None
        };

        let tmpl = PreviewFragment {
            asset_id,
            primary_preview_url: preview_url,
            smart_preview_url: smart_url,
            has_smart_preview: has_smart,
            has_online_source: true,
            error: None,
            is_video: details.asset_type == "video",
            video_url: if details.asset_type == "video" {
                Some(crate::web::templates::video_url(content_hash))
            } else {
                None
            },
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

/// POST /api/asset/{id}/preview-variant — set or clear the preview variant override.
pub async fn set_preview_variant(
    State(state): State<Arc<AppState>>,
    Path(asset_id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let engine = state.query_engine();
        let content_hash = body.get("content_hash").and_then(|v| v.as_str());
        engine.set_preview_variant(&asset_id, content_hash)?;
        Ok::<_, anyhow::Error>(serde_json::json!({"ok": true}))
    })
    .await;

    match result {
        Ok(Ok(json)) => Json(json).into_response(),
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e:#}")).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
    }
}

/// POST /api/asset/{id}/variant-role — change a variant's role.
pub async fn set_variant_role(
    State(state): State<Arc<AppState>>,
    Path(asset_id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let engine = state.query_engine();
        let content_hash = body.get("content_hash").and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing content_hash"))?;
        let role = body.get("role").and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing role"))?;
        engine.set_variant_role(&asset_id, content_hash, role)?;
        Ok::<_, anyhow::Error>(serde_json::json!({"ok": true, "role": role}))
    })
    .await;

    match result {
        Ok(Ok(json)) => Json(json).into_response(),
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e:#}")).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
    }
}

/// POST /api/asset/{id}/reimport-metadata — clear and re-extract metadata from source files.
pub async fn reimport_metadata(
    State(state): State<Arc<AppState>>,
    Path(asset_id): Path<String>,
) -> Response {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let engine = state.query_engine();
        let tags = engine.reimport_metadata(&asset_id)?;
        state.dropdown_cache.invalidate_tags();
        let tmpl = TagsFragment {
            asset_id,
            tags,
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
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e:#}")).into_response(),
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
    let log = state.log_requests;
    let count = req.asset_ids.len();
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let start = std::time::Instant::now();
        let engine = state.query_engine();
        let label_str = req.label.filter(|s| !s.trim().is_empty());
        let validated = match label_str {
            Some(ref s) => match crate::models::Asset::validate_color_label(s) {
                Ok(canonical) => canonical,
                Err(e) => return Err(anyhow::anyhow!(e)),
            },
            None => None,
        };
        let results = engine.batch_set_color_label(&req.asset_ids, validated);
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
        let failed = errors.len() as u32;
        if log {
            eprintln!("batch_label: {} assets in {:.1?} ({} ok, {} err)", count, start.elapsed(), succeeded, failed);
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
