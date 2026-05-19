# Changelog

All notable changes to the Digital Asset Manager are documented here.

## v4.5.16 (2026-05-19)

Asset detail page: pending_writeback markers now update immediately
after any rating/tag/description/label edit. Previously the recipes
block was server-rendered once on page load and never refreshed, so
the ↻ icons and the `pending` summary badge stayed stale until the
user manually reloaded the page.

Fix structure:

1. **New partial template** `templates/recipes_fragment.html` carries
   the existing `<details>` block markup. `RecipesFragment` in
   `src/web/templates.rs` builds it from `AssetDetails` using the
   same recipe-grouping logic as the inline render.
2. **New endpoint** `GET /api/asset/{id}/recipes-fragment` returns
   that partial.
3. **Asset detail page** wraps the recipes block in `<div
   id="recipes-block" hx-get="…/recipes-fragment"
   hx-trigger="pending-changed from:body" hx-swap="innerHTML">`. Initial
   render stays inline (no flash of loading state); subsequent
   updates hit the partial endpoint.
4. **Edit endpoints** (`set_rating`, `set_description`, `set_name`,
   `set_color_label`, `add_tags`, `remove_tag`, `clear_tags`) now
   append `HX-Trigger: pending-changed` to their successful responses
   via a shared `with_pending_trigger` helper. htmx picks up the
   header and fires the event on `document.body`; the recipes block
   listens and refreshes.
5. **Writeback button** (`writebackAsset` JS) no longer reloads the
   whole page on success — it fires the same `pending-changed` event
   so the markers clear in place and the button itself goes away
   when the refreshed partial reports nothing pending.

Set-date doesn't trigger pending writeback (it's not an XMP-written
field), so its response is unchanged. The tags-page endpoint and
global tag mutations (rename/split/delete) also keep their original
responses — they're not invoked from an asset detail page where the
trigger would have an effect.

Tests unchanged: 813 + 252 standard, 933 + 285 pro. The change is
template wiring + a response header; verifying it via the CLI test
harness would require spinning up the axum router and reading the
response headers, which doesn't fit our current test infrastructure.
Manual verification: open an asset, edit the rating, watch the ↻
markers appear in the recipes section without a page reload.

## v4.5.15 (2026-05-19)

Three related fixes to the XMP-roundtrip stack, surfaced by real-world
testing of v4.5.14:

1. **`insert_recipe` now preserves `pending_writeback`** (the root
   cause of YAML/SQLite divergence). The `INSERT OR REPLACE` was
   omitting that column, so every catalog write (rebuild-catalog,
   reimport, sync) silently reset the flag back to the schema default
   (0) — even when the in-memory recipe (loaded from YAML) carried
   `pending_writeback: true`. Symptom: `maki writeback --asset <id>`
   reports "0 written" while the YAML sidecar still has
   `pending_writeback: true`, the web UI shows no pending indicator,
   and the actual edit never makes it to disk. The fix adds
   `pending_writeback` to the column list in `Catalog::insert_recipe`
   so state survives every catalog write.

2. **Sidecar-wins gate extended to `reimport_metadata_inner`** — the
   path behind `maki refresh --reimport` and the web UI's "Reload
   metadata" button. v4.5.14 added this gate to the two refresh-media
   loops but missed the third copy of the same iterate-JPEG-locations-
   and-read-embedded-XMP pattern. Without the gate, "Reload metadata"
   re-injected the JPEG's stale flat keywords just like the unfixed
   refresh path used to. Same `Catalog::asset_has_recipe` helper, now
   called at all three sites; the docstring lists them so a future
   fourth site is easy to wire correctly.

3. **`maki writeback --force` flag** (and matching web Maintain
   dialog checkbox) — escape hatch for the case where the catalog's
   pending state is wrong (cleared by a stale `insert_recipe`, or
   simply not flagged because you want to re-canonicalise after an
   upgrade). Different from `--all`: `--all` expands the scope to
   every recipe in the catalog; `--force` keeps the explicit scope
   (`--asset`, query, or `--volume`) but ignores the pending-flag
   filter. So `maki writeback --asset 016cc7dd --force` re-writes
   the two recipes attached to that asset regardless of pending state.

### Operator note — repairing existing diverged state

If you're upgrading from a release where `insert_recipe` had already
clobbered SQLite's pending flags, run `maki rebuild-catalog --asset
<id>` (or catalog-wide `maki rebuild-catalog`) once after upgrading.
The rebuild reads from YAML (source of truth) and re-inserts every
recipe with the fixed code path, so the pending state from YAML now
lands in SQLite correctly. After that, plain `maki writeback` will
find the pending recipes again. Alternatively, `maki writeback
--asset <id> --force` re-writes regardless of state and clears
pending in both stores.

Tests: 813 + 252 standard, 933 + 285 pro (+1 lib for `insert_recipe`
regression, +1 CLI standard for reimport, +2 CLI pro for `--force` +
the pro-gated reimport coverage).

### Regression coverage

- `catalog::tests::insert_recipe_preserves_pending_writeback_flag` —
  inserts a recipe with `pending_writeback: true`, calls
  `insert_recipe` again with the same id, asserts SQLite still reports
  pending. Verified to FAIL on v4.5.14 (clobber to 0) and PASS on
  v4.5.15 (state survives).
- `refresh_reimport_skips_embedded_xmp_when_sidecar_present` — builds
  a JPEG with embedded `StaleEmbeddedTag` keyword, imports it
  alongside an empty `.xmp` sidecar, runs `maki refresh --reimport
  --asset <id>`, asserts the stale keyword does NOT come back.
  Mirrors the existing `refresh_media_skips_when_sidecar_present`
  fixture, exercising the third leak site.
- `writeback_force_rewrites_non_pending_recipe` (pro) — imports an
  asset with a non-pending recipe, runs plain `maki writeback --asset
  <id>` (asserts "0 written" — no-op), then `--force` (asserts
  "already in sync" — opt-in path triggered). Locks the semantics:
  `--force` keeps the explicit scope, drops the pending filter.

## v4.5.14 (2026-05-19)

Closes two XMP-roundtrip leaks that let non-hierarchical tags reappear
in the catalog after a clean writeback. Symptom: user reports flat
keywords (`Bavaria`, `Fools Theater`, …) coming back into the asset's
tag list on the next `refresh --media`, even after `tag rename` /
`tag clear` cycles had moved them into the canonical hierarchy.

Both leaks share a root cause: a `.xmp` sidecar is supposed to be the
single source of truth for an asset's metadata, but two parallel paths
were sneaking around it.

Tests: 812 + 251 standard, 932 + 283 pro (+6 lib / +1 CLI per build).

### Namespace-URI-aware XMP `hierarchicalSubject` writeback

XMP is keyed by namespace URI, not prefix string. The Adobe Lightroom
namespace (`http://ns.adobe.com/lightroom/1.0/`) can be bound to any
prefix via an `xmlns:` declaration — most files use `lr:`, some older
CaptureOne / third-party exports use `lightroom:`, and nothing in the
spec stops a tool from inventing its own. `lr:hierarchicalSubject` and
`lightroom:hierarchicalSubject` are *the same XMP field*.

`update_hierarchical_in_string` in `src/xmp_reader.rs` previously
matched on the literal prefix `lr:` only. Writeback patched the `lr:`
block and left any parallel `lightroom:` block intact. On the next
`refresh --media` the reader (which already uses prefix-agnostic
`local_name()`) absorbed *both* blocks, and the legacy block's
flat-leaf entries (`Bavaria`, `Fools Theater`, …) merged back into
`asset.tags`.

Fix: the writer now scans `xmlns:X="…/lightroom/1.0/"` declarations,
collects every prefix bound to the Lightroom namespace (always
including `lr` and `lightroom` as fallbacks), builds a dynamic regex
from that set, and canonicalises the file to exactly one `lr:` block.
Multiple blocks → tags are union-merged → single `lr:` block replaces
all of them. Exotic prefixes are detected and collapsed. Byte-equivalence
is preserved for the common case (single canonical `lr:` block, no
edits — the fast path edits in place, doesn't re-render).

Regression tests in `xmp_reader::tests`:
- `collect_lightroom_prefixes_finds_alien_bindings` — `lr` + `lightroom`
  + `lrc` all detected from one XMP.
- `update_hierarchical_collapses_dual_namespace_blocks` — the real-world
  bug fixture (both `lr:` pipe-paths and `lightroom:` flat leaves
  present) → exactly one `lr:` block survives, with merged contents.
- `update_hierarchical_collapses_alien_prefix` — exotic `lrc:` block
  canonicalised to `lr:`; `xmlns:lr=` injected if not previously
  present.
- `update_hierarchical_canonical_lr_only_is_byte_stable` — the no-op
  case returns input bytes unchanged (verifies the fast path didn't
  introduce SHA drift on writebacks that should be idempotent).

### Sidecar-wins precedence for embedded-XMP on `refresh --media`

`refresh --media` (and `sync-metadata --media` Phase 3) iterates every
JPEG/TIFF file location for every asset and feeds the embedded XMP
through `reapply_xmp_data`. That function is documented as add-only —
"Keywords: merge (add new; cannot remove since we don't track
provenance)". Combined with the fact that MAKI's writeback only patches
the `.xmp` sidecar (it never modifies embedded XMP inside JPEG/TIFF
containers), this meant: for any asset with both a master + a JPEG
variant, the JPEG's embedded XMP was *frozen* at import time, and
every `refresh --media` re-injected its stale flat keywords into
`asset.tags`. Once in, they could never leave (no provenance).

`merge_hierarchical_keywords` suppresses flat keywords that are
*components* of hierarchical paths, so identical-spelling collisions
(`Bayern` in dc:subject + `…|Bayern|…` in hierarchicalSubject) were
already filtered out. But spelling drift (`Bavaria` flat vs `Bayern`
canonical, `Fools Theater` flat vs `FoolsTheater` canonical) slipped
straight through because the strings aren't equal.

Fix: new `Catalog::asset_has_recipe(asset_id)` helper in
`src/catalog/recipe_query.rs` — `SELECT COUNT(*) FROM recipes JOIN
variants WHERE asset_id = ?1`. Both refresh-media loops in
`src/asset_service/refresh.rs` now call it after resolving the owning
asset for a variant location, *before* extracting embedded XMP. If
true, the file is reported as `RefreshStatus::SidecarPresent` (new
enum variant) and skipped entirely.

The rule: **the `.xmp` sidecar is authoritative; embedded XMP in any
variant is potentially stale and ignored when a sidecar exists**. If
an asset has no sidecar at all (rare — happens on imports that opted
out of sidecar creation), the embedded-XMP path still runs unchanged.

`--log` output labels skipped files as `skipped (sidecar present)`
on the CLI and `sidecar-present` in the web Maintain SSE stream, so
the new behaviour is auditable on the first run after upgrade.

Integration test `refresh_media_skips_when_sidecar_present` in
`tests/cli.rs` builds a real JPEG with embedded XMP carrying a
`StaleEmbeddedTag` keyword, imports it alongside an empty `.xmp`
sidecar, runs `tag clear` to remove the keyword from the catalog,
then runs `refresh --media` and asserts the keyword does NOT come
back. Unit tests in `catalog::tests` cover the new query in both
directions (asset with recipe → true; asset with only variants → false).

### Roadmap (deferred)

Same namespace-URI canonicalisation could be applied to the other
multi-prefix-in-the-wild XMP namespaces (`dc:`, `digikam:` /
`digiKam:`, `MicrosoftPhoto:` / `mp:`). Not done in this release —
the Lightroom namespace was the one actively causing user-visible
data drift; the others are theoretical risks today.

## v4.5.13 (2026-05-19)

Docs-only release. Zero code changes — the binary is byte-equivalent
to v4.5.12 modulo the version string. Cut as a separate release so
the GitHub release page surfaces the refreshed VLM model guide as a
shipped artifact rather than a `main`-branch-only doc update.

Tests: 806 + 250 standard, 926 + 282 pro (unchanged from v4.5.12).

### VLM model guide refresh + Gemma 4 family

`doc/manual/reference/10-vlm-models.md` rewritten with practical
decision frameworks rather than feature-list-first model catalogues:

- **New "Should I Upgrade?" section** answering the question
  directly: stay on your current model if it works for navigation;
  upgrade for generic / repetitive descriptions, wrong subject ID,
  OCR, or multilingual needs. Names three concrete upgrade paths
  from popular defaults — `gemma3:4b` → `gemma4:e4b`,
  `qwen2.5vl:3b` → `qwen3-vl:8b`, `moondream` → `gemma4:e2b`.

- **New "Testing a Candidate Model on Your Own Photos" workflow** —
  concrete 3-step A/B recipe using the existing per-asset model
  selector and the `--force` re-describe flag. The honest way to
  decide a model swap is to compare on photos the user actually
  cares about; this makes the workflow obvious.

- **Gemma 4 family added** to all three Tested Models tables
  (Recommended / Budget / Large) with Ollama tag names, effective
  parameter counts, RAM, download size, and latency estimates:
  - `gemma4:e2b` (2.3 B effective, text+image+audio) — small/fast
  - `gemma4:e4b` (4.5 B effective, text+image+audio) — direct
    upgrade path for `gemma3:4b` users
  - `gemma4:26b` (25.2 B total / 3.8 B active MoE, text+image)
  - `gemma4:31b` (30.7 B dense, text+image)

  Plus the gotcha that `-mlx-bf16`, `-mxfp8`, and `-nvfp4` quant
  variants are text-only and silently drop image input — stick
  with default tags or `-it-q4_K_M` / `-it-q8_0` quants for
  `maki describe`.

- **MLX backend note** under the Ollama section: recent Ollama
  releases bundle Apple's MLX framework as the default inference
  path on Apple Silicon. Transparent on the HTTP API — no MAKI
  config changes — but the same MLX-only-tags-are-text-only
  gotcha applies.

- **Per-model config example expanded** with an annotated
  multi-model setup explaining what each override is for
  (`timeout` for cold-start loading, `num_ctx` for thinking
  models, `temperature` for batch consistency, `max_image_edge`
  for throughput, `max_tokens` for response length cap).

- **Lead paragraph** adds an explicit "Model swap is pure config"
  note — zero code changes needed to try a new model. The model
  name is a string in `[vlm].model`; MAKI is just an HTTP client
  to Ollama / OpenAI-compatible APIs.

- **Qwen3.5 section softened** — dropped unverifiable "early
  fusion" architectural claims and the stale "as of March 2026"
  Ollama caveat date. Replaced with a how-to-spot-broken-vision
  smoke test that stays useful regardless of which Ollama
  version the user is on.

By-use-case and by-hardware decision tables also updated to
include Gemma 4 placements at every RAM tier.

## v4.5.12 (2026-05-18)

Tag-data hygiene + browse-page selection safety release. One real
performance win, one real bug, one new CLI tool, and two browser-side
UX changes around the persistent selection state.

Tests: 806 + 250 standard, 926 + 282 pro (+8 / +0 / +8 / +0 from v4.5.11
— eight new lib tests for the stray-`|` normaliser cases and one for the
new Rust-side leaf-detection algorithm).

### Tags page: Rust-side leaf detection (~3× faster)

`list_leaf_tag_counts` previously ran a `NOT EXISTS` subquery against
`json_each(a.tags)` per (asset, tag) pair — O(N × M²) over a virtual
table SQLite can't index. On a 90 k-asset catalog the tags page hit
~1 s; on a 260 k-asset / 4.5 k-tag catalog ~1 s as well.

Replaced with a streaming Rust-side computation. A single
`SELECT a.id, je.value FROM assets a, json_each(a.tags) AS je WHERE …
ORDER BY a.id, je.value` hands the rows back already lex-sorted within
each asset; a per-asset accumulator + linear walk decides leaf-ness via
a one-step `starts_with` check on the next tag (the lex-sort guarantees
any descendant `T|...` is the immediately-following entry, so a single
peek suffices). Same signature, same return type, case-sensitivity
preserved (Rust's `str::starts_with` is byte-exact, matching the SQL
version's `GLOB` rather than `LIKE`).

Measured: ~360 ms on a 90 k catalog (down from ~1 s), ~900 ms on a 260 k
catalog (down from ~980 ms — the larger catalogs scale closer to linear
in N because the second `json_each` no longer multiplies in). Regression
test `list_leaf_tag_counts_classifies_per_asset_correctly` locks four
shapes: deep hierarchy with single leaf, hierarchy with sibling leaves,
mixed case (`color` + `Color|red` — case-sensitive prefix means
lowercase `color` stays a leaf), and flat-only tag sets.

### Tag normalisation: strip stray `|` markers + plug XMP-import gap

A user found a tag ` |München` in their catalog — leading whitespace,
then `|München`. The `|`-prefix-anchor search syntax meant
`tag:|München` matched 12 k legitimately hierarchical tags ending in
`|München` too, hiding the one offender in a giant result set.

Two gaps, both closed:

1. `normalize_tag_for_storage` previously stripped whitespace and
   control chars but accepted leading / trailing / consecutive `|`
   separators verbatim. Now also:

   - Strips leading and trailing `|` (anonymous root parent / trailing
     child — semantically meaningless).
   - Drops empty middle segments (`foo||bar` → `foo|bar`).
   - Trims whitespace around each `|` segment (`foo | bar` →
     `foo|bar`).
   - Drops the tag entirely if it's only `|`s with nothing else.

2. `apply_xmp_data` and `reapply_xmp_data` (the XMP-keyword ingestion
   sites — used by `maki import`, `maki refresh --media`, and the new
   `sync --apply` media-modified path) used to push merged keywords
   into `asset.tags` verbatim, bypassing the normaliser that
   `maki tag` and the web UI's tag input both go through. Both now
   route every XMP-sourced keyword through `normalize_tag_for_storage`.

Seven new unit tests in `tag_util::tests` lock the new normaliser
behaviour (leading pipe, leading whitespace + pipe, trailing pipe,
empty middle segment, all-pipes, whitespace around pipes, idempotent
on already-clean hierarchies).

### `maki tag scan` — find assets with malformed tags

The shell incantation `maki search 'tag:"= |München"'` works for
some malformed tags but not when the "whitespace" isn't an ASCII space
(NBSP U+00A0, tab, zero-width space, ogham, etc. — all valid Unicode
whitespace, all things Lightroom / CaptureOne export quirks can emit).

New command:

```bash
maki tag scan          # human-readable
maki tag scan --json   # machine-readable
```

Walks every asset's tag list and reports any tag value that doesn't
survive a round-trip through `normalize_tag_for_storage`. For each
offender, the output names the asset, dumps the raw tag with its byte
sequence in hex (so the whitespace flavour is self-diagnosing), and
shows the canonical form the normaliser would produce:

```
b6598546-2eb6-4126-b3e7-a6dbad5faee1  (unnamed)
    raw:  " |München"
    hex:  20 7c 4d c3 bc 6e 63 68 65 6e
    fix:  "München"
```

Read-only. Pair with `maki tag fix-unicode --apply` (for NFC/NFD),
`maki tag rename` / `tag delete --apply` (for targeted edits), or
edit the sidecar by hand and reload via `maki rebuild-catalog --asset
<id>` (existing command, single-asset rebuild from the corrected
sidecar).

Reference manual gains a `maki tag scan` section with the full
behaviour catalogue and example output.

### Browse: persistent off-page selection indicator

User reported accidentally tagging assets they'd forgotten were still
selected from a previous visit. The selection state persists across
navigation by design, but the visual cue when returning to the browse
page was identical regardless of whether the selection was new or
carried over — the warning surface disappeared the moment the user
looked away.

Two-part fix:

1. **One-shot pulse** on the count badge when the page loads with a
   selection that came from a non-detail navigation (Tags / Stats /
   Maintain / …). Attention-grabber on landing. Detail-page round
   trips don't pulse — that's the normal browsing flow.
2. **Persistent off-page indicator** in the batch toolbar. A yellow
   `⚠ N selected off this page` pill is visible whenever at least one
   selected asset isn't currently rendered as a card on the current
   page. Recomputed on every selection toggle AND every htmx swap of
   `#results` (pagination, filter change, sort), so the count stays
   accurate as you page around with a persistent selection. The batch
   count itself ("12 selected") also tints yellow when the condition
   holds — count + pill read as one warning chunk.

The pill is condition-driven, not event-driven: no click-to-dismiss.
It clears automatically the moment the off-page count drops to zero
(you page to where the selection lives, clear it, or deselect
everything that isn't visible). This is the protection against the
real danger case — coming back to a browse page where *none* of the
selected assets are visible, where the previous one-shot pill would
disappear after the first click.

## v4.5.11 (2026-05-17)

Workflow-driven release focused on the catalog↔disk reconciliation paths.
Three sync/writeback enhancements driven by real-world friction during a
~90k-asset curating session, one preview-regeneration fix from a stitched-
panorama DNG, and a latent Windows path-handling bug that the new sync
test surfaced.

Tests: 798 + 250 standard, 918 + 282 pro (up from 798 / 249 / 918 / 279).

### Sync now handles re-exported media at the same path

`maki sync --apply` used to error with `Hash mismatch at <path>: expected
A, got B` and refuse to touch the catalog whenever a media variant file
(JPEG, TIFF, …) was overwritten with different content at its existing
path — the typical CaptureOne / Lightroom re-export case. The error
accumulated in `result.errors` and nothing in the catalog moved, leaving
the user with no in-MAKI way to fix it.

`sync --apply` now mirrors the recipe-modified handling, adapted for the
content-addressed variant model. For each modified media file:

- Drops the stale `file_location` row at this path for the old hash.
- Inserts a full variant for the new hash (not a stub), inheriting role +
  original_filename from the previous variant. Format is re-derived from
  the file extension; file_size from disk.
- **EXIF / dimensions / embedded-XMP are extracted from the fresh bytes**
  via the same `exif_reader::extract` + `embedded_xmp::extract_embedded_xmp`
  paths `maki import` uses. A re-export usually carries updated exposure
  / develop settings, and dimensions can shift if the export size changed
  — both matter for facet queries.
- **Previews are regenerated** in the same pass, keyed by the new hash, so
  the asset is immediately viewable. No separate `maki refresh --media`
  or `maki generate-previews` step.
- Asset sidecar is updated: previous variant loses this location, new
  variant added alongside.

The previous variant stays in the catalog with whatever other locations
it had (a backup volume that hasn't been rsynced yet still references the
previous bytes). If it becomes truly locationless, `maki cleanup --apply`
removes it and any orphaned preview keyed by the old hash.

Without `--apply`, sync now reports `modified` for these files (same
counter as recipes use) instead of pushing a confusing "Hash mismatch"
error.

### Writeback self-reconciles when XMP already matches catalog

`maki writeback` walks pending recipes and writes the catalog's current
values to each XMP. `xmp_reader::update_*` was already idempotent at the
field level (skip if no change), but when every field returned
`Ok(false)` — typical case after an external rsync from a primary volume
to a registered backup volume — the catalog's stored `content_hash` for
the recipe stayed stuck at the pre-rsync value. A subsequent
`maki verify --volume Archive-Backup` would flag every rsync'd recipe as
a hash mismatch.

`writeback_process` now re-hashes the file on the no-write path and
updates the catalog's stored hash if it drifted. The pending flag clears
as before. New summary counter `already_in_sync` distinguishes these
recipes from genuinely-written ones — the user's 90k-recipe rsync-then-
writeback case now reports `0 written, 90000 already in sync` instead
of misleadingly claiming 90000 file writes.

Surfaces in four places: CLI summary (`0 written, 90000 already in sync`),
`--log` per-file lines (`X.xmp — already in sync (no write)`), `--json`
(new `already_in_sync` field), and the web Maintain dialog's writeback
toast.

### `[writeback] mirror_tags` config option

When tag renames / splits / deletes / fix-unicode operations accumulate
in the catalog before the next flush, the additive default writeback
writes the new tags but leaves the old keywords stranded in XMP. A
subsequent re-import absorbs them back into the catalog and silently
undoes the rename. The user hit this on a 90k-image curating session
followed by a re-import.

New config slot:

```toml
[writeback]
mirror_tags = true   # default: false
```

When `true`, every `maki writeback` (CLI) and every web Maintain →
Writeback run defaults to mirror semantics — XMP keyword lists stay in
lock-step with the catalog. No `--mirror-tags` flag needed.

Default `false` keeps backward compatibility for mixed-tool workflows
(Lightroom / CaptureOne / Bridge writing their own keywords MAKI
shouldn't strip). CLI `--mirror-tags` and the dialog checkbox still
work and OR with the config.

### Preview regeneration: 2 GiB image cap + visible errors

A user hit a silent failure regenerating previews of a stitched-panorama
DNG. Two underlying problems, both fixed:

- `generate_raw` used `image::load_from_memory()` and `image::open()` on
  dcraw / dcraw_emu output with the image crate's default 512 MiB
  allocation cap. Stitched-panorama DNGs (Capture One in this case) embed
  a full-resolution preview JPEG rather than a thumbnail, and the
  half-size TIFF dcraw_emu emits can easily exceed 512 MiB at panorama
  dimensions. Both decode sites now use the same 2 GiB cap that
  `generate_image` and the AI preprocess path already had.
- The web `generate_preview` handler propagated regen errors via `?`,
  returning 500. htmx silently drops 5xx response bodies, so the user
  got "internal server error" with no detail. Errors now flow through
  `PreviewFragment.error` and render inline in the existing
  `.preview-error` block alongside any previews still on disk.

### Sync path-separator fix on Windows

`sync_inner` built each disk file's `relative_path` via
`strip_prefix(mount_point).to_string_lossy()`. On Windows that returns
backslash-separated; the catalog stores every `file_locations.relative_path`
through `FileLocation::relative_path_str()`, which always emits forward
slashes. So on Windows: sync looked up `exports\IMG_001.jpg`, catalog
had `exports/IMG_001.jpg`, comparison missed, the file was classified as
"new" and the catalog row as "missing".

Pre-existing tests didn't catch this — the recipe-modified test used a
root-level file with no separator; the move test used subdirs but
exercised the move-detection code path, which still functionally worked
(at the cost of silently writing backslashes into the catalog whenever a
Windows user moved a subdir'd file). The new media-modified test was the
first sync test that combined a subdir layout with the same-path-different-
hash branch, which actually needs the path lookup to succeed.

One-line fix at the strip_prefix call site normalizes `\` → `/`. All
downstream comparisons against catalog-stored paths now line up; move
detection on Windows for subdir'd content also writes correctly-
normalized paths into the catalog going forward.

## v4.5.10 (2026-05-15)

Browse-page QoL release. Two features and one small fix on top of v4.5.9, all browser-side.

Tests: 798 + 249 standard, 918 + 279 pro (unchanged from v4.5.9 — all changes are JS/CSS or simple server-side routing).

### Remember the last filter across visits

Curating sessions where the user bounces between detail and grid views previously dropped them at page 1 of the default view every time they returned to `/`. They had to retype the query (or save it as a saved-search first). Now the browse page persists `{search, page, ids, ts}` to `localStorage` on every `htmx:afterSwap`, and a bare-`/` visit restores it.

Page restoration is asset-ID-aware, not naive: on restore, `/api/all-ids?<saved-search>` returns the current ordered ID list under the saved filter+sort, and the script walks the remembered IDs to find the first one still present. The current page is computed from that ID's position. Result: returning to a half-curated set lands on the same view even when intervening retags or deletions changed the result count. Falls back to page 1 if every remembered ID is gone.

A "Restored from last session" pill appears next to the saved-search chips on the visit that triggered the restore (then auto-strips via `history.replaceState` so a refresh doesn't re-flash it). A `× Clear` button next to the Save-search button wipes both the active filter and the saved state, landing on the default view.

Gated by `[browse] remember_latest_filter = true` (default on). Surfaced through `/api/build-info` alongside the existing slideshow toggles. `/api/all-ids` now also returns `per_page` so the JS computes pages directly instead of inferring from totals.

### Per-chip negate toggle on tag filter

Each tag chip on the browse filter bar now carries a `+`/`−` toggle on its left that flips the chip between include (default) and exclude. Negated chips route into the search's `tags_exclude` (NOT) clause instead of `tags`, equivalent to typing `-tag:foo` in the query field but reachable directly from the chip — useful for the common "show me everything under `subject|animal` *except* cat" pattern.

Wire format: leading `-` on the chip's `data-tag` value (outermost prefix, before the existing `=` / `/` / `^` mode/case markers). The form's `getSelectedTags().join(",")` carries `data-tag` through to the URL's `tag=` param; `build_parsed_search` now detects per-item leading `-` and pushes into `parsed.tags_exclude`. `SearchOptions.tags_exclude` is the same slot the `parse_search_query` path uses for `-tag:foo` in the q field — so this is a routing change in `build_parsed_search`, not a new filter type.

Visually distinctive when active: a filled red badge with a white minus inside (reads as a "no-entry" stop icon, stays legible against the chip's red-tinted background); the chip's whole background gains a red tint; the label gets a strikethrough. Three cues so the polarity reads from across the page, not just from the small toggle. `stripTagPrefixes` parses leading `-` so URL→chip restore (initial load AND popstate) round-trips correctly.

### apply-vocabulary.py: zsh-safe quoting

`scripts/apply-vocabulary.py` was emitting bare `=label` tokens for the `OLD_TAG` argument of `maki tag rename`. zsh's `EQUALS` option (on by default, default shell on macOS) treats an unquoted token starting with `=` as a command-path lookup — `=cat` expanded to `/bin/cat`, breaking the emitted invocation. Added a `shell_quote()` wrapper that wraps `shlex.quote()` and always single-quotes tokens starting with `=`, regardless of whether POSIX shell rules would require it. Output is now `maki tag rename '=cat' 'subject|animal|domestic'` — zsh leaves the `'=cat'` literal alone.

## v4.5.9 (2026-05-14)

Bug-fix and quality-of-life patch on top of v4.5.8's Review-tags feature.

Tests: 798 + 249 standard, 918 + 279 pro (up from 798 / 249 / 918 / 277).

### Writeback: clear pending_writeback in YAML on successful flush

The web UI's "Write back to XMP" button and the `maki writeback` CLI were leaving the YAML sidecar's `pending_writeback: true` flag in place after successfully flushing an online recipe — even though SQLite and the XMP file on disk were both correctly in sync. A subsequent `maki rebuild-catalog`, which trusts YAML as the source of truth, would silently reintroduce a phantom pending flag on a recipe whose disk file is already current.

The v4.5.6 fix correctly tracked per-recipe success in a `cleared_recipe_ids: HashSet<String>` and gated the after-loop sidecar save on it — but kept a `&& r.pending_writeback` guard inside the save loop. The `file_changed` branch in the per-recipe loop had already pre-cleared the in-memory flag (so the sidecar save would carry the new state), and the stale guard then saw the already-cleared value, set `any_changed = false`, and skipped `store.save()`. Net effect on a successful writeback: SQLite cleared ✓, XMP updated ✓, YAML stuck on `pending_writeback: true` ✗.

Fix: drop the stale guard. `cleared_recipe_ids.contains(...)` is already the authoritative "this recipe succeeded" signal, and any recipe in that set warrants a sidecar save (the in-memory recipe has both an updated content_hash and a cleared pending flag that need to land on disk). New regression test `writeback_clears_yaml_pending_flag_on_success` reproduces the bug — verified to FAIL without the fix.

### Writeback: name offline volumes in the skipped summary

When a recipe is skipped because its volume is offline, the summary only said `N skipped` — the user had to guess (or grep `--log` output) which drive to reconnect. The summary now lists volume labels:

```
Writeback: 0 written, 1 skipped (offline volumes: Archive 2025)
```

`WritebackResult` gains a `skipped_offline_volumes: BTreeSet<String>` field populated with deduped, sorted volume labels of every offline volume that held at least one skipped recipe. Surfaced in four places: CLI summary, `--log` per-file lines (`X.xmp — skipped (volume offline: Archive 2025)`), `--json` (new optional `skipped_offline_volumes` array field), and the web Maintain dialog's writeback toast.

### `scripts/apply-vocabulary.py` — flat-to-hierarchical tag migration

New maintenance helper that reads the AI vocabulary YAML and emits `maki tag rename` / `maki tag split` commands to convert flat catalog tags into their hierarchical home per the vocabulary mapping. Driven by the same `my-labels.yaml` format used by `[ai].labels`:

```yaml
sunset:                                       # → emits `tag split`
  - subject|nature|sky
  - technique|lighting|golden hour
concert: subject|performing arts|concert      # → emits `tag rename`
landscape: landscape                          # skipped (identity)
weather: null                                 # skipped (no mapping)
```

Two recurring situations this catches:

1. **Vocabulary growth.** Adding `dog: subject|animal|domestic` only steers *future* AI suggestions — assets previously tagged with the bare `dog` keep the flat form. The script aligns them on demand.
2. **Inbound imports.** Sidecar XMP from Lightroom / CaptureOne typically arrives flat (`sunset`, `concert`, `landscape`). The script promotes them to your canonical hierarchy in one pass.

Dry-run by default. Three vocab sources: explicit YAML file argument, `--default` (built-in), or no args (active `[ai].labels`). Ships its own minimal vocab parser so there's no PyYAML dependency on the user's box. Rename commands use the `=` whole-path marker so the move only touches the bare flat form — `something|label` collisions on the same leaf stay intact. Documented in [Maintenance → Aligning Flat Tags to the AI Vocabulary](doc/manual/user-guide/07-maintenance.md#aligning-flat-tags-to-the-ai-vocabulary-pro).

### Asset detail page: real tooltip on pending-writeback marker

The ↻ glyph next to a recipe with staged-but-unflushed XMP edits had `cursor:help` plus a `title` attribute. Browsers do show that title — but with a 1–2 s delay that users routinely mistake for "no tooltip", especially since the cursor change implies help is imminent. Replaced with a CSS-driven instant tooltip (visible on hover or keyboard focus) plus an `aria-label` for screen readers. Text expanded from "Pending writeback" to a one-sentence explanation of what the marker means and how to flush it ("Write back to XMP" button below, or `maki writeback`).

### Cross-facet flat-tag writeback contract lock

Added regression test `writeback_flat_tags_cover_all_facets` that creates an asset with hierarchical tags across four facets (subject, person, event, location), runs `maki writeback`, and asserts every flat component lands in `dc:subject` and every hierarchical path lands in `lr:hierarchicalSubject`. The flattener in `query.rs` already iterates all facets via `.flat_map(|t| t.split('|'))`, but the contract had no test guarding against future facet-name special-casing. This test now fails CI if anyone introduces such a filter.

## v4.5.8 (2026-05-14)

A focused web-UI release: **Review tags…** — an interception point in batch auto-tagging that surfaces every AI-suggested tag as a per-tag candidate set the user can prune before any tag is written. Solves the workflow complaint that motivated v4.5.7's hierarchical mapping: even with clean hierarchical suggestions, false positives still accumulate across dozens of vacation shots and the only remedy was hand-cleaning afterwards. Now the user reviews once, per tag, and false positives never land on disk.

Tests: 798 + 249 standard, 918 + 277 pro (up from 797 / 249 / 915 / 277).

### Review tags — per-tag candidate review

A new **Review tags…** button on the browse toolbar (next to Auto-tag). On click, operates on the current selection if any, otherwise on every asset matching the live filter (resolved via the existing `/api/all-ids`).

Flow:

1. The button kicks off a new background job (`POST /api/maintain/suggest-tags-review`, `JobKind::SuggestTagsReview`) that loops every asset in scope and aggregates per-asset SigLIP suggestions into an inverted `tag → [(asset_id, confidence, source_label, existing)]` index. The collection pass runs at half the configured `[ai].threshold` so the review modal's confidence slider has range both above and below the user's normal auto-tag floor.
2. The review modal opens **immediately** on POST return — in a "computing" phase with a progress bar that subscribes to the job's SSE stream. The model-load wait (30–60 s on first request) is labelled explicitly so the user knows the system isn't hung. Per-asset ticks update the bar live.
3. On the terminal `done` event, the modal transitions to a two-pane review UI: tag list on the left (sorted by candidate count, each row showing the count + a mean-confidence bar), thumbnail grid on the right (pre-checked, with confidence overlay and a "from: skyscraper →" caption when the SigLIP label maps non-identity to the hierarchical tag). A threshold slider auto-toggles cells by confidence; a "skip already-tagged" filter hides assets that already carry the tag (or a descendant).
4. **Apply** commits via the existing `POST /api/batch/tags` with `{asset_ids, tags: [tag], remove: false}` — no new commit endpoint. The row strikes through, the next un-reviewed tag opens. **Skip** advances without writing. **Close** exits the modal; the job keeps running server-side but the in-memory result drops on close (re-attach is out of scope for v1 — re-run if needed).

The toast in the corner is intentionally suppressed for this job kind. The modal **is** the feedback surface — a small toast in the corner is too easy to miss when the AI model can take a full minute to warm up before the first per-asset tick arrives.

### Internal refactor: SuggestContext

The historical `suggest_tags_inner` (single-asset web endpoint) and `batch_auto_tag_inner` (multi-asset web endpoint) duplicated ~150 lines: model load, label encoding with cache check, per-asset image resolution, image encoding, embedding persistence, classification, vocabulary mapping, existing-tag check. Both are now thin wrappers over a `SuggestContext` that hoists the per-batch setup (model guard, encoded labels, vocabulary, threshold, online volumes) and exposes a single `suggestions_for(asset_id)` method that runs the per-asset loop body. The single-asset web endpoint builds a context for one asset; `batch_auto_tag_inner` loops and applies non-existing suggestions; `batch_suggest_tags_review_inner` loops and aggregates into the inverted index.

A new `suggestions_with_meta_for` variant also returns the asset's preview URL + filename (derived from the same `engine.show()` call) so the review modal can render thumbnails without a second DB round-trip per asset.

### Generic `Job.result` payload

The existing `JobRegistry` already supported live progress via SSE and re-attachable per-job status, but the suggest-tags-review aggregate (a tag → candidates index, ~10–50 KB for typical batches) is too structured to ship through the per-tick SSE stream. `Job` gains an optional `result: Mutex<Option<serde_json::Value>>` field that producers populate before `finish` and a new `GET /api/jobs/{id}/result` endpoint returns it:

- `200` with JSON body when a payload is set.
- `204 No Content` when the job completed without setting a payload.
- `425 Too Early` when the job is still running.
- `404` when the job ID isn't in the registry.

Generic by design — any future job that produces structured terminal output can use the same plumbing (e.g. a hypothetical "find duplicates in current selection" job that returns a cluster graph).

### Misc

- `Job.set_result_round_trips_payload` regression test.
- Two `TagBucket` unit tests covering mean/max confidence accumulation and the empty-bucket case.
- Tagging guide gains a new "Reviewing suggestions before committing (web UI)" subsection under the *Phase 5: Enrich* section.

## v4.5.7 (2026-05-12)

A mixed release. Headline feature is **hierarchical AI tagging** — the SigLIP zero-shot classifier still produces flat labels under the hood, but a new vocabulary YAML format maps each label to the canonical hierarchical tag(s) MAKI applies, eliminating the post-suggestion cleanup pass users had been doing by hand. Headline fix is the **`encode_texts` chunking that stopped Suggest tags from OOM-killing the server** on real-world custom vocabularies. Plus comment-preserving config saves, a writeback bugfix, and several smaller refinements.

Tests: 797 + 249 standard, 915 + 277 pro (up from 794 / 249 / 901 / 277).

### AI vocabulary with hierarchical mapping

The SigLIP zero-shot classifier is inherently flat — it scores an image against bare labels like "sunset" or "wedding". Users who maintain a hierarchical tag taxonomy (the tagging guide's whole pitch — `lighting|sunset`, `event|wedding`, …) had to manually rename every "new" tag the AI applied. Reported workflow: run Suggest tags, get 20 suggestions back, spend 5 minutes converting each one.

A new YAML format for `[ai] labels` carries both the labels the model sees AND the hierarchical tag(s) to suggest when a label scores. Detected by extension on load:

```yaml
# my-labels.yaml
sunset:
  - subject|nature|sky
  - technique|lighting|golden hour
concert: subject|performing arts|concert
dog: subject|animal|domestic
abstract: subject|concept|abstract
weather: null     # null = leave flat, no canonical mapping
```

Keys are the labels (single source of truth — no separate `my-labels.txt` to keep in sync). Values: a single hierarchical tag, a list (one-to-many fan-out — `sunset` lands on BOTH the sky subject and the golden-hour lighting in one suggestion), or `null` (leave the label flat). Apply step fans out + dedups by max confidence so two flat labels that map to the same tag don't double-suggest.

A new module `src/ai_vocabulary.rs` carries the loader and apply step. The flow runs for both `POST /api/asset/{id}/suggest-tags` and `POST /api/batch/auto-tag`, and CLI `maki auto-tag` goes through the same path via `AssetService::auto_tag` — same hierarchy-aware result at both surfaces.

`SuggestTagsResponse` gains a `source_label: Option<String>`. When the vocabulary applied a non-identity mapping, the dropdown shows a small "(from sunset)" caption next to the hierarchical tag so the user sees what the AI actually classified the image as underneath the applied tag.

Built-in default vocabulary `src/default-vocabulary.yaml` maps the 96 default photographic labels into the canonical hierarchy documented in the tagging guide. A new strict consistency test `default_vocabulary_targets_exist_in_canonical_hierarchy` walks the AI vocabulary, takes each non-identity mapping target, and asserts it appears in the parsed-canonical set from `crate::vocabulary::parse_vocabulary(default_vocabulary())`. This locks the contract: any future edit that introduces a mapping not in the canonical hierarchy fails CI.

Backward compat is total. Three concentric layers:

1. No `[ai].labels` set → built-in default labels with the default hierarchical mapping (new).
2. `[ai].labels = "x.txt"` → load txt as flat list, identity mapping (today's behaviour, unchanged).
3. `[ai].labels = "x.yaml"` → load vocabulary + mapping (new).

The label cache (`state.ai_label_cache`) gains a labels-equality check so editing the vocabulary mid-session invalidates the cached embeddings instead of silently mis-indexing.

### `maki ai export-vocabulary`

New CLI command group `maki ai` for AI utilities (only subcommand so far is `export-vocabulary`):

```bash
# Fork the built-in default vocabulary to start your own
maki ai export-vocabulary --default > my-labels.yaml

# Migrate an existing legacy .txt file to YAML (preserves order)
maki ai export-vocabulary > my-labels.yaml

# Save anywhere via --output
maki ai export-vocabulary --default --output ~/photos/vocab.yaml
```

Parallels `maki tag export-vocabulary` for the catalog tag taxonomy — same `--default` flag shape, same stdout-by-default + `--output FILE`. When the active vocabulary is a `.yaml` file, output is verbatim (preserves user comments + key order). When it's a `.txt` file, it's converted to YAML with identity mappings (`label: null`) as a migration template.

### `subject|vehicle` promoted to its own facet

Following the tagging guide's "Thinking in facets" principle: vehicles aren't always urban (a tractor in a field, a fishing boat at sea, an airplane in flight). The default catalog vocabulary loses `subject|urban|transport` and gains a top-level `subject|vehicle` facet with seven leaves: `car, truck, motorcycle, bicycle, train, airplane, boat`. The default AI vocabulary re-points `car`/`bicycle`/`train`/`airplane`/`boat` accordingly. `road` stays on `subject|urban|street` (road infra is street, not vehicle).

`vehicle` is the one species-deep branch in the default — most others stop at genus level. The tagging guide's "When AI specificity meets your catalog hierarchy" section explains the reasoning so users can apply the same logic to their own catalogs.

User data is unaffected — catalogs already using `subject|urban|transport|*` keep working unchanged. The change is purely to the default scaffold for new catalogs and to where new AI suggestions land.

### Suggest-tags safety nets

A user report: Suggest tags hanging for 90+ seconds, then `Failed to fetch` in the browser, then `zsh: killed` on the CLI. Process RSS hit 95 GB before macOS jetsam killed it. Three findings:

- **`encode_texts` was running the entire vocabulary through ONNX in a single batch.** For a 322-label custom vocabulary on SigLIP 2 Large multilingual (1024-dim, 24+ transformer layers) with the CoreML execution provider on Apple Silicon, intermediate attention activations balloon proportional to `batch * seq_len² * heads * layers`. The 322-label batch pushed the process past 90 GB resident. **Fix: chunk into 32-label mini-batches.** Output is bit-identical (each label still passes through the same encoder; only the batch-dimension grouping changes). Per-call ONNX allocation drops by an order of magnitude.

- **`image::ImageReader::no_limits()` removed the crate's 512 MiB decode cap in the AI preprocess path,** so an unexpectedly large outlier image (200 MP film scan, malformed file) could OOM the process during decode-before-resize. **Fix: bounded 2 GiB cap via `image::Limits`.** Medium-format 16-bit TIFFs (~600 MB decoded) still work; outliers fail with an actionable error pointing at `maki generate-previews --upgrade`.

- **The suggest-tags hot path had no progress markers,** so when CoreML compilation or label encoding took 60+ seconds the user saw nothing on stderr — couldn't tell whether the process was stuck or working. **Fix: phase timing prints behind `--verbose`** at every boundary (image resolved, model lock acquired, model loaded, labels encoded, image encoded, classified). When the request hangs, the last line printed before the kill names the responsible phase.

### Settings save: preserve comments + per-key formatting

Opening the Settings dialog and clicking Save without changing anything used to rewrite `maki.toml` — comments above sections gone, every default-valued field spelled out, the user's notes on which SigLIP variant is active dropped. Cause: the save path ran the typed `CatalogConfig` through `toml::to_string_pretty`, which round-trips data correctly but doesn't preserve formatting metadata.

The web save path now uses a field-level diff against the on-disk representation, applied to a `toml_edit::DocumentMut` parsed from the existing file. `DocumentMut` preserves comments, blank lines, key order, single vs double quotes — every formatting choice the user made by hand. The diff walks both `new_config` and `current_config` as `serde_json` trees and only touches keys whose values actually differ. Touched keys get value-replaced in place (the key's decor stays); untouched keys are never visited.

A no-op save now round-trips byte-identical to the input — three new regression tests pin this contract:

- `save_with_comments_no_op_is_byte_identical`
- `save_with_comments_single_field_change_keeps_comments`
- `save_with_comments_optional_clear_removes_line`

`CatalogConfig::save` itself is untouched — still used by `maki init` and any CLI path that writes a fresh file. Only the Settings dialog's save endpoint flows through the new function.

### Settings form: pristine-state tracking per widget

Follow-up to the comment-preserving save. The form was still adding entries to `[import.profiles.<name>]` sub-tables on no-op saves — most visibly default-`false` values for the Option<bool> fields (`embeddings`, `descriptions`, `smart_previews`). Cause was JS-side: rendering an `Option<bool>=None` produces an unchecked checkbox that looks identical to `Option<bool>=Some(false)`. The widget's collector then always returned `cb.checked` → so an untouched checkbox-rendered-from-None reported `false` on submit, the server-side diff saw `None → Some(false)` as a real change, and the toml_edit pass dutifully added `embeddings = false` to the section. Same shape for Option<Vec<String>>=None rendered as an empty CSV input.

Each widget builder (enum select, bool checkbox, number input, text input, string-list input) now captures the `current` value at render time and only flips a `dirty` flag when the user fires an actual input/change event. The collector returns `originalValue` while pristine, and only computes from form state once the user has touched it. `undefined` (key absent from the loaded JSON) is preserved as `undefined` so `JSON.stringify` drops it from the request — `#[serde(default)]` then fills in `Vec::new()` / `false` / `None` on the server, matching the original "absent" state exactly.

### Writeback: don't clear pending flag on skipped recipes

`maki writeback` was force-clearing `pending_writeback` on every recipe in the per-asset process set after the inner per-recipe loop ran, without distinguishing recipes that actually completed from recipes that hit `continue` for an offline volume or missing file. The SQLite-side clear inside the loop was correctly gated (`continue` skipped it), but the YAML sidecar's clear loop afterwards iterated `recipe_entries` blindly and zeroed every flag — so the sidecar diverged from the DB on multi-variant assets with one offline variant.

User-visible symptom: an asset with a variant on an unplugged drive would lose its `pending_writeback: true` mark in the YAML sidecar even though the actual write got skipped. A subsequent `rebuild-catalog` from YAML would then silently lose the staged edit.

Fix tracks per-recipe success in a `cleared_recipe_ids: HashSet<String>` populated only when a recipe reaches the success path, and the sidecar-save loop now only touches recipes in that set. Skipped recipes keep their pending state in both layers so the next `maki writeback` picks them up when the volume comes back online.

`--all`, `--mirror-tags`, and the web `POST /api/maintain/writeback` all flow through the same `writeback_process` and are covered by the same fix. New regression test `writeback_preserves_pending_when_volume_offline` registers a removable volume, edits, renames its mount to take it offline, runs writeback, restores the mount, and asserts both `maki status` and the YAML sidecar still report pending.

## v4.5.6 (2026-05-11)

A bug-fix patch. One regression in the writeback flow: `maki writeback` was clearing the YAML sidecar's `pending_writeback` flag on recipes it had actually *skipped* (offline volume, missing file), so a `rebuild-catalog` from YAML would silently lose those staged edits.

Tests: 794 + 249 standard, 901 + **277** pro (one new regression test, `writeback_preserves_pending_when_volume_offline`).

### Writeback: pending flag preserved on skipped recipes

`writeback_process` had two flag-clear paths: a per-recipe `clear_pending_writeback()` inside the per-recipe loop (correctly gated — `continue` for offline/missing files skipped it) and a YAML-side sidecar clear AFTER the loop that iterated the asset's full `recipe_entries` list and zeroed every flag regardless. So on a multi-variant asset with one offline variant, the SQLite row for the skipped recipe stayed correct (`pending_writeback = true`) but the YAML sidecar's recipe entry got cleared. The two layers diverged. Practical impact: a subsequent `maki rebuild-catalog` (which rebuilds the catalog from YAML, the source of truth) would silently drop the staged edit, because YAML claimed everything had been flushed.

Fix: track per-recipe success in a `cleared_recipe_ids: HashSet<String>` populated only when a recipe reaches the success path, and have the sidecar-save loop honour that set instead of the broader `recipe_entries`. Skipped recipes now keep `pending_writeback: true` in both layers, so the next `maki writeback` picks them up when the volume comes back online.

Coverage check while in there:

- The four inline writeback paths (`write_back_rating_to_xmp_inner` and siblings for tags / description / label) were already per-recipe-safe via `mark_recipe_pending` / `clear_recipe_pending` helpers — they touch only the specific recipe's flag, no cross-contamination. No change needed.
- `[writeback] enabled = false` semantics still match the v4.5.0 design (the config flag governs only *automatic* flush on every edit; `is_writeback_enabled()` is checked inside the `_inner` methods, which treats "off" the same as "offline": mark pending, skip the file write). `writeback_process()` itself has no enabled-gate — the explicit manual flush always runs, as documented.
- `--all` and `--mirror-tags` flow through the same `writeback_process()` so they're covered by the same fix.
- The web `POST /api/maintain/writeback` route calls `engine.writeback()` directly — also covered.

## v4.5.5 (2026-05-11)

A web-UI release: `maki.toml` becomes editable in the browser via a schema-driven Settings dialog, the Maintain dialog grows the same forms → progress → result lifecycle the Import dialog already had, and a `sync --apply --remove-stale` + tag-rename race no longer aborts bulk tag operations.

Tests: 794 + 249 standard, 901 + 276 pro (up from 790 / 249 / 897 / 276 — four new config-round-trip regression tests on each build flavour). No engine signature changes; the only Rust-side behaviour shift is config-serialisation determinism (always emit every section, in declaration order).

### Settings dialog

New top-nav entry **Settings** (next to Maintain) opens a modal that renders every option in `maki.toml` as a form widget. The form is generated from the `CatalogConfig` JSON Schema (via the `schemars` 0.8 crate, with `JsonSchema` derived on every config type) so a new option in `src/config.rs` shows up in the dialog automatically — no separate UI registration step.

Widget mapping per type:

- `bool` → checkbox
- `i*` / `u*` → number input (with `step="any"` so direct typing of floats / step-mismatched values isn't dropped)
- `f64` → number input
- `String` → text input
- `String` with an `enum` derive on the type → `<select>` of variants
- `Vec<String>` → comma-separated text input
- nested struct → collapsible `<details>` section with a TOML-style title (`[preview]`, `[vlm]`, …)
- `BTreeMap<String, T>` → one editable sub-section per existing key (e.g. `[import.profiles.hinterhalt]`, `[vlm.model_config."qwen2.5vl:3b"]`)

Three endpoints back the dialog:

- `GET /api/config` — current values + raw `maki.toml` body
- `GET /api/config/schema` — `schemars::schema_for!(CatalogConfig)` output
- `POST /api/config` — validate by deserialising the submitted JSON back into `CatalogConfig` (serde catches type errors and unknown keys), then save. The previous file gets copied to `maki.toml.bak` before each save so a bad edit is recoverable.

Restart-required hints come from a `needs_restart(old, new)` server-side classifier that knows which options are bound at startup (`[serve].port`, `[preview].max_edge`, `[ai].model`, …). Touched fields show a small **restart required** pill next to the label; the post-save toast is amber when any restart-required field changed and green when everything took effect immediately.

UX:

- Lives in its own modal, **not** a Maintain tab — Maintain operations and Settings have very different cadences (one-off long-running ops vs. routine config tweaks) and bundling them together made the Maintain modal feel overloaded.
- **Save button focused on render** so `Enter` after a single edit commits the change without a mouse hop.
- **Floating toast** for save feedback (reusing the `.job-toast` chrome) sits bottom-right so the user sees confirmation even if they Escape out of the dialog before the response lands.
- **Escape** closes the dialog (with the dialog's own keydown handler scoped to its own open state so it doesn't fight the Maintain dialog's identical handler when both are wired into the same document).

#### Adding / renaming hashmap keys stays manual

Hashmap-typed config (`[import.profiles.<name>]`, `[vlm.model_config."<model>"]`) renders each existing key as an editable sub-section, but **adding or renaming entries is `maki.toml`-only** for now. Rationale: the schema can't express which keys are legal — VLM model names must match what's loaded on the Ollama side, profile names are referenced from `import --profile <name>` invocations — and a free-form name field would be too easy to get out of sync. The form shows a hint to that effect at the bottom of each hashmap section.

#### Config serialisation determinism

Pre-v4.5.5 each section in `CatalogConfig` had `skip_serializing_if = "is_default_*"` so an unmodified default section was omitted from the saved file. That made the on-disk file shorter but had two failure modes the Settings dialog exposed:

1. Saving from the Settings dialog with no changes would silently omit explicit-default values like `[writeback] enabled = false`, leaving the user uncertain whether the click took effect.
2. Once any field in a previously-skipped section was customised, that section's position in the file changed (skipped sections leave a gap), making the file diff-noisy under version control across saves of the same config.

Now every section is always emitted in declaration order. Files are slightly longer but byte-stable across no-op saves and across machines. Three new regression tests (`save_load_save_is_byte_identical`, `explicit_false_default_round_trips`, `float_values_round_trip_cleanly`) lock the invariants in.

Same release also switches `f32` → `f64` on every config float field. `f32` values round-tripped through TOML serialisation gain trailing-precision noise (`0.1` → `0.10000000149011612`) because the f32 → f64 widening conversion when TOML's parser reads back the saved string isn't a clean inverse. `f64` throughout the config eliminates the drift; cast back to `f32` happens only at the boundaries that need it (VLM payloads, ONNX inputs, contact-sheet margins).

`BTreeMap` (not `HashMap`) for `import.profiles` and `vlm.model_config` so the saved file lists entries in alphabetical order, deterministic across saves — `HashMap` iteration is randomised, which made the file diff-noisy under version control between two saves of the same data.

### Maintain dialog: forms → progress → result phases

The Maintain modal previously closed itself on submit and tracked the running job only through the small nav-bar toast. The Import dialog, by contrast, keeps its modal open with a larger progress area until the job finishes. Inconsistent — and the toast's terse comma-joined summary lost detail for ops with many counters (cleanup, sync, refresh).

The Maintain dialog now uses the same three-phase lifecycle as Import:

- **forms** — the seven tab forms (initial state and after Close on a finished job).
- **progress** — visible while a job is running. Status line + last 50 log lines + a "Minimize to toast" button.
- **result** — terminal summary as a counter list (one line per non-zero counter — `12 written`, `4 conflicts`, …) + Close.

Implementation mirrors `import_dialog_js.html`'s `showPhase()` switch plus a self-contained `attachProgressStream()` that subscribes to `/api/jobs/{id}/progress` SSE for the running job.

Two related changes:

- `reattachIfRunning` (the function called when the user clicks the Maintain nav badge while a job is running) now reopens the **dialog in progress phase**, not the toast. Larger surface, same Minimize button is one click away. Matches what the Import dialog does.
- On terminal `done: true`, the browse-side tag autocomplete cache is invalidated (`window._damTagCache = null`). Maintain ops that change the catalog tag set — `cleanup`, `sync --apply --remove-stale` — could otherwise serve stale autocomplete entries from the 30-second cache. Belt-and-suspenders fix.

### Tag bulk mutations: tolerate missing-sidecar errors mid-loop

Reported flow: user runs `maki sync --apply --remove-stale` from a shell while `maki serve` is up, then renames a tag from the web UI and gets *"no such file or directory"*. The web server's catalog connection sees the fresh row state (SQLite WAL handles that), but the YAML sidecar for some asset is gone on disk while its row still references it.

`tag_rename`'s per-asset loop did `store.load(uuid)?`, which propagated the I/O error through the whole bulk operation and aborted everything from the failing asset onward — so the rename was applied to assets up to the missing sidecar and silently skipped for the rest.

Fix: `tag_rename`, `tag_split`, `tag_delete`, and `tag_normalize` (which powers `tag fix-unicode`) now catch the load failure (plus the upstream UUID parse failure), record it on the result struct as `errors: Vec<String>`, and continue the loop. The catalog or on-disk state will be cleaned up by `maki cleanup --apply`; the bulk tag operation no longer aborts on a transient inconsistency it didn't create.

## v4.5.4 (2026-05-08)

A small feature release: lightbox slideshow mode. Click a thumbnail to open the lightbox, hit Spacebar (or the new ▶ button in the toolbar), and the lightbox auto-advances through the result set.

Tests: 790 + 249 standard, 897 + 276 pro (unchanged from v4.5.3 — the slideshow is browser-side JS exercising the existing `navigate()` function).

### Slideshow

Two new toolbar buttons in the lightbox top-right (between the labels group and Info):

- **▶ / ▌▌** — start / pause. Same as pressing **Space**.
- **↻ / →** — loop-after-last toggle. `↻` (active) wraps from the last asset of the entire result set back to the first; `→` stops at the last asset.

Three keyboard shortcuts (scoped to lightbox-open so Space doesn't fight the browser's default scroll-down on other pages):

- **Space** — toggle play/pause.
- **`+` / `-`** while running — bump the cadence by ±1 second (clamped 1–60). When paused these keep their existing zoom behaviour. The current value flashes briefly in a small "5s" badge next to the play button.

Auto-pause on every deliberate user action so manual moves override the auto-advance: clicking prev/next, pressing arrow keys, changing the rating, changing the label, pressing Escape (which closes the lightbox and stops the slideshow).

**Cross-page advance**: the slideshow walks the entire result set, not just the current browse page. When the auto-advance reaches the end of the visible page, the host page swaps in the next page's results via htmx and the lightbox reopens at index 0; the slideshow timer keeps ticking through the swap so the cadence holds. Implementation hooks into the existing `_onNavigateBeyond` callback that prev/next at page boundaries already used.

**Loop-from-end-back-to-start**: when loop is on AND the slideshow reaches the last asset of the last page, the host's `onNavigateBeyond` handler queries the lightbox (`isSlideshowLooping()` / `isSlideshowRunning()`) and navigates to page 1. The pagination jump uses URL-rewrite + `htmx.ajax` rather than the old "click the nth-child pagination link" approach, which is more robust to pagination markup changes. Loop-off at the same boundary detects the no-progress case (same item ID after `navigate(1)`) and stops the slideshow cleanly so it doesn't tick forever on a frozen view.

### Configuration

Two new keys in `[browse]`:

```toml
[browse]
slideshow_seconds = 5     # default 5; range 1–60
slideshow_loop = true     # default true
```

Loaded by `AppState::new` from `maki.toml` and surfaced to the lightbox JS via the existing `/api/build-info` endpoint (which gained the two new fields without disturbing the import dialog's `{ai, pro}` gating). Runtime adjustments via `+`/`-` and the loop button apply for the current session only.

## v4.5.3 (2026-05-08)

A bug-fix patch release. Four web-UI fixes, all reported during real-world tag-cleanup work on a multi-thousand-asset catalog.

Tests: 790 + 249 standard, 897 + 276 pro (up from 782 / 249 / 889 / 276 — eight new round-trip tests in `saved_search`).

### Saved searches: round-trip every filter type

A saved search like `path:Pictures/Masters/2026/2026-05/` reloaded the browse page with no path filter applied — the chip looked like a no-op. Cause: `SavedSearch::to_url_params` only emitted URL params for `q`, `type`, `tag`, `format`, `label`, `rating`, `sort`. Anything else (`path`, `volume`, `collection`, `person`, plus every niche filter like `camera:`, `iso:`, `tagcount:`, `geo:`, `has_faces:`, exclude variants, …) was silently parsed and dropped.

Fix is two-pass: (1) emit dedicated URL params for the few widget-backed filters so the dropdowns reflect the saved values; (2) take the raw query, drop the tokens those URL params consumed, and stuff the remainder into `q=`. Token-level rules:

- **Multi-value URL params** (`tag`, `person`) — every occurrence consumed, emitted comma-joined since `build_parsed_search` accepts comma-separated chip lists.
- **Single-value URL params** (`type`, `format`, `label`, `volume`, `collection`, `path`, `rating`) — first occurrence consumed; subsequent occurrences fall through to `q=` so the catalog ANDs them in.
- **Negations** (`-tag:rejected`, `-camera:Phone`, …) — kept (no URL param for negation).
- **Niche filters and free text** (`camera:`, `iso:`, `tagcount:`, `geo_bbox:`, `has_faces:`, etc.) — kept.
- **Whitespace values** (`tag:"Fools Theater"`) — re-quoted before joining so the next tokenizer pass keeps the same boundaries.
- **`label:none`** — round-trips via the `label=none` sentinel that `build_parsed_search` already understands.

`query::tokenize_query` was promoted from `pub(super)` to `pub` so the saved-search module can reuse it directly. Adding a new structured filter to the search engine now requires zero changes here unless a dedicated widget arrives with it — the remainder catches anything we don't explicitly route.

### Browse → detail navigation: cross-page boundary no longer duplicates IDs

Walking through assets via the detail-page Next button across more than one page boundary appended duplicate IDs to the cached `browseIds` list. Symptom: the same images came back as the user kept clicking next, especially noticeable while editing tags (which shrinks the result set, making the duplicates look interleaved with new assets rather than identical repeats).

Cause: the prev-boundary handler decremented a single `maki-browse-page` counter after fetching the previous page, but the next-boundary handler **never** updated any counter. Once the user reached the end of the concatenated list, the boundary code re-fetched `browsePage + 1` against the stale value and appended the same page again.

Fix: split `maki-browse-page` into `maki-browse-page-min` (tracked by the prev handler) and `maki-browse-page-max` (tracked by the next handler). Each handler updates its own counter; the guards `pageMin > 1` / `pageMax < totalPages` reliably stop fetching once the entire result set is materialized. Legacy `maki-browse-page` key still written so a detail tab open during the upgrade keeps navigating correctly.

### Import dialog log: phase-boundary events render as boundary lines

Mid-import the activity log started showing entries like `phase_started undefined` followed by `embed phase_started`. Not a race condition — the SSE event order was fine. Cause: the JS log renderer had explicit branches for `phase === 'embed'` and `phase === 'describe'` per-file events but no branch for `PhaseStarted` / `PhaseSkipped` *boundary* events, which carry no `file` or `asset` field. Boundary events for the `import` and `auto_group` phases fell through to the generic else branch and rendered `evt.status + ' ' + evt.file` → literal "phase_started undefined". Boundary events for `embed` / `describe` hit the per-phase branch with no asset → "embed phase_started ".

Fix: dedicated up-front branch for `status === 'phase_started' || 'phase_skipped'`. Boundary lines now read `— import phase started —`, `— embed phase started —`, etc., styled with a small `.import-log-phase` CSS class (slightly bolder, body-text colour) so the section breaks stand out from per-file noise. `PhaseSkipped` events also carry the reason (e.g. *"— embed phase skipped: no AI build —"*).

## v4.5.2 (2026-05-07)

A web-UI release. Round 2 of the Maintain dialog adds four more tabs (Generate previews, Sync, Refresh, Cleanup), the tags page gets a one-click vocabulary export button, and the Maintain dialog grows wider so the seven tab labels fit on a single row.

Tests unchanged: 782 + 249 standard, 889 + 276 pro.

### Maintain dialog: 4 new tabs

The Maintain modal grows from 3 tabs to 7. The new ones:

- **Previews** — `POST /api/maintain/generate-previews`. Volume scope, three checkboxes: `--smart` (also generate the 2560 px smart preview), `--upgrade` (regenerate only when the best-preview variant changed — cheap "fix what fix-roles broke" sweep), `--force` (regenerate even existing previews). Catalog-mode only — the CLI's PATHS-mode "regenerate previews for files I just touched" stays CLI-only since web users pick scope from the catalog rather than the filesystem. Catalog-walk loop replicates the CLI handler in the web route (~80 LOC of `metadata_store.list()` → best-preview variant pick → file location resolve → `PreviewGenerator` call); a future shared helper is queued.

- **Sync** — `POST /api/maintain/sync`. File-layer reconciliation: detect moved, modified, new, and missing files on disk vs. catalog locations. Required volume picker (the engine method takes a `Volume`), optional volume-relative `Subpath` field with **filesystem autocomplete** (`/api/volumes/{id}/browse?prefix=…`), `--apply`, `--remove-stale` (auto-disabled until `--apply` is checked, mirroring the engine's own constraint). Subpath is canonicalize-and-prefix-checked against the volume mount so values containing `..` can't escape the volume.

- **Refresh** — `POST /api/maintain/refresh`. Re-read metadata from `.xmp` sidecar/recipe files that changed on disk. Volume scope, `--media` (also re-extract embedded XMP from JPEG/TIFF), `--dry-run`. The CLI's `--reimport` and `--exif-only` modes stay CLI-only (advanced/destructive options without enough web-UX justification yet).

- **Cleanup** — `POST /api/maintain/cleanup`. Remove orphaned records and files: stale catalog locations, locationless variants, orphaned assets, previews, smart previews, embeddings, and face crops. Volume + path-prefix scope (the path field has the same filesystem autocomplete as the Sync tab), `--apply` (otherwise report-only).

`JobKind` enum gained `GeneratePreviews` / `Sync` / `Refresh` / `Cleanup` variants. The shared toast learned 10 new summary counters (`generated`, `upgraded`, `refreshed`, `moved`, `new_files`, `missing`, `stale_removed`, plus `removed_*` variants for cleanup). Same at-most-one-job-per-kind enforcement and same `/api/jobs` re-attach plumbing as the round-1 tabs.

### Maintain dialog UX polish

- **Modal width raised 500 → 750 px** (override scoped to `#maintain-modal` only — the import modal stays at 500 px since its phased layout doesn't need extra room). Without it, seven tab labels overflowed and wrapped to a second line, which reads as a separate sub-nav rather than a single tab bar.
- **Defensive tab-bar CSS**: `overflow-x: auto` + `nowrap` + `flex-shrink: 0` on each tab so a future 8th tab scrolls horizontally instead of wrapping.
- **Checkbox layout fix**: the inherited `.import-modal label { flex-direction: column }` rule (correct for the form-row labels stacking "Volume" above its select) leaked into checkbox labels and stacked input/text/strong/span vertically. Explicit `flex-direction: row` + `margin-bottom: 0` + `flex-shrink: 0` reset.
- **Path autocomplete on Sync and Cleanup**: same UX as the import dialog's subfolder field (↑/↓/Tab/Enter to drill, Esc to dismiss, 120 ms debounce). Driven by the existing `/api/volumes/{id}/browse` endpoint. Volume label → ID resolution via a `volumesByLabel` map populated when the dialog first opens. Volume change clears the autocomplete cache so the next keystroke hits the new volume's tree.

### Tags page: Export vocabulary button

A one-click UI alternative to running `maki tag export-vocabulary` from a shell. New "Export vocabulary…" button next to the page title (`.tag-page-header` flex row) opens a small modal:

- Format radio: **YAML** (default), **Keyword text** (Lightroom / Capture One), or **JSON**.
- `Annotate with per-tag asset count` (= `--counts`). Auto-disabled when "Keyword text" is selected (LR / C1 reject comments).
- `Prune planned-but-unused entries` (= `--prune`).
- `Built-in default vocabulary only` (= `--default`). Greys out the counts and prune options.

Clicking **Download** navigates to `GET /api/tags/export-vocabulary?format=…&counts=1&prune=1&default=1`, which streams back the rendered file with `Content-Disposition: attachment` so the browser triggers a save dialog. Output is byte-identical to the CLI command with the same flags — same rendering pipeline (`src/vocabulary.rs`), same sanitisation behaviour.

## v4.5.1 (2026-05-06)

A web-UI release: long-running maintenance commands (writeback, sync-metadata, verify) get a launcher dialog instead of being CLI-only, and the top-of-page nav bar gets a "Catalog" dropdown to keep the seven existing entries from sprawling.

Tests unchanged: 782 + 249 standard, 889 + 276 pro.

### Maintain dialog

A new "**Maintain**" entry sits next to "Import" in the navigation, with the same pulsing-dot status badge and the same re-attach-to-running-job click behaviour. Click it to open a tabbed modal:

- **Writeback tab** — volume + query scope, three checkboxes:
  - `Write all XMP recipes in scope (not just pending)` — the `--all` knob, useful for rematerialising catalog metadata after large catalog-only restructuring.
  - `Mirror tags — remove XMP keywords the catalog no longer has` — the `--mirror-tags` knob (auto-disabled until "Write all" is checked, mirroring the clap-level constraint). Reads each XMP's `dc:subject` and `lr:hierarchicalSubject`, diffs against the asset's catalog tags, removes stale entries.
  - `Dry run — preview without writing`.
- **Sync metadata tab** — bidirectional XMP ↔ catalog sync. Volume scope, `--media` (re-extract embedded XMP from JPEG/TIFF), `--dry-run`. Summary surfaces inbound / outbound / conflicts / unchanged / media-refreshed counts.
- **Verify tab** — content-hash check for media + recipes on disk. Volume scope, max-age-days threshold to skip recently-verified files.

Each form posts to a new `POST /api/maintain/<op>` endpoint that returns `{job_id}` immediately and streams per-file progress through the existing `JobRegistry` SSE pipeline. Closing the dialog while a job runs is fine — the standard progress toast keeps watching, and the pulsing dot on the Maintain entry stays lit until the job finishes.

`JobKind` enum gained `Writeback`, `SyncMetadata`, `Verify` variants. The shared progress toast learned the new summary counters (`written`, `inbound`, `outbound`, `conflicts`, `unchanged`, `media_refreshed`, `verified`, `modified`) so terminal events render readable summaries. The shared nav-badge polling drives both the Import and Maintain badges from a single `/api/jobs` poll, filtering by JobKind on the client side. At-most-one-job-per-kind is enforced server-side (409 on conflict); different kinds run in parallel.

Engine APIs are reused as-is — no signature changes to `engine.writeback`, `service.sync_metadata`, `service.verify`. The route boundary handles the small adaptation between the dialog's volume-label string and `sync_metadata`'s `Option<&Volume>` parameter.

### Catalog nav dropdown

Five top-level entries (Stats, Analytics, Backup, Volumes, Duplicates) collapsed into one "Catalog ▾" dropdown to keep the nav bar from sprawling now that Maintain landed alongside Import. The dropdown trigger sits in the same nav band; its menu pops down with the body theme's card surface (so menu text uses regular body colour, not the white-on-coloured nav contrast). Click outside or Escape closes; the caret rotates 180° to indicate state. `aria-haspopup`/`aria-expanded`/`role="menu"`/`role="menuitem"` wired for screen readers. Pages themselves are unchanged.

### Layout fix

Maintain dialog checkbox labels rendered each child on its own line — input, text node, `<strong>`, `<span>` all stacked vertically — because the inherited `.import-modal label` rule sets `flex-direction: column` (correct for "Volume" stacking above its select) and the override never reset it. Explicit `flex-direction: row` plus `margin-bottom: 0` and `flex-shrink: 0` on the input keeps each checkbox row on a single line that wraps naturally.

## v4.5.0 (2026-05-05)

A behaviour-and-UX release. The headline change is a rework of XMP writeback semantics so the safety-net default (`[writeback] enabled = false`) no longer blocks change tracking or the explicit `maki writeback` command — the config flag now controls *only* automatic flush on every edit. A new `--mirror-tags` flag reconciles XMP keyword lists with the catalog after large catalog-only restructuring (rename, split, delete, fix-unicode), so accumulated drift can be flushed in one shot. Browse gains Shift-Cmd-A "select all matching the current filter" with a confirmation modal — the missing primitive for "I forgot to embed/auto-tag this 500-photo shoot." The tags page click count now equals what the click target shows. Tag autocomplete keeps middle-of-hierarchy matches visible and lets you drill into a prefix.

Tests: 782 + 249 standard, 889 + 276 pro (up from 779 + 249 / 886 + 273 in v4.4.15) — three new pro-CLI regression tests cover the writeback / mirror-tags / disabled-then-flush flows; lib counts grew through expanded vocabulary and tag-filter assertions.

### Writeback rework: auto-flush split from tracking, manual flush always works

Before: `[writeback] enabled = false` killed three things in one go — automatic XMP write on every edit (intended), the per-recipe `pending_writeback` tracking (unintended; the inline path short-circuited at entry), and the explicit `maki writeback` command (outright bail-out). Users keeping the flag off as a safety net during development therefore lost the audit trail of staged edits *and* couldn't flush manually without flipping the config off and on for every batch.

`[writeback] enabled` now means strictly "auto-write on every edit." Every metadata edit (rating, label, description, tags) marks the asset's XMP recipe(s) `pending_writeback = true` regardless. `maki writeback` is the explicit manual flush and runs whether or not the flag is set. `maki status`'s pending-writeback hint reads `→ maki writeback (auto-flush off; this is the manual flush)` when auto-flush is off, instead of the old "enable [writeback] in maki.toml" detour.

Inside `_inner` writeback methods, the disabled state is treated like an offline volume: mark pending, skip the file write, save the sidecar. Caller-side `if self.is_writeback_enabled() { write_back_…_to_xmp_inner(…) }` guards (six sites in `query.rs`) gone. Two early-return bail-outs in `writeback` / `writeback_process` gone. `sync_metadata`'s outbound phase explicitly passes `mirror_tags = false` and `is_writeback_enabled` is no longer checked there; the inbound phase is unaffected.

### `maki writeback --mirror-tags` reconciles XMP with catalog

The classic flush has always been **additive only**: it pushes catalog tags onto the XMP, but never removes XMP tags the catalog no longer has. So renames, splits, deletions, and `tag fix-unicode` performed in MAKI leave the OLD keyword stranded in `dc:subject` and `lr:hierarchicalSubject` on disk alongside the new one — a pre-existing bug, made unavoidable by users who keep auto-flush off and rely on manual flush.

`--mirror-tags` (requires `--all`) reads the existing `dc:subject` + `lr:hierarchicalSubject` from each XMP, diffs against the asset's catalog tags, and removes the stale entries before writing the current set. Result: XMP keyword lists exactly mirror catalog tags. Other metadata fields (rating/label/description) were already replace-semantic, so they need no special handling. Pair-required with `--all` because mirror mode is most useful as a broad rematerialise sweep — running it on the narrow pending-only default is rarely what users want, and clap's `requires` makes the intended scope obvious. Default behaviour unchanged: bare `maki writeback` and `--all` (without `--mirror-tags`) stay additive, preserving back-compat for users who mix MAKI tags with externally-added XMP keywords.

Typical use after large catalog-only restructuring:

```
maki writeback --all --mirror-tags --log
maki writeback --all --mirror-tags "tag:wedding"   # scoped variant
```

### Browse: Shift-Cmd-A selects all assets matching the current filter

Cmd-A already covered "select everything visible" (the current page). The gap: there was no way to operate on results that span more than one page without manually paging through and shift-clicking — useless for the concrete case of "I forgot to embed / auto-tag / add a tag during import for this 500-photo concert shoot."

Wire **Shift-Cmd-A** (Mac) / Shift-Ctrl-A (Linux/Windows) to "select all matching the current filter." A new `GET /api/all-ids?<search-params>` endpoint returns every matching asset ID in one round-trip (no thumbnails, just UUIDs — fits any realistic catalog). When the result spans more than one page a confirmation modal appears, sized to `total_pages`, with replace/add radios when an existing selection is non-empty. Single-page filters skip the dialog and merge silently. After a multi-page select-all the bottom toolbar reads `487 selected (across 9 pages)` so the wider scope is always visible. The full set then drives every existing batch toolbar action (Embed, Add Tag, Auto-Tag, Detect Faces, …) without further changes.

The new endpoint is also a primitive future "operate on full filter result" features can build on without adding more endpoints.

### Tags page: click count matches row count

The tags page row "color" with own_count 174 / leaf 23 used to navigate via `?tag=color` (default browse semantic = case-insensitive any-path-position) and land on a result of 540 — way off. Two semantic gaps stacked up: default browse `tag:color` is case-insensitive (collapses lowercase `color` into `Color` cousins), and matches "color" at any path position (so a stray `something|color` path is included).

Click targets now use the `=^` (whole-path + case-sensitive) and `=^/` (whole-path + leaf-only + case-sensitive) markers so the click result count exactly equals the row's own_count / leaf_count. To make `=^/` work, the tag filter parser now allows `=` and `/` to combine — they previously were mutually exclusive, with `=` silently dropping `/`. The combined SQL branch wraps `=`-positives in parentheses and AND-NOTs both descendant-position guards (`"<path>|…` from root, `|<path>|…` from any mid-tree position).

Inside `list_leaf_tag_counts`, the descendant guard switched LIKE → GLOB so the case-sensitive leaf computation isn't confounded by ASCII case-folding: `["color", "Color|red"]` correctly leaf-counts "color" without LIKE treating "Color|red" as a descendant.

Two earlier attempts at this fix (a flat case-fold of `list_all_tags`, then a hierarchical case-fold) were merged and reverted within the same release window — both overcorrected by collapsing intentionally-distinct case variants into single rows. Reverts are part of the history; the eventual fix moved the gating from the catalog query to the navigation layer.

### Tag autocomplete improvements

Two UX fixes for the tag-filter dropdown (also reused on the asset-detail tag input and tag rename / split modals):

- **Middle-of-hierarchy matches stay visible.** Earlier MAKI versions hid the parent row in favour of the more specific children, making it hard to land at exactly the level you intended. Now any node whose own name (any segment) matches the query stays in the list alongside its descendants.
- **Drill into a hierarchy by clicking the prefix.** Each suggestion that has a hierarchy renders the prefix in a dimmed style. Clicking the dimmed prefix narrows the autocomplete to that level — useful when a query produces dozens of suggestions across unrelated parents.

### `maki tag export-vocabulary --counts` + nested JSON

Two additions to the export-vocabulary command for vocabulary curation:

- `--counts` annotates each entry with its per-tag asset count. In YAML output (`# N assets` trailing comment, file still parses), the count gives a quick read on which tags are heavily-used vs. candidates for retirement. In TEXT (Lightroom / Capture One keyword) output the flag is silently ignored — those tools reject comments. JSON format already emits counts unconditionally.
- `--format json` emits the vocabulary as a nested object: each node has `count`, optional `children` keyed by sub-segment. Identical in information content to YAML + counts; the JSON shape is just easier to consume from dashboards or integration scripts.

### Smaller fixes

- **Path autocomplete cap raised 20 → 100** on the filter bar. A typical year/month/day catalog tree exceeds 20 at every level (≥30 years × 12 months × 31 days plus multi-shoot day suffixes). Server-side `/api/paths` now defaults to 100 too (still hard-clamped to 100 to prevent runaway responses).
- **Browse pagination clamp**: deleting / grouping the last few items on the last page no longer dumps the user into an empty "No results found" screen. The `/api/page-ids` and main browse handler clamp the requested page to the last available page when the result set shrinks.
- Two browser regressions from v4.4.15's template-comment refactor fixed: leading HTML comments before `{% extends %}` triggered quirks mode; an HTML comment inside a `<script>` partial broke JS parsing.

## v4.4.15 (2026-05-04)

A maintenance release. No new user-facing features — the entire release is internal restructuring driven by a fresh QA report (`doc/qa-report/qa-report.md`, refreshed at v4.4.14). The headline outcome: every large file in the codebase has been split along its natural cleavage planes, the `main.rs` god-module has been collapsed to a 2.8-kLOC dispatcher, and the import command now demonstrates a clean kernel/workflow/handler separation that future commands can follow.

Tests (779 + 249 standard, 886 + 273 pro) are unchanged from v4.4.14 — the refactor is structural and behavior-preserving by design.

### Big files split by section

The QA report identified four 6–9 kLOC god-modules. All four are now directory-modules with multi-file `impl` blocks:

- **`catalog.rs` 9200 → 4524 LOC** (most of the residual is tests). 17 submodules under `src/catalog/`: `schema`, `asset_crud`, `variant_crud`, `recipe_crud`, `volume`, `lookup`, `duplicates`, `recipe_query`, `rebuild`, `stats`, `search_builder`, `search_exec`, `facets`, `tags`, `analytics`, `backup`, `cleanup`. Six cross-section private helpers (`build_search_where`, `load_locations_for_hash`, `compute_duplicate_stats`, `load_duplicate_entries`, `backfill_gps_columns`, `stats_per_volume`) lifted to `pub(super)`.
- **`asset_service.rs` 8886 → 2759 LOC**. 12 submodules under `src/asset_service/`: `import`, `relocate`, `verify`, `sync`, `cleanup`, `volume`, `dedup`, `refresh`, `fix`, `export`, `ai`, `video`. Three cross-section helpers (`apply_modified_recipe`, `update_sidecar_file_location_path`, `update_sidecar_recipe_path`) lifted to `pub(super)`.
- **`query.rs` 6820 → 6028 LOC** with the parsing layer (`parse_search_query` + `ParsedSearch` + `NumericFilter` + `tokenize_query` + `normalize_path_for_search` — ~800 LOC) extracted into `query/parse.rs`. Public API unchanged via `pub use parse::*;`.
- **`web/routes/ai.rs` 1614 → split** into `ai/{tags,embed,similarity,faces,stroll,mod}.rs`. `mod.rs` keeps the shared `resolve_model_dir` and `resolve_labels` helpers.

The pattern in every case: keep struct + ctor + module-level helpers + tests in the original file (now the module root); each `// ═══ X ═══` section becomes a sibling submodule with `impl Catalog { ... }` / `impl AssetService { ... }` blocks. No struct split, no public API change.

### main.rs collapsed to a dispatcher

`main.rs` was 9725 LOC at v4.4.14 — CLI argument parsing, the dispatcher's match, all 40 command handlers, and a handful of helpers. Now:

- **`main.rs` 2797 LOC** — `Cli` derive structs, the `Commands` enum + sub-enums, `main()`, and the dispatcher (`run_command`, 859 LOC of one-liner match arms).
- **`commands.rs` 7004 LOC** (new) — every `run_X_command` handler plus their private helpers (`merge_trailing_ids`, `resolve_person_id`, `resolve_face_id`, `print_*_human`).

The dispatcher's 859 lines are now a flat list of arms like `Commands::Import { … } => commands::run_import_command(…)`. Each handler is its own self-contained function with `(json, log, verbosity)` plus the destructured fields. A future per-command split of `commands.rs` (one file per command, mirroring the `catalog/` and `asset_service/` patterns) is queued but not done in this release — the file is still a single 7-kLOC unit.

### Import workflow extracted

The architectural counterpart to the structural splits: `AssetService::import_workflow` lifts the orchestration that lives between input parsing and the kernel call. Both the CLI (`run_import_command`) and the web (`web/routes/import.rs`) used to duplicate the entire sandwich — profile resolution, file-type filter assembly, volume resolution, tag merging, the post-import auto-group neighborhood scan with preview upgrade, the post-import embed phase, the post-import describe phase. Reproduced in two places, with subtle drift (the web version skipped the auto-group neighborhood scan and the post-group preview upgrade).

The new `import_workflow` lives in `asset_service/workflow.rs`. Inputs:

```
ImportRequest        { paths, volume_label, profile, include, skip,
                       add_tags, dry_run, smart, auto_group, embed,
                       describe }
ImportEvent<'a>      { PhaseStarted, PhaseSkipped, File, Embed,
                       Describe } — frontend-agnostic progress
ImportWorkflowResult { import, auto_group, embed, describe }
```

The CLI handler (run_import_command 454 → 234 LOC) and the web handler (run_import_dry + run_import_with_progress + the two helper functions, 446 → 327 LOC) shrink to thin "translate input → ImportRequest → call workflow → translate output" adapters. The web inherits the CLI's auto-group neighborhood scan + post-group preview upgrade for free.

This is a demonstration of the kernel/workflow/handler pattern. The other commands with both CLI and web frontends (embed, describe, auto-tag, detect-faces) have a thinner duplication surface — the kernel methods (`embed_assets`, `describe_assets`, etc.) already are the workflow — so a similar wrapper would be ceremony for little gain. Not extended in this release.

### Documentation polish

- 29 source files gained `//!` module-level docs.
- 17 large templates gained leading HTML purpose comments (matching the pattern set by `import_dialog.html` and `job_toast.html` in v4.4.14).
- The few remaining undocumented `pub` items in the top-three files (`Catalog::open`, `SearchSort::from_str`, `FileStatus`, `AssetService` + `::new`, `QueryEngine` + `::new`) all got doc comments.
- A new `doc/qa-report/qa-report.md` (with the prior 2026-04-17 report archived under `qa-report/archive/`) documents the punch-list and tracks which items landed in which commits.

### Smaller DRY cleanups

- New `config::load_config()` helper returns `(PathBuf, CatalogConfig)`. **27** paired `find_catalog_root() + CatalogConfig::load()` call sites in `main.rs` collapsed.
- New `config::resolve_model_dir(model_dir_root, model_id)` — single source of truth for `~/`-expansion. The web helper delegates; three inline copies in `main.rs` removed.
- New `web::routes::resolve_asset_id_or_err(catalog, prefix)` — replaces 7 copy-paste sites of the `resolve_asset_id().ok_or_else(...)` pattern. Unified error message format.
- New `web::routes::spawn_catalog_blocking<T>(...)` async helper. Returns `Result<T, Response>` so handlers `?`-short-circuit the uniform 500-with-formatted-error path. Three demo sites migrated; ~100 remaining sites are opportunistic.
- `build_search_where` shrank 357 → 205 LOC via 6 new per-filter helpers (`add_text_filters`, `add_format_filter`, `add_volume_filter`, `add_path_filter`, `add_date_filters`, `add_geo_filters`).
- `parse_search_query` shrank 242 → 186 LOC via four module-scope lookup tables (`SIMPLE_FILTERS`, `NUMERIC_FILTERS`, `STRING_FILTERS`, `BOOLEAN_TOKENS`). Adding a new filter of those shapes is now one table line.
- Naming: the lone `classify_impl` outlier renamed to `classify_inner` so the codebase consistently uses `_inner` for private helpers.

### Tests + miscellany

- Tests: 779 unit + 249 CLI + 14 doc on standard build (unchanged from v4.4.14). Pro: 886 + 273 + 14 (unchanged).
- Cross-platform CI passes. No new dependencies, no API changes.
- Side-change from the workflow callback design: `import_with_callback` and `embed_assets` callbacks went from `Fn` → `FnMut` (strictly more permissive; no caller broke). `describe_assets` stays `Fn + Sync` for parallel execution; the workflow wraps the outer FnMut in a Mutex when calling it.
- Side-change from extraction: `run_writeback_command` and `run_sync_metadata_command` lost their inner `if cli.timing { … }` print. The outer dispatcher in `main()` already prints a wall-clock summary; the inner ones were duplicates.

## v4.4.14 (2026-05-03)

A maintenance + UX release. The headline change is **live progress for every long-running web operation** — import, embed, auto-tag, detect-faces, and describe all flow through a new generic `JobRegistry` and a reusable progress toast, replacing the per-route bespoke plumbing (and the silent multi-minute "click and pray" UX on large batches). Plus a Unicode-NFC normalisation pass that fixes duplicate-tag bugs caused by NFC/NFD encoding drift, the missing standalone embed surfaces in the web UI, and the post-import embed/describe phases that were specified in CLI but never wired into the web import dialog.

### `maki tag fix-unicode` — collapse NFC/NFD duplicate tags

The user-reported symptom: a tag `Ö-HA` showing up twice on the tags page, once between O and P, once after Z, with neither matching the other in search. Diagnosis: NFC vs NFD encoding drift. macOS path APIs lean **NFD** (`O` + combining diaeresis, two code points), most XMP writers and the W3C/IETF stack produce **NFC** (precomposed `Ö`, single code point). The two render identically but compare as different byte strings, so a single logical tag ends up as two distinct catalog entries.

Two-part fix:

1. **Write-path canonicalisation**: `tag_input_to_storage` now applies NFC normalisation at the existing input chokepoint, so every new or edited tag is stored canonically regardless of source. After this, no new mismatches can appear.
2. **Migration command**: new **`maki tag fix-unicode [--apply]`** — same dry-run-by-default rhythm as `fix-dates` / `fix-roles` / `expand-ancestors`. Walks every asset, NFC-normalises its tag list, deduplicates per-asset (so an asset that had both forms ends up with one), and saves through the standard sidecar + catalog write path. XMP writeback (when enabled) replays the deduplications. Idempotent — running it again on a normalised catalog reports `0 fixed`.

```bash
maki tag fix-unicode                # dry-run preview
maki tag fix-unicode --apply
maki tag fix-unicode --apply --log  # per-asset progress
```

8 new tests (4 in tag_util for NFC at the chokepoint, 4 in query for the migration's dedup/skip/dry-run paths).

A **NOTES ON SORT ORDER** section in the reference page documents the trade-off: NFC stores `Ö` as U+00D6 which sorts after Z under SQLite's default byte collation; NFD's "near base letter" ordering is accidental, not deliberate (Swedish/Finnish dictionaries put Ö after Z; German DIN 5007-1 puts it next to O — there's no single right answer at the byte level). NFC is still the right storage form because it matches every external system MAKI rounds-trips through (Lightroom, Capture One, XMP, modern filesystems).

### Web import: post-import embed and describe phases

The CLI's `maki import` has run post-import embed and VLM describe phases for ages, gated by `[import] embeddings = true` / `[import] descriptions = true` config or per-invocation `--embed` / `--describe` flags. The web import dialog skipped both — anything kicked off through the browser stopped after the basic import + auto-group, leaving the embeddings the user expected to find missing.

Now wired up:

- **Embeddings checkbox** (visible on `ai` builds) and **Descriptions checkbox** (visible on `pro` builds) on the dialog form, hidden when the running binary doesn't support the feature (detected via new `GET /api/build-info` returning `{ai, pro}`).
- Same precedence semantics as CLI: explicit checkbox wins, otherwise falls back to config.
- Per-asset progress flows through the existing SSE stream as phase-tagged events (`{phase: "embed", asset, embedded}` / `{phase: "describe", asset, described}`); the dialog's status line and log render those distinctly.
- Sub-helper extracted: `AssetService::embed_assets` lifts the SigLIP loop out of `maki embed` and the post-import phase into a single service method (~150 LOC deduplicated). Same dedup, same image fallback (smart preview > regular preview > original on online volume), same skip-already-embedded guard.

### Standalone embed surfaces in the web UI

Auto-tag generates embeddings as a side-effect of classification, but a user who only wants similarity coverage without applying any tags had no path. Three new surfaces, all reusing `AssetService::embed_assets`:

- **Browse toolbar**: new `Embed` button between Auto-tag and Detect faces (ai-gated). Confirms count, posts to `/api/batch/embed`, watches via the new toast.
- **Asset detail**: `Embed` button inside the *Similar images* details, before *Find similar*. Updates the inline status with "Embedding generated" / "already exists". The "no similar images" hint now points at this button instead of telling users to run auto-tag.
- **Endpoints**: `POST /api/batch/embed` and `POST /api/asset/{id}/embed`, both ai-gated.

### Generic `JobRegistry` and live-progress toast

Up to v4.4.13, only `maki import` had live SSE progress + re-attach in the web UI. Every other long-running batch (auto-tag, detect-faces, describe, the new embed) ran synchronously via `spawn_blocking` and held the HTTP request open until done — fine for 5 assets, painful for 5000 (no feedback, request timeouts, broken if the user reloads).

The shape of "broadcast channel + ring buffer + atomic counters + status endpoint + re-attach" is the same for every job kind. Duplicating it five times would have been ~600 LOC of boilerplate, so it's lifted once into a generic abstraction:

- **`src/web/jobs.rs`**: `JobRegistry { jobs: HashMap<JobId, Arc<Job>> }`. Each `Job` carries id, kind (Import / Embed / AutoTag / DetectFaces / Describe), started_at, broadcast sender, ring buffer of recent events, JSON progress snapshot, completed flag. `Job::emit` centralises "broadcast + push to ring + update snapshot"; `Job::finish` sets the flag and emits a terminal `done: true`. Recently-finished jobs stay in the registry briefly (16-job history) so re-attach after page reload still shows the final state.
- **Generic endpoints**: `GET /api/jobs` (snapshot of running + recent), `GET /api/jobs/{id}` (single status), `GET /api/jobs/{id}/progress` (SSE stream). Replaces the import-specific `/api/import/status` and `/api/import/progress` (the dialog and nav badge moved to the generic endpoints in lockstep).
- **8 new unit tests** covering ID uniqueness, ring eviction at capacity, completed history eviction, finish semantics, latest-by-kind lookup, and snapshot counts.

All four batch endpoints migrated to the new pattern in one pass. POST returns `{job_id}` immediately; per-asset events emit `{processed, total, status, asset, ...}`; terminal events carry kind-specific counts. A reusable **progress toast** (`templates/job_toast.{html,js_html}`, mounted from base.html) provides `window.makiJob.watch(jobId, label, {onDone})` — a small floating bottom-right widget that subscribes to the job's SSE stream, shows live "X / Y done — status (asset)" lines, and on done swaps to a summary line built from whatever counters the terminal event carries (`embedded` / `succeeded` / `tags_applied` / `faces_detected` / `descriptions_set` / etc.). Auto-dismisses after 4s on success; sticks on error.

For 5 assets this is a ~2-second toast flash. For 5000 assets the HTTP request no longer holds open and the user gets real progress.

### Import dialog: minimize-to-toast

A "Minimize to toast" button on the import-progress phase hands the running import off to the same global toast and closes the modal. The job keeps running on the server unchanged (it was already a registered Job), but the user gets their screen back and sees a small floating progress line in the corner instead of a full-screen modal blocking work. Re-opening the import link while still running re-attaches via the existing `/api/jobs` flow — the dialog's progress phase reappears and takes over from the toast.

Tear-down ordering matters: close the dialog's own EventSource *before* handing the job to the toast, otherwise both listeners race on the same broadcast and the done event lands twice. On non-error done, refresh the browse grid via htmx if the user is on `/` so newly-imported assets appear.

### Single-asset endpoints stay synchronous

Per-asset endpoints (`POST /api/asset/{id}/embed`, `POST /api/asset/{id}/detect-faces`) deliberately *don't* go through the registry — a single image is fast (a few hundred ms), the existing inline-result UI works fine, and adding a brief progress flash for a one-asset operation would be more friction than feedback.

### Tests + miscellany

- Tests: 779 unit + 249 CLI + 14 doc on standard build (was 764 + 249 + 14 in v4.4.13). Pro: 886 + 273 + 14 (was 871 + 273 + 14).
- One additional crate: `unicode-normalization = "0.1"` (small, zero-deps beyond `tinyvec`).
- 90 subcommands total (was 89) — `tag fix-unicode` is the new addition.

## v4.4.13 (2026-04-30)

A tag-management feature pack: a new `tag delete` command and matching web UI, the tags-page count semantics rewritten so the numbers actually mean something, and a handful of UX fixes around tag editing.

### `maki tag delete` — the missing primitive

Completes the `rename` / `split` / `delete` family. Same dry-run-by-default safety pattern, same marker grammar (`=tag` / `/tag` for leaf-only, `^tag` for case-sensitive), cascades to descendants by default. Newly-orphaned ancestors on each asset are cleaned up automatically.

```bash
maki tag delete "lansdcape" --apply                # typo fix, drops everywhere
maki tag delete "event|wedding-jane-2025" --apply  # remove a whole branch
maki tag delete "=subject|nature" --apply          # leaf-only: skip assets that have a deeper child
```

The web UI's tags page gains a **trash button (×)** on every row, hover-tinted to the destructive accent, opening a Preview→Apply confirmation modal — same Enter-twice rhythm as the rename and split modals. Backend: `POST /api/tag/delete`. CLI: 7 unit tests covering cascade, dry-run, sibling preservation, leaf-only with/without descendants, empty-tag rejection.

### Tags-page counts: own vs leaf

The previous parenthesised number on each tag row was defined as `own_count + sum of descendants' own_counts`, which is mathematically nonsense given MAKI's auto-expansion storage model: a parent's `own_count` already covers every asset that has any descendant, so summing the descendants again double-counts. Asset A tagged `location|Germany|Bayern|München` plus asset B tagged `location|Berlin` rendered `location` as `2 (6)` — the 6 was just rolled-up tag-string occurrences across the chain, not a meaningful asset count.

Replaced with `(N as leaf)` — assets where this tag is the *deepest* level on that asset (no descendant of it is also present). Matches `tag:/foo` (leaf-only chip mode):

- For a parent tag, surfaces "assets sloppily tagged at exactly this level when they could be more specific" — actionable signal.
- For a true leaf, equals own_count and the UI omits the parens.
- For a properly-tagged parent (every photo specialised down to a deeper child), leaf-count is 0 — also omitted, so cleanly-tagged hierarchies show a single number.

The `(N as leaf)` text is **clickable** — links to `/?tag=/<name>` (browse with leaf-only filter), so users can act on the candidate-for-finer-tagging set in one click.

Computed via a new `Catalog::list_leaf_tag_counts()` using the same `json_each` SQL engine `list_all_tags` uses, with a `NOT EXISTS (descendant on same asset)` subquery. Pure SQL, no per-asset Rust iteration.

### Browse: result-count delta hints

Next to the result count on the browse page, show inline hints when more matches exist behind a UI flag:

```
152 assets matching "tag:Bayerischer Wald" · 73 more in stacks · 12 more without default filter
                                              ^^^^^^^^^^^^^^^^^   ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
                                              click → ?stacks=0   click → ?nodefault=1
```

Each segment shows only when its delta is non-zero, so the count line stays clean when nothing's hidden. Implementation: a `compute_count_deltas` helper mutating `opts` for the cheap stacks delta and re-running `build_parsed_search` with `nodefault=1` forced for the default-filter delta. `htmx:afterSwap` now syncs the stack-toggle state from the URL after every swap so links that flip `&stacks=` leave the toggle button in the correct state instead of requiring a second click.

The same hint pattern fixed the user's specific confusion: tags page showed `Bayerischer Wald` with 225 assets but browse only listed 152 — the 73 hidden behind stack collapse are now visible as a clickable delta.

### Facet sidebar: tag list cap raised from 30 to 5000

The tags section of the facet sidebar was capped at 30 rows ordered by count desc — for any non-trivial filter, lower-frequency co-occurring tags silently disappeared (the report: `GPK Los Angeles Workshop` not appearing under `tag:=abandoned`). Worse, surviving descendants whose parent got truncated rendered with synthetic count-0 parents in the JS tree-build. Bump to 5000; real catalogues even mid-restructure are around 4500 *total* tags catalogue-wide, far below the cap.

### Tag-modal autocomplete consistency

The split modal's target inputs had no autocomplete. Extracted the rename modal's autocomplete logic into a shared `attachTagAutocomplete(input, ac, onAccept, onSubmit)` helper (~70 LOC) and wired both modals through it. Net ~50 LOC removed even after adding the helper. **Split-modal keyboard flow** now matches rename: Enter on a non-last target advances to the next row, Enter on the last row submits (preview if Apply is disabled, apply otherwise) — same Enter-twice rhythm.

### Tags-page tree pre-order (carryover polish)

The pre-order tree fix shipped in v4.4.12 had one outstanding case: parent rows accumulated `(0 as leaf)` clutter even when the subtree was perfectly clean. The leaf-count semantics in this release fix that — only show the parenthesised number when it's actionable.

Tests: 764 unit + 249 CLI integration + 14 doc on standard build (was 753 + 249 + 14 in v4.4.12).

## v4.4.12 (2026-04-29)

Bug fix: tag-page tree rendering put children of prefix-sharing parents in the wrong place.

### `build_tag_tree` now emits in tree pre-order

The tags page (`/tags`) renders a flat list of `(name, depth)` entries, with CSS handling indentation by depth. The old builder produced entries in **lexicographic full-path order** (BTreeMap iteration). That broke when a tag had both flat siblings and `|`-children sharing a prefix:

```
Bricking Bavaria       (depth 3, parent)
Bricking Bavaria 2012  (depth 3, flat sibling — name starts with " 2012")
Bricking Bavaria 2015  (depth 3, flat sibling)
…
Bricking Bavaria 2025  (depth 3, flat sibling)
Bricking Bavaria|2011  (depth 4, real child of `Bricking Bavaria`)
```

`|` (0x7C) sorts *after* ` ` (0x20), so the renamed child `…|2011` ended up at the bottom of the prefix block — visually dissociated from its parent and looking like a child of `Bricking Bavaria 2025`'s subtree. The repro from the wild: someone renamed `Bricking Bavaria 2011` → `Bricking Bavaria|2011` to start migrating to a hierarchical structure, and the result rendered confusingly.

Fix: emit in tree pre-order — parent first, then all descendants alphabetically by leaf segment, then the next sibling at the same depth. The browse facet panel's JS tree walker already used this approach correctly; this just brings the server-side builder in line.

After the fix, the same input renders as:

```
Bricking Bavaria       (depth 3)
  Bricking Bavaria|2011  (depth 4) ← directly under its parent
Bricking Bavaria 2012  (depth 3)
…
```

Tests: 4 new unit tests in `web::routes::tags` lock in pre-order behavior, synthetic-parent handling, case-insensitive sibling sort, and total-count rollup. Standard build now at 757 lib + 249 CLI + 14 doc tests.

## v4.4.11 (2026-04-28)

The faceted sidebar becomes the navigation surface. Every facet row is now click-to-narrow, tags render hierarchically, and the section order is rebalanced so the curated tag taxonomy sits where most users actually filter.

### Tag strolling

The `/api/facets` endpoint already returned per-tag counts for the current filtered result set, but the sidebar rendered them as static labels. Now every row is clickable, and clicking dispatches to the right filter setter:

| Facet | Click action |
|-------|--------------|
| Tag | Adds the **full path** as a chip on the filter bar. Picking `2024` under `Holzkirchner Blues- und Jazztage` adds the disambiguated tag, not the bare leaf. |
| Rating | Sets the star widget to that exact rating. |
| Label | Sets the color-dot widget to that label. |
| Year | Appends `date:YYYY` to the search box. |
| Format | Toggles the format checkbox (additive multi-select). |
| Volume | Sets the volume dropdown to that volume. |
| Geotagged | Appends `geotagged:1` to the search box. |

After every click the panel re-fetches against the new filter, so the next layer of co-occurring facets is computed live. Click-loop a few rows in and you've drilled to a precise subset without ever opening the search syntax docs. Keyboard accessible — Tab to focus, Enter or Space to activate. Three new `window.*` helpers in `filter_bar_js` (`toggleFormatFilter`, `setVolumeFilterById`, `addQueryTerm`) keep the chip/widget logic centralized so the facet dispatcher just delegates.

### Hierarchical tag rendering

MAKI auto-expands every hierarchical tag to its ancestor paths on storage, which means the flat list returned by `/api/facets` already contains every level. Reshape it client-side into a tree and render depth-first with indented rows. CSS does the indentation via a `--depth` variable on each row + `padding-left: calc(0.3rem + var(--depth) * 0.85rem)` — no nested DOM. Counts at each level reflect the union of descendants, which makes parent rows useful filter targets in their own right (`event` showing 2,419 means "any photo with anything under `event`").

### Reordered sections

Old: Ratings → Labels → Formats → Volumes → Tags → Years → Geotagged.

New: Ratings → Labels → **Tags** → Years → Formats → Volumes → Geotagged.

Tags promoted from 5th to 3rd because the curated taxonomy is most users' primary filtering axis; formats and volumes are usually set up once and rarely toggled. New users see Tags expanded by default (the existing default-open behaviour for unsaved sections).

### Worked example, in case the abstract is too abstract

Want to find under-tagged photos at the Holzkirchner Blues- und Jazztage? Open browse, click `event` in the Tags section → filter narrows. Click `festival > Holzkirchner Blues- und Jazztage` → narrows to all 13 years. The Tags section now lists `…|2024 (12)`, `…|2023 (8)`, …; the Years section shows the matching calendar years. Click the Years row for `2018`, and if its asset count is greater than the festival's `…|2018` row count, those extra photos are festival shots missing the festival tag — go fix them.

### Polish

- `window.onFilterChange` exposed alongside `window.triggerSearch` so external panels can request a search refresh after manipulating widgets.
- User guide chapter 6 (Web UI) "Faceted sidebar" section rewritten — the previous "read-only statistical breakdown" framing is now wrong; replaced with click-action table, hierarchy explanation, and the worked example above.

Tests: 753 unit + 249 CLI integration + 14 doc on standard. No new tests needed — frontend-only behaviour change validated end-to-end against the existing `/api/facets` endpoint.

## v4.4.10 (2026-04-28)

Headline: a new `maki status` command. Plus smarter status-badge polling on the web UI.

### `maki status` — catalog health at a glance

Read-only survey that aggregates signals already exposed by other commands (cleanup dry-run, backup-status, schema-version, embedding / face-scan coverage queries) into one prioritized report. Every actionable item ends with a `→ command` suggestion so users don't have to consult docs to know the next step.

```
$ maki status
Gathering catalog status (scanning derived files; may take a moment)...
MAKI catalog status — /Users/you/.maki

Catalog
  Schema:   v8 (current)
  Counts:   12,847 assets · 18,203 variants · 9,614 recipes · 21,118 file locations
  Storage:  1.8 TB across 3 volume(s) (2 online, 1 offline)

Cleanup
  ✗ 5 locationless variant(s)                          → maki cleanup --apply
  ✗ 47 orphaned embedding file(s) on disk              → maki cleanup --apply

Pending work
  ✗ 28 pending XMP writeback(s) on offline volume(s)   → mount the volumes, then `maki writeback`
  ✗ 142 asset(s) without an embedding                  → maki embed

Backup coverage
  ✗ 124 of 12847 asset(s) (1.0%) have fewer than 2 copies → maki backup-status --at-risk

Volumes
  ● Photos       /Volumes/Photos    10234 asset(s), 1.2 TB [media]
  ● Backup-A     /Volumes/Backup-A  10234 asset(s), 1.2 TB [backup]
  ○ Travel-2026  /Volumes/Travel    810 asset(s), 35 GB [working] (offline)
```

Sections:

- **Catalog**: schema version (with a `run maki migrate` hint if the stored version is older than the constant), asset / variant / recipe / file-location counts, total bytes rolled up from `variants.file_size`, online/offline volume split.
- **Cleanup**: locationless variants and orphan-on-disk counts (previews / smart previews / embeddings / face crops). Reuses the existing `service.cleanup(None, None, false, ...)` dry-run — same passes, same SQL, same disk scan — so the cost matches `maki cleanup --dry-run`. On a 12k-asset catalog this dominates runtime at ~30s; a one-line stderr prelude announces the wait so users don't think the command hung. Suppressed under `--json`.
- **Pending work**: pending XMP writebacks split by online/offline target volume (different message when `[writeback] enabled = false`), assets without an embedding (AI builds), assets with NULL `face_scan_status` (AI builds). All AI fields are `null` on standard builds.
- **Backup coverage**: at-risk count vs total at the configured `--min-copies` (default 2, matching `backup-status`).
- **Volumes**: registered volumes sorted online-first with per-volume asset count + size + purpose tag. `●` = online, `○` = offline.

`--json` emits the full `StatusReport` struct for scripting. Always exits 0 — `status` is informational, not a check.

### Web nav badge: smarter polling

The import-status badge polled `/api/import/status` every 4 seconds unconditionally — fine during an active import, wasteful on an idle tab left open all day (~900 requests/hour for an endpoint with no work to do).

Now:

- **4 s** while a job is running (unchanged responsiveness during imports).
- **30 s** when idle (catches CLI-started jobs without chattering).
- **0** when the tab is hidden — paused entirely until `visibilitychange`, with an immediate refresh on resume so the badge reflects reality before the next tick.
- Cadence swaps the moment `running` flips, no waiting a full cycle to tighten/loosen.

Trade-off: a CLI-started import takes up to 30 s to surface in an open browser tab (vs ~4 s before). That feels right — the user who started a CLI import isn't watching the browser anyway.

### Polish

- User guide chapter 5 (Browse & Search) gains a new "Catalog Health" section explaining when to use `status` vs `stats`.
- New reference page in `doc/manual/reference/04-retrieve-commands.md` with full options, examples, and `SEE ALSO` cross-refs.
- Cheat sheet adds a one-liner; `stats`'s description tightened to "statistics breakdown" to clarify it's no longer the catch-all health command.
- CLAUDE.md command count bumped (44 → 45 / 87 → 88).

Tests: 753 unit + 249 CLI integration + 14 doc on standard build. No new tests for `status` itself — it's pure aggregation of already-tested primitives, and the empty-catalog smoke test confirms structure.

## v4.4.9 (2026-04-27)

Two themes: web import is now reachable from anywhere and survives page reloads; the CLI is more talkative about follow-up commands so users don't get stuck mid-workflow.

### Global import dialog

The import dialog used to live only on `/volumes` and lost its progress feed on page reload. It now:

- Has a **global "Import" nav entry** with a pulsing-dot **status badge** whenever a job is running. Click while a job is in flight and the dialog re-attaches to the live SSE feed instead of opening the volume picker.
- Picks a volume up front: when invoked from the global nav (no volume preselected), the dialog asks which mounted volume to import from. The per-volume buttons on `/volumes` skip this step.
- **Re-attaches to running jobs**: SSE handler subscribes first, replays a 100-event ring buffer of recent events, then chains live broadcast — a page reload mid-import doesn't lose the activity log.
- **Path autocomplete on the subfolder field**: shell-style hierarchical completion. Type to see directory entries from disk, `↑`/`↓`/`Tab`/`Enter`/`Esc`, drill on directory, commit on file. Backed by a new `GET /api/volumes/{id}/browse?prefix=&limit=&filter=&hidden=` endpoint with a `canonicalize().starts_with(mount_canon)` security clamp — `..` traversal and inside-the-mount symlinks pointing outward are rejected with 403. 8 unit tests cover the security boundary including the symlink-escape case.
- **Chip-based tag picker** for "Additional tags", matching the filter-bar UX: autocomplete from `/api/tags`, Enter/comma/click to add, Backspace on empty input removes the last chip. Half-typed text auto-commits on Import / Dry Run so it doesn't get silently dropped. No mode (`=`/`/`) or case (`cc`/`Cc`) toggles — those are search-time concepts irrelevant when applying tags.
- **Subfolder input** stretches full form width like the other fields.
- **"Browse imported" link** actually scopes the result: exact `id:` filter for ≤80 imported assets, falling back to volume + subfolder + `sort=date_desc` for larger batches. Previously it pointed at the unfiltered browse page.
- New `GET /api/import/profiles` endpoint feeds the dialog's profile dropdown, so the partial template carries no template-variable dependencies and works as an include from any page.
- New `GET /api/import/status` reports running totals (`imported`, `skipped`, `locations_added`, `recipes`, `started_at`) in addition to `running` / `job_id`. The nav badge polls this every 4 s.

### Workflow hints

A new pattern — `Tip:` lines at the end of state-changing commands — closes UX gaps where one command leaves the catalog needing a follow-up but doesn't say so. Same shape everywhere: count + action + command:

- **`sync --apply --remove-stale`** → hints `cleanup` when locationless variants linger after stale-location removal. This is the real-world case that prompted the feature: deleting jpgs on disk, running sync, and then being surprised that the variants (often the *selected* preview pick) lingered until the next manual `cleanup --apply`.
- **`dedup --apply`** → same trap; same hint.
- **`fix-roles --apply`** → hints `generate-previews --upgrade` when the best-preview variant changed for some assets (cached previews still reflect the old best).
- **`auto-group --apply`** (standalone) → same: merging donors into a target reorders variants.
- **`generate-previews`** → lists *which volumes were offline* when variants were skipped, instead of silently producing a low file count.
- **`import`** *(ai/pro)* → hints `embed` / `describe` when neither flag was passed and `[import]` config didn't enable them.
- **`rebuild-catalog`** *(ai)* → counts assets without an embedding row and assets with NULL `face_scan_status`; hints `embed` / `faces detect` for each non-zero count. Embeddings restored only if their binary files were on disk; the rest must be regenerated.

### CI fix

`cargo install cargo-about --locked` started silently skipping the `cargo about` binary when 0.9.0 gated it behind a non-default `cli` feature. The Release workflow now passes `--features cli` so license generation works again.

### Polish

- User guide: Web UI chapter rewritten for the global Import nav and the new dialog behaviour. Maintenance chapter notes the post-sync cleanup hint.
- Reference: REST API doc gains a "Volume Management" section (list/register/rename/purpose/remove/browse) and an "Import API" section (start/progress/status/profiles), neither documented before.
- Cheat sheet, tagging poster, search filters card: version bumped to v4.4.9; no content changes (workflow hints don't fit the format).

Tests: 753 unit + 249 CLI integration + 14 doc on standard build. Path-resolution security boundary covered by 8 new tests in `web::routes::volumes`.

## v4.4.8 (2026-04-24)

Tag vocabulary interchange with Lightroom / Capture One, plus two long-standing bugs in scoped maintenance commands.

### Export your MAKI vocabulary to Lightroom and Capture One

```
maki tag export-vocabulary --format text \
    --prune --output ~/Desktop/maki-keywords.txt
```

New `--format text` produces a **tab-indented keyword file** — the format both Lightroom (`Metadata → Import Keywords…`) and Capture One (`Image → Keywords → Import Keywords → Keyword Text File`) accept. Hierarchy is preserved, so `location|Germany|Bayern|München` becomes a nested keyword tree inside the target tool. The curation work you do in MAKI (vocabulary.yaml, `tag rename`, `tag split`) now travels with you into culling sessions in your RAW processor. Default format remains `yaml` for MAKI's own use; existing `--prune`, `--default`, `--output` flags work with both formats.

Output is normalized for the target tools (both LR and C1 silently reject keywords containing certain characters, which aborts the entire import):

- XML entities (`&amp;`, `&lt;`, `&quot;`, numeric `&#NN;`) are decoded to their literal characters. Legacy XMP data occasionally leaks entity escapes into tag names; Capture One's "Invalid character at line N" error on import often points straight at one of these.
- `,` and `;` are replaced with spaces. Both tools treat them as keyword delimiters on import, and they're delimiter-like in MAKI's own tag-input syntax too.
- Whitespace runs collapse; leading/trailing whitespace is trimmed; control chars stripped; tags empty after sanitization are skipped.
- Any sanitized tags are listed to stderr with their before/after form, so you can `maki tag rename` the originals if you want.

### Auto-split on ingest stops comma-tags at the source

The most common source of comma-containing tags — AI auto-tag pulling label strings like `"red, gold, white"` whole from the label file — is now blocked at the single tag-write chokepoint (`QueryEngine::tag`). Tag inputs on **add** run through `normalize_tag_inputs()`: splits on `,` and `;`, collapses whitespace, strips control characters, drops empty segments. Every ingest path shares this chokepoint (CLI `maki tag`, the web UI's add-tags panel, `maki auto-tag --apply`, web-API tag add), so it plugs all of them in one place. Splits emit a one-line `note:` to stderr so the user sees what MAKI turned a single input into. Removes preserve the literal string so existing offending tags can still be cleaned up by their exact catalog name.

### Fix: `maki sync --apply --remove-stale --path <dir>` actually removes missing XMP files now

The sync loop has parallel branches for missing media files and missing recipe (XMP) files. The media-file branch correctly `catalog.delete_file_location()` + updates the sidecar under `apply && remove_stale`; the recipe branch only bumped a counter and moved on. So a catalog with 5 missing XMP files showed the same 5 after `--apply --remove-stale`. The recipe branch now mirrors the media branch: `catalog.delete_recipe(recipe_id) + self.remove_sidecar_recipe(...)` under the same gate. The existing orphaned-asset cleanup at the end of `sync()` then picks up assets whose last location went away.

### Fix: `maki cleanup --path <dir>` no longer mixes path-scoped and whole-catalog counts

`cleanup` runs seven passes — three path-scoped (stale locations, locationless variants, orphaned assets), four catalog-wide (orphaned previews, smart previews, embeddings, face files). The catalog-wide passes compare files under `<catalog_root>/{previews,embeddings,faces}` against the entire catalog — those directories aren't partitioned by volume or path, so restricting the scan to a subset is meaningless. But running them alongside path-scoped passes produced confusing output (e.g. `42 checked, 16 orphaned assets, 5991 orphaned embeddings, 2343 orphaned face files` on a `--path` that only held a handful of recipes). Passes 4-7 now skip entirely when `--volume` or `--path` is set, and the CLI prints a note pointing users at a scope-free `maki cleanup` to catch global orphans.

### Polish

- Cheat sheet: `export-vocabulary --format yaml|text` row added.
- Tagging quick guide: `export-vocabulary --format text` row added.
- Tagging Guide chapter: new "Sharing your vocabulary with Lightroom and Capture One" subsection under *The Vocabulary File*.

Tests: 745 unit + 249 CLI integration + 7 doc (standard). 11 new `tag_util` tests for `normalize_tag_for_storage` / `normalize_tag_inputs`; 8 new `vocabulary` tests for `tags_to_keyword_text` (flat / nested / deduplicated branches / deep hierarchy / empty / no-comments / entity decode / comma+semicolon sanitize / skip-empty).

## v4.4.7 (2026-04-22)

Small feature pack: `tagcount:` search filter, path autocomplete on the filter bar, and a proper in-CLI search filter reference via `--help`.

### `tagcount:N` search filter — count the intentional tags

New numeric filter counting **leaf tags** on each asset — the tags the user actually applied, excluding auto-expanded ancestor paths. An asset tagged `subject|nature|landscape` has 3 stored tags (`subject`, `subject|nature`, `subject|nature|landscape`) but only 1 leaf. `tagcount:` uses the leaf count because that matches what the user intended.

```
maki search "tagcount:0"          # completely untagged
maki search "tagcount:1"          # single-tag assets
maki search "tagcount:5+"         # heavily tagged
maki search "tagcount:2-4"        # lightly-tagged range
maki search "tagcount:0 rating:4+"  # untagged keepers worth reviewing
```

Uses the usual numeric-filter grammar (`N` / `N+` / `A-B` / `A,B`). Especially useful during tag restructuring: `tagcount:0` catches gaps, `tagcount:10+` surfaces noise candidates.

**Storage**: denormalised into a new `leaf_tag_count` column on `assets` (schema v7 → v8) so the filter is a direct indexed comparison, not a JSON-each subquery per row. On large catalogues this is the difference between an interactive filter and a multi-second wait — restructuring queries rarely have other narrowing filters to pre-shrink the row set. The migration backfills existing rows once; all subsequent tag mutations (`tag add`/`remove`/`rename`/`split`/`clear`, reimport, auto-tag, VLM describe) already route through `insert_asset`, which recomputes the count.

Tests: 7 unit tests for `tag_util::leaf_tag_count` (empty, singleton, deep single hierarchy, shared ancestors, mixed flat/hierarchical, case-insensitive, prefix-collision, duplicate guard); 4 parse tests for the filter syntax; end-to-end search test seeding 5 assets with varied tag shapes; regression test asserting the denormalised count stays in sync across tag mutations (catches future drift if a write path bypasses `insert_asset`).

### Path autocomplete on the filter bar

The Path input on the browse page now offers shell-style hierarchical completion. Type to get suggestions at the current directory level; accept a directory (trailing `/`) and the dropdown immediately fetches the next level; accept a file leaf to commit the filter. Focus the field to browse from scratch.

- **Keyboard**: `↑`/`↓` to navigate, `Tab` or `Enter` to accept, `Escape` to close.
- **Wildcards**: typing `*` anywhere suppresses autocomplete (the filter already handles `*` patterns).
- **Absolute paths**: paste a path starting at a registered volume's mount point and the mount prefix is stripped automatically; the dropdown pins to that volume.
- **Volume scoping**: if a volume is selected in the Volume dropdown, suggestions narrow to that volume.

New `GET /api/paths?q=&volume=&limit=` backend (~80 LOC in `src/web/routes/browse.rs`) with SQL-side `GROUP BY` aggregation on a computed next-segment expression — critical for correctness on dense directories. A naive fetch-then-dedupe-in-Rust approach misses sibling directories when one holds thousands of files (the row sample gets monopolised by the dense directory and siblings never appear in the fetched set). With SQL aggregation each directory collapses to one row *before* `LIMIT` applies, so siblings always show up regardless of how many files lives under them. The `substr(relative_path, ?len + 1)` expression uses character-count positions (matching SQLite's TEXT semantics), so prefixes containing multi-byte UTF-8 (`München`, etc.) work correctly.

Frontend (~140 LOC in `templates/filter_bar_js.html`): debounced input (120ms), late-response guard, keyboard nav, accept-and-continue for directories, accept-and-commit for files. Reuses the existing `.tag-autocomplete` dropdown styling.

10 unit tests for the SQL aggregation using an in-memory SQLite, including a regression test for the "dense first sibling hides later siblings" bug (5000 files in directory A, 1 file in B — both must appear).

### `maki search --help` embeds the filter reference

Previously `-h` and `--help` both rendered the same one-line pointer, leaving no on-CLI way to learn the filter syntax. Now:

- `maki search -h` — compact one-liner (unchanged behaviour).
- `maki search --help` and `maki help search` — full categorised reference (~60 lines, fits one terminal screen), grouped into TEXT & METADATA, NUMERIC, DATE, STATUS, PRO, and COMBINING sections with one-line examples per filter.

Implementation: `long_help` arg attribute pointing at a `SEARCH_QUERY_LONG_HELP` string const at the top of `main.rs`. The full per-filter manual page stays at `doc/manual/reference/06-search-filters.md` and the printable 2-page PDF at `maki doc filters`.

### Polish & fixes

- Tagging poster: slight layout fixes — `\raggedright` inside the worked-example tcolorbox so the closing note doesn't justify with wide word gaps; `\sectheadbreak` macro for the "Three places for event-related tags" heading so its two-line subtitle stacks below the title in the narrow sidebar; card-footer paragraphs in event/project/color cards broken into one-sentence-per-line for scannability.
- Search filter quickref PDF: new `tagcount:` row in the numeric-filter table.
- Dependency hygiene: bumped `lofty` to 0.24 (0.23.2 was yanked); transitive dep `core2` (unmaintained, all versions yanked) dropped out when `ravif` went to 0.13 via the `image` 0.25.10 bump. `cargo deny check` now passes clean in CI.

## v4.4.6 (2026-04-20)

Feature release: new `tag split` operation for one-to-many tag restructuring (CLI + web UI), a printable "Tagging Quick Guide" poster, and a capstone illustration in the tagging-guide chapter.

### `maki tag split OLD NEW1 [NEW2 ...] [--keep] [--apply]`

When restructuring tags you often want *one* tag to become *several* at once — the classic cases are migrating an event tag into the canonical pair (`subject|event|wedding-jane-2025` → `event|wedding-jane-2025` + `subject|event|wedding`) and separating a combined tag (`"A & B"` → `"A"` and `"B"`). Previously this required two passes or ad-hoc shell pipelines. `tag split` does it atomically.

```bash
# Restructure into scene-type + specific-occasion in one pass:
maki tag split "subject|event|wedding-jane-2025" \
    "event|wedding-jane-2025" "subject|event|wedding" --apply

# Separate a combined tag:
maki tag split "A & B" "A" "B" --apply

# Add a broader tag alongside the original (additive / copy — keep source in place):
maki tag split "sunset" "color|warm" --keep --apply
```

Semantics:

- **Exact-tag-only**. Operates on assets where OLD is a leaf on that asset. Assets where OLD has descendants (e.g. they also carry `OLD|foo`) are skipped — non-leaf split has ambiguous semantics; use `tag rename` for cascading renames.
- Target tags are expanded to include all ancestor paths, same as regular `tag`.
- **`--keep`** preserves OLD in place (additive / copy mode).
- Accepts the same optional markers on OLD as `tag rename` (`=`, `/`, `^`). The `|` prefix-anchor marker is rejected — split operates on one tag at a time.
- Dry-run by default; `--apply` commits. `--log` shows per-asset action. XMP writeback wired in when enabled.

Seven engine-level unit tests and three CLI integration tests cover basic split, `--keep`, dry-run, non-leaf skip, target-already-present dedup, empty-targets error, and `|` marker rejection.

### Web UI: split-tag modal on the tags page

Matching UI on `/tags`: a second button next to the rename pencil — only on **leaf** rows (non-leaf rows get an invisible alignment placeholder so the grid stays even). Click opens a modal with:

- Source tag shown read-only.
- Two target inputs by default; "+ Add another target" to grow, per-row ✕ to remove.
- "Keep source tag (add alongside instead of replacing)" checkbox — the dialog title flips between **"Replace Tag with Multiple"** and **"Add Tags Alongside"** based on the checkbox so the user sees the mode at a glance.
- Preview → Apply flow identical to the rename modal. Preview shows `N split (of M matched)` without mutating; Apply commits and reloads the page.

Restricting the button to leaves (via the `has_children` flag on the tag-tree entry) avoids the UX confusion where split silently skipped non-leaf-on-asset cases. Backend is a thin `POST /api/tag/split` wrapping `engine.tag_split()`. The button icon is an inline SVG Y-split rather than a font glyph — renders consistently regardless of font support and inherits the button's `currentColor`.

### Tagging Quick Guide poster

New printable quickref at `doc/quickref/tagging.pdf` — A3 landscape, one page, intended as a wall poster beside the monitor. Three bands:

1. **Principles** — 8 short rules in a 4×2 grid with a thin rule between rows for optical separation.
2. **The Facets** — 4×2 grid of coloured facet cards (subject, event, location, person, technique, project, color) with the "When to promote to top-level" decision helper filling the 8th slot. All cards equal height via tcolorbox's `equal height group`. Beside them, a worked-example sidebar shows one photo from Jane's wedding with 9 single-line tags colour-coded by facet, plus a "three places for event-related tags" example distinguishing performing arts / generic gathering scene / specific occasion.
3. **Tag commands** — two-column reference: CLI operations (add / remove / clear / rename / split / expand-ancestors / export-vocabulary) on the left, `tag:` search-filter syntax on the right.

Brand palette consistent with the existing `cheat-sheet.tex` / `search-filters.tex` quickref family. Facet colours (subject=blue, event=salmon, location=teal, person=purple, technique=stone, project=amber, color=rose) are intended for future web-UI facet chips too. New build script `doc/quickref/build-tagging-pdf.sh` matches the existing `build-search-filters-pdf.sh` convention. Release workflow (`.github/workflows/release.yml`) attaches `tagging.pdf` to every future release. Discoverable from the CLI via `maki doc tagging`.

### Tagging guide: capstone illustration

The "Putting it all together" section of the tagging guide now has a proper faceted illustration — a central photo with 9 tag chips arranged in a horizontal ring around it, each chip coloured by its facet. Produced as SVG in the maki-marketing sibling repo (`brand/illustrations/tagging-facets.svg`), rendered to `doc/images/maki-tagging.png` for the manual. Replaces the earlier mermaid flowchart which was structurally incapable of a radial/orthogonal layout.

Supporting fix: the chapter gets an explicit `\clearpage` before this section so the illustration lands on its own page with heading, intro sentence, figure, and all seven facet bullets reading as one coherent unit — instead of the image floating to the next page while the surrounding prose ran on the previous page (the pandoc default).

Prose updates aligned with the new image: the example search query uses `person:Jane` (matching the `person|friend|Jane` chip), and the technique paragraph mentions "silhouette composition" (matching the `technique|composition|silhouette` chip that illustrates a different sub-axis than golden-hour lighting).

### Default vocabulary and docs

- `doc/manual/reference/02-ingest-commands.md` — full command-reference entry for `tag split` between `tag rename` and `tag clear`, with SYNOPSIS / DESCRIPTION / ARGUMENTS / OPTIONS / EXAMPLES / SEE ALSO.
- `doc/manual/user-guide/11-tagging-guide.md` — the migration example in the event-facet discussion now demonstrates `tag split` handling the "one old tag → specific-occasion + scene-type" case in a single command.
- `doc/manual/index.md` — TOC tag subcommand list updated to include `split`.
- Command count in CLAUDE.md: 86 → 87 subcommands (top-level count unchanged at 44).

## v4.4.5 (2026-04-18)

Maintenance release: internal refactoring of the largest files plus a substantial expansion of the tagging guide. No user-visible behaviour changes; all tests pass on both standard and `ai` feature builds.

### Tagging guide: new "Thinking in facets" framework

New subsection in the tagging guide (chapter 11) walks through the *orthogonal-axes* mental model with two worked examples, giving readers the reasoning behind facet decisions rather than just the recommended taxonomy:

- **Events** — specific instances (`event|wedding-jane-2025`) belong in a top-level `event|` facet, not nested under `subject|event`. Date-driven instances pollute the stable subject taxonomy; a specific wedding is not a *kind* of thing photos can depict, it's an *occasion*. Generic ceremony/gathering scene types (wedding, exhibition, workshop, sports event, non-music festival) stay under `subject|event` — they answer a different question. A Jane's-wedding photo typically carries both `subject|event|wedding` (scene type) and `event|wedding-jane-2025` (specific occasion).
- **Color** — dominant color is an independent axis, neither subject nor technique. Recommendation: top-level `color|red`, `color|monochrome`, etc. Includes a caveat about not duplicating MAKI's editorial color-label field if you only ever tag the five standard colors.

Structural updates to the recommended vocabulary:

- `event` and `color` added as opt-in facets alongside the five core ones (subject, location, person, technique, project).
- New `event hierarchy` section with flat-vs-year-grouped naming advice and a three-layer explanation (`subject|performing arts|concert` for performances, `subject|event|wedding` for non-performance gathering scene types, `event|wedding-jane-2025` for specific occasions).
- New `color (optional)` section with a ~15-term starter vocabulary.
- Per-image tag counts and total-vocabulary table updated.
- **Built-in default vocabulary synced to the guide**: `maki init` and `maki tag export-vocabulary --default` now include the top-level `event` and `color` facets, the reordered `subject|event` subtree with a clarifying comment, `subject|object|other`, and `technique|effect|lens flare`. Two new unit tests pin the top-level facet set and the `color|*` leaves so future drift is caught automatically.

### Internal refactoring: largest files broken up

Two refactoring passes (P1+P2 and P3) targeted the four biggest files identified in a fresh QA pass of the codebase after the v4.4.4 release:

**`main.rs` — run_command + build_search_where**:

- `run_faces_command` extracted from `run_command` — 617 lines lifted out into its own function. `run_command`'s Faces arm shrinks from 617 lines to a 5-line delegation.
- Two helpers extracted from `build_search_where`: `add_id_list_filter` (replaces 6 copies of the "id IN (...) from precomputed list" pattern) and `add_location_health_filters` (extracts the ~50-line orphan/stale/missing block). `build_search_where` drops from 467 to 350 lines.

**`web/routes.rs` split into 13 submodules (6,599 → 348 LOC in `mod.rs`, 95% reduction)**:

| Module | LOC | Contents |
|---|---:|---|
| `ai.rs` | 1310 | all `#[cfg(feature="ai")]` handlers |
| `media.rs` | 1056 | compare, serve previews/video, writeback, VLM, export |
| `browse.rs` | 832 | browse/search/asset-page/facets |
| `assets.rs` | 634 | per-asset mutations and batch variants |
| `stacks.rs` | 342 | stack/group/batch-delete handlers |
| `tags.rs` | 334 | tag CRUD + batch_tags |
| `import.rs` | 286 | web import job + SSE progress |
| `duplicates.rs` | 266 | duplicates_page + dedup APIs |
| `calendar_map.rs` | 252 | calendar and map APIs |
| `volumes.rs` | 222 | volume CRUD |
| `saved_search.rs` | 218 | saved-search CRUD |
| `collections.rs` | 217 | collections + batch_group/auto_group |
| `stats.rs` | 172 | stats/analytics/backup + format-groups helper |

`mod.rs` now holds only cross-submodule shared helpers (`resolve_best_variant_idx`, `build_parsed_search`, `merge_search_params`, `resolve_collection_ids`, `intersect_name_groups`, etc.). `web/mod.rs` (the axum router wiring) is unchanged — every handler is still reachable as `routes::handler_name` via `pub use` re-exports.

**Deduplication (P2)**:

- `Volume::online_map(&[Volume])` extracted — 7 identical `HashMap<String, &Volume>` construction sites across `main.rs` and `asset_service.rs` collapsed to one-liners.
- `resolve_collection_ids()` extracted — 13 copies (7 include + 6 exclude) of the collection-name → asset-ID resolution loop across 7 route handlers, factored out following the `intersect_name_groups` pattern.

**CLI output helpers (P3b)**: new `src/cli_output.rs` module holding `format_duration`, `format_size` (consolidating 3 independent implementations that had drifted to different GB precisions), and an `item_status(id, verb, elapsed)` helper for the dominant `"  {id} — {verb} ({duration})"` progress pattern. ~16 call sites in `main.rs` migrated to the new helper, unifying the format across all bulk-operation progress output.

### QA: stale doc references corrected

A pre-release audit surfaced three drifted references:

- `README.md`: command count 39 → 44, added 5 missing commands (`create-sidecars`, `fix-recipes`, `doc`, `licenses`, `update-location`).
- `roadmap.md`: version reference brought up to v4.4.4; v4.4.3 and v4.4.4 milestones added.
- `specification.md`: schema reference v6 → v7; added `face_scan_status` column and `faces.yaml` `recognition_model` persistence note.

### Internals

- New `doc/qa-report.md` — the codebase analysis that drove the refactoring priorities in this release. Identifies top-level LOC distribution (`catalog.rs`, `asset_service.rs`, `main.rs`, `web/routes.rs` were the four largest; last of those is now 13 files), largest functions, duplication hotspots, and prioritised cleanup proposals.
- No schema migration in this release (SCHEMA_VERSION stays at 7).

## v4.4.4 (2026-04-16)

Tag search gets a new disambiguation marker — and the existing one swaps semantics to match the user's natural reading. Targeted fix for a real gap, with a deliberate breaking change while the change is still cheap.

### Whole-path tag match (`tag:=…`) — **breaking change to `=` semantics**

If the catalog contains the same tag at multiple hierarchy levels — e.g. `Legoland` at root, `location|Denmark|Legoland`, and `location|Germany|Legoland` — there was previously no way to select only one of them. `tag:Legoland` matches all three, and the old `tag:=Legoland` (leaf-only-at-any-level) also matched all three since each is a leaf in its own branch.

**New mapping**:

| Marker | Meaning |
|---|---|
| `tag:=Legoland` | **Whole path: full tag value equals "Legoland"** — matches only the root-level standalone tag |
| `tag:/Legoland` | Leaf only at any level — matches all three (each is a leaf) |
| `tag:^Legoland` | Case-sensitive (unchanged) |
| `tag:|Legoland` | Prefix anchor (unchanged) |

Works at any depth: `tag:=location|Denmark|Legoland` matches exactly that path and nothing else.

**Why swap the markers**: `=` reads naturally as "equals" / exact value match, which is what most users instinctively expect. The previous mapping (introduced in v4.3.20) stretched `=` to mean "leaf-only at any level," which fought the intuition. Now `=` matches its visual meaning, and the niche leaf-only semantic moves to `/`.

**Migration**: users with saved searches or scripts using `=foo` for leaf-only-at-any-level should swap to `/foo`. For root-level tags without same-named leaves elsewhere, both old and new `=foo` give identical results — the divergence only appears when the catalog has the same tag at multiple hierarchy levels (the case where disambiguation matters anyway).

### Web UI: tri-state mode toggle on tag chips

The mode badge on each tag chip now cycles through three states instead of two:

```
▼   default — match at any hierarchy level (broadest)
=   whole path — exact tag value only (disambiguates root-level tags)
/   leaf only — match at any level but only as a leaf
```

Cycle order puts `=` first (the more useful narrow mode) so two clicks gets you what you usually want. Tooltips spell out each mode's behavior.

### `tag rename` accepts both markers

In rename, both `=` and `/` collapse to the same behavior: rename only assets where the tag value equals the given path exactly, and skip assets where that tag has descendants. The underlying SQL (`je.value = ?` on `json_each(tags)`) is whole-path equality by construction, so the two markers naturally converge there. The descendant-skip logic on top makes `=Foo` behave correctly as "rename this exact tag, don't touch its children."

### Internals

- `tag_like_parts` in `catalog.rs` parses both markers, runs a single `LIKE '%"stored"%'` for `=` (whole-path) and the existing four-pattern leaf check for `/`. Conflict resolution: `=` wins over `/` (stricter); `|` wins over both.
- New unit tests cover whole-path disambiguation (`search_tag_whole_path_match`) and leaf-only-with-ancestor-expansion behaviour with the swapped marker (`search_tag_exact_with_ancestor_expansion`).

## v4.4.3 (2026-04-15)

User-visible UX win plus a round of data-integrity fixes prompted by living with the v4.4.x face workflow.

### `maki faces detect` no longer re-scans zero-face assets

Previously, every run of `maki faces detect --query "*" --apply` re-scanned every asset without a face record — landscapes, product shots, documents — because the skip logic relied on "does this asset have any face records?" rather than "has this asset ever been scanned?" On large catalogs the wasted work added up to hours per run, and worse, deleting a bad detection would silently recreate it on the next run.

New `face_scan_status` column on `assets` (schema v6→v7) distinguishes "never scanned" from "scanned, regardless of outcome." Detection stamps the flag whenever it completes, and skips on the flag instead. The column is persisted to the Asset YAML sidecar too, so `rebuild-catalog` doesn't lose the scan history.

Practical effect on a 50k-asset catalog with mostly landscapes: first `faces detect` run processes everything, subsequent runs touch only newly-imported assets. Deleted detections stay deleted.

### Dual-storage invariant: data-integrity audit *(internal)*

Audit of `faces.yaml` surfaced one SQLite-only field (`recognition_model`) that violated the "SQLite is derivable from YAML" invariant — a `rebuild-catalog` would have stripped the model tags. Fixed:

- Added `recognition_model` to the YAML `FaceRecord` struct; updated `export_all_faces` and `import_faces_from_yaml` to round-trip it.
- Added `post_migration_sync` hook in `Catalog::open_and_migrate`. When the v5→v6 migration runs (backfilling model tags in SQLite), the hook re-exports `faces.yaml` immediately so the tags also land in the source-of-truth sidecar. Catalogs migrated before this release can run `maki faces export` once to reach the same state.
- The new `face_scan_status` field is in the Asset YAML sidecar from the start. `rebuild-catalog` has a legacy fallback that stamps it on any asset with face records whose sidecar predates the field.

No user-visible behaviour change from the audit — these were bugs lurking behind the `rebuild-catalog` path that would have surfaced later. After this release, `rebuild-catalog` is fully faithful for all face-related state.

### User guide: new "Visual Discovery" chapter

New user-guide chapter (12) covering face recognition, similarity search, and stroll as **workflows** rather than command references. Follows the established Tagging Guide pattern: Why It Matters → three topical workflows → Common Problems → cheat sheet → reference pointers.

Notable content: when to cluster vs. assign per-asset (the "I ran cluster and nothing happened" case is the first Common Problems entry), how to read the `maki faces similarity` histogram to pick a threshold, the three stroll modes and when to use each, maintenance rhythm for face recognition over time.

The chapter title deliberately frames the feature set as "finding photos by what they look like" rather than "AI features" — the operative distinction is content vs. metadata, not implementation.

Chapter order rearranged: Organizing → Tagging Guide → **Visual Discovery** → Archive Lifecycle. All working-with-the-catalog chapters now group together before the long-game storage chapter.

## v4.4.2 (2026-04-15)

Filter bar UX polish — picking up where v4.4.1 left off after live-testing the new face workflow.

### People picker in the filter bar

The browse filter bar's person picker is now a **chip-based multi-select**, sitting on the same line as the tag chip input. Layout: `[tags] [people] [path]` — three wide chip/text inputs of the same shape.

Interactions match the tag chip UX:
- Type to filter, ↑/↓/Enter to add a chip
- Backspace in the empty input removes the last chip
- × on a chip removes just that one
- Esc clears the typing buffer

People chips are tinted teal to visually distinguish from the salmon tag chips — same shape, clearly a different filter dimension.

URL stays backward-compatible: `?person=Alice` (single, from people-page click) still works; chip selection now uses `?person=Alice,Bob` (comma-separated).

### Multiple chips are now AND, not OR

Multiple tag chips and multiple people chips both behaved as OR ("any of these"), which contradicts the natural expectation that "select X and Y in the filter" means "show photos containing **both** X and Y". The bug came from the URL transport collapsing chip values into one comma-separated entry, and the catalog interpreting comma as OR.

Now:
- Two **tag chips** → asset must have **both** tags
- Two **person chips** → asset must contain **both** people
- The documented `tag:a,b` / `person:a,b` syntax in the q field still means OR (escape hatch for power users)

### Internals

- `intersect_name_groups()` helper deduplicates the seven copies of the person-resolver loop and computes intersection-across-entries with OR-within-entry, matching the established tag semantics.

## v4.4.1 (2026-04-15)

Follow-up to v4.4.0's face recognition rewrite — everything in this release is UX polish for the people workflow the new pipeline unlocked. Clustering produces good clusters but often leaves small splinter clusters of the same person alongside a main cluster, and the `/people` page and face-assign dropdown needed to scale beyond a handful of named people.

### People page — merge-multiple-clusters UI *(Pro)*

Select any number of person cards with the checkbox in the corner; a sticky toolbar appears showing the count. Click the ◎ badge on any selected card to pick the merge target (others become sources). Merge opens a confirmation modal with thumbnails and Target/Source badges. One click finalizes the merge. Batch merge goes through a single `POST /api/people/{target_id}/merge` with `source_ids: [...]` — the existing endpoint also accepts this plural form, so the CLI and single-source merges still work.

### People page — automatic merge suggestions *(Pro)*

A new "Merge suggestions" panel surfaces pairs of people whose centroid face embeddings are similar enough to likely be the same person. For each pair:

- Both clusters side-by-side with thumbnails and face counts
- A percentage match score and an arrow showing the default merge direction
- **Swap** button to reverse direction; **Merge** to commit; **Not the same person** to dismiss for the session

Smart defaults: if one side is named, it becomes the target (preserves naming); otherwise the larger cluster becomes the target (keeps the big cluster intact). Dismissals persist in `sessionStorage`; the Refresh button clears them and re-scans.

Backend: `GET /api/people/merge-suggestions?threshold=0.4&limit=20` computes per-person centroid similarity on demand. Scales cleanly to ~2000 people (sub-second).

### People page — filter by name

A text input above the grid filters person cards client-side by name (including the synthesized "Unknown (abc12345)" labels for unnamed clusters). Live count shows "N of M matching". Esc clears. Handles low thousands of cards without any jank.

### Asset detail page — searchable face-assign combobox *(Pro)*

The "Assign to…" dropdown is now a typeahead input. Type to filter the people list, ↑/↓/Enter to navigate and pick, Esc to cancel. Always offers an inline "+ Create new person '\<query\>'" option when there's text. Results capped at 30 rows with a "showing first N of M" footer when truncated — so hundreds of named people don't turn the UI into an unusable scroll-fest.

### Data model

- `/api/people/{id}/merge` accepts either `{"source_id": "..."}` (singular, v4.4.0 API) or `{"source_ids": [...]}` (plural, used by the new batch merge UI).
- New endpoint: `/api/people/merge-suggestions`.
- New `FaceStore` methods: `merge_people_batch`, `suggest_person_merges`.

## v4.4.0 (2026-04-15)

### Face Recognition — Full Pipeline Rewrite *(Pro)*

This release overhauls the face recognition pipeline end to end. The previous version produced cosine similarities clustered in a narrow band (~0.65–0.95 regardless of who was in the image), making auto-clustering effectively unusable. The new pipeline produces a proper bimodal distribution — different people at ~0 similarity, same person at 0.5–0.9 — and clusters cleanly.

Key changes:

- **New recognition model** — ArcFace ResNet-100 FP32 (`onnxmodelzoo/arcfaceresnet100-8`, ~261 MB) replaces the previous INT8 variant (~28 MB). Much better embedding quality.
- **Proper 5-point landmark alignment** — each detected face is warped into a canonical 112×112 template via a least-squares similarity transform before embedding. Matches InsightFace's reference preprocessing. Without alignment, ArcFace treats every face as visually similar regardless of identity.
- **Corrected preprocessing** — the model has normalization nodes (`Sub`, `Mul`) baked into its ONNX graph. MAKI now passes raw `[0, 255]` pixel values and lets the model apply its own mean/std. Previous versions applied the normalization externally as well, double-normalizing and collapsing the embedding space.
- **Agglomerative hierarchical clustering** — replaces the old greedy single-linkage algorithm. Order-independent, uses average linkage (UPGMA) via the Lance-Williams update formula. Produces tighter, better-separated clusters.
- **Model version tracking** — new `recognition_model` column on `faces` (schema v5→v6). Clustering filters to the current model id; old embeddings are skipped with a warning.
- **New defaults** — `face_cluster_threshold` `0.5 → 0.35`, `face_min_confidence` `0.5 → 0.7`. Tuned for the new pipeline.

### New commands *(Pro)*

- **`maki faces clean [--apply]`** — delete unassigned face records. Useful after experimenting with thresholds or after a model upgrade.
- **`maki faces similarity [--query …] [--top N]`** — diagnostic command that prints percentile stats and a histogram of pairwise cosine similarities for a scoped face set. Use it to pick a clustering threshold by finding the valley between inter-person and intra-person humps.
- **`maki faces dump-aligned [--query …]`** — save the 112×112 aligned crops to disk for visual verification of the alignment pipeline.

### New flags

- **`--min-confidence`** on `maki faces cluster` — drop low-confidence face detections before clustering. Defaults to `[ai] face_min_confidence` (0.7).
- **`--force`** on `maki faces detect` — re-detect/re-embed faces even on assets that already have face records. Required when upgrading the recognition model.

### Other improvements

- **`stack from-tag --remove-tags`** now sweeps up orphan tags on single-asset or already-stacked tags too, not just those forming new stacks. Makes it a true post-migration cleanup flag.
- **`tag rename =`** uses leaf-only semantics, matching `=` in search. Only renames assets where the tag has no descendants, skipping ancestor-expanded duplicates.
- **Hierarchical tag search matches at any level** — `tag:Altstätten` now finds `location|Switzerland|Altstätten`, not just root-level entries. Four LIKE patterns cover standalone, parent, leaf-child, and mid-path positions. No substring matching.
- **People filter preserved across pagination and sort** in the browse UI — previously lost on "next page".
- **Unnamed face clusters are browseable from the people page** — clicking "Unknown (abc12345)" now actually filters the browse to that cluster's assets. The filter uses the person's UUID so it works regardless of whether the cluster has been named.
- **Asset detail page shows cluster assignment for unnamed faces** — instead of the "Assign to…" dropdown, unnamed-cluster faces show as a clickable "Unknown (abc12345)" link.
- **Quoting hint on empty search** — when a query returns no results and looks like it has unquoted spaces in a filter (e.g. `tag:foo bar`), MAKI prints a reminder: values with spaces need inner quotes, `tag:"foo bar"`.
- **Asset ID whitespace trimming** — `resolve_asset_id` now trims whitespace (including non-breaking spaces) from the prefix, handling copy-paste artifacts from the web UI.
- **Stronger active state for ∅ filter icons** (rating "unrated", label "unlabeled") — a solid colored border plus bold text, matching the color-dot selection style.
- **`label:none` search filter** — find assets without any color label, matching the existing `rating:0` and `volume:none` patterns. Available in CLI search, web UI filter bar (∅ icon next to color dots), and saved searches.

### Upgrading from v4.3.x

Existing face embeddings are from an older model variant and will not cluster with new ones. They remain in the database untouched but are skipped by clustering with a clear warning (`maki faces status` shows the count).

```
maki faces download              # fetch the ~261 MB FP32 model
maki faces status                # see how many faces are stale
maki faces clean --apply         # delete stale unassigned faces
maki faces detect --force --query <scope> --apply  # re-embed with the new pipeline
```

Schema migration v5→v6 runs automatically on first launch.

## v4.3.20 (2026-04-14)

### New Features
- **`label:none` search filter** — find assets without any color label, matching the existing `rating:0` (unrated) and `volume:none` patterns. Available in CLI search, web UI filter bar (∅ icon next to color dots), and saved searches.
- **Tag search matches at any hierarchy level** — `tag:Altstätten` now finds `location|Switzerland|Altstätten`. Previously only matched root-level parents; now four LIKE patterns cover standalone, parent, leaf-child, and mid-path positions. No substring matching — `tag:eagle` does NOT match `eagles`.

### Bug Fixes
- **`tag rename =` uses leaf-only semantics** — consistent with search `=` behavior. Previously `=` only prevented cascade to descendants but still matched all assets with the exact tag (including expanded ancestors). Now skips assets where the tag also has children, matching the browse UI's exact-level chip behavior.
- **Quoting hint on empty search** — when `maki search` returns no results and the query has both a filter and free text (suggesting forgotten inner quotes), a hint is printed: `tag:"my tag"`.
- **Asset ID whitespace trimming** — `resolve_asset_id` now trims whitespace (including non-breaking spaces) from the prefix, preventing failures from copy-paste artifacts.

### UI
- **Stronger active state for ∅ filter icons** — both "unrated" and "unlabeled" ∅ icons now show a visible border and bold text when active, matching the color dot selection style.

### Documentation
- Tag hierarchy examples use singular form (animal|bird|eagle) matching the recommended convention.
- Search filters reference updated for `label:none` and hierarchical tag matching.
- Quick reference card: dropped Pro explanation line, tightened spacing to fit page 1, added `label:none`, updated version.

## v4.3.19 (2026-04-12)

### Bug Fixes
- **`scattered:` now counts distinct session roots** — previously counted distinct directories, inflating the count for assets with files in Capture/, Selects/, Output/ of the same shoot. Now uses the same session root detection as auto-group, so `scattered:2+` correctly means "files in different shoots." Custom `session_root(path, pattern)` SQLite function with regex caching for performance (~2s on 260k catalog, down from 56s before caching).
- **`copies:` now counts distinct volumes** — previously counted total file location rows. An asset with RAW + JPEG on the same volume showed `copies:2` but wasn't actually backed up. Now `copies:1` matches the backup-status page's "AT RISK" count exactly.
- **Rename autocomplete UX** — Enter without a selection now just closes the list (next Enter triggers preview/apply). Increased suggestion limit from 8 to 30 with scrollable dropdown. Fixed stale suggestions staying visible when typing a new (non-matching) tag name.

### New Features
- **`--default` flag for `tag export-vocabulary`** — exports only the built-in default vocabulary, ignoring catalog tags and existing vocabulary.yaml. Useful for inspecting new default categories after a MAKI upgrade.
- **Subject qualifiers in default vocabulary** — three new cross-cutting branches under `subject`: `style` (vintage, modern, retro, rustic, industrial, classic), `condition` (abandoned, ruined, restored, weathered, pristine), `mood` (dramatic, serene, playful, mysterious, melancholic, joyful).
- **`--path` flag for fix-scattered-groups.py** — scopes both the search AND the analysis to a specific directory tree, so exports and screensaver directories outside the path don't trigger splits.

### Code Quality
- **P1: Deduplicated web route filter parsing** — extracted `build_parsed_search()` helper. 6 route handlers (browse, search, page_ids, calendar, map, facets) migrated from ~50 lines of copy-pasted param extraction each. Net -116 lines.
- **P2: Replaced 7 production `unwrap()` calls** with descriptive `expect()` messages in asset_service.rs and catalog.rs.
- **P3: Standardized error message capitalization** to lowercase across ~177 `anyhow::bail!()` / `anyhow::anyhow!()` messages, matching Rust convention.
- **Section markers and TOC** added to the three largest source files (asset_service.rs 8.6k, catalog.rs 8.6k, query.rs 5.8k lines) for IDE navigation.

### Documentation
- Web UI guide: documented tag rename modal (pencil icon, autocomplete, Enter-Enter workflow) and recipe grouping display.
- Fixed undefined LaTeX reference in manual PDF build.

## v4.3.18 (2026-04-12)

### New Features
- **Tag rename in the web UI** — each tag on the tags page has a pencil icon (visible on hover) that opens a rename modal. The modal has a From (read-only), To (editable with tag autocomplete), Preview (dry run), and Apply button. Enter key acts as Preview first, then Apply once the preview confirms the change — two keystrokes to rename. No more switching between browser and terminal for tag cleanup.

### Bug Fixes
- **`tag:=` exact-level match with ancestor-expanded tags** — with CaptureOne/Lightroom ancestor expansion, tagging an asset `location|Germany|Bayern|Holzkirchen|Marktplatz` also creates standalone ancestor tags `Holzkirchen`, `Bayern`, etc. Previously `tag:=Holzkirchen` still matched this asset because it had the standalone tag. Now the exact-level check also excludes assets where the tag appears as a mid-path component (`|Holzkirchen|...`) in any hierarchical tag — so `tag:=Holzkirchen` correctly means "Holzkirchen is the deepest level, nothing more specific below it."
- **Backup page bar chart** — removed misleading dark track background (looked like it represented data but was just empty space); fixed number alignment when "AT RISK" badge was present by moving the badge before the count.

### Enhancements
- **Variant location count on stats page** — the variants stat card now shows total file locations in parentheses when they differ from the variant count (e.g. "357474 VARIANTS (714948 LOCATIONS)"), matching the recipe card format.

## v4.3.17 (2026-04-11)

### Enhancements
- **Recipe grouping on detail page** — recipes are now grouped by content hash (same as variants are grouped by their locations). An XMP sidecar file on 3 volumes shows as "Recipes (1, 3 locations)" instead of "3 recipes". Different XMP content (e.g. pre/post edit) naturally shows as separate recipe entries. Mirrors the variant display model exactly.
- **Variant and recipe location counts** — both the Variants and Recipes section headers now show "N, M locations" when items exist on multiple volumes. Consistent display format across both sections.
- **Distinct import status for recipe locations** — `--log` output and summary distinguish "recipe" (new content, metadata processed) from "recipe location added" (identical content already known, just tracked). New `recipes_location_added` counter in JSON output.
- **Stats page shows unique recipe count** — the recipe stat card shows unique recipes as the primary number, with total locations in parentheses when backup copies inflate the count.

### New
- **`scripts/sync-backup.sh`** — rsync-based full catalog backup script. Complements the git-based `backup-catalog.sh` (metadata only) by including previews, embeddings, and catalog.db. Checkpoints SQLite WAL before syncing. Supports `--dry-run`, custom destination, and external drive targets. Uses macOS-compatible rsync options (`-rlt` instead of `-a --no-perms`).

### Documentation
- Data model reference: updated Recipe description to explain the variant-parallel grouping concept and the content-hash dedup on import.

## v4.3.16 (2026-04-11)

### Bug Fixes
- **Importing backup volumes no longer re-merges old metadata** *(critical)* — when importing a backup copy of a volume (e.g. after rsync), the XMP sidecar files are byte-identical to the ones MAKI already processed from the original volume. Previously, re-attaching these recipes merged their metadata as if it were new, undoing tag renames, label changes, etc. made in MAKI since the backup was created (because tags merge as union, re-introducing old values). Now MAKI checks whether the asset already has a recipe with the same content hash; if so, the recipe is recorded for backup location tracking but the metadata merge is skipped. Genuinely modified recipes (different hash, e.g. from CaptureOne/Lightroom edits) are still processed normally.

### Documentation
- Import command reference: note that identical recipe copies from backup volumes are tracked but don't re-merge metadata.
- Tag rename reference: added hierarchy refactoring examples (move branch deeper, move to new root, flatten hierarchy, consolidate synonyms with merge).

## v4.3.15 (2026-04-10)

### New Features
- **`tag:|xyz` prefix anchor** — match any tag whose hierarchy component **starts with** `xyz`, at any level (root or descendant). `tag:|wed` matches assets tagged `wedding`, `wedding-2024`, `events|wedding`, `events|wedding|2024-05-12`. Useful for finding tag families with shared prefixes (`2024-*`, `wedding-*`) or for narrowing on short letter combinations like `nen`/`ken` that appear inside many words. Stacks with `^` for case-sensitive prefix anchor (`tag:^|Wed`). The `=` exact-level marker is silently ignored when `|` is present (they conflict — a prefix anchor implicitly includes descendants).
- **`|xyz` autocomplete prefix anchor** — same syntax in the browse-page tag filter dropdown and the tags-page search input. Default substring search is unchanged; type a leading `|` to anchor the query to a hierarchy component start. Also fixes the leaf-suppression filter so intermediate hierarchy levels (e.g. `events|wedding` with descendants below) become selectable when the user is targeting a non-leaf component.
- **`description:` / `desc:` search filter** — case-insensitive substring match against the asset's description column. Unlike free-text search (which matches name + filename + description + source metadata at once), this filter targets only the description, making it useful for finding assets by VLM-generated content or manual captions without noise. Supports negation, comma-OR, and quoted multi-word values like the other text filters.

### Enhancements
- **`maki tag rename` accepts the same `=`/`^` prefix markers as `tag:` search** — closes a consistency gap between search and rename. By default, rename is case-insensitive and cascades to descendants (unchanged). New prefix markers on `OLD_TAG`:
  - `=Foo` — exact level only, does not touch `Foo|child` tags
  - `^Foo` — case-sensitive, treats `Foo` and `foo` as different tags
  - `=^Foo` / `^=Foo` — both, in any order

  Useful for cleaning up case-duplicate tags after spotting them on the tags page: e.g. `maki tag rename "^Landscape" "landscape" --apply` renames only the capitalized variant, leaving the lowercase one alone. The new modes are 100% consistent with the `tag:` search filter syntax — what you can find with search, you can rename. NEW_TAG is always taken literally (no prefix parsing). The `|` prefix-anchor marker is rejected for rename with a clear error: collapsing distinct tags into one is rarely intended; users should compose targeted renames instead.

  Backend: `Catalog::assets_with_tag_or_prefix` extended with `case_sensitive` and `exact_only` flags. Case-sensitive queries use SQLite `GLOB` (matching the `tag:^` search path). Tag rename has 5 new tests covering the new modes and the order-independence of `=^` vs `^=`. Tag search filter has 2 new tests for `tag:|xyz` (including case-sensitive `^|`). The `description:` filter has 5 new tests (parser variants and end-to-end search with negation).

### Documentation
- Search filter reference: expanded `tag:` section with the `|` prefix anchor and the marker combination rules; new dedicated `description:` section with the free-text comparison note.
- `tag rename` reference: new markers documented with a table and 3 new examples (case-sensitive only, exact-level only, combined).
- Cheat sheet and search filter quickref: added `tag:|wed`, `description:cat`, and the `tag rename` marker hint.
- Tag autocomplete placeholder/tooltip in the browse filter bar and tags-page search input now mention the `|xyz` anchored syntax.

## v4.3.14 (2026-04-09)

### New Features
- **Case-sensitive tag matching via `^` prefix** — tag matching is still case-insensitive by default (the right choice for 99% of searches), but you can now prefix a tag with `^` to force a case-sensitive match: `tag:^Landscape` matches `Landscape` but not `landscape`. Useful for cleaning up case-duplicate tags after spotting them on the tags page (which already counts case-sensitively). Stackable with the existing `=` exact-level marker in any order: `tag:=^Foo` or `tag:^=Foo`. Backend uses SQLite `GLOB` instead of `LIKE` for these queries.
- **Per-chip case-sensitivity toggle in the web UI** — each tag chip in the filter bar now has a small `cc`/`Cc` toggle next to the existing `▼`/`=` exact-level toggle. Click to flip that specific chip between case-insensitive (`cc`, default) and case-sensitive (`Cc`). Different chips can have different modes in the same query. State persists through URL round-trips because the `^` prefix is embedded in the tag value itself.
- **Unrated filter (`rating:0`)** — `rating:0` now matches both `rating = 0` and `rating IS NULL` (unrated assets), matching the user's mental model where "unrated" and "0 stars" are the same thing. Any rating filter whose range includes 0 (`rating:0-2`, `rating:0,3`, etc.) is wrapped in `(a.rating IS NULL OR ...)` by a new `rating_clause` helper. Filters that don't match 0 (`rating:3+`, `rating:2-4`) still correctly exclude NULL.
- **∅ marker in the rating filter UI** — a clickable `∅` icon before the stars toggles the `rating:0` filter. Gives the "show me the rest" (unrated) case first-class UI access.

### Bug Fixes
- **Color label filter returned "No results found"** *(regression, since labels were introduced)* — the equality filter helper lowercased the search value (`"Red"` → `"red"`) but labels are stored capitalized (`"Red"`, `"Blue"`, ...). SQLite's default `=` is case-sensitive, so the match always failed. Fixed by using `COLLATE NOCASE` on the SQL clause and preserving the user's original casing. Users can now type `label:red`, `label:Red`, or `label:RED` interchangeably.

### Documentation
- Search filter reference: expanded `rating:` section with the new unrated semantics and SQL behavior for the NULL-handling cases; expanded `tag:` section with `^` case-sensitive syntax and the per-chip UI toggle; updated `label:` SQL behavior to document `COLLATE NOCASE`.
- Cheat sheet: added `rating:0` to the numeric filter table; added `tag:=landscape` and `tag:^Landscape` rows to the text-and-metadata table.
- Search filter quickref (`.tex`/`.md`): added the new tag and rating syntax examples.

## v4.3.13 (2026-04-08)

### New Features
- **License compliance infrastructure** — every release archive now ships `THIRD_PARTY_LICENSES.md` (generated by `cargo-about`) with the full license text of every Rust crate compiled into the MAKI binary. CI runs `cargo-deny` on every push to validate that all dependencies use only permissive open-source licenses (Apache-2.0, MIT, BSD, ISC, MPL-2.0, NCSA, Unicode, Zlib, BSL-1.0, CC0, 0BSD) and to catch security advisories. The release workflow runs the same validation as a gate before building binaries.
- **`maki licenses` command** — new top-level CLI command. Prints MAKI's own license, the third-party Rust crate summary, AI model attribution (Google Research / Hugging Face), and external tool notes. `--summary` for short version, `--json` for scripting.
- **Manual appendix `Licenses & Acknowledgements`** — new chapter at `reference/11-licenses.md` covering MAKI, bundled Rust crates, AI models, and external tools.

### Enhancements
- **Fully permissive dependency tree** — dropped the `viuer` terminal preview dependency, which transitively pulled in `ansi_colours` (LGPL-3.0). This was the last copyleft license in the entire dependency graph; MAKI binaries are now 100% under permissive licenses.
- **`maki preview` simplified** — always opens the asset's preview file in the OS default image viewer (`open` on macOS, `xdg-open` on Linux, `start` on Windows). The inline terminal display via `viuer` is gone (low quality, only worked in iTerm2/Kitty/Sixel terminals), and the now-redundant `--open` flag has been removed.
- **`auto-tag --download <model_id>` positional argument** — `maki auto-tag --download siglip2-large-256-multi` now works as expected. Previously the model id was parsed as the search query and silently fell back to the model in `[ai] model` config.

### Bug Fixes
- **`maki embed` model switching documentation** — multiple docs incorrectly told users to run `maki embed --force` after switching the AI model. The actual behavior is much better: embeddings are keyed by `(asset_id, model_id)`, so `maki embed ''` (without `--force`) only generates the missing embeddings for the new model. This makes model switches restart-safe and saves hours of unnecessary re-embedding on large catalogs.

### Documentation
- **New "Switching models" section** in the setup guide — comprehensive workflow for changing the active AI model without re-embedding everything. Includes verification commands (`sqlite3 .maki/catalog.db ...`), restart-safety guarantees, and disk cleanup notes.
- **CLI reference completeness audit** — added missing entries: `licenses` command in `04-retrieve-commands.md` and the manual index; `dedup`, `update-location`, `fix-roles`, `fix-dates`, `fix-recipes`, `duplicates`, `backup-status`, and `licenses` on the cheat sheet; `siglip2-base-256-multi` and `siglip2-large-256-multi` in the `auto-tag` model table; positional model id syntax for `auto-tag --download`. CLAUDE.md command count updated.
- **Cheat sheet refreshed** to v4.3.13 with all new and previously-missing commands.

## v4.3.12 (2026-04-08)

### New Features
- **Volumes page in web UI** — manage registered volumes from the browser at `/volumes`. List with status badges, register/rename/set-purpose/remove inline, and an Import button on online volumes that opens a modal with profile, tags, auto-group, and smart-preview options. Live progress streamed via Server-Sent Events as files are imported. Plug in a card, register, import, browse — all without dropping into the CLI.
- **`*` wildcards in `path:` filter** — `path:Pictures/*/Capture` matches any year/month folder; `path:*/2026/*/wedding` finds wedding shoots anywhere; `path:*party` does substring search. Patterns without leading `*` stay fast (index scan); leading `*` opts into a full-table scan with the slowdown documented inline. Backward-compatible: existing `path:Pictures/2026` queries behave identically.
- **SigLIP 2 multilingual models** *(Pro)* — two new model variants enable `text:` search in German, French, Spanish, Italian, Japanese, Chinese, and many other languages:
  - `siglip2-base-256-multi` (~410 MB, 768-dim)
  - `siglip2-large-256-multi` (~920 MB, 1024-dim)

  Set `[ai] model = "siglip2-base-256-multi"` in `maki.toml` and run `maki embed '' --force` to re-embed your catalog. Image embeddings are stored per `(asset_id, model_id)`, so the old English embeddings remain available if you switch back. See [AI Models](doc/manual/user-guide/02-setup.md#ai-models-pro) in the setup guide.

### Bug Fixes
- **`auto-tag --download <model>` positional argument** — previously `maki auto-tag --download siglip2-large-256-multi` parsed the model name as the search query and silently downloaded the model from `[ai] model` in config instead. Now positional model ids are accepted when `--download` or `--remove-model` is set.

### Documentation
- New **AI Models** section in setup guide explaining the four SigLIP variants (English/multilingual × base/large), when to switch, and how to migrate.
- New **Volumes Page** section in web UI guide describing the page layout, register form, and import dialog with live progress.
- `path:` filter reference rewritten to document the wildcard syntax with examples and performance notes.
- `text:` filter reference includes a multilingual subsection with the config snippet.

## v4.3.11 (2026-04-07)

### New Features
- **`[group] session_root_pattern`** — configurable regex for auto-group session root detection. Default `^\d{4}-\d{2}` matches date-prefixed directories (e.g., `2024-10-05-wedding`). Users with different directory naming can customize via `maki.toml`. Empty string falls back to parent-directory grouping.

### Bug Fixes
- **Auto-group session root detection** — fixed nested output directories (`Output/Final/Web`) producing wrong session roots. Now correctly finds the deepest date-prefixed directory component.
- **Auto-group directory-local safety** *(critical)* — auto-group now restricts stem matching to files within the same session root by default. Prevents catastrophic cross-shoot merging (e.g., `DSC_0001` from unrelated shoots). Use `--global` to opt into cross-directory matching.

### Enhancements
- **`refresh --exif-only`** — selective EXIF re-extraction without full metadata reimport. Useful for re-reading camera data after parser improvements.
- **Auto-group progress logging** — `--log` shows per-group details in real time during processing.
- **Tag count in detail page** — section header shows the number of tags on the asset.
- **fix-scattered-groups.py** — rewritten to use session root detection (matching maki's Rust implementation), with working Phase 4 re-grouping scoped to affected assets. Computes split-off asset IDs via UUID v5.

### Documentation
- **`[group]` configuration** — new section in configuration reference documenting `session_root_pattern` with examples.
- **Auto-group command reference** — updated to explain session root detection, configurable pattern, and link to config docs.
- Auto-group safety fix, `--global`, `--exif-only`, and progress logging documented across manual, cheat sheet, and CLAUDE.md.

## v4.3.10 (2026-04-06)

### Enhancements
- **Tags page filter persists** across navigation — filter text is saved in sessionStorage, restored when navigating back to the tags page. Enables the tag cleanup workflow: filter → click tag → fix in browse → navigate back.
- **Tag autocomplete refreshes on focus** — picks up CLI tag changes without restarting the server. Server-side `/api/tags` now queries SQLite directly (bypasses stale cache).
- **Ensemble category** in person hierarchy for named groups (band, choir, orchestra, team). Default vocabulary updated.

### Bug Fixes
- **Batch delete showToast error** — delete succeeded but success message failed with "showToast is not defined". Fixed.
- **fix-scattered-groups.py** — disabled the auto-regroup phase that incorrectly regrouped the entire catalog.

### Documentation
- **XMP sidecar prerequisite** — new table with per-tool settings for CaptureOne, Lightroom, RawTherapee, DxO, darktable. Cross-referenced from setup and tagging chapters.
- **Tagging Guide** — clarified overlapping subject categories (person vs performing arts), leaf-level tag counts vs stored totals, ensemble category with examples, singular forms in hierarchy examples.

## v4.3.9 (2026-04-05)

### New Features
- **`maki doc`** — opens documentation PDFs in the browser. `maki doc manual`, `maki doc cheatsheet`, `maki doc filters`. Links to latest GitHub release — always up to date, no local files needed.
- **Web UI documentation links** — the keyboard shortcuts help dialog (`?`) now includes a "Documentation" footer with links to the User Manual, Cheat Sheet, and Search Filter Reference PDFs.

### Documentation
- **Archive Lifecycle** — branded "Asset & Metadata Workflow" illustration replaces the mermaid flowchart, showing the complete data flow from camera to backup with MAKI at the center.

## v4.3.8 (2026-04-05)

### New Features
- **`tag:=X` exact-level match** — prefix with `=` to match assets tagged at exactly this level, excluding those with deeper descendant tags. CLI: `maki search "tag:=location|Germany|Bayern"`. Web UI: click `▼` on a tag chip to toggle to `=` (this-level-only) mode.
- **`rebuild-catalog --asset`** — per-asset rebuild from sidecar YAML. Deletes and re-inserts a single asset's SQLite rows (variants, locations, recipes, embeddings, faces) in seconds, avoiding a full rebuild that takes hours on large catalogs.
- **`[cli]` config section** — default global flags in `maki.toml`: `log`, `time`, `verbose`. OR'd with command-line flags.

### Enhancements
- **Split hardening** — refuses to split off the identity variant (the one that generated the asset UUID). Clear error message with guidance. New asset IDs from split now use the correct DAM_NAMESPACE (consistent with import).
- **Sync `--remove-stale` auto-cleanup** — assets that become locationless after stale removal are automatically deleted with their sidecars.
- **Verify `--max-age` optimization** — queries SQLite for stale locations instead of loading all sidecars. For a 260k-asset catalog with 95% verification, loads ~13k sidecars instead of 260k.
- **Autocomplete intermediate nodes** — tag autocomplete now shows intermediate hierarchy levels when the query matches their last component (e.g., typing "Wolfratshausen" shows both the city and venues below it).
- **Volume label badges** — detail page shows volume labels as styled chips instead of plain text in variant and recipe locations.

### Bug Fixes
- **Split UUID namespace** — split-created assets now get the correct UUID (DAM_NAMESPACE instead of NAMESPACE_URL). Existing wrong IDs can be fixed with `scripts/check-split-ids.py`.

### Documentation
- `[cli]` config section, `rebuild-catalog --asset`, `refresh --reimport`, split identity variant protection, sync auto-cleanup — all documented in reference and cheat sheet.
- **Tagging Guide** — new "How MAKI stores hierarchical tags (the roundtrip)" section explaining the import/writeback cycle.

## v4.3.7 (2026-04-04)

### New Features
- **`maki refresh --reimport`** — CLI equivalent of the web UI "Re-import metadata" button. Clears and re-extracts all metadata (tags, description, rating, label, EXIF) from source files. Also fully re-syncs SQLite with the sidecar YAML, fixing variant/location/recipe mismatches from merge/split operations.

### Bug Fixes
- **Reimport metadata** — now re-extracts EXIF data (camera, lens, date) and recalculates `created_at` from earliest EXIF date. Previously only re-extracted XMP metadata.
- **Reimport SQLite sync** — deletes and re-inserts all variants, file locations, and recipes from the sidecar YAML. Cleans up orphaned SQLite rows from stale merge/group operations. Deduplicates recipes and locations by path.
- **Detail page preview sizing** — preview image no longer shrinks to a thumbnail when the variants table has long file paths. Preview column has a 300px minimum; paths wrap instead of stretching.

## v4.3.6 (2026-04-04)

### New Features
- **Tag vocabulary file** (`vocabulary.yaml`) — predefined tag hierarchy for autocomplete guidance. `maki init` creates a default vocabulary based on the Tagging Guide. Planned-but-unused tags appear in CLI tab completion and web UI autocomplete. Edit the YAML tree to define your vocabulary structure.
- **`maki tag export-vocabulary`** — exports the current tag tree as `vocabulary.yaml`, merging with existing planned entries. Use `--prune` to remove unused entries.

### Documentation
- **Tagging Guide** — new "The Vocabulary File" section covering purpose, editing, bootstrapping, and comparison with AI labels.
- **Reference** — `tag expand-ancestors` and `tag export-vocabulary` command documentation.
- Roadmap cleaned up: completed proposals moved to archive.

## v4.3.5 (2026-04-04)

### New Features
- **Tag hierarchy ancestor expansion** — adding a hierarchical tag (e.g., `person|artist|musician|Peter`) now automatically stores all ancestor paths (`person`, `person|artist`, `person|artist|musician`), matching CaptureOne/Lightroom conventions. Removing a tag cleans up orphaned ancestors (ancestors no longer needed by any other descendant).
- **`maki tag expand-ancestors`** — retroactive cleanup command that expands ancestor paths for existing tags created before this feature. Run once to align your catalog with the new convention.
- **XMP writeback matches CaptureOne format** — `dc:subject` now writes flat individual component names (not pipe-separated paths), `lr:hierarchicalSubject` writes all ancestor paths. Matches what CaptureOne/Lightroom produce.

### Enhancements
- **Web UI autocomplete** — filters to show only leaf tags, suppressing intermediate ancestor entries that would clutter the dropdown.

## v4.3.4 (2026-04-03)

### New Features
- **Tag hierarchy separator aligned with Lightroom/CaptureOne** — `|` (pipe) is now the hierarchy separator everywhere (CLI, web UI, search, display). `>` accepted as alternative input. `/` is now a literal character — no more escaping. Aligned with `lr:hierarchicalSubject` standard.
- **`maki tag clear`** — new subcommand to remove all tags from an asset in one operation.
- **Tag rename cascades to descendants** — renaming a parent tag also renames all descendant tags (e.g., `maki tag rename "localtion" "location"` also renames `localtion|Germany|Bayern` to `location|Germany|Bayern`). Similar prefixes without `|` are not affected.

### Bug Fixes
- **Web UI tag display** — tag chips, autocomplete suggestions, tag page, and stats no longer convert `|` to `/` for display.

### Documentation
- **Quoting guide** in search filter reference — new sections covering spaces, dashes (negation trap), and hierarchy separators in filter values, with quick-reference table.

## v4.3.3 (2026-04-03)

### Bug Fixes
- **Tag rename case-only bug** — `maki tag rename "Livestream" "livestream"` no longer deletes the tag. The case-insensitive check was matching the old tag as "already having the target", causing deletion instead of rename.

### Enhancements
- **Tag rename feedback** — reports three distinct actions: renamed (replaced), removed (merged with existing target), skipped (already correct). Per-asset detail with `--log`.
- **Deterministic YAML output** — `source_metadata` in sidecar files now uses `BTreeMap` (sorted keys) instead of `HashMap` (random order). Eliminates noisy git diffs from key reordering.
- **Git-based catalog backup** — `maki init` creates a `.gitignore` excluding derived files (SQLite, previews, embeddings). New `scripts/backup-catalog.sh` for snapshotting before bulk operations.
- **Bulk ID processing** — new scripting chapter section covering xargs, shell loops, `maki shell` scripts, and stdin-reading commands for operating on lists of asset IDs.

### Documentation
- **Tagging Guide** — refined place name convention (English for countries, local names from regions down), fixed case inconsistencies in hierarchy examples, added note on region language choice.

## v4.3.2 (2026-04-02)

### Enhancements
- **Tag rename: hierarchy-aware ancestor cleanup** — when renaming a flat tag to a hierarchical one (e.g., "Munich" to "location/Germany/Bavaria/Munich"), standalone tags that are now ancestors of the new tag are automatically removed. Prevents redundancy since hierarchical search matches ancestors.
- **Tag rename: case-insensitive matching** — consistent with tag search. `maki tag rename "Concert" "concert"` finds and normalizes all case variants. Ancestor cleanup is also case-insensitive.

## v4.3.1 (2026-04-02)

### New Features
- **`maki tag rename`** — rename a tag across all assets in a single pass. Updates catalog, YAML sidecars, and XMP recipe files atomically. Useful for reorganizing flat tags into hierarchies, fixing typos, or consolidating synonyms.

### Enhancements
- **Sync dry-run feedback** — `maki sync` without `--apply` now shows "Dry run —" prefix and hints for `--apply` and `--remove-stale` when changes or missing files are detected.

### Documentation
- **New chapter: Tagging Guide** (ch 11) — tagging principles, recommended vocabulary structure with five facets (subject, location, person, technique, project), auto-tagging label design, catalog cleanup workflow, IPTC standards, and a quick-start checklist.
- **Volume split and rename reference sections** added to Setup Commands reference.
- **Zero undefined cross-references** in PDF — added explicit pandoc anchor IDs to all headings with `*(Pro)*` suffix or bracket notation.
- **Python scripts** — fixed `dam` → `maki` in `fix-orphaned-xmp.py`; extracted manual examples into standalone scripts (`maki_helpers.py`, `tag-analysis.py`, `backup-audit.py`, `batch-rate-from-csv.py`).

## v4.3.0 (2026-03-29)

### New Features
- **`media` volume purpose** — new purpose for transient source devices (memory cards, card readers). Media volumes are excluded from `backup-status` coverage calculations. Purpose values now follow workflow order: media, working, archive, backup, cloud.
- **Import profiles** — named preset configurations in `[import.profiles.<name>]` sections of `maki.toml`. Profiles override the base `[import]` config; CLI flags override both. Supports all import fields plus `include`/`skip` file type groups. Selected via `maki import --profile <name>`.
- **`maki create-sidecars`** — new standalone command that creates XMP sidecar files for assets with metadata (ratings, tags, labels, descriptions) but no existing XMP recipe. Enables CaptureOne/Lightroom to pick up MAKI metadata. Supports query scoping, volume filter, and report-only dry run.
- **`--create-sidecars` on relocate** — generates XMP sidecars at the destination when copying files to a new volume. Includes `dc:subject`, `lr:hierarchicalSubject`, `xmp:Rating`, `xmp:Label`, and `dc:description`.
- **Auto-label on `volume add`** — label is now optional. When only a path is given, the label is auto-derived from the last path component (e.g., `/Volumes/EOS_DIGITAL` becomes `"EOS_DIGITAL"`).
- **`volume list` filters** — new `--purpose`, `--offline`, `--online` flags for filtering volumes by role and availability. Useful for finding stale card volumes.

### Bug Fixes
- **Variant roles in mixed RAW+non-RAW assets** — `group`, `auto-group`, and `fix-roles` now assign non-RAW variants the `Export` role (was `Alternate`). This gives processed JPEGs/TIFFs priority for preview generation (Export scores 300 vs Alternate 50). `import --auto-group` automatically upgrades previews from export variants after grouping.
- **`fix-roles` scope** — now also corrects `Alternate` non-RAW variants in mixed assets, not just `Original` ones.
- **`dam` → `maki` in scripts** — fixed leftover `dam` references in `scripts/fix-orphaned-xmp.py` from the v4.0.0 binary rename.

### Documentation
- **Card-first workflow** documented in the Archive Lifecycle chapter and import strategies: import from card, cull on smart previews, copy only keepers with XMP sidecars.
- **Command overview tables** on each reference chapter title page (Setup, Ingest, Organize, Retrieve, Maintain).
- **Python scripts** extracted from the manual into `scripts/`: `maki_helpers.py`, `tag-analysis.py`, `backup-audit.py`, `batch-rate-from-csv.py`.
- Comprehensive user guide improvements (see v4.2.2 for the full list).

## v4.2.2 (2026-03-28)

### New Features
- **`duration:` search filter** — filter assets by duration in seconds. Supports exact (`duration:60`), minimum (`duration:30+`), and range (`duration:10-60`) syntax via the unified NumericFilter. Denormalized `duration` column on the assets table for efficient filtering.
- **`codec:` search filter** — filter assets by video codec (e.g. `codec:h264`, `codec:hevc`). Denormalized `codec` column on the assets table. Schema v5.
- **Video proxy generation** — hover-to-play proxy clips in the browse grid. Proxies generated automatically during import and preview generation when ffmpeg is available.

### Documentation
- **New chapter: The Archive Lifecycle** (ch 11) — complete storage strategy with lifecycle diagram, 6-stage workflow (import, cull, archive, backup, verify, export), 3-2-1 backup rule, and a concrete monthly workflow example.
- **Contact sheets** (ch 05) — client proofing, shoot overviews, layout presets, grouping, copyright, and field selection.
- **Deleting assets** (ch 04) — when to delete vs. cull, report-only default, catalog-only vs. physical deletion, batch deletion.
- **Drive failure recovery** (ch 07) — step-by-step playbook from damage assessment through cleanup and backup rebuild.
- **Working with video** (ch 03) — ffprobe metadata, video previews, duration/codec search filters, mixed photo+video shoots.
- **Import strategies** (ch 03) — card reader, tethered shooting, migrating from other DAMs, cloud-synced folders, selective import.
- **Multi-tool round-trips** (ch 07) — concrete CaptureOne/Lightroom scenarios with summary table of which sync command to use.
- **Preview management** (ch 07) — upgrading after external processing, smart previews for offline zoom, force regeneration.
- **Storage hygiene** (ch 07) — expanded duplicate analysis (same-volume vs. cross-volume), backup-status with `--at-risk`/`--min-copies`/`--volume`, piping into relocate.
- **Batch relocate** (ch 07) — `--query` for migrating entire shoots or years, two-pass copy-then-move safety pattern.
- **Export workflows** (ch 05) — ZIP delivery, mirror layout for tool handoff, symlinks for temp working folders.
- **Incremental verification** (ch 07) — `--max-age` for practical weekly runs, `--force` override.
- **Volume split and rename** (ch 02), **fix-recipes** (ch 07), **saved search `--favorite`** (ch 04), **stack `from-tag`** (ch 04), **show `--locations`** (ch 05).

## v4.2.1 (2026-03-26)

### New Features
- **`maki show --locations`** — lists all file locations (variant + recipe) as `volume:path`, one per line. With `--json`, includes variant filename, format, and role.

### Enhancements
- **Compact detail page Type row** — type, format, codec, resolution, framerate, and duration shown as badges in one row. Works for both images (resolution from EXIF) and videos (all fields from ffprobe). Replaces the 3 separate video-only rows.
- **Shared service layer for face detection** — `AssetService::detect_faces()` eliminates CLI/web code duplication and fixes inconsistent force/clear behavior. Web batch detect now uses the same code path as CLI `maki faces detect`.
- **Shared video metadata backfill** — `AssetService::backfill_video_metadata()` replaces identical inline code in CLI and web.
- **Zero compiler warnings** — fixed unused variable and dead code warnings in standard (non-Pro) builds.

## v4.2.0 (2026-03-26)

### New Features
- **Video playback** — HTML5 video player on the asset detail page and in the lightbox. Duration badges (e.g. "1:23") on browse grid thumbnails. Video metadata (duration, codec, resolution, framerate) extracted via `ffprobe` at import time and shown on the detail page.
- **Video metadata backfill** — `maki generate-previews` and the web UI "Regenerate previews" button now run `ffprobe` on existing video assets to backfill metadata that was missing before this feature.
- **Video serving with seeking** — `/video/{hash}` route serves original video files with HTTP range request support for browser seeking.

### Enhancements
- **Preview cache-busting fix** — browse page now busts changed preview thumbnails on all page loads and htmx swaps (not just bfcache restoration).
- **Schema v4** — denormalized `video_duration` column on the assets table for efficient browse card rendering.

## v4.1.3 (2026-03-25)

### New Features
- **`maki volume split`** — split a subdirectory from an existing volume into a new volume. Inverse of `volume combine`: moves matching file locations and recipes with path prefix stripped, source volume preserved. Dry-run by default.
- **`maki volume rename`** — rename a volume label in both `volumes.yaml` and SQLite catalog.
- **`--clear-tags` on `maki edit`** — removes all tags from an asset. Useful for cleaning up merged tags after splitting mis-grouped assets.
- **Improved `scattered:` filter** — now counts distinct directory paths ignoring volume (backup copies in the same relative path no longer count as scattered). New `/N` depth syntax: `scattered:2+/1` compares only the first N path segments, so `2026-03-10/Selects/` and `2026-03-10/Output/` are the same at depth 1.

### Enhancements
- **VLM describe gated behind Pro** — `maki describe`, `import --describe`, and web UI describe buttons now require MAKI Pro.
- **Writeback and sync-metadata gated behind Pro** — `maki writeback` and `maki sync-metadata` now require MAKI Pro. `maki refresh` (read-only) stays in the standard edition.
- **Consistent Pro markers** — all Pro features use subtle *(Pro)* labels in section headers and table entries throughout the manual, cheat sheet, and search filter reference.
- **Doc fixes** — JSON field name `file_locations` → `locations` in docs, missing Pro markers on faces commands and web UI pages.

## v4.1.2 (2026-03-24)

### Enhancements
- **Website link in `--help`** — `maki --help` now shows `https://maki-dam.com` at the bottom for docs, downloads, and support.

## v4.1.1 (2026-03-24)

### Enhancements
- **Star rating filter cycle** — click cycle changed from exact→minimum→clear to minimum→exact→clear for more natural progressive narrowing (e.g. 3+ → 3 → all).
- **Repo structure cleanup** — brand images moved to `doc/images/`, quick reference cards to `doc/quickref/`. Symlinks replaced with relative path resolution.

## v4.1.0 (2026-03-24)

### New Features
- **MAKI Pro edition** — AI builds are now branded as "MAKI Pro". Version string shows `maki 4.1.0 Pro`, web UI footer shows `v4.1.0 Pro`. New `--features pro` build flag serves as product tier above the technical `ai` flag, enabling future non-AI pro features.
- **Search Filter Reference card** — 2-page A4 portrait reference card with all 34 search filters, combining syntax, sort options, output formats, and common recipes. Matches cheat sheet branding. PDF at `doc/quickref/search-filters.pdf`.

### Enhancements
- **Release artifacts renamed** — AI binaries renamed from `-ai` to `-pro` suffix (e.g. `maki-4.1.0-macos-arm64-pro.tar.gz`).
- **GPU acceleration automatic on macOS** — macOS Pro builds now include CoreML support automatically. Users no longer need to know about the `ai-gpu` feature flag.
- **Manual updated for MAKI Pro branding** — all references to `--features ai` in user-facing documentation replaced with "MAKI Pro". New Editions section in the overview chapter. Installation instructions cover pre-built binaries.
- **Cheat sheet updated** — `[AI]` badges replaced with `[Pro]`, "AI Filters" section renamed to "Pro Filters".

## v4.0.12 (2026-03-23)

### Enhancements
- **13 branded screenshots** — all manual screenshots updated with MAKI branding. 6 new views added: lightbox, stroll, map, calendar, analytics, similarity browse, compare.
- **GitHub repo renamed** to `thoherr/maki` (old URLs auto-redirect).

## v4.0.11 (2026-03-22)

### Enhancements
- **Automated binary releases** — GitHub Actions release workflow builds 6 binaries (macOS ARM, Linux x86_64, Windows x86_64 × standard/AI) on tag push. Archives include binary, README, and LICENSE. PDFs attached from repo.

## v4.0.10 (2026-03-22)

### New Features
- **XMP writeback safeguard** — writeback is now disabled by default. Edits to rating, tags, description, and color label are stored safely in the catalog but NOT written to XMP files on disk until `[writeback] enabled = true` is set in `maki.toml`. Prevents accidental modification of Lightroom/CaptureOne XMP files. `maki writeback --dry-run` still works for previewing. Edits are never lost — enable writeback later and run `maki writeback --all` to push all accumulated changes.

## v4.0.9 (2026-03-22)

### New Features
- **Cheat sheet** — 2-page landscape A4 reference card with all 41 commands, search filter syntax, key workflows, and configuration reference. PDF at `doc/quickref/cheat-sheet.pdf`.

### Bug Fixes
- **Group metadata merge** — grouping now keeps the highest rating, first non-None color label and description from donors instead of silently discarding them.
- **`maki init`** — now creates `smart_previews/` directory.

### Enhancements
- **Consistent MAKI/maki naming** — ~81 fixes across 15 manual files: MAKI (uppercase) for the product, maki (lowercase) for the CLI command, DAM → MAKI everywhere.
- **Product overview illustration** — high-res marketing graphic on the manual's first content page.
- **Manual layout** — architecture diagram horizontal items, import pipeline split, auto-group algorithm compact, module dependency graph simplified.
- **Smart preview documentation** — added throughout the manual (overview, ingest, setup, module reference).
- **Windows VLM setup** — Ollama install instructions for Windows.

## v4.0.8 (2026-03-21)

### Bug Fixes
- **`maki init` creates `smart_previews/` directory** — was missing from initialization.
- **`assets/` → `metadata/`** — three documentation references used the old directory name.

### Enhancements
- **Smart preview documentation** — added throughout the manual: overview, ingest chapter (config options, directory structure), setup guide, setup commands, module reference.
- **Manual layout improvements** — architecture diagram with horizontal subgraph items, import pipeline split into two compact diagrams, auto-group algorithm as horizontal flowchart, module dependency graph simplified, table row spacing increased, module table column widths adjusted, diagrams centered when scaled, page breaks for better flow.
- **Windows VLM setup** — Ollama install instructions for Windows added.
- **Config example** — clarified as excerpt, not complete reference.

## v4.0.7 (2026-03-20)

### Bug Fixes
- **`--smart` generated only smart previews** — `generate-previews --smart` now generates both regular thumbnails and smart previews, matching `import --smart` behavior.

### Enhancements
- **Complete CLI documentation audit** — 8 discrepancies fixed: `stack from-tag` and `faces status` subcommands documented, missing options added (`--min-confidence`, `--force`, `--favorite`), command count corrected to 41, `--verbose` added to custom help.
- **Overview chapter restructured** — "Core Concepts" section with horizontal flowchart diagram (Asset highlighted in brand color), FileLocation folded into Variant, Collection and Saved Search added as user-facing entities.
- **PDF manual quality** — zero Unicode warnings (fallback fonts for ⊞ ↗ ℹ ✓), page break before Developer Guide, ER diagram moved to avoid whitespace, mermaid width hints supported in build script.
- **Filter availability table** corrected — all filters work in web UI search box.
- **Button name** — "Generate smart preview" → "Regenerate previews" in docs.

## v4.0.6 (2026-03-20)

### Bug Fixes
- **Large TIFF preview/embedding failure** — 16-bit medium format TIFFs (e.g. 8256×6192 from Fujifilm GFX) exceeded the image crate's default memory limit, causing both preview generation and AI embedding to fail. Removed the limit since files are trusted local content and the decoded image is resized immediately.
- **`--query` in error messages** — auto-tag and embed error messages showed `--query` syntax but query is a positional argument.
- **`*` not a wildcard** — `*` was treated as free-text search matching filenames. Empty string `""` is now used for "all assets" in code and documentation.

### Enhancements
- **Filter availability table** — corrected to show that all filters work in the web UI search box, with dedicated controls highlighted separately.
- **`--query` → positional in docs** — ~30 examples across 3 documentation files updated for auto-tag, embed, describe.

## v4.0.5 (2026-03-20)

### New Features
- **Unified numeric filter syntax** — all numeric search filters (rating, iso, focal, f, width, height, copies, variants, scattered, faces, stale) now support the same consistent syntax: `x` (exact), `x+` (minimum), `x-y` (range), `x,y` (OR values), `x,y+` (combined). For example, `iso:100,400`, `width:1920-3840`, `rating:2,4+` all work.
- **`orphan:false` filter** — new filter for assets with at least one file location (inverse of `orphan:true`).
- **Rating ranges** — `rating:3-5` matches 3, 4, or 5 stars.

### Bug Fixes
- **`*` query matched only ~37 assets** — `*` was treated as free-text search, not a wildcard. Empty string `""` is now used for "all assets" in code, error messages, and documentation.
- **`scattered:2+` silently ignored** — the `+` suffix wasn't stripped. Now works like other numeric filters.
- **`--query` in error messages** — auto-tag, embed, and describe error messages showed `--query` syntax but query is a positional argument.

### Enhancements
- **Unified `NumericFilter` enum** — replaced 20 separate fields with 11 `Option<NumericFilter>`, removing ~100 lines of duplicate parsing and SQL code. One parser (`parse_numeric_filter`), one SQL builder (`numeric_clause`).
- **Complete search filter documentation** — all 34 filters now consistently documented in the quick reference, command reference, and full filter reference.
- **Maintenance cycle diagram** — fixed to show the fork between `sync-metadata` (combined) and separate `refresh` → `writeback` paths.
- **Metadata precedence** — corrected documentation to match implementation (first-set-wins on import, sidecar-overwrites on update).
- **`--log` flag description** — updated to list all 15+ supported commands, not just three.
- **Mermaid diagram line breaks** — `\n` → `<br/>` for correct PDF rendering.

## v4.0.4 (2026-03-19)

### Bug Fixes
- **Tags with double quotes** — tags containing `"` (e.g. `"Sir" Oliver Mally`) now work correctly in browse, search, and tag filtering. Fixed both the SQL LIKE matching (now handles JSON-escaped `\"` form) and the JavaScript string injection (custom `js_string` filter with `|safe` bypass).

### Enhancements
- **Doc tests** — 10 new documentation examples covering `parse_search_query`, `parse_date_input`, `render_template`, `parse_format`, tag utilities, `FileLocation::relative_path_str`, and `Asset::validate_color_label`. These serve as both API documentation and regression tests.
- **Tag matching tests** — 4 new unit tests for tags with special characters (double quotes, apostrophes, ampersands) to prevent regressions.
- **Updated branding** — cover page logo and header icon updated from current marketing assets.

### Documentation
- Updated roadmap with v4.0.1–v4.0.3 completed milestones and Phase 3 auto-stack proposal.
- Added i18n proposal for multi-language manual (English/German).
- Removed redundant catalog structure screenshot (code block is easier to maintain).

## v4.0.3 (2026-03-18)

### New Features
- **Windows support** — full cross-platform path normalization (all stored paths use forward slashes), `tool_available()` uses `where` on Windows, 8MB stack size via MSVC linker flags, `\\?\` extended path prefix handling.
- **GitHub Actions CI** — automated build and test on macOS, Linux, and Windows, both standard and AI feature builds (6 combinations).

### Enhancements
- **Missing tool warnings** — maki now prints a warning (once per tool) when dcraw/libraw or ffmpeg are not found, instead of silently falling back to info card previews.
- **External tools documentation** — changed from "optional" to "highly recommended" with Windows install commands (winget/scoop).
- **README branding** — replaced text title with MAKI logo and tagline.

## v4.0.2 (2026-03-18)

### New Features
- **Similarity browse** — "Browse similar" button on the detail page navigates to the browse grid with `similar:<id>` query. Cards show similarity percentage badges. `min_sim:` filter accepts 0-100 percentage threshold (e.g. `min_sim:90`). Auto-sorts by similarity. Source asset included at 100%.
- **Stack by similarity** — "Stack similar" button on the detail page finds visually similar assets via embedding index and creates a stack with the current asset as pick. Configurable threshold (default 85%).
- **Stack management in browse toolbar** — context-sensitive buttons appear based on selection: "+ Stack" (add unstacked assets to an existing stack), "− Stack" (remove from stack), "Set pick" (set stack representative).
- **Stack management on detail page** — "Remove from stack" button for stacked assets.

### Enhancements
- **Filter bar layout** — reorganized into two rows: tag filter and path prefix side-by-side on top, rating stars, color dots, and dropdown selectors on the bottom row. Dropdowns reordered: collections, people, types, formats, volumes.
- **Sort by similarity** — new "Similarity" sort button in browse toolbar when viewing similar results.

### Bug Fixes
- **`--mode tags` used wrong prompt** — tags mode was using the config's describe prompt instead of the JSON tags prompt.
- **Prose VLM responses no longer fail** — saved as description with a helpful note instead of erroring.

## v4.0.1 (2026-03-17)

### New Features
- **Default browse filter** — new `[browse] default_filter` option in `maki.toml` applies a persistent search filter to all browse, search, stroll, analytics, and map views. Uses standard search syntax (e.g. `"-tag:rest"`, `"rating:1+"`). A toggle in the web UI filter bar lets you temporarily disable it. Not applied to operational commands like `export` or `describe`.

### Bug Fixes
- **`--mode tags` used wrong prompt** — tags mode was using the config's describe prompt instead of the JSON tags prompt, causing models to return prose instead of structured tags. Now always uses the correct tags-specific prompt.
- **Prose VLM responses no longer fail** — when a model returns prose instead of JSON tags, the response is saved as a description with a helpful note, instead of reporting an error.

### Documentation
- **New manual chapter**: *Organizing and Culling* — covers rating vs. curation, tag-based and rating-based culling workflows, the default filter feature, and practical workflow examples.
- **Configuration reference** updated with `[browse]` section documentation.

## v4.0.0 (2026-03-16)

### Breaking Changes
- **Renamed binary from `dam` to `maki`** — the CLI command is now `maki` (Media Asset Keeper & Indexer). All subcommands work identically: `maki init`, `maki import`, `maki search`, etc. Existing users should rename `dam.toml` to `maki.toml` and `~/.dam/` to `~/.maki/`. For backward compatibility, `maki.toml` lookup falls back to `dam.toml` with a deprecation notice.
- **Configuration file renamed** — `dam.toml` → `maki.toml`. The old filename is still accepted with a warning.
- **Data directory renamed** — `~/.dam/` → `~/.maki/` (AI models, shell history). Old paths are not auto-migrated.

### New Features
- **MAKI brand identity** — full visual rebrand of the web UI with brand color palette (salmon/coral for images, amber for video, teal for audio, nori blue for documents), favicon, SVG logo in navigation bar, asset type color-coded badges, Inter font family, and updated light/dark mode palettes.
- **Branded PDF manual** — custom cover page with MAKI logo and tagline, branded headers and footers throughout.

### Enhancements
- **All documentation updated** — README, user manual, command reference, architecture docs, and CHANGELOG updated with the new command name, config filename, and data paths. ~4,300 references across ~60 files.

## v3.2.6 (2026-03-15)

### Enhancements
- **Document `maki import --describe` flag** — the `--describe` flag for generating VLM descriptions during import was missing from the command reference. Now fully documented with usage, config equivalent, and JSON output keys.
- **Consolidate planning documents** — removed 4 obsolete planning files from `doc/proposals/archive/` (superseded roadmap, idea notebook, completed enhancement lists). Retained 10 design documents for implemented features as architectural reference. Updated roadmap with current status.
- **Thread verbosity through web server** — `--verbose` / `-v` flag now works with `maki serve`, showing VLM prompts, timing, and operational flow in server logs. Previously all web routes silently used quiet mode.

## v3.2.5 (2026-03-15)

### New Features
- **Per-model VLM configuration** — `[vlm.model_config."model-name"]` sections in `maki.toml` let you override `max_tokens`, `temperature`, `timeout`, `max_image_edge`, `num_ctx`, `top_p`, `top_k`, `repeat_penalty`, and `prompt` per model. Parameters merge: per-model overrides global, CLI overrides both.
- **Ollama sampling parameters** — new `num_ctx`, `top_p`, `top_k`, `repeat_penalty` fields in `[vlm]` config and as CLI flags (`--num-ctx`, `--top-p`, `--top-k`, `--repeat-penalty`). Passed in Ollama `options` object; `top_p` and `repeat_penalty` also sent to OpenAI-compatible endpoints.
- **VLM image resizing** — new `[vlm] max_image_edge` config (and per-model override) resizes images before sending to the VLM, reducing vision encoder processing time and preventing timeouts on memory-constrained machines.
- **Pending writeback indicator** — the asset detail page now shows an orange sync icon on recipes with pending XMP write-back changes (edits made while the volume was offline). A "Write back to XMP" button replays queued edits when the volume comes online.

### Enhancements
- **Default VLM timeout increased** — raised from 120s to 300s to accommodate model swapping on memory-constrained machines (Ollama unloads/reloads when switching models).

## v3.2.4 (2026-03-15)

### New Features
- **VLM model selector in web UI** — when `[vlm] models` is configured in `maki.toml`, a dropdown appears next to the "Describe" button on the asset detail page and the batch Describe button in the browse toolbar, letting you choose which VLM model to use per request.

### Enhancements
- **Thinking model support** — Qwen3-VL and other models that use `<think>` reasoning tags now work correctly. maki sends `think: false` to disable extended thinking and strips any `<think>...</think>` tags from responses.
- **Ollama-first endpoint order** — VLM calls now try the Ollama native API (`/api/generate`) first, falling back to the OpenAI-compatible endpoint (`/v1/chat/completions`) on 404. This avoids a double round-trip for Ollama users and ensures `think: false` is honored.
- **Default max_tokens increased** — VLM default `max_tokens` raised from 200 to 500, giving models enough headroom for detailed descriptions.

### Bug Fixes
- **Fix buildSearchUrl error** — batch describe, batch auto-tag, and batch detect-faces no longer show a "buildSearchUrl is not defined" error after completion.

## v3.2.3 (2026-03-14)

### New Features
- **`--verbose` (-v) global flag** — shows operational decisions and program flow to stderr. Placed between `--log` and `--debug` in verbosity hierarchy. `--debug` implies `--verbose`. Shows info like file counts, volume detection, exclude patterns, VLM endpoint/model/mode, search query details, and preview generation method.
- **`maki edit --role --variant`** — change a variant's role (original, alternate, processed, export, sidecar) from the CLI. Updates both YAML sidecar and SQLite catalog, recomputes denormalized columns.
- **`maki cleanup --path`** — scope stale-location scanning to a path prefix instead of full volume. Absolute paths auto-detect the volume and convert to relative prefix.
- **Locationless variant pruning** — new cleanup pass removes variants with zero file locations from assets that still have other located variants. Prevents ghost variants from accumulating after file moves or reimports.

#### Web UI
- **Variant role dropdown** — inline dropdown selector on asset detail page variants table for multi-variant assets, with immediate save via API.
- **Modal keyboard handling** — Enter confirms and Escape cancels in all custom modal dialogs (group merge, export, batch delete). Default button receives focus on open.

### Enhancements
- **Improved VLM error messages** — detect empty responses (with `finish_reason` hints), unexpected formats, and suggest `ollama ps` for Ollama-specific issues. Show configured model at startup with availability warning.
- **VLM Model Guide** — new reference document (`doc/manual/reference/10-vlm-models.md`) with tested models, backends, and hardware recommendations.

## v3.2.2 (2026-03-14)

### New Features
- **CLI `--zip` export** — `maki export <query> <target> --zip` writes a ZIP archive instead of copying files to a directory. Appends `.zip` extension if missing. Layout, all-variants, and sidecar options work the same as directory export.
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
- **Writeback reference entry** — added formal `maki writeback` section to the maintain commands reference (SYNOPSIS, OPTIONS, EXAMPLES, SEE ALSO), matching the format of all other commands.
- **Manual index completeness** — updated command lists to include all documented commands (added `delete`, `split`, `embed`, `preview`, `contact-sheet`, `backup-status`, `stack`, `faces`, `sync-metadata`, `writeback`, `dedup`, `fix-recipes`, `migrate`).
- Fixed stale version reference in shell example output.

## v3.2.0 (2026-03-14)

### New Features
- **Web UI export as ZIP** — download selected assets or all filtered results as a ZIP archive directly from the browser. "Export" button in the batch toolbar for selected assets; "Export all" link in the results bar for the current search/filter state. Modal dialog offers layout (flat/mirror), all-variants, and include-sidecars options. Backend streams the ZIP via a temp file to handle large exports. New `POST /api/batch/export` endpoint accepts either explicit asset IDs or the full set of browse filter parameters (type, tag, format, volume, rating, label, collection, path, person).

### Bug Fixes
- **Dark mode modals** — fixed unreadable text in group-confirm and export modals by using correct CSS variables (`--text`, `--bg-input`) instead of undefined `--text-main` and `--bg-hover`.

## v3.1.0 (2026-03-13)

### New Features
- **`maki preview`** — display asset preview images directly in the terminal using viuer (auto-detects iTerm2, Kitty, Sixel, Unicode half-block fallback). Also available as a shell built-in (`preview $picks`). `--open` flag launches the preview in the OS default viewer instead.

### Enhancements
- **Consistent positional query** — `writeback`, `fix-dates`, `fix-recipes`, `sync-metadata`, `describe`, `auto-tag`, and `embed` now accept a positional search query as the first argument (same syntax as `maki search`), replacing the previous `--query` flag. Example: `maki describe "rating:4+"` instead of `maki describe --query "rating:4+"`.
- **Shell variable expansion** — all seven commands above now support shell variable expansion (`$var`, `_`) via hidden trailing asset IDs, so `describe $picks` and `writeback _` work in the interactive shell.
- **Scope filtering for writeback** — `maki writeback` can now be narrowed by query, `--asset`, or `--volume` to process only matching recipes instead of the entire catalog.
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
- **`maki shell`** — interactive asset management shell with readline-based REPL, replacing one-shot CLI invocations for interactive workflows. Features:
  - **Named variables** — `$picks = search "rating:5 date:2024"` stores result sets; `$picks` expands to asset IDs in any subsequent command
  - **Implicit `_` variable** — always holds asset IDs from the last command
  - **Session defaults** — `set --json` / `set --log` / `set --debug` / `set --time` auto-inject flags into all commands
  - **Tab completion** — subcommands, `--flags`, `$variables`, `tag:names`, `volume:labels` (cached from catalog)
  - **Script files** — `maki shell script.maki` executes `.maki` files with variables, comments, and shared session state
  - **Single-command mode** — `maki shell -c 'search "rating:5"'` for one-liners in external scripts
  - **`--strict` flag** — exit on first error in scripts and `-c` mode
  - **`source <file>`** — execute a script inline, sharing the current session's variables and defaults
  - **`reload`** — re-read config, refresh tab completion data, clear variables and defaults
  - **Smart quote handling** — `search text:"woman with glasses"` works without multi-level quoting (mid-token quotes preserved, token-wrapping quotes stripped)
  - **Blocked commands** — `init`, `migrate`, `serve`, `shell` are rejected with a clear message
  - **History** — persisted to `.maki/shell_history` in the catalog directory

### Enhancements
- **`maki --help` reorganization** — `serve` and `shell` grouped under new "Interactive" category (previously `serve` was under "Retrieve")

## v2.5.3 (2026-03-12)

### Enhancements
- **Concurrent VLM requests** — the `[vlm] concurrency` setting is now fully functional. Set `concurrency = 4` in `maki.toml` to process multiple assets in parallel during `maki describe`, `maki import --describe`, and web UI batch describe. Uses scoped threads with chunked processing: preparation and result application remain sequential (catalog writes), while VLM HTTP calls (base64 encoding + curl) run concurrently. Default remains `1` (sequential) for backward compatibility.

## v2.5.2 (2026-03-12)

### New Features
- **`variants:` search filter** — filter by variant count per asset. `variants:3` (exactly 3), `variants:5+` (5 or more). Uses denormalized `variant_count` column — no JOIN needed.
- **`scattered:` search filter** — find assets whose variants span multiple directories. `scattered:2` finds assets with file locations in 2+ distinct volume:directory combinations. Useful for auditing mis-grouped assets after import.
- **Configurable `text:` search limit** — the result count for AI text-to-image search is now configurable at three levels: inline syntax `text:"query":100`, `[ai] text_limit` in `maki.toml` (default 50), and hardcoded fallback of 50. Applies to both CLI and web UI.
- **Re-import metadata** — button on the asset detail page that clears tags, description, rating, and color label, then re-extracts from variant source files (XMP sidecars and embedded XMP in JPEG/TIFF). Useful for cleaning up metadata after splitting mis-grouped assets.

### Bug Fixes
- **Stale browse after detail mutations** — dissolving a stack, changing the pick, or other detail page mutations now mark the browse page as dirty. On back-navigation (including bfcache), the browse grid automatically refreshes.
- **Stale stack pick on back-navigation** — browse page now sends `Cache-Control: no-store` to prevent the browser from serving stale HTML on back button.
- **Silent error on preview regenerate** — regenerate/rotate preview buttons are now hidden when source files are offline. If the volume goes offline mid-session, an error message is shown instead of a silent 500.

## v2.5.1 (2026-03-11)

### New Features
- **Analytics dashboard** (`/analytics`) — shooting frequency, camera/lens usage, rating distribution, format breakdown, monthly import volume, and storage per volume charts. Accessible from the nav bar under Maintain.
- **Batch relocate** — `maki relocate --query <QUERY> --target <VOLUME>` moves entire search results to a target volume in one command. Also supports stdin piping (`maki search -q "..." | maki relocate --target <VOL>`) and multiple positional IDs. Backward compatible with the existing single-asset `maki relocate <ID> <VOL>` syntax.
- **Drag-and-drop** — drag browse cards onto the collection dropdown to add assets to a collection. Drag stack members on the detail page to reorder (drop to first position sets the pick). Visual feedback with drop highlights and toast notifications.
- **Per-stack expand/collapse** — click the stack badge (⊞ N) on a browse card to expand or collapse just that stack, independent of the global collapse toggle. When globally expanded, clicking a badge collapses only that stack; re-clicking restores it.

### Bug Fixes
- **Stack member count on detail page** — detail page now shows all stack members including the current asset, fixing an off-by-one where the pick was excluded from the member list.
- **Per-stack expand with global expand** — clicking the stack badge when stacks were globally expanded no longer adds duplicate cards. Now correctly hides non-pick members of just that stack.
- **Keyboard focus preservation** — global stack toggle and htmx swaps now preserve focus by asset ID instead of grid index, preventing focus from jumping to the wrong card.

## v2.5.0 (2026-03-11)

### New Features
- **`text:` semantic search filter** — natural language image search using SigLIP's text encoder. Encode a text query into the same embedding space as image embeddings and find matching images via dot-product similarity. Supports quoted multi-word queries: `text:"sunset on the beach"`, `text:"colorful flowers" rating:3+`. Returns top 50 results, composable with all other filters. Requires `--features ai` and embeddings generated via `maki embed` or `maki import --embed`. Available in CLI, web UI, and saved searches.
- **`maki import --describe`** — auto-describe imported assets via VLM as a post-import phase. Checks VLM endpoint availability (5s timeout), then calls the configured VLM for each new asset. Silently skips if endpoint is not reachable. Can be enabled permanently via `[import] descriptions = true` in `maki.toml`. JSON output includes `descriptions_generated`, `descriptions_skipped`, and `describe_tags_applied` keys.

## v2.4.2 (2026-03-10)

### New Commands
- **`maki describe`** — generate image descriptions and tags using a vision-language model (VLM). Sends preview images to any OpenAI-compatible API server (Ollama, LM Studio, vLLM) — no feature gate or special build needed. Three modes: `--mode describe` (default, natural language descriptions), `--mode tags` (JSON tag suggestions), `--mode both` (two separate VLM calls for description + tags). Report-only by default; `--apply` writes results. `--force` overwrites existing descriptions. `--dry-run` skips VLM calls entirely. Supports `--json`, `--log`, `--time`.

### New Features
- **VLM web UI integration** — "Describe" button on asset detail page and batch "Describe" button in browse toolbar. VLM availability detected at server startup with a 5-second health check. Buttons hidden when no VLM endpoint is reachable.
- **Configurable VLM temperature** — `--temperature` CLI flag and `[vlm] temperature` config option (default 0.7) control sampling randomness. Lower values (0.0) give deterministic output; higher values give more varied results.
- **`[vlm]` configuration section** — full VLM config in `maki.toml`: endpoint, model, max_tokens, prompt, timeout, temperature, mode, concurrency. CLI flags override config values.
- **Truncated JSON recovery** — VLM tag responses that are cut off by max_tokens are salvaged: complete JSON strings are extracted from partial arrays.
- **Tag deduplication** — VLM-suggested tags are deduplicated case-insensitively before merging with existing asset tags.
- **Ollama native API fallback** — if the OpenAI-compatible `/v1/chat/completions` endpoint returns 404, automatically falls back to Ollama's native `/api/generate` endpoint.

## v2.4.1 (2026-03-09)

### New Features
- **CoreML GPU acceleration** — new `--features ai-gpu` enables CoreML execution provider on macOS for SigLIP and face detection/recognition. `[ai] execution_provider` config option (`"auto"`, `"cpu"`, `"coreml"`). Shared `build_onnx_session()` helper with automatic CPU fallback. Linux CUDA and Windows DirectML tracked as roadmap items.
- **Clickable tags on detail page** — tag chips on the asset detail page link to `/?tag=...` for browsing by tag. Sets `maki-browse-focus` before navigating so the browse page scrolls to the originating asset.

### Bug Fixes
- **Fix stroll page Escape key navigation loop** — popstate handler was pushing new history entries, creating an infinite back loop. Added `skipPush` parameter and history depth tracking.
- **Fix stroll Escape exiting browser fullscreen** — added fullscreen guard; uses `history.back()` instead of `location.href` assignment.
- **Defer stroll Escape navigation (150ms)** — keyup event was firing on bfcache-restored page, causing immediate fullscreen exit. `setTimeout(150)` lets keyup complete first.
- **Apply deferred Escape to detail and compare pages** — same fullscreen fix pattern as stroll for consistent behavior across all pages.

## v2.4.0 (2026-03-09)

### New Commands
- **`maki contact-sheet`** — Generate PDF contact sheets from search results. Image-based rendering at 300 DPI with configurable layout (dense/standard/large), paper size (A4/letter/A3), metadata fields, color label display (border/dot/none), section grouping (date/volume/collection/label), and copyright text. Smart previews used by default with fallback to regular. Configurable via `[contact_sheet]` in `maki.toml` and CLI flags.
- **`maki split`** — Extract variants from an asset into new standalone assets. Each extracted variant becomes a separate asset with role `original`, inheriting tags, rating, color label, and description. Associated recipes move with the variant. Available via CLI, web API (`POST /api/asset/{id}/split`), and detail page UI (variant checkboxes + "Extract as new asset(s)" button).

### New Features
- **Alternate variant role** — New `alternate` role (score 50) for donor originals during grouping and import. Replaces the semantically incorrect `export` role when re-roling donor variants in `group`, `auto-group`, `split`, `import` (RAW+JPEG pairs), and `fix-roles`. Ranks below `original` (100) for preview selection, reflecting "second best" status.
- **Group button in web UI** — Direct merge of selected assets (distinct from "Group by name" which uses stem matching). Focused asset (keyboard navigation) becomes the merge target. Thumbnail confirm modal shows all selected assets with target highlighted.
- **Grouped help output** — `maki --help` now shows commands organized by category (Setup, Ingest & Edit, Organize, Retrieve, Maintain) with section headers. Output paginated through `less` when stdout is a terminal.
- **Browse selection fix** — Selection cleared on forced page reload (Ctrl+Shift+R) but preserved across back-navigation and query changes for shopping-cart workflow.
- **Group confirm modal** — Visual confirmation dialog with thumbnails of selected assets before merging, replacing plain text confirm. Off-page assets show ID placeholder.

### Bug Fixes
- Contact sheet footer version printed without "v" prefix for consistency
- Fixed stale "exports" wording in group comment and confirm dialog

## v2.3.5 (2026-03-09)

### New Features
- **`maki sync-metadata` command** — bidirectional XMP metadata sync in a single command. Phase 1 (Inbound): detects externally modified XMP recipe files and re-reads metadata. Phase 2 (Outbound): writes pending DAM edits to XMP. Phase 3 (Media, with `--media`): re-extracts embedded XMP from JPEG/TIFF files. Detects conflicts when both sides changed. Supports `--volume`, `--asset`, `--dry-run`, `--json`, `--log`, `--time`.
- **`id:` search filter** — query assets by UUID prefix in both CLI and web UI. `maki search "id:c654e"` matches assets whose ID starts with the given prefix.

### Enhancements
- **Comprehensive derived file cleanup** — `maki cleanup`, `maki delete`, and `maki volume remove` now handle all derived file types: regular previews, smart previews, SigLIP embedding binaries, face crop thumbnails, ArcFace embedding binaries, and embedding/face DB records. Previously only regular previews were cleaned up, leaving orphaned files to accumulate.
- **Seven-pass cleanup** — `maki cleanup` now runs 7 passes (up from 3): stale locations, orphaned assets (with full derived file removal), orphaned previews, orphaned smart previews, orphaned SigLIP embeddings, orphaned face crops, and orphaned ArcFace embeddings. New counters reported in both human and JSON output.

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
- **`maki writeback` command** — writes back pending metadata changes (rating, label, tags, description) to XMP recipe files. When edits are made while a volume is offline, recipes are automatically marked `pending_writeback`. The new command replays writes when volumes come online. Flags: `--volume`, `--asset`, `--all`, `--dry-run`. Supports `--json`, `--log`, `--time`.

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
- **`stroll_discover_pool` config** — `maki.toml` `[serve]` section supports `stroll_discover_pool` (default 80) to control the candidate pool size for Discover mode.

## v2.3.1 (2026-03-08)

### Enhancements
- **Elliptical satellite layout** — stroll page satellites now follow an elliptical orbit that adapts to the viewport aspect ratio, using more horizontal space in landscape and more vertical space in portrait orientations.
- **Fan-out slider** — replaces the depth slider (0–8) with a fan-out slider (0–10) that shows transitive L2 neighbors behind focused satellites. Focused satellite pulls 30% toward center when fan-out is active to make room for L2 thumbnails.
- **Direction-dependent L2 radius** — L2 neighbor arcs spread wider horizontally and narrower vertically, making better use of available screen space.
- **L2 thumbnail metadata** — L2 (transitive neighbor) thumbnails now show name, rating, color label, and similarity score, consistent with L1 satellite display.
- **L1/L2 keyboard navigation** — Arrow Up/Down moves between L1 satellites and their L2 neighbors. Hover suppression during keyboard navigation prevents focus catch-back.
- **Stroll slider configuration** — `maki.toml` `[serve]` section supports `stroll_neighbors`, `stroll_neighbors_max`, `stroll_fanout`, and `stroll_fanout_max` to configure stroll page slider defaults and ranges.

## v2.3.0 (2026-03-07)

### New Features
- **Stroll page** (feature-gated: `--features ai`) — graph-based visual similarity exploration at `/stroll`. A center image surrounded by radially arranged satellite images shows visually similar assets. Click any satellite to navigate — it becomes the new center with fresh neighbors. Features: viewport-adaptive sizing, smart preview loading, keyboard navigation (arrow keys cycle satellites, Enter navigates, `d` opens detail page), rating stars and color label dots on all images, similarity percentage badges, browser history integration (`pushState`/`popstate`). Neighbor count adjustable via slider (5–25, default 12) in a fixed bottom-left overlay. Entry points: nav bar "Stroll" link, `s` keyboard shortcut on browse/lightbox/detail pages, "Stroll from here" button on detail page, or direct URL `/stroll?id=<asset-id>`. Without an `id`, picks a random embedded asset.
- Stroll page depth slider (0–8) for exploring neighbors-of-neighbors — lazy-loaded, cached, with deduplication and fade-in animation
- **`similar:` search filter** (feature-gated: `--features ai`) — find visually similar assets from the CLI using stored embeddings. Syntax: `similar:<asset-id>` (top 20 results) or `similar:<asset-id>:<limit>` (custom limit). Composable with all other search filters, e.g. `maki search "similar:abc12345 rating:3+ tag:landscape"`. Uses the in-memory `EmbeddingIndex` for fast dot-product search. Requires embeddings to have been generated via `maki embed` or `maki import --embed`.
- **Collapsible filter bar** — the browse and stroll pages share an identical filter bar (search input, tag chips, rating stars, color label dots, type/format/volume/collection/person dropdowns, path prefix). Toggle with Shift+F or the "Filters" button. State persisted in localStorage. Auto-opens when filters are active.

### Performance
- **Schema version fast-check** — CLI commands no longer run ~30 migration statements on every invocation. A `schema_version` table tracks the current schema version; commands check it with a single fast query and exit with an error if outdated (`Error: catalog schema is outdated ... Run 'maki migrate' to update.`). Saves ~2 seconds per CLI invocation on migrated catalogs. Only `maki init` and `maki migrate` modify the schema.

### Bug Fixes
- **MicrosoftPhoto:Rating normalization** — XMP parser matched both `xmp:Rating` (0–5) and `MicrosoftPhoto:Rating` (percentage scale 0–100) as "Rating" after stripping namespace prefix. Percentage values (20/40/60/80/100) are now converted to 1–5 scale. `maki migrate` fixes existing SQLite and YAML sidecar data automatically.
- **Rating display clamp** — star rendering in JS (stroll satellite navigation) and API responses now clamped to max 5, preventing display corruption from out-of-range values.

### Enhancements
- **Shared filter bar partials** — extracted `filter_bar.html` and `filter_bar_js.html` as reusable Askama template includes, eliminating ~400 lines of duplicated filter UI code between browse and stroll pages. Both pages define an `onFilterChange()` callback; browse triggers htmx form submit, stroll rebuilds the similarity query.
- **`maki migrate` rating repair** — migration now fixes YAML sidecar files with out-of-range rating values (MicrosoftPhoto:Rating percentages) alongside the SQLite fix. Reports count of fixed sidecars.
- **`maki migrate` output** — now prints the schema version number: `Schema migrations applied successfully (schema version N).` JSON output includes `schema_version` and `fixed_ratings` fields.

## v2.2.2 (2026-03-07)

### New Features
- **`maki migrate` command** — explicit CLI command for running database schema migrations. Migrations now run once at program startup for all commands (not per-connection), making this command useful for manual migration or scripting.
- **`maki import --embed`** — generate SigLIP image embeddings for visual similarity search during import (requires `--features ai`). Runs as a post-import phase using preview images. Can be enabled permanently via `[import] embeddings = true` in `maki.toml`. Silently skips if the AI model is not downloaded.

### Performance
- **SQLite performance pragmas** — all database connections now use WAL journal mode, 256 MB mmap, 20 MB cache, `synchronous=NORMAL`, and in-memory temp store. Significant improvement for read-heavy web UI workloads.
- **Single DB connection per detail page request** — asset detail page went from 3 separate SQLite connections to 1, eliminating redundant connection overhead.
- **Combined search query** — browse page now uses `COUNT(*) OVER()` window function to get row count and results in a single query instead of two separate queries.
- **Migrations removed from hot path** — `Catalog::open()` no longer runs schema migrations. Migrations run once at program startup via `Catalog::open_and_migrate()`. Per-request connections in the web server skip migration checks entirely.
- **Dropdown cache warming at server startup** — tag, format, volume, collection, and people dropdown data is pre-loaded when `maki serve` starts, so the first browse page load is as fast as subsequent ones.

## v2.2.1 (2026-03-06)

### New Features
- **`maki faces export`** — exports faces and people from SQLite to YAML files (`faces.yaml`, `people.yaml`) and ArcFace face embeddings to binary files (`embeddings/arcface/<prefix>/<face_id>.bin`). One-time migration command to populate the new file-based persistence layer from existing SQLite data.
- **`maki embed --export`** — exports SigLIP image similarity embeddings from SQLite to binary files (`embeddings/<model>/<prefix>/<asset_id>.bin`). One-time migration for existing embedding data.

### Enhancements
- **Dual persistence for faces, people, and embeddings** — all face/people/embedding write paths (CLI and web UI) now persist data to both SQLite and YAML/binary files. Face records are stored in `faces.yaml`, people in `people.yaml`, ArcFace embeddings as binary files under `embeddings/arcface/`, and SigLIP embeddings under `embeddings/<model>/`. This mirrors the existing pattern used by collections and stacks.
- **`rebuild-catalog` restores AI data** — `rebuild-catalog` now drops and restores the `faces`, `people`, and `embeddings` SQLite tables from YAML and binary files, ensuring no AI data is lost during catalog rebuilds.
- **`maki delete` cleans up AI files** — deleting assets now removes associated ArcFace and SigLIP binary files and updates `faces.yaml`/`people.yaml`.

## v2.2.0 (2026-03-05)

### New Features
- **Face detection** (feature-gated: `--features ai`) — `maki faces detect [--query <Q>] [--asset <id>] [--volume <label>] [--apply]` detects faces in images using YuNet ONNX model. Stores face bounding boxes, confidence scores, and 512-dim ArcFace embeddings. Generates 150×150 JPEG crop thumbnails in `faces/` directory. Reports faces found per asset. Supports `--json`, `--log`, `--time`.
- **Face auto-clustering** — `maki faces cluster [--query <Q>] [--asset <id>] [--volume <label>] [--threshold <F>] [--apply]` groups similar face embeddings into unnamed person groups using greedy single-linkage clustering. Default threshold 0.5 (configurable via `[ai] face_cluster_threshold`). Without `--apply` shows dry-run cluster sizes. Scope filters (`--query`, `--asset`, `--volume`) limit which faces are clustered.
- **People management CLI** — `maki faces people [--json]` lists all people with face counts. `maki faces name <ID> <NAME>` names a person. `maki faces merge <TARGET> <SOURCE>` merges two people. `maki faces delete-person <ID>` deletes a person. `maki faces unassign <FACE_ID>` removes a face from its person.
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
- Fix `maki faces detect --asset` finding zero results (use direct asset ID resolution)

## v2.1.2 (2026-03-05)

### New Features
- **`maki embed` command** (feature-gated: `--features ai`) — batch-generate image embeddings for visual similarity search without tagging. `maki embed [--query <Q>] [--asset <id>] [--volume <label>] [--model <id>] [--force]`. Requires at least one scope filter. `--force` regenerates even if an embedding already exists. Reports embedded/skipped/error counts. Supports `--json`, `--log`, `--time`.

### Enhancements
- **In-memory embedding index** — similarity search (`maki auto-tag --similar`, web UI "Find similar") now uses a contiguous in-memory float buffer (`EmbeddingIndex`) instead of per-query SQLite blob scanning. The index is loaded lazily on first query and cached for the server lifetime. At 100k assets, search drops from seconds to <10ms. Top-K selection uses a min-heap instead of full sort.
- **Opportunistic embedding storage** — the web UI "Suggest tags" and batch "Auto-tag" endpoints now store image embeddings as a side effect, building up the similarity search index without requiring a separate `maki embed` step.
- **Deferred model loading in similarity search** — `find_similar_inner` no longer acquires the AI model lock when the query embedding already exists in the store, avoiding unnecessary contention and startup latency on repeat searches.

## v2.1.1 (2026-03-04)

### New Features
- **Multi-model support for AI auto-tagging** — the system now supports multiple SigLIP model variants. A new `--model` flag on `maki auto-tag` selects the model (default: `siglip-vit-b16-256`). Available models: SigLIP ViT-B/16-256 (768-dim, ~207 MB) and SigLIP ViT-L/16-256 (1024-dim, ~670 MB). `--list-models` shows all known models with download status, size, and active indicator. Embeddings are stored per-model (composite PK) so switching models doesn't corrupt existing data. Configurable via `[ai] model` in `maki.toml`.
- **AI tag suggestions show already-applied tags** — the web UI "Suggest tags" panel now shows all matching tags, including ones already on the asset. Already-applied tags appear dimmed with an "already applied" label and cannot be re-added. "Accept all" renamed to "Accept new" and only applies tags not yet on the asset.

### Enhancements
- **Merged preview regeneration button** — the asset detail page now has a single "Regenerate previews" button that regenerates both the regular preview and the smart preview in one operation, with cache-busted URLs so the browser shows the new images without requiring a page reload.
- **Scope guard for auto-tag** — `maki auto-tag` now requires at least one scope filter (`--query`, `--asset`, `--volume`, or `--similar`) to prevent accidental full-catalog processing.

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
- **AI auto-tagging** — `maki auto-tag [--query <QUERY>] [--asset <id>] [--volume <label>] [--threshold 0.25] [--labels <file>] [--apply]` uses SigLIP ViT-B/16-256 (via ONNX Runtime) for zero-shot image classification against a configurable tag vocabulary (~100 default photography categories). Report-only by default; `--apply` writes suggested tags to assets. Feature-gated behind `--features ai` so non-AI users pay zero binary/dependency cost. Model files (~207 MB quantized) downloaded from HuggingFace on first use via `--download`. Model management: `--list-models`, `--remove-model`. Visual similarity search: `--similar <asset-id>` finds the 20 most visually similar assets using stored 768-dim embeddings. Configurable via `[ai]` section in `maki.toml` (threshold, labels file, model directory, prompt template). Supports `--json`, `--log`, `--time`.

### New Modules (ai feature)
- `src/ai.rs` — SigLIP model wrapper: ONNX session management, image preprocessing (256×256 squash resize, normalize to [-1,1]), SentencePiece tokenization (pad to 64), sigmoid scoring (`logit_scale * dot + logit_bias`), ~100 default photography labels.
- `src/model_manager.rs` — Download and cache management for SigLIP ONNX model files from HuggingFace (Xenova/siglip-base-patch16-256).
- `src/embedding_store.rs` — SQLite-backed 768-dim float vector storage with brute-force cosine similarity search.

### Testing
- Added 41 unit tests for AI modules (preprocessing, tokenization, normalization, cosine similarity, embedding store, model manager) and 13 integration tests covering auto-tag dry run, apply, JSON output, custom labels, threshold, similarity search, and non-image skipping.

## v1.8.9 (2026-03-02)

### New Features
- **Export command** — `maki export <QUERY> <TARGET> [--layout flat|mirror] [--symlink] [--all-variants] [--include-sidecars] [--dry-run] [--overwrite]` copies files matching a search query to a target directory. Default exports the best variant per asset in flat layout (filename collisions resolved by appending an 8-character hash suffix). `--layout mirror` preserves source directory structure (multi-volume assets get a volume-label prefix). `--symlink` creates symlinks instead of copies. `--all-variants` exports every variant instead of just the best. `--include-sidecars` also copies recipe files (.xmp, .cos, etc.). `--dry-run` reports the plan without writing. `--overwrite` re-copies even if the target already has a matching hash. Files are integrity-verified via SHA-256 after copy. Supports `--json`, `--log`, `--time`.

### Testing
- Added 5 unit tests for flat-mode filename collision resolution and 12 integration tests covering all export modes (flat, mirror, dry-run, skip existing, overwrite, sidecars, symlink, all-variants, best-variant-only, filename collision, JSON output, no results).

## v1.8.8 (2026-03-02)

### Enhancements
- **Multi-select format filter** — the browse page format filter is now a grouped multi-select dropdown panel instead of a single-select dropdown. Formats are organized by category (RAW, Image, Video, Audio, Other) with group-level "All RAW"/"All Image" toggle checkboxes. Each format shows its variant count. Multiple formats can be selected simultaneously (e.g., all RAW formats, or NEF + TIFF). Trigger button shows compact text: single format name, group name when a full group is selected, or "nef +3..." for mixed selections. Sends comma-separated values to the existing OR filter backend.

## v1.8.7 (2026-03-02)

### New Features
- **Delete command** — `maki delete <ASSET_IDS...> [--apply] [--remove-files]` removes assets from the catalog. Default is report-only mode (shows what would be deleted). `--apply` executes deletion (asset rows, variants, file locations, recipes, previews, sidecar YAML, collection memberships, stack membership). `--remove-files` (requires `--apply`) also deletes physical files from disk. Supports stdin piping (`maki search -q "orphan:true" | maki delete --apply`), asset ID prefix matching, `--json`, `--log`, `--time`.

## v1.8.6 (2026-03-02)

### New Features
- **Incremental verify** — `maki verify --max-age <DAYS>` skips files verified within the given number of days, enabling fast periodic checks on large catalogs. `--force` overrides the skip and re-verifies everything. Configurable default via `[verify] max_age_days` in `maki.toml`.
- **Search negation and OR operators** — prefix any filter or free-text term with `-` to exclude matches (`-tag:rejected`, `-sunset`). Use commas within a filter value for OR logic (`tag:alice,bob`, `format:nef,cr3`, `label:Red,Orange`). Combinable: `type:image,video -format:xmp`.

### Enhancements
- **Recipe verified_at persistence** — verify now persists `verified_at` timestamps to sidecar YAML for both variant locations and recipe locations, so incremental verify works correctly across catalog rebuilds.
- **Show command recipe details** — `maki show` now displays variant hash and volume:path for each recipe, matching the detail level shown for variant locations.
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
- **Configurable page size** — the number of results per page in the browse grid is now configurable via `[serve] per_page` in `maki.toml` (default: 60). Also available as `maki serve --per-page N` CLI flag.
- **Page-turn keyboard shortcuts** — Shift+Left/Right arrow keys navigate to the previous/next page in the browse grid and lightbox. In the lightbox, regular arrow keys at page boundaries automatically trigger cross-page navigation with a loading spinner overlay.

### Enhancements
- **Batch operation performance** — batch tag, rating, and label operations now share a single catalog connection, device registry, and content store across all assets instead of opening fresh instances per asset. Batch tagging 30+ assets is now ~10× faster.
- **Batch toolbar feedback** — the batch toolbar shows "Processing N assets..." with a pulsing animation while operations are in progress, instead of silently disabling buttons.
- **Lightbox cross-page loading indicator** — when navigating across a page boundary in the lightbox, a spinner overlay appears and further arrow key presses are blocked until the new page loads.
- **Detail page nav loading indicator** — small spinners appear next to the Prev/Next buttons while adjacent page IDs are being fetched at page boundaries.
- **Preserve selection after batch operations** — batch tag, rating, and label operations no longer clear the selection, allowing multiple operations on the same set of assets.
- **Preview cache freshness** — preview and smart preview HTTP responses now include `Cache-Control: no-cache`, ensuring browsers revalidate after rotation or regeneration instead of serving stale cached images. Combined with `Last-Modified` headers, unchanged previews still get fast 304 responses.
- **Batch operation timing logs** — when `maki serve --log` is enabled, batch operations log timing to stderr (e.g. `batch_tag: 30 assets in 1.2s (30 ok, 0 err)`).

## v1.8.2 (2026-03-01)

### New Features
- **Editable asset date** — set or clear an asset's creation date via CLI (`maki edit --date 2024-12-25` / `--clear-date`) or the web UI (inline date editor on the asset detail page, `PUT /api/asset/{id}/date` endpoint). Updates both sidecar YAML and SQLite catalog.
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
- **Unified browse/lightbox/detail navigation** — clicking the lightbox image opens the detail page; clicking the detail page image opens the lightbox. All three views form a seamless navigation loop with focus tracked via `maki-browse-focus` in sessionStorage. Lightbox open, navigate, and close sync the focused card. Arrow key navigation in lightbox and detail updates which card will be focused on return to browse.
- **Browse state preservation on back-navigation** — scroll position, batch selection, and keyboard focus are now preserved when navigating back from the detail or compare page. Selection is persisted to sessionStorage (`maki-browse-selection`) on `pagehide` and restored on fresh page loads. On bfcache return, the DOM is preserved as-is (no more htmx refresh that was destroying state). Focus is restored from sessionStorage with `scrollIntoView` to approximate scroll position.
- **Compare page Escape fix** — added `preventDefault()` to the Escape key handler on the compare page, fixing unreliable back-navigation that required double-pressing Escape.
- **Cursor feedback** — lightbox and detail page preview images now show `cursor: pointer` to indicate they are clickable navigation targets.

## v1.7.0 (2026-02-28)

### New Features
- **Smart previews** — a second preview tier at 2560px (configurable) for high-resolution offline browsing. Smart previews are stored alongside regular thumbnails in `smart_previews/<hash-prefix>/<hash>.jpg` and enable zoom and pan in the web UI even when the original media volume is offline.
  - **Import `--smart` flag**: `maki import --smart <PATHS...>` generates smart previews alongside regular thumbnails during import. Can also be enabled permanently via `[import] smart_previews = true` in `maki.toml`.
  - **On-demand generation**: Set `[preview] generate_on_demand = true` in `maki.toml` to have the web server generate smart previews automatically when first requested. The first load takes a few seconds (pulsing HD badge shown); subsequent loads are instant.
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
- **Import `--add-tag` flag** — `maki import --add-tag landscape --add-tag 2026 <PATHS...>` adds tags to every imported asset. Repeatable. Merged with `[import] auto_tags` from config and XMP tags.
- **Asset folder link** — the asset detail page shows clickable links to the folder containing each variant file.

### Bug Fixes
- **generate-previews PATHS mode** — fix fallback to hash-based variant lookup when the file is not on the expected volume, preventing "variant not found" errors for files with valid catalog entries on other volumes.

## v1.6.3 (2026-02-27)

### Enhancements
- **Recipe cleanup during dedup** — when dedup removes a duplicate file location, co-located recipe files (XMP sidecars etc.) in the same directory are automatically cleaned up from disk, catalog, and sidecar YAML. Applies to both `maki dedup --apply` and the web UI's per-location "Remove" and "Auto-resolve" actions. Recipe counts shown in dry-run output and web UI confirm dialog.
- **Dedup prefer config default** — new `[dedup]` section in `maki.toml` with a `prefer` field. Sets a default path substring for the `--prefer` flag in both CLI and web UI. The web UI duplicates page pre-populates a "Prefer keeping" input from config. CLI `--prefer` overrides the config value.
- **Dedup prefer uses substring matching** — the `--prefer` flag now matches anywhere in the relative path (substring) rather than requiring the path to start with the prefix. This correctly handles nested directories like `Session/Selects/photo.nef` when prefer is set to `Selects`.
- **CLI filter flags for duplicates and dedup** — `maki duplicates` gains `--filter-format` and `--path` flags matching the web UI's filter controls. `maki dedup` gains `--filter-format` and `--path` flags to scope dedup operations by file format or path prefix. The `--volume` flag on `duplicates` now uses proper SQL filtering instead of post-filtering.

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
  - **CLI**: `maki stack create/add/remove/pick/dissolve/list/show` (alias `st`). Full `--json` support. Stacks persist in `stacks.yaml` and survive `rebuild-catalog`.
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
- **CLI `--favorite` flag** — `maki ss save --favorite "Name" "query"` marks a saved search as favorite. `maki ss list` shows `[*]` marker next to favorites.
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
- **`maki dedup`** — remove same-volume duplicate file locations. Identifies variants with 2+ copies on the same volume, keeps the "best" copy (by `--prefer` path prefix, verification recency, path length), and removes the rest. `--min-copies N` ensures at least N total copies survive across all volumes. Report-only by default; `--apply` to delete files and remove location records. Supports `--volume`, `--json`, `--log`, `--time`.
- **`maki backup-status`** — check backup coverage and find under-backed-up assets. Shows aggregate overview (totals, coverage by volume purpose, location distribution, volume gaps, at-risk count). `--at-risk` lists under-backed-up assets using the same output formats as `maki search`. `--min-copies N` sets the threshold (default: 2). `--volume <label>` shows which assets are missing from a specific volume. Optional positional query scopes the analysis to matching assets. Supports `--format`, `-q`, `--json`, `--time`.

## v1.4.0 (2026-02-24)

### New Features
- **Volume purpose** — volumes can now be assigned a logical purpose (`working`, `archive`, `backup`, `cloud`) describing their role in the storage hierarchy. `maki volume add --purpose <purpose>` sets purpose at registration, `maki volume set-purpose <volume> <purpose>` changes it later. Purpose is shown in `maki volume list` and included in `--json` output. This metadata lays the groundwork for smart duplicate analysis and backup coverage reporting (see storage workflow proposal).
- **Enhanced `maki duplicates`** — three new flags for targeted duplicate analysis:
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
- **`maki fix-recipes`** — re-attach recipe files (`.xmp`, `.cos`, etc.) that were misclassified as standalone assets during import. Scans the catalog for assets whose only variant is a recipe-type file, finds the correct parent variant by matching filename stem and directory, and re-attaches them. Dry-run by default (`--apply` to execute).

### Enhancements
- **15 additional RAW format extensions** — added support for `.3fr`, `.cap`, `.dcr`, `.eip`, `.fff`, `.iiq`, `.k25`, `.kdc`, `.mdc`, `.mef`, `.mos`, `.mrw`, `.obm`, `.ptx`, `.rwz` camera formats
- **`import --auto-group`** — after normal import, runs auto-grouping scoped to the neighborhood of imported files (one directory level up from each imported file). Avoids catalog-wide false positives from restarting camera counters. Combines with `--dry-run` and `--json`.

## v1.3.1 (2026-02-24)

### New Features
- **`maki fix-dates` command** — scan assets and correct `created_at` dates from variant EXIF metadata and file modification times. Fixes assets imported with wrong dates (import timestamp instead of capture date). Re-extracts EXIF from files on disk for assets imported before `date_taken` was stored in metadata. Backfills `date_taken` into variant source_metadata on apply so future runs work without the volume online. Reports offline volumes clearly with skip counts and mount instructions. Dry-run by default (`--apply` to execute). Supports `--volume`, `--asset`, `--json`, `--log`, `--time`.

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
- **`maki serve --log`** — the global `--log` flag now enables request logging on the web server, printing `METHOD /path -> STATUS (duration)` to stderr for each HTTP request.

## v1.1.1 (2026-02-23)

### Enhancements
- **`path:` filter normalization** — the `path:` search filter now accepts filesystem paths in the CLI: `~` expands to `$HOME`, `./` and `../` resolve relative to the current working directory, and absolute paths matching a registered volume's mount point are automatically stripped to volume-relative with the volume filter implicitly applied. Plain relative paths (no `./` prefix) remain volume-relative prefix matches as before.

## v1.1.0 (2026-02-23)

### New Features
- **Export-based preview selection** — previews now prefer Export > Processed > Original variants for display. RAW+JPEG assets show the processed JPEG preview instead of the flat dcraw rendering. Affects `maki show`, web UI asset detail page, and `generate-previews` catalog mode.
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
- **`maki fix-roles` command** — scan multi-variant assets and re-role non-RAW variants from Original to Export when a RAW variant exists. Fixes assets imported before the auto-grouping role fix. Dry-run by default (`--apply` to execute). Supports `--volume`, `--asset`, `--json`, `--log`, `--time`.
- **Import auto-grouping role fix** — newly imported RAW+non-RAW pairs now correctly assign Export role to non-RAW variants (previously both were marked Original)

## v0.7.0 (2026-02-23)

### New Features
- **`maki auto-group` command** — automatically group assets by filename stem across directories, solving the problem where CaptureOne exports land in different directories than their RAW originals. Uses fuzzy prefix + separator matching (e.g., `Z91_8561.ARW` matches `Z91_8561-1-HighRes-(c)_2025_Thomas Herrmann.tif`). Chain resolution ensures multiple export levels all group to the shortest root stem. RAW files are preferred as the group target; donors are re-roled from Original to Export. Dry-run by default (`--apply` to execute). Supports `--json`, `--log`, `--time`.
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
- **Saved searches** (smart albums) — `maki saved-search` (alias `ss`) with save, list, run, delete subcommands; stored in `searches.toml`; web UI chips on browse page with rename/delete on hover
- **Collections** (static albums) — `maki collection` (alias `col`) with create, list, show, add, remove, delete subcommands; SQLite-backed with YAML persistence; search filter `collection:<name>`; web UI batch toolbar integration
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
- **`maki refresh` command** — re-read metadata from changed sidecar/recipe files without full re-import; supports `--dry-run`, `--json`, `--log`, `--time`

## v0.4.4 (2026-02-21)

### New Features
- **Color labels** — first-class 7-color label support (Red, Orange, Yellow, Green, Blue, Pink, Purple); XMP `xmp:Label` extraction, CLI editing (`maki edit --label`), web UI color dot picker, browse filtering, batch operations, XMP write-back
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
- **`maki update-location` command** — update file path in catalog after manual moves on disk

## v0.3.4 (2026-02-20)

### New Features
- **Extended `maki cleanup`** — now removes orphaned assets (all variants have zero locations) and orphaned preview files, in addition to stale location records
- **Search location health filters** — `orphan:true`, `missing:true`, `stale:N`, `volume:none`

## v0.3.3 (2026-02-20)

### New Features
- **`maki cleanup` command** — remove stale file location records for files no longer on disk

## v0.3.2 (2026-02-20)

### New Features
- **`maki sync` command** — reconcile catalog with disk after external file moves, renames, or modifications

## v0.3.1 (2026-02-20)

### New Features
- **`maki edit` command** — set or clear asset name, description, and rating from CLI
- **Photo workflow integration proposal** — documented gaps and planned features for CaptureOne integration

## v0.3.0 (2026-02-20)

### New Features
- **Version display** in web UI navigation bar

## v0.2.0 (2026-02-19)

### New Features
- **Web UI** (`maki serve`) — browse/search page with filter dropdowns, asset detail page, tag editing, rating support
- **First-class rating** — `Option<u8>` field on Asset with CLI search, web UI stars, XMP extraction
- **Stats page** in web UI with bar charts and tag cloud
- **Tags page** in web UI
- **Multi-tag chip input** with autocomplete on browse page
- **Metadata search** with indexed columns and extended filter syntax (camera, lens, ISO, focal, aperture, dimensions)
- **Info card previews** for non-visual formats (audio, documents) and as fallback for missing external tools
- **`maki.toml` configuration** — preview settings, serve settings, import exclude/auto_tags
- **`--log` flag** on `generate-previews` for per-file progress

### Bug Fixes
- Fix multi-component ASCII EXIF fields (Fuji lens_model parsing)

## v0.1.0 (2026-02-18)

### New Features
- **`maki init`** — initialize catalog with SQLite schema, volume registry, config
- **`maki volume add/list`** — register and list storage volumes with online/offline detection
- **`maki import`** — SHA-256 hashing, EXIF extraction, stem-based auto-grouping, recipe handling, duplicate location tracking, preview generation
- **`maki search`** — text, type, tag, format filters
- **`maki show`** — full asset details with variants, locations, metadata
- **`maki tag`** — add/remove tags
- **`maki group`** — manually merge variant assets
- **`maki duplicates`** — find files with identical content across locations
- **`maki generate-previews`** — thumbnails for images, RAW (dcraw/LibRaw), video (ffmpeg)
- **`maki rebuild-catalog`** — regenerate SQLite from YAML sidecars
- **`maki relocate`** — copy/move assets between volumes with integrity verification
- **`maki verify`** — re-hash files to detect corruption or bit rot
- **Output formatting** — `--json`, `--format` templates, `-q` quiet mode, `-t` elapsed time
- **XMP metadata extraction** — keywords, rating, description, color label, creator, rights
