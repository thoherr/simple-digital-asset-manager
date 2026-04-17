//! Stats, analytics, and backup-status routes, plus format-group helpers
//! used by browse/search sidebars.

use std::sync::Arc;

use askama::Template;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};

use crate::cli_output::format_size;
use crate::device_registry::DeviceRegistry;
use crate::web::templates::{AnalyticsPage, BackupPage, FormatGroup, FormatOption, StatsPage};
use crate::web::AppState;

/// GET /api/stats — catalog stats as JSON.
pub async fn stats_api(State(state): State<Arc<AppState>>) -> Response {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let catalog = state.catalog()?;
        let (assets, variants, recipes, total_size, _locs) = catalog.stats_overview()?;
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
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e:#}")).into_response(),
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

/// GET /analytics — analytics dashboard page.
pub async fn analytics_page(State(state): State<Arc<AppState>>) -> Response {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let catalog = state.catalog()?;
        let data = catalog.build_analytics(15)?;
        let tmpl = AnalyticsPage {
            data,
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

/// GET /backup — backup status dashboard.
pub async fn backup_page(State(state): State<Arc<AppState>>) -> Response {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let catalog = state.catalog()?;
        let registry = DeviceRegistry::new(&state.catalog_root);
        let vol_list = registry.list()?;
        let volumes_info: Vec<(String, String, bool, Option<String>)> = vol_list
            .iter()
            .filter(|v| v.purpose.as_ref() != Some(&crate::models::VolumePurpose::Media))
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

/// Classify a format extension into a group key.
fn classify_format(fmt: &str) -> &'static str {
    if crate::asset_service::is_raw_extension(fmt) {
        return "raw";
    }
    match fmt {
        "jpg" | "jpeg" | "png" | "tiff" | "tif" | "webp" | "heic" | "heif" | "gif" | "bmp"
        | "svg" | "ico" | "psd" | "xcf" => "image",
        "mp4" | "mov" | "avi" | "mkv" | "wmv" | "flv" | "webm" | "m4v" | "mpg" | "mpeg"
        | "3gp" | "mts" | "m2ts" => "video",
        "mp3" | "wav" | "flac" | "aac" | "ogg" | "wma" | "m4a" | "aiff" | "alac" => "audio",
        _ => "other",
    }
}

/// Build grouped format options from (name, count) pairs.
pub(super) fn build_format_groups(format_counts: Vec<(String, u64)>) -> Vec<FormatGroup> {
    let group_order: &[(&str, &str)] = &[
        ("raw", "RAW"),
        ("image", "Image"),
        ("video", "Video"),
        ("audio", "Audio"),
        ("other", "Other"),
    ];
    let mut groups: std::collections::HashMap<&str, Vec<FormatOption>> =
        std::collections::HashMap::new();
    for (name, count) in format_counts {
        let key = classify_format(&name);
        groups.entry(key).or_default().push(FormatOption { name, count });
    }
    group_order
        .iter()
        .filter_map(|&(key, label)| {
            groups.remove(key).map(|formats| FormatGroup {
                key: key.to_string(),
                label: label.to_string(),
                formats,
            })
        })
        .collect()
}
