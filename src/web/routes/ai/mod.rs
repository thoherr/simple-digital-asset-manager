//! AI-gated route handlers, split by domain.
//!
//! All items are `#[cfg(feature = "ai")]` — this module is only compiled
//! when the `ai` feature is enabled. The parent `routes::mod` declares it
//! behind the same feature gate.
//!
//! Submodules:
//! - [`tags`] — `suggest-tags`, batch `auto-tag`.
//! - [`embed`] — standalone embed (browse toolbar, asset detail).
//! - [`similarity`] — `find-similar` and stack-by-similarity.
//! - [`faces`] — face detection, person assignment, people page.
//! - [`stroll`] — visual exploration page.
//!
//! Shared helpers (model dir resolution, label loading) live here so each
//! submodule can use them via `super::`.

mod embed;
mod faces;
mod similarity;
mod stroll;
mod tags;

pub use embed::*;
pub use faces::*;
pub use similarity::*;
pub use stroll::*;
pub use tags::*;

/// Resolve the directory holding the active SigLIP model.
///
/// Re-exported for `web/routes/browse.rs` (used by similar-search resolution).
/// Submodules call this via `super::resolve_model_dir`.
pub(super) fn resolve_model_dir(config: &crate::config::AiConfig) -> std::path::PathBuf {
    crate::config::resolve_model_dir(&config.model_dir, &config.model)
}

/// Load the auto-tag label list (from a configured labels file, or the built-in
/// default vocabulary).
pub(super) fn resolve_labels(config: &crate::config::AiConfig) -> Result<Vec<String>, String> {
    if let Some(ref labels_path) = config.labels {
        crate::ai::load_labels_from_file(std::path::Path::new(labels_path))
            .map_err(|e| format!("Failed to load labels: {e}"))
    } else {
        Ok(crate::ai::DEFAULT_LABELS.iter().map(|s| s.to_string()).collect())
    }
}
