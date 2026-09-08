import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { chmod, lstat, mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { resolve } from 'node:path';
import test from 'node:test';
import { runOperatorDeployment } from './deploy-cloudflare-operator.mjs';
import { runOperatorRollback } from './rollback-cloudflare-operator.mjs';
import {
    createOperatorWranglerSnapshot,
    removeOperatorWranglerSnapshot,
    validateOperatorWranglerSnapshot,
} from './operator-deployment-bundle.mjs';

const accountId = 'ab'.repeat(16);
const zoneId = 'cd'.repeat(16);
const token = 'test-token-that-is-long-enough';
const versionIds = {
    public: '11111111-1111-4111-8111-111111111111',
    runtime: '22222222-2222-4222-8222-222222222222',
    signer: '33333333-3333-4333-8333-333333333333',
    datadir: '44444444-4444-4444-8444-444444444444',
    legacy: '50505050-5050-4050-8050-505050505050',
};
const previousVersionIds = {
    public: '10101010-1010-4010-8010-101010101010',
    runtime: '20202020-2020-4020-8020-202020202020',
    signer: '30303030-3030-4030-8030-303030303030',
};
const previousDeploymentIds = {
    public: 'a1010101-0101-4101-8101-010101010101',
    runtime: 'a2020202-0202-4202-8202-020202020202',
    signer: 'a3030303-0303-4303-8303-030303030303',
};
const routes = [
    { pattern: 'robinhood.phiresky.xyz/api*', script: null },
    { pattern: 'robinhood.phiresky.xyz/.well-known/acme-challenge/*', script: null },
    { pattern: 'robinhood.phiresky.xyz/wasm/*', script: 'robinhood-runtime-assets' },
    { pattern: 'robinhood.phiresky.xyz/datadirs/*', script: 'robinhood-datadir-assets' },
    { pattern: 'robinhood.phiresky.xyz/*', script: 'robinhood-public-site' },
];

async function fixture(t) {
    const root = await mkdtemp(resolve(tmpdir(), 'operator-cloudflare-deploy-'));
    t.after(() => rm(root, { force: true, recursive: true }));
    const repoRoot = resolve(root, 'repo');
    const wasmRoot = resolve(repoRoot, 'wasm-www');
    await mkdir(resolve(wasmRoot, 'node_modules/.bin'), { recursive: true });
    await mkdir(resolve(wasmRoot, 'deploy'));
    const wrangler = resolve(wasmRoot, 'node_modules/.bin/wrangler');
    await writeFile(wrangler, '#!/bin/sh\nexit 1\n');
    await chmod(wrangler, 0o755);
    await writeFile(resolve(wasmRoot, 'deploy/public-routes.json'), JSON.stringify({ routes }));
    const zoneAuditPath = resolve(root, 'zone-audit.json');
    await writeFile(zoneAuditPath, '{}');
    return { repoRoot, root, wasmRoot, zoneAuditPath };
}

function dependencies(value, calls) {
    const manifest = {
        datadir: {
            authority_sha256: '11'.repeat(32),
            deployment_receipt_sha256: '22'.repeat(32),
            inventory_sha256: '33'.repeat(32),
            public_root_url: 'https://robinhood.phiresky.xyz/datadirs/',
            route_pattern: 'robinhood.phiresky.xyz/datadirs/*',
            worker_name: 'robinhood-datadir-assets',
            worker_version_id: versionIds.datadir,
        },
        materialization: { approved_receipt_sha256: '99'.repeat(32) },
        publication_lock_sha256: '44'.repeat(32),
        source_commit: '55'.repeat(20),
    };
    const wranglerSnapshotAuthorities = Object.fromEntries(
        ['runtime', 'signer', 'public'].map(label => [label, { label }]),
    );
    return {
        apiReadinessImpl: async () => { calls.push('api-ready'); },
        createWranglerSnapshotImpl: async ({ label, stage }) => ({
            capPath: stage,
            configPath: `deploy/wrangler-${label}.json`,
            cwd: stage,
            label,
            sealedPath: stage,
        }),
        currentWorkerImpl: async ({ worker }) => {
            calls.push(`rollback:${worker}`);
            if (worker === 'robinhood') {
                return {
                    deploymentId: 'a5050505-0505-4505-8505-050505050505',
                    versionId: versionIds.legacy,
                };
            }
            const label = Object.entries({
                public: 'robinhood-public-site',
                runtime: 'robinhood-runtime-assets',
                signer: 'robinhood-identity-signer',
            }).find(([, name]) => name === worker)?.[0];
            if (label !== undefined) {
                return {
                    deploymentId: previousDeploymentIds[label],
                    versionId: previousVersionIds[label],
                };
            }
            return worker === 'robinhood-datadir-assets'
                ? { deploymentId: 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa', versionId: versionIds.datadir }
                : null;
        },
        execFileImpl: async (_file, args, options) => {
            if (args[0] === '--version') return { stderr: '', stdout: '4.127.1\n' };
            const config = args.at(-1);
            const label = config.match(/wrangler-(runtime|signer|public)\.json$/u)?.[1];
            assert(label);
            if (args.includes('--dry-run')) {
                calls.push(`dry:${label}`);
                return { stderr: '', stdout: '' };
            }
            calls.push(`deploy:${label}`);
            const worker = label === 'signer' ? 'robinhood-identity-signer' : `robinhood-${label === 'public' ? 'public-site' : 'runtime-assets'}`;
            await writeFile(options.env.WRANGLER_OUTPUT_FILE_PATH, `${JSON.stringify({
                type: 'deploy',
                version_id: versionIds[label],
                worker_name: worker,
            })}\n`);
            return { stderr: '', stdout: '' };
        },
        proveWorkerImpl: async ({ versionId, worker }) => {
            calls.push(`prove:${worker}:${versionId}`);
            return {
                current: {
                    deploymentId: worker === 'robinhood-datadir-assets'
                        ? 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa'
                        : 'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb',
                    versionId,
                },
                version: { id: versionId },
            };
        },
        reconcileRoutesImpl: async ({ apply }) => {
            calls.push(apply ? 'routes' : 'routes-final');
            // Cloudflare does not promise the bundle's authority order in
            // route-list responses.
            return { after: [...routes].reverse() };
        },
        removeWranglerSnapshotImpl: async () => {},
        smokeImpl: async () => { calls.push('smoke'); },
        stageImpl: async ({ stage }) => {
            await mkdir(resolve(stage, 'deploy'), { recursive: true });
            for (const label of ['runtime', 'signer', 'public']) {
                await writeFile(resolve(stage, 'deploy', `wrangler-${label}.json`), '{}');
            }
            return { manifest, stage, wranglerSnapshotAuthorities };
        },
        validateStageImpl: async ({ origin, phase }) => {
            calls.push(`stage:${phase}${origin === undefined ? '' : `:${origin}`}`);
            return { manifest };
        },
        validateBundleImpl: async () => ({ manifest, routes }),
        validateWranglerSnapshotImpl: async () => {},
        validateZoneAuditImpl: async () => {
            calls.push('zone-audit');
            const value = {
                approval: { retire_public_routes: ['robinhood.phiresky.xyz/api/*'] },
                snapshot: { worker_routes: [{ pattern: 'https://robinhood.phiresky.xyz/*', script: 'robinhood' }] },
            };
            return {
                ...value,
                auditSha256: createHash('sha256').update(`${JSON.stringify({
                    approval: value.approval,
                    schema_version: 1,
                    snapshot: value.snapshot,
                })}\n`).digest('hex'),
            };
        },
        verifyConfigImpl: async () => {},
    };
}

test('preflight performs every read-only gate and no deploy mutation', async t => {
    const value = await fixture(t);
    const calls = [];
    const result = await runOperatorDeployment({
        accountId,
        apiToken: token,
        bundle: 'unused',
        evidence: undefined,
        execute: false,
        expectedManifestSha256: '77'.repeat(32),
        expectedZoneAuditSha256: '88'.repeat(32),
        processNodeVersion: 'v24.19.0',
        repoRoot: value.repoRoot,
        stage: resolve(value.root, 'stage-preflight'),
        zoneAuditPath: value.zoneAuditPath,
        zoneId,
        ...dependencies(value, calls),
    });
    assert.equal(result.mutated, false);
    assert.deepEqual(calls, [
        'stage:post-stage',
        'stage:snapshot-pre:runtime', 'dry:runtime', 'stage:snapshot-post:runtime',
        'stage:snapshot-pre:signer', 'dry:signer', 'stage:snapshot-post:signer',
        'stage:snapshot-pre:public', 'dry:public', 'stage:snapshot-post:public',
        'zone-audit', 'api-ready',
        `prove:robinhood-datadir-assets:${versionIds.datadir}`,
    ]);
});

test('execute deploys runtime then signer/public, proves versions, reconciles routes, and records private rollback evidence', async t => {
    const value = await fixture(t);
    const calls = [];
    const evidence = resolve(value.root, 'evidence');
    const result = await runOperatorDeployment({
        accountId,
        apiToken: token,
        bundle: 'unused',
        evidence,
        execute: true,
        expectedManifestSha256: '77'.repeat(32),
        expectedZoneAuditSha256: '88'.repeat(32),
        processNodeVersion: 'v24.19.0',
        repoRoot: value.repoRoot,
        stage: resolve(value.root, 'stage-execute'),
        zoneAuditPath: value.zoneAuditPath,
        zoneId,
        ...dependencies(value, calls),
    });
    assert.equal(result.mutated, true);
    const deploymentOrder = calls.filter(item => item.startsWith('deploy:') || item === 'routes' || item === 'smoke');
    assert.deepEqual(deploymentOrder, ['deploy:runtime', 'deploy:signer', 'deploy:public', 'routes', 'smoke']);
    const receipt = JSON.parse(await readFile(resolve(evidence, 'deployment.json'), 'utf8'));
    const rollback = JSON.parse(await readFile(resolve(evidence, 'rollback.json'), 'utf8'));
    const rollbackBytes = await readFile(resolve(evidence, 'rollback.json'));
    const deploymentBytes = await readFile(resolve(evidence, 'deployment.json'));
    assert.equal(receipt.schema_version, 2);
    assert.equal(receipt.account_id, accountId);
    assert.equal(receipt.zone_id, zoneId);
    assert.equal(receipt.deployed.runtime.version_id, versionIds.runtime);
    assert.equal(receipt.deployed.runtime.worker_name, 'robinhood-runtime-assets');
    assert.equal(receipt.datadir.worker_version_id, versionIds.datadir);
    assert.equal(receipt.materialization_receipt_sha256, '99'.repeat(32));
    assert.deepEqual(receipt.routes, routes);
    assert.equal(receipt.rollback_evidence_sha256, createHash('sha256').update(rollbackBytes).digest('hex'));
    assert.equal(result.rollbackEvidenceSha256, createHash('sha256').update(rollbackBytes).digest('hex'));
    assert.equal(result.deploymentEvidenceSha256, createHash('sha256').update(deploymentBytes).digest('hex'));
    assert.equal(rollback.schema_version, 2);
    assert.equal(rollback.workers.datadir.version_id, versionIds.datadir);
    assert.equal(rollback.workers.datadir.worker_name, 'robinhood-datadir-assets');
    assert.deepEqual(rollback.previous_routes, [{
        pattern: 'https://robinhood.phiresky.xyz/*',
        script: 'robinhood',
    }]);
    assert.deepEqual(rollback.route_workers.robinhood, {
        deployment_id: 'a5050505-0505-4505-8505-050505050505',
        version_id: versionIds.legacy,
        worker_name: 'robinhood',
    });
    const identities = [
        ...Object.values(rollback.workers),
        ...Object.values(rollback.route_workers),
    ];
    const rollbackFetch = async (urlValue, options = {}) => {
        const url = new URL(urlValue);
        if (url.pathname === `/client/v4/zones/${zoneId}`) {
            return {
                ok: true,
                status: 200,
                async json() {
                    return {
                        errors: [],
                        result: { account: { id: accountId }, id: zoneId, name: 'phiresky.xyz', status: 'active' },
                        success: true,
                    };
                },
            };
        }
        if (url.pathname === `/client/v4/zones/${zoneId}/workers/routes`) {
            return {
                ok: true,
                status: 200,
                async json() {
                    return {
                        errors: [],
                        result: [...routes].reverse().map((route, index) => ({ id: `${index + 1}`.repeat(32), ...route })),
                        success: true,
                    };
                },
            };
        }
        const identity = identities.find(item => url.pathname.endsWith(`/deployments/${item.deployment_id}`)
            || url.pathname.endsWith(`/versions/${item.version_id}`));
        assert(identity, `unexpected rollback ABI request ${options.method ?? 'GET'} ${url.pathname}`);
        const resultValue = url.pathname.includes('/deployments/')
            ? { id: identity.deployment_id, versions: [{ percentage: 100, version_id: identity.version_id }] }
            : { id: identity.version_id };
        return {
            ok: true,
            status: 200,
            async json() { return { errors: [], result: resultValue, success: true }; },
        };
    };
    const deployedByWorker = new Map(Object.values(receipt.deployed)
        .map(identity => [identity.worker_name, {
            deploymentId: identity.deployment_id,
            versionId: identity.version_id,
        }]));
    deployedByWorker.set(rollback.workers.datadir.worker_name, {
        deploymentId: rollback.workers.datadir.deployment_id,
        versionId: rollback.workers.datadir.version_id,
    });
    deployedByWorker.set(rollback.route_workers.robinhood.worker_name, {
        deploymentId: rollback.route_workers.robinhood.deployment_id,
        versionId: rollback.route_workers.robinhood.version_id,
    });
    const rollbackPreflight = await runOperatorRollback({
        accountId,
        apiToken: token,
        bundle: 'unused',
        currentWorkerImpl: async ({ worker }) => deployedByWorker.get(worker),
        deploymentSha256: result.deploymentEvidenceSha256,
        evidence,
        execute: false,
        expectedManifestSha256: '77'.repeat(32),
        fetchImpl: rollbackFetch,
        processNodeVersion: 'v24.19.0',
        repoRoot: value.repoRoot,
        requireUserTokenImpl: async () => {},
        rollbackSha256: result.rollbackEvidenceSha256,
        smokeImpl: async () => {},
        validateBundleImpl: async () => ({
            manifest: {
                datadir: receipt.datadir,
                materialization: { approved_receipt_sha256: receipt.materialization_receipt_sha256 },
                publication_lock_sha256: receipt.publication_lock_sha256,
                source_commit: receipt.source_commit,
            },
            routes,
        }),
        verifyConfigImpl: async () => {},
        zoneId,
    });
    assert.equal(rollbackPreflight.mutated, false);
    assert.ok(calls.indexOf('stage:pre-live') < calls.indexOf('stage:origin:runtime'));
    assert.ok(calls.indexOf('stage:origin:runtime') < calls.indexOf('deploy:runtime'));
    assert.ok(calls.indexOf('stage:origin:signer') < calls.indexOf('deploy:signer'));
    assert.ok(calls.indexOf('stage:origin:public') < calls.indexOf('deploy:public'));
});

test('a delayed stage substitution after runtime upload blocks the next origin before upload or routes', async t => {
    const value = await fixture(t);
    const calls = [];
    let tampered = false;
    const deps = dependencies(value, calls);
    const originalExec = deps.execFileImpl;
    deps.execFileImpl = async (...args) => {
        const result = await originalExec(...args);
        if (calls.at(-1) === 'deploy:runtime') tampered = true;
        return result;
    };
    const originalValidateStage = deps.validateStageImpl;
    deps.validateStageImpl = async options => {
        if (options.phase === 'origin' && options.origin === 'signer' && tampered) {
            calls.push('stage:origin:signer:rejected');
            throw new Error('delayed stage substitution');
        }
        return originalValidateStage(options);
    };
    await assert.rejects(runOperatorDeployment({
        accountId,
        apiToken: token,
        bundle: 'unused',
        evidence: resolve(value.root, 'evidence-delayed'),
        execute: true,
        expectedManifestSha256: '77'.repeat(32),
        expectedZoneAuditSha256: '88'.repeat(32),
        processNodeVersion: 'v24.19.0',
        repoRoot: value.repoRoot,
        stage: resolve(value.root, 'stage-delayed'),
        zoneAuditPath: value.zoneAuditPath,
        zoneId,
        ...deps,
    }), /delayed stage substitution/u);
    assert.ok(calls.includes('deploy:runtime'));
    assert.ok(calls.includes('stage:origin:signer:rejected'));
    assert.ok(!calls.includes('deploy:signer'));
    assert.ok(!calls.includes('routes'));
});

test('the immediate pre-live closure proof blocks the first Worker mutation', async t => {
    const value = await fixture(t);
    const calls = [];
    const deps = dependencies(value, calls);
    const originalValidateStage = deps.validateStageImpl;
    deps.validateStageImpl = async options => {
        if (options.phase === 'pre-live') {
            calls.push('stage:pre-live:rejected');
            throw new Error('pre-live stage substitution');
        }
        return originalValidateStage(options);
    };
    await assert.rejects(runOperatorDeployment({
        accountId,
        apiToken: token,
        bundle: 'unused',
        evidence: resolve(value.root, 'evidence-pre-live'),
        execute: true,
        expectedManifestSha256: '77'.repeat(32),
        expectedZoneAuditSha256: '88'.repeat(32),
        processNodeVersion: 'v24.19.0',
        repoRoot: value.repoRoot,
        stage: resolve(value.root, 'stage-pre-live'),
        zoneAuditPath: value.zoneAuditPath,
        zoneId,
        ...deps,
    }), /pre-live stage substitution/u);
    assert.ok(calls.includes('stage:pre-live:rejected'));
    assert.ok(!calls.some(call => call.startsWith('deploy:')));
    assert.ok(!calls.includes('routes'));
});

test('a previous Worker drift after capture blocks the corresponding upload', async t => {
    const value = await fixture(t);
    const calls = [];
    const deps = dependencies(value, calls);
    const originalCurrent = deps.currentWorkerImpl;
    let runtimeReads = 0;
    deps.currentWorkerImpl = async options => {
        const current = await originalCurrent(options);
        if (options.worker === 'robinhood-runtime-assets' && ++runtimeReads === 2) {
            return {
                deploymentId: 'd2020202-0202-4202-8202-020202020202',
                versionId: previousVersionIds.runtime,
            };
        }
        return current;
    };
    await assert.rejects(runOperatorDeployment({
        accountId,
        apiToken: token,
        bundle: 'unused',
        evidence: resolve(value.root, 'evidence-rollback-capture-drift'),
        execute: true,
        expectedManifestSha256: '77'.repeat(32),
        expectedZoneAuditSha256: '88'.repeat(32),
        processNodeVersion: 'v24.19.0',
        repoRoot: value.repoRoot,
        stage: resolve(value.root, 'stage-rollback-capture-drift'),
        zoneAuditPath: value.zoneAuditPath,
        zoneId,
        ...deps,
    }), /runtime Worker changed after rollback capture/u);
    assert.ok(!calls.some(call => call.startsWith('deploy:')));
    assert.ok(!calls.includes('routes'));
});

test('post-smoke signer, public, and legacy route Worker drift prevent terminal deployment evidence', async t => {
    for (const drift of ['signer', 'public', 'legacy']) {
        await t.test(drift, async t2 => {
            const value = await fixture(t2);
            const calls = [];
            const deps = dependencies(value, calls);
            if (drift === 'legacy') {
                const originalCurrent = deps.currentWorkerImpl;
                let reads = 0;
                deps.currentWorkerImpl = async options => {
                    const current = await originalCurrent(options);
                    if (options.worker === 'robinhood' && ++reads === 3) {
                        return {
                            deploymentId: 'd5050505-0505-4505-8505-050505050505',
                            versionId: versionIds.legacy,
                        };
                    }
                    return current;
                };
            } else {
                const worker = drift === 'signer'
                    ? 'robinhood-identity-signer'
                    : 'robinhood-public-site';
                const originalProve = deps.proveWorkerImpl;
                let proofs = 0;
                deps.proveWorkerImpl = async options => {
                    const proof = await originalProve(options);
                    if (options.worker === worker && ++proofs === 3) {
                        return {
                            ...proof,
                            current: {
                                ...proof.current,
                                deploymentId: 'dddddddd-dddd-4ddd-8ddd-dddddddddddd',
                            },
                        };
                    }
                    return proof;
                };
            }
            const evidence = resolve(value.root, `evidence-final-${drift}-drift`);
            await assert.rejects(runOperatorDeployment({
                accountId,
                apiToken: token,
                bundle: 'unused',
                evidence,
                execute: true,
                expectedManifestSha256: '77'.repeat(32),
                expectedZoneAuditSha256: '88'.repeat(32),
                processNodeVersion: 'v24.19.0',
                repoRoot: value.repoRoot,
                stage: resolve(value.root, `stage-final-${drift}-drift`),
                zoneAuditPath: value.zoneAuditPath,
                zoneId,
                ...deps,
            }), /Worker changed/u);
            assert.ok(calls.includes('smoke'));
            assert.equal(await lstat(resolve(evidence, 'deployment.json')).catch(() => undefined), undefined);
        });
    }
});

test('deployment redacts reflected API tokens from imported helper failures', async t => {
    const value = await fixture(t);
    const calls = [];
    const deps = dependencies(value, calls);
    deps.validateZoneAuditImpl = async () => {
        throw new Error(`Cloudflare reflected bearer ${token}`);
    };
    let failure;
    try {
        await runOperatorDeployment({
            accountId,
            apiToken: token,
            bundle: 'unused',
            evidence: undefined,
            execute: false,
            expectedManifestSha256: '77'.repeat(32),
            expectedZoneAuditSha256: '88'.repeat(32),
            processNodeVersion: 'v24.19.0',
            repoRoot: value.repoRoot,
            stage: resolve(value.root, 'stage-token-redaction'),
            zoneAuditPath: value.zoneAuditPath,
            zoneId,
            ...deps,
        });
    } catch (error) {
        failure = error;
    }
    assert.ok(failure instanceof Error);
    assert.ok(!failure.message.includes(token));
    assert.match(failure.message, /\[REDACTED\]/u);
});

test('the exact Node gate rejects before staging or evidence writes', async t => {
    for (const processNodeVersion of ['v24.18.0', 'v26.0.0']) {
        await t.test(processNodeVersion, async t2 => {
            const value = await fixture(t2);
            const calls = [];
            const stage = resolve(value.root, `stage-wrong-node-${processNodeVersion}`);
            const evidence = resolve(value.root, `evidence-wrong-node-${processNodeVersion}`);
            await assert.rejects(runOperatorDeployment({
                accountId,
                apiToken: token,
                bundle: 'unused',
                evidence,
                execute: true,
                expectedManifestSha256: '77'.repeat(32),
                expectedZoneAuditSha256: '88'.repeat(32),
                processNodeVersion,
                repoRoot: value.repoRoot,
                stage,
                zoneAuditPath: value.zoneAuditPath,
                zoneId,
                ...dependencies(value, calls),
            }), /requires exact Node\.js v24\.19\.0/u);
            assert.deepEqual(calls, []);
            assert.equal(await lstat(stage).catch(() => undefined), undefined);
            assert.equal(await lstat(evidence).catch(() => undefined), undefined);
        });
    }
});

test('every dry run and live upload is enclosed by a fresh validated and removed snapshot', async t => {
    const value = await fixture(t);
    const calls = [];
    const deps = dependencies(value, calls);
    const validations = new Map();
    deps.createWranglerSnapshotImpl = async ({ label, stage }) => {
        calls.push(`snapshot:create:${label}`);
        return {
            capPath: stage,
            configPath: `deploy/wrangler-${label}.json`,
            cwd: stage,
            label,
            sealedPath: stage,
        };
    };
    deps.validateWranglerSnapshotImpl = async ({ label }) => {
        const count = (validations.get(label) ?? 0) + 1;
        validations.set(label, count);
        calls.push(`snapshot:${count % 2 === 1 ? 'pre' : 'post'}:${label}`);
    };
    deps.removeWranglerSnapshotImpl = async ({ label }) => { calls.push(`snapshot:remove:${label}`); };
    await runOperatorDeployment({
        accountId,
        apiToken: token,
        bundle: 'unused',
        evidence: resolve(value.root, 'evidence-snapshot-order'),
        execute: true,
        expectedManifestSha256: '77'.repeat(32),
        expectedZoneAuditSha256: '88'.repeat(32),
        processNodeVersion: 'v24.19.0',
        repoRoot: value.repoRoot,
        stage: resolve(value.root, 'stage-snapshot-order'),
        zoneAuditPath: value.zoneAuditPath,
        zoneId,
        ...deps,
    });
    const observed = calls.filter(call => call.startsWith('snapshot:')
        || call.startsWith('stage:snapshot-') || call.startsWith('dry:') || call.startsWith('deploy:'));
    const expected = [];
    for (const action of ['dry', 'deploy']) {
        for (const label of ['runtime', 'signer', 'public']) {
            expected.push(
                `snapshot:create:${label}`,
                `snapshot:pre:${label}`,
                `stage:snapshot-pre:${label}`,
                `${action}:${label}`,
                `snapshot:post:${label}`,
                `stage:snapshot-post:${label}`,
                `snapshot:remove:${label}`,
            );
        }
    }
    assert.deepEqual(observed, expected);
});

test('runtime and signer-to-public stage substitutions stop the next upload', async t => {
    for (const [origin, expectedUploads] of [
        ['runtime', []],
        ['public', ['deploy:runtime', 'deploy:signer']],
    ]) {
        await t.test(origin, async t2 => {
            const value = await fixture(t2);
            const calls = [];
            const deps = dependencies(value, calls);
            const originalValidateStage = deps.validateStageImpl;
            deps.validateStageImpl = async options => {
                if (options.phase === 'origin' && options.origin === origin) {
                    calls.push(`stage:origin:${origin}:rejected`);
                    throw new Error(`${origin} origin changed before upload`);
                }
                return originalValidateStage(options);
            };
            await assert.rejects(runOperatorDeployment({
                accountId,
                apiToken: token,
                bundle: 'unused',
                evidence: resolve(value.root, `evidence-origin-${origin}`),
                execute: true,
                expectedManifestSha256: '77'.repeat(32),
                expectedZoneAuditSha256: '88'.repeat(32),
                processNodeVersion: 'v24.19.0',
                repoRoot: value.repoRoot,
                stage: resolve(value.root, `stage-origin-${origin}`),
                zoneAuditPath: value.zoneAuditPath,
                zoneId,
                ...deps,
            }), new RegExp(`${origin} origin changed before upload`, 'u'));
            assert.deepEqual(calls.filter(call => call.startsWith('deploy:')), expectedUploads);
            assert.ok(calls.includes(`stage:origin:${origin}:rejected`));
            assert.ok(!calls.includes('routes'));
        });
    }
});

test('snapshot authority, content, config, mode, and closure tampering fail both boundaries and always clean up', async t => {
    for (const kind of ['authority', 'content', 'config', 'mode', 'closure']) {
        for (const phase of ['pre', 'post']) {
            await t.test(`${kind}-${phase}`, async t2 => {
                const value = await fixture(t2);
                const calls = [];
                const deps = dependencies(value, calls);
                let runtimeValidations = 0;
                deps.createWranglerSnapshotImpl = async ({ label, stage }) => {
                    calls.push(`snapshot:create:${label}`);
                    return {
                        capPath: stage,
                        configPath: `deploy/wrangler-${label}.json`,
                        cwd: stage,
                        label,
                        sealedPath: stage,
                    };
                };
                deps.validateWranglerSnapshotImpl = async ({ label }) => {
                    calls.push(`snapshot:validate:${label}`);
                    if (label !== 'runtime') return;
                    runtimeValidations += 1;
                    const boundary = runtimeValidations === 1 ? 'pre' : 'post';
                    if (boundary === phase) throw new Error(`snapshot ${kind} ${phase} tamper`);
                };
                deps.removeWranglerSnapshotImpl = async ({ label }) => { calls.push(`snapshot:remove:${label}`); };
                await assert.rejects(runOperatorDeployment({
                    accountId,
                    apiToken: token,
                    bundle: 'unused',
                    evidence: undefined,
                    execute: false,
                    expectedManifestSha256: '77'.repeat(32),
                    expectedZoneAuditSha256: '88'.repeat(32),
                    processNodeVersion: 'v24.19.0',
                    repoRoot: value.repoRoot,
                    stage: resolve(value.root, `stage-snapshot-${kind}-${phase}`),
                    zoneAuditPath: value.zoneAuditPath,
                    zoneId,
                    ...deps,
                }), new RegExp(`snapshot ${kind} ${phase} tamper`, 'u'));
                assert.ok(calls.includes('snapshot:create:runtime'));
                assert.ok(calls.includes('snapshot:remove:runtime'));
                assert.equal(calls.includes('dry:runtime'), phase === 'post');
                assert.ok(!calls.includes('dry:signer'));
            });
        }
    }
});

test('a missing snapshot authority is rejected before snapshot creation or Wrangler', async t => {
    const value = await fixture(t);
    const calls = [];
    const deps = dependencies(value, calls);
    const originalStage = deps.stageImpl;
    deps.stageImpl = async options => {
        const staged = await originalStage(options);
        delete staged.wranglerSnapshotAuthorities.runtime;
        return staged;
    };
    deps.createWranglerSnapshotImpl = async () => {
        calls.push('snapshot:create');
        throw new Error('must not create');
    };
    await assert.rejects(runOperatorDeployment({
        accountId,
        apiToken: token,
        bundle: 'unused',
        evidence: undefined,
        execute: false,
        expectedManifestSha256: '77'.repeat(32),
        expectedZoneAuditSha256: '88'.repeat(32),
        processNodeVersion: 'v24.19.0',
        repoRoot: value.repoRoot,
        stage: resolve(value.root, 'stage-missing-snapshot-authority'),
        zoneAuditPath: value.zoneAuditPath,
        zoneId,
        ...deps,
    }), /omits Wrangler runtime snapshot authority/u);
    assert.ok(!calls.includes('snapshot:create'));
    assert.ok(!calls.includes('dry:runtime'));
});

test('Wrangler action, post-validation, and cleanup failures are aggregated without skipping cleanup', async t => {
    const value = await fixture(t);
    const calls = [];
    const deps = dependencies(value, calls);
    let runtimeValidations = 0;
    deps.execFileImpl = async (_file, args) => {
        if (args[0] === '--version') return { stderr: '', stdout: '4.127.1\n' };
        calls.push('wrangler:action');
        throw new Error('Wrangler action failed');
    };
    deps.validateWranglerSnapshotImpl = async ({ label }) => {
        if (label === 'runtime' && ++runtimeValidations === 2) throw new Error('snapshot post-validation failed');
    };
    deps.removeWranglerSnapshotImpl = async ({ label }) => {
        calls.push(`snapshot:cleanup:${label}`);
        if (label === 'runtime') throw new Error('snapshot cleanup failed');
    };
    let failure;
    try {
        await runOperatorDeployment({
            accountId,
            apiToken: token,
            bundle: 'unused',
            evidence: undefined,
            execute: false,
            expectedManifestSha256: '77'.repeat(32),
            expectedZoneAuditSha256: '88'.repeat(32),
            processNodeVersion: 'v24.19.0',
            repoRoot: value.repoRoot,
            stage: resolve(value.root, 'stage-snapshot-aggregate'),
            zoneAuditPath: value.zoneAuditPath,
            zoneId,
            ...deps,
        });
    } catch (error) {
        failure = error;
    }
    assert.ok(failure instanceof AggregateError);
    assert.deepEqual(failure.errors.map(error => error.message), [
        'Wrangler action failed',
        'snapshot post-validation failed',
        'snapshot cleanup failed',
    ]);
    assert.ok(calls.includes('snapshot:cleanup:runtime'));
});

test('dry-run Wrangler subprocesses scrub ambient output and Node injection variables', async t => {
    const value = await fixture(t);
    const calls = [];
    const deps = dependencies(value, calls);
    const ambient = {
        CLOUDFLARE_ACCOUNT_ID: process.env.CLOUDFLARE_ACCOUNT_ID,
        CLOUDFLARE_API_BASE_URL: process.env.CLOUDFLARE_API_BASE_URL,
        CLOUDFLARE_API_TOKEN: process.env.CLOUDFLARE_API_TOKEN,
        NODE_OPTIONS: process.env.NODE_OPTIONS,
        NODE_PATH: process.env.NODE_PATH,
        WRANGLER_AMBIENT_TEST: process.env.WRANGLER_AMBIENT_TEST,
        WRANGLER_OUTPUT_FILE_PATH: process.env.WRANGLER_OUTPUT_FILE_PATH,
    };
    Object.assign(process.env, {
        CLOUDFLARE_ACCOUNT_ID: 'ambient-account',
        CLOUDFLARE_API_BASE_URL: 'https://attacker.invalid',
        CLOUDFLARE_API_TOKEN: 'ambient-token',
        NODE_OPTIONS: '--require=/definitely/not/allowed.cjs',
        NODE_PATH: '/definitely/not/allowed',
        WRANGLER_AMBIENT_TEST: 'must-be-removed',
        WRANGLER_OUTPUT_FILE_PATH: '/tmp/ambient-wrangler-output-must-not-be-used',
    });
    deps.execFileImpl = async (_file, args, options) => {
        if (args[0] === '--version') return { stderr: '', stdout: '4.127.1\n' };
        assert.ok(args.includes('--dry-run'));
        assert.equal(options.env.NODE_OPTIONS, undefined);
        assert.equal(options.env.NODE_PATH, undefined);
        assert.equal(options.env.WRANGLER_AMBIENT_TEST, undefined);
        assert.equal(options.env.WRANGLER_OUTPUT_FILE_PATH, undefined);
        assert.equal(options.env.CLOUDFLARE_ACCOUNT_ID, accountId);
        assert.equal(options.env.CLOUDFLARE_API_TOKEN, token);
        assert.equal(options.env.CLOUDFLARE_API_BASE_URL, undefined);
        assert.equal(options.env.WRANGLER_SEND_METRICS, 'false');
        assert.equal(options.env.WRANGLER_NO_SKILLS_UPDATE_PROMPTS, 'true');
        assert.match(options.env.WRANGLER_LOG_PATH, /\/\.wrangler-logs$/u);
        assert.match(options.env.HOME, /\/\.home$/u);
        assert.match(options.env.XDG_CACHE_HOME, /\/\.xdg-cache$/u);
        assert.match(options.env.XDG_CONFIG_HOME, /\/\.xdg-config$/u);
        assert.notEqual(options.env.WRANGLER_LOG_PATH, process.env.WRANGLER_LOG_PATH);
        calls.push(`scrubbed:${args.at(-1)}`);
        return { stderr: '', stdout: '' };
    };
    try {
        await runOperatorDeployment({
            accountId,
            apiToken: token,
            bundle: 'unused',
            evidence: undefined,
            execute: false,
            expectedManifestSha256: '77'.repeat(32),
            expectedZoneAuditSha256: '88'.repeat(32),
            processNodeVersion: 'v24.19.0',
            repoRoot: value.repoRoot,
            stage: resolve(value.root, 'stage-env-scrub'),
            zoneAuditPath: value.zoneAuditPath,
            zoneId,
            ...deps,
        });
    } finally {
        for (const [key, previous] of Object.entries(ambient)) {
            if (previous === undefined) delete process.env[key];
            else process.env[key] = previous;
        }
    }
    assert.equal(calls.filter(call => call.startsWith('scrubbed:')).length, 3);
});

test('deployment preflight can use the real snapshot filesystem helpers and removes every snapshot', async t => {
    const value = await fixture(t);
    const calls = [];
    const deps = dependencies(value, calls);
    const origins = { public: 'dist', runtime: 'runtime-dist', signer: 'signer-dist' };
    const removed = [];
    deps.stageImpl = async ({ stage }) => {
        await mkdir(resolve(stage, 'deploy'), { recursive: true });
        const wranglerSnapshotAuthorities = {};
        for (const label of ['runtime', 'signer', 'public']) {
            const origin = origins[label];
            const bytes = Buffer.from(`approved-${label}-asset\n`);
            const config = Buffer.from('{}\n');
            await mkdir(resolve(stage, origin), { recursive: true });
            await writeFile(resolve(stage, origin, 'asset.txt'), bytes);
            await writeFile(resolve(stage, 'deploy', `wrangler-${label}.json`), config);
            wranglerSnapshotAuthorities[label] = {
                configByteLength: config.length,
                configName: `wrangler-${label}.json`,
                configSha256: createHash('sha256').update(config).digest('hex'),
                directories: ['.'],
                files: {
                    'asset.txt': {
                        byteLength: bytes.length,
                        sha256: createHash('sha256').update(bytes).digest('hex'),
                    },
                },
                origin,
            };
        }
        return {
            manifest: (await deps.validateBundleImpl()).manifest,
            stage,
            wranglerSnapshotAuthorities,
        };
    };
    deps.createWranglerSnapshotImpl = async options => {
        const snapshot = await createOperatorWranglerSnapshot(options);
        removed.push(snapshot.path);
        return snapshot;
    };
    deps.validateWranglerSnapshotImpl = validateOperatorWranglerSnapshot;
    deps.removeWranglerSnapshotImpl = removeOperatorWranglerSnapshot;
    await runOperatorDeployment({
        accountId,
        apiToken: token,
        bundle: 'unused',
        evidence: undefined,
        execute: false,
        expectedManifestSha256: '77'.repeat(32),
        expectedZoneAuditSha256: '88'.repeat(32),
        processNodeVersion: 'v24.19.0',
        repoRoot: value.repoRoot,
        stage: resolve(value.root, 'stage-real-snapshots'),
        zoneAuditPath: value.zoneAuditPath,
        zoneId,
        ...deps,
    });
    assert.equal(removed.length, 3);
    for (const path of removed) assert.equal(await lstat(path).catch(() => undefined), undefined);
});

test('live routes and approved audit evidence use frozen validated objects, never raced path rereads', async t => {
    const value = await fixture(t);
    const calls = [];
    const deps = dependencies(value, calls);
    const originalStage = deps.stageImpl;
    deps.stageImpl = async options => {
        const staged = await originalStage(options);
        await writeFile(resolve(value.wasmRoot, 'deploy/public-routes.json'), JSON.stringify({
            routes: [{ pattern: 'attacker.invalid/*', script: 'attacker' }],
        }));
        return staged;
    };
    deps.reconcileRoutesImpl = async ({ expectedRoutes }) => {
        assert.deepEqual(expectedRoutes, routes);
        return { after: routes };
    };
    const approval = { retire_public_routes: ['robinhood.phiresky.xyz/api/*'] };
    const snapshot = { worker_routes: [{ pattern: 'robinhood.phiresky.xyz/*', script: 'robinhood' }] };
    const approvedBytes = Buffer.from(`${JSON.stringify({ approval, schema_version: 1, snapshot })}\n`);
    deps.validateZoneAuditImpl = async () => {
        await writeFile(value.zoneAuditPath, '{"attacker":true}\n');
        return {
            approval,
            auditSha256: createHash('sha256').update(approvedBytes).digest('hex'),
            snapshot,
        };
    };
    const evidence = resolve(value.root, 'evidence-frozen-inputs');
    await runOperatorDeployment({
        accountId,
        apiToken: token,
        bundle: 'unused',
        evidence,
        execute: true,
        expectedManifestSha256: '77'.repeat(32),
        expectedZoneAuditSha256: '88'.repeat(32),
        processNodeVersion: 'v24.19.0',
        repoRoot: value.repoRoot,
        stage: resolve(value.root, 'stage-frozen-inputs'),
        zoneAuditPath: value.zoneAuditPath,
        zoneId,
        ...deps,
    });
    assert.deepEqual(await readFile(resolve(evidence, 'approved-zone-audit.json')), approvedBytes);
});
