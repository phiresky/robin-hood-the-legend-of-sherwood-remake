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
cargo build --release --bin convert_datadir
target/release/convert_datadir \
    --input "$source_datadir" \
    --output "$output_dir" \
    --format shipping \
    --map-format jxl-q80 \
    --interface-image-format jxl-q80 \
    --rle-sprite-format jxl-q80 \
    --audio-format opus \
    --zstd-window-log 30 \
    "$mode"

manifest="$output_dir/Data/datadir.bin"
if [[ ! -s "$manifest" ]]; then
    echo "conversion did not produce $manifest" >&2
    exit 1
fi

echo "web shipping datadir ready: $manifest"
