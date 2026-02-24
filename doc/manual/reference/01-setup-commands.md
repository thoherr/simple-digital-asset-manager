# Setup Commands

Commands for initializing a catalog and registering storage volumes.

---

## dam init

### NAME

dam-init -- initialize a new catalog in the current directory

### SYNOPSIS

```
dam [GLOBAL FLAGS] init
```

### DESCRIPTION

Creates a new dam catalog rooted in the current working directory. This sets up the directory structure, configuration file, SQLite database, and volume registry needed to begin managing assets.

The following files and directories are created:

- `dam.toml` -- catalog configuration file (preview settings, serve settings, import exclusions)
- `metadata/` -- directory for YAML sidecar files (source of truth for asset metadata)
- `previews/` -- directory for generated preview thumbnails
- `catalog.db` -- SQLite database (derived cache for fast queries)
- `volumes.yaml` -- storage volume registry

If `dam.toml` already exists in the current directory, the command fails with an error to prevent accidental re-initialization.

### ARGUMENTS

None.

### OPTIONS

This command only accepts [global flags](00-cli-conventions.md#global-flags).

`--json` outputs `{"status": "initialized", "path": "<catalog_root>"}`.

### EXAMPLES

Initialize a catalog in a new directory:

```bash
mkdir ~/Photos && cd ~/Photos
dam init
```

Initialize and verify with JSON output:

```bash
cd /Volumes/Archive/PhotoLibrary
dam init --json
# {"status": "initialized", "path": "/Volumes/Archive/PhotoLibrary"}
```

Attempt to re-initialize (fails safely):

```bash
cd ~/Photos
dam init
# Error: A dam catalog already exists in this directory.
```

### SEE ALSO

[volume add](#dam-volume-add) -- register a storage volume after initialization.
[CLI Conventions](00-cli-conventions.md) -- catalog discovery, global flags, exit codes.

---

## dam volume add

### NAME

dam-volume-add -- register a new storage volume with the catalog

### SYNOPSIS

```
dam [GLOBAL FLAGS] volume add <LABEL> <PATH> [--purpose <PURPOSE>]
```

### DESCRIPTION

Registers a storage volume (a directory tree containing media files) with the catalog. Each volume is assigned a UUID and tracked by its label and mount point path. Volumes allow dam to manage files spread across multiple disks, external drives, and network mounts.

The label is a human-readable name for the volume (e.g., "Photos2026", "Archive", "ExternalSSD"). The path is the mount point or root directory of the volume. The path must exist at the time of registration.

An optional `--purpose` flag assigns a logical role to the volume in the storage hierarchy. This metadata is used by duplicate analysis and backup coverage commands to distinguish between unwanted duplicates (same file on the same working volume) and wanted backups (same file on an archive or backup volume). The purpose can be changed later with `dam volume set-purpose`.

After registration, files under the volume's path can be imported and tracked. If the volume becomes unavailable (e.g., an external drive is disconnected), it is reported as "offline" in `dam volume list`, and commands that need to access its files will skip it gracefully.

### ARGUMENTS

**LABEL** (required)
: Human-readable name for the volume. Used in `--volume` flags across commands.

**PATH** (required)
: Absolute path to the volume's mount point or root directory.

### OPTIONS

`--purpose <PURPOSE>`
: Logical role of the volume. Valid values: `working` (active editing drive), `archive` (long-term primary storage), `backup` (redundancy copy), `cloud` (cloud sync folder). Optional — volumes without a purpose are treated as unclassified.

`--json` outputs `{"id": "<uuid>", "label": "<label>", "path": "<path>", "purpose": "<purpose>"}`.

### EXAMPLES

Register an external drive:

```bash
dam volume add "Photos" /Volumes/PhotoDrive
# Registered volume 'Photos' (a1b2c3d4-e5f6-7890-abcd-ef1234567890)
#   Path: /Volumes/PhotoDrive
```

Register a volume with a purpose:

```bash
dam volume add "Master Media" /Volumes/MediaDrive --purpose archive
# Registered volume 'Master Media' (e5f67890-...)
#   Path: /Volumes/MediaDrive
#   Purpose: archive
```

Register multiple volumes for a multi-disk workflow:

```bash
dam volume add "Laptop" /Volumes/MacintoshHD --purpose working
dam volume add "Master Media" /Volumes/MediaDrive --purpose archive
dam volume add "Backup A" /Volumes/BackupDisk --purpose backup
dam volume add "Dropbox" ~/Dropbox/Photos --purpose cloud
```

### SEE ALSO

[volume list](#dam-volume-list) -- list registered volumes and their status.
[import](02-ingest-commands.md#dam-import) -- import files from a volume.
[relocate](05-maintain-commands.md#dam-relocate) -- copy or move asset files between volumes.

---

## dam volume list

### NAME

dam-volume-list -- list all registered volumes and their online/offline status

### SYNOPSIS

```
dam [GLOBAL FLAGS] volume list
```

### DESCRIPTION

Displays all storage volumes registered with the catalog, along with their UUIDs, labels, mount point paths, and current status.

A volume is reported as **online** if its mount point path exists on disk, and **offline** if the path is not accessible (e.g., the drive is disconnected or the network share is unmounted). Offline volumes are silently skipped by commands that access files on disk (import, verify, sync, cleanup, etc.).

### ARGUMENTS

None.

### OPTIONS

This command only accepts [global flags](00-cli-conventions.md#global-flags).

`--json` outputs an array of `{"id", "label", "path", "volume_type", "purpose", "is_online"}` objects.

### EXAMPLES

List all volumes:

```bash
dam volume list
# Laptop (a1b2c3d4-...) [online] [working]
#   Path: /Volumes/MacintoshHD
# Master Media (e5f67890-...) [online] [archive]
#   Path: /Volumes/MediaDrive
# Backup A (f1234567-...) [offline] [backup]
#   Path: /Volumes/BackupDisk
```

List volumes as JSON for scripting:

```bash
dam volume list --json | jq '.[] | select(.is_online) | .label'
```

Check if a specific volume is online:

```bash
dam volume list --json | jq '.[] | select(.label == "Archive") | .is_online'
```

### SEE ALSO

[volume add](#dam-volume-add) -- register a new volume.
[volume set-purpose](#dam-volume-set-purpose) -- change a volume's purpose.
[stats](04-retrieve-commands.md#dam-stats) -- `--volumes` flag shows per-volume asset counts and sizes.
[CLI Conventions](00-cli-conventions.md) -- catalog discovery rules.

---

## dam volume set-purpose

### NAME

dam-volume-set-purpose -- set or clear the logical purpose of a volume

### SYNOPSIS

```
dam [GLOBAL FLAGS] volume set-purpose <VOLUME> <PURPOSE>
```

### DESCRIPTION

Changes the purpose of an existing volume. The purpose describes the volume's role in the storage hierarchy and is used by duplicate analysis and backup coverage commands to distinguish between working copies, archives, and backups.

### ARGUMENTS

**VOLUME** (required)
: Volume label or UUID.

**PURPOSE** (required)
: One of `working`, `archive`, `backup`, `cloud`, or `none` (to clear the purpose).

### OPTIONS

This command only accepts [global flags](00-cli-conventions.md#global-flags).

`--json` outputs `{"id": "<uuid>", "label": "<label>", "purpose": "<purpose>"}`.

### EXAMPLES

Set a volume's purpose:

```bash
dam volume set-purpose "Photos" archive
# Volume 'Photos' purpose set to: archive
```

Clear a volume's purpose:

```bash
dam volume set-purpose "Photos" none
# Volume 'Photos' purpose cleared.
```

### SEE ALSO

[volume add](#dam-volume-add) -- register a new volume (with optional `--purpose`).
[volume list](#dam-volume-list) -- list volumes and their purposes.

---

Next: [Ingest Commands](02-ingest-commands.md) -- `import`, `tag`, `edit`, `group`, `auto-group`.
