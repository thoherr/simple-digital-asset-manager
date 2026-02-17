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
- **Interface**: CLI-first (`dam` command), optional web GUI
- **Catalog**: SQLite (cache/index), YAML sidecar files (source of truth)
- **Key crates**: clap, sha2, serde, rusqlite, axum, kamadak-exif, quick-xml, image
- **External tools**: dcraw/libraw (RAW previews), ffmpeg (video thumbnails)

## Architecture

See `doc/architecture-overview.md` for the high-level system design and `doc/component-specification.md` for detailed component specs.

Core layers: CLI → Core Library (Asset Service, Content Store, Metadata Store, Device Registry, Query Engine, Preview Generator) → Storage (Local Catalog + Media Volumes).

## Status

Core CLI is functional. See `specification.md` for full requirements.

**Implemented commands**: `init`, `volume add/list`, `import`, `search`, `show`, `tag`, `group`, `duplicates`, `generate-previews`, `rebuild-catalog`

**Import behavior**:
- **Stem-based auto-grouping**: Files sharing the same filename stem in the same directory are grouped into one Asset during import. RAW files take priority as the primary variant (defining asset identity and EXIF data). Additional media files become extra variants on the same asset.
- **Recipe handling**: Processing sidecars (`.xmp`, `.cos`, `.cot`, `.cop`, `.pp3`, `.dop`, `.on1`) are attached as Recipe records to the primary variant rather than imported as standalone assets.
- **XMP metadata extraction**: When an `.xmp` sidecar is attached as a recipe, its contents are parsed (via `xmp_reader` module) and merged into the asset/variant. Keywords (`dc:subject`) become asset tags (deduplicated), `dc:description` sets the asset description (if not already set), and `xmp:Rating`, `xmp:Label`, `dc:creator`, `dc:rights` are stored in the primary variant's `source_metadata`. EXIF takes precedence for any overlapping `source_metadata` keys.
- **Duplicate location tracking**: When a file's content hash already exists, the new file location is added to the existing variant rather than silently skipping. Only truly skips when the exact same location (volume + path) is already recorded.
- **Preview generation**: During import, 800px JPEG thumbnails are generated for each variant. Standard image formats use the `image` crate; RAW files use `dcraw` or `dcraw_emu` (LibRaw); videos use `ffmpeg`. Previews are stored in `previews/<hash-prefix>/<hash>.jpg`. Missing external tools are silently skipped — preview failure never blocks import.
- **Show command**: Displays variants, attached recipes, and preview status for an asset.
- **Generate-previews command**: Generates missing previews for all assets, or a specific asset with `--asset`. Supports `--force` to regenerate existing previews.

**Not yet implemented**: `relocate`, `verify`
