#!/bin/bash
#
# Snapshot the MAKI catalog metadata using git.
#
# Usage:
#   bash scripts/backup-catalog.sh                        # auto-generated message
#   bash scripts/backup-catalog.sh "Before tag cleanup"   # custom message
#
# Run this before bulk operations (tag rename, fix-roles, dedup, etc.)
# to enable easy rollback with 'git log' and 'git checkout'.
#
# The .gitignore created by 'maki init' excludes derived files (SQLite
# database, previews, embeddings). Only source-of-truth files are tracked:
# metadata YAML sidecars, volumes.yaml, collections, stacks, searches,
# and maki.toml.
#
# To restore after a mistake:
#   cd <catalog-root>
#   git log --oneline                    # find the commit to restore
#   git diff HEAD~1                      # see what changed
#   git checkout <commit> -- metadata/   # restore metadata files
#   maki rebuild-catalog                 # rebuild SQLite from restored YAML

set -e

# Find catalog root (walk up from current directory looking for maki.toml)
ROOT="$PWD"
while [ "$ROOT" != "/" ]; do
    [ -f "$ROOT/maki.toml" ] && break
    ROOT="$(dirname "$ROOT")"
done

if [ ! -f "$ROOT/maki.toml" ]; then
    echo "Error: no maki catalog found (no maki.toml in current or parent directories)" >&2
    exit 1
fi

cd "$ROOT"

# Initialize git repo if needed
if [ ! -d .git ]; then
    git init -q
    echo "Initialized git repository in $ROOT"
fi

# Snapshot
git add -A
if git diff --cached --quiet; then
    echo "No changes to commit."
else
    MSG="${1:-Catalog snapshot $(date +%Y-%m-%d_%H:%M:%S)}"
    git commit -m "$MSG" -q
    echo "Catalog backed up: $(git log --oneline -1)"
fi
