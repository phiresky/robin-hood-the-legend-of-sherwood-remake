import type { HighscoreApi } from './api.js';
import { withAbort } from '../cancellation.js';
import type { LeaderboardSigningBridge } from './signing.js';
import type {
    PlayerProfile,
    DeletionTarget,
    AbuseReportCategory,
    DeletionRequestClaim,
    SubmissionOwnerStatusClaim,
    UsernameUpdateClaim,
} from './types.js';
import {
    parseSignedDeletionRequest,
    parseSignedSubmissionOwnerStatusRequest,
    parseSignedUsernameUpdate,
} from './account-contract.js';
import { canonicalJson } from './canonical.js';

/** Clock used for `signed_at_unix_ms`; injectable for tests. */
export type SigningClock = () => number;

/** A route owns the entire sign/write operation, including unabortable signer awaits. */
export async function updateUsername(
    api: Pick<HighscoreApi, 'updateUsername'>,
    bridge: LeaderboardSigningBridge, profile: PlayerProfile, username: string,
    signal: AbortSignal, status: (message: string) => void, now: SigningClock = Date.now,
): Promise<PlayerProfile> {
    signal.throwIfAborted();
    const error = usernameValidationError(username);
    if (error !== null) throw new Error(error);
    const claim: UsernameUpdateClaim = {
        schema_version: 2, public_key: profile.publicKey, signed_at_unix_ms: now(), username,
    };
    status('Signing the typed rename request…');
    const document = await withAbort(signal, () => bridge.signUsernameUpdate(JSON.stringify(claim)));
    signal.throwIfAborted();
    const signed = parseSignedUsernameUpdate(JSON.parse(document) as unknown);
    assertSignedClaim(signed.request, claim, 'rename');
    const updated = await api.updateUsername(profile.publicKey, signed, signal);
    signal.throwIfAborted();
    if (updated.publicKey !== profile.publicKey || updated.username !== username) {
        throw new Error('The server returned a different player profile after rename.');
    }
    return updated;
}

export async function deleteRecord(
    api: Pick<HighscoreApi, 'requestDeletion'>,
    bridge: LeaderboardSigningBridge, target: DeletionTarget,
    signal: AbortSignal, status: (message: string) => void, now: SigningClock = Date.now,
) {
    signal.throwIfAborted();
    const publicKey = await withAbort(signal, () => bridge.publicKey());
    signal.throwIfAborted();
    const claim: DeletionRequestClaim = {
        schema_version: 2, public_key: publicKey, signed_at_unix_ms: now(), target,
    };
    status('Signing the exact deletion target…');
    const document = await withAbort(signal, () => bridge.signDeletionRequest(JSON.stringify(claim)));
    signal.throwIfAborted();
    const signed = parseSignedDeletionRequest(JSON.parse(document) as unknown);
    assertSignedClaim(signed.request, claim, 'deletion');
    const receipt = await api.requestDeletion(signed, signal);
    signal.throwIfAborted();
    if (!sameDeletionTarget(receipt.target, target)) {
        throw new Error('The server returned a deletion receipt for a different target.');
    }
    return receipt;
}

/** Owner-signed private lifecycle read for one submission. */
export async function submissionOwnerStatus(
    api: Pick<HighscoreApi, 'submissionOwnerStatus'>,
    bridge: LeaderboardSigningBridge, submissionId: string,
    signal: AbortSignal, now: SigningClock = Date.now,
) {
    signal.throwIfAborted();
    const publicKey = await withAbort(signal, () => bridge.publicKey());
    signal.throwIfAborted();
    const claim: SubmissionOwnerStatusClaim = {
        schema_version: 2, public_key: publicKey, signed_at_unix_ms: now(), submission_id: submissionId,
    };
    const document = await withAbort(signal, () => bridge.signSubmissionOwnerStatus(JSON.stringify(claim)));
    signal.throwIfAborted();
    const signed = parseSignedSubmissionOwnerStatusRequest(JSON.parse(document) as unknown);
    assertSignedClaim(signed.request, claim, 'owner-status');
    const response = await api.submissionOwnerStatus(signed, signal);
    signal.throwIfAborted();
    return response;
}

export async function reportRecord(
    api: Pick<HighscoreApi, 'report'>,
    target: Parameters<HighscoreApi['report']>[0], category: AbuseReportCategory, detail: string, signal: AbortSignal,
) {
    signal.throwIfAborted();
    if (detail.trim() !== detail || detail.length === 0 || new TextEncoder().encode(detail).byteLength > 2000) {
        throw new Error('Enter 1–2000 bytes without surrounding whitespace.');
    }
    const receipt = await api.report(target, category, detail, signal);
    signal.throwIfAborted();
    return receipt;
}

export function usernameValidationError(username: string): string | null {
    const byteLength = new TextEncoder().encode(username).byteLength;
    if (byteLength === 0 || byteLength > 48) return 'Enter a display name between 1 and 48 UTF-8 bytes.';
    if (username.trim() !== username) return 'Remove surrounding whitespace.';
    if (/\p{Cc}|\p{Bidi_Control}/u.test(username)) return 'Control and bidirectional override characters are not allowed.';
    return null;
}

/** The signer must return a signature over exactly the claim it was given. */
export function assertSignedClaim(signed: unknown, requested: unknown, label: string): void {
    if (canonicalJson(signed, 'signed claim', 0) !== canonicalJson(requested, 'requested claim', 0)) {
        throw new Error(`The identity signer changed a signed ${label} claim.`);
    }
}

export function sameDeletionTarget(left: DeletionTarget, right: DeletionTarget): boolean {
    return left.kind === right.kind && (left.kind === 'run'
        ? left.run_id === (right.kind === 'run' ? right.run_id : undefined)
        : left.submission_id === (right.kind === 'submission' ? right.submission_id : undefined));
}
