# MAKI Roadmap

Living document tracking planned enhancements. Previous proposals (all implemented or deferred) are in `archive/`. Active proposals are in `doc/proposals/`.

Current version: **v4.4.6** (2026-04-20)

---

## Active Proposals

### Manual Translation (i18n)

Produce the MAKI user manual in English and German from a single source using inline language markers. See `doc/proposals/manual-i18n.md`.

**Status:** Proposal written, not started.

**Complexity:** Low (tooling), Medium (translation effort).

---


## Tier 1 — High Value

### Auto-Stack by Similarity (Catalog-wide) *(Pro)*

Discover natural visual clusters across the catalog and propose stacks. Phase 3 of the similarity browse proposal (Phases 1–2 implemented in v4.0.2). See `archive/proposal-similarity-browse-and-grouping.md`.

**Scope:**
- `maki auto-stack --threshold 85` — scan all embedded assets, cluster by similarity, propose stacks
- Pick selection: highest-rated in each cluster
- `--dry-run` for review, `--apply` to create
- Clustering algorithm: greedy connected-components over embedding similarity matrix

**Complexity:** Medium. Embedding infrastructure and stacking exist; needs clustering algorithm and CLI command.

### Watch Mode

Auto-import and sync on filesystem changes. After a CaptureOne session, new files appear in the catalog without manual `maki import`.

**Scope:**
- `maki watch [PATHS...] [--volume <label>]` — monitors directories for new/changed files
- Poll-based initially (simple, cross-platform); fsevents/inotify optional later
- Triggers import for new files, refresh for changed recipes
- Configurable via `[watch]` section in `maki.toml` (poll interval, exclude patterns)
- Runs as foreground process (like `maki serve`), logs activity to stderr

**Complexity:** Medium. Core import/refresh logic exists; needs a polling loop and file-change detection.

### GPU-Accelerated Embeddings (Linux/Windows) *(Pro)*

SigLIP embedding generation on CPU is slow for large catalogs. GPU backends make batch embedding practical at scale.

**Status:** CoreML (macOS) included automatically in Pro builds since v4.1.0. Linux/Windows pending.

**Open:**
- CUDA execution provider for Linux (requires `ort/cuda` feature, CUDA Toolkit + cuDNN)
- DirectML execution provider for Windows (requires `ort/directml` feature)
- Testing and packaging across platforms

**Complexity:** Low for adding providers (code pattern exists), high for testing/packaging.

### IPTC/EXIF Write-Back *(Pro)*

Write metadata changes back into JPEG/TIFF files directly, not just XMP sidecars. Some workflows and stock photo submissions require embedded metadata.

**Scope:**
- `maki writeback --embed` writes rating, tags, description, label into file's embedded metadata
- IPTC keywords, caption/description, urgency (mapped from rating)
- Preserves existing embedded metadata; only updates DAM-managed fields
- Re-hashes file after write, updates variant content hash

**Complexity:** High. Modifying binary file metadata without corruption requires careful handling.

---

## Tier 2 — Workflow Convenience

### Mobile & Tablet Browsing

The web UI works on mobile but isn't optimized. Combined with read-only multi-user access, this enables "show photos to clients on iPad" and remote browsing from any device on the LAN.

**Scope:**
- Responsive layout improvements: touch-friendly grid, swipe navigation in lightbox
- Collapsible filter bar for small screens
- `--read-only` mode (disables all write endpoints) for safe sharing
- Optional basic auth (`[serve] username/password` in `maki.toml`)

**Complexity:** Medium. CSS/layout work plus read-only mode (which is mostly route-level guards).

### Advanced Contact Sheet Templates *(Pro)*

Professional-grade contact sheet layouts beyond the current defaults. Templates for client proofing, portfolio review, and print production.

**Scope:**
- Additional layout presets (grid with metadata overlay, filmstrip, portfolio pages)
- Custom template system (user-defined layouts via config)
- Gated behind `pro` feature flag

**Complexity:** Medium.

### Web UI Export Progress

The ZIP export modal shows "Preparing..." with no progress feedback.

**Scope:**
- Server sends export plan summary (file count, estimated size) before starting
- Client shows a progress bar or asset counter
- Warn before very large downloads (> 1 GB)

**Complexity:** Low-Medium.

### Volume Health Monitoring

Surface drive health and verification staleness proactively.

**Scope:**
- Per-volume staleness warnings in `maki stats --verified`
- `maki verify --report` health summary
- Web UI volume health indicators on backup page

**Complexity:** Low.

---

## Tier 3 — Polish & Future

### Undo / Edit History

Track metadata changes with timestamps for audit trail and undo capability.

**Scope:**
- `asset_history` table: asset_id, field, old_value, new_value, timestamp, source
- `maki history <asset-id>` and `maki undo <asset-id>`
- Web UI history panel on detail page

**Complexity:** High.

### Tethered Shooting

Live import during a connected camera session. Essentially watch mode + auto-import + immediate preview in the web UI.

**Scope:**
- Build on watch mode (Tier 1) with lower latency
- Auto-open imported assets in the web UI
- CaptureOne hot folder integration as primary use case

**Complexity:** Medium (requires watch mode first).

### Print Workflow

Print selected assets with proper page layout. Currently only contact sheets are supported.

**Scope:**
- `maki print` command or web UI print button
- Single-image and multi-image layouts with configurable margins
- ICC color profile support for accurate print colors

**Complexity:** High. Color management is complex; layout engine already exists in contact sheet code.

---

## Completed (Archived)

Design documents for completed features are in `doc/proposals/archive/`. Key milestones:

- **v0.1–v1.0**: Core CLI — import, search, metadata, volumes, previews
- **v1.1–v1.4**: Storage workflow — dedup, backup-status, copies filter, volume purpose
- **v1.5–v1.8**: Web UI — lightbox, dark mode, calendar, map, compare, facets, stacks, collections
- **v1.8.9**: Export command
- **v2.0–v2.1**: AI — auto-tag, embeddings, similarity search, suggest tags
- **v2.2**: Performance — SQLite pragmas, single connection, denormalized columns
- **v2.3**: Stroll, sync-metadata, comprehensive cleanup, faces/people
- **v2.4**: Contact sheet export, split command, alternate variant role, CoreML GPU, VLM descriptions
- **v2.5**: Text-to-image search, auto-describe during import, concurrent VLM, analytics, batch relocate, drag-and-drop, per-stack expand/collapse
- **v3.0**: Interactive shell — REPL with variables, tab completion, script files
- **v3.1**: Preview command, consistent positional query and shell variable expansion
- **v3.2**: Web UI export ZIP, batch delete, shell export, per-model VLM config, verbose threading, documentation consolidation
- **v4.0**: MAKI rebrand (binary `dam` → `maki`, config `dam.toml` → `maki.toml`, full visual rebrand), branded PDF manual
- **v4.0.1–v4.0.12**: Default browse filter, similarity browse, Windows support, CI/CD, unified numeric filters, XMP writeback safeguard, cheat sheet, automated releases, branded screenshots
- **v4.1.x (Video Playback)**: HTML5 video player on detail page and lightbox, duration badges on browse cards, video metadata extraction via ffprobe (duration, codec, resolution, framerate) at import time, `generate-previews --force` backfills video metadata for existing assets. Phase 2 `duration:` and `codec:` search filters implemented (duration uses denormalized `video_duration` column with full `NumericFilter` support; codec uses denormalized `video_codec` column with LIKE matching). Remaining Phase 2 filter: `resolution:` (but `width:`/`height:` already cover this).
- **v4.1.x**: MAKI Pro edition (`pro` feature flag, `-pro` release artifacts), VLM/writeback/sync-metadata gated behind Pro, search filter reference card, `volume split`/`rename`, `edit --clear-tags`, improved `scattered:` filter with `/N` depth, star rating filter UX, consistent *(Pro)* markers in manual, repo cleanup (`doc/images/`, `doc/quickref/`)
- **v4.2.x**: Card-first workflow (media volume purpose, create-sidecars, auto-label, volume list filters), import profiles, video proxy generation with hover-to-play, user guide improvements (17 items across 3 passes)
- **v4.3.x**: Tag hierarchy separator aligned with LR/C1 (`|` and `>`), tag rename/clear/expand-ancestors/export-vocabulary subcommands, ancestor expansion matching CaptureOne convention, vocabulary.yaml for planned tag hierarchy with autocomplete integration, git-based catalog backup, BTreeMap for deterministic YAML, Tagging Guide chapter, configurable `[group] session_root_pattern` regex
- **v4.3.12**: Volumes page in web UI with register/rename/purpose/remove and import dialog with live SSE progress, `*` wildcards in `path:` filter (full pattern matching with leading-`*` opt-in for slow scans), SigLIP 2 multilingual model variants (`siglip2-base-256-multi`, `siglip2-large-256-multi`) for `text:` search in German, French, Spanish, Italian, Japanese, Chinese, etc. — see `archive/proposal-multilingual-text-search.md`
- **v4.3.13**: License compliance infrastructure — `cargo-about` generates `THIRD_PARTY_LICENSES.md` shipped in every release archive, `cargo-deny` validates allowed-license allowlist on every CI run, new `maki licenses` CLI command, new manual appendix `reference/11-licenses.md` covering MAKI license / Rust crates / AI models / external tools. Dropped `viuer` dependency (last LGPL-3.0 transitive dep) → fully permissive dependency tree. Simplified `maki preview` to always open in OS default viewer (removed inline terminal display and `--open` flag). Doc fix: clarified `maki embed` does NOT need `--force` when switching AI models — embeddings are keyed per `(asset_id, model_id)`, so a model switch only generates the missing embeddings. New "Switching models" section in setup guide.
- **v4.3.14–v4.3.19**: Tag rename in web UI (pencil icon + autocomplete modal), `scattered:`/`copies:` semantics fixes (distinct session roots / distinct volumes), new default subject vocabulary branches (style / condition / mood), code-quality pass (route handler deduplication, `unwrap()` → `expect()`, lowercase error messages, section markers in the 3 largest source files).
- **v4.3.20**: `label:none` search filter and browse UI icon (matching `rating:0` / `volume:none` patterns), stronger active-state styling on ∅ filter icons, `tag rename =` now uses leaf-only semantics (consistent with search `=`), hierarchical tag search matches at any level (`tag:Altstätten` finds `location|Switzerland|Altstätten`), quoting hint on empty search results, asset-ID whitespace trimming (handles copy-paste NBSP), browse page person filter preserved across pagination/sort, unnamed face clusters browseable from people page and asset detail, `stack from-tag --remove-tags` cleans up orphan tags too (not just newly-stacked), quickref cheat sheet trimmed to 2 pages.
- **v4.4.0**: Face recognition pipeline rewrite — ArcFace ResNet-100 **FP32** model (replacing INT8) with proper **5-point landmark alignment** to the canonical 112×112 template, **corrected preprocessing** (raw pixel values; the model's internal Sub/Mul nodes do the normalization — external normalization was double-normalizing and collapsing embeddings), **agglomerative hierarchical clustering** (average linkage / UPGMA, order-independent) replacing greedy single-linkage, **model version tracking** via new `recognition_model` column on faces (schema v5→v6, faces from an older model variant are skipped by clustering with a warning), new defaults (`face_cluster_threshold` 0.5→0.35, `face_min_confidence` 0.5→0.7), three new diagnostic/maintenance commands: `maki faces clean` (delete unassigned orphans), `maki faces similarity` (histogram + percentile stats for picking a threshold), `maki faces dump-aligned` (save the 112×112 aligned crops for visual verification), `--min-confidence` flag on `cluster`, `--force` on `detect`. Full upgrade path documented in the user guide and CHANGELOG.
- **v4.4.1**: People workflow UX — batch merge UI on the `/people` page (checkbox-select any N clusters, pick the merge target with a bullseye badge, confirmation modal with thumbnails), automatic **merge suggestions** panel that surfaces pairs of clusters whose centroid embeddings are similar enough to likely be the same person (common after auto-clustering), client-side name filter on the people grid, searchable typeahead combobox replacing the plain `<select>` in the face-assign dropdown on the asset detail page. New endpoint `GET /api/people/merge-suggestions`; `POST /api/people/{id}/merge` also accepts a plural `source_ids` array for batch merges.
- **v4.4.2**: Filter bar refinements — the browse filter bar's person picker is now a chip-based multi-select on the same row as tag chips (layout: `[tags] [people] [path]`). **Multiple tag chips and multiple person chips now AND** (asset must contain all selected), fixing a bug where chip selection was silently OR'd at the catalog level. Comma-OR (`tag:a,b`, `person:a,b`) still works in the q field for power users. Person chips are tinted teal to distinguish from salmon tag chips; same interaction grammar (type, ↑/↓/Enter, backspace to remove last chip).
- **v4.4.3**: `face_scan_status` column on assets (schema v6→v7) so `faces detect` no longer re-scans zero-face assets on every run. Persisted to both SQLite and YAML sidecar (maintaining the dual-storage invariant). Dual-storage audit fixed `faces.recognition_model` round-trip through faces.yaml and added `post_migration_sync` hook. New "Visual Discovery" user-guide chapter (12) covering face recognition, similarity search, and stroll as workflows.
- **v4.4.4**: Tag search marker swap — `tag:=` is now **whole-path match** (exact tag value equality, reads naturally as "equals"), `tag:/` is leaf-only-at-any-level. Breaking change vs v4.4.3 where the assignments were reversed. Tri-state tag chip toggle cycles ▼ → = → /. Disambiguates same-named tags at different hierarchy levels (the "Legoland" problem — root-level `Legoland` vs `location|Denmark|Legoland` vs `location|Germany|Legoland`).
- **v4.4.5**: Maintenance release. Internal refactoring of the four largest files (`main.rs`, `web/routes.rs`, `catalog.rs`, `asset_service.rs`) — `run_faces_command` lifted out of `run_command` (P1a), `build_search_where` split into named helpers (P1b), `Volume::online_map()` and `resolve_collection_ids()` extracted to deduplicate 20 call sites (P2), new `src/cli_output.rs` module consolidating three drifted copies of `format_size` + the `item_status(id, verb, elapsed)` pattern (P3b), and `web/routes.rs` split from 6,599 LOC into 13 domain submodules (348 LOC in `mod.rs`) (P3c). Tagging guide gains the "Thinking in facets" framework, worked examples for events and color, and the opt-in `event` / `color` top-level facets synced into the built-in default vocabulary. New `doc/qa-report.md` with the codebase analysis that drove the priorities.
- **v4.4.6**: New `maki tag split OLD NEW1 [NEW2...] [--keep]` command for one-to-many tag restructuring (e.g. `subject|event|wedding-jane-2025` → `event|wedding-jane-2025` + `subject|event|wedding` in one pass; or with `--keep`, additive / copy semantics). Matching split UI on the web tags page — a second button on leaf rows only (non-leaf rows get an alignment placeholder), opens a modal with dynamic target-input list, `Keep source` checkbox, Preview/Apply flow identical to rename. New "Tagging Quick Guide" A3 landscape poster at `doc/quickref/tagging.pdf` (printable wall reference with facet cards, decision helper, worked example, and tag-command cheat). Tagging-guide chapter gets a capstone illustration (SVG from maki-marketing rendered to PNG) showing one photo with nine tags from seven facets.
