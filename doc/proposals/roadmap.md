# Roadmap: Planned & Proposed Features

Consolidated list of unimplemented features and new ideas, organized by theme. Items from `enhancements.md` and `proposal-future-enhancements.md` are merged here with updated priorities reflecting the current state of the project (v1.8.9).

The main focus is on an **optimized workflow for finding, evaluating, and managing the best images** from a large multi-year archive, and on getting a **clear overview of assets** across volumes.

---

## Tier 1 — High-Impact Workflow Features

### 1. Side-by-Side Compare

Select 2–4 assets and view them in a split layout with synchronized zoom and pan.

**Web UI:** `/compare?ids=a,b,c` page. Each asset fills an equal portion of the viewport. Scroll/zoom on one image applies to all (synchronized via JS). Toggle sync on/off. Show key metadata below each image (name, rating, label, focal length, aperture, ISO, shutter speed). Rating and label editable inline.

**Trigger:** "Compare" button in the batch toolbar when 2–4 assets are selected. Keyboard shortcut `c` in the browse grid when 2–4 cards are selected.

**Why:** Essential for choosing between similar shots (same scene, different exposure/composition). Currently requires opening multiple browser tabs or relying on memory.

**Prerequisites:** Smart previews (below) would make this usable for offline volumes.

> **Status (v1.7.0):** Fully implemented. Compare view at `/compare?ids=...` with flex columns, synchronized zoom/pan (toggle with `s` key), interactive rating/label per column, EXIF display, smart preview loading with HD badge. Keyboard: arrows for focus, `d` detail, `s` sync toggle, `,` `.` `+` `-` zoom, `0`–`5` rating, Alt+1–7 labels. "Compare" button in batch toolbar (requires 2–4 selected).

---

### 2. Map View for Geotagged Photos

EXIF GPS coordinates are already extracted into `source_metadata` during import. A map view would surface this data.

**Web UI:** `/map` page with an OpenStreetMap tile layer (Leaflet.js). Assets with GPS data shown as clustered markers. Click a cluster to see thumbnails. Click a thumbnail to open the asset detail page or lightbox. Filter controls (same as browse page) narrow the visible markers.

**Search filter:** `geo:<lat>,<lng>,<radius_km>` or `geo:<south>,<west>,<north>,<east>` bounding box.

**Schema:** Add indexed `latitude REAL` and `longitude REAL` columns on the `assets` table (denormalized from primary variant's `source_metadata` GPS fields). Backfill during migration.

**Why:** Spatial browsing is a natural complement to temporal browsing (calendar view). Essential for travel photography, event coverage, and "where was this taken?" queries. Combined with date and tag filters, enables powerful multi-dimensional browsing.

> **Status (v1.8.0):** Fully implemented. Map view as third browse mode toggle (grid/calendar/map) with Leaflet.js + MarkerCluster embedded as static assets. GPS parsed to decimal degrees during import, denormalized as `latitude`/`longitude` on assets table with composite index, backfilled on migration. `geo:` search filter with `any`/`none`/lat,lng,radius/bounding-box syntax. Thumbnail popups open the navigable lightbox; metadata links to detail page. Dark mode tile inversion. `m` keyboard shortcut. `GET /api/map` endpoint reuses `build_search_where()` for full filter consistency.

---

### 3. Smart Previews for Offline Browsing

Current previews are 800px thumbnails — enough for browse cards but too small for meaningful evaluation in the lightbox or compare view.

**Command:** `dam generate-smart-previews [--asset <id>] [--volume <label>] [--max-edge <N>]`

**Behavior:** Generate a second tier of previews at 2560px (configurable), stored as lossy JPEG/WebP in `smart-previews/<hash-prefix>/<hash>.jpg`. Used by the lightbox and compare view when the source volume is offline. Config: `[preview]` section gains `smart_max_edge` (default 2560) and `smart_quality` (default 85).

**Web UI:** Lightbox automatically uses smart preview when available, falling back to 800px thumbnail. Compare view requires smart previews for meaningful side-by-side evaluation.

**Why:** The core value proposition is browsing without media mounted. 800px thumbnails limit this to the grid view. Smart previews extend offline utility to full-screen review, rating, and comparison — the workflow that matters most for culling.

> **Status (v1.7.0):** Fully implemented. Smart previews at 2560px stored in `smart_previews/<hash-prefix>/<hash>.jpg`. Generated via `dam import --smart`, `[import] smart_previews = true` config, detail page button, or on-demand (`[preview] generate_on_demand = true`). Web UI shows regular preview instantly, background-loads smart preview with pulsing HD badge. Enables zoom and pan in lightbox, detail page, and compare view (mouse wheel, drag, click toggle, keyboard `,` `.` `+` `-`). Config: `smart_max_edge` (default 2560), `smart_quality` (default 85).

---

### 4. AI-Assisted Tagging & Visual Similarity

Integrate with a local vision model to suggest tags from image content and enable visual similarity search.

**Command:** `dam auto-tag [--asset <id>] [--model clip] [--threshold 0.5] [--apply]`

**Approach:** Use CLIP embeddings (via `ort` crate for ONNX runtime, or shell out to a Python process) to classify images against a configurable label set. Suggest tags with confidence scores. Store embeddings per-variant for similarity queries.

**Visual similarity:** `dam search --similar <asset-id>` — find the N most visually similar assets by cosine distance. Web UI: "Find similar" button on asset detail page.

**Web UI:** "Suggest tags" button on asset detail page shows suggested tags with confidence badges. Click to accept individually or "Accept all."

**Why:** Manual tagging is the biggest friction point. Even 70–80% accuracy dramatically speeds up the workflow. Visual similarity helps find related shots across sessions. Local models (CLIP, SigLIP) avoid privacy concerns.

---

## Tier 2 — Workflow Convenience

### 5. Import Profiles

Named presets for common import scenarios, configured in `dam.toml`.

```toml
[import.profiles.tethered]
include = ["raw"]
auto_tags = ["inbox", "tethered"]
exclude = ["*.tmp", "*.lock"]

[import.profiles.export]
include = ["image"]
skip = ["raw"]
auto_tags = ["export", "review"]
```

**Usage:** `dam import --profile tethered /Volumes/Photos/Capture/`

**Why:** Power users repeat the same import flag combinations. Profiles reduce typing and prevent mistakes. Natural extension of the existing `[import]` config.

---

### 6. Watch Mode

```
dam watch [PATHS...] [--volume <label>] [--profile <name>]
```

File system watcher (via `notify` crate) that auto-imports/syncs when files change. Useful for monitoring a CaptureOne session's output folder during active editing.

**Use cases:**
- Tethered shooting: new RAW files auto-imported
- Export monitoring: processed TIFFs/JPEGs picked up and grouped with RAW originals
- Recipe changes: XMP/COS modifications detected and metadata refreshed in real time

**Design considerations:**
- Debounce events (files written in stages)
- Volume mount/unmount handling
- Optional auto preview generation
- Foreground process vs. background daemon
- Combine with import profiles for scenario-specific behavior

---

### 7. Export Command

```
dam export <query> <target> [--layout flat|mirror] [--symlink] [--all-variants] [--include-sidecars] [--dry-run] [--overwrite]
```

Export matching assets to a directory, optionally with sidecars.

**Use cases:**
- `dam export "rating:5 tag:portfolio" /tmp/delivery/` — gather best-of for client delivery
- `dam export "collection:Print" /Volumes/USB/ --include-sidecars` — with XMP/COS for another workstation
- `dam export "tag:instagram" ~/Export/` — flat directory for social media

**Options:** Copy vs. symlink, mirror source paths vs. flat, filename conflict resolution (hash suffix), best variant only vs. all variants, sidecar inclusion, dry-run, overwrite.

> **Status (v1.8.9):** Fully implemented. Flat layout with hash-suffix collision resolution, mirror layout preserving directory structure (multi-volume gets volume-label prefix). Symlink mode. Best-variant-only (default) or all-variants. Sidecar inclusion. Dry-run. Overwrite. SHA-256 integrity verification on copy. Supports `--json`, `--log`, `--time`.

---

### 8. Undo / Edit History

Track metadata changes with timestamps for auditability and mistake recovery.

**Data model:** Add a `history` list to sidecar YAML:

```yaml
history:
  - timestamp: 2026-02-23T14:30:00Z
    field: rating
    old: 3
    new: 5
  - timestamp: 2026-02-23T14:31:00Z
    field: tags
    action: add
    values: [portfolio]
```

**CLI:** `dam history <asset-id>` shows the changelog. `dam undo <asset-id>` reverts the last change.

**Web UI:** History panel on asset detail page (collapsible). Undo button per entry.

**Why:** Batch operations on large selections are powerful but error-prone. "I accidentally set 200 assets to 1 star" has no recovery path today.

---

## Tier 3 — Polish & Advanced

### 9. IPTC / Structured Metadata

Support IPTC Core fields for professional photographers and stock agencies: headline, caption, copyright notice, creator, credit line, source, usage terms, city, country, sublocation.

**Data model:** `iptc_metadata: HashMap<String, String>` on Asset, stored in sidecar YAML, indexed in SQLite for searchable fields.

**Search:** `iptc:city=Berlin`, `iptc:country=Germany`.

**Why:** Stock photographers need IPTC for agency submission. Currently this data is buried in `source_metadata`.

---

### 10. Drag-and-Drop Enhancements

**Stack reordering:** Drag to reorder stack members on the asset detail page. DOM reordering triggers backend position updates.

**Collection management:** Drag asset cards from the browse grid onto collection chips in a sidebar. Add `position INTEGER` to `collection_assets` for manual ordering. Drag to reorder within a collection view.

**Why:** Direct manipulation feels more natural than dropdown+button workflows for curation tasks.

---

### 11. Statistics & Insights Dashboard

Expand the stats page with visual analytics for large archives:

- **Shooting frequency chart** — assets per month/year as a bar chart (complements the calendar heatmap)
- **Gear usage breakdown** — shots per camera body, per lens, showing which gear gets used most
- **Rating distribution** — histogram of ratings across the catalog, per collection, or per time period
- **Label workflow funnel** — how many assets at each color label stage (useful when labels represent workflow states like "review → select → edit → deliver")
- **Storage growth over time** — cumulative bytes per volume, showing growth trajectory
- **Tag cloud** — weighted visualization of tag frequency (already have the data from tags page)

**Why:** Large archives accumulate metadata that tells a story about shooting habits, gear usage, and workflow efficiency. Currently this requires manual SQL queries or export+spreadsheet.

---

### 12. Faceted Browse Sidebar

Replace or supplement the top filter row with a persistent left sidebar showing faceted navigation:

- **Tags:** Collapsible tree (reuse tags page component), click to toggle filter
- **Dates:** Year > Month > Day drill-down with counts
- **Volumes:** List with online/offline indicators and counts
- **Ratings:** Star filter with count per rating level
- **Labels:** Color dots with counts
- **Collections:** List with counts

Each facet updates counts in real time based on the current filter combination (like e-commerce faceted search).

**Why:** The current filter row works but doesn't show the distribution of values or give a sense of "what's available." A sidebar with counts enables discovery ("I have 340 unrated assets from 2024") and helps narrow searches iteratively.

> **Status (v1.8.1):** Implemented as a read-only statistical sidebar. Toggleable via results bar button or `f` key. Shows distribution by rating (bar chart), color label (color dots), format, volume, tags (top 30), year (bar chart), and geotagged count. All sections collapsible with state persisted. Filtering remains in the top filter bar (no click-to-filter). Backed by `GET /api/facets` endpoint with 8 aggregate queries reusing `build_search_where()`.

---

## Priority Summary

| Priority | Feature | Effort | Impact | Theme | Status |
|----------|---------|--------|--------|-------|--------|
| 1 | Side-by-side compare | Medium | Very high | Evaluation | **Done** (v1.7.0) |
| 2 | Map view | Medium | High | Overview / browsing | **Done** (v1.8.0) |
| 3 | Smart previews | Medium | High | Offline evaluation | **Done** (v1.7.0) |
| 4 | AI tagging & similarity | High | Very high | Discovery / organization | |
| 5 | Import profiles | Low | Medium | Workflow convenience | |
| 6 | Watch mode | Medium | Medium | Workflow automation | |
| 7 | Export command | Medium | Medium | Delivery | **Done** (v1.8.9) |
| 8 | Undo / edit history | Medium | Medium | Safety | |
| 9 | IPTC metadata | Medium | Low–Medium | Professional workflow | |
| 10 | Drag-and-drop | Low | Low | UX polish | |
| 11 | Statistics dashboard | Medium | Medium | Overview / insights | |
| 12 | Faceted browse sidebar | Medium | High | Overview / discovery | **Done** (v1.8.1) |

Items 1, 2, 3, 7, and 12 are complete as of v1.8.9, delivering the core "find and evaluate the best images" workflow with compare view, smart previews, spatial browsing via the map view, faceted overview sidebar, and file export for delivery. The highest-priority remaining item is 4 (AI tagging). Item 11 provides additional "overview of assets" dimension.
