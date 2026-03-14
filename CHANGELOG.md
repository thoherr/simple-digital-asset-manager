# Changelog

All notable changes to the Digital Asset Manager are documented here.

## v3.2.2 (2026-03-14)

### New Features
- **CLI `--zip` export** — `dam export <query> <target> --zip` writes a ZIP archive instead of copying files to a directory. Appends `.zip` extension if missing. Layout, all-variants, and sidecar options work the same as directory export.
- **Shell tilde expansion** — `~` and `~/path` expand to `$HOME` in shell tokens (e.g. `export $picks ~/Desktop/out`).
- **Shell `export` built-in** — `export` is now a shell built-in with full variable expansion and `--zip` support. Multi-ID variables export all assets in a single operation.
- **Web UI batch delete** — delete button in the browse toolbar with confirmation modal, asset thumbnails, "remove files from disk" checkbox, and automatic grid refresh. New `POST /api/batch/delete` endpoint.
- **Editable ZIP filename** — the web export modal now includes a text field to customize the archive name.

### Bug Fixes
- **Multi-ID shell export** — exporting a variable with multiple asset IDs now exports all files instead of only the last one (`ParsedSearch.asset_ids` changed from `Option<String>` to `Vec<String>`).

### Internal
- Deduplicated ZIP-building logic: web export and CLI `--zip` share `AssetService::export_zip_for_ids()`.

## v3.2.1 (2026-03-14)

### Documentation
- **Writeback reference entry** — added formal `dam writeback` section to the maintain commands reference (SYNOPSIS, OPTIONS, EXAMPLES, SEE ALSO), matching the format of all other commands.
- **Manual index completeness** — updated command lists to include all documented commands (added `delete`, `split`, `embed`, `preview`, `contact-sheet`, `backup-status`, `stack`, `faces`, `sync-metadata`, `writeback`, `dedup`, `fix-recipes`, `migrate`).
- Fixed stale version reference in shell example output.

## v3.2.0 (2026-03-14)

### New Features
- **Web UI export as ZIP** — download selected assets or all filtered results as a ZIP archive directly from the browser. "Export" button in the batch toolbar for selected assets; "Export all" link in the results bar for the current search/filter state. Modal dialog offers layout (flat/mirror), all-variants, and include-sidecars options. Backend streams the ZIP via a temp file to handle large exports. New `POST /api/batch/export` endpoint accepts either explicit asset IDs or the full set of browse filter parameters (type, tag, format, volume, rating, label, collection, path, person).

### Bug Fixes
- **Dark mode modals** — fixed unreadable text in group-confirm and export modals by using correct CSS variables (`--text`, `--bg-input`) instead of undefined `--text-main` and `--bg-hover`.

## v3.1.0 (2026-03-13)

### New Features
- **`dam preview`** — display asset preview images directly in the terminal using viuer (auto-detects iTerm2, Kitty, Sixel, Unicode half-block fallback). Also available as a shell built-in (`preview $picks`). `--open` flag launches the preview in the OS default viewer instead.

### Enhancements
- **Consistent positional query** — `writeback`, `fix-dates`, `fix-recipes`, `sync-metadata`, `describe`, `auto-tag`, and `embed` now accept a positional search query as the first argument (same syntax as `dam search`), replacing the previous `--query` flag. Example: `dam describe "rating:4+"` instead of `dam describe --query "rating:4+"`.
- **Shell variable expansion** — all seven commands above now support shell variable expansion (`$var`, `_`) via hidden trailing asset IDs, so `describe $picks` and `writeback _` work in the interactive shell.
- **Scope filtering for writeback** — `dam writeback` can now be narrowed by query, `--asset`, or `--volume` to process only matching recipes instead of the entire catalog.
- **Scope filtering for fix-dates/fix-recipes/sync-metadata** — these commands now support the same query/asset/asset_ids scope resolution as other multi-asset commands.

## v3.0.3 (2026-03-13)

### Performance
- **SQLite connection pool** — web server reuses pre-opened database connections instead of opening a new one per request, eliminating repeated PRAGMA setup overhead.
- **Split COUNT/data queries** — browse pagination replaced `COUNT(*) OVER()` window function (which forced full result materialization) with a separate lightweight count query, reducing browse times from 1–6s to under 300ms.
- **Version-guarded migrations** — `run_migrations()` checks the stored schema version and skips all work when the catalog is already current, reducing startup to a single SELECT query.

### Code Quality
- **Deduplicated migration blocks** — `initialize()` now creates base tables and delegates to `run_migrations()` instead of duplicating ~130 lines of ALTER TABLE / CREATE INDEX / backfill statements.
- **Deduplicated image finder** — `find_image_for_ai()` and `find_image_for_vlm()` (~100 lines each) consolidated into a shared `find_image_for_processing()` with a predicate parameter.
- **Deduplicated best-variant resolution** — extracted `resolve_best_variant_idx()` helper, replacing 3 copies of the stored-hash-with-algorithmic-fallback pattern in web routes.
- **Unified variant scoring** — merged `role_score_enum`/`role_score_str` and `best_preview_index`/`best_preview_index_details` into shared implementations.
- **Gated AI-only imports** — `PeoplePage`, `PersonCard` imports and `people` field on `DropdownCacheInner` are now behind `#[cfg(feature = "ai")]`, eliminating compiler warnings when building without the `ai` feature.

## v3.0.2 (2026-03-13)

### New Features
- **Preview variant override** — manually choose which variant represents an asset in the browse grid, detail page, and contact sheets, overriding the default Export > Processed > Original scoring. Click the star icon in the variant table on the detail page to set. Stored in sidecar YAML and respected by `generate-previews`, rotate, and regenerate.

### Bug Fixes
- **Group confirmation popup** — the merge confirmation dialog showed only truncated asset IDs instead of thumbnails and names. Fixed a `data-id` vs `data-asset-id` attribute mismatch that prevented card lookup.

## v3.0.1 (2026-03-12)

### Bug Fixes
- **`volume:<label>` search filter** — the CLI `search` command silently ignored `volume:<label>` filters (only `volume:none` worked). Now resolves volume labels case-insensitively, supports comma-OR (`volume:Vol1,Vol2`), and negation (`-volume:Label`).
- **Shell variable expansion for single-asset commands** — variables like `$picks` or `_` containing multiple asset IDs now correctly loop single-asset commands (`tag`, `edit`, `show`, `split`, `update-location`) per ID, instead of appending all IDs as trailing arguments.

### Enhancements
- **Clear tags button** — detail page now shows a "× Clear" button next to tags, with confirmation dialog, to remove all tags from an asset at once.
- **Tag filter keyboard navigation** — browse page tag autocomplete now supports Arrow Up/Down to highlight suggestions, Enter to select, and Escape to dismiss (matching the detail page behavior).

## v3.0.0 (2026-03-12)

### New Commands
- **`dam shell`** — interactive asset management shell with readline-based REPL, replacing one-shot CLI invocations for interactive workflows. Features:
  - **Named variables** — `$picks = search "rating:5 date:2024"` stores result sets; `$picks` expands to asset IDs in any subsequent command
  - **Implicit `_` variable** — always holds asset IDs from the last command
  - **Session defaults** — `set --json` / `set --log` / `set --debug` / `set --time` auto-inject flags into all commands
  - **Tab completion** — subcommands, `--flags`, `$variables`, `tag:names`, `volume:labels` (cached from catalog)
  - **Script files** — `dam shell script.dam` executes `.dam` files with variables, comments, and shared session state
  - **Single-command mode** — `dam shell -c 'search "rating:5"'` for one-liners in external scripts
  - **`--strict` flag** — exit on first error in scripts and `-c` mode
  - **`source <file>`** — execute a script inline, sharing the current session's variables and defaults
  - **`reload`** — re-read config, refresh tab completion data, clear variables and defaults
  - **Smart quote handling** — `search text:"woman with glasses"` works without multi-level quoting (mid-token quotes preserved, token-wrapping quotes stripped)
  - **Blocked commands** — `init`, `migrate`, `serve`, `shell` are rejected with a clear message
  - **History** — persisted to `.dam/shell_history` in the catalog directory

### Enhancements
- **`dam --help` reorganization** — `serve` and `shell` grouped under new "Interactive" category (previously `serve` was under "Retrieve")

## v2.5.3 (2026-03-12)

### Enhancements
- **Concurrent VLM requests** — the `[vlm] concurrency` setting is now fully functional. Set `concurrency = 4` in `dam.toml` to process multiple assets in parallel during `dam describe`, `dam import --describe`, and web UI batch describe. Uses scoped threads with chunked processing: preparation and result application remain sequential (catalog writes), while VLM HTTP calls (base64 encoding + curl) run concurrently. Default remains `1` (sequential) for backward compatibility.

## v2.5.2 (2026-03-12)

### New Features
- **`variants:` search filter** — filter by variant count per asset. `variants:3` (exactly 3), `variants:5+` (5 or more). Uses denormalized `variant_count` column — no JOIN needed.
- **`scattered:` search filter** — find assets whose variants span multiple directories. `scattered:2` finds assets with file locations in 2+ distinct volume:directory combinations. Useful for auditing mis-grouped assets after import.
- **Configurable `text:` search limit** — the result count for AI text-to-image search is now configurable at three levels: inline syntax `text:"query":100`, `[ai] text_limit` in `dam.toml` (default 50), and hardcoded fallback of 50. Applies to both CLI and web UI.
- **Re-import metadata** — button on the asset detail page that clears tags, description, rating, and color label, then re-extracts from variant source files (XMP sidecars and embedded XMP in JPEG/TIFF). Useful for cleaning up metadata after splitting mis-grouped assets.

### Bug Fixes
- **Stale browse after detail mutations** — dissolving a stack, changing the pick, or other detail page mutations now mark the browse page as dirty. On back-navigation (including bfcache), the browse grid automatically refreshes.
- **Stale stack pick on back-navigation** — browse page now sends `Cache-Control: no-store` to prevent the browser from serving stale HTML on back button.
- **Silent error on preview regenerate** — regenerate/rotate preview buttons are now hidden when source files are offline. If the volume goes offline mid-session, an error message is shown instead of a silent 500.

## v2.5.1 (2026-03-11)

### New Features
- **Analytics dashboard** (`/analytics`) — shooting frequency, camera/lens usage, rating distribution, format breakdown, monthly import volume, and storage per volume charts. Accessible from the nav bar under Maintain.
- **Batch relocate** — `dam relocate --query <QUERY> --target <VOLUME>` moves entire search results to a target volume in one command. Also supports stdin piping (`dam search -q "..." | dam relocate --target <VOL>`) and multiple positional IDs. Backward compatible with the existing single-asset `dam relocate <ID> <VOL>` syntax.
- **Drag-and-drop** — drag browse cards onto the collection dropdown to add assets to a collection. Drag stack members on the detail page to reorder (drop to first position sets the pick). Visual feedback with drop highlights and toast notifications.
- **Per-stack expand/collapse** — click the stack badge (⊞ N) on a browse card to expand or collapse just that stack, independent of the global collapse toggle. When globally expanded, clicking a badge collapses only that stack; re-clicking restores it.

### Bug Fixes
- **Stack member count on detail page** — detail page now shows all stack members including the current asset, fixing an off-by-one where the pick was excluded from the member list.
- **Per-stack expand with global expand** — clicking the stack badge when stacks were globally expanded no longer adds duplicate cards. Now correctly hides non-pick members of just that stack.
- **Keyboard focus preservation** — global stack toggle and htmx swaps now preserve focus by asset ID instead of grid index, preventing focus from jumping to the wrong card.

## v2.5.0 (2026-03-11)

### New Features
- **`text:` semantic search filter** — natural language image search using SigLIP's text encoder. Encode a text query into the same embedding space as image embeddings and find matching images via dot-product similarity. Supports quoted multi-word queries: `text:"sunset on the beach"`, `text:"colorful flowers" rating:3+`. Returns top 50 results, composable with all other filters. Requires `--features ai` and embeddings generated via `dam embed` or `dam import --embed`. Available in CLI, web UI, and saved searches.
- **`dam import --describe`** — auto-describe imported assets via VLM as a post-import phase. Checks VLM endpoint availability (5s timeout), then calls the configured VLM for each new asset. Silently skips if endpoint is not reachable. Can be enabled permanently via `[import] descriptions = true` in `dam.toml`. JSON output includes `descriptions_generated`, `descriptions_skipped`, and `describe_tags_applied` keys.

## v2.4.2 (2026-03-10)

### New Commands
- **`dam describe`** — generate image descriptions and tags using a vision-language model (VLM). Sends preview images to any OpenAI-compatible API server (Ollama, LM Studio, vLLM) — no feature gate or special build needed. Three modes: `--mode describe` (default, natural language descriptions), `--mode tags` (JSON tag suggestions), `--mode both` (two separate VLM calls for description + tags). Report-only by default; `--apply` writes results. `--force` overwrites existing descriptions. `--dry-run` skips VLM calls entirely. Supports `--json`, `--log`, `--time`.

### New Features
- **VLM web UI integration** — "Describe" button on asset detail page and batch "Describe" button in browse toolbar. VLM availability detected at server startup with a 5-second health check. Buttons hidden when no VLM endpoint is reachable.
- **Configurable VLM temperature** — `--temperature` CLI flag and `[vlm] temperature` config option (default 0.7) control sampling randomness. Lower values (0.0) give deterministic output; higher values give more varied results.
- **`[vlm]` configuration section** — full VLM config in `dam.toml`: endpoint, model, max_tokens, prompt, timeout, temperature, mode, concurrency. CLI flags override config values.
- **Truncated JSON recovery** — VLM tag responses that are cut off by max_tokens are salvaged: complete JSON strings are extracted from partial arrays.
- **Tag deduplication** — VLM-suggested tags are deduplicated case-insensitively before merging with existing asset tags.
- **Ollama native API fallback** — if the OpenAI-compatible `/v1/chat/completions` endpoint returns 404, automatically falls back to Ollama's native `/api/generate` endpoint.

## v2.4.1 (2026-03-09)

### New Features
- **CoreML GPU acceleration** — new `--features ai-gpu` enables CoreML execution provider on macOS for SigLIP and face detection/recognition. `[ai] execution_provider` config option (`"auto"`, `"cpu"`, `"coreml"`). Shared `build_onnx_session()` helper with automatic CPU fallback. Linux CUDA and Windows DirectML tracked as roadmap items.
- **Clickable tags on detail page** — tag chips on the asset detail page link to `/?tag=...` for browsing by tag. Sets `dam-browse-focus` before navigating so the browse page scrolls to the originating asset.

### Bug Fixes
- **Fix stroll page Escape key navigation loop** — popstate handler was pushing new history entries, creating an infinite back loop. Added `skipPush` parameter and history depth tracking.
- **Fix stroll Escape exiting browser fullscreen** — added fullscreen guard; uses `history.back()` instead of `location.href` assignment.
- **Defer stroll Escape navigation (150ms)** — keyup event was firing on bfcache-restored page, causing immediate fullscreen exit. `setTimeout(150)` lets keyup complete first.
- **Apply deferred Escape to detail and compare pages** — same fullscreen fix pattern as stroll for consistent behavior across all pages.

## v2.4.0 (2026-03-09)

### New Commands
- **`dam contact-sheet`** — Generate PDF contact sheets from search results. Image-based rendering at 300 DPI with configurable layout (dense/standard/large), paper size (A4/letter/A3), metadata fields, color label display (border/dot/none), section grouping (date/volume/collection/label), and copyright text. Smart previews used by default with fallback to regular. Configurable via `[contact_sheet]` in `dam.toml` and CLI flags.
- **`dam split`** — Extract variants from an asset into new standalone assets. Each extracted variant becomes a separate asset with role `original`, inheriting tags, rating, color label, and description. Associated recipes move with the variant. Available via CLI, web API (`POST /api/asset/{id}/split`), and detail page UI (variant checkboxes + "Extract as new asset(s)" button).

### New Features
- **Alternate variant role** — New `alternate` role (score 50) for donor originals during grouping and import. Replaces the semantically incorrect `export` role when re-roling donor variants in `group`, `auto-group`, `split`, `import` (RAW+JPEG pairs), and `fix-roles`. Ranks below `original` (100) for preview selection, reflecting "second best" status.
- **Group button in web UI** — Direct merge of selected assets (distinct from "Group by name" which uses stem matching). Focused asset (keyboard navigation) becomes the merge target. Thumbnail confirm modal shows all selected assets with target highlighted.
- **Grouped help output** — `dam --help` now shows commands organized by category (Setup, Ingest & Edit, Organize, Retrieve, Maintain) with section headers. Output paginated through `less` when stdout is a terminal.
- **Browse selection fix** — Selection cleared on forced page reload (Ctrl+Shift+R) but preserved across back-navigation and query changes for shopping-cart workflow.
- **Group confirm modal** — Visual confirmation dialog with thumbnails of selected assets before merging, replacing plain text confirm. Off-page assets show ID placeholder.

### Bug Fixes
- Contact sheet footer version printed without "v" prefix for consistency
- Fixed stale "exports" wording in group comment and confirm dialog

## v2.3.5 (2026-03-09)

### New Features
- **`dam sync-metadata` command** — bidirectional XMP metadata sync in a single command. Phase 1 (Inbound): detects externally modified XMP recipe files and re-reads metadata. Phase 2 (Outbound): writes pending DAM edits to XMP. Phase 3 (Media, with `--media`): re-extracts embedded XMP from JPEG/TIFF files. Detects conflicts when both sides changed. Supports `--volume`, `--asset`, `--dry-run`, `--json`, `--log`, `--time`.
- **`id:` search filter** — query assets by UUID prefix in both CLI and web UI. `dam search "id:c654e"` matches assets whose ID starts with the given prefix.

### Enhancements
- **Comprehensive derived file cleanup** — `dam cleanup`, `dam delete`, and `dam volume remove` now handle all derived file types: regular previews, smart previews, SigLIP embedding binaries, face crop thumbnails, ArcFace embedding binaries, and embedding/face DB records. Previously only regular previews were cleaned up, leaving orphaned files to accumulate.
- **Seven-pass cleanup** — `dam cleanup` now runs 7 passes (up from 3): stale locations, orphaned assets (with full derived file removal), orphaned previews, orphaned smart previews, orphaned SigLIP embeddings, orphaned face crops, and orphaned ArcFace embeddings. New counters reported in both human and JSON output.

### Bug Fixes
- **FK constraint error in cleanup/delete** — cleanup and volume-remove failed with "FOREIGN KEY constraint failed" when deleting orphaned assets that had faces, stacks, or collection memberships. Now clears all dependent records before asset deletion.
- **Face preview thumbnails** — people page now auto-backfills `representative_face_id` for people who had no thumbnail (e.g., after clustering).
- **Nav menu items on non-browse pages** — Stroll and People menu items no longer disappear when navigating away from the browse page.

## v2.3.4 (2026-03-09)

### Enhancements
- **Shared lightbox component** — lightbox with full rating/label editing is now available on browse, detail, and stroll pages. Extracted as a reusable shared component with items-based API and page-specific callbacks.
- **Chained detail navigation** — navigating through similar images (detail→similar→detail) now uses `history.back()` for correct back-button behavior at any depth.
- **Shift+B shortcut** — jump directly to the browse grid from detail, stroll, or compare pages.
- **Nav menu reorganization** — menu items grouped by function (Explore, Organize, Maintain) with visual separators for clarity.
- **Updated navigation docs** — state diagram expanded with stroll, compare, shared lightbox, and all navigation paths.

## v2.3.3 (2026-03-08)

### New Features
- **`embed:` search filter** — `embed:any` and `embed:none` filters to find assets with or without AI embeddings. Works in CLI, web UI, and saved searches. Composable with all other filters.
- **`dam writeback` command** — writes back pending metadata changes (rating, label, tags, description) to XMP recipe files. When edits are made while a volume is offline, recipes are automatically marked `pending_writeback`. The new command replays writes when volumes come online. Flags: `--volume`, `--asset`, `--all`, `--dry-run`. Supports `--json`, `--log`, `--time`.

### Bug Fixes
- **Stroll→detail→back navigation** — opening an asset detail page from the stroll page now correctly returns to stroll (not browse) on Escape, Back, or image click. Stroll stores navigation context in sessionStorage.

### Internal
- Schema version bumped to 2 (`pending_writeback` column on `recipes` table).

## v2.3.2 (2026-03-08)

### Bug Fixes
- **Fix FK constraint error in group/auto-group** — `insert_asset()` used `INSERT OR REPLACE` which SQLite implements as DELETE+INSERT, triggering foreign key violations from variants/faces/collections referencing the asset. Changed to `INSERT ... ON CONFLICT DO UPDATE` (true upsert). Also added proper FK cleanup in `group()` before deleting donor assets.

### New Features
- **Stroll modes** — three modes for neighbor selection: **Nearest** (default, top N by similarity), **Discover** (random N from configurable pool), **Explore** (skip first K nearest, then take N). Mode selector buttons in the stroll control panel.
- **Cross-session filtering** — "Other shoots" toggle excludes assets from the same directory/session when finding similar neighbors. Uses parent directory as session root.
- **`stroll_discover_pool` config** — `dam.toml` `[serve]` section supports `stroll_discover_pool` (default 80) to control the candidate pool size for Discover mode.

## v2.3.1 (2026-03-08)

### Enhancements
- **Elliptical satellite layout** — stroll page satellites now follow an elliptical orbit that adapts to the viewport aspect ratio, using more horizontal space in landscape and more vertical space in portrait orientations.
- **Fan-out slider** — replaces the depth slider (0–8) with a fan-out slider (0–10) that shows transitive L2 neighbors behind focused satellites. Focused satellite pulls 30% toward center when fan-out is active to make room for L2 thumbnails.
- **Direction-dependent L2 radius** — L2 neighbor arcs spread wider horizontally and narrower vertically, making better use of available screen space.
- **L2 thumbnail metadata** — L2 (transitive neighbor) thumbnails now show name, rating, color label, and similarity score, consistent with L1 satellite display.
- **L1/L2 keyboard navigation** — Arrow Up/Down moves between L1 satellites and their L2 neighbors. Hover suppression during keyboard navigation prevents focus catch-back.
- **Stroll slider configuration** — `dam.toml` `[serve]` section supports `stroll_neighbors`, `stroll_neighbors_max`, `stroll_fanout`, and `stroll_fanout_max` to configure stroll page slider defaults and ranges.

## v2.3.0 (2026-03-07)

### New Features
- **Stroll page** (feature-gated: `--features ai`) — graph-based visual similarity exploration at `/stroll`. A center image surrounded by radially arranged satellite images shows visually similar assets. Click any satellite to navigate — it becomes the new center with fresh neighbors. Features: viewport-adaptive sizing, smart preview loading, keyboard navigation (arrow keys cycle satellites, Enter navigates, `d` opens detail page), rating stars and color label dots on all images, similarity percentage badges, browser history integration (`pushState`/`popstate`). Neighbor count adjustable via slider (5–25, default 12) in a fixed bottom-left overlay. Entry points: nav bar "Stroll" link, `s` keyboard shortcut on browse/lightbox/detail pages, "Stroll from here" button on detail page, or direct URL `/stroll?id=<asset-id>`. Without an `id`, picks a random embedded asset.
- Stroll page depth slider (0–8) for exploring neighbors-of-neighbors — lazy-loaded, cached, with deduplication and fade-in animation
- **`similar:` search filter** (feature-gated: `--features ai`) — find visually similar assets from the CLI using stored embeddings. Syntax: `similar:<asset-id>` (top 20 results) or `similar:<asset-id>:<limit>` (custom limit). Composable with all other search filters, e.g. `dam search "similar:abc12345 rating:3+ tag:landscape"`. Uses the in-memory `EmbeddingIndex` for fast dot-product search. Requires embeddings to have been generated via `dam embed` or `dam import --embed`.
- **Collapsible filter bar** — the browse and stroll pages share an identical filter bar (search input, tag chips, rating stars, color label dots, type/format/volume/collection/person dropdowns, path prefix). Toggle with Shift+F or the "Filters" button. State persisted in localStorage. Auto-opens when filters are active.

### Performance
- **Schema version fast-check** — CLI commands no longer run ~30 migration statements on every invocation. A `schema_version` table tracks the current schema version; commands check it with a single fast query and exit with an error if outdated (`Error: catalog schema is outdated ... Run 'dam migrate' to update.`). Saves ~2 seconds per CLI invocation on migrated catalogs. Only `dam init` and `dam migrate` modify the schema.

### Bug Fixes
- **MicrosoftPhoto:Rating normalization** — XMP parser matched both `xmp:Rating` (0–5) and `MicrosoftPhoto:Rating` (percentage scale 0–100) as "Rating" after stripping namespace prefix. Percentage values (20/40/60/80/100) are now converted to 1–5 scale. `dam migrate` fixes existing SQLite and YAML sidecar data automatically.
- **Rating display clamp** — star rendering in JS (stroll satellite navigation) and API responses now clamped to max 5, preventing display corruption from out-of-range values.

### Enhancements
- **Shared filter bar partials** — extracted `filter_bar.html` and `filter_bar_js.html` as reusable Askama template includes, eliminating ~400 lines of duplicated filter UI code between browse and stroll pages. Both pages define an `onFilterChange()` callback; browse triggers htmx form submit, stroll rebuilds the similarity query.
- **`dam migrate` rating repair** — migration now fixes YAML sidecar files with out-of-range rating values (MicrosoftPhoto:Rating percentages) alongside the SQLite fix. Reports count of fixed sidecars.
- **`dam migrate` output** — now prints the schema version number: `Schema migrations applied successfully (schema version N).` JSON output includes `schema_version` and `fixed_ratings` fields.

## v2.2.2 (2026-03-07)

### New Features
- **`dam migrate` command** — explicit CLI command for running database schema migrations. Migrations now run once at program startup for all commands (not per-connection), making this command useful for manual migration or scripting.
- **`dam import --embed`** — generate SigLIP image embeddings for visual similarity search during import (requires `--features ai`). Runs as a post-import phase using preview images. Can be enabled permanently via `[import] embeddings = true` in `dam.toml`. Silently skips if the AI model is not downloaded.

### Performance
- **SQLite performance pragmas** — all database connections now use WAL journal mode, 256 MB mmap, 20 MB cache, `synchronous=NORMAL`, and in-memory temp store. Significant improvement for read-heavy web UI workloads.
- **Single DB connection per detail page request** — asset detail page went from 3 separate SQLite connections to 1, eliminating redundant connection overhead.
- **Combined search query** — browse page now uses `COUNT(*) OVER()` window function to get row count and results in a single query instead of two separate queries.
- **Migrations removed from hot path** — `Catalog::open()` no longer runs schema migrations. Migrations run once at program startup via `Catalog::open_and_migrate()`. Per-request connections in the web server skip migration checks entirely.
- **Dropdown cache warming at server startup** — tag, format, volume, collection, and people dropdown data is pre-loaded when `dam serve` starts, so the first browse page load is as fast as subsequent ones.

## v2.2.1 (2026-03-06)

### New Features
- **`dam faces export`** — exports faces and people from SQLite to YAML files (`faces.yaml`, `people.yaml`) and ArcFace face embeddings to binary files (`embeddings/arcface/<prefix>/<face_id>.bin`). One-time migration command to populate the new file-based persistence layer from existing SQLite data.
- **`dam embed --export`** — exports SigLIP image similarity embeddings from SQLite to binary files (`embeddings/<model>/<prefix>/<asset_id>.bin`). One-time migration for existing embedding data.

### Enhancements
- **Dual persistence for faces, people, and embeddings** — all face/people/embedding write paths (CLI and web UI) now persist data to both SQLite and YAML/binary files. Face records are stored in `faces.yaml`, people in `people.yaml`, ArcFace embeddings as binary files under `embeddings/arcface/`, and SigLIP embeddings under `embeddings/<model>/`. This mirrors the existing pattern used by collections and stacks.
- **`rebuild-catalog` restores AI data** — `rebuild-catalog` now drops and restores the `faces`, `people`, and `embeddings` SQLite tables from YAML and binary files, ensuring no AI data is lost during catalog rebuilds.
- **`dam delete` cleans up AI files** — deleting assets now removes associated ArcFace and SigLIP binary files and updates `faces.yaml`/`people.yaml`.

## v2.2.0 (2026-03-05)

### New Features
- **Face detection** (feature-gated: `--features ai`) — `dam faces detect [--query <Q>] [--asset <id>] [--volume <label>] [--apply]` detects faces in images using YuNet ONNX model. Stores face bounding boxes, confidence scores, and 512-dim ArcFace embeddings. Generates 150×150 JPEG crop thumbnails in `faces/` directory. Reports faces found per asset. Supports `--json`, `--log`, `--time`.
- **Face auto-clustering** — `dam faces cluster [--query <Q>] [--asset <id>] [--volume <label>] [--threshold <F>] [--apply]` groups similar face embeddings into unnamed person groups using greedy single-linkage clustering. Default threshold 0.5 (configurable via `[ai] face_cluster_threshold`). Without `--apply` shows dry-run cluster sizes. Scope filters (`--query`, `--asset`, `--volume`) limit which faces are clustered.
- **People management CLI** — `dam faces people [--json]` lists all people with face counts. `dam faces name <ID> <NAME>` names a person. `dam faces merge <TARGET> <SOURCE>` merges two people. `dam faces delete-person <ID>` deletes a person. `dam faces unassign <FACE_ID>` removes a face from its person.
- **People web page** (`/people`) — gallery grid of person cards with representative face crop thumbnails, names, face counts. Inline rename, merge, delete. "Cluster" button to run auto-clustering from the UI.
- **Asset detail faces section** — detected faces shown as chips with crop thumbnails and confidence scores. "Detect faces" button triggers on-demand detection. Assign/unassign faces to people via dropdown.
- **Browse face filters** — `faces:any` / `faces:none` / `faces:N` / `faces:N+` filter by face count. `person:<name>` / `-person:<name>` filter by assigned person. Person dropdown in browse filter row.
- **Batch face detection** — "Detect faces" button in browse batch toolbar for selected assets.
- **Face count badge** on browse cards (like variant count badge).
- **Denormalized `face_count` column** on assets table for fast filtering.

### New API Endpoints
- `GET /api/asset/{id}/faces`, `POST /api/asset/{id}/detect-faces`, `POST /api/batch/detect-faces`
- `GET /people`, `GET /api/people`, `PUT /api/people/{id}/name`, `POST /api/people/{id}/merge`, `DELETE /api/people/{id}`
- `PUT /api/faces/{face_id}/assign`, `DELETE /api/faces/{face_id}/unassign`, `POST /api/faces/cluster`

### New Modules (ai feature)
- `src/face.rs` — FaceDetector: YuNet detection + ArcFace recognition ONNX pipeline, multi-stride output decoder, face crop generation
- `src/face_store.rs` — FaceStore: SQLite-backed face/people persistence, embedding clustering, auto-cluster

### Bug Fixes
- Fix multi-stride YuNet model output parsing (12 separate tensors at strides 8/16/32)
- Fix `dam faces detect --asset` finding zero results (use direct asset ID resolution)

## v2.1.2 (2026-03-05)

### New Features
- **`dam embed` command** (feature-gated: `--features ai`) — batch-generate image embeddings for visual similarity search without tagging. `dam embed [--query <Q>] [--asset <id>] [--volume <label>] [--model <id>] [--force]`. Requires at least one scope filter. `--force` regenerates even if an embedding already exists. Reports embedded/skipped/error counts. Supports `--json`, `--log`, `--time`.

### Enhancements
- **In-memory embedding index** — similarity search (`dam auto-tag --similar`, web UI "Find similar") now uses a contiguous in-memory float buffer (`EmbeddingIndex`) instead of per-query SQLite blob scanning. The index is loaded lazily on first query and cached for the server lifetime. At 100k assets, search drops from seconds to <10ms. Top-K selection uses a min-heap instead of full sort.
- **Opportunistic embedding storage** — the web UI "Suggest tags" and batch "Auto-tag" endpoints now store image embeddings as a side effect, building up the similarity search index without requiring a separate `dam embed` step.
- **Deferred model loading in similarity search** — `find_similar_inner` no longer acquires the AI model lock when the query embedding already exists in the store, avoiding unnecessary contention and startup latency on repeat searches.

## v2.1.1 (2026-03-04)

### New Features
- **Multi-model support for AI auto-tagging** — the system now supports multiple SigLIP model variants. A new `--model` flag on `dam auto-tag` selects the model (default: `siglip-vit-b16-256`). Available models: SigLIP ViT-B/16-256 (768-dim, ~207 MB) and SigLIP ViT-L/16-256 (1024-dim, ~670 MB). `--list-models` shows all known models with download status, size, and active indicator. Embeddings are stored per-model (composite PK) so switching models doesn't corrupt existing data. Configurable via `[ai] model` in `dam.toml`.
- **AI tag suggestions show already-applied tags** — the web UI "Suggest tags" panel now shows all matching tags, including ones already on the asset. Already-applied tags appear dimmed with an "already applied" label and cannot be re-added. "Accept all" renamed to "Accept new" and only applies tags not yet on the asset.

### Enhancements
- **Merged preview regeneration button** — the asset detail page now has a single "Regenerate previews" button that regenerates both the regular preview and the smart preview in one operation, with cache-busted URLs so the browser shows the new images without requiring a page reload.
- **Scope guard for auto-tag** — `dam auto-tag` now requires at least one scope filter (`--query`, `--asset`, `--volume`, or `--similar`) to prevent accidental full-catalog processing.

### Bug Fixes
- **Fix RAW preview orientation** — `dcraw_emu` already pixel-rotates its output, but the code was reading EXIF orientation from the source RAW file and applying it again, turning portrait images back to landscape (affected e.g. Nikon Z9 NEF files). Fixed by reading orientation from the output TIFF instead. Also fixed the `dcraw -e -c` path to apply EXIF orientation from the embedded JPEG (for cameras that don't pixel-rotate their embedded previews).

## v2.1.0 (2026-03-03)

### New Features
- **Web UI AI auto-tagging** — two new integration points for AI-powered tag suggestions, feature-gated behind `--features ai`:
  - **"Suggest tags" button on asset detail page** — click to analyze the asset image with SigLIP, then review suggested tags as interactive chips with confidence percentages. Accept individual tags (✓), dismiss them (×), or "Accept all" at once. Accepted tags are applied via the existing tag API and appear immediately in the tag list. The button shows "Analyzing..." while the model processes.
  - **"Auto-tag" button in batch toolbar** — select assets in the browse grid and click "Auto-tag" to bulk-apply AI tag suggestions above the configured confidence threshold. A confirmation dialog shows the count of selected assets. Results report how many tags were applied to how many assets. Selection clears and the grid refreshes after the operation.
  - **Lazy model loading** — the SigLIP model and label embeddings are loaded on first request and cached in server memory for the lifetime of the process. Subsequent requests reuse the cached model with no loading delay.
  - **Two new API endpoints** — `POST /api/asset/{id}/suggest-tags` returns JSON suggestions with tag name and confidence score; `POST /api/batch/auto-tag` accepts `{asset_ids}` and returns `{succeeded, failed, tags_applied, errors}`.
  - **Zero impact without AI feature** — when compiled without `--features ai`, the buttons are absent from the UI and the endpoints are not registered. No additional dependencies, no binary size increase.

## v2.0.1 (2026-03-03)

### New Features
- **AI auto-tagging** — `dam auto-tag [--query <QUERY>] [--asset <id>] [--volume <label>] [--threshold 0.25] [--labels <file>] [--apply]` uses SigLIP ViT-B/16-256 (via ONNX Runtime) for zero-shot image classification against a configurable tag vocabulary (~100 default photography categories). Report-only by default; `--apply` writes suggested tags to assets. Feature-gated behind `--features ai` so non-AI users pay zero binary/dependency cost. Model files (~207 MB quantized) downloaded from HuggingFace on first use via `--download`. Model management: `--list-models`, `--remove-model`. Visual similarity search: `--similar <asset-id>` finds the 20 most visually similar assets using stored 768-dim embeddings. Configurable via `[ai]` section in `dam.toml` (threshold, labels file, model directory, prompt template). Supports `--json`, `--log`, `--time`.

### New Modules (ai feature)
- `src/ai.rs` — SigLIP model wrapper: ONNX session management, image preprocessing (256×256 squash resize, normalize to [-1,1]), SentencePiece tokenization (pad to 64), sigmoid scoring (`logit_scale * dot + logit_bias`), ~100 default photography labels.
- `src/model_manager.rs` — Download and cache management for SigLIP ONNX model files from HuggingFace (Xenova/siglip-base-patch16-256).
- `src/embedding_store.rs` — SQLite-backed 768-dim float vector storage with brute-force cosine similarity search.

### Testing
- Added 41 unit tests for AI modules (preprocessing, tokenization, normalization, cosine similarity, embedding store, model manager) and 13 integration tests covering auto-tag dry run, apply, JSON output, custom labels, threshold, similarity search, and non-image skipping.

## v1.8.9 (2026-03-02)

### New Features
- **Export command** — `dam export <QUERY> <TARGET> [--layout flat|mirror] [--symlink] [--all-variants] [--include-sidecars] [--dry-run] [--overwrite]` copies files matching a search query to a target directory. Default exports the best variant per asset in flat layout (filename collisions resolved by appending an 8-character hash suffix). `--layout mirror` preserves source directory structure (multi-volume assets get a volume-label prefix). `--symlink` creates symlinks instead of copies. `--all-variants` exports every variant instead of just the best. `--include-sidecars` also copies recipe files (.xmp, .cos, etc.). `--dry-run` reports the plan without writing. `--overwrite` re-copies even if the target already has a matching hash. Files are integrity-verified via SHA-256 after copy. Supports `--json`, `--log`, `--time`.

### Testing
- Added 5 unit tests for flat-mode filename collision resolution and 12 integration tests covering all export modes (flat, mirror, dry-run, skip existing, overwrite, sidecars, symlink, all-variants, best-variant-only, filename collision, JSON output, no results).

## v1.8.8 (2026-03-02)

### Enhancements
- **Multi-select format filter** — the browse page format filter is now a grouped multi-select dropdown panel instead of a single-select dropdown. Formats are organized by category (RAW, Image, Video, Audio, Other) with group-level "All RAW"/"All Image" toggle checkboxes. Each format shows its variant count. Multiple formats can be selected simultaneously (e.g., all RAW formats, or NEF + TIFF). Trigger button shows compact text: single format name, group name when a full group is selected, or "nef +3..." for mixed selections. Sends comma-separated values to the existing OR filter backend.

## v1.8.7 (2026-03-02)

### New Features
- **Delete command** — `dam delete <ASSET_IDS...> [--apply] [--remove-files]` removes assets from the catalog. Default is report-only mode (shows what would be deleted). `--apply` executes deletion (asset rows, variants, file locations, recipes, previews, sidecar YAML, collection memberships, stack membership). `--remove-files` (requires `--apply`) also deletes physical files from disk. Supports stdin piping (`dam search -q "orphan:true" | dam delete --apply`), asset ID prefix matching, `--json`, `--log`, `--time`.

## v1.8.6 (2026-03-02)

### New Features
- **Incremental verify** — `dam verify --max-age <DAYS>` skips files verified within the given number of days, enabling fast periodic checks on large catalogs. `--force` overrides the skip and re-verifies everything. Configurable default via `[verify] max_age_days` in `dam.toml`.
- **Search negation and OR operators** — prefix any filter or free-text term with `-` to exclude matches (`-tag:rejected`, `-sunset`). Use commas within a filter value for OR logic (`tag:alice,bob`, `format:nef,cr3`, `label:Red,Orange`). Combinable: `type:image,video -format:xmp`.

### Enhancements
- **Recipe verified_at persistence** — verify now persists `verified_at` timestamps to sidecar YAML for both variant locations and recipe locations, so incremental verify works correctly across catalog rebuilds.
- **Show command recipe details** — `dam show` now displays variant hash and volume:path for each recipe, matching the detail level shown for variant locations.
- **Fix orphaned XMP script** — added `--remove` flag to `scripts/fix-orphaned-xmp.py` for deleting the orphaned standalone asset after relocation.

### Bug Fixes
- **Fix verify recipe hash mismatch** — verify was passing the recipe's `content_hash` where the variant's `content_hash` was expected when updating `verified_at`, causing recipe verification timestamps to not persist correctly.

### Testing
- Added 11 new tests covering verify data flows: `is_recently_verified` edge cases, `get_location_verified_at` queries, `VerifyConfig` parsing, and 4 end-to-end integration tests (JSON output, `--max-age` skip, `--force` override, recipe `verified_at` round-trip).

## v1.8.5 (2026-03-01)

### Enhancements
- **Recipe location on detail page** — recipes now show the full volume location (volume label + path) with reveal-in-file-manager and open-terminal buttons, matching the variant location display.
- **Scripting documentation** — new user guide chapter covering bash and Python scripting patterns, jq reporting, workflow automation, and a walkthrough of the `scripts/fix-orphaned-xmp.py` utility script.
- **PDF cross-document links** — internal links between manual chapters now work correctly in the PDF. Previously they pointed to `.md` files; now they resolve to in-document anchors.
- **Fix orphaned XMP script** — new Python utility (`scripts/fix-orphaned-xmp.py`) to relocate XMP sidecar files that were imported as standalone assets instead of being attached as recipes. Supports `--path` scoping for large catalogs and dry-run by default.

## v1.8.4 (2026-03-01)

### Enhancements
- **Tag autocomplete on assignment inputs** — the batch toolbar tag input and the asset detail page tag input now offer autocomplete suggestions from the catalog's tag list as you type. Navigate suggestions with arrow keys, select with Enter or click. Hierarchical tags show their path prefix in muted text. The browse tag filter input already had autocomplete; the batch and detail inputs now share the same tag data.
- **Stale tag list fix** — creating a brand-new tag via batch operations or the detail page now immediately refreshes the autocomplete tag list. Previously, newly created tags only appeared after a full page reload.
- **Browse results loading indicator** — the results grid fades to reduced opacity while page navigation, sorting, or search requests are in flight, giving immediate visual feedback on Shift+arrow page turns and other htmx-driven updates.

## v1.8.3 (2026-03-01)

### New Features
- **EXIF auto-orientation** — preview generation now reads EXIF orientation tags and automatically rotates/flips the image to its correct display orientation. Applies to JPEG, TIFF, and RAW previews (both standard and smart). Previously, images shot in portrait mode could appear sideways in the browse grid and lightbox.
- **Manual rotation** — a "Rotate" button on the asset detail page cycles the preview rotation 90° clockwise (0° → 90° → 180° → 270° → 0°). Rotation is persisted per asset (sidecar YAML + SQLite) and applied on top of EXIF auto-orientation. Both regular and smart previews are regenerated with the new rotation. The rotation state is stored in `preview_rotation` on the asset model.
- **Configurable page size** — the number of results per page in the browse grid is now configurable via `[serve] per_page` in `dam.toml` (default: 60). Also available as `dam serve --per-page N` CLI flag.
- **Page-turn keyboard shortcuts** — Shift+Left/Right arrow keys navigate to the previous/next page in the browse grid and lightbox. In the lightbox, regular arrow keys at page boundaries automatically trigger cross-page navigation with a loading spinner overlay.

### Enhancements
- **Batch operation performance** — batch tag, rating, and label operations now share a single catalog connection, device registry, and content store across all assets instead of opening fresh instances per asset. Batch tagging 30+ assets is now ~10× faster.
- **Batch toolbar feedback** — the batch toolbar shows "Processing N assets..." with a pulsing animation while operations are in progress, instead of silently disabling buttons.
- **Lightbox cross-page loading indicator** — when navigating across a page boundary in the lightbox, a spinner overlay appears and further arrow key presses are blocked until the new page loads.
- **Detail page nav loading indicator** — small spinners appear next to the Prev/Next buttons while adjacent page IDs are being fetched at page boundaries.
- **Preserve selection after batch operations** — batch tag, rating, and label operations no longer clear the selection, allowing multiple operations on the same set of assets.
- **Preview cache freshness** — preview and smart preview HTTP responses now include `Cache-Control: no-cache`, ensuring browsers revalidate after rotation or regeneration instead of serving stale cached images. Combined with `Last-Modified` headers, unchanged previews still get fast 304 responses.
- **Batch operation timing logs** — when `dam serve --log` is enabled, batch operations log timing to stderr (e.g. `batch_tag: 30 assets in 1.2s (30 ok, 0 err)`).

## v1.8.2 (2026-03-01)

### New Features
- **Editable asset date** — set or clear an asset's creation date via CLI (`dam edit --date 2024-12-25` / `--clear-date`) or the web UI (inline date editor on the asset detail page, `PUT /api/asset/{id}/date` endpoint). Updates both sidecar YAML and SQLite catalog.
- **Reveal in file manager** — asset detail page shows a folder icon button (📂) next to each file location on online volumes. Clicking it reveals the file in Finder (macOS), Explorer (Windows), or the file manager (Linux). Backed by `POST /api/open-location` endpoint.
- **Open terminal** — a `>_` button next to the reveal icon opens a terminal window in the file's parent directory (Terminal.app on macOS, cmd on Windows, system terminal emulator on Linux). Backed by `POST /api/open-terminal` endpoint.

## v1.8.1 (2026-03-01)

### New Features
- **Faceted browse sidebar** — a toggleable sidebar on the browse page showing a read-only statistical breakdown of the current result set. Displays distribution counts grouped by rating (with bar chart), color label (with color dots), format, volume, tag (top 30), year (with bar chart), and geotagged asset count. Counts update automatically when search filters change. Each section is collapsible with state persisted in the browser. Hidden by default; toggle via the funnel icon button in the results bar or the `f` keyboard shortcut. Preference persisted in localStorage. Hidden on narrow viewports (<768px). Backed by `GET /api/facets` endpoint running 8 aggregate queries that reuse `build_search_where()` for full filter consistency.

## v1.8.0 (2026-03-01)

### New Features
- **Map view for geotagged photos** — a third browse view mode alongside grid and calendar, showing asset locations on an OpenStreetMap map. Geotagged assets appear as clustered markers with thumbnail popups. All browse filters (tag, rating, label, type, format, volume, collection, path, date) apply to the map. Click a thumbnail to open the lightbox (with full prev/next navigation), click the name/metadata area to go to the detail page.
  - **GPS coordinate extraction** — EXIF GPS data is parsed to decimal degrees during import and stored as denormalized `latitude`/`longitude` columns on the assets table (indexed). Existing catalogs are backfilled automatically on first open.
  - **`geo:` search filter** — `geo:any` (has GPS), `geo:none` (no GPS), `geo:lat,lng,radius_km` (bounding circle), `geo:south,west,north,east` (bounding box). Works in CLI, web UI, and saved searches.
  - **Embedded map libraries** — Leaflet.js 1.9.4 and MarkerCluster 1.5.3 are embedded as static assets (no external CDN dependency). Marker images included for offline use.
  - **Dark mode** — map tiles are inverted for dark theme consistency. Popups and controls adapt to the current theme.
  - **Keyboard shortcut** — `m` toggles map view. View state persists in localStorage.
- **Lightbox standalone mode** — `openWithData()` method allows the lightbox to open with explicit asset data (used as fallback when a map marker's asset is not on the current grid page). The lightbox prefers the normal navigable mode when the card exists in the DOM.

## v1.7.1 (2026-02-28)

### Enhancements
- **Unified browse/lightbox/detail navigation** — clicking the lightbox image opens the detail page; clicking the detail page image opens the lightbox. All three views form a seamless navigation loop with focus tracked via `dam-browse-focus` in sessionStorage. Lightbox open, navigate, and close sync the focused card. Arrow key navigation in lightbox and detail updates which card will be focused on return to browse.
- **Browse state preservation on back-navigation** — scroll position, batch selection, and keyboard focus are now preserved when navigating back from the detail or compare page. Selection is persisted to sessionStorage (`dam-browse-selection`) on `pagehide` and restored on fresh page loads. On bfcache return, the DOM is preserved as-is (no more htmx refresh that was destroying state). Focus is restored from sessionStorage with `scrollIntoView` to approximate scroll position.
- **Compare page Escape fix** — added `preventDefault()` to the Escape key handler on the compare page, fixing unreliable back-navigation that required double-pressing Escape.
- **Cursor feedback** — lightbox and detail page preview images now show `cursor: pointer` to indicate they are clickable navigation targets.

## v1.7.0 (2026-02-28)

### New Features
- **Smart previews** — a second preview tier at 2560px (configurable) for high-resolution offline browsing. Smart previews are stored alongside regular thumbnails in `smart_previews/<hash-prefix>/<hash>.jpg` and enable zoom and pan in the web UI even when the original media volume is offline.
  - **Import `--smart` flag**: `dam import --smart <PATHS...>` generates smart previews alongside regular thumbnails during import. Can also be enabled permanently via `[import] smart_previews = true` in `dam.toml`.
  - **On-demand generation**: Set `[preview] generate_on_demand = true` in `dam.toml` to have the web server generate smart previews automatically when first requested. The first load takes a few seconds (pulsing HD badge shown); subsequent loads are instant.
  - **Manual generation**: "Generate smart preview" button on the asset detail page (`POST /api/asset/{id}/smart-preview`).
  - **Configuration**: `[preview]` section gains `smart_max_edge` (default 2560), `smart_quality` (default 85), and `generate_on_demand` (default false). `[import]` section gains `smart_previews` (default false).
- **Compare view** — side-by-side comparison of 2–4 assets at `/compare?ids=...`. Select assets in the browse grid and click the "Compare" button in the batch toolbar.
  - Synchronized zoom and pan across all columns (toggle with `s` key or checkbox)
  - Interactive rating stars and color label dots per asset
  - Full EXIF display (camera, lens, focal length, aperture, shutter speed, ISO)
  - Keyboard navigation: arrow keys for focus, `d` for detail page, `s` for sync toggle, `0`–`5` for rating, Alt+1–7 for labels, letter keys for labels
  - Smart preview upgrade with HD badge
- **Zoom and pan** — mouse wheel zoom, drag-to-pan, and click-to-toggle (fit ↔ 100%) for smart previews in the lightbox, asset detail page, and compare view. Keyboard shortcuts: `,` (fit), `.` (100%), `+` (zoom in), `-` (zoom out). Zoom is enabled when a smart preview is available.
- **Progressive smart preview loading** — the lightbox and detail page show the regular preview instantly, then background-load the smart preview and swap it in when ready. A pulsing "HD" badge provides visual feedback while the smart preview generates. The badge briefly shows with solid opacity after the smart preview loads as a status indicator.
- **Import `--add-tag` flag** — `dam import --add-tag landscape --add-tag 2026 <PATHS...>` adds tags to every imported asset. Repeatable. Merged with `[import] auto_tags` from config and XMP tags.
- **Asset folder link** — the asset detail page shows clickable links to the folder containing each variant file.

### Bug Fixes
- **generate-previews PATHS mode** — fix fallback to hash-based variant lookup when the file is not on the expected volume, preventing "variant not found" errors for files with valid catalog entries on other volumes.

## v1.6.3 (2026-02-27)

### Enhancements
- **Recipe cleanup during dedup** — when dedup removes a duplicate file location, co-located recipe files (XMP sidecars etc.) in the same directory are automatically cleaned up from disk, catalog, and sidecar YAML. Applies to both `dam dedup --apply` and the web UI's per-location "Remove" and "Auto-resolve" actions. Recipe counts shown in dry-run output and web UI confirm dialog.
- **Dedup prefer config default** — new `[dedup]` section in `dam.toml` with a `prefer` field. Sets a default path substring for the `--prefer` flag in both CLI and web UI. The web UI duplicates page pre-populates a "Prefer keeping" input from config. CLI `--prefer` overrides the config value.
- **Dedup prefer uses substring matching** — the `--prefer` flag now matches anywhere in the relative path (substring) rather than requiring the path to start with the prefix. This correctly handles nested directories like `Session/Selects/photo.nef` when prefer is set to `Selects`.
- **CLI filter flags for duplicates and dedup** — `dam duplicates` gains `--filter-format` and `--path` flags matching the web UI's filter controls. `dam dedup` gains `--filter-format` and `--path` flags to scope dedup operations by file format or path prefix. The `--volume` flag on `duplicates` now uses proper SQL filtering instead of post-filtering.

## v1.6.2 (2026-02-27)

### New Features
- **Duplicates page** — new `/duplicates` page in the web UI showing duplicate file groups with summary cards (total groups, wasted space, same-volume count), mode tabs (All / Same Volume / Cross Volume), and filters (path prefix, format, volume). Per-location "Remove" buttons delete individual file copies from disk. "Auto-resolve" button removes all same-volume duplicates in one click. Each group header shows a clickable preview thumbnail; clicking opens a lightbox overlay with prev/next navigation (arrow keys), keyboard shortcut `d` to open the detail page, and Escape to close. Back/Escape on the detail page returns to the duplicates page.
- **Duplicates dedup API** — `POST /api/dedup/resolve` auto-resolves same-volume duplicates, `DELETE /api/dedup/location` removes a specific file location.

## v1.6.1 (2026-02-26)

### Enhancements
- **Keyboard help panel** — press `?` on any page (or click the "?" button in the nav bar) to see all available keyboard shortcuts. The overlay shows shortcuts organized by category, specific to the current page (browse, lightbox, or asset detail). Press Escape or click outside to dismiss.
- **Detail page navigation** — the asset detail page now has Prev/Next buttons and arrow key navigation for stepping through browse results. Uses sessionStorage for unlimited multi-hop navigation (not limited to one step). Escape and Back return to the browse page with search state preserved.
- **Detail page rating and label shortcuts** — rating (0-5) and color label (Alt/Option+1-7, r/o/y/g/b/p/u/x) keyboard shortcuts now work on the asset detail page, matching browse and lightbox behavior.
- **Lightbox top bar rating and label** — interactive rating stars and color label dots are now always visible in the lightbox top bar, eliminating the need to open the info panel for quick edits.
- **Lightbox/detail page switching** — press `d` in the lightbox to open the detail page; press `l` on the detail page to return to the lightbox at that asset.
- **macOS Option+number fix** — Alt/Option+number shortcuts for color labels now work correctly on macOS (uses physical key codes instead of character values).

## v1.6.0 (2026-02-26)

### New Features
- **Stacks (scene grouping)** — group burst shots, bracketing sequences, and similar-scene images into lightweight anonymous stacks. The browse grid collapses stacks to show only the "pick" image with a count badge, reducing visual clutter. Click the stack toggle (⊞) in the results bar to expand/collapse all stacks globally. Stacks are position-ordered (index 0 = pick), one stack per asset, with auto-dissolve when only one member remains.
  - **CLI**: `dam stack create/add/remove/pick/dissolve/list/show` (alias `st`). Full `--json` support. Stacks persist in `stacks.yaml` and survive `rebuild-catalog`.
  - **Web UI browse**: Stack badge (⊞ N) on cards, colored left border per stack (hue derived from stack ID) for visual grouping, collapse/expand toggle button, "Stack" and "Unstack" batch toolbar buttons.
  - **Web UI asset detail**: Stack members section with thumbnail strip, "Set as pick" and "Dissolve stack" buttons.
  - **Search filter**: `stacked:true` / `stacked:false` to find stacked or unstacked assets.
  - **Calendar**: Respects stack collapse state in heatmap counts.
- **Hierarchical tags** — tags can now contain `/` as a hierarchy separator (e.g. `animals/birds/eagles`). Searching for a parent tag (e.g. `tag:animals`) matches all descendants. The tags page displays a collapsible tree view with own-count and total-count columns. Interoperates with Lightroom's `lr:hierarchicalSubject` XMP field: hierarchical subjects are imported, merged with flat `dc:subject` tags (deduplicating components), and written back on change. Internally stored with `|` as separator to avoid conflicts with literal `/` in tag names.

### Enhancements
- **Tag search with literal slashes** — tags containing literal `/` characters (not hierarchy separators) are now handled correctly in search and web display.

## v1.5.3 (2026-02-25)

### New Features
- **Calendar heatmap view** — the browse page now has a Grid/Calendar view toggle. The calendar view shows a GitHub-style year-at-a-glance heatmap with day cells colored by asset count (quartile-based 5-level scale). Navigate between years with arrow buttons and year chips. Click any day to filter the grid to that date. All existing search filters (tag, rating, label, type, format, volume, collection, path) apply to the calendar aggregation. Includes full dark mode support and `localStorage` persistence for view mode.
- **Date search filters** — three new query filters for filtering assets by creation date:
  - `date:2026-02-25` — prefix match (day, month, or year granularity)
  - `dateFrom:2026-01-15` — inclusive lower bound
  - `dateUntil:2026-02-28` — inclusive upper bound (converted to exclusive internally)
  - All three compose with each other and all existing filters. Available in CLI, web UI (via query input), and saved searches.
- **Calendar API endpoint** — `GET /api/calendar?year=2026` returns JSON with per-day asset counts and available years, respecting all search filter parameters.

## v1.5.2 (2026-02-25)

### New Features
- **Saved search favorites** — saved searches now have a `favorite` field that controls which ones appear as chips on the browse page. Non-favorites are hidden from the browse page but remain accessible via the management page and CLI.
- **Saved searches management page** — new `/saved-searches` page in the web UI provides a table view of all saved searches with star toggle (favorite/unfavorite), rename, and delete actions. Accessible via "Searches" link in the navigation bar and "Manage..." link on the browse page.

### Enhancements
- **Browse page Save button** — now defaults to `favorite: true` so newly saved searches appear immediately as browse chips. Before prompting for a name, checks for duplicate queries and alerts if the search is already saved.
- **CLI `--favorite` flag** — `dam ss save --favorite "Name" "query"` marks a saved search as favorite. `dam ss list` shows `[*]` marker next to favorites.
- **New API endpoints** — `PUT /api/saved-searches/{name}/favorite` toggles favorite status, `PUT /api/saved-searches/{name}/rename` renames a saved search with collision detection.
- **Simplified browse chips** — saved search chips on the browse page are now clean links without inline rename/delete buttons (those moved to the management page).

## v1.5.1 (2026-02-25)

### Performance
- **Database indexes for large catalogs** — added 6 missing indexes on `file_locations(content_hash)`, `file_locations(volume_id)`, `assets(created_at)`, `assets(best_variant_hash)`, `variants(format)`, and `recipes(variant_hash)`. Dramatically speeds up browse, search, stats, and backup-status queries at scale (tested with 150k+ assets, 220k+ variants). Indexes are created automatically on first open after upgrade.
- **Optimized stats and backup-status queries** — consolidated ~20+ sequential SQL queries into ~8 with SQL-side aggregation. Tag frequency counting uses `json_each()` instead of loading all asset JSON into Rust. Directory counting per volume uses SQL `RTRIM` trick instead of loading all file_location rows. Recipe format extraction moved to SQL. Backup-status derives at-risk count from the volume distribution query (eliminating a redundant full scan) and batches per-volume gap queries into a single `GROUP BY`.

### Enhancements
- **Three-state rating filter** — clicking a star in the browse rating filter now cycles through exact match (e.g. "3"), minimum match (e.g. "3+"), and clear. Star 5 remains two-state (5 and 5+ are identical). Makes it easy to filter for exactly 1-star photos for culling.

## v1.5.0 (2026-02-25)

### New Features
- **Dark mode** — the web UI now supports dark mode. Automatically follows the OS/browser preference (`prefers-color-scheme: dark`). A toggle button (sun/moon) in the navigation bar lets you switch manually between light and dark themes. The preference is persisted in `localStorage` and applied instantly on page load (no flash of unstyled content). Covers all pages: browse, asset detail, tags, collections, stats, and backup status.
- **Grid density controls** — three density presets for the browse grid: **Compact** (smaller thumbnails, hidden metadata), **Normal** (default), and **Large** (bigger thumbnails, two-line titles). Toggle buttons with grid icons appear in the results bar next to sort controls. Persisted in `localStorage`. The keyboard navigation column count adjusts automatically.
- **Lightbox viewer** — clicking a thumbnail in the browse grid now opens a full-screen lightbox overlay instead of navigating to the asset detail page. Navigate between assets with on-screen arrow buttons or Left/Right arrow keys. Toggle a side info panel (i key or toolbar button) showing type, format, date, variant count, interactive rating stars, and color label dots. Changes made in the lightbox (rating, label) are written to the API and reflected in the grid behind. Press Escape to close, or click the "Detail" link to open the full asset detail page. Keyboard shortcuts for rating (0-5) and label (r/o/y/g/b/p/u/x, Alt+0-7) work inside the lightbox.

## v1.4.1 (2026-02-25)

### New Commands
- **`dam dedup`** — remove same-volume duplicate file locations. Identifies variants with 2+ copies on the same volume, keeps the "best" copy (by `--prefer` path prefix, verification recency, path length), and removes the rest. `--min-copies N` ensures at least N total copies survive across all volumes. Report-only by default; `--apply` to delete files and remove location records. Supports `--volume`, `--json`, `--log`, `--time`.
- **`dam backup-status`** — check backup coverage and find under-backed-up assets. Shows aggregate overview (totals, coverage by volume purpose, location distribution, volume gaps, at-risk count). `--at-risk` lists under-backed-up assets using the same output formats as `dam search`. `--min-copies N` sets the threshold (default: 2). `--volume <label>` shows which assets are missing from a specific volume. Optional positional query scopes the analysis to matching assets. Supports `--format`, `-q`, `--json`, `--time`.

## v1.4.0 (2026-02-24)

### New Features
- **Volume purpose** — volumes can now be assigned a logical purpose (`working`, `archive`, `backup`, `cloud`) describing their role in the storage hierarchy. `dam volume add --purpose <purpose>` sets purpose at registration, `dam volume set-purpose <volume> <purpose>` changes it later. Purpose is shown in `dam volume list` and included in `--json` output. This metadata lays the groundwork for smart duplicate analysis and backup coverage reporting (see storage workflow proposal).
- **Enhanced `dam duplicates`** — three new flags for targeted duplicate analysis:
  - `--same-volume` — find variants with 2+ locations on the same volume (likely unwanted copies)
  - `--cross-volume` — find variants on 2+ different volumes (intentional backups)
  - `--volume <label>` — post-filter results to entries involving a specific volume
  - Output now shows volume purpose (e.g. `[backup]`), volume count, same-volume warnings, and verification timestamps (in `--format full`)
  - `DuplicateEntry` JSON output includes `volume_count`, `same_volume_groups`, and enriched `LocationDetails` with `volume_id`, `volume_purpose`, `verified_at`
- **`copies:` search filter** — find assets by total file location count. `copies:1` finds single-copy assets (no backup), `copies:2+` finds assets with at least two copies. Same syntax pattern as `rating:`. Works in CLI, saved searches, and web UI.

## v1.3.2 (2026-02-24)

### New Features
- **PDF manual generation** — `doc/manual/build-pdf.sh` script produces a complete PDF manual from the 21 Markdown source files. Renders mermaid diagrams to PNG, generates table of contents, headers/footers with version and date, and per-command page breaks in the reference section. Requires pandoc, XeLaTeX, and mermaid-cli.

### New Commands
- **`dam fix-recipes`** — re-attach recipe files (`.xmp`, `.cos`, etc.) that were misclassified as standalone assets during import. Scans the catalog for assets whose only variant is a recipe-type file, finds the correct parent variant by matching filename stem and directory, and re-attaches them. Dry-run by default (`--apply` to execute).

### Enhancements
- **15 additional RAW format extensions** — added support for `.3fr`, `.cap`, `.dcr`, `.eip`, `.fff`, `.iiq`, `.k25`, `.kdc`, `.mdc`, `.mef`, `.mos`, `.mrw`, `.obm`, `.ptx`, `.rwz` camera formats
- **`import --auto-group`** — after normal import, runs auto-grouping scoped to the neighborhood of imported files (one directory level up from each imported file). Avoids catalog-wide false positives from restarting camera counters. Combines with `--dry-run` and `--json`.

## v1.3.1 (2026-02-24)

### New Features
- **`dam fix-dates` command** — scan assets and correct `created_at` dates from variant EXIF metadata and file modification times. Fixes assets imported with wrong dates (import timestamp instead of capture date). Re-extracts EXIF from files on disk for assets imported before `date_taken` was stored in metadata. Backfills `date_taken` into variant source_metadata on apply so future runs work without the volume online. Reports offline volumes clearly with skip counts and mount instructions. Dry-run by default (`--apply` to execute). Supports `--volume`, `--asset`, `--json`, `--log`, `--time`.

### Enhancements
- **Import date fallback chain** — import now uses EXIF DateTimeOriginal → file modification time → current time (previously fell through to current time when EXIF was missing, causing many assets to get the import timestamp as their date)
- **Second variant date update** — when a second variant joins a stem group during import, if it has an older EXIF date or mtime than the asset's current `created_at`, the asset date is updated
- **EXIF `date_taken` stored in source_metadata** — DateTimeOriginal is now persisted in variant source_metadata as `date_taken` (RFC 3339), enabling `fix-dates` and future date-aware features to work from metadata alone

## v1.3.0 (2026-02-23)

### New Features
- **Comprehensive user manual** — 21 markdown files in `doc/manual/` covering every command, filter, and configuration option, organized into User Guide (7 workflow chapters), Reference Guide (10 man-page style command docs), and Developer Guide (3 pages: REST API, module reference, build/test)
- **9 Mermaid diagrams** — ER diagrams, architecture layers, round-trip workflow, XMP sync sequence, import pipeline, auto-group algorithm, maintenance cycle, data model, and module dependency graph
- **7 web UI screenshots** — browse page, saved search chips, asset detail, batch toolbar, tags page, collections page, and catalog structure
- **README Documentation section** — links to all three guide sections

## v1.2.0 (2026-02-23)

### Enhancements
- **Browse grid deduplication** — assets with multiple variants (e.g. RAW+JPEG) now appear as a single card in the browse grid instead of one card per variant. Implemented via a denormalized `best_variant_hash` column on the `assets` table, computed at write time using the same Export > Processed > Original scoring as preview selection. Search queries with no variant-level filters skip the `variants` JOIN entirely for faster queries.
- **Primary format display** — browse cards now show the asset's identity format (e.g. NEF, RAF) instead of the preview variant's format (JPG). A denormalized `primary_variant_format` column prefers Original+RAW, then Original+any, then the best variant's format.
- **Variant count badge** — browse cards show a variant count badge (e.g. "3v") when an asset has more than one variant, making multi-variant assets visible at a glance.
- **`dam serve --log`** — the global `--log` flag now enables request logging on the web server, printing `METHOD /path -> STATUS (duration)` to stderr for each HTTP request.

## v1.1.1 (2026-02-23)

### Enhancements
- **`path:` filter normalization** — the `path:` search filter now accepts filesystem paths in the CLI: `~` expands to `$HOME`, `./` and `../` resolve relative to the current working directory, and absolute paths matching a registered volume's mount point are automatically stripped to volume-relative with the volume filter implicitly applied. Plain relative paths (no `./` prefix) remain volume-relative prefix matches as before.

## v1.1.0 (2026-02-23)

### New Features
- **Export-based preview selection** — previews now prefer Export > Processed > Original variants for display. RAW+JPEG assets show the processed JPEG preview instead of the flat dcraw rendering. Affects `dam show`, web UI asset detail page, and `generate-previews` catalog mode.
- **`generate-previews --upgrade`** — regenerate previews for assets where a better variant (export/processed) exists than the one currently previewed. Useful after importing exports alongside existing RAW files.

## v1.0.0 (2026-02-23)

First stable release. All planned features are implemented, all tests pass, documentation is complete. Ready for production use.

### Highlights

- **22 CLI commands** covering the full asset management lifecycle: import, search, browse, edit, group, relocate, verify, sync, refresh, cleanup, and more
- **Web UI** with search, filtering, inline editing, batch operations, keyboard navigation, saved searches, and collections
- **Bidirectional XMP sync** with CaptureOne, Lightroom, and other photo editing tools
- **Content-addressable storage** with SHA-256 deduplication and integrity verification across multiple offline volumes
- **Stem-based auto-grouping** for RAW+JPEG+sidecar bundles, with fuzzy cross-directory grouping for exports

### Changes since v0.7.1

- Add 10 integration tests (group, fix-roles, refresh, edit --label)
- Complete documentation: architecture overview, component specification, specification
- Move specification into doc/ directory

## v0.7.1 (2026-02-23)

### New Features
- **`dam fix-roles` command** — scan multi-variant assets and re-role non-RAW variants from Original to Export when a RAW variant exists. Fixes assets imported before the auto-grouping role fix. Dry-run by default (`--apply` to execute). Supports `--volume`, `--asset`, `--json`, `--log`, `--time`.
- **Import auto-grouping role fix** — newly imported RAW+non-RAW pairs now correctly assign Export role to non-RAW variants (previously both were marked Original)

## v0.7.0 (2026-02-23)

### New Features
- **`dam auto-group` command** — automatically group assets by filename stem across directories, solving the problem where CaptureOne exports land in different directories than their RAW originals. Uses fuzzy prefix + separator matching (e.g., `Z91_8561.ARW` matches `Z91_8561-1-HighRes-(c)_2025_Thomas Herrmann.tif`). Chain resolution ensures multiple export levels all group to the shortest root stem. RAW files are preferred as the group target; donors are re-roled from Original to Export. Dry-run by default (`--apply` to execute). Supports `--json`, `--log`, `--time`.
- **"Group by name" batch button** in web UI — select assets on the browse page and click "Group by name" to auto-group them by filename stem with a confirmation dialog

### Bug Fixes
- **`group` now preserves recipes** — merging donor assets into a target now copies recipe records, preventing recipe loss on `rebuild-catalog`
- **`group` re-roles donor variants** — donor variants with role "original" are changed to "export" in both sidecar YAML and SQLite catalog, correctly reflecting their derived status

## v0.6.4 (2026-02-22)

### Improvements
- **Auto-search on all filter changes** — removed the explicit Search button; text inputs (query, path) auto-search with 300ms debounce, dropdowns (type, format, volume, collection) trigger immediately on change, matching the existing behavior of stars, labels, and tags

## v0.6.3 (2026-02-22)

### New Features
- **`path:` search filter** — filter assets by file location path prefix (e.g., `path:Capture/2026-02-22`), with quoted value support for paths with spaces; works in CLI, web UI (dedicated input in filter row), and saved searches
- **Grouped `--help` output** — CLI help now groups commands logically (Core, Organization, Maintenance, Output) for easier discovery

## v0.6.2 (2026-02-22)

### New Features
- **Collection filter dropdown** in browse page filter row — collections are now composable with all other search filters (tag, rating, type, format, volume) directly from the browse page
- Batch toolbar collection buttons now sync from the filter-row dropdown instead of URL params

## v0.6.1 (2026-02-22)

### New Features
- **Collection removal** from web UI — asset detail page shows collection membership chips with × remove buttons
- **Collection creation** from web UI — `/collections` page with "+ New Collection" button

## v0.6.0 (2026-02-22)

### New Features
- **Saved searches** (smart albums) — `dam saved-search` (alias `ss`) with save, list, run, delete subcommands; stored in `searches.toml`; web UI chips on browse page with rename/delete on hover
- **Collections** (static albums) — `dam collection` (alias `col`) with create, list, show, add, remove, delete subcommands; SQLite-backed with YAML persistence; search filter `collection:<name>`; web UI batch toolbar integration
- **Quoted filter values** — search parser supports double-quoted values for multi-word filters (`tag:"Fools Theater"`, `collection:"My Favorites"`)

### Bug Fixes
- Fix saved search chip hover showing rename/delete buttons incorrectly

## v0.5.1 (2026-02-22)

### New Features
- **Import `--dry-run` flag** — preview what an import would do without writing to catalog, sidecar, or disk
- **Inline name editing** in web UI — pencil icon toggle, text input with Save/Cancel

## v0.5.0 (2026-02-22)

### New Features
- **Keyboard navigation** on browse page — arrow keys navigate cards (column-aware), Enter opens detail, Space toggles selection, 1–5/0 set/clear rating, Alt+1–7/0 set/clear color label, letter keys r/o/y/g/b/p/u/x for quick label

## v0.4.5 (2026-02-21)

### New Features
- **`dam refresh` command** — re-read metadata from changed sidecar/recipe files without full re-import; supports `--dry-run`, `--json`, `--log`, `--time`

## v0.4.4 (2026-02-21)

### New Features
- **Color labels** — first-class 7-color label support (Red, Orange, Yellow, Green, Blue, Pink, Purple); XMP `xmp:Label` extraction, CLI editing (`dam edit --label`), web UI color dot picker, browse filtering, batch operations, XMP write-back
- **Batch operations** in web UI — multi-select checkboxes, fixed bottom toolbar with tag add/remove, rating stars, color label dots
- **Keyboard shortcut hints** — platform-aware Cmd/Ctrl labels on toolbar buttons

### Bug Fixes
- Fix Ctrl+A not working after checkbox click
- Remove unreliable shift-click range selection, replace with Cmd/Ctrl+A

## v0.4.3 (2026-02-21)

### New Features
- **Description XMP write-back** — description changes written back to `.xmp` recipe files on disk
- **Inline description editing** in web UI — pencil icon toggle, textarea with Save/Cancel

## v0.4.2 (2026-02-20)

### New Features
- **Tag XMP write-back** — tag changes written back to `.xmp` recipe files using operation-level deltas (preserves tags added independently in CaptureOne)

## v0.4.1 (2026-02-20)

### New Features
- **Rating XMP write-back** — rating changes written back to `.xmp` recipe files on disk, enabling bidirectional sync with CaptureOne

### Bug Fixes
- Fix back button and reload showing raw HTML instead of full browse page
- Refresh browse results when returning via back button (bfcache)

## v0.4.0 (2026-02-20)

### New Features
- **Browse page redesign** — sort controls (Name/Date/Size with direction indicators), top pagination, star rating filter (click stars for minimum threshold)

### Bug Fixes
- Fix rating loss on pagination when sort changes

## v0.3.5 (2026-02-20)

### New Features
- **Tags page enhancements** — sortable columns (name/count), live text filter, multi-column CSS layout
- **`dam update-location` command** — update file path in catalog after manual moves on disk

## v0.3.4 (2026-02-20)

### New Features
- **Extended `dam cleanup`** — now removes orphaned assets (all variants have zero locations) and orphaned preview files, in addition to stale location records
- **Search location health filters** — `orphan:true`, `missing:true`, `stale:N`, `volume:none`

## v0.3.3 (2026-02-20)

### New Features
- **`dam cleanup` command** — remove stale file location records for files no longer on disk

## v0.3.2 (2026-02-20)

### New Features
- **`dam sync` command** — reconcile catalog with disk after external file moves, renames, or modifications

## v0.3.1 (2026-02-20)

### New Features
- **`dam edit` command** — set or clear asset name, description, and rating from CLI
- **Photo workflow integration proposal** — documented gaps and planned features for CaptureOne integration

## v0.3.0 (2026-02-20)

### New Features
- **Version display** in web UI navigation bar

## v0.2.0 (2026-02-19)

### New Features
- **Web UI** (`dam serve`) — browse/search page with filter dropdowns, asset detail page, tag editing, rating support
- **First-class rating** — `Option<u8>` field on Asset with CLI search, web UI stars, XMP extraction
- **Stats page** in web UI with bar charts and tag cloud
- **Tags page** in web UI
- **Multi-tag chip input** with autocomplete on browse page
- **Metadata search** with indexed columns and extended filter syntax (camera, lens, ISO, focal, aperture, dimensions)
- **Info card previews** for non-visual formats (audio, documents) and as fallback for missing external tools
- **`dam.toml` configuration** — preview settings, serve settings, import exclude/auto_tags
- **`--log` flag** on `generate-previews` for per-file progress

### Bug Fixes
- Fix multi-component ASCII EXIF fields (Fuji lens_model parsing)

## v0.1.0 (2026-02-18)

### New Features
- **`dam init`** — initialize catalog with SQLite schema, volume registry, config
- **`dam volume add/list`** — register and list storage volumes with online/offline detection
- **`dam import`** — SHA-256 hashing, EXIF extraction, stem-based auto-grouping, recipe handling, duplicate location tracking, preview generation
- **`dam search`** — text, type, tag, format filters
- **`dam show`** — full asset details with variants, locations, metadata
- **`dam tag`** — add/remove tags
- **`dam group`** — manually merge variant assets
- **`dam duplicates`** — find files with identical content across locations
- **`dam generate-previews`** — thumbnails for images, RAW (dcraw/LibRaw), video (ffmpeg)
- **`dam rebuild-catalog`** — regenerate SQLite from YAML sidecars
- **`dam relocate`** — copy/move assets between volumes with integrity verification
- **`dam verify`** — re-hash files to detect corruption or bit rot
- **Output formatting** — `--json`, `--format` templates, `-q` quiet mode, `-t` elapsed time
- **XMP metadata extraction** — keywords, rating, description, color label, creator, rights
