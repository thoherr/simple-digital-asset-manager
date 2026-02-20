# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

A digital asset manager designed for large collections of images and videos (terabytes across multiple offline storage devices). Key design goals:

- Content-addressable storage for originals (SHA-based, since originals like RAW files are immutable)
- Text-based metadata in sidecar files
- Grouping of asset variants (RAW/JPEG, different processing versions)
- Deduplication of identical files
- Transparent file relocation across storage devices
- Management of processing recipes (CaptureOne, Photoshop, etc.)
- Location-independent navigation and retrieval

## Technology

- **Language**: Rust
- **Platforms**: macOS, Linux
- **Interface**: CLI-first (`dam` command)
- **Catalog**: SQLite (cache/index), YAML sidecar files (source of truth)
- **Key crates**: clap, sha2, serde, rusqlite, kamadak-exif, quick-xml, image, imageproc, ab_glyph, lofty, uuid, axum, askama, tokio, tower-http, toml, glob-match
- **External tools**: dcraw/libraw (RAW previews), ffmpeg (video thumbnails)

## Architecture

See `doc/architecture-overview.md` for the high-level system design and `doc/component-specification.md` for detailed component specs.

Core layers: CLI → Core Library (Asset Service, Content Store, Metadata Store, Device Registry, Query Engine, Preview Generator) → Storage (Local Catalog + Media Volumes).

## Status

Core CLI is functional. See `specification.md` for full requirements.

**Implemented commands**: `init`, `volume add/list`, `import`, `search`, `show`, `tag`, `edit`, `group`, `duplicates`, `generate-previews`, `rebuild-catalog`, `relocate`, `update-location`, `verify`, `sync`, `cleanup`, `stats`, `serve`

**Import behavior**:
- **Stem-based auto-grouping**: Files sharing the same filename stem in the same directory are grouped into one Asset during import. RAW files take priority as the primary variant (defining asset identity and EXIF data). Additional media files become extra variants on the same asset.
- **Recipe handling**: Processing sidecars (`.xmp`, `.cos`, `.cot`, `.cop`, `.pp3`, `.dop`, `.on1`) are attached as Recipe records to the primary variant rather than imported as standalone assets. Recipes are identified by location `(volume_id, relative_path)` rather than content hash, so re-importing after external edits updates in place instead of creating duplicates.
- **Standalone recipe resolution**: When a recipe file is imported without a co-located media file (e.g., importing `DSC_001.xmp` after `DSC_001.nef` was already imported), the system finds the parent variant by matching the filename stem and directory, and attaches the recipe to it.
- **XMP metadata extraction**: When an `.xmp` sidecar is attached as a recipe, its contents are parsed (via `xmp_reader` module) and merged into the asset/variant. Keywords (`dc:subject`) become asset tags (deduplicated), `dc:description` sets the asset description (if not already set), `xmp:Rating` is promoted to `asset.rating` (first-class `Option<u8>` field), and `xmp:Label`, `dc:creator`, `dc:rights` are stored in the primary variant's `source_metadata`. Rating is also kept in `source_metadata` for provenance. EXIF takes precedence for any overlapping `source_metadata` keys. On re-import with changed content, XMP metadata is re-extracted: description is overwritten, rating is overwritten, keywords are merged, and source_metadata keys are overwritten.
- **Duplicate location tracking**: When a file's content hash already exists, the new file location is added to the existing variant rather than silently skipping. Only truly skips when the exact same location (volume + path) is already recorded.
- **Preview generation**: During import, previews are generated for each variant. Standard image formats use the `image` crate (800px JPEG thumbnails); RAW files use `dcraw` or `dcraw_emu` (LibRaw); videos use `ffmpeg`. Non-visual formats (audio, documents, unknown) get an info card — an 800x600 JPEG showing file metadata (name, format, size, and audio-specific properties like duration/bitrate extracted via `lofty`). Info cards are also generated as a fallback when external tools are missing for RAW/video files. Info card rendering uses `imageproc`/`ab_glyph` with an embedded DejaVu Sans font. Previews are stored in `previews/<hash-prefix>/<hash>.jpg`. Preview failure never blocks import.
- **Show command**: Displays variants, attached recipes, and preview status for an asset.
- **Edit command**: `dam edit <asset-id> [--name <name>] [--description <text>] [--rating <1-5>] [--clear-name] [--clear-description] [--clear-rating]`. Sets or clears asset name, description, and rating from the CLI. At least one flag is required. Updates both sidecar YAML and SQLite catalog. Supports `--json` for structured output. Uses `QueryEngine::edit()` with `EditFields` (triple-option pattern: `None` = no change, `Some(None)` = clear, `Some(Some(x))` = set).
- **Import `--volume` flag**: `dam import [--volume <label>] <PATHS...>` — when provided, uses the specified volume instead of auto-detecting from the first path. Useful when auto-detection picks the wrong volume.
- **Generate-previews command**: `dam generate-previews [PATHS...] [--asset <id>] [--volume <label>] [--include <group>] [--skip <group>] [--force]`. Without PATHS, iterates all catalog assets (optionally filtered by `--asset` or `--volume`). With PATHS, resolves files on disk and looks up their variants in the catalog. `--include`/`--skip` filter by file type group. `--force` regenerates existing previews.
- **Relocate command**: Copies or moves all files of an asset (variants + recipes) to a target volume. `dam relocate <asset-id> <target-volume> [--remove-source] [--dry-run]`. Without `--remove-source`, files are copied and the asset gains additional locations. With `--remove-source`, source files are deleted after verified copy. `--dry-run` shows what would happen without making changes. Preserves relative paths on the target volume. Verifies file integrity via SHA-256 after copy.

- **Update-location command**: Updates a file's catalog path after it was manually moved on disk. `dam update-location <asset-id> --from <old-path> --to <new-path> [--volume <label>]`. `--to` must be an absolute path to the file's current location. `--from` can be absolute or volume-relative. Volume is auto-detected from `--to` via `find_volume_for_path()`, or specified via `--volume`. Verifies the file at `--to` has the same content hash as the catalog record (safety check). Updates both SQLite catalog and sidecar YAML. Handles both variant file locations and recipe file locations. Supports `--json` (via global flag).

- **Verify command**: Re-hashes files on disk and compares against stored content hashes to detect corruption or bit rot. `dam verify [PATHS...] [--volume <label>] [--asset <id>]`. Without arguments, verifies all file locations on all online volumes. With paths, verifies specific files/directories. `--volume` limits to a specific volume; `--asset` limits to a specific asset. Updates `verified_at` timestamps on successful verification. Exits with code 1 if any mismatches are found. Recipe files that have been modified externally are reported as "modified" (not "FAILED") and do not trigger exit code 1 — their stored hash is updated to reflect the new content.

- **Sync command**: Reconciles catalog with disk reality after external tools move, rename, or modify files. `dam sync <PATHS...> [--volume <label>] [--apply] [--remove-stale]`. Without `--apply`, runs in report-only mode (safe default). With `--apply`, updates catalog and sidecar files for moved files and modified recipes. `--remove-stale` (requires `--apply`) removes catalog location records for confirmed-missing files. Detects: unchanged files (hash matches at expected path), moved files (known hash at new path, old path gone), new files (unknown hash), modified recipes (same path, different hash), missing files (catalog location but file gone from disk). New files are reported but not auto-imported — user runs `dam import` separately. Supports `--json`, `--log`, `--time` flags.

- **Cleanup command**: Scans all file locations and recipes across online volumes, checking for files that no longer exist on disk. `dam cleanup [--volume <label>] [--list] [--apply]`. Without `--apply`, runs in report-only mode (safe default). With `--apply`, performs three passes: (1) removes stale location records and recipe records for missing files from catalog and sidecar YAML, (2) deletes orphaned assets (assets where all variants have zero file_locations) along with their recipes, variants, catalog rows, and sidecar YAML files, (3) removes orphaned preview files (previews whose content hash no longer matches any variant in the catalog). `--volume` limits stale-location scanning to a specific volume (otherwise checks all online volumes). `--list` prints stale entries to stderr (unlike `--log` which prints all entries including ok). Skips offline volumes with a note. Checks both variant file locations and recipe file locations. Report-only mode predicts orphaned assets and previews that would result from removing stale locations. Supports `--json`, `--log`, `--time` flags.

- **Stats command**: Shows catalog statistics. `dam stats [--types] [--volumes] [--tags] [--verified] [--all] [--limit N]`. Without flags, shows overview only (assets, variants, recipes, volumes, total size). `--types` adds asset type and format breakdown. `--volumes` adds per-volume details (assets, size, directories, verification). `--tags` shows tag usage frequencies. `--verified` shows verification health. `--all` enables all sections. `--limit N` controls top-N lists (default 20). Supports `--json` for structured output.

- **Serve command**: `dam serve [--port <port>] [--bind <addr>]`. Starts a web UI server (default `127.0.0.1:8080`). Browse page with two-row search bar (full-width text input + button on row 1; tag filter, clickable star rating filter, type/format/volume dropdowns on row 2). Results bar with inline sort toggle buttons (Name/Date/Size with ▲/▼ direction indicators), top pagination (« ‹ pages › »), and page indicator — bottom pagination also present. Asset detail page with preview, metadata, editable tags, inline editable star rating, variants, and recipes. Tags page (`/tags`) with sortable columns (name/count, clickable headers with ▲/▼ direction indicators), live text filter (2+ characters), and multi-column layout (CSS `columns`, auto-adapts to viewport). Uses htmx for partial page updates (tags, rating, search results, sort, pagination). Star ratings displayed on browse cards. `PUT /api/asset/{id}/rating` endpoint for inline editing. SQLite connections are opened per-request via `spawn_blocking`. Previews served via `tower-http::ServeDir`. Static assets (htmx.min.js, style.css) embedded at compile time.

**Output formatting**:
- **Global `--json` flag**: Available on all commands. Outputs structured JSON to stdout; human-readable messages go to stderr. All data types (`SearchRow`, `AssetDetails`, `ImportResult`, `VerifyResult`, `SyncResult`, `CleanupResult`, `RelocateResult`, `DuplicateEntry`) derive `serde::Serialize`.
- **Global `--log` / `-l` flag**: Per-file progress logging for multi-file commands (import, verify, sync, cleanup, generate-previews). Each file prints `filename — status (duration)` to stderr.
- **Global `--debug` / `-d` flag**: Shows stderr output from external tools (ffmpeg, dcraw, dcraw_emu) for diagnosing preview generation issues. Prints the command line and stderr to eprintln.
- **Global `--time` / `-t` flag**: Shows total elapsed time after command execution.
- **Rating**: First-class `Option<u8>` field on `Asset` (persisted in sidecar YAML and SQLite `assets.rating` column). Extracted from XMP during import. Editable via `QueryEngine::set_rating()` and web UI inline stars. Filterable in CLI search (`rating:N` exact, `rating:N+` minimum) and web UI (clickable star filter). Displayed as stars in `dam show` output and web UI browse cards/asset detail.
- **`search --format`**: Presets: `ids` (one UUID per line), `short` (default), `full` (with tags/description), `json` (JSON array). Custom templates: `'{id}\t{name}\t{tags}'` with placeholder substitution. Result count suppressed when `--format` is explicit.
- **`search -q`**: Shorthand for `--format=ids`, for scripting (e.g. `for id in $(dam search -q tag:landscape); do ...`).
- **Search location health filters**: `orphan:true` (assets with zero file_location records), `missing:true` (assets where at least one location points to a non-existent file on an online volume — requires disk I/O), `stale:N` (assets with at least one location not verified in N days, or never verified), `volume:none` (assets with no locations on any currently online volume). These filters combine with all other search filters. `orphan:true` and `stale:N` are pure SQL; `missing:true` pre-computes affected asset IDs via disk checks; `volume:none` pre-computes online volume IDs from DeviceRegistry.
- **`duplicates --format`**: Same presets as search, plus `{locations}` placeholder for templates.
- **Format module** (`src/format.rs`): Template engine with `{placeholder}` substitution, escape sequences (`\t`, `\n`), preset parsing.

**Configuration** (`dam.toml`):
- Parsed via `toml` crate with serde. All sections optional; missing fields use defaults. Validated on load.
- `default_volume: Option<Uuid>` — fallback volume for import.
- `[preview]`: `max_edge: u32` (default 800), `format: "jpeg"|"webp"` (default jpeg), `quality: u8` 1–100 (default 85, JPEG only; WebP is lossless via `image` crate). Stored in `PreviewConfig`. Passed to `PreviewGenerator` and `AssetService`.
- `[serve]`: `port: u16` (default 8080), `bind: String` (default "127.0.0.1"). CLI `--port`/`--bind` flags override.
- `[import]`: `exclude: Vec<String>` (glob patterns matched against filenames via `glob_match`), `auto_tags: Vec<String>` (merged into new assets, deduplicated with XMP tags). Passed to `import_with_callback` and `resolve_files`.
- Config structs: `CatalogConfig` in `src/config.rs` with sub-structs `PreviewConfig`, `ServeConfig`, `ImportConfig`. User-facing documentation in `README.md`.
