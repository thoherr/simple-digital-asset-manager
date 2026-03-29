#!/usr/bin/env python3
"""
Fix orphaned XMP files that were imported as standalone assets.

Problem: XMP sidecar files that ended up in a different directory than their
parent RAW/media file (e.g., XMP stayed in Capture/ while RAW was moved to
Selects/). During import, these became standalone assets of type "other"
instead of being attached as recipes.

Solution: This script finds all type:other format:xmp assets, locates the
matching RAW file by filename stem (cross-directory within a search root),
and moves the XMP file next to the RAW. After running this script, use:

    maki sync <volume-path> --apply    # update catalog for moved files
    maki fix-recipes --apply           # reattach XMPs as recipes

Usage:
    python3 scripts/fix-orphaned-xmp.py [--apply] [--remove] [--volume LABEL] [--path PREFIX]

    --path PREFIX   Scope to assets whose file path starts with PREFIX.
                    Can be an absolute path (auto-stripped to volume-relative)
                    or a volume-relative prefix (e.g. "2026-02" for a month
                    of sessions, or "2026-02-15" for a single day/session).
                    Also limits the RAW file search to the session root
                    (one level up from the XMP's directory).

    --remove        Delete XMP files that have no matching parent media file
                    (instead of skipping them). Useful for cleaning up stale
                    sidecars left behind in Capture/ folders.

Without --apply, runs in dry-run mode (report only, no files moved/removed).
"""

import argparse
import json
import os
import shutil
import subprocess
import sys
from pathlib import Path


def run_maki(*args):
    """Run a maki command and return parsed JSON output."""
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


def run_maki_lines(*args):
    """Run a maki command and return stdout lines."""
    cmd = ["maki"] + list(args)
    result = subprocess.run(cmd, capture_output=True, text=True)
    if result.returncode != 0:
        return []
    return [line.strip() for line in result.stdout.strip().splitlines() if line.strip()]


def get_volumes():
    """Get volume info: id -> {label, path, is_online}."""
    data = run_maki("volume", "list")
    if not data:
        return {}
    volumes = {}
    for v in data:
        volumes[v["id"]] = {
            "label": v["label"],
            "path": v["path"],
            "is_online": v.get("is_online", False),
        }
    return volumes


def find_raw_by_stem(stem, search_root, exclude_path):
    """
    Search for a RAW file with the given stem under search_root.
    Returns the absolute path if found, or None.
    """
    raw_extensions = {
        ".nef", ".nrw", ".cr2", ".cr3", ".arw", ".orf", ".raf",
        ".rw2", ".dng", ".pef", ".srw", ".x3f", ".iiq", ".3fr",
        ".rwl", ".raw", ".mos",
    }
    # Also check common image formats as fallback
    image_extensions = {".jpg", ".jpeg", ".tif", ".tiff", ".png", ".heic"}

    candidates = []

    try:
        result = subprocess.run(
            ["find", str(search_root), "-name", f"{stem}.*", "-type", "f"],
            capture_output=True, text=True, timeout=30,
        )
        for line in result.stdout.strip().splitlines():
            p = Path(line.strip())
            if str(p) == str(exclude_path):
                continue
            ext = p.suffix.lower()
            if ext in raw_extensions:
                candidates.append(("raw", p))
            elif ext in image_extensions:
                candidates.append(("image", p))
    except (subprocess.TimeoutExpired, Exception) as e:
        print(f"  WARNING: find timed out for stem '{stem}': {e}", file=sys.stderr)
        return None

    if not candidates:
        return None

    # Prefer RAW files over image files
    raw_matches = [p for role, p in candidates if role == "raw"]
    if raw_matches:
        return raw_matches[0]
    return candidates[0][1]


def main():
    parser = argparse.ArgumentParser(
        description="Fix orphaned XMP files imported as standalone assets"
    )
    parser.add_argument(
        "--apply", action="store_true",
        help="Actually move/remove files (default: dry-run report only)"
    )
    parser.add_argument(
        "--remove", action="store_true",
        help="Delete XMP files that have no matching parent media file"
    )
    parser.add_argument(
        "--volume", type=str, default=None,
        help="Limit to a specific volume label"
    )
    parser.add_argument(
        "--path", type=str, default=None,
        help="Path prefix to scope the search (e.g. '2026-02' for a month, "
             "or an absolute path like /Volumes/Photos/2026-02)"
    )
    args = parser.parse_args()

    mode = "APPLY" if args.apply else "DRY RUN"
    print(f"=== Fix Orphaned XMP Files ({mode}) ===\n")

    # Step 1: Find all orphaned XMP assets
    query = "type:other format:xmp"
    if args.volume:
        query += " volume:" + args.volume
    if args.path:
        # maki search handles path normalization (absolute -> volume-relative)
        query += f" path:{args.path}"
    ids = run_maki_lines("search", "-q", query)
    if not ids:
        print("No orphaned XMP assets found.")
        return

    print(f"Found {len(ids)} orphaned XMP asset(s)\n")

    # Step 2: Get volume info
    volumes = get_volumes()
    if not volumes:
        print("ERROR: Could not load volumes", file=sys.stderr)
        sys.exit(1)

    moved = 0
    removed = 0
    no_parent = 0
    offline = 0
    errors = 0

    for asset_id in ids:
        # Load full asset details
        details = run_maki("show", asset_id)
        if not details:
            errors += 1
            continue

        # Get the XMP file location
        variant = details.get("variants", [{}])[0] if details.get("variants") else {}
        locations = variant.get("locations", [])
        if not locations:
            print(f"  {asset_id}: no file locations, skipping")
            errors += 1
            continue

        filename = variant.get("original_filename", "?")

        for loc in locations:
            vol_id = loc.get("volume_id", "")
            rel_path = loc.get("relative_path", "")
            vol_info = volumes.get(vol_id)

            if not vol_info:
                print(f"  {filename}: unknown volume {vol_id}, skipping")
                errors += 1
                continue

            if not vol_info["is_online"]:
                print(f"  {filename}: volume '{vol_info['label']}' is offline, skipping")
                offline += 1
                continue

            mount = vol_info["path"]
            xmp_abs = Path(mount) / rel_path
            if not xmp_abs.exists():
                print(f"  {filename}: file not found at {xmp_abs}, skipping")
                errors += 1
                continue

            # Extract stem (handle compound extensions like DSC_001.NRW.xmp)
            stem = Path(rel_path).stem
            # If stem still has an extension (compound), strip it
            if "." in stem:
                stem = stem.rsplit(".", 1)[0]

            # Determine search root: go up one level from the XMP's directory
            # to the session root (e.g., Capture/ -> session/, Selects/ is a
            # sibling). This keeps the find fast and avoids false positives
            # from other sessions with restarted camera counters.
            xmp_dir = xmp_abs.parent
            session_root = xmp_dir.parent
            # Safety: don't search above the volume mount point
            if not str(session_root).startswith(str(mount)):
                session_root = Path(mount)

            parent_path = find_raw_by_stem(stem, session_root, xmp_abs)
            if not parent_path:
                if args.remove:
                    xmp_rel = os.path.relpath(xmp_abs, mount)
                    print(f"  {filename}: no parent, remove {xmp_rel}")
                    if args.apply:
                        try:
                            os.remove(str(xmp_abs))
                            removed += 1
                        except Exception as e:
                            print(f"    ERROR: {e}", file=sys.stderr)
                            errors += 1
                    else:
                        removed += 1
                else:
                    print(f"  {filename}: no matching media file for stem '{stem}' under {os.path.relpath(session_root, mount)}/")
                    no_parent += 1
                continue

            # Determine target path: same directory as parent, same filename
            target_dir = parent_path.parent
            target_path = target_dir / xmp_abs.name

            if target_path == xmp_abs:
                print(f"  {filename}: already in correct location")
                continue

            if target_path.exists():
                if args.remove:
                    xmp_rel = os.path.relpath(xmp_abs, mount)
                    print(f"  {filename}: target exists, remove {xmp_rel}")
                    if args.apply:
                        try:
                            os.remove(str(xmp_abs))
                            removed += 1
                        except Exception as e:
                            print(f"    ERROR: {e}", file=sys.stderr)
                            errors += 1
                    else:
                        removed += 1
                else:
                    print(f"  {filename}: target already exists: {target_path}")
                    no_parent += 1
                continue

            # Compute relative paths for display
            xmp_rel = os.path.relpath(xmp_abs, mount)
            target_rel = os.path.relpath(target_path, mount)
            print(f"  {filename}: {xmp_rel} -> {target_rel}")

            if args.apply:
                try:
                    shutil.move(str(xmp_abs), str(target_path))
                    moved += 1
                except Exception as e:
                    print(f"    ERROR: {e}", file=sys.stderr)
                    errors += 1
            else:
                moved += 1

    # Summary
    print(f"\n=== Summary ({mode}) ===")
    print(f"  Would move:  {moved}" if not args.apply else f"  Moved:       {moved}")
    print(f"  Would remove:{removed}" if not args.apply else f"  Removed:     {removed}")
    print(f"  No parent:   {no_parent}")
    print(f"  Offline:     {offline}")
    print(f"  Errors:      {errors}")

    changes = moved + removed
    if changes > 0 and not args.apply:
        print(f"\nRe-run with --apply to move/remove files, then:")
        print(f"  maki sync <volume-path> --apply    # update catalog for moved/removed files")
        if moved > 0:
            print(f"  maki fix-recipes --apply            # reattach moved XMPs as recipes")
        if removed > 0:
            print(f"  maki cleanup --apply                # remove stale records for deleted files")
    elif changes > 0 and args.apply:
        print(f"\nNext steps:")
        print(f"  maki sync <volume-path> --apply    # update catalog for moved/removed files")
        if moved > 0:
            print(f"  maki fix-recipes --apply            # reattach moved XMPs as recipes")
        if removed > 0:
            print(f"  maki cleanup --apply                # remove stale records for deleted files")


if __name__ == "__main__":
    main()
