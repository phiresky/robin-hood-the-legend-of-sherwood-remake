// `?run=<run id>`: play a verified leaderboard run on the engine build that
// recorded it. RunDetailV2.viewer.runtime_build is the replay's recorded engine
// version, the 12-hex ROBIN_GIT_HASH prefix that also names `/wasm/<short>/`.

export const RUN_QUERY_KEY = 'run';
export const RANKED_REPLAY_MEDIA_TYPE = 'application/x-robin-rhrec+compact';
const RUNTIME_BUILD_RE = /^[0-9a-f]{12}$/u;
const SHA256_RE = /^[0-9a-f]{64}$/u;
const MAX_RUN_JSON_BYTES = 2 * 1024 * 1024;
const MAX_REPLAY_BYTES = 32 * 1024 * 1024;

export type RunReplay = {
    readonly runId: string;
    readonly runtimeBuild: string;
    /** Exact compact replay (`rhrec-<runtimeBuild>-...`). */
    readonly content: string;
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

type Launch = { readonly runtimeBuild: string; readonly sha256: string; readonly byteLength: number };

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
    if (viewer.content_requirement !== 'bundled_demo') {
        // TODO: support user_local_retail runs with a local Full content picker outside multiplayer joins.
        throw new Error(`run ${runId} needs a local Full installation, which browser replay playback does not support yet`);
    }
    const runtimeBuild = viewer.runtime_build;
    if (typeof runtimeBuild !== 'string' || !RUNTIME_BUILD_RE.test(runtimeBuild)
        || run.recorded_engine_version !== runtimeBuild) {
        throw new Error(`run ${runId} does not name a published engine build`);
    }
    const artifact = record(record(run.replay, 'run.replay').artifact, 'run.replay.artifact');
    if (typeof artifact.sha256 !== 'string' || !SHA256_RE.test(artifact.sha256)
        || typeof artifact.byte_length !== 'number' || !Number.isSafeInteger(artifact.byte_length)
        || artifact.byte_length <= 0 || artifact.byte_length > MAX_REPLAY_BYTES
        || artifact.media_type !== RANKED_REPLAY_MEDIA_TYPE) {
        throw new Error(`run ${runId} has an invalid replay artifact`);
    }
    return { runtimeBuild, sha256: artifact.sha256, byteLength: artifact.byte_length };
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
    const bytes = await get(fetchImpl, `${runUrl}/replay`, RANKED_REPLAY_MEDIA_TYPE, launch.byteLength, signal);
    const digest = new Uint8Array(await crypto.subtle.digest('SHA-256', bytes));
    const hex = Array.from(digest, byte => byte.toString(16).padStart(2, '0')).join('');
    if (bytes.byteLength !== launch.byteLength || hex !== launch.sha256) {
        throw new Error(`run ${runId}: the replay does not match its published identity`);
    }
    const content = new TextDecoder('utf-8', { fatal: true }).decode(bytes);
    if (!content.startsWith(`rhrec-${launch.runtimeBuild}-`)) {
        throw new Error(`run ${runId}: the replay was not recorded by engine build ${launch.runtimeBuild}`);
    }
    return { runId, runtimeBuild: launch.runtimeBuild, content };
}

async function get(
    fetchImpl: typeof fetch,
    url: string,
    mediaType: string,
    limit: number,
    signal: AbortSignal,
): Promise<Uint8Array<ArrayBuffer>> {
    const response = await fetchImpl(url, {
        headers: { accept: mediaType },
        credentials: 'omit',
        cache: 'no-store',
        redirect: 'error',
        referrerPolicy: 'no-referrer',
        signal,
    });
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
