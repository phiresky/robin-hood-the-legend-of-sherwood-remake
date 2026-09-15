import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import test from 'node:test';
import { HighscoreApi, PublicApiError } from './api.js';
import { leaderboardPage } from './model-fixtures.js';
import type { SelectedBoardFilters } from './state.js';
import { canonicalDocumentSha256Sync } from './canonical.js';
import type { SignedDeletionRequest, SignedSubmissionOwnerStatusRequest, SignedUsernameUpdate } from './types.js';

const digest = '11'.repeat(32);
const substitutedPlayerKey = '22'.repeat(32);
const signedDeletion: SignedDeletionRequest = {
    schema_version: 2,
    request: { schema_version: 2, public_key: digest, signed_at_unix_ms: 1_800_000_000_000, target: { kind: 'run', run_id: 'run-1' } },
    algorithm: 'ed25519',
    signature: '33'.repeat(64),
};

for (const method of ['GET', 'POST'] as const) {
    test(`${method} JSON transport retains decoding, bounds, error envelopes and request policies`, async t => {
        const call = (api: HighscoreApi): Promise<unknown> => method === 'GET'
            ? api.metadata() : api.requestDeletion(signedDeletion);
        const api = new HighscoreApi('https://scores.example/api/v1');
        let response: Response;
        t.mock.method(globalThis, 'fetch', async (_url: unknown, init: RequestInit) => {
            assert.equal(init.method, method);
            assert.equal(init.credentials, 'omit');
            assert.equal(init.mode, 'cors');
            assert.equal(init.cache, 'no-store');
            assert.equal(init.redirect, 'error');
            assert.equal(init.referrerPolicy, 'no-referrer');
            assert.ok(init.signal instanceof AbortSignal);
            assert.equal(new Headers(init.headers).get('accept'), 'application/json');
            assert.equal(new Headers(init.headers).get('content-type'), method === 'POST' ? 'application/json' : null);
            assert.equal(init.body, method === 'POST' ? JSON.stringify(signedDeletion) : undefined);
            return response;
        });
        response = new Response('{');
        await assert.rejects(call(api), { code: 'invalid_json', message: 'The server returned invalid JSON.' });
        response = new Response(new Uint8Array([0xff]));
        await assert.rejects(call(api), TypeError);
        response = new Response(JSON.stringify({ schema_version: 1, error: { code: 'denied', message: 'Denied.' } }), { status: 403 });
        await assert.rejects(call(api), { status: 403, code: 'denied', message: 'Denied.' });
        response = new Response('{', { status: 503 });
        await assert.rejects(call(api), { status: 503, code: 'http_503' });
        for (const advertised of [false, true]) {
            for (const rejecting of [false, true]) {
                let cancelled = false;
                response = new Response(new ReadableStream<Uint8Array>({
                    start(controller) { controller.enqueue(new Uint8Array(2 * 1024 * 1024 + 1)); },
                    cancel() {
                        cancelled = true;
                        return rejecting ? Promise.reject(new Error('cleanup failed')) : new Promise<void>(() => {});
                    },
                }), { headers: advertised ? { 'content-length': String(2 * 1024 * 1024 + 1) } : {} });
                await assert.rejects(call(api), { code: 'response_too_large' });
                assert.equal(cancelled, true);
            }
        }
    });

    test(`${method} JSON deadlines and caller cancellation cover headers and body`, { timeout: 2000 }, async t => {
        const call = (signal?: AbortSignal): Promise<unknown> => {
            const api = new HighscoreApi('https://scores.example/api/v1', 20);
            return method === 'GET' ? api.metadata(signal) : api.requestDeletion(signedDeletion, signal);
        };
        for (const stalledBody of [false, true]) {
            for (const abort of [false, true]) {
                let cancelled = false;
                let started!: () => void;
                const entered = new Promise<void>(resolve => { started = resolve; });
                t.mock.method(globalThis, 'fetch', async () => {
                    if (!stalledBody) {
                        started();
                        return new Promise<Response>(() => {});
                    }
                    return new Response(new ReadableStream<Uint8Array>({
                        pull() { started(); },
                        cancel() { cancelled = true; return Promise.reject(new Error('cleanup failed')); },
                    }));
                });
                const controller = new AbortController();
                const reason = new Error('caller stopped request');
                const pending = call(controller.signal);
                await entered;
                // Let the response reader attach before aborting a body read.
                await new Promise<void>(resolve => setTimeout(resolve, 0));
                if (abort) controller.abort(reason);
                await assert.rejects(pending, error => abort ? error === reason
                    : error instanceof PublicApiError && error.code === 'network_timeout');
                assert.equal(cancelled, stalledBody);
                t.mock.restoreAll();
            }
        }
    });
}

function fingerprint(publicKeyHex: string): string {
    return createHash('sha256')
        .update(Buffer.from('robinhood-run-key-fingerprint-v1\0'))
        .update(Buffer.from(publicKeyHex, 'hex'))
        .digest('hex')
        .slice(0, 32);
}

const boardFilters: SelectedBoardFilters = {
    boardId: 'demo-standard-normal',
    missionId: 'Dem_Lei_MP',
    metric: 'original_score',
    maxConcurrentPlayers: null,
    cursor: 'requested-cursor',
};

const emptyBoardPage = (): Record<string, unknown> => leaderboardPage([]);

test('API errors use the frozen nested public error envelope', async () => {
    const originalFetch = globalThis.fetch;
    globalThis.fetch = (async () => new Response(JSON.stringify({
        schema_version: 1,
        error: { code: 'board_not_provisioned', message: 'No official board is provisioned.' },
    }), { status: 404, headers: { 'content-type': 'application/json' } })) as typeof fetch;
    try {
        await assert.rejects(
            new HighscoreApi('https://scores.example/api/v1').metadata(),
            (error: unknown) => error instanceof PublicApiError
                && error.status === 404
                && error.code === 'board_not_provisioned'
                && error.message === 'No official board is provisioned.',
        );
    } finally {
        globalThis.fetch = originalFetch;
    }
});

test('JSON streaming is cancelled at its limit without trusting Content-Length', async () => {
    const originalFetch = globalThis.fetch;
    let cancelled = false;
    const chunk = new Uint8Array(1024 * 1024);
    globalThis.fetch = (async () => new Response(new ReadableStream<Uint8Array>({
        start(controller) {
            controller.enqueue(chunk);
            controller.enqueue(chunk);
            controller.enqueue(chunk);
        },
        cancel() { cancelled = true; },
    }), { status: 200, headers: { 'content-type': 'application/json' } })) as typeof fetch;
    try {
        await assert.rejects(
            new HighscoreApi('https://scores.example/api/v1').metadata(),
            (error: unknown) => error instanceof PublicApiError && error.code === 'response_too_large',
        );
        assert.equal(cancelled, true);
    } finally {
        globalThis.fetch = originalFetch;
    }
});

test('API deadline covers stalled response headers even when a fetch adapter ignores abort', async () => {
    const originalFetch = globalThis.fetch;
    globalThis.fetch = (() => new Promise<Response>(() => {
        // Intentionally never supplies headers and ignores the request signal.
    })) as typeof fetch;
    try {
        await assert.rejects(
            new HighscoreApi('https://scores.example/api/v1', 10).metadata(),
            (error: unknown) => error instanceof PublicApiError && error.code === 'network_timeout',
        );
    } finally {
        globalThis.fetch = originalFetch;
    }
});

test('API deadline cancels a response which stalls between body chunks', async () => {
    const originalFetch = globalThis.fetch;
    let cancelled = false;
    globalThis.fetch = (async () => new Response(new ReadableStream<Uint8Array>({
        start(controller) { controller.enqueue(new TextEncoder().encode('{')); },
        cancel() { cancelled = true; },
    }), { status: 200, headers: { 'content-type': 'application/json' } })) as typeof fetch;
    try {
        await assert.rejects(
            new HighscoreApi('https://scores.example/api/v1', 10).metadata(),
            (error: unknown) => error instanceof PublicApiError && error.code === 'network_timeout',
        );
        assert.equal(cancelled, true);
    } finally {
        globalThis.fetch = originalFetch;
    }
});

test('standalone replay fetch accepts only the canonical CompactRhrec media type', async () => {
    const originalFetch = globalThis.fetch;
    try {
        let accepted: string | null = null;
        globalThis.fetch = (async (_input, init) => {
            accepted = new Headers(init?.headers).get('accept');
            return new Response(new Uint8Array([1, 2, 3]), {
                status: 200,
                headers: { 'content-type': 'application/x-robin-rhrec+compact' },
            });
        }) as typeof fetch;
        assert.deepEqual(
            await new HighscoreApi('https://scores.example/api/v1').replayBytes('run-1'),
            new Uint8Array([1, 2, 3]),
        );
        assert.equal(accepted, null);
    } finally {
        globalThis.fetch = originalFetch;
    }
});

test('standalone replay fetch rejects a JSONL response without format negotiation', async () => {
    const originalFetch = globalThis.fetch;
    try {
        let cancelled = false;
        globalThis.fetch = (async () => new Response(new ReadableStream<Uint8Array>({
            start(controller) { controller.enqueue(new Uint8Array([1, 2, 3])); },
            cancel() { cancelled = true; },
        }), {
            status: 200,
            headers: { 'content-type': 'application/x-robin-rhrec+jsonl' },
        })) as typeof fetch;
        await assert.rejects(
            new HighscoreApi('https://scores.example/api/v1').replayBytes('run-1'),
            (error: unknown) => error instanceof PublicApiError
                && error.code === 'unexpected_media_type',
        );
        assert.equal(cancelled, true);
    } finally {
        globalThis.fetch = originalFetch;
    }
});

test('leaderboard route sends the flat V2 query and rejects cursor substitution', async () => {
    const originalFetch = globalThis.fetch;
    const requests: string[] = [];
    globalThis.fetch = (async input => {
        requests.push(String(input));
        return new Response(JSON.stringify(emptyBoardPage()), {
            status: 200,
            headers: { 'content-type': 'application/json' },
        });
    }) as typeof fetch;
    try {
        const api = new HighscoreApi('https://scores.example/api/v1');
        await assert.rejects(
            api.board(boardFilters),
            (error: unknown) => error instanceof PublicApiError
                && error.code === 'leaderboard_cursor_mismatch',
        );
        assert.equal(
            requests[0],
            'https://scores.example/api/v1/leaderboards?schema_version=2&board_id=demo-standard-normal&mission_id=Dem_Lei_MP&metric=original_score&limit=25&cursor=requested-cursor',
        );
    } finally {
        globalThis.fetch = originalFetch;
    }
});

test('player run history uses the typed bounded query and rejects response identity substitution', async () => {
    const originalFetch = globalThis.fetch;
    const requests: string[] = [];
    const responses: unknown[] = [{
        schema_version: 2,
        player: {
            schema_version: 1,
            username: 'Robin',
            public_key: digest,
            public_key_fingerprint: fingerprint(digest),
        },
        accepted_sequence_watermark: 0,
        runs: [],
        personal_bests: [],
        next_cursor: null,
    }, {
        schema_version: 2,
        player: {
            schema_version: 1,
            username: 'Substituted Player',
            public_key: substitutedPlayerKey,
            public_key_fingerprint: fingerprint(substitutedPlayerKey),
        },
        accepted_sequence_watermark: 0,
        runs: [],
        personal_bests: [],
        next_cursor: null,
    }];
    globalThis.fetch = (async input => {
        requests.push(String(input));
        return new Response(JSON.stringify(responses.shift()), {
            status: 200,
            headers: { 'content-type': 'application/json' },
        });
    }) as typeof fetch;
    try {
        const api = new HighscoreApi('https://scores.example/api/v1');
        const page = await api.playerRuns(digest, 'opaque+cursor');
        assert.equal(page.player.publicKey, digest);
        assert.equal(
            requests[0],
            `https://scores.example/api/v1/players/${digest}/runs?schema_version=1&limit=25&cursor=opaque%2Bcursor`,
        );
        await assert.rejects(
            api.playerRuns(digest, null),
            (error: unknown) => error instanceof PublicApiError
                && error.code === 'player_history_identity_mismatch',
        );
    } finally {
        globalThis.fetch = originalFetch;
    }
});

test('player history rejects hostile keys and cursors before network access', async () => {
    const originalFetch = globalThis.fetch;
    let calls = 0;
    globalThis.fetch = (async () => {
        calls += 1;
        throw new Error('fetch must not run');
    }) as typeof fetch;
    try {
        const api = new HighscoreApi('https://scores.example/api/v1');
        await assert.rejects(api.playerRuns('../private', null), /public key identity/u);
        await assert.rejects(api.playerRuns(digest, 'cursor\nsmuggle'), /cursor is invalid/u);
        await assert.rejects(api.playerRuns(digest, 'x'.repeat(4097)), /cursor is invalid/u);
        assert.equal(calls, 0);
    } finally {
        globalThis.fetch = originalFetch;
    }
});

test('a first leaderboard page correctly binds the absence of a cursor', async () => {
    const originalFetch = globalThis.fetch;
    globalThis.fetch = (async () => new Response(JSON.stringify(emptyBoardPage()), {
        status: 200,
        headers: { 'content-type': 'application/json' },
    })) as typeof fetch;
    try {
        const page = await new HighscoreApi('https://scores.example/api/v1').board({
            ...boardFilters,
            cursor: null,
        });
        assert.equal(page.previousCursor, null);
    } finally {
        globalThis.fetch = originalFetch;
    }
});

test('signed owner requests use one-step routes and bind owner-status responses to the request digest', async () => {
    const originalFetch = globalThis.fetch;
    const rename: SignedUsernameUpdate = {
        schema_version: 2,
        request: { schema_version: 2, public_key: digest, signed_at_unix_ms: 1_800_000_000_000, username: 'Robin' },
        algorithm: 'ed25519',
        signature: '44'.repeat(64),
    };
    const status: SignedSubmissionOwnerStatusRequest = {
        schema_version: 2,
        request: { schema_version: 2, public_key: digest, signed_at_unix_ms: 1_800_000_000_000, submission_id: 'sub-1' },
        algorithm: 'ed25519',
        signature: '55'.repeat(64),
    };
    const requests: { url: string; method: string | undefined; body: unknown }[] = [];
    const responses: unknown[] = [
        { schema_version: 1, username: 'Robin', public_key: digest, public_key_fingerprint: fingerprint(digest) },
        { schema_version: 1, deletion_request_id: 'del-1', target: { kind: 'run', run_id: 'run-1' }, tombstoned_at_unix_ms: 1, purge_eligible_at_unix_ms: null },
        { schema_version: 2, submission_id: 'sub-1', public_key: digest, request_sha256: canonicalDocumentSha256Sync(status), state: { state: 'verifying' } },
        { schema_version: 2, submission_id: 'sub-1', public_key: digest, request_sha256: '66'.repeat(32), state: { state: 'verifying' } },
    ];
    globalThis.fetch = (async (input, init) => {
        requests.push({ url: String(input), method: init?.method, body: JSON.parse(String(init?.body)) as unknown });
        return new Response(JSON.stringify(responses.shift()), { status: 200, headers: { 'content-type': 'application/json' } });
    }) as typeof fetch;
    try {
        const api = new HighscoreApi('https://scores.example/api/v1');
        assert.equal((await api.updateUsername(digest, rename)).username, 'Robin');
        assert.equal((await api.requestDeletion(signedDeletion)).requestId, 'del-1');
        assert.deepEqual((await api.submissionOwnerStatus(status)).lifecycle, { state: 'verifying' });
        await assert.rejects(api.submissionOwnerStatus(status), /does not answer the signed request/u);
        await assert.rejects(api.updateUsername(substitutedPlayerKey, rename), { code: 'signed_request_mismatch' });
        assert.deepEqual(requests.map(request => [request.method, request.url]), [
            ['PUT', `https://scores.example/api/v1/players/${digest}/username`],
            ['POST', 'https://scores.example/api/v1/deletion-requests'],
            ['POST', 'https://scores.example/api/v1/submissions/sub-1/private-status'],
            ['POST', 'https://scores.example/api/v1/submissions/sub-1/private-status'],
        ]);
        assert.deepEqual(requests.map(request => request.body), [rename, signedDeletion, status, status]);
        assert.equal(requests.some(request => request.url.includes('challenge')), false);
    } finally {
        globalThis.fetch = originalFetch;
    }
});
