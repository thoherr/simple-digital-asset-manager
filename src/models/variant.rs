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
    Alternate,
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
        VariantRole::Alternate => 50,
        VariantRole::Sidecar => 0,
    }
}

fn role_score_str(role: &str) -> u64 {
    match role.to_lowercase().as_str() {
        "export" => 300,
        "processed" => 200,
        "original" => 100,
        "alternate" => 50,
        _ => 0,
    }
}

fn variant_score(role_score: u64, format: &str, file_size: u64) -> u64 {
    let format_bonus: u64 = if is_standard_image_format(format) { 50 } else { 0 };
    let size_bonus = (file_size / 1_000_000).min(49);
    role_score + format_bonus + size_bonus
}

/// Return the index of the variant best suited for preview display.
/// Prefers Export > Processed > Original > Alternate, image formats over RAW, larger files.
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
/// If `override_hash` is set and matches a variant, use that; otherwise score algorithmically.
pub fn compute_best_variant_hash(variants: &[Variant]) -> Option<String> {
    best_preview_index(variants).map(|i| variants[i].content_hash.clone())
}

/// Like `compute_best_variant_hash`, but respects a user-set preview override.
pub fn compute_best_variant_hash_with_override(
    variants: &[Variant],
    preview_variant: Option<&str>,
) -> Option<String> {
    if let Some(override_hash) = preview_variant {
        if variants.iter().any(|v| v.content_hash == override_hash) {
            return Some(override_hash.to_string());
        }
    }
    compute_best_variant_hash(variants)
}

/// Return the "primary" format for display — the identity format of the asset.
/// Prefers: Original+RAW first, then Original+any, then falls back to best variant.
pub fn compute_primary_format(variants: &[Variant]) -> Option<String> {
    // First: any Original variant that is RAW
    if let Some(v) = variants.iter().find(|v| {
        v.role == VariantRole::Original && crate::asset_service::is_raw_extension(&v.format)
    }) {
        return Some(v.format.clone());
    }
    // Second: any Original variant
    if let Some(v) = variants.iter().find(|v| v.role == VariantRole::Original) {
        return Some(v.format.clone());
    }
    // Fallback: best variant's format
    best_preview_index(variants).map(|i| variants[i].format.clone())
}

/// Compute GPS coordinates from an asset's variants.
///
/// Prefers the Original-role variant, falls back to any variant with GPS.
/// Reads `gps_latitude_decimal`/`gps_longitude_decimal` first, then parses DMS strings.
pub fn compute_gps_from_variants(variants: &[Variant]) -> (Option<f64>, Option<f64>) {
    // Sort: prefer original role
    let candidates: Vec<&Variant> = {
        let mut originals: Vec<&Variant> = variants.iter().filter(|v| v.role == VariantRole::Original).collect();
        let mut others: Vec<&Variant> = variants.iter().filter(|v| v.role != VariantRole::Original).collect();
        originals.append(&mut others);
        originals
    };

    for v in candidates {
        // Try decimal values first
        if let (Some(lat_str), Some(lon_str)) = (
            v.source_metadata.get("gps_latitude_decimal"),
            v.source_metadata.get("gps_longitude_decimal"),
        ) {
            if let (Ok(lat), Ok(lon)) = (lat_str.parse::<f64>(), lon_str.parse::<f64>()) {
                return (Some(lat), Some(lon));
            }
        }
        // Fall back to DMS strings
        if let (Some(lat_str), Some(lon_str)) = (
            v.source_metadata.get("gps_latitude"),
            v.source_metadata.get("gps_longitude"),
        ) {
            if let (Some(lat), Some(lon)) = (
                crate::exif_reader::parse_dms_string(lat_str),
                crate::exif_reader::parse_dms_string(lon_str),
            ) {
                return (Some(lat), Some(lon));
            }
        }
    }
    (None, None)
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

    #[test]
    fn best_preview_prefers_original_over_alternate() {
        let variants = vec![
            make_variant(VariantRole::Alternate, "jpg", 5_000_000),
            make_variant(VariantRole::Original, "nef", 25_000_000),
        ];
        assert_eq!(best_preview_index(&variants), Some(1));
    }

    #[test]
    fn best_preview_alternate_over_sidecar() {
        let variants = vec![
            make_variant(VariantRole::Sidecar, "xmp", 1_000),
            make_variant(VariantRole::Alternate, "jpg", 5_000_000),
        ];
        assert_eq!(best_preview_index(&variants), Some(1));
    }

    #[test]
    fn best_preview_details_alternate_score() {
        let variants = vec![
            make_details("alternate", "jpg", 5_000_000),
            make_details("original", "nef", 25_000_000),
        ];
        // Original (100) > Alternate (50+50 format bonus) — both score 100, but original has larger file
        assert_eq!(best_preview_index_details(&variants), Some(1));
    }

    #[test]
    fn primary_format_prefers_raw_original() {
        let variants = vec![
            make_variant(VariantRole::Original, "nef", 25_000_000),
            make_variant(VariantRole::Export, "jpg", 5_000_000),
        ];
        assert_eq!(compute_primary_format(&variants).as_deref(), Some("nef"));
    }

    #[test]
    fn primary_format_falls_back_to_original() {
        let variants = vec![
            make_variant(VariantRole::Original, "jpg", 5_000_000),
            make_variant(VariantRole::Export, "tiff", 50_000_000),
        ];
        assert_eq!(compute_primary_format(&variants).as_deref(), Some("jpg"));
    }

    #[test]
    fn primary_format_falls_back_to_best_variant() {
        let variants = vec![
            make_variant(VariantRole::Export, "jpg", 5_000_000),
        ];
        assert_eq!(compute_primary_format(&variants).as_deref(), Some("jpg"));
    }

    #[test]
    fn primary_format_empty_returns_none() {
        let empty: Vec<Variant> = vec![];
        assert_eq!(compute_primary_format(&empty), None);
    }

    #[test]
    fn override_selects_specified_variant() {
        let variants = vec![
            make_variant(VariantRole::Original, "nef", 25_000_000),
            make_variant(VariantRole::Export, "jpg", 5_000_000),
        ];
        // Without override: export wins
        assert_eq!(
            compute_best_variant_hash(&variants).as_deref(),
            Some("sha256:jpg_5000000")
        );
        // With override: original wins
        assert_eq!(
            compute_best_variant_hash_with_override(&variants, Some("sha256:nef_25000000")).as_deref(),
            Some("sha256:nef_25000000")
        );
    }

    #[test]
    fn override_with_invalid_hash_falls_back() {
        let variants = vec![
            make_variant(VariantRole::Original, "nef", 25_000_000),
            make_variant(VariantRole::Export, "jpg", 5_000_000),
        ];
        // Invalid override falls back to algorithmic
        assert_eq!(
            compute_best_variant_hash_with_override(&variants, Some("sha256:nonexistent")).as_deref(),
            Some("sha256:jpg_5000000")
        );
    }

    #[test]
    fn override_none_uses_algorithmic() {
        let variants = vec![
            make_variant(VariantRole::Original, "nef", 25_000_000),
            make_variant(VariantRole::Export, "jpg", 5_000_000),
        ];
        assert_eq!(
            compute_best_variant_hash_with_override(&variants, None).as_deref(),
            Some("sha256:jpg_5000000")
        );
    }
}
