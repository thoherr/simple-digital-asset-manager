# dam — Digital Asset Manager

A command-line digital asset manager built in Rust, designed for large collections of images, videos, and other media spread across multiple storage devices.

## Key Features

- **Content-addressable storage** — files identified by SHA-256 hash, enabling deduplication and integrity verification
- **Multi-volume support** — manage assets across external drives, NAS, and offline storage (terabytes scale)
- **Variant grouping** — automatically groups related files (RAW + JPEG + XMP) into a single asset by filename stem
- **Recipe management** — tracks processing sidecars from CaptureOne, Lightroom/XMP, RawTherapee, DxO, and ON1
- **EXIF/XMP extraction** — camera metadata, keywords, ratings, color labels, and descriptions extracted at import
- **Preview generation** — thumbnails for images (via `image` crate), RAW files (via dcraw/LibRaw), videos (via ffmpeg), and info cards for audio/documents
- **Integrity verification** — detect bit rot and corruption by re-hashing files against stored checksums
- **Asset relocation** — copy or move assets between volumes with integrity verification
- **Saved searches & collections** — save named search queries (smart albums) and curate static asset lists (collections)
- **Flexible output** — JSON output on all commands, custom format templates for scripting
- **Web UI** — browser-based interface for browsing, searching, and editing assets with saved search chips, collection filter dropdown, and collection management

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
dam search "tag:landscape"
dam stats --all

# Start the web UI
dam serve
# Open http://127.0.0.1:8080
```

## Commands

| Command | Description |
|---------|-------------|
| `dam init` | Initialize a new catalog in the current directory |
| `dam volume add/list` | Register and list storage volumes |
| `dam import <paths...>` | Import files with auto-grouping and metadata extraction |
| `dam search <query>` | Search by text, tags, type, format, rating, camera, location health, and more |
| `dam show <id>` | Display full asset details |
| `dam edit <id> [flags]` | Edit name, description, rating, color label |
| `dam tag <id> <tags...>` | Add or remove tags |
| `dam group <hashes...>` | Manually group variants into one asset |
| `dam auto-group [query]` | Auto-group assets by filename stem (fuzzy prefix matching) |
| `dam duplicates` | Find files with identical content across locations |
| `dam generate-previews` | Generate or regenerate preview thumbnails |
| `dam relocate <id> <volume>` | Copy or move an asset to another volume |
| `dam verify` | Re-hash files to detect corruption |
| `dam sync <paths...>` | Reconcile catalog with moved/modified/missing files |
| `dam refresh` | Re-read metadata from changed sidecar/recipe files |
| `dam cleanup` | Remove stale locations, orphaned assets, and orphaned previews |
| `dam stats` | Show catalog statistics |
| `dam fix-roles` | Fix variant roles in RAW+non-RAW asset groups |
| `dam rebuild-catalog` | Rebuild SQLite index from sidecar files |
| `dam saved-search` (alias `ss`) | Save, list, run, and delete named searches |
| `dam collection` (alias `col`) | Create, list, show, add to, remove from, and delete collections |
| `dam serve` | Start the web UI server |

**Global flags**: `--json` (machine-readable output), `-l`/`--log` (per-file progress for import, verify, sync, refresh, cleanup, generate-previews), `-d`/`--debug` (external tool stderr), `-t`/`--time` (elapsed time). See `dam --help` and `dam <command> --help` for full usage.

### Search filters

`dam search` supports prefix filters that can be combined freely:

| Filter | Example | Description |
|--------|---------|-------------|
| `type:` | `type:image` | Asset type (image, video, audio, document, other) |
| `tag:` | `tag:landscape` | Tag name |
| `format:` | `format:jpg` | File format |
| `rating:` | `rating:3+` or `rating:5` | Minimum or exact rating |
| `label:` | `label:Red` | Color label (Red, Orange, Yellow, Green, Blue, Pink, Purple) |
| `camera:` | `camera:fuji` | Camera model (partial match) |
| `lens:` | `lens:56mm` | Lens model (partial match) |
| `iso:` | `iso:100-800` | ISO range, exact, or minimum |
| `focal:` | `focal:35-70` | Focal length range |
| `f:` | `f:1.4-2.8` | Aperture range |
| `width:` | `width:4000+` | Minimum image width |
| `height:` | `height:2000+` | Minimum image height |
| `meta:` | `meta:label=Red` | Source metadata key=value |
| `orphan:true` | `orphan:true` | Assets with no file locations |
| `missing:true` | `missing:true` | Assets with files missing from disk |
| `stale:` | `stale:30` | Locations not verified in N days |
| `volume:none` | `volume:none` | Assets with no locations on online volumes |
| `collection:` | `collection:"My Favorites"` | Assets in a collection |
| `path:` | `path:Capture/2026-02-22` | Assets with files under a path prefix |

Remaining tokens are free-text search across name, filename, description, and metadata. Values with spaces can be quoted: `tag:"Fools Theater"`, `path:"Photos/Family Trip"`.

## Configuration

All settings live in `dam.toml` at the catalog root (created by `dam init`). Every section and field is optional — an empty file or a missing section uses the defaults shown below.

```toml
# Default volume for import when auto-detection is ambiguous
# default_volume = "550e8400-e29b-41d4-a716-446655440000"

[preview]
max_edge = 800        # Maximum pixel size of the longest edge (default: 800)
format = "jpeg"       # Preview format: "jpeg" or "webp" (default: "jpeg")
quality = 85          # JPEG quality 1–100 (default: 85; ignored for webp)

[serve]
port = 8080           # Web UI port (default: 8080, overridden by --port)
bind = "127.0.0.1"    # Web UI bind address (default: "127.0.0.1", overridden by --bind)

[import]
exclude = [           # Filename patterns to skip during import (glob syntax)
  "Thumbs.db",
  ".DS_Store",
  "*.tmp",
]
auto_tags = [         # Tags automatically applied to every newly imported asset
  "inbox",
]
```

**Notes:**
- CLI flags (`--port`, `--bind`) override the corresponding `dam.toml` values.
- `exclude` patterns match filenames only (not paths) using glob syntax (`*`, `?`, `[...]`).
- `auto_tags` are merged with any tags extracted from XMP sidecars (no duplicates).
- Changing `format` or `max_edge` affects newly generated previews; existing previews are not regenerated automatically (use `dam generate-previews --force`).

## Architecture

The system uses a two-tier storage model:

- **YAML sidecar files** are the source of truth for all metadata (human-readable, diffable, never lost)
- **SQLite catalog** is a derived index for fast queries (rebuildable from sidecars via `dam rebuild-catalog`)

Assets live on **media volumes** (external drives, NAS) while the catalog stays local with enough data (index + thumbnails) to browse without media mounted.

See [`doc/architecture-overview.md`](doc/architecture-overview.md) for the system design and [`doc/component-specification.md`](doc/component-specification.md) for detailed component specs.

## Optional External Tools

- **dcraw** or **LibRaw** (dcraw_emu) — RAW file preview extraction
- **ffmpeg** — video thumbnail extraction

These are optional. When missing, RAW and video files get an info card preview instead.

## Technology

Rust, SQLite, clap, axum, askama, htmx. See [`Cargo.toml`](Cargo.toml) for the full dependency list.

## Requirements

- Rust 2021 edition (stable)
- macOS or Linux
