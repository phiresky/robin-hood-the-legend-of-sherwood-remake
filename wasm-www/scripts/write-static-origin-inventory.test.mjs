import assert from 'node:assert/strict';
import test from 'node:test';
import { mkdir, mkdtemp, readFile, rm, symlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { resolve } from 'node:path';
import { writeStaticOriginInventory } from './write-static-origin-inventory.mjs';

const sourceCommit = 'ab'.repeat(20);
const cargoLockSha256 = 'cd'.repeat(32);

test('static origin inventory is sorted, typed, hashed, and excludes _headers controls', async t => {
    const temporary = await mkdtemp(resolve(tmpdir(), 'static-inventory-'));
    t.after(() => rm(temporary, { recursive: true, force: true }));
    const root = resolve(temporary, 'public');
    const output = resolve(temporary, 'public.json');
    await mkdir(resolve(root, 'assets'), { recursive: true });
    await writeFile(resolve(root, '_headers'), 'control');
    await writeFile(resolve(root, 'index.html'), '<!doctype html>');
    await writeFile(resolve(root, 'assets/app.js'), 'export {};');
    await writeFile(resolve(root, 'assets/bridge.js'), 'export {};');
    await writeFile(resolve(root, 'assets/bridge_bg.wasm'), Buffer.from([0]));
    const inventory = await writeStaticOriginInventory({
        origin: 'public', root, sourceCommit, cargoLockSha256, output,
    });
    assert.deepEqual(inventory.artifacts.map(artifact => artifact.path), [
        'assets/app.js',
        'assets/bridge.js',
        'assets/bridge_bg.wasm',
        'index.html',
    ]);
    assert.equal(inventory.artifacts[0].media_type, 'text/javascript');
    assert.match(inventory.artifacts[0].sha256, /^[0-9a-f]{64}$/u);
    assert.equal((await readFile(output, 'utf8')).endsWith('\n'), false);
});

test('static origin inventory rejects symlinks and in-origin output', async t => {
    const temporary = await mkdtemp(resolve(tmpdir(), 'static-inventory-bad-'));
    t.after(() => rm(temporary, { recursive: true, force: true }));
    const root = resolve(temporary, 'runtime');
    await mkdir(root);
    await writeFile(resolve(temporary, 'outside.js'), 'export {};');
    await symlink(resolve(temporary, 'outside.js'), resolve(root, 'linked.js'));
    await assert.rejects(writeStaticOriginInventory({
        origin: 'runtime', root, sourceCommit, cargoLockSha256, output: resolve(temporary, 'runtime.json'),
    }), /symlink/u);
    await rm(resolve(root, 'linked.js'));
    await writeFile(resolve(root, 'index.html'), '<!doctype html>');
    await assert.rejects(writeStaticOriginInventory({
        origin: 'runtime', root, sourceCommit, cargoLockSha256, output: resolve(root, 'inventory.json'),
    }), /outside/u);
});
