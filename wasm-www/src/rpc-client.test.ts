import assert from 'node:assert/strict';
import test from 'node:test';
import { createRpcClient } from './rpc-client.ts';

test('RPC client forwards requests and supplies the default null params', async () => {
    const calls: unknown[] = [];
    const rpc = createRpcClient({ rh_rpc: async <T>(request: unknown): Promise<T> => {
        calls.push(request);
        return { frame: 7 } as T;
    } });
    assert.deepEqual(await rpc('state'), { frame: 7 });
    await rpc('go-to-frame', { frame: 7 });
    assert.deepEqual(calls, [{ method: 'state', params: null }, { method: 'go-to-frame', params: { frame: 7 } }]);
});

test('RPC client normalizes Rust string rejections and preserves existing Errors', async () => {
    const error = new Error('engine not ready');
    for (const failure of ['unknown method: state', error]) {
        const rpc = createRpcClient({ rh_rpc: async () => { throw failure; } });
        await assert.rejects(rpc('state'), (caught: unknown) => {
            assert.ok(caught instanceof Error);
            if (failure instanceof Error) assert.equal(caught, failure);
            else {
                assert.equal(caught.message, failure);
                assert.equal(caught.cause, failure);
            }
            return true;
        });
    }
});

test('RPC client fails explicitly when the wasm bridge is missing', () => {
    assert.throws(() => createRpcClient({}), /does not export rh_rpc/);
});
