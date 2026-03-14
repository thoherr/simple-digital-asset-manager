# VLM Model Guide

This document covers vision-language models (VLMs) compatible with `dam describe`. It lists tested models, hardware requirements, quality notes, and setup instructions for each inference backend.

dam uses the **OpenAI-compatible `/v1/chat/completions`** endpoint with base64-encoded images. Any server that implements this API works — Ollama, llama.cpp, vLLM, LM Studio, SGLang, or cloud providers.

---

## Quick Start

1. Install [Ollama](https://ollama.com) (or another backend — see [Backends](#backends) below).
2. Pull a model: `ollama pull qwen2.5vl:3b`
3. Test connectivity: `dam describe --check`
4. Describe assets: `dam describe "description:none" --apply --log`

To switch models, either set `[vlm] model` in `dam.toml` or pass `--model` on the command line.

---

## Tested Models

All timings measured on Apple M3 Pro (18 GB) with Ollama, using preview images (~800px). Your results will vary with hardware, image size, and model quantization.

### Recommended for Photography

| Model | Ollama tag | Size | RAM | Speed | Quality | Notes |
|-------|-----------|------|-----|-------|---------|-------|
| **Qwen2.5-VL 3B** | `qwen2.5vl:3b` | 2.0 GB | ~3 GB | ~8--12s | Very good | Default. Best balance of speed, quality, and resource use. |
| **Qwen3-VL 8B** | `qwen3-vl:8b` | 5.2 GB | ~6 GB | ~15--20s | Excellent | Recommended upgrade from the default. Noticeably better descriptions. |
| **Qwen3-VL 4B** | `qwen3-vl:4b` | 2.8 GB | ~4 GB | ~10--15s | Very good | Good step up from Qwen2.5-VL 3B without much extra RAM. |
| **Qwen2.5-VL 7B** | `qwen2.5vl:7b` | 4.7 GB | ~6 GB | ~20--36s | Excellent | Proven, widely tested. |
| **Gemma 3 4B** | `gemma3:4b` | 3.3 GB | ~4 GB | ~10--15s | Very good | Strong reasoning, good at scene understanding. |

### Budget / Batch Processing

| Model | Ollama tag | Size | RAM | Speed | Quality | Notes |
|-------|-----------|------|-----|-------|---------|-------|
| **Moondream 2B** | `moondream` | 1.7 GB | ~2 GB | ~3--5s | Good | Fastest option. Good for bulk describe passes before refining with a larger model. |
| **SmolVLM 2.2B** | `smolvlm` | 1.5 GB | ~2 GB | ~4--8s | Good | HuggingFace, very compact. Similar niche to Moondream. |

### Large / High Quality

| Model | Ollama tag | Size | RAM | Speed | Quality | Notes |
|-------|-----------|------|-----|-------|---------|-------|
| **Qwen3-VL 32B** | `qwen3-vl:32b` | 20 GB | ~24 GB | ~60--90s | Outstanding | Best quality via Ollama. Needs 32 GB+ system RAM. |
| **LLaVA 1.6 7B** | `llava:7b` | 4.7 GB | ~6 GB | ~15--25s | Good | Well-established, wide compatibility. |

### Qwen3.5 (Next Generation)

Qwen3.5 models use **early fusion** — vision and text are processed jointly from the earliest layers, giving better visual reasoning than the separate-encoder approach of older models. All Qwen3.5 models are natively multimodal (no separate "-VL" variant).

| Model | Size | RAM | Quality | Backend | Notes |
|-------|------|-----|---------|---------|-------|
| **Qwen3.5 4B** | ~3 GB | ~4 GB | Very good | llama.cpp, vLLM | Comparable to Qwen3-VL 8B in some benchmarks. |
| **Qwen3.5 9B** | ~6 GB | ~8 GB | Excellent | llama.cpp, vLLM | Best quality-per-GB. Strong upgrade path. |
| **Qwen3.5 27B** | ~16 GB | ~20 GB | Outstanding | llama.cpp, vLLM | Needs significant RAM; best local quality. |

**Ollama caveat (as of March 2026):** Ollama cannot handle Qwen3.5 vision — the model's `mmproj` vision files are not supported yet. Text-only works, but image input silently fails. Use llama.cpp or vLLM for Qwen3.5 multimodal. This will likely be resolved in a future Ollama release.

---

## Backends

### Ollama (Recommended)

The simplest setup. Ollama manages model downloads, quantization, and serves an OpenAI-compatible API.

**Install:** https://ollama.com

```bash
# Pull a model
ollama pull qwen2.5vl:3b

# The server starts automatically; default endpoint is http://localhost:11434
dam describe --check
```

```toml
# dam.toml
[vlm]
endpoint = "http://localhost:11434"
model = "qwen2.5vl:3b"
```

**Supported models:** All models with Ollama vision support — Qwen2.5-VL, Qwen3-VL, LLaVA, Moondream, Gemma 3, SmolVLM. Not Qwen3.5 (see caveat above).

**Concurrency:** Ollama handles one request at a time by default. For `concurrency > 1`, set `OLLAMA_NUM_PARALLEL` environment variable or increase `num_parallel` in Ollama's config.

### llama.cpp

Direct inference with GGUF model files. Supports Qwen3.5 multimodal via the `--mmproj` flag.

**Install:** https://github.com/ggerganov/llama.cpp

```bash
# Download model files (e.g. from HuggingFace)
# You need both the main model and the vision projector (mmproj) file

# Start the server with vision support
llama-server \
  -m Qwen3.5-9B-Q4_K_M.gguf \
  --mmproj mmproj-BF16.gguf \
  --host 0.0.0.0 \
  --port 8080
```

```toml
# dam.toml
[vlm]
endpoint = "http://localhost:8080"
model = "Qwen3.5-9B"
```

**Key points:**
- Vision requires the `--mmproj` flag with the separate vision projector file
- Quantized GGUF files (Q4_K_M, Q5_K_M) reduce RAM needs significantly
- No automatic model management — you download and manage files yourself
- Serves the OpenAI-compatible API at `/v1/chat/completions`

### vLLM

High-throughput inference server, best for GPU machines and batch processing. Supports Qwen3.5 natively.

**Install:** https://docs.vllm.ai

```bash
# Requires vLLM 0.17.0+ for Qwen3.5
pip install vllm

# Start the server
vllm serve Qwen/Qwen3.5-9B --host 0.0.0.0 --port 8000
```

```toml
# dam.toml
[vlm]
endpoint = "http://localhost:8000"
model = "Qwen/Qwen3.5-9B"
concurrency = 4   # vLLM handles parallel requests well
```

**Key points:**
- Best throughput for batch processing (set `concurrency` higher)
- GPU recommended (CUDA, ROCm); CPU inference is slow
- Automatic model download from HuggingFace
- Full OpenAI-compatible API

### LM Studio

Desktop application with a GUI for model management and a built-in server.

**Install:** https://lmstudio.ai

```toml
# dam.toml
[vlm]
endpoint = "http://localhost:1234"
model = "qwen2.5-vl-3b"   # Use the model name shown in LM Studio
```

### Cloud APIs

Any OpenAI-compatible cloud API works. Note that cloud APIs charge per request and dam does not set authentication headers — use a local proxy if your endpoint requires an API key.

```toml
# dam.toml — example with a self-hosted proxy that adds auth
[vlm]
endpoint = "https://api.openai.com"
model = "gpt-4o"
```

---

## Choosing a Model

### By Use Case

| Scenario | Recommended Model | Why |
|----------|-------------------|-----|
| **Daily use, modest hardware** | Qwen2.5-VL 3B | Fast, 3 GB RAM, good quality |
| **Best quality via Ollama** | Qwen3-VL 8B | Excellent descriptions, reasonable speed |
| **Bulk first pass** | Moondream 2B | 3--5s per image, good-enough descriptions |
| **Best local quality** | Qwen3.5 9B (llama.cpp) | Early fusion, strong reasoning |
| **32 GB+ Mac or GPU server** | Qwen3-VL 32B or Qwen3.5 27B | Near-cloud quality |
| **Tag suggestions** | Qwen3-VL 8B | Structured output reliability |
| **Architectural / technical** | Gemma 3 4B or Qwen3-VL 8B | Good at detail and materials |
| **Multilingual descriptions** | Qwen3.5 (any size) | 201 languages |

### By Hardware

| System RAM | GPU VRAM | Suggested Models |
|-----------|----------|------------------|
| 8 GB | — | Moondream 2B, SmolVLM 2.2B |
| 16 GB | — | Qwen2.5-VL 3B (default), Gemma 3 4B, Qwen3-VL 4B |
| 24--32 GB | — | Qwen3-VL 8B, Qwen2.5-VL 7B, Qwen3.5 9B |
| 32 GB+ | — | Qwen3-VL 32B, Qwen3.5 27B |
| — | 8 GB | Qwen3-VL 8B (FP16), Qwen3.5 9B (Q4) |
| — | 16 GB+ | Qwen3.5 27B (Q4), Qwen3-VL 32B (Q4) |

---

## Model Comparison

### Qwen Model Generations

| | Qwen2.5-VL | Qwen3-VL | Qwen3.5 |
|-|-----------|----------|---------|
| **Architecture** | Late fusion (separate vision encoder) | Late fusion (improved encoder) | Early fusion (native multimodal) |
| **Vision quality** | Good | Very good | Best |
| **Document understanding** | Good | Better | Best (90.8 OmniDocBench) |
| **Context length** | 128K | 128K | 256K |
| **Languages** | ~29 | ~119 | 201 |
| **Ollama vision** | Yes | Yes | Not yet |
| **llama.cpp vision** | Yes | Yes | Yes (with mmproj) |

For most users, **Qwen3-VL via Ollama** is the practical sweet spot right now. When Ollama adds Qwen3.5 vision support, it will become the clear best choice.

### Description Quality (Subjective)

Based on describing the same set of 50 photographs (landscapes, portraits, architecture, street):

- **Moondream 2B**: Short, accurate, occasionally generic. Good at identifying subjects, weaker on mood/lighting.
- **Qwen2.5-VL 3B**: Solid all-rounder. Good detail, reasonable sentence structure. Occasionally repetitive across similar images.
- **Qwen3-VL 4B**: Noticeable step up in vocabulary and scene understanding over Qwen2.5-VL 3B.
- **Gemma 3 4B**: Strong on materials, textures, and spatial relationships. Slightly less natural prose.
- **Qwen3-VL 8B**: Rich, natural descriptions. Good at lighting, mood, composition. Rarely generic.
- **Qwen2.5-VL 7B**: Similar quality to Qwen3-VL 8B but slower.
- **Qwen3.5 9B**: Best visual reasoning. Catches subtle details (reflections, depth of field, lens characteristics). Needs llama.cpp or vLLM.

---

## Tips

**Start small, upgrade later.** Run `dam describe` with Moondream for a quick first pass, then re-describe the best assets with a larger model using `--force`:

```bash
dam describe "description:none" --model moondream --apply --log
dam describe "rating:4+" --model qwen3-vl:8b --apply --force --log
```

**Use `--mode tags` for discovery.** VLM-suggested tags can surface patterns you haven't tagged manually:

```bash
dam describe --mode tags "date:2024-06" --apply --log
```

**Lower temperature for batch consistency.** When describing hundreds of images, `--temperature 0` gives more uniform output:

```bash
dam describe "type:image" --temperature 0 --apply --log
```

**Increase timeout for first load.** Ollama loads the model into memory on first request, which can take 30--60 seconds for larger models:

```bash
dam describe --model qwen3-vl:8b --timeout 300 --asset a1b2c3d4
```

**Use concurrency with capable servers.** If your server handles parallel requests (vLLM, GPU Ollama with `OLLAMA_NUM_PARALLEL`):

```toml
[vlm]
concurrency = 4
```

---

## See Also

- [Configuration Reference — \[vlm\] section](08-configuration.md#vlm-section) — all configuration options
- [dam describe](02-ingest-commands.md#dam-describe) — command reference
- [Ingesting Assets](../user-guide/03-ingest.md) — auto-describe during import
