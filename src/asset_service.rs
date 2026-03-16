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
pub enum FileStatus {
    Imported,
    LocationAdded,
    Skipped,
    RecipeAttached,
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

    /// Import files: hash, deduplicate, create assets/variants, write sidecars, insert into DB.
    pub fn import(
        &self,
        paths: &[PathBuf],
        volume: &Volume,
        filter: &FileTypeFilter,
    ) -> Result<ImportResult> {
        self.import_with_callback(paths, volume, filter, &[], &[], false, false, |_, _, _| {})
    }

    /// Import files with a per-file callback reporting path, status, and elapsed time.
    /// With `dry_run`, reports what would happen without writing to catalog, sidecar, or disk.
    /// With `smart`, generates smart previews (2560px) alongside regular previews.
    pub fn import_with_callback(
        &self,
        paths: &[PathBuf],
        volume: &Volume,
        filter: &FileTypeFilter,
        exclude_patterns: &[String],
        auto_tags: &[String],
        dry_run: bool,
        smart: bool,
        on_file: impl Fn(&Path, FileStatus, Duration),
    ) -> Result<ImportResult> {
        let content_store = ContentStore::new(&self.catalog_root);
        let metadata_store = MetadataStore::new(&self.catalog_root);
        let catalog = Catalog::open(&self.catalog_root)?;
        let preview_gen = crate::preview::PreviewGenerator::new(&self.catalog_root, self.verbosity, &self.preview_config);

        if !dry_run {
            catalog.ensure_volume(volume)?;
        }

        let files = resolve_files(paths, exclude_patterns);
        let groups = group_by_stem(&files, filter);

        if self.verbosity.verbose {
            eprintln!("  Import: {} file(s) resolved, {} group(s)", files.len(), groups.len());
        }

        let mut imported = 0;
        let mut locations_added = 0;
        let mut skipped = 0;
        let mut recipes_attached = 0;
        let mut recipes_updated = 0;
        let mut previews_generated = 0;
        let mut smart_previews_generated = 0;
        let mut new_asset_ids = Vec::new();
        let mut imported_dirs: std::collections::HashSet<String> = std::collections::HashSet::new();

        for group in &groups {
            // Track the asset created/found for this group's primary variant
            let mut group_asset: Option<Asset> = None;
            let mut primary_variant_hash: Option<String> = None;

            // Pass 1: Process media files (RAW first due to sorting in group_by_stem)
            for file_path in &group.media_files {
                let file_start = Instant::now();

                // Track volume-relative directory for auto-group neighborhood
                if let Ok(rel) = file_path.strip_prefix(&volume.mount_point) {
                    if let Some(parent) = rel.parent() {
                        imported_dirs.insert(parent.to_string_lossy().to_string());
                    }
                }

                let content_hash = content_store
                    .ingest(file_path, volume)
                    .with_context(|| format!("Failed to hash {}", file_path.display()))?;

                if catalog.has_variant(&content_hash)? {
                    // Variant exists — check if we should add a new location
                    let relative_path = file_path
                        .strip_prefix(&volume.mount_point)
                        .with_context(|| {
                            format!(
                                "File {} is not under volume mount point {}",
                                file_path.display(),
                                volume.mount_point.display()
                            )
                        })?;

                    let location = FileLocation {
                        volume_id: volume.id,
                        relative_path: relative_path.to_path_buf(),
                        verified_at: None,
                    };

                    let asset_id = catalog
                        .find_asset_id_by_variant(&content_hash)?
                        .with_context(|| {
                            format!("Variant {} exists but no owning asset found", content_hash)
                        })?;
                    let asset_id: Uuid = asset_id.parse().with_context(|| {
                        format!("Invalid asset UUID: {}", asset_id)
                    })?;
                    let mut asset = metadata_store.load(asset_id)?;

                    // Find the variant and check if this exact location already exists
                    let variant = asset
                        .variants
                        .iter_mut()
                        .find(|v| v.content_hash == content_hash);
                    if let Some(variant) = variant {
                        let already_tracked = variant.locations.iter().any(|l| {
                            l.volume_id == location.volume_id
                                && l.relative_path == location.relative_path
                        });
                        if already_tracked {
                            // Even though skipped, use this as the group asset if needed
                            if group_asset.is_none() {
                                primary_variant_hash = Some(content_hash.clone());
                                group_asset = Some(asset);
                            }
                            skipped += 1;
                            on_file(file_path, FileStatus::Skipped, file_start.elapsed());
                            continue;
                        }
                        if !dry_run {
                            variant.locations.push(location.clone());
                            metadata_store.save(&asset)?;
                            catalog.insert_file_location(&content_hash, &location)?;
                        }
                        if group_asset.is_none() {
                            primary_variant_hash = Some(content_hash.clone());
                            group_asset = Some(asset);
                        }
                        locations_added += 1;
                        on_file(file_path, FileStatus::LocationAdded, file_start.elapsed());
                    } else {
                        if group_asset.is_none() {
                            primary_variant_hash = Some(content_hash.clone());
                            group_asset = Some(asset);
                        }
                        skipped += 1;
                        on_file(file_path, FileStatus::Skipped, file_start.elapsed());
                    }
                    continue;
                }

                // New variant
                let ext = file_path
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("");

                let filename = file_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("unknown")
                    .to_string();

                let file_size = std::fs::metadata(file_path)
                    .with_context(|| {
                        format!("Failed to read metadata for {}", file_path.display())
                    })?
                    .len();

                let relative_path = file_path
                    .strip_prefix(&volume.mount_point)
                    .with_context(|| {
                        format!(
                            "File {} is not under volume mount point {}",
                            file_path.display(),
                            volume.mount_point.display()
                        )
                    })?;

                let location = FileLocation {
                    volume_id: volume.id,
                    relative_path: relative_path.to_path_buf(),
                    verified_at: None,
                };

                if group_asset.is_none() {
                    // First new media file creates the asset
                    let asset_type = determine_asset_type(ext);
                    let exif_data = crate::exif_reader::extract(file_path);

                    let mut asset = Asset::new(asset_type, &content_hash);
                    // Date fallback chain: EXIF DateTimeOriginal → file mtime → Utc::now()
                    if let Some(date_taken) = exif_data.date_taken {
                        asset.created_at = date_taken;
                    } else if let Some(mtime) = file_mtime(file_path) {
                        asset.created_at = mtime;
                    }
                    asset.name = Some(group.stem.clone());

                    // Apply auto_tags (merge, no duplicates)
                    for tag in auto_tags {
                        if !asset.tags.contains(tag) {
                            asset.tags.push(tag.clone());
                        }
                    }

                    let variant = Variant {
                        content_hash: content_hash.clone(),
                        asset_id: asset.id,
                        role: VariantRole::Original,
                        format: ext.to_lowercase(),
                        file_size,
                        original_filename: filename,
                        source_metadata: exif_data.source_metadata,
                        locations: vec![location.clone()],
                    };

                    asset.variants.push(variant.clone());
                    primary_variant_hash = Some(content_hash.clone());

                    // Extract embedded XMP from JPEG/TIFF
                    let embedded_xmp = crate::embedded_xmp::extract_embedded_xmp(file_path);
                    if !embedded_xmp.keywords.is_empty()
                        || embedded_xmp.description.is_some()
                        || !embedded_xmp.source_metadata.is_empty()
                    {
                        apply_xmp_data(&embedded_xmp, &mut asset, &content_hash);
                    }

                    if !dry_run {
                        // Write sidecar + catalog immediately for first variant
                        metadata_store.save(&asset).with_context(|| {
                            format!("Failed to write sidecar for {}", file_path.display())
                        })?;
                        catalog.insert_asset(&asset)?;
                        catalog.insert_variant(&variant)?;
                        catalog.insert_file_location(&content_hash, &location)?;

                        // Generate preview for the newly imported variant
                        match preview_gen.generate(&content_hash, file_path, ext) {
                            Ok(Some(_)) => previews_generated += 1,
                            Ok(None) => {}
                            Err(e) => eprintln!("  Preview warning: {e:#}"),
                        }
                        if smart {
                            match preview_gen.generate_smart(&content_hash, file_path, ext) {
                                Ok(Some(_)) => smart_previews_generated += 1,
                                Ok(None) => {}
                                Err(e) => eprintln!("  Smart preview warning: {e:#}"),
                            }
                        }
                    }

                    new_asset_ids.push(asset.id.to_string());
                    group_asset = Some(asset);
                } else {
                    // Additional media file → add variant to existing group asset
                    let asset = group_asset.as_mut().unwrap();
                    let exif_data = crate::exif_reader::extract(file_path);

                    // If this variant has an older date, update the asset's created_at
                    let variant_date = exif_data.date_taken.or_else(|| file_mtime(file_path));
                    if let Some(vd) = variant_date {
                        if vd < asset.created_at {
                            asset.created_at = vd;
                        }
                    }

                    // If the primary variant is RAW and this file is not, it's an alternate
                    let primary_is_raw = asset.variants.first()
                        .map(|v| is_raw_extension(&v.format))
                        .unwrap_or(false);
                    let role = if primary_is_raw && !is_raw_extension(ext) {
                        VariantRole::Alternate
                    } else {
                        VariantRole::Original
                    };

                    let variant = Variant {
                        content_hash: content_hash.clone(),
                        asset_id: asset.id,
                        role,
                        format: ext.to_lowercase(),
                        file_size,
                        original_filename: filename,
                        source_metadata: exif_data.source_metadata,
                        locations: vec![location.clone()],
                    };

                    if !dry_run {
                        asset.variants.push(variant.clone());

                        // Extract embedded XMP from JPEG/TIFF
                        let embedded_xmp = crate::embedded_xmp::extract_embedded_xmp(file_path);
                        if !embedded_xmp.keywords.is_empty()
                            || embedded_xmp.description.is_some()
                            || !embedded_xmp.source_metadata.is_empty()
                        {
                            apply_xmp_data(&embedded_xmp, asset, &content_hash);
                        }

                        metadata_store.save(asset).with_context(|| {
                            format!("Failed to write sidecar for {}", file_path.display())
                        })?;
                        catalog.insert_asset(asset)?;
                        catalog.insert_variant(&variant)?;
                        catalog.insert_file_location(&content_hash, &location)?;

                        // Generate preview for the additional variant
                        match preview_gen.generate(&content_hash, file_path, ext) {
                            Ok(Some(_)) => previews_generated += 1,
                            Ok(None) => {}
                            Err(e) => eprintln!("  Preview warning: {e:#}"),
                        }
                        if smart {
                            match preview_gen.generate_smart(&content_hash, file_path, ext) {
                                Ok(Some(_)) => smart_previews_generated += 1,
                                Ok(None) => {}
                                Err(e) => eprintln!("  Smart preview warning: {e:#}"),
                            }
                        }
                    }
                }

                imported += 1;
                on_file(file_path, FileStatus::Imported, file_start.elapsed());
            }

            // Pass 2: Process recipe files
            for file_path in &group.recipe_files {
                let file_start = Instant::now();

                // If no media file was found for this group, treat recipe as standalone media
                if group_asset.is_none() {
                    let content_hash = content_store
                        .ingest(file_path, volume)
                        .with_context(|| format!("Failed to hash {}", file_path.display()))?;

                    if catalog.has_variant(&content_hash)? {
                        // Same dedup logic as media
                        let relative_path = file_path
                            .strip_prefix(&volume.mount_point)
                            .with_context(|| {
                                format!(
                                    "File {} is not under volume mount point {}",
                                    file_path.display(),
                                    volume.mount_point.display()
                                )
                            })?;
                        let location = FileLocation {
                            volume_id: volume.id,
                            relative_path: relative_path.to_path_buf(),
                            verified_at: None,
                        };
                        let asset_id = catalog
                            .find_asset_id_by_variant(&content_hash)?
                            .with_context(|| {
                                format!(
                                    "Variant {} exists but no owning asset found",
                                    content_hash
                                )
                            })?;
                        let asset_id: Uuid = asset_id.parse()?;
                        let mut asset = metadata_store.load(asset_id)?;
                        let variant = asset
                            .variants
                            .iter_mut()
                            .find(|v| v.content_hash == content_hash);
                        if let Some(variant) = variant {
                            let already_tracked = variant.locations.iter().any(|l| {
                                l.volume_id == location.volume_id
                                    && l.relative_path == location.relative_path
                            });
                            if already_tracked {
                                skipped += 1;
                                on_file(file_path, FileStatus::Skipped, file_start.elapsed());
                            } else {
                                if !dry_run {
                                    variant.locations.push(location.clone());
                                    metadata_store.save(&asset)?;
                                    catalog.insert_file_location(&content_hash, &location)?;
                                }
                                locations_added += 1;
                                on_file(
                                    file_path,
                                    FileStatus::LocationAdded,
                                    file_start.elapsed(),
                                );
                            }
                        } else {
                            skipped += 1;
                            on_file(file_path, FileStatus::Skipped, file_start.elapsed());
                        }
                        continue;
                    }

                    // Try to find a parent variant by stem + directory on this volume
                    let ext = file_path
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("");
                    let stem = file_path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("");
                    let relative_path = file_path
                        .strip_prefix(&volume.mount_point)
                        .with_context(|| {
                            format!(
                                "File {} is not under volume mount point {}",
                                file_path.display(),
                                volume.mount_point.display()
                            )
                        })?;
                    let dir_prefix = relative_path
                        .parent()
                        .unwrap_or_else(|| Path::new(""))
                        .to_string_lossy();

                    if let Some((parent_variant_hash, parent_asset_id)) =
                        catalog.find_variant_hash_by_stem_and_directory(
                            stem,
                            &dir_prefix,
                            &volume.id.to_string(),
                            None,
                        )?
                    {
                        // Found parent variant — attach recipe to it
                        let asset_uuid: Uuid = parent_asset_id.parse()?;
                        let mut asset = metadata_store.load(asset_uuid)?;

                        let location = FileLocation {
                            volume_id: volume.id,
                            relative_path: relative_path.to_path_buf(),
                            verified_at: None,
                        };

                        // Location-based dedup on the parent asset
                        let existing_recipe = asset.recipes.iter().find(|r| {
                            r.location.volume_id == volume.id
                                && r.location.relative_path == relative_path
                        });

                        if let Some(existing) = existing_recipe {
                            if existing.content_hash == content_hash {
                                skipped += 1;
                                on_file(file_path, FileStatus::Skipped, file_start.elapsed());
                            } else {
                                if !dry_run {
                                    let recipe_id = existing.id;
                                    let recipe_id_str = recipe_id.to_string();
                                    let recipe_mut = asset.recipes.iter_mut().find(|r| r.id == recipe_id).unwrap();
                                    recipe_mut.content_hash = content_hash.clone();
                                    catalog.update_recipe_content_hash(&recipe_id_str, &content_hash)?;
                                    if ext.eq_ignore_ascii_case("xmp") {
                                        let xmp = crate::xmp_reader::extract(file_path);
                                        reapply_xmp_data(&xmp, &mut asset, &parent_variant_hash);
                                        catalog.insert_asset(&asset)?;
                                        if let Some(v) = asset.variants.iter().find(|v| v.content_hash == parent_variant_hash) {
                                            catalog.insert_variant(v)?;
                                        }
                                    }
                                    metadata_store.save(&asset)?;
                                }
                                recipes_updated += 1;
                                on_file(file_path, FileStatus::RecipeUpdated, file_start.elapsed());
                            }
                        } else {
                            if !dry_run {
                                // Attach new recipe to parent
                                let recipe = Recipe {
                                    id: Uuid::new_v4(),
                                    variant_hash: parent_variant_hash.clone(),
                                    software: determine_recipe_software(ext).to_string(),
                                    recipe_type: RecipeType::Sidecar,
                                    content_hash: content_hash.clone(),
                                    location,
                                    pending_writeback: false,
                                };
                                asset.recipes.push(recipe.clone());
                                if ext.eq_ignore_ascii_case("xmp") {
                                    let xmp = crate::xmp_reader::extract(file_path);
                                    apply_xmp_data(&xmp, &mut asset, &parent_variant_hash);
                                    catalog.insert_asset(&asset)?;
                                    if let Some(v) = asset.variants.iter().find(|v| v.content_hash == parent_variant_hash) {
                                        catalog.insert_variant(v)?;
                                    }
                                }
                                metadata_store.save(&asset)?;
                                catalog.insert_recipe(&recipe)?;
                            }
                            recipes_attached += 1;
                            on_file(file_path, FileStatus::RecipeAttached, file_start.elapsed());
                        }
                        continue;
                    }

                    // No parent found — import as standalone asset
                    let filename = file_path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("unknown")
                        .to_string();
                    let file_size = std::fs::metadata(file_path)?.len();
                    let location = FileLocation {
                        volume_id: volume.id,
                        relative_path: relative_path.to_path_buf(),
                        verified_at: None,
                    };
                    let mut asset = Asset::new(AssetType::Other, &content_hash);
                    asset.name = Some(filename.clone());
                    for tag in auto_tags {
                        if !asset.tags.contains(tag) {
                            asset.tags.push(tag.clone());
                        }
                    }
                    let variant = Variant {
                        content_hash: content_hash.clone(),
                        asset_id: asset.id,
                        role: VariantRole::Original,
                        format: ext.to_lowercase(),
                        file_size,
                        original_filename: filename,
                        source_metadata: Default::default(),
                        locations: vec![location.clone()],
                    };
                    if !dry_run {
                        asset.variants.push(variant.clone());
                        metadata_store.save(&asset)?;
                        catalog.insert_asset(&asset)?;
                        catalog.insert_variant(&variant)?;
                        catalog.insert_file_location(&content_hash, &location)?;
                    }
                    new_asset_ids.push(asset.id.to_string());
                    imported += 1;
                    on_file(file_path, FileStatus::Imported, file_start.elapsed());
                    continue;
                }

                // Recipe file with a group asset: attach as Recipe
                let content_hash = content_store
                    .ingest(file_path, volume)
                    .with_context(|| format!("Failed to hash {}", file_path.display()))?;

                let ext = file_path
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("");

                let relative_path = file_path
                    .strip_prefix(&volume.mount_point)
                    .with_context(|| {
                        format!(
                            "File {} is not under volume mount point {}",
                            file_path.display(),
                            volume.mount_point.display()
                        )
                    })?;

                let location = FileLocation {
                    volume_id: volume.id,
                    relative_path: relative_path.to_path_buf(),
                    verified_at: None,
                };

                let variant_hash = primary_variant_hash
                    .as_ref()
                    .expect("primary_variant_hash should be set when group_asset is Some");

                let asset = group_asset.as_mut().unwrap();

                // Location-based recipe dedup: find existing recipe at same location
                let existing_recipe = asset.recipes.iter().find(|r| {
                    r.location.volume_id == volume.id
                        && r.location.relative_path == relative_path
                });

                if let Some(existing) = existing_recipe {
                    if existing.content_hash == content_hash {
                        // Same location, same hash — nothing changed
                        skipped += 1;
                        on_file(file_path, FileStatus::Skipped, file_start.elapsed());
                        continue;
                    }
                    // Same location, different hash — recipe was modified externally
                    if !dry_run {
                        let recipe_id = existing.id;
                        let recipe_id_str = recipe_id.to_string();

                        // Update in-memory
                        let recipe_mut = asset.recipes.iter_mut().find(|r| r.id == recipe_id).unwrap();
                        recipe_mut.content_hash = content_hash.clone();

                        // Update catalog
                        catalog.update_recipe_content_hash(&recipe_id_str, &content_hash)?;

                        // Re-extract XMP metadata if applicable
                        if ext.eq_ignore_ascii_case("xmp") {
                            let xmp = crate::xmp_reader::extract(file_path);
                            reapply_xmp_data(&xmp, asset, variant_hash);
                            catalog.insert_asset(asset)?;
                            if let Some(v) = asset.variants.iter().find(|v| v.content_hash == *variant_hash) {
                                catalog.insert_variant(v)?;
                            }
                        }

                        metadata_store.save(asset)?;
                    }
                    recipes_updated += 1;
                    on_file(file_path, FileStatus::RecipeUpdated, file_start.elapsed());
                    continue;
                }

                if !dry_run {
                    // No existing recipe at this location — attach new recipe
                    let recipe = Recipe {
                        id: Uuid::new_v4(),
                        variant_hash: variant_hash.clone(),
                        software: determine_recipe_software(ext).to_string(),
                        recipe_type: RecipeType::Sidecar,
                        content_hash,
                        location,
                        pending_writeback: false,
                    };

                    asset.recipes.push(recipe.clone());

                    // Extract metadata from XMP sidecars
                    if ext.eq_ignore_ascii_case("xmp") {
                        let xmp = crate::xmp_reader::extract(file_path);
                        apply_xmp_data(&xmp, asset, variant_hash);
                        catalog.insert_asset(asset)?;
                        if let Some(v) = asset.variants.iter().find(|v| v.content_hash == *variant_hash) {
                            catalog.insert_variant(v)?;
                        }
                    }

                    metadata_store.save(asset)?;
                    catalog.insert_recipe(&recipe)?;
                }

                recipes_attached += 1;
                on_file(file_path, FileStatus::RecipeAttached, file_start.elapsed());
            }
        }

        Ok(ImportResult {
            dry_run,
            imported,
            locations_added,
            skipped,
            recipes_attached,
            recipes_updated,
            previews_generated,
            smart_previews_generated,
            new_asset_ids,
            imported_directories: imported_dirs.into_iter().collect(),
        })
    }

    /// Relocate all files of an asset to a target volume.
    ///
    /// Copies variant files and recipe files, verifies integrity, updates metadata.
    /// With `remove_source`, deletes source files after successful copy.
    /// With `dry_run`, only reports what would happen.
    pub fn relocate(
        &self,
        asset_id: &str,
        target_volume_label: &str,
        remove_source: bool,
        dry_run: bool,
    ) -> Result<RelocateResult> {
        let catalog = Catalog::open(&self.catalog_root)?;
        let metadata_store = MetadataStore::new(&self.catalog_root);
        let content_store = ContentStore::new(&self.catalog_root);
        let registry = DeviceRegistry::new(&self.catalog_root);

        // Resolve asset
        let full_id = catalog
            .resolve_asset_id(asset_id)?
            .ok_or_else(|| anyhow::anyhow!("No asset found matching '{asset_id}'"))?;
        let asset_uuid: Uuid = full_id.parse()?;
        let asset = metadata_store.load(asset_uuid)?;

        // Resolve target volume
        let target_volume = registry.resolve_volume(target_volume_label)?;
        if !target_volume.mount_point.exists() {
            bail!("Target volume '{}' is offline (mount point {} not found)",
                target_volume.label, target_volume.mount_point.display());
        }

        // Get all volumes for resolving source paths
        let volumes = registry.list()?;
        let find_volume = |vol_id: Uuid| -> Option<Volume> {
            volumes.iter().find(|v| v.id == vol_id).cloned()
        };

        // Build copy plan
        let mut plan: Vec<FileCopyPlan> = Vec::new();

        // Plan variant file copies
        for variant in &asset.variants {
            for loc in &variant.locations {
                if loc.volume_id == target_volume.id {
                    continue; // Already on target
                }
                let source_vol = find_volume(loc.volume_id)
                    .ok_or_else(|| anyhow::anyhow!(
                        "Source volume {} not found in registry", loc.volume_id
                    ))?;
                if !source_vol.mount_point.exists() {
                    bail!("Source volume '{}' is offline (mount point {} not found)",
                        source_vol.label, source_vol.mount_point.display());
                }

                let source_path = source_vol.mount_point.join(&loc.relative_path);
                let target_path = target_volume.mount_point.join(&loc.relative_path);

                plan.push(FileCopyPlan {
                    content_hash: variant.content_hash.clone(),
                    source_path,
                    target_path,
                    kind: FileCopyKind::Variant,
                    source_volume_id: loc.volume_id,
                    source_relative_path: loc.relative_path.clone(),
                });
            }
        }

        // Plan recipe file copies
        for recipe in &asset.recipes {
            if recipe.location.volume_id == target_volume.id {
                continue; // Already on target
            }
            let source_vol = find_volume(recipe.location.volume_id)
                .ok_or_else(|| anyhow::anyhow!(
                    "Source volume {} not found in registry", recipe.location.volume_id
                ))?;
            if !source_vol.mount_point.exists() {
                bail!("Source volume '{}' is offline (mount point {} not found)",
                    source_vol.label, source_vol.mount_point.display());
            }

            let source_path = source_vol.mount_point.join(&recipe.location.relative_path);
            let target_path = target_volume.mount_point.join(&recipe.location.relative_path);

            plan.push(FileCopyPlan {
                content_hash: recipe.content_hash.clone(),
                source_path,
                target_path,
                kind: FileCopyKind::Recipe { recipe_id: recipe.id },
                source_volume_id: recipe.location.volume_id,
                source_relative_path: recipe.location.relative_path.clone(),
            });
        }

        // Early return if nothing to do
        if plan.is_empty() {
            return Ok(RelocateResult {
                copied: 0,
                skipped: 0,
                removed: 0,
                actions: vec!["All files already on target volume".to_string()],
            });
        }

        // Dry run: report what would happen
        if dry_run {
            let mut actions = Vec::new();
            let mut would_copy = 0usize;
            let mut would_skip = 0usize;

            for entry in &plan {
                if entry.target_path.exists() {
                    let existing_hash = content_store.hash_file(&entry.target_path)?;
                    if existing_hash == entry.content_hash {
                        actions.push(format!(
                            "SKIP {} (already exists with matching hash)",
                            entry.source_relative_path.display()
                        ));
                        would_skip += 1;
                        continue;
                    }
                }
                let verb = if remove_source { "MOVE" } else { "COPY" };
                actions.push(format!(
                    "{} {} -> {}",
                    verb,
                    entry.source_path.display(),
                    entry.target_path.display()
                ));
                would_copy += 1;
            }

            return Ok(RelocateResult {
                copied: would_copy,
                skipped: would_skip,
                removed: if remove_source { would_copy } else { 0 },
                actions,
            });
        }

        // Phase 1: Copy all files (no metadata changes yet)
        let mut copied = 0usize;
        let mut skipped = 0usize;
        let mut actions = Vec::new();

        for entry in &plan {
            if entry.target_path.exists() {
                let existing_hash = content_store.hash_file(&entry.target_path)?;
                if existing_hash == entry.content_hash {
                    actions.push(format!(
                        "Skipped {} (already exists on target)",
                        entry.source_relative_path.display()
                    ));
                    skipped += 1;
                    continue;
                }
            }
            content_store
                .copy_and_verify(&entry.source_path, &entry.target_path, &entry.content_hash)
                .with_context(|| format!(
                    "Failed to copy {} to {}",
                    entry.source_path.display(),
                    entry.target_path.display()
                ))?;
            actions.push(format!(
                "Copied {} -> {}",
                entry.source_relative_path.display(),
                target_volume.label
            ));
            copied += 1;
        }

        // Phase 2: Update metadata
        let mut asset = metadata_store.load(asset_uuid)?;
        catalog.ensure_volume(&target_volume)?;

        for entry in &plan {
            match &entry.kind {
                FileCopyKind::Variant => {
                    // Add new location to the variant
                    let new_loc = FileLocation {
                        volume_id: target_volume.id,
                        relative_path: entry.source_relative_path.clone(),
                        verified_at: None,
                    };
                    if let Some(variant) = asset.variants.iter_mut().find(|v| v.content_hash == entry.content_hash) {
                        let already_has = variant.locations.iter().any(|l| {
                            l.volume_id == target_volume.id
                                && l.relative_path == entry.source_relative_path
                        });
                        if !already_has {
                            variant.locations.push(new_loc.clone());
                            catalog.insert_file_location(&entry.content_hash, &new_loc)?;
                        }
                    }
                }
                FileCopyKind::Recipe { recipe_id } => {
                    // Update recipe location to target volume
                    if let Some(recipe) = asset.recipes.iter_mut().find(|r| r.id == *recipe_id) {
                        recipe.location.volume_id = target_volume.id;
                        recipe.location.relative_path = entry.source_relative_path.clone();
                    }
                    catalog.update_recipe_location(
                        &recipe_id.to_string(),
                        &target_volume.id.to_string(),
                        &entry.source_relative_path.to_string_lossy(),
                    )?;
                }
            }
        }

        metadata_store.save(&asset)?;

        // Phase 3: Remove sources (only if --remove-source)
        let mut removed = 0usize;
        if remove_source {
            for entry in &plan {
                // Delete source file
                if entry.source_path.exists() {
                    std::fs::remove_file(&entry.source_path)
                        .with_context(|| format!(
                            "Failed to remove source file {}",
                            entry.source_path.display()
                        ))?;
                }

                match &entry.kind {
                    FileCopyKind::Variant => {
                        // Remove old location from variant
                        if let Some(variant) = asset.variants.iter_mut().find(|v| v.content_hash == entry.content_hash) {
                            variant.locations.retain(|l| {
                                !(l.volume_id == entry.source_volume_id
                                    && l.relative_path == entry.source_relative_path)
                            });
                        }
                        catalog.delete_file_location(
                            &entry.content_hash,
                            &entry.source_volume_id.to_string(),
                            &entry.source_relative_path.to_string_lossy(),
                        )?;
                    }
                    FileCopyKind::Recipe { .. } => {
                        // Recipe location already updated to target in Phase 2
                    }
                }

                removed += 1;
            }

            // Save again after removals
            metadata_store.save(&asset)?;

            // Update action messages
            actions = actions
                .into_iter()
                .map(|a| a.replace("Copied", "Moved"))
                .collect();
        }

        Ok(RelocateResult {
            copied,
            skipped,
            removed,
            actions,
        })
    }

    /// Update a file's location in the catalog after it was moved on disk.
    ///
    /// Looks up the old path as a variant file location or recipe, verifies the
    /// file at `to_path` has the same content hash, and updates catalog + sidecar.
    pub fn update_location(
        &self,
        asset_id: &str,
        from_path: &str,
        to_path: &Path,
        volume_label: Option<&str>,
    ) -> Result<UpdateLocationResult> {
        let catalog = Catalog::open(&self.catalog_root)?;
        let metadata_store = MetadataStore::new(&self.catalog_root);
        let content_store = ContentStore::new(&self.catalog_root);
        let registry = DeviceRegistry::new(&self.catalog_root);

        // Resolve volume from --to path or explicit --volume
        let volume = if let Some(label) = volume_label {
            registry.resolve_volume(label)?
        } else {
            registry.find_volume_for_path(to_path)?
        };
        let volume_id_str = volume.id.to_string();

        // Convert to_path to volume-relative
        let new_relative = to_path
            .strip_prefix(&volume.mount_point)
            .with_context(|| {
                format!(
                    "Path '{}' is not under volume '{}' ({})",
                    to_path.display(),
                    volume.label,
                    volume.mount_point.display()
                )
            })?;
        let new_relative_str = new_relative.to_string_lossy().to_string();

        // Convert from_path to volume-relative (strip mount point if absolute)
        let from = Path::new(from_path);
        let old_relative_str = if from.is_absolute() {
            from.strip_prefix(&volume.mount_point)
                .with_context(|| {
                    format!(
                        "Path '{}' is not under volume '{}' ({})",
                        from_path,
                        volume.label,
                        volume.mount_point.display()
                    )
                })?
                .to_string_lossy()
                .to_string()
        } else {
            from_path.to_string()
        };

        // Resolve asset ID
        let full_id = catalog
            .resolve_asset_id(asset_id)?
            .ok_or_else(|| anyhow::anyhow!("No asset found matching '{asset_id}'"))?;

        // Try as variant file location first, then recipe
        if let Some((content_hash, _format)) =
            catalog.find_variant_by_volume_and_path(&volume_id_str, &old_relative_str)?
        {
            // Verify this variant belongs to the resolved asset
            let variant_asset_id = catalog.find_asset_id_by_variant(&content_hash)?;
            if variant_asset_id.as_deref() != Some(&full_id) {
                bail!(
                    "File at '{}' belongs to asset {}, not {}",
                    old_relative_str,
                    variant_asset_id.unwrap_or_else(|| "(unknown)".to_string()),
                    &full_id[..8]
                );
            }

            // Verify file exists at new path
            if !to_path.exists() {
                bail!("File not found at '{}'", to_path.display());
            }

            // Verify content hash matches
            let actual_hash = content_store.hash_file(to_path)?;
            if actual_hash != content_hash {
                bail!(
                    "Hash mismatch: file at '{}' has hash {} but catalog expects {}",
                    to_path.display(),
                    &actual_hash[..16],
                    &content_hash[..16]
                );
            }

            // Update catalog
            catalog.update_file_location_path(
                &content_hash,
                &volume_id_str,
                &old_relative_str,
                &new_relative_str,
            )?;

            // Update sidecar
            self.update_sidecar_file_location_path(
                &metadata_store,
                &catalog,
                &content_hash,
                volume.id,
                &old_relative_str,
                &new_relative_str,
            )?;

            Ok(UpdateLocationResult {
                asset_id: full_id,
                file_type: "variant".to_string(),
                content_hash,
                old_path: old_relative_str,
                new_path: new_relative_str,
                volume_label: volume.label,
            })
        } else if let Some((recipe_id, content_hash, variant_hash)) =
            catalog.find_recipe_by_volume_and_path(&volume_id_str, &old_relative_str)?
        {
            // Verify the recipe's variant belongs to the resolved asset
            let variant_asset_id = catalog.find_asset_id_by_variant(&variant_hash)?;
            if variant_asset_id.as_deref() != Some(&full_id) {
                bail!(
                    "Recipe at '{}' belongs to asset {}, not {}",
                    old_relative_str,
                    variant_asset_id.unwrap_or_else(|| "(unknown)".to_string()),
                    &full_id[..8]
                );
            }

            // Verify file exists at new path
            if !to_path.exists() {
                bail!("File not found at '{}'", to_path.display());
            }

            // Verify content hash matches
            let actual_hash = content_store.hash_file(to_path)?;
            if actual_hash != content_hash {
                bail!(
                    "Hash mismatch: file at '{}' has hash {} but catalog expects {}",
                    to_path.display(),
                    &actual_hash[..16],
                    &content_hash[..16]
                );
            }

            // Update catalog
            catalog.update_recipe_relative_path(&recipe_id, &new_relative_str)?;

            // Update sidecar
            self.update_sidecar_recipe_path(
                &metadata_store,
                &catalog,
                &variant_hash,
                volume.id,
                &old_relative_str,
                &new_relative_str,
            )?;

            Ok(UpdateLocationResult {
                asset_id: full_id,
                file_type: "recipe".to_string(),
                content_hash,
                old_path: old_relative_str,
                new_path: new_relative_str,
                volume_label: volume.label,
            })
        } else {
            bail!(
                "No variant or recipe found at '{}' on volume '{}'",
                old_relative_str,
                volume.label
            );
        }
    }

    /// Verify file integrity by re-hashing files and comparing against stored content hashes.
    ///
    /// Two modes:
    /// - **Path mode** (`paths` non-empty): verify specific files/directories on disk.
    /// - **Catalog mode** (`paths` empty): verify all known file locations, optionally
    ///   filtered by `volume_filter` or `asset_filter`.
    pub fn verify(
        &self,
        paths: &[PathBuf],
        volume_filter: Option<&str>,
        asset_filter: Option<&str>,
        filter: &FileTypeFilter,
        max_age_days: Option<u64>,
        on_file: impl Fn(&Path, VerifyStatus, Duration),
    ) -> Result<VerifyResult> {
        let content_store = ContentStore::new(&self.catalog_root);
        let metadata_store = MetadataStore::new(&self.catalog_root);
        let catalog = Catalog::open(&self.catalog_root)?;
        let registry = DeviceRegistry::new(&self.catalog_root);

        let mut result = VerifyResult {
            verified: 0,
            failed: 0,
            modified: 0,
            skipped: 0,
            skipped_recent: 0,
            errors: Vec::new(),
        };

        if !paths.is_empty() {
            // Path mode
            let files = resolve_files(paths, &[]);
            let volumes = registry.list()?;

            for file_path in &files {
                let file_start = std::time::Instant::now();

                // Skip files whose extension isn't in an enabled type group
                let ext = file_path
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("");
                if !ext.is_empty() && !filter.is_importable(ext) {
                    continue;
                }

                // Find which volume this file is on
                let volume = volumes.iter().find(|v| file_path.starts_with(&v.mount_point));
                let volume = match volume {
                    Some(v) => v,
                    None => {
                        result.skipped += 1;
                        result.errors.push(format!(
                            "No volume found for {}",
                            file_path.display()
                        ));
                        on_file(file_path, VerifyStatus::Skipped, file_start.elapsed());
                        continue;
                    }
                };

                let relative_path = file_path
                    .strip_prefix(&volume.mount_point)
                    .unwrap_or(file_path);

                // Skip recently verified files
                if let Some(days) = max_age_days {
                    let verified_at = catalog.get_location_verified_at(
                        &volume.id.to_string(),
                        &relative_path.to_string_lossy(),
                    )?;
                    if is_recently_verified(verified_at.as_deref(), days) {
                        result.skipped_recent += 1;
                        on_file(file_path, VerifyStatus::SkippedRecent, file_start.elapsed());
                        continue;
                    }
                }

                // Hash the file
                let hash = match content_store.hash_file(file_path) {
                    Ok(h) => h,
                    Err(e) => {
                        result.skipped += 1;
                        result.errors.push(format!(
                            "{}: {}",
                            file_path.display(),
                            e
                        ));
                        on_file(file_path, VerifyStatus::Missing, file_start.elapsed());
                        continue;
                    }
                };

                // Look up variant by hash
                match catalog.find_asset_id_by_variant(&hash)? {
                    Some(_) => {
                        // File matches a known variant — verified
                        result.verified += 1;
                        catalog.update_verified_at(
                            &hash,
                            &volume.id.to_string(),
                            &relative_path.to_string_lossy(),
                        )?;
                        // Also update sidecar verified_at
                        self.update_sidecar_verified_at(
                            &metadata_store,
                            &catalog,
                            &hash,
                            volume.id,
                            relative_path,
                        )?;
                        on_file(file_path, VerifyStatus::Ok, file_start.elapsed());
                    }
                    None => {
                        // Not a variant — check if it's a known recipe file by hash
                        if catalog.has_recipe_by_content_hash(&hash)? {
                            result.verified += 1;
                            catalog.update_recipe_verified_at(
                                &hash,
                                &volume.id.to_string(),
                                &relative_path.to_string_lossy(),
                            )?;
                            on_file(file_path, VerifyStatus::Ok, file_start.elapsed());
                        } else if let Some((recipe_id, _old_hash, variant_hash)) =
                            catalog.find_recipe_by_volume_and_path(
                                &volume.id.to_string(),
                                &relative_path.to_string_lossy(),
                            )?
                        {
                            // Recipe at this location has a different hash — modified
                            catalog.update_recipe_content_hash(&recipe_id, &hash)?;

                            // Update the sidecar via the variant's owning asset
                            if let Some(asset_id_str) = catalog.find_asset_id_by_variant(&variant_hash)? {
                                let asset_uuid: Uuid = asset_id_str.parse()?;
                                let mut asset = metadata_store.load(asset_uuid)?;
                                if let Some(recipe) = asset.recipes.iter_mut().find(|r| {
                                    r.location.volume_id == volume.id
                                        && r.location.relative_path == relative_path
                                }) {
                                    recipe.content_hash = hash.clone();

                                    let ext = relative_path.extension()
                                        .and_then(|e| e.to_str())
                                        .unwrap_or("");
                                    if ext.eq_ignore_ascii_case("xmp") {
                                        let xmp = crate::xmp_reader::extract(file_path);
                                        reapply_xmp_data(&xmp, &mut asset, &variant_hash);
                                        catalog.insert_asset(&asset)?;
                                        if let Some(v) = asset.variants.iter().find(|v| v.content_hash == variant_hash) {
                                            catalog.insert_variant(v)?;
                                        }
                                    }

                                    metadata_store.save(&asset)?;
                                }
                            }

                            result.modified += 1;
                            on_file(file_path, VerifyStatus::Modified, file_start.elapsed());
                        } else {
                            result.skipped += 1;
                            result.errors.push(format!(
                                "Untracked: {}",
                                file_path.display()
                            ));
                            on_file(file_path, VerifyStatus::Untracked, file_start.elapsed());
                        }
                    }
                }
            }
        } else {
            // Catalog mode
            let volume_filter_resolved = match volume_filter {
                Some(label) => Some(registry.resolve_volume(label)?),
                None => None,
            };

            let volumes = registry.list()?;

            let assets = if let Some(asset_id) = asset_filter {
                let full_id = catalog
                    .resolve_asset_id(asset_id)?
                    .ok_or_else(|| anyhow::anyhow!("No asset found matching '{asset_id}'"))?;
                let uuid: Uuid = full_id.parse()?;
                vec![metadata_store.load(uuid)?]
            } else {
                let summaries = metadata_store.list()?;
                summaries
                    .iter()
                    .map(|s| metadata_store.load(s.id))
                    .collect::<Result<Vec<_>>>()?
            };

            for asset in &assets {
                // Verify variant file locations
                for variant in &asset.variants {
                    for loc in &variant.locations {
                        self.verify_location(
                            &content_store,
                            &catalog,
                            &metadata_store,
                            &volumes,
                            volume_filter_resolved.as_ref(),
                            &variant.content_hash,
                            loc,
                            None,
                            max_age_days,
                            &mut result,
                            &on_file,
                        )?;
                    }
                }

                // Verify recipe file locations
                for recipe in &asset.recipes {
                    self.verify_location(
                        &content_store,
                        &catalog,
                        &metadata_store,
                        &volumes,
                        volume_filter_resolved.as_ref(),
                        &recipe.content_hash,
                        &recipe.location,
                        Some(&recipe.variant_hash),
                        max_age_days,
                        &mut result,
                        &on_file,
                    )?;
                }
            }
        }

        Ok(result)
    }

    /// Verify a single file location (used by catalog mode).
    #[allow(clippy::too_many_arguments)]
    fn verify_location(
        &self,
        content_store: &ContentStore,
        catalog: &Catalog,
        metadata_store: &MetadataStore,
        volumes: &[Volume],
        volume_filter: Option<&Volume>,
        content_hash: &str,
        loc: &FileLocation,
        recipe_variant_hash: Option<&str>,
        max_age_days: Option<u64>,
        result: &mut VerifyResult,
        on_file: &impl Fn(&Path, VerifyStatus, Duration),
    ) -> Result<()> {
        let file_start = std::time::Instant::now();

        // Apply volume filter
        if let Some(filter_vol) = volume_filter {
            if loc.volume_id != filter_vol.id {
                return Ok(());
            }
        }

        // Skip recently verified files
        if let Some(days) = max_age_days {
            if let Some(ref verified_at) = loc.verified_at {
                let age = chrono::Utc::now() - *verified_at;
                if age.num_days() < days as i64 {
                    result.skipped_recent += 1;
                    on_file(&loc.relative_path, VerifyStatus::SkippedRecent, file_start.elapsed());
                    return Ok(());
                }
            }
        }

        // Find the volume
        let volume = match volumes.iter().find(|v| v.id == loc.volume_id) {
            Some(v) => v,
            None => {
                result.skipped += 1;
                result.errors.push(format!(
                    "Volume {} not found for {}",
                    loc.volume_id,
                    loc.relative_path.display()
                ));
                on_file(&loc.relative_path, VerifyStatus::Skipped, file_start.elapsed());
                return Ok(());
            }
        };

        // Skip offline volumes
        if !volume.is_online {
            result.skipped += 1;
            on_file(&loc.relative_path, VerifyStatus::Skipped, file_start.elapsed());
            return Ok(());
        }

        let full_path = volume.mount_point.join(&loc.relative_path);

        if !full_path.exists() {
            result.skipped += 1;
            result.errors.push(format!(
                "Missing: {} ({}:{})",
                full_path.display(),
                volume.label,
                loc.relative_path.display()
            ));
            on_file(&full_path, VerifyStatus::Missing, file_start.elapsed());
            return Ok(());
        }

        match content_store.verify(content_hash, &full_path) {
            Ok(true) => {
                result.verified += 1;
                if let Some(variant_hash) = recipe_variant_hash {
                    catalog.update_recipe_verified_at(
                        variant_hash,
                        &volume.id.to_string(),
                        &loc.relative_path.to_string_lossy(),
                    )?;
                    self.update_sidecar_recipe_verified_at(
                        metadata_store,
                        catalog,
                        variant_hash,
                        loc.volume_id,
                        &loc.relative_path,
                    )?;
                } else {
                    catalog.update_verified_at(
                        content_hash,
                        &volume.id.to_string(),
                        &loc.relative_path.to_string_lossy(),
                    )?;
                    self.update_sidecar_verified_at(
                        metadata_store,
                        catalog,
                        content_hash,
                        volume.id,
                        &loc.relative_path,
                    )?;
                }
                on_file(&full_path, VerifyStatus::Ok, file_start.elapsed());
            }
            Ok(false) => {
                if let Some(variant_hash) = recipe_variant_hash {
                    // Recipe files are expected to change — report as modified, not failed
                    let new_hash = content_store.hash_file(&full_path)?;

                    // Update the recipe's stored hash in the catalog
                    if let Some((recipe_id, _, _)) = catalog.find_recipe_by_volume_and_path(
                        &volume.id.to_string(),
                        &loc.relative_path.to_string_lossy(),
                    )? {
                        catalog.update_recipe_content_hash(&recipe_id, &new_hash)?;
                    }

                    // Update the sidecar file
                    if let Some(asset_id) = catalog.find_asset_id_by_variant(variant_hash)? {
                        let uuid: Uuid = asset_id.parse()?;
                        let mut asset = metadata_store.load(uuid)?;
                        if let Some(recipe) = asset.recipes.iter_mut().find(|r| {
                            r.location.volume_id == loc.volume_id
                                && r.location.relative_path == loc.relative_path
                        }) {
                            recipe.content_hash = new_hash.clone();

                            // Re-extract XMP data if applicable
                            let ext = loc.relative_path.extension()
                                .and_then(|e| e.to_str())
                                .unwrap_or("");
                            if ext.eq_ignore_ascii_case("xmp") {
                                let xmp = crate::xmp_reader::extract(&full_path);
                                reapply_xmp_data(&xmp, &mut asset, variant_hash);
                                catalog.insert_asset(&asset)?;
                                if let Some(v) = asset.variants.iter().find(|v| v.content_hash == variant_hash) {
                                    catalog.insert_variant(v)?;
                                }
                            }

                            metadata_store.save(&asset)?;
                        }
                    }

                    result.modified += 1;
                    on_file(&full_path, VerifyStatus::Modified, file_start.elapsed());
                } else {
                    result.failed += 1;
                    result.errors.push(format!(
                        "FAILED: {} ({}:{})",
                        full_path.display(),
                        volume.label,
                        loc.relative_path.display()
                    ));
                    on_file(&full_path, VerifyStatus::Mismatch, file_start.elapsed());
                }
            }
            Err(e) => {
                result.skipped += 1;
                result.errors.push(format!(
                    "Error reading {}: {}",
                    full_path.display(),
                    e
                ));
                on_file(&full_path, VerifyStatus::Missing, file_start.elapsed());
            }
        }

        Ok(())
    }

    /// Update the `verified_at` timestamp in the sidecar YAML for a specific file location.
    fn update_sidecar_verified_at(
        &self,
        metadata_store: &MetadataStore,
        catalog: &Catalog,
        content_hash: &str,
        volume_id: Uuid,
        relative_path: &Path,
    ) -> Result<()> {
        let asset_id = match catalog.find_asset_id_by_variant(content_hash)? {
            Some(id) => id,
            None => return Ok(()),
        };
        let uuid: Uuid = asset_id.parse()?;
        let mut asset = metadata_store.load(uuid)?;
        let now = chrono::Utc::now();

        let mut changed = false;
        for variant in &mut asset.variants {
            if variant.content_hash == content_hash {
                for loc in &mut variant.locations {
                    if loc.volume_id == volume_id && loc.relative_path == relative_path {
                        loc.verified_at = Some(now);
                        changed = true;
                    }
                }
            }
        }

        if changed {
            metadata_store.save(&asset)?;
        }

        Ok(())
    }

    /// Update the sidecar YAML with a recipe's verified_at timestamp.
    fn update_sidecar_recipe_verified_at(
        &self,
        metadata_store: &MetadataStore,
        catalog: &Catalog,
        variant_hash: &str,
        volume_id: Uuid,
        relative_path: &Path,
    ) -> Result<()> {
        let asset_id = match catalog.find_asset_id_by_variant(variant_hash)? {
            Some(id) => id,
            None => return Ok(()),
        };
        let uuid: Uuid = asset_id.parse()?;
        let mut asset = metadata_store.load(uuid)?;
        let now = chrono::Utc::now();

        let mut changed = false;
        for recipe in &mut asset.recipes {
            if recipe.location.volume_id == volume_id
                && recipe.location.relative_path == relative_path
            {
                recipe.location.verified_at = Some(now);
                changed = true;
            }
        }

        if changed {
            metadata_store.save(&asset)?;
        }

        Ok(())
    }

    /// Scan directories and reconcile the catalog with disk reality.
    ///
    /// Detects moved files, new files, modified recipes, and missing files.
    /// Without `apply`, runs in report-only mode. With `apply`, updates the catalog
    /// and sidecar files. `remove_stale` (requires `apply`) removes catalog locations
    /// for confirmed-missing files.
    pub fn sync(
        &self,
        paths: &[PathBuf],
        volume: &Volume,
        apply: bool,
        remove_stale: bool,
        exclude_patterns: &[String],
        on_file: impl Fn(&Path, SyncStatus, Duration),
    ) -> Result<SyncResult> {
        use std::collections::{HashMap, HashSet};

        let content_store = ContentStore::new(&self.catalog_root);
        let metadata_store = MetadataStore::new(&self.catalog_root);
        let catalog = Catalog::open(&self.catalog_root)?;
        let filter = FileTypeFilter::default();

        let mut result = SyncResult {
            unchanged: 0,
            moved: 0,
            new_files: 0,
            modified: 0,
            missing: 0,
            stale_removed: 0,
            errors: Vec::new(),
        };

        let vol_id = volume.id.to_string();

        // Collect all files on disk
        let files = resolve_files(paths, exclude_patterns);

        // Track paths seen on disk (relative to volume mount)
        let mut disk_media_paths: HashSet<String> = HashSet::new();
        let mut disk_recipe_paths: HashSet<String> = HashSet::new();

        // Maps for move detection: content_hash -> new_relative_path
        let mut media_hash_to_new_path: HashMap<String, (String, PathBuf)> = HashMap::new();
        // recipe: content_hash -> (new_relative_path, full_path)
        let mut recipe_hash_to_new_path: HashMap<String, (String, PathBuf)> = HashMap::new();

        // ── Pass 1: Scan disk files ──────────────────────────────────
        for file_path in &files {
            let file_start = Instant::now();

            let ext = file_path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("");

            // Skip files not in any known type group
            if !ext.is_empty() && !filter.is_importable(ext) {
                continue;
            }

            let relative_path = match file_path.strip_prefix(&volume.mount_point) {
                Ok(rp) => rp.to_string_lossy().to_string(),
                Err(_) => {
                    result.errors.push(format!(
                        "File {} is not under volume mount point {}",
                        file_path.display(),
                        volume.mount_point.display()
                    ));
                    continue;
                }
            };

            let hash = match content_store.hash_file(file_path) {
                Ok(h) => h,
                Err(e) => {
                    result.errors.push(format!("{}: {}", file_path.display(), e));
                    continue;
                }
            };

            let is_recipe = filter.is_recipe(ext);

            if is_recipe {
                disk_recipe_paths.insert(relative_path.clone());

                // Look up recipe by location
                match catalog.find_recipe_by_volume_and_path(&vol_id, &relative_path)? {
                    Some((_recipe_id, stored_hash, _variant_hash)) => {
                        if stored_hash == hash {
                            // Unchanged recipe
                            result.unchanged += 1;
                            on_file(file_path, SyncStatus::Unchanged, file_start.elapsed());
                        } else {
                            // Modified recipe (content changed at same path)
                            result.modified += 1;
                            if apply {
                                self.apply_modified_recipe(
                                    &catalog,
                                    &metadata_store,
                                    &_recipe_id,
                                    &hash,
                                    &_variant_hash,
                                    volume,
                                    file_path,
                                    &relative_path,
                                )?;
                            }
                            on_file(file_path, SyncStatus::Modified, file_start.elapsed());
                        }
                    }
                    None => {
                        // Not at this location — could be moved or new
                        if catalog.has_recipe_by_content_hash(&hash)? {
                            // Known hash at different location → potential move
                            recipe_hash_to_new_path.insert(
                                hash,
                                (relative_path, file_path.clone()),
                            );
                        } else {
                            // Completely new recipe file
                            result.new_files += 1;
                            on_file(file_path, SyncStatus::New, file_start.elapsed());
                        }
                    }
                }
            } else {
                disk_media_paths.insert(relative_path.clone());

                // Look up media file by location
                match catalog.find_variant_by_volume_and_path(&vol_id, &relative_path)? {
                    Some((stored_hash, _format)) => {
                        if stored_hash == hash {
                            // Unchanged — optionally update verified_at
                            result.unchanged += 1;
                            if apply {
                                catalog.update_verified_at(&hash, &vol_id, &relative_path)?;
                            }
                            on_file(file_path, SyncStatus::Unchanged, file_start.elapsed());
                        } else {
                            // Content-addressed file changed — this shouldn't happen
                            result.errors.push(format!(
                                "Hash mismatch at {}: expected {}, got {}",
                                relative_path, stored_hash, hash
                            ));
                        }
                    }
                    None => {
                        // Not at this location — could be moved or new
                        if catalog.has_variant(&hash)? {
                            // Known hash at different location → potential move
                            media_hash_to_new_path.insert(
                                hash,
                                (relative_path, file_path.clone()),
                            );
                        } else {
                            // Completely new file
                            result.new_files += 1;
                            on_file(file_path, SyncStatus::New, file_start.elapsed());
                        }
                    }
                }
            }
        }

        // ── Pass 2: Detect missing/moved ─────────────────────────────
        // Compute directory prefixes from scanned paths
        let prefixes = compute_prefixes(paths, &volume.mount_point);

        // Check media file locations
        for prefix in &prefixes {
            let catalog_locations =
                catalog.list_locations_for_volume_under_prefix(&vol_id, prefix)?;

            for (content_hash, cat_path) in &catalog_locations {
                if disk_media_paths.contains(cat_path.as_str()) {
                    continue; // Already handled in Pass 1
                }

                let file_start = Instant::now();

                if let Some((new_path, full_path)) = media_hash_to_new_path.remove(content_hash) {
                    // File was moved
                    result.moved += 1;
                    if apply {
                        catalog.update_file_location_path(
                            content_hash,
                            &vol_id,
                            cat_path,
                            &new_path,
                        )?;
                        // Update sidecar
                        self.update_sidecar_file_location_path(
                            &metadata_store,
                            &catalog,
                            content_hash,
                            volume.id,
                            cat_path,
                            &new_path,
                        )?;
                    }
                    on_file(&full_path, SyncStatus::Moved, file_start.elapsed());
                } else {
                    // File is missing from disk
                    result.missing += 1;
                    let full_path = volume.mount_point.join(cat_path);
                    if apply && remove_stale {
                        catalog.delete_file_location(content_hash, &vol_id, cat_path)?;
                        self.remove_sidecar_file_location(
                            &metadata_store,
                            &catalog,
                            content_hash,
                            volume.id,
                            cat_path,
                        )?;
                        result.stale_removed += 1;
                    }
                    on_file(&full_path, SyncStatus::Missing, file_start.elapsed());
                }
            }
        }

        // Check recipe locations
        for prefix in &prefixes {
            let catalog_recipes =
                catalog.list_recipes_for_volume_under_prefix(&vol_id, prefix)?;

            for (recipe_id, content_hash, variant_hash, cat_path) in &catalog_recipes {
                if disk_recipe_paths.contains(cat_path.as_str()) {
                    continue; // Already handled in Pass 1
                }

                let file_start = Instant::now();

                if let Some((new_path, full_path)) = recipe_hash_to_new_path.remove(&*content_hash) {
                    // Recipe was moved
                    result.moved += 1;
                    if apply {
                        catalog.update_recipe_relative_path(recipe_id, &new_path)?;
                        // Update sidecar
                        self.update_sidecar_recipe_path(
                            &metadata_store,
                            &catalog,
                            variant_hash,
                            volume.id,
                            cat_path,
                            &new_path,
                        )?;
                    }
                    on_file(&full_path, SyncStatus::Moved, file_start.elapsed());
                } else {
                    // Recipe is missing from disk
                    result.missing += 1;
                    let full_path = volume.mount_point.join(cat_path);
                    on_file(&full_path, SyncStatus::Missing, file_start.elapsed());
                }
            }
        }

        // Any remaining entries in hash_to_new_path are files that matched a hash
        // but whose old location wasn't in our scanned prefixes — report as new
        for (_hash, (_path, full_path)) in &media_hash_to_new_path {
            let file_start = Instant::now();
            result.new_files += 1;
            on_file(full_path, SyncStatus::New, file_start.elapsed());
        }
        for (_hash, (_path, full_path)) in &recipe_hash_to_new_path {
            let file_start = Instant::now();
            result.new_files += 1;
            on_file(full_path, SyncStatus::New, file_start.elapsed());
        }

        Ok(result)
    }

    /// Apply a modified recipe: update catalog hash, re-extract XMP if applicable, update sidecar.
    fn apply_modified_recipe(
        &self,
        catalog: &Catalog,
        metadata_store: &MetadataStore,
        recipe_id: &str,
        new_hash: &str,
        variant_hash: &str,
        volume: &Volume,
        file_path: &Path,
        relative_path: &str,
    ) -> Result<()> {
        catalog.update_recipe_content_hash(recipe_id, new_hash)?;

        if let Some(asset_id_str) = catalog.find_asset_id_by_variant(variant_hash)? {
            let asset_uuid: Uuid = asset_id_str.parse()?;
            let mut asset = metadata_store.load(asset_uuid)?;
            if let Some(recipe) = asset.recipes.iter_mut().find(|r| {
                r.location.volume_id == volume.id
                    && r.location.relative_path.to_string_lossy() == relative_path
            }) {
                recipe.content_hash = new_hash.to_string();

                let ext = file_path
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("");
                if ext.eq_ignore_ascii_case("xmp") {
                    let xmp = crate::xmp_reader::extract(file_path);
                    reapply_xmp_data(&xmp, &mut asset, variant_hash);
                    catalog.insert_asset(&asset)?;
                    if let Some(v) = asset.variants.iter().find(|v| v.content_hash == variant_hash)
                    {
                        catalog.insert_variant(v)?;
                    }
                }

                metadata_store.save(&asset)?;
            }
        }
        Ok(())
    }

    /// Update a file location's relative_path in the sidecar YAML.
    fn update_sidecar_file_location_path(
        &self,
        metadata_store: &MetadataStore,
        catalog: &Catalog,
        content_hash: &str,
        volume_id: Uuid,
        old_path: &str,
        new_path: &str,
    ) -> Result<()> {
        let asset_id = match catalog.find_asset_id_by_variant(content_hash)? {
            Some(id) => id,
            None => return Ok(()),
        };
        let uuid: Uuid = asset_id.parse()?;
        let mut asset = metadata_store.load(uuid)?;

        let mut changed = false;
        for variant in &mut asset.variants {
            if variant.content_hash == content_hash {
                for loc in &mut variant.locations {
                    if loc.volume_id == volume_id
                        && loc.relative_path.to_string_lossy() == old_path
                    {
                        loc.relative_path = PathBuf::from(new_path);
                        changed = true;
                    }
                }
            }
        }

        if changed {
            metadata_store.save(&asset)?;
        }
        Ok(())
    }

    /// Remove a file location from the sidecar YAML.
    pub fn remove_sidecar_file_location(
        &self,
        metadata_store: &MetadataStore,
        catalog: &Catalog,
        content_hash: &str,
        volume_id: Uuid,
        relative_path: &str,
    ) -> Result<()> {
        let asset_id = match catalog.find_asset_id_by_variant(content_hash)? {
            Some(id) => id,
            None => return Ok(()),
        };
        let uuid: Uuid = asset_id.parse()?;
        let mut asset = metadata_store.load(uuid)?;

        let mut changed = false;
        for variant in &mut asset.variants {
            if variant.content_hash == content_hash {
                let before = variant.locations.len();
                variant.locations.retain(|loc| {
                    !(loc.volume_id == volume_id
                        && loc.relative_path.to_string_lossy() == relative_path)
                });
                if variant.locations.len() != before {
                    changed = true;
                }
            }
        }

        if changed {
            metadata_store.save(&asset)?;
        }
        Ok(())
    }

    /// Update a recipe's relative_path in the sidecar YAML.
    fn update_sidecar_recipe_path(
        &self,
        metadata_store: &MetadataStore,
        catalog: &Catalog,
        variant_hash: &str,
        volume_id: Uuid,
        old_path: &str,
        new_path: &str,
    ) -> Result<()> {
        let asset_id = match catalog.find_asset_id_by_variant(variant_hash)? {
            Some(id) => id,
            None => return Ok(()),
        };
        let uuid: Uuid = asset_id.parse()?;
        let mut asset = metadata_store.load(uuid)?;

        let mut changed = false;
        for recipe in &mut asset.recipes {
            if recipe.location.volume_id == volume_id
                && recipe.location.relative_path.to_string_lossy() == old_path
            {
                recipe.location.relative_path = PathBuf::from(new_path);
                changed = true;
            }
        }

        if changed {
            metadata_store.save(&asset)?;
        }
        Ok(())
    }

    /// Remove a recipe from the sidecar YAML by matching volume_id + relative_path.
    pub fn remove_sidecar_recipe(
        &self,
        metadata_store: &MetadataStore,
        catalog: &Catalog,
        variant_hash: &str,
        volume_id: Uuid,
        relative_path: &str,
    ) -> Result<()> {
        let asset_id = match catalog.find_asset_id_by_variant(variant_hash)? {
            Some(id) => id,
            None => return Ok(()),
        };
        let uuid: Uuid = asset_id.parse()?;
        let mut asset = metadata_store.load(uuid)?;

        let before = asset.recipes.len();
        asset.recipes.retain(|r| {
            !(r.location.volume_id == volume_id
                && r.location.relative_path.to_string_lossy() == relative_path)
        });

        if asset.recipes.len() != before {
            metadata_store.save(&asset)?;
        }
        Ok(())
    }

    /// Scan all file locations and recipes across online volumes, checking for files
    /// that no longer exist on disk. Optionally remove stale records.
    ///
    /// Also scans for orphaned derived files (previews, smart previews, embeddings,
    /// face crops) and removes them.
    pub fn cleanup(
        &self,
        volume_filter: Option<&str>,
        path_prefix: Option<&str>,
        apply: bool,
        on_file: impl Fn(&Path, CleanupStatus, Duration),
    ) -> Result<CleanupResult> {
        let registry = DeviceRegistry::new(&self.catalog_root);
        let catalog = Catalog::open(&self.catalog_root)?;
        let metadata_store = MetadataStore::new(&self.catalog_root);

        let mut result = CleanupResult {
            checked: 0,
            stale: 0,
            removed: 0,
            skipped_offline: 0,
            locationless_variants: 0,
            removed_variants: 0,
            orphaned_assets: 0,
            removed_assets: 0,
            orphaned_previews: 0,
            removed_previews: 0,
            orphaned_smart_previews: 0,
            removed_smart_previews: 0,
            orphaned_embeddings: 0,
            removed_embeddings: 0,
            orphaned_face_files: 0,
            removed_face_files: 0,
            errors: Vec::new(),
        };

        let volumes = if let Some(label) = volume_filter {
            vec![registry.resolve_volume(label)?]
        } else {
            registry.list()?
        };

        // Collect stale locations for report-mode orphan prediction
        let mut stale_locations: Vec<(String, String, String)> = Vec::new();

        for volume in &volumes {
            if !volume.is_online {
                result.skipped_offline += 1;
                on_file(&volume.mount_point, CleanupStatus::Offline, Duration::ZERO);
                continue;
            }

            let vol_id_str = volume.id.to_string();

            // Check variant file locations
            let prefix = path_prefix.unwrap_or("");
            let locations = catalog.list_locations_for_volume_under_prefix(&vol_id_str, prefix)?;
            for (content_hash, relative_path) in &locations {
                let file_start = Instant::now();
                let full_path = volume.mount_point.join(relative_path);

                if full_path.exists() {
                    result.checked += 1;
                    on_file(&full_path, CleanupStatus::Ok, file_start.elapsed());
                } else {
                    result.stale += 1;
                    on_file(&full_path, CleanupStatus::Stale, file_start.elapsed());
                    if apply {
                        if let Err(e) = catalog.delete_file_location(
                            content_hash,
                            &vol_id_str,
                            relative_path,
                        ) {
                            result.errors.push(format!(
                                "Failed to remove location {}: {e}",
                                relative_path
                            ));
                        } else if let Err(e) = self.remove_sidecar_file_location(
                            &metadata_store,
                            &catalog,
                            content_hash,
                            volume.id,
                            relative_path,
                        ) {
                            result.errors.push(format!(
                                "Failed to update sidecar for {}: {e}",
                                relative_path
                            ));
                        } else {
                            result.removed += 1;
                        }
                    } else {
                        stale_locations.push((
                            content_hash.clone(),
                            vol_id_str.clone(),
                            relative_path.clone(),
                        ));
                    }
                }
            }

            // Check recipe file locations
            let recipes =
                catalog.list_recipes_for_volume_under_prefix(&vol_id_str, prefix)?;
            for (recipe_id, _content_hash, variant_hash, relative_path) in &recipes {
                let file_start = Instant::now();
                let full_path = volume.mount_point.join(relative_path);

                if full_path.exists() {
                    result.checked += 1;
                    on_file(&full_path, CleanupStatus::Ok, file_start.elapsed());
                } else {
                    result.stale += 1;
                    on_file(&full_path, CleanupStatus::Stale, file_start.elapsed());
                    if apply {
                        if let Err(e) = catalog.delete_recipe(recipe_id) {
                            result.errors.push(format!(
                                "Failed to remove recipe {}: {e}",
                                relative_path
                            ));
                        } else if let Err(e) = self.remove_sidecar_recipe(
                            &metadata_store,
                            &catalog,
                            variant_hash,
                            volume.id,
                            relative_path,
                        ) {
                            result.errors.push(format!(
                                "Failed to update sidecar for recipe {}: {e}",
                                relative_path
                            ));
                        } else {
                            result.removed += 1;
                        }
                    }
                }
            }
        }

        // Pass 2: Locationless variants (variant has no locations but asset has other located variants)
        let locationless = if apply {
            catalog.list_locationless_variants()?
        } else {
            catalog.list_would_be_locationless_variants(&stale_locations)?
        };
        result.locationless_variants = locationless.len();

        if apply {
            let preview_gen2 = crate::preview::PreviewGenerator::new(
                &self.catalog_root,
                self.verbosity,
                &self.preview_config,
            );
            // Group by asset_id so we can update sidecars and denormalized columns once per asset
            let mut by_asset: std::collections::HashMap<String, Vec<String>> =
                std::collections::HashMap::new();
            for (asset_id, content_hash) in &locationless {
                by_asset
                    .entry(asset_id.clone())
                    .or_default()
                    .push(content_hash.clone());
            }

            for (asset_id, hashes) in &by_asset {
                for hash in hashes {
                    let file_start = Instant::now();
                    let hash_path = PathBuf::from(hash);

                    // Delete variant from catalog (cascades to file_locations, recipes, embeddings)
                    if let Err(e) = catalog.delete_variant(hash) {
                        result.errors.push(format!(
                            "Failed to delete locationless variant {}: {e}",
                            &hash[..16.min(hash.len())]
                        ));
                        continue;
                    }

                    // Delete derived files
                    let _ = std::fs::remove_file(preview_gen2.preview_path(hash));
                    let _ = std::fs::remove_file(preview_gen2.smart_preview_path(hash));

                    result.removed_variants += 1;
                    on_file(&hash_path, CleanupStatus::LocationlessVariant, file_start.elapsed());
                }

                // Update sidecar: remove the variant(s) from YAML
                if let Ok(uuid) = uuid::Uuid::parse_str(asset_id) {
                    if let Ok(mut asset) = metadata_store.load(uuid) {
                        let hash_set: std::collections::HashSet<&str> =
                            hashes.iter().map(|h| h.as_str()).collect();
                        asset.variants.retain(|v| !hash_set.contains(v.content_hash.as_str()));
                        asset.recipes.retain(|r| !hash_set.contains(r.variant_hash.as_str()));
                        let _ = metadata_store.save(&asset);
                        // Update denormalized columns
                        let _ = catalog.update_denormalized_variant_columns(&asset);
                    }
                }
            }
        }

        // Pass 3: Orphaned assets (all variants have zero file_locations)
        // In apply mode, locations were already removed so we query directly.
        // In report mode, we predict which assets would become orphaned.
        let orphaned_ids = if apply {
            catalog.list_orphaned_asset_ids()?
        } else {
            catalog.list_would_be_orphaned_asset_ids(&stale_locations)?
        };
        result.orphaned_assets = orphaned_ids.len();

        if apply {
            let stack_store = crate::stack::StackStore::new(catalog.conn());
            let preview_gen = crate::preview::PreviewGenerator::new(
                &self.catalog_root,
                self.verbosity,
                &self.preview_config,
            );
            for asset_id in &orphaned_ids {
                let file_start = Instant::now();
                let asset_id_path = PathBuf::from(asset_id);

                // Collect variant hashes and face IDs before deleting DB records
                let variant_hashes: Vec<String> = catalog.conn()
                    .prepare("SELECT content_hash FROM variants WHERE asset_id = ?1")
                    .and_then(|mut s| s.query_map(rusqlite::params![asset_id], |r| r.get(0))
                        .and_then(|rows| rows.collect()))
                    .unwrap_or_default();
                let face_ids: Vec<String> = catalog.conn()
                    .prepare("SELECT id FROM faces WHERE asset_id = ?1")
                    .and_then(|mut s| s.query_map(rusqlite::params![asset_id], |r| r.get(0))
                        .and_then(|rows| rows.collect()))
                    .unwrap_or_default();

                // Remove from stacks, collections, and faces before deleting the asset
                let _ = stack_store.remove(&[asset_id.clone()]);
                let _ = catalog.delete_collection_memberships_for_asset(asset_id);
                let _ = catalog.conn().execute(
                    "DELETE FROM faces WHERE asset_id = ?1",
                    rusqlite::params![asset_id],
                );
                // Delete embedding DB records
                let _ = catalog.conn().execute(
                    "DELETE FROM embeddings WHERE asset_id = ?1",
                    rusqlite::params![asset_id],
                );

                if let Err(e) = catalog.delete_recipes_for_asset(asset_id) {
                    result.errors.push(format!(
                        "Failed to delete recipes for orphaned asset {asset_id}: {e}"
                    ));
                    continue;
                }
                if let Err(e) = catalog.delete_file_locations_for_asset(asset_id) {
                    result.errors.push(format!(
                        "Failed to delete locations for orphaned asset {asset_id}: {e}"
                    ));
                    continue;
                }
                if let Err(e) = catalog.delete_variants_for_asset(asset_id) {
                    result.errors.push(format!(
                        "Failed to delete variants for orphaned asset {asset_id}: {e}"
                    ));
                    continue;
                }
                if let Err(e) = catalog.delete_asset(asset_id) {
                    result.errors.push(format!(
                        "Failed to delete orphaned asset {asset_id}: {e}"
                    ));
                    continue;
                }

                // Delete sidecar YAML
                if let Ok(uuid) = uuid::Uuid::parse_str(asset_id) {
                    if let Err(e) = metadata_store.delete(uuid) {
                        result.errors.push(format!(
                            "Failed to delete sidecar for orphaned asset {asset_id}: {e}"
                        ));
                    }
                }

                // Delete derived files: previews, smart previews, embeddings, face crops
                for hash in &variant_hashes {
                    let _ = std::fs::remove_file(preview_gen.preview_path(hash));
                    let _ = std::fs::remove_file(preview_gen.smart_preview_path(hash));
                }
                for face_id in &face_ids {
                    let prefix = &face_id[..2.min(face_id.len())];
                    let _ = std::fs::remove_file(
                        self.catalog_root.join("faces").join(prefix).join(format!("{face_id}.jpg")),
                    );
                    let _ = std::fs::remove_file(
                        self.catalog_root.join("embeddings").join("arcface").join(prefix).join(format!("{face_id}.bin")),
                    );
                }
                // Delete SigLIP embedding binaries
                for model in &["siglip-vit-b16-256", "siglip-vit-l16-256"] {
                    let prefix = &asset_id[..2.min(asset_id.len())];
                    let _ = std::fs::remove_file(
                        self.catalog_root.join("embeddings").join(model).join(prefix).join(format!("{asset_id}.bin")),
                    );
                }

                result.removed_assets += 1;
                on_file(&asset_id_path, CleanupStatus::OrphanedAsset, file_start.elapsed());
            }
        }

        // Pass 4: Orphaned previews (preview files with no matching variant)
        let variant_hashes = catalog.list_all_variant_hashes()?;
        scan_orphaned_sharded_files(
            &self.catalog_root.join("previews"),
            |stem| {
                let content_hash = format!("sha256:{stem}");
                variant_hashes.contains(&content_hash)
            },
            apply,
            &mut result.orphaned_previews,
            &mut result.removed_previews,
            &mut result.errors,
            &on_file,
        );

        // Pass 5: Orphaned smart previews (same logic, different directory)
        scan_orphaned_sharded_files(
            &self.catalog_root.join("smart_previews"),
            |stem| {
                let content_hash = format!("sha256:{stem}");
                variant_hashes.contains(&content_hash)
            },
            apply,
            &mut result.orphaned_smart_previews,
            &mut result.removed_smart_previews,
            &mut result.errors,
            &on_file,
        );

        // Pass 6: Orphaned embedding binaries (asset_id.bin under embeddings/<model>/)
        let asset_ids_set: HashSet<String> = catalog.list_all_asset_ids()?;
        let emb_base = self.catalog_root.join("embeddings");
        if emb_base.is_dir() {
            if let Ok(model_entries) = std::fs::read_dir(&emb_base) {
                for model_entry in model_entries.flatten() {
                    if !model_entry.path().is_dir() {
                        continue;
                    }
                    let model_name = model_entry.file_name().to_string_lossy().to_string();
                    if model_name == "arcface" {
                        continue; // handled separately in pass 7
                    }
                    scan_orphaned_sharded_files(
                        &model_entry.path(),
                        |stem| asset_ids_set.contains(stem),
                        apply,
                        &mut result.orphaned_embeddings,
                        &mut result.removed_embeddings,
                        &mut result.errors,
                        &on_file,
                    );
                }
            }
        }

        // Pass 7: Orphaned face crop thumbnails (face_id.jpg under faces/)
        let face_ids_set: HashSet<String> = catalog.conn()
            .prepare("SELECT id FROM faces")
            .and_then(|mut s| s.query_map([], |r| r.get(0))
                .and_then(|rows| rows.collect()))
            .unwrap_or_default();
        scan_orphaned_sharded_files(
            &self.catalog_root.join("faces"),
            |stem| face_ids_set.contains(stem),
            apply,
            &mut result.orphaned_face_files,
            &mut result.removed_face_files,
            &mut result.errors,
            &on_file,
        );

        // Pass 7: Orphaned ArcFace embedding binaries (face_id.bin under embeddings/arcface/)
        scan_orphaned_sharded_files(
            &self.catalog_root.join("embeddings").join("arcface"),
            |stem| face_ids_set.contains(stem),
            apply,
            &mut result.orphaned_embeddings,
            &mut result.removed_embeddings,
            &mut result.errors,
            &on_file,
        );

        Ok(result)
    }

    /// Delete assets from the catalog. Report-only by default; `apply` executes deletion.
    /// `remove_files` (requires `apply`) also deletes physical media and recipe files from disk.
    pub fn delete_assets(
        &self,
        asset_ids: &[String],
        apply: bool,
        remove_files: bool,
        on_asset: impl Fn(&str, &DeleteStatus, Duration),
    ) -> Result<DeleteResult> {
        let catalog = Catalog::open(&self.catalog_root)?;
        let metadata_store = MetadataStore::new(&self.catalog_root);
        let registry = DeviceRegistry::new(&self.catalog_root);
        let preview_gen = crate::preview::PreviewGenerator::new(
            &self.catalog_root,
            self.verbosity,
            &self.preview_config,
        );

        // Build volume lookup for file deletion
        let volumes = registry.list().unwrap_or_default();
        let volume_map: std::collections::HashMap<String, &Volume> = volumes
            .iter()
            .map(|v| (v.id.to_string(), v))
            .collect();

        let stack_store = crate::stack::StackStore::new(catalog.conn());
        let mut stacks_changed = false;
        let mut collections_changed = false;

        let mut result = DeleteResult {
            deleted: 0,
            not_found: Vec::new(),
            files_removed: 0,
            previews_removed: 0,
            dry_run: !apply,
            errors: Vec::new(),
        };

        for raw_id in asset_ids {
            let asset_start = Instant::now();

            // 1. Resolve ID (prefix match)
            let asset_id = match catalog.resolve_asset_id(raw_id) {
                Ok(Some(id)) => id,
                Ok(None) => {
                    result.not_found.push(raw_id.clone());
                    on_asset(raw_id, &DeleteStatus::NotFound, asset_start.elapsed());
                    continue;
                }
                Err(e) => {
                    let msg = format!("{raw_id}: {e}");
                    result.errors.push(msg.clone());
                    on_asset(raw_id, &DeleteStatus::Error(msg), asset_start.elapsed());
                    continue;
                }
            };

            // 2. Gather variant hashes (before deleting variants)
            let variant_hashes = catalog.list_variant_hashes_for_asset(&asset_id)
                .unwrap_or_default();

            // 3. Gather file + recipe locations (for --remove-files and report)
            let file_locations = catalog.list_file_locations_for_asset(&asset_id)
                .unwrap_or_default();
            let recipe_locations = catalog.list_recipes_for_asset(&asset_id)
                .unwrap_or_default();

            if apply {
                // 4a. Delete physical files (only if remove_files)
                if remove_files {
                    for (_hash, rel_path, vol_id) in &file_locations {
                        if let Some(vol) = volume_map.get(vol_id.as_str()) {
                            if vol.is_online {
                                let full_path = vol.mount_point.join(rel_path);
                                if full_path.exists() {
                                    if let Err(e) = std::fs::remove_file(&full_path) {
                                        result.errors.push(format!(
                                            "Failed to remove file {}: {e}",
                                            full_path.display()
                                        ));
                                    } else {
                                        result.files_removed += 1;
                                    }
                                }
                            } else {
                                eprintln!(
                                    "  Warning: volume '{}' is offline, skipping file {}",
                                    vol.label, rel_path
                                );
                            }
                        }
                    }
                    for (_recipe_id, _content_hash, _variant_hash, rel_path, vol_id) in &recipe_locations {
                        if let Some(vol) = volume_map.get(vol_id.as_str()) {
                            if vol.is_online {
                                let full_path = vol.mount_point.join(rel_path);
                                if full_path.exists() {
                                    if let Err(e) = std::fs::remove_file(&full_path) {
                                        result.errors.push(format!(
                                            "Failed to remove recipe file {}: {e}",
                                            full_path.display()
                                        ));
                                    } else {
                                        result.files_removed += 1;
                                    }
                                }
                            }
                        }
                    }
                }

                // 4b. Remove from stacks
                if stack_store.remove(&[asset_id.clone()]).unwrap_or(0) > 0 {
                    stacks_changed = true;
                }

                // 4c. Remove collection memberships
                if catalog.delete_collection_memberships_for_asset(&asset_id).unwrap_or(0) > 0 {
                    collections_changed = true;
                }

                // 4c2. Delete faces and their derived files
                let face_ids: Vec<String> = catalog.conn()
                    .prepare("SELECT id FROM faces WHERE asset_id = ?1")
                    .and_then(|mut s| s.query_map(rusqlite::params![&asset_id], |r| r.get(0))
                        .and_then(|rows| rows.collect()))
                    .unwrap_or_default();
                let _ = catalog.conn().execute(
                    "DELETE FROM faces WHERE asset_id = ?1",
                    rusqlite::params![&asset_id],
                );
                for face_id in &face_ids {
                    let prefix = &face_id[..2.min(face_id.len())];
                    let _ = std::fs::remove_file(
                        self.catalog_root.join("faces").join(prefix).join(format!("{face_id}.jpg")),
                    );
                    let _ = std::fs::remove_file(
                        self.catalog_root.join("embeddings").join("arcface").join(prefix).join(format!("{face_id}.bin")),
                    );
                }

                // 4c3. Delete embeddings (DB records + binary files)
                let _ = catalog.conn().execute(
                    "DELETE FROM embeddings WHERE asset_id = ?1",
                    rusqlite::params![&asset_id],
                );
                for model in &["siglip-vit-b16-256", "siglip-vit-l16-256"] {
                    let prefix = &asset_id[..2.min(asset_id.len())];
                    let _ = std::fs::remove_file(
                        self.catalog_root.join("embeddings").join(model).join(prefix).join(format!("{asset_id}.bin")),
                    );
                }

                // 4d. Delete recipes
                if let Err(e) = catalog.delete_recipes_for_asset(&asset_id) {
                    result.errors.push(format!("{asset_id}: failed to delete recipes: {e}"));
                    on_asset(&asset_id, &DeleteStatus::Error(e.to_string()), asset_start.elapsed());
                    continue;
                }

                // 4e. Delete file locations
                if let Err(e) = catalog.delete_file_locations_for_asset(&asset_id) {
                    result.errors.push(format!("{asset_id}: failed to delete locations: {e}"));
                    on_asset(&asset_id, &DeleteStatus::Error(e.to_string()), asset_start.elapsed());
                    continue;
                }

                // 4f. Delete variants
                if let Err(e) = catalog.delete_variants_for_asset(&asset_id) {
                    result.errors.push(format!("{asset_id}: failed to delete variants: {e}"));
                    on_asset(&asset_id, &DeleteStatus::Error(e.to_string()), asset_start.elapsed());
                    continue;
                }

                // 4g. Delete asset
                if let Err(e) = catalog.delete_asset(&asset_id) {
                    result.errors.push(format!("{asset_id}: failed to delete asset: {e}"));
                    on_asset(&asset_id, &DeleteStatus::Error(e.to_string()), asset_start.elapsed());
                    continue;
                }

                // 4h. Delete sidecar YAML
                if let Ok(uuid) = Uuid::parse_str(&asset_id) {
                    if let Err(e) = metadata_store.delete(uuid) {
                        result.errors.push(format!("{asset_id}: failed to delete sidecar: {e}"));
                    }
                }

                // 4i. Delete previews
                for hash in &variant_hashes {
                    let preview_path = preview_gen.preview_path(hash);
                    if preview_path.exists() {
                        if std::fs::remove_file(&preview_path).is_ok() {
                            result.previews_removed += 1;
                        }
                    }
                    let smart_path = preview_gen.smart_preview_path(hash);
                    if smart_path.exists() {
                        if std::fs::remove_file(&smart_path).is_ok() {
                            result.previews_removed += 1;
                        }
                    }
                }

                result.deleted += 1;
                on_asset(&asset_id, &DeleteStatus::Deleted, asset_start.elapsed());
            } else {
                // Report mode: count what would be affected
                result.deleted += 1;
                on_asset(&asset_id, &DeleteStatus::Deleted, asset_start.elapsed());
            }
        }

        // Persist stack/collection changes
        if apply && stacks_changed {
            if let Ok(yaml) = stack_store.export_all() {
                let _ = crate::stack::save_yaml(&self.catalog_root, &yaml);
            }
        }
        if apply && collections_changed {
            let col_store = crate::collection::CollectionStore::new(catalog.conn());
            if let Ok(yaml) = col_store.export_all() {
                let _ = crate::collection::save_yaml(&self.catalog_root, &yaml);
            }
        }

        Ok(result)
    }

    /// Remove a volume and all its associated data (locations, recipes, orphaned assets/previews).
    /// Report-only by default; `--apply` executes removal.
    pub fn remove_volume(
        &self,
        label: &str,
        apply: bool,
        on_file: impl Fn(&Path, CleanupStatus, Duration),
    ) -> Result<VolumeRemoveResult> {
        let registry = DeviceRegistry::new(&self.catalog_root);
        let catalog = Catalog::open(&self.catalog_root)?;
        let metadata_store = MetadataStore::new(&self.catalog_root);

        let volume = registry.resolve_volume(label)?;
        let vol_id_str = volume.id.to_string();

        let mut result = VolumeRemoveResult {
            volume_label: volume.label.clone(),
            volume_id: vol_id_str.clone(),
            locations: 0,
            locations_removed: 0,
            recipes: 0,
            recipes_removed: 0,
            orphaned_assets: 0,
            removed_assets: 0,
            orphaned_previews: 0,
            removed_previews: 0,
            apply,
            errors: Vec::new(),
        };

        // Gather all locations and recipes on this volume
        let locations = catalog.list_locations_for_volume_under_prefix(&vol_id_str, "")?;
        let recipes = catalog.list_recipes_for_volume_under_prefix(&vol_id_str, "")?;
        result.locations = locations.len();
        result.recipes = recipes.len();

        // Build stale list for report-mode orphan prediction
        let stale_locations: Vec<(String, String, String)> = locations
            .iter()
            .map(|(hash, path)| (hash.clone(), vol_id_str.clone(), path.clone()))
            .collect();

        if apply {
            // Remove all file locations on this volume
            for (content_hash, relative_path) in &locations {
                let file_start = Instant::now();
                if let Err(e) = catalog.delete_file_location(
                    content_hash,
                    &vol_id_str,
                    relative_path,
                ) {
                    result.errors.push(format!(
                        "Failed to remove location {}: {e}", relative_path
                    ));
                } else if let Err(e) = self.remove_sidecar_file_location(
                    &metadata_store,
                    &catalog,
                    content_hash,
                    volume.id,
                    relative_path,
                ) {
                    result.errors.push(format!(
                        "Failed to update sidecar for {}: {e}", relative_path
                    ));
                } else {
                    result.locations_removed += 1;
                    on_file(
                        &PathBuf::from(relative_path),
                        CleanupStatus::Stale,
                        file_start.elapsed(),
                    );
                }
            }

            // Remove all recipes on this volume
            for (recipe_id, _content_hash, variant_hash, relative_path) in &recipes {
                let file_start = Instant::now();
                if let Err(e) = catalog.delete_recipe(recipe_id) {
                    result.errors.push(format!(
                        "Failed to remove recipe {}: {e}", relative_path
                    ));
                } else if let Err(e) = self.remove_sidecar_recipe(
                    &metadata_store,
                    &catalog,
                    variant_hash,
                    volume.id,
                    relative_path,
                ) {
                    result.errors.push(format!(
                        "Failed to update sidecar for recipe {}: {e}", relative_path
                    ));
                } else {
                    result.recipes_removed += 1;
                    on_file(
                        &PathBuf::from(relative_path),
                        CleanupStatus::Stale,
                        file_start.elapsed(),
                    );
                }
            }
        }

        // Orphaned assets
        let orphaned_ids = if apply {
            catalog.list_orphaned_asset_ids()?
        } else {
            catalog.list_would_be_orphaned_asset_ids(&stale_locations)?
        };
        result.orphaned_assets = orphaned_ids.len();

        if apply {
            let stack_store = crate::stack::StackStore::new(catalog.conn());
            let preview_gen = crate::preview::PreviewGenerator::new(
                &self.catalog_root,
                self.verbosity,
                &self.preview_config,
            );
            for asset_id in &orphaned_ids {
                let file_start = Instant::now();
                let asset_id_path = PathBuf::from(asset_id);

                // Collect variant hashes and face IDs before deleting DB records
                let variant_hashes: Vec<String> = catalog.conn()
                    .prepare("SELECT content_hash FROM variants WHERE asset_id = ?1")
                    .and_then(|mut s| s.query_map(rusqlite::params![asset_id], |r| r.get(0))
                        .and_then(|rows| rows.collect()))
                    .unwrap_or_default();
                let face_ids: Vec<String> = catalog.conn()
                    .prepare("SELECT id FROM faces WHERE asset_id = ?1")
                    .and_then(|mut s| s.query_map(rusqlite::params![asset_id], |r| r.get(0))
                        .and_then(|rows| rows.collect()))
                    .unwrap_or_default();

                // Remove from stacks, collections, faces, and embeddings
                let _ = stack_store.remove(&[asset_id.clone()]);
                let _ = catalog.delete_collection_memberships_for_asset(asset_id);
                let _ = catalog.conn().execute(
                    "DELETE FROM faces WHERE asset_id = ?1",
                    rusqlite::params![asset_id],
                );
                let _ = catalog.conn().execute(
                    "DELETE FROM embeddings WHERE asset_id = ?1",
                    rusqlite::params![asset_id],
                );

                if let Err(e) = catalog.delete_recipes_for_asset(asset_id) {
                    result.errors.push(format!(
                        "Failed to delete recipes for orphaned asset {asset_id}: {e}"
                    ));
                    continue;
                }
                if let Err(e) = catalog.delete_file_locations_for_asset(asset_id) {
                    result.errors.push(format!(
                        "Failed to delete locations for orphaned asset {asset_id}: {e}"
                    ));
                    continue;
                }
                if let Err(e) = catalog.delete_variants_for_asset(asset_id) {
                    result.errors.push(format!(
                        "Failed to delete variants for orphaned asset {asset_id}: {e}"
                    ));
                    continue;
                }
                if let Err(e) = catalog.delete_asset(asset_id) {
                    result.errors.push(format!(
                        "Failed to delete orphaned asset {asset_id}: {e}"
                    ));
                    continue;
                }
                if let Ok(uuid) = uuid::Uuid::parse_str(asset_id) {
                    if let Err(e) = metadata_store.delete(uuid) {
                        result.errors.push(format!(
                            "Failed to delete sidecar for orphaned asset {asset_id}: {e}"
                        ));
                    }
                }

                // Delete derived files
                for hash in &variant_hashes {
                    let _ = std::fs::remove_file(preview_gen.preview_path(hash));
                    let _ = std::fs::remove_file(preview_gen.smart_preview_path(hash));
                }
                for face_id in &face_ids {
                    let prefix = &face_id[..2.min(face_id.len())];
                    let _ = std::fs::remove_file(
                        self.catalog_root.join("faces").join(prefix).join(format!("{face_id}.jpg")),
                    );
                    let _ = std::fs::remove_file(
                        self.catalog_root.join("embeddings").join("arcface").join(prefix).join(format!("{face_id}.bin")),
                    );
                }
                for model in &["siglip-vit-b16-256", "siglip-vit-l16-256"] {
                    let prefix = &asset_id[..2.min(asset_id.len())];
                    let _ = std::fs::remove_file(
                        self.catalog_root.join("embeddings").join(model).join(prefix).join(format!("{asset_id}.bin")),
                    );
                }

                result.removed_assets += 1;
                on_file(&asset_id_path, CleanupStatus::OrphanedAsset, file_start.elapsed());
            }
        }

        // Orphaned previews and smart previews
        let variant_hashes = catalog.list_all_variant_hashes()?;
        scan_orphaned_sharded_files(
            &self.catalog_root.join("previews"),
            |stem| {
                let content_hash = format!("sha256:{stem}");
                variant_hashes.contains(&content_hash)
            },
            apply,
            &mut result.orphaned_previews,
            &mut result.removed_previews,
            &mut result.errors,
            &on_file,
        );
        scan_orphaned_sharded_files(
            &self.catalog_root.join("smart_previews"),
            |stem| {
                let content_hash = format!("sha256:{stem}");
                variant_hashes.contains(&content_hash)
            },
            apply,
            &mut result.orphaned_previews,
            &mut result.removed_previews,
            &mut result.errors,
            &on_file,
        );

        // Finally, remove the volume itself
        if apply {
            if let Err(e) = catalog.delete_volume(&vol_id_str) {
                result.errors.push(format!("Failed to delete volume from catalog: {e}"));
            }
            if let Err(e) = registry.remove(label) {
                result.errors.push(format!("Failed to remove volume from registry: {e}"));
            }
        }

        Ok(result)
    }

    /// Combine a source volume into a target volume, rewriting paths.
    ///
    /// The source must be a subdirectory of the target (same physical disk,
    /// deeper mount point). All file_locations and recipes are moved from source
    /// to target with a computed path prefix. In apply mode, sidecars are
    /// updated first (source of truth), then SQLite bulk update, then the
    /// source volume is removed.
    pub fn combine_volume(
        &self,
        source_label: &str,
        target_label: &str,
        apply: bool,
        on_asset: impl Fn(&str, Duration),
    ) -> Result<VolumeCombineResult> {
        let registry = DeviceRegistry::new(&self.catalog_root);
        let catalog = Catalog::open(&self.catalog_root)?;
        let metadata_store = MetadataStore::new(&self.catalog_root);

        let source = registry.resolve_volume(source_label)?;
        let target = registry.resolve_volume(target_label)?;

        let source_id = source.id.to_string();
        let target_id = target.id.to_string();

        if source.id == target.id {
            bail!(
                "Source and target are the same volume ('{}').",
                source.label
            );
        }

        // Compute path prefix: source mount must be under target mount
        let prefix = source
            .mount_point
            .strip_prefix(&target.mount_point)
            .map_err(|_| {
                anyhow::anyhow!(
                    "Source volume '{}' ({}) is not a subdirectory of target volume '{}' ({}). \
                     Cannot compute path prefix.",
                    source.label,
                    source.mount_point.display(),
                    target.label,
                    target.mount_point.display(),
                )
            })?;

        let prefix_str = if prefix.as_os_str().is_empty() {
            String::new()
        } else {
            let mut p = prefix.to_string_lossy().to_string();
            if !p.ends_with('/') {
                p.push('/');
            }
            p
        };

        // Count locations and recipes
        let locations = catalog.list_locations_for_volume_under_prefix(&source_id, "")?;
        let recipes = catalog.list_recipes_for_volume_under_prefix(&source_id, "")?;
        let asset_ids = catalog.list_asset_ids_on_volume(&source_id)?;

        let mut result = VolumeCombineResult {
            source_label: source.label.clone(),
            source_id: source_id.clone(),
            target_label: target.label.clone(),
            target_id: target_id.clone(),
            path_prefix: prefix_str.clone(),
            locations: locations.len(),
            locations_moved: 0,
            recipes: recipes.len(),
            recipes_moved: 0,
            assets_affected: asset_ids.len(),
            apply,
            errors: Vec::new(),
        };

        if !apply {
            return Ok(result);
        }

        // --- Apply mode ---

        // 1. Update sidecars (source of truth)
        for asset_id_str in &asset_ids {
            let asset_start = Instant::now();
            let uuid = match asset_id_str.parse::<Uuid>() {
                Ok(u) => u,
                Err(e) => {
                    result
                        .errors
                        .push(format!("Invalid asset UUID {asset_id_str}: {e}"));
                    continue;
                }
            };
            match metadata_store.load(uuid) {
                Ok(mut asset) => {
                    let mut changed = false;

                    // Rewrite variant locations
                    for variant in &mut asset.variants {
                        for loc in &mut variant.locations {
                            if loc.volume_id == source.id {
                                loc.volume_id = target.id;
                                let old_path = loc.relative_path.to_string_lossy().to_string();
                                loc.relative_path =
                                    PathBuf::from(format!("{prefix_str}{old_path}"));
                                changed = true;
                            }
                        }
                    }

                    // Rewrite recipe locations
                    for recipe in &mut asset.recipes {
                        if recipe.location.volume_id == source.id {
                            recipe.location.volume_id = target.id;
                            let old_path =
                                recipe.location.relative_path.to_string_lossy().to_string();
                            recipe.location.relative_path =
                                PathBuf::from(format!("{prefix_str}{old_path}"));
                            changed = true;
                        }
                    }

                    if changed {
                        if let Err(e) = metadata_store.save(&asset) {
                            result.errors.push(format!(
                                "Failed to save sidecar for asset {asset_id_str}: {e}"
                            ));
                        }
                    }
                    on_asset(asset_id_str, asset_start.elapsed());
                }
                Err(e) => {
                    result
                        .errors
                        .push(format!("Failed to load sidecar for asset {asset_id_str}: {e}"));
                }
            }
        }

        // 2. Ensure target volume exists in catalog (it may not if nothing was imported onto it)
        catalog.ensure_volume(&target)?;

        // 3. Bulk SQL update
        match catalog.bulk_move_file_locations(&source_id, &target_id, &prefix_str) {
            Ok(n) => result.locations_moved = n,
            Err(e) => result
                .errors
                .push(format!("Failed to move file locations: {e}")),
        }
        match catalog.bulk_move_recipes(&source_id, &target_id, &prefix_str) {
            Ok(n) => result.recipes_moved = n,
            Err(e) => result.errors.push(format!("Failed to move recipes: {e}")),
        }

        // 4. Remove source volume
        if let Err(e) = catalog.delete_volume(&source_id) {
            result
                .errors
                .push(format!("Failed to delete volume from catalog: {e}"));
        }
        if let Err(e) = registry.remove(source_label) {
            result
                .errors
                .push(format!("Failed to remove volume from registry: {e}"));
        }

        Ok(result)
    }

    /// Remove same-volume duplicate file locations.
    ///
    /// For each variant with 2+ locations on the same volume, keeps the "best"
    /// location and removes the rest. In apply mode, deletes physical files and
    /// removes catalog/sidecar location records.
    pub fn dedup(
        &self,
        volume_filter: Option<&str>,
        format_filter: Option<&str>,
        path_prefix: Option<&str>,
        prefer: Option<&str>,
        min_copies: usize,
        apply: bool,
        on_entry: impl Fn(&str, &str, DedupStatus, &str),
    ) -> Result<DedupResult> {
        let registry = DeviceRegistry::new(&self.catalog_root);
        let catalog = Catalog::open(&self.catalog_root)?;
        let metadata_store = MetadataStore::new(&self.catalog_root);

        let filter_volume_id = if let Some(label) = volume_filter {
            let vol = registry.resolve_volume(label)?;
            Some(vol.id.to_string())
        } else {
            None
        };

        let entries = if format_filter.is_some() || path_prefix.is_some() || filter_volume_id.is_some() {
            catalog.find_duplicates_filtered(
                "same",
                filter_volume_id.as_deref(),
                format_filter,
                path_prefix,
            )?
        } else {
            catalog.find_duplicates_same_volume()?
        };

        let mut result = DedupResult {
            duplicates_found: 0,
            locations_to_remove: 0,
            locations_removed: 0,
            files_deleted: 0,
            recipes_removed: 0,
            bytes_freed: 0,
            dry_run: !apply,
            errors: Vec::new(),
        };

        // Build a map of volumes for resolving mount points
        let volumes = registry.list()?;
        let vol_map: std::collections::HashMap<String, &Volume> = volumes
            .iter()
            .map(|v| (v.id.to_string(), v))
            .collect();

        for entry in &entries {
            // Group locations by volume_id
            let mut by_volume: BTreeMap<String, Vec<&crate::catalog::LocationDetails>> =
                BTreeMap::new();
            for loc in &entry.locations {
                by_volume
                    .entry(loc.volume_id.clone())
                    .or_default()
                    .push(loc);
            }

            // Track how many locations we're removing for this variant (for min-copies)
            let mut entry_removals = 0usize;

            for (vol_id, mut locs) in by_volume {
                if locs.len() < 2 {
                    continue;
                }

                // If volume filter set, skip other volumes
                if let Some(ref fid) = filter_volume_id {
                    if &vol_id != fid {
                        continue;
                    }
                }

                result.duplicates_found += 1;

                // Sort by resolution heuristic (best first = keep)
                locs.sort_by(|a, b| {
                    // 1. Prefer locations matching --prefer substring
                    if let Some(prefix) = prefer {
                        let a_match = a.relative_path.contains(prefix);
                        let b_match = b.relative_path.contains(prefix);
                        if a_match != b_match {
                            return if a_match {
                                std::cmp::Ordering::Less
                            } else {
                                std::cmp::Ordering::Greater
                            };
                        }
                    }

                    // 2. Prefer more recently verified (NULL = oldest)
                    let a_ver = a.verified_at.as_deref().unwrap_or("");
                    let b_ver = b.verified_at.as_deref().unwrap_or("");
                    match b_ver.cmp(a_ver) {
                        std::cmp::Ordering::Equal => {}
                        other => return other,
                    }

                    // 3. Prefer shorter relative paths
                    match a.relative_path.len().cmp(&b.relative_path.len()) {
                        std::cmp::Ordering::Equal => {}
                        other => return other,
                    }

                    // 4. Tiebreak: alphabetical
                    a.relative_path.cmp(&b.relative_path)
                });

                let vol_label = locs
                    .first()
                    .map(|l| l.volume_label.as_str())
                    .unwrap_or("?");

                // Keep the first, mark the rest for removal
                on_entry(
                    &entry.original_filename,
                    &locs[0].relative_path,
                    DedupStatus::Keep,
                    vol_label,
                );

                for loc in &locs[1..] {
                    // Check min-copies constraint: total locations across all volumes
                    let remaining = entry.locations.len() - entry_removals;
                    if remaining <= min_copies {
                        on_entry(
                            &entry.original_filename,
                            &loc.relative_path,
                            DedupStatus::Skipped,
                            vol_label,
                        );
                        continue;
                    }

                    entry_removals += 1;
                    result.locations_to_remove += 1;
                    result.bytes_freed += entry.file_size;

                    on_entry(
                        &entry.original_filename,
                        &loc.relative_path,
                        DedupStatus::Remove,
                        vol_label,
                    );

                    // Find co-located recipes (same variant, same volume, same directory)
                    let loc_dir = std::path::Path::new(&loc.relative_path)
                        .parent()
                        .map(|p| p.to_string_lossy().to_string())
                        .unwrap_or_default();
                    let colocated_recipes = catalog
                        .list_recipes_for_variant_on_volume(&entry.content_hash, &vol_id)
                        .unwrap_or_default()
                        .into_iter()
                        .filter(|(_id, _hash, rpath)| {
                            let rdir = std::path::Path::new(rpath)
                                .parent()
                                .map(|p| p.to_string_lossy().to_string())
                                .unwrap_or_default();
                            rdir == loc_dir
                        })
                        .collect::<Vec<_>>();

                    if apply {
                        // Delete the physical file
                        if let Some(vol) = vol_map.get(&vol_id) {
                            if vol.is_online {
                                let full_path = vol.mount_point.join(&loc.relative_path);
                                match std::fs::remove_file(&full_path) {
                                    Ok(()) => {
                                        result.files_deleted += 1;
                                    }
                                    Err(e) => {
                                        result.errors.push(format!(
                                            "Failed to delete {}: {e}",
                                            full_path.display()
                                        ));
                                        continue;
                                    }
                                }
                            }
                        }

                        // Remove from catalog
                        if let Err(e) = catalog.delete_file_location(
                            &entry.content_hash,
                            &vol_id,
                            &loc.relative_path,
                        ) {
                            result.errors.push(format!(
                                "Failed to remove catalog location {}: {e}",
                                loc.relative_path
                            ));
                        } else if let Err(e) = self.remove_sidecar_file_location(
                            &metadata_store,
                            &catalog,
                            &entry.content_hash,
                            vol_id.parse().unwrap_or_default(),
                            &loc.relative_path,
                        ) {
                            result.errors.push(format!(
                                "Failed to update sidecar for {}: {e}",
                                loc.relative_path
                            ));
                        } else {
                            result.locations_removed += 1;
                        }

                        // Clean up co-located recipe files
                        for (recipe_id, _recipe_hash, recipe_path) in &colocated_recipes {
                            if let Some(vol) = vol_map.get(&vol_id) {
                                if vol.is_online {
                                    let recipe_full = vol.mount_point.join(recipe_path);
                                    let _ = std::fs::remove_file(&recipe_full);
                                }
                            }
                            if let Err(e) = catalog.delete_recipe(recipe_id) {
                                result.errors.push(format!(
                                    "Failed to remove recipe {recipe_path}: {e}"
                                ));
                            } else if let Err(e) = self.remove_sidecar_recipe(
                                &metadata_store,
                                &catalog,
                                &entry.content_hash,
                                vol_id.parse().unwrap_or_default(),
                                recipe_path,
                            ) {
                                result.errors.push(format!(
                                    "Failed to update sidecar for recipe {recipe_path}: {e}"
                                ));
                            } else {
                                result.recipes_removed += 1;
                            }
                        }
                    } else {
                        // Dry-run: just count recipes
                        result.recipes_removed += colocated_recipes.len();
                    }
                }
            }
        }

        Ok(result)
    }

    /// Re-read metadata from changed recipe/sidecar files, and optionally
    /// re-extract embedded XMP from JPEG/TIFF media files (`--media`).
    pub fn refresh(
        &self,
        paths: &[PathBuf],
        volume: Option<&Volume>,
        asset_id: Option<&str>,
        dry_run: bool,
        media: bool,
        exclude_patterns: &[String],
        on_file: impl Fn(&Path, RefreshStatus, Duration),
    ) -> Result<RefreshResult> {
        let content_store = ContentStore::new(&self.catalog_root);
        let metadata_store = MetadataStore::new(&self.catalog_root);
        let catalog = Catalog::open(&self.catalog_root)?;
        let registry = DeviceRegistry::new(&self.catalog_root);

        let mut result = RefreshResult {
            unchanged: 0,
            refreshed: 0,
            missing: 0,
            skipped: 0,
            errors: Vec::new(),
        };

        // Collect recipe locations to check: (recipe_id, content_hash, variant_hash, relative_path, volume_id_str)
        let recipe_entries: Vec<(String, String, String, String, String)>;

        if let Some(aid) = asset_id {
            // Asset mode: all recipes for a specific asset
            recipe_entries = catalog.list_recipes_for_asset(aid)?;
        } else if !paths.is_empty() {
            // Path mode: scan files under given paths, filter to recipes, look up each
            let files = resolve_files(paths, exclude_patterns);
            let filter = FileTypeFilter::default();
            let vol = volume.ok_or_else(|| anyhow::anyhow!("No volume resolved for path mode"))?;
            let vol_id = vol.id.to_string();

            let mut entries = Vec::new();
            for file_path in &files {
                let ext = file_path
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("");
                if !filter.is_recipe(ext) {
                    continue;
                }
                let relative_path = match file_path.strip_prefix(&vol.mount_point) {
                    Ok(rp) => rp.to_string_lossy().to_string(),
                    Err(_) => continue,
                };
                if let Some((recipe_id, content_hash, variant_hash)) =
                    catalog.find_recipe_by_volume_and_path(&vol_id, &relative_path)?
                {
                    entries.push((recipe_id, content_hash, variant_hash, relative_path, vol_id.clone()));
                }
            }
            recipe_entries = entries;
        } else if let Some(vol) = volume {
            // Volume mode: all recipes on the specified volume
            let vol_id = vol.id.to_string();
            recipe_entries = catalog
                .list_recipes_for_volume_under_prefix(&vol_id, "")?
                .into_iter()
                .map(|(rid, ch, vh, rp)| (rid, ch, vh, rp, vol_id.clone()))
                .collect();
        } else {
            // All mode: iterate all online volumes
            let volumes = registry.list()?;
            let mut entries = Vec::new();
            for vol in &volumes {
                if !vol.is_online {
                    continue;
                }
                let vol_id = vol.id.to_string();
                for (rid, ch, vh, rp) in
                    catalog.list_recipes_for_volume_under_prefix(&vol_id, "")?
                {
                    entries.push((rid, ch, vh, rp, vol_id.clone()));
                }
            }
            recipe_entries = entries;
        }

        // Resolve volumes for lookup
        let all_volumes = registry.list()?;

        // Process each recipe
        for (recipe_id, stored_hash, variant_hash, relative_path, volume_id_str) in &recipe_entries {
            let file_start = Instant::now();

            // Find the volume
            let vol = match all_volumes.iter().find(|v| v.id.to_string() == *volume_id_str) {
                Some(v) => v,
                None => {
                    result.skipped += 1;
                    on_file(Path::new(&relative_path), RefreshStatus::Offline, file_start.elapsed());
                    continue;
                }
            };

            if !vol.is_online {
                result.skipped += 1;
                on_file(
                    &vol.mount_point.join(relative_path),
                    RefreshStatus::Offline,
                    file_start.elapsed(),
                );
                continue;
            }

            let full_path = vol.mount_point.join(relative_path);

            if !full_path.exists() {
                result.missing += 1;
                on_file(&full_path, RefreshStatus::Missing, file_start.elapsed());
                continue;
            }

            let new_hash = match content_store.hash_file(&full_path) {
                Ok(h) => h,
                Err(e) => {
                    result.errors.push(format!("{}: {}", full_path.display(), e));
                    continue;
                }
            };

            if new_hash == *stored_hash {
                result.unchanged += 1;
                on_file(&full_path, RefreshStatus::Unchanged, file_start.elapsed());
            } else {
                if !dry_run {
                    if let Err(e) = self.apply_modified_recipe(
                        &catalog,
                        &metadata_store,
                        recipe_id,
                        &new_hash,
                        variant_hash,
                        vol,
                        &full_path,
                        relative_path,
                    ) {
                        result.errors.push(format!("{}: {}", full_path.display(), e));
                        continue;
                    }
                }
                result.refreshed += 1;
                on_file(&full_path, RefreshStatus::Refreshed, file_start.elapsed());
            }
        }

        // --- Media file processing (embedded XMP re-extraction) ---
        if media {
            // Collect media file locations: (content_hash, relative_path, volume_id)
            let media_entries: Vec<(String, String, String)>;

            if let Some(aid) = asset_id {
                media_entries = catalog.list_file_locations_for_asset(aid)?;
            } else if !paths.is_empty() {
                let files = resolve_files(paths, exclude_patterns);
                let vol = volume.ok_or_else(|| anyhow::anyhow!("No volume resolved for path mode"))?;
                let vol_id = vol.id.to_string();

                let mut entries = Vec::new();
                for file_path in &files {
                    let ext = file_path
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("");
                    if !is_embedded_xmp_extension(ext) {
                        continue;
                    }
                    let relative_path = match file_path.strip_prefix(&vol.mount_point) {
                        Ok(rp) => rp.to_string_lossy().to_string(),
                        Err(_) => continue,
                    };
                    if let Some((content_hash, _format)) =
                        catalog.find_variant_by_volume_and_path(&vol_id, &relative_path)?
                    {
                        entries.push((content_hash, relative_path, vol_id.clone()));
                    }
                }
                media_entries = entries;
            } else if let Some(vol) = volume {
                let vol_id = vol.id.to_string();
                media_entries = catalog
                    .list_locations_for_volume_under_prefix(&vol_id, "")?
                    .into_iter()
                    .map(|(ch, rp)| (ch, rp, vol_id.clone()))
                    .collect();
            } else {
                let volumes = registry.list()?;
                let mut entries = Vec::new();
                for vol in &volumes {
                    if !vol.is_online {
                        continue;
                    }
                    let vol_id = vol.id.to_string();
                    for (ch, rp) in
                        catalog.list_locations_for_volume_under_prefix(&vol_id, "")?
                    {
                        entries.push((ch, rp, vol_id.clone()));
                    }
                }
                media_entries = entries;
            }

            for (content_hash, relative_path, volume_id_str) in &media_entries {
                // Filter to JPEG/TIFF only
                let ext = Path::new(relative_path)
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("");
                if !is_embedded_xmp_extension(ext) {
                    continue;
                }

                let file_start = Instant::now();

                // Find the volume
                let vol = match all_volumes.iter().find(|v| v.id.to_string() == *volume_id_str) {
                    Some(v) => v,
                    None => {
                        result.skipped += 1;
                        on_file(Path::new(&relative_path), RefreshStatus::Offline, file_start.elapsed());
                        continue;
                    }
                };

                if !vol.is_online {
                    result.skipped += 1;
                    on_file(
                        &vol.mount_point.join(relative_path),
                        RefreshStatus::Offline,
                        file_start.elapsed(),
                    );
                    continue;
                }

                let full_path = vol.mount_point.join(relative_path);

                if !full_path.exists() {
                    result.missing += 1;
                    on_file(&full_path, RefreshStatus::Missing, file_start.elapsed());
                    continue;
                }

                let embedded_xmp = crate::embedded_xmp::extract_embedded_xmp(&full_path);

                // Check if XMP data is non-empty
                if embedded_xmp.keywords.is_empty()
                    && embedded_xmp.description.is_none()
                    && embedded_xmp.source_metadata.is_empty()
                {
                    result.unchanged += 1;
                    on_file(&full_path, RefreshStatus::Unchanged, file_start.elapsed());
                    continue;
                }

                // Load asset and re-apply embedded XMP
                let asset_id_str = match catalog.find_asset_id_by_variant(content_hash)? {
                    Some(id) => id,
                    None => {
                        result.errors.push(format!(
                            "{}: no asset found for variant {}",
                            full_path.display(),
                            content_hash
                        ));
                        continue;
                    }
                };

                let uuid: Uuid = asset_id_str.parse()?;
                let mut asset = metadata_store.load(uuid)?;

                reapply_xmp_data(&embedded_xmp, &mut asset, content_hash);

                if !dry_run {
                    metadata_store.save(&asset)?;
                    catalog.insert_asset(&asset)?;
                    if let Some(v) = asset.variants.iter().find(|v| v.content_hash == *content_hash) {
                        catalog.insert_variant(v)?;
                    }
                }

                result.refreshed += 1;
                on_file(&full_path, RefreshStatus::Refreshed, file_start.elapsed());
            }
        }

        Ok(result)
    }

    /// Bidirectional metadata sync: reads external XMP changes (inbound) and writes pending
    /// DAM changes back (outbound). Detects conflicts where both sides changed.
    ///
    /// Phase 1 (Inbound): For each XMP recipe on online volumes, hash the file. If the hash
    /// differs from stored AND the recipe has no pending_writeback, read external changes.
    /// If both changed, report as conflict.
    ///
    /// Phase 2 (Outbound): Write pending DAM metadata to XMP recipes that weren't conflicting.
    ///
    /// Phase 3 (Media, optional): Re-extract embedded XMP from JPEG/TIFF files.
    pub fn sync_metadata(
        &self,
        volume: Option<&Volume>,
        asset_id: Option<&str>,
        dry_run: bool,
        media: bool,
        _exclude_patterns: &[String],
        on_file: impl Fn(&Path, SyncMetadataStatus, Duration),
    ) -> Result<SyncMetadataResult> {
        let content_store = ContentStore::new(&self.catalog_root);
        let metadata_store = MetadataStore::new(&self.catalog_root);
        let catalog = Catalog::open(&self.catalog_root)?;
        let registry = DeviceRegistry::new(&self.catalog_root);
        let all_volumes = registry.list()?;

        let mut result = SyncMetadataResult {
            inbound: 0,
            outbound: 0,
            unchanged: 0,
            skipped: 0,
            conflicts: 0,
            media_refreshed: 0,
            dry_run,
            errors: Vec::new(),
        };

        // Collect XMP recipes from online volumes
        // Each entry: (recipe_id, content_hash, variant_hash, relative_path, pending_writeback, volume)
        struct RecipeEntry<'a> {
            recipe_id: String,
            stored_hash: String,
            variant_hash: String,
            relative_path: String,
            pending: bool,
            vol: &'a Volume,
        }

        let mut recipes: Vec<RecipeEntry> = Vec::new();

        // Determine which volumes to scan
        let target_volumes: Vec<&Volume> = if let Some(v) = volume {
            vec![v]
        } else {
            all_volumes.iter().filter(|v| v.is_online).collect()
        };

        for vol in &target_volumes {
            if !vol.is_online {
                continue;
            }
            let vol_id = vol.id.to_string();
            let entries = catalog.list_recipes_with_pending_for_volume(&vol_id)?;

            for (rid, ch, vh, rp, pending) in entries {
                // Only XMP files participate in metadata sync
                let is_xmp = rp.to_lowercase().ends_with(".xmp");
                if !is_xmp {
                    continue;
                }

                // Filter by asset if requested
                if let Some(aid) = asset_id {
                    // Look up the asset for this recipe's variant
                    if let Ok(Some(recipe_asset_id)) = catalog.find_asset_id_by_variant(&vh) {
                        if !recipe_asset_id.starts_with(aid) {
                            continue;
                        }
                    } else {
                        continue;
                    }
                }

                recipes.push(RecipeEntry {
                    recipe_id: rid,
                    stored_hash: ch,
                    variant_hash: vh,
                    relative_path: rp,
                    pending,
                    vol,
                });
            }
        }

        // Phase 1: Inbound — read external XMP changes; collect pending recipes for Phase 2
        let mut pending_for_writeback: Vec<(String, String, String, String)> = Vec::new();

        for entry in &recipes {
            let file_start = Instant::now();
            let full_path = entry.vol.mount_point.join(&entry.relative_path);

            if !full_path.exists() {
                result.skipped += 1;
                on_file(&full_path, SyncMetadataStatus::Missing, file_start.elapsed());
                continue;
            }

            let new_hash = match content_store.hash_file(&full_path) {
                Ok(h) => h,
                Err(e) => {
                    result.errors.push(format!("{}: {}", full_path.display(), e));
                    on_file(&full_path, SyncMetadataStatus::Error, file_start.elapsed());
                    continue;
                }
            };

            let disk_changed = new_hash != entry.stored_hash;

            match (disk_changed, entry.pending) {
                (false, false) => {
                    // Nothing to do
                    result.unchanged += 1;
                    on_file(&full_path, SyncMetadataStatus::Unchanged, file_start.elapsed());
                }
                (true, false) => {
                    // External change, no pending DAM edits → inbound (refresh)
                    if !dry_run {
                        if let Err(e) = self.apply_modified_recipe(
                            &catalog,
                            &metadata_store,
                            &entry.recipe_id,
                            &new_hash,
                            &entry.variant_hash,
                            entry.vol,
                            &full_path,
                            &entry.relative_path,
                        ) {
                            result.errors.push(format!("{}: {}", full_path.display(), e));
                            on_file(&full_path, SyncMetadataStatus::Error, file_start.elapsed());
                            continue;
                        }
                    }
                    result.inbound += 1;
                    on_file(&full_path, SyncMetadataStatus::Inbound, file_start.elapsed());
                }
                (false, true) => {
                    // No external change, pending DAM edits → outbound (writeback)
                    // Look up asset_id for the writeback process
                    if let Ok(Some(aid)) = catalog.find_asset_id_by_variant(&entry.variant_hash) {
                        pending_for_writeback.push((
                            entry.recipe_id.clone(),
                            aid,
                            entry.vol.id.to_string(),
                            entry.relative_path.clone(),
                        ));
                    }
                    // Don't count here — will be counted by writeback_process
                }
                (true, true) => {
                    // Both sides changed → conflict
                    result.conflicts += 1;
                    on_file(&full_path, SyncMetadataStatus::Conflict, file_start.elapsed());
                }
            }
        }

        // Phase 2: Outbound — write pending DAM metadata via writeback
        if !pending_for_writeback.is_empty() {
            let engine = crate::query::QueryEngine::new(&self.catalog_root);
            let online: HashMap<uuid::Uuid, PathBuf> = all_volumes
                .iter()
                .filter(|v| v.is_online)
                .map(|v| (v.id, v.mount_point.clone()))
                .collect();

            let wb_result = engine.writeback_process(
                pending_for_writeback,
                &catalog,
                &metadata_store,
                &online,
                &content_store,
                None, // no additional asset filter, already filtered above
                None, // no asset ID set filter
                dry_run,
                false, // log handled by our callback
                None,
            )?;

            result.outbound += wb_result.written as usize;
            result.skipped += wb_result.skipped as usize;
            result.errors.extend(wb_result.errors);
        }

        // Phase 3: Media — re-extract embedded XMP from JPEG/TIFF files (same as refresh --media)
        if media {
            let media_entries: Vec<(String, String, String)>;

            if let Some(aid) = asset_id {
                media_entries = catalog.list_file_locations_for_asset(aid)?;
            } else if let Some(vol) = volume {
                let vol_id = vol.id.to_string();
                media_entries = catalog
                    .list_locations_for_volume_under_prefix(&vol_id, "")?
                    .into_iter()
                    .map(|(ch, rp)| (ch, rp, vol_id.clone()))
                    .collect();
            } else {
                let mut entries = Vec::new();
                for vol in &all_volumes {
                    if !vol.is_online {
                        continue;
                    }
                    let vol_id = vol.id.to_string();
                    for (ch, rp) in catalog.list_locations_for_volume_under_prefix(&vol_id, "")? {
                        entries.push((ch, rp, vol_id.clone()));
                    }
                }
                media_entries = entries;
            }

            for (content_hash, relative_path, volume_id_str) in &media_entries {
                let ext = Path::new(relative_path)
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("");
                if !is_embedded_xmp_extension(ext) {
                    continue;
                }

                let vol = match all_volumes.iter().find(|v| v.id.to_string() == *volume_id_str) {
                    Some(v) if v.is_online => v,
                    _ => continue,
                };

                let full_path = vol.mount_point.join(relative_path);
                if !full_path.exists() {
                    continue;
                }

                let embedded_xmp = crate::embedded_xmp::extract_embedded_xmp(&full_path);

                if embedded_xmp.keywords.is_empty()
                    && embedded_xmp.description.is_none()
                    && embedded_xmp.source_metadata.is_empty()
                {
                    continue;
                }

                let asset_id_str = match catalog.find_asset_id_by_variant(content_hash)? {
                    Some(id) => id,
                    None => continue,
                };

                let uuid: Uuid = asset_id_str.parse()?;
                let mut asset = metadata_store.load(uuid)?;

                reapply_xmp_data(&embedded_xmp, &mut asset, content_hash);

                if !dry_run {
                    metadata_store.save(&asset)?;
                    catalog.insert_asset(&asset)?;
                    if let Some(v) = asset.variants.iter().find(|v| v.content_hash == *content_hash) {
                        catalog.insert_variant(v)?;
                    }
                }

                result.media_refreshed += 1;
            }
        }

        Ok(result)
    }

    /// Fix variant roles: re-role non-RAW variants to Export in assets that have a RAW variant.
    pub fn fix_roles(
        &self,
        paths: &[PathBuf],
        volume_filter: Option<&str>,
        asset_filter: Option<&str>,
        apply: bool,
        on_asset: impl Fn(&str, FixRolesStatus),
    ) -> Result<FixRolesResult> {
        let metadata_store = MetadataStore::new(&self.catalog_root);
        let catalog = Catalog::open(&self.catalog_root)?;
        let registry = DeviceRegistry::new(&self.catalog_root);

        let mut result = FixRolesResult {
            checked: 0,
            fixed: 0,
            variants_fixed: 0,
            already_correct: 0,
            dry_run: !apply,
            errors: Vec::new(),
        };

        // Resolve asset list
        let assets = if let Some(asset_id) = asset_filter {
            let full_id = catalog
                .resolve_asset_id(asset_id)?
                .ok_or_else(|| anyhow::anyhow!("No asset found matching '{asset_id}'"))?;
            let uuid: Uuid = full_id.parse()?;
            vec![metadata_store.load(uuid)?]
        } else if !paths.is_empty() {
            // Path mode: resolve files, find their assets
            let files = resolve_files(paths, &[]);
            let volumes = registry.list()?;
            let content_store = ContentStore::new(&self.catalog_root);
            let mut asset_ids: HashSet<String> = HashSet::new();

            for file_path in &files {
                if !volumes.iter().any(|v| file_path.starts_with(&v.mount_point)) {
                    continue;
                }
                let hash = match content_store.hash_file(file_path) {
                    Ok(h) => h,
                    Err(_) => continue,
                };
                if let Some(aid) = catalog.find_asset_id_by_variant(&hash)? {
                    asset_ids.insert(aid);
                }
            }

            let mut assets = Vec::new();
            for aid in &asset_ids {
                let uuid: Uuid = aid.parse()?;
                assets.push(metadata_store.load(uuid)?);
            }
            assets
        } else {
            // Catalog mode: load all assets
            let summaries = metadata_store.list()?;
            let mut assets = Vec::new();
            for s in &summaries {
                assets.push(metadata_store.load(s.id)?);
            }
            assets
        };

        // Optional volume filter: keep only assets with at least one variant location on that volume
        let volume_filter_resolved = match volume_filter {
            Some(label) => Some(registry.resolve_volume(label)?),
            None => None,
        };

        for mut asset in assets {
            // Volume filter: skip assets without a location on the target volume
            if let Some(ref vol) = volume_filter_resolved {
                let has_location = asset.variants.iter().any(|v| {
                    v.locations.iter().any(|loc| loc.volume_id == vol.id)
                });
                if !has_location {
                    continue;
                }
            }

            result.checked += 1;

            // Skip single-variant assets
            if asset.variants.len() < 2 {
                result.already_correct += 1;
                on_asset(
                    asset.name.as_deref().unwrap_or(&asset.id.to_string()),
                    FixRolesStatus::AlreadyCorrect,
                );
                continue;
            }

            // Check if any variant is RAW
            let has_raw = asset.variants.iter().any(|v| is_raw_extension(&v.format));
            if !has_raw {
                result.already_correct += 1;
                on_asset(
                    asset.name.as_deref().unwrap_or(&asset.id.to_string()),
                    FixRolesStatus::AlreadyCorrect,
                );
                continue;
            }

            // Find non-RAW variants with role == Original
            let fixable: Vec<usize> = asset
                .variants
                .iter()
                .enumerate()
                .filter(|(_, v)| !is_raw_extension(&v.format) && v.role == VariantRole::Original)
                .map(|(i, _)| i)
                .collect();

            if fixable.is_empty() {
                result.already_correct += 1;
                on_asset(
                    asset.name.as_deref().unwrap_or(&asset.id.to_string()),
                    FixRolesStatus::AlreadyCorrect,
                );
                continue;
            }

            if apply {
                for &idx in &fixable {
                    asset.variants[idx].role = VariantRole::Alternate;
                    catalog.update_variant_role(
                        &asset.variants[idx].content_hash,
                        "alternate",
                    )?;
                }
                metadata_store.save(&asset)?;
                catalog.update_denormalized_variant_columns(&asset)?;
            }

            result.fixed += 1;
            result.variants_fixed += fixable.len();
            on_asset(
                asset.name.as_deref().unwrap_or(&asset.id.to_string()),
                FixRolesStatus::Fixed,
            );
        }

        Ok(result)
    }

    /// Fix asset dates by examining variant metadata and file modification times.
    ///
    /// For each asset, finds the oldest plausible date from:
    /// 1. EXIF DateTimeOriginal stored in variant `source_metadata["date_taken"]`
    /// 2. Re-extracted EXIF from files on disk (for assets imported before date_taken was stored)
    /// 3. File modification time on disk
    ///
    /// Sources 2 and 3 require the volume to be online. Assets whose only locations
    /// are on offline volumes are counted as `skipped_offline`.
    ///
    /// When applying, also backfills `date_taken` into variant source_metadata so
    /// future runs work from metadata alone without needing the volume online.
    ///
    /// Report-only by default; pass `apply=true` to update sidecars and catalog.
    pub fn fix_dates(
        &self,
        volume_filter: Option<&str>,
        asset_filter: Option<&str>,
        apply: bool,
        on_asset: impl Fn(&str, FixDatesStatus, Option<&str>),
    ) -> Result<FixDatesResult> {
        use chrono::{DateTime, NaiveDateTime, Utc};

        let metadata_store = MetadataStore::new(&self.catalog_root);
        let catalog = Catalog::open(&self.catalog_root)?;
        let registry = DeviceRegistry::new(&self.catalog_root);
        let volumes = registry.list()?;

        // Collect offline volume labels for warnings
        let offline_volumes: Vec<String> = volumes.iter()
            .filter(|v| !v.is_online)
            .map(|v| v.label.clone())
            .collect();

        let mut result = FixDatesResult {
            checked: 0,
            fixed: 0,
            already_correct: 0,
            no_date: 0,
            skipped_offline: 0,
            dry_run: !apply,
            offline_volumes: offline_volumes.clone(),
            errors: Vec::new(),
        };

        // Resolve asset list
        let assets = if let Some(asset_id) = asset_filter {
            let full_id = catalog
                .resolve_asset_id(asset_id)?
                .ok_or_else(|| anyhow::anyhow!("No asset found matching '{asset_id}'"))?;
            let uuid: Uuid = full_id.parse()?;
            vec![metadata_store.load(uuid)?]
        } else {
            let summaries = metadata_store.list()?;
            let mut assets = Vec::new();
            for s in &summaries {
                assets.push(metadata_store.load(s.id)?);
            }
            assets
        };

        // Optional volume filter
        let volume_filter_resolved = match volume_filter {
            Some(label) => Some(registry.resolve_volume(label)?),
            None => None,
        };

        for mut asset in assets {
            // Volume filter: skip assets without a location on the target volume
            if let Some(ref vol) = volume_filter_resolved {
                let has_location = asset.variants.iter().any(|v| {
                    v.locations.iter().any(|loc| loc.volume_id == vol.id)
                });
                if !has_location {
                    continue;
                }
            }

            result.checked += 1;
            let asset_name = asset.name.clone()
                .unwrap_or_else(|| asset.variants.first()
                    .map(|v| v.original_filename.clone())
                    .unwrap_or_else(|| asset.id.to_string()));

            // Collect candidate dates from all variants
            let mut candidates: Vec<DateTime<Utc>> = Vec::new();
            let mut has_metadata_date = false;
            let mut all_offline = true;
            let mut backfill_dates: Vec<(usize, DateTime<Utc>)> = Vec::new();

            for (vi, variant) in asset.variants.iter().enumerate() {
                // 1. Check source_metadata for stored date_taken
                if let Some(date_str) = variant.source_metadata.get("date_taken") {
                    if let Ok(dt) = DateTime::parse_from_rfc3339(date_str) {
                        candidates.push(dt.with_timezone(&Utc));
                        has_metadata_date = true;
                    } else {
                        let s = date_str.trim_matches('"');
                        if let Ok(ndt) = NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S")
                            .or_else(|_| NaiveDateTime::parse_from_str(s, "%Y:%m:%d %H:%M:%S"))
                        {
                            candidates.push(ndt.and_utc());
                            has_metadata_date = true;
                        }
                    }
                }

                // 2. Check files on disk for online volumes
                for loc in &variant.locations {
                    if let Some(vol) = volumes.iter().find(|v| v.id == loc.volume_id) {
                        if vol.is_online {
                            all_offline = false;
                            let full_path = vol.mount_point.join(&loc.relative_path);

                            // Re-extract EXIF if no date_taken in metadata
                            if !has_metadata_date {
                                let exif_data = crate::exif_reader::extract(&full_path);
                                if let Some(dt) = exif_data.date_taken {
                                    candidates.push(dt);
                                    // Remember to backfill this date into source_metadata
                                    backfill_dates.push((vi, dt));
                                }
                            }

                            // File mtime as fallback
                            if let Some(mtime) = file_mtime(&full_path) {
                                candidates.push(mtime);
                            }
                        }
                    }
                }
            }

            // If no metadata date and all locations are offline, skip with specific status
            if candidates.is_empty() && all_offline && !asset.variants.is_empty() {
                // Check if the asset actually has locations on offline volumes
                let has_offline_locations = asset.variants.iter().any(|v| {
                    v.locations.iter().any(|loc| {
                        volumes.iter().any(|vol| vol.id == loc.volume_id && !vol.is_online)
                    })
                });
                if has_offline_locations {
                    result.skipped_offline += 1;
                    on_asset(&asset_name, FixDatesStatus::SkippedOffline, None);
                    continue;
                }
            }

            if candidates.is_empty() {
                result.no_date += 1;
                on_asset(&asset_name, FixDatesStatus::NoDate, None);
                continue;
            }

            // Pick the oldest date
            let oldest = candidates.into_iter().min().unwrap();

            // Compare with current created_at (allow 1 second tolerance for rounding)
            let diff = (asset.created_at - oldest).num_seconds().abs();
            if diff <= 1 {
                // Even if date is correct, backfill date_taken into source_metadata if missing
                if apply && !backfill_dates.is_empty() {
                    for (vi, dt) in &backfill_dates {
                        asset.variants[*vi].source_metadata.insert(
                            "date_taken".to_string(),
                            dt.to_rfc3339(),
                        );
                    }
                    metadata_store.save(&asset)?;
                    catalog.insert_asset(&asset)?;
                }
                result.already_correct += 1;
                on_asset(&asset_name, FixDatesStatus::AlreadyCorrect, None);
                continue;
            }

            let old_date = asset.created_at.format("%Y-%m-%d %H:%M:%S").to_string();
            let new_date = oldest.format("%Y-%m-%d %H:%M:%S").to_string();
            let detail = format!("{old_date} → {new_date}");

            if apply {
                asset.created_at = oldest;
                // Backfill date_taken into source_metadata
                for (vi, dt) in &backfill_dates {
                    asset.variants[*vi].source_metadata.insert(
                        "date_taken".to_string(),
                        dt.to_rfc3339(),
                    );
                }
                metadata_store.save(&asset)?;
                catalog.update_asset_created_at(&asset.id.to_string(), &oldest)?;
                // Also update catalog variant metadata if we backfilled
                if !backfill_dates.is_empty() {
                    catalog.insert_asset(&asset)?;
                }
            }

            result.fixed += 1;
            on_asset(&asset_name, FixDatesStatus::Fixed, Some(&detail));
        }

        Ok(result)
    }

    /// Re-attach recipe files that were imported as standalone assets.
    /// Finds single-variant assets with recipe extensions, tries to match them
    /// to a parent variant by stem + directory, and converts them to Recipe records.
    pub fn fix_recipes(
        &self,
        volume_filter: Option<&str>,
        asset_filter: Option<&str>,
        apply: bool,
        on_asset: impl Fn(&str, FixRecipesStatus),
    ) -> Result<FixRecipesResult> {
        let metadata_store = MetadataStore::new(&self.catalog_root);
        let catalog = Catalog::open(&self.catalog_root)?;
        let registry = DeviceRegistry::new(&self.catalog_root);

        let mut result = FixRecipesResult {
            checked: 0,
            reattached: 0,
            no_parent: 0,
            skipped: 0,
            dry_run: !apply,
            errors: Vec::new(),
        };

        // Resolve optional volume filter
        let volume_id = match volume_filter {
            Some(label) => Some(registry.resolve_volume(label)?.id.to_string()),
            None => None,
        };

        // Resolve optional asset filter
        let asset_id = match asset_filter {
            Some(prefix) => Some(
                catalog
                    .resolve_asset_id(prefix)?
                    .ok_or_else(|| anyhow::anyhow!("No asset found matching '{prefix}'"))?,
            ),
            None => None,
        };

        let candidates = catalog.list_recipe_only_assets(
            volume_id.as_deref(),
            asset_id.as_deref(),
        )?;

        for (standalone_id, content_hash, format) in &candidates {
            result.checked += 1;

            // Load the standalone asset
            let standalone_uuid: Uuid = standalone_id.parse()?;
            let standalone = match metadata_store.load(standalone_uuid) {
                Ok(a) => a,
                Err(e) => {
                    result.errors.push(format!("{standalone_id}: {e}"));
                    continue;
                }
            };

            let asset_name = standalone
                .name
                .clone()
                .unwrap_or_else(|| {
                    standalone
                        .variants
                        .first()
                        .map(|v| v.original_filename.clone())
                        .unwrap_or_else(|| standalone_id.clone())
                });

            // Get the variant's file location to determine stem + directory
            let variant = match standalone.variants.first() {
                Some(v) => v,
                None => {
                    result.skipped += 1;
                    on_asset(&asset_name, FixRecipesStatus::Skipped);
                    continue;
                }
            };

            let location = match variant.locations.first() {
                Some(l) => l,
                None => {
                    // No file location — can't determine stem/directory
                    result.skipped += 1;
                    on_asset(&asset_name, FixRecipesStatus::Skipped);
                    continue;
                }
            };

            let rel_path = &location.relative_path;
            let stem = rel_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("");
            let dir_prefix = rel_path
                .parent()
                .unwrap_or_else(|| Path::new(""))
                .to_string_lossy();
            let vol_id_str = location.volume_id.to_string();

            // Try to find parent variant by stem + directory (exclude self)
            let exclude = Some(standalone_id.as_str());
            let mut parent = catalog.find_variant_hash_by_stem_and_directory(
                stem,
                &dir_prefix,
                &vol_id_str,
                exclude,
            )?;

            // If not found and stem contains a dot (compound extension like DSC_001.NRW.xmp),
            // strip the last extension and retry
            if parent.is_none() {
                if let Some(dot_pos) = stem.rfind('.') {
                    let stripped_stem = &stem[..dot_pos];
                    parent = catalog.find_variant_hash_by_stem_and_directory(
                        stripped_stem,
                        &dir_prefix,
                        &vol_id_str,
                        exclude,
                    )?;
                }
            }

            let (parent_hash, parent_asset_id) = match parent {
                Some(p) => p,
                None => {
                    result.no_parent += 1;
                    on_asset(&asset_name, FixRecipesStatus::NoParentFound);
                    continue;
                }
            };

            if apply {
                // Load parent asset
                let parent_uuid: Uuid = parent_asset_id.parse()?;
                let mut parent_asset = metadata_store.load(parent_uuid)?;

                // Create recipe record
                let recipe = Recipe {
                    id: Uuid::new_v4(),
                    variant_hash: parent_hash.clone(),
                    software: determine_recipe_software(format).to_string(),
                    recipe_type: RecipeType::Sidecar,
                    content_hash: content_hash.clone(),
                    location: location.clone(),
                    pending_writeback: false,
                };

                // Apply XMP metadata if this is an XMP file
                if format.eq_ignore_ascii_case("xmp") {
                    // Find the file on disk to extract XMP
                    let volumes = registry.list()?;
                    let vol = volumes.iter().find(|v| v.id == location.volume_id);
                    if let Some(vol) = vol {
                        if vol.is_online {
                            let file_path = vol.mount_point.join(rel_path);
                            if file_path.exists() {
                                let xmp = crate::xmp_reader::extract(&file_path);
                                apply_xmp_data(&xmp, &mut parent_asset, &parent_hash);
                            }
                        }
                    }
                }

                parent_asset.recipes.push(recipe.clone());
                metadata_store.save(&parent_asset)?;
                catalog.insert_asset(&parent_asset)?;
                if let Some(v) = parent_asset
                    .variants
                    .iter()
                    .find(|v| v.content_hash == parent_hash)
                {
                    catalog.insert_variant(v)?;
                }
                catalog.insert_recipe(&recipe)?;
                catalog.update_denormalized_variant_columns(&parent_asset)?;

                // Delete standalone asset: recipes → locations → variants → asset → sidecar
                let id_str = standalone_id.as_str();
                catalog.delete_recipes_for_asset(id_str)?;
                catalog.delete_file_locations_for_asset(id_str)?;
                catalog.delete_variants_for_asset(id_str)?;
                catalog.delete_asset(id_str)?;
                metadata_store.delete(standalone_uuid)?;
            }

            result.reattached += 1;
            on_asset(&asset_name, FixRecipesStatus::Reattached);
        }

        Ok(result)
    }

    /// Export files matching a search query to a target directory.
    ///
    /// Searches the catalog, resolves file locations on online volumes, and copies
    /// (or symlinks) files to the target directory. By default exports only the best
    /// variant per asset; `all_variants` exports every variant. `include_sidecars`
    /// also copies recipe files. `dry_run` reports the plan without writing files.
    /// Build an export plan: resolve assets, find online file locations, compute target paths.
    ///
    /// Returns `(plan, assets_matched, errors)`. The plan entries have `target_path` set
    /// relative to `target_base` (for directory export) or as ZIP entry names.
    pub fn build_export_plan(
        &self,
        asset_ids: &[String],
        target_base: &Path,
        layout: ExportLayout,
        all_variants: bool,
        include_sidecars: bool,
    ) -> Result<(Vec<ExportFilePlan>, usize, Vec<String>)> {
        use crate::catalog::Catalog;
        use crate::models::variant::best_preview_index_details;

        let catalog = Catalog::open(&self.catalog_root)?;
        let registry = DeviceRegistry::new(&self.catalog_root);

        let assets_matched = asset_ids.len();

        // Load volumes for resolving online mount points
        let volumes = registry.list()?;
        let online_volumes: std::collections::HashMap<String, &crate::models::Volume> = volumes
            .iter()
            .filter(|v| v.is_online)
            .map(|v| (v.id.to_string(), v))
            .collect();

        let mut involved_volume_ids: HashSet<String> = HashSet::new();
        let mut plan: Vec<ExportFilePlan> = Vec::new();
        let mut planned_hashes: HashSet<String> = HashSet::new();
        let mut flat_seen: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        let mut errors: Vec<String> = Vec::new();

        for asset_id in asset_ids {
            let details = match catalog.load_asset_details(asset_id)? {
                Some(d) => d,
                None => {
                    errors.push(format!("Asset {} not found in catalog", &asset_id[..8]));
                    continue;
                }
            };

            let variant_indices: Vec<usize> = if all_variants {
                (0..details.variants.len()).collect()
            } else {
                match best_preview_index_details(&details.variants) {
                    Some(i) => vec![i],
                    None => {
                        errors.push(format!("Asset {} has no variants", &asset_id[..8]));
                        continue;
                    }
                }
            };

            for vi in &variant_indices {
                let variant = &details.variants[*vi];
                if planned_hashes.contains(&variant.content_hash) {
                    continue;
                }

                let loc = variant.locations.iter().find(|l| {
                    online_volumes.contains_key(&l.volume_id)
                });
                let loc = match loc {
                    Some(l) => l,
                    None => {
                        errors.push(format!(
                            "Asset {} variant {} — all locations offline",
                            &asset_id[..8],
                            &variant.content_hash[..12]
                        ));
                        continue;
                    }
                };

                let vol = online_volumes[&loc.volume_id];
                let source_path = vol.mount_point.join(&loc.relative_path);

                let target_path = match layout {
                    ExportLayout::Flat => {
                        resolve_flat_target(
                            target_base,
                            &variant.original_filename,
                            &variant.content_hash,
                            &mut flat_seen,
                        )
                    }
                    ExportLayout::Mirror => {
                        involved_volume_ids.insert(loc.volume_id.clone());
                        target_base.join(&loc.relative_path)
                    }
                };

                planned_hashes.insert(variant.content_hash.clone());
                plan.push(ExportFilePlan {
                    asset_id: asset_id.clone(),
                    content_hash: variant.content_hash.clone(),
                    source_path,
                    target_path,
                    file_size: variant.file_size,
                    is_sidecar: false,
                });
            }

            if include_sidecars {
                for recipe in &details.recipes {
                    let (vol_id, rel_path) = match (&recipe.volume_id, &recipe.relative_path) {
                        (Some(vid), Some(rp)) => (vid, rp),
                        _ => continue,
                    };

                    if planned_hashes.contains(&recipe.content_hash) {
                        continue;
                    }

                    let vol = match online_volumes.get(vol_id.as_str()) {
                        Some(v) => v,
                        None => continue,
                    };

                    let source_path = vol.mount_point.join(rel_path);
                    let filename = Path::new(rel_path)
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();

                    let target_path = match layout {
                        ExportLayout::Flat => {
                            resolve_flat_target(
                                target_base,
                                &filename,
                                &recipe.content_hash,
                                &mut flat_seen,
                            )
                        }
                        ExportLayout::Mirror => {
                            involved_volume_ids.insert(vol_id.clone());
                            target_base.join(rel_path)
                        }
                    };

                    let file_size = source_path.metadata().map(|m| m.len()).unwrap_or(0);
                    planned_hashes.insert(recipe.content_hash.clone());
                    plan.push(ExportFilePlan {
                        asset_id: asset_id.clone(),
                        content_hash: recipe.content_hash.clone(),
                        source_path,
                        target_path,
                        file_size,
                        is_sidecar: true,
                    });
                }
            }
        }

        // Mirror layout: if multiple volumes involved, prefix with volume label
        if layout == ExportLayout::Mirror && involved_volume_ids.len() > 1 {
            for entry in &mut plan {
                for vol in &volumes {
                    if vol.is_online
                        && entry.source_path.starts_with(&vol.mount_point)
                    {
                        if let Ok(rel) = entry.source_path.strip_prefix(&vol.mount_point) {
                            entry.target_path = target_base.join(&vol.label).join(rel);
                        }
                        break;
                    }
                }
            }
        }

        Ok((plan, assets_matched, errors))
    }

    pub fn export(
        &self,
        query: &str,
        target_dir: &Path,
        layout: ExportLayout,
        symlink: bool,
        all_variants: bool,
        include_sidecars: bool,
        dry_run: bool,
        overwrite: bool,
        on_file: impl Fn(&Path, &ExportStatus, Duration),
    ) -> Result<ExportResult> {
        let engine = crate::query::QueryEngine::new(&self.catalog_root);
        let content_store = ContentStore::new(&self.catalog_root);

        // Phase 1: Search
        let search_results = engine.search(query)?;
        let assets_matched = search_results.len();

        if assets_matched == 0 {
            return Ok(ExportResult {
                dry_run,
                assets_matched: 0,
                files_exported: 0,
                files_skipped: 0,
                sidecars_exported: 0,
                total_bytes: 0,
                errors: Vec::new(),
            });
        }

        // Phase 2: Build plan
        let asset_ids: Vec<String> = search_results.iter().map(|r| r.asset_id.clone()).collect();
        let (plan, _, errors) = self.build_export_plan(&asset_ids, target_dir, layout, all_variants, include_sidecars)?;

        // Phase 3: Execute or dry-run
        let mut result = ExportResult {
            dry_run,
            assets_matched,
            files_exported: 0,
            files_skipped: 0,
            sidecars_exported: 0,
            total_bytes: 0,
            errors,
        };

        for entry in &plan {
            let file_start = Instant::now();

            if dry_run {
                if entry.is_sidecar {
                    result.sidecars_exported += 1;
                } else {
                    result.files_exported += 1;
                }
                result.total_bytes += entry.file_size;
                on_file(&entry.target_path, &ExportStatus::Copied, file_start.elapsed());
                continue;
            }

            // Check if target already exists with matching hash
            if !overwrite && entry.target_path.exists() {
                match content_store.hash_file(&entry.target_path) {
                    Ok(existing_hash) if existing_hash == entry.content_hash => {
                        result.files_skipped += 1;
                        on_file(
                            &entry.target_path,
                            &ExportStatus::Skipped,
                            file_start.elapsed(),
                        );
                        continue;
                    }
                    _ => {} // different hash or error — proceed with copy/overwrite
                }
            }

            // Create parent directories
            if let Some(parent) = entry.target_path.parent() {
                if let Err(e) = std::fs::create_dir_all(parent) {
                    let msg = format!(
                        "{} — failed to create directory: {}",
                        entry.target_path.display(),
                        e
                    );
                    result.errors.push(msg.clone());
                    on_file(&entry.target_path, &ExportStatus::Error(msg), file_start.elapsed());
                    continue;
                }
            }

            if symlink {
                match create_symlink(&entry.source_path, &entry.target_path) {
                    Ok(()) => {
                        if entry.is_sidecar {
                            result.sidecars_exported += 1;
                        } else {
                            result.files_exported += 1;
                        }
                        result.total_bytes += entry.file_size;
                        on_file(
                            &entry.target_path,
                            &ExportStatus::Linked,
                            file_start.elapsed(),
                        );
                    }
                    Err(e) => {
                        let msg = format!(
                            "{} — symlink failed: {}",
                            entry.target_path.display(),
                            e
                        );
                        result.errors.push(msg.clone());
                        on_file(
                            &entry.target_path,
                            &ExportStatus::Error(msg),
                            file_start.elapsed(),
                        );
                    }
                }
            } else {
                match content_store.copy_and_verify(
                    &entry.source_path,
                    &entry.target_path,
                    &entry.content_hash,
                ) {
                    Ok(()) => {
                        if entry.is_sidecar {
                            result.sidecars_exported += 1;
                        } else {
                            result.files_exported += 1;
                        }
                        result.total_bytes += entry.file_size;
                        on_file(
                            &entry.target_path,
                            &ExportStatus::Copied,
                            file_start.elapsed(),
                        );
                    }
                    Err(e) => {
                        let msg = format!(
                            "{} — copy failed: {}",
                            entry.target_path.display(),
                            e
                        );
                        result.errors.push(msg.clone());
                        on_file(
                            &entry.target_path,
                            &ExportStatus::Error(msg),
                            file_start.elapsed(),
                        );
                    }
                }
            }
        }

        Ok(result)
    }

    /// Export matching assets as a ZIP archive.
    pub fn export_zip(
        &self,
        query: &str,
        zip_path: &Path,
        layout: ExportLayout,
        all_variants: bool,
        include_sidecars: bool,
        on_file: impl Fn(&Path, &ExportStatus, Duration),
    ) -> Result<ExportResult> {
        let engine = crate::query::QueryEngine::new(&self.catalog_root);

        let search_results = engine.search(query)?;
        let assets_matched = search_results.len();

        if assets_matched == 0 {
            return Ok(ExportResult {
                dry_run: false,
                assets_matched: 0,
                files_exported: 0,
                files_skipped: 0,
                sidecars_exported: 0,
                total_bytes: 0,
                errors: Vec::new(),
            });
        }

        let asset_ids: Vec<String> = search_results.iter().map(|r| r.asset_id.clone()).collect();
        self.export_zip_for_ids(&asset_ids, zip_path, layout, all_variants, include_sidecars, on_file)
    }

    /// Export specific asset IDs as a ZIP archive.
    pub fn export_zip_for_ids(
        &self,
        asset_ids: &[String],
        zip_path: &Path,
        layout: ExportLayout,
        all_variants: bool,
        include_sidecars: bool,
        on_file: impl Fn(&Path, &ExportStatus, Duration),
    ) -> Result<ExportResult> {
        use std::io::Write;
        use zip::write::{SimpleFileOptions, ZipWriter};

        let dummy_base = Path::new("");
        let (plan, assets_matched, errors) =
            self.build_export_plan(asset_ids, dummy_base, layout, all_variants, include_sidecars)?;

        let mut result = ExportResult {
            dry_run: false,
            assets_matched,
            files_exported: 0,
            files_skipped: 0,
            sidecars_exported: 0,
            total_bytes: 0,
            errors,
        };

        if plan.is_empty() {
            return Ok(result);
        }

        if let Some(parent) = zip_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let file = std::fs::File::create(zip_path)?;
        let writer = std::io::BufWriter::with_capacity(1024 * 1024, file);
        let mut zip = ZipWriter::new(writer);
        let options = SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);

        for entry in &plan {
            let file_start = Instant::now();
            let entry_name = entry.target_path.to_string_lossy().replace('\\', "/");
            let entry_name = entry_name.trim_start_matches('/').trim_start_matches("./");

            if let Err(e) = zip.start_file(entry_name, options) {
                let msg = format!("{entry_name} — zip entry failed: {e}");
                result.errors.push(msg.clone());
                on_file(&entry.target_path, &ExportStatus::Error(msg), file_start.elapsed());
                continue;
            }

            let src = match std::fs::File::open(&entry.source_path) {
                Ok(f) => f,
                Err(e) => {
                    let msg = format!("{entry_name} — open failed: {e}");
                    result.errors.push(msg.clone());
                    on_file(&entry.target_path, &ExportStatus::Error(msg), file_start.elapsed());
                    continue;
                }
            };
            let mut reader = std::io::BufReader::with_capacity(256 * 1024, src);
            let mut buf = vec![0u8; 256 * 1024];
            loop {
                let n = match std::io::Read::read(&mut reader, &mut buf) {
                    Ok(0) => break,
                    Ok(n) => n,
                    Err(_) => break,
                };
                if zip.write_all(&buf[..n]).is_err() {
                    break;
                }
            }

            if entry.is_sidecar {
                result.sidecars_exported += 1;
            } else {
                result.files_exported += 1;
            }
            result.total_bytes += entry.file_size;
            on_file(&entry.target_path, &ExportStatus::Copied, file_start.elapsed());
        }

        zip.finish().map_err(|e| anyhow::anyhow!("Failed to finalize ZIP: {e}"))?;
        Ok(result)
    }

    /// Auto-tag assets using SigLIP zero-shot classification.
    #[cfg(feature = "ai")]
    pub fn auto_tag(
        &self,
        query: Option<&str>,
        asset_id: Option<&str>,
        volume: Option<&str>,
        threshold: f32,
        labels: &[String],
        prompt_template: &str,
        apply: bool,
        model_dir: &std::path::Path,
        model_id: &str,
        execution_provider: &str,
        on_asset: impl Fn(&str, &crate::ai::AutoTagStatus, Duration),
    ) -> Result<crate::ai::AutoTagResult> {
        use crate::ai::{self, AutoTagResult, AutoTagStatus, AssetSuggestions, SigLipModel};
        use crate::catalog::Catalog;
        use crate::embedding_store::EmbeddingStore;
        use crate::preview::PreviewGenerator;

        let catalog = Catalog::open(&self.catalog_root)?;
        let engine = crate::query::QueryEngine::new(&self.catalog_root);
        let preview_gen = PreviewGenerator::new(&self.catalog_root, self.verbosity, &self.preview_config);
        let registry = DeviceRegistry::new(&self.catalog_root);
        let volumes = registry.list()?;
        let online_volumes: std::collections::HashMap<String, &crate::models::Volume> = volumes
            .iter()
            .filter(|v| v.is_online)
            .map(|v| (v.id.to_string(), v))
            .collect();

        // Load model
        let mut model = SigLipModel::load_with_provider(model_dir, model_id, self.verbosity, execution_provider)?;

        // Prepare label texts with prompt template
        let prompted_labels: Vec<String> = labels
            .iter()
            .map(|l| ai::apply_prompt_template(prompt_template, l))
            .collect();

        // Pre-encode all label texts
        let label_embs = model.encode_texts(&prompted_labels)?;

        // Resolve target assets
        let asset_ids: Vec<String> = if let Some(id) = asset_id {
            let full_id = catalog
                .resolve_asset_id(id)?
                .ok_or_else(|| anyhow::anyhow!("No asset found matching '{id}'"))?;
            vec![full_id]
        } else {
            let q = if let Some(query) = query {
                let volume_part = volume.map(|v| format!(" volume:{v}")).unwrap_or_default();
                format!("{query}{volume_part}")
            } else if let Some(v) = volume {
                format!("volume:{v}")
            } else {
                "*".to_string()
            };
            let results = engine.search(&q)?;
            results.into_iter().map(|r| r.asset_id).collect()
        };

        let mut result = AutoTagResult {
            assets_processed: 0,
            assets_skipped: 0,
            tags_suggested: 0,
            tags_applied: 0,
            errors: Vec::new(),
            dry_run: !apply,
            suggestions: Vec::new(),
        };

        // Initialize embedding store
        let _ = EmbeddingStore::initialize(catalog.conn());
        let emb_store = EmbeddingStore::new(catalog.conn());

        for aid in &asset_ids {
            let asset_start = Instant::now();

            // Load asset details to find preview/image file
            let details = match catalog.load_asset_details(aid)? {
                Some(d) => d,
                None => {
                    let msg = format!("Asset {} not found", &aid[..8.min(aid.len())]);
                    result.errors.push(msg.clone());
                    on_asset(aid, &AutoTagStatus::Error(msg), asset_start.elapsed());
                    continue;
                }
            };

            // Find an image to process: smart preview > regular preview > original on online volume
            let image_path = self.find_image_for_ai(&details, &preview_gen, &online_volumes);

            let image_path = match image_path {
                Some(p) => p,
                None => {
                    let msg = format!(
                        "No processable image for asset {}",
                        &aid[..8.min(aid.len())]
                    );
                    result.assets_skipped += 1;
                    on_asset(aid, &AutoTagStatus::Skipped(msg), asset_start.elapsed());
                    continue;
                }
            };

            // Check if the image format is supported
            let ext = image_path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("");
            if !ai::is_supported_image(ext) {
                let msg = format!(
                    "Unsupported format '{}' for asset {}",
                    ext,
                    &aid[..8.min(aid.len())]
                );
                result.assets_skipped += 1;
                on_asset(aid, &AutoTagStatus::Skipped(msg), asset_start.elapsed());
                continue;
            }

            // Encode image
            let image_emb = match model.encode_image(&image_path) {
                Ok(emb) => emb,
                Err(e) => {
                    let msg = format!(
                        "Failed to encode image for {}: {e}",
                        &aid[..8.min(aid.len())]
                    );
                    result.errors.push(msg.clone());
                    on_asset(aid, &AutoTagStatus::Error(msg), asset_start.elapsed());
                    continue;
                }
            };

            // Store embedding
            if let Err(e) = emb_store.store(aid, &image_emb, model_id) {
                eprintln!("Warning: failed to store embedding for {}: {e}", &aid[..8.min(aid.len())]);
            }

            // Classify
            let suggestions = if self.verbosity.debug {
                eprintln!("  [debug] asset {} — image: {}", &aid[..8.min(aid.len())], image_path.display());
                let norm: f32 = image_emb.iter().map(|x| x * x).sum::<f32>().sqrt();
                eprintln!("  [debug] embedding norm: {norm:.6} (expected ~1.0 for L2-normalized)");
                model.classify_debug(&image_emb, labels, &label_embs, threshold)
            } else {
                model.classify(&image_emb, labels, &label_embs, threshold)
            };

            // Filter out tags already on the asset
            let existing_tags: HashSet<String> = details
                .tags
                .iter()
                .map(|t| t.to_lowercase())
                .collect();
            let new_suggestions: Vec<_> = suggestions
                .into_iter()
                .filter(|s| {
                    let dominated = existing_tags.contains(&s.tag.to_lowercase());
                    if dominated && self.verbosity.debug {
                        eprintln!("  [debug] skipping '{}' ({:.2}%) — tag already exists on asset", s.tag, s.confidence * 100.0);
                    }
                    !dominated
                })
                .collect();

            result.tags_suggested += new_suggestions.len();

            if apply && !new_suggestions.is_empty() {
                let new_tags: Vec<String> = new_suggestions.iter().map(|s| s.tag.clone()).collect();
                match engine.tag(aid, &new_tags, false) {
                    Ok(_) => {
                        result.tags_applied += new_tags.len();
                    }
                    Err(e) => {
                        let msg = format!(
                            "Failed to apply tags to {}: {e}",
                            &aid[..8.min(aid.len())]
                        );
                        result.errors.push(msg.clone());
                    }
                }
            }

            result.assets_processed += 1;
            result.suggestions.push(AssetSuggestions {
                asset_id: aid.clone(),
                suggested_tags: new_suggestions.clone(),
                applied: apply,
            });

            let status = if apply {
                AutoTagStatus::Applied(new_suggestions)
            } else {
                AutoTagStatus::Suggested(new_suggestions)
            };
            on_asset(aid, &status, asset_start.elapsed());
        }

        Ok(result)
    }

    /// Find the best image file for processing.
    /// Priority: smart preview > regular preview > original on online volume.
    /// The `is_supported` predicate controls which original file extensions are accepted.
    fn find_image_for_processing(
        &self,
        details: &crate::catalog::AssetDetails,
        preview_gen: &crate::preview::PreviewGenerator,
        online_volumes: &std::collections::HashMap<String, &crate::models::Volume>,
        is_supported: impl Fn(&str) -> bool,
    ) -> Option<PathBuf> {
        // Try smart preview of best variant
        if let Some(best) = crate::models::variant::best_preview_index_details(&details.variants) {
            let variant = &details.variants[best];
            let smart_path = preview_gen.smart_preview_path(&variant.content_hash);
            if smart_path.exists() {
                return Some(smart_path);
            }
            let preview_path = preview_gen.preview_path(&variant.content_hash);
            if preview_path.exists() {
                return Some(preview_path);
            }
        }

        // Fall back to any preview we can find
        for variant in &details.variants {
            let smart_path = preview_gen.smart_preview_path(&variant.content_hash);
            if smart_path.exists() {
                return Some(smart_path);
            }
            let preview_path = preview_gen.preview_path(&variant.content_hash);
            if preview_path.exists() {
                return Some(preview_path);
            }
        }

        // Fall back to original file on an online volume
        for variant in &details.variants {
            for loc in &variant.locations {
                if let Some(vol) = online_volumes.get(&loc.volume_id) {
                    let full_path = vol.mount_point.join(&loc.relative_path);
                    if full_path.exists() {
                        let ext = full_path
                            .extension()
                            .and_then(|e| e.to_str())
                            .unwrap_or("");
                        if is_supported(ext) {
                            return Some(full_path);
                        }
                    }
                }
            }
        }

        None
    }

    /// Find the best image file for AI embedding/detection.
    #[cfg(feature = "ai")]
    pub fn find_image_for_ai(
        &self,
        details: &crate::catalog::AssetDetails,
        preview_gen: &crate::preview::PreviewGenerator,
        online_volumes: &std::collections::HashMap<String, &crate::models::Volume>,
    ) -> Option<PathBuf> {
        self.find_image_for_processing(details, preview_gen, online_volumes, |ext| {
            crate::ai::is_supported_image(ext)
        })
    }

    /// Find the best image file for VLM processing.
    pub fn find_image_for_vlm(
        &self,
        details: &crate::catalog::AssetDetails,
        preview_gen: &crate::preview::PreviewGenerator,
        online_volumes: &std::collections::HashMap<String, &crate::models::Volume>,
    ) -> Option<PathBuf> {
        self.find_image_for_processing(details, preview_gen, online_volumes, |ext| {
            matches!(
                ext.to_lowercase().as_str(),
                "jpg" | "jpeg" | "png" | "webp" | "tif" | "tiff" | "bmp" | "gif"
            )
        })
    }

    /// Batch-describe assets using a VLM endpoint.
    pub fn describe(
        &self,
        query: Option<&str>,
        asset_id: Option<&str>,
        volume: Option<&str>,
        endpoint: &str,
        model: &str,
        params: &crate::vlm::VlmParams,
        mode: crate::vlm::DescribeMode,
        apply: bool,
        force: bool,
        dry_run: bool,
        concurrency: u32,
        on_asset: impl Fn(&str, &crate::vlm::DescribeStatus, std::time::Duration) + Sync,
    ) -> Result<crate::vlm::BatchDescribeResult> {
        self.describe_inner(query, asset_id, volume, None, endpoint, model, params, mode, apply, force, dry_run, concurrency, on_asset)
    }

    /// Describe specific assets by ID (for post-import phase).
    pub fn describe_assets(
        &self,
        asset_ids: &[String],
        endpoint: &str,
        model: &str,
        params: &crate::vlm::VlmParams,
        mode: crate::vlm::DescribeMode,
        force: bool,
        dry_run: bool,
        concurrency: u32,
        on_asset: impl Fn(&str, &crate::vlm::DescribeStatus, std::time::Duration) + Sync,
    ) -> Result<crate::vlm::BatchDescribeResult> {
        self.describe_inner(None, None, None, Some(asset_ids), endpoint, model, params, mode, true, force, dry_run, concurrency, on_asset)
    }

    fn describe_inner(
        &self,
        query: Option<&str>,
        asset_id: Option<&str>,
        volume: Option<&str>,
        explicit_ids: Option<&[String]>,
        endpoint: &str,
        model: &str,
        params: &crate::vlm::VlmParams,
        mode: crate::vlm::DescribeMode,
        apply: bool,
        force: bool,
        dry_run: bool,
        concurrency: u32,
        on_asset: impl Fn(&str, &crate::vlm::DescribeStatus, std::time::Duration) + Sync,
    ) -> Result<crate::vlm::BatchDescribeResult> {
        use crate::vlm::{self, BatchDescribeResult, DescribeMode, DescribeResult, DescribeStatus};

        let catalog = crate::catalog::Catalog::open(&self.catalog_root)?;
        let engine = crate::query::QueryEngine::new(&self.catalog_root);
        let preview_gen =
            crate::preview::PreviewGenerator::new(&self.catalog_root, self.verbosity, &self.preview_config);
        let registry = DeviceRegistry::new(&self.catalog_root);
        let volumes = registry.list()?;
        let online_volumes: HashMap<String, &crate::models::Volume> = volumes
            .iter()
            .filter(|v| v.is_online)
            .map(|v| (v.id.to_string(), v))
            .collect();

        // Resolve target assets
        let asset_ids: Vec<String> = if let Some(ids) = explicit_ids {
            ids.to_vec()
        } else if let Some(id) = asset_id {
            let full_id = catalog
                .resolve_asset_id(id)?
                .ok_or_else(|| anyhow::anyhow!("No asset found matching '{id}'"))?;
            vec![full_id]
        } else {
            let q = if let Some(query) = query {
                let volume_part = volume.map(|v| format!(" volume:{v}")).unwrap_or_default();
                format!("{query}{volume_part}")
            } else if let Some(v) = volume {
                format!("volume:{v}")
            } else {
                "*".to_string()
            };
            let results = engine.search(&q)?;
            results.into_iter().map(|r| r.asset_id).collect()
        };

        let wants_description = mode == DescribeMode::Describe || mode == DescribeMode::Both;
        let concurrency = (concurrency.max(1)) as usize;

        if self.verbosity.verbose {
            eprintln!("  Describe: {} candidate asset(s), concurrency={concurrency}", asset_ids.len());
        }

        let mut result = BatchDescribeResult {
            described: 0,
            skipped: 0,
            failed: 0,
            tags_applied: 0,
            errors: Vec::new(),
            dry_run: !apply || dry_run,
            mode: mode.to_string(),
            results: Vec::new(),
        };

        // Phase 1: Prepare work items (sequential — needs catalog reads)
        struct WorkItem {
            asset_id: String,
            image_path: std::path::PathBuf,
            existing_tags: HashSet<String>,
        }
        let mut work_items: Vec<WorkItem> = Vec::new();

        for aid in &asset_ids {
            let asset_start = std::time::Instant::now();
            let short_id = &aid[..8.min(aid.len())];

            // Load asset details
            let details = match catalog.load_asset_details(aid)? {
                Some(d) => d,
                None => {
                    let msg = format!("Asset {short_id} not found");
                    result.errors.push(msg.clone());
                    result.failed += 1;
                    result.results.push(DescribeResult {
                        asset_id: aid.clone(),
                        description: None,
                        tags: Vec::new(),
                        status: DescribeStatus::Error(msg.clone()),
                    });
                    on_asset(aid, &DescribeStatus::Error(msg), asset_start.elapsed());
                    continue;
                }
            };

            // In describe/both modes, skip if description exists and --force not set
            if wants_description && !force {
                if let Some(ref desc) = details.description {
                    if !desc.is_empty() {
                        let msg = "already has description".to_string();
                        result.skipped += 1;
                        result.results.push(DescribeResult {
                            asset_id: aid.clone(),
                            description: Some(desc.clone()),
                            tags: Vec::new(),
                            status: DescribeStatus::Skipped(msg.clone()),
                        });
                        on_asset(aid, &DescribeStatus::Skipped(msg), asset_start.elapsed());
                        continue;
                    }
                }
            }

            // Find image
            let image_path = self.find_image_for_vlm(&details, &preview_gen, &online_volumes);
            let image_path = match image_path {
                Some(p) => p,
                None => {
                    let msg = format!("No preview/image for asset {short_id}. Run `maki generate-previews` first.");
                    result.skipped += 1;
                    result.results.push(DescribeResult {
                        asset_id: aid.clone(),
                        description: None,
                        tags: Vec::new(),
                        status: DescribeStatus::Skipped(msg.clone()),
                    });
                    on_asset(aid, &DescribeStatus::Skipped(msg), asset_start.elapsed());
                    continue;
                }
            };

            if dry_run {
                let msg = format!("would process (image: {})", image_path.display());
                result.described += 1;
                result.results.push(DescribeResult {
                    asset_id: aid.clone(),
                    description: None,
                    tags: Vec::new(),
                    status: DescribeStatus::Described,
                });
                on_asset(aid, &DescribeStatus::Skipped(msg), asset_start.elapsed());
                continue;
            }

            let existing_tags: HashSet<String> = details
                .tags
                .iter()
                .map(|t| t.to_lowercase())
                .collect();

            work_items.push(WorkItem {
                asset_id: aid.clone(),
                image_path,
                existing_tags,
            });
        }

        // Phase 2: VLM calls in parallel batches
        let verbosity = self.verbosity;
        for chunk in work_items.chunks(concurrency) {
            // Each chunk runs concurrently using scoped threads
            let vlm_results: Vec<(String, HashSet<String>, std::time::Duration, Result<vlm::VlmOutput, String>)> =
                std::thread::scope(|s| {
                    let handles: Vec<_> = chunk
                        .iter()
                        .map(|item| {
                            let aid = &item.asset_id;
                            let image_path = &item.image_path;
                            s.spawn(move || {
                                let start = std::time::Instant::now();
                                let short_id = &aid[..8.min(aid.len())];

                                // Encode image to base64
                                let vlm_max_edge = if params.max_image_edge > 0 { Some(params.max_image_edge) } else { None };
                                let image_base64 = match vlm::encode_image_base64(image_path, vlm_max_edge) {
                                    Ok(b) => b,
                                    Err(e) => {
                                        return (
                                            aid.clone(),
                                            start.elapsed(),
                                            Err(format!("Failed to read image for {short_id}: {e}")),
                                        );
                                    }
                                };

                                // Call VLM
                                let prompt = params.prompt.as_deref()
                                    .unwrap_or_else(|| vlm::default_prompt_for_mode(mode));
                                match vlm::call_vlm_with_mode(
                                    endpoint, model, &image_base64, prompt,
                                    params, mode, verbosity,
                                ) {
                                    Ok(output) => {
                                        if output.description.as_ref().map_or(true, |d| d.is_empty())
                                            && output.tags.is_empty()
                                        {
                                            (
                                                aid.clone(),
                                                start.elapsed(),
                                                Err(format!(
                                                    "VLM returned empty response for {short_id} — \
                                                     model \"{model}\" may not support vision or failed to load"
                                                )),
                                            )
                                        } else {
                                            (aid.clone(), start.elapsed(), Ok(output))
                                        }
                                    }
                                    Err(e) => (
                                        aid.clone(),
                                        start.elapsed(),
                                        Err(format!("VLM failed for {short_id}: {e}")),
                                    ),
                                }
                            })
                        })
                        .collect();

                    handles
                        .into_iter()
                        .zip(chunk.iter())
                        .map(|(h, item)| {
                            let (aid, elapsed, vlm_result) = h.join().unwrap();
                            (aid, item.existing_tags.clone(), elapsed, vlm_result)
                        })
                        .collect()
                });

            // Phase 3: Apply results sequentially (catalog writes not thread-safe)
            for (aid, existing_tags, elapsed, vlm_result) in vlm_results {
                let short_id = &aid[..8.min(aid.len())];

                match vlm_result {
                    Err(msg) => {
                        result.errors.push(msg.clone());
                        result.failed += 1;
                        result.results.push(DescribeResult {
                            asset_id: aid.clone(),
                            description: None,
                            tags: Vec::new(),
                            status: DescribeStatus::Error(msg.clone()),
                        });
                        on_asset(&aid, &DescribeStatus::Error(msg), elapsed);
                    }
                    Ok(output) => {
                        if apply {
                            // Apply description
                            if let Some(ref desc) = output.description {
                                if !desc.is_empty() {
                                    let edit_fields = crate::query::EditFields {
                                        name: None,
                                        description: Some(Some(desc.clone())),
                                        rating: None,
                                        color_label: None,
                                        created_at: None,
                                    };
                                    if let Err(e) = engine.edit(&aid, edit_fields) {
                                        let msg = format!("Failed to save description for {short_id}: {e}");
                                        result.errors.push(msg.clone());
                                        result.failed += 1;
                                        result.results.push(DescribeResult {
                                            asset_id: aid.clone(),
                                            description: output.description,
                                            tags: output.tags,
                                            status: DescribeStatus::Error(msg.clone()),
                                        });
                                        on_asset(&aid, &DescribeStatus::Error(msg), elapsed);
                                        continue;
                                    }
                                }
                            }

                            // Apply tags — deduplicated against existing tags
                            if !output.tags.is_empty() {
                                let new_tags: Vec<String> = output
                                    .tags
                                    .iter()
                                    .filter(|t| !existing_tags.contains(&t.to_lowercase()))
                                    .cloned()
                                    .collect();

                                if !new_tags.is_empty() {
                                    match engine.tag(&aid, &new_tags, false) {
                                        Ok(_) => {
                                            result.tags_applied += new_tags.len();
                                        }
                                        Err(e) => {
                                            let msg = format!("Failed to apply tags for {short_id}: {e}");
                                            result.errors.push(msg.clone());
                                        }
                                    }
                                }
                            }
                        }

                        result.described += 1;
                        result.results.push(DescribeResult {
                            asset_id: aid.clone(),
                            description: output.description.clone(),
                            tags: output.tags.clone(),
                            status: DescribeStatus::Described,
                        });
                        on_asset(&aid, &DescribeStatus::Described, elapsed);
                    }
                }
            }
        }

        Ok(result)
    }
}

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
pub(crate) fn determine_asset_type(ext: &str) -> AssetType {
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
                let mut m = std::collections::HashMap::new();
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
        let result = service.relocate(&asset_id, "vol2", false, false).unwrap();
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

        let result = service.relocate(&asset_id, "vol2", true, false).unwrap();
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

        let result = service.relocate(&asset_id, "vol2", false, false).unwrap();
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
        service.relocate(&asset_id, "vol2", false, false).unwrap();

        // Second relocate — vol1 location still generates a plan entry,
        // but the file already exists on vol2 with matching hash, so it's skipped
        let result = service.relocate(&asset_id, "vol2", false, false).unwrap();
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

        let result = service.relocate(&asset_id, "vol2", false, true).unwrap();
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
        let result = service.relocate(&asset_id, "vol1", false, false).unwrap();
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
            .relocate(&asset_id, "offline-vol", false, false)
            .unwrap_err();
        assert!(err.to_string().contains("offline"));
    }

    #[test]
    fn relocate_fails_for_unknown_asset_id() {
        let (catalog_dir, _vol1_dir, _vol2_dir, _vol1, _vol2) = setup_relocate();

        let service = AssetService::new(catalog_dir.path(), crate::Verbosity::quiet(), &crate::config::PreviewConfig::default());
        let err = service
            .relocate("nonexistent-id", "vol2", false, false)
            .unwrap_err();
        assert!(err.to_string().contains("No asset found"));
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
            .relocate(&asset_id, "nonexistent-vol", false, false)
            .unwrap_err();
        assert!(err.to_string().contains("No volume found"));
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

        // Verify the JPG variant is now Alternate in sidecar
        let asset = metadata_store.load(summaries[0].id).unwrap();
        let jpg = asset.variants.iter().find(|v| v.format == "jpg").unwrap();
        assert_eq!(jpg.role, VariantRole::Alternate);
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
