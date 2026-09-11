//! Host-neutral wire types for replay submission and verification.
//!
//! This crate deliberately does not depend on the game engine, iroh, an HTTP
//! framework, a database, or a particular hosting provider.  It owns only the
//! canonical documents which those components exchange.  In particular, it
//! never treats a client claim as a verified game result.

mod authentication;
pub mod bitcode_value;
mod canonical;
mod digest;
mod envelope;
mod manifest;
mod moderation;
mod offer_binding;
mod query;
mod rejection_code;
pub mod strict_json;
mod validation;
mod verification_result;
mod verifier_job;

pub use authentication::{SignatureVerificationError, verify_ed25519_strict};
pub use offer_binding::validate_offer_binding;

pub use canonical::{
    CanonicalDocument, CanonicalDocumentError, CanonicalError, CanonicalValue, canonical_json_bytes,
};
pub use digest::{
    ChallengeNonce32, Digest32, HexError, OpaqueId, PublicKey32, Signature64, SimulationSeed64,
    SimulationSeedError,
};
pub use envelope::{
    CAMPAIGN_CONTINUATION_PREFLIGHT_CONTROLLER_SIGNATURE_DOMAIN_V1,
    CAMPAIGN_CONTINUATION_PREFLIGHT_GRANT_SIGNATURE_DOMAIN_V1,
    CAMPAIGN_CONTINUATION_PREFLIGHT_HOST_SIGNATURE_DOMAIN_V1,
    CAMPAIGN_CONTINUATION_SIGNATURE_DOMAIN_V1, COMPETITION_RUN_GRANT_REQUEST_SIGNATURE_DOMAIN_V1,
    COMPETITION_RUN_GRANT_SIGNATURE_DOMAIN_V1, CampaignAggregationConsentV1,
    CampaignChainReceiptV1, CampaignChainStateV1, CampaignCompleteEvidenceV1,
    CampaignContinuationAuthorizationClaimV1, CampaignContinuationAuthorizationV1,
    CampaignContinuationPreflightGrantClaimV1, CampaignContinuationPreflightGrantV1,
    CampaignContinuationPreflightRequestClaimV1, CampaignContinuationPreflightRequestV1,
    CampaignSessionKindV1, CompetitionRunGrantClaimV1, CompetitionRunGrantRequestClaimV1,
    CompetitionRunGrantRequestV1, CompetitionRunGrantV1,
    FRESH_RUN_PREFLIGHT_GRANT_SIGNATURE_DOMAIN_V1, FRESH_RUN_PREFLIGHT_REQUEST_SIGNATURE_DOMAIN_V1,
    FreshRunPreflightGrantClaimV1, FreshRunPreflightGrantV1, FreshRunPreflightRequestClaimV1,
    FreshRunPreflightRequestV1, FreshRunScopeV1, InitialStateExpectationV1,
    InputProvenanceStatusV1, InputTaintKindV1, InputTaintV1, LEADERBOARD_CO_SIGN_PAYLOAD_DOMAIN_V1,
    LEADERBOARD_CO_SIGN_PAYLOAD_LENGTH_V1, LeaderboardCoSignInstanceV1, LeaderboardCoSignPurposeV1,
    LeaderboardCoSignRequestV1, MAX_CAMPAIGN_SESSIONS_V1, MAX_PARTICIPANT_INSTANCES_V1,
    MAX_REPLAY_SEATS_V1, NAMED_SEAT_JOIN_SIGNATURE_DOMAIN_V1, NamedSeatJoinAttestationV1,
    NamedSeatJoinClaimV1, ParticipantClaimV1, ParticipantPublicDisclosureV1,
    ParticipantSignatureV1, PreparedMissionInputsSealV1, RANKED_CAMPAIGN_MEDIA_TYPE_V1,
    RANKED_REPLAY_MEDIA_TYPE_V1, REPLAY_SESSION_GENESIS_SIGNATURE_DOMAIN_V1, RankedSessionConfigV1,
    ReplayArtifactV1, ReplaySeatLifecycleEventV1, ReplaySeatLifecycleKindV1,
    ReplaySessionGenesisClaimV1, ReplaySessionGenesisV1, ReplaySessionTranscriptV1, RunScopeKindV1,
    ScopeRequestV1, SignatureAlgorithmV1, SignedSubmissionV1, SpeechTimingAuthorityV1,
    SubmissionArtifactsV1, SubmissionEnvelopeV1, SubmissionOfferRequestV1, SubmissionOfferV1,
    TerminalOutcomeV1, UploadChallengeV1, UsernameChallengeRequestV1, UsernameChallengeV1,
    UsernameUpdateEnvelopeV1, VerificationInfrastructureFailureCodeV1,
    VerificationInfrastructureFailureV1, VerificationLimitsV1, VerificationRejectionCodeV1,
    VerificationRejectionV1, VerificationRequestV1, VerificationResultV1, VerificationStatusV1,
    VerifiedAchievementEvaluationV1, VerifiedAchievementV1, VerifiedCampaignAggregateV1,
    VerifiedCampaignSessionV1, VerifiedRunV1, VerifierAdmissionFailureCodeV1,
    VerifierWorkerOutputV1, validate_official_ranked_scope_subject_v1,
};
pub use manifest::{
    AchievementPolicyModeV1, AchievementPolicyV1, ActiveTimeDefinitionV1,
    AnonymousParticipantPolicyV1, ArtifactRefV1, BINARYEN_WASM_OPT_VERSION_V1,
    BrowserIdentitySignerBuildIdentityV2, BrowserIdentitySignerBuildRecipeV2,
    BrowserIdentitySignerDeploymentPolicyV2, BrowserPagesArtifactV2,
    BrowserPagesShellBuildIdentityV2, BrowserPagesShellBuildRecipeV2, BrowserViewerBuildIdentityV2,
    BrowserViewerEngineBuildIdentityV2, BrowserViewerEngineBuildRecipeV2, BuildManifestV1,
    BuildManifestV2, BuildToolAuthorityDocumentV1, BuildToolAuthorityV1, BuildToolRoleV1,
    CampaignAggregationConsentPolicyV1, CampaignCompletionPolicyRequirementV1,
    CampaignCompletionPolicyV1, CampaignContentEntryV1, CampaignContentManifestV1,
    CampaignRosterContinuityV1, CanonicalCampaignStateKindV1, CanonicalCampaignStatePinV1,
    CanonicalCampaignStateRequirementV1, CanonicalStartPolicyV1, ContentClosureKindV1,
    ContentFileRoleV1, ContentFileV1, ContentManifestV1, FrameCountingPolicyV1,
    FullCampaignChainPolicyV1, FullCampaignTimeAggregationV1, ImmutablePolicyIdentityV1,
    ImmutablePolicyKindV1, ImmutablePolicyManifestV1, InputProvenanceEligibilityV1,
    MetricRankingPolicyV1, NamedArtifactV1, NamedParticipantPolicyV1, NativeBuildPlatformV2,
    NativeLinkageV2, OFFICIAL_DEMO_FIELD_MISSION_IDS_V1,
    OFFICIAL_FULL_CAMPAIGN_GENESIS_MISSION_ID_V1, OFFICIAL_FULL_FIELD_MISSION_IDS_V1,
    OFFICIAL_FULL_HEADQUARTERS_MISSION_ID_V1, OFFICIAL_PROJECTION_EXPORT_REPORT_SCHEMA_VERSION_V2,
    OFFICIAL_PROJECTION_EXPORTER_MEDIA_TYPE_V2, OFFICIAL_PROJECTION_EXPORTER_VERSION_V2,
    OFFICIAL_PROJECTION_RECEIPT_SCHEMA_VERSION_V2,
    OFFICIAL_SIMULATION_CONTENT_PROJECTION_SCHEMA_VERSION_V1,
    OFFICIAL_VIEWER_BUILD_REPORT_SCHEMA_VERSION_V2, OfficialBuiltInOverlayBindingV2,
    OfficialBuiltInOverlayKindV2, OfficialBuiltInOverlaySourceManifestV2, OfficialContentEditionV1,
    OfficialContentSubjectV1, OfficialProjectionAudioDurationPolicyV1,
    OfficialProjectionAuthorityManifestV2, OfficialProjectionCampaignPolicyV1,
    OfficialProjectionDifficultyV1, OfficialProjectionExecutionPolicyV1,
    OfficialProjectionExportReportV2, OfficialProjectionExporterBuildIdentityV2,
    OfficialProjectionExporterIdentityV1, OfficialProjectionExporterIdentityV2,
    OfficialProjectionExporterPlatformV2, OfficialProjectionHostStatePolicyV1,
    OfficialProjectionLocalePolicyV1, OfficialProjectionOverlayPolicyV1,
    OfficialProjectionSourceFormatV1, OfficialProjectionSubjectReceiptV1,
    OfficialSimulationProjectionReceiptV1, OfficialSimulationProjectionReceiptV2,
    OfficialSourceClosureKindV2, OfficialSourceFileV1, OfficialSourceTreeManifestV1,
    OfficialSourceTreeManifestV2, OfficialViewerBuildReportV2,
    OfficialViewerOriginArtifactInventoryV2, PaginationTieBreakV1, ParticipantEligibilityV1,
    PublishedRulesetV1, RANKED_REPLAY_VERIFIER_MEDIA_TYPE_V2, RANKED_SIMULATION_POLICY_VERSION_V1,
    RankedSimulationDifficultyV1, RankedSimulationPolicyV1, RankedSimulationPresetV1,
    ResourceLocaleRootV1, RulesConfigConstraintV1, RulesConfigIdentityV1, RulesetBoardScopeV1,
    RulesetManifestV1, RulesetOperationalStatusV1, RulesetSeedPolicyV1, RunCompositionPolicyV1,
    RustToolchainAuthorityV1, SIMULATION_CONTENT_COMPONENT_MEDIA_TYPE_V1, ScoreAlgorithmV1,
    ScoreOverflowPolicyV1, SimulationContentComponentDocumentV1, SimulationContentComponentKindV1,
    SimulationContentComponentV1, SimulationSpeechTimingSourceV1, TerminalResultPolicyV1,
    TickDurationV1, VerifierBuildIdentityV2, VersionedBuildManifest, ViewerArtifactRoleV1,
    VisibleTiePolicyV1, WABT_WASM_STRIP_VERSION_V1, WASM_BINDGEN_CLI_AUTHORITY_SHA256_V1,
    WASM_BINDGEN_CLI_VERSION_V1, build_artifact_object_path_v1, demo_content_object_path_v1,
    official_achievement_policies_v1, official_content_manifest_name_v1,
    official_content_subjects_v1, official_full_campaign_completion_policy_v1,
    simulation_component_filename_v1, simulation_content_component_relative_path_v1,
    validate_official_content_subjects_v1, validate_official_projection_receipt_matrix_v2,
};
pub use moderation::{
    AbuseReportAcceptedV1, AbuseReportCategoryV1, AbuseReportTargetV1, AbuseReportV1,
    DELETION_REQUEST_SIGNATURE_DOMAIN_V1, DeletionChallengeRequestV1, DeletionChallengeV1,
    DeletionReceiptV1, DeletionRequestEnvelopeV1, DeletionTargetV1,
};
pub use query::{
    AchievementSummaryV1, AggregatePublicParticipantV1, BoardCategoryV1, BoardMetricV1,
    BoardMetricValueV1, CampaignSessionDetailV1, CompetitionManifestV1,
    CompetitionParticipantCompositionV1, CompetitionSeedPolicyV1, CompetitionStateV1,
    CompetitionSummaryV1, FullCampaignFacetV1, FullCampaignSessionKindV1, FullCampaignSessionV1,
    LeaderboardCursorV1, LeaderboardEntryV1, LeaderboardMetadataV1, LeaderboardOrderAnchorV1,
    LeaderboardPageV1, LeaderboardQuerySubjectV1, LeaderboardQueryV1, LeaderboardSubjectV1,
    MissionFacetV1, PlayerPersonalBestV1, PlayerProfileV1, PlayerRunHistoryEntryV1,
    PlayerRunHistoryFilterV1, PlayerRunHistoryPageV1, PlayerRunHistoryQueryV1,
    PublicAchievementDecisionV1, PublicBuildV1, PublicCampaignAggregateProofV1,
    PublicCampaignAggregateRequestV1, PublicCampaignCompleteEvidenceV1,
    PublicCampaignSessionBindingV1, PublicNamedParticipantClaimV1, PublicParticipantV1,
    PublicVerificationProofV1, PublicVerificationRequestV1, RulesetFacetV1, RunContentIdentityV1,
    RunDetailV1, RunFilterV1, RunMetricsV1, RunSummaryV1,
    SUBMISSION_OWNER_STATUS_SIGNATURE_DOMAIN_V1, SubmissionAcceptedV1, SubmissionFailureCodeV1,
    SubmissionLifecycleV1, SubmissionOwnerStatusChallengeRequestV1,
    SubmissionOwnerStatusChallengeV1, SubmissionOwnerStatusEnvelopeV1,
    SubmissionOwnerStatusResponseV1, VerifiedRunCompositionV1, ViewerAvailabilityV1,
    ViewerContentRequirementV1, ViewerLaunchV1,
};
pub use validation::{Validate, ValidationError};
pub use verifier_job::{
    CampaignSessionBindingV1, MAX_VERIFIER_JOB_CONFIG_BYTES_V1, VerifierJobConfigCatalogV1,
    VerifierJobConfigV1, VerifierJobRouteV1, VerifierJobTemplateV1,
};

/// Every explicitly named `V1` document carries this value on the wire.
pub const SCHEMA_VERSION_V1: u32 = 1;
/// Exact SQLx migration level shared by release manifests, backup identities,
/// and the high-score runtime.
pub const HIGHSCORES_DATABASE_SCHEMA_VERSION: i64 = 2;

/// Exact save format emitted by a build eligible for the current ranked
/// replay contract. The engine and ranking-service publication gate share
/// this constant so a stale build cannot be advertised as current.
pub const CURRENT_RANKED_SAVE_SCHEMA_VERSION_V1: u32 = 75;

/// Exact replay schema whose compact bitcode bytes are simultaneously the
/// submitted, verifier-resimulated, retained, and publicly downloadable
/// artifact. Older Rust schemas are intentionally outside the service
/// contract and must be rejected rather than normalized.
pub const CURRENT_RANKED_REPLAY_SCHEMA_VERSION_V1: u32 = 33;

/// Exact multiplayer wire protocol carried by current ranked session genesis,
/// build manifests, and immutable ruleset allowlists. A current replay schema
/// may not be paired with an older self-consistent network tuple.
pub const CURRENT_RANKED_NETWORK_PROTOCOL_VERSION_V1: u32 = 41;
