//! SigLIP vision-language models for zero-shot image classification.
//!
//! Only compiled when the `ai` feature is enabled.

use std::path::Path;

use anyhow::{Context, Result};
use ndarray::{Array2, Array4, Axis};
use ort::session::Session;
use ort::value::Tensor;
use serde::Serialize;
use tokenizers::Tokenizer;

/// Build an ONNX session with the best available execution provider.
///
/// When built with `--features ai-gpu`, registers CoreML on macOS (using
/// Neural Engine on Apple Silicon, Metal on Intel Macs). Falls back to
/// CPU silently if the provider is unavailable or unsupported by the model.
///
/// The `provider` parameter selects the strategy: "auto" picks the best
/// available, "cpu" forces CPU-only, "coreml" requests CoreML explicitly.
pub fn build_onnx_session(model_path: &Path, provider: &str, verbosity: crate::Verbosity) -> Result<Session> {
    let debug = verbosity.debug;
    let builder = Session::builder()
        .context("Failed to create ONNX session builder")?
        .with_intra_threads(4)
        .context("Failed to set intra threads")?;

    let provider_lower = provider.to_lowercase();
    let _use_gpu = provider_lower == "auto" || provider_lower == "coreml";

    #[cfg(all(feature = "ai-gpu", target_os = "macos"))]
    let builder = if _use_gpu {
        use ort::ep;
        let coreml = ep::CoreML::default().build();
        if debug {
            eprintln!("  [debug] registering CoreML execution provider");
        }
        builder
            .with_execution_providers([coreml])
            .context("Failed to register CoreML execution provider")?
    } else {
        builder
    };

    #[cfg(not(all(feature = "ai-gpu", target_os = "macos")))]
    if _use_gpu && provider_lower != "auto" && debug {
        eprintln!("  [debug] GPU provider '{provider}' requested but ai-gpu feature not enabled; using CPU");
    }

    let session = builder
        .commit_from_file(model_path)
        .with_context(|| format!("Failed to load ONNX model from {}", model_path.display()))?;

    if debug {
        #[cfg(all(feature = "ai-gpu", target_os = "macos"))]
        if _use_gpu {
            eprintln!("  [debug] session created (CoreML available)");
        }
    }

    Ok(session)
}

/// Model specification — defines all parameters for a SigLIP model variant.
#[derive(Debug, Clone)]
pub struct ModelSpec {
    pub id: &'static str,
    pub display_name: &'static str,
    pub hf_repo: &'static str,
    pub embedding_dim: usize,
    pub image_size: usize,
    /// Stored as log(scale) — must be exponentiated before use:
    ///   logit = exp(logit_scale) * dot_product + logit_bias
    pub logit_scale: f32,
    pub logit_bias: f32,
    pub max_text_len: usize,
    pub pad_token_id: u32,
}

/// All known model variants.
pub const MODEL_SPECS: &[ModelSpec] = &[
    ModelSpec {
        id: "siglip-vit-b16-256",
        display_name: "SigLIP ViT-B/16-256",
        hf_repo: "Xenova/siglip-base-patch16-256",
        embedding_dim: 768,
        image_size: 256,
        logit_scale: 4.7129,
        logit_bias: -12.9283,
        max_text_len: 64,
        pad_token_id: 1,
    },
    ModelSpec {
        id: "siglip-vit-l16-256",
        display_name: "SigLIP ViT-L/16-256",
        hf_repo: "Xenova/siglip-large-patch16-256",
        embedding_dim: 1024,
        image_size: 256,
        logit_scale: 4.7007,
        logit_bias: -12.6546,
        max_text_len: 64,
        pad_token_id: 1,
    },
    ModelSpec {
        // SigLIP 2 multilingual base — supports German, French, Spanish, Italian,
        // Japanese, Chinese, and many other languages via Gemma SentencePiece
        // tokenizer (vocab 256k). Drop-in replacement for siglip-vit-b16-256 in
        // dimensions and resolution. Switching models requires re-embedding the
        // catalog: `maki embed --force`.
        id: "siglip2-base-256-multi",
        display_name: "SigLIP 2 Base 256 (multilingual)",
        hf_repo: "onnx-community/siglip2-base-patch16-256-ONNX",
        embedding_dim: 768,
        image_size: 256,
        // logit_scale / logit_bias are only used by auto-tag (multi-label
        // classification). The values from SigLIP 1 base are reasonable
        // approximations for similarity ranking but should be replaced with
        // the actual SigLIP 2 values if precise auto-tag confidence calibration
        // is needed.
        logit_scale: 4.7129,
        logit_bias: -12.9283,
        max_text_len: 64,
        // Gemma tokenizer uses pad_token_id = 0 (vs 1 for SigLIP 1's tokenizer)
        pad_token_id: 0,
    },
    ModelSpec {
        // SigLIP 2 multilingual large — same multilingual capabilities as the
        // base variant, but with a larger model (1024-dim embeddings) for
        // higher quality at the cost of more memory and slower inference.
        // Drop-in replacement for siglip-vit-l16-256 in dimensions.
        id: "siglip2-large-256-multi",
        display_name: "SigLIP 2 Large 256 (multilingual)",
        hf_repo: "onnx-community/siglip2-large-patch16-256-ONNX",
        embedding_dim: 1024,
        image_size: 256,
        // Reasonable approximations from SigLIP 1 large; refine if needed.
        logit_scale: 4.7007,
        logit_bias: -12.6546,
        max_text_len: 64,
        pad_token_id: 0,
    },
];

/// Default model ID.
pub const DEFAULT_MODEL_ID: &str = "siglip-vit-b16-256";

/// Look up a model spec by ID.
pub fn get_model_spec(id: &str) -> Option<&'static ModelSpec> {
    MODEL_SPECS.iter().find(|s| s.id == id)
}

/// A suggested tag with confidence score.
#[derive(Debug, Clone, Serialize)]
pub struct AutoTagSuggestion {
    pub tag: String,
    pub confidence: f32,
}

/// Per-asset suggestion results.
#[derive(Debug, Clone, Serialize)]
pub struct AssetSuggestions {
    pub asset_id: String,
    pub suggested_tags: Vec<AutoTagSuggestion>,
    pub applied: bool,
}

/// Overall auto-tag operation result.
#[derive(Debug, Clone, Serialize)]
pub struct AutoTagResult {
    pub assets_processed: usize,
    pub assets_skipped: usize,
    pub tags_suggested: usize,
    pub tags_applied: usize,
    pub errors: Vec<String>,
    pub dry_run: bool,
    pub suggestions: Vec<AssetSuggestions>,
}

/// Callback status for per-asset progress reporting.
pub enum AutoTagStatus {
    Suggested(Vec<AutoTagSuggestion>),
    Applied(Vec<AutoTagSuggestion>),
    Skipped(String),
    Error(String),
}

/// Cached ONNX output metadata (name, number of dimensions).
#[allow(dead_code)]
struct OutputInfo {
    name: String,
    ndim: Option<usize>,
}

/// SigLIP vision-language model wrapper.
pub struct SigLipModel {
    vision: Session,
    text: Session,
    tokenizer: Tokenizer,
    verbosity: crate::Verbosity,
    vision_outputs: Vec<OutputInfo>,
    text_outputs: Vec<OutputInfo>,
    spec: &'static ModelSpec,
}

impl SigLipModel {
    /// Load ONNX sessions and tokenizer from the model directory.
    pub fn load(model_dir: &Path, model_id: &str) -> Result<Self> {
        Self::load_with_debug(model_dir, model_id, crate::Verbosity::quiet())
    }

    /// Load ONNX sessions with debug logging and execution provider selection.
    pub fn load_with_provider(model_dir: &Path, model_id: &str, verbosity: crate::Verbosity, provider: &str) -> Result<Self> {
        Self::load_internal(model_dir, model_id, verbosity, provider)
    }

    /// Load ONNX sessions with debug logging enabled (CPU provider).
    pub fn load_with_debug(model_dir: &Path, model_id: &str, verbosity: crate::Verbosity) -> Result<Self> {
        Self::load_internal(model_dir, model_id, verbosity, "auto")
    }

    fn load_internal(model_dir: &Path, model_id: &str, verbosity: crate::Verbosity, provider: &str) -> Result<Self> {
        let debug = verbosity.debug;
        let spec = get_model_spec(model_id)
            .ok_or_else(|| anyhow::anyhow!("unknown model: {model_id}"))?;
        let vision_path = model_dir.join("onnx").join("vision_model_quantized.onnx");
        let text_path = model_dir.join("onnx").join("text_model_quantized.onnx");
        let tokenizer_path = model_dir.join("tokenizer.json");

        // Fall back to non-quantized if quantized not found
        let vision_path = if vision_path.exists() {
            vision_path
        } else {
            let fp32 = model_dir.join("onnx").join("vision_model.onnx");
            if !fp32.exists() {
                anyhow::bail!(
                    "Vision model not found. Expected {} or {}",
                    vision_path.display(),
                    fp32.display()
                );
            }
            fp32
        };

        let text_path = if text_path.exists() {
            text_path
        } else {
            let fp32 = model_dir.join("onnx").join("text_model.onnx");
            if !fp32.exists() {
                anyhow::bail!(
                    "Text model not found. Expected {} or {}",
                    text_path.display(),
                    fp32.display()
                );
            }
            fp32
        };

        if !tokenizer_path.exists() {
            anyhow::bail!("tokenizer not found at {}", tokenizer_path.display());
        }

        let vision = build_onnx_session(&vision_path, provider, verbosity)?;
        let text = build_onnx_session(&text_path, provider, verbosity)?;

        // Cache output metadata (names + dimensionality from dtype)
        let vision_outputs: Vec<OutputInfo> = vision.outputs().iter().map(|o| {
            let ndim = match o.dtype() {
                ort::value::ValueType::Tensor { shape, .. } => Some(shape.len()),
                _ => None,
            };
            OutputInfo { name: o.name().to_string(), ndim }
        }).collect();

        let text_outputs: Vec<OutputInfo> = text.outputs().iter().map(|o| {
            let ndim = match o.dtype() {
                ort::value::ValueType::Tensor { shape, .. } => Some(shape.len()),
                _ => None,
            };
            OutputInfo { name: o.name().to_string(), ndim }
        }).collect();

        if debug {
            eprintln!("  [debug] vision model outputs:");
            for (i, o) in vision.outputs().iter().enumerate() {
                eprintln!("    [{i}] '{}' {:?}", o.name(), o.dtype());
            }
            eprintln!("  [debug] text model outputs:");
            for (i, o) in text.outputs().iter().enumerate() {
                eprintln!("    [{i}] '{}' {:?}", o.name(), o.dtype());
            }
        }

        let tokenizer = Tokenizer::from_file(&tokenizer_path)
            .map_err(|e| anyhow::anyhow!("failed to load tokenizer: {e}"))?;

        Ok(Self {
            vision,
            text,
            tokenizer,
            verbosity,
            vision_outputs,
            text_outputs,
            spec,
        })
    }

    /// Return the model ID.
    pub fn model_id(&self) -> &'static str {
        self.spec.id
    }

    /// Return the embedding dimensionality.
    pub fn embedding_dim(&self) -> usize {
        self.spec.embedding_dim
    }

    /// Return the model spec.
    pub fn spec(&self) -> &'static ModelSpec {
        self.spec
    }

    /// Encode an image file into an embedding vector.
    pub fn encode_image(&mut self, image_path: &Path) -> Result<Vec<f32>> {
        let tensor = preprocess_image(image_path, self.spec.image_size)?;
        let input_value = Tensor::from_array(tensor)
            .context("Failed to create vision input tensor")?;
        let outputs = self.vision.run(
            ort::inputs!["pixel_values" => input_value],
        )?;

        let emb = extract_pooled_embedding(&outputs, &self.vision_outputs, "vision", self.verbosity.debug)?;
        Ok(l2_normalize(&emb))
    }

    /// Encode a batch of text strings into embedding vectors.
    pub fn encode_texts(&mut self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }

        let input_ids = tokenize_batch(&self.tokenizer, texts, self.spec.max_text_len, self.spec.pad_token_id)?;
        let input_value = Tensor::from_array(input_ids)
            .context("Failed to create text input tensor")?;
        let outputs = self.text.run(
            ort::inputs!["input_ids" => input_value],
        )?;

        let pooled = extract_pooled_embedding(&outputs, &self.text_outputs, "text", self.verbosity.debug)?;
        let shape_dim = pooled.len();
        let batch_size = texts.len();

        // If the pooled output is [batch, dim], split into per-item embeddings
        let dim = shape_dim / batch_size;
        let mut result = Vec::with_capacity(batch_size);
        for i in 0..batch_size {
            let start = i * dim;
            let end = start + dim;
            result.push(l2_normalize(&pooled[start..end]));
        }

        Ok(result)
    }

    /// Classify an image embedding against label embeddings using SigLIP sigmoid scoring.
    /// Returns suggestions above the threshold, sorted by confidence (descending).
    pub fn classify(
        &self,
        image_emb: &[f32],
        labels: &[String],
        label_embs: &[Vec<f32>],
        threshold: f32,
    ) -> Vec<AutoTagSuggestion> {
        self.classify_inner(image_emb, labels, label_embs, threshold, false)
    }

    /// Classify with optional debug output showing per-label scoring details.
    pub fn classify_debug(
        &self,
        image_emb: &[f32],
        labels: &[String],
        label_embs: &[Vec<f32>],
        threshold: f32,
    ) -> Vec<AutoTagSuggestion> {
        self.classify_inner(image_emb, labels, label_embs, threshold, true)
    }

    fn classify_inner(
        &self,
        image_emb: &[f32],
        labels: &[String],
        label_embs: &[Vec<f32>],
        threshold: f32,
        debug: bool,
    ) -> Vec<AutoTagSuggestion> {
        let logit_scale = self.spec.logit_scale;
        let logit_bias = self.spec.logit_bias;
        let scale = logit_scale.exp();

        if debug {
            eprintln!("  [debug] scoring: exp({logit_scale:.3}) = {scale:.1}, bias = {logit_bias:.3}, threshold = {threshold:.3}");
        }

        // Collect all scores for debug sorting
        let mut all_scores: Vec<(String, f32, f32, f32)> = Vec::new();

        let mut suggestions: Vec<AutoTagSuggestion> = labels
            .iter()
            .zip(label_embs.iter())
            .filter_map(|(label, label_emb)| {
                let dot: f32 = image_emb
                    .iter()
                    .zip(label_emb.iter())
                    .map(|(a, b)| a * b)
                    .sum();
                let logit = scale * dot + logit_bias;
                let confidence = sigmoid(logit);

                if debug {
                    all_scores.push((label.clone(), dot, logit, confidence));
                }

                if confidence >= threshold {
                    Some(AutoTagSuggestion {
                        tag: label.clone(),
                        confidence,
                    })
                } else {
                    None
                }
            })
            .collect();

        if debug {
            all_scores.sort_by(|a, b| b.3.partial_cmp(&a.3).unwrap());
            let top_n = all_scores.len().min(15);
            eprintln!("  [debug] top {top_n} label scores:");
            for (label, dot, logit, conf) in &all_scores[..top_n] {
                let marker = if *conf >= threshold { ">>>" } else { "   " };
                eprintln!("  {marker} {label:30} dot={dot:+.4}  logit={logit:+.2}  sigmoid={conf:.4}");
            }
        }

        suggestions.sort_by(|a, b| b.confidence.partial_cmp(&a.confidence).unwrap());
        suggestions
    }
}

/// Extract the pooled embedding from ONNX model outputs.
///
/// ONNX SigLIP encoder models typically have two outputs:
///   [0] `last_hidden_state` — 3D tensor [batch, seq_len, hidden_dim]
///   [1] `pooler_output`     — 2D tensor [batch, hidden_dim]
///
/// We need the 2D pooled output for cross-modal alignment. Using the 3D
/// `last_hidden_state` (flattened) produces random-noise dot products because
/// the embeddings are not in the shared vision-language space.
fn extract_pooled_embedding(
    outputs: &ort::session::SessionOutputs,
    output_meta: &[OutputInfo],
    model_name: &str,
    debug: bool,
) -> Result<Vec<f32>> {
    let num_outputs = output_meta.len();

    // Debug: log all output shapes
    if debug {
        for i in 0..num_outputs {
            if let Ok(tensor) = outputs[i].try_extract_array::<f32>() {
                eprintln!("  [debug] {model_name} output[{i}] '{}' shape={:?}", output_meta[i].name, tensor.shape());
            }
        }
    }

    // First pass: look for a 2D output (the pooled embedding)
    for i in 0..num_outputs {
        let tensor = outputs[i]
            .try_extract_array::<f32>()
            .with_context(|| format!("Failed to extract {model_name} output[{i}] '{}'", output_meta[i].name))?;
        let shape = tensor.shape();

        if shape.len() == 2 {
            // [batch, dim] — this is the pooled embedding
            let emb: Vec<f32> = tensor.iter().copied().collect();
            if debug {
                eprintln!("  [debug] using {model_name} output[{i}] '{}' as pooled embedding (dim={})", output_meta[i].name, shape[1]);
            }
            return Ok(emb);
        }
    }

    // No 2D output found — all outputs are 3D (last_hidden_state only).
    // Fall back: take first token (CLS) from the first 3D output.
    for i in 0..num_outputs {
        let tensor = outputs[i]
            .try_extract_array::<f32>()
            .with_context(|| format!("Failed to extract {model_name} output[{i}]"))?;
        let shape = tensor.shape();
        if shape.len() == 3 {
            let dim = shape[2];
            let emb: Vec<f32> = tensor
                .index_axis(Axis(0), 0)  // batch 0
                .index_axis(Axis(0), 0)  // token 0 (CLS)
                .iter()
                .copied()
                .collect();
            eprintln!(
                "Warning: {model_name} model has no pooled output; using CLS token from '{}' (dim={dim}). Results may be degraded.",
                output_meta[i].name
            );
            return Ok(emb);
        }
    }

    anyhow::bail!(
        "No suitable embedding output found in {model_name} model. Outputs: {}",
        output_meta.iter().map(|o| o.name.as_str()).collect::<Vec<_>>().join(", ")
    )
}

/// Preprocess an image for SigLIP: resize to NxN, normalize to [-1, 1].
fn preprocess_image(path: &Path, image_size: usize) -> Result<Array4<f32>> {
    // Remove memory limit — medium format 16-bit TIFFs can exceed 600 MB when decoded,
    // but we immediately resize to a small square so peak memory is brief.
    let mut reader = image::ImageReader::open(path)
        .with_context(|| format!("Failed to open image: {}", path.display()))?;
    reader.no_limits();
    let img = reader.decode()
        .with_context(|| format!("Failed to decode image: {}", path.display()))?;

    let resized = img.resize_exact(
        image_size as u32,
        image_size as u32,
        image::imageops::FilterType::CatmullRom,
    );

    let rgb = resized.to_rgb8();
    let mut tensor = Array4::<f32>::zeros((1, 3, image_size, image_size));

    for y in 0..image_size {
        for x in 0..image_size {
            let pixel = rgb.get_pixel(x as u32, y as u32);
            for c in 0..3 {
                tensor[[0, c, y, x]] = (pixel[c] as f32 / 255.0 - 0.5) / 0.5;
            }
        }
    }

    Ok(tensor)
}

/// Tokenize a batch of texts, padding to max_text_len.
fn tokenize_batch(tokenizer: &Tokenizer, texts: &[String], max_text_len: usize, pad_token_id: u32) -> Result<Array2<i64>> {
    let batch_size = texts.len();
    let mut input_ids = Array2::<i64>::from_elem((batch_size, max_text_len), pad_token_id as i64);

    for (i, text) in texts.iter().enumerate() {
        let encoding = tokenizer
            .encode(text.as_str(), true)
            .map_err(|e| anyhow::anyhow!("tokenization failed: {e}"))?;

        let ids = encoding.get_ids();
        let len = ids.len().min(max_text_len);
        for j in 0..len {
            input_ids[[i, j]] = ids[j] as i64;
        }
    }

    Ok(input_ids)
}

/// L2-normalize a vector.
fn l2_normalize(v: &[f32]) -> Vec<f32> {
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm < 1e-12 {
        return v.to_vec();
    }
    v.iter().map(|x| x / norm).collect()
}

/// Sigmoid function.
fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}

/// Cosine similarity between two vectors.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a < 1e-12 || norm_b < 1e-12 {
        return 0.0;
    }
    dot / (norm_a * norm_b)
}

/// Default photography labels for zero-shot classification.
/// Each label is a short category name; the prompt template wraps it.
pub const DEFAULT_LABELS: &[&str] = &[
    // Scene types
    "landscape",
    "portrait",
    "street photography",
    "architecture",
    "cityscape",
    "seascape",
    "aerial view",
    "panorama",
    "interior",
    "still life",
    "macro",
    "wildlife",
    "underwater",
    "astrophotography",
    "night photography",
    // Nature
    "mountain",
    "forest",
    "beach",
    "ocean",
    "river",
    "lake",
    "waterfall",
    "desert",
    "field",
    "garden",
    "flowers",
    "trees",
    "sky",
    "clouds",
    "sunset",
    "sunrise",
    "fog",
    "snow",
    "rain",
    // People
    "person",
    "group of people",
    "child",
    "family",
    "couple",
    "crowd",
    "self-portrait",
    // Animals
    "dog",
    "cat",
    "bird",
    "horse",
    "fish",
    "insect",
    "butterfly",
    // Urban
    "building",
    "bridge",
    "road",
    "car",
    "bicycle",
    "train",
    "airplane",
    "boat",
    "skyscraper",
    // Food & Objects
    "food",
    "drink",
    "book",
    "clothing",
    "jewelry",
    "furniture",
    // Events
    "wedding",
    "concert",
    "festival",
    "sports",
    "celebration",
    "ceremony",
    // Artistic
    "black and white",
    "long exposure",
    "silhouette",
    "reflection",
    "shadow",
    "abstract",
    "minimalist",
    "texture",
    "pattern",
    "bokeh",
    "motion blur",
    // Seasonal
    "spring",
    "summer",
    "autumn",
    "winter",
    // Travel
    "travel",
    "landmark",
    "monument",
    "ruins",
    "market",
    "museum",
    "church",
    "temple",
    // Technical
    "document",
    "screenshot",
    "infographic",
    "product photography",
];

/// Apply the prompt template to a label (e.g. "a photograph of {}" + "sunset" → "a photograph of sunset").
pub fn apply_prompt_template(template: &str, label: &str) -> String {
    template.replace("{}", label)
}

/// Load labels from a text file (one per line, # comments, blank lines ignored).
pub fn load_labels_from_file(path: &Path) -> Result<Vec<String>> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read labels file: {}", path.display()))?;

    let labels: Vec<String> = content
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| l.to_string())
        .collect();

    if labels.is_empty() {
        anyhow::bail!("labels file is empty: {}", path.display());
    }

    Ok(labels)
}

/// Check if a file extension is a supported image format for SigLIP processing.
pub fn is_supported_image(ext: &str) -> bool {
    matches!(
        ext.to_lowercase().as_str(),
        "jpg" | "jpeg" | "png" | "gif" | "bmp" | "tiff" | "tif" | "webp"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn l2_normalize_unit_vector() {
        let v = vec![1.0, 0.0, 0.0];
        let n = l2_normalize(&v);
        assert!((n[0] - 1.0).abs() < 1e-6);
        assert!(n[1].abs() < 1e-6);
        assert!(n[2].abs() < 1e-6);
    }

    #[test]
    fn l2_normalize_scales_correctly() {
        let v = vec![3.0, 4.0];
        let n = l2_normalize(&v);
        assert!((n[0] - 0.6).abs() < 1e-6);
        assert!((n[1] - 0.8).abs() < 1e-6);
    }

    #[test]
    fn l2_normalize_zero_vector() {
        let v = vec![0.0, 0.0, 0.0];
        let n = l2_normalize(&v);
        assert_eq!(n, v);
    }

    #[test]
    fn sigmoid_zero() {
        assert!((sigmoid(0.0) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn sigmoid_large_positive() {
        assert!((sigmoid(10.0) - 1.0).abs() < 1e-4);
    }

    #[test]
    fn sigmoid_large_negative() {
        assert!(sigmoid(-10.0) < 1e-4);
    }

    #[test]
    fn cosine_similarity_identical() {
        let a = vec![1.0, 2.0, 3.0];
        let sim = cosine_similarity(&a, &a);
        assert!((sim - 1.0).abs() < 1e-6);
    }

    #[test]
    fn cosine_similarity_orthogonal() {
        let a = vec![1.0, 0.0];
        let b = vec![0.0, 1.0];
        let sim = cosine_similarity(&a, &b);
        assert!(sim.abs() < 1e-6);
    }

    #[test]
    fn cosine_similarity_opposite() {
        let a = vec![1.0, 0.0];
        let b = vec![-1.0, 0.0];
        let sim = cosine_similarity(&a, &b);
        assert!((sim + 1.0).abs() < 1e-6);
    }

    #[test]
    fn prompt_template_substitution() {
        assert_eq!(
            apply_prompt_template("a photograph of {}", "sunset"),
            "a photograph of sunset"
        );
    }

    #[test]
    fn prompt_template_no_placeholder() {
        assert_eq!(apply_prompt_template("hello world", "sunset"), "hello world");
    }

    #[test]
    fn default_labels_count() {
        assert!(DEFAULT_LABELS.len() >= 90, "Expected at least 90 default labels, got {}", DEFAULT_LABELS.len());
    }

    #[test]
    fn default_labels_no_duplicates() {
        let mut seen = std::collections::HashSet::new();
        for label in DEFAULT_LABELS {
            assert!(seen.insert(label), "Duplicate label: {label}");
        }
    }

    #[test]
    fn is_supported_image_common_formats() {
        assert!(is_supported_image("jpg"));
        assert!(is_supported_image("JPEG"));
        assert!(is_supported_image("png"));
        assert!(is_supported_image("webp"));
    }

    #[test]
    fn is_supported_image_raw_not_supported() {
        assert!(!is_supported_image("nef"));
        assert!(!is_supported_image("cr2"));
        assert!(!is_supported_image("arw"));
    }

    #[test]
    fn is_supported_image_non_image() {
        assert!(!is_supported_image("mp4"));
        assert!(!is_supported_image("mp3"));
        assert!(!is_supported_image("pdf"));
    }

    fn default_spec() -> &'static ModelSpec {
        get_model_spec(DEFAULT_MODEL_ID).unwrap()
    }

    #[test]
    fn classify_empty_labels() {
        let spec = default_spec();
        let image_emb = l2_normalize(&vec![1.0; spec.embedding_dim]);
        let labels: Vec<String> = Vec::new();
        let label_embs: Vec<Vec<f32>> = Vec::new();

        let suggestions = score_and_filter(spec, &image_emb, &labels, &label_embs, 0.25);
        assert!(suggestions.is_empty());
    }

    #[test]
    fn classify_threshold_filters() {
        let spec = default_spec();
        let image_emb = l2_normalize(&vec![1.0; spec.embedding_dim]);

        let similar_emb = l2_normalize(&vec![1.0; spec.embedding_dim]);
        let mut different = vec![0.0f32; spec.embedding_dim];
        different[0] = 1.0;
        let different_emb = l2_normalize(&different);

        let labels = vec!["similar".to_string(), "different".to_string()];
        let label_embs = vec![similar_emb, different_emb];

        let all = score_and_filter(spec, &image_emb, &labels, &label_embs, 0.0);
        assert_eq!(all.len(), 2);
        assert!(
            all[0].confidence > all[1].confidence,
            "Expected 'similar' to score higher than 'different'"
        );
        assert_eq!(all[0].tag, "similar");
        assert!(all[0].confidence > 0.99, "Expected high confidence for identical vectors, got {}", all[0].confidence);

        let some = score_and_filter(spec, &image_emb, &labels, &label_embs, 0.5);
        assert_eq!(some.len(), 1, "Expected only 'similar' at threshold 0.5");
        assert_eq!(some[0].tag, "similar");
    }

    #[test]
    fn model_spec_registry() {
        assert!(MODEL_SPECS.len() >= 2);
        assert!(get_model_spec("siglip-vit-b16-256").is_some());
        assert!(get_model_spec("siglip-vit-l16-256").is_some());
        assert!(get_model_spec("nonexistent").is_none());

        let b16 = get_model_spec("siglip-vit-b16-256").unwrap();
        assert_eq!(b16.embedding_dim, 768);
        let l16 = get_model_spec("siglip-vit-l16-256").unwrap();
        assert_eq!(l16.embedding_dim, 1024);
    }

    /// Standalone scoring function for testing (mirrors SigLipModel::classify_inner logic).
    fn score_and_filter(
        spec: &ModelSpec,
        image_emb: &[f32],
        labels: &[String],
        label_embs: &[Vec<f32>],
        threshold: f32,
    ) -> Vec<AutoTagSuggestion> {
        let scale = spec.logit_scale.exp();
        let mut results: Vec<AutoTagSuggestion> = labels
            .iter()
            .zip(label_embs.iter())
            .filter_map(|(label, label_emb)| {
                let dot: f32 = image_emb
                    .iter()
                    .zip(label_emb.iter())
                    .map(|(a, b)| a * b)
                    .sum();
                let logit = scale * dot + spec.logit_bias;
                let confidence = sigmoid(logit);
                if confidence >= threshold {
                    Some(AutoTagSuggestion {
                        tag: label.clone(),
                        confidence,
                    })
                } else {
                    None
                }
            })
            .collect();
        results.sort_by(|a, b| b.confidence.partial_cmp(&a.confidence).unwrap());
        results
    }

    #[test]
    fn load_labels_from_file_basic() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("labels.txt");
        std::fs::write(&path, "landscape\nportrait\n# comment\n\nocean\n").unwrap();
        let labels = load_labels_from_file(&path).unwrap();
        assert_eq!(labels, vec!["landscape", "portrait", "ocean"]);
    }

    #[test]
    fn load_labels_from_file_empty_errors() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("labels.txt");
        std::fs::write(&path, "# only comments\n\n").unwrap();
        assert!(load_labels_from_file(&path).is_err());
    }
}
