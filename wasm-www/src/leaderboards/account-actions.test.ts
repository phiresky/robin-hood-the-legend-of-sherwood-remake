import assert from 'node:assert/strict';
import { generateKeyPairSync, sign as ed25519Sign, verify as ed25519Verify } from 'node:crypto';
import test from 'node:test';
import {
    updateUsername, deleteRecord, reportRecord, submissionOwnerStatus, usernameValidationError,
} from './account-actions.js';
import { SIGNED_REQUEST_DOMAINS, signedRequestSigningBytes } from './account-contract.js';
import type { HighscoreApi } from './api.js';
import { canonicalDocumentSha256Sync } from './canonical.js';
import type { LeaderboardSigningBridge } from './signing.js';
import type { DeletionTarget } from './types.js';

const { privateKey, publicKey: publicKeyObject } = generateKeyPairSync('ed25519');
const ownerKey = Buffer.from(publicKeyObject.export({ format: 'jwk' }).x!, 'base64url').toString('hex');
const profile = { publicKey: ownerKey, username: 'before', publicKeyFingerprint: 'fingerprint' };
const target: DeletionTarget = { kind: 'run', run_id: 'run-1' };
const clock = () => 1_800_000_000_000;

function signer(domain: string) {
    return async (claimJson: string): Promise<string> => {
        const request = JSON.parse(claimJson) as unknown;
        const signature = ed25519Sign(null, signedRequestSigningBytes(domain, request), privateKey).toString('hex');
        return JSON.stringify({ schema_version: 2, request, algorithm: 'ed25519', signature });
    };
}

const bridge: LeaderboardSigningBridge = {
    publicKey: async () => ownerKey,
    signUsernameUpdate: signer(SIGNED_REQUEST_DOMAINS.usernameUpdate),
    signSubmissionOwnerStatus: signer(SIGNED_REQUEST_DOMAINS.submissionOwnerStatus),
    signDeletionRequest: signer(SIGNED_REQUEST_DOMAINS.deletionRequest),
};

function verifies(domain: string, document: { request: unknown; signature: string }): boolean {
    return ed25519Verify(null, signedRequestSigningBytes(domain, document.request), publicKeyObject,
        Buffer.from(document.signature, 'hex'));
}

type AccountApi = Pick<HighscoreApi, 'updateUsername' | 'requestDeletion' | 'submissionOwnerStatus'>;

function api(calls: string[]): AccountApi {
    return {
        updateUsername: async (key, signed) => {
            calls.push('rename-write');
            assert.equal(key, ownerKey);
            assert.deepEqual(signed.request, { schema_version: 2, public_key: ownerKey, signed_at_unix_ms: clock(), username: signed.request.username });
            assert.ok(verifies(SIGNED_REQUEST_DOMAINS.usernameUpdate, signed));
            return { ...profile, username: signed.request.username };
        },
        requestDeletion: async signed => {
            calls.push('delete-write');
            assert.deepEqual(signed.request, { schema_version: 2, public_key: ownerKey, signed_at_unix_ms: clock(), target });
            assert.ok(verifies(SIGNED_REQUEST_DOMAINS.deletionRequest, signed));
            return { requestId: 'delete-1', target: signed.request.target, tombstonedAtUnixMs: 1, purgeEligibleAtUnixMs: null };
        },
        submissionOwnerStatus: async signed => {
            calls.push('owner-status-read');
            assert.ok(verifies(SIGNED_REQUEST_DOMAINS.submissionOwnerStatus, signed));
            return {
                submissionId: signed.request.submission_id, publicKey: signed.request.public_key,
                requestSha256: canonicalDocumentSha256Sync(signed), lifecycle: { state: 'queued' },
            };
        },
    };
}

test('owner controllers sign timestamped claims in one step and send only validated signed documents', async () => {
    const calls: string[] = [];
    const signal = new AbortController().signal;
    assert.equal((await updateUsername(api(calls), bridge, profile, 'after', signal, () => {}, clock)).username, 'after');
    assert.deepEqual((await deleteRecord(api(calls), bridge, target, signal, () => {}, clock)).target, target);
    assert.equal((await submissionOwnerStatus(api(calls), bridge, 'sub-1', signal, clock)).submissionId, 'sub-1');
    assert.deepEqual(calls, ['rename-write', 'delete-write', 'owner-status-read']);
});

test('signer claim substitutions and malformed signed documents never reach account requests', async () => {
    const calls: string[] = [];
    const signal = new AbortController().signal;
    const tamper = (domain: string, change: (claim: Record<string, unknown>) => void) => async (json: string) => {
        const claim = JSON.parse(json) as Record<string, unknown>;
        change(claim);
        return signer(domain)(JSON.stringify(claim));
    };
    const substituted: LeaderboardSigningBridge = {
        ...bridge,
        signUsernameUpdate: tamper(SIGNED_REQUEST_DOMAINS.usernameUpdate, claim => { claim.username = 'attacker'; }),
        signDeletionRequest: tamper(SIGNED_REQUEST_DOMAINS.deletionRequest, claim => { claim.target = { kind: 'run', run_id: 'other' }; }),
        signSubmissionOwnerStatus: tamper(SIGNED_REQUEST_DOMAINS.submissionOwnerStatus, claim => { claim.signed_at_unix_ms = 1; }),
    };
    await assert.rejects(updateUsername(api(calls), substituted, profile, 'after', signal, () => {}, clock), /changed a signed rename/u);
    await assert.rejects(deleteRecord(api(calls), substituted, target, signal, () => {}, clock), /changed a signed deletion/u);
    await assert.rejects(submissionOwnerStatus(api(calls), substituted, 'sub-1', signal, clock), /changed a signed owner-status/u);
    const legacy: LeaderboardSigningBridge = {
        ...bridge,
        signDeletionRequest: async json => JSON.stringify({
            schema_version: 1, challenge: JSON.parse(json) as unknown, signature: '34'.repeat(64),
        }),
    };
    await assert.rejects(deleteRecord(api(calls), legacy, target, signal, () => {}, clock), /schema_version must be 2|unknown field/u);
    assert.deepEqual(calls, []);
});

test('navigation cancels pending signer work and prevents subsequent privileged requests', async () => {
    for (const operation of ['rename', 'delete', 'owner-status']) {
        const calls: string[] = [], controller = new AbortController();
        let entered!: () => void;
        const signing = new Promise<void>(resolve => { entered = resolve; });
        const wait = (): Promise<string> => { entered(); return new Promise(() => {}); };
        const stalled = { ...bridge, signUsernameUpdate: wait, signDeletionRequest: wait, signSubmissionOwnerStatus: wait };
        const pending = operation === 'rename'
            ? updateUsername(api(calls), stalled, profile, 'after', controller.signal, () => {}, clock)
            : operation === 'delete'
                ? deleteRecord(api(calls), stalled, target, controller.signal, () => {}, clock)
                : submissionOwnerStatus(api(calls), stalled, 'sub-1', controller.signal, clock);
        const rejection = assert.rejects(pending, { name: 'AbortError' });
        await signing; controller.abort(); await rejection;
        assert.deepEqual(calls, []);
    }
});

test('public name/report validation rejects byte overflow and controls before network work', async () => {
    for (const name of ['', ' x', 'x ', '😀'.repeat(13), 'x‮y', 'x\ny']) assert.notEqual(usernameValidationError(name), null);
    assert.equal(usernameValidationError('😀'.repeat(12)), null);
    let submitted = false;
    const reporter = { report: async () => { submitted = true; return { reportId: 'r', receivedAtUnixMs: 1 }; } };
    await assert.rejects(reportRecord(reporter, target, 'other', '😀'.repeat(501), new AbortController().signal), /2000 bytes/u);
    assert.equal(submitted, false);
    assert.equal((await reportRecord(reporter, target, 'other', 'details', new AbortController().signal)).reportId, 'r');
});
