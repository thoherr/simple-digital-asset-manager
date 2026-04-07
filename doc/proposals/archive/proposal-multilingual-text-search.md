# Proposal: German (and other languages) for `text:` search

## Status

**Implemented in v4.3.12** — 2026-04-07

`siglip2-base-256-multi` model added to `MODEL_SPECS`. End-to-end test confirmed: model downloads, image embedding works, German text queries produce meaningful (distinct) embeddings. To use:

```toml
[ai]
model = "siglip2-base-256-multi"
```

Then `maki auto-tag --download` to fetch the model, `maki embed '' --force` to re-embed the catalog.

## Problem

The `text:` search filter (Pro feature) uses SigLIP image-text embeddings to find images by natural language description. The currently bundled models are English-only:

- `siglip-vit-b16-256` (Xenova/siglip-base-patch16-256)
- `siglip-vit-l16-256` (Xenova/siglip-large-patch16-256)

Both were trained on English captions and ship with an English-only tokenizer (~32k vocab). Typing `text:Sonnenuntergang über dem Meer` produces a meaningless embedding and irrelevant results. German-speaking users (and any non-English user) cannot use semantic search in their native language.

## Goal

Enable users to type queries in German (and ideally other languages) for `text:` search, with quality comparable to the English experience.

## Options

### Path A: Switch to a multilingual SigLIP model (recommended)

Google released **SigLIP 2** with explicit multilingual variants. The relevant ones on Hugging Face:

- `google/siglip2-base-patch16-256` — multilingual, ~109M params
- `google/siglip2-base-patch16-naflex` — flex resolution
- `google/siglip2-large-patch16-256`

These are trained on **multilingual web-scale image-text pairs (WebLI)** including German, French, Spanish, Italian, Japanese, Chinese, etc. The text tower uses a **multilingual SentencePiece tokenizer (~250k vocab)** instead of the English-only one.

**What needs to change in MAKI**:

1. **Add a new entry to `MODEL_SPECS`** in `src/ai.rs`:
   ```rust
   ModelSpec {
       id: "siglip2-base-256-multi",
       display_name: "SigLIP 2 Base 256 (multilingual)",
       hf_repo: "onnx-community/siglip2-base-patch16-256",  // ONNX-converted variant
       embedding_dim: 768,
       image_size: 256,
       logit_scale: ...,  // read from model config
       logit_bias: ...,
       max_text_len: 64,
       pad_token_id: ...,
   }
   ```

2. **ONNX export — verified available**. The repo `onnx-community/siglip2-base-patch16-256-ONNX` contains exactly the file layout MAKI needs:
   - `onnx/vision_model_quantized.onnx` (94.7 MB)
   - `onnx/text_model_quantized.onnx` (283 MB)
   - `tokenizer.json` (34.4 MB, multilingual SentencePiece, vocab 256k)
   - `config.json`, `preprocessor_config.json`, `tokenizer_config.json`

   Other variants in the same family are also available if higher quality or different resolution is desired:

   | Repo | Image | Notes |
   |------|-------|-------|
   | `siglip2-base-patch16-224-ONNX` | 224 | smaller, faster |
   | **`siglip2-base-patch16-256-ONNX`** | **256** | **drop-in replacement of current default** |
   | `siglip2-base-patch16-384-ONNX` | 384 | better detail, slower |
   | `siglip2-large-patch16-256-ONNX` | 256 | higher quality, 2-3× slower |
   | `siglip2-so400m-patch14-384-ONNX` | 384 | best quality, much slower |

3. **Confirm tokenizer compatibility**. The Rust `tokenizers` crate (already a dependency) supports SentencePiece via `tokenizer.json`. The multilingual model's tokenizer is bigger (~250k vocab vs ~32k) but the loading code doesn't care — it's just a JSON file. **No code changes needed** to the tokenizer path.

4. **Re-embed the catalog**. Image embeddings from the old model are not compatible with the new model (different vector spaces). Users must run `maki embed --force` to regenerate. On a 260k catalog this is hours of GPU/CPU time. The embedding store keys by `(asset_id, model_id)`, so old embeddings stay around but become unused — `maki cleanup` should reclaim them.

5. **Document the migration path**. Users would set `[ai] model = "siglip2-base-256-multi"` in `maki.toml` and run `maki embed --force`. The old model stays available; switching is a per-catalog choice.

**Effort**: Half a day if the ONNX export exists on HF, a full day if conversion is needed. Tokenizer behavior is automatic.

**Pros**:
- Genuine semantic understanding of German queries, including phrases like *"Wolken über den Bergen bei Sonnenuntergang"*
- Covers many other languages for free (French, Spanish, Italian, Japanese, etc.)
- No runtime translation overhead
- Single source of truth — the embedding model itself understands the language

**Cons**:
- One-time re-embedding cost on the user side
- Larger model file (multilingual tokenizer adds ~10-20MB)
- Quality on English queries may be marginally lower than the dedicated English model (typical multilingual tradeoff)

### Path B: Translate German queries to English at query time

Detect German input and translate to English before encoding:

- **Online**: call DeepL, Google Translate, or LibreTranslate. Adds latency, network dependency, privacy concerns.
- **Offline**: bundle a small translation model (e.g. `Helsinki-NLP/opus-mt-de-en`, ~75MB ONNX). Same complexity as adding any other ONNX model to the project. Faster than API but adds binary size.

**Pros**:
- No re-embedding needed
- Works with the existing English model

**Cons**:
- *Worse* quality than a true multilingual model — translation loses nuance ("Stimmung" → "mood" or "atmosphere"?)
- Requires language detection (`text:party` is valid English, but also German)
- Adds dependency (network or extra model)
- Doesn't scale to other languages without adding more translation models

**Recommendation**: Skip.

### Path C: Manual concept dictionary

Add an optional `[ai.synonyms]` section in `maki.toml`:
```toml
[ai.synonyms]
"Sonnenuntergang" = "sunset"
"Strand" = "beach"
"Hochzeit" = "wedding"
```

At query time, look up each word in the dictionary and substitute. Trivial to implement, lets users customize for their vocabulary, but it's a hack — doesn't compose, doesn't handle phrases, doesn't generalize.

**Useful only as a stopgap** or for power users who want exact control over a small vocabulary. Could ship alongside Path A as a fine-tuning escape hatch.

## Recommendation

**Go with Path A (SigLIP 2 multilingual)** as a new opt-in model alongside the existing English ones. It's the only solution that gives genuine semantic understanding of German queries. The migration cost is real (re-embedding) but it's a one-time operation, and users who don't care about non-English search can keep using the current model.

## Work breakdown (Path A)

1. Verify `onnx-community/siglip2-base-patch16-256` (or similar) exists and has quantized variants — **30 min**
2. Add `MODEL_SPECS` entry and test loading — **1 hour**
3. Test text encoding with a few German queries against an embedded test catalog — **1 hour**
4. Document in `[ai] model` config reference, mention re-embed requirement — **30 min**
5. Add an integration test using a tiny image set — **optional, 1 hour**

**Total**: ~4 hours of focused work, plus the catalog re-embed time on the user side.

## Verification notes (2026-04-07)

The simplified `config.json` in the ONNX repo doesn't include all the parameters MAKI's `ModelSpec` needs:

- `embedding_dim`: **768** (same as SigLIP 1 base — architecture is identical)
- `max_text_len`: **64** (SigLIP 2 default, same as SigLIP 1)
- `logit_scale` / `logit_bias`: not in the simplified config. These are only used by auto-tag (multi-label classification), **not** by `text:` search. For text search alone, placeholder values would work. To enable auto-tag with this model, dump these from the original `google/siglip2-base-patch16-256` PyTorch config with a one-time Python snippet.
- `pad_token_id`: needs verification from `tokenizer_config.json` (likely `1`, same as SigLIP 1).

## Open questions

- Should we make the multilingual model the default for new catalogs, or keep the English model as default and document multilingual as an opt-in?
- Do we want to expose a `text_lang:` hint in the query syntax, or just trust the multilingual model to figure it out from the query text?
- How do we communicate the "you must re-embed" requirement clearly enough that users don't try the new model and conclude it's broken?
- Should we offer multiple SigLIP 2 size variants in `MODEL_SPECS` upfront, or start with just `siglip2-base-256-multi` and add larger variants on demand?
