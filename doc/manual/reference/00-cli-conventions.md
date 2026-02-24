# CLI Conventions

This page documents the global flags, output behavior, exit codes, and scripting patterns that apply across all dam commands.


## Global Flags

Four flags are available on every command. They can appear before or after the subcommand name.

### `--json`

Switches output to machine-readable JSON on stdout. Human-readable status messages are sent to stderr so they do not interfere with JSON parsing.

```bash
dam search "rating:5" --json
dam import /path/to/files --json
dam stats --json
```

All data types (`SearchRow`, `AssetDetails`, `ImportResult`, `VerifyResult`, `SyncResult`, `CleanupResult`, `RelocateResult`, `DuplicateEntry`, etc.) implement `serde::Serialize` and produce well-formed JSON.

### `-l` / `--log`

Enables per-file progress logging to stderr. The format depends on the command:

- **Multi-file commands** (import, verify, sync, refresh, cleanup, generate-previews): each file prints a line in the format `filename -- status (duration)`.
- **`dam serve`**: each HTTP request prints `METHOD /path -> STATUS (duration)`.

```bash
dam import /Volumes/Photos/2026 --log
dam verify --volume "Archive" --log
dam serve --log
```

### `-d` / `--debug`

Shows stderr output from external tools (ffmpeg, dcraw, dcraw_emu). Useful for diagnosing preview generation issues. Prints both the command line and the tool's stderr via `eprintln`.

```bash
dam generate-previews --force --debug
dam import /path/to/raw-files --debug
```

### `-t` / `--time`

Prints total elapsed wall-clock time after command execution.

```bash
dam import /Volumes/Photos/2026 --time
# ...
# Elapsed: 12.34s
```

Flags can be combined freely:

```bash
dam import /path --json --log --time
```


## Catalog Discovery

dam locates the active catalog by searching for a `dam.toml` file:

1. Check the current working directory.
2. Walk up through parent directories until one containing `dam.toml` is found.
3. If the filesystem root is reached without finding one, exit with an error.

```bash
cd ~/Photos
dam stats            # works -- dam.toml is here

cd ~/Photos/metadata
dam stats            # works -- finds dam.toml in parent

cd /tmp
dam stats            # fails -- no dam.toml above /tmp
```

Error message when no catalog is found:

```
Error: No dam catalog found. Run `dam init` to create one.
```


## Asset ID Matching

Most commands that accept an asset ID (e.g. `show`, `edit`, `relocate`, `generate-previews --asset`) support **unique prefix matching**. You do not need to type the full UUID -- any unambiguous prefix is enough.

```bash
# Full UUID
dam show a1b2c3d4-e5f6-7890-abcd-ef1234567890

# Unique prefix (works if only one asset ID starts with "a1b2c")
dam show a1b2c

# Ambiguous prefix (multiple matches) -- dam reports an error
dam show a1
```

This applies to both command arguments and filter values where asset IDs are expected.


## Exit Codes

| Code | Meaning |
|------|---------|
| **0** | Success. Command completed without errors. |
| **1** | Failure. Examples: `verify` found hash mismatches, a referenced asset/volume was not found, a required argument was missing, or any other command error. |

Standard Rust/clap error handling applies for invalid arguments, missing subcommands, and unknown flags -- these also exit with a non-zero code and print usage help to stderr.


## Output Conventions

dam separates machine output from human messages:

| Stream | Content |
|--------|---------|
| **stdout** | Command results: search results, asset details, JSON output, format-template output |
| **stderr** | Progress messages, per-file logs (`--log`), debug output (`--debug`), timing (`--time`), warnings, and errors |

This separation means you can safely pipe or redirect stdout without capturing status noise:

```bash
dam search -q "tag:landscape" > asset-ids.txt
dam stats --json | jq '.total_size'
```

### Search result count

`dam search` prints a result count header by default (e.g. `Found 42 assets`). This header is suppressed when an explicit `--format` is given (including `-q`, which is shorthand for `--format=ids`), keeping output clean for scripting.


## Scripting Patterns

### Get just IDs for piping

```bash
dam search -q "tag:landscape"
```

`-q` is shorthand for `--format=ids` and prints one UUID per line with no header.

### JSON processing with jq

```bash
dam search "rating:5" --json | jq '.[].id'
dam show abc123 --json | jq '.variants[].filename'
dam stats --json | jq '{assets: .assets, size: .total_size}'
```

### Pipe search results into a collection

```bash
dam search -q "rating:5 tag:travel" | xargs dam col add "Travel Best"
```

### Batch operations via shell loop

```bash
for id in $(dam search -q "type:video"); do
  dam edit "$id" --label Blue
done
```

### Dry-run before applying

Several commands default to report-only mode and require `--apply` to make changes:

```bash
# Preview what would be imported
dam import --dry-run /path/to/files

# See what auto-group would merge (no --apply = dry-run)
dam auto-group "tag:landscape"
dam auto-group "tag:landscape" --apply    # actually merge

# See what sync would update (no --apply = report-only)
dam sync /Volumes/Photos
dam sync /Volumes/Photos --apply          # actually update

# See what cleanup would remove (no --apply = report-only)
dam cleanup
dam cleanup --apply                       # actually remove stale records
```

### Combining flags for verbose dry-runs

```bash
dam import --dry-run /path/to/files --log --time --json
dam sync /Volumes/Photos --log --json
```


## Safe Defaults

Commands that modify or delete data use conservative defaults:

| Command | Default behavior | Flag to commit changes |
|---------|-----------------|----------------------|
| `sync` | Report-only (no changes) | `--apply` |
| `cleanup` | Report-only (no changes) | `--apply` |
| `auto-group` | Report-only (no changes) | `--apply` |
| `fix-roles` | Report-only (no changes) | `--apply` |
| `fix-dates` | Report-only (no changes) | `--apply` |
| `import` | Imports immediately | `--dry-run` to preview |
| `relocate` | Copies immediately | `--dry-run` to preview |

The pattern is consistent: commands that scan and potentially alter many records default to showing you what they *would* do. You opt in to changes with `--apply`. Commands that operate on explicitly named files (import, relocate) run immediately but offer `--dry-run` for previewing.


## Command Categories

Quick reference of all dam commands, organized by workflow stage:

| Category | Commands | Purpose |
|----------|----------|---------|
| **Setup** | `init`, `volume add`, `volume list` | Create catalog, register storage volumes |
| **Ingest** | `import`, `tag`, `edit`, `group`, `auto-group` | Bring files in, apply metadata, merge variants |
| **Organize** | `collection` (`col`), `saved-search` (`ss`) | Curate static and smart albums |
| **Retrieve** | `search`, `show`, `duplicates`, `stats`, `serve` | Find assets, inspect details, browse in web UI |
| **Maintain** | `verify`, `sync`, `refresh`, `cleanup`, `relocate`, `update-location`, `generate-previews`, `fix-roles`, `fix-dates`, `rebuild-catalog` | Integrity checks, disk reconciliation, housekeeping |

Commands with aliases are shown with the alias in parentheses. For example, `dam col add` is equivalent to `dam collection add`, and `dam ss run` is equivalent to `dam saved-search run`.

---

Next: [Setup Commands](01-setup-commands.md) -- `init`, `volume add`, `volume list`.
