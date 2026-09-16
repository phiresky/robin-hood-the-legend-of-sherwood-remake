import { fullGameContentBuild } from './mission-launch.ts';
import { installDiagnostics } from './diagnostics.js';
import { requestFullContentFolder } from './content-picker.js';
import { fetchWithProgress, fetchJson, fetchRuntimeWasm } from './boot-transport.js';
import { withAbort } from './cancellation.js';
import { installCanvasBackingStore } from './canvas-lifecycle.js';
import { preloadRuntimeAssets } from './asset-preload.js';
import { bootGame, loadRuntimeInParallel, onFirstRuntimeFrame, type BuildSelection, type BrowserJoinContext, type RobinWasmModule } from './boot-lifecycle.js';
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
    applyPreparedReplay, prepareReplayWithRuntime, replayFromQuery, replayRuntimeOverride, type PreparedReplay,
    installShareButton,
    validateReplayInWorker,
    type RobinRpc,
} from './replay.js';
import { installTimeline } from './timeline.js';
import { createRpcClient } from './rpc-client.js';
import { fetchRunReplay, parseHostedReplayContent, runFromQuery, runPlaybackBuild, type RunReplay } from './run-replay.js';

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

// Launch paths that present nothing for a long time (or a runtime that never
// announces its first frame) must not keep the overlay over the canvas forever.
const BOOT_OVERLAY_FALLBACK_MS = 10_000;

/** Keep the HTML overlay up across the handoff until the runtime's first
 * presented frame (normally the mission loading screen) replaces it, so the
 * page never shows a blank canvas. */
function removeBootProgressOnFirstFrame(): void {
    onFirstRuntimeFrame(window, BOOT_OVERLAY_FALLBACK_MS, outcome => {
        // A boot failure after handoff keeps its error visible.
        if (bpRoot?.classList.contains('bp-error') === true) return;
        logOk(outcome === 'presented'
            ? '[boot overlay removed: first runtime frame presented]'
            : `[boot overlay removed: no runtime frame after ${BOOT_OVERLAY_FALLBACK_MS} ms]`);
        // The frame reaches the compositor at the next rendering opportunity.
        requestAnimationFrame(() => bootProgressDone());
    });
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
// The leaderboard API is the same-origin `/api` route in production.
const RUN_API_BASE = import.meta.env.DEV
    ? pageParams.get('api') ?? `${window.location.origin}/api/v1`
    : `${window.location.origin}/api/v1`;
const runQuery = runFromQuery(pageParams);

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
let disposeTimeline: (() => void) | undefined;
window.addEventListener('pagehide', event => {
    if (!event.persisted) {
        disposeCanvasBackingStore();
        disposeTimeline?.();
    }
});

const logOk = (t: string): void => appendLogLine(logEl, t);
const logErr = (t: string): void => appendLogLine(logEl, t, 'err');

const diagnostics = installDiagnostics();
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
            diagnostics.log(line);
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

async function resolveBuild(
    ticket: VerifiedBrowserJoinTicket | undefined,
    run: RunReplay | null,
    signal: AbortSignal,
): Promise<BuildSelection> {
    if (ticket !== undefined) {
        return { short: ticket.payload.engine_version.slice(0, 12), source: 'multiplayer' };
    }
    // Only reviewed host fixes may replace a run's recorded simulation build.
    if (run !== null) return { short: runPlaybackBuild(run.runtimeBuild), source: 'replay' };
    const wasmBase = pageParams.get('wasm-base') ?? pageParams.get('wasm_base');
    if (wasmBase !== null && wasmBase.length > 0) {
        return {
            short: 'local',
            source: 'latest',
            buildBase: new URL(wasmBase, window.location.href).toString().replace(/\/$/, ''),
        };
    }

    // A compact recording's hash is provenance: it plays on the latest runtime.
    const replayOverride = replayRuntimeOverride(pageParams.get('replay'));
    if (replayOverride !== undefined) {
        return { short: replayOverride, source: 'replay' };
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

// The engine decodes exactly one native datadir format. Published builds pin
// their Demo generation in manifest.json, so replays of older builds keep
// loading the retained older datadir. `?wasm-base=` development builds have no
// manifest and use the current generation.
const CURRENT_DEMO_DATADIR_PATH = '/datadirs/demo-leicester/v18/v18-web-opus-q80.rhdata.zst';
const PUBLISHED_DEMO_ORIGIN = 'https://robinhood.phiresky.xyz';

type DemoDatadirIdentity = { readonly sha256: string; readonly byteLength: number };

async function selectedDemoDatadir(
    base: string,
    build: BuildSelection,
    signal: AbortSignal,
): Promise<{ readonly url: string; readonly identity?: DemoDatadirIdentity }> {
    if (build.short === 'local') return { url: `${BINARIES_BASE}${CURRENT_DEMO_DATADIR_PATH}` };
    const manifest = await fetchJson<{ readonly multiplayerContent?: { readonly demo?: Record<string, unknown> } }>(
        `${base}/manifest.json`, signal,
    );
    const demo = manifest.multiplayerContent?.demo;
    const url = demo?.url;
    const sha256 = demo?.sha256;
    const byteLength = demo?.byteLength;
    if (typeof url !== 'string' || !url.startsWith(`${PUBLISHED_DEMO_ORIGIN}/datadirs/`)
        || typeof sha256 !== 'string' || !/^[0-9a-f]{64}$/.test(sha256)
        || typeof byteLength !== 'number' || !Number.isSafeInteger(byteLength)) {
        throw new Error(`build ${build.short} manifest does not pin a same-origin Demo datadir`);
    }
    return { url: `${BINARIES_BASE}${url.slice(PUBLISHED_DEMO_ORIGIN.length)}`, identity: { sha256, byteLength } };
}

async function selectedFullReplayDatadir(
    recordedBuild: string, signal: AbortSignal,
): Promise<{ readonly url: string; readonly identity: DemoDatadirIdentity; readonly parts: readonly string[] }> {
    const content = parseHostedReplayContent(await fetchJson<unknown>(
        `${BINARIES_BASE}/datadirs/replays/v2/${recordedBuild}.json`, signal,
    ), BINARIES_BASE);
    return { url: content.url, identity: content, parts: content.parts };
}

async function verifyDatadir(
    datadir: Uint8Array<ArrayBuffer>,
    identity: DemoDatadirIdentity,
    signal: AbortSignal,
): Promise<void> {
    const digest = new Uint8Array(await withAbort(signal, () => crypto.subtle.digest('SHA-256', datadir)));
    const hex = Array.from(digest, byte => byte.toString(16).padStart(2, '0')).join('');
    if (datadir.byteLength !== identity.byteLength || hex !== identity.sha256) {
        throw new Error('Game data does not match the selected replay or build');
    }
}

const bootAbort = new AbortController();
window.addEventListener('pagehide', event => {
    if (!event.persisted) bootAbort.abort();
});

async function main(): Promise<void> {
    if (pageParams.get('embed') === '1') document.documentElement.classList.add('embedded-replay');
    if (runQuery !== null && (pageParams.has('replay') || capturedBrowserJoinCode !== undefined)) {
        throw new Error('run= cannot be combined with replay= or a multiplayer invitation');
    }
    const runReplay = runQuery === null
        ? null
        : await fetchRunReplay(runQuery, RUN_API_BASE, fetch, bootAbort.signal);
    if (runReplay !== null) {
        logOk(`[leaderboard run ${runReplay.runId} recorded by build ${runReplay.runtimeBuild}]`);
        // Archived runtimes recognize this launch option before their main menu starts.
        const launchUrl = new URL(location.href);
        launchUrl.searchParams.set('wait-for-command', 'true');
        history.replaceState(history.state, '', launchUrl);
    }
    const replayQuery = runReplay === null
        ? replayFromQuery(pageParams)
        : { content: runReplay.content, paused: true };
    let preparedReplay: PreparedReplay | null = null;
    logOk(crossOriginIsolated
        ? `[cross-origin isolated: sprite decode may use ${navigator.hardwareConcurrency} threads]`
        : '[not cross-origin isolated: sprite decode stays single-threaded]');
    await bootGame({
        buildsBase: WASM_BUILDS_BASE,
        prepareJoin: signal => prepareBrowserJoin(capturedBrowserJoinCode, signal),
        resolveBuild: async (ticket, signal) => {
            const selection = await resolveBuild(ticket, runReplay, signal);
            diagnostics.setBuild(selection.short);
            return selection;
        },
        loadManifest: async (base, ticket, signal) => parseMultiplayerBuildManifest(await fetchJson(`${base}/manifest.json`, signal), ticket),
        loadRuntime: async (base, compressed, latest, signal) => {
            const prepared = await prepareReplayWithRuntime(
                replayQuery, base,
                runtimeSignal => loadWasmModule(base, compressed, latest, runtimeSignal),
                async (content, admissionSignal) => {
                    performance.mark('robin-replay-admission-start');
                    await validateReplayInWorker(content, `${base}/replay_admission.js`, `${base}/replay_admission_bg.wasm`, admissionSignal);
                    admissionSignal.throwIfAborted();
                    performance.mark('robin-replay-admission-accepted');
                }, signal,
            );
            preparedReplay = prepared.replay;
            return prepared.runtime;
        },
        prepareContent: (ticket, manifest, signal) => prepareMultiplayerContent(ticket, manifest, requestFullContentFolder, signal),
        loadDefaultContent: async (base, build, signal) => {
            const fullBuild = fullGameContentBuild(build.short, pageParams.get('edition'), runReplay, replayQuery !== null);
            const demo: { url: string; identity?: DemoDatadirIdentity; parts?: readonly string[] } = fullBuild !== null
                ? await selectedFullReplayDatadir(fullBuild, signal)
                : await selectedDemoDatadir(base, build, signal);
            const urls = demo.parts ?? [demo.url];
            const chunks: Uint8Array<ArrayBuffer>[] = [];
            let completed = 0;
            for (const url of urls) {
                const response = await fetchWithProgress(
                    url, build.source === 'latest' ? 'no-cache' : 'force-cache', 'application/zstd',
                    (loaded, total) => {
                        const expected = demo.identity?.byteLength ?? total;
                        bootProgress('gamedata', 'loading game data…',
                            expected > 0 ? (completed + loaded) / expected : 0,
                            progressDetail(completed + loaded, expected));
                    }, signal,
                );
                const chunk = new Uint8Array(await withAbort(signal, () => response.arrayBuffer()));
                chunks.push(chunk);
                completed += chunk.byteLength;
            }
            const datadir = new Uint8Array(completed);
            let offset = 0;
            for (const chunk of chunks) { datadir.set(chunk, offset); offset += chunk.byteLength; }
            if (demo.identity !== undefined) await verifyDatadir(datadir, demo.identity, signal);
            return { datadir, dataBaseUrl: demo.url.slice(0, demo.url.lastIndexOf('/')) };
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
            bootProgress('boot', 'starting game…', 1);
            removeBootProgressOnFirstFrame();
        },
        installReplay: async (rpc, wasm, buildBase) => {
            if (shareReplayButton !== null) {
                installShareButton(shareReplayButton, rpc);
            }
            const replayLoaded = await applyPreparedReplay(rpc, content => {
                if (wasm.wasm_mark_compact_replay_validated === undefined) {
                    throw new Error('selected wasm build cannot accept an isolated replay proof');
                }
                wasm.wasm_mark_compact_replay_validated(content);
            }, preparedReplay, buildBase);
            bootAbort.signal.throwIfAborted();
            if (replayLoaded) {
                performance.mark('robin-replay-queue-accepted');
                logOk('[replay queued from URL]');
                if (replayTimeline !== null && !new URL(location.href).searchParams.has('notimeline')) {
                    disposeTimeline?.();
                    disposeTimeline = installTimeline(replayTimeline, rpc);
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
    disposeTimeline?.();
    disposeTimeline = undefined;
    const rpc = createRpcClient(wasm);
    globalThis.robinRpc = rpc;
    return rpc;
}


main().catch((e: unknown) => {
    const msg = e instanceof Error ? e.message : String(e);
    // eslint-disable-next-line no-console
    console.error(msg);
    diagnostics.failure(e);
    bootProgressError(`boot failed: ${msg}`);
    logErr(`[boot failed] ${msg}`);
});
