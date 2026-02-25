# Setup

This chapter covers building dam from source, initializing a catalog, registering storage volumes, and configuring the system to match your workflow.


## Installation

### Building from source

dam is written in Rust. You need a working Rust toolchain (rustc + cargo). Install one via [rustup](https://rustup.rs/) if you have not already.

Clone the repository and build a release binary:

```bash
git clone https://github.com/your-org/dam.git
cd dam
cargo build --release
```

The binary is at `target/release/dam`. Copy or symlink it to a directory on your `PATH`:

```bash
cp target/release/dam /usr/local/bin/
```

Verify the installation:

```bash
dam --version
```

### Supported platforms

dam builds and runs on **macOS** and **Linux**. Both x86_64 and ARM (Apple Silicon) are supported.

### Optional external tools

dam handles standard image formats (JPEG, PNG, TIFF, WebP) natively. Two external tools extend its preview capabilities:

| Tool | Purpose | Install |
|------|---------|---------|
| **dcraw** or **dcraw_emu** (LibRaw) | RAW file previews (NEF, ARW, CR2, CR3, etc.) | `brew install dcraw` or `brew install libraw` on macOS; your package manager on Linux |
| **ffmpeg** | Video thumbnail extraction | `brew install ffmpeg` on macOS; your package manager on Linux |

If these tools are not installed, dam still imports RAW and video files. It generates an info card (a placeholder JPEG showing file metadata) instead of a rendered preview. You can install the tools later and run `dam generate-previews --force` to regenerate real previews.


## Initializing a Catalog

A catalog is a directory that holds dam's database, configuration, metadata sidecars, and preview images. Create one by navigating to the directory you want as the catalog root and running `dam init`:

```bash
mkdir ~/Photos
cd ~/Photos
dam init
```

Output:

```
Initialized new dam catalog in /Users/you/Photos
```

This creates the following structure:

```
~/Photos/
  dam.toml          # Configuration file
  catalog.db        # SQLite database (cache/index)
  volumes.yaml      # Registered storage volumes
  metadata/         # YAML sidecar files (source of truth)
  previews/         # Generated preview images
```

![Catalog directory structure after dam init](../screenshots/catalog-structure.png)

### How catalog detection works

After initialization, you can run dam commands from the catalog root or any subdirectory. dam locates the catalog by walking up from your current working directory, looking for a `dam.toml` file. This means you can organize files in subdirectories and still run commands without specifying the catalog path.

```bash
cd ~/Photos
dam stats            # works -- dam.toml is here

cd ~/Photos/metadata
dam stats            # also works -- finds dam.toml in parent

cd /tmp
dam stats            # fails -- no dam.toml above /tmp
```

If no catalog is found, dam prints:

```
Error: No dam catalog found. Run `dam init` to create one.
```

### Reinitializing

`dam init` refuses to overwrite an existing catalog. If you need to start fresh, delete the catalog files first (`dam.toml`, `catalog.db`, `metadata/`, `previews/`, `volumes.yaml`) and run `dam init` again.


## Registering Volumes

A **volume** represents a storage location -- a local directory, an external drive, or a NAS mount point. Before importing files, you must register the volume they live on.

### Adding a volume

```bash
dam volume add "Photos 2024" /Volumes/PhotosDrive
```

Output:

```
Registered volume 'Photos 2024' (a1b2c3d4-e5f6-7890-abcd-ef1234567890)
  Path: /Volumes/PhotosDrive
```

Each volume gets a UUID that stays constant even if the drive letter or mount point changes. The label is a human-readable name you choose.

A few more examples:

```bash
# External SSD
dam volume add "Travel SSD" /Volumes/TravelPhotos

# Network-attached storage
dam volume add "NAS Archive" /Volumes/nas/photos

# Local directory
dam volume add "Local Work" /Users/you/Photography
```

### Listing volumes

```bash
dam volume list
```

Output:

```
Photos 2024 (a1b2c3d4-e5f6-7890-abcd-ef1234567890) [online]
  Path: /Volumes/PhotosDrive
Travel SSD (b2c3d4e5-f6a7-8901-bcde-f12345678901) [offline]
  Path: /Volumes/TravelPhotos
```

### Online vs. offline

dam checks whether each volume's mount point directory exists on disk:

- **Online**: The directory exists. dam can read files, generate previews, and verify integrity.
- **Offline**: The directory does not exist (drive disconnected, NAS unreachable). dam still knows about the volume and its assets, but file operations are skipped gracefully.

This design lets you manage a photo library that spans multiple external drives. You can search, browse, and view cached previews for assets on offline volumes. When you reconnect a drive, dam picks it up automatically.

### Multiple volumes

There is no limit on the number of volumes. A typical setup might look like:

- A fast local SSD for current work
- One or more external drives for archive storage
- A NAS for backups

Assets can have files on multiple volumes simultaneously (see [Maintenance](07-maintenance.md) for the `relocate` command).

### Volume purposes

Each volume can optionally be assigned a **purpose** that describes its role in your storage hierarchy:

| Purpose     | Meaning |
|-------------|---------|
| `working`   | Active editing drive — fast SSD with current projects |
| `archive`   | Long-term primary storage — the "master" copy |
| `backup`    | Redundancy copy — exists purely for safety |
| `cloud`     | Cloud-synced folder (Dropbox, iCloud, Google Drive) |

```bash
dam volume add "Laptop SSD" /Volumes/MacintoshHD --purpose working
dam volume add "Archive"    /Volumes/MediaDrive   --purpose archive
dam volume add "Backup A"   /Volumes/BackupDisk   --purpose backup
dam volume add "Dropbox"    ~/Dropbox/Photos      --purpose cloud
```

Purpose metadata drives two features:

- **Duplicate analysis** (`dam duplicates`): Distinguishes unwanted duplicates (same file twice on the same working drive) from wanted redundancy (same file on an archive and a backup).
- **Backup coverage** (`dam backup-status`): Reports which assets lack copies on archive or backup volumes and flags at-risk assets.

You can set or change a purpose at any time:

```bash
dam volume set-purpose "Laptop SSD" archive
dam volume set-purpose "Laptop SSD" none      # clear
```

Volumes without a purpose are treated as unclassified — they still work for import, search, and all other operations, but are excluded from purpose-based analysis.

### Symlinks and path resolution

dam resolves symlinks when registering volumes and importing files. All paths stored in the catalog are **physical (canonical) paths**, not the symlink paths you may see in your filesystem.

This matters when your directory layout uses symlinks to span multiple disks. For example, if you have:

```
/Volumes/Pictures/masters/2025/   (real directory)
/Volumes/Pictures/masters/2026 → /Volumes/Dropbox/masters/2026/  (symlink)
```

and you register `/Volumes/Pictures` as a volume, then:

- Files under `.../masters/2025/` are tracked on the "Pictures" volume as expected.
- Files under `.../masters/2026/` resolve through the symlink to `/Volumes/Dropbox/masters/2026/`, which is **outside** the Pictures volume mount point. Import will fail with "No registered volume contains path" unless the Dropbox path is also registered as a volume.

**Why dam resolves symlinks:**

- **Reliable verification.** `dam verify` re-hashes files by their catalog path. If the catalog stored a symlink path and the symlink later changed target, verification would silently check the wrong file — or fail when the link breaks.
- **Correct offline detection.** A volume is "online" when its mount point exists. A broken symlink inside an online volume would cause confusing partial failures.
- **Predictable sync behavior.** `dam sync` detects moved and missing files by comparing disk state to catalog paths. Symlink changes would cause false "moved" or "missing" reports for every file under the changed link.
- **Unambiguous volume mapping.** Each file belongs to exactly one volume (determined by longest mount-point prefix match on the physical path). Symlinks could cause the same file to appear to belong to two different volumes depending on which path you use.

**The recommended workaround** is to register each physical storage location as its own volume:

```bash
dam volume add "Pictures"  /Volumes/Pictures  --purpose archive
dam volume add "Dropbox"   /Volumes/Dropbox   --purpose cloud
```

Both volumes work independently. Import auto-detects the correct volume based on the resolved physical path. `backup-status` shows correct copy counts across both. You can still navigate your symlinked directory structure normally — dam simply tracks where files physically reside.

### Removing a volume

If a volume is no longer needed (drive decommissioned, cloud service cancelled), you can cleanly remove it:

```bash
dam volume remove "Old Drive"          # report what would be removed
dam volume remove "Old Drive" --apply  # actually remove
```

This removes all file location and recipe records on that volume from the catalog and sidecar files, deletes any assets that become orphaned (no remaining file locations), and cleans up orphaned preview files. Without `--apply`, it runs in report-only mode so you can review the impact first.

See the [volume remove reference](../reference/01-setup-commands.md#dam-volume-remove) for details.


## Configuration (dam.toml)

The `dam.toml` file at the catalog root controls dam's behavior. All sections are optional -- an empty file (or one with only comments) uses sensible defaults.

Here is a complete example showing every available option:

```toml
# Default volume for import when auto-detection is ambiguous
default_volume = "a1b2c3d4-e5f6-7890-abcd-ef1234567890"

[preview]
max_edge = 800        # Maximum width/height in pixels (default: 800)
format = "jpeg"       # "jpeg" or "webp" (default: "jpeg")
quality = 85          # JPEG quality 1-100 (default: 85; ignored for webp)

[serve]
port = 8080           # Web UI port (default: 8080)
bind = "127.0.0.1"    # Bind address (default: "127.0.0.1")

[import]
exclude = [           # Glob patterns to skip during import
    ".DS_Store",
    "Thumbs.db",
    "*.tmp",
]
auto_tags = [         # Tags automatically applied to every new asset
    "inbox",
    "unreviewed",
]
```

### Section summary

**`default_volume`** -- UUID of the volume to use when `dam import` cannot auto-detect the correct volume from the file path. Useful if you always import from the same drive.

**`[preview]`** -- Controls preview generation. `max_edge` sets the longest side of generated thumbnails. `format` chooses between JPEG (smaller, lossy) and WebP (lossless via the `image` crate). `quality` only applies to JPEG output.

**`[serve]`** -- Web UI server settings. Change `bind` to `"0.0.0.0"` to allow access from other machines on your network. CLI flags `--port` and `--bind` override these values.

**`[import]`** -- `exclude` patterns are matched against filenames (not full paths) using glob syntax. Common choices: OS junk files, editor temp files, thumbnail caches. `auto_tags` are merged with any tags extracted from XMP metadata during import; useful for tagging everything in a session as "unreviewed" for later triage.

For a complete reference of every option and its behavior, see the [Configuration Reference](../reference/08-configuration.md).

---

Next: [Ingesting Assets](03-ingest.md) -- importing files into the catalog, auto-grouping, and metadata extraction.
