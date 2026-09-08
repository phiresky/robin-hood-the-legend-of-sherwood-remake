import { admissionFixture } from './replay-admission-wasm-fixture.mjs';
import assert from 'node:assert/strict';
import { execFile as execFileCallback } from 'node:child_process';
import { createHash } from 'node:crypto';
import {
    chmod, cp, link, lstat, mkdir, mkdtemp, open, readFile, readdir, rename, rm, symlink, writeFile,
} from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { basename, dirname, resolve } from 'node:path';
import test from 'node:test';
import { promisify } from 'node:util';
import {
    OperatorBundleInstalledButParentSyncFailed,
    OperatorBundleInstalledCommandError,
    OperatorBundleInstalledInvalid,
    OperatorBundleInstallStateUncertain,
    atomicInstallNoreplace,
    assembleOperatorDeploymentBundle,
    copyBoundedTree,
    createOperatorWranglerSnapshot,
    readExactOperatorCheckout,
    removeOperatorWranglerSnapshot,
    stageOperatorDeploymentBundle,
    validateCloudflarePublicationMaterialization,
    validateOperatorDeploymentBundle,
    validateOperatorDeploymentStage,
    validateOperatorWranglerSnapshot,
} from './operator-deployment-bundle.mjs';
import { assembleRuntimeCorpus } from './assemble-runtime-corpus.mjs';
import { authorRuntimeJavascriptModules } from './runtime-javascript-modules.mjs';
import { verifyStaticOriginInventory } from './verify-static-origin-inventory.mjs';
import { verifyRuntimeSourceContract } from './verify-runtime-source-contract.mjs';
import { buildStaticOriginInventory } from './write-static-origin-inventory.mjs';

const sourceCommit = 'ab'.repeat(20);
const sourceTreeSha1 = 'bc'.repeat(20);
const datadirSourceCommit = 'cd'.repeat(20);
const datadirCargoLockSha256 = '11'.repeat(32);
const publicationManifestSha256 = '66'.repeat(32);
const publicationLockSha256 = '77'.repeat(32);
const versionId = '12345678-90ab-4def-8123-456789abcdef';
const execFile = promisify(execFileCallback);
const deploymentConfigFiles = ['wrangler-public.json', 'wrangler-runtime.json', 'wrangler-signer.json'];
const originOrder = ['public', 'identity_signer', 'deployment_authority'];
const originFacts = {
    public: { inventory: 'inventories/cloudflare-public-v1.json', root: 'cloudflare-public' },
    identity_signer: { inventory: 'inventories/cloudflare-identity-signer-v1.json', root: 'cloudflare-identity-signer' },
    deployment_authority: { inventory: 'inventories/deployment-authority-v1.json', root: 'deployment' },
};

function canonical(value) {
    if (Array.isArray(value)) return value.map(canonical);
    if (value !== null && typeof value === 'object') {
        return Object.fromEntries(Object.entries(value).sort(([left], [right]) => Buffer.compare(Buffer.from(left), Buffer.from(right)))
            .map(([key, item]) => [key, canonical(item)]));
    }
    return value;
}

function bytes(value, newline = false) { return Buffer.from(`${JSON.stringify(canonical(value))}${newline ? '\n' : ''}`); }
function sha256(value) { return createHash('sha256').update(value).digest('hex'); }
function artifact(value, mediaType = 'application/octet-stream') {
    return { byte_length: value.length, media_type: mediaType, sha256: sha256(value) };
}

function mediaType(path) {
    if (path.endsWith('.json')) return 'application/json';
    if (path.endsWith('.html')) return 'text/html';
    if (path === '_headers') return 'text/plain';
    return 'application/octet-stream';
}

async function originInventory(root, origin) {
    const paths = [];
    const directories = new Set(['.']);
    async function visit(directory, prefix = '') {
        for (const entry of (await readdir(directory, { withFileTypes: true })).sort((a, b) => Buffer.compare(Buffer.from(a.name), Buffer.from(b.name)))) {
            const path = prefix === '' ? entry.name : `${prefix}/${entry.name}`;
            if (entry.isDirectory()) {
                directories.add(path);
                await visit(resolve(directory, entry.name), path);
            }
            else paths.push(path);
        }
    }
    await visit(root);
    return canonical({
        directories: [...directories]
            .sort((left, right) => Buffer.compare(Buffer.from(left), Buffer.from(right)))
            .map(path => ({ path, unix_mode: 0o555 })),
        files: await Promise.all(paths.map(async path => {
            const value = await readFile(resolve(root, path));
            return { artifact: artifact(value, mediaType(path)), path, unix_mode: 0o444 };
        })),
        origin,
        root: originFacts[origin].root,
        schema_version: 1,
    });
}

function outputInventory(inventories, inventoryArtifacts) {
    const files = [];
    const directories = new Set(['.', 'inventories']);
    for (const origin of originOrder) {
        const inventory = inventories[origin];
        for (const directory of inventory.directories) {
            directories.add(directory.path === '.' ? inventory.root : `${inventory.root}/${directory.path}`);
        }
        for (const file of inventory.files) {
            files.push({
                ...file,
                artifact: { ...file.artifact, media_type: 'application/octet-stream' },
                path: `${inventory.root}/${file.path}`,
            });
        }
        files.push({
            artifact: { ...inventoryArtifacts[origin], media_type: 'application/octet-stream' },
            path: originFacts[origin].inventory,
            unix_mode: 0o444,
        });
    }
    const order = (left, right) => Buffer.compare(Buffer.from(left.path), Buffer.from(right.path));
    return canonical({
        directories: [...directories].map(path => ({ path, unix_mode: 0o555 })).sort(order),
        files: files.sort(order),
    });
}

async function makeReadOnly(path) {
    const facts = await lstat(path);
    if (facts.isDirectory()) {
        for (const entry of await readdir(path)) await makeReadOnly(resolve(path, entry));
        await chmod(path, 0o555);
    } else await chmod(path, 0o444);
}

async function makeWritable(path) {
    const facts = await lstat(path).catch(() => undefined);
    if (facts?.isDirectory()) {
        await chmod(path, 0o700);
        for (const entry of await readdir(path)) await makeWritable(resolve(path, entry));
    } else if (facts?.isFile()) await chmod(path, 0o600);
}

async function makeHandoffSealed(path) {
    const facts = await lstat(path);
    if (facts.isDirectory()) {
        for (const entry of await readdir(path)) await makeHandoffSealed(resolve(path, entry));
        await chmod(path, 0o550);
    } else await chmod(path, 0o440);
}

async function createDeploymentConfig(path) {
    await mkdir(path);
    for (const file of deploymentConfigFiles) await writeFile(resolve(path, file), '{}');
}

async function installValidRuntimeForRealVerifier(value) {
    const repoRoot = resolve(import.meta.dirname, '..', '..');
    const contract = await verifyRuntimeSourceContract(repoRoot);
    const addition = resolve(value.root, 'runtime-addition');
    const build = resolve(addition, 'wasm', sourceCommit.slice(0, 12));
    const wasm = Buffer.from('wasm fixture');
    const wasmGzip = Buffer.from('compressed wasm fixture');
    const client = 'snippets/robin_rs-build/js/browser_identity_client.js';
    await mkdir(resolve(build, 'Data/Interface/Fonts'), { recursive: true });
    await mkdir(resolve(build, 'Data/Interface/UI'), { recursive: true });
    await mkdir(resolve(build, 'snippets/robin_rs-build/js'), { recursive: true });
    await writeFile(
        resolve(build, 'robin.js'),
        `import { requestIdentity } from './${client}';\nexport default async () => requestIdentity();`,
    );
    await writeFile(resolve(build, client), 'export const requestIdentity = () => undefined;\n');
    await writeFile(resolve(build, 'robin.js.gz'), 'compressed js fixture');
    await writeFile(resolve(build, 'robin_bg.wasm'), wasm);
    await writeFile(resolve(build, 'robin_bg.wasm.gz'), wasmGzip);
    await writeFile(resolve(build, 'Data/Interface/Fonts/arial.ttf'), 'font fixture');
    await writeFile(resolve(build, 'Data/Interface/UI/marker.png'), 'image fixture');
    await writeFile(resolve(build, 'preload-assets.json'), `${JSON.stringify([
        { path: 'Data/Interface/Fonts/arial.ttf', url: 'Data/Interface/Fonts/arial.ttf' },
        { path: 'Data/Interface/UI/marker.png', url: 'Data/Interface/UI/marker.png' },
    ], null, 2)}\n`);
    const admissionJs = Buffer.from('export function validate_compact_replay() {}');
    const admissionWasm = admissionFixture();
    await writeFile(resolve(build, 'replay_admission.js'), admissionJs);
    await writeFile(resolve(build, 'replay_admission_bg.wasm'), admissionWasm);
    const javascriptModules = await authorRuntimeJavascriptModules(build, { replayAdmission: true });
    const datadir = JSON.parse(await readFile(
        resolve(value.materializationRoot, 'deployment/datadir-authority.json'),
        'utf8',
    )).demo;
    const manifest = `${JSON.stringify({
        commit: sourceCommit,
        short: sourceCommit.slice(0, 12),
        builtAt: '2026-08-30T12:00:00Z',
        netProtocol: contract.netProtocol,
        ticketSchema: contract.ticketSchema,
        multiplayerContent: {
            schema: contract.contentSchema,
            demo: {
                url: datadir.datadir_url,
                sha256: datadir.datadir_sha256,
                byteLength: datadir.datadir_byte_length,
                nativeContentSha256: datadir.native_content_sha256,
            },
            full: { manifestSha256: 'ee'.repeat(32) },
        },
        files: {
            js: 'robin.js',
            jsGzip: 'robin.js.gz',
            wasm: 'robin_bg.wasm',
            wasmGzip: 'robin_bg.wasm.gz',
            replayAdmissionJs: 'replay_admission.js',
            replayAdmissionWasm: 'replay_admission_bg.wasm',
        },
        javascriptModules,
        sha256: { wasm: sha256(wasm), wasmGzip: sha256(wasmGzip), replayAdmissionJs: sha256(admissionJs), replayAdmissionWasm: sha256(admissionWasm) },
    }, null, 2)}\n`;
    await writeFile(resolve(build, 'manifest.json'), manifest);
    await writeFile(resolve(addition, 'wasm/latest.json'), manifest);
    const assembled = resolve(value.root, 'real-runtime');
    await assembleRuntimeCorpus({
        addition,
        datadirAuthority: resolve(value.materializationRoot, 'deployment/datadir-authority.json'),
        datadirDeployment: resolve(value.materializationRoot, 'deployment/datadir-deployment.json'),
        existing: null,
        output: assembled,
    });
    await makeWritable(value.runtimeRoot);
    await rm(resolve(value.runtimeRoot, 'runtime'), { recursive: true });
    await rename(assembled, resolve(value.runtimeRoot, 'runtime'));
    await rewriteRuntimeInventory(value, sourceCommit);
    value.repoRoot = repoRoot;
}

async function rewriteApprovedMaterialization(value) {
    const receiptPath = resolve(value.materializationRoot, 'cloudflare-publication-materialization-v1.json');
    const receipt = JSON.parse(await readFile(receiptPath, 'utf8'));
    const inventories = {};
    const inventoryArtifacts = {};
    for (const origin of originOrder) {
        inventories[origin] = await originInventory(
            resolve(value.materializationRoot, originFacts[origin].root),
            origin,
        );
        const inventoryBytes = bytes(inventories[origin]);
        await writeFile(resolve(value.materializationRoot, originFacts[origin].inventory), inventoryBytes);
        inventoryArtifacts[origin] = artifact(inventoryBytes, 'application/json');
        receipt.origins[originOrder.indexOf(origin)].inventory = inventoryArtifacts[origin];
    }
    receipt.output_inventory = outputInventory(inventories, inventoryArtifacts);
    const receiptBytes = bytes(receipt);
    value.materializationReceiptSha256 = sha256(receiptBytes);
    await writeFile(receiptPath, receiptBytes);
    await writeFile(
        resolve(value.materializationRoot, 'cloudflare-publication-materialization-v1.sha256'),
        value.materializationReceiptSha256,
    );
    await makeReadOnly(value.materializationRoot);
}

async function rewriteRuntimeInventory(value, commit) {
    await makeWritable(value.runtimeRoot);
    const inventory = await buildStaticOriginInventory({
        cargoLockSha256: sha256(await readFile(resolve(value.repoRoot, 'Cargo.lock'))),
        origin: 'runtime',
        root: resolve(value.runtimeRoot, 'runtime'),
        sourceCommit: commit,
    });
    const inventoryBytes = bytes(inventory);
    value.runtimeInventorySha256 = sha256(inventoryBytes);
    await writeFile(resolve(value.runtimeRoot, 'inventories/runtime.json'), inventoryBytes);
    await makeHandoffSealed(value.runtimeRoot);
}

async function fixture(t) {
    const root = await mkdtemp(resolve(tmpdir(), 'operator-deployment-bundle-v2-'));
    t.after(async () => {
        await makeWritable(root);
        await rm(root, { force: true, recursive: true });
    });
    const repoRoot = resolve(root, 'repo');
    const materializationRoot = resolve(root, 'materialization');
    const runtimeRoot = resolve(root, 'wasm-static');
    const runtimeOrigin = resolve(runtimeRoot, 'runtime');
    const routesPath = resolve(root, 'routes.json');
    const cargoLock = Buffer.from('exact cargo lock\n');
    const cargoLockSha256 = sha256(cargoLock);
    await mkdir(resolve(materializationRoot, 'cloudflare-public'), { recursive: true });
    await mkdir(resolve(materializationRoot, 'cloudflare-identity-signer'), { recursive: true });
    await mkdir(resolve(materializationRoot, 'deployment'), { recursive: true });
    await mkdir(resolve(materializationRoot, 'inventories'), { recursive: true });
    await mkdir(resolve(runtimeOrigin, 'wasm'), { recursive: true });
    await mkdir(resolve(runtimeRoot, 'inventories'));
    await mkdir(repoRoot);
    await writeFile(resolve(repoRoot, 'Cargo.lock'), cargoLock);
    for (const [directory, content] of [
        [resolve(materializationRoot, 'cloudflare-public'), '<!doctype html>'],
        [resolve(materializationRoot, 'cloudflare-identity-signer'), '<!doctype html>'],
    ]) {
        await writeFile(resolve(directory, '_headers'), 'controls');
        await writeFile(resolve(directory, 'index.html'), content);
    }
    await writeFile(resolve(runtimeOrigin, '_headers'), 'runtime controls');
    await writeFile(resolve(runtimeOrigin, 'wasm/latest.json'), '{}');
    const authority = {
        cargo_lock_sha256: datadirCargoLockSha256,
        demo: {
            content_manifest_sha256: '22'.repeat(32),
            content_manifest_url: 'https://robinhood.phiresky.xyz/datadirs/demo-leicester/robinhood-web-content.json',
            datadir_byte_length: 42,
            datadir_sha256: '33'.repeat(32),
            datadir_url: 'https://robinhood.phiresky.xyz/datadirs/demo-leicester/v8-web-opus-q80.rhdata.zst',
            native_content_sha256: '44'.repeat(32),
        },
        inventory_sha256: '55'.repeat(32),
        public_root_url: 'https://robinhood.phiresky.xyz/datadirs/',
        route_pattern: 'robinhood.phiresky.xyz/datadirs/*',
        schema_version: 1,
        source_commit: datadirSourceCommit,
        worker_name: 'robinhood-datadir-assets',
    };
    const authorityBytes = bytes(authority);
    const receipt = {
        authority_sha256: sha256(authorityBytes),
        demo: authority.demo,
        inventory_sha256: authority.inventory_sha256,
        public_root_url: authority.public_root_url,
        route_pattern: authority.route_pattern,
        schema_version: authority.schema_version,
        source_commit: authority.source_commit,
        worker_name: authority.worker_name,
        worker_version_id: versionId,
    };
    const receiptBytes = bytes(receipt);
    const routes = [
        { pattern: 'robinhood.phiresky.xyz/api*', script: null },
        { pattern: 'robinhood.phiresky.xyz/.well-known/acme-challenge/*', script: null },
        { pattern: 'robinhood.phiresky.xyz/wasm/*', script: 'robinhood-runtime-assets' },
        { pattern: 'robinhood.phiresky.xyz/datadirs/*', script: 'robinhood-datadir-assets' },
        { pattern: 'robinhood.phiresky.xyz/*', script: 'robinhood-public-site' },
    ];
    const exposure = {
        backend_api_manifest_root: 'backend/manifests',
        backend_api_route: '/api*',
        cloudflare_routes: routes,
        cloudflare_zone: 'phiresky.xyz',
        identity_signer_origin: 'https://identity.robinhood.phiresky.xyz',
        identity_signer_static_root: 'cloudflare-identity-signer',
        operator_private_paths: [
            'backend/publication-v3.json', 'deployment', 'private', 'publication-lock-v3.json',
            'publication-lock-v3.sha256', 'publication-manifest-v3.json', 'publication-manifest-v3.sha256',
        ],
        public_origin: 'https://robinhood.phiresky.xyz',
        public_static_root: 'cloudflare-public',
        schema_version: 3,
    };
    await writeFile(resolve(materializationRoot, 'deployment/datadir-authority.json'), authorityBytes);
    await writeFile(resolve(materializationRoot, 'deployment/datadir-deployment.json'), receiptBytes);
    await writeFile(resolve(materializationRoot, 'deployment/exposure-v3.json'), bytes(exposure));
    await writeFile(resolve(runtimeOrigin, 'wasm/datadir-deployment.json'), receiptBytes);
    await writeFile(routesPath, JSON.stringify({ routes, schema_version: 1, worker_name: 'robinhood-public-site', zone_name: 'phiresky.xyz' }));
    const inventories = {};
    const inventoryArtifacts = {};
    for (const origin of originOrder) {
        inventories[origin] = await originInventory(resolve(materializationRoot, originFacts[origin].root), origin);
        const inventoryBytes = bytes(inventories[origin]);
        await writeFile(resolve(materializationRoot, originFacts[origin].inventory), inventoryBytes);
        inventoryArtifacts[origin] = artifact(inventoryBytes, 'application/json');
    }
    const materializationReceipt = canonical({
        cargo_lock_sha256: cargoLockSha256,
        origins: originOrder.map(origin => ({
            inventory: inventoryArtifacts[origin],
            inventory_path: originFacts[origin].inventory,
            origin,
            root: originFacts[origin].root,
        })),
        output_inventory: outputInventory(inventories, inventoryArtifacts),
        publication_lock_sha256: publicationLockSha256,
        publication_manifest_sha256: publicationManifestSha256,
        publication_schema_version: 3,
        schema_version: 1,
        source_commit: sourceCommit,
        source_tree_sha1: sourceTreeSha1,
    });
    const materializationReceiptBytes = bytes(materializationReceipt);
    const materializationReceiptSha256 = sha256(materializationReceiptBytes);
    await writeFile(resolve(materializationRoot, 'cloudflare-publication-materialization-v1.json'), materializationReceiptBytes);
    await writeFile(resolve(materializationRoot, 'cloudflare-publication-materialization-v1.sha256'), materializationReceiptSha256);
    const runtimeInventory = await buildStaticOriginInventory({
        cargoLockSha256,
        origin: 'runtime',
        root: runtimeOrigin,
        sourceCommit,
    });
    const runtimeInventoryBytes = bytes(runtimeInventory);
    const runtimeInventorySha256 = sha256(runtimeInventoryBytes);
    await writeFile(resolve(runtimeRoot, 'inventories/runtime.json'), runtimeInventoryBytes);
    await makeReadOnly(materializationRoot);
    await makeHandoffSealed(runtimeRoot);
    const verifyOriginImpl = async (origin, originRoot, inventoryPath, expectedInventorySha256) => verifyStaticOriginInventory({
        expectedInventorySha256,
        inventoryPath,
        origin,
        root: originRoot,
    }).then(verified => origin === 'runtime'
        ? { ...verified, latest: { commit: verified.inventory.source_commit } }
        : verified);
    return {
        cargoLockSha256,
        deploymentConfigSha256: Object.fromEntries(deploymentConfigFiles.map(file => [file, sha256(Buffer.from('{}'))])),
        materializationReceiptSha256,
        materializationRoot,
        output: resolve(root, 'bundle'),
        repoRoot,
        root,
        routesPath,
        runtimeInventorySha256,
        runtimeRoot,
        verifyOriginImpl,
        verifyPublicBuildImpl: async () => {},
        verifySignerBuildImpl: async () => {},
    };
}

function assemblyOptions(value, additions = {}) {
    return {
        checkoutImpl: async () => ({
            cargoLockSha256: value.cargoLockSha256,
            deploymentConfigSha256: value.deploymentConfigSha256,
            sourceCommit,
            sourceTreeSha1,
        }),
        expectedMaterializationReceiptSha256: value.materializationReceiptSha256,
        expectedRuntimeInventorySha256: value.runtimeInventorySha256,
        materializationRoot: value.materializationRoot,
        output: value.output,
        repoRoot: value.repoRoot,
        routesPath: value.routesPath,
        runtimeRoot: value.runtimeRoot,
        verifyOriginImpl: value.verifyOriginImpl,
        verifyPublicBuildImpl: value.verifyPublicBuildImpl,
        verifySignerBuildImpl: value.verifySignerBuildImpl,
        ...additions,
    };
}

function validationOptions(value, assembled, additions = {}) {
    return {
        bundle: value.output,
        checkoutImpl: async () => ({
            cargoLockSha256: value.cargoLockSha256,
            deploymentConfigSha256: value.deploymentConfigSha256,
            sourceCommit,
            sourceTreeSha1,
        }),
        expectedManifestSha256: assembled.manifestSha256,
        repoRoot: value.repoRoot,
        routesPath: value.routesPath,
        verifyOriginImpl: value.verifyOriginImpl,
        verifyPublicBuildImpl: value.verifyPublicBuildImpl,
        verifySignerBuildImpl: value.verifySignerBuildImpl,
        ...additions,
    };
}

async function assemblingEntries(value) {
    const prefix = `.${basename(value.output)}.assembling-`;
    return (await readdir(value.root)).filter(entry => entry.startsWith(prefix));
}

test('assembles and stages one V2-only bundle from the frozen materialization wire', async t => {
    const value = await fixture(t);
    const admitted = await validateCloudflarePublicationMaterialization({
        expectedReceiptSha256: value.materializationReceiptSha256,
        root: value.materializationRoot,
    });
    assert.equal(admitted.receipt.publication_schema_version, 3);
    assert.ok(admitted.receipt.output_inventory.files.every(file => file.artifact.media_type === 'application/octet-stream'));
    const assembled = await assembleOperatorDeploymentBundle(assemblyOptions(value));
    assert.equal(assembled.manifest.schema_version, 2);
    assert.equal(assembled.manifest.materialization.approved_receipt_sha256, value.materializationReceiptSha256);
    assert.equal(assembled.manifest.materialization.receipt.artifact.media_type, 'application/json');
    assert.equal(assembled.manifest.materialization.sidecar.artifact.media_type, 'text/plain');
    assert.ok(Object.values(assembled.manifest.materialization.origin_inventories)
        .every(binding => binding.artifact.media_type === 'application/json'));
    assert.equal(assembled.manifest.datadir.worker_version_id, versionId);
    assert.equal(await lstat(resolve(value.output, 'deployment-v1.json')).catch(() => undefined), undefined);
    assert.equal((await lstat(resolve(value.output, 'deployment-v2.json'))).mode & 0o777, 0o444);
    assert.deepEqual(await assemblingEntries(value), []);

    const config = resolve(value.root, 'deploy');
    await createDeploymentConfig(config);
    await writeFile(resolve(config, 'operator-secret.txt'), 'must not be staged');
    await writeFile(resolve(config, 'wrangler-v1.json'), '{}');
    const stage = resolve(value.root, 'stage');
    const staged = await stageOperatorDeploymentBundle({
        ...validationOptions(value, assembled),
        deploymentConfigDirectory: config,
        stage,
    });
    assert.equal((await lstat(resolve(stage, 'runtime-dist'))).mode & 0o777, 0o555);
    assert.equal((await lstat(resolve(stage, 'deployment-v2.json'))).mode & 0o777, 0o444);
    assert.equal(await lstat(resolve(stage, 'deployment-v1.json')).catch(() => undefined), undefined);
    assert.equal(await lstat(resolve(stage, 'deploy/operator-secret.txt')).catch(() => undefined), undefined);
    assert.equal(await lstat(resolve(stage, 'deploy/wrangler-v1.json')).catch(() => undefined), undefined);
    for (const label of ['runtime', 'signer', 'public']) {
        const authority = staged.wranglerSnapshotAuthorities[label];
        const snapshot = await createOperatorWranglerSnapshot({ authority, label, stage });
        await validateOperatorWranglerSnapshot({ authority, label, snapshot: snapshot.sealedPath });
        assert.equal((await lstat(snapshot.capPath)).mode & 0o777, 0o700);
        assert.equal((await lstat(snapshot.sealedPath)).mode & 0o777, 0o500);
        assert.equal((await lstat(snapshot.cwd)).mode & 0o777, 0o700);
        const path = snapshot.path;
        await removeOperatorWranglerSnapshot(snapshot);
        assert.equal(await lstat(path).catch(() => undefined), undefined);
    }
});

test('assembly composes its retained runtime capability with the real runtime verifier', async t => {
    const value = await fixture(t);
    await installValidRuntimeForRealVerifier(value);
    const assembled = await assembleOperatorDeploymentBundle(assemblyOptions(value, {
        verifyOriginImpl: undefined,
    }));
    assert.equal(assembled.manifest.schema_version, 2);
    assert.equal(assembled.manifest.source_commit, sourceCommit);
    assert.equal((await lstat(resolve(value.output, 'deployment-v2.json'))).mode & 0o777, 0o444);
    assert.deepEqual(await assemblingEntries(value), []);
});

test('real pinned Wrangler dry-runs from private writable scratch while sealed inputs stay exact', async t => {
    const root = await mkdtemp(resolve(tmpdir(), 'operator-real-wrangler-snapshot-'));
    t.after(() => rm(root, { force: true, recursive: true }));
    const stage = resolve(root, 'stage');
    const repoWasmRoot = resolve(import.meta.dirname, '..');
    const wrangler = resolve(repoWasmRoot, 'node_modules/.bin/wrangler');
    await mkdir(resolve(stage, 'deploy'), { recursive: true });
    for (const [label, origin] of [['runtime', 'runtime-dist'], ['signer', 'signer-dist'], ['public', 'dist']]) {
        await mkdir(resolve(stage, origin), { recursive: true });
        const assetName = label === 'runtime' ? 'asset.wasm' : 'index.html';
        const assetBytes = label === 'runtime' ? Buffer.from('minimal wasm asset') : Buffer.from('<!doctype html>');
        const headersBytes = Buffer.from('/immutable/*\n  Cache-Control: public, max-age=31536000, immutable\n');
        await writeFile(resolve(stage, origin, assetName), assetBytes);
        await writeFile(resolve(stage, origin, '_headers'), headersBytes);
        const configName = `wrangler-${label}.json`;
        const configBytes = await readFile(resolve(repoWasmRoot, 'deploy', configName));
        await writeFile(resolve(stage, 'deploy', configName), configBytes);
        const authority = {
            configByteLength: configBytes.length,
            configName,
            configSha256: sha256(configBytes),
            directories: ['.'],
            files: {
                '_headers': { byteLength: headersBytes.length, sha256: sha256(headersBytes) },
                [assetName]: { byteLength: assetBytes.length, sha256: sha256(assetBytes) },
            },
            origin,
        };
        const snapshot = await createOperatorWranglerSnapshot({ authority, label, stage });
        const snapshotPath = snapshot.path;
        try {
            const dryRun = await execFile(wrangler, ['deploy', '--dry-run', '--config', snapshot.configPath], {
                cwd: snapshot.cwd,
                encoding: 'utf8',
                env: {
                    ...process.env,
                    HOME: resolve(snapshot.cwd, '.home'),
                    NODE_OPTIONS: undefined,
                    NODE_PATH: undefined,
                    PATH: `${dirname(process.execPath)}:${process.env.PATH ?? '/usr/bin:/bin'}`,
                    WRANGLER_NO_SKILLS_UPDATE_PROMPTS: 'true',
                    WRANGLER_LOG_PATH: resolve(snapshot.cwd, '.wrangler-logs'),
                    WRANGLER_SEND_METRICS: 'false',
                    XDG_CACHE_HOME: resolve(snapshot.cwd, '.xdg-cache'),
                    XDG_CONFIG_HOME: resolve(snapshot.cwd, '.xdg-config'),
                },
                maxBuffer: 16 * 1024 * 1024,
                timeout: 60_000,
            });
            assert.doesNotMatch(dryRun.stderr, /Failed to write log|EEXIST/u);
            await validateOperatorWranglerSnapshot({ authority, label, snapshot: snapshot.sealedPath });
            assert.ok((await lstat(resolve(snapshot.cwd, '.wrangler'))).isDirectory());
        } finally {
            await removeOperatorWranglerSnapshot(snapshot);
        }
        assert.equal(await lstat(snapshotPath).catch(() => undefined), undefined);
    }
});

test('real Wrangler snapshot validator rejects content, config, mode, link, closure, and authority drift', async t => {
    const value = await fixture(t);
    const assembled = await assembleOperatorDeploymentBundle(assemblyOptions(value));
    const config = resolve(value.root, 'snapshot-adversary-config');
    await createDeploymentConfig(config);
    const stage = resolve(value.root, 'snapshot-adversary-stage');
    const staged = await stageOperatorDeploymentBundle({
        ...validationOptions(value, assembled),
        deploymentConfigDirectory: config,
        stage,
    });
    for (const kind of ['content', 'config', 'file mode', 'directory mode', 'extra', 'symlink', 'hardlink', 'authority']) {
        await t.test(kind, async () => {
            const label = 'runtime';
            const authority = staged.wranglerSnapshotAuthorities[label];
            const snapshot = await createOperatorWranglerSnapshot({ authority, label, stage });
            try {
                const origin = resolve(snapshot.sealedPath, 'runtime-dist');
                const asset = resolve(origin, 'wasm/latest.json');
                if (kind === 'content') {
                    await chmod(asset, 0o600);
                    await writeFile(asset, '{"tampered":true}');
                    await chmod(asset, 0o400);
                } else if (kind === 'config') {
                    const path = resolve(snapshot.sealedPath, 'deploy/wrangler-runtime.json');
                    await chmod(path, 0o600);
                    await writeFile(path, '{"tampered":true}');
                    await chmod(path, 0o400);
                } else if (kind === 'file mode') await chmod(asset, 0o600);
                else if (kind === 'directory mode') await chmod(origin, 0o700);
                else if (kind === 'extra') {
                    await chmod(origin, 0o700);
                    await writeFile(resolve(origin, 'extra'), 'extra');
                    await chmod(resolve(origin, 'extra'), 0o400);
                    await chmod(origin, 0o500);
                } else if (kind === 'symlink') {
                    await chmod(resolve(origin, 'wasm'), 0o700);
                    await rm(asset);
                    await symlink(resolve(origin, '_headers'), asset);
                    await chmod(resolve(origin, 'wasm'), 0o500);
                } else if (kind === 'hardlink') {
                    await chmod(resolve(origin, 'wasm'), 0o700);
                    await rm(asset);
                    await link(resolve(origin, '_headers'), asset);
                    await chmod(resolve(origin, 'wasm'), 0o500);
                }
                const approved = kind === 'authority'
                    ? { ...authority, configSha256: 'ee'.repeat(32) }
                    : authority;
                await assert.rejects(
                    validateOperatorWranglerSnapshot({ authority: approved, label, snapshot: snapshot.sealedPath }),
                    /authority|closure|differs|mode|owner-read-only|singleton|symlink/u,
                );
            } finally {
                await removeOperatorWranglerSnapshot(snapshot);
            }
            assert.equal(await lstat(snapshot.path).catch(() => undefined), undefined);
        });
    }
});

test('exact Node gate rejects wrong 24.x and 26 before bundle writes', async t => {
    for (const processNodeVersion of ['v24.18.0', 'v26.0.0']) {
        await t.test(processNodeVersion, async t2 => {
            const value = await fixture(t2);
            let copied = false;
            await assert.rejects(assembleOperatorDeploymentBundle(assemblyOptions(value, {
                copyImpl: async () => { copied = true; },
                processNodeVersion,
            })), /requires exact Node\.js v24\.19\.0/u);
            assert.equal(copied, false);
            assert.equal(await lstat(value.output).catch(() => undefined), undefined);

            const assembled = await assembleOperatorDeploymentBundle(assemblyOptions(value));
            await assert.rejects(validateOperatorDeploymentBundle(validationOptions(value, assembled, {
                processNodeVersion,
            })), /requires exact Node\.js v24\.19\.0/u);
        });
    }
});

test('assemble CLI requires both independent approval digests in its exact six-position contract', async () => {
    const script = resolve(import.meta.dirname, 'operator-deployment-bundle.mjs');
    await assert.rejects(
        execFile(process.execPath, [script, 'assemble', 'materialization', 'runtime', 'output', 'receipt-sha', 'repo']),
        error => {
            assert.match(error.stderr, /APPROVED_RUNTIME_INVENTORY_SHA256 REPO_ROOT/u);
            return true;
        },
    );
});

test('rejects self-consistent extra deployment-authority files and directories', async t => {
    for (const [name, mutate] of [
        ['file', value => writeFile(resolve(value.materializationRoot, 'deployment/operator-note.json'), '{}')],
        ['directory', value => mkdir(resolve(value.materializationRoot, 'deployment/unused'))],
    ]) {
        await t.test(name, async t2 => {
            const value = await fixture(t2);
            await makeWritable(value.materializationRoot);
            await mutate(value);
            await rewriteApprovedMaterialization(value);
            await assert.rejects(validateCloudflarePublicationMaterialization({
                expectedReceiptSha256: value.materializationReceiptSha256,
                root: value.materializationRoot,
            }), /exact frozen deployment document closure/u);
        });
    }
});

test('rejects every fixed private namespace in a self-consistent materialized origin', async t => {
    for (const term of [
        'private', 'projection-authority', 'projection-exporter', 'projection-receipt',
        'source-tree-manifest', 'projection-execution', 'verifier-source-binding',
        'campaign-state', 'operator-config', 'Full', 'datadirs',
    ]) {
        await t.test(term, async t2 => {
            const value = await fixture(t2);
            await makeWritable(value.materializationRoot);
            const directory = resolve(value.materializationRoot, 'cloudflare-public', term);
            await mkdir(directory);
            await writeFile(resolve(directory, 'leak.json'), '{}');
            await rewriteApprovedMaterialization(value);
            await assert.rejects(validateCloudflarePublicationMaterialization({
                expectedReceiptSha256: value.materializationReceiptSha256,
                root: value.materializationRoot,
            }), /forbidden private authority path/u);
        });
    }
});

test('approved runtime handoff is the exact sealed two-entry singleton closure', async t => {
    for (const [name, mutate, reseal = true] of [
        ['extra root file', value => writeFile(resolve(value.runtimeRoot, 'ambient.txt'), 'ambient')],
        ['extra inventory', value => writeFile(resolve(value.runtimeRoot, 'inventories/old.json'), '{}')],
        ['runtime symlink', async value => {
            await symlink(resolve(value.runtimeRoot, 'runtime/_headers'), resolve(value.runtimeRoot, 'runtime/linked'));
        }],
        ['runtime hardlink', value => link(
            resolve(value.runtimeRoot, 'runtime/_headers'),
            resolve(value.runtimeRoot, 'runtime/linked'),
        )],
        ['runtime FIFO', value => execFile('/usr/bin/mkfifo', [resolve(value.runtimeRoot, 'runtime/fifo')])],
        ['wrong root mode', async value => { await chmod(value.runtimeRoot, 0o750); }, false],
        ['wrong inventory mode', async value => { await chmod(resolve(value.runtimeRoot, 'inventories/runtime.json'), 0o400); }, false],
    ]) {
        await t.test(name, async t2 => {
            const value = await fixture(t2);
            await makeWritable(value.runtimeRoot);
            await mutate(value);
            if (reseal) await makeHandoffSealed(value.runtimeRoot);
            else {
                if (name === 'wrong root mode') {
                    for (const entry of await readdir(value.runtimeRoot)) await makeHandoffSealed(resolve(value.runtimeRoot, entry));
                } else {
                    await makeHandoffSealed(value.runtimeRoot);
                    await chmod(resolve(value.runtimeRoot, 'inventories/runtime.json'), 0o400);
                }
            }
            await assert.rejects(
                assembleOperatorDeploymentBundle(assemblyOptions(value)),
                /contain exactly|closure|mode|symlink|singleton|non-regular|authority|mount/u,
            );
        });
    }
});

test('bounded retained copier rejects oversized, overdeep, special, and aliased inputs', async t => {
    const root = await mkdtemp(resolve(tmpdir(), 'operator-bounded-copy-adversary-'));
    t.after(() => rm(root, { force: true, recursive: true }));

    const oversized = resolve(root, 'oversized');
    await mkdir(oversized);
    const oversizedHandle = await open(resolve(oversized, 'large.bin'), 'w');
    await oversizedHandle.truncate(25 * 1024 * 1024 + 1);
    await oversizedHandle.close();
    await assert.rejects(
        copyBoundedTree(oversized, resolve(root, 'oversized-copy')),
        /bounded regular-file size|bounded read/u,
    );

    const overdeep = resolve(root, 'overdeep');
    await mkdir(overdeep);
    let directory = overdeep;
    for (let depth = 0; depth < 65; depth += 1) {
        directory = resolve(directory, 'd');
        await mkdir(directory);
    }
    await writeFile(resolve(directory, 'leaf'), 'leaf');
    await assert.rejects(
        copyBoundedTree(overdeep, resolve(root, 'overdeep-copy')),
        /maximum directory depth/u,
    );

    const special = resolve(root, 'special');
    await mkdir(special);
    await execFile('/usr/bin/mkfifo', [resolve(special, 'fifo')]);
    await assert.rejects(
        copyBoundedTree(special, resolve(root, 'special-copy')),
        /non-regular entry/u,
    );

    const source = resolve(root, 'source');
    await mkdir(source);
    await writeFile(resolve(source, 'value'), 'value');
    await symlink(source, resolve(root, 'source-alias'));
    await assert.rejects(
        copyBoundedTree(resolve(root, 'source-alias'), resolve(root, 'alias-copy')),
        /non-directory|changed|symlink/u,
    );
});

test('rejects old runtime authority and forged runtime inventory relabeling', async t => {
    const old = await fixture(t);
    await rewriteRuntimeInventory(old, 'de'.repeat(20));
    await assert.rejects(
        assembleOperatorDeploymentBundle(assemblyOptions(old)),
        /runtime inventory\/latest source authority differs/u,
    );

    const forged = await fixture(t);
    await makeWritable(forged.runtimeRoot);
    const inventoryPath = resolve(forged.runtimeRoot, 'inventories/runtime.json');
    const inventory = JSON.parse(await readFile(inventoryPath, 'utf8'));
    inventory.source_commit = 'ef'.repeat(20);
    await writeFile(inventoryPath, bytes(inventory));
    await makeHandoffSealed(forged.runtimeRoot);
    await assert.rejects(
        assembleOperatorDeploymentBundle(assemblyOptions(forged)),
        /independently approved digest|SHA-256 mismatch/u,
    );
});

test('checkout authority ignores replace refs and ambient repository/config substitution', async t => {
    const root = await mkdtemp(resolve(tmpdir(), 'operator-checkout-authority-'));
    t.after(() => rm(root, { force: true, recursive: true }));
    const repo = resolve(root, 'repo');
    await mkdir(repo);
    const git = async args => (await execFile('/usr/bin/git', args, { cwd: repo, encoding: 'utf8' })).stdout.trim();
    await git(['init', '--quiet']);
    await git(['config', 'user.email', 'operator-test@example.invalid']);
    await git(['config', 'user.name', 'Operator Test']);
    await writeFile(resolve(repo, 'tracked.txt'), 'reviewed\n');
    const cargoLock = Buffer.from('checkout cargo lock\n');
    await writeFile(resolve(repo, 'Cargo.lock'), cargoLock);
    await mkdir(resolve(repo, 'wasm-www/deploy'), { recursive: true });
    for (const file of deploymentConfigFiles) await writeFile(resolve(repo, 'wasm-www/deploy', file), '{}');
    await git(['add', 'Cargo.lock', 'tracked.txt', 'wasm-www/deploy']);
    await git(['commit', '--quiet', '-m', 'reviewed']);
    const reviewedCommit = await git(['rev-parse', 'HEAD']);
    const reviewedTree = await git(['rev-parse', 'HEAD^{tree}']);
    await writeFile(resolve(repo, 'tracked.txt'), 'attacker\n');
    await git(['add', 'tracked.txt']);
    await git(['commit', '--quiet', '-m', 'attacker']);
    const attackerCommit = await git(['rev-parse', 'HEAD']);
    const attackerTree = await git(['rev-parse', 'HEAD^{tree}']);
    await git(['checkout', '--quiet', '--detach', reviewedCommit]);
    await git(['replace', reviewedCommit, attackerCommit]);
    assert.equal(await git(['rev-parse', `${reviewedCommit}^{tree}`]), attackerTree);
    const attackerWorktree = resolve(root, 'attacker-worktree');
    await mkdir(attackerWorktree);
    await writeFile(resolve(attackerWorktree, 'Cargo.lock'), 'attacker cargo lock\n');
    await git(['config', 'core.fileMode', 'false']);
    await git(['config', 'core.fsmonitor', 'true']);
    await git(['config', 'core.worktree', attackerWorktree]);

    const invalidConfig = resolve(root, 'invalid-gitconfig');
    await writeFile(invalidConfig, '[invalid\n');
    const checkout = await readExactOperatorCheckout(repo, execFile, {
        ...process.env,
        GIT_CONFIG_COUNT: '1',
        GIT_CONFIG_GLOBAL: invalidConfig,
        GIT_CONFIG_KEY_0: 'core.worktree',
        GIT_CONFIG_SYSTEM: invalidConfig,
        GIT_CONFIG_VALUE_0: resolve(root, 'attacker-worktree'),
        GIT_DIR: resolve(root, 'attacker.git'),
        GIT_WORK_TREE: resolve(root, 'attacker-worktree'),
    });
    assert.deepEqual(checkout, {
        cargoLockSha256: sha256(cargoLock),
        deploymentConfigSha256: Object.fromEntries(deploymentConfigFiles.map(file => [file, sha256(Buffer.from('{}'))])),
        sourceCommit: reviewedCommit,
        sourceTreeSha1: reviewedTree,
    });

    await chmod(resolve(repo, 'tracked.txt'), 0o755);
    await assert.rejects(readExactOperatorCheckout(repo), /tracked changes|could not be proven clean/u);
    await chmod(resolve(repo, 'tracked.txt'), 0o644);

    let raced = false;
    await assert.rejects(readExactOperatorCheckout(repo, async (...args) => {
        const result = await execFile(...args);
        if (!raced && args[1].includes('diff')) {
            raced = true;
            await writeFile(resolve(repo, 'Cargo.lock'), 'post-diff attacker cargo lock\n');
        }
        return result;
    }), /Cargo\.lock differs from the exact commit/u);
});

test('rejects OOB receipt mismatch and every substituted receipt projection', async t => {
    const mismatch = await fixture(t);
    await assert.rejects(assembleOperatorDeploymentBundle(assemblyOptions(mismatch, {
        expectedMaterializationReceiptSha256: 'ff'.repeat(32),
    })), /independently approved digest/u);

    for (const field of [
        'cargo_lock_sha256', 'publication_lock_sha256', 'publication_manifest_sha256',
        'publication_schema_version', 'source_commit', 'source_tree_sha1',
    ]) {
        await t.test(field, async t2 => {
            const value = await fixture(t2);
            const assembled = await assembleOperatorDeploymentBundle(assemblyOptions(value));
            const path = resolve(value.output, 'deployment-v2.json');
            await chmod(path, 0o600);
            const manifest = JSON.parse(await readFile(path, 'utf8'));
            manifest[field] = field === 'publication_schema_version' ? 2
                : field.endsWith('sha256') ? 'ee'.repeat(32) : 'ee'.repeat(20);
            const changed = bytes(manifest, true);
            await writeFile(path, changed);
            await assert.rejects(validateOperatorDeploymentBundle(validationOptions(value, {
                manifestSha256: sha256(changed),
            })), /invalid|differs|not the exact/u);
        });
    }
});

test('rejects every nested deployment manifest authority substitution', async t => {
    for (const [name, mutate] of [
        ['schema', manifest => { manifest.schema_version = 1; }],
        ['routes', manifest => { manifest.routes_sha256 = 'ee'.repeat(32); }],
        ['datadir', manifest => { manifest.datadir.worker_name = 'attacker-worker'; }],
        ['public origin path', manifest => { manifest.origins.public.directory = 'origins/attacker'; }],
        ['runtime inventory', manifest => { manifest.runtime.inventory_sha256 = 'ee'.repeat(32); }],
        ['receipt binding path', manifest => { manifest.materialization.receipt.path = 'attacker.json'; }],
        ['receipt binding media', manifest => { manifest.materialization.receipt.artifact.media_type = 'text/plain'; }],
        ['signer inventory binding', manifest => {
            manifest.materialization.origin_inventories.identity_signer.artifact.sha256 = 'ee'.repeat(32);
        }],
    ]) {
        await t.test(name, async t2 => {
            const value = await fixture(t2);
            await assembleOperatorDeploymentBundle(assemblyOptions(value));
            const path = resolve(value.output, 'deployment-v2.json');
            await chmod(path, 0o600);
            const manifest = JSON.parse(await readFile(path, 'utf8'));
            mutate(manifest);
            const changed = bytes(manifest, true);
            await writeFile(path, changed);
            await chmod(path, 0o444);
            await assert.rejects(validateOperatorDeploymentBundle(validationOptions(value, {
                manifestSha256: sha256(changed),
            })), /invalid|differs|authority|path|substituted|schema/u);
        });
    }
});

test('rejects carried receipt, sidecar, inventory, and runtime receipt substitution', async t => {
    for (const [name, relative, mutate] of [
        ['receipt', 'authorities/materialization/cloudflare-publication-materialization-v1.json', value => Buffer.concat([value, Buffer.from(' ')])],
        ['sidecar', 'authorities/materialization/cloudflare-publication-materialization-v1.sha256', () => Buffer.from('ee'.repeat(32))],
        ['inventory', 'authorities/materialization/inventories/cloudflare-public-v1.json', value => Buffer.concat([value, Buffer.from(' ')])],
        ['runtime receipt', 'origins/runtime/wasm/datadir-deployment.json', value => Buffer.concat([value, Buffer.from(' ')])],
    ]) {
        await t.test(name, async t2 => {
            const value = await fixture(t2);
            const assembled = await assembleOperatorDeploymentBundle(assemblyOptions(value));
            const path = resolve(value.output, relative);
            await chmod(path, 0o600);
            await writeFile(path, mutate(await readFile(path)));
            await assert.rejects(validateOperatorDeploymentBundle(validationOptions(value, assembled)), /differs|canonical|substituted|mismatch/u);
        });
    }
});

test('rejects every carried origin and deployment authority substitution', async t => {
    for (const [name, relative] of [
        ['signer inventory', 'authorities/materialization/inventories/cloudflare-identity-signer-v1.json'],
        ['deployment inventory', 'authorities/materialization/inventories/deployment-authority-v1.json'],
        ['public origin', 'origins/public/index.html'],
        ['signer origin', 'origins/identity-signer/index.html'],
        ['datadir authority', 'authorities/materialization/deployment/datadir-authority.json'],
        ['datadir receipt', 'authorities/materialization/deployment/datadir-deployment.json'],
        ['deployment exposure', 'authorities/materialization/deployment/exposure-v3.json'],
    ]) {
        await t.test(name, async t2 => {
            const value = await fixture(t2);
            const assembled = await assembleOperatorDeploymentBundle(assemblyOptions(value));
            const path = resolve(value.output, relative);
            await chmod(path, 0o600);
            await writeFile(path, Buffer.concat([await readFile(path), Buffer.from(' ')]));
            await chmod(path, 0o444);
            await assert.rejects(
                validateOperatorDeploymentBundle(validationOptions(value, assembled)),
                /authority|canonical|differs|substituted|mismatch/u,
            );
        });
    }
});

test('rejects missing/extra closure, wrong modes, symlinks, hardlinks, and all V1 bundle forms', async t => {
    for (const [name, mutate] of [
        ['missing', async value => {
            await chmod(resolve(value.output, 'origins/public'), 0o700);
            await rm(resolve(value.output, 'origins/public/index.html'));
        }],
        ['extra', async value => {
            await chmod(resolve(value.output, 'authorities/materialization'), 0o700);
            await writeFile(resolve(value.output, 'authorities/materialization/extra.json'), '{}');
        }],
        ['mode', async value => chmod(resolve(value.output, 'origins/public/index.html'), 0o644)],
        ['directory mode', async value => chmod(resolve(value.output, 'origins/public'), 0o545)],
        ['symlink', async value => {
            await chmod(resolve(value.output, 'origins/public'), 0o700);
            const path = resolve(value.output, 'origins/public/index.html');
            await rm(path);
            await symlink(resolve(value.output, 'origins/public/_headers'), path);
        }],
        ['hardlink', async value => {
            await chmod(resolve(value.output, 'origins/public'), 0o700);
            const path = resolve(value.output, 'origins/public/index.html');
            const external = resolve(value.root, 'external-hardlink');
            await writeFile(external, await readFile(path));
            await rm(path);
            await link(external, path);
        }],
        ['internal hardlink', async value => {
            await chmod(resolve(value.output, 'origins/public'), 0o700);
            const path = resolve(value.output, 'origins/public/index.html');
            await rm(path);
            await link(resolve(value.output, 'origins/public/_headers'), path);
        }],
        ['directory symlink', async value => {
            await chmod(resolve(value.output, 'origins'), 0o700);
            const path = resolve(value.output, 'origins/public');
            await chmod(path, 0o700);
            await rm(path, { recursive: true });
            await symlink(resolve(value.output, 'origins/identity-signer'), path);
        }],
        ['special node', async value => {
            await chmod(resolve(value.output, 'origins/public'), 0o700);
            const path = resolve(value.output, 'origins/public/index.html');
            await rm(path);
            await execFile('/usr/bin/mkfifo', [path]);
        }],
        ['V1 extra', async value => {
            await chmod(value.output, 0o700);
            await writeFile(resolve(value.output, 'deployment-v1.json'), '{}');
        }],
        ['V1 only', async value => {
            await chmod(value.output, 0o700);
            await rename(resolve(value.output, 'deployment-v2.json'), resolve(value.output, 'deployment-v1.json'));
        }],
    ]) {
        await t.test(name, async t2 => {
            const value = await fixture(t2);
            const assembled = await assembleOperatorDeploymentBundle(assemblyOptions(value));
            await mutate(value);
            await assert.rejects(validateOperatorDeploymentBundle(validationOptions(value, assembled)));
        });
    }

    const rootAlias = await fixture(t);
    const assembled = await assembleOperatorDeploymentBundle(assemblyOptions(rootAlias));
    const alias = resolve(rootAlias.root, 'bundle-alias');
    await symlink(rootAlias.output, alias);
    await assert.rejects(validateOperatorDeploymentBundle(validationOptions(rootAlias, assembled, {
        bundle: alias,
    })), /real directory/u);
});

test('source path replacement between admission and copy fails closed and cleans staging', async t => {
    const value = await fixture(t);
    let swapped = false;
    await assert.rejects(assembleOperatorDeploymentBundle(assemblyOptions(value, {
        copyImpl: async (source, destination, options) => {
            if (!swapped) {
                swapped = true;
                const retained = resolve(value.root, 'retained-materialization');
                await rename(value.materializationRoot, retained);
                await cp(retained, value.materializationRoot, { recursive: true });
                await makeWritable(value.materializationRoot);
                await writeFile(resolve(value.materializationRoot, 'cloudflare-public/index.html'), 'substituted');
            }
            return copyBoundedTree(source, destination, options);
        },
    })), /differs|mode|authority|materialization/u);
    assert.equal(await lstat(value.output).catch(() => undefined), undefined);
    assert.deepEqual(await assemblingEntries(value), []);
});

test('approved runtime mutation at the copy boundary fails closed and cleans staging', async t => {
    const value = await fixture(t);
    let mutated = false;
    await assert.rejects(assembleOperatorDeploymentBundle(assemblyOptions(value, {
        copyImpl: async (source, destination, options) => {
            if (!mutated && options.label === 'approved runtime origin copy') {
                mutated = true;
                const latest = resolve(value.runtimeRoot, 'runtime/wasm/latest.json');
                await chmod(latest, 0o600);
                await writeFile(latest, '{"attacker":true}');
                await chmod(latest, 0o440);
            }
            return copyBoundedTree(source, destination, options);
        },
    })), /changed|authority|inventory|differs/u);
    assert.equal(await lstat(value.output).catch(() => undefined), undefined);
    assert.deepEqual(await assemblingEntries(value), []);
});

test('post-stage tampering is rejected by self-contained stage validation', async t => {
    const value = await fixture(t);
    const assembled = await assembleOperatorDeploymentBundle(assemblyOptions(value));
    const config = resolve(value.root, 'deploy');
    await createDeploymentConfig(config);
    const stage = resolve(value.root, 'stage');
    await stageOperatorDeploymentBundle({
        ...validationOptions(value, assembled), deploymentConfigDirectory: config, stage,
    });
    const tampered = resolve(stage, 'dist/index.html');
    await chmod(tampered, 0o600);
    await writeFile(tampered, 'delayed substitution');
    await assert.rejects(validateOperatorDeploymentStage({
        deploymentConfigDirectory: config,
        checkoutImpl: async () => ({
            cargoLockSha256: value.cargoLockSha256,
            deploymentConfigSha256: value.deploymentConfigSha256,
            sourceCommit,
            sourceTreeSha1,
        }),
        expectedManifestSha256: assembled.manifestSha256,
        repoRoot: value.repoRoot,
        routesPath: value.routesPath,
        stage,
        verifyOriginImpl: value.verifyOriginImpl,
        verifyPublicBuildImpl: value.verifyPublicBuildImpl,
        verifySignerBuildImpl: value.verifySignerBuildImpl,
    }), /differs|mode|authority/u);
});

test('staging consumes only the three exact tracked Wrangler configs and rejects drift', async t => {
    const value = await fixture(t);
    const assembled = await assembleOperatorDeploymentBundle(assemblyOptions(value));
    for (const kind of ['missing', 'changed', 'symlink', 'copy-boundary race']) {
        await t.test(kind, async t2 => {
            const config = resolve(value.root, `deploy-${kind.replaceAll(' ', '-')}`);
            await createDeploymentConfig(config);
            const publicConfig = resolve(config, 'wrangler-public.json');
            if (kind === 'missing') await rm(publicConfig);
            else if (kind === 'changed') await writeFile(publicConfig, '{"changed":true}');
            else if (kind === 'symlink') {
                await rm(publicConfig);
                await symlink(resolve(config, 'wrangler-runtime.json'), publicConfig);
            }
            let raced = false;
            await assert.rejects(stageOperatorDeploymentBundle({
                ...validationOptions(value, assembled),
                copyImpl: async (source, destination, options) => {
                    if (kind === 'copy-boundary race' && !raced) {
                        raced = true;
                        await writeFile(publicConfig, '{"raced":true}');
                    }
                    return copyBoundedTree(source, destination, options);
                },
                deploymentConfigDirectory: config,
                stage: resolve(value.root, `stage-config-${kind.replaceAll(' ', '-')}`),
            }), /config|singleton|tracked|exact/u);
            assert.equal(
                await lstat(resolve(value.root, `stage-config-${kind.replaceAll(' ', '-')}`)).catch(() => undefined),
                undefined,
            );
        });
    }
});

test('late failure securely cleans a sealed sibling stage and NOREPLACE preserves racing output', async t => {
    const failed = await fixture(t);
    await assert.rejects(assembleOperatorDeploymentBundle(assemblyOptions(failed, {
        verifyOriginImpl: async (...args) => {
            await failed.verifyOriginImpl(...args);
            throw new Error('injected post-seal verifier failure');
        },
    })), /injected post-seal verifier failure/u);
    assert.equal(await lstat(failed.output).catch(() => undefined), undefined);
    assert.deepEqual(await assemblingEntries(failed), []);

    const raced = await fixture(t);
    await assert.rejects(assembleOperatorDeploymentBundle(assemblyOptions(raced, {
        installImpl: async (source, destination, options) => {
            await mkdir(destination);
            await writeFile(resolve(destination, 'sentinel'), 'racing-operator');
            await atomicInstallNoreplace(source, destination, options);
        },
    })), /destination appeared during atomic installation/u);
    assert.equal(await readFile(resolve(raced.output, 'sentinel'), 'utf8'), 'racing-operator');
    assert.deepEqual(await assemblingEntries(raced), []);
});

test('post-install parent sync failure remains an unambiguous installed V2 outcome', async t => {
    const value = await fixture(t);
    await assert.rejects(assembleOperatorDeploymentBundle(assemblyOptions(value, {
        syncParentImpl: async () => { throw new Error('injected parent fsync failure'); },
    })), error => {
        assert.ok(error instanceof OperatorBundleInstalledButParentSyncFailed);
        assert.equal(error.code, 'OPERATOR_BUNDLE_INSTALLED_PARENT_SYNC_FAILED');
        assert.equal(error.destination, value.output);
        assert.equal(error.installed, true);
        return true;
    });
    assert.ok((await readFile(resolve(value.output, 'deployment-v2.json'))).length > 0);
    assert.equal(await lstat(resolve(value.output, 'deployment-v1.json')).catch(() => undefined), undefined);
    assert.deepEqual(await assemblingEntries(value), []);
});

test('atomic install reconciles command error, invalid install, and uncertain persistence', async t => {
    const commandError = await fixture(t);
    await assert.rejects(assembleOperatorDeploymentBundle(assemblyOptions(commandError, {
        installImpl: (source, destination, options) => atomicInstallNoreplace(source, destination, {
            ...options,
            moveImpl: async (parentHandle, sourceName, destinationName) => {
                const parent = `/proc/self/fd/${parentHandle.fd}`;
                await rename(resolve(parent, sourceName), resolve(parent, destinationName));
                throw new Error('injected mv post-rename error');
            },
        }),
    })), error => {
        assert.ok(error instanceof OperatorBundleInstalledCommandError);
        assert.equal(error.installed, true);
        return true;
    });
    assert.ok((await lstat(commandError.output)).isDirectory());

    const invalid = await fixture(t);
    await assert.rejects(assembleOperatorDeploymentBundle(assemblyOptions(invalid, {
        installImpl: async (source, destination, options) => {
            const result = await atomicInstallNoreplace(source, destination, options);
            const manifest = resolve(destination, 'deployment-v2.json');
            await chmod(manifest, 0o600);
            await writeFile(manifest, '{}');
            return result;
        },
    })), error => {
        assert.ok(error instanceof OperatorBundleInstalledInvalid);
        assert.equal(error.installed, true);
        assert.equal(error.valid, false);
        return true;
    });
    assert.ok((await lstat(invalid.output)).isDirectory());

    const uncertain = await fixture(t);
    await assert.rejects(assembleOperatorDeploymentBundle(assemblyOptions(uncertain, {
        installImpl: (source, destination, options) => atomicInstallNoreplace(source, destination, {
            ...options,
            moveImpl: async (parentHandle, sourceName) => {
                const parent = `/proc/self/fd/${parentHandle.fd}`;
                await rename(resolve(parent, sourceName), resolve(parent, `${sourceName}.lost`));
                throw new Error('injected unobservable move');
            },
        }),
    })), error => {
        assert.ok(error instanceof OperatorBundleInstallStateUncertain);
        assert.equal(error.state_uncertain, true);
        return true;
    });
    assert.equal(await lstat(uncertain.output).catch(() => undefined), undefined);
});

test('stage installation preserves racing outputs and reports durability and uncertain states exactly', async t => {
    const value = await fixture(t);
    const assembled = await assembleOperatorDeploymentBundle(assemblyOptions(value));
    const config = resolve(value.root, 'stage-install-config');
    await createDeploymentConfig(config);
    const options = stage => ({
        ...validationOptions(value, assembled),
        deploymentConfigDirectory: config,
        stage,
    });

    const raced = resolve(value.root, 'stage-raced');
    await assert.rejects(stageOperatorDeploymentBundle({
        ...options(raced),
        installImpl: async (source, destination, installOptions) => {
            await mkdir(destination);
            await writeFile(resolve(destination, 'sentinel'), 'racing operator');
            return atomicInstallNoreplace(source, destination, installOptions);
        },
    }), /destination appeared during atomic installation/u);
    assert.equal(await readFile(resolve(raced, 'sentinel'), 'utf8'), 'racing operator');

    const unsynced = resolve(value.root, 'stage-unsynced');
    await assert.rejects(stageOperatorDeploymentBundle({
        ...options(unsynced),
        syncParentImpl: async () => { throw new Error('injected stage parent fsync failure'); },
    }), error => {
        assert.ok(error instanceof OperatorBundleInstalledButParentSyncFailed);
        assert.equal(error.destination, unsynced);
        assert.equal(error.installed, true);
        return true;
    });
    assert.ok((await lstat(unsynced)).isDirectory());

    const uncertain = resolve(value.root, 'stage-uncertain');
    await assert.rejects(stageOperatorDeploymentBundle({
        ...options(uncertain),
        installImpl: (source, destination, installOptions) => atomicInstallNoreplace(source, destination, {
            ...installOptions,
            moveImpl: async (parentHandle, sourceName) => {
                const parent = `/proc/self/fd/${parentHandle.fd}`;
                await rename(resolve(parent, sourceName), resolve(parent, `${sourceName}.lost`));
                throw new Error('injected unobservable stage move');
            },
        }),
    }), error => {
        assert.ok(error instanceof OperatorBundleInstallStateUncertain);
        assert.equal(error.state_uncertain, true);
        return true;
    });
    assert.equal(await lstat(uncertain).catch(() => undefined), undefined);
});

test('stage installation validates the installed path after the atomic rename', async t => {
    const value = await fixture(t);
    const assembled = await assembleOperatorDeploymentBundle(assemblyOptions(value));
    const config = resolve(value.root, 'stage-installed-path-config');
    await createDeploymentConfig(config);
    const stage = resolve(value.root, 'stage-installed-path');
    let installed = false;
    let installedValidationCount = 0;
    await stageOperatorDeploymentBundle({
        ...validationOptions(value, assembled),
        deploymentConfigDirectory: config,
        installImpl: async (source, destination, installOptions) => {
            const outcome = await atomicInstallNoreplace(source, destination, installOptions);
            installed = true;
            return outcome;
        },
        stage,
        verifySignerBuildImpl: async root => {
            if (installed) {
                assert.ok(root.endsWith('/stage-installed-path/signer-dist'));
                assert.doesNotMatch(root, /\.stage-installed-path\.staging-/u);
                installedValidationCount += 1;
            }
        },
    });
    assert.ok(installedValidationCount >= 2);
});
