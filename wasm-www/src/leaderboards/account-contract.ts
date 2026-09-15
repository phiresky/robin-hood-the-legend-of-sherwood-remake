// Public account, username and moderation wire contracts.
import {
    type PlayerProfile,
    type SignedRequest,
    type UsernameUpdateClaim,
    type SignedUsernameUpdate,
    type DeletionRequestClaim,
    type SignedDeletionRequest,
    type SubmissionOwnerStatusClaim,
    type SignedSubmissionOwnerStatusRequest,
    type SubmissionLifecycle,
    type SubmissionOwnerStatus,
    type VerificationRejectionCode,
    type DeletionReceipt,
    type AbuseReportAccepted,
    type DeletionTarget,
} from './types.js';
import {
    versionedObject,
    versionedObjectV2,
    strictObject,
    publicKey,
    boundedString,
    opaqueId,
    nonzeroHex,
    nonzeroSha256,
    positiveUnixMilliseconds,
    object,
    enumeration,
    assertExactKeys,
} from './decode.js';
import { canonicalDocumentSha256Sync, canonicalJson } from './canonical.js';
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

/** NUL-terminated per-operation signing domains (robin_run_protocol `*_SIGNATURE_DOMAIN_V2`). */
export const SIGNED_REQUEST_DOMAINS = Object.freeze({
    /** SubmissionV3 claim (schema_version 3); the signed wrapper stays schema_version 2. */
    submission: 'robinhood/leaderboards/3/submission\0',
    usernameUpdate: 'robinhood/leaderboards/2/username-update\0',
    deletionRequest: 'robinhood/leaderboards/2/deletion-request\0',
    submissionOwnerStatus: 'robinhood/leaderboards/2/submission-owner-status\0',
});

/** Mirrors `SignedRequestV2::signing_bytes`: ASCII domain followed by the canonical claim. */
export function signedRequestSigningBytes(domain: string, claim: unknown): Uint8Array {
    return new TextEncoder().encode(`${domain}${canonicalJson(claim, 'signed_request.request', 0)}`);
}

export function parseUsernameUpdateClaim(value: unknown): UsernameUpdateClaim {
    const obj = versionedObjectV2(value, 'username_update', ['public_key', 'signed_at_unix_ms', 'username']);
    return {
        schema_version: 2,
        public_key: publicKey(obj.public_key, 'username_update.public_key'),
        signed_at_unix_ms: positiveUnixMilliseconds(obj.signed_at_unix_ms, 'username_update.signed_at_unix_ms'),
        username: boundedString(obj.username, 'username_update.username', 48),
    };
}

export function parseDeletionRequestClaim(value: unknown): DeletionRequestClaim {
    const obj = versionedObjectV2(value, 'deletion_request', ['public_key', 'signed_at_unix_ms', 'target']);
    return {
        schema_version: 2,
        public_key: publicKey(obj.public_key, 'deletion_request.public_key'),
        signed_at_unix_ms: positiveUnixMilliseconds(obj.signed_at_unix_ms, 'deletion_request.signed_at_unix_ms'),
        target: parseDeletionTarget(obj.target, 'deletion_request.target'),
    };
}

export function parseSubmissionOwnerStatusClaim(value: unknown): SubmissionOwnerStatusClaim {
    const obj = versionedObjectV2(value, 'submission_owner_status', [
        'public_key', 'signed_at_unix_ms', 'submission_id',
    ]);
    return {
        schema_version: 2,
        public_key: publicKey(obj.public_key, 'submission_owner_status.public_key'),
        signed_at_unix_ms: positiveUnixMilliseconds(
            obj.signed_at_unix_ms,
            'submission_owner_status.signed_at_unix_ms',
        ),
        submission_id: opaqueId(obj.submission_id, 'submission_owner_status.submission_id'),
    };
}

function parseSignedRequest<T>(
    value: unknown,
    path: string,
    parseClaim: (claim: unknown) => T,
): SignedRequest<T> {
    const obj = versionedObjectV2(value, path, ['request', 'algorithm', 'signature']);
    return {
        schema_version: 2,
        request: parseClaim(obj.request),
        algorithm: enumeration(obj.algorithm, ['ed25519'] as const, `${path}.algorithm`),
        signature: nonzeroHex(obj.signature, `${path}.signature`, 128),
    };
}

export function parseSignedUsernameUpdate(value: unknown): SignedUsernameUpdate {
    return parseSignedRequest(value, 'signed_username_update', parseUsernameUpdateClaim);
}

export function parseSignedDeletionRequest(value: unknown): SignedDeletionRequest {
    return parseSignedRequest(value, 'signed_deletion_request', parseDeletionRequestClaim);
}

export function parseSignedSubmissionOwnerStatusRequest(value: unknown): SignedSubmissionOwnerStatusRequest {
    return parseSignedRequest(value, 'signed_submission_owner_status', parseSubmissionOwnerStatusClaim);
}

const REJECTION_CODES = [
    'malformed_replay', 'resource_limit', 'unsupported_schema', 'content_not_allowed',
    'config_mismatch', 'starting_state_mismatch', 'command_not_allowed', 'timeline_invalid',
    'state_hash_mismatch', 'terminal_invalid', 'result_invariant_mismatch',
    'input_provenance_ineligible', 'simulation_budget_exceeded',
] as const satisfies readonly VerificationRejectionCode[];

export function parseSubmissionLifecycle(value: unknown, path: string): SubmissionLifecycle {
    const obj = object(value, path);
    const state = enumeration(obj.state, [
        'queued', 'verifying', 'retry_pending', 'accepted', 'rejected', 'failed',
    ] as const, `${path}.state`);
    switch (state) {
        case 'queued':
        case 'verifying':
        case 'retry_pending':
            assertExactKeys(obj, path, ['state']);
            return { state };
        case 'accepted':
            strictObject(obj, path, ['state', 'run_id']);
            return { state, runId: opaqueId(obj.run_id, `${path}.run_id`) };
        case 'rejected':
            strictObject(obj, path, ['state', 'code', 'safe_message']);
            return {
                state,
                code: enumeration(obj.code, REJECTION_CODES, `${path}.code`),
                safeMessage: boundedString(obj.safe_message, `${path}.safe_message`, 500),
            };
        case 'failed':
            strictObject(obj, path, ['state', 'code', 'safe_message']);
            return {
                state,
                code: enumeration(obj.code, ['verification_infrastructure'] as const, `${path}.code`),
                safeMessage: boundedString(obj.safe_message, `${path}.safe_message`, 500),
            };
    }
}

/**
 * Parses `SubmissionOwnerStatusResponseV2` and binds it to the exact signed
 * request: submission, owner key and canonical request digest must all match.
 */
export function parseSubmissionOwnerStatusResponse(
    value: unknown,
    request: SignedSubmissionOwnerStatusRequest,
): SubmissionOwnerStatus {
    const obj = versionedObjectV2(value, 'submission_owner_status_response', [
        'submission_id', 'public_key', 'request_sha256', 'state',
    ]);
    const response: SubmissionOwnerStatus = {
        submissionId: opaqueId(obj.submission_id, 'submission_owner_status_response.submission_id'),
        publicKey: publicKey(obj.public_key, 'submission_owner_status_response.public_key'),
        requestSha256: nonzeroSha256(obj.request_sha256, 'submission_owner_status_response.request_sha256'),
        lifecycle: parseSubmissionLifecycle(obj.state, 'submission_owner_status_response.state'),
    };
    if (response.submissionId !== request.request.submission_id
        || response.publicKey !== request.request.public_key
        || response.requestSha256 !== canonicalDocumentSha256Sync(request)) {
        throw new Error('submission_owner_status_response does not answer the signed request');
    }
    return response;
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
