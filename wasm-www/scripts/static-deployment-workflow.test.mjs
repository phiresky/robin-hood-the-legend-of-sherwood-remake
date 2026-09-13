import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import test from 'node:test';
import { runtimeBuildPlan } from './build-runtime.mjs';

const repository = resolve(import.meta.dirname, '../..');

async function repositoryFile(path) {
    return readFile(resolve(repository, path), 'utf8');
}

function ordered(text, markers) {
    let previous = -1;
    for (const marker of markers) {
        const index = text.indexOf(marker);
        assert(index >= 0, `missing ${marker}`);
        assert(index > previous, `${marker} is out of order`);
        previous = index;
    }
}

test('Cloudflare deploy verifies builds, deploys signer before public, then reconciles routes', async () => {
    const script = await repositoryFile('wasm-www/scripts/deploy-cloudflare.sh');
    ordered(script, [
        'pnpm verify:public',
        'pnpm verify:signer',
        'deploy_worker signer robinhood-identity-signer',
        'deploy_worker public robinhood-public-site',
        'sync-cloudflare-routes.mjs --apply',
        'sync-cloudflare-routes.mjs --check',
        'pnpm smoke:cloudflare',
    ]);
    const workflow = await repositoryFile('.github/workflows/deploy-static-workers.yml');
    assert.match(workflow, /^\s*workflow_dispatch:/mu);
    assert.match(workflow, /run: wasm-www\/scripts\/deploy-cloudflare\.sh$/mu);
    for (const text of [workflow, await repositoryFile('.github/workflows/build-static-runtime.yml')]) {
        assert.match(text, /scripts\/install_pinned_wasm_bindgen\.sh/u);
        assert.match(text, /wasm-bindgen 0\.2\.128/u);
        assert.doesNotMatch(text, /cargo install wasm-bindgen-cli|wrangler pages|github-pages/iu);
    }
});

test('runtime build prunes the private vault and authors the retained JavaScript closure', async () => {
    const text = await repositoryFile('.github/workflows/build-static-runtime.yml');
    ordered(text, ['node wasm-www/scripts/stage-runtime-addition.mjs', 'actions/upload-artifact@']);
    const plan = runtimeBuildPlan();
    const nameIndex = plan.bindgenArgs.indexOf('--out-name');
    assert(nameIndex >= 0);
    assert.equal(plan.bindgenArgs[nameIndex + 1], 'robin');
    const staging = await repositoryFile('wasm-www/scripts/stage-runtime-addition.mjs');
    ordered(staging, [
        "['--check', 'wasm-www/runtime-contract.json']",
        'buildRuntime({ outDir: artifact',
        'stage-browser-identity-origin.mjs',
        "run('gzip', ['-9', '-n', '-k'",
        'runtime-javascript-modules.mjs',
        'javascriptModules, sha256:',
        'wasm-www/scripts/verify-runtime-corpus.mjs',
    ]);
    assert(staging.includes("'engine', artifact, 'robin.js'"));
    assert(!staging.includes('browser_identity_vault.js'));
});

test('shipping datadir wrapper selects the locked converter package and tools feature', async () => {
    const text = await repositoryFile('scripts/build_web_shipping_datadir.sh');
    assert.match(
        text,
        /^cargo build --locked --release -p robin_rs --bin convert_datadir --features tools$/mu,
    );
    assert.doesNotMatch(text, /^cargo build --release --bin convert_datadir$/mu);
});
