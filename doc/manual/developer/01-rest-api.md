# REST API Reference

The `dam serve` command starts an Axum-based web server (default `127.0.0.1:8080`) that exposes both HTML page routes and JSON/HTML-partial API endpoints. All endpoints use `spawn_blocking` for SQLite access and open a fresh catalog connection per request via `Catalog::open_fast()`.

Base URL: `http://127.0.0.1:8080` (configurable via `--bind` and `--port`)

---

## Page Routes

These routes return full HTML pages (for direct browser requests) or HTML partials (for htmx requests).

### `GET /` -- Browse Page

Returns the asset browse grid with search, filtering, sorting, and pagination.

| Parameter    | Type   | Default     | Description                                      |
|--------------|--------|-------------|--------------------------------------------------|
| `q`          | string | `""`        | Free-text search query (supports filter syntax)  |
| `type`       | string | `""`        | Asset type filter (e.g. `image`, `video`)        |
| `tag`        | string | `""`        | Tag filter                                       |
| `format`     | string | `""`        | File format filter, comma-separated for OR (e.g. `nef,cr3`) |
| `volume`     | string | `""`        | Volume label filter                              |
| `rating`     | string | `""`        | Rating filter (`5` exact, `3+` minimum)          |
| `label`      | string | `""`        | Color label filter (e.g. `Red`, `Green`)         |
| `collection` | string | `""`        | Collection name filter                           |
| `path`       | string | `""`        | Path prefix filter                               |
| `sort`       | string | `date_desc` | Sort order (see below)                           |
| `page`       | u32    | `1`         | Page number (configurable results per page, default 60) |

**Sort values**: `date_desc`, `date_asc`, `name_desc`, `name_asc`, `size_desc`, `size_asc`

**Behavior**: Detects `HX-Request` header. htmx requests receive a `ResultsPartial` (just the results grid). Direct browser requests receive the full `BrowsePage` with layout, CSS, filter controls, saved search chips, and all dropdown options.

```bash
# Full page (browser)
curl http://localhost:8080/

# With filters
curl "http://localhost:8080/?q=sunset&rating=4%2B&sort=date_desc&page=1"

# htmx partial
curl -H "HX-Request: true" "http://localhost:8080/?tag=landscape&page=2"
```

### `GET /asset/{id}` -- Asset Detail Page

Returns the detail page for a single asset, including preview image, metadata, editable tags, rating, label, name, description, variants, recipes, and collection memberships.

| Parameter | Type   | Description             |
|-----------|--------|-------------------------|
| `id`      | string | Asset UUID (path param) |

```bash
curl http://localhost:8080/asset/550e8400-e29b-41d4-a716-446655440000
```

**Errors**: Returns 404 with `<h1>Not Found</h1>` if the asset does not exist.

### `GET /tags` -- Tags Page

Returns an HTML page listing all tags with counts. Supports sortable columns and live text filtering.

```bash
curl http://localhost:8080/tags
```

### `GET /stats` -- Stats Page

Returns an HTML page showing catalog statistics: overview, type/format breakdown, per-volume details, tag frequencies, and verification health.

```bash
curl http://localhost:8080/stats
```

### `GET /collections` -- Collections Page

Returns an HTML page listing all collections with a "New Collection" button.

```bash
curl http://localhost:8080/collections
```

### `GET /people` -- People Page

*Requires `--features ai` compilation.*

Returns an HTML page showing all detected people with face crop thumbnails, names, face counts, and management controls (rename, merge, delete). Includes a "Cluster" button to run auto-clustering from the UI.

```bash
curl http://localhost:8080/people
```

### `GET /backup` -- Backup Status Page

Returns an HTML page showing backup health: summary cards (total assets, at-risk count, min copies), volume distribution bar chart, coverage by purpose table, and volume gaps table.

```bash
curl http://localhost:8080/backup
```

### `GET /stroll` -- Stroll Page

*Requires `--features ai` compilation.*

Returns an interactive visual similarity exploration page. Centers on a randomly selected asset (or a specific asset via query parameter) and displays its nearest neighbors by SigLIP embedding distance. Clicking a neighbor re-centers the view on that asset.

| Parameter | Type   | Default  | Description                              |
|-----------|--------|----------|------------------------------------------|
| `id`      | string | (random) | Asset UUID to center on (optional)       |

```bash
# Random starting point
curl http://localhost:8080/stroll

# Start from a specific asset
curl "http://localhost:8080/stroll?id=550e8400-e29b-41d4-a716-446655440000"
```

---

## Search API

### `GET /api/search`

Same query parameters as `GET /`. Intended for htmx consumption.

**Behavior**: Non-htmx requests (missing `HX-Request` header) are redirected to `/?{params}` so that direct browser loads, back-button navigations, and bookmarks render the full page. htmx requests receive a `ResultsPartial`.

```bash
# htmx request (returns HTML partial)
curl -H "HX-Request: true" "http://localhost:8080/api/search?q=sunset&page=1"

# Browser request (redirects to /?q=sunset&page=1)
curl -v "http://localhost:8080/api/search?q=sunset&page=1"
# < HTTP/1.1 303 See Other
# < location: /?q=sunset&page=1
```

---

## Asset Editing

### `POST /api/asset/{id}/tags` -- Add Tags

Adds one or more tags to an asset. Triggers XMP write-back.

| Field | Type   | Description                          |
|-------|--------|--------------------------------------|
| `tags` | string | Comma-separated tag names (form-encoded) |

**Content-Type**: `application/x-www-form-urlencoded`

**Response**: HTML partial -- updated tag list fragment.

```bash
curl -X POST http://localhost:8080/api/asset/{id}/tags \
  -d "tags=landscape,nature,sunset"
```

### `DELETE /api/asset/{id}/tags/{tag}` -- Remove Tag

Removes a single tag from an asset. Triggers XMP write-back.

| Parameter | Type   | Description              |
|-----------|--------|--------------------------|
| `id`      | string | Asset UUID (path param)  |
| `tag`     | string | Tag to remove (path param, URL-encoded) |

**Response**: HTML partial -- updated tag list fragment.

```bash
curl -X DELETE http://localhost:8080/api/asset/{id}/tags/landscape
```

### `PUT /api/asset/{id}/rating` -- Set Rating

Sets or clears the asset's star rating. A value of `0` or `null` clears the rating. Triggers XMP write-back.

| Field    | Type       | Description                   |
|----------|------------|-------------------------------|
| `rating` | u8 or null | Rating value 1-5, or 0/null to clear |

**Content-Type**: `application/x-www-form-urlencoded`

**Response**: HTML partial -- updated rating fragment.

```bash
# Set rating to 5 stars
curl -X PUT http://localhost:8080/api/asset/{id}/rating \
  -d "rating=5"

# Clear rating
curl -X PUT http://localhost:8080/api/asset/{id}/rating \
  -d "rating=0"
```

### `PUT /api/asset/{id}/description` -- Set Description

Sets or clears the asset's description. An empty string clears the description. Triggers XMP write-back.

| Field         | Type          | Description                    |
|---------------|---------------|--------------------------------|
| `description` | string or null | Description text, or empty to clear |

**Content-Type**: `application/x-www-form-urlencoded`

**Response**: HTML partial -- updated description fragment.

```bash
# Set description
curl -X PUT http://localhost:8080/api/asset/{id}/description \
  -d "description=A%20beautiful%20sunset%20over%20the%20mountains"

# Clear description
curl -X PUT http://localhost:8080/api/asset/{id}/description \
  -d "description="
```

### `PUT /api/asset/{id}/name` -- Set Name

Sets or clears the asset's display name. An empty string clears the name (reverts to filename fallback).

| Field  | Type          | Description                  |
|--------|---------------|------------------------------|
| `name` | string or null | Display name, or empty to clear |

**Content-Type**: `application/x-www-form-urlencoded`

**Response**: HTML partial -- updated name fragment (includes fallback filename display when name is cleared).

```bash
# Set name
curl -X PUT http://localhost:8080/api/asset/{id}/name \
  -d "name=Mountain%20Sunset"

# Clear name
curl -X PUT http://localhost:8080/api/asset/{id}/name \
  -d "name="
```

### `PUT /api/asset/{id}/label` -- Set Color Label

Sets or clears the asset's color label. Accepts case-insensitive color names from the 7-color set: Red, Orange, Yellow, Green, Blue, Pink, Purple. An empty string clears the label. Triggers XMP write-back.

| Field   | Type   | Description                          |
|---------|--------|--------------------------------------|
| `label` | string | Color name (case-insensitive), or empty to clear |

**Content-Type**: `application/x-www-form-urlencoded`

**Response**: HTML partial -- updated label fragment.

```bash
# Set label
curl -X PUT http://localhost:8080/api/asset/{id}/label \
  -d "label=Red"

# Clear label
curl -X PUT http://localhost:8080/api/asset/{id}/label \
  -d "label="
```

### `PUT /api/asset/{id}/date` -- Set Date

Sets or clears the asset's creation date. Accepts an ISO date or datetime string. An empty string clears the date.

| Field  | Type          | Description                  |
|--------|---------------|------------------------------|
| `date` | string or null | ISO date string (e.g. `2024-12-25`), or empty to clear |

**Content-Type**: `application/x-www-form-urlencoded`

**Response**: HTML partial -- updated date fragment.

```bash
# Set date
curl -X PUT http://localhost:8080/api/asset/{id}/date \
  -d "date=2024-12-25"

# Clear date
curl -X PUT http://localhost:8080/api/asset/{id}/date \
  -d "date="
```

### `POST /api/open-location` -- Reveal in File Manager

Opens the system file manager with the specified file selected. macOS uses `open -R` (Finder); Windows uses `explorer /select,` (Explorer); Linux uses `xdg-open` on the parent directory.

| Field           | Type   | Description                     |
|-----------------|--------|---------------------------------|
| `volume_id`     | string | Volume UUID                     |
| `relative_path` | string | File path relative to the volume mount point |

**Content-Type**: `application/json`

**Response**: `{"ok": true}` on success, or error message.

```bash
curl -X POST http://localhost:8080/api/open-location \
  -H "Content-Type: application/json" \
  -d '{"volume_id":"abc-123","relative_path":"Photos/DSC_001.nef"}'
```

### `POST /api/open-terminal` -- Open Terminal

Opens a terminal window in the file's parent directory. macOS uses Terminal.app; Windows uses cmd; Linux tries common terminal emulators.

| Field           | Type   | Description                     |
|-----------------|--------|---------------------------------|
| `volume_id`     | string | Volume UUID                     |
| `relative_path` | string | File path relative to the volume mount point |

**Content-Type**: `application/json`

**Response**: `{"ok": true}` on success, or error message.

```bash
curl -X POST http://localhost:8080/api/open-terminal \
  -H "Content-Type: application/json" \
  -d '{"volume_id":"abc-123","relative_path":"Photos/DSC_001.nef"}'
```

### `POST /api/asset/{id}/preview` -- Regenerate Previews

Regenerates both the regular preview thumbnail and the smart preview for the asset's best variant. Requires the source file to be on an online volume. Returns cache-busted URLs so the browser displays the newly generated images without requiring a page reload.

**Content-Type**: None (no body required)

**Response**: HTML partial -- updated preview fragment with cache-busted URLs.

```bash
curl -X POST http://localhost:8080/api/asset/{id}/preview
```

### `POST /api/asset/{id}/rotate` -- Rotate Preview

Cycles the preview rotation 90° clockwise (0° → 90° → 180° → 270° → 0°). Regenerates both regular and smart previews with EXIF auto-orientation applied. The rotation is persisted per asset.

**Content-Type**: None (no body required)

**Response**: HTML partial -- updated preview fragment with cache-busted URLs.

```bash
curl -X POST http://localhost:8080/api/asset/{id}/rotate
```

### `PUT /api/asset/{id}/stack-pick` -- Set Stack Pick

Sets this asset as the pick (position 0) of its stack. The previous pick swaps to this asset's former position. Persists to both SQLite and `stacks.yaml`.

| Parameter | Type   | Description             |
|-----------|--------|-------------------------|
| `id`      | string | Asset UUID (path param) |

**Response**:
```json
{
  "status": "ok"
}
```

**Errors**: Returns 500 if the asset is not in a stack.

```bash
curl -X PUT http://localhost:8080/api/asset/{id}/stack-pick
```

### `DELETE /api/asset/{id}/stack` -- Dissolve Stack

Dissolves the entire stack that this asset belongs to. All members are unstacked. Persists to both SQLite and `stacks.yaml`.

| Parameter | Type   | Description             |
|-----------|--------|-------------------------|
| `id`      | string | Asset UUID (path param) |

**Response**:
```json
{
  "status": "ok"
}
```

**Errors**: Returns 500 if the asset is not in a stack.

```bash
curl -X DELETE http://localhost:8080/api/asset/{id}/stack
```

---

## Batch Operations

All batch endpoints accept JSON request bodies and return JSON responses. They loop over individual assets, collecting successes and failures independently.

### `POST /api/batch/tags` -- Batch Add/Remove Tags

Adds or removes tags on multiple assets. Each individual operation triggers XMP write-back.

**Content-Type**: `application/json`

**Request body**:
```json
{
  "asset_ids": ["uuid-1", "uuid-2", "uuid-3"],
  "tags": ["landscape", "nature"],
  "remove": false
}
```

| Field      | Type     | Description                              |
|------------|----------|------------------------------------------|
| `asset_ids` | string[] | Array of asset UUIDs                    |
| `tags`     | string[] | Tag names to add or remove               |
| `remove`   | bool     | `false` to add tags, `true` to remove    |

**Response**:
```json
{
  "succeeded": 3,
  "failed": 0,
  "errors": []
}
```

```bash
curl -X POST http://localhost:8080/api/batch/tags \
  -H "Content-Type: application/json" \
  -d '{"asset_ids": ["uuid-1", "uuid-2"], "tags": ["landscape"], "remove": false}'
```

### `PUT /api/batch/rating` -- Batch Set Rating

Sets or clears the rating on multiple assets. Each individual operation triggers XMP write-back.

**Content-Type**: `application/json`

**Request body**:
```json
{
  "asset_ids": ["uuid-1", "uuid-2"],
  "rating": 5
}
```

| Field      | Type          | Description                          |
|------------|---------------|--------------------------------------|
| `asset_ids` | string[]     | Array of asset UUIDs                 |
| `rating`   | u8 or null    | Rating 1-5, or null/0 to clear      |

**Response**:
```json
{
  "succeeded": 2,
  "failed": 0,
  "errors": []
}
```

```bash
# Set rating
curl -X PUT http://localhost:8080/api/batch/rating \
  -H "Content-Type: application/json" \
  -d '{"asset_ids": ["uuid-1", "uuid-2"], "rating": 4}'

# Clear rating
curl -X PUT http://localhost:8080/api/batch/rating \
  -H "Content-Type: application/json" \
  -d '{"asset_ids": ["uuid-1", "uuid-2"], "rating": null}'
```

### `PUT /api/batch/label` -- Batch Set Label

Sets or clears the color label on multiple assets. The label is validated against the 7-color set before processing any assets. Each individual operation triggers XMP write-back.

**Content-Type**: `application/json`

**Request body**:
```json
{
  "asset_ids": ["uuid-1", "uuid-2"],
  "label": "Red"
}
```

| Field      | Type          | Description                                  |
|------------|---------------|----------------------------------------------|
| `asset_ids` | string[]     | Array of asset UUIDs                         |
| `label`    | string or null | Color name (case-insensitive), or empty/null to clear |

**Response**:
```json
{
  "succeeded": 2,
  "failed": 0,
  "errors": []
}
```

```bash
curl -X PUT http://localhost:8080/api/batch/label \
  -H "Content-Type: application/json" \
  -d '{"asset_ids": ["uuid-1", "uuid-2"], "label": "Green"}'
```

### `POST /api/batch/collection` -- Add to Collection

Adds assets to a named collection. Persists to both SQLite and YAML.

**Content-Type**: `application/json`

**Request body**:
```json
{
  "asset_ids": ["uuid-1", "uuid-2"],
  "collection": "Best of 2025"
}
```

| Field        | Type     | Description            |
|--------------|----------|------------------------|
| `asset_ids`  | string[] | Array of asset UUIDs   |
| `collection` | string   | Collection name        |

**Response**:
```json
{
  "added": 2,
  "collection": "Best of 2025"
}
```

```bash
curl -X POST http://localhost:8080/api/batch/collection \
  -H "Content-Type: application/json" \
  -d '{"asset_ids": ["uuid-1", "uuid-2"], "collection": "Best of 2025"}'
```

### `DELETE /api/batch/collection` -- Remove from Collection

Removes assets from a named collection. Persists to both SQLite and YAML.

**Content-Type**: `application/json`

**Request body**:
```json
{
  "asset_ids": ["uuid-1", "uuid-2"],
  "collection": "Best of 2025"
}
```

| Field        | Type     | Description            |
|--------------|----------|------------------------|
| `asset_ids`  | string[] | Array of asset UUIDs   |
| `collection` | string   | Collection name        |

**Response**:
```json
{
  "removed": 2,
  "collection": "Best of 2025"
}
```

```bash
curl -X DELETE http://localhost:8080/api/batch/collection \
  -H "Content-Type: application/json" \
  -d '{"asset_ids": ["uuid-1", "uuid-2"], "collection": "Best of 2025"}'
```

### `POST /api/batch/stack` -- Create Stack

Creates a stack from the selected assets. The first asset becomes the pick (position 0). Assets already in a stack are rejected. Persists to both SQLite and `stacks.yaml`.

**Content-Type**: `application/json`

**Request body**:
```json
{
  "asset_ids": ["uuid-1", "uuid-2", "uuid-3"]
}
```

| Field      | Type     | Description                              |
|------------|----------|------------------------------------------|
| `asset_ids` | string[] | Array of asset UUIDs (minimum 2)        |

**Response**:
```json
{
  "stack_id": "550e8400-e29b-41d4-a716-446655440000",
  "member_count": 3
}
```

**Errors**: Returns 500 if any asset is already in a stack, or fewer than 2 assets are provided.

```bash
curl -X POST http://localhost:8080/api/batch/stack \
  -H "Content-Type: application/json" \
  -d '{"asset_ids": ["uuid-1", "uuid-2", "uuid-3"]}'
```

### `DELETE /api/batch/stack` -- Unstack Assets

Removes assets from their stacks. If a stack is left with 1 or fewer members, it is automatically dissolved. Persists to both SQLite and `stacks.yaml`.

**Content-Type**: `application/json`

**Request body**:
```json
{
  "asset_ids": ["uuid-1", "uuid-2"]
}
```

| Field      | Type     | Description            |
|------------|----------|------------------------|
| `asset_ids` | string[] | Array of asset UUIDs  |

**Response**:
```json
{
  "removed": 2
}
```

```bash
curl -X DELETE http://localhost:8080/api/batch/stack \
  -H "Content-Type: application/json" \
  -d '{"asset_ids": ["uuid-1", "uuid-2"]}'
```

### `POST /api/asset/{id}/suggest-tags` -- AI Tag Suggestions

*Requires `--features ai` compilation.*

Analyzes the asset's preview image with SigLIP and returns suggested tags with confidence scores. Tags already on the asset are included but marked with `existing: true`. The model is lazy-loaded on first request and cached in server memory.

| Parameter | Type   | Description             |
|-----------|--------|-------------------------|
| `id`      | string | Asset UUID (path param) |

**Response**: `application/json`

```json
[
  {"tag": "landscape", "confidence": 0.85, "existing": false},
  {"tag": "mountain", "confidence": 0.42, "existing": false},
  {"tag": "nature", "confidence": 0.31, "existing": true}
]
```

| Field       | Type   | Description                         |
|-------------|--------|-------------------------------------|
| `tag`       | string | Suggested tag name                  |
| `confidence`| float  | Confidence score (0.0–1.0)          |
| `existing`  | bool   | Whether the tag is already on the asset |

```bash
curl -X POST http://localhost:8080/api/asset/550e8400-e29b-41d4-a716-446655440000/suggest-tags
```

### `POST /api/batch/auto-tag` -- Batch AI Auto-Tag

*Requires `--features ai` compilation.*

Auto-tags selected assets using SigLIP. For each asset, encodes the preview image, classifies against the configured label vocabulary, and applies tags above the threshold. Existing tags are not duplicated.

**Content-Type**: `application/json`

**Request body**:
```json
{
  "asset_ids": ["uuid-1", "uuid-2", "uuid-3"]
}
```

| Field      | Type     | Description            |
|------------|----------|------------------------|
| `asset_ids` | string[] | Array of asset UUIDs  |

**Response**:
```json
{
  "succeeded": 3,
  "failed": 0,
  "tags_applied": 12,
  "errors": []
}
```

| Field          | Type     | Description                                  |
|----------------|----------|----------------------------------------------|
| `succeeded`    | u32      | Assets successfully processed                |
| `failed`       | u32      | Assets that failed                           |
| `tags_applied` | u32      | Total new tags applied across all assets     |
| `errors`       | string[] | Error messages for failed assets             |

```bash
curl -X POST http://localhost:8080/api/batch/auto-tag \
  -H "Content-Type: application/json" \
  -d '{"asset_ids": ["uuid-1", "uuid-2", "uuid-3"]}'
```

### `POST /api/batch/detect-faces` -- Batch Face Detection

*Requires `--features ai` compilation.*

Detects faces in the preview images of the selected assets using YuNet + ArcFace. Stores face records, embeddings, and crop thumbnails.

**Content-Type**: `application/json`

**Request body**:
```json
{
  "asset_ids": ["uuid-1", "uuid-2", "uuid-3"]
}
```

| Field      | Type     | Description            |
|------------|----------|------------------------|
| `asset_ids` | string[] | Array of asset UUIDs  |

**Response**:
```json
{
  "succeeded": 3,
  "failed": 0,
  "faces_detected": 7,
  "errors": []
}
```

```bash
curl -X POST http://localhost:8080/api/batch/detect-faces \
  -H "Content-Type: application/json" \
  -d '{"asset_ids": ["uuid-1", "uuid-2", "uuid-3"]}'
```

### `POST /api/asset/{id}/split` -- Split Variants Out of Asset

Splits one or more variants out of a multi-variant asset into new standalone assets. Each specified variant becomes a new asset with its own sidecar YAML. Recipes attached to split variants move with them.

**Content-Type**: `application/json`

**Request body**:
```json
{
  "variant_hashes": ["sha256:abcdef1234...", "sha256:567890abcd..."]
}
```

| Field            | Type     | Description                              |
|------------------|----------|------------------------------------------|
| `variant_hashes` | string[] | Content hashes of variants to split out  |

**Response**:
```json
{
  "source_id": "550e8400-e29b-41d4-a716-446655440000",
  "new_assets": [
    {
      "asset_id": "660f9511-f30c-52e5-b827-557766551111",
      "variant_hash": "sha256:abcdef1234...",
      "original_filename": "DSC_001_edit.tif"
    }
  ]
}
```

| Field                          | Type     | Description                          |
|--------------------------------|----------|--------------------------------------|
| `source_id`                    | string   | UUID of the original (source) asset  |
| `new_assets[].asset_id`       | string   | UUID of the newly created asset      |
| `new_assets[].variant_hash`   | string   | Content hash of the split variant    |
| `new_assets[].original_filename` | string | Original filename of the variant   |

**Errors**: Returns 404 if the asset does not exist. Returns 400 if a variant hash is not found on the asset, or if splitting would leave the source asset with zero variants.

```bash
curl -X POST http://localhost:8080/api/asset/{id}/split \
  -H "Content-Type: application/json" \
  -d '{"variant_hashes": ["sha256:abcdef1234..."]}'
```

### `POST /api/batch/group` -- Merge Assets

Merges selected assets into a single asset. Donor variants are moved to the target asset, donor assets are removed. Tags and recipes are merged.

**Content-Type**: `application/json`

**Request body**:
```json
{
  "asset_ids": ["uuid-1", "uuid-2", "uuid-3"],
  "target_id": "uuid-1"
}
```

| Field       | Type           | Description                                          |
|-------------|----------------|------------------------------------------------------|
| `asset_ids` | string[]       | Array of asset UUIDs to merge                        |
| `target_id` | string or null | Optional target asset UUID (default: auto-selected)  |

**Response**:
```json
{
  "target_id": "uuid-1",
  "variants_moved": 2,
  "donors_removed": 2
}
```

| Field            | Type   | Description                          |
|------------------|--------|--------------------------------------|
| `target_id`      | string | UUID of the target (surviving) asset |
| `variants_moved` | u32    | Number of variants moved to target   |
| `donors_removed` | u32    | Number of donor assets removed       |

**Errors**: Returns 400 if fewer than 2 asset IDs are provided.

```bash
curl -X POST http://localhost:8080/api/batch/group \
  -H "Content-Type: application/json" \
  -d '{"asset_ids": ["uuid-1", "uuid-2", "uuid-3"], "target_id": "uuid-1"}'
```

### `POST /api/batch/auto-group` -- Auto-Group by Stem

Groups selected assets by filename stem using fuzzy prefix matching. Merges donor variants into target assets (shortest stem, preferring RAW originals).

**Content-Type**: `application/json`

**Request body**:
```json
{
  "asset_ids": ["uuid-1", "uuid-2", "uuid-3"]
}
```

| Field      | Type     | Description            |
|------------|----------|------------------------|
| `asset_ids` | string[] | Array of asset UUIDs  |

**Response**:
```json
{
  "groups_merged": 1,
  "donors_removed": 2,
  "variants_moved": 2
}
```

```bash
curl -X POST http://localhost:8080/api/batch/auto-group \
  -H "Content-Type: application/json" \
  -d '{"asset_ids": ["uuid-1", "uuid-2", "uuid-3"]}'
```

---

## Face & People APIs

*All face and people endpoints require `--features ai` compilation.*

### `GET /api/asset/{id}/faces` -- List Faces for Asset

Returns all detected faces for an asset as a JSON array.

**Response**: `application/json`

```json
[
  {
    "id": "face-uuid-1",
    "asset_id": "asset-uuid",
    "person_id": "person-uuid",
    "bbox_x": 0.25,
    "bbox_y": 0.15,
    "bbox_w": 0.12,
    "bbox_h": 0.18,
    "confidence": 0.95,
    "created_at": "2026-03-05T10:30:00Z"
  }
]
```

```bash
curl http://localhost:8080/api/asset/{id}/faces
```

### `POST /api/asset/{id}/detect-faces` -- Detect Faces in Asset

Runs face detection on a single asset's preview image. Stores detected faces with embeddings and crop thumbnails.

**Response**: `application/json`

```json
{
  "faces_detected": 3
}
```

```bash
curl -X POST http://localhost:8080/api/asset/{id}/detect-faces
```

### `GET /api/people` -- List All People

Returns all people with face counts as a JSON array.

**Response**: `application/json`

```json
[
  {
    "id": "person-uuid",
    "name": "Alice",
    "face_count": 42,
    "representative_face_id": "face-uuid"
  }
]
```

```bash
curl http://localhost:8080/api/people
```

### `PUT /api/people/{id}/name` -- Name a Person

Sets or updates the name of a person.

**Content-Type**: `application/json`

**Request body**:
```json
{
  "name": "Alice"
}
```

**Response**:
```json
{
  "status": "ok"
}
```

```bash
curl -X PUT http://localhost:8080/api/people/{id}/name \
  -H "Content-Type: application/json" \
  -d '{"name": "Alice"}'
```

### `POST /api/people/{id}/merge` -- Merge People

Merges the source person into the target person. All faces from the source are reassigned to the target.

**Content-Type**: `application/json`

**Request body**:
```json
{
  "source_id": "person-uuid-to-merge"
}
```

**Response**:
```json
{
  "status": "ok",
  "faces_moved": 15
}
```

```bash
curl -X POST http://localhost:8080/api/people/{id}/merge \
  -H "Content-Type: application/json" \
  -d '{"source_id": "person-uuid-to-merge"}'
```

### `DELETE /api/people/{id}` -- Delete a Person

Deletes a person. All faces assigned to this person become unassigned.

**Response**:
```json
{
  "status": "ok"
}
```

```bash
curl -X DELETE http://localhost:8080/api/people/{id}
```

### `PUT /api/faces/{face_id}/assign` -- Assign Face to Person

Assigns a face to a person.

**Content-Type**: `application/json`

**Request body**:
```json
{
  "person_id": "person-uuid"
}
```

**Response**:
```json
{
  "status": "ok"
}
```

```bash
curl -X PUT http://localhost:8080/api/faces/{face_id}/assign \
  -H "Content-Type: application/json" \
  -d '{"person_id": "person-uuid"}'
```

### `DELETE /api/faces/{face_id}/unassign` -- Unassign Face from Person

Removes the person assignment from a face.

**Response**:
```json
{
  "status": "ok"
}
```

```bash
curl -X DELETE http://localhost:8080/api/faces/{face_id}/unassign
```

### `POST /api/faces/cluster` -- Auto-Cluster Faces

Runs auto-clustering on unassigned faces, grouping similar faces into new person records.

**Content-Type**: `application/json`

**Request body** (optional):
```json
{
  "threshold": 0.5
}
```

| Field       | Type  | Default | Description                         |
|-------------|-------|---------|-------------------------------------|
| `threshold` | float | 0.5     | Similarity threshold for clustering |

**Response**:
```json
{
  "people_created": 12,
  "faces_assigned": 87,
  "singletons_skipped": 5
}
```

```bash
curl -X POST http://localhost:8080/api/faces/cluster \
  -H "Content-Type: application/json" \
  -d '{"threshold": 0.5}'
```

---

## Data APIs

### `GET /api/calendar` -- Calendar Heatmap Data

Returns per-day asset counts for a given year, respecting all search filters. Used by the browse page calendar heatmap view.

| Parameter    | Type   | Default        | Description                                      |
|--------------|--------|----------------|--------------------------------------------------|
| `year`       | i32    | current year   | Year to aggregate                                |
| `q`          | string | `""`           | Free-text search query (supports filter syntax)  |
| `type`       | string | `""`           | Asset type filter                                |
| `tag`        | string | `""`           | Tag filter                                       |
| `format`     | string | `""`           | File format filter, comma-separated for OR       |
| `volume`     | string | `""`           | Volume ID filter                                 |
| `rating`     | string | `""`           | Rating filter                                    |
| `label`      | string | `""`           | Color label filter                               |
| `collection` | string | `""`           | Collection name filter                           |
| `path`       | string | `""`           | Path prefix filter                               |

**Response**: `application/json`

```json
{
  "year": 2026,
  "counts": {
    "2026-01-15": 23,
    "2026-01-16": 8,
    "2026-02-25": 14
  },
  "years": [2024, 2025, 2026]
}
```

| Field    | Type              | Description                                    |
|----------|-------------------|------------------------------------------------|
| `year`   | i32               | The requested year                             |
| `counts` | object            | Map of `"YYYY-MM-DD"` → asset count (days with 0 assets are omitted) |
| `years`  | i32[]             | All distinct years that have assets in the catalog |

```bash
# Get 2026 calendar data
curl http://localhost:8080/api/calendar?year=2026

# With filters
curl "http://localhost:8080/api/calendar?year=2026&tag=landscape&rating=4%2B"
```

### `GET /api/stroll/neighbors` -- Embedding Neighbors

*Requires `--features ai` compilation.*

Returns the nearest neighbors of an asset by SigLIP embedding similarity. Used by the Stroll page for visual exploration and by the fan-out feature, which fetches transitive neighbors when a satellite is focused (L2 neighbors-of-neighbors).

| Parameter | Type   | Default | Description                              |
|-----------|--------|---------|------------------------------------------|
| `id`      | string | --      | Asset UUID to find neighbors for (required) |
| `n`       | u32    | `12`    | Number of neighbors to return (range: 5–25) |
| `q`       | string | `""`    | Optional search query to scope neighbors |

**Response**: `application/json`

```json
{
  "center": {
    "id": "550e8400-e29b-41d4-a716-446655440000",
    "name": "DSC_001",
    "preview_hash": "a1b2c3d4..."
  },
  "neighbors": [
    {
      "id": "660f9511-f30c-52e5-b827-557766551111",
      "name": "DSC_002",
      "preview_hash": "b2c3d4e5...",
      "similarity": 0.87
    }
  ]
}
```

| Field              | Type   | Description                         |
|--------------------|--------|-------------------------------------|
| `center.id`        | string | Asset UUID of the center asset      |
| `center.name`      | string | Display name or filename            |
| `center.preview_hash` | string | Content hash for preview URL     |
| `neighbors[].id`   | string | Neighbor asset UUID                 |
| `neighbors[].name` | string | Display name or filename            |
| `neighbors[].preview_hash` | string | Content hash for preview URL |
| `neighbors[].similarity` | float | Cosine similarity score (0.0–1.0) |

```bash
# Get 12 nearest neighbors
curl "http://localhost:8080/api/stroll/neighbors?id=550e8400-e29b-41d4-a716-446655440000"

# Get 24 neighbors, scoped to landscapes
curl "http://localhost:8080/api/stroll/neighbors?id=550e8400-e29b-41d4-a716-446655440000&n=24&q=tag:landscape"
```

### `GET /api/tags` -- List All Tags

Returns all tags with their usage counts as a JSON array of `[name, count]` tuples.

**Response**: `application/json`

```json
[
  ["landscape", 42],
  ["nature", 38],
  ["portrait", 15]
]
```

```bash
curl http://localhost:8080/api/tags
```

### `GET /api/stats` -- Catalog Stats

Returns a JSON object with catalog overview statistics.

**Response**: `application/json`

```json
{
  "assets": 1250,
  "variants": 2100,
  "recipes": 800,
  "total_size": 524288000000
}
```

```bash
curl http://localhost:8080/api/stats
```

### `GET /api/collections` -- List Collections

Returns all collections as a JSON array.

**Response**: `application/json`

```bash
curl http://localhost:8080/api/collections
```

### `POST /api/collections` -- Create Collection

Creates a new collection. Persists to both SQLite and YAML.

**Content-Type**: `application/json`

**Request body**:
```json
{
  "name": "Best of 2025",
  "description": "Top picks from this year"
}
```

| Field         | Type          | Description                       |
|---------------|---------------|-----------------------------------|
| `name`        | string        | Collection name (required, unique) |
| `description` | string or null | Optional description              |

**Response** (201 Created):
```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "name": "Best of 2025",
  "description": "Top picks from this year"
}
```

**Errors**: Returns 409 Conflict if a collection with the same name already exists.

```bash
curl -X POST http://localhost:8080/api/collections \
  -H "Content-Type: application/json" \
  -d '{"name": "Best of 2025", "description": "Top picks from this year"}'
```

### `GET /api/saved-searches` -- List Saved Searches

Returns all saved searches as a JSON array.

**Response**: `application/json`

```json
[
  {
    "name": "5-star landscapes",
    "query": "tag:landscape rating:5",
    "sort": "date_desc"
  }
]
```

```bash
curl http://localhost:8080/api/saved-searches
```

### `POST /api/saved-searches` -- Create/Update Saved Search

Creates a new saved search or updates an existing one with the same name. Persists to `searches.toml`.

**Content-Type**: `application/json`

**Request body**:
```json
{
  "name": "5-star landscapes",
  "query": "tag:landscape rating:5",
  "sort": "date_desc"
}
```

| Field  | Type          | Description                     |
|--------|---------------|---------------------------------|
| `name` | string        | Search name (required)          |
| `query` | string       | Search query string (required)  |
| `sort` | string or null | Sort order (optional)          |

**Response**:
```json
{
  "status": "saved",
  "name": "5-star landscapes"
}
```

```bash
curl -X POST http://localhost:8080/api/saved-searches \
  -H "Content-Type: application/json" \
  -d '{"name": "5-star landscapes", "query": "tag:landscape rating:5", "sort": "date_desc"}'
```

### `DELETE /api/saved-searches/{name}` -- Delete Saved Search

Deletes a saved search by name. Persists the change to `searches.toml`.

| Parameter | Type   | Description                        |
|-----------|--------|------------------------------------|
| `name`    | string | Saved search name (path param, URL-encoded) |

**Response**:
```json
{
  "status": "deleted",
  "name": "5-star landscapes"
}
```

**Errors**: Returns 404 if no saved search with that name exists.

```bash
curl -X DELETE http://localhost:8080/api/saved-searches/5-star%20landscapes
```

---

## Static Assets

### `GET /static/htmx.min.js`

Serves the embedded htmx library. Compiled into the binary at build time.

**Response**: `application/javascript`

### `GET /static/style.css`

Serves the embedded stylesheet. Compiled into the binary at build time.

**Response**: `text/css`

### `GET /preview/*`

Serves preview images from the `previews/` directory within the catalog root. Handled by `tower-http::ServeDir`.

Preview files are stored at `previews/<hash-prefix>/<hash>.jpg` (or `.webp` depending on configuration).

```bash
curl http://localhost:8080/preview/a1/a1b2c3d4e5f6...abc.jpg -o preview.jpg
```

---

## Error Handling

All endpoints return standard HTTP status codes:

| Code | Meaning                                              |
|------|------------------------------------------------------|
| 200  | Success                                              |
| 201  | Created (collection creation)                        |
| 303  | See Other (non-htmx redirect from `/api/search`)    |
| 404  | Not Found (missing asset or saved search)            |
| 409  | Conflict (duplicate collection name)                 |
| 500  | Internal Server Error (database or processing error) |

Error responses are plain text with a descriptive message.

---

## Content Types Summary

| Endpoint Pattern                      | Request Content-Type             | Response Content-Type  |
|---------------------------------------|----------------------------------|------------------------|
| Page routes (`/`, `/asset/*`, etc.)   | --                               | `text/html`            |
| Single-asset editing (`PUT/POST/DELETE /api/asset/*`) | `application/x-www-form-urlencoded` | `text/html` (partial) |
| Batch operations (`/api/batch/*`)     | `application/json`               | `application/json`     |
| Data APIs (`GET /api/*`)              | --                               | `application/json`     |
| Mutation APIs (`POST/DELETE /api/*`)  | `application/json`               | `application/json`     |
