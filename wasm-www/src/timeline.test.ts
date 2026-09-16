import assert from 'node:assert/strict';
import test, { type TestContext } from 'node:test';
import { JSDOM } from 'jsdom';
import { installTimeline } from './timeline.ts';
import type { RobinRpc } from './replay.ts';

test('timeline hides live sessions and displays replay frames without live-frame state', async t => {
    const dom = new JSDOM('<div></div>');
    const originalWindow = Object.getOwnPropertyDescriptor(globalThis, 'window');
    const originalDocument = Object.getOwnPropertyDescriptor(globalThis, 'document');
    let poll: (() => void) | undefined;
    t.after(() => {
        poll = undefined;
        dom.window.close();
        for (const [name, descriptor] of [['window', originalWindow], ['document', originalDocument]] as const) {
            if (descriptor === undefined) Reflect.deleteProperty(globalThis, name);
            else Object.defineProperty(globalThis, name, descriptor);
        }
    });
    Object.defineProperty(globalThis, 'document', { configurable: true, value: dom.window.document });
    Object.defineProperty(globalThis, 'window', { configurable: true, value: {
        setInterval(callback: () => void, delay: number) {
            assert.equal(delay, 500);
            poll = callback;
            return 1;
        },
        clearInterval(id: number) { assert.equal(id, 1); poll = undefined; },
    } });
    let reply: unknown = { frame: 2500, replay: null };
    let failure: Error | string | undefined;
    const rpc: RobinRpc = async <T>(method: string): Promise<T> => {
        assert.equal(method, 'state');
        if (failure !== undefined) throw failure;
        return reply as T;
    };
    const container = dom.window.document.querySelector('div')!;
    installTimeline(container, rpc);
    await new Promise(resolve => setImmediate(resolve));
    assert.equal(container.style.display, 'none');

    reply = { frame: 2500, replay: { frame: 25, total: 100, paused: true } };
    poll!();
    await new Promise(resolve => setImmediate(resolve));
    assert.equal(container.style.display, 'flex');
    assert.equal(container.querySelector('input')!.value, '25');
    assert.equal(container.querySelector('input')!.max, '100');
    assert.equal(container.querySelector('button')!.textContent, 'Play');
    assert.deepEqual([...container.querySelectorAll('span')].map(span => span.textContent), ['00:01', '00:04']);

    failure = 'engine not ready';
    poll!();
    await new Promise(resolve => setImmediate(resolve));
    assert.equal(container.style.display, 'none');
    assert.notEqual(poll, undefined);
    failure = 'unknown method: state';
    poll!();
    await new Promise(resolve => setImmediate(resolve));
    assert.equal(container.style.display, 'none');
    assert.equal(poll, undefined);
});

function pendingTimeline(t: TestContext) {
    const dom = new JSDOM('<div></div>');
    const originals = ['window', 'document'].map(name => [name, Object.getOwnPropertyDescriptor(globalThis, name)] as const);
    let tick = (): void => { throw new Error('interval not installed'); };
    let cleared = 0;
    Object.defineProperty(globalThis, 'document', { configurable: true, value: dom.window.document });
    Object.defineProperty(globalThis, 'window', { configurable: true, value: {
        setInterval(callback: () => void) { tick = callback; return 1; },
        clearInterval(id: number) { assert.equal(id, 1); cleared++; },
    } });
    const calls: Array<{ method: string; params: unknown; resolve: (reply: unknown) => void; reject: (error: unknown) => void }> = [];
    const rpc: RobinRpc = <T>(method: string, params?: unknown) => new Promise<T>((resolve, reject) => {
        calls.push({ method, params, resolve: reply => resolve(reply as T), reject });
    });
    const container = dom.window.document.querySelector('div')!;
    const dispose = installTimeline(container, rpc);
    t.after(() => {
        dispose();
        dom.window.close();
        for (const [name, descriptor] of originals) {
            if (descriptor === undefined) Reflect.deleteProperty(globalThis, name);
            else Object.defineProperty(globalThis, name, descriptor);
        }
    });
    const scrub = container.querySelector('input')!;
    scrub.max = '100';
    return {
        calls, container, dispose, scrub,
        tick: () => tick(),
        cleared: () => cleared,
        seek(frame: number) {
            scrub.value = String(frame);
            scrub.dispatchEvent(new dom.window.Event('input'));
        },
    };
}

const settle = (): Promise<void> => new Promise(resolve => setImmediate(resolve));

test('timeline has one pending poll and ignores replies and events after disposal', async t => {
    const timeline = pendingTimeline(t);
    timeline.tick();
    timeline.tick();
    assert.equal(timeline.calls.length, 1);
    timeline.dispose();
    timeline.dispose();
    assert.equal(timeline.cleared(), 1);
    timeline.calls[0]!.resolve({ replay: { frame: 90, total: 100, paused: false } });
    await settle();
    assert.equal(timeline.container.style.display, 'none');
    assert.equal(timeline.scrub.value, '0');
    timeline.tick();
    timeline.seek(50);
    timeline.container.querySelector('button')!.click();
    assert.equal(timeline.calls.length, 1);
});

test('timeline coalesces seeks and discards a pre-interaction state reply', async t => {
    const timeline = pendingTimeline(t);
    timeline.seek(2);
    timeline.seek(4);
    timeline.seek(6);
    assert.deepEqual(timeline.calls.map(call => call.method), ['state', 'state']);
    timeline.calls[0]!.resolve({ replay: { frame: 0, total: 100, paused: false } });
    await settle();
    assert.equal(timeline.scrub.value, '6');
    timeline.tick();
    assert.equal(timeline.calls.length, 2);
    timeline.calls[1]!.resolve({ replay: { frame: 0 } });
    await settle();
    assert.equal(timeline.calls[2]!.method, 'state');
    timeline.calls[2]!.resolve({ replay: { frame: 0 } });
    await settle();
    assert.deepEqual(timeline.calls[3]!.params, { frame: 6, auto_dismiss: true });
    timeline.calls[3]!.resolve(null);
    await settle();
    timeline.tick();
    assert.equal(timeline.calls[4]!.method, 'state');
    timeline.calls[4]!.resolve({ replay: { frame: 6, total: 100, paused: true } });
    await settle();
    assert.equal(timeline.container.style.display, 'flex');
});

test('disposal retires queued seeks and late rejected requests quietly', async t => {
    const timeline = pendingTimeline(t);
    const warnings: unknown[][] = [];
    t.mock.method(console, 'warn', (...args: unknown[]) => { warnings.push(args); });
    timeline.seek(10);
    timeline.seek(30);
    timeline.dispose();
    timeline.calls[0]!.reject('engine not ready');
    timeline.calls[1]!.reject('runtime retired');
    await settle();
    assert.equal(timeline.calls.length, 2);
    assert.deepEqual(warnings, []);
});

test('failed seeks release the queue and still send its latest target', async t => {
    const timeline = pendingTimeline(t);
    const warnings: unknown[][] = [];
    t.mock.method(console, 'warn', (...args: unknown[]) => { warnings.push(args); });
    timeline.seek(10);
    timeline.seek(30);
    timeline.calls[1]!.reject('seek failed');
    await settle();
    assert.equal(warnings.length, 1);
    assert.equal(timeline.calls[2]!.method, 'state');
    timeline.calls[2]!.resolve({ replay: { frame: 25 } });
    await settle();
    assert.deepEqual(timeline.calls[3]!.params, { frame: 30, auto_dismiss: true });
});

test('play/pause interrupts a pending seek before another batch is sent', async t => {
    const timeline = pendingTimeline(t);
    timeline.calls[0]!.resolve({ replay: { frame: 0, total: 100, paused: true } });
    await settle();
    timeline.seek(90);
    timeline.container.querySelector('button')!.click();
    assert.equal(timeline.calls[2]!.method, 'set-paused');
    timeline.calls[1]!.resolve({ replay: { frame: 0 } });
    timeline.calls[2]!.resolve(null);
    await settle();
    assert.equal(timeline.calls.length, 3);
    assert.equal(timeline.container.hasAttribute('aria-busy'), false);
});
