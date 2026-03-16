# Implemented Features — v2.5.x

Features completed during the v2.5 release cycle and moved from the active roadmap.

---

## Batch Relocate (v2.5.1)

Move entire query results to a target volume in one command.

**Done:**
- `maki relocate --query <QUERY> --target <VOLUME> [--remove-source] [--dry-run]`
- Stdin piping: `maki search -q "date:2024 volume:Working" | maki relocate --target "Archive 2024"`
- Multiple positional IDs with `--target`
- Backward compatible with single-asset `maki relocate <ID> <VOL>`
- Progress reporting with `--log`, batch summary with `--json`

---

## Drag-and-Drop in Web UI (v2.5.1)

Reorder stacks, add to collections, and manage groups via drag-and-drop in the browser.

**Done:**
- Drag browse cards onto collection dropdown to add to collection
- Drag stack members on detail page to reorder (drop to first = set pick)
- HTML5 drag-and-drop API with visual feedback (drop highlights, toast notifications)

---

## Ollama VLM Integration (v2.4.2–v2.5.3)

Natural language image descriptions via local vision-language models. Phases 1–4 complete (v2.4.2: CLI + web UI; v2.5.0: auto-describe + text search; v2.5.3: concurrent requests). See [proposal](proposal-vlm-integration.md).

**Done:**
- `maki describe` command with `--mode describe|tags|both`, `--apply`, `--force`, `--dry-run`
- OpenAI-compatible API with Ollama native fallback
- Configurable temperature, timeout, model, endpoint, prompt via `[vlm]` in `maki.toml`
- "Describe" button on detail page, batch "Describe" in toolbar
- VLM startup health check, `vlm_enabled` template flag
- Truncated JSON recovery, tag deduplication
- `text:` semantic search filter — natural language image search via SigLIP text encoder
- `maki import --describe` — auto-describe during import via VLM post-import phase
- Concurrent VLM requests via `[vlm] concurrency` setting (v2.5.3)

---

## Statistics Dashboard (v2.5.1)

Shooting analytics beyond the current `maki stats` command.

**Done:**
- `/analytics` page with shooting frequency, camera/lens usage, rating distribution, format breakdown, monthly import volume, and storage per volume charts
- Auto-scaling bar charts and sparkline rendering
- Nav bar link under Maintain group
