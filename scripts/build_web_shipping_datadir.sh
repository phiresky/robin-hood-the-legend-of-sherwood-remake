#!/usr/bin/env bash
set -euo pipefail

if [[ $# -lt 2 || $# -gt 3 ]]; then
    echo "usage: $0 <source-datadir> <output-dir> [--resume]" >&2
    exit 2
fi

source_datadir=$1
output_dir=$2
mode=${3:---force}
if [[ "$mode" != "--force" && "$mode" != "--resume" ]]; then
    echo "third argument must be --force or --resume" >&2
    exit 2
fi

# This is the canonical browser artifact recipe. Keep these explicit: the
# converter's source-preserving defaults are appropriate for native builds,
# but would silently produce the much larger raw-map/source-audio artifact.
# JXL is q80 across the board (maps, minimaps, interface images, and the
# RLE patch/ambient-animation sprite bucket — the latter is WEB ONLY: it
# breaks framebuffer parity, so native shipping keeps exact RLE).
# The RLE atlas encode shells out to `cjxl`, which must be on PATH.
#
# Opus audio must be encoded by libopus 1.6.1 (docs/COMPRESSION.md,
# 2026-09-14). The converter loads $libopus_dir/libopus.so.0, requires its
# version string, and checks from the loader trace that every ffmpeg process
# really used that file; any other libopus fails the conversion.
#
# Music encodes from the lossless remaster WAVs. The directory is passed
# explicitly: the converter used to look it up relative to the current
# directory, which silently fell back to the game files when run from a
# worktree. Which remaster belongs to which track comes from the tracked
# crates/robin_rs/src/bin/convert_datadir/lossless_music_mapping.json, selected
# by the sha256 of the datadir's music files; the drop's own mapping.json is
# superseded and ignored.
toolchain=${ROBIN_RELEASE_TOOLCHAIN:-$HOME/.local/share/robin_hood/deployment-toolchain}
libopus_dir=${ROBIN_LIBOPUS_DIR:-$toolchain/libopus-1.6.1/lib}
main_repo=$(dirname "$(git rev-parse --path-format=absolute --git-common-dir)")
lossless_music_dir=${ROBIN_LOSSLESS_MUSIC_DIR:-$main_repo/datadirs/music-rhmods-lossless}
if [[ ! -e "$libopus_dir/libopus.so.0" ]]; then
    echo "missing $libopus_dir/libopus.so.0 (build libopus 1.6.1 into the toolchain or set ROBIN_LIBOPUS_DIR)" >&2
    exit 1
fi
if [[ ! -d "$lossless_music_dir" ]]; then
    echo "missing lossless music WAV directory $lossless_music_dir (set ROBIN_LOSSLESS_MUSIC_DIR)" >&2
    exit 1
fi
cargo build --locked --release -p robin_rs --bin convert_datadir --features tools
target/release/convert_datadir \
    --input "$source_datadir" \
    --output "$output_dir" \
    --format shipping \
    --libopus-dir "$libopus_dir" \
    --lossless-music-dir "$lossless_music_dir" \
    --map-format jxl-q80 \
    --interface-image-format jxl-q80 \
    --rle-sprite-format jxl-q80 \
    --audio-format opus \
    --zstd-window-log 30 \
    --web-content-manifest \
    --web-content-edition demo \
    "$mode"

manifest="$output_dir/Data/datadir.bin"
if [[ ! -s "$manifest" ]]; then
    echo "conversion did not produce $manifest" >&2
    exit 1
fi

content_manifest="$output_dir/Data/robinhood-web-content.json"
if [[ ! -s "$content_manifest" ]]; then
    echo "conversion did not produce $content_manifest" >&2
    exit 1
fi

echo "web shipping datadir ready: $manifest ($content_manifest)"
