import { createHash } from 'node:crypto';
import { readFile, lstat } from 'node:fs/promises';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { buildStaticOriginInventory } from './write-static-origin-inventory.mjs';

const DIGEST = /^[0-9a-f]{64}$/u;

function sha256(bytes) {
    return createHash('sha256').update(bytes).digest('hex');
}

function parseInventory(bytes, path) {
    let inventory;
    try {
        inventory = JSON.parse(bytes.toString('utf8'));
    } catch (error) {
        throw new Error(`static inventory is not valid JSON (${path}): ${error instanceof Error ? error.message : String(error)}`);
    }
    if (inventory === null || typeof inventory !== 'object' || Array.isArray(inventory)) {
        throw new Error('static inventory must be an object');
    }
    return inventory;
}

export async function verifyStaticOriginInventory({
    origin,
    root,
    inventoryPath,
    expectedInventorySha256,
}) {
    if (expectedInventorySha256 !== undefined && !DIGEST.test(expectedInventorySha256)) {
        throw new Error('expected inventory SHA-256 must be 64 lowercase hexadecimal characters');
    }
    const rootFacts = await lstat(root).catch(() => undefined);
    if (rootFacts === undefined || !rootFacts.isDirectory() || rootFacts.isSymbolicLink()) {
        throw new Error(`static origin is not a real directory: ${resolve(root)}`);
    }
    const inventoryBytes = await readFile(inventoryPath).catch(error => {
        throw new Error(`static inventory cannot be read: ${error instanceof Error ? error.message : String(error)}`);
    });
    const actualInventorySha256 = sha256(inventoryBytes);
    if (expectedInventorySha256 !== undefined && actualInventorySha256 !== expectedInventorySha256) {
        throw new Error(`static inventory SHA-256 mismatch: expected ${expectedInventorySha256}, got ${actualInventorySha256}`);
    }
    const inventory = parseInventory(inventoryBytes, inventoryPath);
    const expected = await buildStaticOriginInventory({
        origin,
        root,
        sourceCommit: inventory.source_commit,
        cargoLockSha256: inventory.cargo_lock_sha256,
    });
    const canonicalBytes = Buffer.from(JSON.stringify(expected));
    if (!inventoryBytes.equals(canonicalBytes)) {
        throw new Error('static inventory does not exactly match the canonical physical origin inventory');
    }
    return { inventory, inventorySha256: actualInventorySha256 };
}

async function main() {
    const [origin, root, inventoryPath, expectedInventorySha256, extra] = process.argv.slice(2);
    if ([origin, root, inventoryPath].some(value => value === undefined) || extra !== undefined) {
        throw new Error('usage: verify-static-origin-inventory.mjs ORIGIN ROOT INVENTORY [EXPECTED_INVENTORY_SHA256]');
    }
    const verified = await verifyStaticOriginInventory({
        origin,
        root,
        inventoryPath,
        expectedInventorySha256,
    });
    console.log(`verified exact ${verified.inventory.origin} inventory ${verified.inventorySha256}`);
}

const invokedPath = process.argv[1];
if (invokedPath !== undefined && import.meta.url === pathToFileURL(resolve(invokedPath)).href) {
    main().catch(error => {
        console.error(error instanceof Error ? error.message : String(error));
        process.exitCode = 1;
    });
}
