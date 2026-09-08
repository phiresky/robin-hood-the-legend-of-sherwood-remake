import { createHash } from 'node:crypto';
import { lstat, readFile, readdir } from 'node:fs/promises';
import { relative, resolve, sep } from 'node:path';
import { pathToFileURL } from 'node:url';
import { validateDatadirHeaders } from './verify-cloudflare-deployment.mjs';

export const CLOUDFLARE_FREE_ASSET_LIMIT = 20_000;
export const CLOUDFLARE_ASSET_BYTES_LIMIT = 25 * 1024 * 1024;
export const DEMO_ROOT = 'datadirs/demo-leicester';
export const DEMO_PATH = `${DEMO_ROOT}/v8-web-opus-q80.rhdata.zst`;
export const DEMO_CONTENT_MANIFEST_PATH = `${DEMO_ROOT}/robinhood-web-content.json`;
export const WEB_CONTENT_MANIFEST_NAME = 'robinhood-web-content.json';
export const WEB_CONTENT_MANIFEST_SCHEMA = 2;
const DIGEST = /^[0-9a-f]{64}$/u;
const FULL_COMMIT = /^[0-9a-f]{40}$/u;

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

function parseWebContentManifest(bytes, label) {
    const manifest = json(bytes, label);
    exactKeys(manifest, [
        'schema', 'edition', 'engine_version', 'native_content_sha256', 'datadir', 'files',
    ], label);
    exact(manifest.schema, WEB_CONTENT_MANIFEST_SCHEMA, `${label} schema`);
    exact(manifest.edition, 'demo', `${label} edition`);
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
        content_manifest_url: `https://robinhood.phiresky.xyz/${DEMO_CONTENT_MANIFEST_PATH}`,
        content_manifest_sha256: manifestSha256,
        datadir_url: `https://robinhood.phiresky.xyz/${DEMO_PATH}`,
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
    const actual = new Set(facts.map(file => file.path));
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

/** Validate the standalone, deployable datadir Worker corpus. */
export async function verifyDatadirCorpus(directory) {
    const root = resolve(directory);
    const facts = await regularFilesBelow(root, 'datadir corpus');
    const metrics = enforceCloudflareCapacity(facts, 'datadir corpus');
    const files = new Set(facts.map(file => file.path));
    if (!files.has('_headers') || !files.has(DEMO_CONTENT_MANIFEST_PATH) || !files.has(DEMO_PATH)) {
        throw new Error('datadir corpus requires _headers and the complete Demo content closure');
    }
    validateDatadirHeaders(await readFile(resolve(root, '_headers'), 'utf8'));
    const allowedPrefix = `${DEMO_ROOT}/`;
    const invalid = [...files].filter(path => path !== '_headers' && !path.startsWith(allowedPrefix));
    if (invalid.length > 0) throw new Error(`datadir corpus contains non-Demo paths: ${invalid.join(', ')}`);

    const manifestBytes = await readFile(resolve(root, DEMO_CONTENT_MANIFEST_PATH));
    const manifest = parseWebContentManifest(manifestBytes, DEMO_CONTENT_MANIFEST_PATH);
    const expected = new Set([
        DEMO_CONTENT_MANIFEST_PATH,
        DEMO_PATH,
        ...manifest.files.map(file => `${DEMO_ROOT}/${file.path}`),
    ]);
    const actual = [...files].filter(path => path !== '_headers');
    const missing = [...expected].filter(path => !files.has(path));
    const extra = actual.filter(path => !expected.has(path));
    if (missing.length > 0 || extra.length > 0) {
        throw new Error(`datadir corpus closure mismatch; missing [${missing.join(', ')}], extra [${extra.join(', ')}]`);
    }
    await verifyContentObject(root, DEMO_PATH, manifest.datadir, 'datadir corpus');
    for (const file of manifest.files) {
        await verifyContentObject(root, `${DEMO_ROOT}/${file.path}`, file, 'datadir corpus');
    }
    return {
        ...metrics,
        demo: demoDetails(manifest, sha256(manifestBytes)),
        manifest,
        stableClosure: JSON.stringify({
            schema: manifest.schema,
            edition: manifest.edition,
            native_content_sha256: manifest.native_content_sha256,
            datadir: manifest.datadir,
            files: manifest.files,
        }),
    };
}

async function main() {
    const [directory, extra] = process.argv.slice(2);
    if (directory === undefined || extra !== undefined) {
        throw new Error('usage: node scripts/verify-datadir-corpus.mjs DIRECTORY');
    }
    const metrics = await verifyDatadirCorpus(directory);
    console.log(`verified standalone Demo datadir corpus: ${metrics.assetCount} assets, ${metrics.totalBytes} bytes`);
}

const invokedPath = process.argv[1];
if (invokedPath !== undefined && import.meta.url === pathToFileURL(resolve(invokedPath)).href) {
    main().catch(error => {
        console.error(error instanceof Error ? error.message : String(error));
        process.exitCode = 1;
    });
}
