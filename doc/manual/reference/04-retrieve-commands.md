# Retrieve Commands

Commands for finding assets, inspecting details, and browsing the catalog.

---

## dam search

### NAME

dam-search -- search for assets using filters and free-text keywords

### SYNOPSIS

```
dam [GLOBAL FLAGS] search <QUERY> [--format <FMT>] [-q]
```

### DESCRIPTION

Searches the catalog for assets matching the given query. The query string supports both free-text keywords (matched against filenames and metadata) and structured filter prefixes.

**Filter syntax**: `prefix:value`. Values containing spaces must be double-quoted: `tag:"Fools Theater"`.

**Available filters**:

| Filter | Description | Examples |
|--------|-------------|----------|
| `type:` | Asset type | `type:image`, `type:video`, `type:audio` |
| `tag:` | Tag name (hierarchical: matches descendants) | `tag:landscape`, `tag:"golden hour"`, `tag:animals/birds` |
| `format:` | File format | `format:jpg`, `format:nef`, `format:mp4` |
| `rating:` | Star rating (exact) | `rating:5` |
| `rating:N+` | Star rating (minimum) | `rating:3+` |
| `label:` | Color label | `label:Red`, `label:Green` |
| `camera:` | Camera model (partial match) | `camera:fuji`, `camera:"Canon EOS R5"` |
| `lens:` | Lens model (partial match) | `lens:56mm`, `lens:"RF 50mm f/1.2"` |
| `iso:` | ISO value or range | `iso:100`, `iso:100-800` |
| `focal:` | Focal length or range (mm) | `focal:50`, `focal:35-70` |
| `f:` | Aperture or range | `f:1.4`, `f:1.4-2.8` |
| `width:` | Minimum width (pixels) | `width:4000`, `width:4000+` |
| `height:` | Minimum height (pixels) | `height:2000`, `height:2000+` |
| `meta:` | Source metadata key=value | `meta:Copyright=2026` |
| `path:` | File location path prefix | `path:Capture/2026-02`, `path:/Volumes/Photos/2026` |
| `collection:` | Collection membership | `collection:Favorites`, `collection:"My Picks"` |
| `date:` | Creation date (prefix match) | `date:2026-02-25`, `date:2026-02`, `date:2026` |
| `dateFrom:` | Creation date lower bound (inclusive) | `dateFrom:2026-01-01` |
| `dateUntil:` | Creation date upper bound (inclusive) | `dateUntil:2026-12-31` |
| `copies:` | File location count (exact) | `copies:1`, `copies:2` |
| `copies:N+` | File location count (minimum) | `copies:2+`, `copies:3+` |
| `orphan:true` | Assets with zero file locations | `orphan:true` |
| `missing:true` | Assets with files missing on disk | `missing:true` |
| `stale:N` | Not verified in N days | `stale:30`, `stale:90` |
| `volume:none` | Assets not on any online volume | `volume:none` |
| `stacked:true` | Assets in a stack | `stacked:true` |
| `stacked:false` | Assets not in any stack | `stacked:false` |

Filters can be freely combined. Free-text tokens that do not match a filter prefix are joined as a text search against filenames and metadata.

**Path normalization**: The `path:` filter automatically normalizes paths. `~` expands to `$HOME`, `./` and `../` resolve relative to the current working directory, and absolute paths matching a registered volume's mount point are stripped to volume-relative form with the volume filter implicitly applied (e.g., `path:/Volumes/Photos/Capture/2026` becomes `path:Capture/2026` scoped to the Photos volume).

**Output formats**: The `--format` flag controls output. Presets: `ids` (one UUID per line), `short` (default: abbreviated ID, name, type, format, date), `full` (adds tags, description, rating, label), `json` (JSON array). Custom templates use `{placeholder}` substitution with escape sequences (`\t`, `\n`).

Available placeholders: `{id}`, `{name}`, `{filename}`, `{type}`, `{format}`, `{tags}`, `{description}`, `{rating}`, `{label}`, `{date}`, `{size}`.

The result count header (e.g., "Found 42 assets") is suppressed when an explicit `--format` is given.

### ARGUMENTS

**QUERY** (required)
: Search query string with optional filter prefixes and free-text keywords.

### OPTIONS

**--format \<FMT\>**
: Output format preset or custom template.

**-q** / **--quiet**
: Shorthand for `--format=ids`. Prints one asset ID per line with no header, ideal for piping.

`--json` (global flag) is equivalent to `--format json`.

### EXAMPLES

Search by tag and minimum rating:

```bash
dam search "tag:landscape rating:4+"
```

Find all videos:

```bash
dam search "type:video"
```

Search with camera and aperture filters:

```bash
dam search 'camera:"Canon EOS R5" f:1.4-2.8'
```

Get IDs for scripting:

```bash
dam search -q "tag:travel label:Green"
```

Custom format template:

```bash
dam search "rating:5" --format '{id}\t{name}\t{label}'
```

Search within a path and pipe to a collection:

```bash
dam search -q "path:Capture/2026-02-22 rating:4+" | xargs dam col add "Feb Selects"
```

Find assets with files missing on disk:

```bash
dam search "missing:true"
```

Find assets with no backup (only one copy):

```bash
dam search "copies:1"
```

Find highly-rated assets with at least two copies:

```bash
dam search "copies:2+ rating:4+"
```

Find orphaned assets (no file locations):

```bash
dam search "orphan:true"
```

### SEE ALSO

[show](#dam-show) -- display full details for a specific asset.
[saved-search](03-organize-commands.md#dam-saved-search-save) -- save and re-run searches.
[CLI Conventions](00-cli-conventions.md) -- output conventions, scripting patterns.

---

## dam show

### NAME

dam-show -- display full details for an asset

### SYNOPSIS

```
dam [GLOBAL FLAGS] show <ASSET_ID>
```

### DESCRIPTION

Displays comprehensive details for a single asset, including:

- Asset metadata: ID, name, description, type, tags, rating (as stars), color label, creation date.
- Variants: content hash, role (original/export/processed/sidecar), format, file size, original filename, file locations (volume + path), source metadata (camera, lens, ISO, etc.), and preview status.
- Recipes: content hash, format (XMP, COS, etc.), original filename, and file locations.

Asset IDs support unique prefix matching (see [CLI Conventions](00-cli-conventions.md#asset-id-matching)).

The display logic for previews prefers Export > Processed > Original variant previews (skipping Sidecar). Within the same role, standard image formats are preferred over RAW, with file size as a tiebreaker.

### ARGUMENTS

**ASSET_ID** (required)
: The asset ID or a unique prefix of it.

### OPTIONS

This command only accepts [global flags](00-cli-conventions.md#global-flags).

`--json` outputs an `AssetDetails` object with full asset, variant, and recipe information.

### EXAMPLES

Show an asset by full ID:

```bash
dam show a1b2c3d4-e5f6-7890-abcd-ef1234567890
```

Show an asset by prefix:

```bash
dam show a1b2c
```

Show as JSON and extract variant filenames:

```bash
dam show a1b2c --json | jq '.variants[].original_filename'
```

Show as JSON and list file locations:

```bash
dam show a1b2c --json | jq '.variants[].file_locations[]'
```

### SEE ALSO

[search](#dam-search) -- find assets to inspect.
[edit](02-ingest-commands.md#dam-edit) -- modify the fields shown here.
[tag](02-ingest-commands.md#dam-tag) -- add or remove tags.

---

## dam export

### NAME

dam-export -- copy files matching a search query to a directory

### SYNOPSIS

```
dam [GLOBAL FLAGS] export <QUERY> <TARGET> [OPTIONS]
```

### DESCRIPTION

Exports files from the catalog to a target directory. Searches for assets matching the query, resolves their file locations on online volumes, and copies (or symlinks) files to the target.

By default, only the **best variant** per asset is exported (Export > Processed > Original, image formats preferred, file size tiebreaker). Use `--all-variants` to export every variant.

**Layout modes**:

- **`flat`** (default): All files placed in the target root. Filename collisions (different content, same filename) are resolved by appending `_<8-char-hash>` before the extension. Files with identical content hashes reuse the same target path.
- **`mirror`**: Preserves the source volume-relative directory structure. When files span multiple volumes, each volume's files are placed under a `<volume-label>/` prefix to avoid path collisions.

Files are copied with SHA-256 integrity verification. Existing files at the target path are skipped if their content hash matches (use `--overwrite` to force re-copy).

### ARGUMENTS

**QUERY** (required)
: Search query string (same syntax as `dam search`).

**TARGET** (required)
: Target directory path. Created automatically if it does not exist (except in `--dry-run` mode).

### OPTIONS

**--layout \<MODE\>**
: File layout mode: `flat` (default) or `mirror`.

**--symlink**
: Create symbolic links instead of copying files. On Unix, uses `std::os::unix::fs::symlink`; on Windows, uses `std::os::windows::fs::symlink_file`.

**--all-variants**
: Export every variant of each matching asset. Default is best variant only.

**--include-sidecars**
: Also copy recipe/sidecar files (`.xmp`, `.cos`, `.cot`, `.cop`, `.pp3`, `.dop`, `.on1`) alongside the exported variants.

**--dry-run**
: Report the export plan without writing any files or creating directories.

**--overwrite**
: Re-copy files even if the target already contains a file with a matching content hash. Default behavior skips matching files.

`--json` outputs an `ExportResult` object with fields: `dry_run`, `assets_matched`, `files_exported`, `files_skipped`, `sidecars_exported`, `total_bytes`, `errors`.

### EXAMPLES

Export best-of picks to a delivery folder:

```bash
dam export "rating:5 tag:portfolio" /tmp/delivery/
```

Export with directory structure preserved:

```bash
dam export "collection:Print" /Volumes/USB/export --layout mirror
```

Include sidecars for another workstation:

```bash
dam export "tag:client" /tmp/handoff/ --include-sidecars
```

Create symlinks instead of copies:

```bash
dam export "type:image rating:4+" ~/links/ --symlink
```

Export all variants (RAW + processed):

```bash
dam export "tag:portfolio" /tmp/all/ --all-variants
```

Dry run to see what would be exported:

```bash
dam export "collection:Best" /tmp/test/ --dry-run
```

JSON output for scripting:

```bash
dam --json export "rating:5" /tmp/out/ | jq '.files_exported'
```

### SEE ALSO

[search](#dam-search) -- find assets to export.
[relocate](05-maintain-commands.md#dam-relocate) -- move/copy asset files between volumes with catalog updates.
[CLI Conventions](00-cli-conventions.md) -- global flags, scripting patterns.

---

## dam duplicates

### NAME

dam-duplicates -- find files with identical content at multiple locations

### SYNOPSIS

```
dam [GLOBAL FLAGS] duplicates [--format <FMT>] [--same-volume] [--cross-volume] [--volume <LABEL>] [--filter-format <FMT>] [--path <PREFIX>]
```

### DESCRIPTION

Finds variants whose content hash appears at more than one file location. This detects files that exist on multiple volumes or at multiple paths, helping identify redundant copies or verify backup coverage.

Each result shows the content hash, filename, volume count, and all locations where the identical file exists. The `short` and `full` formats annotate locations with the volume's purpose (if set) and flag same-volume duplicate groups. The `full` format additionally shows the last verification timestamp for each location.

**Duplicate modes**:

- **Default** (no flag): All variants with 2+ file locations, regardless of volume layout.
- **`--same-volume`**: Only variants with 2+ locations on the **same** volume. These are likely unwanted copies (e.g., accidentally imported twice from different paths on the same drive).
- **`--cross-volume`**: Only variants with locations on 2+ **different** volumes. These represent intentional backups or copies across drives.

The `--volume`, `--filter-format`, and `--path` flags narrow results via SQL filtering. All filters compose with any mode.

### ARGUMENTS

None.

### OPTIONS

**--format \<FMT\>**
: Output format. Same presets as `dam search`: `ids`, `short` (default), `full`, `json`. Custom templates support all search placeholders plus `{locations}` and `{volumes}` (distinct volume count). Location strings include the volume purpose in brackets (e.g., `Photos[working]:Capture/photo.jpg`).

**--same-volume**
: Show only same-volume duplicates. Mutually exclusive with `--cross-volume`.

**--cross-volume**
: Show only cross-volume copies. Mutually exclusive with `--same-volume`.

**--volume \<LABEL\>**
: Filter results to entries involving this volume. Combines with any mode.

**--filter-format \<FORMAT\>**
: Filter to entries matching this file format (e.g. `nef`, `jpg`).

**--path \<PREFIX\>**
: Filter to entries with a location under this path prefix.

`--json` outputs an array of `DuplicateEntry` objects (includes `volume_count` and `same_volume_groups` fields).

### EXAMPLES

Find all duplicates:

```bash
dam duplicates
```

Find likely unwanted same-volume duplicates:

```bash
dam duplicates --same-volume
```

Show cross-volume copies (backup verification):

```bash
dam duplicates --cross-volume
```

Filter to duplicates involving a specific volume:

```bash
dam duplicates --volume "Backup Drive"
```

Show full details with verification timestamps:

```bash
dam duplicates --format full
```

List duplicates as JSON:

```bash
dam duplicates --json | jq '.[].locations'
```

Cross-volume copies as JSON for a specific volume:

```bash
dam duplicates --cross-volume --volume Photos --json
```

Custom format showing hash and locations:

```bash
dam duplicates --format '{hash}\t{filename}\t{volumes} volumes\t{locations}'
```

### SEE ALSO

[verify](05-maintain-commands.md#dam-verify) -- verify file integrity on disk.
[cleanup](05-maintain-commands.md#dam-cleanup) -- remove stale location records.
[search](#dam-search) -- use `copies:` filter for location-count-based queries.

---

## dam stats

### NAME

dam-stats -- show catalog statistics

### SYNOPSIS

```
dam [GLOBAL FLAGS] stats [OPTIONS]
```

### DESCRIPTION

Displays summary statistics about the catalog. Without any section flags, shows a compact overview: total assets, variants, recipes, volumes, and total file size.

Additional sections can be enabled with flags to show breakdowns by type, format, volume, tag usage, and verification health. `--all` enables all sections.

### ARGUMENTS

None.

### OPTIONS

**--types**
: Show asset type breakdown (image, video, audio, etc.) and format distribution.

**--volumes**
: Show per-volume details: asset count, total size, directory count, and verification status.

**--tags**
: Show tag usage frequencies (most-used tags first).

**--verified**
: Show verification health: how many files have been verified, when, and how many are overdue.

**--all**
: Enable all sections (equivalent to `--types --volumes --tags --verified`).

**--limit \<N\>**
: Maximum number of entries for top-N lists (default: 20). Applies to format breakdown, tag list, etc.

`--json` outputs a structured JSON object with all requested sections.

### EXAMPLES

Quick overview:

```bash
dam stats
```

Full statistics:

```bash
dam stats --all
```

Show only tag frequencies:

```bash
dam stats --tags --limit 50
```

Show volume details as JSON:

```bash
dam stats --volumes --json | jq '.volumes[] | {label, assets, size}'
```

Show verification health:

```bash
dam stats --verified
```

### SEE ALSO

[search](#dam-search) -- find specific assets matching criteria.
[verify](05-maintain-commands.md#dam-verify) -- run verification checks.
[volume list](01-setup-commands.md#dam-volume-list) -- list volumes with online/offline status.

---

## dam backup-status

### NAME

dam-backup-status -- check backup coverage and find under-backed-up assets

### SYNOPSIS

```
dam [GLOBAL FLAGS] backup-status [QUERY] [OPTIONS]
```

### DESCRIPTION

Answers the question: "Are my important assets safely backed up?"

In **overview mode** (default), displays aggregate statistics about backup coverage:

- **Totals**: asset count, variant count, file location count.
- **Coverage by volume purpose**: how many assets exist on volumes of each purpose (Working, Archive, Backup, Cloud), with percentages.
- **Volume distribution**: histogram of assets by number of distinct volumes they exist on (0, 1, 2, 3+), with "AT RISK" markers for assets below the threshold. Multiple variants or locations on the same volume count as one — what matters for backup safety is how many distinct volumes hold the asset.
- **At-risk summary**: count of assets with fewer than `--min-copies` locations, with hints for listing them.
- **Volume gaps**: per-volume count of missing assets (assets in scope but not on that volume).

In **at-risk listing mode** (`--at-risk`, `-q`, or `--format`), outputs a list of under-backed-up assets using the same output formats as `dam search`. When `--volume` is specified, lists assets missing from that specific volume instead of those with fewer than `--min-copies` locations overall.

An optional positional `QUERY` argument scopes the analysis to matching assets (same syntax as `dam search`).

### ARGUMENTS

**QUERY** (optional)
: Search query to scope the asset universe. Same syntax as `dam search`. When omitted, all catalog assets are analyzed.

### OPTIONS

**--at-risk**
: Switch to listing mode. Output under-backed-up assets instead of the overview.

**--min-copies \<N\>**
: Threshold for "adequately backed up" (default: 2). Assets on fewer than N distinct volumes are considered at-risk.

**--volume \<LABEL\>**
: In overview mode, adds a detailed volume coverage section for this volume. In at-risk listing mode, lists assets missing from this specific volume.

**--format \<FMT\>**
: Output format for at-risk listings. Same presets as `dam search`: `ids`, `short`, `full`, `json`, or a custom template.

**-q** / **--quiet**
: Shorthand for `--format=ids`. Prints one asset ID per line, ideal for piping to other commands.

`--json` (global flag) outputs a structured `BackupStatusResult` object in overview mode, or a JSON array of `SearchRow` objects in at-risk listing mode.

### EXAMPLES

Quick overview of backup coverage:

```bash
dam backup-status
```

Scope to highly-rated images:

```bash
dam backup-status "rating:3+ type:image"
```

Require 3 copies and check coverage:

```bash
dam backup-status --min-copies 3
```

List at-risk asset IDs for scripting:

```bash
dam backup-status --at-risk -q
```

Find assets missing from a specific volume:

```bash
dam backup-status --volume "Master Media" --at-risk -q
```

Pipe at-risk assets to relocate:

```bash
dam backup-status --volume "Master Media" --at-risk -q "rating:3+" \
  | xargs -I{} dam relocate {} "Master Media"
```

Add at-risk assets to a collection for review:

```bash
dam backup-status --at-risk -q | xargs dam collection add "Needs Backup"
```

JSON output for scripting:

```bash
dam --json backup-status | jq '.at_risk_count'
```

### SEE ALSO

[search](#dam-search) -- use `copies:` filter for location-count-based queries.
[duplicates](#dam-duplicates) -- find duplicate files across volumes.
[stats](#dam-stats) -- general catalog statistics.
[verify](05-maintain-commands.md#dam-verify) -- verify file integrity.

---

## dam serve

### NAME

dam-serve -- start the web UI server

### SYNOPSIS

```
dam [GLOBAL FLAGS] serve [--port <PORT>] [--bind <ADDR>]
```

### DESCRIPTION

Starts a local web server that provides a browser-based interface for browsing, searching, and editing assets. The web UI offers a rich interactive experience including:

- **Browse page**: Grid of asset thumbnails with a two-row search bar. Row 1: full-width text input. Row 2: tag filter, star rating filter, color label dots, type/format/volume/collection dropdowns, and path prefix input. All filters auto-search with 300ms debounce on text inputs and immediate trigger on dropdowns, stars, labels, and tags.
- **Asset detail page**: Full preview, editable metadata (inline name, description, star rating, color label, tags), variant list, recipe list, and collection membership chips.
- **Tags page** (`/tags`): Sortable tag list with counts, live text filter, and multi-column layout.
- **Collections page** (`/collections`): List of all collections with creation button.
- **Stacks**: When stack collapsing is enabled (default), stacked assets are collapsed in the browse grid to show only the pick image with a stack count badge. A toggle button in the results bar switches between collapsed and expanded views. The `stacked:true` and `stacked:false` search filters are available in the query input.
- **Batch operations**: Checkbox selection on browse cards with a fixed bottom toolbar for batch tagging, rating, labeling, auto-grouping, stacking, and unstacking. "Stack" creates a new stack from selected assets; "Unstack" removes selected assets from their stacks.
- **Keyboard navigation**: Arrow keys navigate between cards, Enter opens details, Space toggles selection, 1-5 sets rating, 0 clears rating, Alt+1-7 sets label, single letters (r/o/y/g/b/p/u) set label by color initial, x clears label.
- **Saved search chips**: Clickable chips on the browse page load saved searches into the filter UI.
- **Tags page**: Shows a collapsible tree view for hierarchical tags (tags containing `/` as a hierarchy separator). Non-hierarchical tags continue to display in the flat multi-column layout.

The server defaults to `127.0.0.1:8080`. These can be overridden by CLI flags or the `[serve]` section in `dam.toml`. CLI flags take precedence over configuration.

SQLite connections are opened per-request. Previews are served as static files. Static assets (htmx.min.js, style.css) are embedded at compile time.

### ARGUMENTS

None.

### OPTIONS

**--port \<PORT\>**
: Port to listen on. Default: 8080, or the value from `dam.toml` `[serve]` section.

**--bind \<ADDR\>**
: Address to bind to. Default: `127.0.0.1`, or the value from `dam.toml` `[serve]` section.

`--log` (global flag) enables per-request logging to stderr in the format `METHOD /path -> STATUS (duration)`.

### EXAMPLES

Start the web UI with defaults:

```bash
dam serve
# Listening on http://127.0.0.1:8080
```

Start on a custom port:

```bash
dam serve --port 9090
```

Bind to all interfaces (for LAN access):

```bash
dam serve --bind 0.0.0.0 --port 8080
```

Start with request logging:

```bash
dam serve --log
```

Start with all diagnostics:

```bash
dam serve --log --time
```

### SEE ALSO

[search](#dam-search) -- CLI equivalent of the web UI browse page.
[show](#dam-show) -- CLI equivalent of the web UI asset detail page.
[CLI Conventions](00-cli-conventions.md) -- `dam.toml` configuration reference.

---

Previous: [Organize Commands](03-organize-commands.md) -- `collection`, `saved-search`, `stack`.
Next: [Maintain Commands](05-maintain-commands.md) -- `verify`, `sync`, `refresh`, `cleanup`, `relocate`, `update-location`, `generate-previews`, `fix-roles`, `fix-dates`, `rebuild-catalog`.
