use std::path::PathBuf;

use clap::{Parser, Subcommand};
use dam::asset_service::AssetService;
use dam::catalog::Catalog;
use dam::config::CatalogConfig;
use dam::device_registry::DeviceRegistry;
use dam::query::QueryEngine;

#[derive(Parser)]
#[command(name = "dam", about = "Digital Asset Manager", version)]
struct Cli {
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
    },

    /// Search assets
    Search {
        /// Search query
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

    /// Move asset to another volume
    Relocate {
        /// Asset ID
        asset_id: String,

        /// Target volume label or ID
        volume: String,
    },

    /// Check file integrity
    Verify {
        /// Limit verification to a specific volume
        #[arg(long)]
        volume: Option<String>,
    },

    /// Find duplicate files
    Duplicates,

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
        Commands::Import { paths } => {
            let catalog_root = dam::config::find_catalog_root()?;
            let registry = DeviceRegistry::new(&catalog_root);

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
            let result = service.import(&canonical_paths, &volume)?;

            println!(
                "Import complete: {} imported, {} skipped (duplicate)",
                result.imported, result.skipped
            );
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
        Commands::Relocate { asset_id, volume } => {
            println!("Relocating asset {asset_id} to volume {volume}...");
            println!("not yet implemented");
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
            println!("not yet implemented");
            Ok(())
        }
        Commands::RebuildCatalog => {
            println!("Rebuilding catalog from sidecar files...");
            println!("not yet implemented");
            Ok(())
        }
    })();

    if let Err(e) = result {
        eprintln!("Error: {e:#}");
        std::process::exit(1);
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
