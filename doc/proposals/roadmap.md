# DAM Roadmap

Living document tracking planned enhancements. Previous proposals (all implemented or deferred) are in `archive/`.

Current version: **v2.4.1** (2026-03-09)

---

## Tier 1 — High Value

### Watch Mode

Auto-import and sync on filesystem changes. After a CaptureOne session, new files appear in the catalog without manual `dam import`.

**Scope:**
- `dam watch [PATHS...] [--volume <label>]` — monitors directories for new/changed files
- Poll-based initially (simple, cross-platform); fsevents/inotify optional later
- Triggers import for new files, refresh for changed recipes
- Configurable via `[watch]` section in `dam.toml` (poll interval, exclude patterns)
- Runs as foreground process (like `dam serve`), logs activity to stderr

**Complexity:** Medium. Core import/refresh logic exists; needs a polling loop and file-change detection.

### GPU-Accelerated Embeddings

SigLIP embedding generation on CPU is slow for large catalogs. GPU backends make batch embedding practical at scale.

**Status:** CoreML (macOS) implemented in v2.4.1 via `--features ai-gpu`. Linux/Windows pending.

**Done:**
- `--features ai-gpu` enables CoreML execution provider on macOS (Neural Engine on Apple Silicon, Metal on Intel)
- `[ai] execution_provider` config option ("auto", "cpu", "coreml")
- Shared `build_onnx_session()` helper used by SigLIP and face detection/recognition
- Automatic fallback to CPU when provider unavailable

**Open:**
- CUDA execution provider for Linux (requires `ort/cuda` feature, CUDA Toolkit + cuDNN)
- DirectML execution provider for Windows (requires `ort/directml` feature)
- Testing on Linux and Windows platforms
- Batch processing with GPU-optimal batch sizes

**Complexity:** Low for adding providers (code pattern exists), high for testing/packaging across platforms.

### IPTC/EXIF Write-Back

Write metadata changes back into JPEG/TIFF files directly, not just XMP sidecars. Some workflows and stock photo submissions require embedded metadata.

**Scope:**
- `dam writeback --embed` writes rating, tags, description, label into file's embedded metadata
- IPTC keywords, caption/description, urgency (mapped from rating)
- EXIF UserComment for description (optional)
- Preserves existing embedded metadata; only updates DAM-managed fields
- Re-hashes file after write, updates variant content hash

**Complexity:** High. Modifying binary file metadata without corruption requires careful handling. Could use `img_parts` or `little_cms` crates.

---

## Tier 2 — Workflow Convenience

### Import Profiles

Named preset configurations for different import scenarios (studio shoot, travel, phone backup).

**Scope:**
- `[import.profiles.<name>]` sections in `dam.toml`
- Each profile: auto_tags, exclude patterns, target volume, smart_previews, embeddings
- `dam import --profile studio <PATHS...>` selects a profile
- Profiles inherit from `[import]` defaults, override specific fields
- `dam import --list-profiles` shows available profiles

**Complexity:** Low. Config parsing and merge logic only.

### Multi-User Web Access

Allow browsing the catalog from other devices on the LAN (iPad, phone, second computer).

**Scope:**
- `dam serve --bind 0.0.0.0` already works for LAN access
- Add `--read-only` mode: disables all write endpoints (rating, tags, labels, delete, etc.)
- Optional: basic auth (`[serve] username/password` in `dam.toml`)
- Responsive CSS improvements for mobile viewports (browse grid already adapts)

**Complexity:** Low-Medium. Most infrastructure exists; needs endpoint guards and auth middleware.

### Volume Health Monitoring

Surface drive health and verification staleness proactively.

**Scope:**
- Extend `dam stats --verified` with per-volume staleness warnings
- `dam verify --report` generates a health summary (oldest unverified, % coverage, estimated time)
- Optional: track volume mount/unmount history for "last seen" timestamps
- Web UI: volume health indicators on backup page

**Complexity:** Low. Builds on existing verify timestamps and stats infrastructure.

### Batch Relocate

Move entire query results to a target volume in one command.

**Scope:**
- `dam relocate --query <QUERY> --target <VOLUME> [--remove-source] [--dry-run]`
- Or piped: `dam search -q "date:2024 volume:Working" | dam relocate --target "Archive 2024"`
- Progress reporting with `--log`
- Dry-run with size estimate

**Complexity:** Low. `relocate` per-asset logic exists; needs a query-driven loop.

---

## Tier 3 — Polish & Future

### Undo / Edit History

Track metadata changes with timestamps for audit trail and undo capability.

**Scope:**
- `asset_history` table: asset_id, field, old_value, new_value, timestamp, source (cli/web/sync)
- `dam history <asset-id>` shows change log
- `dam undo <asset-id>` reverts last change (or last N)
- Web UI: history panel on detail page

**Complexity:** High. Requires change tracking in every write path (edit, tag, rating, label, import, refresh, sync-metadata).

### Drag-and-Drop in Web UI

Reorder stacks, add to collections, and manage groups via drag-and-drop in the browser.

**Scope:**
- Drag browse cards onto collection sidebar to add
- Drag within stack view to reorder (set pick)
- HTML5 drag-and-drop API with touch fallback

**Complexity:** Medium. Frontend-heavy; backend endpoints already exist.

### Ollama VLM Integration

Natural language image descriptions via local vision-language models.

**Scope:**
- `dam describe [--query <Q>] [--model <ollama-model>] [--apply]`
- Sends preview image + prompt to local Ollama server
- Stores generated description as asset description (or separate field)
- Batch processing with rate limiting
- Could also power semantic search (embed descriptions for text-to-image retrieval)

**Complexity:** Medium. HTTP client to Ollama API; main question is practical quality vs. tag-based search.

### Statistics Dashboard

Shooting analytics beyond the current `dam stats` command.

**Scope:**
- Shooting frequency heatmap (already have calendar view — could add stats overlay)
- Camera/lens usage breakdown over time
- Rating distribution trends
- Storage growth projections
- Web UI: `/analytics` page with charts

**Complexity:** Medium. Aggregate queries exist in stats; needs charting (could use lightweight JS library).

---

## Completed (Archived)

All previous proposals are in `doc/proposals/archive/`. Key milestones:

- **v0.1–v1.0**: Core CLI — import, search, metadata, volumes, previews
- **v1.1–v1.4**: Storage workflow — dedup, backup-status, copies filter, volume purpose
- **v1.5–v1.8**: Web UI — lightbox, dark mode, calendar, map, compare, facets, stacks, collections
- **v1.8.9**: Export command
- **v2.0–v2.1**: AI — auto-tag, embeddings, similarity search, suggest tags
- **v2.2**: Performance — SQLite pragmas, single connection, denormalized columns
- **v2.3**: Stroll, sync-metadata, comprehensive cleanup, faces/people
- **v2.4**: Contact sheet export, split command, alternate variant role, grouped CLI help, CoreML GPU acceleration
