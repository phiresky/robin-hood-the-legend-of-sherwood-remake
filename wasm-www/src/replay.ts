import { withAbort } from './cancellation.ts';
import { runReplayValidation } from './replay-worker.ts';

export const REPLAY_QUERY_KEY = 'replay';
export const PAUSED_QUERY_KEY = 'paused';

export type RobinRpc = <T = unknown>(method: string, params?: unknown) => Promise<T>;

export type IsolatedReplayAdmission = {
    readonly validate: (content: string) => Promise<void>;
    readonly markValidated: (content: string) => void;
};

export type ReplayQuery = { readonly content: string; readonly paused: boolean };
export type PreparedReplay = ReplayQuery & { readonly buildBase: string };

export function replayFromQuery(params = new URLSearchParams(window.location.search)): ReplayQuery | null {
    const content = params.get(REPLAY_QUERY_KEY);
    if (content === null || content.length === 0) {
        return null;
    }
    const pausedRaw = params.get(PAUSED_QUERY_KEY);
    const paused = pausedRaw === null || !/^(0|false|no|off)$/i.test(pausedRaw);
    return { content, paused };
}

export async function applyReplayFromQuery(
    rpc: RobinRpc,
    admission: IsolatedReplayAdmission,
): Promise<boolean> {
    const replay = replayFromQuery();
    if (replay === null) {
        return false;
    }
    const prepared = await prepareReplay(replay, '', admission.validate);
    return applyPreparedReplay(rpc, admission.markValidated, prepared, '');
}

/** Validation is bound to this exact query snapshot and selected artifact. */
export async function prepareReplay(
    replay: ReplayQuery | null,
    buildBase: string,
    validate: (content: string) => Promise<void>,
): Promise<PreparedReplay | null> {
    if (replay === null) return null;
    const snapshot = Object.freeze({ content: replay.content, paused: replay.paused, buildBase });
    // Untrusted bitcode parsing stays in a worker with separate linear memory.
    await validate(snapshot.content);
    return snapshot;
}

/** Join independent runtime/admission work before boot; abort siblings on failure. */
export async function prepareReplayWithRuntime<T>(
    replay: ReplayQuery | null,
    buildBase: string,
    loadRuntime: (signal: AbortSignal) => Promise<T>,
    validate: (content: string, signal: AbortSignal) => Promise<void>,
    signal: AbortSignal,
): Promise<{ readonly runtime: T; readonly replay: PreparedReplay | null }> {
    const failed = new AbortController();
    const loadingSignal = AbortSignal.any([signal, failed.signal]);
    try {
        const [runtime, prepared] = await Promise.all([
            withAbort(loadingSignal, () => loadRuntime(loadingSignal)),
            prepareReplay(replay, buildBase, content => withAbort(loadingSignal, () => validate(content, loadingSignal))),
        ]);
        loadingSignal.throwIfAborted();
        return { runtime, replay: prepared };
    } catch (error) {
        failed.abort(error);
        throw error;
    }
}

export async function applyPreparedReplay(
    rpc: RobinRpc,
    markValidated: (content: string) => void,
    replay: PreparedReplay | null,
    buildBase: string,
): Promise<boolean> {
    if (replay === null) return false;
    if (replay.buildBase !== buildBase) throw new Error('prepared replay belongs to a different browser artifact');
    markValidated(replay.content);
    await rpc('load-replay', { data: replay.content, paused: replay.paused });
    return true;
}

export async function validateReplayInWorker(
    content: string,
    jsUrl: string,
    wasmUrl: string,
    signal?: AbortSignal,
): Promise<void> {
    const worker = new Worker(new URL('./replay_validation_worker.ts', import.meta.url), {
        type: 'module',
        name: 'robin-replay-admission',
    });
    await runReplayValidation(worker, { compact: content, jsUrl, wasmUrl }, signal === undefined ? {} : { signal });
}

export function installShareButton(button: HTMLButtonElement, rpc: RobinRpc): void {
    const originalLabel = button.textContent ?? 'Share replay';
    button.addEventListener('click', () => {
        void (async (): Promise<void> => {
            button.disabled = true;
            try {
                const reply = await rpc<{ content: string }>('get-replay');
                if (reply.content.length === 0) {
                    button.title = 'replay empty - nothing to share yet';
                    button.textContent = 'no replay yet';
                    return;
                }
                const url = buildShareUrl(reply.content, { paused: true });
                await navigator.clipboard.writeText(url);
                button.title = url;
                button.textContent = 'link copied';
            } catch (e) {
                const msg = e instanceof Error ? e.message : String(e);
                button.title = `share failed: ${msg}`;
                button.textContent = 'share failed';
                console.error('replay: share button failed:', e);
            } finally {
                setTimeout(() => {
                    button.disabled = false;
                    button.textContent = originalLabel;
                }, 2000);
            }
        })();
    });
}

function buildShareUrl(content: string, opts?: { paused?: boolean }): string {
    const url = new URL(window.location.href);
    url.searchParams.set(REPLAY_QUERY_KEY, content);
    if (opts?.paused === false) {
        url.searchParams.set(PAUSED_QUERY_KEY, '0');
    } else {
        url.searchParams.delete(PAUSED_QUERY_KEY);
    }
    return url.toString();
}
