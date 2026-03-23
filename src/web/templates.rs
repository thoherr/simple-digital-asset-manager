use askama::Template;

use crate::catalog::{AnalyticsData, AssetDetails, BackupStatusResult, CatalogStats, SearchRow};

/// Compute preview URL from a content hash like "sha256:abcdef...".
pub fn preview_url(content_hash: &str, ext: &str) -> String {
    let hex = content_hash.strip_prefix("sha256:").unwrap_or(content_hash);
    let prefix = &hex[..2.min(hex.len())];
    format!("/preview/{prefix}/{hex}.{ext}")
}

/// Compute smart preview URL from a content hash.
pub fn smart_preview_url(content_hash: &str, ext: &str) -> String {
    let hex = content_hash.strip_prefix("sha256:").unwrap_or(content_hash);
    let prefix = &hex[..2.min(hex.len())];
    format!("/smart-preview/{prefix}/{hex}.{ext}")
}

/// Format a byte count for display.
pub fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;

    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

/// Truncate a date string to just the date portion.
pub fn format_date(date_str: &str) -> String {
    date_str.split('T').next().unwrap_or(date_str).to_string()
}

/// Pre-computed asset card for template rendering.
pub struct AssetCard {
    pub asset_id: String,
    pub display_name: String,
    pub asset_type: String,
    pub format: String,
    pub date: String,
    pub preview_url: String,
    pub rating: Option<u8>,
    pub color_label: Option<String>,
    pub variant_count: u32,
    pub stack_count: Option<u32>,
    pub stack_id: Option<String>,
    pub prev_id: Option<String>,
    pub next_id: Option<String>,
    pub preview_rotation: Option<u16>,
    pub face_count: u32,
    pub similarity: Option<f32>,
}

impl AssetCard {
    pub fn from_row(row: &SearchRow, preview_ext: &str) -> Self {
        // Use primary_format (Original RAW) when available, otherwise fall back to best variant format
        let format = row.primary_format.as_deref().unwrap_or(&row.format).to_string();
        Self {
            asset_id: row.asset_id.clone(),
            display_name: row
                .name
                .as_deref()
                .unwrap_or(&row.original_filename)
                .to_string(),
            asset_type: row.asset_type.clone(),
            format,
            date: format_date(&row.created_at),
            preview_url: preview_url(&row.content_hash, preview_ext),
            rating: row.rating,
            color_label: row.color_label.clone(),
            variant_count: row.variant_count,
            stack_count: row.stack_count.filter(|&n| n >= 2),
            stack_id: row.stack_id.clone(),
            prev_id: None,
            next_id: None,
            preview_rotation: row.preview_rotation,
            face_count: row.face_count,
            similarity: None,
        }
    }

    /// Return similarity as a percentage (0-100), if set.
    pub fn similarity_pct(&self) -> Option<u32> {
        self.similarity.map(|s| (s * 100.0) as u32)
    }

    /// Build the detail page URL with optional prev/next query params.
    pub fn detail_url(&self) -> String {
        let mut qs = Vec::new();
        if let Some(ref p) = self.prev_id {
            qs.push(format!("prev={p}"));
        }
        if let Some(ref n) = self.next_id {
            qs.push(format!("next={n}"));
        }
        if qs.is_empty() {
            format!("/asset/{}", self.asset_id)
        } else {
            format!("/asset/{}?{}", self.asset_id, qs.join("&"))
        }
    }
}

/// Link adjacent cards with prev/next IDs for detail page navigation.
pub fn link_cards(cards: &mut [AssetCard]) {
    for i in 0..cards.len() {
        if i > 0 {
            cards[i].prev_id = Some(cards[i - 1].asset_id.clone());
        }
        if i + 1 < cards.len() {
            cards[i].next_id = Some(cards[i + 1].asset_id.clone());
        }
    }
}

/// Generate star display HTML for a rating value.
pub fn stars_html(rating: Option<u8>) -> String {
    match rating {
        Some(r) if r > 0 => {
            let mut s = String::new();
            for i in 1..=5 {
                if i <= r {
                    s.push('\u{2605}');
                } else {
                    s.push('\u{2606}');
                }
            }
            s
        }
        _ => String::new(),
    }
}

/// Tag option for dropdowns.
pub struct TagOption {
    pub name: String,
    pub count: u64,
}

/// Format option for dropdowns.
pub struct FormatOption {
    pub name: String,
    pub count: u64,
}

/// Grouped format options for the multi-select format filter.
pub struct FormatGroup {
    pub key: String,
    pub label: String,
    pub formats: Vec<FormatOption>,
}

/// Volume option for dropdowns.
pub struct VolumeOption {
    pub id: String,
    pub label: String,
}

/// Collection option for dropdowns.
pub struct CollectionOption {
    pub name: String,
}

/// Person option for dropdowns.
pub struct PersonOption {
    pub id: String,
    pub name: String,
}

/// Pre-computed variant for template rendering.
pub struct VariantRow {
    pub content_hash: String,
    pub role: String,
    pub original_filename: String,
    pub format: String,
    pub size: String,
    pub locations: Vec<LocationRow>,
    pub source_metadata: Vec<(String, String)>,
}

pub struct LocationRow {
    pub volume_label: String,
    pub volume_id: String,
    pub relative_path: String,
    pub is_online: bool,
}

pub struct RecipeRow {
    pub recipe_type: String,
    pub software: String,
    pub volume_label: String,
    pub volume_id: String,
    pub relative_path: String,
    pub is_online: bool,
    pub pending_writeback: bool,
}

/// A saved search for display in the browse page.
pub struct SavedSearchChip {
    pub name: String,
    pub url_params: String,
}

/// A saved search entry for the management page.
pub struct SavedSearchEntry {
    pub name: String,
    pub query: String,
    pub sort: String,
    pub favorite: bool,
    pub url_params: String,
}

#[derive(Template)]
#[template(path = "saved_searches.html")]
pub struct SavedSearchesPage {
    pub searches: Vec<SavedSearchEntry>,
    pub ai_enabled: bool,
    pub vlm_enabled: bool,
}

#[derive(Template)]
#[template(path = "browse.html")]
pub struct BrowsePage {
    pub query: String,
    pub asset_type: String,
    pub tag: String,
    pub format_filter: String,
    pub volume: String,
    pub rating: String,
    pub label: String,
    pub sort: String,
    pub cards: Vec<AssetCard>,
    pub total: u64,
    pub page: u32,
    pub per_page: u32,
    pub total_pages: u32,
    pub all_tags: Vec<TagOption>,
    pub format_groups: Vec<FormatGroup>,
    pub all_volumes: Vec<VolumeOption>,
    pub all_collections: Vec<CollectionOption>,
    pub all_people: Vec<PersonOption>,
    pub collection: String,
    pub path: String,
    pub person: String,
    pub saved_searches: Vec<SavedSearchChip>,
    pub collapse_stacks: bool,
    pub ai_enabled: bool,
    pub vlm_enabled: bool,
    pub vlm_models: Vec<String>,
    pub default_filter: String,
    pub default_filter_active: bool,
    pub has_similarity: bool,
}

#[derive(Template)]
#[template(path = "results.html")]
pub struct ResultsPartial {
    pub query: String,
    pub asset_type: String,
    pub tag: String,
    pub format_filter: String,
    pub volume: String,
    pub rating: String,
    pub label: String,
    pub collection: String,
    pub path: String,
    pub sort: String,
    pub cards: Vec<AssetCard>,
    pub total: u64,
    pub page: u32,
    pub per_page: u32,
    pub total_pages: u32,
    pub collapse_stacks: bool,
    pub has_similarity: bool,
}

#[derive(Template)]
#[template(path = "asset.html")]
pub struct AssetPage {
    pub asset_id: String,
    pub display_name: String,
    pub name: Option<String>,
    pub fallback_name: String,
    pub asset_type: String,
    pub primary_format: String,
    pub variant_count: usize,
    pub created_at: String,
    pub description: Option<String>,
    pub rating: Option<u8>,
    pub color_label: Option<String>,
    pub tags: Vec<String>,
    pub primary_preview_url: Option<String>,
    pub smart_preview_url: Option<String>,
    pub has_smart_preview: bool,
    pub has_online_source: bool,
    pub error: Option<String>,
    pub best_variant_hash: String,
    pub variants: Vec<VariantRow>,
    pub recipes: Vec<RecipeRow>,
    pub has_pending_writeback: bool,
    pub collections: Vec<AssetCollectionChip>,
    pub stack_members: Vec<StackMemberCard>,
    pub is_stack_pick: bool,
    pub prev_id: Option<String>,
    pub next_id: Option<String>,
    pub ai_enabled: bool,
    pub vlm_enabled: bool,
    pub vlm_models: Vec<String>,
    pub faces: Vec<FaceRow>,
    pub all_people: Vec<PersonOption>,
}

/// A detected face on the asset detail page.
pub struct FaceRow {
    pub face_id: String,
    pub crop_url: Option<String>,
    pub confidence_pct: u32,
    pub person_name: Option<String>,
    pub person_id: Option<String>,
}

/// Collections the asset belongs to, shown on asset detail page.
pub struct AssetCollectionChip {
    pub name: String,
}

/// Person card for the people gallery page.
pub struct PersonCard {
    pub id: String,
    pub name: String,
    pub face_count: usize,
    pub crop_url: Option<String>,
}

#[derive(Template)]
#[template(path = "people.html")]
pub struct PeoplePage {
    pub people: Vec<PersonCard>,
    pub ai_enabled: bool,
    pub vlm_enabled: bool,
}

/// A member of a stack, shown on asset detail page.
pub struct StackMemberCard {
    pub asset_id: String,
    pub display_name: String,
    pub preview_url: String,
    pub is_pick: bool,
}

impl AssetPage {
    pub fn from_details(
        details: AssetDetails,
        preview: Option<String>,
        smart_preview: Option<String>,
        has_smart_preview: bool,
        collections: Vec<String>,
        stack_members: Vec<StackMemberCard>,
        is_stack_pick: bool,
        volume_online: &std::collections::HashMap<String, bool>,
        best_variant_hash: String,
    ) -> Self {
        let fallback_name = details
            .variants
            .first()
            .map(|v| v.original_filename.clone())
            .unwrap_or_else(|| "Untitled".to_string());

        let display_name = details
            .name
            .as_deref()
            .unwrap_or(&fallback_name)
            .to_string();

        // Primary format: prefer original RAW, then original any, then first variant
        let primary_format = details
            .variants
            .iter()
            .find(|v| v.role == "original" && crate::asset_service::is_raw_extension(&v.format))
            .or_else(|| details.variants.iter().find(|v| v.role == "original"))
            .or_else(|| details.variants.first())
            .map(|v| v.format.clone())
            .unwrap_or_default();

        let variant_count = details.variants.len();

        let variants = details
            .variants
            .iter()
            .map(|v| {
                let mut meta: Vec<(String, String)> = v
                    .source_metadata
                    .iter()
                    .map(|(k, val)| (k.clone(), val.clone()))
                    .collect();
                meta.sort_by(|a, b| a.0.cmp(&b.0));

                VariantRow {
                    content_hash: v.content_hash.clone(),
                    role: v.role.clone(),
                    original_filename: v.original_filename.clone(),
                    format: v.format.clone(),
                    size: format_size(v.file_size),
                    locations: v
                        .locations
                        .iter()
                        .map(|l| LocationRow {
                            volume_label: l.volume_label.clone(),
                            volume_id: l.volume_id.clone(),
                            relative_path: l.relative_path.clone(),
                            is_online: volume_online.get(&l.volume_id).copied().unwrap_or(false),
                        })
                        .collect(),
                    source_metadata: meta,
                }
            })
            .collect();

        let recipes: Vec<RecipeRow> = details
            .recipes
            .iter()
            .map(|r| {
                let vid = r.volume_id.clone().unwrap_or_default();
                let online = volume_online.get(&vid).copied().unwrap_or(false);
                RecipeRow {
                    recipe_type: r.recipe_type.clone(),
                    software: r.software.clone(),
                    volume_label: r.volume_label.clone().unwrap_or_else(|| "-".to_string()),
                    volume_id: vid,
                    relative_path: r.relative_path.as_deref().unwrap_or("-").to_string(),
                    is_online: online,
                    pending_writeback: r.pending_writeback,
                }
            })
            .collect();

        // Check if the best variant has any online file location
        let best_idx = crate::models::variant::best_preview_index_details(&details.variants);
        let has_online_source = best_idx.map_or(false, |idx| {
            details.variants[idx].locations.iter().any(|loc| {
                volume_online.get(&loc.volume_id).copied().unwrap_or(false)
            })
        });

        Self {
            asset_id: details.id,
            display_name,
            name: details.name,
            fallback_name,
            asset_type: details.asset_type,
            primary_format,
            variant_count,
            created_at: format_date(&details.created_at),
            description: details.description,
            rating: details.rating,
            color_label: details.color_label,
            tags: details.tags,
            primary_preview_url: preview,
            smart_preview_url: smart_preview,
            has_smart_preview,
            has_online_source,
            error: None,
            best_variant_hash,
            variants,
            has_pending_writeback: recipes.iter().any(|r| r.pending_writeback),
            recipes,
            collections: collections
                .into_iter()
                .map(|name| AssetCollectionChip { name })
                .collect(),
            stack_members,
            is_stack_pick,
            prev_id: None,
            next_id: None,
            ai_enabled: false,
            vlm_enabled: false,
            vlm_models: Vec::new(),
            faces: Vec::new(),
            all_people: Vec::new(),
        }
    }
}

pub struct TagPageEntry {
    pub name: String,
    pub count: u64,
}

pub struct TagTreeEntry {
    pub name: String,         // Internal form with `|` for hierarchy (used in JS tree ops)
    pub display_name: String, // User-facing form with `/` for hierarchy (used in URLs/links)
    pub display: String,      // Leaf segment only
    pub depth: u32,
    pub own_count: u64,
    pub total_count: u64,
    pub has_children: bool,
}

#[derive(Template)]
#[template(path = "tags.html")]
pub struct TagsPage {
    pub tags: Vec<TagTreeEntry>,
    pub total_tags: u64,
    pub ai_enabled: bool,
    pub vlm_enabled: bool,
}

#[derive(Template)]
#[template(path = "stats.html")]
pub struct StatsPage {
    pub stats: CatalogStats,
    pub total_size_fmt: String,
    pub ai_enabled: bool,
    pub vlm_enabled: bool,
}

#[derive(Template)]
#[template(path = "analytics.html")]
pub struct AnalyticsPage {
    pub data: AnalyticsData,
    pub ai_enabled: bool,
    pub vlm_enabled: bool,
}

#[derive(Template)]
#[template(path = "backup.html")]
pub struct BackupPage {
    pub result: BackupStatusResult,
    pub total_assets_fmt: String,
    pub ai_enabled: bool,
    pub vlm_enabled: bool,
}

#[derive(Template)]
#[template(path = "preview_fragment.html")]
pub struct PreviewFragment {
    pub asset_id: String,
    pub primary_preview_url: Option<String>,
    pub smart_preview_url: Option<String>,
    pub has_smart_preview: bool,
    pub has_online_source: bool,
    pub error: Option<String>,
}

#[derive(Template)]
#[template(path = "tags_fragment.html")]
pub struct TagsFragment {
    pub asset_id: String,
    pub tags: Vec<String>,
}

#[derive(Template)]
#[template(path = "description_fragment.html")]
pub struct DescriptionFragment {
    pub asset_id: String,
    pub description: Option<String>,
}

#[derive(Template)]
#[template(path = "name_fragment.html")]
pub struct NameFragment {
    pub asset_id: String,
    pub name: Option<String>,
    pub fallback_name: String,
}

#[derive(Template)]
#[template(path = "rating_fragment.html")]
pub struct RatingFragment {
    pub asset_id: String,
    pub rating: Option<u8>,
}

#[derive(Template)]
#[template(path = "label_fragment.html")]
pub struct LabelFragment {
    pub asset_id: String,
    pub color_label: Option<String>,
}

#[derive(Template)]
#[template(path = "date_fragment.html")]
pub struct DateFragment {
    pub asset_id: String,
    pub created_at: String,
}

#[derive(Template)]
#[template(path = "collections.html")]
pub struct CollectionsPage {
    pub collections: Vec<crate::collection::CollectionSummary>,
    pub ai_enabled: bool,
    pub vlm_enabled: bool,
}

#[derive(Template)]
#[template(path = "duplicates.html")]
pub struct DuplicatesPage {
    pub entries: Vec<crate::catalog::DuplicateEntry>,
    pub mode: String,
    pub total_groups: usize,
    pub total_wasted: u64,
    pub same_volume_count: usize,
    pub volume: String,
    pub format_filter: String,
    pub path: String,
    pub all_volumes: Vec<VolumeOption>,
    pub all_formats: Vec<FormatOption>,
    pub dedup_prefer: String,
    pub ai_enabled: bool,
    pub vlm_enabled: bool,
}

/// Pre-computed asset data for the compare page.
pub struct CompareAsset {
    pub asset_id: String,
    pub display_name: String,
    pub created_at: String,
    pub preview_url: String,
    pub rating: Option<u8>,
    pub color_label: Option<String>,
    pub camera: String,
    pub lens: String,
    pub focal_length: String,
    pub aperture: String,
    pub shutter_speed: String,
    pub iso: String,
}

impl CompareAsset {
    pub fn from_details(details: &AssetDetails, preview_url: String) -> Self {
        let fallback_name = details
            .variants
            .first()
            .map(|v| v.original_filename.clone())
            .unwrap_or_else(|| "Untitled".to_string());

        let display_name = details
            .name
            .as_deref()
            .unwrap_or(&fallback_name)
            .to_string();

        // Extract EXIF metadata from first variant's source_metadata
        let meta = details
            .variants
            .first()
            .map(|v| &v.source_metadata)
            .cloned()
            .unwrap_or_default();

        let make = meta.get("Make").cloned().unwrap_or_default();
        let model = meta.get("Model").cloned().unwrap_or_default();
        let camera = if make.is_empty() && model.is_empty() {
            String::new()
        } else if model.starts_with(&make) {
            model
        } else {
            format!("{make} {model}").trim().to_string()
        };

        Self {
            asset_id: details.id.clone(),
            display_name,
            created_at: format_date(&details.created_at),
            preview_url,
            rating: details.rating,
            color_label: details.color_label.clone(),
            camera,
            lens: meta.get("LensModel").cloned().unwrap_or_default(),
            focal_length: meta.get("FocalLength").cloned().unwrap_or_default(),
            aperture: meta.get("FNumber").cloned().unwrap_or_default(),
            shutter_speed: meta.get("ExposureTime").cloned().unwrap_or_default(),
            iso: meta.get("ISOSpeedRatings").cloned().unwrap_or_default(),
        }
    }
}

#[derive(Template)]
#[template(path = "compare.html")]
pub struct ComparePage {
    pub assets: Vec<CompareAsset>,
    pub ai_enabled: bool,
    pub vlm_enabled: bool,
}

/// A neighbor asset on the stroll page.
pub struct StrollNeighbor {
    pub asset_id: String,
    pub name: String,
    pub preview_url: String,
    pub similarity: f32,
    pub similarity_pct: u32,
    pub rating: Option<u8>,
    pub color_label: Option<String>,
}

/// The center asset on the stroll page.
pub struct StrollCenter {
    pub asset_id: String,
    pub name: String,
    pub preview_url: String,
    pub smart_preview_url: Option<String>,
    pub rating: Option<u8>,
    pub color_label: Option<String>,
    pub format: String,
    pub created_at: String,
}

#[derive(Template)]
#[template(path = "stroll.html")]
pub struct StrollPage {
    pub center: StrollCenter,
    pub neighbors: Vec<StrollNeighbor>,
    pub query: String,
    pub neighbor_count: u32,
    pub stroll_neighbors_max: u32,
    pub stroll_fanout: u32,
    pub stroll_fanout_max: u32,
    pub ai_enabled: bool,
    pub vlm_enabled: bool,
    // Filter bar data (shared with browse page)
    pub tag: String,
    pub rating: String,
    pub label: String,
    pub asset_type: String,
    pub format_filter: String,
    pub format_groups: Vec<FormatGroup>,
    pub all_tags: Vec<TagOption>,
    pub all_volumes: Vec<VolumeOption>,
    pub all_collections: Vec<CollectionOption>,
    pub all_people: Vec<PersonOption>,
    pub volume: String,
    pub collection: String,
    pub path: String,
    pub person: String,
    pub default_filter: String,
    pub default_filter_active: bool,
}

/// Custom askama filters for templates.
mod filters {
    pub fn fmt_bytes(bytes: &u64) -> ::askama::Result<String> {
        Ok(super::format_size(*bytes))
    }

    pub fn pct1(val: &f64) -> ::askama::Result<String> {
        Ok(format!("{val:.1}"))
    }

    pub fn pct0(val: &f64) -> ::askama::Result<String> {
        Ok(format!("{val:.0}"))
    }

    pub fn verify_class(pct: &f64) -> ::askama::Result<String> {
        Ok(if *pct >= 80.0 {
            "fill-good"
        } else if *pct >= 40.0 {
            "fill-warn"
        } else {
            "fill-low"
        }
        .to_string())
    }

    pub fn version(_s: &str) -> ::askama::Result<String> {
        let v = env!("CARGO_PKG_VERSION");
        if cfg!(feature = "pro") {
            Ok(format!("{v} Pro"))
        } else {
            Ok(v.to_string())
        }
    }

    pub fn backup_bar_class(label: &str, min_copies: &u64) -> ::askama::Result<String> {
        // Parse leading digit(s) from label like "0 volumes", "1 volume", "3+ volumes"
        let n: u64 = label.chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse().unwrap_or(0);
        Ok(if n < *min_copies {
            "fill-low"
        } else if n == *min_copies {
            "fill-warn"
        } else {
            "fill-good"
        }
        .to_string())
    }

    /// Escape a string for safe embedding in a JavaScript double-quoted string literal.
    /// Handles `"`, `\`, `<`, newlines, and other special chars.
    /// Returns `Safe` to bypass Askama's HTML auto-escaping (we handle escaping ourselves).
    pub fn js_string(s: &str) -> ::askama::Result<String> {
        let mut out = String::with_capacity(s.len());
        for c in s.chars() {
            match c {
                '\\' => out.push_str("\\\\"),
                '"' => out.push_str("\\\""),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                '<' => out.push_str("\\x3c"),  // prevent </script> injection
                '>' => out.push_str("\\x3e"),
                _ => out.push(c),
            }
        }
        Ok(out)
    }

    /// Convert tag from storage form (`|` separator) to display form (`/` separator).
    /// Uses simple replacement (no `\/` escaping) since web display is read-only.
    pub fn tag_display(tag: &str) -> ::askama::Result<String> {
        Ok(tag.replace('|', "/"))
    }

    /// Hash a stack ID to an HSL color for visual grouping.
    pub fn stack_color(stack_id: &str) -> ::askama::Result<String> {
        let hash: u32 = stack_id.bytes().fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32));
        let hue = hash % 360;
        Ok(format!("hsl({hue}, 60%, 50%)"))
    }

    pub fn label_color(name: &str) -> ::askama::Result<String> {
        Ok(match name {
            "Red" => "#e74c3c",
            "Orange" => "#e67e22",
            "Yellow" => "#f1c40f",
            "Green" => "#27ae60",
            "Blue" => "#3498db",
            "Pink" => "#e91e8e",
            "Purple" => "#9b59b6",
            _ => "#999",
        }
        .to_string())
    }
}
