# Component Specification

## Data Model

### Asset
The central entity. Represents a logical asset (e.g. "photo of sunset at beach, 2024-07-15").

| Field | Type | Description |
|---|---|---|
| id | UUID | Stable identifier |
| name | String | User-assigned name (optional) |
| created_at | DateTime | When the asset was first imported |
| asset_type | Enum | Image, Video, Audio, Document, Other |
| tags | Vec<String> | User-defined tags |
| description | String | Free-text description (optional) |
| rating | Option<u8> | User/XMP rating 1–5, or unset |
| color_label | Option<String> | Color label: Red, Orange, Yellow, Green, Blue, Pink, Purple, or unset |
| best_variant_hash | String | Denormalized: content hash of the best display variant (Export > Processed > Original, image formats preferred, size tiebreak). Computed at write time. |
| primary_variant_format | String | Denormalized: identity format of the asset (Original+RAW first, then Original+any, then best variant). Computed at write time. |
| variant_count | u32 | Denormalized: number of variants belonging to this asset. Computed at write time. |
| stack_id | Option<UUID> | Stack this asset belongs to (NULL if unstacked) |
| stack_position | Option<i64> | Position within the stack (0 = pick) |

An asset groups one or more **variants**.

### Variant
A concrete file belonging to an asset. Multiple variants form a group (e.g. RAW + JPEG + edited TIFF).

| Field | Type | Description |
|---|---|---|
| content_hash | SHA-256 | Primary identity, derived from file content |
| asset_id | UUID | Parent asset |
| role | Enum | Original, Processed, Export, Sidecar |
| format | String | File extension / MIME type |
| file_size | u64 | Size in bytes |
| original_filename | String | Filename at import time |
| source_metadata | Map | EXIF, XMP, and other embedded metadata extracted at import |
| locations | Vec<FileLocation> | Where this variant physically lives |

### FileLocation
A physical location of a variant on a specific volume.

| Field | Type | Description |
|---|---|---|
| volume_id | UUID | Which volume |
| relative_path | PathBuf | Path relative to volume root |
| verified_at | DateTime | Last time hash was verified at this location |

### Volume
A storage device or mount point.

| Field | Type | Description |
|---|---|---|
| id | UUID | Stable identifier |
| label | String | Human-readable name (e.g. "Photos Archive 2024") |
| mount_point | PathBuf | Expected mount path (e.g. /Volumes/PhotosArchive) |
| volume_type | Enum | Local, External, Network |
| is_online | bool | Derived at runtime from mount point availability |

### Recipe
Processing instructions associated with a variant. During import, files with recognized recipe extensions that share a filename stem with a media file in the same directory are automatically attached as recipes rather than imported as variants. Standalone recipe files (imported without a co-located media file) are resolved to their parent variant by matching filename stem and directory.

Known recipe extensions: `.xmp` (Adobe/Lightroom/CaptureOne), `.cos` / `.cot` / `.cop` (CaptureOne session/template/preset), `.pp3` (RawTherapee), `.dop` (DxO), `.on1` (ON1).

| Field | Type | Description |
|---|---|---|
| id | UUID | Stable identifier |
| variant_hash | SHA-256 | Which variant this recipe belongs to |
| software | String | e.g. "CaptureOne 23", "Photoshop 2024" |
| recipe_type | Enum | Sidecar (XMP, COS, etc.), EmbeddedExport |
| content_hash | SHA-256 | Hash of the recipe file itself (mutable — updated when file changes) |
| location | FileLocation | Where the recipe file lives (primary identity for dedup) |
| verified_at | DateTime | Last time hash was verified at this location |

### Stack
A scene grouping that collapses multiple assets into a single pick in the browse grid. Anonymous (no name or description), position-based ordering. Stacks auto-dissolve when reduced to one member or fewer.

| Field | Type | Description |
|---|---|---|
| id | UUID | Stable identifier |
| created_at | DateTime | When the stack was created |
| member_count | u64 | Number of assets in the stack |

Stack membership is tracked on the `assets` table via `stack_id` and `stack_position` columns. Position 0 is the pick. Stacks are persisted in both SQLite (`stacks` table) and `stacks.yaml` at the catalog root for rebuild resilience.

**Design decision — location-based identity**: Recipes are identified by their location `(variant_hash, volume_id, relative_path)` rather than their content hash. This is because recipe files (XMP, COS, etc.) are routinely edited by external software. Re-importing after an external edit updates the recipe in place (new hash, re-extracted XMP metadata) rather than creating a duplicate. During verification, a changed recipe hash is reported as "modified" (not a failure) and the stored hash is updated.

## Components

### 1. Content Store

**Responsibility**: file identity, deduplication, and physical location tracking.

**Operations**:
- `ingest(path) -> SHA-256` — hash a file, register it. If hash already exists, skip copy (dedup).
- `locate(hash) -> Vec<FileLocation>` — find all known locations of a file.
- `relocate(hash, from_volume, to_volume)` — move/copy a file between volumes, update locations.
- `verify(hash, location) -> bool` — re-hash file at location, confirm integrity.
- `remove_location(hash, location)` — unregister a location (file moved/deleted externally).

**Storage model**: referenced mode — files stay in their original directory structure on each volume. The content store indexes their hash and location but never moves or renames originals. This preserves interoperability with tools like CaptureOne that expect a specific directory layout. Deduplication is logical (same hash → same variant) rather than physical.

### 2. Metadata Store

**Responsibility**: persist and retrieve all asset metadata as text-based sidecar files.

**Sidecar format**: YAML, one file per asset.
```yaml
# <catalog_root>/metadata/<uuid-prefix>/<uuid>.yaml
id: 550e8400-e29b-41d4-a716-446655440000
name: "Sunset at beach"
asset_type: image
tags: [landscape, sunset, beach, vacation-2024]
description: "Golden hour shot from Koh Lanta"
created_at: 2024-07-15T18:32:00Z
variants:
  - content_hash: "sha256:abcdef..."
    role: original
    format: NEF
    file_size: 52428800
    original_filename: "DSC_4521.NEF"
  - content_hash: "sha256:123456..."
    role: processed
    format: TIFF
    file_size: 104857600
    original_filename: "DSC_4521_edited.tiff"
recipes:
  - variant: "sha256:abcdef..."
    software: "CaptureOne 23"
    recipe_type: sidecar
    content_hash: "sha256:fedcba..."
```

**Operations**:
- `save(asset)` — write/update sidecar YAML.
- `load(asset_id) -> Asset` — read sidecar YAML.
- `list() -> Vec<AssetSummary>` — enumerate all known assets.
- `sync_to_catalog()` — rebuild SQLite catalog from sidecar files (source of truth → cache).

### 3. Local Catalog (SQLite)

**Responsibility**: fast queryable index over all metadata. Rebuilt from sidecar files.

**Tables** mirror the data model: `assets`, `variants`, `file_locations`, `volumes`, `recipes`, `stacks`, `collections`, `collection_assets`.

This is a **derived cache**, not the source of truth. Running `dam rebuild-catalog` regenerates it from sidecar files. This means:
- No data loss if the SQLite file is deleted.
- Sidecars can be edited manually or by external tools.
- The catalog can include denormalized fields for fast queries (e.g. extracted EXIF date, camera model).

### 4. Device Registry

**Responsibility**: volume management and online/offline detection.

**Operations**:
- `register(label, mount_point, type) -> Volume` — add a new volume.
- `list() -> Vec<Volume>` — list all volumes with online/offline status.
- `resolve_volume(label_or_id) -> Volume` — find a volume by label or UUID.
- `find_volume_for_path(path) -> Volume` — find which registered volume contains a given path.

**Online detection**: checks if the mount point directory exists (`mount_point.exists()`).

### 5. Asset Service

**Responsibility**: high-level operations that orchestrate the other components.

**Operations**:
- `import(paths, volume_id) -> ImportResult` — hash files, extract metadata (EXIF etc.), create assets, create variants, write sidecars, update catalog. Auto-groups files that share the same filename stem and reside in the same directory (e.g. `DSC_4521.NEF`, `DSC_4521.jpg`, `DSC_4521.xmp`, `DSC_4521.cos` all become one asset). Media files become variants; processing sidecars (`.xmp`, `.cos`, `.cot`, `.cop`, etc.) are attached as recipes. Standalone recipe files (no co-located media) are resolved to parent variants by matching filename stem and directory on the same volume. When a file's content hash already exists, the new file location is added to the existing variant (both sidecar and catalog) rather than being silently skipped. Only truly skips when the exact location (volume + relative path) is already tracked. Re-importing a modified recipe updates it in place (new hash, re-extracted XMP metadata). Reports per-file status as `Imported`, `LocationAdded`, `Skipped`, `RecipeAttached`, or `RecipeUpdated`. Supports `--include`/`--skip` flags for file type group filtering.
- `group(variant_hashes) -> Asset` — manually group variants into one asset.
- `tag(asset_id, tags)` — add tags to an asset.
- `relocate(asset_id, target_volume)` — move all variants of an asset to another volume. Supports `--remove-source` (move instead of copy) and `--dry-run`.
- `find_duplicates() -> Vec<DuplicateGroup>` — find variants with same hash on multiple locations.
- `verify(paths, volume, asset) -> VerifyResult` — re-hash files on disk and compare against stored content hashes. Reports `Ok`, `Mismatch`, `Modified` (recipe with changed hash), `Missing`, `Skipped`, or `Untracked`. Modified recipes are not treated as failures — their stored hash is updated. Supports path mode (verify specific files/dirs), catalog mode (verify all locations), `--volume`, `--asset`, and `--include`/`--skip` filters.
- `refresh(paths, volume, asset_id, dry_run, media) -> RefreshResult` — re-read metadata from changed recipe/sidecar files. Iterates recipe file locations, compares on-disk hash to stored hash, and for changed files re-extracts XMP metadata and updates catalog + sidecar. Reports `Unchanged`, `Refreshed`, `Missing`, or `Offline`. When `media` is true, also scans JPEG/TIFF variant files and re-extracts embedded XMP metadata. Lighter than `sync` — only touches metadata, never file locations.
- `fix_roles(paths, volume, asset, apply) -> FixRolesResult` — scan multi-variant assets with a RAW variant and re-role non-RAW variants from `Original` to `Export`. Assets with only non-RAW variants are untouched. Dry-run by default; `--apply` writes changes to both sidecar YAML and SQLite catalog.
- `cleanup(volume, apply) -> CleanupResult` — remove stale location/recipe records, orphaned assets, and orphaned previews.
- `sync(paths, volume, apply, remove_stale) -> SyncResult` — reconcile catalog with disk after external file moves/renames/modifications.

### 6. Query Engine

**Responsibility**: search and filter assets via the SQLite catalog.

**Query capabilities**:
- Filter by: tags, date range, asset type, format, rating (`rating:N` exact, `rating:N+` minimum), color label (`label:Red`), date (`date:2026-02-25` prefix match, `dateFrom:` inclusive lower bound, `dateUntil:` inclusive upper bound), camera model, lens, ISO, focal length, aperture, dimensions, volume, online/offline status
- Location health filters: `orphan:true` (no file locations), `missing:true` (files missing from disk), `stale:N` (not verified in N days), `volume:none` (no locations on online volumes)
- Full-text search over name, filename, description, and source metadata
- Sort by: date, name, file size, import date
- Output: asset list with summary info, or detailed asset view

**Editing capabilities**:
- `tag(asset_id, tags, remove)` — add or remove tags, with XMP write-back
- `edit(asset_id, fields)` — set/clear name, description, rating, color label, and date via `EditFields` (triple-option pattern: `None` = no change, `Some(None)` = clear, `Some(Some(x))` = set). Rating, description, and label changes trigger XMP write-back.
- `set_rating(asset_id, rating)` / `set_color_label(asset_id, label)` — individual field setters used by web UI and batch operations
- `auto_group(asset_ids, apply)` — group assets by filename stem using fuzzy prefix matching

### 7. Preview Generator

**Responsibility**: create and cache thumbnails for browsing.

**Approach**:
- Images: use `image` crate for common formats, shell out to `dcraw` or `libraw` for RAW files.
- Videos: shell out to `ffmpeg` to extract a frame.
- Non-visual formats (audio, documents, unknown): generate an info card — an 800x600 JPEG showing file metadata (name, format, size, and audio-specific properties like duration/bitrate via `lofty`). Uses `imageproc` for text rendering with an embedded DejaVu Sans font (`ab_glyph`).
- Fallback: when external tools (dcraw, ffmpeg) are missing, RAW and video files also get an info card instead of no preview.
- Store previews in `<catalog_root>/previews/<hash-prefix>/<hash>.jpg` at a standard size (800px longest edge for visual previews, 800x600 for info cards).
- Generate on import, regenerate on demand.

### 8. Output Formatting

**Responsibility**: flexible output for scripting, pipelines, and machine consumption.

**Module**: `src/format.rs` — template engine with `{placeholder}` substitution and escape sequences.

**Capabilities**:
- **Global `--json` flag**: available on all commands. Outputs structured JSON to stdout; human-readable messages go to stderr. All result types derive `serde::Serialize`.
- **`--format` flag** (on `search` and `duplicates`): presets (`ids`, `short`, `full`, `json`) or custom templates (`'{id}\t{name}\t{tags}'`). When `--format` is explicit, result counts are suppressed.
- **`-q`/`--quiet`** (on `search`): shorthand for `--format=ids`, outputting one UUID per line for scripting.
- **Template placeholders**: `{id}`, `{short_id}`, `{name}`, `{filename}`, `{type}`, `{format}`, `{date}`, `{tags}`, `{description}`, `{label}`, `{hash}`. Templates support `\t` and `\n` escape sequences.

### 9. Stats

**Responsibility**: aggregate and display catalog statistics from the SQLite index.

**Implementation**: query methods on `Catalog` (in `src/catalog.rs`) compute counts, breakdowns, and coverage metrics. The `build_stats()` method assembles all sections into a `CatalogStats` struct, merging catalog data with device registry (online/offline status).

**Sections** (each gated by a CLI flag):
- **Overview** (always shown): asset/variant/recipe counts, volume totals (online/offline), total file size.
- **Types** (`--types`): asset type breakdown with percentages, top variant formats, recipe format distribution.
- **Volumes** (`--volumes`): per-volume asset/variant/recipe counts, size, directory count, format list, verification coverage.
- **Tags** (`--tags`): unique tag count, tagged/untagged asset counts, top tags by frequency.
- **Verification** (`--verified`): location verification coverage, oldest/newest timestamps, per-volume breakdown.

**Flags**: `--all` enables all sections. `--limit N` controls top-N lists (default 20). `--json` outputs structured `CatalogStats` JSON.

**Edge cases**: empty catalog returns all zeros without errors. Division-by-zero for percentages is guarded. Volumes with no files are included in `--volumes` with zero counts. Recipe format is extracted from `relative_path` extension in Rust, falling back to "unknown".

### 10. Web UI

**Responsibility**: browser-based interface for browsing, searching, and editing assets.

**Module**: `src/web/` — axum server with askama templates and htmx interactivity.

**Architecture**:
- `AppState` holds the catalog root path, preview config, and `log_requests` flag. Schema migrations run once at server startup via `Catalog::open()`. Each request opens a fresh connection via `Catalog::open_fast()` (skips migrations) through `tokio::task::spawn_blocking` (since `rusqlite::Connection` is not `Send`). When `--log` is enabled, a middleware layer logs each request's method, URI, status, and duration to stderr.
- Static assets (htmx.min.js, style.css) are embedded at compile time via `include_bytes!`/`include_str!`.
- Preview images are served directly from the catalog's `previews/` directory via `tower-http::ServeDir`.

**Routes**:
- `GET /` — browse page with search, filter dropdowns (type, tag, format, volume, collection, rating), color label filter dots, sort, pagination, grid density controls (compact/normal/large), thumbnail grid with star ratings and color label dots. Grid/Calendar view toggle (calendar shows year-at-a-glance heatmap with per-day asset counts, year navigation, day-click filtering). Lightbox overlay on card click (prev/next navigation, info panel with rating/label editing). Batch operations toolbar with tag, rating, and label editing. Dark mode toggle in nav bar (persisted in localStorage, follows OS preference). Favorite saved searches shown as chips with "Manage..." link to `/saved-searches`.
- `GET /asset/{id}` — asset detail with preview, metadata, editable tags, inline editable star rating, inline color label picker (7 color dots), variants, recipes
- `GET /tags` — tags page with sortable columns (name/count), live text filter, multi-column layout
- `GET /api/search` — results partial (htmx target) with pagination
- `POST /api/asset/{id}/tags` — add tags, returns tags fragment
- `DELETE /api/asset/{id}/tags/{tag}` — remove tag, returns tags fragment
- `PUT /api/asset/{id}/rating` — set/clear rating (form: `rating=N`), returns rating fragment
- `PUT /api/asset/{id}/description` — set/clear description, returns description fragment
- `PUT /api/asset/{id}/label` — set/clear color label (form: `label=Red`), returns label fragment
- `POST /api/asset/{id}/preview` — generate preview on demand
- `PUT /api/batch/rating` — batch set/clear rating for multiple assets
- `POST /api/batch/tags` — batch add/remove tags for multiple assets
- `PUT /api/batch/label` — batch set/clear color label for multiple assets
- `GET /api/tags` — all tags as JSON (for autocomplete)
- `GET /api/stats` — catalog stats as JSON
- `GET /collections` — collections page listing all collections with "+ New Collection" button
- `GET /api/collections` — list all collections as JSON
- `POST /api/collections` — create a new collection (JSON: `{name, description?}`)
- `POST /api/batch/collection` — batch add assets to a collection (JSON: `{asset_ids, collection}`)
- `DELETE /api/batch/collection` — batch remove assets from a collection (JSON: `{asset_ids, collection}`)
- `POST /api/batch/auto-group` — batch auto-group selected assets by stem (JSON: `{asset_ids}`)
- `POST /api/batch/stack` — create a stack from selected assets (JSON: `{asset_ids}`)
- `DELETE /api/batch/stack` — unstack selected assets (JSON: `{asset_ids}`)
- `PUT /api/asset/{id}/stack-pick` — set this asset as the stack pick
- `DELETE /api/asset/{id}/stack` — dissolve the stack this asset belongs to
- `PUT /api/asset/{id}/name` — set/clear asset name (form: `name=text`), returns name fragment
- `GET /api/saved-searches` — list all saved searches as JSON
- `POST /api/saved-searches` — save a new search (JSON: `{name, query, sort?, favorite?}`)
- `DELETE /api/saved-searches/{name}` — delete a saved search
- `PUT /api/saved-searches/{name}/favorite` — toggle favorite status (JSON: `{favorite: bool}`)
- `PUT /api/saved-searches/{name}/rename` — rename a saved search (JSON: `{new_name}`)
- `GET /api/calendar` — calendar heatmap data as JSON (`{year, counts, years}`), respects all search filter params
- `GET /saved-searches` — saved searches management page (table with favorite toggle, rename, delete)
- `GET /duplicates` — duplicates page with summary cards, mode tabs (all/same/cross), filters (path/format/volume), preview thumbnails, lightbox overlay, per-location remove buttons, and auto-resolve
- `POST /api/dedup/resolve` — auto-resolve same-volume duplicates with optional filters and prefer (deletes files and co-located recipes, returns `DedupResult` with `locations_removed, recipes_removed, bytes_freed, errors`)
- `DELETE /api/dedup/location` — remove a specific file location and co-located recipes (JSON: `{content_hash, volume_id, relative_path}`)

**Catalog extensions** (in `src/catalog.rs`):
- `SearchOptions` / `SearchSort` / `SearchPage` — paginated search with volume filter, date filters, and dynamic sort
- `search_paginated()` / `search_count()` — paginated search queries
- `calendar_counts(year, opts)` — per-day asset counts for a given year, respecting all search filters
- `calendar_years()` — distinct years that have assets
- `list_all_tags()` — unique tags with counts
- `list_all_formats()` — distinct variant formats
- `list_volumes()` — volume IDs and labels

### 11. Config Module

**Responsibility**: parse and provide catalog configuration from `dam.toml`.

**Module**: `src/config.rs` — `CatalogConfig` struct with sub-structs `PreviewConfig`, `ServeConfig`, `ImportConfig`, `DedupConfig`.

**Sections**:
- `[preview]`: `max_edge` (default 800), `format` ("jpeg"/"webp", default "jpeg"), `quality` (1–100, default 85)
- `[serve]`: `port` (default 8080), `bind` (default "127.0.0.1"). CLI flags override.
- `[import]`: `exclude` (glob patterns matched against filenames), `auto_tags` (merged into new assets)
- `[dedup]`: `prefer` (default path substring for `--prefer` flag, used by CLI and web UI)
- `default_volume`: fallback volume UUID for import

All sections and fields are optional — missing fields use defaults.

### 12. EXIF Reader

**Responsibility**: extract EXIF metadata from image files at import time.

**Module**: `src/exif_reader.rs` — uses `kamadak-exif` crate.

**Extracted fields**: camera model, lens model, ISO, focal length, aperture (f-number), image dimensions (width/height), date/time original. All stored in the variant's `source_metadata` map.

### 13. XMP Reader

**Responsibility**: extract and write back XMP metadata for bidirectional sync with photo editing tools.

**Module**: `src/xmp_reader.rs` — uses `quick-xml` crate for parsing, string-based find/replace for write-back.

**Read operations**: `extract_xmp_metadata(path)` — parses `dc:subject` (keywords/tags), `dc:description`, `xmp:Rating`, `xmp:Label`, `dc:creator`, `dc:rights` from XMP sidecar files.

**Write operations** (all preserve existing XMP structure):
- `update_rating(path, rating)` — write `xmp:Rating` value
- `update_tags(path, added, removed)` — delta-based `dc:subject`/`rdf:Bag` editing (preserves externally-added tags)
- `update_description(path, description)` — write/clear/inject `dc:description`/`rdf:Alt`/`rdf:li`
- `update_label(path, label)` — write/clear `xmp:Label`

After each write, the file is re-hashed and the recipe's `content_hash` is updated in both catalog and sidecar.

### 14. Collection Store

**Responsibility**: manage static album collections.

**Module**: `src/collection_store.rs` — dual storage: SQLite tables (`collections`, `collection_assets`) for fast queries + `collections.yaml` at catalog root for persistence across `rebuild-catalog`.

**Operations**:
- `create(name, description)` — create a new collection
- `list()` — list all collections with asset counts
- `show(name)` — list asset IDs in a collection
- `add(name, asset_ids)` — add assets to a collection
- `remove(name, asset_ids)` — remove assets from a collection
- `delete(name)` — delete a collection
- `restore_from_yaml()` — rebuild SQLite tables from YAML (used during `rebuild-catalog`)

### 15. Saved Search Store

**Responsibility**: manage named search queries (smart albums).

**Module**: `src/saved_search_store.rs` — stored in `searches.toml` at catalog root.

**Operations**:
- `save(name, query, sort, favorite)` — save or replace a named search
- `list()` — list all saved searches with query, sort, and favorite status
- `run(name)` — execute a saved search and return results
- `delete(name)` — delete a saved search

**Favorite field**: Each saved search has a `favorite: bool` field (default `false`). Only favorites are shown as chips on the browse page. The `/saved-searches` management page shows all searches and allows toggling favorites.

### 16. Stack Store

**Responsibility**: manage asset stacks (scene groupings).

**Module**: `src/stack.rs` — dual storage: SQLite `stacks` table for fast queries + `stacks.yaml` at catalog root for persistence across `rebuild-catalog`.

**Operations**:
- `create(asset_ids)` — create a new stack (minimum 2 assets, first is the pick)
- `add(reference_asset_id, new_asset_ids)` — add assets to an existing stack
- `remove(asset_ids)` — remove assets from their stacks (auto-dissolves if <=1 member remains)
- `set_pick(asset_id)` — set an asset as the stack pick (position 0)
- `dissolve(asset_id)` — dissolve the entire stack
- `list()` — list all stacks with summary info
- `stack_for_asset(asset_id)` — get the stack and ordered members for an asset
- `export_all()` — export all stacks to `StacksFile` for YAML persistence
- `import_from_yaml(file)` — rebuild SQLite from YAML (used during `rebuild-catalog`)

**Browse grid integration**: When stacks are collapsed (default), only the pick (position 0) is shown. Stack badges indicate member count.

### 17. CLI

**Global flags**:
- `--json` — output machine-readable JSON
- `-l` / `--log` — log individual file progress (import, verify, sync, refresh, cleanup, generate-previews); per-request logging for `serve`
- `-d` / `--debug` — show stderr output from external tools (ffmpeg, dcraw, dcraw_emu)
- `-t` / `--time` — show elapsed time after command execution

**Subcommands**:
```
dam init                                          # initialize a new catalog
dam volume add <label> <path>                     # register a volume
dam volume list                                   # list volumes and status
dam import <paths...> [--volume V] [--include G] [--skip G]  # import files
dam search <query> [--format F] [-q]              # search assets (label:Red filter)
dam show <asset-id>                               # show asset details
dam tag <asset-id> [--remove] <tags...>           # add/remove tags
dam edit <id> [--name N] [--description T] [--rating R] [--label C] [--clear-*]  # edit metadata
dam group <variant-hashes...>                     # group variants into one asset
dam relocate <id> <vol> [--remove-source] [--dry-run]  # copy/move asset
dam verify [PATHS...] [--volume V] [--asset ID] [--include G] [--skip G]  # check file integrity
dam sync <PATHS...> [--volume V] [--apply] [--remove-stale]  # reconcile catalog with disk
dam refresh [PATHS...] [--volume V] [--asset ID] [--dry-run] [--media]  # re-read metadata from changed sidecars
dam update-location <id> --from <old> --to <new> [--volume V]  # update path after manual move
dam cleanup [--volume V] [--list] [--apply]       # remove stale locations, orphaned assets, and previews
dam duplicates [--same-volume] [--cross-volume] [--volume V] [--filter-format F] [--path P] [--format FMT]  # find duplicates
dam dedup [--volume V] [--prefer S] [--filter-format F] [--path P] [--min-copies N] [--apply]  # remove same-volume duplicates
dam generate-previews [PATHS...] [--asset ID] [--volume V] [--include G] [--skip G] [--force]  # generate thumbnails
dam stats [--types] [--volumes] [--tags] [--verified] [--all] [--limit N]  # catalog statistics
dam auto-group [QUERY] [--apply]                  # group assets by filename stem
dam fix-roles [PATHS...] [--volume V] [--asset ID] [--apply]  # fix variant roles in RAW+non-RAW groups
dam saved-search save|list|run|delete             # manage saved searches (alias: ss, save supports --favorite)
dam collection create|list|show|add|remove|delete # manage collections (alias: col)
dam rebuild-catalog                               # rebuild SQLite from sidecars
dam serve [--port P] [--bind ADDR] [--log]         # start web UI server (--log for request logging)
```

## Catalog Directory Structure

```
<catalog_root>/                       # e.g. ~/dam/ or wherever `dam init` was run
  dam.toml                            # catalog configuration (default volume, preferences)
  catalog.db                          # SQLite index (derived, rebuildable)
  searches.toml                       # saved search definitions
  collections.yaml                    # collection membership (persists across rebuild-catalog)
  stacks.yaml                         # stack membership (persists across rebuild-catalog)
  metadata/
    55/
      550e8400-e29b-41d4-...yaml      # asset sidecar files, sharded by UUID prefix
  previews/
    ab/
      abcdef1234....jpg               # thumbnails, sharded by content hash prefix
  volumes.yaml                        # volume registry
```
