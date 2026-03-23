# MAKI Roadmap

Living document tracking planned enhancements. Previous proposals (all implemented or deferred) are in `archive/`. Active proposals are in `doc/proposals/`.

Current version: **v4.0.12** (2026-03-23)

---

## Active Proposals

### Similarity Stacking (Phase 3)

Auto-discover visual clusters across the catalog and propose stacks. See `doc/proposals/similarity-browse-and-grouping.md`.

**Status:** Phase 1 (similarity browse) and Phase 2 (stack-by-similarity from detail page) implemented in v4.0.2. Phase 3 (catalog-wide auto-stack) pending.

**Scope:**
- `maki auto-stack --threshold 85` — scan all embedded assets, cluster by similarity, propose stacks
- Pick selection: highest-rated in each cluster
- `--dry-run` for review, `--apply` to create
- Clustering algorithm: greedy connected-components over embedding similarity matrix

**Complexity:** Medium. Embedding infrastructure and stacking exist; needs clustering algorithm and CLI command.

---

## Tier 1 — High Value

### Watch Mode

Auto-import and sync on filesystem changes. After a CaptureOne session, new files appear in the catalog without manual `maki import`.

**Scope:**
- `maki watch [PATHS...] [--volume <label>]` — monitors directories for new/changed files
- Poll-based initially (simple, cross-platform); fsevents/inotify optional later
- Triggers import for new files, refresh for changed recipes
- Configurable via `[watch]` section in `maki.toml` (poll interval, exclude patterns)
- Runs as foreground process (like `maki serve`), logs activity to stderr

**Complexity:** Medium. Core import/refresh logic exists; needs a polling loop and file-change detection.

### GPU-Accelerated Embeddings (Linux/Windows)

SigLIP embedding generation on CPU is slow for large catalogs. GPU backends make batch embedding practical at scale.

**Status:** CoreML (macOS) implemented in v2.4.1 via `--features ai-gpu`. Linux/Windows pending.

**Open:**
- CUDA execution provider for Linux (requires `ort/cuda` feature, CUDA Toolkit + cuDNN)
- DirectML execution provider for Windows (requires `ort/directml` feature)
- Testing on Linux and Windows platforms
- Batch processing with GPU-optimal batch sizes

**Complexity:** Low for adding providers (code pattern exists), high for testing/packaging across platforms.

### IPTC/EXIF Write-Back

Write metadata changes back into JPEG/TIFF files directly, not just XMP sidecars. Some workflows and stock photo submissions require embedded metadata.

**Scope:**
- `maki writeback --embed` writes rating, tags, description, label into file's embedded metadata
- IPTC keywords, caption/description, urgency (mapped from rating)
- Preserves existing embedded metadata; only updates DAM-managed fields
- Re-hashes file after write, updates variant content hash

**Complexity:** High. Modifying binary file metadata without corruption requires careful handling.

---

### Web UI Export Progress

The ZIP export modal shows "Preparing..." with no progress feedback.

**Scope:**
- Server sends export plan summary (file count, estimated size) before starting
- Client shows a progress bar or asset counter
- Warn before very large downloads (> 1 GB)

**Complexity:** Low-Medium.

---

## Tier 2 — Workflow Convenience

### Import Profiles

Named preset configurations for different import scenarios (studio shoot, travel, phone backup).

**Scope:**
- `[import.profiles.<name>]` sections in `maki.toml`
- `maki import --profile studio <PATHS...>` selects a profile
- Profiles inherit from `[import]` defaults, override specific fields

**Complexity:** Low.

### Multi-User Web Access

Allow browsing the catalog from other devices on the LAN.

**Scope:**
- `--read-only` mode: disables all write endpoints
- Optional basic auth (`[serve] username/password` in `maki.toml`)
- Responsive CSS improvements for mobile viewports

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
- **v4.0.1**: Default browse filter (`[browse] default_filter`), VLM tags mode fix, organizing/culling manual chapter
- **v4.0.2**: Similarity browse (scores on cards, `min_sim:` filter, sort by similarity), stack-by-similarity from detail page, stack management toolbar (add/remove/set-pick), filter bar two-row layout
- **v4.0.3**: Windows support (cross-platform path normalization, `\\?\` prefix handling, 8MB stack, tool detection), GitHub Actions CI (macOS/Linux/Windows × standard/AI), missing tool warnings, README branding
- **v4.0.4**: Tag quote fix (`"Sir" Oliver Mally`), doc tests (11), tag matching tests for special characters, updated branding assets
- **v4.0.5**: Unified `NumericFilter` enum (all numeric filters support x, x+, x-y, x,y syntax), `orphan:false` filter, rating ranges, complete search filter documentation consistency, `*` not a wildcard fix
- **v4.0.6**: Large TIFF preview/embedding fix (`no_limits()`), `--query` → positional in docs/error messages, filter availability table corrected
- **v4.0.7**: `--smart` preview fix (generates both), CLI doc audit (8 discrepancies), overview chapter restructured (Core Concepts), PDF quality (zero Unicode warnings, fallback fonts, diagram improvements)
- **v4.0.8**: `maki init` creates `smart_previews/`, `assets/` → `metadata/` doc fix, smart preview documentation throughout manual, layout improvements (compact diagrams, centered scaling, table row spacing, module dependency graph), Windows VLM setup
- **v4.0.9**: Cheat sheet (2-page landscape A4), group metadata merge (highest rating, first label/description), consistent MAKI/maki naming (~81 fixes), product overview illustration, DAM → MAKI in all remaining references
- **v4.0.10**: XMP writeback safeguard (`[writeback] enabled = false` by default), documented across manual with warnings and recommended settings
- **v4.0.11**: Automated binary release workflow (6 binaries: macOS ARM, Linux x86_64, Windows x86_64 × standard/AI), ubuntu-24.04 for Linux AI (glibc 2.38+)
- **v4.0.12**: 13 branded screenshots (7 new views: lightbox, stroll, map, calendar, analytics, similarity browse, compare), repo renamed to `thoherr/maki`
