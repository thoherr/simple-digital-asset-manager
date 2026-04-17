//! HTTP route handlers for the `maki serve` web UI.
//!
//! Each submodule handles a resource domain (browse, assets, tags, stacks,
//! collections, saved_search, calendar_map, duplicates, import, media, stats,
//! volumes, ai). This `mod.rs` only holds shared helpers used across submodules.

use crate::device_registry::DeviceRegistry;
use crate::query::{normalize_path_for_search, parse_search_query, ParsedSearch};

use super::AppState;

#[cfg(feature = "ai")]
mod ai;
#[cfg(feature = "ai")]
pub use ai::*;
mod assets;
pub use assets::*;
mod browse;
pub use browse::*;
mod calendar_map;
pub use calendar_map::*;
mod collections;
pub use collections::*;
mod duplicates;
pub use duplicates::*;
mod import;
pub use import::*;
mod media;
pub use media::*;
mod saved_search;
pub use saved_search::*;
mod stacks;
pub use stacks::*;
mod stats;
pub use stats::*;
mod tags;
pub use tags::*;
mod volumes;
pub use volumes::*;

/// Resolve the best variant index for an asset, respecting user override.
/// Looks up the stored best_variant_hash, falls back to algorithmic scoring.
pub(super) fn resolve_best_variant_idx(
    catalog: &crate::catalog::Catalog,
    asset_id: &str,
    variants: &[crate::catalog::VariantDetails],
) -> anyhow::Result<usize> {
    let stored_hash = catalog.get_asset_best_variant_hash(asset_id).unwrap_or(None);
    stored_hash.as_ref()
        .and_then(|h| variants.iter().position(|v| &v.content_hash == h))
        .or_else(|| crate::models::variant::best_preview_index_details(variants))
        .ok_or_else(|| anyhow::anyhow!("asset has no variants"))
}

/// Resolve `similar:` filter: look up embedding, search index, return matching IDs with scores.
/// Returns (ordered_ids, score_map). The source asset is included with similarity 100%.
/// Empty if no `similar:` filter is active.
#[cfg(feature = "ai")]
fn resolve_similar_filter(
    catalog: &crate::catalog::Catalog,
    state: &AppState,
    parsed: &crate::query::ParsedSearch,
) -> anyhow::Result<(Vec<String>, std::collections::HashMap<String, f32>)> {
    use std::collections::HashMap;
    if let Some(ref similar_ref) = parsed.similar {
        let full_id = catalog
            .resolve_asset_id(similar_ref)?
            .ok_or_else(|| anyhow::anyhow!("no asset found matching '{similar_ref}'"))?;
        let model_id = &state.ai_config.model;
        let spec = crate::ai::get_model_spec(model_id);
        if let Some(spec) = spec {
            let emb_store = crate::embedding_store::EmbeddingStore::new(catalog.conn());
            let query_emb = emb_store
                .get(&full_id, model_id)?
                .ok_or_else(|| anyhow::anyhow!(
                    "No embedding for '{similar_ref}'. Run `maki embed --asset {full_id}` first."
                ))?;
            // limit defaults to 40 results (including the source asset)
            let limit = parsed.similar_limit.unwrap_or(40);
            // min_sim is specified as percentage 0-100, convert to 0.0-1.0
            let min_sim = parsed.min_sim.unwrap_or(0.0) / 100.0;
            // Ensure embedding index is loaded
            let needs_load = state.ai_embedding_index.read().unwrap().is_none();
            if needs_load {
                if let Ok(index) = crate::embedding_store::EmbeddingIndex::load(
                    catalog.conn(), model_id, spec.embedding_dim,
                ) {
                    *state.ai_embedding_index.write().unwrap() = Some(index);
                }
            }
            // Search excludes the source — we add it back with score 1.0
            let results = {
                let idx_guard = state.ai_embedding_index.read().unwrap();
                if let Some(ref idx) = *idx_guard {
                    idx.search(&query_emb, limit.saturating_sub(1), Some(&full_id))
                } else {
                    Vec::new()
                }
            };
            let mut filtered: Vec<(String, f32)> = Vec::with_capacity(results.len() + 1);
            // Include the source asset itself at 100%
            filtered.push((full_id.clone(), 1.0));
            for (id, sim) in results {
                if sim >= min_sim {
                    filtered.push((id, sim));
                }
            }
            let scores: HashMap<String, f32> = filtered.iter().cloned().collect();
            let ids: Vec<String> = filtered.into_iter().map(|(id, _)| id).collect();
            return Ok((ids, scores));
        }
    }
    Ok((Vec::new(), std::collections::HashMap::new()))
}

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
    /// Set to "1" to disable the default filter from maki.toml [browse].
    pub nodefault: Option<String>,
}








/// Shared result from `build_parsed_search` — holds the parsed query and
/// extracted filter state that every browse/search/calendar/map handler needs.
pub(super) struct BrowseFilters {
    pub(super) parsed: ParsedSearch,
    // Raw param values for template rendering (display current filter state)
    pub(super) query: String,
    pub(super) asset_type: String,
    pub(super) tag: String,
    pub(super) format_filter: String,
    pub(super) volume: String,
    pub(super) rating: String,
    pub(super) label: String,
    pub(super) collection: String,
    pub(super) path: String,
    pub(super) person: String,
    pub(super) path_volume_id: Option<String>,
    pub(super) sort_str: String,
    pub(super) page: u32,
    pub(super) collapse_stacks: bool,
    pub(super) nodefault: bool,
}

/// Extract and merge all browse filter parameters from URL query params.
/// This is the single source of truth for how SearchParams → ParsedSearch
/// works across browse_page, search_api, page_ids_api, calendar_api, map_api,
/// and facets_api. Each handler calls this, then adds handler-specific logic
/// (template rendering, JSON formatting, etc.).
/// Resolve a list of comma-OR'd / entry-ANDed name groups against a
/// lookup function and return the asset IDs that match all entries.
///
/// Matches the catalog's tag semantics: comma within an entry is OR
/// ("any of these names"), separate entries are AND ("must match all
/// of these"). For person filters this means `person:Alice person:Bob`
/// returns assets that contain BOTH Alice and Bob, while
/// `person:Alice,Bob` returns assets that contain EITHER.
///
/// Returns an empty Vec when `entries` is empty (caller should not call
/// this when there's no filter to apply).
#[cfg(feature = "ai")]
pub(super) fn intersect_name_groups<F>(entries: &[String], lookup: F) -> Vec<String>
where
    F: Fn(&str) -> Vec<String>,
{
    let mut current: Option<std::collections::HashSet<String>> = None;
    for entry in entries {
        let mut group: std::collections::HashSet<String> = std::collections::HashSet::new();
        for name in entry.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()) {
            for id in lookup(name) {
                group.insert(id);
            }
        }
        current = match current {
            None => Some(group),
            Some(prev) => Some(prev.intersection(&group).cloned().collect()),
        };
    }
    current.unwrap_or_default().into_iter().collect()
}

/// Resolve a list of comma-OR'd collection name entries to asset IDs.
///
/// Each entry may be a comma-separated list (OR within entry). Multiple calls
/// are union'd (OR across entries) — collections don't AND like tags/persons.
/// Returns a Vec of distinct asset IDs. Returns empty Vec on no matches.
pub(super) fn resolve_collection_ids(entries: &[String], conn: &rusqlite::Connection) -> Vec<String> {
    let col_store = crate::collection::CollectionStore::new(conn);
    let mut all_ids = std::collections::HashSet::new();
    for col_entry in entries {
        for col_name in col_entry.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()) {
            if let Ok(ids) = col_store.asset_ids_for_collection(col_name) {
                all_ids.extend(ids);
            }
        }
    }
    all_ids.into_iter().collect()
}

pub(super) fn build_parsed_search(params: &SearchParams, state: &AppState) -> BrowseFilters {
    let query = params.q.as_deref().unwrap_or("");
    let asset_type = params.asset_type.as_deref().unwrap_or("");
    let tag = params.tag.as_deref().unwrap_or("");
    let fmt = params.format.as_deref().unwrap_or("");
    let volume = params.volume.as_deref().unwrap_or("").to_string();
    let rating_str = params.rating.as_deref().unwrap_or("");
    let label_str = params.label.as_deref().unwrap_or("");
    let sort_str = params.sort.as_deref().unwrap_or("date_desc").to_string();
    let page = params.page.unwrap_or(1).max(1);
    let collection_str = params.collection.as_deref().unwrap_or("");
    let path_str = params.path.as_deref().unwrap_or("");
    let person_str = params.person.as_deref().unwrap_or("");
    let collapse_stacks = params.stacks.as_deref().unwrap_or("1") == "1";
    let nodefault = params.nodefault.as_deref() == Some("1");

    // Parse query + overlay explicit dropdown params
    let mut parsed = parse_search_query(query);
    if !asset_type.is_empty() { parsed.asset_types.push(asset_type.to_string()); }
    // Tags from the URL param: comma is the chip-list separator (= AND across
    // entries at the catalog level). Note: this overrides the historical
    // "comma = OR" behaviour for the dedicated `tag=` URL param. Power users
    // who want OR can still type `tag:a,b` in the q field — that goes through
    // parse_search_query, which preserves the comma as one entry → catalog OR.
    if !tag.is_empty() {
        for t in tag.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()) {
            parsed.tags.push(t.to_string());
        }
    }
    if !fmt.is_empty() { parsed.formats.push(fmt.to_string()); }
    if !rating_str.is_empty() { parsed.rating = crate::query::parse_numeric_filter(rating_str); }
    if label_str == "none" {
        parsed.color_label_none = true;
    } else if !label_str.is_empty() {
        parsed.color_labels.push(label_str.to_string());
    }

    // Apply default filter from config
    apply_default_filter(&mut parsed, &state.default_filter, nodefault);

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

    // Push collection/person from dropdowns. The `person` URL param accepts
    // a comma-separated list (sent by the chip-based people picker); legacy
    // single-value URLs from shared links still work since they have no comma.
    if !collection_str.is_empty() { parsed.collections.push(collection_str.to_string()); }
    if !person_str.is_empty() {
        for p in person_str.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()) {
            parsed.persons.push(p.to_string());
        }
    }

    BrowseFilters {
        parsed,
        query: query.to_string(),
        asset_type: asset_type.to_string(),
        tag: tag.to_string(),
        format_filter: fmt.to_string(),
        volume,
        rating: rating_str.to_string(),
        label: label_str.to_string(),
        collection: collection_str.to_string(),
        path: path_str.to_string(),
        person: person_str.to_string(),
        path_volume_id,
        sort_str,
        page,
        collapse_stacks,
        nodefault,
    }
}

/// Merge explicit dropdown params into a ParsedSearch.
/// Used by handlers not yet migrated to build_parsed_search.
pub(super) fn merge_search_params(
    query: &str,
    asset_type: &str,
    tag: &str,
    format: &str,
    rating_str: &str,
    label: &str,
) -> ParsedSearch {
    let mut parsed = parse_search_query(query);
    if !asset_type.is_empty() { parsed.asset_types.push(asset_type.to_string()); }
    if !tag.is_empty() {
        for t in tag.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()) {
            parsed.tags.push(t.to_string());
        }
    }
    if !format.is_empty() { parsed.formats.push(format.to_string()); }
    if !rating_str.is_empty() { parsed.rating = crate::query::parse_numeric_filter(rating_str); }
    if label == "none" {
        parsed.color_label_none = true;
    } else if !label.is_empty() {
        parsed.color_labels.push(label.to_string());
    }
    parsed
}

/// Apply the default filter from config to a parsed search, unless disabled.
fn apply_default_filter(parsed: &mut ParsedSearch, default_filter: &Option<String>, nodefault: bool) {
    if nodefault {
        return;
    }
    if let Some(df) = default_filter {
        if !df.is_empty() {
            let default_parsed = parse_search_query(df);
            parsed.merge_from(&default_parsed);
        }
    }
}


// --- Calendar heatmap & map --- (moved to routes::calendar_map)


// --- Facets ---

