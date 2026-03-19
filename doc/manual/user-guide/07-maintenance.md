# Maintenance

Over time, files move, drives get swapped, external tools edit recipes, and storage devices accumulate stale references. This chapter covers the commands that keep your catalog accurate and your files healthy.

The six core maintenance commands form a cycle:

```mermaid
flowchart LR
    V["maki verify<br/>(detect corruption)"]
    S["maki sync<br/>(reconcile moved/<br/>modified files)"]
    SM["maki sync-metadata<br/>(bidirectional<br/>XMP sync)"]
    R["maki refresh<br/>(re-read changed<br/>recipes)"]
    C["maki cleanup<br/>(remove stale<br/>records)"]

    V --> S --> SM --> R --> C --> V
```

`maki sync-metadata` replaces the separate `writeback` + `refresh` steps for most workflows — it handles both directions in a single command and detects conflicts.

Each command is safe by default -- destructive operations require an explicit `--apply` flag, and most commands support `--dry-run` or report-only mode.


## Verification

`maki verify` re-hashes every file on disk and compares the result to the content hash stored in the catalog. This detects silent corruption, bit rot, and accidental modifications.

### Verify everything

```bash
maki verify
```

This walks all file locations on all online volumes. Offline volumes are skipped automatically.

Sample output:

```
Verify complete: 1847 verified, 2 modified, 0 FAILED, 3 skipped
```

### Verify specific files or directories

```bash
maki verify /Volumes/PhotosDrive/Capture/2026-02-01
```

Only files under the given path are checked.

### Limit to a volume

```bash
maki verify --volume "Photos 2024"
```

Useful when you reconnect a drive and want to spot-check it before trusting its contents.

### Verify a single asset

```bash
maki verify --asset a1b2c3d4
```

Asset IDs can be abbreviated to a unique prefix.

### Filter by file type

```bash
# Only verify RAW files
maki verify --include raw

# Skip audio files
maki verify --skip audio
```

### Two verification modes

**Catalog mode** (no paths): `maki verify` walks all file locations known to the catalog on online volumes. This checks whether your cataloged files are intact.

**Path mode** (with paths): `maki verify /some/path` scans files on disk and looks them up in the catalog. This also detects files that are not in the catalog at all.

### What the results mean

- **verified** -- file hash matches the catalog. The `verified_at` timestamp is updated.
- **modified** -- a recipe file (`.xmp`, `.cos`, etc.) was changed externally. maki updates the stored hash and reports it as "modified" rather than "FAILED". This is expected when CaptureOne or Lightroom edits a sidecar.
- **FAILED** -- a media file's hash does not match. This indicates corruption, accidental overwrite, or a file that was replaced. Investigate immediately.
- **MISSING** -- a file referenced in the catalog no longer exists on disk (catalog mode only). The file's location record exists but the file is gone from an online volume.
- **UNTRACKED** -- a file was found on disk but is not in the catalog (path mode only). The file's content hash does not match any known variant or recipe. Run `maki import` to bring it into the catalog, or ignore it if it is not a media file.
- **skipped** -- the file is on an offline volume, or the path could not be read.

If any files fail verification (FAILED status), maki exits with code 1. Scripts can check `$?` to detect problems:

```bash
maki verify --volume "Archive" || echo "Integrity check failed!"
```

### Monitoring flags

```bash
# Per-file progress to stderr
maki verify --log

# Machine-readable output
maki verify --json

# Show elapsed time
maki verify --time

# Combine them
maki verify --volume "Photos 2024" --log --time
```

### Checking verification age

Use the `stale:N` search filter to find assets that have not been verified recently:

```bash
# Assets not verified in the last 30 days
maki search "stale:30"

# Assets never verified
maki search "stale:0"
```

See [Browsing & Searching](05-browse-and-search.md) for more on search filters.


## Sync

`maki sync` reconciles the catalog with what is actually on disk. Run it after moving, renaming, or deleting files with external tools (Finder, `mv`, CaptureOne's "move to folder", etc.).

Unlike `verify` (which only checks hashes), `sync` scans the filesystem for structural changes: files that moved, new files that appeared, and files that disappeared.

### Report mode (safe default)

```bash
maki sync /Volumes/PhotosDrive/Photos/
```

Without `--apply`, sync scans the directory and reports what it finds, but changes nothing:

```
Sync complete: 1200 unchanged, 3 moved, 2 new, 1 modified, 4 missing
  Tip: run 'maki import' to import new files.
```

### Apply changes

```bash
maki sync /Volumes/PhotosDrive/Photos/ --apply
```

With `--apply`, sync updates the catalog and sidecar files:

- **Moved files**: The catalog path is updated to the new location (same content hash found at a different path, old path gone).
- **Modified recipes**: Recipe hash is updated, and if it is an XMP file, metadata is re-extracted.
- **Missing files**: Reported but not removed (use `--remove-stale`).
- **New files**: Reported but not imported. Run `maki import` separately.

### Removing stale records

```bash
maki sync /Volumes/PhotosDrive/Photos/ --apply --remove-stale
```

`--remove-stale` (which requires `--apply`) removes catalog location records for files that are confirmed missing. Use this when you intentionally deleted files and want the catalog to reflect that.

### Scoping to a volume

```bash
maki sync /Volumes/PhotosDrive/ --volume "Photos 2024"
```

Explicitly sets the volume context when auto-detection picks the wrong one.

### Detection categories

| Status | Meaning | Action with `--apply` |
|--------|---------|----------------------|
| unchanged | Hash matches at expected path | None |
| moved | Known hash found at new path, old path gone | Path updated in catalog |
| new | Unknown hash, not in catalog | Reported (run `maki import`) |
| modified | Same path, different hash (recipe files) | Hash updated, XMP re-extracted |
| missing | Catalog location exists but file is gone | Reported, or removed with `--remove-stale` |

### Monitoring flags

```bash
maki sync /Volumes/PhotosDrive/ --apply --log --time --json
```

`--log` shows per-file status, `--time` shows elapsed time, `--json` outputs structured results.


## Refresh

`maki refresh` re-reads metadata from recipe and media files without scanning the full filesystem. It is lighter than `sync` -- it only checks files the catalog already knows about, comparing their on-disk hash to the stored hash.

This is the right tool after editing in CaptureOne, Lightroom, or any other tool that modifies XMP sidecars.

### Refresh all recipes

```bash
maki refresh
```

Checks every recipe file location on all online volumes. For each recipe whose hash has changed:

- **XMP recipes** (`.xmp`): re-extracts keywords, rating, description, and color label, then updates the catalog and sidecar YAML.
- **Non-XMP recipes** (`.cos`, `.pp3`, `.dop`, etc.): hash updated, but no metadata extraction (these formats are opaque to maki).

### Refresh specific paths

```bash
maki refresh /Volumes/PhotosDrive/Capture/2026-02-01/
```

Only checks recipe files under the given path.

### Limit to a volume or asset

```bash
# All recipes on a specific volume
maki refresh --volume "Photos 2024"

# Only recipes for a specific asset
maki refresh --asset a1b2c3d4
```

### Re-extract embedded XMP from media files

```bash
maki refresh --media
```

The `--media` flag also scans JPEG and TIFF variant files, re-extracting embedded XMP metadata. This is useful in two scenarios:

1. **Retroactive extraction**: You imported files before the embedded XMP feature existed and want to pick up keywords/ratings/labels that were embedded all along.
2. **External edits**: A tool like CaptureOne or Lightroom modified the embedded XMP in a JPEG/TIFF file.

### Dry run

```bash
maki refresh --dry-run
```

Shows what would change without applying anything. Combine with `--log` for detailed output:

```bash
maki refresh --dry-run --log
```

Sample output:

```
  DSC_001.xmp — changed (12ms)
  DSC_002.xmp — unchanged (3ms)
  DSC_003.xmp — changed (11ms)
Dry run — Refresh complete: 2 refreshed, 1 unchanged, 0 missing, 0 skipped (offline)
```

### Monitoring flags

```bash
maki refresh --log --time --json
```


## Write Back

`maki writeback` replays pending metadata writes to XMP recipe files. When you edit metadata (rating, label, tags, description) while a volume is offline, the XMP write-back is skipped and the recipe is marked with a `pending_writeback` flag. The edits are safe in the YAML sidecar and SQLite catalog, but the `.xmp` files on disk still have old values. This command pushes those pending changes to XMP when the volume comes back online.

### Process pending write-backs

```bash
maki writeback
```

Without flags, only recipes with `pending_writeback=1` are processed. Each recipe's XMP file is updated with the current asset metadata (rating, label, tags, description), then re-hashed and the pending flag is cleared.

### Write back all XMP recipes

```bash
maki writeback --all
```

The `--all` flag writes current metadata to every XMP recipe, not just pending ones. Useful for an initial sync or to force-push all DAM metadata to XMP files.

### Scope to a volume or asset

```bash
# Only write back to recipes on a specific volume
maki writeback --volume "Photos 2024"

# Only write back for a specific asset
maki writeback --asset a1b2c3d4
```

### Dry run

```bash
maki writeback --dry-run
```

Shows what would be written without modifying any files. Combine with `--log` for detailed output.

### How pending tracking works

When any metadata edit (CLI `maki edit`, `maki tag`, web UI stars/labels/tags/description) triggers XMP write-back, each recipe is checked:

- **Volume online, file exists**: XMP is written, `pending_writeback` stays 0.
- **Volume offline or file missing**: `pending_writeback` is set to 1. The flag persists until `maki writeback` clears it.

The flag records the *intent* to write back, not *what* changed. When writeback runs, it reads the current asset metadata and writes all four fields (rating, label, tags, description) to the XMP file.

### Recommended workflow: volume comes back online

```bash
# 1. Push DAM edits to XMP (DAM wins for fields edited while offline)
maki writeback --volume "Archive 2025"

# 2. Pull any CaptureOne/Lightroom edits from XMP
maki refresh --volume "Archive 2025"
```

Order matters: writeback first ensures DAM edits land in the XMP files. Then refresh picks up anything the external tool changed independently.

### Monitoring flags

```bash
maki writeback --log --time --json
```


## Sync Metadata

`maki sync-metadata` performs bidirectional XMP metadata sync in a single command — combining the inbound (refresh) and outbound (writeback) steps with conflict detection.

### Basic usage

```bash
maki sync-metadata
```

This runs three phases:

1. **Inbound**: Detects externally modified XMP recipe files and re-reads their metadata (keywords, rating, description, color label).
2. **Outbound**: Finds recipes marked `pending_writeback` and writes current DAM metadata back to the XMP file.
3. **Conflict detection**: When both the XMP file changed on disk AND the recipe has pending DAM edits, the recipe is reported as a conflict and skipped.

### Scope to a volume or asset

```bash
maki sync-metadata --volume "Photos 2024"
maki sync-metadata --asset a1b2c3d4
```

### Include embedded XMP

```bash
maki sync-metadata --media
```

The `--media` flag adds a third phase that re-extracts embedded XMP from JPEG/TIFF variant files — useful after external tools modify embedded metadata.

### Dry run

```bash
maki sync-metadata --dry-run --log
```

Shows what would change without modifying any files.

### When to use sync-metadata vs. writeback + refresh

- **`sync-metadata`**: The recommended single command for most workflows. Handles both directions and detects conflicts.
- **`writeback` + `refresh`**: Use separately when you want explicit control over direction (e.g., force DAM edits to win with `writeback --all`, then pull external changes with `refresh`).


## Cleanup

`maki cleanup` scans all file locations and recipes across online volumes, checking whether the referenced files still exist on disk. It removes stale records, locationless variants, and orphaned derived files in eight passes.

### Report mode (safe default)

```bash
maki cleanup
```

Without `--apply`, cleanup reports what it finds:

```
Cleanup complete: 1500 checked, 12 stale, 3 orphaned assets, 5 orphaned previews, 2 orphaned embeddings
  Run with --apply to remove stale records and orphaned files.
```

### Apply cleanup

```bash
maki cleanup --apply
```

The eight passes:

1. **Stale location and recipe records**: Removes catalog entries for files that no longer exist on disk (variant file locations and recipe file locations). Updates sidecar YAML files accordingly.
2. **Locationless variants**: Removes variants with zero remaining file locations from assets that still have other located variants. Prevents ghost variants from accumulating after file moves or reimports.
3. **Orphaned assets**: Deletes assets where all variants have zero file locations remaining. Their recipes, variants, faces, embeddings, previews, smart previews, face crops, embedding binaries, catalog rows, and sidecar YAML files are removed.
4. **Orphaned previews**: Removes preview JPEG files whose content hash no longer matches any variant in the catalog.
5. **Orphaned smart previews**: Same for the `smart_previews/` directory.
6. **Orphaned embeddings**: Removes SigLIP embedding binaries whose asset ID no longer exists.
7. **Orphaned face crops**: Removes face crop thumbnails whose face ID no longer exists.
8. **Orphaned ArcFace embeddings**: Removes face embedding binaries whose face ID no longer exists.

### Limit to a specific volume

```bash
maki cleanup --volume "Photos 2024" --apply
```

Only scans file locations on the specified volume. Useful after removing a drive's contents intentionally.

### Limit to a specific path

```bash
maki cleanup --path "Capture/2026-02" --apply
maki cleanup --volume "Photos" --path "Archive/Old" --apply --log
```

Scopes stale-location scanning to files under a path prefix. Absolute paths are auto-detected to extract the volume and relative prefix.

### List stale entries

```bash
maki cleanup --list
```

Prints stale entries to stderr (similar to `--log`, but only shows stale entries rather than every file checked).

### Offline volumes

Offline volumes are skipped automatically with a note. maki never removes records for files on an offline volume -- it cannot know whether the file is truly gone or just disconnected.

### Monitoring flags

```bash
maki cleanup --apply --log --time --json
```


## Relocating Assets

`maki relocate` copies (or moves) all of an asset's files -- variants and recipes -- to another volume. Use this to migrate assets between drives, create backups on a second volume, or consolidate scattered files.

### Copy to another volume

```bash
maki relocate a1b2c3d4 "Archive Drive"
```

This copies all files for the asset to the target volume, preserving their relative paths. The asset now has files on both volumes. After the copy, each file is verified via SHA-256 to ensure integrity.

### Move (copy + delete source)

```bash
maki relocate a1b2c3d4 "Archive Drive" --remove-source
```

With `--remove-source`, the source files are deleted after successful copy and verification. The asset's locations are updated to point to the new volume only.

### Dry run

```bash
maki relocate a1b2c3d4 "Archive Drive" --dry-run
```

Shows what would be copied or moved without making any changes:

```
Dry run — no changes made:
  Copy Capture/2026-02-01/DSC_001.nef → Archive Drive:Capture/2026-02-01/DSC_001.nef
  Copy Capture/2026-02-01/DSC_001.xmp → Archive Drive:Capture/2026-02-01/DSC_001.xmp
```


## Updating File Locations

`maki update-location` fixes the catalog after you manually moved a single file on disk. Unlike `sync` (which scans a directory), this command targets one specific file.

### Basic usage

```bash
maki update-location a1b2c3d4 \
    --from Capture/2026-02-01/DSC_001.nef \
    --to /Volumes/PhotosDrive/Archive/2026/DSC_001.nef
```

- `--to` must be an absolute path to the file's current location on disk.
- `--from` can be absolute or volume-relative (the path as it appears in the catalog).

### How it works

1. maki resolves the asset by ID (or unique prefix).
2. The volume is auto-detected from `--to` by matching against registered volume mount points. You can override with `--volume`.
3. maki hashes the file at `--to` and compares it to the stored content hash. If they do not match, the command fails (safety check -- you may have pointed to the wrong file).
4. The catalog and sidecar YAML are updated with the new path.

### Specifying the volume explicitly

```bash
maki update-location a1b2c3d4 \
    --from Capture/old/DSC_001.nef \
    --to /Volumes/NewDrive/Capture/new/DSC_001.nef \
    --volume "New Drive"
```

This also handles recipe file locations, not just variant files.


## Rebuilding the Catalog

`maki rebuild-catalog` wipes the SQLite database and reconstructs it from the YAML sidecar files, which are the source of truth.

```bash
maki rebuild-catalog
```

Sample output:

```
Rebuild complete: 1847 assets, 2914 variants, 312 recipes, 3 collections, 45 stacks
```

### When to use it

- The SQLite database is corrupted or deleted.
- You manually edited sidecar YAML files and want the catalog to reflect those changes.
- Something seems out of sync and you want a clean slate.

### What is preserved

- **Sidecar YAML files**: Untouched (they are the source, not the target).
- **Collections**: Restored from `collections.yaml` at the catalog root.
- **Stacks**: Restored from `stacks.yaml` at the catalog root (member order and pick assignments are preserved).
- **Faces and people** *(ai feature)*: Restored from `faces.yaml` and `people.yaml`. ArcFace face embeddings restored from binary files in `embeddings/arcface/`.
- **Image embeddings** *(ai feature)*: SigLIP embeddings restored from binary files in `embeddings/<model>/`.
- **Preview files**: Untouched (they are content-addressed by hash).
- **Volumes**: Re-registered from `volumes.yaml`.

### What is regenerated

- All SQLite tables (assets, variants, recipes, file locations, tags, stacks).
- Denormalized columns (`best_variant_hash`, `primary_variant_format`, `variant_count`).
- Collection membership records (from `collections.yaml`).
- Stack membership records (from `stacks.yaml`).

This is a safe operation -- only the derived cache (SQLite) is rebuilt. No files on disk are modified or deleted.


## Fix Dates

`maki fix-dates` corrects asset dates that were set incorrectly during import. This commonly happens when files lack EXIF metadata (e.g., old JPEGs without DateTimeOriginal) — the import timestamp was used instead of the capture date.

### How it works

For each asset, `fix-dates` collects candidate dates from all variants:

1. **EXIF DateTimeOriginal** from stored `source_metadata` (for assets imported since v1.3.1)
2. **Re-extracted EXIF** from the file on disk (for older assets without stored date)
3. **File modification time** on disk

The oldest date across all variants becomes the corrected `created_at`.

### Report mode (safe default)

```bash
maki fix-dates
```

Shows what would be changed:

```
Warning: volume 'Archive' is offline — cannot read files for date extraction
Fix-dates: 5000 checked, 3200 fixed, 800 already correct, 1000 skipped (volume offline)
  Run with --apply to make changes.
  Mount offline volumes and re-run to fix remaining assets.
```

### Apply fixes

```bash
maki fix-dates --apply --log
```

Updates both the SQLite catalog and sidecar YAML files. Also backfills EXIF dates into variant metadata so future runs work without needing the volume online.

### Offline volumes

`fix-dates` needs file access for EXIF re-extraction and file modification times. Assets on offline volumes are skipped with a clear message. Mount the volume and re-run to fix them.

### Scope to a volume or asset

```bash
# Fix dates for assets on a specific volume
maki fix-dates --volume "Photos 2024" --apply --log

# Fix a single asset
maki fix-dates --asset a1b2c3d4 --apply
```


## Fix Roles

`maki fix-roles` corrects variant roles in assets that have both RAW and non-RAW variants. In these groups, the RAW variant should be the Original and non-RAW variants should be Exports. If roles are incorrect (e.g., both marked as Original), this command fixes them.

### Report mode (safe default)

```bash
maki fix-roles
```

Shows what would be changed:

```
Dry run — Fix-roles: 150 checked, 3 fixed (5 variant(s)), 147 already correct
  Run with --apply to make changes.
```

### Apply fixes

```bash
maki fix-roles --apply
```

Updates both the SQLite catalog and sidecar YAML files.

### Scope to a volume or asset

```bash
# Only check assets on a specific volume
maki fix-roles --volume "Photos 2024" --apply

# Fix a single asset
maki fix-roles --asset a1b2c3d4 --apply
```


## Recommended Maintenance Workflow

A practical schedule for keeping your catalog healthy:

### 1. Periodic integrity checks

Run `maki verify` on a regular schedule -- weekly for active volumes, monthly for archives:

```bash
# Verify the active working drive
maki verify --volume "Work SSD"

# Find assets that haven't been verified in 90 days
maki search "stale:90"
```

### 2. After editing in external tools

When you finish a session in CaptureOne, Lightroom, or another tool that edits XMP sidecars, run `maki refresh` to pick up the changes:

```bash
maki refresh --volume "Work SSD" --log
```

If the external tool also modified embedded metadata in JPEG/TIFF exports:

```bash
maki refresh --media --volume "Work SSD"
```

### 3. After moving or reorganizing files

If you moved files on disk (renamed directories, reorganized folder structure), run `maki sync` to reconcile:

```bash
# First, see what changed
maki sync /Volumes/PhotosDrive/Photos/

# Then apply
maki sync /Volumes/PhotosDrive/Photos/ --apply
```

If new files appeared (e.g., CaptureOne exported new TIFFs), import them:

```bash
maki import /Volumes/PhotosDrive/Photos/Exports/
```

### 4. Periodic cleanup

Run `maki cleanup` periodically to remove stale records for files that no longer exist:

```bash
# See what would be cleaned up
maki cleanup --list

# Apply
maki cleanup --apply
```

### 5. Nuclear option: rebuild

If the catalog seems fundamentally out of sync -- searches return wrong results, show displays stale data -- rebuild from the sidecar files:

```bash
maki rebuild-catalog
```

This is safe and fast. The sidecar YAML files are the source of truth; the SQLite database is just a derived cache.

### Putting it all together

A typical maintenance session after reconnecting an archive drive:

```bash
# 1. Check file integrity
maki verify --volume "Archive 2025" --time

# 2. Push any DAM edits made while the drive was offline
maki writeback --volume "Archive 2025"

# 3. Pick up any recipe changes made while the drive was connected elsewhere
maki refresh --volume "Archive 2025"

# 4. Reconcile any moved/renamed files
maki sync /Volumes/Archive2025/ --apply

# 5. Clean up any stale records
maki cleanup --volume "Archive 2025" --apply

# 6. Confirm everything looks good
maki stats --volumes
```

---

Next: [Scripting](08-scripting.md) -- shell and Python scripting patterns for workflow automation.

For complete flag and option details on every maintenance command, see the [Maintain Commands Reference](../reference/05-maintain-commands.md).
