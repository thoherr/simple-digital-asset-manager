//! Face detection and recognition pipeline using ONNX models.
//!
//! Uses YuNet for face detection (bounding boxes + landmarks) and
//! ArcFace for face recognition (512-dimensional embeddings).
//! Only compiled when the `ai` feature is enabled.

use std::path::Path;

use anyhow::{Context, Result};
use ndarray::{Array4, ArrayView2, Axis};
use ort::value::Tensor;

use crate::ai::build_onnx_session;

/// Face model specification.
#[derive(Debug, Clone)]
pub struct FaceModelSpec {
    pub id: &'static str,
    pub display_name: &'static str,
    pub hf_repo: &'static str,
    pub filename: &'static str,
    /// Size in bytes (approximate, for download progress).
    pub approx_size: u64,
}

/// Detection model: YuNet face detector.
pub const DETECTION_MODEL: FaceModelSpec = FaceModelSpec {
    id: "yunet-face-detection",
    display_name: "YuNet Face Detection",
    hf_repo: "opencv/face_detection_yunet",
    filename: "face_detection_yunet_2023mar.onnx",
    approx_size: 230_000,
};

/// Recognition model: ArcFace for face embeddings.
pub const RECOGNITION_MODEL: FaceModelSpec = FaceModelSpec {
    id: "arcface-resnet100",
    display_name: "ArcFace ResNet-100",
    hf_repo: "onnxmodelzoo/arcfaceresnet100-11-int8",
    filename: "arcfaceresnet100-11-int8.onnx",
    approx_size: 28_000_000,
};

/// All face model specs for listing/downloading.
pub const FACE_MODEL_SPECS: &[&FaceModelSpec] = &[&DETECTION_MODEL, &RECOGNITION_MODEL];

/// A detected face with bounding box and landmarks.
#[derive(Debug, Clone)]
pub struct DetectedFace {
    /// Bounding box in normalized coordinates [0, 1].
    pub bbox_x: f32,
    pub bbox_y: f32,
    pub bbox_w: f32,
    pub bbox_h: f32,
    /// Detection confidence score.
    pub confidence: f32,
    /// 5-point face landmarks (right eye, left eye, nose, right mouth, left mouth)
    /// in normalized coordinates. Each point is (x, y).
    pub landmarks: [(f32, f32); 5],
}

/// Face detection + recognition pipeline.
pub struct FaceDetector {
    detector: ort::session::Session,
    recognizer: ort::session::Session,
    verbosity: crate::Verbosity,
}

impl FaceDetector {
    /// Load the face detection and recognition ONNX models.
    pub fn load(model_dir: &Path, verbosity: crate::Verbosity) -> Result<Self> {
        Self::load_with_provider(model_dir, verbosity, "auto")
    }

    /// Load with a specific execution provider ("auto", "cpu", "coreml").
    pub fn load_with_provider(model_dir: &Path, verbosity: crate::Verbosity, provider: &str) -> Result<Self> {
        let debug = verbosity.debug;
        let detect_path = model_dir.join(&DETECTION_MODEL.filename);
        let recog_path = model_dir.join(&RECOGNITION_MODEL.filename);

        if !detect_path.exists() {
            anyhow::bail!(
                "Face detection model not found at {}. Run 'maki faces download' first.",
                detect_path.display()
            );
        }
        if !recog_path.exists() {
            anyhow::bail!(
                "Face recognition model not found at {}. Run 'maki faces download' first.",
                recog_path.display()
            );
        }

        let detector = build_onnx_session(&detect_path, provider, verbosity)?;
        let recognizer = build_onnx_session(&recog_path, provider, verbosity)?;

        if debug {
            eprintln!("  [debug] detection model inputs:");
            for (i, inp) in detector.inputs().iter().enumerate() {
                eprintln!("    [{i}] '{}' {:?}", inp.name(), inp.dtype());
            }
            eprintln!("  [debug] detection model outputs:");
            for (i, out) in detector.outputs().iter().enumerate() {
                eprintln!("    [{i}] '{}' {:?}", out.name(), out.dtype());
            }
            eprintln!("  [debug] recognition model inputs:");
            for (i, inp) in recognizer.inputs().iter().enumerate() {
                eprintln!("    [{i}] '{}' {:?}", inp.name(), inp.dtype());
            }
            eprintln!("  [debug] recognition model outputs:");
            for (i, out) in recognizer.outputs().iter().enumerate() {
                eprintln!("    [{i}] '{}' {:?}", out.name(), out.dtype());
            }
        }

        Ok(Self {
            detector,
            recognizer,
            verbosity,
        })
    }

    /// Detect faces in an image. Returns detected faces with bounding boxes and landmarks.
    ///
    /// The image is resized to 640x640 for detection. Bounding boxes and landmarks
    /// are returned in normalized [0, 1] coordinates relative to the original image.
    pub fn detect_faces(
        &mut self,
        image_path: &Path,
        min_confidence: f32,
    ) -> Result<Vec<DetectedFace>> {
        let img = image::open(image_path)
            .with_context(|| format!("Failed to open image: {}", image_path.display()))?;

        let orig_w = img.width() as f32;
        let orig_h = img.height() as f32;

        // YuNet expects specific input sizes; 640x640 is the common default
        let input_w: u32 = 640;
        let input_h: u32 = 640;

        let resized = img.resize_exact(input_w, input_h, image::imageops::FilterType::Triangle);
        let rgb = resized.to_rgb8();

        // Build NCHW tensor normalized to [0, 1]
        let mut tensor =
            Array4::<f32>::zeros((1, 3, input_h as usize, input_w as usize));
        for y in 0..input_h as usize {
            for x in 0..input_w as usize {
                let pixel = rgb.get_pixel(x as u32, y as u32);
                for c in 0..3 {
                    tensor[[0, c, y, x]] = pixel[c] as f32;
                }
            }
        }

        let input_value = Tensor::from_array(tensor).context("Failed to create input tensor")?;

        // Get detector metadata before mutable borrow
        let input_name = self.detector.inputs().first()
            .map(|i| i.name().to_string())
            .unwrap_or_else(|| "input".to_string());
        let num_outputs = self.detector.outputs().len();
        let output_names: Vec<String> = self.detector.outputs().iter()
            .map(|o| o.name().to_string().to_lowercase())
            .collect();

        let outputs = self.detector.run(
            ort::inputs![input_name.as_str() => input_value],
        ).context("Face detection inference failed")?;

        // Parse detection outputs
        let faces = parse_detections(
            &outputs, num_outputs, &output_names,
            input_w as f32, input_h as f32, orig_w, orig_h,
            min_confidence, self.verbosity.debug,
        )?;

        if self.verbosity.debug {
            eprintln!(
                "  [debug] detected {} faces (min_confidence={min_confidence:.2})",
                faces.len()
            );
        }

        Ok(faces)
    }

    /// Extract a face embedding using ArcFace.
    ///
    /// Crops and aligns the face from the source image using the 5-point landmarks,
    /// then runs through the ArcFace recognition model to produce a 512-dim embedding.
    pub fn embed_face(
        &mut self,
        image_path: &Path,
        face: &DetectedFace,
    ) -> Result<Vec<f32>> {
        let img = image::open(image_path)
            .with_context(|| format!("Failed to open image: {}", image_path.display()))?;

        let w = img.width() as f32;
        let h = img.height() as f32;

        // Crop the face region with some padding
        let pad = 0.2; // 20% padding around the face bbox
        let crop_x = ((face.bbox_x - face.bbox_w * pad) * w).max(0.0) as u32;
        let crop_y = ((face.bbox_y - face.bbox_h * pad) * h).max(0.0) as u32;
        let crop_w = ((face.bbox_w * (1.0 + 2.0 * pad)) * w).min(w - crop_x as f32) as u32;
        let crop_h = ((face.bbox_h * (1.0 + 2.0 * pad)) * h).min(h - crop_y as f32) as u32;

        let crop_w = crop_w.max(1);
        let crop_h = crop_h.max(1);

        let cropped = img.crop_imm(crop_x, crop_y, crop_w, crop_h);

        // ArcFace expects 112x112 input
        let resized = cropped.resize_exact(112, 112, image::imageops::FilterType::CatmullRom);
        let rgb = resized.to_rgb8();

        // Build NCHW tensor, normalized to [0, 1] range (standard ArcFace preprocessing)
        let mut tensor = Array4::<f32>::zeros((1, 3, 112, 112));
        for y in 0..112usize {
            for x in 0..112usize {
                let pixel = rgb.get_pixel(x as u32, y as u32);
                for c in 0..3 {
                    // ArcFace standard normalization: (pixel - 127.5) / 127.5
                    tensor[[0, c, y, x]] = (pixel[c] as f32 - 127.5) / 127.5;
                }
            }
        }

        let input_value =
            Tensor::from_array(tensor).context("Failed to create recognition input tensor")?;

        // Find input name
        let input_name = self.recognizer.inputs().first()
            .map(|i| i.name().to_string())
            .unwrap_or_else(|| "input".to_string());

        let outputs = self.recognizer.run(
            ort::inputs![input_name.as_str() => input_value],
        ).context("Face recognition inference failed")?;

        // Extract embedding from first output
        let embedding_tensor = outputs[0]
            .try_extract_array::<f32>()
            .context("Failed to extract recognition embedding")?;

        if self.verbosity.debug {
            eprintln!(
                "  [debug] recognition output shape={:?}",
                embedding_tensor.shape()
            );
        }

        let emb: Vec<f32> = embedding_tensor.iter().copied().collect();

        // L2 normalize
        Ok(l2_normalize(&emb))
    }

    /// Detect faces and extract embeddings in one pass.
    pub fn detect_and_embed(
        &mut self,
        image_path: &Path,
        min_confidence: f32,
    ) -> Result<Vec<(DetectedFace, Vec<f32>)>> {
        let faces = self.detect_faces(image_path, min_confidence)?;

        let mut results = Vec::with_capacity(faces.len());
        for face in faces {
            match self.embed_face(image_path, &face) {
                Ok(embedding) => results.push((face, embedding)),
                Err(e) => {
                    if self.verbosity.debug {
                        eprintln!(
                            "  [debug] failed to embed face at ({:.2}, {:.2}): {e:#}",
                            face.bbox_x, face.bbox_y
                        );
                    }
                }
            }
        }

        Ok(results)
    }

    /// Check if face models are downloaded.
    pub fn models_exist(model_dir: &Path) -> bool {
        model_dir.join(&DETECTION_MODEL.filename).exists()
            && model_dir.join(&RECOGNITION_MODEL.filename).exists()
    }

    /// Download face models from HuggingFace via curl.
    pub fn download_models(
        model_dir: &Path,
        on_progress: impl Fn(&str, u64, u64),
    ) -> Result<()> {
        std::fs::create_dir_all(model_dir)
            .with_context(|| format!("Failed to create model directory: {}", model_dir.display()))?;

        let specs: Vec<&FaceModelSpec> = FACE_MODEL_SPECS.to_vec();
        let total = specs.len() as u64;

        for (i, spec) in specs.iter().enumerate() {
            let dest = model_dir.join(spec.filename);
            let url = format!(
                "https://huggingface.co/{}/resolve/main/{}",
                spec.hf_repo, spec.filename
            );

            on_progress(spec.display_name, i as u64 + 1, total);

            if dest.exists() {
                continue;
            }

            let tmp_dest = dest.with_extension("download");
            let status = std::process::Command::new("curl")
                .args([
                    "-fSL",
                    "--progress-bar",
                    "-o",
                    tmp_dest.to_str().unwrap(),
                    &url,
                ])
                .status()
                .context("Failed to run curl. Is curl installed?")?;

            if !status.success() {
                std::fs::remove_file(&tmp_dest).ok();
                anyhow::bail!(
                    "Download failed for {} (curl exit code: {})",
                    spec.display_name,
                    status
                );
            }

            std::fs::rename(&tmp_dest, &dest).with_context(|| {
                format!(
                    "Failed to rename {} to {}",
                    tmp_dest.display(),
                    dest.display()
                )
            })?;
        }

        Ok(())
    }
}

/// Parse detection model outputs into DetectedFace structs (free function to avoid borrow issues).
fn parse_detections(
    outputs: &ort::session::SessionOutputs,
    num_outputs: usize,
    output_names: &[String],
    input_w: f32,
    input_h: f32,
    _orig_w: f32,
    _orig_h: f32,
    min_confidence: f32,
    debug: bool,
) -> Result<Vec<DetectedFace>> {
    if debug {
        for i in 0..num_outputs {
            if let Ok(tensor) = outputs[i].try_extract_array::<f32>() {
                eprintln!(
                    "  [debug] detection output[{i}] '{}' shape={:?}",
                    output_names.get(i).map(|s| s.as_str()).unwrap_or("?"),
                    tensor.shape()
                );
            }
        }
    }

    // Try to extract a 2D [N, 15] output (YuNet 2023 format)
    for i in 0..num_outputs {
        if let Ok(tensor) = outputs[i].try_extract_array::<f32>() {
            let shape = tensor.shape();
            if shape.len() == 2 && shape[1] >= 15 {
                return parse_yunet_2d(
                    tensor.view().into_dimensionality::<ndarray::Ix2>()
                        .context("Failed to convert to 2D array")?,
                    input_w, input_h, min_confidence,
                );
            }
            if shape.len() == 3 && shape[2] >= 15 {
                let squeezed = tensor.index_axis(Axis(0), 0);
                return parse_yunet_2d(
                    squeezed.into_dimensionality::<ndarray::Ix2>()
                        .context("Failed to squeeze to 2D array")?,
                    input_w, input_h, min_confidence,
                );
            }
        }
    }

    // Try multi-stride YuNet format: cls_8/16/32, obj_8/16/32, bbox_8/16/32, kps_8/16/32
    if num_outputs == 12 {
        let has_strides = output_names.iter().any(|n| n.contains("cls_")) &&
            output_names.iter().any(|n| n.contains("bbox_"));
        if has_strides {
            return parse_yunet_multi_stride(outputs, output_names, input_w, input_h, min_confidence, debug);
        }
    }

    // Fallback: try separate loc/conf/iou outputs
    if num_outputs >= 2 {
        return parse_multi_output_detections(outputs, num_outputs, output_names, input_w, input_h, min_confidence);
    }

    anyhow::bail!("Unrecognized detection model output format ({num_outputs} outputs)");
}

/// Parse YuNet 2023 format: [N, 15] tensor.
fn parse_yunet_2d(
    detections: ArrayView2<f32>,
    input_w: f32,
    input_h: f32,
    min_confidence: f32,
) -> Result<Vec<DetectedFace>> {
    let mut faces = Vec::new();

    for row in detections.axis_iter(Axis(0)) {
        let score = row[14];
        if score < min_confidence {
            continue;
        }

        let x = row[0] / input_w;
        let y = row[1] / input_h;
        let w = row[2] / input_w;
        let h = row[3] / input_h;

        let landmarks = [
            (row[4] / input_w, row[5] / input_h),
            (row[6] / input_w, row[7] / input_h),
            (row[8] / input_w, row[9] / input_h),
            (row[10] / input_w, row[11] / input_h),
            (row[12] / input_w, row[13] / input_h),
        ];

        faces.push(DetectedFace {
            bbox_x: x.max(0.0).min(1.0),
            bbox_y: y.max(0.0).min(1.0),
            bbox_w: w.max(0.0).min(1.0),
            bbox_h: h.max(0.0).min(1.0),
            confidence: score,
            landmarks,
        });
    }

    faces.sort_by(|a, b| b.confidence.partial_cmp(&a.confidence).unwrap());
    Ok(faces)
}

/// Parse YuNet multi-stride format with separate cls/obj/bbox/kps outputs at strides 8, 16, 32.
///
/// Decoding follows the OpenCV implementation:
///   score = sqrt(clamp(cls, 0, 1) * clamp(obj, 0, 1))
///   cx = (col + bbox[0]) * stride,  cy = (row + bbox[1]) * stride
///   w  = exp(bbox[2]) * stride,     h  = exp(bbox[3]) * stride
///   landmark_x = (col + kps[k*2]) * stride, landmark_y = (row + kps[k*2+1]) * stride
fn parse_yunet_multi_stride(
    outputs: &ort::session::SessionOutputs,
    output_names: &[String],
    input_w: f32,
    input_h: f32,
    min_confidence: f32,
    debug: bool,
) -> Result<Vec<DetectedFace>> {
    let strides: &[u32] = &[8, 16, 32];

    // Build name→index map
    let name_idx: std::collections::HashMap<&str, usize> = output_names.iter().enumerate()
        .map(|(i, n)| (n.as_str(), i))
        .collect();

    let mut candidates: Vec<(f32, f32, f32, f32, f32, [(f32, f32); 5], f32)> = Vec::new();

    for (si, &stride) in strides.iter().enumerate() {
        let cols = (input_w as u32 / stride) as usize;
        let rows = (input_h as u32 / stride) as usize;

        let suffix = format!("_{stride}");
        let cls_name = format!("cls{suffix}");
        let obj_name = format!("obj{suffix}");
        let bbox_name = format!("bbox{suffix}");
        let kps_name = format!("kps{suffix}");

        let cls_idx = name_idx.get(cls_name.as_str()).copied()
            .unwrap_or(si);
        let obj_idx = name_idx.get(obj_name.as_str()).copied()
            .unwrap_or(si + strides.len());
        let bbox_idx = name_idx.get(bbox_name.as_str()).copied()
            .unwrap_or(si + strides.len() * 2);
        let kps_idx = name_idx.get(kps_name.as_str()).copied()
            .unwrap_or(si + strides.len() * 3);

        let cls_tensor = outputs[cls_idx].try_extract_array::<f32>()
            .context("Failed to extract cls tensor")?;
        let obj_tensor = outputs[obj_idx].try_extract_array::<f32>()
            .context("Failed to extract obj tensor")?;
        let bbox_tensor = outputs[bbox_idx].try_extract_array::<f32>()
            .context("Failed to extract bbox tensor")?;
        let kps_tensor = outputs[kps_idx].try_extract_array::<f32>()
            .context("Failed to extract kps tensor")?;

        let cls_flat: Vec<f32> = cls_tensor.iter().copied().collect();
        let obj_flat: Vec<f32> = obj_tensor.iter().copied().collect();
        let bbox_flat: Vec<f32> = bbox_tensor.iter().copied().collect();
        let kps_flat: Vec<f32> = kps_tensor.iter().copied().collect();

        let n = cols * rows;
        for r in 0..rows {
            for c in 0..cols {
                let idx = r * cols + c;
                if idx >= n { break; }

                let cls_score = cls_flat.get(idx).copied().unwrap_or(0.0).clamp(0.0, 1.0);
                let obj_score = obj_flat.get(idx).copied().unwrap_or(0.0).clamp(0.0, 1.0);
                let score = (cls_score * obj_score).sqrt();

                if score < min_confidence {
                    continue;
                }

                let b = idx * 4;
                let cx = (c as f32 + bbox_flat.get(b).copied().unwrap_or(0.0)) * stride as f32;
                let cy = (r as f32 + bbox_flat.get(b + 1).copied().unwrap_or(0.0)) * stride as f32;
                let w = bbox_flat.get(b + 2).copied().unwrap_or(0.0).exp() * stride as f32;
                let h = bbox_flat.get(b + 3).copied().unwrap_or(0.0).exp() * stride as f32;

                // Convert center to top-left
                let x = cx - w * 0.5;
                let y = cy - h * 0.5;

                let k = idx * 10;
                let mut landmarks = [(0.0f32, 0.0f32); 5];
                for lm in 0..5 {
                    let lx = (c as f32 + kps_flat.get(k + lm * 2).copied().unwrap_or(0.0)) * stride as f32;
                    let ly = (r as f32 + kps_flat.get(k + lm * 2 + 1).copied().unwrap_or(0.0)) * stride as f32;
                    landmarks[lm] = (lx / input_w, ly / input_h);
                }

                candidates.push((x, y, w, h, score, landmarks, score));
            }
        }
    }

    if debug {
        eprintln!("  [debug] multi-stride: {} candidates before NMS", candidates.len());
    }

    // Simple greedy NMS
    candidates.sort_by(|a, b| b.4.partial_cmp(&a.4).unwrap());
    let nms_threshold = 0.3f32;
    let mut keep = vec![true; candidates.len()];

    for i in 0..candidates.len() {
        if !keep[i] { continue; }
        for j in (i + 1)..candidates.len() {
            if !keep[j] { continue; }
            let iou = compute_iou(
                candidates[i].0, candidates[i].1, candidates[i].2, candidates[i].3,
                candidates[j].0, candidates[j].1, candidates[j].2, candidates[j].3,
            );
            if iou > nms_threshold {
                keep[j] = false;
            }
        }
    }

    let faces: Vec<DetectedFace> = candidates.iter().zip(keep.iter())
        .filter(|(_, &k)| k)
        .map(|(&(x, y, w, h, score, landmarks, _), _)| {
            DetectedFace {
                bbox_x: (x / input_w).clamp(0.0, 1.0),
                bbox_y: (y / input_h).clamp(0.0, 1.0),
                bbox_w: (w / input_w).clamp(0.0, 1.0),
                bbox_h: (h / input_h).clamp(0.0, 1.0),
                confidence: score,
                landmarks,
            }
        })
        .collect();

    if debug {
        eprintln!("  [debug] multi-stride: {} faces after NMS", faces.len());
    }

    Ok(faces)
}

/// Compute IoU (intersection over union) between two boxes in (x, y, w, h) format.
fn compute_iou(x1: f32, y1: f32, w1: f32, h1: f32, x2: f32, y2: f32, w2: f32, h2: f32) -> f32 {
    let inter_x = x1.max(x2);
    let inter_y = y1.max(y2);
    let inter_r = (x1 + w1).min(x2 + w2);
    let inter_b = (y1 + h1).min(y2 + h2);

    let inter_w = (inter_r - inter_x).max(0.0);
    let inter_h = (inter_b - inter_y).max(0.0);
    let inter_area = inter_w * inter_h;

    let area1 = w1 * h1;
    let area2 = w2 * h2;
    let union_area = area1 + area2 - inter_area;

    if union_area <= 0.0 { 0.0 } else { inter_area / union_area }
}

/// Fallback parser for multi-output detection models (loc + conf + iou).
fn parse_multi_output_detections(
    outputs: &ort::session::SessionOutputs,
    num_outputs: usize,
    output_names: &[String],
    input_w: f32,
    input_h: f32,
    min_confidence: f32,
) -> Result<Vec<DetectedFace>> {
    let mut loc_data: Option<Vec<f32>> = None;
    let mut conf_data: Option<Vec<f32>> = None;
    let mut loc_cols = 0usize;

    for i in 0..num_outputs {
        if let Ok(tensor) = outputs[i].try_extract_array::<f32>() {
            let shape = tensor.shape();
            let name = output_names.get(i).map(|s| s.as_str()).unwrap_or("");

            if (name.contains("loc") || (shape.len() >= 2 && shape[shape.len() - 1] >= 14))
                && loc_data.is_none()
            {
                loc_cols = *shape.last().unwrap_or(&0);
                loc_data = Some(tensor.iter().copied().collect());
            } else if (name.contains("conf") || name.contains("score")
                || (shape.len() >= 2 && shape[shape.len() - 1] == 2))
                && conf_data.is_none()
            {
                conf_data = Some(tensor.iter().copied().collect());
            }
        }
    }

    let loc = loc_data.context("No location tensor found in detection output")?;
    let conf = conf_data.context("No confidence tensor found in detection output")?;

    if loc_cols < 4 {
        anyhow::bail!("Location tensor has too few columns: {loc_cols}");
    }

    let n_detections = loc.len() / loc_cols;
    let conf_cols = conf.len() / n_detections;
    let mut faces = Vec::new();

    for i in 0..n_detections {
        let score = if conf_cols >= 2 {
            conf[i * conf_cols + 1]
        } else {
            conf[i * conf_cols]
        };

        if score < min_confidence {
            continue;
        }

        let base = i * loc_cols;
        let x = loc[base] / input_w;
        let y = loc[base + 1] / input_h;
        let w = loc[base + 2] / input_w;
        let h = loc[base + 3] / input_h;

        let landmarks = if loc_cols >= 14 {
            [
                (loc[base + 4] / input_w, loc[base + 5] / input_h),
                (loc[base + 6] / input_w, loc[base + 7] / input_h),
                (loc[base + 8] / input_w, loc[base + 9] / input_h),
                (loc[base + 10] / input_w, loc[base + 11] / input_h),
                (loc[base + 12] / input_w, loc[base + 13] / input_h),
            ]
        } else {
            let cx = x + w / 2.0;
            let cy = y + h / 2.0;
            [
                (cx - w * 0.15, cy - h * 0.15),
                (cx + w * 0.15, cy - h * 0.15),
                (cx, cy),
                (cx - w * 0.12, cy + h * 0.2),
                (cx + w * 0.12, cy + h * 0.2),
            ]
        };

        faces.push(DetectedFace {
            bbox_x: x.max(0.0).min(1.0),
            bbox_y: y.max(0.0).min(1.0),
            bbox_w: w.max(0.0).min(1.0),
            bbox_h: h.max(0.0).min(1.0),
            confidence: score,
            landmarks,
        });
    }

    faces.sort_by(|a, b| b.confidence.partial_cmp(&a.confidence).unwrap());
    Ok(faces)
}

/// Crop a face from an image and save as a 150×150 JPEG thumbnail.
///
/// Saves to `faces/<face_id[0..2]>/<face_id>.jpg` under `catalog_root`.
/// Returns the path on success; errors are non-fatal.
pub fn save_face_crop(
    image_path: &Path,
    face: &DetectedFace,
    face_id: &str,
    catalog_root: &Path,
) -> Result<std::path::PathBuf> {
    let img = image::open(image_path)
        .with_context(|| format!("Failed to open image for face crop: {}", image_path.display()))?;

    let w = img.width() as f32;
    let h = img.height() as f32;

    // Crop with 20% padding (same as embed_face)
    let pad = 0.2;
    let crop_x = ((face.bbox_x - face.bbox_w * pad) * w).max(0.0) as u32;
    let crop_y = ((face.bbox_y - face.bbox_h * pad) * h).max(0.0) as u32;
    let crop_w = ((face.bbox_w * (1.0 + 2.0 * pad)) * w).min(w - crop_x as f32) as u32;
    let crop_h = ((face.bbox_h * (1.0 + 2.0 * pad)) * h).min(h - crop_y as f32) as u32;

    let crop_w = crop_w.max(1);
    let crop_h = crop_h.max(1);

    let cropped = img.crop_imm(crop_x, crop_y, crop_w, crop_h);
    let resized = cropped.resize_exact(150, 150, image::imageops::FilterType::CatmullRom);

    let prefix = &face_id[..2.min(face_id.len())];
    let dir = catalog_root.join("faces").join(prefix);
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("Failed to create faces dir: {}", dir.display()))?;

    let path = dir.join(format!("{face_id}.jpg"));
    resized
        .save(&path)
        .with_context(|| format!("Failed to save face crop: {}", path.display()))?;

    Ok(path)
}

/// Check if a face crop thumbnail exists.
pub fn face_crop_exists(face_id: &str, catalog_root: &Path) -> bool {
    let prefix = &face_id[..2.min(face_id.len())];
    catalog_root
        .join("faces")
        .join(prefix)
        .join(format!("{face_id}.jpg"))
        .exists()
}

/// L2-normalize a vector.
fn l2_normalize(v: &[f32]) -> Vec<f32> {
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm < 1e-12 {
        return v.to_vec();
    }
    v.iter().map(|x| x / norm).collect()
}

/// Default face model directory: `~/.maki/models/faces/`.
pub fn default_face_model_dir() -> Result<std::path::PathBuf> {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .context("Cannot determine home directory")?;
    Ok(std::path::PathBuf::from(home)
        .join(".maki")
        .join("models")
        .join("faces"))
}

/// Resolve face model directory from AiConfig.
pub fn resolve_face_model_dir(config: &crate::config::AiConfig) -> std::path::PathBuf {
    let model_dir_str = &config.model_dir;
    let model_base = if model_dir_str.starts_with("~/") {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_else(|_| ".".to_string());
        std::path::PathBuf::from(home).join(&model_dir_str[2..])
    } else {
        std::path::PathBuf::from(model_dir_str)
    };
    model_base.join("faces")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn l2_normalize_unit_vector() {
        let v = vec![1.0, 0.0, 0.0];
        let normed = l2_normalize(&v);
        assert!((normed[0] - 1.0).abs() < 1e-6);
        assert!((normed[1]).abs() < 1e-6);
    }

    #[test]
    fn l2_normalize_scales() {
        let v = vec![3.0, 4.0];
        let normed = l2_normalize(&v);
        assert!((normed[0] - 0.6).abs() < 1e-6);
        assert!((normed[1] - 0.8).abs() < 1e-6);
    }

    #[test]
    fn l2_normalize_zero_vector() {
        let v = vec![0.0; 512];
        let normed = l2_normalize(&v);
        assert_eq!(normed, v);
    }

    #[test]
    fn default_face_model_dir_path() {
        let dir = default_face_model_dir().unwrap();
        assert!(
            dir.to_str().unwrap().contains(".maki/models/faces"),
            "Expected .maki/models/faces path, got: {}",
            dir.display()
        );
    }

    #[test]
    fn models_exist_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!FaceDetector::models_exist(dir.path()));
    }

    #[test]
    fn face_model_specs_complete() {
        assert_eq!(FACE_MODEL_SPECS.len(), 2);
        assert_eq!(FACE_MODEL_SPECS[0].id, "yunet-face-detection");
        assert_eq!(FACE_MODEL_SPECS[1].id, "arcface-resnet100");
    }

    #[test]
    fn detected_face_struct() {
        let face = DetectedFace {
            bbox_x: 0.1,
            bbox_y: 0.2,
            bbox_w: 0.3,
            bbox_h: 0.4,
            confidence: 0.95,
            landmarks: [(0.15, 0.25), (0.25, 0.25), (0.2, 0.35), (0.15, 0.45), (0.25, 0.45)],
        };
        assert!((face.confidence - 0.95).abs() < 1e-6);
        assert_eq!(face.landmarks.len(), 5);
    }
}
