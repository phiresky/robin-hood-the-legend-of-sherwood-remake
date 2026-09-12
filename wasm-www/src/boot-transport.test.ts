import assert from 'node:assert/strict';
import test from 'node:test';
import { brotliCompressSync, gzipSync } from 'node:zlib';
import { fetchJson, fetchPrecompressedWasm, fetchRuntimeWasm, fetchWithProgress } from './boot-transport.ts';
import { withAbort } from './cancellation.ts';

for (const cancelBehavior of ['reject', 'stall'] as const) {
    test(`unused responses never delay fallback or mask errors when cancellation will ${cancelBehavior}`, { timeout: 2000 }, async () => {
        const bytes = new Uint8Array([0, 97, 115, 109]);
        for (const outcome of ['gzip', 'error', 'abort', 'raw'] as const) {
            const controller = new AbortController();
            const cancelled: string[] = [];
            const urls: string[] = [];
            const originalError = new Error('gzip fetch failed');
            const unused = (url: string, status: number): Response => new Response(new ReadableStream({
                cancel() {
                    cancelled.push(url);
                    return cancelBehavior === 'reject'
                        ? Promise.reject(new Error('cleanup failed'))
                        : new Promise<void>(() => {});
                },
            }), { status });
            const raw = new Response(bytes);
            const pending = fetchRuntimeWasm('asset.wasm', true, 'default', () => {}, controller.signal, async url => {
                const name = String(url);
                urls.push(name);
                if (name.endsWith('.br')) return unused(name, 404);
                if (name.endsWith('.gz')) {
                    if (outcome === 'error') throw originalError;
                    if (outcome === 'abort') {
                        controller.abort(originalError);
                        return new Promise<Response>(() => {});
                    }
                    return outcome === 'raw' ? unused(name, 404) : new Response(gzipSync(bytes));
                }
                return outcome === 'raw' ? raw : unused(name, 404);
            });
            if (outcome === 'error' || outcome === 'abort') {
                await assert.rejects(pending, error => error === originalError);
            } else {
                const result = await pending;
                assert.deepEqual(new Uint8Array(await result.arrayBuffer()), bytes);
                if (outcome === 'raw') assert.equal(raw.bodyUsed, true);
            }
            assert.deepEqual(urls, ['asset.wasm.br', 'asset.wasm', 'asset.wasm.gz']);
            assert.deepEqual(cancelled, outcome === 'raw'
                ? ['asset.wasm.br', 'asset.wasm.gz']
                : ['asset.wasm.br', 'asset.wasm']);
        }
    });
}

test('abort releases an uncooperative provider and prevents already-aborted operations', async () => {
    const controller = new AbortController();
    const pending = withAbort(controller.signal, () => new Promise<never>(() => {}));
    await Promise.resolve();
    controller.abort();
    await assert.rejects(pending, { name: 'AbortError' });
    let called = false;
    await assert.rejects(withAbort(controller.signal, async () => { called = true; }), { name: 'AbortError' });
    assert.equal(called, false);
});

test('download preserves bytes, progress, cache policy and cancellation signal', async () => {
    const signal = new AbortController().signal;
    const progress: number[][] = [];
    const response = await fetchWithProgress('asset', 'force-cache', 'application/wasm',
        (loaded, total) => progress.push([loaded, total]), signal, async (_url, init) => {
            assert.equal(init?.signal, signal);
            assert.equal(init?.cache, 'force-cache');
            return new Response(new Uint8Array([1, 2]), { headers: { 'Content-Length': '2' } });
        });
    assert.equal(response.headers.get('Content-Type'), 'application/wasm');
    assert.deepEqual([...new Uint8Array(await response.arrayBuffer())], [1, 2]);
    assert.deepEqual(progress, [[2, 2], [2, 2]]);
});

test('precompressed wasm handles raw gzip, browser-decoded gzip and missing sidecars', async () => {
    const signal = new AbortController().signal;
    for (const decoded of [false, true]) {
        const bytes = new Uint8Array([0, 97, 115, 109]);
        const response = await fetchPrecompressedWasm('asset.gz', 'force-cache', () => {}, signal,
            async () => new Response(decoded ? bytes : gzipSync(bytes), {
                headers: decoded ? { 'Content-Encoding': 'gzip' } : {},
            }));
        assert.ok(response);
        assert.deepEqual(new Uint8Array(await response.arrayBuffer()), bytes);
    }
    assert.equal(await fetchPrecompressedWasm('missing.gz', 'default', () => {}, signal,
        async () => new Response(null, { status: 404 })), undefined);
    await assert.rejects(fetchJson('bad', signal, async () => new Response(null, { status: 500 })), /HTTP 500/u);
});

test('abort cancels a stalled response body and JSON provider', async () => {
    const controller = new AbortController();
    let cancelled = false;
    const response = await fetchWithProgress('asset', 'default', 'application/wasm', () => {}, controller.signal,
        async () => new Response(new ReadableStream({ cancel() { cancelled = true; } })));
    const reading = response.arrayBuffer();
    controller.abort();
    await assert.rejects(reading, { name: 'AbortError' });
    assert.equal(cancelled, true);
    const jsonController = new AbortController();
    const json = fetchJson('manifest', jsonController.signal, async () => new Promise<Response>(() => {}));
    jsonController.abort();
    await assert.rejects(json, { name: 'AbortError' });
});

test('Chrome uses HTTP Brotli without a Brotli stream decoder or sidecar request', async t => {
    const Original = globalThis.DecompressionStream;
    t.mock.method(globalThis, 'DecompressionStream', function(format: string) {
        if (format === 'brotli') throw new TypeError('unsupported');
        return new Original(format as CompressionFormat);
    });
    const bytes = new Uint8Array([0, 97, 115, 109]);
    const progress: number[][] = [];
    const urls: string[] = [];
    const response = await fetchRuntimeWasm('asset.wasm', true, 'force-cache', (loaded, total) => progress.push([loaded, total]), new AbortController().signal,
        async url => { urls.push(String(url)); return new Response(bytes, { headers: { 'Content-Encoding': 'br', 'Content-Length': '2' } }); });
    assert.equal(response.headers.get('Content-Type'), 'application/wasm');
    assert.deepEqual(new Uint8Array(await response.arrayBuffer()), bytes);
    assert.deepEqual(progress, [[4, 0], [4, 4]]);
    assert.deepEqual(urls, ['asset.wasm']);
});

test('Brotli raw streaming, missing sidecar and HTTP errors retain distinct outcomes', async () => {
    const signal = new AbortController().signal;
    const bytes = new Uint8Array([0, 97, 115, 109]);
    const response = await fetchPrecompressedWasm('asset.br', 'force-cache', () => {}, signal,
        async () => new Response(brotliCompressSync(bytes)), 'brotli');
    assert.ok(response);
    assert.deepEqual(new Uint8Array(await response.arrayBuffer()), bytes);
    assert.equal(await fetchPrecompressedWasm('missing.br', 'default', () => {}, signal,
        async () => new Response(null, { status: 404 }), 'brotli'), undefined);
    await assert.rejects(fetchPrecompressedWasm('bad.br', 'default', () => {}, signal,
        async () => new Response(null, { status: 500 }), 'brotli'), /HTTP 500/u);
});

test('identity hosts use gzip when present and retain a raw-only response without refetching', async t => {
    const Original = globalThis.DecompressionStream;
    t.mock.method(globalThis, 'DecompressionStream', function(format: string) {
        if (format === 'brotli') throw new TypeError('unsupported');
        return new Original(format as CompressionFormat);
    });
    for (const hasGzip of [false, true]) {
        const urls: string[] = [];
        let cancelled = false;
        const bytes = new Uint8Array([0, 97, 115, 109]);
        const response = await fetchRuntimeWasm('asset.wasm', true, 'force-cache', () => {}, new AbortController().signal, async url => {
            urls.push(String(url));
            if (url === 'asset.wasm.gz') return hasGzip ? new Response(gzipSync(bytes)) : new Response(null, { status: 404 });
            assert.equal(url, 'asset.wasm');
            return new Response(new ReadableStream({
                start(controller) { controller.enqueue(bytes); },
                pull(controller) { controller.close(); },
                cancel() { cancelled = true; },
            }));
        });
        assert.equal(cancelled, hasGzip);
        assert.deepEqual(new Uint8Array(await response.arrayBuffer()), bytes);
        assert.deepEqual(urls, ['asset.wasm', 'asset.wasm.gz']);
    }
});

test('Brotli-capable clients prefer the offline sidecar without fetching raw WASM', async () => {
    const bytes = new Uint8Array([0, 97, 115, 109]);
    const response = await fetchRuntimeWasm('asset.wasm', true, 'force-cache', () => {}, new AbortController().signal, async url => {
        assert.equal(url, 'asset.wasm.br');
        return new Response(brotliCompressSync(bytes));
    });
    assert.deepEqual(new Uint8Array(await response.arrayBuffer()), bytes);
});

test('a gzip-only host remains usable and cancellation stops a stalled fallback', async t => {
    const Original = globalThis.DecompressionStream;
    t.mock.method(globalThis, 'DecompressionStream', function(format: string) {
        if (format === 'brotli') throw new TypeError('unsupported');
        return new Original(format as CompressionFormat);
    });
    const bytes = new Uint8Array([0, 97, 115, 109]);
    const response = await fetchRuntimeWasm('asset.wasm', true, 'default', () => {}, new AbortController().signal, async url =>
        url === 'asset.wasm.gz' ? new Response(gzipSync(bytes)) : new Response(null, { status: 404 }));
    assert.deepEqual(new Uint8Array(await response.arrayBuffer()), bytes);
    const controller = new AbortController();
    let fallbackStarted!: () => void;
    const entered = new Promise<void>(resolve => { fallbackStarted = resolve; });
    let cancelled = false;
    const pending = fetchRuntimeWasm('asset.wasm', true, 'default', () => {}, controller.signal, async url => {
        if (url === 'asset.wasm.gz') { fallbackStarted(); return new Promise<Response>(() => {}); }
        return new Response(new ReadableStream({ cancel() { cancelled = true; } }));
    });
    await entered;
    controller.abort();
    await assert.rejects(pending, { name: 'AbortError' });
    assert.ok(cancelled);
});
