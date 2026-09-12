import assert from 'node:assert/strict';
import test from 'node:test';
import { forEachConcurrent, preloadRuntimeAssets, resolvePreloadEntries } from './asset-preload.ts';

test('preload manifest rejects malformed entries and resolves historical and explicit URL forms', () => {
    const base = 'https://runtime.example/build';
    assert.deepEqual(resolvePreloadEntries(['a', { path: 'b', url: './b.bin' }], 'manifest', base), [
        { path: 'a', assetUrl: `${base}/a` }, { path: 'b', assetUrl: `${base}/b.bin` },
    ]);
    for (const raw of [{}, [null], [4], [{ path: 4 }], [{ path: 'ok', url: {} }], ['']]) {
        assert.throws(() => resolvePreloadEntries(raw, 'manifest', base), /manifest/u);
    }
});

test('bounded workers drain in-flight work, preserve index error order and never exceed the limit', async () => {
    let active = 0, maximum = 0;
    const completed: number[] = [];
    await assert.rejects(forEachConcurrent([0, 1, 2, 3, 4], 2, async index => {
        active++; maximum = Math.max(maximum, active);
        await new Promise(resolve => setImmediate(resolve));
        active--; completed.push(index);
        if (index === 1 || index === 3) throw new Error(`failure ${index}`);
    }), /failure 1/u);
    assert.equal(maximum, 2);
    assert.equal(active, 0);
    assert.deepEqual(completed.sort(), [0, 1, 2, 3, 4]);
    await assert.rejects(forEachConcurrent([], 0, async () => {}), /positive integer/u);
});

test('worker errors use manifest order even when later failures finish first', async () => {
    let releaseFirst!: () => void;
    const firstPending = new Promise<void>(resolve => { releaseFirst = resolve; });
    const firstFailure = new Error('first manifest failure');
    const completed: number[] = [];
    await assert.rejects(forEachConcurrent([0, 1, 2], 2, async index => {
        if (index === 0) {
            await firstPending;
            completed.push(index);
            throw firstFailure;
        }
        completed.push(index);
        if (index === 2) releaseFirst();
        throw new Error(`failure ${index}`);
    }), error => error === firstFailure);
    assert.deepEqual(completed, [1, 2, 0]);
    await assert.rejects(forEachConcurrent([0], 1, async () => { throw 'non-Error failure'; }),
        { name: 'Error', message: 'non-Error failure' });
});

test('cancellation takes precedence over recorded worker failures', async () => {
    const controller = new AbortController();
    const reason = new Error('cancelled');
    const visited: number[] = [];
    await assert.rejects(forEachConcurrent([0, 1, 2], 1, async index => {
        visited.push(index);
        if (index === 1) controller.abort(reason);
        throw new Error(`failure ${index}`);
    }, controller.signal), error => error === reason);
    assert.deepEqual(visited, [0, 1]);
    await forEachConcurrent([], 1, async () => { assert.fail('empty queue must not run'); });
});

test('preload fetches and installs each asset before declaring completion', async () => {
    const installed: string[] = [], progress: number[] = [];
    await preloadRuntimeAssets({ default: async () => {}, wasm_boot: () => {},
        wasm_preload_asset: (path, bytes) => { assert.deepEqual([...bytes], [1, 2]); installed.push(path); },
    }, 'https://runtime.example/build', false, {
        fetch: async input => String(input).endsWith('preload-assets.json')
            ? Response.json(['a', 'b']) : new Response(new Uint8Array([1, 2])),
        log: () => {}, progress: fraction => { progress.push(fraction); },
    });
    assert.deepEqual(installed.sort(), ['a', 'b']);
    assert.deepEqual(progress, [0.5, 1]);
});

test('injected browser fetch is called without the dependency object as its receiver', async () => {
    let requests = 0;
    await preloadRuntimeAssets({ default: async () => {}, wasm_boot: () => {}, wasm_preload_asset: () => {} },
        'https://runtime.example/build', false, {
            fetch: async function (this: unknown, input) {
                assert.equal(this, undefined, 'Window.fetch cannot be invoked with an arbitrary dependency object as this');
                requests++;
                return String(input).endsWith('preload-assets.json') ? Response.json(['a']) : new Response(new Uint8Array([1]));
            },
            log: () => {}, progress: () => {},
        });
    assert.equal(requests, 2, 'both manifest and asset paths use the detached fetch');
});

test('cancelled preload forwards the signal, stops dequeuing and never installs late bytes', async () => {
    const controller = new AbortController();
    let installed = false;
    let requested = 0;
    await assert.rejects(preloadRuntimeAssets({ default: async () => {}, wasm_boot: () => {},
        wasm_preload_asset: () => { installed = true; },
    }, 'https://runtime.example/build', false, {
        signal: controller.signal,
        fetch: async (input, init) => {
            assert.equal(init?.signal, controller.signal);
            if (String(input).endsWith('preload-assets.json')) return Response.json(Array.from({ length: 30 }, (_, i) => String(i)));
            requested++;
            controller.abort();
            return new Response(new Uint8Array([1]));
        },
        log: () => {}, progress: () => {},
    }), { name: 'AbortError' });
    assert.equal(installed, false);
    assert.ok(requested > 0 && requested <= 12);
});
