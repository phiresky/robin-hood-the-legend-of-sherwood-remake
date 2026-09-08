import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { chmod, lstat, mkdir, mkdtemp, readFile, readdir, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { resolve } from 'node:path';
import test from 'node:test';
import { runDatadirOperatorDeployment } from './deploy-datadir-operator.mjs';

const sourceCommit = 'ab'.repeat(20);
const versionId = '12345678-90ab-4def-8123-456789abcdef';

async function writable(path) {
    const facts = await lstat(path).catch(() => undefined);
    if (facts?.isDirectory()) {
        await chmod(path, 0o700);
        for (const entry of await readdir(path)) await writable(resolve(path, entry));
    } else if (facts?.isFile()) await chmod(path, 0o600);
}

test('manual operation stages only verified datadir bytes and emits the exact post-deploy receipt/evidence', async t => {
    const root = await mkdtemp(resolve(tmpdir(), 'manual-datadir-deploy-'));
    t.after(async () => { await writable(root); await rm(root, { force: true, recursive: true }); });
    const repoRoot = resolve(root, 'repo');
    const wasmRoot = resolve(repoRoot, 'wasm-www');
    const datadirRoot = resolve(root, 'datadir-dist');
    const inventoryPath = resolve(root, 'datadir-inventory.json');
    const authorityPath = resolve(root, 'datadir-authority.json');
    const cargoLock = Buffer.from('lock\n');
    const cargoLockSha256 = createHash('sha256').update(cargoLock).digest('hex');
    await mkdir(resolve(wasmRoot, 'node_modules/.bin'), { recursive: true });
    await mkdir(resolve(wasmRoot, 'deploy'));
    await mkdir(datadirRoot);
    await writeFile(resolve(repoRoot, 'Cargo.lock'), cargoLock);
    await writeFile(resolve(datadirRoot, '_headers'), 'immutable');
    await writeFile(resolve(datadirRoot, 'demo.json'), '{}');
    await writeFile(inventoryPath, '{}');
    await writeFile(authorityPath, '{}');
    await writeFile(resolve(wasmRoot, 'deploy/wrangler-datadir.json'), '{}');
    const wrangler = resolve(wasmRoot, 'node_modules/.bin/wrangler');
    await writeFile(wrangler, '#!/bin/sh\nexit 1\n');
    await chmod(wrangler, 0o755);
    const calls = [];
    const execFileImpl = async (file, args, options) => {
        if (file === 'git') return { stdout: `${sourceCommit}\n` };
        if (args[0] === '--version') return { stdout: '4.127.1\n' };
        if (args.includes('--dry-run')) { calls.push('dry-run'); return { stdout: '' }; }
        calls.push('deploy');
        await writeFile(options.env.WRANGLER_OUTPUT_FILE_PATH, `${JSON.stringify({
            type: 'deploy', version_id: versionId, worker_name: 'robinhood-datadir-assets',
        })}\n`);
        return { stdout: '' };
    };
    const authority = {
        cargo_lock_sha256: cargoLockSha256,
        source_commit: sourceCommit,
        worker_name: 'robinhood-datadir-assets',
    };
    const receiptPath = resolve(root, 'datadir-deployment.json');
    const evidence = resolve(root, 'evidence');
    const result = await runDatadirOperatorDeployment({
        accountId: '11'.repeat(16),
        apiToken: 'test-token-that-is-long-enough',
        authorityPath,
        currentWorkerImpl: async () => null,
        datadirRoot,
        evidence,
        execFileImpl,
        execute: true,
        expectedAuthoritySha256: '22'.repeat(32),
        inventoryPath,
        processNodeVersion: 'v24.19.0',
        proveWorkerImpl: async () => ({
            current: { deploymentId: 'aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee', versionId },
        }),
        receiptPath,
        repoRoot,
        stage: resolve(root, 'stage'),
        verifyAuthorityImpl: async () => ({
            authority,
            authoritySha256: '22'.repeat(32),
            inventorySha256: '33'.repeat(32),
        }),
        verifyReceiptImpl: async () => ({ receiptSha256: '44'.repeat(32) }),
        writeReceiptImpl: async ({ output, workerVersionId }) => {
            assert.equal(workerVersionId, versionId);
            await writeFile(output, '{}');
            return { receiptSha256: '44'.repeat(32) };
        },
    });
    assert.deepEqual(calls, ['dry-run', 'deploy']);
    assert.equal(result.workerVersionId, versionId);
    assert.deepEqual(JSON.parse(await readFile(resolve(evidence, 'deployment.json'), 'utf8')), {
        authority_sha256: '22'.repeat(32),
        finished_at: JSON.parse(await readFile(resolve(evidence, 'deployment.json'), 'utf8')).finished_at,
        inventory_sha256: '33'.repeat(32),
        receipt_sha256: '44'.repeat(32),
        rollback: null,
        schema_version: 1,
        worker_deployment_id: 'aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee',
        worker_name: 'robinhood-datadir-assets',
        worker_version_id: versionId,
    });
});
