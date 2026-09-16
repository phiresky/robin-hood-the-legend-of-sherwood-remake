import assert from 'node:assert/strict';
import test from 'node:test';
import {
    LEADERBOARD_IDENTITY_PROTOCOL,
    decodeLeaderboardIdentityRequest,
    dispatchLeaderboardIdentityRequest,
    installLeaderboardIdentitySigner,
    type LeaderboardIdentityBridgeModule,
} from './protocol.js';

const requestId = 'ab'.repeat(16);
const publicKey = '12'.repeat(32);
const signature = '34'.repeat(64);
const signedAt = 1_800_000_000_000;

const claims = {
    sign_username_update: { schema_version: 2, public_key: publicKey, signed_at_unix_ms: signedAt, username: 'Robin' },
    sign_submission: {
        schema_version: 3,
        uploader_public_key: publicKey,
        signed_at_unix_ms: signedAt,
        public_disclosure: 'named_profile',
        board_id: 'demo-standard-normal',
        mission_id: 'Dem_Lei_MP',
        replay: { artifact: { sha256: '56'.repeat(32), byte_length: 1, media_type: 'application/x-robin-rhrec' }, replay_schema_version: 1 },
        requested_metrics: ['original_score'],
    },
    sign_submission_owner_status: { schema_version: 2, public_key: publicKey, signed_at_unix_ms: signedAt, submission_id: 'sub-1' },
    sign_deletion_request: { schema_version: 2, public_key: publicKey, signed_at_unix_ms: signedAt, target: { kind: 'run', run_id: 'run-1' } },
} as const;
type SigningOperation = keyof typeof claims;
const claimJson = (operation: SigningOperation): string => JSON.stringify(claims[operation]);

test('installed signer ignores wrong origins/windows and rejects prohibited operations without signing', async t => {
    const original = Object.getOwnPropertyDescriptor(globalThis, 'window');
    t.after(() => {
        if (original) Object.defineProperty(globalThis, 'window', original);
        else Reflect.deleteProperty(globalThis, 'window');
    });
    const replies: { data: unknown; origin: string }[] = [];
    const parent = { postMessage: (data: unknown, origin: string) => replies.push({ data, origin }) };
    let receive!: (event: { source: unknown; origin: string; data: unknown }) => void;
    const host = { top: parent, self: {}, parent, addEventListener: (_: string, fn: typeof receive) => { receive = fn; } };
    Object.defineProperty(globalThis, 'window', { configurable: true, value: host });
    const calls: string[] = [];
    installLeaderboardIdentitySigner(bridge(calls));
    const origin = 'https://robinhood.phiresky.xyz';
    const json = claimJson('sign_username_update');
    assert.deepEqual(replies, [{ data: { protocol: LEADERBOARD_IDENTITY_PROTOCOL, kind: 'ready' }, origin }]);
    replies.length = 0;
    receive({ source: {}, origin, data: request('sign_username_update', json) });
    receive({ source: parent, origin: 'https://attacker.example', data: request('sign_username_update', json) });
    await new Promise(resolve => setImmediate(resolve));
    assert.equal(replies.length, 0);
    assert.deepEqual(calls, [`authorize:${origin}`]);
    for (const operation of ['sign_raw', 'sign_bytes', 'sign_message', 'sign_arbitrary']) {
        receive({ source: parent, origin, data: request(operation, json) });
        await new Promise(resolve => setImmediate(resolve));
        assert.equal((replies.at(-1)?.data as unknown as { ok: boolean }).ok, false);
    }
    assert.deepEqual(calls, [`authorize:${origin}`]);
    receive({ source: parent, origin, data: { ...request('sign_username_update', json), requestId: 'cd'.repeat(16) } });
    await new Promise(resolve => setImmediate(resolve));
    assert.equal(calls.at(-1), 'username');
    assert.equal((replies.at(-1)?.data as unknown as { ok: boolean }).ok, true);
    assert.ok(replies.every(reply => reply.origin === origin));
    host.top = host.self as typeof parent;
    assert.throws(() => installLeaderboardIdentitySigner(bridge([])), /top-level/u);
});

function request(operation: string, payloadJson?: string): Record<string, unknown> {
    return {
        protocol: LEADERBOARD_IDENTITY_PROTOCOL,
        requestId,
        operation,
        ...(payloadJson === undefined ? {} : { payloadJson }),
    };
}

/** Stands in for the wasm vault: echoes the claim into the frozen result shapes. */
function bridge(calls: string[]): LeaderboardIdentityBridgeModule {
    const signed = async (name: string, _origin: string, value: string): Promise<string> => {
        calls.push(name);
        return JSON.stringify({ schema_version: 2, request: JSON.parse(value) as unknown, algorithm: 'ed25519', signature });
    };
    return {
        robinhoodAuthorizeLeaderboardIdentityParent: origin => { calls.push(`authorize:${origin}`); },
        robinhoodLeaderboardIdentityStatus: async () => JSON.stringify({ kind: 'status', publicKey }),
        robinhoodLeaderboardPublicKey: async () => publicKey,
        robinhoodSignUsernameUpdate: (origin, value) => signed('username', origin, value),
        robinhoodSignSubmissionClaim: async (_origin, value) => {
            calls.push('submission');
            const claim = JSON.parse(value) as { uploader_public_key: string };
            return JSON.stringify({ public_key: claim.uploader_public_key, signature });
        },
        robinhoodSignSubmissionOwnerStatus: (origin, value) => signed('owner_status', origin, value),
        robinhoodSignDeletionRequest: (origin, value) => signed('deletion', origin, value),
    };
}

test('decoder accepts only the closed leaderboard operation set and exact fields', () => {
    for (const operation of ['status', 'public_key']) {
        assert.equal(decodeLeaderboardIdentityRequest(request(operation)).operation, operation);
    }
    for (const operation of Object.keys(claims) as SigningOperation[]) {
        assert.equal(decodeLeaderboardIdentityRequest(request(operation, claimJson(operation))).operation, operation);
    }
    for (const operation of [
        'raw', 'sign_raw', 'approve_rekey', 'sign_session_genesis', 'toString', '__proto__',
        // Removed with the ranked protocol V2 simplification.
        'sign_multiplayer_leaderboard_request',
        'sign_named_seat_join',
        'sign_replay_session_genesis',
        'sign_competition_run_grant_request',
        'sign_fresh_run_preflight_request',
        'sign_campaign_continuation_preflight_as_host',
        'sign_campaign_continuation_preflight_as_controller',
        'sign_campaign_continuation',
    ]) {
        assert.throws(() => decodeLeaderboardIdentityRequest(request(operation, claimJson('sign_submission'))), /unsupported/u);
    }
    assert.throws(() => decodeLeaderboardIdentityRequest({
        ...request('status'), payloadJson: claimJson('sign_submission'),
    }), /invalid fields/u);
    assert.throws(() => decodeLeaderboardIdentityRequest({
        ...request('sign_submission', claimJson('sign_submission')), legacyRawKeyPresent: false,
    }), /invalid fields/u);
    assert.throws(() => decodeLeaderboardIdentityRequest(request('sign_submission')), /invalid fields/u);
});

test('sign operations take the timestamped claim itself and reject challenge embeddings', () => {
    const rejects = (operation: SigningOperation, claim: unknown, pattern: RegExp) => assert.throws(
        () => decodeLeaderboardIdentityRequest(request(operation, JSON.stringify(claim))),
        (error: unknown) => (error as { code?: string }).code === 'invalid_claim' && pattern.test((error as Error).message),
    );
    const { signed_at_unix_ms: _signedAt, ...untimed } = claims.sign_deletion_request;
    rejects('sign_deletion_request', untimed, /missing or unknown fields/u);
    rejects('sign_deletion_request', { schema_version: 1, challenge: claims.sign_deletion_request, signature: '0'.repeat(128) }, /missing or unknown fields/u);
    rejects('sign_username_update', { ...claims.sign_username_update, username_challenge_nonce: '56'.repeat(32) }, /missing or unknown fields/u);
    rejects('sign_submission', { ...claims.sign_submission, upload_challenge: {} }, /missing or unknown fields/u);
    rejects('sign_submission', { ...claims.sign_submission, schema_version: 2 }, /schema_version must be 3/u);
    rejects('sign_submission_owner_status', { ...claims.sign_submission_owner_status, schema_version: 3 }, /schema_version must be 2/u);
    rejects('sign_username_update', { ...claims.sign_username_update, public_key: '0'.repeat(64) }, /public_key is invalid/u);
    rejects('sign_username_update', { ...claims.sign_username_update, signed_at_unix_ms: 0 }, /positive integer/u);
    rejects('sign_username_update', { ...claims.sign_username_update, signed_at_unix_ms: 1.5 }, /positive integer/u);
});

test('dispatch routes every typed operation and freezes result shapes', async () => {
    const calls: string[] = [];
    const signer = bridge(calls);
    const status = await dispatchLeaderboardIdentityRequest(request('status'), signer);
    assert.deepEqual(status, {
        protocol: LEADERBOARD_IDENTITY_PROTOCOL,
        requestId,
        ok: true,
        result: { kind: 'status', publicKey },
    });
    const key = await dispatchLeaderboardIdentityRequest(request('public_key'), signer);
    assert.deepEqual(key, {
        protocol: LEADERBOARD_IDENTITY_PROTOCOL,
        requestId,
        ok: true,
        result: { kind: 'public_key', publicKey },
    });
    for (const [operation, call] of [
        ['sign_username_update', 'username'],
        ['sign_submission_owner_status', 'owner_status'],
        ['sign_deletion_request', 'deletion'],
    ] as const) {
        const result = await dispatchLeaderboardIdentityRequest(request(operation, claimJson(operation)), signer);
        assert.deepEqual(result, {
            protocol: LEADERBOARD_IDENTITY_PROTOCOL,
            requestId,
            ok: true,
            result: {
                kind: 'signed_document',
                documentJson: JSON.stringify({ schema_version: 2, request: claims[operation], algorithm: 'ed25519', signature }),
            },
        });
        assert.equal(calls.at(-1), call);
    }
    const submission = await dispatchLeaderboardIdentityRequest(request('sign_submission', claimJson('sign_submission')), signer);
    assert.deepEqual(submission, {
        protocol: LEADERBOARD_IDENTITY_PROTOCOL,
        requestId,
        ok: true,
        result: { kind: 'participant_signature', participantSignatureJson: JSON.stringify({ public_key: publicKey, signature }) },
    });
    assert.equal(calls.at(-1), 'submission');
});

test('bridge results that change the claim or its wrapper fail closed', async () => {
    const base = bridge([]);
    const cases: [SigningOperation, Partial<LeaderboardIdentityBridgeModule>][] = [
        ['sign_deletion_request', { robinhoodSignDeletionRequest: async (_origin, value) => JSON.stringify({
            schema_version: 2, request: { ...JSON.parse(value) as object, target: { kind: 'run', run_id: 'other' } }, algorithm: 'ed25519', signature,
        }) }],
        ['sign_username_update', { robinhoodSignUsernameUpdate: async (_origin, value) => JSON.stringify({
            schema_version: 1, request: JSON.parse(value) as unknown, algorithm: 'ed25519', signature,
        }) }],
        ['sign_submission_owner_status', { robinhoodSignSubmissionOwnerStatus: async (_origin, value) => JSON.stringify({
            schema_version: 2, request: JSON.parse(value) as unknown, algorithm: 'ed25519', signature: '0'.repeat(128),
        }) }],
        ['sign_submission', { robinhoodSignSubmissionClaim: async () => JSON.stringify({ public_key: '78'.repeat(32), signature }) }],
    ];
    for (const [operation, override] of cases) {
        const response = await dispatchLeaderboardIdentityRequest(request(operation, claimJson(operation)), { ...base, ...override });
        assert.equal(response.ok, false, operation);
        if (!response.ok) assert.equal(response.error.code, 'invalid_bridge_result', operation);
    }
});

test('payload and bridge result limits fail closed', async () => {
    assert.throws(
        () => decodeLeaderboardIdentityRequest(request('sign_deletion_request', `{"x":"${'a'.repeat(8192)}"}`)),
        /exceeds/u,
    );
    const signer: LeaderboardIdentityBridgeModule = {
        ...bridge([]),
        robinhoodLeaderboardPublicKey: async () => '0'.repeat(64),
    };
    const response = await dispatchLeaderboardIdentityRequest(request('public_key'), signer);
    assert.equal(response.ok, false);
    if (!response.ok) assert.equal(response.error.code, 'invalid_bridge_result');
});
