# Proposal: Future Enhancements

Longer-term feature ideas for DAM, carried forward from the completed [Photo Workflow Integration proposal](proposal-photo-workflow-integration.md).

---

## 1. Watch Mode

```
dam watch [PATHS...] [--volume <label>]
```

File system watcher (via `notify` crate) that auto-imports/syncs when files change. Useful for monitoring a CaptureOne session's output folder during an active editing session.

**Use cases:**
- Leave `dam watch /Volumes/PhotosDrive/Sessions/2026-02-23/Capture/` running while shooting tethered — new RAW files are imported automatically
- Monitor an export folder — processed TIFFs/JPEGs are picked up and grouped with their RAW originals
- Detect recipe modifications (XMP/COS) and refresh metadata in real time

**Design considerations:**
- Should debounce events (files are often written in stages)
- Needs to handle volume mount/unmount gracefully
- Could optionally trigger preview generation on new imports
- Consider whether to run as foreground process or background daemon

---

## 2. Export Command

> **Status (v1.8.9):** Fully implemented. See `dam export --help` for usage.

```
dam export <query> <target> [--layout flat|mirror] [--symlink] [--all-variants] [--include-sidecars] [--dry-run] [--overwrite]
```

Export matching assets to a directory, optionally with sidecars. Useful for preparing files for delivery or for feeding into another tool.

**Use cases:**
- `dam export "rating:5 tag:portfolio" /tmp/delivery/` — gather best-of selections for client delivery
- `dam export "collection:Print" /Volumes/USB/ --include-sidecars` — export with XMP/COS sidecars for handoff to another workstation
- `dam export "tag:instagram" ~/Export/` — flat directory (no subdirectories) for social media upload

**Implemented features:** Copy or symlink, flat (hash-suffix collision resolution) or mirror (preserves directory structure with volume-label prefix), best variant or all variants, sidecar inclusion, dry-run, overwrite, SHA-256 integrity verification, `--json`/`--log`/`--time`.
