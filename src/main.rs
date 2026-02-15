use clap::{Parser, Subcommand};
use dam::catalog::Catalog;
use dam::config::CatalogConfig;
use dam::device_registry::DeviceRegistry;

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

    /// Add tags to an asset
    Tag {
        /// Asset ID
        asset_id: String,

        /// Tags to add
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
            println!("Importing {} path(s)...", paths.len());
            println!("not yet implemented");
            Ok(())
        }
        Commands::Search { query } => {
            println!("Searching for '{query}'...");
            println!("not yet implemented");
            Ok(())
        }
        Commands::Show { asset_id } => {
            println!("Showing asset {asset_id}...");
            println!("not yet implemented");
            Ok(())
        }
        Commands::Tag { asset_id, tags } => {
            println!("Tagging asset {asset_id} with {:?}...", tags);
            println!("not yet implemented");
            Ok(())
        }
        Commands::Group { variant_hashes } => {
            println!("Grouping {} variant(s)...", variant_hashes.len());
            println!("not yet implemented");
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
