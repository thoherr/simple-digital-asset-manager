# Proposal: Stroll Page — Graph-Based Visual Exploration

> **Status**: Implemented (v2.3.0 / v2.3.1 / v2.3.2) — Phase 1 (visual similarity), Phase 2 (filter integration), and level-2 transitive neighbors are complete. v2.3.1 renamed "depth" to "fan-out", added elliptical satellite layout, and direction-dependent L2 arc radius. v2.3.2 added stroll modes (Nearest/Discover/Explore) and cross-session filtering.

A new `/stroll` page for navigating the catalog by traversing connections between assets — primarily visual similarity, but extensible to shared tags, nearby dates, nearby GPS locations, and more.

---

## Motivation

The current browse experience is list/grid-based: you define a query, get results, paginate. The detail page shows one asset at a time. The lightbox is linear (prev/next within the current result set).

None of these let you **explore** the catalog organically — following visual threads, discovering unexpected connections, wandering from one image to related ones without a predefined query. The stroll page fills that gap: pick any asset, see what's nearby in embedding space, click through, repeat. It's the DAM equivalent of Wikipedia link-surfing.

---

## UX Design

### Layout

```
+------------------------------------------------------------------+
|  nav bar  [Browse] [Stroll] [Tags] ...           [?] [theme]    |
+------------------------------------------------------------------+
|                                                                    |
|                    [ o ]  [ o ]  [ o ]                            |
|                   /       |       \                               |
|               [ o ]   [ CENTER ]   [ o ]                          |
|                   \       |       /                               |
|                    [ o ]  [ o ]  [ o ]                            |
|                                                                    |
|  +--------------------------------------------------------------+ |
|  | filter bar (optional, collapsed by default)                  | |
|  +--------------------------------------------------------------+ |
+------------------------------------------------------------------+
```

- **Center image**: The current asset of interest. Shown as a large preview (similar to detail page size, ~600px). Overlay shows name/filename, rating stars, color label dot. Click opens lightbox; `d` opens detail page — same shortcuts as browse.
- **Satellite images**: 8-12 neighbors arranged in a radial/circular layout around the center. Shown as thumbnails (~120-150px). Connected to center by subtle lines (SVG or CSS).
- **Focused satellite**: On hover or arrow-key navigation, the focused satellite scales up to a mid-size (~250px) with a smooth transition. Shows name and similarity score as an overlay.
- **Level-2 neighbors**: When a satellite has focus and the fan-out slider is > 0, its own nearest neighbors (excluding the center and all level-1 satellites) appear as smaller thumbnails radiating outward from the focused satellite. The L2 arc radius adapts based on the satellite's direction from the center — satellites near the viewport edges use a shorter radius to avoid overflow. See [Level-2 Transitive Neighbors](#level-2-transitive-neighbors) below for full details.
- **Navigation**: Click or Enter on a focused satellite makes it the new center. The view animates: old center shrinks away, new center grows, new satellites load. Clicking an L2 thumbnail also navigates (becomes the new center).

### Interaction Model

| Input | Action |
|-------|--------|
| Arrow keys (or Tab) | Cycle focus through satellite images |
| Enter / Click satellite | Navigate: satellite becomes new center, new neighbors load |
| Click center image | Open lightbox (same as browse) |
| `d` | Open detail page for focused asset (center if no satellite focused) |
| `Escape` | Return to browse page (or previous page) |
| `0-5` | Set rating on focused asset |
| `r/o/y/g/b/p/u/x` | Set color label on focused asset |
| Back button | Navigate to previous center (browser history) |
| `f` | Toggle filter bar |
| Fan-Out slider (0–10) | Controls how many level-2 neighbors appear per focused satellite (0 = off, configurable max) |

### Arrow Key Navigation for Radial Layout

Satellites are arranged in angular positions (like clock positions). Arrow key mapping options:

**Option A — Angular (recommended):** Left/Right rotate focus clockwise/counterclockwise around the ring. Up focuses the center image. Down from center focuses the nearest satellite.

**Option B — Spatial:** Arrow keys move focus to the nearest satellite in that screen direction. More intuitive for irregular layouts but harder to implement predictably.

Option A is simpler and more predictable. The focused position is just an index into the sorted satellite array, and Left/Right wrap around.

### Entry Points

- **Nav bar**: "Stroll" link (always visible, like Tags/Collections)
- **Detail page**: "Stroll from here" button/link
- **Browse page**: Right-click context menu or keyboard shortcut on a card
- **URL**: `/stroll?id=<asset-id>` — bookmarkable, shareable
- **No ID**: `/stroll` without an `id` param picks a random asset with an embedding

### Filter Integration (Phase 2)

A collapsible filter bar at the bottom (or top) of the stroll page, reusing the same search controls as the browse page. When active, neighbor queries are restricted to the filter subset. This means: "stroll through my 4+ star landscapes" or "stroll within this collection."

Implementation: the `/api/stroll/neighbors` endpoint accepts a `q` parameter (same query syntax as browse). The backend first computes similarity neighbors, then filters them against the query. If too few pass the filter, increase the raw limit and re-filter (up to a cap).

### Level-2 Transitive Neighbors

When exploring from a center asset, the level-1 satellites show direct neighbors. Level-2 (L2) transitive neighbors extend this by showing _each satellite's_ neighbors — the neighbors-of-neighbors — giving a deeper peek into the graph without navigating away.

#### Fan-Out Slider

A range slider in the bottom-left control panel (alongside any existing controls) sets how many L2 thumbnails to show per focused satellite:

- **Range**: 0–10 (configurable)
- **Default**: 0 (off — no L2 thumbnails shown)
- **Label**: Shows the current value next to the slider

When fan-out is 0, stroll behaves exactly as before. At fan-out 1–10, focusing a satellite triggers an L2 fetch (see below) and displays that many secondary thumbnails around it.

#### Lazy-Loading

L2 neighbors are fetched on demand: when a satellite receives focus (via mouse hover or arrow-key navigation) and the fan-out slider is > 0, a request is made to `/api/stroll/neighbors` for that satellite's asset ID. This avoids loading L2 data for all satellites upfront, keeping the initial page load fast.

#### Deduplication

The L2 result set is filtered client-side before rendering. Any asset that is the current center or already present as a level-1 satellite is excluded from the L2 thumbnails. This prevents visual duplication — if the center's neighbor A also lists neighbor B (which is already a level-1 satellite), B does not appear again as an L2 thumbnail around A.

#### Caching

L2 results are cached in memory (keyed by satellite asset ID). Once a satellite's L2 neighbors have been fetched, re-focusing that satellite renders them instantly from the cache without another network request. The cache is cleared on navigation (when the center changes and a new set of level-1 satellites loads).

#### Visual Design

- **Size**: L2 thumbnails render at approximately 60% of the satellite thumbnail size, making the hierarchy visually clear (center > satellite > L2).
- **Position**: L2 thumbnails are arranged in a 90-degree arc that radiates outward from the focused satellite, away from the center. The arc direction is determined by the satellite's angular position relative to the center. The arc radius is direction-dependent: satellites near the left/right edges of the viewport use a shorter L2 radius to prevent thumbnails from overflowing the visible area.
- **Connection lines**: Thin dashed SVG lines connect each L2 thumbnail back to its parent satellite. These are visually distinct from the solid lines connecting satellites to the center.
- **Animation**: L2 thumbnails fade in when they appear (CSS opacity transition), giving a smooth reveal rather than a sudden pop.
- **Resize handling**: L2 thumbnail positions are recalculated on window resize, just like satellite positions, so the layout stays coherent at any viewport size.

#### Navigation

Clicking an L2 thumbnail navigates: the clicked asset becomes the new center, its neighbors load as level-1 satellites, and the cycle continues. This is the same behavior as clicking a level-1 satellite — L2 thumbnails are full navigation targets, not just decorative.

---

## Connection Types

### Phase 1: Visual Similarity (MVP)

Uses the existing `EmbeddingIndex` / `EmbeddingStore` infrastructure. The `/api/asset/{id}/similar` endpoint already returns up to 20 similar assets with scores. For the stroll page, we need a dedicated endpoint (or extend the existing one) that returns a fixed number of neighbors with preview URLs and metadata.

### Phase 2: Multi-Dimensional Connections (Future)

Each connection type produces a set of "nearby" asset IDs with a score and a label:

| Type | How | Score | Label shown |
|------|-----|-------|-------------|
| Visual similarity | SigLIP embedding dot product | 0.0-1.0 | "87% similar" |
| Shared tags | Jaccard index of tag sets | 0.0-1.0 | "3 shared tags" |
| Nearby date | `abs(date_a - date_b)` in hours | inverse | "2 hours apart" |
| Nearby location | Haversine distance in km | inverse | "1.2 km away" |
| Same collection | Shared collection membership | binary | "in 'Favorites'" |
| Same person | Shared detected person | binary | "Alice" |

In multi-mode, the connection lines could be colored or styled by type (e.g., solid = similarity, dashed = shared tags, dotted = date). A mode toggle or dropdown selects which connection type(s) are active.

For the MVP, only visual similarity. The architecture should make it easy to add more types later — each is just a function `(asset_id, limit) -> Vec<(asset_id, score, label)>`.

---

## Technical Design

### New API Endpoint

```
GET /api/stroll/neighbors?id=<asset-id>&limit=12&q=<optional-query>
```

Response:

```json
{
  "center": {
    "asset_id": "...",
    "name": "DSC_4521.NEF",
    "preview_url": "/preview/ab/abcdef123456.jpg",
    "smart_preview_url": "/smart-preview/ab/abcdef123456.jpg",
    "rating": 4,
    "color_label": "Green",
    "format": "nef",
    "created_at": "2026-02-15T10:30:00Z"
  },
  "neighbors": [
    {
      "asset_id": "...",
      "name": "DSC_4522.NEF",
      "preview_url": "...",
      "rating": 3,
      "color_label": null,
      "similarity": 0.87,
      "connection": "visual"
    },
    ...
  ]
}
```

Backend logic (Phase 1):
1. Look up the asset's stored embedding (error if none)
2. Query `EmbeddingIndex.search()` for `limit * 2` candidates (over-fetch for filtering headroom)
3. If `q` is provided, filter candidates against the query (resolve IDs via `build_search_where` with `similar_asset_ids`)
4. Take top `limit` results
5. Load preview URLs and metadata for center + neighbors

### New Page Route

```
GET /stroll  (with optional ?id=<asset-id>)
```

Server-rendered Askama template with the initial center asset data embedded. Neighbors loaded via the API endpoint (htmx or fetch). This allows the initial page load to show the center image immediately while neighbors load asynchronously.

### Frontend Architecture

**CSS Layout:** CSS `position: absolute` within a relative container. Satellites positioned using `transform: translate()` computed from angular positions on an elliptical orbit. Center image at 50%/50%. The elliptical layout uses separate horizontal and vertical radii to better fill widescreen viewports.

```
angle_i = (2 * PI * i) / num_neighbors
x_i = center_x + radius_x * cos(angle_i)
y_i = center_y + radius_y * sin(angle_i)
```

**Connection lines:** SVG overlay layer (absolutely positioned, pointer-events: none) with `<line>` elements from center to each satellite.

**Transitions:** CSS transitions on `transform`, `width`, `height`, `opacity` for smooth focus/navigation animations. On navigation, the old satellites fade out, new ones fade in. The center image cross-fades.

**State management:** A single JS IIFE (`window.damStroll`) managing:
- `centerId` — current center asset ID
- `neighbors[]` — current neighbor data
- `focusIndex` — which satellite has keyboard focus (-1 = center)
- `history[]` — breadcrumb trail for back navigation
- `fanOut` — current fan-out slider value (0–10, default 0)
- `l2Cache` — map of satellite asset ID to fetched L2 neighbor arrays (cleared on navigation)

**History:** Each navigation pushes to `window.history` via `pushState({ id })`, so the browser back button works naturally. URL updates to `/stroll?id=<new-id>`.

### Dark Mode

Reuses existing CSS custom properties. Connection lines use `var(--text-muted)`. Focused satellite border uses `var(--accent)`.

### Responsive Considerations

On narrow viewports (<768px), the radial layout could collapse to a horizontal scrollable strip below the center image (like a carousel). Or simply reduce the radius and number of satellites (8 -> 6).

---

## Implementation Plan

### Phase 1 — MVP (visual similarity only)

1. **API endpoint** `GET /api/stroll/neighbors`
   - Reuse `find_similar_inner` logic from web routes (embedding lookup, index search)
   - Return center metadata + neighbor list with preview URLs and similarity scores
   - Feature-gated behind `ai`

2. **Askama template** `StrollPage`
   - Full page with nav bar, center container, SVG overlay
   - Embeds center asset data for instant render
   - `<div id="stroll-container">` as the main interactive area

3. **CSS** additions to `style.css`
   - `.stroll-container` — relative positioned, fixed aspect ratio
   - `.stroll-center` — large preview, centered
   - `.stroll-satellite` — thumbnail, absolutely positioned
   - `.stroll-satellite.focused` — mid-size scale-up transition
   - `.stroll-line` — SVG line styling (solid for L1, dashed/thin for L2)
   - `.stroll-overlay` — rating/label/name overlay on images
   - `.stroll-l2` — L2 thumbnail (~60% satellite size), fade-in animation
   - `.stroll-l2-line` — dashed SVG line from L2 thumbnail to parent satellite
   - `.stroll-fanout-slider` — fan-out control in bottom-left panel
   - Dark mode overrides

4. **JavaScript** IIFE in the template
   - `fetch('/api/stroll/neighbors?id=...')` on load and navigation
   - Radial positioning calculation
   - Arrow key navigation (Left/Right cycle, Enter navigates)
   - Mouse hover for focus, click to navigate
   - Rating/label keyboard shortcuts (reuse existing key handlers)
   - `pushState` for history
   - Lightbox integration (`window.damLightbox.openWithData()`)
   - Fan-out slider change handler: updates `fanOut`, re-renders L2 for focused satellite
   - L2 lazy-load on satellite focus: fetch, deduplicate (exclude center + L1 IDs), cache, render
   - L2 repositioning on window resize
   - L2 click handler: navigate (same as satellite click)

5. **Router + nav** updates
   - Add `/stroll` route in `build_router`
   - Add "Stroll" link to nav bar template
   - Add "Stroll from here" link on detail page (when ai feature enabled)

### Phase 2 — Filter Integration

6. Add `q` parameter to the neighbors endpoint
7. Add collapsible filter bar to the stroll template
8. Filter candidates against query before returning

### Phase 3 — Multi-Connection Types

9. Add tag-based neighbor function (Jaccard on tag sets, pure SQL)
10. Add date-based neighbor function (nearest by `created_at`)
11. Add geo-based neighbor function (nearest by GPS coordinates)
12. Connection type selector UI (toggle buttons or dropdown)
13. Color-coded connection lines by type
14. Mixed-mode: combine multiple connection types with weighted scoring

---

## Strolling Without Going in Circles

### The Problem

With large collections of visually similar images — hundreds of concert shots, event photos, or landscape series — pure similarity search creates a local trap. The top-N neighbors of any image in such a cluster are almost always other images from the same cluster. Strolling devolves into walking in tight circles through near-duplicates, never escaping to discover unexpected connections.

Even filtering (e.g., `rating:4+`) only shrinks the cluster from 300 to 100 — still dense enough to dominate all neighbor slots. The stroll page needs mechanisms to inject diversity, break out of local clusters, and make exploration genuinely serendipitous.

### Approach 1: Stroll Modes

A mode selector in the control panel (alongside the neighbors/fan-out sliders) that changes *how* neighbors are selected from the similarity index:

| Mode | Behavior | Use case |
|------|----------|----------|
| **Nearest** (default) | Top N by similarity score | Focused exploration of very similar assets |
| **Discover** | Random N drawn from top M (e.g., 12 from top 80) | Same general neighborhood, but different faces each time |
| **Explore** | Skip the first X most similar, then take N | Jump over the near-duplicate shell to more distant connections |
| **Diverse** | MMR-based selection (see Approach 2) | Maximize variety while staying relevant |

**Discover mode** is the simplest to implement: over-fetch (e.g., 80 candidates), shuffle, take N. Each navigation or reload produces a different set. The randomness breaks the repetitive loops while keeping results in the same broad region of embedding space.

**Explore mode** adds an "offset" or "skip" parameter: ignore the K nearest neighbors and show the next N. This directly addresses the near-duplicate problem by jumping past the tight cluster. A slider could control the skip distance (0 = nearest, 50 = skip 50 nearest, etc.), or it could be a fixed mode that auto-calculates a reasonable skip based on the similarity score distribution (e.g., skip all assets above 0.9 similarity).

Both modes could apply independently to L1 and L2 — e.g., L1 in Explore mode to escape the cluster, L2 in Nearest mode to peek at what's close to each satellite.

### Approach 2: Diversity-Aware Selection (MMR)

Maximal Marginal Relevance (MMR) is a well-known technique from information retrieval. Instead of picking the N most similar assets independently, it picks them *iteratively*: each next pick maximizes similarity to the center while minimizing similarity to assets already picked. This naturally spreads the selection across different sub-clusters.

**Algorithm** (greedy, per-query):
```
candidates = top_100_similar(center)
selected = []
for i in 0..N:
    best = argmax over candidates of:
        λ * sim(candidate, center) - (1-λ) * max(sim(candidate, s) for s in selected)
    selected.append(best)
    candidates.remove(best)
```

The parameter λ (0.0–1.0) controls the trade-off: λ=1.0 is pure similarity (same as today), λ=0.0 is pure diversity (maximally spread out), λ=0.5 is balanced. This could be exposed as a "Diversity" slider in the UI, or simply fixed at a reasonable default (e.g., 0.6).

**Performance**: For N=12 picks from 100 candidates, this requires ~1200 dot products — trivial given that embeddings are 256-dim floats. The entire selection completes in <1ms.

**Implementation**: The MMR logic would live in `EmbeddingIndex` (or a new `DiverseSearch` utility), computing pairwise similarities from the already-loaded embedding buffer. The API endpoint gains a `diversity` parameter (0.0–1.0, default 0.0 for backward compatibility).

This is the most principled approach and subsumes the simpler "Discover" mode (random selection achieves diversity by accident; MMR achieves it optimally).

### Approach 3: Location-Aware Filtering

Ignore assets from the same shooting session when computing neighbors. Two variants:

**a) Path-based session detection**: Assets imported from the same directory (or same parent directory) are assumed to be from the same session. When selecting neighbors for an asset at `Capture/2026-02-22/concert/DSC_4521.NEF`, exclude all assets whose path shares the prefix `Capture/2026-02-22/concert/`. The exclusion depth could be configurable (same directory, same parent, same grandparent).

**b) Time-based session detection**: Assets created within a configurable time window (e.g., ±4 hours, ±1 day) of the center asset are excluded. This handles cases where the same session spans multiple directories (card changes, multiple cameras) and doesn't rely on path structure.

**c) Combined**: Exclude assets that share *both* a similar path prefix *and* a close creation date. This is more conservative — it won't accidentally exclude unrelated assets that happen to be in nearby directories.

This could be a toggle: "Cross-session only" or "Different shoots only". When active, the neighbor query first computes the center asset's session (path prefix or date range), then excludes those asset IDs before similarity ranking. The exclusion set can be precomputed with a single SQL query.

This approach is particularly powerful for the concert/event use case: you'd see visually similar images from *other* concerts, other events, other locations — exactly the kind of unexpected connection that makes strolling interesting.

Could apply to L1 only (L2 still shows local neighbors of the satellite, which may be from the same session) or to both levels. Applying to L1 only is probably more useful: "show me similar things from elsewhere" at the top level, with L2 providing local context around each discovery.

### Approach 4: Visited-Asset Down-Ranking

Track which assets the user has visited (navigated to as center) during the current stroll session, and down-rank them in neighbor selection. This doesn't prevent circles entirely (the same cluster still dominates), but it gradually pushes the user outward by removing already-visited nodes from consideration.

**Simple version**: Exclude visited asset IDs from the candidate set entirely.

**Soft version**: Multiply the similarity score of visited assets by a decay factor (e.g., 0.5), so they can still appear but are less likely to. This avoids dead-ends where all close neighbors have been visited.

The visited set is already partially tracked (browser history / `pushState`), so the client could pass visited IDs to the API. Alternatively, the server could maintain a session-scoped visited set (but this adds statefulness).

### Approach 5: Cluster-Aware Sampling

A more sophisticated variant: detect dense clusters in the candidate set and show one representative per cluster rather than multiple from the same cluster.

**Algorithm**: Take top 100 similar assets. Run a simple clustering (e.g., greedy: first asset starts cluster 1; each subsequent asset joins the nearest cluster if similarity > threshold, otherwise starts a new cluster). Pick the top representative from each cluster (highest similarity to center). This naturally deduplicates near-identical images.

This is similar to MMR but more explicit about cluster boundaries. It could be combined with any of the above approaches.

### Approach 6: "Jump" Button

A simple escape hatch: a button (or keyboard shortcut, e.g., `j`) that teleports to a random asset with *low* similarity to the current center. This is the nuclear option for breaking out of a rut — instead of gradually pushing the boundary, it jumps to a completely different part of the catalog.

The random asset could be: (a) truly random (any embedded asset), (b) anti-similar (lowest similarity to current center — the most *different* image in the catalog), or (c) random from a different cluster/session.

### Recommendation

These approaches are complementary, not mutually exclusive. A phased implementation:

1. **Quick win — Discover mode + Explore mode**: Random-from-top-M and skip-first-K are trivial to implement (just parameters on the existing query). Add as mode buttons or a single dropdown. Immediately breaks the repetitive loop for most users. **Implemented in v2.3.2.**

2. **Principled solution — MMR diversity slider**: Adds a `diversity` parameter to the neighbor API. More elegant than random sampling and gives the user fine-grained control. Subsumes Discover mode.

3. **Domain-specific — Cross-session toggle**: Adds an "Other shoots only" toggle that excludes same-path/same-date assets. Extremely effective for the photographer workflow. Requires a bit of SQL work but no changes to the embedding logic. **Implemented in v2.3.2.**

4. **Polish — Visited tracking + Jump**: Down-rank visited assets and add a "Jump" shortcut. Small quality-of-life improvements that compound over time.

### UX Considerations

- **Discoverability**: New users should get useful strolling out of the box. The default mode (Nearest + some diversity) should work well without tweaking. Power users can adjust sliders and modes.
- **Predictability vs. serendipity**: Random/diverse modes sacrifice reproducibility (same center → different neighbors each time). This is a feature for exploration but could confuse users who expect consistency. Consider showing the active mode prominently.
- **Composability with filters**: All modes should compose with the existing filter bar. "Cross-session Explore mode within my 4-star landscapes" should just work.
- **L1 vs. L2 modes**: Modes could apply differently to L1 and L2. A reasonable default: L1 uses the selected mode (diverse/explore/cross-session), L2 always uses Nearest (showing the local neighborhood around each satellite). This gives breadth at L1 and depth at L2.
- **Control panel layout**: The mode selector, diversity slider, and cross-session toggle add controls to an already-busy panel. Group them under a collapsible "Stroll mode" section, or use a dropdown that exposes relevant sub-controls for the selected mode.

---

## Open Questions

1. **Number of satellites**: 8? 10? 12? More feels richer but clutters. Start with 10, make it configurable or adaptive to viewport size.

2. **Animation style**: Should the transition between centers be a smooth morph (satellite grows to center size while moving to center position) or a quick fade? Morph looks better but is more complex. Start with a fade/crossfade, upgrade later.

3. **Embedding coverage**: Not all assets have embeddings. When strolling lands on an asset without an embedding, the page could: (a) show a message "No visual connections — generate embeddings with `dam embed`", (b) fall back to tag/date-based connections, or (c) offer to generate the embedding on-demand (like the detail page's "smart preview" button). Option (c) is nicest but requires the AI model.

4. **Performance**: Loading neighbors is fast (<50ms with the in-memory index). The bottleneck is loading 10-12 preview images. Use lazy loading (`loading="lazy"`) and/or progressive reveal (show placeholders, swap in images as they load).

5. **Breadcrumb trail**: Show a small breadcrumb strip of recently visited centers? This helps the user retrace their path without relying on browser back. Could be a horizontal strip of tiny thumbnails at the bottom.

6. **Mobile**: The radial layout doesn't work well on small screens. A vertical layout (center image on top, neighbors as a horizontal scroll strip below) might work better. Or just hide the stroll page on mobile and show a message.
