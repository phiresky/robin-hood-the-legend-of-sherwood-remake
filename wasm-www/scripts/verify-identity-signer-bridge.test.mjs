import assert from 'node:assert/strict';
import { mkdtemp, mkdir, readFile, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { verifyIdentitySignerBridge } from './verify-identity-signer-bridge.mjs';

const signerCargo = await readFile(new URL('../../crates/robin_identity_signer/Cargo.toml', import.meta.url), 'utf8');
const signerLibrary = await readFile(new URL('../../crates/robin_rs/src/lib.rs', import.meta.url), 'utf8');
const signerBuild = await readFile(new URL('../../scripts/build_identity_signer.sh', import.meta.url), 'utf8');
const signerBridgeSource = await readFile(
    new URL('../../crates/robin_identity_signer/src/bin/leaderboard_identity_bridge.rs', import.meta.url),
    'utf8',
);

const origins = [
    'https://robinhood.phiresky.xyz',
    'https://identity.robinhood.phiresky.xyz',
];
const exports = [
    'robinhoodAuthorizeLeaderboardIdentityParent',
    'robinhoodLeaderboardIdentityStatus',
    'robinhoodLeaderboardPublicKey',
    'robinhoodSignUsernameUpdate',
    'robinhoodSignSubmissionClaim',
    'robinhoodSignMultiplayerLeaderboardRequest',
    'robinhoodSignNamedSeatJoin',
    'robinhoodSignReplaySessionGenesis',
    'robinhoodSignCompetitionRunGrantRequest',
    'robinhoodSignCampaignContinuation',
    'robinhoodSignCampaignContinuationPreflightAsController',
    'robinhoodSignCampaignContinuationPreflightAsHost',
    'robinhoodSignFreshRunPreflightRequest',
    'robinhoodSignSubmissionOwnerStatus',
    'robinhoodSignDeletionRequest',
];

function uleb(value) {
    const result = [];
    do {
        let byte = value & 0x7f;
        value >>>= 7;
        if (value !== 0) byte |= 0x80;
        result.push(byte);
    } while (value !== 0);
    return result;
}

function section(identifier, contents) {
    return [identifier, ...uleb(contents.length), ...contents];
}

function utf8(value) {
    const bytes = [...new TextEncoder().encode(value)];
    return [...uleb(bytes.length), ...bytes];
}

function fixtureWasm(exportStart = false, startSection = false) {
    const header = [0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00];
    const originsCustomSection = section(0, [...utf8('origins'), ...new TextEncoder().encode(origins.join('\0'))]);
    if (!exportStart && !startSection) return Buffer.from([...header, ...originsCustomSection]);
    const type = section(1, [1, 0x60, 0, 0]);
    const functions = section(3, [1, 0]);
    const exported = exportStart ? section(7, [1, ...utf8('_start'), 0, 0]) : [];
    const start = startSection ? section(8, [0]) : [];
    const code = section(10, [1, 2, 0, 0x0b]);
    return Buffer.from([
        ...header, ...type, ...functions, ...exported, ...start, ...code, ...originsCustomSection,
    ]);
}

async function bridgeFixture({
    extraExport = false,
    eagerIndexedDb = false,
    exportStart = false,
    startSection = false,
} = {}) {
    const root = await mkdtemp(join(tmpdir(), 'verified-signer-bridge.'));
    const snippets = join(root, 'snippets/robin_rs-test/js');
    await mkdir(snippets, { recursive: true });
    await writeFile(
        join(snippets, 'browser_identity_vault.js'),
        await readFile(new URL('../../crates/robin_identity_signer/js/browser_identity_vault.js', import.meta.url)),
    );
    const declarations = exports.map(name => `export function ${name}() {}`).join('\n');
    await writeFile(join(root, 'leaderboard_identity_bridge.js'), [
        "import './snippets/robin_rs-test/js/browser_identity_vault.js';",
        declarations,
        'export function initSync() {}',
        `export default async function init() { ${eagerIndexedDb ? 'void indexedDB;' : ''} }`,
        extraExport ? 'export const rawSigner = true;' : '',
    ].join('\n'));
    await writeFile(
        join(root, 'leaderboard_identity_bridge_bg.wasm'),
        fixtureWasm(exportStart, startSection),
    );
    return root;
}

test('bridge verification accepts only the exact typed, no-start, vault-only closure', async () => {
    const root = await bridgeFixture();
    const result = await verifyIdentitySignerBridge(root);
    assert.deepEqual(result.exports, ['default', 'initSync', ...exports].sort());
    assert.equal(result.files.some(path => path.endsWith('browser_identity_client.js')), false);
});

test('bridge verification rejects export drift and a WebAssembly start surface', async () => {
    await assert.rejects(
        verifyIdentitySignerBridge(await bridgeFixture({ extraExport: true })),
        /typed surface/u,
    );
    await assert.rejects(
        verifyIdentitySignerBridge(await bridgeFixture({ exportStart: true })),
        /no-start signer bridge exports forbidden _start/u,
    );
    await assert.rejects(
        verifyIdentitySignerBridge(await bridgeFixture({ startSection: true })),
        /no-start signer bridge contains a WebAssembly start section/u,
    );
});

test('bridge verification detects eager IndexedDB access during initialization', async () => {
    await assert.rejects(
        verifyIdentitySignerBridge(await bridgeFixture({ eagerIndexedDb: true })),
        /eagerly accessed IndexedDB/u,
    );
});

test('signer is a standalone minimal crate and game modules are explicit', () => {
    assert.match(signerCargo, /^name = "robin_identity_signer"$/mu);
    assert.match(signerCargo, /^identity-signer-bridge = \[\]$/mu);
    assert.doesNotMatch(signerCargo, /robin_rs|robin_engine|robin_assets|wgpu|winit/u);
    assert.doesNotMatch(signerLibrary, /declare_full_game_modules|identity-signer-bridge/u);
    assert.match(signerLibrary, /pub mod game;/u);
    assert.match(signerBuild, /-p robin_identity_signer/u);
    assert.match(signerBuild, /--no-default-features \\\s+--features identity-signer-bridge/u);
    assert.doesNotMatch(signerBuild, /--features audio/u);
    assert.match(signerBridgeSource, /cfg_attr\(target_arch = "wasm32", no_main\)/u);
    assert.match(signerBridgeSource, /cfg\(not\(target_arch = "wasm32"\)\)[\s\S]+fn main\(\) \{\}/u);
    assert.doesNotMatch(signerBridgeSource, /wasm_bindgen\s*\(\s*start/u);
});
