#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 1 ]]; then
    echo "usage: $0 ABSOLUTE_WASM_BINDGEN_0_2_127" >&2
    exit 2
fi

wasm_bindgen="$1"
if [[ "$wasm_bindgen" != /* || ! -f "$wasm_bindgen" || -L "$wasm_bindgen" || ! -x "$wasm_bindgen" ]]; then
    echo "wasm-bindgen must be an absolute regular executable: $wasm_bindgen" >&2
    exit 2
fi
if [[ "$($wasm_bindgen --version)" != 'wasm-bindgen 0.2.127' ]]; then
    echo "identity signer requires the exact accepted wasm-bindgen 0.2.127 CLI" >&2
    exit 1
fi

script_directory="$(cd "$(dirname "$0")" && pwd -P)"
repository="$(cd "$script_directory/.." && pwd -P)"
generated="$(mktemp -d)"
trap 'rm -rf -- "$generated"' EXIT

cd "$repository/wasm-www"
VITE_GAME_ORIGIN='https://robinhood.phiresky.xyz' pnpm build:signer-shell

cd "$repository"
RUSTC_WRAPPER='' \
CARGO_INCREMENTAL=0 \
ROBINHOOD_LEADERBOARD_WEB_ORIGIN='https://robinhood.phiresky.xyz' \
ROBINHOOD_IDENTITY_SIGNER_ORIGIN='https://identity.robinhood.phiresky.xyz' \
cargo build \
    -Zbuild-std=std,panic_abort \
    --locked \
    --target wasm32-unknown-unknown \
    --profile wasm-release \
    --no-default-features \
    --features identity-signer-bridge \
    -p robin_identity_signer \
    --bin leaderboard_identity_bridge

"$wasm_bindgen" \
    --target web \
    --no-typescript \
    --out-name leaderboard_identity_bridge \
    --out-dir "$generated" \
    "$repository/target/wasm32-unknown-unknown/wasm-release/leaderboard_identity_bridge.wasm"

node "$repository/wasm-www/scripts/stage-browser-identity-origin.mjs" \
    identity_signer "$generated" leaderboard_identity_bridge.js
node "$repository/wasm-www/scripts/verify-identity-signer-bridge.mjs" "$generated"

bridge_destination="$repository/wasm-www/signer-dist/identity-signer/bridge"
if [[ -e "$bridge_destination" ]]; then
    echo "Vite signer shell unexpectedly emitted the generated bridge path" >&2
    exit 1
fi
mkdir -p "$bridge_destination"
cp -R "$generated"/. "$bridge_destination"/

cd "$repository/wasm-www"
pnpm verify:signer
