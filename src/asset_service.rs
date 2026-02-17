use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use uuid::Uuid;

use crate::catalog::Catalog;
use crate::content_store::ContentStore;
use crate::metadata_store::MetadataStore;
use crate::models::{
    Asset, AssetType, FileLocation, Recipe, RecipeType, Variant, VariantRole, Volume,
};

/// Status of a single file during import.
pub enum FileStatus {
    Imported,
    LocationAdded,
    Skipped,
    RecipeAttached,
}

/// Result of an import operation.
pub struct ImportResult {
    pub imported: usize,
    pub locations_added: usize,
    pub skipped: usize,
    pub recipes_attached: usize,
}

/// A group of files sharing the same stem in the same directory.
struct StemGroup {
    _dir: PathBuf,
    stem: String,
    media_files: Vec<PathBuf>,
    recipe_files: Vec<PathBuf>,
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
    pub fn import(&self, paths: &[PathBuf], volume: &Volume) -> Result<ImportResult> {
        self.import_with_callback(paths, volume, |_, _, _| {})
    }

    /// Import files with a per-file callback reporting path, status, and elapsed time.
    pub fn import_with_callback(
        &self,
        paths: &[PathBuf],
        volume: &Volume,
        on_file: impl Fn(&Path, FileStatus, Duration),
    ) -> Result<ImportResult> {
        let content_store = ContentStore::new(&self.catalog_root);
        let metadata_store = MetadataStore::new(&self.catalog_root);
        let catalog = Catalog::open(&self.catalog_root)?;

        catalog.ensure_volume(volume)?;

        let files = resolve_files(paths);
        let groups = group_by_stem(&files);

        let mut imported = 0;
        let mut locations_added = 0;
        let mut skipped = 0;
        let mut recipes_attached = 0;

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

                    // No group asset and variant doesn't exist: import as standalone
                    let ext = file_path
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("");
                    let filename = file_path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("unknown")
                        .to_string();
                    let file_size = std::fs::metadata(file_path)?.len();
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

                // Check if this recipe is already attached
                let already_attached = asset.recipes.iter().any(|r| r.content_hash == content_hash);
                if already_attached {
                    skipped += 1;
                    on_file(file_path, FileStatus::Skipped, file_start.elapsed());
                    continue;
                }

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
        })
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

/// Check if a file extension belongs to a processing recipe/sidecar.
fn is_recipe_extension(ext: &str) -> bool {
    matches!(
        ext.to_lowercase().as_str(),
        "xmp" | "cos" | "cot" | "cop" | "pp3" | "dop" | "on1"
    )
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
fn group_by_stem(files: &[PathBuf]) -> Vec<StemGroup> {
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

        let key = (dir.clone(), stem.clone());
        let group = map.entry(key).or_insert_with(|| StemGroup {
            _dir: dir,
            stem,
            media_files: Vec::new(),
            recipe_files: Vec::new(),
        });

        if is_recipe_extension(ext) {
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
    fn is_recipe_extension_works() {
        assert!(is_recipe_extension("xmp"));
        assert!(is_recipe_extension("XMP"));
        assert!(is_recipe_extension("cos"));
        assert!(is_recipe_extension("pp3"));
        assert!(!is_recipe_extension("jpg"));
        assert!(!is_recipe_extension("nef"));
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
    fn group_by_stem_basic() {
        let files = vec![
            PathBuf::from("/photos/DSC_001.nef"),
            PathBuf::from("/photos/DSC_001.jpg"),
            PathBuf::from("/photos/DSC_001.xmp"),
            PathBuf::from("/photos/DSC_002.jpg"),
        ];
        let groups = group_by_stem(&files);
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
        let files = vec![
            PathBuf::from("/a/photo.jpg"),
            PathBuf::from("/b/photo.jpg"),
        ];
        let groups = group_by_stem(&files);
        assert_eq!(groups.len(), 2);
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
        let r1 = service.import(&[dir_a.join("photo.jpg")], &volume).unwrap();
        assert_eq!(r1.imported, 1);
        assert_eq!(r1.locations_added, 0);
        assert_eq!(r1.skipped, 0);

        // Second import — same content, different path
        let r2 = service.import(&[dir_b.join("photo.jpg")], &volume).unwrap();
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
            .import(&[vol_dir.path().join("photo.jpg")], &volume)
            .unwrap();
        assert_eq!(r1.imported, 1);

        // Second import — exact same path
        let r2 = service
            .import(&[vol_dir.path().join("photo.jpg")], &volume)
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
            .import(&[photos.clone()], &volume)
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
        let result = service.import(&[photos.clone()], &volume).unwrap();

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
        let result = service.import(&[photos.clone()], &volume).unwrap();

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
            .import(&[dir_a, dir_b], &volume)
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
            .import(&[vol_dir.path().join("solo.jpg")], &volume)
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
        let result = service.import(&[photos], &volume).unwrap();
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
        service.import(&[photos], &volume).unwrap();

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
        service.import(&[photos], &volume).unwrap();

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
}
