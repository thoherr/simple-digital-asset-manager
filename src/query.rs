use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::Result;

use crate::catalog::{AssetDetails, Catalog, SearchOptions, SearchRow};
use crate::content_store::ContentStore;
use crate::device_registry::DeviceRegistry;
use crate::metadata_store::MetadataStore;
use crate::models::volume::Volume;
use crate::models::Asset;
use crate::xmp_reader;

/// Parsed search query with all supported filter prefixes.
#[derive(Debug, Default)]
pub struct ParsedSearch {
    pub text: Option<String>,
    pub asset_type: Option<String>,
    pub tag: Option<String>,
    pub format: Option<String>,
    pub rating_min: Option<u8>,
    pub rating_exact: Option<u8>,
    pub camera: Option<String>,
    pub lens: Option<String>,
    pub iso_min: Option<i64>,
    pub iso_max: Option<i64>,
    pub focal_min: Option<f64>,
    pub focal_max: Option<f64>,
    pub f_min: Option<f64>,
    pub f_max: Option<f64>,
    pub width_min: Option<i64>,
    pub height_min: Option<i64>,
    pub meta_filters: Vec<(String, String)>,
    pub orphan: bool,
    pub stale_days: Option<u64>,
    pub missing: bool,
    pub volume_none: bool,
    pub color_label: Option<String>,
    pub collection: Option<String>,
    pub path_prefix: Option<String>,
    pub copies_exact: Option<u64>,
    pub copies_min: Option<u64>,
    pub date_prefix: Option<String>,
    pub date_from: Option<String>,
    pub date_until: Option<String>,
}

impl ParsedSearch {
    /// Convert to `SearchOptions` for passing to catalog search methods.
    pub fn to_search_options(&self) -> SearchOptions<'_> {
        SearchOptions {
            text: self.text.as_deref(),
            asset_type: self.asset_type.as_deref(),
            tag: self.tag.as_deref(),
            format: self.format.as_deref(),
            rating_min: self.rating_min,
            rating_exact: self.rating_exact,
            camera: self.camera.as_deref(),
            lens: self.lens.as_deref(),
            iso_min: self.iso_min,
            iso_max: self.iso_max,
            focal_min: self.focal_min,
            focal_max: self.focal_max,
            f_min: self.f_min,
            f_max: self.f_max,
            width_min: self.width_min,
            height_min: self.height_min,
            meta_filters: self
                .meta_filters
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str()))
                .collect(),
            orphan: self.orphan,
            stale_days: self.stale_days,
            color_label: self.color_label.as_deref(),
            path_prefix: self.path_prefix.as_deref(),
            copies_exact: self.copies_exact,
            copies_min: self.copies_min,
            date_prefix: self.date_prefix.as_deref(),
            date_from: self.date_from.as_deref(),
            date_until: self.date_until.as_deref(),
            ..Default::default()
        }
    }
}

/// Tokenize a search query respecting double-quoted values.
///
/// Splits on whitespace, but `prefix:"multi word value"` stays as a single token
/// with quotes stripped from the value. Unquoted tokens work as before.
///
/// Examples:
///   `tag:"Fools Theater" rating:4+` → `["tag:Fools Theater", "rating:4+"]`
///   `tag:landscape type:image`      → `["tag:landscape", "type:image"]`
///   `hello world`                   → `["hello", "world"]`
fn tokenize_query(query: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut chars = query.chars().peekable();

    while chars.peek().is_some() {
        // Skip whitespace
        while chars.peek().map_or(false, |c| c.is_whitespace()) {
            chars.next();
        }
        if chars.peek().is_none() {
            break;
        }

        let mut token = String::new();
        let mut in_quotes = false;

        while let Some(&c) = chars.peek() {
            if in_quotes {
                chars.next();
                if c == '"' {
                    in_quotes = false;
                } else {
                    token.push(c);
                }
            } else if c == '"' {
                chars.next();
                in_quotes = true;
            } else if c.is_whitespace() {
                break;
            } else {
                chars.next();
                token.push(c);
            }
        }

        if !token.is_empty() {
            tokens.push(token);
        }
    }

    tokens
}

/// Parse a search query string into structured filters.
///
/// Supports prefix filters: `type:image`, `tag:landscape`, `format:jpg`, `rating:3+`,
/// `camera:fuji`, `lens:56mm`, `iso:3200`, `iso:100-800`, `focal:50`, `focal:35-70`,
/// `f:2.8`, `f:1.4-2.8`, `width:4000+`, `height:2000+`, `meta:key=value`.
/// Values with spaces can be quoted: `tag:"Fools Theater"`, `camera:"Canon EOS R5"`.
/// Remaining tokens are joined as free-text search.
pub fn parse_search_query(query: &str) -> ParsedSearch {
    let mut parsed = ParsedSearch::default();
    let mut text_parts = Vec::new();

    for token in tokenize_query(query) {
        if let Some(value) = token.strip_prefix("type:") {
            parsed.asset_type = Some(value.to_string());
        } else if let Some(value) = token.strip_prefix("tag:") {
            parsed.tag = Some(value.to_string());
        } else if let Some(value) = token.strip_prefix("format:") {
            parsed.format = Some(value.to_string());
        } else if let Some(value) = token.strip_prefix("rating:") {
            if let Some(num_str) = value.strip_suffix('+') {
                if let Ok(n) = num_str.parse::<u8>() {
                    parsed.rating_min = Some(n);
                }
            } else if let Ok(n) = value.parse::<u8>() {
                parsed.rating_exact = Some(n);
            }
        } else if let Some(value) = token.strip_prefix("camera:") {
            parsed.camera = Some(value.to_string());
        } else if let Some(value) = token.strip_prefix("lens:") {
            parsed.lens = Some(value.to_string());
        } else if let Some(value) = token.strip_prefix("iso:") {
            parse_int_range(&value, &mut parsed.iso_min, &mut parsed.iso_max);
        } else if let Some(value) = token.strip_prefix("focal:") {
            parse_float_range(&value, &mut parsed.focal_min, &mut parsed.focal_max);
        } else if let Some(value) = token.strip_prefix("f:") {
            parse_float_range(&value, &mut parsed.f_min, &mut parsed.f_max);
        } else if let Some(value) = token.strip_prefix("width:") {
            if let Some(num_str) = value.strip_suffix('+') {
                if let Ok(n) = num_str.parse::<i64>() {
                    parsed.width_min = Some(n);
                }
            } else if let Ok(n) = value.parse::<i64>() {
                parsed.width_min = Some(n);
            }
        } else if let Some(value) = token.strip_prefix("height:") {
            if let Some(num_str) = value.strip_suffix('+') {
                if let Ok(n) = num_str.parse::<i64>() {
                    parsed.height_min = Some(n);
                }
            } else if let Ok(n) = value.parse::<i64>() {
                parsed.height_min = Some(n);
            }
        } else if let Some(value) = token.strip_prefix("meta:") {
            if let Some((key, val)) = value.split_once('=') {
                parsed.meta_filters.push((key.to_string(), val.to_string()));
            }
        } else if token == "orphan:true" {
            parsed.orphan = true;
        } else if token == "missing:true" {
            parsed.missing = true;
        } else if let Some(value) = token.strip_prefix("stale:") {
            if let Ok(days) = value.parse::<u64>() {
                parsed.stale_days = Some(days);
            }
        } else if token == "volume:none" {
            parsed.volume_none = true;
        } else if let Some(value) = token.strip_prefix("label:") {
            parsed.color_label = Some(value.to_string());
        } else if let Some(value) = token.strip_prefix("collection:") {
            parsed.collection = Some(value.to_string());
        } else if let Some(value) = token.strip_prefix("path:") {
            parsed.path_prefix = Some(value.to_string());
        } else if let Some(value) = token.strip_prefix("copies:") {
            if let Some(num_str) = value.strip_suffix('+') {
                parsed.copies_min = num_str.parse().ok();
            } else {
                parsed.copies_exact = value.parse().ok();
            }
        } else if let Some(value) = token.strip_prefix("date:") {
            parsed.date_prefix = Some(value.to_string());
        } else if let Some(value) = token.strip_prefix("dateFrom:") {
            parsed.date_from = Some(value.to_string());
        } else if let Some(value) = token.strip_prefix("dateUntil:") {
            parsed.date_until = Some(value.to_string());
        } else {
            text_parts.push(token);
        }
    }

    if !text_parts.is_empty() {
        parsed.text = Some(text_parts.join(" "));
    }

    parsed
}

/// Parse an integer range value: "3200" (exact), "3200+" (min), "100-800" (range).
fn parse_int_range(value: &str, min: &mut Option<i64>, max: &mut Option<i64>) {
    if let Some(num_str) = value.strip_suffix('+') {
        if let Ok(n) = num_str.parse::<i64>() {
            *min = Some(n);
        }
    } else if let Some((lo, hi)) = value.split_once('-') {
        if let (Ok(lo_n), Ok(hi_n)) = (lo.parse::<i64>(), hi.parse::<i64>()) {
            *min = Some(lo_n);
            *max = Some(hi_n);
        }
    } else if let Ok(n) = value.parse::<i64>() {
        *min = Some(n);
        *max = Some(n);
    }
}

/// Parse a float range value: "2.8" (exact), "2.8+" (min), "1.4-2.8" (range).
fn parse_float_range(value: &str, min: &mut Option<f64>, max: &mut Option<f64>) {
    if let Some(num_str) = value.strip_suffix('+') {
        if let Ok(n) = num_str.parse::<f64>() {
            *min = Some(n);
        }
    } else if let Some((lo, hi)) = value.split_once('-') {
        if let (Ok(lo_n), Ok(hi_n)) = (lo.parse::<f64>(), hi.parse::<f64>()) {
            *min = Some(lo_n);
            *max = Some(hi_n);
        }
    } else if let Ok(n) = value.parse::<f64>() {
        *min = Some(n);
        *max = Some(n);
    }
}

/// Check if `short` is a prefix-match for `long` with a separator boundary.
///
/// Returns true if `short == long` (exact match) or if `long` starts with `short`
/// and the character immediately following in `long` is non-alphanumeric.
/// This prevents `DSC_001` from matching `DSC_0010` while allowing
/// `DSC_001` to match `DSC_001-Edit` or `DSC_001_v2`.
fn stem_prefix_matches(short: &str, long: &str) -> bool {
    if short == long {
        return true;
    }
    if !long.starts_with(short) {
        return false;
    }
    // The character right after the prefix must be a non-alphanumeric separator
    match long[short.len()..].chars().next() {
        Some(c) => !c.is_alphanumeric(),
        None => true,
    }
}

/// Result of a group operation.
#[derive(Debug)]
pub struct GroupResult {
    /// The asset ID that all variants were merged into.
    pub target_id: String,
    /// Number of variants moved from donor assets.
    pub variants_moved: usize,
    /// Number of donor assets removed.
    pub donors_removed: usize,
}

/// One stem group found by `auto_group`.
#[derive(Debug, serde::Serialize)]
pub struct StemGroupEntry {
    pub stem: String,
    pub target_id: String,
    pub asset_ids: Vec<String>,
    pub donor_count: usize,
}

/// Result of an auto-group operation.
#[derive(Debug, serde::Serialize)]
pub struct AutoGroupResult {
    pub groups: Vec<StemGroupEntry>,
    pub total_donors_merged: usize,
    pub total_variants_moved: usize,
    pub dry_run: bool,
}

/// Fields to edit on an asset. `None` = no change, `Some(None)` = clear, `Some(Some(x))` = set.
pub struct EditFields {
    pub name: Option<Option<String>>,
    pub description: Option<Option<String>>,
    pub rating: Option<Option<u8>>,
    pub color_label: Option<Option<String>>,
}

/// Result of an edit operation.
#[derive(Debug, serde::Serialize)]
pub struct EditResult {
    pub asset_id: String,
    pub name: Option<String>,
    pub description: Option<String>,
    pub rating: Option<u8>,
    pub color_label: Option<String>,
}

/// Result of a tag add/remove operation.
pub struct TagResult {
    /// Tags that were actually added or removed.
    pub changed: Vec<String>,
    /// The full set of tags after the operation.
    pub current_tags: Vec<String>,
}

/// Resolve and normalize a `path:` filter value for search.
///
/// When `cwd` is provided (CLI context):
/// - `~` or `~/...` is expanded to the user's home directory
/// - `./...` or `../...` is resolved relative to `cwd`
///
/// After resolution, if the path is absolute and matches a volume mount point
/// (longest prefix match), returns (volume-relative path, Some(volume_id)).
/// Otherwise returns (path, None) unchanged.
pub fn normalize_path_for_search(
    path: &str,
    volumes: &[Volume],
    cwd: Option<&std::path::Path>,
) -> (String, Option<String>) {
    // Step 1: Expand ~ and resolve ./ ../ when cwd is available
    let resolved = if let Some(cwd) = cwd {
        if path == "~" {
            std::env::var("HOME")
                .map(|h| h.to_string())
                .unwrap_or_else(|_| path.to_string())
        } else if let Some(rest) = path.strip_prefix("~/") {
            std::env::var("HOME")
                .map(|h| std::path::PathBuf::from(h).join(rest).to_string_lossy().to_string())
                .unwrap_or_else(|_| path.to_string())
        } else if path.starts_with("./") || path.starts_with("../") {
            let joined = cwd.join(path);
            // Clean the path components (handle ./ and ../) without requiring
            // the path to exist on disk (unlike canonicalize)
            clean_path(&joined)
        } else {
            path.to_string()
        }
    } else {
        path.to_string()
    };

    // Step 2: If absolute, try to match a volume mount point
    let p = std::path::Path::new(&resolved);
    if !p.is_absolute() {
        return (resolved, None);
    }

    let mut best: Option<&Volume> = None;
    let mut best_len = 0;

    for v in volumes {
        if p.starts_with(&v.mount_point) {
            let len = v.mount_point.as_os_str().len();
            if len > best_len {
                best = Some(v);
                best_len = len;
            }
        }
    }

    match best {
        Some(vol) => {
            let relative = p
                .strip_prefix(&vol.mount_point)
                .unwrap()
                .to_string_lossy()
                .to_string();
            (relative, Some(vol.id.to_string()))
        }
        None => (resolved, None),
    }
}

/// Logically clean a path by resolving `.` and `..` components without
/// touching the filesystem (unlike `canonicalize` which requires the path to exist).
fn clean_path(path: &std::path::Path) -> String {
    let mut parts: Vec<&std::ffi::OsStr> = Vec::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {} // skip .
            std::path::Component::ParentDir => {
                parts.pop(); // go up
            }
            other => parts.push(other.as_os_str()),
        }
    }
    let result: std::path::PathBuf = parts.iter().collect();
    result.to_string_lossy().to_string()
}

/// Search and filter assets via the SQLite catalog.
pub struct QueryEngine {
    catalog_root: std::path::PathBuf,
}

impl QueryEngine {
    pub fn new(catalog_root: &Path) -> Self {
        Self {
            catalog_root: catalog_root.to_path_buf(),
        }
    }

    /// Search assets by a free-text query string.
    ///
    /// Supports prefix filters: `type:image`, `tag:landscape`, `format:jpg`, `rating:3+`,
    /// `camera:fuji`, `lens:56mm`, `iso:3200`, `focal:50`, `f:2.8`, `width:4000+`,
    /// `height:2000+`, `meta:key=value`.
    /// Remaining tokens are joined as free-text search against name/filename/description/metadata.
    pub fn search(&self, query: &str) -> Result<Vec<SearchRow>> {
        let mut parsed = parse_search_query(query);

        // Normalize path: ~, ./, ../, /absolute → volume-relative + volume filter
        let path_volume_id: Option<String>;
        if parsed.path_prefix.is_some() {
            let registry = DeviceRegistry::new(&self.catalog_root);
            let volumes = registry.list()?;
            let cwd = std::env::current_dir().ok();
            let (normalized, vol_id) = normalize_path_for_search(
                parsed.path_prefix.as_deref().unwrap(),
                &volumes,
                cwd.as_deref(),
            );
            parsed.path_prefix = Some(normalized);
            path_volume_id = vol_id;
        } else {
            path_volume_id = None;
        }

        let mut opts = SearchOptions {
            per_page: u32::MAX,
            ..parsed.to_search_options()
        };

        if let Some(ref vid) = path_volume_id {
            opts.volume = Some(vid);
        }

        let catalog = Catalog::open(&self.catalog_root)?;

        // Pre-compute missing asset IDs if needed (requires disk I/O)
        let missing_ids;
        if parsed.missing {
            let registry = DeviceRegistry::new(&self.catalog_root);
            let volumes = registry.list()?;
            let online: HashMap<String, std::path::PathBuf> = volumes
                .iter()
                .filter(|v| v.is_online)
                .map(|v| (v.id.to_string(), v.mount_point.clone()))
                .collect();
            let all_locs = catalog.list_all_locations_with_assets()?;
            let mut ids = HashSet::new();
            for (asset_id, volume_id, relative_path) in &all_locs {
                if let Some(mount) = online.get(volume_id) {
                    if !mount.join(relative_path).exists() {
                        ids.insert(asset_id.clone());
                    }
                }
            }
            missing_ids = ids.into_iter().collect::<Vec<_>>();
            opts.missing_asset_ids = Some(&missing_ids);
        }

        // Pre-compute collection asset IDs
        let collection_ids;
        if let Some(ref col_name) = parsed.collection {
            let store = crate::collection::CollectionStore::new(catalog.conn());
            collection_ids = store.asset_ids_for_collection(col_name)?;
            opts.collection_asset_ids = Some(&collection_ids);
        }

        // Pre-compute online volume IDs for volume:none
        let online_vol_ids;
        if parsed.volume_none {
            let registry = DeviceRegistry::new(&self.catalog_root);
            let volumes = registry.list()?;
            online_vol_ids = volumes
                .iter()
                .filter(|v| v.is_online)
                .map(|v| v.id.to_string())
                .collect::<Vec<_>>();
            opts.no_online_locations = Some(&online_vol_ids);
        }

        catalog.search_paginated(&opts)
    }

    /// Look up a single asset by its full ID or a unique prefix.
    pub fn show(&self, asset_id_prefix: &str) -> Result<AssetDetails> {
        let catalog = Catalog::open(&self.catalog_root)?;
        let full_id = catalog
            .resolve_asset_id(asset_id_prefix)?
            .ok_or_else(|| anyhow::anyhow!("No asset found matching '{asset_id_prefix}'"))?;
        catalog
            .load_asset_details(&full_id)?
            .ok_or_else(|| anyhow::anyhow!("Asset '{full_id}' not found in catalog"))
    }

    /// Group variants (identified by content hashes) into a single asset.
    ///
    /// Picks the oldest asset as the target, moves all other variants into it,
    /// merges tags, and deletes donor assets.
    pub fn group(&self, variant_hashes: &[String]) -> Result<GroupResult> {
        if variant_hashes.is_empty() {
            anyhow::bail!("No variant hashes provided");
        }

        let catalog = Catalog::open(&self.catalog_root)?;
        let store = MetadataStore::new(&self.catalog_root);

        // Step 1: Look up owning asset for each hash
        let mut asset_ids = Vec::new();
        for hash in variant_hashes {
            let asset_id = catalog
                .find_asset_id_by_variant(hash)?
                .ok_or_else(|| anyhow::anyhow!("No variant found with hash '{hash}'"))?;
            asset_ids.push(asset_id);
        }

        // Step 2: Collect unique asset IDs
        let unique_ids: Vec<String> = {
            let mut seen = HashSet::new();
            asset_ids
                .iter()
                .filter(|id| seen.insert((*id).clone()))
                .cloned()
                .collect()
        };

        if unique_ids.len() == 1 {
            return Ok(GroupResult {
                target_id: unique_ids.into_iter().next().unwrap(),
                variants_moved: 0,
                donors_removed: 0,
            });
        }

        // Step 3: Load all assets from sidecar, pick oldest as target
        let mut assets: Vec<crate::models::Asset> = unique_ids
            .iter()
            .map(|id| {
                let uuid: uuid::Uuid = id.parse()?;
                store.load(uuid)
            })
            .collect::<Result<_>>()?;

        assets.sort_by_key(|a| a.created_at);
        let target_id = assets[0].id;
        let mut target = assets.remove(0);
        let donors = assets; // remaining are donors

        // Step 4: Merge variants and tags from donors into target
        let mut variants_moved = 0;
        let existing_tags: HashSet<String> = target.tags.iter().cloned().collect();
        let mut all_tags = existing_tags;

        for donor in &donors {
            for variant in &donor.variants {
                let mut moved_variant = variant.clone();
                moved_variant.asset_id = target_id;
                // Donor's "original" variants become exports in the target asset
                if moved_variant.role == crate::models::VariantRole::Original {
                    moved_variant.role = crate::models::VariantRole::Export;
                }
                target.variants.push(moved_variant);
                variants_moved += 1;
            }
            for tag in &donor.tags {
                if all_tags.insert(tag.clone()) {
                    target.tags.push(tag.clone());
                }
            }
            for recipe in &donor.recipes {
                target.recipes.push(recipe.clone());
            }
        }

        // Step 5: Save target sidecar and update catalog
        store.save(&target)?;
        catalog.insert_asset(&target)?;

        // Step 6: Update variant rows in catalog and clean up donors
        for donor in &donors {
            for variant in &donor.variants {
                catalog.update_variant_asset_id(
                    &variant.content_hash,
                    &target_id.to_string(),
                )?;
                // Re-role originals to exports in the catalog too
                if variant.role == crate::models::VariantRole::Original {
                    catalog.update_variant_role(&variant.content_hash, "export")?;
                }
            }
            store.delete(donor.id)?;
            catalog.delete_asset(&donor.id.to_string())?;
        }

        let donors_removed = donors.len();

        Ok(GroupResult {
            target_id: target_id.to_string(),
            variants_moved,
            donors_removed,
        })
    }

    /// Auto-group assets by filename stem using fuzzy prefix matching.
    ///
    /// Two stems match if the shorter is a prefix of the longer and the next
    /// character in the longer string is non-alphanumeric (a separator like
    /// `-`, `_`, ` `, `(`, etc.). This handles the common case where export
    /// tools append suffixes to the original filename:
    /// `Z91_8561.ARW` → `Z91_8561-1-HighRes-(c)_2025_Name.tif`.
    ///
    /// Picks the best target per group (RAW preferred, then oldest) and merges.
    pub fn auto_group(&self, asset_ids: &[String], dry_run: bool) -> Result<AutoGroupResult> {
        let catalog = Catalog::open(&self.catalog_root)?;

        // Deduplicate input IDs
        let unique_ids: Vec<String> = {
            let mut seen = HashSet::new();
            asset_ids
                .iter()
                .filter(|id| seen.insert((*id).clone()))
                .cloned()
                .collect()
        };

        // Load details for each asset and extract stem
        struct StemEntry {
            stem: String,
            asset_id: String,
            details: crate::catalog::AssetDetails,
        }
        let mut entries: Vec<StemEntry> = Vec::new();
        for id in &unique_ids {
            let details = match catalog.load_asset_details(id)? {
                Some(d) => d,
                None => continue,
            };
            let stem = if let Some(v) = details.variants.first() {
                std::path::Path::new(&v.original_filename)
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_uppercase())
                    .unwrap_or_default()
            } else {
                continue;
            };
            if stem.is_empty() {
                continue;
            }
            entries.push(StemEntry { stem, asset_id: id.clone(), details });
        }

        // Sort by stem length (shortest first) for prefix resolution
        entries.sort_by_key(|e| e.stem.len());

        // Resolve each stem to its root (shortest valid prefix-match)
        let mut roots: Vec<String> = Vec::new();
        let mut stem_to_root: HashMap<String, String> = HashMap::new();

        for entry in &entries {
            let stem = &entry.stem;
            if stem_to_root.contains_key(stem) {
                // Another asset with the same stem already resolved
                continue;
            }
            let mut found_root = None;
            for root in &roots {
                if stem_prefix_matches(root, stem) {
                    found_root = Some(root.clone());
                    break; // first (shortest) root wins
                }
            }
            match found_root {
                Some(root) => {
                    stem_to_root.insert(stem.clone(), root);
                }
                None => {
                    roots.push(stem.clone());
                    stem_to_root.insert(stem.clone(), stem.clone());
                }
            }
        }

        // Group assets by resolved root stem
        let mut group_map: HashMap<String, Vec<(String, crate::catalog::AssetDetails)>> =
            HashMap::new();
        for entry in entries {
            let root = stem_to_root.get(&entry.stem).unwrap();
            group_map
                .entry(root.clone())
                .or_default()
                .push((entry.asset_id, entry.details));
        }

        // Filter to groups with >1 distinct asset and merge
        let mut groups = Vec::new();
        let mut total_donors_merged = 0;
        let mut total_variants_moved = 0;

        for (root_stem, mut entries) in group_map {
            if entries.len() < 2 {
                continue;
            }

            // Sort: prefer asset with RAW variant, then oldest by created_at
            entries.sort_by(|a, b| {
                let a_raw = a.1.variants.iter().any(|v| {
                    crate::asset_service::is_raw_extension(&v.format)
                });
                let b_raw = b.1.variants.iter().any(|v| {
                    crate::asset_service::is_raw_extension(&v.format)
                });
                b_raw.cmp(&a_raw).then_with(|| a.1.created_at.cmp(&b.1.created_at))
            });

            let target_id = entries[0].0.clone();
            let all_ids: Vec<String> = entries.iter().map(|e| e.0.clone()).collect();
            let donor_count = entries.len() - 1;

            if !dry_run {
                let all_hashes: Vec<String> = entries
                    .iter()
                    .flat_map(|e| e.1.variants.iter().map(|v| v.content_hash.clone()))
                    .collect();
                let result = self.group(&all_hashes)?;
                total_variants_moved += result.variants_moved;
                total_donors_merged += result.donors_removed;
            } else {
                let donor_variants: usize = entries[1..]
                    .iter()
                    .map(|e| e.1.variants.len())
                    .sum();
                total_variants_moved += donor_variants;
                total_donors_merged += donor_count;
            }

            groups.push(StemGroupEntry {
                stem: root_stem,
                target_id,
                asset_ids: all_ids,
                donor_count,
            });
        }

        // Sort groups by stem for deterministic output
        groups.sort_by(|a, b| a.stem.cmp(&b.stem));

        Ok(AutoGroupResult {
            groups,
            total_donors_merged,
            total_variants_moved,
            dry_run,
        })
    }

    /// Add or remove tags on an asset. Updates both sidecar YAML and SQLite catalog.
    pub fn tag(&self, asset_id_prefix: &str, tags: &[String], remove: bool) -> Result<TagResult> {
        let catalog = Catalog::open(&self.catalog_root)?;
        let full_id = catalog
            .resolve_asset_id(asset_id_prefix)?
            .ok_or_else(|| anyhow::anyhow!("No asset found matching '{asset_id_prefix}'"))?;

        let uuid: uuid::Uuid = full_id.parse()?;
        let store = MetadataStore::new(&self.catalog_root);
        let mut asset = store.load(uuid)?;

        let changed;
        if remove {
            let to_remove: std::collections::HashSet<&str> =
                tags.iter().map(|s| s.as_str()).collect();
            let mut actually_removed = Vec::new();
            asset.tags.retain(|t| {
                if to_remove.contains(t.as_str()) {
                    actually_removed.push(t.clone());
                    false
                } else {
                    true
                }
            });
            changed = actually_removed;
        } else {
            let existing: std::collections::HashSet<String> =
                asset.tags.iter().cloned().collect();
            let mut added = Vec::new();
            for tag in tags {
                if !existing.contains(tag) {
                    asset.tags.push(tag.clone());
                    added.push(tag.clone());
                }
            }
            changed = added;
        }

        store.save(&asset)?;
        catalog.insert_asset(&asset)?;

        if !changed.is_empty() {
            let (to_add, to_remove) = if remove {
                (Vec::new(), changed.clone())
            } else {
                (changed.clone(), Vec::new())
            };
            self.write_back_tags_to_xmp(&mut asset, &to_add, &to_remove, &catalog, &store);
        }

        Ok(TagResult {
            changed,
            current_tags: asset.tags.clone(),
        })
    }

    /// Edit asset metadata (name, description, rating). Updates both sidecar YAML and SQLite.
    pub fn edit(&self, asset_id_prefix: &str, fields: EditFields) -> Result<EditResult> {
        let catalog = Catalog::open(&self.catalog_root)?;
        let full_id = catalog
            .resolve_asset_id(asset_id_prefix)?
            .ok_or_else(|| anyhow::anyhow!("No asset found matching '{asset_id_prefix}'"))?;

        let uuid: uuid::Uuid = full_id.parse()?;
        let store = MetadataStore::new(&self.catalog_root);
        let mut asset = store.load(uuid)?;

        if let Some(name) = &fields.name {
            asset.name = name.clone();
        }
        if let Some(description) = &fields.description {
            // Normalize empty string to None (clear)
            asset.description = description
                .as_ref()
                .filter(|s| !s.is_empty())
                .cloned();
        }
        let rating_changed = fields.rating.is_some();
        if let Some(rating) = &fields.rating {
            asset.rating = *rating;
        }
        let label_changed = fields.color_label.is_some();
        if let Some(label) = &fields.color_label {
            asset.color_label = label.clone();
        }

        store.save(&asset)?;
        catalog.insert_asset(&asset)?;

        if rating_changed {
            let rating = asset.rating;
            self.write_back_rating_to_xmp(&mut asset, rating, &catalog, &store);
        }

        if fields.description.is_some() {
            let desc = asset.description.clone();
            self.write_back_description_to_xmp(&mut asset, desc.as_deref(), &catalog, &store);
        }

        if label_changed {
            let label = asset.color_label.clone();
            self.write_back_label_to_xmp(&mut asset, label.as_deref(), &catalog, &store);
        }

        Ok(EditResult {
            asset_id: full_id,
            name: asset.name,
            description: asset.description,
            rating: asset.rating,
            color_label: asset.color_label,
        })
    }

    /// Set the name on an asset. Updates both sidecar YAML and SQLite catalog.
    /// No XMP write-back needed — name has no XMP equivalent.
    /// Returns the new name value.
    pub fn set_name(
        &self,
        asset_id_prefix: &str,
        name: Option<String>,
    ) -> Result<Option<String>> {
        let catalog = Catalog::open(&self.catalog_root)?;
        let full_id = catalog
            .resolve_asset_id(asset_id_prefix)?
            .ok_or_else(|| anyhow::anyhow!("No asset found matching '{asset_id_prefix}'"))?;

        let uuid: uuid::Uuid = full_id.parse()?;
        let store = MetadataStore::new(&self.catalog_root);
        let mut asset = store.load(uuid)?;

        asset.name = name;
        store.save(&asset)?;
        catalog.insert_asset(&asset)?;

        Ok(asset.name)
    }

    /// Set the rating on an asset. Updates both sidecar YAML and SQLite catalog.
    /// Also writes back the rating to any `.xmp` recipe files on disk.
    /// Returns the new rating value.
    pub fn set_rating(&self, asset_id_prefix: &str, rating: Option<u8>) -> Result<Option<u8>> {
        let catalog = Catalog::open(&self.catalog_root)?;
        let full_id = catalog
            .resolve_asset_id(asset_id_prefix)?
            .ok_or_else(|| anyhow::anyhow!("No asset found matching '{asset_id_prefix}'"))?;

        let uuid: uuid::Uuid = full_id.parse()?;
        let store = MetadataStore::new(&self.catalog_root);
        let mut asset = store.load(uuid)?;

        asset.rating = rating;
        store.save(&asset)?;
        catalog.update_asset_rating(&full_id, rating)?;

        self.write_back_rating_to_xmp(&mut asset, rating, &catalog, &store);

        Ok(rating)
    }

    /// Write back a rating change to `.xmp` recipe files on disk.
    ///
    /// For each XMP recipe on an online volume, updates the `xmp:Rating` value,
    /// re-hashes the file, and updates the recipe's content hash in catalog and sidecar.
    /// Silently skips offline volumes and missing files.
    fn write_back_rating_to_xmp(
        &self,
        asset: &mut Asset,
        rating: Option<u8>,
        catalog: &Catalog,
        store: &MetadataStore,
    ) {
        let registry = DeviceRegistry::new(&self.catalog_root);
        let volumes = match registry.list() {
            Ok(v) => v,
            Err(e) => {
                eprintln!("Warning: could not load volumes for XMP write-back: {e}");
                return;
            }
        };

        let online: HashMap<uuid::Uuid, &std::path::Path> = volumes
            .iter()
            .filter(|v| v.is_online)
            .map(|v| (v.id, v.mount_point.as_path()))
            .collect();

        let content_store = ContentStore::new(&self.catalog_root);
        let mut sidecar_dirty = false;

        for recipe in &mut asset.recipes {
            let ext = recipe
                .location
                .relative_path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase();
            if ext != "xmp" {
                continue;
            }

            let mount_point = match online.get(&recipe.location.volume_id) {
                Some(mp) => *mp,
                None => continue, // volume offline
            };

            let full_path = mount_point.join(&recipe.location.relative_path);
            if !full_path.exists() {
                continue;
            }

            match xmp_reader::update_rating(&full_path, rating) {
                Ok(true) => {
                    // File was modified — re-hash and update catalog
                    match content_store.hash_file(&full_path) {
                        Ok(new_hash) => {
                            if let Err(e) = catalog.update_recipe_content_hash(
                                &recipe.id.to_string(),
                                &new_hash,
                            ) {
                                eprintln!(
                                    "Warning: could not update recipe hash in catalog: {e}"
                                );
                            }
                            recipe.content_hash = new_hash;
                            sidecar_dirty = true;
                        }
                        Err(e) => {
                            eprintln!("Warning: could not re-hash XMP file: {e}");
                        }
                    }
                }
                Ok(false) => {} // no change needed
                Err(e) => {
                    eprintln!(
                        "Warning: could not write rating to {}: {e}",
                        full_path.display()
                    );
                }
            }
        }

        if sidecar_dirty {
            if let Err(e) = store.save(asset) {
                eprintln!("Warning: could not save sidecar after XMP write-back: {e}");
            }
        }
    }

    /// Write back tag add/remove operations to `.xmp` recipe files on disk.
    ///
    /// For each XMP recipe on an online volume, applies the same delta (add/remove)
    /// to the `dc:subject` keyword list, re-hashes, and updates the recipe's content
    /// hash in catalog and sidecar. Silently skips offline volumes and missing files.
    fn write_back_tags_to_xmp(
        &self,
        asset: &mut Asset,
        tags_to_add: &[String],
        tags_to_remove: &[String],
        catalog: &Catalog,
        store: &MetadataStore,
    ) {
        let registry = DeviceRegistry::new(&self.catalog_root);
        let volumes = match registry.list() {
            Ok(v) => v,
            Err(e) => {
                eprintln!("Warning: could not load volumes for XMP tag write-back: {e}");
                return;
            }
        };

        let online: HashMap<uuid::Uuid, &std::path::Path> = volumes
            .iter()
            .filter(|v| v.is_online)
            .map(|v| (v.id, v.mount_point.as_path()))
            .collect();

        let content_store = ContentStore::new(&self.catalog_root);
        let mut sidecar_dirty = false;

        for recipe in &mut asset.recipes {
            let ext = recipe
                .location
                .relative_path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase();
            if ext != "xmp" {
                continue;
            }

            let mount_point = match online.get(&recipe.location.volume_id) {
                Some(mp) => *mp,
                None => continue,
            };

            let full_path = mount_point.join(&recipe.location.relative_path);
            if !full_path.exists() {
                continue;
            }

            let changed_dc = match xmp_reader::update_tags(&full_path, tags_to_add, tags_to_remove)
            {
                Ok(c) => c,
                Err(e) => {
                    eprintln!(
                        "Warning: could not write tags to {}: {e}",
                        full_path.display()
                    );
                    false
                }
            };
            let changed_lr =
                match xmp_reader::update_hierarchical_subjects(&full_path, tags_to_add, tags_to_remove)
                {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!(
                            "Warning: could not write hierarchical subjects to {}: {e}",
                            full_path.display()
                        );
                        false
                    }
                };
            if changed_dc || changed_lr {
                match content_store.hash_file(&full_path) {
                    Ok(new_hash) => {
                        if let Err(e) = catalog.update_recipe_content_hash(
                            &recipe.id.to_string(),
                            &new_hash,
                        ) {
                            eprintln!(
                                "Warning: could not update recipe hash in catalog: {e}"
                            );
                        }
                        recipe.content_hash = new_hash;
                        sidecar_dirty = true;
                    }
                    Err(e) => {
                        eprintln!("Warning: could not re-hash XMP file: {e}");
                    }
                }
            }
        }

        if sidecar_dirty {
            if let Err(e) = store.save(asset) {
                eprintln!("Warning: could not save sidecar after XMP tag write-back: {e}");
            }
        }
    }

    /// Set the color label on an asset. Updates both sidecar YAML and SQLite catalog.
    /// Also writes back the label to any `.xmp` recipe files on disk.
    /// Returns the new label value.
    pub fn set_color_label(&self, asset_id_prefix: &str, label: Option<String>) -> Result<Option<String>> {
        let catalog = Catalog::open(&self.catalog_root)?;
        let full_id = catalog
            .resolve_asset_id(asset_id_prefix)?
            .ok_or_else(|| anyhow::anyhow!("No asset found matching '{asset_id_prefix}'"))?;

        let uuid: uuid::Uuid = full_id.parse()?;
        let store = MetadataStore::new(&self.catalog_root);
        let mut asset = store.load(uuid)?;

        asset.color_label = label.clone();
        store.save(&asset)?;
        catalog.update_asset_color_label(&full_id, label.as_deref())?;

        self.write_back_label_to_xmp(&mut asset, label.as_deref(), &catalog, &store);

        Ok(label)
    }

    /// Set the description on an asset. Updates both sidecar YAML and SQLite catalog.
    /// Also writes back the description to any `.xmp` recipe files on disk.
    /// Returns the new description value.
    pub fn set_description(
        &self,
        asset_id_prefix: &str,
        description: Option<String>,
    ) -> Result<Option<String>> {
        let catalog = Catalog::open(&self.catalog_root)?;
        let full_id = catalog
            .resolve_asset_id(asset_id_prefix)?
            .ok_or_else(|| anyhow::anyhow!("No asset found matching '{asset_id_prefix}'"))?;

        let uuid: uuid::Uuid = full_id.parse()?;
        let store = MetadataStore::new(&self.catalog_root);
        let mut asset = store.load(uuid)?;

        asset.description = description.clone();
        store.save(&asset)?;
        catalog.insert_asset(&asset)?;

        self.write_back_description_to_xmp(&mut asset, description.as_deref(), &catalog, &store);

        Ok(asset.description)
    }

    /// Write back a description change to `.xmp` recipe files on disk.
    ///
    /// For each XMP recipe on an online volume, updates the `dc:description` value,
    /// re-hashes the file, and updates the recipe's content hash in catalog and sidecar.
    /// Silently skips offline volumes and missing files.
    fn write_back_description_to_xmp(
        &self,
        asset: &mut Asset,
        description: Option<&str>,
        catalog: &Catalog,
        store: &MetadataStore,
    ) {
        let registry = DeviceRegistry::new(&self.catalog_root);
        let volumes = match registry.list() {
            Ok(v) => v,
            Err(e) => {
                eprintln!("Warning: could not load volumes for XMP description write-back: {e}");
                return;
            }
        };

        let online: HashMap<uuid::Uuid, &std::path::Path> = volumes
            .iter()
            .filter(|v| v.is_online)
            .map(|v| (v.id, v.mount_point.as_path()))
            .collect();

        let content_store = ContentStore::new(&self.catalog_root);
        let mut sidecar_dirty = false;

        for recipe in &mut asset.recipes {
            let ext = recipe
                .location
                .relative_path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase();
            if ext != "xmp" {
                continue;
            }

            let mount_point = match online.get(&recipe.location.volume_id) {
                Some(mp) => *mp,
                None => continue,
            };

            let full_path = mount_point.join(&recipe.location.relative_path);
            if !full_path.exists() {
                continue;
            }

            match xmp_reader::update_description(&full_path, description) {
                Ok(true) => {
                    match content_store.hash_file(&full_path) {
                        Ok(new_hash) => {
                            if let Err(e) = catalog.update_recipe_content_hash(
                                &recipe.id.to_string(),
                                &new_hash,
                            ) {
                                eprintln!(
                                    "Warning: could not update recipe hash in catalog: {e}"
                                );
                            }
                            recipe.content_hash = new_hash;
                            sidecar_dirty = true;
                        }
                        Err(e) => {
                            eprintln!("Warning: could not re-hash XMP file: {e}");
                        }
                    }
                }
                Ok(false) => {}
                Err(e) => {
                    eprintln!(
                        "Warning: could not write description to {}: {e}",
                        full_path.display()
                    );
                }
            }
        }

        if sidecar_dirty {
            if let Err(e) = store.save(asset) {
                eprintln!("Warning: could not save sidecar after XMP description write-back: {e}");
            }
        }
    }

    /// Write back a color label change to `.xmp` recipe files on disk.
    ///
    /// For each XMP recipe on an online volume, updates the `xmp:Label` value,
    /// re-hashes the file, and updates the recipe's content hash in catalog and sidecar.
    /// Silently skips offline volumes and missing files.
    fn write_back_label_to_xmp(
        &self,
        asset: &mut Asset,
        label: Option<&str>,
        catalog: &Catalog,
        store: &MetadataStore,
    ) {
        let registry = DeviceRegistry::new(&self.catalog_root);
        let volumes = match registry.list() {
            Ok(v) => v,
            Err(e) => {
                eprintln!("Warning: could not load volumes for XMP label write-back: {e}");
                return;
            }
        };

        let online: HashMap<uuid::Uuid, &std::path::Path> = volumes
            .iter()
            .filter(|v| v.is_online)
            .map(|v| (v.id, v.mount_point.as_path()))
            .collect();

        let content_store = ContentStore::new(&self.catalog_root);
        let mut sidecar_dirty = false;

        for recipe in &mut asset.recipes {
            let ext = recipe
                .location
                .relative_path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase();
            if ext != "xmp" {
                continue;
            }

            let mount_point = match online.get(&recipe.location.volume_id) {
                Some(mp) => *mp,
                None => continue,
            };

            let full_path = mount_point.join(&recipe.location.relative_path);
            if !full_path.exists() {
                continue;
            }

            match xmp_reader::update_label(&full_path, label) {
                Ok(true) => {
                    match content_store.hash_file(&full_path) {
                        Ok(new_hash) => {
                            if let Err(e) = catalog.update_recipe_content_hash(
                                &recipe.id.to_string(),
                                &new_hash,
                            ) {
                                eprintln!(
                                    "Warning: could not update recipe hash in catalog: {e}"
                                );
                            }
                            recipe.content_hash = new_hash;
                            sidecar_dirty = true;
                        }
                        Err(e) => {
                            eprintln!("Warning: could not re-hash XMP file: {e}");
                        }
                    }
                }
                Ok(false) => {}
                Err(e) => {
                    eprintln!(
                        "Warning: could not write label to {}: {e}",
                        full_path.display()
                    );
                }
            }
        }

        if sidecar_dirty {
            if let Err(e) = store.save(asset) {
                eprintln!("Warning: could not save sidecar after XMP label write-back: {e}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Catalog;
    use crate::models::{Asset, AssetType};

    /// Set up a temp catalog with one asset and its sidecar, returning (dir, asset_id).
    fn setup_tag_env() -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        let catalog_root = dir.path();

        // Init catalog
        let catalog = Catalog::open(catalog_root).unwrap();
        catalog.initialize().unwrap();

        // Create and persist an asset
        let mut asset = Asset::new(AssetType::Image, "sha256:tag_env");
        asset.tags = vec!["existing".to_string()];
        catalog.insert_asset(&asset).unwrap();

        let store = MetadataStore::new(catalog_root);
        store.save(&asset).unwrap();

        (dir, asset.id.to_string())
    }

    use crate::models::{Variant, VariantRole};

    /// Set up a temp catalog with two assets, each with one variant, for group tests.
    /// Returns (dir, hash1, hash2, asset_id1, asset_id2).
    fn setup_group_env() -> (tempfile::TempDir, String, String, String, String) {
        let dir = tempfile::tempdir().unwrap();
        let catalog_root = dir.path();

        let catalog = Catalog::open(catalog_root).unwrap();
        catalog.initialize().unwrap();
        let store = MetadataStore::new(catalog_root);

        // Create first asset (older)
        let mut asset1 = Asset::new(AssetType::Image, "sha256:hash1");
        asset1.created_at = chrono::Utc::now() - chrono::Duration::hours(2);
        asset1.tags = vec!["landscape".to_string()];
        let variant1 = Variant {
            content_hash: "sha256:hash1".to_string(),
            asset_id: asset1.id,
            role: VariantRole::Original,
            format: "arw".to_string(),
            file_size: 25_000_000,
            original_filename: "DSC_001.ARW".to_string(),
            source_metadata: Default::default(),
            locations: vec![],
        };
        asset1.variants.push(variant1.clone());
        catalog.insert_asset(&asset1).unwrap();
        catalog.insert_variant(&variant1).unwrap();
        store.save(&asset1).unwrap();

        // Create second asset (newer)
        let mut asset2 = Asset::new(AssetType::Image, "sha256:hash2");
        asset2.tags = vec!["nature".to_string()];
        let variant2 = Variant {
            content_hash: "sha256:hash2".to_string(),
            asset_id: asset2.id,
            role: VariantRole::Original,
            format: "jpg".to_string(),
            file_size: 5_000_000,
            original_filename: "DSC_001.JPG".to_string(),
            source_metadata: Default::default(),
            locations: vec![],
        };
        asset2.variants.push(variant2.clone());
        catalog.insert_asset(&asset2).unwrap();
        catalog.insert_variant(&variant2).unwrap();
        store.save(&asset2).unwrap();

        let id1 = asset1.id.to_string();
        let id2 = asset2.id.to_string();
        (dir, "sha256:hash1".to_string(), "sha256:hash2".to_string(), id1, id2)
    }

    #[test]
    fn group_two_variants_from_two_assets() {
        let (dir, hash1, hash2, id1, id2) = setup_group_env();
        let engine = QueryEngine::new(dir.path());

        let result = engine.group(&[hash1, hash2]).unwrap();

        // Target should be the older asset (asset1)
        assert_eq!(result.target_id, id1);
        assert_eq!(result.variants_moved, 1);
        assert_eq!(result.donors_removed, 1);

        // Target should now have both variants
        let details = engine.show(&id1).unwrap();
        assert_eq!(details.variants.len(), 2);

        // Original variant keeps its role, donor variant becomes export
        let original = details.variants.iter().find(|v| v.content_hash == "sha256:hash1").unwrap();
        assert_eq!(original.role, "original");
        let moved = details.variants.iter().find(|v| v.content_hash == "sha256:hash2").unwrap();
        assert_eq!(moved.role, "export");

        // Donor should be gone
        assert!(engine.show(&id2).is_err());
    }

    #[test]
    fn group_already_same_asset_is_noop() {
        let (dir, hash1, _, id1, _) = setup_group_env();
        let engine = QueryEngine::new(dir.path());

        let result = engine.group(&[hash1.clone(), hash1]).unwrap();

        assert_eq!(result.target_id, id1);
        assert_eq!(result.variants_moved, 0);
        assert_eq!(result.donors_removed, 0);
    }

    #[test]
    fn group_nonexistent_hash_errors() {
        let (dir, _, _, _, _) = setup_group_env();
        let engine = QueryEngine::new(dir.path());

        let result = engine.group(&["sha256:bogus".to_string()]);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("No variant found"));
    }

    #[test]
    fn group_merges_tags() {
        let (dir, hash1, hash2, id1, _) = setup_group_env();
        let engine = QueryEngine::new(dir.path());

        engine.group(&[hash1, hash2]).unwrap();

        let details = engine.show(&id1).unwrap();
        assert!(details.tags.contains(&"landscape".to_string()));
        assert!(details.tags.contains(&"nature".to_string()));
    }

    #[test]
    fn tag_add_new() {
        let (dir, id) = setup_tag_env();
        let engine = QueryEngine::new(dir.path());

        let result = engine
            .tag(&id, &["landscape".to_string(), "nature".to_string()], false)
            .unwrap();

        assert_eq!(result.changed, vec!["landscape", "nature"]);
        assert_eq!(result.current_tags, vec!["existing", "landscape", "nature"]);
    }

    #[test]
    fn tag_add_duplicate_is_noop() {
        let (dir, id) = setup_tag_env();
        let engine = QueryEngine::new(dir.path());

        let result = engine.tag(&id, &["existing".to_string()], false).unwrap();

        assert!(result.changed.is_empty());
        assert_eq!(result.current_tags, vec!["existing"]);
    }

    #[test]
    fn tag_remove_existing() {
        let (dir, id) = setup_tag_env();
        let engine = QueryEngine::new(dir.path());

        let result = engine.tag(&id, &["existing".to_string()], true).unwrap();

        assert_eq!(result.changed, vec!["existing"]);
        assert!(result.current_tags.is_empty());
    }

    #[test]
    fn tag_remove_nonexistent_is_noop() {
        let (dir, id) = setup_tag_env();
        let engine = QueryEngine::new(dir.path());

        let result = engine.tag(&id, &["nope".to_string()], true).unwrap();

        assert!(result.changed.is_empty());
        assert_eq!(result.current_tags, vec!["existing"]);
    }

    #[test]
    fn tag_persists_to_sidecar_and_catalog() {
        let (dir, id) = setup_tag_env();
        let engine = QueryEngine::new(dir.path());

        engine.tag(&id, &["new_tag".to_string()], false).unwrap();

        // Verify sidecar
        let uuid: uuid::Uuid = id.parse().unwrap();
        let store = MetadataStore::new(dir.path());
        let asset = store.load(uuid).unwrap();
        assert!(asset.tags.contains(&"new_tag".to_string()));

        // Verify catalog
        let details = engine.show(&id).unwrap();
        assert!(details.tags.contains(&"new_tag".to_string()));
    }

    // ── parse_search_query tests ──────────────────────────────────

    #[test]
    fn parse_camera_filter() {
        let p = parse_search_query("camera:fuji");
        assert_eq!(p.camera.as_deref(), Some("fuji"));
        assert!(p.text.is_none());
    }

    #[test]
    fn parse_lens_filter() {
        let p = parse_search_query("lens:56mm");
        assert_eq!(p.lens.as_deref(), Some("56mm"));
    }

    #[test]
    fn parse_iso_exact() {
        let p = parse_search_query("iso:3200");
        assert_eq!(p.iso_min, Some(3200));
        assert_eq!(p.iso_max, Some(3200));
    }

    #[test]
    fn parse_iso_min() {
        let p = parse_search_query("iso:3200+");
        assert_eq!(p.iso_min, Some(3200));
        assert!(p.iso_max.is_none());
    }

    #[test]
    fn parse_iso_range() {
        let p = parse_search_query("iso:100-800");
        assert_eq!(p.iso_min, Some(100));
        assert_eq!(p.iso_max, Some(800));
    }

    #[test]
    fn parse_focal_exact() {
        let p = parse_search_query("focal:50");
        assert!((p.focal_min.unwrap() - 50.0).abs() < 0.01);
        assert!((p.focal_max.unwrap() - 50.0).abs() < 0.01);
    }

    #[test]
    fn parse_focal_range() {
        let p = parse_search_query("focal:35-70");
        assert!((p.focal_min.unwrap() - 35.0).abs() < 0.01);
        assert!((p.focal_max.unwrap() - 70.0).abs() < 0.01);
    }

    #[test]
    fn parse_f_exact() {
        let p = parse_search_query("f:2.8");
        assert!((p.f_min.unwrap() - 2.8).abs() < 0.01);
        assert!((p.f_max.unwrap() - 2.8).abs() < 0.01);
    }

    #[test]
    fn parse_f_min() {
        let p = parse_search_query("f:2.8+");
        assert!((p.f_min.unwrap() - 2.8).abs() < 0.01);
        assert!(p.f_max.is_none());
    }

    #[test]
    fn parse_f_range() {
        let p = parse_search_query("f:1.4-2.8");
        assert!((p.f_min.unwrap() - 1.4).abs() < 0.01);
        assert!((p.f_max.unwrap() - 2.8).abs() < 0.01);
    }

    #[test]
    fn parse_width_min() {
        let p = parse_search_query("width:4000+");
        assert_eq!(p.width_min, Some(4000));
    }

    #[test]
    fn parse_height_min() {
        let p = parse_search_query("height:2000+");
        assert_eq!(p.height_min, Some(2000));
    }

    #[test]
    fn parse_meta_filter() {
        let p = parse_search_query("meta:label=Red");
        assert_eq!(p.meta_filters.len(), 1);
        assert_eq!(p.meta_filters[0].0, "label");
        assert_eq!(p.meta_filters[0].1, "Red");
    }

    #[test]
    fn parse_mixed_filters_with_text() {
        let p = parse_search_query("camera:fuji sunset iso:400 landscape");
        assert_eq!(p.camera.as_deref(), Some("fuji"));
        assert_eq!(p.iso_min, Some(400));
        assert_eq!(p.iso_max, Some(400));
        assert_eq!(p.text.as_deref(), Some("sunset landscape"));
    }

    #[test]
    fn parse_existing_filters_still_work() {
        let p = parse_search_query("type:image tag:nature format:jpg rating:3+");
        assert_eq!(p.asset_type.as_deref(), Some("image"));
        assert_eq!(p.tag.as_deref(), Some("nature"));
        assert_eq!(p.format.as_deref(), Some("jpg"));
        assert_eq!(p.rating_min, Some(3));
        assert!(p.rating_exact.is_none());
    }

    #[test]
    fn parse_quoted_tag_with_spaces() {
        let p = parse_search_query(r#"tag:"Fools Theater" rating:4+"#);
        assert_eq!(p.tag.as_deref(), Some("Fools Theater"));
        assert_eq!(p.rating_min, Some(4));
        assert!(p.text.is_none());
    }

    #[test]
    fn parse_quoted_camera_and_lens() {
        let p = parse_search_query(r#"camera:"Canon EOS R5" lens:"RF 50mm f/1.2""#);
        assert_eq!(p.camera.as_deref(), Some("Canon EOS R5"));
        assert_eq!(p.lens.as_deref(), Some("RF 50mm f/1.2"));
    }

    #[test]
    fn parse_quoted_label() {
        let p = parse_search_query(r#"label:"light blue" type:image"#);
        assert_eq!(p.color_label.as_deref(), Some("light blue"));
        assert_eq!(p.asset_type.as_deref(), Some("image"));
    }

    #[test]
    fn parse_quoted_collection() {
        let p = parse_search_query(r#"collection:"My Favorites""#);
        assert_eq!(p.collection.as_deref(), Some("My Favorites"));
    }

    #[test]
    fn parse_mixed_quoted_and_unquoted() {
        let p = parse_search_query(r#"sunset tag:"Fools Theater" rating:5"#);
        assert_eq!(p.tag.as_deref(), Some("Fools Theater"));
        assert_eq!(p.rating_exact, Some(5));
        assert_eq!(p.text.as_deref(), Some("sunset"));
    }

    #[test]
    fn tokenize_basic() {
        assert_eq!(tokenize_query("hello world"), vec!["hello", "world"]);
        assert_eq!(tokenize_query(r#"tag:"two words""#), vec!["tag:two words"]);
        assert_eq!(
            tokenize_query(r#"tag:"a b" rating:3+"#),
            vec!["tag:a b", "rating:3+"]
        );
        // Unmatched quote: consumes rest of input
        assert_eq!(tokenize_query(r#"tag:"open"#), vec!["tag:open"]);
        // Empty input
        assert!(tokenize_query("").is_empty());
        assert!(tokenize_query("   ").is_empty());
    }

    #[test]
    fn parse_orphan_filter() {
        let p = parse_search_query("orphan:true");
        assert!(p.orphan);
        assert!(p.text.is_none());
    }

    #[test]
    fn parse_missing_filter() {
        let p = parse_search_query("missing:true");
        assert!(p.missing);
        assert!(p.text.is_none());
    }

    #[test]
    fn parse_stale_filter() {
        let p = parse_search_query("stale:30");
        assert_eq!(p.stale_days, Some(30));
        assert!(p.text.is_none());
    }

    #[test]
    fn parse_stale_filter_zero() {
        let p = parse_search_query("stale:0");
        assert_eq!(p.stale_days, Some(0));
    }

    #[test]
    fn parse_volume_none_filter() {
        let p = parse_search_query("volume:none");
        assert!(p.volume_none);
        assert!(p.text.is_none());
    }

    #[test]
    fn parse_location_health_combined() {
        let p = parse_search_query("orphan:true stale:7 tag:landscape");
        assert!(p.orphan);
        assert_eq!(p.stale_days, Some(7));
        assert_eq!(p.tag.as_deref(), Some("landscape"));
        assert!(!p.missing);
        assert!(!p.volume_none);
    }

    #[test]
    fn parse_label_filter() {
        let p = parse_search_query("label:Red");
        assert_eq!(p.color_label.as_deref(), Some("Red"));
        assert!(p.text.is_none());
    }

    #[test]
    fn parse_label_with_other_filters() {
        let p = parse_search_query("label:Blue tag:landscape sunset");
        assert_eq!(p.color_label.as_deref(), Some("Blue"));
        assert_eq!(p.tag.as_deref(), Some("landscape"));
        assert_eq!(p.text.as_deref(), Some("sunset"));
    }

    #[test]
    fn parse_path_filter() {
        let p = parse_search_query("path:Capture/2026-02-22");
        assert_eq!(p.path_prefix.as_deref(), Some("Capture/2026-02-22"));
        assert!(p.text.is_none());
    }

    #[test]
    fn parse_path_filter_quoted() {
        let p = parse_search_query(r#"path:"Photos/My Trip""#);
        assert_eq!(p.path_prefix.as_deref(), Some("Photos/My Trip"));
    }

    #[test]
    fn parse_path_with_other_filters() {
        let p = parse_search_query("path:Capture/2026 rating:3+ tag:landscape");
        assert_eq!(p.path_prefix.as_deref(), Some("Capture/2026"));
        assert_eq!(p.rating_min, Some(3));
        assert_eq!(p.tag.as_deref(), Some("landscape"));
        assert!(p.text.is_none());
    }

    // ── copies filter parse tests ─────────────────────────────────

    #[test]
    fn parse_copies_exact() {
        let p = parse_search_query("copies:2");
        assert_eq!(p.copies_exact, Some(2));
        assert!(p.copies_min.is_none());
        assert!(p.text.is_none());
    }

    #[test]
    fn parse_copies_min() {
        let p = parse_search_query("copies:2+");
        assert_eq!(p.copies_min, Some(2));
        assert!(p.copies_exact.is_none());
        assert!(p.text.is_none());
    }

    #[test]
    fn parse_copies_with_other_filters() {
        let p = parse_search_query("copies:3+ rating:4+ tag:landscape");
        assert_eq!(p.copies_min, Some(3));
        assert_eq!(p.rating_min, Some(4));
        assert_eq!(p.tag.as_deref(), Some("landscape"));
    }

    // ── date filter parse tests ─────────────────────────────────────

    #[test]
    fn parse_date_prefix_day() {
        let p = parse_search_query("date:2026-02-25");
        assert_eq!(p.date_prefix.as_deref(), Some("2026-02-25"));
        assert!(p.text.is_none());
    }

    #[test]
    fn parse_date_prefix_month() {
        let p = parse_search_query("date:2026-02");
        assert_eq!(p.date_prefix.as_deref(), Some("2026-02"));
    }

    #[test]
    fn parse_date_prefix_year() {
        let p = parse_search_query("date:2026");
        assert_eq!(p.date_prefix.as_deref(), Some("2026"));
    }

    #[test]
    fn parse_date_from() {
        let p = parse_search_query("dateFrom:2026-01-15");
        assert_eq!(p.date_from.as_deref(), Some("2026-01-15"));
        assert!(p.text.is_none());
    }

    #[test]
    fn parse_date_until() {
        let p = parse_search_query("dateUntil:2026-02-28");
        assert_eq!(p.date_until.as_deref(), Some("2026-02-28"));
    }

    #[test]
    fn parse_date_range_combined() {
        let p = parse_search_query("dateFrom:2026-01-01 dateUntil:2026-12-31 tag:landscape");
        assert_eq!(p.date_from.as_deref(), Some("2026-01-01"));
        assert_eq!(p.date_until.as_deref(), Some("2026-12-31"));
        assert_eq!(p.tag.as_deref(), Some("landscape"));
    }

    // ── group recipe preservation tests ──────────────────────────────

    #[test]
    fn group_preserves_recipes() {
        use crate::models::{Recipe, RecipeType};
        use crate::models::volume::FileLocation;

        let (dir, hash1, hash2, id1, id2) = setup_group_env();

        // Add a recipe to the donor (asset2)
        let store = MetadataStore::new(dir.path());
        let uuid2: uuid::Uuid = id2.parse().unwrap();
        let mut asset2 = store.load(uuid2).unwrap();
        asset2.recipes.push(Recipe {
            id: uuid::Uuid::new_v4(),
            variant_hash: "sha256:hash2".to_string(),
            software: "Adobe/CaptureOne".to_string(),
            recipe_type: RecipeType::Sidecar,
            content_hash: "sha256:recipe_hash".to_string(),
            location: FileLocation {
                volume_id: uuid::Uuid::nil(),
                relative_path: "DSC_001.xmp".into(),
                verified_at: None,
            },
        });
        store.save(&asset2).unwrap();

        let engine = QueryEngine::new(dir.path());
        engine.group(&[hash1, hash2]).unwrap();

        // Verify recipe is on the target sidecar
        let uuid1: uuid::Uuid = id1.parse().unwrap();
        let target = store.load(uuid1).unwrap();
        assert_eq!(target.recipes.len(), 1);
        assert_eq!(target.recipes[0].variant_hash, "sha256:hash2");
    }

    // ── auto_group tests ─────────────────────────────────────────────

    #[test]
    fn auto_group_merges_same_stem() {
        let (dir, _, _, id1, id2) = setup_group_env();
        // Both assets have variants with stem DSC_001 (ARW and JPG)
        let engine = QueryEngine::new(dir.path());

        let result = engine
            .auto_group(&[id1.clone(), id2.clone()], false)
            .unwrap();

        assert_eq!(result.groups.len(), 1);
        assert_eq!(result.total_donors_merged, 1);
        assert!(!result.dry_run);

        // RAW asset (id1) should be the target
        assert_eq!(result.groups[0].target_id, id1);

        // Only one asset should remain
        let details = engine.show(&id1).unwrap();
        assert_eq!(details.variants.len(), 2);
        assert!(engine.show(&id2).is_err());
    }

    #[test]
    fn auto_group_dry_run_does_not_modify() {
        let (dir, _, _, id1, id2) = setup_group_env();
        let engine = QueryEngine::new(dir.path());

        let result = engine
            .auto_group(&[id1.clone(), id2.clone()], true)
            .unwrap();

        assert_eq!(result.groups.len(), 1);
        assert_eq!(result.total_donors_merged, 1);
        assert!(result.dry_run);

        // Both assets should still exist
        assert!(engine.show(&id1).is_ok());
        assert!(engine.show(&id2).is_ok());
    }

    #[test]
    fn auto_group_different_stems_no_match() {
        let dir = tempfile::tempdir().unwrap();
        let catalog_root = dir.path();
        let catalog = Catalog::open(catalog_root).unwrap();
        catalog.initialize().unwrap();
        let store = MetadataStore::new(catalog_root);

        let mut asset1 = Asset::new(AssetType::Image, "sha256:aaa");
        let v1 = Variant {
            content_hash: "sha256:aaa".to_string(),
            asset_id: asset1.id,
            role: VariantRole::Original,
            format: "jpg".to_string(),
            file_size: 1000,
            original_filename: "IMG_001.JPG".to_string(),
            source_metadata: Default::default(),
            locations: vec![],
        };
        asset1.variants.push(v1.clone());
        catalog.insert_asset(&asset1).unwrap();
        catalog.insert_variant(&v1).unwrap();
        store.save(&asset1).unwrap();

        let mut asset2 = Asset::new(AssetType::Image, "sha256:bbb");
        let v2 = Variant {
            content_hash: "sha256:bbb".to_string(),
            asset_id: asset2.id,
            role: VariantRole::Original,
            format: "jpg".to_string(),
            file_size: 2000,
            original_filename: "IMG_002.JPG".to_string(),
            source_metadata: Default::default(),
            locations: vec![],
        };
        asset2.variants.push(v2.clone());
        catalog.insert_asset(&asset2).unwrap();
        catalog.insert_variant(&v2).unwrap();
        store.save(&asset2).unwrap();

        let engine = QueryEngine::new(catalog_root);
        let result = engine
            .auto_group(
                &[asset1.id.to_string(), asset2.id.to_string()],
                false,
            )
            .unwrap();

        assert!(result.groups.is_empty());
        assert_eq!(result.total_donors_merged, 0);
    }

    // ── stem_prefix_matches tests ────────────────────────────────────

    #[test]
    fn stem_prefix_exact_match() {
        assert!(stem_prefix_matches("DSC_001", "DSC_001"));
    }

    #[test]
    fn stem_prefix_separator_dash() {
        assert!(stem_prefix_matches("Z91_8561", "Z91_8561-1-HIGHRES"));
    }

    #[test]
    fn stem_prefix_separator_underscore() {
        assert!(stem_prefix_matches("DSC_001", "DSC_001_V2"));
    }

    #[test]
    fn stem_prefix_separator_space() {
        assert!(stem_prefix_matches("DSC_001", "DSC_001 (1)"));
    }

    #[test]
    fn stem_prefix_separator_paren() {
        assert!(stem_prefix_matches("IMG_1234", "IMG_1234(EDIT)"));
    }

    #[test]
    fn stem_prefix_rejects_digit_continuation() {
        // DSC_001 should NOT match DSC_0010 (different shot number)
        assert!(!stem_prefix_matches("DSC_001", "DSC_0010"));
    }

    #[test]
    fn stem_prefix_rejects_letter_continuation() {
        assert!(!stem_prefix_matches("IMG", "IMAGES"));
    }

    #[test]
    fn stem_prefix_no_match() {
        assert!(!stem_prefix_matches("DSC_001", "IMG_001"));
    }

    // ── fuzzy auto_group tests ───────────────────────────────────────

    /// Helper: create a single-variant asset in the catalog/sidecar.
    fn create_asset_with_filename(
        catalog: &Catalog,
        store: &MetadataStore,
        hash: &str,
        filename: &str,
        format: &str,
    ) -> String {
        let mut asset = Asset::new(AssetType::Image, hash);
        let v = Variant {
            content_hash: hash.to_string(),
            asset_id: asset.id,
            role: VariantRole::Original,
            format: format.to_string(),
            file_size: 1000,
            original_filename: filename.to_string(),
            source_metadata: Default::default(),
            locations: vec![],
        };
        asset.variants.push(v.clone());
        catalog.insert_asset(&asset).unwrap();
        catalog.insert_variant(&v).unwrap();
        store.save(&asset).unwrap();
        asset.id.to_string()
    }

    #[test]
    fn auto_group_fuzzy_prefix_match() {
        let dir = tempfile::tempdir().unwrap();
        let catalog_root = dir.path();
        let catalog = Catalog::open(catalog_root).unwrap();
        catalog.initialize().unwrap();
        let store = MetadataStore::new(catalog_root);

        let id_raw = create_asset_with_filename(
            &catalog, &store, "sha256:raw1", "Z91_8561.ARW", "arw",
        );
        let id_export = create_asset_with_filename(
            &catalog, &store, "sha256:exp1",
            "Z91_8561-1-HighRes-(c)_2025_Thomas Herrmann.TIF", "tif",
        );

        let engine = QueryEngine::new(catalog_root);
        let result = engine
            .auto_group(&[id_raw.clone(), id_export.clone()], false)
            .unwrap();

        assert_eq!(result.groups.len(), 1);
        assert_eq!(result.total_donors_merged, 1);
        // RAW asset should be the target
        assert_eq!(result.groups[0].target_id, id_raw);
    }

    #[test]
    fn auto_group_fuzzy_rejects_numeric_continuation() {
        let dir = tempfile::tempdir().unwrap();
        let catalog_root = dir.path();
        let catalog = Catalog::open(catalog_root).unwrap();
        catalog.initialize().unwrap();
        let store = MetadataStore::new(catalog_root);

        let id1 = create_asset_with_filename(
            &catalog, &store, "sha256:f1", "DSC_001.ARW", "arw",
        );
        let id2 = create_asset_with_filename(
            &catalog, &store, "sha256:f2", "DSC_0010.JPG", "jpg",
        );

        let engine = QueryEngine::new(catalog_root);
        let result = engine
            .auto_group(&[id1, id2], false)
            .unwrap();

        // Should NOT match — these are different shots
        assert!(result.groups.is_empty());
    }

    #[test]
    fn auto_group_fuzzy_chain_resolves_to_shortest_root() {
        let dir = tempfile::tempdir().unwrap();
        let catalog_root = dir.path();
        let catalog = Catalog::open(catalog_root).unwrap();
        catalog.initialize().unwrap();
        let store = MetadataStore::new(catalog_root);

        let id_raw = create_asset_with_filename(
            &catalog, &store, "sha256:c1", "Z91_8561.ARW", "arw",
        );
        let id_v1 = create_asset_with_filename(
            &catalog, &store, "sha256:c2", "Z91_8561-1.JPG", "jpg",
        );
        let id_v2 = create_asset_with_filename(
            &catalog, &store, "sha256:c3",
            "Z91_8561-1-HighRes.TIF", "tif",
        );

        let engine = QueryEngine::new(catalog_root);
        let result = engine
            .auto_group(&[id_raw.clone(), id_v1.clone(), id_v2.clone()], false)
            .unwrap();

        // All three should be in one group
        assert_eq!(result.groups.len(), 1);
        assert_eq!(result.groups[0].asset_ids.len(), 3);
        assert_eq!(result.total_donors_merged, 2);
        // RAW asset should be the target
        assert_eq!(result.groups[0].target_id, id_raw);
    }

    // ── normalize_path_for_search tests ────────────────────────────

    use crate::models::volume::{Volume, VolumeType};

    fn make_volume(label: &str, mount: &str) -> Volume {
        Volume {
            id: uuid::Uuid::new_v4(),
            label: label.to_string(),
            mount_point: std::path::PathBuf::from(mount),
            volume_type: VolumeType::External,
            purpose: None,
            is_online: true,
        }
    }

    #[test]
    fn normalize_absolute_path_matching_volume() {
        let vol = make_volume("Photos", "/Volumes/Photos");
        let (rel, vid) = normalize_path_for_search(
            "/Volumes/Photos/Capture/2026", &[vol.clone()], None,
        );
        assert_eq!(rel, "Capture/2026");
        assert_eq!(vid, Some(vol.id.to_string()));
    }

    #[test]
    fn normalize_absolute_path_no_match() {
        let vol = make_volume("Photos", "/Volumes/Photos");
        let (rel, vid) = normalize_path_for_search("/mnt/other/data", &[vol], None);
        assert_eq!(rel, "/mnt/other/data");
        assert!(vid.is_none());
    }

    #[test]
    fn normalize_relative_path_unchanged() {
        let vol = make_volume("Photos", "/Volumes/Photos");
        let (rel, vid) = normalize_path_for_search("Capture/2026", &[vol], None);
        assert_eq!(rel, "Capture/2026");
        assert!(vid.is_none());
    }

    #[test]
    fn normalize_picks_longest_mount_point() {
        let vol_parent = make_volume("Root", "/Volumes");
        let vol_child = make_volume("Photos", "/Volumes/Photos");
        let volumes = vec![vol_parent, vol_child.clone()];
        let (rel, vid) = normalize_path_for_search(
            "/Volumes/Photos/Capture/2026", &volumes, None,
        );
        assert_eq!(rel, "Capture/2026");
        assert_eq!(vid, Some(vol_child.id.to_string()));
    }

    #[test]
    fn normalize_tilde_expands_to_home() {
        let home = std::env::var("HOME").unwrap();
        let vol = make_volume("Home", &home);
        let cwd = std::path::Path::new("/tmp");

        let (rel, vid) = normalize_path_for_search(
            "~/Photos/2026", &[vol.clone()], Some(cwd),
        );
        assert_eq!(rel, "Photos/2026");
        assert_eq!(vid, Some(vol.id.to_string()));
    }

    #[test]
    fn normalize_tilde_alone() {
        let home = std::env::var("HOME").unwrap();
        let vol = make_volume("Home", &home);
        let cwd = std::path::Path::new("/tmp");

        let (rel, vid) = normalize_path_for_search("~", &[vol.clone()], Some(cwd));
        assert_eq!(rel, "");
        assert_eq!(vid, Some(vol.id.to_string()));
    }

    #[test]
    fn normalize_tilde_without_cwd_unchanged() {
        let vol = make_volume("Photos", "/Volumes/Photos");
        let (rel, vid) = normalize_path_for_search("~/Photos", &[vol], None);
        assert_eq!(rel, "~/Photos");
        assert!(vid.is_none());
    }

    #[test]
    fn normalize_dot_slash_resolves_relative_to_cwd() {
        let vol = make_volume("Photos", "/Volumes/Photos");
        let cwd = std::path::Path::new("/Volumes/Photos/Capture");

        let (rel, vid) = normalize_path_for_search(
            "./2026-02-22", &[vol.clone()], Some(cwd),
        );
        assert_eq!(rel, "Capture/2026-02-22");
        assert_eq!(vid, Some(vol.id.to_string()));
    }

    #[test]
    fn normalize_dotdot_resolves_relative_to_cwd() {
        let vol = make_volume("Photos", "/Volumes/Photos");
        let cwd = std::path::Path::new("/Volumes/Photos/Capture/2026");

        let (rel, vid) = normalize_path_for_search(
            "../2025", &[vol.clone()], Some(cwd),
        );
        assert_eq!(rel, "Capture/2025");
        assert_eq!(vid, Some(vol.id.to_string()));
    }

    #[test]
    fn normalize_plain_relative_unchanged_even_with_cwd() {
        let vol = make_volume("Photos", "/Volumes/Photos");
        let cwd = std::path::Path::new("/Volumes/Photos/Capture");

        let (rel, vid) = normalize_path_for_search(
            "Capture/2026", &[vol], Some(cwd),
        );
        // Plain relative paths stay as volume-relative prefix matches
        assert_eq!(rel, "Capture/2026");
        assert!(vid.is_none());
    }
}
