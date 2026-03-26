# MAKI Roadmap

Living document tracking planned enhancements. Previous proposals (all implemented or deferred) are in `archive/`. Active proposals are in `doc/proposals/`.

Current version: **v4.1.3** (2026-03-25)

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

### Import Profiles

Named preset configurations for different import scenarios (studio shoot, travel, phone backup).

**Scope:**
- `[import.profiles.<name>]` sections in `maki.toml`
- `maki import --profile studio <PATHS...>` selects a profile
- Profiles inherit from `[import]` defaults, override specific fields

**Complexity:** Low.

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
- **v4.1.x (Video Playback)**: HTML5 video player on detail page and lightbox, duration badges on browse cards, video metadata extraction via ffprobe (duration, codec, resolution, framerate) at import time, `generate-previews --force` backfills video metadata for existing assets. Phase 2 `duration:` search filter implemented (uses denormalized `video_duration` column, full `NumericFilter` support). Remaining Phase 2 filters (`codec:`, `resolution:`) pending.
- **v4.1.x**: MAKI Pro edition (`pro` feature flag, `-pro` release artifacts), VLM/writeback/sync-metadata gated behind Pro, search filter reference card, `volume split`/`rename`, `edit --clear-tags`, improved `scattered:` filter with `/N` depth, star rating filter UX, consistent *(Pro)* markers in manual, repo cleanup (`doc/images/`, `doc/quickref/`)
