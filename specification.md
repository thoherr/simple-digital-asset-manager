# simple digital asset manager

## mandatory requirements

- suitable for all kinds of digital assets, especially images and videos
- all formats of RAW files must be supported (e.g. NEF, RAF etc.)
- multiple variants of the same asset must be grouped and/or navigatable (e.g. RAW / JPEG, different processing etc.)
- media files can be stored on one or more offline devices (we are talking about terrabytes)
- duplicates should be stored only once
- original files (and maybe also variants?) can move (e.g. on different storage device) transparently
- processing instructions / recipies etc. should be managed as well. This should include software like CaptureOne, Photoshop etc.

## basic technical ideas

- an original file (most of the time this is the RAW file) is never changed, so we can make it content adressable (e.g. SHA)
- metadata stored in sidecar files (how can we link media and sidecar)?
- navigation / retrieval independent of location of media files
- all should be text based
- should we use git as backend (but probably only the storage part)?

## implementation status

### implemented

- **`init`** — initialize a new catalog in the current directory (creates metadata dir, SQLite schema, volume registry, config)
- **`volume add/list`** — register storage volumes and list them with online/offline status
- **`import`** — hash files (SHA-256), extract EXIF metadata, create assets/variants, write YAML sidecars, insert into SQLite catalog. `dam import <paths...> [--volume V] [--include G] [--skip G]`
  - Stem-based auto-grouping: files sharing the same filename stem in the same directory are grouped into one asset (e.g. `DSC_4521.nef` + `DSC_4521.jpg` → 1 asset, 2 variants)
  - RAW files take priority as the primary variant (defining asset identity via deterministic UUID and EXIF-based `created_at`)
  - Recipe handling: processing sidecars (`.xmp`, `.cos`, `.cot`, `.cop`, `.pp3`, `.dop`, `.on1`) are attached as Recipe records to the primary variant. Recipes are identified by location (volume + path), not content hash — re-importing after external edits updates the recipe in place and re-extracts XMP metadata. Standalone recipe imports (no co-located media) resolve to parent variants by stem + directory matching
  - Duplicate location tracking: re-importing the same content from a different path adds the new location to the existing variant
  - `--volume` overrides auto-detection of which volume the files belong to
  - Per-file progress logging with `-l` flag; elapsed timing with `-t` flag
  - Summary only reports non-zero stat categories
- **`search`** — search assets by text, type, tag, or format via SQLite catalog
- **`show`** — display full asset details including variants, locations, source metadata, and recipes
- **`tag`** — add or remove tags on an asset (with `--remove` flag)
- **`group`** — manually group variants into one asset by content hash (merges donor assets, combines tags)
- **`rebuild-catalog`** — drop and rebuild SQLite catalog from YAML sidecar files (including recipes)
- **`duplicates`** — find files with the same content hash across multiple locations, showing all volume/path pairs
- **`generate-previews`** — generate missing preview thumbnails. Supports `PATHS` (resolve files on disk), `--asset`, `--volume`, `--include`/`--skip` (file type groups), and `--force` (regenerate existing)
- **Preview generation during import** — previews are generated for each imported variant. Uses the `image` crate for standard formats (800px JPEG thumbnails), `dcraw`/`dcraw_emu` (LibRaw) for RAW files, and `ffmpeg` for videos. Non-visual formats (audio, documents, unknown) get an info card — an 800x600 JPEG showing file metadata (name, format, size, and audio properties like duration/bitrate via `lofty`). When external tools (dcraw, ffmpeg) are missing, RAW and video files also fall back to an info card. Previews stored in `previews/<hash-prefix>/<hash>.jpg`. Preview failure never blocks import.
- **`show`** now displays preview status (path if exists, "(none)" otherwise)
- **`relocate`** — copy or move all asset files (variants + recipes) to a target volume: `dam relocate <asset-id> <target-volume> [--remove-source] [--dry-run]`. Copies files with SHA-256 integrity verification, preserves relative paths, updates sidecar and catalog metadata. Without `--remove-source`, the asset gains additional locations. With `--remove-source`, source files are deleted after verified copy. `--dry-run` shows the plan without making changes.
- **`verify`** — re-hash files on disk and compare against stored content hashes to detect corruption or bit rot: `dam verify [PATHS...] [--volume <label>] [--asset <id>]`. Without arguments, verifies all file locations on all online volumes. With paths, verifies specific files or directories. `--volume` limits to a specific volume; `--asset` limits to a specific asset. Updates `verified_at` timestamps on successful verification. Exits with code 1 if any mismatches are found. Modified recipe files are reported as "modified" (not "FAILED") and do not trigger exit code 1 — their stored hash is updated to reflect the new content.
- **Output formatting** — flexible output for scripting and machine consumption:
  - Global `--json` flag on all commands: outputs structured JSON to stdout, human messages to stderr
  - Global `--debug` / `-d` flag: shows stderr output from external tools (ffmpeg, dcraw, dcraw_emu) for diagnosing preview generation issues
  - `search --format=<preset|template>`: presets are `ids` (one UUID per line), `short` (default compact), `full` (with tags/description), `json` (JSON array). Custom templates use `{placeholder}` syntax, e.g. `'{id}\t{name}\t{tags}'`. Supported placeholders: `id`, `short_id`, `name`, `filename`, `type`, `format`, `date`, `tags`, `description`, `hash`
  - `search -q` / `--quiet`: shorthand for `--format=ids`
  - `duplicates --format=<preset|template>`: same presets, with additional `{locations}` placeholder
  - When `--format` is explicitly set, result counts are suppressed
- **`stats`** — show catalog statistics: `dam stats [--types] [--volumes] [--tags] [--verified] [--all] [--limit N]`. Without flags, shows overview (assets, variants, recipes, volumes, total size). `--types` adds asset type breakdown with percentages and top variant/recipe formats. `--volumes` adds per-volume details (asset/variant/recipe counts, size, directories, formats, verification coverage). `--tags` shows unique tag count, tagged/untagged assets, and top tags by frequency. `--verified` shows verification health (coverage, oldest/newest timestamps, per-volume breakdown). `--all` enables all sections. `--limit N` controls top-N lists (default 20). Supports `--json` for structured output.

- **`serve`** — start web UI server: `dam serve [--port <port>] [--bind <addr>]`. Default `127.0.0.1:8080`. Browse/search page with filter dropdowns (type, tag, format, volume, rating), sort options, pagination (60 per page), and thumbnail grid with star ratings. Asset detail page with preview, metadata table, inline tag editing (add/remove via htmx), inline star rating (click to set/clear via htmx PUT), variants table, recipes list, and collapsible source metadata. Rating filter supports minimum threshold (e.g. 3+) and exact match. Uses axum + askama templates + htmx for partial page updates. Static assets embedded at compile time.
- **Rating** — first-class `Option<u8>` field on Asset, persisted in sidecar YAML and SQLite. Extracted from XMP `xmp:Rating` during import (conservative merge on first import, overwrite on re-import). Searchable via CLI `rating:N` (exact) and `rating:N+` (minimum). Displayed as stars in `dam show` output. Editable via web UI inline star rating with htmx. `PUT /api/asset/{id}/rating` endpoint accepts `rating=N` (1-5) or `rating=0` to clear.

### not yet implemented
