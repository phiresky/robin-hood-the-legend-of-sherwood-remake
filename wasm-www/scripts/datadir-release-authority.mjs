import { createHash } from 'node:crypto';
import { lstat, readFile, writeFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { DEPLOYMENT } from './verify-cloudflare-deployment.mjs';
import { verifyDatadirCorpus } from './verify-datadir-corpus.mjs';
import { verifyStaticOriginInventory } from './verify-static-origin-inventory.mjs';
import { writeStaticOriginInventory } from './write-static-origin-inventory.mjs';

const DIGEST = /^[0-9a-f]{64}$/u;
const COMMIT = /^[0-9a-f]{40}$/u;
const VERSION_ID = /^[0-9a-f]{8}-(?:[0-9a-f]{4}-){3}[0-9a-f]{12}$/u;
const DEMO_KEYS = Object.freeze([
    'content_manifest_url', 'content_manifest_sha256', 'datadir_url',
    'datadir_sha256', 'datadir_byte_length', 'native_content_sha256',
]);
const AUTHORITY_KEYS = Object.freeze([
    'schema_version', 'source_commit', 'cargo_lock_sha256', 'inventory_sha256',
    'worker_name', 'route_pattern', 'public_root_url', 'demo',
]);
const RECEIPT_KEYS = Object.freeze([
    'schema_version', 'authority_sha256', 'inventory_sha256', 'source_commit',
    'worker_name', 'worker_version_id', 'route_pattern', 'public_root_url', 'demo',
]);

function sha256(bytes) {
    return createHash('sha256').update(bytes).digest('hex');
}

function exactKeys(value, expected, label) {
    if (value === null || typeof value !== 'object' || Array.isArray(value)) {
        throw new Error(`${label} must be an object`);
    }
    const actual = Object.keys(value).sort();
    const wanted = [...expected].sort();
    if (JSON.stringify(actual) !== JSON.stringify(wanted)) {
        throw new Error(`${label} has unexpected keys: ${actual.join(', ')}`);
    }
}

function exact(value, expected, label) {
    if (value !== expected) throw new Error(`${label} must be ${JSON.stringify(expected)}`);
}

function canonical(value) {
    if (Array.isArray(value)) return value.map(canonical);
    if (value !== null && typeof value === 'object') {
        return Object.fromEntries(Object.entries(value)
            .sort(([left], [right]) => Buffer.compare(Buffer.from(left), Buffer.from(right)))
            .map(([key, item]) => [key, canonical(item)]));
    }
    return value;
}

function canonicalBytes(value) {
    return Buffer.from(JSON.stringify(canonical(value)));
}

function parseCanonical(bytes, label) {
    let value;
    try {
        value = JSON.parse(bytes.toString('utf8'));
    } catch (error) {
        throw new Error(`${label} is not valid JSON: ${error instanceof Error ? error.message : String(error)}`);
    }
    if (!canonicalBytes(value).equals(bytes)) throw new Error(`${label} is not canonical JSON`);
    return value;
}

function validateDemo(value, label) {
    exactKeys(value, DEMO_KEYS, label);
    for (const field of [
        'content_manifest_sha256', 'datadir_sha256', 'native_content_sha256',
    ]) {
        if (!DIGEST.test(value[field])) throw new Error(`${label} ${field} must be lowercase SHA-256`);
    }
    if (!Number.isSafeInteger(value.datadir_byte_length) || value.datadir_byte_length <= 0) {
        throw new Error(`${label} datadir_byte_length must be a positive safe integer`);
    }
    exact(
        value.content_manifest_url,
        `${DEPLOYMENT.publicOrigin}/datadirs/demo-leicester/robinhood-web-content.json`,
        `${label} content_manifest_url`,
    );
    exact(
        value.datadir_url,
        `${DEPLOYMENT.publicOrigin}/datadirs/demo-leicester/v8-web-opus-q80.rhdata.zst`,
        `${label} datadir_url`,
    );
}

function validateSharedAuthority(value, label) {
    exact(value.schema_version, 1, `${label} schema_version`);
    if (!COMMIT.test(value.source_commit)) throw new Error(`${label} source_commit is invalid`);
    if (!DIGEST.test(value.inventory_sha256)) throw new Error(`${label} inventory_sha256 is invalid`);
    exact(value.worker_name, DEPLOYMENT.datadirWorker, `${label} worker_name`);
    exact(value.route_pattern, `${DEPLOYMENT.publicHost}/datadirs/*`, `${label} route_pattern`);
    exact(value.public_root_url, `${DEPLOYMENT.publicOrigin}/datadirs/`, `${label} public_root_url`);
    validateDemo(value.demo, `${label} demo`);
}

export async function readDatadirReleaseAuthority(authorityPath, expectedAuthoritySha256) {
    if (expectedAuthoritySha256 !== undefined && !DIGEST.test(expectedAuthoritySha256)) {
        throw new Error('expected datadir authority SHA-256 must be lowercase hexadecimal');
    }
    const bytes = await readFile(authorityPath);
    const authoritySha256 = sha256(bytes);
    if (expectedAuthoritySha256 !== undefined && authoritySha256 !== expectedAuthoritySha256) {
        throw new Error(`datadir authority SHA-256 mismatch: expected ${expectedAuthoritySha256}, got ${authoritySha256}`);
    }
    const authority = parseCanonical(bytes, 'datadir authority');
    exactKeys(authority, AUTHORITY_KEYS, 'datadir authority');
    validateSharedAuthority(authority, 'datadir authority');
    if (!DIGEST.test(authority.cargo_lock_sha256)) {
        throw new Error('datadir authority cargo_lock_sha256 is invalid');
    }
    return { authority, authorityBytes: bytes, authoritySha256 };
}

export async function writeDatadirReleaseAuthority({
    root,
    sourceCommit,
    cargoLockSha256,
    inventoryPath,
    authorityPath,
}) {
    const corpus = await verifyDatadirCorpus(root);
    const inventory = await writeStaticOriginInventory({
        origin: 'datadir', root, sourceCommit, cargoLockSha256, output: inventoryPath,
    });
    const inventoryBytes = await readFile(inventoryPath);
    const inventorySha256 = sha256(inventoryBytes);
    const authority = canonical({
        schema_version: 1,
        source_commit: sourceCommit,
        cargo_lock_sha256: cargoLockSha256,
        inventory_sha256: inventorySha256,
        worker_name: DEPLOYMENT.datadirWorker,
        route_pattern: `${DEPLOYMENT.publicHost}/datadirs/*`,
        public_root_url: `${DEPLOYMENT.publicOrigin}/datadirs/`,
        demo: corpus.demo,
    });
    const authorityBytes = canonicalBytes(authority);
    await writeFile(authorityPath, authorityBytes, { flag: 'wx' });
    return { authority, authoritySha256: sha256(authorityBytes), inventory, inventorySha256 };
}

export async function verifyDatadirReleaseAuthority({
    root,
    inventoryPath,
    authorityPath,
    expectedAuthoritySha256,
}) {
    const corpus = await verifyDatadirCorpus(root);
    const verifiedAuthority = await readDatadirReleaseAuthority(authorityPath, expectedAuthoritySha256);
    const verifiedInventory = await verifyStaticOriginInventory({
        origin: 'datadir', root, inventoryPath,
        expectedInventorySha256: verifiedAuthority.authority.inventory_sha256,
    });
    exact(
        verifiedAuthority.authority.source_commit,
        verifiedInventory.inventory.source_commit,
        'datadir authority/inventory source_commit',
    );
    exact(
        verifiedAuthority.authority.cargo_lock_sha256,
        verifiedInventory.inventory.cargo_lock_sha256,
        'datadir authority/inventory cargo_lock_sha256',
    );
    if (JSON.stringify(verifiedAuthority.authority.demo) !== JSON.stringify(canonical(corpus.demo))) {
        throw new Error('datadir authority Demo identity does not match the exact physical corpus');
    }
    return {
        ...verifiedAuthority,
        inventory: verifiedInventory.inventory,
        inventorySha256: verifiedInventory.inventorySha256,
    };
}

export async function writeDatadirDeploymentReceipt({ authorityPath, workerVersionId, output }) {
    if (!VERSION_ID.test(workerVersionId)) throw new Error('datadir Worker version must be a lowercase UUID');
    const { authority, authoritySha256 } = await readDatadirReleaseAuthority(authorityPath);
    const receipt = canonical({
        schema_version: 1,
        authority_sha256: authoritySha256,
        inventory_sha256: authority.inventory_sha256,
        source_commit: authority.source_commit,
        worker_name: authority.worker_name,
        worker_version_id: workerVersionId,
        route_pattern: authority.route_pattern,
        public_root_url: authority.public_root_url,
        demo: authority.demo,
    });
    const receiptBytes = canonicalBytes(receipt);
    await writeFile(output, receiptBytes, { flag: 'wx' });
    return { receipt, receiptBytes, receiptSha256: sha256(receiptBytes) };
}

export async function verifyDatadirDeploymentReceipt({
    authorityPath,
    receiptPath,
    expectedReceiptSha256,
}) {
    if (expectedReceiptSha256 !== undefined && !DIGEST.test(expectedReceiptSha256)) {
        throw new Error('expected datadir receipt SHA-256 must be lowercase hexadecimal');
    }
    const verifiedAuthority = await readDatadirReleaseAuthority(authorityPath);
    const receiptBytes = await readFile(receiptPath);
    const receiptSha256 = sha256(receiptBytes);
    if (expectedReceiptSha256 !== undefined && receiptSha256 !== expectedReceiptSha256) {
        throw new Error(`datadir receipt SHA-256 mismatch: expected ${expectedReceiptSha256}, got ${receiptSha256}`);
    }
    const receipt = parseCanonical(receiptBytes, 'datadir deployment receipt');
    exactKeys(receipt, RECEIPT_KEYS, 'datadir deployment receipt');
    validateSharedAuthority(receipt, 'datadir deployment receipt');
    if (!DIGEST.test(receipt.authority_sha256)) throw new Error('receipt authority_sha256 is invalid');
    if (!VERSION_ID.test(receipt.worker_version_id)) throw new Error('receipt worker_version_id is invalid');
    exact(receipt.authority_sha256, verifiedAuthority.authoritySha256, 'receipt authority_sha256');
    for (const field of [
        'inventory_sha256', 'source_commit', 'worker_name', 'route_pattern', 'public_root_url',
    ]) {
        exact(receipt[field], verifiedAuthority.authority[field], `receipt ${field}`);
    }
    if (JSON.stringify(receipt.demo) !== JSON.stringify(verifiedAuthority.authority.demo)) {
        throw new Error('receipt Demo identity differs from its release authority');
    }
    return {
        ...verifiedAuthority,
        receipt,
        receiptBytes,
        receiptSha256,
    };
}

async function main() {
    const [mode, ...args] = process.argv.slice(2);
    if (mode === 'author' && args.length === 5) {
        const result = await writeDatadirReleaseAuthority({
            root: args[0], sourceCommit: args[1], cargoLockSha256: args[2],
            inventoryPath: args[3], authorityPath: args[4],
        });
        console.log(`authored datadir authority ${result.authoritySha256} and inventory ${result.inventorySha256}`);
    } else if (mode === 'verify' && (args.length === 3 || args.length === 4)) {
        const result = await verifyDatadirReleaseAuthority({
            root: args[0], inventoryPath: args[1], authorityPath: args[2],
            expectedAuthoritySha256: args[3],
        });
        console.log(`verified datadir authority ${result.authoritySha256} and inventory ${result.inventorySha256}`);
    } else if (mode === 'receipt' && args.length === 3) {
        const result = await writeDatadirDeploymentReceipt({
            authorityPath: args[0], workerVersionId: args[1], output: args[2],
        });
        console.log(`authored datadir deployment receipt ${result.receiptSha256}`);
    } else if (mode === 'verify-receipt' && (args.length === 2 || args.length === 3)) {
        const result = await verifyDatadirDeploymentReceipt({
            authorityPath: args[0], receiptPath: args[1], expectedReceiptSha256: args[2],
        });
        console.log(`verified datadir deployment receipt ${result.receiptSha256}`);
    } else {
        throw new Error('usage: datadir-release-authority.mjs author ROOT SOURCE_COMMIT CARGO_LOCK_SHA256 INVENTORY AUTHORITY | verify ROOT INVENTORY AUTHORITY [EXPECTED_AUTHORITY_SHA256] | receipt AUTHORITY WORKER_VERSION_ID OUTPUT | verify-receipt AUTHORITY RECEIPT [EXPECTED_RECEIPT_SHA256]');
    }
}

const invokedPath = process.argv[1];
if (invokedPath !== undefined && import.meta.url === pathToFileURL(resolve(invokedPath)).href) {
    main().catch(error => {
        console.error(error instanceof Error ? error.message : String(error));
        process.exitCode = 1;
    });
}
