#!/usr/bin/env bash
# Threaded (atomics + shared memory) browser build.
#
# Produces the same robin.js/robin_bg.wasm layout as the plain wasm-release
# pipeline (README "wasm" section), but with the rayon worker pool enabled
# for parallel VQ sprite decode:
#   - rustflags from scripts/wasm-threads.cargo-config.toml are APPENDED to
#     the base .cargo/config.toml wasm flags via `cargo --config` (config
#     arrays merge), so the plain build stays byte-identical;
#   - `-Zbuild-std` recompiles std with atomics (already required by the
#     plain wasm-release pipeline for size reasons);
#   - the `wasm-threads` cargo feature switches the mission-install decode
#     path to the worker pool (serial fallback stays compiled in for
#     non-cross-origin-isolated pages).
#
# usage: scripts/build-wasm-threads.sh [--bench] [--out-dir DIR] [--no-opt]
#   --bench    build the robin_assets wasm_decode_bench example instead of
#              the robin binary (out-dir defaults to wasm-www/pkg-bench)
#   --no-opt   skip the wasm-opt/wasm-strip post-pass (faster iteration;
#              the deployed pipeline always runs it)
set -euo pipefail
cd "$(dirname "$0")/.."

bench=0
run_opt=1
out_dir=""
while [ $# -gt 0 ]; do
    case "$1" in
        --bench) bench=1 ;;
        --no-opt) run_opt=0 ;;
        --out-dir)
            shift
            out_dir="$1"
            ;;
        *)
            echo "unknown argument: $1" >&2
            exit 2
            ;;
    esac
    shift
done

if [ "$bench" = 1 ]; then
    out_dir="${out_dir:-wasm-www/pkg-bench}"
    build_args=(-p robin_assets --features wasm-threads --example wasm_decode_bench)
    wasm=target/wasm32-unknown-unknown/wasm-release/examples/wasm_decode_bench.wasm
else
    out_dir="${out_dir:-wasm-www/pkg}"
    build_args=(--no-default-features --features audio,wasm-threads -p robin_rs --bin robin)
    wasm=target/wasm32-unknown-unknown/wasm-release/robin.wasm
fi

cargo build \
    --config scripts/wasm-threads.cargo-config.toml \
    -Zbuild-std=std,panic_abort \
    --target wasm32-unknown-unknown \
    --profile wasm-release \
    "${build_args[@]}"

# A module without a shared memory import means the atomics rustflags were
# lost (e.g. a plain RUSTFLAGS env var replaced the merged config arrays) —
# everything would still link and run, just silently single-threaded.
# No `grep -q`: under pipefail an early-exiting grep SIGPIPEs wasm-objdump
# and turns a successful match into a spurious failure.
if ! wasm-objdump -x -j Import "$wasm" | grep -E "memory\[[0-9]+\].* shared " >/dev/null; then
    echo "error: $wasm has no shared memory import; atomics flags were dropped" >&2
    exit 1
fi

bindgen_args=(--target web --out-dir "$out_dir")
if [ "$bench" = 0 ]; then
    # Match the deployed artifact names (`robin.js` / `robin_bg.wasm`).
    bindgen_args+=(--out-name robin)
fi
wasm-bindgen "${bindgen_args[@]}" "$wasm"
if [ "$run_opt" = 1 ]; then
    node wasm-www/scripts/optimize-wasm.mjs "$out_dir"
fi
echo "threaded wasm build ready in $out_dir"
