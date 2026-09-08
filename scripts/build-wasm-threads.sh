#!/usr/bin/env bash
# Compatibility wrapper; the shared local/CI build command owns Cargo,
# threading validation, bindgen naming and optimization.
set -euo pipefail
cd "$(dirname "$0")/.."
exec node wasm-www/scripts/build-runtime.mjs --threads "$@"
