import type { RobinRpc } from './replay.js';

const PRESENT_INTERVAL_MS = 40;
const SIMULATION_BUDGET_MS = 30;

/** Leave a browser paint opportunity between bounded batches of replay work. */
export async function seekReplay(
    rpc: RobinRpc,
    target: number,
    options: {
        cancelled: () => boolean;
        progress: (frame: number) => void;
        now?: () => number;
        present?: (delay: number) => Promise<void>;
    },
): Promise<void> {
    const now = options.now ?? (() => performance.now());
    const present = options.present ?? (delay => new Promise<void>(resolve => {
        window.setTimeout(resolve, delay);
    }));
    const requestedStateAt = now();
    const state = await rpc<{ replay: { frame: number } | null }>('state');
    const schedulingMs = Math.min(PRESENT_INTERVAL_MS, now() - requestedStateAt);
    if (options.cancelled()) return;
    if (state.replay === null) throw new Error('Replay is no longer active');
    let frame = state.replay.frame;
    // Backward seeks otherwise replay the entire prefix in one blocking call.
    if (target < frame) {
        await rpc('go-to-frame', { frame: 0, auto_dismiss: true });
        frame = 0;
        if (options.cancelled()) return;
        options.progress(frame);
        await present(0);
    }
    let batch = 8;
    while (frame < target && !options.cancelled()) {
        const next = Math.min(target, frame + batch);
        const started = now();
        await rpc('go-to-frame', { frame: next, auto_dismiss: true });
        const elapsed = Math.max(now() - started, 1);
        // Grow cautiously; shrink immediately if a batch exceeds the budget.
        const workMs = Math.max(1, elapsed - Math.min(schedulingMs, elapsed / 2));
        batch = Math.max(8, Math.min(batch * 2, Math.floor((next - frame) * SIMULATION_BUDGET_MS / workMs)));
        frame = next;
        if (options.cancelled()) return;
        options.progress(frame);
        if (frame < target) await present(Math.max(0, PRESENT_INTERVAL_MS - elapsed));
    }
}
