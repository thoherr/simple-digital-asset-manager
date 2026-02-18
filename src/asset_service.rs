use std::collections::{BTreeMap, HashSet};
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
            "raw", "cr2", "cr3", "nef", "arw", "orf", "rw2", "dng", "raf", "pef", "srw", // RAW
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
    pub imported: usize,
    pub locations_added: usize,
    pub skipped: usize,
    pub recipes_attached: usize,
    pub recipes_updated: usize,
    pub previews_generated: usize,
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
    Untracked,
}

/// Result of a verify operation.
#[derive(serde::Serialize)]
pub struct VerifyResult {
    pub verified: usize,
    pub failed: usize,
    pub modified: usize,
    pub skipped: usize,
    pub errors: Vec<String>,
}

/// High-level operations that orchestrate the other components.
pub struct AssetService {
    catalog_root: PathBuf,
}

impl AssetService {
    pub fn new(catalog_root: &Path) -> Self {
        Self {
            catalog_root: catalog_root.to_path_buf(),
        }
    }

    /// Import files: hash, deduplicate, create assets/variants, write sidecars, insert into DB.
    pub fn import(
        &self,
        paths: &[PathBuf],
        volume: &Volume,
        filter: &FileTypeFilter,
    ) -> Result<ImportResult> {
        self.import_with_callback(paths, volume, filter, |_, _, _| {})
    }

    /// Import files with a per-file callback reporting path, status, and elapsed time.
    pub fn import_with_callback(
        &self,
        paths: &[PathBuf],
        volume: &Volume,
        filter: &FileTypeFilter,
        on_file: impl Fn(&Path, FileStatus, Duration),
    ) -> Result<ImportResult> {
        let content_store = ContentStore::new(&self.catalog_root);
        let metadata_store = MetadataStore::new(&self.catalog_root);
        let catalog = Catalog::open(&self.catalog_root)?;
        let preview_gen = crate::preview::PreviewGenerator::new(&self.catalog_root);

        catalog.ensure_volume(volume)?;

        let files = resolve_files(paths);
        let groups = group_by_stem(&files, filter);

        let mut imported = 0;
        let mut locations_added = 0;
        let mut skipped = 0;
        let mut recipes_attached = 0;
        let mut recipes_updated = 0;
        let mut previews_generated = 0;

        for group in &groups {
            // Track the asset created/found for this group's primary variant
            let mut group_asset: Option<Asset> = None;
            let mut primary_variant_hash: Option<String> = None;

            // Pass 1: Process media files (RAW first due to sorting in group_by_stem)
            for file_path in &group.media_files {
                let file_start = Instant::now();

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
                        variant.locations.push(location.clone());
                        metadata_store.save(&asset)?;
                        catalog.insert_file_location(&content_hash, &location)?;
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
                    if let Some(date_taken) = exif_data.date_taken {
                        asset.created_at = date_taken;
                    }
                    asset.name = Some(group.stem.clone());

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

                    group_asset = Some(asset);
                } else {
                    // Additional media file → add variant to existing group asset
                    let asset = group_asset.as_mut().unwrap();
                    let exif_data = crate::exif_reader::extract(file_path);

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

                    metadata_store.save(asset).with_context(|| {
                        format!("Failed to write sidecar for {}", file_path.display())
                    })?;
                    catalog.insert_variant(&variant)?;
                    catalog.insert_file_location(&content_hash, &location)?;

                    // Generate preview for the additional variant
                    match preview_gen.generate(&content_hash, file_path, ext) {
                        Ok(Some(_)) => previews_generated += 1,
                        Ok(None) => {}
                        Err(e) => eprintln!("  Preview warning: {e:#}"),
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
                                variant.locations.push(location.clone());
                                metadata_store.save(&asset)?;
                                catalog.insert_file_location(&content_hash, &location)?;
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
                                recipes_updated += 1;
                                on_file(file_path, FileStatus::RecipeUpdated, file_start.elapsed());
                            }
                        } else {
                            // Attach new recipe to parent
                            let recipe = Recipe {
                                id: Uuid::new_v4(),
                                variant_hash: parent_variant_hash.clone(),
                                software: determine_recipe_software(ext).to_string(),
                                recipe_type: RecipeType::Sidecar,
                                content_hash: content_hash.clone(),
                                location,
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
                    asset.variants.push(variant.clone());
                    metadata_store.save(&asset)?;
                    catalog.insert_asset(&asset)?;
                    catalog.insert_variant(&variant)?;
                    catalog.insert_file_location(&content_hash, &location)?;
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
                    recipes_updated += 1;
                    on_file(file_path, FileStatus::RecipeUpdated, file_start.elapsed());
                    continue;
                }

                // No existing recipe at this location — attach new recipe
                let recipe = Recipe {
                    id: Uuid::new_v4(),
                    variant_hash: variant_hash.clone(),
                    software: determine_recipe_software(ext).to_string(),
                    recipe_type: RecipeType::Sidecar,
                    content_hash,
                    location,
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

                recipes_attached += 1;
                on_file(file_path, FileStatus::RecipeAttached, file_start.elapsed());
            }
        }

        Ok(ImportResult {
            imported,
            locations_added,
            skipped,
            recipes_attached,
            recipes_updated,
            previews_generated,
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
            errors: Vec::new(),
        };

        if !paths.is_empty() {
            // Path mode
            let files = resolve_files(paths);
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
                            false,
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
                        true,
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
        is_recipe: bool,
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
                if is_recipe {
                    catalog.update_recipe_verified_at(
                        content_hash,
                        &volume.id.to_string(),
                        &loc.relative_path.to_string_lossy(),
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
                if is_recipe {
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
                    if let Some(asset_id) = catalog.find_asset_id_by_variant(content_hash)? {
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
                                reapply_xmp_data(&xmp, &mut asset, content_hash);
                                catalog.insert_asset(&asset)?;
                                if let Some(v) = asset.variants.iter().find(|v| v.content_hash == content_hash) {
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
}

/// Merge XMP metadata into an asset and its primary variant.
/// - Keywords merge into `asset.tags` (deduplicated)
/// - Description sets `asset.description` if not already set
/// - source_metadata merges into the variant (EXIF takes precedence via `or_insert`)
fn apply_xmp_data(xmp: &crate::xmp_reader::XmpData, asset: &mut Asset, variant_hash: &str) {
    for kw in &xmp.keywords {
        if !asset.tags.contains(kw) {
            asset.tags.push(kw.clone());
        }
    }

    if asset.description.is_none() {
        asset.description.clone_from(&xmp.description);
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
    for kw in &xmp.keywords {
        if !asset.tags.contains(kw) {
            asset.tags.push(kw.clone());
        }
    }

    if xmp.description.is_some() {
        asset.description.clone_from(&xmp.description);
    }

    if let Some(variant) = asset.variants.iter_mut().find(|v| v.content_hash == variant_hash) {
        for (key, val) in &xmp.source_metadata {
            variant.source_metadata.insert(key.clone(), val.clone());
        }
    }
}

/// Determine the asset type from a file extension.
fn determine_asset_type(ext: &str) -> AssetType {
    match ext.to_lowercase().as_str() {
        // Images
        "jpg" | "jpeg" | "png" | "gif" | "bmp" | "tiff" | "tif" | "webp" | "heic" | "heif"
        | "raw" | "cr2" | "cr3" | "nef" | "arw" | "orf" | "rw2" | "dng" | "raf" | "pef"
        | "srw" | "svg" | "ico" | "psd" | "xcf" => AssetType::Image,
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
fn is_raw_extension(ext: &str) -> bool {
    matches!(
        ext.to_lowercase().as_str(),
        "raw" | "cr2" | "cr3" | "nef" | "arw" | "orf" | "rw2" | "dng" | "raf" | "pef" | "srw"
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
/// Skips hidden files/directories (starting with '.').
fn resolve_files(paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut result = Vec::new();
    for path in paths {
        if path.is_dir() {
            collect_files_recursive(path, &mut result);
        } else if path.is_file() {
            result.push(path.clone());
        }
    }
    result
}

fn collect_files_recursive(dir: &Path, result: &mut Vec<PathBuf>) {
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
        if path.is_dir() {
            collect_files_recursive(&path, result);
        } else if path.is_file() {
            result.push(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn determine_asset_type_images() {
        assert_eq!(determine_asset_type("jpg"), AssetType::Image);
        assert_eq!(determine_asset_type("CR2"), AssetType::Image);
        assert_eq!(determine_asset_type("PNG"), AssetType::Image);
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
        assert!(is_raw_extension("nef"));
        assert!(is_raw_extension("CR2"));
        assert!(is_raw_extension("dng"));
        assert!(!is_raw_extension("jpg"));
        assert!(!is_raw_extension("xmp"));
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

        let files = resolve_files(&[dir.path().to_path_buf()]);
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

        let files = resolve_files(&[dir.path().to_path_buf()]);
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

        let service = AssetService::new(catalog_dir.path());

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

        let service = AssetService::new(catalog_dir.path());

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

        let service = AssetService::new(catalog_dir.path());
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
        assert_eq!(asset.name.as_deref(), Some("DSC_4521"));
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

        let service = AssetService::new(catalog_dir.path());
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

        let service = AssetService::new(catalog_dir.path());
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

        let service = AssetService::new(catalog_dir.path());
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

        let service = AssetService::new(catalog_dir.path());
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

        let service = AssetService::new(catalog_dir.path());
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

        let service = AssetService::new(catalog_dir.path());
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

        let service = AssetService::new(catalog_dir.path());
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
            .register("vol1", vol1_dir.path(), crate::models::VolumeType::Local)
            .unwrap();
        let vol2 = registry
            .register("vol2", vol2_dir.path(), crate::models::VolumeType::Local)
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

        let service = AssetService::new(catalog_dir.path());
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

        let service = AssetService::new(catalog_dir.path());
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

        let service = AssetService::new(catalog_dir.path());
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

        let service = AssetService::new(catalog_dir.path());
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

        let service = AssetService::new(catalog_dir.path());
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

        let service = AssetService::new(catalog_dir.path());
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

        let service = AssetService::new(catalog_dir.path());
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

        let service = AssetService::new(catalog_dir.path());
        let err = service
            .relocate("nonexistent-id", "vol2", false, false)
            .unwrap_err();
        assert!(err.to_string().contains("No asset found"));
    }

    #[test]
    fn relocate_fails_for_unknown_volume() {
        let (catalog_dir, vol1_dir, _vol2_dir, vol1, _vol2) = setup_relocate();

        std::fs::write(vol1_dir.path().join("photo.jpg"), "unknown vol test").unwrap();

        let service = AssetService::new(catalog_dir.path());
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
}
