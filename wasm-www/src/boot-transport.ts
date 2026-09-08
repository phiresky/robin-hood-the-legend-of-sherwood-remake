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
    return responseWithProgress(resp, url, contentType, onProgress, signal);
}

function responseWithProgress(
    resp: Response,
    url: string,
    contentType: string,
    onProgress: (loaded: number, total: number) => void,
    signal: AbortSignal,
): Response {
    if (!resp.ok) throw new Error(`fetch ${url}: HTTP ${resp.status}`);
    if (resp.body === null) throw new Error(`fetch ${url}: response has no body`);
    // Fetch exposes decoded bytes after native HTTP decompression.
    const encoded = resp.headers.has('Content-Encoding');
    const total = encoded ? 0 : Number(resp.headers.get('Content-Length') ?? 0);
    let loaded = 0;
    const counted = resp.body.pipeThrough(new TransformStream<Uint8Array, Uint8Array>({
        transform(chunk, controller): void {
            loaded += chunk.byteLength;
            onProgress(loaded, total);
            controller.enqueue(chunk);
        },
        flush(): void {
            onProgress(loaded, loaded);
        },
    }), { signal });
    return new Response(counted, { headers: { 'Content-Type': contentType } });
}

/** Prefer smaller offline Brotli when supported, otherwise native HTTP encoding.
 * Uncompressed static hosts retain the historical explicit gzip fallback.
 */
export async function fetchRuntimeWasm(
    url: string,
    preferCompressed: boolean,
    cache: RequestCache,
    onProgress: (loaded: number, total: number) => void,
    signal: AbortSignal,
    fetcher: typeof fetch = fetch,
): Promise<Response> {
    if (!preferCompressed) return fetchWithProgress(url, cache, 'application/wasm', onProgress, signal, fetcher);
    const brotli = await fetchPrecompressedWasm(`${url}.br`, cache, onProgress, signal, fetcher, 'brotli');
    if (brotli !== undefined) return brotli;
    // Cloudflare serves application/wasm with negotiated HTTP compression.
    // This lets Chrome use Brotli without a JS Brotli DecompressionStream.
    const response = await withAbort(signal, () => fetcher(url, { cache, signal }));
    const encoding = response.headers.get('Content-Encoding')?.trim().toLowerCase();
    if (response.status !== 404 && (!response.ok || (encoding !== undefined && encoding !== 'identity' && encoding !== ''))) {
        return responseWithProgress(response, url, 'application/wasm', onProgress, signal);
    }
    // Retain the identity response until a gzip sibling is confirmed. Raw-only
    // static hosts can then consume that same response after the gzip 404.
    if (typeof DecompressionStream === 'undefined') {
        return responseWithProgress(response, url, 'application/wasm', onProgress, signal);
    }
    let gzip: Response | undefined;
    try {
        gzip = await fetchPrecompressedWasm(`${url}.gz`, cache, onProgress, signal, fetcher);
    } catch (error) {
        await response.body?.cancel();
        throw error;
    }
    if (gzip === undefined) return responseWithProgress(response, url, 'application/wasm', onProgress, signal);
    await response.body?.cancel();
    return gzip;
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
    format: 'gzip' | 'brotli' = 'gzip',
): Promise<Response | undefined> {
    if (typeof DecompressionStream === 'undefined') return undefined;
    let decoder: DecompressionStream;
    try {
        // TypeScript's DOM union predates the optional Brotli extension.
        decoder = new DecompressionStream(format as CompressionFormat);
    } catch (error) {
        if (format !== 'brotli' || !(error instanceof TypeError)) throw error;
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
    const httpEncoding = format === 'brotli' ? 'br' : 'gzip';
    const browserDecoded = resp.headers.get('Content-Encoding')?.toLowerCase().split(',').map(value => value.trim()).includes(httpEncoding) === true;
    // HTTP decoding hides compressed byte progress from Fetch. Report decoded
    // bytes with an unknown total until EOF rather than divide by the smaller
    // compressed Content-Length and claim completion early.
    const total = browserDecoded ? 0 : Number(resp.headers.get('Content-Length') ?? 0);
    let loaded = 0;
    const countedBody = resp.body.pipeThrough(
        new TransformStream<Uint8Array<ArrayBuffer>, Uint8Array<ArrayBuffer>>({
            transform(chunk, controller): void {
                loaded += chunk.byteLength;
                onProgress(loaded, total);
                controller.enqueue(chunk);
            },
            flush(): void {
                if (browserDecoded) onProgress(loaded, loaded);
            },
        }),
        { signal },
    );
    const body = browserDecoded ? countedBody : countedBody.pipeThrough(decoder, { signal });
    // A Response lets wasm-bindgen retain instantiateStreaming. Static hosts
    // generally label `.wasm.gz` as generic binary data, so provide the MIME
    // type WebAssembly.instantiateStreaming requires.
    return new Response(body, {
        headers: { 'Content-Type': 'application/wasm' },
    });
}
