# dam — Digital Asset Manager

A command-line digital asset manager built in Rust, designed for large collections of images, videos, and other media spread across multiple storage devices.

## Key Features

- **Content-addressable storage** — files identified by SHA-256 hash, enabling deduplication and integrity verification
- **Multi-volume support** — manage assets across external drives, NAS, and offline storage (terabytes scale)
- **Variant grouping** — automatically groups related files (RAW + JPEG + XMP) into a single asset by filename stem
- **Recipe management** — tracks processing sidecars from CaptureOne, Lightroom/XMP, RawTherapee, DxO, and ON1
- **EXIF/XMP extraction** — camera metadata, keywords, ratings, color labels, and descriptions extracted at import
- **Bidirectional XMP sync** — rating, tag, description, and label changes written back to `.xmp` recipe files
- **Preview generation** — thumbnails for images, RAW files (via dcraw/LibRaw), videos (via ffmpeg), and info cards for audio/documents
- **Integrity verification** — detect bit rot and corruption by re-hashing files against stored checksums
- **Stacks** — group burst shots and similar-scene images into collapsible stacks, showing only the "pick" in the browse grid
- **Hierarchical tags** — tree-structured keywords with Lightroom `lr:hierarchicalSubject` interop
- **Saved searches & collections** — smart albums (dynamic queries) and static albums (curated lists)
- **Web UI** — browser-based interface with search, inline editing, batch operations, keyboard navigation, lightbox viewer, dark mode, grid density controls, and calendar heatmap
- **Flexible output** — JSON on all commands, custom format templates, quiet mode for scripting

## Quick Start

```
cargo build --release

# Initialize a catalog
dam init

# Register a storage volume
dam volume add "Photos 2024" /Volumes/PhotosDrive

# Import files
dam import /Volumes/PhotosDrive/Photos/

# Search and browse
dam search "tag:landscape rating:4+"
dam stats --all

# Start the web UI
dam serve
# Open http://127.0.0.1:8080
```

## Commands

26 commands covering setup, import, search, editing, maintenance, and more:

`init` · `volume add/list/combine/remove` · `import` · `search` · `show` · `edit` · `tag` · `group` · `auto-group` · `stack` · `duplicates` · `dedup` · `generate-previews` · `relocate` · `verify` · `sync` · `refresh` · `cleanup` · `stats` · `backup-status` · `fix-roles` · `fix-dates` · `rebuild-catalog` · `saved-search` · `collection` · `serve`

**Global flags**: `--json`, `--log`, `--debug`, `--time`. Run `dam --help` or `dam <command> --help` for usage.

See the [Command Reference](doc/manual/reference/01-setup-commands.md) for detailed documentation of every command, or the [Search Filters Reference](doc/manual/reference/06-search-filters.md) for the 20+ filter types available in `dam search`.

## Architecture

The system uses a two-tier storage model:

- **YAML sidecar files** are the source of truth for all metadata (human-readable, diffable, never lost)
- **SQLite catalog** is a derived index for fast queries (rebuildable from sidecars via `dam rebuild-catalog`)

Assets live on **media volumes** (external drives, NAS) while the catalog stays local with enough data (index + thumbnails) to browse without media mounted.

See [`doc/architecture-overview.md`](doc/architecture-overview.md) for the system design and [`doc/component-specification.md`](doc/component-specification.md) for detailed component specs.

## Documentation

The full **[User Manual](doc/manual/index.md)** covers:

- **[User Guide](doc/manual/user-guide/01-overview.md)** — workflow-oriented guides from setup through maintenance
- **[Reference Guide](doc/manual/reference/00-cli-conventions.md)** — man-page style docs for every command, filter, and config option
- **[Developer Guide](doc/manual/developer/01-rest-api.md)** — REST API, module reference, and build/test instructions

Configuration is documented in the [Configuration Reference](doc/manual/reference/08-configuration.md). All settings live in `dam.toml` at the catalog root; every field is optional with sensible defaults.

## Optional External Tools

- **dcraw** or **LibRaw** (dcraw_emu) — RAW file preview extraction
- **ffmpeg** — video thumbnail extraction

These are optional. When missing, RAW and video files get an info card preview instead.

## Technology

Rust, SQLite, clap, axum, askama, htmx. See [`Cargo.toml`](Cargo.toml) for the full dependency list.

## Requirements

- Rust 2021 edition (stable)
- macOS or Linux
