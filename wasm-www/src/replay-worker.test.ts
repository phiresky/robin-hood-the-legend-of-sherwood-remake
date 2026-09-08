import assert from 'node:assert/strict';
import test from 'node:test';
import { runReplayValidation, validateReplayModule, type ReplayValidatorModule, type ReplayValidationRequest, type ReplayValidationWorker } from './replay-worker.ts';

class WorkerFake extends EventTarget {
    sent: ReplayValidationRequest | undefined;
    terminated = 0;
    postMessage(request: ReplayValidationRequest): void { this.sent = request; }
    terminate(): void { this.terminated++; }
    port(): ReplayValidationWorker { return this as unknown as ReplayValidationWorker; }
}
const request = { compact: 'rhrec-abc-bytes', jsUrl: 'validator.js', wasmUrl: 'validator.wasm' };

test('replay worker accepts only the exact reply and terminates after success or rejection', async () => {
    for (const reply of [{ status: 'accepted' }, { status: 'accepted', extra: true }, { status: 'rejected', error: 'bad replay' }, null, {}]) {
        const worker = new WorkerFake();
        const pending = runReplayValidation(worker.port(), request);
        const expected = reply !== null && reply.status === 'accepted' && !('extra' in reply)
            ? pending : assert.rejects(pending, /bad replay|invalid reply/u);
        assert.deepEqual(worker.sent, request);
        worker.dispatchEvent(new MessageEvent('message', { data: reply }));
        await expected;
        assert.equal(worker.terminated, 1);
    }
});

test('cancellation and deadline always terminate worker and prevent proof acceptance', async () => {
    for (const mode of ['before', 'during', 'deadline']) {
        const controller = new AbortController(), worker = new WorkerFake();
        if (mode === 'before') controller.abort();
        const pending = runReplayValidation(worker.port(), request, { signal: controller.signal, timeoutMs: 5 });
        const rejection = assert.rejects(pending, mode === 'deadline' ? /exceeded/u : { name: 'AbortError' });
        if (mode === 'during') controller.abort();
        await rejection;
        assert.equal(worker.terminated, 1);
        if (mode === 'before') assert.equal(worker.sent, undefined);
    }
});

test('uncloneable replies and worker failures reject and clean up', async () => {
    for (const type of ['messageerror', 'error']) {
        const worker = new WorkerFake();
        const pending = runReplayValidation(worker.port(), request);
        const rejection = assert.rejects(pending, /invalid reply|crashed/u);
        worker.dispatchEvent(new Event(type));
        await rejection;
        assert.equal(worker.terminated, 1);
    }
});

test('isolated validator fetch overlaps glue import and validates only after initialization', async () => {
    let resolveImport!: (module: ReplayValidatorModule) => void;
    const imported = new Promise<ReplayValidatorModule>(resolve => { resolveImport = resolve; });
    let started!: () => void;
    const fetched = new Promise<void>(resolve => { started = resolve; });
    const response = new Response(new Uint8Array([0, 97, 115, 109]));
    const calls: string[] = [];
    const pending = validateReplayModule(request, {
        importModule: async url => { assert.equal(url, request.jsUrl); return imported; },
        fetchModule: async url => { assert.equal(url, request.wasmUrl); started(); return response; },
    });
    await fetched;
    assert.equal(calls.length, 0);
    resolveImport({
        default: async init => { assert.equal(init.module_or_path, response); calls.push('initialized'); },
        validate_compact_replay: content => { assert.equal(content, request.compact); calls.push('validated'); },
    });
    await pending;
    assert.deepEqual(calls, ['initialized', 'validated']);
});

test('validator HTTP and module failures reject before untrusted replay decoding', async () => {
    for (const failure of ['http', 'init', 'export']) {
        let decoded = false;
        await assert.rejects(validateReplayModule(request, {
            importModule: async () => ({
                default: async () => { if (failure === 'init') throw new Error('init failed'); },
                ...(failure === 'export' ? {} : { validate_compact_replay: () => { decoded = true; } }),
            }),
            fetchModule: async () => new Response(null, { status: failure === 'http' ? 500 : 200 }),
        }), /HTTP 500|init failed|no isolated replay validator/);
        assert.equal(decoded, false);
    }
});
