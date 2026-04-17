//! Media serving, comparison, file-manager integration, writeback, VLM describe, and ZIP export.

use std::sync::Arc;

use askama::Template;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use axum::Json;

use crate::device_registry::DeviceRegistry;
use crate::query::normalize_path_for_search;
use crate::web::templates::{CompareAsset, ComparePage};
use crate::web::AppState;

use super::{merge_search_params, resolve_collection_ids};
#[cfg(feature = "ai")]
use super::intersect_name_groups;

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
                        Some(crate::web::templates::preview_url(&v.content_hash, &preview_ext))
                    } else {
                        None
                    }
                })
                .unwrap_or_default();

            assets.push(CompareAsset::from_details(&details, purl));
        }

        let tmpl = ComparePage { assets, ai_enabled: state.ai_enabled, vlm_enabled: state.vlm_enabled };
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

    if file_path.exists() {
        return serve_smart_file(&file_path, &file).await;
    }

    if !state.smart_on_demand {
        return StatusCode::NOT_FOUND.into_response();
    }

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

        let format = catalog
            .get_variant_format(&content_hash)?
            .ok_or_else(|| anyhow::anyhow!("variant not found"))?;

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
            .ok_or_else(|| anyhow::anyhow!("no online source file"))?;

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

/// GET /video/{hash} — serve a video file with range request support for seeking.
pub async fn serve_video(
    State(state): State<Arc<AppState>>,
    Path(hash): Path<String>,
    headers: axum::http::HeaderMap,
) -> Response {
    let content_hash = format!("sha256:{hash}");

    let state2 = state.clone();
    let resolved = tokio::task::spawn_blocking(move || {
        let catalog = state2.catalog()?;
        let locations = catalog.get_variant_file_locations(&content_hash)?;
        let format = catalog.get_variant_format(&content_hash)?;
        let registry = DeviceRegistry::new(&state2.catalog_root);
        let volumes = registry.list()?;

        let source_path = locations
            .iter()
            .find_map(|(vol_id, rel_path)| {
                let vol = volumes.iter().find(|v| v.id.to_string() == *vol_id)?;
                if !vol.is_online { return None; }
                let path = vol.mount_point.join(rel_path);
                if path.exists() { Some(path) } else { None }
            });

        Ok::<_, anyhow::Error>((source_path, format))
    })
    .await;

    let (source_path, format) = match resolved {
        Ok(Ok((Some(path), format))) => (path, format),
        _ => return StatusCode::NOT_FOUND.into_response(),
    };

    let content_type = match format.as_deref().unwrap_or("mp4") {
        "mp4" | "m4v" => "video/mp4",
        "mov" => "video/quicktime",
        "webm" => "video/webm",
        "mkv" => "video/x-matroska",
        "avi" => "video/x-msvideo",
        "mts" | "m2ts" => "video/mp2t",
        "3gp" => "video/3gpp",
        _ => "video/mp4",
    };

    let file = match tokio::fs::File::open(&source_path).await {
        Ok(f) => f,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };
    let metadata = match file.metadata().await {
        Ok(m) => m,
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    let total_size = metadata.len();

    if let Some(range_header) = headers.get(axum::http::header::RANGE) {
        if let Ok(range_str) = range_header.to_str() {
            if let Some(range) = range_str.strip_prefix("bytes=") {
                let parts: Vec<&str> = range.splitn(2, '-').collect();
                let start: u64 = parts[0].parse().unwrap_or(0);
                let end: u64 = if parts.len() > 1 && !parts[1].is_empty() {
                    parts[1].parse().unwrap_or(total_size - 1)
                } else {
                    total_size - 1
                };

                if start >= total_size {
                    return StatusCode::RANGE_NOT_SATISFIABLE.into_response();
                }
                let end = end.min(total_size - 1);
                let chunk_size = end - start + 1;

                use tokio::io::{AsyncReadExt, AsyncSeekExt};
                let mut file = file;
                if file.seek(std::io::SeekFrom::Start(start)).await.is_err() {
                    return StatusCode::INTERNAL_SERVER_ERROR.into_response();
                }
                let mut buf = vec![0u8; chunk_size as usize];
                if file.read_exact(&mut buf).await.is_err() {
                    let mut file = match tokio::fs::File::open(&source_path).await {
                        Ok(f) => f,
                        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
                    };
                    let _ = file.seek(std::io::SeekFrom::Start(start)).await;
                    buf.clear();
                    buf.resize(chunk_size as usize, 0);
                    let n = file.read(&mut buf).await.unwrap_or(0);
                    buf.truncate(n);
                }

                return Response::builder()
                    .status(StatusCode::PARTIAL_CONTENT)
                    .header("Content-Type", content_type)
                    .header("Content-Length", buf.len().to_string())
                    .header("Content-Range", format!("bytes {start}-{end}/{total_size}"))
                    .header("Accept-Ranges", "bytes")
                    .body(axum::body::Body::from(buf))
                    .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response());
            }
        }
    }

    use tokio::io::AsyncReadExt;
    let mut file = file;
    let mut buf = Vec::with_capacity(total_size as usize);
    if file.read_to_end(&mut buf).await.is_err() {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", content_type)
        .header("Content-Length", total_size.to_string())
        .header("Accept-Ranges", "bytes")
        .body(axum::body::Body::from(buf))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
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

#[derive(Debug, serde::Deserialize)]
pub struct OpenLocationRequest {
    pub volume_id: String,
    pub relative_path: String,
}

/// POST /api/open-location — reveal a file in the system file manager.
pub async fn open_location(
    State(state): State<Arc<AppState>>,
    Json(req): Json<OpenLocationRequest>,
) -> Response {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let registry = DeviceRegistry::new(&state.catalog_root);
        let volumes = registry.list()?;
        let vol = volumes
            .iter()
            .find(|v| v.id.to_string() == req.volume_id)
            .ok_or_else(|| anyhow::anyhow!("volume not found"))?;

        if !vol.is_online {
            anyhow::bail!("volume '{}' is offline", vol.label);
        }

        let full_path = vol.mount_point.join(&req.relative_path);
        if !full_path.exists() {
            anyhow::bail!("file not found on disk: {}", full_path.display());
        }

        #[cfg(target_os = "macos")]
        {
            std::process::Command::new("open")
                .arg("-R")
                .arg(&full_path)
                .spawn()
                .map_err(|e| anyhow::anyhow!("failed to open Finder: {e}"))?;
        }
        #[cfg(target_os = "linux")]
        {
            if let Some(parent) = full_path.parent() {
                std::process::Command::new("xdg-open")
                    .arg(parent)
                    .spawn()
                    .map_err(|e| anyhow::anyhow!("failed to open file manager: {e}"))?;
            }
        }
        #[cfg(target_os = "windows")]
        {
            std::process::Command::new("explorer")
                .arg("/select,")
                .arg(&full_path)
                .spawn()
                .map_err(|e| anyhow::anyhow!("failed to open Explorer: {e}"))?;
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
        {
            anyhow::bail!("reveal in file manager is not supported on this platform");
        }

        Ok::<_, anyhow::Error>(())
    })
    .await;

    match result {
        Ok(Ok(())) => Json(serde_json::json!({"ok": true})).into_response(),
        Ok(Err(e)) => (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
    }
}

/// POST /api/open-terminal — open a terminal in the file's parent directory.
pub async fn open_terminal(
    State(state): State<Arc<AppState>>,
    Json(req): Json<OpenLocationRequest>,
) -> Response {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let registry = DeviceRegistry::new(&state.catalog_root);
        let volumes = registry.list()?;
        let vol = volumes
            .iter()
            .find(|v| v.id.to_string() == req.volume_id)
            .ok_or_else(|| anyhow::anyhow!("volume not found"))?;

        if !vol.is_online {
            anyhow::bail!("volume '{}' is offline", vol.label);
        }

        let full_path = vol.mount_point.join(&req.relative_path);
        let dir = if full_path.is_dir() {
            full_path
        } else {
            full_path.parent()
                .ok_or_else(|| anyhow::anyhow!("cannot determine parent directory"))?
                .to_path_buf()
        };
        if !dir.exists() {
            anyhow::bail!("directory not found on disk: {}", dir.display());
        }

        #[cfg(target_os = "macos")]
        {
            std::process::Command::new("open")
                .arg("-a")
                .arg("Terminal")
                .arg(&dir)
                .spawn()
                .map_err(|e| anyhow::anyhow!("failed to open Terminal: {e}"))?;
        }
        #[cfg(target_os = "linux")]
        {
            let terminals = ["x-terminal-emulator", "gnome-terminal", "konsole", "xterm"];
            let mut launched = false;
            for term in &terminals {
                let res = std::process::Command::new(term)
                    .arg("--working-directory")
                    .arg(&dir)
                    .spawn();
                if res.is_ok() {
                    launched = true;
                    break;
                }
            }
            if !launched {
                anyhow::bail!("no terminal emulator found");
            }
        }
        #[cfg(target_os = "windows")]
        {
            std::process::Command::new("cmd")
                .args(["/c", "start", "cmd"])
                .current_dir(&dir)
                .spawn()
                .map_err(|e| anyhow::anyhow!("failed to open command prompt: {e}"))?;
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
        {
            anyhow::bail!("open terminal is not supported on this platform");
        }

        Ok::<_, anyhow::Error>(())
    })
    .await;

    match result {
        Ok(Ok(())) => Json(serde_json::json!({"ok": true})).into_response(),
        Ok(Err(e)) => (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
    }
}

// --- Writeback ---

/// POST /api/asset/{id}/writeback — write pending metadata changes to XMP recipe files.
pub async fn writeback_asset(
    State(state): State<Arc<AppState>>,
    Path(asset_id): Path<String>,
) -> Response {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let engine = state.query_engine();
        let catalog = state.catalog().map_err(|e| e.to_string())?;
        let full_id = catalog
            .resolve_asset_id(&asset_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("Asset not found: {asset_id}"))?;

        let wb_result = engine
            .writeback(None, Some(&full_id), None, false, false, false, None)
            .map_err(|e| e.to_string())?;
        Ok::<_, String>(wb_result)
    })
    .await;

    match result {
        Ok(Ok(wb)) => Json(serde_json::json!({
            "written": wb.written,
            "skipped": wb.skipped,
            "failed": wb.failed,
        }))
        .into_response(),
        Ok(Err(msg)) => (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e}")).into_response(),
    }
}

// --- VLM Describe ---

#[derive(serde::Deserialize)]
pub struct VlmDescribeRequest {
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
}

#[derive(serde::Serialize)]
pub struct VlmDescribeResponse {
    pub description: Option<String>,
    pub tags: Vec<String>,
}

/// POST /api/asset/{id}/vlm-describe — describe a single asset via VLM.
pub async fn vlm_describe_asset(
    State(state): State<Arc<AppState>>,
    Path(asset_id): Path<String>,
    Json(body): Json<VlmDescribeRequest>,
) -> Response {
    let state = state.clone();
    let result: Result<Result<VlmDescribeResponse, String>, _> =
        tokio::task::spawn_blocking(move || {
            vlm_describe_asset_inner(&state, &asset_id, body.mode.as_deref(), body.model.as_deref())
        })
        .await;

    match result {
        Ok(Ok(resp)) => Json(resp).into_response(),
        Ok(Err(msg)) => (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e}")).into_response(),
    }
}

fn vlm_describe_asset_inner(
    state: &AppState,
    asset_id: &str,
    mode_str: Option<&str>,
    model_override: Option<&str>,
) -> Result<VlmDescribeResponse, String> {
    use crate::vlm::{self, DescribeMode};

    let vlm = &state.vlm_config;
    let mode = mode_str
        .map(|s| DescribeMode::from_str(s).map_err(|e| e.to_string()))
        .transpose()?
        .unwrap_or(DescribeMode::Describe);
    let model = model_override.unwrap_or(&vlm.model);
    let params = vlm.params_for_model(model);
    let prompt = params.prompt.as_deref()
        .unwrap_or_else(|| vlm::default_prompt_for_mode(mode));

    let engine = state.query_engine();
    let service = state.asset_service();
    let preview_gen = state.preview_generator();

    let catalog = state.catalog().map_err(|e| e.to_string())?;
    let full_id = catalog
        .resolve_asset_id(asset_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Asset not found: {asset_id}"))?;

    let details = engine.show(&full_id).map_err(|e| e.to_string())?;

    let registry = DeviceRegistry::new(&state.catalog_root);
    let volumes = registry.list().map_err(|e| e.to_string())?;
    let online_volumes: std::collections::HashMap<String, &crate::models::Volume> = volumes
        .iter()
        .filter(|v| v.is_online)
        .map(|v| (v.id.to_string(), v))
        .collect();

    let image_path = service.find_image_for_vlm(&details, &preview_gen, &online_volumes)
        .ok_or_else(|| "No preview image available. Run `maki generate-previews` first.".to_string())?;

    let max_edge = if params.max_image_edge > 0 { Some(params.max_image_edge) } else { None };
    let image_base64 = vlm::encode_image_base64(&image_path, max_edge).map_err(|e| e.to_string())?;
    let output = vlm::call_vlm_with_mode(
        &vlm.endpoint,
        model,
        &image_base64,
        prompt,
        &params,
        mode,
        state.verbosity,
    )
    .map_err(|e| e.to_string())?;

    if let Some(ref desc) = output.description {
        if !desc.is_empty() {
            let edit_fields = crate::query::EditFields {
                name: None,
                description: Some(Some(desc.clone())),
                rating: None,
                color_label: None,
                created_at: None,
            };
            engine.edit(&full_id, edit_fields).map_err(|e| e.to_string())?;
        }
    }

    if !output.tags.is_empty() {
        let existing_tags: std::collections::HashSet<String> = details
            .tags
            .iter()
            .map(|t| t.to_lowercase())
            .collect();
        let new_tags: Vec<String> = output
            .tags
            .iter()
            .filter(|t| !existing_tags.contains(&t.to_lowercase()))
            .cloned()
            .collect();
        if !new_tags.is_empty() {
            engine.tag(&full_id, &new_tags, false).map_err(|e| e.to_string())?;
        }
    }

    Ok(VlmDescribeResponse {
        description: output.description,
        tags: output.tags,
    })
}

#[derive(serde::Deserialize)]
pub struct BatchVlmDescribeRequest {
    pub asset_ids: Vec<String>,
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
}

#[derive(serde::Serialize)]
pub struct BatchVlmDescribeResponse {
    pub succeeded: u32,
    pub failed: u32,
    pub descriptions_set: u32,
    pub tags_applied: u32,
    pub errors: Vec<String>,
}

/// POST /api/batch/describe — batch describe assets via VLM.
pub async fn batch_vlm_describe(
    State(state): State<Arc<AppState>>,
    Json(body): Json<BatchVlmDescribeRequest>,
) -> Response {
    let state2 = state.clone();
    let result: Result<Result<BatchVlmDescribeResponse, String>, _> =
        tokio::task::spawn_blocking(move || {
            batch_vlm_describe_inner(&state2, &body.asset_ids, body.mode.as_deref(), body.model.as_deref())
        })
        .await;

    if let Ok(Ok(ref resp)) = result {
        if resp.tags_applied > 0 {
            state.dropdown_cache.invalidate_tags();
        }
    }

    match result {
        Ok(Ok(resp)) => Json(resp).into_response(),
        Ok(Err(msg)) => (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e}")).into_response(),
    }
}

fn batch_vlm_describe_inner(
    state: &AppState,
    asset_ids: &[String],
    mode_str: Option<&str>,
    model_override: Option<&str>,
) -> Result<BatchVlmDescribeResponse, String> {
    use crate::vlm::{self, DescribeMode};

    let vlm = &state.vlm_config;
    let mode = mode_str
        .map(|s| DescribeMode::from_str(s).map_err(|e| e.to_string()))
        .transpose()?
        .unwrap_or(DescribeMode::Describe);
    let vlm_model = model_override.unwrap_or(&vlm.model);
    let params = vlm.params_for_model(vlm_model);
    let prompt = params.prompt.as_deref()
        .unwrap_or_else(|| vlm::default_prompt_for_mode(mode));

    let engine = state.query_engine();
    let service = state.asset_service();
    let preview_gen = state.preview_generator();
    let registry = DeviceRegistry::new(&state.catalog_root);
    let volumes = registry.list().map_err(|e| e.to_string())?;
    let online_volumes: std::collections::HashMap<String, &crate::models::Volume> = volumes
        .iter()
        .filter(|v| v.is_online)
        .map(|v| (v.id.to_string(), v))
        .collect();

    let wants_description = mode == DescribeMode::Describe || mode == DescribeMode::Both;
    let concurrency = (vlm.concurrency.max(1)) as usize;

    let mut result = BatchVlmDescribeResponse {
        succeeded: 0,
        failed: 0,
        descriptions_set: 0,
        tags_applied: 0,
        errors: Vec::new(),
    };

    struct WebWorkItem {
        full_id: String,
        original_id: String,
        image_path: std::path::PathBuf,
        existing_tags: std::collections::HashSet<String>,
    }
    let mut work_items: Vec<WebWorkItem> = Vec::new();

    for aid in asset_ids {
        let catalog = match state.catalog() {
            Ok(c) => c,
            Err(e) => {
                result.errors.push(format!("Catalog error: {e}"));
                result.failed += 1;
                continue;
            }
        };
        let full_id = match catalog.resolve_asset_id(aid) {
            Ok(Some(id)) => id,
            _ => {
                result.errors.push(format!("Asset not found: {aid}"));
                result.failed += 1;
                continue;
            }
        };

        let details = match engine.show(&full_id) {
            Ok(d) => d,
            Err(e) => {
                result.errors.push(format!("{aid}: {e}"));
                result.failed += 1;
                continue;
            }
        };

        if wants_description {
            if let Some(ref desc) = details.description {
                if !desc.is_empty() {
                    result.succeeded += 1;
                    continue;
                }
            }
        }

        let image_path = match service.find_image_for_vlm(&details, &preview_gen, &online_volumes) {
            Some(p) => p,
            None => {
                result.succeeded += 1;
                continue;
            }
        };

        let existing_tags: std::collections::HashSet<String> = details
            .tags
            .iter()
            .map(|t| t.to_lowercase())
            .collect();

        work_items.push(WebWorkItem {
            full_id,
            original_id: aid.clone(),
            image_path,
            existing_tags,
        });
    }

    let vlm_endpoint = &vlm.endpoint;
    let vlm_max_edge = if params.max_image_edge > 0 { Some(params.max_image_edge) } else { None };

    for chunk in work_items.chunks(concurrency) {
        let vlm_results: Vec<(String, String, std::collections::HashSet<String>, Result<vlm::VlmOutput, String>)> =
            std::thread::scope(|s| {
                let handles: Vec<_> = chunk
                    .iter()
                    .map(|item| {
                        let image_path = &item.image_path;
                        let params = &params;
                        s.spawn(move || {
                            let image_base64 = match vlm::encode_image_base64(image_path, vlm_max_edge) {
                                Ok(b) => b,
                                Err(e) => return Err(format!("{}: {e}", item.original_id)),
                            };

                            vlm::call_vlm_with_mode(
                                vlm_endpoint, vlm_model, &image_base64, prompt,
                                params, mode, state.verbosity,
                            )
                            .map_err(|e| format!("{}: {e}", item.original_id))
                        })
                    })
                    .collect();

                handles
                    .into_iter()
                    .zip(chunk.iter())
                    .map(|(h, item)| {
                        let vlm_result = h.join().unwrap();
                        (
                            item.full_id.clone(),
                            item.original_id.clone(),
                            item.existing_tags.clone(),
                            vlm_result,
                        )
                    })
                    .collect()
            });

        for (full_id, original_id, existing_tags, vlm_result) in vlm_results {
            match vlm_result {
                Err(msg) => {
                    result.errors.push(msg);
                    result.failed += 1;
                }
                Ok(output) => {
                    if let Some(ref desc) = output.description {
                        if !desc.is_empty() {
                            let edit_fields = crate::query::EditFields {
                                name: None,
                                description: Some(Some(desc.clone())),
                                rating: None,
                                color_label: None,
                                created_at: None,
                            };
                            if let Err(e) = engine.edit(&full_id, edit_fields) {
                                result.errors.push(format!("{original_id}: {e}"));
                                result.failed += 1;
                                continue;
                            }
                            result.descriptions_set += 1;
                        }
                    }

                    if !output.tags.is_empty() {
                        let new_tags: Vec<String> = output
                            .tags
                            .iter()
                            .filter(|t| !existing_tags.contains(&t.to_lowercase()))
                            .cloned()
                            .collect();
                        if !new_tags.is_empty() {
                            let count = new_tags.len();
                            if let Err(e) = engine.tag(&full_id, &new_tags, false) {
                                result.errors.push(format!("{original_id}: {e}"));
                                result.failed += 1;
                                continue;
                            }
                            result.tags_applied += count as u32;
                        }
                    }

                    result.succeeded += 1;
                }
            }
        }
    }

    Ok(result)
}

// ─── Export ZIP ─────────────────────────────────────────────────────

#[derive(serde::Deserialize, Default)]
pub struct ExportFilters {
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
    pub person: Option<String>,
}

#[derive(serde::Deserialize)]
pub struct ExportZipRequest {
    #[serde(default)]
    pub asset_ids: Vec<String>,
    #[serde(default)]
    pub filters: Option<ExportFilters>,
    #[serde(default = "default_layout")]
    pub layout: String,
    #[serde(default)]
    pub all_variants: bool,
    #[serde(default)]
    pub include_sidecars: bool,
}

fn default_layout() -> String {
    "flat".to_string()
}

/// Stream a ZIP archive of exported assets as a download.
pub async fn export_zip(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ExportZipRequest>,
) -> Response {
    use axum::body::Body;
    use axum::http::header;
    use crate::asset_service::{AssetService, ExportLayout};

    let catalog_root = state.catalog_root.clone();
    let preview_config = state.preview_config.clone();

    let asset_ids = if !req.asset_ids.is_empty() {
        req.asset_ids
    } else {
        let state2 = state.clone();
        let filters = req.filters.unwrap_or_default();
        match tokio::task::spawn_blocking(move || {
            let catalog = state2.catalog()?;

            let query = filters.q.as_deref().unwrap_or("");
            let asset_type = filters.asset_type.as_deref().unwrap_or("");
            let tag = filters.tag.as_deref().unwrap_or("");
            let format = filters.format.as_deref().unwrap_or("");
            let volume = filters.volume.as_deref().unwrap_or("");
            let rating_str = filters.rating.as_deref().unwrap_or("");
            let label_str = filters.label.as_deref().unwrap_or("");
            let collection_str = filters.collection.as_deref().unwrap_or("");
            let path_str = filters.path.as_deref().unwrap_or("");
            let person_str = filters.person.as_deref().unwrap_or("");

            let mut parsed = merge_search_params(query, asset_type, tag, format, rating_str, label_str);

            let path_volume_id = if !path_str.is_empty() {
                let registry = crate::device_registry::DeviceRegistry::new(&state2.catalog_root);
                let vols = registry.list().unwrap_or_default();
                let (normalized, vol_id) = normalize_path_for_search(path_str, &vols, None);
                if !normalized.is_empty() {
                    parsed.path_prefixes.push(normalized);
                }
                vol_id
            } else {
                None
            };

            if !collection_str.is_empty() {
                parsed.collections.push(collection_str.to_string());
            }
            if !person_str.is_empty() {
                for p in person_str.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()) {
                    parsed.persons.push(p.to_string());
                }
            }

            let mut opts = parsed.to_search_options();
            if !volume.is_empty() {
                opts.volume = Some(volume);
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

            opts.per_page = u32::MAX;
            opts.page = 1;
            catalog.search_paginated(&opts)
        }).await {
            Ok(Ok(rows)) => rows.into_iter().map(|r| r.asset_id).collect(),
            Ok(Err(e)) => {
                return (StatusCode::BAD_REQUEST, format!("Search failed: {e}")).into_response();
            }
            Err(e) => {
                return (StatusCode::INTERNAL_SERVER_ERROR, format!("Task failed: {e}")).into_response();
            }
        }
    };

    if asset_ids.is_empty() {
        return (StatusCode::BAD_REQUEST, "No assets matched").into_response();
    }

    let layout = if req.layout == "mirror" { ExportLayout::Mirror } else { ExportLayout::Flat };
    let all_variants = req.all_variants;
    let include_sidecars = req.include_sidecars;
    let count = asset_ids.len();

    let ids = asset_ids;
    let root = catalog_root.clone();
    let pc = preview_config.clone();
    let tmp = std::env::temp_dir().join(format!("maki-export-{}.zip", std::process::id()));
    let tmp2 = tmp.clone();
    let zip_result = tokio::task::spawn_blocking(move || -> Result<std::path::PathBuf, String> {
        let service = AssetService::new(&root, state.verbosity, &pc);
        let result = service.export_zip_for_ids(&ids, &tmp2, layout, all_variants, include_sidecars, |_, _, _| {})
            .map_err(|e| format!("Export failed: {e}"))?;
        if result.files_exported == 0 && result.sidecars_exported == 0 {
            let _ = std::fs::remove_file(&tmp2);
            return Err("No exportable files found (volumes may be offline)".to_string());
        }
        Ok(tmp2)
    }).await;

    let zip_path = match zip_result {
        Ok(Ok(p)) => p,
        Ok(Err(e)) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response();
        }
        Err(e) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, format!("Task failed: {e}")).into_response();
        }
    };

    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Vec<u8>, std::io::Error>>(32);
    let zip_path_clone = zip_path.clone();
    tokio::task::spawn_blocking(move || {
        use std::io::Read;
        let file = match std::fs::File::open(&zip_path_clone) {
            Ok(f) => f,
            Err(_) => return,
        };
        let mut reader = std::io::BufReader::with_capacity(512 * 1024, file);
        let mut buf = vec![0u8; 512 * 1024];
        loop {
            let n = match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(_) => break,
            };
            if tx.blocking_send(Ok(buf[..n].to_vec())).is_err() {
                break;
            }
        }
        drop(tx);
        let _ = std::fs::remove_file(&zip_path_clone);
    });

    let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
    let body = Body::from_stream(stream);

    let filename = format!("maki-export-{}-assets.zip", count);

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/zip")
        .header(header::CONTENT_DISPOSITION, format!("attachment; filename=\"{filename}\""))
        .body(body)
        .unwrap_or_else(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Failed to build response").into_response())
}
