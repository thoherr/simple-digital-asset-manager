# Component Specification

## Data Model

### Asset
The central entity. Represents a logical asset (e.g. "photo of sunset at beach, 2024-07-15").

| Field | Type | Description |
|---|---|---|
| id | UUID | Stable identifier |
| name | String | User-assigned name (optional) |
| created_at | DateTime | When the asset was first imported |
| asset_type | Enum | Image, Video, Audio, Document, Other |
| tags | Vec<String> | User-defined tags |
| description | String | Free-text description (optional) |

An asset groups one or more **variants**.

### Variant
A concrete file belonging to an asset. Multiple variants form a group (e.g. RAW + JPEG + edited TIFF).

| Field | Type | Description |
|---|---|---|
| content_hash | SHA-256 | Primary identity, derived from file content |
| asset_id | UUID | Parent asset |
| role | Enum | Original, Processed, Export, Sidecar |
| format | String | File extension / MIME type |
| file_size | u64 | Size in bytes |
| original_filename | String | Filename at import time |
| source_metadata | Map | EXIF, XMP, and other embedded metadata extracted at import |
| locations | Vec<FileLocation> | Where this variant physically lives |

### FileLocation
A physical location of a variant on a specific volume.

| Field | Type | Description |
|---|---|---|
| volume_id | UUID | Which volume |
| relative_path | PathBuf | Path relative to volume root |
| verified_at | DateTime | Last time hash was verified at this location |

### Volume
A storage device or mount point.

| Field | Type | Description |
|---|---|---|
| id | UUID | Stable identifier |
| label | String | Human-readable name (e.g. "Photos Archive 2024") |
| mount_point | PathBuf | Expected mount path (e.g. /Volumes/PhotosArchive) |
| volume_type | Enum | Local, External, Network |
| is_online | bool | Derived at runtime from mount point availability |

### Recipe
Processing instructions associated with a variant. During import, files with recognized recipe extensions that share a filename stem with a media file in the same directory are automatically attached as recipes rather than imported as variants. Standalone recipe files (imported without a co-located media file) are resolved to their parent variant by matching filename stem and directory.

Known recipe extensions: `.xmp` (Adobe/Lightroom/CaptureOne), `.cos` / `.cot` / `.cop` (CaptureOne session/template/preset), `.pp3` (RawTherapee), `.dop` (DxO), `.on1` (ON1).

| Field | Type | Description |
|---|---|---|
| id | UUID | Stable identifier |
| variant_hash | SHA-256 | Which variant this recipe belongs to |
| software | String | e.g. "CaptureOne 23", "Photoshop 2024" |
| recipe_type | Enum | Sidecar (XMP, COS, etc.), EmbeddedExport |
| content_hash | SHA-256 | Hash of the recipe file itself (mutable — updated when file changes) |
| location | FileLocation | Where the recipe file lives (primary identity for dedup) |
| verified_at | DateTime | Last time hash was verified at this location |

**Design decision — location-based identity**: Recipes are identified by their location `(variant_hash, volume_id, relative_path)` rather than their content hash. This is because recipe files (XMP, COS, etc.) are routinely edited by external software. Re-importing after an external edit updates the recipe in place (new hash, re-extracted XMP metadata) rather than creating a duplicate. During verification, a changed recipe hash is reported as "modified" (not a failure) and the stored hash is updated.

## Components

### 1. Content Store

**Responsibility**: file identity, deduplication, and physical location tracking.

**Operations**:
- `ingest(path) -> SHA-256` — hash a file, register it. If hash already exists, skip copy (dedup).
- `locate(hash) -> Vec<FileLocation>` — find all known locations of a file.
- `relocate(hash, from_volume, to_volume)` — move/copy a file between volumes, update locations.
- `verify(hash, location) -> bool` — re-hash file at location, confirm integrity.
- `remove_location(hash, location)` — unregister a location (file moved/deleted externally).

**Storage layout on a volume**:
```
<volume_root>/
  dam-store/
    ab/cd/abcdef1234...   # files stored by hash prefix (2-level sharding)
```

Alternatively, files can remain in their original directory structure and the content store only tracks their location (non-destructive mode). This is important for interoperability with tools like CaptureOne that expect a specific directory layout.

**Design decision — managed vs. referenced storage**:
- **Managed**: files are moved into the hash-based directory structure. Full dedup, clean layout.
- **Referenced**: files stay where they are. The store just indexes their hash and path. No dedup on disk, but no disruption to existing workflows.
- Both modes should be supported per-volume.

### 2. Metadata Store

**Responsibility**: persist and retrieve all asset metadata as text-based sidecar files.

**Sidecar format**: YAML, one file per asset.
```yaml
# <catalog_root>/metadata/<uuid-prefix>/<uuid>.yaml
id: 550e8400-e29b-41d4-a716-446655440000
name: "Sunset at beach"
asset_type: image
tags: [landscape, sunset, beach, vacation-2024]
description: "Golden hour shot from Koh Lanta"
created_at: 2024-07-15T18:32:00Z
variants:
  - content_hash: "sha256:abcdef..."
    role: original
    format: NEF
    file_size: 52428800
    original_filename: "DSC_4521.NEF"
  - content_hash: "sha256:123456..."
    role: processed
    format: TIFF
    file_size: 104857600
    original_filename: "DSC_4521_edited.tiff"
recipes:
  - variant: "sha256:abcdef..."
    software: "CaptureOne 23"
    recipe_type: sidecar
    content_hash: "sha256:fedcba..."
```

**Operations**:
- `save(asset)` — write/update sidecar YAML.
- `load(asset_id) -> Asset` — read sidecar YAML.
- `list() -> Vec<AssetSummary>` — enumerate all known assets.
- `sync_to_catalog()` — rebuild SQLite catalog from sidecar files (source of truth → cache).

### 3. Local Catalog (SQLite)

**Responsibility**: fast queryable index over all metadata. Rebuilt from sidecar files.

**Tables** mirror the data model: `assets`, `variants`, `file_locations`, `volumes`, `recipes`.

This is a **derived cache**, not the source of truth. Running `dam rebuild-catalog` regenerates it from sidecar files. This means:
- No data loss if the SQLite file is deleted.
- Sidecars can be edited manually or by external tools.
- The catalog can include denormalized fields for fast queries (e.g. extracted EXIF date, camera model).

### 4. Device Registry

**Responsibility**: volume management and online/offline detection.

**Operations**:
- `register(label, mount_point, type) -> Volume` — add a new volume.
- `list() -> Vec<Volume>` — list all volumes with online/offline status.
- `detect_online()` — check which mount points are currently available.
- `scan(volume_id)` — scan a volume for new/changed/deleted files.

**Online detection**: simply checks if the mount point directory exists and contains a marker file (`.dam-volume-id` containing the volume UUID).

### 5. Asset Service

**Responsibility**: high-level operations that orchestrate the other components.

**Operations**:
- `import(paths, volume_id) -> ImportResult` — hash files, extract metadata (EXIF etc.), create assets, create variants, write sidecars, update catalog. Auto-groups files that share the same filename stem and reside in the same directory (e.g. `DSC_4521.NEF`, `DSC_4521.jpg`, `DSC_4521.xmp`, `DSC_4521.cos` all become one asset). Media files become variants; processing sidecars (`.xmp`, `.cos`, `.cot`, `.cop`, etc.) are attached as recipes. Standalone recipe files (no co-located media) are resolved to parent variants by matching filename stem and directory on the same volume. When a file's content hash already exists, the new file location is added to the existing variant (both sidecar and catalog) rather than being silently skipped. Only truly skips when the exact location (volume + relative path) is already tracked. Re-importing a modified recipe updates it in place (new hash, re-extracted XMP metadata). Reports per-file status as `Imported`, `LocationAdded`, `Skipped`, `RecipeAttached`, or `RecipeUpdated`. Supports `--include`/`--skip` flags for file type group filtering.
- `group(variant_hashes) -> Asset` — manually group variants into one asset.
- `ungroup(asset_id, variant_hash)` — remove a variant from a group.
- `tag(asset_id, tags)` — add tags to an asset.
- `relocate(asset_id, target_volume)` — move all variants of an asset to another volume. Supports `--remove-source` (move instead of copy) and `--dry-run`.
- `find_duplicates() -> Vec<DuplicateGroup>` — find variants with same hash on multiple locations.
- `verify(paths, volume, asset) -> VerifyResult` — re-hash files on disk and compare against stored content hashes. Reports `Ok`, `Mismatch`, `Modified` (recipe with changed hash), `Missing`, `Skipped`, or `Untracked`. Modified recipes are not treated as failures — their stored hash is updated. Supports path mode (verify specific files/dirs), catalog mode (verify all locations), `--volume`, `--asset`, and `--include`/`--skip` filters.

### 6. Query Engine

**Responsibility**: search and filter assets via the SQLite catalog.

**Query capabilities**:
- Filter by: tags, date range, asset type, format, camera model, volume, online/offline status
- Full-text search over name and description
- Sort by: date, name, file size, import date
- Output: asset list with summary info, or detailed asset view

### 7. Preview Generator

**Responsibility**: create and cache thumbnails for browsing.

**Approach**:
- Images: use `image` crate for common formats, shell out to `dcraw` or `libraw` for RAW files.
- Videos: shell out to `ffmpeg` to extract a frame.
- Store previews in `<catalog_root>/previews/<hash-prefix>/<hash>.jpg` at a standard size (e.g. 800px longest edge).
- Generate on import, regenerate on demand.

### 8. Output Formatting

**Responsibility**: flexible output for scripting, pipelines, and machine consumption.

**Module**: `src/format.rs` — template engine with `{placeholder}` substitution and escape sequences.

**Capabilities**:
- **Global `--json` flag**: available on all commands. Outputs structured JSON to stdout; human-readable messages go to stderr. All result types derive `serde::Serialize`.
- **`--format` flag** (on `search` and `duplicates`): presets (`ids`, `short`, `full`, `json`) or custom templates (`'{id}\t{name}\t{tags}'`). When `--format` is explicit, result counts are suppressed.
- **`-q`/`--quiet`** (on `search`): shorthand for `--format=ids`, outputting one UUID per line for scripting.
- **Template placeholders**: `{id}`, `{short_id}`, `{name}`, `{filename}`, `{type}`, `{format}`, `{date}`, `{tags}`, `{description}`, `{hash}`. Templates support `\t` and `\n` escape sequences.

### 9. Stats

**Responsibility**: aggregate and display catalog statistics from the SQLite index.

**Implementation**: query methods on `Catalog` (in `src/catalog.rs`) compute counts, breakdowns, and coverage metrics. The `build_stats()` method assembles all sections into a `CatalogStats` struct, merging catalog data with device registry (online/offline status).

**Sections** (each gated by a CLI flag):
- **Overview** (always shown): asset/variant/recipe counts, volume totals (online/offline), total file size.
- **Types** (`--types`): asset type breakdown with percentages, top variant formats, recipe format distribution.
- **Volumes** (`--volumes`): per-volume asset/variant/recipe counts, size, directory count, format list, verification coverage.
- **Tags** (`--tags`): unique tag count, tagged/untagged asset counts, top tags by frequency.
- **Verification** (`--verified`): location verification coverage, oldest/newest timestamps, per-volume breakdown.

**Flags**: `--all` enables all sections. `--limit N` controls top-N lists (default 20). `--json` outputs structured `CatalogStats` JSON.

**Edge cases**: empty catalog returns all zeros without errors. Division-by-zero for percentages is guarded. Volumes with no files are included in `--volumes` with zero counts. Recipe format is extracted from `relative_path` extension in Rust, falling back to "unknown".

### 10. CLI

**Global flags**:
- `-t` / `--time` — show elapsed time after command execution
- `-l` / `--log` — log individual file progress during import/verify
- `--json` — output machine-readable JSON
- `-d` / `--debug` — show stderr output from external tools (ffmpeg, dcraw, dcraw_emu)

**Subcommands**:
```
dam init                                          # initialize a new catalog
dam volume add <label> <path>                     # register a volume
dam volume list                                   # list volumes and status
dam import <paths...> [--include G] [--skip G]    # import files
dam search <query> [--format F] [-q]              # search assets
dam show <asset-id>                               # show asset details
dam tag <asset-id> [--remove] <tags...>           # add/remove tags
dam group <variant-hashes...>                     # group variants into one asset
dam relocate <id> <vol> [--remove-source] [--dry-run]  # copy/move asset
dam verify [PATHS...] [--volume V] [--asset ID]   # check file integrity
dam duplicates [--format F]                       # find duplicates
dam generate-previews [--asset <id>] [--force]    # generate thumbnails
dam stats [--types] [--volumes] [--tags] [--verified] [--all] [--limit N]  # catalog statistics
dam rebuild-catalog                               # rebuild SQLite from sidecars
```

## Catalog Directory Structure

```
<catalog_root>/                       # e.g. ~/dam/ or wherever `dam init` was run
  dam.toml                            # catalog configuration (default volume, preferences)
  catalog.db                          # SQLite index (derived, rebuildable)
  metadata/
    55/
      550e8400-e29b-41d4-...yaml      # asset sidecar files, sharded by UUID prefix
  previews/
    ab/
      abcdef1234....jpg               # thumbnails, sharded by content hash prefix
  volumes.yaml                        # volume registry
```
