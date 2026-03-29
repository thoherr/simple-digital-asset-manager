#!/usr/bin/env python3
"""
Find singleton tags (used only once) — potential typos or inconsistencies.

Usage:
    python3 scripts/tag-analysis.py
"""

import sys
sys.path.insert(0, "scripts")
from maki_helpers import maki_json

stats = maki_json("stats", "--tags")
if not stats or "tags" not in stats:
    print("No tag statistics available.", file=sys.stderr)
    sys.exit(1)

singletons = [tag for tag in stats["tags"].get("top_tags", []) if tag["count"] == 1]
if singletons:
    print(f"Found {len(singletons)} singleton tag(s):")
    for tag in singletons:
        print(f"  {tag['tag']}")
else:
    print("No singleton tags found.")
