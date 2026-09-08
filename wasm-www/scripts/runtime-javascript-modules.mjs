import { createHash } from 'node:crypto';
import { lstat, readFile, readdir } from 'node:fs/promises';
import { dirname, extname, relative, resolve, sep } from 'node:path';
import { pathToFileURL } from 'node:url';
import { parse } from 'es-module-lexer/js';

const ENTRY = 'robin.js';
const CLIENT = 'browser_identity_client.js';
const VAULT = 'browser_identity_vault.js';
const STATIC_IMPORT = 1;
const IMPORT_META = 3;
const DIGEST = /^[0-9a-f]{64}$/u;
const MODULE_PATH = /^(?:[A-Za-z0-9_.-]+\/)*[A-Za-z0-9_.-]+\.js$/u;

function utf8Order(left, right) {
    return Buffer.compare(Buffer.from(left, 'utf8'), Buffer.from(right, 'utf8'));
}

function sha256(bytes) {
    return createHash('sha256').update(bytes).digest('hex');
}

function exactKeys(value, keys, label) {
    if (value === null || typeof value !== 'object' || Array.isArray(value)) {
        throw new Error(`${label} must be an object`);
    }
    const actual = Object.keys(value).sort(utf8Order);
    const expected = [...keys].sort(utf8Order);
    if (JSON.stringify(actual) !== JSON.stringify(expected)) {
        throw new Error(`${label} has unexpected keys: ${actual.join(', ')}`);
    }
}

function validateClaim(claim, index) {
    const label = `runtime JavaScript module claim ${index}`;
    exactKeys(claim, ['byteLength', 'path', 'sha256'], label);
    if (typeof claim.path !== 'string'
        || !MODULE_PATH.test(claim.path)
        || claim.path.startsWith('/')
        || claim.path.split('/').some(segment => segment === '.' || segment === '..')) {
        throw new Error(`${label} has a non-canonical path`);
    }
    if (claim.path === ENTRY) throw new Error(`${label} must not redeclare the entry module`);
    if (claim.path.endsWith(`/${VAULT}`) || claim.path === VAULT) {
        throw new Error('runtime JavaScript module closure must never contain the identity vault');
    }
    if (!Number.isSafeInteger(claim.byteLength) || claim.byteLength <= 0) {
        throw new Error(`${label} has an invalid byte length`);
    }
    if (typeof claim.sha256 !== 'string' || !DIGEST.test(claim.sha256)) {
        throw new Error(`${label} has an invalid SHA-256`);
    }
}

function canonicalRelativeImport(specifier) {
    if (typeof specifier !== 'string'
        || specifier.includes('\\')
        || specifier.includes('?')
        || specifier.includes('#')) return false;
    const segments = specifier.split('/');
    let index;
    if (segments[0] === '.') index = 1;
    else {
        index = 0;
        while (segments[index] === '..') index += 1;
        if (index === 0) return false;
    }
    return index < segments.length
        && segments.slice(index).every(segment => segment !== '' && segment !== '.' && segment !== '..');
}

async function javascriptFiles(root) {
    const files = new Map();
    async function visit(directory) {
        for (const entry of (await readdir(directory, { withFileTypes: true }))
            .sort((left, right) => utf8Order(left.name, right.name))) {
            const absolute = resolve(directory, entry.name);
            const path = relative(root, absolute).split(sep).join('/');
            const facts = await lstat(absolute);
            if (facts.isSymbolicLink()) {
                throw new Error(`runtime JavaScript module closure contains a symlink: ${path}`);
            }
            if (facts.isDirectory()) await visit(absolute);
            else if (facts.isFile() && extname(path) === '.js') files.set(path, absolute);
            else if (!facts.isFile()) {
                throw new Error(`runtime JavaScript module closure contains a non-regular entry: ${path}`);
            }
        }
    }
    const facts = await lstat(root).catch(() => undefined);
    if (facts === undefined || !facts.isDirectory() || facts.isSymbolicLink()) {
        throw new Error(`runtime JavaScript build is not a real directory: ${root}`);
    }
    await visit(root);
    return files;
}

async function deriveRuntimeJavascriptModules(directory) {
    const root = resolve(directory);
    const files = await javascriptFiles(root);
    if (!files.has(ENTRY)) throw new Error(`runtime JavaScript build is missing ${ENTRY}`);
    const vaults = [...files.keys()].filter(path => path === VAULT || path.endsWith(`/${VAULT}`));
    if (vaults.length !== 0) {
        throw new Error(`runtime JavaScript module closure contains forbidden identity vault: ${vaults.join(', ')}`);
    }
    const clients = [...files.keys()].filter(path => path === CLIENT || path.endsWith(`/${CLIENT}`));
    if (clients.length !== 1) {
        throw new Error(`runtime JavaScript module closure must contain exactly one ${CLIENT}; found ${clients.length}`);
    }

    const graph = new Map();
    for (const [path, absolute] of files) {
        const source = await readFile(absolute, 'utf8');
        let imports;
        try {
            [imports] = parse(source, path);
        } catch (error) {
            throw new Error(`runtime JavaScript module is invalid: ${path}`, { cause: error });
        }
        const targets = [];
        for (const imported of imports) {
            if (imported.t === IMPORT_META) continue;
            if (imported.t !== STATIC_IMPORT || imported.a !== -1 || imported.n === undefined) {
                throw new Error(`runtime JavaScript module has a dynamic, phased, or attributed import: ${path}`);
            }
            if (!canonicalRelativeImport(imported.n)) {
                throw new Error(`runtime JavaScript module has a non-canonical relative import: ${imported.n}`);
            }
            const target = resolve(dirname(absolute), imported.n);
            const targetPath = relative(root, target).split(sep).join('/');
            if (targetPath.startsWith('../') || !files.has(targetPath)) {
                throw new Error(`runtime JavaScript module ${path} imports missing module ${imported.n}`);
            }
            targets.push(targetPath);
        }
        graph.set(path, targets);
    }

    const state = new Map();
    const reachable = new Set();
    function visit(path) {
        if (state.get(path) === 'visiting') {
            throw new Error('runtime JavaScript module graph contains a cycle');
        }
        if (state.get(path) === 'visited') return;
        state.set(path, 'visiting');
        reachable.add(path);
        for (const target of graph.get(path) ?? []) visit(target);
        state.set(path, 'visited');
    }
    visit(ENTRY);
    const orphans = [...files.keys()].filter(path => !reachable.has(path));
    if (orphans.length !== 0) {
        throw new Error(`runtime JavaScript module closure contains orphan modules: ${orphans.join(', ')}`);
    }
    if (!reachable.has(clients[0])) {
        throw new Error(`runtime JavaScript entry cannot reach ${CLIENT}`);
    }

    const claims = [];
    for (const path of [...reachable].filter(path => path !== ENTRY).sort(utf8Order)) {
        const bytes = await readFile(files.get(path));
        claims.push({ path, byteLength: bytes.byteLength, sha256: sha256(bytes) });
    }
    if (claims.length === 0 || claims.length > 32) {
        throw new Error(`runtime JavaScript module closure has invalid size ${claims.length}`);
    }
    return claims;
}

export async function authorRuntimeJavascriptModules(directory) {
    return deriveRuntimeJavascriptModules(directory);
}

export async function verifyRuntimeJavascriptModules(directory, claims) {
    if (!Array.isArray(claims) || claims.length === 0 || claims.length > 32) {
        throw new Error('runtime JavaScript module claims must contain between 1 and 32 entries');
    }
    for (const [index, claim] of claims.entries()) validateClaim(claim, index);
    if (!claims.every((claim, index) => (
        index === 0 || utf8Order(claims[index - 1].path, claim.path) < 0
    ))) {
        throw new Error('runtime JavaScript module claims are not unique and UTF-8 sorted');
    }
    const derived = await deriveRuntimeJavascriptModules(directory);
    if (JSON.stringify(claims) !== JSON.stringify(derived)) {
        throw new Error('runtime JavaScript module claims do not match the exact imported module closure');
    }
    return derived;
}

async function main() {
    const [directory, extra] = process.argv.slice(2);
    if (directory === undefined || extra !== undefined) {
        throw new Error('usage: node runtime-javascript-modules.mjs BUILD_DIRECTORY');
    }
    console.log(JSON.stringify(await authorRuntimeJavascriptModules(directory)));
}

const invokedPath = process.argv[1];
if (invokedPath !== undefined && import.meta.url === pathToFileURL(resolve(invokedPath)).href) {
    main().catch(error => {
        console.error(error instanceof Error ? error.message : String(error));
        process.exitCode = 1;
    });
}
