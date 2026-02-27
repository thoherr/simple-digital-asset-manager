use std::path::PathBuf;

use clap::{Parser, Subcommand};
use dam::asset_service::AssetService;
use dam::catalog::Catalog;
use dam::config::CatalogConfig;
use dam::device_registry::DeviceRegistry;
use dam::metadata_store::MetadataStore;
use dam::query::QueryEngine;

#[derive(Parser)]
#[command(name = "dam", about = "Digital Asset Manager", version,
    after_help = "\
Quick Reference:
  Setup:      init, volume
  Ingest:     import, tag, edit, group, auto-group
  Organize:   collection (col), saved-search (ss), stack (st)
  Retrieve:   search, show, duplicates, stats, backup-status, serve
  Maintain:   verify, sync, refresh, cleanup, dedup, relocate,
              update-location, generate-previews, fix-roles,
              fix-dates, rebuild-catalog"
)]
struct Cli {
    /// Output machine-readable JSON (valid JSON on stdout, messages on stderr)
    #[arg(long, global = true, display_order = 30)]
    json: bool,

    /// Log individual file progress (import, verify, generate-previews)
    #[arg(short = 'l', long = "log", global = true, display_order = 40)]
    log: bool,

    /// Show debug output from external tools (ffmpeg, dcraw)
    #[arg(short = 'd', long = "debug", global = true, display_order = 41)]
    debug: bool,

    /// Show elapsed time after command execution
    #[arg(short = 't', long = "time", global = true, display_order = 42)]
    timing: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    // --- Setup ---

    /// Initialize a new catalog in the current directory
    #[command(display_order = 1)]
    Init,

    /// Manage storage volumes
    #[command(subcommand, display_order = 2)]
    Volume(VolumeCommands),

    // --- Ingest & Edit ---

    /// Import files into the catalog
    #[command(display_order = 10)]
    Import {
        /// Paths to files or directories to import
        paths: Vec<String>,

        /// Import onto a specific volume (instead of auto-detecting from path)
        #[arg(long, display_order = 10)]
        volume: Option<String>,

        /// Include additional file type groups (e.g. captureone, documents)
        #[arg(long, display_order = 12)]
        include: Vec<String>,

        /// Skip default file type groups (e.g. audio, xmp)
        #[arg(long, display_order = 13)]
        skip: Vec<String>,

        /// Add a tag to every imported asset (repeatable)
        #[arg(long = "add-tag", display_order = 15)]
        add_tags: Vec<String>,

        /// Show what would be imported without making changes
        #[arg(long, display_order = 20)]
        dry_run: bool,

        /// Auto-group imported files with nearby catalog assets by filename stem
        #[arg(long, display_order = 21)]
        auto_group: bool,
    },

    /// Add or remove tags on an asset
    #[command(display_order = 11)]
    Tag {
        /// Asset ID
        asset_id: String,

        /// Remove the specified tags instead of adding them
        #[arg(long)]
        remove: bool,

        /// Tags to add or remove
        tags: Vec<String>,
    },

    /// Edit asset metadata (name, description, rating)
    #[command(display_order = 12)]
    Edit {
        /// Asset ID (or unique prefix)
        asset_id: String,

        /// Set asset name
        #[arg(long)]
        name: Option<String>,

        /// Clear asset name
        #[arg(long)]
        clear_name: bool,

        /// Set asset description
        #[arg(long)]
        description: Option<String>,

        /// Clear asset description
        #[arg(long)]
        clear_description: bool,

        /// Set rating (1-5)
        #[arg(long, value_parser = clap::value_parser!(u8).range(1..=5))]
        rating: Option<u8>,

        /// Clear rating
        #[arg(long)]
        clear_rating: bool,

        /// Set color label (Red, Orange, Yellow, Green, Blue, Pink, Purple)
        #[arg(long)]
        label: Option<String>,

        /// Clear color label
        #[arg(long)]
        clear_label: bool,
    },

    /// Group variants into one asset
    #[command(display_order = 13)]
    Group {
        /// Content hashes of variants to group
        variant_hashes: Vec<String>,
    },

    /// Auto-group assets by filename stem
    #[command(display_order = 14)]
    AutoGroup {
        /// Search query to scope assets (same syntax as dam search)
        query: Option<String>,
        /// Apply grouping (default: report-only)
        #[arg(long)]
        apply: bool,
    },

    // --- Organize ---

    /// Manage collections (static albums)
    #[command(subcommand, alias = "col", display_order = 20)]
    Collection(CollectionCommands),

    /// Manage saved searches (smart albums)
    #[command(subcommand, alias = "ss", display_order = 21)]
    SavedSearch(SavedSearchCommands),

    /// Manage stacks (scene grouping)
    #[command(subcommand, alias = "st", display_order = 22)]
    Stack(StackCommands),

    // --- Retrieve ---

    /// Search assets
    #[command(display_order = 30)]
    Search {
        /// Free-text keywords and optional filters (type:image, tag:landscape, format:jpg,
        /// rating:3+, camera:fuji, lens:56mm, iso:100-800, focal:35-70, f:1.4-2.8,
        /// width:4000+, height:2000+, meta:key=value, orphan:true, missing:true,
        /// stale:30, volume:none)
        query: String,

        /// Output format: ids, short, full, json, or a custom template (e.g. '{id}\t{name}')
        #[arg(long)]
        format: Option<String>,

        /// Shorthand for --format=ids (one asset ID per line, for scripting)
        #[arg(short = 'q', long = "quiet")]
        quiet: bool,
    },

    /// Show asset details
    #[command(display_order = 31)]
    Show {
        /// Asset ID
        asset_id: String,
    },

    /// Find duplicate files
    #[command(display_order = 32)]
    Duplicates {
        /// Output format: ids, short, full, json, or a custom template (e.g. '{hash}\t{filename}')
        #[arg(long)]
        format: Option<String>,

        /// Show only same-volume duplicates (likely unwanted copies)
        #[arg(long, display_order = 10)]
        same_volume: bool,

        /// Show only cross-volume copies (wanted backups)
        #[arg(long, display_order = 11)]
        cross_volume: bool,

        /// Filter to entries involving this volume
        #[arg(long, display_order = 12)]
        volume: Option<String>,

        /// Filter to entries matching this file format (e.g. nef, jpg)
        #[arg(long, display_order = 13)]
        filter_format: Option<String>,

        /// Filter to entries with a location under this path prefix
        #[arg(long, display_order = 14)]
        path: Option<String>,
    },

    /// Show catalog statistics
    #[command(display_order = 33)]
    Stats {
        /// Show asset type and format breakdown
        #[arg(long)]
        types: bool,

        /// Show per-volume details
        #[arg(long)]
        volumes: bool,

        /// Show tag usage statistics
        #[arg(long)]
        tags: bool,

        /// Show verification health
        #[arg(long)]
        verified: bool,

        /// Show all sections
        #[arg(long)]
        all: bool,

        /// Max entries for top-N lists (default: 20)
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },

    /// Check backup coverage and find under-backed-up assets
    #[command(name = "backup-status", display_order = 35)]
    BackupStatus {
        /// Search query to scope the asset universe (same syntax as dam search)
        query: Option<String>,

        /// List under-backed-up assets (fewer than --min-copies locations)
        #[arg(long, display_order = 10)]
        at_risk: bool,

        /// Minimum copies for adequate backup (default: 2)
        #[arg(long, default_value = "2", display_order = 11)]
        min_copies: u64,

        /// Show which scoped assets are missing from this volume
        #[arg(long, display_order = 12)]
        volume: Option<String>,

        /// Output format for --at-risk listings (ids, short, full, json, or template)
        #[arg(long, display_order = 20)]
        format: Option<String>,

        /// Shorthand for --format=ids (one asset ID per line, for scripting)
        #[arg(short = 'q', long = "quiet", display_order = 21)]
        quiet: bool,
    },

    /// Start the web UI server
    #[command(display_order = 36)]
    Serve {
        /// Port to listen on (default: 8080, or from dam.toml [serve] port)
        #[arg(long, display_order = 10)]
        port: Option<u16>,

        /// Address to bind to (default: 127.0.0.1, or from dam.toml [serve] bind)
        #[arg(long, display_order = 11)]
        bind: Option<String>,
    },

    // --- Maintenance ---

    /// Check file integrity
    #[command(display_order = 40)]
    Verify {
        /// Files or directories to verify
        paths: Vec<String>,

        /// Limit verification to a specific volume
        #[arg(long, display_order = 10)]
        volume: Option<String>,

        /// Verify only a specific asset
        #[arg(long, display_order = 11)]
        asset: Option<String>,

        /// Include additional file type groups (e.g. captureone, documents)
        #[arg(long, display_order = 12)]
        include: Vec<String>,

        /// Skip default file type groups (e.g. audio, xmp)
        #[arg(long, display_order = 13)]
        skip: Vec<String>,
    },

    /// Sync catalog with disk changes (moved/modified/missing files)
    #[command(display_order = 41)]
    Sync {
        /// Paths to files or directories to scan
        paths: Vec<String>,

        /// Use a specific volume (instead of auto-detecting from path)
        #[arg(long, display_order = 10)]
        volume: Option<String>,

        /// Apply changes to catalog and sidecar files
        #[arg(long, display_order = 20)]
        apply: bool,

        /// Remove catalog locations for missing files (requires --apply)
        #[arg(long, display_order = 21)]
        remove_stale: bool,
    },

    /// Re-read metadata from changed sidecar/recipe files
    #[command(display_order = 42)]
    Refresh {
        /// Paths to files or directories to scan
        paths: Vec<String>,

        /// Limit to a specific volume
        #[arg(long, display_order = 10)]
        volume: Option<String>,

        /// Refresh only a specific asset
        #[arg(long, display_order = 11)]
        asset: Option<String>,

        /// Preview what would change without applying
        #[arg(long, display_order = 20)]
        dry_run: bool,

        /// Also re-extract embedded XMP from JPEG/TIFF media files
        #[arg(long, display_order = 21)]
        media: bool,
    },

    /// Remove stale file location records (files no longer on disk)
    #[command(display_order = 43)]
    Cleanup {
        /// Limit to a specific volume
        #[arg(long, display_order = 10)]
        volume: Option<String>,

        /// List stale entries
        #[arg(long, display_order = 15)]
        list: bool,

        /// Apply changes (remove stale records from catalog and sidecar files)
        #[arg(long, display_order = 20)]
        apply: bool,
    },

    /// Remove same-volume duplicate file locations
    #[command(display_order = 44)]
    Dedup {
        /// Limit to a specific volume
        #[arg(long, display_order = 10)]
        volume: Option<String>,

        /// Prefer keeping locations whose path contains this string
        #[arg(long, display_order = 11)]
        prefer: Option<String>,

        /// Filter to a specific file format (e.g. nef, jpg)
        #[arg(long, display_order = 12)]
        filter_format: Option<String>,

        /// Filter to locations under this path prefix
        #[arg(long, display_order = 13)]
        path: Option<String>,

        /// Minimum total copies to preserve per variant (default: 1)
        #[arg(long, display_order = 14, default_value = "1")]
        min_copies: usize,

        /// Apply changes (delete files and remove location records)
        #[arg(long, display_order = 20)]
        apply: bool,
    },

    /// Copy or move asset files to another volume
    #[command(display_order = 45)]
    Relocate {
        /// Asset ID
        asset_id: String,

        /// Target volume label or ID
        volume: String,

        /// Delete source files after successful copy and verification
        #[arg(long)]
        remove_source: bool,

        /// Show what would happen without making changes
        #[arg(long)]
        dry_run: bool,
    },

    /// Update a file's catalog path after it was moved on disk
    #[command(name = "update-location", display_order = 46)]
    UpdateLocation {
        /// Asset ID (or unique prefix)
        asset_id: String,

        /// Old path (absolute or volume-relative) where the file was before
        #[arg(long)]
        from: String,

        /// New absolute path where the file is now
        #[arg(long)]
        to: String,

        /// Volume label or ID (auto-detected from --to if omitted)
        #[arg(long)]
        volume: Option<String>,
    },

    /// Generate or regenerate preview thumbnails
    #[command(display_order = 47)]
    GeneratePreviews {
        /// Files or directories to generate previews for
        paths: Vec<String>,

        /// Limit to variants on a specific volume
        #[arg(long, display_order = 10)]
        volume: Option<String>,

        /// Only generate preview for a specific asset
        #[arg(long, display_order = 11)]
        asset: Option<String>,

        /// Include additional file type groups (e.g. captureone, documents)
        #[arg(long, display_order = 12)]
        include: Vec<String>,

        /// Skip default file type groups (e.g. audio, xmp)
        #[arg(long, display_order = 13)]
        skip: Vec<String>,

        /// Force regeneration even if previews already exist
        #[arg(long, display_order = 20)]
        force: bool,

        /// Regenerate previews for assets where a better variant (export/processed) exists
        #[arg(long, display_order = 21)]
        upgrade: bool,

        /// Generate smart previews (high-resolution, 2560px) instead of thumbnails
        #[arg(long, display_order = 22)]
        smart: bool,
    },

    /// Fix variant roles (re-role non-RAW variants to Export in RAW+non-RAW groups)
    #[command(display_order = 48)]
    FixRoles {
        /// Files or directories to scope the fix
        paths: Vec<String>,

        /// Limit to a specific volume
        #[arg(long, display_order = 10)]
        volume: Option<String>,

        /// Fix only a specific asset
        #[arg(long, display_order = 11)]
        asset: Option<String>,

        /// Apply changes (default: report-only dry run)
        #[arg(long, display_order = 20)]
        apply: bool,
    },

    /// Fix asset dates from variant EXIF metadata and file modification times
    #[command(display_order = 49)]
    FixDates {
        /// Limit to a specific volume
        #[arg(long, display_order = 10)]
        volume: Option<String>,

        /// Fix only a specific asset
        #[arg(long, display_order = 11)]
        asset: Option<String>,

        /// Apply changes (default: report-only dry run)
        #[arg(long, display_order = 20)]
        apply: bool,
    },

    /// Re-attach recipe files that were imported as standalone assets
    #[command(display_order = 50)]
    FixRecipes {
        /// Limit to a specific volume
        #[arg(long, display_order = 10)]
        volume: Option<String>,

        /// Fix only a specific asset
        #[arg(long, display_order = 11)]
        asset: Option<String>,

        /// Apply changes (default: report-only dry run)
        #[arg(long, display_order = 20)]
        apply: bool,
    },

    /// Rebuild SQLite catalog from sidecar files
    #[command(display_order = 51)]
    RebuildCatalog,
}

#[derive(Subcommand)]
enum VolumeCommands {
    /// Register a new volume
    Add {
        /// Human-readable label for the volume
        label: String,

        /// Mount point path
        path: String,

        /// Volume purpose (working, archive, backup, cloud)
        #[arg(long)]
        purpose: Option<String>,
    },

    /// List all volumes and their status
    List,

    /// Set or clear the purpose of a volume
    SetPurpose {
        /// Volume label or UUID
        volume: String,

        /// Purpose (working, archive, backup, cloud) or "none" to clear
        purpose: String,
    },

    /// Remove a volume and all its locations, recipes, and orphaned assets
    Remove {
        /// Volume label or UUID
        volume: String,

        /// Actually remove (default is report-only)
        #[arg(long)]
        apply: bool,
    },

    /// Combine a source volume into a target volume, rewriting paths
    Combine {
        /// Source volume label or UUID (will be removed)
        source: String,

        /// Target volume label or UUID (receives locations/recipes)
        target: String,

        /// Actually combine (default is report-only)
        #[arg(long)]
        apply: bool,
    },
}

#[derive(Subcommand)]
enum SavedSearchCommands {
    /// Save a search with a name
    Save {
        /// Name for this saved search
        name: String,

        /// Search query (same format as `dam search`)
        query: String,

        /// Sort order (e.g. date_desc, name_asc, size_desc)
        #[arg(long)]
        sort: Option<String>,

        /// Mark as favorite (shown as chip on browse page)
        #[arg(long)]
        favorite: bool,
    },

    /// List all saved searches
    List,

    /// Run a saved search by name
    Run {
        /// Name of the saved search to execute
        name: String,

        /// Output format: ids, short, full, json, or a custom template
        #[arg(long)]
        format: Option<String>,
    },

    /// Delete a saved search by name
    Delete {
        /// Name of the saved search to delete
        name: String,
    },
}

#[derive(Subcommand)]
enum CollectionCommands {
    /// Create a new collection
    Create {
        /// Collection name
        name: String,

        /// Optional description
        #[arg(long)]
        description: Option<String>,
    },

    /// List all collections
    List,

    /// Show collection contents
    Show {
        /// Collection name
        name: String,

        /// Output format: ids, short, full, json, or a custom template
        #[arg(long)]
        format: Option<String>,
    },

    /// Add assets to a collection
    Add {
        /// Collection name
        name: String,

        /// Asset IDs to add
        asset_ids: Vec<String>,
    },

    /// Remove assets from a collection
    Remove {
        /// Collection name
        name: String,

        /// Asset IDs to remove
        asset_ids: Vec<String>,
    },

    /// Delete a collection
    Delete {
        /// Collection name
        name: String,
    },
}

#[derive(Subcommand)]
enum StackCommands {
    /// Create a new stack from the given assets
    Create {
        /// Asset IDs to stack (first becomes pick)
        asset_ids: Vec<String>,
    },

    /// Add assets to an existing stack
    Add {
        /// Any asset already in the target stack
        reference: String,

        /// Asset IDs to add
        asset_ids: Vec<String>,
    },

    /// Remove assets from their stack
    Remove {
        /// Asset IDs to remove from stacks
        asset_ids: Vec<String>,
    },

    /// Set the pick (top) of a stack
    Pick {
        /// Asset ID to make the pick
        asset_id: String,
    },

    /// Dissolve an entire stack
    Dissolve {
        /// Any asset in the stack to dissolve
        asset_id: String,
    },

    /// List all stacks
    List,

    /// Show members of a stack
    Show {
        /// Any asset in the stack
        asset_id: String,

        /// Output format: ids, short, full, json, or a custom template
        #[arg(long)]
        format: Option<String>,
    },

    /// Convert matching tags into stacks
    FromTag {
        /// Tag pattern with {} as wildcard (e.g. "Aperture Stack {}")
        pattern: String,

        /// Remove the matched tag from stacked assets after conversion
        #[arg(long)]
        remove_tags: bool,

        /// Actually create stacks (default: report only)
        #[arg(long)]
        apply: bool,
    },
}

fn main() {
    let cli = Cli::parse();
    let start = std::time::Instant::now();

    let result: anyhow::Result<()> = (|| match cli.command {
        Commands::Init => {
            let catalog_root = std::env::current_dir()?;
            let config_path = catalog_root.join("dam.toml");
            if config_path.exists() {
                anyhow::bail!("A dam catalog already exists in this directory.");
            }

            // Create directories
            std::fs::create_dir_all(catalog_root.join("metadata"))?;
            std::fs::create_dir_all(catalog_root.join("previews"))?;

            // Write config
            CatalogConfig::default().save(&catalog_root)?;

            // Initialize SQLite schema
            let catalog = Catalog::open(&catalog_root)?;
            catalog.initialize()?;

            // Write empty volumes registry
            DeviceRegistry::init(&catalog_root)?;

            if cli.json {
                println!("{}", serde_json::json!({
                    "status": "initialized",
                    "path": catalog_root.display().to_string()
                }));
            } else {
                println!("Initialized new dam catalog in {}", catalog_root.display());
            }
            Ok(())
        }
        Commands::Volume(cmd) => match cmd {
            VolumeCommands::Add { label, path, purpose } => {
                let catalog_root = dam::config::find_catalog_root()?;
                let registry = DeviceRegistry::new(&catalog_root);
                let parsed_purpose = if let Some(ref p) = purpose {
                    Some(dam::models::VolumePurpose::parse(p).ok_or_else(|| {
                        anyhow::anyhow!("Invalid purpose '{}'. Valid values: working, archive, backup, cloud", p)
                    })?)
                } else {
                    None
                };
                let volume = registry.register(
                    &label,
                    std::path::Path::new(&path),
                    dam::models::VolumeType::Local,
                    parsed_purpose,
                )?;
                if cli.json {
                    println!("{}", serde_json::json!({
                        "id": volume.id.to_string(),
                        "label": volume.label,
                        "path": volume.mount_point.display().to_string(),
                        "purpose": volume.purpose.as_ref().map(|p| p.as_str()),
                    }));
                } else {
                    println!("Registered volume '{}' ({})", volume.label, volume.id);
                    println!("  Path: {}", volume.mount_point.display());
                    if let Some(ref p) = volume.purpose {
                        println!("  Purpose: {}", p);
                    } else {
                        eprintln!("  Hint: use --purpose <working|archive|backup|cloud> to set the volume's role");
                    }
                }
                Ok(())
            }
            VolumeCommands::List => {
                let catalog_root = dam::config::find_catalog_root()?;
                let registry = DeviceRegistry::new(&catalog_root);
                let volumes = registry.list()?;
                if cli.json {
                    let json_volumes: Vec<serde_json::Value> = volumes.iter().map(|v| {
                        serde_json::json!({
                            "id": v.id.to_string(),
                            "label": v.label,
                            "path": v.mount_point.display().to_string(),
                            "volume_type": format!("{:?}", v.volume_type).to_lowercase(),
                            "purpose": v.purpose.as_ref().map(|p| p.as_str()),
                            "is_online": v.is_online,
                        })
                    }).collect();
                    println!("{}", serde_json::to_string_pretty(&json_volumes)?);
                } else if volumes.is_empty() {
                    println!("No volumes registered.");
                } else {
                    for v in &volumes {
                        let status = if v.is_online { "online" } else { "offline" };
                        let purpose_tag = v.purpose.as_ref()
                            .map(|p| format!(" [{}]", p))
                            .unwrap_or_default();
                        println!("{} ({}) [{}]{}", v.label, v.id, status, purpose_tag);
                        println!("  Path: {}", v.mount_point.display());
                    }
                }
                Ok(())
            }
            VolumeCommands::SetPurpose { volume, purpose } => {
                let catalog_root = dam::config::find_catalog_root()?;
                let registry = DeviceRegistry::new(&catalog_root);
                let parsed_purpose = if purpose == "none" || purpose == "clear" {
                    None
                } else {
                    Some(dam::models::VolumePurpose::parse(&purpose).ok_or_else(|| {
                        anyhow::anyhow!("Invalid purpose '{}'. Valid values: working, archive, backup, cloud, none", purpose)
                    })?)
                };
                let vol = registry.set_purpose(&volume, parsed_purpose)?;
                // Update the SQLite cache too
                let catalog = dam::catalog::Catalog::open(&catalog_root)?;
                catalog.ensure_volume(&vol)?;
                if cli.json {
                    println!("{}", serde_json::json!({
                        "id": vol.id.to_string(),
                        "label": vol.label,
                        "purpose": vol.purpose.as_ref().map(|p| p.as_str()),
                    }));
                } else if let Some(ref p) = vol.purpose {
                    println!("Volume '{}' purpose set to: {}", vol.label, p);
                } else {
                    println!("Volume '{}' purpose cleared.", vol.label);
                }
                Ok(())
            }
            VolumeCommands::Remove { volume, apply } => {
                let catalog_root = dam::config::find_catalog_root()?;
                let config = CatalogConfig::load(&catalog_root)?;
                let service = AssetService::new(&catalog_root, cli.debug, &config.preview);

                let show_log = cli.log;
                let result = if show_log {
                    use dam::asset_service::CleanupStatus;
                    service.remove_volume(
                        &volume,
                        apply,
                        |path, status, elapsed| {
                            match status {
                                CleanupStatus::Stale => {
                                    let name = path.file_name()
                                        .and_then(|n| n.to_str())
                                        .unwrap_or_else(|| path.to_str().unwrap_or("?"));
                                    eprintln!("  {} — removed ({})", name, format_duration(elapsed));
                                }
                                CleanupStatus::OrphanedAsset => {
                                    let name = path.to_str().unwrap_or("?");
                                    eprintln!("  {} — orphaned asset removed ({})", name, format_duration(elapsed));
                                }
                                CleanupStatus::OrphanedPreview => {
                                    let name = path.file_name()
                                        .and_then(|n| n.to_str())
                                        .unwrap_or_else(|| path.to_str().unwrap_or("?"));
                                    eprintln!("  {} — orphaned preview removed ({})", name, format_duration(elapsed));
                                }
                                _ => {}
                            }
                        },
                    )?
                } else {
                    service.remove_volume(
                        &volume,
                        apply,
                        |_, _, _| {},
                    )?
                };

                if cli.json {
                    println!("{}", serde_json::to_string_pretty(&result)?);
                } else {
                    for err in &result.errors {
                        eprintln!("  {err}");
                    }

                    if apply {
                        let mut parts = vec![
                            format!("{} locations removed", result.locations_removed),
                            format!("{} recipes removed", result.recipes_removed),
                        ];
                        if result.removed_assets > 0 {
                            parts.push(format!("{} orphaned assets removed", result.removed_assets));
                        }
                        if result.removed_previews > 0 {
                            parts.push(format!("{} orphaned previews removed", result.removed_previews));
                        }
                        println!("Volume '{}' removed: {}", result.volume_label, parts.join(", "));
                    } else {
                        let mut parts = vec![
                            format!("{} locations", result.locations),
                            format!("{} recipes", result.recipes),
                        ];
                        if result.orphaned_assets > 0 {
                            parts.push(format!("{} orphaned assets", result.orphaned_assets));
                        }
                        if result.orphaned_previews > 0 {
                            parts.push(format!("{} orphaned previews", result.orphaned_previews));
                        }
                        println!("Volume '{}' would remove: {}", result.volume_label, parts.join(", "));
                        if result.locations > 0 || result.recipes > 0 {
                            println!("  Run with --apply to remove.");
                        }
                    }
                }
                Ok(())
            }
            VolumeCommands::Combine { source, target, apply } => {
                let catalog_root = dam::config::find_catalog_root()?;
                let config = CatalogConfig::load(&catalog_root)?;
                let service = AssetService::new(&catalog_root, cli.debug, &config.preview);

                let show_log = cli.log;
                let result = service.combine_volume(
                    &source,
                    &target,
                    apply,
                    |asset_id, elapsed| {
                        if show_log {
                            eprintln!("  {} — updated ({})", asset_id, format_duration(elapsed));
                        }
                    },
                )?;

                if cli.json {
                    println!("{}", serde_json::to_string_pretty(&result)?);
                } else {
                    for err in &result.errors {
                        eprintln!("  {err}");
                    }

                    if apply {
                        println!(
                            "Volume '{}' combined into '{}': {} locations moved, {} recipes moved ({} assets, prefix '{}')",
                            result.source_label,
                            result.target_label,
                            result.locations_moved,
                            result.recipes_moved,
                            result.assets_affected,
                            result.path_prefix,
                        );
                    } else {
                        println!(
                            "Would combine '{}' into '{}': {} locations, {} recipes ({} assets, prefix '{}')",
                            result.source_label,
                            result.target_label,
                            result.locations,
                            result.recipes,
                            result.assets_affected,
                            result.path_prefix,
                        );
                        if result.locations > 0 || result.recipes > 0 {
                            println!("  Run with --apply to combine.");
                        }
                    }
                }
                Ok(())
            }
        },
        Commands::Import {
            paths,
            volume,
            include,
            skip,
            add_tags,
            dry_run,
            auto_group,
        } => {
            use dam::asset_service::FileTypeFilter;

            let catalog_root = dam::config::find_catalog_root()?;
            let config = CatalogConfig::load(&catalog_root)?;
            let registry = DeviceRegistry::new(&catalog_root);

            // Build file type filter
            let mut filter = FileTypeFilter::default();

            // Check for conflicts: same group in both --include and --skip
            for group in &include {
                if skip.contains(group) {
                    anyhow::bail!(
                        "Group '{}' cannot be both included and skipped.",
                        group
                    );
                }
            }

            for group in &include {
                filter.include(group)?;
            }
            for group in &skip {
                filter.skip(group)?;
            }

            // Canonicalize input paths
            let canonical_paths: Vec<PathBuf> = paths
                .iter()
                .map(|p| {
                    std::fs::canonicalize(p)
                        .unwrap_or_else(|_| PathBuf::from(p))
                })
                .collect();

            if canonical_paths.is_empty() {
                anyhow::bail!("No paths specified for import.");
            }

            // Resolve volume: explicit --volume flag, or auto-detect from path
            let volume = if let Some(label) = &volume {
                registry.resolve_volume(label)?
            } else {
                registry.find_volume_for_path(&canonical_paths[0])?
            };

            // Merge config auto_tags with CLI --add-tag values
            let mut all_tags = config.import.auto_tags.clone();
            for tag in &add_tags {
                if !all_tags.contains(tag) {
                    all_tags.push(tag.clone());
                }
            }

            let service = AssetService::new(&catalog_root, cli.debug, &config.preview);
            let result = if cli.log {
                use dam::asset_service::FileStatus;
                service.import_with_callback(&canonical_paths, &volume, &filter, &config.import.exclude, &all_tags, dry_run, |path, status, elapsed| {
                    let label = match status {
                        FileStatus::Imported => "imported",
                        FileStatus::LocationAdded => "location added",
                        FileStatus::Skipped => "skipped",
                        FileStatus::RecipeAttached => "recipe",
                        FileStatus::RecipeUpdated => "recipe updated",
                    };
                    let name = path.file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or_else(|| path.to_str().unwrap_or("?"));
                    eprintln!("  {} — {} ({})", name, label, format_duration(elapsed));
                })?
            } else {
                service.import_with_callback(&canonical_paths, &volume, &filter, &config.import.exclude, &all_tags, dry_run, |_, _, _| {})?
            };

            // Post-import auto-group phase
            let auto_group_result = if auto_group
                && (result.imported > 0 || result.locations_added > 0)
            {
                use dam::catalog::Catalog;
                use std::path::Path;

                let catalog = Catalog::open(&catalog_root)?;
                let volume_id = volume.id.to_string();

                // Compute neighborhood prefixes: go up one level from each
                // imported directory to get the "session root"
                let session_roots: std::collections::HashSet<String> = result
                    .imported_directories
                    .iter()
                    .map(|dir| {
                        Path::new(dir)
                            .parent()
                            .map(|p| p.to_string_lossy().to_string())
                            .unwrap_or_default()
                    })
                    .collect();
                let prefixes: Vec<String> = session_roots.into_iter().collect();

                // Find all existing catalog assets in the neighborhood
                let neighbor_ids = catalog
                    .find_asset_ids_by_volume_and_path_prefixes(&volume_id, &prefixes)?;

                // Merge with newly imported asset IDs and deduplicate
                let mut all_ids: Vec<String> = result.new_asset_ids.clone();
                let existing: std::collections::HashSet<String> =
                    all_ids.iter().cloned().collect();
                for id in neighbor_ids {
                    if !existing.contains(&id) {
                        all_ids.push(id);
                    }
                }

                if all_ids.len() > 1 {
                    let engine = QueryEngine::new(&catalog_root);
                    let ag_result = engine.auto_group(&all_ids, dry_run)?;
                    if !ag_result.groups.is_empty() {
                        Some(ag_result)
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            };

            if cli.json {
                let mut json_val = serde_json::to_value(&result)?;
                if let Some(ref ag) = auto_group_result {
                    json_val["auto_group"] = serde_json::to_value(ag)?;
                }
                println!("{}", serde_json::to_string_pretty(&json_val)?);
            } else {
                let mut parts: Vec<String> = Vec::new();
                if result.imported > 0 {
                    parts.push(format!("{} imported", result.imported));
                }
                if result.skipped > 0 {
                    parts.push(format!("{} skipped", result.skipped));
                }
                if result.locations_added > 0 {
                    parts.push(format!("{} location(s) added", result.locations_added));
                }
                if result.recipes_attached > 0 {
                    parts.push(format!("{} recipe(s) attached", result.recipes_attached));
                }
                if result.recipes_updated > 0 {
                    parts.push(format!("{} recipe(s) updated", result.recipes_updated));
                }
                if result.previews_generated > 0 {
                    parts.push(format!("{} preview(s) generated", result.previews_generated));
                }
                if parts.is_empty() {
                    println!("Import: nothing to import");
                } else if dry_run {
                    println!("Dry run — would import: {}", parts.join(", "));
                } else {
                    println!("Import complete: {}", parts.join(", "));
                }

                if let Some(ref ag) = auto_group_result {
                    if cli.log {
                        for group in &ag.groups {
                            let short_id = &group.target_id[..8.min(group.target_id.len())];
                            eprintln!(
                                "  {} — {} asset(s) → target {short_id}",
                                group.stem,
                                group.asset_ids.len(),
                            );
                        }
                    }
                    println!(
                        "Auto-group: {} stem group(s), {} donor(s) {}, {} variant(s) moved",
                        ag.groups.len(),
                        ag.total_donors_merged,
                        if dry_run { "would merge" } else { "merged" },
                        ag.total_variants_moved,
                    );
                }
            }
            Ok(())
        }
        Commands::Search { query, format, quiet } => {
            use dam::format::{self, OutputFormat};

            let catalog_root = dam::config::find_catalog_root()?;
            let engine = QueryEngine::new(&catalog_root);
            let results = engine.search(&query)?;

            // Determine output format
            let output_format = if quiet {
                OutputFormat::Ids
            } else if let Some(fmt) = &format {
                format::parse_format(fmt).map_err(|e| anyhow::anyhow!(e))?
            } else if cli.json {
                OutputFormat::Json
            } else {
                OutputFormat::Short
            };

            let explicit_format = quiet || format.is_some();

            if results.is_empty() {
                match output_format {
                    OutputFormat::Json => println!("[]"),
                    _ => {
                        if !explicit_format {
                            println!("No results found.");
                        }
                    }
                }
            } else {
                match output_format {
                    OutputFormat::Ids => {
                        for row in &results {
                            println!("{}", row.asset_id);
                        }
                    }
                    OutputFormat::Short => {
                        for row in &results {
                            let display_name = row
                                .name
                                .as_deref()
                                .unwrap_or(&row.original_filename);
                            let short_id = &row.asset_id[..8];
                            println!(
                                "{}  {} [{}] ({}) — {}",
                                short_id, display_name, row.asset_type, row.display_format(), row.created_at
                            );
                        }
                        if !explicit_format {
                            println!("\n{} result(s)", results.len());
                        }
                    }
                    OutputFormat::Full => {
                        for row in &results {
                            let display_name = row
                                .name
                                .as_deref()
                                .unwrap_or(&row.original_filename);
                            let short_id = &row.asset_id[..8];
                            let tags = if row.tags.is_empty() {
                                String::new()
                            } else {
                                format!(" tags:{}", row.tags.join(","))
                            };
                            let desc = row.description.as_deref().unwrap_or("");
                            println!(
                                "{}  {} [{}] ({}) — {}{} {}",
                                short_id, display_name, row.asset_type, row.display_format(),
                                row.created_at, tags, desc
                            );
                        }
                        if !explicit_format {
                            println!("\n{} result(s)", results.len());
                        }
                    }
                    OutputFormat::Json => {
                        println!("{}", serde_json::to_string_pretty(&results)?);
                    }
                    OutputFormat::Template(ref tpl) => {
                        for row in &results {
                            let tags_str = row.tags.join(", ");
                            let desc = row.description.as_deref().unwrap_or("");
                            let label = row.color_label.as_deref().unwrap_or("");
                            let values = format::search_row_values(
                                &row.asset_id,
                                row.name.as_deref(),
                                &row.original_filename,
                                &row.asset_type,
                                row.display_format(),
                                &row.created_at,
                                &tags_str,
                                desc,
                                &row.content_hash,
                                label,
                            );
                            println!("{}", format::render_template(tpl, &values));
                        }
                    }
                }
            }
            Ok(())
        }
        Commands::Show { asset_id } => {
            let catalog_root = dam::config::find_catalog_root()?;
            let config = CatalogConfig::load(&catalog_root)?;
            let engine = QueryEngine::new(&catalog_root);
            let details = engine.show(&asset_id)?;

            if cli.json {
                println!("{}", serde_json::to_string_pretty(&details)?);
            } else {
                let preview_gen = dam::preview::PreviewGenerator::new(&catalog_root, cli.debug, &config.preview);

                println!("Asset: {}", details.id);
                if let Some(name) = &details.name {
                    println!("Name:  {name}");
                }
                println!("Type:  {}", details.asset_type);
                println!("Date:  {}", details.created_at);
                if !details.tags.is_empty() {
                    let display_tags: Vec<String> = details.tags.iter()
                        .map(|t| dam::tag_util::tag_storage_to_display(t))
                        .collect();
                    println!("Tags:  {}", display_tags.join(", "));
                }
                if let Some(rating) = details.rating {
                    let stars: String = (1..=5).map(|i| if i <= rating { '\u{2605}' } else { '\u{2606}' }).collect();
                    println!("Rating: {stars} ({rating}/5)");
                }
                if let Some(label) = &details.color_label {
                    println!("Label: {label}");
                }
                if let Some(desc) = &details.description {
                    println!("Description: {desc}");
                }

                // Show preview status for the best preview variant
                if let Some(idx) = dam::models::variant::best_preview_index_details(&details.variants) {
                    let v = &details.variants[idx];
                    let preview_path = preview_gen.preview_path(&v.content_hash);
                    if preview_gen.has_preview(&v.content_hash) {
                        println!("Preview: {}", preview_path.display());
                    } else {
                        println!("Preview: (none)");
                    }
                }

                if !details.variants.is_empty() {
                    println!("\nVariants:");
                    for v in &details.variants {
                        println!(
                            "  [{}] {} ({}, {})",
                            v.role,
                            v.original_filename,
                            v.format,
                            format_size(v.file_size)
                        );
                        println!("    Hash: {}", v.content_hash);
                        for loc in &v.locations {
                            println!(
                                "    Location: {} \u{2192} {}",
                                loc.volume_label, loc.relative_path
                            );
                        }
                        if !v.source_metadata.is_empty() {
                            let mut keys: Vec<&String> = v.source_metadata.keys().collect();
                            keys.sort();
                            for key in keys {
                                println!("    {}: {}", key, v.source_metadata[key]);
                            }
                        }
                    }
                }

                if !details.recipes.is_empty() {
                    println!("\nRecipes:");
                    for r in &details.recipes {
                        println!("  [{}] {} ({})", r.recipe_type, r.software, r.content_hash);
                        if let Some(path) = &r.relative_path {
                            println!("    Path: {path}");
                        }
                    }
                }
            }

            Ok(())
        }
        Commands::Tag { asset_id, remove, tags } => {
            let catalog_root = dam::config::find_catalog_root()?;
            let engine = QueryEngine::new(&catalog_root);
            // Convert user-facing tag input to storage form:
            // `/` → `|` (hierarchy), `\/` → `/` (literal slash)
            let storage_tags: Vec<String> = tags.iter()
                .map(|t| dam::tag_util::tag_input_to_storage(t))
                .collect();
            let result = engine.tag(&asset_id, &storage_tags, remove)?;

            if cli.json {
                println!("{}", serde_json::json!({
                    "changed": result.changed,
                    "tags": result.current_tags,
                }));
            } else {
                // Display tags with `/` for hierarchy, `\/` for literal slashes
                let display_changed: Vec<String> = result.changed.iter()
                    .map(|t| dam::tag_util::tag_storage_to_display(t))
                    .collect();
                let display_tags: Vec<String> = result.current_tags.iter()
                    .map(|t| dam::tag_util::tag_storage_to_display(t))
                    .collect();
                if !display_changed.is_empty() {
                    if remove {
                        println!("Removed tags: {}", display_changed.join(", "));
                    } else {
                        println!("Added tags: {}", display_changed.join(", "));
                    }
                }
                if display_tags.is_empty() {
                    println!("Tags: (none)");
                } else {
                    println!("Tags: {}", display_tags.join(", "));
                }
            }
            Ok(())
        }
        Commands::Edit { asset_id, name, clear_name, description, clear_description, rating, clear_rating, label, clear_label } => {
            use dam::query::EditFields;

            if name.is_none() && !clear_name && description.is_none() && !clear_description && rating.is_none() && !clear_rating && label.is_none() && !clear_label {
                anyhow::bail!("No edit flags provided. Use --name, --description, --rating, --label, --clear-name, --clear-description, --clear-rating, or --clear-label.");
            }

            // Validate label if provided
            let label_field = if clear_label {
                Some(None)
            } else if let Some(ref l) = label {
                match dam::models::Asset::validate_color_label(l) {
                    Ok(canonical) => Some(canonical),
                    Err(e) => anyhow::bail!(e),
                }
            } else {
                None
            };

            let fields = EditFields {
                name: if clear_name {
                    Some(None)
                } else {
                    name.map(Some)
                },
                description: if clear_description {
                    Some(None)
                } else {
                    description.map(Some)
                },
                rating: if clear_rating {
                    Some(None)
                } else {
                    rating.map(Some)
                },
                color_label: label_field,
            };

            let catalog_root = dam::config::find_catalog_root()?;
            let engine = QueryEngine::new(&catalog_root);
            let result = engine.edit(&asset_id, fields)?;

            if cli.json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                if let Some(name) = &result.name {
                    println!("Name: {name}");
                } else {
                    println!("Name: (none)");
                }
                if let Some(desc) = &result.description {
                    println!("Description: {desc}");
                } else {
                    println!("Description: (none)");
                }
                if let Some(r) = result.rating {
                    let stars: String = (1..=5).map(|i| if i <= r { '\u{2605}' } else { '\u{2606}' }).collect();
                    println!("Rating: {stars} ({r}/5)");
                } else {
                    println!("Rating: (none)");
                }
                if let Some(l) = &result.color_label {
                    println!("Label: {l}");
                } else {
                    println!("Label: (none)");
                }
            }
            Ok(())
        }
        Commands::Group { variant_hashes } => {
            let catalog_root = dam::config::find_catalog_root()?;
            let engine = QueryEngine::new(&catalog_root);
            let result = engine.group(&variant_hashes)?;

            if cli.json {
                println!("{}", serde_json::json!({
                    "target_id": result.target_id,
                    "variants_moved": result.variants_moved,
                    "donors_removed": result.donors_removed,
                }));
            } else {
                let short_id = &result.target_id[..8];
                println!(
                    "Grouped {} variant(s) into asset {short_id}",
                    variant_hashes.len()
                );
                if result.donors_removed > 0 {
                    println!("  Merged {} donor asset(s)", result.donors_removed);
                } else {
                    println!("  Already grouped (no changes)");
                }
            }
            Ok(())
        }
        Commands::AutoGroup { query, apply } => {
            let catalog_root = dam::config::find_catalog_root()?;
            let engine = QueryEngine::new(&catalog_root);

            // Search to get asset IDs, deduplicate (search returns one row per variant)
            let results = engine.search(query.as_deref().unwrap_or(""))?;
            let asset_ids: Vec<String> = {
                let mut seen = std::collections::HashSet::new();
                results
                    .iter()
                    .filter(|r| seen.insert(r.asset_id.clone()))
                    .map(|r| r.asset_id.clone())
                    .collect()
            };

            let result = engine.auto_group(&asset_ids, !apply)?;

            if cli.json {
                println!("{}", serde_json::to_string(&result)?);
            } else {
                if result.groups.is_empty() {
                    eprintln!("No groupable assets found");
                } else {
                    if cli.log {
                        for group in &result.groups {
                            let short_id = &group.target_id[..8.min(group.target_id.len())];
                            eprintln!(
                                "{} — {} asset(s) → target {short_id}",
                                group.stem,
                                group.asset_ids.len(),
                            );
                        }
                    }
                    println!(
                        "{} stem group(s), {} donor(s) {}, {} variant(s) moved",
                        result.groups.len(),
                        result.total_donors_merged,
                        if apply { "merged" } else { "would merge" },
                        result.total_variants_moved,
                    );
                }
                if !apply {
                    eprintln!("Dry run — use --apply to merge");
                }
            }
            Ok(())
        }
        Commands::Relocate {
            asset_id,
            volume,
            remove_source,
            dry_run,
        } => {
            let catalog_root = dam::config::find_catalog_root()?;
            let config = CatalogConfig::load(&catalog_root)?;
            let service = AssetService::new(&catalog_root, cli.debug, &config.preview);
            let result = service.relocate(&asset_id, &volume, remove_source, dry_run)?;

            if cli.json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                if dry_run {
                    println!("Dry run — no changes made:");
                }

                for action in &result.actions {
                    println!("  {action}");
                }

                let verb = if remove_source { "moved" } else { "copied" };
                let mut parts: Vec<String> = Vec::new();
                if result.copied > 0 {
                    parts.push(format!("{} {verb}", result.copied));
                }
                if result.skipped > 0 {
                    parts.push(format!("{} skipped", result.skipped));
                }
                if parts.is_empty() {
                    if result.actions.len() == 1 {
                        // The "already on target" message was printed above
                    } else {
                        println!("Relocate: nothing to do");
                    }
                } else {
                    println!("Relocate complete: {}", parts.join(", "));
                }
            }

            Ok(())
        }
        Commands::Verify { paths, volume, asset, include, skip } => {
            use dam::asset_service::FileTypeFilter;

            let catalog_root = dam::config::find_catalog_root()?;
            let config = CatalogConfig::load(&catalog_root)?;
            let service = AssetService::new(&catalog_root, cli.debug, &config.preview);

            // Build file type filter (same logic as import)
            let mut filter = FileTypeFilter::default();
            for group in &include {
                if skip.contains(group) {
                    anyhow::bail!(
                        "Group '{}' cannot be both included and skipped.",
                        group
                    );
                }
            }
            for group in &include {
                filter.include(group)?;
            }
            for group in &skip {
                filter.skip(group)?;
            }

            let canonical_paths: Vec<PathBuf> = paths
                .iter()
                .map(|p| {
                    std::fs::canonicalize(p)
                        .unwrap_or_else(|_| PathBuf::from(p))
                })
                .collect();

            let result = if cli.log {
                use dam::asset_service::VerifyStatus;
                service.verify(
                    &canonical_paths,
                    volume.as_deref(),
                    asset.as_deref(),
                    &filter,
                    |path, status, elapsed| {
                        let label = match status {
                            VerifyStatus::Ok => "OK",
                            VerifyStatus::Mismatch => "FAILED",
                            VerifyStatus::Modified => "MODIFIED",
                            VerifyStatus::Missing => "MISSING",
                            VerifyStatus::Skipped => "SKIPPED",
                            VerifyStatus::Untracked => "UNTRACKED",
                        };
                        let name = path.file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or_else(|| path.to_str().unwrap_or("?"));
                        eprintln!("  {} — {} ({})", name, label, format_duration(elapsed));
                    },
                )?
            } else {
                service.verify(
                    &canonical_paths,
                    volume.as_deref(),
                    asset.as_deref(),
                    &filter,
                    |_, _, _| {},
                )?
            };

            if cli.json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                // Print error details
                for err in &result.errors {
                    eprintln!("  {err}");
                }

                // Print summary
                let mut parts: Vec<String> = Vec::new();
                if result.verified > 0 {
                    parts.push(format!("{} verified", result.verified));
                }
                if result.modified > 0 {
                    parts.push(format!("{} modified", result.modified));
                }
                if result.failed > 0 {
                    parts.push(format!("{} FAILED", result.failed));
                }
                if result.skipped > 0 {
                    parts.push(format!("{} skipped", result.skipped));
                }
                if parts.is_empty() {
                    println!("Verify: nothing to verify");
                } else {
                    println!("Verify complete: {}", parts.join(", "));
                }
            }

            if result.failed > 0 {
                std::process::exit(1);
            }

            Ok(())
        }
        Commands::Sync { paths, volume, apply, remove_stale } => {
            if paths.is_empty() {
                anyhow::bail!("No paths specified for sync.");
            }
            if remove_stale && !apply {
                anyhow::bail!("--remove-stale requires --apply.");
            }

            let catalog_root = dam::config::find_catalog_root()?;
            let config = CatalogConfig::load(&catalog_root)?;
            let registry = DeviceRegistry::new(&catalog_root);

            let canonical_paths: Vec<PathBuf> = paths
                .iter()
                .map(|p| {
                    std::fs::canonicalize(p)
                        .unwrap_or_else(|_| PathBuf::from(p))
                })
                .collect();

            let volume = if let Some(label) = &volume {
                registry.resolve_volume(label)?
            } else {
                registry.find_volume_for_path(&canonical_paths[0])?
            };

            let service = AssetService::new(&catalog_root, cli.debug, &config.preview);
            let result = if cli.log {
                use dam::asset_service::SyncStatus;
                service.sync(
                    &canonical_paths,
                    &volume,
                    apply,
                    remove_stale,
                    &config.import.exclude,
                    |path, status, elapsed| {
                        let label = match status {
                            SyncStatus::Unchanged => "unchanged",
                            SyncStatus::Moved => "moved",
                            SyncStatus::New => "new",
                            SyncStatus::Modified => "modified",
                            SyncStatus::Missing => "missing",
                        };
                        let name = path.file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or_else(|| path.to_str().unwrap_or("?"));
                        eprintln!("  {} — {} ({})", name, label, format_duration(elapsed));
                    },
                )?
            } else {
                service.sync(
                    &canonical_paths,
                    &volume,
                    apply,
                    remove_stale,
                    &config.import.exclude,
                    |_, _, _| {},
                )?
            };

            if cli.json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                for err in &result.errors {
                    eprintln!("  {err}");
                }

                let mut parts: Vec<String> = Vec::new();
                if result.unchanged > 0 {
                    parts.push(format!("{} unchanged", result.unchanged));
                }
                if result.moved > 0 {
                    parts.push(format!("{} moved", result.moved));
                }
                if result.new_files > 0 {
                    parts.push(format!("{} new", result.new_files));
                }
                if result.modified > 0 {
                    parts.push(format!("{} modified", result.modified));
                }
                if result.missing > 0 {
                    parts.push(format!("{} missing", result.missing));
                }
                if result.stale_removed > 0 {
                    parts.push(format!("{} stale removed", result.stale_removed));
                }
                if parts.is_empty() {
                    println!("Sync: nothing to sync");
                } else {
                    println!("Sync complete: {}", parts.join(", "));
                }
                if result.new_files > 0 {
                    println!("  Tip: run 'dam import' to import new files.");
                }
            }

            Ok(())
        }
        Commands::Refresh { paths, volume, asset, dry_run, media } => {
            let catalog_root = dam::config::find_catalog_root()?;
            let config = CatalogConfig::load(&catalog_root)?;
            let registry = DeviceRegistry::new(&catalog_root);

            let canonical_paths: Vec<PathBuf> = paths
                .iter()
                .map(|p| {
                    std::fs::canonicalize(p)
                        .unwrap_or_else(|_| PathBuf::from(p))
                })
                .collect();

            // Resolve volume
            let resolved_volume = if let Some(label) = &volume {
                Some(registry.resolve_volume(label)?)
            } else if !canonical_paths.is_empty() {
                Some(registry.find_volume_for_path(&canonical_paths[0])?)
            } else {
                None
            };

            // Resolve asset ID prefix
            let resolved_asset_id = if let Some(prefix) = &asset {
                let catalog = Catalog::open(&catalog_root)?;
                match catalog.resolve_asset_id(prefix)? {
                    Some(id) => Some(id),
                    None => anyhow::bail!("No asset found matching '{prefix}'"),
                }
            } else {
                None
            };

            let service = AssetService::new(&catalog_root, cli.debug, &config.preview);
            let result = if cli.log {
                use dam::asset_service::RefreshStatus;
                service.refresh(
                    &canonical_paths,
                    resolved_volume.as_ref(),
                    resolved_asset_id.as_deref(),
                    dry_run,
                    media,
                    &config.import.exclude,
                    |path, status, elapsed| {
                        let label = match status {
                            RefreshStatus::Unchanged => "unchanged",
                            RefreshStatus::Refreshed => "refreshed",
                            RefreshStatus::Missing => "missing",
                            RefreshStatus::Offline => "offline",
                        };
                        let name = path.file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or_else(|| path.to_str().unwrap_or("?"));
                        eprintln!("  {} — {} ({})", name, label, format_duration(elapsed));
                    },
                )?
            } else {
                service.refresh(
                    &canonical_paths,
                    resolved_volume.as_ref(),
                    resolved_asset_id.as_deref(),
                    dry_run,
                    media,
                    &config.import.exclude,
                    |_, _, _| {},
                )?
            };

            if cli.json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                for err in &result.errors {
                    eprintln!("  {err}");
                }

                if dry_run {
                    eprint!("Dry run — ");
                }

                let mut parts: Vec<String> = Vec::new();
                if result.refreshed > 0 {
                    parts.push(format!("{} refreshed", result.refreshed));
                }
                if result.unchanged > 0 {
                    parts.push(format!("{} unchanged", result.unchanged));
                }
                if result.missing > 0 {
                    parts.push(format!("{} missing", result.missing));
                }
                if result.skipped > 0 {
                    parts.push(format!("{} skipped (offline)", result.skipped));
                }
                if parts.is_empty() {
                    println!("Refresh: nothing to check");
                } else {
                    println!("Refresh complete: {}", parts.join(", "));
                }
            }

            Ok(())
        }
        Commands::Cleanup { volume, list, apply } => {
            let catalog_root = dam::config::find_catalog_root()?;
            let config = CatalogConfig::load(&catalog_root)?;
            let service = AssetService::new(&catalog_root, cli.debug, &config.preview);

            let show_log = cli.log;
            let show_list = list;
            let result = if show_log || show_list {
                use dam::asset_service::CleanupStatus;
                service.cleanup(
                    volume.as_deref(),
                    apply,
                    |path, status, elapsed| {
                        match status {
                            CleanupStatus::Ok if show_log => {
                                let name = path.file_name()
                                    .and_then(|n| n.to_str())
                                    .unwrap_or_else(|| path.to_str().unwrap_or("?"));
                                eprintln!("  {} — ok ({})", name, format_duration(elapsed));
                            }
                            CleanupStatus::Stale => {
                                let name = path.file_name()
                                    .and_then(|n| n.to_str())
                                    .unwrap_or_else(|| path.to_str().unwrap_or("?"));
                                eprintln!("  {} — stale ({})", name, format_duration(elapsed));
                            }
                            CleanupStatus::Offline => {
                                let name = path.file_name()
                                    .and_then(|n| n.to_str())
                                    .unwrap_or_else(|| path.to_str().unwrap_or("?"));
                                eprintln!("  {} — offline", name);
                            }
                            CleanupStatus::OrphanedAsset => {
                                let name = path.to_str().unwrap_or("?");
                                eprintln!("  {} — orphaned asset removed ({})", name, format_duration(elapsed));
                            }
                            CleanupStatus::OrphanedPreview => {
                                let name = path.file_name()
                                    .and_then(|n| n.to_str())
                                    .unwrap_or_else(|| path.to_str().unwrap_or("?"));
                                eprintln!("  {} — orphaned preview removed ({})", name, format_duration(elapsed));
                            }
                            _ => {}
                        }
                    },
                )?
            } else {
                service.cleanup(
                    volume.as_deref(),
                    apply,
                    |_, _, _| {},
                )?
            };

            if cli.json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                for err in &result.errors {
                    eprintln!("  {err}");
                }

                if result.skipped_offline > 0 {
                    eprintln!(
                        "  Skipped {} offline volume(s).",
                        result.skipped_offline
                    );
                }

                if apply {
                    let mut parts = vec![
                        format!("{} checked", result.checked),
                        format!("{} stale", result.stale),
                        format!("{} removed", result.removed),
                    ];
                    if result.removed_assets > 0 {
                        parts.push(format!("{} orphaned assets removed", result.removed_assets));
                    }
                    if result.removed_previews > 0 {
                        parts.push(format!("{} orphaned previews removed", result.removed_previews));
                    }
                    println!("Cleanup complete: {}", parts.join(", "));
                } else {
                    let mut parts = vec![
                        format!("{} checked", result.checked),
                        format!("{} stale", result.stale),
                    ];
                    if result.orphaned_assets > 0 {
                        parts.push(format!("{} orphaned assets", result.orphaned_assets));
                    }
                    if result.orphaned_previews > 0 {
                        parts.push(format!("{} orphaned previews", result.orphaned_previews));
                    }
                    println!("Cleanup complete: {}", parts.join(", "));
                    if result.stale > 0 || result.orphaned_assets > 0 || result.orphaned_previews > 0 {
                        println!("  Run with --apply to remove stale records, orphaned assets, and previews.");
                    }
                }
            }

            Ok(())
        }
        Commands::Dedup { volume, prefer, filter_format, path, min_copies, apply } => {
            let catalog_root = dam::config::find_catalog_root()?;
            let config = CatalogConfig::load(&catalog_root)?;
            let service = AssetService::new(&catalog_root, cli.debug, &config.preview);

            // CLI --prefer overrides config [dedup] prefer
            let effective_prefer = prefer.or(config.dedup.prefer);

            let show_log = cli.log;
            let result = if show_log {
                use dam::asset_service::DedupStatus;
                service.dedup(
                    volume.as_deref(),
                    filter_format.as_deref(),
                    path.as_deref(),
                    effective_prefer.as_deref(),
                    min_copies,
                    apply,
                    |filename, path, status, vol_label| {
                        match status {
                            DedupStatus::Keep => {
                                eprintln!("  {} — keep ({}, {})", filename, path, vol_label);
                            }
                            DedupStatus::Remove => {
                                eprintln!("  {} — remove ({}, {})", filename, path, vol_label);
                            }
                            DedupStatus::Skipped => {
                                eprintln!("  {} — skipped, min-copies ({}, {})", filename, path, vol_label);
                            }
                        }
                    },
                )?
            } else {
                service.dedup(
                    volume.as_deref(),
                    filter_format.as_deref(),
                    path.as_deref(),
                    effective_prefer.as_deref(),
                    min_copies,
                    apply,
                    |_, _, _, _| {},
                )?
            };

            if cli.json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                for err in &result.errors {
                    eprintln!("  {err}");
                }

                if apply {
                    let recipe_msg = if result.recipes_removed > 0 {
                        format!(", {} recipes removed", result.recipes_removed)
                    } else {
                        String::new()
                    };
                    println!(
                        "Dedup: {} duplicate groups, {} locations removed, {} files deleted{} ({})",
                        result.duplicates_found,
                        result.locations_removed,
                        result.files_deleted,
                        recipe_msg,
                        format_size(result.bytes_freed),
                    );
                } else {
                    let recipe_msg = if result.recipes_removed > 0 {
                        format!(", {} recipe files", result.recipes_removed)
                    } else {
                        String::new()
                    };
                    println!(
                        "Dedup: {} duplicate groups, {} redundant locations{} ({} reclaimable)",
                        result.duplicates_found,
                        result.locations_to_remove,
                        recipe_msg,
                        format_size(result.bytes_freed),
                    );
                    if result.locations_to_remove > 0 {
                        println!("  Run with --apply to remove redundant files.");
                    }
                }
            }

            Ok(())
        }
        Commands::UpdateLocation { asset_id, from, to, volume } => {
            let catalog_root = dam::config::find_catalog_root()?;
            let config = CatalogConfig::load(&catalog_root)?;
            let service = AssetService::new(&catalog_root, cli.debug, &config.preview);

            let to_path = std::fs::canonicalize(&to)
                .unwrap_or_else(|_| PathBuf::from(&to));

            let result = service.update_location(
                &asset_id,
                &from,
                &to_path,
                volume.as_deref(),
            )?;

            if cli.json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                let short_id = &result.asset_id[..8];
                println!(
                    "Updated {} location for asset {short_id} on volume '{}'",
                    result.file_type, result.volume_label,
                );
                println!("  {} -> {}", result.old_path, result.new_path);
            }
            Ok(())
        }
        Commands::Duplicates { format, same_volume, cross_volume, volume, filter_format, path } => {
            use dam::format::{self, OutputFormat};

            if same_volume && cross_volume {
                anyhow::bail!("--same-volume and --cross-volume are mutually exclusive");
            }

            let catalog_root = dam::config::find_catalog_root()?;
            let catalog = Catalog::open(&catalog_root)?;

            // Resolve volume label → ID for the SQL filter (unknown volume → empty results)
            let vol_id = if let Some(ref label) = volume {
                let registry = DeviceRegistry::new(&catalog_root);
                match registry.resolve_volume(label) {
                    Ok(v) => Some(v.id.to_string()),
                    Err(_) => Some("nonexistent".to_string()),
                }
            } else {
                None
            };

            let mode = if same_volume { "same" } else if cross_volume { "cross" } else { "all" };
            let has_filters = vol_id.is_some() || filter_format.is_some() || path.is_some();

            let entries = if has_filters {
                catalog.find_duplicates_filtered(
                    mode,
                    vol_id.as_deref(),
                    filter_format.as_deref(),
                    path.as_deref(),
                )?
            } else if same_volume {
                catalog.find_duplicates_same_volume()?
            } else if cross_volume {
                catalog.find_duplicates_cross_volume()?
            } else {
                catalog.find_duplicates()?
            };

            let explicit_format = format.is_some();

            let output_format = if let Some(fmt) = &format {
                format::parse_format(fmt).map_err(|e| anyhow::anyhow!(e))?
            } else if cli.json {
                OutputFormat::Json
            } else {
                OutputFormat::Short
            };

            if entries.is_empty() {
                match output_format {
                    OutputFormat::Json => println!("[]"),
                    _ => {
                        if !explicit_format {
                            if same_volume {
                                println!("No same-volume duplicates found.");
                            } else if cross_volume {
                                println!("No cross-volume copies found.");
                            } else {
                                println!("No duplicates found.");
                            }
                        }
                    }
                }
            } else {
                match output_format {
                    OutputFormat::Ids => {
                        for entry in &entries {
                            println!("{}", entry.content_hash);
                        }
                    }
                    OutputFormat::Short | OutputFormat::Full => {
                        let is_full = matches!(output_format, OutputFormat::Full);
                        for entry in &entries {
                            let display_name = entry
                                .asset_name
                                .as_deref()
                                .unwrap_or(&entry.original_filename);
                            let vol_info = if entry.volume_count > 1 {
                                format!(" [{} volumes]", entry.volume_count)
                            } else {
                                String::new()
                            };
                            println!(
                                "{} ({}, {}){}",
                                display_name,
                                entry.format,
                                format_size(entry.file_size),
                                vol_info,
                            );
                            println!("  Hash: {}", entry.content_hash);
                            for loc in &entry.locations {
                                let purpose = loc
                                    .volume_purpose
                                    .as_deref()
                                    .map(|p| format!(" [{}]", p))
                                    .unwrap_or_default();
                                if is_full {
                                    let verified = loc
                                        .verified_at
                                        .as_deref()
                                        .unwrap_or("never");
                                    println!(
                                        "    {}{} \u{2192} {} (verified: {})",
                                        loc.volume_label, purpose, loc.relative_path, verified
                                    );
                                } else {
                                    println!(
                                        "    {}{} \u{2192} {}",
                                        loc.volume_label, purpose, loc.relative_path
                                    );
                                }
                            }
                            if !entry.same_volume_groups.is_empty() {
                                println!(
                                    "  \u{26a0} same-volume duplicates on: {}",
                                    entry.same_volume_groups.join(", ")
                                );
                            }
                        }
                        if !explicit_format {
                            let label = if same_volume {
                                "same-volume duplicate(s)"
                            } else if cross_volume {
                                "cross-volume copie(s)"
                            } else {
                                "file(s) with duplicate locations"
                            };
                            println!(
                                "\n{} {}",
                                entries.len(),
                                label,
                            );
                        }
                    }
                    OutputFormat::Json => {
                        println!("{}", serde_json::to_string_pretty(&entries)?);
                    }
                    OutputFormat::Template(ref tpl) => {
                        for entry in &entries {
                            let mut values = std::collections::HashMap::new();
                            values.insert("hash", entry.content_hash.clone());
                            values.insert("filename", entry.original_filename.clone());
                            values.insert("format", entry.format.clone());
                            values.insert("size", format_size(entry.file_size));
                            values.insert("name", entry.asset_name.as_deref()
                                .unwrap_or(&entry.original_filename).to_string());
                            let locs: Vec<String> = entry.locations.iter()
                                .map(|l| {
                                    let purpose = l.volume_purpose.as_deref()
                                        .map(|p| format!("[{}]", p))
                                        .unwrap_or_default();
                                    format!("{}{}:{}", l.volume_label, purpose, l.relative_path)
                                })
                                .collect();
                            values.insert("locations", locs.join(", "));
                            values.insert("volumes", entry.volume_count.to_string());
                            println!("{}", format::render_template(tpl, &values));
                        }
                    }
                }
            }
            Ok(())
        }
        Commands::GeneratePreviews { paths, asset, volume, include, skip, force, upgrade, smart } => {
            use dam::asset_service::FileTypeFilter;

            let catalog_root = dam::config::find_catalog_root()?;
            let config = CatalogConfig::load(&catalog_root)?;
            let preview_gen = dam::preview::PreviewGenerator::new(&catalog_root, cli.debug, &config.preview);
            let metadata_store = MetadataStore::new(&catalog_root);
            let registry = dam::device_registry::DeviceRegistry::new(&catalog_root);
            let catalog = dam::catalog::Catalog::open(&catalog_root)?;
            let volumes = registry.list()?;

            // Build file type filter
            let mut filter = FileTypeFilter::default();
            for group in &include {
                if skip.contains(group) {
                    anyhow::bail!(
                        "Group '{}' cannot be both included and skipped.",
                        group
                    );
                }
            }
            for group in &include {
                filter.include(group)?;
            }
            for group in &skip {
                filter.skip(group)?;
            }

            let mut generated = 0usize;
            let mut skipped = 0usize;
            let mut failed = 0usize;
            let mut upgraded = 0usize;

            // Canonicalize input paths
            let canonical_paths: Vec<PathBuf> = paths
                .iter()
                .map(|p| {
                    std::fs::canonicalize(p)
                        .unwrap_or_else(|_| PathBuf::from(p))
                })
                .collect();

            if !canonical_paths.is_empty() {
                // PATHS mode: resolve files, look up each in catalog
                let files = dam::asset_service::resolve_files(&canonical_paths, &config.import.exclude);

                for file_path in &files {
                    // Filter by extension
                    let ext = file_path
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("");
                    if !ext.is_empty() && !filter.is_importable(ext) {
                        continue;
                    }

                    // Find which volume this file is on
                    let vol = volumes.iter().find(|v| file_path.starts_with(&v.mount_point));
                    let vol = match vol {
                        Some(v) => v,
                        None => continue,
                    };

                    let relative_path = file_path
                        .strip_prefix(&vol.mount_point)
                        .unwrap_or(file_path);

                    // Look up variant in catalog
                    if let Some((content_hash, format)) = catalog.find_variant_by_volume_and_path(
                        &vol.id.to_string(),
                        &relative_path.to_string_lossy(),
                    )? {
                        let file_start = std::time::Instant::now();
                        let result = if smart {
                            if force { preview_gen.regenerate_smart(&content_hash, file_path, &format) }
                            else { preview_gen.generate_smart(&content_hash, file_path, &format) }
                        } else if force {
                            preview_gen.regenerate(&content_hash, file_path, &format)
                        } else {
                            preview_gen.generate(&content_hash, file_path, &format)
                        };
                        let file_elapsed = file_start.elapsed();
                        let name = file_path.file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or_else(|| file_path.to_str().unwrap_or("?"));
                        match result {
                            Ok(Some(_)) => {
                                generated += 1;
                                if cli.log { eprintln!("  {} — generated ({})", name, format_duration(file_elapsed)); }
                            }
                            Ok(None) => {
                                skipped += 1;
                                if cli.log { eprintln!("  {} — skipped ({})", name, format_duration(file_elapsed)); }
                            }
                            Err(e) => {
                                eprintln!("  Failed for {}: {e:#} ({})", file_path.display(), format_duration(file_elapsed));
                                failed += 1;
                            }
                        }
                    }
                }
            } else {
                // Catalog mode: iterate assets
                let volume_filter = match &volume {
                    Some(label) => Some(registry.resolve_volume(label)?),
                    None => None,
                };

                let assets = if let Some(asset_id) = &asset {
                    let engine = QueryEngine::new(&catalog_root);
                    let details = engine.show(asset_id)?;
                    let uuid: uuid::Uuid = details.id.parse()?;
                    vec![metadata_store.load(uuid)?]
                } else {
                    let summaries = metadata_store.list()?;
                    summaries
                        .iter()
                        .map(|s| metadata_store.load(s.id))
                        .collect::<Result<Vec<_>, _>>()?
                };

                for asset_data in &assets {
                    // Select the best variant for preview generation
                    let idx = dam::models::variant::best_preview_index(&asset_data.variants).unwrap_or(0);
                    if let Some(variant) = asset_data.variants.get(idx) {
                        // In --upgrade mode, skip assets where the best variant is already the first
                        if upgrade && idx == 0 {
                            skipped += 1;
                            continue;
                        }

                        // Apply format filter
                        let ext = &variant.format;
                        if !ext.is_empty() && !filter.is_importable(ext) {
                            skipped += 1;
                            continue;
                        }

                        // Try to find a reachable file for this variant
                        let source_path = variant.locations.iter().find_map(|loc| {
                            // Apply volume filter
                            if let Some(ref vf) = volume_filter {
                                if loc.volume_id != vf.id {
                                    return None;
                                }
                            }
                            volumes.iter().find_map(|v| {
                                if v.id == loc.volume_id && v.is_online {
                                    let full = v.mount_point.join(&loc.relative_path);
                                    if full.exists() { Some(full) } else { None }
                                } else {
                                    None
                                }
                            })
                        });

                        if let Some(path) = source_path {
                            let file_start = std::time::Instant::now();
                            let result = if smart {
                                if force || upgrade { preview_gen.regenerate_smart(&variant.content_hash, &path, &variant.format) }
                                else { preview_gen.generate_smart(&variant.content_hash, &path, &variant.format) }
                            } else if force || upgrade {
                                preview_gen.regenerate(&variant.content_hash, &path, &variant.format)
                            } else {
                                preview_gen.generate(&variant.content_hash, &path, &variant.format)
                            };
                            let file_elapsed = file_start.elapsed();
                            let name = path.file_name()
                                .and_then(|n| n.to_str())
                                .unwrap_or_else(|| path.to_str().unwrap_or("?"));
                            match result {
                                Ok(Some(_)) => {
                                    generated += 1;
                                    if upgrade { upgraded += 1; }
                                    if cli.log { eprintln!("  {} — {} ({})", name, if upgrade { "upgraded" } else { "generated" }, format_duration(file_elapsed)); }
                                }
                                Ok(None) => {
                                    skipped += 1;
                                    if cli.log { eprintln!("  {} — skipped ({})", name, format_duration(file_elapsed)); }
                                }
                                Err(e) => {
                                    eprintln!("  Failed for {}: {e:#} ({})", asset_data.id, format_duration(file_elapsed));
                                    failed += 1;
                                }
                            }
                        } else {
                            skipped += 1;
                        }
                    } else {
                        skipped += 1;
                    }
                }
            }

            let preview_label = if smart { "smart preview(s)" } else { "preview(s)" };
            if cli.json {
                let mut result = serde_json::json!({
                    "generated": generated,
                    "skipped": skipped,
                    "failed": failed,
                });
                if upgrade {
                    result["upgraded"] = serde_json::json!(upgraded);
                }
                if smart {
                    result["smart"] = serde_json::json!(true);
                }
                println!("{result}");
            } else {
                if upgrade && upgraded > 0 {
                    println!(
                        "Generated {} {} ({} upgraded), {} skipped, {} failed",
                        generated, preview_label, upgraded, skipped, failed
                    );
                } else {
                    println!(
                        "Generated {} {}, {} skipped, {} failed",
                        generated, preview_label, skipped, failed
                    );
                }
            }
            Ok(())
        }
        Commands::FixRoles { paths, volume, asset, apply } => {
            let catalog_root = dam::config::find_catalog_root()?;
            let config = CatalogConfig::load(&catalog_root)?;
            let service = AssetService::new(&catalog_root, cli.debug, &config.preview);

            let canonical_paths: Vec<PathBuf> = paths
                .iter()
                .map(|p| {
                    std::fs::canonicalize(p)
                        .unwrap_or_else(|_| PathBuf::from(p))
                })
                .collect();

            let show_log = cli.log;
            let result = service.fix_roles(
                &canonical_paths,
                volume.as_deref(),
                asset.as_deref(),
                apply,
                |name, status| {
                    if show_log {
                        let label = match status {
                            dam::asset_service::FixRolesStatus::AlreadyCorrect => "ok",
                            dam::asset_service::FixRolesStatus::Fixed => {
                                if apply { "fixed" } else { "would fix" }
                            }
                        };
                        eprintln!("  {} — {}", name, label);
                    }
                },
            )?;

            if cli.json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                for err in &result.errors {
                    eprintln!("  {err}");
                }

                if !apply && result.fixed > 0 {
                    eprint!("Dry run — ");
                }

                println!(
                    "Fix-roles: {} checked, {} fixed ({} variant(s)), {} already correct",
                    result.checked, result.fixed, result.variants_fixed, result.already_correct
                );

                if !apply && result.fixed > 0 {
                    println!("  Run with --apply to make changes.");
                }
            }

            Ok(())
        }
        Commands::FixDates { volume, asset, apply } => {
            let catalog_root = dam::config::find_catalog_root()?;
            let config = CatalogConfig::load(&catalog_root)?;
            let service = AssetService::new(&catalog_root, cli.debug, &config.preview);

            let show_log = cli.log;
            let result = service.fix_dates(
                volume.as_deref(),
                asset.as_deref(),
                apply,
                |name, status, detail| {
                    if show_log {
                        let label = match status {
                            dam::asset_service::FixDatesStatus::AlreadyCorrect => "ok".to_string(),
                            dam::asset_service::FixDatesStatus::NoDate => "no date available".to_string(),
                            dam::asset_service::FixDatesStatus::SkippedOffline => "skipped (volume offline)".to_string(),
                            dam::asset_service::FixDatesStatus::Fixed => {
                                let action = if apply { "fixed" } else { "would fix" };
                                if let Some(d) = detail {
                                    format!("{action}: {d}")
                                } else {
                                    action.to_string()
                                }
                            }
                        };
                        eprintln!("  {} — {}", name, label);
                    }
                },
            )?;

            if cli.json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                // Print offline volume warnings
                if !result.offline_volumes.is_empty() {
                    for vol_label in &result.offline_volumes {
                        eprintln!("Warning: volume '{}' is offline — cannot read files for date extraction", vol_label);
                    }
                }

                for err in &result.errors {
                    eprintln!("  {err}");
                }

                if !apply && result.fixed > 0 {
                    eprint!("Dry run — ");
                }

                let mut parts = vec![
                    format!("{} checked", result.checked),
                    format!("{} fixed", result.fixed),
                    format!("{} already correct", result.already_correct),
                ];
                if result.skipped_offline > 0 {
                    parts.push(format!("{} skipped (volume offline)", result.skipped_offline));
                }
                if result.no_date > 0 {
                    parts.push(format!("{} no date available", result.no_date));
                }

                println!("Fix-dates: {}", parts.join(", "));

                if !apply && result.fixed > 0 {
                    println!("  Run with --apply to make changes.");
                }
                if result.skipped_offline > 0 {
                    println!("  Mount offline volumes and re-run to fix remaining assets.");
                }
            }

            Ok(())
        }
        Commands::FixRecipes { volume, asset, apply } => {
            let catalog_root = dam::config::find_catalog_root()?;
            let config = CatalogConfig::load(&catalog_root)?;
            let service = AssetService::new(&catalog_root, cli.debug, &config.preview);

            let show_log = cli.log;
            let result = service.fix_recipes(
                volume.as_deref(),
                asset.as_deref(),
                apply,
                |name, status| {
                    if show_log {
                        let label = match status {
                            dam::asset_service::FixRecipesStatus::Reattached => {
                                if apply { "reattached" } else { "would reattach" }
                            }
                            dam::asset_service::FixRecipesStatus::NoParentFound => "no parent found",
                            dam::asset_service::FixRecipesStatus::Skipped => "skipped",
                        };
                        eprintln!("  {} — {}", name, label);
                    }
                },
            )?;

            if cli.json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                for err in &result.errors {
                    eprintln!("  {err}");
                }

                if !apply && result.reattached > 0 {
                    eprint!("Dry run — ");
                }

                let mut parts = vec![
                    format!("{} checked", result.checked),
                    format!("{} reattached", result.reattached),
                ];
                if result.no_parent > 0 {
                    parts.push(format!("{} no parent found", result.no_parent));
                }
                if result.skipped > 0 {
                    parts.push(format!("{} skipped", result.skipped));
                }

                println!("Fix-recipes: {}", parts.join(", "));

                if !apply && result.reattached > 0 {
                    println!("  Run with --apply to make changes.");
                }
            }

            Ok(())
        }
        Commands::RebuildCatalog => {
            let catalog_root = dam::config::find_catalog_root()?;
            let catalog = Catalog::open(&catalog_root)?;
            catalog.initialize()?;

            // Ensure volume rows exist so FK references work
            let registry = DeviceRegistry::new(&catalog_root);
            for volume in registry.list()? {
                catalog.ensure_volume(&volume)?;
            }

            // Clear existing data rows
            catalog.rebuild()?;

            // Sync sidecar files into catalog
            let store = MetadataStore::new(&catalog_root);
            let result = store.sync_to_catalog(&catalog)?;

            // Restore collections from YAML
            let collections_restored = {
                let col_file = dam::collection::load_yaml(&catalog_root).unwrap_or_default();
                if !col_file.collections.is_empty() {
                    let col_store = dam::collection::CollectionStore::new(catalog.conn());
                    col_store.import_from_yaml(&col_file).unwrap_or(0)
                } else {
                    0
                }
            };

            // Restore stacks from YAML
            let stacks_restored = {
                let stacks_file = dam::stack::load_yaml(&catalog_root).unwrap_or_default();
                if !stacks_file.stacks.is_empty() {
                    let stack_store = dam::stack::StackStore::new(catalog.conn());
                    stack_store.import_from_yaml(&stacks_file).unwrap_or(0)
                } else {
                    0
                }
            };

            if cli.json {
                println!("{}", serde_json::json!({
                    "synced": result.synced,
                    "errors": result.errors,
                    "collections_restored": collections_restored,
                    "stacks_restored": stacks_restored,
                }));
            } else {
                println!("Rebuild complete: {} asset(s) synced", result.synced);
                if collections_restored > 0 {
                    println!("  {} collection(s) restored", collections_restored);
                }
                if stacks_restored > 0 {
                    println!("  {} stack(s) restored", stacks_restored);
                }
                if result.errors > 0 {
                    println!("  {} error(s) encountered", result.errors);
                }
            }
            Ok(())
        }
        Commands::Serve { port, bind } => {
            let catalog_root = dam::config::find_catalog_root()?;
            let config = CatalogConfig::load(&catalog_root)?;
            let port = port.unwrap_or(config.serve.port);
            let bind = bind.unwrap_or_else(|| config.serve.bind.clone());
            let rt = tokio::runtime::Runtime::new()?;
            rt.block_on(dam::web::serve(catalog_root, &bind, port, config.preview, cli.log, config.dedup.prefer))?;
            Ok(())
        }
        Commands::Stats { types, volumes, tags, verified, all, limit } => {
            let catalog_root = dam::config::find_catalog_root()?;
            let catalog = Catalog::open(&catalog_root)?;
            let registry = DeviceRegistry::new(&catalog_root);
            let vol_list = registry.list()?;

            let volumes_info: Vec<(String, String, bool, Option<String>)> = vol_list
                .iter()
                .map(|v| (v.label.clone(), v.id.to_string(), v.is_online, v.purpose.as_ref().map(|p| p.as_str().to_string())))
                .collect();

            let show_types = types || all;
            let show_volumes = volumes || all;
            let show_tags = tags || all;
            let show_verified = verified || all;

            let stats = catalog.build_stats(
                &volumes_info,
                show_types,
                show_volumes,
                show_tags,
                show_verified,
                limit,
            )?;

            if cli.json {
                println!("{}", serde_json::to_string_pretty(&stats)?);
            } else {
                print_stats_human(&stats);
            }
            Ok(())
        }
        Commands::BackupStatus { query, at_risk, min_copies, volume, format, quiet } => {
            use dam::format::{self, OutputFormat};

            let catalog_root = dam::config::find_catalog_root()?;
            let catalog = Catalog::open(&catalog_root)?;
            let registry = DeviceRegistry::new(&catalog_root);
            let vol_list = registry.list()?;

            let volumes_info: Vec<(String, String, bool, Option<String>)> = vol_list
                .iter()
                .map(|v| (v.label.clone(), v.id.to_string(), v.is_online, v.purpose.as_ref().map(|p| p.as_str().to_string())))
                .collect();

            // Resolve target volume if specified
            let target_volume = if let Some(ref vol_label) = volume {
                Some(registry.resolve_volume(vol_label)?)
            } else {
                None
            };
            let target_volume_id = target_volume.as_ref().map(|v| v.id.to_string());

            // Scope: optional query → asset IDs
            let scope_ids: Option<Vec<String>> = if let Some(ref q) = query {
                let engine = QueryEngine::new(&catalog_root);
                let results = engine.search(q)?;
                let ids: Vec<String> = results.iter().map(|r| r.asset_id.clone()).collect();
                Some(ids)
            } else {
                None
            };
            let scope_refs = scope_ids.as_deref();

            // Determine mode: at-risk listing vs overview
            let listing_mode = at_risk || quiet || format.is_some();

            if listing_mode {
                // Get at-risk IDs
                let risk_ids = if let Some(ref tvid) = target_volume_id {
                    catalog.backup_status_missing_from_volume(scope_refs, tvid)?
                } else {
                    catalog.backup_status_at_risk_ids(scope_refs, min_copies)?
                };

                // Fetch full SearchRow data for output formatting
                let results = if risk_ids.is_empty() {
                    Vec::new()
                } else {
                    let opts = dam::catalog::SearchOptions {
                        collection_asset_ids: Some(&risk_ids),
                        per_page: u32::MAX,
                        ..Default::default()
                    };
                    catalog.search_paginated(&opts)?
                };

                let output_format = if quiet {
                    OutputFormat::Ids
                } else if let Some(fmt) = &format {
                    format::parse_format(fmt).map_err(|e| anyhow::anyhow!(e))?
                } else if cli.json {
                    OutputFormat::Json
                } else {
                    OutputFormat::Short
                };

                let explicit_format = quiet || format.is_some();

                if results.is_empty() {
                    match output_format {
                        OutputFormat::Json => println!("[]"),
                        _ => {
                            if !explicit_format {
                                println!("No at-risk assets found.");
                            }
                        }
                    }
                } else {
                    match output_format {
                        OutputFormat::Ids => {
                            for row in &results {
                                println!("{}", row.asset_id);
                            }
                        }
                        OutputFormat::Short => {
                            for row in &results {
                                let display_name = row.name.as_deref().unwrap_or(&row.original_filename);
                                let short_id = &row.asset_id[..8];
                                println!(
                                    "{}  {} [{}] ({}) — {}",
                                    short_id, display_name, row.asset_type, row.display_format(), row.created_at
                                );
                            }
                            if !explicit_format {
                                println!("\n{} at-risk asset(s)", results.len());
                            }
                        }
                        OutputFormat::Full => {
                            for row in &results {
                                let display_name = row.name.as_deref().unwrap_or(&row.original_filename);
                                let short_id = &row.asset_id[..8];
                                let tags = if row.tags.is_empty() {
                                    String::new()
                                } else {
                                    format!(" tags:{}", row.tags.join(","))
                                };
                                let desc = row.description.as_deref().unwrap_or("");
                                println!(
                                    "{}  {} [{}] ({}) — {}{} {}",
                                    short_id, display_name, row.asset_type, row.display_format(),
                                    row.created_at, tags, desc
                                );
                            }
                            if !explicit_format {
                                println!("\n{} at-risk asset(s)", results.len());
                            }
                        }
                        OutputFormat::Json => {
                            println!("{}", serde_json::to_string_pretty(&results)?);
                        }
                        OutputFormat::Template(ref tpl) => {
                            for row in &results {
                                let tags_str = row.tags.join(", ");
                                let desc = row.description.as_deref().unwrap_or("");
                                let label = row.color_label.as_deref().unwrap_or("");
                                let values = format::search_row_values(
                                    &row.asset_id,
                                    row.name.as_deref(),
                                    &row.original_filename,
                                    &row.asset_type,
                                    row.display_format(),
                                    &row.created_at,
                                    &tags_str,
                                    desc,
                                    &row.content_hash,
                                    label,
                                );
                                println!("{}", format::render_template(tpl, &values));
                            }
                        }
                    }
                }
            } else {
                // Overview mode
                let result = catalog.backup_status_overview(
                    scope_refs,
                    &volumes_info,
                    min_copies,
                    target_volume_id.as_deref(),
                )?;

                if cli.json {
                    println!("{}", serde_json::to_string_pretty(&result)?);
                } else {
                    print_backup_status_human(&result);
                }
            }
            Ok(())
        }
        Commands::Collection(cmd) => {
            let catalog_root = dam::config::find_catalog_root()?;
            let catalog = Catalog::open(&catalog_root)?;
            let store = dam::collection::CollectionStore::new(catalog.conn());
            match cmd {
                CollectionCommands::Create { name, description } => {
                    let col = store.create(&name, description.as_deref())?;
                    // Persist to YAML
                    let yaml = store.export_all()?;
                    dam::collection::save_yaml(&catalog_root, &yaml)?;
                    if cli.json {
                        println!("{}", serde_json::json!({
                            "id": col.id.to_string(),
                            "name": col.name,
                        }));
                    } else {
                        println!("Created collection '{}'", col.name);
                    }
                    Ok(())
                }
                CollectionCommands::List => {
                    let list = store.list()?;
                    if cli.json {
                        println!("{}", serde_json::to_string_pretty(&list)?);
                    } else if list.is_empty() {
                        println!("No collections.");
                    } else {
                        for c in &list {
                            let desc = c.description.as_deref().unwrap_or("");
                            if desc.is_empty() {
                                println!("  {} ({} assets)", c.name, c.asset_count);
                            } else {
                                println!("  {} ({} assets) — {}", c.name, c.asset_count, desc);
                            }
                        }
                    }
                    Ok(())
                }
                CollectionCommands::Show { name, format } => {
                    use dam::format::{self, OutputFormat};

                    let col = store.get_by_name(&name)?
                        .ok_or_else(|| anyhow::anyhow!("No collection named '{name}'"))?;

                    if col.asset_ids.is_empty() {
                        if cli.json {
                            println!("{}", serde_json::to_string_pretty(&col)?);
                        } else {
                            println!("Collection '{}' is empty.", name);
                        }
                        return Ok(());
                    }

                    // Search with collection filter
                    let engine = QueryEngine::new(&catalog_root);
                    let query_str = format!("collection:{}", name);
                    let results = engine.search(&query_str)?;

                    let output_format = if let Some(fmt) = &format {
                        format::parse_format(fmt).map_err(|e| anyhow::anyhow!(e))?
                    } else if cli.json {
                        OutputFormat::Json
                    } else {
                        OutputFormat::Short
                    };

                    let explicit_format = format.is_some();

                    if results.is_empty() {
                        match output_format {
                            OutputFormat::Json => println!("[]"),
                            _ => {
                                if !explicit_format {
                                    println!("Collection '{}': no matching assets.", name);
                                }
                            }
                        }
                    } else {
                        match output_format {
                            OutputFormat::Ids => {
                                for row in &results {
                                    println!("{}", row.asset_id);
                                }
                            }
                            OutputFormat::Short => {
                                if !explicit_format {
                                    println!("Collection '{}':", name);
                                }
                                for row in &results {
                                    let display_name = row.name.as_deref().unwrap_or(&row.original_filename);
                                    let short_id = &row.asset_id[..8];
                                    println!("  {}  {} [{}] ({})", short_id, display_name, row.asset_type, row.display_format());
                                }
                                if !explicit_format {
                                    println!("\n{} asset(s)", results.len());
                                }
                            }
                            OutputFormat::Full => {
                                if !explicit_format {
                                    println!("Collection '{}':", name);
                                }
                                for row in &results {
                                    let display_name = row.name.as_deref().unwrap_or(&row.original_filename);
                                    let short_id = &row.asset_id[..8];
                                    let tags = if row.tags.is_empty() {
                                        String::new()
                                    } else {
                                        format!(" tags:{}", row.tags.join(","))
                                    };
                                    println!("  {}  {} [{}] ({}){}", short_id, display_name, row.asset_type, row.display_format(), tags);
                                }
                                if !explicit_format {
                                    println!("\n{} asset(s)", results.len());
                                }
                            }
                            OutputFormat::Json => {
                                println!("{}", serde_json::to_string_pretty(&results)?);
                            }
                            OutputFormat::Template(ref tpl) => {
                                for row in &results {
                                    let tags_str = row.tags.join(", ");
                                    let desc = row.description.as_deref().unwrap_or("");
                                    let label = row.color_label.as_deref().unwrap_or("");
                                    let values = format::search_row_values(
                                        &row.asset_id,
                                        row.name.as_deref(),
                                        &row.original_filename,
                                        &row.asset_type,
                                        row.display_format(),
                                        &row.created_at,
                                        &tags_str,
                                        desc,
                                        &row.content_hash,
                                        label,
                                    );
                                    println!("{}", format::render_template(tpl, &values));
                                }
                            }
                        }
                    }
                    Ok(())
                }
                CollectionCommands::Add { name, asset_ids } => {
                    // Read from stdin if no IDs provided
                    let ids = if asset_ids.is_empty() {
                        use std::io::BufRead;
                        std::io::stdin().lock().lines()
                            .filter_map(|l| l.ok())
                            .map(|l| l.trim().to_string())
                            .filter(|l| !l.is_empty())
                            .collect()
                    } else {
                        asset_ids
                    };
                    if ids.is_empty() {
                        anyhow::bail!("No asset IDs specified.");
                    }
                    let added = store.add_assets(&name, &ids)?;
                    // Persist to YAML
                    let yaml = store.export_all()?;
                    dam::collection::save_yaml(&catalog_root, &yaml)?;
                    if cli.json {
                        println!("{}", serde_json::json!({
                            "added": added,
                            "collection": name,
                        }));
                    } else {
                        println!("Added {} asset(s) to '{}'", added, name);
                    }
                    Ok(())
                }
                CollectionCommands::Remove { name, asset_ids } => {
                    if asset_ids.is_empty() {
                        anyhow::bail!("No asset IDs specified.");
                    }
                    let removed = store.remove_assets(&name, &asset_ids)?;
                    // Persist to YAML
                    let yaml = store.export_all()?;
                    dam::collection::save_yaml(&catalog_root, &yaml)?;
                    if cli.json {
                        println!("{}", serde_json::json!({
                            "removed": removed,
                            "collection": name,
                        }));
                    } else {
                        println!("Removed {} asset(s) from '{}'", removed, name);
                    }
                    Ok(())
                }
                CollectionCommands::Delete { name } => {
                    store.delete(&name)?;
                    // Persist to YAML
                    let yaml = store.export_all()?;
                    dam::collection::save_yaml(&catalog_root, &yaml)?;
                    if cli.json {
                        println!("{}", serde_json::json!({
                            "status": "deleted",
                            "name": name,
                        }));
                    } else {
                        println!("Deleted collection '{name}'");
                    }
                    Ok(())
                }
            }
        }
        Commands::Stack(cmd) => {
            let catalog_root = dam::config::find_catalog_root()?;
            let catalog = Catalog::open(&catalog_root)?;
            let store = dam::stack::StackStore::new(catalog.conn());
            match cmd {
                StackCommands::Create { asset_ids } => {
                    if asset_ids.len() < 2 {
                        anyhow::bail!("A stack requires at least 2 assets");
                    }
                    let stack = store.create(&asset_ids)?;
                    let yaml = store.export_all()?;
                    dam::stack::save_yaml(&catalog_root, &yaml)?;
                    if cli.json {
                        println!("{}", serde_json::json!({
                            "id": stack.id.to_string(),
                            "member_count": stack.asset_ids.len(),
                            "pick": stack.asset_ids[0],
                        }));
                    } else {
                        println!("Created stack {} ({} assets, pick: {})",
                            &stack.id.to_string()[..8],
                            stack.asset_ids.len(),
                            &stack.asset_ids[0][..8.min(stack.asset_ids[0].len())]);
                    }
                    Ok(())
                }
                StackCommands::Add { reference, asset_ids } => {
                    let added = store.add(&reference, &asset_ids)?;
                    let yaml = store.export_all()?;
                    dam::stack::save_yaml(&catalog_root, &yaml)?;
                    if cli.json {
                        println!("{}", serde_json::json!({ "added": added }));
                    } else {
                        println!("Added {} asset(s) to stack", added);
                    }
                    Ok(())
                }
                StackCommands::Remove { asset_ids } => {
                    if asset_ids.is_empty() {
                        anyhow::bail!("No asset IDs specified.");
                    }
                    let removed = store.remove(&asset_ids)?;
                    let yaml = store.export_all()?;
                    dam::stack::save_yaml(&catalog_root, &yaml)?;
                    if cli.json {
                        println!("{}", serde_json::json!({ "removed": removed }));
                    } else {
                        println!("Removed {} asset(s) from stack(s)", removed);
                    }
                    Ok(())
                }
                StackCommands::Pick { asset_id } => {
                    store.set_pick(&asset_id)?;
                    let yaml = store.export_all()?;
                    dam::stack::save_yaml(&catalog_root, &yaml)?;
                    if cli.json {
                        println!("{}", serde_json::json!({ "pick": asset_id }));
                    } else {
                        println!("Set {} as stack pick", &asset_id[..8.min(asset_id.len())]);
                    }
                    Ok(())
                }
                StackCommands::Dissolve { asset_id } => {
                    store.dissolve(&asset_id)?;
                    let yaml = store.export_all()?;
                    dam::stack::save_yaml(&catalog_root, &yaml)?;
                    if cli.json {
                        println!("{}", serde_json::json!({ "status": "dissolved" }));
                    } else {
                        println!("Stack dissolved");
                    }
                    Ok(())
                }
                StackCommands::List => {
                    let list = store.list()?;
                    if cli.json {
                        println!("{}", serde_json::to_string_pretty(&list)?);
                    } else if list.is_empty() {
                        println!("No stacks.");
                    } else {
                        for s in &list {
                            let pick = s.pick_asset_id.as_deref().unwrap_or("?");
                            let short_id = &s.id[..8.min(s.id.len())];
                            let short_pick = &pick[..8.min(pick.len())];
                            println!("  {} ({} assets, pick: {})", short_id, s.member_count, short_pick);
                        }
                    }
                    Ok(())
                }
                StackCommands::Show { asset_id, format } => {
                    let (stack_id, members) = store.stack_for_asset(&asset_id)?
                        .ok_or_else(|| anyhow::anyhow!("Asset {asset_id} is not in a stack"))?;
                    if cli.json {
                        println!("{}", serde_json::json!({
                            "stack_id": stack_id,
                            "members": members,
                            "pick": members.first(),
                        }));
                    } else if let Some(ref fmt) = format {
                        if fmt == "ids" {
                            for id in &members {
                                println!("{}", id);
                            }
                        } else {
                            let short_sid = &stack_id[..8.min(stack_id.len())];
                            println!("Stack {}:", short_sid);
                            for (i, id) in members.iter().enumerate() {
                                let marker = if i == 0 { " [pick]" } else { "" };
                                println!("  {}{}", id, marker);
                            }
                        }
                    } else {
                        let short_sid = &stack_id[..8.min(stack_id.len())];
                        println!("Stack {}:", short_sid);
                        for (i, id) in members.iter().enumerate() {
                            let marker = if i == 0 { " [pick]" } else { "" };
                            println!("  {}{}", id, marker);
                        }
                    }
                    Ok(())
                }
                StackCommands::FromTag { pattern, remove_tags, apply } => {
                    let engine = QueryEngine::new(&catalog_root);
                    let result = engine.stack_from_tag(&pattern, remove_tags, apply, cli.log)?;

                    if cli.json {
                        println!("{}", serde_json::to_string_pretty(&result)?);
                    } else {
                        let mode = if result.dry_run { " (dry run)" } else { "" };
                        println!("Tags matched: {}{}", result.tags_matched, mode);
                        println!("Tags skipped: {}", result.tags_skipped);
                        println!("Stacks created: {}", result.stacks_created);
                        println!("Assets stacked: {}", result.assets_stacked);
                        println!("Assets already stacked (skipped): {}", result.assets_skipped);
                        if remove_tags {
                            println!("Tags removed: {}", result.tags_removed);
                        }
                    }
                    Ok(())
                }
            }
        }
        Commands::SavedSearch(cmd) => {
            let catalog_root = dam::config::find_catalog_root()?;
            match cmd {
                SavedSearchCommands::Save { name, query, sort, favorite } => {
                    let mut file = dam::saved_search::load(&catalog_root)?;
                    // Replace existing entry with same name, or append
                    let entry = dam::saved_search::SavedSearch {
                        name: name.clone(),
                        query,
                        sort,
                        favorite,
                    };
                    if let Some(existing) = file.searches.iter_mut().find(|s| s.name == name) {
                        *existing = entry;
                    } else {
                        file.searches.push(entry);
                    }
                    dam::saved_search::save(&catalog_root, &file)?;
                    if cli.json {
                        println!("{}", serde_json::json!({
                            "status": "saved",
                            "name": name,
                        }));
                    } else {
                        println!("Saved search '{name}'");
                    }
                    Ok(())
                }
                SavedSearchCommands::List => {
                    let file = dam::saved_search::load(&catalog_root)?;
                    if cli.json {
                        println!("{}", serde_json::to_string_pretty(&file.searches)?);
                    } else if file.searches.is_empty() {
                        println!("No saved searches.");
                    } else {
                        for ss in &file.searches {
                            let sort_info = ss.sort.as_deref().unwrap_or("date_desc");
                            let fav = if ss.favorite { " [*]" } else { "" };
                            println!("  {}{} — {} (sort: {})", ss.name, fav, ss.query, sort_info);
                        }
                    }
                    Ok(())
                }
                SavedSearchCommands::Run { name, format } => {
                    use dam::format::{self, OutputFormat};

                    let file = dam::saved_search::load(&catalog_root)?;
                    let ss = dam::saved_search::find_by_name(&file, &name)
                        .ok_or_else(|| anyhow::anyhow!("No saved search named '{name}'"))?;

                    let engine = QueryEngine::new(&catalog_root);
                    let results = engine.search(&ss.query)?;

                    let output_format = if let Some(fmt) = &format {
                        format::parse_format(fmt).map_err(|e| anyhow::anyhow!(e))?
                    } else if cli.json {
                        OutputFormat::Json
                    } else {
                        OutputFormat::Short
                    };

                    let explicit_format = format.is_some();

                    if results.is_empty() {
                        match output_format {
                            OutputFormat::Json => println!("[]"),
                            _ => {
                                if !explicit_format {
                                    println!("No results found.");
                                }
                            }
                        }
                    } else {
                        match output_format {
                            OutputFormat::Ids => {
                                for row in &results {
                                    println!("{}", row.asset_id);
                                }
                            }
                            OutputFormat::Short => {
                                for row in &results {
                                    let display_name = row
                                        .name
                                        .as_deref()
                                        .unwrap_or(&row.original_filename);
                                    let short_id = &row.asset_id[..8];
                                    println!(
                                        "{}  {} [{}] ({}) — {}",
                                        short_id, display_name, row.asset_type, row.display_format(), row.created_at
                                    );
                                }
                                if !explicit_format {
                                    println!("\n{} result(s)", results.len());
                                }
                            }
                            OutputFormat::Full => {
                                for row in &results {
                                    let display_name = row
                                        .name
                                        .as_deref()
                                        .unwrap_or(&row.original_filename);
                                    let short_id = &row.asset_id[..8];
                                    let tags = if row.tags.is_empty() {
                                        String::new()
                                    } else {
                                        format!(" tags:{}", row.tags.join(","))
                                    };
                                    let desc = row.description.as_deref().unwrap_or("");
                                    println!(
                                        "{}  {} [{}] ({}) — {}{} {}",
                                        short_id, display_name, row.asset_type, row.display_format(),
                                        row.created_at, tags, desc
                                    );
                                }
                                if !explicit_format {
                                    println!("\n{} result(s)", results.len());
                                }
                            }
                            OutputFormat::Json => {
                                println!("{}", serde_json::to_string_pretty(&results)?);
                            }
                            OutputFormat::Template(ref tpl) => {
                                for row in &results {
                                    let tags_str = row.tags.join(", ");
                                    let desc = row.description.as_deref().unwrap_or("");
                                    let label = row.color_label.as_deref().unwrap_or("");
                                    let values = format::search_row_values(
                                        &row.asset_id,
                                        row.name.as_deref(),
                                        &row.original_filename,
                                        &row.asset_type,
                                        row.display_format(),
                                        &row.created_at,
                                        &tags_str,
                                        desc,
                                        &row.content_hash,
                                        label,
                                    );
                                    println!("{}", format::render_template(tpl, &values));
                                }
                            }
                        }
                    }
                    Ok(())
                }
                SavedSearchCommands::Delete { name } => {
                    let mut file = dam::saved_search::load(&catalog_root)?;
                    let before = file.searches.len();
                    file.searches.retain(|s| s.name != name);
                    if file.searches.len() == before {
                        anyhow::bail!("No saved search named '{name}'");
                    }
                    dam::saved_search::save(&catalog_root, &file)?;
                    if cli.json {
                        println!("{}", serde_json::json!({
                            "status": "deleted",
                            "name": name,
                        }));
                    } else {
                        println!("Deleted saved search '{name}'");
                    }
                    Ok(())
                }
            }
        }
    })();

    if cli.timing {
        eprintln!("Elapsed: {}", format_duration(start.elapsed()));
    }

    if let Err(e) = result {
        eprintln!("Error: {e:#}");
        std::process::exit(1);
    }
}

fn format_duration(d: std::time::Duration) -> String {
    let total_millis = d.as_millis();
    let hours = total_millis / 3_600_000;
    let minutes = (total_millis % 3_600_000) / 60_000;
    let seconds = (total_millis % 60_000) / 1_000;
    let millis = total_millis % 1_000;

    if hours > 0 {
        format!("{hours}h {minutes:02}m {seconds:02}.{millis:03}s")
    } else if minutes > 0 {
        format!("{minutes}m {seconds:02}.{millis:03}s")
    } else {
        format!("{seconds}.{millis:03}s")
    }
}

fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;

    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

fn print_stats_human(stats: &dam::catalog::CatalogStats) {
    let o = &stats.overview;
    println!("Catalog Overview");
    println!("  Assets:    {}", o.assets);
    println!("  Variants:  {}", o.variants);
    println!("  Recipes:   {}", o.recipes);
    println!("  Volumes:   {} ({} online, {} offline)", o.volumes_total, o.volumes_online, o.volumes_offline);
    println!("  Total size: {}", format_size(o.total_size));

    if let Some(types) = &stats.types {
        println!("\nAsset Types");
        for t in &types.asset_types {
            println!("  {:<12} {:>6}  ({:.1}%)", t.asset_type, t.count, t.percentage);
        }
        if !types.variant_formats.is_empty() {
            println!("\nVariant Formats");
            for f in &types.variant_formats {
                println!("  {:<12} {:>6}", f.format, f.count);
            }
        }
        if !types.recipe_formats.is_empty() {
            println!("\nRecipe Formats");
            for f in &types.recipe_formats {
                println!("  {:<12} {:>6}", f.format, f.count);
            }
        }
    }

    if let Some(volumes) = &stats.volumes {
        println!("\nVolumes");
        for v in volumes {
            let status = if v.is_online { "online" } else { "offline" };
            if let Some(purpose) = &v.purpose {
                println!("  {} [{}] [{}]", v.label, status, purpose);
            } else {
                println!("  {} [{}]", v.label, status);
            }
            println!("    Assets: {}  Variants: {}  Recipes: {}", v.assets, v.variants, v.recipes);
            println!("    Size: {}  Directories: {}", format_size(v.size), v.directories);
            if !v.formats.is_empty() {
                println!("    Formats: {}", v.formats.join(", "));
            }
            println!("    Verified: {}/{} ({:.1}%)", v.verified_count, v.total_locations, v.verification_pct);
            if let Some(oldest) = &v.oldest_verified_at {
                println!("    Oldest verification: {oldest}");
            }
        }
    }

    if let Some(tags) = &stats.tags {
        println!("\nTags");
        println!("  Unique tags:     {}", tags.unique_tags);
        println!("  Tagged assets:   {}", tags.tagged_assets);
        println!("  Untagged assets: {}", tags.untagged_assets);
        if !tags.top_tags.is_empty() {
            println!("\n  Top Tags");
            for t in &tags.top_tags {
                println!("    {:<20} {:>4}", t.tag, t.count);
            }
        }
    }

    if let Some(v) = &stats.verified {
        println!("\nVerification");
        println!("  Total locations:    {}", v.total_locations);
        println!("  Verified:           {}", v.verified_locations);
        println!("  Unverified:         {}", v.unverified_locations);
        println!("  Coverage:           {:.1}%", v.coverage_pct);
        if let Some(oldest) = &v.oldest_verified_at {
            println!("  Oldest verified:    {oldest}");
        }
        if let Some(newest) = &v.newest_verified_at {
            println!("  Newest verified:    {newest}");
        }
        if !v.per_volume.is_empty() {
            println!("\n  Per Volume");
            for pv in &v.per_volume {
                let status = if pv.is_online { "online" } else { "offline" };
                let purpose_tag = pv.purpose.as_ref().map(|p| format!(" [{}]", p)).unwrap_or_default();
                println!(
                    "    {} [{}]{}: {}/{} ({:.1}%)",
                    pv.label, status, purpose_tag, pv.verified, pv.locations, pv.coverage_pct
                );
            }
        }
    }
}

fn print_backup_status_human(result: &dam::catalog::BackupStatusResult) {
    println!("Backup Status ({})", result.scope);
    println!("{}", "=".repeat(40));
    println!();
    println!("Total assets:          {:>8}", result.total_assets);
    println!("Total variants:        {:>8}", result.total_variants);
    println!("Total file locations:  {:>8}", result.total_file_locations);

    if !result.purpose_coverage.is_empty() {
        println!();
        println!("Coverage by volume purpose:");
        for pc in &result.purpose_coverage {
            // Capitalize first letter for display
            let display_purpose = {
                let mut chars = pc.purpose.chars();
                match chars.next() {
                    None => String::new(),
                    Some(c) => c.to_uppercase().to_string() + chars.as_str(),
                }
            };
            println!(
                "  {:<10} ({} volume{}):  {:>6} assets ({:.1}%)",
                display_purpose,
                pc.volume_count,
                if pc.volume_count == 1 { "" } else { "s" },
                pc.asset_count,
                pc.asset_percentage,
            );
        }
    }

    println!();
    println!("Volume distribution:");
    for bucket in &result.location_distribution {
        if bucket.asset_count == 0 {
            continue;
        }
        let label = match bucket.volume_count.as_str() {
            "0" => "0 volumes (orphaned):",
            "1" => "1 volume only:",
            "2" => "2 volumes:",
            _ => "3+ volumes:",
        };
        let at_risk = if bucket.volume_count == "0" || bucket.volume_count == "1" {
            "  <- AT RISK"
        } else {
            ""
        };
        println!("  {:<26} {:>6} assets{}", label, bucket.asset_count, at_risk);
    }

    if result.at_risk_count > 0 {
        println!();
        println!(
            "At-risk assets ({} on fewer than {} volume{}):",
            result.at_risk_count,
            result.min_copies,
            if result.min_copies == 1 { "" } else { "s" },
        );
        println!("  Use 'dam backup-status --at-risk' to list them");
        println!("  Use 'dam backup-status --at-risk -q' for asset IDs (pipeable)");
    } else {
        println!();
        println!(
            "All assets exist on {} or more volume{}. No at-risk assets.",
            result.min_copies,
            if result.min_copies == 1 { "" } else { "s" },
        );
    }

    if let Some(ref detail) = result.volume_detail {
        println!();
        let purpose_tag = detail.purpose.as_ref().map(|p| format!(" [{}]", p)).unwrap_or_default();
        println!("Volume detail: {}{}", detail.volume_label, purpose_tag);
        println!("  Present: {} / {} ({:.1}%)", detail.present_count, detail.total_scoped, detail.coverage_pct);
        println!("  Missing: {}", detail.missing_count);
    }

    if !result.volume_gaps.is_empty() {
        println!();
        println!("Volume gaps:");
        for gap in &result.volume_gaps {
            let purpose_tag = gap.purpose.as_ref().map(|p| format!(" [{}]", p)).unwrap_or_default();
            println!("  {}{}:  missing {} assets", gap.volume_label, purpose_tag, gap.missing_count);
        }
    }
}
