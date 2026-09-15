#!/usr/bin/env bash
# Build the pinned AVIF encoder toolchain used by the web datadir recipe:
# static avifenc/avifdec from libavif v1.4.2 on libaom v3.15.0.
#
# Neither project publishes static Linux release binaries, so this builds
# from exact upstream commits. libavif's own fetched dependencies (libyuv,
# libpng, zlib, libjpeg-turbo, libargparse) are pinned by tag/commit inside
# that libavif commit's cmake modules, so the build inputs are fixed by the
# two commits below. The resulting binaries link only libc/libm.
#
#   scripts/install_pinned_avif_tools.sh ABSENT_ABSOLUTE_DESTINATION
#
# Prints the directory containing avifenc and avifdec. release.sh expects
# both on PATH (copy them into the deployment toolchain's bin/).
# Requires: git, cmake >= 3.22, ninja, nasm, a C/C++ compiler, network.
set -euo pipefail

if [[ $# -ne 1 ]]; then
    echo "usage: $0 ABSENT_ABSOLUTE_DESTINATION" >&2
    exit 2
fi

destination="$1"
if [[ "$destination" != /* || -e "$destination" ]]; then
    echo "destination must be an absent absolute path: $destination" >&2
    exit 2
fi

aom_url='https://aomedia.googlesource.com/aom'
aom_tag='v3.15.0'
aom_commit='de4c1d1edc49723a78954d30a83690aa1937422f'
libavif_url='https://github.com/AOMediaCodec/libavif'
libavif_tag='v1.4.2'
libavif_commit='c5240fc79fe5c2407e10afd35f5505ef6333ea49'
expected_version='Version: 1.4.2 (aom [enc/dec]:3.15.0)'

for tool in git cmake ninja nasm cc c++; do
    command -v "$tool" >/dev/null || { echo "missing build tool: $tool" >&2; exit 1; }
done

mkdir -p "$destination/src"
jobs="$(nproc)"

checkout() {
    local url="$1" tag="$2" commit="$3" dir="$4"
    git clone --quiet --depth 1 --branch "$tag" "$url" "$dir"
    local actual
    actual="$(git -C "$dir" rev-parse HEAD)"
    if [[ "$actual" != "$commit" ]]; then
        echo "$url $tag resolved to $actual, expected $commit" >&2
        exit 1
    fi
}

checkout "$aom_url" "$aom_tag" "$aom_commit" "$destination/src/aom"
checkout "$libavif_url" "$libavif_tag" "$libavif_commit" "$destination/src/libavif"

prefix="$destination/prefix"
cmake -S "$destination/src/aom" -B "$destination/build-aom" -G Ninja \
    -DCMAKE_BUILD_TYPE=Release \
    -DCMAKE_INSTALL_PREFIX="$prefix" \
    -DBUILD_SHARED_LIBS=0 \
    -DENABLE_DOCS=0 -DENABLE_EXAMPLES=0 -DENABLE_TESTS=0 -DENABLE_TOOLS=0 \
    -DCONFIG_AV1_DECODER=1 -DCONFIG_AV1_ENCODER=1
ninja -C "$destination/build-aom" -j "$jobs"
ninja -C "$destination/build-aom" install

PKG_CONFIG_PATH="$prefix/lib/pkgconfig" cmake -S "$destination/src/libavif" -B "$destination/build-avif" -G Ninja \
    -DCMAKE_BUILD_TYPE=Release \
    -DCMAKE_INSTALL_PREFIX="$prefix" \
    -DCMAKE_PREFIX_PATH="$prefix" \
    -DBUILD_SHARED_LIBS=OFF \
    -DAVIF_BUILD_APPS=ON \
    -DAVIF_CODEC_AOM=SYSTEM -DAVIF_CODEC_AOM_DECODE=ON -DAVIF_CODEC_AOM_ENCODE=ON \
    -DAVIF_LIBYUV=LOCAL -DAVIF_JPEG=LOCAL -DAVIF_ZLIBPNG=LOCAL \
    -DAVIF_LIBSHARPYUV=OFF -DAVIF_LIBXML2=OFF
ninja -C "$destination/build-avif" -j "$jobs"
ninja -C "$destination/build-avif" install

mkdir -p "$destination/bin"
for name in avifenc avifdec; do
    install -m 0755 "$prefix/bin/$name" "$destination/bin/$name"
    executable="$destination/bin/$name"
    if [[ ! -f "$executable" || -L "$executable" || ! -x "$executable" ]]; then
        echo "build did not produce an exact regular executable: $executable" >&2
        exit 1
    fi
    if [[ "$("$executable" --version | head -n 1)" != "$expected_version" ]]; then
        echo "$name reported an unexpected version: $("$executable" --version | head -n 1)" >&2
        exit 1
    fi
done

printf '%s\n' "$destination/bin"
