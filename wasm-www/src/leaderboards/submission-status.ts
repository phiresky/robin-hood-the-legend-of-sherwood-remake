import { opaqueId, strictObject, versionedObject } from './decode.js';

export type PublicSubmissionStatus = {
    readonly submissionId: string;
    readonly state: 'queued' | 'verifying' | 'retry_pending' | 'verified' | 'rejected' | 'failed';
    readonly runId: string | null;
};

export function parsePublicSubmissionStatus(value: unknown, requestedId: string): PublicSubmissionStatus {
    const root = versionedObject(value, 'submission status', ['submission_id', 'state']);
    const submissionId = opaqueId(root.submission_id, 'submission_id');
    if (submissionId !== requestedId) throw new Error('Submission status does not match the requested ID.');
    const raw = root.state;
    if (raw === null || typeof raw !== 'object' || Array.isArray(raw)) throw new Error('Invalid submission state.');
    const state = (raw as Record<string, unknown>).state;
    if (state === 'verified') {
        const obj = strictObject(raw, 'verified state', ['state', 'run_id']);
        return { submissionId, state, runId: opaqueId(obj.run_id, 'run_id') };
    }
    if (state !== 'queued' && state !== 'verifying' && state !== 'retry_pending' && state !== 'rejected' && state !== 'failed') {
        throw new Error('Unknown submission state.');
    }
    strictObject(raw, 'submission state', ['state']);
    return { submissionId, state, runId: null };
}

export function submissionPresentation(status: PublicSubmissionStatus): { title: string; message: string; pending: boolean } {
    switch (status.state) {
        case 'queued': return { title: 'Unverified — queued', message: 'Your replay was received and is waiting for verification. It is not on the leaderboard yet.', pending: true };
        case 'verifying': return { title: 'Unverified — verifying', message: 'The server is replaying this recording and checking its result. This page updates automatically.', pending: true };
        case 'retry_pending': return { title: 'Unverified — retry pending', message: 'Verification was interrupted. The server will retry automatically.', pending: true };
        case 'verified': return { title: 'Verified', message: 'Verification completed. Open the verified result to see its score, settings and replay.', pending: false };
        case 'rejected': return { title: 'Not verified', message: 'This replay did not pass verification. It has not earned a leaderboard result.', pending: false };
        case 'failed': return { title: 'Verification failed', message: 'The server could not complete verification. This submission has not earned a leaderboard result.', pending: false };
    }
}
