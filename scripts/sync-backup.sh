#!/bin/bash
#
# Sync a full MAKI catalog backup using rsync.
#
# Usage:
#   bash scripts/sync-backup.sh                              # backup to ../maki.backup/
#   bash scripts/sync-backup.sh /Volumes/BackupDisk/catalog  # backup to external drive
#   bash scripts/sync-backup.sh --dry-run                    # preview what would change
#   bash scripts/sync-backup.sh /path/to/dest --dry-run      # preview to specific target
#
# This creates a 1:1 mirror of the entire catalog directory including:
#   - maki.toml, volumes.yaml, collections, searches, vocabulary
#   - metadata/ (YAML sidecars — source of truth)
#   - catalog.db (SQLite — derived, but expensive to rebuild)
#   - previews/, smart-previews/, embeddings/, faces/
#
# Complements scripts/backup-catalog.sh (git-based, metadata only) by
# including all derived files so a restore doesn't require hours of
# rebuilding previews and embeddings.
#
# Safety:
#   - SQLite WAL is checkpointed before syncing (ensures catalog.db
#     is self-contained and safe to copy).
#   - Permissions/ownership are NOT synced (uses -rlt instead of -a to
#     skip perms/owner/group — just file content + timestamps).
#   - --delete removes files from backup that no longer exist in source.
#   - Excludes .git/ (from backup-catalog.sh) and temporary files.
#
# Restore:
#   rsync -a --no-perms --no-owner --no-group /path/to/backup/ /path/to/catalog/
#   # or simply copy the backup directory back

set -e

# ── Parse arguments ──────────────────────────────────────────────
DRY_RUN=""
DEST=""
for arg in "$@"; do
    case "$arg" in
        --dry-run|-n) DRY_RUN="--dry-run" ;;
        *) DEST="$arg" ;;
    esac
done

# ── Find catalog root ───────────────────────────────────────────
ROOT="$PWD"
while [ "$ROOT" != "/" ]; do
    [ -f "$ROOT/maki.toml" ] && break
    ROOT="$(dirname "$ROOT")"
done

if [ ! -f "$ROOT/maki.toml" ]; then
    echo "Error: no maki catalog found (no maki.toml in current or parent directories)" >&2
    exit 1
fi

# ── Default destination: ../maki.backup/ ─────────────────────────
if [ -z "$DEST" ]; then
    DEST="$(dirname "$ROOT")/maki.backup"
fi

echo "Source:  $ROOT/"
echo "Dest:    $DEST/"
if [ -n "$DRY_RUN" ]; then
    echo "Mode:    DRY RUN (no changes)"
fi
echo ""

# ── Checkpoint SQLite WAL ────────────────────────────────────────
# This ensures catalog.db is self-contained (no pending WAL journal).
# Safe to skip if sqlite3 is not installed — rsync will copy whatever
# state catalog.db is in, which is fine if no MAKI command is running.
DB="$ROOT/catalog.db"
if [ -f "$DB" ] && command -v sqlite3 >/dev/null 2>&1; then
    sqlite3 "$DB" "PRAGMA wal_checkpoint(TRUNCATE);" >/dev/null 2>&1 || true
fi

# ── Run rsync ────────────────────────────────────────────────────
# Uses only flags compatible with both GNU rsync and macOS openrsync.
rsync -rlt --delete \
    --exclude='.git/' \
    --exclude='*.download' \
    --exclude='catalog.db-wal' \
    --exclude='catalog.db-shm' \
    --stats \
    $DRY_RUN \
    "$ROOT/" "$DEST/"

echo ""
echo "Catalog backup complete."
