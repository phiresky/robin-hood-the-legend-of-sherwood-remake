#!/usr/bin/env bash
# Build a leaderboard service release tarball:
#   robin-highscores-<commit>.tar.zst
#     robin-highscores-<commit>/{bin/,ops/,SOURCE_COMMIT,SHA256SUMS}
#
# or, with --with-verifier, a verifier release tarball:
#   robin-replay-verifier-<commit>.tar.zst
#     robin-replay-verifier-<commit>/{bin/robin-replay-verifier,SOURCE_COMMIT,SHA256SUMS}
#
# Usage: ops/build-release.sh [--with-verifier] [OUTPUT_DIR]
#
# The verifier must resimulate replays recorded by the live web runtime
# bit-identically, so build the verifier release at the commit of that runtime,
# not at the service commit. It is a fully static x86_64-unknown-linux-musl
# binary because the sandbox root contains nothing else. Install it as
# releases/<commit>/ and point the `authority` symlink at it (see README).
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
staging=$(mktemp -d)
trap 'rm -rf "$staging"' EXIT
mkdir -p "$output_dir"

if ((with_verifier)); then
    target=x86_64-unknown-linux-musl
    cargo build --locked --release --target "$target" -p robin_replay_verifier --bin robin-replay-verifier
    binary="target/$target/release/robin-replay-verifier"
    if readelf -l "$binary" | grep -q INTERP; then
        echo "error: $binary is dynamically linked; the verifier sandbox needs a static binary" >&2
        exit 1
    fi
    name="robin-replay-verifier-$commit"
    stage="$staging/$name"
    mkdir -p "$stage/bin"
    install -m 0555 "$binary" "$stage/bin/robin-replay-verifier"
    printf '%s\n' "$commit" >"$stage/SOURCE_COMMIT"
    (cd "$stage" && sha256sum bin/robin-replay-verifier SOURCE_COMMIT >SHA256SUMS)
    tar --zstd -cf "$output_dir/$name.tar.zst" -C "$staging" "$name"
    echo "$output_dir/$name.tar.zst"
    exit 0
fi

cargo build --locked --release -p robin_highscores --bins
name="robin-highscores-$commit"
stage="$staging/$name"
mkdir -p "$stage/bin" "$stage/ops/systemd"
for binary in robin-highscores-server robin-highscores-worker robin-highscores-admin; do
    install -m 0555 "target/release/$binary" "$stage/bin/$binary"
done
ops="crates/robin_highscores/ops"
install -m 0555 "$ops/deploy.sh" "$ops/rollback.sh" "$ops/backup.sh" "$stage/ops/"
install -m 0444 "$ops"/systemd/* "$stage/ops/systemd/"
printf '%s\n' "$commit" >"$stage/SOURCE_COMMIT"
(cd "$stage" && find bin ops SOURCE_COMMIT -type f -print0 | sort -z | xargs -0 sha256sum >SHA256SUMS)

tar --zstd -cf "$output_dir/$name.tar.zst" -C "$staging" "$name"
echo "$output_dir/$name.tar.zst"
