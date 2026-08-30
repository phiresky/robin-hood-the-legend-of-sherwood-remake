export const REPLAY_QUERY_KEY = 'replay';
export const PAUSED_QUERY_KEY = 'paused';

export type RobinRpc = <T = unknown>(method: string, params?: unknown) => Promise<T>;

export type IsolatedReplayAdmission = {
    readonly validate: (content: string) => Promise<void>;
    readonly markValidated: (content: string) => void;
};

const REPLAY_VALIDATION_WALL_MS = 15_000;

export function replayFromQuery(): { content: string; paused: boolean } | null {
    const params = new URLSearchParams(window.location.search);
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
    // The worker owns a separate wasm linear memory. A malformed bitcode graph
    // can trap/exhaust that worker, but is never decoded in the live game's
    // wasm instance until the exact one-shot digest has been installed.
    await admission.validate(replay.content);
    admission.markValidated(replay.content);
    await rpc('load-replay', {
        data: replay.content,
        paused: replay.paused,
    });
    return true;
}

export async function validateReplayInWorker(
    content: string,
    jsUrl: string,
    wasmUrl: string,
): Promise<void> {
    type Reply =
        | { readonly status: 'accepted' }
        | { readonly status: 'rejected'; readonly error: string };

    const worker = new Worker(new URL('./replay_validation_worker.ts', import.meta.url), {
        type: 'module',
        name: 'robin-replay-admission',
    });
    try {
        await new Promise<void>((resolve, reject) => {
            const timeout = window.setTimeout(() => {
                reject(new Error(
                    `isolated replay validation exceeded ${REPLAY_VALIDATION_WALL_MS / 1000} seconds`,
                ));
            }, REPLAY_VALIDATION_WALL_MS);
            const finish = (action: () => void): void => {
                window.clearTimeout(timeout);
                action();
            };
            worker.addEventListener('message', (event: MessageEvent<Reply>) => {
                const reply = event.data;
                if (reply.status === 'accepted') {
                    finish(resolve);
                } else {
                    finish(() => reject(new Error(reply.error)));
                }
            }, { once: true });
            worker.addEventListener('error', (event) => {
                finish(() => reject(new Error(
                    event.message || 'isolated replay validator worker crashed',
                )));
            }, { once: true });
            worker.addEventListener('messageerror', () => {
                finish(() => reject(new Error('isolated replay validator returned an invalid reply')));
            }, { once: true });
            worker.postMessage({ compact: content, jsUrl, wasmUrl });
        });
    } finally {
        worker.terminate();
    }
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
