use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Preview output format.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PreviewFormat {
    Jpeg,
    Webp,
}

impl Default for PreviewFormat {
    fn default() -> Self {
        Self::Jpeg
    }
}

impl PreviewFormat {
    /// File extension for this format.
    pub fn extension(&self) -> &'static str {
        match self {
            PreviewFormat::Jpeg => "jpg",
            PreviewFormat::Webp => "webp",
        }
    }
}

/// Preview generation configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PreviewConfig {
    #[serde(default = "default_max_edge")]
    pub max_edge: u32,
    #[serde(default)]
    pub format: PreviewFormat,
    #[serde(default = "default_quality")]
    pub quality: u8,
    #[serde(default = "default_smart_max_edge")]
    pub smart_max_edge: u32,
    #[serde(default = "default_smart_quality")]
    pub smart_quality: u8,
    #[serde(default)]
    pub generate_on_demand: bool,
}

fn default_max_edge() -> u32 {
    800
}

fn default_quality() -> u8 {
    85
}

fn default_smart_max_edge() -> u32 {
    2560
}

fn default_smart_quality() -> u8 {
    85
}

impl Default for PreviewConfig {
    fn default() -> Self {
        Self {
            max_edge: 800,
            format: PreviewFormat::Jpeg,
            quality: 85,
            smart_max_edge: 2560,
            smart_quality: 85,
            generate_on_demand: false,
        }
    }
}

/// Web server configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ServeConfig {
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default = "default_bind")]
    pub bind: String,
    #[serde(default = "default_per_page")]
    pub per_page: u32,
    #[serde(default = "default_stroll_neighbors")]
    pub stroll_neighbors: u32,
    #[serde(default = "default_stroll_neighbors_max")]
    pub stroll_neighbors_max: u32,
    #[serde(default = "default_stroll_fanout")]
    pub stroll_fanout: u32,
    #[serde(default = "default_stroll_fanout_max")]
    pub stroll_fanout_max: u32,
    #[serde(default = "default_stroll_discover_pool")]
    pub stroll_discover_pool: u32,
}

fn default_port() -> u16 {
    8080
}

fn default_bind() -> String {
    "127.0.0.1".to_string()
}

fn default_per_page() -> u32 {
    60
}

fn default_stroll_neighbors() -> u32 {
    12
}

fn default_stroll_neighbors_max() -> u32 {
    25
}

fn default_stroll_fanout() -> u32 {
    5
}

fn default_stroll_fanout_max() -> u32 {
    10
}

fn default_stroll_discover_pool() -> u32 {
    80
}

impl Default for ServeConfig {
    fn default() -> Self {
        Self {
            port: 8080,
            bind: "127.0.0.1".to_string(),
            per_page: 60,
            stroll_neighbors: 12,
            stroll_neighbors_max: 25,
            stroll_fanout: 5,
            stroll_fanout_max: 10,
            stroll_discover_pool: 80,
        }
    }
}

/// Import behavior configuration.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ImportConfig {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub auto_tags: Vec<String>,
    #[serde(default)]
    pub smart_previews: bool,
    #[serde(default)]
    pub embeddings: bool,
    #[serde(default)]
    pub descriptions: bool,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub profiles: HashMap<String, ImportProfile>,
}

/// A named import profile that overrides `[import]` defaults.
/// Fields are optional — unset fields inherit from the base `[import]` config.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ImportProfile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exclude: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_tags: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub smart_previews: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embeddings: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub descriptions: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub include: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skip: Vec<String>,
}

impl ImportConfig {
    /// Resolve a profile by name, merging it with the base config.
    /// Profile values override base values; unset profile fields inherit from base.
    pub fn resolve_profile(&self, name: &str) -> Option<ImportConfig> {
        let profile = self.profiles.get(name)?;
        Some(ImportConfig {
            exclude: profile.exclude.clone().unwrap_or_else(|| self.exclude.clone()),
            auto_tags: profile.auto_tags.clone().unwrap_or_else(|| self.auto_tags.clone()),
            smart_previews: profile.smart_previews.unwrap_or(self.smart_previews),
            embeddings: profile.embeddings.unwrap_or(self.embeddings),
            descriptions: profile.descriptions.unwrap_or(self.descriptions),
            profiles: HashMap::new(),
        })
    }
}

fn is_default_preview(p: &PreviewConfig) -> bool {
    *p == PreviewConfig::default()
}

fn is_default_serve(s: &ServeConfig) -> bool {
    *s == ServeConfig::default()
}

fn is_default_import(i: &ImportConfig) -> bool {
    *i == ImportConfig::default()
}

/// Auto-group behavior configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GroupConfig {
    /// Regex pattern to identify session root directories.
    /// Auto-group uses this to determine which directory level is the "shoot"
    /// boundary — files below the matching directory can be grouped, files in
    /// different matching directories cannot.
    ///
    /// Default: `^\d{4}-\d{2}` (matches directories starting with YYYY-MM,
    /// e.g., 2024-10, 2024-10-05-wedding, 2025-05-09-event).
    ///
    /// Set to an empty string to fall back to parent-directory grouping.
    #[serde(default = "default_session_root_pattern")]
    pub session_root_pattern: String,
}

fn default_session_root_pattern() -> String {
    r"^\d{4}-\d{2}".to_string()
}

impl Default for GroupConfig {
    fn default() -> Self {
        Self {
            session_root_pattern: default_session_root_pattern(),
        }
    }
}

fn is_default_group(g: &GroupConfig) -> bool {
    *g == GroupConfig::default()
}

/// Dedup behavior configuration.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct DedupConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefer: Option<String>,
}

fn is_default_dedup(d: &DedupConfig) -> bool {
    *d == DedupConfig::default()
}

/// Verify behavior configuration.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct VerifyConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_age_days: Option<u64>,
}

fn is_default_verify(v: &VerifyConfig) -> bool {
    *v == VerifyConfig::default()
}

/// AI auto-tagging configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AiConfig {
    #[serde(default = "default_ai_model")]
    pub model: String,
    #[serde(default = "default_ai_threshold")]
    pub threshold: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub labels: Option<String>,
    #[serde(default = "default_ai_model_dir")]
    pub model_dir: String,
    #[serde(default = "default_ai_prompt")]
    pub prompt: String,
    #[serde(default = "default_face_cluster_threshold")]
    pub face_cluster_threshold: f32,
    #[serde(default = "default_face_min_confidence")]
    pub face_min_confidence: f32,
    /// Execution provider for ONNX inference: "auto", "cpu", "coreml".
    /// "auto" selects the best available provider for the platform.
    #[serde(default = "default_execution_provider")]
    pub execution_provider: String,
    /// Default result limit for `text:` search filter (default 50).
    #[serde(default = "default_text_limit")]
    pub text_limit: usize,
}

fn default_text_limit() -> usize {
    50
}

fn default_ai_model() -> String {
    "siglip-vit-b16-256".to_string()
}

fn default_ai_threshold() -> f32 {
    0.1
}

fn default_execution_provider() -> String {
    "auto".to_string()
}

fn default_ai_model_dir() -> String {
    "~/.maki/models".to_string()
}

fn default_ai_prompt() -> String {
    "a photograph of {}".to_string()
}

fn default_face_cluster_threshold() -> f32 {
    // Tuned for the aligned FP32 ArcFace pipeline. Intra-person cosine
    // similarity typically falls in 0.5–0.9; inter-person similarity is
    // negative-to-slightly-positive. 0.35 sits near the valley between
    // the two humps — slightly aggressive (merges borderline same-person
    // pairs) but reliably separates different people. Lower to 0.3 for
    // stricter same-person clusters with fewer splits; raise to 0.4+ for
    // cleaner but smaller clusters.
    0.35
}

fn default_face_min_confidence() -> f32 {
    0.7
}

impl Default for AiConfig {
    fn default() -> Self {
        Self {
            model: "siglip-vit-b16-256".to_string(),
            threshold: 0.1,
            labels: None,
            model_dir: "~/.maki/models".to_string(),
            prompt: "a photograph of {}".to_string(),
            face_cluster_threshold: 0.35,
            face_min_confidence: 0.7,
            execution_provider: "auto".to_string(),
            text_limit: 50,
        }
    }
}

fn is_default_ai(a: &AiConfig) -> bool {
    *a == AiConfig::default()
}

/// Per-model VLM parameter overrides.
///
/// All fields are optional — when absent, the global `[vlm]` value is used.
/// Configured under `[vlm.model_config."model-name"]` in `maki.toml`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct VlmModelConfig {
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub timeout: Option<u32>,
    pub max_image_edge: Option<u32>,
    pub num_ctx: Option<u32>,
    pub top_p: Option<f32>,
    pub top_k: Option<u32>,
    pub repeat_penalty: Option<f32>,
    pub prompt: Option<String>,
}

/// VLM (vision-language model) configuration for image descriptions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VlmConfig {
    /// VLM server endpoint (Ollama, LM Studio, vLLM, or any OpenAI-compatible API).
    #[serde(default = "default_vlm_endpoint")]
    pub endpoint: String,

    /// Default model name.
    #[serde(default = "default_vlm_model")]
    pub model: String,

    /// Maximum tokens in response.
    #[serde(default = "default_vlm_max_tokens")]
    pub max_tokens: u32,

    /// Custom prompt (overrides built-in).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,

    /// Request timeout in seconds.
    #[serde(default = "default_vlm_timeout")]
    pub timeout: u32,

    /// Default mode: "describe", "tags", "both".
    #[serde(default = "default_vlm_mode")]
    pub mode: String,

    /// Sampling temperature (0.0 = deterministic, 1.0+ = creative).
    #[serde(default = "default_vlm_temperature")]
    pub temperature: f32,

    /// Concurrent requests (for servers that handle parallelism).
    #[serde(default = "default_vlm_concurrency")]
    pub concurrency: u32,

    /// Maximum pixel size of the longest edge for images sent to VLM.
    /// Images larger than this are resized before encoding. 0 = no resizing.
    #[serde(default)]
    pub max_image_edge: u32,

    /// Context window size (Ollama `num_ctx`). 0 = use model default.
    #[serde(default)]
    pub num_ctx: u32,

    /// Nucleus sampling threshold. 0.0 = disabled (use temperature only).
    #[serde(default)]
    pub top_p: f32,

    /// Top-K sampling. 0 = disabled.
    #[serde(default)]
    pub top_k: u32,

    /// Repeat penalty. 0.0 = disabled.
    #[serde(default)]
    pub repeat_penalty: f32,

    /// Additional models available for selection in the web UI.
    /// The default `model` is always included as the first option.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<String>,

    /// Per-model parameter overrides.
    #[serde(default, skip_serializing_if = "std::collections::HashMap::is_empty")]
    pub model_config: std::collections::HashMap<String, VlmModelConfig>,
}

fn default_vlm_endpoint() -> String {
    "http://localhost:11434".to_string()
}

fn default_vlm_model() -> String {
    "qwen2.5vl:3b".to_string()
}

fn default_vlm_max_tokens() -> u32 {
    500
}

fn default_vlm_timeout() -> u32 {
    300
}

fn default_vlm_mode() -> String {
    "describe".to_string()
}

fn default_vlm_temperature() -> f32 {
    0.7
}

fn default_vlm_concurrency() -> u32 {
    1
}

impl Default for VlmConfig {
    fn default() -> Self {
        Self {
            endpoint: "http://localhost:11434".to_string(),
            model: "qwen2.5vl:3b".to_string(),
            max_tokens: 500,
            prompt: None,
            timeout: 300,
            mode: "describe".to_string(),
            temperature: 0.7,
            concurrency: 1,
            max_image_edge: 0,
            num_ctx: 0,
            top_p: 0.0,
            top_k: 0,
            repeat_penalty: 0.0,
            models: Vec::new(),
            model_config: std::collections::HashMap::new(),
        }
    }
}

impl VlmConfig {
    /// Build resolved parameters for a specific model, merging per-model overrides
    /// over global defaults.
    pub fn params_for_model(&self, model: &str) -> crate::vlm::VlmParams {
        let mc = self.model_config.get(model);
        crate::vlm::VlmParams {
            max_tokens: mc.and_then(|c| c.max_tokens).unwrap_or(self.max_tokens),
            timeout: mc.and_then(|c| c.timeout).unwrap_or(self.timeout),
            temperature: mc.and_then(|c| c.temperature).unwrap_or(self.temperature),
            max_image_edge: mc.and_then(|c| c.max_image_edge).unwrap_or(self.max_image_edge),
            num_ctx: mc.and_then(|c| c.num_ctx).unwrap_or(self.num_ctx),
            top_p: mc.and_then(|c| c.top_p).unwrap_or(self.top_p),
            top_k: mc.and_then(|c| c.top_k).unwrap_or(self.top_k),
            repeat_penalty: mc.and_then(|c| c.repeat_penalty).unwrap_or(self.repeat_penalty),
            prompt: mc.and_then(|c| c.prompt.clone()).or_else(|| self.prompt.clone()),
        }
    }

    /// Returns the full list of models for web UI selection.
    /// Default `model` is always first, followed by any extras from `models`.
    pub fn available_models(&self) -> Vec<String> {
        let mut result = vec![self.model.clone()];
        for m in &self.models {
            if !result.contains(m) {
                result.push(m.clone());
            }
        }
        result
    }
}

fn is_default_vlm(v: &VlmConfig) -> bool {
    *v == VlmConfig::default()
}

/// Contact sheet default configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContactSheetDefaults {
    #[serde(default = "default_cs_layout")]
    pub layout: String,
    #[serde(default = "default_cs_paper")]
    pub paper: String,
    #[serde(default = "default_cs_fields")]
    pub fields: String,
    #[serde(default = "default_cs_margin")]
    pub margin: f32,
    #[serde(default = "default_cs_quality")]
    pub quality: u8,
    #[serde(default = "default_cs_label_style")]
    pub label_style: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub copyright: Option<String>,
}

fn default_cs_layout() -> String { "standard".to_string() }
fn default_cs_paper() -> String { "a4".to_string() }
fn default_cs_fields() -> String { "filename,date,rating".to_string() }
fn default_cs_margin() -> f32 { 10.0 }
fn default_cs_quality() -> u8 { 90 }
fn default_cs_label_style() -> String { "border".to_string() }

impl Default for ContactSheetDefaults {
    fn default() -> Self {
        Self {
            layout: "standard".to_string(),
            paper: "a4".to_string(),
            fields: "filename,date,rating".to_string(),
            margin: 10.0,
            quality: 90,
            label_style: "border".to_string(),
            copyright: None,
        }
    }
}

fn is_default_contact_sheet(c: &ContactSheetDefaults) -> bool {
    *c == ContactSheetDefaults::default()
}

/// XMP writeback configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WritebackConfig {
    /// Enable XMP writeback. When false (default), MAKI will not modify
    /// recipe/XMP files on disk. Edits to rating, tags, description, and
    /// color label are stored in the catalog but not written to XMP until
    /// this is enabled.
    #[serde(default)]
    pub enabled: bool,
}

impl Default for WritebackConfig {
    fn default() -> Self {
        Self { enabled: false }
    }
}

fn is_default_writeback(w: &WritebackConfig) -> bool {
    *w == WritebackConfig::default()
}

/// Default CLI flags. These are OR'd with command-line flags —
/// setting `log = true` here is like always passing `--log`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct CliDefaults {
    /// Enable --log by default
    #[serde(default)]
    pub log: bool,
    /// Enable --time by default
    #[serde(default)]
    pub time: bool,
    /// Enable --verbose by default
    #[serde(default)]
    pub verbose: bool,
}

fn is_default_cli(c: &CliDefaults) -> bool {
    *c == CliDefaults::default()
}

/// Browse behavior configuration.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct BrowseConfig {
    /// Default search filter applied to browse/search/stroll views.
    /// Uses standard search syntax (e.g. "rating:1+", "-tag:rest").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_filter: Option<String>,
}

fn is_default_browse(b: &BrowseConfig) -> bool {
    *b == BrowseConfig::default()
}

/// Catalog configuration stored in maki.toml.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_volume: Option<Uuid>,
    #[serde(default, skip_serializing_if = "is_default_preview")]
    pub preview: PreviewConfig,
    #[serde(default, skip_serializing_if = "is_default_serve")]
    pub serve: ServeConfig,
    #[serde(default, skip_serializing_if = "is_default_import")]
    pub import: ImportConfig,
    #[serde(default, skip_serializing_if = "is_default_dedup")]
    pub dedup: DedupConfig,
    #[serde(default, skip_serializing_if = "is_default_verify")]
    pub verify: VerifyConfig,
    #[serde(default, skip_serializing_if = "is_default_ai")]
    pub ai: AiConfig,
    #[serde(default, skip_serializing_if = "is_default_contact_sheet")]
    pub contact_sheet: ContactSheetDefaults,
    #[serde(default, skip_serializing_if = "is_default_vlm")]
    pub vlm: VlmConfig,
    #[serde(default, skip_serializing_if = "is_default_browse")]
    pub browse: BrowseConfig,
    #[serde(default, skip_serializing_if = "is_default_writeback")]
    pub writeback: WritebackConfig,
    #[serde(default, skip_serializing_if = "is_default_cli")]
    pub cli: CliDefaults,
    #[serde(default, skip_serializing_if = "is_default_group")]
    pub group: GroupConfig,
}

impl Default for CatalogConfig {
    fn default() -> Self {
        Self {
            default_volume: None,
            preview: PreviewConfig::default(),
            serve: ServeConfig::default(),
            import: ImportConfig::default(),
            dedup: DedupConfig::default(),
            verify: VerifyConfig::default(),
            ai: AiConfig::default(),
            contact_sheet: ContactSheetDefaults::default(),
            vlm: VlmConfig::default(),
            browse: BrowseConfig::default(),
            writeback: WritebackConfig::default(),
            cli: CliDefaults::default(),
            group: GroupConfig::default(),
        }
    }
}

impl CatalogConfig {
    /// Load configuration from a maki.toml file (falls back to legacy dam.toml).
    pub fn load(catalog_root: &Path) -> Result<Self> {
        let path = catalog_root.join("maki.toml");
        let legacy_path = catalog_root.join("dam.toml");
        let config_path = if path.exists() {
            path
        } else if legacy_path.exists() {
            legacy_path
        } else {
            return Ok(Self::default());
        };
        let contents = std::fs::read_to_string(&config_path)?;
        let config: Self = toml::from_str(&contents)?;
        config.validate()?;
        Ok(config)
    }

    /// Save configuration to a maki.toml file.
    pub fn save(&self, catalog_root: &Path) -> Result<()> {
        let path = catalog_root.join("maki.toml");
        let contents = toml::to_string_pretty(self)?;
        std::fs::write(path, contents)?;
        Ok(())
    }

    /// Validate configuration values.
    pub fn validate(&self) -> Result<()> {
        if self.preview.max_edge == 0 {
            anyhow::bail!("preview.max_edge must be greater than 0");
        }
        if self.preview.quality == 0 || self.preview.quality > 100 {
            anyhow::bail!("preview.quality must be between 1 and 100");
        }
        if self.preview.smart_max_edge == 0 {
            anyhow::bail!("preview.smart_max_edge must be greater than 0");
        }
        if self.preview.smart_quality == 0 || self.preview.smart_quality > 100 {
            anyhow::bail!("preview.smart_quality must be between 1 and 100");
        }
        if self.serve.per_page == 0 || self.serve.per_page > 1000 {
            anyhow::bail!("serve.per_page must be between 1 and 1000");
        }
        if self.ai.threshold < 0.0 || self.ai.threshold > 1.0 {
            anyhow::bail!("ai.threshold must be between 0.0 and 1.0");
        }
        Ok(())
    }
}

/// Find the catalog root by looking for maki.toml (or legacy dam.toml) in current and parent directories.
pub fn find_catalog_root() -> Result<PathBuf> {
    let mut dir = std::env::current_dir()?;
    loop {
        if dir.join("maki.toml").exists() {
            return Ok(dir);
        }
        if dir.join("dam.toml").exists() {
            eprintln!("Note: found legacy dam.toml — consider renaming to maki.toml");
            return Ok(dir);
        }
        if !dir.pop() {
            anyhow::bail!("no maki catalog found. Run `maki init` to create one.");
        }
    }
}

/// Locate the catalog root and load its `maki.toml`.
///
/// Convenience helper for command handlers that need both the path and the
/// parsed config — almost every long-running command does. Returns
/// `(catalog_root, config)`.
pub fn load_config() -> Result<(PathBuf, CatalogConfig)> {
    let root = find_catalog_root()?;
    let config = CatalogConfig::load(&root)?;
    Ok((root, config))
}

/// Resolve a model directory by expanding a `~/`-prefixed path and joining
/// the model id. Used by both the CLI and the web AI routes.
///
/// `model_dir_root` is `[ai] model_dir` from `maki.toml` — typically
/// `~/.maki/models`. `model_id` is the SigLIP model name (e.g. `siglip2-large-patch16-512`).
/// Returns `model_dir_root/<model_id>` with `~/` expanded against `$HOME`
/// (or `%USERPROFILE%` on Windows). Falls back to `.` if neither is set;
/// this is rare enough on real systems that bubbling the error would just
/// add noise.
#[cfg(feature = "ai")]
pub fn resolve_model_dir(model_dir_root: &str, model_id: &str) -> PathBuf {
    let base = if let Some(rest) = model_dir_root.strip_prefix("~/") {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_else(|_| ".".to_string());
        PathBuf::from(home).join(rest)
    } else {
        PathBuf::from(model_dir_root)
    };
    base.join(model_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serialize_default_config() {
        let config = CatalogConfig::default();
        let toml_str = toml::to_string_pretty(&config).unwrap();
        // Default config should be empty (all fields skipped)
        assert!(toml_str.trim().is_empty(), "got: {toml_str}");
    }

    #[test]
    fn serialize_with_default_volume() {
        let uuid = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let config = CatalogConfig {
            default_volume: Some(uuid),
            ..Default::default()
        };
        let toml_str = toml::to_string_pretty(&config).unwrap();
        assert!(toml_str.contains("default_volume = \"550e8400-e29b-41d4-a716-446655440000\""));
    }

    #[test]
    fn parse_empty_config() {
        let config: CatalogConfig = toml::from_str("").unwrap();
        assert!(config.default_volume.is_none());
        assert_eq!(config.preview, PreviewConfig::default());
        assert_eq!(config.serve, ServeConfig::default());
        assert_eq!(config.import, ImportConfig::default());
    }

    #[test]
    fn parse_comment_only() {
        let config: CatalogConfig = toml::from_str("# maki catalog configuration\n").unwrap();
        assert!(config.default_volume.is_none());
    }

    #[test]
    fn parse_with_default_volume() {
        let input = "default_volume = \"550e8400-e29b-41d4-a716-446655440000\"\n";
        let config: CatalogConfig = toml::from_str(input).unwrap();
        assert_eq!(
            config.default_volume.unwrap().to_string(),
            "550e8400-e29b-41d4-a716-446655440000"
        );
    }

    #[test]
    fn parse_default_volume_only_backward_compat() {
        // Old maki.toml files that only had default_volume should still parse
        let input = "default_volume = \"550e8400-e29b-41d4-a716-446655440000\"\n";
        let config: CatalogConfig = toml::from_str(input).unwrap();
        assert!(config.default_volume.is_some());
        assert_eq!(config.preview, PreviewConfig::default());
    }

    #[test]
    fn round_trip() {
        let uuid = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let original = CatalogConfig {
            default_volume: Some(uuid),
            ..Default::default()
        };
        let toml_str = toml::to_string_pretty(&original).unwrap();
        let parsed: CatalogConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(original.default_volume, parsed.default_volume);
    }

    #[test]
    fn round_trip_none() {
        let original = CatalogConfig::default();
        let toml_str = toml::to_string_pretty(&original).unwrap();
        let parsed: CatalogConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(original.default_volume, parsed.default_volume);
    }

    #[test]
    fn parse_invalid_uuid_errors() {
        let input = "default_volume = \"not-a-uuid\"\n";
        assert!(toml::from_str::<CatalogConfig>(input).is_err());
    }

    #[test]
    fn parse_full_config() {
        let input = r#"
default_volume = "550e8400-e29b-41d4-a716-446655440000"

[preview]
max_edge = 1200
format = "webp"
quality = 90

[serve]
port = 9090
bind = "0.0.0.0"

[import]
exclude = ["Thumbs.db", "*.tmp", ".DS_Store"]
auto_tags = ["inbox", "unreviewed"]
"#;
        let config: CatalogConfig = toml::from_str(input).unwrap();
        assert!(config.default_volume.is_some());
        assert_eq!(config.preview.max_edge, 1200);
        assert_eq!(config.preview.format, PreviewFormat::Webp);
        assert_eq!(config.preview.quality, 90);
        assert_eq!(config.serve.port, 9090);
        assert_eq!(config.serve.bind, "0.0.0.0");
        assert_eq!(config.import.exclude, vec!["Thumbs.db", "*.tmp", ".DS_Store"]);
        assert_eq!(config.import.auto_tags, vec!["inbox", "unreviewed"]);
    }

    #[test]
    fn parse_partial_config_missing_sections() {
        let input = r#"
[preview]
max_edge = 1000
"#;
        let config: CatalogConfig = toml::from_str(input).unwrap();
        assert!(config.default_volume.is_none());
        assert_eq!(config.preview.max_edge, 1000);
        assert_eq!(config.preview.format, PreviewFormat::Jpeg); // default
        assert_eq!(config.preview.quality, 85); // default
        assert_eq!(config.serve, ServeConfig::default());
        assert_eq!(config.import, ImportConfig::default());
    }

    #[test]
    fn full_round_trip() {
        let original = CatalogConfig {
            default_volume: None,
            preview: PreviewConfig {
                max_edge: 1200,
                format: PreviewFormat::Webp,
                quality: 90,
                smart_max_edge: 3000,
                smart_quality: 92,
                ..Default::default()
            },
            serve: ServeConfig {
                port: 9090,
                bind: "0.0.0.0".to_string(),
                ..Default::default()
            },
            import: ImportConfig {
                exclude: vec!["*.tmp".to_string()],
                auto_tags: vec!["test".to_string()],
                ..Default::default()
            },
            dedup: DedupConfig::default(),
            verify: VerifyConfig::default(),
            ai: AiConfig::default(),
            contact_sheet: ContactSheetDefaults::default(),
            vlm: VlmConfig::default(),
            browse: BrowseConfig::default(),
            writeback: WritebackConfig::default(),
            cli: CliDefaults::default(),
            group: GroupConfig::default(),
        };
        let toml_str = toml::to_string_pretty(&original).unwrap();
        let parsed: CatalogConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.preview, original.preview);
        assert_eq!(parsed.serve, original.serve);
        assert_eq!(parsed.import, original.import);
    }

    #[test]
    fn validate_zero_max_edge_errors() {
        let config = CatalogConfig {
            preview: PreviewConfig {
                max_edge: 0,
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn validate_zero_quality_errors() {
        let config = CatalogConfig {
            preview: PreviewConfig {
                quality: 0,
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn validate_quality_over_100_errors() {
        let config = CatalogConfig {
            preview: PreviewConfig {
                quality: 101,
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn validate_zero_smart_max_edge_errors() {
        let config = CatalogConfig {
            preview: PreviewConfig {
                smart_max_edge: 0,
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn validate_zero_smart_quality_errors() {
        let config = CatalogConfig {
            preview: PreviewConfig {
                smart_quality: 0,
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn validate_smart_quality_over_100_errors() {
        let config = CatalogConfig {
            preview: PreviewConfig {
                smart_quality: 101,
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn validate_default_passes() {
        CatalogConfig::default().validate().unwrap();
    }

    #[test]
    fn preview_format_extension() {
        assert_eq!(PreviewFormat::Jpeg.extension(), "jpg");
        assert_eq!(PreviewFormat::Webp.extension(), "webp");
    }

    #[test]
    fn parse_smart_preview_config() {
        let input = r#"
[preview]
smart_max_edge = 3000
smart_quality = 92
"#;
        let config: CatalogConfig = toml::from_str(input).unwrap();
        assert_eq!(config.preview.smart_max_edge, 3000);
        assert_eq!(config.preview.smart_quality, 92);
        assert_eq!(config.preview.max_edge, 800); // default
        assert_eq!(config.preview.quality, 85); // default
    }

    #[test]
    fn parse_smart_preview_defaults_when_missing() {
        let input = r#"
[preview]
max_edge = 1000
"#;
        let config: CatalogConfig = toml::from_str(input).unwrap();
        assert_eq!(config.preview.smart_max_edge, 2560); // default
        assert_eq!(config.preview.smart_quality, 85); // default
    }

    #[test]
    fn load_creates_default_when_no_file() {
        let dir = tempfile::tempdir().unwrap();
        let config = CatalogConfig::load(dir.path()).unwrap();
        assert!(config.default_volume.is_none());
        assert_eq!(config.preview, PreviewConfig::default());
    }

    #[test]
    fn save_and_load_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let original = CatalogConfig {
            default_volume: None,
            preview: PreviewConfig {
                max_edge: 1200,
                format: PreviewFormat::Webp,
                quality: 90,
                smart_max_edge: 3000,
                smart_quality: 92,
                ..Default::default()
            },
            serve: ServeConfig::default(),
            import: ImportConfig {
                exclude: vec!["*.tmp".to_string()],
                auto_tags: vec![],
                ..Default::default()
            },
            dedup: DedupConfig::default(),
            verify: VerifyConfig::default(),
            ai: AiConfig::default(),
            contact_sheet: ContactSheetDefaults::default(),
            vlm: VlmConfig::default(),
            browse: BrowseConfig::default(),
            writeback: WritebackConfig::default(),
            cli: CliDefaults::default(),
            group: GroupConfig::default(),
        };
        original.save(dir.path()).unwrap();
        let loaded = CatalogConfig::load(dir.path()).unwrap();
        assert_eq!(loaded.preview, original.preview);
        assert_eq!(loaded.import.exclude, original.import.exclude);
    }

    #[test]
    fn parse_browse_default_filter() {
        let input = "[browse]\ndefault_filter = \"rating:1+ -tag:rest\"\n";
        let config: CatalogConfig = toml::from_str(input).unwrap();
        assert_eq!(config.browse.default_filter, Some("rating:1+ -tag:rest".to_string()));
    }

    #[test]
    fn parse_browse_empty_section() {
        let input = "[browse]\n";
        let config: CatalogConfig = toml::from_str(input).unwrap();
        assert_eq!(config.browse.default_filter, None);
    }

    #[test]
    fn parse_verify_config() {
        let input = "[verify]\nmax_age_days = 30\n";
        let config: CatalogConfig = toml::from_str(input).unwrap();
        assert_eq!(config.verify.max_age_days, Some(30));
    }

    #[test]
    fn parse_import_profiles() {
        let input = r#"
[import]
auto_tags = ["inbox"]
smart_previews = true

[import.profiles.card]
auto_tags = ["from-card"]
include = ["captureone"]

[import.profiles.studio]
smart_previews = true
embeddings = true
descriptions = true
skip = ["audio"]
"#;
        let config: CatalogConfig = toml::from_str(input).unwrap();
        assert_eq!(config.import.profiles.len(), 2);
        assert!(config.import.profiles.contains_key("card"));
        assert!(config.import.profiles.contains_key("studio"));

        let card = config.import.profiles.get("card").unwrap();
        assert_eq!(card.auto_tags, Some(vec!["from-card".to_string()]));
        assert_eq!(card.smart_previews, None); // inherits from base
        assert_eq!(card.include, vec!["captureone".to_string()]);

        let studio = config.import.profiles.get("studio").unwrap();
        assert_eq!(studio.smart_previews, Some(true));
        assert_eq!(studio.embeddings, Some(true));
        assert_eq!(studio.descriptions, Some(true));
        assert_eq!(studio.skip, vec!["audio".to_string()]);
    }

    #[test]
    fn resolve_import_profile_inherits_base() {
        let config = ImportConfig {
            exclude: vec![".DS_Store".to_string()],
            auto_tags: vec!["inbox".to_string()],
            smart_previews: false,
            embeddings: false,
            descriptions: false,
            profiles: {
                let mut m = std::collections::HashMap::new();
                m.insert("card".to_string(), ImportProfile {
                    auto_tags: Some(vec!["from-card".to_string()]),
                    smart_previews: Some(true),
                    ..Default::default()
                });
                m
            },
        };

        let resolved = config.resolve_profile("card").unwrap();
        // Overridden by profile
        assert_eq!(resolved.auto_tags, vec!["from-card".to_string()]);
        assert!(resolved.smart_previews);
        // Inherited from base
        assert_eq!(resolved.exclude, vec![".DS_Store".to_string()]);
        assert!(!resolved.embeddings);
        assert!(!resolved.descriptions);
    }

    #[test]
    fn resolve_unknown_profile_returns_none() {
        let config = ImportConfig::default();
        assert!(config.resolve_profile("nonexistent").is_none());
    }
}
