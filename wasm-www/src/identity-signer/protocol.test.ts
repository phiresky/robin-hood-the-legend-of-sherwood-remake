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
const json = '{"schema_version":1}';

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

function bridge(calls: string[]): LeaderboardIdentityBridgeModule {
    const signed = async (name: string, _origin: string, value: string): Promise<string> => {
        calls.push(name);
        return value;
    };
    return {
        robinhoodAuthorizeLeaderboardIdentityParent: origin => { calls.push(`authorize:${origin}`); },
        robinhoodLeaderboardIdentityStatus: async () => JSON.stringify({ kind: 'status', publicKey }),
        robinhoodLeaderboardPublicKey: async () => publicKey,
        robinhoodSignUsernameUpdate: (origin, value) => signed('username', origin, value),
        robinhoodSignSubmissionClaim: (origin, value) => signed('submission', origin, value),
        robinhoodSignMultiplayerLeaderboardRequest: (origin, value) => signed('multiplayer', origin, value),
        robinhoodSignNamedSeatJoin: (origin, value) => signed('named_seat_join', origin, value),
        robinhoodSignReplaySessionGenesis: (origin, value) => signed('replay_session_genesis', origin, value),
        robinhoodSignCompetitionRunGrantRequest: (origin, value) => signed('grant', origin, value),
        robinhoodSignFreshRunPreflightRequest: (origin, value) => signed('fresh_preflight', origin, value),
        robinhoodSignCampaignContinuationPreflightAsHost: (origin, value) => signed('continuation_preflight_host', origin, value),
        robinhoodSignCampaignContinuationPreflightAsController: (origin, value) => signed('continuation_preflight_controller', origin, value),
        robinhoodSignCampaignContinuation: (origin, value) => signed('continuation', origin, value),
        robinhoodSignSubmissionOwnerStatus: (origin, value) => signed('owner_status', origin, value),
        robinhoodSignDeletionRequest: (origin, value) => signed('deletion', origin, value),
    };
}

test('decoder accepts only the closed leaderboard operation set and exact fields', () => {
    for (const operation of ['status', 'public_key']) {
        assert.equal(decodeLeaderboardIdentityRequest(request(operation)).operation, operation);
    }
    for (const operation of [
        'sign_username_update',
        'sign_submission',
        'sign_multiplayer_leaderboard_request',
        'sign_named_seat_join',
        'sign_replay_session_genesis',
        'sign_competition_run_grant_request',
        'sign_fresh_run_preflight_request',
        'sign_campaign_continuation_preflight_as_host',
        'sign_campaign_continuation_preflight_as_controller',
        'sign_campaign_continuation',
        'sign_submission_owner_status',
        'sign_deletion_request',
    ]) {
        assert.equal(decodeLeaderboardIdentityRequest(request(operation, json)).operation, operation);
    }
    for (const operation of [
        'raw', 'sign_raw', 'approve_rekey', 'sign_session_genesis',
    ]) {
        assert.throws(() => decodeLeaderboardIdentityRequest(request(operation, json)), /unsupported/u);
    }
    assert.throws(() => decodeLeaderboardIdentityRequest({
        ...request('status'), payloadJson: json,
    }), /invalid fields/u);
    assert.throws(() => decodeLeaderboardIdentityRequest({
        ...request('sign_submission', json), legacyRawKeyPresent: false,
    }), /invalid fields/u);
    assert.throws(() => decodeLeaderboardIdentityRequest(request('sign_submission')), /invalid fields/u);
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
    for (const [operation, call, kind] of [
        ['sign_username_update', 'username', 'signed_document'],
        ['sign_submission', 'submission', 'participant_signature'],
        ['sign_multiplayer_leaderboard_request', 'multiplayer', 'participant_signature'],
        ['sign_named_seat_join', 'named_seat_join', 'signed_document'],
        ['sign_replay_session_genesis', 'replay_session_genesis', 'signed_document'],
        ['sign_competition_run_grant_request', 'grant', 'signed_document'],
        ['sign_fresh_run_preflight_request', 'fresh_preflight', 'signed_document'],
        ['sign_campaign_continuation_preflight_as_host', 'continuation_preflight_host', 'participant_signature'],
        ['sign_campaign_continuation_preflight_as_controller', 'continuation_preflight_controller', 'participant_signature'],
        ['sign_campaign_continuation', 'continuation', 'signed_document'],
        ['sign_submission_owner_status', 'owner_status', 'signed_document'],
        ['sign_deletion_request', 'deletion', 'signed_document'],
    ] as const) {
        const result = await dispatchLeaderboardIdentityRequest(request(operation, json), signer);
        assert.equal(result.ok, true);
        if (result.ok) assert.equal((result.result as { kind: string }).kind, kind);
        assert.equal(calls.at(-1), call);
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
