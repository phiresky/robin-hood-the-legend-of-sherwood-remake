import { withAbort } from './cancellation.ts';
import type { VerifiedBrowserJoinTicket } from './join_ticket.js';
import type { MultiplayerBuildManifest, PreparedMultiplayerContent } from './multiplayer_content.js';
import type { RobinRpc } from './replay.js';

export type BuildSelection = {
    readonly short: string;
    readonly source: 'latest' | 'replay' | 'multiplayer';
    readonly buildBase?: string;
};

export type BrowserJoinContext = {
    readonly ticket: VerifiedBrowserJoinTicket;
    readonly redeemed: boolean;
};

export type RobinWasmModule = {
    readonly default: (init?: {
        module_or_path?: string | URL | Request | Response | ArrayBuffer;
    }) => Promise<unknown>;
    readonly wasm_boot: (datadir: Uint8Array, dataBaseUrl: string) => void;
    readonly wasm_multiplayer_compatibility?: () => unknown;
    readonly wasm_set_multiplayer_join_ticket?: (code: string, redeemed: boolean) => void;
    readonly wasm_preload_asset?: (path: string, bytes: Uint8Array) => void;
    readonly wasm_preload_shipping_file?: (path: string, bytes: Uint8Array) => void;
    readonly wasm_mark_compact_replay_validated?: (compact: string) => void;
    readonly rh_rpc?: <T = unknown>(request: { method: string; params: unknown }) => Promise<T>;
};


export type BootContent = { readonly datadir: Uint8Array; readonly dataBaseUrl: string };
export type BootDependencies = {
    readonly buildsBase: string;
    readonly prepareJoin: (signal: AbortSignal) => Promise<BrowserJoinContext | undefined>;
    readonly resolveBuild: (ticket: VerifiedBrowserJoinTicket | undefined, signal: AbortSignal) => Promise<BuildSelection>;
    readonly loadManifest: (base: string, ticket: VerifiedBrowserJoinTicket, signal: AbortSignal) => Promise<MultiplayerBuildManifest>;
    readonly loadRuntime: (base: string, compressed: boolean, latest: boolean, signal: AbortSignal) => Promise<RobinWasmModule>;
    readonly prepareContent: (ticket: VerifiedBrowserJoinTicket, manifest: MultiplayerBuildManifest, signal: AbortSignal) => Promise<PreparedMultiplayerContent>;
    readonly loadDefaultContent: (latest: boolean, signal: AbortSignal) => Promise<BootContent>;
    readonly preloadLocalAssets: (wasm: RobinWasmModule, assets: PreparedMultiplayerContent['assets']) => void;
    readonly preloadShippingFiles: (wasm: RobinWasmModule, files: PreparedMultiplayerContent['shippingFiles']) => void;
    readonly preloadAssets: (wasm: RobinWasmModule, base: string, latest: boolean, signal: AbortSignal) => Promise<void>;
    readonly installRpc: (wasm: RobinWasmModule) => RobinRpc;
    readonly runtimeStarted: () => void;
    readonly installReplay: (rpc: RobinRpc, wasm: RobinWasmModule, base: string, signal: AbortSignal) => Promise<void>;
    readonly progress: (phase: 'engine-js' | 'boot', label: string, fraction: number) => void;
    readonly log: (message: string) => void;
};

/** One boot attempt. All browser/transport effects are supplied by the shell.
 * No ticket, secret, or content bytes are included in diagnostic state.
 * Cancellation prevents subsequent side effects; adapters may additionally abort in-flight I/O.
 */
export async function bootGame(deps: BootDependencies, signal: AbortSignal): Promise<void> {
    signal.throwIfAborted();
    deps.progress('engine-js', 'loading engine…', 0);
    const join = await withAbort(signal, () => deps.prepareJoin(signal));
    signal.throwIfAborted();
    const build = await withAbort(signal, () => deps.resolveBuild(join?.ticket, signal));
    signal.throwIfAborted();
    const base = build.buildBase ?? `${deps.buildsBase}/${build.short}`;
    deps.log(`[selected ${build.source} build ${build.short}]`);
    const manifest = join === undefined ? undefined : await withAbort(signal, () => deps.loadManifest(base, join.ticket, signal));
    signal.throwIfAborted();
    if (join !== undefined) {
        deps.log(`[authenticated browser invitation via ${join.ticket.payload.relay_url}]`);
        deps.log('[privacy: the selected relay can observe IP addresses, timing, and byte counts; game traffic is end-to-end encrypted]');
    }
    deps.log('[loading wasm module]');
    const loadRuntime = (runtimeSignal: AbortSignal): Promise<RobinWasmModule> =>
        withAbort(runtimeSignal, () => deps.loadRuntime(base, build.short !== 'local', build.source === 'latest', runtimeSignal));
    let wasm: RobinWasmModule;
    let content: BootContent;
    let shippingFiles: PreparedMultiplayerContent['shippingFiles'] | undefined;
    if (join === undefined) {
        // Default content is independent of WASM. Start core asset preloads as
        // soon as WASM is ready, and cancel siblings if either branch fails.
        const failed = new AbortController();
        const loadingSignal = AbortSignal.any([signal, failed.signal]);
        try {
            [wasm, content] = await Promise.all([
                loadRuntime(loadingSignal).then(async loaded => {
                    loadingSignal.throwIfAborted();
                    deps.log('[wasm module ready, preloading assets]');
                    await withAbort(loadingSignal, () => deps.preloadAssets(loaded, base, build.source === 'latest', loadingSignal));
                    return loaded;
                }),
                withAbort(loadingSignal, () => deps.loadDefaultContent(build.source === 'latest', loadingSignal)),
            ]);
        } catch (error) {
            failed.abort(error);
            throw error;
        }
    } else {
        // Local content access stays behind authenticated runtime validation.
        wasm = await loadRuntime(signal);
        signal.throwIfAborted();
        if (manifest === undefined) throw new Error('multiplayer build manifest is missing');
        assertMultiplayerWasmCompatibility(wasm, join.ticket);
        if (wasm.wasm_set_multiplayer_join_ticket === undefined) {
            throw new Error('selected browser artifact has no multiplayer ticket entry point');
        }
        wasm.wasm_set_multiplayer_join_ticket(join.ticket.code, join.redeemed);
        const multiplayer = await withAbort(signal, () => deps.prepareContent(join.ticket, manifest, signal));
        signal.throwIfAborted();
        content = { datadir: multiplayer.datadir, dataBaseUrl: multiplayer.dataBaseUrl };
        shippingFiles = multiplayer.shippingFiles;
        deps.preloadLocalAssets(wasm, multiplayer.assets);
        await withAbort(signal, () => deps.preloadAssets(wasm, base, build.source === 'latest', signal));
    }
    signal.throwIfAborted();
    deps.log(`[datadir ready: ${content.datadir.byteLength} bytes]`);
    const rpc = deps.installRpc(wasm);
    deps.progress('boot', 'starting game…', 0.5);
    wasm.wasm_boot(content.datadir, content.dataBaseUrl);
    if (shippingFiles !== undefined) {
        deps.preloadShippingFiles(wasm, shippingFiles);
        shippingFiles = undefined;
    }
    deps.runtimeStarted();
    await withAbort(signal, () => rpc('info'));
    signal.throwIfAborted();
    await withAbort(signal, () => deps.installReplay(rpc, wasm, base, signal));
}

export function assertMultiplayerWasmCompatibility(
    wasm: RobinWasmModule,
    ticket: VerifiedBrowserJoinTicket,
): void {
    if (wasm.wasm_multiplayer_compatibility === undefined) {
        throw new Error('selected browser artifact does not export multiplayer compatibility data');
    }
    const raw = wasm.wasm_multiplayer_compatibility();
    if (raw === null || typeof raw !== 'object' || Array.isArray(raw)) {
        throw new Error('browser artifact returned malformed multiplayer compatibility data');
    }
    const object = raw as Record<string, unknown>;
    const keys = Object.keys(object);
    const expectedKeys = ['engineCommit', 'artifactShort', 'netProtocol', 'ticketSchema'];
    if (keys.length !== expectedKeys.length || keys.some((key, index) => key !== expectedKeys[index])) {
        throw new Error('browser artifact returned non-canonical multiplayer compatibility data');
    }
    if (
        object.engineCommit !== ticket.payload.engine_version
        || object.artifactShort !== ticket.payload.engine_version.slice(0, 12)
        || object.netProtocol !== ticket.payload.net_protocol
        || object.ticketSchema !== ticket.payload.schema
    ) {
        throw new Error('loaded browser artifact does not exactly match the host-signed invitation');
    }
}

/** Import URL-based worker glue while fetching the streaming WASM response. */
export async function loadRuntimeInParallel(
    importModule: (signal: AbortSignal) => Promise<RobinWasmModule>,
    fetchModule: (signal: AbortSignal) => Promise<Response>,
    signal: AbortSignal,
): Promise<RobinWasmModule> {
    const failed = new AbortController();
    const loadingSignal = AbortSignal.any([signal, failed.signal]);
    try {
        const [wasm, response] = await Promise.all([
            withAbort(loadingSignal, () => importModule(loadingSignal)),
            withAbort(loadingSignal, () => fetchModule(loadingSignal)),
        ]);
        loadingSignal.throwIfAborted();
        await withAbort(loadingSignal, () => wasm.default({ module_or_path: response }));
        return wasm;
    } catch (error) {
        failed.abort(error);
        throw error;
    }
}
