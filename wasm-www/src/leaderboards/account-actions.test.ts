import assert from 'node:assert/strict';
import test from 'node:test';
import { updateUsername, deleteRecord, reportRecord, usernameValidationError } from './account-actions.js';
import type { HighscoreApi } from './api.js';
import type { LeaderboardSigningBridge } from './signing.js';
import type { DeletionChallenge, DeletionTarget } from './types.js';

const profile = { publicKey: '12'.repeat(32), username: 'before', publicKeyFingerprint: 'fingerprint' };
const target: DeletionTarget = { kind: 'run', run_id: 'run-1' };
const sign = async (json: string): Promise<string> => JSON.stringify({ ...JSON.parse(json), signature: '34'.repeat(64) });
const bridge: LeaderboardSigningBridge = { publicKey: async () => profile.publicKey, signUsernameUpdate: sign, signDeletionRequest: sign };
const challenge: DeletionChallenge = { schema_version: 1, deletion_challenge_id: 'challenge-1', deletion_challenge_nonce: '56'.repeat(32),
    expires_at_unix_ms: 2_000_000_000_000, public_key: profile.publicKey, target };
function api(calls: string[]): Pick<HighscoreApi, 'usernameChallenge' | 'updateUsername' | 'deletionChallenge' | 'requestDeletion'> {
    return {
        usernameChallenge: async () => { calls.push('rename-challenge'); return { id: 'challenge-1', nonce: '56'.repeat(32), expiresAtUnixMs: 2_000_000_000_000 }; },
        updateUsername: async (key, envelope) => {
            calls.push('rename-write'); assert.equal(key, profile.publicKey); assert.equal(envelope.signature, '34'.repeat(64));
            return { ...profile, username: envelope.username };
        },
        deletionChallenge: async () => { calls.push('delete-challenge'); return challenge; },
        requestDeletion: async envelope => { calls.push('delete-write'); return { requestId: 'delete-1', target: envelope.challenge.target, tombstonedAtUnixMs: 1, purgeEligibleAtUnixMs: null }; },
    };
}

test('owner controllers sign exact claims and write only validated returned documents', async () => {
    const calls: string[] = [];
    assert.equal((await updateUsername(api(calls), bridge, profile, 'after', new AbortController().signal, () => {})).username, 'after');
    assert.deepEqual((await deleteRecord(api(calls), bridge, target, new AbortController().signal, () => {})).target, target);
    assert.deepEqual(calls, ['rename-challenge', 'rename-write', 'delete-challenge', 'delete-write']);
});

test('signer substitutions and mismatched deletion challenges never reach account writes', async () => {
    const calls: string[] = [];
    const signer = { ...bridge, signUsernameUpdate: async (json: string) => sign(JSON.stringify({ ...JSON.parse(json), username: 'attacker' })),
        signDeletionRequest: async (json: string) => { const value = JSON.parse(json); value.challenge.target = { kind: 'run', run_id: 'other' }; return sign(JSON.stringify(value)); } };
    await assert.rejects(updateUsername(api(calls), signer, profile, 'after', new AbortController().signal, () => {}), /changed a signed rename/u);
    await assert.rejects(deleteRecord(api(calls), signer, target, new AbortController().signal, () => {}), /changed a signed deletion/u);
    await assert.rejects(deleteRecord({ ...api(calls), deletionChallenge: async () => ({ ...challenge, public_key: '78'.repeat(32) }) }, bridge, target, new AbortController().signal, () => {}), /different owner or target/u);
    assert.equal(calls.some(call => call.endsWith('write')), false);
});

test('navigation cancels pending signer work and prevents subsequent privileged writes', async () => {
    for (const operation of ['rename', 'delete']) {
        const calls: string[] = [], controller = new AbortController();
        let entered!: () => void;
        const signing = new Promise<void>(resolve => { entered = resolve; });
        const wait = (): Promise<string> => { entered(); return new Promise(() => {}); };
        const signer = { ...bridge, signUsernameUpdate: wait, signDeletionRequest: wait };
        const pending = operation === 'rename'
            ? updateUsername(api(calls), signer, profile, 'after', controller.signal, () => {})
            : deleteRecord(api(calls), signer, target, controller.signal, () => {});
        const rejection = assert.rejects(pending, { name: 'AbortError' });
        await signing; controller.abort(); await rejection;
        assert.equal(calls.some(call => call.endsWith('write')), false);
    }
});

test('public name/report validation rejects byte overflow and controls before network work', async () => {
    for (const name of ['', ' x', 'x ', '😀'.repeat(13), 'x\u202ey', 'x\ny']) assert.notEqual(usernameValidationError(name), null);
    assert.equal(usernameValidationError('😀'.repeat(12)), null);
    let submitted = false;
    const reporter = { report: async () => { submitted = true; return { reportId: 'r', receivedAtUnixMs: 1 }; } };
    await assert.rejects(reportRecord(reporter, target, 'other', '😀'.repeat(501), new AbortController().signal), /2000 bytes/u);
    assert.equal(submitted, false);
    assert.equal((await reportRecord(reporter, target, 'other', 'details', new AbortController().signal)).reportId, 'r');
});
