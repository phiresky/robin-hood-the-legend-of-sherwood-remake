import assert from 'node:assert/strict';
import test from 'node:test';
import { RouteController } from './route-controller.js';

test('superseded routes cannot report stale errors or clear the current busy indicator', async () => {
    const route = new RouteController();
    const states: boolean[] = [], errors: unknown[] = [], rendered: string[] = [];
    const callbacks = { busy: (value: boolean) => { states.push(value); }, error: (error: unknown) => { errors.push(error); } };
    let finishFirst!: () => void, finishSecond!: () => void;
    const firstReady = new Promise<void>(resolve => { finishFirst = resolve; });
    const secondReady = new Promise<void>(resolve => { finishSecond = resolve; });
    const first = route.run(async signal => { await firstReady; signal.throwIfAborted(); rendered.push('first'); }, callbacks);
    const firstSignal = route.signal;
    const second = route.run(async signal => { await secondReady; signal.throwIfAborted(); rendered.push('second'); }, callbacks);
    assert.equal(firstSignal?.aborted, true);
    finishFirst(); await first;
    assert.deepEqual(states, [true, true]);
    assert.deepEqual(rendered, []);
    assert.deepEqual(errors, []);
    finishSecond(); await second;
    assert.deepEqual(states, [true, true, false]);
    assert.deepEqual(rendered, ['second']);
});

test('active failures surface, cancellation suppresses late adapter failures, and the next route remains usable', async () => {
    const route = new RouteController(), errors: unknown[] = [];
    const callbacks = { busy: () => {}, error: (error: unknown) => { errors.push(error); } };
    const failure = new Error('network');
    await route.run(async () => { throw failure; }, callbacks);
    assert.deepEqual(errors, [failure]);
    await route.run(async () => { route.cancel(); throw new Error('late response'); }, callbacks);
    assert.deepEqual(errors, [failure]);
    await route.run(async signal => { assert.equal(signal.aborted, false); }, callbacks);
});
