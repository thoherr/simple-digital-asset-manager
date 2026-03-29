#!/usr/bin/env python3
"""
Apply ratings from an external CSV file.

Expected CSV format:
    asset_id,rating
    a1b2c3d4-...,5
    e5f6a7b8-...,3

Usage:
    python3 scripts/batch-rate-from-csv.py ratings.csv [--apply]
"""

import argparse
import csv
import subprocess
import sys


def main():
    parser = argparse.ArgumentParser(description="Apply ratings from CSV")
    parser.add_argument("csv_file", help="CSV file with asset_id and rating columns")
    parser.add_argument("--apply", action="store_true", help="Actually apply ratings (default: dry run)")
    args = parser.parse_args()

    applied = 0
    errors = 0

    with open(args.csv_file) as f:
        reader = csv.DictReader(f)
        for row in reader:
            asset_id = row["asset_id"]
            rating = row["rating"]
            if args.apply:
                result = subprocess.run(
                    ["maki", "edit", asset_id, "--rating", rating],
                    capture_output=True, text=True
                )
                if result.returncode == 0:
                    print(f"  {asset_id[:8]}: rated {rating}")
                    applied += 1
                else:
                    print(f"  {asset_id[:8]}: FAILED — {result.stderr.strip()}")
                    errors += 1
            else:
                print(f"  {asset_id[:8]}: would rate {rating}")
                applied += 1

    verb = "Applied" if args.apply else "Would apply"
    print(f"\n{verb} {applied} rating(s), {errors} error(s)")
    if not args.apply and applied > 0:
        print("Run with --apply to make changes.")


if __name__ == "__main__":
    main()
