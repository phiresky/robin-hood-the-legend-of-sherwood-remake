import assert from 'node:assert/strict';
import test from 'node:test';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { stageRuntimeAddition, validateContentIdentity } from './stage-runtime-addition.mjs';

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
