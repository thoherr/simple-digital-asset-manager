# Proposal: AI Auto-Tagging

Zero-shot image classification using CLIP/SigLIP embeddings for automated tag suggestions, plus visual similarity search as a bonus feature.

Referenced from [enhancements.md](enhancements.md) item 15 and [roadmap.md](roadmap.md).

---

## CLI Interface

```
dam auto-tag [--asset <id>] [--volume <label>] [--query <QUERY>]
             [--model mobile|standard|accurate|ollama:<name>]
             [--threshold 0.5] [--labels <file>]
             [--apply] [--json] [--log] [--time]
dam auto-tag --list-models
dam auto-tag --download <model>
dam auto-tag --remove-model <model>
```

Given a configurable label vocabulary (e.g., `landscape`, `portrait`, `architecture`, `animals/birds`), the model scores each image against every label and suggests tags above the threshold. Report-only by default (`--apply` writes tags).

**Bonus feature**: With embeddings stored per-asset, `dam search --similar <asset-id>` finds visually similar images by cosine distance.

**Web UI**: "Suggest tags" button on the asset detail page. Shows suggested tags with confidence badges. Click to accept.

---

## Approach Comparison

### Option A: Embedded ONNX Runtime (`ort` crate)

Ship ONNX Runtime as a linked library. Model files downloaded on first use or bundled.

| Aspect | Impact |
|--------|--------|
| **New dependency** | `ort` crate (~v1.13+), links against `libonnxruntime` |
| **Binary size increase** | +50-150 MB for the ONNX Runtime shared library (platform-dependent). The `dam` binary itself grows ~1-2 MB for the Rust wrapper code. With `minimal-build` feature: ~30-60 MB |
| **Model files on disk** | CLIP ViT-B/32: ~340 MB (vision encoder, FP32) or ~85 MB (INT8 quantized). Text encoder: ~250 MB (FP32) or ~65 MB (INT8). Total: **150-600 MB** depending on precision. Stored in `~/.dam/models/` or catalog `models/` dir |
| **Runtime memory** | ~400-800 MB RSS during inference (model loaded + image tensors + ONNX Runtime overhead). Freed after batch completes |
| **CPU inference** | ~50-200 ms per image on modern CPU (Apple Silicon / x86-64 with AVX2). ViT-B/32 is the smallest/fastest CLIP variant |
| **Compile time** | +2-5 minutes (ONNX Runtime build or download during `cargo build`) |
| **Platform support** | macOS (arm64, x86_64), Linux (x86_64, aarch64), Windows (x86_64) -- all supported by `ort` prebuilt binaries |

### Option B: Python Subprocess

Shell out to a Python script that uses `transformers` + `onnx_clip` or `open_clip`.

| Aspect | Impact |
|--------|--------|
| **New dependency** | Python 3.8+, pip packages (`transformers`, `Pillow`, `onnxruntime`) |
| **Binary size increase** | ~0 (just a bundled .py script) |
| **Disk overhead** | Python env + packages: ~500 MB-2 GB. Model files: same as above |
| **Runtime memory** | Same model memory + Python interpreter overhead (~100 MB extra) |
| **CPU inference** | Similar speed (same ONNX Runtime underneath), but ~500 ms startup penalty per invocation |
| **User friction** | Requires Python installed. Version conflicts. Virtual env management. Not self-contained |

### Option C: External API (Ollama / Local VLM)

Shell out to a local vision-language model server (Ollama with Qwen2.5-VL, Gemma 3, Moondream, etc.).

| Aspect | Impact |
|--------|--------|
| **New dependency** | Ollama installed and running (`ollama serve`) |
| **Binary size increase** | ~0 (HTTP client only, `reqwest` already used or trivial to add) |
| **Model files** | 1-6 GB per model (managed by Ollama, not by dam) |
| **Runtime memory** | 2-8 GB depending on model (Moondream 2B: ~2 GB, Qwen 7B: ~6 GB) |
| **Accuracy** | Higher for descriptions and subjective tags; can discover novel tags. Qwen2.5-VL 7B: 0.33% hallucination rate |
| **Latency** | 5-36 seconds per image on CPU (Moondream ~5s, Qwen 3B ~24s, Qwen 7B ~36s) |
| **Structured output** | Ollama supports JSON schema constraints natively |

---

## Models

This section compares the models available for Option A (embedded ONNX) and Option C (Ollama VLM), their sizes, accuracy, speed, and trade-offs. Understanding the model landscape is important because it drives the user-facing `--model` flag, the download/disk budget, and the accuracy expectations.

### Option A Models: CLIP-Family (Zero-Shot Classification via ONNX)

All CLIP-family models work the same way: encode the image and a set of text labels into a shared embedding space, then rank labels by cosine similarity. The user defines a tag vocabulary; the model scores each tag. No training required -- new tags work instantly.

#### OpenAI CLIP (Original)

| Model | Total Params | Image Encoder | Input Res | Embed Dim | ImageNet ZS | ONNX Vision Size (FP32) | CPU Inference |
|-------|-------------|---------------|-----------|-----------|-------------|------------------------|---------------|
| ViT-B/32 | 151M | 87M | 224x224 | 512 | 63.0% | ~340 MB | ~117 ms |
| ViT-B/16 | 150M | 86M | 224x224 | 512 | 68.3% | ~340 MB | ~200 ms |
| ViT-L/14 | 428M | 303M | 224x224 | 768 | 75.5% | ~1.2 GB | ~500 ms |
| ViT-L/14@336 | 428M | 303M | 336x336 | 768 | 76.2% | ~1.2 GB | ~800 ms |

Well-tested ONNX exports: [Qdrant split encoders](https://huggingface.co/Qdrant/clip-ViT-B-32-vision), [immich-app collection](https://huggingface.co/immich-app), [lakeraai/onnx_clip](https://github.com/lakeraai/onnx_clip). INT8 quantized variants reduce vision encoder to ~85 MB (ViT-B/32) or ~300 MB (ViT-L/14).

**Strengths**: Most widely deployed, best ecosystem support, well-documented ONNX exports, used by Immich.
**Weaknesses**: Lowest accuracy of the CLIP family. Training data (WIT-400M) is smaller than newer datasets.

#### OpenCLIP (LAION-Trained)

Community-trained CLIP models on larger datasets (LAION-2B, DataComp).

| Model | Training Data | ImageNet ZS | Notes |
|-------|--------------|-------------|-------|
| ViT-B/32 | LAION-2B | 66.6% | +3.6% over OpenAI ViT-B/32 |
| ViT-B/16 | LAION-400M | 67.1% | |
| ViT-L/14 | LAION-2B | 75.2% | Matches OpenAI |
| ViT-H/14 | LAION-2B | 78.0% | Best open CLIP (2023) |
| ViT-bigG/14 | LAION-2B | 80.1% | ~1.8B params, very large |

ONNX exports available via OpenCLIP tooling. Same architecture as OpenAI CLIP, so file sizes are comparable.

**Multilingual variants**: XLM-RoBERTa text encoders support 100+ languages. Relevant if tag vocabularies include non-English labels.

**Strengths**: Better accuracy than OpenAI CLIP at same model size. Open training data. Multilingual options.
**Weaknesses**: Larger models (ViT-H, ViT-G) are impractical for a CLI tool. ONNX exports less standardized than OpenAI's.

#### Google SigLIP

Replaces CLIP's softmax contrastive loss with a pairwise sigmoid loss. Consistently outperforms CLIP at equivalent model sizes.

| Model | Params | Input Res | ImageNet ZS | ONNX Vision Size (est.) |
|-------|--------|-----------|-------------|------------------------|
| ViT-B/16-256 | 86M | 256x256 | 74.1% (v1) / 79.1% (v2) | ~340 MB |
| ViT-B/16-384 | 86M | 384x384 | 76.7% (v1) | ~340 MB |
| ViT-L/16-256 | 303M | 256x256 | ~82% (v2) | ~1.2 GB |
| ViT-SO400M/14-384 | 400M | 384x384 | 83-84% (v2) | ~1.6 GB |

ONNX exports: [deepghs/siglip_onnx](https://huggingface.co/deepghs/siglip_onnx) (split image/text encoders), [Xenova/siglip-base-patch16-384](https://huggingface.co/Xenova/siglip-base-patch16-384), [immich-app collection](https://huggingface.co/immich-app).

**Strengths**: Best accuracy-to-size ratio. SigLIP ViT-B/16-256 (86M params) matches OpenAI ViT-L/14 (428M params) -- same accuracy at 1/5 the size and ~2x the speed. SigLIP 2 adds native multilingual support (140+ languages). Used by Moondream as the vision backbone.
**Weaknesses**: Different tokenizer than CLIP (SentencePiece vs BPE). Fewer pre-built ONNX exports than CLIP, though this is improving. Higher input resolution variants (384, 512) are slower.

#### Apple MobileCLIP

Distilled CLIP models optimized for edge/mobile deployment. Dramatically smaller image encoders.

| Model | Vision Params | ImageNet ZS | ONNX Vision Size (est.) | CPU Latency |
|-------|--------------|-------------|------------------------|-------------|
| MobileCLIP-S0 | 11.4M | 67.8% | ~46 MB (FP32) / ~12 MB (INT8) | ~3 ms (mobile) |
| MobileCLIP-S1 | 21.5M | 72.6% | ~86 MB (FP32) | ~5 ms |
| MobileCLIP-S2 | 35.7M | 74.4% | ~143 MB (FP32) | ~7 ms |
| MobileCLIP-B | 86.3M | 76.8% | ~340 MB (FP32) | ~14 ms |
| MobileCLIP2-S0 | 11.4M | 71.5% | ~46 MB (FP32) | ~3 ms |
| MobileCLIP2-S2 | 35.7M | 77.2% | ~143 MB (FP32) | ~7 ms |

Integrated into [OpenCLIP](https://huggingface.co/apple/MobileCLIP-S1-OpenCLIP). CoreML exports officially supported; ONNX export via standard PyTorch paths.

**Strengths**: Tiny image encoders. MobileCLIP-S0 at 11.4M vision params gets 67.8% ImageNet -- matching OpenAI ViT-B/16 at ~7.5x smaller. MobileCLIP2-S2 at 35.7M params beats SigLIP ViT-B/16 while being 2.3x faster. INT8-quantized S0 vision encoder would be ~12 MB -- negligible disk impact.
**Weaknesses**: Less community adoption for ONNX deployment. Accuracy gap widens on fine-grained categories. Still uses CLIP's BPE tokenizer (text encoder is standard CLIP).

#### TinyCLIP (Microsoft)

Knowledge-distilled tiny CLIP models, extremely small but lower accuracy.

| Model | Total Params | ImageNet ZS | Notes |
|-------|-------------|-------------|-------|
| ViT-8M/16-Text-3M | 11M | 41.1% | Smallest, but too weak for practical tagging |
| ViT-39M/16-Text-19M | 58M | 63.5% | Approaching usability |
| ViT-61M/32-Text-29M | 90M | 64.8% | Comparable to OpenAI ViT-B/32 |

**Strengths**: Very small. Fast. **Weaknesses**: Accuracy too low for most variants. MobileCLIP achieves better accuracy at similar sizes.

### Option C Models: Vision-Language Models (via Ollama)

VLMs generate free-form text descriptions of images. For tagging, the model is prompted to output a structured list (JSON via Ollama's structured output mode). Fundamentally different approach: the model *describes* the image rather than scoring it against a fixed vocabulary.

| Model | Params | Download | RAM (Q4) | CPU Speed | Hallucination Rate | Tag Quality |
|-------|--------|----------|----------|-----------|-------------------|-------------|
| Moondream 2 | 1.86B | ~1.1 GB | ~2 GB | ~5-15 s/img | Higher | Basic |
| Qwen2.5-VL 3B | 3B | ~2 GB | ~3 GB | ~24 s/img | 1.33% | Good |
| Gemma 3 4B | 4B | ~2.7 GB | ~4 GB | ~31 s/img | 2.0% | Good |
| LLaVA 1.6 7B | 7B | ~4.1 GB | ~6 GB | ~30 s/img | Moderate | Good |
| Qwen2.5-VL 7B | 7B | ~4.3 GB | ~6 GB | ~36 s/img | 0.33% | Best |
| Llama 3.2 Vision 11B | 11B | ~6.4 GB | ~8 GB+ | ~50 s/img | Moderate | Good |

Benchmarks from [PhotoPrism's model comparison](https://docs.photoprism.app/developer-guide/vision/model-comparison/) (AMD Ryzen AI 9, CPU-only). Ollama supports [structured JSON output](https://ollama.com/blog/structured-outputs) natively, enabling schema-constrained responses like `{"tags": [...], "description": "..."}`.

**Strengths**: Can discover tags not in any predefined vocabulary. Understands complex scenes compositionally ("woman walking dog in park at sunset"). Can provide descriptions alongside tags. Better at specific object identification. Qwen2.5-VL 7B has remarkably low hallucination (0.33%).
**Weaknesses**: 100-300x slower than CLIP on CPU. Requires 2-8 GB RAM while running. Output can be inconsistent (same image may get different tags on different runs). Requires Ollama as external dependency. Not practical for batch-processing thousands of images during import.

### Accuracy for Photography Tagging Specifically

The ImageNet zero-shot accuracy numbers above measure general object classification. For photography-specific tags (landscape, portrait, macro, street, architecture, golden hour, etc.), the picture is more nuanced:

**CLIP-family (zero-shot, ~100 photography tags)**:
- Broad categories (landscape, portrait, architecture, food, animals): 80-90% precision
- Medium categories (sunset, beach, forest, city, studio): 70-80% precision
- Subjective/aesthetic tags (moody, cinematic, editorial, dreamy): 40-60% precision
- Fine-grained (specific bird species, architectural styles, camera techniques): 30-50% precision

**VLMs (prompted for tags)**:
- Broad categories: 85-95% precision (slightly better)
- Medium categories: 75-85% precision
- Subjective/aesthetic: 60-75% precision (significantly better -- understands "mood")
- Fine-grained: 50-70% precision (better compositional understanding)

The CLIP accuracy improves significantly with prompt engineering: `"a photograph of a landscape"` works much better than just `"landscape"`. The text encoder prompt template is configurable.

### Model Selection Strategy

#### Should the user choose models?

Yes, but with sensible defaults and simple choices. The `--model` flag and `[ai] model` config accept a model identifier. Recommended tiers:

| Identifier | Model | Use Case |
|------------|-------|----------|
| `mobile` | MobileCLIP2-S0 (11.4M) | Fastest, smallest download (~50 MB). Good enough for broad categories |
| `standard` (default) | SigLIP ViT-B/16-256 (86M) | Best accuracy-to-speed ratio. ~340 MB download |
| `accurate` | SigLIP ViT-SO400M/14-384 (400M) | Highest accuracy. ~1.6 GB download. Slower (~500 ms/img) |
| `ollama:<model>` | Any Ollama vision model | VLM mode. e.g., `ollama:qwen2.5vl:3b`. Requires Ollama running |

The `mobile`, `standard`, and `accurate` tiers use Option A (embedded ONNX). The `ollama:*` prefix switches to Option C (HTTP API to Ollama). This way a single `--model` flag covers both approaches, and the user can experiment without recompiling.

#### Model management

```
dam auto-tag --list-models          # show available / downloaded models
dam auto-tag --download <model>     # pre-download a model
dam auto-tag --remove-model <model> # delete cached model files
```

Models are cached in `~/.dam/models/<model-id>/` (or `[ai] model_dir` from config). Each model directory contains the ONNX file(s) and a `manifest.json` with the expected SHA-256 hashes. On first use, the model is downloaded from HuggingFace with a progress bar. Integrity is verified before loading.

When the user switches models, embeddings stored from a different model are invalidated (the `model` column in the `embeddings` table tracks this). Re-tagging with a new model recomputes embeddings. Similarity search only compares embeddings from the same model.

### Recommendation

**Default: SigLIP ViT-B/16-256 (`standard`)**

This model hits the sweet spot:
- 79.1% ImageNet zero-shot (SigLIP 2) -- matches CLIP ViT-L/14 accuracy at 1/5 the parameter count
- ~340 MB download (FP32), ~85 MB with INT8 quantization
- ~200 ms per image on CPU -- fast enough for batch import
- 512-dimension embeddings -- compact for storage
- ONNX exports available via [deepghs/siglip_onnx](https://huggingface.co/deepghs/siglip_onnx) and [immich-app](https://huggingface.co/immich-app)
- Multilingual text encoder (SigLIP 2) -- tag vocabularies can include non-English labels

For users with constrained disk or who want maximum speed, `mobile` (MobileCLIP2-S0) is a compelling alternative at ~50 MB with 71.5% accuracy. For users who already run Ollama, `ollama:qwen2.5vl:3b` provides the highest quality with no additional disk cost to dam.

The hybrid approach (ONNX for bulk tagging + optional Ollama for on-demand enrichment) gives users the best of both worlds without forcing either dependency.

---

## Recommendation: Option A (Embedded `ort`) with Optional Feature Flag

**Why:**
1. **Self-contained** -- no external dependencies at runtime. "It just works" after model download.
2. **Fast** -- 50-200 ms/image is practical for batch processing thousands of images.
3. **Fits the project philosophy** -- dam is a single-binary CLI tool. Adding a Python dependency undermines that.
4. **Controllable size** -- use Cargo feature flag (`--features ai`) so users who don't want it pay zero cost. The ONNX Runtime library is only linked when the feature is enabled.
5. **Model download on demand** -- model files (~150 MB quantized) are downloaded on first `dam auto-tag` invocation, not bundled in the binary.

**Recommended default model**: SigLIP ViT-B/16-256 (`standard` tier). See the [Models](#models) section for the full comparison.

- 79.1% ImageNet zero-shot -- matches CLIP ViT-L/14 (a 5x larger model)
- ~340 MB download (FP32), ~85 MB with INT8 quantization
- ~200 ms/image on CPU -- fast enough for batch import
- 512-dimensional embeddings (compact for storage)
- ONNX exports available via deepghs/siglip_onnx and immich-app
- SigLIP 2 adds native multilingual support (140+ languages)
- Hybrid `ollama:<model>` tier allows VLM enrichment without additional compile-time cost

---

## Implementation Plan

### New Crate Dependencies

```toml
[dependencies]
ort = { version = "1.13", optional = true, default-features = false, features = ["download-binaries"] }
ndarray = { version = "0.16", optional = true }  # tensor manipulation

[features]
default = []
ai = ["ort", "ndarray"]
```

### New Files (~1500-2000 lines total)

| File | Lines | Purpose |
|------|-------|---------|
| `src/clip.rs` | ~400 | CLIP model loading, image preprocessing (resize/crop/normalize to 224x224, RGB f32 tensor), text tokenization (BPE), inference, cosine similarity |
| `src/embedding_store.rs` | ~200 | SQLite table `embeddings(variant_hash TEXT PK, embedding BLOB, model TEXT)`. Store/retrieve 512-dim f32 vectors. Cosine distance query for similarity search |
| `src/auto_tagger.rs` | ~300 | Orchestration: load label vocabulary, encode labels (cached), encode each image, compute similarities, threshold, return suggestions. Callback-based progress |
| `src/model_manager.rs` | ~200 | Download model files from HuggingFace on first use, verify SHA-256, cache in `~/.dam/models/` or catalog-relative `models/`. Version/integrity tracking |
| `src/asset_service.rs` | ~150 | New `auto_tag()` method + `AutoTagResult` struct |
| `src/main.rs` | ~80 | CLI registration + handler for `auto-tag` and `search --similar` |
| `tests/cli.rs` | ~200 | Integration tests (mocked model or small test model) |

### Schema Changes

```sql
CREATE TABLE IF NOT EXISTS embeddings (
    variant_hash TEXT PRIMARY KEY,
    embedding BLOB NOT NULL,      -- 512 x f32 = 2048 bytes per asset
    model TEXT NOT NULL DEFAULT 'clip-vit-b32'
);
CREATE INDEX idx_embeddings_model ON embeddings(model);
```

Storage overhead: ~2 KB per asset. For 100,000 assets: ~200 MB in SQLite.

### Image Preprocessing Pipeline (Pure Rust, No New Deps)

The existing `image` crate already handles loading/resizing. Preprocessing for CLIP:

1. Resize to 224x224 (center crop)
2. Convert to RGB f32 [0,1]
3. Normalize with CLIP means `[0.48145466, 0.4578275, 0.40821073]` and stds `[0.26862954, 0.26130258, 0.27577711]`
4. Reshape to `[1, 3, 224, 224]` NCHW tensor

### Text Tokenization

CLIP uses BPE tokenization. Options:

- **Embed a minimal BPE tokenizer** (~300 lines of Rust + ~1 MB vocab file) -- self-contained
- **Use `tokenizers` crate** from HuggingFace -- well-tested but adds ~5 MB to binary

Recommendation: embed minimal BPE. Label vocabularies are short (50-200 labels), so performance doesn't matter.

---

## Total Overhead Summary

| Metric | Without `--features ai` | With `--features ai` |
|--------|------------------------|---------------------|
| **Binary size** | 15 MB (unchanged) | ~65-165 MB (+50-150 MB for ONNX Runtime) |
| **Disk (models)** | 0 | ~150 MB (quantized) to ~600 MB (FP32), downloaded on first use |
| **Disk (embeddings)** | 0 | ~2 KB/asset (200 MB for 100K assets) |
| **RAM at rest** | unchanged | unchanged (models not loaded until needed) |
| **RAM during inference** | n/a | ~400-800 MB |
| **CPU during inference** | n/a | ~50-200 ms/image (one core saturated) |
| **Compile time** | ~90s | ~120-180s (+30-90s for ONNX Runtime download/link) |

---

## Effort Estimate

| Phase | Effort |
|-------|--------|
| Model manager + download | 1 day |
| CLIP preprocessing + inference | 2 days |
| BPE tokenizer | 1 day |
| Embedding store (SQLite) | 0.5 day |
| Auto-tagger orchestration | 1 day |
| CLI + handler | 0.5 day |
| `--similar` search integration | 1 day |
| Web UI (suggest tags button, similar-images link) | 1 day |
| Tests (unit + integration) | 1 day |
| Documentation | 0.5 day |
| **Total** | **~9-10 days** |

---

## Pros and Cons

### Pros

- **Biggest UX win remaining** -- manual tagging is the #1 friction point in any DAM
- **Even 70% accuracy saves massive time** -- user accepts/rejects rather than typing
- **Visual similarity search** is a natural bonus -- "find photos like this one" with near-zero extra effort
- **Feature flag keeps it optional** -- users who don't want AI pay nothing
- **No cloud dependency** -- all inference runs locally, no privacy concerns
- **Hierarchical tag support already exists** -- suggested tags can use the existing `/`-separated hierarchy
- **XMP write-back already exists** -- accepted suggestions flow back to CaptureOne/Lightroom automatically

### Cons

- **Large optional dependency** -- ONNX Runtime adds 50-150 MB to binary when enabled
- **Model download friction** -- first-time users must download ~150 MB (quantized) before it works
- **Accuracy ceiling** -- CLIP zero-shot tops out at ~70-80% on general photography categories. Niche subjects (specific bird species, architectural styles) need fine-tuning
- **No GPU acceleration on macOS by default** -- `ort` supports CoreML EP but requires additional build configuration. CPU-only is still fast enough for batch processing
- **BPE tokenizer is CLIP-specific** -- switching to SigLIP later would require a different tokenizer
- **Embedding storage grows linearly** -- 2 KB/asset is modest but adds up in very large catalogs (500K assets = 1 GB)
- **Compile-time complexity** -- ONNX Runtime linking can be finicky on some platforms, especially cross-compilation

---

## Configuration

```toml
[ai]
model = "standard"              # mobile | standard | accurate | ollama:<name>
labels = "labels.txt"           # custom label vocabulary file (one per line)
threshold = 0.25                # minimum confidence to suggest
model_dir = "~/.dam/models"     # where to cache downloaded models
prompt = "a photograph of a {}"  # text encoder prompt template ({} = tag name)
```

Default label vocabulary (~100 common photography categories) would be embedded in the binary, with `labels.txt` as an override.

---

## Edge Cases

- **No preview available**: fall back to loading the original file via `image` crate (slower but works)
- **Non-image assets** (video, audio, documents): skip with warning, or extract a frame for video (already done for preview generation via ffmpeg)
- **Offline volumes**: use existing preview JPEGs (800px or 2560px smart previews) -- no need for originals
- **Model not downloaded**: prompt user to run `dam auto-tag --download` or auto-download with confirmation
- **Label vocabulary empty**: use built-in default (~100 photography categories)
- **Threshold too low**: warn if suggesting >20 tags per asset (likely noise)

---

## Future Extension: Face Recognition

Auto-tagging with CLIP/SigLIP classifies images by *category* ("portrait", "group photo", "person") but cannot identify *who* is in a photo. Recognizing specific individuals (e.g., "Thomas", "Anna") requires a separate face recognition pipeline. This section documents how face recognition could be added as a follow-on feature, reusing the ONNX runtime and embedding infrastructure from the auto-tagging implementation.

### Why CLIP Cannot Do This

CLIP maps images and text into a shared semantic space. It understands *what* things look like conceptually, but has no memory of specific instances. It can match "a photo of a person wearing glasses" but cannot learn that a particular face belongs to "Thomas". Individual identity requires a dedicated face embedding model trained to produce unique, stable vectors per person.

### How Face Recognition Works

The pipeline used by Apple Photos, Google Photos, Immich, and Lightroom is well-established:

1. **Face detection** -- locate face bounding boxes in each image. Models: SCRFD, RetinaFace, or MediaPipe Face Detection. Outputs: coordinates + confidence for each face.
2. **Face embedding** -- encode each detected face crop into a 128-512 dimensional vector. Models: ArcFace, FaceNet, AdaFace. The vector is a compact "fingerprint" of that face's identity.
3. **Face clustering** -- group similar face embeddings across the entire library using unsupervised clustering (DBSCAN, HDBSCAN, or Chinese Whispers). Each cluster represents one person. No labels needed at this stage.
4. **User labeling** -- the user names clusters: "this cluster is Thomas", "this one is Anna". From then on, new photos containing a face close to a named cluster are automatically tagged with that person's name (e.g., `people/Thomas`).
5. **Incremental updates** -- when new photos are imported, faces are detected, embedded, and matched against existing clusters/named identities. New unknown faces form new unnamed clusters.

### ONNX Models Required

| Model | Purpose | ONNX Size | Notes |
|-------|---------|-----------|-------|
| SCRFD-2.5GF | Face detection | ~3 MB | Lightweight, accurate. Used by Immich and InsightFace |
| ArcFace-R50 | Face embedding (128-dim) | ~85 MB (FP32) / ~22 MB (INT8) | State-of-the-art face recognition. InsightFace ecosystem |
| ArcFace-R18 | Face embedding (smaller) | ~30 MB (FP32) / ~8 MB (INT8) | Faster, slightly less accurate |

Both models run on the same `ort` ONNX Runtime already required for auto-tagging. No additional runtime dependencies.

### Schema

```sql
CREATE TABLE IF NOT EXISTS faces (
    id INTEGER PRIMARY KEY,
    asset_id TEXT NOT NULL REFERENCES assets(id),
    variant_hash TEXT NOT NULL,
    bbox_x REAL NOT NULL,           -- normalized 0..1
    bbox_y REAL NOT NULL,
    bbox_w REAL NOT NULL,
    bbox_h REAL NOT NULL,
    confidence REAL NOT NULL,
    embedding BLOB NOT NULL,        -- 128 x f32 = 512 bytes per face
    cluster_id INTEGER,
    person_name TEXT,               -- user-assigned label, NULL until named
    UNIQUE(asset_id, bbox_x, bbox_y, bbox_w, bbox_h)
);
CREATE INDEX idx_faces_asset ON faces(asset_id);
CREATE INDEX idx_faces_cluster ON faces(cluster_id);
CREATE INDEX idx_faces_person ON faces(person_name);
```

Storage overhead: ~0.5 KB per detected face. For a library with 100,000 photos and an average of 0.5 faces per photo: ~25 MB.

### CLI Interface

```
dam faces [--asset <id>] [--volume <label>] [--query <QUERY>]
          [--cluster] [--apply] [--json] [--log] [--time]
dam faces label <CLUSTER_ID> <NAME>
dam faces list [--unnamed]
dam faces show <NAME>
```

- `dam faces` without `--cluster` detects and embeds faces (stores to DB). Report-only by default.
- `dam faces --cluster` runs clustering on all unlabeled face embeddings.
- `dam faces label 42 "Thomas"` names cluster 42. All faces in that cluster are tagged `people/Thomas` using the existing hierarchical tag system.
- `dam faces list` shows all known people with face counts. `--unnamed` shows unnamed clusters.
- `dam faces show "Thomas"` lists assets containing Thomas.

### Web UI

- **People page** (`/people`): grid of face cluster thumbnails, each showing a representative face crop, the assigned name (or "Unknown #42"), and a count. Click to browse all photos of that person. Inline rename.
- **Asset detail page**: detected faces highlighted with bounding boxes on hover. Each face shows its name or cluster ID. Click to assign/change name.
- **Browse filter**: `person:Thomas` search filter to find all photos containing Thomas.

### What CLIP Can Still Contribute

CLIP is useful for *attribute-based* person description alongside face identity:

- "a photo of a child" / "a photo of an elderly person"
- "a photo of someone wearing glasses"
- "a photo of someone in a wedding dress"
- "a group photo" / "a selfie"

These categorical tags complement face identity tags. Auto-tagging might produce `portrait, people/Thomas, outdoors` where `portrait` comes from CLIP and `people/Thomas` comes from face recognition.

### Implementation Effort

| Phase | Effort |
|-------|--------|
| Face detection (SCRFD via `ort`) | 1 day |
| Face embedding (ArcFace via `ort`) | 1 day |
| Face clustering (DBSCAN in pure Rust) | 1 day |
| Schema + face store | 0.5 day |
| CLI commands | 1 day |
| Web UI (people page, detail overlays, browse filter) | 2 days |
| Tests | 1 day |
| **Total** | **~7-8 days** |

This assumes the ONNX runtime, model manager, and embedding infrastructure from auto-tagging are already in place. Without that foundation, add ~3 days for shared infrastructure.

### Recommended Approach

Build face recognition as a **Phase 2** after auto-tagging ships:

1. **Phase 1**: Auto-tagging with CLIP/SigLIP (this proposal). Proves out the ONNX runtime integration, model manager, embedding store, and the `--features ai` build flag.
2. **Phase 2**: Face recognition. Adds SCRFD + ArcFace models to the existing model manager. Reuses the same `ort` session management. New `faces` table alongside the existing `embeddings` table. Face identity tags flow through the existing tag system and XMP write-back.

The total additional disk cost for face recognition models is modest: ~25 MB (INT8) on top of the ~85-340 MB already required for auto-tagging.

---

## Alternative: Start with Option C, Migrate to A

A lower-effort first step (2-3 days) would be to shell out to Ollama's API (`POST http://localhost:11434/api/generate` with a vision model) for tag suggestions. This avoids all the ONNX/model complexity but requires Ollama installed. Could serve as a prototype to validate the UX before investing in the embedded approach.

---

## References

### ONNX Runtime & Rust Integration
- [pykeio/ort -- Rust ONNX Runtime bindings](https://github.com/pykeio/ort)
- [ort documentation](https://ort.pyke.io/)
- [ONNX Runtime shared library size discussion](https://github.com/microsoft/onnxruntime/issues/6160)

### CLIP Models
- [OpenAI CLIP](https://github.com/openai/CLIP)
- [OpenCLIP (LAION)](https://github.com/mlfoundations/open_clip)
- [CLIP ViT-B/32 ONNX models (sayantan47)](https://huggingface.co/sayantan47/clip-vit-b32-onnx)
- [Qdrant CLIP ViT-B/32 vision encoder](https://huggingface.co/Qdrant/clip-ViT-B-32-vision)
- [Qdrant CLIP ViT-B/32 text encoder](https://huggingface.co/Qdrant/clip-ViT-B-32-text)
- [onnx_clip -- lightweight CLIP without PyTorch](https://github.com/lakeraai/onnx_clip)
- [CLIP-ONNX -- 3x speedup library](https://github.com/Lednik7/CLIP-ONNX)
- [immich-app ONNX model collection](https://huggingface.co/immich-app)

### SigLIP Models
- [SigLIP paper](https://arxiv.org/pdf/2303.15343)
- [SigLIP 2 announcement](https://huggingface.co/blog/siglip2)
- [SigLIP vs CLIP memory comparison](https://github.com/mlfoundations/open_clip/discussions/872)
- [SigLIP vs CLIP detailed comparison](https://blog.ritwikraha.dev/choosing-between-siglip-and-clip-for-language-image-pretraining)
- [deepghs/siglip_onnx -- split ONNX exports](https://huggingface.co/deepghs/siglip_onnx)

### MobileCLIP & TinyCLIP
- [Apple MobileCLIP](https://github.com/apple/ml-mobileclip)
- [MobileCLIP paper](https://arxiv.org/html/2311.17049v2)
- [MobileCLIP2 paper](https://arxiv.org/html/2508.20691v1)
- [TinyCLIP (Microsoft)](https://github.com/wkcn/TinyCLIP)

### Vision-Language Models (Ollama)
- [Ollama vision models](https://ollama.com/search?c=vision)
- [Ollama structured outputs](https://ollama.com/blog/structured-outputs)
- [PhotoPrism vision model comparison](https://docs.photoprism.app/developer-guide/vision/model-comparison/)
- [Moondream](https://moondream.ai/)

### Face Recognition
- [InsightFace (ArcFace, SCRFD)](https://github.com/deepinsight/insightface)
- [Immich facial recognition](https://immich.app/docs/features/facial-recognition/)

### Real-World Implementations
- [Immich smart search (uses CLIP)](https://docs.immich.app/features/searching/)
- [PhotoPrism AI classification](https://docs.photoprism.app/developer-guide/vision/)
