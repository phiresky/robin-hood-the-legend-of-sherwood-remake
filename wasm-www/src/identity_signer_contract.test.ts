import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { startIdentitySigner, type InitializedBridge } from './identity-signer/startup.ts';

const signerHtml = readFileSync(new URL('../identity-signer/index.html', import.meta.url), 'utf8');

test('signer initializes the exact wasm bridge before registering either dispatcher', async () => {
    const calls: string[] = [];
    let resolveReady!: () => void;
    const ready = new Promise<void>(resolve => { resolveReady = resolve; });
    const bridge = { default: async ({ module_or_path }: { module_or_path: string }) => {
        calls.push(module_or_path);
        await ready;
    } } as InitializedBridge;
    const pending = startIdentitySigner({
        load: async url => { calls.push(url); return bridge; },
        leaderboard: actual => { assert.equal(actual, bridge); calls.push('leaderboard'); },
        multiplayer: () => { calls.push('multiplayer'); },
    });
    await Promise.resolve();
    assert.deepEqual(calls, ['/identity-signer/bridge/leaderboard_identity_bridge.js',
        '/identity-signer/bridge/leaderboard_identity_bridge_bg.wasm']);
    resolveReady();
    await pending;
    assert.deepEqual(calls.slice(-2), ['leaderboard', 'multiplayer']);
});

test('module or wasm initialization failures leave both dispatchers unavailable', async () => {
    for (const failure of ['load', 'initialize']) {
        const calls: string[] = [];
        await assert.rejects(startIdentitySigner({
            load: async () => {
                if (failure === 'load') throw new Error('load');
                return { default: async () => { throw new Error('initialize'); } } as unknown as InitializedBridge;
            },
            leaderboard: () => { calls.push('leaderboard'); },
            multiplayer: () => { calls.push('multiplayer'); },
        }), new RegExp(failure));
        assert.deepEqual(calls, []);
    }
});

test('signer document permits only same-origin code, WebAssembly, and storage', () => {
    assert.match(signerHtml, /connect-src 'self';/u);
    assert.match(signerHtml, /default-src 'none'/u);
    assert.match(signerHtml, /script-src 'self' 'wasm-unsafe-eval'/u);
    assert.doesNotMatch(signerHtml, /(?:^|\s)'unsafe-eval'(?:\s|;)|https:|wss:|blob:/u);
});
