import assert from 'node:assert/strict';
import test from 'node:test';
import { runReplayValidation, type ReplayValidationRequest, type ReplayValidationWorker } from './replay-worker.ts';

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
