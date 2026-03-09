use std::io::{BufWriter, Cursor};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use ab_glyph::FontRef;
use anyhow::{Context, Result};
use image::codecs::jpeg::JpegEncoder;
use image::{Rgb, RgbImage};
use imageproc::drawing::{draw_filled_rect_mut, draw_hollow_rect_mut, draw_text_mut, text_size};
use imageproc::rect::Rect;

use crate::catalog::{AssetDetails, Catalog, SearchRow, VariantDetails};
use crate::models::variant::best_preview_index_details;
use crate::preview::PreviewGenerator;
use crate::query::QueryEngine;

static FONT_DATA: &[u8] = include_bytes!("fonts/DejaVuSans.ttf");

// Page colors (white background, dark text)
const PAGE_BG: Rgb<u8> = Rgb([255, 255, 255]);
const TEXT_COLOR: Rgb<u8> = Rgb([40, 40, 40]);
const DIM_COLOR: Rgb<u8> = Rgb([130, 130, 130]);
const BORDER_COLOR: Rgb<u8> = Rgb([200, 200, 200]);
const HEADER_BG: Rgb<u8> = Rgb([245, 245, 245]);
const PLACEHOLDER_BG: Rgb<u8> = Rgb([220, 220, 220]);
const STAR_COLOR: Rgb<u8> = Rgb([200, 160, 30]);
const STAR_DIM: Rgb<u8> = Rgb([200, 200, 200]);

// Color label RGB values
const LABEL_RED: Rgb<u8> = Rgb([220, 50, 50]);
const LABEL_ORANGE: Rgb<u8> = Rgb([230, 140, 30]);
const LABEL_YELLOW: Rgb<u8> = Rgb([210, 190, 30]);
const LABEL_GREEN: Rgb<u8> = Rgb([50, 170, 70]);
const LABEL_BLUE: Rgb<u8> = Rgb([50, 100, 220]);
const LABEL_PINK: Rgb<u8> = Rgb([210, 80, 160]);
const LABEL_PURPLE: Rgb<u8> = Rgb([130, 60, 200]);

fn label_color(name: &str) -> Option<Rgb<u8>> {
    match name.to_lowercase().as_str() {
        "red" => Some(LABEL_RED),
        "orange" => Some(LABEL_ORANGE),
        "yellow" => Some(LABEL_YELLOW),
        "green" => Some(LABEL_GREEN),
        "blue" => Some(LABEL_BLUE),
        "pink" => Some(LABEL_PINK),
        "purple" => Some(LABEL_PURPLE),
        _ => None,
    }
}

/// How color labels are rendered on contact sheet thumbnails.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LabelStyle {
    /// Colored border around the entire thumbnail cell.
    Border,
    /// Small colored dot next to the filename.
    Dot,
    /// No color label rendering.
    None,
}

impl Default for LabelStyle {
    fn default() -> Self {
        Self::Border
    }
}

impl std::fmt::Display for LabelStyle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Border => write!(f, "border"),
            Self::Dot => write!(f, "dot"),
            Self::None => write!(f, "none"),
        }
    }
}

impl std::str::FromStr for LabelStyle {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "border" => Ok(Self::Border),
            "dot" => Ok(Self::Dot),
            "none" => Ok(Self::None),
            _ => anyhow::bail!("Invalid label style '{}'. Valid: border, dot, none", s),
        }
    }
}

use serde::{Deserialize, Serialize};

/// Layout presets for contact sheets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContactSheetLayout {
    Dense,
    Standard,
    Large,
}

impl ContactSheetLayout {
    fn default_columns(&self) -> u32 {
        match self {
            Self::Dense => 6,
            Self::Standard => 4,
            Self::Large => 3,
        }
    }

    fn default_rows(&self) -> u32 {
        match self {
            Self::Dense => 8,
            Self::Standard => 5,
            Self::Large => 3,
        }
    }

    fn default_fields(&self) -> Vec<MetadataField> {
        match self {
            Self::Dense => vec![MetadataField::Filename],
            Self::Standard => vec![
                MetadataField::Filename,
                MetadataField::Date,
                MetadataField::Rating,
            ],
            Self::Large => vec![
                MetadataField::Filename,
                MetadataField::Date,
                MetadataField::Rating,
                MetadataField::Tags,
            ],
        }
    }
}

impl std::str::FromStr for ContactSheetLayout {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "dense" => Ok(Self::Dense),
            "standard" => Ok(Self::Standard),
            "large" => Ok(Self::Large),
            _ => anyhow::bail!(
                "Unknown layout '{}'. Valid layouts: dense, standard, large",
                s
            ),
        }
    }
}

/// Paper sizes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaperSize {
    A4,
    Letter,
    A3,
}

impl PaperSize {
    /// Paper dimensions in mm (width, height) in portrait orientation.
    fn dimensions_mm(&self) -> (f32, f32) {
        match self {
            Self::A4 => (210.0, 297.0),
            Self::Letter => (215.9, 279.4),
            Self::A3 => (297.0, 420.0),
        }
    }

    /// Paper dimensions in pixels at given DPI.
    fn dimensions_px(&self, dpi: u32, landscape: bool) -> (u32, u32) {
        let (w_mm, h_mm) = self.dimensions_mm();
        let w = (w_mm / 25.4 * dpi as f32).round() as u32;
        let h = (h_mm / 25.4 * dpi as f32).round() as u32;
        if landscape {
            (h, w)
        } else {
            (w, h)
        }
    }

    fn name(&self) -> &'static str {
        match self {
            Self::A4 => "a4",
            Self::Letter => "letter",
            Self::A3 => "a3",
        }
    }
}

impl std::str::FromStr for PaperSize {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "a4" => Ok(Self::A4),
            "letter" => Ok(Self::Letter),
            "a3" => Ok(Self::A3),
            _ => anyhow::bail!("Unknown paper size '{}'. Valid sizes: a4, letter, a3", s),
        }
    }
}

/// Metadata fields that can be displayed below each thumbnail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetadataField {
    Filename,
    Name,
    Date,
    Rating,
    Label,
    Tags,
    Format,
    Id,
    Dimensions,
    Camera,
    Lens,
}

impl std::str::FromStr for MetadataField {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "filename" => Ok(Self::Filename),
            "name" => Ok(Self::Name),
            "date" => Ok(Self::Date),
            "rating" => Ok(Self::Rating),
            "label" => Ok(Self::Label),
            "tags" => Ok(Self::Tags),
            "format" => Ok(Self::Format),
            "id" => Ok(Self::Id),
            "dimensions" => Ok(Self::Dimensions),
            "camera" => Ok(Self::Camera),
            "lens" => Ok(Self::Lens),
            _ => anyhow::bail!(
                "Unknown metadata field '{}'. Valid: filename, name, date, rating, label, tags, format, id, dimensions, camera, lens",
                s
            ),
        }
    }
}

/// Field to group assets by (section headers).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupByField {
    Date,
    Volume,
    Collection,
    Label,
}

impl std::str::FromStr for GroupByField {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "date" => Ok(Self::Date),
            "volume" => Ok(Self::Volume),
            "collection" => Ok(Self::Collection),
            "label" => Ok(Self::Label),
            _ => anyhow::bail!(
                "Unknown group-by field '{}'. Valid: date, volume, collection, label",
                s
            ),
        }
    }
}

/// Configuration for contact sheet generation.
pub struct ContactSheetConfig {
    pub layout: ContactSheetLayout,
    pub columns: Option<u32>,
    pub rows: Option<u32>,
    pub paper: PaperSize,
    pub landscape: bool,
    pub title: Option<String>,
    pub fields: Option<Vec<MetadataField>>,
    pub sort: Option<String>,
    pub use_smart_previews: bool,
    pub group_by: Option<GroupByField>,
    pub margin_mm: f32,
    pub label_style: LabelStyle,
    pub quality: u8,
    pub copyright: Option<String>,
}

impl Default for ContactSheetConfig {
    fn default() -> Self {
        Self {
            layout: ContactSheetLayout::Standard,
            columns: None,
            rows: None,
            paper: PaperSize::A4,
            landscape: false,
            title: None,
            fields: None,
            sort: None,
            use_smart_previews: true,
            group_by: None,
            margin_mm: 10.0,
            label_style: LabelStyle::Border,
            quality: 90,
            copyright: None,
        }
    }
}

impl ContactSheetConfig {
    fn effective_columns(&self) -> u32 {
        self.columns.unwrap_or_else(|| self.layout.default_columns())
    }

    fn effective_rows(&self) -> u32 {
        self.rows.unwrap_or_else(|| self.layout.default_rows())
    }

    fn effective_fields(&self) -> Vec<MetadataField> {
        self.fields
            .clone()
            .unwrap_or_else(|| self.layout.default_fields())
    }

    /// Whether any requested fields need full AssetDetails (beyond SearchRow).
    fn needs_details(&self) -> bool {
        let fields = self.effective_fields();
        fields.iter().any(|f| matches!(f, MetadataField::Dimensions | MetadataField::Camera | MetadataField::Lens))
    }
}

/// Result of a contact sheet generation.
#[derive(Debug, Serialize)]
pub struct ContactSheetResult {
    pub assets: usize,
    pub pages: usize,
    pub layout: String,
    pub paper: String,
    pub output: String,
    pub dry_run: bool,
    pub errors: Vec<String>,
}

/// Progress callback status.
pub enum ContactSheetStatus {
    Rendering,
    Complete,
    Error,
}

const DPI: u32 = 300;
const HEADER_HEIGHT_MM: f32 = 8.0;
const FOOTER_HEIGHT_MM: f32 = 6.0;
const CELL_GAP_MM: f32 = 4.0;
const SECTION_HEADER_HEIGHT_MM: f32 = 8.0;
const LABEL_BORDER_WIDTH: u32 = 6; // pixels at 300 DPI (~0.5mm)

fn mm_to_px(mm: f32) -> u32 {
    (mm / 25.4 * DPI as f32).round() as u32
}

/// Metadata extracted for a single cell.
struct CellData {
    /// Path to the preview image file.
    preview_path: Option<PathBuf>,
    /// Collected metadata field values (in order of `fields`).
    field_values: Vec<String>,
    /// Color label name (if any).
    color_label: Option<String>,
    /// Rating (if any).
    rating: Option<u8>,
    /// Group-by value (for section headers).
    group_value: Option<String>,
    /// Original filename (for placeholder text).
    filename: String,
}

/// Generate a contact sheet PDF.
pub fn generate_contact_sheet(
    catalog_root: &Path,
    query: &str,
    output: &Path,
    config: &ContactSheetConfig,
    dry_run: bool,
    on_progress: impl Fn(&str, ContactSheetStatus, Duration),
) -> Result<ContactSheetResult> {
    let engine = QueryEngine::new(catalog_root);
    let preview_config = crate::config::PreviewConfig::default();
    let preview_gen = PreviewGenerator::new(catalog_root, false, &preview_config);

    // Search for matching assets
    let results = engine.search(query)?;
    if results.is_empty() {
        anyhow::bail!("No assets match the query");
    }

    let cols = config.effective_columns();
    let rows = config.effective_rows();
    let cells_per_page = cols * rows;
    let fields = config.effective_fields();

    // Collect cell data
    let needs_details = config.needs_details();
    let catalog = if needs_details {
        Some(Catalog::open(catalog_root)?)
    } else {
        None
    };

    let mut cells: Vec<CellData> = Vec::with_capacity(results.len());
    for row in &results {
        let details = if needs_details {
            catalog.as_ref().and_then(|c| c.load_asset_details(&row.asset_id).ok().flatten())
        } else {
            None
        };

        let preview_path = resolve_preview_path(&preview_gen, row, config.use_smart_previews);
        let field_values = collect_field_values(row, details.as_ref(), &fields);
        let group_value = config.group_by.map(|g| group_value_for(row, g));

        cells.push(CellData {
            preview_path,
            field_values,
            color_label: row.color_label.clone(),
            rating: row.rating,
            group_value,
            filename: row.original_filename.clone(),
        });
    }

    // Calculate page count (account for section headers consuming row space)
    let page_count = if config.group_by.is_some() {
        count_pages_with_groups(&cells, cols, rows)
    } else {
        (cells.len() as u32 + cells_per_page - 1) / cells_per_page
    };

    let layout_name = match config.layout {
        ContactSheetLayout::Dense => "dense",
        ContactSheetLayout::Standard => "standard",
        ContactSheetLayout::Large => "large",
    };
    let orientation = if config.landscape { "landscape" } else { "portrait" };

    if dry_run {
        let msg = format!(
            "Contact sheet: {} assets, {} pages ({}, {} {})",
            cells.len(),
            page_count,
            layout_name,
            config.paper.name().to_uppercase(),
            orientation,
        );
        on_progress(&msg, ContactSheetStatus::Complete, Duration::ZERO);
        return Ok(ContactSheetResult {
            assets: cells.len(),
            pages: page_count as usize,
            layout: layout_name.to_string(),
            paper: config.paper.name().to_string(),
            output: output.to_string_lossy().to_string(),
            dry_run: true,
            errors: vec![],
        });
    }

    // Check output is writable
    if let Some(parent) = output.parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Cannot create output directory: {}", parent.display()))?;
        }
    }

    // Render pages
    let (page_w, page_h) = config.paper.dimensions_px(DPI, config.landscape);
    let margin = mm_to_px(config.margin_mm);
    let header_h = mm_to_px(HEADER_HEIGHT_MM);
    let footer_h = mm_to_px(FOOTER_HEIGHT_MM);
    let cell_gap = mm_to_px(CELL_GAP_MM);

    let content_w = page_w - 2 * margin;
    let content_h = page_h - 2 * margin - header_h - footer_h;

    // Cell sizing
    let total_gap_x = cell_gap * (cols - 1);
    let total_gap_y = cell_gap * (rows - 1);
    let cell_w = (content_w - total_gap_x) / cols;
    let cell_h = (content_h - total_gap_y) / rows;

    // Within each cell: image area + text area
    let field_count = fields.len() as u32;
    let text_line_height = cell_h / (12 + field_count * 2).max(1); // approximate
    let text_area_h = text_line_height * field_count + if field_count > 0 { cell_gap / 2 } else { 0 };
    let img_area_h = cell_h.saturating_sub(text_area_h);

    let font = FontRef::try_from_slice(FONT_DATA).expect("embedded font is valid");

    // Font scale based on cell width
    let meta_scale = (cell_w as f32 / 12.0).clamp(8.0, 24.0);
    let header_scale = (page_w as f32 / 60.0).clamp(14.0, 36.0);
    let footer_scale = header_scale * 0.7;
    let section_scale = header_scale * 0.85;

    let mut page_jpegs: Vec<Vec<u8>> = Vec::new();
    let mut errors: Vec<String> = Vec::new();

    // Paginate cells (with group headers)
    let pages = paginate_cells(&cells, cols, rows, config.group_by.is_some());

    let title_text = config.title.as_deref().unwrap_or("");
    let total_pages = pages.len();

    for (page_idx, page_items) in pages.iter().enumerate() {
        let start = Instant::now();
        let mut img = RgbImage::from_pixel(page_w, page_h, PAGE_BG);

        // Draw header
        draw_header(
            &mut img,
            &font,
            header_scale,
            margin,
            page_w,
            title_text,
            query,
            page_idx,
        );

        // Draw footer
        draw_footer(
            &mut img,
            &font,
            footer_scale,
            margin,
            page_w,
            page_h,
            footer_h,
            page_idx + 1,
            total_pages,
            config.copyright.as_deref(),
        );

        // Draw cells
        let content_top = margin + header_h;
        let mut grid_row = 0u32;
        let mut grid_col = 0u32;

        for item in page_items {
            match item {
                PageItem::SectionHeader(text) => {
                    let y = content_top + grid_row * (cell_h + cell_gap);
                    draw_section_header(&mut img, &font, section_scale, margin, content_w, y, text);
                    grid_row += 1;
                    grid_col = 0;
                }
                PageItem::Cell(cell_idx) => {
                    let cell = &cells[*cell_idx];
                    let x = margin + grid_col * (cell_w + cell_gap);
                    let y = content_top + grid_row * (cell_h + cell_gap);

                    draw_cell(
                        &mut img,
                        &font,
                        meta_scale,
                        x,
                        y,
                        cell_w,
                        img_area_h,
                        cell_h,
                        cell,
                        &fields,
                        &config.label_style,
                        &mut errors,
                    );

                    grid_col += 1;
                    if grid_col >= cols {
                        grid_col = 0;
                        grid_row += 1;
                    }
                }
            }
        }

        // Encode page as JPEG
        let mut buf = Vec::new();
        {
            let writer = BufWriter::new(Cursor::new(&mut buf));
            let mut encoder = JpegEncoder::new_with_quality(writer, config.quality);
            encoder
                .encode(img.as_raw(), page_w, page_h, image::ExtendedColorType::Rgb8)
                .with_context(|| format!("Failed to encode page {}", page_idx + 1))?;
        }

        on_progress(
            &format!("Page {}/{}", page_idx + 1, total_pages),
            ContactSheetStatus::Rendering,
            start.elapsed(),
        );

        page_jpegs.push(buf);
    }

    // Create PDF
    write_pdf(output, &page_jpegs, page_w, page_h)?;

    on_progress(
        &format!("Written {}", output.display()),
        ContactSheetStatus::Complete,
        Duration::ZERO,
    );

    Ok(ContactSheetResult {
        assets: cells.len(),
        pages: total_pages,
        layout: layout_name.to_string(),
        paper: config.paper.name().to_string(),
        output: output.to_string_lossy().to_string(),
        dry_run: false,
        errors,
    })
}

// ── Preview resolution ──────────────────────────────────────────────────────

fn resolve_preview_path(
    gen: &PreviewGenerator,
    row: &SearchRow,
    prefer_smart: bool,
) -> Option<PathBuf> {
    let hash = &row.content_hash;
    if prefer_smart {
        let smart = gen.smart_preview_path(hash);
        if smart.exists() {
            return Some(smart);
        }
    }
    let regular = gen.preview_path(hash);
    if regular.exists() {
        Some(regular)
    } else {
        None
    }
}

// ── Field value collection ──────────────────────────────────────────────────

fn collect_field_values(
    row: &SearchRow,
    details: Option<&AssetDetails>,
    fields: &[MetadataField],
) -> Vec<String> {
    fields
        .iter()
        .filter_map(|f| {
            let val = match f {
                MetadataField::Filename => Some(row.original_filename.clone()),
                MetadataField::Name => row
                    .name
                    .as_ref()
                    .or(Some(&row.original_filename))
                    .cloned(),
                MetadataField::Date => {
                    // Extract just the date part (YYYY-MM-DD)
                    Some(row.created_at.chars().take(10).collect())
                }
                MetadataField::Rating => row.rating.map(|r| {
                    (1..=5)
                        .map(|i| if i <= r { '\u{2605}' } else { '\u{2606}' })
                        .collect()
                }),
                MetadataField::Label => row.color_label.clone(),
                MetadataField::Tags => {
                    if row.tags.is_empty() {
                        None
                    } else {
                        Some(row.tags.join(", "))
                    }
                }
                MetadataField::Format => Some(row.display_format().to_string()),
                MetadataField::Id => Some(row.asset_id.chars().take(8).collect()),
                MetadataField::Dimensions => {
                    details.and_then(|d| {
                        let v = best_variant(d)?;
                        let w = v.source_metadata.get("image_width")?;
                        let h = v.source_metadata.get("image_height")?;
                        Some(format!("{}×{}", w, h))
                    })
                }
                MetadataField::Camera => {
                    details.and_then(|d| {
                        let v = best_variant(d)?;
                        v.source_metadata.get("camera_model").cloned()
                    })
                }
                MetadataField::Lens => {
                    details.and_then(|d| {
                        let v = best_variant(d)?;
                        v.source_metadata.get("lens").cloned()
                    })
                }
            };
            val.filter(|s| !s.is_empty())
        })
        .collect()
}

fn best_variant(details: &AssetDetails) -> Option<&VariantDetails> {
    best_preview_index_details(&details.variants).map(|i| &details.variants[i])
}

fn group_value_for(row: &SearchRow, field: GroupByField) -> String {
    match field {
        GroupByField::Date => row.created_at.chars().take(10).collect(),
        GroupByField::Label => row
            .color_label
            .as_deref()
            .unwrap_or("No label")
            .to_string(),
        // Volume and Collection require extra lookups; use a simplified approach
        GroupByField::Volume | GroupByField::Collection => String::new(),
    }
}

// ── Pagination ──────────────────────────────────────────────────────────────

enum PageItem {
    SectionHeader(String),
    Cell(usize),
}

fn paginate_cells(
    cells: &[CellData],
    cols: u32,
    rows: u32,
    has_groups: bool,
) -> Vec<Vec<PageItem>> {
    let mut pages: Vec<Vec<PageItem>> = Vec::new();
    let mut current_page: Vec<PageItem> = Vec::new();
    let mut rows_used: u32 = 0;
    let mut col_in_row: u32 = 0;
    let mut last_group: Option<&str> = None;

    for (i, cell) in cells.iter().enumerate() {
        // Check for section header
        if has_groups {
            let gv = cell.group_value.as_deref().unwrap_or("");
            let need_header = match last_group {
                Some(prev) => prev != gv,
                None => true,
            };
            if need_header {
                // Finish current row if partially filled
                if col_in_row > 0 {
                    rows_used += 1;
                    col_in_row = 0;
                }
                // Widow prevention: if header would be last row, start new page
                if rows_used >= rows || (rows_used == rows - 1) {
                    if !current_page.is_empty() {
                        pages.push(current_page);
                        current_page = Vec::new();
                    }
                    rows_used = 0;
                }
                current_page.push(PageItem::SectionHeader(gv.to_string()));
                rows_used += 1;
                last_group = cell.group_value.as_deref();
            }
        }

        // Check if we need a new page
        if col_in_row == 0 && rows_used >= rows {
            pages.push(current_page);
            current_page = Vec::new();
            rows_used = 0;
        }

        current_page.push(PageItem::Cell(i));
        col_in_row += 1;
        if col_in_row >= cols {
            col_in_row = 0;
            rows_used += 1;
        }
    }

    if !current_page.is_empty() {
        pages.push(current_page);
    }
    pages
}

fn count_pages_with_groups(cells: &[CellData], cols: u32, rows: u32) -> u32 {
    paginate_cells(cells, cols, rows, true).len() as u32
}

// ── Drawing functions ───────────────────────────────────────────────────────

fn draw_header(
    img: &mut RgbImage,
    font: &FontRef,
    scale: f32,
    margin: u32,
    page_w: u32,
    title: &str,
    query: &str,
    page_idx: usize,
) {
    let header_h = mm_to_px(HEADER_HEIGHT_MM);
    let y = margin as i32;

    // Background strip
    draw_filled_rect_mut(
        img,
        Rect::at(margin as i32, y).of_size(page_w - 2 * margin, header_h),
        HEADER_BG,
    );

    let text_y = y + (header_h as i32 - scale as i32) / 2;

    // Title left
    if !title.is_empty() {
        draw_text_mut(img, TEXT_COLOR, margin as i32 + 10, text_y, scale, font, title);
    }

    // Query right (all pages)
    if !query.is_empty() {
        let small_scale = scale * 0.7;
        let (qw, _) = text_size(small_scale, font, query);
        let qx = (page_w - margin) as i32 - qw as i32 - 10;
        draw_text_mut(img, DIM_COLOR, qx, text_y, small_scale, font, query);
    }
}

fn draw_footer(
    img: &mut RgbImage,
    font: &FontRef,
    scale: f32,
    margin: u32,
    page_w: u32,
    page_h: u32,
    footer_h: u32,
    page_num: usize,
    total_pages: usize,
    copyright: Option<&str>,
) {
    let y = (page_h - margin - footer_h) as i32;
    let text_y = y + (footer_h as i32 - scale as i32) / 2;

    // Left: branding + version + date
    let version = env!("CARGO_PKG_VERSION");
    let date = chrono::Local::now().format("%Y-%m-%d").to_string();
    let left_text = format!("dam v{} \u{2022} {}", version, date);
    draw_text_mut(img, DIM_COLOR, margin as i32 + 10, text_y, scale, font, &left_text);

    // Center: copyright text (if provided)
    if let Some(cr) = copyright {
        if !cr.is_empty() {
            let (cw, _) = text_size(scale, font, cr);
            let cx = (page_w as i32 - cw as i32) / 2;
            draw_text_mut(img, DIM_COLOR, cx, text_y, scale, font, cr);
        }
    }

    // Right: page N of M
    let right_text = format!("Page {} of {}", page_num, total_pages);
    let (rw, _) = text_size(scale, font, &right_text);
    let rx = (page_w - margin) as i32 - rw as i32 - 10;
    draw_text_mut(img, DIM_COLOR, rx, text_y, scale, font, &right_text);
}

fn draw_section_header(
    img: &mut RgbImage,
    font: &FontRef,
    scale: f32,
    margin: u32,
    content_w: u32,
    y: u32,
    text: &str,
) {
    let h = mm_to_px(SECTION_HEADER_HEIGHT_MM);
    let text_y = y as i32 + (h as i32 - scale as i32) / 2;

    // Line
    let line_y = y as i32 + h as i32 / 2;
    draw_filled_rect_mut(
        img,
        Rect::at(margin as i32, line_y).of_size(content_w, 2),
        BORDER_COLOR,
    );

    // Text with white background padding
    if !text.is_empty() {
        let (tw, th) = text_size(scale, font, text);
        let tx = margin as i32 + 20;
        // Clear area behind text
        draw_filled_rect_mut(
            img,
            Rect::at(tx - 8, text_y - 2).of_size(tw + 16, th + 4),
            PAGE_BG,
        );
        draw_text_mut(img, DIM_COLOR, tx, text_y, scale, font, text);
    }
}

fn draw_cell(
    img: &mut RgbImage,
    font: &FontRef,
    meta_scale: f32,
    x: u32,
    y: u32,
    cell_w: u32,
    img_area_h: u32,
    cell_h: u32,
    cell: &CellData,
    fields: &[MetadataField],
    label_style: &LabelStyle,
    errors: &mut Vec<String>,
) {
    // Draw thin border
    draw_hollow_rect_mut(
        img,
        Rect::at(x as i32, y as i32).of_size(cell_w, cell_h),
        BORDER_COLOR,
    );

    // Color label border
    if *label_style == LabelStyle::Border {
        if let Some(ref label_name) = cell.color_label {
            if let Some(color) = label_color(label_name) {
                for i in 1..=LABEL_BORDER_WIDTH {
                    draw_hollow_rect_mut(
                        img,
                        Rect::at(x as i32 + i as i32, y as i32 + i as i32)
                            .of_size(cell_w - 2 * i, cell_h - 2 * i),
                        color,
                    );
                }
            }
        }
    }

    // Draw thumbnail
    let thumb_margin = LABEL_BORDER_WIDTH + 2;
    let avail_w = cell_w.saturating_sub(2 * thumb_margin);
    let avail_h = img_area_h.saturating_sub(thumb_margin);

    match &cell.preview_path {
        Some(path) => {
            match image::open(path) {
                Ok(thumb_img) => {
                    let thumb = thumb_img.to_rgb8();
                    let (tw, th) = (thumb.width(), thumb.height());

                    // Scale to fit
                    let scale_x = avail_w as f32 / tw as f32;
                    let scale_y = avail_h as f32 / th as f32;
                    let scale = scale_x.min(scale_y).min(1.0);
                    let new_w = (tw as f32 * scale) as u32;
                    let new_h = (th as f32 * scale) as u32;

                    let resized = image::imageops::resize(
                        &thumb,
                        new_w.max(1),
                        new_h.max(1),
                        image::imageops::FilterType::Triangle,
                    );

                    // Center in image area
                    let cx = x + thumb_margin + (avail_w.saturating_sub(new_w)) / 2;
                    let cy = y + thumb_margin + (avail_h.saturating_sub(new_h)) / 2;

                    image::imageops::overlay(img, &resized, cx as i64, cy as i64);
                }
                Err(e) => {
                    errors.push(format!("{}: {}", path.display(), e));
                    draw_placeholder(img, x + thumb_margin, y + thumb_margin, avail_w, avail_h, font, &cell.filename);
                }
            }
        }
        None => {
            draw_placeholder(img, x + thumb_margin, y + thumb_margin, avail_w, avail_h, font, &cell.filename);
        }
    }

    // Draw metadata text below image
    let text_top = y + img_area_h + 4;
    let max_text_w = cell_w.saturating_sub(2 * thumb_margin);
    let line_spacing = (meta_scale * 1.3) as u32;
    let mut ty = text_top;

    for (i, field) in fields.iter().enumerate() {
        if i >= cell.field_values.len() {
            break;
        }
        let value = &cell.field_values[i];

        // Color label dot next to filename
        if i == 0 && *label_style == LabelStyle::Dot {
            if let Some(ref label_name) = cell.color_label {
                if let Some(color) = label_color(label_name) {
                    let dot_size = (meta_scale * 0.7) as u32;
                    let dot_x = x + thumb_margin;
                    let dot_y = ty + (meta_scale as u32 - dot_size) / 2;
                    draw_filled_rect_mut(
                        img,
                        Rect::at(dot_x as i32, dot_y as i32).of_size(dot_size, dot_size),
                        color,
                    );
                    // Draw text after dot
                    let text_x = dot_x + dot_size + 4;
                    let truncated = truncate_to_width(value, meta_scale, font, max_text_w.saturating_sub(dot_size + 4));
                    draw_text_mut(img, TEXT_COLOR, text_x as i32, ty as i32, meta_scale, font, &truncated);
                    ty += line_spacing;
                    continue;
                }
            }
        }

        // Rating stars get special color
        if matches!(field, MetadataField::Rating) {
            if let Some(rating) = cell.rating {
                draw_rating_stars(img, font, meta_scale, x + thumb_margin, ty, rating);
                ty += line_spacing;
                continue;
            }
        }

        let truncated = truncate_to_width(value, meta_scale, font, max_text_w);
        let color = if matches!(field, MetadataField::Date | MetadataField::Id | MetadataField::Label) {
            DIM_COLOR
        } else {
            TEXT_COLOR
        };
        draw_text_mut(img, color, (x + thumb_margin) as i32, ty as i32, meta_scale, font, &truncated);
        ty += line_spacing;
    }
}

fn draw_placeholder(
    img: &mut RgbImage,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    font: &FontRef,
    filename: &str,
) {
    draw_filled_rect_mut(
        img,
        Rect::at(x as i32, y as i32).of_size(w, h),
        PLACEHOLDER_BG,
    );

    // Draw filename centered
    let scale = (w as f32 / 15.0).clamp(6.0, 14.0);
    let truncated = truncate_to_width(filename, scale, font, w.saturating_sub(10));
    let (tw, th) = text_size(scale, font, &truncated);
    let tx = x as i32 + (w as i32 - tw as i32) / 2;
    let ty = y as i32 + (h as i32 - th as i32) / 2;
    draw_text_mut(img, DIM_COLOR, tx, ty, scale, font, &truncated);
}

fn draw_rating_stars(
    img: &mut RgbImage,
    font: &FontRef,
    scale: f32,
    x: u32,
    y: u32,
    rating: u8,
) {
    let mut cx = x as i32;
    for i in 1..=5u8 {
        let ch = if i <= rating { "\u{2605}" } else { "\u{2606}" };
        let color = if i <= rating { STAR_COLOR } else { STAR_DIM };
        draw_text_mut(img, color, cx, y as i32, scale, font, ch);
        let (cw, _) = text_size(scale, font, ch);
        cx += cw as i32 + 1;
    }
}

fn truncate_to_width(text: &str, scale: f32, font: &FontRef, max_width: u32) -> String {
    let (w, _) = text_size(scale, font, text);
    if w <= max_width {
        return text.to_string();
    }

    let ellipsis = "...";
    let (ew, _) = text_size(scale, font, ellipsis);
    let target = max_width.saturating_sub(ew);

    let chars: Vec<char> = text.chars().collect();
    let mut lo = 0usize;
    let mut hi = chars.len();
    while lo < hi {
        let mid = (lo + hi + 1) / 2;
        let prefix: String = chars[..mid].iter().collect();
        let (pw, _) = text_size(scale, font, &prefix);
        if pw <= target {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }

    if lo == 0 {
        ellipsis.to_string()
    } else {
        let prefix: String = chars[..lo].iter().collect();
        format!("{}{}", prefix, ellipsis)
    }
}

// ── PDF generation ──────────────────────────────────────────────────────────

fn write_pdf(output: &Path, page_jpegs: &[Vec<u8>], page_w: u32, page_h: u32) -> Result<()> {
    use printpdf::*;

    // Convert pixel dimensions to mm for PDF page size
    let w_mm = page_w as f32 / DPI as f32 * 25.4;
    let h_mm = page_h as f32 / DPI as f32 * 25.4;

    let (doc, page1, layer1) = PdfDocument::new(
        "Contact Sheet",
        Mm(w_mm),
        Mm(h_mm),
        "Layer 1",
    );

    for (i, jpeg_data) in page_jpegs.iter().enumerate() {
        let (page_ref, layer_ref) = if i == 0 {
            (page1, layer1)
        } else {
            let (p, l) = doc.add_page(Mm(w_mm), Mm(h_mm), "Layer 1");
            (p, l)
        };

        let current_layer = doc.get_page(page_ref).get_layer(layer_ref);

        let image = Image::from(ImageXObject {
            width: Px(page_w as usize),
            height: Px(page_h as usize),
            color_space: ColorSpace::Rgb,
            bits_per_component: ColorBits::Bit8,
            interpolate: true,
            image_data: decode_jpeg_to_raw(jpeg_data)?,
            image_filter: None,
            smask: None,
            clipping_bbox: None,
        });

        image.add_to_layer(
            current_layer,
            ImageTransform {
                translate_x: Some(Mm(0.0)),
                translate_y: Some(Mm(0.0)),
                scale_x: Some(w_mm / (page_w as f32 / DPI as f32 * 25.4)),
                scale_y: Some(h_mm / (page_h as f32 / DPI as f32 * 25.4)),
                ..Default::default()
            },
        );
    }

    let pdf_bytes = doc.save_to_bytes()?;
    std::fs::write(output, pdf_bytes)
        .with_context(|| format!("Failed to write PDF to {}", output.display()))?;

    Ok(())
}

fn decode_jpeg_to_raw(jpeg_data: &[u8]) -> Result<Vec<u8>> {
    let img = image::load_from_memory_with_format(jpeg_data, image::ImageFormat::Jpeg)
        .context("Failed to decode JPEG page image")?;
    Ok(img.to_rgb8().into_raw())
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paper_size_dimensions() {
        let (w, h) = PaperSize::A4.dimensions_px(300, false);
        assert_eq!(w, 2480);
        assert_eq!(h, 3508);

        let (w, h) = PaperSize::A4.dimensions_px(300, true);
        assert_eq!(w, 3508);
        assert_eq!(h, 2480);
    }

    #[test]
    fn paper_size_letter() {
        let (w, h) = PaperSize::Letter.dimensions_px(300, false);
        assert!(w > 2500 && w < 2600);
        assert!(h > 3200 && h < 3400);
    }

    #[test]
    fn layout_defaults() {
        assert_eq!(ContactSheetLayout::Dense.default_columns(), 6);
        assert_eq!(ContactSheetLayout::Dense.default_rows(), 8);
        assert_eq!(ContactSheetLayout::Standard.default_columns(), 4);
        assert_eq!(ContactSheetLayout::Standard.default_rows(), 5);
        assert_eq!(ContactSheetLayout::Large.default_columns(), 3);
        assert_eq!(ContactSheetLayout::Large.default_rows(), 3);
    }

    #[test]
    fn parse_layout() {
        assert_eq!("dense".parse::<ContactSheetLayout>().unwrap(), ContactSheetLayout::Dense);
        assert_eq!("Standard".parse::<ContactSheetLayout>().unwrap(), ContactSheetLayout::Standard);
        assert_eq!("LARGE".parse::<ContactSheetLayout>().unwrap(), ContactSheetLayout::Large);
        assert!("unknown".parse::<ContactSheetLayout>().is_err());
    }

    #[test]
    fn parse_paper_size() {
        assert_eq!("a4".parse::<PaperSize>().unwrap(), PaperSize::A4);
        assert_eq!("Letter".parse::<PaperSize>().unwrap(), PaperSize::Letter);
        assert_eq!("A3".parse::<PaperSize>().unwrap(), PaperSize::A3);
        assert!("legal".parse::<PaperSize>().is_err());
    }

    #[test]
    fn parse_metadata_fields() {
        assert_eq!("filename".parse::<MetadataField>().unwrap(), MetadataField::Filename);
        assert_eq!("Rating".parse::<MetadataField>().unwrap(), MetadataField::Rating);
        assert_eq!("CAMERA".parse::<MetadataField>().unwrap(), MetadataField::Camera);
        assert!("unknown".parse::<MetadataField>().is_err());
    }

    #[test]
    fn parse_group_by() {
        assert_eq!("date".parse::<GroupByField>().unwrap(), GroupByField::Date);
        assert_eq!("Label".parse::<GroupByField>().unwrap(), GroupByField::Label);
        assert!("invalid".parse::<GroupByField>().is_err());
    }

    #[test]
    fn parse_label_style() {
        assert_eq!("border".parse::<LabelStyle>().unwrap(), LabelStyle::Border);
        assert_eq!("Dot".parse::<LabelStyle>().unwrap(), LabelStyle::Dot);
        assert_eq!("NONE".parse::<LabelStyle>().unwrap(), LabelStyle::None);
        assert!("invalid".parse::<LabelStyle>().is_err());
    }

    #[test]
    fn effective_config() {
        let config = ContactSheetConfig::default();
        assert_eq!(config.effective_columns(), 4);
        assert_eq!(config.effective_rows(), 5);
        assert_eq!(config.effective_fields().len(), 3);

        let config = ContactSheetConfig {
            columns: Some(6),
            rows: Some(8),
            ..Default::default()
        };
        assert_eq!(config.effective_columns(), 6);
        assert_eq!(config.effective_rows(), 8);
    }

    #[test]
    fn needs_details_basic_fields() {
        let config = ContactSheetConfig::default();
        assert!(!config.needs_details());
    }

    #[test]
    fn needs_details_camera_field() {
        let config = ContactSheetConfig {
            fields: Some(vec![MetadataField::Camera]),
            ..Default::default()
        };
        assert!(config.needs_details());
    }

    #[test]
    fn mm_to_px_conversion() {
        assert_eq!(mm_to_px(25.4), 300); // 1 inch = 300 px at 300 DPI
    }

    #[test]
    fn label_color_lookup() {
        assert!(label_color("Red").is_some());
        assert!(label_color("red").is_some());
        assert!(label_color("BLUE").is_some());
        assert!(label_color("invalid").is_none());
    }

    #[test]
    fn pagination_simple() {
        let cells: Vec<CellData> = (0..10)
            .map(|_| CellData {
                preview_path: None,
                field_values: vec![],
                color_label: None,
                rating: None,
                group_value: None,
                filename: "test.jpg".into(),
            })
            .collect();

        let pages = paginate_cells(&cells, 4, 5, false);
        assert_eq!(pages.len(), 1); // 10 cells fit on 1 page (4*5=20)
    }

    #[test]
    fn pagination_multi_page() {
        let cells: Vec<CellData> = (0..25)
            .map(|_| CellData {
                preview_path: None,
                field_values: vec![],
                color_label: None,
                rating: None,
                group_value: None,
                filename: "test.jpg".into(),
            })
            .collect();

        let pages = paginate_cells(&cells, 4, 5, false);
        assert_eq!(pages.len(), 2); // 25 cells, 20 per page
    }

    #[test]
    fn pagination_exact_fit() {
        let cells: Vec<CellData> = (0..20)
            .map(|_| CellData {
                preview_path: None,
                field_values: vec![],
                color_label: None,
                rating: None,
                group_value: None,
                filename: "test.jpg".into(),
            })
            .collect();

        let pages = paginate_cells(&cells, 4, 5, false);
        assert_eq!(pages.len(), 1); // exactly 20 = 4*5
    }

    #[test]
    fn pagination_one_over() {
        let cells: Vec<CellData> = (0..21)
            .map(|_| CellData {
                preview_path: None,
                field_values: vec![],
                color_label: None,
                rating: None,
                group_value: None,
                filename: "test.jpg".into(),
            })
            .collect();

        let pages = paginate_cells(&cells, 4, 5, false);
        assert_eq!(pages.len(), 2); // 21 cells overflow to page 2
    }

    #[test]
    fn pagination_with_groups() {
        let cells: Vec<CellData> = (0..8)
            .map(|i| CellData {
                preview_path: None,
                field_values: vec![],
                color_label: None,
                rating: None,
                group_value: Some(if i < 4 { "A" } else { "B" }.into()),
                filename: "test.jpg".into(),
            })
            .collect();

        // 4 cols, 5 rows. Group A header (1 row) + 4 cells (1 row) + Group B header (1 row) + 4 cells (1 row) = 4 rows
        let pages = paginate_cells(&cells, 4, 5, true);
        assert_eq!(pages.len(), 1);
    }

    #[test]
    fn truncation() {
        let font = FontRef::try_from_slice(FONT_DATA).unwrap();
        let text = "This is a very long filename that should be truncated.jpg";
        let truncated = truncate_to_width(text, 12.0, &font, 100);
        assert!(truncated.len() < text.len());
        assert!(truncated.ends_with("..."));
    }

    #[test]
    fn truncation_short_text_unchanged() {
        let font = FontRef::try_from_slice(FONT_DATA).unwrap();
        let text = "short.jpg";
        let truncated = truncate_to_width(text, 12.0, &font, 500);
        assert_eq!(truncated, text);
    }

    #[test]
    fn label_style_default_is_border() {
        assert_eq!(LabelStyle::default(), LabelStyle::Border);
    }
}
