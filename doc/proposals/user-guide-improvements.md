# Proposal: User Guide Improvements

Audit of the MAKI user manual (user guide chapters 01–10) against the full CLI feature set and real-world workflow needs. The reference chapters are comprehensive; this proposal focuses on gaps in the user guide where implemented features are undocumented or lack motivational/workflow context.

**Date:** 2026-03-28

**Status:** Pass 1 and 2 implemented (2026-03-28). Pass 3 pending.

---

## A. Commands and Features Missing from the User Guide

### 1. Contact Sheets (`maki contact-sheet`)

**Gap:** Zero coverage in the user guide. The command has rich options (layout presets, paper sizes, grouping, copyright, field selection, sort, label styling) but is only documented in the reference.

**Why it matters:** Contact sheets are a core photography workflow tool — client proofing, archival reference prints, portfolio review boards, shoot overviews for art directors or editors.

**Suggested content:** New section in chapter 05 (Browse & Search) or chapter 04 (Organize). Cover use cases:

- **Client proofing:** Generate a PDF to email or print for review. Show `--title`, `--copyright`, `--fields "filename,rating"`.
- **Shoot overview:** Group by date or label to see the shape of a shoot at a glance. Show `--group-by date`, `--landscape`, `--layout dense`.
- **Portfolio review:** Select rated images and produce a large-format sheet. Show `--layout large --paper a3`.
- **Archival reference:** A printed record of what's on an archive drive. Show `--group-by volume`, `--fields "filename,date,format"`.

**Example commands:**

```bash
# Client proofing sheet for a wedding
maki contact-sheet "tag:wedding date:2026-03 rating:3+" wedding-proofs.pdf \
  --title "Johnson Wedding — March 2026" --copyright "Jane Doe Photography" \
  --fields "filename,rating" --layout standard

# Shoot overview grouped by shooting date
maki contact-sheet "path:Capture/2026-03" march-overview.pdf \
  --group-by date --landscape --layout dense

# Archival reference for a backup drive
maki contact-sheet "volume:Archive" archive-index.pdf \
  --fields "filename,date,format" --group-by volume --layout dense
```

**Effort:** Medium. New section, ~1.5 pages.

---

### 2. Deleting Assets (`maki delete`)

**Gap:** No user guide coverage at all. The command has important safety semantics: report-only by default, `--apply` to execute, `--remove-files` as the truly destructive option.

**Why it matters:** Every photographer eventually needs to permanently remove assets. The distinction between removing catalog records and deleting physical files is critical — especially when files exist on multiple volumes.

**Suggested content:** New section in chapter 04 (Organize), logically following the culling/curation discussion in chapter 10. Cover:

- **When to delete vs. cull:** Tagging as `rest` hides images from browsing but preserves them. Deletion is permanent. The guide should help users make this decision.
- **Report-only default:** `maki delete <id>` shows what would happen. `--apply` executes.
- **Catalog-only vs. physical deletion:** Without `--remove-files`, only the catalog and sidecar records are removed — the original files stay on disk. With `--remove-files`, physical files are deleted from all online volumes.
- **Multi-volume behavior:** Deletion removes locations on all online volumes. Offline volumes are skipped (files survive there until a future cleanup).
- **Batch deletion from stdin:** `maki search -q "tag:reject" | maki delete --apply` for bulk cleanup after culling.

**Example commands:**

```bash
# Preview what would be deleted
maki delete a1b2c3d4

# Remove from catalog only (files stay on disk)
maki delete a1b2c3d4 --apply

# Remove from catalog AND delete physical files
maki delete a1b2c3d4 --apply --remove-files

# Bulk delete all rejected images
maki search -q "tag:reject" | maki delete --apply --remove-files --log
```

**Effort:** Low–medium. ~1 page.

---

### 3. Preview Management (`maki generate-previews`)

**Gap:** Only mentioned in passing during import. The `--upgrade`, `--force`, and `--smart` flags are never explained. Users don't know how to fix stale or low-quality previews.

**Why it matters:** Preview quality directly affects the browsing and web UI experience. After processing in external tools (e.g., exporting TIFFs from CaptureOne), the preview may still show the original RAW rendering. The `--upgrade` flag solves this.

**Suggested content:** Section in chapter 07 (Maintenance). Cover:

- **When previews get stale:** After processing in external tools, the auto-generated preview was made from the RAW original. Use `--upgrade` to regenerate from the better export/processed variant.
- **Smart previews for offline work:** `--smart` generates 2560px previews that enable zoom and pan in the web UI even when the original volume is offline.
- **Force regeneration:** `--force` replaces all existing previews — useful after changing `[preview]` config (max_edge, format, quality).
- **Scoping:** `--volume`, `--asset`, path arguments, `--include`/`--skip` to target specific subsets.

**Example commands:**

```bash
# Upgrade previews where a processed/export variant exists
maki generate-previews --upgrade --log

# Generate smart previews for an entire volume before disconnecting
maki generate-previews --volume "Work SSD" --smart --log

# Regenerate all previews after changing config
maki generate-previews --force --log --time
```

**Effort:** Low–medium. ~1 page.

---

### 4. Volume Split and Rename (`maki volume split`, `maki volume rename`)

**Gap:** Setup chapter covers add/remove/combine but not split or rename.

**Why it matters:** Storage reorganization is common — moving a subfolder to a new drive, relabeling volumes after hardware changes.

**Suggested content:** Expand chapter 02 (Setup). Cover:

- **Split:** When you physically move a subfolder to a new drive. `maki volume split "Photos" "New Drive" --path "Archive/2024" --apply` creates a new volume from a subdirectory, rewriting all file locations.
- **Rename:** When drive labels change or you want clearer names. `maki volume rename "Old Label" "New Label"`.

**Effort:** Low. ~0.5 page addition to existing section.

---

### 5. Fix Recipes (`maki fix-recipes`)

**Gap:** Not mentioned in the user guide. Repairs a specific situation where recipe files were imported as standalone assets instead of being attached to their media files.

**Why it matters:** This happens when recipe and media files are imported in separate passes, or when recipe files appear in a different directory from their media. Without this command, users end up with phantom assets for `.xmp` files.

**Suggested content:** Brief addition to chapter 07 alongside fix-roles and fix-dates. One paragraph explaining the scenario plus an example.

**Effort:** Low. ~0.25 page.

---

## B. Commands Covered but Lacking Workflow Context

### 6. Backup Status — Full Options

**Gap:** Mentioned in one line in setup and in the storage hygiene section, but `--at-risk`, `--min-copies`, `--volume`, and `--query` are never shown.

**Why it matters:** These options make backup-status actionable rather than informational. `--at-risk` produces a list you can pipe into `relocate` to actually fix coverage gaps.

**Suggested content:** Expand the Storage Hygiene section in chapter 07:

```bash
# Which assets have fewer than 2 copies?
maki backup-status --at-risk

# Stricter policy: require 3 copies
maki backup-status --min-copies 3 --at-risk

# Which of my rated images aren't on the backup drive yet?
maki backup-status --volume "Backup" --at-risk -q "rating:3+"

# Fix gaps: copy at-risk assets to the backup drive
maki backup-status --at-risk -q | maki relocate --target "Backup Drive"
```

**Effort:** Low. ~0.5 page expansion.

---

### 7. Export — Full Workflow Context

**Gap:** Export is covered but `--zip`, `--layout mirror` vs `flat`, and `--symlink` lack motivational context.

**Why it matters:** Different delivery scenarios need different export strategies.

**Suggested content:** Expand the export section in chapter 05:

- **`--zip`** for email/upload delivery (single file, easy to share)
- **`--layout mirror`** for preserving folder structure (handoff to other tools, archival export)
- **`--symlink`** for temporary working folders without copying gigabytes (edit in Photoshop, then delete the symlink folder)
- **`--include-sidecars`** for round-tripping with CaptureOne/Lightroom (export with XMP, edit externally, re-import)
- **`--all-variants`** for delivering both RAW + processed to a client or collaborator

**Effort:** Low. ~0.5 page expansion.

---

### 8. Verify — Incremental Verification

**Gap:** User guide shows basic verify but not `--max-age` or `--force`.

**Why it matters:** On a library of 50,000+ files, full verification takes hours. Incremental verification makes weekly runs practical.

**Suggested content:** Expand verify in chapter 07:

```bash
# Skip files verified within the last 30 days
maki verify --max-age 30 --log

# Force re-verification of everything
maki verify --force --log --time

# Find assets that haven't been verified recently
maki search "stale:90"
```

Also mention the `[verify] max_age_days` config option for setting the default.

**Effort:** Low. ~0.25 page expansion.

---

### 9. Relocate — Batch Mode

**Gap:** User guide only shows single-asset relocate. The `--query` and `--target` flags for batch relocation are the realistic use case.

**Why it matters:** Migrating an entire year's shoots to an archive drive is a common task. The single-asset example doesn't convey this.

**Suggested content:** Expand relocate in chapter 07:

```bash
# Preview: migrate all 2024 photos from work SSD to archive
maki relocate --query "date:2024 volume:WorkSSD" --target "Archive 2024" --dry-run

# Execute the migration
maki relocate --query "date:2024 volume:WorkSSD" --target "Archive 2024" --log

# Move (copy + delete source) after confirming
maki relocate --query "date:2024 volume:WorkSSD" --target "Archive 2024" --remove-source --log
```

**Effort:** Low. ~0.5 page expansion.

---

### 10. Saved Search `--favorite`

**Gap:** Not mentioned. Favorites appear as quick-access chips on the web UI browse page.

**Suggested content:** Add to chapter 04 (Saved Searches section):

```bash
# Save a search and mark it as favorite for the browse toolbar
maki ss save "Five Stars" "rating:5" --favorite
maki ss save "Unrated" "rating:0 type:image" --favorite
```

Explain that favorite searches show as clickable chips in the web UI filter bar for one-click access.

**Effort:** Low. 2–3 sentences + example.

---

### 11. Stack `from-tag`

**Gap:** Not in user guide. Bridges CaptureOne/Lightroom workflows where stacking metadata is embedded in tags.

**Suggested content:** Add to chapter 04 (Stacks section):

```bash
# Preview: find CaptureOne auto-stack tags and convert to MAKI stacks
maki stack from-tag "Aperture Stack {}"

# Apply and clean up the tags afterwards
maki stack from-tag "Aperture Stack {}" --remove-tags --apply
```

Explain the `{}` wildcard pattern and how it groups assets by the matched value.

**Effort:** Low. ~0.25 page.

---

### 12. Show `--locations`

**Gap:** Not mentioned. Quick way to check where an asset's files live.

**Suggested content:** Brief mention in chapter 05 under Show:

```bash
# Quick check: where are this asset's files?
maki show a1b2c3d4 --locations
# Output: Photos:Capture/2026-01-15/DSC_1234.NEF
#         Backup:Capture/2026-01-15/DSC_1234.NEF
```

Useful before relocating or deleting, to confirm which volumes hold copies.

**Effort:** Low. 2–3 sentences + example.

---

## C. Missing Workflow and Best-Practice Topics

### 13. The Archive Lifecycle (new section)

**Gap:** The user guide explains individual commands but never paints the complete picture of how files flow through storage tiers over time.

**Why it matters:** This is the "big picture" that ties volumes, purposes, relocate, backup-status, and verify into a coherent strategy. An ambitious amateur managing terabytes across multiple drives needs this mental model.

**Suggested content:** New section in chapter 02 (Setup) or a standalone chapter. Cover the lifecycle:

1. **Import to working SSD** — fast storage for current projects (`--purpose working`)
2. **Cull and rate** — use the web UI or CLI to separate picks from rejects
3. **Relocate to archive** — batch-move completed work to archive drive (`maki relocate --query ... --target "Archive"`)
4. **Create backup copies** — relocate (without `--remove-source`) to a backup drive
5. **Verify periodically** — `maki verify --max-age 30` on each drive
6. **Check backup coverage** — `maki backup-status --at-risk` to find gaps
7. **Clean up working drive** — `maki relocate --remove-source` once archive + backup are confirmed

Show a concrete example: a photographer's monthly workflow from shoot to archive.

**Effort:** Medium. ~2 pages. High value.

---

### 14. Working with Video

**Gap:** MAKI handles video (import, previews via ffmpeg, duration/codec/resolution metadata, search filters) but the user guide never discusses video-specific considerations.

**Why it matters:** Most photographers also shoot video. Mixed photo+video shoots are common. Users need to know what MAKI can and can't do with video files.

**Suggested content:** New section in chapter 03 (Ingest) or chapter 05 (Browse & Search). Cover:

- **What MAKI extracts from video:** duration, codec, resolution, framerate (via ffprobe)
- **Video previews:** Generated by ffmpeg (first non-black frame). Requires ffmpeg installed.
- **Video proxy generation:** Hover-to-play in the web UI uses proxy clips.
- **Search filters for video:** `type:video`, `duration:30+`, `duration:10-60`, `codec:h264`, `codec:hevc`
- **Mixed shoots:** `type:image` and `type:video` filters to work with each type separately

**Effort:** Low–medium. ~1 page.

---

### 15. Recovering from Drive Failures (new section)

**Gap:** The tools for recovery exist (`rebuild-catalog`, `cleanup`, `backup-status --at-risk`, `relocate`) but the user guide doesn't walk through the scenario.

**Why it matters:** This is exactly when users need the most guidance and are under the most stress. A clear playbook reduces panic.

**Suggested content:** New section in chapter 07 (Maintenance). Walk through the scenario:

1. **Don't panic.** MAKI's sidecar files and catalog are separate from your media files. If the catalog drive is fine, you have all metadata.
2. **Assess the damage:** `maki volume list` — which volume is offline? `maki backup-status --at-risk` — which assets were only on the failed drive?
3. **If you have backups:** The assets with cross-volume copies are safe. `maki search "volume:FailedDrive copies:2+"` shows what's backed up.
4. **Clean up stale records:** Once you've recovered what you can, `maki cleanup --volume "FailedDrive" --apply` removes references to the dead drive.
5. **Rebuild backup coverage:** `maki backup-status --at-risk` to find remaining gaps, then `maki relocate` to copy surviving assets to a new backup.
6. **If the catalog is lost:** `maki rebuild-catalog` regenerates it from sidecar YAML files (the source of truth).

**Effort:** Medium. ~1.5 pages. Very high value for peace of mind.

---

### 16. Multi-tool Round-trips (expand chapter 07)

**Gap:** The sync/refresh/writeback/sync-metadata commands enable round-trips with CaptureOne, Lightroom, etc., but the user guide doesn't spell out concrete workflows by tool.

**Why it matters:** Most MAKI users also use CaptureOne, Lightroom, or similar. "I edited in CaptureOne, now what?" is a frequent question.

**Suggested content:** Expand chapter 07 or add a new section. Cover concrete scenarios:

- **"I rated/tagged in CaptureOne, now I want those changes in MAKI":**
  `maki refresh --volume "Work SSD" --log` (reads changed XMP sidecars)

- **"I rated/tagged in MAKI, now I want those changes in CaptureOne/Lightroom":**
  `maki writeback --volume "Work SSD"` (writes MAKI metadata to XMP)

- **"I want bidirectional sync after working in both tools":**
  `maki sync-metadata --volume "Work SSD"` (reads external changes + writes pending MAKI edits, reports conflicts)

- **"I imported old files before embedded XMP extraction existed":**
  `maki refresh --media` (re-extracts embedded XMP from JPEG/TIFF)

Explain when to use `sync-metadata` (the combined command) vs. the separate `refresh`/`writeback` pair (finer control, e.g., when you want MAKI edits to always win).

**Effort:** Medium. ~1.5 pages.

---

### 17. Import Strategies for Different Scenarios (expand chapter 03)

**Gap:** Chapter 03 shows the import pipeline but doesn't address different real-world import scenarios.

**Why it matters:** The way you import differs significantly between card reader dumps, tethered shooting folders, and migrating from another DAM.

**Suggested content:** Expand chapter 03 with a "Common Import Scenarios" section:

- **Card reader / memory card:** Import the card, use `--add-tag` for shoot identification, `--auto-group` for RAW+JPEG pairing.
- **Tethered shooting folder:** Import the capture folder. Re-run import later to pick up new files (existing files are skipped by content hash).
- **Migrating from another DAM:** Import the managed folder structure. Use `--include captureone` or `--include rawtherapee` to pick up tool-specific recipe files. Then `maki refresh` to read existing XMP metadata.
- **Cloud-synced folder:** Register as a volume with `--purpose cloud`. Be aware of partial sync states.
- **Re-importing from backups:** Shows cross-volume copies (covered in existing "Finding Duplicates" section, just link to it).
- **`--add-tag` for shoot tagging:** Tag all imports from one session for easy retrieval later:
  ```bash
  maki import /Volumes/Card/DCIM --add-tag "shoot:johnson-wedding"
  ```

**Effort:** Medium. ~1.5 pages.

---

## Priority and Sequencing

### Pass 1 — High impact, fills critical gaps (DONE 2026-03-28)

| # | Topic | Type | Effort | Chapter | Status |
|---|-------|------|--------|---------|--------|
| 1 | Contact Sheets | Missing command | Medium | 05 | Done |
| 2 | Deleting Assets | Missing command | Low–Med | 04 | Done |
| 13 | Archive Lifecycle | Workflow | Medium | New ch. 11 | Done |
| 15 | Drive Failure Recovery | Workflow | Medium | 07 | Done |

### Pass 2 — Practical workflow improvements (DONE 2026-03-28)

| # | Topic | Type | Effort | Chapter | Status |
|---|-------|------|--------|---------|--------|
| 6 | Backup Status full workflow | Expand | Low | 07 | Done |
| 7 | Export full workflow | Expand | Low | 05 | Done |
| 9 | Relocate batch mode | Expand | Low | 07 | Done |
| 3 | Preview Management | Missing command | Low–Med | 07 | Done |
| 16 | Multi-tool Round-trips | Workflow | Medium | 07 | Done |
| 8 | Verify incremental | Expand | Low | 07 | Done (bonus) |

### Pass 3 — Completeness and polish

| # | Topic | Type | Effort | Chapter |
|---|-------|------|--------|---------|
| 4 | Volume Split/Rename | Missing command | Low | 02 |
| 5 | Fix Recipes | Missing command | Low | 07 |
| 10 | Saved Search --favorite | Expand | Low | 04 |
| 11 | Stack from-tag | Expand | Low | 04 |
| 12 | Show --locations | Expand | Low | 05 |
| 14 | Working with Video | Workflow | Low–Med | 03/05 |
| 17 | Import Strategies | Workflow | Medium | 03 |
