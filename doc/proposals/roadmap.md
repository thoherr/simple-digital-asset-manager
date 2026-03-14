# DAM Roadmap

Living document tracking planned enhancements. Previous proposals (all implemented or deferred) are in `archive/`.

Current version: **v3.2.0** (2026-03-14)

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
- `dam writeback --embed` writes rating, tags, description, label into file's embedded metadata
- IPTC keywords, caption/description, urgency (mapped from rating)
- EXIF UserComment for description (optional)
- Preserves existing embedded metadata; only updates DAM-managed fields
- Re-hashes file after write, updates variant content hash

**Complexity:** High. Modifying binary file metadata without corruption requires careful handling. Could use `img_parts` or `little_cms` crates.

---

### Web UI Export Progress

The ZIP export modal shows "Preparing..." with no progress feedback. For large exports this can take minutes with no indication of progress.

**Scope:**
- Server sends export plan summary (file count, estimated size) before starting the ZIP build
- Client shows a progress bar or asset counter during ZIP creation
- Options: SSE stream, polling endpoint, or initial size estimate + indeterminate progress bar
- Warn user before very large downloads (e.g. "> 1 GB, continue?")

**Complexity:** Low-Medium. Backend plan info already available via `build_export_plan()`; needs a two-phase request or SSE channel.

### Web UI Delete

The batch toolbar covers most operations but `dam delete` isn't exposed in the web UI. With export now available, a natural workflow is export-then-delete.

**Scope:**
- "Delete" button in batch toolbar (with confirmation modal showing asset count and warning)
- `DELETE /api/batch/delete` endpoint — calls `AssetService::delete()` for each asset
- Option to delete only from current volume vs. all copies
- Refresh grid after deletion

**Complexity:** Low. `AssetService::delete()` exists; needs a route, JS handler, and confirmation dialog.

### Shell `export` Built-in

The interactive shell (`dam shell`) doesn't expose the `export` command. Now that `build_export_plan()` is extracted, adding `export $picks /tmp/out` would complete the shell's coverage.

**Scope:**
- `export <query-or-ids> <target-dir> [--layout flat|mirror] [--all-variants] [--include-sidecars] [--dry-run]`
- Reuses `AssetService::export()` directly
- Supports shell variables (`export $picks ~/Desktop/out`)

**Complexity:** Low. All export logic exists; just needs a shell command entry and argument parsing.

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
- **v2.4**: Contact sheet export, split command, alternate variant role, grouped CLI help, CoreML GPU acceleration, VLM image descriptions
- **v2.5**: Text-to-image semantic search, auto-describe during import, concurrent VLM, analytics dashboard, batch relocate, drag-and-drop, per-stack expand/collapse, audit filters (variants/scattered), metadata reimport
- **v3.0**: Asset management shell — interactive REPL with named variables, tab completion, session defaults, script files, source command, `-c` one-liner mode
- **v3.1**: Preview command, consistent positional query and shell variable expansion for all multi-asset commands
- **v3.2**: Web UI export as ZIP download (selected assets and filtered results), dark mode modal fixes
