import assert from 'node:assert/strict';
import test from 'node:test';
import { mkdir, mkdtemp, readFile, rm, symlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { resolve } from 'node:path';
import { gzipSync } from 'node:zlib';
import { verifyPublicBuild, verifySignerBuild } from './verify-static-build.mjs';

const publicHeaders = await readFile(new URL('../deploy/public-headers.txt', import.meta.url), 'utf8');
const signerHeaders = await readFile(new URL('../deploy/signer-headers.txt', import.meta.url), 'utf8');
const vaultSource = await readFile(
    new URL('../../crates/robin_identity_signer/js/browser_identity_vault.js', import.meta.url),
);
const bridgeExports = [
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

function signerFixtureWasm() {
    const text = new TextEncoder().encode([
        'https://robinhood.phiresky.xyz',
        'https://identity.robinhood.phiresky.xyz',
    ].join('\0'));
    const name = new TextEncoder().encode('origins');
    const custom = [...uleb(name.length), ...name, ...text];
    return Buffer.from([
        0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00,
        0, ...uleb(custom.length), ...custom,
    ]);
}

async function withRoot(prefix, run) {
    const root = await mkdtemp(resolve(tmpdir(), prefix));
    try { await run(root); } finally { await rm(root, { recursive: true, force: true }); }
}

async function publicFixture(root) {
    await mkdir(resolve(root, 'assets'), { recursive: true });
    await mkdir(resolve(root, 'leaderboards'), { recursive: true });
    await writeFile(resolve(root, '_headers'), publicHeaders);
    await writeFile(resolve(root, 'assets/game.js'), [
        "const signer='https://identity.robinhood.phiresky.xyz';",
        "const api='/api/v1';",
    ].join('\n'));
    await writeFile(resolve(root, 'assets/boards.js'), "const api='/api/v1';");
    await writeFile(resolve(root, 'index.html'), `<!doctype html>
        <meta http-equiv="Content-Security-Policy" content="default-src 'none'; connect-src 'self' https: wss:; frame-src https://identity.robinhood.phiresky.xyz">
        <script type="module" src="/assets/game.js"></script>`);
    await writeFile(resolve(root, 'leaderboards/index.html'), `<!doctype html>
        <meta http-equiv="Content-Security-Policy" content="default-src 'none'; connect-src 'self'; frame-src https://identity.robinhood.phiresky.xyz">
        <script type="module" src="../assets/boards.js"></script>`);
}

async function signerFixture(root) {
    await mkdir(resolve(root, 'assets'), { recursive: true });
    await mkdir(resolve(root, 'identity-signer'), { recursive: true });
    const bridge = resolve(root, 'identity-signer/bridge');
    const snippets = resolve(bridge, 'snippets/robin_rs-test/js');
    await mkdir(snippets, { recursive: true });
    await writeFile(resolve(root, '_headers'), signerHeaders);
    await writeFile(resolve(root, 'assets/signer.js'), [
        "const protocol='robinhood.multiplayer-identity.v1';",
        "const domain='robinhood/browser-seat-proof/v1';",
        "const leaderboard='robinhood.browser-identity.v1';",
        "const bridge='/identity-signer/bridge/leaderboard_identity_bridge.js';",
        "const wasm='/identity-signer/bridge/leaderboard_identity_bridge_bg.wasm';",
    ].join('\n'));
    await writeFile(resolve(snippets, 'browser_identity_vault.js'), vaultSource);
    await writeFile(resolve(bridge, 'leaderboard_identity_bridge.js'), [
        "import './snippets/robin_rs-test/js/browser_identity_vault.js';",
        ...bridgeExports.map(name => `export function ${name}() {}`),
        'export function initSync() {}',
        'export default async function init() {}',
    ].join('\n'));
    await writeFile(resolve(bridge, 'leaderboard_identity_bridge_bg.wasm'), signerFixtureWasm());
    await writeFile(resolve(root, 'identity-signer/index.html'), `<!doctype html>
        <meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'self' 'wasm-unsafe-eval'; connect-src 'self'">
        <script type="module" src="/assets/signer.js"></script>`);
}

test('public build accepts same-origin API and isolated signer topology', async () => {
    await withRoot('public-build-', async root => {
        await publicFixture(root);
        await writeFile(
            resolve(root, 'assets/attribution.js'),
            [
                '// Algorithm attribution: https://swordfish90.github.io/cheap-upscaling-triangulation/',
                '// Unrelated project: https://example.github.io/bin',
            ].join('\n'),
        );
        await verifyPublicBuild(root);
    });
});

test('public build accepts genuine viewer gzip artifacts and rejects arbitrary extensions', async () => {
    await withRoot('public-viewer-gzip-', async root => {
        await publicFixture(root);
        const viewer = resolve(root, 'builds', 'ab'.repeat(32), 'viewer');
        await mkdir(viewer, { recursive: true });
        await writeFile(resolve(viewer, 'robin.js.gz'), gzipSync(Buffer.from('export default 1;')));
        await writeFile(resolve(viewer, 'robin_bg.wasm.gz'), gzipSync(Buffer.from('wasm fixture')));
        await verifyPublicBuild(root);
    });
    await withRoot('public-arbitrary-extension-', async root => {
        await publicFixture(root);
        await writeFile(resolve(root, 'assets/unreviewed.bin'), 'unreviewed');
        await assert.rejects(verifyPublicBuild(root), /non-public file extension/u);
    });
});

test('public build rejects signer leakage, retired fallbacks, and symlinks', async () => {
    await withRoot('public-leak-', async root => {
        await publicFixture(root);
        await mkdir(resolve(root, 'identity-signer'));
        await writeFile(resolve(root, 'identity-signer/index.html'), 'leak');
        await assert.rejects(verifyPublicBuild(root), /must not contain the isolated signer/u);
    });
    await withRoot('public-pages-', async root => {
        await publicFixture(root);
        const fallbacks = [
            "'https://phiresky.github.io/robin-hood-the-legend-of-sherwood-remake-binaries/wasm'",
            "'HTTP://PHIRESKY.GITHUB.IO:80/robin-hood-the-legend-of-sherwood-remake-binaries?build=old#wasm'",
            "'https://phiresky.github.io:443/other-path?fallback=1'",
            "'https://example.invalid/?fallback=robin-hood-the-legend-of-sherwood-remake-binaries#wasm'",
            "'https://phiresky%2Egithub%2Eio/robin%2Dhood%2Dthe%2Dlegend%2Dof%2Dsherwood%2Dremake%2Dbinaries/wasm'",
            "'https://phire' + 'sky.github.io/robin-' + 'hood-the-legend-of-sherwood-remake-binaries/wasm'",
        ];
        for (const fallback of fallbacks) {
            await writeFile(resolve(root, 'assets/game.js'), `const fallback=${fallback}; const api='/api/v1'; const signer='https://identity.robinhood.phiresky.xyz';`);
            await assert.rejects(verifyPublicBuild(root), /retired GitHub Pages/u);
        }
    });
    await withRoot('public-link-', async root => {
        await publicFixture(root);
        await symlink(resolve(root, 'assets/game.js'), resolve(root, 'assets/alias.js'));
        await assert.rejects(verifyPublicBuild(root), /symbolic link/u);
    });
});

test('signer build accepts only the unified exact signer protocols and bridge', async () => {
    await withRoot('signer-build-', async root => {
        await signerFixture(root);
        await verifySignerBuild(root);
        await writeFile(resolve(root, 'assets/signer.js'), [
            "const op='sign_raw';",
            "const protocol='robinhood.multiplayer-identity.v1';",
            "const domain='robinhood/browser-seat-proof/v1';",
            "const leaderboard='robinhood.browser-identity.v1';",
            "const bridge='/identity-signer/bridge/leaderboard_identity_bridge.js';",
            "const wasm='/identity-signer/bridge/leaderboard_identity_bridge_bg.wasm';",
        ].join('\n'));
        await assert.rejects(verifySignerBuild(root), /generic operation/u);
    });
});
