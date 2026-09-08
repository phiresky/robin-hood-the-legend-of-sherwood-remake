import assert from 'node:assert/strict';
import test from 'node:test';

import { applyReplayFromQuery, type RobinRpc } from './replay.ts';

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
