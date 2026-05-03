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
    let html = match super::spawn_catalog_blocking(move || {
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
        let tmpl = crate::web::templates::VolumesPage {
            volumes: rows,
            ai_enabled: state.ai_enabled,
            vlm_enabled: state.vlm_enabled,
        };
        Ok(tmpl.render()?)
    })
    .await
    {
        Ok(html) => html,
        Err(resp) => return resp,
    };
    Html(html).into_response()
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

#[derive(Debug, serde::Deserialize)]
pub struct BrowseVolumeParams {
    /// Volume-relative path prefix (forward-slash separated). Empty = mount root.
    #[serde(default)]
    pub prefix: String,
    /// Maximum entries to return (default 50, capped at 500).
    pub limit: Option<usize>,
    /// Set to `1` to include hidden (dotfile) entries. Default off.
    pub hidden: Option<String>,
    /// Filter mode: `dirs` (only directories), `files` (only files), `all` (default).
    pub filter: Option<String>,
}

#[derive(Debug, serde::Serialize)]
pub struct BrowseVolumeEntry {
    pub name: String,
    pub is_dir: bool,
    /// Volume-relative path of this entry (forward slashes).
    pub relative_path: String,
}

/// GET /api/volumes/{id}/browse?prefix=&limit=&hidden=&filter=
///
/// List filesystem entries (directories and files) under a volume's mount point,
/// scoped to `mount_point.join(prefix)`. Used by the import dialog's path
/// autocomplete: paths NOT yet in the catalog (which is what `/api/paths` covers).
///
/// Security: the resolved path is canonicalized and asserted to start with the
/// canonicalized mount point — `..` traversal that escapes the mount is rejected
/// with 403.
pub async fn browse_volume_api(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    axum::extract::Query(params): axum::extract::Query<BrowseVolumeParams>,
) -> Response {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let registry = crate::device_registry::DeviceRegistry::new(&state.catalog_root);
        let volume = registry.resolve_volume(&id)?;
        if !volume.is_online {
            anyhow::bail!("volume is offline");
        }

        let mount_canon = std::fs::canonicalize(&volume.mount_point)
            .map_err(|e| anyhow::anyhow!("mount point not accessible: {e}"))?;

        let prefix = params.prefix.trim_matches('/');
        let target = resolve_browse_target(&mount_canon, prefix)?;

        if !target.is_dir() {
            anyhow::bail!("path is not a directory");
        }

        let limit = params.limit.unwrap_or(50).min(500);
        let show_hidden = params.hidden.as_deref() == Some("1");
        let filter = params.filter.as_deref().unwrap_or("all");

        let mut entries: Vec<BrowseVolumeEntry> = Vec::new();
        let read = std::fs::read_dir(&target)
            .map_err(|e| anyhow::anyhow!("read_dir failed: {e}"))?;
        for entry in read.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if !show_hidden && name.starts_with('.') {
                continue;
            }
            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
            match filter {
                "dirs" if !is_dir => continue,
                "files" if is_dir => continue,
                _ => {}
            }
            let relative_path = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{}/{}", prefix, name)
            };
            entries.push(BrowseVolumeEntry { name, is_dir, relative_path });
        }

        // Directories first, then files; alpha within each, case-insensitive.
        entries.sort_by(|a, b| {
            b.is_dir.cmp(&a.is_dir)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        entries.truncate(limit);

        Ok::<_, anyhow::Error>(serde_json::json!({
            "prefix": prefix,
            "entries": entries,
        }))
    })
    .await;

    match result {
        Ok(Ok(json)) => Json(json).into_response(),
        Ok(Err(e)) => {
            let msg = format!("{e:#}");
            let status = if msg.contains("escapes") {
                StatusCode::FORBIDDEN
            } else if msg.contains("not accessible") || msg.contains("not a directory") {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::BAD_REQUEST
            };
            (status, msg).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
    }
}

/// Resolve a volume-relative `prefix` into a canonical filesystem path
/// under `mount_canon`. Rejects paths that escape the mount point via
/// `..` traversal or symlinks.
fn resolve_browse_target(
    mount_canon: &std::path::Path,
    prefix: &str,
) -> anyhow::Result<std::path::PathBuf> {
    if prefix.is_empty() {
        return Ok(mount_canon.to_path_buf());
    }
    let mut p = mount_canon.to_path_buf();
    for segment in prefix.split('/') {
        if segment.is_empty() {
            continue;
        }
        p.push(segment);
    }
    let canon = std::fs::canonicalize(&p)
        .map_err(|e| anyhow::anyhow!("path not accessible: {e}"))?;
    if !canon.starts_with(mount_canon) {
        anyhow::bail!("path escapes volume mount point");
    }
    Ok(canon)
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn resolve_empty_prefix_returns_mount_root() {
        let tmp = tempfile::tempdir().unwrap();
        let mount = std::fs::canonicalize(tmp.path()).unwrap();
        let r = resolve_browse_target(&mount, "").unwrap();
        assert_eq!(r, mount);
    }

    #[test]
    fn resolve_subdir_returns_subdir() {
        let tmp = tempfile::tempdir().unwrap();
        let mount = std::fs::canonicalize(tmp.path()).unwrap();
        fs::create_dir(mount.join("photos")).unwrap();
        let r = resolve_browse_target(&mount, "photos").unwrap();
        assert_eq!(r, mount.join("photos"));
    }

    #[test]
    fn resolve_nested_subdir() {
        let tmp = tempfile::tempdir().unwrap();
        let mount = std::fs::canonicalize(tmp.path()).unwrap();
        fs::create_dir_all(mount.join("2026/04")).unwrap();
        let r = resolve_browse_target(&mount, "2026/04").unwrap();
        assert_eq!(r, mount.join("2026/04"));
    }

    #[test]
    fn resolve_rejects_dotdot_escape() {
        // `prefix=..` would land outside the mount → must error.
        let tmp = tempfile::tempdir().unwrap();
        let mount = std::fs::canonicalize(tmp.path()).unwrap();
        let r = resolve_browse_target(&mount, "..");
        assert!(r.is_err());
        let msg = format!("{:#}", r.unwrap_err());
        assert!(msg.contains("escapes"), "got: {msg}");
    }

    #[test]
    fn resolve_rejects_dotdot_through_subdir() {
        // `subdir/../..` → traverses outside the mount.
        let tmp = tempfile::tempdir().unwrap();
        let mount = std::fs::canonicalize(tmp.path()).unwrap();
        fs::create_dir(mount.join("a")).unwrap();
        let r = resolve_browse_target(&mount, "a/../..");
        assert!(r.is_err());
        let msg = format!("{:#}", r.unwrap_err());
        assert!(msg.contains("escapes"), "got: {msg}");
    }

    #[test]
    fn resolve_collapses_double_slashes() {
        // Empty segments from `//` are skipped without falling out of the mount.
        let tmp = tempfile::tempdir().unwrap();
        let mount = std::fs::canonicalize(tmp.path()).unwrap();
        fs::create_dir_all(mount.join("a/b")).unwrap();
        let r = resolve_browse_target(&mount, "a//b").unwrap();
        assert_eq!(r, mount.join("a/b"));
    }

    #[test]
    fn resolve_rejects_nonexistent_path() {
        // Canonicalize fails → `not accessible`.
        let tmp = tempfile::tempdir().unwrap();
        let mount = std::fs::canonicalize(tmp.path()).unwrap();
        let r = resolve_browse_target(&mount, "does-not-exist");
        assert!(r.is_err());
        let msg = format!("{:#}", r.unwrap_err());
        assert!(msg.contains("not accessible"), "got: {msg}");
    }

    #[test]
    fn resolve_rejects_symlink_escape() {
        // A symlink inside the mount that points outside must be rejected
        // by the canonical-path starts_with check.
        #[cfg(unix)]
        {
            let outside = tempfile::tempdir().unwrap();
            let outside_canon = std::fs::canonicalize(outside.path()).unwrap();
            let tmp = tempfile::tempdir().unwrap();
            let mount = std::fs::canonicalize(tmp.path()).unwrap();
            std::os::unix::fs::symlink(&outside_canon, mount.join("link")).unwrap();

            let r = resolve_browse_target(&mount, "link");
            assert!(r.is_err());
            let msg = format!("{:#}", r.unwrap_err());
            assert!(msg.contains("escapes"), "got: {msg}");
        }
    }
}
