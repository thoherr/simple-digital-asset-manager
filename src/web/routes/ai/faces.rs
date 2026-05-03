//! Face detection, person assignment, people page, merge/cluster utilities.
//!
//! The largest of the AI submodules — covers the full face-recognition
//! lifecycle from per-asset / batch detect through person CRUD and the
//! people gallery template.

use std::sync::Arc;

use askama::Template;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use axum::Json;

use crate::web::templates::{PeoplePage, PersonCard};
use crate::web::AppState;

// --- Face recognition handlers ---

/// GET /api/asset/{id}/faces — list faces for an asset.
pub async fn asset_faces(
    State(state): State<Arc<AppState>>,
    Path(asset_id): Path<String>,
) -> Response {
    let result = tokio::task::spawn_blocking(move || {
        let catalog = state.catalog()?;
        let face_store = crate::face_store::FaceStore::new(catalog.conn());
        let faces = face_store.faces_for_asset(&asset_id)?;
        let result: Vec<serde_json::Value> = faces.iter().map(|f| {
            let crop_url = if crate::face::face_crop_exists(&f.id, &state.catalog_root) {
                Some(format!("/face/{}/{}.jpg", &f.id[..2.min(f.id.len())], f.id))
            } else {
                None
            };
            let person_name = f.person_id.as_ref().and_then(|pid| {
                face_store.get_person(pid).ok().flatten().and_then(|p| p.name)
            });
            serde_json::json!({
                "face_id": f.id,
                "confidence": f.confidence,
                "bbox": [f.bbox_x, f.bbox_y, f.bbox_w, f.bbox_h],
                "person_id": f.person_id,
                "person_name": person_name,
                "crop_url": crop_url,
            })
        }).collect();
        Ok::<_, anyhow::Error>(result)
    }).await;

    match result {
        Ok(Ok(faces)) => Json(faces).into_response(),
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e}")).into_response(),
    }
}

/// POST /api/asset/{id}/detect-faces — detect faces for a single asset.
pub async fn detect_faces_for_asset(
    State(state): State<Arc<AppState>>,
    Path(asset_id): Path<String>,
) -> Response {
    let state2 = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        detect_faces_inner(&state2, &[asset_id])
    }).await;

    match result {
        Ok(Ok(json)) => Json(json).into_response(),
        Ok(Err(msg)) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": msg}))).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": format!("{e}")}))).into_response(),
    }
}

/// POST /api/batch/detect-faces — start a face-detection job for selected assets.
///
/// Returns `{job_id}` immediately. Progress flows through `/api/jobs/{id}/progress`;
/// the terminal event carries `{succeeded, faces_detected, errors, done: true}`.
pub async fn batch_detect_faces(
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    use crate::web::jobs::JobKind;

    let asset_ids: Vec<String> = body.get("asset_ids")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    // Pre-flight: face models present?
    let face_model_dir = crate::face::resolve_face_model_dir(&state.ai_config);
    if !crate::face::FaceDetector::models_exist(&face_model_dir) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Face models not downloaded. Run 'maki faces download' first.",
        )
            .into_response();
    }

    let job = state.jobs.start(JobKind::DetectFaces);
    let job_id = job.id.clone();
    let total = asset_ids.len();
    job.emit(&serde_json::json!({
        "phase": "detect_faces",
        "done": false,
        "processed": 0,
        "total": total,
        "status": "starting",
    }));

    let state2 = state.clone();
    let job_for_task = job.clone();
    let exec_provider = state.ai_config.execution_provider.clone();
    let min_conf = state.ai_config.face_min_confidence;

    tokio::spawn(async move {
        let job_inner = job_for_task.clone();
        let processed = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let processed_for_cb = processed.clone();
        let job_for_cb = job_inner.clone();
        let state_for_blocking = state2.clone();

        let result = tokio::task::spawn_blocking(move || {
            let mut detector = match crate::face::FaceDetector::load_with_provider(
                &face_model_dir,
                state_for_blocking.verbosity,
                &exec_provider,
            ) {
                Ok(d) => d,
                Err(e) => return Err(format!("Failed to load face detector: {e:#}")),
            };
            let service = state_for_blocking.asset_service();
            service
                .detect_faces(
                    &asset_ids,
                    &mut detector,
                    min_conf,
                    true,
                    true,
                    move |aid, faces, _elapsed| {
                        use std::sync::atomic::Ordering::Relaxed;
                        let n = processed_for_cb.fetch_add(1, Relaxed) + 1;
                        let short = &aid[..8.min(aid.len())];
                        job_for_cb.emit(&serde_json::json!({
                            "phase": "detect_faces",
                            "done": false,
                            "processed": n,
                            "total": total,
                            "asset": short,
                            "faces": faces,
                        }));
                    },
                )
                .map_err(|e| format!("{e:#}"))
        })
        .await;

        let terminal = match result {
            Ok(Ok(r)) => serde_json::json!({
                "phase": "detect_faces",
                "succeeded": r.assets_processed,
                "faces_detected": r.faces_detected,
                "errors": r.errors,
            }),
            Ok(Err(e)) => serde_json::json!({"phase": "detect_faces", "error": e}),
            Err(e) => serde_json::json!({"phase": "detect_faces", "error": format!("{e}")}),
        };
        job_for_task.finish(terminal);
        state2.jobs.mark_done(&job_for_task.id);
    });

    Json(serde_json::json!({"job_id": job_id, "status": "started"})).into_response()
}

fn detect_faces_inner(state: &AppState, asset_ids: &[String]) -> Result<serde_json::Value, String> {
    let face_model_dir = crate::face::resolve_face_model_dir(&state.ai_config);
    if !crate::face::FaceDetector::models_exist(&face_model_dir) {
        return Err("Face models not downloaded. Run 'maki faces download' first.".to_string());
    }

    let mut detector = crate::face::FaceDetector::load_with_provider(&face_model_dir, state.verbosity, &state.ai_config.execution_provider)
        .map_err(|e| format!("Failed to load face detector: {e:#}"))?;

    let service = state.asset_service();
    let result = service.detect_faces(
        asset_ids,
        &mut detector,
        state.ai_config.face_min_confidence,
        true,
        true,
        |_, _, _| {},
    ).map_err(|e| format!("{e:#}"))?;

    Ok(serde_json::json!({
        "succeeded": result.assets_processed,
        "faces_detected": result.faces_detected,
        "errors": result.errors,
    }))
}

/// PUT /api/faces/{face_id}/assign — assign a face to a person.
pub async fn assign_face(
    State(state): State<Arc<AppState>>,
    Path(face_id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    let person_id: String = match body.get("person_id").and_then(|v| v.as_str()) {
        Some(pid) => pid.to_string(),
        None => return (StatusCode::BAD_REQUEST, "Missing person_id").into_response(),
    };

    match super::super::spawn_catalog_blocking(move || {
        let catalog = state.catalog()?;
        let face_store = crate::face_store::FaceStore::new(catalog.conn());
        face_store.assign_face_to_person(&face_id, &person_id)?;
        let _ = face_store.save_all_yaml(&state.catalog_root);
        state.dropdown_cache.invalidate_people();
        Ok(())
    }).await {
        Ok(()) => Json(serde_json::json!({"ok": true})).into_response(),
        Err(resp) => resp,
    }
}

/// DELETE /api/faces/{face_id}/unassign — unassign a face from its person.
pub async fn unassign_face_api(
    State(state): State<Arc<AppState>>,
    Path(face_id): Path<String>,
) -> Response {
    match super::super::spawn_catalog_blocking(move || {
        let catalog = state.catalog()?;
        let face_store = crate::face_store::FaceStore::new(catalog.conn());
        face_store.unassign_face(&face_id)?;
        let _ = face_store.save_all_yaml(&state.catalog_root);
        state.dropdown_cache.invalidate_people();
        Ok(())
    }).await {
        Ok(()) => Json(serde_json::json!({"ok": true})).into_response(),
        Err(resp) => resp,
    }
}

/// DELETE /api/faces/{face_id} — delete a face detection (e.g., false positive).
pub async fn delete_face_api(
    State(state): State<Arc<AppState>>,
    Path(face_id): Path<String>,
) -> Response {
    let catalog_root = state.catalog_root.clone();
    let result = tokio::task::spawn_blocking(move || {
        let catalog = state.catalog()?;
        let face_store = crate::face_store::FaceStore::new(catalog.conn());
        if let Some(asset_id) = face_store.delete_face(&face_id)? {
            catalog.update_face_count(&asset_id)?;
            let prefix = &face_id[..2.min(face_id.len())];
            let crop = catalog_root.join("faces").join(prefix).join(format!("{face_id}.jpg"));
            let _ = std::fs::remove_file(crop);
            crate::face_store::delete_arcface_binary(&catalog_root, &face_id);
        }
        let _ = face_store.save_all_yaml(&catalog_root);
        Ok::<_, anyhow::Error>(())
    }).await;

    match result {
        Ok(Ok(())) => Json(serde_json::json!({"ok": true})).into_response(),
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": format!("{e:#}")}))).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": format!("{e}")}))).into_response(),
    }
}

/// GET /people — people gallery page.
pub async fn people_page(
    State(state): State<Arc<AppState>>,
) -> Response {
    let result = tokio::task::spawn_blocking(move || {
        let catalog = state.catalog()?;
        let face_store = crate::face_store::FaceStore::new(catalog.conn());
        let people_list = face_store.list_people()?;

        let people: Vec<PersonCard> = people_list.into_iter().map(|(p, count)| {
            let crop_url = p.representative_face_id.as_ref().and_then(|fid| {
                if crate::face::face_crop_exists(fid, &state.catalog_root) {
                    Some(format!("/face/{}/{}.jpg", &fid[..2.min(fid.len())], fid))
                } else {
                    None
                }
            });
            PersonCard {
                name: p.name.unwrap_or_else(|| format!("Unknown ({})", &p.id[..8.min(p.id.len())])),
                id: p.id,
                face_count: count,
                crop_url,
            }
        }).collect();

        let tmpl = PeoplePage {
            people,
            ai_enabled: true,
            vlm_enabled: state.vlm_enabled,
        };
        Ok::<_, anyhow::Error>(tmpl.render()?)
    }).await;

    match result {
        Ok(Ok(html)) => Html(html).into_response(),
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e}")).into_response(),
    }
}

/// GET /api/people — JSON list of people (for dropdown).
pub async fn list_people_api(
    State(state): State<Arc<AppState>>,
) -> Response {
    let result = tokio::task::spawn_blocking(move || {
        let catalog = state.catalog()?;
        let face_store = crate::face_store::FaceStore::new(catalog.conn());
        let people = face_store.list_people()?;
        let json: Vec<serde_json::Value> = people.into_iter().map(|(p, count)| {
            serde_json::json!({
                "id": p.id,
                "name": p.name,
                "face_count": count,
                "representative_face_id": p.representative_face_id,
            })
        }).collect();
        Ok::<_, anyhow::Error>(json)
    }).await;

    match result {
        Ok(Ok(json)) => Json(json).into_response(),
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e}")).into_response(),
    }
}

/// POST /api/people — create a new person.
pub async fn create_person_api(
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    let name = body.get("name").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
    if name.is_empty() {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": "name is required"}))).into_response();
    }
    let result = tokio::task::spawn_blocking(move || {
        let catalog = state.catalog()?;
        let face_store = crate::face_store::FaceStore::new(catalog.conn());
        let id = face_store.create_person(Some(&name))?;
        let _ = face_store.save_all_yaml(&state.catalog_root);
        Ok::<_, anyhow::Error>(serde_json::json!({"id": id, "name": name}))
    }).await;

    match result {
        Ok(Ok(json)) => Json(json).into_response(),
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": format!("{e:#}")}))).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": format!("{e}")}))).into_response(),
    }
}

/// PUT /api/people/{id}/name — rename a person.
pub async fn name_person_api(
    State(state): State<Arc<AppState>>,
    Path(person_id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    let name: String = match body.get("name").and_then(|v| v.as_str()) {
        Some(n) => n.to_string(),
        None => return (StatusCode::BAD_REQUEST, "Missing name").into_response(),
    };

    let result = tokio::task::spawn_blocking(move || {
        let catalog = state.catalog()?;
        let face_store = crate::face_store::FaceStore::new(catalog.conn());
        face_store.name_person(&person_id, &name)?;
        let _ = face_store.save_all_yaml(&state.catalog_root);
        state.dropdown_cache.invalidate_people();
        Ok::<_, anyhow::Error>(())
    }).await;

    match result {
        Ok(Ok(())) => Json(serde_json::json!({"ok": true})).into_response(),
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e}")).into_response(),
    }
}

/// POST /api/people/{id}/merge — merge source people into target.
pub async fn merge_person_api(
    State(state): State<Arc<AppState>>,
    Path(target_id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    let source_ids: Vec<String> = if let Some(arr) = body.get("source_ids").and_then(|v| v.as_array()) {
        arr.iter().filter_map(|v| v.as_str().map(String::from)).collect()
    } else if let Some(s) = body.get("source_id").and_then(|v| v.as_str()) {
        vec![s.to_string()]
    } else {
        return (StatusCode::BAD_REQUEST, "Missing source_id or source_ids").into_response();
    };
    if source_ids.is_empty() {
        return (StatusCode::BAD_REQUEST, "Empty source_ids").into_response();
    }

    let result = tokio::task::spawn_blocking(move || {
        let catalog = state.catalog()?;
        let face_store = crate::face_store::FaceStore::new(catalog.conn());
        let moved = face_store.merge_people_batch(&target_id, &source_ids)?;
        let _ = face_store.save_all_yaml(&state.catalog_root);
        state.dropdown_cache.invalidate_people();
        Ok::<_, anyhow::Error>((moved, source_ids.len()))
    }).await;

    match result {
        Ok(Ok((moved, n))) => Json(serde_json::json!({"ok": true, "faces_moved": moved, "people_merged": n})).into_response(),
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e}")).into_response(),
    }
}

/// DELETE /api/people/{id} — delete a person.
pub async fn delete_person_api(
    State(state): State<Arc<AppState>>,
    Path(person_id): Path<String>,
) -> Response {
    let result = tokio::task::spawn_blocking(move || {
        let catalog = state.catalog()?;
        let face_store = crate::face_store::FaceStore::new(catalog.conn());
        face_store.delete_person(&person_id)?;
        let _ = face_store.save_all_yaml(&state.catalog_root);
        state.dropdown_cache.invalidate_people();
        Ok::<_, anyhow::Error>(())
    }).await;

    match result {
        Ok(Ok(())) => Json(serde_json::json!({"ok": true})).into_response(),
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e}")).into_response(),
    }
}

/// GET /api/people/merge-suggestions — find pairs likely to be the same person.
pub async fn merge_suggestions_api(
    State(state): State<Arc<AppState>>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Response {
    let threshold: f32 = params.get("threshold").and_then(|s| s.parse().ok()).unwrap_or(0.4);
    let limit: usize = params.get("limit").and_then(|s| s.parse().ok()).unwrap_or(20);

    let result = tokio::task::spawn_blocking(move || {
        let catalog = state.catalog()?;
        let face_store = crate::face_store::FaceStore::new(catalog.conn());
        let pairs = face_store.suggest_person_merges(threshold, limit)?;

        let people = face_store.list_people()?;
        let lookup: std::collections::HashMap<String, (Option<String>, usize, Option<String>)> =
            people.into_iter().map(|(p, count)| (p.id.clone(), (p.name, count, p.representative_face_id))).collect();

        let items: Vec<serde_json::Value> = pairs.into_iter().filter_map(|(a, b, sim)| {
            let info_a = lookup.get(&a)?;
            let info_b = lookup.get(&b)?;
            let crop_for = |fid: &Option<String>| -> Option<String> {
                fid.as_ref().and_then(|f| {
                    if crate::face::face_crop_exists(f, &state.catalog_root) {
                        Some(format!("/face/{}/{}.jpg", &f[..2.min(f.len())], f))
                    } else { None }
                })
            };
            Some(serde_json::json!({
                "similarity": sim,
                "a": {
                    "id": a,
                    "name": info_a.0.clone().unwrap_or_else(|| format!("Unknown ({})", &a[..8.min(a.len())])),
                    "face_count": info_a.1,
                    "crop_url": crop_for(&info_a.2),
                    "named": info_a.0.is_some(),
                },
                "b": {
                    "id": b,
                    "name": info_b.0.clone().unwrap_or_else(|| format!("Unknown ({})", &b[..8.min(b.len())])),
                    "face_count": info_b.1,
                    "crop_url": crop_for(&info_b.2),
                    "named": info_b.0.is_some(),
                },
            }))
        }).collect();

        Ok::<_, anyhow::Error>(items)
    }).await;

    match result {
        Ok(Ok(items)) => Json(serde_json::json!({"suggestions": items})).into_response(),
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e}")).into_response(),
    }
}

/// POST /api/faces/cluster — run auto-clustering.
pub async fn cluster_faces_api(
    State(state): State<Arc<AppState>>,
) -> Response {
    let result = tokio::task::spawn_blocking(move || {
        let catalog = state.catalog()?;
        let _ = crate::face_store::FaceStore::initialize(catalog.conn());
        let face_store = crate::face_store::FaceStore::new(catalog.conn());
        let threshold = state.ai_config.face_cluster_threshold;
        let min_confidence = state.ai_config.face_min_confidence;
        let result = face_store.auto_cluster(threshold, min_confidence, None)?;
        let _ = face_store.save_all_yaml(&state.catalog_root);
        state.dropdown_cache.invalidate_people();
        Ok::<_, anyhow::Error>(result)
    }).await;

    match result {
        Ok(Ok(result)) => Json(serde_json::json!({
            "people_created": result.people_created,
            "faces_assigned": result.faces_assigned,
            "singletons_skipped": result.singletons_skipped,
        })).into_response(),
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e}")).into_response(),
    }
}
