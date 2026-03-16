# Proposal: Pending XMP Write-Back for Offline Volumes

> **Status**: Implemented (v2.3.3) — Phase 1 complete

When metadata is edited in the DAM (rating, color label, tags, description) while a volume is offline, the XMP write-back to `.xmp` recipe files is silently skipped. Those edits exist only in the YAML sidecar and SQLite catalog. When the volume comes back online, there is no mechanism to push them to the XMP files — and running `maki refresh` would actually overwrite DAM edits with the stale XMP content.

This proposal adds a **pending write-back queue** so DAM edits are never lost, even across offline/online transitions.

---

## Problem

The current write-back flow is fire-and-forget:

1. User edits rating/label/tags/description (CLI or web UI)
2. DAM updates YAML sidecar + SQLite catalog
3. DAM iterates XMP recipes, writes changes to files on online volumes
4. Offline volumes are skipped with `eprintln` warning — **no record is kept**

This creates a gap in the roundtrip workflow:

```
DAM edit (volume offline)  →  edits saved in YAML/SQLite only
Volume comes online        →  XMP files still have old values
maki refresh                →  XMP overwrites DAM edits (data loss!)
```

The problem is especially acute in a CaptureOne workflow: you rate/tag in DAM while the external drive is disconnected, then connect it and want to sync both directions — CaptureOne changes into DAM, and DAM changes into XMP.

---

## Design

### Core idea: dirty flag on recipes

Add a `pending_writeback` boolean (or timestamp) to each recipe record. When an XMP write-back is attempted but skipped due to an offline volume or missing file, the recipe is marked dirty. A new `maki writeback` command (or `--writeback` flag on existing commands) replays pending writes when the volume is back online.

### Data model

**SQLite** — new column on `recipes` table:

```sql
ALTER TABLE recipes ADD COLUMN pending_writeback INTEGER NOT NULL DEFAULT 0;
```

`1` = has unwritten DAM edits, `0` = clean.

**YAML sidecar** — new optional field on recipe records:

```yaml
recipes:
  - id: "abc123"
    pending_writeback: true   # only present when dirty
```

### Write-back flow (updated)

Current (`write_back_*_to_xmp_inner` methods in `query.rs`):

```
for each XMP recipe:
    if volume offline → eprintln warning, skip
    if file missing   → eprintln warning, skip
    write to file, re-hash, update recipe hash
```

Proposed:

```
for each XMP recipe:
    if volume offline → mark recipe pending_writeback=1, skip
    if file missing   → mark recipe pending_writeback=1, skip
    write to file, re-hash, update recipe hash
    clear pending_writeback=0
```

The flag captures the *intent* to write back. It doesn't record *what* changed — the current asset metadata (rating, label, tags, description) in the YAML sidecar is always the source of truth. The writeback command reads the current values and writes them all.

### New command: `maki writeback`

```
maki writeback [--volume <label>] [--asset <id>] [--all] [--dry-run]
```

- Without flags: processes only recipes with `pending_writeback=1`
- `--all`: writes back current metadata to all XMP recipes (useful for initial sync or force-push)
- `--volume`: limit to a specific volume
- `--asset`: limit to a specific asset
- `--dry-run`: report what would be written without modifying files

For each pending recipe:
1. Check volume is online, file exists
2. Read current asset metadata from catalog (rating, label, tags, description)
3. Apply all four write-back operations to the XMP file
4. Re-hash file, update recipe content hash in catalog + sidecar
5. Clear `pending_writeback` flag

Supports `--json`, `--log`, `--time`.

### Integration with `maki refresh`

The recommended workflow for a volume coming back online:

```bash
# 1. Push DAM edits to XMP (DAM wins for fields edited while offline)
maki writeback --volume MyDrive

# 2. Pull CaptureOne edits from XMP (CaptureOne wins for fields it changed)
maki refresh --volume MyDrive
```

Order matters: writeback first ensures DAM edits land in XMP. Then refresh picks up anything CaptureOne changed independently. For fields edited in *both* tools, the last writer wins — which is CaptureOne in this sequence (since refresh runs second). This is usually correct: if you explicitly changed something in CaptureOne after the DAM edit, you probably want the CaptureOne value.

For the opposite priority (DAM always wins), reverse the order or use `maki writeback --volume MyDrive` after refresh.

A combined convenience command could be considered:

```bash
maki sync-metadata --volume MyDrive   # writeback + refresh in one step
```

But this can be a later enhancement — the two-command workflow is explicit and predictable.

### Web UI integration

When a volume comes online (detected on next request), the web UI could show a notification: "Volume X has N pending write-backs. [Write back now]". This is optional and can be added later.

### Batch write-back on volume mount

For power users, a hook or config option:

```toml
[writeback]
auto_on_mount = true   # future enhancement
```

---

## Edge cases

**Recipe has no XMP file yet**: Some assets have no `.xmp` sidecar at all (e.g., JPEGs with only embedded XMP). The pending writeback only applies to existing XMP recipe records. Creating new XMP files from scratch is a separate feature (out of scope).

**Multiple edits while offline**: Each edit overwrites the previous in YAML/SQLite. The pending flag just says "something changed." When writeback runs, it writes the *current* values — intermediate states are not preserved. This is intentional: the YAML sidecar is the source of truth, not a change log.

**Volume stays offline indefinitely**: The pending flag persists. No timeout, no data loss. Writeback runs whenever the volume eventually comes online.

**Recipe file modified externally while offline**: Writeback overwrites the XMP file with DAM values. If CaptureOne also edited the same file, those edits are lost for the overlapping fields. The recommended workflow (writeback *then* refresh) handles this: refresh re-reads the file after writeback, but since writeback just wrote it, refresh sees no change. For truly concurrent edits to the *same* field, last-writer-wins is the only sane policy without a full merge engine.

**`maki refresh --media`**: The `--media` flag re-reads embedded XMP from JPEG/TIFF files. This is unaffected by pending writeback since embedded XMP is read-only (DAM doesn't write back to embedded XMP, only to `.xmp` sidecar files).

---

## Implementation plan

### Phase 1: Core dirty tracking + writeback command

1. **Schema**: Add `pending_writeback` column to `recipes` table (migration + initialize)
2. **Write-back methods**: Update `write_back_*_to_xmp_inner` in `query.rs` to set `pending_writeback=1` on skip, clear on success
3. **Catalog methods**: `mark_recipe_pending_writeback(recipe_id)`, `clear_recipe_pending_writeback(recipe_id)`, `list_pending_writeback_recipes(volume_filter)`
4. **YAML sidecar**: Add `pending_writeback` field to recipe serialization (skip if false for clean output)
5. **CLI command**: `maki writeback` with `--volume`, `--asset`, `--all`, `--dry-run`, `--json`, `--log`, `--time`
6. **Tests**: Unit tests for dirty tracking, integration tests for offline→online writeback cycle

### Phase 2: Convenience and UI (optional)

7. **Web UI**: Pending writeback indicator in nav bar or volume status
8. **Combined command**: `maki sync-metadata` (writeback + refresh)
9. **`maki refresh --writeback-first`**: Auto-run writeback before refresh

---

## Alternatives considered

**Timestamp-based conflict resolution**: Compare `modified_at` on DAM edits vs XMP file mtime. More complex, fragile (filesystem timestamps aren't always reliable, timezone issues, network drives). The dirty flag is simpler and more predictable.

**Full bidirectional merge**: Track per-field change history on both sides, three-way merge. Extremely complex, hard to reason about, overkill for the use case. The two-step workflow (writeback then refresh) achieves the same result with explicit user control.

**Change log / event sourcing**: Record every edit as an event, replay on writeback. More flexible but adds significant complexity. Since we only need the *current* state (not history), the dirty flag is sufficient.

---

## Summary

| Aspect | Current | Proposed |
|--------|---------|----------|
| Offline write-back | Silently skipped, lost | Queued via `pending_writeback` flag |
| Volume comes online | Manual refresh (may overwrite DAM edits) | `maki writeback` then `maki refresh` |
| Data model | No tracking | Boolean flag on recipe (SQLite + YAML) |
| Complexity | — | Low (flag + one new command) |
| Risk of data loss | High (refresh overwrites) | Low (explicit two-step workflow) |
