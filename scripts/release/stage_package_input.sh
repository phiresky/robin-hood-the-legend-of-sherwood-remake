#!/usr/bin/env bash
set -euo pipefail

runtime=$1
target=$2
executable=$3
pack_executable=$4

mkdir -p target/package-input target/release-assets
cp \
  "target/${target}/release/${executable}" \
  "target/package-input/${pack_executable}"
cp README.md target/package-input/
mkdir -p target/package-input/docs
cp docs/MODDING_TOOLS.md docs/JSON_PATCH_MODS.md target/package-input/docs/
suffix=""
if [[ "${runtime}" == win-x64 ]]; then suffix=".exe"; fi
cp "target/${target}/release/robin-replay-admission${suffix}" target/package-input/
for tool in cpf_to_json encode_mod_sprites disasm_scb dump_res; do
  cp "target/${target}/release/${tool}${suffix}" target/package-input/
done
# Engine-shipped overlay datadirs, resolved relative to the
# executable at runtime: the core overlay (bitmap fonts, UI
# icons) is required; mods/ (e.g. hackable levels) ships when
# the checkout carries any.
mkdir -p target/package-input/assets
cp -R assets/core-datadir target/package-input/assets/
if [ -d mods ]; then
  cp -R mods target/package-input/
fi
