//! MAKI binary entrypoint — parses the CLI, dispatches to command handlers,
//! and converts the result to a process exit code.
//!
//! Long match arms are extracted into standalone `run_X_command` functions
//! (see `run_import_command`, `run_tag_command`, etc.). The remaining inline
//! arms are the smaller commands; they get extracted opportunistically as
//! they're touched.

use std::path::PathBuf;

mod commands;
use commands::*;

use clap::{Parser, Subcommand};
use maki::catalog::Catalog;
use maki::cli_output::format_duration;
use maki::config::CatalogConfig;
use maki::device_registry::DeviceRegistry;
use maki::query::QueryEngine;

/// Long help text for the `maki search QUERY` argument. Shown on
/// `maki search --help` and `maki help search`. Short help (shown on
/// `-h`) stays compact; this expands into a categorised reference
/// sized to fit one terminal screen. Full details in
/// doc/manual/reference/06-search-filters.md and the printable PDF
/// at `maki doc filters`.
const SEARCH_QUERY_LONG_HELP: &str = "\
Free-text keywords and filter expressions.

Combining: space = AND (repeat a filter to AND), comma = OR within one filter.
Prefix `-` on any filter to exclude (e.g. `-tag:rejected`).

TEXT & METADATA
  tag:landscape           tag match at any level
  tag:=Foo                whole-path match (exact tag value)
  tag:/Foo                leaf only at any level
  tag:^Foo                case-sensitive
  tag:|wed                prefix anchor (wedding, wedding-2024, ...)
  type:image              asset type (image, video, audio, document)
  format:nef              file format (e.g. format:jpg,jpeg)
  label:Red / label:none  colour label (or \"unlabeled\")
  camera:fuji             camera (substring match on EXIF)
  lens:56mm               lens (substring)
  description:cat         description substring (alias: desc:)
  collection:Fav          collection membership
  path:Pictures/2026      path prefix (* wildcards supported)
  id:72a0                 asset ID prefix
  meta:key=val            raw source-metadata field match

NUMERIC (all support: N exact / N+ min / A-B range / A,B OR-list)
  rating:3+               rating (0 = unrated)
  tagcount:0              number of intentional (leaf) tags on the asset
  iso:100-800             ISO range
  focal:35-70             focal length (mm)
  f:1.4-2.8               aperture
  width:4000+             minimum pixel width
  height:2000+            minimum pixel height
  copies:2+               number of file copies across volumes
  variants:2+             number of variants on the asset
  scattered:2+            distinct directories (add `/N` for depth)
  duration:30+            video duration (seconds)

DATE
  date:2026               year prefix (also 2026-03, 2026-03-15)
  dateFrom:2026-01        from (inclusive)
  dateUntil:2026-12       until (inclusive)

STATUS
  orphan:true / orphan:false    with/without file locations
  missing:true                  files missing on disk
  stale:30                      unverified for N days
  stacked:true / stacked:false  in a stack (collapsed in browse)
  volume:Archive                on specific volume (or volume:none)
  geo:any / geo:none            has / lacks GPS coordinates
  geo:<S,W,N,E>                 GPS bounding box
  codec:h264                    video codec (substring)

PRO (require --features pro)
  faces:2+ / faces:any    face count or any-faces filter
  person:Alice            named person (repeat to AND, comma to OR)
  similar:<id>            visually similar to an asset
  min_sim:90              similarity threshold (0-100%)
  text:sunset             CLIP text-to-image search
  embed:any / embed:none  has / lacks SigLIP embedding

COMBINING
  tag:a,b                 a OR b (comma within one filter)
  tag:a tag:b             a AND b (repeat the filter)
  -tag:rejected           NOT

Full reference: doc/manual/reference/06-search-filters.md
Printable 2-page PDF: `maki doc filters`
";

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
        /// Free-text keywords and filter expressions. Run with --help for the full filter list.
        #[arg(long_help = SEARCH_QUERY_LONG_HELP)]
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

    /// Catalog health overview — pending cleanup, work, backup coverage, volumes
    #[command(display_order = 34)]
    Status {
        /// Minimum copies for backup coverage (default: 2)
        #[arg(long, default_value = "2", display_order = 10)]
        min_copies: u64,
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

    /// Write pending metadata changes to XMP recipe files
    ///
    /// Manually flushes recipe edits (rating, label, description, tags)
    /// from the catalog to the .xmp files on disk. Always works regardless
    /// of `[writeback] enabled` in maki.toml — that config controls only
    /// AUTOMATIC writeback on every edit; this command is the explicit
    /// manual flush, intended for users who keep auto-flush off as a
    /// safety net but want to push staged changes for a specific set of
    /// assets.
    ///
    /// By default writes only recipes flagged `pending_writeback`. With
    /// `--all` (combined with a query/volume) it rewrites every XMP in
    /// the matching set, even those without pending markers — useful for
    /// rematerialising catalog metadata onto disk after large catalog-
    /// only edits.
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

        /// Write all XMP recipes in the matching set, not just those
        /// flagged pending. Use this together with a query/volume filter
        /// to rematerialise catalog metadata for a known asset set.
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
        /// Which document: manual, cheatsheet, filters, tagging (default: manual)
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

    /// Split one tag into two or more across all assets
    ///
    /// Replaces OLD_TAG with every tag in NEW_TAGS (two or more). Acts only on
    /// assets where OLD_TAG is a leaf — assets where OLD_TAG has descendants
    /// are skipped (use `tag rename` for cascading renames). Use `--keep` to
    /// keep OLD_TAG in place and add the new tags alongside (a pure
    /// "duplicate" / "copy" operation).
    ///
    /// Examples:
    ///   maki tag split "A & B" A B --apply
    ///   maki tag split "concert-jane-2024" "subject|performing arts|concert" "event|concert-jane-2024" --apply
    ///   maki tag split sunset "color|warm" --keep --apply
    Split {
        /// Current tag name. Accepts the same optional markers as `tag rename`
        /// (`=`, `/`, `^`). Split is always exact-tag-only, so `=` and `/`
        /// are redundant no-ops; `^` enables case-sensitive matching.
        old_tag: String,

        /// Target tag names (one or more). Always taken literally.
        #[arg(required = true, num_args = 1..)]
        new_tags: Vec<String>,

        /// Keep the source tag in place and only add the targets
        /// (additive / copy semantics instead of replace).
        #[arg(long)]
        keep: bool,

        /// Apply changes (default: report-only)
        #[arg(long)]
        apply: bool,
    },

    /// Remove all tags from an asset
    Clear {
        /// Asset ID (or unique prefix)
        asset_id: String,
    },

    /// Delete a tag from every asset that has it (cascades to descendants by default)
    Delete {
        /// Tag to delete. Use the same markers as `tag rename`:
        /// `=tag` or `/tag` for leaf-only (skip assets where the tag has descendants),
        /// `^tag` for case-sensitive match.
        tag: String,

        /// Apply changes (default: report-only)
        #[arg(long)]
        apply: bool,
    },

    /// Normalise tag values to Unicode NFC across the catalog (collapse NFC/NFD duplicates)
    #[command(name = "fix-unicode")]
    FixUnicode {
        /// Apply changes (default: report-only)
        #[arg(long)]
        apply: bool,
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

    /// Export current tag tree as a vocabulary file
    #[command(name = "export-vocabulary")]
    ExportVocabulary {
        /// Output file (default: vocabulary.{yaml,txt,json} in catalog root, depending on --format)
        #[arg(long)]
        output: Option<String>,

        /// Output format: yaml (MAKI vocabulary, default), text (tab-indented keyword list for Lightroom/Capture One), or json (nested object with counts, for programmatic use)
        #[arg(long, default_value = "yaml")]
        format: String,

        /// Remove vocabulary entries that have no assets (only keep used tags)
        #[arg(long)]
        prune: bool,

        /// Export only the built-in default vocabulary (ignore catalog tags and existing vocabulary.yaml)
        #[arg(long)]
        default: bool,

        /// Annotate each tag with its asset count.
        ///
        /// YAML: emitted as a `# N assets` trailing comment (still valid YAML;
        /// MAKI's autocomplete loader ignores comments). JSON: every node has
        /// a `count` field already, so the flag is implied. Text format
        /// (Lightroom / Capture One keyword text) doesn't support comments —
        /// the flag is silently ignored to keep the file importable.
        #[arg(long)]
        counts: bool,
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

        /// Remove the matched tag from ALL assets that carry it, including
        /// orphans (single-asset tags that can't form a stack) and already-
        /// stacked assets. Useful for cleaning up after a migration.
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

        /// Similarity threshold for clustering (0.0–1.0). Higher = stricter.
        /// Typical range: 0.55–0.75 for the bundled face model.
        #[arg(long)]
        threshold: Option<f32>,

        /// Skip face detections with confidence below this value (0.0–1.0).
        /// Low-quality detections (blurry, partial, profile) produce noisy
        /// embeddings that hurt clustering. Defaults to `[ai] face_min_confidence`
        /// from maki.toml (default 0.5).
        #[arg(long)]
        min_confidence: Option<f32>,

        /// Apply clustering (default: dry-run showing cluster sizes)
        #[arg(long)]
        apply: bool,
    },

    /// Delete face records that are not assigned to any person
    Clean {
        /// Apply the deletion (default: report-only)
        #[arg(long)]
        apply: bool,
    },

    /// Save aligned 112x112 face crops for visual debugging
    DumpAligned {
        /// Search query to scope which assets
        #[arg(long)]
        query: Option<String>,

        /// Specific asset (ID or prefix)
        #[arg(long)]
        asset: Option<String>,

        /// Output directory for aligned crops
        #[arg(long, default_value = "./aligned-faces")]
        output: PathBuf,

        /// Maximum number of faces to save (0 = unlimited)
        #[arg(long, default_value = "20")]
        limit: usize,
    },

    /// Analyze face embedding similarity distribution (diagnostic)
    Similarity {
        /// Search query to scope which assets' faces to analyze
        #[arg(long)]
        query: Option<String>,

        /// Analyze faces from a specific asset (ID or prefix)
        #[arg(long)]
        asset: Option<String>,

        /// Limit to faces on assets from a specific volume
        #[arg(long)]
        volume: Option<String>,

        /// Minimum confidence for faces to include
        #[arg(long, default_value = "0.0")]
        min_confidence: f32,

        /// Show top-N nearest neighbors for each face (0 = none, just summary)
        #[arg(long, default_value = "0")]
        top: usize,

        /// Include already-assigned faces (default: only unassigned)
        #[arg(long)]
        all: bool,
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
  doc                Open documentation in the browser (manual, cheatsheet, filters, tagging)
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
        Commands::Volume(cmd) => run_volume_command(cmd, cli.json, cli.log, verbosity),
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
        } => run_import_command(
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
            cli.json,
            cli.log,
            verbosity,
        ),
        Commands::Search { query, format, quiet } => {
            use maki::format::{self, OutputFormat};

            let (catalog_root, config) = maki::config::load_config()?;
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
                            // Hint: if the query has a filter like tag:X followed by
                            // unquoted words, the user likely forgot inner quotes.
                            let parsed = maki::query::parse_search_query(&query);
                            let has_filter = !parsed.tags.is_empty() || !parsed.cameras.is_empty()
                                || !parsed.lenses.is_empty() || !parsed.descriptions.is_empty()
                                || !parsed.collections.is_empty() || !parsed.path_prefixes.is_empty();
                            if has_filter && parsed.text.is_some() {
                                eprintln!("Hint: values with spaces need inner quotes, e.g. tag:\"my tag\"");
                                eprintln!("  Shell example: maki search 'tag:\"my tag\" rating:3+'");
                            }
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
        Commands::Show {
            asset_id,
            locations,
        } => run_show_command(
            asset_id,
            locations,
            cli.json,
            cli.log,
            verbosity,
        ),
        Commands::Preview {
            asset_id,
        } => run_preview_command(
            asset_id,
            cli.json,
            cli.log,
            verbosity,
        ),
        Commands::Tag { asset_id, remove, tags, subcmd } => run_tag_command(
            asset_id, remove, tags, subcmd, cli.json, cli.log,
        ),
        Commands::Edit {
            asset_id,
            name,
            clear_name,
            description,
            clear_description,
            rating,
            clear_rating,
            label,
            clear_label,
            clear_tags,
            date,
            clear_date,
            role,
            variant,
        } => run_edit_command(
            asset_id,
            name,
            clear_name,
            description,
            clear_description,
            rating,
            clear_rating,
            label,
            clear_label,
            clear_tags,
            date,
            clear_date,
            role,
            variant,
            cli.json,
            cli.log,
            verbosity,
        ),
        Commands::Group {
            variant_hashes,
        } => run_group_command(
            variant_hashes,
            cli.json,
            cli.log,
            verbosity,
        ),
        Commands::Split {
            asset_id,
            variant_hashes,
        } => run_split_command(
            asset_id,
            variant_hashes,
            cli.json,
            cli.log,
            verbosity,
        ),
        Commands::Delete {
            asset_ids,
            apply,
            remove_files,
        } => run_delete_command(
            asset_ids,
            apply,
            remove_files,
            cli.json,
            cli.log,
            verbosity,
        ),
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
        } => run_describe_command(
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
            cli.json,
            cli.log,
            verbosity,
        ),
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
        } => run_auto_tag_command(
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
            cli.json,
            cli.log,
            verbosity,
        ),
        #[cfg(feature = "ai")]
        Commands::Embed {
            query,
            asset,
            volume,
            model,
            force,
            export,
            asset_ids,
        } => run_embed_command(
            query,
            asset,
            volume,
            model,
            force,
            export,
            asset_ids,
            cli.json,
            cli.log,
            verbosity,
        ),
        #[cfg(feature = "ai")]
        Commands::Faces(cmd) => {
            run_faces_command(cmd, cli.json, cli.log, verbosity)?;
            Ok(())
        }

        Commands::AutoGroup {
            query,
            apply,
            global,
        } => run_auto_group_command(
            query,
            apply,
            global,
            cli.json,
            cli.log,
            verbosity,
        ),
        Commands::Relocate {
            asset_ids,
            target,
            query,
            remove_source,
            create_sidecars,
            dry_run,
        } => run_relocate_command(
            asset_ids,
            target,
            query,
            remove_source,
            create_sidecars,
            dry_run,
            cli.json,
            cli.log,
            verbosity,
        ),
        Commands::Verify {
            paths,
            volume,
            asset,
            include,
            skip,
            max_age,
            force,
        } => run_verify_command(
            paths,
            volume,
            asset,
            include,
            skip,
            max_age,
            force,
            cli.json,
            cli.log,
            verbosity,
        ),
        Commands::Sync {
            paths,
            volume,
            apply,
            remove_stale,
        } => run_sync_command(
            paths,
            volume,
            apply,
            remove_stale,
            cli.json,
            cli.log,
            verbosity,
        ),
        #[cfg(feature = "pro")]
        Commands::SyncMetadata {
            query,
            volume,
            asset,
            dry_run,
            media,
            asset_ids,
        } => run_sync_metadata_command(
            query,
            volume,
            asset,
            dry_run,
            media,
            asset_ids,
            cli.json,
            cli.log,
            verbosity,
        ),
        Commands::Refresh {
            paths,
            volume,
            asset,
            dry_run,
            media,
            reimport,
            exif_only,
        } => run_refresh_command(
            paths,
            volume,
            asset,
            dry_run,
            media,
            reimport,
            exif_only,
            cli.json,
            cli.log,
            verbosity,
        ),
        #[cfg(feature = "pro")]
        Commands::Writeback {
            query,
            volume,
            asset,
            all,
            dry_run,
            asset_ids,
        } => run_writeback_command(
            query,
            volume,
            asset,
            all,
            dry_run,
            asset_ids,
            cli.json,
            cli.log,
            verbosity,
        ),
        Commands::Cleanup {
            volume,
            path,
            list,
            apply,
        } => run_cleanup_command(
            volume,
            path,
            list,
            apply,
            cli.json,
            cli.log,
            verbosity,
        ),
        Commands::Dedup {
            volume,
            prefer,
            filter_format,
            path,
            min_copies,
            apply,
        } => run_dedup_command(
            volume,
            prefer,
            filter_format,
            path,
            min_copies,
            apply,
            cli.json,
            cli.log,
            verbosity,
        ),
        Commands::UpdateLocation {
            asset_id,
            from,
            to,
            volume,
        } => run_update_location_command(
            asset_id,
            from,
            to,
            volume,
            cli.json,
            cli.log,
            verbosity,
        ),
        Commands::Duplicates {
            format,
            same_volume,
            cross_volume,
            volume,
            filter_format,
            path,
        } => run_duplicates_command(
            format,
            same_volume,
            cross_volume,
            volume,
            filter_format,
            path,
            cli.json,
            cli.log,
            verbosity,
        ),
        Commands::GeneratePreviews {
            paths,
            volume,
            asset,
            include,
            skip,
            force,
            upgrade,
            smart,
        } => run_generate_previews_command(
            paths,
            volume,
            asset,
            include,
            skip,
            force,
            upgrade,
            smart,
            cli.json,
            cli.log,
            verbosity,
        ),
        Commands::FixRoles {
            paths,
            volume,
            asset,
            apply,
        } => run_fix_roles_command(
            paths,
            volume,
            asset,
            apply,
            cli.json,
            cli.log,
            verbosity,
        ),
        Commands::FixDates {
            query,
            volume,
            asset,
            apply,
            asset_ids,
        } => run_fix_dates_command(
            query,
            volume,
            asset,
            apply,
            asset_ids,
            cli.json,
            cli.log,
            verbosity,
        ),
        Commands::FixRecipes {
            query,
            volume,
            asset,
            apply,
            asset_ids,
        } => run_fix_recipes_command(
            query,
            volume,
            asset,
            apply,
            asset_ids,
            cli.json,
            cli.log,
            verbosity,
        ),
        Commands::CreateSidecars {
            query,
            volume,
            asset,
            apply,
            asset_ids,
        } => run_create_sidecars_command(
            query,
            volume,
            asset,
            apply,
            asset_ids,
            cli.json,
            cli.log,
            verbosity,
        ),
        Commands::RebuildCatalog { asset } => run_rebuild_catalog_command(asset, cli.json),
        Commands::Licenses {
            summary,
        } => run_licenses_command(
            summary,
            cli.json,
            cli.log,
            verbosity,
        ),
        Commands::Doc {
            document,
        } => run_doc_command(
            document,
            cli.json,
            cli.log,
            verbosity,
        ),
        Commands::Serve {
            port,
            bind,
        } => run_serve_command(
            port,
            bind,
            cli.json,
            cli.log,
            verbosity,
        ),
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
        } => run_contact_sheet_command(
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
            cli.json,
            cli.log,
            verbosity,
        ),
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
        } => run_export_command(
            query,
            target,
            layout,
            symlink,
            all_variants,
            include_sidecars,
            dry_run,
            overwrite,
            zip,
            cli.json,
            cli.log,
            verbosity,
        ),
        Commands::Stats {
            types,
            volumes,
            tags,
            verified,
            all,
            limit,
        } => run_stats_command(
            types,
            volumes,
            tags,
            verified,
            all,
            limit,
            cli.json,
            cli.log,
            verbosity,
        ),
        Commands::Status {
            min_copies,
        } => run_status_command(
            min_copies,
            cli.json,
            cli.log,
            verbosity,
        ),
        Commands::BackupStatus {
            query,
            at_risk,
            min_copies,
            volume,
            format,
            quiet,
        } => run_backup_status_command(
            query,
            at_risk,
            min_copies,
            volume,
            format,
            quiet,
            cli.json,
            cli.log,
            verbosity,
        ),
        Commands::Collection(cmd) => run_collection_command(cmd, cli.json, cli.log, verbosity),
        Commands::Stack(cmd) => run_stack_command(cmd, cli.json, cli.log, verbosity),
        Commands::SavedSearch(cmd) => run_saved_search_command(cmd, cli.json, cli.log, verbosity),
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























































































