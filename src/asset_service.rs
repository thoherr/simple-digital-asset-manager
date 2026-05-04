// ═══════════════════════════════════════════════════════════════════════════════
// asset_service.rs — Core asset lifecycle operations
// ═══════════════════════════════════════════════════════════════════════════════
//
// Table of Contents:
//   1. IMPORTS & CONSTANTS .............. File type groups, use declarations
//   2. FILE TYPE FILTER ................. FileTypeFilter struct & impl
//   3. RESULT TYPES ..................... FileStatus, ImportResult, RelocateResult, etc.
//   4. ASSET SERVICE STRUCT ............. AssetService + scan_orphaned_sharded_files
//   5. IMPORT ........................... import, import_with_callback
//   6. RELOCATE & UPDATE LOCATION ....... relocate, update_location
//   7. VERIFY ........................... verify, verify_location
//   8. SYNC ............................. sync (reconcile catalog with disk)
//   9. CLEANUP & DELETE ................. cleanup, delete_assets
//  10. VOLUME OPERATIONS ................ remove_volume, combine_volume, split_volume
//  11. DEDUP ............................ dedup (same-volume duplicate removal)
//  12. REFRESH & SYNC METADATA .......... refresh, sync_metadata
//  13. FIX COMMANDS ..................... fix_roles, fix_dates, fix_recipes
//  14. EXPORT ........................... build_export_plan, export, export_zip
//  15. AI & FACES ....................... auto_tag, detect_faces, describe (feature-gated)
//  16. VIDEO METADATA ................... backfill_video_metadata
//  17. FREE FUNCTIONS ................... compute_prefixes, apply_xmp_data, helpers
//  18. TESTS ............................ Unit and integration tests
// ═══════════════════════════════════════════════════════════════════════════════

// ═══ IMPORTS & CONSTANTS ═══

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use uuid::Uuid;

use crate::catalog::Catalog;
use crate::content_store::ContentStore;
use crate::device_registry::DeviceRegistry;
use crate::metadata_store::MetadataStore;
use crate::models::{
    Asset, AssetType, FileLocation, Recipe, RecipeType, Variant, VariantRole, Volume,
};

// ═══ FILE TYPE FILTER ═══

/// File type group definitions: (name, extensions, default_on).
const GROUPS: &[(&str, &[&str], bool)] = &[
    (
        "images",
        &[
            "jpg", "jpeg", "png", "gif", "bmp", "tiff", "tif", "webp", "heic", "heif", "svg",
            "ico", "psd", "xcf", // standard image formats
            "raw", "cr2", "cr3", "crw", "nef", "nrw", "arw", "sr2", "srf", "orf", "rw2",
            "dng", "raf", "pef", "srw", "mrw", "3fr", "fff", "iiq", "erf", "kdc", "dcr",
            "mef", "mos", "rwl", "bay", "x3f", // RAW
        ],
        true,
    ),
    (
        "video",
        &[
            "mp4", "mov", "avi", "mkv", "wmv", "flv", "webm", "m4v", "mpg", "mpeg", "3gp",
            "mts", "m2ts",
        ],
        true,
    ),
    (
        "audio",
        &[
            "mp3", "wav", "flac", "aac", "ogg", "wma", "m4a", "aiff", "alac",
        ],
        true,
    ),
    ("xmp", &["xmp"], true),
    (
        "documents",
        &[
            "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "txt", "md", "rtf", "csv",
            "json", "xml", "html", "htm",
        ],
        false,
    ),
    ("captureone", &["cos", "cot", "cop"], false),
    ("rawtherapee", &["pp3"], false),
    ("dxo", &["dop"], false),
    ("on1", &["on1"], false),
];

/// Recipe group names — extensions in these groups are treated as recipe sidecars.
const RECIPE_GROUPS: &[&str] = &["xmp", "captureone", "rawtherapee", "dxo", "on1"];

/// Controls which file types are imported based on enabled/disabled groups.
pub struct FileTypeFilter {
    enabled_groups: HashSet<String>,
}

impl FileTypeFilter {
    /// Create a filter with only the default groups enabled.
    pub fn new() -> Self {
        let enabled = GROUPS
            .iter()
            .filter(|(_, _, default_on)| *default_on)
            .map(|(name, _, _)| name.to_string())
            .collect();
        Self {
            enabled_groups: enabled,
        }
    }

    /// Enable an additional group. Returns an error for unknown group names.
    pub fn include(&mut self, group: &str) -> Result<()> {
        if !GROUPS.iter().any(|(name, _, _)| *name == group) {
            bail!(
                "Unknown file type group '{}'. Valid groups: {}",
                group,
                Self::group_names()
                    .iter()
                    .map(|(n, _)| *n)
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        self.enabled_groups.insert(group.to_string());
        Ok(())
    }

    /// Disable a group. Returns an error for unknown group names.
    pub fn skip(&mut self, group: &str) -> Result<()> {
        if !GROUPS.iter().any(|(name, _, _)| *name == group) {
            bail!(
                "Unknown file type group '{}'. Valid groups: {}",
                group,
                Self::group_names()
                    .iter()
                    .map(|(n, _)| *n)
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        self.enabled_groups.remove(group);
        Ok(())
    }

    /// Returns true if the extension belongs to any enabled group.
    pub fn is_importable(&self, ext: &str) -> bool {
        let lower = ext.to_lowercase();
        for (name, extensions, _) in GROUPS {
            if self.enabled_groups.contains(*name) && extensions.contains(&lower.as_str()) {
                return true;
            }
        }
        false
    }

    /// Returns true if the extension is a recipe type AND its group is enabled.
    pub fn is_recipe(&self, ext: &str) -> bool {
        let lower = ext.to_lowercase();
        for (name, extensions, _) in GROUPS {
            if RECIPE_GROUPS.contains(name)
                && extensions.contains(&lower.as_str())
                && self.enabled_groups.contains(*name)
            {
                return true;
            }
        }
        false
    }

    /// List all group names with their default-on status (for help/error messages).
    pub fn group_names() -> Vec<(&'static str, bool)> {
        GROUPS.iter().map(|(name, _, default)| (*name, *default)).collect()
    }
}

impl Default for FileTypeFilter {
    fn default() -> Self {
        Self::new()
    }
}

/// Status of a single file during import.
// ═══ RESULT TYPES ═══

pub enum FileStatus {
    Imported,
    LocationAdded,
    Skipped,
    RecipeAttached,
    RecipeLocationAdded,
    RecipeUpdated,
}

/// Result of an import operation.
#[derive(serde::Serialize)]
pub struct ImportResult {
    pub dry_run: bool,
    pub imported: usize,
    pub locations_added: usize,
    pub skipped: usize,
    pub recipes_attached: usize,
    pub recipes_location_added: usize,
    pub recipes_updated: usize,
    pub previews_generated: usize,
    pub smart_previews_generated: usize,
    /// Asset IDs created during this import (for post-import auto-grouping).
    #[serde(skip)]
    pub new_asset_ids: Vec<String>,
    /// Volume-relative directory paths of imported files (for neighborhood scoping).
    #[serde(skip)]
    pub imported_directories: Vec<String>,
}

/// Result of a relocate operation.
#[derive(Debug, serde::Serialize)]
pub struct RelocateResult {
    pub copied: usize,
    pub skipped: usize,
    pub removed: usize,
    pub actions: Vec<String>,
}

/// What kind of file is being relocated.
enum FileCopyKind {
    Variant,
    Recipe { recipe_id: Uuid },
}

/// A planned file copy for relocation.
struct FileCopyPlan {
    content_hash: String,
    source_path: PathBuf,
    target_path: PathBuf,
    kind: FileCopyKind,
    /// The volume_id + relative_path of the source location (for removal)
    source_volume_id: Uuid,
    source_relative_path: PathBuf,
}

/// A group of files sharing the same stem in the same directory.
struct StemGroup {
    _dir: PathBuf,
    stem: String,
    media_files: Vec<PathBuf>,
    recipe_files: Vec<PathBuf>,
}

/// Status of a single file during verification.
pub enum VerifyStatus {
    Ok,
    Mismatch,
    Modified,
    Missing,
    Skipped,
    SkippedRecent,
    Untracked,
}

/// Result of a verify operation.
#[derive(serde::Serialize)]
pub struct VerifyResult {
    pub verified: usize,
    pub failed: usize,
    pub modified: usize,
    pub skipped: usize,
    pub skipped_recent: usize,
    pub errors: Vec<String>,
}

/// Check if a verified_at timestamp is within `max_age_days` of now.
fn is_recently_verified(verified_at: Option<&str>, max_age_days: u64) -> bool {
    if let Some(ts) = verified_at {
        if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(ts) {
            let age = chrono::Utc::now() - dt.with_timezone(&chrono::Utc);
            return age.num_days() < max_age_days as i64;
        }
    }
    false
}

/// Status of a single file during sync.
pub enum SyncStatus {
    Unchanged,
    Moved,
    New,
    Modified,
    Missing,
}

/// Result of a sync operation.
#[derive(Debug, serde::Serialize)]
pub struct SyncResult {
    pub unchanged: usize,
    pub moved: usize,
    pub new_files: usize,
    pub modified: usize,
    pub missing: usize,
    pub stale_removed: usize,
    pub orphaned_cleaned: usize,
    /// Number of variants that ended up with zero file_locations after sync,
    /// but whose asset still has other variants (so the asset itself wasn't
    /// orphaned and removed). These are the "selected for preview but offline"
    /// variants that confuse subsequent preview/regenerate calls — `maki
    /// cleanup --apply` removes them. The CLI uses this to surface a
    /// next-step hint after `sync --apply --remove-stale`.
    #[serde(default)]
    pub locationless_after: usize,
    pub errors: Vec<String>,
}

/// Status of a single file during cleanup.
pub enum CleanupStatus {
    Ok,
    Stale,
    Offline,
    LocationlessVariant,
    OrphanedAsset,
    OrphanedFile,
}

/// Result of an update-location operation.
#[derive(Debug, serde::Serialize)]
pub struct UpdateLocationResult {
    pub asset_id: String,
    pub file_type: String,
    pub content_hash: String,
    pub old_path: String,
    pub new_path: String,
    pub volume_label: String,
}

/// Result of a cleanup operation.
#[derive(Debug, serde::Serialize)]
pub struct CleanupResult {
    pub checked: usize,
    pub stale: usize,
    pub removed: usize,
    pub skipped_offline: usize,
    pub locationless_variants: usize,
    pub removed_variants: usize,
    pub orphaned_assets: usize,
    pub removed_assets: usize,
    pub orphaned_previews: usize,
    pub removed_previews: usize,
    pub orphaned_smart_previews: usize,
    pub removed_smart_previews: usize,
    pub orphaned_embeddings: usize,
    pub removed_embeddings: usize,
    pub orphaned_face_files: usize,
    pub removed_face_files: usize,
    pub errors: Vec<String>,
    /// `true` if the caller passed `--volume` or `--path`, causing the global
    /// orphan passes (previews, smart previews, embeddings, face files) to be
    /// skipped. The CLI uses this to print a note suggesting a scope-free run
    /// to catch those.
    #[serde(default)]
    pub skipped_global_passes: bool,
}

/// Result of a volume remove operation.
#[derive(Debug, serde::Serialize)]
pub struct VolumeRemoveResult {
    pub volume_label: String,
    pub volume_id: String,
    pub locations: usize,
    pub locations_removed: usize,
    pub recipes: usize,
    pub recipes_removed: usize,
    pub orphaned_assets: usize,
    pub removed_assets: usize,
    pub orphaned_previews: usize,
    pub removed_previews: usize,
    pub apply: bool,
    pub errors: Vec<String>,
}

/// Result of a volume combine operation.
#[derive(Debug, serde::Serialize)]
pub struct VolumeCombineResult {
    pub source_label: String,
    pub source_id: String,
    pub target_label: String,
    pub target_id: String,
    pub path_prefix: String,
    pub locations: usize,
    pub locations_moved: usize,
    pub recipes: usize,
    pub recipes_moved: usize,
    pub assets_affected: usize,
    pub apply: bool,
    pub errors: Vec<String>,
}

/// Result of a volume split operation.
#[derive(Debug, serde::Serialize)]
pub struct VolumeSplitResult {
    pub source_label: String,
    pub source_id: String,
    pub new_label: String,
    pub new_id: String,
    pub path_prefix: String,
    pub locations: usize,
    pub locations_moved: usize,
    pub recipes: usize,
    pub recipes_moved: usize,
    pub assets_affected: usize,
    pub apply: bool,
    pub errors: Vec<String>,
}

/// Result of generating SigLIP embeddings for a batch of assets.
#[cfg(feature = "ai")]
#[derive(Debug, serde::Serialize)]
pub struct EmbedAssetsResult {
    pub embedded: u32,
    pub skipped: u32,
    pub errors: Vec<String>,
}

/// Per-asset status during embedding.
#[cfg(feature = "ai")]
pub enum EmbedStatus {
    Embedded,
    Skipped(&'static str),
    Error(String),
}

/// Result of face detection on a batch of assets.
#[cfg(feature = "ai")]
#[derive(Debug, serde::Serialize)]
pub struct DetectFacesResult {
    pub assets_processed: u32,
    pub assets_skipped: u32,
    pub faces_detected: u32,
    pub errors: Vec<String>,
}

/// Status of a single recipe during refresh.
pub enum RefreshStatus {
    Unchanged,
    Refreshed,
    Missing,
    Offline,
}

/// Result of a refresh operation.
#[derive(Debug, serde::Serialize)]
pub struct RefreshResult {
    pub unchanged: usize,
    pub refreshed: usize,
    pub missing: usize,
    pub skipped: usize,
    pub errors: Vec<String>,
}

/// Result of a sync-metadata operation.
#[derive(Debug, Default, serde::Serialize)]
pub struct SyncMetadataResult {
    /// Recipes where external changes were read in (inbound).
    pub inbound: usize,
    /// Recipes where pending DAM changes were written out (outbound).
    pub outbound: usize,
    /// Recipes unchanged on disk with no pending changes.
    pub unchanged: usize,
    /// Recipes skipped (offline volume or missing file).
    pub skipped: usize,
    /// Recipes with both external changes and pending DAM edits (conflict).
    pub conflicts: usize,
    /// Media files with re-extracted embedded XMP (if --media).
    pub media_refreshed: usize,
    /// Whether this was a dry run.
    pub dry_run: bool,
    /// Error messages.
    pub errors: Vec<String>,
}

/// Status of a single recipe during sync-metadata.
pub enum SyncMetadataStatus {
    Inbound,
    Outbound,
    Unchanged,
    Missing,
    Offline,
    Conflict,
    Error,
}

/// Status of a single asset during fix-roles.
pub enum FixRolesStatus {
    AlreadyCorrect,
    Fixed,
}

/// Result of a fix-roles operation.
#[derive(Debug, serde::Serialize)]
pub struct FixRolesResult {
    pub checked: usize,
    pub fixed: usize,
    pub variants_fixed: usize,
    pub already_correct: usize,
    pub dry_run: bool,
    pub errors: Vec<String>,
}

/// Result of a delete operation.
#[derive(Debug, serde::Serialize)]
pub struct DeleteResult {
    pub deleted: usize,
    pub not_found: Vec<String>,
    pub files_removed: usize,
    pub previews_removed: usize,
    pub dry_run: bool,
    pub errors: Vec<String>,
}

/// Status of a single asset during delete.
pub enum DeleteStatus {
    Deleted,
    NotFound,
    Error(String),
}

/// Status of a single asset during fix-dates.
pub enum FixDatesStatus {
    AlreadyCorrect,
    Fixed,
    NoDate,
    SkippedOffline,
}

/// Result of a fix-dates operation.
#[derive(Debug, Default, serde::Serialize)]
pub struct FixDatesResult {
    pub checked: usize,
    pub fixed: usize,
    pub already_correct: usize,
    pub no_date: usize,
    pub skipped_offline: usize,
    pub dry_run: bool,
    pub offline_volumes: Vec<String>,
    pub errors: Vec<String>,
}

/// Status of a single asset during fix-recipes.
pub enum FixRecipesStatus {
    Reattached,
    NoParentFound,
    Skipped,
}

/// Result of a fix-recipes operation.
#[derive(Debug, Default, serde::Serialize)]
pub struct FixRecipesResult {
    pub checked: usize,
    pub reattached: usize,
    pub no_parent: usize,
    pub skipped: usize,
    pub dry_run: bool,
    pub errors: Vec<String>,
}

/// Status of a single location during dedup.
pub enum DedupStatus {
    Keep,
    Remove,
    Skipped,
}

/// Result of a dedup operation.
#[derive(Debug, serde::Serialize)]
pub struct DedupResult {
    pub duplicates_found: usize,
    pub locations_to_remove: usize,
    pub locations_removed: usize,
    pub files_deleted: usize,
    pub recipes_removed: usize,
    pub bytes_freed: u64,
    pub dry_run: bool,
    pub errors: Vec<String>,
}

/// Layout strategy for exported files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportLayout {
    /// All files in the target root; collisions resolved by appending hash suffix.
    Flat,
    /// Preserves source volume-relative directory structure; multi-volume gets volume-label prefix.
    Mirror,
}

/// Status of a single file during export.
pub enum ExportStatus {
    Copied,
    Linked,
    Skipped,
    Error(String),
}

/// Result of an export operation.
#[derive(Debug, serde::Serialize)]
pub struct ExportResult {
    pub dry_run: bool,
    pub assets_matched: usize,
    pub files_exported: usize,
    pub files_skipped: usize,
    pub sidecars_exported: usize,
    pub total_bytes: u64,
    pub errors: Vec<String>,
}

/// Internal plan entry for a single file to export.
pub struct ExportFilePlan {
    pub asset_id: String,
    pub content_hash: String,
    pub source_path: PathBuf,
    pub target_path: PathBuf,
    pub file_size: u64,
    pub is_sidecar: bool,
}

/// Resolve a flat-mode target path, handling filename collisions.
///
/// `seen` tracks lowercase filename → content_hash. If the same filename maps to a
/// different hash, a `_<hash[..8]>` suffix is inserted before the extension.
fn resolve_flat_target(
    target_dir: &Path,
    filename: &str,
    content_hash: &str,
    seen: &mut std::collections::HashMap<String, String>,
) -> PathBuf {
    let key = filename.to_lowercase();
    match seen.get(&key) {
        Some(existing_hash) if existing_hash == content_hash => {
            // Same content, reuse the same target path
            target_dir.join(filename)
        }
        Some(_) => {
            // Different content with same filename — add hash suffix
            // content_hash is "sha256:<hex>", extract last 8 hex chars
            let hex_part = content_hash
                .strip_prefix("sha256:")
                .unwrap_or(content_hash);
            let hash_suffix = &hex_part[hex_part.len().saturating_sub(8)..];
            let path = Path::new(filename);
            let stem = path.file_stem().unwrap_or_default().to_string_lossy();
            let new_name = if let Some(ext) = path.extension() {
                format!("{}_{}.{}", stem, hash_suffix, ext.to_string_lossy())
            } else {
                format!("{}_{}", stem, hash_suffix)
            };
            seen.insert(new_name.to_lowercase(), content_hash.to_string());
            target_dir.join(new_name)
        }
        None => {
            seen.insert(key, content_hash.to_string());
            target_dir.join(filename)
        }
    }
}

/// Create a symlink (platform-gated).
#[cfg(unix)]
fn create_symlink(source: &Path, target: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(source, target)
}

#[cfg(windows)]
fn create_symlink(source: &Path, target: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_file(source, target)
}

/// Get file modification time as DateTime<Utc>. Returns None on any error.
fn file_mtime(path: &Path) -> Option<chrono::DateTime<chrono::Utc>> {
    let metadata = std::fs::metadata(path).ok()?;
    let modified = metadata.modified().ok()?;
    Some(chrono::DateTime::<chrono::Utc>::from(modified))
}

/// High-level operations that orchestrate the other components.
// ═══ ASSET SERVICE STRUCT ═══

pub struct AssetService {
    catalog_root: PathBuf,
    verbosity: crate::Verbosity,
    preview_config: crate::config::PreviewConfig,
}

/// Walk a sharded directory (`dir/<prefix>/<id>.<ext>`) and find files whose stem
/// is not in the valid set. Counts orphaned/removed and optionally deletes.
fn scan_orphaned_sharded_files(
    dir: &Path,
    is_valid: impl Fn(&str) -> bool,
    apply: bool,
    orphaned: &mut usize,
    removed: &mut usize,
    errors: &mut Vec<String>,
    on_file: &impl Fn(&Path, CleanupStatus, Duration),
) {
    if !dir.is_dir() {
        return;
    }
    let Ok(shard_entries) = std::fs::read_dir(dir) else {
        return;
    };
    for shard_entry in shard_entries.flatten() {
        if !shard_entry.path().is_dir() {
            continue;
        }
        let Ok(file_entries) = std::fs::read_dir(shard_entry.path()) else {
            continue;
        };
        for file_entry in file_entries.flatten() {
            let path = file_entry.path();
            if !path.is_file() {
                continue;
            }
            let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
            if stem.is_empty() {
                continue;
            }
            if !is_valid(stem) {
                *orphaned += 1;
                if apply {
                    let file_start = Instant::now();
                    if let Err(e) = std::fs::remove_file(&path) {
                        errors.push(format!(
                            "Failed to remove orphaned file {}: {e}",
                            path.display()
                        ));
                    } else {
                        *removed += 1;
                        on_file(&path, CleanupStatus::OrphanedFile, file_start.elapsed());
                    }
                }
            }
        }
    }
}

impl AssetService {
    pub fn new(catalog_root: &Path, verbosity: crate::Verbosity, preview_config: &crate::config::PreviewConfig) -> Self {
        Self {
            catalog_root: catalog_root.to_path_buf(),
            verbosity,
            preview_config: preview_config.clone(),
        }
    }

}

mod import;
mod relocate;
mod verify;
mod sync;
mod cleanup;
mod volume;
mod dedup;
mod refresh;
mod fix;
mod export;
mod ai;
mod video;


// ═══ FREE FUNCTIONS ═══

/// Compute directory prefixes from scanned paths relative to the volume mount point.
fn compute_prefixes(paths: &[PathBuf], mount_point: &Path) -> Vec<String> {
    let mut prefixes = Vec::new();
    for path in paths {
        let canonical = path.canonicalize().unwrap_or_else(|_| path.clone());
        if let Ok(rel) = canonical.strip_prefix(mount_point) {
            if canonical.is_dir() {
                let prefix = rel.to_string_lossy().to_string();
                if prefix.is_empty() {
                    prefixes.push(String::new());
                } else {
                    prefixes.push(format!("{prefix}/"));
                }
            } else {
                // Single file — use its parent directory as prefix
                if let Some(parent) = rel.parent() {
                    let prefix = parent.to_string_lossy().to_string();
                    if prefix.is_empty() {
                        prefixes.push(String::new());
                    } else {
                        prefixes.push(format!("{prefix}/"));
                    }
                }
            }
        } else {
            // Path not under mount point — use empty prefix (scan everything)
            prefixes.push(String::new());
        }
    }
    if prefixes.is_empty() {
        prefixes.push(String::new());
    }
    prefixes.sort();
    prefixes.dedup();
    prefixes
}

/// Merge XMP metadata into an asset and its primary variant.
/// - Keywords merge into `asset.tags` (deduplicated)
/// - Description sets `asset.description` if not already set
/// - source_metadata merges into the variant (EXIF takes precedence via `or_insert`)
/// Merge flat `dc:subject` keywords with hierarchical `lr:hierarchicalSubject` keywords.
///
/// Hierarchical tags use `|` as separator internally. Flat keywords that are components
/// of any hierarchical tag are suppressed (e.g., `birds` is suppressed when
/// `animals|birds|eagles` exists). Non-component flat keywords are kept as-is.
fn merge_hierarchical_keywords(
    flat_keywords: &[String],
    hierarchical_keywords: &[String],
) -> Vec<String> {
    use std::collections::HashSet;

    if hierarchical_keywords.is_empty() {
        return flat_keywords.to_vec();
    }

    // Collect all individual components from hierarchical tags (split on `|`)
    let components: HashSet<&str> = hierarchical_keywords
        .iter()
        .flat_map(|h| h.split('|'))
        .collect();

    let mut result: Vec<String> = Vec::new();
    let mut seen: HashSet<&str> = HashSet::new();

    // Add hierarchical tags first
    for h in hierarchical_keywords {
        if seen.insert(h.as_str()) {
            result.push(h.clone());
        }
    }

    // Add flat keywords that are NOT components of any hierarchical tag
    for kw in flat_keywords {
        if !components.contains(kw.as_str()) && !seen.contains(kw.as_str()) {
            seen.insert(kw.as_str());
            result.push(kw.clone());
        }
    }

    result
}

/// Normalize a rating value: convert MicrosoftPhoto:Rating percentage scale (1-100)
/// to xmp:Rating 1-5 scale. Values already in 1-5 range pass through unchanged.
pub fn normalize_rating(r: u8) -> u8 {
    if r <= 5 {
        r
    } else {
        // MicrosoftPhoto:Rating percentages: 1→1, 25→2, 50→3, 75→4, 99/100→5
        match r {
            1..=12 => 1,
            13..=37 => 2,
            38..=62 => 3,
            63..=87 => 4,
            _ => 5,
        }
    }
}

/// Public wrapper for `apply_xmp_data` — used by `QueryEngine::reimport_metadata`.
pub fn apply_xmp_data_pub(xmp: &crate::xmp_reader::XmpData, asset: &mut Asset, variant_hash: &str) {
    apply_xmp_data(xmp, asset, variant_hash);
}

fn apply_xmp_data(xmp: &crate::xmp_reader::XmpData, asset: &mut Asset, variant_hash: &str) {
    let merged = merge_hierarchical_keywords(&xmp.keywords, &xmp.hierarchical_keywords);
    for kw in &merged {
        if !asset.tags.contains(kw) {
            asset.tags.push(kw.clone());
        }
    }

    if asset.description.is_none() {
        asset.description.clone_from(&xmp.description);
    }

    // Promote rating to asset level (conservative: only if not already set)
    if asset.rating.is_none() {
        if let Some(rating_str) = xmp.source_metadata.get("rating") {
            if let Ok(r) = rating_str.parse::<u8>() {
                asset.rating = Some(normalize_rating(r));
            }
        }
    }

    // Promote color label to asset level (conservative: only if not already set)
    if asset.color_label.is_none() {
        if let Some(label_str) = xmp.source_metadata.get("label") {
            if let Ok(Some(canonical)) = Asset::validate_color_label(label_str) {
                asset.color_label = Some(canonical);
            }
        }
    }

    if let Some(variant) = asset.variants.iter_mut().find(|v| v.content_hash == variant_hash) {
        for (key, val) in &xmp.source_metadata {
            variant
                .source_metadata
                .entry(key.clone())
                .or_insert_with(|| val.clone());
        }
    }
}

/// Re-apply XMP metadata when a recipe file has been modified.
/// Unlike `apply_xmp_data` (initial import, conservative merge):
/// - Keywords: merge (add new; cannot remove since we don't track provenance)
/// - Description: overwrite (user explicitly edited the XMP)
/// - source_metadata: overwrite XMP-sourced keys (rating, label, creator, copyright)
fn reapply_xmp_data(xmp: &crate::xmp_reader::XmpData, asset: &mut Asset, variant_hash: &str) {
    let merged = merge_hierarchical_keywords(&xmp.keywords, &xmp.hierarchical_keywords);
    for kw in &merged {
        if !asset.tags.contains(kw) {
            asset.tags.push(kw.clone());
        }
    }

    if xmp.description.is_some() {
        asset.description.clone_from(&xmp.description);
    }

    // Overwrite rating on re-import (matches overwrite semantics)
    if let Some(rating_str) = xmp.source_metadata.get("rating") {
        if let Ok(r) = rating_str.parse::<u8>() {
            asset.rating = Some(normalize_rating(r));
        }
    }

    // Overwrite color label on re-import (matches overwrite semantics)
    if let Some(label_str) = xmp.source_metadata.get("label") {
        if let Ok(Some(canonical)) = Asset::validate_color_label(label_str) {
            asset.color_label = Some(canonical);
        }
    }

    if let Some(variant) = asset.variants.iter_mut().find(|v| v.content_hash == variant_hash) {
        for (key, val) in &xmp.source_metadata {
            variant.source_metadata.insert(key.clone(), val.clone());
        }
    }
}

/// Check if a file extension supports embedded XMP extraction (JPEG/TIFF).
fn is_embedded_xmp_extension(ext: &str) -> bool {
    matches!(ext.to_lowercase().as_str(), "jpg" | "jpeg" | "tif" | "tiff")
}

/// Determine the asset type from a file extension.
pub fn determine_asset_type(ext: &str) -> AssetType {
    match ext.to_lowercase().as_str() {
        // Images
        "jpg" | "jpeg" | "png" | "gif" | "bmp" | "tiff" | "tif" | "webp" | "heic" | "heif"
        | "raw" | "cr2" | "cr3" | "crw" | "nef" | "nrw" | "arw" | "sr2" | "srf" | "orf"
        | "rw2" | "dng" | "raf" | "pef" | "srw" | "mrw" | "3fr" | "fff" | "iiq" | "erf"
        | "kdc" | "dcr" | "mef" | "mos" | "rwl" | "bay" | "x3f"
        | "svg" | "ico" | "psd" | "xcf" => AssetType::Image,
        // Video
        "mp4" | "mov" | "avi" | "mkv" | "wmv" | "flv" | "webm" | "m4v" | "mpg" | "mpeg"
        | "3gp" | "mts" | "m2ts" => AssetType::Video,
        // Audio
        "mp3" | "wav" | "flac" | "aac" | "ogg" | "wma" | "m4a" | "aiff" | "alac" => {
            AssetType::Audio
        }
        // Documents
        "pdf" | "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" | "txt" | "md" | "rtf"
        | "csv" | "json" | "xml" | "html" | "htm" => AssetType::Document,
        _ => AssetType::Other,
    }
}

/// Check if a file extension is a RAW camera format.
pub fn is_raw_extension(ext: &str) -> bool {
    matches!(
        ext.to_lowercase().as_str(),
        "raw" | "cr2" | "cr3" | "crw" | "nef" | "nrw" | "arw" | "sr2" | "srf"
            | "orf" | "rw2" | "dng" | "raf" | "pef" | "srw" | "mrw"
            | "3fr" | "fff" | "iiq" | "erf" | "kdc" | "dcr"
            | "mef" | "mos" | "rwl" | "bay" | "x3f"
    )
}

/// Infer the processing software from a recipe file extension.
fn determine_recipe_software(ext: &str) -> &'static str {
    match ext.to_lowercase().as_str() {
        "xmp" => "Adobe/CaptureOne",
        "cos" | "cot" | "cop" => "CaptureOne",
        "pp3" => "RawTherapee",
        "dop" => "DxO",
        "on1" => "ON1",
        _ => "Unknown",
    }
}

/// Group resolved files by (parent_directory, file_stem).
/// Media files are sorted with RAW extensions first, then alphabetically by extension.
/// Files with extensions not importable by the filter are skipped entirely.
fn group_by_stem(files: &[PathBuf], filter: &FileTypeFilter) -> Vec<StemGroup> {
    let mut map: BTreeMap<(PathBuf, String), StemGroup> = BTreeMap::new();

    for file in files {
        let dir = file.parent().unwrap_or_else(|| Path::new("")).to_path_buf();
        let stem = file
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        let ext = file
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");

        if !filter.is_importable(ext) {
            continue;
        }

        let key = (dir.clone(), stem.clone());
        let group = map.entry(key).or_insert_with(|| StemGroup {
            _dir: dir,
            stem,
            media_files: Vec::new(),
            recipe_files: Vec::new(),
        });

        if filter.is_recipe(ext) {
            group.recipe_files.push(file.clone());
        } else {
            group.media_files.push(file.clone());
        }
    }

    // Sort media files: RAW first, then by extension alphabetically
    for group in map.values_mut() {
        group.media_files.sort_by(|a, b| {
            let ext_a = a.extension().and_then(|e| e.to_str()).unwrap_or("");
            let ext_b = b.extension().and_then(|e| e.to_str()).unwrap_or("");
            let a_raw = is_raw_extension(ext_a);
            let b_raw = is_raw_extension(ext_b);
            match (a_raw, b_raw) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => ext_a.to_lowercase().cmp(&ext_b.to_lowercase()),
            }
        });
        group.recipe_files.sort();
    }

    map.into_values().collect()
}

/// Expand paths: if a path is a directory, recurse into it collecting files.
/// Skips hidden files/directories (starting with '.') and files matching exclude patterns.
pub fn resolve_files(paths: &[PathBuf], exclude_patterns: &[String]) -> Vec<PathBuf> {
    let mut result = Vec::new();
    for path in paths {
        if path.is_dir() {
            collect_files_recursive(path, exclude_patterns, &mut result);
        } else if path.is_file() {
            let name_str = path.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            if !is_excluded_name(&name_str, exclude_patterns) {
                result.push(path.clone());
            }
        }
    }
    result
}

fn collect_files_recursive(dir: &Path, exclude_patterns: &[String], result: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str.starts_with('.') {
            continue;
        }
        if is_excluded_name(&name_str, exclude_patterns) {
            continue;
        }
        if path.is_dir() {
            collect_files_recursive(&path, exclude_patterns, result);
        } else if path.is_file() {
            result.push(path);
        }
    }
}

/// Check if a filename matches any of the exclude patterns.
fn is_excluded_name(name: &str, patterns: &[String]) -> bool {
    for pattern in patterns {
        if glob_match::glob_match(pattern, name) {
            return true;
        }
    }
    false
}

// ═══ TESTS ═══

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn determine_asset_type_images() {
        assert_eq!(determine_asset_type("jpg"), AssetType::Image);
        assert_eq!(determine_asset_type("CR2"), AssetType::Image);
        assert_eq!(determine_asset_type("PNG"), AssetType::Image);
        // Newly added RAW formats
        assert_eq!(determine_asset_type("nrw"), AssetType::Image);
        assert_eq!(determine_asset_type("crw"), AssetType::Image);
        assert_eq!(determine_asset_type("3fr"), AssetType::Image);
        assert_eq!(determine_asset_type("iiq"), AssetType::Image);
        assert_eq!(determine_asset_type("x3f"), AssetType::Image);
    }

    #[test]
    fn determine_asset_type_video() {
        assert_eq!(determine_asset_type("mp4"), AssetType::Video);
        assert_eq!(determine_asset_type("MOV"), AssetType::Video);
    }

    #[test]
    fn determine_asset_type_unknown() {
        assert_eq!(determine_asset_type("xyz"), AssetType::Other);
        assert_eq!(determine_asset_type(""), AssetType::Other);
    }

    #[test]
    fn filter_default_includes_standard_types() {
        let f = FileTypeFilter::default();
        // Images
        assert!(f.is_importable("jpg"));
        assert!(f.is_importable("nef"));
        assert!(f.is_importable("PNG"));
        // Newly added RAW formats
        assert!(f.is_importable("nrw"));
        assert!(f.is_importable("crw"));
        assert!(f.is_importable("3fr"));
        assert!(f.is_importable("iiq"));
        assert!(f.is_importable("mrw"));
        assert!(f.is_importable("x3f"));
        // Video
        assert!(f.is_importable("mp4"));
        // Audio
        assert!(f.is_importable("mp3"));
        // XMP
        assert!(f.is_importable("xmp"));
        // Non-default groups should NOT be importable
        assert!(!f.is_importable("cos"));
        assert!(!f.is_importable("pp3"));
        assert!(!f.is_importable("dop"));
        assert!(!f.is_importable("on1"));
        assert!(!f.is_importable("pdf"));
    }

    #[test]
    fn filter_include_adds_group() {
        let mut f = FileTypeFilter::default();
        assert!(!f.is_importable("cos"));
        f.include("captureone").unwrap();
        assert!(f.is_importable("cos"));
        assert!(f.is_importable("cot"));
        assert!(f.is_importable("cop"));
    }

    #[test]
    fn filter_skip_removes_group() {
        let mut f = FileTypeFilter::default();
        assert!(f.is_importable("mp3"));
        f.skip("audio").unwrap();
        assert!(!f.is_importable("mp3"));
        assert!(!f.is_importable("wav"));
        // Other defaults still work
        assert!(f.is_importable("jpg"));
    }

    #[test]
    fn filter_unknown_group_errors() {
        let mut f = FileTypeFilter::default();
        assert!(f.include("bogus").is_err());
        assert!(f.skip("nonexistent").is_err());
    }

    #[test]
    fn filter_is_recipe_respects_enabled() {
        let f = FileTypeFilter::default();
        // xmp is recipe when enabled by default
        assert!(f.is_recipe("xmp"));
        assert!(f.is_recipe("XMP"));
        // cos is NOT recipe when captureone not enabled
        assert!(!f.is_recipe("cos"));
        assert!(!f.is_recipe("pp3"));

        let mut f2 = FileTypeFilter::default();
        f2.include("captureone").unwrap();
        assert!(f2.is_recipe("cos"));
        assert!(f2.is_recipe("cot"));
        // pp3 still not recipe (rawtherapee not enabled)
        assert!(!f2.is_recipe("pp3"));
    }

    #[test]
    fn filter_group_names_lists_all() {
        let names = FileTypeFilter::group_names();
        assert!(names.iter().any(|(n, _)| *n == "images"));
        assert!(names.iter().any(|(n, _)| *n == "captureone"));
        assert!(names.iter().any(|(n, d)| *n == "images" && *d));
        assert!(names.iter().any(|(n, d)| *n == "captureone" && !*d));
    }

    #[test]
    fn is_raw_extension_works() {
        // Original set
        assert!(is_raw_extension("nef"));
        assert!(is_raw_extension("CR2"));
        assert!(is_raw_extension("dng"));
        assert!(is_raw_extension("arw"));
        assert!(is_raw_extension("raf"));
        // Newly added formats
        assert!(is_raw_extension("nrw"));
        assert!(is_raw_extension("NRW"));
        assert!(is_raw_extension("crw"));
        assert!(is_raw_extension("mrw"));
        assert!(is_raw_extension("sr2"));
        assert!(is_raw_extension("srf"));
        assert!(is_raw_extension("3fr"));
        assert!(is_raw_extension("fff"));
        assert!(is_raw_extension("iiq"));
        assert!(is_raw_extension("erf"));
        assert!(is_raw_extension("kdc"));
        assert!(is_raw_extension("dcr"));
        assert!(is_raw_extension("mef"));
        assert!(is_raw_extension("mos"));
        assert!(is_raw_extension("rwl"));
        assert!(is_raw_extension("bay"));
        assert!(is_raw_extension("x3f"));
        // Non-RAW
        assert!(!is_raw_extension("jpg"));
        assert!(!is_raw_extension("xmp"));
        assert!(!is_raw_extension("png"));
        assert!(!is_raw_extension("tiff"));
    }

    #[test]
    fn determine_recipe_software_works() {
        assert_eq!(determine_recipe_software("xmp"), "Adobe/CaptureOne");
        assert_eq!(determine_recipe_software("cos"), "CaptureOne");
        assert_eq!(determine_recipe_software("pp3"), "RawTherapee");
        assert_eq!(determine_recipe_software("dop"), "DxO");
        assert_eq!(determine_recipe_software("on1"), "ON1");
        assert_eq!(determine_recipe_software("txt"), "Unknown");
    }

    #[test]
    fn reapply_xmp_data_overwrites_metadata() {
        let mut asset = Asset::new(AssetType::Image, "sha256:reapply_test");
        asset.description = Some("old description".to_string());
        asset.tags = vec!["existing_tag".to_string()];

        let variant = Variant {
            content_hash: "sha256:reapply_test".to_string(),
            asset_id: asset.id,
            role: VariantRole::Original,
            format: "nef".to_string(),
            file_size: 100,
            original_filename: "test.nef".to_string(),
            source_metadata: {
                let mut m = std::collections::BTreeMap::new();
                m.insert("rating".to_string(), "3".to_string());
                m.insert("exif_key".to_string(), "exif_value".to_string());
                m
            },
            locations: vec![],
        };
        asset.variants.push(variant);

        let xmp = crate::xmp_reader::XmpData {
            keywords: vec!["new_tag".to_string(), "existing_tag".to_string()],
            hierarchical_keywords: vec![],
            description: Some("new description".to_string()),
            source_metadata: {
                let mut m = std::collections::HashMap::new();
                m.insert("rating".to_string(), "5".to_string());
                m.insert("label".to_string(), "Red".to_string());
                m
            },
        };

        reapply_xmp_data(&xmp, &mut asset, "sha256:reapply_test");

        // Description overwritten
        assert_eq!(asset.description.as_deref(), Some("new description"));
        // Tags merged (no duplicates)
        assert!(asset.tags.contains(&"existing_tag".to_string()));
        assert!(asset.tags.contains(&"new_tag".to_string()));
        assert_eq!(asset.tags.len(), 2);
        // source_metadata: XMP keys overwritten, non-XMP keys preserved
        let meta = &asset.variants[0].source_metadata;
        assert_eq!(meta.get("rating").unwrap(), "5");
        assert_eq!(meta.get("label").unwrap(), "Red");
        assert_eq!(meta.get("exif_key").unwrap(), "exif_value");
    }

    #[test]
    fn group_by_stem_basic() {
        let filter = FileTypeFilter::default();
        let files = vec![
            PathBuf::from("/photos/DSC_001.nef"),
            PathBuf::from("/photos/DSC_001.jpg"),
            PathBuf::from("/photos/DSC_001.xmp"),
            PathBuf::from("/photos/DSC_002.jpg"),
        ];
        let groups = group_by_stem(&files, &filter);
        assert_eq!(groups.len(), 2);

        let g1 = groups.iter().find(|g| g.stem == "DSC_001").unwrap();
        assert_eq!(g1.media_files.len(), 2);
        assert_eq!(g1.recipe_files.len(), 1);
        // RAW should be first
        assert!(g1.media_files[0].to_str().unwrap().ends_with(".nef"));
        assert!(g1.media_files[1].to_str().unwrap().ends_with(".jpg"));

        let g2 = groups.iter().find(|g| g.stem == "DSC_002").unwrap();
        assert_eq!(g2.media_files.len(), 1);
        assert_eq!(g2.recipe_files.len(), 0);
    }

    #[test]
    fn group_by_stem_different_dirs_separate() {
        let filter = FileTypeFilter::default();
        let files = vec![
            PathBuf::from("/a/photo.jpg"),
            PathBuf::from("/b/photo.jpg"),
        ];
        let groups = group_by_stem(&files, &filter);
        assert_eq!(groups.len(), 2);
    }

    #[test]
    fn group_by_stem_skips_non_importable() {
        let filter = FileTypeFilter::default();
        let files = vec![
            PathBuf::from("/photos/DSC_001.nef"),
            PathBuf::from("/photos/DSC_001.cos"), // captureone not enabled
            PathBuf::from("/photos/readme.pdf"),   // documents not enabled
        ];
        let groups = group_by_stem(&files, &filter);
        assert_eq!(groups.len(), 1);
        let g = &groups[0];
        assert_eq!(g.media_files.len(), 1);
        assert_eq!(g.recipe_files.len(), 0);
    }

    #[test]
    fn resolve_files_skips_hidden() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("visible.txt"), "hello").unwrap();
        std::fs::write(dir.path().join(".hidden"), "secret").unwrap();

        let files = resolve_files(&[dir.path().to_path_buf()], &[]);
        assert_eq!(files.len(), 1);
        assert!(files[0].file_name().unwrap().to_str().unwrap() == "visible.txt");
    }

    #[test]
    fn resolve_files_recurses_directories() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("subdir");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(dir.path().join("a.txt"), "a").unwrap();
        std::fs::write(sub.join("b.txt"), "b").unwrap();

        let files = resolve_files(&[dir.path().to_path_buf()], &[]);
        assert_eq!(files.len(), 2);
    }

    /// Default filter for import tests.
    fn default_filter() -> FileTypeFilter {
        FileTypeFilter::default()
    }

    /// Set up a minimal catalog in a temp directory for import tests.
    fn setup_catalog(dir: &std::path::Path) {
        std::fs::create_dir_all(dir.join("metadata")).unwrap();
        crate::config::CatalogConfig::default().save(dir).unwrap();
        let catalog = crate::catalog::Catalog::open(dir).unwrap();
        catalog.initialize().unwrap();
    }

    #[test]
    fn import_duplicate_from_different_path_adds_location() {
        let catalog_dir = tempfile::tempdir().unwrap();
        setup_catalog(catalog_dir.path());

        // Create a volume with two copies of the same file at different paths
        let vol_dir = tempfile::tempdir().unwrap();
        let dir_a = vol_dir.path().join("a");
        let dir_b = vol_dir.path().join("b");
        std::fs::create_dir_all(&dir_a).unwrap();
        std::fs::create_dir_all(&dir_b).unwrap();
        std::fs::write(dir_a.join("photo.jpg"), "identical content").unwrap();
        std::fs::write(dir_b.join("photo.jpg"), "identical content").unwrap();

        let volume = crate::models::Volume::new(
            "test-vol".into(),
            vol_dir.path().to_path_buf(),
            crate::models::VolumeType::Local,
        );

        let service = AssetService::new(catalog_dir.path(), crate::Verbosity::quiet(), &crate::config::PreviewConfig::default());

        // First import
        let r1 = service.import(&[dir_a.join("photo.jpg")], &volume, &default_filter()).unwrap();
        assert_eq!(r1.imported, 1);
        assert_eq!(r1.locations_added, 0);
        assert_eq!(r1.skipped, 0);

        // Second import — same content, different path
        let r2 = service.import(&[dir_b.join("photo.jpg")], &volume, &default_filter()).unwrap();
        assert_eq!(r2.imported, 0);
        assert_eq!(r2.locations_added, 1);
        assert_eq!(r2.skipped, 0);

        // Verify sidecar has 2 locations
        let catalog = crate::catalog::Catalog::open(catalog_dir.path()).unwrap();
        let content_hash = crate::content_store::ContentStore::new(catalog_dir.path())
            .ingest(&dir_a.join("photo.jpg"), &volume)
            .unwrap();
        let asset_id_str = catalog.find_asset_id_by_variant(&content_hash).unwrap().unwrap();
        let asset_id: uuid::Uuid = asset_id_str.parse().unwrap();
        let metadata_store = crate::metadata_store::MetadataStore::new(catalog_dir.path());
        let asset = metadata_store.load(asset_id).unwrap();
        let variant = &asset.variants[0];
        assert_eq!(variant.locations.len(), 2);
        assert_eq!(
            variant.locations[0].relative_path,
            std::path::Path::new("a/photo.jpg")
        );
        assert_eq!(
            variant.locations[1].relative_path,
            std::path::Path::new("b/photo.jpg")
        );
    }

    #[test]
    fn import_duplicate_from_same_path_is_skipped() {
        let catalog_dir = tempfile::tempdir().unwrap();
        setup_catalog(catalog_dir.path());

        let vol_dir = tempfile::tempdir().unwrap();
        std::fs::write(vol_dir.path().join("photo.jpg"), "some content").unwrap();

        let volume = crate::models::Volume::new(
            "test-vol".into(),
            vol_dir.path().to_path_buf(),
            crate::models::VolumeType::Local,
        );

        let service = AssetService::new(catalog_dir.path(), crate::Verbosity::quiet(), &crate::config::PreviewConfig::default());

        // First import
        let r1 = service
            .import(&[vol_dir.path().join("photo.jpg")], &volume, &default_filter())
            .unwrap();
        assert_eq!(r1.imported, 1);

        // Second import — exact same path
        let r2 = service
            .import(&[vol_dir.path().join("photo.jpg")], &volume, &default_filter())
            .unwrap();
        assert_eq!(r2.imported, 0);
        assert_eq!(r2.locations_added, 0);
        assert_eq!(r2.skipped, 1);
    }

    #[test]
    fn import_raw_and_jpg_groups_into_one_asset() {
        let catalog_dir = tempfile::tempdir().unwrap();
        setup_catalog(catalog_dir.path());

        let vol_dir = tempfile::tempdir().unwrap();
        let photos = vol_dir.path().join("photos");
        std::fs::create_dir_all(&photos).unwrap();
        std::fs::write(photos.join("DSC_4521.nef"), "raw file content").unwrap();
        std::fs::write(photos.join("DSC_4521.jpg"), "jpeg file content").unwrap();

        let volume = crate::models::Volume::new(
            "test-vol".into(),
            vol_dir.path().to_path_buf(),
            crate::models::VolumeType::Local,
        );

        let service = AssetService::new(catalog_dir.path(), crate::Verbosity::quiet(), &crate::config::PreviewConfig::default());
        let result = service
            .import(&[photos.clone()], &volume, &default_filter())
            .unwrap();

        assert_eq!(result.imported, 2);
        assert_eq!(result.recipes_attached, 0);

        // Verify: one asset with two variants
        let metadata_store = crate::metadata_store::MetadataStore::new(catalog_dir.path());
        let summaries = metadata_store.list().unwrap();
        assert_eq!(summaries.len(), 1);

        let asset = metadata_store.load(summaries[0].id).unwrap();
        assert_eq!(asset.variants.len(), 2);
        // RAW should be first (defines the asset)
        assert_eq!(asset.variants[0].format, "nef");
        assert_eq!(asset.variants[1].format, "jpg");
        // RAW is original, JPG is an alternate
        assert_eq!(asset.variants[0].role, VariantRole::Original);
        assert_eq!(asset.variants[1].role, VariantRole::Alternate);
        assert_eq!(asset.name.as_deref(), Some("DSC_4521"));
    }

    #[test]
    fn import_two_jpgs_both_original() {
        let catalog_dir = tempfile::tempdir().unwrap();
        setup_catalog(catalog_dir.path());

        let vol_dir = tempfile::tempdir().unwrap();
        let photos = vol_dir.path().join("photos");
        std::fs::create_dir_all(&photos).unwrap();
        std::fs::write(photos.join("photo.jpg"), "jpeg content").unwrap();
        std::fs::write(photos.join("photo.png"), "png content").unwrap();

        let volume = crate::models::Volume::new(
            "test-vol".into(),
            vol_dir.path().to_path_buf(),
            crate::models::VolumeType::Local,
        );

        let service = AssetService::new(catalog_dir.path(), crate::Verbosity::quiet(), &crate::config::PreviewConfig::default());
        let result = service
            .import(&[photos.clone()], &volume, &default_filter())
            .unwrap();

        assert_eq!(result.imported, 2);

        let metadata_store = crate::metadata_store::MetadataStore::new(catalog_dir.path());
        let summaries = metadata_store.list().unwrap();
        assert_eq!(summaries.len(), 1);

        let asset = metadata_store.load(summaries[0].id).unwrap();
        assert_eq!(asset.variants.len(), 2);
        // No RAW present → both variants stay Original
        assert_eq!(asset.variants[0].role, VariantRole::Original);
        assert_eq!(asset.variants[1].role, VariantRole::Original);
    }

    #[test]
    fn import_raw_and_xmp_creates_asset_with_recipe() {
        let catalog_dir = tempfile::tempdir().unwrap();
        setup_catalog(catalog_dir.path());

        let vol_dir = tempfile::tempdir().unwrap();
        let photos = vol_dir.path().join("photos");
        std::fs::create_dir_all(&photos).unwrap();
        std::fs::write(photos.join("DSC_4521.nef"), "raw file data").unwrap();
        std::fs::write(photos.join("DSC_4521.xmp"), "xmp sidecar data").unwrap();

        let volume = crate::models::Volume::new(
            "test-vol".into(),
            vol_dir.path().to_path_buf(),
            crate::models::VolumeType::Local,
        );

        let service = AssetService::new(catalog_dir.path(), crate::Verbosity::quiet(), &crate::config::PreviewConfig::default());
        let result = service
            .import(&[photos.clone()], &volume, &default_filter())
            .unwrap();

        assert_eq!(result.imported, 1);
        assert_eq!(result.recipes_attached, 1);

        let metadata_store = crate::metadata_store::MetadataStore::new(catalog_dir.path());
        let summaries = metadata_store.list().unwrap();
        assert_eq!(summaries.len(), 1);

        let asset = metadata_store.load(summaries[0].id).unwrap();
        assert_eq!(asset.variants.len(), 1);
        assert_eq!(asset.recipes.len(), 1);
        assert_eq!(asset.recipes[0].software, "Adobe/CaptureOne");
    }

    #[test]
    fn import_raw_jpg_xmp_cos_groups_correctly() {
        let catalog_dir = tempfile::tempdir().unwrap();
        setup_catalog(catalog_dir.path());

        let vol_dir = tempfile::tempdir().unwrap();
        let photos = vol_dir.path().join("photos");
        std::fs::create_dir_all(&photos).unwrap();
        std::fs::write(photos.join("DSC_4521.nef"), "raw data").unwrap();
        std::fs::write(photos.join("DSC_4521.jpg"), "jpeg data").unwrap();
        std::fs::write(photos.join("DSC_4521.xmp"), "xmp data").unwrap();
        std::fs::write(photos.join("DSC_4521.cos"), "cos data").unwrap();

        let volume = crate::models::Volume::new(
            "test-vol".into(),
            vol_dir.path().to_path_buf(),
            crate::models::VolumeType::Local,
        );

        let service = AssetService::new(catalog_dir.path(), crate::Verbosity::quiet(), &crate::config::PreviewConfig::default());
        let mut filter = FileTypeFilter::default();
        filter.include("captureone").unwrap();
        let result = service.import(&[photos.clone()], &volume, &filter).unwrap();

        assert_eq!(result.imported, 2);
        assert_eq!(result.recipes_attached, 2);

        let metadata_store = crate::metadata_store::MetadataStore::new(catalog_dir.path());
        let summaries = metadata_store.list().unwrap();
        assert_eq!(summaries.len(), 1);

        let asset = metadata_store.load(summaries[0].id).unwrap();
        assert_eq!(asset.variants.len(), 2);
        assert_eq!(asset.recipes.len(), 2);
    }

    #[test]
    fn import_same_stem_different_dirs_creates_separate_assets() {
        let catalog_dir = tempfile::tempdir().unwrap();
        setup_catalog(catalog_dir.path());

        let vol_dir = tempfile::tempdir().unwrap();
        let dir_a = vol_dir.path().join("a");
        let dir_b = vol_dir.path().join("b");
        std::fs::create_dir_all(&dir_a).unwrap();
        std::fs::create_dir_all(&dir_b).unwrap();
        // Different content so they don't dedup
        std::fs::write(dir_a.join("photo.jpg"), "content A").unwrap();
        std::fs::write(dir_b.join("photo.jpg"), "content B").unwrap();

        let volume = crate::models::Volume::new(
            "test-vol".into(),
            vol_dir.path().to_path_buf(),
            crate::models::VolumeType::Local,
        );

        let service = AssetService::new(catalog_dir.path(), crate::Verbosity::quiet(), &crate::config::PreviewConfig::default());
        let result = service
            .import(&[dir_a, dir_b], &volume, &default_filter())
            .unwrap();

        assert_eq!(result.imported, 2);

        let metadata_store = crate::metadata_store::MetadataStore::new(catalog_dir.path());
        let summaries = metadata_store.list().unwrap();
        assert_eq!(summaries.len(), 2);
    }

    #[test]
    fn import_solo_file_works_unchanged() {
        let catalog_dir = tempfile::tempdir().unwrap();
        setup_catalog(catalog_dir.path());

        let vol_dir = tempfile::tempdir().unwrap();
        std::fs::write(vol_dir.path().join("solo.jpg"), "solo content").unwrap();

        let volume = crate::models::Volume::new(
            "test-vol".into(),
            vol_dir.path().to_path_buf(),
            crate::models::VolumeType::Local,
        );

        let service = AssetService::new(catalog_dir.path(), crate::Verbosity::quiet(), &crate::config::PreviewConfig::default());
        let result = service
            .import(&[vol_dir.path().join("solo.jpg")], &volume, &default_filter())
            .unwrap();

        assert_eq!(result.imported, 1);
        assert_eq!(result.recipes_attached, 0);

        let metadata_store = crate::metadata_store::MetadataStore::new(catalog_dir.path());
        let summaries = metadata_store.list().unwrap();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].variant_count, 1);
    }

    /// Helper to create an XMP sidecar with given keywords, rating, label, description.
    fn make_xmp(
        keywords: &[&str],
        rating: Option<u8>,
        label: Option<&str>,
        description: Option<&str>,
        creator: Option<&str>,
        copyright: Option<&str>,
    ) -> String {
        let mut parts = Vec::new();
        parts.push(r#"<?xml version="1.0" encoding="UTF-8"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:dc="http://purl.org/dc/elements/1.1/"
    xmlns:xmp="http://ns.adobe.com/xap/1.0/""#.to_string());
        if let Some(r) = rating {
            parts.push(format!("\n    xmp:Rating=\"{r}\""));
        }
        if let Some(l) = label {
            parts.push(format!("\n    xmp:Label=\"{l}\""));
        }
        parts.push(">".to_string());
        if !keywords.is_empty() {
            parts.push("   <dc:subject>\n    <rdf:Bag>".to_string());
            for kw in keywords {
                parts.push(format!("     <rdf:li>{kw}</rdf:li>"));
            }
            parts.push("    </rdf:Bag>\n   </dc:subject>".to_string());
        }
        if let Some(desc) = description {
            parts.push(format!(
                "   <dc:description>\n    <rdf:Alt>\n     <rdf:li xml:lang=\"x-default\">{desc}</rdf:li>\n    </rdf:Alt>\n   </dc:description>"
            ));
        }
        if let Some(c) = creator {
            parts.push(format!(
                "   <dc:creator>\n    <rdf:Seq>\n     <rdf:li>{c}</rdf:li>\n    </rdf:Seq>\n   </dc:creator>"
            ));
        }
        if let Some(cr) = copyright {
            parts.push(format!(
                "   <dc:rights>\n    <rdf:Alt>\n     <rdf:li xml:lang=\"x-default\">{cr}</rdf:li>\n    </rdf:Alt>\n   </dc:rights>"
            ));
        }
        parts.push("  </rdf:Description>\n </rdf:RDF>\n</x:xmpmeta>".to_string());
        parts.join("\n")
    }

    #[test]
    fn import_xmp_extracts_tags_and_metadata() {
        let catalog_dir = tempfile::tempdir().unwrap();
        setup_catalog(catalog_dir.path());

        let vol_dir = tempfile::tempdir().unwrap();
        let photos = vol_dir.path().join("photos");
        std::fs::create_dir_all(&photos).unwrap();
        std::fs::write(photos.join("DSC_001.nef"), "raw file data").unwrap();
        let xmp = make_xmp(
            &["landscape", "sunset"],
            Some(4),
            Some("Blue"),
            None,
            None,
            None,
        );
        std::fs::write(photos.join("DSC_001.xmp"), &xmp).unwrap();

        let volume = crate::models::Volume::new(
            "test-vol".into(),
            vol_dir.path().to_path_buf(),
            crate::models::VolumeType::Local,
        );

        let service = AssetService::new(catalog_dir.path(), crate::Verbosity::quiet(), &crate::config::PreviewConfig::default());
        let result = service.import(&[photos], &volume, &default_filter()).unwrap();
        assert_eq!(result.imported, 1);
        assert_eq!(result.recipes_attached, 1);

        let metadata_store = crate::metadata_store::MetadataStore::new(catalog_dir.path());
        let summaries = metadata_store.list().unwrap();
        let asset = metadata_store.load(summaries[0].id).unwrap();

        assert!(asset.tags.contains(&"landscape".to_string()));
        assert!(asset.tags.contains(&"sunset".to_string()));
        assert_eq!(
            asset.variants[0].source_metadata.get("rating").map(|s| s.as_str()),
            Some("4")
        );
        assert_eq!(
            asset.variants[0].source_metadata.get("label").map(|s| s.as_str()),
            Some("Blue")
        );
    }

    #[test]
    fn import_xmp_sets_description() {
        let catalog_dir = tempfile::tempdir().unwrap();
        setup_catalog(catalog_dir.path());

        let vol_dir = tempfile::tempdir().unwrap();
        let photos = vol_dir.path().join("photos");
        std::fs::create_dir_all(&photos).unwrap();
        std::fs::write(photos.join("DSC_002.nef"), "raw data 2").unwrap();
        let xmp = make_xmp(&[], None, None, Some("A great photo"), None, None);
        std::fs::write(photos.join("DSC_002.xmp"), &xmp).unwrap();

        let volume = crate::models::Volume::new(
            "test-vol".into(),
            vol_dir.path().to_path_buf(),
            crate::models::VolumeType::Local,
        );

        let service = AssetService::new(catalog_dir.path(), crate::Verbosity::quiet(), &crate::config::PreviewConfig::default());
        service.import(&[photos], &volume, &default_filter()).unwrap();

        let metadata_store = crate::metadata_store::MetadataStore::new(catalog_dir.path());
        let summaries = metadata_store.list().unwrap();
        let asset = metadata_store.load(summaries[0].id).unwrap();
        assert_eq!(asset.description.as_deref(), Some("A great photo"));
    }

    #[test]
    fn import_xmp_does_not_overwrite_existing_tags() {
        let catalog_dir = tempfile::tempdir().unwrap();
        setup_catalog(catalog_dir.path());

        let vol_dir = tempfile::tempdir().unwrap();
        let photos = vol_dir.path().join("photos");
        std::fs::create_dir_all(&photos).unwrap();
        std::fs::write(photos.join("DSC_003.nef"), "raw data 3").unwrap();
        let xmp = make_xmp(&["nature", "forest"], Some(5), None, None, None, None);
        std::fs::write(photos.join("DSC_003.xmp"), &xmp).unwrap();

        let volume = crate::models::Volume::new(
            "test-vol".into(),
            vol_dir.path().to_path_buf(),
            crate::models::VolumeType::Local,
        );

        let service = AssetService::new(catalog_dir.path(), crate::Verbosity::quiet(), &crate::config::PreviewConfig::default());
        service.import(&[photos], &volume, &default_filter()).unwrap();

        // Now manually add a tag to the asset
        let metadata_store = crate::metadata_store::MetadataStore::new(catalog_dir.path());
        let summaries = metadata_store.list().unwrap();
        let mut asset = metadata_store.load(summaries[0].id).unwrap();
        asset.tags.push("manual-tag".to_string());
        metadata_store.save(&asset).unwrap();

        // Verify original XMP tags + manual tag are all present
        let asset = metadata_store.load(summaries[0].id).unwrap();
        assert!(asset.tags.contains(&"nature".to_string()));
        assert!(asset.tags.contains(&"forest".to_string()));
        assert!(asset.tags.contains(&"manual-tag".to_string()));
    }

    /// Set up a catalog with volumes registered for relocate tests.
    /// Returns (catalog_dir, vol1_dir, vol2_dir, vol1, vol2).
    fn setup_relocate() -> (
        tempfile::TempDir,
        tempfile::TempDir,
        tempfile::TempDir,
        Volume,
        Volume,
    ) {
        let catalog_dir = tempfile::tempdir().unwrap();
        setup_catalog(catalog_dir.path());
        crate::device_registry::DeviceRegistry::init(catalog_dir.path()).unwrap();

        let vol1_dir = tempfile::tempdir().unwrap();
        let vol2_dir = tempfile::tempdir().unwrap();

        let registry = crate::device_registry::DeviceRegistry::new(catalog_dir.path());
        let vol1 = registry
            .register("vol1", vol1_dir.path(), crate::models::VolumeType::Local, None)
            .unwrap();
        let vol2 = registry
            .register("vol2", vol2_dir.path(), crate::models::VolumeType::Local, None)
            .unwrap();

        // Ensure volumes are in the catalog DB too
        let catalog = crate::catalog::Catalog::open(catalog_dir.path()).unwrap();
        catalog.ensure_volume(&vol1).unwrap();
        catalog.ensure_volume(&vol2).unwrap();

        (catalog_dir, vol1_dir, vol2_dir, vol1, vol2)
    }

    #[test]
    fn relocate_copies_variant_to_target_volume() {
        let (catalog_dir, vol1_dir, vol2_dir, vol1, _vol2) = setup_relocate();

        // Create a file on vol1
        std::fs::write(vol1_dir.path().join("photo.jpg"), "photo data").unwrap();

        let service = AssetService::new(catalog_dir.path(), crate::Verbosity::quiet(), &crate::config::PreviewConfig::default());
        service
            .import(
                &[vol1_dir.path().join("photo.jpg")],
                &vol1,
                &default_filter(),
            )
            .unwrap();

        // Get asset ID
        let metadata_store = MetadataStore::new(catalog_dir.path());
        let summaries = metadata_store.list().unwrap();
        let asset_id = summaries[0].id.to_string();

        // Relocate to vol2
        let result = service.relocate(&asset_id, "vol2", false, false, false).unwrap();
        assert_eq!(result.copied, 1);
        assert_eq!(result.removed, 0);

        // Verify file exists on vol2
        assert!(vol2_dir.path().join("photo.jpg").exists());
        // File also still on vol1
        assert!(vol1_dir.path().join("photo.jpg").exists());

        // Verify sidecar has locations on both volumes
        let asset = metadata_store.load(summaries[0].id).unwrap();
        assert_eq!(asset.variants[0].locations.len(), 2);
    }

    #[test]
    fn relocate_with_remove_source_moves_files() {
        let (catalog_dir, vol1_dir, vol2_dir, vol1, _vol2) = setup_relocate();

        std::fs::write(vol1_dir.path().join("photo.jpg"), "move me").unwrap();

        let service = AssetService::new(catalog_dir.path(), crate::Verbosity::quiet(), &crate::config::PreviewConfig::default());
        service
            .import(
                &[vol1_dir.path().join("photo.jpg")],
                &vol1,
                &default_filter(),
            )
            .unwrap();

        let metadata_store = MetadataStore::new(catalog_dir.path());
        let summaries = metadata_store.list().unwrap();
        let asset_id = summaries[0].id.to_string();

        let result = service.relocate(&asset_id, "vol2", true, false, false).unwrap();
        assert_eq!(result.copied, 1);
        assert_eq!(result.removed, 1);

        // File should be on vol2 but not on vol1
        assert!(vol2_dir.path().join("photo.jpg").exists());
        assert!(!vol1_dir.path().join("photo.jpg").exists());

        // Sidecar should have only vol2 location
        let asset = metadata_store.load(summaries[0].id).unwrap();
        assert_eq!(asset.variants[0].locations.len(), 1);
        assert_eq!(asset.variants[0].locations[0].volume_id, _vol2.id);
    }

    #[test]
    fn relocate_copies_recipes_alongside_variants() {
        let (catalog_dir, vol1_dir, vol2_dir, vol1, _vol2) = setup_relocate();

        let photos = vol1_dir.path().join("photos");
        std::fs::create_dir_all(&photos).unwrap();
        std::fs::write(photos.join("DSC.nef"), "raw data for relocate").unwrap();
        std::fs::write(photos.join("DSC.xmp"), "xmp recipe data").unwrap();

        let service = AssetService::new(catalog_dir.path(), crate::Verbosity::quiet(), &crate::config::PreviewConfig::default());
        service
            .import(&[photos], &vol1, &default_filter())
            .unwrap();

        let metadata_store = MetadataStore::new(catalog_dir.path());
        let summaries = metadata_store.list().unwrap();
        let asset_id = summaries[0].id.to_string();

        let result = service.relocate(&asset_id, "vol2", false, false, false).unwrap();
        // Should copy both variant and recipe
        assert_eq!(result.copied, 2);

        // Both files should exist on vol2
        assert!(vol2_dir.path().join("photos/DSC.nef").exists());
        assert!(vol2_dir.path().join("photos/DSC.xmp").exists());
    }

    #[test]
    fn relocate_skips_already_present_files() {
        let (catalog_dir, vol1_dir, _vol2_dir, vol1, _vol2) = setup_relocate();

        std::fs::write(vol1_dir.path().join("photo.jpg"), "skip test").unwrap();

        let service = AssetService::new(catalog_dir.path(), crate::Verbosity::quiet(), &crate::config::PreviewConfig::default());
        service
            .import(
                &[vol1_dir.path().join("photo.jpg")],
                &vol1,
                &default_filter(),
            )
            .unwrap();

        let metadata_store = MetadataStore::new(catalog_dir.path());
        let summaries = metadata_store.list().unwrap();
        let asset_id = summaries[0].id.to_string();

        // First relocate
        service.relocate(&asset_id, "vol2", false, false, false).unwrap();

        // Second relocate — vol1 location still generates a plan entry,
        // but the file already exists on vol2 with matching hash, so it's skipped
        let result = service.relocate(&asset_id, "vol2", false, false, false).unwrap();
        assert_eq!(result.copied, 0);
        assert_eq!(result.skipped, 1);
        assert!(result.actions[0].contains("already exists"));
    }

    #[test]
    fn relocate_dry_run_makes_no_changes() {
        let (catalog_dir, vol1_dir, vol2_dir, vol1, _vol2) = setup_relocate();

        std::fs::write(vol1_dir.path().join("photo.jpg"), "dry run test").unwrap();

        let service = AssetService::new(catalog_dir.path(), crate::Verbosity::quiet(), &crate::config::PreviewConfig::default());
        service
            .import(
                &[vol1_dir.path().join("photo.jpg")],
                &vol1,
                &default_filter(),
            )
            .unwrap();

        let metadata_store = MetadataStore::new(catalog_dir.path());
        let summaries = metadata_store.list().unwrap();
        let asset_id = summaries[0].id.to_string();

        let result = service.relocate(&asset_id, "vol2", false, false, true).unwrap();
        assert_eq!(result.copied, 1);

        // File should NOT exist on vol2
        assert!(!vol2_dir.path().join("photo.jpg").exists());

        // Sidecar should still only have vol1 location
        let asset = metadata_store.load(summaries[0].id).unwrap();
        assert_eq!(asset.variants[0].locations.len(), 1);
    }

    #[test]
    fn relocate_noop_when_already_on_target() {
        let (catalog_dir, vol1_dir, _vol2_dir, vol1, _vol2) = setup_relocate();

        std::fs::write(vol1_dir.path().join("photo.jpg"), "noop test").unwrap();

        let service = AssetService::new(catalog_dir.path(), crate::Verbosity::quiet(), &crate::config::PreviewConfig::default());
        service
            .import(
                &[vol1_dir.path().join("photo.jpg")],
                &vol1,
                &default_filter(),
            )
            .unwrap();

        let metadata_store = MetadataStore::new(catalog_dir.path());
        let summaries = metadata_store.list().unwrap();
        let asset_id = summaries[0].id.to_string();

        // Relocate to same volume
        let result = service.relocate(&asset_id, "vol1", false, false, false).unwrap();
        assert_eq!(result.copied, 0);
        assert_eq!(result.skipped, 0);
        assert!(result.actions[0].contains("already on target"));
    }

    #[test]
    fn relocate_fails_if_target_volume_offline() {
        let (catalog_dir, vol1_dir, _vol2_dir, vol1, _vol2) = setup_relocate();

        std::fs::write(vol1_dir.path().join("photo.jpg"), "offline test").unwrap();

        let service = AssetService::new(catalog_dir.path(), crate::Verbosity::quiet(), &crate::config::PreviewConfig::default());
        service
            .import(
                &[vol1_dir.path().join("photo.jpg")],
                &vol1,
                &default_filter(),
            )
            .unwrap();

        let metadata_store = MetadataStore::new(catalog_dir.path());
        let summaries = metadata_store.list().unwrap();
        let asset_id = summaries[0].id.to_string();

        // Register an offline volume
        let registry = crate::device_registry::DeviceRegistry::new(catalog_dir.path());
        registry
            .register(
                "offline-vol",
                std::path::Path::new("/nonexistent/mount"),
                crate::models::VolumeType::External,
                None,
            )
            .unwrap();

        let err = service
            .relocate(&asset_id, "offline-vol", false, false, false)
            .unwrap_err();
        assert!(err.to_string().contains("offline"));
    }

    #[test]
    fn relocate_fails_for_unknown_asset_id() {
        let (catalog_dir, _vol1_dir, _vol2_dir, _vol1, _vol2) = setup_relocate();

        let service = AssetService::new(catalog_dir.path(), crate::Verbosity::quiet(), &crate::config::PreviewConfig::default());
        let err = service
            .relocate("nonexistent-id", "vol2", false, false, false)
            .unwrap_err();
        assert!(err.to_string().contains("no asset found"));
    }

    #[test]
    fn relocate_fails_for_unknown_volume() {
        let (catalog_dir, vol1_dir, _vol2_dir, vol1, _vol2) = setup_relocate();

        std::fs::write(vol1_dir.path().join("photo.jpg"), "unknown vol test").unwrap();

        let service = AssetService::new(catalog_dir.path(), crate::Verbosity::quiet(), &crate::config::PreviewConfig::default());
        service
            .import(
                &[vol1_dir.path().join("photo.jpg")],
                &vol1,
                &default_filter(),
            )
            .unwrap();

        let metadata_store = MetadataStore::new(catalog_dir.path());
        let summaries = metadata_store.list().unwrap();
        let asset_id = summaries[0].id.to_string();

        let err = service
            .relocate(&asset_id, "nonexistent-vol", false, false, false)
            .unwrap_err();
        assert!(err.to_string().contains("no volume found"));
    }

    #[test]
    fn resolve_files_excludes_patterns() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("photo.jpg"), "image").unwrap();
        std::fs::write(dir.path().join("Thumbs.db"), "thumbs").unwrap();
        std::fs::write(dir.path().join("cache.tmp"), "temp").unwrap();
        std::fs::write(dir.path().join("notes.txt"), "notes").unwrap();

        let exclude = vec!["Thumbs.db".to_string(), "*.tmp".to_string()];
        let files = resolve_files(&[dir.path().to_path_buf()], &exclude);
        let names: Vec<String> = files
            .iter()
            .map(|f| f.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert!(names.contains(&"photo.jpg".to_string()));
        assert!(names.contains(&"notes.txt".to_string()));
        assert!(!names.contains(&"Thumbs.db".to_string()));
        assert!(!names.contains(&"cache.tmp".to_string()));
    }

    #[test]
    fn resolve_files_exclude_in_subdirs() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("subdir");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("photo.jpg"), "image").unwrap();
        std::fs::write(sub.join("Thumbs.db"), "thumbs").unwrap();

        let exclude = vec!["Thumbs.db".to_string()];
        let files = resolve_files(&[dir.path().to_path_buf()], &exclude);
        assert_eq!(files.len(), 1);
        assert!(files[0].file_name().unwrap().to_str().unwrap() == "photo.jpg");
    }

    #[test]
    fn import_auto_tags_applied() {
        let catalog_dir = tempfile::tempdir().unwrap();
        setup_catalog(catalog_dir.path());

        let vol_dir = tempfile::tempdir().unwrap();
        std::fs::write(vol_dir.path().join("photo.jpg"), "test content for tags").unwrap();

        let volume = crate::models::Volume::new(
            "test-vol".into(),
            vol_dir.path().to_path_buf(),
            crate::models::VolumeType::Local,
        );

        let service = AssetService::new(catalog_dir.path(), crate::Verbosity::quiet(), &crate::config::PreviewConfig::default());
        let auto_tags = vec!["inbox".to_string(), "unreviewed".to_string()];
        let result = service.import_with_callback(
            &[vol_dir.path().join("photo.jpg")],
            &volume,
            &default_filter(),
            &[],
            &auto_tags,
            false,
            false,
            |_, _, _| {},
        ).unwrap();

        assert_eq!(result.imported, 1);

        let metadata_store = crate::metadata_store::MetadataStore::new(catalog_dir.path());
        let summaries = metadata_store.list().unwrap();
        let asset = metadata_store.load(summaries[0].id).unwrap();
        assert!(asset.tags.contains(&"inbox".to_string()));
        assert!(asset.tags.contains(&"unreviewed".to_string()));
    }

    #[test]
    fn import_exclude_patterns_skip_files() {
        let catalog_dir = tempfile::tempdir().unwrap();
        setup_catalog(catalog_dir.path());

        let vol_dir = tempfile::tempdir().unwrap();
        std::fs::write(vol_dir.path().join("photo.jpg"), "real image").unwrap();
        std::fs::write(vol_dir.path().join("Thumbs.db"), "thumbnail cache").unwrap();

        let volume = crate::models::Volume::new(
            "test-vol".into(),
            vol_dir.path().to_path_buf(),
            crate::models::VolumeType::Local,
        );

        let service = AssetService::new(catalog_dir.path(), crate::Verbosity::quiet(), &crate::config::PreviewConfig::default());
        let exclude = vec!["Thumbs.db".to_string()];
        let result = service.import_with_callback(
            &[vol_dir.path().to_path_buf()],
            &volume,
            &default_filter(),
            &exclude,
            &[],
            false,
            false,
            |_, _, _| {},
        ).unwrap();

        assert_eq!(result.imported, 1);
    }

    #[test]
    fn is_excluded_name_works() {
        assert!(is_excluded_name("Thumbs.db", &["Thumbs.db".to_string()]));
        assert!(is_excluded_name("cache.tmp", &["*.tmp".to_string()]));
        assert!(is_excluded_name(".DS_Store", &[".DS_Store".to_string()]));
        assert!(!is_excluded_name("photo.jpg", &["*.tmp".to_string()]));
        assert!(!is_excluded_name("photo.jpg", &[]));
    }

    #[test]
    fn fix_roles_corrects_raw_jpg_pair() {
        let catalog_dir = tempfile::tempdir().unwrap();
        setup_catalog(catalog_dir.path());

        let vol_dir = tempfile::tempdir().unwrap();
        let photos = vol_dir.path().join("photos");
        std::fs::create_dir_all(&photos).unwrap();
        std::fs::write(photos.join("DSC_001.nef"), "raw file content").unwrap();
        std::fs::write(photos.join("DSC_001.jpg"), "jpeg file content").unwrap();

        let volume = crate::models::Volume::new(
            "test-vol".into(),
            vol_dir.path().to_path_buf(),
            crate::models::VolumeType::Local,
        );

        let service = AssetService::new(catalog_dir.path(), crate::Verbosity::quiet(), &crate::config::PreviewConfig::default());
        service.import(&[photos.clone()], &volume, &default_filter()).unwrap();

        // Manually set the JPG variant back to Original (simulating pre-fix import)
        let metadata_store = crate::metadata_store::MetadataStore::new(catalog_dir.path());
        let summaries = metadata_store.list().unwrap();
        let mut asset = metadata_store.load(summaries[0].id).unwrap();
        let jpg_idx = asset.variants.iter().position(|v| v.format == "jpg").unwrap();
        asset.variants[jpg_idx].role = VariantRole::Original;
        metadata_store.save(&asset).unwrap();
        let catalog = crate::catalog::Catalog::open(catalog_dir.path()).unwrap();
        catalog.update_variant_role(&asset.variants[jpg_idx].content_hash, "original").unwrap();

        // Run fix_roles with apply
        let result = service.fix_roles(&[], None, None, true, |_, _| {}).unwrap();
        assert_eq!(result.checked, 1);
        assert_eq!(result.fixed, 1);
        assert_eq!(result.variants_fixed, 1);
        assert_eq!(result.already_correct, 0);
        assert!(!result.dry_run);

        // Verify the JPG variant is now Export in sidecar
        let asset = metadata_store.load(summaries[0].id).unwrap();
        let jpg = asset.variants.iter().find(|v| v.format == "jpg").unwrap();
        assert_eq!(jpg.role, VariantRole::Export);
        // RAW should still be Original
        let raw = asset.variants.iter().find(|v| v.format == "nef").unwrap();
        assert_eq!(raw.role, VariantRole::Original);
    }

    #[test]
    fn fix_roles_skips_non_raw_groups() {
        let catalog_dir = tempfile::tempdir().unwrap();
        setup_catalog(catalog_dir.path());

        let vol_dir = tempfile::tempdir().unwrap();
        let photos = vol_dir.path().join("photos");
        std::fs::create_dir_all(&photos).unwrap();
        std::fs::write(photos.join("photo.jpg"), "jpeg content").unwrap();
        std::fs::write(photos.join("photo.png"), "png content").unwrap();

        let volume = crate::models::Volume::new(
            "test-vol".into(),
            vol_dir.path().to_path_buf(),
            crate::models::VolumeType::Local,
        );

        let service = AssetService::new(catalog_dir.path(), crate::Verbosity::quiet(), &crate::config::PreviewConfig::default());
        service.import(&[photos.clone()], &volume, &default_filter()).unwrap();

        // Both should be Original (no RAW)
        let metadata_store = crate::metadata_store::MetadataStore::new(catalog_dir.path());
        let summaries = metadata_store.list().unwrap();
        let asset = metadata_store.load(summaries[0].id).unwrap();
        assert!(asset.variants.iter().all(|v| v.role == VariantRole::Original));

        // Run fix_roles — should not change anything
        let result = service.fix_roles(&[], None, None, true, |_, _| {}).unwrap();
        assert_eq!(result.checked, 1);
        assert_eq!(result.fixed, 0);
        assert_eq!(result.variants_fixed, 0);
        assert_eq!(result.already_correct, 1);

        // Verify roles unchanged
        let asset = metadata_store.load(summaries[0].id).unwrap();
        assert!(asset.variants.iter().all(|v| v.role == VariantRole::Original));
    }

    #[test]
    fn fix_roles_dry_run_does_not_modify() {
        let catalog_dir = tempfile::tempdir().unwrap();
        setup_catalog(catalog_dir.path());

        let vol_dir = tempfile::tempdir().unwrap();
        let photos = vol_dir.path().join("photos");
        std::fs::create_dir_all(&photos).unwrap();
        std::fs::write(photos.join("DSC_002.nef"), "raw data here").unwrap();
        std::fs::write(photos.join("DSC_002.jpg"), "jpeg data here").unwrap();

        let volume = crate::models::Volume::new(
            "test-vol".into(),
            vol_dir.path().to_path_buf(),
            crate::models::VolumeType::Local,
        );

        let service = AssetService::new(catalog_dir.path(), crate::Verbosity::quiet(), &crate::config::PreviewConfig::default());
        service.import(&[photos.clone()], &volume, &default_filter()).unwrap();

        // Manually set the JPG variant back to Original
        let metadata_store = crate::metadata_store::MetadataStore::new(catalog_dir.path());
        let summaries = metadata_store.list().unwrap();
        let mut asset = metadata_store.load(summaries[0].id).unwrap();
        let jpg_idx = asset.variants.iter().position(|v| v.format == "jpg").unwrap();
        asset.variants[jpg_idx].role = VariantRole::Original;
        metadata_store.save(&asset).unwrap();
        let catalog = crate::catalog::Catalog::open(catalog_dir.path()).unwrap();
        catalog.update_variant_role(&asset.variants[jpg_idx].content_hash, "original").unwrap();

        // Run fix_roles with dry run (apply=false)
        let result = service.fix_roles(&[], None, None, false, |_, _| {}).unwrap();
        assert_eq!(result.fixed, 1);
        assert_eq!(result.variants_fixed, 1);
        assert!(result.dry_run);

        // Verify roles are NOT changed in sidecar
        let asset = metadata_store.load(summaries[0].id).unwrap();
        let jpg = asset.variants.iter().find(|v| v.format == "jpg").unwrap();
        assert_eq!(jpg.role, VariantRole::Original, "dry run should not modify sidecar");
    }

    #[test]
    fn import_uses_mtime_fallback_when_no_exif() {
        use chrono::Utc;

        let catalog_dir = tempfile::tempdir().unwrap();
        setup_catalog(catalog_dir.path());

        let vol_dir = tempfile::tempdir().unwrap();
        let photo_path = vol_dir.path().join("plain.jpg");
        // Write a file with no EXIF data
        std::fs::write(&photo_path, "no exif data here just plain bytes").unwrap();

        // Set a specific mtime in the past (2020-01-15)
        let target_time = std::time::SystemTime::UNIX_EPOCH
            + std::time::Duration::from_secs(1579046400); // 2020-01-15 00:00:00 UTC
        let file_times = std::fs::FileTimes::new()
            .set_modified(target_time);
        let file = std::fs::File::options().write(true).open(&photo_path).unwrap();
        file.set_times(file_times).unwrap();

        let volume = crate::models::Volume::new(
            "test-vol".into(),
            vol_dir.path().to_path_buf(),
            crate::models::VolumeType::Local,
        );

        let service = AssetService::new(catalog_dir.path(), crate::Verbosity::quiet(), &crate::config::PreviewConfig::default());
        let result = service.import(
            &[photo_path],
            &volume,
            &default_filter(),
        ).unwrap();
        assert_eq!(result.imported, 1);

        let metadata_store = crate::metadata_store::MetadataStore::new(catalog_dir.path());
        let summaries = metadata_store.list().unwrap();
        let asset = metadata_store.load(summaries[0].id).unwrap();

        // Should use mtime, not current time
        let now = Utc::now();
        let diff_from_now = (now - asset.created_at).num_hours();
        assert!(diff_from_now > 24, "created_at should be in 2020, not near now; got {}", asset.created_at);

        // Check it's close to our target mtime (2020-01-15)
        let expected = chrono::DateTime::<Utc>::from(target_time);
        let diff_from_target = (asset.created_at - expected).num_seconds().abs();
        assert!(diff_from_target <= 1, "created_at should match file mtime; got {} vs expected {}", asset.created_at, expected);
    }

    #[test]
    fn import_second_variant_updates_date_if_older() {
        use chrono::Utc;

        let catalog_dir = tempfile::tempdir().unwrap();
        setup_catalog(catalog_dir.path());

        let vol_dir = tempfile::tempdir().unwrap();

        // Create two files with the same stem (will be grouped)
        let raw_path = vol_dir.path().join("photo.arw");
        let jpg_path = vol_dir.path().join("photo.jpg");
        std::fs::write(&raw_path, "fake raw content for date test").unwrap();
        std::fs::write(&jpg_path, "fake jpg content for date test").unwrap();

        // Set RAW to newer mtime (2024), JPG to older mtime (2020)
        let newer_time = std::time::SystemTime::UNIX_EPOCH
            + std::time::Duration::from_secs(1704067200); // 2024-01-01
        let older_time = std::time::SystemTime::UNIX_EPOCH
            + std::time::Duration::from_secs(1579046400); // 2020-01-15

        let raw_file = std::fs::File::options().write(true).open(&raw_path).unwrap();
        raw_file.set_times(std::fs::FileTimes::new().set_modified(newer_time)).unwrap();
        let jpg_file = std::fs::File::options().write(true).open(&jpg_path).unwrap();
        jpg_file.set_times(std::fs::FileTimes::new().set_modified(older_time)).unwrap();

        let volume = crate::models::Volume::new(
            "test-vol".into(),
            vol_dir.path().to_path_buf(),
            crate::models::VolumeType::Local,
        );

        let service = AssetService::new(catalog_dir.path(), crate::Verbosity::quiet(), &crate::config::PreviewConfig::default());
        let result = service.import(
            &[vol_dir.path().to_path_buf()],
            &volume,
            &default_filter(),
        ).unwrap();
        assert_eq!(result.imported, 2); // RAW + JPG grouped into one asset

        let metadata_store = crate::metadata_store::MetadataStore::new(catalog_dir.path());
        let summaries = metadata_store.list().unwrap();
        assert_eq!(summaries.len(), 1, "should be one asset (grouped)");
        let asset = metadata_store.load(summaries[0].id).unwrap();

        // Asset date should be the older of the two mtimes (2020-01-15)
        let expected = chrono::DateTime::<Utc>::from(older_time);
        let diff = (asset.created_at - expected).num_seconds().abs();
        assert!(diff <= 1, "created_at should be the older variant date; got {} vs expected {}", asset.created_at, expected);
    }

    #[test]
    fn fix_dates_corrects_wrong_date() {
        use chrono::{DateTime, Utc, TimeZone};

        let catalog_dir = tempfile::tempdir().unwrap();
        setup_catalog(catalog_dir.path());
        DeviceRegistry::init(catalog_dir.path()).unwrap();

        let vol_dir = tempfile::tempdir().unwrap();
        let photo_path = vol_dir.path().join("photo.jpg");
        std::fs::write(&photo_path, "content for fix-dates test").unwrap();

        // Set mtime to 2020
        let old_time = std::time::SystemTime::UNIX_EPOCH
            + std::time::Duration::from_secs(1579046400); // 2020-01-15
        let file = std::fs::File::options().write(true).open(&photo_path).unwrap();
        file.set_times(std::fs::FileTimes::new().set_modified(old_time)).unwrap();

        // Register volume so fix_dates can find it
        let registry = DeviceRegistry::new(catalog_dir.path());
        let volume = registry.register("test-vol", vol_dir.path(), crate::models::VolumeType::Local, None).unwrap();

        let service = AssetService::new(catalog_dir.path(), crate::Verbosity::quiet(), &crate::config::PreviewConfig::default());
        service.import(
            &[photo_path],
            &volume,
            &default_filter(),
        ).unwrap();

        // Manually set asset date to "now" (simulating the old bug)
        let metadata_store = MetadataStore::new(catalog_dir.path());
        let summaries = metadata_store.list().unwrap();
        let mut asset = metadata_store.load(summaries[0].id).unwrap();
        let wrong_date = Utc.with_ymd_and_hms(2026, 2, 23, 12, 0, 0).unwrap();
        asset.created_at = wrong_date;
        metadata_store.save(&asset).unwrap();
        let catalog = crate::catalog::Catalog::open(catalog_dir.path()).unwrap();
        catalog.insert_asset(&asset).unwrap();

        // Run fix_dates in dry-run mode
        let result = service.fix_dates(None, None, false, |_, _, _| {}).unwrap();
        assert_eq!(result.fixed, 1);
        assert!(result.dry_run);

        // Verify date NOT changed (dry run)
        let asset = metadata_store.load(summaries[0].id).unwrap();
        let diff = (asset.created_at - wrong_date).num_seconds().abs();
        assert!(diff <= 1, "dry run should not change date");

        // Run fix_dates with apply
        let result = service.fix_dates(None, None, true, |_, _, _| {}).unwrap();
        assert_eq!(result.fixed, 1);
        assert!(!result.dry_run);

        // Verify date IS changed to the older mtime
        let asset = metadata_store.load(summaries[0].id).unwrap();
        let expected = DateTime::<Utc>::from(old_time);
        let diff = (asset.created_at - expected).num_seconds().abs();
        assert!(diff <= 1, "fix_dates should correct date to file mtime; got {} vs {}", asset.created_at, expected);
    }

    #[test]
    fn fix_dates_uses_source_metadata_date_taken() {
        use chrono::{Utc, TimeZone};

        let catalog_dir = tempfile::tempdir().unwrap();
        setup_catalog(catalog_dir.path());
        DeviceRegistry::init(catalog_dir.path()).unwrap();

        let vol_dir = tempfile::tempdir().unwrap();
        let photo_path = vol_dir.path().join("photo.jpg");
        std::fs::write(&photo_path, "content for date_taken metadata test").unwrap();

        let registry = DeviceRegistry::new(catalog_dir.path());
        let volume = registry.register("test-vol", vol_dir.path(), crate::models::VolumeType::Local, None).unwrap();

        let service = AssetService::new(catalog_dir.path(), crate::Verbosity::quiet(), &crate::config::PreviewConfig::default());
        service.import(&[photo_path], &volume, &default_filter()).unwrap();

        // Manually set a date_taken in source_metadata and a wrong created_at
        let metadata_store = MetadataStore::new(catalog_dir.path());
        let summaries = metadata_store.list().unwrap();
        let mut asset = metadata_store.load(summaries[0].id).unwrap();
        let exif_date = Utc.with_ymd_and_hms(2019, 6, 15, 10, 30, 0).unwrap();
        asset.variants[0].source_metadata.insert(
            "date_taken".to_string(),
            exif_date.to_rfc3339(),
        );
        let wrong_date = Utc.with_ymd_and_hms(2026, 2, 23, 12, 0, 0).unwrap();
        asset.created_at = wrong_date;
        metadata_store.save(&asset).unwrap();
        let catalog = crate::catalog::Catalog::open(catalog_dir.path()).unwrap();
        catalog.insert_asset(&asset).unwrap();

        // Run fix_dates with apply
        let result = service.fix_dates(None, None, true, |_, _, _| {}).unwrap();
        assert_eq!(result.fixed, 1);

        // Should pick the EXIF date (2019) which is older than file mtime
        let asset = metadata_store.load(summaries[0].id).unwrap();
        let diff = (asset.created_at - exif_date).num_seconds().abs();
        assert!(diff <= 1, "fix_dates should use source_metadata date_taken; got {} vs {}", asset.created_at, exif_date);
    }

    #[test]
    fn fix_dates_already_correct_no_change() {
        let catalog_dir = tempfile::tempdir().unwrap();
        setup_catalog(catalog_dir.path());
        DeviceRegistry::init(catalog_dir.path()).unwrap();

        let vol_dir = tempfile::tempdir().unwrap();
        let photo_path = vol_dir.path().join("photo.jpg");
        std::fs::write(&photo_path, "correct date test content").unwrap();

        let registry = DeviceRegistry::new(catalog_dir.path());
        let volume = registry.register("test-vol", vol_dir.path(), crate::models::VolumeType::Local, None).unwrap();

        let service = AssetService::new(catalog_dir.path(), crate::Verbosity::quiet(), &crate::config::PreviewConfig::default());
        service.import(&[photo_path], &volume, &default_filter()).unwrap();

        // The import now uses mtime fallback, so created_at should already be correct.
        // fix_dates should report it as already correct.
        let result = service.fix_dates(None, None, false, |_, _, _| {}).unwrap();
        assert_eq!(result.already_correct, 1);
        assert_eq!(result.fixed, 0);
    }

    #[test]
    fn merge_hierarchical_deduplicates_components() {
        let flat = vec![
            "animals".to_string(),
            "birds".to_string(),
            "eagles".to_string(),
            "sunset".to_string(),
        ];
        let hier = vec!["animals|birds|eagles".to_string()];
        let result = merge_hierarchical_keywords(&flat, &hier);
        assert_eq!(result, vec!["animals|birds|eagles", "sunset"]);
    }

    #[test]
    fn merge_hierarchical_empty_hierarchical() {
        let flat = vec!["landscape".to_string(), "sunset".to_string()];
        let hier: Vec<String> = vec![];
        let result = merge_hierarchical_keywords(&flat, &hier);
        assert_eq!(result, vec!["landscape", "sunset"]);
    }

    #[test]
    fn merge_hierarchical_multiple_hierarchies() {
        let flat = vec![
            "animals".to_string(),
            "birds".to_string(),
            "nature".to_string(),
            "sky".to_string(),
            "sunset".to_string(),
            "portrait".to_string(),
        ];
        let hier = vec![
            "animals|birds|eagles".to_string(),
            "nature|sky|sunset".to_string(),
        ];
        let result = merge_hierarchical_keywords(&flat, &hier);
        assert_eq!(
            result,
            vec![
                "animals|birds|eagles",
                "nature|sky|sunset",
                "portrait",
            ]
        );
    }

    #[test]
    fn merge_hierarchical_no_flat_overlap() {
        let flat = vec!["portrait".to_string(), "studio".to_string()];
        let hier = vec!["animals|birds".to_string()];
        let result = merge_hierarchical_keywords(&flat, &hier);
        assert_eq!(result, vec!["animals|birds", "portrait", "studio"]);
    }

    #[test]
    fn is_recently_verified_none_returns_false() {
        assert!(!is_recently_verified(None, 30));
    }

    #[test]
    fn is_recently_verified_recent_returns_true() {
        let now = chrono::Utc::now().to_rfc3339();
        assert!(is_recently_verified(Some(&now), 30));
    }

    #[test]
    fn is_recently_verified_old_returns_false() {
        let old = (chrono::Utc::now() - chrono::Duration::days(60)).to_rfc3339();
        assert!(!is_recently_verified(Some(&old), 30));
    }

    #[test]
    fn is_recently_verified_invalid_returns_false() {
        assert!(!is_recently_verified(Some("not-a-date"), 30));
    }

    #[test]
    fn resolve_flat_target_no_collision() {
        let dir = PathBuf::from("/tmp/export");
        let mut seen = std::collections::HashMap::new();
        let result = resolve_flat_target(&dir, "photo.jpg", "sha256:aabb", &mut seen);
        assert_eq!(result, dir.join("photo.jpg"));
    }

    #[test]
    fn resolve_flat_target_same_hash_reuses_name() {
        let dir = PathBuf::from("/tmp/export");
        let mut seen = std::collections::HashMap::new();
        let r1 = resolve_flat_target(&dir, "photo.jpg", "sha256:aabb", &mut seen);
        let r2 = resolve_flat_target(&dir, "photo.jpg", "sha256:aabb", &mut seen);
        assert_eq!(r1, r2);
    }

    #[test]
    fn resolve_flat_target_collision_adds_suffix() {
        let dir = PathBuf::from("/tmp/export");
        let mut seen = std::collections::HashMap::new();
        resolve_flat_target(&dir, "photo.jpg", "sha256:aaaa1111", &mut seen);
        let r2 = resolve_flat_target(&dir, "photo.jpg", "sha256:bbbb2222", &mut seen);
        assert_eq!(r2, dir.join("photo_bbbb2222.jpg"));
    }

    #[test]
    fn resolve_flat_target_no_extension() {
        let dir = PathBuf::from("/tmp/export");
        let mut seen = std::collections::HashMap::new();
        resolve_flat_target(&dir, "README", "sha256:aaaa", &mut seen);
        let r2 = resolve_flat_target(&dir, "README", "sha256:bbbb", &mut seen);
        assert_eq!(r2, dir.join("README_bbbb"));
    }

    #[test]
    fn resolve_flat_target_case_insensitive() {
        let dir = PathBuf::from("/tmp/export");
        let mut seen = std::collections::HashMap::new();
        resolve_flat_target(&dir, "Photo.JPG", "sha256:aaaa", &mut seen);
        let r2 = resolve_flat_target(&dir, "photo.jpg", "sha256:bbbb", &mut seen);
        // Different hash with same name (case-insensitive) should get suffix
        assert!(r2.file_name().unwrap().to_str().unwrap().contains("bbbb"));
    }

    #[test]
    fn normalize_rating_passthrough_1_to_5() {
        assert_eq!(normalize_rating(0), 0);
        assert_eq!(normalize_rating(1), 1);
        assert_eq!(normalize_rating(2), 2);
        assert_eq!(normalize_rating(3), 3);
        assert_eq!(normalize_rating(4), 4);
        assert_eq!(normalize_rating(5), 5);
    }

    #[test]
    fn normalize_rating_microsoft_percentage_scale() {
        // MicrosoftPhoto:Rating values: 1→1★, 25→2★, 50→3★, 75→4★, 99/100→5★
        assert_eq!(normalize_rating(1), 1); // edge: also valid as xmp:Rating
        assert_eq!(normalize_rating(20), 2);
        assert_eq!(normalize_rating(25), 2);
        assert_eq!(normalize_rating(40), 3);
        assert_eq!(normalize_rating(50), 3);
        assert_eq!(normalize_rating(60), 3);
        assert_eq!(normalize_rating(75), 4);
        assert_eq!(normalize_rating(80), 4);
        assert_eq!(normalize_rating(99), 5);
        assert_eq!(normalize_rating(100), 5);
        assert_eq!(normalize_rating(255), 5);
    }

    #[test]
    fn normalize_rating_boundary_values() {
        assert_eq!(normalize_rating(6), 1);   // just above xmp range
        assert_eq!(normalize_rating(12), 1);  // upper boundary of 1★
        assert_eq!(normalize_rating(13), 2);  // lower boundary of 2★
        assert_eq!(normalize_rating(37), 2);  // upper boundary of 2★
        assert_eq!(normalize_rating(38), 3);  // lower boundary of 3★
        assert_eq!(normalize_rating(62), 3);  // upper boundary of 3★
        assert_eq!(normalize_rating(63), 4);  // lower boundary of 4★
        assert_eq!(normalize_rating(87), 4);  // upper boundary of 4★
        assert_eq!(normalize_rating(88), 5);  // lower boundary of 5★
    }
}
