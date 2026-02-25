# Web UI

dam includes a browser-based interface for browsing, searching, and editing your catalog. It runs as a local web server and provides a responsive grid view of your assets, inline metadata editing, batch operations, and keyboard-driven navigation. All changes made in the web UI are written to both the SQLite catalog and YAML sidecar files, and any editable fields (rating, tags, description, color label) are automatically synced back to XMP recipe files on disk.


## Starting the Web UI

Launch the server with `dam serve`:

```bash
dam serve
```

Output:

```
Listening on http://127.0.0.1:8080
```

Open that URL in your browser to start browsing. The server runs in the foreground; press Ctrl+C to stop it.

### Custom port and bind address

Use `--port` and `--bind` to change the listening address:

```bash
dam serve --port 9090 --bind 0.0.0.0
```

Binding to `0.0.0.0` makes the UI accessible from other devices on your local network (use with caution on untrusted networks).

### Request logging

Add the `--log` flag to print each HTTP request to stderr:

```bash
dam serve --log
```

Output on stderr:

```
GET / -> 200 (12ms)
GET /static/style.css -> 200 (1ms)
GET /previews/ab/ab3f...jpg -> 200 (2ms)
```

This is useful for debugging slow requests or understanding access patterns.

### Configuration via dam.toml

You can set default values for port and bind address in the `[serve]` section of `dam.toml`:

```toml
[serve]
port = 9090
bind = "127.0.0.1"
```

Command-line flags always override `dam.toml` settings. See the [Configuration Reference](../reference/08-configuration.md) for details.


## Browse Page

The browse page is the main entry point for the web UI. It shows a searchable, filterable grid of asset thumbnails.

![Browse page with search bar, filter controls, and results grid](../screenshots/browse-page.png)

### Search bar

The search bar has two rows:

**Row 1** -- a full-width text input for free-text search. Type any keyword, filename, or structured filter (like `camera:"Canon EOS R5"`) and results update as you type with a 300ms debounce. Press Enter to search immediately without waiting for the debounce.

**Row 2** -- a row of filter controls, left to right:

- **Tag filter**: a chip-based input with autocomplete. Type to see tag suggestions, click or press Enter to add a tag chip. Multiple tags narrow the results (AND logic). Remove a tag by clicking the x on its chip, or press Backspace in an empty input to remove the last chip. Adding or removing a tag triggers an immediate search.
- **Star rating filter**: five clickable stars. Click star 3 to filter for rating 3 and above (shown as "3+"). Click star 5 to filter for exactly 5 stars. Click the active star again to clear the filter. Triggers an immediate search.
- **Color label filter**: seven colored dots (Red, Orange, Yellow, Green, Blue, Pink, Purple). Click a dot to filter by that label. Click the active dot again to clear. Triggers an immediate search.
- **Type dropdown**: filter by asset type (Image, Video, Audio, Document). Triggers immediately on change.
- **Format dropdown**: filter by variant format (NEF, ARW, JPEG, etc.). Populated from the formats present in your catalog. Triggers immediately on change.
- **Volume dropdown**: filter by storage volume. Only shown when you have registered volumes. Triggers immediately on change.
- **Collection dropdown**: filter by collection membership. Only shown when you have collections. Triggers immediately on change.
- **Path prefix input**: filter by file location path prefix. Type a directory path to see only assets stored under that path. Debounces at 300ms, Enter fires immediately.

All filters compose with each other. You can combine a text search with a tag, a minimum rating, a color label, and a volume restriction at the same time.

### Results grid

Below the search bar, results appear as a grid of thumbnail cards. Each card shows:

- A preview thumbnail (lazy-loaded for performance)
- The asset name (or filename as fallback)
- Badges for type and primary format (e.g., "image" and "NEF")
- A variant count badge when the asset has multiple variants (e.g., "3v")
- Star rating (filled stars)
- Color label dot

Click a card to open the [asset detail page](#asset-detail-page).

### Sorting

Above the results grid, sort toggle buttons let you order results by **Name**, **Date**, or **Size**. Each button toggles between ascending and descending. The active sort shows a direction arrow. Clicking a sort button updates results immediately.

### Pagination

When results span multiple pages, pagination controls appear both above and below the grid:

- First page, previous page, numbered page links, next page, last page
- A "Page X of Y" indicator

Page numbers with ellipsis keep the controls compact for large result sets.

### Saved search chips

![Saved search chips row on the browse page](../screenshots/saved-search-chips.png)

Below the search bar, a row of saved search chips provides quick access to your named searches (smart albums). Each chip loads its stored query and filters into the search bar when clicked.

- **Save the current search**: click the "Save" button at the end of the chip row. A prompt asks for a name, and the current search state (text query, filters, sort order) is saved.
- **Rename**: hover over a chip to reveal the rename button. Click it and enter a new name.
- **Delete**: hover over a chip to reveal the delete button. Click it and confirm.

Saved searches are stored in `searches.toml` at the catalog root and work identically to CLI saved searches (see [Organizing Assets](04-organize.md) for the `dam saved-search` command).

### How page updates work

The web UI uses [htmx](https://htmx.org/) for partial page updates. When you search, filter, sort, or paginate, only the results area reloads -- the search bar and its state remain intact. This makes interactions fast and fluid.

Because the URL updates with every search (via `hx-push-url`), the browser back button, forward button, reload, and bookmarks all work correctly. Navigating back from an asset detail page also refreshes results to reflect any edits you made.


## Asset Detail Page

Click any card in the browse grid to open the asset detail page.

![Asset detail page with preview, metadata editing, and variants](../screenshots/asset-detail.png)

### Preview

The left side shows a large preview image. This is the best available preview for the asset, preferring export variants over processed variants over originals, and standard image formats over RAW.

### Editable metadata

The right side contains the asset's metadata, all editable inline:

**Name** -- displayed as a heading. Click the pencil icon to switch to an inline text input with Save and Cancel buttons. Saving an empty name clears it, and the display falls back to the original filename in muted italic. The name is stored on the asset.

**Description** -- click the pencil icon to switch to a textarea with Save and Cancel buttons. Saving an empty description clears it. Changes are written back to XMP recipe files on disk.

**Rating** -- five clickable stars. Click a star to set that rating. Click the same star again to clear the rating. Changes are written back to XMP recipe files on disk.

**Color label** -- seven colored dots (Red, Orange, Yellow, Green, Blue, Pink, Purple) with a label name shown below the active dot. Click a dot to set that label; click the active dot again to clear it. Changes are written back to XMP recipe files on disk.

**Tags** -- displayed as removable chips. Click the x on a chip to remove that tag. Use the text input below to add new tags. Changes are written back to XMP recipe files on disk with operation-level deltas (tags added independently in CaptureOne or Lightroom are preserved).

### Asset information

Below the editable fields:

- **ID**: the asset's UUID
- **Type**: image, video, audio, or document
- **Date**: the asset's creation date (from EXIF or import time)

### Collections

If the asset belongs to any collections, they appear as clickable chips. Click a chip to browse that collection. Click the x button on a chip to remove the asset from that collection.

### Variants

An expandable section lists all variants of the asset in a table with columns for role, filename, format, size, and file locations (volume and path). This gives you a complete picture of where the asset's files live across your storage volumes.

### Recipes

An expandable section lists attached recipe files (XMP sidecars, CaptureOne settings, etc.) with columns for recipe type, software, and file path.

### Source metadata

If the primary variant has EXIF or XMP metadata, an expandable section displays it as key-value pairs (camera model, lens, exposure settings, GPS coordinates, etc.).


## Batch Operations

The web UI supports applying changes to multiple assets at once. Select assets in the browse grid and use the batch toolbar to tag, rate, label, group, or organize them into collections.

![Batch toolbar with selection count, tag/rating/label controls](../screenshots/batch-toolbar.png)

### Selecting assets

Each browse card has a checkbox that appears on hover. Once any card is selected, all checkboxes become permanently visible (until the selection is cleared) so you can see what is selected at a glance. Selected cards receive a visible selection border.

- **Click a checkbox** to toggle selection of individual cards
- **"Select page"** button selects all cards on the current page
- **"Clear"** button deselects everything

### Batch toolbar

A fixed toolbar appears at the bottom of the screen whenever one or more assets are selected. It shows the selection count and provides these controls:

**Tags**: a text input with "+ Tag" and "- Tag" buttons. Type a tag name and click "+ Tag" to add it to all selected assets, or "- Tag" to remove it. Press Enter in the input to add.

**Rating**: five clickable stars and a clear button (x). Click a star to set that rating on all selected assets. Click x to clear rating.

**Color label**: seven colored dots and a clear button (x). Click a dot to set that label on all selected assets. Click x to clear label.

**Collection**: a dropdown listing your collections (plus a "New..." option to create one inline). The buttons next to it are context-sensitive:
- When you are **not** browsing a collection, a "+ Collection" button adds the selected assets to the chosen collection.
- When you **are** browsing a collection (the collection filter is active), a "- Collection" button removes the selected assets from that collection. The dropdown auto-selects the current collection.

**Group by name**: merges the selected assets by filename stem. A confirmation dialog explains the action. Assets whose filenames share a common prefix (e.g., `DSC_001.nef` and `DSC_001.jpg`) are merged into a single asset with multiple variants. This cannot be undone.

After every batch operation, the selection clears and the results grid refreshes to reflect the changes. All toolbar buttons are disabled during the operation to prevent double submissions.

### Keyboard shortcuts for selection

| Key | Action |
|-----|--------|
| Cmd+A (Mac) / Ctrl+A | Select all cards on the current page |
| Escape | Clear selection (if any), otherwise clear keyboard focus |

These shortcuts are suppressed when focus is in a text input, textarea, or dropdown.


## Keyboard Navigation

The browse page supports full keyboard navigation for efficient photo culling and rating workflows. No mouse required.

### Movement

| Key | Action |
|-----|--------|
| Arrow Left | Move focus to the previous card |
| Arrow Right | Move focus to the next card |
| Arrow Up | Move focus up one row (column-aware) |
| Arrow Down | Move focus down one row (column-aware) |

The focused card has a blue outline, visually distinct from the selection highlight. If no card is focused, the first arrow key press focuses the first card.

### Actions on the focused card

| Key | Action |
|-----|--------|
| Enter | Open the focused card's asset detail page |
| Space | Toggle selection of the focused card |
| 1-5 | Set rating (applies to focused card, or to all selected if a batch selection is active) |
| 0 | Clear rating |
| Alt+1 through Alt+7 | Set color label (1=Red, 2=Orange, 3=Yellow, 4=Green, 5=Blue, 6=Pink, 7=Purple) |
| Alt+0 | Clear color label |
| r | Set Red label |
| o | Set Orange label |
| y | Set Yellow label |
| g | Set Green label |
| b | Set Blue label |
| p | Set Pink label |
| u | Set Purple label |
| x | Clear label |

Single-letter label shortcuts and number keys for rating operate on the focused card when no batch selection is active. When assets are selected (selection count > 0), rating keys apply to the entire batch.

### Focus persistence

Focus position is preserved across pagination and sort changes. If you are focused on card 5 and sort by name, card 5 (by position) remains focused after the grid reloads.

### Input suppression

All keyboard shortcuts are suppressed when focus is in a text input, textarea, or select dropdown. This prevents accidental rating or label changes while typing a search query or tag name.


## Tags Page

The tags page shows all tags in your catalog with their usage counts.

![Tags page with sortable columns and text filter](../screenshots/tags-page.png)

Navigate to `/tags` or click "Tags" in the navigation bar.

### Features

- **Sortable columns**: click the "Tag" header to sort alphabetically, or the "Assets" header to sort by usage count. Click again to reverse direction. The active sort shows a direction arrow.
- **Live text filter**: type in the filter input to narrow the tag list. Filtering begins at 2 characters. The count display updates to show "X of Y" tags.
- **Multi-column layout**: tags flow into multiple columns automatically, adapting to the viewport width.
- **Clickable tags**: click any tag name to jump to the browse page filtered by that tag.


## Collections Page

The collections page lists all your static collections (manually curated albums).

![Collections page with collection cards and create button](../screenshots/collections-page.png)

Navigate to `/collections` or click "Collections" in the navigation bar.

### Features

- **Collection cards**: each collection is shown as a card with its name, asset count, and description (if set). Click a card to browse its assets on the main browse page.
- **"+ New Collection" button**: prompts for a name and optional description, then creates the collection immediately. The page reloads to show the new card.

Collections created here are the same as those created via the CLI `dam collection create` command. See [Organizing Assets](04-organize.md) for full details on managing collections.


## Stats Page

Navigate to `/stats` or click "Stats" in the navigation bar to see a visual overview of your catalog.

The stats page displays:

- **Overview cards**: total assets, variants, recipes, online/total volumes, and total storage size
- **Asset types**: bar chart showing the distribution of images, videos, audio files, and documents
- **Variant and recipe formats**: bar charts showing format breakdowns (NEF, ARW, JPEG, XMP, etc.)
- **Volumes**: table with per-volume details including label, online/offline status, asset count, variant count, recipe count, total size, formats present, and verification coverage
- **Tags**: summary of unique tags, tagged assets, and untagged assets, plus a weighted tag cloud of the most-used tags
- **Verification health**: overall coverage bar, plus a per-volume breakdown of verification status

This is the web equivalent of `dam stats --all` on the command line. See [Browsing & Searching](05-browse-and-search.md) for the CLI stats command.


## Backup Status Page

Navigate to `/backup` or click "Backup" in the navigation bar to see backup health at a glance.

The backup status page displays:

- **Summary cards**: total assets, at-risk count (highlighted in red when > 0), and minimum copies threshold (default 2)
- **At-risk link**: when at-risk assets exist, a prominent link navigates to the browse page filtered to `copies:1` for immediate review
- **Volume distribution**: horizontal bar chart showing how many assets exist on 0, 1, 2, or 3+ volumes, with red/amber/green coloring and "AT RISK" badges on under-backed-up buckets
- **Coverage by purpose**: table showing each volume purpose (working, archive, backup) with the number of volumes, asset count, and a coverage bar
- **Volume gaps**: table listing volumes with missing assets, showing the volume label, purpose, and missing count

This is the web equivalent of `dam backup-status` on the command line. See [Maintenance](07-maintenance.md) for the CLI backup-status command.

---

Next: [Maintenance](07-maintenance.md) -- verification, sync, refresh, cleanup, and file relocation.
