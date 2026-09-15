//! Host-neutral wire types for replay submission and verification.
//!
//! This crate deliberately does not depend on the game engine, an HTTP
//! framework, a database, or a particular hosting provider. It owns only the
//! documents which the client, the leaderboard service and the replay
//! verifier exchange. It never treats a client claim as a verified result:
//! the verifier resimulates every uploaded replay.

#[cfg(feature = "authentication")]
pub mod authentication;
pub mod board;
pub mod diagnostics;
pub mod moderation;
pub mod query;
pub mod rejection_code;
pub mod signed_request;
pub mod strict_json;
pub mod submission;
pub mod verification;

// Plain run-identity modules live in the `robin_run_types` leaf so the
// deterministic engine can use them without depending on this crate.
pub use robin_run_types::{canonical, digest, validation};

#[cfg(feature = "authentication")]
pub use authentication::{SignatureVerificationError, verify_ed25519_strict};
pub use board::{
    BoardMetricV1, BoardMissionV2, BoardV2, LeaderboardMetadataV2, TickDurationV1,
    ViewerContentRequirementV2,
};
pub use canonical::{
    CanonicalDocument, CanonicalDocumentError, CanonicalError, CanonicalValue, DomainSignedClaim,
    canonical_json_bytes,
};
pub use digest::{
    Digest32, HexError, OpaqueId, PublicKey32, Signature64, SimulationSeed64, SimulationSeedError,
};
pub use moderation::{
    AbuseReportAcceptedV1, AbuseReportCategoryV1, AbuseReportTargetV1, AbuseReportV1,
    DELETION_REQUEST_SIGNATURE_DOMAIN_V2, DeletionReceiptV1, DeletionRequestV2, DeletionTargetV1,
    SignedDeletionRequestV2,
};
pub use query::{
    AchievementSummaryV1, BoardMetricValueV2, LeaderboardCursorV2, LeaderboardEntryV2,
    LeaderboardOrderAnchorV2, LeaderboardPageV2, LeaderboardQueryV2, PlayerPersonalBestV2,
    PlayerProfileV1, PlayerRunHistoryEntryV2, PlayerRunHistoryFilterV1, PlayerRunHistoryPageV2,
    PlayerRunHistoryQueryV1, PublicAchievementDecisionV1, PublicParticipantV1,
    PublicSubmissionStateV1, PublicSubmissionStatusV1, RunDetailV2, RunFilterV2, RunMetricsV1,
    RunSummaryV2, SUBMISSION_OWNER_STATUS_SIGNATURE_DOMAIN_V2,
    SignedSubmissionOwnerStatusRequestV2, SubmissionAcceptedV1, SubmissionFailureCodeV1,
    SubmissionLifecycleV1, SubmissionOwnerStatusRequestV2, SubmissionOwnerStatusResponseV2,
    ViewerAvailabilityV2, ViewerLaunchV2,
};
pub use robin_run_types::{
    ArtifactRefV1, BoardSimulationPolicyV1, MAX_PARTICIPANT_INSTANCES_V1, MAX_REPLAY_SEATS_V1,
    OfficialContentEditionV1, RANKED_SIMULATION_POLICY_VERSION_V1, RankedSimulationDifficultyV1,
    RankedSimulationPolicyV1, RankedSimulationPresetV1, SCHEMA_VERSION_V1, SCHEMA_VERSION_V2,
};
#[cfg(feature = "authentication")]
pub use signed_request::SignedRequestError;
pub use signed_request::{
    SIGNED_REQUEST_DEFAULT_MAX_AGE_MS, SIGNED_REQUEST_DEFAULT_MAX_FUTURE_SKEW_MS,
    SignatureAlgorithmV1, SignedRequestClaim, SignedRequestFreshnessError, SignedRequestV2,
    SignedRequestWindowV1,
};
pub use submission::{
    InputIneligibilityReasonV1, InputProvenanceStatusV1, InputTaintKindV1, InputTaintV1,
    ParticipantPublicDisclosureV1, RANKED_REPLAY_MEDIA_TYPE_V1, ReplayArtifactV1,
    SUBMISSION_SIGNATURE_DOMAIN_V2, SignedSubmissionV2, SignedUsernameUpdateV2, SubmissionV2,
    TerminalOutcomeV1, USERNAME_UPDATE_SIGNATURE_DOMAIN_V2, UsernameUpdateV2,
};
pub use validation::{Validate, ValidationError};
pub use verification::{
    AchievementPolicyModeV1, AchievementPolicyV1, MAX_VERIFIER_JOB_BYTES_V2,
    VerificationInfrastructureFailureCodeV1, VerificationInfrastructureFailureV1,
    VerificationLimitsV1, VerificationRejectionCodeV1, VerificationRejectionV1,
    VerificationStatusV2, VerifiedAchievementEvaluationV1, VerifiedAchievementV1, VerifiedRunV2,
    VerifierJobV2, VerifierOutputV2, official_achievement_policies_v1,
    validate_authoritative_achievements,
};

/// Exact SQLx migration level shared by the high-score runtime and its
/// deployment scripts.
pub const HIGHSCORES_DATABASE_SCHEMA_VERSION: i64 = 6;

/// Exact replay schema whose compact bitcode bytes are simultaneously the
/// submitted, verifier-resimulated, retained, and publicly downloadable
/// artifact. Older schemas are rejected rather than normalized.
pub const CURRENT_RANKED_REPLAY_SCHEMA_VERSION_V1: u32 = 44;

/// Exact multiplayer wire protocol of the verifier and current clients. A
/// replay recorded under another network protocol is not rankable.
pub const CURRENT_RANKED_NETWORK_PROTOCOL_VERSION_V1: u32 = 53;

#[cfg(test)]
mod tests {
    #[test]
    fn every_signature_domain_is_distinct_and_terminated() {
        let domains = [
            <crate::SubmissionV2 as crate::SignedRequestClaim>::DOMAIN,
            <crate::UsernameUpdateV2 as crate::SignedRequestClaim>::DOMAIN,
            <crate::DeletionRequestV2 as crate::SignedRequestClaim>::DOMAIN,
            <crate::SubmissionOwnerStatusRequestV2 as crate::SignedRequestClaim>::DOMAIN,
        ];
        let mut seen = std::collections::BTreeSet::new();
        for domain in domains {
            assert_eq!(domain.last(), Some(&0));
            assert!(seen.insert(domain), "duplicate signature domain");
        }
    }
}
