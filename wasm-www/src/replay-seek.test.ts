import assert from 'node:assert/strict';
import test from 'node:test';
import { seekReplay } from './replay-seek.ts';
import type { RobinRpc } from './replay.ts';

test('both seek directions restore a checkpoint before simulating the remainder', async () => {
    for (const initial of [0, 4400]) {
        let frame = initial, simulated = 0;
        const rpc: RobinRpc = async <T>(method: string, params?: unknown): Promise<T> => {
            if (method === 'state') return { replay: { frame, checkpoint_seek: true } } as T;
            const request = params as { frame: number; checkpoint_only?: boolean };
            if (request.checkpoint_only) { assert.equal(request.frame, 4010); frame = 4000; }
            else { simulated += request.frame - frame; frame = request.frame; }
            return { frame } as T;
        };
        await seekReplay(rpc, 4010, { cancelled: () => false, progress: () => {}, now: () => 0, present: async () => {} });
        assert.equal(frame, 4010);
        assert.equal(simulated, 10);
    }
});

test('sidecar arriving during a seek is used without restarting', async () => {
    let frame = 0, time = 0, probes = 0, simulated = 0;
    const rpc: RobinRpc = async <T>(method: string, params?: unknown): Promise<T> => {
        if (method === 'state') return { replay: { frame, checkpoint_seek: true } } as T;
        const request = params as { frame: number; checkpoint_only?: boolean };
        if (request.checkpoint_only) { if (++probes === 2) frame = 4000; }
        else { simulated += request.frame - frame; frame = request.frame; time += 40; }
        return { frame } as T;
    };
    await seekReplay(rpc, 4010, { cancelled: () => false, progress: () => {}, now: () => time,
        checkpointRevision: () => time >= 250 ? 1 : 0, present: async () => { time += 40; } });
    assert.equal(frame, 4010);
    assert.equal(probes, 2);
    assert(simulated < 250);
});

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

test('slow presentation does not shrink fast simulation into tiny batches', async () => {
    let frame = 0, time = 0;
    const batches: number[] = [];
    const rpc: RobinRpc = async <T>(method: string, params?: unknown): Promise<T> => {
        if (method === 'state') return { replay: { frame, checkpoint_seek: true } } as T;
        if ((params as { checkpoint_only?: boolean }).checkpoint_only) {
            assert.equal(frame, 0, 'do not re-probe unchanged checkpoints between batches');
            return { frame } as T;
        }
        const next = (params as { frame: number }).frame;
        const work = (next - frame) * 0.6;
        batches.push(next - frame);
        time += 150 + work;
        frame = next;
        return { work_ms: work } as T;
    };
    await seekReplay(rpc, 249, {
        cancelled: () => false, progress: () => {}, now: () => time,
        present: async delay => { time += delay; },
    });
    assert.equal(frame, 249);
    assert(batches.length <= 8, `used ${batches.length} batches for a checkpoint remainder`);
    assert(batches.every(size => size <= 50), 'simulation stays within the 30 ms budget');
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
