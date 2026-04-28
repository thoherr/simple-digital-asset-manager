//! Aggregated catalog health report — backing data for `maki status`.
//!
//! Stitches together signals already exposed by other commands (cleanup
//! dry-run, backup-status overview, schema-version peek, embedding /
//! face-scan coverage queries) into one read-only "what needs attention"
//! survey. Render is intentionally split off in the CLI handler so the
//! same `StatusReport` struct serializes cleanly to JSON.
//!
//! Performance note: orphan-on-disk counts come from the existing
//! `AssetService::cleanup` dry-run pass, which scans `<catalog_root>/
//! {previews,smart-previews,embeddings,faces}` against the catalog. On
//! large catalogs this dominates runtime — but it's the same scan
//! `maki cleanup` already runs without `--apply`, so the cost is known.
//! Status doesn't run the path-scoped passes; orphan-on-disk is a
//! catalog-wide concern by definition.

use std::path::Path;

use anyhow::Result;
use serde::Serialize;

use crate::catalog::Catalog;
use crate::device_registry::DeviceRegistry;
use crate::Verbosity;

/// Top-level structured result from a status survey.
#[derive(Debug, Serialize)]
pub struct StatusReport {
    pub catalog_root: String,
    pub catalog: CatalogOverview,
    pub cleanup: CleanupNeeds,
    pub pending: PendingWork,
    pub backup: BackupSnapshot,
    pub volumes: Vec<VolumeRow>,
    /// Whether the build had the AI feature enabled — affects how the
    /// renderer interprets `pending.assets_without_embedding` etc.
    pub ai_enabled: bool,
}

#[derive(Debug, Serialize)]
pub struct CatalogOverview {
    pub schema_version: u32,
    pub schema_current: u32,
    pub assets: u64,
    pub variants: u64,
    pub recipes: u64,
    pub file_locations: u64,
    /// Total size of all variants on disk (rolled up from the variants table).
    pub total_bytes: u64,
}

#[derive(Debug, Serialize)]
pub struct CleanupNeeds {
    pub locationless_variants: usize,
    pub orphaned_assets: usize,
    pub orphaned_previews: usize,
    pub orphaned_smart_previews: usize,
    pub orphaned_embeddings: usize,
    pub orphaned_face_files: usize,
}

#[derive(Debug, Serialize)]
pub struct PendingWork {
    /// XMP writeback queue split by whether the target volume is reachable.
    pub pending_writebacks_online: usize,
    pub pending_writebacks_offline: usize,
    /// Whether the writeback feature is enabled in `[writeback]` config —
    /// when false, queued writebacks won't process until the user opts in.
    pub writeback_enabled: bool,
    /// Assets with no row in the embeddings table (any model). Only meaningful
    /// when `ai_enabled = true`. None if AI feature isn't compiled in.
    pub assets_without_embedding: Option<u64>,
    /// Assets with face_scan_status NULL or 'pending'. Same caveat as above.
    pub assets_without_face_scan: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct BackupSnapshot {
    /// Assets with fewer than `min_copies` distinct volume copies.
    pub at_risk: u64,
    pub min_copies: u64,
    pub total_assets: u64,
}

#[derive(Debug, Serialize)]
pub struct VolumeRow {
    pub label: String,
    pub mount_point: String,
    pub purpose: Option<String>,
    pub is_online: bool,
    pub asset_count: u64,
    pub size_bytes: u64,
}

/// Run all the catalog-state queries needed for `maki status`.
///
/// `min_copies` defaults to the user's `[backup] min_copies` (the same
/// default `backup-status` uses). `ai_enabled` should match the build
/// feature so AI-only fields are computed only when relevant.
pub fn gather(
    catalog_root: &Path,
    verbosity: Verbosity,
    preview_config: &crate::config::PreviewConfig,
    min_copies: u64,
    ai_enabled: bool,
) -> Result<StatusReport> {
    let catalog = Catalog::open(catalog_root)?;
    let registry = DeviceRegistry::new(catalog_root);
    let volumes = registry.list().unwrap_or_default();

    // ── Catalog overview ─────────────────────────────────
    let (assets, variants, recipes, total_bytes, file_locations) = catalog.stats_overview()?;
    let catalog_overview = CatalogOverview {
        schema_version: catalog.schema_version(),
        schema_current: crate::catalog::SCHEMA_VERSION,
        assets,
        variants,
        recipes,
        file_locations,
        total_bytes,
    };

    // ── Cleanup needs (dry-run cleanup, catalog-wide) ───
    // Reuses the existing cleanup pipeline — same passes, same SQL,
    // same disk scan — without `--apply`. Drops the per-file callback
    // since we only want totals.
    let service = crate::asset_service::AssetService::new(catalog_root, verbosity, preview_config);
    let cleanup_dry = service.cleanup(None, None, false, |_, _, _| {})?;
    let cleanup = CleanupNeeds {
        locationless_variants: cleanup_dry.locationless_variants,
        orphaned_assets: cleanup_dry.orphaned_assets,
        orphaned_previews: cleanup_dry.orphaned_previews,
        orphaned_smart_previews: cleanup_dry.orphaned_smart_previews,
        orphaned_embeddings: cleanup_dry.orphaned_embeddings,
        orphaned_face_files: cleanup_dry.orphaned_face_files,
    };

    // ── Pending work ────────────────────────────────────
    let pending_writebacks_all = catalog.list_pending_writeback_recipes(None)?;
    let online_volume_ids: std::collections::HashSet<String> = volumes
        .iter()
        .filter(|v| v.is_online)
        .map(|v| v.id.to_string())
        .collect();
    let mut pwb_on = 0usize;
    let mut pwb_off = 0usize;
    // tuple = (recipe_id, asset_id, volume_id, relative_path)
    for r in &pending_writebacks_all {
        if online_volume_ids.contains(&r.2) {
            pwb_on += 1;
        } else {
            pwb_off += 1;
        }
    }
    let writeback_enabled = crate::config::CatalogConfig::load(catalog_root)
        .map(|c| c.writeback.enabled)
        .unwrap_or(false);

    let (assets_without_embedding, assets_without_face_scan) = if ai_enabled {
        let total = assets as i64;
        let with_emb: i64 = catalog
            .conn()
            .query_row(
                "SELECT COUNT(DISTINCT asset_id) FROM embeddings",
                [],
                |r| r.get(0),
            )
            .unwrap_or(0);
        let no_face_scan: i64 = catalog
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM assets \
                 WHERE face_scan_status IS NULL OR face_scan_status = 'pending'",
                [],
                |r| r.get(0),
            )
            .unwrap_or(0);
        (
            Some((total - with_emb).max(0) as u64),
            Some(no_face_scan.max(0) as u64),
        )
    } else {
        (None, None)
    };

    let pending = PendingWork {
        pending_writebacks_online: pwb_on,
        pending_writebacks_offline: pwb_off,
        writeback_enabled,
        assets_without_embedding,
        assets_without_face_scan,
    };

    // ── Backup coverage ─────────────────────────────────
    let volumes_info: Vec<(String, String, bool, Option<String>)> = volumes
        .iter()
        .map(|v| {
            (
                v.id.to_string(),
                v.label.clone(),
                v.is_online,
                v.purpose.as_ref().map(|p| p.as_str().to_string()),
            )
        })
        .collect();
    let bs = catalog.backup_status_overview(None, &volumes_info, min_copies, None)?;
    let backup = BackupSnapshot {
        at_risk: bs.at_risk_count,
        min_copies,
        total_assets: bs.total_assets,
    };

    // ── Volumes ─────────────────────────────────────────
    let mut volume_rows: Vec<VolumeRow> = Vec::with_capacity(volumes.len());
    for v in &volumes {
        let vol_id = v.id.to_string();
        // Per-volume asset count + size: distinct assets that have at least
        // one variant location on this volume.
        let (count, size): (i64, i64) = catalog
            .conn()
            .query_row(
                "SELECT COUNT(DISTINCT v.asset_id), COALESCE(SUM(v.file_size), 0) \
                 FROM variants v \
                 JOIN file_locations fl ON fl.content_hash = v.content_hash \
                 WHERE fl.volume_id = ?1",
                rusqlite::params![&vol_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap_or((0, 0));
        volume_rows.push(VolumeRow {
            label: v.label.clone(),
            mount_point: v.mount_point.to_string_lossy().to_string(),
            purpose: v.purpose.as_ref().map(|p| p.as_str().to_string()),
            is_online: v.is_online,
            asset_count: count.max(0) as u64,
            size_bytes: size.max(0) as u64,
        });
    }
    // Online first, then offline; alpha within each.
    volume_rows.sort_by(|a, b| {
        b.is_online
            .cmp(&a.is_online)
            .then_with(|| a.label.to_lowercase().cmp(&b.label.to_lowercase()))
    });

    Ok(StatusReport {
        catalog_root: catalog_root.display().to_string(),
        catalog: catalog_overview,
        cleanup,
        pending,
        backup,
        volumes: volume_rows,
        ai_enabled,
    })
}
