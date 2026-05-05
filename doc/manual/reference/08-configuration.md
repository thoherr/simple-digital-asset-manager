# Configuration Reference (maki.toml)

The `maki.toml` file stores catalog-level configuration. It lives at the root of your catalog directory (the same directory that contains the `metadata/`, `previews/`, and `catalog.db` files).

---

## File Location

`maki.toml` is created automatically by `maki init`. maki locates it by searching the current directory and walking up through parent directories until it finds a directory containing `maki.toml`.

```
my-catalog/
  maki.toml              <-- configuration file
  catalog.db
  metadata/
  previews/
  searches.toml
  collections.yaml
```

All sections and fields are optional. A missing file or an empty file is equivalent to all-defaults. A comment-only file is also valid:

```toml
# maki catalog configuration
```

---

## Top-Level Fields

### default_volume

- **Type:** UUID string (optional)
- **Default:** none

Fallback volume for `maki import` when auto-detection from the file path is ambiguous or fails. When set, import uses this volume if it cannot determine the correct volume from the first path argument.

```toml
default_volume = "550e8400-e29b-41d4-a716-446655440000"
```

Find your volume UUIDs with `maki volume list --json`.

---

## [preview] Section

Controls how preview thumbnails are generated during import and by `maki generate-previews`.

### max_edge

- **Type:** unsigned integer
- **Default:** `800`
- **Validation:** must be greater than 0

Maximum pixel size of the longest edge for generated thumbnails. The shorter edge is scaled proportionally. Applies to all preview types (standard images, RAW conversions, video frames, and info cards).

### format

- **Type:** string (`"jpeg"` or `"webp"`)
- **Default:** `"jpeg"`

Output format for preview files. JPEG produces lossy compressed thumbnails controlled by the `quality` setting. WebP produces lossless thumbnails via the `image` crate (the `quality` setting is ignored for WebP).

### quality

- **Type:** unsigned integer (1--100)
- **Default:** `85`
- **Validation:** must be between 1 and 100

JPEG compression quality. Higher values produce larger files with better visual quality. Only applies when `format = "jpeg"`. Ignored for WebP.

### smart_max_edge

- **Type:** unsigned integer
- **Default:** `2560`
- **Validation:** must be greater than 0

Maximum pixel size of the longest edge for smart previews. Smart previews are a higher-resolution preview tier (separate from regular thumbnails) that enable zoom and pan in the web UI.

### smart_quality

- **Type:** unsigned integer (1--100)
- **Default:** `85`

JPEG compression quality for smart previews. Only applies when the preview format is JPEG.

### generate_on_demand

- **Type:** boolean
- **Default:** `false`

When `true`, the web server generates smart previews automatically on first request. The first load takes a few seconds while the preview is generated; subsequent requests are served from disk. A pulsing "HD" badge in the lightbox and detail page provides visual feedback during generation.

When `false`, smart previews must be generated explicitly via `maki import --smart`, the "Regenerate previews" button on the asset detail page, or a future batch command.

### Notes

Changing `max_edge` or `format` affects only newly generated previews. Existing previews are not automatically regenerated. Use `maki generate-previews --force` to regenerate all previews with the new settings.

Smart previews are stored in a separate directory (`smart_previews/`) and do not replace regular thumbnails.

```toml
[preview]
max_edge = 1200
format = "jpeg"
quality = 90
smart_max_edge = 2560
smart_quality = 85
generate_on_demand = true
```

---

## [serve] Section

Controls the built-in web UI server started by `maki serve`.

### port

- **Type:** unsigned 16-bit integer
- **Default:** `8080`

TCP port for the web server.

### bind

- **Type:** string (IP address)
- **Default:** `"127.0.0.1"`

Bind address for the web server. Use `"127.0.0.1"` to restrict access to the local machine. Use `"0.0.0.0"` to allow connections from other devices on the network.

### per_page

- **Type:** unsigned 32-bit integer
- **Default:** `60`

Number of results per page in the browse grid.

### stroll_neighbors

- **Type:** unsigned 32-bit integer
- **Default:** `12`

Initial number of neighbor thumbnails shown around the center image on the stroll page. Controls the default position of the neighbor count slider.

### stroll_neighbors_max

- **Type:** unsigned 32-bit integer
- **Default:** `25`

Maximum value for the neighbor count slider on the stroll page.

### stroll_discover_pool

- **Type:** unsigned 32-bit integer
- **Default:** `80`

Candidate pool size for Discover mode on the stroll page. In Discover mode, the server fetches this many nearest neighbors and then randomly samples from the pool to produce the displayed satellites. Larger values increase variety; smaller values keep results closer to the center image.

### stroll_fanout

- **Type:** unsigned 32-bit integer
- **Default:** `5`

Initial fan-out count for L2 transitive neighbors on the stroll page. Controls how many second-level neighbors fan out from a focused satellite. Set to `0` to disable fan-out by default.

### stroll_fanout_max

- **Type:** unsigned 32-bit integer
- **Default:** `10`

Maximum value for the fan-out slider on the stroll page.

### CLI Override

The `--port`, `--bind`, and `--per-page` flags on `maki serve` override the values from `maki.toml`:

```bash
maki serve --port 9090 --bind 0.0.0.0 --per-page 100
```

```toml
[serve]
port = 8080
bind = "127.0.0.1"
per_page = 100
stroll_neighbors = 12
stroll_neighbors_max = 25
stroll_fanout = 5
stroll_fanout_max = 10
stroll_discover_pool = 80
```

---

## [import] Section

Controls import behavior for `maki import`.

### exclude

- **Type:** array of glob pattern strings
- **Default:** `[]` (empty, nothing excluded)

Glob patterns matched against filenames (not full paths). Files matching any pattern are skipped during import. Useful for excluding OS-generated files, temporary files, and other non-media clutter.

Pattern matching uses the `glob-match` crate with standard glob syntax: `*` matches any sequence of characters, `?` matches a single character.

```toml
[import]
exclude = [
    "Thumbs.db",
    ".DS_Store",
    "*.tmp",
    "*.bak",
    "desktop.ini",
]
```

### auto_tags

- **Type:** array of tag strings
- **Default:** `[]` (empty, no auto-tags)

Tags automatically applied to every newly imported asset. These are merged with any tags extracted from XMP metadata, CLI `--add-tag` values, and deduplicated (no duplicate tags are created).

Useful for marking import batches or applying a default workflow status:

```toml
[import]
auto_tags = ["inbox", "unreviewed"]
```

### smart_previews

- **Type:** boolean
- **Default:** `false`

When `true`, import automatically generates smart previews (high-resolution, 2560px) alongside regular thumbnails. Equivalent to passing `--smart` on every `maki import` command. Smart preview dimensions are controlled by `[preview] smart_max_edge`.

```toml
[import]
smart_previews = true
```

### embeddings

*(Pro)*

- **Type:** boolean
- **Default:** `false`

When `true`, import automatically generates SigLIP image embeddings for visual similarity search alongside previews. Equivalent to passing `--embed` on every `maki import` command. Embeddings enable `maki auto-tag --similar` and the web UI "Find similar" button.

Uses the model configured in `[ai] model`. Silently skips if the model is not downloaded. Non-image assets are skipped.

```toml
[import]
embeddings = true
```

### descriptions

- **Type:** boolean
- **Default:** `false`

When `true`, import automatically generates VLM descriptions for newly imported assets as a post-import phase. Equivalent to passing `--describe` on every `maki import` command.

Uses the VLM configured in `[vlm]` (endpoint, model, prompt, mode, temperature). Silently skips if the VLM endpoint is not available. Assets that already have descriptions are skipped. Works with all `[vlm] mode` settings (describe, tags, both).

```toml
[import]
descriptions = true
```

### profiles

- **Type:** table of named profiles
- **Default:** (none)

Named preset configurations for different import scenarios. Each profile is a sub-table under `[import.profiles]` that overrides the base `[import]` config. Unset fields inherit from the base. CLI flags override both the profile and base config.

Profile fields (all optional):

| Field | Type | Description |
|-------|------|-------------|
| `exclude` | string array | Override exclude patterns |
| `auto_tags` | string array | Override auto-tags (replaces base, not merged) |
| `smart_previews` | boolean | Override smart preview generation |
| `embeddings` | boolean | Override embedding generation |
| `descriptions` | boolean | Override VLM description generation |
| `include` | string array | File type groups to include (e.g. `captureone`) |
| `skip` | string array | File type groups to skip (e.g. `audio`) |

```toml
[import]
exclude = [".DS_Store", "Thumbs.db"]
auto_tags = ["inbox"]

[import.profiles.card]
auto_tags = ["from-card"]
smart_previews = true

[import.profiles.studio]
auto_tags = ["studio"]
smart_previews = true
embeddings = true
descriptions = true
include = ["captureone"]
skip = ["audio"]
```

Usage: `maki import --profile card /Volumes/CARD/DCIM`

---

## [dedup] Section

Controls dedup behavior for `maki dedup` and the web UI's auto-resolve action.

### prefer

- **Type:** string (optional)
- **Default:** none

Default path substring for the `--prefer` flag. When set, dedup prefers keeping file locations whose relative path contains this string. Useful for always keeping files in a curated directory (e.g. `Selects`) while removing copies elsewhere.

The CLI `--prefer` flag overrides this value. The web UI duplicates page pre-populates its "Prefer keeping" input from this setting.

```toml
[dedup]
prefer = "Selects"
```

---

## [verify] Section

Controls incremental verify behavior for `maki verify`.

### max_age_days

- **Type:** integer (optional)
- **Default:** none

Default value for the `--max-age` flag. When set, `maki verify` skips files verified within the given number of days. The CLI `--max-age` flag overrides this value. `--force` overrides both.

```toml
[verify]
max_age_days = 30
```

---

## [group] Section

Controls how `maki auto-group` identifies session boundaries (shoot/event roots) when partitioning assets into directory neighborhoods.

### session_root_pattern

- **Type:** string (regex)
- **Default:** `^\d{4}-\d{2}`

A regular expression matched against each directory component in a variant's path. The deepest (rightmost) matching component becomes the session root — all files below that directory are considered part of the same session and eligible for stem matching.

The default pattern matches directory names starting with `YYYY-MM` (e.g., `2024-10`, `2024-10-05-wedding`, `2025-05-09-event`), which works for the common `year/year-month/year-month-day-event/` hierarchy.

Set to an empty string to disable session root detection entirely; auto-group then falls back to parent-directory grouping (each immediate parent directory is its own group scope).

```toml
[group]
# Default: date-prefixed directories
session_root_pattern = '^\d{4}-\d{2}'

# Match directories starting with "shoot-" or "project-"
# session_root_pattern = '^(shoot|project)-'

# Disable session root detection (parent-directory fallback)
# session_root_pattern = ''
```

---

## [contact_sheet] Section

Default settings for `maki contact-sheet`. All fields are optional; CLI flags override these values.

| Key | Type | Default | Description |
|---|---|---|---|
| `layout` | string | `"standard"` | Layout preset: `dense`, `standard`, `large` |
| `paper` | string | `"a4"` | Paper size: `a4`, `letter`, `a3` |
| `fields` | string | `"filename,date,rating"` | Comma-separated metadata fields |
| `margin` | float | `15.0` | Page margin in mm |
| `quality` | integer | `92` | JPEG quality for page images (1--100) |
| `label_style` | string | `"border"` | Color label display: `border`, `dot`, `none` |
| `copyright` | string | `""` | Copyright text for page footer |

```toml
[contact_sheet]
layout = "dense"
paper = "a3"
label_style = "dot"
copyright = "© 2026 Thomas Herrmann"
```

---

## [ai] Section *(Pro)* {#ai-section}

Controls AI auto-tagging behavior for `maki auto-tag`.

### model

- **Type:** string
- **Default:** `"siglip-vit-b16-256"`

Which SigLIP model to use. Available models:

| Model ID | Size | Embedding dim | Notes |
|----------|------|---------------|-------|
| `siglip-vit-b16-256` | ~207 MB | 768 | English-only, good balance (default) |
| `siglip-vit-l16-256` | ~670 MB | 1024 | English-only, higher accuracy |
| `siglip2-base-256-multi` | ~410 MB | 768 | **Multilingual** (German, French, Spanish, Italian, Japanese, Chinese, etc.) |
| `siglip2-large-256-multi` | ~920 MB | 1024 | **Multilingual**, higher accuracy, slower |

The CLI `--model` flag overrides this value. Embeddings are stored per `(asset_id, model_id)`, so switching models doesn't corrupt existing data — the old model's embeddings stay intact. After switching, run `maki embed ''` (without `--force`) to generate embeddings for the new model: it processes only assets that don't yet have an embedding for the active model, and is restart-safe. See [Switching models](../user-guide/02-setup.md#switching-models) in the setup guide for the full workflow.

**Multilingual model**: `siglip2-base-256-multi` is Google's SigLIP 2 base, trained on the WebLI dataset across many languages. Use this if you want to type `text:` queries in German or any non-English language. It uses the Gemma SentencePiece tokenizer (vocab 256k) and is a drop-in replacement for `siglip-vit-b16-256` in dimensions and image resolution.

### threshold

- **Type:** float (0.0--1.0)
- **Default:** `0.1`
- **Validation:** must be between 0.0 and 1.0

Minimum confidence score for a tag to be suggested. Higher values produce fewer but more confident suggestions. Lower values produce more suggestions with more noise.

### labels

- **Type:** string (file path, optional)
- **Default:** none (uses built-in ~100 photography categories)

Path to a custom labels file (one label per line). When set, overrides the built-in default labels. The CLI `--labels` flag overrides this value.

### model_dir

- **Type:** string (directory path)
- **Default:** `"~/.maki/models"`

Where to cache downloaded model files. The `~` prefix is expanded to the user's home directory.

### prompt

- **Type:** string
- **Default:** `"a photograph of {}"`

Text encoder prompt template. The `{}` placeholder is replaced with each label name before encoding. Adjusting the prompt can improve classification accuracy for specific use cases (e.g., `"a photo of a {}"` or `"a professional photograph of {}"`).

### execution_provider

- **Type:** string (`"auto"`, `"cpu"`, `"coreml"`)
- **Default:** `"auto"`

> GPU providers require the macOS Pro build (which includes CoreML support). On other platforms, all values fall back to CPU.

Selects the ONNX Runtime execution provider for AI inference (SigLIP, YuNet, ArcFace). `"auto"` uses CoreML when available on macOS (Neural Engine on Apple Silicon, Metal on Intel), falling back to CPU. `"cpu"` forces CPU-only inference. `"coreml"` explicitly requests CoreML (errors if unavailable).

### face_cluster_threshold

- **Type:** float (0.0--1.0)
- **Default:** `0.35`

Minimum average cosine similarity for two face clusters to be merged during agglomerative clustering. Tuned for the aligned FP32 ArcFace pipeline where intra-person similarity typically falls in 0.5–0.9 and inter-person similarity is near zero or slightly negative.

Higher values produce tighter, more conservative clusters (more singletons, less risk of mixing people). Lower values produce bigger clusters at the risk of merging similar-looking different people. Use `maki faces similarity` to inspect your data's actual distribution before picking a value far from the default.

### face_min_confidence

- **Type:** float (0.0--1.0)
- **Default:** `0.7`

Default value for the `--min-confidence` flag on `maki faces cluster` — face detections below this confidence are dropped before clustering (blurry, profile, or partial faces produce noisy embeddings that hurt cluster purity). Can also be passed per-invocation to override.

### text_limit

- **Type:** unsigned integer
- **Default:** `50`

Maximum number of results returned by `text:` semantic search queries. This is the default limit used when no inline limit is specified in the query. Can be overridden per-query with the `text:"query":limit` syntax.

```toml
[ai]
threshold = 0.3
labels = "my-labels.txt"
model_dir = "~/.maki/models"
prompt = "a photograph of {}"
execution_provider = "auto"
face_cluster_threshold = 0.35
face_min_confidence = 0.7
text_limit = 50
```

---

## [vlm] Section *(Pro)* {#vlm-section}

Controls the VLM (vision-language model) integration for `maki describe`.

### endpoint

- **Type:** string (URL)
- **Default:** `"http://localhost:11434"`

Base URL of the VLM server. Any server implementing the OpenAI-compatible `/v1/chat/completions` endpoint works. Ollama also supports its native `/api/generate` endpoint as an automatic fallback.

**Local servers:**
- [Ollama](https://ollama.com) -- `http://localhost:11434` (default)
- [LM Studio](https://lmstudio.ai) -- `http://localhost:1234`
- [llama.cpp server](https://github.com/ggerganov/llama.cpp) -- `http://localhost:8080`

**Cloud APIs** (OpenAI-compatible, require API key in environment):
- OpenAI -- `https://api.openai.com` (model: `gpt-4o`)
- Groq -- `https://api.groq.com/openai` (model: `llama-3.2-90b-vision-preview`)
- Together AI -- `https://api.together.xyz` (model: `meta-llama/Llama-3.2-90B-Vision-Instruct-Turbo`)

> **Note:** Cloud APIs charge per request. maki does not set authentication headers -- if your endpoint requires an API key, you may need to use a local proxy or set the key via the server's own configuration.

### model

- **Type:** string
- **Default:** `"qwen2.5vl:3b"`

Model name passed to the VLM server. For Ollama, this is the model tag (e.g., `moondream`, `qwen2.5vl:3b`, `qwen2.5vl:7b`). For cloud APIs, this is the model identifier (e.g., `gpt-4o`).

**Recommended models for photography** (tested with Ollama on Apple Silicon):

| Model | Size | RAM | Speed (M3 Pro) | Quality |
|-------|------|-----|-----------------|---------|
| Moondream 2B | 1.7 GB | ~2 GB | ~3--5s | Good, fast for batch |
| Qwen2.5-VL 3B | 2.0 GB | ~3 GB | ~8--12s | Very good (default) |
| Qwen3-VL 4B | 2.8 GB | ~4 GB | ~10--15s | Very good |
| Gemma 3 4B | 3.3 GB | ~4 GB | ~10--15s | Very good |
| Qwen3-VL 8B | 5.2 GB | ~6 GB | ~15--20s | Excellent |
| Qwen2.5-VL 7B | 4.7 GB | ~6 GB | ~20--36s | Excellent |

See [VLM Model Guide](10-vlm-models.md) for a comprehensive comparison including Qwen3.5, backend setup instructions, and hardware recommendations.

### max_tokens

- **Type:** unsigned 32-bit integer
- **Default:** `500`

Maximum number of tokens in the VLM response. 500 tokens is typically 3--5 sentences, leaving headroom for models that use internal reasoning tokens.

### prompt

- **Type:** string (optional)
- **Default:** none (uses built-in photography-focused prompt)

Custom system prompt sent to the VLM. When not set, uses a built-in prompt appropriate for the mode (describe or tags). In `--mode both`, custom prompts are ignored because each of the two calls uses its specialized built-in prompt.

Override for specialized workflows:

```toml
[vlm]
prompt = "Describe the architectural style, materials, and notable design features."
```

### mode

- **Type:** string
- **Default:** `"describe"`

Default output mode for `maki describe`. One of: `describe` (natural language description), `tags` (JSON tag suggestions), `both` (two separate VLM calls: one for description, one for tags).

### temperature

- **Type:** float
- **Default:** `0.7`

Sampling temperature controlling randomness in VLM output. `0.0` = deterministic (same input always produces the same output), `0.7` = balanced (default), `1.0+` = more creative/varied. Lower values give more consistent results; higher values give more diverse but potentially less accurate output.

### timeout

- **Type:** unsigned 32-bit integer (seconds)
- **Default:** `300`

Maximum time to wait for a VLM response. Larger models on CPU may need higher timeouts. Assets that time out are reported as errors and skipped.

### concurrency

- **Type:** unsigned 32-bit integer
- **Default:** `1`

Number of concurrent VLM requests. When greater than 1, `maki describe`, `maki import --describe`, and web UI batch describe process multiple assets in parallel. Each batch of `concurrency` assets sends VLM HTTP calls concurrently using scoped threads; preparation (skip checks, image lookup) and result application (catalog writes) remain sequential. Set to the number of simultaneous requests your VLM server can handle efficiently — for local Ollama this depends on available VRAM and model size.

### models

- **Type:** array of strings (optional)
- **Default:** `[]` (empty — only the default model is shown)

List of VLM model names to offer in the web UI model selector dropdown on the asset detail page. When configured with two or more models (including the default `model`), a dropdown appears next to the "Describe" button letting you choose which model to use per request. The first entry is pre-selected. Models not in this list are not offered in the web UI (useful when your Ollama server has many models loaded but only some are suitable for image description).

```toml
[vlm]
model = "moondream"
models = ["moondream", "qwen3-vl:4b"]
```

The CLI `maki describe --model` flag is unaffected by this setting — it accepts any model name regardless of the `models` list.

### num_ctx

- **Type:** unsigned 32-bit integer
- **Default:** `0` (not set — use server default)

Context window size passed to the VLM server. When non-zero, overrides the model's default context length. Useful for models that benefit from a larger context (e.g., `num_ctx = 4096` for Qwen models). A value of `0` means "not set" — the server uses its own default.

### top_p

- **Type:** float
- **Default:** `0.0` (not set — use server default)

Nucleus sampling parameter. When non-zero, only tokens whose cumulative probability exceeds `top_p` are considered. Lower values produce more focused output. A value of `0.0` means "not set" — the server uses its own default.

### top_k

- **Type:** unsigned 32-bit integer
- **Default:** `0` (not set — use server default)

Top-K sampling parameter. When non-zero, limits token selection to the K most probable candidates. A value of `0` means "not set" — the server uses its own default.

### repeat_penalty

- **Type:** float
- **Default:** `0.0` (not set — use server default)

Repetition penalty applied to tokens that have already appeared. Values above `1.0` discourage repetition; `1.0` disables the penalty. A value of `0.0` means "not set" — the server uses its own default.

### Per-Model Configuration

You can override any VLM setting for specific models using `[vlm.model_config."model-name"]` sections. When `maki describe` runs with a given model (via `--model` or `[vlm] model`), any matching per-model section is merged on top of the global `[vlm]` settings. CLI flags always win over both.

This is useful when different models need different timeouts, context sizes, or sampling parameters:

```toml
[vlm]
model = "qwen2.5vl:3b"
timeout = 300

[vlm.model_config."qwen3-vl:4b"]
max_image_edge = 384
timeout = 300
num_ctx = 4096

[vlm.model_config."moondream:latest"]
max_tokens = 200
temperature = 0.1
```

When running `maki describe --model qwen3-vl:4b`, the per-model overrides apply: `timeout` becomes 300, `num_ctx` becomes 4096, and `max_image_edge` becomes 384. All other settings fall through from the global `[vlm]` section.

Per-model sections support all the same fields as the global `[vlm]` section (except `model`, `models`, and `model_config` itself).

### CLI Override

The `--endpoint`, `--model`, `--prompt`, `--max-tokens`, `--timeout`, `--temperature`, `--mode`, `--num-ctx`, `--top-p`, `--top-k`, and `--repeat-penalty` flags on `maki describe` override the values from `maki.toml` (including per-model overrides).

```toml
[vlm]
endpoint = "http://localhost:11434"
model = "qwen2.5vl:3b"
max_tokens = 500
timeout = 300
temperature = 0.7
mode = "describe"
concurrency = 1
num_ctx = 0
top_p = 0.0
top_k = 0
repeat_penalty = 0.0
# models = ["qwen2.5vl:3b", "moondream"]
# prompt = "Custom prompt here."

# Per-model overrides (optional)
# [vlm.model_config."qwen3-vl:4b"]
# timeout = 300
# num_ctx = 4096
```

---

## [browse] Section

Controls default browsing behavior for both the CLI `maki search` and the web UI.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `default_filter` | string | *(none)* | Search filter applied to all browse, search, and stroll views. Uses the same syntax as the search bar. |

The default filter is AND'd with whatever the user types — it acts as a persistent base filter. In the web UI, a toggle next to the search bar lets you temporarily disable it.

**Scope**: Applied to browse, search results, stroll, analytics, and map views. NOT applied to operational commands (`export`, `describe`, `contact-sheet`, `auto-tag`, etc.) where you pass your own explicit query.

### Examples

```toml
# Only show rated assets (hide unreviewed and unrated)
[browse]
default_filter = "rating:1+"

# Hide assets tagged "rest" (recommended for most users)
[browse]
default_filter = "-tag:rest"

# Combine multiple conditions
[browse]
default_filter = "-tag:rest -tag:rejected"

# Show only images
[browse]
default_filter = "type:image"
```

---

## [writeback] Section

Controls whether metadata edits flow into the `.xmp` recipe files on your storage volumes **automatically** on every change.

### enabled

- **Type:** boolean
- **Default:** `false`

This flag governs *automatic* (a.k.a. inline) writeback. It does **not** disable manual writeback — `maki writeback` always works regardless of this setting and is the supported way to keep auto-flush off as a safety net while still pushing staged metadata onto disk on demand.

| `enabled` | What happens on rating/label/description/tag edits |
|-----------|----------------------------------------------------|
| `false` *(default)* | Edit lands in YAML sidecar + SQLite catalog. The recipe is flagged `pending_writeback = true`. The `.xmp` file on disk is **not** touched. Run `maki writeback` to flush staged changes whenever you want. |
| `true` | Edit lands in YAML/SQLite **and** is written to the recipe's `.xmp` immediately (or queued as `pending_writeback` if the recipe lives on an offline volume). |

> **Note:** Edits are always tracked. Even with `enabled = false`, the catalog and the recipe's `pending_writeback` flag mark exactly which assets have staged edits — `maki status` surfaces the count, and `maki writeback` (optionally narrowed by query/volume) flushes them. The flag is purely "should every keystroke touch disk, or do I want a single explicit flush button?"

```toml
[writeback]
enabled = true   # auto-flush on every edit
```

To rematerialise the catalog metadata onto disk for a specific asset set without changing the config, use `maki writeback --all <query>`. That writes every XMP in the matching set whether or not it's flagged pending — useful right after a large catalog-only restructuring (rename, split, rebuild).

---

## [cli] Section

Default global flags. Values are OR'd with command-line flags — setting `log = true` here is equivalent to always passing `--log`.

### log

- **Type:** boolean
- **Default:** `false`

Enable per-file progress logging (`--log`) by default on all commands.

### time

- **Type:** boolean
- **Default:** `false`

Show elapsed time (`--time`) by default after every command.

### verbose

- **Type:** boolean
- **Default:** `false`

Enable verbose output (`--verbose`) by default.

```toml
[cli]
log = true
time = true
```

---

## Full Example

A complete `maki.toml` with all options set and annotated:

```toml
# Default volume for import when auto-detection is ambiguous.
# Find volume UUIDs with: maki volume list --json
default_volume = "550e8400-e29b-41d4-a716-446655440000"

[preview]
# Maximum pixel size of the longest edge for thumbnails.
max_edge = 1200
# Output format: "jpeg" (lossy) or "webp" (lossless).
format = "jpeg"
# JPEG quality (1-100). Ignored for WebP.
quality = 90
# Smart preview: maximum pixel size of the longest edge.
smart_max_edge = 2560
# Smart preview: JPEG quality (1-100).
smart_quality = 85
# Generate smart previews on first web request.
generate_on_demand = true

[serve]
# Web UI port. Override with: maki serve --port 9090
port = 8080
# Bind address. Use "0.0.0.0" to allow network access.
bind = "127.0.0.1"
# Stroll page: initial neighbor count and slider maximum.
stroll_neighbors = 12
stroll_neighbors_max = 25
# Stroll page: initial fan-out count and slider maximum.
stroll_fanout = 5
stroll_fanout_max = 10
# Stroll page: candidate pool size for Discover mode.
stroll_discover_pool = 80

[import]
# Glob patterns to exclude during import (matched against filenames).
exclude = [
    "Thumbs.db",
    ".DS_Store",
    "*.tmp",
    "*.bak",
    "desktop.ini",
]
# Tags automatically applied to every new asset.
auto_tags = ["inbox", "unreviewed"]
# Generate smart previews during import.
smart_previews = true
# Generate embeddings for visual similarity search during import (Pro).
embeddings = true
# Generate VLM descriptions during import (requires running Ollama or compatible endpoint).
descriptions = true

[dedup]
# Default path substring for --prefer (keep files whose path contains this).
prefer = "Selects"

[verify]
# Skip files verified within this many days (incremental verify).
max_age_days = 30

[contact_sheet]
# Layout preset: "dense", "standard", "large".
layout = "standard"
# Paper size: "a4", "letter", "a3".
paper = "a4"
# Comma-separated metadata fields below each thumbnail.
fields = "filename,date,rating"
# Page margin in mm.
margin = 15.0
# JPEG quality for page images (1-100).
quality = 92
# Color label display: "border", "dot", "none".
label_style = "border"
# Copyright text for page footer.
copyright = ""

# AI auto-tagging settings (Pro).
[ai]
model = "siglip-vit-b16-256"
threshold = 0.3
# labels = "my-labels.txt"
model_dir = "~/.maki/models"
prompt = "a photograph of {}"
# GPU acceleration (included automatically on macOS Pro builds).
# execution_provider = "auto"

# XMP writeback: disabled by default for safety.
# Enable to write rating/tags/label/description back to .xmp files on disk.
[writeback]
enabled = false

# VLM image description settings (Pro).
[vlm]
endpoint = "http://localhost:11434"
model = "qwen2.5vl:3b"
max_tokens = 500
timeout = 300
temperature = 0.7
mode = "describe"
# Models offered in web UI dropdown (empty = only default model, no dropdown).
# models = ["qwen2.5vl:3b", "moondream"]
# prompt = "Describe this photograph concisely."
# Sampling parameters (0 = not set, use server default).
# num_ctx = 4096
# top_p = 0.9
# top_k = 40
# repeat_penalty = 1.1

# Per-model VLM overrides (optional).
# [vlm.model_config."moondream:latest"]
# max_tokens = 200
# temperature = 0.1
```

---

## Minimal Examples

### Preview-only: larger thumbnails in WebP

```toml
[preview]
max_edge = 1600
format = "webp"
```

### Import-only: exclude system files and auto-tag

```toml
[import]
exclude = [".DS_Store", "Thumbs.db", "*.tmp"]
auto_tags = ["2026-import"]
```

### Serve-only: custom port for LAN access

```toml
[serve]
port = 9090
bind = "0.0.0.0"
```

---

## Defaults Summary

When a field is absent from `maki.toml`, these defaults apply:

| Field | Default |
|-------|---------|
| `default_volume` | none |
| `preview.max_edge` | `800` |
| `preview.format` | `"jpeg"` |
| `preview.quality` | `85` |
| `preview.smart_max_edge` | `2560` |
| `preview.smart_quality` | `85` |
| `preview.generate_on_demand` | `false` |
| `serve.port` | `8080` |
| `serve.bind` | `"127.0.0.1"` |
| `serve.per_page` | `60` |
| `serve.stroll_neighbors` | `12` |
| `serve.stroll_neighbors_max` | `25` |
| `serve.stroll_fanout` | `5` |
| `serve.stroll_fanout_max` | `10` |
| `serve.stroll_discover_pool` | `80` |
| `import.exclude` | `[]` |
| `import.auto_tags` | `[]` |
| `import.smart_previews` | `false` |
| `import.embeddings` | `false` |
| `import.descriptions` | `false` |
| `dedup.prefer` | none |
| `verify.max_age_days` | none |
| `contact_sheet.layout` | `"standard"` |
| `contact_sheet.paper` | `"a4"` |
| `contact_sheet.fields` | `"filename,date,rating"` |
| `contact_sheet.margin` | `15.0` |
| `contact_sheet.quality` | `92` |
| `contact_sheet.label_style` | `"border"` |
| `contact_sheet.copyright` | `""` |
| `ai.model` | `"siglip-vit-b16-256"` |
| `ai.threshold` | `0.1` |
| `ai.labels` | none |
| `ai.model_dir` | `"~/.maki/models"` |
| `ai.prompt` | `"a photograph of {}"` |
| `ai.execution_provider` | `"auto"` |
| `ai.face_cluster_threshold` | `0.35` |
| `ai.face_min_confidence` | `0.7` |
| `ai.text_limit` | `50` |
| `vlm.endpoint` | `"http://localhost:11434"` |
| `vlm.model` | `"qwen2.5vl:3b"` |
| `vlm.max_tokens` | `500` |
| `vlm.prompt` | none (built-in) |
| `vlm.mode` | `"describe"` |
| `vlm.temperature` | `0.7` |
| `vlm.timeout` | `300` |
| `vlm.concurrency` | `1` |
| `vlm.models` | `[]` (empty) |
| `vlm.num_ctx` | `0` (server default) |
| `vlm.top_p` | `0.0` (server default) |
| `vlm.top_k` | `0` (server default) |
| `vlm.repeat_penalty` | `0.0` (server default) |
| `writeback.enabled` | `false` |
| `browse.default_filter` | none |

---

## Related Topics

- [Setup (User Guide)](../user-guide/02-setup.md) -- creating a catalog with `maki init`
- [CLI Conventions](00-cli-conventions.md) -- global flags and catalog discovery
- [Search Filters Reference](06-search-filters.md) -- search query syntax
