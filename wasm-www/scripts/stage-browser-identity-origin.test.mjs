import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import {
    existsSync,
    mkdtempSync,
    mkdirSync,
    readFileSync,
    rmSync,
    writeFileSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import test from 'node:test';
import { stageBrowserIdentityOrigin } from './stage-browser-identity-origin.mjs';

const clientSource = readFileSync(new URL('../../crates/robin_rs/js/browser_identity_client.js', import.meta.url));
const vaultSource = readFileSync(new URL('../../crates/robin_identity_signer/js/browser_identity_vault.js', import.meta.url));
const snippetDirectory = 'snippets/robin_rs-build/js';

function digest(bytes) {
    return createHash('sha256').update(bytes).digest('hex');
}

function writeGeneratedOrigin(root, entryName, liveModule) {
    const snippets = join(root, snippetDirectory);
    mkdirSync(snippets, { recursive: true });
    writeFileSync(join(snippets, 'browser_identity_client.js'), clientSource);
    writeFileSync(join(snippets, 'browser_identity_vault.js'), vaultSource);
    writeFileSync(join(root, entryName), `import './${snippetDirectory}/${liveModule}';\n`);
}

test('signer staging retains only the byte-exact vault closure', () => {
    const root = mkdtempSync(join(tmpdir(), 'identity-signer-closure.'));
    writeGeneratedOrigin(root, 'leaderboard_identity_bridge.js', 'browser_identity_vault.js');

    const result = stageBrowserIdentityOrigin(
        'identity_signer',
        root,
        'leaderboard_identity_bridge.js',
    );
    const vault = join(root, snippetDirectory, 'browser_identity_vault.js');
    const client = join(root, snippetDirectory, 'browser_identity_client.js');
    assert.equal(result.retained, `${snippetDirectory}/browser_identity_vault.js`);
    assert.equal(existsSync(client), false);
    assert.equal(digest(readFileSync(vault)), digest(vaultSource));
    assert.notEqual(digest(vaultSource), digest(clientSource));
});

test('engine staging retains only the byte-exact public client closure', () => {
    const root = mkdtempSync(join(tmpdir(), 'identity-engine-closure.'));
    writeGeneratedOrigin(root, 'robin.js', 'browser_identity_client.js');

    const result = stageBrowserIdentityOrigin('engine', root, 'robin.js');
    const vault = join(root, snippetDirectory, 'browser_identity_vault.js');
    const client = join(root, snippetDirectory, 'browser_identity_client.js');
    assert.equal(result.retained, `${snippetDirectory}/browser_identity_client.js`);
    assert.equal(existsSync(vault), false);
    assert.equal(digest(readFileSync(client)), digest(clientSource));
    assert.notEqual(digest(clientSource), digest(vaultSource));
});

test('staging fails before pruning on referenced, missing, or duplicate snippets', () => {
    const referenced = mkdtempSync(join(tmpdir(), 'identity-signer-reference.'));
    writeGeneratedOrigin(referenced, 'leaderboard_identity_bridge.js', 'browser_identity_vault.js');
    writeFileSync(
        join(referenced, 'unexpected.js'),
        `import './${snippetDirectory}/browser_identity_client.js';\n`,
    );
    assert.throws(
        () => stageBrowserIdentityOrigin(
            'identity_signer',
            referenced,
            'leaderboard_identity_bridge.js',
        ),
        /refusing to discard referenced browser_identity_client\.js/u,
    );
    assert.equal(existsSync(join(referenced, snippetDirectory, 'browser_identity_client.js')), true);

    for (const [label, mutate, expected] of [
        [
            'missing vault',
            root => rmSync(join(root, snippetDirectory, 'browser_identity_vault.js')),
            /exactly one generated browser_identity_vault\.js; found 0/u,
        ],
        [
            'duplicate vault',
            root => {
                const duplicate = join(root, 'other', 'browser_identity_vault.js');
                mkdirSync(dirname(duplicate), { recursive: true });
                writeFileSync(duplicate, vaultSource);
            },
            /exactly one generated browser_identity_vault\.js; found 2/u,
        ],
    ]) {
        const root = mkdtempSync(join(tmpdir(), `identity-signer-${label.replaceAll(' ', '-')}.`));
        writeGeneratedOrigin(root, 'leaderboard_identity_bridge.js', 'browser_identity_vault.js');
        mutate(root);
        assert.throws(
            () => stageBrowserIdentityOrigin(
                'identity_signer',
                root,
                'leaderboard_identity_bridge.js',
            ),
            expected,
        );
    }
});

test('isolated crates need only their owned byte-exact, reachable transport', t => {
    for (const [role, owned, opposite] of [
        ['engine', 'browser_identity_client.js', 'browser_identity_vault.js'],
        ['identity_signer', 'browser_identity_vault.js', 'browser_identity_client.js'],
    ]) {
        const root = mkdtempSync(join(tmpdir(), `identity-isolated-${role}.`));
        t.after(() => rmSync(root, { recursive: true }));
        writeGeneratedOrigin(root, 'entry.js', owned);
        rmSync(join(root, snippetDirectory, opposite));
        assert.equal(stageBrowserIdentityOrigin(role, root, 'entry.js').removed, null);

        writeFileSync(join(root, 'unexpected.js'), `import './${snippetDirectory}/${opposite}';\n`);
        assert.throws(() => stageBrowserIdentityOrigin(role, root, 'entry.js'), /imports undeclared JavaScript/u);
        rmSync(join(root, 'unexpected.js'));

        writeFileSync(join(root, snippetDirectory, owned), 'export const corrupted = true;\n');
        assert.throws(() => stageBrowserIdentityOrigin(role, root, 'entry.js'), /differs from its checked-in source authority/u);
    }
});

test('staging requires the retained snippet in a closed static import graph', () => {
    const root = mkdtempSync(join(tmpdir(), 'identity-signer-graph.'));
    writeGeneratedOrigin(root, 'leaderboard_identity_bridge.js', 'browser_identity_vault.js');
    writeFileSync(join(root, 'leaderboard_identity_bridge.js'), 'export const noVault = true;\n');
    assert.throws(
        () => stageBrowserIdentityOrigin(
            'identity_signer',
            root,
            'leaderboard_identity_bridge.js',
        ),
        /cannot reach its required browser_identity_vault\.js/u,
    );
});

test('staging hash-checks both generated snippets before removing either one', () => {
    const root = mkdtempSync(join(tmpdir(), 'identity-signer-snippet-drift.'));
    writeGeneratedOrigin(root, 'leaderboard_identity_bridge.js', 'browser_identity_vault.js');
    writeFileSync(join(root, snippetDirectory, 'browser_identity_client.js'), 'export const drift = true;\n');
    assert.throws(
        () => stageBrowserIdentityOrigin(
            'identity_signer',
            root,
            'leaderboard_identity_bridge.js',
        ),
        /generated browser_identity_client\.js differs from its checked-in source authority/u,
    );
    assert.equal(existsSync(join(root, snippetDirectory, 'browser_identity_vault.js')), true);
    assert.equal(existsSync(join(root, snippetDirectory, 'browser_identity_client.js')), true);
});
