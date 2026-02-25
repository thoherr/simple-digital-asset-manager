# Changelog

All notable changes to the Digital Asset Manager are documented here.

## v1.5.0

### New Features
- **Dark mode** — the web UI now supports dark mode. Automatically follows the OS/browser preference (`prefers-color-scheme: dark`). A toggle button (sun/moon) in the navigation bar lets you switch manually between light and dark themes. The preference is persisted in `localStorage` and applied instantly on page load (no flash of unstyled content). Covers all pages: browse, asset detail, tags, collections, stats, and backup status.
- **Grid density controls** — three density presets for the browse grid: **Compact** (smaller thumbnails, hidden metadata), **Normal** (default), and **Large** (bigger thumbnails, two-line titles). Toggle buttons with grid icons appear in the results bar next to sort controls. Persisted in `localStorage`. The keyboard navigation column count adjusts automatically.
- **Lightbox viewer** — clicking a thumbnail in the browse grid now opens a full-screen lightbox overlay instead of navigating to the asset detail page. Navigate between assets with on-screen arrow buttons or Left/Right arrow keys. Toggle a side info panel (i key or toolbar button) showing type, format, date, variant count, interactive rating stars, and color label dots. Changes made in the lightbox (rating, label) are written to the API and reflected in the grid behind. Press Escape to close, or click the "Detail" link to open the full asset detail page. Keyboard shortcuts for rating (0-5) and label (r/o/y/g/b/p/u/x, Alt+0-7) work inside the lightbox.

## v1.4.1

### New Commands
- **`dam dedup`** — remove same-volume duplicate file locations. Identifies variants with 2+ copies on the same volume, keeps the "best" copy (by `--prefer` path prefix, verification recency, path length), and removes the rest. `--min-copies N` ensures at least N total copies survive across all volumes. Report-only by default; `--apply` to delete files and remove location records. Supports `--volume`, `--json`, `--log`, `--time`.
- **`dam backup-status`** — check backup coverage and find under-backed-up assets. Shows aggregate overview (totals, coverage by volume purpose, location distribution, volume gaps, at-risk count). `--at-risk` lists under-backed-up assets using the same output formats as `dam search`. `--min-copies N` sets the threshold (default: 2). `--volume <label>` shows which assets are missing from a specific volume. Optional positional query scopes the analysis to matching assets. Supports `--format`, `-q`, `--json`, `--time`.

## v1.4.0

### New Features
- **Volume purpose** — volumes can now be assigned a logical purpose (`working`, `archive`, `backup`, `cloud`) describing their role in the storage hierarchy. `dam volume add --purpose <purpose>` sets purpose at registration, `dam volume set-purpose <volume> <purpose>` changes it later. Purpose is shown in `dam volume list` and included in `--json` output. This metadata lays the groundwork for smart duplicate analysis and backup coverage reporting (see storage workflow proposal).
- **Enhanced `dam duplicates`** — three new flags for targeted duplicate analysis:
  - `--same-volume` — find variants with 2+ locations on the same volume (likely unwanted copies)
  - `--cross-volume` — find variants on 2+ different volumes (intentional backups)
  - `--volume <label>` — post-filter results to entries involving a specific volume
  - Output now shows volume purpose (e.g. `[backup]`), volume count, same-volume warnings, and verification timestamps (in `--format full`)
  - `DuplicateEntry` JSON output includes `volume_count`, `same_volume_groups`, and enriched `LocationDetails` with `volume_id`, `volume_purpose`, `verified_at`
- **`copies:` search filter** — find assets by total file location count. `copies:1` finds single-copy assets (no backup), `copies:2+` finds assets with at least two copies. Same syntax pattern as `rating:`. Works in CLI, saved searches, and web UI.

## v1.3.2

### New Features
- **PDF manual generation** — `doc/manual/build-pdf.sh` script produces a complete PDF manual from the 21 Markdown source files. Renders mermaid diagrams to PNG, generates table of contents, headers/footers with version and date, and per-command page breaks in the reference section. Requires pandoc, XeLaTeX, and mermaid-cli.

### New Commands
- **`dam fix-recipes`** — re-attach recipe files (`.xmp`, `.cos`, etc.) that were misclassified as standalone assets during import. Scans the catalog for assets whose only variant is a recipe-type file, finds the correct parent variant by matching filename stem and directory, and re-attaches them. Dry-run by default (`--apply` to execute).

### Enhancements
- **15 additional RAW format extensions** — added support for `.3fr`, `.cap`, `.dcr`, `.eip`, `.fff`, `.iiq`, `.k25`, `.kdc`, `.mdc`, `.mef`, `.mos`, `.mrw`, `.obm`, `.ptx`, `.rwz` camera formats
- **`import --auto-group`** — after normal import, runs auto-grouping scoped to the neighborhood of imported files (one directory level up from each imported file). Avoids catalog-wide false positives from restarting camera counters. Combines with `--dry-run` and `--json`.

## v1.3.1

### New Features
- **`dam fix-dates` command** — scan assets and correct `created_at` dates from variant EXIF metadata and file modification times. Fixes assets imported with wrong dates (import timestamp instead of capture date). Re-extracts EXIF from files on disk for assets imported before `date_taken` was stored in metadata. Backfills `date_taken` into variant source_metadata on apply so future runs work without the volume online. Reports offline volumes clearly with skip counts and mount instructions. Dry-run by default (`--apply` to execute). Supports `--volume`, `--asset`, `--json`, `--log`, `--time`.

### Enhancements
- **Import date fallback chain** — import now uses EXIF DateTimeOriginal → file modification time → current time (previously fell through to current time when EXIF was missing, causing many assets to get the import timestamp as their date)
- **Second variant date update** — when a second variant joins a stem group during import, if it has an older EXIF date or mtime than the asset's current `created_at`, the asset date is updated
- **EXIF `date_taken` stored in source_metadata** — DateTimeOriginal is now persisted in variant source_metadata as `date_taken` (RFC 3339), enabling `fix-dates` and future date-aware features to work from metadata alone

## v1.3.0

### New Features
- **Comprehensive user manual** — 21 markdown files in `doc/manual/` covering every command, filter, and configuration option, organized into User Guide (7 workflow chapters), Reference Guide (10 man-page style command docs), and Developer Guide (3 pages: REST API, module reference, build/test)
- **9 Mermaid diagrams** — ER diagrams, architecture layers, round-trip workflow, XMP sync sequence, import pipeline, auto-group algorithm, maintenance cycle, data model, and module dependency graph
- **7 web UI screenshots** — browse page, saved search chips, asset detail, batch toolbar, tags page, collections page, and catalog structure
- **README Documentation section** — links to all three guide sections

## v1.2.0

### Enhancements
- **Browse grid deduplication** — assets with multiple variants (e.g. RAW+JPEG) now appear as a single card in the browse grid instead of one card per variant. Implemented via a denormalized `best_variant_hash` column on the `assets` table, computed at write time using the same Export > Processed > Original scoring as preview selection. Search queries with no variant-level filters skip the `variants` JOIN entirely for faster queries.
- **Primary format display** — browse cards now show the asset's identity format (e.g. NEF, RAF) instead of the preview variant's format (JPG). A denormalized `primary_variant_format` column prefers Original+RAW, then Original+any, then the best variant's format.
- **Variant count badge** — browse cards show a variant count badge (e.g. "3v") when an asset has more than one variant, making multi-variant assets visible at a glance.
- **`dam serve --log`** — the global `--log` flag now enables request logging on the web server, printing `METHOD /path -> STATUS (duration)` to stderr for each HTTP request.

## v1.1.1

### Enhancements
- **`path:` filter normalization** — the `path:` search filter now accepts filesystem paths in the CLI: `~` expands to `$HOME`, `./` and `../` resolve relative to the current working directory, and absolute paths matching a registered volume's mount point are automatically stripped to volume-relative with the volume filter implicitly applied. Plain relative paths (no `./` prefix) remain volume-relative prefix matches as before.

## v1.1.0

### New Features
- **Export-based preview selection** — previews now prefer Export > Processed > Original variants for display. RAW+JPEG assets show the processed JPEG preview instead of the flat dcraw rendering. Affects `dam show`, web UI asset detail page, and `generate-previews` catalog mode.
- **`generate-previews --upgrade`** — regenerate previews for assets where a better variant (export/processed) exists than the one currently previewed. Useful after importing exports alongside existing RAW files.

## v1.0.0

First stable release. All planned features are implemented, all tests pass, documentation is complete. Ready for production use.

### Highlights

- **22 CLI commands** covering the full asset management lifecycle: import, search, browse, edit, group, relocate, verify, sync, refresh, cleanup, and more
- **Web UI** with search, filtering, inline editing, batch operations, keyboard navigation, saved searches, and collections
- **Bidirectional XMP sync** with CaptureOne, Lightroom, and other photo editing tools
- **Content-addressable storage** with SHA-256 deduplication and integrity verification across multiple offline volumes
- **Stem-based auto-grouping** for RAW+JPEG+sidecar bundles, with fuzzy cross-directory grouping for exports

### Changes since v0.7.1

- Add 10 integration tests (group, fix-roles, refresh, edit --label)
- Complete documentation: architecture overview, component specification, specification
- Move specification into doc/ directory

## v0.7.1

### New Features
- **`dam fix-roles` command** — scan multi-variant assets and re-role non-RAW variants from Original to Export when a RAW variant exists. Fixes assets imported before the auto-grouping role fix. Dry-run by default (`--apply` to execute). Supports `--volume`, `--asset`, `--json`, `--log`, `--time`.
- **Import auto-grouping role fix** — newly imported RAW+non-RAW pairs now correctly assign Export role to non-RAW variants (previously both were marked Original)

## v0.7.0

### New Features
- **`dam auto-group` command** — automatically group assets by filename stem across directories, solving the problem where CaptureOne exports land in different directories than their RAW originals. Uses fuzzy prefix + separator matching (e.g., `Z91_8561.ARW` matches `Z91_8561-1-HighRes-(c)_2025_Thomas Herrmann.tif`). Chain resolution ensures multiple export levels all group to the shortest root stem. RAW files are preferred as the group target; donors are re-roled from Original to Export. Dry-run by default (`--apply` to execute). Supports `--json`, `--log`, `--time`.
- **"Group by name" batch button** in web UI — select assets on the browse page and click "Group by name" to auto-group them by filename stem with a confirmation dialog

### Bug Fixes
- **`group` now preserves recipes** — merging donor assets into a target now copies recipe records, preventing recipe loss on `rebuild-catalog`
- **`group` re-roles donor variants** — donor variants with role "original" are changed to "export" in both sidecar YAML and SQLite catalog, correctly reflecting their derived status

## v0.6.4

### Improvements
- **Auto-search on all filter changes** — removed the explicit Search button; text inputs (query, path) auto-search with 300ms debounce, dropdowns (type, format, volume, collection) trigger immediately on change, matching the existing behavior of stars, labels, and tags

## v0.6.3

### New Features
- **`path:` search filter** — filter assets by file location path prefix (e.g., `path:Capture/2026-02-22`), with quoted value support for paths with spaces; works in CLI, web UI (dedicated input in filter row), and saved searches
- **Grouped `--help` output** — CLI help now groups commands logically (Core, Organization, Maintenance, Output) for easier discovery

## v0.6.2

### New Features
- **Collection filter dropdown** in browse page filter row — collections are now composable with all other search filters (tag, rating, type, format, volume) directly from the browse page
- Batch toolbar collection buttons now sync from the filter-row dropdown instead of URL params

## v0.6.1

### New Features
- **Collection removal** from web UI — asset detail page shows collection membership chips with × remove buttons
- **Collection creation** from web UI — `/collections` page with "+ New Collection" button

## v0.6.0

### New Features
- **Saved searches** (smart albums) — `dam saved-search` (alias `ss`) with save, list, run, delete subcommands; stored in `searches.toml`; web UI chips on browse page with rename/delete on hover
- **Collections** (static albums) — `dam collection` (alias `col`) with create, list, show, add, remove, delete subcommands; SQLite-backed with YAML persistence; search filter `collection:<name>`; web UI batch toolbar integration
- **Quoted filter values** — search parser supports double-quoted values for multi-word filters (`tag:"Fools Theater"`, `collection:"My Favorites"`)

### Bug Fixes
- Fix saved search chip hover showing rename/delete buttons incorrectly

## v0.5.1

### New Features
- **Import `--dry-run` flag** — preview what an import would do without writing to catalog, sidecar, or disk
- **Inline name editing** in web UI — pencil icon toggle, text input with Save/Cancel

## v0.5.0

### New Features
- **Keyboard navigation** on browse page — arrow keys navigate cards (column-aware), Enter opens detail, Space toggles selection, 1–5/0 set/clear rating, Alt+1–7/0 set/clear color label, letter keys r/o/y/g/b/p/u/x for quick label

## v0.4.5

### New Features
- **`dam refresh` command** — re-read metadata from changed sidecar/recipe files without full re-import; supports `--dry-run`, `--json`, `--log`, `--time`

## v0.4.4

### New Features
- **Color labels** — first-class 7-color label support (Red, Orange, Yellow, Green, Blue, Pink, Purple); XMP `xmp:Label` extraction, CLI editing (`dam edit --label`), web UI color dot picker, browse filtering, batch operations, XMP write-back
- **Batch operations** in web UI — multi-select checkboxes, fixed bottom toolbar with tag add/remove, rating stars, color label dots
- **Keyboard shortcut hints** — platform-aware Cmd/Ctrl labels on toolbar buttons

### Bug Fixes
- Fix Ctrl+A not working after checkbox click
- Remove unreliable shift-click range selection, replace with Cmd/Ctrl+A

## v0.4.3

### New Features
- **Description XMP write-back** — description changes written back to `.xmp` recipe files on disk
- **Inline description editing** in web UI — pencil icon toggle, textarea with Save/Cancel

## v0.4.2

### New Features
- **Tag XMP write-back** — tag changes written back to `.xmp` recipe files using operation-level deltas (preserves tags added independently in CaptureOne)

## v0.4.1

### New Features
- **Rating XMP write-back** — rating changes written back to `.xmp` recipe files on disk, enabling bidirectional sync with CaptureOne

### Bug Fixes
- Fix back button and reload showing raw HTML instead of full browse page
- Refresh browse results when returning via back button (bfcache)

## v0.4.0

### New Features
- **Browse page redesign** — sort controls (Name/Date/Size with direction indicators), top pagination, star rating filter (click stars for minimum threshold)

### Bug Fixes
- Fix rating loss on pagination when sort changes

## v0.3.5

### New Features
- **Tags page enhancements** — sortable columns (name/count), live text filter, multi-column CSS layout
- **`dam update-location` command** — update file path in catalog after manual moves on disk

## v0.3.4

### New Features
- **Extended `dam cleanup`** — now removes orphaned assets (all variants have zero locations) and orphaned preview files, in addition to stale location records
- **Search location health filters** — `orphan:true`, `missing:true`, `stale:N`, `volume:none`

## v0.3.3

### New Features
- **`dam cleanup` command** — remove stale file location records for files no longer on disk

## v0.3.2

### New Features
- **`dam sync` command** — reconcile catalog with disk after external file moves, renames, or modifications

## v0.3.1

### New Features
- **`dam edit` command** — set or clear asset name, description, and rating from CLI
- **Photo workflow integration proposal** — documented gaps and planned features for CaptureOne integration

## v0.3.0

### New Features
- **Version display** in web UI navigation bar

## v0.2.0

### New Features
- **Web UI** (`dam serve`) — browse/search page with filter dropdowns, asset detail page, tag editing, rating support
- **First-class rating** — `Option<u8>` field on Asset with CLI search, web UI stars, XMP extraction
- **Stats page** in web UI with bar charts and tag cloud
- **Tags page** in web UI
- **Multi-tag chip input** with autocomplete on browse page
- **Metadata search** with indexed columns and extended filter syntax (camera, lens, ISO, focal, aperture, dimensions)
- **Info card previews** for non-visual formats (audio, documents) and as fallback for missing external tools
- **`dam.toml` configuration** — preview settings, serve settings, import exclude/auto_tags
- **`--log` flag** on `generate-previews` for per-file progress

### Bug Fixes
- Fix multi-component ASCII EXIF fields (Fuji lens_model parsing)

## v0.1.0

### New Features
- **`dam init`** — initialize catalog with SQLite schema, volume registry, config
- **`dam volume add/list`** — register and list storage volumes with online/offline detection
- **`dam import`** — SHA-256 hashing, EXIF extraction, stem-based auto-grouping, recipe handling, duplicate location tracking, preview generation
- **`dam search`** — text, type, tag, format filters
- **`dam show`** — full asset details with variants, locations, metadata
- **`dam tag`** — add/remove tags
- **`dam group`** — manually merge variant assets
- **`dam duplicates`** — find files with identical content across locations
- **`dam generate-previews`** — thumbnails for images, RAW (dcraw/LibRaw), video (ffmpeg)
- **`dam rebuild-catalog`** — regenerate SQLite from YAML sidecars
- **`dam relocate`** — copy/move assets between volumes with integrity verification
- **`dam verify`** — re-hash files to detect corruption or bit rot
- **Output formatting** — `--json`, `--format` templates, `-q` quiet mode, `-t` elapsed time
- **XMP metadata extraction** — keywords, rating, description, color label, creator, rights
