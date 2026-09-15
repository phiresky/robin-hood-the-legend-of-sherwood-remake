import assert from 'node:assert/strict';
import test from 'node:test';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { runtimeBuildPlan } from './build-runtime.mjs';
import { CURRENT_DEMO_URL } from './datadir-release.mjs';
import {
    RUNTIME_ADDITION_BUILD, readRuntimeContract, stageRuntimeAddition, validateFullManifestSha256,
} from './stage-runtime-addition.mjs';

const release = Object.freeze({
    schema: 1, url: CURRENT_DEMO_URL, sha256: 'a'.repeat(64), byte_length: 26214400, native_content_sha256: 'b'.repeat(64),
});

test('production runtime additions are the threaded game build', () => {
    assert.equal(RUNTIME_ADDITION_BUILD.threads, true);
    assert.equal(RUNTIME_ADDITION_BUILD.requireIdentity, true);
    const plan = runtimeBuildPlan({ outDir: '/tmp/addition', ...RUNTIME_ADDITION_BUILD });
    assert(plan.cargo.includes('scripts/wasm-threads.cargo-config.toml'));
    assert(plan.cargo.includes('audio,wasm-threads'));
    assert(plan.cargo.includes('robin'));
    assert.equal(plan.optimize, null);
});

test('staging reads the compatibility numbers from the checked-in runtime contract', async () => {
    const contract = await readRuntimeContract();
    for (const key of ['netProtocol', 'ticketSchema', 'contentSchema']) assert(Number.isSafeInteger(contract[key]));
    assert.equal(validateFullManifestSha256(''), null);
    assert.throws(() => validateFullManifestSha256('bad'), /Full manifest/u);
});

test('staging rejects a missing or invalid datadir release before creating an artifact', async () => {
    const root = await mkdtemp(join(tmpdir(), 'robin-runtime-stage-'));
    try {
        const output = join(root, 'addition');
        await assert.rejects(stageRuntimeAddition({ root: output, bindgen: '/must-not-run' }), /--datadir-release/u);
        const path = join(root, 'datadir-release.json');
        for (const override of [{ byte_length: 26214401 }, { sha256: 'A'.repeat(64) }, { url: 'https://robinhood.phiresky.xyz/datadirs/old.rhdata.zst' }]) {
            await rm(path, { force: true });
            await writeFile(path, JSON.stringify({ ...release, ...override }));
            await assert.rejects(stageRuntimeAddition({ root: output, bindgen: '/must-not-run', datadirRelease: path }));
        }
        await rm(path);
        await writeFile(path, JSON.stringify(release));
        await assert.rejects(stageRuntimeAddition({ root: output, bindgen: '/must-not-run', datadirRelease: path, fullSha: 'bad' }), /Full manifest/u);
        await assert.rejects(readFile(join(output, 'wasm')), /ENOENT/u);
    } finally {
        await rm(root, { recursive: true });
    }
});

test('an existing addition is preserved and rejected before any build command', async () => {
    const root = await mkdtemp(join(tmpdir(), 'robin-runtime-stage-'));
    try {
        const path = join(root, 'datadir-release.json');
        await writeFile(path, JSON.stringify(release));
        await writeFile(join(root, 'retained'), 'existing addition');
        await assert.rejects(stageRuntimeAddition({ root, bindgen: '/must-not-run', datadirRelease: path }), { code: 'EEXIST' });
        assert.equal(await readFile(join(root, 'retained'), 'utf8'), 'existing addition');
    } finally {
        await rm(root, { recursive: true });
    }
});
