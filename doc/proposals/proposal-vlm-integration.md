# Proposal: VLM Integration for Image Descriptions

**Status: Phases 1–3 implemented in v2.4.2.** Phase 4 (advanced features) remains open.

Natural language image descriptions via local vision-language models. Complements the existing SigLIP zero-shot classification (fixed tag vocabulary, fast, low memory) with open-ended understanding (scene descriptions, context, relationships, mood).

Referenced from [roadmap.md](roadmap.md) item "Ollama VLM Integration".

---

## Motivation

SigLIP auto-tagging classifies images against a predefined label set — effective for categorical tags like "landscape" or "portrait" but unable to describe *what's happening* in an image. Vision-language models (VLMs) generate free-form text descriptions that capture scene context, spatial relationships, artistic qualities, and narrative content that tags alone cannot express.

**Use cases:**
- Auto-generate descriptions for assets that lack them (batch enrichment)
- Semantic search via description text (`description:sunset over mountains`)
- Accessibility (alt-text for web UI previews)
- Archive documentation (what/where/when for future reference)
- Structured metadata extraction (suggested tags, location hints, event type)

---

## Approach Comparison

### Option A: Ollama (Local VLM Server)

Ollama manages model downloads, quantization, and GPU memory. DAM connects via HTTP API.

| Aspect | Details |
|--------|---------|
| **Dependency** | Ollama installed and running (`ollama serve`) |
| **Binary impact** | ~0 (HTTP calls via curl subprocess or minimal HTTP client) |
| **Model files** | 1–6 GB per model (managed by Ollama, not DAM) |
| **Runtime memory** | 2–8 GB depending on model size |
| **Latency** | 2–36s per image depending on model and hardware |
| **GPU support** | Automatic via Ollama (Metal on macOS, CUDA on Linux) |
| **Structured output** | JSON schema constraints via Ollama's `format` parameter |
| **Model updates** | `ollama pull` — independent of DAM releases |

**Strengths:** No new Rust dependencies. Model management delegated to Ollama. GPU acceleration handled transparently. Large model ecosystem. User can pick any model.

**Weaknesses:** Requires Ollama installed separately. Another process to manage. Network hop (localhost, negligible). Ollama must be running.

### Option B: ONNX Runtime (Embedded VLM)

Run a small VLM directly via the existing `ort` crate, same as SigLIP.

| Aspect | Details |
|--------|---------|
| **Dependency** | `ort` (already present), tokenizer for the specific VLM |
| **Binary impact** | +0 (reuses existing ONNX infrastructure) |
| **Model files** | 500 MB–2 GB (ONNX exports of small VLMs) |
| **Runtime memory** | 1–4 GB |
| **Latency** | 5–30s per image on CPU; faster with CoreML/CUDA |
| **GPU support** | Via existing `--features ai-gpu` infrastructure |
| **Structured output** | Must implement sampling/decoding in Rust |

**Strengths:** Self-contained — no external processes. Reuses existing AI infrastructure. Offline by design.

**Weaknesses:** Very few VLMs have reliable ONNX exports with text generation. Implementing autoregressive decoding (token-by-token generation with KV cache) in Rust is substantial work. Model selection limited to what's available as ONNX. Moondream and SmolVLM are the only practical options today.

### Option C: Generic HTTP Backend (Ollama, LM Studio, vLLM, OpenAI-compatible)

Support any server implementing the OpenAI-compatible `/v1/chat/completions` API with vision.

| Aspect | Details |
|--------|---------|
| **Dependency** | Any OpenAI-compatible vision API server |
| **Compatibility** | Ollama, LM Studio, vLLM, llama.cpp server, text-generation-inference |
| **Latency** | Varies by backend and model |
| **Flexibility** | User controls backend completely |
| **Cloud option** | Could also target OpenAI/Anthropic APIs (with user's API key) |

**Strengths:** Maximum flexibility — user chooses their stack. Future-proof against backend changes. One implementation covers many servers.

**Weaknesses:** Slightly more complex error handling (different servers have different quirks). Image encoding format varies (some expect base64 in message, some expect URLs).

### Recommendation

**Phase 1: Option C (OpenAI-compatible API)** with Ollama as the documented/tested backend. The OpenAI chat completions API is a de facto standard that Ollama, LM Studio, vLLM, and others all implement. This gives users maximum choice without locking to one backend.

**Rationale:** Ollama is the most accessible local VLM server (one-command install, automatic GPU), but the OpenAI-compatible API format is trivially portable. Building against the standard API means users can also use LM Studio, a remote vLLM instance, or even a cloud API if they choose.

---

## VLM Models (Recommended for Photography)

Tested with Ollama on Apple Silicon (M-series). Focus on models that handle photographic content well.

| Model | Size | VRAM | Speed (M3 Pro) | Quality | Notes |
|-------|------|------|-----------------|---------|-------|
| **Moondream 2B** | 1.7 GB | ~2 GB | ~3–5s | Good | Fast, lightweight, good for batch |
| **Qwen2.5-VL 3B** | 2.0 GB | ~3 GB | ~8–12s | Very good | Best quality/speed trade-off |
| **Qwen2.5-VL 7B** | 4.7 GB | ~6 GB | ~20–36s | Excellent | Best quality, slow on CPU |
| **Gemma 3 4B** | 3.3 GB | ~4 GB | ~10–15s | Very good | Google's latest, strong reasoning |
| **LLaVA 1.6 7B** | 4.7 GB | ~6 GB | ~15–25s | Good | Well-established, widely tested |
| **SmolVLM 2.2B** | 1.5 GB | ~2 GB | ~4–8s | Good | HuggingFace, very compact |

**Default recommendation:** Qwen2.5-VL 3B — best balance of quality, speed, and memory for a photographer's workflow.

---

## CLI Interface

```
dam describe [--query <Q>] [--asset <id>] [--volume <label>]
             [--model <name>] [--endpoint <url>]
             [--prompt <text>] [--max-tokens <N>]
             [--mode describe|tags|both]
             [--apply] [--force] [--dry-run]
             [--json] [--log] [--time]
```

### Arguments

| Flag | Default | Description |
|------|---------|-------------|
| `--query <Q>` | — | Scope to assets matching search query |
| `--asset <id>` | — | Single asset |
| `--volume <label>` | — | Scope to volume |
| `--model <name>` | `qwen2.5vl:3b` | Ollama model name (or any model the endpoint serves) |
| `--endpoint <url>` | `http://localhost:11434` | VLM server base URL |
| `--prompt <text>` | (built-in) | Custom system/user prompt |
| `--max-tokens <N>` | 200 | Maximum response length |
| `--mode` | `describe` | What to generate: `describe` (prose description), `tags` (structured tag suggestions), `both` |
| `--apply` | false | Write descriptions to assets (report-only by default) |
| `--force` | false | Overwrite existing descriptions |
| `--dry-run` | false | Show what would happen without calling the VLM |
| `--json` | false | Structured JSON output |
| `--log` | false | Per-asset progress to stderr |
| `--time` | false | Show elapsed time |

### Examples

```bash
# Describe all undescribed assets (report only)
dam describe --query "description:none"

# Apply descriptions to a specific volume
dam describe --volume "2024 Archive" --apply

# Use a faster model for bulk processing
dam describe --query "date:2024-06" --model moondream --apply

# Generate tag suggestions from VLM (complementary to SigLIP)
dam describe --asset abc123 --mode tags

# Use a remote server
dam describe --endpoint http://gpu-server:11434 --model qwen2.5vl:7b --apply

# Custom prompt for architectural photography
dam describe --prompt "Describe the architectural style, materials, and notable features of this building." --query "tag:architecture" --apply

# Dry run with JSON output
dam describe --query "rating:4+" --dry-run --json
```

---

## Modes

### `describe` (default)

Generates a natural language description (1–3 sentences) stored in `asset.description`. Uses a photography-aware prompt:

> *Describe this photograph in 1–3 concise sentences. Focus on the subject, setting, lighting, and mood. Be specific about what you see, not what you interpret.*

### `tags`

Requests structured tag suggestions via JSON schema. The VLM analyzes the image holistically and suggests tags that SigLIP's fixed vocabulary might miss (e.g., "golden hour", "leading lines", "family reunion"). Output format:

```json
{"tags": ["golden hour", "silhouette", "beach", "family", "warm tones"]}
```

Tags are merged with existing tags (deduplicated, XMP write-back triggered).

### `both`

Runs both modes as two separate VLM calls per asset — one for description, one for tags. Each call uses its optimal prompt for best results. Equivalent to running `--mode describe` and `--mode tags` independently.

---

## Configuration (`dam.toml`)

```toml
[vlm]
# VLM server endpoint (Ollama, LM Studio, vLLM, or any OpenAI-compatible API)
endpoint = "http://localhost:11434"

# Default model name
model = "qwen2.5vl:3b"

# Maximum tokens in response
max_tokens = 200

# Default mode: "describe", "tags", "both"
mode = "describe"

# Custom prompt (overrides built-in; use {mode} placeholder for mode-specific instructions)
# prompt = "Describe this photograph concisely."

# Request timeout in seconds
timeout = 120

# Concurrent requests (for servers that handle parallelism)
concurrency = 1
```

CLI flags override config values. Missing `[vlm]` section uses defaults.

---

## API Protocol

Uses the OpenAI-compatible chat completions endpoint, which Ollama (and others) implement:

```
POST {endpoint}/v1/chat/completions
Content-Type: application/json

{
  "model": "qwen2.5vl:3b",
  "messages": [
    {
      "role": "user",
      "content": [
        {
          "type": "image_url",
          "image_url": { "url": "data:image/jpeg;base64,{base64_image}" }
        },
        {
          "type": "text",
          "text": "Describe this photograph in 1-3 concise sentences..."
        }
      ]
    }
  ],
  "max_tokens": 200,
  "temperature": 0.3,
  "stream": false
}
```

For `tags` mode, add JSON schema constraint (Ollama's `format` field or OpenAI's `response_format`):

```json
{
  "response_format": {
    "type": "json_schema",
    "json_schema": {
      "name": "tags",
      "schema": {
        "type": "object",
        "properties": {
          "tags": { "type": "array", "items": { "type": "string" } }
        },
        "required": ["tags"]
      }
    }
  }
}
```

### Image Encoding

Preview images (smart preview preferred, then regular preview, then original) are base64-encoded and sent inline. Smart previews at 2560px are ideal — high enough quality for VLM analysis while keeping payload size reasonable (~200–500 KB base64).

### Fallback: Native Ollama API

If the `/v1/chat/completions` endpoint is not available (older Ollama versions), fall back to Ollama's native `/api/generate` endpoint:

```
POST {endpoint}/api/generate
{
  "model": "qwen2.5vl:3b",
  "prompt": "Describe this photograph...",
  "images": ["{base64_image}"],
  "stream": false
}
```

Detection: try `/v1/chat/completions` first; if 404, use `/api/generate`.

---

## Web UI Integration

### Asset Detail Page

- **"Describe" button** next to the description field (similar to "Suggest tags" button)
- Shows a loading spinner while waiting for the VLM response
- Populates the description field with the result; user can edit before saving
- Optional: show VLM-suggested tags as accept/dismiss chips (like AI tag suggestions)

### Batch Toolbar

- **"Describe" button** in the batch toolbar (like "Auto-tag")
- Confirm dialog showing asset count and estimated time
- Processes selected assets, writing descriptions where empty (or overwriting with `--force`)
- Refresh grid after completion

### Configuration

- `vlm_enabled` bool on template structs (like `ai_enabled`) controls button visibility
- Buttons hidden when no VLM endpoint is configured or reachable
- Startup health check: `GET {endpoint}/api/tags` (Ollama) to verify server is running

---

## Implementation

### New Files

| File | Purpose |
|------|---------|
| `src/vlm.rs` | VLM client: HTTP calls, prompt construction, response parsing |
| `src/config.rs` | `VlmConfig` struct (new `[vlm]` section) |

### Modified Files

| File | Changes |
|------|---------|
| `src/main.rs` | `Describe` command variant, handler |
| `src/lib.rs` | `pub mod vlm;` |
| `src/asset_service.rs` | `describe()` batch method (follows `auto_tag()` pattern) |
| `src/web/mod.rs` | Routes for describe endpoints |
| `src/web/routes.rs` | `POST /api/asset/{id}/describe`, `POST /api/batch/describe` |
| `src/web/templates.rs` | `vlm_enabled` field on page structs |
| `templates/asset.html` | "Describe" button, loading state |
| `templates/browse.html` | Batch "Describe" button |

### HTTP Client Approach

Use `curl` subprocess (consistent with existing model downloads in `model_manager.rs` and `face.rs`):

```rust
fn call_vlm(endpoint: &str, model: &str, image_base64: &str, prompt: &str, max_tokens: u32, timeout: u32) -> Result<String> {
    let body = serde_json::json!({
        "model": model,
        "messages": [{
            "role": "user",
            "content": [
                {"type": "image_url", "image_url": {"url": format!("data:image/jpeg;base64,{image_base64}")}},
                {"type": "text", "text": prompt}
            ]
        }],
        "max_tokens": max_tokens,
        "temperature": 0.3,
        "stream": false
    });

    let output = Command::new("curl")
        .args(["-sS", "-X", "POST",
               &format!("{endpoint}/v1/chat/completions"),
               "-H", "Content-Type: application/json",
               "-d", "@-",
               "--max-time", &timeout.to_string()])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    // Write body to stdin, read response from stdout
    // Parse JSON response, extract message content
}
```

No new crate dependencies required. If `curl` proves limiting (e.g., for concurrent requests in the web UI), `reqwest` can be added later as an optional dependency.

### Batch Processing Flow

Follows the existing `auto_tag()` pattern:

1. Resolve target assets (query/asset/volume filter, require at least one scope)
2. For each asset:
   a. Skip if description exists and `--force` not set
   b. Find best image (`find_image_for_ai()` — reuses existing logic)
   c. Read and base64-encode the image
   d. Call VLM endpoint
   e. Parse response (description text or JSON tags)
   f. If `--apply`: write description via `engine.edit()` or tags via `engine.tag()`
   g. Report progress via callback
3. Return result with counts: `described`, `skipped`, `failed`, `errors`

### Feature Gate

**No feature gate needed.** The VLM client is just HTTP calls — no heavy dependencies. The feature is opt-in by configuration (`[vlm]` section in `dam.toml`) and the server being available. Buttons in the web UI are hidden when no endpoint is configured.

This is a key advantage over the SigLIP integration which requires the `ai` feature flag for ONNX Runtime.

---

## Phases

### Phase 1: CLI `describe` Command — *Implemented v2.4.2*

- `src/vlm.rs`: HTTP client, prompt templates, response parsing
- `VlmConfig` in `src/config.rs`
- `dam describe` command with `--mode describe` only
- Batch processing with `--apply`, `--force`, `--dry-run`
- `--json`, `--log`, `--time` output flags
- Fallback from OpenAI API to native Ollama API
- Connectivity check (`dam describe --check`)

### Phase 2: Tag Suggestions Mode — *Implemented v2.4.2*

- `--mode tags` with JSON schema constraint
- `--mode both` as two separate VLM calls (one describe, one tags)
- Merge VLM-suggested tags with existing tags (deduplicated, case-insensitive)
- XMP write-back for new tags
- Truncated JSON recovery for tag responses cut off by max_tokens
- Configurable temperature (`--temperature` flag, `[vlm] temperature` config)

### Phase 3: Web UI — *Implemented v2.4.2*

- "Describe" button on asset detail page
- Batch "Describe" in browse toolbar
- Loading states, error handling
- `vlm_enabled` template flag with startup health check (5s timeout)

### Phase 4: Advanced Features (Future)

- Concurrent requests (`concurrency > 1`) for servers with batching
- Custom prompt library (`[vlm.prompts]` with named presets)
- Description-based semantic search (embed descriptions with SigLIP text encoder)
- Auto-describe during import (`dam import --describe`)
- Comparison mode: show multiple model outputs side-by-side for prompt tuning

---

## Edge Cases

| Scenario | Handling |
|----------|----------|
| Ollama not running | Error with helpful message: "VLM server not reachable at {endpoint}. Start Ollama with `ollama serve`." |
| Model not pulled | Error from API includes model name: "model 'qwen2.5vl:3b' not found. Run `ollama pull qwen2.5vl:3b`." |
| Timeout (slow model) | Configurable timeout, default 120s. Skip asset on timeout, report as error. |
| Empty response | Skip, report as error. Don't clear existing description. |
| Very long response | Truncate at `max_tokens`. Warn if response was cut off. |
| Non-image asset | Skip (audio, video, documents). Report as skipped. |
| No preview available | Skip. Suggest running `dam generate-previews` first. |
| Existing description | Skip unless `--force`. Report as skipped. |
| Hallucinated content | Can't prevent, but low temperature (0.3) and specific prompts reduce it. User reviews in report-only mode before `--apply`. |
| Rate limiting | `concurrency: 1` by default. Sequential processing is safe. |
| Offline volumes | Skip assets with no online file locations (consistent with other AI commands). |
| curl not installed | Error with message (same as existing model download handling). |

---

## Comparison with Existing SigLIP Auto-Tagging

| Aspect | SigLIP Auto-Tag | VLM Describe |
|--------|----------------|--------------|
| **Output** | Tags from fixed vocabulary | Free-form text descriptions |
| **Speed** | ~50–150 ms/image | ~3–36s/image |
| **Memory** | ~400–800 MB (model in process) | 0 MB in DAM (model in Ollama) |
| **Dependencies** | `--features ai`, ONNX models | `curl`, Ollama running |
| **GPU** | CoreML via `--features ai-gpu` | Automatic via Ollama |
| **Offline** | Yes (model embedded) | No (needs running server) |
| **Discovery** | Limited to predefined labels | Can identify novel concepts |
| **Batch scale** | 10k images in minutes | 10k images in hours/days |
| **Best for** | Categorical filtering, similarity search | Documentation, search enrichment |

The two approaches are complementary: SigLIP for fast categorical tagging, VLM for rich descriptions when quality matters more than speed.

---

## Open Questions

1. **Description vs. separate field?** Store VLM output in `asset.description` (existing field) or a new `asset.ai_description` field? Using the existing field is simpler and integrates with search immediately, but may overwrite human-written descriptions. Recommendation: use `asset.description` with `--force` guard — same field, user controls when to overwrite.

2. **Prompt tuning UX?** Different photography genres need different prompts (wildlife vs. architecture vs. portraits). A prompt library (`[vlm.prompts]` in config) could help, but adds complexity. Start simple with a single configurable prompt; add presets if users request them.

3. **Streaming?** For web UI, streaming responses would show text appearing progressively. The OpenAI API supports `"stream": true` with SSE. Nice-to-have for Phase 3 but not essential — a loading spinner is fine initially.

4. **Cost awareness for cloud APIs?** If users point the endpoint at OpenAI or Anthropic, each image costs money. Should the CLI show estimated cost? Probably out of scope — just document that cloud APIs have per-request costs.
