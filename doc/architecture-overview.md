# Architecture Overview

## System Layers

The system is organized in four layers, from top to bottom:

### 1. Interface Layer
- **CLI** — subcommand-based interface (`maki import`, `maki search`, `maki relocate`, etc.)
- **Interactive Shell** — `maki shell` provides a readline-based REPL with cached catalog state, named variables (`$name = search ...`), session defaults (`set --json`), tab completion, `.maki` script files, and `source` for script composition. Shares the same command dispatcher as the CLI; each command opens a fresh SQLite connection (same per-request pattern as the web server).
- **Web UI** — browser-based interface via `maki serve`. Uses axum (HTTP), askama (templates), htmx (interactivity). Opens fresh SQLite connections per request via `spawn_blocking`. Serves preview images from the catalog's `previews/` directory. Integrates with the local OS for file management (reveal in Finder, open terminal).

### 2. Core Library
- **Asset Service** — orchestrates import, deletion, grouping, relocation, verification, deduplication, role fixing, face detection, auto-tagging, VLM description, and video metadata extraction. Main business logic layer shared by both CLI and web UI to ensure identical behavior.
- **Content Store** — SHA-256 hashing, deduplication, mapping hash → physical location(s). A file *is* its hash.
- **Metadata Store** — text-based sidecar files (YAML). Stores tags, descriptions, variant groupings, processing recipes. Human-readable and diffable.
- **Device Registry** — tracks volumes, mount points, online/offline status. Allows referencing files on unmounted media.
- **Query Engine** — searches the local catalog and performs metadata editing (tags, rating, color label, grouping, auto-grouping).
- **Preview Generator** — creates thumbnails/previews using external tools (dcraw/libraw, ffmpeg). Caches in local catalog.
- **Format Module** — template engine for flexible CLI output (presets, custom templates, JSON serialization).
- **Config Module** — parses `maki.toml` configuration (preview settings, serve settings, import exclusions/auto-tags).
- **EXIF Reader** — extracts EXIF metadata from image files (camera, lens, ISO, focal length, aperture, dimensions, dates).
- **XMP Reader** — extracts and writes back XMP metadata (keywords, rating, description, color label) for bidirectional sync with CaptureOne/Lightroom.
- **Collection Store** — manages static album collections (dual storage: SQLite for queries + YAML for rebuild persistence).
- **Saved Search Store** — manages named search queries (stored in TOML).
- **Stack Store** — manages asset stacks / scene groupings (dual storage: SQLite `stacks` table + `stacks.yaml` for rebuild persistence). Stacks collapse multiple assets into a single pick in the browse grid.
- **Face Detection Service** *(feature-gated: `--features ai`)* — detects faces in asset images using YuNet ONNX model (outputs bbox + 5-point landmarks), aligns each face to a canonical 112×112 template via a least-squares similarity transform on the 5 landmarks, then computes 512-dim ArcFace ResNet-100 FP32 embeddings. Generates face crop thumbnails for the UI. Multi-stride output decoder for YuNet model variants.
- **Face Store** *(feature-gated: `--features ai`)* — Dual persistence for detected faces and named people: SQLite tables (`faces`, `people`) for fast queries, plus `faces.yaml`/`people.yaml` and ArcFace binary embeddings (`embeddings/arcface/`) for rebuild resilience. Each face row records the `recognition_model` that produced its embedding so model upgrades can detect and skip incompatible embeddings instead of mixing vector spaces. Agglomerative hierarchical clustering (average linkage / UPGMA) for auto-grouping similar faces — order-independent and produces tighter clusters than the greedy single-linkage previously used. Denormalized `face_count` on assets table for fast filtering.
- **Embedding Store** *(feature-gated: `--features ai`)* — Dual persistence for image embeddings: SQLite `embeddings` table for queries + binary files under `embeddings/<model>/` for rebuild resilience. In-memory `EmbeddingIndex` for fast similarity search, powering the `similar:` search filter, the stroll page's graph-based visual exploration, and the "Find similar" web UI feature.

### 3. Storage Layer
- **Local Catalog** — always available on local disk. Contains asset index, cached metadata, thumbnails, volume registry, collection and stack membership, face/people data, and image embeddings. Small compared to originals.
- **Media Volumes** — external/offline drives holding the actual asset files. May be unmounted.

## Key Design Decisions

- **Content-addressable storage**: originals are immutable and identified by SHA-256 hash.
- **Text-based metadata**: sidecar files (YAML) are the source of truth for all metadata.
- **SQLite as local catalog**: fast queries, single file, no server. Acts as cache/index over the authoritative sidecar files. All connections use WAL mode, memory-mapped I/O (256 MB), and tuned pragmas for optimal read performance. A `schema_version` table tracks the current schema; commands verify it at startup with a single fast query and exit with an error if outdated. Only `maki init` and `maki migrate` modify the schema.
- **Offline-capable**: the local catalog holds enough information (index + thumbnails) to browse and search without media being mounted.
- **Duplicate location tracking**: when the same content (same SHA-256) is found at a new file path, the location is added to the existing variant rather than being silently skipped. This preserves knowledge of all physical copies, enables the `duplicates` command, and supports future cleanup/consolidation workflows.
- **Stem-based auto-grouping**: files sharing the same filename stem in the same directory are grouped into one asset during import (e.g. `DSC_4521.NEF` + `DSC_4521.jpg` + `DSC_4521.xmp`). Media files become variants; processing sidecars (XMP, COS, etc.) are attached as recipes. No timestamp matching is required — directory co-location and stem identity are sufficient. For cross-directory grouping (e.g., CaptureOne exports landing in a different directory), the `auto-group` command uses fuzzy prefix + separator matching to find related assets by filename stem across the entire catalog or a scoped search.
- **Location-based recipe identity**: recipes are identified by their location `(volume_id, relative_path)` rather than content hash, because recipe files are routinely edited by external software. Re-importing or verifying a modified recipe updates it in place rather than creating duplicates or reporting failures.
- **Bidirectional XMP sync**: rating, tags, description, and color label changes are written back to `.xmp` sidecar files on disk, enabling round-trip editing with tools like CaptureOne and Lightroom. XMP metadata (`xmp:Rating`, `xmp:Label`, `dc:subject`, `dc:description`) is extracted during import and promoted to first-class Asset fields.
- **Denormalized query columns**: the `assets` table caches `best_variant_hash`, `primary_variant_format`, and `variant_count` computed at write time. This allows the browse grid to join directly to the best variant (one row per asset, no GROUP BY) and display the identity format and variant count without expensive per-query computation. When no variant-level filters are active, the `variants` table is not joined at all.
- **Scriptable output**: all commands support `--json` for machine-readable output. Listing commands (`search`, `duplicates`) additionally support `--format` with presets and custom templates for flexible pipeline integration.

## Technology

- **Language**: Rust
- **Platforms**: macOS, Linux, Windows
- **Key crates**: clap (CLI), sha2 (hashing), serde (serialization), rusqlite (SQLite), kamadak-exif (EXIF parsing), quick-xml (XMP parsing), image (preview generation), imageproc/ab_glyph (info card text rendering), lofty (audio metadata), uuid (asset identity), axum (web server), askama (templates), tokio (async runtime), tower-http (static file serving)
- **External tools**: dcraw/libraw (RAW previews), ffmpeg (video thumbnails)
