import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import test from 'node:test';
import { runtimeBuildPlan } from './build-runtime.mjs';

const repository = resolve(import.meta.dirname, '../..');

async function workflow(name) {
    return readFile(resolve(repository, '.github/workflows', name), 'utf8');
}

async function repositoryFile(path) {
    return readFile(resolve(repository, path), 'utf8');
}

function ordered(text, markers) {
    let previous = -1;
    for (const marker of markers) {
        const index = text.indexOf(marker);
        assert(index >= 0, `workflow is missing ${marker}`);
        assert(index > previous, `${marker} is out of release order`);
        previous = index;
    }
}

test('runtime deployment re-verifies the exact corpus before a route-free Worker deploy', async () => {
    const text = await workflow('deploy-static-runtime.yml');
    assert.match(text, /^\s*workflow_dispatch:/mu);
    assert.match(text, /name: cloudflare-production/u);
    ordered(text, [
        'verify-static-origin-inventory.mjs',
        'verify:runtime',
        'verify:runtime-wrangler',
        'wrangler deploy --config deploy/wrangler-runtime.json',
        'extract-wrangler-deploy-version.mjs',
        'sync-cloudflare-routes.mjs --prove-runtime',
    ]);
    assert.doesNotMatch(text, /sync-cloudflare-routes\.mjs --apply/u);
    assert.doesNotMatch(text, /wrangler pages|github-pages|fullgame|fullgame_shipping/iu);
});

test('runtime build prunes the private vault and authors the retained JavaScript closure', async () => {
    const text = await workflow('build-static-runtime.yml');
    ordered(text, [
        'node wasm-www/scripts/stage-runtime-addition.mjs',
        'write-static-origin-inventory.mjs',
        'actions/upload-artifact@',
    ]);
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

test('datadir deployment is separately approved, verified, deployed, and receipted', async () => {
    const text = await workflow('deploy-static-datadir.yml');
    assert.match(text, /^\s*workflow_dispatch:/mu);
    assert.match(text, /name: cloudflare-production/u);
    ordered(text, [
        'datadir-release-authority.mjs verify',
        'verify:datadir-wrangler',
        'wrangler deploy --config deploy/wrangler-datadir.json',
        'extract-wrangler-deploy-version.mjs',
        'sync-cloudflare-routes.mjs --prove-datadir',
        'datadir-release-authority.mjs receipt',
    ]);
    assert.doesNotMatch(text, /runtime-dist|wrangler-runtime|fullgame|fullgame_shipping/iu);
});

test('first static deployment proves API bypass and runtime existence before public routes', async () => {
    const text = await workflow('deploy-static-workers.yml');
    ordered(text, [
        'sync-cloudflare-routes.mjs --prepare-api',
        'sync-cloudflare-routes.mjs --prove-runtime',
        'sync-cloudflare-routes.mjs --prove-datadir',
        'wrangler deploy --config deploy/wrangler-signer.json',
        'wrangler deploy --config deploy/wrangler-public.json',
        'sync-cloudflare-routes.mjs --apply',
        'smoke:cloudflare',
    ]);
    assert.match(text, /build_identity_signer\.sh/u);
    assert.match(text, /wasm-bindgen 0\.2\.127/u);
    assert.doesNotMatch(text, /wrangler pages|github-pages/iu);
});

test('shipping datadir wrapper selects the locked converter package and tools feature', async () => {
    const text = await repositoryFile('scripts/build_web_shipping_datadir.sh');
    assert.match(
        text,
        /^cargo build --locked --release -p robin_rs --bin convert_datadir --features tools$/mu,
    );
    assert.doesNotMatch(text, /^cargo build --release --bin convert_datadir$/mu);
});
