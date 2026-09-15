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
        'deletion_request',
        'submission',
        'submission_owner_status',
        'username_update',
    ];
    assert.deepEqual(Object.keys(vault.signingDomains).sort(), expected);
    assert.deepEqual(Object.keys(vault.signingLimits).sort(), expected);
    const v2Domains = {
        deletion_request: 'robinhood/leaderboards/2/deletion-request\0',
        submission: 'robinhood/leaderboards/2/submission\0',
        submission_owner_status: 'robinhood/leaderboards/2/submission-owner-status\0',
        username_update: 'robinhood/leaderboards/2/username-update\0',
    };
    for (const [operation, domain] of Object.entries(v2Domains)) {
        assert.deepEqual(vault.signingDomains[operation], encoder.encode(domain));
        const legacy = encoder.encode(`${domain.replace('/2/', '/1/')}{}`);
        assert.throws(() => vault.validateSigningMessage(operation, legacy), /does not use/u);
    }
    for (const operation of expected) {
        const domain = vault.signingDomains[operation];
        const message = new Uint8Array(domain.byteLength + 2);
        message.set(domain);
        message.set(encoder.encode('{}'), domain.byteLength);
        assert.equal(vault.validateSigningMessage(operation, message), message);
    }
    const legacySubmission = encoder.encode('robinhood/leaderboards/1/submission\0{}');
    assert.throws(() => vault.validateSigningMessage('submission', legacySubmission), /does not use/u);
    for (const operation of [
        'raw',
        'sign_raw',
        'session_genesis',
        'replay_session_genesis',
        'named_seat_join',
        'multiplayer_submission',
        'campaign_continuation',
        'fresh_run_preflight_request',
        'competition_run_grant_request',
    ]) {
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
        'sign_deletion_request',
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
