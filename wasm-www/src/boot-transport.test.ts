import assert from 'node:assert/strict';
import test from 'node:test';
import { gzipSync } from 'node:zlib';
import { fetchJson, fetchPrecompressedWasm, fetchWithProgress } from './boot-transport.ts';
import { withAbort } from './cancellation.ts';

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
    assert.deepEqual(progress, [[2, 2]]);
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
