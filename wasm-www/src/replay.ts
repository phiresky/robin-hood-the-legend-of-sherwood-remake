import { withAbort } from './cancellation.ts';
import { runReplayValidation } from './replay-worker.ts';

export const REPLAY_QUERY_KEY = 'replay';
export const PAUSED_QUERY_KEY = 'paused';

export type RobinRpc = <T = unknown>(method: string, params?: unknown) => Promise<T>;

export type ReplayQuery = { readonly content: Uint8Array; readonly paused: boolean };
export type PreparedReplay = ReplayQuery & { readonly buildBase: string };

/** Only an explicit runtime hash overrides latest; a recording's hash is provenance. */
export function replayRuntimeOverride(replay: string | null): string | undefined {
    if (replay === null || replay.length === 0) return undefined;
    if (/^[0-9a-f]{7,40}$/i.test(replay)) return replay;
    if (/^rhrec1-[A-Za-z0-9_-]+$/.test(replay)) return undefined;
    throw new Error('replay= must be a binary replay link or a git hash');
}

export function replayFromQuery(params = new URLSearchParams(window.location.search)): ReplayQuery | null {
    const content = params.get(REPLAY_QUERY_KEY);
    if (content === null || content.length === 0 || /^[0-9a-f]{7,40}$/i.test(content)) {
        return null;
    }
    const pausedRaw = params.get(PAUSED_QUERY_KEY);
    const paused = pausedRaw === null || !/^(0|false|no|off)$/i.test(pausedRaw);
    return { content: decodeReplayLink(content), paused };
}

/** Validation is bound to this exact query snapshot and selected artifact. */
export async function prepareReplay(
    replay: ReplayQuery | null,
    buildBase: string,
    validate: (content: Uint8Array) => Promise<void>,
): Promise<PreparedReplay | null> {
    if (replay === null) return null;
    const bytes = replay.content.slice();
    const snapshot = Object.freeze({ get content() { return bytes.slice(); }, paused: replay.paused, buildBase });
    // Untrusted bitcode parsing stays in a worker with separate linear memory.
    await validate(snapshot.content);
    return snapshot;
}

/** Join independent runtime/admission work before boot; abort siblings on failure. */
export async function prepareReplayWithRuntime<T>(
    replay: ReplayQuery | null,
    buildBase: string,
    loadRuntime: (signal: AbortSignal) => Promise<T>,
    validate: (content: Uint8Array, signal: AbortSignal) => Promise<void>,
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
    markValidated: (content: Uint8Array) => void,
    replay: PreparedReplay | null,
    buildBase: string,
): Promise<boolean> {
    if (replay === null) return false;
    if (replay.buildBase !== buildBase) throw new Error('prepared replay belongs to a different browser artifact');
    const content = replay.content;
    markValidated(content);
    await rpc('load-replay', { data: content, paused: replay.paused });
    return true;
}

export async function validateReplayInWorker(
    content: Uint8Array,
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
                const reply = await rpc<{ data: Uint8Array }>('get-replay');
                if (reply.data.length === 0) {
                    button.title = 'replay empty - nothing to share yet';
                    button.textContent = 'no replay yet';
                    return;
                }
                const url = buildShareUrl(reply.data, { paused: true });
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

function buildShareUrl(content: Uint8Array, opts?: { paused?: boolean }): string {
    const url = new URL(window.location.href);
    url.searchParams.set(REPLAY_QUERY_KEY, encodeReplayLink(content));
    if (opts?.paused === false) {
        url.searchParams.set(PAUSED_QUERY_KEY, '0');
    } else {
        url.searchParams.delete(PAUSED_QUERY_KEY);
    }
    return url.toString();
}

// URLs require text. Only the share-link boundary uses Base64; artifacts and
// worker/RPC/HTTP transports retain the original binary bytes.
export function encodeReplayLink(bytes: Uint8Array): string {
    let raw = '';
    for (let start = 0; start < bytes.length; start += 8192) {
        raw += String.fromCharCode(...bytes.subarray(start, start + 8192));
    }
    return 'rhrec1-' + btoa(raw).replaceAll('+', '-').replaceAll('/', '_').replace(/=+$/u, '');
}

export function decodeReplayLink(text: string): Uint8Array {
    if (text.length > 90 * 1024 * 1024) throw new Error('replay link exceeds input limit');
    if (!/^rhrec1-[A-Za-z0-9_-]+$/u.test(text)) throw new Error('expected a binary replay link');
    const payload = text.slice(7);
    const raw = atob(payload.replaceAll('-', '+').replaceAll('_', '/'));
    const bytes = Uint8Array.from(raw, ch => ch.charCodeAt(0));
    if (encodeReplayLink(bytes) !== text) throw new Error('noncanonical binary replay link');
    return bytes;
}
