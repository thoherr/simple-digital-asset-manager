use std::path::PathBuf;

use clap::{Parser, Subcommand};
use dam::asset_service::AssetService;
use dam::catalog::Catalog;
use dam::config::CatalogConfig;
use dam::device_registry::DeviceRegistry;
use dam::metadata_store::MetadataStore;
use dam::query::QueryEngine;

#[derive(Parser)]
#[command(name = "dam", about = "Digital Asset Manager", version)]
struct Cli {
    /// Show elapsed time after command execution
    #[arg(short = 't', long = "time", global = true)]
    timing: bool,

    /// Log individual file progress during import
    #[arg(short = 'l', long = "log", global = true)]
    log: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize a new catalog in the current directory
    Init,

    /// Manage storage volumes
    #[command(subcommand)]
    Volume(VolumeCommands),

    /// Import files into the catalog
    Import {
        /// Paths to files or directories to import
        paths: Vec<String>,

        /// Include additional file type groups (e.g. captureone, documents)
        #[arg(long)]
        include: Vec<String>,

        /// Skip default file type groups (e.g. audio, xmp)
        #[arg(long)]
        skip: Vec<String>,
    },

    /// Search assets
    Search {
        /// Free-text keywords and optional filters (type:image, tag:landscape, format:jpg)
        query: String,
    },

    /// Show asset details
    Show {
        /// Asset ID
        asset_id: String,
    },

    /// Add or remove tags on an asset
    Tag {
        /// Asset ID
        asset_id: String,

        /// Remove the specified tags instead of adding them
        #[arg(long)]
        remove: bool,

        /// Tags to add or remove
        tags: Vec<String>,
    },

    /// Group variants into one asset
    Group {
        /// Content hashes of variants to group
        variant_hashes: Vec<String>,
    },

    /// Copy or move asset files to another volume
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

    /// Check file integrity
    Verify {
        /// Limit verification to a specific volume
        #[arg(long)]
        volume: Option<String>,
    },

    /// Find duplicate files
    Duplicates,

    /// Generate or regenerate preview thumbnails
    GeneratePreviews {
        /// Only generate preview for a specific asset
        #[arg(long)]
        asset: Option<String>,

        /// Force regeneration even if previews already exist
        #[arg(long)]
        force: bool,
    },

    /// Rebuild SQLite catalog from sidecar files
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
    },

    /// List all volumes and their status
    List,
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

            println!("Initialized new dam catalog in {}", catalog_root.display());
            Ok(())
        }
        Commands::Volume(cmd) => match cmd {
            VolumeCommands::Add { label, path } => {
                let catalog_root = dam::config::find_catalog_root()?;
                let registry = DeviceRegistry::new(&catalog_root);
                let volume = registry.register(
                    &label,
                    std::path::Path::new(&path),
                    dam::models::VolumeType::Local,
                )?;
                println!("Registered volume '{}' ({})", volume.label, volume.id);
                println!("  Path: {}", volume.mount_point.display());
                Ok(())
            }
            VolumeCommands::List => {
                let catalog_root = dam::config::find_catalog_root()?;
                let registry = DeviceRegistry::new(&catalog_root);
                let volumes = registry.list()?;
                if volumes.is_empty() {
                    println!("No volumes registered.");
                } else {
                    for v in &volumes {
                        let status = if v.is_online { "online" } else { "offline" };
                        println!("{} ({}) [{}]", v.label, v.id, status);
                        println!("  Path: {}", v.mount_point.display());
                    }
                }
                Ok(())
            }
        },
        Commands::Import {
            paths,
            include,
            skip,
        } => {
            use dam::asset_service::FileTypeFilter;

            let catalog_root = dam::config::find_catalog_root()?;
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

            // Find which volume contains the first path
            let volume = registry.find_volume_for_path(&canonical_paths[0])?;

            let service = AssetService::new(&catalog_root);
            let result = if cli.log {
                use dam::asset_service::FileStatus;
                service.import_with_callback(&canonical_paths, &volume, &filter, |path, status, elapsed| {
                    let label = match status {
                        FileStatus::Imported => "imported",
                        FileStatus::LocationAdded => "location added",
                        FileStatus::Skipped => "skipped",
                        FileStatus::RecipeAttached => "recipe",
                    };
                    let name = path.file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or_else(|| path.to_str().unwrap_or("?"));
                    eprintln!("  {} — {} ({})", name, label, format_duration(elapsed));
                })?
            } else {
                service.import(&canonical_paths, &volume, &filter)?
            };

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
            if result.previews_generated > 0 {
                parts.push(format!("{} preview(s) generated", result.previews_generated));
            }
            if parts.is_empty() {
                println!("Import: nothing to import");
            } else {
                println!("Import complete: {}", parts.join(", "));
            }
            Ok(())
        }
        Commands::Search { query } => {
            let catalog_root = dam::config::find_catalog_root()?;
            let engine = QueryEngine::new(&catalog_root);
            let results = engine.search(&query)?;
            if results.is_empty() {
                println!("No results found.");
            } else {
                for row in &results {
                    let display_name = row
                        .name
                        .as_deref()
                        .unwrap_or(&row.original_filename);
                    let short_id = &row.asset_id[..8];
                    println!(
                        "{}  {} [{}] ({}) — {}",
                        short_id, display_name, row.asset_type, row.format, row.created_at
                    );
                }
                println!("\n{} result(s)", results.len());
            }
            Ok(())
        }
        Commands::Show { asset_id } => {
            let catalog_root = dam::config::find_catalog_root()?;
            let engine = QueryEngine::new(&catalog_root);
            let details = engine.show(&asset_id)?;
            let preview_gen = dam::preview::PreviewGenerator::new(&catalog_root);

            println!("Asset: {}", details.id);
            if let Some(name) = &details.name {
                println!("Name:  {name}");
            }
            println!("Type:  {}", details.asset_type);
            println!("Date:  {}", details.created_at);
            if !details.tags.is_empty() {
                println!("Tags:  {}", details.tags.join(", "));
            }
            if let Some(desc) = &details.description {
                println!("Description: {desc}");
            }

            // Show preview status for the primary variant
            if let Some(primary) = details.variants.first() {
                let preview_path = preview_gen.preview_path(&primary.content_hash);
                if preview_gen.has_preview(&primary.content_hash) {
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

            Ok(())
        }
        Commands::Tag { asset_id, remove, tags } => {
            let catalog_root = dam::config::find_catalog_root()?;
            let engine = QueryEngine::new(&catalog_root);
            let result = engine.tag(&asset_id, &tags, remove)?;

            if !result.changed.is_empty() {
                if remove {
                    println!("Removed tags: {}", result.changed.join(", "));
                } else {
                    println!("Added tags: {}", result.changed.join(", "));
                }
            }
            if result.current_tags.is_empty() {
                println!("Tags: (none)");
            } else {
                println!("Tags: {}", result.current_tags.join(", "));
            }
            Ok(())
        }
        Commands::Group { variant_hashes } => {
            let catalog_root = dam::config::find_catalog_root()?;
            let engine = QueryEngine::new(&catalog_root);
            let result = engine.group(&variant_hashes)?;

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
            Ok(())
        }
        Commands::Relocate {
            asset_id,
            volume,
            remove_source,
            dry_run,
        } => {
            let catalog_root = dam::config::find_catalog_root()?;
            let service = AssetService::new(&catalog_root);
            let result = service.relocate(&asset_id, &volume, remove_source, dry_run)?;

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

            Ok(())
        }
        Commands::Verify { volume } => {
            if let Some(vol) = &volume {
                println!("Verifying volume {vol}...");
            } else {
                println!("Verifying all online volumes...");
            }
            println!("not yet implemented");
            Ok(())
        }
        Commands::Duplicates => {
            let catalog_root = dam::config::find_catalog_root()?;
            let catalog = Catalog::open(&catalog_root)?;
            let entries = catalog.find_duplicates()?;

            if entries.is_empty() {
                println!("No duplicates found.");
            } else {
                for entry in &entries {
                    let display_name = entry
                        .asset_name
                        .as_deref()
                        .unwrap_or(&entry.original_filename);
                    println!(
                        "{} ({}, {})",
                        display_name,
                        entry.format,
                        format_size(entry.file_size)
                    );
                    println!("  Hash: {}", entry.content_hash);
                    for loc in &entry.locations {
                        println!(
                            "    {} \u{2192} {}",
                            loc.volume_label, loc.relative_path
                        );
                    }
                }
                println!(
                    "\n{} file(s) with duplicate locations",
                    entries.len()
                );
            }
            Ok(())
        }
        Commands::GeneratePreviews { asset, force } => {
            let catalog_root = dam::config::find_catalog_root()?;
            let preview_gen = dam::preview::PreviewGenerator::new(&catalog_root);
            let metadata_store = MetadataStore::new(&catalog_root);
            let registry = dam::device_registry::DeviceRegistry::new(&catalog_root);
            let volumes = registry.list()?;

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

            let mut generated = 0usize;
            let mut skipped = 0usize;
            let mut failed = 0usize;

            for asset in &assets {
                // Use the primary (first) variant
                if let Some(variant) = asset.variants.first() {
                    // Try to find a reachable file for this variant
                    let source_path = variant.locations.iter().find_map(|loc| {
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
                        let result = if force {
                            preview_gen.regenerate(&variant.content_hash, &path, &variant.format)
                        } else {
                            preview_gen.generate(&variant.content_hash, &path, &variant.format)
                        };
                        match result {
                            Ok(Some(_)) => generated += 1,
                            Ok(None) => skipped += 1,
                            Err(e) => {
                                eprintln!("  Failed for {}: {e:#}", asset.id);
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

            println!(
                "Generated {} preview(s), {} skipped, {} failed",
                generated, skipped, failed
            );
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

            println!("Rebuild complete: {} asset(s) synced", result.synced);
            if result.errors > 0 {
                println!("  {} error(s) encountered", result.errors);
            }
            Ok(())
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
