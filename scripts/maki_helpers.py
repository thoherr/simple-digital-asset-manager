#!/usr/bin/env python3
"""
Helper functions for calling MAKI from Python scripts.

Usage:
    from maki_helpers import maki_json, maki_ids

    stats = maki_json("stats", "--tags")
    ids = maki_ids("rating:5")
"""

import json
import subprocess
import sys


def maki_json(*args):
    """Run a maki command with --json and return parsed output."""
    cmd = ["maki", "--json"] + list(args)
    result = subprocess.run(cmd, capture_output=True, text=True)
    if result.returncode != 0:
        print(f"Error: maki {' '.join(args)}: {result.stderr.strip()}", file=sys.stderr)
        return None
    return json.loads(result.stdout)


def maki_ids(*args):
    """Run a maki search with -q and return a list of asset IDs."""
    cmd = ["maki", "search", "-q"] + list(args)
    result = subprocess.run(cmd, capture_output=True, text=True)
    if result.returncode != 0:
        return []
    return [line.strip() for line in result.stdout.strip().splitlines() if line.strip()]
