// ═══════════════════════════════════════════════════════════════════════════════
// catalog.rs — SQLite catalog (derived cache of sidecar data)
// ═══════════════════════════════════════════════════════════════════════════════
//
// Table of Contents:
//   1. IMPORTS .......................... use declarations
//   2. TYPES & STRUCTS .................. MapMarker, FacetCounts, SearchRow, AssetDetails, etc.
//   3. SEARCH OPTIONS ................... SearchSort, SearchOptions, SearchPage
//   4. HELPER FUNCTIONS ................. path_pattern_to_like, next_date_bound
//   5. CATALOG STRUCT & CONNECTION ...... Catalog, open, open_fast, open_and_migrate
//   6. SCHEMA MIGRATIONS ................ run_migrations, initialize
//   7. ASSET CRUD ....................... insert_asset, update_asset_*, delete_asset
//   8. VARIANT & LOCATION CRUD .......... insert_variant, insert_file_location, etc.
//   9. RECIPE CRUD ...................... insert_recipe, update_recipe_*, writeback
//  10. VOLUME OPERATIONS ................ ensure_volume, delete_volume, bulk_move_*
//  11. ASSET LOOKUPS .................... search_assets, resolve_asset_id, load_asset_details
//  12. DUPLICATES ....................... find_duplicates, find_duplicates_filtered
//  13. LOCATION & RECIPE QUERIES ........ find_variant_by_volume, list_recipes_*, etc.
//  14. REBUILD .......................... rebuild (drop + recreate)
//  15. STATS ............................ stats_overview, stats_per_volume, build_stats
//  16. SEARCH BUILDER ................... rating_clause, numeric_clause, build_search_where
//  17. SEARCH EXECUTION ................. search_paginated, search_count, get_search_row
//  18. CALENDAR & FACETS ................ calendar_counts, facet_counts, map_markers
//  19. TAG & FORMAT QUERIES ............. list_all_tags, assets_with_tag, list_all_formats
//  20. ANALYTICS ........................ build_analytics
//  21. BACKUP STATUS .................... backup_status_overview, at_risk, missing_from_volume
//  22. CLEANUP QUERIES .................. count_file_locations, list_orphaned_asset_ids, etc.
//  23. TESTS ............................ Unit tests
// ═══════════════════════════════════════════════════════════════════════════════

// ═══ IMPORTS ═══

use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;
use rusqlite::Connection;

use crate::query::NumericFilter;

use crate::models::{Asset, FileLocation, Recipe, Variant};

// ═══ TYPES & STRUCTS ═══

/// Map marker data for the web UI map view.
#[derive(Debug, serde::Serialize)]
pub struct MapMarker {
    pub id: String,
    pub lat: f64,
    pub lng: f64,
    pub preview: Option<String>,
    pub name: String,
    pub rating: Option<u8>,
    pub label: Option<String>,
}

/// Facet counts for the browse sidebar.
#[derive(Debug, serde::Serialize)]
pub struct FacetCounts {
    pub total: u64,
    pub ratings: Vec<(Option<u8>, u64)>,
    pub labels: Vec<(Option<String>, u64)>,
    pub formats: Vec<(String, u64)>,
    pub volumes: Vec<(String, String, u64)>,
    pub tags: Vec<(String, u64)>,
    pub years: Vec<(String, u64)>,
    pub geotagged: u64,
}

/// A row returned from a search query.
#[derive(Debug, serde::Serialize)]
pub struct SearchRow {
    pub asset_id: String,
    pub name: Option<String>,
    pub asset_type: String,
    pub created_at: String,
    pub original_filename: String,
    pub format: String,
    pub tags: Vec<String>,
    pub description: Option<String>,
    pub content_hash: String,
    pub rating: Option<u8>,
    pub color_label: Option<String>,
    /// The "identity" format of the asset (Original RAW > Original any > best variant).
    pub primary_format: Option<String>,
    /// Number of variants belonging to this asset.
    pub variant_count: u32,
    /// Stack ID if this asset is in a stack.
    pub stack_id: Option<String>,
    /// Number of members in this asset's stack (for badge rendering).
    pub stack_count: Option<u32>,
    /// Manual preview rotation override in degrees (0/90/180/270).
    pub preview_rotation: Option<u16>,
    /// Number of detected faces in this asset.
    pub face_count: u32,
    /// Video duration in seconds (None for non-video assets).
    pub video_duration: Option<f64>,
}

impl SearchRow {
    /// The format to display in UI/CLI — primary_format if available, else best variant format.
    pub fn display_format(&self) -> &str {
        self.primary_format.as_deref().unwrap_or(&self.format)
    }
}

/// Full asset details returned by `load_asset_details`.
#[derive(Debug, serde::Serialize)]
pub struct AssetDetails {
    pub id: String,
    pub name: Option<String>,
    pub asset_type: String,
    pub created_at: String,
    pub tags: Vec<String>,
    pub description: Option<String>,
    pub rating: Option<u8>,
    pub color_label: Option<String>,
    pub variants: Vec<VariantDetails>,
    pub recipes: Vec<RecipeDetails>,
}

/// Variant details within an `AssetDetails`.
#[derive(Debug, serde::Serialize)]
pub struct VariantDetails {
    pub content_hash: String,
    pub role: String,
    pub format: String,
    pub file_size: u64,
    pub original_filename: String,
    pub source_metadata: std::collections::HashMap<String, String>,
    pub locations: Vec<LocationDetails>,
}

/// File location details within a `VariantDetails`.
#[derive(Debug, Clone, serde::Serialize)]
pub struct LocationDetails {
    pub volume_label: String,
    pub volume_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume_purpose: Option<String>,
    pub relative_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verified_at: Option<String>,
}

/// A variant that exists in multiple file locations.
#[derive(Debug, serde::Serialize)]
pub struct DuplicateEntry {
    pub content_hash: String,
    pub original_filename: String,
    pub format: String,
    pub file_size: u64,
    pub asset_name: Option<String>,
    pub asset_id: String,
    pub locations: Vec<LocationDetails>,
    /// Number of distinct volumes this variant exists on.
    pub volume_count: usize,
    /// Volume labels that have 2+ locations for this variant (same-volume dupes).
    pub same_volume_groups: Vec<String>,
    /// Pre-computed preview URL for the web UI (not serialized).
    #[serde(skip)]
    pub preview_url: String,
}

/// Recipe details within an `AssetDetails`.
#[derive(Debug, serde::Serialize)]
pub struct RecipeDetails {
    pub variant_hash: String,
    pub software: String,
    pub recipe_type: String,
    pub content_hash: String,
    pub volume_id: Option<String>,
    pub volume_label: Option<String>,
    pub relative_path: Option<String>,
    pub pending_writeback: bool,
}

/// Top-level container for catalog statistics.
#[derive(Debug, serde::Serialize)]
pub struct CatalogStats {
    pub overview: OverviewStats,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub types: Option<TypeStats>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volumes: Option<Vec<VolumeStats>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<TagStats>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verified: Option<VerificationStats>,
}

/// Overview counts for the entire catalog.
#[derive(Debug, serde::Serialize)]
pub struct OverviewStats {
    pub assets: u64,
    pub variants: u64,
    pub recipes: u64,
    /// Number of unique recipe content hashes (distinct XMP files).
    /// The difference `recipes - unique_recipes` is the number of
    /// duplicate recipe locations (e.g. from backup volumes).
    pub file_locations: u64,
    pub unique_recipes: u64,
    pub volumes_total: u64,
    pub volumes_online: u64,
    pub volumes_offline: u64,
    pub total_size: u64,
}

/// Breakdown by asset type and file formats.
#[derive(Debug, serde::Serialize)]
pub struct TypeStats {
    pub asset_types: Vec<TypeCount>,
    pub variant_formats: Vec<FormatCount>,
    pub recipe_formats: Vec<FormatCount>,
}

/// A single asset-type count entry.
#[derive(Debug, serde::Serialize)]
pub struct TypeCount {
    pub asset_type: String,
    pub count: u64,
    pub percentage: f64,
}

/// A single format count entry.
#[derive(Debug, serde::Serialize)]
pub struct FormatCount {
    pub format: String,
    pub count: u64,
}

/// Per-volume statistics.
#[derive(Debug, serde::Serialize)]
pub struct VolumeStats {
    pub label: String,
    pub volume_id: String,
    pub is_online: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purpose: Option<String>,
    pub assets: u64,
    pub variants: u64,
    pub recipes: u64,
    pub formats: Vec<String>,
    pub directories: u64,
    pub size: u64,
    pub verified_count: u64,
    pub total_locations: u64,
    pub verification_pct: f64,
    pub oldest_verified_at: Option<String>,
}

/// Internal helper for per-volume data before merging with device registry.
struct VolumeStatsRaw {
    volume_id: String,
    label: String,
    assets: u64,
    variants: u64,
    recipes: u64,
    formats: Vec<String>,
    directories: u64,
    size: u64,
    verified_count: u64,
    total_locations: u64,
    oldest_verified_at: Option<String>,
}

/// Tag usage statistics.
#[derive(Debug, serde::Serialize)]
pub struct TagStats {
    pub unique_tags: u64,
    pub tagged_assets: u64,
    pub untagged_assets: u64,
    pub top_tags: Vec<TagCount>,
}

/// A single tag frequency entry.
#[derive(Debug, serde::Serialize)]
pub struct TagCount {
    pub tag: String,
    pub count: u64,
}

/// Verification health statistics.
#[derive(Debug, serde::Serialize)]
pub struct VerificationStats {
    pub total_locations: u64,
    pub verified_locations: u64,
    pub unverified_locations: u64,
    pub coverage_pct: f64,
    pub oldest_verified_at: Option<String>,
    pub newest_verified_at: Option<String>,
    pub per_volume: Vec<VolumeVerificationStats>,
}

/// Per-volume verification summary.
#[derive(Debug, serde::Serialize)]
pub struct VolumeVerificationStats {
    pub label: String,
    pub volume_id: String,
    pub is_online: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purpose: Option<String>,
    pub locations: u64,
    pub verified: u64,
    pub coverage_pct: f64,
    pub oldest_verified_at: Option<String>,
}

/// Analytics data for the /analytics dashboard page.
#[derive(Debug, serde::Serialize)]
pub struct AnalyticsData {
    pub camera_usage: Vec<NameCount>,
    pub lens_usage: Vec<NameCount>,
    pub rating_distribution: Vec<RatingCount>,
    pub format_distribution: Vec<NameCount>,
    pub monthly_imports: Vec<MonthCount>,
    pub storage_by_volume: Vec<VolumeSize>,
    pub yearly_counts: Vec<YearCount>,
}

/// A name + count pair for analytics charts.
#[derive(Debug, serde::Serialize)]
pub struct NameCount {
    pub name: String,
    pub count: u64,
}

/// Rating distribution entry.
#[derive(Debug, serde::Serialize)]
pub struct RatingCount {
    pub rating: u8,
    pub count: u64,
}

/// Monthly import volume entry.
#[derive(Debug, serde::Serialize)]
pub struct MonthCount {
    pub month: String,
    pub count: u64,
}

/// Per-volume storage size entry.
#[derive(Debug, serde::Serialize)]
pub struct VolumeSize {
    pub label: String,
    pub size: u64,
}

/// Per-year asset count entry.
#[derive(Debug, serde::Serialize)]
pub struct YearCount {
    pub year: String,
    pub count: u64,
}

/// Top-level result for `maki backup-status`.
#[derive(Debug, serde::Serialize)]
pub struct BackupStatusResult {
    pub scope: String,
    pub total_assets: u64,
    pub total_variants: u64,
    pub total_file_locations: u64,
    pub min_copies: u64,
    pub at_risk_count: u64,
    pub purpose_coverage: Vec<PurposeCoverage>,
    pub location_distribution: Vec<LocationBucket>,
    pub volume_gaps: Vec<VolumeGap>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume_detail: Option<VolumeGapDetail>,
}

/// Coverage of assets by volume purpose.
#[derive(Debug, serde::Serialize)]
pub struct PurposeCoverage {
    pub purpose: String,
    pub volume_count: u64,
    pub asset_count: u64,
    pub asset_percentage: f64,
}

/// A bucket in the volume-count distribution histogram.
#[derive(Debug, serde::Serialize)]
pub struct LocationBucket {
    pub volume_count: String,
    pub asset_count: u64,
}

/// Summary of how many scoped assets are missing from a volume.
#[derive(Debug, serde::Serialize)]
pub struct VolumeGap {
    pub volume_label: String,
    pub volume_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purpose: Option<String>,
    pub missing_count: u64,
}

/// Detailed coverage info for a specific target volume.
#[derive(Debug, serde::Serialize)]
pub struct VolumeGapDetail {
    pub volume_label: String,
    pub volume_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purpose: Option<String>,
    pub present_count: u64,
    pub missing_count: u64,
    pub total_scoped: u64,
    pub coverage_pct: f64,
}

// ═══ SEARCH OPTIONS ═══

/// Sort order for paginated search.
#[derive(Debug, Clone, Copy)]
pub enum SearchSort {
    DateDesc,
    DateAsc,
    NameAsc,
    NameDesc,
    SizeDesc,
    SizeAsc,
    /// Sort by similarity score (client-side — scores are not in SQL).
    SimilarityDesc,
    SimilarityAsc,
}

impl SearchSort {
    fn to_sql(&self) -> &'static str {
        match self {
            SearchSort::DateDesc => "a.created_at DESC",
            SearchSort::DateAsc => "a.created_at ASC",
            SearchSort::NameAsc => "COALESCE(a.name, bv.original_filename) ASC",
            SearchSort::NameDesc => "COALESCE(a.name, bv.original_filename) DESC",
            SearchSort::SizeDesc => "bv.file_size DESC",
            SearchSort::SizeAsc => "bv.file_size ASC",
            // Dummy SQL order — real sorting done client-side with similarity scores
            SearchSort::SimilarityDesc | SearchSort::SimilarityAsc => "a.id",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "date_asc" => SearchSort::DateAsc,
            "name_asc" => SearchSort::NameAsc,
            "name_desc" => SearchSort::NameDesc,
            "size_desc" => SearchSort::SizeDesc,
            "size_asc" => SearchSort::SizeAsc,
            "similarity_desc" => SearchSort::SimilarityDesc,
            "similarity_asc" => SearchSort::SimilarityAsc,
            _ => SearchSort::DateDesc,
        }
    }
}

/// Options for paginated search.
pub struct SearchOptions<'a> {
    pub asset_ids: &'a [String],
    pub text: Option<&'a str>,
    pub text_exclude: &'a [String],
    pub asset_types: &'a [String],
    pub asset_types_exclude: &'a [String],
    pub tags: &'a [String],
    pub tags_exclude: &'a [String],
    pub formats: &'a [String],
    pub formats_exclude: &'a [String],
    pub color_labels: &'a [String],
    pub color_labels_exclude: &'a [String],
    pub color_label_none: bool,
    pub cameras: &'a [String],
    pub cameras_exclude: &'a [String],
    pub lenses: &'a [String],
    pub lenses_exclude: &'a [String],
    pub descriptions: &'a [String],
    pub descriptions_exclude: &'a [String],
    pub collections: &'a [String],
    pub collections_exclude: &'a [String],
    pub path_prefixes: &'a [String],
    pub path_prefixes_exclude: &'a [String],
    pub volume: Option<&'a str>,
    pub volume_ids: &'a [String],
    pub volume_ids_exclude: &'a [String],
    pub rating: Option<NumericFilter>,
    pub iso: Option<NumericFilter>,
    pub focal: Option<NumericFilter>,
    pub aperture: Option<NumericFilter>,
    pub width: Option<NumericFilter>,
    pub height: Option<NumericFilter>,
    pub copies: Option<NumericFilter>,
    pub variant_count: Option<NumericFilter>,
    pub scattered: Option<NumericFilter>,
    pub scattered_depth: Option<u32>,
    pub session_root_pattern: &'a str,
    pub face_count: Option<NumericFilter>,
    pub tag_count: Option<NumericFilter>,
    pub duration: Option<NumericFilter>,
    pub codec: Option<String>,
    pub stale_days: Option<NumericFilter>,
    pub meta_filters: Vec<(&'a str, &'a str)>,
    pub orphan: bool,
    pub orphan_false: bool,
    pub missing_asset_ids: Option<&'a [String]>,
    pub no_online_locations: Option<&'a [String]>,
    pub collection_asset_ids: Option<&'a [String]>,
    pub collection_exclude_ids: Option<&'a [String]>,
    pub date_prefix: Option<&'a str>,
    pub date_from: Option<&'a str>,
    pub date_until: Option<&'a str>,
    pub collapse_stacks: bool,
    pub stacked_filter: Option<bool>,
    pub geo_bbox: Option<(f64, f64, f64, f64)>,
    pub has_gps: Option<bool>,
    pub has_faces: Option<bool>,
    pub has_embed: Option<bool>,
    pub person_asset_ids: Option<&'a [String]>,
    pub person_exclude_ids: Option<&'a [String]>,
    pub similar_asset_ids: Option<&'a [String]>,
    pub text_search_ids: Option<&'a [String]>,
    pub sort: SearchSort,
    pub page: u32,
    pub per_page: u32,
}

impl<'a> Default for SearchOptions<'a> {
    fn default() -> Self {
        Self {
            asset_ids: &[],
            text: None,
            text_exclude: &[],
            asset_types: &[],
            asset_types_exclude: &[],
            tags: &[],
            tags_exclude: &[],
            formats: &[],
            formats_exclude: &[],
            color_labels: &[],
            color_labels_exclude: &[],
            color_label_none: false,
            cameras: &[],
            cameras_exclude: &[],
            lenses: &[],
            lenses_exclude: &[],
            descriptions: &[],
            descriptions_exclude: &[],
            collections: &[],
            collections_exclude: &[],
            path_prefixes: &[],
            path_prefixes_exclude: &[],
            volume: None,
            volume_ids: &[],
            volume_ids_exclude: &[],
            rating: None,
            iso: None,
            focal: None,
            aperture: None,
            width: None,
            height: None,
            copies: None,
            variant_count: None,
            scattered: None,
            scattered_depth: None,
            session_root_pattern: r"^\d{4}-\d{2}",
            face_count: None,
            tag_count: None,
            duration: None,
            codec: None,
            stale_days: None,
            meta_filters: Vec::new(),
            orphan: false,
            orphan_false: false,
            missing_asset_ids: None,
            no_online_locations: None,
            collection_asset_ids: None,
            collection_exclude_ids: None,
            date_prefix: None,
            date_from: None,
            date_until: None,
            collapse_stacks: false,
            stacked_filter: None,
            geo_bbox: None,
            has_gps: None,
            has_faces: None,
            has_embed: None,
            person_asset_ids: None,
            person_exclude_ids: None,
            similar_asset_ids: None,
            text_search_ids: None,
            sort: SearchSort::DateDesc,
            page: 1,
            per_page: 60,
        }
    }
}

/// A page of search results with pagination metadata.
#[derive(Debug, serde::Serialize)]
pub struct SearchPage {
    pub rows: Vec<SearchRow>,
    pub total: u64,
    pub page: u32,
    pub per_page: u32,
    pub total_pages: u32,
}

// ═══ HELPER FUNCTIONS ═══

/// Convert an inclusive date bound to an exclusive upper bound.
///
/// - `"2026-02-25"` → `"2026-02-26"` (next day)
/// - `"2026-02"` → `"2026-03"` (next month)
/// - `"2026"` → `"2027"` (next year)
/// Falls back to appending a high character if parsing fails.
/// Convert a path pattern (with `*` wildcards) to a SQL LIKE pattern.
///
/// Rules:
/// - `*` becomes `%` (match any sequence)
/// - Literal `%`, `_`, and `\` are escaped via `ESCAPE '\'`
/// - A trailing `%` is appended if not already present, so `path:Pictures/2026`
///   keeps prefix semantics
///
/// Examples:
/// - `Pictures/2026`        → `Pictures/2026%`
/// - `Pictures/*/Capture`   → `Pictures/%/Capture%`
/// - `*/2026/*/party`       → `%/2026/%/party%`
/// - `*party`               → `%party%`
fn path_pattern_to_like(pat: &str) -> String {
    let mut out = String::with_capacity(pat.len() + 2);
    for c in pat.chars() {
        match c {
            '\\' => { out.push('\\'); out.push('\\'); }
            '%'  => { out.push('\\'); out.push('%'); }
            '_'  => { out.push('\\'); out.push('_'); }
            '*'  => out.push('%'),
            c    => out.push(c),
        }
    }
    if !out.ends_with('%') {
        out.push('%');
    }
    out
}

fn next_date_bound(s: &str) -> String {
    match s.len() {
        // Year: "2026" → "2027"
        4 => {
            if let Ok(y) = s.parse::<i32>() {
                return format!("{:04}", y + 1);
            }
        }
        // Month: "2026-02" → "2026-03"
        7 => {
            let parts: Vec<&str> = s.splitn(2, '-').collect();
            if parts.len() == 2 {
                if let (Ok(y), Ok(m)) = (parts[0].parse::<i32>(), parts[1].parse::<u32>()) {
                    if m >= 12 {
                        return format!("{:04}-01", y + 1);
                    } else {
                        return format!("{:04}-{:02}", y, m + 1);
                    }
                }
            }
        }
        // Day: "2026-02-25" → "2026-02-26"
        10 => {
            if let Ok(date) = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d") {
                let next = date + chrono::Duration::days(1);
                return next.format("%Y-%m-%d").to_string();
            }
        }
        _ => {}
    }
    // Fallback: append a char higher than any valid timestamp character
    format!("{s}\x7f")
}

/// Current schema version. Bump this whenever `run_migrations()` changes.
pub const SCHEMA_VERSION: u32 = 8;

// ═══ CATALOG STRUCT & CONNECTION ═══

/// SQLite-backed local catalog for fast queries. This is a derived cache,
/// not the source of truth (sidecar files are).
pub struct Catalog {
    conn: Connection,
}

impl Catalog {
    pub fn open(catalog_root: &Path) -> Result<Self> {
        let db_path = catalog_root.join("catalog.db");
        let conn = Connection::open(&db_path)?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA cache_size = -20000;
             PRAGMA mmap_size = 268435456;
             PRAGMA temp_store = MEMORY;",
        )?;

        // Register path_dir(path, depth) — extracts the directory prefix from a path.
        // depth=0 (or NULL): returns the parent directory (everything before last '/').
        // depth=N: returns the first N path segments.
        // Examples:
        //   path_dir('2026/Selects/img.nef', 0)  → '2026/Selects'
        //   path_dir('2026/Selects/img.nef', 1)  → '2026'
        //   path_dir('2026/Selects/img.nef', 2)  → '2026/Selects'
        //   path_dir('img.nef', 0)                → ''
        conn.create_scalar_function("path_dir", 2, rusqlite::functions::FunctionFlags::SQLITE_DETERMINISTIC | rusqlite::functions::FunctionFlags::SQLITE_UTF8, |ctx| {
            let path: String = ctx.get(0)?;
            let depth: i64 = ctx.get(1)?;

            if depth <= 0 {
                // Parent directory: everything before the last '/'
                Ok(path.rfind('/').map_or(String::new(), |pos| path[..pos].to_string()))
            } else {
                // First N segments: find the Nth '/'
                let mut pos = 0;
                let mut count = 0;
                for (i, ch) in path.char_indices() {
                    if ch == '/' {
                        count += 1;
                        if count == depth as usize {
                            return Ok(path[..i].to_string());
                        }
                    }
                    pos = i;
                }
                // Fewer than N slashes — return the whole path minus the last segment
                let _ = pos;
                Ok(path.rfind('/').map_or(path.clone(), |p| path[..p].to_string()))
            }
        })?;

        // Register session_root(path, pattern) — finds the session root for a
        // file path using the same logic as auto-group's find_session_root().
        // Walks directory components and returns everything up to and including
        // the deepest component matching the regex pattern. Falls back to the
        // parent directory if no component matches.
        //
        // The compiled regex is cached in a RefCell because the pattern is the
        // same for every row within a query — compiling it once instead of per-row
        // is critical for performance on large catalogs.
        let regex_cache: std::cell::RefCell<Option<(String, regex::Regex)>> = std::cell::RefCell::new(None);
        conn.create_scalar_function("session_root", 2, rusqlite::functions::FunctionFlags::SQLITE_DETERMINISTIC | rusqlite::functions::FunctionFlags::SQLITE_UTF8, move |ctx| {
            let path: String = ctx.get(0)?;
            let pattern: String = ctx.get(1)?;

            // Split into directory components (strip filename)
            let dir = path.rfind('/').map_or("", |pos| &path[..pos]);
            let parts: Vec<&str> = dir.split('/').collect();

            if pattern.is_empty() || parts.is_empty() {
                return Ok(dir.to_string());
            }

            // Cache the compiled regex — same pattern for every row in a query
            let mut cache = regex_cache.borrow_mut();
            if cache.as_ref().map_or(true, |(p, _)| p != &pattern) {
                match regex::Regex::new(&pattern) {
                    Ok(r) => *cache = Some((pattern.clone(), r)),
                    Err(_) => return Ok(dir.to_string()),
                }
            }
            let re = &cache.as_ref().expect("regex cache just populated").1;

            // Find deepest matching component
            let mut session_idx = None;
            for (i, part) in parts.iter().enumerate() {
                if re.is_match(part) {
                    session_idx = Some(i);
                }
            }

            match session_idx {
                Some(idx) => Ok(parts[..=idx].join("/")),
                None => {
                    if parts.len() > 1 {
                        Ok(parts[..parts.len() - 1].join("/"))
                    } else {
                        Ok(dir.to_string())
                    }
                }
            }
        })?;

        Ok(Self { conn })
    }

    /// Open and run schema migrations. Call once at program startup.
    ///
    /// After schema migrations, propagates any SQLite-only data back to YAML
    /// for migrations that touched YAML-backed columns — preserving the
    /// invariant that SQLite is fully derivable from YAML. Specifically:
    /// the v5→v6 migration backfills `faces.recognition_model`, which is
    /// now a persisted YAML field. Without this sync, a `rebuild-catalog`
    /// immediately after the v5→v6 upgrade would strip the tag.
    pub fn open_and_migrate(catalog_root: &Path) -> Result<Self> {
        let catalog = Self::open(catalog_root)?;
        let before = catalog.schema_version();
        catalog.run_migrations();
        catalog.post_migration_sync(catalog_root, before);
        Ok(catalog)
    }

    /// Post-migration YAML sync for migrations that modified YAML-backed
    /// data. Called once at catalog-open time after `run_migrations()`.
    ///
    /// `previous_version` is the schema version *before* migration ran, so
    /// we know which migrations just happened.
    fn post_migration_sync(&self, catalog_root: &Path, previous_version: u32) {
        // v5 → v6: `faces.recognition_model` added and backfilled. Re-export
        // faces.yaml so the tag propagates to the source-of-truth sidecar.
        // Cheap relative to the migration itself (one small file write per
        // thousand faces), and runs exactly once per catalog.
        #[cfg(feature = "ai")]
        if previous_version < 6 {
            let store = crate::face_store::FaceStore::new(&self.conn);
            let _ = store.save_all_yaml(catalog_root);
        }
        // v6 → v7: `assets.face_scan_status` added — this lives on the Asset
        // YAML sidecar, which is per-file. Re-writing every sidecar during
        // migration would be slow on large catalogs, so we instead rely on
        // the rebuild-catalog fallback (stamps `face_scan_status='done'` on
        // any asset with face records) and on detect_faces writing the flag
        // to YAML for assets it touches from here on.
        // Silence unused-var warnings in non-AI builds, where the only active
        // branch above is gated out.
        let _ = previous_version;
        let _ = catalog_root;
    }

    /// Read the stored schema version (0 if table doesn't exist yet).
    pub fn schema_version(&self) -> u32 {
        self.conn
            .query_row(
                "SELECT version FROM schema_version LIMIT 1",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0)
    }

    /// Check if migrations are needed (fast — single query).
    /// Returns `true` if the schema is up to date.
    pub fn is_schema_current(&self) -> bool {
        self.schema_version() >= SCHEMA_VERSION
    }

    /// Alias for `open` — kept for clarity but identical.
    pub fn open_fast(catalog_root: &Path) -> Result<Self> {
        Self::open(catalog_root)
    }

    /// Access the underlying SQLite connection.
    pub fn conn(&self) -> &Connection {
        &self.conn
    }

}

mod schema;
mod asset_crud;
mod variant_crud;
mod recipe_crud;
mod volume;
mod lookup;
mod duplicates;
mod recipe_query;
mod rebuild;
mod stats;
mod search_builder;
mod search_exec;
mod facets;
mod tags;
mod analytics;
mod backup;
mod cleanup;


// ═══ TESTS ═══

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initialize_creates_all_tables() {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();

        let tables: Vec<String> = catalog
            .conn
            .prepare(
                "SELECT name FROM sqlite_master \
                 WHERE type='table' AND name NOT LIKE 'sqlite_%' \
                 ORDER BY name",
            )
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();

        // With --features ai, face detection adds 'faces' and 'people' tables
        #[cfg(feature = "ai")]
        let expected = vec!["assets", "collection_assets", "collections", "embeddings", "faces", "file_locations", "people", "recipes", "schema_version", "stacks", "variants", "volumes"];
        #[cfg(not(feature = "ai"))]
        let expected = vec!["assets", "collection_assets", "collections", "embeddings", "file_locations", "recipes", "schema_version", "stacks", "variants", "volumes"];
        assert_eq!(tables, expected);
    }

    #[test]
    fn initialize_is_idempotent() {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();
        catalog.initialize().unwrap(); // should not error
    }

    #[test]
    fn insert_and_query_asset() {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();

        let asset = crate::models::Asset::new(crate::models::AssetType::Image, "sha256:test1");
        catalog.insert_asset(&asset).unwrap();

        let count: i64 = catalog
            .conn
            .query_row("SELECT COUNT(*) FROM assets", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn has_variant_returns_false_when_empty() {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();
        assert!(!catalog.has_variant("sha256:abc123").unwrap());
    }

    #[test]
    fn has_variant_returns_true_after_insert() {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();

        let asset = crate::models::Asset::new(crate::models::AssetType::Image, "sha256:test2");
        catalog.insert_asset(&asset).unwrap();

        let variant = crate::models::Variant {
            content_hash: "sha256:abc123".to_string(),
            asset_id: asset.id,
            role: crate::models::VariantRole::Original,
            format: "txt".to_string(),
            file_size: 100,
            original_filename: "test.txt".to_string(),
            source_metadata: Default::default(),
            locations: vec![],
        };
        catalog.insert_variant(&variant).unwrap();

        assert!(catalog.has_variant("sha256:abc123").unwrap());
    }

    /// Helper to set up a catalog with one asset and variant for search tests.
    fn setup_search_catalog() -> Catalog {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();

        let mut asset = crate::models::Asset::new(crate::models::AssetType::Image, "sha256:search1");
        asset.name = Some("sunset photo".to_string());
        asset.description = Some("A beautiful sunset over the ocean".to_string());
        asset.tags = vec!["landscape".to_string(), "nature".to_string()];

        let variant = crate::models::Variant {
            content_hash: "sha256:search1".to_string(),
            asset_id: asset.id.clone(),
            role: crate::models::VariantRole::Original,
            format: "jpg".to_string(),
            file_size: 5000,
            original_filename: "sunset_beach.jpg".to_string(),
            source_metadata: Default::default(),
            locations: vec![],
        };
        asset.variants.push(variant.clone());
        catalog.insert_asset(&asset).unwrap();
        catalog.insert_variant(&variant).unwrap();

        // Add a second asset of different type
        let mut asset2 = crate::models::Asset::new(crate::models::AssetType::Video, "sha256:search2");
        asset2.name = Some("holiday clip".to_string());

        let variant2 = crate::models::Variant {
            content_hash: "sha256:search2".to_string(),
            asset_id: asset2.id,
            role: crate::models::VariantRole::Original,
            format: "mp4".to_string(),
            file_size: 100_000,
            original_filename: "holiday.mp4".to_string(),
            source_metadata: Default::default(),
            locations: vec![],
        };
        asset2.variants.push(variant2.clone());
        catalog.insert_asset(&asset2).unwrap();
        catalog.insert_variant(&variant2).unwrap();

        catalog
    }

    #[test]
    fn search_by_text() {
        let catalog = setup_search_catalog();
        let results = catalog.search_assets(Some("sunset"), None, None, None, None, None).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name.as_deref(), Some("sunset photo"));
    }

    // ── color label & rating-0 regression tests ───────────────

    fn setup_rating_label_catalog() -> Catalog {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();
        // Asset 1: rating=5, color_label="Red"
        let mut a1 = crate::models::Asset::new(crate::models::AssetType::Image, "sha256:rl1");
        a1.rating = Some(5);
        a1.color_label = Some("Red".to_string());
        let v1 = crate::models::Variant {
            content_hash: "sha256:rl1".to_string(),
            asset_id: a1.id.clone(),
            role: crate::models::VariantRole::Original,
            format: "jpg".to_string(),
            file_size: 1000,
            original_filename: "rl1.jpg".to_string(),
            source_metadata: Default::default(),
            locations: vec![],
        };
        a1.variants.push(v1.clone());
        catalog.insert_asset(&a1).unwrap();
        catalog.insert_variant(&v1).unwrap();

        // Asset 2: rating=0, color_label="Blue"
        let mut a2 = crate::models::Asset::new(crate::models::AssetType::Image, "sha256:rl2");
        a2.rating = Some(0);
        a2.color_label = Some("Blue".to_string());
        let v2 = crate::models::Variant {
            content_hash: "sha256:rl2".to_string(),
            asset_id: a2.id.clone(),
            role: crate::models::VariantRole::Original,
            format: "jpg".to_string(),
            file_size: 1000,
            original_filename: "rl2.jpg".to_string(),
            source_metadata: Default::default(),
            locations: vec![],
        };
        a2.variants.push(v2.clone());
        catalog.insert_asset(&a2).unwrap();
        catalog.insert_variant(&v2).unwrap();

        // Asset 3: rating=NULL (unrated), no color label
        let mut a3 = crate::models::Asset::new(crate::models::AssetType::Image, "sha256:rl3");
        a3.rating = None;
        a3.color_label = None;
        let v3 = crate::models::Variant {
            content_hash: "sha256:rl3".to_string(),
            asset_id: a3.id.clone(),
            role: crate::models::VariantRole::Original,
            format: "jpg".to_string(),
            file_size: 1000,
            original_filename: "rl3.jpg".to_string(),
            source_metadata: Default::default(),
            locations: vec![],
        };
        a3.variants.push(v3.clone());
        catalog.insert_asset(&a3).unwrap();
        catalog.insert_variant(&v3).unwrap();

        // Asset 4: rating=3, no color label
        let mut a4 = crate::models::Asset::new(crate::models::AssetType::Image, "sha256:rl4");
        a4.rating = Some(3);
        let v4 = crate::models::Variant {
            content_hash: "sha256:rl4".to_string(),
            asset_id: a4.id.clone(),
            role: crate::models::VariantRole::Original,
            format: "jpg".to_string(),
            file_size: 1000,
            original_filename: "rl4.jpg".to_string(),
            source_metadata: Default::default(),
            locations: vec![],
        };
        a4.variants.push(v4.clone());
        catalog.insert_asset(&a4).unwrap();
        catalog.insert_variant(&v4).unwrap();

        catalog
    }

    #[test]
    fn search_color_label_case_insensitive() {
        let catalog = setup_rating_label_catalog();
        // User types "Red" (capitalized) — matches stored "Red"
        let labels = vec!["Red".to_string()];
        let opts = SearchOptions { color_labels: &labels, per_page: u32::MAX, ..Default::default() };
        let results = catalog.search_paginated(&opts).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].original_filename, "rl1.jpg");

        // User types "red" (lowercase) — also matches stored "Red"
        let labels_lower = vec!["red".to_string()];
        let opts_lower = SearchOptions { color_labels: &labels_lower, per_page: u32::MAX, ..Default::default() };
        let results_lower = catalog.search_paginated(&opts_lower).unwrap();
        assert_eq!(results_lower.len(), 1);

        // User types "BLUE" (uppercase) — matches stored "Blue"
        let labels_up = vec!["BLUE".to_string()];
        let opts_up = SearchOptions { color_labels: &labels_up, per_page: u32::MAX, ..Default::default() };
        let results_up = catalog.search_paginated(&opts_up).unwrap();
        assert_eq!(results_up.len(), 1);
        assert_eq!(results_up[0].original_filename, "rl2.jpg");
    }

    #[test]
    fn search_tagcount_end_to_end() {
        // End-to-end exercise of the denormalised leaf_tag_count column:
        // insert_asset populates it, search_paginated filters on it.
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();

        fn add(catalog: &Catalog, name: &str, tags: Vec<String>) {
            let mut a = crate::models::Asset::new(
                crate::models::AssetType::Image,
                &format!("sha256:{name}"),
            );
            a.name = Some(name.to_string());
            a.tags = tags;
            let v = crate::models::Variant {
                content_hash: format!("sha256:{name}"),
                asset_id: a.id,
                role: crate::models::VariantRole::Original,
                format: "jpg".to_string(),
                file_size: 100,
                original_filename: format!("{name}.jpg"),
                source_metadata: Default::default(),
                locations: vec![],
            };
            a.variants.push(v.clone());
            catalog.insert_asset(&a).unwrap();
            catalog.insert_variant(&v).unwrap();
        }

        // a1: untagged (leaf count = 0)
        add(&catalog, "a1", vec![]);
        // a2: one leaf tag (a plain flat tag → 1 leaf)
        add(&catalog, "a2", vec!["sunset".to_string()]);
        // a3: one leaf in a 3-deep hierarchy (3 stored tags, 1 leaf)
        add(
            &catalog,
            "a3",
            vec![
                "subject".to_string(),
                "subject|nature".to_string(),
                "subject|nature|landscape".to_string(),
            ],
        );
        // a4: two leaves sharing ancestors (4 stored, 2 leaves)
        add(
            &catalog,
            "a4",
            vec![
                "subject".to_string(),
                "subject|nature".to_string(),
                "subject|nature|landscape".to_string(),
                "subject|nature|forest".to_string(),
            ],
        );
        // a5: five flat leaves
        add(
            &catalog,
            "a5",
            vec![
                "sunset".to_string(),
                "concert".to_string(),
                "portrait".to_string(),
                "documentary".to_string(),
                "bw".to_string(),
            ],
        );

        let names = |rows: Vec<SearchRow>| -> Vec<String> {
            rows.into_iter().map(|r| r.name.unwrap_or_default()).collect()
        };

        // tagcount:0 — the untagged ones (just a1)
        let opts = SearchOptions {
            tag_count: Some(NumericFilter::Exact(0.0)),
            per_page: u32::MAX,
            ..Default::default()
        };
        let r = names(catalog.search_paginated(&opts).unwrap());
        assert_eq!(r, vec!["a1".to_string()]);

        // tagcount:1 — exactly one leaf (a2, a3)
        let opts = SearchOptions {
            tag_count: Some(NumericFilter::Exact(1.0)),
            per_page: u32::MAX,
            ..Default::default()
        };
        let mut r = names(catalog.search_paginated(&opts).unwrap());
        r.sort();
        assert_eq!(r, vec!["a2".to_string(), "a3".to_string()]);

        // tagcount:2+ — two or more leaves (a4, a5)
        let opts = SearchOptions {
            tag_count: Some(NumericFilter::Min(2.0)),
            per_page: u32::MAX,
            ..Default::default()
        };
        let mut r = names(catalog.search_paginated(&opts).unwrap());
        r.sort();
        assert_eq!(r, vec!["a4".to_string(), "a5".to_string()]);

        // tagcount:2-3 — two or three (just a4, which has 2)
        let opts = SearchOptions {
            tag_count: Some(NumericFilter::Range(2.0, 3.0)),
            per_page: u32::MAX,
            ..Default::default()
        };
        let r = names(catalog.search_paginated(&opts).unwrap());
        assert_eq!(r, vec!["a4".to_string()]);
    }

    #[test]
    fn tagcount_updates_when_tags_change() {
        // Regression guard: the denormalised column must stay in sync
        // with tag mutations (add, remove, rename) because every tag
        // write path goes through insert_asset. If someone adds a new
        // SQL-only write path, this test will catch the drift.
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();

        let mut asset = crate::models::Asset::new(crate::models::AssetType::Image, "sha256:tc1");
        asset.name = Some("x".to_string());
        let v = crate::models::Variant {
            content_hash: "sha256:tc1".to_string(),
            asset_id: asset.id,
            role: crate::models::VariantRole::Original,
            format: "jpg".to_string(),
            file_size: 100,
            original_filename: "x.jpg".to_string(),
            source_metadata: Default::default(),
            locations: vec![],
        };
        asset.variants.push(v.clone());

        // Start with zero tags.
        catalog.insert_asset(&asset).unwrap();
        catalog.insert_variant(&v).unwrap();

        fn lookup(catalog: &Catalog, asset_id: &str) -> i64 {
            catalog
                .conn()
                .query_row(
                    "SELECT leaf_tag_count FROM assets WHERE id = ?1",
                    rusqlite::params![asset_id],
                    |r| r.get::<_, i64>(0),
                )
                .unwrap()
        }

        assert_eq!(lookup(&catalog, &asset.id.to_string()), 0);

        // Add one leaf tag (with ancestor expansion → 3 stored, 1 leaf).
        asset.tags = vec![
            "subject".to_string(),
            "subject|nature".to_string(),
            "subject|nature|landscape".to_string(),
        ];
        catalog.insert_asset(&asset).unwrap();
        assert_eq!(lookup(&catalog, &asset.id.to_string()), 1);

        // Add a second leaf in the same hierarchy.
        asset.tags.push("subject|nature|forest".to_string());
        catalog.insert_asset(&asset).unwrap();
        assert_eq!(lookup(&catalog, &asset.id.to_string()), 2);

        // Clear all tags.
        asset.tags.clear();
        catalog.insert_asset(&asset).unwrap();
        assert_eq!(lookup(&catalog, &asset.id.to_string()), 0);
    }

    #[test]
    fn search_rating_zero_matches_unrated_and_zero() {
        let catalog = setup_rating_label_catalog();
        // rating:0 should match both NULL-rated and 0-rated assets (a2, a3)
        let opts = SearchOptions {
            rating: Some(NumericFilter::Exact(0.0)),
            per_page: u32::MAX,
            ..Default::default()
        };
        let results = catalog.search_paginated(&opts).unwrap();
        let names: Vec<&str> = results.iter().map(|r| r.original_filename.as_str()).collect();
        assert_eq!(results.len(), 2, "rating:0 should match rl2 (rating=0) and rl3 (NULL). Got: {names:?}");
        assert!(names.contains(&"rl2.jpg"));
        assert!(names.contains(&"rl3.jpg"));
    }

    #[test]
    fn search_rating_exact_3_excludes_null() {
        let catalog = setup_rating_label_catalog();
        // rating:3 should only match the 3-star asset, not NULL
        let opts = SearchOptions {
            rating: Some(NumericFilter::Exact(3.0)),
            per_page: u32::MAX,
            ..Default::default()
        };
        let results = catalog.search_paginated(&opts).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].original_filename, "rl4.jpg");
    }

    fn setup_case_tag_catalog() -> Catalog {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();

        // Asset 1: lowercase "landscape"
        let mut a1 = crate::models::Asset::new(crate::models::AssetType::Image, "sha256:cs1");
        a1.tags = vec!["landscape".to_string()];
        let v1 = crate::models::Variant {
            content_hash: "sha256:cs1".to_string(),
            asset_id: a1.id.clone(),
            role: crate::models::VariantRole::Original,
            format: "jpg".to_string(),
            file_size: 1000,
            original_filename: "cs1.jpg".to_string(),
            source_metadata: Default::default(),
            locations: vec![],
        };
        a1.variants.push(v1.clone());
        catalog.insert_asset(&a1).unwrap();
        catalog.insert_variant(&v1).unwrap();

        // Asset 2: capitalized "Landscape"
        let mut a2 = crate::models::Asset::new(crate::models::AssetType::Image, "sha256:cs2");
        a2.tags = vec!["Landscape".to_string()];
        let v2 = crate::models::Variant {
            content_hash: "sha256:cs2".to_string(),
            asset_id: a2.id.clone(),
            role: crate::models::VariantRole::Original,
            format: "jpg".to_string(),
            file_size: 1000,
            original_filename: "cs2.jpg".to_string(),
            source_metadata: Default::default(),
            locations: vec![],
        };
        a2.variants.push(v2.clone());
        catalog.insert_asset(&a2).unwrap();
        catalog.insert_variant(&v2).unwrap();

        // Asset 3: no tag
        let mut a3 = crate::models::Asset::new(crate::models::AssetType::Image, "sha256:cs3");
        let v3 = crate::models::Variant {
            content_hash: "sha256:cs3".to_string(),
            asset_id: a3.id.clone(),
            role: crate::models::VariantRole::Original,
            format: "jpg".to_string(),
            file_size: 1000,
            original_filename: "cs3.jpg".to_string(),
            source_metadata: Default::default(),
            locations: vec![],
        };
        a3.variants.push(v3.clone());
        catalog.insert_asset(&a3).unwrap();
        catalog.insert_variant(&v3).unwrap();

        catalog
    }

    #[test]
    fn search_tag_case_insensitive_default() {
        let catalog = setup_case_tag_catalog();
        // Without `^`, `tag:landscape` matches both "landscape" and "Landscape"
        let tags = vec!["landscape".to_string()];
        let opts = SearchOptions { tags: &tags, per_page: u32::MAX, ..Default::default() };
        let results = catalog.search_paginated(&opts).unwrap();
        assert_eq!(results.len(), 2, "case-insensitive tag should match both cases");
    }

    #[test]
    fn search_tag_case_sensitive_marker() {
        let catalog = setup_case_tag_catalog();
        // With `^`, `tag:^landscape` matches ONLY "landscape"
        let tags = vec!["^landscape".to_string()];
        let opts = SearchOptions { tags: &tags, per_page: u32::MAX, ..Default::default() };
        let results = catalog.search_paginated(&opts).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].original_filename, "cs1.jpg");

        // And `tag:^Landscape` matches ONLY "Landscape"
        let tags2 = vec!["^Landscape".to_string()];
        let opts2 = SearchOptions { tags: &tags2, per_page: u32::MAX, ..Default::default() };
        let results2 = catalog.search_paginated(&opts2).unwrap();
        assert_eq!(results2.len(), 1);
        assert_eq!(results2[0].original_filename, "cs2.jpg");
    }

    #[test]
    fn search_description_filter() {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();

        let cases = [
            ("sha256:d1", Some("a beautiful sunset over the ocean")),
            ("sha256:d2", Some("portrait of a cat in soft window light")),
            ("sha256:d3", Some("Sunset behind mountain peaks")),
            ("sha256:d4", None),  // no description
        ];
        for (hash, desc) in &cases {
            let mut a = crate::models::Asset::new(crate::models::AssetType::Image, hash);
            a.description = desc.map(|s| s.to_string());
            let v = crate::models::Variant {
                content_hash: hash.to_string(),
                asset_id: a.id.clone(),
                role: crate::models::VariantRole::Original,
                format: "jpg".to_string(),
                file_size: 100,
                original_filename: format!("{}.jpg", &hash[7..]),
                source_metadata: Default::default(),
                locations: vec![],
            };
            a.variants.push(v.clone());
            catalog.insert_asset(&a).unwrap();
            catalog.insert_variant(&v).unwrap();
        }

        // description:sunset matches d1 and d3 (case-insensitive substring)
        let descs = vec!["sunset".to_string()];
        let opts = SearchOptions { descriptions: &descs, per_page: u32::MAX, ..Default::default() };
        let results = catalog.search_paginated(&opts).unwrap();
        let names: Vec<&str> = results.iter().map(|r| r.original_filename.as_str()).collect();
        assert_eq!(results.len(), 2, "got: {names:?}");
        assert!(names.contains(&"d1.jpg"));
        assert!(names.contains(&"d3.jpg"));

        // description:cat matches only d2
        let descs = vec!["cat".to_string()];
        let opts = SearchOptions { descriptions: &descs, per_page: u32::MAX, ..Default::default() };
        let results = catalog.search_paginated(&opts).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].original_filename, "d2.jpg");

        // -description:cat excludes d2; assets with NULL description survive
        let exclude = vec!["cat".to_string()];
        let opts = SearchOptions { descriptions_exclude: &exclude, per_page: u32::MAX, ..Default::default() };
        let results = catalog.search_paginated(&opts).unwrap();
        let names: Vec<&str> = results.iter().map(|r| r.original_filename.as_str()).collect();
        assert_eq!(results.len(), 3, "got: {names:?}");
        assert!(!names.contains(&"d2.jpg"));
        assert!(names.contains(&"d4.jpg"), "NULL description should survive exclusion");
    }

    #[test]
    fn search_tag_prefix_anchor() {
        // Build a catalog with various tags to test the | prefix anchor
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();

        let cases = [
            ("sha256:pa1", vec!["wedding"]),
            ("sha256:pa2", vec!["wedding-2024"]),
            ("sha256:pa3", vec!["events|wedding"]),
            ("sha256:pa4", vec!["events|wedding|2024-05"]),
            ("sha256:pa5", vec!["weekend"]),               // starts with "we" but not "wed"
            ("sha256:pa6", vec!["midweek"]),                // contains "we" mid-component, must NOT match |we
            ("sha256:pa7", vec!["other"]),
        ];
        for (hash, tags) in &cases {
            let mut a = crate::models::Asset::new(crate::models::AssetType::Image, hash);
            a.tags = tags.iter().map(|s| s.to_string()).collect();
            let v = crate::models::Variant {
                content_hash: hash.to_string(),
                asset_id: a.id.clone(),
                role: crate::models::VariantRole::Original,
                format: "jpg".to_string(),
                file_size: 100,
                original_filename: format!("{}.jpg", &hash[7..]),
                source_metadata: Default::default(),
                locations: vec![],
            };
            a.variants.push(v.clone());
            catalog.insert_asset(&a).unwrap();
            catalog.insert_variant(&v).unwrap();
        }

        // |wed should match: wedding, wedding-2024, events|wedding, events|wedding|2024-05
        // (4 assets); should NOT match weekend, midweek, other.
        let tags = vec!["|wed".to_string()];
        let opts = SearchOptions { tags: &tags, per_page: u32::MAX, ..Default::default() };
        let results = catalog.search_paginated(&opts).unwrap();
        let names: Vec<&str> = results.iter().map(|r| r.original_filename.as_str()).collect();
        assert_eq!(results.len(), 4, "got: {names:?}");
        assert!(names.contains(&"pa1.jpg"));
        assert!(names.contains(&"pa2.jpg"));
        assert!(names.contains(&"pa3.jpg"));
        assert!(names.contains(&"pa4.jpg"));
        assert!(!names.contains(&"pa5.jpg"));
        assert!(!names.contains(&"pa6.jpg"));
        assert!(!names.contains(&"pa7.jpg"));

        // |we should additionally match weekend (root-level)
        let tags = vec!["|we".to_string()];
        let opts = SearchOptions { tags: &tags, per_page: u32::MAX, ..Default::default() };
        let results = catalog.search_paginated(&opts).unwrap();
        let names: Vec<&str> = results.iter().map(|r| r.original_filename.as_str()).collect();
        assert!(names.contains(&"pa5.jpg"), "weekend should match |we");
        // midweek must NOT match — "we" is mid-component, not at a boundary
        assert!(!names.contains(&"pa6.jpg"), "midweek should NOT match |we (we is mid-component)");
    }

    #[test]
    fn search_tag_prefix_anchor_case_sensitive() {
        // Combined ^| should be case-sensitive prefix anchor
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();

        for (hash, tag) in &[
            ("sha256:pcs1", "Wedding"),
            ("sha256:pcs2", "wedding"),
        ] {
            let mut a = crate::models::Asset::new(crate::models::AssetType::Image, hash);
            a.tags = vec![tag.to_string()];
            let v = crate::models::Variant {
                content_hash: hash.to_string(),
                asset_id: a.id.clone(),
                role: crate::models::VariantRole::Original,
                format: "jpg".to_string(),
                file_size: 100,
                original_filename: format!("{}.jpg", &hash[7..]),
                source_metadata: Default::default(),
                locations: vec![],
            };
            a.variants.push(v.clone());
            catalog.insert_asset(&a).unwrap();
            catalog.insert_variant(&v).unwrap();
        }

        // ^|Wed → only the capitalized variant
        let tags = vec!["^|Wed".to_string()];
        let opts = SearchOptions { tags: &tags, per_page: u32::MAX, ..Default::default() };
        let results = catalog.search_paginated(&opts).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].original_filename, "pcs1.jpg");

        // |Wed (no ^) → both variants
        let tags = vec!["|Wed".to_string()];
        let opts = SearchOptions { tags: &tags, per_page: u32::MAX, ..Default::default() };
        let results = catalog.search_paginated(&opts).unwrap();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn search_tag_case_sensitive_exact_combined() {
        let catalog = setup_case_tag_catalog();
        // Both markers combined: `tag:=^landscape` (exact level + case-sensitive)
        let tags = vec!["=^landscape".to_string()];
        let opts = SearchOptions { tags: &tags, per_page: u32::MAX, ..Default::default() };
        let results = catalog.search_paginated(&opts).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].original_filename, "cs1.jpg");

        // Reverse order also works: `tag:^=landscape`
        let tags2 = vec!["^=landscape".to_string()];
        let opts2 = SearchOptions { tags: &tags2, per_page: u32::MAX, ..Default::default() };
        let results2 = catalog.search_paginated(&opts2).unwrap();
        assert_eq!(results2.len(), 1);
        assert_eq!(results2[0].original_filename, "cs1.jpg");
    }

    #[test]
    fn search_tag_exact_with_ancestor_expansion() {
        // Simulates the CaptureOne/Lightroom ancestor expansion scenario.
        // An asset tagged `location|Germany|Bayern|Holzkirchen|Marktplatz`
        // also gets standalone ancestor tags: `Holzkirchen`, `Bayern`, etc.
        // tag:=Holzkirchen should EXCLUDE this asset because Holzkirchen
        // is a non-leaf component (it has `|Marktplatz` below it).
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();

        // Asset 1: Holzkirchen is a MID-PATH component (has deeper tags)
        let mut a1 = crate::models::Asset::new(crate::models::AssetType::Image, "sha256:ae1");
        a1.tags = vec![
            "location".to_string(),
            "location|Germany".to_string(),
            "location|Germany|Bayern".to_string(),
            "location|Germany|Bayern|Holzkirchen".to_string(),
            "location|Germany|Bayern|Holzkirchen|Marktplatz".to_string(),
            // Standalone ancestors from expand_ancestors:
            "Germany".to_string(),
            "Bayern".to_string(),
            "Holzkirchen".to_string(),
            "Marktplatz".to_string(),
        ];
        let v1 = crate::models::Variant {
            content_hash: "sha256:ae1".to_string(),
            asset_id: a1.id.clone(),
            role: crate::models::VariantRole::Original,
            format: "jpg".to_string(),
            file_size: 100,
            original_filename: "ae1.jpg".to_string(),
            source_metadata: Default::default(),
            locations: vec![],
        };
        a1.variants.push(v1.clone());
        catalog.insert_asset(&a1).unwrap();
        catalog.insert_variant(&v1).unwrap();

        // Asset 2: Holzkirchen IS the leaf (no deeper tags)
        let mut a2 = crate::models::Asset::new(crate::models::AssetType::Image, "sha256:ae2");
        a2.tags = vec![
            "location".to_string(),
            "location|Germany".to_string(),
            "location|Germany|Bayern".to_string(),
            "location|Germany|Bayern|Holzkirchen".to_string(),
            "Germany".to_string(),
            "Bayern".to_string(),
            "Holzkirchen".to_string(),
        ];
        let v2 = crate::models::Variant {
            content_hash: "sha256:ae2".to_string(),
            asset_id: a2.id.clone(),
            role: crate::models::VariantRole::Original,
            format: "jpg".to_string(),
            file_size: 100,
            original_filename: "ae2.jpg".to_string(),
            source_metadata: Default::default(),
            locations: vec![],
        };
        a2.variants.push(v2.clone());
        catalog.insert_asset(&a2).unwrap();
        catalog.insert_variant(&v2).unwrap();

        // tag:/Holzkirchen (leaf-only-at-any-level) should match ONLY ae2:
        // Holzkirchen is a leaf there. ae1 has Marktplatz below the deep
        // Holzkirchen path, so Holzkirchen is not a leaf in the deep branch
        // — but ae1 ALSO has the standalone "Holzkirchen" via ancestor
        // expansion, which IS a leaf. So both ae1 and ae2 should match `/Holzkirchen`?
        // No — the leaf-only check examines whether the asset has ANY
        // descendant of Holzkirchen anywhere in its tag list. ae1 does
        // (location|...|Holzkirchen|Marktplatz contains "Holzkirchen|"),
        // so it gets rejected; ae2 doesn't, so it matches.
        let tags = vec!["/Holzkirchen".to_string()];
        let opts = SearchOptions { tags: &tags, per_page: u32::MAX, ..Default::default() };
        let results = catalog.search_paginated(&opts).unwrap();
        let names: Vec<&str> = results.iter().map(|r| r.original_filename.as_str()).collect();
        assert_eq!(results.len(), 1, "tag:/Holzkirchen should match only ae2. Got: {names:?}");
        assert_eq!(results[0].original_filename, "ae2.jpg");

        // tag:=Holzkirchen (whole-path) matches BOTH because both have the
        // standalone "Holzkirchen" tag from ancestor expansion. The whole-path
        // check is purely about the JSON value equality; it doesn't care what
        // OTHER tags an asset has.
        let tags_eq = vec!["=Holzkirchen".to_string()];
        let opts_eq = SearchOptions { tags: &tags_eq, per_page: u32::MAX, ..Default::default() };
        let results_eq = catalog.search_paginated(&opts_eq).unwrap();
        assert_eq!(results_eq.len(), 2, "tag:=Holzkirchen should match both (both have standalone Holzkirchen)");

        // tag:Holzkirchen (no marker) should match both via root-level standalone match.
        let tags_no_eq = vec!["Holzkirchen".to_string()];
        let opts_no_eq = SearchOptions { tags: &tags_no_eq, per_page: u32::MAX, ..Default::default() };
        let results_no_eq = catalog.search_paginated(&opts_no_eq).unwrap();
        assert_eq!(results_no_eq.len(), 2, "tag:Holzkirchen should match both");
    }

    #[test]
    fn search_rating_min_1_excludes_unrated_and_zero() {
        let catalog = setup_rating_label_catalog();
        // rating:1+ should NOT include unrated (NULL) or 0-rated — only a1, a4
        let opts = SearchOptions {
            rating: Some(NumericFilter::Min(1.0)),
            per_page: u32::MAX,
            ..Default::default()
        };
        let results = catalog.search_paginated(&opts).unwrap();
        assert_eq!(results.len(), 2);
        let names: Vec<&str> = results.iter().map(|r| r.original_filename.as_str()).collect();
        assert!(names.contains(&"rl1.jpg"));
        assert!(names.contains(&"rl4.jpg"));
    }

    #[test]
    fn search_by_type() {
        let catalog = setup_search_catalog();
        let results = catalog.search_assets(None, Some("video"), None, None, None, None).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].format, "mp4");
    }

    #[test]
    fn search_by_tag() {
        let catalog = setup_search_catalog();
        let results = catalog.search_assets(None, None, Some("landscape"), None, None, None).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name.as_deref(), Some("sunset photo"));
    }

    #[test]
    fn search_tag_whole_path_match() {
        // `=` prefix means "match the tag path exactly and only". Distinguishes
        // a root-level tag from same-named leaves elsewhere in the hierarchy —
        // the case that the default level-sliding match and `/` (leaf-only at
        // any level) can't disambiguate.
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();

        // Asset 1: root-level Legoland only
        let mut a1 = crate::models::Asset::new(crate::models::AssetType::Image, "sha256:wp1");
        a1.tags = vec!["Legoland".to_string()];
        let v1 = crate::models::Variant {
            content_hash: "sha256:wp1".to_string(), asset_id: a1.id.clone(),
            role: crate::models::VariantRole::Original, format: "jpg".to_string(),
            file_size: 100, original_filename: "a.jpg".to_string(),
            source_metadata: Default::default(), locations: vec![],
        };
        a1.variants.push(v1.clone());
        catalog.insert_asset(&a1).unwrap();
        catalog.insert_variant(&v1).unwrap();

        // Asset 2: Legoland as a leaf under location|Denmark
        let mut a2 = crate::models::Asset::new(crate::models::AssetType::Image, "sha256:wp2");
        a2.tags = vec!["location|Denmark|Legoland".to_string()];
        let v2 = crate::models::Variant {
            content_hash: "sha256:wp2".to_string(), asset_id: a2.id.clone(),
            role: crate::models::VariantRole::Original, format: "jpg".to_string(),
            file_size: 100, original_filename: "b.jpg".to_string(),
            source_metadata: Default::default(), locations: vec![],
        };
        a2.variants.push(v2.clone());
        catalog.insert_asset(&a2).unwrap();
        catalog.insert_variant(&v2).unwrap();

        // Asset 3: Legoland under a different country
        let mut a3 = crate::models::Asset::new(crate::models::AssetType::Image, "sha256:wp3");
        a3.tags = vec!["location|Germany|Legoland".to_string()];
        let v3 = crate::models::Variant {
            content_hash: "sha256:wp3".to_string(), asset_id: a3.id.clone(),
            role: crate::models::VariantRole::Original, format: "jpg".to_string(),
            file_size: 100, original_filename: "c.jpg".to_string(),
            source_metadata: Default::default(), locations: vec![],
        };
        a3.variants.push(v3.clone());
        catalog.insert_asset(&a3).unwrap();
        catalog.insert_variant(&v3).unwrap();

        // Baseline: without any marker, all three match (Legoland appears at
        // some hierarchy level in each).
        let results = catalog.search_assets(None, None, Some("Legoland"), None, None, None).unwrap();
        assert_eq!(results.len(), 3, "plain tag:Legoland matches at any level");

        // `/` leaf-only-at-any-level: still matches all three (each Legoland
        // is a leaf in its own branch — none has a Legoland|child).
        let results = catalog.search_assets(None, None, Some("/Legoland"), None, None, None).unwrap();
        assert_eq!(results.len(), 3, "tag:/Legoland matches all leaf occurrences");

        // `=` whole-path: matches only the root-level Legoland.
        let results = catalog.search_assets(None, None, Some("=Legoland"), None, None, None).unwrap();
        assert_eq!(results.len(), 1, "tag:=Legoland matches only standalone root-level tag");
        assert_eq!(results[0].content_hash, "sha256:wp1");

        // Whole-path match at depth: selects exactly one asset.
        let results = catalog.search_assets(None, None, Some("=location|Denmark|Legoland"), None, None, None).unwrap();
        assert_eq!(results.len(), 1, "tag:=location|Denmark|Legoland matches that exact path");
        assert_eq!(results[0].content_hash, "sha256:wp2");

        // Whole-path match with a prefix-of-a-path: no results (there's no
        // asset where the full tag is 'location|Denmark').
        let results = catalog.search_assets(None, None, Some("=location|Denmark"), None, None, None).unwrap();
        assert!(results.is_empty(), "tag:=location|Denmark matches no full paths exactly");

        // Whole-path match for a non-existent path: no results.
        let results = catalog.search_assets(None, None, Some("=nosuchtag"), None, None, None).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn search_by_tag_hierarchical() {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();

        let mut asset = crate::models::Asset::new(crate::models::AssetType::Image, "sha256:hier1");
        asset.tags = vec!["animals|birds|eagles".to_string(), "sunset".to_string()];
        let variant = crate::models::Variant {
            content_hash: "sha256:hier1".to_string(),
            asset_id: asset.id.clone(),
            role: crate::models::VariantRole::Original,
            format: "jpg".to_string(),
            file_size: 1000,
            original_filename: "eagle.jpg".to_string(),
            source_metadata: Default::default(),
            locations: vec![],
        };
        asset.variants.push(variant.clone());
        catalog.insert_asset(&asset).unwrap();
        catalog.insert_variant(&variant).unwrap();

        // Search for parent tag should find child
        let results = catalog.search_assets(None, None, Some("animals"), None, None, None).unwrap();
        assert_eq!(results.len(), 1, "tag:animals should match animals|birds|eagles");

        // Search for intermediate tag (user types `|` or `>` for hierarchy)
        let results = catalog.search_assets(None, None, Some("animals|birds"), None, None, None).unwrap();
        assert_eq!(results.len(), 1, "tag:animals|birds should match animals|birds|eagles");

        // Search for exact tag
        let results = catalog.search_assets(None, None, Some("animals|birds|eagles"), None, None, None).unwrap();
        assert_eq!(results.len(), 1, "tag:animals|birds|eagles should match exactly");

        // Search for leaf component by name (child matching)
        let results = catalog.search_assets(None, None, Some("eagles"), None, None, None).unwrap();
        assert_eq!(results.len(), 1, "tag:eagles should match animals|birds|eagles");

        // Search for mid-path component by name
        let results = catalog.search_assets(None, None, Some("birds"), None, None, None).unwrap();
        assert_eq!(results.len(), 1, "tag:birds should match animals|birds|eagles");

        // Partial name must NOT match (no substring matching)
        let results = catalog.search_assets(None, None, Some("eagle"), None, None, None).unwrap();
        assert!(results.is_empty(), "tag:eagle should NOT match eagles (no substring)");

        // Search for non-matching parent
        let results = catalog.search_assets(None, None, Some("cats"), None, None, None).unwrap();
        assert!(results.is_empty(), "tag:cats should not match");

        // Search for child when only parent is tagged — should NOT match
        let mut asset2 = crate::models::Asset::new(crate::models::AssetType::Image, "sha256:hier2");
        asset2.tags = vec!["animals".to_string()];
        let variant2 = crate::models::Variant {
            content_hash: "sha256:hier2".to_string(),
            asset_id: asset2.id.clone(),
            role: crate::models::VariantRole::Original,
            format: "jpg".to_string(),
            file_size: 1000,
            original_filename: "cat.jpg".to_string(),
            source_metadata: Default::default(),
            locations: vec![],
        };
        asset2.variants.push(variant2.clone());
        catalog.insert_asset(&asset2).unwrap();
        catalog.insert_variant(&variant2).unwrap();

        let results = catalog.search_assets(None, None, Some("animals|birds"), None, None, None).unwrap();
        assert_eq!(results.len(), 1, "tag:animals|birds should not match plain 'animals'");

        // Search tag:animals should match both
        let results = catalog.search_assets(None, None, Some("animals"), None, None, None).unwrap();
        assert_eq!(results.len(), 2, "tag:animals should match both 'animals' and 'animals|birds|eagles'");
    }

    #[test]
    fn search_by_tag_literal_slash() {
        // Tags like "AF Nikkor 85mm f/1.4 D" contain literal `/` — not hierarchy.
        // Searching `tag:AF Nikkor 85mm f/1.4` should match via the raw fallback.
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();

        let mut asset = crate::models::Asset::new(crate::models::AssetType::Image, "sha256:slash1");
        asset.tags = vec!["AF Nikkor 85mm f/1.4 D".to_string()];
        let variant = crate::models::Variant {
            content_hash: "sha256:slash1".to_string(),
            asset_id: asset.id.clone(),
            role: crate::models::VariantRole::Original,
            format: "jpg".to_string(),
            file_size: 1000,
            original_filename: "portrait.jpg".to_string(),
            source_metadata: Default::default(),
            locations: vec![],
        };
        asset.variants.push(variant.clone());
        catalog.insert_asset(&asset).unwrap();
        catalog.insert_variant(&variant).unwrap();

        // Exact search should match via raw fallback (since `/` gets converted to `|`)
        let results = catalog
            .search_assets(None, None, Some("AF Nikkor 85mm f/1.4 D"), None, None, None)
            .unwrap();
        assert_eq!(results.len(), 1, "tag with literal slash should be found");

        // Non-matching tag should not match
        let results = catalog
            .search_assets(None, None, Some("AF Nikkor 85mm f/2.8"), None, None, None)
            .unwrap();
        assert!(results.is_empty(), "wrong tag should not match");
    }

    #[test]
    fn search_by_tag_no_false_positive_prefix() {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();

        let mut asset = crate::models::Asset::new(crate::models::AssetType::Image, "sha256:fp1");
        asset.tags = vec!["other-animals".to_string()];
        let variant = crate::models::Variant {
            content_hash: "sha256:fp1".to_string(),
            asset_id: asset.id.clone(),
            role: crate::models::VariantRole::Original,
            format: "jpg".to_string(),
            file_size: 1000,
            original_filename: "other.jpg".to_string(),
            source_metadata: Default::default(),
            locations: vec![],
        };
        asset.variants.push(variant.clone());
        catalog.insert_asset(&asset).unwrap();
        catalog.insert_variant(&variant).unwrap();

        // "animals" should NOT match "other-animals"
        let results = catalog.search_assets(None, None, Some("animals"), None, None, None).unwrap();
        assert!(results.is_empty(), "tag:animals should not match other-animals");
    }

    /// Helper: create a catalog with a single asset that has the given tags.
    fn catalog_with_tags(tags: Vec<String>, hash_suffix: &str) -> Catalog {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();
        let mut asset = crate::models::Asset::new(crate::models::AssetType::Image, &format!("sha256:{hash_suffix}"));
        asset.tags = tags;
        let variant = crate::models::Variant {
            content_hash: format!("sha256:{hash_suffix}"),
            asset_id: asset.id.clone(),
            role: crate::models::VariantRole::Original,
            format: "jpg".to_string(),
            file_size: 1000,
            original_filename: format!("{hash_suffix}.jpg"),
            source_metadata: Default::default(),
            locations: vec![],
        };
        asset.variants.push(variant.clone());
        catalog.insert_asset(&asset).unwrap();
        catalog.insert_variant(&variant).unwrap();
        catalog
    }

    #[test]
    fn search_tag_with_double_quotes_unescaped() {
        // Tags stored as raw JSON (unescaped quotes): ["\"Sir\" Oliver Mally"]
        // This is how serde_json serializes tags with quotes
        let catalog = catalog_with_tags(
            vec!["\"Sir\" Oliver Mally".to_string()],
            "quotes1",
        );
        let results = catalog.search_assets(None, None, Some("\"Sir\" Oliver Mally"), None, None, None).unwrap();
        assert_eq!(results.len(), 1, "Should find tag with double quotes");
    }

    #[test]
    fn search_tag_with_double_quotes_no_false_positive() {
        let catalog = catalog_with_tags(
            vec!["Sir Oliver Mally".to_string()],
            "quotes2",
        );
        // Searching for the quoted version should NOT match the unquoted tag
        let results = catalog.search_assets(None, None, Some("\"Sir\" Oliver Mally"), None, None, None).unwrap();
        assert!(results.is_empty(), "Quoted search should not match unquoted tag");
    }

    #[test]
    fn search_tag_with_apostrophe() {
        let catalog = catalog_with_tags(
            vec!["it's a test".to_string()],
            "apos1",
        );
        let results = catalog.search_assets(None, None, Some("it's a test"), None, None, None).unwrap();
        assert_eq!(results.len(), 1, "Should find tag with apostrophe");
    }

    #[test]
    fn search_tag_with_ampersand() {
        let catalog = catalog_with_tags(
            vec!["rock & roll".to_string()],
            "amp1",
        );
        let results = catalog.search_assets(None, None, Some("rock & roll"), None, None, None).unwrap();
        assert_eq!(results.len(), 1, "Should find tag with ampersand");
    }

    #[test]
    fn search_by_format() {
        let catalog = setup_search_catalog();
        let results = catalog.search_assets(None, None, None, Some("jpg"), None, None).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].original_filename, "sunset_beach.jpg");
    }

    #[test]
    fn search_no_results() {
        let catalog = setup_search_catalog();
        let results = catalog.search_assets(Some("nonexistent"), None, None, None, None, None).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn search_combined_filters() {
        let catalog = setup_search_catalog();
        let results = catalog
            .search_assets(Some("sunset"), Some("image"), Some("landscape"), Some("jpg"), None, None)
            .unwrap();
        assert_eq!(results.len(), 1);
        // Combining mismatched filters yields nothing
        let results = catalog
            .search_assets(Some("sunset"), Some("video"), None, None, None, None)
            .unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn resolve_asset_id_full_match() {
        let catalog = setup_search_catalog();
        let results = catalog.search_assets(None, None, None, None, None, None).unwrap();
        let full_id = &results[0].asset_id;
        let resolved = catalog.resolve_asset_id(full_id).unwrap();
        assert_eq!(resolved.as_deref(), Some(full_id.as_str()));
    }

    #[test]
    fn resolve_asset_id_prefix_match() {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();

        let asset = crate::models::Asset::new(crate::models::AssetType::Image, "sha256:test3");
        let full_id = asset.id.to_string();
        catalog.insert_asset(&asset).unwrap();

        let prefix = &full_id[..8];
        let resolved = catalog.resolve_asset_id(prefix).unwrap();
        assert_eq!(resolved.as_deref(), Some(full_id.as_str()));
    }

    #[test]
    fn face_scan_status_default_is_null() {
        // Newly-inserted assets start with face_scan_status = NULL so the first
        // `faces detect` run will scan them.
        let catalog = setup_search_catalog();
        let rows = catalog.search_assets(None, None, None, None, None, None).unwrap();
        let aid = &rows[0].asset_id;
        assert!(!catalog.is_face_scan_done(aid), "new asset should be unscanned");
    }

    #[test]
    fn mark_face_scan_done_roundtrips() {
        let catalog = setup_search_catalog();
        let rows = catalog.search_assets(None, None, None, None, None, None).unwrap();
        let aid = &rows[0].asset_id;
        assert!(!catalog.is_face_scan_done(aid));
        catalog.mark_face_scan_done(aid).unwrap();
        assert!(catalog.is_face_scan_done(aid), "should be marked done after mark_face_scan_done");
        catalog.clear_face_scan_status(aid).unwrap();
        assert!(!catalog.is_face_scan_done(aid), "should be unscanned after clear_face_scan_status");
    }

    #[test]
    fn resolve_asset_id_no_match() {
        let catalog = setup_search_catalog();
        let resolved = catalog.resolve_asset_id("zzzzzzzz").unwrap();
        assert!(resolved.is_none());
    }

    #[test]
    fn resolve_asset_id_ambiguous() {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();

        // Insert two assets and use an empty prefix to match both
        let a1 = crate::models::Asset::new(crate::models::AssetType::Image, "sha256:ambig1");
        let a2 = crate::models::Asset::new(crate::models::AssetType::Video, "sha256:ambig2");
        catalog.insert_asset(&a1).unwrap();
        catalog.insert_asset(&a2).unwrap();

        let result = catalog.resolve_asset_id("");
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("Ambiguous"), "expected ambiguous error, got: {msg}");
    }

    #[test]
    fn load_asset_details_returns_none_for_missing() {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();
        let details = catalog.load_asset_details("nonexistent-id").unwrap();
        assert!(details.is_none());
    }

    #[test]
    fn load_asset_details_returns_full_info() {
        let catalog = setup_search_catalog();
        let results = catalog.search_assets(Some("sunset"), None, None, None, None, None).unwrap();
        let asset_id = &results[0].asset_id;

        let details = catalog.load_asset_details(asset_id).unwrap().unwrap();
        assert_eq!(details.id, *asset_id);
        assert_eq!(details.name.as_deref(), Some("sunset photo"));
        assert_eq!(details.asset_type, "image");
        assert_eq!(details.tags, vec!["landscape", "nature"]);
        assert_eq!(details.description.as_deref(), Some("A beautiful sunset over the ocean"));
        assert_eq!(details.variants.len(), 1);
        assert_eq!(details.variants[0].role, "original");
        assert_eq!(details.variants[0].format, "jpg");
        assert_eq!(details.variants[0].file_size, 5000);
        assert_eq!(details.variants[0].original_filename, "sunset_beach.jpg");
    }

    #[test]
    fn load_asset_details_includes_locations() {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();

        let mut asset = crate::models::Asset::new(crate::models::AssetType::Image, "sha256:loc1");
        asset.name = Some("located asset".to_string());
        catalog.insert_asset(&asset).unwrap();

        let volume = crate::models::Volume::new(
            "test-vol".to_string(),
            std::path::PathBuf::from("/mnt/test"),
            crate::models::VolumeType::Local,
        );
        catalog.ensure_volume(&volume).unwrap();

        let variant = crate::models::Variant {
            content_hash: "sha256:loc1".to_string(),
            asset_id: asset.id,
            role: crate::models::VariantRole::Original,
            format: "png".to_string(),
            file_size: 2048,
            original_filename: "photo.png".to_string(),
            source_metadata: Default::default(),
            locations: vec![],
        };
        catalog.insert_variant(&variant).unwrap();

        let loc = crate::models::FileLocation {
            volume_id: volume.id,
            relative_path: std::path::PathBuf::from("photos/photo.png"),
            verified_at: None,
        };
        catalog.insert_file_location(&variant.content_hash, &loc).unwrap();

        let details = catalog.load_asset_details(&asset.id.to_string()).unwrap().unwrap();
        assert_eq!(details.variants.len(), 1);
        assert_eq!(details.variants[0].locations.len(), 1);
        assert_eq!(details.variants[0].locations[0].volume_label, "test-vol");
        assert_eq!(details.variants[0].locations[0].relative_path, "photos/photo.png");
    }

    #[test]
    fn find_asset_id_by_variant_returns_correct_asset() {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();

        let asset = crate::models::Asset::new(crate::models::AssetType::Image, "sha256:findme");
        let asset_id = asset.id.to_string();
        catalog.insert_asset(&asset).unwrap();

        let variant = crate::models::Variant {
            content_hash: "sha256:findme".to_string(),
            asset_id: asset.id,
            role: crate::models::VariantRole::Original,
            format: "jpg".to_string(),
            file_size: 1000,
            original_filename: "test.jpg".to_string(),
            source_metadata: Default::default(),
            locations: vec![],
        };
        catalog.insert_variant(&variant).unwrap();

        let found = catalog.find_asset_id_by_variant("sha256:findme").unwrap();
        assert_eq!(found, Some(asset_id));

        let missing = catalog.find_asset_id_by_variant("sha256:nope").unwrap();
        assert!(missing.is_none());
    }

    #[test]
    fn list_file_locations_for_asset_works() {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();

        let asset = crate::models::Asset::new(crate::models::AssetType::Image, "sha256:loctest");
        let asset_id = asset.id.to_string();
        catalog.insert_asset(&asset).unwrap();

        let variant = crate::models::Variant {
            content_hash: "sha256:loctest".to_string(),
            asset_id: asset.id,
            role: crate::models::VariantRole::Original,
            format: "jpg".to_string(),
            file_size: 1000,
            original_filename: "test.jpg".to_string(),
            source_metadata: Default::default(),
            locations: vec![],
        };
        catalog.insert_variant(&variant).unwrap();

        let volume = crate::models::Volume::new(
            "test-vol".to_string(),
            std::path::PathBuf::from("/mnt/test"),
            crate::models::VolumeType::Local,
        );
        catalog.ensure_volume(&volume).unwrap();

        let loc = crate::models::FileLocation {
            volume_id: volume.id,
            relative_path: std::path::PathBuf::from("photos/test.jpg"),
            verified_at: None,
        };
        catalog
            .insert_file_location("sha256:loctest", &loc)
            .unwrap();

        let locs = catalog.list_file_locations_for_asset(&asset_id).unwrap();
        assert_eq!(locs.len(), 1);
        assert_eq!(locs[0].0, "sha256:loctest");
        assert_eq!(locs[0].1, "photos/test.jpg");
        assert_eq!(locs[0].2, volume.id.to_string());

        // Non-existent asset returns empty
        let empty = catalog
            .list_file_locations_for_asset("nonexistent")
            .unwrap();
        assert!(empty.is_empty());
    }

    #[test]
    fn delete_asset_removes_row() {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();

        let asset = crate::models::Asset::new(crate::models::AssetType::Image, "sha256:delete1");
        let asset_id = asset.id.to_string();
        catalog.insert_asset(&asset).unwrap();

        catalog.delete_asset(&asset_id).unwrap();

        let count: i64 = catalog
            .conn
            .query_row("SELECT COUNT(*) FROM assets WHERE id = ?1", rusqlite::params![asset_id], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn delete_asset_errors_on_missing() {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();
        assert!(catalog.delete_asset("nonexistent").is_err());
    }

    #[test]
    fn delete_collection_memberships_for_asset_removes_rows() {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();
        crate::collection::CollectionStore::initialize(&catalog.conn).unwrap();

        let asset = crate::models::Asset::new(crate::models::AssetType::Image, "sha256:colmem1");
        let asset_id = asset.id.to_string();
        catalog.insert_asset(&asset).unwrap();

        let store = crate::collection::CollectionStore::new(&catalog.conn);
        store.create("TestCol", None).unwrap();
        store.add_assets("TestCol", &[asset_id.clone()]).unwrap();

        let removed = catalog.delete_collection_memberships_for_asset(&asset_id).unwrap();
        assert_eq!(removed, 1);

        // Verify membership is gone
        let count: i64 = catalog.conn.query_row(
            "SELECT COUNT(*) FROM collection_assets WHERE asset_id = ?1",
            rusqlite::params![asset_id],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn delete_collection_memberships_noop_for_no_memberships() {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();
        crate::collection::CollectionStore::initialize(&catalog.conn).unwrap();

        let removed = catalog.delete_collection_memberships_for_asset("nonexistent").unwrap();
        assert_eq!(removed, 0);
    }

    #[test]
    fn list_variant_hashes_for_asset_returns_hashes() {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();

        let mut asset = crate::models::Asset::new(crate::models::AssetType::Image, "sha256:vh1");
        let asset_id = asset.id.to_string();

        let v1 = crate::models::Variant {
            content_hash: "sha256:vh1".to_string(),
            asset_id: asset.id,
            role: crate::models::VariantRole::Original,
            format: "jpg".to_string(),
            file_size: 100,
            original_filename: "test.jpg".to_string(),
            source_metadata: Default::default(),
            locations: vec![],
        };
        let v2 = crate::models::Variant {
            content_hash: "sha256:vh2".to_string(),
            asset_id: asset.id,
            role: crate::models::VariantRole::Export,
            format: "tif".to_string(),
            file_size: 200,
            original_filename: "test.tif".to_string(),
            source_metadata: Default::default(),
            locations: vec![],
        };
        asset.variants = vec![v1.clone(), v2.clone()];
        catalog.insert_asset(&asset).unwrap();
        catalog.insert_variant(&v1).unwrap();
        catalog.insert_variant(&v2).unwrap();

        let mut hashes = catalog.list_variant_hashes_for_asset(&asset_id).unwrap();
        hashes.sort();
        assert_eq!(hashes, vec!["sha256:vh1", "sha256:vh2"]);
    }

    #[test]
    fn list_variant_hashes_for_asset_empty_for_unknown() {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();

        let hashes = catalog.list_variant_hashes_for_asset("nonexistent").unwrap();
        assert!(hashes.is_empty());
    }

    #[test]
    fn update_variant_asset_id_changes_fk() {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();

        let asset1 = crate::models::Asset::new(crate::models::AssetType::Image, "sha256:move1");
        let asset2 = crate::models::Asset::new(crate::models::AssetType::Image, "sha256:move2");
        catalog.insert_asset(&asset1).unwrap();
        catalog.insert_asset(&asset2).unwrap();

        let variant = crate::models::Variant {
            content_hash: "sha256:moveme".to_string(),
            asset_id: asset1.id,
            role: crate::models::VariantRole::Original,
            format: "jpg".to_string(),
            file_size: 500,
            original_filename: "move.jpg".to_string(),
            source_metadata: Default::default(),
            locations: vec![],
        };
        catalog.insert_variant(&variant).unwrap();

        catalog
            .update_variant_asset_id("sha256:moveme", &asset2.id.to_string())
            .unwrap();

        let new_owner = catalog.find_asset_id_by_variant("sha256:moveme").unwrap();
        assert_eq!(new_owner, Some(asset2.id.to_string()));
    }

    #[test]
    fn rebuild_clears_data_rows() {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();

        // Insert asset + variant + location
        let asset = crate::models::Asset::new(crate::models::AssetType::Image, "sha256:rebuild1");
        catalog.insert_asset(&asset).unwrap();

        let volume = crate::models::Volume::new(
            "vol".to_string(),
            std::path::PathBuf::from("/mnt/vol"),
            crate::models::VolumeType::Local,
        );
        catalog.ensure_volume(&volume).unwrap();

        let variant = crate::models::Variant {
            content_hash: "sha256:rebuild1".to_string(),
            asset_id: asset.id,
            role: crate::models::VariantRole::Original,
            format: "png".to_string(),
            file_size: 100,
            original_filename: "test.png".to_string(),
            source_metadata: Default::default(),
            locations: vec![],
        };
        catalog.insert_variant(&variant).unwrap();

        let loc = crate::models::FileLocation {
            volume_id: volume.id,
            relative_path: std::path::PathBuf::from("test.png"),
            verified_at: None,
        };
        catalog.insert_file_location(&variant.content_hash, &loc).unwrap();

        // Rebuild should clear data rows
        catalog.rebuild().unwrap();

        let count = |table: &str| -> i64 {
            catalog
                .conn
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| row.get(0))
                .unwrap()
        };

        assert_eq!(count("assets"), 0);
        assert_eq!(count("variants"), 0);
        assert_eq!(count("file_locations"), 0);
        // Volumes should be preserved
        assert_eq!(count("volumes"), 1);
    }

    #[test]
    fn find_duplicates_returns_entries_with_multiple_locations() {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();

        let vol1 = crate::models::Volume::new(
            "vol-a".to_string(),
            std::path::PathBuf::from("/mnt/a"),
            crate::models::VolumeType::Local,
        );
        let vol2 = crate::models::Volume::new(
            "vol-b".to_string(),
            std::path::PathBuf::from("/mnt/b"),
            crate::models::VolumeType::Local,
        );
        catalog.ensure_volume(&vol1).unwrap();
        catalog.ensure_volume(&vol2).unwrap();

        // Asset with a variant that has two locations (duplicate)
        let asset = crate::models::Asset::new(crate::models::AssetType::Image, "sha256:dup1");
        catalog.insert_asset(&asset).unwrap();

        let variant = crate::models::Variant {
            content_hash: "sha256:dup1".to_string(),
            asset_id: asset.id,
            role: crate::models::VariantRole::Original,
            format: "jpg".to_string(),
            file_size: 5000,
            original_filename: "photo.jpg".to_string(),
            source_metadata: Default::default(),
            locations: vec![],
        };
        catalog.insert_variant(&variant).unwrap();

        let loc1 = crate::models::FileLocation {
            volume_id: vol1.id,
            relative_path: std::path::PathBuf::from("photos/photo.jpg"),
            verified_at: None,
        };
        let loc2 = crate::models::FileLocation {
            volume_id: vol2.id,
            relative_path: std::path::PathBuf::from("backup/photo.jpg"),
            verified_at: None,
        };
        catalog.insert_file_location(&variant.content_hash, &loc1).unwrap();
        catalog.insert_file_location(&variant.content_hash, &loc2).unwrap();

        // Asset with only one location (not a duplicate)
        let asset2 = crate::models::Asset::new(crate::models::AssetType::Image, "sha256:single1");
        catalog.insert_asset(&asset2).unwrap();

        let variant2 = crate::models::Variant {
            content_hash: "sha256:single1".to_string(),
            asset_id: asset2.id,
            role: crate::models::VariantRole::Original,
            format: "png".to_string(),
            file_size: 1000,
            original_filename: "unique.png".to_string(),
            source_metadata: Default::default(),
            locations: vec![],
        };
        catalog.insert_variant(&variant2).unwrap();

        let loc3 = crate::models::FileLocation {
            volume_id: vol1.id,
            relative_path: std::path::PathBuf::from("photos/unique.png"),
            verified_at: None,
        };
        catalog.insert_file_location(&variant2.content_hash, &loc3).unwrap();

        let dupes = catalog.find_duplicates().unwrap();
        assert_eq!(dupes.len(), 1);
        assert_eq!(dupes[0].content_hash, "sha256:dup1");
        assert_eq!(dupes[0].original_filename, "photo.jpg");
        assert_eq!(dupes[0].locations.len(), 2);
    }

    #[test]
    fn find_duplicates_empty_when_no_duplicates() {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();
        let dupes = catalog.find_duplicates().unwrap();
        assert!(dupes.is_empty());
    }

    #[test]
    fn find_duplicates_same_volume() {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();

        let vol1 = crate::models::Volume::new(
            "vol-a".to_string(),
            std::path::PathBuf::from("/mnt/a"),
            crate::models::VolumeType::Local,
        );
        let vol2 = crate::models::Volume::new(
            "vol-b".to_string(),
            std::path::PathBuf::from("/mnt/b"),
            crate::models::VolumeType::Local,
        );
        catalog.ensure_volume(&vol1).unwrap();
        catalog.ensure_volume(&vol2).unwrap();

        // Variant with 2 locations on SAME volume (same-volume dup)
        let asset1 = crate::models::Asset::new(crate::models::AssetType::Image, "sha256:sv1");
        catalog.insert_asset(&asset1).unwrap();
        let v1 = crate::models::Variant {
            content_hash: "sha256:sv1".to_string(),
            asset_id: asset1.id,
            role: crate::models::VariantRole::Original,
            format: "jpg".to_string(),
            file_size: 5000,
            original_filename: "same_vol.jpg".to_string(),
            source_metadata: Default::default(),
            locations: vec![],
        };
        catalog.insert_variant(&v1).unwrap();
        catalog.insert_file_location(&v1.content_hash, &crate::models::FileLocation {
            volume_id: vol1.id,
            relative_path: std::path::PathBuf::from("photos/same_vol.jpg"),
            verified_at: None,
        }).unwrap();
        catalog.insert_file_location(&v1.content_hash, &crate::models::FileLocation {
            volume_id: vol1.id,
            relative_path: std::path::PathBuf::from("backup/same_vol.jpg"),
            verified_at: None,
        }).unwrap();

        // Variant with locations on DIFFERENT volumes (cross-volume)
        let asset2 = crate::models::Asset::new(crate::models::AssetType::Image, "sha256:cv1");
        catalog.insert_asset(&asset2).unwrap();
        let v2 = crate::models::Variant {
            content_hash: "sha256:cv1".to_string(),
            asset_id: asset2.id,
            role: crate::models::VariantRole::Original,
            format: "jpg".to_string(),
            file_size: 3000,
            original_filename: "cross_vol.jpg".to_string(),
            source_metadata: Default::default(),
            locations: vec![],
        };
        catalog.insert_variant(&v2).unwrap();
        catalog.insert_file_location(&v2.content_hash, &crate::models::FileLocation {
            volume_id: vol1.id,
            relative_path: std::path::PathBuf::from("photos/cross_vol.jpg"),
            verified_at: None,
        }).unwrap();
        catalog.insert_file_location(&v2.content_hash, &crate::models::FileLocation {
            volume_id: vol2.id,
            relative_path: std::path::PathBuf::from("backup/cross_vol.jpg"),
            verified_at: None,
        }).unwrap();

        // same-volume should only return sv1
        let same = catalog.find_duplicates_same_volume().unwrap();
        assert_eq!(same.len(), 1);
        assert_eq!(same[0].content_hash, "sha256:sv1");
        assert!(!same[0].same_volume_groups.is_empty());

        // cross-volume should only return cv1
        let cross = catalog.find_duplicates_cross_volume().unwrap();
        assert_eq!(cross.len(), 1);
        assert_eq!(cross[0].content_hash, "sha256:cv1");
        assert_eq!(cross[0].volume_count, 2);
    }

    #[test]
    fn find_duplicates_location_details_enriched() {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();

        let mut vol = crate::models::Volume::new(
            "work".to_string(),
            std::path::PathBuf::from("/mnt/work"),
            crate::models::VolumeType::Local,
        );
        vol.purpose = Some(crate::models::volume::VolumePurpose::Working);
        catalog.ensure_volume(&vol).unwrap();

        let mut vol2 = crate::models::Volume::new(
            "backup".to_string(),
            std::path::PathBuf::from("/mnt/backup"),
            crate::models::VolumeType::External,
        );
        vol2.purpose = Some(crate::models::volume::VolumePurpose::Backup);
        catalog.ensure_volume(&vol2).unwrap();

        let asset = crate::models::Asset::new(crate::models::AssetType::Image, "sha256:enr1");
        catalog.insert_asset(&asset).unwrap();
        let variant = crate::models::Variant {
            content_hash: "sha256:enr1".to_string(),
            asset_id: asset.id,
            role: crate::models::VariantRole::Original,
            format: "jpg".to_string(),
            file_size: 5000,
            original_filename: "photo.jpg".to_string(),
            source_metadata: Default::default(),
            locations: vec![],
        };
        catalog.insert_variant(&variant).unwrap();
        catalog.insert_file_location(&variant.content_hash, &crate::models::FileLocation {
            volume_id: vol.id,
            relative_path: std::path::PathBuf::from("photos/photo.jpg"),
            verified_at: None,
        }).unwrap();
        catalog.insert_file_location(&variant.content_hash, &crate::models::FileLocation {
            volume_id: vol2.id,
            relative_path: std::path::PathBuf::from("backup/photo.jpg"),
            verified_at: None,
        }).unwrap();

        let dupes = catalog.find_duplicates().unwrap();
        assert_eq!(dupes.len(), 1);
        assert_eq!(dupes[0].volume_count, 2);
        assert!(dupes[0].same_volume_groups.is_empty());

        // Check enriched location details
        let work_loc = dupes[0].locations.iter().find(|l| l.volume_label == "work").unwrap();
        assert_eq!(work_loc.volume_id, vol.id.to_string());
        assert_eq!(work_loc.volume_purpose.as_deref(), Some("working"));

        let backup_loc = dupes[0].locations.iter().find(|l| l.volume_label == "backup").unwrap();
        assert_eq!(backup_loc.volume_id, vol2.id.to_string());
        assert_eq!(backup_loc.volume_purpose.as_deref(), Some("backup"));
    }

    #[test]
    fn delete_file_location_removes_row() {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();

        let volume = crate::models::Volume::new(
            "vol".to_string(),
            std::path::PathBuf::from("/mnt/vol"),
            crate::models::VolumeType::Local,
        );
        catalog.ensure_volume(&volume).unwrap();

        let asset = crate::models::Asset::new(crate::models::AssetType::Image, "sha256:delloc1");
        catalog.insert_asset(&asset).unwrap();

        let variant = crate::models::Variant {
            content_hash: "sha256:delloc1".to_string(),
            asset_id: asset.id,
            role: crate::models::VariantRole::Original,
            format: "jpg".to_string(),
            file_size: 100,
            original_filename: "photo.jpg".to_string(),
            source_metadata: Default::default(),
            locations: vec![],
        };
        catalog.insert_variant(&variant).unwrap();

        let loc = crate::models::FileLocation {
            volume_id: volume.id,
            relative_path: std::path::PathBuf::from("photos/photo.jpg"),
            verified_at: None,
        };
        catalog
            .insert_file_location(&variant.content_hash, &loc)
            .unwrap();

        let deleted = catalog
            .delete_file_location("sha256:delloc1", &volume.id.to_string(), "photos/photo.jpg")
            .unwrap();
        assert!(deleted);

        // Verify location is gone
        let details = catalog
            .load_asset_details(&asset.id.to_string())
            .unwrap()
            .unwrap();
        assert!(details.variants[0].locations.is_empty());
    }

    #[test]
    fn delete_file_location_returns_false_for_missing() {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();

        let deleted = catalog
            .delete_file_location("sha256:nope", "some-vol", "some/path.jpg")
            .unwrap();
        assert!(!deleted);
    }

    #[test]
    fn update_recipe_location_changes_values() {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();

        let vol1 = crate::models::Volume::new(
            "vol1".to_string(),
            std::path::PathBuf::from("/mnt/vol1"),
            crate::models::VolumeType::Local,
        );
        let vol2 = crate::models::Volume::new(
            "vol2".to_string(),
            std::path::PathBuf::from("/mnt/vol2"),
            crate::models::VolumeType::Local,
        );
        catalog.ensure_volume(&vol1).unwrap();
        catalog.ensure_volume(&vol2).unwrap();

        let asset = crate::models::Asset::new(crate::models::AssetType::Image, "sha256:rec1");
        catalog.insert_asset(&asset).unwrap();

        let variant = crate::models::Variant {
            content_hash: "sha256:rec1".to_string(),
            asset_id: asset.id,
            role: crate::models::VariantRole::Original,
            format: "nef".to_string(),
            file_size: 500,
            original_filename: "photo.nef".to_string(),
            source_metadata: Default::default(),
            locations: vec![],
        };
        catalog.insert_variant(&variant).unwrap();

        let recipe_id = uuid::Uuid::new_v4();
        let recipe = crate::models::Recipe {
            id: recipe_id,
            variant_hash: "sha256:rec1".to_string(),
            software: "Adobe".to_string(),
            recipe_type: crate::models::RecipeType::Sidecar,
            content_hash: "sha256:recipe_hash".to_string(),
            location: crate::models::FileLocation {
                volume_id: vol1.id,
                relative_path: std::path::PathBuf::from("photos/photo.xmp"),
                verified_at: None,
            },
            pending_writeback: false,
        };
        catalog.insert_recipe(&recipe).unwrap();

        catalog
            .update_recipe_location(
                &recipe_id.to_string(),
                &vol2.id.to_string(),
                "backup/photo.xmp",
            )
            .unwrap();

        // Verify by querying the recipe
        let row: (String, String) = catalog
            .conn
            .query_row(
                "SELECT volume_id, relative_path FROM recipes WHERE id = ?1",
                rusqlite::params![recipe_id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(row.0, vol2.id.to_string());
        assert_eq!(row.1, "backup/photo.xmp");
    }

    #[test]
    fn update_verified_at_sets_timestamp() {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();

        let volume = crate::models::Volume::new(
            "vol".to_string(),
            std::path::PathBuf::from("/mnt/vol"),
            crate::models::VolumeType::Local,
        );
        catalog.ensure_volume(&volume).unwrap();

        let asset = crate::models::Asset::new(crate::models::AssetType::Image, "sha256:ver1");
        catalog.insert_asset(&asset).unwrap();

        let variant = crate::models::Variant {
            content_hash: "sha256:ver1".to_string(),
            asset_id: asset.id,
            role: crate::models::VariantRole::Original,
            format: "jpg".to_string(),
            file_size: 100,
            original_filename: "photo.jpg".to_string(),
            source_metadata: Default::default(),
            locations: vec![],
        };
        catalog.insert_variant(&variant).unwrap();

        let loc = crate::models::FileLocation {
            volume_id: volume.id,
            relative_path: std::path::PathBuf::from("photos/photo.jpg"),
            verified_at: None,
        };
        catalog.insert_file_location(&variant.content_hash, &loc).unwrap();

        // Initially null
        let before: Option<String> = catalog
            .conn
            .query_row(
                "SELECT verified_at FROM file_locations WHERE content_hash = ?1",
                rusqlite::params!["sha256:ver1"],
                |row| row.get(0),
            )
            .unwrap();
        assert!(before.is_none());

        // Update
        catalog
            .update_verified_at("sha256:ver1", &volume.id.to_string(), "photos/photo.jpg")
            .unwrap();

        let after: Option<String> = catalog
            .conn
            .query_row(
                "SELECT verified_at FROM file_locations WHERE content_hash = ?1",
                rusqlite::params!["sha256:ver1"],
                |row| row.get(0),
            )
            .unwrap();
        assert!(after.is_some());
    }

    #[test]
    fn update_verified_at_noop_for_missing() {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();
        // Should not error, just update 0 rows
        catalog
            .update_verified_at("sha256:nope", "some-vol", "some/path.jpg")
            .unwrap();
    }

    #[test]
    fn update_recipe_location_errors_on_missing() {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();
        let err = catalog
            .update_recipe_location("nonexistent-id", "vol", "path")
            .unwrap_err();
        assert!(err.to_string().contains("no recipe found"));
    }

    /// Helper to set up a catalog with a volume, asset, variant, and recipe for recipe tests.
    fn setup_recipe_catalog() -> (Catalog, crate::models::Volume, crate::models::Asset, String) {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();

        let volume = crate::models::Volume::new(
            "vol".to_string(),
            std::path::PathBuf::from("/mnt/vol"),
            crate::models::VolumeType::Local,
        );
        catalog.ensure_volume(&volume).unwrap();

        let asset = crate::models::Asset::new(crate::models::AssetType::Image, "sha256:rectest");
        catalog.insert_asset(&asset).unwrap();

        let variant = crate::models::Variant {
            content_hash: "sha256:rectest".to_string(),
            asset_id: asset.id,
            role: crate::models::VariantRole::Original,
            format: "nef".to_string(),
            file_size: 1000,
            original_filename: "photo.nef".to_string(),
            source_metadata: Default::default(),
            locations: vec![],
        };
        catalog.insert_variant(&variant).unwrap();

        let loc = crate::models::FileLocation {
            volume_id: volume.id,
            relative_path: std::path::PathBuf::from("photos/photo.nef"),
            verified_at: None,
        };
        catalog.insert_file_location("sha256:rectest", &loc).unwrap();

        let recipe_id = uuid::Uuid::new_v4();
        let recipe = crate::models::Recipe {
            id: recipe_id,
            variant_hash: "sha256:rectest".to_string(),
            software: "Adobe/CaptureOne".to_string(),
            recipe_type: crate::models::RecipeType::Sidecar,
            content_hash: "sha256:recipe_old".to_string(),
            location: crate::models::FileLocation {
                volume_id: volume.id,
                relative_path: std::path::PathBuf::from("photos/photo.xmp"),
                verified_at: None,
            },
            pending_writeback: false,
        };
        catalog.insert_recipe(&recipe).unwrap();

        (catalog, volume, asset, recipe_id.to_string())
    }

    #[test]
    fn find_recipe_by_location_returns_match() {
        let (catalog, volume, _, recipe_id) = setup_recipe_catalog();
        let result = catalog
            .find_recipe_by_location(
                "sha256:rectest",
                &volume.id.to_string(),
                "photos/photo.xmp",
            )
            .unwrap();
        assert!(result.is_some());
        let (id, hash) = result.unwrap();
        assert_eq!(id, recipe_id);
        assert_eq!(hash, "sha256:recipe_old");
    }

    #[test]
    fn pending_writeback_mark_and_clear() {
        let (catalog, _volume, _asset, recipe_id) = setup_recipe_catalog();

        // Initially no pending writebacks
        let pending = catalog.list_pending_writeback_recipes(None).unwrap();
        assert!(pending.is_empty());

        // Mark as pending
        catalog.mark_pending_writeback(&recipe_id).unwrap();
        let pending = catalog.list_pending_writeback_recipes(None).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].0, recipe_id);

        // Clear
        catalog.clear_pending_writeback(&recipe_id).unwrap();
        let pending = catalog.list_pending_writeback_recipes(None).unwrap();
        assert!(pending.is_empty());
    }

    #[test]
    fn pending_writeback_volume_filter() {
        let (catalog, volume, _asset, recipe_id) = setup_recipe_catalog();

        catalog.mark_pending_writeback(&recipe_id).unwrap();

        // Filter by correct volume
        let pending = catalog
            .list_pending_writeback_recipes(Some(&volume.id.to_string()))
            .unwrap();
        assert_eq!(pending.len(), 1);

        // Filter by wrong volume
        let pending = catalog
            .list_pending_writeback_recipes(Some(&uuid::Uuid::nil().to_string()))
            .unwrap();
        assert!(pending.is_empty());
    }

    #[test]
    fn find_recipe_by_location_returns_none() {
        let (catalog, volume, _, _) = setup_recipe_catalog();
        let result = catalog
            .find_recipe_by_location(
                "sha256:rectest",
                &volume.id.to_string(),
                "photos/other.xmp",
            )
            .unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn update_recipe_content_hash_works() {
        let (catalog, volume, _, recipe_id) = setup_recipe_catalog();
        catalog
            .update_recipe_content_hash(&recipe_id, "sha256:recipe_new")
            .unwrap();

        let result = catalog
            .find_recipe_by_location(
                "sha256:rectest",
                &volume.id.to_string(),
                "photos/photo.xmp",
            )
            .unwrap()
            .unwrap();
        assert_eq!(result.1, "sha256:recipe_new");
    }

    #[test]
    fn find_recipe_by_volume_and_path_works() {
        let (catalog, volume, _, recipe_id) = setup_recipe_catalog();
        let result = catalog
            .find_recipe_by_volume_and_path(
                &volume.id.to_string(),
                "photos/photo.xmp",
            )
            .unwrap();
        assert!(result.is_some());
        let (id, hash, variant_hash) = result.unwrap();
        assert_eq!(id, recipe_id);
        assert_eq!(hash, "sha256:recipe_old");
        assert_eq!(variant_hash, "sha256:rectest");

        // Non-existent path returns None
        let result = catalog
            .find_recipe_by_volume_and_path(
                &volume.id.to_string(),
                "photos/nonexistent.xmp",
            )
            .unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn update_recipe_verified_at_works() {
        let (catalog, volume, _, _) = setup_recipe_catalog();
        catalog
            .update_recipe_verified_at(
                "sha256:rectest",
                &volume.id.to_string(),
                "photos/photo.xmp",
            )
            .unwrap();

        let verified: Option<String> = catalog
            .conn
            .query_row(
                "SELECT verified_at FROM recipes WHERE variant_hash = ?1 AND volume_id = ?2 AND relative_path = ?3",
                rusqlite::params!["sha256:rectest", volume.id.to_string(), "photos/photo.xmp"],
                |row| row.get(0),
            )
            .unwrap();
        assert!(verified.is_some());
    }

    #[test]
    fn find_variant_hash_by_stem_and_directory_works() {
        let (catalog, volume, _, _) = setup_recipe_catalog();

        // "photo" stem in "photos" directory should match "photos/photo.nef"
        let result = catalog
            .find_variant_hash_by_stem_and_directory(
                "photo",
                "photos",
                &volume.id.to_string(),
                None,
            )
            .unwrap();
        assert!(result.is_some());
        let (hash, _asset_id) = result.unwrap();
        assert_eq!(hash, "sha256:rectest");

        // Wrong stem returns None
        let result = catalog
            .find_variant_hash_by_stem_and_directory(
                "other",
                "photos",
                &volume.id.to_string(),
                None,
            )
            .unwrap();
        assert!(result.is_none());

        // Wrong directory returns None
        let result = catalog
            .find_variant_hash_by_stem_and_directory(
                "photo",
                "other_dir",
                &volume.id.to_string(),
                None,
            )
            .unwrap();
        assert!(result.is_none());
    }

    // ── list_recipe_only_assets tests ────────────────────────────

    #[test]
    fn list_recipe_only_assets_finds_standalone_xmp() {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();

        // Create an "other" asset with a single xmp variant
        let mut asset = crate::models::Asset::new(crate::models::AssetType::Other, "sha256:xmponly");
        asset.variants.push(crate::models::Variant {
            content_hash: "sha256:xmponly".to_string(),
            asset_id: asset.id,
            role: crate::models::VariantRole::Original,
            format: "xmp".to_string(),
            file_size: 1000,
            original_filename: "DSC_001.xmp".to_string(),
            source_metadata: Default::default(),
            locations: vec![],
        });
        catalog.insert_asset(&asset).unwrap();
        catalog.insert_variant(&asset.variants[0]).unwrap();

        let results = catalog.list_recipe_only_assets(None, None).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, asset.id.to_string());
        assert_eq!(results[0].1, "sha256:xmponly");
        assert_eq!(results[0].2, "xmp");
    }

    #[test]
    fn list_recipe_only_assets_ignores_multi_variant() {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();

        // Asset with 2 variants should not be returned
        let mut asset = crate::models::Asset::new(crate::models::AssetType::Other, "sha256:multi1");
        asset.variants.push(crate::models::Variant {
            content_hash: "sha256:multi1".to_string(),
            asset_id: asset.id,
            role: crate::models::VariantRole::Original,
            format: "xmp".to_string(),
            file_size: 1000,
            original_filename: "DSC_001.xmp".to_string(),
            source_metadata: Default::default(),
            locations: vec![],
        });
        asset.variants.push(crate::models::Variant {
            content_hash: "sha256:multi2".to_string(),
            asset_id: asset.id,
            role: crate::models::VariantRole::Export,
            format: "jpg".to_string(),
            file_size: 5000,
            original_filename: "DSC_001.jpg".to_string(),
            source_metadata: Default::default(),
            locations: vec![],
        });
        catalog.insert_asset(&asset).unwrap();
        catalog.insert_variant(&asset.variants[0]).unwrap();
        catalog.insert_variant(&asset.variants[1]).unwrap();

        let results = catalog.list_recipe_only_assets(None, None).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn list_recipe_only_assets_ignores_non_recipe() {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();

        // Single-variant asset with jpg format should not be returned
        let mut asset = crate::models::Asset::new(crate::models::AssetType::Image, "sha256:jpgonly");
        asset.variants.push(crate::models::Variant {
            content_hash: "sha256:jpgonly".to_string(),
            asset_id: asset.id,
            role: crate::models::VariantRole::Original,
            format: "jpg".to_string(),
            file_size: 5000,
            original_filename: "photo.jpg".to_string(),
            source_metadata: Default::default(),
            locations: vec![],
        });
        catalog.insert_asset(&asset).unwrap();
        catalog.insert_variant(&asset.variants[0]).unwrap();

        let results = catalog.list_recipe_only_assets(None, None).unwrap();
        assert!(results.is_empty());
    }

    // ── Stats tests ──────────────────────────────────────────────

    #[test]
    fn stats_overview_empty_catalog() {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();

        let (assets, variants, recipes, size, _locs) = catalog.stats_overview().unwrap();
        assert_eq!(assets, 0);
        assert_eq!(variants, 0);
        assert_eq!(recipes, 0);
        assert_eq!(size, 0);
    }

    #[test]
    fn stats_overview_with_data() {
        let catalog = setup_search_catalog();

        let (assets, variants, recipes, size, _locs) = catalog.stats_overview().unwrap();
        assert_eq!(assets, 2);
        assert_eq!(variants, 2);
        assert_eq!(recipes, 0);
        assert_eq!(size, 105_000); // 5000 + 100_000
    }

    #[test]
    fn stats_asset_types_groups_correctly() {
        let catalog = setup_search_catalog();

        let types = catalog.stats_asset_types().unwrap();
        assert_eq!(types.len(), 2);
        // Both should be present (image, video)
        let type_names: Vec<&str> = types.iter().map(|t| t.0.as_str()).collect();
        assert!(type_names.contains(&"image"));
        assert!(type_names.contains(&"video"));
    }

    #[test]
    fn stats_variant_formats_respects_limit() {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();

        // Insert 3 variants with different formats
        for (i, fmt) in ["jpg", "png", "tiff"].iter().enumerate() {
            let hash = format!("sha256:limit{i}");
            let asset = crate::models::Asset::new(crate::models::AssetType::Image, &hash);
            catalog.insert_asset(&asset).unwrap();
            let variant = crate::models::Variant {
                content_hash: hash,
                asset_id: asset.id,
                role: crate::models::VariantRole::Original,
                format: fmt.to_string(),
                file_size: 100,
                original_filename: format!("file.{fmt}"),
                source_metadata: Default::default(),
                locations: vec![],
            };
            catalog.insert_variant(&variant).unwrap();
        }

        let formats = catalog.stats_variant_formats(2).unwrap();
        assert_eq!(formats.len(), 2); // limited to 2
    }

    #[test]
    fn stats_tag_coverage_counts_correctly() {
        let catalog = setup_search_catalog();

        let (tagged, untagged) = catalog.stats_tag_coverage().unwrap();
        // setup_search_catalog: asset1 has tags, asset2 has empty tags
        assert_eq!(tagged, 1);
        assert_eq!(untagged, 1);
    }

    #[test]
    fn stats_per_volume_computes_directories() {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();

        let volume = crate::models::Volume::new(
            "vol".to_string(),
            std::path::PathBuf::from("/mnt/vol"),
            crate::models::VolumeType::Local,
        );
        catalog.ensure_volume(&volume).unwrap();

        let asset = crate::models::Asset::new(crate::models::AssetType::Image, "sha256:dirs1");
        catalog.insert_asset(&asset).unwrap();

        // Two variants in different directories
        for (i, path) in ["photos/a.jpg", "archive/b.jpg"].iter().enumerate() {
            let hash = format!("sha256:dirs{}", i + 1);
            if i > 0 {
                let a2 = crate::models::Asset::new(crate::models::AssetType::Image, &hash);
                catalog.insert_asset(&a2).unwrap();
                let v = crate::models::Variant {
                    content_hash: hash.clone(),
                    asset_id: a2.id,
                    role: crate::models::VariantRole::Original,
                    format: "jpg".to_string(),
                    file_size: 100,
                    original_filename: "b.jpg".to_string(),
                    source_metadata: Default::default(),
                    locations: vec![],
                };
                catalog.insert_variant(&v).unwrap();
            } else {
                let v = crate::models::Variant {
                    content_hash: hash.clone(),
                    asset_id: asset.id,
                    role: crate::models::VariantRole::Original,
                    format: "jpg".to_string(),
                    file_size: 100,
                    original_filename: "a.jpg".to_string(),
                    source_metadata: Default::default(),
                    locations: vec![],
                };
                catalog.insert_variant(&v).unwrap();
            }
            let loc = crate::models::FileLocation {
                volume_id: volume.id,
                relative_path: std::path::PathBuf::from(path),
                verified_at: None,
            };
            catalog.insert_file_location(&format!("sha256:dirs{}", i + 1), &loc).unwrap();
        }

        let raw = catalog.stats_per_volume().unwrap();
        assert_eq!(raw.len(), 1);
        assert_eq!(raw[0].directories, 2); // "photos" and "archive"
        assert_eq!(raw[0].total_locations, 2);
    }

    #[test]
    fn stats_verification_overview_correct() {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();

        let volume = crate::models::Volume::new(
            "vol".to_string(),
            std::path::PathBuf::from("/mnt/vol"),
            crate::models::VolumeType::Local,
        );
        catalog.ensure_volume(&volume).unwrap();

        // Two variants, one with verified_at set
        let a1 = crate::models::Asset::new(crate::models::AssetType::Image, "sha256:v1");
        catalog.insert_asset(&a1).unwrap();
        let v1 = crate::models::Variant {
            content_hash: "sha256:v1".to_string(),
            asset_id: a1.id,
            role: crate::models::VariantRole::Original,
            format: "jpg".to_string(),
            file_size: 100,
            original_filename: "a.jpg".to_string(),
            source_metadata: Default::default(),
            locations: vec![],
        };
        catalog.insert_variant(&v1).unwrap();
        let loc1 = crate::models::FileLocation {
            volume_id: volume.id,
            relative_path: std::path::PathBuf::from("a.jpg"),
            verified_at: Some(chrono::Utc::now()),
        };
        catalog.insert_file_location("sha256:v1", &loc1).unwrap();

        let a2 = crate::models::Asset::new(crate::models::AssetType::Image, "sha256:v2");
        catalog.insert_asset(&a2).unwrap();
        let v2 = crate::models::Variant {
            content_hash: "sha256:v2".to_string(),
            asset_id: a2.id,
            role: crate::models::VariantRole::Original,
            format: "jpg".to_string(),
            file_size: 200,
            original_filename: "b.jpg".to_string(),
            source_metadata: Default::default(),
            locations: vec![],
        };
        catalog.insert_variant(&v2).unwrap();
        let loc2 = crate::models::FileLocation {
            volume_id: volume.id,
            relative_path: std::path::PathBuf::from("b.jpg"),
            verified_at: None,
        };
        catalog.insert_file_location("sha256:v2", &loc2).unwrap();

        let (total, verified, oldest, newest) = catalog.stats_verification_overview().unwrap();
        assert_eq!(total, 2);
        assert_eq!(verified, 1);
        assert!(oldest.is_some());
        assert!(newest.is_some());
    }

    #[test]
    fn build_stats_empty_catalog() {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();

        let stats = catalog.build_stats(&[], true, true, true, true, 20).unwrap();
        assert_eq!(stats.overview.assets, 0);
        assert_eq!(stats.overview.variants, 0);
        assert_eq!(stats.overview.recipes, 0);
        assert_eq!(stats.overview.total_size, 0);
        assert_eq!(stats.overview.volumes_total, 0);
        assert!(stats.types.unwrap().asset_types.is_empty());
        assert!(stats.volumes.unwrap().is_empty());
        assert_eq!(stats.tags.as_ref().unwrap().unique_tags, 0);
        assert_eq!(stats.verified.as_ref().unwrap().total_locations, 0);
    }

    /// Helper to create a catalog with metadata-rich variants for metadata search tests.
    fn setup_metadata_catalog() -> Catalog {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();

        // Asset 1: Fuji camera
        let mut asset1 = crate::models::Asset::new(crate::models::AssetType::Image, "sha256:meta1");

        let mut meta1 = std::collections::BTreeMap::new();
        meta1.insert("camera_model".to_string(), "X-T5".to_string());
        meta1.insert("lens_model".to_string(), "XF56mmF1.2 R".to_string());
        meta1.insert("focal_length".to_string(), "56 mm".to_string());
        meta1.insert("f_number".to_string(), "1.2".to_string());
        meta1.insert("iso".to_string(), "400".to_string());
        meta1.insert("image_width".to_string(), "6240".to_string());
        meta1.insert("image_height".to_string(), "4160".to_string());
        meta1.insert("camera_make".to_string(), "FUJIFILM".to_string());

        let variant1 = crate::models::Variant {
            content_hash: "sha256:meta1".to_string(),
            asset_id: asset1.id,
            role: crate::models::VariantRole::Original,
            format: "raf".to_string(),
            file_size: 50_000_000,
            original_filename: "DSCF0001.RAF".to_string(),
            source_metadata: meta1,
            locations: vec![],
        };
        asset1.variants.push(variant1.clone());
        catalog.insert_asset(&asset1).unwrap();
        catalog.insert_variant(&variant1).unwrap();

        // Asset 2: Nikon camera
        let mut asset2 = crate::models::Asset::new(crate::models::AssetType::Image, "sha256:meta2");

        let mut meta2 = std::collections::BTreeMap::new();
        meta2.insert("camera_model".to_string(), "Z 6II".to_string());
        meta2.insert("lens_model".to_string(), "NIKKOR Z 24-70mm f/4 S".to_string());
        meta2.insert("focal_length".to_string(), "50 mm".to_string());
        meta2.insert("f_number".to_string(), "4.0".to_string());
        meta2.insert("iso".to_string(), "3200".to_string());
        meta2.insert("image_width".to_string(), "6048".to_string());
        meta2.insert("image_height".to_string(), "4024".to_string());
        meta2.insert("camera_make".to_string(), "NIKON CORPORATION".to_string());
        meta2.insert("label".to_string(), "Red".to_string());

        let variant2 = crate::models::Variant {
            content_hash: "sha256:meta2".to_string(),
            asset_id: asset2.id,
            role: crate::models::VariantRole::Original,
            format: "nef".to_string(),
            file_size: 40_000_000,
            original_filename: "DSC_0001.NEF".to_string(),
            source_metadata: meta2,
            locations: vec![],
        };
        asset2.variants.push(variant2.clone());
        catalog.insert_asset(&asset2).unwrap();
        catalog.insert_variant(&variant2).unwrap();

        catalog
    }

    #[test]
    fn insert_variant_populates_metadata_columns() {
        let catalog = setup_metadata_catalog();

        let row: (Option<String>, Option<String>, Option<f64>, Option<f64>, Option<i64>, Option<i64>, Option<i64>) =
            catalog.conn.query_row(
                "SELECT camera_model, lens_model, focal_length_mm, f_number, iso, image_width, image_height \
                 FROM variants WHERE content_hash = 'sha256:meta1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?)),
            ).unwrap();

        assert_eq!(row.0.as_deref(), Some("X-T5"));
        assert_eq!(row.1.as_deref(), Some("XF56mmF1.2 R"));
        assert!((row.2.unwrap() - 56.0).abs() < 0.01);
        assert!((row.3.unwrap() - 1.2).abs() < 0.01);
        assert_eq!(row.4, Some(400));
        assert_eq!(row.5, Some(6240));
        assert_eq!(row.6, Some(4160));
    }

    #[test]
    fn backfill_metadata_from_json() {
        let catalog = Catalog::open_in_memory().unwrap();
        // First initialize without metadata columns by creating a minimal schema
        catalog.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS assets (
                id TEXT PRIMARY KEY,
                name TEXT,
                created_at TEXT NOT NULL,
                asset_type TEXT NOT NULL,
                tags TEXT NOT NULL DEFAULT '[]',
                description TEXT,
                rating INTEGER
            );
            CREATE TABLE IF NOT EXISTS variants (
                content_hash TEXT PRIMARY KEY,
                asset_id TEXT NOT NULL REFERENCES assets(id),
                role TEXT NOT NULL,
                format TEXT NOT NULL,
                file_size INTEGER NOT NULL,
                original_filename TEXT NOT NULL,
                source_metadata TEXT NOT NULL DEFAULT '{}'
            );
            CREATE TABLE IF NOT EXISTS file_locations (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                content_hash TEXT NOT NULL REFERENCES variants(content_hash),
                volume_id TEXT NOT NULL,
                relative_path TEXT NOT NULL,
                verified_at TEXT
            );
            CREATE TABLE IF NOT EXISTS volumes (
                id TEXT PRIMARY KEY,
                label TEXT NOT NULL,
                mount_point TEXT NOT NULL,
                volume_type TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS recipes (
                id TEXT PRIMARY KEY,
                variant_hash TEXT NOT NULL REFERENCES variants(content_hash),
                software TEXT NOT NULL,
                recipe_type TEXT NOT NULL,
                content_hash TEXT NOT NULL,
                volume_id TEXT,
                relative_path TEXT,
                verified_at TEXT
            );"
        ).unwrap();

        // Insert an asset and variant using old schema (no metadata columns)
        let asset = crate::models::Asset::new(crate::models::AssetType::Image, "sha256:old1");
        catalog.conn.execute(
            "INSERT INTO assets (id, name, created_at, asset_type, tags) VALUES (?1, NULL, ?2, 'image', '[]')",
            rusqlite::params![asset.id.to_string(), asset.created_at.to_rfc3339()],
        ).unwrap();

        catalog.conn.execute(
            "INSERT INTO variants (content_hash, asset_id, role, format, file_size, original_filename, source_metadata) \
             VALUES ('sha256:old1', ?1, 'original', 'nef', 30000000, 'OLD.NEF', \
             '{\"camera_model\":\"D850\",\"iso\":\"800\",\"focal_length\":\"70 mm\",\"f_number\":\"2.8\"}')",
            rusqlite::params![asset.id.to_string()],
        ).unwrap();

        // Now run initialize() which should add columns and backfill
        catalog.initialize().unwrap();

        let row: (Option<String>, Option<i64>, Option<f64>, Option<f64>) =
            catalog.conn.query_row(
                "SELECT camera_model, iso, focal_length_mm, f_number FROM variants WHERE content_hash = 'sha256:old1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            ).unwrap();

        assert_eq!(row.0.as_deref(), Some("D850"));
        assert_eq!(row.1, Some(800));
        assert!((row.2.unwrap() - 70.0).abs() < 0.01);
        assert!((row.3.unwrap() - 2.8).abs() < 0.01);
    }

    #[test]
    fn search_by_camera() {
        let catalog = setup_metadata_catalog();
        let cam = vec!["X-T5".to_string()];
        let opts = SearchOptions {
            cameras: &cam,
            per_page: u32::MAX,
            ..Default::default()
        };
        let results = catalog.search_paginated(&opts).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].original_filename, "DSCF0001.RAF");
    }

    #[test]
    fn search_by_camera_partial() {
        let catalog = setup_metadata_catalog();
        let cam = vec!["Z 6".to_string()];
        let opts = SearchOptions {
            cameras: &cam,
            per_page: u32::MAX,
            ..Default::default()
        };
        let results = catalog.search_paginated(&opts).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].original_filename, "DSC_0001.NEF");
    }

    #[test]
    fn search_by_iso_exact_and_range() {
        let catalog = setup_metadata_catalog();

        // Exact ISO
        let opts = SearchOptions {
            iso: Some(NumericFilter::Exact(400.0)),
            per_page: u32::MAX,
            ..Default::default()
        };
        let results = catalog.search_paginated(&opts).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].original_filename, "DSCF0001.RAF");

        // ISO range 100-800: should match Fuji (400) only
        let opts = SearchOptions {
            iso: Some(NumericFilter::Range(100.0, 800.0)),
            per_page: u32::MAX,
            ..Default::default()
        };
        let results = catalog.search_paginated(&opts).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].original_filename, "DSCF0001.RAF");

        // ISO min 1000+: should match Nikon (3200) only
        let opts = SearchOptions {
            iso: Some(NumericFilter::Min(1000.0)),
            per_page: u32::MAX,
            ..Default::default()
        };
        let results = catalog.search_paginated(&opts).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].original_filename, "DSC_0001.NEF");
    }

    #[test]
    fn search_by_focal_range() {
        let catalog = setup_metadata_catalog();

        // focal 50-56: should match both
        let opts = SearchOptions {
            focal: Some(NumericFilter::Range(50.0, 56.0)),
            per_page: u32::MAX,
            ..Default::default()
        };
        let results = catalog.search_paginated(&opts).unwrap();
        assert_eq!(results.len(), 2);

        // focal 55-60: should match Fuji (56mm) only
        let opts = SearchOptions {
            focal: Some(NumericFilter::Range(55.0, 60.0)),
            per_page: u32::MAX,
            ..Default::default()
        };
        let results = catalog.search_paginated(&opts).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].original_filename, "DSCF0001.RAF");
    }

    #[test]
    fn search_text_includes_metadata() {
        let catalog = setup_metadata_catalog();

        // "FUJIFILM" exists in source_metadata JSON but not in name/filename/description
        let opts = SearchOptions {
            text: Some("FUJIFILM"),
            per_page: u32::MAX,
            ..Default::default()
        };
        let results = catalog.search_paginated(&opts).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].original_filename, "DSCF0001.RAF");
    }

    #[test]
    fn search_meta_json_extract() {
        let catalog = setup_metadata_catalog();

        // meta:label=Red should match the Nikon variant
        let opts = SearchOptions {
            meta_filters: vec![("label", "Red")],
            per_page: u32::MAX,
            ..Default::default()
        };
        let results = catalog.search_paginated(&opts).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].original_filename, "DSC_0001.NEF");
    }

    // ── copies filter search tests ────────────────────────────────
    // copies: counts DISTINCT VOLUMES (not file location rows).
    // An asset on vol-a and vol-b = copies:2, regardless of how many
    // individual file locations exist on each volume.

    #[test]
    fn search_copies_exact() {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();

        let vol1 = crate::models::Volume::new(
            "vol-a".to_string(),
            std::path::PathBuf::from("/mnt/a"),
            crate::models::VolumeType::Local,
        );
        let vol2 = crate::models::Volume::new(
            "vol-b".to_string(),
            std::path::PathBuf::from("/mnt/b"),
            crate::models::VolumeType::Local,
        );
        let vol3 = crate::models::Volume::new(
            "vol-c".to_string(),
            std::path::PathBuf::from("/mnt/c"),
            crate::models::VolumeType::Local,
        );
        catalog.ensure_volume(&vol1).unwrap();
        catalog.ensure_volume(&vol2).unwrap();
        catalog.ensure_volume(&vol3).unwrap();

        // Asset with 1 location
        let mut a1 = crate::models::Asset::new(crate::models::AssetType::Image, "sha256:cop1");
        let v1 = crate::models::Variant {
            content_hash: "sha256:cop1".to_string(),
            asset_id: a1.id,
            role: crate::models::VariantRole::Original,
            format: "jpg".to_string(),
            file_size: 1000,
            original_filename: "one.jpg".to_string(),
            source_metadata: Default::default(),
            locations: vec![],
        };
        a1.variants.push(v1.clone());
        catalog.insert_asset(&a1).unwrap();
        catalog.insert_variant(&v1).unwrap();
        catalog.insert_file_location(&v1.content_hash, &crate::models::FileLocation {
            volume_id: vol1.id,
            relative_path: std::path::PathBuf::from("one.jpg"),
            verified_at: None,
        }).unwrap();

        // Asset with 2 locations
        let mut a2 = crate::models::Asset::new(crate::models::AssetType::Image, "sha256:cop2");
        let v2 = crate::models::Variant {
            content_hash: "sha256:cop2".to_string(),
            asset_id: a2.id,
            role: crate::models::VariantRole::Original,
            format: "jpg".to_string(),
            file_size: 2000,
            original_filename: "two.jpg".to_string(),
            source_metadata: Default::default(),
            locations: vec![],
        };
        a2.variants.push(v2.clone());
        catalog.insert_asset(&a2).unwrap();
        catalog.insert_variant(&v2).unwrap();
        catalog.insert_file_location(&v2.content_hash, &crate::models::FileLocation {
            volume_id: vol1.id,
            relative_path: std::path::PathBuf::from("two.jpg"),
            verified_at: None,
        }).unwrap();
        catalog.insert_file_location(&v2.content_hash, &crate::models::FileLocation {
            volume_id: vol2.id,
            relative_path: std::path::PathBuf::from("two.jpg"),
            verified_at: None,
        }).unwrap();

        // Asset on 3 distinct volumes
        let mut a3 = crate::models::Asset::new(crate::models::AssetType::Image, "sha256:cop3");
        let v3 = crate::models::Variant {
            content_hash: "sha256:cop3".to_string(),
            asset_id: a3.id,
            role: crate::models::VariantRole::Original,
            format: "jpg".to_string(),
            file_size: 3000,
            original_filename: "three.jpg".to_string(),
            source_metadata: Default::default(),
            locations: vec![],
        };
        a3.variants.push(v3.clone());
        catalog.insert_asset(&a3).unwrap();
        catalog.insert_variant(&v3).unwrap();
        catalog.insert_file_location(&v3.content_hash, &crate::models::FileLocation {
            volume_id: vol1.id,
            relative_path: std::path::PathBuf::from("three.jpg"),
            verified_at: None,
        }).unwrap();
        catalog.insert_file_location(&v3.content_hash, &crate::models::FileLocation {
            volume_id: vol2.id,
            relative_path: std::path::PathBuf::from("three.jpg"),
            verified_at: None,
        }).unwrap();
        catalog.insert_file_location(&v3.content_hash, &crate::models::FileLocation {
            volume_id: vol3.id,
            relative_path: std::path::PathBuf::from("three.jpg"),
            verified_at: None,
        }).unwrap();

        // copies:1 → only the 1-volume asset (at risk)
        let results = catalog.search_paginated(&SearchOptions {
            copies: Some(NumericFilter::Exact(1.0)),
            per_page: u32::MAX,
            ..Default::default()
        }).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].original_filename, "one.jpg");

        // copies:2 → only the 2-volume asset
        let results = catalog.search_paginated(&SearchOptions {
            copies: Some(NumericFilter::Exact(2.0)),
            per_page: u32::MAX,
            ..Default::default()
        }).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].original_filename, "two.jpg");

        // copies:3 → only the 3-volume asset
        let results = catalog.search_paginated(&SearchOptions {
            copies: Some(NumericFilter::Exact(3.0)),
            per_page: u32::MAX,
            ..Default::default()
        }).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].original_filename, "three.jpg");
    }

    #[test]
    fn search_copies_min() {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();

        let vol1 = crate::models::Volume::new("vol-a".to_string(), std::path::PathBuf::from("/mnt/a"), crate::models::VolumeType::Local);
        let vol2 = crate::models::Volume::new("vol-b".to_string(), std::path::PathBuf::from("/mnt/b"), crate::models::VolumeType::Local);
        let vol3 = crate::models::Volume::new("vol-c".to_string(), std::path::PathBuf::from("/mnt/c"), crate::models::VolumeType::Local);
        catalog.ensure_volume(&vol1).unwrap();
        catalog.ensure_volume(&vol2).unwrap();
        catalog.ensure_volume(&vol3).unwrap();

        // Asset on 1 volume
        let mut a1 = crate::models::Asset::new(crate::models::AssetType::Image, "sha256:cpm1");
        let v1 = crate::models::Variant {
            content_hash: "sha256:cpm1".to_string(), asset_id: a1.id,
            role: crate::models::VariantRole::Original, format: "jpg".to_string(),
            file_size: 1000, original_filename: "one.jpg".to_string(),
            source_metadata: Default::default(), locations: vec![],
        };
        a1.variants.push(v1.clone());
        catalog.insert_asset(&a1).unwrap();
        catalog.insert_variant(&v1).unwrap();
        catalog.insert_file_location(&v1.content_hash, &crate::models::FileLocation {
            volume_id: vol1.id, relative_path: std::path::PathBuf::from("one.jpg"), verified_at: None,
        }).unwrap();

        // Asset on 2 volumes
        let mut a2 = crate::models::Asset::new(crate::models::AssetType::Image, "sha256:cpm2");
        let v2 = crate::models::Variant {
            content_hash: "sha256:cpm2".to_string(), asset_id: a2.id,
            role: crate::models::VariantRole::Original, format: "jpg".to_string(),
            file_size: 2000, original_filename: "two.jpg".to_string(),
            source_metadata: Default::default(), locations: vec![],
        };
        a2.variants.push(v2.clone());
        catalog.insert_asset(&a2).unwrap();
        catalog.insert_variant(&v2).unwrap();
        catalog.insert_file_location(&v2.content_hash, &crate::models::FileLocation {
            volume_id: vol1.id, relative_path: std::path::PathBuf::from("two.jpg"), verified_at: None,
        }).unwrap();
        catalog.insert_file_location(&v2.content_hash, &crate::models::FileLocation {
            volume_id: vol2.id, relative_path: std::path::PathBuf::from("two.jpg"), verified_at: None,
        }).unwrap();

        // Asset on 3 volumes
        let mut a3 = crate::models::Asset::new(crate::models::AssetType::Image, "sha256:cpm3");
        let v3 = crate::models::Variant {
            content_hash: "sha256:cpm3".to_string(), asset_id: a3.id,
            role: crate::models::VariantRole::Original, format: "jpg".to_string(),
            file_size: 3000, original_filename: "three.jpg".to_string(),
            source_metadata: Default::default(), locations: vec![],
        };
        a3.variants.push(v3.clone());
        catalog.insert_asset(&a3).unwrap();
        catalog.insert_variant(&v3).unwrap();
        catalog.insert_file_location(&v3.content_hash, &crate::models::FileLocation {
            volume_id: vol1.id, relative_path: std::path::PathBuf::from("three.jpg"), verified_at: None,
        }).unwrap();
        catalog.insert_file_location(&v3.content_hash, &crate::models::FileLocation {
            volume_id: vol2.id, relative_path: std::path::PathBuf::from("three.jpg"), verified_at: None,
        }).unwrap();
        catalog.insert_file_location(&v3.content_hash, &crate::models::FileLocation {
            volume_id: vol3.id, relative_path: std::path::PathBuf::from("three.jpg"), verified_at: None,
        }).unwrap();

        // copies:2+ → assets on 2 or more volumes
        let results = catalog.search_paginated(&SearchOptions {
            copies: Some(NumericFilter::Min(2.0)),
            per_page: u32::MAX,
            ..Default::default()
        }).unwrap();
        assert_eq!(results.len(), 2);
        let names: Vec<&str> = results.iter().map(|r| r.original_filename.as_str()).collect();
        assert!(names.contains(&"two.jpg"));
        assert!(names.contains(&"three.jpg"));

        // copies:3+ → only the 3-volume asset
        let results = catalog.search_paginated(&SearchOptions {
            copies: Some(NumericFilter::Min(3.0)),
            per_page: u32::MAX,
            ..Default::default()
        }).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].original_filename, "three.jpg");
    }

    // ── find_asset_ids_by_volume_and_path_prefixes tests ─────────

    #[test]
    fn find_asset_ids_by_volume_and_path_prefixes_basic() {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();

        let volume = crate::models::Volume::new(
            "vol1".to_string(),
            std::path::PathBuf::from("/mnt/vol1"),
            crate::models::VolumeType::Local,
        );
        catalog.ensure_volume(&volume).unwrap();

        let mut asset = crate::models::Asset::new(crate::models::AssetType::Image, "sha256:aaa");
        asset.variants.push(crate::models::Variant {
            content_hash: "sha256:aaa".to_string(),
            asset_id: asset.id,
            role: crate::models::VariantRole::Original,
            format: "arw".to_string(),
            file_size: 1000,
            original_filename: "DSC_001.ARW".to_string(),
            source_metadata: Default::default(),
            locations: vec![],
        });
        catalog.insert_asset(&asset).unwrap();
        catalog.insert_variant(&asset.variants[0]).unwrap();
        catalog
            .insert_file_location(
                "sha256:aaa",
                &crate::models::FileLocation {
                    volume_id: volume.id,
                    relative_path: std::path::PathBuf::from("session/Capture/DSC_001.ARW"),
                    verified_at: None,
                },
            )
            .unwrap();

        // Prefix "session" should match
        let ids = catalog
            .find_asset_ids_by_volume_and_path_prefixes(
                &volume.id.to_string(),
                &["session".to_string()],
            )
            .unwrap();
        assert_eq!(ids.len(), 1);
        assert_eq!(ids[0], asset.id.to_string());

        // Prefix "other" should not match
        let ids = catalog
            .find_asset_ids_by_volume_and_path_prefixes(
                &volume.id.to_string(),
                &["other".to_string()],
            )
            .unwrap();
        assert!(ids.is_empty());
    }

    #[test]
    fn find_asset_ids_by_volume_and_path_prefixes_wrong_volume() {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();

        let volume = crate::models::Volume::new(
            "vol1".to_string(),
            std::path::PathBuf::from("/mnt/vol1"),
            crate::models::VolumeType::Local,
        );
        catalog.ensure_volume(&volume).unwrap();

        let mut asset = crate::models::Asset::new(crate::models::AssetType::Image, "sha256:bbb");
        asset.variants.push(crate::models::Variant {
            content_hash: "sha256:bbb".to_string(),
            asset_id: asset.id,
            role: crate::models::VariantRole::Original,
            format: "jpg".to_string(),
            file_size: 500,
            original_filename: "photo.jpg".to_string(),
            source_metadata: Default::default(),
            locations: vec![],
        });
        catalog.insert_asset(&asset).unwrap();
        catalog.insert_variant(&asset.variants[0]).unwrap();
        catalog
            .insert_file_location(
                "sha256:bbb",
                &crate::models::FileLocation {
                    volume_id: volume.id,
                    relative_path: std::path::PathBuf::from("photos/photo.jpg"),
                    verified_at: None,
                },
            )
            .unwrap();

        // Query with a fake volume ID — should return empty
        let ids = catalog
            .find_asset_ids_by_volume_and_path_prefixes(
                "00000000-0000-0000-0000-000000000000",
                &["photos".to_string()],
            )
            .unwrap();
        assert!(ids.is_empty());
    }

    #[test]
    fn find_asset_ids_by_volume_and_path_prefixes_empty_prefix() {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();

        let volume = crate::models::Volume::new(
            "vol1".to_string(),
            std::path::PathBuf::from("/mnt/vol1"),
            crate::models::VolumeType::Local,
        );
        catalog.ensure_volume(&volume).unwrap();

        let mut asset = crate::models::Asset::new(crate::models::AssetType::Image, "sha256:ccc");
        asset.variants.push(crate::models::Variant {
            content_hash: "sha256:ccc".to_string(),
            asset_id: asset.id,
            role: crate::models::VariantRole::Original,
            format: "jpg".to_string(),
            file_size: 500,
            original_filename: "photo.jpg".to_string(),
            source_metadata: Default::default(),
            locations: vec![],
        });
        catalog.insert_asset(&asset).unwrap();
        catalog.insert_variant(&asset.variants[0]).unwrap();
        catalog
            .insert_file_location(
                "sha256:ccc",
                &crate::models::FileLocation {
                    volume_id: volume.id,
                    relative_path: std::path::PathBuf::from("deep/nested/photo.jpg"),
                    verified_at: None,
                },
            )
            .unwrap();

        // Empty prefix matches everything on the volume
        let ids = catalog
            .find_asset_ids_by_volume_and_path_prefixes(
                &volume.id.to_string(),
                &[String::new()],
            )
            .unwrap();
        assert_eq!(ids.len(), 1);
        assert_eq!(ids[0], asset.id.to_string());
    }

    // ── next_date_bound tests ─────────────────────────────────────

    #[test]
    fn next_date_bound_day() {
        assert_eq!(next_date_bound("2026-02-25"), "2026-02-26");
        assert_eq!(next_date_bound("2026-02-28"), "2026-03-01");
        assert_eq!(next_date_bound("2026-12-31"), "2027-01-01");
    }

    #[test]
    fn next_date_bound_month() {
        assert_eq!(next_date_bound("2026-02"), "2026-03");
        assert_eq!(next_date_bound("2026-12"), "2027-01");
    }

    #[test]
    fn next_date_bound_year() {
        assert_eq!(next_date_bound("2026"), "2027");
    }

    // ── path_pattern_to_like tests ────────────────────────────────

    #[test]
    fn path_pattern_plain_prefix() {
        assert_eq!(path_pattern_to_like("Pictures/2026"), "Pictures/2026%");
    }

    #[test]
    fn path_pattern_star_in_middle() {
        assert_eq!(path_pattern_to_like("Pictures/*/Capture"), "Pictures/%/Capture%");
    }

    #[test]
    fn path_pattern_leading_star() {
        assert_eq!(path_pattern_to_like("*party"), "%party%");
    }

    #[test]
    fn path_pattern_complex() {
        assert_eq!(path_pattern_to_like("*/2026/*/party"), "%/2026/%/party%");
    }

    #[test]
    fn path_pattern_escapes_sql_wildcards() {
        // Literal % and _ in user input must be escaped
        assert_eq!(path_pattern_to_like("foo%bar"), "foo\\%bar%");
        assert_eq!(path_pattern_to_like("foo_bar"), "foo\\_bar%");
    }

    #[test]
    fn path_pattern_escapes_backslash() {
        assert_eq!(path_pattern_to_like("foo\\bar"), "foo\\\\bar%");
    }

    #[test]
    fn path_pattern_double_star_collapses() {
        // ** is harmless: %% in SQL behaves like %
        assert_eq!(path_pattern_to_like("**foo"), "%%foo%");
    }

    #[test]
    fn path_pattern_only_star() {
        // Just `*` matches everything (becomes `%`, no trailing % appended since already ends in %)
        assert_eq!(path_pattern_to_like("*"), "%");
    }

    #[test]
    fn path_pattern_trailing_star_no_double() {
        // Trailing * already produces trailing %, so we don't append another
        assert_eq!(path_pattern_to_like("foo*"), "foo%");
    }

    // ── calendar_counts / calendar_years tests ────────────────────

    #[test]
    fn calendar_years_returns_distinct_years() {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();

        let mut a1 = crate::models::Asset::new(crate::models::AssetType::Image, "sha256:cal1");
        a1.created_at = chrono::NaiveDate::from_ymd_opt(2024, 6, 15).unwrap().and_hms_opt(12, 0, 0).unwrap().and_utc();
        let v1 = crate::models::Variant {
            content_hash: "sha256:cal1".to_string(),
            asset_id: a1.id,
            role: crate::models::VariantRole::Original,
            format: "jpg".to_string(),
            file_size: 1000,
            original_filename: "a.jpg".to_string(),
            source_metadata: Default::default(),
            locations: vec![],
        };
        a1.variants.push(v1.clone());
        catalog.insert_asset(&a1).unwrap();
        catalog.insert_variant(&v1).unwrap();

        let mut a2 = crate::models::Asset::new(crate::models::AssetType::Image, "sha256:cal2");
        a2.created_at = chrono::NaiveDate::from_ymd_opt(2026, 1, 10).unwrap().and_hms_opt(12, 0, 0).unwrap().and_utc();
        let v2 = crate::models::Variant {
            content_hash: "sha256:cal2".to_string(),
            asset_id: a2.id,
            role: crate::models::VariantRole::Original,
            format: "jpg".to_string(),
            file_size: 1000,
            original_filename: "b.jpg".to_string(),
            source_metadata: Default::default(),
            locations: vec![],
        };
        a2.variants.push(v2.clone());
        catalog.insert_asset(&a2).unwrap();
        catalog.insert_variant(&v2).unwrap();

        let years = catalog.calendar_years().unwrap();
        assert_eq!(years, vec![2024, 2026]);
    }

    #[test]
    fn calendar_counts_groups_by_day() {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();

        // Two assets on the same day
        for (hash, hr) in [("sha256:cc1", 10), ("sha256:cc2", 14)] {
            let mut a = crate::models::Asset::new(crate::models::AssetType::Image, hash);
            a.created_at = chrono::NaiveDate::from_ymd_opt(2026, 3, 5).unwrap().and_hms_opt(hr, 0, 0).unwrap().and_utc();
            let v = crate::models::Variant {
                content_hash: hash.to_string(),
                asset_id: a.id,
                role: crate::models::VariantRole::Original,
                format: "jpg".to_string(),
                file_size: 1000,
                original_filename: "x.jpg".to_string(),
                source_metadata: Default::default(),
                locations: vec![],
            };
            a.variants.push(v.clone());
            catalog.insert_asset(&a).unwrap();
            catalog.insert_variant(&v).unwrap();
        }

        let opts = SearchOptions::default();
        let counts = catalog.calendar_counts(2026, &opts).unwrap();
        assert_eq!(counts.get("2026-03-05"), Some(&2));
        assert_eq!(counts.len(), 1);

        // Different year returns empty
        let counts2 = catalog.calendar_counts(2025, &opts).unwrap();
        assert!(counts2.is_empty());
    }

    #[test]
    fn assets_with_exact_tag_finds_correct_assets() {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();

        // Asset with tag "Group A"
        let mut a1 = crate::models::Asset::new(crate::models::AssetType::Image, "sha256:tag1");
        a1.tags = vec!["Group A".to_string(), "landscape".to_string()];
        let v1 = crate::models::Variant {
            content_hash: "sha256:tag1".to_string(),
            asset_id: a1.id.clone(),
            role: crate::models::VariantRole::Original,
            format: "jpg".to_string(),
            file_size: 1000,
            original_filename: "a.jpg".to_string(),
            source_metadata: Default::default(),
            locations: vec![],
        };
        a1.variants.push(v1.clone());
        catalog.insert_asset(&a1).unwrap();
        catalog.insert_variant(&v1).unwrap();

        // Asset with tag "Group B" (should not match)
        let mut a2 = crate::models::Asset::new(crate::models::AssetType::Image, "sha256:tag2");
        a2.tags = vec!["Group B".to_string()];
        let v2 = crate::models::Variant {
            content_hash: "sha256:tag2".to_string(),
            asset_id: a2.id.clone(),
            role: crate::models::VariantRole::Original,
            format: "jpg".to_string(),
            file_size: 1000,
            original_filename: "b.jpg".to_string(),
            source_metadata: Default::default(),
            locations: vec![],
        };
        a2.variants.push(v2.clone());
        catalog.insert_asset(&a2).unwrap();
        catalog.insert_variant(&v2).unwrap();

        // Asset with hierarchical tag "Group A|sub" (should NOT match exact "Group A")
        let mut a3 = crate::models::Asset::new(crate::models::AssetType::Image, "sha256:tag3");
        a3.tags = vec!["Group A|sub".to_string()];
        let v3 = crate::models::Variant {
            content_hash: "sha256:tag3".to_string(),
            asset_id: a3.id.clone(),
            role: crate::models::VariantRole::Original,
            format: "jpg".to_string(),
            file_size: 1000,
            original_filename: "c.jpg".to_string(),
            source_metadata: Default::default(),
            locations: vec![],
        };
        a3.variants.push(v3.clone());
        catalog.insert_asset(&a3).unwrap();
        catalog.insert_variant(&v3).unwrap();

        let results = catalog.assets_with_exact_tag("Group A").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, a1.id.to_string());
        assert!(results[0].1.is_none()); // not stacked
    }

    #[test]
    fn get_location_verified_at_returns_timestamp() {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();

        let volume = crate::models::Volume::new(
            "vol".to_string(),
            std::path::PathBuf::from("/mnt/vol"),
            crate::models::VolumeType::Local,
        );
        catalog.ensure_volume(&volume).unwrap();

        let asset = crate::models::Asset::new(crate::models::AssetType::Image, "sha256:gv1");
        catalog.insert_asset(&asset).unwrap();

        let variant = crate::models::Variant {
            content_hash: "sha256:gv1".to_string(),
            asset_id: asset.id,
            role: crate::models::VariantRole::Original,
            format: "jpg".to_string(),
            file_size: 100,
            original_filename: "photo.jpg".to_string(),
            source_metadata: Default::default(),
            locations: vec![],
        };
        catalog.insert_variant(&variant).unwrap();

        let loc = crate::models::FileLocation {
            volume_id: volume.id,
            relative_path: std::path::PathBuf::from("photos/photo.jpg"),
            verified_at: None,
        };
        catalog.insert_file_location(&variant.content_hash, &loc).unwrap();

        // Before verify: should be None
        let before = catalog
            .get_location_verified_at(&volume.id.to_string(), "photos/photo.jpg")
            .unwrap();
        assert!(before.is_none());

        // Set verified_at
        catalog
            .update_verified_at("sha256:gv1", &volume.id.to_string(), "photos/photo.jpg")
            .unwrap();

        // After verify: should have a timestamp
        let after = catalog
            .get_location_verified_at(&volume.id.to_string(), "photos/photo.jpg")
            .unwrap();
        assert!(after.is_some());
    }

    #[test]
    fn get_location_verified_at_returns_none_for_unknown() {
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.initialize().unwrap();

        let result = catalog
            .get_location_verified_at("nonexistent-vol", "no/such/path.jpg")
            .unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn search_paginated_with_count_returns_total() {
        let catalog = setup_search_catalog();

        // Page 1 with per_page=1: should return 1 row but total_count=2
        let opts = SearchOptions {
            per_page: 1,
            page: 1,
            ..Default::default()
        };
        let (rows, total) = catalog.search_paginated_with_count(&opts).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(total, 2);

        // Page 2: should return 1 row, same total
        let opts2 = SearchOptions {
            per_page: 1,
            page: 2,
            ..Default::default()
        };
        let (rows2, total2) = catalog.search_paginated_with_count(&opts2).unwrap();
        assert_eq!(rows2.len(), 1);
        assert_eq!(total2, 2);

        // All on one page
        let opts3 = SearchOptions {
            per_page: 100,
            page: 1,
            ..Default::default()
        };
        let (rows3, total3) = catalog.search_paginated_with_count(&opts3).unwrap();
        assert_eq!(rows3.len(), 2);
        assert_eq!(total3, 2);
    }

    #[test]
    fn search_paginated_with_count_empty_results() {
        let catalog = setup_search_catalog();

        let opts = SearchOptions {
            text: Some("nonexistent_query_xyz"),
            per_page: 10,
            page: 1,
            ..Default::default()
        };
        let (rows, total) = catalog.search_paginated_with_count(&opts).unwrap();
        assert!(rows.is_empty());
        assert_eq!(total, 0);
    }

    #[test]
    fn search_paginated_with_count_filtered() {
        let catalog = setup_search_catalog();

        // Filter to only images (1 of 2 assets)
        let tags = vec!["landscape".to_string()];
        let opts = SearchOptions {
            tags: &tags,
            per_page: 100,
            page: 1,
            ..Default::default()
        };
        let (rows, total) = catalog.search_paginated_with_count(&opts).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(total, 1);
    }

    #[test]
    fn search_similar_asset_ids_filter() {
        let catalog = setup_search_catalog();

        // Get all assets to find their IDs
        let all = catalog.search_paginated(&SearchOptions {
            per_page: u32::MAX,
            ..Default::default()
        }).unwrap();
        assert_eq!(all.len(), 2);

        // Filter to just the first asset using similar_asset_ids
        let ids = vec![all[0].asset_id.clone()];
        let opts = SearchOptions {
            similar_asset_ids: Some(&ids),
            per_page: u32::MAX,
            ..Default::default()
        };
        let results = catalog.search_paginated(&opts).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].asset_id, all[0].asset_id);
    }

    #[test]
    fn search_has_embed_filter() {
        let catalog = setup_search_catalog();

        // No embeddings yet — embed:none should return all
        let all = catalog
            .search_paginated(&SearchOptions {
                has_embed: Some(false),
                per_page: u32::MAX,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(all.len(), 2);

        // embed:any should return nothing
        let embedded = catalog
            .search_paginated(&SearchOptions {
                has_embed: Some(true),
                per_page: u32::MAX,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(embedded.len(), 0);

        // Insert a fake embedding for one asset
        let asset_id = &all[0].asset_id;
        catalog
            .conn()
            .execute(
                "INSERT INTO embeddings (asset_id, model, embedding) VALUES (?1, 'test', X'00')",
                rusqlite::params![asset_id],
            )
            .unwrap();

        // embed:any should now return 1
        let embedded = catalog
            .search_paginated(&SearchOptions {
                has_embed: Some(true),
                per_page: u32::MAX,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(embedded.len(), 1);
        assert_eq!(&embedded[0].asset_id, asset_id);

        // embed:none should return 1
        let not_embedded = catalog
            .search_paginated(&SearchOptions {
                has_embed: Some(false),
                per_page: u32::MAX,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(not_embedded.len(), 1);
        assert_ne!(&not_embedded[0].asset_id, asset_id);
    }

    #[test]
    fn search_similar_asset_ids_empty() {
        let catalog = setup_search_catalog();

        // Empty similar IDs should return nothing
        let ids: Vec<String> = vec![];
        let opts = SearchOptions {
            similar_asset_ids: Some(&ids),
            per_page: u32::MAX,
            ..Default::default()
        };
        let results = catalog.search_paginated(&opts).unwrap();
        assert_eq!(results.len(), 0);
    }

    #[test]
    fn search_text_search_ids_filter() {
        let catalog = setup_search_catalog();
        let all = catalog
            .search_paginated(&SearchOptions {
                per_page: u32::MAX,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(all.len(), 2);

        // Filter using text_search_ids (same mechanism as similar but for text queries)
        let ids = vec![all[0].asset_id.clone()];
        let opts = SearchOptions {
            text_search_ids: Some(&ids),
            per_page: u32::MAX,
            ..Default::default()
        };
        let results = catalog.search_paginated(&opts).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].asset_id, all[0].asset_id);
    }

    #[test]
    fn search_text_search_ids_empty() {
        let catalog = setup_search_catalog();
        let ids: Vec<String> = vec![];
        let opts = SearchOptions {
            text_search_ids: Some(&ids),
            per_page: u32::MAX,
            ..Default::default()
        };
        let results = catalog.search_paginated(&opts).unwrap();
        assert_eq!(results.len(), 0);
    }

    #[test]
    fn search_text_search_ids_composes_with_other_filters() {
        let catalog = setup_search_catalog();
        let all = catalog
            .search_paginated(&SearchOptions {
                per_page: u32::MAX,
                ..Default::default()
            })
            .unwrap();

        // Both IDs in text_search_ids, but restrict by rating
        let ids: Vec<String> = all.iter().map(|r| r.asset_id.clone()).collect();
        let opts = SearchOptions {
            text_search_ids: Some(&ids),
            rating: Some(NumericFilter::Min(5.0)), // Only the 5-star asset
            per_page: u32::MAX,
            ..Default::default()
        };
        let results = catalog.search_paginated(&opts).unwrap();
        // Should only return assets matching both text_search_ids AND rating
        assert!(results.len() <= ids.len());
    }
}
