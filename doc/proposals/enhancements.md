# Proposal: Enhancement Ideas

Ideas for new functionality, UI/UX improvements, and workflow additions, organized by impact and implementation scope.

Watch mode is specified separately in [`proposal-future-enhancements.md`](proposal-future-enhancements.md). Export is now implemented (v1.8.9).

---

## High-Value Workflow Features

### 1. Smart Duplicate Resolution

`dam duplicates` finds files with identical content at multiple locations, but there is no workflow to *resolve* them. A dedicated command would close this gap.

```
dam deduplicate [--keep-volume <label>] [--prefer-online] [--min-copies N] [--apply]
```

**Behavior:**
- For each duplicate set, pick the "best" location(s) to keep using a scoring heuristic: prefer online volumes, most recently verified, user-specified preferred volume.
- `--min-copies N` ensures at least N locations survive (default 1). Safety guard against accidental single-point-of-failure.
- `--keep-volume` always keeps locations on the named volume.
- Report-only by default (`--apply` to execute), consistent with `sync`, `cleanup`, `auto-group`.
- Removing a location means deleting the file from disk and the location record from catalog/sidecar.

**Web UI:** A `/duplicates` page showing duplicate groups with side-by-side location comparison. "Keep this" / "Remove" buttons per location. Batch "auto-resolve" using the same heuristic as the CLI.

**Why:** Large multi-volume archives accumulate duplicates from backup runs, re-imports, and relocations. Currently the user must manually identify and remove them.

> **Status (v1.4.1 / v1.6.2 / v1.6.3):** Fully implemented. CLI `dam dedup` with `--prefer`, `--min-copies`, `--apply`, `--volume`, path/format filters. Web UI `/duplicates` page with summary cards, mode tabs, filters, per-location removal, auto-resolve with prefer input, recipe cleanup. Config: `[dedup] prefer` default.

---

### 2. Hierarchical Tags

Photographers commonly organize keywords in trees: `People/Family/Thomas`, `Location/Europe/Germany/Berlin`, `Genre/Landscape/Mountains`. CaptureOne and Lightroom both support hierarchical keywords via XMP `lr:hierarchicalSubject`.

**Data model:** Store tags as flat strings with `/` separator (e.g. `Location/Europe/Germany`). No schema changes needed — the hierarchy is purely in the naming convention. Parent tags are implicit: tagging an asset `Location/Europe/Germany` means it matches `tag:Location`, `tag:Location/Europe`, and `tag:Location/Europe/Germany`.

**Search:** `tag:Location/` (trailing slash) matches all sub-tags. `tag:Location/Europe` matches `Location/Europe` and all children. Exact match without trailing slash works as today.

**Web UI tags page:** Tree view with expand/collapse. Indent children under parents. Click a node to filter the browse grid. Drag tags to reparent.

**Web UI tag input:** Autocomplete shows hierarchical paths. Typing `Loc` suggests `Location/Europe/Germany`, `Location/Asia/Japan`, etc.

**XMP round-trip:** Read `lr:hierarchicalSubject` on import. Write back using the same element. Flat `dc:subject` keywords continue to work as before (non-hierarchical).

**Why:** Flat tag lists become unmanageable beyond ~50 tags. Hierarchy enables both broad and narrow filtering without explosion of tag names.

> **Status (v1.6.0):** Fully implemented. Tags with `/` hierarchy separator, parent tag matching in search, collapsible tree view on tags page with own-count and total-count. `lr:hierarchicalSubject` XMP round-trip. Internally stored with `|` separator.

---

### 3. Map View for Geotagged Photos

EXIF GPS coordinates are already extracted into `source_metadata` during import. A map view would make this data browsable.

**Web UI:** `/map` page with an OpenStreetMap tile layer (via Leaflet.js). Assets with GPS data shown as markers, clustered at zoom levels. Click a cluster to see thumbnails. Click a thumbnail to open the asset detail page. Filter controls (same as browse page) narrow the visible markers.

**Search filter:** `geo:<lat>,<lng>,<radius_km>` — bounding circle query. Alternatively `geo:<south>,<west>,<north>,<east>` for bounding box.

**Schema:** Add indexed columns `latitude REAL` and `longitude REAL` on the `variants` table (or a dedicated `locations_geo` table). Populated from `source_metadata` GPS fields during import and migration backfill.

**Why:** Spatial browsing is a natural complement to temporal browsing. Useful for travel photography, location scouting, and finding "all photos taken at this venue."

> **Status (v1.8.0):** Fully implemented. Map view as third browse mode (grid/calendar/map toggle). Leaflet.js + MarkerCluster embedded as static assets. Denormalized `latitude`/`longitude` on assets table. `geo:` filter with `any`/`none`/lat,lng,radius/bounding-box syntax. Thumbnail popups with lightbox integration. Dark mode support. `m` keyboard shortcut.

---

### 4. Timeline / Calendar View

A date-oriented browsing mode for navigating large archives by time.

**Web UI:** `/timeline` page with a year → month → day drill-down. Each level shows a summary row (thumbnail strip, asset count, size) that expands on click. Day view shows the browse grid filtered to that date. Supports the same filters as the browse page.

**Implementation:** Pure UI feature — the data already exists via `created_at` on assets. A few SQL queries grouped by `date(created_at)` with counts. No schema changes.

**Why:** "When did I shoot this?" is the most common retrieval question after "what is this?" Date-based navigation is missing from the current text-search-only UI.

> **Status (v1.5.3):** Fully implemented as calendar heatmap view on the browse page (Grid/Calendar toggle). Year-at-a-glance with day cells colored by asset count. Year navigation with arrow buttons and year chips. Click day to filter grid. All search filters apply to calendar aggregation. Date filters: `date:`, `dateFrom:`, `dateUntil:`. API: `GET /api/calendar`.

---

### 5. Backup Coverage Report

A dedicated view showing which assets are at risk of data loss.

```
dam backup-status [--min-copies N] [--volume <label>]
```

**Output:**
- Assets with 0 online locations (equivalent to `volume:none`)
- Assets with fewer than N total locations (default 2)
- Assets existing on only one volume
- Per-volume coverage: "Volume X has 4,231 assets, 892 are exclusive to this volume"

**Web UI:** Section on the stats page, or a dedicated `/backup` page with a volume-by-volume matrix.

**Why:** Combines the information from `volume:none`, `orphan:true`, and `duplicates` into a single actionable report. Currently the user must run multiple commands and mentally correlate the results.

> **Status (v1.4.1):** Fully implemented. CLI `dam backup-status` with `--min-copies`, `--volume`, `--at-risk`, format/query options. Web UI `/backup` page with summary cards, volume distribution bar chart, coverage by purpose table, volume gaps, clickable at-risk link.

---

## UI/UX Improvements

### 6. Lightbox / Fullscreen View

The single biggest UX gap for photo browsing. Currently, viewing an asset requires navigating to the detail page and using the browser back button to return.

**Behavior:** Click a browse card thumbnail (or press Enter on focused card) to open a fullscreen overlay. Arrow keys navigate between results in the current search order. Close with Escape. An info panel (toggled with `i`) shows metadata, tags, rating, label. Rating and label editable directly in lightbox via the same keyboard shortcuts (1-5, r/o/y/g/b/p/u).

**Implementation:** Client-side overlay using the existing preview images. Preload adjacent previews for smooth navigation. The results list from the current search provides the navigation order. No new API endpoints needed — asset data can be embedded in the browse cards as `data-` attributes or fetched on demand.

**Why:** Photo culling requires rapid comparison of many images. Page-per-image navigation breaks the flow. Every professional photo tool has a full-screen review mode.

> **Status (v1.6.1):** Fully implemented. Lightbox with keyboard navigation, rating/label editing (keyboard and mouse), info panel, and always-visible rating stars and color label dots in the top bar. Detail page has prev/next navigation with unlimited multi-hop via sessionStorage. `d` key opens detail from lightbox, `l` returns to lightbox from detail page. Global keyboard help panel (`?` key) shows all shortcuts per page. Alt/Option+number uses `e.code` for macOS compatibility.

---

### 7. Adjustable Grid Density

The browse grid currently uses a fixed ~6 columns. Different tasks need different densities.

**Options:** Compact (8-10 columns, tiny thumbnails), Normal (5-6, current), Large (3-4, big previews). Toggle buttons in the results bar or a slider.

**Implementation:** CSS grid with `grid-template-columns` controlled by a CSS variable. Preference stored in `localStorage`. Cards adapt aspect ratio and hide/show metadata (compact hides stars/labels/format, large shows more text).

**Why:** Culling wants large previews. Organizing wants to see many assets at once. One size doesn't fit all.

> **Status (v1.5.0):** Fully implemented. Three density presets (Compact/Normal/Large) via `[data-density]` attribute. CSS variable `--grid-min` (120px/200px/300px). Toggle buttons with SVG grid icons in results bar. `localStorage` persistence. Keyboard nav column count adjusts automatically.

---

### 8. Side-by-Side Compare

Select 2-4 assets and compare them in a split view with synchronized zoom and pan.

**Web UI:** `/compare?ids=a,b,c` page. Each asset fills an equal portion of the viewport. Scroll/zoom on one image applies to all (synchronized via JS). Toggle sync on/off. Show key metadata below each image (name, rating, label, focal length, aperture). Rating and label editable.

**Trigger:** "Compare" button in the batch toolbar when 2-4 assets are selected.

**Why:** Essential for choosing between similar shots (same scene, different exposure/composition). Currently requires opening multiple browser tabs.

> **Status (v1.7.0):** Fully implemented. Compare view at `/compare?ids=...` with flex columns, synchronized zoom/pan (`s` key toggle), interactive rating/label per column, EXIF display, smart preview loading with HD badge. Keyboard: arrows for focus, `d` detail, `s` sync toggle, `,` `.` `+` `-` zoom, `0`–`5` rating, Alt+1–7 labels.

---

### 9. Dark Mode

Many photographers prefer dark interfaces to reduce eye strain and minimize color perception bias when evaluating images.

**Implementation:** CSS custom properties for all colors. Toggle button in the nav bar. Preference stored in `localStorage`. Respect `prefers-color-scheme` media query as default. Dark theme: dark gray backgrounds (#1e1e2e), light text (#cdd6f4), muted borders, adjusted card shadows.

**Why:** Low-effort, high-appreciation feature. The current light theme works but feels out of place for a photo management tool.

> **Status (v1.5.0):** Fully implemented. Sun/moon toggle in nav bar, OS preference via `prefers-color-scheme`, `localStorage('dam-theme')` persistence, FOUC-preventing inline script, CSS custom property overrides via `[data-theme="dark"]`.

---

### 10. Drag-and-Drop Collection Management

**Browse → Collection:** Drag asset cards from the browse grid onto collection chips in a sidebar or the saved search chip row. Drop target highlights on hover.

**Collection ordering:** Add a `position INTEGER` column to `collection_assets`. Drag to reorder within a collection. `dam collection show` respects order. Web UI collection view supports drag reorder.

**Why:** The current workflow (select assets → pick collection from dropdown → click button) works but feels clunky for curating. Direct manipulation is more natural.

---

## Deeper Functionality

### 11. Smart Previews for Offline Browsing

Current previews are 800px thumbnails — enough for browse cards but too small for meaningful evaluation on a retina display.

```
dam generate-smart-previews [--asset <id>] [--volume <label>] [--max-edge <N>]
```

**Behavior:** Generate a second tier of previews at 2560px (configurable), stored as lossy JPEG/WebP in `smart-previews/<hash-prefix>/<hash>.jpg`. Used by the lightbox view and compare view when the source volume is offline. `[preview]` config gets a `smart_max_edge` and `smart_quality` setting.

**Why:** The core value proposition of dam is browsing without media mounted. 800px thumbnails limit this to the grid view. Smart previews extend offline utility to full-screen review and editing decisions.

> **Status (v1.7.0):** Fully implemented. Smart previews at 2560px stored in `smart_previews/<hash-prefix>/<hash>.jpg`. Generated via `dam import --smart`, `[import] smart_previews = true` config, detail page button, or on-demand (`[preview] generate_on_demand = true`). Enables zoom and pan in lightbox, detail page, and compare view. Config: `smart_max_edge` (default 2560), `smart_quality` (default 85).

---

### 12. Import Profiles

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

```
dam import --profile tethered /Volumes/Photos/Capture/
```

**Why:** Power users repeat the same import flag combinations. Profiles reduce typing and prevent mistakes. Natural extension of the existing `[import]` config section.

---

### 13. Undo / Edit History

Track metadata changes with timestamps for auditability and mistake recovery.

**Data model:** Add a `history` list to the sidecar YAML:

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

**CLI:** `dam history <asset-id>` shows the changelog. `dam undo <asset-id>` reverts the last change (with confirmation).

**Web UI:** History panel on the asset detail page (collapsible). Undo button for each entry.

**Why:** Batch operations on large selections are powerful but error-prone. "I accidentally set 200 assets to 1 star" has no recovery path today.

---

### 14. Backup Coverage Report

See [item 5](#5-backup-coverage-report) above.

---

### 15. AI-Assisted Tagging

Integrate with a local vision model or API to suggest tags from image content.

```
dam auto-tag [--asset <id>] [--model clip] [--threshold 0.5] [--apply]
```

**Approach:** Use CLIP embeddings (via `ort` crate for ONNX runtime, or shell out to a Python script) to classify images against a configurable label set. Suggest tags with confidence scores. Report-only by default.

**Web UI:** "Suggest tags" button on the asset detail page. Shows suggested tags with confidence badges. Click to accept.

**Visual similarity search:** With CLIP embeddings stored per-asset, enable "find similar" — given an asset, find the N most visually similar assets by cosine distance. `dam search --similar <asset-id>`.

**Why:** Manual tagging is the biggest friction point in any DAM. Even imperfect suggestions (70-80% accuracy) dramatically speed up the workflow. Local models avoid privacy concerns.

---

### 16. IPTC / Structured Metadata

Beyond XMP keywords, support IPTC Core fields for professional photographers and stock agencies.

**Fields:** headline, caption/abstract, copyright notice, creator, credit line, source, usage terms, city, country, sublocation.

**Data model:** Add `iptc_metadata: HashMap<String, String>` to Asset or Variant, stored in sidecar YAML and indexed in SQLite for searchable fields (city, country).

**Search:** `iptc:city=Berlin`, `iptc:country=Germany`, or a generic `iptc:key=value` filter.

**Why:** Stock photographers and photojournalists need IPTC metadata for agency submission. Currently this data is lost or buried in `source_metadata`.

---

## Priority Recommendations

Ranked by impact-to-effort ratio, building on existing infrastructure:

| Priority | Enhancement | Effort | Impact | Status |
|----------|-------------|--------|--------|--------|
| 1 | Lightbox / fullscreen view | Medium | Very high | **Done** (v1.5.0, enhanced v1.6.1, v1.7.1) |
| 2 | Smart duplicate resolution | Medium | High | **Done** (CLI v1.4.1, web UI v1.6.2, recipe cleanup v1.6.3) |
| 3 | Hierarchical tags | Medium | High | **Done** (v1.6.0) |
| 4 | Adjustable grid density | Low | High | **Done** (v1.5.0) |
| 5 | Timeline / calendar view | Medium | High | **Done** (v1.5.3) |
| 6 | Dark mode | Low | Medium | **Done** (v1.5.0) |
| 7 | Side-by-side compare | Medium | High | **Done** (v1.7.0) |
| 8 | Import profiles | Low | Medium | |
| 9 | Backup coverage report | Low | Medium | **Done** (v1.4.1) |
| 10 | Smart previews | Medium | Medium | **Done** (v1.7.0) |
| 11 | Stacks (scene grouping) | Medium | High | **Done** (v1.6.0) |
| 12 | Stack from tag conversion | Low | Medium | **Done** (v1.6.0) — `dam stack from-tag` converts matching tags to stacks |
| 13 | Drag-and-drop stack reordering | Low | Low | Planned — reorder stack members on asset detail page |
| 14 | Keyboard help & detail page nav | Low | Medium | **Done** (v1.6.1) — `?` help overlay, detail prev/next, d/l lightbox switching |

Items 1–7, 9–12, and 14 are complete (13 of 14 items). The remaining items are 8 (import profiles) and 13 (drag-and-drop).

Watch mode is specified in [`proposal-future-enhancements.md`](proposal-future-enhancements.md) and in the [roadmap](roadmap.md). Export was implemented in v1.8.9.
