#!/usr/bin/env python3
"""
Cross-volume backup audit: find assets with only one copy (at risk).

Usage:
    python3 scripts/backup-audit.py [--limit N]
"""

import argparse
import sys
sys.path.insert(0, "scripts")
from maki_helpers import maki_json, maki_ids

parser = argparse.ArgumentParser(description="Find under-backed-up assets")
parser.add_argument("--limit", type=int, default=10, help="Max assets to show (default: 10)")
args = parser.parse_args()

at_risk = maki_ids("copies:1")
print(f"Assets with only 1 copy: {len(at_risk)}")

if at_risk:
    print(f"\nFirst {min(args.limit, len(at_risk))} at-risk assets:")
    for asset_id in at_risk[:args.limit]:
        details = maki_json("show", asset_id)
        if details and details.get("variants"):
            filename = details["variants"][0]["original_filename"]
            locations = sum(len(v.get("locations", [])) for v in details["variants"])
            print(f"  {filename} ({locations} location(s))")
