#!/usr/bin/env bash
# Deploy a service release tarball built by ops/build-release.sh.
#
# Usage: ops/deploy.sh robin-highscores-<commit>.tar.zst
#
# Steps: verify + unpack into releases/<commit>; stop services; snapshot the DB
# into backups/pre-deploy-<commit>-<time>/; migrate; swap `current`; start;
# wait for /readyz; keep the newest 3 service releases.
#
# On failure after services were stopped:
#   * no migration ran  -> `current` is restored and services restarted;
#   * a migration ran   -> `current` is restored but services stay stopped,
#     because the previous binaries reject the newer schema; restore the
#     printed snapshot first.
# The `authority` symlink names the verifier release (bin/robin-replay-verifier,
# built at the live web runtime commit; worker.toml launches it through the
# symlink). This script never modifies or prunes it, and refuses a service
# release whose commit directory is the verifier release.
set -euo pipefail

root="${ROBIN_HIGHSCORES_ROOT:-$HOME/.local/opt/robin-highscores}"
server_config="${ROBIN_HIGHSCORES_SERVER_CONFIG:-$HOME/.config/robin-highscores/server.toml}"
state="${ROBIN_HIGHSCORES_STATE:-$HOME/.local/share/robin-highscores}"
health_url="${ROBIN_HIGHSCORES_HEALTH_URL:-http://127.0.0.1:8787/readyz}"
keep_releases=3
releases="$root/releases"

die() {
    echo "deploy: $*" >&2
    exit 1
}

[[ $# -eq 1 ]] || die "usage: deploy.sh <release tarball>"
tarball=$(realpath "$1")
[[ -L "$root/authority" && -d "$root/authority/" ]] || die "$root/authority must be a symlink to the verifier release"
authority=$(realpath "$root/authority")
[[ -x "$authority/bin/robin-replay-verifier" ]] || die "verifier release $authority has no executable bin/robin-replay-verifier"
previous=""
if [[ -L "$root/current" ]]; then
    previous=$(realpath "$root/current")
fi

# --- unpack and verify ---------------------------------------------------------
incoming="$releases/.incoming.$$"
rm -rf "$incoming"
mkdir -p "$incoming"
trap 'rm -rf "$incoming"' EXIT
tar -xf "$tarball" -C "$incoming"
shopt -s nullglob
unpacked=("$incoming"/robin-highscores-*/)
shopt -u nullglob
[[ ${#unpacked[@]} -eq 1 ]] || die "tarball must contain exactly one robin-highscores-<commit>/ directory"
unpacked=${unpacked[0]%/}
commit=$(<"$unpacked/SOURCE_COMMIT")
[[ "$commit" =~ ^[0-9a-f]{7,40}$ ]] || die "invalid SOURCE_COMMIT '$commit'"
[[ "$(basename "$unpacked")" == "robin-highscores-$commit" ]] || die "directory name does not match SOURCE_COMMIT"
(cd "$unpacked" && sha256sum --quiet -c SHA256SUMS) || die "SHA256SUMS verification failed"
for binary in robin-highscores-server robin-highscores-worker robin-highscores-admin; do
    [[ -x "$unpacked/bin/$binary" ]] || die "release is missing bin/$binary"
done

target="$releases/$commit"
if [[ -e "$target" ]]; then
    resolved=$(realpath "$target")
    [[ "$resolved" != "$authority" ]] || die "$commit is the verifier release named by authority; refusing to replace it"
    [[ "$resolved" != "$previous" ]] || die "$commit is already current"
    rm -rf "$target"
fi
mv "$unpacked" "$target"
touch "$target"
rm -rf "$incoming"
trap - EXIT

admin="$target/bin/robin-highscores-admin"

# --- cutover -------------------------------------------------------------------
services_stopped=0
swapped=0
migrated=0
snapshot=""
schema_before=""
schema_after=""

swap_current() {
    ln -sfn "$1" "$root/current.new"
    mv -T "$root/current.new" "$root/current"
}

on_exit() {
    local status=$?
    trap - EXIT
    if ((status == 0)); then
        return
    fi
    echo "deploy: FAILED (exit $status)" >&2
    if ((services_stopped == 0)); then
        exit "$status"
    fi
    if ((swapped)) && [[ -n "$previous" ]]; then
        swap_current "$previous"
        echo "deploy: restored current -> $previous" >&2
    fi
    if ((migrated)); then
        systemctl --user stop robin-highscores-worker.service robin-highscores-api.service || true
        cat >&2 <<EOF
deploy: the database was migrated from schema $schema_before to $schema_after;
deploy: the previous release cannot run on it. Services are left STOPPED.
deploy: To recover:
deploy:   1. copy $snapshot over database_path from $server_config
deploy:      (and delete its -wal/-shm files)
deploy:   2. systemctl --user start robin-highscores.target
EOF
    elif [[ -n "$previous" ]]; then
        systemctl --user daemon-reload || true
        if systemctl --user start robin-highscores.target; then
            echo "deploy: previous release restarted" >&2
        else
            echo "deploy: previous release failed to restart; inspect journalctl --user" >&2
        fi
    else
        echo "deploy: no previous release; services left stopped" >&2
    fi
    exit "$status"
}
trap on_exit EXIT

systemctl --user stop robin-highscores-worker.service robin-highscores-api.service
services_stopped=1

mkdir -p "$state/backups"
snapshot_dir="$state/backups/pre-deploy-$commit-$(date -u +%Y%m%dT%H%M%S.%NZ)"
mkdir "$snapshot_dir"
snapshot="$snapshot_dir/highscores.sqlite3"
"$admin" --config "$server_config" snapshot-db "$snapshot"

schema_before=$("$admin" --config "$server_config" database-schema-version)
if ! "$admin" --config "$server_config" migrate; then
    migrated=1
    schema_after="(failed migration)"
    exit 1
fi
schema_after=$("$admin" --config "$server_config" database-schema-version)
if [[ "$schema_after" != "$schema_before" ]]; then
    migrated=1
fi

swap_current "$target"
swapped=1
systemctl --user daemon-reload
systemctl --user start robin-highscores.target
curl -fsS --retry 10 --retry-delay 2 --retry-connrefused "$health_url" >/dev/null

trap - EXIT
echo "deploy: $commit is live (schema $schema_before -> $schema_after; snapshot $snapshot)"

# --- prune old service releases --------------------------------------------------
kept=0
current=$(realpath "$root/current")
while IFS= read -r release; do
    release=${release%/}
    resolved=$(realpath "$release")
    if [[ "$resolved" == "$authority" || "$resolved" == "$current" ]]; then
        continue
    fi
    kept=$((kept + 1))
    # `current` counts as one of the kept releases.
    if ((kept >= keep_releases)); then
        echo "deploy: pruning $(basename "$release")"
        rm -rf "$release"
    fi
done < <(ls -1dt "$releases"/*/)

# Pre-deploy snapshots: keep the newest 5.
ls -1dt "$state"/backups/pre-deploy-*/ | tail -n +6 | while IFS= read -r old; do
    echo "deploy: pruning snapshot $(basename "$old")"
    rm -rf "$old"
done
