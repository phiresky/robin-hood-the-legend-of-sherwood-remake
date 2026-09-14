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
# Images are browser-decoded AVIF q60 across the board (maps, keyed minimaps,
# interface images, and the RLE patch/ambient-animation sprite bucket — the
# latter is WEB ONLY: it breaks framebuffer parity, so native shipping keeps
# exact RLE). The encodes shell out to the pinned `avifenc`/`avifdec`
# (libavif 1.4.2 on libaom 3.15.0), which must be on PATH; see
# scripts/install_pinned_avif_tools.sh.
cargo build --locked --release -p robin_rs --bin convert_datadir --features tools
target/release/convert_datadir \
    --input "$source_datadir" \
    --output "$output_dir" \
    --format shipping \
    --map-format avif-q60 \
    --interface-image-format avif-q60 \
    --rle-sprite-format avif-q60 \
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
