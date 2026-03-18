# Proposal: Similarity Browse and Stacking

## Motivation

The SigLIP embedding-based similarity search works very well for finding visually related images. Currently it's used in stroll navigation and the detail page's "similar images" section, but there's no easy way to go from "these images are similar" to "let me process them" (tag, stack, cull).

Key use case: burst shots and near-duplicates. A photographer shoots 15 similar frames, wants to keep the best 2-3 rated, and stack the rest behind the hero shot so they don't clutter browsing.

## Grouping vs. Stacking

**Grouping** (variants) merges assets into one — the others become alternate versions of a single asset, sharing identity, rating, tags, and description. This is appropriate for RAW + JPEG of the same shot, edited exports, or different crops.

**Stacking** keeps assets independent — each retains its own rating, tags, and description. The stack pick (highest-rated or manually chosen) represents the stack in the browse grid. Expanding the stack shows all members. Unstacking is non-destructive.

For burst shots and visually similar images, **stacking is the right concept**. These are genuinely different photographs (different moment, slightly different composition) — not variants of the same asset. The existing `auto-group` command groups by filename stem (RAW+JPEG pairs). Similarity-based organization should use stacking instead.

Summary:
- `group` / `auto-group` — by filename stem, creates variants (same asset)
- `stack` / `auto-stack` — by similarity or manual selection, keeps independent assets, collapses in browse

## Phase 1: Browse with Similarity Scores (implemented)

- **"Browse similar" button** on the detail page → navigates to browse with `similar:<asset_id>`.
- **Similarity score on browse cards** — teal percentage badge when `similar:` is active.
- **`min_sim:` filter** — percentage threshold (e.g. `similar:abc123 min_sim:90` for >= 90%).
- **Sort by similarity** — auto-default when `similar:` is active, with toolbar button.
- Source asset included at 100%. Results fetched on single page for correct sorting.

## Phase 2: Stack by Similarity (Targeted) — implemented in v4.0.2

**Goal:** One-click stacking of similar images around a hero shot from the detail page.

- **"Stack similar" button** on the detail page — finds all assets above a configurable threshold and stacks them with the current asset as pick.
- **Threshold** configurable via `[ai] similarity_stack_threshold` in `maki.toml` (default ~85, meaning 85% similarity).
- **Workflow:** Browse similar → review the set → click "Stack similar" → burst collapses behind the hero shot in browse.
- Assets keep their individual ratings, tags, descriptions. The pick is the stack representative.
- If the user later rates another member higher, they can change the pick.

## Phase 3: Auto-Stack by Similarity (Catalog-wide)

**Goal:** Discover natural visual clusters across the entire catalog and propose stacks.

- `maki auto-stack --threshold 85` — scan all embedded assets, find clusters where pairwise similarity exceeds threshold, propose as stacks.
- Pick selection: highest-rated asset in each cluster, or first imported if no ratings.
- `--dry-run` for review before applying (show proposed stacks with member counts and similarity ranges).
- `--apply` to create the stacks.
- Clustering algorithm: greedy connected-components or single-linkage clustering over the embedding similarity matrix. Similar to face clustering but over whole-image embeddings.
- Computationally O(n²) pairwise, but can be optimized:
  - Batch dot products over the contiguous embedding buffer (already in `EmbeddingIndex`)
  - Skip pairs below a cheap early-exit threshold
  - For very large catalogs, approximate nearest neighbors (e.g. IVF or random projection)

## Overlap with Default Filter / Culling

This feature complements the `[browse] default_filter` and `rest` tag workflow:

1. Browse similar → review the burst
2. Stack them (they collapse in browse via stack collapsing, hero shot visible)
3. Optionally tag the non-picks as `rest` for additional filtering
4. Or just rely on stack collapse — expanding shows all members when needed

Stacking is cleaner than tagging for bursts because the images stay visually associated with their best shot and can be expanded/collapsed on demand.

## Implementation Notes

- Similarity scores come from `EmbeddingIndex::search()` which returns `Vec<(String, f32)>` (asset_id, cosine similarity).
- The `similar:` filter is parsed in `query.rs` behind `#[cfg(feature = "ai")]`.
- Phase 1 flow: `similar:` → resolved in browse routes via `resolve_similar_filter()` → returns IDs + scores → scores populated on `AssetCard.similarity` → displayed as badge, used for sorting.
- Stacking infrastructure already exists: `StackStore` in `catalog.rs`, `maki stack` CLI command, stack collapse in browse grid, stack pick selection.
- Phase 2 needs: embedding lookup + threshold filter + call to `StackStore::create_stack()` with the matching asset IDs.
- Phase 3 needs: iterate all embeddings, compute pairwise similarities, cluster, propose stacks.
