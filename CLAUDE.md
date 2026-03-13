# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

A digital asset manager designed for large collections of images and videos (terabytes across multiple offline storage devices). Content-addressable storage, text-based metadata in sidecar files, variant grouping, deduplication, transparent file relocation, recipe management, and location-independent navigation.

## Technology

- **Language**: Rust
- **Platforms**: macOS, Linux, Windows
- **Interface**: CLI-first (`dam` command), web UI (`dam serve`)
- **Catalog**: SQLite (cache/index), YAML sidecar files (source of truth)
- **Key crates**: clap, sha2, serde, rusqlite, kamadak-exif, quick-xml, regex, image, imageproc, ab_glyph, lofty, uuid, axum, askama, tokio, tower-http, toml, glob-match; **optional (ai feature)**: ort, ndarray, tokenizers; **optional (ai-gpu feature)**: ort with CoreML execution provider
- **External tools**: dcraw/libraw (RAW previews), ffmpeg (video thumbnails), curl (AI model download, VLM calls)

## Architecture

Core layers: CLI → Core Library (Asset Service, Content Store, Metadata Store, Device Registry, Query Engine, Preview Generator) → Storage (Local Catalog + Media Volumes).

See `doc/architecture-overview.md` for design decisions, `doc/component-specification.md` for data model, components, routes, and directory structure.

## Implemented Commands

38 commands: `init`, `volume add/list/combine/remove`, `import`, `delete`, `export`, `contact-sheet`, `describe`, `search`, `show`, `tag`, `edit`, `group`, `split`, `auto-group`, `auto-tag`, `embed`, `faces`, `stack`, `duplicates`, `dedup`, `generate-previews`, `fix-roles`, `fix-dates`, `fix-recipes`, `rebuild-catalog`, `migrate`, `relocate`, `update-location`, `verify`, `sync`, `sync-metadata`, `refresh`, `cleanup`, `writeback`, `backup-status`, `stats`, `serve`, `saved-search`, `collection`, `shell`

See `doc/specification.md` for detailed command behavior, flags, and search filter documentation.

## Key Patterns

### Dual Storage
YAML sidecars are the source of truth; SQLite catalog is a derived cache rebuilt via `dam rebuild-catalog`. All write paths must update both stores.

### Schema Migrations
- **Version-guarded**: `run_migrations()` reads the stored version once and only executes blocks where `current < N`. On an up-to-date catalog, startup is a single SELECT query.
- Idempotent: `let _ = conn.execute_batch("ALTER TABLE ... ADD COLUMN ...")` — silently ignores if column exists
- Backfill with `WHERE column IS NULL` guard
- `schema_version` table with `SCHEMA_VERSION` constant in `catalog.rs`. All commands (except `init`/`migrate`) check version at startup; exit with error if outdated
- `initialize()` creates base tables then delegates to `run_migrations()` for all columns, indexes, backfills, and version stamping
- Bump `SCHEMA_VERSION` whenever `run_migrations()` changes; add a new `if current < N` block

### Denormalized Columns on `assets` Table
`best_variant_hash`, `primary_variant_format`, `variant_count`, `face_count`, `stack_id`, `stack_position`, `latitude`, `longitude` — computed at write time, must be updated in all write paths (`insert_asset`, `update_denormalized_variant_columns`, `fix_roles`, `StackStore` operations, `FaceStore::update_face_count`).

### SQLite Connection Pool (Web Server)
`CatalogPool` holds pre-opened connections (RAII `PooledCatalog` returned on drop). `Catalog::open_fast()` (pragmas only, no migrations) via `spawn_blocking`. Schema migrations run once at startup.

### Conditional JOINs in Search
`build_search_where()` returns `(where, params, needs_fl_join, needs_v_join)` — variant/location tables only joined when filters need them.

### XMP Write-Back
Rating, tag, description, and label changes are written back to `.xmp` recipe files on disk. After writing, the file is re-hashed and the recipe's `content_hash` is updated in both SQLite and YAML sidecar. Offline volumes are silently skipped with `pending_writeback=true` flag. `dam writeback` replays pending writes when volumes come online.

### Feature Gates
- `--features ai`: SigLIP embeddings, auto-tag, face detection/recognition, stroll page, similarity search, text-to-image search
- `--features ai-gpu`: CoreML execution provider on macOS (additive to `ai`)
- VLM (`dam describe`): no feature gate — HTTP calls to external server

### Output Formatting Conventions
- All commands support `--json` (structured JSON to stdout, messages to stderr)
- Multi-file commands support `--log` (per-file progress to stderr)
- `--debug` shows external tool stderr; `--time` shows elapsed time
- `search --format` supports presets (`ids`, `short`, `full`, `json`) and custom templates (`'{id}\t{name}'`)

### Test Helpers
`setup_search_catalog()` and `setup_metadata_catalog()` require `asset.variants` populated before `catalog.insert_asset()` (because of denormalized columns).

## Configuration (`dam.toml`)

All sections optional; missing fields use defaults. Sections: `[preview]` (max_edge, format, quality, smart_max_edge, smart_quality, generate_on_demand), `[serve]` (port, bind, per_page, stroll_neighbors, stroll_fanout, stroll_discover_pool), `[import]` (exclude, auto_tags, smart_previews, embeddings, descriptions), `[dedup]` (prefer), `[verify]` (max_age_days), `[ai]` (model, threshold, labels, model_dir, prompt, execution_provider, face_cluster_threshold, face_min_confidence), `[vlm]` (endpoint, model, max_tokens, prompt, timeout, temperature, mode, concurrency), `[contact_sheet]` (layout, paper, landscape, title, fields, sort, group_by, label_style, copyright, margin, quality). Config structs: `CatalogConfig` in `src/config.rs` with sub-structs.

See `doc/manual/reference/08-configuration.md` for full documentation of every option.

## Documentation References

| Topic | Document |
|-------|----------|
| Feature spec & command behavior | `doc/specification.md` |
| Architecture & design decisions | `doc/architecture-overview.md` |
| Data model, components, routes | `doc/component-specification.md` |
| User manual (guides + reference) | `doc/manual/index.md` |
| Search filters reference | `doc/manual/reference/06-search-filters.md` |
| Configuration reference | `doc/manual/reference/08-configuration.md` |
| Roadmap & proposals | `doc/proposals/roadmap.md` |
