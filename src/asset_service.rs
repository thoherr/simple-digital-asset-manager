use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

use crate::catalog::Catalog;
use crate::content_store::ContentStore;
use crate::metadata_store::MetadataStore;
use crate::models::{Asset, AssetType, FileLocation, Variant, VariantRole, Volume};

/// Status of a single file during import.
pub enum FileStatus {
    Imported,
    LocationAdded,
    Skipped,
}

/// Result of an import operation.
pub struct ImportResult {
    pub imported: usize,
    pub locations_added: usize,
    pub skipped: usize,
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
        let mut imported = 0;
        let mut locations_added = 0;
        let mut skipped = 0;

        for file_path in &files {
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
                let asset_id: uuid::Uuid = asset_id.parse().with_context(|| {
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
                        skipped += 1;
                        on_file(file_path, FileStatus::Skipped, file_start.elapsed());
                        continue;
                    }
                    variant.locations.push(location.clone());
                    metadata_store.save(&asset)?;
                    catalog.insert_file_location(&content_hash, &location)?;
                    locations_added += 1;
                    on_file(file_path, FileStatus::LocationAdded, file_start.elapsed());
                } else {
                    skipped += 1;
                    on_file(file_path, FileStatus::Skipped, file_start.elapsed());
                }
                continue;
            }

            let ext = file_path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("");
            let asset_type = determine_asset_type(ext);

            let filename = file_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string();

            let file_size = std::fs::metadata(file_path)
                .with_context(|| format!("Failed to read metadata for {}", file_path.display()))?
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

            let exif_data = crate::exif_reader::extract(file_path);

            let mut asset = Asset::new(asset_type, &content_hash);
            if let Some(date_taken) = exif_data.date_taken {
                asset.created_at = date_taken;
            }
            asset.name = Some(filename.clone());

            let location = FileLocation {
                volume_id: volume.id,
                relative_path: relative_path.to_path_buf(),
                verified_at: None,
            };

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

            // Write sidecar YAML (source of truth)
            metadata_store
                .save(&asset)
                .with_context(|| format!("Failed to write sidecar for {}", file_path.display()))?;

            // Insert into SQLite (cache)
            catalog.insert_asset(&asset)?;
            catalog.insert_variant(&variant)?;
            catalog.insert_file_location(&content_hash, &location)?;

            imported += 1;
            on_file(file_path, FileStatus::Imported, file_start.elapsed());
        }

        Ok(ImportResult { imported, locations_added, skipped })
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
        assert_eq!(variant.locations[0].relative_path, std::path::Path::new("a/photo.jpg"));
        assert_eq!(variant.locations[1].relative_path, std::path::Path::new("b/photo.jpg"));
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
        let r1 = service.import(&[vol_dir.path().join("photo.jpg")], &volume).unwrap();
        assert_eq!(r1.imported, 1);

        // Second import — exact same path
        let r2 = service.import(&[vol_dir.path().join("photo.jpg")], &volume).unwrap();
        assert_eq!(r2.imported, 0);
        assert_eq!(r2.locations_added, 0);
        assert_eq!(r2.skipped, 1);
    }
}
