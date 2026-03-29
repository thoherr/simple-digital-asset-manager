# Proposal: Card-First Workflow

Enable MAKI to manage the complete lifecycle from memory card to archive, eliminating the manual copy step and allowing culling on previews before any files reach the working drive.

**Date:** 2026-03-29

**Status:** Phase 1 implemented (2026-03-29). Phase 2–3 pending.

---

## Motivation

Today's recommended workflow is:

1. Copy files from memory card to working SSD (manually, via rsync/Finder)
2. `maki import` from the working SSD
3. Cull, rate, tag
4. Archive and back up

Step 1 is outside MAKI's control. The user copies *everything* — including the 70% of shots they'll reject — to fast storage before MAKI even sees the files. On a 1000-shot wedding day with 50 MB RAW files, that's 50 GB copied, of which 35 GB may be rejects that are never opened.

The idea: let MAKI import directly from the memory card, generate previews, and let the user cull on smart previews *before* copying keepers to the working drive. This eliminates wasted copies and lets the photographer start reviewing immediately after inserting the card.

---

## Proposed Workflow

```bash
# 1. Mount card, register as transient volume
maki volume add "Card-2026-03-29" /Volumes/CARD --purpose media

# 2. Import from card (hash + preview generation, no file copy)
maki import /Volumes/CARD/DCIM \
  --add-tag "shoot:johnson-wedding" --auto-group --smart --log

# 3. Eject card — previews and smart previews are local, culling works offline

# 4. Cull in web UI on smart previews — rate keepers, skip rejects
maki serve

# 5. Copy only keepers to working drive
maki relocate --query "volume:Card-2026-03-29 rating:1+" \
  --target "Work SSD" --create-sidecars --log

# 6. Clean up card volume
maki cleanup --volume "Card-2026-03-29" --apply
maki volume remove "Card-2026-03-29" --apply

# 7. Open in CaptureOne — files + XMP sidecars with MAKI ratings/tags are on the SSD
```

---

## Required Changes

### 1. Volume purpose `media` (low effort)

A new volume purpose for transient source media (memory cards, card readers, camera USB connections). Semantics:

- `media` volumes are expected to go offline quickly
- `backup-status` ignores `media` volumes (a file only on a card is not "backed up")
- `duplicates --cross-volume` treats media→working copies as expected, not flagged

**Implementation:** Add `"media"` to the `VolumePurpose` enum. Update `backup-status` and `duplicates` to handle the new purpose. Straightforward.

### 2. XMP sidecar creation on relocate: `--create-sidecars` (medium effort)

When relocating files to a new volume, optionally create `.xmp` sidecar files at the destination for assets that have metadata (ratings, tags, labels, descriptions) but no existing XMP recipe. This enables external tools (CaptureOne, Lightroom) to pick up MAKI metadata immediately.

**Implementation:** After copying files, for each asset that has metadata but no XMP recipe on the target volume, generate a minimal XMP sidecar with the current rating, tags, label, and description. Register it as a recipe in the catalog and YAML sidecar. Re-use the existing XMP writing code from `writeback`.

**Alternative:** Instead of a relocate flag, a standalone `maki create-sidecars` command that generates XMP files for assets that don't have them. More flexible (works independently of relocate) but adds another command.

**Open question:** Should this be `--create-sidecars` on relocate, a standalone command, or both?

### 3. Incremental card import (no change needed)

MAKI already handles this: re-running `maki import` on the same path skips files whose content hash is already in the catalog. Multi-session events (photo workshops, multi-day events) work naturally — import after each card swap, MAKI only processes new files.

### 4. Volume cleanup convenience (low effort, optional)

A shorthand for the card cleanup sequence:

```bash
maki volume remove "Card-2026-03-29" --apply
```

This already works — `volume remove` cleans up all location/recipe records and orphaned assets. The user just needs to know about it. Could add a `--purpose media` filter to `volume list` to find stale card volumes.

---

## Design Considerations

### Volume proliferation

Every card insertion creates a volume. Over a year, a busy photographer might accumulate 100+ card volumes. This is manageable — volumes are lightweight (one entry in `volumes.yaml`) and old ones are cleaned up with `volume remove`. But it's worth considering:

- **Auto-naming:** Could auto-generate volume names from the card label or mount path (e.g., `Card-EOS_DIGITAL-2026-03-29`).
- **Ephemeral volumes:** A `--ephemeral` flag on `volume add` that marks the volume for automatic cleanup after all its files have been relocated.
- **Bulk cleanup:** `maki volume list --purpose media --offline` to find stale card volumes, piped into removal.

### Card speed during import

Import from a slow SD card (30–90 MB/s read) with preview generation will be slower than importing from an SSD. For a 1000-file shoot:

- Hashing: ~15–30 min on UHS-I SD, ~2–5 min on SSD
- Preview generation: similar (CPU-bound, not I/O-bound)
- Smart preview generation: slightly slower from card (reads full file)

Mitigation: the user can eject the card after import and continue culling on previews. Or use a fast card reader (UHS-II, CFexpress) where the speed difference is smaller.

### Offline culling

After card ejection, the card volume goes offline. Smart previews (2560px) enable zoom and pan in the web UI. Regular previews (800px) work for grid browsing. The user can:

- Rate, tag, label, describe — all stored in catalog + YAML
- Browse with zoom via smart previews
- Build collections, create stacks

They cannot:
- Access original files (expected — they're on the card)
- Export originals (must relocate first)
- Verify file integrity (card offline)

This is the same behavior as any offline volume, just applied earlier in the workflow.

### CaptureOne/Lightroom integration

The key question: when files land on the working SSD, can external tools see MAKI's metadata?

- **With `--create-sidecars`:** Yes. XMP files are created alongside the media files with ratings, tags, labels, and descriptions. CaptureOne and Lightroom read these on import.
- **Without:** No. The metadata exists only in MAKI's catalog and YAML sidecars. The user would need to run `maki writeback` after creating XMP recipes manually.

The `--create-sidecars` feature is the critical enabler for this workflow.

### Existing `maki import` from card

Note: `maki import /Volumes/CARD/DCIM` already works today. The card just needs to be registered as a volume first. The missing pieces are:

1. The `media` purpose (for correct backup-status behavior)
2. XMP sidecar creation on relocate (for external tool integration)
3. Documentation and best practices

---

## Alternatives Considered

### `maki ingest` command

A combined command that does `volume add` + `import` + (optionally) `relocate` in one step:

```bash
maki ingest /Volumes/CARD/DCIM --target "Work SSD" --add-tag "shoot:wedding"
```

**Rejected for now.** This hides too much and makes it harder to insert a culling step between import and relocate. The separate commands give the user control over when to cull and what to copy. Could revisit as a convenience wrapper later.

### Renaming `relocate` to `copy`

`maki copy` is more intuitive for the card→SSD step than `maki relocate`.

**Possible but not necessary.** `relocate` accurately describes what happens (files move between volumes in the catalog). Adding `copy` as an alias is low-cost and might improve discoverability. But having two names for the same command could confuse. Defer unless users ask for it.

### Direct card import without volume registration

Skip `volume add` — just import and auto-create a transient volume.

**Risky.** Auto-creating volumes from import paths could accidentally register unexpected directories. Better to keep volume registration explicit.

---

## Implementation Plan

### Phase 1: Foundation (low effort)

1. Add `media` volume purpose to the enum
2. Update `backup-status` to exclude `media` volumes from coverage calculation
3. Update `duplicates` to handle `media` volumes appropriately
4. Document the card-first workflow pattern (even without `--create-sidecars`, the basic flow works)

### Phase 2: XMP sidecar creation (medium effort)

1. Implement XMP sidecar generation for assets without existing XMP recipes
2. Add `--create-sidecars` flag to `maki relocate`
3. Consider standalone `maki create-sidecars` command
4. Update documentation with CaptureOne/Lightroom integration examples

### Phase 3: Convenience (low effort, optional)

1. Auto-naming for card volumes
2. `--ephemeral` flag for automatic volume cleanup
3. `maki volume list --purpose media --offline` for stale volume discovery
4. Consider `copy` alias for `relocate`

---

## Open Questions

1. **Should `--create-sidecars` be the default on relocate, or opt-in?** Opt-in is safer (doesn't create unexpected files), but the card-first workflow always wants it.

2. **Should we support importing from unmounted card images (`.dmg`, `.iso`)?** Probably out of scope, but worth noting.

3. **Is `media` the right name for the purpose?** Alternatives: `source`, `card`, `capture`, `ingest`. `media` is generic enough to cover USB-connected cameras, card readers, and other transient sources.

4. **Should the web UI have a "Card Import" workflow?** A guided wizard: select card → import → cull → copy keepers. This would make the workflow more discoverable but is significant UI work.
