import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import {
    chmod,
    lstat,
    mkdir,
    mkdtemp,
    readFile,
    readdir,
    rm,
    writeFile,
} from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { resolve } from 'node:path';
import test from 'node:test';
import { runOperatorRollback } from './rollback-cloudflare-operator.mjs';

const accountId = 'ab'.repeat(16);
const zoneId = 'cd'.repeat(16);
const apiToken = 'private-test-token-never-log-this';
const manifestSha256 = '77'.repeat(32);
const labels = ['runtime', 'signer', 'public'];
const workers = {
    datadir: 'robinhood-datadir-assets',
    public: 'robinhood-public-site',
    runtime: 'robinhood-runtime-assets',
    signer: 'robinhood-identity-signer',
};
const oldVersions = {
    datadir: '40404040-4040-4040-8040-404040404040',
    public: '10101010-1010-4010-8010-101010101010',
    runtime: '20202020-2020-4020-8020-202020202020',
    signer: '30303030-3030-4030-8030-303030303030',
};
const newVersions = {
    public: '11111111-1111-4111-8111-111111111111',
    runtime: '22222222-2222-4222-8222-222222222222',
    signer: '33333333-3333-4333-8333-333333333333',
};
const oldDeployments = {
    datadir: 'a4040404-0404-4404-8404-040404040404',
    public: 'a1010101-0101-4101-8101-010101010101',
    runtime: 'a2020202-0202-4202-8202-020202020202',
    signer: 'a3030303-0303-4303-8303-030303030303',
};
const newDeployments = {
    public: 'b1111111-1111-4111-8111-111111111111',
    runtime: 'b2222222-2222-4222-8222-222222222222',
    signer: 'b3333333-3333-4333-8333-333333333333',
};
const deployedRoutes = [
    { pattern: 'robinhood.phiresky.xyz/api*', script: null },
    { pattern: 'robinhood.phiresky.xyz/.well-known/acme-challenge/*', script: null },
    { pattern: 'robinhood.phiresky.xyz/wasm/*', script: workers.runtime },
    { pattern: 'robinhood.phiresky.xyz/datadirs/*', script: workers.datadir },
    { pattern: 'robinhood.phiresky.xyz/*', script: workers.public },
];
const previousRoutes = [
    { pattern: 'http://robinhood.phiresky.xyz/api/*', script: null },
    { pattern: 'https://robinhood.phiresky.xyz/api/*', script: null },
    { pattern: 'robinhood.phiresky.xyz/.well-known/acme-challenge/*', script: null },
    { pattern: 'robinhood.phiresky.xyz/wasm/*', script: workers.runtime },
    { pattern: 'robinhood.phiresky.xyz/datadirs/*', script: workers.datadir },
    { pattern: 'robinhood.phiresky.xyz/*', script: workers.public },
];

function utf8Order(left, right) {
    return Buffer.compare(Buffer.from(left, 'utf8'), Buffer.from(right, 'utf8'));
}

function canonical(value) {
    if (Array.isArray(value)) return value.map(canonical);
    if (value !== null && typeof value === 'object') {
        return Object.fromEntries(Object.entries(value)
            .sort(([left], [right]) => utf8Order(left, right))
            .map(([key, item]) => [key, canonical(item)]));
    }
    return value;
}

function bytes(value) {
    return Buffer.from(`${JSON.stringify(canonical(value))}\n`);
}

function digest(value) {
    return createHash('sha256').update(value).digest('hex');
}

function identity(label, old) {
    return {
        deployment_id: old ? oldDeployments[label] : newDeployments[label],
        version_id: old ? oldVersions[label] : newVersions[label],
        worker_name: workers[label],
    };
}

async function fixture(t) {
    const root = await mkdtemp(resolve(tmpdir(), 'operator-cloudflare-rollback-'));
    t.after(() => rm(root, { recursive: true, force: true }));
    const repoRoot = resolve(root, 'repo');
    const evidence = resolve(root, 'evidence');
    const transaction = resolve(root, 'transaction');
    await mkdir(resolve(repoRoot, 'wasm-www/deploy'), { recursive: true });
    await mkdir(evidence, { mode: 0o700 });
    await chmod(evidence, 0o700);
    const manifest = {
        datadir: {
            authority_sha256: '11'.repeat(32),
            deployment_receipt_sha256: '22'.repeat(32),
            inventory_sha256: '33'.repeat(32),
            public_root_url: 'https://robinhood.phiresky.xyz/datadirs/',
            route_pattern: 'robinhood.phiresky.xyz/datadirs/*',
            worker_name: workers.datadir,
            worker_version_id: oldVersions.datadir,
        },
        materialization: { approved_receipt_sha256: '99'.repeat(32) },
        publication_lock_sha256: '44'.repeat(32),
        source_commit: '55'.repeat(20),
    };
    const rollback = canonical({
        account_id: accountId,
        deployment_manifest_sha256: manifestSha256,
        previous_routes: previousRoutes,
        public_host: 'robinhood.phiresky.xyz',
        route_workers: {},
        schema_version: 2,
        source_commit: manifest.source_commit,
        workers: {
            datadir: {
                deployment_id: oldDeployments.datadir,
                version_id: oldVersions.datadir,
                worker_name: workers.datadir,
            },
            ...Object.fromEntries(labels.map(label => [label, identity(label, true)])),
        },
        zone_audit_sha256: '88'.repeat(32),
        zone_id: zoneId,
    });
    const rollbackBytes = bytes(rollback);
    const rollbackSha256 = digest(rollbackBytes);
    const deployment = canonical({
        account_id: accountId,
        datadir: manifest.datadir,
        deployed: Object.fromEntries(labels.map(label => [label, identity(label, false)])),
        deployment_manifest_sha256: manifestSha256,
        finished_at: '2026-09-01T12:00:00.000Z',
        materialization_receipt_sha256: manifest.materialization.approved_receipt_sha256,
        publication_lock_sha256: manifest.publication_lock_sha256,
        rollback_evidence_sha256: rollbackSha256,
        routes: deployedRoutes,
        schema_version: 2,
        source_commit: manifest.source_commit,
        zone_audit_sha256: rollback.zone_audit_sha256,
        zone_id: zoneId,
    });
    const deploymentBytes = bytes(deployment);
    const deploymentSha256 = digest(deploymentBytes);
    await writeFile(resolve(evidence, 'rollback.json'), rollbackBytes, { mode: 0o600 });
    await writeFile(resolve(evidence, 'deployment.json'), deploymentBytes, { mode: 0o600 });
    await chmod(resolve(evidence, 'rollback.json'), 0o600);
    await chmod(resolve(evidence, 'deployment.json'), 0o600);
    return {
        deployment,
        deploymentSha256,
        evidence,
        manifest,
        repoRoot,
        rollback,
        rollbackSha256,
        root,
        transaction,
    };
}

async function rewriteRollbackAuthority(value, mutate) {
    mutate(value.rollback);
    const rollbackBytes = bytes(value.rollback);
    value.rollbackSha256 = digest(rollbackBytes);
    await writeFile(resolve(value.evidence, 'rollback.json'), rollbackBytes);
    value.deployment.rollback_evidence_sha256 = value.rollbackSha256;
    const deploymentBytes = bytes(value.deployment);
    value.deploymentSha256 = digest(deploymentBytes);
    await writeFile(resolve(value.evidence, 'deployment.json'), deploymentBytes);
}

function response(result, { ok = true, status = 200, errors = [] } = {}) {
    return {
        headers: new Headers(),
        ok,
        status,
        async json() { return { errors, result, success: ok }; },
    };
}

function cloudflareMock() {
    const state = {
        calls: [],
        nextDeployment: 0,
        routes: deployedRoutes.map((route, index) => ({ id: `${index + 1}`.repeat(32), ...route })),
        workers: {
            datadir: { deploymentId: oldDeployments.datadir, versionId: oldVersions.datadir },
            ...Object.fromEntries(labels.map(label => [label, {
                deploymentId: newDeployments[label], versionId: newVersions[label],
            }])),
        },
    };
    const labelForWorker = worker => Object.entries(workers).find(([, name]) => name === worker)?.[0];
    const fetchImpl = async (urlValue, options = {}) => {
        const url = new URL(urlValue);
        const method = options.method ?? 'GET';
        state.calls.push({ body: options.body, method, url: url.href });
        if (url.pathname === `/client/v4/zones/${zoneId}` && method === 'GET') {
            return response({ account: { id: accountId }, id: zoneId, name: 'phiresky.xyz', status: 'active' });
        }
        if (url.pathname === `/client/v4/zones/${zoneId}/workers/routes`) {
            if (method === 'GET') return response(state.routes);
            if (method === 'POST') {
                const body = JSON.parse(options.body);
                state.routes.push({ id: 'e'.repeat(32), pattern: body.pattern, script: body.script ?? null });
                return response(state.routes.at(-1));
            }
        }
        if (url.pathname.startsWith(`/client/v4/zones/${zoneId}/workers/routes/`)) {
            const id = url.pathname.split('/').at(-1);
            const index = state.routes.findIndex(route => route.id === id);
            if (index < 0) return response(null, { ok: false, status: 404 });
            if (method === 'DELETE') {
                state.routes.splice(index, 1);
                return response(null);
            }
            if (method === 'PUT') {
                const body = JSON.parse(options.body);
                state.routes[index] = { id, pattern: body.pattern, script: body.script ?? null };
                return response(state.routes[index]);
            }
        }
        const match = url.pathname.match(/^\/client\/v4\/accounts\/[0-9a-f]{32}\/workers\/scripts\/([^/]+)\/(deployments|versions)(?:\/([^/]+))?$/u);
        if (match !== null) {
            const [, worker, kind, id] = match;
            const label = labelForWorker(worker);
            assert(label, `unknown worker ${worker}`);
            if (kind === 'deployments' && method === 'GET') {
                assert.equal(id, oldDeployments[label]);
                return response({
                    id,
                    versions: [{ percentage: 100, version_id: oldVersions[label] }],
                });
            }
            if (kind === 'versions' && method === 'GET') {
                assert.equal(id, oldVersions[label]);
                return response({ id });
            }
            if (kind === 'deployments' && method === 'POST') {
                const body = JSON.parse(options.body);
                assert.deepEqual(body.versions, [{ percentage: 100, version_id: oldVersions[label] }]);
                assert.equal(body.strategy, 'percentage');
                state.nextDeployment += 1;
                const idValue = `c${String(state.nextDeployment).padStart(7, '0')}-0000-4000-8000-${String(state.nextDeployment).padStart(12, '0')}`;
                state.workers[label] = { deploymentId: idValue, versionId: oldVersions[label] };
                return response({
                    id: idValue,
                    versions: [{ percentage: 100, version_id: oldVersions[label] }],
                });
            }
        }
        throw new Error(`unexpected mocked Cloudflare request ${method} ${url.href}`);
    };
    const currentWorkerImpl = async ({ worker }) => {
        const label = labelForWorker(worker);
        assert(label);
        return { ...state.workers[label] };
    };
    return { currentWorkerImpl, fetchImpl, state };
}

function options(value, api, overrides = {}) {
    return {
        accountId,
        apiToken,
        bundle: 'accepted-operator-bundle-v2',
        currentWorkerImpl: api.currentWorkerImpl,
        deploymentSha256: value.deploymentSha256,
        evidence: value.evidence,
        execute: false,
        expectedManifestSha256: manifestSha256,
        fetchImpl: api.fetchImpl,
        processNodeVersion: 'v24.19.0',
        repoRoot: value.repoRoot,
        requireUserTokenImpl: async () => {},
        rollbackSha256: value.rollbackSha256,
        smokeImpl: async () => { api.state.calls.push({ method: 'SMOKE', url: 'smoke' }); },
        transaction: value.transaction,
        validateBundleImpl: async () => ({ manifest: value.manifest, routes: deployedRoutes }),
        verifyConfigImpl: async () => {},
        zoneId,
        ...overrides,
    };
}

function mutations(api) {
    return api.state.calls.filter(call => ['POST', 'PUT', 'DELETE'].includes(call.method));
}

function sortedRouteState(routes) {
    return routes.map(({ pattern, script }) => ({ pattern, script: script ?? null }))
        .sort((left, right) => utf8Order(left.pattern, right.pattern));
}

function crashJournalEvents(value, api) {
    const events = [{
        name: '00-authority.json',
        value: {
            account_id: accountId,
            deployment_evidence_sha256: value.deploymentSha256,
            rollback_evidence_sha256: value.rollbackSha256,
            schema_version: 1,
            zone_id: zoneId,
        },
    }];
    for (let index = 0; index < labels.length; index += 1) {
        const label = labels[index];
        events.push({
            name: `${(index + 1) * 10}-${label}-intent.json`,
            value: {
                deployment_evidence_sha256: value.deploymentSha256,
                operation: `worker:${label}`,
                rollback_evidence_sha256: value.rollbackSha256,
                schema_version: 1,
            },
        }, {
            name: `${(index + 1) * 10 + 1}-${label}-done.json`,
            value: {
                active_deployment_id: api.state.workers[label].deploymentId,
                schema_version: 1,
                version_id: oldVersions[label],
                worker_name: workers[label],
            },
        });
    }
    events.push({
        name: '40-routes-intent.json',
        value: {
            deployment_evidence_sha256: value.deploymentSha256,
            operation: 'routes',
            rollback_evidence_sha256: value.rollbackSha256,
            schema_version: 1,
        },
    }, {
        name: '41-routes-done.json',
        value: { routes: previousRoutes, schema_version: 1 },
    });
    const receiptWorkers = Object.fromEntries(labels.map(label => [label, {
        active_deployment_id: api.state.workers[label].deploymentId,
        version_id: oldVersions[label],
        worker_name: workers[label],
    }]));
    events.push({
        name: '50-complete.json',
        value: {
            account_id: accountId,
            deployment_evidence_sha256: value.deploymentSha256,
            rollback_evidence_sha256: value.rollbackSha256,
            routes: previousRoutes,
            schema_version: 1,
            status: 'complete',
            workers: receiptWorkers,
            zone_id: zoneId,
        },
    });
    return events;
}

test('preflight authenticates evidence, old deployments, zone, workers, and routes without mutation or journal writes', async t => {
    const value = await fixture(t);
    const api = cloudflareMock();
    const result = await runOperatorRollback(options(value, api));
    assert.equal(result.mutated, false);
    assert.deepEqual(result.completed_workers, []);
    assert.equal(result.remaining_route_mutations, 3);
    assert.deepEqual(mutations(api), []);
    assert.equal(await lstat(value.transaction).catch(() => undefined), undefined);
    const recordedLookups = api.state.calls.filter(call => call.url.includes('/deployments/') || call.url.includes('/versions/'));
    assert.equal(recordedLookups.length, 8);
});

test('execute rolls back exact versions then routes, writes private durable evidence, and is idempotent', async t => {
    const value = await fixture(t);
    const api = cloudflareMock();
    const first = await runOperatorRollback(options(value, api, { execute: true }));
    assert.equal(first.mutated, true);
    assert.deepEqual(Object.fromEntries(labels.map(label => [label, api.state.workers[label].versionId])), {
        public: oldVersions.public,
        runtime: oldVersions.runtime,
        signer: oldVersions.signer,
    });
    assert.deepEqual(sortedRouteState(api.state.routes), sortedRouteState(previousRoutes));
    const liveMutations = mutations(api);
    assert.deepEqual(liveMutations.map(call => {
        if (call.url.endsWith('/deployments')) return `worker:${JSON.parse(call.body).versions[0].version_id}`;
        if (call.method === 'POST') return 'route:POST';
        return 'route:DELETE';
    }), [
        `worker:${oldVersions.runtime}`,
        `worker:${oldVersions.signer}`,
        `worker:${oldVersions.public}`,
        'route:POST', 'route:POST',
        'route:DELETE',
    ]);
    const entries = await readdir(value.transaction);
    assert.deepEqual(entries.sort(), [
        '00-authority.json',
        '10-runtime-intent.json', '11-runtime-done.json',
        '20-signer-intent.json', '21-signer-done.json',
        '30-public-intent.json', '31-public-done.json',
        '40-routes-intent.json', '41-routes-done.json',
        '50-complete.json',
    ].sort());
    for (const entry of entries) {
        const path = resolve(value.transaction, entry);
        assert.equal((await lstat(path)).mode & 0o777, 0o600);
        assert.ok(!(await readFile(path, 'utf8')).includes(apiToken));
    }
    assert.equal(
        first.rollbackReceiptSha256,
        digest(await readFile(resolve(value.transaction, '50-complete.json'))),
    );
    assert.equal((await lstat(value.transaction)).mode & 0o777, 0o700);
    const mutationCount = mutations(api).length;
    const second = await runOperatorRollback(options(value, api, { execute: true }));
    assert.equal(second.mutated, true);
    assert.equal(mutations(api).length, mutationCount);
    assert.deepEqual(second.receipt, first.receipt);
});

test('a crash after Worker mutation but before completion evidence resumes without duplicate activation', async t => {
    const value = await fixture(t);
    const api = cloudflareMock();
    const baseFetch = api.fetchImpl;
    let crash = true;
    api.fetchImpl = async (url, request) => {
        const result = await baseFetch(url, request);
        if (crash && request?.method === 'POST' && String(url).endsWith(`/${workers.runtime}/deployments`)) {
            crash = false;
            throw new Error('simulated process loss after runtime activation');
        }
        return result;
    };
    await assert.rejects(
        runOperatorRollback(options(value, api, { execute: true })),
        /simulated process loss/u,
    );
    assert.equal(api.state.workers.runtime.versionId, oldVersions.runtime);
    const beforePreflightEntries = await readdir(value.transaction);
    const beforePreflightMutations = mutations(api).length;
    const preflight = await runOperatorRollback(options(value, api));
    assert.deepEqual(preflight.completed_workers, ['runtime']);
    assert.deepEqual(await readdir(value.transaction), beforePreflightEntries);
    assert.equal(mutations(api).length, beforePreflightMutations);
    await runOperatorRollback(options(value, api, { execute: true }));
    const runtimePosts = mutations(api).filter(call => call.url.endsWith(`/${workers.runtime}/deployments`));
    assert.equal(runtimePosts.length, 1);
});

test('a crash after a route write resumes from the exact logical prefix', async t => {
    const value = await fixture(t);
    const api = cloudflareMock();
    const baseFetch = api.fetchImpl;
    let crash = true;
    api.fetchImpl = async (url, request) => {
        const result = await baseFetch(url, request);
        if (crash && request?.method === 'POST' && String(url).includes('/workers/routes')) {
            crash = false;
            throw new Error('simulated process loss after route creation');
        }
        return result;
    };
    await assert.rejects(
        runOperatorRollback(options(value, api, { execute: true })),
        /simulated process loss after route/u,
    );
    await runOperatorRollback(options(value, api, { execute: true }));
    const firstRouteCreates = mutations(api).filter(call => call.method === 'POST'
        && call.url.endsWith('/workers/routes')
        && JSON.parse(call.body).pattern === previousRoutes[0].pattern);
    assert.equal(firstRouteCreates.length, 1);
    assert.deepEqual(sortedRouteState(api.state.routes), sortedRouteState(previousRoutes));
});

test('torn journal writes resume at authority, intent, done, routes, and completion boundaries', async t => {
    for (const targetName of [
        '00-authority.json',
        '10-runtime-intent.json',
        '11-runtime-done.json',
        '40-routes-intent.json',
        '41-routes-done.json',
        '50-complete.json',
    ]) {
        await t.test(targetName, async t2 => {
            const value = await fixture(t2);
            const api = cloudflareMock();
            await mkdir(value.transaction, { mode: 0o700 });
            await chmod(value.transaction, 0o700);
            const order = [
                '00-authority.json',
                '10-runtime-intent.json', '11-runtime-done.json',
                '20-signer-intent.json', '21-signer-done.json',
                '30-public-intent.json', '31-public-done.json',
                '40-routes-intent.json', '41-routes-done.json',
                '50-complete.json',
            ];
            const targetIndex = order.indexOf(targetName);
            for (let index = 0; index < labels.length; index += 1) {
                const label = labels[index];
                const doneIndex = order.indexOf(`${(index + 1) * 10 + 1}-${label}-done.json`);
                if (doneIndex <= targetIndex) {
                    api.state.workers[label] = {
                        deploymentId: `d${String(index + 1).padStart(7, '0')}-0000-4000-8000-${String(index + 1).padStart(12, '0')}`,
                        versionId: oldVersions[label],
                    };
                }
            }
            if (targetIndex >= order.indexOf('41-routes-done.json')) {
                api.state.routes = previousRoutes.map((route, index) => ({
                    id: (index + 5).toString(16).repeat(32),
                    ...route,
                }));
            }
            const events = crashJournalEvents(value, api);
            for (const event of events.slice(0, targetIndex)) {
                await writeFile(resolve(value.transaction, event.name), bytes(event.value), { mode: 0o600 });
            }
            const target = events[targetIndex];
            const targetBytes = bytes(target.value);
            await writeFile(
                resolve(value.transaction, `${target.name}.partial`),
                targetBytes.subarray(0, Math.max(1, Math.floor(targetBytes.length / 2))),
                { mode: 0o600 },
            );
            await assert.rejects(runOperatorRollback(options(value, api)), /incomplete/u);
            await runOperatorRollback(options(value, api, { execute: true }));
            assert.equal(await lstat(resolve(value.transaction, `${target.name}.partial`)).catch(() => undefined), undefined);
            assert.ok(await lstat(resolve(value.transaction, '50-complete.json')));
        });
    }
});

test('out-of-order Worker drift during activation is detected before the next rollback write', async t => {
    const value = await fixture(t);
    const api = cloudflareMock();
    const baseFetch = api.fetchImpl;
    api.fetchImpl = async (url, request) => {
        const result = await baseFetch(url, request);
        if (request?.method === 'POST' && String(url).endsWith(`/${workers.runtime}/deployments`)) {
            api.state.workers.public = {
                deploymentId: 'd0000001-0000-4000-8000-000000000001',
                versionId: oldVersions.public,
            };
        }
        return result;
    };
    await assert.rejects(
        runOperatorRollback(options(value, api, { execute: true })),
        /not a prefix|neither its authenticated/u,
    );
    assert.equal(mutations(api).filter(call => call.url.endsWith('/deployments')).length, 1);
});

test('datadir drift after the public completion is rejected before route intent or writes', async t => {
    const value = await fixture(t);
    const api = cloudflareMock();
    const baseCurrent = api.currentWorkerImpl;
    let allOldRuntimeReads = 0;
    api.currentWorkerImpl = async optionsValue => {
        if (optionsValue.worker === workers.runtime
            && labels.every(label => api.state.workers[label].versionId === oldVersions[label])
            && ++allOldRuntimeReads === 2) {
            api.state.workers.datadir = {
                deploymentId: 'd4040404-0404-4404-8404-040404040404',
                versionId: oldVersions.datadir,
            };
        }
        return baseCurrent(optionsValue);
    };
    await assert.rejects(
        runOperatorRollback(options(value, api, { execute: true })),
        /datadir Worker changed/u,
    );
    assert.equal(mutations(api).filter(call => call.url.includes('/workers/routes')).length, 0);
    assert.equal(await lstat(resolve(value.transaction, '40-routes-intent.json')).catch(() => undefined), undefined);
});

test('route substitution after prefix observation is not overwritten by POST, PUT, or DELETE', async t => {
    for (const method of ['POST', 'PUT', 'DELETE']) {
        await t.test(method, async t2 => {
            const value = await fixture(t2);
            if (method === 'PUT') {
                await rewriteRollbackAuthority(value, rollback => {
                    rollback.previous_routes = deployedRoutes.map(route => ({ ...route }));
                    rollback.previous_routes.find(route => route.pattern.endsWith('/wasm/*')).script = workers.public;
                });
            } else if (method === 'DELETE') {
                await rewriteRollbackAuthority(value, rollback => {
                    rollback.previous_routes = deployedRoutes.slice(1).map(route => ({ ...route }));
                });
            }
            const api = cloudflareMock();
            const baseFetch = api.fetchImpl;
            let routeLists = 0;
            api.fetchImpl = async (url, request) => {
                if ((request?.method ?? 'GET') === 'GET' && String(url).endsWith('/workers/routes')
                    && ++routeLists === 3) {
                    api.state.routes.find(route => route.pattern.endsWith('/datadirs/*')).script = workers.public;
                }
                return baseFetch(url, request);
            };
            await assert.rejects(
                runOperatorRollback(options(value, api, { execute: true })),
                new RegExp(`routes changed before rollback ${method}`, 'u'),
            );
            assert.equal(mutations(api).filter(call => call.url.includes('/workers/routes')).length, 0);
        });
    }
});

test('route drift during smoke prevents terminal completion evidence', async t => {
    const value = await fixture(t);
    const api = cloudflareMock();
    const invocation = options(value, api, {
        execute: true,
        smokeImpl: async () => {
            api.state.routes.find(route => route.pattern.endsWith('/wasm/*')).script = workers.public;
        },
    });
    await assert.rejects(runOperatorRollback(invocation), /routes changed after rollback smoke/u);
    assert.equal(await lstat(resolve(value.transaction, '50-complete.json')).catch(() => undefined), undefined);
});

test('evidence identity, canonical bytes, modes, and live divergence fail before Cloudflare mutation', async t => {
    for (const kind of ['account', 'canonical', 'mode', 'live']) {
        await t.test(kind, async t2 => {
            const value = await fixture(t2);
            const api = cloudflareMock();
            let invocation = options(value, api);
            if (kind === 'account') invocation = { ...invocation, accountId: 'ef'.repeat(16) };
            if (kind === 'canonical') {
                const path = resolve(value.evidence, 'deployment.json');
                const parsed = JSON.parse(await readFile(path, 'utf8'));
                const altered = Buffer.from(`${JSON.stringify(parsed, null, 2)}\n`);
                await writeFile(path, altered);
                invocation = { ...invocation, deploymentSha256: digest(altered) };
            }
            if (kind === 'mode') await chmod(resolve(value.evidence, 'rollback.json'), 0o644);
            if (kind === 'live') api.state.workers.runtime.versionId = '99999999-9999-4999-8999-999999999999';
            await assert.rejects(runOperatorRollback(invocation), /differs|canonical|mode|neither/u);
            assert.deepEqual(mutations(api), []);
            assert.equal(await lstat(value.transaction).catch(() => undefined), undefined);
        });
    }
});

test('recorded deployment substitution and split traffic fail closed', async t => {
    const value = await fixture(t);
    const api = cloudflareMock();
    const baseFetch = api.fetchImpl;
    api.fetchImpl = async (url, request) => {
        if (String(url).endsWith(`/deployments/${oldDeployments.runtime}`)) {
            return response({
                id: oldDeployments.runtime,
                versions: [
                    { percentage: 50, version_id: oldVersions.runtime },
                    { percentage: 50, version_id: newVersions.runtime },
                ],
            });
        }
        return baseFetch(url, request);
    };
    await assert.rejects(runOperatorRollback(options(value, api)), /exact 100% version/u);
    assert.deepEqual(mutations(api), []);
});

test('previous routes cannot introduce a Worker script without exact deployment identity', async t => {
    const value = await fixture(t);
    const api = cloudflareMock();
    value.rollback.previous_routes[4].script = 'ambient-legacy-worker';
    const rollbackBytes = bytes(value.rollback);
    const rollbackSha256 = digest(rollbackBytes);
    await writeFile(resolve(value.evidence, 'rollback.json'), rollbackBytes);
    value.deployment.rollback_evidence_sha256 = rollbackSha256;
    const deploymentBytes = bytes(value.deployment);
    const deploymentSha256 = digest(deploymentBytes);
    await writeFile(resolve(value.evidence, 'deployment.json'), deploymentBytes);
    await assert.rejects(runOperatorRollback(options(value, api, {
        deploymentSha256,
        rollbackSha256,
    })), /route Worker identities differ/u);
    assert.deepEqual(mutations(api), []);
});

test('API failures redact the bearer token and never create transaction evidence', async t => {
    const value = await fixture(t);
    const api = cloudflareMock();
    const baseFetch = api.fetchImpl;
    api.fetchImpl = async (url, request) => {
        if (String(url).endsWith(`/versions/${oldVersions.runtime}`)) {
            return response(null, {
                errors: [{ message: `denied bearer ${apiToken}` }],
                ok: false,
                status: 403,
            });
        }
        return baseFetch(url, request);
    };
    let failure;
    try { await runOperatorRollback(options(value, api)); } catch (error) { failure = error; }
    assert.ok(failure instanceof Error);
    assert.ok(!failure.message.includes(apiToken));
    assert.match(failure.message, /\[REDACTED\]/u);
    assert.equal(await lstat(value.transaction).catch(() => undefined), undefined);
});

test('imported current-deployment helper failures also redact the bearer token', async t => {
    const value = await fixture(t);
    const api = cloudflareMock();
    const invocation = options(value, api, {
        currentWorkerImpl: async () => {
            throw new Error(`helper accidentally echoed ${apiToken}`);
        },
    });
    let failure;
    try { await runOperatorRollback(invocation); } catch (error) { failure = error; }
    assert.ok(failure instanceof Error);
    assert.ok(!failure.message.includes(apiToken));
    assert.match(failure.message, /\[REDACTED\]/u);
    assert.deepEqual(mutations(api), []);
});

test('wrong Node and a substituted journal authority fail before mutation', async t => {
    const value = await fixture(t);
    const api = cloudflareMock();
    await assert.rejects(
        runOperatorRollback(options(value, api, { processNodeVersion: 'v26.0.0' })),
        /requires exact Node\.js v24\.19\.0/u,
    );
    assert.equal(api.state.calls.length, 0);
    await mkdir(value.transaction, { mode: 0o700 });
    await writeFile(resolve(value.transaction, '00-authority.json'), bytes({
        account_id: 'ef'.repeat(16),
        deployment_evidence_sha256: value.deploymentSha256,
        rollback_evidence_sha256: value.rollbackSha256,
        schema_version: 1,
        zone_id: zoneId,
    }), { mode: 0o600 });
    await assert.rejects(runOperatorRollback(options(value, api)), /transaction authority differs/u);
    assert.deepEqual(mutations(api), []);
});

test('an absent previous Worker version is rejected as non-rollbackable evidence', async t => {
    const value = await fixture(t);
    const api = cloudflareMock();
    value.rollback.workers.public = null;
    const rollbackBytes = bytes(value.rollback);
    const rollbackSha256 = digest(rollbackBytes);
    await writeFile(resolve(value.evidence, 'rollback.json'), rollbackBytes);
    value.deployment.rollback_evidence_sha256 = rollbackSha256;
    const deploymentBytes = bytes(value.deployment);
    const deploymentSha256 = digest(deploymentBytes);
    await writeFile(resolve(value.evidence, 'deployment.json'), deploymentBytes);
    await assert.rejects(runOperatorRollback(options(value, api, {
        deploymentSha256,
        rollbackSha256,
    })), /records no previous Worker version/u);
    assert.deepEqual(mutations(api), []);
});
