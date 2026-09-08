import assert from 'node:assert/strict';
import { lstat, readFile, readdir } from 'node:fs/promises';
import { dirname, extname, relative, resolve, sep } from 'node:path';
import { pathToFileURL } from 'node:url';
import { parse } from 'es-module-lexer/js';

const ENTRY = 'leaderboard_identity_bridge.js';
const WASM = 'leaderboard_identity_bridge_bg.wasm';
const VAULT = 'browser_identity_vault.js';
const CLIENT = 'browser_identity_client.js';
const GAME_ORIGIN = 'https://robinhood.phiresky.xyz';
const SIGNER_ORIGIN = 'https://identity.robinhood.phiresky.xyz';
const EXPECTED_EXPORTS = [
    'default',
    'initSync',
    'robinhoodAuthorizeLeaderboardIdentityParent',
    'robinhoodLeaderboardIdentityStatus',
    'robinhoodLeaderboardPublicKey',
    'robinhoodSignCampaignContinuation',
    'robinhoodSignCampaignContinuationPreflightAsController',
    'robinhoodSignCampaignContinuationPreflightAsHost',
    'robinhoodSignMultiplayerLeaderboardRequest',
    'robinhoodSignNamedSeatJoin',
    'robinhoodSignReplaySessionGenesis',
    'robinhoodSignCompetitionRunGrantRequest',
    'robinhoodSignDeletionRequest',
    'robinhoodSignFreshRunPreflightRequest',
    'robinhoodSignSubmissionClaim',
    'robinhoodSignSubmissionOwnerStatus',
    'robinhoodSignUsernameUpdate',
].sort();

async function regularFiles(root) {
    const files = [];
    async function visit(directory) {
        for (const entry of await readdir(directory, { withFileTypes: true })) {
            const path = resolve(directory, entry.name);
            const facts = await lstat(path);
            const relativePath = relative(root, path).split(sep).join('/');
            if (facts.isSymbolicLink()) throw new Error(`signer bridge contains symlink ${relativePath}`);
            if (facts.isDirectory()) await visit(path);
            else if (facts.isFile()) files.push(path);
            else throw new Error(`signer bridge contains non-regular entry ${relativePath}`);
        }
    }
    await visit(root);
    return files;
}

function exactBasename(files, basename) {
    const matches = files.filter(path => path.endsWith(`${sep}${basename}`) || path === basename);
    if (matches.length !== 1) {
        throw new Error(`signer bridge must contain exactly one ${basename}; found ${matches.length}`);
    }
    return matches[0];
}

function wasmSectionIds(bytes) {
    if (bytes.length < 8 || !bytes.subarray(0, 8).equals(Buffer.from([
        0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00,
    ]))) {
        throw new Error('signer bridge is not a canonical WebAssembly module');
    }
    let offset = 8;
    const identifiers = [];
    function uleb() {
        let value = 0;
        let shift = 0;
        for (;;) {
            if (offset >= bytes.length || shift > 28) throw new Error('signer bridge has malformed section length');
            const byte = bytes[offset];
            offset += 1;
            value |= (byte & 0x7f) << shift;
            if ((byte & 0x80) === 0) return value >>> 0;
            shift += 7;
        }
    }
    while (offset < bytes.length) {
        const identifier = bytes[offset];
        offset += 1;
        const length = uleb();
        if (identifier > 12 || offset + length > bytes.length) {
            throw new Error('signer bridge has a malformed WebAssembly section');
        }
        identifiers.push(identifier);
        offset += length;
    }
    return identifiers;
}

export async function verifyIdentitySignerBridge(directory, { exerciseInitialization = true } = {}) {
    const root = resolve(directory);
    const rootFacts = await lstat(root).catch(() => undefined);
    if (rootFacts === undefined || !rootFacts.isDirectory() || rootFacts.isSymbolicLink()) {
        throw new Error(`signer bridge is not a regular directory: ${root}`);
    }
    const files = await regularFiles(root);
    for (const path of files) {
        const relativePath = relative(root, path).split(sep).join('/');
        if (!['.js', '.wasm'].includes(extname(path))) {
            throw new Error(`signer bridge has an unexpected generated artifact: ${relativePath}`);
        }
    }
    const entry = resolve(root, ENTRY);
    const wasm = resolve(root, WASM);
    if (!files.includes(entry) || !files.includes(wasm)) {
        throw new Error(`signer bridge is missing ${ENTRY} or ${WASM}`);
    }
    const vault = exactBasename(files, VAULT);
    if (files.some(path => path.endsWith(`${sep}${CLIENT}`) || path === CLIENT)) {
        throw new Error(`signer bridge must not retain ${CLIENT}`);
    }
    const sourceVault = await readFile(new URL('../../crates/robin_identity_signer/js/browser_identity_vault.js', import.meta.url));
    assert.deepEqual(await readFile(vault), sourceVault, 'generated signer vault differs from checked-in source');

    const javascript = new Set(files.filter(path => path.endsWith('.js')));
    const graph = new Map();
    let entryExports = [];
    for (const path of javascript) {
        const source = await readFile(path, 'utf8');
        const [imports, exports] = parse(source, relative(root, path));
        if (path === entry) entryExports = exports.map(item => item.n).sort();
        const targets = [];
        for (const imported of imports) {
            if (imported.t === 3) continue;
            if (imported.t !== 1 || imported.a !== -1 || imported.n === undefined) {
                throw new Error(`signer bridge has a dynamic, phased, or attributed import in ${relative(root, path)}`);
            }
            if (!(imported.n.startsWith('./') || imported.n.startsWith('../'))
                || imported.n.includes('\\') || imported.n.includes('?') || imported.n.includes('#')) {
                throw new Error(`signer bridge has a non-relative import ${imported.n}`);
            }
            const target = resolve(dirname(path), imported.n);
            if (!javascript.has(target)) {
                throw new Error(`signer bridge imports undeclared JavaScript ${imported.n}`);
            }
            targets.push(target);
        }
        graph.set(path, targets);
    }
    assert.deepEqual(entryExports, EXPECTED_EXPORTS, 'signer bridge exports drifted from the typed surface');

    const reachable = new Set();
    function visit(path) {
        if (reachable.has(path)) return;
        reachable.add(path);
        for (const target of graph.get(path) ?? []) visit(target);
    }
    visit(entry);
    if (reachable.size !== javascript.size || !reachable.has(vault)) {
        throw new Error('signer bridge JavaScript is not one exact reachable import closure');
    }

    const wasmBytes = await readFile(wasm);
    const module = new WebAssembly.Module(wasmBytes);
    const wasmExports = WebAssembly.Module.exports(module).map(item => item.name);
    for (const forbidden of ['_start', 'main']) {
        if (wasmExports.includes(forbidden)) {
            throw new Error(`no-start signer bridge exports forbidden ${forbidden}`);
        }
    }
    // wasm-bindgen's own `__wbindgen_start` initializes its externref table and
    // is present even without an application start hook. The actual no-start
    // contract is absence of WebAssembly section 8 plus the eager-IDB test.
    if (wasmSectionIds(wasmBytes).includes(8)) {
        throw new Error('no-start signer bridge contains a WebAssembly start section');
    }
    const binaryText = wasmBytes.toString('latin1');
    for (const origin of [GAME_ORIGIN, SIGNER_ORIGIN]) {
        if (!binaryText.includes(origin)) {
            throw new Error(`signer bridge was not compiled for exact origin ${origin}`);
        }
    }
    if (/https?:\/\/(?:localhost|127\.0\.0\.1|\[::1\])/u.test(binaryText)) {
        throw new Error('signer bridge contains a loopback deployment origin');
    }
    const compiledText = (await Promise.all([...javascript].map(path => readFile(path, 'utf8')))).join('\n');
    for (const forbidden of ['sign_raw', 'sign_bytes', 'export_private_key']) {
        if (compiledText.includes(forbidden) || binaryText.includes(forbidden)) {
            throw new Error(`signer bridge exposes forbidden generic operation ${forbidden}`);
        }
    }

    if (exerciseInitialization) {
        const prior = Object.getOwnPropertyDescriptor(globalThis, 'indexedDB');
        Object.defineProperty(globalThis, 'indexedDB', {
            configurable: true,
            get() {
                throw new Error('signer bridge eagerly accessed IndexedDB during module initialization');
            },
        });
        try {
            const generated = await import(`${pathToFileURL(entry).href}?verify=${Date.now()}`);
            await generated.default({ module_or_path: wasmBytes });
        } finally {
            if (prior === undefined) Reflect.deleteProperty(globalThis, 'indexedDB');
            else Object.defineProperty(globalThis, 'indexedDB', prior);
        }
    }

    return {
        files: files.map(path => relative(root, path).split(sep).join('/')).sort(),
        exports: entryExports,
    };
}

async function main() {
    const [directory] = process.argv.slice(2);
    if (directory === undefined || process.argv.length !== 3) {
        throw new Error('usage: node scripts/verify-identity-signer-bridge.mjs BRIDGE_DIRECTORY');
    }
    await verifyIdentitySignerBridge(directory);
}

const invoked = process.argv[1];
if (invoked !== undefined && import.meta.url === pathToFileURL(resolve(invoked)).href) {
    main().catch(error => {
        console.error(error instanceof Error ? error.message : String(error));
        process.exitCode = 1;
    });
}
