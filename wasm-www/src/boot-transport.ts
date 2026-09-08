import { withAbort } from './cancellation.ts';

export async function fetchWithProgress(
    url: string,
    cache: RequestCache,
    contentType: string,
    onProgress: (loaded: number, total: number) => void,
    signal: AbortSignal,
    fetcher: typeof fetch = fetch,
): Promise<Response> {
    const resp = await withAbort(signal, () => fetcher(url, { cache, signal }));
    if (!resp.ok) {
        throw new Error(`fetch ${url}: HTTP ${resp.status}`);
    }
    if (resp.body === null) {
        throw new Error(`fetch ${url}: response has no body`);
    }
    const total = Number(resp.headers.get('Content-Length') ?? 0);
    let loaded = 0;
    const counted = resp.body.pipeThrough(
        new TransformStream<Uint8Array, Uint8Array>({
            transform(chunk, controller): void {
                loaded += chunk.byteLength;
                onProgress(loaded, total);
                controller.enqueue(chunk);
            },
        }),
        { signal },
    );
    return new Response(counted, { headers: { 'Content-Type': contentType } });
}


export async function fetchJson<T>(url: string, signal: AbortSignal, fetcher: typeof fetch = fetch): Promise<T> {
    const resp = await withAbort(signal, () => fetcher(url, { cache: 'no-cache', signal }));
    if (!resp.ok) {
        throw new Error(`fetch ${url}: HTTP ${resp.status}`);
    }
    return await withAbort(signal, () => resp.json()) as T;
}


export async function fetchPrecompressedWasm(
    url: string,
    cache: RequestCache,
    onProgress: (loaded: number, total: number) => void,
    signal: AbortSignal,
    fetcher: typeof fetch = fetch,
): Promise<Response | undefined> {
    if (typeof DecompressionStream === 'undefined') {
        return undefined;
    }
    const resp = await withAbort(signal, () => fetcher(url, { cache, signal }));
    if (resp.status === 404) {
        return undefined;
    }
    if (!resp.ok) {
        throw new Error(`fetch ${url}: HTTP ${resp.status}`);
    }
    if (resp.body === null) {
        throw new Error(`fetch ${url}: response has no body`);
    }
    // Count the network-side bytes (before decompression) so progress lines
    // up with the `.gz` Content-Length actually crossing the wire.
    const total = Number(resp.headers.get('Content-Length') ?? 0);
    let loaded = 0;
    const countedBody = resp.body.pipeThrough(
        new TransformStream<Uint8Array<ArrayBuffer>, Uint8Array<ArrayBuffer>>({
            transform(chunk, controller): void {
                loaded += chunk.byteLength;
                onProgress(loaded, total);
                controller.enqueue(chunk);
            },
        }),
        { signal },
    );
    const body = resp.headers.get('Content-Encoding')?.toLowerCase().includes('gzip') === true
        ? countedBody
        : countedBody.pipeThrough(new DecompressionStream('gzip'), { signal });
    // A Response lets wasm-bindgen retain instantiateStreaming. Static hosts
    // generally label `.wasm.gz` as generic binary data, so provide the MIME
    // type WebAssembly.instantiateStreaming requires.
    return new Response(body, {
        headers: { 'Content-Type': 'application/wasm' },
    });
}
