# Search Filter Reference

Complete reference for all search filters available in `maki search`, the web UI browse page, and saved searches. Filters use a `key:value` syntax and can be mixed freely with free-text terms in a single query string.

All filters combine with AND -- every filter must match for an asset to appear in results.

---

## Negation and OR Syntax

### Negation (Excluding Results)

Prefix any filter or free-text term with `-` to exclude matches:

```
maki search "-tag:rejected"               # exclude assets with the "rejected" tag
maki search "-format:xmp"                 # exclude XMP files
maki search "-type:other"                 # exclude "other" type assets
maki search "-label:Red"                  # exclude Red labeled assets
maki search "-camera:phone"               # exclude phone camera shots
maki search "landscape -tag:processed"    # landscapes that aren't tagged processed
maki search "-sunset"                     # exclude free-text match on "sunset"
```

Negation works with all filter types and free-text terms.

### OR Within a Filter (Comma Operator)

Use commas within a single filter value to match any of the given options (OR logic):

```
maki search "tag:alice,bob"               # assets tagged alice OR bob (or both)
maki search "format:nef,cr3"              # NEF or CR3 format
maki search "type:image,video"            # images or videos
maki search "label:Red,Orange"            # Red or Orange labeled assets
maki search "rating:4,5"                  # 4-star or 5-star assets
```

The comma operator works within a single filter value. To require multiple tags, repeat the filter:

```
maki search "tag:landscape tag:sunset"    # assets with BOTH landscape AND sunset tags
```

### Combining Negation and OR

Negation and OR can be combined in complex queries:

```
maki search "type:image,video -format:xmp"              # images or videos, but not XMP files
maki search "tag:landscape,portrait -label:Red"         # landscape or portrait, excluding Red labeled
maki search "format:nef,cr3 -tag:rejected rating:3+"    # RAW files (NEF or CR3), not rejected, 3+ stars
```

---

## Free Text

**Syntax:** any token that does not match a recognized `prefix:value` pattern

**Description:** Remaining tokens after all prefix filters are extracted are joined into a single free-text string. This text is matched against asset names, original filenames, descriptions, and source metadata values (partial, case-insensitive).

**Examples:**

```
maki search "sunset"
maki search "golden hour mountains"
maki search "sunset tag:landscape"       # "sunset" is free text, tag:landscape is a filter
```

**SQL behavior:** `WHERE (a.name LIKE '%text%' OR a.original_filename LIKE '%text%' OR a.description LIKE '%text%' OR v.source_metadata LIKE '%text%')`. Triggers a JOIN to the variants table for metadata matching.

---

## type

**Syntax:** `type:<value>`

**Values:** `image`, `video`, `audio`, `document`, `other`

**Description:** Filters by asset type. Exact match, case-sensitive.

**Examples:**

```
maki search "type:image"
maki search "type:video rating:3+"
```

**SQL behavior:** `WHERE a.asset_type = ?`. Pure assets-table filter, no JOIN required.

---

## tag

**Syntax:** `tag:<name>`, `tag:"<multi-word name>"`, `tag:=<name>` (exact level), or `tag:^<name>` (case-sensitive)

**Description:** Filters to assets that have a specific tag. Supports quoted values for multi-word tags.

**Hierarchical matching:** Tags can be organized hierarchically using `|` as a separator (e.g., `animals|birds|eagles`), aligned with Lightroom and CaptureOne conventions. `>` is also accepted as an alternative separator. Searching for a parent tag matches all descendants: `tag:animals` finds assets tagged `animals`, `animals|birds`, and `animals|birds|eagles`. Searching for an intermediate level also works: `tag:animals|birds` matches both `animals|birds` and `animals|birds|eagles`.

**This-level-only match:** Prefix with `=` to match assets tagged at exactly this level, excluding those with deeper tags in the same branch. `tag:=location|Germany|Bayern` matches assets whose deepest tag in this branch is `Bayern` — NOT assets that also have `location|Germany|Bayern|München`. In the web UI, click the `▼` indicator on a tag chip to toggle to `=` (this-level-only) mode.

**Case-sensitive match:** Tag matching is case-insensitive by default (`tag:landscape` matches both `landscape` and `Landscape`). Prefix with `^` to make the match case-sensitive — useful when cleaning up tag duplicates like `landscape` vs `Landscape`, which the tags page shows as distinct entries. In the web UI, each tag chip has a small `cc`/`Cc` toggle next to the `▼`/`=` mode toggle: click it to flip that specific chip between case-insensitive (`cc`) and case-sensitive (`Cc`) matching.

The `=` and `^` prefixes can be combined in any order: `tag:=^Foo` or `tag:^=Foo`.

`/` is treated as a literal character in tag names (e.g., `f/1.4` works naturally without escaping).

**Examples:**

```
maki search "tag:landscape"
maki search 'tag:"Fools Theater"'
maki search 'tag:"Black and White" rating:4+'
maki search "tag:animals"                      # matches animals, animals|birds, animals|birds|eagles
maki search "tag:animals|birds"                # matches animals|birds and animals|birds|eagles
maki search "tag:=animals|birds"               # this level only: has birds but no deeper tag
maki search 'tag:="location|Germany|Bayern"'   # Bayern level only, not cities/venues below
maki search "tag:^Landscape"                   # case-sensitive: matches "Landscape" but NOT "landscape"
maki search "tag:=^Animals"                    # exact level AND case-sensitive
```

**SQL behavior:** `WHERE (a.tags LIKE '%"stored"%' OR a.tags LIKE '%"stored|%')`. The second LIKE clause enables parent-matches-children semantics. With `=` prefix: `WHERE (a.tags LIKE '%"stored"%' AND a.tags NOT LIKE '%"stored|%')` — matches the tag but excludes assets with deeper descendants. With `^` prefix: SQLite `GLOB` is used instead of `LIKE` — GLOB is case-sensitive and uses `*` as the wildcard (literal `*` and `?` in tag names are an unsupported edge case for case-sensitive search).

---

## format

**Syntax:** `format:<extension>`

**Description:** Filters by file format (extension) of any variant on the asset. Case-insensitive match against the variant's format field.

**Examples:**

```
maki search "format:nef"
maki search "format:jpg"
maki search "format:mp4 type:video"
maki search "format:tif tag:processed"
```

**SQL behavior:** `WHERE v.format = ?`. Triggers a JOIN to the variants table.

---

## rating

**Syntax:** `rating:<N>` | `rating:<N>+` | `rating:<N>-<M>` | `rating:<N>,<M>` | `rating:<N>,<M>+`

**Values:** 0 through 5 (0 = unrated)

**Description:** Filters by the asset's star rating (1--5 scale, 0 = unrated). Supports the full numeric filter syntax:

| Syntax | Meaning | Example |
|--------|---------|---------|
| `N` | exactly N | `rating:5` |
| `N+` | N or more | `rating:3+` |
| `N-M` | from N to M | `rating:3-5` |
| `N,M` | exactly N OR M | `rating:2,4` |
| `N,M+` | exactly N OR M+ | `rating:2,4+` |

Ratings are extracted from XMP during import. Microsoft Photo ratings (percentage values 1--99) are automatically normalized to the 1--5 scale (1=1, 25=2, 50=3, 75=4, 99=5).

**Examples:**

```
maki search "rating:5"          # exactly 5 stars
maki search "rating:3+"         # 3 stars or more
maki search "rating:3-5"        # 3, 4, or 5 stars
maki search "rating:2,4"        # exactly 2 or 4 stars
maki search "rating:2,4+"       # exactly 2, or 4 stars and above
maki search "rating:0"          # unrated assets
```

**SQL behavior:**
- Exact: `WHERE a.rating = ?`
- Minimum: `WHERE a.rating >= ?`
- Range: `WHERE a.rating >= ? AND a.rating <= ?`
- Values: `WHERE a.rating IN (?, ?)`
- Combined: `WHERE (a.rating IN (?) OR a.rating >= ?)`

Pure assets-table filter, no JOIN required.

---

## label

**Syntax:** `label:<color>`

**Values:** `Red`, `Orange`, `Yellow`, `Green`, `Blue`, `Pink`, `Purple` (case-insensitive input, stored as title-case)

**Description:** Filters by color label. The 7-color set is a superset of Lightroom's 5 colors, matching CaptureOne's label palette.

**Examples:**

```
maki search "label:Red"
maki search "label:blue"
maki search "label:Green rating:4+"
```

**SQL behavior:** `WHERE a.color_label = ?`. Pure assets-table filter, no JOIN required.

---

## camera

**Syntax:** `camera:<text>` or `camera:"<multi-word text>"`

**Description:** Partial match against the `camera_model` field in variant source metadata. Case-insensitive substring match.

**Examples:**

```
maki search "camera:fuji"
maki search 'camera:"Canon EOS R5"'
maki search 'camera:"NIKON Z 9" rating:4+'
```

**SQL behavior:** `WHERE v.source_metadata LIKE '%camera_model%' AND v.source_metadata LIKE '%value%'`. Triggers a JOIN to the variants table.

---

## lens

**Syntax:** `lens:<text>` or `lens:"<multi-word text>"`

**Description:** Partial match against the `lens` field in variant source metadata. Case-insensitive substring match.

**Examples:**

```
maki search "lens:56mm"
maki search 'lens:"RF 50mm f/1.2"'
maki search 'lens:"24-70" camera:sony'
```

**SQL behavior:** `WHERE v.source_metadata LIKE '%lens%' AND v.source_metadata LIKE '%value%'`. Triggers a JOIN to the variants table.

---

## iso

**Syntax:** `iso:<N>` (exact) or `iso:<N>+` (minimum) or `iso:<min>-<max>` (range)

**Description:** Filters by ISO sensitivity from variant source metadata. Supports exact match, minimum threshold, and inclusive range.

**Examples:**

```
maki search "iso:3200"          # exactly ISO 3200
maki search "iso:3200+"         # ISO 3200 or higher
maki search "iso:100-800"       # ISO between 100 and 800 inclusive
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
maki search "focal:50"          # exactly 50mm
maki search "focal:200+"        # 200mm or longer
maki search "focal:35-70"       # between 35mm and 70mm inclusive
```

**SQL behavior:** Same pattern as `iso` but on the `focal_length` metadata field. Triggers a JOIN to the variants table.

---

## f (aperture)

**Syntax:** `f:<N>` (exact) or `f:<N>+` (minimum) or `f:<min>-<max>` (range)

**Description:** Filters by f-number (aperture) from variant source metadata. Supports decimal values.

**Examples:**

```
maki search "f:2.8"             # exactly f/2.8
maki search "f:1.4+"           # f/1.4 or wider (numerically >= 1.4)
maki search "f:1.4-2.8"        # between f/1.4 and f/2.8 inclusive
```

**SQL behavior:** Same pattern as `iso` but on the `f_number` metadata field with floating-point comparison. Triggers a JOIN to the variants table.

---

## width

**Syntax:** `width:<N>` (exact/minimum) or `width:<N>+` (minimum, explicit)

**Description:** Filters by image width in pixels from variant source metadata.

**Examples:**

```
maki search "width:4000+"       # 4000 pixels wide or more
maki search "width:3840"        # exactly 3840 pixels (4K UHD width)
maki search "width:4000+ height:2000+"
```

**SQL behavior:** `WHERE CAST(json_extract(v.source_metadata, '$.width') AS INTEGER) >= ?`. Triggers a JOIN to the variants table. Both bare numbers and `+`-suffixed numbers set a minimum threshold.

---

## height

**Syntax:** `height:<N>` (exact/minimum) or `height:<N>+` (minimum, explicit)

**Description:** Filters by image height in pixels from variant source metadata.

**Examples:**

```
maki search "height:2000+"      # 2000 pixels tall or more
maki search "height:2160"       # exactly 2160 pixels (4K UHD height)
```

**SQL behavior:** Same pattern as `width` but on the `height` metadata field. Triggers a JOIN to the variants table.

---

## meta

**Syntax:** `meta:<key>=<value>`

**Description:** Generic key-value match against variant source metadata. The key and value are matched against the JSON source_metadata blob. Useful for filtering on metadata fields that don't have a dedicated prefix filter.

**Examples:**

```
maki search "meta:software=CaptureOne"
maki search "meta:camera_make=SONY"
maki search "meta:color_space=sRGB"
```

**SQL behavior:** `WHERE v.source_metadata LIKE '%key%value%'`. Triggers a JOIN to the variants table. Multiple `meta:` filters can be specified and all must match (AND).

---

## path

**Syntax:** `path:<pattern>` or `path:"<pattern with spaces>"`

**Description:** Restricts results to assets that have at least one file location whose volume-relative path matches the given pattern. Without wildcards, this is a fast prefix match. The `*` character is a wildcard that matches any sequence of characters (including slashes).

**Wildcards:**

| Pattern | Meaning | Speed |
|---------|---------|-------|
| `path:Pictures/2026` | Prefix from root: matches `Pictures/2026...` | fast (index scan) |
| `path:Pictures/*/Capture` | Anything between `Pictures/` and `/Capture` | fast (left-anchored) |
| `path:*/2026/*/party` | Slash-anchored substring search | slow (full scan) |
| `path:*party` | Substring match anywhere | slow (full scan) |

A trailing `*` is implicit — `path:Pictures/2026` and `path:Pictures/2026*` are equivalent.

**Performance note:** Patterns with no leading `*` use the SQLite index on `relative_path` and are fast even on large catalogs. Patterns starting with `*` force a full table scan and are noticeably slower on catalogs with hundreds of thousands of assets.

**Automatic normalization** (CLI only, not in web UI or saved searches):

| Input | Behavior |
|-------|----------|
| `path:Capture/2026` | Used as-is (volume-relative prefix match) |
| `path:~/Photos/2026` | `~` expanded to `$HOME`, then matched against volumes |
| `path:./subdir` | Resolved relative to current working directory |
| `path:../other` | Resolved relative to current working directory |
| `path:/Volumes/Photos/Capture/2026` | Stripped to `Capture/2026` with implicit `volume:` filter for the Photos volume |

Patterns containing `*` skip the absolute-path / volume normalization (they pass through unchanged). An explicit `volume:` filter in the same query takes precedence over the auto-detected volume from path normalization.

**Examples:**

```
maki search "path:Capture/2026-02-22"
maki search 'path:"Photos/Family Trip"'
maki search "path:Capture/2026 rating:3+ tag:landscape"
maki search "path:/Volumes/Photos/Capture/2026"     # auto-normalized
maki search "path:*/2026/*/wedding"                  # find any wedding shoot in 2026
maki search "path:*party"                            # any path containing "party"
```

**SQL behavior:** `WHERE fl.relative_path LIKE 'pattern%' ESCAPE '\'`. The user's `*` is translated to SQL `%`; literal `%` and `_` in the user input are escaped. Triggers a JOIN to the file_locations table.

---

## date

**Syntax:** `date:<prefix>`

**Values:** `YYYY-MM-DD` (day), `YYYY-MM` (month), or `YYYY` (year)

**Description:** Prefix match on the asset's creation date. Matches any asset whose `created_at` timestamp starts with the given prefix. This gives you day, month, or year granularity in a single filter.

**Examples:**

```
maki search "date:2026-02-25"       # assets created on Feb 25, 2026
maki search "date:2026-02"          # assets created in February 2026
maki search "date:2026"             # assets created in 2026
maki search "date:2026-02 tag:landscape"
```

**SQL behavior:** `WHERE a.created_at LIKE 'prefix%'`. Pure assets-table filter, no JOIN required. Uses the index on `assets.created_at`.

---

## dateFrom

**Syntax:** `dateFrom:<date>`

**Values:** `YYYY-MM-DD` (or any date prefix)

**Description:** Inclusive lower bound on the asset's creation date. Commonly used with `dateUntil:` to define a date range.

**Examples:**

```
maki search "dateFrom:2026-01-01"                        # from Jan 1, 2026 onward
maki search "dateFrom:2026-01-01 dateUntil:2026-03-31"   # Q1 2026
maki search "dateFrom:2025-06-01 rating:4+"
```

**SQL behavior:** `WHERE a.created_at >= ?`. String comparison is correct for RFC 3339 timestamps. Pure assets-table filter, no JOIN required.

---

## dateUntil

**Syntax:** `dateUntil:<date>`

**Values:** `YYYY-MM-DD`, `YYYY-MM`, or `YYYY`

**Description:** Inclusive upper bound on the asset's creation date. Internally converted to an exclusive next-day/month/year boundary for correct range semantics.

**Examples:**

```
maki search "dateUntil:2026-02-28"                       # up to and including Feb 28
maki search "dateFrom:2026-01-01 dateUntil:2026-12-31"   # full year 2026
maki search "dateUntil:2025-12-31 type:video"            # all videos before 2026
```

**SQL behavior:** Converts the inclusive bound to exclusive: `"2026-02-28"` → `a.created_at < "2026-03-01"`. Pure assets-table filter, no JOIN required.

---

## collection

**Syntax:** `collection:<name>` or `collection:"<multi-word name>"`

**Description:** Restricts results to assets that belong to a specific static collection (album). The collection's member asset IDs are pre-loaded and passed as a filter set.

**Examples:**

```
maki search "collection:Favorites"
maki search 'collection:"My Favorites"'
maki search 'collection:"Travel 2026" rating:4+'
```

**SQL behavior:** Pre-computes the set of asset IDs in the named collection, then filters with `WHERE a.id IN (...)`. Pure in-memory filter after initial lookup.

---

## volume

**Syntax:** `volume:<label>`, `volume:<label1>,<label2>`, `-volume:<label>`, `volume:none`

**Description:** Restricts results to assets that have at least one file location on the specified volume. Volume labels are matched case-insensitively. Supports negation (`-volume:Archive`) and comma-separated OR (`volume:Photos,Working`). The special value `volume:none` finds assets with no file locations on any currently online volume.

**Examples:**

```
maki search "volume:Photos"                  # assets on the Photos volume
maki search "volume:ScreenSaver type:image"  # images on the ScreenSaver volume
maki search "volume:Photos,Working"          # assets on either volume
maki search "-volume:Archive"                # exclude assets on the Archive volume
maki search "volume:none"                    # assets not on any online volume
maki search "volume:none orphan:false"       # has locations, but all on offline volumes
maki search "volume:\"External SSD\""        # volume label with spaces (quoted)
```

In the web UI, the volume filter is also available as a dropdown control that passes the volume UUID directly to the search backend.

**SQL behavior:**
- `volume:<label>`: Label is resolved to a volume UUID via DeviceRegistry. `WHERE fl.volume_id IN (...)`. Triggers a JOIN to the file_locations table.
- `-volume:<label>`: Resolved to UUID, then `WHERE a.id NOT IN (SELECT DISTINCT fl2.asset_id FROM file_locations fl2 WHERE fl2.volume_id IN (...))`.
- `volume:none`: Pre-computes online volume IDs from DeviceRegistry. Adds `WHERE NOT EXISTS (SELECT 1 FROM file_locations fl JOIN variants v ... WHERE fl.volume_id IN (...online_ids...))`.

---

## id

**Syntax:** `id:<prefix>`

**Description:** Matches assets whose UUID starts with the given prefix. Useful for quickly finding a specific asset by its ID (or the beginning of it) in both CLI and web UI.

**Examples:**

```
maki search "id:c654e"
maki search "id:c654efa4-4e55"
```

**SQL behavior:** `WHERE a.id LIKE 'prefix%'`. Pure SQL prefix match.

---

## orphan

**Syntax:** `orphan:true` | `orphan:false`

**Description:** Filters by whether an asset has any file location records.

- `orphan:true` — assets with zero file locations (files removed but metadata remains)
- `orphan:false` — assets with at least one file location (has files on disk)

**Examples:**

```
maki search "orphan:true"                   # assets with no files on disk
maki search "orphan:false type:image"       # images that have files
maki search "volume:none orphan:false"      # has files, but not on any online volume
```

**SQL behavior:** `orphan:true` uses `NOT EXISTS (...)`, `orphan:false` uses `EXISTS (...)` on the file_locations/variants subquery. Pure SQL, no disk I/O.

---

## missing

**Syntax:** `missing:true`

**Description:** Finds assets where at least one file location points to a file that no longer exists on disk (on an online volume). This requires checking the filesystem and may be slow on large catalogs.

**Examples:**

```
maki search "missing:true"
maki search "missing:true type:video"
```

**SQL behavior:** Pre-computes affected asset IDs by iterating all file locations on online volumes and checking `Path::exists()`. The resulting ID set is passed as `WHERE a.id IN (...)`. Offline volumes are skipped.

---

## stale

**Syntax:** `stale:<N>`

**Values:** Number of days (0 or more)

**Description:** Finds assets with at least one file location that has not been verified in the specified number of days, or that has never been verified. Useful for identifying files due for integrity checks.

**Examples:**

```
maki search "stale:30"          # not verified in 30+ days (or never)
maki search "stale:0"           # never verified at all
maki search "stale:90 type:image"
```

**SQL behavior:** `WHERE EXISTS (SELECT 1 FROM file_locations fl JOIN variants v ... WHERE (fl.verified_at IS NULL OR fl.verified_at < datetime('now', '-N days')))`. Pure SQL, no disk I/O.

---

## stacked

**Syntax:** `stacked:true` or `stacked:false`

**Description:** Filters by whether an asset belongs to a stack. `stacked:true` finds assets that are members of a stack; `stacked:false` finds assets that are not in any stack.

**Examples:**

```
maki search "stacked:true"                  # all stacked assets
maki search "stacked:false rating:5"        # 5-star assets not in a stack
maki search "stacked:true type:image"       # stacked images
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
maki search "copies:1"              # single-copy assets (backup risk)
maki search "copies:2"              # exactly 2 copies
maki search "copies:3+"             # 3 or more copies
maki search "copies:1 rating:4+"    # highly-rated assets with no backup
maki search "copies:2+ type:video"  # backed-up videos
```

**SQL behavior:** Scalar subquery: `(SELECT COUNT(*) FROM file_locations fl2 JOIN variants v2 ON fl2.content_hash = v2.content_hash WHERE v2.asset_id = a.id) = N` (or `>= N` for minimum). Self-contained, no outer JOIN flags needed.

---

## variants

**Syntax:** `variants:<N>` (exact) or `variants:<N>+` (minimum)

**Values:** Non-negative integer

**Description:** Filters by the number of variants belonging to an asset. This counts distinct content-addressed files (originals, alternates, exports, processed) grouped under one asset. Useful for auditing mis-grouped assets (too many variants) or finding single-file assets.

Common patterns:
- `variants:1` — assets with exactly one variant (single file)
- `variants:3+` — assets with 3 or more variants (potentially mis-grouped)
- `variants:5+` — suspiciously large groups worth manual review

**Examples:**

```
maki search "variants:1"              # single-variant assets
maki search "variants:3+"             # assets with 3+ variants
maki search "variants:5+ type:image"  # images with many variants
```

**SQL behavior:** Direct filter on the denormalized `a.variant_count` column. No JOIN required.

---

## scattered

**Syntax:** `scattered:<N>` or `scattered:<N>/<depth>`

**Values:** Positive integer (minimum number of distinct directories). Optional `/<depth>` specifies how many path segments to compare.

**Description:** Finds assets whose variant files are stored in multiple distinct directories. Counts distinct directory paths across all file locations of an asset's variants, ignoring the volume — so backup copies in the same relative path on different volumes don't count as scattered.

By default, the full parent directory (everything before the filename) is compared. The optional `/<depth>` parameter truncates paths to the first N segments before counting, which is useful when subdirectories like `Selects/` and `Output/` within the same shoot folder shouldn't count as scattered.

**Examples:**

```
maki search "scattered:2+"                   # files in 2+ distinct directories
maki search "scattered:2+ variants:3+"       # scattered + many variants
maki search "scattered:2+/1"                 # scattered at top-level directory
                                             # (2026-03-10/Selects/a.nef and
                                             #  2026-03-10/Output/a.nef → same at depth 1)
maki search "scattered:2+/2"                 # scattered at 2 directory levels
```

**Depth examples** with paths `2026-03-10/Selects/img.nef` and `2026-03-10/Output/img.nef`:

| Syntax | Compared as | Count | Scattered? |
|--------|------------|-------|------------|
| `scattered:2+` | `2026-03-10/Selects` vs `2026-03-10/Output` | 2 | yes |
| `scattered:2+/1` | `2026-03-10` vs `2026-03-10` | 1 | no |
| `scattered:2+/2` | `2026-03-10/Selects` vs `2026-03-10/Output` | 2 | yes |

**SQL behavior:** Uses a custom `path_dir(path, depth)` function to extract directory prefixes. Scalar subquery counting `DISTINCT path_dir(relative_path, depth)` across `file_locations` joined through `variants`. Self-contained, no outer JOIN flags needed.

---

## duration

**Syntax:** `duration:<N>` | `duration:<N>+` | `duration:<min>-<max>` | `duration:<N>,<M>`

**Values:** Duration in seconds. Supports all standard numeric filter forms: exact (`60`), minimum (`30+`), range (`10-120`), OR (`30,60`), OR+min (`30,60+`).

**Description:** Filters by video duration. Duration is extracted from video files via ffprobe during import and stored as a denormalized `video_duration` column on the assets table. Non-video assets (images, audio) have no duration and will not match this filter.

**Examples:**

```
maki search "duration:60"                       # exactly 60 seconds
maki search "duration:30+"                      # 30 seconds or longer
maki search "duration:10-120"                   # between 10s and 2 minutes
maki search "duration:30,60"                    # exactly 30s or 60s
maki search "type:video duration:60+"           # videos at least 1 minute
maki search "duration:10-30 rating:4+"          # short clips, highly rated
```

**SQL behavior:** Direct filter on the denormalized `a.video_duration` column using `NumericFilter` (exact/min/range/values). No JOIN required.

---

## codec

**Syntax:** `codec:<text>`

**Values:** Partial, case-insensitive string match against the video codec.

**Description:** Filters by video codec. The codec is extracted from video files via ffprobe during import and stored as a denormalized `video_codec` column on the assets table. Common values include `h264`, `hevc` (H.265), `prores`, `av1`, `vp9`. Non-video assets have no codec and will not match this filter.

**Examples:**

```
maki search "codec:h264"                        # H.264 encoded videos
maki search "codec:hevc"                        # H.265/HEVC videos
maki search "codec:prores"                      # ProRes videos
maki search "type:video codec:h264 rating:4+"   # highly rated H.264 videos
maki search "codec:hevc duration:60+"           # HEVC videos at least 1 minute
```

**SQL behavior:** `WHERE a.video_codec LIKE '%value%'` (case-insensitive via SQLite `COLLATE NOCASE`). Pure assets-table filter, no JOIN required.

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
maki search "geo:any"                           # all geotagged assets
maki search "geo:none"                          # assets without GPS data
maki search "geo:52.52,13.405,5"                # within 5km of Berlin center
maki search "geo:48.0,11.0,53.0,14.0"           # bounding box covering Germany
maki search "geo:any rating:4+ tag:landscape"   # geotagged 4+ star landscapes
```

**SQL behavior:**
- `geo:any`: `WHERE a.latitude IS NOT NULL AND a.longitude IS NOT NULL`
- `geo:none`: `WHERE (a.latitude IS NULL OR a.longitude IS NULL)`
- Bounding box/circle: `WHERE a.latitude >= ? AND a.latitude <= ? AND a.longitude >= ? AND a.longitude <= ?`

Pure assets-table filter, no JOIN required. Uses the composite index on `(latitude, longitude)`.

---

## faces *(Pro)*

**Syntax:** `faces:any` | `faces:none` | `faces:<N>` | `faces:<N>+`

**Description:** Filters by the number of detected faces on an asset. Requires face detection to have been run.

**Modes:**

| Form | Description |
|------|-------------|
| `faces:any` | Assets with at least one detected face |
| `faces:none` | Assets with no detected faces |
| `faces:3` | Assets with exactly 3 detected faces |
| `faces:2+` | Assets with 2 or more detected faces |

**Examples:**

```
maki search "faces:any"                     # all assets with faces
maki search "faces:none type:image"         # images without detected faces
maki search "faces:3"                       # group portraits (exactly 3 faces)
maki search "faces:2+ tag:portrait"         # portraits with multiple people
```

**SQL behavior:** Uses the denormalized `face_count` column on the assets table. Pure assets-table filter, no JOIN required.

---

## person

**Syntax:** `person:<name>` or `person:"<multi-word name>"`

**Description:** Filters to assets that contain at least one face assigned to the named person. Supports quoted values for multi-word names.

**Examples:**

```
maki search "person:Alice"
maki search 'person:"John Smith"'
maki search "person:Alice rating:4+"
maki search "-person:Alice"                 # exclude assets with Alice
```

**SQL behavior:** Looks up the person ID by name, then filters via `WHERE EXISTS (SELECT 1 FROM faces WHERE faces.asset_id = a.id AND faces.person_id = ?)`.

---

## similar *(Pro)*

**Syntax:** `similar:<asset-id>` or `similar:<asset-id>:<limit>`

**Description:** Finds visually similar assets using stored SigLIP embeddings. Returns the top N most similar assets (default 20). Requires embeddings to have been generated via `maki embed` or `maki import --embed`. The reference asset ID supports prefix matching (e.g. `similar:abc1` resolves like all other asset ID references).

**Examples:**

```
maki search "similar:72a0bb4b"                          # top 20 similar to this asset
maki search "similar:72a0bb4b:50"                       # top 50 similar
maki search "similar:72a0bb4b rating:3+ tag:landscape"  # similar AND 3+ stars AND landscape tag
maki search -q "similar:72a0bb4b"                       # just IDs, for scripting
```

**Behavior:** Looks up the stored embedding for the reference asset, loads all embeddings into an in-memory index, and performs a dot-product similarity search. If no embedding exists for the reference asset, exits with an error suggesting `maki embed --asset <id>`. The result set can be further filtered by all other search filters (AND logic).

---

## min_sim *(Pro)*

**Syntax:** `min_sim:<percent>`

**Description:** Sets a minimum similarity threshold (0--100) when used with `similar:`. Only assets with similarity at or above this percentage are included. Without `min_sim:`, all results from the similarity search are returned.

**Examples:**

```
maki search "similar:72a0bb4b min_sim:90"          # only >= 90% similar
maki search "similar:72a0bb4b min_sim:80 type:image"  # >= 80% AND images only
```

**Behavior:** The percentage is converted to a 0.0--1.0 cosine similarity threshold internally. Values are clamped to the 0--100 range. Most useful for finding near-duplicates (min_sim:95) or visually very close images (min_sim:85).

---

## embed *(Pro)*

**Syntax:** `embed:any` | `embed:true` | `embed:none` | `embed:false`

**Description:** Filters by whether an asset has a stored AI embedding (SigLIP image embedding). Requires embeddings to have been generated via `maki embed` or `maki import --embed`.

**Examples:**

```
maki search "embed:any"                     # assets with AI embeddings
maki search "embed:true"                    # same as embed:any
maki search "embed:none"                    # assets without AI embeddings
maki search "embed:false"                   # same as embed:none
maki search "embed:none type:image"         # images that still need embeddings
```

**SQL behavior:** Uses an `EXISTS` / `NOT EXISTS` subquery on the `embeddings` table: `WHERE EXISTS (SELECT 1 FROM embeddings e WHERE e.asset_id = a.id)`. Pure subquery, no JOIN required.

---

## text *(Pro)*

**Syntax:** `text:<query>`, `text:"<multi-word query>"`, or `text:"<query>":<limit>`

**Description:** Natural language image search using SigLIP's text encoder. Encodes the query text into the same embedding space as image embeddings, then finds the most similar images via dot-product similarity. Requires embeddings to have been generated via `maki embed` or `maki import --embed`.

The result limit defaults to 50 and can be configured at three levels (highest priority wins):

1. **Inline syntax**: `text:"sunset":100` — per-query override
2. **`[ai] text_limit`** in `maki.toml` — catalog-wide default
3. **Hardcoded fallback**: 50

**Examples:**

```
maki search "text:sunset"                                   # images matching "sunset" (default limit)
maki search "text:\"colorful flowers in a garden\""         # multi-word query
maki search "text:\"person on a beach\" rating:3+"          # combined with other filters
maki search "text:\"mountain landscape\":100"               # return top 100 matches
```

**Behavior:** Loads the SigLIP model (text encoder), encodes the query string into an embedding vector, loads the in-memory embedding index, and returns the top N assets by dot-product similarity (default 50, configurable). The result set can be further filtered by all other search filters (AND logic). Since SigLIP is a vision-language model trained on image-text pairs, queries describe visual content ("red car", "sunset over water", "portrait of a woman") rather than metadata. Results quality depends on how well the SigLIP model generalizes.

**Multilingual queries:** The default model (`siglip-vit-b16-256`) is English-only. For German, French, Spanish, Italian, Japanese, Chinese, and many other languages, switch to the multilingual model:

```toml
[ai]
model = "siglip2-base-256-multi"
```

Then run `maki embed ''` (no `--force` needed — embeddings are stored per `(asset_id, model_id)`, so the new model only embeds assets that don't yet have one for it; existing English-model embeddings are untouched). Once switched, German queries like `text:"Sonnenuntergang am Strand"` or `text:"Hochzeit im Park"` work natively without translation. See [Switching models](../user-guide/02-setup.md#switching-models) in the setup guide for the full workflow and verification commands.

---

## Combining Filters

All filters are combined with AND logic. Every specified filter must match for an asset to appear in results. Free-text terms are also AND-combined with all prefix filters.

**Example combinations:**

```
# 5-star landscape images shot with a Nikon
maki search "rating:5 tag:landscape type:image camera:NIKON"

# High-ISO night shots in RAW format
maki search "iso:3200+ format:nef tag:night"

# Wide-angle portraits with shallow depth of field
maki search "focal:24-35 f:1.4-2.8 tag:portrait"

# Unverified images on the Photos volume, imported from a specific path
maki search "stale:30 type:image path:Capture/2026-01"

# 4K or larger images in a specific collection
maki search 'width:3840+ collection:"Best of 2026"'

# Orphaned video assets (no files on disk)
maki search "orphan:true type:video"

# Everything labeled Red with 4+ stars
maki search "label:Red rating:4+"

# Single-copy RAW files (backup risk)
maki search "copies:1 format:nef"

# Well-backed-up 5-star images
maki search "copies:2+ rating:5 type:image"

# Assets from a specific date range
maki search "dateFrom:2026-01-01 dateUntil:2026-03-31 tag:landscape"

# Everything shot in February 2026
maki search "date:2026-02"

# Unstacked 5-star images (candidates for stacking review)
maki search "stacked:false rating:5 type:image"

# Find stacked assets with a hierarchical tag
maki search "stacked:true tag:animals|birds"

# Visually similar assets, filtered to 4+ stars (Pro + embeddings)
maki search "similar:72a0bb4b rating:4+"

# Geotagged photos within 5km of a location
maki search "geo:52.52,13.405,5 rating:4+"

# All geotagged landscape images
maki search "geo:any tag:landscape type:image"
```

---

## Quoting and Special Characters

### Values with spaces

Use double quotes around filter values that contain spaces. When typing at the shell, wrap the entire query in single quotes to prevent the shell from stripping the inner double quotes.

```bash
# Shell-safe quoting: single quotes outside, double quotes for values
maki search 'tag:"Fools Theater"'
maki search 'camera:"Canon EOS R5" lens:"RF 50mm f/1.2"'
maki search 'collection:"My Favorites" rating:4+'
maki search 'path:"Photos/Family Trip" type:image'
```

Single-word values work without quotes:

```bash
maki search "tag:landscape"
maki search "camera:fuji"
```

### Values with dashes

Without quotes, a leading `-` is interpreted as negation (`-tag:rejected` means "not tagged rejected"). When a dash appears *inside* a value (like a project name), quoting prevents misinterpretation:

```bash
# Wrong: "Geflüchtete" is parsed as negated free-text, not part of the tag
maki search "tag:project/Angekommen im Stadtbild - Geflüchtete im Portait"

# Correct: inner quotes keep the entire value together
maki search 'tag:"project/Angekommen im Stadtbild - Geflüchtete im Portait"'
```

### Hierarchy separators

Tags use `|` as the hierarchy separator. In the shell, `|` is the pipe operator, so always quote tag values containing `|`:

```bash
maki search 'tag:"subject|performing arts|concert"'
maki search 'tag:subject'  # matches all descendants — no | needed
```

### Quick reference

| Character | In filter value | Solution |
|-----------|----------------|----------|
| Space | Splits into multiple tokens | `tag:"golden hour"` |
| `-` | Interpreted as negation | `tag:"my - project"` |
| `\|` | Shell pipe | `'tag:"subject\|nature"'` |
| `"` | Ends quoted value | `tag:"say \"hello\""` (rare) |

---

## Filter Availability

All filters work in the CLI (`maki search`), the web UI search box, and in saved searches. The web UI additionally provides dedicated controls for the most common filters:

| Filter | Web UI control | Saved Searches |
|--------|----------------|:-:|
| Free text | search box | yes |
| `type:` | dropdown + search box | yes |
| `tag:` | tag chips + search box | yes |
| `format:` | multi-select panel + search box | yes |
| `rating:` | star clicks + search box | yes |
| `label:` | color dots + search box | yes |
| `camera:` | search box | yes |
| `lens:` | search box | yes |
| `iso:` | search box | yes |
| `focal:` | search box | yes |
| `f:` | search box | yes |
| `width:` | search box | yes |
| `height:` | search box | yes |
| `meta:` | search box | yes |
| `path:` | text input + search box | yes |
| `collection:` | dropdown + search box | yes |
| `volume:` | dropdown + search box | yes |
| `copies:` | search box | yes |
| `variants:` | search box | yes |
| `scattered:` | search box | yes |
| `date:` | search box | yes |
| `dateFrom:` | search box | yes |
| `dateUntil:` | search box | yes |
| `id:` | search box | yes |
| `orphan:` | search box | yes |
| `missing:true` | search box | yes |
| `stale:` | search box | yes |
| `stacked:` | search box | yes |
| `geo:` | search box | yes |
| `faces:` | search box | yes |
| `person:` | dropdown + search box | yes |
| `similar:` *(Pro)* | "Browse similar" button + search box | no |
| `min_sim:` *(Pro)* | search box | no |
| `text:` *(Pro)* | search box | yes |
| `embed:` *(Pro)* | search box | yes |

---

## Related Topics

- [Browse & Search (User Guide)](../user-guide/05-browse-and-search.md) -- practical search workflows and output formatting
- [Format Templates Reference](07-format-templates.md) -- controlling output with `--format` and custom templates
- [Configuration Reference](08-configuration.md) -- `maki.toml` settings
