#!/usr/bin/env python3
"""
Fix accidentally over-grouped assets by splitting them based on directory structure.

Finds assets with scattered variants (files in different directories) and splits
them so each directory-group becomes its own asset. After splitting, reimports
metadata and re-groups by filename stem within each session.

Uses the same session root detection as `maki auto-group`: the deepest directory
component matching the [group] session_root_pattern regex (default: ^\d{4}-\d{2})
defines the session boundary. Variants in the same session root stay together;
variants in different session roots are split apart.

Usage:
    # Preview what would be split (dry run)
    python3 scripts/fix-scattered-groups.py --min-scattered 4

    # Apply the fixes
    python3 scripts/fix-scattered-groups.py --min-scattered 4 --apply

    # Process a specific asset
    python3 scripts/fix-scattered-groups.py --asset a1b2c3d4

    # Custom session root pattern (overrides maki.toml)
    python3 scripts/fix-scattered-groups.py --min-scattered 4 --pattern '^(shoot|project)-'

    # Start with the worst offenders, review, then widen
    python3 scripts/fix-scattered-groups.py --min-scattered 10 --apply
    python3 scripts/fix-scattered-groups.py --min-scattered 4 --apply
"""

import argparse
import json
import os
import re
import subprocess
import sys
from collections import defaultdict
from pathlib import PurePosixPath


def maki_json(*args):
    """Run a maki command with --json and return parsed output."""
    cmd = ["maki", "--json"] + list(args)
    result = subprocess.run(cmd, capture_output=True, text=True)
    if result.returncode != 0:
        print(f"  ERROR: maki {' '.join(args)}: {result.stderr.strip()}", file=sys.stderr)
        return None
    try:
        return json.loads(result.stdout)
    except json.JSONDecodeError:
        print(f"  ERROR: failed to parse JSON from: maki {' '.join(args)}", file=sys.stderr)
        return None


def maki_ids(*args):
    """Run a maki search with -q and return a list of asset IDs."""
    cmd = ["maki", "search", "-q"] + list(args)
    result = subprocess.run(cmd, capture_output=True, text=True)
    if result.returncode != 0:
        return []
    return [line.strip() for line in result.stdout.strip().splitlines() if line.strip()]


def maki_run(*args):
    """Run a maki command and return (success, stdout, stderr)."""
    cmd = ["maki"] + list(args)
    result = subprocess.run(cmd, capture_output=True, text=True)
    return result.returncode == 0, result.stdout.strip(), result.stderr.strip()


DEFAULT_SESSION_ROOT_PATTERN = r"^\d{4}-\d{2}"


def load_session_root_pattern():
    """Load session_root_pattern from maki.toml if available."""
    try:
        import tomllib
    except ImportError:
        try:
            import tomli as tomllib
        except ImportError:
            return DEFAULT_SESSION_ROOT_PATTERN

    # Walk up from cwd to find maki.toml
    path = os.getcwd()
    while True:
        toml_path = os.path.join(path, "maki.toml")
        if os.path.isfile(toml_path):
            with open(toml_path, "rb") as f:
                config = tomllib.load(f)
            return config.get("group", {}).get("session_root_pattern", DEFAULT_SESSION_ROOT_PATTERN)
        parent = os.path.dirname(path)
        if parent == path:
            break
        path = parent
    return DEFAULT_SESSION_ROOT_PATTERN


def find_session_root(path, pattern):
    """Find the session root for a file path, matching maki's find_session_root().

    Walks directory components and finds the deepest one matching the pattern.
    Falls back to parent directory if no component matches.
    """
    parts = PurePosixPath(path).parts[:-1]  # remove filename
    if not parts:
        return "(root)"

    dir_path = "/".join(parts)

    if not pattern:
        # No pattern = fall back to parent directory
        return "/".join(parts[:-1]) if len(parts) > 1 else dir_path

    regex = re.compile(pattern)
    session_idx = None
    for i, part in enumerate(parts):
        if regex.search(part):
            session_idx = i

    if session_idx is not None:
        return "/".join(parts[:session_idx + 1])

    # No match — fall back to parent directory
    return "/".join(parts[:-1]) if len(parts) > 1 else dir_path


def analyze_asset(asset_id, pattern):
    """Analyze an asset and return variant groups by session root."""
    details = maki_json("show", asset_id)
    if not details or "variants" not in details:
        return None, None

    # Group variants by session root
    groups = defaultdict(list)
    for variant in details["variants"]:
        content_hash = variant["content_hash"]
        for loc in variant.get("locations", []):
            path = loc.get("relative_path", "")
            session = find_session_root(path, pattern)
            groups[session].append({
                "content_hash": content_hash,
                "filename": variant["original_filename"],
                "format": variant["format"],
                "path": path,
            })

    return details, dict(groups)


def main():
    parser = argparse.ArgumentParser(
        description="Fix accidentally over-grouped assets by splitting on directory structure"
    )
    parser.add_argument("--min-scattered", type=int, default=4,
                        help="Minimum scattered level to process (default: 4)")
    parser.add_argument("--pattern", type=str, default=None,
                        help="Session root regex pattern (overrides maki.toml; default: ^\\d{4}-\\d{2})")
    parser.add_argument("--asset", type=str,
                        help="Process a specific asset ID instead of searching")
    parser.add_argument("--apply", action="store_true",
                        help="Actually perform splits (default: dry run)")
    parser.add_argument("--skip-reimport", action="store_true",
                        help="Skip metadata reimport after split")
    parser.add_argument("--skip-regroup", action="store_true",
                        help="Skip auto-group after split")
    parser.add_argument("--limit", type=int, default=0,
                        help="Process at most N assets (0 = unlimited)")
    args = parser.parse_args()

    # Resolve session root pattern
    pattern = args.pattern if args.pattern is not None else load_session_root_pattern()

    # Find affected assets
    if args.asset:
        asset_ids = [args.asset]
    else:
        query = f"scattered:{args.min_scattered}+"
        print(f"Searching for assets with {query}...")
        asset_ids = maki_ids(query)
        print(f"Found {len(asset_ids)} asset(s)")

    if not asset_ids:
        print("No assets to process.")
        return

    if args.limit > 0:
        asset_ids = asset_ids[:args.limit]
        print(f"Processing first {args.limit} asset(s)")

    # Phase 1: Analyze
    print(f"\n{'=' * 60}")
    print(f"{'DRY RUN' if not args.apply else 'APPLYING'} — session root pattern: {pattern or '(none, parent-dir fallback)'}")
    print(f"{'=' * 60}\n")

    total_splits = 0
    split_plan = []

    for i, asset_id in enumerate(asset_ids):
        short_id = asset_id[:8]
        details, groups = analyze_asset(asset_id, pattern)
        if not details or not groups:
            print(f"  [{i+1}/{len(asset_ids)}] {short_id} — skipped (could not load)")
            continue

        name = details.get("name") or details["variants"][0]["original_filename"]
        total_variants = len(details["variants"])

        if len(groups) <= 1:
            # All variants in same directory — nothing to split
            continue

        print(f"  [{i+1}/{len(asset_ids)}] {short_id} ({name}) — {total_variants} variants in {len(groups)} directory groups:")
        for dir_key, variants in sorted(groups.items()):
            hashes = [v["content_hash"] for v in variants]
            files = [f"{v['filename']} ({v['format']})" for v in variants]
            print(f"    {dir_key}/")
            for f in files:
                print(f"      {f}")

        # Determine which group to keep with the original asset.
        # The asset ID is derived from a specific variant's hash (UUID v5).
        # We must keep the group containing that variant to avoid splitting
        # away the identity variant. Find it by checking which variant hash
        # was used to generate the asset UUID.
        #
        # Since we can't easily recompute UUID v5 in Python without the
        # namespace, we use a safe heuristic: keep the group containing
        # the FIRST variant listed (variants[0] is typically the original
        # that created the asset).
        first_hash = details["variants"][0]["content_hash"]
        keep_dir = None
        for dir_key, variants in groups.items():
            if any(v["content_hash"] == first_hash for v in variants):
                keep_dir = dir_key
                break
        if keep_dir is None:
            # Fallback: keep the largest group
            keep_dir = max(groups.items(), key=lambda x: len(x[1]))[0]

        keep_variants = groups[keep_dir]
        split_groups = [(d, v) for d, v in groups.items() if d != keep_dir]

        print(f"    → Keep {len(keep_variants)} variant(s) in {keep_dir}/ (contains identity variant)")
        for dir_key, variants in split_groups:
            hashes = list(set(v["content_hash"] for v in variants))
            print(f"    → Split {len(variants)} variant(s) from {dir_key}/")
            split_plan.append({
                "asset_id": asset_id,
                "split_hashes": hashes,
                "dir": dir_key,
            })
            total_splits += 1
        print()

    print(f"{'=' * 60}")
    print(f"Summary: {total_splits} split(s) across {len(asset_ids)} asset(s)")

    if not args.apply:
        print("Dry run — no changes made. Run with --apply to execute.")
        return

    if total_splits == 0:
        print("Nothing to split.")
        return

    # Phase 2: Execute splits
    print(f"\nExecuting {total_splits} split(s)...\n")

    new_asset_ids = []
    for entry in split_plan:
        asset_id = entry["asset_id"]
        hashes = entry["split_hashes"]
        short_id = asset_id[:8]

        # maki split <asset-id> <hash1> <hash2> ...
        result = maki_json("split", asset_id, *hashes)
        if result:
            new_ids = result.get("new_asset_ids", [])
            print(f"  Split {short_id}: {len(hashes)} variant(s) from {entry['dir']}/")
            for nid in new_ids:
                print(f"    New asset: {nid[:8]}")
                new_asset_ids.append(nid)
            new_asset_ids.append(asset_id)  # reimport the source too
        else:
            # Fallback: try without --json
            ok, stdout, stderr = maki_run("split", asset_id, *hashes)
            if ok:
                print(f"  Split {short_id}: {len(hashes)} variant(s) from {entry['dir']}/")
                new_asset_ids.append(asset_id)
            else:
                print(f"  FAILED split {short_id}: {stderr}")

    # Phase 3: Reimport metadata
    if not args.skip_reimport and new_asset_ids:
        print(f"\nReimporting metadata for {len(new_asset_ids)} affected asset(s)...")
        # Also reimport the newly created assets — find them by searching
        # for recently modified assets (the split creates new ones)
        for aid in set(new_asset_ids):
            ok, stdout, stderr = maki_run("refresh", "--reimport", "--asset", aid)
            if ok:
                print(f"  Reimported {aid[:8]}")
            else:
                print(f"  FAILED reimport {aid[:8]}: {stderr}")

    # Phase 4: Re-group by stem (scoped to affected assets)
    # auto-group is directory-local by default (uses session_root_pattern),
    # so it's safe to run on the affected assets without cross-session merging.
    if not args.skip_regroup and new_asset_ids:
        unique_ids = sorted(set(new_asset_ids))
        # Build an id: query to scope auto-group to affected assets only
        id_query = " ".join(f"id:{aid}" for aid in unique_ids)
        print(f"\nRe-grouping {len(unique_ids)} affected asset(s) by filename stem...")
        ok, stdout, stderr = maki_run("auto-group", "--apply", "--log", id_query)
        if ok:
            print(f"  {stdout}")
        else:
            print(f"  Auto-group: {stderr}")

    print("\nDone.")
    print("Review the results in the web UI and run 'maki generate-previews --upgrade' if needed.")


if __name__ == "__main__":
    main()
