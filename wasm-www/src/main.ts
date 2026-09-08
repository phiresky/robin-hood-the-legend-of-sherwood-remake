import { requestFullContentFolder } from './content-picker.js';
import { fetchWithProgress, fetchJson, fetchRuntimeWasm } from './boot-transport.js';
import { withAbort } from './cancellation.js';
import { installCanvasBackingStore } from './canvas-lifecycle.js';
import { preloadRuntimeAssets } from './asset-preload.js';
import { bootGame, loadRuntimeInParallel, type BuildSelection, type BrowserJoinContext, type RobinWasmModule } from './boot-lifecycle.js';
import { appendLogLine, appendLogLines } from './log.js';
import {
    authenticateBrowserJoinTicket,
    captureAndScrubBrowserJoinCode,
    validateBrowserJoinTicketUse,
    type VerifiedBrowserJoinTicket,
} from './join_ticket.js';
import {
    parseMultiplayerBuildManifest,
    prepareMultiplayerContent,
    type PreparedMultiplayerContent,
} from './multiplayer_content.js';
import {
    installBrowserMultiplayerIdentity,
    wasInvitationRedeemed,
} from './multiplayer_identity.js';
import {
    applyReplayFromQuery,
    installShareButton,
    validateReplayInWorker,
    type RobinRpc,
} from './replay.js';
import { installTimeline } from './timeline.js';

declare global {
    // Optional test/dev override for loading binaries from a local checkout.
    // Keep this global so the deployed HTML can stay config-free.
    var ROBIN_WASM_BINARIES_BASE: string | undefined;
    var robinRpc: ((method: string, params?: unknown) => Promise<unknown>) | undefined;
}

// --- boot progress overlay -------------------------------------------------
// One weighted bar from page load until the game's own canvas rendering
// takes over. Weights approximate the byte/time split of a cold load; byte
// progress interpolates inside each phase.
const BOOT_PHASES = [
    ['engine-js', 2],
    ['engine', 48],
    ['engine-start', 6],
    ['assets', 4],
    ['gamedata', 35],
    ['boot', 5],
] as const;
type BootPhase = (typeof BOOT_PHASES)[number][0];
const bpRoot = document.getElementById('boot-progress');
const bpFill = document.getElementById('bp-fill');
const bpLabel = document.getElementById('bp-label');
const bpDetail = document.getElementById('bp-detail');
const bootPhaseProgress = new Map<BootPhase, number>();

function bootProgress(phase: BootPhase, label: string, frac: number, detail = ''): void {
    if (bpFill === null || bpLabel === null || bpDetail === null) {
        return;
    }
    // Downloads overlap, so a later phase must not imply earlier ones finished.
    const clamped = Math.min(Math.max(frac, 0), 1);
    bootPhaseProgress.set(phase, Math.max(bootPhaseProgress.get(phase) ?? 0, clamped));
    let completed = 0;
    let total = 0;
    for (const [name, weight] of BOOT_PHASES) {
        completed += weight * (bootPhaseProgress.get(name) ?? 0);
        total += weight;
    }
    bpFill.style.width = `${((completed / total) * 100).toFixed(1)}%`;
    bpLabel.textContent = label;
    bpDetail.textContent = detail;
}

function bootProgressDone(): void {
    bpRoot?.remove();
}

function bootProgressError(message: string): void {
    bpRoot?.classList.add('bp-error');
    if (bpLabel !== null) {
        bpLabel.textContent = message;
    }
}

const progressMb = (n: number): string => `${(n / 1e6).toFixed(1)} MB`;
const progressDetail = (loaded: number, total: number): string =>
    total > 0 ? `${progressMb(loaded)} / ${progressMb(total)}` : progressMb(loaded);

type BuildManifest = {
    readonly commit?: unknown;
    readonly short?: unknown;
};

const pageParams = new URLSearchParams(window.location.search);
// Capture and erase the ticket before `main` can issue its first artifact or
// content request. Fragment data never reaches the origin server; replacing
// this history entry prevents later copy/paste and browser-history exposure.
const capturedBrowserJoinCode = captureAndScrubBrowserJoinCode(new URL(window.location.href));
const binariesBaseOverride =
    pageParams.get('binaries-base') ??
    pageParams.get('binaries_base') ??
    globalThis.ROBIN_WASM_BINARIES_BASE;
if (!import.meta.env.DEV && binariesBaseOverride !== undefined && binariesBaseOverride !== null) {
    throw new Error('Production engine and data assets are fixed to the same Cloudflare origin.');
}
const BINARIES_BASE = import.meta.env.DEV
    ? binariesBaseOverride ?? window.location.origin
    : window.location.origin;
const WASM_BUILDS_BASE = `${BINARIES_BASE}/wasm`;
const HASH_RE = /^[0-9a-f]{7,40}$/i;
const COMPACT_REPLAY_RE = /^rhrec-([0-9a-f]{7,40})-/i;

const logEl = document.querySelector<HTMLDivElement>('#log');
if (logEl === null) {
    throw new Error('main.ts: missing #log element in index.html');
}
const shareReplayButton = document.querySelector<HTMLButtonElement>('#share-replay');
const fullscreenButton = document.querySelector<HTMLButtonElement>('#fullscreen');
const replayTimeline = document.querySelector<HTMLDivElement>('#replay-timeline');
const gameCanvas = document.querySelector<HTMLCanvasElement>('#canvas');
if (gameCanvas === null) {
    throw new Error('main.ts: missing #canvas element in index.html');
}
const { sync: syncCanvasBackingStore, dispose: disposeCanvasBackingStore } = installCanvasBackingStore(gameCanvas);
window.addEventListener('pagehide', event => {
    if (!event.persisted) disposeCanvasBackingStore();
});

const logOk = (t: string): void => appendLogLine(logEl, t);
const logErr = (t: string): void => appendLogLine(logEl, t, 'err');

installConsoleMirror(logEl);
installFullscreenButton(fullscreenButton);

function installConsoleMirror(target: HTMLElement): void {
    const pendingLines: Array<{ text: string; cls?: 'err' }> = [];
    let flushScheduled = false;
    const flush = (): void => {
        flushScheduled = false;
        appendLogLines(target, pendingLines.splice(0));
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

async function resolveBuild(ticket: VerifiedBrowserJoinTicket | undefined, signal: AbortSignal): Promise<BuildSelection> {
    if (ticket !== undefined) {
        return { short: ticket.payload.engine_version.slice(0, 12), source: 'multiplayer' };
    }
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

    const latest = await fetchJson<BuildManifest>(`${WASM_BUILDS_BASE}/latest.json`, signal);
    const short = String(latest.short ?? latest.commit ?? '');
    if (!HASH_RE.test(short)) {
        throw new Error(`latest.json did not contain a valid git hash: ${short}`);
    }
    return { short, source: 'latest' };
}

async function prepareBrowserJoin(code: string | undefined, signal: AbortSignal): Promise<BrowserJoinContext | undefined> {
    if (code === undefined) return undefined;
    const ticket = await withAbort(signal, () => authenticateBrowserJoinTicket(code));
    const redeemed = await withAbort(signal, () => wasInvitationRedeemed(ticket.payload.session_id));
    validateBrowserJoinTicketUse(ticket, Math.floor(Date.now() / 1000), redeemed);
    // The non-extractable durable signer must exist before the wasm relay
    // client can prove seat ownership. Storage/WebCrypto failure is fatal.
    await withAbort(signal, () => installBrowserMultiplayerIdentity());
    return { ticket, redeemed };
}

function preloadLocalAssets(
    wasm: RobinWasmModule,
    assets: PreparedMultiplayerContent['assets'],
): void {
    if (assets.length === 0) return;
    if (wasm.wasm_preload_asset === undefined) {
        throw new Error('selected browser artifact cannot preload authenticated Full assets');
    }
    for (const asset of assets) wasm.wasm_preload_asset(asset.path, asset.bytes);
}

function preloadLocalShippingFiles(
    wasm: RobinWasmModule,
    files: PreparedMultiplayerContent['shippingFiles'],
): void {
    if (files.length === 0) return;
    if (wasm.wasm_preload_shipping_file === undefined) {
        throw new Error('selected browser artifact cannot preload authenticated Full mission data');
    }
    // wasm_boot installs the datadir synchronously and only then schedules
    // the game future. Do not await/yield until every verified split file is
    // in Rust's cache, so an accidental network fallback is impossible.
    for (const file of files) wasm.wasm_preload_shipping_file(file.path, file.bytes);
}

const bootAbort = new AbortController();
window.addEventListener('pagehide', event => {
    if (!event.persisted) bootAbort.abort();
});

async function main(): Promise<void> {
    logOk(crossOriginIsolated
        ? `[cross-origin isolated: sprite decode may use ${navigator.hardwareConcurrency} threads]`
        : '[not cross-origin isolated: sprite decode stays single-threaded]');
    await bootGame({
        buildsBase: WASM_BUILDS_BASE,
        prepareJoin: signal => prepareBrowserJoin(capturedBrowserJoinCode, signal),
        resolveBuild,
        loadManifest: async (base, ticket, signal) => parseMultiplayerBuildManifest(await fetchJson(`${base}/manifest.json`, signal), ticket),
        loadRuntime: loadWasmModule,
        prepareContent: (ticket, manifest, signal) => prepareMultiplayerContent(ticket, manifest, requestFullContentFolder, signal),
        loadDefaultContent: async (latest, signal) => {
            const dataUrl = `${BINARIES_BASE}/datadirs/demo-leicester/v8-web-opus-q80.rhdata.zst`;
            const response = await fetchWithProgress(
                dataUrl, latest ? 'no-cache' : 'force-cache', 'application/zstd',
                (loaded, total) => bootProgress('gamedata', 'loading game data…',
                    total > 0 ? loaded / total : 0, progressDetail(loaded, total)),
                signal,
            );
            return { datadir: new Uint8Array(await withAbort(signal, () => response.arrayBuffer())), dataBaseUrl: dataUrl.slice(0, dataUrl.lastIndexOf('/')) };
        },
        preloadLocalAssets,
        preloadShippingFiles: preloadLocalShippingFiles,
        preloadAssets: (wasm, base, latest, signal) => preloadRuntimeAssets(wasm, base, latest, {
            signal,
            fetch,
            log: logOk,
            progress: (fraction, detail) => bootProgress('assets', 'loading interface assets…', fraction, detail),
        }),
        installRpc: installRpcClient,
        runtimeStarted: () => {
            // winit may reset the backing store on attachment. Restore it after that turn.
            requestAnimationFrame(() => requestAnimationFrame(syncCanvasBackingStore));
            logOk('[handed off to Rust - winit drives rAF from here]');
            requestAnimationFrame(() => bootProgressDone());
        },
        installReplay: async (rpc, wasm, buildBase) => {
            if (shareReplayButton !== null) {
                installShareButton(shareReplayButton, rpc);
            }
            const replayLoaded = await applyReplayFromQuery(rpc, {
                validate: async (content): Promise<void> => {
                    await validateReplayInWorker(
                        content,
                        `${buildBase}/replay_admission.js`,
                        `${buildBase}/replay_admission_bg.wasm`,
                        bootAbort.signal,
                    );
                },
                markValidated: (content): void => {
                    if (wasm.wasm_mark_compact_replay_validated === undefined) {
                        throw new Error('selected wasm build cannot accept an isolated replay proof');
                    }
                    wasm.wasm_mark_compact_replay_validated(content);
                },
            });
            if (replayLoaded) {
                logOk('[replay queued from URL - start a mission to play it back]');
                if (replayTimeline !== null && !new URL(location.href).searchParams.has('notimeline')) {
                    installTimeline(replayTimeline, rpc);
                }
            }
        },
        progress: bootProgress,
        log: logOk,
    }, bootAbort.signal);
}

async function loadWasmModule(
    buildBase: string,
    preferPrecompressed: boolean,
    noCache: boolean,
    signal: AbortSignal,
): Promise<RobinWasmModule> {
    const jsUrl = `${buildBase}/robin.js`;
    const wasmUrl = `${buildBase}/robin_bg.wasm`;
    const cache: RequestCache = noCache ? 'no-cache' : 'force-cache';

    // The JS glue is always imported from its real URL — never through a
    // decompressed-blob module. Worker-pool builds statically import their
    // `snippets/` worker helper relative to the module URL, and each Web
    // Worker re-imports the glue by that same URL; a blob: module would
    // break both. Static Assets serves JavaScript with ordinary compression,
    // so nothing is lost by skipping the precompressed `.gz` sibling.

    const onWasmBytes = (loaded: number, total: number): void => {
        bootProgress(
            'engine',
            'loading engine…',
            total > 0 ? loaded / total : 0,
            progressDetail(loaded, total),
        );
    };
    // Keep native HTTP compression and optional precompressed sidecars
    // streaming into compilation; local builds use the ordinary URL directly.
    const wasm = await loadRuntimeInParallel(
        async loadingSignal => {
            const module = await withAbort(loadingSignal, () => import(/* @vite-ignore */ jsUrl)) as RobinWasmModule;
            loadingSignal.throwIfAborted();
            bootProgress('engine-js', 'loading engine…', 1);
            return module;
        },
        loadingSignal => fetchRuntimeWasm(wasmUrl, preferPrecompressed, cache, onWasmBytes, loadingSignal),
        signal,
    );
    signal.throwIfAborted();
    bootProgress('engine-start', 'engine ready', 1);
    return wasm;
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


main().catch((e: unknown) => {
    const msg = e instanceof Error ? e.message : String(e);
    // eslint-disable-next-line no-console
    console.error(msg);
    bootProgressError(`boot failed: ${msg}`);
    logErr(`[boot failed] ${msg}`);
});
