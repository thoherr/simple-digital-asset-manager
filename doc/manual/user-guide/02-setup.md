# Setup

This chapter covers building MAKI from source, initializing a catalog, registering storage volumes, and configuring the system to match your workflow.


## Installation

### Pre-built binaries

Download a pre-built binary from the [GitHub releases page](https://github.com/thoherr/maki/releases). Each release provides two editions for every platform:

| Archive name | Edition |
|-------------|---------|
| `maki-VERSION-PLATFORM.tar.gz` | MAKI (standard) |
| `maki-VERSION-PLATFORM-pro.tar.gz` | MAKI Pro (with AI features) |

Extract the archive and copy the `maki` binary to a directory on your `PATH`:

```bash
tar xzf maki-4.2.1-macos-arm64-pro.tar.gz
cp maki /usr/local/bin/
```

Verify the installation:

```bash
maki --version
# MAKI:     maki 4.2.1
# MAKI Pro: maki 4.2.1 Pro
```

### Building from source

MAKI is written in Rust. You need a working Rust toolchain (rustc + cargo). Install one via [rustup](https://rustup.rs/) if you have not already.

```bash
git clone https://github.com/thoherr/maki.git
cd maki
cargo build --release                    # standard edition
cargo build --release --features pro     # Pro edition
cargo build --release --features pro,ai-gpu  # Pro with GPU acceleration (macOS only)
```

The binary is at `target/release/maki`. Copy or symlink it to a directory on your `PATH`.

### Supported platforms

MAKI builds and runs on **macOS**, **Linux**, and **Windows**. Both x86_64 and ARM (Apple Silicon) are supported.

### External tools (highly recommended)

MAKI handles standard image formats (JPEG, PNG, TIFF, WebP) natively. The following external tools extend its capabilities:

| Tool | Purpose | Install |
|------|---------|---------|
| **dcraw** or **dcraw_emu** (LibRaw) | RAW file previews (NEF, ARW, CR2, CR3, etc.) | `brew install dcraw` or `brew install libraw` on macOS; your package manager on Linux; `winget install LibRaw.LibRaw` or `scoop install libraw` on Windows |
| **ffmpeg** / **ffprobe** | Video thumbnail extraction and video metadata (duration, codec, resolution, framerate). `ffprobe` is included with the ffmpeg package. | `brew install ffmpeg` on macOS; your package manager on Linux; `winget install Gyan.FFmpeg` or `scoop install ffmpeg` on Windows |
| **curl** | AI model download and VLM image descriptions | Pre-installed on macOS and most Linux distributions; `winget install cURL.cURL` or `scoop install curl` on Windows |

When an external tool is missing, MAKI prints a warning on first use explaining what is needed and why. It still imports RAW and video files, but generates an info card (a placeholder JPEG showing file metadata) instead of a rendered preview. You can install the tools later and run `maki generate-previews --force` to regenerate real previews.


## Initializing a Catalog

A catalog is a directory that holds MAKI's database, configuration, metadata sidecars, and preview images. Create one by navigating to the directory you want as the catalog root and running `maki init`:

```bash
mkdir ~/Photos
cd ~/Photos
maki init
```

Output:

```
Initialized new maki catalog in /Users/you/Photos
```

This creates the following structure:

```
~/Photos/
  maki.toml          # Configuration file
  catalog.db        # SQLite database (cache/index)
  volumes.yaml      # Registered storage volumes
  metadata/         # YAML sidecar files (source of truth)
  previews/         # Preview thumbnails (800px)
  smart_previews/   # High-resolution previews for zoom/pan (2560px)
```

### How catalog detection works

After initialization, you can run `maki` commands from the catalog root or any subdirectory. MAKI locates the catalog by walking up from your current working directory, looking for a `maki.toml` file. This means you can organize files in subdirectories and still run commands without specifying the catalog path.

```bash
cd ~/Photos
maki stats            # works -- maki.toml is here

cd ~/Photos/metadata
maki stats            # also works -- finds maki.toml in parent

cd /tmp
maki stats            # fails -- no maki.toml above /tmp
```

If no catalog is found, MAKI prints:

```
Error: No maki catalog found. Run `maki init` to create one.
```

### Reinitializing

`maki init` refuses to overwrite an existing catalog. If you need to start fresh, delete the catalog files first (`maki.toml`, `catalog.db`, `metadata/`, `previews/`, `smart_previews/`, `volumes.yaml`) and run `maki init` again.


## Registering Volumes

A **volume** represents a storage location -- a local directory, an external drive, or a NAS mount point. Before importing files, you must register the volume they live on.

### Adding a volume

```bash
maki volume add "Photos 2024" /Volumes/PhotosDrive
```

Output:

```
Registered volume 'Photos 2024' (a1b2c3d4-e5f6-7890-abcd-ef1234567890)
  Path: /Volumes/PhotosDrive
```

Each volume gets a UUID that stays constant even if the drive letter or mount point changes. The label is a human-readable name you choose.

If you omit the label, it is auto-derived from the last component of the path. This is handy for transient volumes like memory cards:

```bash
# Auto-label: derives "EOS_DIGITAL" from the path
maki volume add /Volumes/EOS_DIGITAL --purpose media
```

A few more examples:

```bash
# External SSD
maki volume add "Travel SSD" /Volumes/TravelPhotos

# Network-attached storage
maki volume add "NAS Archive" /Volumes/nas/photos

# Local directory
maki volume add "Local Work" /Users/you/Photography
```

### Listing volumes

```bash
maki volume list
```

Output:

```
Photos 2024 (a1b2c3d4-e5f6-7890-abcd-ef1234567890) [online]
  Path: /Volumes/PhotosDrive
Travel SSD (b2c3d4e5-f6a7-8901-bcde-f12345678901) [offline]
  Path: /Volumes/TravelPhotos
```

### Online vs. offline

MAKI checks whether each volume's mount point directory exists on disk:

- **Online**: The directory exists. maki can read files, generate previews, and verify integrity.
- **Offline**: The directory does not exist (drive disconnected, NAS unreachable). maki still knows about the volume and its assets, but file operations are skipped gracefully.

This design lets you manage a photo library that spans multiple external drives. You can search, browse, and view cached previews for assets on offline volumes. When you reconnect a drive, MAKI picks it up automatically.

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
| `media`     | Transient source media — memory cards, card readers, camera USB |
| `working`   | Active editing drive — fast SSD with current projects |
| `archive`   | Long-term primary storage — the "master" copy |
| `backup`    | Redundancy copy — exists purely for safety |
| `cloud`     | Cloud-synced folder (Dropbox, iCloud, Google Drive) |

```bash
maki volume add "Card"       /Volumes/CARD         --purpose media
maki volume add "Laptop SSD" /Volumes/MacintoshHD --purpose working
maki volume add "Archive"    /Volumes/MediaDrive   --purpose archive
maki volume add "Backup A"   /Volumes/BackupDisk   --purpose backup
maki volume add "Dropbox"    ~/Dropbox/Photos      --purpose cloud
```

Purpose metadata drives two features:

- **Duplicate analysis** (`maki duplicates`): Distinguishes unwanted duplicates (same file twice on the same working drive) from wanted redundancy (same file on an archive and a backup).
- **Backup coverage** (`maki backup-status`): Reports which assets lack copies on archive or backup volumes and flags at-risk assets. Media volumes are excluded from coverage calculations — a file only on a memory card is not considered backed up.

You can set or change a purpose at any time:

```bash
maki volume set-purpose "Laptop SSD" archive
maki volume set-purpose "Laptop SSD" none      # clear
```

Volumes without a purpose are treated as unclassified — they still work for import, search, and all other operations, but are excluded from purpose-based analysis.

### Symlinks and path resolution

MAKI resolves symlinks when registering volumes and importing files. All paths stored in the catalog are **physical (canonical) paths**, not the symlink paths you may see in your filesystem.

This matters when your directory layout uses symlinks to span multiple disks. For example, if you have:

```
/Volumes/Pictures/masters/2025/   (real directory)
/Volumes/Pictures/masters/2026 → /Volumes/Dropbox/masters/2026/  (symlink)
```

and you register `/Volumes/Pictures` as a volume, then:

- Files under `.../masters/2025/` are tracked on the "Pictures" volume as expected.
- Files under `.../masters/2026/` resolve through the symlink to `/Volumes/Dropbox/masters/2026/`, which is **outside** the Pictures volume mount point. Import will fail with "No registered volume contains path" unless the Dropbox path is also registered as a volume.

**Why MAKI resolves symlinks:**

- **Reliable verification.** `maki verify` re-hashes files by their catalog path. If the catalog stored a symlink path and the symlink later changed target, verification would silently check the wrong file — or fail when the link breaks.
- **Correct offline detection.** A volume is "online" when its mount point exists. A broken symlink inside an online volume would cause confusing partial failures.
- **Predictable sync behavior.** `maki sync` detects moved and missing files by comparing disk state to catalog paths. Symlink changes would cause false "moved" or "missing" reports for every file under the changed link.
- **Unambiguous volume mapping.** Each file belongs to exactly one volume (determined by longest mount-point prefix match on the physical path). Symlinks could cause the same file to appear to belong to two different volumes depending on which path you use.

**The recommended workaround** is to register each physical storage location as its own volume:

```bash
maki volume add "Pictures"  /Volumes/Pictures  --purpose archive
maki volume add "Dropbox"   /Volumes/Dropbox   --purpose cloud
```

Both volumes work independently. Import auto-detects the correct volume based on the resolved physical path. `backup-status` shows correct copy counts across both. You can still navigate your symlinked directory structure normally — maki simply tracks where files physically reside.

### Removing a volume

If a volume is no longer needed (drive decommissioned, cloud service cancelled), you can cleanly remove it:

```bash
maki volume remove "Old Drive"          # report what would be removed
maki volume remove "Old Drive" --apply  # actually remove
```

This removes all file location and recipe records on that volume from the catalog and sidecar files, deletes any assets that become orphaned (no remaining file locations), and cleans up orphaned preview files. Without `--apply`, it runs in report-only mode so you can review the impact first.

See the [volume remove reference](../reference/01-setup-commands.md#maki-volume-remove) for details.

### Combining volumes

If you initially registered year-based subdirectories as separate volumes (e.g., `media_2024`, `media_2025` under `/Volumes/Media`) and now want to consolidate them into a single disk-level volume, use `volume combine`:

```bash
# First register the parent directory as a volume
maki volume add "Media" /Volumes/Media

# Preview what combining would do
maki volume combine "media_2024" "Media"
# Would combine 'media_2024' into 'Media': 4832 locations, 312 recipes (3210 assets, prefix 'media_2024/')

# Execute the combination
maki volume combine "media_2024" "Media" --apply
```

The source volume's mount point must be a subdirectory of the target's. All file paths are automatically rewritten with the appropriate prefix (e.g., `photo.jpg` becomes `media_2024/photo.jpg`). After combining, the source volume is removed. You can repeat this for each year-volume to consolidate into one.

See the [volume combine reference](../reference/01-setup-commands.md#maki-volume-combine) for details.

### Splitting a volume

The inverse of combine: split a subdirectory off into its own volume. This is useful when you physically move a folder to a new drive:

```bash
# Preview what splitting would do
maki volume split "Photos" "Archive 2024" --path "Archive/2024"

# Execute the split
maki volume split "Photos" "Archive 2024" --path "Archive/2024" --apply
```

All file locations under the specified path are reassigned to the new volume, with path prefixes rewritten accordingly. You can optionally assign a purpose to the new volume:

```bash
maki volume split "Photos" "Archive 2024" --path "Archive/2024" --purpose archive --apply
```

See the [volume split reference](../reference/01-setup-commands.md#maki-volume-split) for details.

### Renaming a volume

If a drive label changes or you want a clearer name:

```bash
maki volume rename "Old Label" "New Label"
```

This updates the volume label everywhere (catalog, sidecar YAML files, volume registry). No files are moved or modified on disk.


## Configuration (maki.toml)

The `maki.toml` file at the catalog root controls MAKI's behavior. All sections are optional -- an empty file (or one with only comments) uses sensible defaults.

Here is an example showing common options (see the [Configuration Reference](../reference/08-configuration.md) for the full list):

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

**`default_volume`** -- UUID of the volume to use when `maki import` cannot auto-detect the correct volume from the file path. Useful if you always import from the same drive.

**`[preview]`** -- Controls preview generation. `max_edge` sets the longest side of generated thumbnails. `format` chooses between JPEG (smaller, lossy) and WebP (lossless via the `image` crate). `quality` only applies to JPEG output.

**`[serve]`** -- Web UI server settings. Change `bind` to `"0.0.0.0"` to allow access from other machines on your network. CLI flags `--port` and `--bind` override these values.

**`[import]`** -- `exclude` patterns are matched against filenames (not full paths) using glob syntax. Common choices: OS junk files, editor temp files, thumbnail caches. `auto_tags` are merged with any tags extracted from XMP metadata during import; useful for tagging everything in a session as "unreviewed" for later triage.

### Recommended settings

If you use Lightroom, CaptureOne, or another tool that reads `.xmp` sidecar files, enable XMP writeback so your edits in MAKI are visible to those tools:

```toml
[writeback]
enabled = true
```

> **Warning:** Enabling writeback causes MAKI to modify `.xmp` files on your storage volumes whenever you change ratings, tags, labels, or descriptions. If you prefer MAKI to keep its edits strictly in its own catalog (YAML sidecars + SQLite), leave writeback disabled (the default). You can always enable it later and run `maki writeback --all` to push all edits at once.

For a complete reference of every option and its behavior, see the [Configuration Reference](../reference/08-configuration.md).

---

Next: [Ingesting Assets](03-ingest.md) -- importing files into the catalog, auto-grouping, and metadata extraction.
