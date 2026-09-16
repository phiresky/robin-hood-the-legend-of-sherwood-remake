import type { RobinRpc } from './replay.ts';

/** Start after the mission has entered its frame loop, without holding up boot. */
export async function loadReplayCheckpoints(
    rpc: RobinRpc,
    signal: AbortSignal,
    deps: {
        download: () => Promise<Uint8Array>;
        validate: (bytes: Uint8Array) => Promise<void>;
        install: (bytes: Uint8Array) => void;
        wait?: () => Promise<void>;
    },
): Promise<void> {
    const wait = deps.wait ?? (() => new Promise<void>(resolve => setTimeout(resolve, 100)));
    for (;;) {
        signal.throwIfAborted();
        const state = await rpc<{ replay: unknown }>('state');
        signal.throwIfAborted();
        if (state.replay != null) break;
        await wait();
    }
    const bytes = await deps.download();
    signal.throwIfAborted();
    if (bytes.length === 0) return;
    await deps.validate(bytes);
    signal.throwIfAborted();
    deps.install(bytes);
}
