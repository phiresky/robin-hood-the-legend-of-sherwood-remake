#!/usr/bin/env bash
set -euo pipefail

# Populate vendor/slang-shaders/ with the libretro slang-shaders collection used
# by the optional `retroarch-shaders` robin_rs feature (desktop preset
# discovery, and the `#reference` in assets/shader_presets/*.slangp).
#
# The collection used to be committed to this repository. The pinned commit is
# the upstream revision whose root tree is byte-identical to that former
# vendored copy (git tree 373be088afc2d70a948c6c77f7f4ff39974ebccb), so fetching
# it restores exactly the same files.
#
# Usage: scripts/fetch-slang-shaders.sh [--force]
#   --force   replace an existing vendor/slang-shaders/ directory

readonly repo_url="https://github.com/libretro/slang-shaders.git"
readonly pinned_commit="3b0d6aa1d134a168478cd9c904a866d969f8882b"
readonly pinned_tree="373be088afc2d70a948c6c77f7f4ff39974ebccb"

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
dest="$root/vendor/slang-shaders"
force=0

case "${1:-}" in
    "") ;;
    --force) force=1 ;;
    *)
        printf 'usage: %s [--force]\n' "$0" >&2
        exit 2
        ;;
esac

if [[ -e "$dest" ]]; then
    if [[ "$force" != 1 ]]; then
        printf 'error: %s already exists; pass --force to replace it\n' "$dest" >&2
        exit 1
    fi
fi

staging="$(mktemp -d "$root/vendor/.slang-shaders.XXXXXX")"
trap 'rm -rf -- "$staging"' EXIT

git -C "$staging" init -q
git -C "$staging" remote add origin "$repo_url"
git -C "$staging" fetch -q --depth 1 origin "$pinned_commit"
git -C "$staging" checkout -q --detach FETCH_HEAD

actual_tree="$(git -C "$staging" rev-parse 'HEAD^{tree}')"
if [[ "$actual_tree" != "$pinned_tree" ]]; then
    printf 'error: fetched tree %s does not match pinned tree %s\n' \
        "$actual_tree" "$pinned_tree" >&2
    exit 1
fi

rm -rf -- "$staging/.git"
if [[ -e "$dest" ]]; then
    rm -rf -- "$dest"
fi
mv -- "$staging" "$dest"
trap - EXIT
printf 'slang-shaders %s installed at %s\n' "$pinned_commit" "$dest"
