use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::volume::FileLocation;
use crate::catalog::VariantDetails;

/// The role a variant plays within an asset group.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VariantRole {
    Original,
    Processed,
    Export,
    Sidecar,
}

/// A concrete file belonging to an asset.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Variant {
    pub content_hash: String,
    pub asset_id: Uuid,
    pub role: VariantRole,
    pub format: String,
    pub file_size: u64,
    pub original_filename: String,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub source_metadata: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub locations: Vec<FileLocation>,
}

fn is_standard_image_format(ext: &str) -> bool {
    matches!(
        ext.to_lowercase().as_str(),
        "jpg" | "jpeg" | "png" | "tiff" | "tif" | "webp"
    )
}

fn role_score_enum(role: &VariantRole) -> u64 {
    match role {
        VariantRole::Export => 300,
        VariantRole::Processed => 200,
        VariantRole::Original => 100,
        VariantRole::Sidecar => 0,
    }
}

fn role_score_str(role: &str) -> u64 {
    match role.to_lowercase().as_str() {
        "export" => 300,
        "processed" => 200,
        "original" => 100,
        _ => 0,
    }
}

fn variant_score(role_score: u64, format: &str, file_size: u64) -> u64 {
    let format_bonus: u64 = if is_standard_image_format(format) { 50 } else { 0 };
    let size_bonus = (file_size / 1_000_000).min(49);
    role_score + format_bonus + size_bonus
}

/// Return the index of the variant best suited for preview display.
/// Prefers Export > Processed > Original, image formats over RAW, larger files.
pub fn best_preview_index(variants: &[Variant]) -> Option<usize> {
    if variants.is_empty() {
        return None;
    }
    variants
        .iter()
        .enumerate()
        .max_by_key(|(_, v)| variant_score(role_score_enum(&v.role), &v.format, v.file_size))
        .map(|(i, _)| i)
}

/// Return the content hash of the best variant for display (browse grid, search results).
/// Reuses the same scoring as `best_preview_index()`.
pub fn compute_best_variant_hash(variants: &[Variant]) -> Option<String> {
    best_preview_index(variants).map(|i| variants[i].content_hash.clone())
}

/// Return the index of the best preview variant from catalog `VariantDetails` (role is a String).
pub fn best_preview_index_details(variants: &[VariantDetails]) -> Option<usize> {
    if variants.is_empty() {
        return None;
    }
    variants
        .iter()
        .enumerate()
        .max_by_key(|(_, v)| variant_score(role_score_str(&v.role), &v.format, v.file_size))
        .map(|(i, _)| i)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_variant(role: VariantRole, format: &str, file_size: u64) -> Variant {
        Variant {
            content_hash: format!("sha256:{format}_{file_size}"),
            asset_id: Uuid::nil(),
            role,
            format: format.to_string(),
            file_size,
            original_filename: format!("test.{format}"),
            source_metadata: HashMap::new(),
            locations: vec![],
        }
    }

    fn make_details(role: &str, format: &str, file_size: u64) -> VariantDetails {
        VariantDetails {
            content_hash: format!("sha256:{format}_{file_size}"),
            role: role.to_string(),
            format: format.to_string(),
            file_size,
            original_filename: format!("test.{format}"),
            source_metadata: HashMap::new(),
            locations: vec![],
        }
    }

    #[test]
    fn best_preview_prefers_export_over_original() {
        let variants = vec![
            make_variant(VariantRole::Original, "nef", 25_000_000),
            make_variant(VariantRole::Export, "jpg", 5_000_000),
        ];
        assert_eq!(best_preview_index(&variants), Some(1));
    }

    #[test]
    fn best_preview_falls_back_to_original() {
        let variants = vec![make_variant(VariantRole::Original, "nef", 25_000_000)];
        assert_eq!(best_preview_index(&variants), Some(0));
    }

    #[test]
    fn best_preview_skips_sidecar() {
        let variants = vec![
            make_variant(VariantRole::Sidecar, "xmp", 1_000),
            make_variant(VariantRole::Original, "nef", 25_000_000),
        ];
        assert_eq!(best_preview_index(&variants), Some(1));
    }

    #[test]
    fn best_preview_prefers_image_format_within_same_role() {
        let variants = vec![
            make_variant(VariantRole::Original, "nef", 25_000_000),
            make_variant(VariantRole::Original, "jpg", 5_000_000),
        ];
        assert_eq!(best_preview_index(&variants), Some(1));
    }

    #[test]
    fn best_preview_details_prefers_export() {
        let variants = vec![
            make_details("original", "nef", 25_000_000),
            make_details("export", "jpg", 5_000_000),
        ];
        assert_eq!(best_preview_index_details(&variants), Some(1));
    }

    #[test]
    fn best_preview_empty_returns_none() {
        let empty: Vec<Variant> = vec![];
        assert_eq!(best_preview_index(&empty), None);
    }

    #[test]
    fn best_preview_tiebreak_by_file_size() {
        let variants = vec![
            make_variant(VariantRole::Export, "jpg", 2_000_000),
            make_variant(VariantRole::Export, "jpg", 10_000_000),
        ];
        assert_eq!(best_preview_index(&variants), Some(1));
    }
}
