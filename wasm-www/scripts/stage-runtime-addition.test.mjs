import assert from 'node:assert/strict';
import test from 'node:test';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { runtimeBuildPlan } from './build-runtime.mjs';
import { RUNTIME_ADDITION_BUILD, stageRuntimeAddition, validateContentIdentity } from './stage-runtime-addition.mjs';

test('production runtime additions are the threaded game build', () => {
    assert.equal(RUNTIME_ADDITION_BUILD.threads, true);
    assert.equal(RUNTIME_ADDITION_BUILD.requireIdentity, true);
    const plan = runtimeBuildPlan({ outDir: '/tmp/addition', ...RUNTIME_ADDITION_BUILD });
    assert(plan.cargo.includes('scripts/wasm-threads.cargo-config.toml'));
    assert(plan.cargo.includes('audio,wasm-threads'));
    assert(plan.cargo.includes('robin'));
    assert.equal(plan.optimize, null);
});

test('staging rejects invalid identity and size before creating an artifact', () => {
    const valid = { demoSha: 'a'.repeat(64), nativeDemoSha: 'b'.repeat(64), demoBytes: '26214400' };
    assert.equal(validateContentIdentity(valid).demoBytes, 26214400);
    for (const override of [{ demoSha: 'A'.repeat(64) }, { nativeDemoSha: '' },
        { demoBytes: '026' }, { demoBytes: '0' }, { demoBytes: '26214401' }, { fullSha: 'bad' }]) {
        assert.throws(() => validateContentIdentity({ ...valid, ...override }));
    }
});

test('an existing addition is preserved and rejected before any build command', async () => {
    const root = await mkdtemp(join(tmpdir(), 'robin-runtime-stage-'));
    try {
        await writeFile(join(root, 'retained'), 'existing addition');
        await assert.rejects(stageRuntimeAddition({ root, bindgen: '/must-not-run',
            demoSha: 'a'.repeat(64), nativeDemoSha: 'b'.repeat(64), demoBytes: 1 }), { code: 'EEXIST' });
        assert.equal(await readFile(join(root, 'retained'), 'utf8'), 'existing addition');
    } finally {
        await rm(root, { recursive: true });
    }
});
