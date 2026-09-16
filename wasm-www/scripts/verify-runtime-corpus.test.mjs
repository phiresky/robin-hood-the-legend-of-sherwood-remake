import { addFullReplayContent } from './add-full-replay-content.mjs';
import { admissionFixture } from './replay-admission-wasm-fixture.mjs';
import assert from 'node:assert/strict';
import { brotliCompressSync } from 'node:zlib';
import { writeBrotliWasm } from './compress-runtime-wasm.mjs';
import { createHash } from 'node:crypto';
import { chmod, cp, lstat, mkdir, mkdtemp, open, readdir, readFile, rm, symlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { resolve } from 'node:path';
import test from 'node:test';
import { assembleDatadirCorpus } from './assemble-datadir-corpus.mjs';
import { assembleRuntimeCorpus } from './assemble-runtime-corpus.mjs';
import { readDatadirRelease, writeDatadirRelease } from './datadir-release.mjs';
import { stageCloudflareHeaders } from './stage-cloudflare-headers.mjs';
import {
    DEMO_CONTENT_MANIFEST_PATH,
    DEMO_PARENT_ROOT,
    DEMO_PATH,
    DEMO_ROOT,
    verifyDatadirCorpus,
    verifyDemoWebContentPackage,
} from './verify-datadir-corpus.mjs';
import {
    CLOUDFLARE_ASSET_BYTES_LIMIT,
    CLOUDFLARE_FREE_ASSET_LIMIT,
    LEGACY_DATADIR_RECEIPT_PATH,
    enforceCloudflareCapacity,
    verifyRuntimeCorpus,
} from './verify-runtime-corpus.mjs';
import { authorRuntimeJavascriptModules } from './runtime-javascript-modules.mjs';

const short = '1234567890ab';
const sourceCommit = `${short}${'c'.repeat(28)}`;
const nativeContentSha256 = 'd'.repeat(64);
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

async function demoPackage({ edition = 'demo', datadirText = 'exact Demo package' } = {}) {
    const root = await mkdtemp(resolve(tmpdir(), 'demo-web-content-'));
    const data = resolve(root, 'Data');
    const datadir = Buffer.from(datadirText);
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
    await writeFile(resolve(build, 'Data/AudioDurations.json'), '{}');
    await writeFile(resolve(build, 'Data/Interface/Fonts/arial.ttf'), 'font fixture');
    await writeFile(resolve(build, 'Data/Interface/UI/marker.png'), 'image fixture');
    await writeFile(resolve(build, 'preload-assets.json'), `${JSON.stringify([
        { path: 'Data/AudioDurations.json', url: 'Data/AudioDurations.json' },
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

test('datadir release describes the current Demo object of an assembled corpus', async t => {
    const source = await demoPackage();
    const root = await mkdtemp(resolve(tmpdir(), 'datadir-release-'));
    t.after(() => Promise.all([source.root, root].map(path => rm(path, { recursive: true, force: true }))));
    const dist = resolve(root, 'datadir-dist');
    await assembleDatadirCorpus({ existing: null, demo: source.root, output: dist });
    const corpus = await verifyDatadirCorpus(dist);
    assert.equal(corpus.assetCount, 8);

    const output = resolve(root, 'datadir-release.json');
    await writeDatadirRelease({ root: dist, output, retainedGenerations: [] });
    assert.deepEqual(await readDatadirRelease(output), {
        schema: 1,
        url: `https://robinhood.phiresky.xyz/${DEMO_PATH}`,
        sha256: sha(source.datadir),
        byte_length: source.datadir.length,
        native_content_sha256: nativeContentSha256,
    });
    await assert.rejects(writeDatadirRelease({ root: dist, output, retainedGenerations: [] }), /EEXIST/u);
    await rm(resolve(dist, DEMO_PATH));
    await assert.rejects(writeDatadirRelease({ root: dist, output: resolve(root, 'other.json'), retainedGenerations: [] }));
});

test('runtime corpus is wasm-only and assembles without any datadir document', async t => {
    const addition = await runtimeAddition();
    const root = await mkdtemp(resolve(tmpdir(), 'runtime-release-'));
    const output = resolve(root, 'runtime-dist');
    t.after(() => Promise.all([
        rm(addition, { recursive: true, force: true }),
        rm(root, { recursive: true, force: true }),
    ]));

    assert.equal((await verifyRuntimeCorpus(addition, { addition: true })).assetCount, 11);
    await assert.rejects(verifyRuntimeCorpus(addition), /requires _headers/u);
    const assembled = await assembleRuntimeCorpus({ existing: null, addition, output });
    assert.equal(assembled.assetCount, 11);
    assert.equal((await verifyRuntimeCorpus(output)).latest.short, short);
    const runtimeHandle = await open(output, 'r');
    try {
        const capability = `/proc/self/fd/${runtimeHandle.fd}/.`;
        assert.equal((await verifyRuntimeCorpus(capability)).latest.short, short);
        for (const malformed of [
            `/proc/self/fd/${runtimeHandle.fd}`,
            `/proc/self/fd/0${runtimeHandle.fd}/.`,
            `/proc/self/fd/${runtimeHandle.fd}//wasm`,
            `/proc/self/fd/${runtimeHandle.fd}/wasm/..`,
        ]) {
            await assert.rejects(verifyRuntimeCorpus(malformed), /retained-directory capability is not canonical/u);
        }
    } finally {
        await runtimeHandle.close();
    }
    const runtimeAlias = resolve(root, 'runtime-alias');
    await symlink(output, runtimeAlias);
    await assert.rejects(verifyRuntimeCorpus(runtimeAlias), /runtime corpus is not a real directory/u);
    await assert.rejects(readFile(resolve(output, DEMO_PATH)), /ENOENT/u);
    await assert.rejects(readFile(resolve(output, LEGACY_DATADIR_RECEIPT_PATH)), /ENOENT/u);
});

test('runtime update retains immutable wasm versions and drops the legacy datadir receipt', async t => {
    const first = await runtimeAddition('111111111111');
    const second = await runtimeAddition('222222222222');
    const root = await mkdtemp(resolve(tmpdir(), 'runtime-update-'));
    const original = resolve(root, 'original');
    const updated = resolve(root, 'updated');
    t.after(() => Promise.all([
        rm(first, { recursive: true, force: true }), rm(second, { recursive: true, force: true }),
        rm(root, { recursive: true, force: true }),
    ]));
    await assembleRuntimeCorpus({ existing: null, addition: first, output: original });
    // The deployed corpus predates the current header policy (no CORP) and
    // still carries the removed datadir receipt. Assembly discards both, so
    // they must not block the release, while strict verification rejects them.
    const legacyHeaders = (await readFile(resolve(original, '_headers'), 'utf8'))
        .replace('  Cross-Origin-Resource-Policy: same-origin\n', '');
    await rm(resolve(original, '_headers'));
    await writeFile(resolve(original, '_headers'), legacyHeaders);
    await assert.rejects(verifyRuntimeCorpus(original), /Cross-Origin-Resource-Policy/u);
    await writeFile(resolve(original, LEGACY_DATADIR_RECEIPT_PATH), '{}');
    await assert.rejects(verifyRuntimeCorpus(original, { priorUpload: false }), /undeclared or non-wasm path/u);
    const result = await assembleRuntimeCorpus({ existing: original, addition: second, output: updated });
    assert.equal(result.assetCount, 21);
    assert.match(await readFile(resolve(updated, '_headers'), 'utf8'), /Cross-Origin-Resource-Policy: same-origin/u);
    assert.equal(JSON.parse(await readFile(resolve(updated, 'wasm/latest.json'))).short, '222222222222');
    assert.equal(JSON.parse(await readFile(resolve(updated, 'wasm/111111111111/manifest.json'))).short, '111111111111');
    await assert.rejects(readFile(resolve(updated, DEMO_CONTENT_MANIFEST_PATH)), /ENOENT/u);
    await assert.rejects(readFile(resolve(updated, LEGACY_DATADIR_RECEIPT_PATH)), /ENOENT/u);
});

async function setTreeModes(path, directoryMode, fileMode) {
    const facts = await lstat(path);
    if (!facts.isDirectory()) {
        await chmod(path, fileMode);
        return;
    }
    // Writable first so children can be changed, then the requested mode.
    await chmod(path, 0o700);
    for (const entry of await readdir(path)) await setTreeModes(resolve(path, entry), directoryMode, fileMode);
    await chmod(path, directoryMode);
}

async function treeModes(path, prefix = '') {
    const modes = [[prefix, (await lstat(path)).mode & 0o777]];
    if ((await lstat(path)).isDirectory()) {
        for (const entry of (await readdir(path)).sort()) modes.push(...await treeModes(resolve(path, entry), `${prefix}/${entry}`));
    }
    return modes;
}

test('runtime update assembles onto a read-only archived corpus without making it writable', async t => {
    const first = await runtimeAddition('111111111111');
    const second = await runtimeAddition('222222222222');
    const root = await mkdtemp(resolve(tmpdir(), 'runtime-readonly-'));
    const original = resolve(root, 'original');
    const updated = resolve(root, 'updated');
    t.after(async () => {
        await setTreeModes(original, 0o755, 0o644).catch(() => {});
        await Promise.all([
            rm(first, { recursive: true, force: true }), rm(second, { recursive: true, force: true }),
            rm(root, { recursive: true, force: true }),
        ]);
    });
    await assembleRuntimeCorpus({ existing: null, addition: first, output: original });
    await writeFile(resolve(original, LEGACY_DATADIR_RECEIPT_PATH), '{}');
    await setTreeModes(original, 0o555, 0o444);
    const archivedModes = await treeModes(original);

    const result = await assembleRuntimeCorpus({ existing: original, addition: second, output: updated });
    assert.equal(result.assetCount, 21);
    assert.equal(JSON.parse(await readFile(resolve(updated, 'wasm/latest.json'))).short, '222222222222');
    assert.equal(JSON.parse(await readFile(resolve(updated, 'wasm/111111111111/manifest.json'))).short, '111111111111');
    assert.deepEqual(await treeModes(original), archivedModes);
    assert.equal(JSON.parse(await readFile(resolve(original, 'wasm/latest.json'))).short, '111111111111');

    // A failed assembly removes its read-only-derived staging tree.
    const failed = resolve(root, 'failed');
    await assert.rejects(assembleRuntimeCorpus({ existing: original, addition: first, output: failed }));
    assert.deepEqual((await readdir(root)).filter(name => name.includes('assembling')), []);
    assert.deepEqual(await treeModes(original), archivedModes);
});

/** A published corpus holding only an earlier generation at the parent root. */
async function retainedGenerationCorpus() {
    const retained = await demoPackage({ datadirText: 'retained Demo package' });
    const root = await mkdtemp(resolve(tmpdir(), 'datadir-retained-'));
    const prior = resolve(root, 'prior');
    const manifestBytes = await readFile(resolve(retained.root, 'Data/robinhood-web-content.json'));
    const generation = {
        root: DEMO_PARENT_ROOT,
        datadirPath: `${DEMO_PARENT_ROOT}/v8-web-opus-q80.rhdata.zst`,
        contentManifestPath: `${DEMO_PARENT_ROOT}/robinhood-web-content.json`,
        contentManifestSha256: sha(manifestBytes),
        datadirSha256: sha(retained.datadir),
        datadirByteLength: retained.datadir.length,
        nativeContentSha256,
    };
    await mkdir(resolve(prior, DEMO_PARENT_ROOT), { recursive: true });
    await writeFile(resolve(prior, generation.datadirPath), retained.datadir);
    await writeFile(resolve(prior, generation.contentManifestPath), manifestBytes);
    for (const object of demoObjects) {
        const path = resolve(prior, DEMO_PARENT_ROOT, object.path);
        await mkdir(resolve(path, '..'), { recursive: true });
        await writeFile(path, object.bytes);
    }
    await stageCloudflareHeaders('datadir', prior);
    return { root, retained, prior, retainedGenerations: [generation] };
}

test('datadir update adds the current generation beside every retained object', async t => {
    const fixture = await retainedGenerationCorpus();
    const current = await demoPackage();
    t.after(() => Promise.all([fixture.root, fixture.retained.root, current.root]
        .map(path => rm(path, { recursive: true, force: true }))));
    const { retainedGenerations } = fixture;
    // The converter's unpublished dependency plan never enters the corpus.
    await writeFile(resolve(current.root, 'Data/conversion-plan.json'), '{}');

    await assert.rejects(verifyDatadirCorpus(fixture.prior, { retainedGenerations }), /closure mismatch; missing/u);
    assert.equal((await verifyDatadirCorpus(fixture.prior, { retainedGenerations, requireCurrent: false })).demo, undefined);
    await assert.rejects(verifyDatadirCorpus(fixture.prior, { requireCurrent: false }), /extra/u);

    const output = resolve(fixture.root, 'updated');
    const result = await assembleDatadirCorpus({ existing: fixture.prior, demo: current.root, output, retainedGenerations });
    assert.equal(result.demo.datadir_url, `https://robinhood.phiresky.xyz/${DEMO_PATH}`);
    assert.deepEqual(await readFile(resolve(output, retainedGenerations[0].datadirPath)), fixture.retained.datadir);
    assert.deepEqual(await readFile(resolve(output, DEMO_PATH)), current.datadir);
    await assert.rejects(readFile(resolve(output, DEMO_ROOT, 'conversion-plan.json')), /ENOENT/u);

    await rm(resolve(output, retainedGenerations[0].datadirPath));
    await assert.rejects(
        verifyDatadirCorpus(output, { retainedGenerations }),
        /retained Demo generation .* closure mismatch; missing/u,
    );
});

test('runtime update keeps builds pinned to a retained generation and binds latest to the current one', async t => {
    const fixture = await retainedGenerationCorpus();
    const old = await runtimeAddition('777777777777');
    const addition = await runtimeAddition('888888888888');
    t.after(() => Promise.all([fixture.root, fixture.retained.root, old, addition]
        .map(path => rm(path, { recursive: true, force: true }))));
    const { retainedGenerations } = fixture;
    const [generation] = retainedGenerations;

    await rewriteRuntimeManifest(old, document => {
        document.multiplayerContent.demo = {
            url: `https://robinhood.phiresky.xyz/${generation.datadirPath}`,
            sha256: generation.datadirSha256,
            byteLength: generation.datadirByteLength,
            nativeContentSha256: generation.nativeContentSha256,
        };
    });
    // The prior upload's latest still names the generation the datadir
    // rebuild has just retained.
    const priorRuntime = resolve(fixture.root, 'prior-runtime');
    await cp(resolve(old, 'wasm'), resolve(priorRuntime, 'wasm'), { recursive: true });
    await stageCloudflareHeaders('runtime', priorRuntime);
    await verifyRuntimeCorpus(priorRuntime, { retainedGenerations, priorUpload: true });
    await assert.rejects(verifyRuntimeCorpus(priorRuntime, { retainedGenerations }), /select the current Demo datadir generation/u);
    await assert.rejects(verifyRuntimeCorpus(priorRuntime, { priorUpload: true }), /Demo URL/u);

    const updated = resolve(fixture.root, 'updated-runtime');
    const result = await assembleRuntimeCorpus({ existing: priorRuntime, addition, output: updated, retainedGenerations });
    assert.equal(result.latest.short, '888888888888');
    assert.equal(
        JSON.parse(await readFile(resolve(updated, 'wasm/777777777777/manifest.json'))).multiplayerContent.demo.url,
        `https://robinhood.phiresky.xyz/${generation.datadirPath}`,
    );

    // A retained build must keep the exact bytes it was published with.
    const tampered = resolve(fixture.root, 'tampered-runtime');
    await cp(priorRuntime, tampered, { recursive: true });
    await rewriteRuntimeManifest(tampered, document => { document.multiplayerContent.demo.byteLength += 1; });
    await assert.rejects(verifyRuntimeCorpus(tampered, { retainedGenerations, priorUpload: true }), /retained Demo byteLength/u);
});

test('two builds cannot name different bytes under the current Demo generation path', async t => {
    const first = await runtimeAddition('aaaaaaaaaaaa');
    const second = await runtimeAddition('bbbbbbbbbbbb');
    const root = await mkdtemp(resolve(tmpdir(), 'runtime-generation-'));
    t.after(() => Promise.all([first, second, root].map(path => rm(path, { recursive: true, force: true }))));
    await rewriteRuntimeManifest(second, document => { document.multiplayerContent.demo.sha256 = 'f'.repeat(64); });
    const original = resolve(root, 'original');
    await assembleRuntimeCorpus({ existing: null, addition: first, output: original });
    await assert.rejects(
        assembleRuntimeCorpus({ existing: original, addition: second, output: resolve(root, 'updated') }),
        /needs a new generation directory/u,
    );
});

test('builds published before audio timings joined the preload closure remain valid', async t => {
    const root = await runtimeAddition();
    t.after(() => rm(root, { recursive: true, force: true }));
    const build = resolve(root, 'wasm', short);
    await rm(resolve(build, 'Data/AudioDurations.json'));
    await writeFile(resolve(build, 'preload-assets.json'), JSON.stringify([
        { path: 'Data/Interface/Fonts/arial.ttf', url: 'Data/Interface/Fonts/arial.ttf' },
        { path: 'Data/Interface/UI/marker.png', url: 'Data/Interface/UI/marker.png' },
    ]));
    await verifyRuntimeCorpus(root, { addition: true });
    await writeFile(resolve(build, 'preload-assets.json'), JSON.stringify([
        { path: 'Data/Interface/UI/marker.png', url: 'Data/Interface/UI/marker.png' },
        { path: 'Data/Interface/Fonts/arial.ttf', url: 'Data/Interface/Fonts/arial.ttf' },
    ]));
    await assert.rejects(verifyRuntimeCorpus(root, { addition: true }), /begin with the optional audio timings and the font/u);
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

test('optional Brotli sidecars are packaged, digest-bound and decode to the declared WASM', async t => {
    const root = await runtimeAddition();
    t.after(() => rm(root, { recursive: true, force: true }));
    const path = resolve(root, 'wasm', short, 'robin_bg.wasm');
    await writeBrotliWasm(path);
    const bytes = await readFile(`${path}.br`);
    await rewriteRuntimeManifest(root, manifest => {
        manifest.files.wasmBrotli = 'robin_bg.wasm.br';
        manifest.sha256.wasmBrotli = sha(bytes);
    });
    await verifyRuntimeCorpus(root, { addition: true });
    const wrong = brotliCompressSync(Buffer.from('different wasm'));
    await writeFile(`${path}.br`, wrong);
    await assert.rejects(verifyRuntimeCorpus(root, { addition: true }), /wasmBrotli digest/u);
    await rewriteRuntimeManifest(root, manifest => { manifest.sha256.wasmBrotli = sha(wrong); });
    await assert.rejects(verifyRuntimeCorpus(root, { addition: true }), /Brotli decoded wasm digest/u);
});

test('current runtime admission is independently hashed, memory-capped and included in its JS closure', async t => {
    const root = await runtimeAddition();
    t.after(() => rm(root, { recursive: true, force: true }));
    const build = resolve(root, 'wasm', short);
    const js = Buffer.from('export default async function init() {}\nexport function validate_compact_replay() {}\n');
    const wasm = admissionFixture();
    await writeFile(resolve(build, 'replay_admission.js'), js);
    await writeFile(resolve(build, 'replay_admission_bg.wasm'), wasm);
    const claims = await authorRuntimeJavascriptModules(build, { replayAdmission: true });
    await rewriteRuntimeManifest(root, manifest => {
        manifest.files.replayAdmissionJs = 'replay_admission.js';
        manifest.files.replayAdmissionWasm = 'replay_admission_bg.wasm';
        manifest.sha256.replayAdmissionJs = sha(js);
        manifest.sha256.replayAdmissionWasm = sha(wasm);
        manifest.javascriptModules = claims;
    });
    await verifyRuntimeCorpus(root, { addition: true });
    const unbounded = admissionFixture({ max: null });
    await writeFile(resolve(build, 'replay_admission_bg.wasm'), unbounded);
    await assert.rejects(verifyRuntimeCorpus(root, { addition: true }), /replayAdmissionWasm digest/);
    await rewriteRuntimeManifest(root, manifest => { manifest.sha256.replayAdmissionWasm = sha(unbounded); });
    await assert.rejects(verifyRuntimeCorpus(root, { addition: true }), /memory capped/);
});


test('hosted Full replay content preserves Demo data and rejects tampered bindings', async t => {
    const demo = await demoPackage();
    const full = await demoPackage({ edition: 'full' });
    const root = await mkdtemp(resolve(tmpdir(), 'full-replay-content-'));
    t.after(() => Promise.all([demo.root, full.root, root].map(path => rm(path, { recursive: true, force: true }))));
    const corpus = resolve(root, 'corpus');
    await assembleDatadirCorpus({ existing: null, demo: demo.root, output: corpus });
    await addFullReplayContent({ corpus, source: full.root, build: '1699bc12ffb8', retainedGenerations: [] });
    const bindingPath = resolve(corpus, 'datadirs/replays/1699bc12ffb8.json');
    const binding = JSON.parse(await readFile(bindingPath, 'utf8'));
    assert.equal(binding.sha256, full.document.datadir.sha256);
    assert.equal((await verifyDatadirCorpus(corpus)).demo.datadir_sha256, demo.document.datadir.sha256);
    await writeFile(bindingPath, JSON.stringify({ ...binding, sha256: '0'.repeat(64) }));
    await assert.rejects(verifyDatadirCorpus(corpus), /digest|sha256/u);
});
