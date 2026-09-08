// Public account, username and moderation wire contracts.
import {
    type PlayerProfile,
    type UsernameChallenge,
    type UsernameUpdateEnvelope,
    type DeletionChallenge,
    type DeletionRequestEnvelope,
    type DeletionReceipt,
    type AbuseReportAccepted,
    type DeletionTarget,
} from './types.js';
import {
    versionedObject,
    publicKey,
    boundedString,
    opaqueId,
    nonzeroHex,
    positiveUnixMilliseconds,
    object,
    enumeration,
    assertExactKeys,
} from './decode.js';
import { validatePublicFingerprint } from './participants-metrics.js';

export function parsePlayerProfile(value: unknown): PlayerProfile {
    const obj = versionedObject(value, 'player', ['username', 'public_key', 'public_key_fingerprint']);
    const key = publicKey(obj.public_key, 'player.public_key');
    return {
        username: boundedString(obj.username, 'player.username', 48),
        publicKey: key,
        publicKeyFingerprint: validatePublicFingerprint(obj.public_key_fingerprint, key, 'player.public_key_fingerprint'),
    };
}

export function parseUsernameChallenge(value: unknown): UsernameChallenge {
    const obj = versionedObject(value, 'username_challenge', [
        'username_challenge_id', 'username_challenge_nonce', 'expires_at_unix_ms',
    ]);
    return {
        id: opaqueId(obj.username_challenge_id, 'username_challenge.username_challenge_id'),
        nonce: nonzeroHex(obj.username_challenge_nonce, 'username_challenge.username_challenge_nonce', 64),
        expiresAtUnixMs: positiveUnixMilliseconds(obj.expires_at_unix_ms, 'username_challenge.expires_at_unix_ms'),
    };
}

export function parseUsernameUpdateEnvelope(value: unknown): UsernameUpdateEnvelope {
    const obj = versionedObject(value, 'username_update', [
        'username_challenge_id', 'username_challenge_nonce', 'public_key', 'username', 'signature',
    ]);
    return {
        schema_version: 1,
        username_challenge_id: opaqueId(obj.username_challenge_id, 'username_update.username_challenge_id'),
        username_challenge_nonce: nonzeroHex(
            obj.username_challenge_nonce,
            'username_update.username_challenge_nonce',
            64,
        ),
        public_key: publicKey(obj.public_key, 'username_update.public_key'),
        username: boundedString(obj.username, 'username_update.username', 48),
        signature: nonzeroHex(obj.signature, 'username_update.signature', 128),
    };
}

export function parseDeletionChallenge(value: unknown): DeletionChallenge {
    const obj = versionedObject(value, 'deletion_challenge', [
        'deletion_challenge_id', 'deletion_challenge_nonce', 'expires_at_unix_ms', 'public_key', 'target',
    ]);
    return {
        schema_version: 1,
        deletion_challenge_id: opaqueId(obj.deletion_challenge_id, 'deletion_challenge.deletion_challenge_id'),
        deletion_challenge_nonce: nonzeroHex(
            obj.deletion_challenge_nonce,
            'deletion_challenge.deletion_challenge_nonce',
            64,
        ),
        expires_at_unix_ms: positiveUnixMilliseconds(
            obj.expires_at_unix_ms,
            'deletion_challenge.expires_at_unix_ms',
        ),
        public_key: publicKey(obj.public_key, 'deletion_challenge.public_key'),
        target: parseDeletionTarget(obj.target, 'deletion_challenge.target'),
    };
}

export function parseDeletionRequestEnvelope(value: unknown): DeletionRequestEnvelope {
    const obj = versionedObject(value, 'deletion_request', ['challenge', 'signature']);
    return {
        schema_version: 1,
        challenge: parseDeletionChallenge(obj.challenge),
        signature: nonzeroHex(obj.signature, 'deletion_request.signature', 128),
    };
}

export function parseDeletionReceipt(value: unknown): DeletionReceipt {
    const obj = versionedObject(value, 'deletion_receipt', [
        'deletion_request_id', 'target', 'tombstoned_at_unix_ms', 'purge_eligible_at_unix_ms',
    ]);
    const tombstonedAtUnixMs = positiveUnixMilliseconds(
        obj.tombstoned_at_unix_ms,
        'deletion_receipt.tombstoned_at_unix_ms',
    );
    const purgeEligibleAtUnixMs = obj.purge_eligible_at_unix_ms === null
        ? null
        : positiveUnixMilliseconds(obj.purge_eligible_at_unix_ms, 'deletion_receipt.purge_eligible_at_unix_ms');
    if (purgeEligibleAtUnixMs !== null && purgeEligibleAtUnixMs <= tombstonedAtUnixMs) {
        throw new Error('deletion_receipt purge eligibility must follow its tombstone time');
    }
    return {
        requestId: opaqueId(obj.deletion_request_id, 'deletion_receipt.deletion_request_id'),
        target: parseDeletionTarget(obj.target, 'deletion_receipt.target'),
        tombstonedAtUnixMs,
        purgeEligibleAtUnixMs,
    };
}

export function parseAbuseReportAccepted(value: unknown): AbuseReportAccepted {
    const obj = versionedObject(value, 'report', ['report_id', 'received_at_unix_ms']);
    return {
        reportId: opaqueId(obj.report_id, 'report.report_id'),
        receivedAtUnixMs: positiveUnixMilliseconds(obj.received_at_unix_ms, 'report.received_at_unix_ms'),
    };
}

export function parseDeletionTarget(value: unknown, path: string): DeletionTarget {
    const obj = object(value, path);
    const kind = enumeration(obj.kind, ['run', 'submission'] as const, `${path}.kind`);
    if (kind === 'run') {
        assertExactKeys(obj, path, ['kind', 'run_id']);
        return { kind, run_id: opaqueId(obj.run_id, `${path}.run_id`) };
    }
    assertExactKeys(obj, path, ['kind', 'submission_id']);
    return { kind, submission_id: opaqueId(obj.submission_id, `${path}.submission_id`) };
}
