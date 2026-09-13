#!/usr/bin/env bash
# Build, verify and deploy the Cloudflare static Workers from this checkout.
#
# Usage (from anywhere):
#   CLOUDFLARE_ACCOUNT_ID=... CLOUDFLARE_ZONE_ID=... CLOUDFLARE_API_TOKEN=... \
#   ROBINHOOD_WASM_BINDGEN=/abs/path/wasm-bindgen-0.2.127 \
#     wasm-www/scripts/deploy-cloudflare.sh [--datadir] [--runtime]
#
# --datadir  also deploy wasm-www/datadir-dist (assembled beforehand with
#            assemble-datadir-corpus.mjs); writes target/datadir-deployment.json
#            when target/datadir-authority.json exists.
# --runtime  also deploy wasm-www/runtime-dist (assembled beforehand with
#            assemble-runtime-corpus.mjs; verify:runtime needs
#            target/datadir-authority.json).
# Without a flag, ROBINHOOD_DATADIR_VERSION_ID / ROBINHOOD_RUNTIME_VERSION_ID must
# name the live Worker versions (`pnpm exec wrangler deployments list --config
# deploy/wrangler-runtime.json`); route reconciliation proves they exist.
#
# Rollback: `pnpm --dir wasm-www exec wrangler rollback --config
# deploy/wrangler-<public|signer|runtime|datadir>.json [VERSION_ID]`.
set -euo pipefail

deploy_datadir=0
deploy_runtime=0
for argument in "$@"; do
    case "$argument" in
        --datadir) deploy_datadir=1 ;;
        --runtime) deploy_runtime=1 ;;
        *) echo "usage: $0 [--datadir] [--runtime]" >&2; exit 2 ;;
    esac
done
: "${CLOUDFLARE_ACCOUNT_ID:?}" "${CLOUDFLARE_ZONE_ID:?}" "${CLOUDFLARE_API_TOKEN:?}" "${ROBINHOOD_WASM_BINDGEN:?}"

cd "$(dirname "$0")/.."
repository="$(cd .. && pwd -P)"
test "$(pnpm exec wrangler --version)" = "4.127.1"

# Deploy one Worker and print the version ID from Wrangler's NDJSON output.
deploy_worker() {
    local config="$1" worker="$2" output
    output="$(mktemp)"
    WRANGLER_OUTPUT_FILE_PATH="$output" pnpm exec wrangler deploy --config "deploy/wrangler-$config.json" >&2
    node -e '
        const [path, worker] = process.argv.slice(1);
        const events = require("node:fs").readFileSync(path, "utf8").split("\n")
            .filter(line => line.length > 0).map(line => JSON.parse(line))
            .filter(event => event.type === "deploy");
        if (events.length !== 1 || events[0].worker_name !== worker
            || !/^[0-9a-f]{8}-(?:[0-9a-f]{4}-){3}[0-9a-f]{12}$/u.test(events[0].version_id)) {
            throw new Error(`no single canonical deploy event for ${worker} in ${path}`);
        }
        console.log(events[0].version_id);
    ' "$output" "$worker"
    rm -f -- "$output"
}

pnpm verify:deployment-config
pnpm build:site
pnpm verify:public
pnpm build:signer
pnpm verify:signer
pnpm verify:wrangler

if [[ $deploy_datadir == 1 ]]; then
    pnpm verify:datadir
    pnpm verify:datadir-wrangler
    ROBINHOOD_DATADIR_VERSION_ID="$(deploy_worker datadir robinhood-datadir-assets)"
    export ROBINHOOD_DATADIR_VERSION_ID
    node scripts/sync-cloudflare-routes.mjs --prove-datadir
    if [[ -f "$repository/target/datadir-authority.json" ]]; then
        node scripts/datadir-release-authority.mjs receipt \
            "$repository/target/datadir-authority.json" "$ROBINHOOD_DATADIR_VERSION_ID" \
            "$repository/target/datadir-deployment.json"
    fi
fi
if [[ $deploy_runtime == 1 ]]; then
    pnpm verify:runtime
    pnpm verify:runtime-wrangler
    ROBINHOOD_RUNTIME_VERSION_ID="$(deploy_worker runtime robinhood-runtime-assets)"
    export ROBINHOOD_RUNTIME_VERSION_ID
fi
: "${ROBINHOOD_DATADIR_VERSION_ID:?pass --datadir or name the live datadir Worker version}"
: "${ROBINHOOD_RUNTIME_VERSION_ID:?pass --runtime or name the live runtime Worker version}"

deploy_worker signer robinhood-identity-signer
deploy_worker public robinhood-public-site
node scripts/sync-cloudflare-routes.mjs --apply
node scripts/sync-cloudflare-routes.mjs --check
pnpm smoke:cloudflare
echo "deployed; datadir=$ROBINHOOD_DATADIR_VERSION_ID runtime=$ROBINHOOD_RUNTIME_VERSION_ID"
