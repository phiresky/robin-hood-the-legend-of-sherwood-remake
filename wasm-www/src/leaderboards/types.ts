// Normalized leaderboard protocol V2 values (robin_run_protocol::{board, query}).
// Wire input is validated by public-response.ts and account-contract.ts.

export type BoardMetric = 'original_score' | 'fastest_success';

export type ContentEdition = 'demo' | 'full';

export type ViewerContentRequirement = 'bundled_demo' | 'user_local_retail';

export type TickDuration = { readonly numeratorMicros: number; readonly denominator: number };

export type RankedSimulationPolicy = {
    readonly version: 1;
    readonly preset: 'standard' | 'original_parity' | 'custom';
    readonly difficulty: 'easy' | 'medium' | 'hard' | 'legendary' | 'custom';
};

export type BoardSimulationPolicy =
    | { readonly kind: 'fixed'; readonly policy: RankedSimulationPolicy }
    | { readonly kind: 'any_config' };

export type BoardMission = { readonly missionId: string; readonly displayName: string };

export type Board = {
    readonly boardId: string;
    readonly displayName: string;
    readonly edition: ContentEdition;
    readonly presetId: string;
    readonly presetName: string;
    readonly difficultyId: string;
    readonly difficultyName: string;
    readonly simulationPolicy: BoardSimulationPolicy;
    readonly allowStateLoad: boolean;
    readonly metrics: readonly BoardMetric[];
    readonly viewerContentRequirement: ViewerContentRequirement;
    readonly missions: readonly BoardMission[];
};

export type BoardMetadata = {
    readonly tickDuration: TickDuration;
    readonly boards: readonly Board[];
};

export type PublicParticipant = {
    readonly seat: number;
    readonly username: string;
    readonly publicKey: string;
    readonly publicKeyFingerprint: string;
};

export type BoardMetricValue =
    | { readonly metric: 'original_score'; readonly points: number }
    | { readonly metric: 'fastest_success'; readonly activeSimulationTicks: number };

export type RunMetrics = {
    readonly originalScoreDelta: number;
    readonly activeSimulationTicks: number;
    readonly ransomCollected: number;
};

export type RunFilter = {
    readonly boardId: string;
    readonly missionId: string;
    readonly metric: BoardMetric;
    readonly maxConcurrentPlayers: number | null;
    readonly playerPublicKey: string | null;
};

export type LeaderboardEntry = {
    readonly position: number;
    readonly rank: number;
    readonly runId: string;
    readonly metricValue: BoardMetricValue;
    readonly maxConcurrentPlayers: number;
    readonly participantInstanceCount: number;
    /** Named uploader, or null when the uploader chose anonymous disclosure. */
    readonly uploader: PublicParticipant | null;
    readonly replaySha256: string;
    readonly acceptedSequence: number;
    readonly verifiedAtUnixMs: number;
};

export type LeaderboardOrderAnchor = {
    readonly position: number;
    readonly rank: number;
    readonly metricValue: BoardMetricValue;
    readonly acceptedSequence: number;
    readonly verifiedAtUnixMs: number;
    readonly runId: string;
};

export type LeaderboardCursor = {
    readonly querySha256: string;
    readonly acceptedSequenceWatermark: number;
    readonly last: LeaderboardOrderAnchor;
    readonly opaqueToken: string;
};

export type BoardPage = {
    readonly filter: RunFilter;
    readonly entries: readonly LeaderboardEntry[];
    readonly acceptedSequenceWatermark: number;
    readonly previousCursor: LeaderboardCursor | null;
    readonly nextCursorDocument: LeaderboardCursor | null;
    readonly nextCursor: string | null;
};

export type RunSummary = {
    readonly runId: string;
    readonly boardId: string;
    readonly missionId: string;
    readonly maxConcurrentPlayers: number;
    readonly participantInstanceCount: number;
    readonly uploader: PublicParticipant | null;
    readonly metrics: RunMetrics;
};

export type PlayerRunHistoryEntry = {
    readonly playerPublicKey: string;
    readonly run: RunSummary;
    readonly verifiedAtUnixMs: number;
};

export type PlayerPersonalBest = {
    readonly filter: RunFilter & { readonly playerPublicKey: string };
    readonly runId: string;
    readonly metricValue: BoardMetricValue;
};

export type PlayerRunHistoryPage = {
    readonly player: PlayerProfile;
    readonly acceptedSequenceWatermark: number;
    readonly runs: readonly PlayerRunHistoryEntry[];
    readonly personalBests: readonly PlayerPersonalBest[];
    readonly nextCursor: string | null;
};

export type Achievement = {
    readonly id: string;
    readonly label: string;
    readonly evaluation: 'unverifiable' | 'not_earned' | 'earned';
};

export type ArtifactRef = { readonly sha256: string; readonly byteLength: number; readonly mediaType: string };

export type ReplayArtifact = {
    readonly artifact: ArtifactRef;
    readonly replaySchemaVersion: number;
};

export type ViewerLaunch = {
    readonly availability:
        | { readonly status: 'available' }
        | { readonly status: 'unavailable'; readonly safeReason: string };
    readonly contentRequirement: ViewerContentRequirement;
    /** Runtime build under `/wasm/<build>/`; equals the replay's recorded engine version. */
    readonly runtimeBuild: string;
};

export type CanonicalValue = null | boolean | number | string | readonly CanonicalValue[] | {
    readonly [key: string]: CanonicalValue;
};

export type RunDetail = {
    readonly runId: string;
    readonly boardId: string;
    readonly missionId: string;
    readonly edition: ContentEdition;
    readonly metrics: RunMetrics;
    readonly maxConcurrentPlayers: number;
    readonly participantInstanceCount: number;
    readonly uploader: PublicParticipant | null;
    readonly verifiedAtUnixMs: number;
    readonly replay: ReplayArtifact;
    readonly recordedEngineVersion: string;
    readonly simConfig: Readonly<Record<string, CanonicalValue>>;
    readonly startingCampaignScore: number;
    readonly finalCampaignScore: number;
    readonly achievements: readonly Achievement[];
    readonly viewer: ViewerLaunch;
};

export type PlayerProfile = { readonly username: string; readonly publicKey: string; readonly publicKeyFingerprint: string };

/**
 * robin_run_protocol::signed_request::SignedRequestV2. The player signs
 * `domain || canonical_json(request)`; `signed_at_unix_ms` bounds replay.
 */
export type SignedRequest<T> = {
    readonly schema_version: 2;
    readonly request: T;
    readonly algorithm: 'ed25519';
    readonly signature: string;
};

/** robin_run_protocol::UsernameUpdateV2 */
export type UsernameUpdateClaim = {
    readonly schema_version: 2;
    readonly public_key: string;
    readonly signed_at_unix_ms: number;
    readonly username: string;
};

export type SignedUsernameUpdate = SignedRequest<UsernameUpdateClaim>;

export type DeletionTarget =
    | { readonly kind: 'run'; readonly run_id: string }
    | { readonly kind: 'submission'; readonly submission_id: string };

/** robin_run_protocol::DeletionRequestV2 */
export type DeletionRequestClaim = {
    readonly schema_version: 2;
    readonly public_key: string;
    readonly signed_at_unix_ms: number;
    readonly target: DeletionTarget;
};

export type SignedDeletionRequest = SignedRequest<DeletionRequestClaim>;

/** robin_run_protocol::SubmissionOwnerStatusRequestV2 */
export type SubmissionOwnerStatusClaim = {
    readonly schema_version: 2;
    readonly public_key: string;
    readonly signed_at_unix_ms: number;
    readonly submission_id: string;
};

export type SignedSubmissionOwnerStatusRequest = SignedRequest<SubmissionOwnerStatusClaim>;

export type VerificationRejectionCode =
    | 'malformed_replay' | 'resource_limit' | 'unsupported_schema' | 'content_not_allowed'
    | 'config_mismatch' | 'starting_state_mismatch' | 'command_not_allowed' | 'timeline_invalid'
    | 'state_hash_mismatch' | 'terminal_invalid' | 'result_invariant_mismatch'
    | 'input_provenance_ineligible' | 'simulation_budget_exceeded';

/** robin_run_protocol::SubmissionLifecycleV1 */
export type SubmissionLifecycle =
    | { readonly state: 'queued' | 'verifying' | 'retry_pending' }
    | { readonly state: 'accepted'; readonly runId: string }
    | { readonly state: 'rejected'; readonly code: VerificationRejectionCode; readonly safeMessage: string }
    | { readonly state: 'failed'; readonly code: 'verification_infrastructure'; readonly safeMessage: string };

/** robin_run_protocol::SubmissionOwnerStatusResponseV2, bound to the request it answers. */
export type SubmissionOwnerStatus = {
    readonly submissionId: string;
    readonly publicKey: string;
    readonly requestSha256: string;
    readonly lifecycle: SubmissionLifecycle;
};

export type DeletionReceipt = {
    readonly requestId: string;
    readonly target: DeletionTarget;
    readonly tombstonedAtUnixMs: number;
    readonly purgeEligibleAtUnixMs: number | null;
};

export type AbuseReportCategory =
    | 'suspected_cheating' | 'offensive_identity' | 'privacy' | 'copyright' | 'other';

export type AbuseReportAccepted = { readonly reportId: string; readonly receivedAtUnixMs: number };

export type JsonObject = Readonly<Record<string, unknown>>;
