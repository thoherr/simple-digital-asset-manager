use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use uuid::Uuid;

use crate::catalog::Catalog;
use crate::content_store::ContentStore;
use crate::metadata_store::MetadataStore;
use crate::models::{Asset, AssetType, FileLocation, Variant, VariantRole, Volume};

/// A group of variants that share the same content hash.
pub struct DuplicateGroup {
    pub content_hash: String,
    pub locations: Vec<FileLocation>,
}

/// An integrity issue found during verification.
pub struct IntegrityIssue {
    pub content_hash: String,
    pub location: FileLocation,
    pub issue: String,
}

/// Result of an import operation.
pub struct ImportResult {
    pub imported: usize,
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
        let content_store = ContentStore::new(&self.catalog_root);
        let metadata_store = MetadataStore::new(&self.catalog_root);
        let catalog = Catalog::open(&self.catalog_root)?;

        catalog.ensure_volume(volume)?;

        let files = resolve_files(paths);
        let mut imported = 0;
        let mut skipped = 0;

        for file_path in &files {
            let content_hash = content_store
                .ingest(file_path, volume)
                .with_context(|| format!("Failed to hash {}", file_path.display()))?;

            if catalog.has_variant(&content_hash)? {
                skipped += 1;
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

            let mut asset = Asset::new(asset_type);
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
                source_metadata: Default::default(),
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
        }

        Ok(ImportResult { imported, skipped })
    }

    /// Manually group variants into one asset.
    pub fn group(&self, _variant_hashes: &[&str]) -> Result<Asset> {
        anyhow::bail!("not yet implemented")
    }

    /// Remove a variant from a group.
    pub fn ungroup(&self, _asset_id: Uuid, _variant_hash: &str) -> Result<()> {
        anyhow::bail!("not yet implemented")
    }

    /// Add tags to an asset.
    pub fn tag(&self, _asset_id: Uuid, _tags: &[String]) -> Result<()> {
        anyhow::bail!("not yet implemented")
    }

    /// Move all variants of an asset to another volume.
    pub fn relocate(&self, _asset_id: Uuid, _target_volume: Uuid) -> Result<()> {
        anyhow::bail!("not yet implemented")
    }

    /// Find variants with the same hash on multiple locations.
    pub fn find_duplicates(&self) -> Result<Vec<DuplicateGroup>> {
        anyhow::bail!("not yet implemented")
    }

    /// Verify hashes for a volume or all online volumes.
    pub fn check_integrity(&self, _volume_id: Option<Uuid>) -> Result<Vec<IntegrityIssue>> {
        anyhow::bail!("not yet implemented")
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
}
