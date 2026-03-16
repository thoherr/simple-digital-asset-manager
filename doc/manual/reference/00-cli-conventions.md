# CLI Conventions

This page documents the global flags, output behavior, exit codes, and scripting patterns that apply across all maki commands.


## Global Flags

Five flags are available on every command. They can appear before or after the subcommand name.

### `--json`

Switches output to machine-readable JSON on stdout. Human-readable status messages are sent to stderr so they do not interfere with JSON parsing.

```bash
maki search "rating:5" --json
maki import /path/to/files --json
maki stats --json
```

All data types (`SearchRow`, `AssetDetails`, `ImportResult`, `VerifyResult`, `SyncResult`, `CleanupResult`, `RelocateResult`, `DuplicateEntry`, etc.) implement `serde::Serialize` and produce well-formed JSON.

### `-l` / `--log`

Enables per-file progress logging to stderr. The format depends on the command:

- **Multi-file commands** (import, verify, sync, refresh, cleanup, generate-previews): each file prints a line in the format `filename -- status (duration)`.
- **`maki serve`**: each HTTP request prints `METHOD /path -> STATUS (duration)`.

```bash
maki import /Volumes/Photos/2026 --log
maki verify --volume "Archive" --log
maki serve --log
```

### `-v` / `--verbose`

Shows operational decisions and program flow to stderr. Useful for understanding what a command is doing without the low-level detail of `--debug`. Examples of verbose output:

- **import**: number of resolved files, detected volume, exclude patterns, auto-tags
- **describe**: VLM endpoint, model, mode, concurrency, candidate count
- **search**: parsed query, result count
- **preview generation**: source format, generation method (image/RAW/video/info card), max edge size

`--debug` implies `--verbose` — you never need to pass both.

```bash
maki import /Volumes/Photos/2026 --verbose
maki describe "rating:5" --verbose
maki search "tag:landscape" --verbose
```

### `-d` / `--debug`

Shows stderr output from external tools (ffmpeg, dcraw, dcraw_emu) and implies `--verbose`. Useful for diagnosing preview generation issues. Prints both the command line and the tool's stderr via `eprintln`.

```bash
maki generate-previews --force --debug
maki import /path/to/raw-files --debug
```

### `-t` / `--time`

Prints total elapsed wall-clock time after command execution.

```bash
maki import /Volumes/Photos/2026 --time
# ...
# Elapsed: 12.34s
```

Flags can be combined freely:

```bash
maki import /path --json --log --time
```


## Catalog Discovery

maki locates the active catalog by searching for a `maki.toml` file:

1. Check the current working directory.
2. Walk up through parent directories until one containing `maki.toml` is found.
3. If the filesystem root is reached without finding one, exit with an error.

```bash
cd ~/Photos
maki stats            # works -- maki.toml is here

cd ~/Photos/metadata
maki stats            # works -- finds maki.toml in parent

cd /tmp
maki stats            # fails -- no maki.toml above /tmp
```

Error message when no catalog is found:

```
Error: No maki catalog found. Run `maki init` to create one.
```


## Asset ID Matching

Most commands that accept an asset ID (e.g. `show`, `edit`, `relocate`, `generate-previews --asset`) support **unique prefix matching**. You do not need to type the full UUID -- any unambiguous prefix is enough.

```bash
# Full UUID
maki show a1b2c3d4-e5f6-7890-abcd-ef1234567890

# Unique prefix (works if only one asset ID starts with "a1b2c")
maki show a1b2c

# Ambiguous prefix (multiple matches) -- maki reports an error
maki show a1
```

This applies to both command arguments and filter values where asset IDs are expected.


## Exit Codes

| Code | Meaning |
|------|---------|
| **0** | Success. Command completed without errors. |
| **1** | Failure. Examples: `verify` found hash mismatches, a referenced asset/volume was not found, a required argument was missing, or any other command error. |

Standard Rust/clap error handling applies for invalid arguments, missing subcommands, and unknown flags -- these also exit with a non-zero code and print usage help to stderr.


## Output Conventions

maki separates machine output from human messages:

| Stream | Content |
|--------|---------|
| **stdout** | Command results: search results, asset details, JSON output, format-template output |
| **stderr** | Progress messages, per-file logs (`--log`), debug output (`--debug`), timing (`--time`), warnings, and errors |

This separation means you can safely pipe or redirect stdout without capturing status noise:

```bash
maki search -q "tag:landscape" > asset-ids.txt
maki stats --json | jq '.total_size'
```

### Search result count

`maki search` prints a result count header by default (e.g. `Found 42 assets`). This header is suppressed when an explicit `--format` is given (including `-q`, which is shorthand for `--format=ids`), keeping output clean for scripting.


## Scripting Patterns

### Get just IDs for piping

```bash
maki search -q "tag:landscape"
```

`-q` is shorthand for `--format=ids` and prints one UUID per line with no header.

### JSON processing with jq

```bash
maki search "rating:5" --json | jq '.[].id'
maki show abc123 --json | jq '.variants[].filename'
maki stats --json | jq '{assets: .assets, size: .total_size}'
```

### Pipe search results into a collection

```bash
maki search -q "rating:5 tag:travel" | xargs maki col add "Travel Best"
```

### Batch operations via shell loop

```bash
for id in $(maki search -q "type:video"); do
  maki edit "$id" --label Blue
done
```

### Dry-run before applying

Several commands default to report-only mode and require `--apply` to make changes:

```bash
# Preview what would be imported
maki import --dry-run /path/to/files

# See what auto-group would merge (no --apply = dry-run)
maki auto-group "tag:landscape"
maki auto-group "tag:landscape" --apply    # actually merge

# See what sync would update (no --apply = report-only)
maki sync /Volumes/Photos
maki sync /Volumes/Photos --apply          # actually update

# See what cleanup would remove (no --apply = report-only)
maki cleanup
maki cleanup --apply                       # actually remove stale records
```

### Combining flags for verbose dry-runs

```bash
maki import --dry-run /path/to/files --log --time --json
maki sync /Volumes/Photos --log --json
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
| `volume remove` | Report-only (no changes) | `--apply` |
| `volume combine` | Report-only (no changes) | `--apply` |
| `import` | Imports immediately | `--dry-run` to preview |
| `relocate` | Copies immediately | `--dry-run` to preview |

The pattern is consistent: commands that scan and potentially alter many records default to showing you what they *would* do. You opt in to changes with `--apply`. Commands that operate on explicitly named files (import, relocate) run immediately but offer `--dry-run` for previewing.


## Command Categories

Quick reference of all maki commands, organized by workflow stage:

| Category | Commands | Purpose |
|----------|----------|---------|
| **Setup** | `init`, `volume add`, `volume list`, `volume combine`, `volume remove` | Create catalog, register and manage storage volumes |
| **Ingest** | `import`, `tag`, `edit`, `group`, `auto-group` | Bring files in, apply metadata, merge variants |
| **Organize** | `collection` (`col`), `saved-search` (`ss`) | Curate static and smart albums |
| **Retrieve** | `search`, `show`, `export`, `duplicates`, `stats`, `serve` | Find assets, inspect details, export files, browse in web UI |
| **Maintain** | `verify`, `sync`, `refresh`, `cleanup`, `relocate`, `update-location`, `generate-previews`, `fix-roles`, `fix-dates`, `rebuild-catalog` | Integrity checks, disk reconciliation, housekeeping |

Commands with aliases are shown with the alias in parentheses. For example, `maki col add` is equivalent to `maki collection add`, and `maki ss run` is equivalent to `maki saved-search run`.

---

Next: [Setup Commands](01-setup-commands.md) -- `init`, `volume add`, `volume list`, `volume combine`, `volume remove`.
