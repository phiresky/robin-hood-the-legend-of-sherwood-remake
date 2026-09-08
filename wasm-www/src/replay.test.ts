import assert from 'node:assert/strict';
import test from 'node:test';

import { applyReplayFromQuery, applyPreparedReplay, prepareReplay, prepareReplayWithRuntime, type RobinRpc } from './replay.ts';

function installLocation(url: string): void {
    Object.defineProperty(globalThis, 'window', {
        configurable: true,
        value: { location: new URL(url) },
    });
}

test('cold public playback validates the exact compact bytes before loading', async () => {
    const compact = 'rhrec-0123456789ab-canonical_payload';
    installLocation(`https://game.example/play?replay=${encodeURIComponent(compact)}`);
    const calls: string[] = [];
    const rpc: RobinRpc = async <T>(method: string, params?: unknown): Promise<T> => {
        calls.push(`rpc:${method}:${JSON.stringify(params)}`);
        return undefined as T;
    };

    const loaded = await applyReplayFromQuery(
        rpc,
        {
            validate: async (content) => {
                assert.equal(content, compact);
                calls.push('worker-accepted');
            },
            markValidated: (content) => {
                assert.equal(content, compact);
                calls.push('proof-installed');
            },
        },
    );

    assert.equal(loaded, true);
    assert.deepEqual(calls, [
        'worker-accepted',
        'proof-installed',
        `rpc:load-replay:${JSON.stringify({ data: compact, paused: true })}`,
    ]);
});

test('cold public playback never installs a proof or calls the game after rejection', async () => {
    const compact = 'rhrec-0123456789ab-malformed';
    installLocation(`https://game.example/play?replay=${encodeURIComponent(compact)}&paused=0`);
    let marked = false;
    let rpcCalled = false;
    const rpc: RobinRpc = async <T>(): Promise<T> => {
        rpcCalled = true;
        return undefined as T;
    };

    await assert.rejects(
        applyReplayFromQuery(
            rpc,
            {
                validate: async () => {
                    throw new Error('isolated rejection');
                },
                markValidated: () => {
                    marked = true;
                },
            },
        ),
        /isolated rejection/,
    );
    assert.equal(marked, false);
    assert.equal(rpcCalled, false);
});

function deferred<T>() {
    let resolve!: (value: T) => void;
    const promise = new Promise<T>(done => { resolve = done; });
    return { promise, resolve };
}

test('admission overlaps runtime loading and boot readiness waits for both', async () => {
    const runtime = deferred<string>(), admission = deferred<void>(), entered = deferred<void>();
    let runtimeStarted = false;
    const result = prepareReplayWithRuntime({ content: 'exact-bytes', paused: false }, 'selected-build',
        async () => { runtimeStarted = true; return runtime.promise; },
        async content => { assert.equal(content, 'exact-bytes'); entered.resolve(); return admission.promise; },
        new AbortController().signal);
    await entered.promise;
    assert.ok(runtimeStarted);
    let finished = false;
    void result.then(() => { finished = true; });
    runtime.resolve('runtime');
    await new Promise(resolve => setImmediate(resolve));
    assert.equal(finished, false);
    admission.resolve();
    const prepared = await result;
    assert.equal(prepared.runtime, 'runtime');
    assert.deepEqual(prepared.replay, { content: 'exact-bytes', paused: false, buildBase: 'selected-build' });
});

test('prepared admission captures exact query and rejects cross-build installation', async () => {
    const query = { content: 'original', paused: true };
    const validated = deferred<void>();
    const pending = prepareReplay(query, 'build-a', async content => { assert.equal(content, 'original'); return validated.promise; });
    query.content = 'substituted'; query.paused = false;
    validated.resolve();
    const replay = await pending;
    let marked = '';
    const rpc: RobinRpc = async <T>(_method: string, params?: unknown): Promise<T> => {
        assert.deepEqual(params, { data: 'original', paused: true });
        return undefined as T;
    };
    await assert.rejects(applyPreparedReplay(rpc, content => { marked = content; }, replay, 'build-b'), /different browser artifact/);
    assert.equal(marked, '');
    await applyPreparedReplay(rpc, content => { marked = content; }, replay, 'build-a');
    assert.equal(marked, 'original');
});

test('runtime/admission failure cancels sibling and external abort cannot publish a prepared replay', async () => {
    for (const mode of ['runtime', 'admission', 'abort']) {
        const controller = new AbortController();
        let siblingSignal!: AbortSignal;
        const entered = deferred<void>();
        const pending = prepareReplayWithRuntime({ content: 'bytes', paused: true }, 'build',
            async signal => {
                if (mode === 'runtime') throw new Error('runtime failed');
                siblingSignal = signal; entered.resolve(); return new Promise<never>(() => {});
            },
            async (_content, signal) => {
                if (mode === 'admission') throw new Error('admission failed');
                siblingSignal = signal; entered.resolve(); return new Promise<never>(() => {});
            }, controller.signal);
        await entered.promise;
        if (mode === 'abort') controller.abort();
        await assert.rejects(pending, mode === 'abort' ? { name: 'AbortError' } : new RegExp(mode + ' failed'));
        assert.ok(siblingSignal.aborted);
    }
});
