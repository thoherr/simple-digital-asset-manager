# Ingest Commands

Commands for importing files, applying metadata, and merging asset variants.

---

## dam import

### NAME

dam-import -- import files into the catalog

### SYNOPSIS

```
dam [GLOBAL FLAGS] import [OPTIONS] <PATHS...>
```

### DESCRIPTION

Imports files and directories into the catalog. Each file is hashed (SHA-256), and its metadata is extracted from EXIF data and XMP sidecars. Preview thumbnails are generated during import.

**Stem-based auto-grouping**: Files sharing the same filename stem in the same directory are grouped into one asset. RAW files take priority as the primary variant (defining the asset's identity and EXIF data). Additional media files become extra variants on the same asset.

**Recipe handling**: Processing sidecar files (`.xmp`, `.cos`, `.cot`, `.cop`, `.pp3`, `.dop`, `.on1`) are attached as recipe records to the primary variant rather than imported as standalone assets. Re-importing after external edits updates recipes in place.

**XMP metadata extraction**: When an `.xmp` sidecar is attached, its contents are parsed. Keywords become asset tags, `dc:description` sets the asset description, `xmp:Rating` becomes the asset rating, and `xmp:Label` becomes the color label. EXIF data takes precedence over XMP for overlapping fields.

**Embedded XMP**: JPEG and TIFF files have their embedded XMP metadata extracted automatically (keywords, ratings, descriptions, labels).

**Duplicate handling**: When a file's content hash already exists, the new file location is added to the existing variant. Only truly skips when the exact same location (volume + path) is already recorded.

**Preview generation**: Standard images get 800px JPEG thumbnails via the `image` crate. RAW files use `dcraw`/`dcraw_emu` (LibRaw). Videos use `ffmpeg`. Non-visual formats get an info card. Preview failure never blocks import.

The volume is auto-detected from the first path by matching against registered volume mount points. Use `--volume` to override.

### ARGUMENTS

**PATHS** (required)
: One or more file paths or directory paths to import. Directories are scanned recursively.

### OPTIONS

**--volume \<LABEL\>**
: Use a specific volume instead of auto-detecting from the path. Useful when auto-detection picks the wrong volume.

**--include \<GROUP\>**
: Include additional file type groups that are not imported by default. Can be specified multiple times. Example groups: `captureone`, `documents`.

**--skip \<GROUP\>**
: Skip file type groups that would normally be imported. Can be specified multiple times. Example groups: `audio`, `xmp`.

**--add-tag \<TAG\>**
: Add a tag to every imported asset. Can be specified multiple times. Merged with `[import] auto_tags` from `dam.toml` and XMP-extracted tags, deduplicated.

**--smart**
: Generate smart previews (high-resolution, 2560px) alongside regular thumbnails during import. Smart previews enable zoom and pan in the web UI. Can be enabled permanently via `[import] smart_previews = true` in `dam.toml`. Smart preview dimensions are controlled by `[preview] smart_max_edge`.

**--dry-run**
: Show what would be imported without writing to catalog, sidecar, or disk. Files are still hashed to detect duplicates. Reports the same counters as a real import (imported, skipped, locations added, recipes attached/updated).

**--auto-group**
: After importing, automatically group newly imported assets with nearby catalog assets by filename stem. "Nearby" means assets on the same volume whose files are under sibling directories of the imported files (one level up from each import directory). This handles the common CaptureOne/Lightroom pattern where RAW originals live in `Capture/` and exports in `Output/` under a shared session folder. Uses the same fuzzy prefix matching as `dam auto-group`. When combined with `--dry-run`, the auto-group phase also runs in dry-run mode.

`--json` outputs an `ImportResult` object with `imported`, `skipped`, `locations_added`, `recipes_attached`, `recipes_updated` counters and a `dry_run` boolean. When `--auto-group` produces matches, an `auto_group` key is added with the full `AutoGroupResult`.

### EXAMPLES

Import a directory of photos:

```bash
dam import /Volumes/Photos/Capture/2026-02-22
```

Import with explicit volume and progress logging:

```bash
dam import --volume "Archive" /Volumes/NAS/Photos/2025 --log --time
```

Preview what would be imported without making changes:

```bash
dam import --dry-run /Volumes/SD-Card/DCIM
```

Import only image files, skipping audio and XMP sidecars:

```bash
dam import --skip audio --skip xmp /Volumes/Photos/Mixed
```

Import with smart previews for high-resolution browsing:

```bash
dam import --smart /Volumes/Photos/Capture/2026-02-22
```

Tag assets during import:

```bash
dam import --add-tag landscape --add-tag "2026" /Volumes/Photos/Landscapes
```

Import a CaptureOne session and auto-group RAW+exports:

```bash
dam import --auto-group /Volumes/Photos/2026-02-22/Capture /Volumes/Photos/2026-02-22/Output
```

Import with JSON output for scripting:

```bash
dam import /Volumes/Photos/NewShoot --json | jq '.imported'
```

### SEE ALSO

[tag](#dam-tag) -- add or remove tags after import.
[edit](#dam-edit) -- set name, description, rating, or label.
[auto-group](#dam-auto-group) -- group related assets by filename stem.
[generate-previews](05-maintain-commands.md#dam-generate-previews) -- regenerate or upgrade previews.
[CLI Conventions](00-cli-conventions.md) -- `--log`, `--json`, `--time` behavior.

---

## dam delete

### NAME

dam-delete -- remove assets from the catalog

### SYNOPSIS

```
dam [GLOBAL FLAGS] delete [OPTIONS] [ASSET_IDS...]
```

### DESCRIPTION

Removes assets from the catalog. By default runs in **report-only mode** -- shows what would be deleted without making changes. Use `--apply` to execute the deletion.

When `--apply` is set, the following data is removed for each asset:

- The asset row from the SQLite catalog
- All variants belonging to the asset
- All file location records for those variants
- All recipe records attached to those variants
- All preview and smart preview files
- The YAML sidecar file
- Collection memberships (the asset is removed from all collections)
- Stack membership (the stack auto-dissolves if only one member remains)

With `--remove-files` (which requires `--apply`), physical media files and recipe files are also deleted from disk on online volumes. Files on offline volumes are skipped with a warning.

Asset IDs support unique prefix matching (see [CLI Conventions](00-cli-conventions.md#asset-id-matching)).

When no asset IDs are given on the command line, IDs are read from stdin (one per line). This enables piping from `dam search -q`:

```bash
dam search -q "orphan:true" | dam delete --apply
```

### ARGUMENTS

**ASSET_IDS** (optional)
: One or more asset IDs or unique prefixes. If omitted, reads from stdin.

### OPTIONS

**--apply**
: Execute the deletion. Without this flag, the command only reports what it would do.

**--remove-files**
: Also delete physical files (variant media and recipe files) from disk. Requires `--apply`. Skips files on offline volumes with a warning.

`--json` outputs a `DeleteResult` with `deleted`, `not_found`, `files_removed`, `previews_removed`, `dry_run`, and `errors`.

`--log` prints per-asset status to stderr.

### EXAMPLES

Preview what would be deleted (report-only):

```bash
dam delete a1b2c3d4
```

Delete an asset:

```bash
dam delete --apply a1b2c3d4
```

Delete multiple assets at once:

```bash
dam delete --apply a1b2c3d4 e5f6a7b8
```

Delete an asset and its files from disk:

```bash
dam delete --apply --remove-files a1b2c3d4
```

Delete all orphaned assets (no file locations):

```bash
dam search -q "orphan:true" | dam delete --apply
```

Delete with JSON output for scripting:

```bash
dam delete --apply a1b2c3d4 --json | jq '.deleted'
```

Use a short ID prefix:

```bash
dam delete --apply a1b2
```

### SEE ALSO

[cleanup](05-maintain-commands.md#dam-cleanup) -- remove stale location records and orphaned assets automatically.
[search](04-retrieve-commands.md#dam-search) -- find assets to delete (`orphan:true`, `missing:true`).
[show](04-retrieve-commands.md#dam-show) -- inspect an asset before deleting it.

---

## dam tag

### NAME

dam-tag -- add or remove tags on an asset

### SYNOPSIS

```
dam [GLOBAL FLAGS] tag <ASSET_ID> [--remove] <TAGS...>
```

### DESCRIPTION

Adds or removes tags on an asset. Tags are free-form text strings stored on the asset record. They are persisted in both the YAML sidecar file and the SQLite catalog.

When tags are changed, dam automatically writes the changes back to any `.xmp` recipe files associated with the asset. Tag write-back uses operation-level deltas: added tags are inserted into the XMP `dc:subject` / `rdf:Bag` block, and removed tags are deleted -- tags added independently in external tools like CaptureOne are preserved.

Tags are deduplicated: adding a tag that already exists is a no-op.

Asset IDs support unique prefix matching (see [CLI Conventions](00-cli-conventions.md#asset-id-matching)).

### ARGUMENTS

**ASSET_ID** (required)
: The asset ID or a unique prefix of it.

**TAGS** (required)
: One or more tags to add or remove.

### OPTIONS

**--remove**
: Remove the specified tags instead of adding them.

`--json` outputs a `TagResult` with `asset_id`, `tags_added` or `tags_removed`, and the full `tags` list after the operation.

### EXAMPLES

Add tags to an asset:

```bash
dam tag a1b2c3d4 landscape mountains "golden hour"
```

Remove a tag:

```bash
dam tag a1b2c3d4 --remove landscape
```

Add a multi-word tag:

```bash
dam tag a1b2c "Fools Theater"
```

Tag assets in bulk via search:

```bash
for id in $(dam search -q "path:Capture/2026-02-22"); do
  dam tag "$id" "February 2026"
done
```

### SEE ALSO

[edit](#dam-edit) -- set asset name, description, rating, or label.
[search](04-retrieve-commands.md#dam-search) -- `tag:` filter for finding tagged assets.
[stats](04-retrieve-commands.md#dam-stats) -- `--tags` shows tag usage frequencies.

---

## dam edit

### NAME

dam-edit -- edit asset metadata (name, description, rating, color label, date)

### SYNOPSIS

```
dam [GLOBAL FLAGS] edit <ASSET_ID> [OPTIONS]
```

### DESCRIPTION

Sets or clears an asset's name, description, rating, color label, and creation date from the CLI. At least one option must be provided.

Changes are written to both the YAML sidecar file (source of truth) and the SQLite catalog. Rating, description, and color label changes also trigger XMP write-back to any associated `.xmp` recipe files.

**Rating** is an integer from 1 to 5. Clearing it removes the rating entirely.

**Color label** accepts one of seven colors (case-insensitive): Red, Orange, Yellow, Green, Blue, Pink, Purple. The value is stored in canonical title-case. Clearing it removes the label.

**Name** is a custom display name for the asset. When set, it appears instead of the original filename in search results and the web UI.

**Description** is free-form text. Passing an empty string (`--description ""`) is equivalent to `--clear-description`.

**Date** accepts an ISO date string (e.g. `2024-12-25` or `2024-12-25T14:30:00`). This overrides the asset's `created_at` timestamp. Clearing it is not recommended since the field is always populated at import time.

Asset IDs support unique prefix matching (see [CLI Conventions](00-cli-conventions.md#asset-id-matching)).

### ARGUMENTS

**ASSET_ID** (required)
: The asset ID or a unique prefix of it.

### OPTIONS

**--name \<TEXT\>**
: Set the asset's display name.

**--clear-name**
: Remove the asset's display name (reverts to showing the original filename).

**--description \<TEXT\>**
: Set the asset's description. An empty string clears it.

**--clear-description**
: Remove the asset's description.

**--rating \<1-5\>**
: Set the asset's star rating (1 through 5).

**--clear-rating**
: Remove the asset's star rating.

**--label \<COLOR\>**
: Set the asset's color label. Accepts: Red, Orange, Yellow, Green, Blue, Pink, Purple (case-insensitive).

**--clear-label**
: Remove the asset's color label.

**--date \<YYYY-MM-DD\>**
: Set the asset's creation date (accepts ISO date or datetime strings).

**--clear-date**
: Remove the asset's creation date.

`--json` outputs an `EditResult` with the fields that were changed and their new values.

### EXAMPLES

Set a rating and description:

```bash
dam edit a1b2c3d4 --rating 5 --description "Best shot from the wedding ceremony"
```

Set a color label:

```bash
dam edit a1b2c --label Red
```

Give an asset a display name:

```bash
dam edit a1b2c3d4 --name "Sunset over Lake Constance"
```

Clear the rating and label:

```bash
dam edit a1b2c3d4 --clear-rating --clear-label
```

Correct an asset's date:

```bash
dam edit a1b2c3d4 --date "2024-12-25"
```

Clear the description (two equivalent forms):

```bash
dam edit a1b2c3d4 --clear-description
dam edit a1b2c3d4 --description ""
```

### SEE ALSO

[tag](#dam-tag) -- add or remove tags.
[show](04-retrieve-commands.md#dam-show) -- display full asset details including edited fields.
[search](04-retrieve-commands.md#dam-search) -- `rating:`, `label:` filters for finding assets.

---

## dam group

### NAME

dam-group -- merge variants into a single asset

### SYNOPSIS

```
dam [GLOBAL FLAGS] group <VARIANT_HASHES...>
```

### DESCRIPTION

Merges multiple variants (identified by their content hashes) into a single asset. This is used to combine files that belong together but were imported as separate assets -- for example, a RAW file and its exported TIFF, or multiple processing versions of the same shot.

The target asset is the oldest by creation date among the variants' current assets. All other ("donor") assets have their variants, tags, and recipes merged into the target.

Donor variants that have the role `original` are re-roled to `export` to avoid multiple originals on the same asset. This role change is applied in both the YAML sidecar and the SQLite catalog.

After merging, donor assets are deleted (their sidecar YAML files and catalog rows are removed). The target asset's denormalized columns (best variant hash, primary format, variant count) are updated.

### ARGUMENTS

**VARIANT_HASHES** (required)
: Two or more content hashes (SHA-256 hex strings) of variants to group. Each hash must correspond to an existing variant in the catalog.

### OPTIONS

This command only accepts [global flags](00-cli-conventions.md#global-flags).

### EXAMPLES

Group a RAW file and its export:

```bash
dam group abc123def456... 789012fed345...
```

Find variant hashes from asset details and group them:

```bash
dam show a1b2c --json | jq -r '.variants[].content_hash'
# Use the hashes from two different assets:
dam group <hash1> <hash2>
```

Group three variants (RAW, processed TIFF, final JPEG):

```bash
dam group abc123... def456... 789abc...
```

### SEE ALSO

[auto-group](#dam-auto-group) -- automatically group assets by filename stem.
[show](04-retrieve-commands.md#dam-show) -- display variant hashes for an asset.
[fix-roles](05-maintain-commands.md#dam-fix-roles) -- fix variant roles after grouping.

---

## dam auto-group

### NAME

dam-auto-group -- automatically group assets by filename stem

### SYNOPSIS

```
dam [GLOBAL FLAGS] auto-group [QUERY] [--apply]
```

### DESCRIPTION

Groups assets by filename stem using fuzzy prefix matching. This handles the common case where export tools (CaptureOne, Lightroom, Photoshop) append suffixes to the original filename: `Z91_8561.ARW` matches `Z91_8561-1-HighRes.tif` because `Z91_8561` is a prefix of `Z91_8561-1-HighRes` and the next character (`-`) is non-alphanumeric (a separator).

**Fuzzy prefix matching rules**:
- Two stems match if the shorter is a prefix of the longer and the character immediately after the prefix in the longer string is non-alphanumeric (e.g., `-`, `_`, ` `, `(`).
- This prevents false positives: `DSC_001` does not match `DSC_0010` because `0` is alphanumeric.
- Stems are compared case-insensitively.

**Chain resolution**: When stems form a chain (e.g., `Z91_8561`, `Z91_8561-1`, `Z91_8561-1-HighRes`), all resolve to the shortest root (`Z91_8561`).

**Target selection** within each group: (1) prefer the asset that has a RAW variant, then (2) the oldest asset by creation date.

Without `--apply`, runs in report-only mode (dry run) and shows what would be grouped. With `--apply`, performs the merging: donor variants are moved to the target asset with their role changed from `original` to `export`, tags and recipes are merged, and donor assets are deleted.

An optional search query scopes which assets are considered. Only assets matching the query participate in grouping.

### ARGUMENTS

**QUERY** (optional)
: A search query (same syntax as `dam search`) to limit which assets participate in grouping.

### OPTIONS

**--apply**
: Actually perform the grouping. Without this flag, the command only reports what it would do.

`--json` outputs an `AutoGroupResult` with `groups` (array of group details), `total_donors_merged`, `total_variants_moved`, and `dry_run` boolean.

`--log` prints per-group details to stderr.

### EXAMPLES

Preview what auto-group would merge across the entire catalog:

```bash
dam auto-group
```

Auto-group only assets from a specific import path:

```bash
dam auto-group "path:Capture/2026-02-22"
```

Apply grouping after reviewing the dry-run report:

```bash
dam auto-group --apply
```

Auto-group assets with a specific tag:

```bash
dam auto-group "tag:wedding" --apply --log
```

Auto-group with JSON output for scripting:

```bash
dam auto-group --apply --json | jq '{merged: .total_donors_merged, moved: .total_variants_moved}'
```

### SEE ALSO

[group](#dam-group) -- manually group specific variants by content hash.
[fix-roles](05-maintain-commands.md#dam-fix-roles) -- fix variant roles after grouping.
[search](04-retrieve-commands.md#dam-search) -- query syntax for scoping auto-group.

---

## dam auto-tag

> **Feature-gated**: requires building with `--features ai`. Not available in default builds.

### NAME

dam-auto-tag -- suggest or apply tags to images using AI (SigLIP zero-shot classification)

### SYNOPSIS

```
dam [GLOBAL FLAGS] auto-tag [OPTIONS]
dam [GLOBAL FLAGS] auto-tag --download
dam [GLOBAL FLAGS] auto-tag --remove-model
dam [GLOBAL FLAGS] auto-tag --list-models
dam [GLOBAL FLAGS] auto-tag --similar <ASSET_ID>
```

### DESCRIPTION

Uses SigLIP ViT-B/16-256 (a vision-language model) to classify images against a configurable tag vocabulary via zero-shot classification. Each image is encoded into a 768-dimensional embedding, scored against all label embeddings using sigmoid scoring, and labels above the confidence threshold are suggested as tags.

By default runs in **report-only mode** -- shows suggested tags without applying them. Use `--apply` to write suggested tags to assets.

The model files (~207 MB quantized ONNX) are downloaded from HuggingFace on first use. Use `--download` to pre-download, `--remove-model` to delete cached files, and `--list-models` to show model status and file sizes.

**Image selection**: For each asset, the command looks for the best available image in this order: smart preview (2560px) → regular preview (800px) → original file on an online volume. Non-image assets (video, audio, documents) are skipped.

**Embedding storage**: Image embeddings are stored in the SQLite catalog (`embeddings` table) for reuse. The `--similar` flag uses these stored embeddings to find visually similar assets by cosine similarity.

**Default labels**: ~100 photography categories are built in (landscape, portrait, architecture, animals, food, etc.). Override with a custom labels file via `--labels`.

**Prompt template**: Each label is wrapped with a prompt template (default: `"a photograph of {}"`) before text encoding. Configurable via `[ai] prompt` in `dam.toml`.

### OPTIONS

**--query \<QUERY\>**
: Filter which assets to process using the same search syntax as `dam search`.

**--asset \<ID\>**
: Process a single asset by ID (supports prefix matching).

**--volume \<LABEL\>**
: Process only assets on a specific volume.

**--threshold \<FLOAT\>**
: Minimum confidence score to suggest a tag (default: 0.25). Range: 0.0 to 1.0. Higher values produce fewer but more confident suggestions.

**--labels \<FILE\>**
: Path to a custom labels file (one label per line). Overrides the built-in default labels.

**--apply**
: Write suggested tags to assets. Without this flag, tags are only reported.

**--download**
: Download model files from HuggingFace if not already present. Returns early after download.

**--remove-model**
: Delete cached model files from disk. Returns early.

**--list-models**
: Show model download status and file sizes. Returns early.

**--similar \<ASSET_ID\>**
: Find the 20 most visually similar assets to the given asset, using stored embeddings. The target asset must have been previously processed by `auto-tag`.

`--json` outputs an `AutoTagResult` with `assets_processed`, `assets_skipped`, `tags_suggested`, `tags_applied`, `errors`, `dry_run`, and per-asset `suggestions`.

`--log` prints per-asset status to stderr.

### EXAMPLES

Download the model (first-time setup):

```bash
dam auto-tag --download
```

Preview suggested tags for all images (report-only):

```bash
dam auto-tag
```

Auto-tag a specific asset and apply the tags:

```bash
dam auto-tag --asset a1b2c3d4 --apply
```

Auto-tag images matching a search query with a higher threshold:

```bash
dam auto-tag --query "type:image rating:4+" --threshold 0.4 --apply
```

Use a custom labels file:

```bash
dam auto-tag --labels my-labels.txt --apply
```

Find visually similar images:

```bash
dam auto-tag --similar a1b2c3d4
```

Show model status:

```bash
dam auto-tag --list-models
```

Auto-tag with JSON output for scripting:

```bash
dam auto-tag --query "tag:unreviewed" --json | jq '.suggestions[] | {asset: .asset_id, tags: [.suggested_tags[].tag]}'
```

### SEE ALSO

[tag](#dam-tag) -- manually add or remove tags.
[search](04-retrieve-commands.md#dam-search) -- `tag:` filter for finding tagged assets.
[Configuration](08-configuration.md#ai-section) -- `[ai]` section for default threshold, labels, and prompt template.

---

Previous: [Setup Commands](01-setup-commands.md) -- `init`, `volume add`, `volume list`.
Next: [Organize Commands](03-organize-commands.md) -- `collection`, `saved-search`.
