import assert from 'node:assert/strict';
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { resolve } from 'node:path';
import test from 'node:test';
import {
    authorRuntimeJavascriptModules,
    verifyRuntimeJavascriptModules,
} from './runtime-javascript-modules.mjs';

const modulePath = 'snippets/robin_rs-build/js/browser_identity_client.js';

async function fixture() {
    const root = await mkdtemp(resolve(tmpdir(), 'runtime-javascript-modules-'));
    await mkdir(resolve(root, 'snippets/robin_rs-build/js'), { recursive: true });
    await writeFile(
        resolve(root, 'robin.js'),
        `import { requestIdentity } from './${modulePath}';\nexport { requestIdentity };\n`,
    );
    await writeFile(
        resolve(root, modulePath),
        'export const requestIdentity = () => "public identity request";\n',
    );
    return root;
}

test('authors and verifies the exact imported runtime JavaScript closure', async t => {
    const root = await fixture();
    t.after(() => rm(root, { recursive: true, force: true }));
    const claims = await authorRuntimeJavascriptModules(root);
    assert.deepEqual(claims.map(claim => claim.path), [modulePath]);
    assert.equal(claims[0].byteLength, 64);
    assert.match(claims[0].sha256, /^[0-9a-f]{64}$/u);
    assert.deepEqual(await verifyRuntimeJavascriptModules(root, claims), claims);
});

test('rejects missing, substituted, extra, and tampered module claims', async t => {
    const missing = await fixture();
    const substituted = await fixture();
    const extra = await fixture();
    const tampered = await fixture();
    t.after(() => Promise.all([missing, substituted, extra, tampered]
        .map(root => rm(root, { recursive: true, force: true }))));

    const missingClaims = await authorRuntimeJavascriptModules(missing);
    await rm(resolve(missing, modulePath));
    await assert.rejects(
        verifyRuntimeJavascriptModules(missing, missingClaims),
        /imports missing module|exactly one browser_identity_client\.js; found 0/u,
    );

    const substitutedClaims = await authorRuntimeJavascriptModules(substituted);
    substitutedClaims[0] = {
        ...substitutedClaims[0],
        path: 'snippets/robin_rs-build/js/substitute.js',
    };
    await assert.rejects(
        verifyRuntimeJavascriptModules(substituted, substitutedClaims),
        /do not match the exact imported module closure/u,
    );

    const extraClaims = await authorRuntimeJavascriptModules(extra);
    extraClaims.push({
        path: 'snippets/robin_rs-build/js/extra.js',
        byteLength: 1,
        sha256: '1'.repeat(64),
    });
    await assert.rejects(
        verifyRuntimeJavascriptModules(extra, extraClaims),
        /not unique and UTF-8 sorted|do not match the exact imported module closure/u,
    );

    const tamperedClaims = await authorRuntimeJavascriptModules(tampered);
    await writeFile(resolve(tampered, modulePath), 'export const requestIdentity = () => "tampered";\n');
    await assert.rejects(
        verifyRuntimeJavascriptModules(tampered, tamperedClaims),
        /do not match the exact imported module closure/u,
    );
});

test('rejects an identity vault and every orphan JavaScript module', async t => {
    const vault = await fixture();
    const orphan = await fixture();
    const noncanonical = await fixture();
    t.after(() => Promise.all([vault, orphan, noncanonical]
        .map(root => rm(root, { recursive: true, force: true }))));

    await writeFile(
        resolve(vault, 'snippets/robin_rs-build/js/browser_identity_vault.js'),
        'export const privateKey = "forbidden";\n',
    );
    await assert.rejects(
        authorRuntimeJavascriptModules(vault),
        /forbidden identity vault/u,
    );

    await writeFile(resolve(orphan, 'orphan.js'), 'export const orphan = true;\n');
    await assert.rejects(
        authorRuntimeJavascriptModules(orphan),
        /orphan modules/u,
    );

    await writeFile(
        resolve(noncanonical, 'robin.js'),
        "import './snippets/robin_rs-build/js/../js/browser_identity_client.js';\n",
    );
    await assert.rejects(
        authorRuntimeJavascriptModules(noncanonical),
        /non-canonical relative import/u,
    );
});
