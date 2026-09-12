import assert from 'node:assert/strict';
import test from 'node:test';
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
    let failure: Error | undefined;
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

    failure = new Error('engine not ready');
    poll!();
    await new Promise(resolve => setImmediate(resolve));
    assert.equal(container.style.display, 'none');
    assert.notEqual(poll, undefined);
    failure = new Error('unknown method: state');
    poll!();
    await new Promise(resolve => setImmediate(resolve));
    assert.equal(container.style.display, 'none');
    assert.equal(poll, undefined);
});
