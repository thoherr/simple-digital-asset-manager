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
- **Key crates**: clap, sha2, serde, rusqlite, kamadak-exif, quick-xml, image, imageproc, ab_glyph, lofty, uuid, axum, askama, tokio, tower-http
- **External tools**: dcraw/libraw (RAW previews), ffmpeg (video thumbnails)

## Architecture

See `doc/architecture-overview.md` for the high-level system design and `doc/component-specification.md` for detailed component specs.

Core layers: CLI → Core Library (Asset Service, Content Store, Metadata Store, Device Registry, Query Engine, Preview Generator) → Storage (Local Catalog + Media Volumes).

## Status

Core CLI is functional. See `specification.md` for full requirements.

**Implemented commands**: `init`, `volume add/list`, `import`, `search`, `show`, `tag`, `group`, `duplicates`, `generate-previews`, `rebuild-catalog`, `relocate`, `verify`, `stats`, `serve`

**Import behavior**:
- **Stem-based auto-grouping**: Files sharing the same filename stem in the same directory are grouped into one Asset during import. RAW files take priority as the primary variant (defining asset identity and EXIF data). Additional media files become extra variants on the same asset.
- **Recipe handling**: Processing sidecars (`.xmp`, `.cos`, `.cot`, `.cop`, `.pp3`, `.dop`, `.on1`) are attached as Recipe records to the primary variant rather than imported as standalone assets. Recipes are identified by location `(volume_id, relative_path)` rather than content hash, so re-importing after external edits updates in place instead of creating duplicates.
- **Standalone recipe resolution**: When a recipe file is imported without a co-located media file (e.g., importing `DSC_001.xmp` after `DSC_001.nef` was already imported), the system finds the parent variant by matching the filename stem and directory, and attaches the recipe to it.
- **XMP metadata extraction**: When an `.xmp` sidecar is attached as a recipe, its contents are parsed (via `xmp_reader` module) and merged into the asset/variant. Keywords (`dc:subject`) become asset tags (deduplicated), `dc:description` sets the asset description (if not already set), `xmp:Rating` is promoted to `asset.rating` (first-class `Option<u8>` field), and `xmp:Label`, `dc:creator`, `dc:rights` are stored in the primary variant's `source_metadata`. Rating is also kept in `source_metadata` for provenance. EXIF takes precedence for any overlapping `source_metadata` keys. On re-import with changed content, XMP metadata is re-extracted: description is overwritten, rating is overwritten, keywords are merged, and source_metadata keys are overwritten.
- **Duplicate location tracking**: When a file's content hash already exists, the new file location is added to the existing variant rather than silently skipping. Only truly skips when the exact same location (volume + path) is already recorded.
- **Preview generation**: During import, previews are generated for each variant. Standard image formats use the `image` crate (800px JPEG thumbnails); RAW files use `dcraw` or `dcraw_emu` (LibRaw); videos use `ffmpeg`. Non-visual formats (audio, documents, unknown) get an info card — an 800x600 JPEG showing file metadata (name, format, size, and audio-specific properties like duration/bitrate extracted via `lofty`). Info cards are also generated as a fallback when external tools are missing for RAW/video files. Info card rendering uses `imageproc`/`ab_glyph` with an embedded DejaVu Sans font. Previews are stored in `previews/<hash-prefix>/<hash>.jpg`. Preview failure never blocks import.
- **Show command**: Displays variants, attached recipes, and preview status for an asset.
- **Import `--volume` flag**: `dam import [--volume <label>] <PATHS...>` — when provided, uses the specified volume instead of auto-detecting from the first path. Useful when auto-detection picks the wrong volume.
- **Generate-previews command**: `dam generate-previews [PATHS...] [--asset <id>] [--volume <label>] [--include <group>] [--skip <group>] [--force]`. Without PATHS, iterates all catalog assets (optionally filtered by `--asset` or `--volume`). With PATHS, resolves files on disk and looks up their variants in the catalog. `--include`/`--skip` filter by file type group. `--force` regenerates existing previews.
- **Relocate command**: Copies or moves all files of an asset (variants + recipes) to a target volume. `dam relocate <asset-id> <target-volume> [--remove-source] [--dry-run]`. Without `--remove-source`, files are copied and the asset gains additional locations. With `--remove-source`, source files are deleted after verified copy. `--dry-run` shows what would happen without making changes. Preserves relative paths on the target volume. Verifies file integrity via SHA-256 after copy.

- **Verify command**: Re-hashes files on disk and compares against stored content hashes to detect corruption or bit rot. `dam verify [PATHS...] [--volume <label>] [--asset <id>]`. Without arguments, verifies all file locations on all online volumes. With paths, verifies specific files/directories. `--volume` limits to a specific volume; `--asset` limits to a specific asset. Updates `verified_at` timestamps on successful verification. Exits with code 1 if any mismatches are found. Recipe files that have been modified externally are reported as "modified" (not "FAILED") and do not trigger exit code 1 — their stored hash is updated to reflect the new content.

- **Stats command**: Shows catalog statistics. `dam stats [--types] [--volumes] [--tags] [--verified] [--all] [--limit N]`. Without flags, shows overview only (assets, variants, recipes, volumes, total size). `--types` adds asset type and format breakdown. `--volumes` adds per-volume details (assets, size, directories, verification). `--tags` shows tag usage frequencies. `--verified` shows verification health. `--all` enables all sections. `--limit N` controls top-N lists (default 20). Supports `--json` for structured output.

- **Serve command**: `dam serve [--port <port>] [--bind <addr>]`. Starts a web UI server (default `127.0.0.1:8080`). Browse page with search/filter/sort/pagination and rating filter dropdown. Asset detail page with preview, metadata, editable tags, inline editable star rating, variants, and recipes. Uses htmx for partial page updates (tags and rating). Star ratings displayed on browse cards. `PUT /api/asset/{id}/rating` endpoint for inline editing. SQLite connections are opened per-request via `spawn_blocking`. Previews served via `tower-http::ServeDir`. Static assets (htmx.min.js, style.css) embedded at compile time.

**Output formatting**:
- **Global `--json` flag**: Available on all commands. Outputs structured JSON to stdout; human-readable messages go to stderr. All data types (`SearchRow`, `AssetDetails`, `ImportResult`, `VerifyResult`, `RelocateResult`, `DuplicateEntry`) derive `serde::Serialize`.
- **Global `--debug` / `-d` flag**: Shows stderr output from external tools (ffmpeg, dcraw, dcraw_emu) for diagnosing preview generation issues. Prints the command line and stderr to eprintln.
- **Rating**: First-class `Option<u8>` field on `Asset` (persisted in sidecar YAML and SQLite `assets.rating` column). Extracted from XMP during import. Editable via `QueryEngine::set_rating()` and web UI inline stars. Filterable in CLI search (`rating:N` exact, `rating:N+` minimum) and web UI (dropdown). Displayed as stars in `dam show` output and web UI browse cards/asset detail.
- **`search --format`**: Presets: `ids` (one UUID per line), `short` (default), `full` (with tags/description), `json` (JSON array). Custom templates: `'{id}\t{name}\t{tags}'` with placeholder substitution. Result count suppressed when `--format` is explicit.
- **`search -q`**: Shorthand for `--format=ids`, for scripting (e.g. `for id in $(dam search -q tag:landscape); do ...`).
- **`duplicates --format`**: Same presets as search, plus `{locations}` placeholder for templates.
- **Format module** (`src/format.rs`): Template engine with `{placeholder}` substitution, escape sequences (`\t`, `\n`), preset parsing.
