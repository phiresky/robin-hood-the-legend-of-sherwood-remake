//! Shared router end-to-end rig, request builders and manifest fixtures.

pub(crate) use axum::Router;
pub(crate) use axum::body::Body;
pub(crate) use axum::extract::ConnectInfo;
pub(crate) use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE, X_CONTENT_TYPE_OPTIONS};
pub(crate) use axum::http::{Method, Request, StatusCode};
pub(crate) use ed25519_dalek::{Signer as _, SigningKey};
pub(crate) use http_body_util::BodyExt as _;
pub(crate) use robin_highscores::config::{
    AdmissionProfile, CompetitionConfig, LoadedBuildManifest, ManifestRegistry,
};
pub(crate) use robin_highscores::verifier::build_verification_request;
pub(crate) use robin_highscores::web::{AppState, ChallengeRateLimiter, router};
pub(crate) use robin_highscores::{CampaignStore, Database, ReplayStore, ServerConfig};
pub(crate) use robin_run_protocol::{
    AbuseReportAcceptedV1, AbuseReportCategoryV1, AbuseReportTargetV1, AbuseReportV1,
    ActiveTimeDefinitionV1, AnonymousParticipantPolicyV1, ArtifactRefV1, BoardCategoryV1,
    BoardMetricV1, BuildManifestV1, CampaignAggregationConsentPolicyV1,
    CampaignAggregationConsentV1, CampaignChainReceiptV1, CampaignChainStateV1,
    CampaignCompleteEvidenceV1, CampaignCompletionPolicyRequirementV1, CampaignContentEntryV1,
    CampaignContentManifestV1, CampaignContinuationAuthorizationClaimV1,
    CampaignContinuationAuthorizationV1, CampaignContinuationPreflightGrantV1,
    CampaignContinuationPreflightRequestClaimV1, CampaignContinuationPreflightRequestV1,
    CampaignRosterContinuityV1, CampaignSessionDetailV1, CampaignSessionKindV1,
    CanonicalCampaignStateKindV1, CanonicalCampaignStatePinV1, CanonicalCampaignStateRequirementV1,
    CanonicalDocument as _, CanonicalStartPolicyV1, CanonicalValue, ChallengeNonce32,
    CompetitionManifestV1, CompetitionParticipantCompositionV1, CompetitionRunGrantRequestClaimV1,
    CompetitionRunGrantRequestV1, CompetitionRunGrantV1, CompetitionSeedPolicyV1,
    CompetitionStateV1, ContentClosureKindV1, ContentManifestV1, DeletionChallengeRequestV1,
    DeletionChallengeV1, DeletionReceiptV1, DeletionRequestEnvelopeV1, DeletionTargetV1, Digest32,
    FrameCountingPolicyV1, FreshRunPreflightGrantV1, FreshRunPreflightRequestClaimV1,
    FreshRunPreflightRequestV1, FreshRunScopeV1, FullCampaignChainPolicyV1,
    FullCampaignTimeAggregationV1, ImmutablePolicyIdentityV1, ImmutablePolicyKindV1,
    InitialStateExpectationV1, InputProvenanceEligibilityV1, InputProvenanceStatusV1,
    LeaderboardMetadataV1, LeaderboardPageV1, LeaderboardSubjectV1, MetricRankingPolicyV1,
    NamedArtifactV1, NamedParticipantPolicyV1, OFFICIAL_FULL_CAMPAIGN_GENESIS_MISSION_ID_V1,
    OfficialContentEditionV1, OfficialContentSubjectV1, OpaqueId, PaginationTieBreakV1,
    ParticipantClaimV1, ParticipantEligibilityV1, ParticipantPublicDisclosureV1,
    ParticipantSignatureV1, PlayerProfileV1, PlayerRunHistoryPageV1, PublicKey32,
    RANKED_CAMPAIGN_MEDIA_TYPE_V1, RANKED_REPLAY_MEDIA_TYPE_V1, ReplayArtifactV1,
    ReplaySeatLifecycleEventV1, ReplaySeatLifecycleKindV1, ReplaySessionGenesisClaimV1,
    ReplaySessionGenesisV1, ReplaySessionTranscriptV1, ResourceLocaleRootV1,
    RulesConfigConstraintV1, RulesConfigIdentityV1, RulesetBoardScopeV1, RulesetManifestV1,
    RulesetOperationalStatusV1, RulesetSeedPolicyV1, RunCompositionPolicyV1, RunContentIdentityV1,
    RunDetailV1, RunScopeKindV1, SCHEMA_VERSION_V1, SIMULATION_CONTENT_COMPONENT_MEDIA_TYPE_V1,
    ScopeRequestV1, ScoreAlgorithmV1, ScoreOverflowPolicyV1, Signature64, SignatureAlgorithmV1,
    SignedSubmissionV1, SimulationContentComponentKindV1, SimulationContentComponentV1,
    SimulationSeed64, SimulationSpeechTimingSourceV1, SpeechTimingAuthorityV1,
    SubmissionAcceptedV1, SubmissionArtifactsV1, SubmissionEnvelopeV1, SubmissionLifecycleV1,
    SubmissionOfferRequestV1, SubmissionOfferV1, SubmissionOwnerStatusChallengeRequestV1,
    SubmissionOwnerStatusChallengeV1, SubmissionOwnerStatusEnvelopeV1,
    SubmissionOwnerStatusResponseV1, TerminalOutcomeV1, TerminalResultPolicyV1, TickDurationV1,
    UsernameChallengeRequestV1, UsernameChallengeV1, UsernameUpdateEnvelopeV1, Validate as _,
    VerificationLimitsV1, VerificationResultV1, VerificationStatusV1,
    VerifiedAchievementEvaluationV1, VerifiedAchievementV1, VerifiedRunV1, ViewerArtifactRoleV1,
    VisibleTiePolicyV1, official_achievement_policies_v1,
    official_full_campaign_completion_policy_v1,
};
pub(crate) use serde::Serialize;
pub(crate) use serde::de::DeserializeOwned;
pub(crate) use sqlx::Row as _;
pub(crate) use std::collections::BTreeMap;
pub(crate) use std::net::{IpAddr, Ipv4Addr, SocketAddr};
pub(crate) use std::sync::Arc;
pub(crate) use std::time::{Duration, SystemTime, UNIX_EPOCH};
pub(crate) use tokio::io::AsyncReadExt as _;
pub(crate) use tower::ServiceExt as _;

pub(crate) const MISSION_ID: &str = "Dem_Lei_MP";
pub(crate) const CAMPAIGN_MISSION_ID: &str = "H02_Not_EC";
pub(crate) const HQ_MISSION_ID: &str = "H12_Not_MP";
pub(crate) const GENESIS_MISSION_ID: &str = OFFICIAL_FULL_CAMPAIGN_GENESIS_MISSION_ID_V1;
pub(crate) const REPLAY_SCHEMA_VERSION: u32 =
    robin_run_protocol::CURRENT_RANKED_REPLAY_SCHEMA_VERSION_V1;
pub(crate) const NETWORK_PROTOCOL_VERSION: u32 =
    robin_run_protocol::CURRENT_RANKED_NETWORK_PROTOCOL_VERSION_V1;

mod manifests;
mod requests;
mod rig;

pub(crate) use manifests::*;
pub(crate) use requests::*;
pub(crate) use rig::*;
