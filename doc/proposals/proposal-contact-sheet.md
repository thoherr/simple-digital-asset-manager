# Proposal: PDF Contact Sheet Export

## Motivation

Photographers regularly need printed or shareable overview sheets of their work: reviewing a shoot with a client, selecting images for a portfolio, archiving a visual index alongside offline drives, or submitting a proof sheet to an agency. Currently, generating a contact sheet requires exporting files and using an external tool (Lightroom, InDesign, ImageMagick `montage`). A built-in command would complete the workflow inside the DAM.

## Goals

- Generate multi-page PDF contact sheets from any search query
- Include configurable metadata annotations per thumbnail
- Support multiple layout presets (dense proof, presentation, archive index)
- Reuse existing preview images (no re-rendering from originals)
- Work offline (no external dependencies beyond what's already compiled in)

## Non-Goals

- Print-quality output from RAW files (use `dam export` + dedicated print software)
- Interactive PDF features (forms, links, JavaScript)
- Multi-column text layouts or report-style documents

---

## Command Interface

```
dam contact-sheet <QUERY> <OUTPUT> [OPTIONS]
```

### Positional Arguments

| Argument | Description |
|---|---|
| `QUERY` | Search query string (same syntax as `dam search`) |
| `OUTPUT` | Output file path (`.pdf` extension) |

### Options

| Flag | Default | Description |
|---|---|---|
| `--layout <PRESET>` | `standard` | Layout preset: `dense`, `standard`, `large` |
| `--columns <N>` | per preset | Override number of columns |
| `--rows <N>` | per preset | Override rows per page |
| `--paper <SIZE>` | `a4` | Paper size: `a4`, `letter`, `a3` |
| `--landscape` | (portrait) | Landscape orientation |
| `--title <TEXT>` | (none) | Title printed on first page header |
| `--fields <LIST>` | `filename,date,rating` | Comma-separated metadata fields below each thumbnail |
| `--sort <SORT>` | query default | Override sort: `date`, `name`, `rating`, `filename` |
| `--smart` | (off) | Use smart previews (2560px) instead of regular (800px) |
| `--group-by <FIELD>` | (none) | Insert section headers when field value changes: `date`, `volume`, `collection`, `label` |
| `--margin <MM>` | `10` | Page margin in mm |
| `--dry-run` | (off) | Report page count and asset count without generating |
| `--json` | (off) | JSON output (`ContactSheetResult`) |
| `--log` | (off) | Per-asset progress to stderr |
| `--time` | (off) | Elapsed time |

### Layout Presets

| Preset | Columns | Rows | Thumb size | Fields | Use case |
|---|---|---|---|---|---|
| `dense` | 6 | 8 | ~55mm | filename only | Archive index, maximum coverage |
| `standard` | 4 | 5 | ~80mm | filename, date, rating | General review, client proofs |
| `large` | 3 | 3 | ~110mm | filename, date, rating, tags | Portfolio review, detailed proofs |

Column/row overrides take precedence. Thumbnail size is computed from available space after margins and field text height.

### Available Metadata Fields

| Field name | Content | Example |
|---|---|---|
| `filename` | Original filename (truncated if needed) | `DSC_8561.ARW` |
| `name` | Asset name (falls back to filename) | `Sunset at lake` |
| `date` | Creation date (YYYY-MM-DD) | `2026-02-14` |
| `rating` | Star rating as Unicode stars | `★★★★☆` |
| `label` | Color label name | `Red` |
| `tags` | Comma-joined tags (truncated) | `landscape, sunset` |
| `format` | Primary variant format | `NEF` |
| `id` | Short asset ID (first 8 chars) | `c654efa4` |
| `dimensions` | Original pixel dimensions | `6000×4000` |
| `camera` | Camera model from EXIF | `Nikon Z9` |
| `lens` | Lens from EXIF | `50mm f/1.2` |

### Examples

```bash
# Standard proof sheet for a shoot
dam contact-sheet "date:2026-02-14 volume:Working" proof.pdf --title "Studio Session Feb 14"

# Dense archive index for an entire volume
dam contact-sheet "volume:Archive2025" archive-index.pdf --layout dense --landscape

# Large presentation sheet with grouping
dam contact-sheet "collection:Portfolio" portfolio.pdf --layout large --group-by label --fields name,rating,tags

# Client selection sheet — rated images only
dam contact-sheet "rating:3+ tag:selects" selects.pdf --fields filename,rating --title "Client Selects"

# Dry run to check page count
dam contact-sheet "date:2026-02" feb.pdf --dry-run
# → Contact sheet: 347 assets, 18 pages (standard, A4 portrait)

# JSON output for scripting
dam contact-sheet "tag:portfolio" portfolio.pdf --json
# → {"assets": 42, "pages": 3, "layout": "standard", "paper": "a4", "output": "portfolio.pdf"}
```

---

## Page Layout

### Page Structure

```
┌─────────────────────────────────────────────┐
│  [Title / Query]                   [Page N] │  ← header (8mm)
├─────────────────────────────────────────────┤
│                                             │
│  ┌──────┐  ┌──────┐  ┌──────┐  ┌──────┐   │
│  │      │  │      │  │      │  │      │   │
│  │ img  │  │ img  │  │ img  │  │ img  │   │
│  │      │  │      │  │      │  │      │   │
│  ├──────┤  ├──────┤  ├──────┤  ├──────┤   │
│  │meta  │  │meta  │  │meta  │  │meta  │   │
│  └──────┘  └──────┘  └──────┘  └──────┘   │
│                                             │
│  ┌──────┐  ┌──────┐  ┌──────┐  ┌──────┐   │
│  │      │  │      │  │      │  │      │   │
│  │ img  │  │ img  │  │ img  │  │ img  │   │
│  │      │  │      │  │      │  │      │   │
│  ├──────┤  ├──────┤  ├──────┤  ├──────┤   │
│  │meta  │  │meta  │  │meta  │  │meta  │   │
│  └──────┘  └──────┘  └──────┘  └──────┘   │
│                                             │
├─────────────────────────────────────────────┤
│  [dam • query text]            [date] [N/M] │  ← footer (6mm)
└─────────────────────────────────────────────┘
```

### Thumbnail Cells

Each cell is a fixed-size rectangle within the grid. The image is scaled to fit (preserving aspect ratio) and centered within the cell's image area. The metadata text area sits below.

- **Image area**: Square or near-square region, sized to fill the cell width
- **Metadata area**: 1–3 lines of text below the image, font size scaled to layout
- **Cell gap**: 4mm horizontal and vertical spacing between cells
- **Rating stars**: Rendered inline as filled/empty Unicode stars (or small star glyphs)
- **Color label**: Small filled circle next to the filename, matching the label color
- **Border**: Thin (0.25pt) light gray border around each thumbnail for visual separation

### Section Headers (with `--group-by`)

When `--group-by` is active, a full-width horizontal bar spans the grid whenever the grouping field changes:

```
──── 2026-02-14 ──────────────────────────────
```

Section headers consume one row's vertical space. If a header would be the last item on a page, it moves to the next page (widow prevention).

### Header and Footer

- **Header** (first page): Title text (left-aligned, bold), query string (right-aligned, small, gray)
- **Header** (subsequent pages): Title (left), page number (right)
- **Footer** (all pages): `dam` branding + generation date (left), `Page N of M` (right)

---

## Implementation

### Architecture

```
main.rs (CLI)
  → ContactSheetConfig (parsed from flags)
  → AssetService::contact_sheet(query, config, output_path)
      → QueryEngine::search(query)         // resolve assets
      → load preview images (image crate)  // from previews/ or smart_previews/
      → compose pages (image crate)        // lay out grid cells
      → encode PDF (printpdf crate)        // embed JPEG pages as full-page images
```

### Approach: Image-Based PDF

Rather than placing individual JPEG images and text objects into the PDF (which requires precise coordinate math in PDF units and font embedding), the simpler and more reliable approach is:

1. **Compose each page as an in-memory image** using the existing `image`/`imageproc`/`ab_glyph` stack (same approach as info card generation)
2. **Encode each page image as JPEG**
3. **Create a PDF with one full-page JPEG per page** via `printpdf`

This approach:
- Reuses the proven image composition code from `preview.rs`
- Avoids PDF text rendering complexity (font subsetting, encoding, kerning)
- Produces compact PDFs (JPEG compression)
- Gives pixel-perfect control over layout
- Handles star ratings, color dots, and Unicode text trivially

**Page resolution**: 300 DPI. A4 at 300 DPI = 2480×3508 pixels. This gives sharp text and thumbnails while keeping file size reasonable (~200–400 KB per page as JPEG quality 90).

### New Dependency

```toml
[dependencies]
printpdf = "0.7"   # ~50KB, pure Rust, no system dependencies
```

`printpdf` is only used to wrap the composed JPEG pages into a valid PDF container. The heavy lifting (layout, rendering, text) is done by the existing `image`/`imageproc`/`ab_glyph` crates.

### New Files

| File | Purpose |
|---|---|
| `src/contact_sheet.rs` | Layout engine, page composition, PDF generation |

### Key Types

```rust
/// Configuration for contact sheet generation.
pub struct ContactSheetConfig {
    pub layout: ContactSheetLayout,
    pub columns: u32,
    pub rows: u32,
    pub paper: PaperSize,
    pub landscape: bool,
    pub title: Option<String>,
    pub fields: Vec<MetadataField>,
    pub sort: Option<String>,
    pub use_smart_previews: bool,
    pub group_by: Option<GroupByField>,
    pub margin_mm: f32,
}

pub enum ContactSheetLayout { Dense, Standard, Large }
pub enum PaperSize { A4, Letter, A3 }
pub enum MetadataField { Filename, Name, Date, Rating, Label, Tags, Format, Id, Dimensions, Camera, Lens }
pub enum GroupByField { Date, Volume, Collection, Label }

/// Result of a contact sheet generation.
#[derive(Debug, serde::Serialize)]
pub struct ContactSheetResult {
    pub assets: usize,
    pub pages: usize,
    pub layout: String,
    pub paper: String,
    pub output: String,
    pub errors: Vec<String>,
}
```

### Rendering Pipeline

```
for each page:
    1. Create blank RgbImage at page_width_px × page_height_px (white background)
    2. Draw header (title, page number)
    3. For each cell in the grid:
        a. Load preview JPEG from disk (smart or regular)
        b. Resize to fit cell's image area (preserving aspect ratio)
        c. Center and draw onto page image
        d. Draw thin border rectangle
        e. Draw metadata text lines below image
        f. Draw color label dot (if present and field enabled)
        g. Draw rating stars (if present and field enabled)
    4. Draw footer
    5. Draw section header (if group-by and value changed)
    6. Encode page as JPEG (quality 90)
    7. Add to PDF document
```

### EXIF Metadata Access

For fields like `camera`, `lens`, `dimensions` — these come from `source_metadata` on the variant (already extracted during import). The `SearchRow` doesn't carry all of these, so the contact sheet function loads full `VariantDetails` for each asset once to populate the metadata fields.

Optimization: only load `VariantDetails` if the requested `--fields` include metadata beyond what `SearchRow` provides.

### Memory and Performance

- Preview images are loaded one at a time (not all held in memory)
- Each page image (~25 MB uncompressed at 300 DPI) is composed, encoded, and freed before the next
- Expected throughput: ~1–2 seconds per page (dominated by JPEG decode/encode)
- A 500-asset contact sheet (standard layout, 25 pages) should complete in under a minute

### Error Handling

- Missing previews: render a placeholder cell (gray box with filename text, like info cards)
- Missing metadata fields: omit the line (don't show "N/A")
- Zero search results: exit with error message, no PDF created
- Output path not writable: fail early before processing

---

## Configuration

Optional defaults in `dam.toml`:

```toml
[contact_sheet]
layout = "standard"          # default layout preset
paper = "a4"                 # default paper size
fields = "filename,date,rating"  # default metadata fields
margin = 10                  # default margin in mm
smart = false                # use smart previews by default
quality = 90                 # JPEG quality for page images
```

CLI flags override config values.

---

## Web UI Integration (Future)

Not in scope for the initial implementation, but a natural follow-up:

- "Contact sheet" button in the batch toolbar (generates PDF for selected assets)
- "Export as PDF" button in the results bar (generates for current search results)
- `POST /api/contact-sheet` endpoint returning the PDF as a download
- Layout/fields configuration via a modal dialog

---

## Testing

### Unit Tests

- Layout calculation: verify cell sizes for each preset × paper × orientation combination
- Field parsing: verify all metadata field names parse correctly
- Grouping: verify section headers insert at correct positions
- Page count: verify pagination for edge cases (0 assets, 1 asset, exactly fills page, one over)
- Truncation: verify long filenames/tags truncate with ellipsis within cell width

### Integration Tests

- `dam contact-sheet "type:image" output.pdf` — generates valid PDF
- `dam contact-sheet "type:image" output.pdf --dry-run` — reports count without writing
- `dam contact-sheet "type:image" output.pdf --json` — JSON result with page count
- `dam contact-sheet "nonexistent:true" output.pdf` — exits with error on zero results
- `dam contact-sheet "type:image" output.pdf --layout dense --landscape --paper a3` — all options compose

---

## Rollout

### Phase 1: Core Generation

- `dam contact-sheet` command with all options listed above
- Image-based PDF rendering via `printpdf`
- Three layout presets (dense, standard, large)
- All metadata fields
- `--dry-run`, `--json`, `--log`, `--time`

### Phase 2: Enhancements (if needed)

- `--group-by` section headers
- Custom column/row override
- `--template` for custom per-cell text format strings
- `[contact_sheet]` config section in `dam.toml`
- Web UI integration

---

## Open Questions

1. **Should `--smart` be the default?** Smart previews (2560px) produce sharper thumbnails at 300 DPI, but not all assets may have them. Fallback to regular preview if smart is missing?
2. **Color label rendering**: Small filled circle, or a colored border around the entire cell?
3. **Should `printpdf` be feature-gated?** It's a small pure-Rust crate (~50KB) with no system dependencies, so gating seems unnecessary. But if we want to keep the binary lean, it could go behind `--features pdf`.
