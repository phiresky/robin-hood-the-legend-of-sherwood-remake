import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { mkdir, mkdtemp, open, readFile, rm, symlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { resolve } from 'node:path';
import test from 'node:test';
import { assembleDatadirCorpus } from './assemble-datadir-corpus.mjs';
import { assembleRuntimeCorpus } from './assemble-runtime-corpus.mjs';
import {
    verifyDatadirDeploymentReceipt,
    verifyDatadirReleaseAuthority,
    writeDatadirDeploymentReceipt,
    writeDatadirReleaseAuthority,
} from './datadir-release-authority.mjs';
import {
    DEMO_CONTENT_MANIFEST_PATH,
    DEMO_PATH,
    DEMO_ROOT,
    verifyDatadirCorpus,
    verifyDemoWebContentPackage,
} from './verify-datadir-corpus.mjs';
import {
    CLOUDFLARE_ASSET_BYTES_LIMIT,
    CLOUDFLARE_FREE_ASSET_LIMIT,
    DATADIR_BINDING_PATH,
    enforceCloudflareCapacity,
    verifyRuntimeCorpus,
} from './verify-runtime-corpus.mjs';
import { authorRuntimeJavascriptModules } from './runtime-javascript-modules.mjs';

const short = '1234567890ab';
const sourceCommit = `${short}${'c'.repeat(28)}`;
const cargoLockSha256 = 'b'.repeat(64);
const nativeContentSha256 = 'd'.repeat(64);
const workerVersionId = '12345678-90ab-cdef-8123-456789abcdef';
const contract = Object.freeze({ contentSchema: 2, netProtocol: 28, ticketSchema: 3 });

function sha(bytes) {
    return createHash('sha256').update(bytes).digest('hex');
}

const demoObjects = Object.freeze([
    { path: 'audio/assets/music.opus', kind: 'asset', bytes: Buffer.from('demo opus') },
    { path: 'audio/bundles/common.bin', kind: 'asset', bytes: Buffer.from('demo audio bundle') },
    { path: 'audio/menu-w30-1234567890ab.rhmission.zst', kind: 'shipping', bytes: Buffer.from('demo audio index') },
    { path: 'missions/leicester-w30-234567890abc.rhmission.zst', kind: 'shipping', bytes: Buffer.from('demo mission') },
    { path: 'rhs/robin-w30-34567890abcd.rhmission.zst', kind: 'shipping', bytes: Buffer.from('demo rhs') },
    { path: 'terrain/leicester-w30-4567890abcde.rhmission.zst', kind: 'shipping', bytes: Buffer.from('demo terrain') },
]);

async function demoPackage({ edition = 'demo' } = {}) {
    const root = await mkdtemp(resolve(tmpdir(), 'demo-web-content-'));
    const data = resolve(root, 'Data');
    const datadir = Buffer.from('exact Demo package');
    await mkdir(data, { recursive: true });
    await writeFile(resolve(data, 'datadir.bin'), datadir);
    for (const object of demoObjects) {
        const path = resolve(data, object.path);
        await mkdir(resolve(path, '..'), { recursive: true });
        await writeFile(path, object.bytes);
    }
    const document = {
        schema: 2,
        edition,
        engine_version: sourceCommit,
        native_content_sha256: nativeContentSha256,
        datadir: { path: 'datadir.bin', byte_length: datadir.length, sha256: sha(datadir) },
        files: demoObjects.map(object => ({
            path: object.path,
            kind: object.kind,
            byte_length: object.bytes.length,
            sha256: sha(object.bytes),
        })),
    };
    await writeFile(resolve(data, 'robinhood-web-content.json'), JSON.stringify(document));
    return { root, datadir, document };
}

function buildManifest(wasm, wasmGzip, demo, buildShort, javascriptModules) {
    return {
        commit: `${buildShort}${'c'.repeat(28)}`,
        short: buildShort,
        builtAt: '2026-08-30T12:00:00Z',
        netProtocol: contract.netProtocol,
        ticketSchema: contract.ticketSchema,
        multiplayerContent: {
            schema: contract.contentSchema,
            demo: {
                url: `https://robinhood.phiresky.xyz/${DEMO_PATH}`,
                sha256: sha(demo),
                byteLength: demo.length,
                nativeContentSha256,
            },
            full: { manifestSha256: 'e'.repeat(64) },
        },
        files: {
            js: 'robin.js', jsGzip: 'robin.js.gz', wasm: 'robin_bg.wasm', wasmGzip: 'robin_bg.wasm.gz',
        },
        javascriptModules,
        sha256: { wasm: sha(wasm), wasmGzip: sha(wasmGzip) },
    };
}

async function runtimeAddition(buildShort = short) {
    const root = await mkdtemp(resolve(tmpdir(), 'runtime-addition-'));
    const build = resolve(root, 'wasm', buildShort);
    const wasm = Buffer.from('wasm fixture');
    const wasmGzip = Buffer.from('compressed wasm fixture');
    const demo = Buffer.from('exact Demo package');
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
    const javascriptModules = await authorRuntimeJavascriptModules(build);
    const document = `${JSON.stringify(
        buildManifest(wasm, wasmGzip, demo, buildShort, javascriptModules),
        null,
        2,
    )}\n`;
    await writeFile(resolve(build, 'manifest.json'), document);
    await writeFile(resolve(root, 'wasm/latest.json'), document);
    return root;
}

async function rewriteRuntimeManifest(root, mutate) {
    const latest = resolve(root, 'wasm/latest.json');
    const document = JSON.parse(await readFile(latest, 'utf8'));
    mutate(document);
    const bytes = `${JSON.stringify(document, null, 2)}\n`;
    await writeFile(resolve(root, 'wasm', document.short, 'manifest.json'), bytes);
    await writeFile(latest, bytes);
}

async function datadirRelease() {
    const root = await mkdtemp(resolve(tmpdir(), 'datadir-release-'));
    const source = await demoPackage();
    const dist = resolve(root, 'datadir-dist');
    const inventory = resolve(root, 'datadir-inventory.json');
    const authority = resolve(root, 'datadir-authority.json');
    const receipt = resolve(root, 'datadir-deployment.json');
    await assembleDatadirCorpus({ existing: null, demo: source.root, output: dist });
    const authored = await writeDatadirReleaseAuthority({
        root: dist, sourceCommit, cargoLockSha256, inventoryPath: inventory, authorityPath: authority,
    });
    const deployed = await writeDatadirDeploymentReceipt({
        authorityPath: authority, workerVersionId, output: receipt,
    });
    return { root, source, dist, inventory, authority, receipt, authored, deployed };
}

test('Demo converter package and standalone datadir corpus are exact and Demo-only', async t => {
    const demo = await demoPackage();
    const full = await demoPackage({ edition: 'full' });
    const extra = await demoPackage();
    const linked = await demoPackage();
    t.after(() => Promise.all([demo, full, extra, linked]
        .map(value => rm(value.root, { recursive: true, force: true }))));

    assert.equal((await verifyDemoWebContentPackage(demo.root)).copyEntries.length, demoObjects.length + 2);
    await assert.rejects(verifyDemoWebContentPackage(full.root), /edition/u);
    await writeFile(resolve(extra.root, 'Data/unlisted.zst'), 'extra');
    await assert.rejects(verifyDemoWebContentPackage(extra.root), /closure mismatch/u);
    await symlink(resolve(linked.root, 'Data/datadir.bin'), resolve(linked.root, 'Data/alias.bin'));
    await assert.rejects(verifyDemoWebContentPackage(linked.root), /symlink/u);
});

test('standalone datadir authority, inventory, and deployment receipt form one closed chain', async t => {
    const release = await datadirRelease();
    t.after(() => rm(release.root, { recursive: true, force: true }));

    const corpus = await verifyDatadirCorpus(release.dist);
    assert.equal(corpus.assetCount, 8);
    assert.equal(corpus.demo.datadir_url, `https://robinhood.phiresky.xyz/${DEMO_PATH}`);
    const authority = await verifyDatadirReleaseAuthority({
        root: release.dist,
        inventoryPath: release.inventory,
        authorityPath: release.authority,
        expectedAuthoritySha256: release.authored.authoritySha256,
    });
    const receipt = await verifyDatadirDeploymentReceipt({
        authorityPath: release.authority,
        receiptPath: release.receipt,
        expectedReceiptSha256: release.deployed.receiptSha256,
    });
    assert.equal(authority.inventorySha256, receipt.receipt.inventory_sha256);
    assert.equal(receipt.receipt.worker_name, 'robinhood-datadir-assets');
    assert.equal(receipt.receipt.worker_version_id, workerVersionId);
});

test('runtime is wasm-only and requires the exact external authority plus deployed receipt', async t => {
    const addition = await runtimeAddition();
    const release = await datadirRelease();
    const root = await mkdtemp(resolve(tmpdir(), 'runtime-release-'));
    const output = resolve(root, 'runtime-dist');
    t.after(() => Promise.all([
        rm(addition, { recursive: true, force: true }),
        rm(release.root, { recursive: true, force: true }),
        rm(root, { recursive: true, force: true }),
    ]));

    assert.equal((await verifyRuntimeCorpus(addition, { addition: true, expectedContract: contract })).assetCount, 10);
    const assembled = await assembleRuntimeCorpus({
        existing: null,
        addition,
        datadirAuthority: release.authority,
        datadirDeployment: release.receipt,
        output,
    });
    assert.equal(assembled.assetCount, 11);
    await assert.rejects(verifyRuntimeCorpus(output), /requires the external datadir/u);
    assert.equal((await verifyRuntimeCorpus(output, {
        datadirAuthorityPath: release.authority,
    })).datadirDeployment.receipt.worker_version_id, workerVersionId);
    const runtimeHandle = await open(output, 'r');
    try {
        const capability = `/proc/self/fd/${runtimeHandle.fd}/.`;
        assert.equal((await verifyRuntimeCorpus(capability, {
            datadirAuthorityPath: release.authority,
        })).datadirDeployment.receipt.worker_version_id, workerVersionId);
        for (const malformed of [
            `/proc/self/fd/${runtimeHandle.fd}`,
            `/proc/self/fd/0${runtimeHandle.fd}/.`,
            `/proc/self/fd/${runtimeHandle.fd}//wasm`,
            `/proc/self/fd/${runtimeHandle.fd}/wasm/..`,
        ]) {
            await assert.rejects(verifyRuntimeCorpus(malformed, {
                datadirAuthorityPath: release.authority,
            }), /retained-directory capability is not canonical/u);
        }
    } finally {
        await runtimeHandle.close();
    }
    const runtimeAlias = resolve(root, 'runtime-alias');
    await symlink(output, runtimeAlias);
    await assert.rejects(verifyRuntimeCorpus(runtimeAlias, {
        datadirAuthorityPath: release.authority,
    }), /runtime corpus is not a real directory/u);
    await assert.rejects(readFile(resolve(output, DEMO_PATH)), /ENOENT/u);
    assert.equal(
        await readFile(resolve(output, DATADIR_BINDING_PATH), 'utf8'),
        await readFile(release.receipt, 'utf8'),
    );
});

test('runtime update retains immutable wasm versions and never imports the datadir corpus', async t => {
    const first = await runtimeAddition('111111111111');
    const second = await runtimeAddition('222222222222');
    const release = await datadirRelease();
    const root = await mkdtemp(resolve(tmpdir(), 'runtime-update-'));
    const original = resolve(root, 'original');
    const updated = resolve(root, 'updated');
    t.after(() => Promise.all([
        rm(first, { recursive: true, force: true }), rm(second, { recursive: true, force: true }),
        rm(release.root, { recursive: true, force: true }), rm(root, { recursive: true, force: true }),
    ]));
    await assembleRuntimeCorpus({
        existing: null, addition: first, datadirAuthority: release.authority,
        datadirDeployment: release.receipt, output: original,
    });
    const result = await assembleRuntimeCorpus({
        existing: original, addition: second, datadirAuthority: release.authority,
        datadirDeployment: release.receipt, output: updated,
    });
    assert.equal(result.assetCount, 20);
    assert.equal(JSON.parse(await readFile(resolve(updated, 'wasm/latest.json'))).short, '222222222222');
    assert.equal(JSON.parse(await readFile(resolve(updated, 'wasm/111111111111/manifest.json'))).short, '111111111111');
    await assert.rejects(readFile(resolve(updated, DEMO_CONTENT_MANIFEST_PATH)), /ENOENT/u);
});

test('capacity gate enforces Cloudflare asset count and object limits', () => {
    assert.throws(() => enforceCloudflareCapacity(Array.from(
        { length: CLOUDFLARE_FREE_ASSET_LIMIT + 1 },
        (_, index) => ({ path: `wasm/${short}/${index}.json`, size: 1 }),
    )), /20001 assets/u);
    assert.throws(() => enforceCloudflareCapacity([{
        path: `wasm/${short}/large.wasm`, size: CLOUDFLARE_ASSET_BYTES_LIMIT + 1,
    }]), /25 MiB/u);
});

test('runtime rejects any direct datadir byte path', async t => {
    const addition = await runtimeAddition();
    t.after(() => rm(addition, { recursive: true, force: true }));
    await mkdir(resolve(addition, DEMO_ROOT), { recursive: true });
    await writeFile(resolve(addition, DEMO_PATH), 'forbidden');
    await assert.rejects(verifyRuntimeCorpus(addition, { addition: true }), /non-public extension|non-wasm/u);
});

test('runtime manifest rejects missing, substituted, extra, and tampered JavaScript modules', async t => {
    const missing = await runtimeAddition('333333333333');
    const substituted = await runtimeAddition('444444444444');
    const extra = await runtimeAddition('555555555555');
    const tampered = await runtimeAddition('666666666666');
    t.after(() => Promise.all([missing, substituted, extra, tampered]
        .map(root => rm(root, { recursive: true, force: true }))));

    const modulePath = 'snippets/robin_rs-build/js/browser_identity_client.js';
    await rm(resolve(missing, 'wasm/333333333333', modulePath));
    await assert.rejects(
        verifyRuntimeCorpus(missing, { addition: true }),
        /imports missing module|exactly one browser_identity_client\.js; found 0/u,
    );

    await rewriteRuntimeManifest(substituted, manifest => {
        manifest.javascriptModules[0].path = 'snippets/robin_rs-build/js/substitute.js';
    });
    await assert.rejects(
        verifyRuntimeCorpus(substituted, { addition: true }),
        /do not match the exact imported module closure/u,
    );

    await rewriteRuntimeManifest(extra, manifest => {
        manifest.javascriptModules.push({
            path: 'snippets/robin_rs-build/js/extra.js',
            byteLength: 1,
            sha256: '1'.repeat(64),
        });
    });
    await assert.rejects(
        verifyRuntimeCorpus(extra, { addition: true }),
        /not unique and UTF-8 sorted|do not match the exact imported module closure/u,
    );

    await writeFile(
        resolve(tampered, 'wasm/666666666666', modulePath),
        'export const requestIdentity = () => "tampered";\n',
    );
    await assert.rejects(
        verifyRuntimeCorpus(tampered, { addition: true }),
        /do not match the exact imported module closure/u,
    );
});

test('runtime manifest rejects private vault and orphan JavaScript modules', async t => {
    const vault = await runtimeAddition('777777777777');
    const orphan = await runtimeAddition('888888888888');
    t.after(() => Promise.all([vault, orphan]
        .map(root => rm(root, { recursive: true, force: true }))));

    await writeFile(
        resolve(vault, 'wasm/777777777777/snippets/robin_rs-build/js/browser_identity_vault.js'),
        'export const privateKey = "forbidden";\n',
    );
    await assert.rejects(
        verifyRuntimeCorpus(vault, { addition: true }),
        /forbidden identity vault/u,
    );

    await writeFile(
        resolve(orphan, 'wasm/888888888888/orphan.js'),
        'export const orphan = true;\n',
    );
    await assert.rejects(
        verifyRuntimeCorpus(orphan, { addition: true }),
        /orphan modules/u,
    );
});
