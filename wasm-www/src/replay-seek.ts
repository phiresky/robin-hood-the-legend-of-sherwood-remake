import type { RobinRpc } from './replay.js';
import { replayCheckpointRevision } from './replay-checkpoints.ts';

const PRESENT_INTERVAL_MS = 500;
const SIMULATION_BUDGET_MS = PRESENT_INTERVAL_MS - 50;

/** Leave a browser paint opportunity between bounded batches of replay work. */
export async function seekReplay(
    rpc: RobinRpc,
    target: number,
    options: {
        cancelled: () => boolean;
        progress: (frame: number) => void;
        now?: () => number;
        present?: (delay: number) => Promise<void>;
        checkpointRevision?: () => number;
    },
): Promise<void> {
    const now = options.now ?? (() => performance.now());
    const present = options.present ?? (delay => new Promise<void>(resolve => {
        window.setTimeout(resolve, delay);
    }));
    const requestedStateAt = now();
    const state = await rpc<{ replay: { frame: number; checkpoint_seek?: boolean } | null }>('state');
    const schedulingMs = Math.min(PRESENT_INTERVAL_MS, now() - requestedStateAt);
    if (options.cancelled()) return;
    if (state.replay === null) throw new Error('Replay is no longer active');
    let frame = state.replay.frame;
    const checkpointRevision = options.checkpointRevision ?? replayCheckpointRevision;
    let lastRevision = checkpointRevision();
    const checkpointSeek = state.replay.checkpoint_seek === true;
    const restoreCheckpoint = async () => {
        const result = await rpc<{ frame: number }>('go-to-frame', { frame: target, auto_dismiss: true, checkpoint_only: true });
        if (!Number.isSafeInteger(result.frame) || result.frame < 0 || result.frame > target) throw new Error('Invalid replay checkpoint position');
        frame = result.frame;
        if (!options.cancelled()) options.progress(frame);
    };
    if (checkpointSeek) {
        await restoreCheckpoint();
        if (options.cancelled()) return;
    }
    // Backward seeks otherwise replay the entire prefix in one blocking call.
    if (target < frame) {
        await rpc('go-to-frame', { frame: 0, auto_dismiss: true });
        frame = 0;
        if (options.cancelled()) return;
        options.progress(frame);
        await present(0);
    }
    let batch = 128;
    const measurements: { frames: number; work_ms: number; elapsed_ms: number }[] = [];
    while (frame < target && !options.cancelled()) {
        // A background download may finish while a long seek is underway.
        if (checkpointSeek && checkpointRevision() !== lastRevision) {
            lastRevision = checkpointRevision();
            await restoreCheckpoint();
            if (options.cancelled() || frame === target) break;
        }
        const next = Math.min(target, frame + batch);
        const started = now();
        const result = await rpc<{ work_ms?: number }>('go-to-frame', { frame: next, auto_dismiss: true });
        const elapsed = Math.max(now() - started, 1);
        // Grow cautiously; shrink immediately if a batch exceeds the budget.
        // Rendering and RPC scheduling are paid once per batch, not per tick.
        // Including them here would shrink batches when presentation is slow.
        const workMs = Math.max(1, typeof result.work_ms === 'number' && Number.isFinite(result.work_ms) && result.work_ms >= 0
            ? result.work_ms : elapsed - Math.min(schedulingMs, elapsed / 2));
        measurements.push({ frames: next - frame, work_ms: workMs, elapsed_ms: elapsed });
        batch = Math.max(8, Math.min(batch * 2, Math.floor((next - frame) * SIMULATION_BUDGET_MS / workMs)));
        frame = next;
        if (options.cancelled()) return;
        options.progress(frame);
        // Spend the half-second budget advancing playback, not sleeping while
        // the ordinary game loop repeatedly redraws the same paused state.
        if (frame < target) await present(0);
    }
    console.info('[replay seek timing]', JSON.stringify({
        target, elapsed_ms: now() - requestedStateAt, batches: measurements.length,
        catchup_work_ms: measurements.reduce((sum, batch) => sum + batch.work_ms, 0),
        catchup_rpc_ms: measurements.reduce((sum, batch) => sum + batch.elapsed_ms, 0),
    }));
}
