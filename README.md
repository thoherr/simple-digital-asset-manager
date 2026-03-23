<p align="center">
  <img src="doc/manual/maki-wordmark-tagline.png" alt="MAKI — Media Asset Keeper & Indexer" width="400">
</p>

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
- **AI auto-tagging** — zero-shot image classification using SigLIP vision-language models (ViT-B/16-256 or ViT-L/16-256) for automated tag suggestions, visual similarity search via stored embeddings, and natural language image search via `text:` filter (*MAKI Pro*)
- **Face recognition** — detect faces with YuNet, generate ArcFace embeddings, auto-cluster into people groups, and manage named people across your catalog (*MAKI Pro*)
- **Interactive shell** — `maki shell` REPL with named variables (`$picks = search "rating:5"`), tab completion, session defaults, `.maki` script files, and `source` for script composition
- **Web UI** — browser-based interface with search, inline editing, batch operations, keyboard navigation, lightbox viewer, dark mode, grid density controls, calendar heatmap, faceted sidebar, visual similarity stroll page, and OS integration (reveal in Finder, open terminal)
- **Flexible output** — JSON on all commands, custom format templates, quiet mode for scripting

## Quick Start

```
cargo build --release

# Initialize a catalog
maki init

# Register a storage volume
maki volume add "Photos 2024" /Volumes/PhotosDrive

# Import files
maki import /Volumes/PhotosDrive/Photos/

# Search and browse
maki search "tag:landscape rating:4+"
maki stats --all

# Start the web UI
maki serve
# Open http://127.0.0.1:8080

# Or use the interactive shell
maki shell
# photos> $picks = search "rating:5 date:2024"
# photos [picks=38]> export --target /tmp/best $picks
```

## Commands

39 commands covering setup, import, search, editing, maintenance, and more:

`init` · `volume add/list/combine/remove` · `import` · `delete` · `export` · `contact-sheet` · `describe` · `search` · `show` · `preview` · `edit` · `tag` · `group` · `split` · `auto-group` · `auto-tag` · `embed` · `faces` · `stack` · `duplicates` · `dedup` · `generate-previews` · `relocate` · `verify` · `sync` · `sync-metadata` · `refresh` · `cleanup` · `writeback` · `stats` · `backup-status` · `fix-roles` · `fix-dates` · `rebuild-catalog` · `migrate` · `saved-search` · `collection` · `serve` · `shell`

**Global flags**: `--json`, `--log`, `--verbose`, `--debug`, `--time`. Run `maki --help` or `maki <command> --help` for usage.

See the [Command Reference](doc/manual/reference/01-setup-commands.md) for detailed documentation of every command, or the [Search Filters Reference](doc/manual/reference/06-search-filters.md) for the 20+ filter types available in `maki search`.

## Architecture

The system uses a two-tier storage model:

- **YAML sidecar files** are the source of truth for all metadata (human-readable, diffable, never lost)
- **SQLite catalog** is a derived index for fast queries (rebuildable from sidecars via `maki rebuild-catalog`)

Assets live on **media volumes** (external drives, NAS) while the catalog stays local with enough data (index + thumbnails) to browse without media mounted.

See [`doc/architecture-overview.md`](doc/architecture-overview.md) for the system design and [`doc/component-specification.md`](doc/component-specification.md) for detailed component specs.

## Documentation

The full **[User Manual](doc/manual/index.md)** covers:

- **[User Guide](doc/manual/user-guide/01-overview.md)** — workflow-oriented guides from setup through maintenance
- **[Reference Guide](doc/manual/reference/00-cli-conventions.md)** — man-page style docs for every command, filter, and config option
- **[Developer Guide](doc/manual/developer/01-rest-api.md)** — REST API, module reference, and build/test instructions

Configuration is documented in the [Configuration Reference](doc/manual/reference/08-configuration.md). All settings live in `maki.toml` at the catalog root; every field is optional with sensible defaults.

## External Tools (Highly Recommended)

- **dcraw** or **LibRaw** (dcraw_emu) — RAW file preview extraction
- **ffmpeg** — video thumbnail extraction
- **curl** — model file download for AI auto-tagging (only needed with `--features ai`) and VLM image descriptions (`maki describe`)

Install on macOS: `brew install libraw ffmpeg curl`
Install on Linux: `sudo apt install libraw-bin ffmpeg curl` (Debian/Ubuntu)
Install on Windows: `winget install LibRaw.LibRaw Gyan.FFmpeg cURL.cURL` or `scoop install libraw ffmpeg curl`

When missing, RAW and video files get an info card preview instead. maki prints a warning on first use when a tool is not found.

## AI Auto-Tagging (MAKI Pro)

Download the MAKI Pro binary or build with `cargo build --features ai` to enable AI-powered commands. This uses SigLIP vision-language models (via ONNX Runtime) for zero-shot image classification against a configurable tag vocabulary. Two models are available: ViT-B/16-256 (~207 MB, default) and ViT-L/16-256 (~670 MB, higher accuracy). Select with `--model` or `[ai] model` in `maki.toml`. Model files are downloaded from HuggingFace on first use. Commands: `maki auto-tag` for tag suggestion/application, `maki embed` for batch embedding generation, `maki search "similar:<id>"` for visual similarity search, and `maki search "text:\"sunset on the beach\""` for natural language image search. The web UI includes a **Stroll page** (`/stroll`) for graph-based visual exploration — pick an asset, see its nearest visual neighbors arranged radially, click through to explore connections. The "Suggest tags" and "Auto-tag" buttons also store embeddings opportunistically. Similarity search uses an in-memory index for sub-millisecond results at any scale. See the [Configuration Reference](doc/manual/reference/08-configuration.md) for `[ai]` settings.

**GPU acceleration** (macOS): Build with `cargo build --features ai-gpu` to enable CoreML execution provider for hardware-accelerated inference on Apple Silicon (Neural Engine) and Intel Macs (Metal). Falls back to CPU automatically when CoreML is unavailable. Configure via `[ai] execution_provider` in `maki.toml` (`"auto"`, `"cpu"`, `"coreml"`).

## Face Recognition (MAKI Pro)

Download the MAKI Pro binary or build with `cargo build --features ai` to enable face detection and people management. Uses two ONNX models: YuNet for face detection (bounding boxes + landmarks) and ArcFace for face recognition (512-dim embeddings). Models are downloaded via `maki faces download`.

**CLI workflow**: detect faces → cluster into groups → name people:

```
maki faces download                                    # download YuNet + ArcFace models
maki faces detect --query "type:image" --apply         # detect faces in images
maki faces cluster --apply                             # group similar faces into people
maki faces people                                      # list unnamed person groups
maki faces name <person-id> "Alice"                    # name a person
```

**Web UI**: `/people` page with person gallery, asset detail face chips with assign/unassign, browse filter by `faces:` and `person:` filters, batch face detection from the browse toolbar.

**Data persistence**: Face records, people, and embeddings are stored in both SQLite (for queries) and files (YAML + binary) for rebuild resilience. `maki faces export` migrates existing SQLite data to files; `maki embed --export` does the same for image similarity embeddings.

**Search filters**: `faces:any` / `faces:none` / `faces:N` / `faces:N+` (face count), `person:<name>` (assigned person). **Config**: `[ai] face_cluster_threshold` (default 0.5), `[ai] face_min_confidence` (default 0.5).

## VLM Image Descriptions

Generate natural language descriptions and AI-suggested tags using a local vision-language model. Works with any OpenAI-compatible API server (Ollama, LM Studio, vLLM) — no special build features required.

```
ollama pull qwen2.5vl:3b                              # download a VLM
maki describe "description:none" --apply                 # describe undescribed assets
maki describe "date:2024-06" --mode tags --apply          # suggest tags via VLM
maki describe --mode both --volume "Photos" --apply       # both in one pass
maki import --describe /Volumes/Photos/NewShoot/        # auto-describe during import
```

**Web UI**: "Describe" button on asset detail page, batch "Describe" in browse toolbar. VLM availability is detected at server startup.

**Auto-describe on import**: `maki import --describe` generates descriptions for newly imported assets. Enable permanently with `[import] descriptions = true` in `maki.toml`. Silently skips if VLM endpoint is not available.

**Config**: `[vlm]` section in `maki.toml` — endpoint, model, max_tokens, temperature, timeout, mode, prompt, concurrency. CLI flags override config. Set `concurrency` to process multiple assets in parallel during batch describe. See the [Configuration Reference](doc/manual/reference/08-configuration.md) for `[vlm]` settings.

## Technology

Rust, SQLite, clap, axum, askama, htmx. See [`Cargo.toml`](Cargo.toml) for the full dependency list.

## Requirements

- Rust 2021 edition (stable)
- macOS, Linux, or Windows

## License

Licensed under the [Apache License, Version 2.0](LICENSE). See [NOTICE](NOTICE) for attribution.
