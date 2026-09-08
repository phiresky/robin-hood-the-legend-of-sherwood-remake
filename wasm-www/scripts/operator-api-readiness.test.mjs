import assert from 'node:assert/strict';
import test from 'node:test';
import { verifyApiOriginReady } from './operator-api-readiness.mjs';

function headers(extra = {}) {
    return new Headers({
        'cache-control': 'no-store',
        'content-type': 'application/json',
        ...extra,
    });
}

test('API readiness proves two uncached origin responses and hostile preflight rejection', async () => {
    const methods = [];
    await verifyApiOriginReady({
        publicOrigin: 'https://robinhood.phiresky.xyz',
        fetchImpl: async (_url, options) => {
            methods.push(options.method);
            if (options.method === 'OPTIONS') return new Response('', { status: 405 });
            return new Response('{}', { headers: headers(), status: 200 });
        },
    });
    assert.deepEqual(methods, ['GET', 'GET', 'OPTIONS']);
});

test('API readiness rejects static interception and caching before any deployment', async () => {
    for (const headerSet of [
        { 'x-robinhood-static-origin': 'public-v1' },
        { age: '1' },
        { 'cf-cache-status': 'HIT' },
        { 'cache-control': 'public, max-age=60' },
    ]) {
        await assert.rejects(verifyApiOriginReady({
            publicOrigin: 'https://robinhood.phiresky.xyz',
            fetchImpl: async () => new Response('{}', { headers: headers(headerSet), status: 200 }),
        }), /static Worker|cache|Cache-Control/u);
    }
});
