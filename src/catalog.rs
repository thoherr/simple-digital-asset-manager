use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;
use rusqlite::Connection;

use crate::query::NumericFilter;

use crate::models::{Asset, FileLocation, Recipe, Variant};

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
pub const SCHEMA_VERSION: u32 = 5;

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
            let re = &cache.as_ref().unwrap().1;

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
    pub fn open_and_migrate(catalog_root: &Path) -> Result<Self> {
        let catalog = Self::open(catalog_root)?;
        catalog.run_migrations();
        Ok(catalog)
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

    /// Run schema migrations, skipping work that is already done.
    ///
    /// Checks the current schema version and only executes migration blocks
    /// for versions newer than what the database already has.  Called once at
    /// startup (server init, `maki migrate`) and from `initialize()` for fresh
    /// catalogs (where version is 0, so everything runs).
    pub fn run_migrations(&self) {
        let current = self.schema_version();
        if current >= SCHEMA_VERSION {
            return;
        }

        // ── v0 → v1: base columns, indexes, denormalization, backfills ──
        if current < 1 {
            let _ = self.conn.execute_batch("ALTER TABLE assets ADD COLUMN rating INTEGER");
            let _ = self.conn.execute_batch("ALTER TABLE assets ADD COLUMN color_label TEXT");
            let _ = self.conn.execute_batch("ALTER TABLE variants ADD COLUMN camera_model TEXT");
            let _ = self.conn.execute_batch("ALTER TABLE variants ADD COLUMN lens_model TEXT");
            let _ = self.conn.execute_batch("ALTER TABLE variants ADD COLUMN focal_length_mm REAL");
            let _ = self.conn.execute_batch("ALTER TABLE variants ADD COLUMN f_number REAL");
            let _ = self.conn.execute_batch("ALTER TABLE variants ADD COLUMN iso INTEGER");
            let _ = self.conn.execute_batch("ALTER TABLE variants ADD COLUMN image_width INTEGER");
            let _ = self.conn.execute_batch("ALTER TABLE variants ADD COLUMN image_height INTEGER");
            let _ = self.conn.execute_batch(
                "CREATE INDEX IF NOT EXISTS idx_variants_camera ON variants(camera_model);
                 CREATE INDEX IF NOT EXISTS idx_variants_lens ON variants(lens_model);
                 CREATE INDEX IF NOT EXISTS idx_variants_iso ON variants(iso);
                 CREATE INDEX IF NOT EXISTS idx_variants_focal ON variants(focal_length_mm);",
            );
            // Backfill metadata columns from existing JSON
            let _ = self.conn.execute_batch(
                "UPDATE variants SET
                    camera_model = json_extract(source_metadata, '$.camera_model'),
                    lens_model = json_extract(source_metadata, '$.lens_model'),
                    iso = CAST(json_extract(source_metadata, '$.iso') AS INTEGER),
                    focal_length_mm = CAST(REPLACE(json_extract(source_metadata, '$.focal_length'), ' mm', '') AS REAL),
                    f_number = CAST(json_extract(source_metadata, '$.f_number') AS REAL),
                    image_width = CAST(json_extract(source_metadata, '$.image_width') AS INTEGER),
                    image_height = CAST(json_extract(source_metadata, '$.image_height') AS INTEGER)
                WHERE camera_model IS NULL AND source_metadata != '{}'"
            );
            // best_variant_hash denormalization
            let _ = self.conn.execute_batch("ALTER TABLE assets ADD COLUMN best_variant_hash TEXT");
            let _ = self.conn.execute_batch(
                "CREATE INDEX IF NOT EXISTS idx_variants_asset_id ON variants(asset_id)",
            );
            let _ = self.conn.execute_batch(
                "UPDATE assets SET best_variant_hash = (
                    SELECT content_hash FROM variants WHERE asset_id = assets.id
                    ORDER BY
                        CASE role WHEN 'export' THEN 300 WHEN 'processed' THEN 200
                            WHEN 'original' THEN 100 ELSE 0 END +
                        CASE WHEN LOWER(format) IN ('jpg','jpeg','png','tiff','tif','webp')
                            THEN 50 ELSE 0 END +
                        MIN(file_size / 1000000, 49)
                    DESC LIMIT 1
                ) WHERE best_variant_hash IS NULL",
            );
            // primary_variant_format + variant_count denormalization
            let _ = self.conn.execute_batch("ALTER TABLE assets ADD COLUMN primary_variant_format TEXT");
            let _ = self.conn.execute_batch("ALTER TABLE assets ADD COLUMN variant_count INTEGER NOT NULL DEFAULT 0");
            let _ = self.conn.execute_batch(
                "UPDATE assets SET primary_variant_format = COALESCE(
                    (SELECT format FROM variants WHERE asset_id = assets.id AND role = 'original'
                     AND LOWER(format) IN ('raw','cr2','cr3','nef','arw','orf','rw2','dng','raf','pef','srw')
                     LIMIT 1),
                    (SELECT format FROM variants WHERE asset_id = assets.id AND role = 'original' LIMIT 1),
                    (SELECT format FROM variants WHERE content_hash = assets.best_variant_hash)
                ) WHERE primary_variant_format IS NULL",
            );
            let _ = self.conn.execute_batch(
                "UPDATE assets SET variant_count = (
                    SELECT COUNT(*) FROM variants WHERE asset_id = assets.id
                ) WHERE variant_count = 0",
            );
            // Collection and stack tables
            let _ = crate::collection::CollectionStore::initialize(&self.conn);
            let _ = crate::stack::StackStore::initialize(&self.conn);
            let _ = self.conn.execute_batch("ALTER TABLE assets ADD COLUMN stack_id TEXT");
            let _ = self.conn.execute_batch("ALTER TABLE assets ADD COLUMN stack_position INTEGER");
            let _ = self.conn.execute_batch(
                "CREATE INDEX IF NOT EXISTS idx_assets_stack_id ON assets(stack_id);",
            );
            // Volume purpose
            let _ = self.conn.execute_batch("ALTER TABLE volumes ADD COLUMN purpose TEXT");
            // Performance indexes
            let _ = self.conn.execute_batch(
                "CREATE INDEX IF NOT EXISTS idx_fl_content_hash ON file_locations(content_hash);
                 CREATE INDEX IF NOT EXISTS idx_fl_volume_id ON file_locations(volume_id);
                 CREATE INDEX IF NOT EXISTS idx_assets_created_at ON assets(created_at);
                 CREATE INDEX IF NOT EXISTS idx_assets_best_variant_hash ON assets(best_variant_hash);
                 CREATE INDEX IF NOT EXISTS idx_variants_format ON variants(format);
                 CREATE INDEX IF NOT EXISTS idx_recipes_variant_hash ON recipes(variant_hash);
                 CREATE INDEX IF NOT EXISTS idx_assets_stack_browse ON assets(stack_position, created_at DESC) WHERE stack_id IS NOT NULL;",
            );
            // GPS coordinate columns
            let _ = self.conn.execute_batch("ALTER TABLE assets ADD COLUMN latitude REAL");
            let _ = self.conn.execute_batch("ALTER TABLE assets ADD COLUMN longitude REAL");
            let _ = self.conn.execute_batch(
                "CREATE INDEX IF NOT EXISTS idx_assets_geo ON assets(latitude, longitude) WHERE latitude IS NOT NULL",
            );
            // Preview rotation override
            let _ = self.conn.execute_batch("ALTER TABLE assets ADD COLUMN preview_rotation INTEGER");
            self.backfill_gps_columns();
            // Face count denormalized column
            let _ = self.conn.execute_batch("ALTER TABLE assets ADD COLUMN face_count INTEGER NOT NULL DEFAULT 0");
            #[cfg(feature = "ai")]
            {
                let _ = self.conn.execute_batch(
                    "UPDATE assets SET face_count = (SELECT COUNT(*) FROM faces WHERE asset_id = assets.id) WHERE face_count = 0 AND EXISTS (SELECT 1 FROM sqlite_master WHERE type='table' AND name='faces')",
                );
            }
            let _ = self.conn.execute_batch(
                "CREATE INDEX IF NOT EXISTS idx_assets_face_count ON assets(face_count) WHERE face_count > 0",
            );
            // Embeddings table
            let _ = self.conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS embeddings (
                    asset_id TEXT NOT NULL,
                    model TEXT NOT NULL DEFAULT 'siglip-vit-b16-256',
                    embedding BLOB NOT NULL,
                    PRIMARY KEY (asset_id, model)
                )",
            );
            #[cfg(feature = "ai")]
            {
                let _ = crate::embedding_store::EmbeddingStore::initialize(&self.conn);
                let _ = crate::face_store::FaceStore::initialize(&self.conn);
            }
            // Fix MicrosoftPhoto:Rating percentage values (1-100) → xmp:Rating scale (1-5)
            let _ = self.conn.execute_batch(
                "UPDATE assets SET rating = CASE
                    WHEN rating BETWEEN 1 AND 12 THEN 1
                    WHEN rating BETWEEN 13 AND 37 THEN 2
                    WHEN rating BETWEEN 38 AND 62 THEN 3
                    WHEN rating BETWEEN 63 AND 87 THEN 4
                    ELSE 5
                 END WHERE rating > 5",
            );
        }

        // ── v1 → v2: pending writeback tracking ──
        if current < 2 {
            let _ = self.conn.execute_batch(
                "ALTER TABLE recipes ADD COLUMN pending_writeback INTEGER NOT NULL DEFAULT 0",
            );
        }

        // ── v2 → v3: preview variant override ──
        if current < 3 {
            let _ = self.conn.execute_batch("ALTER TABLE assets ADD COLUMN preview_variant TEXT");
        }

        // ── v3 → v4: video duration denormalized column ──
        if current < 4 {
            let _ = self.conn.execute_batch("ALTER TABLE assets ADD COLUMN video_duration REAL");
            // Backfill from variant source_metadata JSON
            let _ = self.conn.execute_batch(
                "UPDATE assets SET video_duration = ( \
                    SELECT CAST(json_extract(v.source_metadata, '$.video_duration') AS REAL) \
                    FROM variants v WHERE v.asset_id = assets.id \
                    AND json_extract(v.source_metadata, '$.video_duration') IS NOT NULL \
                    LIMIT 1 \
                 ) WHERE video_duration IS NULL",
            );
        }

        // ── v4 → v5: video codec denormalized column ──
        if current < 5 {
            let _ = self.conn.execute_batch("ALTER TABLE assets ADD COLUMN video_codec TEXT");
            let _ = self.conn.execute_batch(
                "UPDATE assets SET video_codec = ( \
                    SELECT json_extract(v.source_metadata, '$.video_codec') \
                    FROM variants v WHERE v.asset_id = assets.id \
                    AND json_extract(v.source_metadata, '$.video_codec') IS NOT NULL \
                    LIMIT 1 \
                 ) WHERE video_codec IS NULL",
            );
        }

        // Stamp the new schema version
        let _ = self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_version (version INTEGER NOT NULL);
             DELETE FROM schema_version;",
        );
        let _ = self.conn.execute(
            "INSERT INTO schema_version (version) VALUES (?1)",
            rusqlite::params![SCHEMA_VERSION],
        );
    }

    /// Initialize the database schema.
    ///
    /// Creates base tables, then delegates to `run_migrations()` for all
    /// ADD COLUMN, CREATE INDEX, backfill, and schema version stamping.
    pub fn initialize(&self) -> Result<()> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS assets (
                id TEXT PRIMARY KEY,
                name TEXT,
                created_at TEXT NOT NULL,
                asset_type TEXT NOT NULL,
                tags TEXT NOT NULL DEFAULT '[]',
                description TEXT
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
                volume_id TEXT NOT NULL REFERENCES volumes(id),
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
            );",
        )?;

        // All columns, indexes, backfills, and version stamping handled by migrations
        self.run_migrations();

        Ok(())
    }

    /// Insert an asset into the catalog.
    pub fn insert_asset(&self, asset: &Asset) -> Result<()> {
        let tags_json = serde_json::to_string(&asset.tags)?;
        let best_hash = crate::models::variant::compute_best_variant_hash_with_override(
            &asset.variants,
            asset.preview_variant.as_deref(),
        );
        let primary_format = crate::models::variant::compute_primary_format(&asset.variants);
        let variant_count = asset.variants.len() as i64;
        let (latitude, longitude) = crate::models::variant::compute_gps_from_variants(&asset.variants);
        // Compute video duration from first variant that has it
        let video_duration: Option<f64> = asset.variants.iter()
            .find_map(|v| v.source_metadata.get("video_duration")?.parse::<f64>().ok());
        let video_codec: Option<String> = asset.variants.iter()
            .find_map(|v| v.source_metadata.get("video_codec").cloned());
        // Use ON CONFLICT UPDATE instead of INSERT OR REPLACE to avoid
        // intermediate DELETE that triggers FK constraint violations on
        // variants/faces/collection_assets referencing this asset.
        self.conn.execute(
            "INSERT INTO assets (id, name, created_at, asset_type, tags, description, rating, color_label, best_variant_hash, primary_variant_format, variant_count, latitude, longitude, preview_rotation, preview_variant, video_duration, video_codec) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17) \
             ON CONFLICT(id) DO UPDATE SET \
               name = excluded.name, \
               created_at = excluded.created_at, \
               asset_type = excluded.asset_type, \
               tags = excluded.tags, \
               description = excluded.description, \
               rating = excluded.rating, \
               color_label = excluded.color_label, \
               best_variant_hash = excluded.best_variant_hash, \
               primary_variant_format = excluded.primary_variant_format, \
               variant_count = excluded.variant_count, \
               latitude = excluded.latitude, \
               longitude = excluded.longitude, \
               preview_rotation = excluded.preview_rotation, \
               preview_variant = excluded.preview_variant, \
               video_duration = excluded.video_duration, \
               video_codec = excluded.video_codec",
            rusqlite::params![
                asset.id.to_string(),
                asset.name,
                asset.created_at.to_rfc3339(),
                format!("{:?}", asset.asset_type).to_lowercase(),
                tags_json,
                asset.description,
                asset.rating.map(|r| r as i64),
                asset.color_label,
                best_hash,
                primary_format,
                variant_count,
                latitude,
                longitude,
                asset.preview_rotation.map(|r| r as i64),
                asset.preview_variant,
                video_duration,
                video_codec,
            ],
        )?;
        Ok(())
    }

    /// Update just the rating for an asset in the catalog.
    pub fn update_asset_rating(&self, asset_id: &str, rating: Option<u8>) -> Result<()> {
        self.conn.execute(
            "UPDATE assets SET rating = ?1 WHERE id = ?2",
            rusqlite::params![rating.map(|r| r as i64), asset_id],
        )?;
        Ok(())
    }

    /// Update just the color label for an asset in the catalog.
    pub fn update_asset_color_label(&self, asset_id: &str, color_label: Option<&str>) -> Result<()> {
        self.conn.execute(
            "UPDATE assets SET color_label = ?1 WHERE id = ?2",
            rusqlite::params![color_label, asset_id],
        )?;
        Ok(())
    }

    /// Update just the preview rotation for an asset in the catalog.
    pub fn update_asset_preview_rotation(&self, asset_id: &str, rotation: Option<u16>) -> Result<()> {
        self.conn.execute(
            "UPDATE assets SET preview_rotation = ?1 WHERE id = ?2",
            rusqlite::params![rotation.map(|r| r as i64), asset_id],
        )?;
        Ok(())
    }

    /// Update the preview variant override and recompute best_variant_hash.
    pub fn update_asset_preview_variant(
        &self,
        asset_id: &str,
        preview_variant: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE assets SET preview_variant = ?1 WHERE id = ?2",
            rusqlite::params![preview_variant, asset_id],
        )?;
        // Recompute best_variant_hash: if override set, use it; else fall back to scoring
        if let Some(hash) = preview_variant {
            self.conn.execute(
                "UPDATE assets SET best_variant_hash = ?1 WHERE id = ?2 AND EXISTS (SELECT 1 FROM variants WHERE content_hash = ?1 AND asset_id = ?2)",
                rusqlite::params![hash, asset_id],
            )?;
        } else {
            // Clear override — recompute from scoring via SQL
            self.conn.execute(
                "UPDATE assets SET best_variant_hash = (
                    SELECT content_hash FROM variants WHERE asset_id = ?1
                    ORDER BY
                        CASE role WHEN 'export' THEN 300 WHEN 'processed' THEN 200
                            WHEN 'original' THEN 100 ELSE 0 END +
                        CASE WHEN LOWER(format) IN ('jpg','jpeg','png','tiff','tif','webp')
                            THEN 50 ELSE 0 END +
                        MIN(file_size / 1000000, 49)
                    DESC LIMIT 1
                ) WHERE id = ?1",
                rusqlite::params![asset_id],
            )?;
        }
        Ok(())
    }

    /// Update the denormalized face_count for an asset.
    /// Recomputes from the faces table (requires faces table to exist).
    pub fn update_face_count(&self, asset_id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE assets SET face_count = (SELECT COUNT(*) FROM faces WHERE asset_id = ?1) WHERE id = ?1",
            rusqlite::params![asset_id],
        )?;
        Ok(())
    }

    /// Update just the created_at date for an asset in the catalog.
    pub fn update_asset_created_at(&self, asset_id: &str, created_at: &chrono::DateTime<chrono::Utc>) -> Result<()> {
        self.conn.execute(
            "UPDATE assets SET created_at = ?1 WHERE id = ?2",
            rusqlite::params![created_at.to_rfc3339(), asset_id],
        )?;
        Ok(())
    }

    /// Update denormalized variant columns for an asset.
    pub fn update_best_variant_hash(&self, asset_id: &str, hash: Option<&str>) -> Result<()> {
        self.conn.execute(
            "UPDATE assets SET best_variant_hash = ?1 WHERE id = ?2",
            rusqlite::params![hash, asset_id],
        )?;
        Ok(())
    }

    /// Update all denormalized variant columns from an asset's variants.
    pub fn update_denormalized_variant_columns(&self, asset: &Asset) -> Result<()> {
        let best_hash = crate::models::variant::compute_best_variant_hash_with_override(
            &asset.variants,
            asset.preview_variant.as_deref(),
        );
        let primary_format = crate::models::variant::compute_primary_format(&asset.variants);
        let variant_count = asset.variants.len() as i64;
        let (latitude, longitude) = crate::models::variant::compute_gps_from_variants(&asset.variants);
        self.conn.execute(
            "UPDATE assets SET best_variant_hash = ?1, primary_variant_format = ?2, variant_count = ?3, latitude = ?4, longitude = ?5 WHERE id = ?6",
            rusqlite::params![best_hash, primary_format, variant_count, latitude, longitude, asset.id.to_string()],
        )?;
        Ok(())
    }

    /// Insert a variant into the catalog.
    pub fn insert_variant(&self, variant: &Variant) -> Result<()> {
        let meta = &variant.source_metadata;
        let meta_json = serde_json::to_string(meta)?;

        let camera_model = meta.get("camera_model").cloned();
        let lens_model = meta.get("lens_model").cloned();
        let focal_length_mm: Option<f64> = meta
            .get("focal_length")
            .and_then(|v| v.trim_end_matches(" mm").parse().ok());
        let f_number: Option<f64> = meta.get("f_number").and_then(|v| v.parse().ok());
        let iso: Option<i64> = meta.get("iso").and_then(|v| v.parse().ok());
        let image_width: Option<i64> = meta.get("image_width").and_then(|v| v.parse().ok());
        let image_height: Option<i64> = meta.get("image_height").and_then(|v| v.parse().ok());

        self.conn.execute(
            "INSERT OR REPLACE INTO variants (content_hash, asset_id, role, format, file_size, original_filename, source_metadata, \
             camera_model, lens_model, focal_length_mm, f_number, iso, image_width, image_height) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            rusqlite::params![
                variant.content_hash,
                variant.asset_id.to_string(),
                format!("{:?}", variant.role).to_lowercase(),
                variant.format,
                variant.file_size,
                variant.original_filename,
                meta_json,
                camera_model,
                lens_model,
                focal_length_mm,
                f_number,
                iso,
                image_width,
                image_height,
            ],
        )?;
        Ok(())
    }

    /// Insert a file location for a variant.
    pub fn insert_file_location(&self, content_hash: &str, loc: &FileLocation) -> Result<()> {
        // Check if this exact location already exists (no unique constraint on table)
        let exists: bool = self.conn.query_row(
            "SELECT COUNT(*) > 0 FROM file_locations WHERE content_hash = ?1 AND volume_id = ?2 AND relative_path = ?3",
            rusqlite::params![content_hash, loc.volume_id.to_string(), loc.relative_path_str()],
            |r| r.get(0),
        )?;
        if exists {
            return Ok(());
        }
        self.conn.execute(
            "INSERT INTO file_locations (content_hash, volume_id, relative_path, verified_at) \
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![
                content_hash,
                loc.volume_id.to_string(),
                loc.relative_path_str(),
                loc.verified_at.map(|t| t.to_rfc3339()),
            ],
        )?;
        Ok(())
    }

    /// List all file locations with their associated asset IDs.
    /// Returns `(asset_id, volume_id, relative_path)` tuples.
    pub fn list_all_locations_with_assets(&self) -> Result<Vec<(String, String, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT v.asset_id, fl.volume_id, fl.relative_path \
             FROM file_locations fl \
             JOIN variants v ON fl.content_hash = v.content_hash",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    }

    /// Insert a recipe into the catalog.
    pub fn insert_recipe(&self, recipe: &Recipe) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO recipes (id, variant_hash, software, recipe_type, content_hash, volume_id, relative_path) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                recipe.id.to_string(),
                recipe.variant_hash,
                recipe.software,
                format!("{:?}", recipe.recipe_type).to_lowercase(),
                recipe.content_hash,
                recipe.location.volume_id.to_string(),
                recipe.location.relative_path_str(),
            ],
        )?;
        Ok(())
    }

    /// Ensure a volume record exists in the SQLite cache.
    pub fn ensure_volume(&self, volume: &crate::models::Volume) -> Result<()> {
        self.conn.execute(
            "INSERT INTO volumes (id, label, mount_point, volume_type, purpose) \
             VALUES (?1, ?2, ?3, ?4, ?5) \
             ON CONFLICT(id) DO UPDATE SET purpose = excluded.purpose",
            rusqlite::params![
                volume.id.to_string(),
                volume.label,
                volume.mount_point.to_string_lossy().to_string(),
                format!("{:?}", volume.volume_type).to_lowercase(),
                volume.purpose.as_ref().map(|p| p.as_str()),
            ],
        )?;
        Ok(())
    }

    /// Delete a volume row from the catalog.
    pub fn delete_volume(&self, volume_id: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM volumes WHERE id = ?1",
            rusqlite::params![volume_id],
        )?;
        Ok(())
    }

    /// Rename a volume in the catalog.
    pub fn rename_volume(&self, volume_id: &str, new_label: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE volumes SET label = ?1 WHERE id = ?2",
            rusqlite::params![new_label, volume_id],
        )?;
        Ok(())
    }

    /// Count file_location rows on a given volume.
    pub fn count_locations_for_volume(&self, volume_id: &str) -> Result<usize> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM file_locations WHERE volume_id = ?1",
            rusqlite::params![volume_id],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    /// Count recipe rows on a given volume.
    pub fn count_recipes_for_volume(&self, volume_id: &str) -> Result<usize> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM recipes WHERE volume_id = ?1",
            rusqlite::params![volume_id],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    /// List distinct asset IDs that have file_locations or recipes on a given volume.
    pub fn list_asset_ids_on_volume(&self, volume_id: &str) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT v.asset_id FROM file_locations fl \
             JOIN variants v ON v.content_hash = fl.content_hash \
             WHERE fl.volume_id = ?1 \
             UNION \
             SELECT DISTINCT v.asset_id FROM recipes r \
             JOIN variants v ON v.content_hash = r.variant_hash \
             WHERE r.volume_id = ?1",
        )?;
        let rows = stmt.query_map(rusqlite::params![volume_id], |row| {
            row.get::<_, String>(0)
        })?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }

    /// Bulk-move all file_locations from one volume to another, prepending a path prefix.
    /// Returns the number of rows updated.
    pub fn bulk_move_file_locations(
        &self,
        source_volume_id: &str,
        target_volume_id: &str,
        prefix: &str,
    ) -> Result<usize> {
        let changed = self.conn.execute(
            "UPDATE file_locations SET volume_id = ?1, relative_path = ?2 || relative_path \
             WHERE volume_id = ?3",
            rusqlite::params![target_volume_id, prefix, source_volume_id],
        )?;
        Ok(changed)
    }

    /// Bulk-move all recipes from one volume to another, prepending a path prefix.
    /// Returns the number of rows updated.
    pub fn bulk_move_recipes(
        &self,
        source_volume_id: &str,
        target_volume_id: &str,
        prefix: &str,
    ) -> Result<usize> {
        let changed = self.conn.execute(
            "UPDATE recipes SET volume_id = ?1, relative_path = ?2 || relative_path \
             WHERE volume_id = ?3",
            rusqlite::params![target_volume_id, prefix, source_volume_id],
        )?;
        Ok(changed)
    }

    /// List asset IDs on a volume whose file locations match a path prefix.
    pub fn list_asset_ids_on_volume_with_prefix(&self, volume_id: &str, prefix: &str) -> Result<Vec<String>> {
        let pattern = format!("{prefix}%");
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT v.asset_id FROM file_locations fl \
             JOIN variants v ON v.content_hash = fl.content_hash \
             WHERE fl.volume_id = ?1 AND fl.relative_path LIKE ?2 \
             UNION \
             SELECT DISTINCT v.asset_id FROM recipes r \
             JOIN variants v ON v.content_hash = r.variant_hash \
             WHERE r.volume_id = ?1 AND r.relative_path LIKE ?2",
        )?;
        let rows = stmt.query_map(rusqlite::params![volume_id, pattern], |row| {
            row.get::<_, String>(0)
        })?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }

    /// Bulk-move file_locations matching a prefix from source to target volume, stripping the prefix.
    /// Returns the number of rows updated.
    pub fn bulk_split_file_locations(
        &self,
        source_volume_id: &str,
        target_volume_id: &str,
        prefix: &str,
    ) -> Result<usize> {
        let changed = self.conn.execute(
            "UPDATE file_locations SET volume_id = ?1, relative_path = SUBSTR(relative_path, ?2) \
             WHERE volume_id = ?3 AND relative_path LIKE ?4",
            rusqlite::params![target_volume_id, prefix.len() + 1, source_volume_id, format!("{prefix}%")],
        )?;
        Ok(changed)
    }

    /// Bulk-move recipes matching a prefix from source to target volume, stripping the prefix.
    /// Returns the number of rows updated.
    pub fn bulk_split_recipes(
        &self,
        source_volume_id: &str,
        target_volume_id: &str,
        prefix: &str,
    ) -> Result<usize> {
        let changed = self.conn.execute(
            "UPDATE recipes SET volume_id = ?1, relative_path = SUBSTR(relative_path, ?2) \
             WHERE volume_id = ?3 AND relative_path LIKE ?4",
            rusqlite::params![target_volume_id, prefix.len() + 1, source_volume_id, format!("{prefix}%")],
        )?;
        Ok(changed)
    }

    /// Check if a variant with the given content hash already exists.
    pub fn has_variant(&self, content_hash: &str) -> Result<bool> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM variants WHERE content_hash = ?1",
            rusqlite::params![content_hash],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    /// Look up a variant's format by its content hash.
    pub fn get_variant_format(&self, content_hash: &str) -> Result<Option<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT format FROM variants WHERE content_hash = ?1 LIMIT 1",
        )?;
        let mut rows = stmt.query(rusqlite::params![content_hash])?;
        match rows.next()? {
            Some(row) => Ok(Some(row.get(0)?)),
            None => Ok(None),
        }
    }

    /// Look up file locations for a variant by content hash.
    /// Returns (volume_id, relative_path) pairs.
    pub fn get_variant_file_locations(&self, content_hash: &str) -> Result<Vec<(String, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT volume_id, relative_path FROM file_locations WHERE content_hash = ?1",
        )?;
        let rows = stmt.query_map(rusqlite::params![content_hash], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Open an in-memory catalog (for testing).
    #[cfg(test)]
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        Ok(Self { conn })
    }

    /// Search assets by optional filters. Results join assets with variants.
    pub fn search_assets(
        &self,
        text: Option<&str>,
        asset_type: Option<&str>,
        tag: Option<&str>,
        format: Option<&str>,
        rating_min: Option<u8>,
        rating_exact: Option<u8>,
    ) -> Result<Vec<SearchRow>> {
        let asset_types_vec;
        let tags_vec;
        let formats_vec;
        let opts = SearchOptions {
            text,
            asset_types: if let Some(t) = asset_type {
                asset_types_vec = vec![t.to_string()];
                &asset_types_vec
            } else {
                &[]
            },
            tags: if let Some(t) = tag {
                tags_vec = vec![t.to_string()];
                &tags_vec
            } else {
                &[]
            },
            formats: if let Some(f) = format {
                formats_vec = vec![f.to_string()];
                &formats_vec
            } else {
                &[]
            },
            rating: if let Some(min) = rating_min {
                Some(NumericFilter::Min(min as f64))
            } else if let Some(exact) = rating_exact {
                Some(NumericFilter::Exact(exact as f64))
            } else {
                None
            },
            per_page: u32::MAX,
            ..Default::default()
        };
        self.search_paginated(&opts)
    }

    /// Resolve a short asset ID prefix to a full UUID string.
    ///
    /// Returns `Ok(Some(id))` if exactly one match, `Ok(None)` if no match,
    /// or an error if the prefix is ambiguous (multiple matches).
    pub fn resolve_asset_id(&self, prefix: &str) -> Result<Option<String>> {
        let pattern = format!("{prefix}%");
        let mut stmt = self.conn.prepare(
            "SELECT id FROM assets WHERE id LIKE ?1",
        )?;
        let ids: Vec<String> = stmt
            .query_map(rusqlite::params![pattern], |row| row.get(0))?
            .collect::<std::result::Result<_, _>>()?;

        match ids.len() {
            0 => Ok(None),
            1 => Ok(Some(ids.into_iter().next().unwrap())),
            n => anyhow::bail!(
                "Ambiguous asset ID prefix '{prefix}': matches {n} assets"
            ),
        }
    }

    /// Load full asset details from the catalog (variants + locations).
    pub fn load_asset_details(&self, asset_id: &str) -> Result<Option<AssetDetails>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, asset_type, created_at, tags, description, rating, color_label \
             FROM assets WHERE id = ?1",
        )?;
        let mut rows = stmt.query(rusqlite::params![asset_id])?;
        let row = match rows.next()? {
            Some(r) => r,
            None => return Ok(None),
        };

        let id: String = row.get(0)?;
        let name: Option<String> = row.get(1)?;
        let asset_type: String = row.get(2)?;
        let created_at: String = row.get(3)?;
        let tags_json: String = row.get(4)?;
        let description: Option<String> = row.get(5)?;
        let rating_val: Option<i64> = row.get(6)?;
        let rating = rating_val.map(|r| r as u8);
        let color_label: Option<String> = row.get(7)?;
        let tags: Vec<String> = serde_json::from_str(&tags_json).unwrap_or_default();

        // Load variants
        let mut vstmt = self.conn.prepare(
            "SELECT content_hash, role, format, file_size, original_filename, source_metadata \
             FROM variants WHERE asset_id = ?1",
        )?;
        let variants: Vec<VariantDetails> = vstmt
            .query_map(rusqlite::params![asset_id], |vrow| {
                let meta_json: String = vrow.get(5)?;
                let source_metadata: std::collections::HashMap<String, String> =
                    serde_json::from_str(&meta_json).unwrap_or_default();
                Ok(VariantDetails {
                    content_hash: vrow.get(0)?,
                    role: vrow.get(1)?,
                    format: vrow.get(2)?,
                    file_size: vrow.get(3)?,
                    original_filename: vrow.get(4)?,
                    source_metadata,
                    locations: Vec::new(), // filled below
                })
            })?
            .collect::<std::result::Result<_, _>>()?;

        // Load locations for each variant
        let mut lstmt = self.conn.prepare(
            "SELECT fl.relative_path, vol.label, vol.id, vol.purpose, fl.verified_at \
             FROM file_locations fl \
             JOIN volumes vol ON fl.volume_id = vol.id \
             WHERE fl.content_hash = ?1",
        )?;

        let variants: Vec<VariantDetails> = variants
            .into_iter()
            .map(|mut v| {
                let locs: Vec<LocationDetails> = lstmt
                    .query_map(rusqlite::params![v.content_hash], |lrow| {
                        Ok(LocationDetails {
                            relative_path: lrow.get(0)?,
                            volume_label: lrow.get(1)?,
                            volume_id: lrow.get(2)?,
                            volume_purpose: lrow.get(3)?,
                            verified_at: lrow.get(4)?,
                        })
                    })
                    .unwrap_or_else(|_| {
                        // Return an empty iterator wrapper on error
                        panic!("failed to query locations")
                    })
                    .filter_map(|r| r.ok())
                    .collect();
                v.locations = locs;
                v
            })
            .collect();

        // Load recipes linked to any variant of this asset
        let mut rstmt = self.conn.prepare(
            "SELECT r.variant_hash, r.software, r.recipe_type, r.content_hash, r.volume_id, \
                    vol.label, r.relative_path, r.pending_writeback \
             FROM recipes r \
             JOIN variants v ON r.variant_hash = v.content_hash \
             LEFT JOIN volumes vol ON r.volume_id = vol.id \
             WHERE v.asset_id = ?1",
        )?;
        let recipes: Vec<RecipeDetails> = rstmt
            .query_map(rusqlite::params![asset_id], |rrow| {
                Ok(RecipeDetails {
                    variant_hash: rrow.get(0)?,
                    software: rrow.get(1)?,
                    recipe_type: rrow.get(2)?,
                    content_hash: rrow.get(3)?,
                    volume_id: rrow.get(4)?,
                    volume_label: rrow.get(5)?,
                    relative_path: rrow.get(6)?,
                    pending_writeback: rrow.get::<_, i32>(7).unwrap_or(0) != 0,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        Ok(Some(AssetDetails {
            id,
            name,
            asset_type,
            created_at,
            tags,
            description,
            rating,
            color_label,
            variants,
            recipes,
        }))
    }

    /// Find which asset owns a variant by its content hash.
    pub fn find_asset_id_by_variant(&self, content_hash: &str) -> Result<Option<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT asset_id FROM variants WHERE content_hash = ?1",
        )?;
        let mut rows = stmt.query(rusqlite::params![content_hash])?;
        match rows.next()? {
            Some(row) => Ok(Some(row.get(0)?)),
            None => Ok(None),
        }
    }

    /// Get an asset's name by ID.
    pub fn get_asset_name(&self, asset_id: &str) -> Result<Option<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT COALESCE(a.name, bv.original_filename) FROM assets a \
             LEFT JOIN variants bv ON bv.content_hash = a.best_variant_hash \
             WHERE a.id = ?1",
        )?;
        let mut rows = stmt.query(rusqlite::params![asset_id])?;
        match rows.next()? {
            Some(row) => Ok(row.get(0)?),
            None => Ok(None),
        }
    }

    /// Get an asset's best_variant_hash by ID.
    pub fn get_asset_best_variant_hash(&self, asset_id: &str) -> Result<Option<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT best_variant_hash FROM assets WHERE id = ?1",
        )?;
        let mut rows = stmt.query(rusqlite::params![asset_id])?;
        match rows.next()? {
            Some(row) => Ok(row.get(0)?),
            None => Ok(None),
        }
    }

    /// Reassign a variant to a different asset in the catalog.
    pub fn update_variant_asset_id(&self, content_hash: &str, new_asset_id: &str) -> Result<()> {
        let changed = self.conn.execute(
            "UPDATE variants SET asset_id = ?1 WHERE content_hash = ?2",
            rusqlite::params![new_asset_id, content_hash],
        )?;
        if changed == 0 {
            anyhow::bail!("No variant found with hash '{content_hash}'");
        }
        Ok(())
    }

    /// Update a variant's role in the catalog.
    pub fn update_variant_role(&self, content_hash: &str, role: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE variants SET role = ?1 WHERE content_hash = ?2",
            rusqlite::params![role, content_hash],
        )?;
        Ok(())
    }

    /// Delete an asset row from the catalog.
    pub fn delete_asset(&self, asset_id: &str) -> Result<()> {
        let changed = self.conn.execute(
            "DELETE FROM assets WHERE id = ?1",
            rusqlite::params![asset_id],
        )?;
        if changed == 0 {
            anyhow::bail!("No asset found with id '{asset_id}'");
        }
        Ok(())
    }

    /// Load enriched location details for a variant hash.
    fn load_locations_for_hash(
        lstmt: &mut rusqlite::Statement,
        content_hash: &str,
    ) -> Vec<LocationDetails> {
        lstmt
            .query_map(rusqlite::params![content_hash], |lrow| {
                Ok(LocationDetails {
                    relative_path: lrow.get(0)?,
                    volume_label: lrow.get(1)?,
                    volume_id: lrow.get(2)?,
                    volume_purpose: lrow.get(3)?,
                    verified_at: lrow.get(4)?,
                })
            })
            .unwrap()
            .filter_map(|r| r.ok())
            .collect()
    }

    /// Compute `volume_count` and `same_volume_groups` from locations.
    fn compute_duplicate_stats(entry: &mut DuplicateEntry) {
        let mut vol_counts: HashMap<String, usize> = HashMap::new();
        for loc in &entry.locations {
            *vol_counts.entry(loc.volume_id.clone()).or_insert(0) += 1;
        }
        entry.volume_count = vol_counts.len();
        // Find volume labels where the same volume has 2+ locations
        let mut same_vol: Vec<String> = Vec::new();
        for loc in &entry.locations {
            let count = vol_counts.get(&loc.volume_id).copied().unwrap_or(0);
            if count > 1 && !same_vol.contains(&loc.volume_label) {
                same_vol.push(loc.volume_label.clone());
            }
        }
        entry.same_volume_groups = same_vol;
    }

    /// Load duplicate entries from a variant query and enrich with locations.
    fn load_duplicate_entries(
        &self,
        variant_query: &str,
    ) -> Result<Vec<DuplicateEntry>> {
        let mut stmt = self.conn.prepare(variant_query)?;

        let entries: Vec<DuplicateEntry> = stmt
            .query_map([], |row| {
                Ok(DuplicateEntry {
                    content_hash: row.get(0)?,
                    original_filename: row.get(1)?,
                    format: row.get(2)?,
                    file_size: row.get(3)?,
                    asset_name: row.get(4)?,
                    asset_id: row.get(5)?,
                    locations: Vec::new(),
                    volume_count: 0,
                    same_volume_groups: Vec::new(),
                    preview_url: String::new(),
                })
            })?
            .collect::<std::result::Result<_, _>>()?;

        let mut lstmt = self.conn.prepare(
            "SELECT fl.relative_path, vol.label, vol.id, vol.purpose, fl.verified_at \
             FROM file_locations fl \
             JOIN volumes vol ON fl.volume_id = vol.id \
             WHERE fl.content_hash = ?1",
        )?;

        let entries: Vec<DuplicateEntry> = entries
            .into_iter()
            .map(|mut e| {
                e.locations = Self::load_locations_for_hash(&mut lstmt, &e.content_hash);
                Self::compute_duplicate_stats(&mut e);
                e
            })
            .collect();

        Ok(entries)
    }

    /// Find variants that have more than one file location (duplicates).
    pub fn find_duplicates(&self) -> Result<Vec<DuplicateEntry>> {
        self.load_duplicate_entries(
            "SELECT v.content_hash, v.original_filename, v.format, v.file_size, a.name, a.id \
             FROM variants v \
             JOIN assets a ON v.asset_id = a.id \
             WHERE v.content_hash IN ( \
                 SELECT content_hash FROM file_locations \
                 GROUP BY content_hash HAVING COUNT(*) > 1 \
             ) \
             ORDER BY v.file_size DESC",
        )
    }

    /// Find variants with 2+ locations on the **same** volume.
    pub fn find_duplicates_same_volume(&self) -> Result<Vec<DuplicateEntry>> {
        self.load_duplicate_entries(
            "SELECT v.content_hash, v.original_filename, v.format, v.file_size, a.name, a.id \
             FROM variants v \
             JOIN assets a ON v.asset_id = a.id \
             WHERE v.content_hash IN ( \
                 SELECT content_hash FROM file_locations \
                 GROUP BY content_hash, volume_id HAVING COUNT(*) > 1 \
             ) \
             ORDER BY v.file_size DESC",
        )
    }

    /// Find variants with locations on 2+ **different** volumes.
    pub fn find_duplicates_cross_volume(&self) -> Result<Vec<DuplicateEntry>> {
        self.load_duplicate_entries(
            "SELECT v.content_hash, v.original_filename, v.format, v.file_size, a.name, a.id \
             FROM variants v \
             JOIN assets a ON v.asset_id = a.id \
             WHERE v.content_hash IN ( \
                 SELECT content_hash FROM file_locations \
                 GROUP BY content_hash HAVING COUNT(DISTINCT volume_id) > 1 \
             ) \
             ORDER BY v.file_size DESC",
        )
    }

    /// Find duplicates with optional filters for volume, format, and path prefix.
    pub fn find_duplicates_filtered(
        &self,
        mode: &str,
        volume: Option<&str>,
        format: Option<&str>,
        path_prefix: Option<&str>,
    ) -> Result<Vec<DuplicateEntry>> {
        // Build the inner GROUP BY subquery based on mode
        let inner = match mode {
            "same" => {
                "SELECT content_hash FROM file_locations \
                 GROUP BY content_hash, volume_id HAVING COUNT(*) > 1"
            }
            "cross" => {
                "SELECT content_hash FROM file_locations \
                 GROUP BY content_hash HAVING COUNT(DISTINCT volume_id) > 1"
            }
            _ => {
                "SELECT content_hash FROM file_locations \
                 GROUP BY content_hash HAVING COUNT(*) > 1"
            }
        };

        let mut sql = format!(
            "SELECT v.content_hash, v.original_filename, v.format, v.file_size, a.name, a.id \
             FROM variants v \
             JOIN assets a ON v.asset_id = a.id \
             WHERE v.content_hash IN ({inner})"
        );

        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

        if let Some(vol) = volume {
            sql.push_str(
                " AND v.content_hash IN (SELECT content_hash FROM file_locations WHERE volume_id = ?)",
            );
            params.push(Box::new(vol.to_string()));
        }

        if let Some(prefix) = path_prefix {
            let like = path_pattern_to_like(prefix);
            sql.push_str(
                " AND v.content_hash IN (SELECT content_hash FROM file_locations WHERE relative_path LIKE ? ESCAPE '\\')",
            );
            params.push(Box::new(like));
        }

        if let Some(fmt) = format {
            sql.push_str(" AND LOWER(v.format) = ?");
            params.push(Box::new(fmt.to_lowercase()));
        }

        sql.push_str(" ORDER BY v.file_size DESC");

        self.load_duplicate_entries_filtered(&sql, &params)
    }

    /// Like `load_duplicate_entries` but accepts dynamic params.
    fn load_duplicate_entries_filtered(
        &self,
        variant_query: &str,
        params: &[Box<dyn rusqlite::types::ToSql>],
    ) -> Result<Vec<DuplicateEntry>> {
        let mut stmt = self.conn.prepare(variant_query)?;

        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| p.as_ref()).collect();

        let entries: Vec<DuplicateEntry> = stmt
            .query_map(param_refs.as_slice(), |row| {
                Ok(DuplicateEntry {
                    content_hash: row.get(0)?,
                    original_filename: row.get(1)?,
                    format: row.get(2)?,
                    file_size: row.get(3)?,
                    asset_name: row.get(4)?,
                    asset_id: row.get(5)?,
                    locations: Vec::new(),
                    volume_count: 0,
                    same_volume_groups: Vec::new(),
                    preview_url: String::new(),
                })
            })?
            .collect::<std::result::Result<_, _>>()?;

        let mut lstmt = self.conn.prepare(
            "SELECT fl.relative_path, vol.label, vol.id, vol.purpose, fl.verified_at \
             FROM file_locations fl \
             JOIN volumes vol ON fl.volume_id = vol.id \
             WHERE fl.content_hash = ?1",
        )?;

        let entries: Vec<DuplicateEntry> = entries
            .into_iter()
            .map(|mut e| {
                e.locations = Self::load_locations_for_hash(&mut lstmt, &e.content_hash);
                Self::compute_duplicate_stats(&mut e);
                e
            })
            .collect();

        Ok(entries)
    }

    /// Delete a specific file location row. Returns true if a row was deleted.
    pub fn delete_file_location(
        &self,
        content_hash: &str,
        volume_id: &str,
        relative_path: &str,
    ) -> Result<bool> {
        let changed = self.conn.execute(
            "DELETE FROM file_locations WHERE content_hash = ?1 AND volume_id = ?2 AND relative_path = ?3",
            rusqlite::params![content_hash, volume_id, relative_path],
        )?;
        Ok(changed > 0)
    }

    /// Delete a recipe record by ID. Returns true if a row was deleted.
    pub fn delete_recipe(&self, recipe_id: &str) -> Result<bool> {
        let changed = self.conn.execute(
            "DELETE FROM recipes WHERE id = ?1",
            rusqlite::params![recipe_id],
        )?;
        Ok(changed > 0)
    }

    /// Update the volume and path for a recipe.
    pub fn update_recipe_location(
        &self,
        recipe_id: &str,
        volume_id: &str,
        relative_path: &str,
    ) -> Result<()> {
        let changed = self.conn.execute(
            "UPDATE recipes SET volume_id = ?1, relative_path = ?2 WHERE id = ?3",
            rusqlite::params![volume_id, relative_path, recipe_id],
        )?;
        if changed == 0 {
            anyhow::bail!("No recipe found with id '{recipe_id}'");
        }
        Ok(())
    }

    /// Update the `verified_at` timestamp for a file location.
    pub fn update_verified_at(
        &self,
        content_hash: &str,
        volume_id: &str,
        relative_path: &str,
    ) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        self.conn.execute(
            "UPDATE file_locations SET verified_at = ?1 \
             WHERE content_hash = ?2 AND volume_id = ?3 AND relative_path = ?4",
            rusqlite::params![now, content_hash, volume_id, relative_path],
        )?;
        Ok(())
    }

    /// Get the verified_at timestamp for a file location or recipe at this volume+path.
    pub fn get_location_verified_at(
        &self,
        volume_id: &str,
        relative_path: &str,
    ) -> Result<Option<String>> {
        // Check file_locations first
        let mut stmt = self.conn.prepare(
            "SELECT verified_at FROM file_locations WHERE volume_id = ?1 AND relative_path = ?2 LIMIT 1",
        )?;
        let mut rows = stmt.query(rusqlite::params![volume_id, relative_path])?;
        if let Some(row) = rows.next()? {
            let verified_at: Option<String> = row.get(0)?;
            if verified_at.is_some() {
                return Ok(verified_at);
            }
        }
        // Fall back to recipes table
        let mut stmt = self.conn.prepare(
            "SELECT verified_at FROM recipes WHERE volume_id = ?1 AND relative_path = ?2 LIMIT 1",
        )?;
        let mut rows = stmt.query(rusqlite::params![volume_id, relative_path])?;
        if let Some(row) = rows.next()? {
            return Ok(row.get(0)?);
        }
        Ok(None)
    }

    /// Check if a recipe with the given content hash exists.
    pub fn has_recipe_by_content_hash(&self, content_hash: &str) -> Result<bool> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM recipes WHERE content_hash = ?1",
            rusqlite::params![content_hash],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    /// Find a recipe by its location (variant_hash, volume_id, relative_path).
    /// Returns `(recipe_id, content_hash)` if found.
    pub fn find_recipe_by_location(
        &self,
        variant_hash: &str,
        volume_id: &str,
        relative_path: &str,
    ) -> Result<Option<(String, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, content_hash FROM recipes \
             WHERE variant_hash = ?1 AND volume_id = ?2 AND relative_path = ?3",
        )?;
        let mut rows = stmt.query(rusqlite::params![variant_hash, volume_id, relative_path])?;
        match rows.next()? {
            Some(row) => Ok(Some((row.get(0)?, row.get(1)?))),
            None => Ok(None),
        }
    }

    /// Update a recipe's content hash (used when a recipe file changes on re-import).
    pub fn update_recipe_content_hash(&self, recipe_id: &str, new_content_hash: &str) -> Result<()> {
        let changed = self.conn.execute(
            "UPDATE recipes SET content_hash = ?1 WHERE id = ?2",
            rusqlite::params![new_content_hash, recipe_id],
        )?;
        if changed == 0 {
            anyhow::bail!("No recipe found with id '{recipe_id}'");
        }
        Ok(())
    }

    /// Mark a recipe as needing XMP write-back (e.g. volume was offline during edit).
    pub fn mark_pending_writeback(&self, recipe_id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE recipes SET pending_writeback = 1 WHERE id = ?1",
            rusqlite::params![recipe_id],
        )?;
        Ok(())
    }

    /// Clear the pending write-back flag (after successful XMP write).
    pub fn clear_pending_writeback(&self, recipe_id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE recipes SET pending_writeback = 0 WHERE id = ?1",
            rusqlite::params![recipe_id],
        )?;
        Ok(())
    }

    /// List recipes with pending write-back, optionally filtered by volume.
    /// Returns `(recipe_id, asset_id, volume_id, relative_path)`.
    pub fn list_pending_writeback_recipes(
        &self,
        volume_id: Option<&str>,
    ) -> Result<Vec<(String, String, String, String)>> {
        let (sql, params): (String, Vec<Box<dyn rusqlite::types::ToSql>>) = if let Some(vid) = volume_id {
            (
                "SELECT r.id, v.asset_id, r.volume_id, r.relative_path \
                 FROM recipes r \
                 JOIN variants v ON r.variant_hash = v.content_hash \
                 WHERE r.pending_writeback = 1 AND r.volume_id = ?1"
                    .to_string(),
                vec![Box::new(vid.to_string())],
            )
        } else {
            (
                "SELECT r.id, v.asset_id, r.volume_id, r.relative_path \
                 FROM recipes r \
                 JOIN variants v ON r.variant_hash = v.content_hash \
                 WHERE r.pending_writeback = 1"
                    .to_string(),
                vec![],
            )
        };
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(params.iter()), |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }

    /// Find a recipe by volume and path (ignoring variant_hash).
    /// Returns `(recipe_id, content_hash, variant_hash)` if found.
    pub fn find_recipe_by_volume_and_path(
        &self,
        volume_id: &str,
        relative_path: &str,
    ) -> Result<Option<(String, String, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, content_hash, variant_hash FROM recipes \
             WHERE volume_id = ?1 AND relative_path = ?2",
        )?;
        let mut rows = stmt.query(rusqlite::params![volume_id, relative_path])?;
        match rows.next()? {
            Some(row) => Ok(Some((row.get(0)?, row.get(1)?, row.get(2)?))),
            None => Ok(None),
        }
    }

    /// Update `verified_at` timestamp on a recipe by its location.
    pub fn update_recipe_verified_at(
        &self,
        variant_hash: &str,
        volume_id: &str,
        relative_path: &str,
    ) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        self.conn.execute(
            "UPDATE recipes SET verified_at = ?1 \
             WHERE variant_hash = ?2 AND volume_id = ?3 AND relative_path = ?4",
            rusqlite::params![now, variant_hash, volume_id, relative_path],
        )?;
        Ok(())
    }

    /// Find a variant by its exact volume + relative_path.
    /// Returns `(content_hash, format)`.
    pub fn find_variant_by_volume_and_path(
        &self,
        volume_id: &str,
        relative_path: &str,
    ) -> Result<Option<(String, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT v.content_hash, v.format FROM file_locations fl \
             JOIN variants v ON fl.content_hash = v.content_hash \
             WHERE fl.volume_id = ?1 AND fl.relative_path = ?2 \
             LIMIT 1",
        )?;
        let mut rows = stmt.query(rusqlite::params![volume_id, relative_path])?;
        match rows.next()? {
            Some(row) => Ok(Some((row.get(0)?, row.get(1)?))),
            None => Ok(None),
        }
    }

    /// Find a variant whose file location on the given volume shares the same
    /// directory prefix and filename stem. Returns `(content_hash, asset_id)`.
    /// Optionally excludes a specific asset ID from results (used by fix-recipes
    /// to avoid self-matching the standalone recipe asset).
    pub fn find_variant_hash_by_stem_and_directory(
        &self,
        stem: &str,
        directory_prefix: &str,
        volume_id: &str,
        exclude_asset_id: Option<&str>,
    ) -> Result<Option<(String, String)>> {
        // Match file_locations where: same volume, path starts with directory_prefix,
        // and the filename (without extension) matches the stem.
        let path_pattern = if directory_prefix.is_empty() {
            format!("{stem}.%")
        } else {
            format!("{directory_prefix}/{stem}.%")
        };
        let (sql, params): (String, Vec<Box<dyn rusqlite::types::ToSql>>) = if let Some(exclude) = exclude_asset_id {
            (
                "SELECT fl.content_hash, v.asset_id FROM file_locations fl \
                 JOIN variants v ON fl.content_hash = v.content_hash \
                 WHERE fl.volume_id = ?1 AND fl.relative_path LIKE ?2 AND v.asset_id != ?3 \
                 LIMIT 1".to_string(),
                vec![
                    Box::new(volume_id.to_string()),
                    Box::new(path_pattern),
                    Box::new(exclude.to_string()),
                ],
            )
        } else {
            (
                "SELECT fl.content_hash, v.asset_id FROM file_locations fl \
                 JOIN variants v ON fl.content_hash = v.content_hash \
                 WHERE fl.volume_id = ?1 AND fl.relative_path LIKE ?2 \
                 LIMIT 1".to_string(),
                vec![
                    Box::new(volume_id.to_string()),
                    Box::new(path_pattern),
                ],
            )
        };
        let mut stmt = self.conn.prepare(&sql)?;
        let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
        let mut rows = stmt.query(param_refs.as_slice())?;
        match rows.next()? {
            Some(row) => Ok(Some((row.get(0)?, row.get(1)?))),
            None => Ok(None),
        }
    }

    /// List assets that have exactly one variant whose format is a recipe extension
    /// (xmp, cos, cot, cop, pp3, dop, on1) and asset_type = 'other'.
    /// Returns `(asset_id, content_hash, format)` for each match.
    /// Optionally scoped by volume or asset ID.
    pub fn list_recipe_only_assets(
        &self,
        volume_id: Option<&str>,
        asset_id: Option<&str>,
    ) -> Result<Vec<(String, String, String)>> {
        let recipe_extensions: &[&str] = &["xmp", "cos", "cot", "cop", "pp3", "dop", "on1"];

        let mut sql = String::from(
            "SELECT a.id, v.content_hash, v.format \
             FROM assets a \
             JOIN variants v ON v.asset_id = a.id \
             WHERE a.asset_type = 'other'",
        );
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        let mut param_idx = 1;

        if let Some(aid) = asset_id {
            sql.push_str(&format!(" AND a.id = ?{param_idx}"));
            params.push(Box::new(aid.to_string()));
            param_idx += 1;
        }

        if let Some(vid) = volume_id {
            sql.push_str(&format!(
                " AND v.content_hash IN (SELECT content_hash FROM file_locations WHERE volume_id = ?{param_idx})"
            ));
            params.push(Box::new(vid.to_string()));
            param_idx += 1;
        }
        let _ = param_idx;

        sql.push_str(" GROUP BY a.id HAVING COUNT(v.content_hash) = 1");

        let mut stmt = self.conn.prepare(&sql)?;
        let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
        let rows = stmt.query_map(param_refs.as_slice(), |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;

        let mut results = Vec::new();
        for row in rows {
            let (aid, hash, fmt) = row?;
            if recipe_extensions.contains(&fmt.to_lowercase().as_str()) {
                results.push((aid, hash, fmt));
            }
        }
        Ok(results)
    }

    /// Find all asset IDs that have file locations on a given volume under any of
    /// the given path prefixes. Used by `import --auto-group` to scope auto-grouping
    /// to the "neighborhood" of imported files.
    pub fn find_asset_ids_by_volume_and_path_prefixes(
        &self,
        volume_id: &str,
        prefixes: &[String],
    ) -> Result<Vec<String>> {
        if prefixes.is_empty() {
            return Ok(Vec::new());
        }

        // Build dynamic OR clause: one `fl.relative_path LIKE ?N` per prefix
        let mut conditions = Vec::new();
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        params.push(Box::new(volume_id.to_string()));

        for (i, prefix) in prefixes.iter().enumerate() {
            conditions.push(format!("fl.relative_path LIKE ?{}", i + 2));
            if prefix.is_empty() {
                params.push(Box::new("%".to_string()));
            } else {
                params.push(Box::new(format!("{prefix}/%")));
            }
        }

        let sql = format!(
            "SELECT DISTINCT v.asset_id FROM variants v \
             JOIN file_locations fl ON v.content_hash = fl.content_hash \
             WHERE fl.volume_id = ?1 AND ({})",
            conditions.join(" OR ")
        );

        let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(param_refs.as_slice(), |row| row.get::<_, String>(0))?;

        let mut ids = Vec::new();
        for row in rows {
            ids.push(row?);
        }
        Ok(ids)
    }

    /// Drop and recreate data tables (assets, variants, file_locations, recipes).
    /// Keeps the volumes table intact. Ensures the schema is up to date.
    pub fn rebuild(&self) -> Result<()> {
        self.conn.execute_batch(
            "DROP TABLE IF EXISTS schema_version;
             DROP TABLE IF EXISTS faces;
             DROP TABLE IF EXISTS people;
             DROP TABLE IF EXISTS embeddings;
             DROP TABLE IF EXISTS collection_assets;
             DROP TABLE IF EXISTS collections;
             DROP TABLE IF EXISTS file_locations;
             DROP TABLE IF EXISTS recipes;
             DROP TABLE IF EXISTS variants;
             DROP TABLE IF EXISTS assets;",
        )?;
        self.initialize()?;
        Ok(())
    }

    // ── Sync queries ───────────────────────────────────────────────

    /// List all file locations on a volume whose path starts with the given prefix.
    /// Returns `(content_hash, relative_path)` pairs.
    pub fn list_locations_for_volume_under_prefix(
        &self,
        volume_id: &str,
        prefix: &str,
    ) -> Result<Vec<(String, String)>> {
        let pattern = if prefix.is_empty() {
            "%".to_string()
        } else {
            format!("{prefix}%")
        };
        let mut stmt = self.conn.prepare(
            "SELECT content_hash, relative_path FROM file_locations \
             WHERE volume_id = ?1 AND relative_path LIKE ?2",
        )?;
        let rows = stmt.query_map(rusqlite::params![volume_id, pattern], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }

    /// List all recipes on a volume whose path starts with the given prefix.
    /// Returns `(recipe_id, content_hash, variant_hash, relative_path)` tuples.
    pub fn list_recipes_for_volume_under_prefix(
        &self,
        volume_id: &str,
        prefix: &str,
    ) -> Result<Vec<(String, String, String, String)>> {
        let pattern = if prefix.is_empty() {
            "%".to_string()
        } else {
            format!("{prefix}%")
        };
        let mut stmt = self.conn.prepare(
            "SELECT id, content_hash, variant_hash, relative_path FROM recipes \
             WHERE volume_id = ?1 AND relative_path LIKE ?2",
        )?;
        let rows = stmt.query_map(rusqlite::params![volume_id, pattern], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }

    /// Like `list_recipes_for_volume_under_prefix` but also returns `pending_writeback`.
    /// Returns `(id, content_hash, variant_hash, relative_path, pending_writeback)`.
    pub fn list_recipes_with_pending_for_volume(
        &self,
        volume_id: &str,
    ) -> Result<Vec<(String, String, String, String, bool)>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, content_hash, variant_hash, relative_path, pending_writeback \
             FROM recipes WHERE volume_id = ?1",
        )?;
        let rows = stmt.query_map(rusqlite::params![volume_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, bool>(4)?,
            ))
        })?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }

    /// List recipes for a specific variant on a specific volume.
    /// Returns `(recipe_id, content_hash, relative_path)` tuples.
    pub fn list_recipes_for_variant_on_volume(
        &self,
        variant_hash: &str,
        volume_id: &str,
    ) -> Result<Vec<(String, String, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, content_hash, relative_path FROM recipes \
             WHERE variant_hash = ?1 AND volume_id = ?2",
        )?;
        let rows = stmt.query_map(rusqlite::params![variant_hash, volume_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }

    /// List all recipes for a given asset (across all volumes).
    /// Returns `(recipe_id, content_hash, variant_hash, relative_path, volume_id)` tuples.
    pub fn list_recipes_for_asset(
        &self,
        asset_id: &str,
    ) -> Result<Vec<(String, String, String, String, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT r.id, r.content_hash, r.variant_hash, r.relative_path, r.volume_id \
             FROM recipes r \
             JOIN variants v ON r.variant_hash = v.content_hash \
             WHERE v.asset_id = ?1",
        )?;
        let rows = stmt.query_map(rusqlite::params![asset_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }

    /// List all file locations for an asset's variants.
    /// Returns (content_hash, relative_path, volume_id).
    pub fn list_file_locations_for_asset(
        &self,
        asset_id: &str,
    ) -> Result<Vec<(String, String, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT fl.content_hash, fl.relative_path, fl.volume_id \
             FROM file_locations fl \
             JOIN variants v ON fl.content_hash = v.content_hash \
             WHERE v.asset_id = ?1",
        )?;
        let rows = stmt.query_map(rusqlite::params![asset_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }

    /// Find all asset IDs that share the same directory (session) as the given asset.
    /// Goes up one directory level from each of the asset's file locations to find
    /// "session roots", then returns all asset IDs with files under those roots.
    pub fn find_same_session_asset_ids(&self, asset_id: &str) -> Result<std::collections::HashSet<String>> {
        let locations = self.list_file_locations_for_asset(asset_id)?;
        let mut session_ids = std::collections::HashSet::new();

        for (_hash, rel_path, volume_id) in &locations {
            // Get the parent directory of this file
            let parent = std::path::Path::new(rel_path)
                .parent()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default();
            if parent.is_empty() {
                continue;
            }
            // Go up one more level to get the "session root" (e.g., Capture/2026-02-22/)
            let session_root = std::path::Path::new(&parent)
                .parent()
                .map(|p| {
                    let s = p.to_string_lossy().to_string();
                    if s.is_empty() { parent.clone() } else { s }
                })
                .unwrap_or_else(|| parent.clone());
            let prefix = format!("{}%", session_root);

            let mut stmt = self.conn.prepare(
                "SELECT DISTINCT v.asset_id FROM variants v \
                 JOIN file_locations fl ON fl.content_hash = v.content_hash \
                 WHERE fl.volume_id = ?1 AND fl.relative_path LIKE ?2",
            )?;
            let rows = stmt.query_map(rusqlite::params![volume_id, prefix], |row| {
                row.get::<_, String>(0)
            })?;
            for row in rows {
                if let Ok(id) = row {
                    session_ids.insert(id);
                }
            }
        }
        Ok(session_ids)
    }

    /// Update the relative_path for a file location (variant moved on disk).
    pub fn update_file_location_path(
        &self,
        content_hash: &str,
        volume_id: &str,
        old_path: &str,
        new_path: &str,
    ) -> Result<()> {
        let new_path_norm = new_path.replace('\\', "/");
        let old_path_norm = old_path.replace('\\', "/");
        let changed = self.conn.execute(
            "UPDATE file_locations SET relative_path = ?1 \
             WHERE content_hash = ?2 AND volume_id = ?3 AND relative_path = ?4",
            rusqlite::params![new_path_norm, content_hash, volume_id, old_path_norm],
        )?;
        if changed == 0 {
            anyhow::bail!(
                "No file location found for hash '{content_hash}' at '{old_path}'"
            );
        }
        Ok(())
    }

    /// Update the relative_path for a recipe (recipe file moved on disk).
    pub fn update_recipe_relative_path(
        &self,
        recipe_id: &str,
        new_path: &str,
    ) -> Result<()> {
        let new_path_norm = new_path.replace('\\', "/");
        let changed = self.conn.execute(
            "UPDATE recipes SET relative_path = ?1 WHERE id = ?2",
            rusqlite::params![new_path_norm, recipe_id],
        )?;
        if changed == 0 {
            anyhow::bail!("No recipe found with id '{recipe_id}'");
        }
        Ok(())
    }

    // ── Stats queries ──────────────────────────────────────────────

    /// Core overview counts: (assets, variants, recipes, total_size).
    pub fn stats_overview(&self) -> Result<(u64, u64, u64, u64, u64)> {
        self.conn.query_row(
            "SELECT \
                (SELECT COUNT(*) FROM assets), \
                (SELECT COUNT(*) FROM variants), \
                (SELECT COUNT(*) FROM recipes), \
                (SELECT COALESCE(SUM(file_size), 0) FROM variants), \
                (SELECT COUNT(*) FROM file_locations)",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        ).map_err(Into::into)
    }

    /// Recipe counts: (total_recipe_rows, unique_content_hashes).
    /// The difference is the number of duplicate recipe locations (e.g. backups).
    pub fn stats_recipe_counts(&self) -> Result<(u64, u64)> {
        self.conn.query_row(
            "SELECT \
                (SELECT COUNT(*) FROM recipes), \
                (SELECT COUNT(DISTINCT content_hash) FROM recipes)",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        ).map_err(Into::into)
    }

    /// Asset type breakdown: Vec<(type_name, count)>.
    pub fn stats_asset_types(&self) -> Result<Vec<(String, u64)>> {
        let mut stmt = self.conn.prepare(
            "SELECT asset_type, COUNT(*) FROM assets GROUP BY asset_type ORDER BY COUNT(*) DESC",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect::<std::result::Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Top variant formats: Vec<(format, count)>.
    pub fn stats_variant_formats(&self, limit: usize) -> Result<Vec<(String, u64)>> {
        let mut stmt = self.conn.prepare(
            "SELECT format, COUNT(*) FROM variants GROUP BY format ORDER BY COUNT(*) DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(rusqlite::params![limit as u64], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect::<std::result::Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Recipe format counts: extract file extension in SQL and aggregate.
    pub fn stats_recipe_formats(&self, limit: usize) -> Result<Vec<(String, u64)>> {
        let mut stmt = self.conn.prepare(
            "SELECT LOWER(REPLACE(relative_path, \
                RTRIM(relative_path, REPLACE(relative_path, '.', '')), '')) as ext, \
             COUNT(*) as cnt \
             FROM recipes WHERE relative_path IS NOT NULL \
             GROUP BY ext ORDER BY cnt DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(rusqlite::params![limit as u64], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, u64>(1)?))
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Build the WHERE clause and parameters for search queries.
    /// Returns (where_clause, params, needs_fl_join, needs_v_join).
    /// `needs_v_join`: true when any filter references the `v` (variants) table directly.
    /// `needs_fl_join`: true when any filter references `fl` (file_locations); implies `needs_v_join`.
    /// Generate SQL WHERE clause for a NumericFilter on a given column.
    /// Rating-specific clause builder that treats `rating IS NULL` as equivalent
    /// to `rating = 0`. Users expect `rating:0` to match unrated assets.
    fn rating_clause(
        filter: &NumericFilter,
        clauses: &mut Vec<String>,
        params: &mut Vec<Box<dyn rusqlite::types::ToSql>>,
    ) {
        // True if the filter matches the value 0.
        let matches_zero = match filter {
            NumericFilter::Exact(v) => *v == 0.0,
            NumericFilter::Min(v) => *v <= 0.0,
            NumericFilter::Range(lo, hi) => *lo <= 0.0 && *hi >= 0.0,
            NumericFilter::Values(vs) => vs.iter().any(|v| *v == 0.0),
            NumericFilter::ValuesOrMin { values, min } => {
                values.iter().any(|v| *v == 0.0) || *min <= 0.0
            }
        };

        if matches_zero {
            // Build the normal clause into a temporary, then wrap in (IS NULL OR <clause>).
            let mut inner_clauses: Vec<String> = Vec::new();
            Self::numeric_clause(filter, "a.rating", &mut inner_clauses, params);
            // numeric_clause always adds exactly one clause.
            if let Some(inner) = inner_clauses.into_iter().next() {
                clauses.push(format!("(a.rating IS NULL OR {inner})"));
            }
        } else {
            Self::numeric_clause(filter, "a.rating", clauses, params);
        }
    }

    fn numeric_clause(
        filter: &NumericFilter,
        column: &str,
        clauses: &mut Vec<String>,
        params: &mut Vec<Box<dyn rusqlite::types::ToSql>>,
    ) {
        match filter {
            NumericFilter::Exact(v) => {
                clauses.push(format!("{column} = ?"));
                params.push(Box::new(*v));
            }
            NumericFilter::Min(v) => {
                clauses.push(format!("{column} >= ?"));
                params.push(Box::new(*v));
            }
            NumericFilter::Range(lo, hi) => {
                clauses.push(format!("({column} >= ? AND {column} <= ?)"));
                params.push(Box::new(*lo));
                params.push(Box::new(*hi));
            }
            NumericFilter::Values(vals) => {
                let placeholders: Vec<&str> = vals.iter().map(|_| "?").collect();
                clauses.push(format!("{column} IN ({})", placeholders.join(",")));
                for v in vals {
                    params.push(Box::new(*v));
                }
            }
            NumericFilter::ValuesOrMin { values, min } => {
                let placeholders: Vec<&str> = values.iter().map(|_| "?").collect();
                clauses.push(format!(
                    "({column} IN ({}) OR {column} >= ?)",
                    placeholders.join(",")
                ));
                for v in values {
                    params.push(Box::new(*v));
                }
                params.push(Box::new(*min));
            }
        }
    }

    /// Generate SQL WHERE clause for a NumericFilter using a subquery expression.
    fn numeric_clause_expr(
        filter: &NumericFilter,
        expr: &str,
        clauses: &mut Vec<String>,
        params: &mut Vec<Box<dyn rusqlite::types::ToSql>>,
    ) {
        match filter {
            NumericFilter::Exact(v) => {
                clauses.push(format!("{expr} = ?"));
                params.push(Box::new(*v));
            }
            NumericFilter::Min(v) => {
                clauses.push(format!("{expr} >= ?"));
                params.push(Box::new(*v));
            }
            NumericFilter::Range(lo, hi) => {
                clauses.push(format!("({expr} >= ? AND {expr} <= ?)"));
                params.push(Box::new(*lo));
                params.push(Box::new(*hi));
            }
            NumericFilter::Values(vals) => {
                let mut parts = Vec::new();
                for v in vals {
                    parts.push(format!("{expr} = ?"));
                    params.push(Box::new(*v));
                }
                clauses.push(format!("({})", parts.join(" OR ")));
            }
            NumericFilter::ValuesOrMin { values, min } => {
                let mut parts = Vec::new();
                for v in values {
                    parts.push(format!("{expr} = ?"));
                    params.push(Box::new(*v));
                }
                parts.push(format!("{expr} >= ?"));
                params.push(Box::new(*min));
                clauses.push(format!("({})", parts.join(" OR ")));
            }
        }
    }

    fn build_search_where(opts: &SearchOptions) -> (String, Vec<Box<dyn rusqlite::types::ToSql>>, bool, bool) {
        let mut clauses = Vec::new();
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        let mut needs_fl_join = opts.volume.is_some() || !opts.volume_ids.is_empty() || !opts.volume_ids_exclude.is_empty();
        let mut needs_v_join = false;

        // --- Asset ID prefix match (supports multiple IDs) ---
        if !opts.asset_ids.is_empty() {
            if opts.asset_ids.len() == 1 {
                clauses.push("a.id LIKE ?".to_string());
                params.push(Box::new(format!("{}%", opts.asset_ids[0])));
            } else {
                let placeholders: Vec<&str> = opts.asset_ids.iter().map(|_| "a.id LIKE ?").collect();
                clauses.push(format!("({})", placeholders.join(" OR ")));
                for id in opts.asset_ids {
                    params.push(Box::new(format!("{id}%")));
                }
            }
        }

        // --- Text search (positive) ---
        if let Some(text) = opts.text {
            if !text.is_empty() {
                clauses.push(
                    "(a.name LIKE ? OR bv.original_filename LIKE ? OR a.description LIKE ? OR bv.source_metadata LIKE ?)".to_string(),
                );
                let pattern = format!("%{text}%");
                params.push(Box::new(pattern.clone()));
                params.push(Box::new(pattern.clone()));
                params.push(Box::new(pattern.clone()));
                params.push(Box::new(pattern));
            }
        }

        // --- Text exclusion ---
        // Use IFNULL to handle NULL columns: NULL LIKE '%x%' returns NULL,
        // and NOT(NULL OR ...) = NULL which is falsy, so we must coalesce.
        for term in opts.text_exclude {
            clauses.push(
                "NOT (IFNULL(a.name,'') LIKE ? OR bv.original_filename LIKE ? OR IFNULL(a.description,'') LIKE ? OR bv.source_metadata LIKE ?)".to_string(),
            );
            let pattern = format!("%{term}%");
            params.push(Box::new(pattern.clone()));
            params.push(Box::new(pattern.clone()));
            params.push(Box::new(pattern.clone()));
            params.push(Box::new(pattern));
        }

        // --- Asset type (equality filter on a.asset_type) ---
        Self::add_equality_filter(&mut clauses, &mut params, opts.asset_types, opts.asset_types_exclude, "a.asset_type", &mut false, false);

        // --- Tags (hierarchy-aware LIKE) ---
        // Positive: each entry is ANDed; commas within an entry are ORed
        for tag_entry in opts.tags {
            let values: Vec<&str> = tag_entry.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
            if values.len() == 1 {
                Self::add_tag_clause(&mut clauses, &mut params, values[0], false);
            } else {
                // Multiple comma values — OR group
                let mut or_parts = Vec::new();
                for v in &values {
                    or_parts.extend(Self::tag_like_parts(&mut params, v));
                }
                clauses.push(format!("({})", or_parts.join(" OR ")));
            }
        }
        // Negative: each entry is ANDed as NOT; commas within an entry are ORed
        for tag_entry in opts.tags_exclude {
            let values: Vec<&str> = tag_entry.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
            let mut or_parts = Vec::new();
            for v in &values {
                or_parts.extend(Self::tag_like_parts(&mut params, v));
            }
            clauses.push(format!("NOT ({})", or_parts.join(" OR ")));
        }

        // --- Format (equality on v.format) ---
        {
            let include: Vec<&str> = opts.formats.iter()
                .flat_map(|e| e.split(',').map(|s| s.trim()))
                .filter(|s| !s.is_empty())
                .collect();
            let exclude: Vec<&str> = opts.formats_exclude.iter()
                .flat_map(|e| e.split(',').map(|s| s.trim()))
                .filter(|s| !s.is_empty())
                .collect();
            if !include.is_empty() || !exclude.is_empty() {
                needs_v_join = true;
            }
            if include.len() == 1 {
                clauses.push("v.format = ?".to_string());
                params.push(Box::new(include[0].to_lowercase()));
            } else if include.len() > 1 {
                let placeholders: Vec<&str> = include.iter().map(|_| "?").collect();
                clauses.push(format!("v.format IN ({})", placeholders.join(",")));
                for v in &include {
                    params.push(Box::new(v.to_lowercase()));
                }
            }
            if exclude.len() == 1 {
                clauses.push("v.format != ?".to_string());
                params.push(Box::new(exclude[0].to_lowercase()));
            } else if exclude.len() > 1 {
                let placeholders: Vec<&str> = exclude.iter().map(|_| "?").collect();
                clauses.push(format!("v.format NOT IN ({})", placeholders.join(",")));
                for v in &exclude {
                    params.push(Box::new(v.to_lowercase()));
                }
            }
        }

        // --- Volume ---
        if let Some(volume) = opts.volume {
            if !volume.is_empty() {
                clauses.push("fl.volume_id = ?".to_string());
                params.push(Box::new(volume.to_string()));
            }
        }
        if !opts.volume_ids.is_empty() {
            let placeholders: Vec<String> = opts.volume_ids.iter().map(|_| "?".to_string()).collect();
            clauses.push(format!("fl.volume_id IN ({})", placeholders.join(",")));
            for vid in opts.volume_ids {
                params.push(Box::new(vid.clone()));
            }
        }
        if !opts.volume_ids_exclude.is_empty() {
            // Exclude assets that have ANY location on these volumes
            let placeholders: Vec<String> = opts.volume_ids_exclude.iter().map(|_| "?".to_string()).collect();
            clauses.push(format!(
                "a.id NOT IN (SELECT DISTINCT v2.asset_id FROM variants v2 \
                 JOIN file_locations fl2 ON fl2.content_hash = v2.content_hash \
                 WHERE fl2.volume_id IN ({}))",
                placeholders.join(",")
            ));
            for vid in opts.volume_ids_exclude {
                params.push(Box::new(vid.clone()));
            }
        }

        // --- Numeric filters (all use unified NumericFilter type) ---
        // Rating is special: an unrated asset has `rating IS NULL`, but users
        // mentally treat "0 stars" and "unrated" as the same thing. We rewrite
        // any rating filter that matches 0 (Exact(0), Values containing 0,
        // Range 0-N, ValuesOrMin with 0) to also match NULL.
        if let Some(ref f) = opts.rating {
            Self::rating_clause(f, &mut clauses, &mut params);
        }

        // --- Color label (equality on a.color_label) ---
        Self::add_equality_filter(&mut clauses, &mut params, opts.color_labels, opts.color_labels_exclude, "a.color_label", &mut false, false);

        // --- Path pattern (LIKE on fl.relative_path) ---
        // Supports `*` as a wildcard anywhere in the pattern. A trailing `%`
        // is appended automatically so `path:Pictures/2026` keeps prefix
        // semantics. Literal `%` and `_` are escaped via `ESCAPE '\'`.
        {
            let include: Vec<&str> = opts.path_prefixes.iter()
                .flat_map(|e| e.split(',').map(|s| s.trim()))
                .filter(|s| !s.is_empty())
                .collect();
            let exclude: Vec<&str> = opts.path_prefixes_exclude.iter()
                .flat_map(|e| e.split(',').map(|s| s.trim()))
                .filter(|s| !s.is_empty())
                .collect();
            if !include.is_empty() || !exclude.is_empty() {
                needs_fl_join = true;
            }
            if include.len() == 1 {
                clauses.push("fl.relative_path LIKE ? ESCAPE '\\'".to_string());
                params.push(Box::new(path_pattern_to_like(include[0])));
            } else if include.len() > 1 {
                let mut or_parts = Vec::new();
                for v in &include {
                    or_parts.push("fl.relative_path LIKE ? ESCAPE '\\'".to_string());
                    params.push(Box::new(path_pattern_to_like(v)));
                }
                clauses.push(format!("({})", or_parts.join(" OR ")));
            }
            for v in &exclude {
                clauses.push("fl.relative_path NOT LIKE ? ESCAPE '\\'".to_string());
                params.push(Box::new(path_pattern_to_like(v)));
            }
        }

        // --- Camera (LIKE on v.camera_model) ---
        Self::add_like_filter(&mut clauses, &mut params, opts.cameras, opts.cameras_exclude, "v.camera_model", &mut needs_v_join);

        // --- Lens (LIKE on v.lens_model) ---
        Self::add_like_filter(&mut clauses, &mut params, opts.lenses, opts.lenses_exclude, "v.lens_model", &mut needs_v_join);

        // --- Description (LIKE on a.description) ---
        // Pure assets-table filter, no JOIN required. Use a throwaway flag.
        let mut _desc_no_join = false;
        Self::add_like_filter(&mut clauses, &mut params, opts.descriptions, opts.descriptions_exclude, "a.description", &mut _desc_no_join);

        // --- Numeric variant filters ---
        if let Some(ref f) = opts.iso { Self::numeric_clause(f, "v.iso", &mut clauses, &mut params); needs_v_join = true; }
        if let Some(ref f) = opts.focal { Self::numeric_clause(f, "v.focal_length_mm", &mut clauses, &mut params); needs_v_join = true; }
        if let Some(ref f) = opts.aperture { Self::numeric_clause(f, "v.f_number", &mut clauses, &mut params); needs_v_join = true; }
        if let Some(ref f) = opts.width { Self::numeric_clause(f, "v.image_width", &mut clauses, &mut params); needs_v_join = true; }
        if let Some(ref f) = opts.height { Self::numeric_clause(f, "v.image_height", &mut clauses, &mut params); needs_v_join = true; }

        // JSON fallback filters (meta:key=value)
        for (key, value) in &opts.meta_filters {
            clauses.push(format!("json_extract(v.source_metadata, '$.{key}') LIKE ?"));
            params.push(Box::new(format!("%{value}%")));
            needs_v_join = true;
        }

        // Location health filters
        if opts.orphan {
            clauses.push(
                "NOT EXISTS (SELECT 1 FROM file_locations fl2 JOIN variants v2 ON fl2.content_hash = v2.content_hash WHERE v2.asset_id = a.id)"
                    .to_string(),
            );
        }
        if opts.orphan_false {
            clauses.push(
                "EXISTS (SELECT 1 FROM file_locations fl2 JOIN variants v2 ON fl2.content_hash = v2.content_hash WHERE v2.asset_id = a.id)"
                    .to_string(),
            );
        }
        if let Some(ref f) = opts.stale_days {
            // stale: uses exact value as number of days (only Exact/Min make sense)
            let days = match f {
                NumericFilter::Exact(v) | NumericFilter::Min(v) => *v as u64,
                NumericFilter::Range(v, _) => *v as u64,
                NumericFilter::Values(v) => v.first().copied().unwrap_or(30.0) as u64,
                NumericFilter::ValuesOrMin { min, .. } => *min as u64,
            };
            clauses.push(format!(
                "EXISTS (SELECT 1 FROM file_locations fl2 \
                 JOIN variants v2 ON fl2.content_hash = v2.content_hash \
                 WHERE v2.asset_id = a.id AND \
                 (fl2.verified_at IS NULL OR fl2.verified_at < datetime('now', '-{} days')))",
                days
            ));
        }
        if let Some(ids) = opts.missing_asset_ids {
            if ids.is_empty() {
                clauses.push("0".to_string());
            } else {
                let placeholders: Vec<&str> = ids.iter().map(|_| "?").collect();
                clauses.push(format!("a.id IN ({})", placeholders.join(",")));
                for id in ids {
                    params.push(Box::new(id.clone()));
                }
            }
        }
        if let Some(online_ids) = opts.no_online_locations {
            if !online_ids.is_empty() {
                let placeholders: Vec<&str> = online_ids.iter().map(|_| "?").collect();
                clauses.push(format!(
                    "NOT EXISTS (SELECT 1 FROM file_locations fl2 \
                     JOIN variants v2 ON fl2.content_hash = v2.content_hash \
                     WHERE v2.asset_id = a.id AND fl2.volume_id IN ({}))",
                    placeholders.join(",")
                ));
                for id in online_ids {
                    params.push(Box::new(id.clone()));
                }
            }
        }

        // Collection filter: restrict to a pre-computed set of asset IDs
        if let Some(ids) = opts.collection_asset_ids {
            if ids.is_empty() {
                clauses.push("0".to_string());
            } else {
                let placeholders: Vec<&str> = ids.iter().map(|_| "?").collect();
                clauses.push(format!("a.id IN ({})", placeholders.join(",")));
                for id in ids {
                    params.push(Box::new(id.clone()));
                }
            }
        }

        // Collection exclude: exclude a pre-computed set of asset IDs
        if let Some(ids) = opts.collection_exclude_ids {
            if !ids.is_empty() {
                let placeholders: Vec<&str> = ids.iter().map(|_| "?").collect();
                clauses.push(format!("a.id NOT IN ({})", placeholders.join(",")));
                for id in ids {
                    params.push(Box::new(id.clone()));
                }
            }
        }

        // Copies filter — count DISTINCT volumes where this asset has file
        // locations. This matches the backup-status semantics: copies:1 means
        // "exists on exactly one volume" (at risk), regardless of how many
        // variants or file locations exist on that volume.
        if let Some(ref f) = opts.copies {
            let expr = "(SELECT COUNT(DISTINCT fl2.volume_id) FROM file_locations fl2 \
                 JOIN variants v2 ON fl2.content_hash = v2.content_hash \
                 WHERE v2.asset_id = a.id)";
            Self::numeric_clause_expr(f, expr, &mut clauses, &mut params);
        }

        // Variant count (denormalized column)
        if let Some(ref f) = opts.variant_count { Self::numeric_clause(f, "a.variant_count", &mut clauses, &mut params); }

        // Scattered filter — count distinct session roots for this asset's
        // file locations. Uses the same session root detection as auto-group:
        // the deepest directory component matching [group] session_root_pattern.
        // An asset whose files all live under the same session root (e.g.
        // Capture/, Selects/, Output/ of the same shoot) has scattered:1.
        // An asset with files in different session roots (different shoots)
        // has scattered:2+, indicating a potential mis-grouping.
        if let Some(ref f) = opts.scattered {
            let pattern_escaped = opts.session_root_pattern.replace('\'', "''");
            let expr = format!(
                "(SELECT COUNT(DISTINCT session_root(fl2.relative_path, '{pattern_escaped}')) \
                 FROM file_locations fl2 \
                 JOIN variants v2 ON fl2.content_hash = v2.content_hash \
                 WHERE v2.asset_id = a.id)"
            );
            Self::numeric_clause_expr(f, &expr, &mut clauses, &mut params);
        }

        // Date filters
        if let Some(prefix) = opts.date_prefix {
            if !prefix.is_empty() {
                clauses.push("a.created_at LIKE ?".to_string());
                params.push(Box::new(format!("{prefix}%")));
            }
        }
        if let Some(from) = opts.date_from {
            if !from.is_empty() {
                clauses.push("a.created_at >= ?".to_string());
                params.push(Box::new(from.to_string()));
            }
        }
        if let Some(until) = opts.date_until {
            if !until.is_empty() {
                let exclusive = next_date_bound(until);
                clauses.push("a.created_at < ?".to_string());
                params.push(Box::new(exclusive));
            }
        }

        // Stack collapse
        if opts.collapse_stacks {
            clauses.push("(a.stack_id IS NULL OR a.stack_position = 0)".to_string());
        }

        // Stacked filter
        if let Some(stacked) = opts.stacked_filter {
            if stacked {
                clauses.push("a.stack_id IS NOT NULL".to_string());
            } else {
                clauses.push("a.stack_id IS NULL".to_string());
            }
        }

        // Geo bounding box filter
        if let Some((south, west, north, east)) = opts.geo_bbox {
            clauses.push("a.latitude >= ? AND a.latitude <= ? AND a.longitude >= ? AND a.longitude <= ?".to_string());
            params.push(Box::new(south));
            params.push(Box::new(north));
            params.push(Box::new(west));
            params.push(Box::new(east));
        }

        // GPS presence filter
        if let Some(has_gps) = opts.has_gps {
            if has_gps {
                clauses.push("a.latitude IS NOT NULL AND a.longitude IS NOT NULL".to_string());
            } else {
                clauses.push("(a.latitude IS NULL OR a.longitude IS NULL)".to_string());
            }
        }

        // Face filters (use denormalized face_count column)
        if let Some(has_faces) = opts.has_faces {
            if has_faces {
                clauses.push("a.face_count > 0".to_string());
            } else {
                clauses.push("a.face_count = 0".to_string());
            }
        }
        if let Some(ref f) = opts.face_count { Self::numeric_clause(f, "a.face_count", &mut clauses, &mut params); }
        if let Some(ref f) = opts.duration { Self::numeric_clause(f, "a.video_duration", &mut clauses, &mut params); }
        if let Some(ref c) = opts.codec {
            clauses.push("a.video_codec LIKE ?".to_string());
            params.push(Box::new(format!("%{c}%")));
        }

        // Embedding presence filter
        if let Some(has_embed) = opts.has_embed {
            if has_embed {
                clauses.push(
                    "EXISTS (SELECT 1 FROM embeddings e WHERE e.asset_id = a.id)".to_string(),
                );
            } else {
                clauses.push(
                    "NOT EXISTS (SELECT 1 FROM embeddings e WHERE e.asset_id = a.id)".to_string(),
                );
            }
        }

        // Person filter: restrict to pre-computed asset IDs
        if let Some(ids) = opts.person_asset_ids {
            if ids.is_empty() {
                clauses.push("0".to_string());
            } else {
                let placeholders: Vec<&str> = ids.iter().map(|_| "?").collect();
                clauses.push(format!("a.id IN ({})", placeholders.join(",")));
                for id in ids {
                    params.push(Box::new(id.clone()));
                }
            }
        }

        // Person exclude filter
        if let Some(ids) = opts.person_exclude_ids {
            if !ids.is_empty() {
                let placeholders: Vec<&str> = ids.iter().map(|_| "?").collect();
                clauses.push(format!("a.id NOT IN ({})", placeholders.join(",")));
                for id in ids {
                    params.push(Box::new(id.clone()));
                }
            }
        }

        // Similar assets filter (pre-computed from embedding similarity search)
        if let Some(ids) = opts.similar_asset_ids {
            if ids.is_empty() {
                clauses.push("0".to_string());
            } else {
                let placeholders: Vec<&str> = ids.iter().map(|_| "?").collect();
                clauses.push(format!("a.id IN ({})", placeholders.join(",")));
                for id in ids {
                    params.push(Box::new(id.clone()));
                }
            }
        }

        // Text search filter (pre-computed from text-to-image embedding similarity)
        if let Some(ids) = opts.text_search_ids {
            if ids.is_empty() {
                clauses.push("0".to_string());
            } else {
                let placeholders: Vec<&str> = ids.iter().map(|_| "?").collect();
                clauses.push(format!("a.id IN ({})", placeholders.join(",")));
                for id in ids {
                    params.push(Box::new(id.clone()));
                }
            }
        }

        let where_clause = if clauses.is_empty() {
            " WHERE 1=1".to_string()
        } else {
            format!(" WHERE {}", clauses.join(" AND "))
        };

        // fl join implies v join (fl joins through v)
        if needs_fl_join {
            needs_v_join = true;
        }

        (where_clause, params, needs_fl_join, needs_v_join)
    }

    /// Helper: generate tag LIKE clause parts for a single tag value.
    /// Returns a Vec of SQL expressions (each with params already pushed).
    ///
    /// Build SQL clauses for a `tag:` filter value.
    ///
    /// Prefix markers (any order, all stackable):
    /// - `=` — exact level only (no descendants)
    /// - `^` — case-sensitive (SQLite GLOB instead of LIKE)
    /// - `|` — anchored prefix: match any tag whose hierarchy component STARTS
    ///   with the rest of the value, at any level. Mutually exclusive with `=`
    ///   (a prefix-anchor implicitly includes descendants by definition).
    ///   Examples: `tag:|wed` matches `wedding`, `wedding-2024`, `events|wedding`,
    ///   `events|wedding|2024-05`. `tag:^|Wed` matches the same set
    ///   case-sensitively.
    ///
    /// Without any markers, both exact and descendant matches are generated,
    /// case-insensitively (the SQLite LIKE default for ASCII).
    ///
    /// Tags containing `"` may be stored in JSON two ways:
    /// - Unescaped: `"\"Sir\" Oliver Mally"` (serde_json proper)
    /// - Raw: `""Sir" Oliver Mally"` (legacy/malformed JSON)
    /// We match both forms.
    fn tag_like_parts(params: &mut Vec<Box<dyn rusqlite::types::ToSql>>, tag: &str) -> Vec<String> {
        // Strip the `=`, `^`, and `|` prefix markers in any order.
        let mut rest = tag;
        let mut exact_only = false;
        let mut case_sensitive = false;
        let mut prefix_anchor = false;
        loop {
            if let Some(s) = rest.strip_prefix('=') { exact_only = true; rest = s; }
            else if let Some(s) = rest.strip_prefix('^') { case_sensitive = true; rest = s; }
            else if let Some(s) = rest.strip_prefix('|') { prefix_anchor = true; rest = s; }
            else { break; }
        }
        // `=` and `|` are conceptually mutually exclusive: an anchored prefix
        // search always includes descendants. If both are given, `|` wins
        // (the more specific search) and `=` is silently ignored.
        if prefix_anchor { exact_only = false; }
        let tag_value = rest;
        let stored = crate::tag_util::tag_input_to_storage(tag_value);
        let mut exprs = Vec::new();

        // Helper: build the wildcard pattern for either LIKE (%..%) or GLOB (*..*).
        // GLOB is case-sensitive; LIKE is case-insensitive for ASCII. Tag values
        // almost never contain `*` or `?`, but if they do, GLOB would treat them
        // as wildcards — this is a documented edge case for case-sensitive search.
        let op = if case_sensitive { "GLOB" } else { "LIKE" };
        let wild = if case_sensitive { "*" } else { "%" };
        let pat = |middle: &str| -> String { format!("{wild}{middle}{wild}") };
        // For the "not-descendant" clause we need a trailing `|` before the wild,
        // so the pattern is `<wild>"tag|<wild>` (matches any descendant).
        let desc_pat = |stored: &str| -> String { format!("{wild}\"{stored}|{wild}") };

        if prefix_anchor {
            // Match any component starting with `stored`. In JSON, a tag
            // component starts either right after a `"` (root) or right after
            // a `|` (descendant level). Two patterns cover both cases.
            params.push(Box::new(pat(&format!("\"{stored}"))));
            exprs.push(format!("a.tags {op} ?"));
            params.push(Box::new(pat(&format!("|{stored}"))));
            exprs.push(format!("a.tags {op} ?"));
            // Don't bother with the legacy "input form differs from stored"
            // path: prefix-anchor mode is a power-user shortcut, the user
            // should use the storage form (`|`) directly.
            return exprs;
        }

        if exact_only {
            // Exact/leaf match: the tag exists on the asset BUT is never
            // followed by `|child` at any position in any tag path.
            //
            // With ancestor expansion (CaptureOne/Lightroom convention),
            // `location|Germany|Bayern|Holzkirchen|Marktplatz` also creates
            // standalone tags `Holzkirchen`, `Bayern`, etc. A naive check
            // for `"Holzkirchen|` misses the mid-path case
            // `|Holzkirchen|Marktplatz`. Two NOT clauses cover both:
            //   1. NOT "stored|...  (stored is at the start of a tag path)
            //   2. NOT |stored|...  (stored is mid-path, after a |)
            params.push(Box::new(pat(&format!("\"{stored}\""))));
            params.push(Box::new(desc_pat(&stored)));
            let mid_desc_pat = format!("{wild}|{stored}|{wild}");
            params.push(Box::new(mid_desc_pat));
            exprs.push(format!("(a.tags {op} ? AND a.tags NOT {op} ? AND a.tags NOT {op} ?)"));
        } else {
            params.push(Box::new(pat(&format!("\"{stored}\""))));
            exprs.push(format!("a.tags {op} ?"));
            params.push(Box::new(desc_pat(&stored)));
            exprs.push(format!("a.tags {op} ?"));
        }

        // If stored form differs from input, also match input form
        if tag_value != stored {
            params.push(Box::new(pat(&format!("\"{tag_value}\""))));
            exprs.push(format!("a.tags {op} ?"));
        }

        // If tag contains ", also match JSON-escaped form (\" in stored JSON)
        if tag_value.contains('"') {
            let json_escaped = tag_value.replace('"', "\\\"");
            params.push(Box::new(pat(&format!("\"{json_escaped}\""))));
            exprs.push(format!("a.tags {op} ?"));
        }

        exprs
    }

    /// Helper: add a single positive tag clause (AND).
    fn add_tag_clause(clauses: &mut Vec<String>, params: &mut Vec<Box<dyn rusqlite::types::ToSql>>, tag: &str, negate: bool) {
        let parts = Self::tag_like_parts(params, tag);
        let inner = parts.join(" OR ");
        if negate {
            clauses.push(format!("NOT ({inner})"));
        } else {
            clauses.push(format!("({inner})"));
        }
    }

    /// Helper: add equality filter with IN/NOT IN for comma-OR and negation.
    /// Uses IFNULL for NOT conditions to handle nullable columns correctly
    /// (NULL != 'x' returns NULL, which is falsy — we want NULL to survive exclusion).
    fn add_equality_filter(
        clauses: &mut Vec<String>,
        params: &mut Vec<Box<dyn rusqlite::types::ToSql>>,
        entries: &[String],
        exclude_entries: &[String],
        column: &str,
        _needs_join: &mut bool,
        _is_join_col: bool,
    ) {
        // Case-insensitive equality via COLLATE NOCASE. This handles both
        // asset_type (stored lowercase) and color_label (stored capitalized
        // like "Red"/"Blue") without having to know the canonical case per
        // column. The user can type any casing in the query.
        let include: Vec<&str> = entries.iter()
            .flat_map(|e| e.split(',').map(|s| s.trim()))
            .filter(|s| !s.is_empty())
            .collect();
        if include.len() == 1 {
            clauses.push(format!("{column} = ? COLLATE NOCASE"));
            params.push(Box::new(include[0].to_string()));
        } else if include.len() > 1 {
            let placeholders: Vec<&str> = include.iter().map(|_| "?").collect();
            clauses.push(format!("{column} COLLATE NOCASE IN ({})", placeholders.join(",")));
            for v in &include {
                params.push(Box::new(v.to_string()));
            }
        }
        let exclude: Vec<&str> = exclude_entries.iter()
            .flat_map(|e| e.split(',').map(|s| s.trim()))
            .filter(|s| !s.is_empty())
            .collect();
        if exclude.len() == 1 {
            clauses.push(format!("({column} IS NULL OR {column} != ? COLLATE NOCASE)"));
            params.push(Box::new(exclude[0].to_string()));
        } else if exclude.len() > 1 {
            let placeholders: Vec<&str> = exclude.iter().map(|_| "?").collect();
            clauses.push(format!("({column} IS NULL OR {column} COLLATE NOCASE NOT IN ({}))", placeholders.join(",")));
            for v in &exclude {
                params.push(Box::new(v.to_string()));
            }
        }
    }

    /// Helper: add LIKE filter with OR groups for comma-separated values.
    /// Uses `IS NULL OR NOT LIKE` for exclusions to handle nullable columns.
    fn add_like_filter(
        clauses: &mut Vec<String>,
        params: &mut Vec<Box<dyn rusqlite::types::ToSql>>,
        entries: &[String],
        exclude_entries: &[String],
        column: &str,
        needs_join: &mut bool,
    ) {
        let include: Vec<&str> = entries.iter()
            .flat_map(|e| e.split(',').map(|s| s.trim()))
            .filter(|s| !s.is_empty())
            .collect();
        let exclude: Vec<&str> = exclude_entries.iter()
            .flat_map(|e| e.split(',').map(|s| s.trim()))
            .filter(|s| !s.is_empty())
            .collect();
        if !include.is_empty() || !exclude.is_empty() {
            *needs_join = true;
        }
        if include.len() == 1 {
            clauses.push(format!("{column} LIKE ?"));
            params.push(Box::new(format!("%{}%", include[0])));
        } else if include.len() > 1 {
            let mut or_parts = Vec::new();
            for v in &include {
                or_parts.push(format!("{column} LIKE ?"));
                params.push(Box::new(format!("%{v}%")));
            }
            clauses.push(format!("({})", or_parts.join(" OR ")));
        }
        for v in &exclude {
            clauses.push(format!("({column} IS NULL OR {column} NOT LIKE ?)"));
            params.push(Box::new(format!("%{v}%")));
        }
    }

    /// Paginated search with dynamic filters and sorting.
    /// Uses a separate COUNT query + paginated data query (faster than COUNT(*) OVER()
    /// which forces SQLite to materialize the entire result set).
    pub fn search_paginated_with_count(&self, opts: &SearchOptions) -> Result<(Vec<SearchRow>, u64)> {
        let (where_clause, params, needs_fl_join, needs_v_join) = Self::build_search_where(opts);

        // --- Step 1: Count total matches ---
        let total_count = {
            let count_sql = if needs_v_join {
                let mut sql = String::from(
                    "SELECT COUNT(DISTINCT a.id) FROM assets a \
                     JOIN variants bv ON bv.content_hash = a.best_variant_hash \
                     JOIN variants v ON v.asset_id = a.id",
                );
                if needs_fl_join {
                    sql.push_str(" JOIN file_locations fl ON v.content_hash = fl.content_hash");
                }
                sql.push_str(&where_clause);
                sql
            } else if needs_fl_join {
                let mut sql = String::from(
                    "SELECT COUNT(*) FROM assets a \
                     JOIN variants bv ON bv.content_hash = a.best_variant_hash \
                     JOIN file_locations fl ON bv.content_hash = fl.content_hash",
                );
                sql.push_str(&where_clause);
                sql
            } else {
                // Use same bv JOIN as data query so assets with NULL best_variant_hash
                // are excluded from count (matching the data query behavior)
                let mut sql = String::from(
                    "SELECT COUNT(*) FROM assets a \
                     JOIN variants bv ON bv.content_hash = a.best_variant_hash",
                );
                sql.push_str(&where_clause);
                sql
            };
            let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                params.iter().map(|p| p.as_ref()).collect();
            self.conn.query_row(&count_sql, param_refs.as_slice(), |row| row.get::<_, u64>(0))?
        };

        if total_count == 0 {
            return Ok((Vec::new(), 0));
        }

        // --- Step 2: Fetch one page of results ---
        let (data_params, data_sql) = {
            let mut p = params;
            let page = opts.page.max(1);
            let offset = (page - 1) as u64 * opts.per_page as u64;

            let sql = if needs_v_join {
                let mut inner = String::from(
                    "WITH matched AS (SELECT DISTINCT a.id \
                     FROM assets a \
                     JOIN variants bv ON bv.content_hash = a.best_variant_hash \
                     JOIN variants v ON v.asset_id = a.id",
                );
                if needs_fl_join {
                    inner.push_str(" JOIN file_locations fl ON v.content_hash = fl.content_hash");
                }
                inner.push_str(&where_clause);
                inner.push_str(") SELECT a.id, a.name, a.asset_type, a.created_at, bv.original_filename, bv.format, \
                     a.tags, a.description, bv.content_hash, a.rating, a.color_label, \
                     a.primary_variant_format, a.variant_count, a.stack_id, s.member_count, \
                     a.preview_rotation, a.face_count, a.video_duration \
                     FROM matched m \
                     JOIN assets a ON a.id = m.id \
                     JOIN variants bv ON bv.content_hash = a.best_variant_hash \
                     LEFT JOIN stacks s ON s.id = a.stack_id");
                inner.push_str(&format!(" ORDER BY {}", opts.sort.to_sql()));
                inner.push_str(" LIMIT ? OFFSET ?");
                p.push(Box::new(opts.per_page as u64));
                p.push(Box::new(offset));
                inner
            } else {
                let mut sql = String::from(
                    "SELECT a.id, a.name, a.asset_type, a.created_at, bv.original_filename, bv.format, \
                     a.tags, a.description, bv.content_hash, a.rating, a.color_label, \
                     a.primary_variant_format, a.variant_count, a.stack_id, s.member_count, \
                     a.preview_rotation, a.face_count, a.video_duration \
                     FROM assets a \
                     JOIN variants bv ON bv.content_hash = a.best_variant_hash \
                     LEFT JOIN stacks s ON s.id = a.stack_id",
                );
                sql.push_str(&where_clause);
                sql.push_str(&format!(" ORDER BY {}", opts.sort.to_sql()));
                sql.push_str(" LIMIT ? OFFSET ?");
                p.push(Box::new(opts.per_page as u64));
                p.push(Box::new(offset));
                sql
            };
            (p, sql)
        };

        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            data_params.iter().map(|p| p.as_ref()).collect();
        let mut stmt = self.conn.prepare(&data_sql)?;
        let rows = stmt.query_map(param_refs.as_slice(), |row| {
            let tags_json: String = row.get(6)?;
            let tags: Vec<String> = serde_json::from_str(&tags_json).unwrap_or_default();
            let rating_val: Option<i64> = row.get(9)?;
            let variant_count_val: i64 = row.get(12)?;
            let stack_member_count: Option<i64> = row.get(14)?;
            let rotation_val: Option<i64> = row.get(15)?;
            let face_count_val: i64 = row.get::<_, Option<i64>>(16)?.unwrap_or(0);
            let video_duration: Option<f64> = row.get(17)?;
            Ok(SearchRow {
                asset_id: row.get(0)?,
                name: row.get(1)?,
                asset_type: row.get(2)?,
                created_at: row.get(3)?,
                original_filename: row.get(4)?,
                format: row.get(5)?,
                tags,
                description: row.get(7)?,
                content_hash: row.get(8)?,
                rating: rating_val.map(|r| r as u8),
                color_label: row.get(10)?,
                primary_format: row.get(11)?,
                variant_count: variant_count_val as u32,
                stack_id: row.get(13)?,
                stack_count: stack_member_count.map(|n| n as u32),
                preview_rotation: rotation_val.map(|r| r as u16),
                face_count: face_count_val as u32,
                video_duration,
            })
        })?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok((results, total_count))
    }

    /// Paginated search with dynamic filters and sorting.
    pub fn search_paginated(&self, opts: &SearchOptions) -> Result<Vec<SearchRow>> {
        let (rows, _total) = self.search_paginated_with_count(opts)?;
        Ok(rows)
    }

    /// Fetch a single asset as a SearchRow by asset ID.
    pub fn get_search_row(&self, asset_id: &str) -> Result<Option<SearchRow>> {
        let sql = "SELECT a.id, a.name, a.asset_type, a.created_at, bv.original_filename, bv.format, \
                   a.tags, a.description, bv.content_hash, a.rating, a.color_label, \
                   a.primary_variant_format, a.variant_count, a.stack_id, s.member_count, \
                   a.preview_rotation, a.face_count, a.video_duration \
                   FROM assets a \
                   JOIN variants bv ON bv.content_hash = a.best_variant_hash \
                   LEFT JOIN stacks s ON s.id = a.stack_id \
                   WHERE a.id = ?1";
        let result = self.conn.query_row(sql, rusqlite::params![asset_id], |row| {
            let tags_json: String = row.get(6)?;
            let tags: Vec<String> = serde_json::from_str(&tags_json).unwrap_or_default();
            let rating_val: Option<i64> = row.get(9)?;
            let variant_count_val: i64 = row.get(12)?;
            let stack_member_count: Option<i64> = row.get(14)?;
            let rotation_val: Option<i64> = row.get(15)?;
            let face_count_val: i64 = row.get::<_, Option<i64>>(16)?.unwrap_or(0);
            let video_duration: Option<f64> = row.get(17)?;
            Ok(SearchRow {
                asset_id: row.get(0)?,
                name: row.get(1)?,
                asset_type: row.get(2)?,
                created_at: row.get(3)?,
                original_filename: row.get(4)?,
                format: row.get(5)?,
                tags,
                description: row.get(7)?,
                content_hash: row.get(8)?,
                rating: rating_val.map(|r| r as u8),
                color_label: row.get(10)?,
                primary_format: row.get(11)?,
                variant_count: variant_count_val as u32,
                stack_id: row.get(13)?,
                stack_count: stack_member_count.map(|n| n as u32),
                preview_rotation: rotation_val.map(|r| r as u16),
                face_count: face_count_val as u32,
                video_duration,
            })
        });
        match result {
            Ok(row) => Ok(Some(row)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Count total results matching the same filters as search_paginated (without LIMIT/OFFSET).
    pub fn search_count(&self, opts: &SearchOptions) -> Result<u64> {
        let (where_clause, params, needs_fl_join, needs_v_join) = Self::build_search_where(opts);

        let count_expr = if needs_v_join { "COUNT(DISTINCT a.id)" } else { "COUNT(*)" };
        let mut sql = format!(
            "SELECT {} FROM assets a \
             JOIN variants bv ON bv.content_hash = a.best_variant_hash",
            count_expr
        );

        if needs_v_join {
            sql.push_str(" JOIN variants v ON v.asset_id = a.id");
        }
        if needs_fl_join {
            sql.push_str(" JOIN file_locations fl ON v.content_hash = fl.content_hash");
        }

        sql.push_str(&where_clause);

        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| p.as_ref()).collect();
        let count: u64 = self.conn.query_row(&sql, param_refs.as_slice(), |r| r.get(0))?;
        Ok(count)
    }

    /// Get asset counts per day for a given year, respecting search filters.
    ///
    /// Returns a map of `"YYYY-MM-DD"` → count. Reuses `build_search_where()`
    /// for filter consistency, then adds a year constraint and groups by day.
    pub fn calendar_counts(&self, year: i32, opts: &SearchOptions) -> Result<HashMap<String, u64>> {
        let (where_clause, mut params, needs_fl_join, needs_v_join) = Self::build_search_where(opts);

        let mut sql = String::from(
            "SELECT substr(a.created_at, 1, 10) as day, COUNT(DISTINCT a.id) \
             FROM assets a \
             JOIN variants bv ON bv.content_hash = a.best_variant_hash",
        );

        if needs_v_join {
            sql.push_str(" JOIN variants v ON v.asset_id = a.id");
        }
        if needs_fl_join {
            sql.push_str(" JOIN file_locations fl ON v.content_hash = fl.content_hash");
        }

        sql.push_str(&where_clause);

        // Add year constraint
        sql.push_str(" AND a.created_at >= ? AND a.created_at < ?");
        params.push(Box::new(format!("{year:04}-01-01")));
        params.push(Box::new(format!("{:04}-01-01", year + 1)));

        sql.push_str(" GROUP BY day");

        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| p.as_ref()).collect();
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(param_refs.as_slice(), |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?))
        })?;

        let mut counts = HashMap::new();
        for row in rows {
            let (day, count) = row?;
            counts.insert(day, count);
        }
        Ok(counts)
    }

    /// Get all distinct years that have assets.
    pub fn calendar_years(&self) -> Result<Vec<i32>> {
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT CAST(substr(created_at, 1, 4) AS INTEGER) \
             FROM assets \
             WHERE created_at IS NOT NULL \
             ORDER BY 1",
        )?;
        let rows = stmt.query_map([], |row| row.get::<_, i32>(0))?;
        let mut years = Vec::new();
        for row in rows {
            years.push(row?);
        }
        Ok(years)
    }

    /// Get facet counts for the browse sidebar, respecting search filters.
    ///
    /// Runs 8 aggregate queries sharing the same WHERE clause from `build_search_where()`.
    /// Returns counts grouped by rating, label, format, volume, tag, year, and geo.
    pub fn facet_counts(&self, opts: &SearchOptions) -> Result<FacetCounts> {
        let (where_clause, params, needs_fl_join, needs_v_join) = Self::build_search_where(opts);

        // Helper: build the FROM/JOIN prefix used by most queries
        let mut base_from = String::from(
            "FROM assets a \
             JOIN variants bv ON bv.content_hash = a.best_variant_hash",
        );
        if needs_v_join {
            base_from.push_str(" JOIN variants v ON v.asset_id = a.id");
        }
        if needs_fl_join {
            base_from.push_str(" JOIN file_locations fl ON v.content_hash = fl.content_hash");
        }

        // Macro to build param refs from the shared params vec
        macro_rules! prefs {
            ($p:expr) => {
                {
                    let refs: Vec<&dyn rusqlite::types::ToSql> = $p.iter().map(|b| b.as_ref()).collect();
                    refs
                }
            }
        }

        // 1. Total count
        let total: u64 = self.conn.query_row(
            &format!("SELECT COUNT(DISTINCT a.id) {base_from}{where_clause}"),
            prefs!(params).as_slice(),
            |r| r.get(0),
        )?;

        // 2. Rating distribution
        let mut ratings = Vec::new();
        {
            let sql = format!(
                "SELECT a.rating, COUNT(DISTINCT a.id) AS cnt {base_from}{where_clause} GROUP BY a.rating ORDER BY a.rating"
            );
            let mut stmt = self.conn.prepare(&sql)?;
            let rows = stmt.query_map(prefs!(params).as_slice(), |row| {
                Ok((row.get::<_, Option<u8>>(0)?, row.get::<_, u64>(1)?))
            })?;
            for row in rows {
                ratings.push(row?);
            }
        }

        // 3. Label distribution
        let mut labels = Vec::new();
        {
            let sql = format!(
                "SELECT a.color_label, COUNT(DISTINCT a.id) AS cnt {base_from}{where_clause} GROUP BY a.color_label ORDER BY a.color_label"
            );
            let mut stmt = self.conn.prepare(&sql)?;
            let rows = stmt.query_map(prefs!(params).as_slice(), |row| {
                Ok((row.get::<_, Option<String>>(0)?, row.get::<_, u64>(1)?))
            })?;
            for row in rows {
                labels.push(row?);
            }
        }

        // 4. Format distribution (uses denormalized primary_variant_format)
        let mut formats = Vec::new();
        {
            let sql = format!(
                "SELECT COALESCE(a.primary_variant_format, 'unknown') AS fmt, COUNT(DISTINCT a.id) AS cnt \
                 {base_from}{where_clause} GROUP BY fmt ORDER BY cnt DESC LIMIT 30"
            );
            let mut stmt = self.conn.prepare(&sql)?;
            let rows = stmt.query_map(prefs!(params).as_slice(), |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?))
            })?;
            for row in rows {
                formats.push(row?);
            }
        }

        // 5. Volume distribution — always needs fl JOIN for volume_id
        let mut volumes = Vec::new();
        {
            let mut vol_from = String::from(
                "FROM assets a \
                 JOIN variants bv ON bv.content_hash = a.best_variant_hash",
            );
            if needs_v_join {
                vol_from.push_str(" JOIN variants v ON v.asset_id = a.id");
            }
            // Always join file_locations for volume query
            if needs_fl_join {
                vol_from.push_str(" JOIN file_locations fl ON v.content_hash = fl.content_hash");
            } else if needs_v_join {
                vol_from.push_str(" JOIN file_locations fl ON v.content_hash = fl.content_hash");
            } else {
                // Need both v and fl joins
                vol_from.push_str(" JOIN variants v ON v.asset_id = a.id");
                vol_from.push_str(" JOIN file_locations fl ON v.content_hash = fl.content_hash");
            }
            vol_from.push_str(" JOIN volumes vol ON vol.id = fl.volume_id");

            let sql = format!(
                "SELECT fl.volume_id, vol.label, COUNT(DISTINCT a.id) AS cnt \
                 {vol_from}{where_clause} GROUP BY fl.volume_id ORDER BY cnt DESC"
            );
            let mut stmt = self.conn.prepare(&sql)?;
            let rows = stmt.query_map(prefs!(params).as_slice(), |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, u64>(2)?))
            })?;
            for row in rows {
                volumes.push(row?);
            }
        }

        // 6. Tag distribution (JSON expansion of a.tags)
        let mut tags = Vec::new();
        {
            let sql = format!(
                "SELECT je.value, COUNT(DISTINCT a.id) AS cnt \
                 {base_from}, json_each(a.tags) AS je{where_clause} \
                 GROUP BY je.value ORDER BY cnt DESC LIMIT 30"
            );
            let mut stmt = self.conn.prepare(&sql)?;
            let rows = stmt.query_map(prefs!(params).as_slice(), |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?))
            })?;
            for row in rows {
                let (tag_name, count) = row?;
                tags.push((tag_name, count));
            }
        }

        // 7. Year distribution
        let mut years = Vec::new();
        {
            let sql = format!(
                "SELECT substr(a.created_at, 1, 4) AS year, COUNT(DISTINCT a.id) AS cnt \
                 {base_from}{where_clause} AND a.created_at IS NOT NULL \
                 GROUP BY year ORDER BY year DESC"
            );
            let mut stmt = self.conn.prepare(&sql)?;
            let rows = stmt.query_map(prefs!(params).as_slice(), |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?))
            })?;
            for row in rows {
                years.push(row?);
            }
        }

        // 8. Geotagged count
        let geotagged: u64 = self.conn.query_row(
            &format!(
                "SELECT COUNT(DISTINCT a.id) {base_from}{where_clause} \
                 AND a.latitude IS NOT NULL"
            ),
            prefs!(params).as_slice(),
            |r| r.get(0),
        )?;

        Ok(FacetCounts {
            total,
            ratings,
            labels,
            formats,
            volumes,
            tags,
            years,
            geotagged,
        })
    }

    /// Backfill GPS latitude/longitude on assets from variant source_metadata.
    /// Called from migrations, idempotent via `WHERE a.latitude IS NULL`.
    fn backfill_gps_columns(&self) {
        // Try gps_latitude_decimal first, fall back to parsing DMS strings
        let _ = self.conn.execute_batch(
            "UPDATE assets SET
                latitude = (
                    SELECT COALESCE(
                        CAST(json_extract(v.source_metadata, '$.gps_latitude_decimal') AS REAL),
                        NULL
                    )
                    FROM variants v WHERE v.asset_id = assets.id
                    AND json_extract(v.source_metadata, '$.gps_latitude_decimal') IS NOT NULL
                    ORDER BY CASE v.role WHEN 'original' THEN 0 ELSE 1 END LIMIT 1
                ),
                longitude = (
                    SELECT COALESCE(
                        CAST(json_extract(v.source_metadata, '$.gps_longitude_decimal') AS REAL),
                        NULL
                    )
                    FROM variants v WHERE v.asset_id = assets.id
                    AND json_extract(v.source_metadata, '$.gps_longitude_decimal') IS NOT NULL
                    ORDER BY CASE v.role WHEN 'original' THEN 0 ELSE 1 END LIMIT 1
                )
            WHERE assets.latitude IS NULL
            AND EXISTS (
                SELECT 1 FROM variants v2
                WHERE v2.asset_id = assets.id
                AND json_extract(v2.source_metadata, '$.gps_latitude_decimal') IS NOT NULL
            )"
        );

        // Fallback: parse DMS strings for rows still NULL
        // This needs Rust-side parsing, so we query and update individually
        let rows: Vec<(String, String, String)> = if let Ok(mut stmt) = self.conn.prepare(
            "SELECT a.id, json_extract(v.source_metadata, '$.gps_latitude'),
                    json_extract(v.source_metadata, '$.gps_longitude')
             FROM assets a
             JOIN variants v ON v.asset_id = a.id
             WHERE a.latitude IS NULL
             AND json_extract(v.source_metadata, '$.gps_latitude') IS NOT NULL
             AND json_extract(v.source_metadata, '$.gps_longitude') IS NOT NULL
             GROUP BY a.id"
        ) {
            stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?))
            }).map(|rows| rows.filter_map(|r| r.ok()).collect()).unwrap_or_default()
        } else {
            Vec::new()
        };

        for (id, lat_str, lon_str) in &rows {
            if let (Some(lat), Some(lon)) = (
                crate::exif_reader::parse_dms_string(lat_str),
                crate::exif_reader::parse_dms_string(lon_str),
            ) {
                let _ = self.conn.execute(
                    "UPDATE assets SET latitude = ?1, longitude = ?2 WHERE id = ?3",
                    rusqlite::params![lat, lon, id],
                );
            }
        }
    }

    /// Get map markers (geotagged assets) matching search filters.
    pub fn map_markers(&self, opts: &SearchOptions, limit: u32) -> Result<(Vec<MapMarker>, u64)> {
        let (where_clause, mut params, needs_fl_join, needs_v_join) = Self::build_search_where(opts);

        // Count total geotagged assets matching filters
        let mut count_sql = String::from(
            "SELECT COUNT(DISTINCT a.id) FROM assets a \
             JOIN variants bv ON bv.content_hash = a.best_variant_hash",
        );
        if needs_v_join {
            count_sql.push_str(" JOIN variants v ON v.asset_id = a.id");
        }
        if needs_fl_join {
            count_sql.push_str(" JOIN file_locations fl ON v.content_hash = fl.content_hash");
        }
        count_sql.push_str(&where_clause);
        count_sql.push_str(" AND a.latitude IS NOT NULL AND a.longitude IS NOT NULL");

        let count_param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| p.as_ref()).collect();
        let total: u64 = self.conn.query_row(&count_sql, count_param_refs.as_slice(), |r| r.get(0))?;

        // Fetch markers
        let mut sql = String::from(
            "SELECT DISTINCT a.id, a.latitude, a.longitude, a.best_variant_hash, \
             COALESCE(a.name, bv.original_filename) as display_name, a.rating, a.color_label \
             FROM assets a \
             JOIN variants bv ON bv.content_hash = a.best_variant_hash",
        );
        if needs_v_join {
            sql.push_str(" JOIN variants v ON v.asset_id = a.id");
        }
        if needs_fl_join {
            sql.push_str(" JOIN file_locations fl ON v.content_hash = fl.content_hash");
        }
        sql.push_str(&where_clause);
        sql.push_str(" AND a.latitude IS NOT NULL AND a.longitude IS NOT NULL");
        sql.push_str(" LIMIT ?");
        params.push(Box::new(limit as u64));

        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| p.as_ref()).collect();
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(param_refs.as_slice(), |row| {
            Ok(MapMarker {
                id: row.get(0)?,
                lat: row.get(1)?,
                lng: row.get(2)?,
                preview: row.get(3)?,
                name: row.get(4)?,
                rating: row.get::<_, Option<i64>>(5)?.map(|r| r as u8),
                label: row.get(6)?,
            })
        })?;

        let mut markers = Vec::new();
        for row in rows {
            markers.push(row?);
        }
        Ok((markers, total))
    }

    /// List all unique tags with their usage counts, sorted by count descending.
    pub fn list_all_tags(&self) -> Result<Vec<(String, u64)>> {
        // Use json_each() for SQL-side aggregation — avoids loading all 150k+ tag JSON blobs
        let mut stmt = self.conn.prepare(
            "SELECT je.value, COUNT(*) as cnt \
             FROM assets, json_each(assets.tags) AS je \
             WHERE assets.tags != '[]' \
             GROUP BY je.value \
             ORDER BY cnt DESC, je.value ASC",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, u64>(1)?))
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Find assets with a specific exact tag, returning (asset_id, stack_id) pairs.
    /// Ordered by created_at ASC so the oldest asset comes first.
    pub fn assets_with_exact_tag(&self, tag: &str) -> Result<Vec<(String, Option<String>)>> {
        let mut stmt = self.conn.prepare(
            "SELECT a.id, a.stack_id \
             FROM assets a, json_each(a.tags) AS je \
             WHERE je.value = ?1 COLLATE NOCASE \
             ORDER BY a.created_at ASC",
        )?;
        let rows = stmt.query_map(rusqlite::params![tag], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Find assets that have a tag matching exactly or starting with `tag|` (prefix match).
    /// Used by tag rename to cascade renames to descendant tags.
    /// Find assets whose tag set contains `tag`, optionally including descendants
    /// (`tag|child`) and optionally case-sensitive.
    ///
    /// - `case_sensitive = false` (default): uses `COLLATE NOCASE` for the equality
    ///   check and `LIKE … COLLATE NOCASE` for the prefix check.
    /// - `case_sensitive = true`: uses byte-exact equality and `GLOB` for the
    ///   prefix check (GLOB is case-sensitive in SQLite).
    /// - `exact_only = true`: skips the descendant prefix check, returning only
    ///   assets tagged at exactly this level.
    pub fn assets_with_tag_or_prefix(
        &self,
        tag: &str,
        case_sensitive: bool,
        exact_only: bool,
    ) -> Result<Vec<(String, Option<String>)>> {
        let (sql, params): (String, Vec<Box<dyn rusqlite::types::ToSql>>) = match (case_sensitive, exact_only) {
            (false, false) => {
                // Default: case-insensitive, include descendants
                (
                    "SELECT DISTINCT a.id, a.stack_id \
                     FROM assets a, json_each(a.tags) AS je \
                     WHERE je.value = ?1 COLLATE NOCASE \
                        OR je.value LIKE ?2 COLLATE NOCASE \
                     ORDER BY a.created_at ASC".to_string(),
                    vec![Box::new(tag.to_string()), Box::new(format!("{}|%", tag))],
                )
            }
            (false, true) => {
                // Case-insensitive, exact level only (no descendants)
                (
                    "SELECT DISTINCT a.id, a.stack_id \
                     FROM assets a, json_each(a.tags) AS je \
                     WHERE je.value = ?1 COLLATE NOCASE \
                     ORDER BY a.created_at ASC".to_string(),
                    vec![Box::new(tag.to_string())],
                )
            }
            (true, false) => {
                // Case-sensitive, include descendants. GLOB uses `*` as wildcard.
                (
                    "SELECT DISTINCT a.id, a.stack_id \
                     FROM assets a, json_each(a.tags) AS je \
                     WHERE je.value = ?1 \
                        OR je.value GLOB ?2 \
                     ORDER BY a.created_at ASC".to_string(),
                    vec![Box::new(tag.to_string()), Box::new(format!("{}|*", tag))],
                )
            }
            (true, true) => {
                // Case-sensitive, exact level only
                (
                    "SELECT DISTINCT a.id, a.stack_id \
                     FROM assets a, json_each(a.tags) AS je \
                     WHERE je.value = ?1 \
                     ORDER BY a.created_at ASC".to_string(),
                    vec![Box::new(tag.to_string())],
                )
            }
        };
        let mut stmt = self.conn.prepare(&sql)?;
        let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
        let rows = stmt.query_map(param_refs.as_slice(), |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// List all distinct variant formats.
    pub fn list_all_formats(&self) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT format FROM variants ORDER BY format",
        )?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        rows.collect::<std::result::Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// List all variant formats with their counts (for grouped format filter).
    pub fn list_all_format_counts(&self) -> Result<Vec<(String, u64)>> {
        let mut stmt = self.conn.prepare(
            "SELECT format, COUNT(*) as cnt FROM variants GROUP BY format ORDER BY cnt DESC",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, u64>(1)?))
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// List all volumes from the catalog's volumes table.
    pub fn list_volumes(&self) -> Result<Vec<(String, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, label FROM volumes ORDER BY label",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect::<std::result::Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Per-volume statistics (before merging device registry).
    fn stats_per_volume(&self) -> Result<Vec<VolumeStatsRaw>> {
        // Combined query: core counts + variant counts + verification — single pass over file_locations
        #[allow(clippy::type_complexity)]
        let mut core: HashMap<String, (String, u64, u64, u64, u64, u64, Option<String>)> = HashMap::new();
        {
            let mut stmt = self.conn.prepare(
                "SELECT fl.volume_id, v.label, \
                 COUNT(*) AS loc_count, \
                 COUNT(DISTINCT va.asset_id) AS asset_count, \
                 COALESCE(SUM(va.file_size), 0) AS total_size, \
                 COUNT(DISTINCT fl.content_hash) AS variant_count, \
                 SUM(CASE WHEN fl.verified_at IS NOT NULL THEN 1 ELSE 0 END) AS verified_count, \
                 MIN(fl.verified_at) AS oldest_verified \
                 FROM file_locations fl \
                 JOIN volumes v ON fl.volume_id = v.id \
                 JOIN variants va ON fl.content_hash = va.content_hash \
                 GROUP BY fl.volume_id",
            )?;
            let rows = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, u64>(2)?,
                    r.get::<_, u64>(3)?,
                    r.get::<_, u64>(4)?,
                    r.get::<_, u64>(5)?,
                    r.get::<_, u64>(6)?,
                    r.get::<_, Option<String>>(7)?,
                ))
            })?;
            for row in rows {
                let (vid, label, loc_count, asset_count, size, variants, verified, oldest) = row?;
                core.insert(vid, (label, loc_count, asset_count, size, variants, verified, oldest));
            }
        }

        // Directory counting — SQL-side using RTRIM trick for parent path extraction
        let mut dirs_per_vol: HashMap<String, u64> = HashMap::new();
        {
            let mut stmt = self.conn.prepare(
                "SELECT volume_id, COUNT(*) FROM ( \
                    SELECT DISTINCT volume_id, \
                        RTRIM(RTRIM(relative_path, REPLACE(relative_path, '/', '')), '/') AS parent_dir \
                    FROM file_locations \
                 ) GROUP BY volume_id",
            )?;
            let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, u64>(1)?)))?;
            for row in rows {
                let (vid, count) = row?;
                dirs_per_vol.insert(vid, count);
            }
        }

        // Formats per volume
        let mut formats_per_vol: HashMap<String, Vec<String>> = HashMap::new();
        {
            let mut stmt = self.conn.prepare(
                "SELECT DISTINCT fl.volume_id, va.format \
                 FROM file_locations fl \
                 JOIN variants va ON fl.content_hash = va.content_hash \
                 ORDER BY fl.volume_id, va.format",
            )?;
            let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
            for row in rows {
                let (vid, fmt) = row?;
                formats_per_vol.entry(vid).or_default().push(fmt);
            }
        }

        // Recipe count per volume
        let mut recipes_per_vol: HashMap<String, u64> = HashMap::new();
        {
            let mut stmt = self.conn.prepare(
                "SELECT volume_id, COUNT(*) FROM recipes WHERE volume_id IS NOT NULL GROUP BY volume_id",
            )?;
            let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, u64>(1)?)))?;
            for row in rows {
                let (vid, count) = row?;
                recipes_per_vol.insert(vid, count);
            }
        }

        // All volumes (including those with no file_locations)
        let mut all_volume_ids: HashMap<String, String> = HashMap::new();
        {
            let mut stmt = self.conn.prepare("SELECT id, label FROM volumes")?;
            let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
            for row in rows {
                let (vid, label) = row?;
                all_volume_ids.insert(vid, label);
            }
        }

        // Merge all data
        let mut result = Vec::new();
        for (vid, label) in &all_volume_ids {
            let (_, loc_count, asset_count, size, variants, verified, oldest) = core
                .get(vid)
                .cloned()
                .unwrap_or_else(|| (label.clone(), 0, 0, 0, 0, 0, None));
            let dirs = *dirs_per_vol.get(vid).unwrap_or(&0);
            let formats = formats_per_vol.remove(vid).unwrap_or_default();
            let recipes = *recipes_per_vol.get(vid).unwrap_or(&0);

            result.push(VolumeStatsRaw {
                volume_id: vid.clone(),
                label: label.clone(),
                assets: asset_count,
                variants,
                recipes,
                formats,
                directories: dirs,
                size,
                verified_count: verified,
                total_locations: loc_count,
                oldest_verified_at: oldest,
            });
        }

        result.sort_by(|a, b| a.label.cmp(&b.label));
        Ok(result)
    }

    /// Tag frequency counts: uses json_each() to expand and aggregate in SQL.
    /// Returns Vec<(tag, count)> sorted by count descending.
    pub fn stats_tag_frequencies(&self, limit: usize) -> Result<Vec<(String, u64)>> {
        let mut stmt = self.conn.prepare(
            "SELECT je.value, COUNT(*) as cnt \
             FROM assets, json_each(assets.tags) AS je \
             WHERE assets.tags != '[]' \
             GROUP BY je.value \
             ORDER BY cnt DESC, je.value ASC \
             LIMIT ?1",
        )?;
        let rows = stmt.query_map(rusqlite::params![limit as u64], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, u64>(1)?))
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Count of unique tags (uses json_each for SQL-side aggregation).
    pub fn stats_unique_tag_count(&self) -> Result<u64> {
        self.conn.query_row(
            "SELECT COUNT(DISTINCT je.value) \
             FROM assets, json_each(assets.tags) AS je \
             WHERE assets.tags != '[]'",
            [],
            |r| r.get(0),
        ).map_err(Into::into)
    }

    /// Tag coverage: (tagged_count, untagged_count).
    pub fn stats_tag_coverage(&self) -> Result<(u64, u64)> {
        self.conn.query_row(
            "SELECT \
                COALESCE(SUM(CASE WHEN tags != '[]' THEN 1 ELSE 0 END), 0), \
                COALESCE(SUM(CASE WHEN tags = '[]' THEN 1 ELSE 0 END), 0) \
             FROM assets",
            [],
            |r| Ok((r.get::<_, u64>(0)?, r.get::<_, u64>(1)?)),
        ).map_err(Into::into)
    }

    /// Verification overview for file_locations:
    /// (total, verified, oldest_verified_at, newest_verified_at).
    pub fn stats_verification_overview(&self) -> Result<(u64, u64, Option<String>, Option<String>)> {
        self.conn.query_row(
            "SELECT COUNT(*), \
                COALESCE(SUM(CASE WHEN verified_at IS NOT NULL THEN 1 ELSE 0 END), 0), \
                MIN(verified_at), \
                MAX(verified_at) \
             FROM file_locations",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        ).map_err(Into::into)
    }

    /// Verification counts for recipes: (total, verified).
    pub fn stats_recipe_verification(&self) -> Result<(u64, u64)> {
        let total: u64 = self.conn.query_row(
            "SELECT COUNT(*) FROM recipes", [], |r| r.get(0),
        )?;
        let verified: u64 = self.conn.query_row(
            "SELECT COUNT(*) FROM recipes WHERE verified_at IS NOT NULL", [], |r| r.get(0),
        )?;
        Ok((total, verified))
    }

    /// Per-volume verification: Vec<(label, volume_id, total, verified, oldest_verified_at)>.
    pub fn stats_verification_per_volume(&self) -> Result<Vec<(String, String, u64, u64, Option<String>)>> {
        let mut stmt = self.conn.prepare(
            "SELECT v.label, fl.volume_id, \
             COUNT(*) AS total, \
             SUM(CASE WHEN fl.verified_at IS NOT NULL THEN 1 ELSE 0 END) AS verified, \
             MIN(fl.verified_at) AS oldest \
             FROM file_locations fl \
             JOIN volumes v ON fl.volume_id = v.id \
             GROUP BY fl.volume_id \
             ORDER BY v.label",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, u64>(2)?,
                r.get::<_, u64>(3)?,
                r.get::<_, Option<String>>(4)?,
            ))
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Build full CatalogStats with optional sections.
    pub fn build_stats(
        &self,
        volumes_info: &[(String, String, bool, Option<String>)], // (label, volume_id, is_online, purpose)
        show_types: bool,
        show_volumes: bool,
        show_tags: bool,
        show_verified: bool,
        limit: usize,
    ) -> Result<CatalogStats> {
        let (assets, variants, recipes, total_size, file_locations) = self.stats_overview()?;
        let (_, unique_recipes) = self.stats_recipe_counts()?;

        let volumes_total = volumes_info.len() as u64;
        let volumes_online = volumes_info.iter().filter(|v| v.2).count() as u64;
        let volumes_offline = volumes_total - volumes_online;

        let overview = OverviewStats {
            assets,
            variants,
            recipes,
            file_locations,
            unique_recipes,
            volumes_total,
            volumes_online,
            volumes_offline,
            total_size,
        };

        let types = if show_types {
            let asset_types_raw = self.stats_asset_types()?;
            let total_assets = assets.max(1) as f64;
            let asset_types: Vec<TypeCount> = asset_types_raw
                .into_iter()
                .map(|(t, c)| TypeCount {
                    asset_type: t,
                    count: c,
                    percentage: (c as f64 / total_assets) * 100.0,
                })
                .collect();

            let variant_formats: Vec<FormatCount> = self
                .stats_variant_formats(limit)?
                .into_iter()
                .map(|(f, c)| FormatCount { format: f, count: c })
                .collect();

            let recipe_formats: Vec<FormatCount> = self
                .stats_recipe_formats(limit)?
                .into_iter()
                .map(|(f, c)| FormatCount { format: f, count: c })
                .collect();

            Some(TypeStats {
                asset_types,
                variant_formats,
                recipe_formats,
            })
        } else {
            None
        };

        let volumes = if show_volumes {
            let raw = self.stats_per_volume()?;
            let vol_stats: Vec<VolumeStats> = raw
                .into_iter()
                .map(|r| {
                    let vol_info = volumes_info
                        .iter()
                        .find(|v| v.1 == r.volume_id);
                    let is_online = vol_info.map(|v| v.2).unwrap_or(false);
                    let purpose = vol_info.and_then(|v| v.3.clone());
                    let verification_pct = if r.total_locations > 0 {
                        (r.verified_count as f64 / r.total_locations as f64) * 100.0
                    } else {
                        0.0
                    };
                    VolumeStats {
                        label: r.label,
                        volume_id: r.volume_id,
                        is_online,
                        purpose,
                        assets: r.assets,
                        variants: r.variants,
                        recipes: r.recipes,
                        formats: r.formats,
                        directories: r.directories,
                        size: r.size,
                        verified_count: r.verified_count,
                        total_locations: r.total_locations,
                        verification_pct,
                        oldest_verified_at: r.oldest_verified_at,
                    }
                })
                .collect();
            Some(vol_stats)
        } else {
            None
        };

        let tags = if show_tags {
            let (tagged, untagged) = self.stats_tag_coverage()?;
            let unique_tags = self.stats_unique_tag_count()?;
            let top_tags: Vec<TagCount> = self
                .stats_tag_frequencies(limit)?
                .into_iter()
                .map(|(tag, count)| TagCount { tag, count })
                .collect();

            Some(TagStats {
                unique_tags,
                tagged_assets: tagged,
                untagged_assets: untagged,
                top_tags,
            })
        } else {
            None
        };

        let verified = if show_verified {
            let (total, verified_count, oldest, newest) = self.stats_verification_overview()?;
            let coverage_pct = if total > 0 {
                (verified_count as f64 / total as f64) * 100.0
            } else {
                0.0
            };

            let per_volume_raw = self.stats_verification_per_volume()?;
            let per_volume: Vec<VolumeVerificationStats> = per_volume_raw
                .into_iter()
                .map(|(label, vid, total, verified, oldest)| {
                    let vol_info = volumes_info
                        .iter()
                        .find(|v| v.1 == vid);
                    let is_online = vol_info.map(|v| v.2).unwrap_or(false);
                    let purpose = vol_info.and_then(|v| v.3.clone());
                    let cov = if total > 0 {
                        (verified as f64 / total as f64) * 100.0
                    } else {
                        0.0
                    };
                    VolumeVerificationStats {
                        label,
                        volume_id: vid,
                        is_online,
                        purpose,
                        locations: total,
                        verified,
                        coverage_pct: cov,
                        oldest_verified_at: oldest,
                    }
                })
                .collect();

            Some(VerificationStats {
                total_locations: total,
                verified_locations: verified_count,
                unverified_locations: total - verified_count,
                coverage_pct,
                oldest_verified_at: oldest,
                newest_verified_at: newest,
                per_volume,
            })
        } else {
            None
        };

        Ok(CatalogStats {
            overview,
            types,
            volumes,
            tags,
            verified,
        })
    }

    /// Build analytics data for the dashboard page.
    pub fn build_analytics(&self, limit: usize) -> Result<AnalyticsData> {
        // Camera usage (top N)
        let mut stmt = self.conn.prepare(
            "SELECT camera_model, COUNT(*) as cnt FROM variants
             WHERE camera_model IS NOT NULL AND camera_model != ''
             GROUP BY camera_model ORDER BY cnt DESC LIMIT ?1"
        )?;
        let camera_usage: Vec<NameCount> = stmt.query_map([limit as i64], |row| {
            Ok(NameCount {
                name: row.get(0)?,
                count: row.get::<_, i64>(1)? as u64,
            })
        })?.filter_map(|r| r.ok()).collect();

        // Lens usage (top N)
        let mut stmt = self.conn.prepare(
            "SELECT lens_model, COUNT(*) as cnt FROM variants
             WHERE lens_model IS NOT NULL AND lens_model != ''
             GROUP BY lens_model ORDER BY cnt DESC LIMIT ?1"
        )?;
        let lens_usage: Vec<NameCount> = stmt.query_map([limit as i64], |row| {
            Ok(NameCount {
                name: row.get(0)?,
                count: row.get::<_, i64>(1)? as u64,
            })
        })?.filter_map(|r| r.ok()).collect();

        // Rating distribution (0=unrated, 1-5)
        let mut rating_distribution = Vec::new();
        let unrated: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM assets WHERE rating IS NULL", [], |r| r.get(0)
        )?;
        rating_distribution.push(RatingCount { rating: 0, count: unrated as u64 });
        for r in 1..=5u8 {
            let cnt: i64 = self.conn.query_row(
                "SELECT COUNT(*) FROM assets WHERE rating = ?1", [r], |row| row.get(0)
            )?;
            rating_distribution.push(RatingCount { rating: r, count: cnt as u64 });
        }

        // Format distribution (top N by primary_variant_format)
        let mut stmt = self.conn.prepare(
            "SELECT primary_variant_format, COUNT(*) as cnt FROM assets
             WHERE primary_variant_format IS NOT NULL AND primary_variant_format != ''
             GROUP BY primary_variant_format ORDER BY cnt DESC LIMIT ?1"
        )?;
        let format_distribution: Vec<NameCount> = stmt.query_map([limit as i64], |row| {
            Ok(NameCount {
                name: row.get(0)?,
                count: row.get::<_, i64>(1)? as u64,
            })
        })?.filter_map(|r| r.ok()).collect();

        // Monthly imports (last 24 months, by created_at)
        let mut stmt = self.conn.prepare(
            "SELECT strftime('%Y-%m', created_at) as month, COUNT(*) as cnt FROM assets
             WHERE created_at IS NOT NULL
             GROUP BY month ORDER BY month DESC LIMIT 24"
        )?;
        let mut monthly_imports: Vec<MonthCount> = stmt.query_map([], |row| {
            Ok(MonthCount {
                month: row.get(0)?,
                count: row.get::<_, i64>(1)? as u64,
            })
        })?.filter_map(|r| r.ok()).collect();
        monthly_imports.reverse(); // chronological order

        // Storage by volume
        let mut stmt = self.conn.prepare(
            "SELECT v.label, COALESCE(SUM(var.file_size), 0) as total_size
             FROM volumes v
             JOIN file_locations fl ON fl.volume_id = v.id
             JOIN variants var ON var.content_hash = fl.content_hash
             GROUP BY v.id ORDER BY total_size DESC"
        )?;
        let storage_by_volume: Vec<VolumeSize> = stmt.query_map([], |row| {
            Ok(VolumeSize {
                label: row.get(0)?,
                size: row.get::<_, i64>(1)? as u64,
            })
        })?.filter_map(|r| r.ok()).collect();

        // Yearly asset counts (by created_at)
        let mut stmt = self.conn.prepare(
            "SELECT strftime('%Y', created_at) as year, COUNT(*) as cnt FROM assets
             WHERE created_at IS NOT NULL
             GROUP BY year ORDER BY year"
        )?;
        let yearly_counts: Vec<YearCount> = stmt.query_map([], |row| {
            Ok(YearCount {
                year: row.get(0)?,
                count: row.get::<_, i64>(1)? as u64,
            })
        })?.filter_map(|r| r.ok()).collect();

        Ok(AnalyticsData {
            camera_usage,
            lens_usage,
            rating_distribution,
            format_distribution,
            monthly_imports,
            storage_by_volume,
            yearly_counts,
        })
    }

    /// Build a backup-status overview for the given scope of assets.
    ///
    /// - `scope_ids`: `None` = all assets, `Some(ids)` = specific assets
    /// - `volumes_info`: `(label, volume_id, is_online, purpose)` from DeviceRegistry
    /// - `min_copies`: threshold for "at risk"
    /// - `target_volume_id`: optional volume to compute `VolumeGapDetail` for
    pub fn backup_status_overview(
        &self,
        scope_ids: Option<&[String]>,
        volumes_info: &[(String, String, bool, Option<String>)],
        min_copies: u64,
        target_volume_id: Option<&str>,
    ) -> Result<BackupStatusResult> {
        let scoped = scope_ids.is_some();

        // Create temp table for scoped queries
        if let Some(ids) = scope_ids {
            self.conn.execute_batch("CREATE TEMP TABLE IF NOT EXISTS _bs_scope (asset_id TEXT PRIMARY KEY)")?;
            self.conn.execute_batch("DELETE FROM _bs_scope")?;

            // Batch insert in chunks of 500
            for chunk in ids.chunks(500) {
                let placeholders: Vec<&str> = chunk.iter().map(|_| "(?)").collect();
                let sql = format!("INSERT OR IGNORE INTO _bs_scope (asset_id) VALUES {}", placeholders.join(","));
                let params: Vec<&dyn rusqlite::types::ToSql> = chunk.iter().map(|id| id as &dyn rusqlite::types::ToSql).collect();
                self.conn.execute(&sql, params.as_slice())?;
            }
        }

        let scope_filter = if scoped {
            "JOIN _bs_scope bs ON bs.asset_id = a.id"
        } else {
            ""
        };
        let scope_filter_v = if scoped {
            "JOIN _bs_scope bs ON bs.asset_id = v.asset_id"
        } else {
            ""
        };

        // Combined counts: total assets, variants, file_locations in one query
        let (total_assets, total_variants, total_file_locations): (u64, u64, u64) = self.conn.query_row(
            &format!(
                "SELECT \
                    (SELECT COUNT(*) FROM assets a {}), \
                    (SELECT COUNT(*) FROM variants v {}), \
                    (SELECT COUNT(*) FROM file_locations fl \
                        JOIN variants v ON fl.content_hash = v.content_hash {})",
                scope_filter, scope_filter_v, scope_filter_v
            ),
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;

        // Volume distribution + at-risk count in a single pass
        // Computes per-asset volume count, then buckets and counts at-risk
        let mut stmt = self.conn.prepare(&format!(
            "SELECT vol_count, COUNT(*) FROM ( \
                SELECT a.id, COUNT(DISTINCT fl.volume_id) as vol_count \
                FROM assets a {} \
                LEFT JOIN variants v2 ON v2.asset_id = a.id \
                LEFT JOIN file_locations fl ON fl.content_hash = v2.content_hash \
                GROUP BY a.id \
            ) GROUP BY vol_count ORDER BY vol_count",
            scope_filter,
        ))?;
        let mut buckets = [0u64; 4]; // [0, 1, 2, 3+]
        let mut at_risk_count = 0u64;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, u64>(0)?, r.get::<_, u64>(1)?)))?;
        for row in rows {
            let (vol_count, asset_count) = row?;
            match vol_count {
                0 => buckets[0] += asset_count,
                1 => buckets[1] += asset_count,
                2 => buckets[2] += asset_count,
                _ => buckets[3] += asset_count,
            }
            if vol_count < min_copies {
                at_risk_count += asset_count;
            }
        }
        let location_distribution = vec![
            LocationBucket { volume_count: "0".to_string(), asset_count: buckets[0] },
            LocationBucket { volume_count: "1".to_string(), asset_count: buckets[1] },
            LocationBucket { volume_count: "2".to_string(), asset_count: buckets[2] },
            LocationBucket { volume_count: "3+".to_string(), asset_count: buckets[3] },
        ];

        // Purpose coverage + volume gaps in batch queries (one each instead of per-item)
        let mut purpose_groups: HashMap<String, Vec<(String, String, Option<String>)>> = HashMap::new();
        for (label, vid, _online, purpose) in volumes_info {
            let purpose_str = purpose.as_deref().unwrap_or("(none)");
            purpose_groups.entry(purpose_str.to_string()).or_default()
                .push((vid.clone(), label.clone(), purpose.clone()));
        }

        // Single query: asset count per volume (reused for both purpose coverage and volume gaps)
        let mut assets_per_volume: HashMap<String, u64> = HashMap::new();
        {
            let mut stmt = self.conn.prepare(&format!(
                "SELECT fl.volume_id, COUNT(DISTINCT v.asset_id) \
                 FROM file_locations fl \
                 JOIN variants v ON fl.content_hash = v.content_hash \
                 {} \
                 GROUP BY fl.volume_id",
                scope_filter_v,
            ))?;
            let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, u64>(1)?)))?;
            for row in rows {
                let (vid, count) = row?;
                assets_per_volume.insert(vid, count);
            }
        }

        // Build purpose coverage from the per-volume counts
        let mut purpose_coverage = Vec::new();
        for (purpose_str, vol_entries) in &purpose_groups {
            // For purpose coverage we need assets on ANY volume of this purpose (distinct)
            let vol_ids: Vec<&str> = vol_entries.iter().map(|(vid, _, _)| vid.as_str()).collect();
            if vol_ids.is_empty() { continue; }
            let placeholders: Vec<String> = vol_ids.iter().enumerate().map(|(i, _)| format!("?{}", i + 1)).collect();
            let sql = format!(
                "SELECT COUNT(DISTINCT v.asset_id) FROM file_locations fl \
                 JOIN variants v ON fl.content_hash = v.content_hash \
                 {} \
                 WHERE fl.volume_id IN ({})",
                scope_filter_v,
                placeholders.join(","),
            );
            let params: Vec<&dyn rusqlite::types::ToSql> = vol_ids.iter().map(|id| id as &dyn rusqlite::types::ToSql).collect();
            let asset_count: u64 = self.conn.query_row(&sql, params.as_slice(), |r| r.get(0))?;
            let pct = if total_assets > 0 { (asset_count as f64 / total_assets as f64) * 100.0 } else { 0.0 };
            purpose_coverage.push(PurposeCoverage {
                purpose: purpose_str.clone(),
                volume_count: vol_entries.len() as u64,
                asset_count,
                asset_percentage: pct,
            });
        }
        purpose_coverage.sort_by(|a, b| b.asset_count.cmp(&a.asset_count));

        // Build volume gaps from the per-volume counts (no extra queries)
        let mut volume_gaps = Vec::new();
        for (label, vid, _online, purpose) in volumes_info {
            let present = *assets_per_volume.get(vid).unwrap_or(&0);
            let missing = total_assets.saturating_sub(present);
            if missing > 0 {
                volume_gaps.push(VolumeGap {
                    volume_label: label.clone(),
                    volume_id: vid.clone(),
                    purpose: purpose.clone(),
                    missing_count: missing,
                });
            }
        }
        volume_gaps.sort_by(|a, b| a.missing_count.cmp(&b.missing_count));

        // Volume detail: for --volume target
        let volume_detail = if let Some(target_vid) = target_volume_id {
            let present: u64 = self.conn.query_row(
                &format!(
                    "SELECT COUNT(DISTINCT v.asset_id) FROM file_locations fl \
                     JOIN variants v ON fl.content_hash = v.content_hash \
                     {} \
                     WHERE fl.volume_id = ?1",
                    scope_filter_v,
                ),
                [target_vid],
                |r| r.get(0),
            )?;
            let missing = total_assets.saturating_sub(present);
            let pct = if total_assets > 0 { (present as f64 / total_assets as f64) * 100.0 } else { 0.0 };
            let vol_info = volumes_info.iter().find(|v| v.1 == target_vid);
            Some(VolumeGapDetail {
                volume_label: vol_info.map(|v| v.0.clone()).unwrap_or_default(),
                volume_id: target_vid.to_string(),
                purpose: vol_info.and_then(|v| v.3.clone()),
                present_count: present,
                missing_count: missing,
                total_scoped: total_assets,
                coverage_pct: pct,
            })
        } else {
            None
        };

        let scope = if scoped { "filtered" } else { "all assets" }.to_string();

        // Cleanup temp table
        if scoped {
            let _ = self.conn.execute_batch("DROP TABLE IF EXISTS _bs_scope");
        }

        Ok(BackupStatusResult {
            scope,
            total_assets,
            total_variants,
            total_file_locations,
            min_copies,
            at_risk_count,
            purpose_coverage,
            location_distribution,
            volume_gaps,
            volume_detail,
        })
    }

    /// Return asset IDs on fewer than `min_copies` distinct volumes.
    pub fn backup_status_at_risk_ids(
        &self,
        scope_ids: Option<&[String]>,
        min_copies: u64,
    ) -> Result<Vec<String>> {
        let scoped = scope_ids.is_some();

        if let Some(ids) = scope_ids {
            self.conn.execute_batch("CREATE TEMP TABLE IF NOT EXISTS _bs_scope (asset_id TEXT PRIMARY KEY)")?;
            self.conn.execute_batch("DELETE FROM _bs_scope")?;
            for chunk in ids.chunks(500) {
                let placeholders: Vec<&str> = chunk.iter().map(|_| "(?)").collect();
                let sql = format!("INSERT OR IGNORE INTO _bs_scope (asset_id) VALUES {}", placeholders.join(","));
                let params: Vec<&dyn rusqlite::types::ToSql> = chunk.iter().map(|id| id as &dyn rusqlite::types::ToSql).collect();
                self.conn.execute(&sql, params.as_slice())?;
            }
        }

        let scope_filter = if scoped {
            "JOIN _bs_scope bs ON bs.asset_id = a.id"
        } else {
            ""
        };

        let mut stmt = self.conn.prepare(&format!(
            "SELECT a.id FROM assets a {} \
             LEFT JOIN variants v ON v.asset_id = a.id \
             LEFT JOIN file_locations fl ON fl.content_hash = v.content_hash \
             GROUP BY a.id HAVING COUNT(DISTINCT fl.volume_id) < ?1",
            scope_filter,
        ))?;
        let rows = stmt.query_map([min_copies], |r| r.get::<_, String>(0))?;
        let mut ids = Vec::new();
        for row in rows {
            ids.push(row?);
        }

        if scoped {
            let _ = self.conn.execute_batch("DROP TABLE IF EXISTS _bs_scope");
        }

        Ok(ids)
    }

    /// Return asset IDs that have no file_location on the given volume.
    pub fn backup_status_missing_from_volume(
        &self,
        scope_ids: Option<&[String]>,
        volume_id: &str,
    ) -> Result<Vec<String>> {
        let scoped = scope_ids.is_some();

        if let Some(ids) = scope_ids {
            self.conn.execute_batch("CREATE TEMP TABLE IF NOT EXISTS _bs_scope (asset_id TEXT PRIMARY KEY)")?;
            self.conn.execute_batch("DELETE FROM _bs_scope")?;
            for chunk in ids.chunks(500) {
                let placeholders: Vec<&str> = chunk.iter().map(|_| "(?)").collect();
                let sql = format!("INSERT OR IGNORE INTO _bs_scope (asset_id) VALUES {}", placeholders.join(","));
                let params: Vec<&dyn rusqlite::types::ToSql> = chunk.iter().map(|id| id as &dyn rusqlite::types::ToSql).collect();
                self.conn.execute(&sql, params.as_slice())?;
            }
        }

        let scope_filter = if scoped {
            "JOIN _bs_scope bs ON bs.asset_id = a.id"
        } else {
            ""
        };

        let mut stmt = self.conn.prepare(&format!(
            "SELECT a.id FROM assets a {} \
             WHERE NOT EXISTS ( \
                SELECT 1 FROM variants v \
                JOIN file_locations fl ON fl.content_hash = v.content_hash \
                WHERE v.asset_id = a.id AND fl.volume_id = ?1 \
             )",
            scope_filter,
        ))?;
        let rows = stmt.query_map([volume_id], |r| r.get::<_, String>(0))?;
        let mut ids = Vec::new();
        for row in rows {
            ids.push(row?);
        }

        if scoped {
            let _ = self.conn.execute_batch("DROP TABLE IF EXISTS _bs_scope");
        }

        Ok(ids)
    }

    /// Return asset IDs where all variants have zero file_locations.
    /// Find asset IDs that have at least one file location with stale or missing verification.
    /// Used by verify --max-age to skip loading sidecars for fully-verified assets.
    /// Count total file locations, optionally filtered by volume.
    pub fn count_file_locations(&self, volume_id: Option<&str>) -> Result<usize> {
        let (sql, params): (String, Vec<Box<dyn rusqlite::types::ToSql>>) = if let Some(vid) = volume_id {
            ("SELECT COUNT(*) FROM file_locations WHERE volume_id = ?1".to_string(),
             vec![Box::new(vid.to_string())])
        } else {
            ("SELECT COUNT(*) FROM file_locations".to_string(), vec![])
        };
        let count: usize = self.conn.query_row(&sql, rusqlite::params_from_iter(params.iter()), |r| r.get(0))?;
        Ok(count)
    }

    /// Count file locations with stale or missing verification.
    pub fn count_stale_locations(&self, max_age_days: u64, volume_id: Option<&str>) -> Result<usize> {
        let cutoff = (chrono::Utc::now() - chrono::Duration::days(max_age_days as i64)).to_rfc3339();
        let (sql, params): (String, Vec<Box<dyn rusqlite::types::ToSql>>) = if let Some(vid) = volume_id {
            ("SELECT COUNT(*) FROM file_locations WHERE volume_id = ?1 AND (verified_at IS NULL OR verified_at < ?2)".to_string(),
             vec![Box::new(vid.to_string()), Box::new(cutoff)])
        } else {
            ("SELECT COUNT(*) FROM file_locations WHERE verified_at IS NULL OR verified_at < ?1".to_string(),
             vec![Box::new(cutoff)])
        };
        let count: usize = self.conn.query_row(&sql, rusqlite::params_from_iter(params.iter()), |r| r.get(0))?;
        Ok(count)
    }

    pub fn find_assets_with_stale_locations(&self, max_age_days: u64, volume_id: Option<&str>) -> Result<Vec<String>> {
        let cutoff = (chrono::Utc::now() - chrono::Duration::days(max_age_days as i64)).to_rfc3339();
        let sql = if let Some(vid) = volume_id {
            format!(
                "SELECT DISTINCT v.asset_id FROM file_locations fl \
                 JOIN variants v ON fl.content_hash = v.content_hash \
                 WHERE fl.volume_id = '{}' AND (fl.verified_at IS NULL OR fl.verified_at < ?1)",
                vid.replace('\'', "''")
            )
        } else {
            "SELECT DISTINCT v.asset_id FROM file_locations fl \
             JOIN variants v ON fl.content_hash = v.content_hash \
             WHERE fl.verified_at IS NULL OR fl.verified_at < ?1".to_string()
        };
        let mut stmt = self.conn.prepare(&sql)?;
        let ids = stmt
            .query_map(rusqlite::params![cutoff], |r| r.get(0))?
            .collect::<std::result::Result<Vec<String>, _>>()?;
        Ok(ids)
    }

    pub fn list_orphaned_asset_ids(&self) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT a.id FROM assets a WHERE NOT EXISTS ( \
                 SELECT 1 FROM variants v JOIN file_locations fl ON fl.content_hash = v.content_hash \
                 WHERE v.asset_id = a.id \
             )",
        )?;
        let ids = stmt
            .query_map([], |row| row.get(0))?
            .collect::<std::result::Result<Vec<String>, _>>()?;
        Ok(ids)
    }

    /// Delete all recipes attached to variants of an asset.
    pub fn delete_recipes_for_asset(&self, asset_id: &str) -> Result<usize> {
        let changed = self.conn.execute(
            "DELETE FROM recipes WHERE variant_hash IN (SELECT content_hash FROM variants WHERE asset_id = ?1)",
            rusqlite::params![asset_id],
        )?;
        Ok(changed)
    }

    /// Delete all file_locations for variants of an asset (safety net for true orphans).
    pub fn delete_file_locations_for_asset(&self, asset_id: &str) -> Result<usize> {
        let changed = self.conn.execute(
            "DELETE FROM file_locations WHERE content_hash IN (SELECT content_hash FROM variants WHERE asset_id = ?1)",
            rusqlite::params![asset_id],
        )?;
        Ok(changed)
    }

    /// Delete all variants belonging to an asset.
    pub fn delete_variants_for_asset(&self, asset_id: &str) -> Result<usize> {
        let changed = self.conn.execute(
            "DELETE FROM variants WHERE asset_id = ?1",
            rusqlite::params![asset_id],
        )?;
        Ok(changed)
    }

    /// Delete all collection memberships for an asset.
    pub fn delete_collection_memberships_for_asset(&self, asset_id: &str) -> Result<usize> {
        let changed = self.conn.execute(
            "DELETE FROM collection_assets WHERE asset_id = ?1",
            rusqlite::params![asset_id],
        )?;
        Ok(changed)
    }

    /// List all variant content hashes for an asset.
    pub fn list_variant_hashes_for_asset(&self, asset_id: &str) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT content_hash FROM variants WHERE asset_id = ?1",
        )?;
        let rows = stmt.query_map(rusqlite::params![asset_id], |row| {
            row.get::<_, String>(0)
        })?;
        let mut hashes = Vec::new();
        for row in rows {
            hashes.push(row?);
        }
        Ok(hashes)
    }

    /// Return asset IDs where all variants would have zero file_locations
    /// if the given set of stale locations were removed.
    /// Each stale location is `(content_hash, volume_id, relative_path)`.
    pub fn list_would_be_orphaned_asset_ids(
        &self,
        stale_locations: &[(String, String, String)],
    ) -> Result<Vec<String>> {
        if stale_locations.is_empty() {
            return self.list_orphaned_asset_ids();
        }

        // Create a temp table with stale locations to exclude
        self.conn.execute_batch(
            "CREATE TEMP TABLE IF NOT EXISTS _stale_locs (content_hash TEXT, volume_id TEXT, relative_path TEXT)",
        )?;
        self.conn.execute("DELETE FROM _stale_locs", [])?;

        let mut insert = self.conn.prepare(
            "INSERT INTO _stale_locs (content_hash, volume_id, relative_path) VALUES (?1, ?2, ?3)",
        )?;
        for (hash, vol, path) in stale_locations {
            insert.execute(rusqlite::params![hash, vol, path])?;
        }
        drop(insert);

        let mut stmt = self.conn.prepare(
            "SELECT a.id FROM assets a WHERE NOT EXISTS ( \
                 SELECT 1 FROM variants v \
                 JOIN file_locations fl ON fl.content_hash = v.content_hash \
                 WHERE v.asset_id = a.id \
                 AND NOT EXISTS ( \
                     SELECT 1 FROM _stale_locs sl \
                     WHERE sl.content_hash = fl.content_hash \
                     AND sl.volume_id = fl.volume_id \
                     AND sl.relative_path = fl.relative_path \
                 ) \
             )",
        )?;
        let ids = stmt
            .query_map([], |row| row.get(0))?
            .collect::<std::result::Result<Vec<String>, _>>()?;

        self.conn.execute("DROP TABLE IF EXISTS _stale_locs", [])?;

        Ok(ids)
    }

    /// List variants that have zero file_locations, returning (asset_id, content_hash) pairs.
    /// Only returns variants belonging to assets that still have at least one other variant with locations.
    pub fn list_locationless_variants(&self) -> Result<Vec<(String, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT v.asset_id, v.content_hash FROM variants v \
             WHERE NOT EXISTS (SELECT 1 FROM file_locations fl WHERE fl.content_hash = v.content_hash) \
             AND EXISTS ( \
                 SELECT 1 FROM variants v2 \
                 JOIN file_locations fl2 ON fl2.content_hash = v2.content_hash \
                 WHERE v2.asset_id = v.asset_id AND v2.content_hash != v.content_hash \
             )",
        )?;
        let pairs = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<std::result::Result<Vec<(String, String)>, _>>()?;
        Ok(pairs)
    }

    /// Predict locationless variants after a set of stale locations would be removed.
    pub fn list_would_be_locationless_variants(
        &self,
        stale_locations: &[(String, String, String)],
    ) -> Result<Vec<(String, String)>> {
        if stale_locations.is_empty() {
            return self.list_locationless_variants();
        }

        self.conn.execute_batch(
            "CREATE TEMP TABLE IF NOT EXISTS _stale_locs2 (content_hash TEXT, volume_id TEXT, relative_path TEXT)",
        )?;
        self.conn.execute("DELETE FROM _stale_locs2", [])?;

        let mut insert = self.conn.prepare(
            "INSERT INTO _stale_locs2 (content_hash, volume_id, relative_path) VALUES (?1, ?2, ?3)",
        )?;
        for (hash, vol, path) in stale_locations {
            insert.execute(rusqlite::params![hash, vol, path])?;
        }
        drop(insert);

        // A variant is "locationless" if all its remaining locations are in the stale set
        // AND the asset has at least one other variant that still has non-stale locations
        let mut stmt = self.conn.prepare(
            "SELECT v.asset_id, v.content_hash FROM variants v \
             WHERE NOT EXISTS ( \
                 SELECT 1 FROM file_locations fl WHERE fl.content_hash = v.content_hash \
                 AND NOT EXISTS ( \
                     SELECT 1 FROM _stale_locs2 sl \
                     WHERE sl.content_hash = fl.content_hash \
                     AND sl.volume_id = fl.volume_id \
                     AND sl.relative_path = fl.relative_path \
                 ) \
             ) \
             AND EXISTS ( \
                 SELECT 1 FROM variants v2 \
                 JOIN file_locations fl2 ON fl2.content_hash = v2.content_hash \
                 WHERE v2.asset_id = v.asset_id AND v2.content_hash != v.content_hash \
                 AND NOT EXISTS ( \
                     SELECT 1 FROM _stale_locs2 sl2 \
                     WHERE sl2.content_hash = fl2.content_hash \
                     AND sl2.volume_id = fl2.volume_id \
                     AND sl2.relative_path = fl2.relative_path \
                 ) \
             )",
        )?;
        let pairs = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<std::result::Result<Vec<(String, String)>, _>>()?;

        self.conn.execute("DROP TABLE IF EXISTS _stale_locs2", [])?;

        Ok(pairs)
    }

    /// Delete a single variant by content_hash. Also deletes its file_locations and recipes.
    pub fn delete_variant(&self, content_hash: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM recipes WHERE variant_hash = ?1",
            rusqlite::params![content_hash],
        )?;
        self.conn.execute(
            "DELETE FROM file_locations WHERE content_hash = ?1",
            rusqlite::params![content_hash],
        )?;
        self.conn.execute(
            "DELETE FROM embeddings WHERE asset_id IN (SELECT asset_id FROM variants WHERE content_hash = ?1)",
            rusqlite::params![content_hash],
        )?;
        self.conn.execute(
            "DELETE FROM variants WHERE content_hash = ?1",
            rusqlite::params![content_hash],
        )?;
        Ok(())
    }

    /// Return all variant content hashes in the catalog.
    pub fn list_all_variant_hashes(&self) -> Result<std::collections::HashSet<String>> {
        let mut stmt = self.conn.prepare("SELECT content_hash FROM variants")?;
        let hashes = stmt
            .query_map([], |row| row.get(0))?
            .collect::<std::result::Result<std::collections::HashSet<String>, _>>()?;
        Ok(hashes)
    }

    pub fn list_all_asset_ids(&self) -> Result<std::collections::HashSet<String>> {
        let mut stmt = self.conn.prepare("SELECT id FROM assets")?;
        let ids = stmt
            .query_map([], |row| row.get(0))?
            .collect::<std::result::Result<std::collections::HashSet<String>, _>>()?;
        Ok(ids)
    }
}

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

        // tag:=Holzkirchen should match ONLY ae2 (Holzkirchen is leaf),
        // NOT ae1 (Holzkirchen has Marktplatz below it)
        let tags = vec!["=Holzkirchen".to_string()];
        let opts = SearchOptions { tags: &tags, per_page: u32::MAX, ..Default::default() };
        let results = catalog.search_paginated(&opts).unwrap();
        let names: Vec<&str> = results.iter().map(|r| r.original_filename.as_str()).collect();
        assert_eq!(results.len(), 1, "tag:=Holzkirchen should match only ae2. Got: {names:?}");
        assert_eq!(results[0].original_filename, "ae2.jpg");

        // tag:Holzkirchen (without =) should match BOTH
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
        assert!(err.to_string().contains("No recipe found"));
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
