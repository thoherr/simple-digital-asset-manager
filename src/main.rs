use clap::{Parser, Subcommand};

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

    let result: Result<(), Box<dyn std::error::Error>> = match cli.command {
        Commands::Init => {
            println!("Initializing new dam catalog...");
            println!("not yet implemented");
            Ok(())
        }
        Commands::Volume(cmd) => match cmd {
            VolumeCommands::Add { label, path } => {
                println!("Adding volume '{label}' at {path}...");
                println!("not yet implemented");
                Ok(())
            }
            VolumeCommands::List => {
                println!("not yet implemented");
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
    };

    if let Err(e) = result {
        eprintln!("Error: {e:#}");
        std::process::exit(1);
    }
}
