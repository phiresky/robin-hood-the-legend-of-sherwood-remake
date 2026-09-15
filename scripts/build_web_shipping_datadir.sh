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
#
# Opus audio is encoded by opus-tools' opusenc on libopus 1.6.1
# (docs/COMPRESSION.md, 2026-09-14); ffmpeg only decodes sources to PCM. The
# converter requires `$opus_tools_dir/bin/opusenc --version` to report libopus
# 1.6.1, checks that library's version string, and verifies from the loader
# trace that every opusenc process really used it.
#
# Music encodes from the lossless remaster WAVs. The directory is passed
# explicitly: the converter used to look it up relative to the current
# directory, which silently fell back to the game files when run from a
# worktree. Which remaster belongs to which track comes from the tracked
# crates/robin_rs/src/bin/convert_datadir/lossless_music_mapping.json, selected
# by the sha256 of the datadir's music files; the drop's own mapping.json is
# superseded and ignored.
toolchain=${ROBIN_RELEASE_TOOLCHAIN:-$HOME/.local/share/robin_hood/deployment-toolchain}
opus_tools_dir=${ROBIN_OPUS_TOOLS_DIR:-$toolchain/opus-tools-0.2}
main_repo=$(dirname "$(git rev-parse --path-format=absolute --git-common-dir)")
lossless_music_dir=${ROBIN_LOSSLESS_MUSIC_DIR:-$main_repo/datadirs/music-rhmods-lossless}
if [[ ! -x "$opus_tools_dir/bin/opusenc" ]]; then
    echo "missing $opus_tools_dir/bin/opusenc (build opus-tools 0.2 on libopus 1.6.1 into the toolchain or set ROBIN_OPUS_TOOLS_DIR)" >&2
    exit 1
fi
if ! "$opus_tools_dir/bin/opusenc" --version | grep -qF '(using libopus 1.6.1)'; then
    echo "$opus_tools_dir/bin/opusenc does not report libopus 1.6.1: $("$opus_tools_dir/bin/opusenc" --version | head -n 1)" >&2
    exit 1
fi
if [[ ! -d "$lossless_music_dir" ]]; then
    echo "missing lossless music WAV directory $lossless_music_dir (set ROBIN_LOSSLESS_MUSIC_DIR)" >&2
    exit 1
fi
# The converter runs `avifenc`/`avifdec` from PATH; prefer the toolchain's
# pinned static builds and refuse any other version (AVIF bytes depend on
# the exact libavif/libaom).
export PATH="$toolchain/bin:$PATH"
for avif_tool in avifenc avifdec; do
    if ! command -v "$avif_tool" >/dev/null; then
        echo "$avif_tool not on PATH (scripts/install_pinned_avif_tools.sh, then copy bin/$avif_tool into $toolchain/bin)" >&2
        exit 1
    fi
    if [[ $("$avif_tool" --version | head -n 1) != 'Version: 1.4.2 (aom [enc/dec]:3.15.0)' ]]; then
        echo "$avif_tool is not the pinned libavif 1.4.2 / libaom 3.15.0 build: $("$avif_tool" --version | head -n 1)" >&2
        exit 1
    fi
done
cargo build --locked --release -p robin_rs --bin convert_datadir --features tools
target/release/convert_datadir \
    --input "$source_datadir" \
    --output "$output_dir" \
    --format shipping \
    --opus-tools-dir "$opus_tools_dir" \
    --lossless-music-dir "$lossless_music_dir" \
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
