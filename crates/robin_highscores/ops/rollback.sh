#!/usr/bin/env bash
# Point `current` back at an older service release and restart.
#
# Usage: ops/rollback.sh [commit]   (default: newest other service release)
#
# Refuses when the live database schema is newer than the target supports;
# restore the matching backups/pre-deploy-*/highscores.sqlite3 snapshot first.
set -euo pipefail

root="${ROBIN_HIGHSCORES_ROOT:-$HOME/.local/opt/robin-highscores}"
server_config="${ROBIN_HIGHSCORES_SERVER_CONFIG:-$HOME/.config/robin-highscores/server.toml}"
health_url="${ROBIN_HIGHSCORES_HEALTH_URL:-http://127.0.0.1:8787/readyz}"
releases="$root/releases"

die() {
    echo "rollback: $*" >&2
    exit 1
}

current=$(realpath "$root/current")
authority=$(realpath "$root/authority")
if [[ $# -ge 1 ]]; then
    target="$releases/$1"
else
    target=""
    while IFS= read -r release; do
        release=$(realpath "$release")
        if [[ "$release" != "$current" && "$release" != "$authority" ]]; then
            target=$release
            break
        fi
    done < <(ls -1dt "$releases"/*/)
    [[ -n "$target" ]] || die "no other service release to roll back to"
fi
target=$(realpath "$target")
[[ "$target" != "$current" ]] || die "$(basename "$target") is already current"
[[ "$target" != "$authority" ]] || die "refusing to run services from the authority release"
[[ -x "$target/bin/robin-highscores-server" ]] || die "$target is not a service release"

database=$("$current/bin/robin-highscores-admin" --config "$server_config" database-schema-version)
supported=$("$target/bin/robin-highscores-admin" supported-schema-version) ||
    die "cannot determine the schema supported by $(basename "$target")"
if ((database != supported)); then
    die "database schema is $database but $(basename "$target") requires $supported; restore a matching backups/pre-deploy-*/highscores.sqlite3 snapshot first"
fi

systemctl --user stop robin-highscores-worker.service robin-highscores-api.service
ln -sfn "$target" "$root/current.new"
mv -T "$root/current.new" "$root/current"
systemctl --user daemon-reload
systemctl --user start robin-highscores.target
curl -fsS --retry 10 --retry-delay 2 --retry-connrefused "$health_url" >/dev/null
echo "rollback: $(basename "$target") is live"
