import assert from 'node:assert/strict';
import test from 'node:test';
import { EXPECTED_PUBLIC_ROUTES } from './verify-cloudflare-deployment.mjs';
import { reconcilePublicRoutes } from './sync-cloudflare-routes.mjs';

const zoneId = 'ab'.repeat(16);
const accountId = 'cd'.repeat(16);
const apiToken = 'test-token-that-is-long-enough';
const runtimeVersionId = '12345678-90ab-cdef-8123-456789abcdef';
const runtimeVersionProof = { accountId, versionId: runtimeVersionId };
const datadirVersionId = 'abcdef12-3456-7890-8abc-def123456789';
const datadirVersionProof = { accountId, versionId: datadirVersionId };

function routeApi(initial, { runtimeExists = true, activeRuntimeVersion = runtimeVersionId } = {}) {
    let routes = structuredClone(initial);
    const writes = [];
    const operations = [];
    let nextId = 1;
    const fetchImpl = async (url, options) => {
        const method = options.method;
        if (new URL(url).pathname.includes('/workers/scripts/robinhood-runtime-assets/versions/')) {
            operations.push('prove-runtime-version');
            return runtimeExists
                ? response({ id: runtimeVersionId, number: 7 })
                : response(undefined, { ok: false, status: 404, errors: [{ message: 'not found' }] });
        }
        if (new URL(url).pathname.endsWith('/workers/scripts/robinhood-runtime-assets/deployments')) {
            operations.push('prove-runtime-deployment');
            return response({
                deployments: [{
                    id: '87654321-ba09-fedc-8321-fedcba987654',
                    versions: [{ percentage: 100, version_id: activeRuntimeVersion }],
                }],
            });
        }
        if (new URL(url).pathname.includes('/workers/scripts/robinhood-datadir-assets/versions/')) {
            operations.push('prove-datadir-version');
            return response({ id: datadirVersionId, number: 3 });
        }
        if (new URL(url).pathname.endsWith('/workers/scripts/robinhood-datadir-assets/deployments')) {
            operations.push('prove-datadir-deployment');
            return response({
                deployments: [{
                    id: '11111111-2222-3333-8444-555555555555',
                    versions: [{ percentage: 100, version_id: datadirVersionId }],
                }],
            });
        }
        operations.push(`${method}-route`);
        if (method === 'GET') return response(routes);
        const body = JSON.parse(options.body);
        writes.push(body);
        if (method === 'POST') {
            routes.push({
                id: (nextId++).toString(16).padStart(32, '0'),
                pattern: body.pattern,
                ...(body.script === undefined ? {} : { script: body.script }),
            });
            return response(routes.at(-1));
        }
        if (method === 'PUT') {
            const id = new URL(url).pathname.split('/').at(-1);
            const index = routes.findIndex(route => route.id === id);
            routes[index] = { id, pattern: body.pattern, ...(body.script === undefined ? {} : { script: body.script }) };
            return response(routes[index]);
        }
        throw new Error(`unexpected method ${method}`);
    };
    return { fetchImpl, operations, routes: () => routes, writes };
}

function response(result, { ok = true, status = 200, errors = [] } = {}) {
    return {
        ok,
        status,
        async json() { return { errors, success: ok, result }; },
    };
}

test('route reconciliation creates the query-safe no-script prefix before the broad Worker route', async () => {
    const api = routeApi([]);
    await reconcilePublicRoutes({
        zoneId, apiToken, apply: true, fetchImpl: api.fetchImpl,
        runtimeVersionProof, datadirVersionProof,
    });
    assert.deepEqual(api.operations.slice(0, 4), [
        'prove-runtime-version',
        'prove-runtime-deployment',
        'prove-datadir-version',
        'prove-datadir-deployment',
    ]);
    assert.deepEqual(api.writes, EXPECTED_PUBLIC_ROUTES.map(route => route.script === null
        ? { pattern: route.pattern }
        : { pattern: route.pattern, script: route.script }));
    assert.deepEqual(
        api.routes().map(route => route.script ?? null),
        EXPECTED_PUBLIC_ROUTES.map(route => route.script),
    );
});

test('API preparation establishes the no-script prefix before deploy and tolerates only the correct existing broad route', async () => {
    for (const initial of [[], [{
        id: 'f'.repeat(32),
        pattern: EXPECTED_PUBLIC_ROUTES.at(-1).pattern,
        script: EXPECTED_PUBLIC_ROUTES.at(-1).script,
    }]]) {
        const api = routeApi(initial);
        await reconcilePublicRoutes({
            zoneId,
            apiToken,
            apply: true,
            fetchImpl: api.fetchImpl,
            expectedRoutes: EXPECTED_PUBLIC_ROUTES.slice(0, 1),
            allowedRoutes: EXPECTED_PUBLIC_ROUTES,
        });
        assert.deepEqual(api.writes, [
            { pattern: EXPECTED_PUBLIC_ROUTES[0].pattern },
        ]);
    }

    const substituted = routeApi([{
        id: 'f'.repeat(32),
        pattern: EXPECTED_PUBLIC_ROUTES.at(-1).pattern,
        script: 'attacker-worker',
    }]);
    await assert.rejects(reconcilePublicRoutes({
        zoneId,
        apiToken,
        apply: true,
        fetchImpl: substituted.fetchImpl,
        expectedRoutes: EXPECTED_PUBLIC_ROUTES.slice(0, 1),
        allowedRoutes: EXPECTED_PUBLIC_ROUTES,
    }), /has script/u);
    assert.deepEqual(substituted.writes, []);
});

test('route reconciliation repairs substitutions and is idempotent', async () => {
    const existing = EXPECTED_PUBLIC_ROUTES.map((route, index) => ({
        id: (index + 1).toString(16).padStart(32, '0'),
        pattern: route.pattern,
        ...(route.script === null ? {} : { script: 'attacker-worker' }),
    }));
    const api = routeApi(existing);
    await reconcilePublicRoutes({
        zoneId, apiToken, apply: true, fetchImpl: api.fetchImpl,
        runtimeVersionProof, datadirVersionProof,
    });
    assert.deepEqual(api.writes, EXPECTED_PUBLIC_ROUTES
        .filter(route => route.script !== null)
        .map(route => ({ pattern: route.pattern, script: route.script })));
    api.writes.length = 0;
    await reconcilePublicRoutes({
        zoneId, apiToken, apply: true, fetchImpl: api.fetchImpl,
        runtimeVersionProof, datadirVersionProof,
    });
    assert.deepEqual(api.writes, []);
});

test('full route application fails before its first route request when the runtime version is absent', async () => {
    for (const [proof, options] of [
        [undefined, {}],
        [runtimeVersionProof, { runtimeExists: false }],
        [runtimeVersionProof, { activeRuntimeVersion: 'aaaaaaaa-bbbb-cccc-8ddd-eeeeeeeeeeee' }],
    ]) {
        const api = routeApi([], options);
        await assert.rejects(reconcilePublicRoutes({
            zoneId,
            apiToken,
            apply: true,
            fetchImpl: api.fetchImpl,
            runtimeVersionProof: proof,
            datadirVersionProof,
        }), /runtime Worker version proof|version lookup failed|not the current 100%/u);
        assert.deepEqual(api.writes, []);
        assert(!api.operations.some(operation => operation.endsWith('-route')));
    }
});

test('live check rejects missing, scripted, and unexpected public-host routes', async () => {
    const fixtures = [
        EXPECTED_PUBLIC_ROUTES.slice(1),
        EXPECTED_PUBLIC_ROUTES.map(route => route.script === null ? { ...route, script: 'wrong' } : route),
        [...EXPECTED_PUBLIC_ROUTES, { pattern: 'robinhood.phiresky.xyz/api/*', script: null }],
        [...EXPECTED_PUBLIC_ROUTES, { pattern: 'robinhood.phiresky.xyz', script: null }],
    ];
    for (const fixture of fixtures) {
        const api = routeApi(fixture.map((route, index) => ({
            id: (index + 1).toString(16).padStart(32, '0'),
            pattern: route.pattern,
            ...(route.script === null ? {} : { script: route.script }),
        })));
        await assert.rejects(
            reconcilePublicRoutes({ zoneId, apiToken, apply: false, fetchImpl: api.fetchImpl }),
            /missing Cloudflare route|has script|unexpected routes/u,
        );
    }
});
