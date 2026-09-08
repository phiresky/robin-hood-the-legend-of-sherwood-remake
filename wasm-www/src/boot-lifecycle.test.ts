import assert from 'node:assert/strict';
import test from 'node:test';
import { bootGame, assertMultiplayerWasmCompatibility, type BootDependencies, type RobinWasmModule } from './boot-lifecycle.ts';
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
        assert.equal(signal, controller.signal);
        started();
        return new Promise<RobinWasmModule>(() => {});
    } }, controller.signal);
    await entered;
    controller.abort();
    await assert.rejects(boot, { name: 'AbortError' });
    assert.equal(calls.includes('boot'), false);
});
