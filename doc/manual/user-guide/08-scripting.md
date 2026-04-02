# Scripting

MAKI's CLI is designed for composability with shell scripts and external programs. Every command supports `--json` for structured output, `-q` for piping asset IDs, and stderr/stdout separation so human messages never pollute machine-readable output.

This chapter covers practical scripting patterns using bash and Python. For interactive multi-step workflows with variables and tab completion, see the [Interactive Shell](09-shell.md) chapter — `maki shell` provides a REPL that eliminates much of the piping and quoting complexity shown here.

---

## Bash Scripting

### Batch Tagging by Path

Tag all assets from a specific session directory:

```bash
maki search -q "path:Capture/2026-02-15" | xargs -I{} maki tag {} session/february
```

### Conditional Edits

Set a color label on all unrated images:

```bash
for id in $(maki search -q "type:image rating:0"); do
    maki edit "$id" --label Yellow
done
```

Rate all videos from a specific camera:

```bash
for id in $(maki search -q 'type:video camera:"NIKON Z 9"'); do
    maki edit "$id" --rating 3
done
```

### Multi-Step Workflows

Import, auto-group, and generate smart previews for a new shoot:

```bash
#!/usr/bin/env bash
set -euo pipefail

SESSION="/Volumes/Photos/Capture/2026-02-28"

echo "Importing..."
maki import --smart --auto-group "$SESSION" --log --time

echo "Fixing roles..."
maki fix-roles --apply --log

echo "Done."
maki stats
```

### Exporting for Delivery

Export rated picks to a client folder with sidecars:

```bash
maki export "rating:4+ collection:ClientProject" /tmp/delivery/ --include-sidecars --log
```

Mirror directory structure to a USB drive:

```bash
maki export "tag:portfolio" /Volumes/USB/export/ --layout mirror
```

Dry-run first to check what would be exported:

```bash
maki export "collection:Print" /tmp/test/ --dry-run --json | jq '{files: .files_exported, size: .total_bytes}'
```

### Reporting with jq

Generate a tab-separated report of all 5-star assets with their tags:

```bash
maki search "rating:5" --json \
  | jq -r '.[] | [.original_filename, .created_at, (.tags // [] | join(", "))] | @tsv'
```

List volumes and their online status:

```bash
maki volume list --json | jq -r '.[] | "\(.label)\t\(if .is_online then "online" else "offline" end)\t\(.path // "—")"'
```

Count assets per format:

```bash
maki stats --types --json | jq -r '.types.variant_formats[] | "\(.format)\t\(.count)"'
```

### Building Collections from Filters

Create a "Best of 2026" collection from highly-rated images:

```bash
maki col create "Best of 2026"
maki search -q "rating:4+ dateFrom:2026-01-01 dateUntil:2026-12-31" \
  | xargs maki col add "Best of 2026"
echo "Added $(maki col show 'Best of 2026' -q | wc -l | tr -d ' ') assets"
```

### Verification and Health Checks

Weekly integrity check with notification:

```bash
#!/usr/bin/env bash
set -euo pipefail

LOG="/tmp/maki-verify-$(date +%Y%m%d).log"
if maki verify --log 2>"$LOG"; then
    echo "Verification passed" >> "$LOG"
else
    echo "VERIFICATION FAILURES DETECTED" >> "$LOG"
    # Add notification here (e.g., mail, Slack webhook)
fi
```

Find assets at risk (only one copy):

```bash
maki search -q "copies:1" | wc -l
```

### Sync After External Edits

After editing files in CaptureOne or Lightroom:

```bash
# Detect what changed
maki sync /Volumes/Photos

# Apply catalog updates
maki sync /Volumes/Photos --apply --log

# Re-read modified XMP metadata
maki refresh /Volumes/Photos --log
```

---

## Python Scripting

Python scripts can call `maki` as a subprocess and parse the `--json` output. This is useful for more complex logic that would be unwieldy in bash. The `scripts/` directory in the repository contains ready-to-use helper functions and example scripts.

### Helper Functions

The `scripts/maki_helpers.py` module provides two functions for calling maki from Python:

```python
from maki_helpers import maki_json, maki_ids

# Get parsed JSON output from any maki command
stats = maki_json("stats", "--tags")

# Get a list of asset IDs matching a search
ids = maki_ids("rating:5 tag:landscape")
```

The full source is at `scripts/maki_helpers.py`. Import it into your own scripts with `sys.path.insert(0, "scripts")` or copy the functions directly.

### Example: Tag Analysis Report

Find tags that are only used once (potential typos or inconsistencies). Full script: `scripts/tag-analysis.py`.

```bash
python3 scripts/tag-analysis.py
```

```python
stats = maki_json("stats", "--tags")
if stats and stats.get("tags"):
    for tag in stats["tags"].get("top_tags", []):
        if tag["count"] == 1:
            print(f"  Singleton tag: {tag['tag']}")
```

### Example: Cross-Volume Backup Audit

Check which assets exist on only one volume. Full script: `scripts/backup-audit.py`.

```bash
python3 scripts/backup-audit.py --limit 20
```

```python
at_risk = maki_ids("copies:1")
print(f"Assets with only 1 copy: {len(at_risk)}")

for asset_id in at_risk[:10]:  # show first 10
    details = maki_json("show", asset_id)
    if details:
        filename = details["variants"][0]["original_filename"]
        print(f"  {filename}")
```

### Example: Batch Operations with Progress

Apply ratings based on an external CSV file. Full script: `scripts/batch-rate-from-csv.py`.

```bash
# Dry run
python3 scripts/batch-rate-from-csv.py ratings.csv

# Apply
python3 scripts/batch-rate-from-csv.py ratings.csv --apply
```

```python
import csv

with open("ratings.csv") as f:
    reader = csv.DictReader(f)
    for row in reader:
        asset_id = row["asset_id"]
        rating = row["rating"]
        result = subprocess.run(
            ["maki", "edit", asset_id, "--rating", rating],
            capture_output=True, text=True
        )
        if result.returncode == 0:
            print(f"  {asset_id[:8]}: rated {rating}")
        else:
            print(f"  {asset_id[:8]}: FAILED — {result.stderr.strip()}")
```

---

## Real-World Example: Fix Orphaned XMP Files

The repository includes a complete Python script at `scripts/fix-orphaned-xmp.py` that demonstrates a real-world workflow automation.

**Problem**: XMP sidecar files that ended up in a different directory than their parent RAW file (e.g., XMP stayed in `Capture/` while the RAW was moved to `Selects/`). During import, these became standalone assets of type "other" instead of being attached as recipes.

**Solution**: The script uses maki's CLI to find orphaned XMPs, locates matching RAW files by filename stem, and moves the XMP files to the correct directory. It follows the standard two-phase pattern: make file changes, then let maki reconcile.

```bash
# Dry run — see what would be moved
python3 scripts/fix-orphaned-xmp.py --path 2026-02

# Apply changes
python3 scripts/fix-orphaned-xmp.py --path 2026-02 --apply

# Reconcile catalog with file moves
maki sync /Volumes/Photos --apply
maki fix-recipes --apply
```

Key patterns demonstrated in the script:

- **`maki search -q "type:other format:xmp"`** — find assets matching specific criteria
- **`maki show <id> --json`** — get full asset details including file locations
- **`maki volume list --json`** — enumerate volumes and their mount points
- **Dry-run by default** — `--apply` flag opts in to changes (following maki's convention)
- **Path scoping** — `--path` parameter limits the search to a subset of the catalog

---

## Bulk Operations on a List of IDs

A common pattern: you have a file of asset IDs (one per line) and want to apply the same operation to all of them. There are several approaches, from simple to fast.

### xargs (simple, one process per asset)

```bash
# Tag all assets in the list
cat ids.txt | xargs -I{} maki tag {} livestream

# Rate all assets
cat ids.txt | xargs -I{} maki edit {} --rating 3

# Delete (preview first, then apply)
cat ids.txt | xargs -I{} maki delete {}
cat ids.txt | maki delete --apply
```

This spawns a new `maki` process per line. Fine for hundreds of assets, slow for thousands.

### Shell loop (simple, slightly faster)

```bash
while read id; do
    maki tag "$id" livestream
done < ids.txt
```

Same speed as xargs (one process per asset), but easier to add conditional logic.

### maki shell script (fast, single process)

The interactive shell keeps the catalog open across commands. Generate a script file and run it in one shot:

```bash
# Generate a shell script from the ID list
awk '{print "tag " $0 " livestream"}' ids.txt > /tmp/retag.maki

# Execute all commands in a single maki process
maki shell /tmp/retag.maki
```

This is significantly faster for large batches (thousands of assets) because it avoids the overhead of opening the catalog for every command.

### Commands that read IDs from stdin

Some commands accept asset IDs on stdin when no positional IDs are given:

```bash
# Delete assets from a list
cat ids.txt | maki delete --apply

# Relocate assets from a list
cat ids.txt | maki relocate --target "Archive" --log
```

### Where do ID lists come from?

```bash
# Search results
maki search -q "tag:landscape rating:4+" > landscape-ids.txt

# Backup audit
maki backup-status --at-risk -q > at-risk-ids.txt

# External sources (CSV, spreadsheet, other tools)
cut -d, -f1 ratings.csv | tail -n +2 > ids.txt

# Saved from a previous session
maki shell -c 'search tag:concert' > concert-ids.txt
```

---

## Tips

**Avoid quoting pitfalls.** When search queries contain inner quotes (e.g., `tag:"Fools Theater"`), use single quotes for the outer shell argument:

```bash
maki search -q 'tag:"Fools Theater"'
```

**Use `--json` for reliable parsing.** Human-readable output is designed for readability, not stability. Field positions, formatting, and labels may change between versions. JSON output is versioned and stable.

**Stderr is for humans, stdout is for machines.** Progress messages, warnings, and `--log` output go to stderr. When piping maki's output, only structured results appear on stdout.

**Check exit codes.** maki returns 0 on success and 1 on failure. Commands like `verify` return 1 if any mismatches are found, making them suitable for CI/cron health checks:

```bash
maki verify --volume "Photos" || echo "Integrity check failed!"
```

**Dry-run first.** Most destructive commands (sync, cleanup, auto-group, fix-roles) default to report-only mode. Always review the dry-run output before adding `--apply`.

---

## Related Topics

- [Interactive Shell](09-shell.md) -- `maki shell` REPL with variables, tab completion, script files, and session management
- [CLI Conventions](../reference/00-cli-conventions.md) -- global flags, exit codes, and basic scripting patterns
- [Browsing & Searching](05-browse-and-search.md) -- search syntax and output format options
- [Format Templates Reference](../reference/07-format-templates.md) -- custom output templates for `--format`
- [Search Filters Reference](../reference/06-search-filters.md) -- all available filters
- [REST API](../developer/01-rest-api.md) -- programmatic access via HTTP
