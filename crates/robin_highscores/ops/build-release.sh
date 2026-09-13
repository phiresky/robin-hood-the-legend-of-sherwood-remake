#!/usr/bin/env bash
# Build a leaderboard service release tarball:
#   robin-highscores-<commit>.tar.zst
#     robin-highscores-<commit>/{bin/,ops/,SOURCE_COMMIT,SHA256SUMS}
#
# Usage: ops/build-release.sh [--with-verifier] [OUTPUT_DIR]
#
# Routine service releases ship only server/worker/admin. The verifier binary
# is pinned by sha256 in every ranked BuildManifestV2 and the worker refuses to
# start if it changes, so `--with-verifier` is only for deliberately authoring a
# new authority release; deploy.sh never points the worker at it.
set -euo pipefail

with_verifier=0
if [[ "${1:-}" == "--with-verifier" ]]; then
    with_verifier=1
    shift
fi
output_dir=$(realpath "${1:-.}")

repo=$(git -C "$(dirname "$0")" rev-parse --show-toplevel)
commit=$(git -C "$repo" rev-parse HEAD)
if [[ -n "$(git -C "$repo" status --porcelain --untracked-files=no)" ]]; then
    echo "warning: working tree has uncommitted changes; SOURCE_COMMIT=$commit is not exact" >&2
fi

cd "$repo"
cargo build --locked --release -p robin_highscores --bins
if ((with_verifier)); then
    cargo build --locked --release -p robin_replay_verifier --bin robin-replay-verifier
fi

name="robin-highscores-$commit"
staging=$(mktemp -d)
trap 'rm -rf "$staging"' EXIT
stage="$staging/$name"
mkdir -p "$stage/bin" "$stage/ops/systemd"
for binary in robin-highscores-server robin-highscores-worker robin-highscores-admin; do
    install -m 0555 "target/release/$binary" "$stage/bin/$binary"
done
if ((with_verifier)); then
    install -m 0555 target/release/robin-replay-verifier "$stage/bin/robin-replay-verifier"
fi
ops="crates/robin_highscores/ops"
install -m 0555 "$ops/deploy.sh" "$ops/rollback.sh" "$ops/backup.sh" "$stage/ops/"
install -m 0444 "$ops"/systemd/* "$stage/ops/systemd/"
printf '%s\n' "$commit" >"$stage/SOURCE_COMMIT"
(cd "$stage" && find bin ops SOURCE_COMMIT -type f -print0 | sort -z | xargs -0 sha256sum >SHA256SUMS)

mkdir -p "$output_dir"
tar --zstd -cf "$output_dir/$name.tar.zst" -C "$staging" "$name"
echo "$output_dir/$name.tar.zst"
