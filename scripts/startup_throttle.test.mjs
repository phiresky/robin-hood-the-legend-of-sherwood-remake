import { test } from 'node:test';
import assert from 'node:assert/strict';
import { SharedBandwidth } from './startup_throttle.mjs';
test('all clients share one budget and round-robin chunks, with no initial burst', async () => {
    let time = 0;
    const callbacks = [];
    const events = [];
    const throttle = new SharedBandwidth(2_000_000, { now: () => time, schedule: (fn, delay) => callbacks.push(() => { time += delay; fn(); }) });
    const response = id => ({ write: bytes => events.push([id, bytes.length, time]), end() {} });
    const promises = [throttle.send(response('main'), Buffer.alloc(32768)), throttle.send(response('worker'), Buffer.alloc(32768))];
    while (callbacks.length) callbacks.shift()();
    await Promise.all(promises);
    assert.deepEqual(events.map(e => e[0]), ['main', 'worker', 'main', 'worker']);
    for (let i = 0; i < events.length; i++) assert.ok((i + 1) * 16384 <= events[i][2] * 2000);
});
test('a disconnected response does not hold up other clients', async () => {
    const callbacks = [];
    const throttle = new SharedBandwidth(1000, { schedule: fn => callbacks.push(fn) });
    let ended = false;
    const pending = throttle.send({ destroyed: true }, Buffer.alloc(100));
    const live = throttle.send({ write() {}, end() { ended = true; } }, Buffer.alloc(1));
    while (callbacks.length) callbacks.shift()();
    await Promise.all([pending, live]);
    assert.equal(ended, true);
});
test('fractional millisecond chunks accumulate deadlines instead of losing throughput to rounding', async () => {
    let time = 0;
    const callbacks = [];
    const throttle = new SharedBandwidth(2_000_000, { now: () => time, schedule: (fn, delay) => callbacks.push(() => { time += delay; fn(); }) });
    let bytes = 0;
    const pending = throttle.send({ write: chunk => { bytes += chunk.length; assert.ok(bytes <= time * 2000); }, end() {} }, Buffer.alloc(2_000_000));
    while (callbacks.length) callbacks.shift()();
    await pending;
    assert.ok(time >= 1000 && time <= 1001); // At most one millisecond of rounding for the whole transfer.
    assert.equal(bytes, 2_000_000);
});
