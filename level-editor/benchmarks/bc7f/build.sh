#!/usr/bin/env bash
set -euo pipefail
bench_dir="$(cd -- "$(dirname -- "$0")" && pwd)"
work_dir="$bench_dir/../../work/bc7f-benchmark"
basis_dir="$work_dir/basis_universal"
revision=99f52d63aa6799cbdaecfe977111dc5ec3b31d47
if [[ "$(git -C "$basis_dir" rev-parse HEAD)" != "$revision" ]]; then
  echo "Expected Basis revision $revision" >&2
  exit 1
fi
out_dir="$work_dir"
module_flags=()
if [[ "${BC7F_APP:-0}" == 1 ]]; then
  out_dir="$bench_dir/../../app/src/vendor/bc7f"
  mkdir -p "$out_dir"
  module_flags=(-sEXPORT_ES6=1)
fi
simd_flags=()
if [[ "${BC7F_SIMD:-0}" == 1 ]]; then simd_flags=(-msimd128); fi
"${EMXX:-em++}" "${simd_flags[@]}" "${module_flags[@]}" -O3 -g3 -flto -fno-strict-aliasing -DNDEBUG -DBASISD_SUPPORT_KTX2=0 -DBASISD_SUPPORT_KTX2_ZSTD=0 \
  -I"$basis_dir/transcoder" "$bench_dir/bc7f.cpp" "$basis_dir/transcoder/basisu_transcoder.cpp" \
  -sALLOW_MEMORY_GROWTH=1 -sINITIAL_MEMORY=16777216 -sMAXIMUM_MEMORY=536870912 \
  -sMODULARIZE=1 -sEXPORT_NAME=BC7F -sENVIRONMENT=worker -sFILESYSTEM=0 \
  '-sEXPORTED_FUNCTIONS=["_initialize","_encode","_mip","_malloc","_free"]' \
  '-sEXPORTED_RUNTIME_METHODS=["HEAPU8"]' -o "$out_dir/bc7f.js"

wasm-strip "$out_dir/bc7f.wasm"
