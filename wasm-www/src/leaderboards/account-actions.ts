import type { HighscoreApi } from './api.js';
import { withAbort } from '../cancellation.js';
import type { LeaderboardSigningBridge } from './signing.js';
import type { PlayerProfile, DeletionTarget, AbuseReportCategory } from './types.js';
import { parseUsernameUpdateEnvelope, parseDeletionRequestEnvelope } from './account-contract.js';

/** A route owns the entire challenge/sign/write operation, including unabortable signer awaits. */
export async function updateUsername(
    api: Pick<HighscoreApi, 'usernameChallenge' | 'updateUsername'>,
    bridge: LeaderboardSigningBridge, profile: PlayerProfile, username: string,
    signal: AbortSignal, status: (message: string) => void,
): Promise<PlayerProfile> {
    signal.throwIfAborted();
    const error = usernameValidationError(username);
    if (error !== null) throw new Error(error);
    const challenge = await api.usernameChallenge(profile.publicKey, signal);
    signal.throwIfAborted();
    const unsigned = {
        schema_version: 1, username_challenge_id: challenge.id, username_challenge_nonce: challenge.nonce,
        public_key: profile.publicKey, username, signature: '0'.repeat(128),
    } as const;
    status('Signing the typed rename request…');
    const document = await withAbort(signal, () => bridge.signUsernameUpdate(JSON.stringify(unsigned)));
    signal.throwIfAborted();
    const signed = parseUsernameUpdateEnvelope(JSON.parse(document) as unknown);
    assertUsernameSignatureClaim(signed, unsigned);
    const updated = await api.updateUsername(profile.publicKey, signed, signal);
    signal.throwIfAborted();
    if (updated.publicKey !== profile.publicKey || updated.username !== username) {
        throw new Error('The server returned a different player profile after rename.');
    }
    return updated;
}

export async function deleteRecord(
    api: Pick<HighscoreApi, 'deletionChallenge' | 'requestDeletion'>,
    bridge: LeaderboardSigningBridge, target: DeletionTarget,
    signal: AbortSignal, status: (message: string) => void,
) {
    signal.throwIfAborted();
    const publicKey = await withAbort(signal, () => bridge.publicKey());
    signal.throwIfAborted();
    const challenge = await api.deletionChallenge(publicKey, target, signal);
    signal.throwIfAborted();
    if (challenge.public_key !== publicKey || !sameDeletionTarget(challenge.target, target)) {
        throw new Error('The server returned a deletion challenge for a different owner or target.');
    }
    const unsigned = { schema_version: 1, challenge, signature: '0'.repeat(128) } as const;
    status('Signing the exact deletion target and server challenge…');
    const document = await withAbort(signal, () => bridge.signDeletionRequest(JSON.stringify(unsigned)));
    signal.throwIfAborted();
    const signed = parseDeletionRequestEnvelope(JSON.parse(document) as unknown);
    assertDeletionSignatureClaim(signed, unsigned);
    const receipt = await api.requestDeletion(signed, signal);
    signal.throwIfAborted();
    if (!sameDeletionTarget(receipt.target, target)) {
        throw new Error('The server returned a deletion receipt for a different target.');
    }
    return receipt;
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


export function assertUsernameSignatureClaim(
    signed: ReturnType<typeof parseUsernameUpdateEnvelope>,
    unsigned: Omit<ReturnType<typeof parseUsernameUpdateEnvelope>, 'signature'> & { readonly signature: string },
): void {
    if (signed.schema_version !== unsigned.schema_version
        || signed.username_challenge_id !== unsigned.username_challenge_id
        || signed.username_challenge_nonce !== unsigned.username_challenge_nonce
        || signed.public_key !== unsigned.public_key
        || signed.username !== unsigned.username) {
        throw new Error('The identity signer changed a signed rename claim.');
    }
}


export function assertDeletionSignatureClaim(
    signed: ReturnType<typeof parseDeletionRequestEnvelope>,
    unsigned: { readonly schema_version: 1; readonly challenge: ReturnType<typeof parseDeletionRequestEnvelope>['challenge'] },
): void {
    const a = signed.challenge;
    const b = unsigned.challenge;
    if (signed.schema_version !== unsigned.schema_version
        || a.schema_version !== b.schema_version
        || a.deletion_challenge_id !== b.deletion_challenge_id
        || a.deletion_challenge_nonce !== b.deletion_challenge_nonce
        || a.expires_at_unix_ms !== b.expires_at_unix_ms
        || a.public_key !== b.public_key
        || !sameDeletionTarget(a.target, b.target)) {
        throw new Error('The identity signer changed a signed deletion claim.');
    }
}


export function sameDeletionTarget(left: DeletionTarget, right: DeletionTarget): boolean {
    return left.kind === right.kind && (left.kind === 'run'
        ? left.run_id === (right.kind === 'run' ? right.run_id : undefined)
        : left.submission_id === (right.kind === 'submission' ? right.submission_id : undefined));
}
