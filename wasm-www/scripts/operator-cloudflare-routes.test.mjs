import assert from 'node:assert/strict';
import test from 'node:test';
import {
    establishBootstrapBypasses,
    reconcileOperatorRoutes,
    validateOperatorRouteAuthority,
} from './operator-cloudflare-routes.mjs';

const zoneId = 'ab'.repeat(16);
const token = 'test-token-that-is-long-enough';
const host = 'robinhood.phiresky.xyz';
const expected = [
    { pattern: `${host}/api*`, script: null },
    { pattern: `${host}/.well-known/acme-challenge/*`, script: null },
    { pattern: `${host}/wasm/*`, script: 'robinhood-runtime-assets' },
    { pattern: `${host}/datadirs/*`, script: 'robinhood-datadir-assets' },
    { pattern: `${host}/*`, script: 'robinhood-public-site' },
];

function withIds(routes) {
    return routes.map((route, index) => ({
        id: (index + 1).toString(16).padStart(32, '0'),
        pattern: route.pattern,
        ...(route.script === null ? {} : { script: route.script }),
    }));
}

function response(result) {
    return { ok: true, status: 200, async json() { return { errors: [], result, success: true }; } };
}

function routeApi(initial) {
    let routes = structuredClone(initial);
    const operations = [];
    let nextId = routes.length + 1;
    const fetchImpl = async (url, options) => {
        const method = options.method;
        if (method === 'GET') return response(routes);
        const suffix = new URL(url).pathname.split('/').at(-1);
        if (method === 'DELETE') {
            const removed = routes.find(route => route.id === suffix);
            routes = routes.filter(route => route.id !== suffix);
            operations.push({ method, pattern: removed.pattern });
            return response(null);
        }
        const body = JSON.parse(options.body);
        operations.push({ method, ...body });
        if (method === 'POST') {
            routes.push({ id: (nextId++).toString(16).padStart(32, '0'), ...body });
        } else {
            const index = routes.findIndex(route => route.id === suffix);
            routes[index] = { id: suffix, ...body };
        }
        return response(routes.at(-1));
    };
    return { fetchImpl, operations, routes: () => routes };
}

test('route authority separates immutable datadirs from routine WASM assets', () => {
    validateOperatorRouteAuthority({
        datadirWorker: 'robinhood-datadir-assets',
        expectedRoutes: expected,
        publicHost: host,
        publicWorker: 'robinhood-public-site',
        runtimeWorker: 'robinhood-runtime-assets',
    });
    assert.throws(() => validateOperatorRouteAuthority({
        datadirWorker: 'robinhood-runtime-assets',
        expectedRoutes: expected,
        publicHost: host,
        publicWorker: 'robinhood-public-site',
        runtimeWorker: 'robinhood-runtime-assets',
    }), /exact ordered/u);
});

test('reconciliation writes query-safe routes in order and retires only approved legacy routes last', async () => {
    const initial = withIds([
        { pattern: `${host}/api/*`, script: null },
        { pattern: `${host}/*`, script: 'robinhood' },
    ]);
    const api = routeApi(initial);
    const result = await reconcileOperatorRoutes({
        apiToken: token,
        apply: true,
        approvedSnapshotRoutes: initial,
        expectedRoutes: expected,
        fetchImpl: api.fetchImpl,
        publicHost: host,
        retirePatterns: [`${host}/api/*`],
        zoneId,
    });
    assert.deepEqual(result.writes.map(write => write.pattern), [
        `${host}/api*`,
        `${host}/.well-known/acme-challenge/*`,
        `${host}/wasm/*`,
        `${host}/datadirs/*`,
        `${host}/*`,
        `${host}/api/*`,
    ]);
    assert.equal(result.writes.at(-1).method, 'DELETE');
    assert.deepEqual(result.after.map(route => route.pattern).sort(), expected.map(route => route.pattern).sort());
});

test('reconciliation refuses preflight drift and unapproved retirement without writes', async () => {
    const approved = withIds([{ pattern: `${host}/*`, script: 'robinhood' }]);
    const drifted = withIds([{ pattern: `${host}/*`, script: 'substituted' }]);
    const api = routeApi(drifted);
    await assert.rejects(reconcileOperatorRoutes({
        apiToken: token,
        apply: true,
        approvedSnapshotRoutes: approved,
        expectedRoutes: expected,
        fetchImpl: api.fetchImpl,
        publicHost: host,
        retirePatterns: [],
        zoneId,
    }), /differs from its exact authority/u);
    assert.deepEqual(api.operations, []);

    await assert.rejects(reconcileOperatorRoutes({
        apiToken: token,
        apply: true,
        approvedSnapshotRoutes: drifted,
        expectedRoutes: expected,
        fetchImpl: api.fetchImpl,
        publicHost: host,
        retirePatterns: [`${host}/wasm/*`],
        zoneId,
    }), /overlaps the target authority/u);
    assert.deepEqual(api.operations, []);
});

test('bootstrap adds only query-safe API and permanent ACME no-script routes', async () => {
    const initial = withIds([
        { pattern: `${host}/api/*`, script: null },
        { pattern: `${host}/*`, script: 'robinhood' },
    ]);
    const api = routeApi(initial);
    const result = await establishBootstrapBypasses({
        apiToken: token,
        approvedSnapshotRoutes: initial,
        fetchImpl: api.fetchImpl,
        publicHost: host,
        zoneId,
    });
    assert.deepEqual(api.operations, [
        { method: 'POST', pattern: `${host}/api*` },
        { method: 'POST', pattern: `${host}/.well-known/acme-challenge/*` },
    ]);
    assert.equal(result.after.find(route => route.pattern === `${host}/*`).script, 'robinhood');
    assert(result.after.some(route => route.pattern === `${host}/api/*`));
});
