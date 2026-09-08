import assert from 'node:assert/strict';
import test from 'node:test';
import { leaderboardIdentityClientTestHooks as client } from './browser_identity_client.js';
import { leaderboardIdentityVaultTestHooks as vault } from '../../robin_identity_signer/js/browser_identity_vault.js';

const encoder = new TextEncoder();

test('leaderboards and multiplayer share the exact durable identity record', () => {
    assert.equal(vault.databaseName, 'robinhood-multiplayer-identity-v1');
    assert.equal(vault.databaseVersion, 1);
    assert.equal(vault.identityStore, 'identity');
    assert.equal(vault.redemptionStore, 'redemptions');
    assert.equal(vault.identityKey, 'browser-seat-owner-v1');
});

test('vault accepts only the closed typed signing domains', () => {
    const expected = [
        'campaign_continuation',
        'campaign_continuation_preflight_controller',
        'campaign_continuation_preflight_host',
        'competition_run_grant_request',
        'deletion_request',
        'fresh_run_preflight_request',
        'multiplayer_campaign_continuation',
        'multiplayer_submission',
        'named_seat_join',
        'replay_session_genesis',
        'submission',
        'submission_owner_status',
        'username_update',
    ];
    assert.deepEqual(Object.keys(vault.signingDomains).sort(), expected);
    for (const operation of expected.filter(operation => !operation.startsWith('multiplayer_'))) {
        const domain = vault.signingDomains[operation];
        const message = new Uint8Array(domain.byteLength + 2);
        message.set(domain);
        message.set(encoder.encode('{}'), domain.byteLength);
        assert.equal(vault.validateSigningMessage(operation, message), message);
    }
    const coSignDomain = vault.signingDomains.multiplayer_submission;
    for (const [operation, purpose] of [
        ['multiplayer_campaign_continuation', 1],
        ['multiplayer_submission', 2],
    ]) {
        const message = new Uint8Array(coSignDomain.byteLength + 1 + (3 * 32));
        message.set(coSignDomain);
        message[coSignDomain.byteLength] = purpose;
        assert.equal(vault.validateSigningMessage(operation, message), message);
    }
    const wrongPurpose = new Uint8Array(coSignDomain.byteLength + 1 + (3 * 32));
    wrongPurpose.set(coSignDomain);
    wrongPurpose[coSignDomain.byteLength] = 2;
    assert.throws(
        () => vault.validateSigningMessage('multiplayer_campaign_continuation', wrongPurpose),
        /fixed co-signing payload/u,
    );
    assert.throws(
        () => vault.validateSigningMessage(
            'multiplayer_submission',
            wrongPurpose.subarray(0, wrongPurpose.byteLength - 1),
        ),
        /fixed co-signing payload/u,
    );
    for (const operation of ['raw', 'sign_raw', 'session_genesis']) {
        assert.throws(
            () => vault.validateSigningMessage(operation, encoder.encode('{}')),
            /Unsupported leaderboard identity operation/u,
        );
    }
});

test('vault rejects cross-domain substitution and the client has no legacy operations', () => {
    const usernameDomain = vault.signingDomains.username_update;
    const message = new Uint8Array(usernameDomain.byteLength + 2);
    message.set(usernameDomain);
    message.set(encoder.encode('{}'), usernameDomain.byteLength);
    assert.throws(() => vault.validateSigningMessage('submission', message), /does not use/u);
    assert.deepEqual([...client.operations].sort(), [
        'public_key',
        'sign_campaign_continuation',
        'sign_campaign_continuation_preflight_as_controller',
        'sign_campaign_continuation_preflight_as_host',
        'sign_competition_run_grant_request',
        'sign_deletion_request',
        'sign_fresh_run_preflight_request',
        'sign_multiplayer_leaderboard_request',
        'sign_named_seat_join',
        'sign_replay_session_genesis',
        'sign_submission',
        'sign_submission_owner_status',
        'sign_username_update',
        'status',
    ]);
});

test('signer origin requires an exact isolated HTTPS origin', () => {
    assert.equal(
        client.validateSignerOrigin('https://identity.robinhood.phiresky.xyz', 'https://robinhood.phiresky.xyz'),
        'https://identity.robinhood.phiresky.xyz',
    );
    assert.throws(
        () => client.validateSignerOrigin('https://identity.robinhood.phiresky.xyz/path', 'https://robinhood.phiresky.xyz'),
        /exact origin/u,
    );
    assert.throws(
        () => client.validateSignerOrigin('http://identity.example', 'https://robinhood.phiresky.xyz'),
        /requires HTTPS/u,
    );
    assert.throws(
        () => client.validateSignerOrigin('https://robinhood.phiresky.xyz', 'https://robinhood.phiresky.xyz'),
        /must differ/u,
    );
});
