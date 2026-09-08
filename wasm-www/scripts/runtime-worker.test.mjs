import assert from 'node:assert/strict';
import test from 'node:test';
import worker, { acceptsBrotli } from '../../scripts/wasm-precompressed-worker.mjs';

for (const value of ['br', 'gzip, br', 'BR;q=0.3', '*;q=1']) {
    test(`accepts ${value}`, () => assert.equal(acceptsBrotli(value), true));
}
for (const value of [null, '', 'gzip', 'br;q=0', '*;q=1, br;q=0', 'br;q=invalid', 'br;q=2']) {
    test(`rejects ${value}`, () => assert.equal(acceptsBrotli(value), false));
}
function request(options = {}) {
    return new Request('https://example.test/wasm/abcdef/robin_bg.wasm', { headers: { 'Accept-Encoding': 'br' }, ...options });
}
test('serves sidecar with canonical WASM MIME and preserved security, cache and validator headers', async () => {
    const bytes = new Uint8Array([1, 2, 3]);
    const response = await worker.fetch(request(), { ASSETS: { fetch: async req => {
        assert.equal(new URL(req.url).pathname, '/wasm/abcdef/robin_bg.wasm.br');
        assert.equal(req.headers.get('Accept-Encoding'), 'identity');
        return new Response(bytes, { headers: { 'Content-Type': 'application/octet-stream', ETag: '"br-hash"',
            'Cache-Control': 'public, max-age=31536000, immutable', 'X-Content-Type-Options': 'nosniff', Vary: 'Origin' } });
    } } });
    assert.equal(response.headers.get('Content-Type'), 'application/wasm');
    assert.equal(response.headers.get('Content-Encoding'), 'br');
    assert.equal(response.headers.get('Vary'), 'Origin, Accept-Encoding');
    assert.equal(response.headers.get('ETag'), '"br-hash"');
    assert.equal(response.headers.get('X-Content-Type-Options'), 'nosniff');
    assert.match(response.headers.get('Cache-Control'), /immutable/);
    assert.deepEqual(new Uint8Array(await response.arrayBuffer()), bytes);
});
test('HEAD and conditional revalidation retain status, validators and negotiation', async () => {
    for (const status of [200, 304]) {
        const response = await worker.fetch(request({ method: 'HEAD', headers: { 'Accept-Encoding': 'br', 'If-None-Match': '"br-hash"' } }), { ASSETS: { fetch: async req => {
            assert.equal(req.method, 'HEAD');
            assert.equal(req.headers.get('If-None-Match'), '"br-hash"');
            return new Response(null, { status, headers: { ETag: '"br-hash"' } });
        } } });
        assert.equal(response.status, status);
        assert.equal(response.body, null);
        assert.equal(response.headers.get('Content-Encoding'), 'br');
    }
});
test('missing historical sidecar falls back once; errors do not silently fall back', async () => {
    for (const status of [404, 500]) {
        const urls = [];
        const response = await worker.fetch(request(), { ASSETS: { fetch: async req => {
            urls.push(req.url);
            return urls.length === 1 ? new Response(null, { status }) : new Response('raw');
        } } });
        assert.equal(response.status, status === 404 ? 200 : 500);
        assert.equal(urls.length, status === 404 ? 2 : 1);
    }
});
test('identity and range requests retain original request semantics and Vary', async () => {
    for (const headers of [{ 'Accept-Encoding': 'br;q=0' }, { 'Accept-Encoding': 'br', Range: 'bytes=0-7' }]) {
        const input = request({ headers });
        const response = await worker.fetch(input, { ASSETS: { fetch: async req => {
            assert.equal(req, input);
            return new Response('raw', { status: headers.Range ? 206 : 200 });
        } } });
        assert.equal(response.headers.get('Vary'), 'Accept-Encoding');
        assert.equal(response.headers.get('Content-Encoding'), null);
    }
});
test('uses the original client encoding when Cloudflare normalizes the request header', async () => {
    const input = request();
    Object.defineProperty(input, 'cf', { value: { clientAcceptEncoding: 'gzip, br;q=0' } });
    await worker.fetch(input, { ASSETS: { fetch: async req => {
        assert.equal(req, input);
        return new Response('raw');
    } } });
});
