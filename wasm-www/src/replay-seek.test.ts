import assert from 'node:assert/strict';
import test from 'node:test';
import { seekReplay } from './replay-seek.ts';
import type { RobinRpc } from './replay.ts';

test('long seeks yield between adaptive batches and stop exactly at their target', async () => {
    let frame = 0, time = 0, paints = 0;
    const targets: number[] = [];
    const rpc: RobinRpc = async <T>(method: string, params?: unknown): Promise<T> => {
        if (method === 'state') return { replay: { frame } } as T;
        const next = (params as { frame: number }).frame;
        time += (next - frame) * 2;
        frame = next;
        targets.push(frame);
        return {} as T;
    };
    await seekReplay(rpc, 4000, {
        cancelled: () => false, progress: position => assert.equal(position, frame), now: () => time,
        present: async delay => { paints++; assert(delay >= 0); time += delay; },
    });
    assert.equal(frame, 4000);
    assert(paints > 100, 'progress is painted throughout the seek');
    assert.equal(paints, targets.length - 1);
    assert(targets.every((value, i) => value - (targets[i - 1] ?? 0) <= 15));
});

test('backward seeks rewind to zero, then replay the prefix in visible batches', async () => {
    let frame = 100, cancelled = false;
    const targets: number[] = [];
    const rpc: RobinRpc = async <T>(method: string, params?: unknown): Promise<T> => {
        if (method === 'state') return { replay: { frame } } as T;
        frame = (params as { frame: number }).frame; targets.push(frame); return {} as T;
    };
    await seekReplay(rpc, 80, {
        cancelled: () => cancelled, progress: () => {}, now: () => 0,
        present: async () => { if (frame > 0) cancelled = true; },
    });
    assert.deepEqual(targets, [0, 8]);
});

test('retired and failed seeks do not dispatch subsequent batches', async () => {
    let calls = 0;
    const rpc: RobinRpc = async <T>(): Promise<T> => { calls++; return { replay: { frame: 0 } } as T; };
    await seekReplay(rpc, 4000, { cancelled: () => true, progress: () => assert.fail() });
    assert.equal(calls, 1);
    await assert.rejects(seekReplay(async () => { throw Error('retired'); }, 4000, {
        cancelled: () => false, progress: () => assert.fail(),
    }), /retired/);
});
