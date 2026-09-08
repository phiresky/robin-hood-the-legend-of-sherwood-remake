import assert from 'node:assert/strict';
import test from 'node:test';
import { bootGame, loadRuntimeInParallel, assertMultiplayerWasmCompatibility, type BootDependencies, type RobinWasmModule } from './boot-lifecycle.ts';
import type { VerifiedBrowserJoinTicket } from './join_ticket.ts';

function fixture(calls: string[]): BootDependencies {
    const wasm: RobinWasmModule = {
        default: async () => {},
        wasm_boot: (bytes, base) => { assert.deepEqual([...bytes], [1, 2]); assert.equal(base, 'data'); calls.push('boot'); },
    };
    return {
        buildsBase: 'https://runtime.example/builds',
        prepareJoin: async () => { calls.push('join'); return undefined; },
        resolveBuild: async () => { calls.push('build'); return { short: 'abc', source: 'latest' }; },
        loadManifest: async () => { throw new Error('unexpected multiplayer manifest'); },
        loadRuntime: async (base, compressed, latest) => {
            assert.equal(base, 'https://runtime.example/builds/abc'); assert.ok(compressed && latest);
            calls.push('runtime'); return wasm;
        },
        prepareContent: async () => { throw new Error('unexpected multiplayer content'); },
        loadDefaultContent: async () => { calls.push('content'); return { datadir: new Uint8Array([1, 2]), dataBaseUrl: 'data' }; },
        preloadLocalAssets: () => { throw new Error('unexpected local assets'); },
        preloadShippingFiles: () => { throw new Error('unexpected shipping files'); },
        preloadAssets: async () => { calls.push('preload'); },
        installRpc: () => { calls.push('rpc'); return async <T>(method: string) => {
            assert.equal(method, 'info'); calls.push('ready'); return undefined as T;
        }; },
        runtimeStarted: () => { calls.push('canvas'); },
        installReplay: async () => { calls.push('replay'); },
        progress: () => {}, log: () => {},
    };
}

test('boot preserves preload, runtime/canvas handoff, RPC readiness and replay ordering', async () => {
    const calls: string[] = [];
    await bootGame(fixture(calls), new AbortController().signal);
    assert.deepEqual(calls, ['join', 'build', 'runtime', 'content', 'preload', 'rpc', 'boot', 'canvas', 'ready', 'replay']);
});

test('missing runtime, content and preload failures stop boot and permit a fresh attempt', async () => {
    for (const stage of ['loadRuntime', 'loadDefaultContent', 'preloadAssets'] as const) {
        const calls: string[] = [];
        const deps = fixture(calls);
        await assert.rejects(bootGame({ ...deps, [stage]: async () => { throw new Error(stage); } }, new AbortController().signal), new RegExp(stage));
        assert.equal(calls.includes('boot'), false);
        await bootGame(deps, new AbortController().signal);
        assert.equal(calls.filter(call => call === 'boot').length, 1);
    }
});

test('cancellation stops later runtime side effects even when an adapter completes after abort', async () => {
    const calls: string[] = [];
    const controller = new AbortController();
    const deps = fixture(calls);
    await assert.rejects(bootGame({ ...deps, loadDefaultContent: async latest => {
        controller.abort(); return deps.loadDefaultContent(latest, controller.signal);
    } }, controller.signal), { name: 'AbortError' });
    assert.equal(calls.includes('preload'), false);
    assert.equal(calls.includes('boot'), false);
});

test('multiplayer compatibility rejects missing, malformed, reordered and substituted runtime identities', () => {
    const ticket = { payload: { engine_version: 'a'.repeat(40), net_protocol: 38, schema: 3 } } as VerifiedBrowserJoinTicket;
    const wasm = { default: async () => {}, wasm_boot: () => {} };
    assert.throws(() => assertMultiplayerWasmCompatibility(wasm, ticket), /does not export/u);
    const expected = { engineCommit: 'a'.repeat(40), artifactShort: 'a'.repeat(12), netProtocol: 38, ticketSchema: 3 };
    assert.doesNotThrow(() => assertMultiplayerWasmCompatibility({ ...wasm, wasm_multiplayer_compatibility: () => expected }, ticket));
    for (const raw of [null, [], { ...expected, extra: true }, { ...expected, engineCommit: 'b'.repeat(40) },
        { artifactShort: expected.artifactShort, engineCommit: expected.engineCommit, netProtocol: 38, ticketSchema: 3 }]) {
        assert.throws(() => assertMultiplayerWasmCompatibility({ ...wasm, wasm_multiplayer_compatibility: () => raw }, ticket));
    }
});

test('boot passes its lifetime to adapters and abort does not wait for a stalled runtime', async () => {
    const calls: string[] = [];
    const controller = new AbortController();
    let started!: () => void;
    const entered = new Promise<void>(resolve => { started = resolve; });
    const boot = bootGame({ ...fixture(calls), loadRuntime: async (_base, _compressed, _latest, signal) => {
        assert.equal(signal.aborted, false);
        started();
        return new Promise<RobinWasmModule>(() => {});
    } }, controller.signal);
    await entered;
    controller.abort();
    await assert.rejects(boot, { name: 'AbortError' });
    assert.equal(calls.includes('boot'), false);
});

function deferred<T>() {
    let resolve!: (value: T) => void;
    const promise = new Promise<T>(done => { resolve = done; });
    return { promise, resolve };
}

test('default content overlaps runtime and core preload, but boot waits for both', async () => {
    const calls: string[] = [];
    const deps = fixture(calls);
    const runtime = deferred<RobinWasmModule>();
    const content = deferred<Awaited<ReturnType<BootDependencies['loadDefaultContent']>>>();
    const contentStarted = deferred<void>();
    const preloadStarted = deferred<void>();
    const boot = bootGame({ ...deps,
        loadRuntime: () => runtime.promise,
        loadDefaultContent: () => { contentStarted.resolve(); return content.promise; },
        preloadAssets: async () => { preloadStarted.resolve(); },
    }, new AbortController().signal);
    await contentStarted.promise;
    assert.equal(calls.includes('boot'), false);
    runtime.resolve(await deps.loadRuntime('https://runtime.example/builds/abc', true, true, new AbortController().signal));
    await preloadStarted.promise;
    assert.equal(calls.includes('boot'), false);
    content.resolve(await deps.loadDefaultContent(true, new AbortController().signal));
    await boot;
    assert.ok(calls.includes('boot'));
});

test('content failure cancels stalled runtime and prevents late preloads', async () => {
    const calls: string[] = [];
    const deps = fixture(calls);
    const runtime = deferred<RobinWasmModule>();
    let runtimeSignal!: AbortSignal;
    await assert.rejects(bootGame({ ...deps,
        loadRuntime: async (_base, _compressed, _latest, signal) => { runtimeSignal = signal; return runtime.promise; },
        loadDefaultContent: async () => { throw new Error('content failed'); },
    }, new AbortController().signal), /content failed/);
    assert.ok(runtimeSignal.aborted);
    runtime.resolve(await deps.loadRuntime('https://runtime.example/builds/abc', true, true, new AbortController().signal));
    await new Promise(resolve => setImmediate(resolve));
    assert.equal(calls.includes('preload'), false);
    assert.equal(calls.includes('boot'), false);
});

test('WASM fetch starts before glue completes and initialization waits for both', async () => {
    const imported = deferred<RobinWasmModule>();
    const fetched = deferred<Response>();
    const started = deferred<void>();
    const response = new Response(new Uint8Array([1]));
    let initialized = false;
    const wasm: RobinWasmModule = {
        default: async options => { assert.equal(options?.module_or_path, response); initialized = true; },
        wasm_boot: () => {},
    };
    const loading = loadRuntimeInParallel(() => imported.promise, () => { started.resolve(); return fetched.promise; }, new AbortController().signal);
    await started.promise;
    fetched.resolve(response);
    assert.equal(initialized, false);
    imported.resolve(wasm);
    assert.equal(await loading, wasm);
    assert.equal(initialized, true);
});

test('runtime import/fetch failure cancels sibling and never initializes a late module', async () => {
    for (const failure of ['import', 'fetch']) {
        let initialized = false;
        let siblingSignal!: AbortSignal;
        const lateModule = deferred<RobinWasmModule>();
        const wasm: RobinWasmModule = { default: async () => { initialized = true; }, wasm_boot: () => {} };
        await assert.rejects(loadRuntimeInParallel(
            async signal => {
                if (failure === 'import') throw new Error('import failed');
                siblingSignal = signal;
                return lateModule.promise;
            },
            async signal => {
                if (failure === 'fetch') throw new Error('fetch failed');
                siblingSignal = signal;
                return new Promise<Response>(() => {});
            }, new AbortController().signal), new RegExp(failure + ' failed'));
        assert.ok(siblingSignal.aborted);
        lateModule.resolve(wasm);
        await new Promise(resolve => setImmediate(resolve));
        assert.equal(initialized, false);
    }
});

test('multiplayer content access and preloads stay behind runtime compatibility validation', async () => {
    for (const compatible of [false, true]) {
        const calls: string[] = [];
        const ticket = { code: 'signed', payload: { engine_version: 'a'.repeat(40), net_protocol: 38, schema: 3, relay_url: 'https://relay.example' } } as VerifiedBrowserJoinTicket;
        const deps = fixture(calls);
        const boot = bootGame({ ...deps,
            prepareJoin: async () => ({ ticket, redeemed: true }),
            loadManifest: async () => ({} as Awaited<ReturnType<BootDependencies['loadManifest']>>),
            loadRuntime: async () => ({
                default: async () => {}, wasm_boot: () => { calls.push('boot'); },
                wasm_multiplayer_compatibility: () => ({ engineCommit: (compatible ? 'a' : 'b').repeat(40), artifactShort: 'a'.repeat(12), netProtocol: 38, ticketSchema: 3 }),
                wasm_set_multiplayer_join_ticket: (code, redeemed) => { assert.equal(code, 'signed'); assert.equal(redeemed, true); calls.push('ticket'); },
            }),
            loadDefaultContent: async () => { throw new Error('unexpected default content'); },
            prepareContent: async () => { calls.push('local content'); return { datadir: new Uint8Array([1, 2]), dataBaseUrl: 'data', edition: 'demo', assets: [], shippingFiles: [] }; },
            preloadLocalAssets: () => { calls.push('local assets'); },
            preloadShippingFiles: () => { calls.push('shipping files'); },
        }, new AbortController().signal);
        if (compatible) {
            await boot;
            assert.deepEqual(calls, ['build', 'ticket', 'local content', 'local assets', 'preload', 'rpc', 'boot', 'shipping files', 'canvas', 'ready', 'replay']);
        } else {
            await assert.rejects(boot, /does not exactly match/);
            assert.deepEqual(calls, ['build']);
        }
    }
});
