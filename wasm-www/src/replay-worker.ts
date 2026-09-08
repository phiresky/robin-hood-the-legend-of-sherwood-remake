export type ReplayValidationWorker = Pick<Worker, 'addEventListener' | 'postMessage' | 'terminate'>;
export type ReplayValidationRequest = { readonly compact: string; readonly jsUrl: string; readonly wasmUrl: string };

/** The worker owns untrusted replay parsing; this adapter owns its lifetime and deadline. */
export async function runReplayValidation(
    worker: ReplayValidationWorker,
    request: ReplayValidationRequest,
    options: { readonly signal?: AbortSignal; readonly timeoutMs?: number } = {},
): Promise<void> {
    const timeoutMs = options.timeoutMs ?? 15_000;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let abort: (() => void) | undefined;
    try {
        options.signal?.throwIfAborted();
        await new Promise<void>((resolve, reject) => {
            timer = setTimeout(() => reject(new Error(
                `isolated replay validation exceeded ${timeoutMs / 1000} seconds`,
            )), timeoutMs);
            abort = () => reject(options.signal?.reason);
            options.signal?.addEventListener('abort', abort, { once: true });
            worker.addEventListener('message', (event: MessageEvent<unknown>) => {
                const reply = event.data;
                if (reply !== null && typeof reply === 'object' && 'status' in reply) {
                    if (reply.status === 'accepted' && Object.keys(reply).length === 1) { resolve(); return; }
                    if (reply.status === 'rejected' && 'error' in reply && typeof reply.error === 'string'
                        && Object.keys(reply).length === 2) { reject(new Error(reply.error)); return; }
                }
                reject(new Error('isolated replay validator returned an invalid reply'));
            }, { once: true });
            worker.addEventListener('error', event => reject(new Error(
                event.message || 'isolated replay validator worker crashed',
            )), { once: true });
            worker.addEventListener('messageerror', () => reject(new Error(
                'isolated replay validator returned an invalid reply',
            )), { once: true });
            worker.postMessage(request);
        });
    } finally {
        clearTimeout(timer);
        if (abort !== undefined) options.signal?.removeEventListener('abort', abort);
        worker.terminate();
    }
}
