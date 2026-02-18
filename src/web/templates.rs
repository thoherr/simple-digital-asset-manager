use askama::Template;

use crate::catalog::{AssetDetails, SearchRow};

/// Compute preview URL from a content hash like "sha256:abcdef...".
pub fn preview_url(content_hash: &str) -> String {
    let hex = content_hash.strip_prefix("sha256:").unwrap_or(content_hash);
    let prefix = &hex[..2.min(hex.len())];
    format!("/preview/{prefix}/{hex}.jpg")
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
}

impl AssetCard {
    pub fn from_row(row: &SearchRow) -> Self {
        Self {
            asset_id: row.asset_id.clone(),
            display_name: row
                .name
                .as_deref()
                .unwrap_or(&row.original_filename)
                .to_string(),
            asset_type: row.asset_type.clone(),
            format: row.format.clone(),
            date: format_date(&row.created_at),
            preview_url: preview_url(&row.content_hash),
            rating: row.rating,
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
}

/// Volume option for dropdowns.
pub struct VolumeOption {
    pub id: String,
    pub label: String,
}

/// Pre-computed variant for template rendering.
pub struct VariantRow {
    pub role: String,
    pub original_filename: String,
    pub format: String,
    pub size: String,
    pub locations: Vec<LocationRow>,
    pub source_metadata: Vec<(String, String)>,
}

pub struct LocationRow {
    pub volume_label: String,
    pub relative_path: String,
}

pub struct RecipeRow {
    pub recipe_type: String,
    pub software: String,
    pub relative_path: String,
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
    pub sort: String,
    pub cards: Vec<AssetCard>,
    pub total: u64,
    pub page: u32,
    pub per_page: u32,
    pub total_pages: u32,
    pub all_tags: Vec<TagOption>,
    pub all_formats: Vec<FormatOption>,
    pub all_volumes: Vec<VolumeOption>,
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
    pub sort: String,
    pub cards: Vec<AssetCard>,
    pub total: u64,
    pub page: u32,
    pub per_page: u32,
    pub total_pages: u32,
}

#[derive(Template)]
#[template(path = "asset.html")]
pub struct AssetPage {
    pub asset_id: String,
    pub display_name: String,
    pub asset_type: String,
    pub created_at: String,
    pub description: Option<String>,
    pub rating: Option<u8>,
    pub tags: Vec<String>,
    pub primary_preview_url: Option<String>,
    pub variants: Vec<VariantRow>,
    pub recipes: Vec<RecipeRow>,
}

impl AssetPage {
    pub fn from_details(details: AssetDetails, preview: Option<String>) -> Self {
        let display_name = details
            .name
            .as_deref()
            .or_else(|| {
                details
                    .variants
                    .first()
                    .map(|v| v.original_filename.as_str())
            })
            .unwrap_or("Untitled")
            .to_string();

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
                    role: v.role.clone(),
                    original_filename: v.original_filename.clone(),
                    format: v.format.clone(),
                    size: format_size(v.file_size),
                    locations: v
                        .locations
                        .iter()
                        .map(|l| LocationRow {
                            volume_label: l.volume_label.clone(),
                            relative_path: l.relative_path.clone(),
                        })
                        .collect(),
                    source_metadata: meta,
                }
            })
            .collect();

        let recipes = details
            .recipes
            .iter()
            .map(|r| RecipeRow {
                recipe_type: r.recipe_type.clone(),
                software: r.software.clone(),
                relative_path: r.relative_path.as_deref().unwrap_or("-").to_string(),
            })
            .collect();

        Self {
            asset_id: details.id,
            display_name,
            asset_type: details.asset_type,
            created_at: format_date(&details.created_at),
            description: details.description,
            rating: details.rating,
            tags: details.tags,
            primary_preview_url: preview,
            variants,
            recipes,
        }
    }
}

#[derive(Template)]
#[template(path = "tags_fragment.html")]
pub struct TagsFragment {
    pub asset_id: String,
    pub tags: Vec<String>,
}

#[derive(Template)]
#[template(path = "rating_fragment.html")]
pub struct RatingFragment {
    pub asset_id: String,
    pub rating: Option<u8>,
}
