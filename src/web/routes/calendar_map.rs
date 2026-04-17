//! Calendar heatmap and map marker API routes.

use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;

use super::super::AppState;
use super::{build_parsed_search, resolve_collection_ids, SearchParams};
#[cfg(feature = "ai")]
use super::intersect_name_groups;

#[derive(Debug, serde::Deserialize)]
pub struct CalendarParams {
    pub year: Option<i32>,
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
    pub stacks: Option<String>,
    pub person: Option<String>,
    pub nodefault: Option<String>,
}

/// GET /api/calendar — calendar heatmap data.
pub async fn calendar_api(
    State(state): State<Arc<AppState>>,
    Query(params): Query<CalendarParams>,
) -> Response {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let catalog = state.catalog()?;

        let year = params.year.unwrap_or_else(|| {
            chrono::Utc::now().format("%Y").to_string().parse::<i32>().unwrap_or(2026)
        });

        let search_params = SearchParams {
            q: params.q.clone(),
            asset_type: params.asset_type.clone(),
            tag: params.tag.clone(),
            format: params.format.clone(),
            volume: params.volume.clone(),
            rating: params.rating.clone(),
            label: params.label.clone(),
            collection: params.collection.clone(),
            path: params.path.clone(),
            person: params.person.clone(),
            sort: None,
            page: None,
            stacks: params.stacks.clone(),
            nodefault: params.nodefault.clone(),
        };
        let bf = build_parsed_search(&search_params, &state);
        let parsed = bf.parsed;
        let volume = bf.volume;
        let path_volume_id = bf.path_volume_id;
        let collapse_stacks = bf.collapse_stacks;

        let mut opts = parsed.to_search_options();
        if !volume.is_empty() {
            opts.volume = Some(&volume);
        }
        if let Some(ref vid) = path_volume_id {
            if opts.volume.is_none() {
                opts.volume = Some(vid);
            }
        }
        opts.collapse_stacks = collapse_stacks;

        let collection_ids;
        if !parsed.collections.is_empty() {
            collection_ids = resolve_collection_ids(&parsed.collections, catalog.conn());
            opts.collection_asset_ids = Some(&collection_ids);
        }

        let collection_exclude_ids;
        if !parsed.collections_exclude.is_empty() {
            collection_exclude_ids = resolve_collection_ids(&parsed.collections_exclude, catalog.conn());
            opts.collection_exclude_ids = Some(&collection_exclude_ids);
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

        let counts = catalog.calendar_counts(year, &opts)?;
        let years = catalog.calendar_years()?;

        Ok::<_, anyhow::Error>(serde_json::json!({
            "year": year,
            "counts": counts,
            "years": years,
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

#[derive(Debug, serde::Deserialize)]
pub struct MapParams {
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
    pub stacks: Option<String>,
    pub limit: Option<u32>,
    pub person: Option<String>,
    pub nodefault: Option<String>,
}

/// GET /api/map — map markers for geotagged assets.
pub async fn map_api(
    State(state): State<Arc<AppState>>,
    Query(params): Query<MapParams>,
) -> Response {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let catalog = state.catalog()?;

        let limit = params.limit.unwrap_or(10_000);

        let search_params = SearchParams {
            q: params.q.clone(),
            asset_type: params.asset_type.clone(),
            tag: params.tag.clone(),
            format: params.format.clone(),
            volume: params.volume.clone(),
            rating: params.rating.clone(),
            label: params.label.clone(),
            collection: params.collection.clone(),
            path: params.path.clone(),
            person: params.person.clone(),
            sort: None,
            page: None,
            stacks: params.stacks.clone(),
            nodefault: params.nodefault.clone(),
        };
        let bf = build_parsed_search(&search_params, &state);
        let parsed = bf.parsed;
        let volume = bf.volume;
        let path_volume_id = bf.path_volume_id;
        let collapse_stacks = bf.collapse_stacks;

        let mut opts = parsed.to_search_options();
        if !volume.is_empty() {
            opts.volume = Some(&volume);
        }
        if let Some(ref vid) = path_volume_id {
            if opts.volume.is_none() {
                opts.volume = Some(vid);
            }
        }
        opts.collapse_stacks = collapse_stacks;

        let collection_ids;
        if !parsed.collections.is_empty() {
            collection_ids = resolve_collection_ids(&parsed.collections, catalog.conn());
            opts.collection_asset_ids = Some(&collection_ids);
        }

        let collection_exclude_ids;
        if !parsed.collections_exclude.is_empty() {
            collection_exclude_ids = resolve_collection_ids(&parsed.collections_exclude, catalog.conn());
            opts.collection_exclude_ids = Some(&collection_exclude_ids);
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

        let preview_ext = &state.preview_ext;
        let (markers, total) = catalog.map_markers(&opts, limit)?;

        let markers_json: Vec<serde_json::Value> = markers.iter().map(|m| {
            let preview_url = m.preview.as_ref().map(|h| {
                crate::web::templates::preview_url(h, preview_ext)
            });
            serde_json::json!({
                "id": m.id,
                "lat": m.lat,
                "lng": m.lng,
                "preview": preview_url,
                "name": m.name,
                "rating": m.rating,
                "label": m.label,
            })
        }).collect();

        Ok::<_, anyhow::Error>(serde_json::json!({
            "markers": markers_json,
            "total": total,
            "truncated": total > limit as u64,
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
