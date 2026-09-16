import assert from 'node:assert/strict';
import test from 'node:test';

import { encodeReplayLink, decodeReplayLink, applyPreparedReplay, prepareReplay, prepareReplayWithRuntime, replayFromQuery, replayRuntimeOverride, type RobinRpc } from './replay.ts';

test('cold public playback validates the exact compact bytes before loading', async () => {
    const compact = new Uint8Array([82, 72, 82, 69, 67, 1, 0, 255, 128]);
    const query = replayFromQuery(new URLSearchParams({ replay: encodeReplayLink(compact) }));
    const buildBase = 'https://game.example/builds/selected';
    const calls: string[] = [];
    const runtime = { ready: true };
    const rpc: RobinRpc = async <T>(method: string, params?: unknown): Promise<T> => {
        calls.push(`rpc:${method}:${JSON.stringify(params)}`);
        return undefined as T;
    };

    const prepared = await prepareReplayWithRuntime(query, buildBase,
        async () => { calls.push('runtime-loaded'); return runtime; },
        async content => {
            assert.deepEqual(content, compact);
            calls.push('worker-accepted');
        }, new AbortController().signal);
    assert.equal(prepared.runtime, runtime);
    assert.deepEqual(prepared.replay, { content: compact, paused: true, buildBase });
    calls.push('runtime-and-admission-ready');
    const loaded = await applyPreparedReplay(rpc, content => {
        assert.deepEqual(content, compact);
        calls.push('proof-installed');
    }, prepared.replay, buildBase);

    assert.equal(loaded, true);
    assert.deepEqual(calls, [
        'runtime-loaded',
        'worker-accepted',
        'runtime-and-admission-ready',
        'proof-installed',
        `rpc:load-replay:${JSON.stringify({ data: compact, paused: true })}`,
    ]);
});

test('cold public playback never installs a proof or calls the game after rejection', async () => {
    const compact = new Uint8Array([0, 255]);
    const query = replayFromQuery(new URLSearchParams({ replay: encodeReplayLink(compact), paused: '0' }));
    assert.deepEqual(query, { content: compact, paused: false });
    const buildBase = 'https://game.example/builds/selected';
    let marked = false;
    let rpcCalled = false;
    const rpc: RobinRpc = async <T>(): Promise<T> => {
        rpcCalled = true;
        return undefined as T;
    };

    await assert.rejects(async () => {
        const prepared = await prepareReplayWithRuntime(query, buildBase,
            async () => 'runtime',
            async () => { throw new Error('isolated rejection'); },
            new AbortController().signal);
        await applyPreparedReplay(rpc, () => { marked = true; }, prepared.replay, buildBase);
    }, /isolated rejection/);
    assert.equal(marked, false);
    assert.equal(rpcCalled, false);
});

test('replay query defaults and absence survive the prepared loading path', async () => {
    for (const paused of ['0', 'false', 'NO', 'off']) {
        assert.deepEqual(replayFromQuery(new URLSearchParams({ replay: encodeReplayLink(new Uint8Array([0, 255])), paused })),
            { content: new Uint8Array([0, 255]), paused: false });
    }
    for (const paused of ['', '1', 'true']) {
        assert.deepEqual(replayFromQuery(new URLSearchParams({ replay: encodeReplayLink(new Uint8Array([0, 255])), paused })),
            { content: new Uint8Array([0, 255]), paused: true });
    }
    for (const params of [new URLSearchParams(), new URLSearchParams({ replay: '' })]) {
        const query = replayFromQuery(params);
        assert.equal(query, null);
        const buildBase = 'https://game.example/builds/selected';
        const prepared = await prepareReplayWithRuntime(query, buildBase, async () => 'runtime',
            async () => { assert.fail('absent replay must not request validation'); },
            new AbortController().signal);
        assert.equal(prepared.runtime, 'runtime');
        assert.equal(prepared.replay, null);
        assert.equal(await applyPreparedReplay(
            async () => { assert.fail('absent replay must not call RPC'); },
            () => { assert.fail('absent replay must not install proof'); },
            prepared.replay, buildBase), false);
    }
});

function deferred<T>() {
    let resolve!: (value: T) => void;
    const promise = new Promise<T>(done => { resolve = done; });
    return { promise, resolve };
}

test('admission overlaps runtime loading and boot readiness waits for both', async () => {
    const runtime = deferred<string>(), admission = deferred<void>(), entered = deferred<void>();
    let runtimeStarted = false;
    const result = prepareReplayWithRuntime({ content: new Uint8Array([0, 255, 128]), paused: false }, 'selected-build',
        async () => { runtimeStarted = true; return runtime.promise; },
        async content => { assert.deepEqual(content, new Uint8Array([0, 255, 128])); entered.resolve(); return admission.promise; },
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
    assert.deepEqual(prepared.replay, { content: new Uint8Array([0, 255, 128]), paused: false, buildBase: 'selected-build' });
});

test('prepared admission captures exact query and rejects cross-build installation', async () => {
    const query = { content: new Uint8Array([1, 255]), paused: true };
    const validated = deferred<void>();
    const pending = prepareReplay(query, 'build-a', async content => { assert.deepEqual(content, new Uint8Array([1, 255])); return validated.promise; });
    query.content = new Uint8Array([2]); query.paused = false;
    validated.resolve();
    const replay = await pending;
    let marked: Uint8Array | undefined;
    const rpc: RobinRpc = async <T>(_method: string, params?: unknown): Promise<T> => {
        assert.deepEqual(params, { data: new Uint8Array([1, 255]), paused: true });
        return undefined as T;
    };
    await assert.rejects(applyPreparedReplay(rpc, content => { marked = content; }, replay, 'build-b'), /different browser artifact/);
    assert.equal(marked, undefined);
    await applyPreparedReplay(rpc, content => { marked = content; }, replay, 'build-a');
    assert.deepEqual(marked, new Uint8Array([1, 255]));
});

test('runtime/admission failure cancels sibling and external abort cannot publish a prepared replay', async () => {
    for (const mode of ['runtime', 'admission', 'abort']) {
        const controller = new AbortController();
        let siblingSignal!: AbortSignal;
        const entered = deferred<void>();
        const pending = prepareReplayWithRuntime({ content: new Uint8Array([0, 255]), paused: true }, 'build',
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

test('compact recordings from unpublished commits use the current runtime', () => {
    assert.equal(replayRuntimeOverride(encodeReplayLink(new Uint8Array([0, 255]))), undefined);
    assert.equal(replayRuntimeOverride(null), undefined);
    assert.equal(replayRuntimeOverride(''), undefined);
    assert.equal(replayRuntimeOverride('189abc221ca6'), '189abc221ca6');
    assert.throws(() => replayRuntimeOverride('not-a-replay'), /replay= must be/);
});

test('share links preserve all binary bytes and reject old text artifacts', () => {
    const bytes = Uint8Array.from({ length: 256 }, (_, i) => i);
    assert.deepEqual(decodeReplayLink(encodeReplayLink(bytes)), bytes);
    assert.throws(() => decodeReplayLink('rhrec-0123456789ab-AAAA'), /binary replay link/);
    assert.throws(() => decodeReplayLink('rhrec1-AB'), /noncanonical/);
});

test('prepared replay bytes cannot change after worker admission', async () => {
    const source = new Uint8Array([0, 255, 128]);
    const prepared = await prepareReplay({ content: source, paused: true }, 'build', async () => {});
    assert.ok(prepared);
    source.fill(1);
    prepared.content.fill(2);
    assert.deepEqual(prepared.content, new Uint8Array([0, 255, 128]));
    assert.equal(replayFromQuery(new URLSearchParams({ replay: '0123456789ab' })), null);
});
