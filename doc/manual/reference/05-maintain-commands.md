# Maintain Commands

Commands for integrity checks, disk reconciliation, file relocation, preview generation, and catalog housekeeping.

---

## dam verify

### NAME

dam-verify -- re-hash files on disk and compare against stored content hashes

### SYNOPSIS

```
dam [GLOBAL FLAGS] verify [PATHS...] [--max-age DAYS] [--force] [OPTIONS]
```

### DESCRIPTION

Verifies file integrity by re-hashing files on disk and comparing the computed SHA-256 hash against the stored content hash in the catalog. Detects corruption, bit rot, or unauthorized modification.

**Catalog mode** (no paths): Verifies all file locations on all online volumes. Checks whether cataloged files are intact, detecting OK, FAILED, MODIFIED, MISSING, and SKIPPED statuses. `--volume` limits to a specific volume; `--asset` limits to a specific asset.

**Path mode** (with paths): Scans files at the given paths on disk and looks them up in the catalog by content hash. In addition to the catalog mode statuses, can report UNTRACKED files — files on disk whose content hash does not match any known variant or recipe.

On successful verification, updates the `verified_at` timestamp on each file location record (persisted to both SQLite catalog and sidecar YAML for variant and recipe locations).

**Incremental verify**: Use `--max-age` to skip files that were verified recently. Only files whose `verified_at` timestamp is older than the given number of days (or never verified) are re-hashed. This enables fast periodic checks on large catalogs. `--force` overrides the skip and re-verifies everything. A default `max_age_days` can be set in `dam.toml` under `[verify]`.

**Result statuses:**

| Status | Description | Mode |
|--------|-------------|------|
| **OK** | File hash matches the catalog record. | Both |
| **FAILED** | Media file hash does not match (corruption or replacement). | Both |
| **MODIFIED** | Recipe file changed externally; hash updated in catalog. | Both |
| **MISSING** | Catalog location exists but file is gone from disk. | Catalog |
| **UNTRACKED** | File on disk not found in catalog. | Path |
| **SKIPPED** | Volume offline, path unreadable, or other error. | Both |

**Exit codes**: Exits with code 1 if any hash mismatches (FAILED) are found. Exits with code 0 for all other statuses, including MODIFIED.

**Recipe handling**: Recipe files (XMP, COS, etc.) that have been modified externally are reported as "modified" rather than "FAILED" and do not trigger exit code 1. Their stored hash is updated to reflect the new content.

Offline volumes are silently skipped.

### ARGUMENTS

**PATHS** (optional)
: One or more file paths or directories to verify. When omitted, verifies all file locations on online volumes.

### OPTIONS

**--volume \<LABEL\>**
: Limit verification to a specific volume.

**--asset \<ID\>**
: Verify only the file locations of a specific asset. Supports prefix matching.

**--max-age \<DAYS\>**
: Skip files verified within the given number of days. Enables incremental verification.

**--force**
: Override `--max-age` and re-verify all files.

**--include \<GROUP\>**
: Include additional file type groups. Can be specified multiple times.

**--skip \<GROUP\>**
: Skip file type groups. Can be specified multiple times.

`--json` outputs a `VerifyResult` with `verified`, `failed`, `modified`, `skipped`, `skipped_recent`, `missing` counters and detail arrays.

`--log` prints per-file verification status and timing to stderr.

### EXAMPLES

Verify the entire catalog:

```bash
dam verify
```

Verify a specific volume with progress logging:

```bash
dam verify --volume "Photos" --log --time
```

Verify a single asset:

```bash
dam verify --asset a1b2c3d4
```

Verify a specific directory:

```bash
dam verify /Volumes/Photos/Capture/2026-02-22
```

Incremental verify — skip files checked in the last 30 days:

```bash
dam verify --max-age 30
```

Force re-verify everything, ignoring recent timestamps:

```bash
dam verify --force --max-age 30
```

Verify and check for failures in a script:

```bash
if ! dam verify --volume "Archive"; then
  echo "Integrity check failed!"
fi
```

### SEE ALSO

[sync](#dam-sync) -- reconcile catalog with disk after files are moved or modified.
[cleanup](#dam-cleanup) -- remove stale location records for missing files.
[stats](04-retrieve-commands.md#dam-stats) -- `--verified` shows verification health overview.

---

## dam sync

### NAME

dam-sync -- reconcile catalog with disk changes

### SYNOPSIS

```
dam [GLOBAL FLAGS] sync <PATHS...> [OPTIONS]
```

### DESCRIPTION

Scans paths on disk and reconciles the catalog with the current disk state. Detects files that have been moved, renamed, modified, or deleted by external tools since the last import.

**Detected states**:

| State | Description |
|-------|-------------|
| **Unchanged** | File at expected path with matching hash. |
| **Moved** | Known hash found at a new path; old path is gone. |
| **New** | Unknown hash at a new path (not yet imported). |
| **Modified recipe** | Recipe file at same path but with a different hash. |
| **Missing** | Catalog location points to a file that no longer exists. |

Without `--apply`, runs in **report-only mode** (safe default) and shows what it found without making changes. With `--apply`, updates catalog and sidecar files for moved files and modified recipes. `--remove-stale` (requires `--apply`) removes catalog location records for confirmed-missing files.

New files are reported but not auto-imported -- run `dam import` separately to bring them into the catalog.

### ARGUMENTS

**PATHS** (required)
: One or more file paths or directories to scan.

### OPTIONS

**--volume \<LABEL\>**
: Use a specific volume instead of auto-detecting from the path.

**--apply**
: Apply changes to catalog and sidecar files. Without this flag, only reports what it found.

**--remove-stale**
: Remove catalog location records for missing files. Requires `--apply`.

`--json` outputs a `SyncResult` with counts and detail arrays for each state.

`--log` prints per-file status to stderr.

`--time` shows elapsed wall-clock time.

### EXAMPLES

Preview what sync would find (report-only):

```bash
dam sync /Volumes/Photos/Capture
```

Apply changes for moved and modified files:

```bash
dam sync /Volumes/Photos --apply --log
```

Apply changes and remove stale location records:

```bash
dam sync /Volumes/Photos --apply --remove-stale
```

Sync a specific volume:

```bash
dam sync /Volumes/Archive --volume "Archive" --apply
```

Sync with full diagnostics:

```bash
dam sync /Volumes/Photos --apply --log --time --json
```

### SEE ALSO

[refresh](#dam-refresh) -- re-read metadata from changed recipe files.
[cleanup](#dam-cleanup) -- remove stale records across all volumes.
[verify](#dam-verify) -- check file integrity without reconciliation.
[import](02-ingest-commands.md#dam-import) -- import new files discovered by sync.

---

## dam refresh

### NAME

dam-refresh -- re-read metadata from changed sidecar and recipe files

### SYNOPSIS

```
dam [GLOBAL FLAGS] refresh [PATHS...] [OPTIONS]
```

### DESCRIPTION

Checks recipe files for changes and re-extracts metadata when modifications are detected. For each recipe, compares the on-disk hash to the stored hash. If the file has changed, re-extracts XMP metadata (keywords, rating, description, color label) and updates catalog and sidecar.

This is useful for picking up changes made by external tools like CaptureOne or Lightroom that modify XMP sidecars outside of dam.

Without arguments, checks all recipe locations on all online volumes. With paths, scans recipe files under given paths. `--volume` limits to a specific volume; `--asset` limits to a specific asset's recipes.

**--media** additionally scans JPEG and TIFF variant files and re-extracts their embedded XMP metadata (keywords, rating, description, label, creator, rights). This is useful for retroactively extracting embedded XMP from files imported before the feature existed, or after external tools edit the embedded metadata.

Non-XMP recipes (COS, pp3, etc.) get their hash updated but no metadata extraction.

### ARGUMENTS

**PATHS** (optional)
: One or more file paths or directories to scan for recipe files. When omitted, checks all recipe locations on online volumes.

### OPTIONS

**--volume \<LABEL\>**
: Limit to a specific volume.

**--asset \<ID\>**
: Refresh only a specific asset's recipes. Supports prefix matching.

**--dry-run**
: Report what would change without applying updates.

**--media**
: Also re-extract embedded XMP from JPEG/TIFF media files (not just recipe files).

`--json` outputs a refresh result with changed/unchanged counts and detail arrays.

`--log` prints per-file status to stderr.

`--time` shows elapsed wall-clock time.

### EXAMPLES

Check all recipes for changes (report-only by default):

```bash
dam refresh --dry-run
```

Refresh all recipes on a specific volume:

```bash
dam refresh --volume "Photos" --log
```

Refresh recipes for a single asset:

```bash
dam refresh --asset a1b2c3d4
```

Refresh recipes and also re-extract embedded XMP from media files:

```bash
dam refresh --media --log --time
```

Refresh a specific directory after editing in CaptureOne:

```bash
dam refresh /Volumes/Photos/Capture/2026-02-22 --log
```

### SEE ALSO

[sync](#dam-sync) -- full reconciliation of catalog with disk.
[verify](#dam-verify) -- integrity checking without metadata re-extraction.
[import](02-ingest-commands.md#dam-import) -- initial import with XMP extraction.

---

## dam cleanup

### NAME

dam-cleanup -- remove stale location records, orphaned assets, and orphaned previews

### SYNOPSIS

```
dam [GLOBAL FLAGS] cleanup [OPTIONS]
```

### DESCRIPTION

Scans all file locations and recipes across online volumes, checking for files that no longer exist on disk. Performs three passes:

1. **Stale locations and recipes**: Removes catalog and sidecar records for files that are missing from disk.
2. **Orphaned assets**: Deletes assets where all variants have zero remaining file locations, along with their recipes, variants, catalog rows, and sidecar YAML files.
3. **Orphaned previews**: Removes preview files whose content hash no longer matches any variant in the catalog.

Without `--apply`, runs in **report-only mode** (safe default) and predicts what would be removed, including orphaned assets and previews that would result from removing stale locations.

Offline volumes are skipped with a note.

### ARGUMENTS

None.

### OPTIONS

**--volume \<LABEL\>**
: Limit stale-location scanning to a specific volume. When omitted, checks all online volumes.

**--list**
: Print stale entries to stderr (shows only stale items, unlike `--log` which prints all entries including OK ones).

**--apply**
: Apply changes: remove stale records, delete orphaned assets, and remove orphaned preview files.

`--json` outputs a `CleanupResult` with counts for stale locations, stale recipes, orphaned assets, and orphaned previews.

`--log` prints per-file status to stderr (both OK and stale entries).

`--time` shows elapsed wall-clock time.

### EXAMPLES

Preview what cleanup would remove:

```bash
dam cleanup
```

List only the stale entries:

```bash
dam cleanup --list
```

Apply cleanup across all volumes:

```bash
dam cleanup --apply --log
```

Cleanup a specific volume:

```bash
dam cleanup --volume "Photos" --apply
```

Cleanup with JSON output for scripting:

```bash
dam cleanup --apply --json | jq '{stale: .stale_locations, orphans: .orphaned_assets, previews: .orphaned_previews}'
```

### SEE ALSO

[sync](#dam-sync) -- reconcile individual paths (more targeted than cleanup).
[verify](#dam-verify) -- check integrity without removing records.
[search](04-retrieve-commands.md#dam-search) -- `orphan:true` filter finds assets with no file locations.

---

## dam dedup

### NAME

dam-dedup -- remove same-volume duplicate file locations

### SYNOPSIS

```
dam [GLOBAL FLAGS] dedup [OPTIONS]
```

### DESCRIPTION

Identifies variants with 2+ file locations on the **same** volume and removes the redundant copies. This targets accidental duplicates (e.g. files copied into multiple directories on the same drive) while leaving cross-volume copies untouched (those are intentional backups).

For each set of same-volume duplicate locations, a resolution heuristic selects which copy to **keep**:

1. If `--prefer` is given (or set in `[dedup] prefer` config), prefer locations whose relative path **contains** the specified string (substring match, not prefix-only).
2. Prefer more recently verified files (by `verified_at` timestamp; never-verified sorts oldest).
3. Prefer shorter relative paths (closer to the volume root).
4. Tiebreak: first alphabetically (deterministic).

Before removing a location, the command checks that the variant's total location count across **all** volumes won't drop below `--min-copies`.

When a file location is removed, co-located recipe files (XMP sidecars etc.) in the same directory are automatically cleaned up from disk, catalog, and sidecar YAML.

Without `--apply`, runs in **report-only mode** (safe default): shows what would be removed without making any changes. Recipe file counts are included in the dry-run report.

### ARGUMENTS

None.

### OPTIONS

**--volume \<LABEL\>**
: Limit deduplication to a specific volume. When omitted, processes same-volume duplicates on all volumes.

**--prefer \<STRING\>**
: Prefer keeping locations whose relative path contains this string (substring match). Useful for keeping files in a curated directory (e.g. `--prefer Selects`) while removing copies elsewhere. Falls back to the `[dedup] prefer` value in `dam.toml` when not given on the command line.

**--filter-format \<FORMAT\>**
: Filter to a specific file format (e.g. `nef`, `jpg`). Only processes duplicate groups matching this format.

**--path \<PREFIX\>**
: Filter to locations under this path prefix. Only processes duplicates with locations matching the prefix.

**--min-copies \<N\>** (default: 1)
: Minimum total locations to preserve per variant across all volumes. Prevents removing a location if it would leave fewer than N copies total. Set to 2 to ensure at least one backup copy survives.

**--apply**
: Apply changes: delete physical files and co-located recipe files from disk, remove location and recipe records from catalog and sidecar YAML.

`--json` outputs a `DedupResult` with `duplicates_found`, `locations_to_remove`, `locations_removed`, `files_deleted`, `recipes_removed`, `bytes_freed`, `dry_run`, and `errors`.

`--log` prints per-location status to stderr (keep, remove, skipped).

`--time` shows elapsed wall-clock time.

### EXAMPLES

Preview what dedup would remove:

```bash
dam dedup
```

Remove same-volume duplicates across all volumes:

```bash
dam dedup --apply --log
```

Dedup a specific volume, preferring files under `Selects/`:

```bash
dam dedup --volume "Photos" --prefer "Selects" --apply
```

Ensure at least 2 copies survive per variant:

```bash
dam dedup --min-copies 2 --apply
```

JSON output for scripting:

```bash
dam --json dedup --apply | jq '{groups: .duplicates_found, removed: .locations_removed, freed: .bytes_freed}'
```

### SEE ALSO

[duplicates](04-retrieve-commands.md#dam-duplicates) -- find duplicates without removing them.
[cleanup](#dam-cleanup) -- remove stale locations for files that no longer exist on disk.
[verify](#dam-verify) -- update `verified_at` timestamps used by the dedup heuristic.

---

## dam relocate

### NAME

dam-relocate -- copy or move asset files to another volume

### SYNOPSIS

```
dam [GLOBAL FLAGS] relocate <ASSET_ID> <VOLUME> [OPTIONS]
```

### DESCRIPTION

Copies all files belonging to an asset (variants and recipes) to a target volume. After copying, verifies file integrity via SHA-256 comparison. Preserves the relative path structure on the target volume.

Without `--remove-source`, files are copied and the asset gains additional file locations on the target volume. With `--remove-source`, source files are deleted after verified copy, effectively moving the asset.

Asset IDs support unique prefix matching.

### ARGUMENTS

**ASSET_ID** (required)
: The asset ID or a unique prefix of it.

**VOLUME** (required)
: Target volume label or UUID.

### OPTIONS

**--remove-source**
: Delete source files after successful copy and SHA-256 verification.

**--dry-run**
: Show what would happen without making any changes.

`--json` outputs a `RelocateResult` with details of copied/moved files.

### EXAMPLES

Copy an asset to an archive volume:

```bash
dam relocate a1b2c3d4 "Archive"
```

Move an asset (copy + delete source):

```bash
dam relocate a1b2c3d4 "Archive" --remove-source
```

Preview what would be relocated:

```bash
dam relocate a1b2c3d4 "Backup" --dry-run
```

Relocate with full diagnostics:

```bash
dam relocate a1b2c3d4 "Archive" --remove-source --log --time
```

### SEE ALSO

[update-location](#dam-update-location) -- update path after a manual move.
[verify](#dam-verify) -- verify file integrity after relocation.
[volume list](01-setup-commands.md#dam-volume-list) -- see available volumes.

---

## dam update-location

### NAME

dam-update-location -- update a file's catalog path after it was manually moved on disk

### SYNOPSIS

```
dam [GLOBAL FLAGS] update-location <ASSET_ID> --from <OLD_PATH> --to <NEW_PATH> [--volume <LABEL>]
```

### DESCRIPTION

Updates the catalog to reflect a file that was manually moved on disk (outside of dam). The file at the new path is verified to have the same content hash as the catalog record (safety check against accidental mismatches).

`--from` specifies the old path as recorded in the catalog (absolute or volume-relative). `--to` must be an absolute path to the file's current location on disk.

The volume is auto-detected from `--to` by matching against registered volume mount points, or can be specified explicitly with `--volume`.

Handles both variant file locations and recipe file locations.

Asset IDs support unique prefix matching.

### ARGUMENTS

**ASSET_ID** (required)
: The asset ID or a unique prefix of it.

### OPTIONS

**--from \<OLD_PATH\>** (required)
: The old path where the file was before (absolute or volume-relative).

**--to \<NEW_PATH\>** (required)
: The new absolute path where the file is now.

**--volume \<LABEL\>**
: Volume label or UUID. Auto-detected from `--to` if omitted.

`--json` outputs the updated location details.

### EXAMPLES

Update a file that was moved to a new directory:

```bash
dam update-location a1b2c3d4 \
  --from "Capture/2026-02-22/DSC_001.nef" \
  --to /Volumes/Photos/Processed/2026/DSC_001.nef
```

Update with explicit volume:

```bash
dam update-location a1b2c3d4 \
  --from /Volumes/OldDrive/Photos/IMG_001.jpg \
  --to /Volumes/NewDrive/Photos/IMG_001.jpg \
  --volume "NewDrive"
```

Update a recipe file location:

```bash
dam update-location a1b2c3d4 \
  --from "Capture/2026-02-22/DSC_001.xmp" \
  --to /Volumes/Photos/Processed/2026/DSC_001.xmp
```

### SEE ALSO

[relocate](#dam-relocate) -- copy or move files with dam managing the transfer.
[sync](#dam-sync) -- automatic detection of moved files.

---

## dam generate-previews

### NAME

dam-generate-previews -- generate or regenerate preview thumbnails

### SYNOPSIS

```
dam [GLOBAL FLAGS] generate-previews [PATHS...] [OPTIONS]
```

### DESCRIPTION

Generates preview thumbnails for assets. Standard image formats produce 800px JPEG thumbnails via the `image` crate. RAW files use `dcraw` or `dcraw_emu` (LibRaw). Videos use `ffmpeg`. Non-visual formats (audio, documents) get an info card showing file metadata.

**Without PATHS**: Iterates all catalog assets and generates previews for the best variant of each asset (Export > Processed > Original, standard image formats preferred over RAW, file size tiebreak). Can be filtered with `--asset` or `--volume`.

**With PATHS**: Resolves files on disk and looks up their variants in the catalog, generating previews only for those specific files.

Previews are stored in `previews/<hash-prefix>/<hash>.jpg`. Preview settings (max edge size, format, quality) are configured in the `[preview]` section of `dam.toml`.

### ARGUMENTS

**PATHS** (optional)
: One or more files or directories to generate previews for.

### OPTIONS

**--volume \<LABEL\>**
: Limit to variants on a specific volume.

**--asset \<ID\>**
: Generate preview only for a specific asset. Supports prefix matching.

**--include \<GROUP\>**
: Include additional file type groups. Can be specified multiple times.

**--skip \<GROUP\>**
: Skip file type groups. Can be specified multiple times.

**--force**
: Regenerate previews even if they already exist.

**--upgrade**
: Regenerate previews for assets where a better variant (export or processed) exists than what the current preview was generated from. Skips assets where the best variant is already the source of the preview.

`--json` outputs a result with generated/skipped/failed counts.

`--log` prints per-file generation status to stderr.

`--debug` shows stderr output from external tools (dcraw, ffmpeg) for diagnosing failures.

### EXAMPLES

Generate all missing previews:

```bash
dam generate-previews
```

Regenerate all previews (force):

```bash
dam generate-previews --force --log --time
```

Upgrade previews to use better variants (e.g., after grouping exports with originals):

```bash
dam generate-previews --upgrade --log
```

Generate preview for a single asset:

```bash
dam generate-previews --asset a1b2c3d4
```

Generate previews for files in a specific directory:

```bash
dam generate-previews /Volumes/Photos/Capture/2026-02-22
```

Generate previews with debug output for troubleshooting RAW files:

```bash
dam generate-previews --force --debug --asset a1b2c3d4
```

### SEE ALSO

[import](02-ingest-commands.md#dam-import) -- previews are generated automatically during import.
[serve](04-retrieve-commands.md#dam-serve) -- web UI displays preview thumbnails.

---

## dam fix-roles

### NAME

dam-fix-roles -- fix variant roles in multi-variant assets

### SYNOPSIS

```
dam [GLOBAL FLAGS] fix-roles [PATHS...] [OPTIONS]
```

### DESCRIPTION

Scans assets that contain both RAW and non-RAW variants and re-roles non-RAW variants from `original` to `export`. This corrects role assignments that may have been missed during import or grouping.

In a properly organized asset, only the RAW file should have the `original` role. Non-RAW files (JPEG, TIFF, etc.) in the same asset should be `export` or `processed`.

Without `--apply`, runs in **report-only mode** and shows what roles would change. With `--apply`, updates both YAML sidecar files and the SQLite catalog.

### ARGUMENTS

**PATHS** (optional)
: Files or directories to scope the fix. When omitted, checks all assets.

### OPTIONS

**--volume \<LABEL\>**
: Limit to a specific volume.

**--asset \<ID\>**
: Fix only a specific asset. Supports prefix matching.

**--apply**
: Apply the role changes. Without this flag, only reports what would change.

### EXAMPLES

Preview what roles would change:

```bash
dam fix-roles
```

Apply role fixes:

```bash
dam fix-roles --apply --log
```

Fix roles for a single asset:

```bash
dam fix-roles --asset a1b2c3d4 --apply
```

Fix roles on a specific volume:

```bash
dam fix-roles --volume "Photos" --apply --log --time
```

### SEE ALSO

[group](02-ingest-commands.md#dam-group) -- grouping merges variants and adjusts roles.
[auto-group](02-ingest-commands.md#dam-auto-group) -- automatic grouping with role adjustment.
[show](04-retrieve-commands.md#dam-show) -- inspect current variant roles.

---

## dam fix-dates

### NAME

dam-fix-dates -- fix asset dates from variant EXIF metadata and file modification times

### SYNOPSIS

```
dam [GLOBAL FLAGS] fix-dates [OPTIONS]
```

### DESCRIPTION

Scans assets and corrects their `created_at` date by examining the EXIF DateTimeOriginal metadata and file modification times of their variants. This fixes assets that were imported with the wrong date (e.g., import timestamp instead of capture date).

For each asset, the command collects candidate dates from all variants:

1. **Stored EXIF date**: The `date_taken` field in the variant's `source_metadata` (stored since v1.3.1).
2. **Re-extracted EXIF**: For variants imported before `date_taken` was stored, re-reads the file on disk and extracts EXIF DateTimeOriginal. Requires the volume to be online.
3. **File modification time**: The filesystem mtime of the variant file on disk. Requires the volume to be online.

The oldest date found across all variants is used as the corrected `created_at`. A 1-second tolerance is applied when comparing to the current date (to handle rounding).

Without `--apply`, runs in **report-only mode** and shows what dates would change. With `--apply`, updates both YAML sidecar files and the SQLite catalog. When applying, also backfills `date_taken` into variant `source_metadata` so future runs work from metadata alone without needing the volume online.

**Offline volume handling**: Assets whose only file locations are on offline volumes cannot be fixed (no file access for EXIF re-extraction or mtime). These are counted separately as "skipped (volume offline)" and a warning is printed for each offline volume. Mount the volume and re-run to fix these assets.

### ARGUMENTS

None.

### OPTIONS

**--volume \<LABEL\>**
: Limit to assets with locations on a specific volume.

**--asset \<ID\>**
: Fix only a specific asset. Supports prefix matching.

**--apply**
: Apply date corrections. Without this flag, only reports what would change.

`--json` outputs a `FixDatesResult` with `checked`, `fixed`, `already_correct`, `no_date`, `skipped_offline` counters and `offline_volumes` list.

`--log` prints per-asset status to stderr.

`--time` shows elapsed wall-clock time.

### EXAMPLES

Preview what dates would be corrected:

```bash
dam fix-dates
```

Preview with per-asset detail:

```bash
dam fix-dates --log
```

Apply date corrections:

```bash
dam fix-dates --apply --log
```

Fix dates for a specific volume:

```bash
dam fix-dates --volume "Photos 2024" --apply --log
```

Fix a single asset:

```bash
dam fix-dates --asset a1b2c3d4 --apply
```

### SEE ALSO

[fix-roles](#dam-fix-roles) -- fix variant roles in multi-variant assets.
[fix-recipes](#dam-fix-recipes) -- re-attach recipe files imported as standalone assets.
[refresh](#dam-refresh) -- re-read metadata from changed recipe files.
[import](02-ingest-commands.md#dam-import) -- import now uses EXIF date → file mtime → current time fallback chain.

---

## dam fix-recipes

### NAME

dam-fix-recipes -- re-attach recipe files that were imported as standalone assets

### SYNOPSIS

```
dam [GLOBAL FLAGS] fix-recipes [OPTIONS]
```

### DESCRIPTION

Finds assets that consist of a single variant with a recipe-type extension (xmp, cos, cot, cop, pp3, dop, on1) and `asset_type = other`, then tries to re-attach them as recipe records on a matching parent variant.

This fixes a situation where recipe files were imported before their corresponding media files, or when a media format wasn't recognized at import time (e.g., NRW before extension support was added). In both cases, the recipe file ends up as a standalone asset instead of being attached to the media asset.

**Matching logic**: For each standalone recipe asset, the command extracts the filename stem and directory from the recipe's file location, then searches for a media variant with the same stem in the same directory on the same volume. Compound extensions are handled: if `DSC_001.NRW.xmp` doesn't match directly (stem = `DSC_001.NRW`), the last extension is stripped and `DSC_001` is tried as the stem.

When a parent is found and `--apply` is specified:

1. A `Recipe` record is created on the parent variant.
2. For XMP files, metadata (keywords, rating, description, color label) is extracted and applied to the parent asset.
3. The parent asset's denormalized columns are updated.
4. The standalone recipe asset is fully deleted (recipes, file locations, variants, asset row, and sidecar YAML).

Without `--apply`, runs in **report-only mode** and shows what would change.

### ARGUMENTS

None.

### OPTIONS

**--volume \<LABEL\>**
: Limit to assets with locations on a specific volume.

**--asset \<ID\>**
: Fix only a specific asset. Supports prefix matching.

**--apply**
: Apply the changes. Without this flag, only reports what would change.

`--json` outputs a `FixRecipesResult` with `checked`, `reattached`, `no_parent`, `skipped` counters.

`--log` prints per-asset status to stderr.

`--time` shows elapsed wall-clock time.

### EXAMPLES

Preview what would be reattached:

```bash
dam fix-recipes
```

Preview with per-asset detail:

```bash
dam fix-recipes --log
```

Apply the fixes:

```bash
dam fix-recipes --apply --log
```

Fix a specific asset:

```bash
dam fix-recipes --asset a1b2c3d4 --apply
```

Fix recipes on a specific volume:

```bash
dam fix-recipes --volume "Photos" --apply --log --time
```

### SEE ALSO

[fix-roles](#dam-fix-roles) -- fix variant roles in multi-variant assets.
[fix-dates](#dam-fix-dates) -- fix asset dates from EXIF metadata.
[refresh](#dam-refresh) -- re-read metadata from changed recipe files.
[import](02-ingest-commands.md#dam-import) -- import with recipe attachment logic.

---

## dam rebuild-catalog

### NAME

dam-rebuild-catalog -- rebuild the SQLite catalog from YAML sidecar files

### SYNOPSIS

```
dam [GLOBAL FLAGS] rebuild-catalog
```

### DESCRIPTION

Rebuilds the SQLite catalog database entirely from the YAML sidecar files in the `metadata/` directory. The sidecar files are the source of truth for all asset metadata; the SQLite database is a derived cache for fast queries.

This command is useful when:

- The SQLite database is corrupted or deleted.
- Schema changes require a full rebuild.
- The catalog needs to be verified against sidecar files.

After rebuilding, all denormalized columns (best variant hash, primary format, variant count) are recomputed. Collections are preserved via `collections.yaml`. Stacks are restored from `stacks.yaml`, re-populating the `stacks` table and the `stack_id`/`stack_position` columns on the `assets` table.

### ARGUMENTS

None.

### OPTIONS

This command only accepts [global flags](00-cli-conventions.md#global-flags).

### EXAMPLES

Rebuild the catalog:

```bash
dam rebuild-catalog
```

Rebuild with timing:

```bash
dam rebuild-catalog --time
```

Rebuild with progress logging:

```bash
dam rebuild-catalog --log --time
```

### SEE ALSO

[init](01-setup-commands.md#dam-init) -- initial catalog creation.
[stats](04-retrieve-commands.md#dam-stats) -- verify catalog statistics after rebuild.
[verify](#dam-verify) -- verify file integrity after rebuild.

---

Previous: [Retrieve Commands](04-retrieve-commands.md) -- `search`, `show`, `duplicates`, `stats`, `serve`.
