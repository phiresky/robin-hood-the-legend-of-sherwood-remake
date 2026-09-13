#!/usr/bin/env bash
# Local leaderboard backup: consistent DB snapshot, hard-linked content-addressed
# replay/campaign objects, API secrets and config. Keeps the newest 7.
#
# The DB snapshot is taken first while services keep running, so the copied
# object trees can be slightly ahead of it (orphans are reconciled and GC'd on
# startup) or miss objects purged in between (purge claims resume on startup).
#
# Off-host copy (run from the other machine):
#   rsync -aH --delete robinhood@vps:.local/share/robin-highscores/backups/ ./robin-highscores-backups/
set -euo pipefail

root="${ROBIN_HIGHSCORES_ROOT:-$HOME/.local/opt/robin-highscores}"
config_dir="${ROBIN_HIGHSCORES_CONFIG_DIR:-$HOME/.config/robin-highscores}"
state="${ROBIN_HIGHSCORES_STATE:-$HOME/.local/share/robin-highscores}"
keep=7

backups="$state/backups"
stamp=$(date -u +%Y%m%dT%H%M%SZ)
partial="$backups/.partial-$stamp"
mkdir -p "$partial"
trap 'rm -rf "$partial"' EXIT

"$root/current/bin/robin-highscores-admin" --config "$config_dir/server.toml" \
    snapshot-db "$partial/highscores.sqlite3"
cp -al "$state/replays" "$partial/replays"
cp -al "$state/campaign-states" "$partial/campaign-states"
cp -a "$state/api-secrets" "$partial/api-secrets"
cp -a "$config_dir" "$partial/config"
mv -T "$partial" "$backups/$stamp"
trap - EXIT
echo "backup: $backups/$stamp"

ls -1d "$backups"/[0-9]*Z | sort -r | tail -n +$((keep + 1)) | while IFS= read -r old; do
    echo "backup: pruning $(basename "$old")"
    rm -rf "$old"
done
