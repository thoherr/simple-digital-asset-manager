use std::path::PathBuf;

use clap::{Parser, Subcommand};
use maki::asset_service::AssetService;
use maki::catalog::Catalog;
use maki::config::CatalogConfig;
use maki::device_registry::DeviceRegistry;
use maki::metadata_store::MetadataStore;
use maki::query::QueryEngine;

#[derive(Parser)]
#[command(name = "maki", about = "Media Asset Keeper & Indexer",
    version = if cfg!(feature = "pro") {
        concat!(env!("CARGO_PKG_VERSION"), " Pro")
    } else {
        env!("CARGO_PKG_VERSION")
    },
    after_help = "Use maki --help for grouped overview, or maki <command> --help for details."
)]
struct Cli {
    /// Output machine-readable JSON (valid JSON on stdout, messages on stderr)
    #[arg(long, global = true, display_order = 30)]
    json: bool,

    /// Log per-file progress to stderr (all multi-file commands)
    #[arg(short = 'l', long = "log", global = true, display_order = 40)]
    log: bool,

    /// Show operational decisions and program flow
    #[arg(short = 'v', long = "verbose", global = true, display_order = 41)]
    verbose: bool,

    /// Show debug output from external tools (ffmpeg, dcraw, curl)
    #[arg(short = 'd', long = "debug", global = true, display_order = 42)]
    debug: bool,

    /// Show elapsed time after command execution
    #[arg(short = 't', long = "time", global = true, display_order = 43)]
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

        /// Use a named import profile from maki.toml [import.profiles.<name>]
        #[arg(long, display_order = 11)]
        profile: Option<String>,

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

        /// Generate smart previews (2560px) alongside regular previews
        #[arg(long, display_order = 22)]
        smart: bool,

        /// Generate image embeddings for visual similarity search (requires --features ai)
        #[cfg(feature = "ai")]
        #[arg(long, display_order = 23)]
        embed: bool,

        /// Generate VLM descriptions for imported assets (requires running Ollama)
        #[cfg(feature = "pro")]
        #[arg(long, display_order = 24)]
        describe: bool,
    },

    /// Delete assets from the catalog
    #[command(display_order = 11)]
    Delete {
        /// Asset IDs to delete (reads from stdin if empty)
        asset_ids: Vec<String>,
        /// Execute deletion (default: report-only)
        #[arg(long)]
        apply: bool,
        /// Also delete physical files from disk (requires --apply)
        #[arg(long)]
        remove_files: bool,
    },

    /// Add, remove, or rename tags
    #[command(display_order = 12, args_conflicts_with_subcommands = true)]
    Tag {
        /// Asset ID
        asset_id: Option<String>,

        /// Remove the specified tags instead of adding them
        #[arg(long)]
        remove: bool,

        /// Tags to add or remove
        tags: Vec<String>,

        #[command(subcommand)]
        subcmd: Option<TagCommands>,
    },

    /// Edit asset metadata (name, description, rating)
    #[command(display_order = 13)]
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

        /// Remove all tags
        #[arg(long)]
        clear_tags: bool,

        /// Set date (YYYY, YYYY-MM, YYYY-MM-DD, or ISO 8601)
        #[arg(long)]
        date: Option<String>,

        /// Reset date to now
        #[arg(long)]
        clear_date: bool,

        /// Set variant role (original, alternate, processed, export, sidecar)
        #[arg(long)]
        role: Option<String>,

        /// Variant content hash (required with --role)
        #[arg(long)]
        variant: Option<String>,
    },

    /// Group variants into one asset
    #[command(display_order = 14)]
    Group {
        /// Content hashes of variants to group
        variant_hashes: Vec<String>,
    },

    /// Split variants out of an asset into new standalone assets
    #[command(display_order = 15)]
    Split {
        /// Asset ID to split
        asset_id: String,
        /// Content hashes of variants to extract
        variant_hashes: Vec<String>,
    },

    /// Auto-group assets by filename stem
    #[command(display_order = 16)]
    AutoGroup {
        /// Search query to scope assets (same syntax as maki search)
        query: Option<String>,
        /// Apply grouping (default: report-only)
        #[arg(long)]
        apply: bool,
        /// Group across all directories (DANGEROUS: may merge unrelated assets with same filename)
        #[arg(long)]
        global: bool,
    },

    /// Auto-tag assets using AI vision model (requires --features ai)
    #[cfg(feature = "ai")]
    #[command(display_order = 17)]
    AutoTag {
        /// Search query to scope assets (same syntax as `maki search`)
        #[arg(display_order = 1)]
        query: Option<String>,

        /// Process a specific asset (ID or prefix)
        #[arg(long, display_order = 2)]
        asset: Option<String>,

        /// Limit to assets on a specific volume
        #[arg(long, display_order = 3)]
        volume: Option<String>,

        /// AI model to use (default from maki.toml or siglip-vit-b16-256)
        #[arg(long, display_order = 4)]
        model: Option<String>,

        /// Confidence threshold (0.0–1.0, default from maki.toml or 0.1)
        #[arg(long, display_order = 10)]
        threshold: Option<f32>,

        /// Path to custom labels file (one label per line)
        #[arg(long, display_order = 11)]
        labels: Option<String>,

        /// Apply suggested tags (default: report-only)
        #[arg(long, display_order = 20)]
        apply: bool,

        /// Download the AI model
        #[arg(long, display_order = 30)]
        download: bool,

        /// Remove cached AI model files
        #[arg(long, display_order = 31)]
        remove_model: bool,

        /// Show available AI models
        #[arg(long, display_order = 32)]
        list_models: bool,

        /// Print the active label list (default or from --labels / config)
        #[arg(long, display_order = 33)]
        list_labels: bool,

        /// Find visually similar assets (by asset ID)
        #[arg(long, display_order = 40)]
        similar: Option<String>,

        /// Asset IDs (for shell variable expansion)
        #[arg(hide = true, trailing_var_arg = true)]
        asset_ids: Vec<String>,
    },

    /// Generate embeddings for visual similarity search (requires --features ai)
    #[cfg(feature = "ai")]
    #[command(display_order = 18)]
    Embed {
        /// Search query to scope assets (same syntax as `maki search`)
        #[arg(display_order = 1)]
        query: Option<String>,

        /// Process a specific asset (ID or prefix)
        #[arg(long, display_order = 2)]
        asset: Option<String>,

        /// Limit to assets on a specific volume
        #[arg(long, display_order = 3)]
        volume: Option<String>,

        /// AI model to use (default from maki.toml or siglip-vit-b16-256)
        #[arg(long, display_order = 4)]
        model: Option<String>,

        /// Re-generate embeddings that already exist for the active model. Not needed when switching models — embeddings are stored per (asset_id, model_id), so a model switch only generates the missing ones.
        #[arg(long, display_order = 10)]
        force: bool,

        /// Export all embeddings from SQLite to binary files (no scope required)
        #[arg(long, display_order = 11)]
        export: bool,

        /// Asset IDs (for shell variable expansion)
        #[arg(hide = true, trailing_var_arg = true)]
        asset_ids: Vec<String>,
    },

    /// Face detection and recognition (requires --features ai)
    #[cfg(feature = "ai")]
    #[command(subcommand, display_order = 19)]
    Faces(FacesCommands),

    /// Generate image descriptions using a vision-language model (VLM)
    #[cfg(feature = "pro")]
    #[command(display_order = 16)]
    Describe {
        /// Search query to scope assets (same syntax as `maki search`)
        #[arg(display_order = 1)]
        query: Option<String>,

        /// Process a specific asset (ID or prefix)
        #[arg(long, display_order = 2)]
        asset: Option<String>,

        /// Limit to assets on a specific volume
        #[arg(long, display_order = 3)]
        volume: Option<String>,

        /// VLM model name (default from maki.toml or qwen2.5vl:3b)
        #[arg(long, display_order = 4)]
        model: Option<String>,

        /// VLM server endpoint (default from maki.toml or http://localhost:11434)
        #[arg(long, display_order = 5)]
        endpoint: Option<String>,

        /// Custom prompt for the VLM
        #[arg(long, display_order = 6)]
        prompt: Option<String>,

        /// Maximum tokens in VLM response
        #[arg(long, display_order = 7)]
        max_tokens: Option<u32>,

        /// Request timeout in seconds (default from maki.toml or 300)
        #[arg(long, display_order = 8)]
        timeout: Option<u32>,

        /// What to generate: describe (prose), tags (structured), both
        #[arg(long, display_order = 9, default_value = "describe")]
        mode: String,

        /// Sampling temperature (0.0 = deterministic, 1.0+ = creative)
        #[arg(long, display_order = 10)]
        temperature: Option<f32>,

        /// Context window size for Ollama (num_ctx)
        #[arg(long, display_order = 11)]
        num_ctx: Option<u32>,

        /// Top-p (nucleus) sampling threshold
        #[arg(long, display_order = 12)]
        top_p: Option<f32>,

        /// Top-k sampling: limit to k most likely tokens
        #[arg(long, display_order = 13)]
        top_k: Option<u32>,

        /// Repeat penalty (1.0 = no penalty)
        #[arg(long, display_order = 14)]
        repeat_penalty: Option<f32>,

        /// Apply descriptions to assets (default: report-only)
        #[arg(long, display_order = 20)]
        apply: bool,

        /// Overwrite existing descriptions
        #[arg(long, display_order = 21)]
        force: bool,

        /// Show what would happen without calling the VLM
        #[arg(long, display_order = 22)]
        dry_run: bool,

        /// Check VLM endpoint connectivity
        #[arg(long, display_order = 30)]
        check: bool,

        /// Asset IDs (for shell variable expansion)
        #[arg(hide = true, trailing_var_arg = true)]
        asset_ids: Vec<String>,
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
        /// Free-text keywords and filters. See 'maki help search' or the manual for the
        /// full list. Common filters: type:, tag:, format:, rating:, label:, camera:, lens:,
        /// iso:, focal:, f:, width:, height:, path:, collection:, date:, volume:, copies:,
        /// variants:, scattered:, duration:, codec:, stacked:, geo:, faces: (Pro), person: (Pro), similar: (Pro),
        /// text: (Pro), embed: (Pro)
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

        /// Show only file locations (one per line: volume:path)
        #[arg(long)]
        locations: bool,
    },

    /// Open the asset's preview in the OS default image viewer
    #[command(display_order = 32)]
    Preview {
        /// Asset ID (or prefix)
        asset_id: String,
    },

    /// Find duplicate files
    #[command(display_order = 33)]
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

    /// Generate a PDF contact sheet from search results
    #[command(display_order = 35)]
    ContactSheet {
        /// Search query (same syntax as maki search)
        query: String,

        /// Output PDF file path
        output: String,

        /// Layout preset: dense, standard, large
        #[arg(long, default_value = "standard")]
        layout: String,

        /// Number of columns (overrides layout preset)
        #[arg(long)]
        columns: Option<u32>,

        /// Number of rows per page (overrides layout preset)
        #[arg(long)]
        rows: Option<u32>,

        /// Paper size: a4, letter, a3
        #[arg(long, default_value = "a4")]
        paper: String,

        /// Use landscape orientation
        #[arg(long)]
        landscape: bool,

        /// Title printed on first page header
        #[arg(long)]
        title: Option<String>,

        /// Comma-separated metadata fields below each thumbnail
        #[arg(long)]
        fields: Option<String>,

        /// Override sort order: date, name, rating, filename
        #[arg(long)]
        sort: Option<String>,

        /// Use regular previews (800px) instead of smart previews (default: smart with fallback)
        #[arg(long)]
        no_smart: bool,

        /// Group by field with section headers: date, volume, collection, label
        #[arg(long)]
        group_by: Option<String>,

        /// Page margin in mm
        #[arg(long)]
        margin: Option<f32>,

        /// Color label rendering style: border, dot, none
        #[arg(long)]
        label_style: Option<String>,

        /// JPEG quality for page images (1-100)
        #[arg(long)]
        quality: Option<u8>,

        /// Copyright text displayed in the center of each page footer
        #[arg(long)]
        copyright: Option<String>,

        /// Report page count and asset count without generating
        #[arg(long)]
        dry_run: bool,
    },

    /// Export files matching a search query to a directory
    #[command(display_order = 34)]
    Export {
        /// Search query (same syntax as maki search)
        query: String,

        /// Target directory (created if needed), or ZIP file path with --zip
        target: String,

        /// Layout: flat (default) or mirror (preserves directory structure)
        #[arg(long, default_value = "flat")]
        layout: String,

        /// Create symlinks instead of copies
        #[arg(long)]
        symlink: bool,

        /// Export all variants (default: best variant only)
        #[arg(long)]
        all_variants: bool,

        /// Include recipe/sidecar files (.xmp, .cos, etc.)
        #[arg(long)]
        include_sidecars: bool,

        /// Show what would be exported without writing files
        #[arg(long)]
        dry_run: bool,

        /// Re-copy even if target already has matching content
        #[arg(long)]
        overwrite: bool,

        /// Write a ZIP archive instead of copying files to a directory
        #[arg(long)]
        zip: bool,
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
        /// Search query to scope the asset universe (same syntax as maki search)
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
    #[command(display_order = 55)]
    Serve {
        /// Port to listen on (default: 8080, or from maki.toml [serve] port)
        #[arg(long, display_order = 10)]
        port: Option<u16>,

        /// Address to bind to (default: 127.0.0.1, or from maki.toml [serve] bind)
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

        /// Skip files verified within this many days (overrides maki.toml [verify] max_age_days)
        #[arg(long, display_order = 14)]
        max_age: Option<u64>,

        /// Verify all files regardless of last verification time
        #[arg(long, display_order = 15)]
        force: bool,
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

        /// Clear and re-extract all metadata from source files (XMP + EXIF)
        #[arg(long, display_order = 22)]
        reimport: bool,

        /// Re-extract only EXIF/source metadata from media files, leaving tags/description/rating/label untouched
        #[arg(long, display_order = 23)]
        exif_only: bool,
    },

    /// Bidirectional metadata sync: read external XMP changes and write back pending DAM edits
    #[cfg(feature = "pro")]
    #[command(name = "sync-metadata", display_order = 42)]
    SyncMetadata {
        /// Search query to select assets (same syntax as `maki search`)
        #[arg(display_order = 1)]
        query: Option<String>,

        /// Limit to a specific volume
        #[arg(long, display_order = 10)]
        volume: Option<String>,

        /// Limit to a specific asset
        #[arg(long, display_order = 11)]
        asset: Option<String>,

        /// Preview what would change without applying
        #[arg(long, display_order = 20)]
        dry_run: bool,

        /// Also re-extract embedded XMP from JPEG/TIFF media files
        #[arg(long, display_order = 21)]
        media: bool,

        /// Asset IDs (for shell variable expansion)
        #[arg(hide = true, trailing_var_arg = true)]
        asset_ids: Vec<String>,
    },

    /// Write back pending metadata changes to XMP recipe files
    #[cfg(feature = "pro")]
    #[command(display_order = 42)]
    Writeback {
        /// Search query to select assets (same syntax as `maki search`)
        #[arg(display_order = 1)]
        query: Option<String>,

        /// Limit to a specific volume
        #[arg(long, display_order = 10)]
        volume: Option<String>,

        /// Limit to a specific asset
        #[arg(long, display_order = 11)]
        asset: Option<String>,

        /// Write back all XMP recipes (not just pending ones)
        #[arg(long, display_order = 12)]
        all: bool,

        /// Preview what would be written without modifying files
        #[arg(long, display_order = 20)]
        dry_run: bool,

        /// Asset IDs (for shell variable expansion)
        #[arg(hide = true, trailing_var_arg = true)]
        asset_ids: Vec<String>,
    },

    /// Remove stale file location records (files no longer on disk)
    #[command(display_order = 43)]
    Cleanup {
        /// Limit to a specific volume
        #[arg(long, display_order = 10)]
        volume: Option<String>,

        /// Limit to files under this path prefix (relative to volume root)
        #[arg(long, display_order = 11)]
        path: Option<String>,

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
        /// Asset IDs (reads from stdin if empty and no --query)
        asset_ids: Vec<String>,

        /// Target volume label or ID
        #[arg(long)]
        target: Option<String>,

        /// Search query to select assets for batch relocate
        #[arg(long, display_order = 1)]
        query: Option<String>,

        /// Delete source files after successful copy and verification
        #[arg(long)]
        remove_source: bool,

        /// Create XMP sidecar files at the destination for assets with metadata but no existing recipe
        #[arg(long)]
        create_sidecars: bool,

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

        /// Also generate smart previews (high-resolution, 2560px) alongside thumbnails
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
        /// Search query to select assets (same syntax as `maki search`)
        #[arg(display_order = 1)]
        query: Option<String>,

        /// Limit to a specific volume
        #[arg(long, display_order = 10)]
        volume: Option<String>,

        /// Fix only a specific asset
        #[arg(long, display_order = 11)]
        asset: Option<String>,

        /// Apply changes (default: report-only dry run)
        #[arg(long, display_order = 20)]
        apply: bool,

        /// Asset IDs (for shell variable expansion)
        #[arg(hide = true, trailing_var_arg = true)]
        asset_ids: Vec<String>,
    },

    /// Re-attach recipe files that were imported as standalone assets
    #[command(display_order = 50)]
    FixRecipes {
        /// Search query to select assets (same syntax as `maki search`)
        #[arg(display_order = 1)]
        query: Option<String>,

        /// Limit to a specific volume
        #[arg(long, display_order = 10)]
        volume: Option<String>,

        /// Fix only a specific asset
        #[arg(long, display_order = 11)]
        asset: Option<String>,

        /// Apply changes (default: report-only dry run)
        #[arg(long, display_order = 20)]
        apply: bool,

        /// Asset IDs (for shell variable expansion)
        #[arg(hide = true, trailing_var_arg = true)]
        asset_ids: Vec<String>,
    },

    /// Create XMP sidecar files for assets with metadata but no existing recipe
    #[command(name = "create-sidecars", display_order = 50)]
    CreateSidecars {
        /// Search query to select assets (same syntax as `maki search`)
        #[arg(display_order = 1)]
        query: Option<String>,

        /// Limit to a specific volume
        #[arg(long, display_order = 10)]
        volume: Option<String>,

        /// Create sidecars only for a specific asset
        #[arg(long, display_order = 11)]
        asset: Option<String>,

        /// Apply changes (default: report-only dry run)
        #[arg(long, display_order = 20)]
        apply: bool,

        /// Asset IDs (for shell variable expansion)
        #[arg(hide = true, trailing_var_arg = true)]
        asset_ids: Vec<String>,
    },

    /// Rebuild SQLite catalog from sidecar files
    #[command(display_order = 51)]
    RebuildCatalog {
        /// Rebuild only a specific asset (by ID or prefix)
        #[arg(long)]
        asset: Option<String>,
    },

    /// Run database schema migrations
    #[command(display_order = 52)]
    Migrate,

    /// Open documentation in the browser
    #[command(display_order = 55)]
    Doc {
        /// Which document: manual, cheatsheet, filters (default: manual)
        #[arg(default_value = "manual")]
        document: String,
    },

    /// Show MAKI license and third-party crate licenses
    #[command(display_order = 56)]
    Licenses {
        /// Show summary only (counts and where to find full text)
        #[arg(long)]
        summary: bool,
    },

    /// Start an interactive asset management shell
    #[command(display_order = 56)]
    Shell {
        /// Script file to execute (instead of interactive mode)
        script: Option<String>,

        /// Run a single command and exit
        #[arg(short = 'c', long = "command")]
        command_str: Option<String>,

        /// Exit on first error (scripts only)
        #[arg(long)]
        strict: bool,
    },
}

#[derive(Subcommand)]
enum TagCommands {
    /// Rename a tag across all assets
    Rename {
        /// Current tag name. Optional prefix markers (in any order) match the
        /// `tag:` search filter syntax: `=Foo` matches the exact level only
        /// (no descendants), `^Foo` is case-sensitive, `=^Foo` is both.
        old_tag: String,

        /// New tag name (always taken literally; no prefix parsing)
        new_tag: String,

        /// Apply changes (default: report-only)
        #[arg(long)]
        apply: bool,
    },

    /// Remove all tags from an asset
    Clear {
        /// Asset ID (or unique prefix)
        asset_id: String,
    },

    /// Expand all hierarchical tags to include ancestor paths
    #[command(name = "expand-ancestors")]
    ExpandAncestors {
        /// Search query to select assets (same syntax as `maki search`)
        #[arg(display_order = 1)]
        query: Option<String>,

        /// Limit to a specific asset
        #[arg(long, display_order = 10)]
        asset: Option<String>,

        /// Apply changes (default: report-only)
        #[arg(long, display_order = 20)]
        apply: bool,

        /// Asset IDs (for shell variable expansion)
        #[arg(hide = true, trailing_var_arg = true)]
        asset_ids: Vec<String>,
    },

    /// Export current tag tree as a vocabulary.yaml file
    #[command(name = "export-vocabulary")]
    ExportVocabulary {
        /// Output file (default: vocabulary.yaml in catalog root)
        #[arg(long)]
        output: Option<String>,

        /// Remove vocabulary entries that have no assets (only keep used tags)
        #[arg(long)]
        prune: bool,

        /// Export only the built-in default vocabulary (ignore catalog tags and existing vocabulary.yaml)
        #[arg(long)]
        default: bool,
    },
}

#[derive(Subcommand)]
enum VolumeCommands {
    /// Register a new volume (LABEL PATH or just PATH to auto-derive label)
    Add {
        /// Label and path: "LABEL PATH" or just "PATH" (label derived from path)
        #[arg(required = true, num_args = 1..=2)]
        args: Vec<String>,

        /// Volume purpose (media, working, archive, backup, cloud)
        #[arg(long)]
        purpose: Option<String>,
    },

    /// List all volumes and their status
    List {
        /// Filter by volume purpose (media, working, archive, backup, cloud)
        #[arg(long)]
        purpose: Option<String>,

        /// Show only offline volumes
        #[arg(long)]
        offline: bool,

        /// Show only online volumes
        #[arg(long)]
        online: bool,
    },

    /// Set or clear the purpose of a volume
    SetPurpose {
        /// Volume label or UUID
        volume: String,

        /// Purpose (media, working, archive, backup, cloud) or "none" to clear
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

    /// Split a subdirectory from a volume into a new volume
    Split {
        /// Source volume label or UUID
        source: String,

        /// Label for the new volume
        new_label: String,

        /// Subdirectory path to split off (relative to volume mount point)
        #[arg(long)]
        path: String,

        /// Volume purpose for the new volume (media, working, archive, backup, cloud)
        #[arg(long)]
        purpose: Option<String>,

        /// Actually split (default is report-only)
        #[arg(long)]
        apply: bool,
    },

    /// Rename a volume
    Rename {
        /// Current volume label or UUID
        volume: String,

        /// New label
        new_label: String,
    },
}

#[derive(Subcommand)]
enum SavedSearchCommands {
    /// Save a search with a name
    Save {
        /// Name for this saved search
        name: String,

        /// Search query (same format as `maki search`)
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

#[cfg(feature = "ai")]
#[derive(Subcommand)]
enum FacesCommands {
    /// Detect faces in assets
    Detect {
        /// Search query to scope assets
        #[arg(long)]
        query: Option<String>,

        /// Process a specific asset (ID or prefix)
        #[arg(long)]
        asset: Option<String>,

        /// Limit to assets on a specific volume
        #[arg(long)]
        volume: Option<String>,

        /// Minimum detection confidence (0.0–1.0, default 0.5)
        #[arg(long, default_value = "0.5")]
        min_confidence: f32,

        /// Apply detections to catalog (default: report-only)
        #[arg(long)]
        apply: bool,

        /// Force re-detection even if faces already exist for an asset
        #[arg(long)]
        force: bool,
    },

    /// Download face detection and recognition models
    Download,

    /// Show face detection status
    Status,

    /// Auto-cluster unassigned faces into unnamed people
    Cluster {
        /// Search query to scope which assets' faces to cluster
        #[arg(long)]
        query: Option<String>,

        /// Process faces from a specific asset (ID or prefix)
        #[arg(long)]
        asset: Option<String>,

        /// Limit to faces on assets from a specific volume
        #[arg(long)]
        volume: Option<String>,

        /// Similarity threshold for clustering (0.0–1.0)
        #[arg(long)]
        threshold: Option<f32>,

        /// Apply clustering (default: dry-run showing cluster sizes)
        #[arg(long)]
        apply: bool,
    },

    /// List all people
    People,

    /// Name (or rename) a person
    Name {
        /// Person ID (or prefix)
        person_id: String,

        /// Name to assign
        name: String,
    },

    /// Merge two people (move all faces from source to target)
    Merge {
        /// Target person ID (or prefix) — faces are moved here
        target_id: String,

        /// Source person ID (or prefix) — will be deleted after merge
        source_id: String,
    },

    /// Delete a person (unassigns all their faces)
    DeletePerson {
        /// Person ID (or prefix) to delete
        person_id: String,
    },

    /// Unassign a face from its person
    Unassign {
        /// Face ID (or prefix) to unassign
        face_id: String,
    },

    /// Export faces, people, and embeddings from SQLite to YAML + binary files
    Export,
}

/// Print custom grouped help text through a pager.
fn print_custom_help() {
    let version = env!("CARGO_PKG_VERSION");
    let edition = if cfg!(feature = "pro") { " Pro" } else { "" };
    let ai_note = if cfg!(feature = "pro") { "" } else { "  (download MAKI Pro for: describe, auto-tag, embed, faces, stroll, writeback, sync-metadata)" };

    let help = format!("\
maki{edition} {version} — Media Asset Keeper & Indexer{ai_note}

Usage: maki [OPTIONS] <COMMAND>

Setup:
  init               Initialize a new catalog in the current directory
  volume             Manage storage volumes

Ingest & Edit:
  import             Import files into the catalog
  delete             Delete assets from the catalog
  tag                Add, remove, rename, clear, or expand tags (supports subcommands)
  edit               Edit asset metadata (name, description, rating, label, date)
  group              Group variants into one asset
  split              Split variants out of an asset into new standalone assets
  auto-group         Auto-group assets by filename stem
{describe}{auto_tag}{embed}{faces}

Organize:
  collection         Manage collections (static albums)  [alias: col]
  saved-search       Manage saved searches (smart albums)  [alias: ss]
  stack              Manage stacks (scene grouping)  [alias: st]

Retrieve:
  search             Search assets
  show               Show asset details
  preview            Open the asset's preview in the OS default image viewer
  export             Export files matching a search query to a directory
  contact-sheet      Generate a PDF contact sheet from search results
  duplicates         Find duplicate files
  stats              Show catalog statistics
  backup-status      Check backup coverage and find under-backed-up assets
  doc                Open documentation in the browser (manual, cheatsheet, filters)
  licenses           Show MAKI license and third-party crate licenses

Maintain:
  verify             Check file integrity
  sync               Sync catalog with disk changes (moved/modified/missing files)
  refresh            Re-read metadata from changed sidecar/recipe files{sync_metadata}{writeback}
  cleanup            Remove stale file location records (files no longer on disk)
  dedup              Remove same-volume duplicate file locations
  relocate           Copy or move asset files to another volume
  update-location    Update a file's catalog path after it was moved on disk
  generate-previews  Generate or regenerate preview thumbnails
  fix-roles          Fix variant roles in RAW+non-RAW groups
  fix-dates          Fix asset dates from EXIF metadata and file timestamps
  fix-recipes        Re-attach recipe files that were imported as standalone assets
  rebuild-catalog    Rebuild SQLite catalog from sidecar files
  migrate            Run database schema migrations

Interactive:
  serve              Start the web UI server
  shell              Interactive asset management shell (variables, tab completion, scripts)

Options:
      --json         Output machine-readable JSON
  -v, --verbose      Show operational details (file counts, volume detection, etc.)
  -l, --log          Log individual file progress
  -d, --debug        Show debug output from external tools (implies --verbose)
  -t, --time         Show elapsed time after command execution
  -h, --help         Print help (use <command> --help for details)
  -V, --version      Print version

  https://maki-dam.com — docs, downloads, and support
",
        describe = if cfg!(feature = "pro") { "\n  describe           Generate image descriptions using a VLM" } else { "" },
        auto_tag = if cfg!(feature = "ai") { "\n  auto-tag           Auto-tag assets using AI vision model" } else { "" },
        embed = if cfg!(feature = "ai") { "\n  embed              Generate embeddings for visual similarity search" } else { "" },
        faces = if cfg!(feature = "ai") { "\n  faces              Face detection and recognition" } else { "" },
        sync_metadata = if cfg!(feature = "pro") { "\n  sync-metadata      Bidirectional metadata sync: read XMP changes + write back pending edits" } else { "" },
        writeback = if cfg!(feature = "pro") { "\n  writeback          Write back pending metadata changes to XMP recipe files" } else { "" },
    );

    // Pipe through pager if stdout is a terminal
    if atty_stdout() {
        if let Ok(mut child) = std::process::Command::new("less")
            .args(["-FRSX"])
            .stdin(std::process::Stdio::piped())
            .spawn()
        {
            if let Some(ref mut stdin) = child.stdin {
                use std::io::Write;
                let _ = stdin.write_all(help.as_bytes());
            }
            let _ = child.wait();
            return;
        }
    }
    print!("{help}");
}

/// Check if stdout is a terminal.
fn atty_stdout() -> bool {
    use std::io::IsTerminal;
    std::io::stdout().is_terminal()
}

fn check_schema() {
    if let Ok(root) = maki::config::find_catalog_root() {
        if let Ok(catalog) = Catalog::open(&root) {
            if !catalog.is_schema_current() {
                eprintln!(
                    "Error: catalog schema is outdated (v{}, expected v{}). Run `maki migrate` to update.",
                    catalog.schema_version(),
                    maki::catalog::SCHEMA_VERSION,
                );
                std::process::exit(1);
            }
        }
    }
}

fn main() {
    // Intercept top-level --help / -h before clap parses
    let args: Vec<String> = std::env::args().collect();
    if args.len() == 2 && (args[1] == "--help" || args[1] == "-h") {
        print_custom_help();
        std::process::exit(0);
    }

    let mut cli = Cli::parse();
    let start = std::time::Instant::now();

    // Merge [cli] defaults from maki.toml (if inside a catalog)
    if let Ok(catalog_root) = maki::config::find_catalog_root() {
        if let Ok(config) = CatalogConfig::load(&catalog_root) {
            cli.log = cli.log || config.cli.log;
            cli.timing = cli.timing || config.cli.time;
            cli.verbose = cli.verbose || config.cli.verbose;
        }
    }

    // Handle shell command specially — it has its own loop
    if let Commands::Shell { script, command_str, strict } = &cli.command {
        check_schema();
        let catalog_root = match maki::config::find_catalog_root() {
            Ok(r) => r,
            Err(e) => {
                eprintln!("Error: {e:#}");
                std::process::exit(1);
            }
        };
        let opts = maki::shell::RunOptions {
            script: script.as_ref().map(PathBuf::from),
            command: command_str.clone(),
            strict: *strict,
        };
        maki::shell::run(&catalog_root, opts, |args| {
            let shell_cli = Cli::try_parse_from(&args).map_err(|e| anyhow::anyhow!("{e}"))?;
            run_command(shell_cli)
        });
        return;
    }

    // Check schema version at startup (if inside a catalog).
    // Only `maki init` and `maki migrate` skip this check.
    if !matches!(cli.command, Commands::Init | Commands::Migrate) {
        check_schema();
    }

    let timing = cli.timing;
    let result = run_command(cli);

    if timing {
        eprintln!("Elapsed: {}", format_duration(start.elapsed()));
    }

    if let Err(e) = result {
        eprintln!("Error: {e:#}");
        std::process::exit(1);
    }
}

/// Execute a parsed CLI command. Returns asset IDs produced by the command (for shell _ variable).
fn run_command(cli: Cli) -> anyhow::Result<Vec<String>> {
    let verbosity = maki::Verbosity::new(cli.verbose, cli.debug);
    let mut _asset_ids: Vec<String> = Vec::new();

    let result: anyhow::Result<()> = (|| match cli.command {
        Commands::Init => {
            let catalog_root = std::env::current_dir()?;
            let config_path = catalog_root.join("maki.toml");
            if config_path.exists() {
                anyhow::bail!("a maki catalog already exists in this directory.");
            }

            // Create directories
            std::fs::create_dir_all(catalog_root.join("metadata"))?;
            std::fs::create_dir_all(catalog_root.join("previews"))?;
            std::fs::create_dir_all(catalog_root.join("smart-previews"))?;

            // Write config
            CatalogConfig::default().save(&catalog_root)?;

            // Initialize SQLite schema
            let catalog = Catalog::open(&catalog_root)?;
            catalog.initialize()?;

            // Write empty volumes registry
            DeviceRegistry::init(&catalog_root)?;

            // Write .gitignore for optional git-based backup
            let gitignore_path = catalog_root.join(".gitignore");
            if !gitignore_path.exists() {
                std::fs::write(&gitignore_path, "\
# Derived cache — rebuilt from YAML sidecars via 'maki rebuild-catalog'\n\
catalog.db*\n\
\n\
# Generated thumbnails — regenerated via 'maki generate-previews'\n\
previews/\n\
smart-previews/\n\
\n\
# AI artifacts — regenerated via 'maki embed' / 'maki faces detect'\n\
embeddings/\n\
faces/\n\
")?;
            }

            // Write default tag vocabulary
            let vocab_path = catalog_root.join("vocabulary.yaml");
            if !vocab_path.exists() {
                std::fs::write(&vocab_path, maki::vocabulary::default_vocabulary())?;
            }

            if cli.json {
                println!("{}", serde_json::json!({
                    "status": "initialized",
                    "path": catalog_root.display().to_string()
                }));
            } else {
                println!("Initialized new maki catalog in {}", catalog_root.display());
            }
            Ok(())
        }
        Commands::Volume(cmd) => match cmd {
            VolumeCommands::Add { args, purpose } => {
                // Two positional args: LABEL PATH. One arg: PATH (label derived).
                let (label, path) = if args.len() == 2 {
                    (args[0].clone(), args[1].clone())
                } else {
                    let path = &args[0];
                    let label = std::path::Path::new(path)
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("Volume")
                        .to_string();
                    (label, path.clone())
                };

                let catalog_root = maki::config::find_catalog_root()?;
                let registry = DeviceRegistry::new(&catalog_root);
                let parsed_purpose = if let Some(ref p) = purpose {
                    Some(maki::models::VolumePurpose::parse(p).ok_or_else(|| {
                        anyhow::anyhow!("invalid purpose '{}'. Valid values: media, working, archive, backup, cloud", p)
                    })?)
                } else {
                    None
                };
                let volume = registry.register(
                    &label,
                    std::path::Path::new(&path),
                    maki::models::VolumeType::Local,
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
                        eprintln!("  Hint: use --purpose <media|working|archive|backup|cloud> to set the volume's role");
                    }
                }
                Ok(())
            }
            VolumeCommands::List { purpose, offline, online } => {
                if offline && online {
                    anyhow::bail!("--offline and --online are mutually exclusive");
                }
                let purpose_filter = if let Some(ref p) = purpose {
                    Some(maki::models::VolumePurpose::parse(p).ok_or_else(|| {
                        anyhow::anyhow!("invalid purpose '{}'. Valid values: media, working, archive, backup, cloud", p)
                    })?)
                } else {
                    None
                };

                let catalog_root = maki::config::find_catalog_root()?;
                let registry = DeviceRegistry::new(&catalog_root);
                let volumes: Vec<_> = registry.list()?.into_iter().filter(|v| {
                    if let Some(ref pf) = purpose_filter {
                        if v.purpose.as_ref() != Some(pf) {
                            return false;
                        }
                    }
                    if offline && v.is_online { return false; }
                    if online && !v.is_online { return false; }
                    true
                }).collect();

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
                    if purpose.is_some() || offline || online {
                        println!("No matching volumes.");
                    } else {
                        println!("No volumes registered.");
                    }
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
                let catalog_root = maki::config::find_catalog_root()?;
                let registry = DeviceRegistry::new(&catalog_root);
                let parsed_purpose = if purpose == "none" || purpose == "clear" {
                    None
                } else {
                    Some(maki::models::VolumePurpose::parse(&purpose).ok_or_else(|| {
                        anyhow::anyhow!("invalid purpose '{}'. Valid values: media, working, archive, backup, cloud, none", purpose)
                    })?)
                };
                let vol = registry.set_purpose(&volume, parsed_purpose)?;
                // Update the SQLite cache too
                let catalog = maki::catalog::Catalog::open(&catalog_root)?;
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
                let catalog_root = maki::config::find_catalog_root()?;
                let config = CatalogConfig::load(&catalog_root)?;
                let service = AssetService::new(&catalog_root, verbosity, &config.preview);

                let show_log = cli.log;
                let result = if show_log {
                    use maki::asset_service::CleanupStatus;
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
                                CleanupStatus::LocationlessVariant => {
                                    let name = path.to_str().unwrap_or("?");
                                    eprintln!("  {} — locationless variant removed ({})", name, format_duration(elapsed));
                                }
                                CleanupStatus::OrphanedAsset => {
                                    let name = path.to_str().unwrap_or("?");
                                    eprintln!("  {} — orphaned asset removed ({})", name, format_duration(elapsed));
                                }
                                CleanupStatus::OrphanedFile => {
                                    let name = path.file_name()
                                        .and_then(|n| n.to_str())
                                        .unwrap_or_else(|| path.to_str().unwrap_or("?"));
                                    eprintln!("  {} — orphaned file removed ({})", name, format_duration(elapsed));
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
                let catalog_root = maki::config::find_catalog_root()?;
                let config = CatalogConfig::load(&catalog_root)?;
                let service = AssetService::new(&catalog_root, verbosity, &config.preview);

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
            VolumeCommands::Split { source, new_label, path, purpose, apply } => {
                let catalog_root = maki::config::find_catalog_root()?;
                let config = CatalogConfig::load(&catalog_root)?;
                let service = AssetService::new(&catalog_root, verbosity, &config.preview);

                let show_log = cli.log;
                let result = service.split_volume(
                    &source,
                    &new_label,
                    &path,
                    purpose.as_deref(),
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
                            "Volume '{}' split: new volume '{}' created with {} locations, {} recipes ({} assets, prefix '{}')",
                            result.source_label,
                            result.new_label,
                            result.locations_moved,
                            result.recipes_moved,
                            result.assets_affected,
                            result.path_prefix,
                        );
                    } else {
                        println!(
                            "Would split '{}': new volume '{}' with {} locations, {} recipes ({} assets, prefix '{}')",
                            result.source_label,
                            result.new_label,
                            result.locations,
                            result.recipes,
                            result.assets_affected,
                            result.path_prefix,
                        );
                        if result.locations > 0 || result.recipes > 0 {
                            println!("  Run with --apply to split.");
                        }
                    }
                }
                Ok(())
            }
            VolumeCommands::Rename { volume, new_label } => {
                let catalog_root = maki::config::find_catalog_root()?;
                let registry = DeviceRegistry::new(&catalog_root);
                let vol = registry.resolve_volume(&volume)?;
                let old_label = vol.label.clone();

                registry.rename(&volume, &new_label)?;

                let catalog = maki::catalog::Catalog::open(&catalog_root)?;
                catalog.rename_volume(&vol.id.to_string(), &new_label)?;

                if cli.json {
                    println!("{}", serde_json::json!({
                        "old_label": old_label,
                        "new_label": new_label,
                        "volume_id": vol.id.to_string(),
                    }));
                } else {
                    println!("Volume '{}' renamed to '{}'", old_label, new_label);
                }
                Ok(())
            }
        },
        Commands::Import {
            paths,
            volume,
            profile,
            include,
            skip,
            add_tags,
            dry_run,
            auto_group,
            smart,
            #[cfg(feature = "ai")]
            embed,
            #[cfg(feature = "pro")]
            describe,
        } => {
            use maki::asset_service::FileTypeFilter;

            let catalog_root = maki::config::find_catalog_root()?;
            let config = CatalogConfig::load(&catalog_root)?;

            // Resolve import profile: profile overrides base [import], CLI flags override both
            let import_config = if let Some(ref profile_name) = profile {
                config.import.resolve_profile(profile_name)
                    .ok_or_else(|| anyhow::anyhow!(
                        "Unknown import profile '{}'. Available profiles: {}",
                        profile_name,
                        if config.import.profiles.is_empty() {
                            "(none configured)".to_string()
                        } else {
                            config.import.profiles.keys().cloned().collect::<Vec<_>>().join(", ")
                        }
                    ))?
            } else {
                config.import.clone()
            };

            let smart = smart || import_config.smart_previews;
            let registry = DeviceRegistry::new(&catalog_root);

            // Build file type filter: merge profile include/skip with CLI flags
            let mut filter = FileTypeFilter::default();

            // Profile include/skip first (if a profile was selected)
            let profile_ref = profile.as_deref().and_then(|name| config.import.profiles.get(name));
            if let Some(p) = profile_ref {
                for group in &p.include {
                    filter.include(group)?;
                }
                for group in &p.skip {
                    filter.skip(group)?;
                }
            }

            // CLI flags override (check for conflicts)
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
                anyhow::bail!("no paths specified for import.");
            }

            // Resolve volume: explicit --volume flag, or auto-detect from path
            let volume = if let Some(label) = &volume {
                registry.resolve_volume(label)?
            } else {
                registry.find_volume_for_path(&canonical_paths[0])?
            };

            // Merge config auto_tags with CLI --add-tag values
            let mut all_tags = import_config.auto_tags.clone();
            for tag in &add_tags {
                if !all_tags.contains(tag) {
                    all_tags.push(tag.clone());
                }
            }

            if verbosity.verbose {
                eprintln!("  Import: {} file(s) on volume \"{}\"", canonical_paths.len(), volume.label);
                if let Some(ref p) = profile {
                    eprintln!("  Import: using profile \"{}\"", p);
                }
                if !import_config.exclude.is_empty() {
                    eprintln!("  Import: exclude patterns: {:?}", import_config.exclude);
                }
                if !all_tags.is_empty() {
                    eprintln!("  Import: auto-tags: {}", all_tags.join(", "));
                }
                if smart {
                    eprintln!("  Import: smart previews enabled");
                }
            }

            let service = AssetService::new(&catalog_root, verbosity, &config.preview);
            let result = if cli.log {
                use maki::asset_service::FileStatus;
                service.import_with_callback(&canonical_paths, &volume, &filter, &import_config.exclude, &all_tags, dry_run, smart, |path, status, elapsed| {
                    let label = match status {
                        FileStatus::Imported => "imported",
                        FileStatus::LocationAdded => "location added",
                        FileStatus::Skipped => "skipped",
                        FileStatus::RecipeAttached => "recipe",
                        FileStatus::RecipeLocationAdded => "recipe location added",
                        FileStatus::RecipeUpdated => "recipe updated",
                    };
                    let name = path.file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or_else(|| path.to_str().unwrap_or("?"));
                    eprintln!("  {} — {} ({})", name, label, format_duration(elapsed));
                })?
            } else {
                service.import_with_callback(&canonical_paths, &volume, &filter, &import_config.exclude, &all_tags, dry_run, smart, |_, _, _| {})?
            };

            // Post-import auto-group phase
            let auto_group_result = if auto_group
                && (result.imported > 0 || result.locations_added > 0)
            {
                use maki::catalog::Catalog;
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
                        // Upgrade previews for grouped assets (use Export/Processed variant)
                        if !dry_run {
                            let grouped_ids: Vec<String> = ag_result.groups.iter()
                                .map(|g| g.target_id.clone())
                                .collect();
                            let metadata_store = MetadataStore::new(&catalog_root);
                            let preview_gen = maki::preview::PreviewGenerator::new(&catalog_root, verbosity, &config.preview);
                            let volumes = registry.list()?;
                            let mut upgraded = 0usize;
                            for gid in &grouped_ids {
                                if let Ok(uuid) = gid.parse::<uuid::Uuid>() {
                                    if let Ok(asset) = metadata_store.load(uuid) {
                                        if let Some(idx) = maki::models::variant::best_preview_index(&asset.variants) {
                                            if idx > 0 {
                                                // A better variant exists — upgrade the preview
                                                let v = &asset.variants[idx];
                                                if let Some(loc) = v.locations.first() {
                                                    if let Some(vol) = volumes.iter().find(|vl| vl.id == loc.volume_id && vl.is_online) {
                                                        let file_path = vol.mount_point.join(&loc.relative_path);
                                                        if file_path.exists() {
                                                            let _ = preview_gen.generate(&v.content_hash, &file_path, &v.format);
                                                            if smart {
                                                                let _ = preview_gen.generate_smart(&v.content_hash, &file_path, &v.format);
                                                            }
                                                            upgraded += 1;
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                        // Update denormalized best_variant_hash
                                        if let Ok(cat) = Catalog::open(&catalog_root) {
                                            let _ = cat.update_denormalized_variant_columns(&asset);
                                        }
                                    }
                                }
                            }
                            if upgraded > 0 && verbosity.verbose {
                                eprintln!("  Auto-group: upgraded {} preview(s) from export variants", upgraded);
                            }
                        }
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

            // Post-import embedding phase (AI feature)
            #[cfg(feature = "ai")]
            let embed_result = if !dry_run
                && (embed || import_config.embeddings)
                && !result.new_asset_ids.is_empty()
            {
                use maki::model_manager::ModelManager;

                let model_id = &config.ai.model;
                let model_dir_str = &config.ai.model_dir;
                let model_base = if model_dir_str.starts_with("~/") {
                    let home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE"))?;
                    PathBuf::from(home).join(&model_dir_str[2..])
                } else {
                    PathBuf::from(model_dir_str)
                };
                let model_dir = model_base.join(model_id);
                let mgr = ModelManager::new(&model_dir, model_id)?;

                if mgr.model_exists() {
                    let catalog = maki::catalog::Catalog::open(&catalog_root)?;
                    let _ = maki::embedding_store::EmbeddingStore::initialize(catalog.conn());
                    let emb_store = maki::embedding_store::EmbeddingStore::new(catalog.conn());

                    let service = AssetService::new(&catalog_root, verbosity, &config.preview);
                    let preview_gen = maki::preview::PreviewGenerator::new(
                        &catalog_root,
                        verbosity,
                        &config.preview,
                    );

                    let registry = DeviceRegistry::new(&catalog_root);
                    let volumes_list = registry.list()?;
                    let online_volumes: std::collections::HashMap<String, &maki::models::Volume> =
                        volumes_list
                            .iter()
                            .filter(|v| v.is_online)
                            .map(|v| (v.id.to_string(), v))
                            .collect();

                    let mut ai_model = maki::ai::SigLipModel::load_with_provider(&model_dir, model_id, verbosity, &config.ai.execution_provider)?;

                    let mut embedded = 0u32;
                    let mut embed_skipped = 0u32;
                    for aid in &result.new_asset_ids {
                        let short_id = &aid[..8_usize.min(aid.len())];

                        if emb_store.has_embedding(aid, model_id) {
                            embed_skipped += 1;
                            continue;
                        }

                        let details = match catalog.load_asset_details(aid)? {
                            Some(d) => d,
                            None => continue,
                        };

                        let image_path = match service.find_image_for_ai(&details, &preview_gen, &online_volumes) {
                            Some(p) => p,
                            None => { embed_skipped += 1; continue; }
                        };

                        let ext = image_path.extension().and_then(|e| e.to_str()).unwrap_or("");
                        if !maki::ai::is_supported_image(ext) {
                            embed_skipped += 1;
                            continue;
                        }

                        match ai_model.encode_image(&image_path) {
                            Ok(emb) => {
                                if let Err(e) = emb_store.store(aid, &emb, model_id) {
                                    if cli.log {
                                        eprintln!("  {short_id} — embed error: {e}");
                                    }
                                    continue;
                                }
                                let _ = maki::embedding_store::write_embedding_binary(&catalog_root, model_id, aid, &emb);
                                embedded += 1;
                                if cli.log {
                                    eprintln!("  {short_id} — embedded");
                                }
                            }
                            Err(e) => {
                                if cli.log {
                                    eprintln!("  {short_id} — embed error: {e:#}");
                                }
                            }
                        }
                    }
                    Some((embedded, embed_skipped))
                } else {
                    if cli.log {
                        eprintln!("  Skipping embeddings: model not downloaded. Run 'maki auto-tag --download' first.");
                    }
                    None
                }
            } else {
                None
            };

            // Post-import VLM describe phase
            #[cfg(feature = "pro")]
            let describe_result = if !dry_run
                && (describe || import_config.descriptions)
                && !result.new_asset_ids.is_empty()
            {
                // Check VLM endpoint availability first
                let endpoint = &config.vlm.endpoint;
                let vlm_model = &config.vlm.model;
                let vlm_available = maki::vlm::check_endpoint(endpoint, 5, verbosity).is_ok();

                if vlm_available {
                    let mode = maki::vlm::DescribeMode::from_str(&config.vlm.mode)
                        .unwrap_or(maki::vlm::DescribeMode::Describe);
                    let import_params = config.vlm.params_for_model(vlm_model);
                    let service = AssetService::new(&catalog_root, verbosity, &config.preview);
                    let log = cli.log;
                    match service.describe_assets(
                        &result.new_asset_ids,
                        endpoint,
                        vlm_model,
                        &import_params,
                        mode,
                        false, // force: don't overwrite existing descriptions
                        false, // dry_run
                        config.vlm.concurrency,
                        |aid, status, elapsed| {
                            if log {
                                let short = &aid[..8.min(aid.len())];
                                match status {
                                    maki::vlm::DescribeStatus::Described => {
                                        eprintln!("  {short} — described ({})", format_duration(elapsed));
                                    }
                                    maki::vlm::DescribeStatus::Skipped(msg) => {
                                        eprintln!("  {short} — skipped: {msg}");
                                    }
                                    maki::vlm::DescribeStatus::Error(msg) => {
                                        eprintln!("  {short} — error: {msg}");
                                    }
                                }
                            }
                        },
                    ) {
                        Ok(dr) => Some(dr),
                        Err(e) => {
                            if cli.log {
                                eprintln!("  Describe phase failed: {e:#}");
                            }
                            None
                        }
                    }
                } else {
                    if cli.log {
                        eprintln!("  Skipping descriptions: VLM endpoint not available at {endpoint}");
                    }
                    None
                }
            } else {
                None
            };

            if cli.json {
                #[allow(unused_mut)]
                let mut json_val = serde_json::to_value(&result)?;
                if let Some(ref ag) = auto_group_result {
                    json_val["auto_group"] = serde_json::to_value(ag)?;
                }
                #[cfg(feature = "ai")]
                if let Some((embedded, skipped_embed)) = embed_result {
                    json_val["embeddings_generated"] = serde_json::json!(embedded);
                    json_val["embeddings_skipped"] = serde_json::json!(skipped_embed);
                }
                #[cfg(feature = "pro")]
                if let Some(ref dr) = describe_result {
                    json_val["descriptions_generated"] = serde_json::json!(dr.described);
                    json_val["descriptions_skipped"] = serde_json::json!(dr.skipped);
                    if dr.tags_applied > 0 {
                        json_val["describe_tags_applied"] = serde_json::json!(dr.tags_applied);
                    }
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
                if result.recipes_location_added > 0 {
                    parts.push(format!("{} recipe location(s) added", result.recipes_location_added));
                }
                if result.recipes_updated > 0 {
                    parts.push(format!("{} recipe(s) updated", result.recipes_updated));
                }
                if result.previews_generated > 0 {
                    parts.push(format!("{} preview(s) generated", result.previews_generated));
                }
                if result.smart_previews_generated > 0 {
                    parts.push(format!("{} smart preview(s) generated", result.smart_previews_generated));
                }
                #[cfg(feature = "ai")]
                if let Some((embedded, _)) = embed_result {
                    if embedded > 0 {
                        parts.push(format!("{} embedding(s) generated", embedded));
                    }
                }
                #[cfg(feature = "pro")]
                if let Some(ref dr) = describe_result {
                    if dr.described > 0 {
                        parts.push(format!("{} described", dr.described));
                    }
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
            use maki::format::{self, OutputFormat};

            let catalog_root = maki::config::find_catalog_root()?;
            let config = CatalogConfig::load(&catalog_root)?;
            let engine = QueryEngine::with_default_filter(&catalog_root, config.browse.default_filter);
            let results = engine.search(&query)?;

            if verbosity.verbose {
                eprintln!("  Search: query=\"{query}\", {} result(s)", results.len());
            }

            // Capture asset IDs for shell _ variable
            _asset_ids = results.iter().map(|r| r.asset_id.clone()).collect();

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
        Commands::Show { asset_id, locations } => {
            let catalog_root = maki::config::find_catalog_root()?;
            let config = CatalogConfig::load(&catalog_root)?;
            let engine = QueryEngine::new(&catalog_root);
            let details = engine.show(&asset_id)?;

            if locations {
                if cli.json {
                    let locs: Vec<serde_json::Value> = details.variants.iter()
                        .flat_map(|v| v.locations.iter().map(move |loc| {
                            serde_json::json!({
                                "volume": loc.volume_label,
                                "path": loc.relative_path,
                                "variant": v.original_filename,
                                "format": v.format,
                                "role": v.role,
                            })
                        }))
                        .collect();
                    let recipe_locs: Vec<serde_json::Value> = details.recipes.iter()
                        .filter_map(|r| {
                            let label = r.volume_label.as_deref()?;
                            let path = r.relative_path.as_deref()?;
                            Some(serde_json::json!({
                                "volume": label,
                                "path": path,
                                "variant": r.software,
                                "format": r.recipe_type,
                                "role": "recipe",
                            }))
                        })
                        .collect();
                    let all: Vec<serde_json::Value> = locs.into_iter().chain(recipe_locs).collect();
                    println!("{}", serde_json::to_string_pretty(&all)?);
                } else {
                    for v in &details.variants {
                        for loc in &v.locations {
                            println!("{}:{}", loc.volume_label, loc.relative_path);
                        }
                    }
                    for r in &details.recipes {
                        if let (Some(label), Some(path)) = (&r.volume_label, &r.relative_path) {
                            println!("{label}:{path}");
                        }
                    }
                }
                return Ok(());
            }

            if cli.json {
                println!("{}", serde_json::to_string_pretty(&details)?);
            } else {
                let preview_gen = maki::preview::PreviewGenerator::new(&catalog_root, verbosity, &config.preview);

                println!("Asset: {}", details.id);
                if let Some(name) = &details.name {
                    println!("Name:  {name}");
                }
                println!("Type:  {}", details.asset_type);
                println!("Date:  {}", details.created_at);
                if !details.tags.is_empty() {
                    let display_tags: Vec<String> = details.tags.iter()
                        .map(|t| maki::tag_util::tag_storage_to_display(t))
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
                if let Some(idx) = maki::models::variant::best_preview_index_details(&details.variants) {
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
                        let short_variant = &r.variant_hash[r.variant_hash.len().saturating_sub(8)..];
                        println!("  [{}] {} → …{} ({})", r.recipe_type, r.software, short_variant, r.content_hash);
                        if let Some(path) = &r.relative_path {
                            let label = r.volume_label.as_deref().unwrap_or("?");
                            println!("    Location: {label}:{path}");
                        }
                    }
                }
            }

            Ok(())
        }
        Commands::Preview { asset_id } => {
            let catalog_root = maki::config::find_catalog_root()?;
            let config = CatalogConfig::load(&catalog_root)?;
            let catalog = maki::catalog::Catalog::open(&catalog_root)?;
            let engine = QueryEngine::new(&catalog_root);
            let details = engine.show(&asset_id)?;
            let full_id = &details.id;
            let preview_gen = maki::preview::PreviewGenerator::new(&catalog_root, verbosity, &config.preview);

            // Find best preview file (smart preview > regular preview)
            let best_hash = catalog.get_asset_best_variant_hash(full_id)
                .unwrap_or(None)
                .or_else(|| {
                    maki::models::variant::best_preview_index_details(&details.variants)
                        .map(|i| details.variants[i].content_hash.clone())
                });

            let preview_path = best_hash.as_ref().and_then(|h| {
                let smart = preview_gen.smart_preview_path(h);
                if smart.exists() { return Some(smart); }
                let regular = preview_gen.preview_path(h);
                if regular.exists() { return Some(regular); }
                None
            });

            match preview_path {
                Some(path) => {
                    maki::preview::open_in_viewer(&path)?;
                    if !cli.json {
                        let name = details.name.as_deref().unwrap_or(full_id);
                        eprintln!("Opened preview for {name}");
                    }
                    if cli.json {
                        println!("{}", serde_json::json!({
                            "id": full_id,
                            "preview": path.display().to_string(),
                        }));
                    }
                }
                None => {
                    let name = details.name.as_deref().unwrap_or(full_id);
                    if cli.json {
                        println!("{}", serde_json::json!({
                            "id": full_id,
                            "preview": null,
                        }));
                    } else {
                        eprintln!("No preview available for {name}");
                    }
                }
            }

            Ok(())
        }
        Commands::Tag { asset_id, remove, tags, subcmd } => {
            match subcmd {
                Some(TagCommands::Rename { old_tag, new_tag, apply }) => {
                    let catalog_root = maki::config::find_catalog_root()?;
                    let engine = QueryEngine::new(&catalog_root);
                    let old_storage = maki::tag_util::tag_input_to_storage(&old_tag);
                    let new_storage = maki::tag_util::tag_input_to_storage(&new_tag);
                    let show_log = cli.log;

                    use maki::query::TagRenameAction;
                    let result = engine.tag_rename(&old_storage, &new_storage, apply, |name, action| {
                        if show_log {
                            let verb = match (action, apply) {
                                (TagRenameAction::Renamed, true) => "renamed",
                                (TagRenameAction::Renamed, false) => "would rename",
                                (TagRenameAction::Removed, true) => "removed (already had target)",
                                (TagRenameAction::Removed, false) => "would remove (already has target)",
                                (TagRenameAction::Skipped, _) => "skipped (already correct)",
                            };
                            eprintln!("  {} — {}", name, verb);
                        }
                    })?;

                    if cli.json {
                        println!("{}", serde_json::to_string_pretty(&result)?);
                    } else {
                        let old_display = maki::tag_util::tag_storage_to_display(&old_storage);
                        let new_display = maki::tag_util::tag_storage_to_display(&new_storage);
                        if result.matched == 0 {
                            println!("No assets found with tag \"{}\".", old_display);
                        } else {
                            if !apply && (result.renamed > 0 || result.removed > 0) {
                                eprint!("Dry run — ");
                            }
                            let mut parts = Vec::new();
                            if result.renamed > 0 {
                                parts.push(format!("{} renamed", result.renamed));
                            }
                            if result.removed > 0 {
                                parts.push(format!("{} removed (merged)", result.removed));
                            }
                            if result.skipped > 0 {
                                parts.push(format!("{} skipped", result.skipped));
                            }
                            println!(
                                "Tag rename: \"{}\" → \"{}\": {}",
                                old_display, new_display, parts.join(", "),
                            );
                            if !apply && (result.renamed > 0 || result.removed > 0) {
                                println!("  Run with --apply to rename tags.");
                            }
                        }
                    }
                    Ok(())
                }
                Some(TagCommands::Clear { asset_id }) => {
                    let catalog_root = maki::config::find_catalog_root()?;
                    let engine = QueryEngine::new(&catalog_root);

                    // Load asset to get current tags, then remove all
                    let catalog = maki::catalog::Catalog::open(&catalog_root)?;
                    let full_id = catalog.resolve_asset_id(&asset_id)?
                        .ok_or_else(|| anyhow::anyhow!("no asset found matching '{asset_id}'"))?;
                    let uuid: uuid::Uuid = full_id.parse()?;
                    let metadata_store = maki::metadata_store::MetadataStore::new(&catalog_root);
                    let asset = metadata_store.load(uuid)?;

                    if asset.tags.is_empty() {
                        if cli.json {
                            println!("{}", serde_json::json!({ "changed": [], "tags": [] }));
                        } else {
                            println!("Tags: (none)");
                        }
                        return Ok(());
                    }

                    let tags_to_remove = asset.tags.clone();
                    let result = engine.tag(&asset_id, &tags_to_remove, true)?;

                    if cli.json {
                        println!("{}", serde_json::json!({
                            "changed": result.changed,
                            "tags": result.current_tags,
                        }));
                    } else {
                        let display_removed: Vec<String> = result.changed.iter()
                            .map(|t| maki::tag_util::tag_storage_to_display(t))
                            .collect();
                        println!("Cleared {} tag(s): {}", display_removed.len(), display_removed.join(", "));
                    }
                    Ok(())
                }
                Some(TagCommands::ExpandAncestors { query, asset, apply, asset_ids }) => {
                    let catalog_root = maki::config::find_catalog_root()?;
                    let catalog = maki::catalog::Catalog::open(&catalog_root)?;
                    let metadata_store = maki::metadata_store::MetadataStore::new(&catalog_root);
                    let engine = maki::query::QueryEngine::new(&catalog_root);

                    let scope = engine.resolve_scope(query.as_deref(), asset.as_deref(), &asset_ids)?;
                    let summaries = metadata_store.list()?;
                    let ids: Vec<uuid::Uuid> = match scope {
                        Some(set) => set.iter().filter_map(|id| id.parse().ok()).collect(),
                        None => summaries.iter().map(|s| s.id).collect(),
                    };

                    let mut checked = 0usize;
                    let mut expanded = 0usize;
                    let mut tags_added = 0usize;

                    for asset_id in &ids {
                        let mut asset_obj = match metadata_store.load(*asset_id) {
                            Ok(a) => a,
                            Err(_) => continue,
                        };
                        checked += 1;

                        let full = maki::tag_util::expand_all_ancestors(&asset_obj.tags);
                        let existing: std::collections::HashSet<&str> = asset_obj.tags.iter().map(|s| s.as_str()).collect();
                        let missing: Vec<String> = full.iter()
                            .filter(|t| !existing.contains(t.as_str()))
                            .cloned()
                            .collect();

                        if missing.is_empty() {
                            continue;
                        }

                        if cli.log {
                            let id_str = asset_id.to_string();
                            let name = asset_obj.name.as_deref().unwrap_or(&id_str[..8]);
                            if apply {
                                eprintln!("  {} — {} ancestor(s) added", name, missing.len());
                            } else {
                                eprintln!("  {} — {} ancestor(s) would add", name, missing.len());
                            }
                        }

                        if apply {
                            for tag in &missing {
                                asset_obj.tags.push(tag.clone());
                            }
                            metadata_store.save(&asset_obj)?;
                            catalog.insert_asset(&asset_obj)?;
                        }

                        expanded += 1;
                        tags_added += missing.len();
                    }

                    if cli.json {
                        println!("{}", serde_json::json!({
                            "dry_run": !apply,
                            "checked": checked,
                            "expanded": expanded,
                            "tags_added": tags_added,
                        }));
                    } else {
                        if !apply && expanded > 0 {
                            eprint!("Dry run — ");
                        }
                        println!("Expand ancestors: {} checked, {} assets with missing ancestors, {} tags {}",
                            checked, expanded, tags_added,
                            if apply { "added" } else { "would add" },
                        );
                        if !apply && expanded > 0 {
                            println!("  Run with --apply to expand ancestor tags.");
                        }
                    }
                    Ok(())
                }
                Some(TagCommands::ExportVocabulary { output, prune, default }) => {
                    let catalog_root = maki::config::find_catalog_root()?;

                    if default {
                        // Export only the built-in default vocabulary
                        let yaml = maki::vocabulary::default_vocabulary();
                        let out_path = output.map(std::path::PathBuf::from)
                            .unwrap_or_else(|| catalog_root.join("vocabulary.yaml"));
                        std::fs::write(&out_path, yaml)?;
                        println!("Exported default vocabulary to {}", out_path.display());
                        return Ok(());
                    }

                    let catalog = maki::catalog::Catalog::open(&catalog_root)?;
                    let catalog_tags = catalog.list_all_tags()?;

                    // Merge with existing vocabulary (preserve planned-but-unused entries)
                    let mut all_tags = catalog_tags;
                    if !prune {
                        let vocab = maki::vocabulary::load_vocabulary(&catalog_root);
                        let existing: std::collections::HashSet<String> = all_tags.iter().map(|(name, _)| name.clone()).collect();
                        for vt in vocab {
                            if !existing.contains(&vt) {
                                all_tags.push((vt, 0));
                            }
                        }
                    }
                    all_tags.sort_by(|a, b| a.0.cmp(&b.0));

                    let yaml = maki::vocabulary::tags_to_vocabulary_yaml(&all_tags);
                    let out_path = output.map(std::path::PathBuf::from)
                        .unwrap_or_else(|| catalog_root.join("vocabulary.yaml"));
                    std::fs::write(&out_path, &yaml)?;

                    let used = all_tags.iter().filter(|(_, c)| *c > 0).count();
                    let planned = all_tags.len() - used;
                    if prune {
                        println!("Exported {} tags to {} (pruned, unused entries removed)", used, out_path.display());
                    } else {
                        println!("Exported {} tags to {} ({} used, {} planned)", all_tags.len(), out_path.display(), used, planned);
                    }
                    Ok(())
                }
                None => {
                    // Direct tag add/remove: maki tag <asset> <tags> [--remove]
                    let asset_id = asset_id.ok_or_else(|| anyhow::anyhow!("asset ID is required for tag add/remove"))?;
                    if tags.is_empty() {
                        anyhow::bail!("no tags specified.");
                    }
                    let catalog_root = maki::config::find_catalog_root()?;
                    let engine = QueryEngine::new(&catalog_root);
                    let storage_tags: Vec<String> = tags.iter()
                        .map(|t| maki::tag_util::tag_input_to_storage(t))
                        .collect();
                    let result = engine.tag(&asset_id, &storage_tags, remove)?;

                    if cli.json {
                        println!("{}", serde_json::json!({
                            "changed": result.changed,
                            "tags": result.current_tags,
                        }));
                    } else {
                        let display_changed: Vec<String> = result.changed.iter()
                            .map(|t| maki::tag_util::tag_storage_to_display(t))
                            .collect();
                        let display_tags: Vec<String> = result.current_tags.iter()
                            .map(|t| maki::tag_util::tag_storage_to_display(t))
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
            }
        }
        Commands::Edit { asset_id, name, clear_name, description, clear_description, rating, clear_rating, label, clear_label, clear_tags, date, clear_date, role, variant } => {
            use maki::query::{EditFields, parse_date_input};

            // Handle --role --variant separately from asset-level edits
            if role.is_some() || variant.is_some() {
                let role = role.ok_or_else(|| anyhow::anyhow!("--variant requires --role"))?;
                let variant_hash = variant.ok_or_else(|| anyhow::anyhow!("--role requires --variant"))?;

                let catalog_root = maki::config::find_catalog_root()?;
                let engine = QueryEngine::new(&catalog_root);
                engine.set_variant_role(&asset_id, &variant_hash, &role)?;

                if cli.json {
                    println!("{}", serde_json::json!({
                        "asset_id": asset_id,
                        "variant": variant_hash,
                        "role": role,
                    }));
                } else {
                    let short_hash = &variant_hash[..16.min(variant_hash.len())];
                    println!("Variant {short_hash}… role set to {role}");
                }
                return Ok(());
            }

            if name.is_none() && !clear_name && description.is_none() && !clear_description && rating.is_none() && !clear_rating && label.is_none() && !clear_label && !clear_tags && date.is_none() && !clear_date {
                anyhow::bail!("no edit flags provided. Use --name, --description, --rating, --label, --date, --role/--variant, or --clear-*.");
            }

            // Validate label if provided
            let label_field = if clear_label {
                Some(None)
            } else if let Some(ref l) = label {
                match maki::models::Asset::validate_color_label(l) {
                    Ok(canonical) => Some(canonical),
                    Err(e) => anyhow::bail!(e),
                }
            } else {
                None
            };

            // Parse date if provided
            let date_field = if clear_date {
                Some(None)
            } else if let Some(ref d) = date {
                Some(Some(parse_date_input(d)?))
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
                created_at: date_field,
            };

            let catalog_root = maki::config::find_catalog_root()?;
            let engine = QueryEngine::new(&catalog_root);

            // Clear all tags if requested (before edit, so JSON output includes the result)
            let tags_cleared = if clear_tags {
                let details = engine.show(&asset_id)?;
                if !details.tags.is_empty() {
                    let tag_result = engine.tag(&asset_id, &details.tags, true)?;
                    tag_result.current_tags.is_empty()
                } else {
                    true
                }
            } else {
                false
            };

            let result = engine.edit(&asset_id, fields)?;

            if cli.json {
                let mut json = serde_json::to_value(&result)?;
                if clear_tags {
                    json["tags_cleared"] = serde_json::json!(tags_cleared);
                }
                println!("{}", serde_json::to_string_pretty(&json)?);
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
                if tags_cleared {
                    println!("Tags: cleared");
                }
                // Show date (truncate to YYYY-MM-DD)
                let date_display = result.created_at.split('T').next().unwrap_or(&result.created_at);
                println!("Date: {date_display}");
            }
            Ok(())
        }
        Commands::Group { variant_hashes } => {
            let catalog_root = maki::config::find_catalog_root()?;
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
        Commands::Split { asset_id, variant_hashes } => {
            let catalog_root = maki::config::find_catalog_root()?;
            let engine = QueryEngine::new(&catalog_root);
            let result = engine.split(&asset_id, &variant_hashes)?;

            if cli.json {
                println!("{}", serde_json::to_string(&result)?);
            } else {
                let short_src = &result.source_id[..8];
                println!(
                    "Split {} variant(s) from asset {short_src}",
                    result.new_assets.len()
                );
                for new_asset in &result.new_assets {
                    let short_id = &new_asset.asset_id[..8];
                    println!(
                        "  → {short_id} ({}, {})",
                        new_asset.original_filename, new_asset.variant_hash
                    );
                }
            }
            Ok(())
        }
        Commands::Delete { asset_ids, apply, remove_files } => {
            if remove_files && !apply {
                anyhow::bail!("--remove-files requires --apply");
            }

            // Read from stdin if no IDs provided
            let ids: Vec<String> = if asset_ids.is_empty() {
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
                anyhow::bail!("no asset IDs specified.");
            }

            let catalog_root = maki::config::find_catalog_root()?;
            let config = CatalogConfig::load(&catalog_root)?;
            let service = AssetService::new(&catalog_root, verbosity, &config.preview);

            // Collect face IDs for deleted assets before deletion (for AI cleanup)
            #[cfg(feature = "ai")]
            let ai_cleanup_info: Vec<(String, Vec<String>)> = if apply {
                let catalog = maki::catalog::Catalog::open(&catalog_root)?;
                let _ = maki::face_store::FaceStore::initialize(catalog.conn());
                let face_store = maki::face_store::FaceStore::new(catalog.conn());
                ids.iter().filter_map(|id| {
                    let full_id = catalog.resolve_asset_id(id).ok().flatten()?;
                    let faces = face_store.faces_for_asset(&full_id).unwrap_or_default();
                    let face_ids: Vec<String> = faces.into_iter().map(|f| f.id).collect();
                    Some((full_id, face_ids))
                }).collect()
            } else {
                Vec::new()
            };

            let show_log = cli.log;
            let result = service.delete_assets(
                &ids,
                apply,
                remove_files,
                |id, status, elapsed| {
                    if show_log {
                        let short_id = &id[..8.min(id.len())];
                        match status {
                            maki::asset_service::DeleteStatus::Deleted => {
                                eprintln!("  {short_id} — deleted ({})", format_duration(elapsed));
                            }
                            maki::asset_service::DeleteStatus::NotFound => {
                                eprintln!("  {short_id} — not found");
                            }
                            maki::asset_service::DeleteStatus::Error(msg) => {
                                eprintln!("  {short_id} — error: {msg}");
                            }
                        }
                    }
                },
            )?;

            // Clean up AI files for deleted assets
            #[cfg(feature = "ai")]
            if apply && result.deleted > 0 {
                for (asset_id, face_ids) in &ai_cleanup_info {
                    // Delete ArcFace binaries for each face
                    for face_id in face_ids {
                        maki::face_store::delete_arcface_binary(&catalog_root, face_id);
                    }
                    // Delete SigLIP embedding binary
                    maki::embedding_store::delete_embedding_binary(&catalog_root, "siglip-vit-b16-256", asset_id);
                    maki::embedding_store::delete_embedding_binary(&catalog_root, "siglip-vit-l16-256", asset_id);
                }
                // Update faces/people YAML
                let catalog = maki::catalog::Catalog::open(&catalog_root)?;
                let _ = maki::face_store::FaceStore::initialize(catalog.conn());
                let face_store = maki::face_store::FaceStore::new(catalog.conn());
                let _ = face_store.save_all_yaml(&catalog_root);
            }

            if cli.json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                for err in &result.errors {
                    eprintln!("  {err}");
                }

                if apply {
                    let mut parts = vec![
                        format!("{} deleted", result.deleted),
                    ];
                    if !result.not_found.is_empty() {
                        parts.push(format!("{} not found", result.not_found.len()));
                    }
                    if result.files_removed > 0 {
                        parts.push(format!("{} files removed", result.files_removed));
                    }
                    if result.previews_removed > 0 {
                        parts.push(format!("{} previews removed", result.previews_removed));
                    }
                    println!("Delete complete: {}", parts.join(", "));
                } else {
                    let mut parts = vec![
                        format!("{} would be deleted", result.deleted),
                    ];
                    if !result.not_found.is_empty() {
                        parts.push(format!("{} not found", result.not_found.len()));
                    }
                    println!("Delete (dry run): {}", parts.join(", "));
                    if result.deleted > 0 {
                        println!("  Run with --apply to delete.");
                    }
                }
            }
            Ok(())
        }
        #[cfg(feature = "pro")]
        Commands::Describe {
            query,
            asset,
            volume,
            model,
            endpoint,
            prompt,
            max_tokens,
            timeout,
            mode,
            temperature,
            num_ctx,
            top_p,
            top_k,
            repeat_penalty,
            apply,
            force,
            dry_run,
            check,
            asset_ids,
        } => {
            let catalog_root = maki::config::find_catalog_root()?;
            let config = CatalogConfig::load(&catalog_root)?;

            let endpoint = endpoint.as_deref().unwrap_or(&config.vlm.endpoint);
            let model = model.as_deref().unwrap_or(&config.vlm.model);
            let vlm_mode = maki::vlm::DescribeMode::from_str(&mode)?;

            // Build params: per-model config merged with CLI overrides
            let mut vlm_params = config.vlm.params_for_model(model);
            if let Some(v) = max_tokens { vlm_params.max_tokens = v; }
            if let Some(v) = timeout { vlm_params.timeout = v; }
            if let Some(v) = temperature { vlm_params.temperature = v; }
            if let Some(v) = num_ctx { vlm_params.num_ctx = v; }
            if let Some(v) = top_p { vlm_params.top_p = v; }
            if let Some(v) = top_k { vlm_params.top_k = v; }
            if let Some(v) = repeat_penalty { vlm_params.repeat_penalty = v; }
            if let Some(ref p) = prompt { vlm_params.prompt = Some(p.clone()); }

            if check {
                match maki::vlm::check_endpoint_status(endpoint, vlm_params.timeout, verbosity) {
                    Ok(status) => {
                        let model_status = if status.available_models.is_empty() {
                            format!("Configured model: {model}")
                        } else {
                            match maki::vlm::find_matching_model(model, &status.available_models) {
                                Some(matched) if matched == model => {
                                    format!("Model {model} is available")
                                }
                                Some(matched) => {
                                    format!("Model {matched} matched (from \"{model}\")")
                                }
                                None => {
                                    format!(
                                        "WARNING: model \"{model}\" not found. Pull it with `ollama pull {model}` or set [vlm] model in maki.toml"
                                    )
                                }
                            }
                        };
                        if cli.json {
                            let model_ok = status.available_models.is_empty()
                                || maki::vlm::find_matching_model(model, &status.available_models).is_some();
                            println!("{}", serde_json::json!({
                                "status": "ok",
                                "endpoint": endpoint,
                                "model": model,
                                "model_available": model_ok,
                                "available_models": status.available_models,
                                "message": status.message,
                            }));
                        } else {
                            println!("{}", status.message);
                            println!("{model_status}");
                        }
                    }
                    Err(e) => {
                        if cli.json {
                            println!("{}", serde_json::json!({
                                "status": "error",
                                "endpoint": endpoint,
                                "model": model,
                                "message": format!("{e}"),
                            }));
                        } else {
                            eprintln!("{e}");
                        }
                        anyhow::bail!("vLM endpoint check failed");
                    }
                }
                return Ok(());
            }

            // Merge asset_ids from shell variable expansion into asset/query
            let (query, asset) = merge_trailing_ids(query, asset, &asset_ids);

            if query.is_none() && asset.is_none() && volume.is_none() {
                anyhow::bail!(
                    "No scope specified. Use a query, --asset, or --volume to select assets.\n  \
                     Examples:\n    \
                     maki describe ''                    # all assets\n    \
                     maki describe --asset <id>          # single asset\n    \
                     maki describe 'rating:4+' --apply   # apply to rated assets"
                );
            }

            if verbosity.verbose {
                eprintln!("  VLM: endpoint={endpoint}, model={model}, mode={mode}");
                eprintln!("  VLM: max_tokens={}, timeout={}s, temperature={}, concurrency={}", vlm_params.max_tokens, vlm_params.timeout, vlm_params.temperature, config.vlm.concurrency);
            }

            let service = AssetService::new(&catalog_root, verbosity, &config.preview);

            let show_log = cli.log;
            let result = service.describe(
                query.as_deref(),
                asset.as_deref(),
                volume.as_deref(),
                endpoint,
                model,
                &vlm_params,
                vlm_mode,
                apply,
                force,
                dry_run,
                config.vlm.concurrency,
                |id, status, elapsed| {
                    if show_log {
                        let short_id = &id[..8.min(id.len())];
                        match status {
                            maki::vlm::DescribeStatus::Described => {
                                eprintln!(
                                    "  {short_id} — described ({})",
                                    format_duration(elapsed)
                                );
                            }
                            maki::vlm::DescribeStatus::Skipped(msg) => {
                                eprintln!("  {short_id} — skipped: {msg}");
                            }
                            maki::vlm::DescribeStatus::Error(msg) => {
                                eprintln!("  {short_id} — error: {msg}");
                            }
                        }
                    }
                },
            )?;

            if cli.json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                // Print each result
                for r in &result.results {
                    let short_id = &r.asset_id[..8.min(r.asset_id.len())];
                    match &r.status {
                        maki::vlm::DescribeStatus::Described => {
                            if let Some(ref desc) = r.description {
                                println!("{short_id}: {desc}");
                            }
                            if !r.tags.is_empty() {
                                println!("{short_id}: tags: {}", r.tags.join(", "));
                            }
                        }
                        maki::vlm::DescribeStatus::Skipped(msg) => {
                            if !cli.log {
                                eprintln!("{short_id}: skipped — {msg}");
                            }
                        }
                        maki::vlm::DescribeStatus::Error(msg) => {
                            if !cli.log {
                                eprintln!("{short_id}: error — {msg}");
                            }
                        }
                    }
                }

                let label = if dry_run {
                    "Describe (dry run)"
                } else if apply {
                    "Describe"
                } else {
                    "Describe (report only)"
                };
                let mut parts = vec![format!("{} processed", result.described)];
                if result.skipped > 0 {
                    parts.push(format!("{} skipped", result.skipped));
                }
                if result.failed > 0 {
                    parts.push(format!("{} failed", result.failed));
                }
                if result.tags_applied > 0 {
                    parts.push(format!("{} tags applied", result.tags_applied));
                }
                eprintln!("{label}: {}", parts.join(", "));
                if !apply && !dry_run && result.described > 0 {
                    eprintln!("  Run with --apply to save changes to assets.");
                }
            }
            Ok(())
        }
        #[cfg(feature = "ai")]
        Commands::AutoTag {
            query,
            asset,
            volume,
            model,
            threshold,
            labels,
            apply,
            download,
            remove_model,
            list_models,
            list_labels,
            similar,
            asset_ids,
        } => {
            let (query, asset) = merge_trailing_ids(query, asset, &asset_ids);
            use maki::model_manager::{ModelManager, format_size};

            // List labels can work without a catalog (uses defaults)
            if list_labels {
                use maki::ai::{DEFAULT_LABELS, load_labels_from_file};

                let label_list: Vec<String> = if let Some(ref path) = labels {
                    load_labels_from_file(std::path::Path::new(path))?
                } else {
                    // Try config if catalog exists, fall back to defaults
                    let config_labels = maki::config::find_catalog_root()
                        .ok()
                        .and_then(|root| CatalogConfig::load(&root).ok())
                        .and_then(|c| c.ai.labels.clone());
                    if let Some(ref path) = config_labels {
                        load_labels_from_file(std::path::Path::new(path))?
                    } else {
                        DEFAULT_LABELS.iter().map(|s| s.to_string()).collect()
                    }
                };

                if cli.json {
                    println!("{}", serde_json::to_string_pretty(&label_list)?);
                } else {
                    for label in &label_list {
                        println!("{label}");
                    }
                    eprintln!("\n{} labels", label_list.len());
                }
                return Ok(());
            }

            let catalog_root = maki::config::find_catalog_root()?;
            let config = CatalogConfig::load(&catalog_root)?;

            // Resolve model ID: CLI --model > config ai.model > default.
            // For --download/--remove-model, also accept the positional `query`
            // as a model id when --model isn't given and the positional looks
            // like a known model (this is what users naturally type).
            let model_id_owned: Option<String> = if (download || remove_model) && model.is_none() {
                query
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty() && maki::ai::get_model_spec(s).is_some())
                    .map(|s| s.to_string())
            } else {
                None
            };
            let model_id = model_id_owned
                .as_deref()
                .or(model.as_deref())
                .unwrap_or(&config.ai.model);
            let _spec = maki::ai::get_model_spec(model_id)
                .ok_or_else(|| anyhow::anyhow!(
                    "Unknown model: {model_id}. Run 'maki auto-tag --list-models' to see available models."
                ))?;

            // Resolve model base directory
            let model_dir_str = &config.ai.model_dir;
            let model_base = if model_dir_str.starts_with("~/") {
                let home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE"))?;
                PathBuf::from(home).join(&model_dir_str[2..])
            } else {
                PathBuf::from(model_dir_str)
            };
            let model_dir = model_base.join(model_id);
            let mgr = ModelManager::new(&model_dir, model_id)?;

            // Model management commands
            if download {
                eprintln!("Downloading {} ...", mgr.spec().display_name);
                mgr.ensure_model(|file, current, total| {
                    eprintln!("  [{current}/{total}] {file}");
                })?;
                let total = mgr.total_size();
                if cli.json {
                    println!("{}", serde_json::json!({
                        "status": "downloaded",
                        "model": model_id,
                        "model_dir": model_dir.display().to_string(),
                        "total_size": total,
                    }));
                } else {
                    println!("Model downloaded to {}", model_dir.display());
                    println!("  Total size: {}", format_size(total));
                }
                return Ok(());
            }

            if remove_model {
                mgr.remove_model()?;
                if cli.json {
                    println!("{}", serde_json::json!({
                        "status": "removed",
                        "model": model_id,
                        "model_dir": model_dir.display().to_string(),
                    }));
                } else {
                    println!("Model removed from {}", model_dir.display());
                }
                return Ok(());
            }

            if list_models {
                let models = ModelManager::list_available_models(&model_base);
                if cli.json {
                    let json_models: Vec<serde_json::Value> = models
                        .iter()
                        .map(|(spec, exists, size)| {
                            serde_json::json!({
                                "id": spec.id,
                                "name": spec.display_name,
                                "downloaded": exists,
                                "size": size,
                                "active": spec.id == model_id,
                                "embedding_dim": spec.embedding_dim,
                            })
                        })
                        .collect();
                    println!("{}", serde_json::json!({
                        "model_dir": model_base.display().to_string(),
                        "active_model": model_id,
                        "models": json_models,
                    }));
                } else {
                    println!("Available models (directory: {}):", model_base.display());
                    for (spec, exists, size) in &models {
                        let status = if *exists {
                            format!("downloaded ({})", format_size(*size))
                        } else {
                            "not downloaded".to_string()
                        };
                        let active = if spec.id == model_id { " [active]" } else { "" };
                        println!("  {} — {}{active}", spec.display_name, status);
                        println!("    ID: {}  Embedding dim: {}  Image size: {}px", spec.id, spec.embedding_dim, spec.image_size);
                    }
                }
                return Ok(());
            }

            // Similar search mode
            if let Some(ref similar_id) = similar {
                if !mgr.model_exists() {
                    anyhow::bail!(
                        "Model not downloaded. Run 'maki auto-tag --download --model {model_id}' first."
                    );
                }

                let catalog = maki::catalog::Catalog::open(&catalog_root)?;
                let _ = maki::embedding_store::EmbeddingStore::initialize(catalog.conn());
                let emb_store = maki::embedding_store::EmbeddingStore::new(catalog.conn());

                let full_id = catalog
                    .resolve_asset_id(similar_id)?
                    .ok_or_else(|| anyhow::anyhow!("no asset found matching '{similar_id}'"))?;

                let query_emb = match emb_store.get(&full_id, model_id)? {
                    Some(emb) => emb,
                    None => {
                        // No stored embedding — encode it now
                        let config_preview = &config.preview;
                        let service = AssetService::new(&catalog_root, verbosity, config_preview);
                        let mut ai_model = maki::ai::SigLipModel::load_with_provider(&model_dir, model_id, verbosity, &config.ai.execution_provider)?;
                        let registry = DeviceRegistry::new(&catalog_root);
                        let volumes = registry.list()?;
                        let online_volumes: std::collections::HashMap<String, &maki::models::Volume> =
                            volumes
                                .iter()
                                .filter(|v| v.is_online)
                                .map(|v| (v.id.to_string(), v))
                                .collect();
                        let preview_gen = maki::preview::PreviewGenerator::new(
                            &catalog_root,
                            verbosity,
                            config_preview,
                        );
                        let details = catalog
                            .load_asset_details(&full_id)?
                            .ok_or_else(|| anyhow::anyhow!("asset not found"))?;
                        let image_path = service
                            .find_image_for_ai(&details, &preview_gen, &online_volumes)
                            .ok_or_else(|| {
                                anyhow::anyhow!("no processable image for asset {}", &full_id[..8])
                            })?;
                        let emb = ai_model.encode_image(&image_path)?;
                        emb_store.store(&full_id, &emb, model_id)?;
                        emb
                    }
                };

                let results = emb_store.find_similar(&query_emb, 20, Some(&full_id), model_id)?;

                if cli.json {
                    let json_results: Vec<serde_json::Value> = results
                        .iter()
                        .map(|(id, sim)| {
                            serde_json::json!({
                                "asset_id": id,
                                "similarity": sim,
                            })
                        })
                        .collect();
                    println!("{}", serde_json::to_string_pretty(&json_results)?);
                } else if results.is_empty() {
                    println!("No similar assets found. Run 'maki auto-tag' on more assets to build embeddings.");
                } else {
                    println!(
                        "Assets similar to {} ({} results):",
                        &full_id[..8],
                        results.len()
                    );
                    for (id, sim) in &results {
                        let short_id = &id[..8.min(id.len())];
                        println!("  {short_id}  similarity: {sim:.3}");
                    }
                }
                return Ok(());
            }

            // Main auto-tag flow — require at least one scope filter
            if query.is_none() && asset.is_none() && volume.is_none() && similar.is_none() {
                anyhow::bail!(
                    "No scope specified. Provide a query, --asset, or --volume to select assets.\n  \
                     Examples:\n    \
                     maki auto-tag ''                    # all assets\n    \
                     maki auto-tag --asset <id>          # single asset\n    \
                     maki auto-tag --volume <label>      # one volume\n    \
                     maki auto-tag 'tag:landscape' --apply"
                );
            }

            if !mgr.model_exists() {
                anyhow::bail!(
                    "Model not downloaded. Run 'maki auto-tag --download --model {model_id}' first."
                );
            }

            let threshold = threshold.unwrap_or(config.ai.threshold);

            // Resolve labels
            let label_list: Vec<String> = if let Some(ref labels_path) = labels {
                maki::ai::load_labels_from_file(std::path::Path::new(labels_path))?
            } else if let Some(ref config_labels) = config.ai.labels {
                maki::ai::load_labels_from_file(std::path::Path::new(config_labels))?
            } else {
                maki::ai::DEFAULT_LABELS.iter().map(|s| s.to_string()).collect()
            };

            let prompt = &config.ai.prompt;
            let service = AssetService::new(&catalog_root, verbosity, &config.preview);

            let show_log = cli.log;
            let result = service.auto_tag(
                query.as_deref(),
                asset.as_deref(),
                volume.as_deref(),
                threshold,
                &label_list,
                prompt,
                apply,
                &model_dir,
                model_id,
                &config.ai.execution_provider,
                |id, status, elapsed| {
                    if show_log {
                        let short_id = &id[..8.min(id.len())];
                        match status {
                            maki::ai::AutoTagStatus::Suggested(tags) => {
                                let tag_names: Vec<&str> =
                                    tags.iter().map(|t| t.tag.as_str()).collect();
                                eprintln!(
                                    "  {short_id} — {} tags suggested: {} ({})",
                                    tags.len(),
                                    tag_names.join(", "),
                                    format_duration(elapsed)
                                );
                            }
                            maki::ai::AutoTagStatus::Applied(tags) => {
                                let tag_names: Vec<&str> =
                                    tags.iter().map(|t| t.tag.as_str()).collect();
                                eprintln!(
                                    "  {short_id} — {} tags applied: {} ({})",
                                    tags.len(),
                                    tag_names.join(", "),
                                    format_duration(elapsed)
                                );
                            }
                            maki::ai::AutoTagStatus::Skipped(msg) => {
                                eprintln!("  {short_id} — skipped: {msg}");
                            }
                            maki::ai::AutoTagStatus::Error(msg) => {
                                eprintln!("  {short_id} — error: {msg}");
                            }
                        }
                    }
                },
            )?;

            if cli.json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                for err in &result.errors {
                    eprintln!("  {err}");
                }

                let mode = if apply { "Auto-tag" } else { "Auto-tag (dry run)" };
                let mut parts = vec![
                    format!("{} processed", result.assets_processed),
                ];
                if result.assets_skipped > 0 {
                    parts.push(format!("{} skipped", result.assets_skipped));
                }
                parts.push(format!("{} tags suggested", result.tags_suggested));
                if apply {
                    parts.push(format!("{} tags applied", result.tags_applied));
                }
                if !result.errors.is_empty() {
                    parts.push(format!("{} errors", result.errors.len()));
                }
                println!("{mode}: {}", parts.join(", "));
                if !apply && result.tags_suggested > 0 {
                    println!("  Run with --apply to apply suggested tags.");
                }
            }
            Ok(())
        }
        #[cfg(feature = "ai")]
        Commands::Embed {
            query,
            asset,
            volume,
            model,
            force,
            export,
            asset_ids,
        } => {
            let (query, asset) = merge_trailing_ids(query, asset, &asset_ids);
            use maki::model_manager::ModelManager;

            if export {
                let catalog_root = maki::config::find_catalog_root()?;
                let catalog = maki::catalog::Catalog::open(&catalog_root)?;
                let _ = maki::embedding_store::EmbeddingStore::initialize(catalog.conn());
                let emb_store = maki::embedding_store::EmbeddingStore::new(catalog.conn());

                let mut total = 0u32;
                let models = emb_store.list_models()?;
                for m in &models {
                    let embeddings = emb_store.all_embeddings_for_model(m)?;
                    for (asset_id, emb) in &embeddings {
                        if let Err(e) = maki::embedding_store::write_embedding_binary(&catalog_root, m, asset_id, emb) {
                            eprintln!("  Warning: {}: {e:#}", &asset_id[..8.min(asset_id.len())]);
                        } else {
                            total += 1;
                        }
                    }
                    if !embeddings.is_empty() {
                        eprintln!("  {}: {} embeddings", m, embeddings.len());
                    }
                }
                if cli.json {
                    println!("{}", serde_json::json!({"exported": total, "models": models}));
                } else {
                    println!("Exported {total} embedding binaries");
                }
                return Ok(());
            }

            if query.is_none() && asset.is_none() && volume.is_none() {
                anyhow::bail!(
                    "No scope specified. Provide a query, --asset, or --volume to select assets.\n  \
                     Examples:\n    \
                     maki embed ''                    # all assets\n    \
                     maki embed --asset <id>          # single asset\n    \
                     maki embed --volume <label>      # one volume"
                );
            }

            let catalog_root = maki::config::find_catalog_root()?;
            let config = CatalogConfig::load(&catalog_root)?;

            let model_id = model.as_deref().unwrap_or(&config.ai.model);
            let _spec = maki::ai::get_model_spec(model_id)
                .ok_or_else(|| anyhow::anyhow!(
                    "Unknown model: {model_id}. Run 'maki auto-tag --list-models' to see available models."
                ))?;

            let model_dir_str = &config.ai.model_dir;
            let model_base = if model_dir_str.starts_with("~/") {
                let home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE"))?;
                PathBuf::from(home).join(&model_dir_str[2..])
            } else {
                PathBuf::from(model_dir_str)
            };
            let model_dir = model_base.join(model_id);
            let mgr = ModelManager::new(&model_dir, model_id)?;

            if !mgr.model_exists() {
                anyhow::bail!(
                    "Model not downloaded. Run 'maki auto-tag --download --model {model_id}' first."
                );
            }

            let catalog = maki::catalog::Catalog::open(&catalog_root)?;
            let engine = QueryEngine::new(&catalog_root);

            // Resolve target assets
            let asset_ids: Vec<String> = if let Some(ref id) = asset {
                let full_id = catalog
                    .resolve_asset_id(id)?
                    .ok_or_else(|| anyhow::anyhow!("no asset found matching '{id}'"))?;
                vec![full_id]
            } else {
                let q = if let Some(ref query) = query {
                    let volume_part = volume.as_deref().map(|v| format!(" volume:{v}")).unwrap_or_default();
                    format!("{query}{volume_part}")
                } else if let Some(ref v) = volume {
                    format!("volume:{v}")
                } else {
                    String::new()
                };
                let results = engine.search(&q)?;
                results.into_iter().map(|r| r.asset_id).collect()
            };

            let _ = maki::embedding_store::EmbeddingStore::initialize(catalog.conn());
            let emb_store = maki::embedding_store::EmbeddingStore::new(catalog.conn());

            let registry = DeviceRegistry::new(&catalog_root);
            let volumes_list = registry.list()?;
            let online_volumes: std::collections::HashMap<String, &maki::models::Volume> =
                volumes_list
                    .iter()
                    .filter(|v| v.is_online)
                    .map(|v| (v.id.to_string(), v))
                    .collect();

            let service = AssetService::new(&catalog_root, verbosity, &config.preview);
            let preview_gen = maki::preview::PreviewGenerator::new(
                &catalog_root,
                verbosity,
                &config.preview,
            );

            let mut ai_model = maki::ai::SigLipModel::load_with_provider(&model_dir, model_id, verbosity, &config.ai.execution_provider)?;

            let mut embedded: u32 = 0;
            let mut skipped: u32 = 0;
            let mut errors: Vec<String> = Vec::new();

            for aid in &asset_ids {
                let short_id = &aid[..8_usize.min(aid.len())];
                let asset_start = std::time::Instant::now();

                // Skip if embedding already exists (unless --force)
                if !force && emb_store.has_embedding(aid, model_id) {
                    skipped += 1;
                    if cli.log {
                        eprintln!("  {short_id} — skipped: already exists ({})", format_duration(asset_start.elapsed()));
                    }
                    continue;
                }

                let details = match catalog.load_asset_details(aid)? {
                    Some(d) => d,
                    None => {
                        errors.push(format!("{short_id}: asset not found"));
                        continue;
                    }
                };

                let image_path = match service.find_image_for_ai(&details, &preview_gen, &online_volumes) {
                    Some(p) => p,
                    None => {
                        skipped += 1;
                        if cli.log {
                            eprintln!("  {short_id} — skipped: no processable image");
                        }
                        continue;
                    }
                };

                let ext = image_path
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("");
                if !maki::ai::is_supported_image(ext) {
                    skipped += 1;
                    if cli.log {
                        eprintln!("  {short_id} — skipped: unsupported format '{ext}'");
                    }
                    continue;
                }

                match ai_model.encode_image(&image_path) {
                    Ok(emb) => {
                        if let Err(e) = emb_store.store(aid, &emb, model_id) {
                            errors.push(format!("{short_id}: failed to store: {e}"));
                            continue;
                        }
                        // Write SigLIP embedding binary
                        if let Err(e) = maki::embedding_store::write_embedding_binary(&catalog_root, model_id, aid, &emb) {
                            if verbosity.debug {
                                eprintln!("  {short_id}: embedding binary error: {e:#}");
                            }
                        }
                        embedded += 1;
                        if cli.log {
                            eprintln!("  {short_id} — embedded ({})", format_duration(asset_start.elapsed()));
                        }
                    }
                    Err(e) => {
                        errors.push(format!("{short_id}: {e:#}"));
                    }
                }
            }

            if cli.json {
                println!("{}", serde_json::json!({
                    "embedded": embedded,
                    "skipped": skipped,
                    "errors": errors,
                    "model": model_id,
                    "force": force,
                }));
            } else {
                for err in &errors {
                    eprintln!("  {err}");
                }
                let mut parts = vec![
                    format!("{embedded} embedded"),
                    format!("{skipped} skipped"),
                ];
                if !errors.is_empty() {
                    parts.push(format!("{} errors", errors.len()));
                }
                println!("Embed: {}", parts.join(", "));
            }
            Ok(())
        }
        #[cfg(feature = "ai")]
        Commands::Faces(cmd) => {
            let catalog_root = maki::config::find_catalog_root()?;
            let config = maki::config::CatalogConfig::load(&catalog_root)?;

            let face_model_dir = maki::face::resolve_face_model_dir(&config.ai);

            match cmd {
                FacesCommands::Download => {
                    maki::face::FaceDetector::download_models(&face_model_dir, |name, i, total| {
                        eprintln!("  Downloading {name} ({i}/{total})...");
                    })?;
                    println!("Face models downloaded to {}", face_model_dir.display());
                    Ok(())
                }
                FacesCommands::Status => {
                    let exists = maki::face::FaceDetector::models_exist(&face_model_dir);
                    println!("Face model directory: {}", face_model_dir.display());
                    println!("Models downloaded: {}", if exists { "yes" } else { "no" });

                    let catalog = maki::catalog::Catalog::open(&catalog_root)?;
                    let _ = maki::face_store::FaceStore::initialize(catalog.conn());
                    let store = maki::face_store::FaceStore::new(catalog.conn());
                    println!("Total faces detected: {}", store.total_faces());
                    println!("Total people: {}", store.total_people());

                    if cli.json {
                        let json = serde_json::json!({
                            "model_dir": face_model_dir.to_string_lossy(),
                            "models_downloaded": exists,
                            "total_faces": store.total_faces(),
                            "total_people": store.total_people(),
                        });
                        println!("{}", serde_json::to_string_pretty(&json)?);
                    }
                    Ok(())
                }
                FacesCommands::Detect { query, asset, volume, min_confidence, apply, force } => {
                    if !maki::face::FaceDetector::models_exist(&face_model_dir) {
                        anyhow::bail!(
                            "Face models not downloaded. Run 'maki faces download' first."
                        );
                    }

                    if query.is_none() && asset.is_none() && volume.is_none() {
                        anyhow::bail!(
                            "No scope specified. Use --query, --asset, or --volume to select assets.\n  \
                             Examples:\n    \
                             maki faces detect --query '' --apply     # all assets\n    \
                             maki faces detect --asset <id> --apply   # single asset\n    \
                             maki faces detect --volume <label> --apply"
                        );
                    }

                    let engine = QueryEngine::new(&catalog_root);
                    let service = AssetService::new(&catalog_root, verbosity, &config.preview);

                    // Resolve target assets
                    let catalog = maki::catalog::Catalog::open(&catalog_root)?;
                    let asset_ids: Vec<String> = if let Some(ref aid) = asset {
                        let full_id = catalog
                            .resolve_asset_id(aid)?
                            .ok_or_else(|| anyhow::anyhow!("no asset found matching '{aid}'"))?;
                        vec![full_id]
                    } else {
                        let q = if let Some(ref query) = query {
                            let volume_part = volume.as_deref().map(|v| format!(" volume:{v}")).unwrap_or_default();
                            format!("{query}{volume_part}")
                        } else if let Some(ref v) = volume {
                            format!("volume:{v}")
                        } else {
                            "*".to_string()
                        };
                        let results = engine.search(&q)?;
                        let mut seen = std::collections::HashSet::new();
                        results.into_iter()
                            .filter(|r| seen.insert(r.asset_id.clone()))
                            .map(|r| r.asset_id)
                            .collect()
                    };
                    drop(catalog);

                    let mut detector = maki::face::FaceDetector::load_with_provider(&face_model_dir, verbosity, &config.ai.execution_provider)?;

                    let show_log = cli.log;
                    let result = service.detect_faces(
                        &asset_ids,
                        &mut detector,
                        min_confidence,
                        force,
                        apply,
                        |aid, n, elapsed| {
                            if show_log {
                                let short_id = &aid[..8.min(aid.len())];
                                if n == 0 {
                                    eprintln!("  {short_id} — skipped ({})", format_duration(elapsed));
                                } else {
                                    eprintln!(
                                        "  {short_id} — {} face{} detected ({})",
                                        n, if n == 1 { "" } else { "s" }, format_duration(elapsed)
                                    );
                                }
                            }
                        },
                    )?;

                    let total_faces = result.faces_detected;
                    let total_assets = result.assets_processed;
                    let total_skipped = result.assets_skipped;
                    let errors = result.errors;

                    if cli.json {
                        let json = serde_json::json!({
                            "assets_processed": total_assets,
                            "assets_skipped": total_skipped,
                            "faces_detected": total_faces,
                            "errors": errors,
                            "dry_run": !apply,
                        });
                        println!("{}", serde_json::to_string_pretty(&json)?);
                    } else {
                        for err in &errors {
                            eprintln!("  {err}");
                        }
                        let mode = if apply { "Face detect" } else { "Face detect (dry run)" };
                        let mut parts = vec![
                            format!("{total_assets} assets processed"),
                        ];
                        if total_skipped > 0 {
                            parts.push(format!("{total_skipped} skipped"));
                        }
                        parts.push(format!("{total_faces} faces detected"));
                        if !errors.is_empty() {
                            parts.push(format!("{} errors", errors.len()));
                        }
                        println!("{mode}: {}", parts.join(", "));
                        if !apply && total_faces > 0 {
                            println!("  Run with --apply to store face detections.");
                        }
                    }
                    Ok(())
                }
                FacesCommands::Cluster { query, asset, volume, threshold, apply } => {
                    let catalog = maki::catalog::Catalog::open(&catalog_root)?;
                    let _ = maki::face_store::FaceStore::initialize(catalog.conn());
                    let face_store = maki::face_store::FaceStore::new(catalog.conn());

                    let thresh = threshold.unwrap_or(config.ai.face_cluster_threshold);

                    // Resolve scope to asset IDs (same pattern as maki embed)
                    let scoped_ids: Option<Vec<String>> = if query.is_some() || asset.is_some() || volume.is_some() {
                        let engine = QueryEngine::new(&catalog_root);
                        if let Some(ref a) = asset {
                            let full_id = catalog
                                .resolve_asset_id(a)?
                                .ok_or_else(|| anyhow::anyhow!("no asset found matching '{a}'"))?;
                            Some(vec![full_id])
                        } else {
                            let q = if let Some(ref query) = query {
                                let volume_part = volume.as_deref().map(|v| format!(" volume:{v}")).unwrap_or_default();
                                format!("{query}{volume_part}")
                            } else if let Some(ref v) = volume {
                                format!("volume:{v}")
                            } else {
                                "*".to_string()
                            };
                            let rows = engine.search(&q)?;
                            Some(rows.into_iter().map(|r| r.asset_id).collect())
                        }
                    } else {
                        None
                    };
                    let scope = scoped_ids.as_deref();

                    if apply {
                        let result = face_store.auto_cluster(thresh, scope)?;
                        // Persist faces/people YAML
                        if let Err(e) = face_store.save_all_yaml(&catalog_root) {
                            eprintln!("  Warning: failed to save faces/people YAML: {e:#}");
                        }
                        if cli.json {
                            println!("{}", serde_json::to_string_pretty(&result)?);
                        } else {
                            println!(
                                "Clustered: {} people created, {} faces assigned, {} singletons skipped",
                                result.people_created, result.faces_assigned, result.singletons_skipped
                            );
                        }
                    } else {
                        let (clusters, _unassigned) = face_store.cluster_faces(thresh, scope)?;
                        let total_faces: usize = clusters.iter().map(|c| c.len()).sum();
                        if cli.json {
                            println!("{}", serde_json::json!({
                                "dry_run": true,
                                "clusters": clusters.len(),
                                "faces_in_clusters": total_faces,
                                "cluster_sizes": clusters.iter().map(|c| c.len()).collect::<Vec<_>>(),
                                "threshold": thresh,
                            }));
                        } else {
                            println!("Cluster preview (threshold={thresh:.2}):");
                            for (i, cluster) in clusters.iter().enumerate() {
                                println!("  Cluster {}: {} faces", i + 1, cluster.len());
                            }
                            println!("Total: {} clusters, {} faces", clusters.len(), total_faces);
                            println!("  Run with --apply to create people and assign faces.");
                        }
                    }
                    Ok(())
                }
                FacesCommands::People => {
                    let catalog = maki::catalog::Catalog::open(&catalog_root)?;
                    let _ = maki::face_store::FaceStore::initialize(catalog.conn());
                    let face_store = maki::face_store::FaceStore::new(catalog.conn());

                    let people = face_store.list_people()?;
                    if cli.json {
                        let json_people: Vec<_> = people.iter().map(|(p, count)| {
                            serde_json::json!({
                                "id": p.id,
                                "name": p.name,
                                "representative_face_id": p.representative_face_id,
                                "face_count": count,
                            })
                        }).collect();
                        println!("{}", serde_json::to_string_pretty(&json_people)?);
                    } else {
                        if people.is_empty() {
                            println!("No people found. Run 'maki faces cluster --apply' to create people from detected faces.");
                        } else {
                            println!("{:<10} {:<30} {}", "ID", "Name", "Faces");
                            for (person, count) in &people {
                                let short_id = &person.id[..8.min(person.id.len())];
                                let name = person.name.as_deref().unwrap_or("(unnamed)");
                                println!("{:<10} {:<30} {}", short_id, name, count);
                            }
                        }
                    }
                    Ok(())
                }
                FacesCommands::Name { person_id, name } => {
                    let catalog = maki::catalog::Catalog::open(&catalog_root)?;
                    let _ = maki::face_store::FaceStore::initialize(catalog.conn());
                    let face_store = maki::face_store::FaceStore::new(catalog.conn());

                    // Resolve person ID prefix
                    let full_id = resolve_person_id(&face_store, &person_id)?;
                    face_store.name_person(&full_id, &name)?;
                    let _ = face_store.save_all_yaml(&catalog_root);
                    let short = &full_id[..8.min(full_id.len())];
                    println!("Named person {short} as \"{name}\"");
                    Ok(())
                }
                FacesCommands::Merge { target_id, source_id } => {
                    let catalog = maki::catalog::Catalog::open(&catalog_root)?;
                    let _ = maki::face_store::FaceStore::initialize(catalog.conn());
                    let face_store = maki::face_store::FaceStore::new(catalog.conn());

                    let target = resolve_person_id(&face_store, &target_id)?;
                    let source = resolve_person_id(&face_store, &source_id)?;
                    let moved = face_store.merge_people(&target, &source)?;
                    let _ = face_store.save_all_yaml(&catalog_root);
                    let short_t = &target[..8.min(target.len())];
                    let short_s = &source[..8.min(source.len())];
                    println!("Merged {short_s} into {short_t}: {moved} faces moved");
                    Ok(())
                }
                FacesCommands::DeletePerson { person_id } => {
                    let catalog = maki::catalog::Catalog::open(&catalog_root)?;
                    let _ = maki::face_store::FaceStore::initialize(catalog.conn());
                    let face_store = maki::face_store::FaceStore::new(catalog.conn());

                    let full_id = resolve_person_id(&face_store, &person_id)?;
                    face_store.delete_person(&full_id)?;
                    let _ = face_store.save_all_yaml(&catalog_root);
                    let short = &full_id[..8.min(full_id.len())];
                    println!("Deleted person {short} (faces unassigned)");
                    Ok(())
                }
                FacesCommands::Unassign { face_id } => {
                    let catalog = maki::catalog::Catalog::open(&catalog_root)?;
                    let _ = maki::face_store::FaceStore::initialize(catalog.conn());
                    let face_store = maki::face_store::FaceStore::new(catalog.conn());

                    // Resolve face ID prefix
                    let full_id = resolve_face_id(&face_store, &face_id)?;
                    face_store.unassign_face(&full_id)?;
                    let _ = face_store.save_all_yaml(&catalog_root);
                    let short = &full_id[..8.min(full_id.len())];
                    println!("Unassigned face {short} from its person");
                    Ok(())
                }
                FacesCommands::Export => {
                    let catalog = maki::catalog::Catalog::open(&catalog_root)?;
                    let _ = maki::face_store::FaceStore::initialize(catalog.conn());
                    let face_store = maki::face_store::FaceStore::new(catalog.conn());

                    // Export faces + people YAML
                    face_store.save_all_yaml(&catalog_root)?;
                    let faces_file = face_store.export_all_faces()?;
                    let people_file = face_store.export_all_people()?;

                    // Export ArcFace embedding binaries
                    let mut arcface_count = 0u32;
                    for face in &faces_file.faces {
                        if let Ok(Some(emb)) = face_store.get_face_embedding(&face.id) {
                            if !emb.is_empty() {
                                if let Err(e) = maki::face_store::write_arcface_binary(&catalog_root, &face.id, &emb) {
                                    eprintln!("  Warning: {}: {e:#}", &face.id[..8.min(face.id.len())]);
                                } else {
                                    arcface_count += 1;
                                }
                            }
                        }
                    }

                    if cli.json {
                        println!("{}", serde_json::json!({
                            "faces": faces_file.faces.len(),
                            "people": people_file.people.len(),
                            "arcface_binaries": arcface_count,
                        }));
                    } else {
                        println!("Exported {} faces, {} people to YAML", faces_file.faces.len(), people_file.people.len());
                        println!("Exported {arcface_count} ArcFace embedding binaries");
                    }
                    Ok(())
                }
            }
        }
        Commands::AutoGroup { query, apply, global } => {
            let catalog_root = maki::config::find_catalog_root()?;
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

            let show_log = cli.log;
            let result = if global {
                engine.auto_group_global(&asset_ids, !apply)?
            } else {
                engine.auto_group_with_log(&asset_ids, !apply, |stem, count| {
                    if show_log {
                        eprintln!("  {} — {} asset(s)", stem, count);
                    }
                })?
            };

            if cli.json {
                println!("{}", serde_json::to_string(&result)?);
            } else {
                if result.groups.is_empty() {
                    eprintln!("No groupable assets found");
                } else {
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
            asset_ids,
            target,
            query,
            remove_source,
            create_sidecars,
            dry_run,
        } => {
            let catalog_root = maki::config::find_catalog_root()?;
            let config = CatalogConfig::load(&catalog_root)?;
            let service = AssetService::new(&catalog_root, verbosity, &config.preview);

            // Resolve asset IDs: --query, positional args, or stdin
            let ids: Vec<String> = if let Some(ref q) = query {
                let engine = QueryEngine::new(&catalog_root);
                engine.search(q)?.into_iter().map(|r| r.asset_id).collect()
            } else if asset_ids.is_empty() {
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
                anyhow::bail!("no asset IDs specified. Use --query, positional args, or pipe from stdin.");
            }

            // Determine target volume: --target flag, or second positional arg for single-asset compat
            let target_volume = match target {
                Some(t) => t,
                None => {
                    // Backward compat: `maki relocate <asset-id> <volume>`
                    if ids.len() == 2 && query.is_none() {
                        let vol = ids[1].clone();
                        // Treat as single-asset mode: first arg is asset, second is volume
                        let single_id = ids[0].clone();
                        let result = service.relocate(&single_id, &vol, remove_source, create_sidecars, dry_run)?;

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
                        return Ok(());
                    }
                    anyhow::bail!("--target <volume> is required for batch relocate");
                }
            };

            // Batch relocate
            let total = ids.len();
            let mut total_copied: usize = 0;
            let mut total_skipped: usize = 0;
            let mut total_removed: usize = 0;
            let mut errors: Vec<String> = Vec::new();

            if dry_run && !cli.json {
                println!("Dry run — no changes will be made:");
            }

            for (i, id) in ids.iter().enumerate() {
                match service.relocate(id, &target_volume, remove_source, create_sidecars, dry_run) {
                    Ok(result) => {
                        total_copied += result.copied;
                        total_skipped += result.skipped;
                        total_removed += result.removed;

                        if cli.log {
                            let verb = if remove_source { "moved" } else { "copied" };
                            eprintln!("[{}/{}] {} — {} {verb}, {} skipped",
                                i + 1, total, &id[..8.min(id.len())],
                                result.copied, result.skipped);
                        }
                    }
                    Err(e) => {
                        let msg = format!("{}: {e:#}", &id[..8.min(id.len())]);
                        if cli.log {
                            eprintln!("[{}/{}] ERROR {msg}", i + 1, total);
                        }
                        errors.push(msg);
                    }
                }
            }

            if cli.json {
                println!("{}", serde_json::json!({
                    "assets": total,
                    "copied": total_copied,
                    "skipped": total_skipped,
                    "removed": total_removed,
                    "errors": errors,
                    "dry_run": dry_run,
                }));
            } else {
                let verb = if remove_source { "moved" } else { "copied" };
                let mut parts: Vec<String> = Vec::new();
                parts.push(format!("{total} assets"));
                if total_copied > 0 {
                    parts.push(format!("{total_copied} files {verb}"));
                }
                if total_skipped > 0 {
                    parts.push(format!("{total_skipped} skipped"));
                }
                if !errors.is_empty() {
                    parts.push(format!("{} errors", errors.len()));
                    for e in &errors {
                        eprintln!("  error: {e}");
                    }
                }
                println!("Relocate complete: {}", parts.join(", "));
            }

            Ok(())
        }
        Commands::Verify { paths, volume, asset, include, skip, max_age, force } => {
            use maki::asset_service::FileTypeFilter;

            let catalog_root = maki::config::find_catalog_root()?;
            let config = CatalogConfig::load(&catalog_root)?;
            let service = AssetService::new(&catalog_root, verbosity, &config.preview);

            let max_age_days: Option<u64> = if force {
                None
            } else {
                max_age.or(config.verify.max_age_days)
            };

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
                use maki::asset_service::VerifyStatus;
                service.verify(
                    &canonical_paths,
                    volume.as_deref(),
                    asset.as_deref(),
                    &filter,
                    max_age_days,
                    |path, status, elapsed| {
                        let label = match status {
                            VerifyStatus::Ok => "OK",
                            VerifyStatus::Mismatch => "FAILED",
                            VerifyStatus::Modified => "MODIFIED",
                            VerifyStatus::Missing => "MISSING",
                            VerifyStatus::Skipped => "SKIPPED",
                            VerifyStatus::SkippedRecent => "RECENT",
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
                    max_age_days,
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
                if result.skipped_recent > 0 {
                    let age_label = max_age_days
                        .map(|d| format!("{d} days"))
                        .unwrap_or_else(|| "max age".to_string());
                    parts.push(format!(
                        "{} skipped (verified within {})",
                        result.skipped_recent, age_label
                    ));
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
                anyhow::bail!("verification failed for {} file(s)", result.failed);
            }

            Ok(())
        }
        Commands::Sync { paths, volume, apply, remove_stale } => {
            if paths.is_empty() {
                anyhow::bail!("no paths specified for sync.");
            }
            if remove_stale && !apply {
                anyhow::bail!("--remove-stale requires --apply.");
            }

            let catalog_root = maki::config::find_catalog_root()?;
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

            let service = AssetService::new(&catalog_root, verbosity, &config.preview);
            let result = if cli.log {
                use maki::asset_service::SyncStatus;
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
                if result.orphaned_cleaned > 0 {
                    parts.push(format!("{} orphaned assets cleaned", result.orphaned_cleaned));
                }
                if parts.is_empty() {
                    println!("Sync: nothing to sync");
                } else {
                    if !apply && (result.moved > 0 || result.modified > 0 || result.missing > 0) {
                        eprint!("Dry run — ");
                    }
                    println!("Sync complete: {}", parts.join(", "));
                }
                if !apply && (result.moved > 0 || result.modified > 0) {
                    println!("  Run with --apply to apply changes.");
                }
                if result.missing > 0 && !remove_stale {
                    println!("  Run with --apply --remove-stale to remove missing file records.");
                }
                if result.new_files > 0 {
                    println!("  Tip: run 'maki import' to import new files.");
                }
            }

            Ok(())
        }
        #[cfg(feature = "pro")]
        Commands::SyncMetadata { query, volume, asset, dry_run, media, asset_ids } => {
            let start = std::time::Instant::now();
            let catalog_root = maki::config::find_catalog_root()?;
            let config = CatalogConfig::load(&catalog_root)?;
            let registry = DeviceRegistry::new(&catalog_root);
            let engine = maki::query::QueryEngine::new(&catalog_root);

            // Resolve volume
            let resolved_volume = if let Some(label) = &volume {
                Some(registry.resolve_volume(label)?)
            } else {
                None
            };

            // Resolve scope (query/asset/asset_ids) to individual asset IDs
            let scope = engine.resolve_scope(query.as_deref(), asset.as_deref(), &asset_ids)?;
            let asset_id_list: Vec<Option<String>> = match scope {
                Some(set) => set.into_iter().map(Some).collect(),
                None => vec![None], // process all
            };

            let service = AssetService::new(&catalog_root, verbosity, &config.preview);
            let mut result = maki::asset_service::SyncMetadataResult { dry_run, ..Default::default() };
            for aid in &asset_id_list {
                let r = if cli.log {
                    use maki::asset_service::SyncMetadataStatus;
                    service.sync_metadata(
                        resolved_volume.as_ref(),
                        aid.as_deref(),
                        dry_run,
                        media,
                        &config.import.exclude,
                        |path, status, elapsed| {
                            let label = match status {
                                SyncMetadataStatus::Inbound => "inbound",
                                SyncMetadataStatus::Outbound => "outbound",
                                SyncMetadataStatus::Unchanged => "unchanged",
                                SyncMetadataStatus::Missing => "missing",
                                SyncMetadataStatus::Offline => "offline",
                                SyncMetadataStatus::Conflict => "CONFLICT",
                                SyncMetadataStatus::Error => "error",
                            };
                            let name = path.file_name()
                                .and_then(|n| n.to_str())
                                .unwrap_or_else(|| path.to_str().unwrap_or("?"));
                            eprintln!("  {} — {} ({})", name, label, format_duration(elapsed));
                        },
                    )?
                } else {
                    service.sync_metadata(
                        resolved_volume.as_ref(),
                        aid.as_deref(),
                        dry_run,
                        media,
                        &config.import.exclude,
                        |_, _, _| {},
                    )?
                };
                result.inbound += r.inbound;
                result.outbound += r.outbound;
                result.unchanged += r.unchanged;
                result.conflicts += r.conflicts;
                result.skipped += r.skipped;
                result.media_refreshed += r.media_refreshed;
                result.errors.extend(r.errors);
            }

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
                if result.inbound > 0 {
                    parts.push(format!("{} read from disk", result.inbound));
                }
                if result.outbound > 0 {
                    parts.push(format!("{} written to disk", result.outbound));
                }
                if result.conflicts > 0 {
                    parts.push(format!("{} conflicts (skipped)", result.conflicts));
                }
                if result.media_refreshed > 0 {
                    parts.push(format!("{} media refreshed", result.media_refreshed));
                }
                if result.unchanged > 0 {
                    parts.push(format!("{} unchanged", result.unchanged));
                }
                if result.skipped > 0 {
                    parts.push(format!("{} skipped", result.skipped));
                }
                if parts.is_empty() {
                    println!("Sync metadata: nothing to do");
                } else {
                    println!("Sync metadata: {}", parts.join(", "));
                }

                if result.conflicts > 0 {
                    eprintln!("  Tip: resolve conflicts by running 'maki refresh' (accept external) or 'maki writeback' (keep DAM edits).");
                }
            }

            if cli.timing {
                eprintln!("Time: {:.2}s", start.elapsed().as_secs_f64());
            }

            Ok(())
        }
        Commands::Refresh { paths, volume, asset, dry_run, media, reimport, exif_only } => {
            if reimport || exif_only {
                // --reimport: clear and re-extract all metadata from source files
                // --exif-only: re-extract only EXIF, leave tags/description/rating/label
                let catalog_root = maki::config::find_catalog_root()?;
                let engine = QueryEngine::new(&catalog_root);

                if asset.is_none() && paths.is_empty() {
                    anyhow::bail!("--reimport/--exif-only requires --asset <ID> or asset IDs as arguments");
                }

                let asset_ids: Vec<String> = if let Some(ref id) = asset {
                    vec![id.clone()]
                } else {
                    paths.clone()
                };

                let mut reimported = 0usize;
                for id in &asset_ids {
                    let result = if exif_only {
                        engine.reimport_exif_only(id)
                    } else {
                        engine.reimport_metadata(id)
                    };
                    match result {
                        Ok(tags) => {
                            reimported += 1;
                            if cli.log {
                                let short = if id.len() > 8 { &id[..8] } else { id };
                                eprintln!("  {} — reimported ({} tags)", short, tags.len());
                            }
                        }
                        Err(e) => {
                            eprintln!("  {} — error: {e}", if id.len() > 8 { &id[..8] } else { id.as_str() });
                        }
                    }
                }

                if cli.json {
                    println!("{}", serde_json::json!({ "reimported": reimported }));
                } else {
                    println!("Reimport metadata: {} asset(s) refreshed", reimported);
                }
                return Ok(());
            }

            let catalog_root = maki::config::find_catalog_root()?;
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
                    None => anyhow::bail!("no asset found matching '{prefix}'"),
                }
            } else {
                None
            };

            let service = AssetService::new(&catalog_root, verbosity, &config.preview);
            let result = if cli.log {
                use maki::asset_service::RefreshStatus;
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
        #[cfg(feature = "pro")]
        Commands::Writeback { query, volume, asset, all, dry_run, asset_ids } => {
            let catalog_root = maki::config::find_catalog_root()?;
            let engine = maki::query::QueryEngine::new(&catalog_root);
            let start = std::time::Instant::now();

            let scope = engine.resolve_scope(query.as_deref(), asset.as_deref(), &asset_ids)?;

            let result = engine.writeback(
                volume.as_deref(),
                None, // asset_filter replaced by scope
                scope.as_ref(),
                all,
                dry_run,
                cli.log,
                None,
            )?;

            if cli.json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                if dry_run {
                    eprint!("Dry run: ");
                }
                let mut parts = Vec::new();
                parts.push(format!("{} written", result.written));
                if result.skipped > 0 {
                    parts.push(format!("{} skipped", result.skipped));
                }
                if result.failed > 0 {
                    parts.push(format!("{} failed", result.failed));
                }
                println!("Writeback: {}", parts.join(", "));
                for e in &result.errors {
                    eprintln!("  Error: {e}");
                }
            }

            if cli.timing {
                eprintln!("Time: {:.2}s", start.elapsed().as_secs_f64());
            }

            Ok(())
        }
        Commands::Cleanup { volume, path, list, apply } => {
            let catalog_root = maki::config::find_catalog_root()?;
            let config = CatalogConfig::load(&catalog_root)?;
            let service = AssetService::new(&catalog_root, verbosity, &config.preview);

            // If --path is given without --volume, try to auto-detect the volume
            let volume = if volume.is_none() && path.is_some() {
                let registry = DeviceRegistry::new(&catalog_root);
                let p = std::path::Path::new(path.as_deref().unwrap());
                if p.is_absolute() {
                    // Absolute path: find which volume contains it
                    match registry.find_volume_for_path(p) {
                        Ok(v) => Some(v.label.clone()),
                        Err(_) => None, // fall through — cleanup will check all volumes
                    }
                } else {
                    None
                }
            } else {
                volume
            };

            // Convert absolute --path to relative (strip volume mount point)
            let path_prefix = if let (Some(ref p), Some(ref vol_label)) = (&path, &volume) {
                let abs = std::path::Path::new(p);
                if abs.is_absolute() {
                    let registry = DeviceRegistry::new(&catalog_root);
                    if let Ok(vol) = registry.resolve_volume(vol_label) {
                        abs.strip_prefix(&vol.mount_point)
                            .ok()
                            .and_then(|rel| rel.to_str())
                            .map(|s| s.to_string())
                            .or_else(|| path.clone())
                    } else {
                        path.clone()
                    }
                } else {
                    path.clone()
                }
            } else {
                path
            };

            if verbosity.verbose {
                if let Some(ref prefix) = path_prefix {
                    eprintln!("  Cleanup: path prefix \"{}\"", prefix);
                }
            }

            let show_log = cli.log;
            let show_list = list;
            let result = if show_log || show_list {
                use maki::asset_service::CleanupStatus;
                service.cleanup(
                    volume.as_deref(),
                    path_prefix.as_deref(),
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
                            CleanupStatus::LocationlessVariant => {
                                let name = path.to_str().unwrap_or("?");
                                eprintln!("  {} — locationless variant removed ({})", name, format_duration(elapsed));
                            }
                            CleanupStatus::OrphanedAsset => {
                                let name = path.to_str().unwrap_or("?");
                                eprintln!("  {} — orphaned asset removed ({})", name, format_duration(elapsed));
                            }
                            CleanupStatus::OrphanedFile => {
                                let name = path.file_name()
                                    .and_then(|n| n.to_str())
                                    .unwrap_or_else(|| path.to_str().unwrap_or("?"));
                                eprintln!("  {} — orphaned file removed ({})", name, format_duration(elapsed));
                            }
                            _ => {}
                        }
                    },
                )?
            } else {
                service.cleanup(
                    volume.as_deref(),
                    path_prefix.as_deref(),
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
                    if result.removed_variants > 0 {
                        parts.push(format!("{} locationless variants removed", result.removed_variants));
                    }
                    if result.removed_assets > 0 {
                        parts.push(format!("{} orphaned assets removed", result.removed_assets));
                    }
                    if result.removed_previews > 0 {
                        parts.push(format!("{} orphaned previews removed", result.removed_previews));
                    }
                    if result.removed_smart_previews > 0 {
                        parts.push(format!("{} orphaned smart previews removed", result.removed_smart_previews));
                    }
                    if result.removed_embeddings > 0 {
                        parts.push(format!("{} orphaned embeddings removed", result.removed_embeddings));
                    }
                    if result.removed_face_files > 0 {
                        parts.push(format!("{} orphaned face files removed", result.removed_face_files));
                    }
                    println!("Cleanup complete: {}", parts.join(", "));
                } else {
                    let mut parts = vec![
                        format!("{} checked", result.checked),
                        format!("{} stale", result.stale),
                    ];
                    if result.locationless_variants > 0 {
                        parts.push(format!("{} locationless variants", result.locationless_variants));
                    }
                    if result.orphaned_assets > 0 {
                        parts.push(format!("{} orphaned assets", result.orphaned_assets));
                    }
                    if result.orphaned_previews > 0 {
                        parts.push(format!("{} orphaned previews", result.orphaned_previews));
                    }
                    if result.orphaned_smart_previews > 0 {
                        parts.push(format!("{} orphaned smart previews", result.orphaned_smart_previews));
                    }
                    if result.orphaned_embeddings > 0 {
                        parts.push(format!("{} orphaned embeddings", result.orphaned_embeddings));
                    }
                    if result.orphaned_face_files > 0 {
                        parts.push(format!("{} orphaned face files", result.orphaned_face_files));
                    }
                    println!("Cleanup complete: {}", parts.join(", "));
                    let has_orphans = result.stale > 0
                        || result.locationless_variants > 0
                        || result.orphaned_assets > 0
                        || result.orphaned_previews > 0
                        || result.orphaned_smart_previews > 0
                        || result.orphaned_embeddings > 0
                        || result.orphaned_face_files > 0;
                    if has_orphans {
                        println!("  Run with --apply to remove stale records and orphaned files.");
                    }
                }
            }

            Ok(())
        }
        Commands::Dedup { volume, prefer, filter_format, path, min_copies, apply } => {
            let catalog_root = maki::config::find_catalog_root()?;
            let config = CatalogConfig::load(&catalog_root)?;
            let service = AssetService::new(&catalog_root, verbosity, &config.preview);

            // CLI --prefer overrides config [dedup] prefer
            let effective_prefer = prefer.or(config.dedup.prefer);

            let show_log = cli.log;
            let result = if show_log {
                use maki::asset_service::DedupStatus;
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
            let catalog_root = maki::config::find_catalog_root()?;
            let config = CatalogConfig::load(&catalog_root)?;
            let service = AssetService::new(&catalog_root, verbosity, &config.preview);

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
            use maki::format::{self, OutputFormat};

            if same_volume && cross_volume {
                anyhow::bail!("--same-volume and --cross-volume are mutually exclusive");
            }

            let catalog_root = maki::config::find_catalog_root()?;
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
            use maki::asset_service::FileTypeFilter;

            let catalog_root = maki::config::find_catalog_root()?;
            let config = CatalogConfig::load(&catalog_root)?;
            let preview_gen = maki::preview::PreviewGenerator::new(&catalog_root, verbosity, &config.preview);
            let metadata_store = MetadataStore::new(&catalog_root);
            let registry = maki::device_registry::DeviceRegistry::new(&catalog_root);
            let catalog = maki::catalog::Catalog::open(&catalog_root)?;
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
                let files = maki::asset_service::resolve_files(&canonical_paths, &config.import.exclude);
                let content_store = maki::content_store::ContentStore::new(&catalog_root);

                for file_path in &files {
                    // Filter by extension
                    let ext = file_path
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("");
                    if !ext.is_empty() && !filter.is_importable(ext) {
                        continue;
                    }

                    // Look up variant in catalog: try volume+path first, fall back to content hash
                    let lookup = {
                        let vol = volumes.iter().find(|v| file_path.starts_with(&v.mount_point));
                        if let Some(v) = vol {
                            let relative_path = file_path
                                .strip_prefix(&v.mount_point)
                                .unwrap_or(file_path);
                            catalog.find_variant_by_volume_and_path(
                                &v.id.to_string(),
                                &relative_path.to_string_lossy(),
                            )?
                        } else {
                            None
                        }
                    };
                    // Fall back to hashing the file and looking up by content hash
                    let lookup = match lookup {
                        Some(v) => Some(v),
                        None => {
                            let hash = content_store.hash_file(file_path)?;
                            catalog.get_variant_format(&hash)?.map(|fmt| (hash, fmt))
                        }
                    };

                    if let Some((content_hash, format)) = lookup {
                        let file_start = std::time::Instant::now();
                        // Generate regular preview (always)
                        let result = if force {
                            preview_gen.regenerate(&content_hash, file_path, &format)
                        } else {
                            preview_gen.generate(&content_hash, file_path, &format)
                        };
                        // Also generate smart preview when --smart is set
                        if smart {
                            let _ = if force { preview_gen.regenerate_smart(&content_hash, file_path, &format) }
                            else { preview_gen.generate_smart(&content_hash, file_path, &format) };
                        }
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
                    // Select the best variant for preview generation (respects user override)
                    let idx = asset_data.preview_variant.as_ref()
                        .and_then(|h| asset_data.variants.iter().position(|v| &v.content_hash == h))
                        .or_else(|| maki::models::variant::best_preview_index(&asset_data.variants))
                        .unwrap_or(0);
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
                            // Backfill video metadata if missing
                            if maki::asset_service::determine_asset_type(&variant.format) == maki::models::AssetType::Video
                                && !variant.source_metadata.contains_key("video_duration")
                            {
                                let service = AssetService::new(&catalog_root, verbosity, &config.preview);
                                service.backfill_video_metadata(&asset_data.id.to_string(), &variant.content_hash, &path);
                            }

                            let file_start = std::time::Instant::now();
                            let rotation = asset_data.preview_rotation;
                            // Generate regular preview (always)
                            let result = if force || upgrade {
                                preview_gen.regenerate_with_rotation(&variant.content_hash, &path, &variant.format, rotation)
                            } else {
                                preview_gen.generate(&variant.content_hash, &path, &variant.format)
                            };
                            // Also generate smart preview when --smart is set
                            if smart {
                                let _ = if force || upgrade {
                                    preview_gen.regenerate_smart_with_rotation(&variant.content_hash, &path, &variant.format, rotation)
                                } else {
                                    preview_gen.generate_smart(&variant.content_hash, &path, &variant.format)
                                };
                            }
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
            let catalog_root = maki::config::find_catalog_root()?;
            let config = CatalogConfig::load(&catalog_root)?;
            let service = AssetService::new(&catalog_root, verbosity, &config.preview);

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
                            maki::asset_service::FixRolesStatus::AlreadyCorrect => "ok",
                            maki::asset_service::FixRolesStatus::Fixed => {
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
        Commands::FixDates { query, volume, asset, apply, asset_ids } => {
            let catalog_root = maki::config::find_catalog_root()?;
            let config = CatalogConfig::load(&catalog_root)?;
            let service = AssetService::new(&catalog_root, verbosity, &config.preview);
            let engine = maki::query::QueryEngine::new(&catalog_root);

            // Resolve scope (query/asset/asset_ids) to individual asset IDs
            let scope = engine.resolve_scope(query.as_deref(), asset.as_deref(), &asset_ids)?;
            let asset_id_list: Vec<Option<String>> = match scope {
                Some(set) => set.into_iter().map(Some).collect(),
                None => vec![None], // process all
            };

            let show_log = cli.log;
            let mut result = maki::asset_service::FixDatesResult { dry_run: !apply, ..Default::default() };
            for aid in &asset_id_list {
                let r = service.fix_dates(
                    volume.as_deref(),
                    aid.as_deref(),
                    apply,
                    |name, status, detail| {
                        if show_log {
                            let label = match status {
                                maki::asset_service::FixDatesStatus::AlreadyCorrect => "ok".to_string(),
                                maki::asset_service::FixDatesStatus::NoDate => "no date available".to_string(),
                                maki::asset_service::FixDatesStatus::SkippedOffline => "skipped (volume offline)".to_string(),
                                maki::asset_service::FixDatesStatus::Fixed => {
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
                result.checked += r.checked;
                result.fixed += r.fixed;
                result.already_correct += r.already_correct;
                result.skipped_offline += r.skipped_offline;
                result.no_date += r.no_date;
                result.errors.extend(r.errors);
                for v in r.offline_volumes {
                    if !result.offline_volumes.contains(&v) {
                        result.offline_volumes.push(v);
                    }
                }
            }

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
        Commands::FixRecipes { query, volume, asset, apply, asset_ids } => {
            let catalog_root = maki::config::find_catalog_root()?;
            let config = CatalogConfig::load(&catalog_root)?;
            let service = AssetService::new(&catalog_root, verbosity, &config.preview);
            let engine = maki::query::QueryEngine::new(&catalog_root);

            // Resolve scope (query/asset/asset_ids) to individual asset IDs
            let scope = engine.resolve_scope(query.as_deref(), asset.as_deref(), &asset_ids)?;
            let asset_id_list: Vec<Option<String>> = match scope {
                Some(set) => set.into_iter().map(Some).collect(),
                None => vec![None], // process all
            };

            let show_log = cli.log;
            let mut result = maki::asset_service::FixRecipesResult { dry_run: !apply, ..Default::default() };
            for aid in &asset_id_list {
                let r = service.fix_recipes(
                    volume.as_deref(),
                    aid.as_deref(),
                    apply,
                    |name, status| {
                        if show_log {
                            let label = match status {
                                maki::asset_service::FixRecipesStatus::Reattached => {
                                    if apply { "reattached" } else { "would reattach" }
                                }
                                maki::asset_service::FixRecipesStatus::NoParentFound => "no parent found",
                                maki::asset_service::FixRecipesStatus::Skipped => "skipped",
                            };
                            eprintln!("  {} — {}", name, label);
                        }
                    },
                )?;
                result.checked += r.checked;
                result.reattached += r.reattached;
                result.no_parent += r.no_parent;
                result.skipped += r.skipped;
                result.errors.extend(r.errors);
            }

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
        Commands::CreateSidecars { query, volume, asset, apply, asset_ids } => {
            let catalog_root = maki::config::find_catalog_root()?;
            let catalog = Catalog::open(&catalog_root)?;
            let metadata_store = MetadataStore::new(&catalog_root);
            let content_store = maki::content_store::ContentStore::new(&catalog_root);
            let registry = DeviceRegistry::new(&catalog_root);
            let engine = maki::query::QueryEngine::new(&catalog_root);
            let volumes = registry.list()?;

            // Resolve target volume filter
            let volume_filter = if let Some(ref vol_label) = volume {
                Some(registry.resolve_volume(vol_label)?)
            } else {
                None
            };

            // Resolve asset scope
            let scope = engine.resolve_scope(query.as_deref(), asset.as_deref(), &asset_ids)?;
            let summaries = metadata_store.list()?;
            let ids: Vec<uuid::Uuid> = match scope {
                Some(set) => set.iter().filter_map(|id| id.parse().ok()).collect(),
                None => summaries.iter().map(|s| s.id).collect(),
            };

            let mut created = 0usize;
            let mut skipped = 0usize;
            let mut checked = 0usize;

            for asset_id in &ids {
                let mut asset = match metadata_store.load(*asset_id) {
                    Ok(a) => a,
                    Err(_) => continue,
                };

                let has_metadata = !asset.tags.is_empty()
                    || asset.rating.is_some()
                    || asset.color_label.is_some()
                    || asset.description.is_some();

                if !has_metadata {
                    skipped += 1;
                    continue;
                }

                checked += 1;
                let mut asset_changed = false;

                for variant in &asset.variants {
                    for loc in &variant.locations {
                        // Filter by volume if specified
                        if let Some(ref vf) = volume_filter {
                            if loc.volume_id != vf.id {
                                continue;
                            }
                        }

                        // Check if volume is online
                        let vol = match volumes.iter().find(|v| v.id == loc.volume_id) {
                            Some(v) if v.mount_point.exists() => v,
                            _ => continue,
                        };

                        // Check if this variant already has an XMP recipe on this volume
                        let has_xmp = asset.recipes.iter().any(|r| {
                            r.variant_hash == variant.content_hash
                                && r.location.volume_id == loc.volume_id
                                && r.recipe_type == maki::models::recipe::RecipeType::Sidecar
                        });
                        if has_xmp {
                            continue;
                        }

                        // Build XMP sidecar path
                        let xmp_relative = loc.relative_path.with_extension(
                            format!("{}.xmp", loc.relative_path.extension().unwrap_or_default().to_string_lossy())
                        );
                        let xmp_path = vol.mount_point.join(&xmp_relative);

                        if cli.log {
                            let name = xmp_relative.file_name()
                                .and_then(|n| n.to_str())
                                .unwrap_or("?");
                            if apply {
                                eprintln!("  {} — created", name);
                            } else {
                                eprintln!("  {} — would create", name);
                            }
                        }

                        if apply {
                            if let Some(parent) = xmp_path.parent() {
                                std::fs::create_dir_all(parent)?;
                            }

                            let xmp_content = maki::xmp_reader::create_xmp(
                                &asset.tags,
                                asset.rating,
                                asset.color_label.as_deref(),
                                asset.description.as_deref(),
                            );

                            std::fs::write(&xmp_path, &xmp_content)?;
                            let xmp_hash = content_store.hash_file(&xmp_path)?;

                            let recipe = maki::models::recipe::Recipe {
                                id: uuid::Uuid::new_v4(),
                                variant_hash: variant.content_hash.clone(),
                                software: "MAKI".to_string(),
                                recipe_type: maki::models::recipe::RecipeType::Sidecar,
                                content_hash: xmp_hash,
                                location: maki::models::FileLocation {
                                    volume_id: loc.volume_id,
                                    relative_path: xmp_relative,
                                    verified_at: None,
                                },
                                pending_writeback: false,
                            };
                            catalog.insert_recipe(&recipe)?;
                            asset.recipes.push(recipe);
                            asset_changed = true;
                        }

                        created += 1;
                    }
                }

                if asset_changed {
                    metadata_store.save(&asset)?;
                }
            }

            if cli.json {
                let result = serde_json::json!({
                    "dry_run": !apply,
                    "checked": checked,
                    "created": created,
                    "skipped_no_metadata": skipped,
                });
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                if !apply && created > 0 {
                    eprint!("Dry run — ");
                }
                println!("Create-sidecars: {} checked, {} created, {} skipped (no metadata)", checked, created, skipped);
                if !apply && created > 0 {
                    println!("  Run with --apply to create XMP files.");
                }
            }

            Ok(())
        }
        Commands::RebuildCatalog { asset } => {
            let catalog_root = maki::config::find_catalog_root()?;

            if let Some(ref asset_id) = asset {
                // Per-asset rebuild: delete and re-insert a single asset from its sidecar
                let catalog = Catalog::open(&catalog_root)?;
                let store = MetadataStore::new(&catalog_root);

                // Resolve asset ID (try as UUID first, then prefix match in catalog)
                let uuid: uuid::Uuid = if let Ok(u) = asset_id.parse() {
                    u
                } else if let Some(full) = catalog.resolve_asset_id(asset_id)? {
                    full.parse()?
                } else {
                    // Not in SQLite — try loading sidecar directly
                    anyhow::bail!("asset '{}' not found in catalog. For new assets, use 'maki refresh --reimport --asset {}'", asset_id, asset_id);
                };

                let asset_obj = store.load(uuid)?;
                let id_str = uuid.to_string();

                // Delete all existing rows for this asset (FK checks off for safety)
                let _ = catalog.conn().execute_batch("PRAGMA foreign_keys = OFF");

                // Get all variant hashes (from SQLite, may differ from sidecar)
                let sqlite_hashes: Vec<String> = catalog.conn()
                    .prepare("SELECT content_hash FROM variants WHERE asset_id = ?1")?
                    .query_map(rusqlite::params![&id_str], |r| r.get(0))?
                    .filter_map(|r| r.ok())
                    .collect();

                for hash in &sqlite_hashes {
                    let _ = catalog.conn().execute("DELETE FROM recipes WHERE variant_hash = ?1", rusqlite::params![hash]);
                    let _ = catalog.conn().execute("DELETE FROM file_locations WHERE content_hash = ?1", rusqlite::params![hash]);
                }
                let _ = catalog.conn().execute("DELETE FROM variants WHERE asset_id = ?1", rusqlite::params![&id_str]);
                let _ = catalog.conn().execute("DELETE FROM faces WHERE asset_id = ?1", rusqlite::params![&id_str]);
                let _ = catalog.conn().execute("DELETE FROM embeddings WHERE asset_id = ?1", rusqlite::params![&id_str]);
                let _ = catalog.conn().execute("DELETE FROM collection_assets WHERE asset_id = ?1", rusqlite::params![&id_str]);
                let _ = catalog.conn().execute("DELETE FROM assets WHERE id = ?1", rusqlite::params![&id_str]);

                let _ = catalog.conn().execute_batch("PRAGMA foreign_keys = ON");

                // Re-insert from sidecar
                let registry = DeviceRegistry::new(&catalog_root);
                for volume in registry.list()? {
                    catalog.ensure_volume(&volume)?;
                }

                catalog.insert_asset(&asset_obj)?;
                for variant in &asset_obj.variants {
                    catalog.insert_variant(variant)?;
                    for loc in &variant.locations {
                        catalog.insert_file_location(&variant.content_hash, loc)?;
                    }
                }
                for recipe in &asset_obj.recipes {
                    catalog.insert_recipe(recipe)?;
                }
                catalog.update_denormalized_variant_columns(&asset_obj)?;

                // Restore embedding from binary file if it exists
                #[cfg(feature = "ai")]
                {
                    let emb_store = maki::embedding_store::EmbeddingStore::new(catalog.conn());
                    let emb_base = catalog_root.join("embeddings");
                    if emb_base.exists() {
                        if let Ok(entries) = std::fs::read_dir(&emb_base) {
                            for entry in entries.flatten() {
                                let name = entry.file_name().to_string_lossy().to_string();
                                if name == "arcface" || !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                                    continue;
                                }
                                let prefix = &id_str[..2];
                                let bin_path = emb_base.join(&name).join(prefix).join(format!("{id_str}.bin"));
                                if bin_path.exists() {
                                    if let Ok(data) = std::fs::read(&bin_path) {
                                        let embedding: Vec<f32> = data.chunks_exact(4)
                                            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                                            .collect();
                                        let _ = emb_store.store(&id_str, &embedding, &name);
                                    }
                                }
                            }
                        }
                    }

                    // Restore faces for this asset
                    let face_store = maki::face_store::FaceStore::new(catalog.conn());
                    let faces_file = maki::face_store::load_faces_yaml(&catalog_root).unwrap_or_default();
                    let asset_face_ids: Vec<String> = faces_file.faces.iter()
                        .filter(|f| f.asset_id == id_str)
                        .map(|f| f.id.clone())
                        .collect();
                    if !asset_face_ids.is_empty() {
                        let filtered = maki::face_store::FacesFile {
                            faces: faces_file.faces.into_iter().filter(|f| f.asset_id == id_str).collect(),
                        };
                        let _ = face_store.import_faces_from_yaml(&filtered);
                    }
                    // Restore ArcFace embeddings for this asset's faces
                    let asset_faces = face_store.faces_for_asset(&id_str).unwrap_or_default();
                    for face in &asset_faces {
                        let prefix = &face.id[..2.min(face.id.len())];
                        let bin_path = emb_base.join("arcface").join(prefix).join(format!("{}.bin", face.id));
                        if bin_path.exists() {
                            if let Ok(data) = std::fs::read(&bin_path) {
                                let embedding: Vec<f32> = data.chunks_exact(4)
                                    .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                                    .collect();
                                let _ = face_store.import_face_embedding(&face.id, &embedding);
                            }
                        }
                    }

                    // Update face_count
                    let _ = catalog.conn().execute(
                        "UPDATE assets SET face_count = (SELECT COUNT(*) FROM faces WHERE faces.asset_id = ?1) WHERE id = ?1",
                        rusqlite::params![&id_str],
                    );
                }

                if cli.json {
                    println!("{}", serde_json::json!({
                        "asset_id": id_str,
                        "variants": asset_obj.variants.len(),
                        "recipes": asset_obj.recipes.len(),
                    }));
                } else {
                    println!("Rebuilt asset {}: {} variant(s), {} recipe(s)",
                        &id_str[..8], asset_obj.variants.len(), asset_obj.recipes.len());
                }
                return Ok(());
            }

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
                let col_file = maki::collection::load_yaml(&catalog_root).unwrap_or_default();
                if !col_file.collections.is_empty() {
                    let col_store = maki::collection::CollectionStore::new(catalog.conn());
                    col_store.import_from_yaml(&col_file).unwrap_or(0)
                } else {
                    0
                }
            };

            // Restore stacks from YAML
            let stacks_restored = {
                let stacks_file = maki::stack::load_yaml(&catalog_root).unwrap_or_default();
                if !stacks_file.stacks.is_empty() {
                    let stack_store = maki::stack::StackStore::new(catalog.conn());
                    stack_store.import_from_yaml(&stacks_file).unwrap_or(0)
                } else {
                    0
                }
            };

            // Restore faces, people, and embeddings from files
            #[cfg(feature = "ai")]
            let (people_restored, faces_restored, face_embeddings_restored, embeddings_restored) = {
                let _ = maki::face_store::FaceStore::initialize(catalog.conn());
                let _ = maki::embedding_store::EmbeddingStore::initialize(catalog.conn());
                let face_store = maki::face_store::FaceStore::new(catalog.conn());

                // Import people first (faces reference people via FK)
                let people_file = maki::face_store::load_people_yaml(&catalog_root).unwrap_or_default();
                let people_restored = if !people_file.people.is_empty() {
                    face_store.import_people_from_yaml(&people_file).unwrap_or(0)
                } else {
                    0
                };

                // Import faces (with empty embedding placeholder)
                let faces_file = maki::face_store::load_faces_yaml(&catalog_root).unwrap_or_default();
                let faces_restored = if !faces_file.faces.is_empty() {
                    face_store.import_faces_from_yaml(&faces_file).unwrap_or(0)
                } else {
                    0
                };

                // Restore ArcFace embeddings from binary files
                let mut face_embeddings_restored = 0u32;
                if let Ok(arcface_entries) = maki::face_store::scan_arcface_binaries(&catalog_root) {
                    for (face_id, embedding) in &arcface_entries {
                        if face_store.import_face_embedding(face_id, embedding).is_ok() {
                            face_embeddings_restored += 1;
                        }
                    }
                }

                // Restore SigLIP embeddings from binary files
                let mut embeddings_restored = 0u32;
                let emb_store = maki::embedding_store::EmbeddingStore::new(catalog.conn());
                // Scan all model directories under embeddings/ (skip "arcface")
                let emb_base = catalog_root.join("embeddings");
                if emb_base.exists() {
                    if let Ok(entries) = std::fs::read_dir(&emb_base) {
                        for entry in entries.flatten() {
                            let name = entry.file_name().to_string_lossy().to_string();
                            if name == "arcface" || !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                                continue;
                            }
                            if let Ok(model_entries) = maki::embedding_store::scan_embedding_binaries(&catalog_root, &name) {
                                for (asset_id, embedding) in &model_entries {
                                    if emb_store.store(asset_id, embedding, &name).is_ok() {
                                        embeddings_restored += 1;
                                    }
                                }
                            }
                        }
                    }
                }

                // Backfill face_count denormalized column
                if faces_restored > 0 {
                    let _ = catalog.conn().execute_batch(
                        "UPDATE assets SET face_count = (
                            SELECT COUNT(*) FROM faces WHERE faces.asset_id = assets.id
                        ) WHERE id IN (SELECT DISTINCT asset_id FROM faces)"
                    );
                }

                (people_restored, faces_restored, face_embeddings_restored, embeddings_restored)
            };

            if cli.json {
                #[allow(unused_mut)]
                let mut json = serde_json::json!({
                    "synced": result.synced,
                    "errors": result.errors,
                    "collections_restored": collections_restored,
                    "stacks_restored": stacks_restored,
                });
                #[cfg(feature = "ai")]
                {
                    json["people_restored"] = serde_json::json!(people_restored);
                    json["faces_restored"] = serde_json::json!(faces_restored);
                    json["face_embeddings_restored"] = serde_json::json!(face_embeddings_restored);
                    json["embeddings_restored"] = serde_json::json!(embeddings_restored);
                }
                println!("{}", json);
            } else {
                println!("Rebuild complete: {} asset(s) synced", result.synced);
                if collections_restored > 0 {
                    println!("  {} collection(s) restored", collections_restored);
                }
                if stacks_restored > 0 {
                    println!("  {} stack(s) restored", stacks_restored);
                }
                #[cfg(feature = "ai")]
                {
                    if people_restored > 0 {
                        println!("  {} people restored", people_restored);
                    }
                    if faces_restored > 0 {
                        println!("  {} face(s) restored ({} embeddings)", faces_restored, face_embeddings_restored);
                    }
                    if embeddings_restored > 0 {
                        println!("  {} embedding(s) restored", embeddings_restored);
                    }
                }
                if result.errors > 0 {
                    println!("  {} error(s) encountered", result.errors);
                }
            }
            Ok(())
        }
        Commands::Licenses { summary } => {
            let third_party_url = "https://github.com/thoherr/maki/releases/latest/download/THIRD_PARTY_LICENSES.md";

            if cli.json {
                println!("{}", serde_json::json!({
                    "maki_license": "Apache-2.0",
                    "maki_license_url": "https://github.com/thoherr/maki/blob/main/LICENSE",
                    "third_party_licenses_url": third_party_url,
                    "third_party_summary": "All Rust dependencies use permissive licenses (MIT, Apache-2.0, BSD, ISC, MPL-2.0, NCSA, Unicode, Zlib, BSL-1.0, CC0). See THIRD_PARTY_LICENSES.md in the release archive.",
                    "ai_models": {
                        "license": "Apache-2.0",
                        "source": "Hugging Face (downloaded on demand)",
                        "credit": "Google Research (SigLIP, SigLIP 2)",
                    },
                    "external_tools": "dcraw, libraw, ffmpeg, curl — installed separately by the user under their own licenses; not bundled.",
                }));
                return Ok(());
            }

            println!("MAKI — Media Asset Keeper & Indexer");
            println!("License: Apache-2.0");
            println!("https://github.com/thoherr/maki/blob/main/LICENSE");
            println!();
            println!("Third-party Rust dependencies");
            println!("─────────────────────────────");
            println!("All compiled-in Rust crates use permissive open-source licenses:");
            println!("  Apache-2.0, MIT, BSD-2/3-Clause, ISC, MPL-2.0, NCSA, Unicode, Zlib,");
            println!("  BSL-1.0, CC0, 0BSD.");
            println!();
            println!("The full license text for every dependency is in:");
            println!("  THIRD_PARTY_LICENSES.md (in your MAKI release archive)");
            println!("  {third_party_url}");
            println!();
            println!("AI models (Pro)");
            println!("───────────────");
            println!("SigLIP and SigLIP 2 image-text encoders are downloaded on demand from");
            println!("Hugging Face. Both are released by Google Research under Apache-2.0.");
            println!("Face detection/recognition models (when used) are also Apache-2.0.");
            println!("MAKI does not bundle model weights in the binary.");
            println!();
            println!("External tools");
            println!("──────────────");
            println!("dcraw, libraw, ffmpeg, ffprobe, and curl are called as separate processes");
            println!("when present on the system. They are governed by their own licenses and");
            println!("installed by the user; MAKI does not bundle their code.");

            if !summary {
                println!();
                println!("Run 'maki licenses --summary' for a shorter version, or open the");
                println!("THIRD_PARTY_LICENSES.md file in your release archive for the full");
                println!("text of every dependency's license.");
            }

            Ok(())
        }
        Commands::Doc { document } => {
            let url = match document.to_lowercase().as_str() {
                "manual" | "man" | "guide" => "https://github.com/thoherr/maki/releases/latest/download/maki-manual.pdf",
                "cheatsheet" | "cheat" | "cs" => "https://github.com/thoherr/maki/releases/latest/download/cheat-sheet.pdf",
                "filters" | "search" | "filter" | "sf" => "https://github.com/thoherr/maki/releases/latest/download/search-filters.pdf",
                _ => {
                    anyhow::bail!("unknown document '{}'. Available: manual, cheatsheet, filters", document);
                }
            };
            if cli.json {
                println!("{}", serde_json::json!({ "url": url }));
            } else {
                println!("Opening {url}");
                #[cfg(target_os = "macos")]
                { let _ = std::process::Command::new("open").arg(url).spawn(); }
                #[cfg(target_os = "linux")]
                { let _ = std::process::Command::new("xdg-open").arg(url).spawn(); }
                #[cfg(target_os = "windows")]
                { let _ = std::process::Command::new("cmd").args(["/c", "start", url]).spawn(); }
            }
            Ok(())
        }
        Commands::Serve { port, bind } => {
            let catalog_root = maki::config::find_catalog_root()?;
            let config = CatalogConfig::load(&catalog_root)?;
            let port = port.unwrap_or(config.serve.port);
            let bind = bind.unwrap_or_else(|| config.serve.bind.clone());
            let rt = tokio::runtime::Runtime::new()?;
            #[cfg(feature = "ai")]
            rt.block_on(maki::web::serve(catalog_root, &bind, port, config.preview, cli.log, config.dedup.prefer, config.serve.per_page, config.serve.stroll_neighbors, config.serve.stroll_neighbors_max, config.serve.stroll_fanout, config.serve.stroll_fanout_max, config.serve.stroll_discover_pool, config.ai, config.vlm, config.browse.default_filter, verbosity))?;
            #[cfg(not(feature = "ai"))]
            rt.block_on(maki::web::serve(catalog_root, &bind, port, config.preview, cli.log, config.dedup.prefer, config.serve.per_page, config.serve.stroll_neighbors, config.serve.stroll_neighbors_max, config.serve.stroll_fanout, config.serve.stroll_fanout_max, config.serve.stroll_discover_pool, config.vlm, config.browse.default_filter, verbosity))?;
            Ok(())
        }
        Commands::ContactSheet {
            query,
            output,
            layout,
            columns,
            rows,
            paper,
            landscape,
            title,
            fields,
            sort,
            no_smart,
            group_by,
            margin,
            label_style,
            quality,
            copyright,
            dry_run,
        } => {
            use maki::contact_sheet::{
                generate_contact_sheet, ContactSheetConfig, ContactSheetLayout,
                ContactSheetStatus, GroupByField, LabelStyle, MetadataField, PaperSize,
            };

            let catalog_root = maki::config::find_catalog_root()?;
            let config = CatalogConfig::load(&catalog_root)?;
            let cs_defaults = &config.contact_sheet;

            let cs_layout: ContactSheetLayout = layout.parse()?;
            let cs_paper: PaperSize = paper.parse()?;

            let cs_fields = if let Some(ref f) = fields {
                let parsed: Vec<MetadataField> = f
                    .split(',')
                    .map(|s| s.trim().parse::<MetadataField>())
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                Some(parsed)
            } else if cs_defaults.fields != "filename,date,rating" {
                let parsed: Vec<MetadataField> = cs_defaults
                    .fields
                    .split(',')
                    .map(|s| s.trim().parse::<MetadataField>())
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                Some(parsed)
            } else {
                None // Use layout preset default
            };

            let cs_group_by = group_by
                .map(|g| g.parse::<GroupByField>())
                .transpose()?;

            let cs_label_style: LabelStyle = label_style
                .unwrap_or_else(|| cs_defaults.label_style.clone())
                .parse()?;

            let cs_quality = quality.unwrap_or(cs_defaults.quality);
            let cs_margin = margin.unwrap_or(cs_defaults.margin);

            let cs_config = ContactSheetConfig {
                layout: cs_layout,
                columns,
                rows,
                paper: cs_paper,
                landscape,
                title,
                fields: cs_fields,
                sort,
                use_smart_previews: !no_smart,
                group_by: cs_group_by,
                margin_mm: cs_margin,
                label_style: cs_label_style,
                quality: cs_quality,
                copyright: copyright.or_else(|| cs_defaults.copyright.clone()),
            };

            let output_path = PathBuf::from(&output);
            let show_log = cli.log;

            let result = generate_contact_sheet(
                &catalog_root,
                &query,
                &output_path,
                &cs_config,
                dry_run,
                |msg, status, elapsed| {
                    if show_log || matches!(status, ContactSheetStatus::Complete) {
                        match status {
                            ContactSheetStatus::Rendering => {
                                eprintln!("  {} ({})", msg, format_duration(elapsed));
                            }
                            ContactSheetStatus::Complete => {
                                if !cli.json {
                                    eprintln!("{}", msg);
                                }
                            }
                            ContactSheetStatus::Error => {
                                eprintln!("  Error: {}", msg);
                            }
                        }
                    }
                },
            )?;

            if cli.json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else if !dry_run {
                println!(
                    "Contact sheet: {} assets, {} pages → {}",
                    result.assets, result.pages, result.output,
                );
            }

            Ok(())
        }
        Commands::Export {
            query,
            target,
            layout,
            symlink,
            all_variants,
            include_sidecars,
            dry_run,
            overwrite,
            zip,
        } => {
            use maki::asset_service::{ExportLayout, ExportStatus};

            let export_layout = match layout.as_str() {
                "flat" => ExportLayout::Flat,
                "mirror" => ExportLayout::Mirror,
                _ => anyhow::bail!("unknown layout '{}'. Valid layouts: flat, mirror", layout),
            };

            let catalog_root = maki::config::find_catalog_root()?;
            let config = CatalogConfig::load(&catalog_root)?;
            let service = AssetService::new(&catalog_root, verbosity, &config.preview);

            let show_log = cli.log;
            let log_callback = |path: &std::path::Path, status: &ExportStatus, elapsed: std::time::Duration| {
                if show_log {
                    let name = path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy();
                    match status {
                        ExportStatus::Copied => {
                            eprintln!("  {name} — added ({})", format_duration(elapsed));
                        }
                        ExportStatus::Linked => {
                            eprintln!("  {name} — linked ({})", format_duration(elapsed));
                        }
                        ExportStatus::Skipped => {
                            eprintln!("  {name} — skipped ({})", format_duration(elapsed));
                        }
                        ExportStatus::Error(msg) => {
                            eprintln!("  {name} — error: {msg}");
                        }
                    }
                }
            };

            let result = if zip {
                if symlink {
                    anyhow::bail!("--symlink cannot be used with --zip");
                }
                let zip_path = PathBuf::from(&target);
                // Append .zip if not already present
                let zip_path = if zip_path.extension().and_then(|e| e.to_str()) != Some("zip") {
                    zip_path.with_extension("zip")
                } else {
                    zip_path
                };
                service.export_zip(
                    &query,
                    &zip_path,
                    export_layout,
                    all_variants,
                    include_sidecars,
                    log_callback,
                )?
            } else {
                let target_path = PathBuf::from(&target);
                if !dry_run {
                    std::fs::create_dir_all(&target_path)?;
                }
                service.export(
                    &query,
                    &target_path,
                    export_layout,
                    symlink,
                    all_variants,
                    include_sidecars,
                    dry_run,
                    overwrite,
                    log_callback,
                )?
            };

            if cli.json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                for err in &result.errors {
                    eprintln!("  {err}");
                }

                if dry_run {
                    println!("Export (dry run): {} assets matched, {} files would be exported",
                        result.assets_matched, result.files_exported);
                    if result.sidecars_exported > 0 {
                        println!("  {} sidecars would be exported", result.sidecars_exported);
                    }
                    if result.total_bytes > 0 {
                        println!("  Total size: {}", format_size(result.total_bytes));
                    }
                } else if result.assets_matched == 0 {
                    println!("No assets matched the query.");
                } else if zip {
                    let mut parts = vec![
                        format!("{} files archived", result.files_exported),
                    ];
                    if result.sidecars_exported > 0 {
                        parts.push(format!("{} sidecars", result.sidecars_exported));
                    }
                    println!("Export complete: {}", parts.join(", "));
                    if result.total_bytes > 0 {
                        println!("  Total size: {}", format_size(result.total_bytes));
                    }
                    println!("  Written to: {target}");
                } else {
                    let verb = if symlink { "linked" } else { "copied" };
                    let mut parts = vec![
                        format!("{} files {verb}", result.files_exported),
                    ];
                    if result.sidecars_exported > 0 {
                        parts.push(format!("{} sidecars", result.sidecars_exported));
                    }
                    if result.files_skipped > 0 {
                        parts.push(format!("{} skipped", result.files_skipped));
                    }
                    println!("Export complete: {}", parts.join(", "));
                    if result.total_bytes > 0 {
                        println!("  Total size: {}", format_size(result.total_bytes));
                    }
                }
            }

            Ok(())
        }
        Commands::Stats { types, volumes, tags, verified, all, limit } => {
            let catalog_root = maki::config::find_catalog_root()?;
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
            use maki::format::{self, OutputFormat};

            let catalog_root = maki::config::find_catalog_root()?;
            let catalog = Catalog::open(&catalog_root)?;
            let registry = DeviceRegistry::new(&catalog_root);
            let vol_list = registry.list()?;

            // Exclude media volumes from backup coverage (transient sources like memory cards)
            let volumes_info: Vec<(String, String, bool, Option<String>)> = vol_list
                .iter()
                .filter(|v| v.purpose.as_ref() != Some(&maki::models::VolumePurpose::Media))
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
                    let opts = maki::catalog::SearchOptions {
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
            let catalog_root = maki::config::find_catalog_root()?;
            let catalog = Catalog::open(&catalog_root)?;
            let store = maki::collection::CollectionStore::new(catalog.conn());
            match cmd {
                CollectionCommands::Create { name, description } => {
                    let col = store.create(&name, description.as_deref())?;
                    // Persist to YAML
                    let yaml = store.export_all()?;
                    maki::collection::save_yaml(&catalog_root, &yaml)?;
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
                    use maki::format::{self, OutputFormat};

                    let col = store.get_by_name(&name)?
                        .ok_or_else(|| anyhow::anyhow!("no collection named '{name}'"))?;

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
                        anyhow::bail!("no asset IDs specified.");
                    }
                    let added = store.add_assets(&name, &ids)?;
                    // Persist to YAML
                    let yaml = store.export_all()?;
                    maki::collection::save_yaml(&catalog_root, &yaml)?;
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
                        anyhow::bail!("no asset IDs specified.");
                    }
                    let removed = store.remove_assets(&name, &asset_ids)?;
                    // Persist to YAML
                    let yaml = store.export_all()?;
                    maki::collection::save_yaml(&catalog_root, &yaml)?;
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
                    maki::collection::save_yaml(&catalog_root, &yaml)?;
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
            let catalog_root = maki::config::find_catalog_root()?;
            let catalog = Catalog::open(&catalog_root)?;
            let store = maki::stack::StackStore::new(catalog.conn());
            match cmd {
                StackCommands::Create { asset_ids } => {
                    if asset_ids.len() < 2 {
                        anyhow::bail!("a stack requires at least 2 assets");
                    }
                    let stack = store.create(&asset_ids)?;
                    let yaml = store.export_all()?;
                    maki::stack::save_yaml(&catalog_root, &yaml)?;
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
                    maki::stack::save_yaml(&catalog_root, &yaml)?;
                    if cli.json {
                        println!("{}", serde_json::json!({ "added": added }));
                    } else {
                        println!("Added {} asset(s) to stack", added);
                    }
                    Ok(())
                }
                StackCommands::Remove { asset_ids } => {
                    if asset_ids.is_empty() {
                        anyhow::bail!("no asset IDs specified.");
                    }
                    let removed = store.remove(&asset_ids)?;
                    let yaml = store.export_all()?;
                    maki::stack::save_yaml(&catalog_root, &yaml)?;
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
                    maki::stack::save_yaml(&catalog_root, &yaml)?;
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
                    maki::stack::save_yaml(&catalog_root, &yaml)?;
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
                        .ok_or_else(|| anyhow::anyhow!("asset {asset_id} is not in a stack"))?;
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
            let catalog_root = maki::config::find_catalog_root()?;
            match cmd {
                SavedSearchCommands::Save { name, query, sort, favorite } => {
                    let mut file = maki::saved_search::load(&catalog_root)?;
                    // Replace existing entry with same name, or append
                    let entry = maki::saved_search::SavedSearch {
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
                    maki::saved_search::save(&catalog_root, &file)?;
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
                    let file = maki::saved_search::load(&catalog_root)?;
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
                    use maki::format::{self, OutputFormat};

                    let file = maki::saved_search::load(&catalog_root)?;
                    let ss = maki::saved_search::find_by_name(&file, &name)
                        .ok_or_else(|| anyhow::anyhow!("no saved search named '{name}'"))?;

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
                    let mut file = maki::saved_search::load(&catalog_root)?;
                    let before = file.searches.len();
                    file.searches.retain(|s| s.name != name);
                    if file.searches.len() == before {
                        anyhow::bail!("no saved search named '{name}'");
                    }
                    maki::saved_search::save(&catalog_root, &file)?;
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
        Commands::Migrate => {
            let catalog_root = maki::config::find_catalog_root()?;
            let catalog = Catalog::open_and_migrate(&catalog_root)?;
            #[cfg(feature = "ai")]
            {
                let _ = maki::face_store::FaceStore::initialize(catalog.conn());
                let _ = maki::embedding_store::EmbeddingStore::initialize(catalog.conn());
            }
            // Fix sidecar YAML files with MicrosoftPhoto:Rating percentage values
            let store = maki::metadata_store::MetadataStore::new(&catalog_root);
            let mut fixed_sidecars = 0u32;
            let mut stmt = catalog.conn().prepare(
                "SELECT id FROM assets WHERE rating IS NOT NULL",
            )?;
            let ids: Vec<String> = stmt.query_map([], |row| row.get(0))?
                .filter_map(|r| r.ok())
                .collect();
            drop(stmt);
            for id_str in &ids {
                if let Ok(uuid) = id_str.parse::<uuid::Uuid>() {
                    if let Ok(mut asset) = store.load_raw(uuid) {
                        if let Some(r) = asset.rating {
                            if r > 5 {
                                asset.rating = Some(maki::asset_service::normalize_rating(r));
                                let _ = store.save(&asset);
                                fixed_sidecars += 1;
                            }
                        }
                    }
                }
            }
            let version = catalog.schema_version();
            if cli.json {
                println!("{}", serde_json::json!({"status": "ok", "schema_version": version, "fixed_ratings": fixed_sidecars}));
            } else {
                println!("Schema migrations applied successfully (schema version {version}).");
                if fixed_sidecars > 0 {
                    println!("Fixed {fixed_sidecars} sidecar files with out-of-range rating values.");
                }
            }
            Ok(())
        }
        Commands::Shell { .. } => {
            unreachable!("Shell command is handled before run_command")
        }
    })();

    result.map(|()| _asset_ids)
}

/// Merge trailing asset IDs (from shell variable expansion) into query/asset.
/// Single ID → asset; multiple IDs → `id:xxx id:yyy` query.
#[cfg(any(feature = "pro", feature = "ai"))]
fn merge_trailing_ids(
    query: Option<String>,
    asset: Option<String>,
    asset_ids: &[String],
) -> (Option<String>, Option<String>) {
    if asset_ids.is_empty() {
        return (query, asset);
    }
    if asset_ids.len() == 1 {
        return (query, Some(asset_ids[0].clone()));
    }
    let id_query = asset_ids.iter().map(|id| format!("id:{id}")).collect::<Vec<_>>().join(" ");
    let combined = match query {
        Some(q) => format!("{q} {id_query}"),
        None => id_query,
    };
    (Some(combined), asset)
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

/// Resolve a person ID prefix to a full ID.
#[cfg(feature = "ai")]
fn resolve_person_id(face_store: &maki::face_store::FaceStore, prefix: &str) -> anyhow::Result<String> {
    let people = face_store.list_people()?;
    let matches: Vec<_> = people
        .iter()
        .filter(|(p, _)| p.id.starts_with(prefix))
        .collect();
    match matches.len() {
        0 => anyhow::bail!("no person found matching '{prefix}'"),
        1 => Ok(matches[0].0.id.clone()),
        _ => anyhow::bail!("ambiguous person ID prefix '{prefix}' — matches {} people", matches.len()),
    }
}

/// Resolve a face ID prefix to a full ID.
#[cfg(feature = "ai")]
fn resolve_face_id(face_store: &maki::face_store::FaceStore, prefix: &str) -> anyhow::Result<String> {
    // Try exact match first
    if let Ok(Some(_)) = face_store.get_face(prefix) {
        return Ok(prefix.to_string());
    }
    // Fall back to prefix search via all faces
    let conn = face_store.conn();
    let mut stmt = conn.prepare("SELECT id FROM faces WHERE id LIKE ?1")?;
    let ids: Vec<String> = stmt
        .query_map(rusqlite::params![format!("{prefix}%")], |row| row.get(0))?
        .collect::<Result<Vec<_>, _>>()?;
    match ids.len() {
        0 => anyhow::bail!("no face found matching '{prefix}'"),
        1 => Ok(ids[0].clone()),
        _ => anyhow::bail!("ambiguous face ID prefix '{prefix}' — matches {} faces", ids.len()),
    }
}

fn print_stats_human(stats: &maki::catalog::CatalogStats) {
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

fn print_backup_status_human(result: &maki::catalog::BackupStatusResult) {
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
        println!("  Use 'maki backup-status --at-risk' to list them");
        println!("  Use 'maki backup-status --at-risk -q' for asset IDs (pipeable)");
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
