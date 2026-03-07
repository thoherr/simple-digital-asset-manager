# Proposal: Stroll Page — Graph-Based Visual Exploration

> **Status**: Proposal

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
- **Navigation**: Click or Enter on a focused satellite makes it the new center. The view animates: old center shrinks away, new center grows, new satellites load.

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

**CSS Layout:** CSS `position: absolute` within a relative container. Satellites positioned using `transform: translate()` computed from angular positions. Center image at 50%/50%.

```
angle_i = (2 * PI * i) / num_neighbors
x_i = center_x + radius * cos(angle_i)
y_i = center_y + radius * sin(angle_i)
```

**Connection lines:** SVG overlay layer (absolutely positioned, pointer-events: none) with `<line>` elements from center to each satellite.

**Transitions:** CSS transitions on `transform`, `width`, `height`, `opacity` for smooth focus/navigation animations. On navigation, the old satellites fade out, new ones fade in. The center image cross-fades.

**State management:** A single JS IIFE (`window.damStroll`) managing:
- `centerId` — current center asset ID
- `neighbors[]` — current neighbor data
- `focusIndex` — which satellite has keyboard focus (-1 = center)
- `history[]` — breadcrumb trail for back navigation

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
   - `.stroll-line` — SVG line styling
   - `.stroll-overlay` — rating/label/name overlay on images
   - Dark mode overrides

4. **JavaScript** IIFE in the template
   - `fetch('/api/stroll/neighbors?id=...')` on load and navigation
   - Radial positioning calculation
   - Arrow key navigation (Left/Right cycle, Enter navigates)
   - Mouse hover for focus, click to navigate
   - Rating/label keyboard shortcuts (reuse existing key handlers)
   - `pushState` for history
   - Lightbox integration (`window.damLightbox.openWithData()`)

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

## Open Questions

1. **Number of satellites**: 8? 10? 12? More feels richer but clutters. Start with 10, make it configurable or adaptive to viewport size.

2. **Animation style**: Should the transition between centers be a smooth morph (satellite grows to center size while moving to center position) or a quick fade? Morph looks better but is more complex. Start with a fade/crossfade, upgrade later.

3. **Embedding coverage**: Not all assets have embeddings. When strolling lands on an asset without an embedding, the page could: (a) show a message "No visual connections — generate embeddings with `dam embed`", (b) fall back to tag/date-based connections, or (c) offer to generate the embedding on-demand (like the detail page's "smart preview" button). Option (c) is nicest but requires the AI model.

4. **Performance**: Loading neighbors is fast (<50ms with the in-memory index). The bottleneck is loading 10-12 preview images. Use lazy loading (`loading="lazy"`) and/or progressive reveal (show placeholders, swap in images as they load).

5. **Breadcrumb trail**: Show a small breadcrumb strip of recently visited centers? This helps the user retrace their path without relying on browser back. Could be a horizontal strip of tiny thumbnails at the bottom.

6. **Mobile**: The radial layout doesn't work well on small screens. A vertical layout (center image on top, neighbors as a horizontal scroll strip below) might work better. Or just hide the stroll page on mobile and show a message.
