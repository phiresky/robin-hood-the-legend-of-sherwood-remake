import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { resolve } from 'node:path';
import test from 'node:test';
import { verifyStaticOriginInventory } from './verify-static-origin-inventory.mjs';
import { writeStaticOriginInventory } from './write-static-origin-inventory.mjs';

const sourceCommit = 'ab'.repeat(20);
const cargoLockSha256 = 'cd'.repeat(32);

async function fixture(t) {
    const temporary = await mkdtemp(resolve(tmpdir(), 'verify-static-inventory-'));
    t.after(() => rm(temporary, { recursive: true, force: true }));
    const root = resolve(temporary, 'runtime-dist');
    const inventoryPath = resolve(temporary, 'runtime-authority.json');
    await mkdir(resolve(root, 'wasm'), { recursive: true });
    await writeFile(resolve(root, '_headers'), 'deployment control');
    await writeFile(resolve(root, 'wasm/latest.json'), '{}');
    await writeStaticOriginInventory({
        origin: 'runtime', root, sourceCommit, cargoLockSha256, output: inventoryPath,
    });
    const inventoryBytes = await readFile(inventoryPath);
    return {
        inventoryPath,
        inventorySha256: createHash('sha256').update(inventoryBytes).digest('hex'),
        root,
        temporary,
    };
}

test('exact static inventory is independently reproduced and digest-bound', async t => {
    const value = await fixture(t);
    const verified = await verifyStaticOriginInventory({
        origin: 'runtime',
        root: value.root,
        inventoryPath: value.inventoryPath,
        expectedInventorySha256: value.inventorySha256,
    });
    assert.equal(verified.inventory.origin, 'runtime');
    assert.equal(verified.inventorySha256, value.inventorySha256);
});

test('verification fails closed when the runtime corpus is absent', async t => {
    const value = await fixture(t);
    await assert.rejects(verifyStaticOriginInventory({
        origin: 'runtime',
        root: resolve(value.temporary, 'absent-runtime-dist'),
        inventoryPath: value.inventoryPath,
        expectedInventorySha256: value.inventorySha256,
    }), /static origin is not a real directory/u);
});

test('verification rejects a wrong inventory digest or changed physical corpus', async t => {
    const value = await fixture(t);
    await assert.rejects(verifyStaticOriginInventory({
        origin: 'runtime',
        root: value.root,
        inventoryPath: value.inventoryPath,
        expectedInventorySha256: 'ef'.repeat(32),
    }), /inventory SHA-256 mismatch/u);

    await writeFile(resolve(value.root, 'wasm/latest.json'), '{"changed":true}');
    await assert.rejects(verifyStaticOriginInventory({
        origin: 'runtime',
        root: value.root,
        inventoryPath: value.inventoryPath,
        expectedInventorySha256: value.inventorySha256,
    }), /does not exactly match/u);
});
