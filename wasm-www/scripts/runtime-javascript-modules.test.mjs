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

const helperPath = 'snippets/wasm-bindgen-rayon-38edf6e439f6d70d/src/workerHelpers.no-bundler.js';

async function pinnedHelperSource() {
    // The verifier pins the exact published crate bytes. Read them from the
    // local Cargo registry when present; otherwise the test is not meaningful.
    const { readdir, readFile } = await import('node:fs/promises');
    const home = process.env.CARGO_HOME ?? resolve(process.env.HOME ?? '', '.cargo');
    const registry = resolve(home, 'registry/src');
    for (const index of await readdir(registry).catch(() => [])) {
        const path = resolve(registry, index, 'wasm-bindgen-rayon-1.3.0/src/workerHelpers.no-bundler.js');
        const source = await readFile(path, 'utf8').catch(() => undefined);
        if (source !== undefined) return source;
    }
    return undefined;
}

async function threadedFixture(helperSource) {
    const root = await fixture();
    await mkdir(resolve(root, helperPath, '..'), { recursive: true });
    await writeFile(resolve(root, helperPath), helperSource);
    await writeFile(
        resolve(root, 'robin.js'),
        `import { requestIdentity } from './${modulePath}';\nimport { startWorkers } from './${helperPath}';\nexport { requestIdentity, startWorkers };\n`,
    );
    return root;
}

test('threaded runtime admits only the pinned worker-pool helper and its one engine re-import', async t => {
    const source = await pinnedHelperSource();
    if (source === undefined) {
        t.skip('wasm-bindgen-rayon 1.3.0 is not in the local Cargo registry');
        return;
    }
    const root = await threadedFixture(source);
    t.after(() => rm(root, { recursive: true, force: true }));
    const claims = await authorRuntimeJavascriptModules(root);
    assert.deepEqual(claims.map(claim => claim.path), [modulePath, helperPath]);
    await verifyRuntimeJavascriptModules(root, claims);

    const reshaped = await threadedFixture(`${source}\n// reshaped\n`);
    t.after(() => rm(reshaped, { recursive: true, force: true }));
    await assert.rejects(authorRuntimeJavascriptModules(reshaped), /not the pinned wasm-bindgen-rayon helper/u);

    // The exemption is bound to the helper path: the same dynamic import in
    // any other runtime module stays forbidden.
    const elsewhere = await fixture();
    t.after(() => rm(elsewhere, { recursive: true, force: true }));
    await writeFile(resolve(elsewhere, modulePath), 'export const requestIdentity = url => import(url);\n');
    await assert.rejects(authorRuntimeJavascriptModules(elsewhere), /dynamic, phased, or attributed import/u);
});

test('replay validator is an explicitly authorized standalone second entry', async t => {
    const root = await fixture();
    t.after(() => rm(root, { recursive: true, force: true }));
    await writeFile(resolve(root, 'replay_admission.js'), 'export function validate_compact_replay() {}');
    await assert.rejects(authorRuntimeJavascriptModules(root), /orphan/);
    const claims = await authorRuntimeJavascriptModules(root, { replayAdmission: true });
    assert.ok(claims.some(claim => claim.path === 'replay_admission.js'));
    await verifyRuntimeJavascriptModules(root, claims, { replayAdmission: true });
    await writeFile(resolve(root, 'replay_admission.js'), `import './${modulePath}';`);
    await assert.rejects(authorRuntimeJavascriptModules(root, { replayAdmission: true }), /standalone/);
});
