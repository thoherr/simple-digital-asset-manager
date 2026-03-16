# Proposal: Export-Based Preview Generation

## Motivation

Currently, previews are generated from the **primary variant** of each asset — typically the RAW file. This means the preview shows the unprocessed image: no color grading, no cropping, no exposure corrections. For a DAM used alongside CaptureOne or Lightroom, this creates a disconnect between what you see in the DAM and what the final image actually looks like.

This matters most during **retrieval**: when browsing the catalog months or years later to find images for a project, portfolio, or print, you want to see the edited result — not a flat RAW rendering. The preview should give a reliable impression of the end result.

### Why not apply processing instructions?

CaptureOne's adjustments (stored in `.cos` session files and partially in `.xmp` sidecars) are proprietary and complex — curves, color balance, layers, masks, lens corrections, local adjustments. No open-source tool can interpret them. The XMP sidecar only contains a small subset of metadata (rating, keywords, label), not the actual image processing parameters. LibRaw/dcraw produce a basic RAW development that bears little resemblance to the CaptureOne-processed result.

**The practical solution is to use export files (JPEG, TIFF) that CaptureOne has already rendered.** These contain the full processing result and are standard image formats that the `image` crate handles natively.

---

## Current Behavior

1. During import, a preview is generated for each variant individually (keyed by content hash)
2. The web UI and `maki show` display the preview of the **first variant** (primary/original)
3. For a RAW+JPEG asset, the RAW preview (dcraw rendering) is shown, even though a processed JPEG export exists
4. The `generate-previews` command iterates variants and generates per-variant previews, but the display logic doesn't prefer one over another

## Proposed Change

### Core idea: asset-level "best preview" selection

Instead of always displaying the primary variant's preview, introduce a preference order for which variant's preview represents the asset:

```
Export > Processed > Original
```

When an asset has an Export variant (e.g., a CaptureOne-processed JPEG/TIFF), its preview is used as the asset's display preview. This requires no new image processing — the export file is a standard format that the existing preview generator already handles well.

### Implementation

#### Phase 1: Prefer export variants for display

**Change the preview selection logic** in the places that choose which preview to display:

1. **Web UI asset detail** (`src/web/routes.rs`): Currently uses `details.variants.first()` to find a preview. Change to scan variants in role preference order: `Export` → `Processed` → `Original`, and use the first one that has a preview on disk.

2. **Web UI browse grid** (`src/web/routes.rs`): Same logic — the browse card thumbnail should show the best available preview.

3. **`maki show` output** (`src/main.rs`): Currently shows the primary variant's preview path. Show the best-available preview instead.

This is a **display-only change** — all per-variant previews continue to exist. We're just choosing which one to show as the asset's representative image.

**Estimated scope**: Small. A helper function `best_preview_hash(variants, preview_generator) -> Option<String>` that returns the content hash of the preferred variant, used in 2–3 display sites.

#### Phase 2: Regenerate previews from better sources

Add a mode to `generate-previews` that upgrades asset previews when a better source variant exists:

```
maki generate-previews --upgrade
```

This would:
- Scan all assets with multiple variants
- For each asset where the current display preview comes from an `Original` variant but an `Export` or `Processed` variant exists, (re)generate the preview from the better source
- Report: "Upgraded N previews (from export variants)"

This handles the case where exports were imported after the initial RAW import — the RAW preview already exists, but the export's preview may not have been generated yet (or hasn't been preferred).

**Estimated scope**: Medium. Needs to iterate assets, check variant roles, compare against existing previews, and selectively regenerate.

#### Phase 3: Auto-upgrade on group/import (optional)

When an export variant is added to an asset (via `import`, `auto-group`, or `group`), automatically regenerate the asset's preview from the new variant if it's a better source than the current one.

This ensures previews stay up-to-date without requiring a manual `generate-previews --upgrade` step.

**Estimated scope**: Small addition to import and group flows — check if the newly added variant is preferred over the existing preview source, and regenerate if so.

---

## Design Considerations

### Which variant is "best"?

Role-based preference: `Export` > `Processed` > `Original`. Within the same role, prefer image formats over others (JPEG/TIFF/PNG over video). If multiple exports exist, use the one with the largest file size (likely the highest quality).

### What if the export is a low-res JPEG?

The preview is already a thumbnail (800px longest edge). Even a moderately sized JPEG export will produce a good preview. If the export is smaller than the preview size, we still use it — it's a more accurate representation of the final result than a dcraw RAW rendering.

### What about video assets?

No change for video. Video previews are extracted via ffmpeg (a single frame), and there's no "export variant" concept that would improve this.

### Per-variant previews are preserved

All variants continue to have their own previews (keyed by content hash). The change is only about which preview is chosen as the **asset's representative thumbnail** in the UI and CLI output. The asset detail page could still show per-variant previews if desired.

### No new dependencies

This uses the existing preview generator, variant roles, and image handling. No new crates or external tools.

---

## Summary

| Phase | Change | Scope | Status |
|-------|--------|-------|--------|
| 1 | Prefer export/processed variant previews for display | Small | **IMPLEMENTED** |
| 2 | `generate-previews --upgrade` to regenerate from better sources | Medium | **IMPLEMENTED** |
| 3 | Auto-upgrade preview on import/group when export is added | Small | **IMPLEMENTED** |

All three phases are implemented. The key insight for Phase 3 is that since import already generates previews for every variant and Phase 1's display logic selects the best variant's preview, no additional code is needed for import or group/auto-group — the export variant's preview is automatically preferred once it exists.
