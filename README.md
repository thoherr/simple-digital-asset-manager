# dam — Digital Asset Manager

A command-line digital asset manager built in Rust, designed for large collections of images, videos, and other media spread across multiple storage devices.

## Key Features

- **Content-addressable storage** — files identified by SHA-256 hash, enabling deduplication and integrity verification
- **Multi-volume support** — manage assets across external drives, NAS, and offline storage (terabytes scale)
- **Variant grouping** — automatically groups related files (RAW + JPEG + XMP) into a single asset by filename stem
- **Recipe management** — tracks processing sidecars from CaptureOne, Lightroom/XMP, RawTherapee, DxO, and ON1
- **EXIF/XMP extraction** — camera metadata, keywords, ratings, and descriptions extracted at import
- **Preview generation** — thumbnails for images (via `image` crate), RAW files (via dcraw/LibRaw), videos (via ffmpeg), and info cards for audio/documents
- **Integrity verification** — detect bit rot and corruption by re-hashing files against stored checksums
- **Asset relocation** — copy or move assets between volumes with integrity verification
- **Flexible output** — JSON output on all commands, custom format templates for scripting
- **Web UI** — browser-based interface for browsing, searching, and editing assets

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
| `dam search <query>` | Search by text, tags, type, format, rating, camera, and more |
| `dam show <id>` | Display full asset details |
| `dam tag <id> <tags...>` | Add or remove tags |
| `dam group <hashes...>` | Manually group variants into one asset |
| `dam duplicates` | Find files with identical content across locations |
| `dam generate-previews` | Generate or regenerate preview thumbnails |
| `dam relocate <id> <volume>` | Copy or move an asset to another volume |
| `dam verify` | Re-hash files to detect corruption |
| `dam stats` | Show catalog statistics |
| `dam rebuild-catalog` | Rebuild SQLite index from sidecar files |
| `dam serve` | Start the web UI server |

All commands support `--json` for machine-readable output. See `dam --help` and `dam <command> --help` for full usage.

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
