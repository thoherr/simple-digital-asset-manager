use std::sync::Arc;

use askama::Template;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{Html, IntoResponse, Response};
use axum::{Form, Json};

use crate::catalog::SearchSort;
use crate::query::{normalize_path_for_search, parse_search_query};

use crate::device_registry::DeviceRegistry;

use super::templates::{
    format_size, link_cards, AssetCard, AssetPage, BackupPage, BrowsePage, CollectionOption,
    CompareAsset, ComparePage, DateFragment, DescriptionFragment, DuplicatesPage, FaceRow,
    FormatGroup, FormatOption, LabelFragment, NameFragment, PeoplePage, PersonCard, PersonOption,
    PreviewFragment, RatingFragment, ResultsPartial, SavedSearchChip, SavedSearchEntry,
    SavedSearchesPage, StackMemberCard, StatsPage, TagOption, TagTreeEntry, TagsFragment,
    TagsPage, VolumeOption,
};
use super::AppState;

#[derive(Debug, serde::Deserialize)]
pub struct SearchParams {
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
    pub sort: Option<String>,
    pub page: Option<u32>,
    pub stacks: Option<String>,
}

/// GET / — browse page with initial results (full page for browser, partial for htmx).
pub async fn browse_page(
    State(state): State<Arc<AppState>>,
    Query(params): Query<SearchParams>,
    headers: HeaderMap,
) -> Response {
    let is_htmx = headers.get("HX-Request").is_some();
    let preview_ext = state.preview_ext.clone();
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let catalog = state.catalog()?;

        let query = params.q.as_deref().unwrap_or("");
        let asset_type = params.asset_type.as_deref().unwrap_or("");
        let tag = params.tag.as_deref().unwrap_or("");
        let format = params.format.as_deref().unwrap_or("");
        let volume = params.volume.as_deref().unwrap_or("");
        let rating_str = params.rating.as_deref().unwrap_or("");
        let label_str = params.label.as_deref().unwrap_or("");
        let sort_str = params.sort.as_deref().unwrap_or("date_desc");
        let page = params.page.unwrap_or(1).max(1);

        let collection_str = params.collection.as_deref().unwrap_or("");
        let path_str = params.path.as_deref().unwrap_or("");
        let person_str = params.person.as_deref().unwrap_or("");
        let collapse_stacks = params.stacks.as_deref().unwrap_or("1") == "1";

        let mut parsed = merge_search_params(query, asset_type, tag, format, rating_str, label_str);

        // Normalize absolute path → volume-relative + implicit volume filter
        let path_volume_id = if !path_str.is_empty() {
            let registry = DeviceRegistry::new(&state.catalog_root);
            let vols = registry.list().unwrap_or_default();
            let (normalized, vol_id) = normalize_path_for_search(path_str, &vols, None);
            if !normalized.is_empty() {
                parsed.path_prefixes.push(normalized);
            }
            vol_id
        } else {
            None
        };

        // Push collection from dropdown into parsed struct
        if !collection_str.is_empty() {
            parsed.collections.push(collection_str.to_string());
        }

        // Push person from dropdown into parsed struct
        if !person_str.is_empty() {
            parsed.persons.push(person_str.to_string());
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

        // Resolve collection filter to asset IDs
        let collection_ids;
        if !parsed.collections.is_empty() {
            let col_store = crate::collection::CollectionStore::new(catalog.conn());
            let mut all_ids = std::collections::HashSet::new();
            for col_entry in parsed.collections.iter() {
                for col_name in col_entry.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()) {
                    if let Ok(ids) = col_store.asset_ids_for_collection(col_name) {
                        all_ids.extend(ids);
                    }
                }
            }
            collection_ids = all_ids.into_iter().collect::<Vec<_>>();
            opts.collection_asset_ids = Some(&collection_ids);
        }

        // Resolve collection exclude IDs
        let collection_exclude_ids;
        if !parsed.collections_exclude.is_empty() {
            let col_store = crate::collection::CollectionStore::new(catalog.conn());
            let mut all_ids = std::collections::HashSet::new();
            for col_entry in parsed.collections_exclude.iter() {
                for col_name in col_entry.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()) {
                    if let Ok(ids) = col_store.asset_ids_for_collection(col_name) {
                        all_ids.extend(ids);
                    }
                }
            }
            collection_exclude_ids = all_ids.into_iter().collect::<Vec<_>>();
            opts.collection_exclude_ids = Some(&collection_exclude_ids);
        }

        // Resolve person filter to asset IDs
        let person_ids;
        if !parsed.persons.is_empty() {
            #[cfg(feature = "ai")]
            {
                let face_store = crate::face_store::FaceStore::new(catalog.conn());
                let mut all_ids = std::collections::HashSet::new();
                for person_entry in parsed.persons.iter() {
                    for person_name in person_entry.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()) {
                        if let Ok(ids) = face_store.find_person_asset_ids(person_name) {
                            all_ids.extend(ids);
                        }
                    }
                }
                person_ids = all_ids.into_iter().collect::<Vec<_>>();
                opts.person_asset_ids = Some(&person_ids);
            }
            #[cfg(not(feature = "ai"))]
            {
                person_ids = Vec::<String>::new();
                opts.person_asset_ids = Some(&person_ids);
            }
        }

        let per_page = state.per_page;
        opts.sort = SearchSort::from_str(sort_str);
        opts.page = page;
        opts.per_page = per_page;
        opts.collapse_stacks = collapse_stacks;

        let total = catalog.search_count(&opts)?;
        let rows = catalog.search_paginated(&opts)?;
        let total_pages = ((total as f64) / per_page as f64).ceil() as u32;
        let mut cards: Vec<AssetCard> = rows.iter().map(|r| AssetCard::from_row(r, &preview_ext)).collect();
        link_cards(&mut cards);

        if is_htmx {
            let tmpl = ResultsPartial {
                query: query.to_string(),
                asset_type: asset_type.to_string(),
                tag: tag.to_string(),
                format_filter: format.to_string(),
                volume: volume.to_string(),
                rating: rating_str.to_string(),
                label: label_str.to_string(),
                collection: collection_str.to_string(),
                path: path_str.to_string(),
                sort: sort_str.to_string(),
                cards,
                total,
                page,
                per_page,
                total_pages,
                collapse_stacks,
            };
            return Ok::<_, anyhow::Error>(tmpl.render()?);
        }

        let all_tags: Vec<TagOption> = state.dropdown_cache.get_tags(&catalog)
            .into_iter()
            .map(|(name, count)| TagOption { name, count })
            .collect();
        let format_groups = build_format_groups(state.dropdown_cache.get_formats(&catalog));
        let all_volumes: Vec<VolumeOption> = state.dropdown_cache.get_volumes(&catalog)
            .into_iter()
            .map(|(id, label)| VolumeOption { id, label })
            .collect();
        let all_collections: Vec<CollectionOption> = state.dropdown_cache.get_collections(&catalog)
            .into_iter()
            .map(|name| CollectionOption { name })
            .collect();

        #[cfg(feature = "ai")]
        let all_people: Vec<PersonOption> = state.dropdown_cache.get_people(&catalog)
            .into_iter()
            .map(|(id, name)| PersonOption { id, name })
            .collect();
        #[cfg(not(feature = "ai"))]
        let all_people: Vec<PersonOption> = Vec::new();

        let saved_searches = crate::saved_search::load(&state.catalog_root)
            .unwrap_or_default()
            .searches
            .into_iter()
            .filter(|ss| ss.favorite)
            .map(|ss| {
                let url_params = ss.to_url_params();
                SavedSearchChip {
                    name: ss.name,
                    url_params,
                }
            })
            .collect();

        let tmpl = BrowsePage {
            query: query.to_string(),
            asset_type: asset_type.to_string(),
            tag: tag.to_string(),
            format_filter: format.to_string(),
            volume: volume.to_string(),
            rating: rating_str.to_string(),
            label: label_str.to_string(),
            sort: sort_str.to_string(),
            cards,
            total,
            page,
            per_page,
            total_pages,
            all_tags,
            format_groups,
            all_volumes,
            all_collections,
            all_people,
            collection: collection_str.to_string(),
            path: path_str.to_string(),
            person: person_str.to_string(),
            saved_searches,
            collapse_stacks,
            ai_enabled: state.ai_enabled,
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

/// GET /api/search — redirects non-htmx requests to browse page, returns partial for htmx.
pub async fn search_api(
    State(state): State<Arc<AppState>>,
    Query(params): Query<SearchParams>,
    headers: HeaderMap,
    uri: Uri,
) -> Response {
    // Non-htmx requests (direct browser load, reload, back button) get redirected
    // to the browse page which renders the full HTML with layout and CSS.
    if headers.get("HX-Request").is_none() {
        let query_string = uri.query().unwrap_or("");
        let redirect_url = if query_string.is_empty() {
            "/".to_string()
        } else {
            format!("/?{query_string}")
        };
        return axum::response::Redirect::to(&redirect_url).into_response();
    }

    let preview_ext = state.preview_ext.clone();
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let catalog = state.catalog()?;

        let query = params.q.as_deref().unwrap_or("");
        let asset_type = params.asset_type.as_deref().unwrap_or("");
        let tag = params.tag.as_deref().unwrap_or("");
        let format = params.format.as_deref().unwrap_or("");
        let volume = params.volume.as_deref().unwrap_or("");
        let rating_str = params.rating.as_deref().unwrap_or("");
        let label_str = params.label.as_deref().unwrap_or("");
        let sort_str = params.sort.as_deref().unwrap_or("date_desc");
        let page = params.page.unwrap_or(1).max(1);

        let collection_str = params.collection.as_deref().unwrap_or("");
        let path_str = params.path.as_deref().unwrap_or("");
        let person_str = params.person.as_deref().unwrap_or("");
        let collapse_stacks = params.stacks.as_deref().unwrap_or("1") == "1";

        let mut parsed = merge_search_params(query, asset_type, tag, format, rating_str, label_str);

        // Normalize absolute path → volume-relative + implicit volume filter
        let path_volume_id = if !path_str.is_empty() {
            let registry = DeviceRegistry::new(&state.catalog_root);
            let vols = registry.list().unwrap_or_default();
            let (normalized, vol_id) = normalize_path_for_search(path_str, &vols, None);
            if !normalized.is_empty() {
                parsed.path_prefixes.push(normalized);
            }
            vol_id
        } else {
            None
        };

        // Push collection from dropdown into parsed struct
        if !collection_str.is_empty() {
            parsed.collections.push(collection_str.to_string());
        }

        // Push person from dropdown into parsed struct
        if !person_str.is_empty() {
            parsed.persons.push(person_str.to_string());
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

        // Resolve collection filter to asset IDs
        let collection_ids;
        if !parsed.collections.is_empty() {
            let col_store = crate::collection::CollectionStore::new(catalog.conn());
            let mut all_ids = std::collections::HashSet::new();
            for col_entry in parsed.collections.iter() {
                for col_name in col_entry.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()) {
                    if let Ok(ids) = col_store.asset_ids_for_collection(col_name) {
                        all_ids.extend(ids);
                    }
                }
            }
            collection_ids = all_ids.into_iter().collect::<Vec<_>>();
            opts.collection_asset_ids = Some(&collection_ids);
        }

        // Resolve collection exclude IDs
        let collection_exclude_ids;
        if !parsed.collections_exclude.is_empty() {
            let col_store = crate::collection::CollectionStore::new(catalog.conn());
            let mut all_ids = std::collections::HashSet::new();
            for col_entry in parsed.collections_exclude.iter() {
                for col_name in col_entry.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()) {
                    if let Ok(ids) = col_store.asset_ids_for_collection(col_name) {
                        all_ids.extend(ids);
                    }
                }
            }
            collection_exclude_ids = all_ids.into_iter().collect::<Vec<_>>();
            opts.collection_exclude_ids = Some(&collection_exclude_ids);
        }

        // Resolve person filter to asset IDs
        let person_ids;
        if !parsed.persons.is_empty() {
            #[cfg(feature = "ai")]
            {
                let face_store = crate::face_store::FaceStore::new(catalog.conn());
                let mut all_ids = std::collections::HashSet::new();
                for person_entry in parsed.persons.iter() {
                    for person_name in person_entry.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()) {
                        if let Ok(ids) = face_store.find_person_asset_ids(person_name) {
                            all_ids.extend(ids);
                        }
                    }
                }
                person_ids = all_ids.into_iter().collect::<Vec<_>>();
                opts.person_asset_ids = Some(&person_ids);
            }
            #[cfg(not(feature = "ai"))]
            {
                person_ids = Vec::<String>::new();
                opts.person_asset_ids = Some(&person_ids);
            }
        }

        let per_page = state.per_page;
        opts.sort = SearchSort::from_str(sort_str);
        opts.page = page;
        opts.per_page = per_page;
        opts.collapse_stacks = collapse_stacks;

        let total = catalog.search_count(&opts)?;
        let rows = catalog.search_paginated(&opts)?;
        let total_pages = ((total as f64) / per_page as f64).ceil() as u32;
        let mut cards: Vec<AssetCard> = rows.iter().map(|r| AssetCard::from_row(r, &preview_ext)).collect();
        link_cards(&mut cards);

        let tmpl = ResultsPartial {
            query: query.to_string(),
            asset_type: asset_type.to_string(),
            tag: tag.to_string(),
            format_filter: format.to_string(),
            volume: volume.to_string(),
            rating: rating_str.to_string(),
            label: label_str.to_string(),
            collection: collection_str.to_string(),
            path: path_str.to_string(),
            sort: sort_str.to_string(),
            cards,
            total,
            page,
            per_page,
            total_pages,
            collapse_stacks,
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

/// GET /api/page-ids — returns asset IDs for a given page (for cross-page navigation).
pub async fn page_ids_api(
    State(state): State<Arc<AppState>>,
    Query(params): Query<SearchParams>,
) -> Response {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let catalog = state.catalog()?;

        let query = params.q.as_deref().unwrap_or("");
        let asset_type = params.asset_type.as_deref().unwrap_or("");
        let tag = params.tag.as_deref().unwrap_or("");
        let format = params.format.as_deref().unwrap_or("");
        let volume = params.volume.as_deref().unwrap_or("");
        let rating_str = params.rating.as_deref().unwrap_or("");
        let label_str = params.label.as_deref().unwrap_or("");
        let sort_str = params.sort.as_deref().unwrap_or("date_desc");
        let page = params.page.unwrap_or(1).max(1);

        let collection_str = params.collection.as_deref().unwrap_or("");
        let path_str = params.path.as_deref().unwrap_or("");
        let person_str = params.person.as_deref().unwrap_or("");
        let collapse_stacks = params.stacks.as_deref().unwrap_or("1") == "1";

        let mut parsed = merge_search_params(query, asset_type, tag, format, rating_str, label_str);

        // Normalize absolute path → volume-relative + implicit volume filter
        let path_volume_id = if !path_str.is_empty() {
            let registry = DeviceRegistry::new(&state.catalog_root);
            let vols = registry.list().unwrap_or_default();
            let (normalized, vol_id) = normalize_path_for_search(path_str, &vols, None);
            if !normalized.is_empty() {
                parsed.path_prefixes.push(normalized);
            }
            vol_id
        } else {
            None
        };

        // Push collection from dropdown into parsed struct
        if !collection_str.is_empty() {
            parsed.collections.push(collection_str.to_string());
        }

        // Push person from dropdown into parsed struct
        if !person_str.is_empty() {
            parsed.persons.push(person_str.to_string());
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

        // Resolve collection filter to asset IDs
        let collection_ids;
        if !parsed.collections.is_empty() {
            let col_store = crate::collection::CollectionStore::new(catalog.conn());
            let mut all_ids = std::collections::HashSet::new();
            for col_entry in parsed.collections.iter() {
                for col_name in col_entry.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()) {
                    if let Ok(ids) = col_store.asset_ids_for_collection(col_name) {
                        all_ids.extend(ids);
                    }
                }
            }
            collection_ids = all_ids.into_iter().collect::<Vec<_>>();
            opts.collection_asset_ids = Some(&collection_ids);
        }

        // Resolve collection exclude IDs
        let collection_exclude_ids;
        if !parsed.collections_exclude.is_empty() {
            let col_store = crate::collection::CollectionStore::new(catalog.conn());
            let mut all_ids = std::collections::HashSet::new();
            for col_entry in parsed.collections_exclude.iter() {
                for col_name in col_entry.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()) {
                    if let Ok(ids) = col_store.asset_ids_for_collection(col_name) {
                        all_ids.extend(ids);
                    }
                }
            }
            collection_exclude_ids = all_ids.into_iter().collect::<Vec<_>>();
            opts.collection_exclude_ids = Some(&collection_exclude_ids);
        }

        // Resolve person filter to asset IDs
        let person_ids;
        if !parsed.persons.is_empty() {
            #[cfg(feature = "ai")]
            {
                let face_store = crate::face_store::FaceStore::new(catalog.conn());
                let mut all_ids = std::collections::HashSet::new();
                for person_entry in parsed.persons.iter() {
                    for person_name in person_entry.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()) {
                        if let Ok(ids) = face_store.find_person_asset_ids(person_name) {
                            all_ids.extend(ids);
                        }
                    }
                }
                person_ids = all_ids.into_iter().collect::<Vec<_>>();
                opts.person_asset_ids = Some(&person_ids);
            }
            #[cfg(not(feature = "ai"))]
            {
                person_ids = Vec::<String>::new();
                opts.person_asset_ids = Some(&person_ids);
            }
        }

        let per_page = state.per_page;
        opts.sort = SearchSort::from_str(sort_str);
        opts.page = page;
        opts.per_page = per_page;
        opts.collapse_stacks = collapse_stacks;

        let total = catalog.search_count(&opts)?;
        let total_pages = ((total as f64) / per_page as f64).ceil() as u32;
        let rows = catalog.search_paginated(&opts)?;
        let ids: Vec<String> = rows.iter().map(|r| r.asset_id.clone()).collect();

        Ok::<_, anyhow::Error>(serde_json::json!({
            "ids": ids,
            "page": page,
            "total_pages": total_pages,
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
pub struct AssetPageParams {
    pub prev: Option<String>,
    pub next: Option<String>,
}

/// GET /asset/{id} — asset detail page.
pub async fn asset_page(
    State(state): State<Arc<AppState>>,
    Path(asset_id): Path<String>,
    Query(nav_params): Query<AssetPageParams>,
) -> Response {
    let preview_ext = state.preview_ext.clone();
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let engine = state.query_engine();
        let details = engine.show(&asset_id)?;

        let preview_gen = state.preview_generator();
        let best = crate::models::variant::best_preview_index_details(&details.variants);
        let preview_url = best.and_then(|i| {
            let v = &details.variants[i];
            if preview_gen.has_preview(&v.content_hash) {
                Some(super::templates::preview_url(&v.content_hash, &preview_ext))
            } else {
                None
            }
        });
        let best_hash = best.map(|i| details.variants[i].content_hash.clone());
        let has_smart_preview = best_hash.as_ref().map_or(false, |h| preview_gen.has_smart_preview(h));
        let smart_preview_url = best_hash.as_ref().and_then(|h| {
            if has_smart_preview {
                Some(super::templates::smart_preview_url(h, &preview_ext))
            } else {
                None
            }
        });

        // Load collections this asset belongs to
        let (collections, stack_members, is_stack_pick) = {
            let catalog = state.catalog()?;
            let col_store = crate::collection::CollectionStore::new(catalog.conn());
            let cols = col_store.collections_for_asset(&asset_id).unwrap_or_default();

            let stack_store = crate::stack::StackStore::new(catalog.conn());
            let (members, is_pick) = match stack_store.stack_for_asset(&asset_id).unwrap_or(None) {
                Some((_sid, member_ids)) => {
                    let is_pick = member_ids.first().map_or(false, |id| id == &asset_id);
                    let mut cards = Vec::new();
                    for (i, mid) in member_ids.iter().enumerate() {
                        if mid == &asset_id { continue; }
                        // Load minimal info for stack member
                        let name = catalog.get_asset_name(mid).unwrap_or(None)
                            .unwrap_or_else(|| mid[..8.min(mid.len())].to_string());
                        let hash = catalog.get_asset_best_variant_hash(mid).unwrap_or(None);
                        let purl = hash.map(|h| super::templates::preview_url(&h, &preview_ext))
                            .unwrap_or_default();
                        cards.push(StackMemberCard {
                            asset_id: mid.clone(),
                            display_name: name,
                            preview_url: purl,
                            is_pick: i == 0,
                        });
                    }
                    (cards, is_pick)
                }
                None => (Vec::new(), false),
            };

            (cols, members, is_pick)
        };

        // Build volume online map for reveal-in-finder buttons
        let registry = DeviceRegistry::new(&state.catalog_root);
        let volume_online: std::collections::HashMap<String, bool> = registry
            .list()
            .unwrap_or_default()
            .iter()
            .map(|v| (v.id.to_string(), v.is_online))
            .collect();

        // Load faces for asset detail page
        let (faces, all_people_detail) = {
            #[cfg(feature = "ai")]
            {
                let cat2 = state.catalog()?;
                let face_store = crate::face_store::FaceStore::new(cat2.conn());
                let stored_faces = face_store.faces_for_asset(&asset_id).unwrap_or_default();
                let face_rows: Vec<FaceRow> = stored_faces.iter().map(|f| {
                    let crop_url = if crate::face::face_crop_exists(&f.id, &state.catalog_root) {
                        Some(format!("/face/{}/{}.jpg", &f.id[..2.min(f.id.len())], f.id))
                    } else {
                        None
                    };
                    let person_name = f.person_id.as_ref().and_then(|pid| {
                        face_store.get_person(pid).ok().flatten().and_then(|p| p.name)
                    });
                    FaceRow {
                        face_id: f.id.clone(),
                        crop_url,
                        confidence_pct: (f.confidence * 100.0) as u32,
                        person_name,
                        person_id: f.person_id.clone(),
                    }
                }).collect();
                let people: Vec<PersonOption> = state.dropdown_cache.get_people(&cat2)
                    .into_iter()
                    .map(|(id, name)| PersonOption { id, name })
                    .collect();
                (face_rows, people)
            }
            #[cfg(not(feature = "ai"))]
            {
                (Vec::<FaceRow>::new(), Vec::<PersonOption>::new())
            }
        };

        let mut tmpl = AssetPage::from_details(details, preview_url, smart_preview_url, has_smart_preview, collections, stack_members, is_stack_pick, &volume_online);
        tmpl.prev_id = nav_params.prev;
        tmpl.next_id = nav_params.next;
        tmpl.ai_enabled = state.ai_enabled;
        tmpl.faces = faces;
        tmpl.all_people = all_people_detail;
        Ok::<_, anyhow::Error>(tmpl.render()?)
    })
    .await;

    match result {
        Ok(Ok(html)) => Html(html).into_response(),
        Ok(Err(e)) => {
            let msg = format!("{e:#}");
            if msg.contains("No asset found") {
                (
                    StatusCode::NOT_FOUND,
                    Html(format!("<h1>Not Found</h1><p>{msg}</p>")),
                )
                    .into_response()
            } else {
                (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {msg}")).into_response()
            }
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
    }
}

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
        Ok(Err(e)) => {
            (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e:#}")).into_response()
        }
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
        Ok(Err(e)) => {
            (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e:#}")).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
    }
}

/// GET /api/tags — all tags as JSON (for autocomplete).
pub async fn tags_api(State(state): State<Arc<AppState>>) -> Response {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let catalog = state.catalog()?;
        let tags = state.dropdown_cache.get_tags(&catalog);
        Ok::<_, anyhow::Error>(tags)
    })
    .await;

    match result {
        Ok(Ok(tags)) => axum::Json(tags).into_response(),
        Ok(Err(e)) => {
            (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e:#}")).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
    }
}

/// GET /api/stats — catalog stats as JSON.
pub async fn stats_api(State(state): State<Arc<AppState>>) -> Response {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let catalog = state.catalog()?;
        let (assets, variants, recipes, total_size) = catalog.stats_overview()?;
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
        Ok(Err(e)) => {
            (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e:#}")).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
    }
}

/// Build a tree of tag entries from a flat list of (name, count) pairs.
/// Ensures ancestor paths exist and computes total counts including descendants.
fn build_tag_tree(flat_tags: &[(String, u64)]) -> Vec<TagTreeEntry> {
    use std::collections::BTreeMap;

    // Collect own counts into a sorted map
    let mut own_counts: BTreeMap<String, u64> = BTreeMap::new();
    for (name, count) in flat_tags {
        own_counts.insert(name.clone(), *count);
    }

    // Ensure ancestor paths exist (e.g. "animals|birds|eagles" creates "animals" and "animals|birds")
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

    // Compute total counts (own + all descendants)
    let sorted_names: Vec<String> = own_counts.keys().cloned().collect();
    let mut total_counts: BTreeMap<String, u64> = BTreeMap::new();
    for name in &sorted_names {
        let own = own_counts[name];
        total_counts.insert(name.clone(), own);
    }
    // Accumulate child counts into parents
    for name in sorted_names.iter().rev() {
        let total = total_counts[name];
        if let Some(pipe_pos) = name.rfind('|') {
            let parent = &name[..pipe_pos];
            if let Some(parent_total) = total_counts.get_mut(parent) {
                *parent_total += total;
            }
        }
    }

    // Determine which entries have children
    let mut has_children_set: std::collections::HashSet<String> = std::collections::HashSet::new();
    for name in &sorted_names {
        if let Some(pipe_pos) = name.rfind('|') {
            has_children_set.insert(name[..pipe_pos].to_string());
        }
    }

    // Flatten to Vec with depth; display uses `/` for hierarchy separator
    sorted_names
        .iter()
        .map(|name| {
            let depth = name.matches('|').count() as u32;
            let display = name
                .rsplit('|')
                .next()
                .unwrap_or(name)
                .to_string();
            let display_name = name.replace('|', "/");
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

/// GET /backup — backup status dashboard.
pub async fn backup_page(State(state): State<Arc<AppState>>) -> Response {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let catalog = state.catalog()?;
        let registry = DeviceRegistry::new(&state.catalog_root);
        let vol_list = registry.list()?;
        let volumes_info: Vec<(String, String, bool, Option<String>)> = vol_list
            .iter()
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

use crate::query::ParsedSearch;

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
fn build_format_groups(format_counts: Vec<(String, u64)>) -> Vec<FormatGroup> {
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

/// Parse the `q` param through `parse_search_query` and overlay explicit dropdown params.
/// Returns a `ParsedSearch` (owned) that can be converted to `SearchOptions` by the caller.
fn merge_search_params(
    query: &str,
    asset_type: &str,
    tag: &str,
    format: &str,
    rating_str: &str,
    label: &str,
) -> ParsedSearch {
    let mut parsed = parse_search_query(query);

    // Explicit dropdown params add to the parsed Vecs (don't replace)
    if !asset_type.is_empty() {
        parsed.asset_types.push(asset_type.to_string());
    }
    if !tag.is_empty() {
        parsed.tags.push(tag.to_string());
    }
    if !format.is_empty() {
        parsed.formats.push(format.to_string());
    }
    if !rating_str.is_empty() {
        let (rating_min, rating_exact) = parse_rating_filter(rating_str);
        if rating_min.is_some() {
            parsed.rating_min = rating_min;
            parsed.rating_exact = None;
        }
        if rating_exact.is_some() {
            parsed.rating_exact = rating_exact;
            parsed.rating_min = None;
        }
    }
    if !label.is_empty() {
        parsed.color_labels.push(label.to_string());
    }

    parsed
}

/// Parse a rating filter string into (rating_min, rating_exact).
/// "3+" → (Some(3), None), "5" → (None, Some(5)), "" → (None, None)
fn parse_rating_filter(s: &str) -> (Option<u8>, Option<u8>) {
    if s.is_empty() {
        return (None, None);
    }
    if let Some(num_str) = s.strip_suffix('+') {
        if let Ok(n) = num_str.parse::<u8>() {
            return (Some(n), None);
        }
    }
    if let Ok(n) = s.parse::<u8>() {
        return (None, Some(n));
    }
    (None, None)
}

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
        // Treat 0 as "clear rating"
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
        Ok(Err(e)) => {
            (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e:#}")).into_response()
        }
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
        // Treat empty string as "clear description"
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
        Ok(Err(e)) => {
            (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e:#}")).into_response()
        }
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
        // Treat empty string as "clear name"
        let name = form.name.filter(|s| !s.trim().is_empty());
        let new_name = engine.set_name(&asset_id, name)?;

        // Load fallback name from primary variant's filename
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
        Ok(Err(e)) => {
            (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e:#}")).into_response()
        }
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
            created_at: super::templates::format_date(&date_str),
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

// --- Batch operations ---

#[derive(Debug, serde::Deserialize)]
pub struct BatchRatingRequest {
    pub asset_ids: Vec<String>,
    pub rating: Option<u8>,
}

#[derive(Debug, serde::Deserialize)]
pub struct BatchTagRequest {
    pub asset_ids: Vec<String>,
    pub tags: Vec<String>,
    pub remove: bool,
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
        Ok(Err(e)) => {
            (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e:#}")).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
    }
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
        // Convert user-facing tag input to storage form
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
        Ok(Err(e)) => {
            (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e:#}")).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
    }
}

/// POST /api/asset/{id}/preview — regenerate preview + smart preview, return preview fragment.
pub async fn generate_preview(
    State(state): State<Arc<AppState>>,
    Path(asset_id): Path<String>,
) -> Response {
    let preview_ext = state.preview_ext.clone();
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let engine = state.query_engine();
        let details = engine.show(&asset_id)?;

        let best_idx = crate::models::variant::best_preview_index_details(&details.variants)
            .ok_or_else(|| anyhow::anyhow!("Asset has no variants"))?;
        let variant = &details.variants[best_idx];
        let content_hash = &variant.content_hash;
        let format = &variant.format;

        // Resolve the source file path from the first online location
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
                if path.exists() {
                    Some(path)
                } else {
                    None
                }
            })
            .ok_or_else(|| {
                anyhow::anyhow!("No online file location found for variant")
            })?;

        let preview_gen = state.preview_generator();
        preview_gen.regenerate(content_hash, &source_path, format)?;
        preview_gen.regenerate_smart(content_hash, &source_path, format)?;

        // Cache-bust URLs so browser shows the newly generated images
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let preview_url = if preview_gen.has_preview(content_hash) {
            let url = super::templates::preview_url(content_hash, &preview_ext);
            Some(format!("{url}?t={ts}"))
        } else {
            None
        };

        let has_smart = preview_gen.has_smart_preview(content_hash);
        let smart_url = if has_smart {
            let url = super::templates::smart_preview_url(content_hash, &preview_ext);
            Some(format!("{url}?t={ts}"))
        } else {
            None
        };

        let tmpl = PreviewFragment {
            asset_id,
            primary_preview_url: preview_url,
            smart_preview_url: smart_url,
            has_smart_preview: has_smart,
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

/// POST /api/asset/{id}/rotate — cycle preview rotation 90° CW, regenerate previews.
pub async fn set_rotation(
    State(state): State<Arc<AppState>>,
    Path(asset_id): Path<String>,
) -> Response {
    let preview_ext = state.preview_ext.clone();
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let engine = state.query_engine();
        let catalog = state.catalog()?;

        // Read current rotation from catalog
        let details = engine.show(&asset_id)?;
        let current: Option<u16> = catalog.conn().query_row(
            "SELECT preview_rotation FROM assets WHERE id = ?1",
            [&details.id],
            |r| {
                let val: Option<i64> = r.get(0)?;
                Ok(val.map(|v| v as u16))
            },
        ).unwrap_or(None);

        // Cycle: None→90→180→270→None
        let new_rotation = match current {
            None | Some(0) => Some(90u16),
            Some(90) => Some(180),
            Some(180) => Some(270),
            Some(270) => None,
            Some(_) => Some(90),
        };

        // Persist rotation
        engine.set_preview_rotation(&asset_id, new_rotation)?;

        // Find the best variant and resolve its source file
        let best_idx = crate::models::variant::best_preview_index_details(&details.variants)
            .ok_or_else(|| anyhow::anyhow!("Asset has no variants"))?;
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
            })
            .ok_or_else(|| anyhow::anyhow!("No online file location found for variant"))?;

        // Regenerate previews with the new rotation
        let preview_gen = state.preview_generator();
        preview_gen.regenerate_with_rotation(content_hash, &source_path, format, new_rotation)?;
        if preview_gen.has_smart_preview(content_hash) {
            preview_gen.regenerate_smart_with_rotation(content_hash, &source_path, format, new_rotation)?;
        }

        // Build response with cache-busted URLs
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let preview_url = if preview_gen.has_preview(content_hash) {
            let url = super::templates::preview_url(content_hash, &preview_ext);
            Some(format!("{url}?t={ts}"))
        } else {
            None
        };

        let has_smart = preview_gen.has_smart_preview(content_hash);
        let smart_url = if has_smart {
            let url = super::templates::smart_preview_url(content_hash, &preview_ext);
            Some(format!("{url}?t={ts}"))
        } else {
            None
        };

        let tmpl = PreviewFragment {
            asset_id,
            primary_preview_url: preview_url,
            smart_preview_url: smart_url,
            has_smart_preview: has_smart,
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
        // Treat empty string as "clear label"
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
        Ok(Err(e)) => {
            (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e:#}")).into_response()
        }
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
        Ok(Err(e)) => {
            (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e:#}")).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
    }
}

// --- Saved search API ---

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
        Ok(Err(e)) => {
            (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e:#}")).into_response()
        }
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
        Ok(Err(e)) => {
            (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e:#}")).into_response()
        }
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
            anyhow::bail!("No saved search named '{name}'");
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

// --- Collections ---

/// GET /collections — collections HTML page.
pub async fn collections_page(State(state): State<Arc<AppState>>) -> Response {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let catalog = state.catalog()?;
        let col_store = crate::collection::CollectionStore::new(catalog.conn());
        let collections = col_store.list()?;
        let tmpl = super::templates::CollectionsPage { collections };
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
pub struct CreateCollectionRequest {
    pub name: String,
    pub description: Option<String>,
}

/// POST /api/collections — create a new collection.
pub async fn create_collection_api(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateCollectionRequest>,
) -> Response {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let name = req.name.trim().to_string();
        if name.is_empty() {
            anyhow::bail!("Collection name cannot be empty");
        }
        let description = req.description.as_deref().map(|s| s.trim()).filter(|s| !s.is_empty());
        let catalog = state.catalog()?;
        let col_store = crate::collection::CollectionStore::new(catalog.conn());
        let collection = col_store.create(&name, description)?;
        // Persist to YAML
        let yaml = col_store.export_all()?;
        crate::collection::save_yaml(&state.catalog_root, &yaml)?;
        state.dropdown_cache.invalidate_collections();
        Ok::<_, anyhow::Error>(serde_json::json!({
            "id": collection.id.to_string(),
            "name": collection.name,
            "description": collection.description,
        }))
    })
    .await;

    match result {
        Ok(Ok(json)) => (StatusCode::CREATED, Json(json)).into_response(),
        Ok(Err(e)) => {
            let msg = format!("{e:#}");
            if msg.contains("UNIQUE constraint") || msg.contains("already exists") {
                (StatusCode::CONFLICT, format!("Collection already exists: {msg}")).into_response()
            } else {
                (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {msg}")).into_response()
            }
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
    }
}

/// GET /api/collections — list all collections as JSON.
pub async fn list_collections_api(State(state): State<Arc<AppState>>) -> Response {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let catalog = state.catalog()?;
        let col_store = crate::collection::CollectionStore::new(catalog.conn());
        let collections = col_store.list()?;
        Ok::<_, anyhow::Error>(collections)
    })
    .await;

    match result {
        Ok(Ok(collections)) => Json(collections).into_response(),
        Ok(Err(e)) => {
            (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e:#}")).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
    }
}

#[derive(Debug, serde::Deserialize)]
pub struct BatchCollectionRequest {
    pub asset_ids: Vec<String>,
    pub collection: String,
}

/// DELETE /api/batch/collection — remove assets from a collection.
pub async fn batch_remove_from_collection(
    State(state): State<Arc<AppState>>,
    Json(req): Json<BatchCollectionRequest>,
) -> Response {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let catalog = state.catalog()?;
        let col_store = crate::collection::CollectionStore::new(catalog.conn());
        let removed = col_store.remove_assets(&req.collection, &req.asset_ids)?;
        // Persist to YAML
        let yaml = col_store.export_all()?;
        crate::collection::save_yaml(&state.catalog_root, &yaml)?;
        state.dropdown_cache.invalidate_collections();
        Ok::<_, anyhow::Error>(serde_json::json!({
            "removed": removed,
            "collection": req.collection,
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

/// POST /api/batch/collection — add assets to a collection.
pub async fn batch_add_to_collection(
    State(state): State<Arc<AppState>>,
    Json(req): Json<BatchCollectionRequest>,
) -> Response {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let catalog = state.catalog()?;
        let col_store = crate::collection::CollectionStore::new(catalog.conn());
        let added = col_store.add_assets(&req.collection, &req.asset_ids)?;
        // Persist to YAML
        let yaml = col_store.export_all()?;
        crate::collection::save_yaml(&state.catalog_root, &yaml)?;
        state.dropdown_cache.invalidate_collections();
        Ok::<_, anyhow::Error>(serde_json::json!({
            "added": added,
            "collection": req.collection,
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

#[derive(serde::Deserialize)]
pub struct BatchAutoGroupRequest {
    pub asset_ids: Vec<String>,
}

/// POST /api/batch/auto-group — auto-group selected assets by filename stem.
pub async fn batch_auto_group(
    State(state): State<Arc<AppState>>,
    Json(req): Json<BatchAutoGroupRequest>,
) -> Response {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let engine = state.query_engine();
        let result = engine.auto_group(&req.asset_ids, false)?;
        Ok::<_, anyhow::Error>(serde_json::json!({
            "groups_merged": result.groups.len(),
            "donors_removed": result.total_donors_merged,
            "variants_moved": result.total_variants_moved,
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

// --- Stack batch operations ---

#[derive(serde::Deserialize)]
pub struct BatchStackRequest {
    pub asset_ids: Vec<String>,
}

/// POST /api/batch/stack — create a stack from selected assets.
pub async fn batch_create_stack(
    State(state): State<Arc<AppState>>,
    Json(req): Json<BatchStackRequest>,
) -> Response {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let catalog = state.catalog()?;
        let store = crate::stack::StackStore::new(catalog.conn());
        let stack = store.create(&req.asset_ids)?;
        let yaml = store.export_all()?;
        crate::stack::save_yaml(&state.catalog_root, &yaml)?;
        Ok::<_, anyhow::Error>(serde_json::json!({
            "stack_id": stack.id.to_string(),
            "member_count": stack.asset_ids.len(),
        }))
    })
    .await;

    match result {
        Ok(Ok(json)) => Json(json).into_response(),
        Ok(Err(e)) => {
            (StatusCode::BAD_REQUEST, format!("Error: {e:#}")).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
    }
}

/// DELETE /api/batch/stack — unstack selected assets.
pub async fn batch_unstack(
    State(state): State<Arc<AppState>>,
    Json(req): Json<BatchStackRequest>,
) -> Response {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let catalog = state.catalog()?;
        let store = crate::stack::StackStore::new(catalog.conn());
        let removed = store.remove(&req.asset_ids)?;
        let yaml = store.export_all()?;
        crate::stack::save_yaml(&state.catalog_root, &yaml)?;
        Ok::<_, anyhow::Error>(serde_json::json!({
            "removed": removed,
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

/// PUT /api/asset/{id}/stack-pick — set this asset as the stack pick.
pub async fn set_stack_pick(
    State(state): State<Arc<AppState>>,
    Path(asset_id): Path<String>,
) -> Response {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let catalog = state.catalog()?;
        let store = crate::stack::StackStore::new(catalog.conn());
        store.set_pick(&asset_id)?;
        let yaml = store.export_all()?;
        crate::stack::save_yaml(&state.catalog_root, &yaml)?;
        Ok::<_, anyhow::Error>(serde_json::json!({ "pick": asset_id }))
    })
    .await;

    match result {
        Ok(Ok(json)) => Json(json).into_response(),
        Ok(Err(e)) => {
            (StatusCode::BAD_REQUEST, format!("Error: {e:#}")).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
    }
}

/// DELETE /api/asset/{id}/stack — dissolve the stack this asset belongs to.
pub async fn dissolve_stack(
    State(state): State<Arc<AppState>>,
    Path(asset_id): Path<String>,
) -> Response {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let catalog = state.catalog()?;
        let store = crate::stack::StackStore::new(catalog.conn());
        store.dissolve(&asset_id)?;
        let yaml = store.export_all()?;
        crate::stack::save_yaml(&state.catalog_root, &yaml)?;
        Ok::<_, anyhow::Error>(serde_json::json!({ "status": "dissolved" }))
    })
    .await;

    match result {
        Ok(Ok(json)) => Json(json).into_response(),
        Ok(Err(e)) => {
            (StatusCode::BAD_REQUEST, format!("Error: {e:#}")).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
    }
}

// --- Saved searches management ---

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
        let tmpl = SavedSearchesPage { searches };
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
            .ok_or_else(|| anyhow::anyhow!("No saved search named '{name}'"))?;
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
            anyhow::bail!("Name cannot be empty");
        }
        let mut file = crate::saved_search::load(&state.catalog_root)?;
        // Check for name collision
        if file.searches.iter().any(|s| s.name == new_name) {
            anyhow::bail!("A saved search named '{new_name}' already exists");
        }
        let entry = file
            .searches
            .iter_mut()
            .find(|s| s.name == name)
            .ok_or_else(|| anyhow::anyhow!("No saved search named '{name}'"))?;
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

// --- Calendar heatmap ---

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

        let query = params.q.as_deref().unwrap_or("");
        let person_str = params.person.as_deref().unwrap_or("");
        let asset_type = params.asset_type.as_deref().unwrap_or("");
        let tag = params.tag.as_deref().unwrap_or("");
        let format = params.format.as_deref().unwrap_or("");
        let volume = params.volume.as_deref().unwrap_or("");
        let rating_str = params.rating.as_deref().unwrap_or("");
        let label_str = params.label.as_deref().unwrap_or("");
        let collection_str = params.collection.as_deref().unwrap_or("");
        let path_str = params.path.as_deref().unwrap_or("");

        let mut parsed = merge_search_params(query, asset_type, tag, format, rating_str, label_str);

        // Normalize absolute path → volume-relative + implicit volume filter
        let path_volume_id = if !path_str.is_empty() {
            let registry = DeviceRegistry::new(&state.catalog_root);
            let vols = registry.list().unwrap_or_default();
            let (normalized, vol_id) = normalize_path_for_search(path_str, &vols, None);
            if !normalized.is_empty() {
                parsed.path_prefixes.push(normalized);
            }
            vol_id
        } else {
            None
        };

        // Push collection from dropdown into parsed struct
        if !collection_str.is_empty() {
            parsed.collections.push(collection_str.to_string());
        }

        // Push person from dropdown into parsed struct
        if !person_str.is_empty() {
            parsed.persons.push(person_str.to_string());
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

        // Collapse stacks (default: yes)
        let collapse_stacks = params.stacks.as_deref().unwrap_or("1") == "1";
        opts.collapse_stacks = collapse_stacks;

        // Resolve collection filter to asset IDs
        let collection_ids;
        if !parsed.collections.is_empty() {
            let col_store = crate::collection::CollectionStore::new(catalog.conn());
            let mut all_ids = std::collections::HashSet::new();
            for col_entry in parsed.collections.iter() {
                for col_name in col_entry.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()) {
                    if let Ok(ids) = col_store.asset_ids_for_collection(col_name) {
                        all_ids.extend(ids);
                    }
                }
            }
            collection_ids = all_ids.into_iter().collect::<Vec<_>>();
            opts.collection_asset_ids = Some(&collection_ids);
        }

        // Resolve collection exclude IDs
        let collection_exclude_ids;
        if !parsed.collections_exclude.is_empty() {
            let col_store = crate::collection::CollectionStore::new(catalog.conn());
            let mut all_ids = std::collections::HashSet::new();
            for col_entry in parsed.collections_exclude.iter() {
                for col_name in col_entry.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()) {
                    if let Ok(ids) = col_store.asset_ids_for_collection(col_name) {
                        all_ids.extend(ids);
                    }
                }
            }
            collection_exclude_ids = all_ids.into_iter().collect::<Vec<_>>();
            opts.collection_exclude_ids = Some(&collection_exclude_ids);
        }

        // Resolve person filter to asset IDs
        let person_ids;
        if !parsed.persons.is_empty() {
            #[cfg(feature = "ai")]
            {
                let face_store = crate::face_store::FaceStore::new(catalog.conn());
                let mut all_ids = std::collections::HashSet::new();
                for pe in parsed.persons.iter() {
                    for pn in pe.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()) {
                        if let Ok(ids) = face_store.find_person_asset_ids(pn) {
                            all_ids.extend(ids);
                        }
                    }
                }
                person_ids = all_ids.into_iter().collect::<Vec<_>>();
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

// --- Map ---

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
}

/// GET /api/map — map markers for geotagged assets.
pub async fn map_api(
    State(state): State<Arc<AppState>>,
    Query(params): Query<MapParams>,
) -> Response {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let catalog = state.catalog()?;

        let query = params.q.as_deref().unwrap_or("");
        let asset_type = params.asset_type.as_deref().unwrap_or("");
        let tag = params.tag.as_deref().unwrap_or("");
        let format = params.format.as_deref().unwrap_or("");
        let volume = params.volume.as_deref().unwrap_or("");
        let rating_str = params.rating.as_deref().unwrap_or("");
        let label_str = params.label.as_deref().unwrap_or("");
        let collection_str = params.collection.as_deref().unwrap_or("");
        let path_str = params.path.as_deref().unwrap_or("");
        let person_str = params.person.as_deref().unwrap_or("");
        let limit = params.limit.unwrap_or(10_000);

        let mut parsed = merge_search_params(query, asset_type, tag, format, rating_str, label_str);

        // Normalize absolute path → volume-relative + implicit volume filter
        let path_volume_id = if !path_str.is_empty() {
            let registry = DeviceRegistry::new(&state.catalog_root);
            let vols = registry.list().unwrap_or_default();
            let (normalized, vol_id) = normalize_path_for_search(path_str, &vols, None);
            if !normalized.is_empty() {
                parsed.path_prefixes.push(normalized);
            }
            vol_id
        } else {
            None
        };

        // Push collection from dropdown into parsed struct
        if !collection_str.is_empty() {
            parsed.collections.push(collection_str.to_string());
        }

        // Push person from dropdown into parsed struct
        if !person_str.is_empty() {
            parsed.persons.push(person_str.to_string());
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

        // Collapse stacks (default: yes)
        let collapse_stacks = params.stacks.as_deref().unwrap_or("1") == "1";
        opts.collapse_stacks = collapse_stacks;

        // Resolve collection filter to asset IDs
        let collection_ids;
        if !parsed.collections.is_empty() {
            let col_store = crate::collection::CollectionStore::new(catalog.conn());
            let mut all_ids = std::collections::HashSet::new();
            for col_entry in parsed.collections.iter() {
                for col_name in col_entry.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()) {
                    if let Ok(ids) = col_store.asset_ids_for_collection(col_name) {
                        all_ids.extend(ids);
                    }
                }
            }
            collection_ids = all_ids.into_iter().collect::<Vec<_>>();
            opts.collection_asset_ids = Some(&collection_ids);
        }

        // Resolve collection exclude IDs
        let collection_exclude_ids;
        if !parsed.collections_exclude.is_empty() {
            let col_store = crate::collection::CollectionStore::new(catalog.conn());
            let mut all_ids = std::collections::HashSet::new();
            for col_entry in parsed.collections_exclude.iter() {
                for col_name in col_entry.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()) {
                    if let Ok(ids) = col_store.asset_ids_for_collection(col_name) {
                        all_ids.extend(ids);
                    }
                }
            }
            collection_exclude_ids = all_ids.into_iter().collect::<Vec<_>>();
            opts.collection_exclude_ids = Some(&collection_exclude_ids);
        }

        // Resolve person filter to asset IDs
        let person_ids;
        if !parsed.persons.is_empty() {
            #[cfg(feature = "ai")]
            {
                let face_store = crate::face_store::FaceStore::new(catalog.conn());
                let mut all_ids = std::collections::HashSet::new();
                for pe in parsed.persons.iter() {
                    for pn in pe.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()) {
                        if let Ok(ids) = face_store.find_person_asset_ids(pn) {
                            all_ids.extend(ids);
                        }
                    }
                }
                person_ids = all_ids.into_iter().collect::<Vec<_>>();
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

        // Transform preview hashes to URLs
        let markers_json: Vec<serde_json::Value> = markers.iter().map(|m| {
            let preview_url = m.preview.as_ref().map(|h| {
                super::templates::preview_url(h, preview_ext)
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

// --- Facets ---

#[derive(Debug, serde::Deserialize)]
pub struct FacetParams {
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
}

/// GET /api/facets — facet counts for the browse sidebar.
pub async fn facets_api(
    State(state): State<Arc<AppState>>,
    Query(params): Query<FacetParams>,
) -> Response {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let catalog = state.catalog()?;

        let query = params.q.as_deref().unwrap_or("");
        let asset_type = params.asset_type.as_deref().unwrap_or("");
        let tag = params.tag.as_deref().unwrap_or("");
        let format = params.format.as_deref().unwrap_or("");
        let volume = params.volume.as_deref().unwrap_or("");
        let rating_str = params.rating.as_deref().unwrap_or("");
        let label_str = params.label.as_deref().unwrap_or("");
        let collection_str = params.collection.as_deref().unwrap_or("");
        let path_str = params.path.as_deref().unwrap_or("");
        let person_str = params.person.as_deref().unwrap_or("");

        let mut parsed = merge_search_params(query, asset_type, tag, format, rating_str, label_str);

        // Normalize absolute path → volume-relative + implicit volume filter
        let path_volume_id = if !path_str.is_empty() {
            let registry = DeviceRegistry::new(&state.catalog_root);
            let vols = registry.list().unwrap_or_default();
            let (normalized, vol_id) = normalize_path_for_search(path_str, &vols, None);
            if !normalized.is_empty() {
                parsed.path_prefixes.push(normalized);
            }
            vol_id
        } else {
            None
        };

        // Push collection from dropdown into parsed struct
        if !collection_str.is_empty() {
            parsed.collections.push(collection_str.to_string());
        }

        // Push person from dropdown into parsed struct
        if !person_str.is_empty() {
            parsed.persons.push(person_str.to_string());
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

        // Collapse stacks (default: yes)
        let collapse_stacks = params.stacks.as_deref().unwrap_or("1") == "1";
        opts.collapse_stacks = collapse_stacks;

        // Resolve collection filter to asset IDs
        let collection_ids;
        if !parsed.collections.is_empty() {
            let col_store = crate::collection::CollectionStore::new(catalog.conn());
            let mut all_ids = std::collections::HashSet::new();
            for col_entry in parsed.collections.iter() {
                for col_name in col_entry.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()) {
                    if let Ok(ids) = col_store.asset_ids_for_collection(col_name) {
                        all_ids.extend(ids);
                    }
                }
            }
            collection_ids = all_ids.into_iter().collect::<Vec<_>>();
            opts.collection_asset_ids = Some(&collection_ids);
        }

        // Resolve collection exclude IDs
        let collection_exclude_ids;
        if !parsed.collections_exclude.is_empty() {
            let col_store = crate::collection::CollectionStore::new(catalog.conn());
            let mut all_ids = std::collections::HashSet::new();
            for col_entry in parsed.collections_exclude.iter() {
                for col_name in col_entry.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()) {
                    if let Ok(ids) = col_store.asset_ids_for_collection(col_name) {
                        all_ids.extend(ids);
                    }
                }
            }
            collection_exclude_ids = all_ids.into_iter().collect::<Vec<_>>();
            opts.collection_exclude_ids = Some(&collection_exclude_ids);
        }

        // Resolve person filter to asset IDs
        let person_ids;
        if !parsed.persons.is_empty() {
            #[cfg(feature = "ai")]
            {
                let face_store = crate::face_store::FaceStore::new(catalog.conn());
                let mut all_ids = std::collections::HashSet::new();
                for pe in parsed.persons.iter() {
                    for pn in pe.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()) {
                        if let Ok(ids) = face_store.find_person_asset_ids(pn) {
                            all_ids.extend(ids);
                        }
                    }
                }
                person_ids = all_ids.into_iter().collect::<Vec<_>>();
                opts.person_asset_ids = Some(&person_ids);
            }
            #[cfg(not(feature = "ai"))]
            {
                person_ids = Vec::<String>::new();
                opts.person_asset_ids = Some(&person_ids);
            }
        }

        let facets = catalog.facet_counts(&opts)?;
        Ok::<_, anyhow::Error>(serde_json::json!(facets))
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

// --- Duplicates ---

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
                super::templates::preview_url(&entry.content_hash, &preview_ext);
        }

        let total_groups = entries.len();

        // Compute wasted space: for groups with same-volume dupes,
        // count file_size * (extra copies on same volume) per volume group
        let mut total_wasted: u64 = 0;
        let mut same_volume_count: usize = 0;
        for entry in &entries {
            if !entry.same_volume_groups.is_empty() {
                same_volume_count += 1;
                // Count extra same-volume locations
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

        // Safety check: count remaining locations for this hash
        let location_count: u64 = catalog.conn().query_row(
            "SELECT COUNT(*) FROM file_locations WHERE content_hash = ?1",
            rusqlite::params![req.content_hash],
            |row| row.get(0),
        )?;
        if location_count <= 1 {
            anyhow::bail!("Cannot remove the last copy of a file");
        }

        // Resolve volume and check it's online
        let registry = DeviceRegistry::new(&state.catalog_root);
        let volumes = registry.list()?;
        let vol = volumes
            .iter()
            .find(|v| v.id.to_string() == req.volume_id)
            .ok_or_else(|| anyhow::anyhow!("Volume not found: {}", req.volume_id))?;
        if !vol.is_online {
            anyhow::bail!("Volume '{}' is offline", vol.label);
        }

        // Delete the physical file
        let full_path = vol.mount_point.join(&req.relative_path);
        if full_path.exists() {
            std::fs::remove_file(&full_path).map_err(|e| {
                anyhow::anyhow!("Failed to delete {}: {e}", full_path.display())
            })?;
        }

        // Remove from catalog
        catalog.delete_file_location(&req.content_hash, &req.volume_id, &req.relative_path)?;

        // Update sidecar YAML
        let service = state.asset_service();
        let metadata_store = crate::metadata_store::MetadataStore::new(&state.catalog_root);
        let vol_uuid: uuid::Uuid = req.volume_id.parse().map_err(|e| {
            anyhow::anyhow!("Invalid volume ID '{}': {e}", req.volume_id)
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

        // Clean up co-located recipe files (same variant, same volume, same directory)
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
                        Some(super::templates::preview_url(&v.content_hash, &preview_ext))
                    } else {
                        None
                    }
                })
                .unwrap_or_default();

            assets.push(CompareAsset::from_details(&details, purl));
        }

        let tmpl = ComparePage { assets };
        Ok::<_, anyhow::Error>(tmpl.render()?)
    })
    .await;

    match result {
        Ok(Ok(html)) => Html(html).into_response(),
        Ok(Err(e)) => {
            let msg = format!("{e:#}");
            if msg.contains("No asset found") {
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

    // If the file already exists, serve it directly
    if file_path.exists() {
        return serve_smart_file(&file_path, &file).await;
    }

    // If on-demand generation is disabled, return 404
    if !state.smart_on_demand {
        return StatusCode::NOT_FOUND.into_response();
    }

    // Extract content hash from filename (strip extension, prepend sha256:)
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

        // Look up variant format
        let format = catalog
            .get_variant_format(&content_hash)?
            .ok_or_else(|| anyhow::anyhow!("Variant not found"))?;

        // Find an online source file
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
            .ok_or_else(|| anyhow::anyhow!("No online source file"))?;

        // Generate the smart preview
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
            .ok_or_else(|| anyhow::anyhow!("Volume not found"))?;

        if !vol.is_online {
            anyhow::bail!("Volume '{}' is offline", vol.label);
        }

        let full_path = vol.mount_point.join(&req.relative_path);
        if !full_path.exists() {
            anyhow::bail!("File not found on disk: {}", full_path.display());
        }

        // Platform-specific reveal
        #[cfg(target_os = "macos")]
        {
            std::process::Command::new("open")
                .arg("-R")
                .arg(&full_path)
                .spawn()
                .map_err(|e| anyhow::anyhow!("Failed to open Finder: {e}"))?;
        }
        #[cfg(target_os = "linux")]
        {
            if let Some(parent) = full_path.parent() {
                std::process::Command::new("xdg-open")
                    .arg(parent)
                    .spawn()
                    .map_err(|e| anyhow::anyhow!("Failed to open file manager: {e}"))?;
            }
        }
        #[cfg(target_os = "windows")]
        {
            std::process::Command::new("explorer")
                .arg("/select,")
                .arg(&full_path)
                .spawn()
                .map_err(|e| anyhow::anyhow!("Failed to open Explorer: {e}"))?;
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
        {
            anyhow::bail!("Reveal in file manager is not supported on this platform");
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
            .ok_or_else(|| anyhow::anyhow!("Volume not found"))?;

        if !vol.is_online {
            anyhow::bail!("Volume '{}' is offline", vol.label);
        }

        let full_path = vol.mount_point.join(&req.relative_path);
        let dir = if full_path.is_dir() {
            full_path
        } else {
            full_path.parent()
                .ok_or_else(|| anyhow::anyhow!("Cannot determine parent directory"))?
                .to_path_buf()
        };
        if !dir.exists() {
            anyhow::bail!("Directory not found on disk: {}", dir.display());
        }

        #[cfg(target_os = "macos")]
        {
            std::process::Command::new("open")
                .arg("-a")
                .arg("Terminal")
                .arg(&dir)
                .spawn()
                .map_err(|e| anyhow::anyhow!("Failed to open Terminal: {e}"))?;
        }
        #[cfg(target_os = "linux")]
        {
            // Try common terminal emulators in order of preference
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
                anyhow::bail!("No terminal emulator found");
            }
        }
        #[cfg(target_os = "windows")]
        {
            std::process::Command::new("cmd")
                .args(["/c", "start", "cmd"])
                .current_dir(&dir)
                .spawn()
                .map_err(|e| anyhow::anyhow!("Failed to open command prompt: {e}"))?;
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
        {
            anyhow::bail!("Open terminal is not supported on this platform");
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

// --- AI auto-tag endpoints (feature-gated) ---

#[cfg(feature = "ai")]
#[derive(Debug, serde::Serialize)]
pub struct SuggestTagsResponse {
    pub tag: String,
    pub confidence: f32,
    pub existing: bool,
}

/// POST /api/asset/{id}/suggest-tags — suggest tags for an asset using AI.
#[cfg(feature = "ai")]
pub async fn suggest_tags(
    State(state): State<Arc<AppState>>,
    Path(asset_id): Path<String>,
) -> Response {
    let state = state.clone();
    let result: Result<Result<Vec<SuggestTagsResponse>, String>, _> =
        tokio::task::spawn_blocking(move || {
            suggest_tags_inner(&state, &asset_id)
        })
        .await;

    match result {
        Ok(Ok(suggestions)) => Json(suggestions).into_response(),
        Ok(Err(msg)) => (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
    }
}

#[cfg(feature = "ai")]
fn resolve_model_dir(config: &crate::config::AiConfig) -> std::path::PathBuf {
    let model_dir_str = &config.model_dir;
    let model_base = if model_dir_str.starts_with("~/") {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_else(|_| ".".to_string());
        std::path::PathBuf::from(home).join(&model_dir_str[2..])
    } else {
        std::path::PathBuf::from(model_dir_str)
    };
    model_base.join(&config.model)
}

#[cfg(feature = "ai")]
fn resolve_labels(config: &crate::config::AiConfig) -> Result<Vec<String>, String> {
    if let Some(ref labels_path) = config.labels {
        crate::ai::load_labels_from_file(std::path::Path::new(labels_path))
            .map_err(|e| format!("Failed to load labels: {e}"))
    } else {
        Ok(crate::ai::DEFAULT_LABELS.iter().map(|s| s.to_string()).collect())
    }
}

#[cfg(feature = "ai")]
fn suggest_tags_inner(
    state: &AppState,
    asset_id: &str,
) -> Result<Vec<SuggestTagsResponse>, String> {
    use crate::ai;
    use crate::device_registry::DeviceRegistry;

    let engine = state.query_engine();
    let details = engine.show(asset_id).map_err(|e| format!("{e:#}"))?;

    // Find image to process
    let preview_gen = state.preview_generator();
    let registry = DeviceRegistry::new(&state.catalog_root);
    let volumes = registry.list().map_err(|e| format!("{e:#}"))?;
    let online_volumes: std::collections::HashMap<String, &crate::models::Volume> = volumes
        .iter()
        .filter(|v| v.is_online)
        .map(|v| (v.id.to_string(), v))
        .collect();

    let service = state.asset_service();
    let image_path = service
        .find_image_for_ai(&details, &preview_gen, &online_volumes)
        .ok_or_else(|| "No processable image found for this asset".to_string())?;

    let ext = image_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    if !ai::is_supported_image(ext) {
        return Err(format!("Unsupported image format: {ext}"));
    }

    // Lazy-load model
    let model_dir = resolve_model_dir(&state.ai_config);
    let model_id = &state.ai_config.model;
    let model_guard = state.ai_model.blocking_lock();
    let mut model_opt = model_guard;
    if model_opt.is_none() {
        let m = ai::SigLipModel::load(&model_dir, model_id)
            .map_err(|e| format!("Failed to load AI model: {e:#}"))?;
        *model_opt = Some(m);
    }
    let model = model_opt.as_mut().unwrap();

    // Lazy-compute label embeddings
    let labels = resolve_labels(&state.ai_config)?;
    let label_cache_read = state.ai_label_cache.blocking_read();
    let cached = label_cache_read.is_some();
    drop(label_cache_read);

    let (label_list, label_embs) = if cached {
        let guard = state.ai_label_cache.blocking_read();
        let (l, e) = guard.as_ref().unwrap();
        (l.clone(), e.clone())
    } else {
        let prompt_template = &state.ai_config.prompt;
        let prompted: Vec<String> = labels
            .iter()
            .map(|l| ai::apply_prompt_template(prompt_template, l))
            .collect();
        let embs = model
            .encode_texts(&prompted)
            .map_err(|e| format!("Failed to encode labels: {e:#}"))?;
        let mut guard = state.ai_label_cache.blocking_write();
        *guard = Some((labels.clone(), embs.clone()));
        (labels, embs)
    };

    // Encode image
    let image_emb = model
        .encode_image(&image_path)
        .map_err(|e| format!("Failed to encode image: {e:#}"))?;

    // Store embedding opportunistically (for "Find similar" feature)
    {
        let catalog = crate::catalog::Catalog::open_fast(&state.catalog_root);
        if let Ok(catalog) = catalog {
            let _ = crate::embedding_store::EmbeddingStore::initialize(catalog.conn());
            let emb_store = crate::embedding_store::EmbeddingStore::new(catalog.conn());
            let _ = emb_store.store(asset_id, &image_emb, model_id);
        }
        // Update in-memory index if loaded
        if let Ok(mut idx_guard) = state.ai_embedding_index.write() {
            if let Some(ref mut idx) = *idx_guard {
                idx.upsert(asset_id, &image_emb);
            }
        }
    }

    // Classify
    let threshold = state.ai_config.threshold;
    let suggestions = model.classify(&image_emb, &label_list, &label_embs, threshold);

    // Mark tags already on the asset
    let existing: std::collections::HashSet<String> = details
        .tags
        .iter()
        .map(|t| t.to_lowercase())
        .collect();

    let result: Vec<SuggestTagsResponse> = suggestions
        .into_iter()
        .map(|s| {
            let is_existing = existing.contains(&s.tag.to_lowercase());
            SuggestTagsResponse {
                tag: s.tag,
                confidence: s.confidence,
                existing: is_existing,
            }
        })
        .collect();

    Ok(result)
}

#[cfg(feature = "ai")]
#[derive(Debug, serde::Deserialize)]
pub struct BatchAutoTagRequest {
    pub asset_ids: Vec<String>,
}

#[cfg(feature = "ai")]
#[derive(Debug, serde::Serialize)]
pub struct BatchAutoTagResponse {
    pub succeeded: u32,
    pub failed: u32,
    pub tags_applied: u32,
    pub errors: Vec<String>,
}

/// POST /api/batch/auto-tag — auto-tag selected assets.
#[cfg(feature = "ai")]
pub async fn batch_auto_tag(
    State(state): State<Arc<AppState>>,
    Json(req): Json<BatchAutoTagRequest>,
) -> Response {
    let log = state.log_requests;
    let count = req.asset_ids.len();
    let state2 = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        batch_auto_tag_inner(&state2, req.asset_ids)
    })
    .await;

    match result {
        Ok(Ok(resp)) => {
            if log {
                eprintln!(
                    "batch_auto_tag: {} assets ({} ok, {} err, {} tags)",
                    count, resp.succeeded, resp.failed, resp.tags_applied
                );
            }
            if resp.succeeded > 0 {
                state.dropdown_cache.invalidate_tags();
            }
            Json(resp).into_response()
        }
        Ok(Err(msg)) => (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
    }
}

#[cfg(feature = "ai")]
fn batch_auto_tag_inner(
    state: &AppState,
    asset_ids: Vec<String>,
) -> Result<BatchAutoTagResponse, String> {
    use crate::ai;
    use crate::device_registry::DeviceRegistry;

    let engine = state.query_engine();
    let preview_gen = state.preview_generator();
    let registry = DeviceRegistry::new(&state.catalog_root);
    let volumes = registry.list().map_err(|e| format!("{e:#}"))?;
    let online_volumes: std::collections::HashMap<String, &crate::models::Volume> = volumes
        .iter()
        .filter(|v| v.is_online)
        .map(|v| (v.id.to_string(), v))
        .collect();

    // Lazy-load model
    let model_dir = resolve_model_dir(&state.ai_config);
    let model_id = &state.ai_config.model;
    let mut model_guard = state.ai_model.blocking_lock();
    if model_guard.is_none() {
        let m = ai::SigLipModel::load(&model_dir, model_id)
            .map_err(|e| format!("Failed to load AI model: {e:#}"))?;
        *model_guard = Some(m);
    }
    let model = model_guard.as_mut().unwrap();

    // Lazy-compute label embeddings
    let labels = resolve_labels(&state.ai_config)?;
    let label_cache_read = state.ai_label_cache.blocking_read();
    let cached = label_cache_read.is_some();
    drop(label_cache_read);

    let (label_list, label_embs) = if cached {
        let guard = state.ai_label_cache.blocking_read();
        let (l, e) = guard.as_ref().unwrap();
        (l.clone(), e.clone())
    } else {
        let prompt_template = &state.ai_config.prompt;
        let prompted: Vec<String> = labels
            .iter()
            .map(|l| ai::apply_prompt_template(prompt_template, l))
            .collect();
        let embs = model
            .encode_texts(&prompted)
            .map_err(|e| format!("Failed to encode labels: {e:#}"))?;
        let mut guard = state.ai_label_cache.blocking_write();
        *guard = Some((labels.clone(), embs.clone()));
        (labels, embs)
    };

    let threshold = state.ai_config.threshold;
    let service = state.asset_service();
    let mut resp = BatchAutoTagResponse {
        succeeded: 0,
        failed: 0,
        tags_applied: 0,
        errors: Vec::new(),
    };

    for aid in &asset_ids {
        let details = match engine.show(aid) {
            Ok(d) => d,
            Err(e) => {
                resp.failed += 1;
                resp.errors.push(format!("{}: {e:#}", &aid[..8.min(aid.len())]));
                continue;
            }
        };

        let image_path = match service.find_image_for_ai(&details, &preview_gen, &online_volumes) {
            Some(p) => p,
            None => {
                // Skip assets with no processable image (not an error)
                continue;
            }
        };

        let ext = image_path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");
        if !ai::is_supported_image(ext) {
            continue;
        }

        let image_emb = match model.encode_image(&image_path) {
            Ok(emb) => emb,
            Err(e) => {
                resp.failed += 1;
                resp.errors.push(format!("{}: {e:#}", &aid[..8.min(aid.len())]));
                continue;
            }
        };

        // Store embedding opportunistically (for "Find similar" feature)
        {
            let cat = crate::catalog::Catalog::open_fast(&state.catalog_root);
            if let Ok(cat) = cat {
                let _ = crate::embedding_store::EmbeddingStore::initialize(cat.conn());
                let es = crate::embedding_store::EmbeddingStore::new(cat.conn());
                let _ = es.store(aid, &image_emb, model_id);
            }
            // Update in-memory index if loaded
            if let Ok(mut idx_guard) = state.ai_embedding_index.write() {
                if let Some(ref mut idx) = *idx_guard {
                    idx.upsert(aid, &image_emb);
                }
            }
        }

        let suggestions = model.classify(&image_emb, &label_list, &label_embs, threshold);

        // Filter out existing tags
        let existing: std::collections::HashSet<String> = details
            .tags
            .iter()
            .map(|t| t.to_lowercase())
            .collect();

        let new_tags: Vec<String> = suggestions
            .into_iter()
            .filter(|s| !existing.contains(&s.tag.to_lowercase()))
            .map(|s| s.tag)
            .collect();

        if new_tags.is_empty() {
            resp.succeeded += 1;
            continue;
        }

        // Apply tags
        match engine.tag(aid, &new_tags, false) {
            Ok(_) => {
                resp.tags_applied += new_tags.len() as u32;
                resp.succeeded += 1;
            }
            Err(e) => {
                resp.failed += 1;
                resp.errors.push(format!("{}: {e:#}", &aid[..8.min(aid.len())]));
            }
        }
    }

    Ok(resp)
}

// --- Visual similarity search endpoint (feature-gated) ---

#[cfg(feature = "ai")]
#[derive(Debug, serde::Serialize)]
pub struct SimilarAssetResponse {
    pub asset_id: String,
    pub similarity: f32,
    pub preview_url: Option<String>,
    pub name: String,
}

/// POST /api/asset/{id}/similar — find visually similar assets.
#[cfg(feature = "ai")]
pub async fn find_similar(
    State(state): State<Arc<AppState>>,
    Path(asset_id): Path<String>,
) -> Response {
    let state = state.clone();
    let result: Result<Result<Vec<SimilarAssetResponse>, String>, _> =
        tokio::task::spawn_blocking(move || find_similar_inner(&state, &asset_id))
            .await;

    match result {
        Ok(Ok(results)) => Json(results).into_response(),
        Ok(Err(msg)) => (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}")).into_response(),
    }
}

#[cfg(feature = "ai")]
fn find_similar_inner(
    state: &AppState,
    asset_id: &str,
) -> Result<Vec<SimilarAssetResponse>, String> {
    use crate::ai;
    use crate::device_registry::DeviceRegistry;
    use crate::embedding_store::EmbeddingStore;

    let model_id = &state.ai_config.model;
    let spec = crate::ai::get_model_spec(model_id)
        .ok_or_else(|| format!("Unknown model: {model_id}"))?;

    // Check if embedding already exists (without loading the AI model)
    let catalog = state.catalog().map_err(|e| format!("{e:#}"))?;
    let _ = EmbeddingStore::initialize(catalog.conn());
    let emb_store = EmbeddingStore::new(catalog.conn());

    let stored_emb = emb_store.get(asset_id, model_id).map_err(|e| format!("{e:#}"))?;

    let query_emb = if let Some(emb) = stored_emb {
        emb
    } else {
        // Need to encode — load model, find image
        let engine = state.query_engine();
        let details = engine.show(asset_id).map_err(|e| format!("{e:#}"))?;

        let preview_gen = state.preview_generator();
        let registry = DeviceRegistry::new(&state.catalog_root);
        let volumes = registry.list().map_err(|e| format!("{e:#}"))?;
        let online_volumes: std::collections::HashMap<String, &crate::models::Volume> = volumes
            .iter()
            .filter(|v| v.is_online)
            .map(|v| (v.id.to_string(), v))
            .collect();

        let service = state.asset_service();
        let image_path = service
            .find_image_for_ai(&details, &preview_gen, &online_volumes)
            .ok_or_else(|| "No processable image found for this asset".to_string())?;

        let ext = image_path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");
        if !ai::is_supported_image(ext) {
            return Err(format!("Unsupported image format: {ext}"));
        }

        let model_dir = resolve_model_dir(&state.ai_config);
        let mut model_guard = state.ai_model.blocking_lock();
        if model_guard.is_none() {
            let m = ai::SigLipModel::load(&model_dir, model_id)
                .map_err(|e| format!("Failed to load AI model: {e:#}"))?;
            *model_guard = Some(m);
        }
        let model = model_guard.as_mut().unwrap();

        let emb = model
            .encode_image(&image_path)
            .map_err(|e| format!("Failed to encode image: {e:#}"))?;

        drop(model_guard); // Release model lock ASAP

        emb_store
            .store(asset_id, &emb, model_id)
            .map_err(|e| format!("Failed to store embedding: {e:#}"))?;
        emb
    };

    // Lazy-load in-memory embedding index
    {
        let needs_load = state.ai_embedding_index.read().unwrap().is_none();
        if needs_load {
            let index = crate::embedding_store::EmbeddingIndex::load(
                catalog.conn(),
                model_id,
                spec.embedding_dim,
            ).map_err(|e| format!("Failed to load embedding index: {e:#}"))?;
            *state.ai_embedding_index.write().unwrap() = Some(index);
        }
    }

    // Update index with the query embedding (in case it was just generated)
    {
        let mut idx_guard = state.ai_embedding_index.write().unwrap();
        if let Some(ref mut idx) = *idx_guard {
            idx.upsert(asset_id, &query_emb);
        }
    }

    // Search in-memory index
    let results = {
        let idx_guard = state.ai_embedding_index.read().unwrap();
        let idx = idx_guard.as_ref().unwrap();
        idx.search(&query_emb, 20, Some(asset_id))
    };

    // Build response with preview URLs and names
    let preview_gen = state.preview_generator();
    let preview_ext = &state.preview_ext;
    let response: Vec<SimilarAssetResponse> = results
        .into_iter()
        .filter_map(|(id, similarity)| {
            let cat = state.catalog().ok()?;
            let d = cat.load_asset_details(&id).ok()??;
            let name = d
                .name
                .clone()
                .unwrap_or_else(|| {
                    d.variants
                        .first()
                        .and_then(|v| {
                            v.locations
                                .first()
                                .map(|fl| {
                                    std::path::Path::new(&fl.relative_path)
                                        .file_name()
                                        .unwrap_or_default()
                                        .to_string_lossy()
                                        .to_string()
                                })
                        })
                        .unwrap_or_else(|| id[..8.min(id.len())].to_string())
                });

            let preview_url = {
                let best_idx = crate::models::variant::best_preview_index_details(&d.variants);
                best_idx.and_then(|i| {
                    let v = &d.variants[i];
                    if preview_gen.has_preview(&v.content_hash) {
                        Some(super::templates::preview_url(&v.content_hash, preview_ext))
                    } else {
                        None
                    }
                })
            };

            Some(SimilarAssetResponse {
                asset_id: id,
                similarity,
                preview_url,
                name,
            })
        })
        .collect();

    Ok(response)
}

// --- Face recognition handlers (feature-gated) ---

/// GET /api/asset/{id}/faces — list faces for an asset.
#[cfg(feature = "ai")]
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
#[cfg(feature = "ai")]
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

/// POST /api/batch/detect-faces — batch detect faces for selected assets.
#[cfg(feature = "ai")]
pub async fn batch_detect_faces(
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    let asset_ids: Vec<String> = body.get("asset_ids")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    let state2 = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        detect_faces_inner(&state2, &asset_ids)
    }).await;

    match result {
        Ok(Ok(json)) => Json(json).into_response(),
        Ok(Err(msg)) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": msg}))).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": format!("{e}")}))).into_response(),
    }
}

#[cfg(feature = "ai")]
fn detect_faces_inner(state: &AppState, asset_ids: &[String]) -> Result<serde_json::Value, String> {
    let face_model_dir = crate::face::resolve_face_model_dir(&state.ai_config);
    if !crate::face::FaceDetector::models_exist(&face_model_dir) {
        return Err("Face models not downloaded. Run 'dam faces download' first.".to_string());
    }

    let mut detector = crate::face::FaceDetector::load(&face_model_dir, false)
        .map_err(|e| format!("Failed to load face detector: {e:#}"))?;

    let min_confidence = state.ai_config.face_min_confidence;
    let catalog = state.catalog().map_err(|e| format!("{e:#}"))?;
    let _ = crate::face_store::FaceStore::initialize(catalog.conn());
    let face_store = crate::face_store::FaceStore::new(catalog.conn());
    let engine = state.query_engine();
    let preview_gen = state.preview_generator();
    let registry = crate::device_registry::DeviceRegistry::new(&state.catalog_root);
    let volumes = registry.list().map_err(|e| format!("{e:#}"))?;
    let online_volumes: std::collections::HashMap<String, &crate::models::Volume> = volumes
        .iter()
        .filter(|v| v.is_online)
        .map(|v| (v.id.to_string(), v))
        .collect();
    let service = state.asset_service();

    let mut total_faces = 0u32;
    let mut total_assets = 0u32;
    let mut errors: Vec<String> = Vec::new();

    for aid in asset_ids {
        let details = match engine.show(aid) {
            Ok(d) => d,
            Err(e) => { errors.push(format!("{}: {e:#}", &aid[..8.min(aid.len())])); continue; }
        };

        let image_path = match service.find_image_for_ai(&details, &preview_gen, &online_volumes) {
            Some(p) => p,
            None => continue,
        };

        match detector.detect_and_embed(&image_path, min_confidence) {
            Ok(face_results) => {
                // Clear existing faces for this asset
                let _ = face_store.delete_faces_for_asset(aid);
                for (face, embedding) in &face_results {
                    let face_id = uuid::Uuid::new_v4().to_string();
                    if let Err(e) = face_store.store_face(
                        &face_id, aid, face.bbox_x, face.bbox_y, face.bbox_w, face.bbox_h,
                        embedding, face.confidence,
                    ) {
                        errors.push(format!("{}: store error: {e:#}", &aid[..8.min(aid.len())]));
                    } else {
                        let _ = crate::face::save_face_crop(&image_path, face, &face_id, &state.catalog_root);
                    }
                }
                let _ = catalog.update_face_count(aid);
                total_faces += face_results.len() as u32;
                total_assets += 1;
            }
            Err(e) => {
                errors.push(format!("{}: {e:#}", &aid[..8.min(aid.len())]));
            }
        }
    }

    Ok(serde_json::json!({
        "succeeded": total_assets,
        "faces_detected": total_faces,
        "errors": errors,
    }))
}

/// PUT /api/faces/{face_id}/assign — assign a face to a person.
#[cfg(feature = "ai")]
pub async fn assign_face(
    State(state): State<Arc<AppState>>,
    Path(face_id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    let person_id: String = match body.get("person_id").and_then(|v| v.as_str()) {
        Some(pid) => pid.to_string(),
        None => return (StatusCode::BAD_REQUEST, "Missing person_id").into_response(),
    };

    let result = tokio::task::spawn_blocking(move || {
        let catalog = state.catalog()?;
        let face_store = crate::face_store::FaceStore::new(catalog.conn());
        face_store.assign_face_to_person(&face_id, &person_id)?;
        state.dropdown_cache.invalidate_people();
        Ok::<_, anyhow::Error>(())
    }).await;

    match result {
        Ok(Ok(())) => Json(serde_json::json!({"ok": true})).into_response(),
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e}")).into_response(),
    }
}

/// DELETE /api/faces/{face_id}/unassign — unassign a face from its person.
#[cfg(feature = "ai")]
pub async fn unassign_face_api(
    State(state): State<Arc<AppState>>,
    Path(face_id): Path<String>,
) -> Response {
    let result = tokio::task::spawn_blocking(move || {
        let catalog = state.catalog()?;
        let face_store = crate::face_store::FaceStore::new(catalog.conn());
        face_store.unassign_face(&face_id)?;
        state.dropdown_cache.invalidate_people();
        Ok::<_, anyhow::Error>(())
    }).await;

    match result {
        Ok(Ok(())) => Json(serde_json::json!({"ok": true})).into_response(),
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e}")).into_response(),
    }
}

/// DELETE /api/faces/{face_id} — delete a face detection (e.g., false positive).
#[cfg(feature = "ai")]
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
            // Remove crop thumbnail
            let prefix = &face_id[..2.min(face_id.len())];
            let crop = catalog_root.join("faces").join(prefix).join(format!("{face_id}.jpg"));
            let _ = std::fs::remove_file(crop);
        }
        Ok::<_, anyhow::Error>(())
    }).await;

    match result {
        Ok(Ok(())) => Json(serde_json::json!({"ok": true})).into_response(),
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": format!("{e:#}")}))).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": format!("{e}")}))).into_response(),
    }
}

/// GET /people — people gallery page.
#[cfg(feature = "ai")]
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
#[cfg(feature = "ai")]
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

/// POST /api/people — create a new person. Body: `{"name": "..."}`.
#[cfg(feature = "ai")]
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
        Ok::<_, anyhow::Error>(serde_json::json!({"id": id, "name": name}))
    }).await;

    match result {
        Ok(Ok(json)) => Json(json).into_response(),
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": format!("{e:#}")}))).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": format!("{e}")}))).into_response(),
    }
}

/// PUT /api/people/{id}/name — rename a person.
#[cfg(feature = "ai")]
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
        state.dropdown_cache.invalidate_people();
        Ok::<_, anyhow::Error>(())
    }).await;

    match result {
        Ok(Ok(())) => Json(serde_json::json!({"ok": true})).into_response(),
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e}")).into_response(),
    }
}

/// POST /api/people/{id}/merge — merge source into target.
#[cfg(feature = "ai")]
pub async fn merge_person_api(
    State(state): State<Arc<AppState>>,
    Path(target_id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    let source_id: String = match body.get("source_id").and_then(|v| v.as_str()) {
        Some(s) => s.to_string(),
        None => return (StatusCode::BAD_REQUEST, "Missing source_id").into_response(),
    };

    let result = tokio::task::spawn_blocking(move || {
        let catalog = state.catalog()?;
        let face_store = crate::face_store::FaceStore::new(catalog.conn());
        let moved = face_store.merge_people(&target_id, &source_id)?;
        state.dropdown_cache.invalidate_people();
        Ok::<_, anyhow::Error>(moved)
    }).await;

    match result {
        Ok(Ok(moved)) => Json(serde_json::json!({"ok": true, "faces_moved": moved})).into_response(),
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e}")).into_response(),
    }
}

/// DELETE /api/people/{id} — delete a person.
#[cfg(feature = "ai")]
pub async fn delete_person_api(
    State(state): State<Arc<AppState>>,
    Path(person_id): Path<String>,
) -> Response {
    let result = tokio::task::spawn_blocking(move || {
        let catalog = state.catalog()?;
        let face_store = crate::face_store::FaceStore::new(catalog.conn());
        face_store.delete_person(&person_id)?;
        state.dropdown_cache.invalidate_people();
        Ok::<_, anyhow::Error>(())
    }).await;

    match result {
        Ok(Ok(())) => Json(serde_json::json!({"ok": true})).into_response(),
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e}")).into_response(),
    }
}

/// POST /api/faces/cluster — run auto-clustering.
#[cfg(feature = "ai")]
pub async fn cluster_faces_api(
    State(state): State<Arc<AppState>>,
) -> Response {
    let result = tokio::task::spawn_blocking(move || {
        let catalog = state.catalog()?;
        let _ = crate::face_store::FaceStore::initialize(catalog.conn());
        let face_store = crate::face_store::FaceStore::new(catalog.conn());
        let threshold = state.ai_config.face_cluster_threshold;
        let result = face_store.auto_cluster(threshold, None)?;
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
