# Scripting

dam's CLI is designed for composability with shell scripts and external programs. Every command supports `--json` for structured output, `-q` for piping asset IDs, and stderr/stdout separation so human messages never pollute machine-readable output.

This chapter covers practical scripting patterns beyond the basics shown in [Browsing & Searching](05-browse-and-search.md) and [CLI Conventions](../reference/00-cli-conventions.md).

---

## Bash Scripting

### Batch Tagging by Path

Tag all assets from a specific session directory:

```bash
dam search -q "path:Capture/2026-02-15" | xargs -I{} dam tag {} session/february
```

### Conditional Edits

Set a color label on all unrated images:

```bash
for id in $(dam search -q "type:image rating:0"); do
    dam edit "$id" --label Yellow
done
```

Rate all videos from a specific camera:

```bash
for id in $(dam search -q 'type:video camera:"NIKON Z 9"'); do
    dam edit "$id" --rating 3
done
```

### Multi-Step Workflows

Import, auto-group, and generate smart previews for a new shoot:

```bash
#!/usr/bin/env bash
set -euo pipefail

SESSION="/Volumes/Photos/Capture/2026-02-28"

echo "Importing..."
dam import --smart --auto-group "$SESSION" --log --time

echo "Fixing roles..."
dam fix-roles --apply --log

echo "Done."
dam stats
```

### Reporting with jq

Generate a tab-separated report of all 5-star assets with their tags:

```bash
dam search "rating:5" --json \
  | jq -r '.[] | [.original_filename, .created_at, (.tags // [] | join(", "))] | @tsv'
```

List volumes and their online status:

```bash
dam volume list --json | jq -r '.[] | "\(.label)\t\(if .is_online then "online" else "offline" end)\t\(.path // "—")"'
```

Count assets per format:

```bash
dam stats --types --json | jq -r '.types.variant_formats[] | "\(.format)\t\(.count)"'
```

### Building Collections from Filters

Create a "Best of 2026" collection from highly-rated images:

```bash
dam col create "Best of 2026"
dam search -q "rating:4+ dateFrom:2026-01-01 dateUntil:2026-12-31" \
  | xargs dam col add "Best of 2026"
echo "Added $(dam col show 'Best of 2026' -q | wc -l | tr -d ' ') assets"
```

### Verification and Health Checks

Weekly integrity check with notification:

```bash
#!/usr/bin/env bash
set -euo pipefail

LOG="/tmp/dam-verify-$(date +%Y%m%d).log"
if dam verify --log 2>"$LOG"; then
    echo "Verification passed" >> "$LOG"
else
    echo "VERIFICATION FAILURES DETECTED" >> "$LOG"
    # Add notification here (e.g., mail, Slack webhook)
fi
```

Find assets at risk (only one copy):

```bash
dam search -q "copies:1" | wc -l
```

### Sync After External Edits

After editing files in CaptureOne or Lightroom:

```bash
# Detect what changed
dam sync /Volumes/Photos

# Apply catalog updates
dam sync /Volumes/Photos --apply --log

# Re-read modified XMP metadata
dam refresh /Volumes/Photos --log
```

---

## Python Scripting

Python scripts can call `dam` as a subprocess and parse the `--json` output. This is useful for more complex logic that would be unwieldy in bash.

### Helper Functions

A minimal helper for calling dam from Python:

```python
import json
import subprocess
import sys

def dam_json(*args):
    """Run a dam command with --json and return parsed output."""
    cmd = ["dam", "--json"] + list(args)
    result = subprocess.run(cmd, capture_output=True, text=True)
    if result.returncode != 0:
        print(f"Error: dam {' '.join(args)}: {result.stderr.strip()}", file=sys.stderr)
        return None
    return json.loads(result.stdout)

def dam_ids(*args):
    """Run a dam search with -q and return a list of asset IDs."""
    cmd = ["dam", "search", "-q"] + list(args)
    result = subprocess.run(cmd, capture_output=True, text=True)
    if result.returncode != 0:
        return []
    return [line.strip() for line in result.stdout.strip().splitlines() if line.strip()]
```

### Example: Tag Analysis Report

Find tags that are only used once (potential typos or inconsistencies):

```python
stats = dam_json("stats", "--tags")
if stats and stats.get("tags"):
    for tag in stats["tags"].get("top_tags", []):
        if tag["count"] == 1:
            print(f"  Singleton tag: {tag['tag']}")
```

### Example: Cross-Volume Backup Audit

Check which assets exist on only one volume:

```python
at_risk = dam_ids("copies:1")
print(f"Assets with only 1 copy: {len(at_risk)}")

for asset_id in at_risk[:10]:  # show first 10
    details = dam_json("show", asset_id)
    if details:
        filename = details["variants"][0]["original_filename"]
        print(f"  {filename}")
```

### Example: Batch Operations with Progress

Apply ratings based on an external CSV file:

```python
import csv

with open("ratings.csv") as f:
    reader = csv.DictReader(f)
    for row in reader:
        asset_id = row["asset_id"]
        rating = row["rating"]
        result = subprocess.run(
            ["dam", "edit", asset_id, "--rating", rating],
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

**Solution**: The script uses dam's CLI to find orphaned XMPs, locates matching RAW files by filename stem, and moves the XMP files to the correct directory. It follows the standard two-phase pattern: make file changes, then let dam reconcile.

```bash
# Dry run — see what would be moved
python3 scripts/fix-orphaned-xmp.py --path 2026-02

# Apply changes
python3 scripts/fix-orphaned-xmp.py --path 2026-02 --apply

# Reconcile catalog with file moves
dam sync /Volumes/Photos --apply
dam fix-recipes --apply
```

Key patterns demonstrated in the script:

- **`dam search -q "type:other format:xmp"`** — find assets matching specific criteria
- **`dam show <id> --json`** — get full asset details including file locations
- **`dam volume list --json`** — enumerate volumes and their mount points
- **Dry-run by default** — `--apply` flag opts in to changes (following dam's convention)
- **Path scoping** — `--path` parameter limits the search to a subset of the catalog

---

## Tips

**Avoid quoting pitfalls.** When search queries contain inner quotes (e.g., `tag:"Fools Theater"`), use single quotes for the outer shell argument:

```bash
dam search -q 'tag:"Fools Theater"'
```

**Use `--json` for reliable parsing.** Human-readable output is designed for readability, not stability. Field positions, formatting, and labels may change between versions. JSON output is versioned and stable.

**Stderr is for humans, stdout is for machines.** Progress messages, warnings, and `--log` output go to stderr. When piping dam's output, only structured results appear on stdout.

**Check exit codes.** dam returns 0 on success and 1 on failure. Commands like `verify` return 1 if any mismatches are found, making them suitable for CI/cron health checks:

```bash
dam verify --volume "Photos" || echo "Integrity check failed!"
```

**Dry-run first.** Most destructive commands (sync, cleanup, auto-group, fix-roles) default to report-only mode. Always review the dry-run output before adding `--apply`.

---

## Related Topics

- [CLI Conventions](../reference/00-cli-conventions.md) -- global flags, exit codes, and basic scripting patterns
- [Browsing & Searching](05-browse-and-search.md) -- search syntax and output format options
- [Format Templates Reference](../reference/07-format-templates.md) -- custom output templates for `--format`
- [Search Filters Reference](../reference/06-search-filters.md) -- all available filters
- [REST API](../developer/01-rest-api.md) -- programmatic access via HTTP
