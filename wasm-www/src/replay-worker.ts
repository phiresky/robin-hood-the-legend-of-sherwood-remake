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

export type ReplayValidatorModule = {
    readonly default: (init: { module_or_path: Response }) => Promise<unknown>;
    readonly validate_compact_replay?: (compact: string) => void;
};

/** Runs only inside the disposable worker; the live game never parses here. */
export async function validateReplayModule(
    request: ReplayValidationRequest,
    deps: {
        readonly importModule: (url: string) => Promise<ReplayValidatorModule>;
        readonly fetchModule: (url: string) => Promise<Response>;
    },
): Promise<void> {
    const [module, response] = await Promise.all([
        deps.importModule(request.jsUrl),
        deps.fetchModule(request.wasmUrl),
    ]);
    if (!response.ok) throw new Error(`fetch replay validator: HTTP ${response.status}`);
    await module.default({ module_or_path: response });
    if (module.validate_compact_replay === undefined) {
        throw new Error('selected wasm build has no isolated replay validator');
    }
    module.validate_compact_replay(request.compact);
}
