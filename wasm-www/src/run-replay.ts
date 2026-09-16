// `?run=<run id>`: play a verified leaderboard run on a compatible engine build.
// RunDetailV2.viewer.runtime_build is the replay's recorded engine
// version, the 12-hex ROBIN_GIT_HASH prefix that also names `/wasm/<short>/`.

export const RUN_QUERY_KEY = 'run';
export const RANKED_REPLAY_MEDIA_TYPE = 'application/x-robin-rhrec';
const LEGACY_REPLAY_MEDIA_TYPE = `${RANKED_REPLAY_MEDIA_TYPE}+compact`;
const RUNTIME_BUILD_RE = /^[0-9a-f]{12}$/u;
const SHA256_RE = /^[0-9a-f]{64}$/u;
const MAX_RUN_JSON_BYTES = 2 * 1024 * 1024;
const MAX_REPLAY_BYTES = 32 * 1024 * 1024;

/** Host-only fixes for archived runtimes; simulation and replay schemas stay identical. */
export function runPlaybackBuild(recordedBuild: string): string {
    // Interactive checkpoint restoration and background sidecar loading.
    if (recordedBuild === 'e7557179eb05') return 'f5c6e859973f';
    // Save isolation, viewer camera/audio controls, and replay speech handling.
    return recordedBuild === '1699bc12ffb8' ? '1f546fbb6547' : recordedBuild;
}

export type RunReplay = {
    readonly runId: string;
    readonly runtimeBuild: string;
    readonly edition: 'demo' | 'full';
    /** Exact binary replay artifact. */
    readonly content: Uint8Array;
    legacyViewer?: true;
};

export function runFromQuery(params: URLSearchParams): string | null {
    const value = params.get(RUN_QUERY_KEY);
    if (value === null || value.length === 0) return null;
    if (new TextEncoder().encode(value).byteLength > 128 || value.trim() !== value
        || /\p{Cc}|\p{Bidi_Control}/u.test(value)) {
        throw new Error('run= is not a valid run id');
    }
    return value;
}

type Launch = { readonly edition: 'demo' | 'full'; readonly runtimeBuild: string; readonly sha256: string; readonly byteLength: number; readonly mediaType: string };

/** Minimal checks of the RunDetailV2 fields playback needs. */
export function parseRunLaunch(document: unknown, runId: string): Launch {
    const run = record(document, 'run');
    if (run.schema_version !== 2 || run.run_id !== runId) {
        throw new Error(`run ${runId}: the server returned a different or unsupported run document`);
    }
    const viewer = record(run.viewer, 'run.viewer');
    const availability = record(viewer.availability, 'run.viewer.availability');
    if (availability.status !== 'available') {
        const reason = typeof availability.safe_reason === 'string' ? availability.safe_reason : 'unavailable';
        throw new Error(`run ${runId} cannot be watched: ${reason}`);
    }
    if (viewer.content_requirement !== 'bundled_demo' && viewer.content_requirement !== 'user_local_retail') {
        throw new Error(`run ${runId} has an unsupported content requirement`);
    }
    const edition = viewer.content_requirement === 'bundled_demo' ? 'demo' : 'full';
    const runtimeBuild = viewer.runtime_build;
    if (typeof runtimeBuild !== 'string' || !RUNTIME_BUILD_RE.test(runtimeBuild)
        || run.recorded_engine_version !== runtimeBuild) {
        throw new Error(`run ${runId} does not name a published engine build`);
    }
    const artifact = record(record(run.replay, 'run.replay').artifact, 'run.replay.artifact');
    if (typeof artifact.sha256 !== 'string' || !SHA256_RE.test(artifact.sha256)
        || typeof artifact.byte_length !== 'number' || !Number.isSafeInteger(artifact.byte_length)
        || artifact.byte_length <= 0 || artifact.byte_length > MAX_REPLAY_BYTES
        || (artifact.media_type !== RANKED_REPLAY_MEDIA_TYPE && artifact.media_type !== LEGACY_REPLAY_MEDIA_TYPE)) {
        throw new Error(`run ${runId} has an invalid replay artifact`);
    }
    return { edition, runtimeBuild, sha256: artifact.sha256, byteLength: artifact.byte_length, mediaType: artifact.media_type };
}

export async function fetchRunReplay(
    runId: string,
    apiBase: string,
    fetchImpl: typeof fetch,
    signal: AbortSignal,
): Promise<RunReplay> {
    const runUrl = `${apiBase.replace(/\/+$/u, '')}/runs/${encodeURIComponent(runId)}`;
    const detail = await get(fetchImpl, runUrl, 'application/json', MAX_RUN_JSON_BYTES, signal);
    let document: unknown;
    try {
        document = JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(detail));
    } catch {
        throw new Error(`run ${runId}: the server returned invalid JSON`);
    }
    const launch = parseRunLaunch(document, runId);
    const bytes = await get(fetchImpl, `${runUrl}/replay`, launch.mediaType, launch.byteLength, signal);
    const digest = new Uint8Array(await crypto.subtle.digest('SHA-256', bytes));
    const hex = Array.from(digest, byte => byte.toString(16).padStart(2, '0')).join('');
    if (bytes.byteLength !== launch.byteLength || hex !== launch.sha256) {
        throw new Error(`run ${runId}: the replay does not match its published identity`);
    }
    const legacy = launch.mediaType === LEGACY_REPLAY_MEDIA_TYPE;
    const header = new TextEncoder().encode(legacy ? `rhrec-${launch.runtimeBuild}-` : `RHREC\x01${launch.runtimeBuild}`);
    if (bytes.length <= header.length || !header.every((byte, index) => bytes[index] === byte)) {
        throw new Error(`run ${runId}: the replay was not recorded by engine build ${launch.runtimeBuild}`);
    }
    if (legacy) return { runId, runtimeBuild: launch.runtimeBuild, edition: launch.edition, content: bytes, legacyViewer: true };
    return { runId, runtimeBuild: launch.runtimeBuild, edition: launch.edition, content: bytes };
}

export async function fetchRunCheckpoints(runId: string, apiBase: string, fetchImpl: typeof fetch, signal: AbortSignal): Promise<Uint8Array> {
    return get(fetchImpl, `${apiBase.replace(/\/+$/u, '')}/runs/${encodeURIComponent(runId)}/checkpoints`, 'application/x-robin-rhseek', 64 * 1024 * 1024, signal, true);
}

async function get(
    fetchImpl: typeof fetch,
    url: string,
    mediaType: string,
    limit: number,
    signal: AbortSignal,
    allowMissing = false,
): Promise<Uint8Array<ArrayBuffer>> {
    const response = await fetchImpl(url, {
        headers: { accept: mediaType },
        credentials: 'omit',
        cache: 'no-store',
        redirect: 'error',
        referrerPolicy: 'no-referrer',
        signal,
    });
    if (allowMissing && response.status === 404) return new Uint8Array();
    if (!response.ok) throw new Error(`${url} returned HTTP ${response.status}`);
    if (!(response.headers.get('content-type') ?? '').startsWith(mediaType)) {
        throw new Error(`${url} did not return ${mediaType}`);
    }
    const declared = response.headers.get('content-length');
    if (declared !== null && Number(declared) > limit) throw new Error(`${url} exceeds ${limit} bytes`);
    const bytes = new Uint8Array(await response.arrayBuffer());
    if (bytes.byteLength > limit) throw new Error(`${url} exceeds ${limit} bytes`);
    return bytes;
}

function record(value: unknown, label: string): Record<string, unknown> {
    if (typeof value !== 'object' || value === null || Array.isArray(value)) {
        throw new Error(`${label} must be an object`);
    }
    return value as Record<string, unknown>;
}


/** Resolve a published, build-specific Full replay content binding. */
export function parseHostedReplayContent(value: unknown, origin: string): {
    readonly url: string; readonly sha256: string; readonly byteLength: number; readonly parts: readonly string[];
} {
    const content = record(value, 'replay content');
    if (typeof content.url !== 'string'
        || !/^\/datadirs\/full\/[0-9a-f]{64}\/datadir\.bin$/u.test(content.url)
        || typeof content.sha256 !== 'string' || !SHA256_RE.test(content.sha256)
        || typeof content.byteLength !== 'number' || !Number.isSafeInteger(content.byteLength)
        || content.byteLength <= 0 || content.byteLength > 256 * 1024 * 1024) {
        throw new Error('Full replay data is not available for this player version');
    }
    const url = new URL(content.url, origin).toString();
    const parts = content.byteLength > 25 * 1024 * 1024
        ? Array.from({ length: Math.ceil(content.byteLength / (24 * 1024 * 1024)) }, (_, index) => `${url}.part${index}`)
        : [url];
    return { url, sha256: content.sha256, byteLength: content.byteLength, parts };
}
