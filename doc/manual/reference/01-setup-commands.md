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

## dam volume remove

### NAME

dam-volume-remove -- remove a volume and all its associated catalog data

### SYNOPSIS

```
dam [GLOBAL FLAGS] volume remove <VOLUME> [--apply]
```

### DESCRIPTION

Removes a volume and all data associated with it: file location records, recipe records, assets that become orphaned (no remaining file locations on any volume), and preview files for those orphaned assets. Also removes the volume from `volumes.yaml` and the SQLite catalog.

By default, runs in **report-only mode** -- shows what would be removed without making changes. Use `--apply` to execute the removal. This is consistent with `cleanup`, `sync`, and `dedup`.

The removal proceeds in phases:

1. **Locations**: Removes all file location records on the volume from the catalog and sidecar YAML files.
2. **Recipes**: Removes all recipe records on the volume from the catalog and sidecar YAML files.
3. **Orphaned assets**: Detects assets where all variants now have zero file locations. In apply mode, deletes these assets along with their variants, remaining recipes, catalog rows, and sidecar YAML files.
4. **Orphaned previews**: Detects preview files whose content hash no longer matches any variant in the catalog. In apply mode, deletes these files.
5. **Volume**: Removes the volume entry from `volumes.yaml` and the SQLite `volumes` table.

In report-only mode, orphaned assets and previews are predicted (what *would* become orphaned if the locations were removed).

### ARGUMENTS

**VOLUME** (required)
: Volume label or UUID.

### OPTIONS

`--apply`
: Execute the removal. Without this flag, only a report is printed.

This command also accepts [global flags](00-cli-conventions.md#global-flags). `--json` outputs a `VolumeRemoveResult` object with fields: `volume_label`, `volume_id`, `locations`, `locations_removed`, `recipes`, `recipes_removed`, `orphaned_assets`, `removed_assets`, `orphaned_previews`, `removed_previews`, `apply`, `errors`. `--log` prints per-file progress to stderr.

### EXAMPLES

Preview what removing a volume would do:

```bash
dam volume remove "Old Drive"
# Volume 'Old Drive' would remove: 1523 locations, 87 recipes, 412 orphaned assets, 412 orphaned previews
#   Run with --apply to remove.
```

Remove the volume:

```bash
dam volume remove "Old Drive" --apply
# Volume 'Old Drive' removed: 1523 locations removed, 87 recipes removed, 412 orphaned assets removed, 412 orphaned previews removed
```

JSON output for scripting:

```bash
dam volume remove "Old Drive" --json
```

### SEE ALSO

[volume list](#dam-volume-list) -- list volumes and their status.
[cleanup](05-maintain-commands.md#dam-cleanup) -- remove stale records for missing files (works across volumes).

---

Next: [Ingest Commands](02-ingest-commands.md) -- `import`, `tag`, `edit`, `group`, `auto-group`.
