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
| `format`     | string | `""`        | File format filter (e.g. `NEF`, `JPEG`)          |
| `volume`     | string | `""`        | Volume label filter                              |
| `rating`     | string | `""`        | Rating filter (`5` exact, `3+` minimum)          |
| `label`      | string | `""`        | Color label filter (e.g. `Red`, `Green`)         |
| `collection` | string | `""`        | Collection name filter                           |
| `path`       | string | `""`        | Path prefix filter                               |
| `sort`       | string | `date_desc` | Sort order (see below)                           |
| `page`       | u32    | `1`         | Page number (60 results per page)                |

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

### `GET /backup` -- Backup Status Page

Returns an HTML page showing backup health: summary cards (total assets, at-risk count, min copies), volume distribution bar chart, coverage by purpose table, and volume gaps table.

```bash
curl http://localhost:8080/backup
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

### `POST /api/asset/{id}/preview` -- Regenerate Preview

Regenerates the preview thumbnail for the asset's primary variant. Requires the source file to be on an online volume.

**Content-Type**: None (no body required)

**Response**: HTML partial -- updated preview fragment.

```bash
curl -X POST http://localhost:8080/api/asset/{id}/preview
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

## Data APIs

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
