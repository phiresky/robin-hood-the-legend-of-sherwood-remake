import { appendLogLine } from './log.js';
import { applyReplayFromQuery, installShareButton, type RobinRpc } from './replay.js';
import { installTimeline } from './timeline.js';

declare global {
    // Optional test/dev override for loading binaries from a local checkout.
    // Keep this global so the deployed HTML can stay config-free.
    var ROBIN_WASM_BINARIES_BASE: string | undefined;
    var robinRpc: ((method: string, params?: unknown) => Promise<unknown>) | undefined;
}

type BuildSelection = {
    readonly short: string;
    readonly source: 'latest' | 'replay';
    readonly buildBase?: string;
};

type BuildManifest = {
    readonly commit?: unknown;
    readonly short?: unknown;
};

type RobinWasmModule = {
    readonly default: (init?: {
        module_or_path?: string | URL | Request | Response | ArrayBuffer;
    }) => Promise<unknown>;
    readonly wasm_boot: (datadir: Uint8Array, dataBaseUrl: string) => void;
    readonly wasm_preload_asset?: (path: string, bytes: Uint8Array) => void;
    readonly rh_rpc?: <T = unknown>(request: { method: string; params: unknown }) => Promise<T>;
};

type PreloadEntry = string | {
    readonly path?: unknown;
    readonly url?: unknown;
};

const DEFAULT_BINARIES_BASE = import.meta.env.DEV
    ? window.location.origin
    : 'https://phiresky.github.io/robin-hood-the-legend-of-sherwood-remake-binaries';
const pageParams = new URLSearchParams(window.location.search);
const BINARIES_BASE =
    pageParams.get('binaries-base') ??
    pageParams.get('binaries_base') ??
    globalThis.ROBIN_WASM_BINARIES_BASE ??
    DEFAULT_BINARIES_BASE;
const WASM_BUILDS_BASE = `${BINARIES_BASE}/wasm`;
const HASH_RE = /^[0-9a-f]{7,40}$/i;
const COMPACT_REPLAY_RE = /^rhrec-([0-9a-f]{7,40})-/i;
const PRELOAD_FETCH_CONCURRENCY = 12;

const logEl = document.querySelector<HTMLDivElement>('#log');
if (logEl === null) {
    throw new Error('main.ts: missing #log element in index.html');
}
const shareReplayButton = document.querySelector<HTMLButtonElement>('#share-replay');
const fullscreenButton = document.querySelector<HTMLButtonElement>('#fullscreen');
const replayTimeline = document.querySelector<HTMLDivElement>('#replay-timeline');

const logOk = (t: string): void => appendLogLine(logEl, t);
const logErr = (t: string): void => appendLogLine(logEl, t, 'err');

installConsoleMirror(logEl);
installFullscreenButton(fullscreenButton);

function installConsoleMirror(target: HTMLElement): void {
    const pendingLines: Array<{ text: string; cls?: 'err' }> = [];
    let flushScheduled = false;
    const flush = (): void => {
        flushScheduled = false;
        for (const { text, cls } of pendingLines.splice(0)) {
            appendLogLine(target, text, cls);
        }
    };
    const enqueue = (text: string, cls?: 'err'): void => {
        pendingLines.push(cls === undefined ? { text } : { text, cls });
        if (!flushScheduled) {
            flushScheduled = true;
            requestAnimationFrame(flush);
        }
    };

    const methods = ['log', 'info', 'warn', 'error'] as const;
    for (const method of methods) {
        const original = console[method].bind(console);
        console[method] = (...args: unknown[]): void => {
            original(...args);
            const line = formatConsoleArgs(args);
            enqueue(line, method === 'error' ? 'err' : undefined);
        };
    }
}

function formatConsoleArgs(args: readonly unknown[]): string {
    const [first, ...rest] = args;
    if (typeof first === 'string' && first.includes('%c')) {
        const styleArgCount = first.match(/%c/g)?.length ?? 0;
        const message = first.replaceAll('%c', '');
        const remaining = rest.slice(styleArgCount);
        return [message, ...remaining].map(formatConsoleArg).join(' ');
    }
    return args.map(formatConsoleArg).join(' ');
}

function formatConsoleArg(arg: unknown): string {
    if (typeof arg === 'string') {
        return arg;
    }
    if (arg instanceof Error) {
        return arg.message;
    }
    try {
        return JSON.stringify(arg);
    } catch {
        return String(arg);
    }
}

function installFullscreenButton(button: HTMLButtonElement | null): void {
    if (button === null) {
        return;
    }
    const canvas = document.querySelector<HTMLCanvasElement>('#canvas');
    button.addEventListener('click', () => {
        void (async (): Promise<void> => {
            try {
                if (document.fullscreenElement !== null) {
                    await document.exitFullscreen();
                } else {
                    await (canvas ?? document.documentElement).requestFullscreen();
                }
            } catch (e) {
                console.error('fullscreen failed:', e);
            }
        })();
    });
    document.addEventListener('fullscreenchange', () => {
        const active = document.fullscreenElement !== null;
        button.textContent = active ? 'Exit fullscreen' : 'Fullscreen';
        button.title = active ? 'Exit fullscreen' : 'Enter fullscreen';
    });
}

async function fetchJson(url: string): Promise<BuildManifest> {
    const resp = await fetch(url, { cache: 'no-cache' });
    if (!resp.ok) {
        throw new Error(`fetch ${url}: HTTP ${resp.status}`);
    }
    return await resp.json() as BuildManifest;
}

function replayBuildHash(replay: string): string {
    const compact = COMPACT_REPLAY_RE.exec(replay);
    if (compact !== null) {
        return compact[1] ?? '';
    }
    if (HASH_RE.test(replay)) {
        return replay;
    }
    throw new Error('replay= must be an rhrec compact replay or a git hash');
}

async function resolveBuild(): Promise<BuildSelection> {
    const wasmBase = pageParams.get('wasm-base') ?? pageParams.get('wasm_base');
    if (wasmBase !== null && wasmBase.length > 0) {
        return {
            short: 'local',
            source: 'latest',
            buildBase: new URL(wasmBase, window.location.href).toString().replace(/\/$/, ''),
        };
    }

    const replay = pageParams.get('replay');
    if (replay !== null && replay.length > 0) {
        const hash = replayBuildHash(replay);
        return { short: hash, source: 'replay' };
    }

    const latest = await fetchJson(`${WASM_BUILDS_BASE}/latest.json`);
    const short = String(latest.short ?? latest.commit ?? '');
    if (!HASH_RE.test(short)) {
        throw new Error(`latest.json did not contain a valid git hash: ${short}`);
    }
    return { short, source: 'latest' };
}

async function main(): Promise<void> {
    const build = await resolveBuild();
    const buildBase = build.buildBase ?? `${WASM_BUILDS_BASE}/${build.short}`;
    logOk(`[selected ${build.source} build ${build.short}]`);

    logOk('[loading wasm module]');
    const wasm = await loadWasmModule(buildBase, build.short !== 'local', build.source === 'latest');

    logOk('[wasm module ready, fetching datadir]');

    const dataUrl = `${BINARIES_BASE}/datadirs/demo-leicester/v6-web-opus-q80.rhdata.zst`;
    const resp = await fetch(dataUrl, {
        cache: build.source === 'latest' ? 'no-cache' : 'force-cache',
    });
    if (!resp.ok) {
        throw new Error(`fetch ${dataUrl}: HTTP ${resp.status}`);
    }
    const buf = await resp.arrayBuffer();
    logOk(`[datadir fetched: ${buf.byteLength} bytes]`);

    await preloadAssets(wasm, buildBase, build.source === 'latest');

    const rpc = installRpcClient(wasm);

    wasm.wasm_boot(new Uint8Array(buf), dataUrl.slice(0, dataUrl.lastIndexOf('/')));
    logOk('[handed off to Rust - winit drives rAF from here]');
    await waitForRpcBridge(rpc);
    if (shareReplayButton !== null) {
        installShareButton(shareReplayButton, rpc);
    }
    const replayLoaded = await applyReplayFromQuery(rpc);
    if (replayLoaded) {
        logOk('[replay queued from URL - start a mission to play it back]');
        if (replayTimeline !== null && !new URL(location.href).searchParams.has('notimeline')) {
            installTimeline(replayTimeline, rpc);
        }
    }
}

async function loadWasmModule(
    buildBase: string,
    preferPrecompressed: boolean,
    noCache: boolean,
): Promise<RobinWasmModule> {
    const jsUrl = `${buildBase}/robin.js`;
    const wasmUrl = `${buildBase}/robin_bg.wasm`;
    const cache: RequestCache = noCache ? 'no-cache' : 'force-cache';

    // GitHub Pages serves checked-in `.gz` files as opaque gzip downloads,
    // without a Content-Encoding header. Decompress those in the browser so
    // the large wasm module does not cross the network uncompressed. Local
    // development keeps the ordinary URL/import path and its useful source
    // identity instead of paying for two failed `.gz` probes on every boot.
    const jsBytes = preferPrecompressed
        ? await fetchPrecompressed(`${jsUrl}.gz`, cache)
        : undefined;
    let wasm: RobinWasmModule;
    if (jsBytes === undefined) {
        wasm = await import(/* @vite-ignore */ jsUrl) as RobinWasmModule;
    } else {
        const moduleUrl = URL.createObjectURL(new Blob([jsBytes], { type: 'text/javascript' }));
        try {
            wasm = await import(/* @vite-ignore */ moduleUrl) as RobinWasmModule;
        } finally {
            URL.revokeObjectURL(moduleUrl);
        }
    }

    const wasmResponse = preferPrecompressed
        ? await fetchPrecompressedWasm(`${wasmUrl}.gz`, cache)
        : undefined;
    await wasm.default({ module_or_path: wasmResponse ?? wasmUrl });
    return wasm;
}

async function fetchPrecompressedWasm(
    url: string,
    cache: RequestCache,
): Promise<Response | undefined> {
    if (typeof DecompressionStream === 'undefined') {
        return undefined;
    }
    const resp = await fetch(url, { cache });
    if (resp.status === 404) {
        return undefined;
    }
    if (!resp.ok) {
        throw new Error(`fetch ${url}: HTTP ${resp.status}`);
    }
    if (resp.body === null) {
        throw new Error(`fetch ${url}: response has no body`);
    }
    const body = resp.headers.get('Content-Encoding')?.toLowerCase().includes('gzip') === true
        ? resp.body
        : resp.body.pipeThrough(new DecompressionStream('gzip'));
    // A Response lets wasm-bindgen retain instantiateStreaming. Static hosts
    // generally label `.wasm.gz` as generic binary data, so provide the MIME
    // type WebAssembly.instantiateStreaming requires.
    return new Response(body, {
        headers: { 'Content-Type': 'application/wasm' },
    });
}

async function fetchPrecompressed(
    url: string,
    cache: RequestCache,
): Promise<ArrayBuffer | undefined> {
    if (typeof DecompressionStream === 'undefined') {
        return undefined;
    }
    const resp = await fetch(url, { cache });
    if (resp.status === 404) {
        return undefined;
    }
    if (!resp.ok) {
        throw new Error(`fetch ${url}: HTTP ${resp.status}`);
    }
    if (resp.body === null) {
        throw new Error(`fetch ${url}: response has no body`);
    }
    // A conventional host may recognize the suffix and attach
    // Content-Encoding itself. Fetch has already decoded such a response.
    if (resp.headers.get('Content-Encoding')?.toLowerCase().includes('gzip') === true) {
        return await resp.arrayBuffer();
    }
    const decompressed = resp.body.pipeThrough(new DecompressionStream('gzip'));
    return await new Response(decompressed).arrayBuffer();
}

function installRpcClient(wasm: RobinWasmModule): RobinRpc {
    if (wasm.rh_rpc === undefined) {
        throw new Error('wasm module does not export rh_rpc');
    }
    const rhRpc = wasm.rh_rpc;
    const rpc: RobinRpc = <T = unknown>(method: string, params: unknown = null): Promise<T> => {
        return rhRpc<T>({ method, params });
    };
    globalThis.robinRpc = rpc;
    return rpc;
}

async function waitForRpcBridge(rpc: RobinRpc): Promise<void> {
    await rpc('info');
}

async function preloadAssets(
    wasm: RobinWasmModule,
    buildBase: string,
    noCache: boolean,
): Promise<void> {
    if (wasm.wasm_preload_asset === undefined) {
        return;
    }
    const preloadAsset = wasm.wasm_preload_asset;
    const manifestUrl = `${buildBase}/preload-assets.json`;
    const manifestResp = await fetch(manifestUrl, {
        cache: noCache ? 'no-cache' : 'force-cache',
    });
    if (manifestResp.status === 404) {
        return;
    }
    if (!manifestResp.ok) {
        throw new Error(`fetch ${manifestUrl}: HTTP ${manifestResp.status}`);
    }
    const raw = await manifestResp.json() as unknown;
    if (!Array.isArray(raw)) {
        throw new Error(`${manifestUrl} must be a JSON array`);
    }
    const entries = (raw as PreloadEntry[]).map((entry, index) => {
        const path = typeof entry === 'string' ? entry : String(entry.path ?? '');
        const url = typeof entry === 'string' ? `${buildBase}/${entry}` : String(entry.url ?? path);
        if (path.length === 0 || url.length === 0) {
            throw new Error(`${manifestUrl} contains an invalid preload entry at index ${index}`);
        }
        const assetUrl = new URL(
            url,
            buildBase.endsWith('/') ? buildBase : `${buildBase}/`,
        ).toString();
        return { path, assetUrl };
    });

    // Fetch and install in the same bounded worker. Keeping every completed
    // ArrayBuffer until all requests finish doubles the preload peak: JS owns
    // all downloads while wasm_preload_asset copies them into Rust.
    await forEachConcurrent(
        entries,
        PRELOAD_FETCH_CONCURRENCY,
        async ({ path, assetUrl }) => {
            const assetResp = await fetch(assetUrl, {
                cache: noCache ? 'no-cache' : 'force-cache',
            });
            if (!assetResp.ok) {
                throw new Error(`fetch ${assetUrl}: HTTP ${assetResp.status}`);
            }
            const bytes = new Uint8Array(await assetResp.arrayBuffer());
            preloadAsset(path, bytes);
            logOk(`[preloaded ${path}: ${bytes.byteLength} bytes]`);
        },
    );
}

async function forEachConcurrent<T>(
    items: readonly T[],
    concurrency: number,
    action: (item: T, index: number) => Promise<void>,
): Promise<void> {
    if (!Number.isInteger(concurrency) || concurrency < 1) {
        throw new Error(`preload concurrency must be a positive integer, got ${concurrency}`);
    }
    const errors = new Array<Error | undefined>(items.length);
    let next = 0;
    const worker = async (): Promise<void> => {
        for (;;) {
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
    const firstError = errors.find((error): error is Error => error !== undefined);
    if (firstError !== undefined) {
        throw firstError;
    }
}

main().catch((e: unknown) => {
    const msg = e instanceof Error ? e.message : String(e);
    // eslint-disable-next-line no-console
    console.error(msg);
    logErr(`[boot failed] ${msg}`);
});
