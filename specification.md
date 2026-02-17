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
- **`import`** — hash files (SHA-256), extract EXIF metadata, create assets/variants, write YAML sidecars, insert into SQLite catalog
  - Stem-based auto-grouping: files sharing the same filename stem in the same directory are grouped into one asset (e.g. `DSC_4521.nef` + `DSC_4521.jpg` → 1 asset, 2 variants)
  - RAW files take priority as the primary variant (defining asset identity via deterministic UUID and EXIF-based `created_at`)
  - Recipe handling: processing sidecars (`.xmp`, `.cos`, `.cot`, `.cop`, `.pp3`, `.dop`, `.on1`) are attached as Recipe records to the primary variant
  - Duplicate location tracking: re-importing the same content from a different path adds the new location to the existing variant
  - Per-file progress logging with `-l` flag; elapsed timing with `-t` flag
  - Summary only reports non-zero stat categories
- **`search`** — search assets by text, type, tag, or format via SQLite catalog
- **`show`** — display full asset details including variants, locations, source metadata, and recipes
- **`tag`** — add or remove tags on an asset (with `--remove` flag)
- **`group`** — manually group variants into one asset by content hash (merges donor assets, combines tags)
- **`rebuild-catalog`** — drop and rebuild SQLite catalog from YAML sidecar files (including recipes)
- **`duplicates`** — find files with the same content hash across multiple locations, showing all volume/path pairs
- **`generate-previews`** — generate missing preview thumbnails for all assets or a specific asset (`--asset`); `--force` regenerates existing previews
- **Preview generation during import** — 800px JPEG thumbnails are generated for each imported variant. Uses the `image` crate for standard formats, `dcraw`/`dcraw_emu` (LibRaw) for RAW files, and `ffmpeg` for videos. Previews stored in `previews/<hash-prefix>/<hash>.jpg`. Missing external tools are silently skipped; preview failure never blocks import.
- **`show`** now displays preview status (path if exists, "(none)" otherwise)

### not yet implemented

- **`relocate`** — move asset files to another volume
- **`verify`** — check file integrity by re-hashing and comparing
- Web GUI for visual browsing
