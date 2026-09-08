import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import test from 'node:test';
import { HighscoreApi, PublicApiError } from './api.js';
import type { BoardFilters } from './state.js';

const digest = '11'.repeat(32);
const substitutedPlayerKey = '22'.repeat(32);

function fingerprint(publicKeyHex: string): string {
    return createHash('sha256')
        .update(Buffer.from('robinhood-run-key-fingerprint-v1\0'))
        .update(Buffer.from(publicKeyHex, 'hex'))
        .digest('hex')
        .slice(0, 32);
}

const fullCampaignFilters: BoardFilters = {
    subject: 'full_campaign',
    metric: 'original_score',
    missionId: null,
    presetId: 'standard',
    difficultyId: 'normal',
    rulesetId: digest,
    contentIdentitySha256: digest,
    rulesConfigSha256: digest,
    competitionManifestSha256: null,
    maxConcurrentPlayers: null,
    cursor: 'requested-cursor',
};

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

test('content-addressed manifest routes reject an invalid digest before fetching', async () => {
    const originalFetch = globalThis.fetch;
    let calls = 0;
    globalThis.fetch = (async () => {
        calls += 1;
        throw new Error('fetch must not run');
    }) as typeof fetch;
    try {
        const api = new HighscoreApi('https://scores.example/api/v1');
        await assert.rejects(api.buildManifest('../not-a-digest'), /non-zero lowercase SHA-256/u);
        await assert.rejects(api.contentManifest('0'.repeat(64)), /non-zero lowercase SHA-256/u);
        await assert.rejects(api.campaignContentManifest('../catalog'), /non-zero lowercase SHA-256/u);
        await assert.rejects(api.rulesConfig('A'.repeat(64)), /non-zero lowercase SHA-256/u);
        await assert.rejects(api.rulesetManifest('short'), /non-zero lowercase SHA-256/u);
        assert.throws(() => api.campaignSessionReplayUrl('aggregate/id', -1), /ordinal/u);
        assert.equal(
            api.campaignSessionReplayUrl('aggregate/id', 7),
            'https://scores.example/api/v1/runs/aggregate%2Fid/sessions/7/replay',
        );
        assert.equal(calls, 0);
    } finally {
        globalThis.fetch = originalFetch;
    }
});

test('campaign content catalog uses its exact immutable digest route', async () => {
    const originalFetch = globalThis.fetch;
    let request: { readonly url: string; readonly cache: RequestCache | undefined } | null = null;
    globalThis.fetch = (async (input, init) => {
        request = { url: String(input), cache: init?.cache };
        return new Response('{}', { headers: { 'content-type': 'application/json' } });
    }) as typeof fetch;
    try {
        await assert.rejects(
            new HighscoreApi('https://scores.example/api/v1').campaignContentManifest(digest),
            /schema_version/u,
        );
        assert.deepEqual(request, {
            url: `https://scores.example/api/v1/campaign-content-manifests/${digest}`,
            cache: 'force-cache',
        });
    } finally {
        globalThis.fetch = originalFetch;
    }
});

test('ruleset lookup cross-binds immutable policy and mutable publication routes', async () => {
    const originalFetch = globalThis.fetch;
    const requests: { readonly url: string; readonly cache: RequestCache | undefined }[] = [];
    globalThis.fetch = (async (input, init) => {
        requests.push({ url: String(input), cache: init?.cache });
        return new Response('{}', { headers: { 'content-type': 'application/json' } });
    }) as typeof fetch;
    try {
        await assert.rejects(
            new HighscoreApi('https://scores.example/api/v1').rulesetManifest(digest),
            /missing required field|schema_version/u,
        );
        assert.deepEqual(requests, [{
            url: `https://scores.example/api/v1/ruleset-manifests/${digest}`,
            cache: 'force-cache',
        }, {
            url: `https://scores.example/api/v1/published-rulesets/${digest}`,
            cache: 'no-store',
        }]);
    } finally {
        globalThis.fetch = originalFetch;
    }
});

test('leaderboard and player routes reject identity substitution', async () => {
    const originalFetch = globalThis.fetch;
    const responses: unknown[] = [{
        schema_version: 1,
        filter: {
            schema_version: 1,
            subject: { kind: 'full_campaign' },
            metric: 'original_score',
            content: { kind: 'full_campaign', campaign_content_manifest_sha256: digest },
            rules_config_sha256: digest,
            ruleset_manifest_sha256: digest,
            competition_manifest_sha256: null,
            max_concurrent_players: null,
        },
        accepted_sequence_watermark: 0,
        previous_cursor: null,
        entries: [],
        next_cursor: null,
    }, {
        schema_version: 1,
        username: 'Substituted Player',
        public_key: substitutedPlayerKey,
        public_key_fingerprint: fingerprint(substitutedPlayerKey),
    }];
    globalThis.fetch = (async () => new Response(JSON.stringify(responses.shift()), {
        status: 200,
        headers: { 'content-type': 'application/json' },
    })) as typeof fetch;
    try {
        const api = new HighscoreApi('https://scores.example/api/v1');
        await assert.rejects(
            api.board(fullCampaignFilters),
            (error: unknown) => error instanceof PublicApiError
                && error.code === 'leaderboard_cursor_mismatch',
        );
        await assert.rejects(
            api.player(digest),
            (error: unknown) => error instanceof PublicApiError
                && error.code === 'player_identity_mismatch',
        );
    } finally {
        globalThis.fetch = originalFetch;
    }
});

test('player run history uses the typed bounded query and rejects response identity substitution', async () => {
    const originalFetch = globalThis.fetch;
    const requests: string[] = [];
    const responses: unknown[] = [{
        schema_version: 1,
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
        schema_version: 1,
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
    globalThis.fetch = (async () => new Response(JSON.stringify({
        schema_version: 1,
        filter: {
            schema_version: 1,
            subject: { kind: 'full_campaign' },
            metric: 'original_score',
            content: { kind: 'full_campaign', campaign_content_manifest_sha256: digest },
            rules_config_sha256: digest,
            ruleset_manifest_sha256: digest,
            competition_manifest_sha256: null,
            max_concurrent_players: null,
        },
        accepted_sequence_watermark: 0,
        previous_cursor: null,
        entries: [],
        next_cursor: null,
    }), {
        status: 200,
        headers: { 'content-type': 'application/json' },
    })) as typeof fetch;
    try {
        const page = await new HighscoreApi('https://scores.example/api/v1').board({
            ...fullCampaignFilters,
            cursor: null,
        });
        assert.equal(page.previousCursor, null);
    } finally {
        globalThis.fetch = originalFetch;
    }
});
