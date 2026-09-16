import { createHash } from 'node:crypto';
import { lstat, readFile, readdir } from 'node:fs/promises';
import { relative, resolve, sep } from 'node:path';
import { pathToFileURL } from 'node:url';
import { validateDatadirHeaders } from './verify-cloudflare-deployment.mjs';

export const CLOUDFLARE_FREE_ASSET_LIMIT = 20_000;
export const CLOUDFLARE_ASSET_BYTES_LIMIT = 25 * 1024 * 1024;
export const DEMO_PARENT_ROOT = 'datadirs/demo-leicester';
// Every Demo datadir generation is served `immutable` under fixed names
// (`robinhood-web-content.json`, the datadir object), so a new native datadir
// format gets its own directory instead of replacing published bytes.
// The directory also changes when a rebuild changes bytes without a format
// bump (v16r2: keyed minimaps, shared-layer locale fallback).
// v17: native shipping datadir format 17 (match-gated VQ sprite coding).
// v17r2: same format, opusenc/libopus 1.6.1 audio and per-datadir music remasters.
// v18: native shipping datadir format 18 (browser-decoded AVIF web images).
export const DEMO_ROOT = `${DEMO_PARENT_ROOT}/v18`;
export const DEMO_PATH = `${DEMO_ROOT}/v18-web-opus-q80.rhdata.zst`;
export const DEMO_CONTENT_MANIFEST_PATH = `${DEMO_ROOT}/robinhood-web-content.json`;
export const WEB_CONTENT_MANIFEST_NAME = 'robinhood-web-content.json';
export const WEB_CONTENT_MANIFEST_SCHEMA = 2;
const DEMO_PUBLIC_ORIGIN = 'https://robinhood.phiresky.xyz';
const CONVERSION_PLAN_PATH = 'Data/conversion-plan.json';
const DIGEST = /^[0-9a-f]{64}$/u;
const FULL_COMMIT = /^[0-9a-f]{40}$/u;

/**
 * Previously published Demo generations. Older wasm builds and replay links
 * pin these exact bytes, so every datadir corpus must keep serving them.
 */
export const RETAINED_DEMO_GENERATIONS = Object.freeze([
    Object.freeze({
        // Native shipping datadir format 15 (`RHDDNA15`), published 2026-08-31.
        root: DEMO_PARENT_ROOT,
        datadirPath: `${DEMO_PARENT_ROOT}/v8-web-opus-q80.rhdata.zst`,
        contentManifestPath: `${DEMO_PARENT_ROOT}/robinhood-web-content.json`,
        contentManifestSha256: 'cd3fb3b379079237c0c480e2332b6bf381e0b7403eed8f834857011af0a9fb20',
        datadirSha256: '4b6e0fcba222b5df2b640ecf630d5101b94c4eae68bd0f9518f96658e28fd742',
        datadirByteLength: 7_966_731,
        nativeContentSha256: 'b86d7c960d960f33a504905bc3b6e7d7dd0b168944fa34fcfb55787c34b3f8b3',
    }),
    Object.freeze({
        // Native shipping datadir format 16 (`RHDDNA16`), published 2026-09-13.
        root: `${DEMO_PARENT_ROOT}/v16`,
        datadirPath: `${DEMO_PARENT_ROOT}/v16/v16-web-opus-q80.rhdata.zst`,
        contentManifestPath: `${DEMO_PARENT_ROOT}/v16/robinhood-web-content.json`,
        contentManifestSha256: '81927ca5e49586bb16d3850cfc7c592efd29898aab7db5e3caf65b814a5150bc',
        datadirSha256: '1028ec06a0b8f96bcb47695b9acc795450396debb631eedaa6e21e0ac9c1121d',
        datadirByteLength: 3_699_421,
        nativeContentSha256: 'b86d7c960d960f33a504905bc3b6e7d7dd0b168944fa34fcfb55787c34b3f8b3',
    }),
    Object.freeze({
        // Native shipping datadir format 16 (`RHDDNA16`), second build
        // (keyed minimaps, shared-layer locale fallback), published 2026-09-14.
        root: `${DEMO_PARENT_ROOT}/v16r2`,
        datadirPath: `${DEMO_PARENT_ROOT}/v16r2/v16r2-web-opus-q80.rhdata.zst`,
        contentManifestPath: `${DEMO_PARENT_ROOT}/v16r2/robinhood-web-content.json`,
        contentManifestSha256: '2c2bb47b00c4d394ef4390f9bced575567258008cf810f4b6258942d61c7cf5b',
        datadirSha256: '33263c2902435fc5ede73d6582a9d71639cbde073027de3aed8647e7f87aed4d',
        datadirByteLength: 3_698_976,
        nativeContentSha256: 'b86d7c960d960f33a504905bc3b6e7d7dd0b168944fa34fcfb55787c34b3f8b3',
    }),
    Object.freeze({
        // Native shipping datadir format 17 (`RHDDNA17`), published 2026-09-14.
        root: `${DEMO_PARENT_ROOT}/v17`,
        datadirPath: `${DEMO_PARENT_ROOT}/v17/v17-web-opus-q80.rhdata.zst`,
        contentManifestPath: `${DEMO_PARENT_ROOT}/v17/robinhood-web-content.json`,
        contentManifestSha256: 'e62b0204ed930612aa560981fa9cc5198cdfbca08d566968242ff848c2c4c8af',
        datadirSha256: '8518ada3bd1981a2f0f053abc2a8954119e365efe1fcd4677dc0ab28ecf04f27',
        datadirByteLength: 3_699_000,
        nativeContentSha256: 'b86d7c960d960f33a504905bc3b6e7d7dd0b168944fa34fcfb55787c34b3f8b3',
    }),
    Object.freeze({
        // Native shipping datadir format 17 (`RHDDNA17`), second build (opusenc on
        // libopus 1.6.1, per-datadir music remasters), published 2026-09-15.
        root: `${DEMO_PARENT_ROOT}/v17r2`,
        datadirPath: `${DEMO_PARENT_ROOT}/v17r2/v17r2-web-opus-q80.rhdata.zst`,
        contentManifestPath: `${DEMO_PARENT_ROOT}/v17r2/robinhood-web-content.json`,
        contentManifestSha256: 'd62fce960fdd2895c45ec5359019e630f505f1fe7a64786d3807a8e701b8953b',
        datadirSha256: '12ef4c0caf2934eeee40d9eb89b33350a52837599f00d55ca1e99947f30c3162',
        datadirByteLength: 3_697_444,
        nativeContentSha256: 'b86d7c960d960f33a504905bc3b6e7d7dd0b168944fa34fcfb55787c34b3f8b3',
    }),
]);

/** The authority/receipt `demo` identity of a retained generation. */
export function retainedDemoDetails(generation) {
    return {
        content_manifest_url: `${DEMO_PUBLIC_ORIGIN}/${generation.contentManifestPath}`,
        content_manifest_sha256: generation.contentManifestSha256,
        datadir_url: `${DEMO_PUBLIC_ORIGIN}/${generation.datadirPath}`,
        datadir_sha256: generation.datadirSha256,
        datadir_byte_length: generation.datadirByteLength,
        native_content_sha256: generation.nativeContentSha256,
    };
}

function exactKeys(value, keys, label) {
    if (value === null || typeof value !== 'object' || Array.isArray(value)) {
        throw new Error(`${label} must be an object`);
    }
    const actual = Object.keys(value).sort();
    const expected = [...keys].sort();
    if (JSON.stringify(actual) !== JSON.stringify(expected)) {
        throw new Error(`${label} has unexpected keys: ${actual.join(', ')}`);
    }
}

function exact(value, expected, label) {
    if (value !== expected) throw new Error(`${label} must be ${JSON.stringify(expected)}`);
}

function positiveInteger(value, label) {
    if (!Number.isSafeInteger(value) || value <= 0) throw new Error(`${label} must be a positive integer`);
}

function sha256(bytes) {
    return createHash('sha256').update(bytes).digest('hex');
}

async function regularFilesBelow(root, label) {
    const result = [];
    const canonicalPaths = new Map();
    async function visit(directory) {
        for (const entry of await readdir(directory, { withFileTypes: true })) {
            const path = resolve(directory, entry.name);
            const logical = relative(root, path).split(sep).join('/');
            if (entry.isSymbolicLink()) throw new Error(`${label} contains a symlink: ${logical}`);
            const folded = logical.toLowerCase();
            const previous = canonicalPaths.get(folded);
            if (previous !== undefined) {
                throw new Error(`${label} paths collide case-insensitively: ${previous} and ${logical}`);
            }
            canonicalPaths.set(folded, logical);
            if (entry.isDirectory()) await visit(path);
            else if (entry.isFile()) result.push({ path: logical, size: (await lstat(path)).size });
            else throw new Error(`${label} contains a non-regular entry: ${logical}`);
        }
    }
    const facts = await lstat(root).catch(() => undefined);
    if (facts === undefined || !facts.isDirectory() || facts.isSymbolicLink()) {
        throw new Error(`${label} is not a real directory: ${root}`);
    }
    await visit(root);
    return result.sort((left, right) => Buffer.compare(Buffer.from(left.path), Buffer.from(right.path)));
}

function enforceCloudflareCapacity(files, label) {
    const served = files.filter(file => file.path !== '_headers');
    if (served.length > CLOUDFLARE_FREE_ASSET_LIMIT) {
        throw new Error(`${label} has ${served.length} assets; Cloudflare Free permits ${CLOUDFLARE_FREE_ASSET_LIMIT}`);
    }
    for (const file of served) {
        if (!Number.isSafeInteger(file.size) || file.size < 0) {
            throw new Error(`${label} asset has an invalid length: ${file.path}`);
        }
        if (file.size > CLOUDFLARE_ASSET_BYTES_LIMIT) {
            throw new Error(`${label} asset exceeds Cloudflare's 25 MiB limit: ${file.path} (${file.size} bytes)`);
        }
    }
    return {
        assetCount: served.length,
        totalBytes: served.reduce((sum, file) => sum + BigInt(file.size), 0n),
    };
}

function json(bytes, label) {
    try {
        return JSON.parse(bytes.toString('utf8'));
    } catch (error) {
        throw new Error(`${label} is not valid JSON: ${error instanceof Error ? error.message : String(error)}`);
    }
}

function canonicalWebContentManifest(manifest) {
    return {
        schema: manifest.schema,
        edition: manifest.edition,
        engine_version: manifest.engine_version,
        native_content_sha256: manifest.native_content_sha256,
        datadir: {
            path: manifest.datadir.path,
            byte_length: manifest.datadir.byte_length,
            sha256: manifest.datadir.sha256,
        },
        files: manifest.files.map(file => ({
            path: file.path,
            kind: file.kind,
            byte_length: file.byte_length,
            sha256: file.sha256,
        })),
    };
}

function validateWebContentPath(path, label) {
    if (!/^[A-Za-z0-9._/-]+$/u.test(path) || path.startsWith('/') || path.endsWith('/')
        || path.includes('\\')
        || path.split('/').some(segment => segment === '' || segment === '.' || segment === '..')) {
        throw new Error(`${label} is not a canonical relative path: ${path}`);
    }
    if (!/^(?:missions|rhs|terrain)\/[A-Za-z0-9._-]+\.rhmission\.zst$/u.test(path)
        && !/^audio\/[A-Za-z0-9._-]+\.rhmission\.zst$/u.test(path)
        && !/^audio\/assets\/[A-Za-z0-9._-]+\.opus$/u.test(path)
        && !/^audio\/bundles\/[A-Za-z0-9._-]+\.bin$/u.test(path)) {
        throw new Error(`${label} is not a supported split Demo path: ${path}`);
    }
}

export function parseWebContentManifest(bytes, label, edition = 'demo') {
    const manifest = json(bytes, label);
    exactKeys(manifest, [
        'schema', 'edition', 'engine_version', 'native_content_sha256', 'datadir', 'files',
    ], label);
    exact(manifest.schema, WEB_CONTENT_MANIFEST_SCHEMA, `${label} schema`);
    exact(manifest.edition, edition, `${label} edition`);
    if (!FULL_COMMIT.test(manifest.engine_version)) throw new Error(`${label} has an invalid engine_version`);
    if (!DIGEST.test(manifest.native_content_sha256)) {
        throw new Error(`${label} has an invalid native_content_sha256`);
    }
    exactKeys(manifest.datadir, ['path', 'byte_length', 'sha256'], `${label} datadir`);
    exact(manifest.datadir.path, 'datadir.bin', `${label} datadir path`);
    positiveInteger(manifest.datadir.byte_length, `${label} datadir byte_length`);
    if (!DIGEST.test(manifest.datadir.sha256)) throw new Error(`${label} has an invalid datadir digest`);
    if (!Array.isArray(manifest.files) || manifest.files.length === 0) {
        throw new Error(`${label} files must be a non-empty array`);
    }

    const seen = new Map([['datadir.bin', 'datadir.bin']]);
    const buckets = new Set();
    let previous;
    for (const [index, file] of manifest.files.entries()) {
        const fileLabel = `${label} files[${index}]`;
        exactKeys(file, ['path', 'kind', 'byte_length', 'sha256'], fileLabel);
        if (typeof file.path !== 'string') throw new Error(`${fileLabel} path must be a string`);
        validateWebContentPath(file.path, `${fileLabel} path`);
        if (previous !== undefined && previous >= file.path) {
            throw new Error(`${label} files must be strictly sorted by path`);
        }
        previous = file.path;
        const folded = file.path.toLowerCase();
        const collision = seen.get(folded);
        if (collision !== undefined) {
            throw new Error(`${label} paths collide case-insensitively: ${collision} and ${file.path}`);
        }
        seen.set(folded, file.path);
        const expectedKind = file.path.startsWith('audio/assets/')
            || file.path.startsWith('audio/bundles/') ? 'asset' : 'shipping';
        exact(file.kind, expectedKind, `${fileLabel} kind`);
        positiveInteger(file.byte_length, `${fileLabel} byte_length`);
        if (!DIGEST.test(file.sha256)) throw new Error(`${fileLabel} has an invalid digest`);
        buckets.add(file.path.split('/')[0]);
    }
    for (const bucket of ['audio', 'missions', 'rhs', 'terrain']) {
        if (!buckets.has(bucket)) throw new Error(`${label} is missing its ${bucket}/ split closure`);
    }
    if (!Buffer.from(JSON.stringify(canonicalWebContentManifest(manifest))).equals(bytes)) {
        throw new Error(`${label} is not canonical typed JSON`);
    }
    return manifest;
}

async function verifyContentObject(root, path, expected, label) {
    const facts = await lstat(resolve(root, path)).catch(() => undefined);
    if (facts === undefined || !facts.isFile() || facts.isSymbolicLink()) {
        throw new Error(`${label} is missing or not a regular file: ${path}`);
    }
    exact(facts.size, expected.byte_length, `${label} byte length for ${path}`);
    if (facts.size > CLOUDFLARE_ASSET_BYTES_LIMIT) {
        throw new Error(`${label} object exceeds Cloudflare's 25 MiB limit: ${path} (${facts.size} bytes)`);
    }
    exact(sha256(await readFile(resolve(root, path))), expected.sha256, `${label} digest for ${path}`);
}

function demoDetails(manifest, manifestSha256) {
    return {
        content_manifest_url: `${DEMO_PUBLIC_ORIGIN}/${DEMO_CONTENT_MANIFEST_PATH}`,
        content_manifest_sha256: manifestSha256,
        datadir_url: `${DEMO_PUBLIC_ORIGIN}/${DEMO_PATH}`,
        datadir_sha256: manifest.datadir.sha256,
        datadir_byte_length: manifest.datadir.byte_length,
        native_content_sha256: manifest.native_content_sha256,
    };
}

/** Validate an untouched canonical converter output root containing `Data/`. */
export async function verifyDemoWebContentPackage(directory) {
    const root = resolve(directory);
    const facts = await regularFilesBelow(root, 'Demo converter output');
    enforceCloudflareCapacity(facts, 'Demo converter output');
    // The converter's dependency plan is an unpublished build record; the Rust
    // packager excludes it from the web content manifest as well.
    const actual = new Set(facts.map(file => file.path).filter(path => path !== CONVERSION_PLAN_PATH));
    const manifestPath = `Data/${WEB_CONTENT_MANIFEST_NAME}`;
    if (!actual.has(manifestPath)) throw new Error(`Demo converter output is missing ${manifestPath}`);
    const manifestBytes = await readFile(resolve(root, manifestPath));
    const manifest = parseWebContentManifest(manifestBytes, manifestPath);
    const expected = new Set([
        manifestPath,
        `Data/${manifest.datadir.path}`,
        ...manifest.files.map(file => `Data/${file.path}`),
    ]);
    const missing = [...expected].filter(path => !actual.has(path));
    const extra = [...actual].filter(path => !expected.has(path));
    if (missing.length > 0 || extra.length > 0) {
        throw new Error(`Demo converter output closure mismatch; missing [${missing.join(', ')}], extra [${extra.join(', ')}]`);
    }
    await verifyContentObject(root, `Data/${manifest.datadir.path}`, manifest.datadir, 'Demo converter output');
    for (const file of manifest.files) {
        await verifyContentObject(root, `Data/${file.path}`, file, 'Demo converter output');
    }
    return {
        root,
        manifest,
        stableClosure: JSON.stringify({
            schema: manifest.schema,
            edition: manifest.edition,
            native_content_sha256: manifest.native_content_sha256,
            datadir: manifest.datadir,
            files: manifest.files,
        }),
        demo: demoDetails(manifest, sha256(manifestBytes)),
        copyEntries: [
            { source: `Data/${manifest.datadir.path}`, destination: DEMO_PATH },
            { source: manifestPath, destination: DEMO_CONTENT_MANIFEST_PATH },
            ...manifest.files.map(file => ({
                source: `Data/${file.path}`,
                destination: `${DEMO_ROOT}/${file.path}`,
            })),
        ],
    };
}

/** Verify one generation's exact content manifest closure inside a corpus. */
async function verifyDemoGeneration(root, files, generation, label) {
    const closure = [generation.contentManifestPath, generation.datadirPath];
    const absent = closure.filter(path => !files.has(path));
    if (absent.length > 0) {
        throw new Error(`${label} closure mismatch; missing [${absent.join(', ')}], extra []`);
    }
    const manifestBytes = await readFile(resolve(root, generation.contentManifestPath));
    const manifest = parseWebContentManifest(manifestBytes, generation.contentManifestPath);
    const paths = [...closure, ...manifest.files.map(file => `${generation.root}/${file.path}`)];
    const missing = paths.filter(path => !files.has(path));
    if (missing.length > 0) {
        throw new Error(`${label} closure mismatch; missing [${missing.join(', ')}], extra []`);
    }
    await verifyContentObject(root, generation.datadirPath, manifest.datadir, label);
    for (const file of manifest.files) {
        await verifyContentObject(root, `${generation.root}/${file.path}`, file, label);
    }
    return {
        manifest,
        manifestSha256: sha256(manifestBytes),
        paths,
        stableClosure: JSON.stringify({
            schema: manifest.schema,
            edition: manifest.edition,
            native_content_sha256: manifest.native_content_sha256,
            datadir: manifest.datadir,
            files: manifest.files,
        }),
    };
}

/**
 * Validate the standalone, deployable datadir Worker corpus: every retained
 * generation byte-for-byte plus the current generation, and nothing else.
 * `requireCurrent: false` admits a prior corpus that predates the current
 * generation (the input of an update); its result then has no `demo`.
 */
export async function verifyDatadirCorpus(directory, { retainedGenerations = [], requireCurrent = true } = {}) {
    const root = resolve(directory);
    const facts = await regularFilesBelow(root, 'datadir corpus');
    const metrics = enforceCloudflareCapacity(facts, 'datadir corpus');
    const files = new Set(facts.map(file => file.path));
    if (!files.has('_headers')) throw new Error('datadir corpus requires _headers');
    validateDatadirHeaders(await readFile(resolve(root, '_headers'), 'utf8'));
    const allowedPrefix = `${DEMO_PARENT_ROOT}/`;
    const invalid = [...files].filter(path => path !== '_headers' && !path.startsWith(allowedPrefix) && !path.startsWith('datadirs/full/') && !path.startsWith('datadirs/replays/'));
    if (invalid.length > 0) throw new Error(`datadir corpus contains non-Demo paths: ${invalid.join(', ')}`);

    const expected = new Set();
    for (const generation of retainedGenerations) {
        const label = `retained Demo generation ${generation.datadirPath}`;
        const retained = await verifyDemoGeneration(root, files, generation, label);
        exact(retained.manifestSha256, generation.contentManifestSha256, `${label} content manifest digest`);
        exact(retained.manifest.datadir.sha256, generation.datadirSha256, `${label} datadir digest`);
        exact(retained.manifest.datadir.byte_length, generation.datadirByteLength, `${label} datadir byte length`);
        exact(
            retained.manifest.native_content_sha256,
            generation.nativeContentSha256,
            `${label} native content identity`,
        );
        for (const path of retained.paths) expected.add(path);
    }
    let current;
    if (requireCurrent || files.has(DEMO_CONTENT_MANIFEST_PATH) || files.has(DEMO_PATH)) {
        current = await verifyDemoGeneration(root, files, {
            root: DEMO_ROOT, datadirPath: DEMO_PATH, contentManifestPath: DEMO_CONTENT_MANIFEST_PATH,
        }, 'datadir corpus');
        for (const path of current.paths) expected.add(path);
    }
    for (const path of files) {
        const match = /^datadirs\/full\/([0-9a-f]{64})\/robinhood-web-content\.json$/u.exec(path);
        if (match === null) continue;
        const bytes = await readFile(resolve(root, path));
        exact(sha256(bytes), match[1], 'Full content manifest address');
        const manifest = parseWebContentManifest(bytes, path, 'full');
        expected.add(path);
        const base = path.slice(0, path.lastIndexOf('/'));
        for (const file of [manifest.datadir, ...manifest.files]) {
            const object = `${base}/${file.path}`;
            await verifyContentObject(root, object, file, 'Full replay content');
            expected.add(object);
        }
    }
    for (const path of files) {
        if (!/^datadirs\/replays\/[0-9a-f]{12}\.json$/u.test(path)) continue;
        const binding = json(await readFile(resolve(root, path)), path);
        exactKeys(binding, ['url', 'sha256', 'byteLength'], path);
        const match = /^\/datadirs\/full\/([0-9a-f]{64})\/datadir\.bin$/u.exec(binding.url);
        if (match === null || !expected.has(binding.url.slice(1))) throw new Error(`${path} references missing Full data`);
        await verifyContentObject(root, binding.url.slice(1), {
            sha256: binding.sha256, byte_length: binding.byteLength,
        }, path);
        expected.add(path);
    }
    const extra = [...files].filter(path => path !== '_headers' && !expected.has(path));
    if (extra.length > 0) {
        throw new Error(`datadir corpus closure mismatch; missing [], extra [${extra.join(', ')}]`);
    }
    return {
        ...metrics,
        demo: current === undefined ? undefined : demoDetails(current.manifest, current.manifestSha256),
        manifest: current?.manifest,
        stableClosure: current?.stableClosure,
    };
}

async function main() {
    const [directory, extra] = process.argv.slice(2);
    if (directory === undefined || extra !== undefined) {
        throw new Error('usage: node scripts/verify-datadir-corpus.mjs DIRECTORY');
    }
    const metrics = await verifyDatadirCorpus(directory, { retainedGenerations: RETAINED_DEMO_GENERATIONS });
    console.log(`verified standalone Demo datadir corpus: ${metrics.assetCount} assets, ${metrics.totalBytes} bytes`);
}

const invokedPath = process.argv[1];
if (invokedPath !== undefined && import.meta.url === pathToFileURL(resolve(invokedPath)).href) {
    main().catch(error => {
        console.error(error instanceof Error ? error.message : String(error));
        process.exitCode = 1;
    });
}
