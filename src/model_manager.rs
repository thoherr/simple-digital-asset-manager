//! Download and cache management for SigLIP ONNX model files.
//!
//! Only compiled when the `ai` feature is enabled.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::ai::{get_model_spec, ModelSpec, MODEL_SPECS};

/// Model files to download (relative paths within the model directory).
const MODEL_FILES: &[&str] = &[
    "onnx/vision_model_quantized.onnx",
    "onnx/text_model_quantized.onnx",
    "tokenizer.json",
];

/// Build the HuggingFace download base URL for a model spec.
fn hf_base_url(spec: &ModelSpec) -> String {
    format!(
        "https://huggingface.co/{}/resolve/main",
        spec.hf_repo
    )
}

/// Manages downloading and caching of SigLIP model files.
pub struct ModelManager {
    model_dir: PathBuf,
    spec: &'static ModelSpec,
}

impl ModelManager {
    /// Create a new ModelManager for the given model.
    pub fn new(model_dir: &Path, model_id: &str) -> Result<Self> {
        let spec = get_model_spec(model_id)
            .ok_or_else(|| anyhow::anyhow!("unknown model: {model_id}"))?;
        Ok(Self {
            model_dir: model_dir.to_path_buf(),
            spec,
        })
    }

    /// Default model base directory: `~/.maki/models/`.
    pub fn default_model_base() -> Result<PathBuf> {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .context("Cannot determine home directory")?;
        Ok(PathBuf::from(home).join(".maki").join("models"))
    }

    /// Default model directory for a specific model: `~/.maki/models/<model_id>/`.
    pub fn default_model_dir(model_id: &str) -> Result<PathBuf> {
        Ok(Self::default_model_base()?.join(model_id))
    }

    /// Return the model spec.
    pub fn spec(&self) -> &'static ModelSpec {
        self.spec
    }

    /// Check if all required model files exist.
    pub fn model_exists(&self) -> bool {
        MODEL_FILES.iter().all(|rel_path| {
            self.model_dir.join(rel_path).exists()
        })
    }

    /// Return the model directory path.
    pub fn model_dir(&self) -> &Path {
        &self.model_dir
    }

    /// Ensure the model is downloaded. Returns the model directory path.
    pub fn ensure_model(
        &self,
        on_progress: impl Fn(&str, u64, u64),
    ) -> Result<PathBuf> {
        if self.model_exists() {
            return Ok(self.model_dir.clone());
        }
        self.download_model(on_progress)?;
        Ok(self.model_dir.clone())
    }

    /// Download model files from HuggingFace.
    pub fn download_model(
        &self,
        on_progress: impl Fn(&str, u64, u64),
    ) -> Result<()> {
        let base_url = hf_base_url(self.spec);
        let total_files = MODEL_FILES.len() as u64;

        for (i, rel_path) in MODEL_FILES.iter().enumerate() {
            let dest = self.model_dir.join(rel_path);
            let url = format!("{base_url}/{rel_path}");

            on_progress(rel_path, i as u64 + 1, total_files);

            if dest.exists() {
                continue;
            }

            // Create parent directories
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("Failed to create directory: {}", parent.display()))?;
            }

            // Download via curl (available on all platforms)
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
                anyhow::bail!("download failed for {rel_path} (curl exit code: {})", status);
            }

            // Rename to final path
            std::fs::rename(&tmp_dest, &dest)
                .with_context(|| format!("Failed to rename {} to {}", tmp_dest.display(), dest.display()))?;
        }

        Ok(())
    }

    /// Remove all cached model files.
    pub fn remove_model(&self) -> Result<()> {
        if self.model_dir.exists() {
            std::fs::remove_dir_all(&self.model_dir)
                .with_context(|| {
                    format!("Failed to remove model directory: {}", self.model_dir.display())
                })?;
        }
        Ok(())
    }

    /// List model files with sizes.
    pub fn list_files(&self) -> Vec<(String, u64)> {
        MODEL_FILES
            .iter()
            .filter_map(|rel_path| {
                let path = self.model_dir.join(rel_path);
                let size = std::fs::metadata(&path).ok()?.len();
                Some((rel_path.to_string(), size))
            })
            .collect()
    }

    /// Total size of cached model files in bytes.
    pub fn total_size(&self) -> u64 {
        self.list_files().iter().map(|(_, s)| s).sum()
    }

    /// List all known models with their download status and size.
    /// Returns `(spec, downloaded, size_bytes)` for each model.
    pub fn list_available_models(model_base: &Path) -> Vec<(&'static ModelSpec, bool, u64)> {
        MODEL_SPECS
            .iter()
            .map(|spec| {
                let dir = model_base.join(spec.id);
                let exists = MODEL_FILES.iter().all(|f| dir.join(f).exists());
                let size: u64 = if exists {
                    MODEL_FILES
                        .iter()
                        .filter_map(|f| std::fs::metadata(dir.join(f)).ok().map(|m| m.len()))
                        .sum()
                } else {
                    0
                };
                (spec, exists, size)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::DEFAULT_MODEL_ID;

    #[test]
    fn default_model_dir_under_home() {
        let dir = ModelManager::default_model_dir(DEFAULT_MODEL_ID).unwrap();
        let dir_str = dir.to_str().unwrap().replace('\\', "/");
        assert!(
            dir_str.contains(".maki/models/siglip-vit-b16-256"),
            "Expected .maki/models path, got: {}",
            dir.display()
        );
    }

    #[test]
    fn default_model_dir_large() {
        let dir = ModelManager::default_model_dir("siglip-vit-l16-256").unwrap();
        let dir_str = dir.to_str().unwrap().replace('\\', "/");
        assert!(
            dir_str.contains(".maki/models/siglip-vit-l16-256"),
            "Expected .maki/models path, got: {}",
            dir.display()
        );
    }

    #[test]
    fn model_exists_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = ModelManager::new(dir.path(), DEFAULT_MODEL_ID).unwrap();
        assert!(!mgr.model_exists());
    }

    #[test]
    fn new_unknown_model_errors() {
        let dir = tempfile::tempdir().unwrap();
        assert!(ModelManager::new(dir.path(), "nonexistent").is_err());
    }

    #[test]
    fn list_files_empty() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = ModelManager::new(dir.path(), DEFAULT_MODEL_ID).unwrap();
        assert!(mgr.list_files().is_empty());
    }

    #[test]
    fn total_size_empty() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = ModelManager::new(dir.path(), DEFAULT_MODEL_ID).unwrap();
        assert_eq!(mgr.total_size(), 0);
    }

    #[test]
    fn remove_model_nonexistent() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = ModelManager::new(&dir.path().join("nonexistent"), DEFAULT_MODEL_ID).unwrap();
        mgr.remove_model().unwrap();
    }

    #[test]
    fn list_available_models_empty() {
        let dir = tempfile::tempdir().unwrap();
        let models = ModelManager::list_available_models(dir.path());
        assert!(models.len() >= 2);
        assert!(models.iter().all(|(_, exists, _)| !exists));
    }

    #[test]
    fn hf_base_url_format() {
        let spec = get_model_spec(DEFAULT_MODEL_ID).unwrap();
        let url = hf_base_url(spec);
        assert!(url.contains("Xenova/siglip-base-patch16-256"));
        let spec_l = get_model_spec("siglip-vit-l16-256").unwrap();
        let url_l = hf_base_url(spec_l);
        assert!(url_l.contains("Xenova/siglip-large-patch16-256"));
    }

}
