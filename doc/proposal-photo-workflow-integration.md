# Proposal: Photo Workflow Tool Integration (v0.3.0+)

## Motivation

DAM is designed to manage large photo/video collections across offline storage devices. In practice, photographers use DAM alongside tools like CaptureOne, Lightroom, or RawTherapee — tools that move, rename, and annotate files independently.

A typical CaptureOne session workflow looks like this:

1. **Import to CaptureOne** — RAW files land in the session's `Capture/` folder
2. **Initial cull & tag** — Photographer adds session keywords, basic ratings in CaptureOne
3. **Import to DAM** — `dam import` from `Capture/`, picking up XMP keywords and ratings
4. **Refine in CaptureOne** — Ratings adjusted, keywords refined, COS adjustments saved
5. **Select** — Chosen images moved to `Selects/` folder within the CaptureOne session
6. **Process & export** — Final edits applied, exports generated

DAM currently handles steps 2–3 well. Steps 4–6 create problems: files move without DAM knowing, metadata changes go unnoticed, and stale location records accumulate silently.

This proposal identifies the gaps and suggests features to close them — not just for CaptureOne, but for any external tool that operates on the same files.

---

## Current Strengths

What already works well for this workflow:

- **Stem-based auto-grouping** — RAW + JPEG + XMP + COS are grouped into one asset automatically
- **Location-based recipe identity** — Re-importing a modified COS/XMP file updates its hash in place, no duplicates
- **XMP metadata extraction** — Keywords, rating, description, label, creator, rights are all captured
- **Re-import semantics** — Changed XMP data overwrites rating/description and merges keywords
- **Multi-location tracking** — An asset can exist on multiple volumes simultaneously
- **Content-addressed integrity** — SHA-256 hashes detect corruption and enable deduplication
- **File type group filtering** — `--include captureone` / `--skip captureone` controls recipe import

## Identified Gaps

### 1. External File Movement Goes Undetected

**The problem:** When CaptureOne moves a file from `Capture/` to `Selects/`, DAM still records the old path. On verify, the old location shows as `Missing`, but:
- The missing location record is never cleaned up
- The file at its new location is unknown to DAM
- Re-importing the new location adds it, but the stale old location persists
- There is no way to find "all assets with missing locations" via search

**Impact:** Over time, the catalog accumulates stale location records, making it unreliable for answering "where is this file?"

### 2. No Metadata Sync After External Edits

**The problem:** After the initial import, CaptureOne edits (refined keywords, changed ratings, new adjustments) are invisible to DAM unless the user manually re-imports. There is no mechanism to:
- Detect that XMP/COS files have changed since last import
- Selectively re-apply changed metadata without a full re-import
- See which assets have pending external changes

**Impact:** DAM metadata drifts out of sync with the "source of truth" in CaptureOne sidecars.

### 3. No Batch Operations in Web UI

**The problem:** The web UI supports editing tags and rating for one asset at a time. A photographer culling 500 images needs to:
- Apply the same tag to many assets at once
- Rate multiple assets in quick succession
- Select a set of assets for bulk operations

**Impact:** The web UI is useful for browsing but not for the review/cull phase of a photo workflow.

### 4. Limited Metadata Editing

**The problem:** Description and name cannot be edited via the web UI or CLI (only set during XMP import). Source metadata is read-only.

### 5. No Saved Searches or Collections

**The problem:** Tags are the only grouping mechanism. There are no saved searches, smart albums, or manual collections. A photographer working on a project has no way to bookmark a filtered view.

### 6. No Dry-Run Import

**The problem:** Before re-importing a directory after external changes, there's no way to preview what would happen — how many new files, how many location additions, how many recipe updates.

---

## Proposed Features

### Phase 1: External Change Detection & Location Management

These features address the core workflow break: files moving outside DAM.

#### 1.1 `dam sync` Command — **IMPLEMENTED** (v0.3.1)

Implemented as `dam sync <PATHS...> [--volume <label>] [--apply] [--remove-stale]`. Report-only by default (safe); `--apply` writes changes. `--remove-stale` (requires `--apply`) removes catalog locations for missing files. Detects unchanged, moved, new, modified, and missing files. New files are not auto-imported — user runs `dam import` separately.

#### 1.2 `dam cleanup` Command

Remove stale location records from the catalog:

```
dam cleanup [--volume <label>] [--dry-run]
```

- Iterates all file locations on the specified volume (or all online volumes)
- Checks if each file exists on disk
- Reports and optionally removes location records for missing files
- Does NOT delete assets — an asset with zero remaining locations is still valid (it's an offline/lost asset)

#### 1.3 Search Filters for Location Health

New search filters to find assets needing attention:

- `missing:true` — Assets where at least one location points to a non-existent file
- `orphan:true` — Assets with zero valid locations (all files missing/offline)
- `stale:true` — Assets with locations not verified in N days (configurable)
- `volume:none` — Assets with no locations on any online volume

#### 1.4 `dam update-location` Command

Manually update a file's location when you know where it moved:

```
dam update-location <asset-id> --from <old-path> --to <new-path> [--volume <label>]
```

For cases where `sync` is overkill and the user knows exactly what changed.

---

### Phase 2: Metadata Sync & Re-import Improvements

#### 2.1 `dam refresh` Command

Re-read metadata from sidecar files (XMP, COS) without a full import:

```
dam refresh [PATHS...] [--volume <label>] [--asset <id>] [--dry-run]
```

- Finds all recipe/sidecar files for matching assets
- Compares current hash to stored hash
- If changed: re-extract metadata (XMP keywords, rating, description) and update catalog
- `--dry-run` shows what would change without applying

This is lighter than `sync` — it only touches metadata, not file locations.

#### 2.2 Dry-Run Mode for Import

```
dam import --dry-run <PATHS...>
```

Preview what an import would do:
- N new assets to create
- N new locations to add to existing assets
- N recipes to attach/update
- N files to skip (already tracked)

No files written, no catalog changes.

#### 2.3 `dam edit` Command

Edit asset metadata from the CLI:

```
dam edit <asset-id> [--name <name>] [--description <text>] [--rating <1-5|clear>]
```

Currently, name and description can only be set via XMP import. This gives direct control.

---

### Phase 3: Web UI Workflow Improvements

#### 3.1 Multi-Select & Batch Operations

- **Checkbox selection** on browse cards (click to select, shift-click for range)
- **Selection toolbar** appearing when assets are selected:
  - "Tag selected" — add/remove tags on all selected assets
  - "Rate selected" — set rating on all selected assets
  - "Clear selection"
- **Select all on page** / **Select all matching query**
- Backend: batch API endpoints (`POST /api/batch/tags`, `PUT /api/batch/rating`)

#### 3.2 Description & Name Editing

- Inline-editable description field on asset detail page (like the existing star rating)
- Inline-editable asset name

#### 3.3 Keyboard Navigation

- Arrow keys to move between assets in browse grid
- Number keys (1–5) to rate the focused asset
- Enter to open asset detail
- Escape to return to browse
- Spacebar to toggle selection

This turns the web UI into a viable culling/review tool.

#### 3.4 Saved Searches

- "Save this search" button on browse page
- Saved searches stored in `dam.toml` or a separate file
- Listed in nav bar or sidebar
- Smart albums: saved search that auto-updates when catalog changes

---

### Phase 4: Advanced Integration (Future)

These are longer-term ideas, listed for completeness.

#### 4.1 Watch Mode

```
dam watch [PATHS...] [--volume <label>]
```

File system watcher (via `notify` crate) that auto-imports/syncs when files change. Useful for monitoring a CaptureOne session's output folder during an active editing session.

#### 4.2 XMP Write-Back

When metadata is edited in DAM (tags, rating, description), write changes back to XMP sidecar files so CaptureOne picks them up. This creates true bidirectional sync.

Requires careful conflict resolution — DAM and CaptureOne could both modify the same XMP file.

#### 4.3 Export Command

```
dam export <query> --target <path> [--format <preset>] [--include-sidecars]
```

Export matching assets to a directory, optionally with sidecars. Useful for preparing files for delivery or for feeding into another tool.

#### 4.4 Collections

Named, manually curated groups of assets (separate from tags). A "Project: Wedding 2026" collection that holds specific picks regardless of their tags or location.

---

## Priority Recommendation

For v0.3.0, focus on **Phase 1** (sync, cleanup, location health) and the most impactful pieces of **Phase 2** (dry-run import, edit command). These address the fundamental workflow break where external tools move files and DAM loses track.

**Phase 3** (web UI batch ops, keyboard nav) would make DAM a viable review/cull tool, reducing dependence on CaptureOne for that step.

**Phase 4** is aspirational and depends on how the tool evolves with actual use.

### Suggested v0.3.0 Scope

1. ~~`dam sync` with dry-run default~~ — **done** (v0.3.1)
2. `dam cleanup` for stale locations
3. `dam import --dry-run`
4. ~~`dam edit` for name/description/rating~~ — **done** (v0.3.1)
5. Search filters: `missing:`, `orphan:`
6. Web UI: description editing, batch tag/rate
