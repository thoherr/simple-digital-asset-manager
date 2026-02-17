# Architecture Overview

## System Layers

The system is organized in four layers, from top to bottom:

### 1. Interface Layer
- **CLI** — subcommand-based interface (`dam import`, `dam search`, `dam relocate`, etc.)
- **Web GUI** (optional) — for visual browsing and tagging, served by an embedded HTTP server

### 2. Core Library
- **Asset Service** — orchestrates import, grouping, relocation, deduplication. Main business logic.
- **Content Store** — SHA-256 hashing, deduplication, mapping hash → physical location(s). A file *is* its hash.
- **Metadata Store** — text-based sidecar files (YAML or TOML). Stores tags, descriptions, variant groupings, processing recipes. Human-readable and diffable.
- **Device Registry** — tracks volumes, mount points, online/offline status. Allows referencing files on unmounted media.
- **Query Engine** — searches the local catalog by metadata fields, dates, tags, file types, etc.
- **Preview Generator** — creates thumbnails/previews using external tools (dcraw/libraw, ffmpeg). Caches in local catalog.

### 3. Storage Layer
- **Local Catalog** — always available on local disk. Contains asset index, cached metadata, thumbnails, volume registry. Small compared to originals.
- **Media Volumes** — external/offline drives holding the actual asset files. May be unmounted.

## Key Design Decisions

- **Content-addressable storage**: originals are immutable and identified by SHA-256 hash.
- **Text-based metadata**: sidecar files (YAML/TOML) are the source of truth for all metadata.
- **SQLite as local catalog**: fast queries, single file, no server. Acts as cache/index over the authoritative sidecar files.
- **Offline-capable**: the local catalog holds enough information (index + thumbnails) to browse and search without media being mounted.
- **Duplicate location tracking**: when the same content (same SHA-256) is found at a new file path, the location is added to the existing variant rather than being silently skipped. This preserves knowledge of all physical copies, enables the `duplicates` command, and supports future cleanup/consolidation workflows.
- **Stem-based auto-grouping**: files sharing the same filename stem in the same directory are grouped into one asset during import (e.g. `DSC_4521.NEF` + `DSC_4521.jpg` + `DSC_4521.xmp`). Media files become variants; processing sidecars (XMP, COS, etc.) are attached as recipes. No timestamp matching is required — directory co-location and stem identity are sufficient.

## Technology

- **Language**: Rust
- **Platforms**: macOS, Linux
- **Key crates**: clap (CLI), sha2 (hashing), serde (serialization), rusqlite (SQLite), axum (web GUI), kamadak-exif (EXIF parsing), image (preview generation)
- **External tools**: dcraw/libraw (RAW previews), ffmpeg (video thumbnails)
