# Ingest Commands

Commands for importing files, applying metadata, and merging asset variants.

---

## maki import

### NAME

maki-import -- import files into the catalog

### SYNOPSIS

```
maki [GLOBAL FLAGS] import [OPTIONS] <PATHS...>
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
: Add a tag to every imported asset. Can be specified multiple times. Merged with `[import] auto_tags` from `maki.toml` and XMP-extracted tags, deduplicated.

**--smart**
: Generate smart previews (high-resolution, 2560px) alongside regular thumbnails during import. Smart previews enable zoom and pan in the web UI. Can be enabled permanently via `[import] smart_previews = true` in `maki.toml`. Smart preview dimensions are controlled by `[preview] smart_max_edge`.

**--embed** *(Pro)*
: Generate SigLIP image embeddings for visual similarity search during import. Embeddings enable `maki auto-tag --similar` and the web UI "Find similar" button. Runs as a post-import phase using preview images (smart preview preferred, then regular preview). Silently skips if the AI model is not downloaded -- run `maki auto-tag --download` first. Can be enabled permanently via `[import] embeddings = true` in `maki.toml`. Non-image assets are skipped. Uses the model configured in `[ai] model`.

**--describe** *(Pro)*
: Generate VLM descriptions for newly imported assets as a post-import phase. Requires a running Ollama instance (or compatible VLM endpoint configured in `[vlm]`). Runs after the embed phase if both are enabled. Uses the VLM model, prompt, and parameters from `[vlm]` config (including per-model overrides). Concurrency is controlled by `[vlm] concurrency`. Can be enabled permanently via `[import] descriptions = true` in `maki.toml`. Non-image assets are skipped.

**--dry-run**
: Show what would be imported without writing to catalog, sidecar, or disk. Files are still hashed to detect duplicates. Reports the same counters as a real import (imported, skipped, locations added, recipes attached/updated).

**--auto-group**
: After importing, automatically group newly imported assets with nearby catalog assets by filename stem. "Nearby" means assets on the same volume whose files are under sibling directories of the imported files (one level up from each import directory). This handles the common CaptureOne/Lightroom pattern where RAW originals live in `Capture/` and exports in `Output/` under a shared session folder. Uses the same fuzzy prefix matching as `maki auto-group`. When combined with `--dry-run`, the auto-group phase also runs in dry-run mode.

`--json` outputs an `ImportResult` object with `imported`, `skipped`, `locations_added`, `recipes_attached`, `recipes_updated` counters and a `dry_run` boolean. When `--auto-group` produces matches, an `auto_group` key is added with the full `AutoGroupResult`. When `--embed` generates embeddings, `embeddings_generated` and `embeddings_skipped` keys are added. When `--describe` generates descriptions, `descriptions_generated` and `descriptions_skipped` keys are added.

### EXAMPLES

Import a directory of photos:

```bash
maki import /Volumes/Photos/Capture/2026-02-22
```

Import with explicit volume and progress logging:

```bash
maki import --volume "Archive" /Volumes/NAS/Photos/2025 --log --time
```

Preview what would be imported without making changes:

```bash
maki import --dry-run /Volumes/SD-Card/DCIM
```

Import only image files, skipping audio and XMP sidecars:

```bash
maki import --skip audio --skip xmp /Volumes/Photos/Mixed
```

Import with smart previews for high-resolution browsing:

```bash
maki import --smart /Volumes/Photos/Capture/2026-02-22
```

Import with embedding generation for visual similarity search *(Pro)*:

```bash
maki import --embed /Volumes/Photos/Capture/2026-02-22
```

Import with both smart previews and embeddings:

```bash
maki import --smart --embed /Volumes/Photos/Capture/2026-02-22
```

Tag assets during import:

```bash
maki import --add-tag landscape --add-tag "2026" /Volumes/Photos/Landscapes
```

Import a CaptureOne session and auto-group RAW+exports:

```bash
maki import --auto-group /Volumes/Photos/2026-02-22/Capture /Volumes/Photos/2026-02-22/Output
```

Import with JSON output for scripting:

```bash
maki import /Volumes/Photos/NewShoot --json | jq '.imported'
```

### SEE ALSO

[tag](#maki-tag) -- add or remove tags after import.
[edit](#maki-edit) -- set name, description, rating, or label.
[auto-group](#maki-auto-group) -- group related assets by filename stem.
[generate-previews](05-maintain-commands.md#maki-generate-previews) -- regenerate or upgrade previews.
[CLI Conventions](00-cli-conventions.md) -- `--log`, `--json`, `--time` behavior.

---

## maki delete

### NAME

maki-delete -- remove assets from the catalog

### SYNOPSIS

```
maki [GLOBAL FLAGS] delete [OPTIONS] [ASSET_IDS...]
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

When no asset IDs are given on the command line, IDs are read from stdin (one per line). This enables piping from `maki search -q`:

```bash
maki search -q "orphan:true" | maki delete --apply
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
maki delete a1b2c3d4
```

Delete an asset:

```bash
maki delete --apply a1b2c3d4
```

Delete multiple assets at once:

```bash
maki delete --apply a1b2c3d4 e5f6a7b8
```

Delete an asset and its files from disk:

```bash
maki delete --apply --remove-files a1b2c3d4
```

Delete all orphaned assets (no file locations):

```bash
maki search -q "orphan:true" | maki delete --apply
```

Delete with JSON output for scripting:

```bash
maki delete --apply a1b2c3d4 --json | jq '.deleted'
```

Use a short ID prefix:

```bash
maki delete --apply a1b2
```

### SEE ALSO

[cleanup](05-maintain-commands.md#maki-cleanup) -- remove stale location records and orphaned assets automatically.
[search](04-retrieve-commands.md#maki-search) -- find assets to delete (`orphan:true`, `missing:true`).
[show](04-retrieve-commands.md#maki-show) -- inspect an asset before deleting it.

---

## maki tag

### NAME

maki-tag -- add or remove tags on an asset

### SYNOPSIS

```
maki [GLOBAL FLAGS] tag <ASSET_ID> [--remove] <TAGS...>
```

### DESCRIPTION

Adds or removes tags on an asset. Tags are free-form text strings stored on the asset record. They are persisted in both the YAML sidecar file and the SQLite catalog.

When tags are changed, MAKI automatically writes the changes back to any `.xmp` recipe files associated with the asset. Tag write-back uses operation-level deltas: added tags are inserted into the XMP `dc:subject` / `rdf:Bag` block, and removed tags are deleted -- tags added independently in external tools like CaptureOne are preserved.

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
maki tag a1b2c3d4 landscape mountains "golden hour"
```

Remove a tag:

```bash
maki tag a1b2c3d4 --remove landscape
```

Add a multi-word tag:

```bash
maki tag a1b2c "Fools Theater"
```

Tag assets in bulk via search:

```bash
for id in $(maki search -q "path:Capture/2026-02-22"); do
  maki tag "$id" "February 2026"
done
```

### SEE ALSO

[edit](#maki-edit) -- set asset name, description, rating, or label.
[search](04-retrieve-commands.md#maki-search) -- `tag:` filter for finding tagged assets.
[stats](04-retrieve-commands.md#maki-stats) -- `--tags` shows tag usage frequencies.

---

## maki edit

### NAME

maki-edit -- edit asset metadata (name, description, rating, color label, date, variant role)

### SYNOPSIS

```
maki [GLOBAL FLAGS] edit <ASSET_ID> [OPTIONS]
```

### DESCRIPTION

Sets or clears an asset's name, description, rating, color label, and creation date from the CLI. Can also change the role of a specific variant. At least one option must be provided.

Changes are written to both the YAML sidecar file (source of truth) and the SQLite catalog. Rating, description, and color label changes also trigger XMP write-back to any associated `.xmp` recipe files.

**Rating** is an integer from 1 to 5. Clearing it removes the rating entirely.

**Color label** accepts one of seven colors (case-insensitive): Red, Orange, Yellow, Green, Blue, Pink, Purple. The value is stored in canonical title-case. Clearing it removes the label.

**Name** is a custom display name for the asset. When set, it appears instead of the original filename in search results and the web UI.

**Description** is free-form text. Passing an empty string (`--description ""`) is equivalent to `--clear-description`.

**Date** accepts an ISO date string (e.g. `2024-12-25` or `2024-12-25T14:30:00`). This overrides the asset's `created_at` timestamp. Clearing it is not recommended since the field is always populated at import time.

**Variant role** (`--role` + `--variant`) changes the role of a specific variant within a multi-variant asset. Valid roles: `original`, `alternate`, `processed`, `export`, `sidecar`. This is useful when import or `fix-roles` assigns an incorrect role (e.g. marking a re-exported JPEG as "alternate" when it should be "export"). Role changes update both sidecar YAML and SQLite catalog, and recompute denormalized columns (best preview variant, primary format).

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

**--role \<ROLE\>**
: Change a variant's role. Must be used with `--variant`. Accepts: original, alternate, processed, export, sidecar.

**--variant \<HASH\>**
: The content hash of the variant whose role should be changed. Must be used with `--role`. Use `maki show <id>` to find variant hashes.

`--json` outputs an `EditResult` with the fields that were changed and their new values.

### EXAMPLES

Set a rating and description:

```bash
maki edit a1b2c3d4 --rating 5 --description "Best shot from the wedding ceremony"
```

Set a color label:

```bash
maki edit a1b2c --label Red
```

Give an asset a display name:

```bash
maki edit a1b2c3d4 --name "Sunset over Lake Constance"
```

Clear the rating and label:

```bash
maki edit a1b2c3d4 --clear-rating --clear-label
```

Correct an asset's date:

```bash
maki edit a1b2c3d4 --date "2024-12-25"
```

Clear the description (two equivalent forms):

```bash
maki edit a1b2c3d4 --clear-description
maki edit a1b2c3d4 --description ""
```

Change a variant's role from alternate to export:

```bash
maki edit a1b2c3d4 --role export --variant sha256:abcdef1234567890...
```

### SEE ALSO

[tag](#maki-tag) -- add or remove tags.
[show](04-retrieve-commands.md#maki-show) -- display full asset details including edited fields.
[search](04-retrieve-commands.md#maki-search) -- `rating:`, `label:` filters for finding assets.
[fix-roles](05-maintain-commands.md#maki-fix-roles) -- batch re-role non-RAW variants in RAW+non-RAW groups.

---

## maki group

### NAME

maki-group -- merge variants into a single asset

### SYNOPSIS

```
maki [GLOBAL FLAGS] group <VARIANT_HASHES...>
```

### DESCRIPTION

Merges multiple variants (identified by their content hashes) into a single asset. This is used to combine files that belong together but were imported as separate assets -- for example, a RAW file and its exported TIFF, or multiple processing versions of the same shot.

The target asset is the oldest by creation date among the variants' current assets. All other ("donor") assets have their variants, tags, and recipes merged into the target.

Donor variants that have the role `original` are re-roled to `alternate` to avoid multiple originals on the same asset. This role change is applied in both the YAML sidecar and the SQLite catalog.

After merging, donor assets are deleted (their sidecar YAML files and catalog rows are removed). The target asset's denormalized columns (best variant hash, primary format, variant count) are updated.

### ARGUMENTS

**VARIANT_HASHES** (required)
: Two or more content hashes (SHA-256 hex strings) of variants to group. Each hash must correspond to an existing variant in the catalog.

### OPTIONS

This command only accepts [global flags](00-cli-conventions.md#global-flags).

### EXAMPLES

Group a RAW file and its export:

```bash
maki group abc123def456... 789012fed345...
```

Find variant hashes from asset details and group them:

```bash
maki show a1b2c --json | jq -r '.variants[].content_hash'
# Use the hashes from two different assets:
maki group <hash1> <hash2>
```

Group three variants (RAW, processed TIFF, final JPEG):

```bash
maki group abc123... def456... 789abc...
```

### SEE ALSO

[split](#maki-split) -- the inverse operation: extract variants into standalone assets.
[auto-group](#maki-auto-group) -- automatically group assets by filename stem.
[show](04-retrieve-commands.md#maki-show) -- display variant hashes for an asset.
[fix-roles](05-maintain-commands.md#maki-fix-roles) -- fix variant roles after grouping.

---

## maki split

### NAME

maki-split -- extract variants from an asset into new standalone assets

### SYNOPSIS

```
maki [GLOBAL FLAGS] split <ASSET_ID> <VARIANT_HASHES...>
```

### DESCRIPTION

Splits one or more variants out of an existing asset, creating a new standalone asset for each extracted variant. This is the inverse of `maki group`.

Each extracted variant:

- Becomes the sole variant of a new asset with role `original`.
- Inherits tags, rating, color label, and description from the source asset.
- Takes associated recipes with it.
- Gets a deterministic UUID derived from its content hash.

The source asset retains all non-extracted variants. At least one variant must remain in the source asset.

### ARGUMENTS

**ASSET_ID** (required)
: Asset ID (or unique prefix) to split.

**VARIANT_HASHES** (required)
: Content hashes of variants to extract.

### OPTIONS

Standard global flags (`--json`, `--log`, `--time`).

### EXAMPLES

Show variants of an asset:

```bash
maki show abc12345
```

Extract a specific variant into its own asset:

```bash
maki split abc12345 sha256:def456...
```

Extract multiple variants (each becomes a separate asset):

```bash
maki split abc12345 sha256:aaa... sha256:bbb...
```

JSON output for scripting:

```bash
maki --json split abc12345 sha256:def456...
```

### SEE ALSO

[group](#maki-group) -- the inverse operation: merge variants into a single asset.
[auto-group](#maki-auto-group) -- automatically group assets by filename stem.
[show](04-retrieve-commands.md#maki-show) -- display variant hashes for an asset.

---

## maki auto-group

### NAME

maki-auto-group -- automatically group assets by filename stem

### SYNOPSIS

```
maki [GLOBAL FLAGS] auto-group [QUERY] [--apply]
```

### DESCRIPTION

Groups assets by filename stem using fuzzy prefix matching. This handles the common case where export tools (CaptureOne, Lightroom, Photoshop) append suffixes to the original filename: `Z91_8561.ARW` matches `Z91_8561-1-HighRes.tif` because `Z91_8561` is a prefix of `Z91_8561-1-HighRes` and the next character (`-`) is non-alphanumeric (a separator).

**Fuzzy prefix matching rules**:
- Two stems match if the shorter is a prefix of the longer and the character immediately after the prefix in the longer string is non-alphanumeric (e.g., `-`, `_`, ` `, `(`).
- This prevents false positives: `DSC_001` does not match `DSC_0010` because `0` is alphanumeric.
- Stems are compared case-insensitively.

**Chain resolution**: When stems form a chain (e.g., `Z91_8561`, `Z91_8561-1`, `Z91_8561-1-HighRes`), all resolve to the shortest root (`Z91_8561`).

**Target selection** within each group: (1) prefer the asset that has a RAW variant, then (2) the oldest asset by creation date.

Without `--apply`, runs in report-only mode (dry run) and shows what would be grouped. With `--apply`, performs the merging: donor variants are moved to the target asset with their role changed from `original` to `alternate`, tags and recipes are merged, and donor assets are deleted.

An optional search query scopes which assets are considered. Only assets matching the query participate in grouping.

### ARGUMENTS

**QUERY** (optional)
: A search query (same syntax as `maki search`) to limit which assets participate in grouping.

### OPTIONS

**--apply**
: Actually perform the grouping. Without this flag, the command only reports what it would do.

`--json` outputs an `AutoGroupResult` with `groups` (array of group details), `total_donors_merged`, `total_variants_moved`, and `dry_run` boolean.

`--log` prints per-group details to stderr.

### EXAMPLES

Preview what auto-group would merge across the entire catalog:

```bash
maki auto-group
```

Auto-group only assets from a specific import path:

```bash
maki auto-group "path:Capture/2026-02-22"
```

Apply grouping after reviewing the dry-run report:

```bash
maki auto-group --apply
```

Auto-group assets with a specific tag:

```bash
maki auto-group "tag:wedding" --apply --log
```

Auto-group with JSON output for scripting:

```bash
maki auto-group --apply --json | jq '{merged: .total_donors_merged, moved: .total_variants_moved}'
```

### SEE ALSO

[group](#maki-group) -- manually group specific variants by content hash.
[fix-roles](05-maintain-commands.md#maki-fix-roles) -- fix variant roles after grouping.
[search](04-retrieve-commands.md#maki-search) -- query syntax for scoping auto-group.

---

## maki auto-tag *(Pro)*

### NAME

maki-auto-tag -- suggest or apply tags to images using AI (SigLIP zero-shot classification, multi-model)

### SYNOPSIS

```
maki [GLOBAL FLAGS] auto-tag [QUERY] [OPTIONS]
maki [GLOBAL FLAGS] auto-tag [OPTIONS] --asset <ID>
maki [GLOBAL FLAGS] auto-tag [OPTIONS] --volume <LABEL>
maki [GLOBAL FLAGS] auto-tag --download [--model <ID>]
maki [GLOBAL FLAGS] auto-tag --remove-model [--model <ID>]
maki [GLOBAL FLAGS] auto-tag --list-models
maki [GLOBAL FLAGS] auto-tag --similar <ASSET_ID>
```

### DESCRIPTION

Uses SigLIP vision-language models to classify images against a configurable tag vocabulary via zero-shot classification. Each image is encoded into an embedding, scored against all label embeddings using sigmoid scoring, and labels above the confidence threshold are suggested as tags.

Two models are available:

| Model ID | Name | Embedding | Size | Notes |
|----------|------|-----------|------|-------|
| `siglip-vit-b16-256` | SigLIP ViT-B/16-256 | 768-dim | ~207 MB | Default, good balance |
| `siglip-vit-l16-256` | SigLIP ViT-L/16-256 | 1024-dim | ~670 MB | Higher accuracy |

Select with `--model <id>` or set `[ai] model` in `maki.toml`. The default is `siglip-vit-b16-256`.

By default runs in **report-only mode** -- shows suggested tags without applying them. Use `--apply` to write suggested tags to assets.

**Scope required**: at least one scope (positional query, `--asset`, or `--volume`) must be specified to prevent accidental full-catalog processing. Use `""` (empty query) to process all assets.

Model files are downloaded from HuggingFace on first use. Use `--download` to pre-download, `--remove-model` to delete cached files, and `--list-models` to show all known models with download status, size, and active indicator.

**Image selection**: For each asset, the command looks for the best available image in this order: smart preview (2560px) → regular preview (800px) → original file on an online volume. Non-image assets (video, audio, documents) are skipped.

**Embedding storage**: Image embeddings are stored in the SQLite catalog (`embeddings` table) per model. Switching models does not corrupt existing embeddings; each model's embeddings are stored separately. The `--similar` flag uses stored embeddings from the active model to find visually similar assets via an in-memory index (dot product on L2-normalized vectors). Embeddings are also stored opportunistically by the web UI "Suggest tags" and batch "Auto-tag" endpoints. Use `maki embed` to batch-generate embeddings without tagging.

**Default labels**: ~100 photography categories are built in (landscape, portrait, architecture, animals, food, etc.). Override with a custom labels file via `--labels`.

**Prompt template**: Each label is wrapped with a prompt template (default: `"a photograph of {}"`) before text encoding. Configurable via `[ai] prompt` in `maki.toml`.

### OPTIONS

**\<QUERY\>** (positional, optional)
: Filter which assets to process using the same search syntax as `maki search`.

**--asset \<ID\>**
: Process a single asset by ID (supports prefix matching).

**--volume \<LABEL\>**
: Process only assets on a specific volume.

**--model \<ID\>**
: Select which SigLIP model to use. Available: `siglip-vit-b16-256` (default), `siglip-vit-l16-256`. Overrides `[ai] model` in `maki.toml`. Also applies to `--download` and `--remove-model`.

**--threshold \<FLOAT\>**
: Minimum confidence score to suggest a tag (default: 0.1). Range: 0.0 to 1.0. Higher values produce fewer but more confident suggestions.

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

**--list-labels**
: Print the active label list (one per line) and exit. Shows the built-in defaults, or the labels from `--labels` / `[ai] labels` config if set. Works without a catalog when using defaults. Pipe to a file to create a custom labels blueprint: `maki auto-tag --list-labels > my-labels.txt`. Supports `--json` (outputs a JSON array).

**--similar \<ASSET_ID\>**
: Find the 20 most visually similar assets to the given asset, using stored embeddings. If the target asset has no stored embedding, it is encoded on-the-fly. Other assets must have been previously processed by `auto-tag` or `maki embed`.

`--json` outputs an `AutoTagResult` with `assets_processed`, `assets_skipped`, `tags_suggested`, `tags_applied`, `errors`, `dry_run`, and per-asset `suggestions`.

`--log` prints per-asset status to stderr.

### EXAMPLES

Download the default model (first-time setup):

```bash
maki auto-tag --download
```

Download the larger model for higher accuracy:

```bash
maki auto-tag --download --model siglip-vit-l16-256
```

Preview suggested tags for all images (report-only):

```bash
maki auto-tag ""
```

Auto-tag a specific asset and apply the tags:

```bash
maki auto-tag --asset a1b2c3d4 --apply
```

Auto-tag images matching a search query with a higher threshold:

```bash
maki auto-tag "type:image rating:4+" --threshold 0.4 --apply
```

Use the larger model for a specific query:

```bash
maki auto-tag "tag:unreviewed" --model siglip-vit-l16-256 --apply
```

Use a custom labels file:

```bash
maki auto-tag --labels my-labels.txt "" --apply
```

Find visually similar images:

```bash
maki auto-tag --similar a1b2c3d4
```

Export default labels as a blueprint for customization:

```bash
maki auto-tag --list-labels > my-labels.txt
# Edit the file, then use it:
maki auto-tag --labels my-labels.txt --apply
```

Show model status:

```bash
maki auto-tag --list-models
```

Auto-tag with JSON output for scripting:

```bash
maki auto-tag "tag:unreviewed" --json | jq '.suggestions[] | {asset: .asset_id, tags: [.suggested_tags[].tag]}'
```

### SEE ALSO

[embed](#maki-embed) -- batch-generate embeddings for similarity search.
[tag](#maki-tag) -- manually add or remove tags.
[search](04-retrieve-commands.md#maki-search) -- `tag:` filter for finding tagged assets.
[Configuration](08-configuration.md#ai-section) -- `[ai]` section for default threshold, labels, and prompt template.

---

## maki embed *(Pro)*

### NAME

maki-embed -- batch-generate image embeddings for visual similarity search

### SYNOPSIS

```
maki [GLOBAL FLAGS] embed [QUERY] [OPTIONS]
maki [GLOBAL FLAGS] embed --asset <ID> [OPTIONS]
maki [GLOBAL FLAGS] embed --volume <LABEL> [OPTIONS]
maki [GLOBAL FLAGS] embed --export
```

### DESCRIPTION

Pre-computes image embeddings for visual similarity search (`maki auto-tag --similar` and the web UI "Find similar" button) without applying any tags. This is useful for building up the similarity search index across your catalog.

For each matching asset, the command finds the best available image (smart preview → regular preview → original file on an online volume), encodes it with SigLIP, and stores the embedding in the SQLite catalog's `embeddings` table and as a binary file under `embeddings/<model>/<prefix>/<asset_id>.bin`.

By default, assets that already have a stored embedding for the active model are skipped. Use `--force` to re-generate embeddings (e.g., after switching to a higher-resolution preview).

**Scope required**: at least one scope (positional query, `--asset`, or `--volume`) must be specified to prevent accidental full-catalog processing. Use `""` (empty query) to process all assets.

**Non-destructive**: embedding generation does not modify any asset metadata, tags, or files. It only writes to the `embeddings` table.

### OPTIONS

**\<QUERY\>** (positional, optional)
: Filter which assets to process using the same search syntax as `maki search`.

**--asset \<ID\>**
: Process a single asset by ID (supports prefix matching).

**--volume \<LABEL\>**
: Process only assets on a specific volume.

**--model \<ID\>**
: Select which SigLIP model to use. Available: `siglip-vit-b16-256` (default), `siglip-vit-l16-256`. Overrides `[ai] model` in `maki.toml`.

**--force**
: Re-generate embeddings even if they already exist for the active model.

**--export**
: Export all existing embeddings from SQLite to binary files. No scope filter required. Useful as a one-time migration to populate the file-based persistence layer from existing data.

`--json` outputs `{embedded, skipped, errors, model, force}`.

`--log` prints per-asset status to stderr.

### EXAMPLES

Generate embeddings for all assets:

```bash
maki embed ""
```

Generate embeddings for a single asset:

```bash
maki embed --asset a1b2c3d4
```

Generate embeddings for a specific volume:

```bash
maki embed --volume "Photos 2024"
```

Force re-generation with a different model:

```bash
maki embed "" --model siglip-vit-l16-256 --force
```

Generate embeddings for untagged images only:

```bash
maki embed "tag:none type:image"
```

Check progress with JSON output:

```bash
maki embed "" --json | jq '{embedded, skipped}'
```

Export all embeddings to binary files (one-time migration):

```bash
maki embed --export
```

### SEE ALSO

[auto-tag](#maki-auto-tag) -- AI tag suggestion and visual similarity search.
[Configuration](08-configuration.md#ai-section) -- `[ai]` section for model and model directory settings.

---

## maki describe *(Pro)*

### NAME

maki-describe -- generate image descriptions using a vision-language model (VLM)

### SYNOPSIS

```
maki [GLOBAL FLAGS] describe [QUERY] [OPTIONS]
```

### DESCRIPTION

Sends preview images to a VLM server and generates natural language descriptions and/or tags. The command uses the OpenAI-compatible chat completions API, which is implemented by Ollama, LM Studio, vLLM, and most local inference servers.

Three modes are available via `--mode`:

- **describe** (default) — generates a natural language description for each asset.
- **tags** — asks the VLM to suggest tags, returned as a JSON array. Tags are deduplicated (case-insensitive) and existing asset tags are preserved.
- **both** — runs describe and tags as two separate VLM calls per asset, combining the results. This is equivalent to running `--mode describe` and `--mode tags` independently, so each call uses its optimal prompt.

By default, the command runs in **report-only mode**: results are generated and displayed but not saved. Use `--apply` to write descriptions/tags to assets. Use `--dry-run` to see what would be processed without calling the VLM at all.

The command requires at least one scope (positional query, `--asset`, or `--volume`) to prevent accidental processing of the entire catalog.

For each asset, the command:
1. Checks if a description already exists (skips unless `--force` is set; tags mode always runs)
2. Finds the best available image (smart preview > regular preview > original on an online volume)
3. Base64-encodes the image and sends it to the VLM endpoint
4. Parses the response and optionally saves descriptions/tags to the asset

### OPTIONS

**\<QUERY\>** (positional, optional)
: Search query to scope which assets are described. Same syntax as `maki search`.

**--asset \<ID\>**
: Process a single asset (ID or unique prefix).

**--volume \<LABEL\>**
: Limit to assets on a specific volume.

**--model \<NAME\>**
: VLM model name. Default from `[vlm] model` in `maki.toml`, or `qwen2.5vl:3b`.

**--endpoint \<URL\>**
: VLM server base URL. Default from `[vlm] endpoint` in `maki.toml`, or `http://localhost:11434`.

**--prompt \<TEXT\>**
: Custom prompt sent to the VLM. Default from `[vlm] prompt` in `maki.toml`, or a built-in photography-focused prompt. In `--mode both`, custom prompts are ignored because each call uses its specialized built-in prompt.

**--max-tokens \<N\>**
: Maximum tokens in the VLM response. Default from `[vlm] max_tokens` in `maki.toml`, or `500`.

**--timeout \<SECONDS\>**
: Timeout for each VLM request. Default from `[vlm] timeout` in `maki.toml`, or `300`. Increase for larger models or first-time model loading.

**--mode \<MODE\>**
: Output mode: `describe` (default), `tags`, or `both`. In `both` mode, two VLM calls are made per asset — one for description, one for tags.

**--temperature \<FLOAT\>**
: Sampling temperature controlling randomness. `0.0` = deterministic (always picks the most likely token), `0.7` = balanced (default), `1.0+` = more creative. Lower values produce more consistent but potentially blander output. Default from `[vlm] temperature` in `maki.toml`, or `0.7`.

**--num-ctx \<N\>**
: Context window size passed to the VLM server (Ollama `num_ctx`). When non-zero, overrides the model's default context length. Useful for models that benefit from a larger context (e.g., `--num-ctx 4096`). Default: `0` (server default).

**--top-p \<FLOAT\>**
: Nucleus sampling threshold. Only tokens whose cumulative probability exceeds this value are considered. Lower values produce more focused output. Default: `0.0` (server default).

**--top-k \<N\>**
: Top-k sampling: limit token selection to the k most likely candidates. Lower values produce more deterministic output. Default: `0` (server default).

**--repeat-penalty \<FLOAT\>**
: Repetition penalty factor. Values above `1.0` discourage the model from repeating tokens. Default: `0.0` (server default).

**--apply**
: Write descriptions and/or tags to assets. Without this flag, results are generated and displayed but not saved.

**--force**
: Overwrite existing descriptions. By default, assets that already have a description are skipped.

**--dry-run**
: Show what would be processed without calling the VLM. No network requests are made.

**--check**
: Test connectivity to the VLM endpoint. Prints the server status and available models, then exits. Does not process any assets.

### EXAMPLES

Check that Ollama is running and see available models:

```bash
maki describe --check
```

Preview descriptions for undescribed assets (report-only):

```bash
maki describe "description:none type:image"
```

Generate and apply descriptions to a specific volume:

```bash
maki describe --volume "Photos 2024" --apply
```

Describe a single asset:

```bash
maki describe --asset a1b2c3d4 --apply
```

Use a faster model for batch processing:

```bash
maki describe "date:2024-06" --model moondream --apply
```

Use a remote server or larger model:

```bash
maki describe --endpoint http://gpu-server:11434 --model qwen2.5vl:7b --apply
```

Generate tags only:

```bash
maki describe --mode tags "tag:untagged" --apply
```

Generate both descriptions and tags in one pass (two VLM calls per asset):

```bash
maki describe --mode both --asset a1b2c3d4 --apply
```

Custom prompt for architectural photography:

```bash
maki describe --prompt "Describe the architectural style, materials, and features." "tag:architecture" --apply
```

Deterministic output (temperature 0) for reproducible batch tagging:

```bash
maki describe --mode tags --temperature 0 "date:2024-06" --apply
```

Increase timeout for a large model's first load:

```bash
maki describe --model qwen2.5vl:7b --timeout 300 --asset a1b2c3d4
```

Use a larger context window and nucleus sampling with a specific model:

```bash
maki describe --model qwen3-vl:4b --num-ctx 4096 --top-p 0.9 --apply
```

Overwrite existing descriptions with a better model:

```bash
maki describe "rating:4+" --model qwen2.5vl:7b --force --apply
```

Dry run with JSON output:

```bash
maki describe "rating:4+" --dry-run --json
```

Use a cloud API (OpenAI-compatible endpoint):

```bash
maki describe ""
```

### SEE ALSO

[edit](#maki-edit) -- manually set or clear an asset's description.
[VLM Model Guide](10-vlm-models.md) -- tested models, backends, hardware guide, and quality comparison.
[Configuration](08-configuration.md#vlm-section) -- `[vlm]` section for endpoint, model, and prompt settings.
[VLM Setup (User Guide)](../user-guide/03-ingest.md#vlm-image-descriptions) -- how to set up a local VLM server.

---

Previous: [Setup Commands](01-setup-commands.md) -- `init`, `volume add`, `volume list`.
Next: [Organize Commands](03-organize-commands.md) -- `collection`, `saved-search`.
