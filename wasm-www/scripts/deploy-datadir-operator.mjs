import { execFile as execFileCallback } from 'node:child_process';
import { createHash } from 'node:crypto';
import { chmod, cp, lstat, mkdir, readFile, readdir, realpath, writeFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { promisify } from 'node:util';
import { currentWorkerDeployment, proveWorkerVersion } from './cloudflare-worker-version.mjs';
import { extractWranglerDeployVersion } from './extract-wrangler-deploy-version.mjs';

const execFile = promisify(execFileCallback);
const NODE_VERSION = 'v24.19.0';
const WRANGLER_VERSION = '4.127.1';
const WORKER = 'robinhood-datadir-assets';

function sha256(bytes) { return createHash('sha256').update(bytes).digest('hex'); }
function canonical(value) {
    if (Array.isArray(value)) return value.map(canonical);
    if (value !== null && typeof value === 'object') {
        return Object.fromEntries(Object.entries(value).sort(([left], [right]) => left.localeCompare(right))
            .map(([key, item]) => [key, canonical(item)]));
    }
    return value;
}
async function requireAbsent(path, label) {
    if (await lstat(path).catch(() => undefined) !== undefined) throw new Error(`${label} must be absent: ${path}`);
}
async function requireExecutable(path, label) {
    const facts = await lstat(path).catch(() => undefined);
    if (facts === undefined || !facts.isFile() || facts.isSymbolicLink() || (facts.mode & 0o111) === 0) throw new Error(`${label} must be a real executable`);
    return realpath(path);
}
async function makeReadOnly(path) {
    const facts = await lstat(path);
    if (facts.isSymbolicLink()) throw new Error(`manual datadir stage contains symlink ${path}`);
    if (facts.isDirectory()) {
        for (const entry of await readdir(path)) await makeReadOnly(resolve(path, entry));
        await chmod(path, 0o555);
    } else if (facts.isFile()) await chmod(path, 0o444);
    else throw new Error(`manual datadir stage contains non-regular entry ${path}`);
}
async function defaultVerifyAuthority(args) {
    const module = await import('./datadir-release-authority.mjs');
    return module.verifyDatadirReleaseAuthority(args);
}
async function defaultWriteReceipt(args) {
    const module = await import('./datadir-release-authority.mjs');
    return module.writeDatadirDeploymentReceipt(args);
}
async function defaultVerifyReceipt(args) {
    const module = await import('./datadir-release-authority.mjs');
    return module.verifyDatadirDeploymentReceipt(args);
}
async function gitHead(repoRoot, execFileImpl) {
    return (await execFileImpl('git', ['rev-parse', 'HEAD'], { cwd: repoRoot, encoding: 'utf8' })).stdout.trim();
}

export async function runDatadirOperatorDeployment({
    datadirRoot,
    inventoryPath,
    authorityPath,
    expectedAuthoritySha256,
    stage,
    receiptPath,
    evidence,
    execute,
    repoRoot,
    accountId,
    apiToken,
    processNodeVersion = process.version,
    execFileImpl = execFile,
    fetchImpl = fetch,
    verifyAuthorityImpl = defaultVerifyAuthority,
    writeReceiptImpl = defaultWriteReceipt,
    verifyReceiptImpl = defaultVerifyReceipt,
    currentWorkerImpl = currentWorkerDeployment,
    proveWorkerImpl = proveWorkerVersion,
}) {
    if (processNodeVersion !== NODE_VERSION) throw new Error(`manual datadir deployment requires exact Node.js ${NODE_VERSION}`);
    const wasmRoot = resolve(repoRoot, 'wasm-www');
    const wrangler = await requireExecutable(resolve(wasmRoot, 'node_modules/.bin/wrangler'), 'pinned Wrangler');
    if ((await execFileImpl(wrangler, ['--version'], { cwd: wasmRoot, encoding: 'utf8' })).stdout.trim() !== WRANGLER_VERSION) {
        throw new Error(`manual datadir deployment requires exact Wrangler ${WRANGLER_VERSION}`);
    }
    const verified = await verifyAuthorityImpl({
        authorityPath,
        expectedAuthoritySha256,
        inventoryPath,
        root: datadirRoot,
    });
    if (verified.authority.worker_name !== WORKER) throw new Error('datadir authority binds the wrong Worker');
    if (await gitHead(repoRoot, execFileImpl) !== verified.authority.source_commit
        || sha256(await readFile(resolve(repoRoot, 'Cargo.lock'))) !== verified.authority.cargo_lock_sha256) {
        throw new Error('manual datadir deployment checkout differs from its exact authority');
    }
    await requireAbsent(stage, 'manual datadir stage');
    await mkdir(stage);
    await cp(datadirRoot, resolve(stage, 'datadir-dist'), { errorOnExist: true, force: false, recursive: true });
    await cp(authorityPath, resolve(stage, 'datadir-authority.json'), { errorOnExist: true, force: false });
    await cp(inventoryPath, resolve(stage, 'datadir-inventory.json'), { errorOnExist: true, force: false });
    await mkdir(resolve(stage, 'deploy'));
    await cp(resolve(wasmRoot, 'deploy/wrangler-datadir.json'), resolve(stage, 'deploy/wrangler-datadir.json'), { errorOnExist: true, force: false });
    await makeReadOnly(stage);
    const config = resolve(stage, 'deploy/wrangler-datadir.json');
    await execFileImpl(wrangler, ['deploy', '--dry-run', '--config', config], {
        cwd: wasmRoot,
        encoding: 'utf8',
        env: process.env,
        timeout: 10 * 60 * 1000,
    });
    if (!execute) return { authoritySha256: verified.authoritySha256, mutated: false, stage };
    await requireAbsent(receiptPath, 'datadir deployment receipt');
    await requireAbsent(evidence, 'private datadir deployment evidence');
    await mkdir(evidence, { mode: 0o700 });
    const rollback = await currentWorkerImpl({ accountId, allowAbsent: true, apiToken, fetchImpl, worker: WORKER });
    const wranglerOutput = resolve(evidence, 'wrangler-datadir.ndjson');
    await execFileImpl(wrangler, ['deploy', '--config', config], {
        cwd: wasmRoot,
        encoding: 'utf8',
        env: { ...process.env, WRANGLER_OUTPUT_FILE_PATH: wranglerOutput },
        timeout: 10 * 60 * 1000,
    });
    await chmod(wranglerOutput, 0o600);
    const workerVersionId = extractWranglerDeployVersion(await readFile(wranglerOutput, 'utf8'), WORKER);
    const proof = await proveWorkerImpl({ accountId, apiToken, fetchImpl, versionId: workerVersionId, worker: WORKER });
    const authored = await writeReceiptImpl({ authorityPath, output: receiptPath, workerVersionId });
    const receipt = await verifyReceiptImpl({
        authorityPath,
        expectedReceiptSha256: authored.receiptSha256,
        receiptPath,
    });
    for (const [source, name] of [
        [authorityPath, 'datadir-authority.json'],
        [inventoryPath, 'datadir-inventory.json'],
        [receiptPath, 'datadir-deployment.json'],
    ]) {
        await cp(source, resolve(evidence, name), { errorOnExist: true, force: false });
        await chmod(resolve(evidence, name), 0o600);
    }
    const deployment = canonical({
        authority_sha256: verified.authoritySha256,
        finished_at: new Date().toISOString(),
        inventory_sha256: verified.inventorySha256,
        receipt_sha256: receipt.receiptSha256,
        rollback,
        schema_version: 1,
        worker_deployment_id: proof.current.deploymentId,
        worker_name: WORKER,
        worker_version_id: workerVersionId,
    });
    await writeFile(resolve(evidence, 'deployment.json'), `${JSON.stringify(deployment)}\n`, { flag: 'wx', mode: 0o600 });
    return { evidence, mutated: true, receiptPath, receiptSha256: receipt.receiptSha256, workerVersionId };
}

function argumentsFrom(values) {
    const result = { execute: false };
    for (let index = 0; index < values.length; index += 1) {
        const key = values[index];
        if (key === '--execute') { result.execute = true; continue; }
        const value = values[index + 1];
        if (value === undefined || !key.startsWith('--')) throw new Error(`invalid manual datadir argument ${key}`);
        result[key.slice(2).replaceAll('-', '_')] = value;
        index += 1;
    }
    for (const key of ['datadir', 'inventory', 'authority', 'authority_sha256', 'stage']) {
        if (result[key] === undefined) throw new Error(`missing --${key.replaceAll('_', '-')}`);
    }
    if (result.execute && (result.receipt === undefined || result.evidence === undefined)) {
        throw new Error('--execute requires absent --receipt and --evidence paths');
    }
    return result;
}

async function main() {
    const values = argumentsFrom(process.argv.slice(2));
    const repoRoot = resolve(import.meta.dirname, '../..');
    const result = await runDatadirOperatorDeployment({
        accountId: process.env.CLOUDFLARE_ACCOUNT_ID,
        apiToken: process.env.CLOUDFLARE_API_TOKEN,
        authorityPath: values.authority,
        datadirRoot: values.datadir,
        evidence: values.evidence,
        execute: values.execute,
        expectedAuthoritySha256: values.authority_sha256,
        inventoryPath: values.inventory,
        receiptPath: values.receipt,
        repoRoot,
        stage: values.stage,
    });
    console.log(result.mutated
        ? `deployed immutable datadir Worker ${result.workerVersionId}; receipt ${result.receiptPath}`
        : `manual datadir preflight passed without mutation: ${result.authoritySha256}`);
}

const invokedPath = process.argv[1];
if (invokedPath !== undefined && import.meta.url === pathToFileURL(resolve(invokedPath)).href) {
    main().catch(error => { console.error(error instanceof Error ? error.message : String(error)); process.exitCode = 1; });
}
