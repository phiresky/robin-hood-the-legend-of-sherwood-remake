import type { RobinWasmModule } from './boot-lifecycle.js';
import { withAbort } from './cancellation.ts';

const PRELOAD_FETCH_CONCURRENCY = 12;
type PreloadEntry = string | { readonly path?: unknown; readonly url?: unknown };
export type PreloadDependencies = {
    readonly signal?: AbortSignal;
    fetch: typeof fetch;
    log: (message: string) => void;
    progress: (fraction: number, detail: string) => void;
};

export function resolvePreloadEntries(raw: unknown, manifestUrl: string, buildBase: string): { path: string; assetUrl: string }[] {
    if (!Array.isArray(raw)) {
        throw new Error(`${manifestUrl} must be a JSON array`);
    }
    return (raw as PreloadEntry[]).map((entry, index) => {
        if (typeof entry !== 'string' && (entry === null || typeof entry !== 'object' || Array.isArray(entry))) {
            throw new Error(`${manifestUrl} contains an invalid preload entry at index ${index}`);
        }
        const path = typeof entry === 'string' ? entry : entry.path;
        const url = typeof entry === 'string' ? `${buildBase}/${entry}` : entry.url ?? path;
        if (typeof path !== 'string' || typeof url !== 'string' || path.length === 0 || url.length === 0) {
            throw new Error(`${manifestUrl} contains an invalid preload entry at index ${index}`);
        }
        const assetUrl = new URL(
            url,
            buildBase.endsWith('/') ? buildBase : `${buildBase}/`,
        ).toString();
        return { path, assetUrl };
    });
}

export async function preloadRuntimeAssets(
    wasm: RobinWasmModule,
    buildBase: string,
    noCache: boolean,
    deps: PreloadDependencies,
): Promise<void> {
    const signal = deps.signal ?? new AbortController().signal;
    signal.throwIfAborted();
    if (wasm.wasm_preload_asset === undefined) {
        return;
    }
    const preloadAsset = wasm.wasm_preload_asset;
    // Browser fetch rejects a dependency object as its receiver. Invoke the
    // injected function detached, as the normal global fetch call would be.
    const fetchAsset = deps.fetch;
    const manifestUrl = `${buildBase}/preload-assets.json`;
    const manifestResp = await withAbort(signal, () => fetchAsset(manifestUrl, {
        cache: noCache ? 'no-cache' : 'force-cache',
        signal,
    }));
    if (manifestResp.status === 404) {
        return;
    }
    if (!manifestResp.ok) {
        throw new Error(`fetch ${manifestUrl}: HTTP ${manifestResp.status}`);
    }
    const raw = await withAbort(signal, () => manifestResp.json()) as unknown;
    const entries = resolvePreloadEntries(raw, manifestUrl, buildBase);

    // Fetch and install in the same bounded worker. Keeping every completed
    // ArrayBuffer until all requests finish doubles the preload peak: JS owns
    // all downloads while wasm_preload_asset copies them into Rust.
    let preloaded = 0;
    await forEachConcurrent(
        entries,
        PRELOAD_FETCH_CONCURRENCY,
        async ({ path, assetUrl }) => {
            const assetResp = await withAbort(signal, () => fetchAsset(assetUrl, {
                cache: noCache ? 'no-cache' : 'force-cache',
                signal,
            }));
            if (!assetResp.ok) {
                throw new Error(`fetch ${assetUrl}: HTTP ${assetResp.status}`);
            }
            const bytes = new Uint8Array(await withAbort(signal, () => assetResp.arrayBuffer()));
            signal.throwIfAborted();
            preloadAsset(path, bytes);
            preloaded += 1;
            deps.progress(preloaded / entries.length, `${preloaded} / ${entries.length}`);
            deps.log(`[preloaded ${path}: ${bytes.byteLength} bytes]`);
        },
        signal,
    );
}

export async function forEachConcurrent<T>(
    items: readonly T[],
    concurrency: number,
    action: (item: T, index: number) => Promise<void>,
    signal?: AbortSignal,
): Promise<void> {
    if (!Number.isInteger(concurrency) || concurrency < 1) {
        throw new Error(`preload concurrency must be a positive integer, got ${concurrency}`);
    }
    const errors = new Array<Error | undefined>(items.length);
    let next = 0;
    const worker = async (): Promise<void> => {
        for (;;) {
            if (signal?.aborted) return;
            const index = next++;
            if (index >= items.length) {
                return;
            }
            try {
                await action(items[index] as T, index);
            } catch (error) {
                errors[index] = error instanceof Error ? error : new Error(String(error));
            }
        }
    };
    const workerCount = Math.min(concurrency, items.length);
    await Promise.all(Array.from({ length: workerCount }, worker));
    signal?.throwIfAborted();
    const firstError = errors.find((error): error is Error => error !== undefined);
    if (firstError !== undefined) {
        throw firstError;
    }
}
