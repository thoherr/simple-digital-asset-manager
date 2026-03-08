# Search Filter Reference

Complete reference for all search filters available in `dam search`, the web UI browse page, and saved searches. Filters use a `key:value` syntax and can be mixed freely with free-text terms in a single query string.

All filters combine with AND -- every filter must match for an asset to appear in results.

---

## Negation and OR Syntax

### Negation (Excluding Results)

Prefix any filter or free-text term with `-` to exclude matches:

```
dam search "-tag:rejected"               # exclude assets with the "rejected" tag
dam search "-format:xmp"                 # exclude XMP files
dam search "-type:other"                 # exclude "other" type assets
dam search "-label:Red"                  # exclude Red labeled assets
dam search "-camera:phone"               # exclude phone camera shots
dam search "landscape -tag:processed"    # landscapes that aren't tagged processed
dam search "-sunset"                     # exclude free-text match on "sunset"
```

Negation works with all filter types and free-text terms.

### OR Within a Filter (Comma Operator)

Use commas within a single filter value to match any of the given options (OR logic):

```
dam search "tag:alice,bob"               # assets tagged alice OR bob (or both)
dam search "format:nef,cr3"              # NEF or CR3 format
dam search "type:image,video"            # images or videos
dam search "label:Red,Orange"            # Red or Orange labeled assets
dam search "rating:4,5"                  # 4-star or 5-star assets
```

The comma operator works within a single filter value. To require multiple tags, repeat the filter:

```
dam search "tag:landscape tag:sunset"    # assets with BOTH landscape AND sunset tags
```

### Combining Negation and OR

Negation and OR can be combined in complex queries:

```
dam search "type:image,video -format:xmp"              # images or videos, but not XMP files
dam search "tag:landscape,portrait -label:Red"         # landscape or portrait, excluding Red labeled
dam search "format:nef,cr3 -tag:rejected rating:3+"    # RAW files (NEF or CR3), not rejected, 3+ stars
```

---

## Free Text

**Syntax:** any token that does not match a recognized `prefix:value` pattern

**Description:** Remaining tokens after all prefix filters are extracted are joined into a single free-text string. This text is matched against asset names, original filenames, descriptions, and source metadata values (partial, case-insensitive).

**Examples:**

```
dam search "sunset"
dam search "golden hour mountains"
dam search "sunset tag:landscape"       # "sunset" is free text, tag:landscape is a filter
```

**SQL behavior:** `WHERE (a.name LIKE '%text%' OR a.original_filename LIKE '%text%' OR a.description LIKE '%text%' OR v.source_metadata LIKE '%text%')`. Triggers a JOIN to the variants table for metadata matching.

---

## type

**Syntax:** `type:<value>`

**Values:** `image`, `video`, `audio`, `document`, `other`

**Description:** Filters by asset type. Exact match, case-sensitive.

**Examples:**

```
dam search "type:image"
dam search "type:video rating:3+"
```

**SQL behavior:** `WHERE a.asset_type = ?`. Pure assets-table filter, no JOIN required.

---

## tag

**Syntax:** `tag:<name>` or `tag:"<multi-word name>"`

**Description:** Filters to assets that have a specific tag. Supports quoted values for multi-word tags.

**Hierarchical matching:** Tags can be organized hierarchically using `/` as a separator (e.g., `animals/birds/eagles`). Searching for a parent tag matches all descendants: `tag:animals` finds assets tagged `animals`, `animals/birds`, and `animals/birds/eagles`. Searching for an intermediate level also works: `tag:animals/birds` matches both `animals/birds` and `animals/birds/eagles`.

Internally, hierarchical tags are stored with `|` as the separator (to avoid conflicts with literal slashes in tag names like `f/1.4`). The search system handles the conversion transparently -- users always type `/` in queries.

**Examples:**

```
dam search "tag:landscape"
dam search 'tag:"Fools Theater"'
dam search 'tag:"Black and White" rating:4+'
dam search "tag:animals"                   # matches animals, animals/birds, animals/birds/eagles
dam search "tag:animals/birds"             # matches animals/birds and animals/birds/eagles
```

**SQL behavior:** `WHERE (a.tags LIKE '%"stored"%' OR a.tags LIKE '%"stored|%')`. The second LIKE clause enables parent-matches-children semantics. Tags containing literal `/` get an additional raw fallback clause.

---

## format

**Syntax:** `format:<extension>`

**Description:** Filters by file format (extension) of any variant on the asset. Case-insensitive match against the variant's format field.

**Examples:**

```
dam search "format:nef"
dam search "format:jpg"
dam search "format:mp4 type:video"
dam search "format:tif tag:processed"
```

**SQL behavior:** `WHERE v.format = ?`. Triggers a JOIN to the variants table.

---

## rating

**Syntax:** `rating:<N>` (exact) or `rating:<N>+` (minimum)

**Values:** 1 through 5

**Description:** Filters by the asset's star rating (1--5 scale). Use a bare number for exact match, or append `+` for "this rating or higher." Ratings are extracted from XMP during import. Microsoft Photo ratings (percentage values 1--99) are automatically normalized to the 1--5 scale (1=1, 25=2, 50=3, 75=4, 99=5).

**Examples:**

```
dam search "rating:5"          # exactly 5 stars
dam search "rating:3+"         # 3 stars or more
dam search "rating:4+ tag:landscape"
```

**SQL behavior:**
- Exact: `WHERE a.rating = ?`
- Minimum: `WHERE a.rating >= ?`

Pure assets-table filter, no JOIN required.

---

## label

**Syntax:** `label:<color>`

**Values:** `Red`, `Orange`, `Yellow`, `Green`, `Blue`, `Pink`, `Purple` (case-insensitive input, stored as title-case)

**Description:** Filters by color label. The 7-color set is a superset of Lightroom's 5 colors, matching CaptureOne's label palette.

**Examples:**

```
dam search "label:Red"
dam search "label:blue"
dam search "label:Green rating:4+"
```

**SQL behavior:** `WHERE a.color_label = ?`. Pure assets-table filter, no JOIN required.

---

## camera

**Syntax:** `camera:<text>` or `camera:"<multi-word text>"`

**Description:** Partial match against the `camera_model` field in variant source metadata. Case-insensitive substring match.

**Examples:**

```
dam search "camera:fuji"
dam search 'camera:"Canon EOS R5"'
dam search 'camera:"NIKON Z 9" rating:4+'
```

**SQL behavior:** `WHERE v.source_metadata LIKE '%camera_model%' AND v.source_metadata LIKE '%value%'`. Triggers a JOIN to the variants table.

---

## lens

**Syntax:** `lens:<text>` or `lens:"<multi-word text>"`

**Description:** Partial match against the `lens` field in variant source metadata. Case-insensitive substring match.

**Examples:**

```
dam search "lens:56mm"
dam search 'lens:"RF 50mm f/1.2"'
dam search 'lens:"24-70" camera:sony'
```

**SQL behavior:** `WHERE v.source_metadata LIKE '%lens%' AND v.source_metadata LIKE '%value%'`. Triggers a JOIN to the variants table.

---

## iso

**Syntax:** `iso:<N>` (exact) or `iso:<N>+` (minimum) or `iso:<min>-<max>` (range)

**Description:** Filters by ISO sensitivity from variant source metadata. Supports exact match, minimum threshold, and inclusive range.

**Examples:**

```
dam search "iso:3200"          # exactly ISO 3200
dam search "iso:3200+"         # ISO 3200 or higher
dam search "iso:100-800"       # ISO between 100 and 800 inclusive
```

**SQL behavior:** Extracts the `iso` value from `v.source_metadata` JSON and applies numeric comparison. Triggers a JOIN to the variants table.

- Exact: sets both min and max to the same value
- Minimum (`+` suffix): sets min only, no upper bound
- Range: sets both min and max

---

## focal

**Syntax:** `focal:<N>` (exact) or `focal:<N>+` (minimum) or `focal:<min>-<max>` (range)

**Description:** Filters by focal length in millimeters from variant source metadata.

**Examples:**

```
dam search "focal:50"          # exactly 50mm
dam search "focal:200+"        # 200mm or longer
dam search "focal:35-70"       # between 35mm and 70mm inclusive
```

**SQL behavior:** Same pattern as `iso` but on the `focal_length` metadata field. Triggers a JOIN to the variants table.

---

## f (aperture)

**Syntax:** `f:<N>` (exact) or `f:<N>+` (minimum) or `f:<min>-<max>` (range)

**Description:** Filters by f-number (aperture) from variant source metadata. Supports decimal values.

**Examples:**

```
dam search "f:2.8"             # exactly f/2.8
dam search "f:1.4+"           # f/1.4 or wider (numerically >= 1.4)
dam search "f:1.4-2.8"        # between f/1.4 and f/2.8 inclusive
```

**SQL behavior:** Same pattern as `iso` but on the `f_number` metadata field with floating-point comparison. Triggers a JOIN to the variants table.

---

## width

**Syntax:** `width:<N>` (exact/minimum) or `width:<N>+` (minimum, explicit)

**Description:** Filters by image width in pixels from variant source metadata.

**Examples:**

```
dam search "width:4000+"       # 4000 pixels wide or more
dam search "width:3840"        # exactly 3840 pixels (4K UHD width)
dam search "width:4000+ height:2000+"
```

**SQL behavior:** `WHERE CAST(json_extract(v.source_metadata, '$.width') AS INTEGER) >= ?`. Triggers a JOIN to the variants table. Both bare numbers and `+`-suffixed numbers set a minimum threshold.

---

## height

**Syntax:** `height:<N>` (exact/minimum) or `height:<N>+` (minimum, explicit)

**Description:** Filters by image height in pixels from variant source metadata.

**Examples:**

```
dam search "height:2000+"      # 2000 pixels tall or more
dam search "height:2160"       # exactly 2160 pixels (4K UHD height)
```

**SQL behavior:** Same pattern as `width` but on the `height` metadata field. Triggers a JOIN to the variants table.

---

## meta

**Syntax:** `meta:<key>=<value>`

**Description:** Generic key-value match against variant source metadata. The key and value are matched against the JSON source_metadata blob. Useful for filtering on metadata fields that don't have a dedicated prefix filter.

**Examples:**

```
dam search "meta:software=CaptureOne"
dam search "meta:camera_make=SONY"
dam search "meta:color_space=sRGB"
```

**SQL behavior:** `WHERE v.source_metadata LIKE '%key%value%'`. Triggers a JOIN to the variants table. Multiple `meta:` filters can be specified and all must match (AND).

---

## path

**Syntax:** `path:<prefix>` or `path:"<prefix with spaces>"`

**Description:** Restricts results to assets that have at least one file location whose volume-relative path starts with the given prefix. Useful for finding everything imported from a specific directory.

**Automatic normalization** (CLI only, not in web UI or saved searches):

| Input | Behavior |
|-------|----------|
| `path:Capture/2026` | Used as-is (volume-relative prefix match) |
| `path:~/Photos/2026` | `~` expanded to `$HOME`, then matched against volumes |
| `path:./subdir` | Resolved relative to current working directory |
| `path:../other` | Resolved relative to current working directory |
| `path:/Volumes/Photos/Capture/2026` | Stripped to `Capture/2026` with implicit `volume:` filter for the Photos volume |

An explicit `volume:` filter in the same query takes precedence over the auto-detected volume from path normalization.

**Examples:**

```
dam search "path:Capture/2026-02-22"
dam search 'path:"Photos/Family Trip"'
dam search "path:Capture/2026 rating:3+ tag:landscape"
dam search "path:/Volumes/Photos/Capture/2026"    # auto-normalized
```

**SQL behavior:** `WHERE fl.relative_path LIKE 'prefix%'`. Triggers a JOIN to the file_locations table.

---

## date

**Syntax:** `date:<prefix>`

**Values:** `YYYY-MM-DD` (day), `YYYY-MM` (month), or `YYYY` (year)

**Description:** Prefix match on the asset's creation date. Matches any asset whose `created_at` timestamp starts with the given prefix. This gives you day, month, or year granularity in a single filter.

**Examples:**

```
dam search "date:2026-02-25"       # assets created on Feb 25, 2026
dam search "date:2026-02"          # assets created in February 2026
dam search "date:2026"             # assets created in 2026
dam search "date:2026-02 tag:landscape"
```

**SQL behavior:** `WHERE a.created_at LIKE 'prefix%'`. Pure assets-table filter, no JOIN required. Uses the index on `assets.created_at`.

---

## dateFrom

**Syntax:** `dateFrom:<date>`

**Values:** `YYYY-MM-DD` (or any date prefix)

**Description:** Inclusive lower bound on the asset's creation date. Commonly used with `dateUntil:` to define a date range.

**Examples:**

```
dam search "dateFrom:2026-01-01"                        # from Jan 1, 2026 onward
dam search "dateFrom:2026-01-01 dateUntil:2026-03-31"   # Q1 2026
dam search "dateFrom:2025-06-01 rating:4+"
```

**SQL behavior:** `WHERE a.created_at >= ?`. String comparison is correct for RFC 3339 timestamps. Pure assets-table filter, no JOIN required.

---

## dateUntil

**Syntax:** `dateUntil:<date>`

**Values:** `YYYY-MM-DD`, `YYYY-MM`, or `YYYY`

**Description:** Inclusive upper bound on the asset's creation date. Internally converted to an exclusive next-day/month/year boundary for correct range semantics.

**Examples:**

```
dam search "dateUntil:2026-02-28"                       # up to and including Feb 28
dam search "dateFrom:2026-01-01 dateUntil:2026-12-31"   # full year 2026
dam search "dateUntil:2025-12-31 type:video"            # all videos before 2026
```

**SQL behavior:** Converts the inclusive bound to exclusive: `"2026-02-28"` → `a.created_at < "2026-03-01"`. Pure assets-table filter, no JOIN required.

---

## collection

**Syntax:** `collection:<name>` or `collection:"<multi-word name>"`

**Description:** Restricts results to assets that belong to a specific static collection (album). The collection's member asset IDs are pre-loaded and passed as a filter set.

**Examples:**

```
dam search "collection:Favorites"
dam search 'collection:"My Favorites"'
dam search 'collection:"Travel 2026" rating:4+'
```

**SQL behavior:** Pre-computes the set of asset IDs in the named collection, then filters with `WHERE a.id IN (...)`. Pure in-memory filter after initial lookup.

---

## volume

**Syntax:** `volume:<label>` (web UI and route handler) or `volume:none` (query parser)

**Description:** The `volume:none` special value finds assets with no file locations on any currently online volume. The `volume:<label>` form (used via the web UI dropdown or programmatically) restricts to assets with at least one file location on the specified volume.

**Examples:**

```
dam search "volume:none"                    # assets not on any online volume
dam search "volume:none orphan:false"       # has locations, but all on offline volumes
```

In the web UI, the volume filter is a dropdown control rather than a typed query token. It passes the volume UUID to the search backend.

**SQL behavior:**
- `volume:none`: Pre-computes online volume IDs from DeviceRegistry. Adds `WHERE NOT EXISTS (SELECT 1 FROM file_locations fl JOIN variants v ... WHERE fl.volume_id IN (...online_ids...))`.
- `volume:<id>`: `WHERE fl.volume_id = ?`. Triggers a JOIN to the file_locations table.

---

## orphan

**Syntax:** `orphan:true`

**Description:** Finds assets that have zero file location records in the catalog. These are assets whose files have all been removed (via cleanup or manual deletion) but whose metadata records remain.

**Examples:**

```
dam search "orphan:true"
dam search "orphan:true type:image"
```

**SQL behavior:** `WHERE NOT EXISTS (SELECT 1 FROM file_locations fl JOIN variants v ON fl.content_hash = v.content_hash WHERE v.asset_id = a.id)`. Pure SQL subquery, no disk I/O.

---

## missing

**Syntax:** `missing:true`

**Description:** Finds assets where at least one file location points to a file that no longer exists on disk (on an online volume). This requires checking the filesystem and may be slow on large catalogs.

**Examples:**

```
dam search "missing:true"
dam search "missing:true type:video"
```

**SQL behavior:** Pre-computes affected asset IDs by iterating all file locations on online volumes and checking `Path::exists()`. The resulting ID set is passed as `WHERE a.id IN (...)`. Offline volumes are skipped.

---

## stale

**Syntax:** `stale:<N>`

**Values:** Number of days (0 or more)

**Description:** Finds assets with at least one file location that has not been verified in the specified number of days, or that has never been verified. Useful for identifying files due for integrity checks.

**Examples:**

```
dam search "stale:30"          # not verified in 30+ days (or never)
dam search "stale:0"           # never verified at all
dam search "stale:90 type:image"
```

**SQL behavior:** `WHERE EXISTS (SELECT 1 FROM file_locations fl JOIN variants v ... WHERE (fl.verified_at IS NULL OR fl.verified_at < datetime('now', '-N days')))`. Pure SQL, no disk I/O.

---

## stacked

**Syntax:** `stacked:true` or `stacked:false`

**Description:** Filters by whether an asset belongs to a stack. `stacked:true` finds assets that are members of a stack; `stacked:false` finds assets that are not in any stack.

**Examples:**

```
dam search "stacked:true"                  # all stacked assets
dam search "stacked:false rating:5"        # 5-star assets not in a stack
dam search "stacked:true type:image"       # stacked images
```

**SQL behavior:**
- `stacked:true`: `WHERE a.stack_id IS NOT NULL`
- `stacked:false`: `WHERE a.stack_id IS NULL`

Pure assets-table filter, no JOIN required.

---

## copies

**Syntax:** `copies:<N>` (exact) or `copies:<N>+` (minimum)

**Values:** Non-negative integer

**Description:** Filters by the total number of file locations across all variants of an asset. This counts every stored copy of every variant — a file on 3 volumes has 3 copies. Useful for finding assets with insufficient backup coverage or identifying heavily-duplicated files.

Common patterns:
- `copies:1` — assets with only a single copy on disk (no backup)
- `copies:2+` — assets with at least two copies (backed up)
- `copies:0` — equivalent to `orphan:true` (no file locations at all)

**Examples:**

```
dam search "copies:1"              # single-copy assets (backup risk)
dam search "copies:2"              # exactly 2 copies
dam search "copies:3+"             # 3 or more copies
dam search "copies:1 rating:4+"    # highly-rated assets with no backup
dam search "copies:2+ type:video"  # backed-up videos
```

**SQL behavior:** Scalar subquery: `(SELECT COUNT(*) FROM file_locations fl2 JOIN variants v2 ON fl2.content_hash = v2.content_hash WHERE v2.asset_id = a.id) = N` (or `>= N` for minimum). Self-contained, no outer JOIN flags needed.

---

## geo

**Syntax:** `geo:any` | `geo:none` | `geo:<lat>,<lng>,<radius_km>` | `geo:<south>,<west>,<north>,<east>`

**Description:** Filters by GPS geolocation. GPS coordinates are extracted from EXIF data during import and stored as denormalized `latitude`/`longitude` columns on the assets table.

**Modes:**

| Form | Description |
|------|-------------|
| `geo:any` | Assets that have GPS coordinates |
| `geo:none` | Assets without GPS coordinates |
| `geo:52.5,13.4,10` | Assets within 10km of latitude 52.5, longitude 13.4 (bounding circle approximated as a bounding box) |
| `geo:48.0,11.0,53.0,14.0` | Assets within the bounding box: south 48.0, west 11.0, north 53.0, east 14.0 |

The 3-parameter form (lat, lng, radius) is converted to a bounding box internally using the approximation `dlat = radius / 111` and `dlng = radius / (111 * cos(lat))`.

**Examples:**

```
dam search "geo:any"                           # all geotagged assets
dam search "geo:none"                          # assets without GPS data
dam search "geo:52.52,13.405,5"                # within 5km of Berlin center
dam search "geo:48.0,11.0,53.0,14.0"           # bounding box covering Germany
dam search "geo:any rating:4+ tag:landscape"   # geotagged 4+ star landscapes
```

**SQL behavior:**
- `geo:any`: `WHERE a.latitude IS NOT NULL AND a.longitude IS NOT NULL`
- `geo:none`: `WHERE (a.latitude IS NULL OR a.longitude IS NULL)`
- Bounding box/circle: `WHERE a.latitude >= ? AND a.latitude <= ? AND a.longitude >= ? AND a.longitude <= ?`

Pure assets-table filter, no JOIN required. Uses the composite index on `(latitude, longitude)`.

---

## faces

**Syntax:** `faces:any` | `faces:none` | `faces:<N>` | `faces:<N>+`

**Description:** Filters by the number of detected faces on an asset. Requires the `ai` feature for face detection to have been run.

**Modes:**

| Form | Description |
|------|-------------|
| `faces:any` | Assets with at least one detected face |
| `faces:none` | Assets with no detected faces |
| `faces:3` | Assets with exactly 3 detected faces |
| `faces:2+` | Assets with 2 or more detected faces |

**Examples:**

```
dam search "faces:any"                     # all assets with faces
dam search "faces:none type:image"         # images without detected faces
dam search "faces:3"                       # group portraits (exactly 3 faces)
dam search "faces:2+ tag:portrait"         # portraits with multiple people
```

**SQL behavior:** Uses the denormalized `face_count` column on the assets table. Pure assets-table filter, no JOIN required.

---

## person

**Syntax:** `person:<name>` or `person:"<multi-word name>"`

**Description:** Filters to assets that contain at least one face assigned to the named person. Supports quoted values for multi-word names.

**Examples:**

```
dam search "person:Alice"
dam search 'person:"John Smith"'
dam search "person:Alice rating:4+"
dam search "-person:Alice"                 # exclude assets with Alice
```

**SQL behavior:** Looks up the person ID by name, then filters via `WHERE EXISTS (SELECT 1 FROM faces WHERE faces.asset_id = a.id AND faces.person_id = ?)`.

---

## similar (AI feature)

**Syntax:** `similar:<asset-id>` or `similar:<asset-id>:<limit>`

**Description:** Finds visually similar assets using stored SigLIP embeddings. Returns the top N most similar assets (default 20). Requires the `ai` feature (`--features ai`) and embeddings to have been generated via `dam embed` or `dam import --embed`. The reference asset ID supports prefix matching (e.g. `similar:abc1` resolves like all other asset ID references).

**Examples:**

```
dam search "similar:72a0bb4b"                          # top 20 similar to this asset
dam search "similar:72a0bb4b:50"                       # top 50 similar
dam search "similar:72a0bb4b rating:3+ tag:landscape"  # similar AND 3+ stars AND landscape tag
dam search -q "similar:72a0bb4b"                       # just IDs, for scripting
```

**Behavior:** Looks up the stored embedding for the reference asset, loads all embeddings into an in-memory index, and performs a dot-product similarity search. If no embedding exists for the reference asset, exits with an error suggesting `dam embed --asset <id>`. The result set can be further filtered by all other search filters (AND logic).

---

## embed (AI feature)

**Syntax:** `embed:any` | `embed:true` | `embed:none` | `embed:false`

**Description:** Filters by whether an asset has a stored AI embedding (SigLIP image embedding). Requires the `ai` feature (`--features ai`) and embeddings to have been generated via `dam embed` or `dam import --embed`.

**Examples:**

```
dam search "embed:any"                     # assets with AI embeddings
dam search "embed:true"                    # same as embed:any
dam search "embed:none"                    # assets without AI embeddings
dam search "embed:false"                   # same as embed:none
dam search "embed:none type:image"         # images that still need embeddings
```

**SQL behavior:** Uses an `EXISTS` / `NOT EXISTS` subquery on the `embeddings` table: `WHERE EXISTS (SELECT 1 FROM embeddings e WHERE e.asset_id = a.id)`. Pure subquery, no JOIN required.

---

## Combining Filters

All filters are combined with AND logic. Every specified filter must match for an asset to appear in results. Free-text terms are also AND-combined with all prefix filters.

**Example combinations:**

```
# 5-star landscape images shot with a Nikon
dam search "rating:5 tag:landscape type:image camera:NIKON"

# High-ISO night shots in RAW format
dam search "iso:3200+ format:nef tag:night"

# Wide-angle portraits with shallow depth of field
dam search "focal:24-35 f:1.4-2.8 tag:portrait"

# Unverified images on the Photos volume, imported from a specific path
dam search "stale:30 type:image path:Capture/2026-01"

# 4K or larger images in a specific collection
dam search 'width:3840+ collection:"Best of 2026"'

# Orphaned video assets (no files on disk)
dam search "orphan:true type:video"

# Everything labeled Red with 4+ stars
dam search "label:Red rating:4+"

# Single-copy RAW files (backup risk)
dam search "copies:1 format:nef"

# Well-backed-up 5-star images
dam search "copies:2+ rating:5 type:image"

# Assets from a specific date range
dam search "dateFrom:2026-01-01 dateUntil:2026-03-31 tag:landscape"

# Everything shot in February 2026
dam search "date:2026-02"

# Unstacked 5-star images (candidates for stacking review)
dam search "stacked:false rating:5 type:image"

# Find stacked assets with a hierarchical tag
dam search "stacked:true tag:animals/birds"

# Visually similar assets, filtered to 4+ stars (requires ai feature + embeddings)
dam search "similar:72a0bb4b rating:4+"

# Geotagged photos within 5km of a location
dam search "geo:52.52,13.405,5 rating:4+"

# All geotagged landscape images
dam search "geo:any tag:landscape type:image"
```

---

## Quoted Values

Use double quotes around filter values that contain spaces. When typing at the shell, wrap the entire query in single quotes to prevent the shell from stripping the inner double quotes.

```bash
# Shell-safe quoting: single quotes outside, double quotes for values
dam search 'tag:"Fools Theater"'
dam search 'camera:"Canon EOS R5" lens:"RF 50mm f/1.2"'
dam search 'collection:"My Favorites" rating:4+'
dam search 'path:"Photos/Family Trip" type:image'
```

Unquoted single-word values continue to work without quotes:

```bash
dam search "tag:landscape"
dam search "camera:fuji"
```

---

## Filter Availability

| Filter | CLI `dam search` | Web UI | Saved Searches |
|--------|:---:|:---:|:---:|
| Free text | yes | yes (text input) | yes |
| `type:` | yes | yes (dropdown) | yes |
| `tag:` | yes | yes (dropdown) | yes |
| `format:` | yes | yes (dropdown) | yes |
| `rating:` | yes | yes (star clicks) | yes |
| `label:` | yes | yes (color dots) | yes |
| `camera:` | yes | no | yes |
| `lens:` | yes | no | yes |
| `iso:` | yes | no | yes |
| `focal:` | yes | no | yes |
| `f:` | yes | no | yes |
| `width:` | yes | no | yes |
| `height:` | yes | no | yes |
| `meta:` | yes | no | yes |
| `path:` | yes | yes (text input) | yes |
| `collection:` | yes | yes (dropdown) | yes |
| `volume:` | dropdown only | yes (dropdown) | yes |
| `volume:none` | yes | no | yes |
| `copies:` | yes | no | yes |
| `date:` | yes | yes (query input) | yes |
| `dateFrom:` | yes | yes (query input) | yes |
| `dateUntil:` | yes | yes (query input) | yes |
| `orphan:true` | yes | no | yes |
| `missing:true` | yes | no | yes |
| `stale:` | yes | no | yes |
| `stacked:` | yes | no | yes |
| `geo:` | yes | yes (query input) | yes |
| `faces:` | yes | yes (query input) | yes |
| `person:` | yes | yes (dropdown) | yes |
| `similar:` | yes (ai feature) | yes (detail page) | no |
| `embed:` | yes (ai feature) | no | yes |

---

## Related Topics

- [Browse & Search (User Guide)](../user-guide/05-browse-and-search.md) -- practical search workflows and output formatting
- [Format Templates Reference](07-format-templates.md) -- controlling output with `--format` and custom templates
- [Configuration Reference](08-configuration.md) -- `dam.toml` settings
