# Proposal: Photo Workflow Tool Integration

## Motivation

DAM is designed to manage large photo/video collections across offline storage devices. In practice, photographers use DAM alongside tools like CaptureOne, Lightroom, or RawTherapee — tools that move, rename, and annotate files independently.

A typical CaptureOne session workflow looks like this:

1. **Import to CaptureOne** — RAW files land in the session's `Capture/` folder
2. **Initial cull & tag** — Photographer adds session keywords, basic ratings in CaptureOne
3. **Import to DAM** — `dam import` from `Capture/`, picking up XMP keywords and ratings
4. **Refine in CaptureOne** — Ratings adjusted, keywords refined, COS adjustments saved
5. **Select** — Chosen images moved to `Selects/` folder within the CaptureOne session
6. **Process & export** — Final edits applied, exports generated

DAM currently handles steps 2–4 well. Steps 5–6 (file movement, batch operations) still create friction, though DAM now has tools (`sync`, `cleanup`) to recover from external file moves.

This proposal identifies the gaps and suggests features to close them — not just for CaptureOne, but for any external tool that operates on the same files.

---

## Current Strengths

What already works well for this workflow:

- **Stem-based auto-grouping** — RAW + JPEG + XMP + COS are grouped into one asset automatically
- **Location-based recipe identity** — Re-importing a modified COS/XMP file updates its hash in place, no duplicates
- **XMP metadata extraction** — Keywords, rating, description, label, creator, rights are all captured
- **Re-import semantics** — Changed XMP data overwrites rating/description and merges keywords
- **XMP write-back** — Rating, tag, description, and color label changes are written back to `.xmp` files on disk, enabling bidirectional sync with CaptureOne (v0.4.1–v0.4.4)
- **Multi-location tracking** — An asset can exist on multiple volumes simultaneously
- **Content-addressed integrity** — SHA-256 hashes detect corruption and enable deduplication
- **File type group filtering** — `--include captureone` / `--skip captureone` controls recipe import
- **External change recovery** — `dam sync` detects moved/modified/missing files, `dam cleanup` removes stale records
- **CLI metadata editing** — `dam edit` for name, description, rating, color label; `dam tag` for tags
- **Web UI inline editing** — Star rating, tags, description, and color label editable on asset detail page
- **Batch operations** — Multi-select with checkbox, batch tag/untag, batch rating, batch color label via fixed toolbar (v0.4.3–v0.4.4)
- **Color labels** — First-class 7-color label support (Red, Orange, Yellow, Green, Blue, Pink, Purple) with XMP `xmp:Label` extraction, CLI editing, web UI color dot picker, browse filtering, and XMP write-back (v0.4.4)

## Identified Gaps

### ~~1. External File Movement Goes Undetected~~ — **RESOLVED**

Addressed by `dam sync` (detects moved/new/missing files), `dam cleanup` (removes stale records and orphans), `dam update-location` (manual path correction), and search filters (`missing:true`, `orphan:true`, `stale:N`, `volume:none`).

### ~~2. No Metadata Sync After External Edits~~ — **RESOLVED**

XMP write-back (v0.4.1–v0.4.3) enables DAM→CaptureOne sync for rating, tags, and description. `dam refresh` provides lightweight CaptureOne→DAM sync by re-reading metadata from changed sidecar/recipe files without a full re-import. Together with `dam sync --apply` (which detects moved/modified/missing files), bidirectional metadata sync is complete.

### ~~3. No Batch Operations in Web UI~~ — **RESOLVED**

Addressed by multi-select checkboxes on browse cards, a fixed bottom toolbar with batch tag (add/remove), batch rating (set/clear), and batch color label (set/clear). Selection state survives pagination/sort. Keyboard shortcuts: Cmd/Ctrl+A to select all on page, Escape to clear. Backend: `POST /api/batch/tags`, `PUT /api/batch/rating`, `PUT /api/batch/label`. Implemented in v0.4.3–v0.4.4.

### ~~4. Limited Metadata Editing~~ — **RESOLVED**

Name, description, and rating are editable via CLI (`dam edit`). Rating, tags, and description are editable inline in the web UI. Changes are written back to XMP sidecar files on disk. Name editing in the web UI is the only remaining gap.

### 5. No Saved Searches or Collections

**The problem:** Tags are the only grouping mechanism. There are no saved searches, smart albums, or manual collections. A photographer working on a project has no way to bookmark a filtered view.

### 6. No Dry-Run Import

**The problem:** Before re-importing a directory after external changes, there's no way to preview what would happen — how many new files, how many location additions, how many recipe updates.

---

## Proposed Features

### Phase 1: External Change Detection & Location Management — **COMPLETE**

All features in this phase are implemented.

#### 1.1 `dam sync` Command — **IMPLEMENTED** (v0.3.1)

Implemented as `dam sync <PATHS...> [--volume <label>] [--apply] [--remove-stale]`. Report-only by default (safe); `--apply` writes changes. `--remove-stale` (requires `--apply`) removes catalog locations for missing files. Detects unchanged, moved, new, modified, and missing files. New files are not auto-imported — user runs `dam import` separately.

#### 1.2 `dam cleanup` Command — **done** (v0.3.1, extended v0.3.4)

Remove stale location records, orphaned assets, and orphaned preview files:

```
dam cleanup [--volume <label>] [--list] [--apply]
```

- Report-only by default (safe); `--apply` writes changes
- **Pass 1:** Iterates all file locations and recipes on the specified volume (or all online volumes), reports and optionally removes records for missing files
- **Pass 2:** Deletes orphaned assets (all variants have zero file_locations) including their recipes, variants, catalog rows, and sidecar YAML
- **Pass 3:** Removes orphaned preview files (content hash no longer matches any variant)
- Report-only mode predicts orphans that would result from removing stale locations

#### 1.3 Search Filters for Location Health — **done** (v0.3.3)

New search filters to find assets needing attention:

- `missing:true` — Assets where at least one location points to a non-existent file
- `orphan:true` — Assets with zero file_location records
- `stale:N` — Assets with at least one location not verified in N days (or never verified)
- `volume:none` — Assets with no locations on any online volume

#### 1.4 `dam update-location` Command — **done**

Manually update a file's location when you know where it moved:

```
dam update-location <asset-id> --from <old-path> --to <new-path> [--volume <label>]
```

Implemented as `dam update-location <asset-id> --from <old-path> --to <new-path> [--volume <label>]`. `--to` must be an absolute path; `--from` can be absolute or volume-relative. Auto-detects volume from `--to` path. Verifies content hash at new location matches catalog record. Updates both SQLite and sidecar YAML. Handles variant file locations and recipe file locations.

---

### Phase 2: Metadata Sync & Re-import Improvements — **PARTIALLY COMPLETE**

#### 2.1 `dam refresh` Command — **IMPLEMENTED**

Re-read metadata from sidecar files (XMP, COS) without a full import:

```
dam refresh [PATHS...] [--volume <label>] [--asset <id>] [--dry-run]
```

- Finds all recipe/sidecar files for matching assets
- Compares current hash to stored hash
- If changed: re-extract metadata (XMP keywords, rating, description, color label) and update catalog
- `--dry-run` shows what would change without applying
- Supports `--json`, `--log`, `--time` flags

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

#### 2.3 `dam edit` Command — **IMPLEMENTED** (v0.3.1, extended v0.4.4)

Implemented as `dam edit <asset-id> [--name <name>] [--description <text>] [--rating <1-5>] [--label <color>] [--clear-name] [--clear-description] [--clear-rating] [--clear-label]`. Supports `--json`. Rating, description, and color label changes trigger XMP write-back.

---

### Phase 3: Web UI Workflow Improvements — **PARTIALLY COMPLETE**

#### 3.1 Multi-Select & Batch Operations — **IMPLEMENTED** (v0.4.3–v0.4.4)

- **Checkbox selection** on browse cards (hover-visible, always-visible when any selected)
- **Fixed bottom toolbar** appearing when assets are selected:
  - Tag input with "+ Tag" / "− Tag" buttons
  - 5 rating stars with clear (×)
  - 7 color label dots with clear (×)
  - "Select page" / "Clear" buttons, selection count
- **Keyboard shortcuts**: Cmd/Ctrl+A selects all on page, Escape clears selection
- Selection state survives htmx pagination/sort swaps
- Backend: `POST /api/batch/tags`, `PUT /api/batch/rating`, `PUT /api/batch/label`
- Each individual operation triggers XMP write-back

#### 3.2 Description & Name Editing — **PARTIALLY IMPLEMENTED** (v0.4.3)

- ~~Inline-editable description field on asset detail page~~ — **done** (pencil icon, textarea, Save/Cancel, `PUT /api/asset/{id}/description`, XMP write-back)
- Inline-editable asset name — not yet implemented in web UI (editable via CLI `dam edit --name`)

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

#### ~~4.2 XMP Write-Back~~ — **IMPLEMENTED** (v0.4.1–v0.4.4)

Rating (v0.4.1), tags (v0.4.2), description (v0.4.3), and color label (v0.4.4) are written back to `.xmp` recipe files on disk whenever changed via CLI or web UI. Uses string-based find/replace to preserve XMP structure. Re-hashes files and updates catalog after modification. Enables bidirectional sync with CaptureOne.

#### 4.3 Export Command

```
dam export <query> --target <path> [--format <preset>] [--include-sidecars]
```

Export matching assets to a directory, optionally with sidecars. Useful for preparing files for delivery or for feeding into another tool.

#### 4.4 Collections

Named, manually curated groups of assets (separate from tags). A "Project: Wedding 2026" collection that holds specific picks regardless of their tags or location.

---

## Implementation Status Summary

| Feature | Status | Version |
|---------|--------|---------|
| `dam sync` | Done | v0.3.1 |
| `dam cleanup` | Done | v0.3.1, v0.3.4 |
| Search location health filters | Done | v0.3.3 |
| `dam update-location` | Done | v0.3.x |
| `dam edit` (CLI) | Done | v0.3.1 |
| XMP write-back (rating) | Done | v0.4.1 |
| XMP write-back (tags) | Done | v0.4.2 |
| XMP write-back (description) | Done | v0.4.3 |
| Web UI description editing | Done | v0.4.3 |
| Multi-select & batch operations | Done | v0.4.3–v0.4.4 |
| Color labels (CLI, web UI, XMP) | Done | v0.4.4 |
| XMP write-back (color label) | Done | v0.4.4 |
| `dam refresh` | Done | v0.4.5 |
| `dam import --dry-run` | Not started | — |
| Web UI name editing | Not started | — |
| Keyboard navigation | Not started | — |
| Saved searches / collections | Not started | — |
| Watch mode | Not started | — |
| Export command | Not started | — |

## Priority Recommendation

**Phase 1** is complete. **Phase 4.2** (XMP write-back) was pulled forward and is complete for all metadata fields (rating, tags, description, color label). **Phase 3.1** (multi-select & batch operations) and **Phase 3.2** (description editing) are complete. Color labels (v0.4.4) round out the metadata feature set with CaptureOne-compatible 7-color support.

The most impactful next steps are:

1. **Keyboard navigation (3.3)** — Arrow keys + number keys for rating would match the speed of CaptureOne's review workflow. Now that batch ops and color labels are in place, this is the main remaining gap for a competitive culling experience.

2. **`dam import --dry-run` (2.2)** — Useful safety net but lower priority than workflow speed improvements.

3. **Web UI name editing** — Minor gap in web UI completeness.
