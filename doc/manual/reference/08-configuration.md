# Configuration Reference (dam.toml)

The `dam.toml` file stores catalog-level configuration. It lives at the root of your catalog directory (the same directory that contains the `metadata/`, `previews/`, and `catalog.db` files).

---

## File Location

`dam.toml` is created automatically by `dam init`. dam locates it by searching the current directory and walking up through parent directories until it finds a directory containing `dam.toml`.

```
my-catalog/
  dam.toml              <-- configuration file
  catalog.db
  metadata/
  previews/
  searches.toml
  collections.yaml
```

All sections and fields are optional. A missing file or an empty file is equivalent to all-defaults. A comment-only file is also valid:

```toml
# dam catalog configuration
```

---

## Top-Level Fields

### default_volume

- **Type:** UUID string (optional)
- **Default:** none

Fallback volume for `dam import` when auto-detection from the file path is ambiguous or fails. When set, import uses this volume if it cannot determine the correct volume from the first path argument.

```toml
default_volume = "550e8400-e29b-41d4-a716-446655440000"
```

Find your volume UUIDs with `dam volume list --json`.

---

## [preview] Section

Controls how preview thumbnails are generated during import and by `dam generate-previews`.

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

When `false`, smart previews must be generated explicitly via `dam import --smart`, the "Generate smart preview" button on the asset detail page, or a future batch command.

### Notes

Changing `max_edge` or `format` affects only newly generated previews. Existing previews are not automatically regenerated. Use `dam generate-previews --force` to regenerate all previews with the new settings.

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

Controls the built-in web UI server started by `dam serve`.

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

The `--port`, `--bind`, and `--per-page` flags on `dam serve` override the values from `dam.toml`:

```bash
dam serve --port 9090 --bind 0.0.0.0 --per-page 100
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

Controls import behavior for `dam import`.

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

When `true`, import automatically generates smart previews (high-resolution, 2560px) alongside regular thumbnails. Equivalent to passing `--smart` on every `dam import` command. Smart preview dimensions are controlled by `[preview] smart_max_edge`.

```toml
[import]
smart_previews = true
```

### embeddings

> **Feature-gated**: requires building with `--features ai`.

- **Type:** boolean
- **Default:** `false`

When `true`, import automatically generates SigLIP image embeddings for visual similarity search alongside previews. Equivalent to passing `--embed` on every `dam import` command. Embeddings enable `dam auto-tag --similar` and the web UI "Find similar" button.

Uses the model configured in `[ai] model`. Silently skips if the model is not downloaded. Non-image assets are skipped.

```toml
[import]
embeddings = true
```

---

## [dedup] Section

Controls dedup behavior for `dam dedup` and the web UI's auto-resolve action.

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

Controls incremental verify behavior for `dam verify`.

### max_age_days

- **Type:** integer (optional)
- **Default:** none

Default value for the `--max-age` flag. When set, `dam verify` skips files verified within the given number of days. The CLI `--max-age` flag overrides this value. `--force` overrides both.

```toml
[verify]
max_age_days = 30
```

---

## [contact_sheet] Section

Default settings for `dam contact-sheet`. All fields are optional; CLI flags override these values.

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

## [ai] Section

> **Feature-gated**: these settings only take effect when dam is built with `--features ai`.

Controls AI auto-tagging behavior for `dam auto-tag`.

### model

- **Type:** string
- **Default:** `"siglip-vit-b16-256"`

Which SigLIP model to use. Available models: `siglip-vit-b16-256` (768-dim, ~207 MB, good balance) and `siglip-vit-l16-256` (1024-dim, ~670 MB, higher accuracy). The CLI `--model` flag overrides this value. Embeddings are stored per model, so switching models doesn't corrupt existing data.

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
- **Default:** `"~/.dam/models"`

Where to cache downloaded model files. The `~` prefix is expanded to the user's home directory.

### prompt

- **Type:** string
- **Default:** `"a photograph of {}"`

Text encoder prompt template. The `{}` placeholder is replaced with each label name before encoding. Adjusting the prompt can improve classification accuracy for specific use cases (e.g., `"a photo of a {}"` or `"a professional photograph of {}"`).

### execution_provider

- **Type:** string (`"auto"`, `"cpu"`, `"coreml"`)
- **Default:** `"auto"`

> Requires building with `--features ai-gpu` for GPU providers. With `--features ai` only, all values fall back to CPU.

Selects the ONNX Runtime execution provider for AI inference (SigLIP, YuNet, ArcFace). `"auto"` uses CoreML when available on macOS (Neural Engine on Apple Silicon, Metal on Intel), falling back to CPU. `"cpu"` forces CPU-only inference. `"coreml"` explicitly requests CoreML (errors if unavailable).

### face_cluster_threshold

- **Type:** float (0.0--1.0)
- **Default:** `0.5`

Similarity threshold for face auto-clustering. Higher values require faces to be more similar to be grouped into the same person. Lower values produce larger groups with more potential false matches.

### face_min_confidence

- **Type:** float (0.0--1.0)
- **Default:** `0.5`

Minimum confidence score for a face detection to be stored. Faces below this threshold are discarded during detection.

```toml
[ai]
threshold = 0.3
labels = "my-labels.txt"
model_dir = "~/.dam/models"
prompt = "a photograph of {}"
execution_provider = "auto"
face_cluster_threshold = 0.5
face_min_confidence = 0.5
```

---

## [vlm] Section

Controls the VLM (vision-language model) integration for `dam describe`. Unlike the `[ai]` section, this requires no special build features -- it works with any dam binary because it uses HTTP calls to an external server.

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

> **Note:** Cloud APIs charge per request. dam does not set authentication headers -- if your endpoint requires an API key, you may need to use a local proxy or set the key via the server's own configuration.

### model

- **Type:** string
- **Default:** `"qwen2.5vl:3b"`

Model name passed to the VLM server. For Ollama, this is the model tag (e.g., `moondream`, `qwen2.5vl:3b`, `qwen2.5vl:7b`). For cloud APIs, this is the model identifier (e.g., `gpt-4o`).

**Recommended models for photography** (tested with Ollama on Apple Silicon):

| Model | Size | RAM | Speed (M3 Pro) | Quality |
|-------|------|-----|-----------------|---------|
| Moondream 2B | 1.7 GB | ~2 GB | ~3--5s | Good, fast for batch |
| Qwen2.5-VL 3B | 2.0 GB | ~3 GB | ~8--12s | Very good (default) |
| Gemma 3 4B | 3.3 GB | ~4 GB | ~10--15s | Very good |
| Qwen2.5-VL 7B | 4.7 GB | ~6 GB | ~20--36s | Excellent |
| LLaVA 1.6 7B | 4.7 GB | ~6 GB | ~15--25s | Good |
| SmolVLM 2.2B | 1.5 GB | ~2 GB | ~4--8s | Good, very compact |

### max_tokens

- **Type:** unsigned 32-bit integer
- **Default:** `200`

Maximum number of tokens in the VLM response. 200 tokens is typically 2--3 sentences. Increase for more detailed descriptions.

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

Default output mode for `dam describe`. One of: `describe` (natural language description), `tags` (JSON tag suggestions), `both` (two separate VLM calls: one for description, one for tags).

### timeout

- **Type:** unsigned 32-bit integer (seconds)
- **Default:** `120`

Maximum time to wait for a VLM response. Larger models on CPU may need higher timeouts. Assets that time out are reported as errors and skipped.

### concurrency

- **Type:** unsigned 32-bit integer
- **Default:** `1`

Reserved for future use. Currently, assets are processed sequentially.

### CLI Override

The `--endpoint`, `--model`, `--prompt`, `--max-tokens`, `--timeout`, and `--mode` flags on `dam describe` override the values from `dam.toml`.

```toml
[vlm]
endpoint = "http://localhost:11434"
model = "qwen2.5vl:3b"
max_tokens = 200
timeout = 120
mode = "describe"
# prompt = "Custom prompt here."
```

---

## Full Example

A complete `dam.toml` with all options set and annotated:

```toml
# Default volume for import when auto-detection is ambiguous.
# Find volume UUIDs with: dam volume list --json
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
# Web UI port. Override with: dam serve --port 9090
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
# Generate embeddings for visual similarity search during import (--features ai).
embeddings = true

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

# AI auto-tagging settings (only with --features ai).
[ai]
model = "siglip-vit-b16-256"
threshold = 0.3
# labels = "my-labels.txt"
model_dir = "~/.dam/models"
prompt = "a photograph of {}"
# GPU acceleration (requires --features ai-gpu).
# execution_provider = "auto"

# VLM image description settings (works with any build).
[vlm]
endpoint = "http://localhost:11434"
model = "qwen2.5vl:3b"
max_tokens = 200
timeout = 120
mode = "describe"
# prompt = "Describe this photograph concisely."
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

When a field is absent from `dam.toml`, these defaults apply:

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
| `ai.model_dir` | `"~/.dam/models"` |
| `ai.prompt` | `"a photograph of {}"` |
| `ai.execution_provider` | `"auto"` |
| `ai.face_cluster_threshold` | `0.5` |
| `ai.face_min_confidence` | `0.5` |
| `vlm.endpoint` | `"http://localhost:11434"` |
| `vlm.model` | `"qwen2.5vl:3b"` |
| `vlm.max_tokens` | `200` |
| `vlm.prompt` | none (built-in) |
| `vlm.mode` | `"describe"` |
| `vlm.timeout` | `120` |
| `vlm.concurrency` | `1` |

---

## Related Topics

- [Setup (User Guide)](../user-guide/02-setup.md) -- creating a catalog with `dam init`
- [CLI Conventions](00-cli-conventions.md) -- global flags and catalog discovery
- [Search Filters Reference](06-search-filters.md) -- search query syntax
