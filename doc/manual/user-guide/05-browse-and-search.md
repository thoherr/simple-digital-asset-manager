# Browsing & Searching

This chapter covers how to find, inspect, and analyze assets in your catalog using the CLI. For the web-based browser interface, see [Web UI](06-web-ui.md).

---

## Searching Assets

The `dam search` command is the primary way to find assets. It accepts free-text keywords and structured filters in a single query string.

**Free-text search** matches against asset names, filenames, descriptions, and source metadata:

```
dam search "sunset"
dam search "beach vacation"
```

**Structured filters** use a `key:value` syntax and can be combined freely with free-text:

```
dam search "tag:landscape type:image rating:3+"
dam search "sunset tag:landscape camera:fuji iso:100-800"
dam search "format:nef focal:35-70 f:1.4-2.8"
```

Tokens that don't match a known filter prefix are treated as free-text, so you can mix them in any order:

```
dam search "type:image sunset rating:4+ golden hour"
```

**Values with spaces** are supported using double quotes inside the query:

```
dam search 'tag:"Fools Theater"'
dam search 'camera:"Canon EOS R5" lens:"RF 50mm f/1.2"'
dam search 'collection:"My Favorites"'
dam search 'path:"Photos/Family Trip"'
```

Note the outer single quotes to prevent your shell from stripping the inner double quotes.

**Negation** excludes matching assets by prefixing any filter or free-text term with `-`:

```
dam search "-tag:rejected"               # exclude rejected assets
dam search "landscape -tag:processed"    # landscapes not yet processed
dam search "-format:xmp -type:other"     # exclude XMP files and "other" types
dam search "-sunset"                     # exclude free-text match on "sunset"
```

**OR within a filter** matches any of several values using commas:

```
dam search "tag:alice,bob"               # tagged alice OR bob (or both)
dam search "format:nef,cr3"              # NEF or CR3 format
dam search "type:image,video"            # images or videos
dam search "label:Red,Orange"            # Red or Orange labeled
```

**Repeated filters = AND**. To require multiple tags, repeat the filter:

```
dam search "tag:landscape tag:sunset"    # BOTH landscape AND sunset tags
```

---

## Search Filters Quick Reference

All filters can be combined in a single query. Remaining tokens become free-text search terms.

| Filter | Syntax | Example |
|--------|--------|---------|
| Asset type | `type:<type>` | `type:image`, `type:video` |
| Tag | `tag:<name>` | `tag:landscape`, `tag:"Fools Theater"` |
| Format | `format:<ext>` | `format:nef`, `format:jpg` |
| Rating (exact) | `rating:<N>` | `rating:5` |
| Rating (minimum) | `rating:<N>+` | `rating:3+` |
| Color label | `label:<color>` | `label:Red`, `label:Blue` |
| Camera | `camera:<text>` | `camera:fuji`, `camera:"Canon EOS R5"` |
| Lens | `lens:<text>` | `lens:56mm`, `lens:"RF 50mm f/1.2"` |
| ISO (exact or range) | `iso:<N>` or `iso:<min>-<max>` | `iso:100`, `iso:100-800` |
| Focal length | `focal:<N>` or `focal:<min>-<max>` | `focal:50`, `focal:35-70` |
| Aperture | `f:<N>` or `f:<min>-<max>` | `f:2.8`, `f:1.4-2.8` |
| Width (minimum) | `width:<N>+` | `width:4000+` |
| Height (minimum) | `height:<N>+` | `height:2000+` |
| Source metadata | `meta:<key>=<value>` | `meta:software=CaptureOne` |
| Path prefix | `path:<prefix>` | `path:Capture/2026-02-22` |
| Collection | `collection:<name>` | `collection:Favorites` |
| Date (prefix match) | `date:<prefix>` | `date:2026-02-25`, `date:2026-02`, `date:2026` |
| Date (from) | `dateFrom:<date>` | `dateFrom:2026-01-01` |
| Date (until) | `dateUntil:<date>` | `dateUntil:2026-12-31` |
| Volume | `volume:none` | `volume:none` |
| Orphan assets | `orphan:true` | `orphan:true` |
| Missing files | `missing:true` | `missing:true` |
| Stale verification | `stale:<days>` | `stale:30` |
| Stacked | `stacked:true` or `stacked:false` | `stacked:true` |
| Face count | `faces:any`, `faces:none`, `faces:N`, `faces:N+` | `faces:2+` |
| Person | `person:<name>` | `person:Alice`, `person:"John Smith"` |

**Hierarchical tag matching**: The `tag:` filter matches hierarchically. Searching for `tag:animals` finds assets tagged `animals`, `animals/birds`, `animals/birds/eagles`, and any other descendant of `animals`. To match only the exact tag, use the full path (e.g., `tag:animals/birds/eagles`).

For the complete filter reference with detailed syntax and behavior, see [Search Filters](../reference/06-search-filters.md).

---

## Path Prefix Filter

The `path:` filter restricts results to assets whose file locations start with a given prefix. This is useful for finding everything imported from a particular directory:

```
dam search "path:Capture/2026-02-22"
dam search "path:Events/Wedding"
```

**Automatic path normalization** in the CLI handles several convenience cases:

- `~` expands to your home directory
- `./` and `../` resolve relative to your current working directory
- Absolute paths that match a registered volume's mount point are automatically stripped to volume-relative, and the corresponding volume filter is applied

For example, if you have a volume mounted at `/Volumes/Photos`:

```
dam search "path:/Volumes/Photos/Capture/2026"
# Equivalent to: dam search "path:Capture/2026" on the Photos volume
```

An explicit `volume:` filter in the same query takes precedence over the auto-detected volume.

---

## Output Formats

Control how search results are displayed with `--format` or `-q`.

### Preset Formats

**Default (short)** -- one line per result with a result count:

```
$ dam search "sunset"
a1b2c3d4  IMG_1234.jpg [image] (JPEG) — 2026-01-15T10:30:00
e5f6a7b8  DSC_5678.nef [image] (NEF) — 2026-01-14T16:45:00

2 result(s)
```

**IDs only** (`--format ids` or `-q`) -- one UUID per line, ideal for scripting:

```
$ dam search -q "tag:landscape"
a1b2c3d4-5678-9abc-def0-123456789abc
e5f6a7b8-1234-5678-9abc-def012345678
```

**Full** (`--format full`) -- includes tags and description:

```
$ dam search "sunset" --format full
a1b2c3d4  IMG_1234.jpg [image] (JPEG) — 2026-01-15T10:30:00 tags:sunset,landscape A golden sunset over the mountains
```

**JSON** (`--format json` or `--json`) -- structured JSON array:

```
$ dam search "sunset" --json
[
  {
    "asset_id": "a1b2c3d4-5678-9abc-def0-123456789abc",
    "original_filename": "IMG_1234.jpg",
    "asset_type": "image",
    ...
  }
]
```

### Custom Templates

Build your own output format using `{placeholder}` syntax:

```
dam search "sunset" --format '{id}\t{name}\t{tags}'
dam search "type:image" --format '{short_id} {filename} [{format}] {label}'
dam search "rating:5" --format '{name}\n  {description}\n'
```

**Available placeholders:**

| Placeholder | Content |
|-------------|---------|
| `{id}` | Full asset UUID |
| `{short_id}` | First 8 characters of the UUID |
| `{name}` | Asset name (falls back to filename) |
| `{filename}` | Original filename |
| `{type}` | Asset type (image, video, audio, etc.) |
| `{format}` | File format (JPEG, NEF, MOV, etc.) |
| `{date}` | Creation date |
| `{tags}` | Comma-separated tags |
| `{description}` | Asset description |
| `{hash}` | Content hash of the best variant |
| `{label}` | Color label (Red, Blue, etc.) |

**Escape sequences:** `\t` (tab), `\n` (newline), `\\` (literal backslash).

When `--format` is explicitly provided, the result count line at the end is suppressed, keeping the output clean for piping and parsing.

For the complete format reference, see [Format Templates](../reference/07-format-templates.md).

---

## Sort Options

The web UI provides inline sort toggle buttons (Name, Date, Size) with ascending/descending direction indicators. In the CLI, sort order is available when saving searches:

```
dam saved-search save "landscapes" "tag:landscape" --sort name_asc
```

Available sort values:

| Sort | Description |
|------|-------------|
| `date_desc` | Newest first (default) |
| `date_asc` | Oldest first |
| `name_asc` | Alphabetical A-Z (by name, falling back to filename) |
| `name_desc` | Alphabetical Z-A |
| `size_asc` | Smallest first |
| `size_desc` | Largest first |

The default sort for `dam search` is `date_desc` (newest first).

---

## Scripting with dam

The `-q` flag and format options make `dam` composable with standard Unix tools.

**Loop over search results:**

```bash
for id in $(dam search -q "tag:landscape"); do
    dam show "$id"
done
```

**Pipe results into collection commands:**

```bash
dam search -q "rating:5" | xargs dam col add "Best"
```

**Extract specific fields with jq:**

```bash
dam search "sunset" --json | jq '.[].asset_id'
dam search "type:video" --json | jq '.[] | {name: .original_filename, date: .created_at}'
```

**Count results:**

```bash
dam search -q "format:nef" | wc -l
```

**Tab-separated output for spreadsheets:**

```bash
dam search "type:image" --format '{name}\t{format}\t{date}\t{tags}'
```

**Combine with other dam commands:**

```bash
# Tag all 5-star images as "portfolio"
dam search -q "rating:5 type:image" | xargs -I{} dam tag {} portfolio

# Generate previews for a specific path
dam search -q "path:Capture/2026-02" | xargs -I{} dam generate-previews --asset {}
```

---

## Showing Asset Details

The `dam show` command displays comprehensive information about a single asset.

```
dam show a1b2c3d4
```

Asset ID prefix matching is supported -- you only need to type enough characters to uniquely identify the asset:

```
dam show a1b2     # Works if "a1b2" is a unique prefix
```

### Output

The human-readable output includes:

```
Asset: a1b2c3d4-5678-9abc-def0-123456789abc
Name:  Golden Sunset
Type:  image
Date:  2026-01-15T10:30:00
Tags:  sunset, landscape, mountains
Rating: ★★★★★ (5/5)
Label: Red
Description: A golden sunset over the mountain range
Preview: /path/to/catalog/previews/a1/a1b2c3d4...jpg

Variants:
  [original] DSC_1234.NEF (NEF, 48.2 MB)
    Hash: abc123def456...
    Location: Photos → Capture/2026-01-15/DSC_1234.NEF
    camera_make: NIKON CORPORATION
    camera_model: NIKON Z 9
    focal_length: 70.0
    f_number: 8.0
    iso: 200
  [export] DSC_1234-Edit.tif (TIFF, 120.5 MB)
    Hash: def789abc012...
    Location: Photos → Export/2026-01-15/DSC_1234-Edit.tif

Recipes:
  [xmp] CaptureOne (e4f5a6b7c8...)
    Path: Capture/2026-01-15/DSC_1234.NEF.xmp
```

For structured output, use the `--json` flag:

```
dam show a1b2c3d4 --json
```

This returns a full `AssetDetails` JSON object with all variants, locations, recipes, and source metadata.

---

## Finding Duplicates

The `dam duplicates` command identifies files with identical content hashes that exist in multiple locations.

```
$ dam duplicates
DSC_1234.NEF (NEF, 48.2 MB)
  Hash: abc123def456...
    Photos → Capture/2026-01-15/DSC_1234.NEF
    Backup → Capture/2026-01-15/DSC_1234.NEF

1 file(s) with duplicate locations
```

This is useful for identifying files that have been copied to multiple volumes (e.g., when migrating data or creating backups).

### Duplicate Output Formats

The `--format` flag supports the same presets as search, plus an additional `{locations}` placeholder for templates:

```
dam duplicates --format ids
dam duplicates --format json
dam duplicates --format '{filename}\t{format}\t{locations}'
```

The `{locations}` placeholder expands to a comma-separated list of all locations where the duplicate file exists.

---

## Catalog Statistics

The `dam stats` command provides an overview of your catalog's contents and health.

### Overview (Default)

```
$ dam stats
Catalog Overview
  Assets:    12,847
  Variants:  18,203
  Recipes:   9,614
  Volumes:   3 (2 online, 1 offline)
  Total size: 1.8 TB
```

### Asset Types and Formats

```
$ dam stats --types
Catalog Overview
  ...

Asset Types
  image         11,204  (87.2%)
  video          1,412  (11.0%)
  audio            231  (1.8%)

Variant Formats
  NEF            8,100
  JPEG           6,543
  TIFF           2,104
  MOV            1,412

Recipe Formats
  xmp            7,200
  cos            2,414
```

### Per-Volume Details

```
$ dam stats --volumes
Catalog Overview
  ...

Volumes
  Photos [online]
    Assets: 10,234  Variants: 14,567  Recipes: 7,890
    Size: 1.2 TB  Directories: 342
    Formats: NEF, JPEG, TIFF, MOV
    Verified: 14,200/14,567 (97.5%)
    Oldest verification: 2026-01-01T08:00:00
  Backup [offline]
    Assets: 8,412  Variants: 12,100  Recipes: 5,200
    Size: 980.4 GB  Directories: 298
    Formats: NEF, JPEG, TIFF
    Verified: 12,100/12,100 (100.0%)
    Oldest verification: 2025-12-15T14:30:00
```

### Tag Statistics

```
$ dam stats --tags
Tags
  Unique tags:     145
  Tagged assets:   9,876
  Untagged assets: 2,971

  Top Tags
    landscape               1,234
    portrait                  987
    street                    654
    ...
```

### Verification Health

```
$ dam stats --verified
Verification
  Total locations:    26,667
  Verified:           26,300
  Unverified:            367
  Coverage:           98.6%
  Oldest verified:    2025-12-15T14:30:00
  Newest verified:    2026-02-22T19:00:00

  Per Volume
    Photos [online]: 14,200/14,567 (97.5%)
    Backup [offline]: 12,100/12,100 (100.0%)
```

### Combined and Tuned Output

Use `--all` to show every section at once:

```
dam stats --all
```

Control the number of entries in top-N lists (default is 20):

```
dam stats --tags --limit 50
```

For structured output:

```
dam stats --all --json
```

---

## Exporting Files

The `dam export` command copies files matching a search query to a target directory — useful for client deliveries, sharing, or copying to external media.

### Basic Export

Export the best variant of each matching asset:

```
dam export "rating:5 tag:portfolio" /tmp/delivery/
```

### Layout Modes

**Flat** (default) — all files in one directory:

```
dam export "collection:Selects" /tmp/flat/
```

Filename collisions from different assets are resolved by appending a hash suffix (e.g., `DSC_001_a1b2c3d4.jpg`).

**Mirror** — preserves source directory structure:

```
dam export "tag:landscape" /Volumes/USB/export --layout mirror
```

When assets span multiple volumes, each volume's files are placed under a `<volume-label>/` prefix.

### Options

Export all variants (not just the best):

```
dam export "tag:portfolio" /tmp/all/ --all-variants
```

Include sidecars (`.xmp`, `.cos`, etc.):

```
dam export "collection:Print" /tmp/handoff/ --include-sidecars
```

Create symlinks instead of copies:

```
dam export "type:image" ~/links/ --symlink
```

Preview without writing files:

```
dam export "rating:4+" /tmp/test/ --dry-run
```

Re-running an export skips files that already exist with matching content. Use `--overwrite` to force re-copy.

For the full command reference, see [export](../reference/04-retrieve-commands.md#dam-export).

---

## Related Topics

- [Organizing Assets](04-organize.md) -- tags, editing, collections, and saved searches
- [Web UI](06-web-ui.md) -- browser-based search with interactive filters, sort controls, and batch operations
- [Search Filters Reference](../reference/06-search-filters.md) -- complete filter syntax documentation
- [Format Templates Reference](../reference/07-format-templates.md) -- all placeholders and template syntax
- [CLI Conventions](../reference/00-cli-conventions.md) -- global flags (`--json`, `--log`, `--time`)
