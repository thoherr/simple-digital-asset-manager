# Retrieve Commands

Commands for finding assets, inspecting details, and browsing the catalog.

---

## maki search

### NAME

maki-search -- search for assets using filters and free-text keywords

### SYNOPSIS

```
maki [GLOBAL FLAGS] search <QUERY> [--format <FMT>] [-q]
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
| `rating:` | Star rating (exact, range, OR) | `rating:5`, `rating:3+`, `rating:3-5`, `rating:2,4` |
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
| `id:` | Asset ID (prefix match) | `id:72a0bb4b` |
| `variants:` | Variant count (exact or minimum) | `variants:2`, `variants:2+` |
| `scattered:` | Variants on N+ different volumes | `scattered:2+` |
| `orphan:true` | Assets with zero file locations | `orphan:true` |
| `orphan:false` | Assets with at least one file location | `orphan:false` |
| `missing:true` | Assets with files missing on disk | `missing:true` |
| `stale:N` | Not verified in N days | `stale:30`, `stale:90` |
| `volume:` | Assets on a specific volume | `volume:Archive`, `volume:none` |
| `stacked:true` | Assets in a stack | `stacked:true` |
| `stacked:false` | Assets not in any stack | `stacked:false` |
| `geo:` | GPS geolocation | `geo:any`, `geo:none`, `geo:52.5,13.4,10` |
| `faces:` | Face count (ai feature) | `faces:any`, `faces:none`, `faces:2+` |
| `person:` | Named person (ai feature) | `person:Alice`, `person:"John Smith"` |
| `similar:` | Visual similarity (ai feature) | `similar:72a0bb4b`, `similar:72a0bb4b:50` |
| `min_sim:` | Minimum similarity threshold (ai) | `min_sim:90` |
| `text:` | Semantic text-to-image search (ai) | `text:sunset`, `text:"woman reading"` |
| `embed:` | Embedding status (ai feature) | `embed:any`, `embed:none` |

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
maki search "tag:landscape rating:4+"
```

Find all videos:

```bash
maki search "type:video"
```

Search with camera and aperture filters:

```bash
maki search 'camera:"Canon EOS R5" f:1.4-2.8'
```

Get IDs for scripting:

```bash
maki search -q "tag:travel label:Green"
```

Custom format template:

```bash
maki search "rating:5" --format '{id}\t{name}\t{label}'
```

Search within a path and pipe to a collection:

```bash
maki search -q "path:Capture/2026-02-22 rating:4+" | xargs maki col add "Feb Selects"
```

Find assets with files missing on disk:

```bash
maki search "missing:true"
```

Find assets with no backup (only one copy):

```bash
maki search "copies:1"
```

Find highly-rated assets with at least two copies:

```bash
maki search "copies:2+ rating:4+"
```

Find orphaned assets (no file locations):

```bash
maki search "orphan:true"
```

Find visually similar assets (requires ai feature + embeddings):

```bash
maki search "similar:72a0bb4b"
maki search "similar:72a0bb4b:50"
maki search "similar:72a0bb4b rating:4+ tag:landscape"
```

### SEE ALSO

[show](#maki-show) -- display full details for a specific asset.
[saved-search](03-organize-commands.md#maki-saved-search-save) -- save and re-run searches.
[CLI Conventions](00-cli-conventions.md) -- output conventions, scripting patterns.

---

## maki show

### NAME

maki-show -- display full details for an asset

### SYNOPSIS

```
maki [GLOBAL FLAGS] show <ASSET_ID>
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
maki show a1b2c3d4-e5f6-7890-abcd-ef1234567890
```

Show an asset by prefix:

```bash
maki show a1b2c
```

Show as JSON and extract variant filenames:

```bash
maki show a1b2c --json | jq '.variants[].original_filename'
```

Show as JSON and list file locations:

```bash
maki show a1b2c --json | jq '.variants[].file_locations[]'
```

### SEE ALSO

[search](#maki-search) -- find assets to inspect.
[edit](02-ingest-commands.md#maki-edit) -- modify the fields shown here.
[tag](02-ingest-commands.md#maki-tag) -- add or remove tags.

---

## maki preview

### NAME

`maki preview` -- display an asset's preview image in the terminal

### SYNOPSIS

```
maki [GLOBAL FLAGS] preview <ASSET_ID> [--open]
```

### DESCRIPTION

Renders the asset's best preview image directly in the terminal using the viuer library. Auto-detects the terminal's graphics protocol (iTerm2, Kitty, Sixel) and falls back to Unicode half-block characters.

With `--open`, launches the preview file in the OS default image viewer instead of rendering in the terminal.

Also available as a shell built-in: `preview <id>`, `preview $var`, `preview _ --open`.

### OPTIONS

- `<ASSET_ID>` -- Asset ID or prefix (required)
- `--open` -- Open in the OS default viewer instead of terminal display

### EXAMPLES

Display a preview in the terminal:
```
maki preview a1b2c3d4
```

Open in the default viewer:
```
maki preview a1b2c --open
```

In the shell with variable expansion:
```
photos> $picks = search "rating:5 date:2024"
photos [picks=12]> preview $picks
```

### SEE ALSO

[show](#maki-show) -- display full asset metadata.
[search](#maki-search) -- find assets to preview.

---

## maki export

### NAME

maki-export -- copy files matching a search query to a directory or ZIP archive

### SYNOPSIS

```
maki [GLOBAL FLAGS] export <QUERY> <TARGET> [OPTIONS]
```

### DESCRIPTION

Exports files from the catalog to a target directory or ZIP archive. Searches for assets matching the query, resolves their file locations on online volumes, and copies (or symlinks) files to the target. With `--zip`, writes a single ZIP archive instead of individual files.

By default, only the **best variant** per asset is exported (Export > Processed > Original, image formats preferred, file size tiebreaker). Use `--all-variants` to export every variant.

**Layout modes**:

- **`flat`** (default): All files placed in the target root. Filename collisions (different content, same filename) are resolved by appending `_<8-char-hash>` before the extension. Files with identical content hashes reuse the same target path.
- **`mirror`**: Preserves the source volume-relative directory structure. When files span multiple volumes, each volume's files are placed under a `<volume-label>/` prefix to avoid path collisions.

Files are copied with SHA-256 integrity verification. Existing files at the target path are skipped if their content hash matches (use `--overwrite` to force re-copy).

### ARGUMENTS

**QUERY** (required)
: Search query string (same syntax as `maki search`).

**TARGET** (required)
: Target directory path (or ZIP file path with `--zip`). Created automatically if it does not exist (except in `--dry-run` mode).

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

**--zip**
: Write a ZIP archive to the target path instead of copying files to a directory. The archive uses stored (uncompressed) entries since media files are already compressed. The `.zip` extension is appended automatically if not present. Cannot be combined with `--symlink`. Layout and sidecar options work the same as directory export.

`--json` outputs an `ExportResult` object with fields: `dry_run`, `assets_matched`, `files_exported`, `files_skipped`, `sidecars_exported`, `total_bytes`, `errors`.

### EXAMPLES

Export best-of picks to a delivery folder:

```bash
maki export "rating:5 tag:portfolio" /tmp/delivery/
```

Export with directory structure preserved:

```bash
maki export "collection:Print" /Volumes/USB/export --layout mirror
```

Include sidecars for another workstation:

```bash
maki export "tag:client" /tmp/handoff/ --include-sidecars
```

Create symlinks instead of copies:

```bash
maki export "type:image rating:4+" ~/links/ --symlink
```

Export all variants (RAW + processed):

```bash
maki export "tag:portfolio" /tmp/all/ --all-variants
```

Export as a ZIP archive:

```bash
maki export "tag:client" ~/Desktop/delivery --zip
```

Dry run to see what would be exported:

```bash
maki export "collection:Best" /tmp/test/ --dry-run
```

JSON output for scripting:

```bash
maki --json export "rating:5" /tmp/out/ | jq '.files_exported'
```

### SEE ALSO

[search](#maki-search) -- find assets to export.
[contact-sheet](#maki-contact-sheet) -- generate a PDF contact sheet from search results.
[relocate](05-maintain-commands.md#maki-relocate) -- move/copy asset files between volumes with catalog updates.
[CLI Conventions](00-cli-conventions.md) -- global flags, scripting patterns.

---

## maki contact-sheet

### NAME

maki-contact-sheet -- generate a PDF contact sheet from search results

### SYNOPSIS

```
maki [GLOBAL FLAGS] contact-sheet <QUERY> <OUTPUT> [OPTIONS]
```

### DESCRIPTION

Generates a printable PDF contact sheet with thumbnail grids from assets matching a search query. Pages are composed as images at 300 DPI and wrapped in PDF. Smart previews (2560px) are preferred with automatic fallback to regular previews (800px).

Layout presets control the grid density:

- **dense** (6x8): Maximum thumbnails per page.
- **standard** (4x5): Balanced view (default).
- **large** (3x3): Detailed thumbnails.

Color labels can be rendered as colored borders around cells, small dots next to filenames, or hidden entirely.

### ARGUMENTS

**QUERY** (required)
: Search query (same syntax as `maki search`).

**OUTPUT** (required)
: Output PDF file path.

### OPTIONS

**--layout \<PRESET\>**
: Layout preset: `dense`, `standard`, `large` (default: `standard`).

**--columns \<N\>**
: Number of columns (overrides layout preset).

**--rows \<N\>**
: Number of rows per page (overrides layout preset).

**--paper \<SIZE\>**
: Paper size: `a4`, `letter`, `a3` (default: `a4`).

**--landscape**
: Use landscape orientation.

**--title \<TEXT\>**
: Title printed in page headers.

**--fields \<FIELDS\>**
: Comma-separated metadata fields: `filename`, `date`, `rating`, `label`, `format`, `size`, `dimensions`.

**--sort \<ORDER\>**
: Sort order: `date`, `name`, `rating`, `filename`.

**--no-smart**
: Use regular previews instead of smart previews.

**--group-by \<FIELD\>**
: Group by field with section headers: `date`, `volume`, `collection`, `label`.

**--margin \<MM\>**
: Page margin in millimeters.

**--label-style \<STYLE\>**
: Color label display: `border`, `dot`, `none` (default: `border`).

**--quality \<N\>**
: JPEG quality for page images, 1-100.

**--copyright \<TEXT\>**
: Copyright text in center of page footer.

**--dry-run**
: Report page/asset count without generating.

### EXAMPLES

Basic contact sheet of all 5-star images:

```bash
maki contact-sheet "rating:5" stars.pdf
```

Dense layout on A3 landscape with title:

```bash
maki contact-sheet "tag:landscape" landscapes.pdf --layout dense --paper a3 --landscape --title "Landscapes 2026"
```

Group by date with copyright:

```bash
maki contact-sheet "" all.pdf --group-by date --copyright "© 2026 Thomas Herrmann"
```

Dry run to check page count:

```bash
maki contact-sheet "format:nef" raw.pdf --dry-run
```

### SEE ALSO

[search](#maki-search) -- find assets matching a query.
[export](#maki-export) -- copy files to a directory.
[generate-previews](05-maintain-commands.md#maki-generate-previews) -- generate or upgrade previews.

---

## maki duplicates

### NAME

maki-duplicates -- find files with identical content at multiple locations

### SYNOPSIS

```
maki [GLOBAL FLAGS] duplicates [--format <FMT>] [--same-volume] [--cross-volume] [--volume <LABEL>] [--filter-format <FMT>] [--path <PREFIX>]
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
: Output format. Same presets as `maki search`: `ids`, `short` (default), `full`, `json`. Custom templates support all search placeholders plus `{locations}` and `{volumes}` (distinct volume count). Location strings include the volume purpose in brackets (e.g., `Photos[working]:Capture/photo.jpg`).

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
maki duplicates
```

Find likely unwanted same-volume duplicates:

```bash
maki duplicates --same-volume
```

Show cross-volume copies (backup verification):

```bash
maki duplicates --cross-volume
```

Filter to duplicates involving a specific volume:

```bash
maki duplicates --volume "Backup Drive"
```

Show full details with verification timestamps:

```bash
maki duplicates --format full
```

List duplicates as JSON:

```bash
maki duplicates --json | jq '.[].locations'
```

Cross-volume copies as JSON for a specific volume:

```bash
maki duplicates --cross-volume --volume Photos --json
```

Custom format showing hash and locations:

```bash
maki duplicates --format '{hash}\t{filename}\t{volumes} volumes\t{locations}'
```

### SEE ALSO

[verify](05-maintain-commands.md#maki-verify) -- verify file integrity on disk.
[cleanup](05-maintain-commands.md#maki-cleanup) -- remove stale location records.
[search](#maki-search) -- use `copies:` filter for location-count-based queries.

---

## maki stats

### NAME

maki-stats -- show catalog statistics

### SYNOPSIS

```
maki [GLOBAL FLAGS] stats [OPTIONS]
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
maki stats
```

Full statistics:

```bash
maki stats --all
```

Show only tag frequencies:

```bash
maki stats --tags --limit 50
```

Show volume details as JSON:

```bash
maki stats --volumes --json | jq '.volumes[] | {label, assets, size}'
```

Show verification health:

```bash
maki stats --verified
```

### SEE ALSO

[search](#maki-search) -- find specific assets matching criteria.
[verify](05-maintain-commands.md#maki-verify) -- run verification checks.
[volume list](01-setup-commands.md#maki-volume-list) -- list volumes with online/offline status.

---

## maki backup-status

### NAME

maki-backup-status -- check backup coverage and find under-backed-up assets

### SYNOPSIS

```
maki [GLOBAL FLAGS] backup-status [QUERY] [OPTIONS]
```

### DESCRIPTION

Answers the question: "Are my important assets safely backed up?"

In **overview mode** (default), displays aggregate statistics about backup coverage:

- **Totals**: asset count, variant count, file location count.
- **Coverage by volume purpose**: how many assets exist on volumes of each purpose (Working, Archive, Backup, Cloud), with percentages.
- **Volume distribution**: histogram of assets by number of distinct volumes they exist on (0, 1, 2, 3+), with "AT RISK" markers for assets below the threshold. Multiple variants or locations on the same volume count as one — what matters for backup safety is how many distinct volumes hold the asset.
- **At-risk summary**: count of assets with fewer than `--min-copies` locations, with hints for listing them.
- **Volume gaps**: per-volume count of missing assets (assets in scope but not on that volume).

In **at-risk listing mode** (`--at-risk`, `-q`, or `--format`), outputs a list of under-backed-up assets using the same output formats as `maki search`. When `--volume` is specified, lists assets missing from that specific volume instead of those with fewer than `--min-copies` locations overall.

An optional positional `QUERY` argument scopes the analysis to matching assets (same syntax as `maki search`).

### ARGUMENTS

**QUERY** (optional)
: Search query to scope the asset universe. Same syntax as `maki search`. When omitted, all catalog assets are analyzed.

### OPTIONS

**--at-risk**
: Switch to listing mode. Output under-backed-up assets instead of the overview.

**--min-copies \<N\>**
: Threshold for "adequately backed up" (default: 2). Assets on fewer than N distinct volumes are considered at-risk.

**--volume \<LABEL\>**
: In overview mode, adds a detailed volume coverage section for this volume. In at-risk listing mode, lists assets missing from this specific volume.

**--format \<FMT\>**
: Output format for at-risk listings. Same presets as `maki search`: `ids`, `short`, `full`, `json`, or a custom template.

**-q** / **--quiet**
: Shorthand for `--format=ids`. Prints one asset ID per line, ideal for piping to other commands.

`--json` (global flag) outputs a structured `BackupStatusResult` object in overview mode, or a JSON array of `SearchRow` objects in at-risk listing mode.

### EXAMPLES

Quick overview of backup coverage:

```bash
maki backup-status
```

Scope to highly-rated images:

```bash
maki backup-status "rating:3+ type:image"
```

Require 3 copies and check coverage:

```bash
maki backup-status --min-copies 3
```

List at-risk asset IDs for scripting:

```bash
maki backup-status --at-risk -q
```

Find assets missing from a specific volume:

```bash
maki backup-status --volume "Master Media" --at-risk -q
```

Pipe at-risk assets to relocate:

```bash
maki backup-status --volume "Master Media" --at-risk -q "rating:3+" \
  | xargs -I{} maki relocate {} "Master Media"
```

Add at-risk assets to a collection for review:

```bash
maki backup-status --at-risk -q | xargs maki collection add "Needs Backup"
```

JSON output for scripting:

```bash
maki --json backup-status | jq '.at_risk_count'
```

### SEE ALSO

[search](#maki-search) -- use `copies:` filter for location-count-based queries.
[duplicates](#maki-duplicates) -- find duplicate files across volumes.
[stats](#maki-stats) -- general catalog statistics.
[verify](05-maintain-commands.md#maki-verify) -- verify file integrity.

---

## maki serve

### NAME

maki-serve -- start the web UI server

### SYNOPSIS

```
maki [GLOBAL FLAGS] serve [--port <PORT>] [--bind <ADDR>]
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
- **Analytics page** (`/analytics`): Shooting frequency, camera/lens usage, rating distribution, format breakdown, monthly import volume, and storage per volume charts.
- **Drag-and-drop**: Drag browse cards onto the collection dropdown to add to a collection. Drag stack members on the detail page to reorder.
- **Per-stack expand/collapse**: Click the stack badge (⊞ N) on a browse card to expand/collapse just that stack, independent of the global toggle.

The server defaults to `127.0.0.1:8080`. These can be overridden by CLI flags or the `[serve]` section in `maki.toml`. CLI flags take precedence over configuration.

SQLite connections are opened per-request. Previews are served as static files. Static assets (htmx.min.js, style.css) are embedded at compile time.

### ARGUMENTS

None.

### OPTIONS

**--port \<PORT\>**
: Port to listen on. Default: 8080, or the value from `maki.toml` `[serve]` section.

**--bind \<ADDR\>**
: Address to bind to. Default: `127.0.0.1`, or the value from `maki.toml` `[serve]` section.

`--log` (global flag) enables per-request logging to stderr in the format `METHOD /path -> STATUS (duration)`.

### EXAMPLES

Start the web UI with defaults:

```bash
maki serve
# Listening on http://127.0.0.1:8080
```

Start on a custom port:

```bash
maki serve --port 9090
```

Bind to all interfaces (for LAN access):

```bash
maki serve --bind 0.0.0.0 --port 8080
```

Start with request logging:

```bash
maki serve --log
```

Start with all diagnostics:

```bash
maki serve --log --time
```

### SEE ALSO

[search](#maki-search) -- CLI equivalent of the web UI browse page.
[show](#maki-show) -- CLI equivalent of the web UI asset detail page.
[CLI Conventions](00-cli-conventions.md) -- `maki.toml` configuration reference.

---

## maki shell

### NAME

maki-shell -- interactive asset management shell

### SYNOPSIS

```
maki [GLOBAL FLAGS] shell [SCRIPT]
maki [GLOBAL FLAGS] shell -c <COMMAND>
maki [GLOBAL FLAGS] shell [--strict] [SCRIPT | -c <COMMAND>]
```

### DESCRIPTION

Starts an interactive shell that keeps catalog state cached across commands. Commands are entered without the `maki` prefix. The shell provides:

- **Readline editing** with persistent history (stored in `.maki/shell_history`) and tab completion for subcommand names, `--flags`, `$variables`, `tag:` names, and `volume:` labels.
- **Named variables** (`$name`) that store asset ID sets from command results, enabling multi-step workflows.
- **Implicit last result** (`_`) that expands to the asset IDs produced by the most recent command.
- **Session defaults** (`set --flag`) that inject flags into every command for the remainder of the session.
- **Script files** for repeatable workflows — plain text files with one command per line, comments with `#`.
- **Source command** that executes a script file within the current session, sharing variables and defaults.
- **Reload** that re-reads configuration, refreshes tab-completion data, and clears all variables and defaults.

In interactive mode, the prompt shows the catalog directory name and any active variable counts:

```
photos [picks=42 best=5]>
```

Certain commands are blocked inside the shell: `init`, `migrate`, `serve`, and `shell`.

### ARGUMENTS

**SCRIPT**
: Path to a script file to execute. Each non-empty, non-comment line is run as a shell command. The shell exits after the script completes.

### OPTIONS

**-c, --command \<COMMAND\>**
: Execute a single command string and exit. Supports multiple lines (separated by newlines) and variable assignments, just like a script.

**--strict**
: Exit with a non-zero status on the first error. Applies to script and `-c` modes. Without `--strict`, errors are printed but execution continues with the next line.

### SHELL-ONLY COMMANDS

These commands are available only inside the shell (interactive, script, and `-c` modes):

| Command | Description |
|---------|-------------|
| `help` | Show a summary of shell features and syntax |
| `quit` / `exit` | End the session (also Ctrl-D) |
| `vars` | List all named variables with asset counts, plus active session defaults |
| `unset $name` | Remove a named variable |
| `unset --flag` | Remove a session default flag |
| `set --flag` | Add a session default flag. Settable flags: `--json`, `--log`, `--debug`, `--time` |
| `reload` | Re-read config, refresh completions, clear all variables and defaults |
| `source <file>` | Execute a script file in the current session, sharing variables and defaults. Paths are resolved relative to the catalog root |

### VARIABLE SYNTAX

**Assignment:**

```
$name = <command>
```

Runs `<command>` and stores the resulting asset IDs in `$name`. The count is printed to stderr. Variable names may contain letters, digits, and underscores.

**Expansion:**

`$name` and `_` expand to asset IDs. The IDs are always placed at the **end** of the argument list, regardless of where the variable appears in the command. This means `tag $picks --add portfolio` and `tag --add portfolio $picks` are equivalent — the IDs always land in the trailing positional slot where commands expect asset IDs. Unknown variables are left as-is.

`_` (standalone, not inside a word like `_foo` or `foo_bar`) expands to the asset IDs from the most recent command that produced results.

### QUOTE HANDLING

The shell uses a two-rule quoting model:

- **Token-start quotes** (quotes at the beginning of a token) are stripped. They act as grouping quotes, like in a POSIX shell: `"tag:landscape rating:4+"` becomes a single token `tag:landscape rating:4+`.
- **Mid-token quotes** (quotes appearing after other characters) are preserved. This keeps search filter syntax intact: `text:"woman with glasses"` passes through unchanged as a single token.

### EXAMPLES

Interactive session with variables:

```
$ maki shell
photos> $picks = search "rating:5 date:2024"
  42 assets → $picks
photos [picks=42]> tag --add portfolio $picks
photos [picks=42]> export --target /tmp/best $picks
photos [picks=42]> quit
```

Run a script file:

```bash
maki shell workflow.maki
```

One-liner with `-c`:

```bash
maki shell -c 'search "tag:landscape rating:4+" --format ids'
```

Session defaults:

```
photos> set --log
  Session default: --log
  Active defaults: --log
photos> search tag:landscape
# (output includes per-file logging)
photos> unset --log
```

Source a file into the current session:

```
photos> source post-import.maki
```

Strict mode for CI scripts:

```bash
maki shell --strict batch-updates.maki
```

### SEE ALSO

[search](#maki-search) -- primary command for finding assets inside the shell.
[CLI Conventions](00-cli-conventions.md) -- global flags (`--json`, `--log`, `--debug`, `--time`) usable as session defaults.

---

Previous: [Organize Commands](03-organize-commands.md) -- `collection`, `saved-search`, `stack`.
Next: [Maintain Commands](05-maintain-commands.md) -- `verify`, `sync`, `refresh`, `cleanup`, `relocate`, `update-location`, `generate-previews`, `fix-roles`, `fix-dates`, `rebuild-catalog`.
