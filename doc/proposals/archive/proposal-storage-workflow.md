# Proposal: Storage Workflow — Duplicate Management and Backup Coverage

## Motivation

A typical photography workflow creates multiple copies of the same file at different locations — some intentional, some accidental:

**Accidental duplicates** (unwanted):
- RAW file copied (instead of moved) from `Capture/` to `Selects/` within a CaptureOne session
- Same session folder imported twice from different paths
- Re-import after reorganizing files on the same drive

**Intentional copies** (wanted backups):
- rsync from laptop SSD to master media drive
- Periodic backup from master drive to backup drives
- Dropbox cloud sync of the working session

The current system already tracks all of these as multiple `file_locations` on a single variant (content-addressed by SHA-256). `maki duplicates` lists them. But there's no way to:

1. **Distinguish unwanted duplicates from wanted backups** — a RAW file at two paths on the same laptop is different from the same RAW on the laptop and on a backup drive
2. **Find backup gaps** — "Which rated assets have no copy on my master drive?"
3. **Clean up unwanted duplicates safely** — remove the redundant local copy while preserving backup copies
4. **Get an at-a-glance storage health overview** — how well-backed-up is my catalog?

---

## Design Principle: Volume Purpose, Not Variant Role

A "Backup" variant role was considered and rejected. The variant role describes *what the content is* (Original, Processed, Export), not *where it lives*. A RAW file is an Original regardless of whether it's on the working drive or a backup drive. Adding a "Backup" role would:

- Break display logic (Export > Processed > Original priority)
- Require re-roling variants when drives change purpose
- Conflate content semantics with storage semantics

The right abstraction is **volume purpose** — metadata on the volume itself that describes its role in the storage hierarchy. This keeps the variant model clean while enabling smart duplicate analysis.

---

## Part 1: Volume Purpose ✅ *Implemented in v1.4.0*

### Data Model

Add an optional `purpose` field to `Volume`:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VolumePurpose {
    Working,   // Active editing — laptop SSD, desktop drive
    Archive,   // Long-term primary storage — master media drive
    Backup,    // Redundancy copy — backup drives, NAS
    Cloud,     // Cloud sync folder — Dropbox, iCloud, Google Drive
}
```

```rust
pub struct Volume {
    pub id: Uuid,
    pub label: String,
    pub mount_point: PathBuf,
    pub volume_type: VolumeType,    // Local / External / Network (physical)
    pub purpose: Option<VolumePurpose>,  // NEW: logical role
    #[serde(skip)]
    pub is_online: bool,
}
```

`purpose` is `Option` so existing volumes continue to work without migration friction. Commands that use purpose treat `None` as "unclassified" (equivalent to Working for duplicate analysis — no special exemptions).

### CLI

```bash
# Set purpose when adding a volume
maki volume add /Volumes/BackupDrive --label "Backup A" --purpose backup

# Update purpose on an existing volume
maki volume set-purpose "Backup A" backup

# Show purpose in volume list
maki volume list
# Photos SSD (a1b2c3d4-...) [online] [working]
#   Path: /Volumes/Photos
# Master Media (e5f67890-...) [online] [archive]
#   Path: /Volumes/MediaDrive
# Backup A (f1234567-...) [offline] [backup]
#   Path: /Volumes/BackupDrive
```

### Persistence

Stored in `volumes.yaml` (source of truth) and `volumes` table in SQLite. Schema migration adds a `purpose TEXT` column to the `volumes` table, defaulting to `NULL`.

---

## Part 2: Smart Duplicate Analysis ✅ *Implemented in v1.4.0*

### Enhanced `maki duplicates` Command

Currently `maki duplicates` lists all variants with multiple file_locations. Enhance it with filtering modes:

```bash
# Current behavior (unchanged default)
maki duplicates

# Same-volume duplicates — likely unwanted
maki duplicates --same-volume
# Shows only variants with 2+ locations on the SAME volume
# Example: DSC_001.nef at Capture/DSC_001.nef AND Selects/DSC_001.nef on "Photos SSD"

# Cross-volume duplicates — likely wanted backups
maki duplicates --cross-volume
# Shows only variants with locations on 2+ DIFFERENT volumes

# Filter by volume
maki duplicates --volume "Photos SSD"
# Shows duplicates involving this specific volume

# Combine with search query
maki duplicates --same-volume "rating:3+ type:image"
# Same-volume duplicates, but only for rated images
```

### Implementation

The `--same-volume` filter groups locations by `volume_id` and reports only variants where any single volume has more than one location. The `--cross-volume` filter reports variants where locations span multiple distinct `volume_id` values.

SQL for same-volume duplicates:
```sql
SELECT fl.content_hash, fl.volume_id, COUNT(*) as loc_count
FROM file_locations fl
GROUP BY fl.content_hash, fl.volume_id
HAVING COUNT(*) > 1
```

SQL for cross-volume duplicates:
```sql
SELECT fl.content_hash, COUNT(DISTINCT fl.volume_id) as vol_count
FROM file_locations fl
GROUP BY fl.content_hash
HAVING COUNT(DISTINCT fl.volume_id) > 1
```

### Output Enhancement

Add context to duplicate listings that helps the user decide what to do:

```
DSC_001.nef (Original, 25.3 MB) — 3 locations on 2 volumes

  Photos SSD [working]:
    Capture/2026-02/DSC_001.nef        verified 2h ago
    Selects/2026-02/DSC_001.nef        verified 2h ago    ← same-volume duplicate

  Master Media [archive]:
    Photos/2026-02/DSC_001.nef         verified 3d ago    ← cross-volume backup
```

The `--json` output includes `same_volume_groups` and `cross_volume_count` fields for scripting.

---

## Part 3: Duplicate Cleanup

### `maki dedup` Command

A guided cleanup for same-volume duplicates:

```bash
# Report mode (default) — show what would be removed
maki dedup [--volume <label>] [--prefer <path-pattern>] [--min-copies N]

# Apply mode — remove duplicate files
maki dedup --apply [--volume <label>] [--prefer <path-pattern>] [--min-copies N]
```

**Flags:**

- `--volume <label>` — limit to duplicates on this volume (default: all volumes, each analyzed independently)
- `--prefer <path-pattern>` — when choosing which copy to keep, prefer locations matching this path prefix (e.g. `--prefer Selects` keeps the Selects copy over the Capture copy)
- `--min-copies N` — ensure at least N total locations survive per variant across all volumes (default: 1). Acts as a safety net — if a variant has 3 locations (2 on working, 1 on archive) and `--min-copies 2`, only 1 of the 2 working-volume copies is removed.
- `--apply` — actually delete files and remove location records (safe default: report-only)
- `--dry-run` — alias for the default report-only mode (explicit)

**Resolution heuristic** (when no `--prefer` given):

For each set of same-volume duplicates, pick which location(s) to keep:
1. Prefer locations with more recent `verified_at` timestamp
2. Prefer shorter relative paths (closer to volume root)
3. If all else equal, keep the first alphabetically (deterministic)

The heuristic only removes locations within the same volume — cross-volume copies are never touched by `dedup`.

**Example workflow:**

```bash
# See what duplicates exist on the laptop
maki dedup --volume "Photos SSD"
# Found 47 same-volume duplicates:
#   DSC_001.nef: keep Selects/2026-02/DSC_001.nef, remove Capture/2026-02/DSC_001.nef
#   DSC_002.nef: keep Selects/2026-02/DSC_002.nef, remove Capture/2026-02/DSC_002.nef
#   ...
# Total: 47 files, 1.2 GB reclaimable

# Prefer Selects over Capture (explicit)
maki dedup --volume "Photos SSD" --prefer Selects

# Apply after reviewing
maki dedup --volume "Photos SSD" --prefer Selects --apply
# Removed 47 duplicate locations (1.2 GB freed)
```

**Safety:**
- Never removes the last location of a variant (absolute minimum: 1)
- `--min-copies` raises this floor across all volumes
- Report-only by default
- Supports `--json`, `--log`, `--time` flags
- Deletes the physical file from disk AND removes the location record from catalog/sidecar

---

## Part 4: Backup Coverage

### `maki backup-status` Command ✅ *Implemented in v1.4.1*

Answers the question: "Are my important assets safely backed up?"

```bash
# Overview
maki backup-status

# Filter to specific assets
maki backup-status "rating:3+ type:image"

# Check coverage on a specific volume
maki backup-status --volume "Master Media"

# Require N copies
maki backup-status --min-copies 2
```

**Output:**

```
Backup Status (all assets)
==========================

Total assets:           4,231
Total variants:         6,892
Total file locations:  12,456

Coverage by volume purpose:
  Working (2 volumes):    4,102 assets (97.0%)
  Archive (1 volume):     3,891 assets (92.0%)
  Backup  (2 volumes):    3,456 assets (81.7%)

Location distribution:
  1 location only:          412 assets  ← AT RISK
  2 locations:            1,823 assets
  3+ locations:           1,996 assets

At-risk assets (1 location, no archive/backup copy):
  Use 'maki backup-status --at-risk' to list them
  Use 'maki backup-status --at-risk -q' for asset IDs (pipeable)

Volume gaps:
  Master Media: missing 340 assets present on working volumes
  Backup A:     missing 775 assets present on working volumes
```

**Flags:**

- `--at-risk` — list assets with fewer than `--min-copies` locations (default: assets with only 1 location)
- `--min-copies N` — threshold for "adequately backed up" (default: 2)
- `--volume <label>` — show which assets are missing from this specific volume
- `--format` / `-q` — output format for `--at-risk` listings (same as `search --format`)
- `--json` — structured output for scripting

**Pipeable to other commands:**

```bash
# Find all rated assets not on the master drive, then relocate them
maki backup-status --volume "Master Media" --at-risk -q "rating:3+" \
  | xargs -I{} maki relocate {} "Master Media"

# Find at-risk assets and add them to a collection for review
maki backup-status --at-risk -q | xargs maki collection add "Needs Backup"
```

### Search Filter: `copies` ✅ *Implemented in v1.4.0*

Add a new search filter for location count:

```bash
# Assets with exactly 1 file location (single point of failure)
maki search "copies:1"

# Assets with 3 or more locations
maki search "copies:3+"

# Highly rated assets with insufficient backup
maki search "rating:4+ copies:1"
```

Implementation: `copies:N` is a pure SQL filter on a COUNT of `file_locations` grouped by `asset_id` (via variant join). `copies:N+` uses `HAVING COUNT(*) >= N`. This avoids disk I/O, unlike `missing:true`.

---

## Part 5: Web UI Integration

### Volume Purpose Display

- `maki volume list` in the web UI (stats page or dedicated volumes page) shows purpose badges
- Volume filter dropdown in the browse page shows purpose next to label

### Duplicates Page Enhancement

- `/duplicates` page (if implemented per `enhancements.md`) shows same-volume vs cross-volume grouping
- Color-coded: same-volume duplicates highlighted as cleanup candidates, cross-volume shown as backup confirmations

### Backup Status Dashboard

- Section on the stats page or a dedicated `/backup` tab
- Volume coverage bars (percentage of assets present on each volume)
- "At risk" count with a link to filtered browse view (`copies:1`)
- Per-volume gap counts with links to filtered views

---

## Implementation Plan

### Phase 1: Volume Purpose (foundation) ✅ *v1.4.0*

**Files modified:**
- `src/models/volume.rs` — added `VolumePurpose` enum, `purpose` field to `Volume`
- `src/device_registry.rs` — serialize/deserialize purpose, added `set_purpose()` method
- `src/catalog.rs` — schema migration for `purpose` column on `volumes` table, updated insert/load queries
- `src/main.rs` — added `--purpose` flag to `volume add`, added `volume set-purpose` subcommand
- `doc/manual/reference/01-setup-commands.md` — documented new flags/subcommand

### Phase 2: Enhanced Duplicates ✅ *v1.4.0*

**Files modified:**
- `src/catalog.rs` — added `find_duplicates_same_volume()`, `find_duplicates_cross_volume()` with shared `load_duplicate_entries()` helper; enriched `LocationDetails` with `volume_id`, `volume_purpose`, `verified_at`; enriched `DuplicateEntry` with `volume_count`, `same_volume_groups`
- `src/main.rs` — added `--same-volume`, `--cross-volume`, `--volume` flags to `duplicates` subcommand; enhanced output with purpose tags, volume counts, same-volume warnings, verification timestamps
- `doc/manual/reference/04-retrieve-commands.md` — documented new flags

### Phase 3: `copies` Search Filter ✅ *v1.4.0*

**Files modified:**
- `src/query.rs` — parse `copies:N` and `copies:N+` filter syntax
- `src/catalog.rs` — `build_search_where()` adds scalar subquery on location count (self-contained, no outer JOIN needed)
- `doc/manual/reference/06-search-filters.md` — documented new filter

### Phase 4: `maki dedup` Command ✅ *v1.4.1*

**Files modified:**
- `src/asset_service.rs` — added `DedupResult`, `DedupStatus`, and `dedup()` method with resolution heuristic (prefer prefix, verified_at, path length, alphabetical) and file deletion
- `src/main.rs` — added `dedup` subcommand with `--volume`, `--prefer`, `--min-copies`, `--apply` flags
- `doc/manual/reference/05-maintain-commands.md` — documented command

### Phase 5: `maki backup-status` Command ✅ *v1.4.1*

**Files modified:**
- `src/catalog.rs` — added `BackupStatusResult` structs + `backup_status_overview()`, `backup_status_at_risk_ids()`, `backup_status_missing_from_volume()` methods. Counts distinct volumes per asset (not file locations) for backup safety.
- `src/main.rs` — added `backup-status` subcommand with `--at-risk`, `--min-copies`, `--volume`, `--format`, `-q` flags; overview + listing modes; `print_backup_status_human()` output.
- `doc/manual/reference/04-retrieve-commands.md` — documented command.

### Phase 6: Web UI ✅ *v1.4.1*

- Volume purpose display in stats/volume list (shipped in v1.4.0)
- Backup status dashboard — dedicated `/backup` page with summary cards, volume distribution chart, purpose coverage table, volume gaps table, and at-risk asset link
- `copies:N` filter integration in browse page (shipped in v1.4.0)

---

## Relationship to Existing Proposals

This proposal **supersedes** the following items from `enhancements.md`:

- **Item 1: Smart Duplicate Resolution** — fully specified here as `maki dedup` (Part 3)
- **Item 5/14: Backup Coverage Report** — fully specified here as `maki backup-status` (Part 4)

The `enhancements.md` items were sketches; this proposal provides the complete design with data model, CLI interface, implementation details, and phase plan.

---

## Example: Full Workflow

Thomas's photography workflow with maki storage management:

```bash
# 1. Setup: label volumes with their purpose
maki volume add /Volumes/LaptopSSD --label "Laptop" --purpose working
maki volume add /Volumes/MediaDrive --label "Master Media" --purpose archive
maki volume add /Volumes/BackupDisk --label "Backup A" --purpose backup

# 2. Import from CaptureOne session (on laptop)
maki import /Volumes/LaptopSSD/Sessions/2026-02-24/Capture/

# 3. After rsync to master drive, import the copies
maki import /Volumes/MediaDrive/Sessions/2026-02-24/
# → "0 imported, 47 locations added" — same hashes, new locations tracked

# 4. After culling and moving selects (locally)
maki sync /Volumes/LaptopSSD/Sessions/2026-02-24/ --apply
# → Detects moved files (Capture → Selects), updates locations

# 5. Check for accidental local duplicates
maki dedup --volume "Laptop"
# → Shows RAWs that exist in both Capture/ and Selects/ (copied not moved)

# 6. Clean up local duplicates, keep the Selects copy
maki dedup --volume "Laptop" --prefer Selects --apply
# → Removes 12 duplicate files, frees 450 MB

# 7. After processing and exporting, check backup coverage
maki backup-status "rating:3+"
# → "23 rated assets missing from Master Media, 45 missing from Backup A"

# 8. Ensure rated assets reach the master drive
maki backup-status --volume "Master Media" --at-risk -q "rating:3+" \
  | xargs -I{} maki relocate {} "Master Media"

# 9. Verify everything is safe before cleaning up the laptop
maki backup-status --min-copies 2 "rating:3+"
# → "All 340 rated assets have 2+ locations ✓"
```

---

## Summary

| Feature | What it solves | Effort | Status |
|---------|---------------|--------|--------|
| Volume purpose | Semantic context for duplicate analysis | Small | ✅ v1.4.0 |
| Enhanced duplicates | Distinguish unwanted from wanted copies | Small | ✅ v1.4.0 |
| `copies:N` filter | Find under-backed-up assets in search | Small | ✅ v1.4.0 |
| `maki dedup` | Clean up same-volume duplicates safely | Medium | ✅ v1.4.1 |
| `maki backup-status` | At-a-glance backup health overview | Medium | ✅ v1.4.1 |
| Web UI integration | Visual backup dashboard | Medium | ✅ v1.4.1 |

All phases shipped: 1–3 in v1.4.0, 4–6 in v1.4.1.
