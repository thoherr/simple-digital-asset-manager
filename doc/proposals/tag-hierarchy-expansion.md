# Proposal: Tag Hierarchy Expansion

How hierarchical tags should be stored and searched — ensuring consistency between tags imported from CaptureOne/Lightroom and tags created or renamed in MAKI.

**Date:** 2026-04-03

**Status:** Investigation. Open for discussion.

---

## Problem

When CaptureOne stores a hierarchical tag `person|refugee|Nasiba`, it writes:

- **`dc:subject`**: flat individual components — `person`, `refugee`, `Nasiba` (for non-hierarchy-aware tools)
- **`lr:hierarchicalSubject`**: all ancestor paths — `person`, `person|refugee`, `person|refugee|Nasiba` (for hierarchy-aware tools)

```xml
<dc:subject>
  <rdf:Bag>
    <rdf:li>person</rdf:li>
    <rdf:li>refugee</rdf:li>
    <rdf:li>Nasiba</rdf:li>
  </rdf:Bag>
</dc:subject>
<lr:hierarchicalSubject>
  <rdf:Bag>
    <rdf:li>person</rdf:li>
    <rdf:li>person|refugee</rdf:li>
    <rdf:li>person|refugee|Nasiba</rdf:li>
  </rdf:Bag>
</lr:hierarchicalSubject>
```

MAKI's `merge_hierarchical_keywords` correctly imports these: hierarchical entries are kept as-is, flat `dc:subject` components that are part of any hierarchical tag are deduplicated. The asset ends up with three tags: `person`, `person|refugee`, `person|refugee|Nasiba`. **Import is correct.**

But when MAKI creates or renames a tag (e.g., `maki tag rename "Peter Schneider" "person|artist|musician|Peter Schneider"`), it stores only the leaf path — the asset gets `person|artist|musician|Peter Schneider` but NOT `person`, `person|artist`, or `person|artist|musician`.

This inconsistency causes:

1. **Search failures**: `tag:Peter Schneider` (prefix match) doesn't find `person|artist|musician|Peter Schneider` because it doesn't start with "Peter Schneider".
2. **Autocomplete confusion**: the browse page shows "Peter Schneider" as a suggestion but selecting it searches for the flat name, returning 0 results.
3. **Mixed tag counts**: on the tags page, C1-imported hierarchies show inflated counts (each ancestor is counted), while MAKI-created hierarchies show only the leaf count.

---

## Options

### Option A: Always expand ancestors (CaptureOne model)

When storing a hierarchical tag `a|b|c|d`, also store `a`, `a|b`, and `a|b|c`.

**Where expansion happens:**
- `maki tag` (add tag) — expand on write
- `maki tag rename` — expand new tag + descendants
- `maki import` — already expanded by C1/LR (preserve as-is)
- Web UI tag add — expand on write

**Pros:**
- Consistent with CaptureOne/Lightroom behavior
- Existing prefix search just works — `tag:musician` matches `person|artist|musician`
- No SQL changes needed
- Round-trip with C1/LR is clean

**Cons:**
- More tags per asset (a 4-level tag creates 4 entries)
- Tag counts on tags page count ancestors (as C1 already does — see screenshot)
- `tag rename` and `tag clear` must handle ancestors (remove old ancestors, add new ones)
- Redundant data in YAML sidecars

### Option B: Component search (match any path segment)

Don't expand ancestors. Instead, enhance the search to match any component.

`tag:musician` generates SQL: `WHERE value LIKE 'musician%' OR value LIKE '%|musician%'`

**Pros:**
- Clean storage — one tag per concept
- No redundant ancestors
- Tag counts reflect actual unique tags

**Cons:**
- Inconsistent with C1/LR-imported data (which already has ancestors)
- Leading `%` in SQL LIKE is slower (though marginal with json_each)
- Two different tag "shapes" in the same catalog (expanded from C1, unexpanded from MAKI)
- Autocomplete still needs fixing for component display
- XMP writeback would write only the leaf — C1 wouldn't see the hierarchy

### Option C: Expand on write, deduplicate on display

Like Option A, but the tags page and UI suppress ancestor-only entries when displaying counts.

**Where expansion happens:** same as Option A.

**Display adjustment:**
- Tags page shows the tree with counts based on leaf tags only (not ancestors)
- Or: two count columns — "direct" (leaf) and "total" (including ancestor matches)

**Pros:**
- Consistent storage and search
- Honest tag counts
- Clean UI

**Cons:**
- More complex display logic
- Still has redundant storage

### Option D: Configurable behavior

A config option `[tags] expand_ancestors = true` (default: true for C1/LR compatibility).

When true: Option A behavior.
When false: Option B behavior.

**Cons:**
- Two code paths to maintain
- Confusing for users who switch

---

## Current MAKI Behavior

### Import from C1/LR
- `lr:hierarchicalSubject` entries are stored as-is (pipe-separated)
- `dc:subject` flat keywords that are components of hierarchical tags are deduplicated (removed)
- But C1 writes ALL ancestor paths into both fields, so ancestors are preserved

### `maki tag add`
- Stores exactly the tag provided — no ancestor expansion

### `maki tag rename`
- Replaces exact and prefix matches — no ancestor expansion for the new tag

### `maki search tag:X`
- Uses `LIKE 'X%'` prefix matching against the JSON tags array
- Matches `X`, `X|child`, `X|child|grandchild` — but NOT `parent|X` or `parent|X|child`

### XMP writeback
- Writes tags to `dc:subject` (converting `|` to `/` for flat keywords)
- Writes hierarchical tags to `lr:hierarchicalSubject` (keeping `|`)
- Does NOT expand ancestors during writeback
- Should match CaptureOne format: `dc:subject` gets flat component names, `lr:hierarchicalSubject` gets all ancestor paths

---

## Recommendation

**Option A** — always expand ancestors. Rationale:

1. **Consistency**: C1/LR already do this. Fighting the industry convention creates a permanent mismatch in mixed catalogs.
2. **Simplicity**: the existing prefix search works unchanged. No SQL modifications.
3. **Round-trip**: XMP writeback should also expand ancestors, so C1/LR see the full hierarchy when reading MAKI's XMP files.
4. **Tag counts**: the "inflated" counts from ancestors are what C1/LR users expect. The tags page already handles this by showing a tree with per-node counts.

### Implementation plan

**Phase 1: Expand on tag operations**
1. When `maki tag <asset> "a|b|c|d"` adds a tag, also add `a`, `a|b`, `a|b|c` (skip if already present)
2. When `maki tag rename` creates new tags, expand the new tag's ancestors
3. When `maki tag <asset> --remove "a|b|c|d"` removes a tag, also remove ancestor tags that are no longer needed by any other descendant
4. When `maki tag clear` removes all tags, straightforward (remove everything)

**Phase 2: Expand on XMP writeback (match CaptureOne format)**
1. When writing to `lr:hierarchicalSubject`, write all ancestor paths (not just the leaf) — matching what CaptureOne writes
2. When writing to `dc:subject`, write flat individual component names (not pipe-separated paths) — matching CaptureOne's flat keyword format

**Phase 3: Fix web UI autocomplete**
1. Browse page autocomplete should insert the full hierarchical path when a suggestion is selected
2. Or: with ancestor expansion, inserting "musician" works because the asset has that as a standalone tag

**Phase 4: Cleanup command**
1. `maki tag expand-ancestors [query] --apply` — retroactively expands ancestors for existing tags that were created without expansion (from MAKI rename/add operations)

---

## Open Questions

1. **Should ancestor removal be smart?** When removing `person|artist|musician|Peter Schneider`, should `person|artist|musician` be removed too — or only if no other descendant uses it? Smart removal is safer but more complex.

2. **Should the tags page show two counts?** "82" (direct) vs "82 (3447)" (including ancestor matches). This would make the inflation transparent.

3. **Should `merge_hierarchical_keywords` during import be changed?** Currently it deduplicates flat keywords that are components of hierarchical tags. With ancestor expansion, this dedup might remove tags we actually want to keep.

4. **Performance**: an asset with 10 hierarchical tags at depth 4 would have ~40 tag entries instead of ~10. Is this a concern for search performance or YAML file size?
