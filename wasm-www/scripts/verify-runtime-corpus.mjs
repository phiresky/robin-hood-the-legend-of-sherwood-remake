import { brotliDecompressSync } from 'node:zlib';
import { createHash } from 'node:crypto';
import { lstat, readFile, readdir } from 'node:fs/promises';
import { extname, relative, resolve, sep } from 'node:path';
import { pathToFileURL } from 'node:url';
import { verifyDatadirDeploymentReceipt } from './datadir-release-authority.mjs';
import {
    CLOUDFLARE_ASSET_BYTES_LIMIT,
    CLOUDFLARE_FREE_ASSET_LIMIT,
    DEMO_PATH,
} from './verify-datadir-corpus.mjs';
import { DEPLOYMENT, validateRuntimeHeaders } from './verify-cloudflare-deployment.mjs';
import { verifyRuntimeJavascriptModules } from './runtime-javascript-modules.mjs';
import { verifyRuntimeSourceContract } from './verify-runtime-source-contract.mjs';

export { CLOUDFLARE_ASSET_BYTES_LIMIT, CLOUDFLARE_FREE_ASSET_LIMIT, DEMO_PATH };
export const DATADIR_BINDING_PATH = 'wasm/datadir-deployment.json';
const DIGEST = /^[0-9a-f]{64}$/u;
const SHORT_COMMIT = /^[0-9a-f]{12}$/u;
const FULL_COMMIT = /^[0-9a-f]{40}$/u;
const ALLOWED_EXTENSIONS = new Set(['.br', '.gz', '.js', '.json', '.png', '.ttf', '.wasm']);
const RETAINED_DIRECTORY_PREFIX = '/proc/self/fd/';
const RETAINED_DIRECTORY_PATH = /^\/proc\/self\/fd\/([^/]+)(.*)$/u;
const CANONICAL_FILE_DESCRIPTOR = /^(?:0|[1-9][0-9]*)$/u;

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

function digest(bytes) {
    return createHash('sha256').update(bytes).digest('hex');
}

async function regularFilesBelow(root) {
    const result = [];
    const canonicalPaths = new Map();
    async function visit(directory) {
        for (const entry of await readdir(directory, { withFileTypes: true })) {
            const path = resolve(directory, entry.name);
            const logical = relative(root, path).split(sep).join('/');
            if (entry.isSymbolicLink()) throw new Error(`runtime corpus contains a symlink: ${logical}`);
            const folded = logical.toLowerCase();
            const previous = canonicalPaths.get(folded);
            if (previous !== undefined) {
                throw new Error(`runtime corpus paths collide case-insensitively: ${previous} and ${logical}`);
            }
            canonicalPaths.set(folded, logical);
            if (entry.isDirectory()) await visit(path);
            else if (entry.isFile()) result.push({ path: logical, size: (await lstat(path)).size });
            else throw new Error(`runtime corpus contains a non-regular entry: ${logical}`);
        }
    }
    const facts = await lstat(root).catch(() => undefined);
    if (facts === undefined || !facts.isDirectory() || facts.isSymbolicLink()) {
        throw new Error(`runtime corpus is not a real directory: ${root}`);
    }
    await visit(root);
    return result.sort((left, right) => Buffer.compare(Buffer.from(left.path), Buffer.from(right.path)));
}

function runtimeRoot(directory) {
    if (typeof directory === 'string' && directory.startsWith(RETAINED_DIRECTORY_PREFIX)) {
        const retained = RETAINED_DIRECTORY_PATH.exec(directory);
        if (retained === null || !CANONICAL_FILE_DESCRIPTOR.test(retained[1])) {
            throw new Error(`runtime corpus retained-directory capability is not canonical: ${directory}`);
        }
        if (retained[2] === '/.') {
            // path.resolve() removes the final "/." and turns the retained
            // directory capability back into the /proc symlink. Preserve the
            // exact spelling so lstat() inspects the opened directory itself.
            return directory;
        }
        const resolved = resolve(directory);
        if (retained[2] === '' || resolved !== directory
            || !resolved.startsWith(`${RETAINED_DIRECTORY_PREFIX}${retained[1]}/`)) {
            throw new Error(`runtime corpus retained-directory capability is not canonical: ${directory}`);
        }
        return resolved;
    }
    return resolve(directory);
}

export function enforceCloudflareCapacity(files) {
    const served = files.filter(file => file.path !== '_headers');
    if (served.length > CLOUDFLARE_FREE_ASSET_LIMIT) {
        throw new Error(`runtime corpus has ${served.length} assets; Cloudflare Free permits ${CLOUDFLARE_FREE_ASSET_LIMIT}`);
    }
    for (const file of served) {
        if (!Number.isSafeInteger(file.size) || file.size < 0) {
            throw new Error(`runtime asset has an invalid length: ${file.path}`);
        }
        if (file.size > CLOUDFLARE_ASSET_BYTES_LIMIT) {
            throw new Error(`runtime asset exceeds Cloudflare's 25 MiB limit: ${file.path} (${file.size} bytes)`);
        }
    }
    return {
        assetCount: served.length,
        totalBytes: served.reduce((sum, file) => sum + BigInt(file.size), 0n),
    };
}

function validatePath(path, addition) {
    if (!/^[\x21-\x7e]+$/u.test(path) || path.startsWith('/') || path.includes('\\')
        || path.split('/').some(segment => segment === '' || segment === '.' || segment === '..')) {
        throw new Error(`runtime corpus has a non-canonical path: ${path}`);
    }
    if (path === '_headers') {
        if (addition) throw new Error('runtime addition must not carry deployment control headers');
        return;
    }
    if (!ALLOWED_EXTENSIONS.has(extname(path).toLowerCase())) {
        throw new Error(`runtime corpus has a non-public extension: ${path}`);
    }
    if (path === 'wasm/latest.json' || path === DATADIR_BINDING_PATH) return;
    if (!/^wasm\/[0-9a-f]{12}\//u.test(path)) {
        throw new Error(`runtime corpus has an undeclared or non-wasm path: ${path}`);
    }
    if (path.endsWith('.map') || path.endsWith('.d.ts') || /(?:^|\/)replays?(?:\/|$)/iu.test(path)) {
        throw new Error(`runtime corpus contains a source or replay path: ${path}`);
    }
}

function json(bytes, label) {
    try {
        return JSON.parse(bytes.toString('utf8'));
    } catch (error) {
        throw new Error(`${label} is not valid JSON: ${error instanceof Error ? error.message : String(error)}`);
    }
}

async function validateManifest(root, manifestPath, files, demoAuthority, addition, expectedContract) {
    const manifestBytes = await readFile(resolve(root, manifestPath));
    const manifest = json(manifestBytes, manifestPath);
    exactKeys(manifest, [
        'builtAt', 'commit', 'files', 'javascriptModules', 'multiplayerContent', 'netProtocol',
        'sha256', 'short', 'ticketSchema',
    ], manifestPath);
    if (!FULL_COMMIT.test(manifest.commit) || !SHORT_COMMIT.test(manifest.short)
        || !manifest.commit.startsWith(manifest.short)) {
        throw new Error(`${manifestPath} has an invalid commit/short binding`);
    }
    exact(manifestPath, `wasm/${manifest.short}/manifest.json`, `${manifestPath} location`);
    if (typeof manifest.builtAt !== 'string'
        || !/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$/u.test(manifest.builtAt)) {
        throw new Error(`${manifestPath} has an invalid builtAt timestamp`);
    }
    positiveInteger(manifest.netProtocol, `${manifestPath} netProtocol`);
    positiveInteger(manifest.ticketSchema, `${manifestPath} ticketSchema`);
    if (expectedContract !== undefined) {
        exact(manifest.netProtocol, expectedContract.netProtocol, `${manifestPath} current netProtocol`);
        exact(manifest.ticketSchema, expectedContract.ticketSchema, `${manifestPath} current ticketSchema`);
    }

    const hasBrotli = Object.hasOwn(manifest.files, 'wasmBrotli');
    exactKeys(manifest.files, ['js', 'jsGzip', 'wasm', 'wasmGzip', ...(hasBrotli ? ['wasmBrotli'] : [])], `${manifestPath} files`);
    const expectedFiles = {
        js: 'robin.js', jsGzip: 'robin.js.gz', wasm: 'robin_bg.wasm', wasmGzip: 'robin_bg.wasm.gz',
        ...(hasBrotli ? { wasmBrotli: 'robin_bg.wasm.br' } : {}),
    };
    for (const [field, name] of Object.entries(expectedFiles)) {
        exact(manifest.files[field], name, `${manifestPath} ${field} path`);
        if (!files.has(`wasm/${manifest.short}/${name}`)) throw new Error(`${manifestPath} references missing ${name}`);
    }
    const digestFields = ['wasm', 'wasmGzip', ...(hasBrotli ? ['wasmBrotli'] : [])];
    exactKeys(manifest.sha256, digestFields, `${manifestPath} sha256`);
    for (const field of digestFields) {
        if (!DIGEST.test(manifest.sha256[field])) throw new Error(`${manifestPath} has an invalid ${field} digest`);
        const actual = digest(await readFile(resolve(root, `wasm/${manifest.short}/${manifest.files[field]}`)));
        exact(actual, manifest.sha256[field], `${manifestPath} ${field} digest`);
    }
    if (hasBrotli) {
        const decoded = brotliDecompressSync(await readFile(resolve(root, `wasm/${manifest.short}/robin_bg.wasm.br`)), { maxOutputLength: CLOUDFLARE_ASSET_BYTES_LIMIT });
        exact(digest(decoded), manifest.sha256.wasm, `${manifestPath} Brotli decoded wasm digest`);
    }
    await verifyRuntimeJavascriptModules(
        resolve(root, 'wasm', manifest.short),
        manifest.javascriptModules,
    );

    exactKeys(manifest.multiplayerContent, ['demo', 'full', 'schema'], `${manifestPath} multiplayerContent`);
    positiveInteger(manifest.multiplayerContent.schema, `${manifestPath} multiplayerContent schema`);
    if (expectedContract !== undefined) {
        exact(manifest.multiplayerContent.schema, expectedContract.contentSchema, `${manifestPath} current content schema`);
    }
    const demo = manifest.multiplayerContent.demo;
    exactKeys(demo, ['byteLength', 'nativeContentSha256', 'sha256', 'url'], `${manifestPath} Demo content`);
    exact(demo.url, `${DEPLOYMENT.publicOrigin}/${DEMO_PATH}`, `${manifestPath} Demo URL`);
    if (!DIGEST.test(demo.sha256) || !DIGEST.test(demo.nativeContentSha256)) {
        throw new Error(`${manifestPath} has an invalid Demo content digest`);
    }
    positiveInteger(demo.byteLength, `${manifestPath} Demo byteLength`);
    if (demo.byteLength > CLOUDFLARE_ASSET_BYTES_LIMIT) {
        throw new Error(`${manifestPath} Demo byteLength exceeds Cloudflare's 25 MiB limit`);
    }
    if (!addition) {
        exact(demo.byteLength, demoAuthority.datadir_byte_length, `${manifestPath} deployed Demo byteLength`);
        exact(demo.sha256, demoAuthority.datadir_sha256, `${manifestPath} deployed Demo digest`);
        exact(
            demo.nativeContentSha256,
            demoAuthority.native_content_sha256,
            `${manifestPath} deployed Demo native content identity`,
        );
    }
    if (manifest.multiplayerContent.full !== null) {
        exactKeys(manifest.multiplayerContent.full, ['manifestSha256'], `${manifestPath} Full content`);
        if (!DIGEST.test(manifest.multiplayerContent.full.manifestSha256)) {
            throw new Error(`${manifestPath} has an invalid Full manifest digest`);
        }
    }
    return { bytes: manifestBytes, manifest };
}

async function validateBuildClosure(root, manifest, files) {
    const prefix = `wasm/${manifest.short}/`;
    const preloadPath = `${prefix}preload-assets.json`;
    if (!files.has(preloadPath)) throw new Error(`${prefix} is missing preload-assets.json`);
    const preload = json(await readFile(resolve(root, preloadPath)), preloadPath);
    if (!Array.isArray(preload) || preload.length < 2) {
        throw new Error(`${preloadPath} must contain a font and at least one UI image`);
    }
    const auxiliary = [];
    for (const [index, entry] of preload.entries()) {
        exactKeys(entry, ['path', 'url'], `${preloadPath}[${index}]`);
        if (typeof entry.path !== 'string'
            || !/^Data\/(?:Interface\/Fonts\/arial\.ttf|Interface\/UI\/[A-Za-z0-9_.-]+\.png)$/u.test(entry.path)) {
            throw new Error(`${preloadPath}[${index}] has a non-canonical path`);
        }
        exact(entry.url, entry.path, `${preloadPath}[${index}] URL`);
        auxiliary.push(entry.path);
    }
    const sorted = [...new Set(auxiliary)].sort((left, right) => Buffer.compare(Buffer.from(left), Buffer.from(right)));
    if (auxiliary[0] !== 'Data/Interface/Fonts/arial.ttf'
        || JSON.stringify(auxiliary) !== JSON.stringify(sorted)) {
        throw new Error(`${preloadPath} must be unique, sorted, and begin with the required font`);
    }
    const expected = new Set([
        `${prefix}manifest.json`, preloadPath,
        ...Object.values(manifest.files).map(name => `${prefix}${name}`),
        ...manifest.javascriptModules.map(module => `${prefix}${module.path}`),
        ...auxiliary.map(path => `${prefix}${path}`),
    ]);
    const actual = [...files].filter(path => path.startsWith(prefix));
    const missing = [...expected].filter(path => !files.has(path));
    const extra = actual.filter(path => !expected.has(path));
    if (missing.length > 0 || extra.length > 0) {
        throw new Error(`${prefix} closure mismatch; missing [${missing.join(', ')}], extra [${extra.join(', ')}]`);
    }
}

export async function verifyRuntimeCorpus(directory, {
    addition = false,
    expectedContract,
    datadirAuthorityPath,
} = {}) {
    const root = runtimeRoot(directory);
    const facts = await regularFilesBelow(root);
    const metrics = enforceCloudflareCapacity(facts);
    const files = new Set(facts.map(file => file.path));
    for (const file of facts) validatePath(file.path, addition);

    let datadirDeployment;
    if (addition) {
        if (files.has('_headers') || files.has(DATADIR_BINDING_PATH)) {
            throw new Error('runtime addition contains deployment controls or a datadir deployment receipt');
        }
    } else {
        if (!files.has('_headers') || !files.has(DATADIR_BINDING_PATH)) {
            throw new Error(`complete runtime corpus requires _headers and ${DATADIR_BINDING_PATH}`);
        }
        if (datadirAuthorityPath === undefined) {
            throw new Error('complete runtime verification requires the external datadir release authority');
        }
        validateRuntimeHeaders(await readFile(resolve(root, '_headers'), 'utf8'));
        datadirDeployment = await verifyDatadirDeploymentReceipt({
            authorityPath: datadirAuthorityPath,
            receiptPath: resolve(root, DATADIR_BINDING_PATH),
        });
    }
    if (!files.has('wasm/latest.json')) throw new Error('runtime corpus is missing wasm/latest.json');

    const manifestPaths = [...files].filter(path => /^wasm\/[0-9a-f]{12}\/manifest\.json$/u.test(path));
    if (manifestPaths.length === 0) throw new Error('runtime corpus contains no versioned manifest');
    if (addition && manifestPaths.length !== 1) {
        throw new Error('runtime addition must contain exactly one immutable wasm build');
    }
    const manifests = new Map();
    for (const path of manifestPaths) {
        const value = await validateManifest(
            root, path, files, datadirDeployment?.receipt.demo, addition, undefined,
        );
        await validateBuildClosure(root, value.manifest, files);
        manifests.set(value.manifest.short, value);
    }
    const latestBytes = await readFile(resolve(root, 'wasm/latest.json'));
    const latest = json(latestBytes, 'wasm/latest.json');
    const selected = manifests.get(latest.short);
    if (selected === undefined || !latestBytes.equals(selected.bytes)) {
        throw new Error('wasm/latest.json must exactly equal one versioned manifest');
    }
    if (expectedContract !== undefined) {
        exact(selected.manifest.netProtocol, expectedContract.netProtocol, 'latest current netProtocol');
        exact(selected.manifest.ticketSchema, expectedContract.ticketSchema, 'latest current ticketSchema');
        exact(
            selected.manifest.multiplayerContent.schema,
            expectedContract.contentSchema,
            'latest current content schema',
        );
    }
    return { ...metrics, datadirDeployment, latest: selected.manifest };
}

async function main() {
    const args = process.argv.slice(2);
    const addition = args.includes('--addition');
    const currentSource = args.includes('--current-source');
    const authorityIndex = args.indexOf('--datadir-authority');
    let datadirAuthorityPath;
    if (authorityIndex !== -1) {
        datadirAuthorityPath = args[authorityIndex + 1];
        if (datadirAuthorityPath === undefined) throw new Error('--datadir-authority requires a path');
    }
    const positionals = args.filter((value, index) => value !== '--addition'
        && value !== '--current-source'
        && value !== '--datadir-authority'
        && index !== authorityIndex + 1);
    if (positionals.length !== 1) {
        throw new Error('usage: node scripts/verify-runtime-corpus.mjs [--addition] [--current-source] [--datadir-authority AUTHORITY] DIRECTORY');
    }
    const expectedContract = currentSource ? await verifyRuntimeSourceContract() : undefined;
    const metrics = await verifyRuntimeCorpus(positionals[0], {
        addition, expectedContract, datadirAuthorityPath,
    });
    console.log(`verified ${addition ? 'runtime addition' : 'complete wasm runtime corpus'}: ${metrics.assetCount} assets, ${metrics.totalBytes} bytes`);
}

const invokedPath = process.argv[1];
if (invokedPath !== undefined && import.meta.url === pathToFileURL(resolve(invokedPath)).href) {
    main().catch(error => {
        console.error(error instanceof Error ? error.message : String(error));
        process.exitCode = 1;
    });
}
