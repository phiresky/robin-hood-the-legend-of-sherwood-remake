use crate::config::{AdmissionProfile, LoadedBuildManifest, ServerConfig};
use crate::db_fence::{
    DatabaseFenceOperation, ProcessDatabaseFenceManager, RuntimeDatabaseFence,
    provision_test_runtime_fence, wait_for_pool_idle,
};
use crate::identity::{normalized_username, verify_signature};
use crate::model::{ChallengePurpose, NewSubmission, SubmissionLifecycle, WorkerJob, now_epoch_ms};
use robin_run_protocol::{
    AnonymousParticipantPolicyV1, ArtifactRefV1, BuildManifestV1, CampaignAggregationConsentV1,
    CampaignContentManifestV1, CampaignSessionBindingV1, CampaignSessionKindV1,
    CanonicalCampaignStatePinV1, CanonicalDocument as _, CompetitionManifestV1,
    CompetitionRunGrantRequestV1, CompetitionRunGrantV1, ContentManifestV1, Digest32,
    InitialStateExpectationV1, InputProvenanceStatusV1, OfficialContentEditionV1,
    OfficialContentSubjectV1, OpaqueId, ParticipantPublicDisclosureV1,
    PublicCampaignAggregateProofV1, PublicCampaignAggregateRequestV1, PublicVerificationProofV1,
    PublicVerificationRequestV1, PublishedRulesetV1, RulesetBoardScopeV1,
    RulesetOperationalStatusV1, RunScopeKindV1, TerminalOutcomeV1, Validate as _,
    VerificationRequestV1, VerificationResultV1, VerificationStatusV1,
    VerifiedAchievementEvaluationV1, VerifiedCampaignAggregateV1, VerifiedCampaignSessionV1,
    VerifierJobRouteV1, validate_official_ranked_scope_subject_v1,
};
use serde::{Deserialize, Serialize};
use sqlx::sqlite::{
    SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteRow, SqliteSynchronous,
};
use sqlx::{QueryBuilder, Row as _, Sqlite, SqlitePool};
use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");
mod acceptance;
mod maintenance;
mod public_queries;
mod uploads;
mod worker;

pub const CURRENT_SCHEMA_VERSION: i64 = robin_run_protocol::HIGHSCORES_DATABASE_SCHEMA_VERSION;

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("database error: {0}")]
    Sql(#[from] sqlx::Error),
    #[error("database migration error: {0}")]
    Migration(#[from] sqlx::migrate::MigrateError),
    #[error("system clock is before the Unix epoch")]
    Clock(#[from] std::time::SystemTimeError),
    #[error("not found")]
    NotFound,
    #[error("challenge is expired, consumed, stale, or for a different operation")]
    InvalidChallenge,
    #[error("submission queue is full")]
    QueueFull,
    #[error("new submission storage admission is temporarily unavailable")]
    AdmissionUnavailable,
    #[error("submission challenge was already used for different immutable content")]
    SubmissionConflict,
    #[error("campaign predecessor was already consumed by another accepted run")]
    CampaignFork,
    #[error("worker does not hold the current submission lease")]
    LeaseLost,
    #[error("verifier result violates admission invariants: {0}")]
    ResultInvariant(String),
    #[error("stored database value is invalid: {0}")]
    Corrupt(String),
}

impl DbError {
    /// Stable, non-sensitive classification for operational logs. Error
    /// messages can contain SQLite details or verifier-derived diagnostics and
    /// must never be emitted by the public service or worker.
    pub const fn safe_log_code(&self) -> &'static str {
        match self {
            Self::Sql(_) => "database_io",
            Self::Migration(_) => "database_migration",
            Self::Clock(_) => "system_clock",
            Self::NotFound => "not_found",
            Self::InvalidChallenge => "invalid_challenge",
            Self::QueueFull => "queue_full",
            Self::AdmissionUnavailable => "storage_admission_unavailable",
            Self::SubmissionConflict => "submission_conflict",
            Self::CampaignFork => "campaign_fork",
            Self::LeaseLost => "lease_lost",
            Self::ResultInvariant(_) => "result_invariant",
            Self::Corrupt(_) => "stored_data_corrupt",
        }
    }
}

#[derive(Clone)]
pub struct Database {
    pool: SqlitePool,
    max_pending_submissions: u32,
    max_concurrent_sensitive_writers: u64,
    max_concurrent_upload_writers: u64,
    fence: ProcessDatabaseFenceManager,
    /// Keep the exact parent, database inode and live WAL/SHM inodes pinned for
    /// the entire pool lifetime. The pool connects through the retained main
    /// file descriptor, never through the mutable configured pathname.
    _database_parent: Arc<cap_std::fs::Dir>,
    _database_file: Arc<std::fs::File>,
    _database_sidecars: Arc<Vec<std::fs::File>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaintenanceWriteClass {
    ApiSensitive,
    ApiUpload,
    ApiMaintenance,
    Worker,
    Admin,
}

impl MaintenanceWriteClass {
    const fn as_str(self) -> &'static str {
        match self {
            Self::ApiSensitive => "api_sensitive",
            Self::ApiUpload => "api_upload",
            Self::ApiMaintenance => "api_maintenance",
            Self::Worker => "worker",
            Self::Admin => "admin",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct IssuedChallenge {
    pub id: String,
    #[serde(with = "hex_array")]
    pub nonce: [u8; 32],
    pub expires_at_ms: u64,
}

#[derive(Debug, Clone)]
pub struct IssuedCompetitionRunGrant {
    pub id: String,
    pub nonce: [u8; 32],
    pub admitted_at_ms: u64,
    pub expires_at_ms: u64,
}

#[derive(Debug, Clone)]
pub struct StoredOffer {
    pub offer_json: String,
    pub public_metadata_json: String,
}

/// Exact database-persisted campaign authority for one leased verifier job.
/// The canonical pin is static lineage authority; the optional session is the
/// run-specific chain position derived from accepted predecessor state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerificationCampaignAuthority {
    pub canonical_campaign_state: CanonicalCampaignStatePinV1,
    pub campaign_session: Option<CampaignSessionBindingV1>,
}

/// Immutable, authenticated metadata which must reserve a challenge before
/// the HTTP layer is allowed to read any replay or campaign bytes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubmissionUploadIntent {
    pub proposed_submission_id: String,
    pub upload_challenge_id: String,
    pub offer_json: String,
    pub envelope_json: String,
    pub controller_public_key: [u8; 32],
    pub session_genesis_sha256: [u8; 32],
    pub session_genesis_host_public_key: [u8; 32],
    pub replay_session_id: [u8; 32],
    pub session_genesis_host_nonce: [u8; 32],
    pub participants: Vec<crate::model::ParticipantClaim>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubmissionUploadLease {
    pub submission_id: String,
    pub upload_challenge_id: String,
    pub lease_token: String,
    pub lease_expires_at_ms: u64,
    pub reservation_expires_at_ms: u64,
    pub offer_json: String,
    pub public_metadata_json: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case", deny_unknown_fields)]
pub enum SubmissionUploadReservation {
    Acquired {
        lease: SubmissionUploadLease,
        resume_uploaded: bool,
    },
    Existing {
        lifecycle: SubmissionLifecycle,
    },
    Busy {
        retry_after_ms: u64,
    },
}

#[derive(Debug, Clone)]
pub struct CampaignPredecessor {
    pub run_id: String,
    pub chain_id: String,
    pub result_sha256: [u8; 32],
    pub verification_request_sha256: [u8; 32],
    pub verification_result_json: String,
    pub final_campaign_sha256: [u8; 32],
    pub final_campaign_bytes: u64,
    pub content_manifest_id: [u8; 32],
    pub campaign_content_manifest_id: [u8; 32],
    pub rules_config_id: [u8; 32],
    pub ruleset_id: [u8; 32],
    pub competition_manifest_id: Option<[u8; 32]>,
    pub campaign_session_ordinal: u32,
    pub max_concurrent_players: u16,
    pub participant_instance_count: u16,
    /// Immutable controller established by the signed host of ordinal zero.
    pub chain_owner_public_key: [u8; 32],
    pub participants: Vec<crate::model::ParticipantClaim>,
}

#[derive(Debug, Clone)]
pub struct PublicIdentity {
    pub public_key: [u8; 32],
    pub username: String,
}

#[derive(Debug, Clone)]
pub struct PublicParticipantRecord {
    pub seat: u16,
    pub identity: PublicIdentity,
}

#[derive(Debug, Clone)]
pub struct BoardRow {
    pub rank: u64,
    pub run_id: String,
    pub composition: BoardComposition,
    pub metric_value: i64,
    pub max_concurrent_players: u16,
    pub participant_instance_count: u32,
    pub named_participant_instance_count: u32,
    pub anonymous_participant_instance_count: u32,
    pub accepted_sequence: u64,
    pub verified_at_ms: u64,
    pub named_participants: Vec<PublicParticipantRecord>,
    pub aggregate_named_participants: Vec<PublicIdentity>,
}

#[derive(Debug, Clone)]
pub enum BoardComposition {
    Mission {
        replay_sha256: [u8; 32],
    },
    FullCampaign {
        ordered_session_run_ids: Vec<String>,
    },
}

#[derive(Debug, Clone)]
pub struct PlayerHistoryRecord {
    pub run_id: String,
    pub composition: BoardComposition,
    pub mission_id: Option<String>,
    pub scope_kind: String,
    pub original_score_delta: i64,
    pub active_simulation_ticks: u64,
    pub ransom_collected: u64,
    pub content_manifest_id: [u8; 32],
    pub config_id: [u8; 32],
    pub ruleset_id: [u8; 32],
    pub competition_manifest_id: Option<[u8; 32]>,
    pub max_concurrent_players: u16,
    pub participant_instance_count: u32,
    pub accepted_sequence: u64,
    pub verified_at_ms: u64,
    pub named_participants: Vec<PublicParticipantRecord>,
    pub aggregate_named_participants: Vec<PublicIdentity>,
}

#[derive(Debug, Clone)]
pub struct PlayerBestRecord {
    pub run_id: String,
    pub mission_id: Option<String>,
    pub scope_kind: String,
    pub metric: String,
    pub value: i64,
    pub content_manifest_id: [u8; 32],
    pub config_id: [u8; 32],
    pub ruleset_id: [u8; 32],
    pub competition_manifest_id: Option<[u8; 32]>,
    pub max_concurrent_players: u16,
}

#[derive(Debug, Clone)]
pub struct BoardCursor {
    pub metric_value: i64,
    pub accepted_sequence: i64,
    pub run_id: String,
}

#[derive(Debug, Clone)]
pub struct PublicRunRecord {
    pub run_id: String,
    pub replay_sha256: [u8; 32],
    pub replay_bytes: u64,
    pub build_manifest_id: [u8; 32],
    pub content_manifest_id: [u8; 32],
    pub campaign_content_manifest_id: Option<[u8; 32]>,
    pub config_id: [u8; 32],
    pub ruleset_id: [u8; 32],
    pub mission_id: String,
    pub scope_kind: String,
    pub competition_manifest_id: Option<[u8; 32]>,
    pub full_campaign_run_id: Option<String>,
    pub campaign_terminal: bool,
    pub starting_campaign_sha256: [u8; 32],
    pub starting_campaign_bytes: u64,
    pub final_campaign_sha256: [u8; 32],
    pub final_campaign_bytes: u64,
    pub public_verification_request_sha256: [u8; 32],
    pub public_verification_result_sha256: [u8; 32],
    pub verification_proof: PublicVerificationProofV1,
    pub input_provenance_json: String,
    pub original_score_delta: i64,
    pub active_simulation_ticks: u64,
    pub ransom_collected: u64,
    pub starting_campaign_score: i32,
    pub final_campaign_score: i32,
    pub campaign_session_kind: Option<String>,
    pub campaign_session_ordinal: Option<u32>,
    pub campaign_hq_sequence: Option<u32>,
    pub max_concurrent_players: u16,
    pub participant_instance_count: u32,
    pub named_participant_instance_count: u32,
    pub anonymous_participant_instance_count: u32,
    pub verified_at_ms: u64,
    pub public_metadata_json: String,
    pub named_participants: Vec<PublicParticipantRecord>,
}

#[derive(Debug, Clone)]
pub struct OwnerCampaignReceiptContext {
    pub chain_id: String,
    pub predecessor_verification_sha256: robin_run_protocol::Digest32,
    pub final_campaign: ArtifactRefV1,
    pub participant_public_keys: Vec<robin_run_protocol::PublicKey32>,
    pub controller_public_key: robin_run_protocol::PublicKey32,
    pub campaign_content_manifest_id: [u8; 32],
    pub config_id: [u8; 32],
    pub ruleset_id: [u8; 32],
    pub competition_manifest_id: Option<[u8; 32]>,
    pub max_concurrent_players: u16,
    pub completed_full_campaign_run_id: Option<String>,
    pub verification_request: VerificationRequestV1,
    pub verification_result: VerificationResultV1,
}

#[derive(Debug, Clone)]
pub struct PublicFullCampaignRecord {
    pub run_id: String,
    pub public_aggregate_request_sha256: [u8; 32],
    pub public_aggregate_result_sha256: [u8; 32],
    pub aggregate_proof: PublicCampaignAggregateProofV1,
    pub campaign_content_manifest_id: [u8; 32],
    pub config_id: [u8; 32],
    pub ruleset_id: [u8; 32],
    pub competition_manifest_id: Option<[u8; 32]>,
    pub starting_campaign_sha256: [u8; 32],
    pub starting_campaign_bytes: u64,
    pub final_campaign_sha256: [u8; 32],
    pub final_campaign_bytes: u64,
    pub starting_campaign_score: i32,
    pub final_campaign_score: i32,
    pub active_simulation_ticks: u64,
    pub ransom_collected: u64,
    pub max_concurrent_players: u16,
    pub participant_instance_count: u32,
    pub named_participant_instance_count: u32,
    pub anonymous_participant_instance_count: u32,
    pub verified_at_ms: u64,
    pub named_participants: Vec<PublicIdentity>,
    pub ordered_session_run_ids: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct DeletionRecord {
    pub id: String,
    pub tombstoned_at_ms: u64,
    pub purge_eligible_at_ms: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct ReplayGcCandidate {
    pub sha256: [u8; 32],
    pub byte_length: u64,
    pub claim_token: String,
}

#[derive(Debug, Clone)]
pub struct CampaignGcCandidate {
    pub sha256: [u8; 32],
    pub byte_length: u64,
    pub claim_token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModerationReportRecord {
    pub id: String,
    pub target_kind: String,
    pub target_id: String,
    pub category: String,
    pub detail: String,
    pub received_at_ms: u64,
    pub moderation_state: String,
    pub moderator_note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModerationAuditRecord {
    pub id: u64,
    pub report_id: Option<String>,
    pub action: String,
    pub previous_state: Option<String>,
    pub new_state: Option<String>,
    pub detail: String,
    pub operator_id: String,
    pub created_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationalCounts {
    pub queued_submissions: u64,
    pub active_upload_reservations: u64,
    pub abandoned_upload_reservations: u64,
    pub accepted_runs: u64,
    pub rejected_retained_submissions: u64,
    pub open_abuse_reports: u64,
    pub replay_objects_live: u64,
    pub replay_objects_purging: u64,
    pub campaign_objects_live: u64,
    pub campaign_objects_purging: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PrivateCampaignAggregateRequestV1 {
    schema_version: u32,
    chain_id: OpaqueId,
    full_campaign_run_id: OpaqueId,
    terminal_run_id: OpaqueId,
    campaign_complete_evidence_sha256: robin_run_protocol::Digest32,
    sessions: Vec<PrivateCampaignAggregateSessionRequestV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PrivateCampaignAggregateSessionRequestV1 {
    ordinal: u32,
    run_id: OpaqueId,
    verification_request_sha256: robin_run_protocol::Digest32,
    verification_result_sha256: robin_run_protocol::Digest32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredProjectionBindingV1 {
    schema_version: u32,
    private_request_sha256: robin_run_protocol::Digest32,
    private_result_sha256: robin_run_protocol::Digest32,
    public_request_sha256: robin_run_protocol::Digest32,
    public_result_sha256: robin_run_protocol::Digest32,
}

impl PrivateCampaignAggregateRequestV1 {
    fn validate(&self) -> Result<(), DbError> {
        if self.schema_version != robin_run_protocol::SCHEMA_VERSION_V1
            || self.campaign_complete_evidence_sha256.is_zero()
            || self.sessions.is_empty()
            || self.sessions.len() > 4_096
            || self.sessions.iter().enumerate().any(|(ordinal, session)| {
                session.ordinal != ordinal as u32
                    || session.verification_request_sha256.is_zero()
                    || session.verification_result_sha256.is_zero()
            })
            || self
                .sessions
                .iter()
                .map(|session| &session.run_id)
                .collect::<BTreeSet<_>>()
                .len()
                != self.sessions.len()
            || self
                .sessions
                .last()
                .is_none_or(|session| session.run_id != self.terminal_run_id)
        {
            return Err(DbError::ResultInvariant(
                "private campaign aggregate request is not canonical".to_owned(),
            ));
        }
        Ok(())
    }

    fn canonical_digest(&self) -> Result<robin_run_protocol::Digest32, DbError> {
        self.validate()?;
        let bytes = robin_run_protocol::canonical_json_bytes(self)
            .map_err(|error| DbError::ResultInvariant(error.to_string()))?;
        Ok(robin_run_protocol::Digest32::digest_bytes(bytes))
    }
}

fn canonical_json_string(value: &(impl Serialize + ?Sized)) -> Result<String, DbError> {
    String::from_utf8(
        robin_run_protocol::canonical_json_bytes(value)
            .map_err(|error| DbError::ResultInvariant(error.to_string()))?,
    )
    .map_err(|error| DbError::ResultInvariant(error.to_string()))
}

fn validate_aggregate_genesis_scope_subject(
    edition: OfficialContentEditionV1,
    subject: &OfficialContentSubjectV1,
    starting_state: &InitialStateExpectationV1,
) -> Result<(), DbError> {
    validate_official_ranked_scope_subject_v1(edition, subject, starting_state).map_err(|_| {
        DbError::ResultInvariant(
            "full campaign aggregate does not begin in the official H01 genesis lane".to_owned(),
        )
    })
}

fn projection_binding_json(
    private_request_sha256: robin_run_protocol::Digest32,
    private_result_sha256: robin_run_protocol::Digest32,
    public_request_sha256: robin_run_protocol::Digest32,
    public_result_sha256: robin_run_protocol::Digest32,
) -> Result<String, DbError> {
    canonical_json_string(&StoredProjectionBindingV1 {
        schema_version: robin_run_protocol::SCHEMA_VERSION_V1,
        private_request_sha256,
        private_result_sha256,
        public_request_sha256,
        public_result_sha256,
    })
}

fn stored_projection_binding(
    row: &SqliteRow,
    private_request_column: &str,
    private_result_column: &str,
    public_request_column: &str,
    public_result_column: &str,
) -> Result<StoredProjectionBindingV1, DbError> {
    let binding_json: String = row.try_get("public_projection_binding_json")?;
    let binding: StoredProjectionBindingV1 = serde_json::from_str(&binding_json)
        .map_err(|error| DbError::Corrupt(format!("public projection binding JSON: {error}")))?;
    let expected = StoredProjectionBindingV1 {
        schema_version: robin_run_protocol::SCHEMA_VERSION_V1,
        private_request_sha256: robin_run_protocol::Digest32::from_bytes(fixed_32(
            row.try_get(private_request_column)?,
        )?),
        private_result_sha256: robin_run_protocol::Digest32::from_bytes(fixed_32(
            row.try_get(private_result_column)?,
        )?),
        public_request_sha256: robin_run_protocol::Digest32::from_bytes(fixed_32(
            row.try_get(public_request_column)?,
        )?),
        public_result_sha256: robin_run_protocol::Digest32::from_bytes(fixed_32(
            row.try_get(public_result_column)?,
        )?),
    };
    let canonical_binding_json = canonical_json_string(&binding)
        .map_err(|error| DbError::Corrupt(format!("public projection binding JSON: {error}")))?;
    if binding_json != canonical_binding_json
        || binding.schema_version != robin_run_protocol::SCHEMA_VERSION_V1
        || binding.private_request_sha256.is_zero()
        || binding.private_result_sha256.is_zero()
        || binding.public_request_sha256.is_zero()
        || binding.public_result_sha256.is_zero()
        || binding != expected
    {
        return Err(DbError::Corrupt(
            "stored public projection binding is not canonical or differs from indexed digests"
                .to_owned(),
        ));
    }
    Ok(binding)
}

fn stored_public_verification_proof(row: &SqliteRow) -> Result<PublicVerificationProofV1, DbError> {
    let binding = stored_projection_binding(
        row,
        "verification_request_sha256",
        "result_sha256",
        "public_verification_request_sha256",
        "public_verification_result_sha256",
    )?;
    let public_request_json: String = row.try_get("public_verification_request_json")?;
    let public_request: PublicVerificationRequestV1 = serde_json::from_str(&public_request_json)
        .map_err(|error| DbError::Corrupt(format!("public verification request JSON: {error}")))?;
    public_request
        .validate()
        .map_err(|error| DbError::Corrupt(format!("public verification request: {error}")))?;
    let public_request_sha256 = public_request.canonical_digest().map_err(|error| {
        DbError::Corrupt(format!("public verification request digest: {error}"))
    })?;
    let public_result_json: String = row.try_get("public_verification_result_json")?;
    let proof: PublicVerificationProofV1 = serde_json::from_str(&public_result_json)
        .map_err(|error| DbError::Corrupt(format!("public verification proof JSON: {error}")))?;
    proof
        .validate()
        .map_err(|error| DbError::Corrupt(format!("public verification proof: {error}")))?;
    let public_result_sha256 = proof
        .canonical_digest()
        .map_err(|error| DbError::Corrupt(format!("public verification proof digest: {error}")))?;
    let canonical_request_json = canonical_json_string(&public_request)
        .map_err(|error| DbError::Corrupt(format!("public request canonical JSON: {error}")))?;
    let canonical_result_json = canonical_json_string(&proof)
        .map_err(|error| DbError::Corrupt(format!("public proof canonical JSON: {error}")))?;
    if public_request_json != canonical_request_json
        || public_result_json != canonical_result_json
        || proof.public_request != public_request
        || proof.public_request_sha256 != public_request_sha256
        || binding.public_request_sha256 != public_request_sha256
        || binding.public_result_sha256 != public_result_sha256
    {
        return Err(DbError::Corrupt(
            "stored public verification documents are not exactly cross-bound".to_owned(),
        ));
    }
    Ok(proof)
}

fn stored_public_aggregate_proof(
    row: &SqliteRow,
) -> Result<(PublicCampaignAggregateProofV1, OpaqueId), DbError> {
    let binding = stored_projection_binding(
        row,
        "aggregate_request_sha256",
        "aggregate_sha256",
        "public_aggregate_request_sha256",
        "public_aggregate_result_sha256",
    )?;
    let private_request_json: String = row.try_get("aggregate_request_json")?;
    let private_request: PrivateCampaignAggregateRequestV1 =
        serde_json::from_str(&private_request_json).map_err(|error| {
            DbError::Corrupt(format!("private aggregate request JSON: {error}"))
        })?;
    private_request.validate()?;
    let private_request_sha256 = private_request.canonical_digest()?;
    let private_result_json: String = row.try_get("aggregate_json")?;
    let private_result: VerifiedCampaignAggregateV1 = serde_json::from_str(&private_result_json)
        .map_err(|error| DbError::Corrupt(format!("private aggregate result JSON: {error}")))?;
    private_result
        .validate()
        .map_err(|error| DbError::Corrupt(format!("private aggregate result: {error}")))?;
    let private_result_sha256 = private_result
        .canonical_digest()
        .map_err(|error| DbError::Corrupt(format!("private aggregate result digest: {error}")))?;
    let private_request_is_bound = private_result.aggregate_request_sha256
        == private_request_sha256
        && private_result.chain_id == private_request.chain_id
        && private_result.full_campaign_run_id == private_request.full_campaign_run_id
        && private_result.campaign_complete_terminal_run_id == private_request.terminal_run_id
        && private_result.campaign_complete_evidence_sha256
            == private_request.campaign_complete_evidence_sha256
        && private_result.sessions.len() == private_request.sessions.len()
        && private_result
            .sessions
            .iter()
            .zip(&private_request.sessions)
            .all(|(result, request)| {
                result.ordinal == request.ordinal
                    && result.run_id == request.run_id
                    && result.verification_request_sha256 == request.verification_request_sha256
                    && result.verification_result_sha256 == request.verification_result_sha256
            });
    let canonical_private_request_json = canonical_json_string(&private_request)
        .map_err(|error| DbError::Corrupt(format!("private aggregate request JSON: {error}")))?;
    let canonical_private_result_json = canonical_json_string(&private_result)
        .map_err(|error| DbError::Corrupt(format!("private aggregate result JSON: {error}")))?;
    if private_request_json != canonical_private_request_json
        || private_result_json != canonical_private_result_json
        || binding.private_request_sha256 != private_request_sha256
        || binding.private_result_sha256 != private_result_sha256
        || !private_request_is_bound
    {
        return Err(DbError::Corrupt(
            "stored private aggregate documents are not canonical and exactly cross-bound"
                .to_owned(),
        ));
    }
    let public_request_json: String = row.try_get("public_aggregate_request_json")?;
    let public_request: PublicCampaignAggregateRequestV1 =
        serde_json::from_str(&public_request_json)
            .map_err(|error| DbError::Corrupt(format!("public aggregate request JSON: {error}")))?;
    public_request
        .validate()
        .map_err(|error| DbError::Corrupt(format!("public aggregate request: {error}")))?;
    let public_request_sha256 = public_request
        .canonical_digest()
        .map_err(|error| DbError::Corrupt(format!("public aggregate request digest: {error}")))?;
    let public_result_json: String = row.try_get("public_aggregate_result_json")?;
    let proof: PublicCampaignAggregateProofV1 = serde_json::from_str(&public_result_json)
        .map_err(|error| DbError::Corrupt(format!("public aggregate proof JSON: {error}")))?;
    proof
        .validate()
        .map_err(|error| DbError::Corrupt(format!("public aggregate proof: {error}")))?;
    let public_result_sha256 = proof
        .canonical_digest()
        .map_err(|error| DbError::Corrupt(format!("public aggregate proof digest: {error}")))?;
    let canonical_request_json = canonical_json_string(&public_request)
        .map_err(|error| DbError::Corrupt(format!("public aggregate request JSON: {error}")))?;
    let canonical_result_json = canonical_json_string(&proof)
        .map_err(|error| DbError::Corrupt(format!("public aggregate proof JSON: {error}")))?;
    if public_request_json != canonical_request_json
        || public_result_json != canonical_result_json
        || proof.public_request != public_request
        || proof.public_request_sha256 != public_request_sha256
        || binding.public_request_sha256 != public_request_sha256
        || binding.public_result_sha256 != public_result_sha256
    {
        return Err(DbError::Corrupt(
            "stored public aggregate documents are not exactly cross-bound".to_owned(),
        ));
    }
    Ok((proof, private_request.chain_id))
}

impl Database {
    pub(crate) fn storage_volume(
        &self,
    ) -> Result<crate::storage_admission::StorageVolume, std::io::Error> {
        crate::storage_admission::StorageVolume::from_pinned_dir("database", &self._database_parent)
    }

    /// Open an already-migrated production database. Serving and worker
    /// processes deliberately never change the schema on startup.
    pub async fn connect(config: &ServerConfig) -> Result<Self, DbError> {
        Self::connect_inner(config, false).await
    }

    /// Explicit administrative migration entry point. This is intentionally
    /// not called by either long-running binary.
    pub async fn migrate(config: &ServerConfig) -> Result<Self, DbError> {
        Self::connect_inner(config, true).await
    }

    async fn connect_inner(config: &ServerConfig, migrate: bool) -> Result<Self, DbError> {
        let runtime_fence_path = if config.allow_test_fence_provisioning {
            let database_parent = config.database_path.parent().ok_or_else(|| {
                DbError::Corrupt("database path has no parent directory".to_owned())
            })?;
            let path = database_parent.join("runtime-fence");
            if !path.exists() {
                std::fs::create_dir_all(database_parent).map_err(sqlx::Error::Io)?;
                provision_test_runtime_fence(&path).map_err(|error| {
                    DbError::Corrupt(format!("could not provision test runtime fence: {error:#}"))
                })?;
            }
            std::fs::canonicalize(path).map_err(sqlx::Error::Io)?
        } else {
            config.runtime_fence_directory.clone()
        };
        let runtime_fence = if config.allow_test_fence_provisioning {
            RuntimeDatabaseFence::open_test(&runtime_fence_path)
        } else {
            RuntimeDatabaseFence::open(&runtime_fence_path)
        }
        .map_err(|error| {
            DbError::Corrupt(format!("runtime database fence is invalid: {error:#}"))
        })?;
        let bootstrap_fence = runtime_fence
            .acquire_one_off_shared()
            .await
            .map_err(|error| {
                DbError::Corrupt(format!(
                    "could not acquire runtime database fence: {error:#}"
                ))
            })?;
        let parent = config
            .database_path
            .parent()
            .ok_or_else(|| DbError::Corrupt("database path has no parent directory".to_owned()))?;
        let leaf = config
            .database_path
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty())
            .ok_or_else(|| DbError::Corrupt("database filename is not valid UTF-8".to_owned()))?
            .to_owned();
        {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(sqlx::Error::Io)?;
            let metadata = tokio::fs::symlink_metadata(parent)
                .await
                .map_err(sqlx::Error::Io)?;
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                return Err(DbError::Corrupt(
                    "database directory must be a real directory".to_owned(),
                ));
            }
            set_private_permissions(parent, true).await?;
        }
        let pinned_parent_path = parent.to_owned();
        let database_parent = Arc::new(
            tokio::task::spawn_blocking(move || {
                crate::secure_fs::pin_private_root(&pinned_parent_path)
            })
            .await
            .map_err(|error| sqlx::Error::Io(std::io::Error::other(error)))?
            .map_err(sqlx::Error::Io)?,
        );
        let open_parent = Arc::clone(&database_parent);
        let open_leaf = leaf.clone();
        let database_file = Arc::new(
            tokio::task::spawn_blocking(move || {
                crate::secure_fs::open_private_database_file(
                    &open_parent,
                    std::path::Path::new(&open_leaf),
                    migrate,
                )
            })
            .await
            .map_err(|error| sqlx::Error::Io(std::io::Error::other(error)))?
            .map_err(sqlx::Error::Io)?,
        );
        #[cfg(target_os = "linux")]
        let database_open_path = {
            use std::os::fd::AsRawFd as _;
            PathBuf::from(format!("/proc/self/fd/{}", database_file.as_raw_fd()))
        };
        #[cfg(not(target_os = "linux"))]
        let database_open_path = config.database_path.clone();
        let options = SqliteConnectOptions::new()
            .filename(database_open_path)
            .create_if_missing(false)
            .foreign_keys(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .busy_timeout(Duration::from_millis(config.database_busy_timeout_ms));
        let pool = SqlitePoolOptions::new()
            .max_connections(16)
            .min_connections(0)
            .max_lifetime(None)
            .idle_timeout(None)
            .connect_with(options)
            .await?;
        if migrate {
            run_migrations(&pool).await?;
        } else {
            ensure_schema_current(&pool).await?;
        }
        verify_pinned_database_leaf(&database_parent, &leaf, &database_file).await?;
        let mut sidecars = Vec::new();
        for suffix in ["-wal", "-shm"] {
            let sidecar_name = format!("{leaf}{suffix}");
            let sidecar_parent = Arc::clone(&database_parent);
            match tokio::task::spawn_blocking(move || {
                crate::secure_fs::open_regular_file(
                    &sidecar_parent,
                    std::path::Path::new(&sidecar_name),
                )
            })
            .await
            .map_err(|error| sqlx::Error::Io(std::io::Error::other(error)))?
            {
                Ok(file) => {
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt as _;
                        if file
                            .metadata()
                            .map_err(sqlx::Error::Io)?
                            .permissions()
                            .mode()
                            & 0o777
                            != crate::secure_fs::SHARED_MUTABLE_FILE_MODE
                        {
                            file.set_permissions(std::fs::Permissions::from_mode(
                                crate::secure_fs::SHARED_MUTABLE_FILE_MODE,
                            ))
                            .map_err(sqlx::Error::Io)?;
                        }
                    }
                    sidecars.push(file);
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(DbError::Sql(sqlx::Error::Io(error))),
            }
        }
        wait_for_pool_idle(&pool).await.map_err(|error| {
            DbError::Corrupt(format!(
                "database pool did not quiesce after connect: {error:#}"
            ))
        })?;
        bootstrap_fence.revalidate().map_err(|error| {
            DbError::Corrupt(format!(
                "runtime database fence changed during connect: {error:#}"
            ))
        })?;
        drop(bootstrap_fence);
        let fence = ProcessDatabaseFenceManager::new(runtime_fence);
        Ok(Self {
            pool,
            max_pending_submissions: config.max_pending_submissions,
            max_concurrent_sensitive_writers: u64::try_from(config.max_concurrent_requests)
                .map_err(|_| {
                    DbError::ResultInvariant("sensitive writer limit overflows".to_owned())
                })?,
            max_concurrent_upload_writers: u64::try_from(config.max_concurrent_uploads).map_err(
                |_| DbError::ResultInvariant("upload writer limit overflows".to_owned()),
            )?,
            fence,
            _database_parent: database_parent,
            _database_file: database_file,
            _database_sidecars: Arc::new(sidecars),
        })
    }

    /// Enter one cancellation-safe process database generation. Long-running
    /// binaries place this around an owned request/job task; dropping the
    /// response waiter never drops the operation token.
    pub async fn begin_fenced_operation(&self) -> Result<DatabaseFenceOperation, DbError> {
        let mut operation = self.fence.begin().await.map_err(|error| {
            DbError::Corrupt(format!("database fence admission failed: {error:#}"))
        })?;
        match self.backup_lock_active().await {
            Ok(false) => Ok(operation),
            Ok(true) => {
                self.fence.mark_quiescing();
                self.fence
                    .finish(&mut operation, &self.pool)
                    .await
                    .map_err(|error| {
                        DbError::Corrupt(format!("database fence drain failed: {error:#}"))
                    })?;
                self.spawn_fence_reopen_probe();
                Err(DbError::QueueFull)
            }
            Err(error) => {
                self.fence.mark_quiescing();
                let finish = self.fence.finish(&mut operation, &self.pool).await;
                if let Err(finish) = finish {
                    return Err(DbError::Corrupt(format!(
                        "database gate check failed ({error}) and fence drain failed: {finish:#}"
                    )));
                }
                Err(error)
            }
        }
    }

    pub async fn finish_fenced_operation(
        &self,
        operation: &mut DatabaseFenceOperation,
    ) -> Result<(), DbError> {
        self.fence
            .finish(operation, &self.pool)
            .await
            .map_err(|error| DbError::Corrupt(format!("database fence drain failed: {error:#}")))
    }

    pub async fn run_fenced_operation<T, F>(&self, operation: F) -> anyhow::Result<T>
    where
        F: Future<Output = anyhow::Result<T>>,
    {
        use futures_util::FutureExt as _;

        let mut fence = self.begin_fenced_operation().await?;
        // A handler/job panic must unwind only after its SQLx future has been
        // dropped and the pool-return barrier has completed. Otherwise the
        // operation token's fail-closed Drop path intentionally retains the
        // process guard forever, turning a recoverable task panic into a
        // shutdown hang.
        let result = AssertUnwindSafe(operation).catch_unwind().await;
        let finish = self.finish_fenced_operation(&mut fence).await;
        match result {
            Ok(operation) => match (operation, finish) {
                (Ok(value), Ok(())) => Ok(value),
                (Ok(_), Err(error)) => Err(error.into()),
                (Err(operation), Ok(())) => Err(operation),
                (Err(operation), Err(finish)) => {
                    Err(operation.context(format!("database fence drain also failed: {finish}")))
                }
            },
            Err(_) => match finish {
                Ok(()) => anyhow::bail!("database operation panicked after fenced drain"),
                Err(error) => Err(anyhow::Error::from(error)
                    .context("database operation panicked and its fence drain also failed")),
            },
        }
    }

    fn spawn_fence_reopen_probe(&self) {
        let database = self.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(50)).await;
                let guard = match database.fence.runtime().acquire_one_off_shared().await {
                    Ok(guard) => guard,
                    Err(_) => continue,
                };
                let active = database.backup_lock_active().await;
                if wait_for_pool_idle(&database.pool).await.is_err() || guard.revalidate().is_err()
                {
                    continue;
                }
                drop(guard);
                if matches!(active, Ok(false)) {
                    database.fence.clear_quiescing();
                    break;
                }
            }
        });
    }

    pub fn runtime_fence(&self) -> &RuntimeDatabaseFence {
        self.fence.runtime()
    }

    pub async fn close_fenced(&self) -> anyhow::Result<()> {
        self.fence.close_pool_when_idle(&self.pool).await
    }

    pub async fn health_check(&self) -> Result<(), DbError> {
        let value: i64 = sqlx::query_scalar("SELECT 1").fetch_one(&self.pool).await?;
        if value != 1 {
            return Err(DbError::Corrupt(
                "health query returned a non-one value".to_owned(),
            ));
        }
        Ok(())
    }

    pub async fn operational_counts(&self) -> Result<OperationalCounts, DbError> {
        let now = now_epoch_ms()?;
        let row = sqlx::query(
            "SELECT \
                (SELECT COUNT(*) FROM submissions WHERE status IN ('queued','verifying','retry_pending') AND tombstoned_at_ms IS NULL) AS queued_submissions, \
                (SELECT COUNT(*) FROM submission_upload_reservations WHERE state IN ('reserved','uploaded') AND reservation_expires_at_ms >= ?) AS active_upload_reservations, \
                (SELECT COUNT(*) FROM submission_upload_reservations WHERE state = 'abandoned' AND reservation_expires_at_ms >= ?) AS abandoned_upload_reservations, \
                (SELECT COUNT(*) FROM verified_runs run JOIN submissions submission ON submission.id = run.submission_id WHERE submission.status = 'accepted' AND submission.tombstoned_at_ms IS NULL) AS accepted_runs, \
                (SELECT COUNT(*) FROM submissions WHERE status = 'rejected' AND tombstoned_at_ms IS NULL) AS rejected_retained_submissions, \
                (SELECT COUNT(*) FROM abuse_reports WHERE moderation_state IN ('open','reviewing')) AS open_abuse_reports, \
                (SELECT COUNT(*) FROM replay_objects WHERE purge_state = 'live') AS replay_objects_live, \
                (SELECT COUNT(*) FROM replay_objects WHERE purge_state = 'purging') AS replay_objects_purging, \
                (SELECT COUNT(*) FROM campaign_objects WHERE purge_state = 'live') AS campaign_objects_live, \
                (SELECT COUNT(*) FROM campaign_objects WHERE purge_state = 'purging') AS campaign_objects_purging",
        )
        .bind(now)
        .bind(now)
        .fetch_one(&self.pool)
        .await?;
        Ok(OperationalCounts {
            queued_submissions: nonnegative_u64(
                row.try_get("queued_submissions")?,
                "queued_submissions",
            )?,
            active_upload_reservations: nonnegative_u64(
                row.try_get("active_upload_reservations")?,
                "active_upload_reservations",
            )?,
            abandoned_upload_reservations: nonnegative_u64(
                row.try_get("abandoned_upload_reservations")?,
                "abandoned_upload_reservations",
            )?,
            accepted_runs: nonnegative_u64(row.try_get("accepted_runs")?, "accepted_runs")?,
            rejected_retained_submissions: nonnegative_u64(
                row.try_get("rejected_retained_submissions")?,
                "rejected_retained_submissions",
            )?,
            open_abuse_reports: nonnegative_u64(
                row.try_get("open_abuse_reports")?,
                "open_abuse_reports",
            )?,
            replay_objects_live: nonnegative_u64(
                row.try_get("replay_objects_live")?,
                "replay_objects_live",
            )?,
            replay_objects_purging: nonnegative_u64(
                row.try_get("replay_objects_purging")?,
                "replay_objects_purging",
            )?,
            campaign_objects_live: nonnegative_u64(
                row.try_get("campaign_objects_live")?,
                "campaign_objects_live",
            )?,
            campaign_objects_purging: nonnegative_u64(
                row.try_get("campaign_objects_purging")?,
                "campaign_objects_purging",
            )?,
        })
    }

    pub async fn register_campaign_object(
        &self,
        sha256: &[u8; 32],
        byte_length: u64,
    ) -> Result<(), DbError> {
        let now = now_epoch_ms()?;
        let byte_length = i64::try_from(byte_length)
            .map_err(|_| DbError::ResultInvariant("campaign length exceeds i64".to_owned()))?;
        sqlx::query(
            "INSERT INTO campaign_objects (sha256, byte_length, created_at_ms) VALUES (?, ?, ?) \
             ON CONFLICT(sha256) DO UPDATE SET purge_state = 'live', purge_token = NULL, \
                 purge_claimed_at_ms = NULL, purged_at_ms = NULL \
             WHERE campaign_objects.byte_length = excluded.byte_length \
               AND campaign_objects.purge_state = 'purged'",
        )
        .bind(sha256.as_slice())
        .bind(byte_length)
        .bind(now)
        .execute(&self.pool)
        .await?;
        let row =
            sqlx::query("SELECT byte_length, purge_state FROM campaign_objects WHERE sha256 = ?")
                .bind(sha256.as_slice())
                .fetch_one(&self.pool)
                .await?;
        if row.try_get::<i64, _>("byte_length")? != byte_length {
            return Err(DbError::Corrupt(
                "campaign object digest has conflicting byte length".to_owned(),
            ));
        }
        if row.try_get::<String, _>("purge_state")? != "live" {
            return Err(DbError::QueueFull);
        }
        Ok(())
    }

    pub async fn accepted_sequence_watermark(&self) -> Result<u64, DbError> {
        let sequence: i64 =
            sqlx::query_scalar("SELECT COALESCE(MAX(sequence), 0) FROM acceptance_sequences")
                .fetch_one(&self.pool)
                .await?;
        nonnegative_u64(sequence, "accepted_sequence_watermark")
    }

    pub async fn leaderboard_visibility_revision(&self) -> Result<u64, DbError> {
        let sequence: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(sequence), 0) FROM leaderboard_visibility_events",
        )
        .fetch_one(&self.pool)
        .await?;
        nonnegative_u64(sequence, "leaderboard_visibility_revision")
    }

    pub async fn issue_challenge(
        &self,
        purpose: ChallengePurpose,
        public_key: [u8; 32],
        ttl: Duration,
        offer_json: Option<&str>,
        public_metadata_json: Option<&str>,
    ) -> Result<IssuedChallenge, DbError> {
        let now = now_epoch_ms()?;
        let ttl_ms = i64::try_from(ttl.as_millis())
            .map_err(|_| DbError::Corrupt("challenge TTL does not fit i64".to_owned()))?;
        let expires = now
            .checked_add(ttl_ms)
            .ok_or_else(|| DbError::Corrupt("challenge expiry overflow".to_owned()))?;
        let id = uuid::Uuid::now_v7().to_string();
        let nonce: [u8; 32] = rand::random();
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        sqlx::query("DELETE FROM competition_run_grants WHERE competition_ends_at_ms <= ?")
            .bind(now)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "DELETE FROM upload_challenges WHERE consumed_at_ms IS NULL AND expires_at_ms < ? \
             AND NOT EXISTS (SELECT 1 FROM competition_run_grants g \
                             WHERE g.upload_challenge_id = upload_challenges.id)",
        )
        .bind(now)
        .execute(&mut *tx)
        .await?;
        let outstanding: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM upload_challenges \
             WHERE consumed_at_ms IS NULL AND expires_at_ms >= ? AND purpose = ?",
        )
        .bind(now)
        .bind(purpose.as_str())
        .fetch_one(&mut *tx)
        .await?;
        let purpose_multiplier = match purpose {
            ChallengePurpose::Submission => 2,
            ChallengePurpose::UsernameUpdate
            | ChallengePurpose::Deletion
            | ChallengePurpose::OwnerStatus => 1,
        };
        if outstanding >= i64::from(self.max_pending_submissions) * purpose_multiplier {
            return Err(DbError::QueueFull);
        }
        sqlx::query(
            "INSERT INTO challenge_generations (public_key, purpose, generation) VALUES (?, ?, 0) \
             ON CONFLICT(public_key, purpose) DO NOTHING",
        )
        .bind(public_key.as_slice())
        .bind(purpose.as_str())
        .execute(&mut *tx)
        .await?;
        let generation: i64 = sqlx::query_scalar(
            "UPDATE challenge_generations SET generation = generation + 1 \
             WHERE public_key = ? AND purpose = ? RETURNING generation",
        )
        .bind(public_key.as_slice())
        .bind(purpose.as_str())
        .fetch_one(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO upload_challenges \
             (id, nonce, purpose, public_key, generation, issued_at_ms, expires_at_ms, offer_json, \
              public_metadata_json) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(nonce.as_slice())
        .bind(purpose.as_str())
        .bind(public_key.as_slice())
        .bind(generation)
        .bind(now)
        .bind(expires)
        .bind(offer_json)
        .bind(public_metadata_json)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(IssuedChallenge {
            id,
            nonce,
            expires_at_ms: u64::try_from(expires)
                .map_err(|_| DbError::Corrupt("negative challenge expiry".to_owned()))?,
        })
    }

    /// Atomically issue or retry an unpredictable, one-use scheduled-run
    /// authorization. Server time is the only interval authority.
    pub async fn issue_competition_run_grant<F>(
        &self,
        request: &CompetitionRunGrantRequestV1,
        competition_starts_at_ms: u64,
        competition_ends_at_ms: u64,
        build_grant: F,
    ) -> Result<CompetitionRunGrantV1, DbError>
    where
        F: FnOnce(&IssuedCompetitionRunGrant) -> Result<CompetitionRunGrantV1, DbError> + Send,
    {
        request
            .validate()
            .map_err(|error| DbError::ResultInvariant(error.to_string()))?;
        let request_sha256 = request
            .canonical_digest()
            .map_err(|error| DbError::ResultInvariant(error.to_string()))?;
        let ranked_sha256 = request
            .claim
            .ranked_session
            .canonical_digest()
            .map_err(|error| DbError::ResultInvariant(error.to_string()))?;
        let competition_sha256 = request
            .claim
            .ranked_session
            .competition_manifest_sha256
            .ok_or_else(|| DbError::ResultInvariant("grant request has no competition".into()))?;
        let now = now_epoch_ms()?;
        let starts = i64::try_from(competition_starts_at_ms)
            .map_err(|_| DbError::ResultInvariant("competition start exceeds i64".into()))?;
        let ends = i64::try_from(competition_ends_at_ms)
            .map_err(|_| DbError::ResultInvariant("competition end exceeds i64".into()))?;
        if now < starts || now >= ends {
            return Err(DbError::InvalidChallenge);
        }
        // The grant must remain usable for a complete mission, which can be
        // much longer than the short post-run upload-offer TTL. Its sole
        // deadline is therefore the competition's exclusive end.
        let expires = ends - 1;
        if expires <= now {
            return Err(DbError::InvalidChallenge);
        }
        let request_json = serde_json::to_string(request)
            .map_err(|error| DbError::ResultInvariant(error.to_string()))?;
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        sqlx::query("DELETE FROM competition_run_grants WHERE competition_ends_at_ms <= ?")
            .bind(now)
            .execute(&mut *tx)
            .await?;
        if let Some(row) = sqlx::query(
            "SELECT grant_json, expires_at_ms, invalidated_at_ms, consumed_at_ms \
             FROM competition_run_grants WHERE request_sha256 = ?",
        )
        .bind(request_sha256.as_bytes().as_slice())
        .fetch_optional(&mut *tx)
        .await?
        {
            if row.try_get::<i64, _>("expires_at_ms")? >= now
                && row
                    .try_get::<Option<i64>, _>("invalidated_at_ms")?
                    .is_none()
                && row.try_get::<Option<i64>, _>("consumed_at_ms")?.is_none()
            {
                let stored: CompetitionRunGrantV1 =
                    serde_json::from_str(row.try_get::<String, _>("grant_json")?.as_str())
                        .map_err(|error| {
                            DbError::Corrupt(format!("competition grant JSON: {error}"))
                        })?;
                stored
                    .validate_request(request)
                    .map_err(|error| DbError::Corrupt(error.to_string()))?;
                tx.commit().await?;
                return Ok(stored);
            }
            return Err(DbError::InvalidChallenge);
        }
        // A malicious client must not be able to turn grants into a bank of
        // concurrently-live upload offers by requesting each offer before it
        // actually plays. Once a grant has been exchanged for an offer, keep
        // that host/competition lane closed until the offer is either consumed
        // by a completed upload or expires under server time.
        let outstanding_offer: i64 = sqlx::query_scalar(
            "SELECT EXISTS(\
                 SELECT 1 FROM competition_run_grants g \
                 JOIN upload_challenges c ON c.id = g.upload_challenge_id \
                 WHERE g.host_public_key = ? AND g.competition_manifest_id = ? \
                   AND g.invalidated_at_ms IS NULL AND g.consumed_at_ms IS NOT NULL \
                   AND g.completed_at_ms IS NULL \
                   AND c.expires_at_ms >= ?\
             )",
        )
        .bind(request.claim.host_public_key.as_bytes().as_slice())
        .bind(competition_sha256.as_bytes().as_slice())
        .bind(now)
        .fetch_one(&mut *tx)
        .await?;
        if outstanding_offer != 0 {
            return Err(DbError::InvalidChallenge);
        }
        let replay_session_seen: i64 = sqlx::query_scalar(
            "SELECT EXISTS(\
                 SELECT 1 FROM competition_run_grants \
                 WHERE host_public_key = ? AND competition_manifest_id = ? \
                   AND replay_session_id = ?\
             )",
        )
        .bind(request.claim.host_public_key.as_bytes().as_slice())
        .bind(competition_sha256.as_bytes().as_slice())
        .bind(request.claim.replay_session_id.as_bytes().as_slice())
        .fetch_one(&mut *tx)
        .await?;
        if replay_session_seen != 0 {
            return Err(DbError::InvalidChallenge);
        }
        sqlx::query(
            "UPDATE competition_run_grants SET invalidated_at_ms = ? \
             WHERE host_public_key = ? AND competition_manifest_id = ? \
               AND invalidated_at_ms IS NULL AND consumed_at_ms IS NULL",
        )
        .bind(now)
        .bind(request.claim.host_public_key.as_bytes().as_slice())
        .bind(competition_sha256.as_bytes().as_slice())
        .execute(&mut *tx)
        .await?;
        let issued = IssuedCompetitionRunGrant {
            id: uuid::Uuid::now_v7().to_string(),
            nonce: rand::random(),
            admitted_at_ms: u64::try_from(now)
                .map_err(|_| DbError::Corrupt("negative grant admission time".into()))?,
            expires_at_ms: u64::try_from(expires)
                .map_err(|_| DbError::Corrupt("negative grant expiry".into()))?,
        };
        let grant = build_grant(&issued)?;
        grant
            .validate_request(request)
            .map_err(|error| DbError::ResultInvariant(error.to_string()))?;
        let grant_json = serde_json::to_string(&grant)
            .map_err(|error| DbError::ResultInvariant(error.to_string()))?;
        sqlx::query(
            "INSERT INTO competition_run_grants \
             (id, nonce, host_public_key, competition_manifest_id, ranked_session_sha256, \
              request_sha256, replay_session_id, request_json, grant_json, admitted_at_ms, \
              expires_at_ms, competition_ends_at_ms) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&issued.id)
        .bind(issued.nonce.as_slice())
        .bind(request.claim.host_public_key.as_bytes().as_slice())
        .bind(competition_sha256.as_bytes().as_slice())
        .bind(ranked_sha256.as_bytes().as_slice())
        .bind(request_sha256.as_bytes().as_slice())
        .bind(request.claim.replay_session_id.as_bytes().as_slice())
        .bind(request_json)
        .bind(grant_json)
        .bind(now)
        .bind(expires)
        .bind(ends)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(grant)
    }

    /// Generate and persist a submission challenge only if its complete offer
    /// can be constructed. The callback is synchronous and runs inside the
    /// write transaction, so validation/serialization failures cannot leave a
    /// quota-consuming row with a NULL offer.
    pub async fn issue_submission_offer<T, F>(
        &self,
        public_key: [u8; 32],
        ttl: Duration,
        competition_grant: Option<&CompetitionRunGrantV1>,
        build_offer: F,
    ) -> Result<(IssuedChallenge, T), DbError>
    where
        T: Send,
        F: FnOnce(&IssuedChallenge) -> Result<(T, String, String), DbError> + Send,
    {
        let now = now_epoch_ms()?;
        let ttl_ms = i64::try_from(ttl.as_millis())
            .map_err(|_| DbError::Corrupt("challenge TTL does not fit i64".to_owned()))?;
        let mut expires = now
            .checked_add(ttl_ms)
            .ok_or_else(|| DbError::Corrupt("challenge expiry overflow".to_owned()))?;
        if let Some(grant) = competition_grant {
            let grant_expiry = i64::try_from(grant.claim.expires_at_unix_ms)
                .map_err(|_| DbError::ResultInvariant("grant expiry exceeds i64".into()))?;
            expires = expires.min(grant_expiry);
        }
        let issued = IssuedChallenge {
            id: uuid::Uuid::now_v7().to_string(),
            nonce: rand::random(),
            expires_at_ms: u64::try_from(expires)
                .map_err(|_| DbError::Corrupt("negative challenge expiry".to_owned()))?,
        };
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        sqlx::query("DELETE FROM competition_run_grants WHERE competition_ends_at_ms <= ?")
            .bind(now)
            .execute(&mut *tx)
            .await?;
        if let Some(grant) = competition_grant {
            let grant_json = serde_json::to_string(grant)
                .map_err(|error| DbError::ResultInvariant(error.to_string()))?;
            let row = sqlx::query(
                "SELECT grant_json, host_public_key, expires_at_ms, invalidated_at_ms, consumed_at_ms \
                 FROM competition_run_grants WHERE id = ?",
            )
            .bind(grant.claim.grant_id.as_str())
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(DbError::InvalidChallenge)?;
            if row.try_get::<String, _>("grant_json")? != grant_json
                || row.try_get::<Vec<u8>, _>("host_public_key")? != public_key
                || row.try_get::<i64, _>("expires_at_ms")? < now
                || row
                    .try_get::<Option<i64>, _>("invalidated_at_ms")?
                    .is_some()
                || row.try_get::<Option<i64>, _>("consumed_at_ms")?.is_some()
            {
                return Err(DbError::InvalidChallenge);
            }
        }
        sqlx::query(
            "DELETE FROM upload_challenges WHERE consumed_at_ms IS NULL AND expires_at_ms < ? \
             AND NOT EXISTS (SELECT 1 FROM competition_run_grants g \
                             WHERE g.upload_challenge_id = upload_challenges.id)",
        )
        .bind(now)
        .execute(&mut *tx)
        .await?;
        let outstanding: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM upload_challenges \
             WHERE consumed_at_ms IS NULL AND expires_at_ms >= ? AND purpose = 'submission'",
        )
        .bind(now)
        .fetch_one(&mut *tx)
        .await?;
        if outstanding >= i64::from(self.max_pending_submissions) * 2 {
            return Err(DbError::QueueFull);
        }
        sqlx::query(
            "INSERT INTO challenge_generations (public_key, purpose, generation) \
             VALUES (?, 'submission', 0) ON CONFLICT(public_key, purpose) DO NOTHING",
        )
        .bind(public_key.as_slice())
        .execute(&mut *tx)
        .await?;
        let generation: i64 = sqlx::query_scalar(
            "UPDATE challenge_generations SET generation = generation + 1 \
             WHERE public_key = ? AND purpose = 'submission' RETURNING generation",
        )
        .bind(public_key.as_slice())
        .fetch_one(&mut *tx)
        .await?;
        let (offer, offer_json, public_metadata_json) = build_offer(&issued)?;
        sqlx::query(
            "INSERT INTO upload_challenges \
             (id, nonce, purpose, public_key, generation, issued_at_ms, expires_at_ms, offer_json, \
              public_metadata_json) VALUES (?, ?, 'submission', ?, ?, ?, ?, ?, ?)",
        )
        .bind(&issued.id)
        .bind(issued.nonce.as_slice())
        .bind(public_key.as_slice())
        .bind(generation)
        .bind(now)
        .bind(expires)
        .bind(offer_json)
        .bind(public_metadata_json)
        .execute(&mut *tx)
        .await?;
        if let Some(grant) = competition_grant {
            let consumed = sqlx::query(
                "UPDATE competition_run_grants SET consumed_at_ms = ?, upload_challenge_id = ? \
                 WHERE id = ? AND invalidated_at_ms IS NULL AND consumed_at_ms IS NULL \
                   AND expires_at_ms >= ?",
            )
            .bind(now)
            .bind(&issued.id)
            .bind(grant.claim.grant_id.as_str())
            .bind(now)
            .execute(&mut *tx)
            .await?;
            if consumed.rows_affected() != 1 {
                return Err(DbError::InvalidChallenge);
            }
        }
        tx.commit().await?;
        Ok((issued, offer))
    }

    /// Issue an owner-status challenge without consulting submission storage.
    /// This intentionally cannot reveal whether the requested ID exists or is
    /// controlled by the supplied key.
    pub async fn issue_owner_status_challenge(
        &self,
        controller_public_key: [u8; 32],
        submission_id: &str,
        ttl: Duration,
    ) -> Result<IssuedChallenge, DbError> {
        let now = now_epoch_ms()?;
        let ttl_ms = i64::try_from(ttl.as_millis())
            .map_err(|_| DbError::Corrupt("challenge TTL does not fit i64".to_owned()))?;
        let expires = now
            .checked_add(ttl_ms)
            .ok_or_else(|| DbError::Corrupt("challenge expiry overflow".to_owned()))?;
        let issued = IssuedChallenge {
            id: uuid::Uuid::now_v7().to_string(),
            nonce: rand::random(),
            expires_at_ms: u64::try_from(expires)
                .map_err(|_| DbError::Corrupt("negative challenge expiry".to_owned()))?,
        };
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        sqlx::query(
            "DELETE FROM submission_owner_status_challenges \
             WHERE consumed_at_ms IS NOT NULL OR expires_at_ms < ?",
        )
        .bind(now)
        .execute(&mut *tx)
        .await?;
        let outstanding: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM submission_owner_status_challenges \
             WHERE consumed_at_ms IS NULL",
        )
        .fetch_one(&mut *tx)
        .await?;
        if outstanding >= i64::from(self.max_pending_submissions) * 2 {
            return Err(DbError::QueueFull);
        }
        sqlx::query(
            "INSERT INTO submission_owner_status_challenges \
             (id, nonce, controller_public_key, submission_id, issued_at_ms, expires_at_ms) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&issued.id)
        .bind(issued.nonce.as_slice())
        .bind(controller_public_key.as_slice())
        .bind(submission_id)
        .bind(now)
        .bind(expires)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(issued)
    }

    /// Atomically consumes a one-use challenge, then evaluates ownership. A
    /// missing, deleted, or differently controlled submission has the same
    /// externally visible error as any other invalid challenge.
    pub async fn consume_owner_status_challenge(
        &self,
        challenge_id: &str,
        challenge_nonce: [u8; 32],
        expires_at_ms: u64,
        controller_public_key: [u8; 32],
        submission_id: &str,
    ) -> Result<SubmissionLifecycle, DbError> {
        let now = now_epoch_ms()?;
        let expires_at_ms = i64::try_from(expires_at_ms).map_err(|_| DbError::InvalidChallenge)?;
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let consumed = sqlx::query(
            "UPDATE submission_owner_status_challenges SET consumed_at_ms = ? \
             WHERE id = ? AND nonce = ? AND controller_public_key = ? AND submission_id = ? \
               AND expires_at_ms = ? AND expires_at_ms >= ? AND consumed_at_ms IS NULL",
        )
        .bind(now)
        .bind(challenge_id)
        .bind(challenge_nonce.as_slice())
        .bind(controller_public_key.as_slice())
        .bind(submission_id)
        .bind(expires_at_ms)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        if consumed.rows_affected() != 1 {
            tx.commit().await?;
            return Err(DbError::InvalidChallenge);
        }

        let row = sqlx::query(
            "SELECT s.id, CASE WHEN f.submission_id IS NULL THEN s.status ELSE 'failed' END AS status, \
                    s.rejection_code, s.created_at_ms, s.updated_at_ms, \
                    r.id AS run_id \
             FROM submissions s LEFT JOIN verified_runs r ON r.submission_id = s.id \
             LEFT JOIN submission_terminal_failures f ON f.submission_id = s.id \
             WHERE s.id = ? AND s.controller_public_key = ?",
        )
        .bind(submission_id)
        .bind(controller_public_key.as_slice())
        .fetch_optional(&mut *tx)
        .await?;
        let lifecycle = row.map(lifecycle_from_row).transpose()?;
        tx.commit().await?;
        lifecycle.ok_or(DbError::InvalidChallenge)
    }

    pub async fn attach_offer(
        &self,
        challenge_id: &str,
        offer_json: &str,
        public_metadata_json: &str,
    ) -> Result<(), DbError> {
        let changed = sqlx::query(
            "UPDATE upload_challenges SET offer_json = ?, public_metadata_json = ? \
             WHERE id = ? AND purpose = 'submission' AND consumed_at_ms IS NULL \
                 AND offer_json IS NULL AND public_metadata_json IS NULL",
        )
        .bind(offer_json)
        .bind(public_metadata_json)
        .bind(challenge_id)
        .execute(&self.pool)
        .await?;
        if changed.rows_affected() != 1 {
            return Err(DbError::InvalidChallenge);
        }
        Ok(())
    }

    pub async fn attach_deletion_challenge(
        &self,
        challenge_id: &str,
        challenge_json: &str,
    ) -> Result<(), DbError> {
        let changed = sqlx::query(
            "UPDATE upload_challenges SET offer_json = ? \
             WHERE id = ? AND purpose = 'deletion' AND consumed_at_ms IS NULL \
                 AND offer_json IS NULL",
        )
        .bind(challenge_json)
        .bind(challenge_id)
        .execute(&self.pool)
        .await?;
        if changed.rows_affected() != 1 {
            return Err(DbError::InvalidChallenge);
        }
        Ok(())
    }

    pub async fn stored_offer(&self, challenge_id: &str) -> Result<StoredOffer, DbError> {
        let now = now_epoch_ms()?;
        let row = sqlx::query(
            "SELECT offer_json, public_metadata_json FROM upload_challenges \
             WHERE id = ? AND purpose = 'submission' AND expires_at_ms >= ? \
               AND (consumed_at_ms IS NULL OR EXISTS (\
                    SELECT 1 FROM submission_upload_reservations r \
                    WHERE r.upload_challenge_id = upload_challenges.id))",
        )
        .bind(challenge_id)
        .bind(now)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(DbError::InvalidChallenge)?;
        Ok(StoredOffer {
            offer_json: row
                .try_get::<Option<String>, _>("offer_json")?
                .ok_or_else(|| DbError::Corrupt("submission challenge has no offer".to_owned()))?,
            public_metadata_json: row
                .try_get::<Option<String>, _>("public_metadata_json")?
                .ok_or_else(|| {
                    DbError::Corrupt("submission challenge has no metadata snapshot".to_owned())
                })?,
        })
    }

    pub async fn apply_deletion(
        &self,
        challenge_id: &str,
        challenge_nonce: [u8; 32],
        public_key: [u8; 32],
        target_kind: &str,
        target_id: &str,
        challenge_json: &str,
        request_json: &str,
        retention: Option<Duration>,
    ) -> Result<DeletionRecord, DbError> {
        let now = now_epoch_ms()?;
        let purge = retention
            .map(|duration| {
                i64::try_from(duration.as_millis())
                    .ok()
                    .and_then(|millis| now.checked_add(millis))
                    .ok_or_else(|| DbError::ResultInvariant("retention overflow".to_owned()))
            })
            .transpose()?;
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let challenge = sqlx::query(
            "SELECT purpose, public_key, nonce, expires_at_ms, consumed_at_ms, offer_json \
             FROM upload_challenges WHERE id = ?",
        )
        .bind(challenge_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(DbError::InvalidChallenge)?;
        if challenge.try_get::<String, _>("purpose")? != ChallengePurpose::Deletion.as_str()
            || challenge.try_get::<Vec<u8>, _>("public_key")?.as_slice() != public_key
            || challenge.try_get::<Vec<u8>, _>("nonce")?.as_slice() != challenge_nonce
            || challenge.try_get::<i64, _>("expires_at_ms")? < now
            || challenge
                .try_get::<Option<i64>, _>("consumed_at_ms")?
                .is_some()
            || challenge
                .try_get::<Option<String>, _>("offer_json")?
                .as_deref()
                != Some(challenge_json)
        {
            return Err(DbError::InvalidChallenge);
        }
        let mut aggregate_run_id: Option<String> = None;
        let submission_id: Option<String> = match target_kind {
            "submission" => {
                sqlx::query_scalar(
                    "SELECT s.id FROM submissions s JOIN submission_participants sp \
                 ON sp.submission_id = s.id WHERE s.id = ? AND sp.seat = 0 AND sp.public_key = ?",
                )
                .bind(target_id)
                .bind(public_key.as_slice())
                .fetch_optional(&mut *tx)
                .await?
            }
            "run" => {
                let submission: Option<String> = sqlx::query_scalar(
                    "SELECT r.submission_id FROM verified_runs r JOIN submission_participants sp \
                 ON sp.submission_id = r.submission_id \
                 WHERE r.id = ? AND sp.seat = 0 AND sp.public_key = ?",
                )
                .bind(target_id)
                .bind(public_key.as_slice())
                .fetch_optional(&mut *tx)
                .await?;
                if submission.is_none() {
                    aggregate_run_id = sqlx::query_scalar(
                        "SELECT fc.id FROM full_campaign_runs fc \
                             JOIN full_campaign_sessions fcs \
                               ON fcs.full_campaign_run_id = fc.id AND fcs.ordinal = 0 \
                             JOIN verified_runs first_run ON first_run.id = fcs.run_id \
                             JOIN submission_participants first_host \
                               ON first_host.submission_id = first_run.submission_id \
                              AND first_host.seat = 0 \
                             WHERE fc.id = ? AND first_host.public_key = ? \
                               AND fc.tombstoned_at_ms IS NULL",
                    )
                    .bind(target_id)
                    .bind(public_key.as_slice())
                    .fetch_optional(&mut *tx)
                    .await?;
                }
                submission
            }
            _ => {
                return Err(DbError::ResultInvariant(
                    "invalid deletion target".to_owned(),
                ));
            }
        };
        let (changed, visibility_changed) = if let Some(submission_id) = submission_id {
            let was_public: i64 = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM verified_runs WHERE submission_id = ?)",
            )
            .bind(&submission_id)
            .fetch_one(&mut *tx)
            .await?;
            let changed = sqlx::query(
                "UPDATE submissions SET tombstoned_at_ms = ?, purge_eligible_at_ms = ?, \
                lease_owner = NULL, lease_expires_at_ms = NULL, \
                status = CASE WHEN status = 'verifying' THEN 'retry_pending' ELSE status END, \
                updated_at_ms = ? WHERE id = ? AND tombstoned_at_ms IS NULL",
            )
            .bind(now)
            .bind(purge)
            .bind(now)
            .bind(&submission_id)
            .execute(&mut *tx)
            .await?;
            (changed, was_public != 0)
        } else if let Some(aggregate_run_id) = aggregate_run_id {
            let changed = sqlx::query(
                "UPDATE full_campaign_runs SET tombstoned_at_ms = ? \
                 WHERE id = ? AND tombstoned_at_ms IS NULL",
            )
            .bind(now)
            .bind(aggregate_run_id)
            .execute(&mut *tx)
            .await?;
            (changed, true)
        } else {
            return Err(DbError::NotFound);
        };
        if changed.rows_affected() != 1 {
            return Err(DbError::NotFound);
        }
        if visibility_changed {
            sqlx::query("INSERT INTO leaderboard_visibility_events (created_at_ms) VALUES (?)")
                .bind(now)
                .execute(&mut *tx)
                .await?;
        }
        let deletion_id = uuid::Uuid::now_v7().to_string();
        sqlx::query(
            "INSERT INTO deletion_requests (id, challenge_id, owner_public_key, target_kind, \
                target_id, request_json, tombstoned_at_ms, purge_eligible_at_ms) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&deletion_id)
        .bind(challenge_id)
        .bind(public_key.as_slice())
        .bind(target_kind)
        .bind(target_id)
        .bind(request_json)
        .bind(now)
        .bind(purge)
        .execute(&mut *tx)
        .await?;
        let consumed = sqlx::query(
            "UPDATE upload_challenges SET consumed_at_ms = ? \
             WHERE id = ? AND consumed_at_ms IS NULL",
        )
        .bind(now)
        .bind(challenge_id)
        .execute(&mut *tx)
        .await?;
        if consumed.rows_affected() != 1 {
            return Err(DbError::InvalidChallenge);
        }
        tx.commit().await?;
        Ok(DeletionRecord {
            id: deletion_id,
            tombstoned_at_ms: u64::try_from(now)
                .map_err(|_| DbError::Corrupt("negative tombstone timestamp".to_owned()))?,
            purge_eligible_at_ms: purge
                .map(u64::try_from)
                .transpose()
                .map_err(|_| DbError::Corrupt("negative purge timestamp".to_owned()))?,
        })
    }

    pub async fn insert_abuse_report(
        &self,
        target_kind: &str,
        target_id: &str,
        active_ruleset_ids: &[[u8; 32]],
        category: &str,
        detail: &str,
        reporter_ip_hash: [u8; 32],
        per_ip_limit: u32,
        per_key_limit: u32,
        per_target_limit: u32,
    ) -> Result<(String, u64), DbError> {
        let now = now_epoch_ms()?;
        let one_hour_ago = now.saturating_sub(60 * 60 * 1000);
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let quota_public_key: [u8; 32] = match target_kind {
            "run" => {
                let mission_ruleset =
                    active_ruleset_predicate("run.ruleset_id", active_ruleset_ids);
                let aggregate_ruleset =
                    active_ruleset_predicate("aggregate.ruleset_id", active_ruleset_ids);
                let statement = format!(
                    "SELECT participant.public_key FROM verified_runs run \
                     JOIN submissions submission ON submission.id = run.submission_id \
                     JOIN submission_participants participant \
                       ON participant.submission_id = submission.id AND participant.seat = 0 \
                     WHERE run.id = ? AND submission.tombstoned_at_ms IS NULL \
                       AND {mission_ruleset} \
                       AND (run.campaign_session_kind IS NULL \
                            OR run.campaign_session_kind = 'field_mission') \
                     UNION ALL \
                     SELECT participant.public_key FROM full_campaign_runs aggregate \
                     JOIN full_campaign_sessions session \
                       ON session.full_campaign_run_id = aggregate.id AND session.ordinal = 0 \
                     JOIN verified_runs run ON run.id = session.run_id \
                     JOIN submissions submission ON submission.id = run.submission_id \
                     JOIN submission_participants participant \
                       ON participant.submission_id = submission.id AND participant.seat = 0 \
                     WHERE aggregate.id = ? AND aggregate.tombstoned_at_ms IS NULL \
                       AND {aggregate_ruleset} \
                       AND NOT EXISTS (SELECT 1 FROM full_campaign_sessions linked_session \
                           JOIN verified_runs linked_run ON linked_run.id = linked_session.run_id \
                           JOIN submissions linked_submission \
                             ON linked_submission.id = linked_run.submission_id \
                           WHERE linked_session.full_campaign_run_id = aggregate.id \
                             AND linked_submission.tombstoned_at_ms IS NOT NULL) \
                     LIMIT 1"
                );
                // SQL contains only fixed column names and hex-encoded ruleset digests; values are bound.
                let key: Option<Vec<u8>> =
                    sqlx::query_scalar(sqlx::AssertSqlSafe(statement.as_str()))
                        .bind(target_id)
                        .bind(target_id)
                        .fetch_optional(&mut *tx)
                        .await?;
                fixed_32(key.ok_or(DbError::NotFound)?)?
            }
            "player" => {
                let key = hex::decode(target_id).map_err(|_| DbError::NotFound)?;
                let key = fixed_32(key)?;
                let exists: i64 = sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM identities WHERE public_key = ?)",
                )
                .bind(key.as_slice())
                .fetch_one(&mut *tx)
                .await?;
                if exists == 0 {
                    return Err(DbError::NotFound);
                }
                key
            }
            _ => return Err(DbError::ResultInvariant("invalid report target".to_owned())),
        };
        let global: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM abuse_reports WHERE received_at_ms >= ?")
                .bind(one_hour_ago)
                .fetch_one(&mut *tx)
                .await?;
        let target_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM abuse_reports WHERE received_at_ms >= ? \
             AND target_kind = ? AND target_id = ?",
        )
        .bind(one_hour_ago)
        .bind(target_kind)
        .bind(target_id)
        .fetch_one(&mut *tx)
        .await?;
        let ip_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM abuse_reports WHERE received_at_ms >= ? \
             AND reporter_ip_hash = ?",
        )
        .bind(one_hour_ago)
        .bind(reporter_ip_hash.as_slice())
        .fetch_one(&mut *tx)
        .await?;
        let key_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM abuse_reports WHERE received_at_ms >= ? \
             AND quota_public_key = ?",
        )
        .bind(one_hour_ago)
        .bind(quota_public_key.as_slice())
        .fetch_one(&mut *tx)
        .await?;
        if global >= 1_000
            || target_count >= i64::from(per_target_limit)
            || ip_count >= i64::from(per_ip_limit)
            || key_count >= i64::from(per_key_limit)
        {
            return Err(DbError::QueueFull);
        }
        let id = uuid::Uuid::now_v7().to_string();
        sqlx::query(
            "INSERT INTO abuse_reports \
             (id, target_kind, target_id, category, detail, received_at_ms, \
              reporter_ip_hash, quota_public_key, updated_at_ms) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(target_kind)
        .bind(target_id)
        .bind(category)
        .bind(detail)
        .bind(now)
        .bind(reporter_ip_hash.as_slice())
        .bind(quota_public_key.as_slice())
        .bind(now)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok((
            id,
            u64::try_from(now)
                .map_err(|_| DbError::Corrupt("negative report timestamp".to_owned()))?,
        ))
    }

    pub async fn moderation_reports(
        &self,
        state: Option<&str>,
        limit: u32,
    ) -> Result<Vec<ModerationReportRecord>, DbError> {
        if limit == 0 || limit > 500 {
            return Err(DbError::ResultInvariant(
                "moderation report limit must be in 1..=500".to_owned(),
            ));
        }
        if state
            .is_some_and(|value| !matches!(value, "open" | "reviewing" | "dismissed" | "actioned"))
        {
            return Err(DbError::ResultInvariant(
                "invalid moderation state".to_owned(),
            ));
        }
        let rows = sqlx::query(
            "SELECT id, target_kind, target_id, category, detail, received_at_ms, \
                    moderation_state, moderator_note FROM abuse_reports \
             WHERE (? IS NULL OR moderation_state = ?) \
             ORDER BY received_at_ms, id LIMIT ?",
        )
        .bind(state)
        .bind(state)
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(ModerationReportRecord {
                    id: row.try_get("id")?,
                    target_kind: row.try_get("target_kind")?,
                    target_id: row.try_get("target_id")?,
                    category: row.try_get("category")?,
                    detail: row.try_get("detail")?,
                    received_at_ms: nonnegative_u64(
                        row.try_get("received_at_ms")?,
                        "received_at_ms",
                    )?,
                    moderation_state: row.try_get("moderation_state")?,
                    moderator_note: row.try_get("moderator_note")?,
                })
            })
            .collect()
    }

    pub async fn moderate_report(
        &self,
        report_id: &str,
        new_state: &str,
        detail: &str,
        operator_id: &str,
    ) -> Result<(), DbError> {
        if !matches!(new_state, "reviewing" | "dismissed" | "actioned")
            || detail.is_empty()
            || detail.len() > 4_000
            || operator_id.is_empty()
            || operator_id.len() > 128
        {
            return Err(DbError::ResultInvariant(
                "invalid moderation action".to_owned(),
            ));
        }
        let now = now_epoch_ms()?;
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let previous: String =
            sqlx::query_scalar("SELECT moderation_state FROM abuse_reports WHERE id = ?")
                .bind(report_id)
                .fetch_optional(&mut *tx)
                .await?
                .ok_or(DbError::NotFound)?;
        if matches!(previous.as_str(), "dismissed" | "actioned") && previous != new_state {
            return Err(DbError::ResultInvariant(
                "closed moderation reports cannot be reopened or rewritten".to_owned(),
            ));
        }
        sqlx::query(
            "UPDATE abuse_reports SET moderation_state = ?, moderator_note = ?, updated_at_ms = ? \
             WHERE id = ?",
        )
        .bind(new_state)
        .bind(detail)
        .bind(now)
        .bind(report_id)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO moderation_audit \
             (report_id, action, previous_state, new_state, detail, operator_id, created_at_ms) \
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(report_id)
        .bind(new_state)
        .bind(previous)
        .bind(new_state)
        .bind(detail)
        .bind(operator_id)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn moderation_audit(
        &self,
        report_id: Option<&str>,
        limit: u32,
    ) -> Result<Vec<ModerationAuditRecord>, DbError> {
        if limit == 0 || limit > 500 {
            return Err(DbError::ResultInvariant(
                "moderation audit limit must be in 1..=500".to_owned(),
            ));
        }
        let rows = sqlx::query(
            "SELECT id, report_id, action, previous_state, new_state, detail, operator_id, \
                    created_at_ms FROM moderation_audit WHERE (? IS NULL OR report_id = ?) \
             ORDER BY id DESC LIMIT ?",
        )
        .bind(report_id)
        .bind(report_id)
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(ModerationAuditRecord {
                    id: nonnegative_u64(row.try_get("id")?, "moderation audit id")?,
                    report_id: row.try_get("report_id")?,
                    action: row.try_get("action")?,
                    previous_state: row.try_get("previous_state")?,
                    new_state: row.try_get("new_state")?,
                    detail: row.try_get("detail")?,
                    operator_id: row.try_get("operator_id")?,
                    created_at_ms: nonnegative_u64(row.try_get("created_at_ms")?, "created_at_ms")?,
                })
            })
            .collect()
    }

    pub async fn campaign_predecessor(&self, run_id: &str) -> Result<CampaignPredecessor, DbError> {
        let row = sqlx::query(
            "SELECT r.id, r.campaign_chain_id, r.result_sha256, r.verification_request_sha256, \
                    r.verification_result_json, r.final_campaign_sha256, r.final_campaign_bytes, \
                    r.content_manifest_id, r.campaign_content_manifest_id, r.config_id, r.ruleset_id, r.competition_manifest_id, \
                    r.campaign_session_ordinal, r.max_concurrent_players, \
                    r.participant_instance_count \
             FROM verified_runs r JOIN submissions s ON s.id = r.submission_id \
             WHERE r.id = ? AND s.status = 'accepted' AND s.tombstoned_at_ms IS NULL \
                 AND r.scope_kind = 'campaign' \
                 AND NOT EXISTS (SELECT 1 FROM full_campaign_runs fc \
                     WHERE fc.chain_id = r.campaign_chain_id) \
                 AND NOT EXISTS (SELECT 1 FROM submissions continuation \
                     WHERE continuation.predecessor_run_id = r.id \
                       AND continuation.status = 'accepted' \
                       AND continuation.tombstoned_at_ms IS NULL)",
        )
        .bind(run_id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(DbError::NotFound)?;
        let broken_ancestor: i64 = sqlx::query_scalar(
            "WITH RECURSIVE chain(run_id, predecessor_run_id) AS ( \
                 SELECT r.id, s.predecessor_run_id FROM verified_runs r \
                 JOIN submissions s ON s.id = r.submission_id WHERE r.id = ? \
                 UNION ALL \
                 SELECT predecessor.id, predecessor_submission.predecessor_run_id \
                 FROM chain current \
                 JOIN verified_runs predecessor ON predecessor.id = current.predecessor_run_id \
                 JOIN submissions predecessor_submission \
                   ON predecessor_submission.id = predecessor.submission_id \
             ) \
             SELECT EXISTS(SELECT 1 FROM chain \
                 JOIN verified_runs chain_run ON chain_run.id = chain.run_id \
                 JOIN submissions chain_submission \
                   ON chain_submission.id = chain_run.submission_id \
                 WHERE chain_submission.status != 'accepted' \
                    OR chain_submission.tombstoned_at_ms IS NOT NULL)",
        )
        .bind(run_id)
        .fetch_one(&self.pool)
        .await?;
        if broken_ancestor != 0 {
            return Err(DbError::NotFound);
        }
        let participant_rows = sqlx::query(
            "SELECT sp.seat, sp.participant_instance_id, sp.public_key, sp.public_disclosure FROM submission_participants sp \
             JOIN verified_runs r ON r.submission_id = sp.submission_id \
             WHERE r.id = ? ORDER BY sp.seat, sp.participant_instance_id",
        )
        .bind(run_id)
        .fetch_all(&self.pool)
        .await?;
        let mut participants = Vec::with_capacity(participant_rows.len());
        for participant in participant_rows {
            participants.push(crate::model::ParticipantClaim {
                seat: u16::try_from(participant.try_get::<i64, _>("seat")?)
                    .map_err(|_| DbError::Corrupt("participant seat out of range".to_owned()))?,
                participant_instance_id: fixed_32(participant.try_get("participant_instance_id")?)?,
                public_key: fixed_32(participant.try_get("public_key")?)?,
                public_disclosure: participant.try_get("public_disclosure")?,
            });
        }
        let owner_rows = sqlx::query(
            "SELECT genesis.host_public_key FROM verified_runs r \
             JOIN submissions s ON s.id = r.submission_id \
             JOIN used_replay_session_geneses genesis ON genesis.submission_id = s.id \
             WHERE r.campaign_chain_id = ? AND r.campaign_session_ordinal = 0 \
               AND r.scope_kind = 'campaign' AND s.status = 'accepted' \
               AND s.tombstoned_at_ms IS NULL",
        )
        .bind(
            row.try_get::<Option<String>, _>("campaign_chain_id")?
                .ok_or_else(|| DbError::Corrupt("campaign run has no chain ID".to_owned()))?,
        )
        .fetch_all(&self.pool)
        .await?;
        if owner_rows.len() != 1 {
            return Err(DbError::Corrupt(
                "campaign chain does not have exactly one live ordinal-zero owner".to_owned(),
            ));
        }
        let chain_owner_public_key = fixed_32(owner_rows[0].try_get("host_public_key")?)?;
        Ok(CampaignPredecessor {
            run_id: row.try_get("id")?,
            chain_id: row
                .try_get::<Option<String>, _>("campaign_chain_id")?
                .ok_or_else(|| DbError::Corrupt("campaign run has no chain ID".to_owned()))?,
            result_sha256: fixed_32(row.try_get("result_sha256")?)?,
            verification_request_sha256: fixed_32(row.try_get("verification_request_sha256")?)?,
            verification_result_json: row.try_get("verification_result_json")?,
            final_campaign_sha256: fixed_32(row.try_get("final_campaign_sha256")?)?,
            final_campaign_bytes: nonnegative_u64(
                row.try_get("final_campaign_bytes")?,
                "final_campaign_bytes",
            )?,
            content_manifest_id: fixed_32(row.try_get("content_manifest_id")?)?,
            campaign_content_manifest_id: fixed_32(row.try_get("campaign_content_manifest_id")?)?,
            rules_config_id: fixed_32(row.try_get("config_id")?)?,
            ruleset_id: fixed_32(row.try_get("ruleset_id")?)?,
            competition_manifest_id: row
                .try_get::<Option<Vec<u8>>, _>("competition_manifest_id")?
                .map(fixed_32)
                .transpose()?,
            campaign_session_ordinal: u32::try_from(
                row.try_get::<Option<i64>, _>("campaign_session_ordinal")?
                    .ok_or_else(|| {
                        DbError::Corrupt("campaign predecessor has no session ordinal".to_owned())
                    })?,
            )
            .map_err(|_| DbError::Corrupt("campaign session ordinal out of range".to_owned()))?,
            max_concurrent_players: u16::try_from(row.try_get::<i64, _>("max_concurrent_players")?)
                .map_err(|_| DbError::Corrupt("player count out of range".to_owned()))?,
            participant_instance_count: u16::try_from(
                row.try_get::<i64, _>("participant_instance_count")?,
            )
            .map_err(|_| DbError::Corrupt("participant instance count out of range".to_owned()))?,
            chain_owner_public_key,
            participants,
        })
    }

    pub async fn apply_username_update(
        &self,
        challenge_id: &str,
        challenge_nonce: [u8; 32],
        public_key: [u8; 32],
        username: &str,
    ) -> Result<(), DbError> {
        let now = now_epoch_ms()?;
        let mut tx = self.pool.begin().await?;
        let challenge = sqlx::query(
            "SELECT purpose, public_key, nonce, generation, expires_at_ms, consumed_at_ms \
             FROM upload_challenges WHERE id = ?",
        )
        .bind(challenge_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(DbError::InvalidChallenge)?;
        let challenge_key: Vec<u8> = challenge.try_get("public_key")?;
        let stored_nonce: Vec<u8> = challenge.try_get("nonce")?;
        let generation: i64 = challenge.try_get("generation")?;
        let expires_at: i64 = challenge.try_get("expires_at_ms")?;
        let consumed_at: Option<i64> = challenge.try_get("consumed_at_ms")?;
        if challenge.try_get::<String, _>("purpose")? != ChallengePurpose::UsernameUpdate.as_str()
            || challenge_key.as_slice() != public_key
            || stored_nonce.as_slice() != challenge_nonce
            || expires_at < now
            || consumed_at.is_some()
        {
            return Err(DbError::InvalidChallenge);
        }

        let prior = sqlx::query(
            "SELECT username, username_generation FROM identities WHERE public_key = ?",
        )
        .bind(public_key.as_slice())
        .fetch_optional(&mut *tx)
        .await?;
        if prior
            .as_ref()
            .is_some_and(|row| row.get::<i64, _>("username_generation") >= generation)
        {
            return Err(DbError::InvalidChallenge);
        }
        sqlx::query(
            "INSERT INTO identities \
             (public_key, username, username_normalized, username_generation, created_at_ms, \
              updated_at_ms) VALUES (?, ?, ?, ?, ?, ?) \
             ON CONFLICT(public_key) DO UPDATE SET \
                 username = excluded.username, \
                 username_normalized = excluded.username_normalized, \
                 username_generation = excluded.username_generation, \
                 updated_at_ms = excluded.updated_at_ms",
        )
        .bind(public_key.as_slice())
        .bind(username)
        .bind(normalized_username(username))
        .bind(generation)
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO username_history \
             (public_key, challenge_id, generation, previous_username, new_username, changed_at_ms) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(public_key.as_slice())
        .bind(challenge_id)
        .bind(generation)
        .bind(prior.as_ref().map(|row| row.get::<String, _>("username")))
        .bind(username)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        let consumed = sqlx::query(
            "UPDATE upload_challenges SET consumed_at_ms = ? \
             WHERE id = ? AND consumed_at_ms IS NULL",
        )
        .bind(now)
        .bind(challenge_id)
        .execute(&mut *tx)
        .await?;
        if consumed.rows_affected() != 1 {
            return Err(DbError::InvalidChallenge);
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn identity_exists(&self, public_key: &[u8; 32]) -> Result<bool, DbError> {
        Ok(sqlx::query_scalar::<_, i64>(
            "SELECT EXISTS(SELECT 1 FROM identities WHERE public_key = ?)",
        )
        .bind(public_key.as_slice())
        .fetch_one(&self.pool)
        .await?
            != 0)
    }

    /// Fail-closed replay check used before issuing a mission-end offer for a
    /// pre-frame-authorized session. Upload reservation repeats the same
    /// predicate transactionally, and durable insertion owns final uniqueness.
    pub async fn replay_session_genesis_used(
        &self,
        host_public_key: &[u8; 32],
        replay_session_id: &[u8; 32],
        host_nonce: &[u8; 32],
        session_genesis_sha256: &[u8; 32],
    ) -> Result<bool, DbError> {
        Ok(sqlx::query_scalar::<_, i64>(
            "SELECT EXISTS(SELECT 1 FROM used_replay_session_geneses \
             WHERE (host_public_key = ? AND replay_session_id = ? AND host_nonce = ?) \
                OR session_genesis_sha256 = ?)",
        )
        .bind(host_public_key.as_slice())
        .bind(replay_session_id.as_slice())
        .bind(host_nonce.as_slice())
        .bind(session_genesis_sha256.as_slice())
        .fetch_one(&self.pool)
        .await?
            != 0)
    }

    /// Pre-frame check for an already consumed host/session identity. The
    /// genesis digest does not exist until the authority grant is embedded and
    /// the host signs the complete genesis, so this intentionally checks only
    /// the immutable tuple already present in the preflight claim.
    pub async fn replay_session_identity_used(
        &self,
        host_public_key: &[u8; 32],
        replay_session_id: &[u8; 32],
        host_nonce: &[u8; 32],
    ) -> Result<bool, DbError> {
        Ok(sqlx::query_scalar::<_, i64>(
            "SELECT EXISTS(SELECT 1 FROM used_replay_session_geneses \
             WHERE host_public_key = ? AND replay_session_id = ? AND host_nonce = ?)",
        )
        .bind(host_public_key.as_slice())
        .bind(replay_session_id.as_slice())
        .bind(host_nonce.as_slice())
        .fetch_one(&self.pool)
        .await?
            != 0)
    }

    pub async fn public_identity(&self, public_key: &[u8; 32]) -> Result<PublicIdentity, DbError> {
        let row = sqlx::query("SELECT username FROM identities WHERE public_key = ?")
            .bind(public_key.as_slice())
            .fetch_optional(&self.pool)
            .await?
            .ok_or(DbError::NotFound)?;
        Ok(PublicIdentity {
            public_key: *public_key,
            username: row.try_get("username")?,
        })
    }

    /// Private continuation identity, callable only after the web layer has
    /// consumed an authenticated owner-status challenge.
    pub async fn owner_campaign_receipt_context(
        &self,
        run_id: &str,
    ) -> Result<Option<OwnerCampaignReceiptContext>, DbError> {
        let row = sqlx::query(
            "SELECT run.scope_kind, run.campaign_chain_id, run.campaign_content_manifest_id, \
                    run.config_id, run.ruleset_id, run.competition_manifest_id, \
                    run.max_concurrent_players, run.result_sha256, \
                    run.verification_result_json, run.verification_request_sha256, \
                    submission.verification_request_json, submission.controller_public_key, \
                    object.sha256, object.byte_length, completed.id AS full_campaign_run_id \
             FROM verified_runs run \
             JOIN submissions submission ON submission.id = run.submission_id \
             LEFT JOIN verified_run_campaign_objects link \
               ON link.run_id = run.id AND link.role = 'final' \
             LEFT JOIN campaign_objects object ON object.sha256 = link.sha256 \
             LEFT JOIN full_campaign_runs completed ON completed.chain_id = run.campaign_chain_id \
             WHERE run.id = ? AND submission.status = 'accepted'",
        )
        .bind(run_id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(DbError::NotFound)?;
        if row.try_get::<String, _>("scope_kind")? == "individual_level" {
            return Ok(None);
        }
        let final_campaign = ArtifactRefV1 {
            sha256: robin_run_protocol::Digest32::from_bytes(fixed_32(
                row.try_get::<Option<Vec<u8>>, _>("sha256")?
                    .ok_or_else(|| {
                        DbError::Corrupt("campaign run has no final object".to_owned())
                    })?,
            )?),
            byte_length: nonnegative_u64(
                row.try_get::<Option<i64>, _>("byte_length")?
                    .ok_or_else(|| {
                        DbError::Corrupt("campaign run has no final length".to_owned())
                    })?,
                "byte_length",
            )?,
            media_type: robin_run_protocol::RANKED_CAMPAIGN_MEDIA_TYPE_V1.to_owned(),
        };
        let controller_public_key = robin_run_protocol::PublicKey32::from_bytes(fixed_32(
            row.try_get("controller_public_key")?,
        )?);
        let participant_rows = sqlx::query(
            "SELECT participant.public_key FROM verified_runs run \
             JOIN submission_participants participant \
               ON participant.submission_id = run.submission_id \
             WHERE run.id = ? ORDER BY participant.public_key",
        )
        .bind(run_id)
        .fetch_all(&self.pool)
        .await?;
        let mut participant_public_keys = participant_rows
            .into_iter()
            .map(|row| {
                fixed_32(row.try_get("public_key")?)
                    .map(robin_run_protocol::PublicKey32::from_bytes)
            })
            .collect::<Result<Vec<_>, DbError>>()?;
        participant_public_keys.sort_unstable();
        participant_public_keys.dedup();
        let verification_request_json = row
            .try_get::<Option<String>, _>("verification_request_json")?
            .ok_or_else(|| DbError::Corrupt("accepted run has no request JSON".to_owned()))?;
        let verification_request: VerificationRequestV1 =
            serde_json::from_str(&verification_request_json)
                .map_err(|error| DbError::Corrupt(format!("private request JSON: {error}")))?;
        verification_request
            .validate()
            .map_err(|error| DbError::Corrupt(format!("private request: {error}")))?;
        let request_sha256 = verification_request
            .canonical_digest()
            .map_err(|error| DbError::Corrupt(format!("private request digest: {error}")))?;
        let verification_result: VerificationResultV1 = serde_json::from_str(
            row.try_get::<String, _>("verification_result_json")?
                .as_str(),
        )
        .map_err(|error| DbError::Corrupt(format!("private result JSON: {error}")))?;
        verification_result
            .validate()
            .map_err(|error| DbError::Corrupt(format!("private result: {error}")))?;
        let result_sha256 = verification_result
            .canonical_digest()
            .map_err(|error| DbError::Corrupt(format!("private result digest: {error}")))?;
        let VerificationStatusV1::Verified(verified) = &verification_result.status else {
            return Err(DbError::Corrupt(
                "accepted campaign run has a non-verified private result".to_owned(),
            ));
        };
        let mut result_participant_keys = verified
            .authenticated_participant_claims
            .iter()
            .map(|claim| claim.public_key)
            .collect::<Vec<_>>();
        result_participant_keys.sort_unstable();
        result_participant_keys.dedup();
        let signed = &verification_request.submission;
        let authoritative_controller = match &signed.submission.offer.starting_state {
            InitialStateExpectationV1::CampaignGenesis { .. } => {
                signed
                    .submission
                    .offer
                    .session_genesis
                    .claim
                    .host_public_key
            }
            InitialStateExpectationV1::CampaignContinuation { .. } => {
                signed
                    .submission
                    .campaign_continuation_authorization
                    .as_ref()
                    .ok_or_else(|| {
                        DbError::Corrupt(
                            "campaign continuation has no private controller authorization"
                                .to_owned(),
                        )
                    })?
                    .claim
                    .campaign_controller_public_key
            }
            InitialStateExpectationV1::IndividualLevel { .. } => {
                return Err(DbError::Corrupt(
                    "campaign row carries individual private request".to_owned(),
                ));
            }
        };
        if request_sha256.as_bytes()
            != &fixed_32(row.try_get::<Vec<u8>, _>("verification_request_sha256")?)?
            || result_sha256.as_bytes() != &fixed_32(row.try_get::<Vec<u8>, _>("result_sha256")?)?
            || verification_result.verification_request_sha256 != request_sha256
            || verified.final_campaign != final_campaign
            || participant_public_keys != result_participant_keys
            || controller_public_key != authoritative_controller
            || signed
                .submission
                .offer
                .session_genesis
                .claim
                .ranked_session
                .campaign_content_manifest_sha256
                != row
                    .try_get::<Option<Vec<u8>>, _>("campaign_content_manifest_id")?
                    .map(fixed_32)
                    .transpose()?
                    .map(robin_run_protocol::Digest32::from_bytes)
            || verification_result.rules_config_sha256.as_bytes()
                != &fixed_32(row.try_get::<Vec<u8>, _>("config_id")?)?
            || verification_result.ruleset_manifest_sha256.as_bytes()
                != &fixed_32(row.try_get::<Vec<u8>, _>("ruleset_id")?)?
            || participant_public_keys
                .binary_search(&controller_public_key)
                .is_err()
        {
            return Err(DbError::Corrupt(
                "campaign receipt indexes differ from retained private proof".to_owned(),
            ));
        }
        Ok(Some(OwnerCampaignReceiptContext {
            chain_id: row
                .try_get::<Option<String>, _>("campaign_chain_id")?
                .ok_or_else(|| DbError::Corrupt("campaign run has no chain ID".to_owned()))?,
            predecessor_verification_sha256: result_sha256,
            final_campaign,
            participant_public_keys,
            controller_public_key,
            campaign_content_manifest_id: fixed_32(row.try_get("campaign_content_manifest_id")?)?,
            config_id: fixed_32(row.try_get("config_id")?)?,
            ruleset_id: fixed_32(row.try_get("ruleset_id")?)?,
            competition_manifest_id: row
                .try_get::<Option<Vec<u8>>, _>("competition_manifest_id")?
                .map(fixed_32)
                .transpose()?,
            max_concurrent_players: u16::try_from(row.try_get::<i64, _>("max_concurrent_players")?)
                .map_err(|_| DbError::Corrupt("campaign player count exceeds u16".to_owned()))?,
            completed_full_campaign_run_id: row.try_get("full_campaign_run_id")?,
            verification_request,
            verification_result,
        }))
    }

    pub async fn submission_lifecycle(&self, id: &str) -> Result<SubmissionLifecycle, DbError> {
        let row = sqlx::query(
            "SELECT s.id, CASE WHEN f.submission_id IS NULL THEN s.status ELSE 'failed' END AS status, \
                    s.rejection_code, s.created_at_ms, s.updated_at_ms, \
                    r.id AS run_id \
             FROM submissions s LEFT JOIN verified_runs r ON r.submission_id = s.id \
             LEFT JOIN submission_terminal_failures f ON f.submission_id = s.id \
             WHERE s.id = ? AND s.tombstoned_at_ms IS NULL",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(DbError::NotFound)?;
        lifecycle_from_row(row)
    }

    async fn create_full_campaign_aggregate(
        &self,
        tx: &mut sqlx::Transaction<'_, Sqlite>,
        terminal_run_id: &str,
        campaign_complete_evidence_sha256: [u8; 32],
        verified_at_ms: i64,
        published_ruleset: &PublishedRulesetV1,
        campaign_content_manifest: &CampaignContentManifestV1,
    ) -> Result<String, DbError> {
        let mut reversed = Vec::new();
        let mut current = terminal_run_id.to_owned();
        for _ in 0..4_096 {
            let row = sqlx::query(
                "SELECT r.id, r.submission_id, r.mission_id, r.verification_request_sha256, r.result_sha256, \
                        r.starting_campaign_sha256, r.starting_campaign_bytes, \
                        r.final_campaign_sha256, r.final_campaign_bytes, \
                        r.starting_campaign_score, r.final_campaign_score, \
                        r.active_simulation_ticks, r.ransom_collected, r.campaign_session_kind, \
                        r.campaign_session_ordinal, r.campaign_hq_sequence, \
                        r.campaign_complete_evidence_sha256, r.verification_result_json, \
                        r.public_verification_request_sha256, r.public_verification_request_json, \
                        r.public_verification_result_sha256, r.public_verification_result_json, \
                        r.public_projection_binding_json, \
                        r.build_manifest_id, r.content_manifest_id, r.campaign_content_manifest_id, \
                        r.config_id, r.ruleset_id, r.canonical_campaign_state_json, \
                        r.max_concurrent_players, r.participant_instance_count, \
                        r.named_participant_instance_count, r.anonymous_participant_instance_count, \
                        s.predecessor_run_id, s.campaign_chain_id, s.competition_manifest_id, \
                        s.starting_state_json, s.public_metadata_json, s.envelope_json, \
                        s.replay_sha256, s.replay_bytes, \
                        s.verification_request_json \
                 FROM verified_runs r JOIN submissions s ON s.id = r.submission_id \
                 WHERE r.id = ? AND r.scope_kind = 'campaign' AND s.status = 'accepted' \
                   AND s.tombstoned_at_ms IS NULL",
            )
            .bind(&current)
            .fetch_optional(&mut **tx)
            .await?
            .ok_or_else(|| {
                DbError::ResultInvariant(
                    "campaign aggregate chain contains a missing or unpublished run".to_owned(),
                )
            })?;
            let predecessor: Option<String> = row.try_get("predecessor_run_id")?;
            reversed.push(row);
            let Some(predecessor) = predecessor else {
                break;
            };
            current = predecessor;
        }
        if reversed.is_empty()
            || reversed.len() == 4_096
                && reversed.last().is_some_and(|row| {
                    row.try_get::<Option<String>, _>("predecessor_run_id")
                        .ok()
                        .flatten()
                        .is_some()
                })
        {
            return Err(DbError::ResultInvariant(
                "campaign aggregate chain is empty or exceeds 4096 sessions".to_owned(),
            ));
        }
        reversed.reverse();

        let first = reversed.first().expect("nonempty campaign chain checked");
        let terminal = reversed.last().expect("nonempty campaign chain checked");
        let chain_id: String = terminal
            .try_get::<Option<String>, _>("campaign_chain_id")?
            .ok_or_else(|| DbError::ResultInvariant("campaign run has no chain ID".to_owned()))?;
        let campaign_content_manifest_id =
            fixed_32(terminal.try_get("campaign_content_manifest_id")?)?;
        let config_id = fixed_32(terminal.try_get("config_id")?)?;
        let ruleset_id = fixed_32(terminal.try_get("ruleset_id")?)?;
        let competition_manifest_id = terminal
            .try_get::<Option<Vec<u8>>, _>("competition_manifest_id")?
            .map(fixed_32)
            .transpose()?;
        let profile: AdmissionProfile = serde_json::from_str(
            terminal
                .try_get::<String, _>("public_metadata_json")?
                .as_str(),
        )
        .map_err(|error| DbError::Corrupt(format!("campaign admission profile: {error}")))?;
        if !profile
            .allowed_scopes
            .iter()
            .any(|scope| scope == "campaign_continuation")
            || published_ruleset
                .manifest
                .board_scopes
                .binary_search(&robin_run_protocol::RulesetBoardScopeV1::FullCampaign)
                .is_err()
        {
            return Err(DbError::ResultInvariant(
                "ruleset does not publish full-campaign boards".to_owned(),
            ));
        }
        let expected_genesis = profile.canonical_campaign_state.artifact.sha256;
        let canonical_campaign_state_json =
            serde_json::to_string(&profile.canonical_campaign_state).map_err(|error| {
                DbError::Corrupt(format!("canonical campaign-state pin JSON: {error}"))
            })?;
        let starting_state: InitialStateExpectationV1 =
            serde_json::from_str(first.try_get::<String, _>("starting_state_json")?.as_str())
                .map_err(|error| DbError::Corrupt(format!("campaign starting state: {error}")))?;
        let starting_state_requirement = starting_state.campaign_state_requirement();
        let InitialStateExpectationV1::CampaignGenesis {
            campaign_sha256, ..
        } = starting_state
        else {
            return Err(DbError::ResultInvariant(
                "full campaign does not start at canonical genesis".to_owned(),
            ));
        };
        if campaign_sha256 != expected_genesis
            || starting_state_requirement != profile.canonical_campaign_state.requirement
            || profile
                .canonical_campaign_state
                .requirement
                .rules_config_sha256
                != Digest32::from_bytes(config_id)
        {
            return Err(DbError::ResultInvariant(
                "full campaign genesis differs from its rules-config-bound operator pin".to_owned(),
            ));
        }

        let mut sessions = Vec::with_capacity(reversed.len());
        let mut session_public_proofs = Vec::with_capacity(reversed.len());
        let mut previous_final = None;
        let mut previous_score = None;
        let mut aggregate_ticks = 0_u64;
        let mut aggregate_ransom = 0_u64;
        let mut aggregate_participant_instances = 0_u32;
        let mut aggregate_named_instances = 0_u32;
        let mut aggregate_anonymous_instances = 0_u32;
        let mut aggregate_max_concurrent = 0_u16;
        let mut authenticated_participant_keys = BTreeSet::new();
        let mut public_named_keys = BTreeSet::new();
        let mut campaign_controller_public_key = None;
        for (session_index, row) in reversed.iter().enumerate() {
            let row_chain: Option<String> = row.try_get("campaign_chain_id")?;
            let row_content = fixed_32(row.try_get("content_manifest_id")?)?;
            let row_campaign_content = fixed_32(row.try_get("campaign_content_manifest_id")?)?;
            let row_config = fixed_32(row.try_get("config_id")?)?;
            let row_ruleset = fixed_32(row.try_get("ruleset_id")?)?;
            let row_competition = row
                .try_get::<Option<Vec<u8>>, _>("competition_manifest_id")?
                .map(fixed_32)
                .transpose()?;
            let row_players = u16::try_from(row.try_get::<i64, _>("max_concurrent_players")?)
                .map_err(|_| DbError::Corrupt("campaign player count exceeds u16".to_owned()))?;
            let row_instances = u16::try_from(row.try_get::<i64, _>("participant_instance_count")?)
                .map_err(|_| {
                    DbError::Corrupt("campaign participant count exceeds u16".to_owned())
                })?;
            if row_chain.as_deref() != Some(chain_id.as_str())
                || row_campaign_content != campaign_content_manifest_id
                || row_config != config_id
                || row_ruleset != ruleset_id
                || row_competition != competition_manifest_id
                || row
                    .try_get::<String, _>("canonical_campaign_state_json")?
                    .as_str()
                    != canonical_campaign_state_json.as_str()
            {
                return Err(DbError::ResultInvariant(
                    "campaign chain changes immutable board identity".to_owned(),
                ));
            }
            let stored_result: VerificationResultV1 = serde_json::from_str(
                row.try_get::<String, _>("verification_result_json")?
                    .as_str(),
            )
            .map_err(|error| DbError::Corrupt(format!("campaign verification result: {error}")))?;
            stored_result.validate().map_err(|error| {
                DbError::Corrupt(format!("campaign verification result: {error}"))
            })?;
            let stored_result_sha256 = stored_result
                .canonical_digest()
                .map_err(|error| DbError::Corrupt(format!("campaign result digest: {error}")))?;
            if stored_result_sha256.as_bytes()
                != &fixed_32(row.try_get::<Vec<u8>, _>("result_sha256")?)?
            {
                return Err(DbError::Corrupt(
                    "campaign result JSON does not match its stored digest".to_owned(),
                ));
            }
            let VerificationStatusV1::Verified(verified) = &stored_result.status else {
                return Err(DbError::Corrupt(
                    "accepted campaign row does not contain a verified result".to_owned(),
                ));
            };
            let signed: robin_run_protocol::SignedSubmissionV1 = serde_json::from_str(
                row.try_get::<String, _>("envelope_json")?.as_str(),
            )
            .map_err(|error| DbError::Corrupt(format!("campaign submission envelope: {error}")))?;
            signed.validate().map_err(|error| {
                DbError::Corrupt(format!("campaign submission envelope: {error}"))
            })?;
            if session_index == 0 {
                let offer = &signed.submission.offer;
                if offer.starting_state != starting_state {
                    return Err(DbError::Corrupt(
                        "campaign genesis offer differs from its indexed starting state".to_owned(),
                    ));
                }
                let ranked = &offer.session_genesis.claim.ranked_session;
                validate_aggregate_genesis_scope_subject(
                    ranked.content_edition,
                    &ranked.content_subject,
                    &offer.starting_state,
                )?;
            }
            let stored_request: VerificationRequestV1 = serde_json::from_str(
                row.try_get::<Option<String>, _>("verification_request_json")?
                    .ok_or_else(|| {
                        DbError::Corrupt(
                            "accepted campaign row has no recorded verification request".to_owned(),
                        )
                    })?
                    .as_str(),
            )
            .map_err(|error| DbError::Corrupt(format!("campaign verification request: {error}")))?;
            let stored_request_sha256 = stored_request
                .canonical_digest()
                .map_err(|error| DbError::Corrupt(format!("campaign request digest: {error}")))?;
            if stored_request.submission != signed
                || stored_request_sha256.as_bytes()
                    != &fixed_32(row.try_get::<Vec<u8>, _>("verification_request_sha256")?)?
            {
                return Err(DbError::Corrupt(
                    "campaign verification request does not match indexed storage".to_owned(),
                ));
            }
            stored_result
                .validate_campaign_complete_evidence(
                    &stored_request,
                    &published_ruleset.manifest,
                    Some(campaign_content_manifest),
                )
                .map_err(|error| {
                    DbError::Corrupt(format!("campaign completion evidence: {error}"))
                })?;
            let stored_public_proof = stored_public_verification_proof(row)?;
            let recomputed_public_proof =
                PublicVerificationProofV1::from_private(&stored_request, &stored_result).map_err(
                    |error| DbError::Corrupt(format!("campaign public proof projection: {error}")),
                )?;
            if stored_public_proof != recomputed_public_proof {
                return Err(DbError::Corrupt(
                    "stored campaign public proof differs from its immutable private documents"
                        .to_owned(),
                ));
            }
            session_public_proofs.push(stored_public_proof);
            let mut row_authenticated_keys = verified
                .authenticated_participant_claims
                .iter()
                .map(|claim| claim.public_key)
                .collect::<Vec<_>>();
            row_authenticated_keys.sort_unstable();
            for key in &row_authenticated_keys {
                authenticated_participant_keys.insert(*key);
            }
            for claim in &verified.authenticated_participant_claims {
                if claim.public_disclosure == ParticipantPublicDisclosureV1::NamedProfile {
                    public_named_keys.insert(claim.public_key);
                }
            }
            aggregate_participant_instances = aggregate_participant_instances
                .checked_add(u32::from(row_instances))
                .ok_or_else(|| {
                    DbError::ResultInvariant(
                        "campaign participant instance count overflow".to_owned(),
                    )
                })?;
            aggregate_named_instances = aggregate_named_instances
                .checked_add(u32::from(verified.named_participant_instance_count))
                .ok_or_else(|| {
                    DbError::ResultInvariant("campaign named participant count overflow".to_owned())
                })?;
            aggregate_anonymous_instances = aggregate_anonymous_instances
                .checked_add(u32::from(verified.anonymous_participant_instance_count))
                .ok_or_else(|| {
                    DbError::ResultInvariant(
                        "campaign anonymous participant count overflow".to_owned(),
                    )
                })?;
            aggregate_max_concurrent = aggregate_max_concurrent.max(row_players);
            let starting_campaign = ArtifactRefV1 {
                sha256: robin_run_protocol::Digest32::from_bytes(fixed_32(
                    row.try_get("starting_campaign_sha256")?,
                )?),
                byte_length: nonnegative_u64(
                    row.try_get("starting_campaign_bytes")?,
                    "starting_campaign_bytes",
                )?,
                media_type: robin_run_protocol::RANKED_CAMPAIGN_MEDIA_TYPE_V1.to_owned(),
            };
            let final_campaign = ArtifactRefV1 {
                sha256: robin_run_protocol::Digest32::from_bytes(fixed_32(
                    row.try_get("final_campaign_sha256")?,
                )?),
                byte_length: nonnegative_u64(
                    row.try_get("final_campaign_bytes")?,
                    "final_campaign_bytes",
                )?,
                media_type: robin_run_protocol::RANKED_CAMPAIGN_MEDIA_TYPE_V1.to_owned(),
            };
            let start_score = i32::try_from(row.try_get::<i64, _>("starting_campaign_score")?)
                .map_err(|_| DbError::Corrupt("campaign start score exceeds i32".to_owned()))?;
            let final_score = i32::try_from(row.try_get::<i64, _>("final_campaign_score")?)
                .map_err(|_| DbError::Corrupt("campaign final score exceeds i32".to_owned()))?;
            if previous_final
                .as_ref()
                .is_some_and(|artifact| artifact != &starting_campaign)
                || previous_score.is_some_and(|score| score != start_score)
            {
                return Err(DbError::ResultInvariant(
                    "campaign chain state or score continuity is broken".to_owned(),
                ));
            }
            previous_final = Some(final_campaign.clone());
            previous_score = Some(final_score);
            let active_ticks = nonnegative_u64(
                row.try_get("active_simulation_ticks")?,
                "active_simulation_ticks",
            )?;
            let ransom = nonnegative_u64(row.try_get("ransom_collected")?, "ransom_collected")?;
            aggregate_ticks = aggregate_ticks.checked_add(active_ticks).ok_or_else(|| {
                DbError::ResultInvariant("campaign active tick sum overflow".to_owned())
            })?;
            aggregate_ransom = aggregate_ransom.checked_add(ransom).ok_or_else(|| {
                DbError::ResultInvariant("campaign ransom sum overflow".to_owned())
            })?;
            let kind = verified.campaign_session_kind.clone().ok_or_else(|| {
                DbError::Corrupt("campaign result is missing its session kind".to_owned())
            })?;
            let ordinal = verified.campaign_session_ordinal.ok_or_else(|| {
                DbError::Corrupt("campaign result is missing its session ordinal".to_owned())
            })?;
            let content_subject = signed
                .submission
                .offer
                .session_genesis
                .claim
                .ranked_session
                .content_subject
                .clone();
            if campaign_content_manifest.content_for(&content_subject)
                != Some(robin_run_protocol::Digest32::from_bytes(row_content))
            {
                return Err(DbError::ResultInvariant(
                    "campaign session content does not resolve through the exact catalog"
                        .to_owned(),
                ));
            }
            if ordinal == 0 {
                if campaign_controller_public_key
                    .replace(
                        signed
                            .submission
                            .offer
                            .session_genesis
                            .claim
                            .host_public_key,
                    )
                    .is_some()
                {
                    return Err(DbError::ResultInvariant(
                        "campaign chain contains multiple ordinal-zero controllers".to_owned(),
                    ));
                }
            }
            let build_manifest_sha256 = stored_result.build_manifest_sha256;
            if verified.starting_campaign != starting_campaign
                || verified.final_campaign != final_campaign
                || verified.starting_campaign_score != start_score
                || verified.final_campaign_score != final_score
                || verified.active_simulation_ticks != active_ticks
                || verified.ransom_collected != ransom
                || verified.max_concurrent_players != row_players
                || verified.participant_instance_count != row_instances
                || stored_result
                    .competition_manifest_sha256
                    .map(|value| value.into_bytes())
                    != row_competition
                || signed.submission.artifacts != stored_result.artifacts
                || row.try_get::<Vec<u8>, _>("replay_sha256")?.as_slice()
                    != stored_result.artifacts.replay.artifact.sha256.as_bytes()
                || row.try_get::<i64, _>("replay_bytes")?
                    != i64::try_from(stored_result.artifacts.replay.artifact.byte_length).map_err(
                        |_| DbError::Corrupt("replay byte length exceeds i64".to_owned()),
                    )?
            {
                return Err(DbError::Corrupt(
                    "campaign typed result differs from indexed storage".to_owned(),
                ));
            }
            sessions.push(VerifiedCampaignSessionV1 {
                ordinal,
                run_id: OpaqueId::new(row.try_get::<String, _>("id")?)
                    .map_err(|error| DbError::Corrupt(error.to_string()))?,
                kind,
                content_subject,
                campaign_aggregation_consent: verified.campaign_aggregation_consent,
                replay: stored_result.artifacts.replay.clone(),
                build_manifest_sha256,
                content_manifest_sha256: stored_result.content_manifest_sha256,
                rules_config_sha256: stored_result.rules_config_sha256,
                ruleset_manifest_sha256: stored_result.ruleset_manifest_sha256,
                competition_manifest_sha256: stored_result.competition_manifest_sha256,
                verification_request_sha256: robin_run_protocol::Digest32::from_bytes(fixed_32(
                    row.try_get("verification_request_sha256")?,
                )?),
                verification_result_sha256: robin_run_protocol::Digest32::from_bytes(fixed_32(
                    row.try_get("result_sha256")?,
                )?),
                starting_campaign,
                final_campaign,
                starting_campaign_score: start_score,
                final_campaign_score: final_score,
                max_concurrent_players: row_players,
                participant_instance_count: row_instances,
                named_participant_instance_count: verified.named_participant_instance_count,
                anonymous_participant_instance_count: verified.anonymous_participant_instance_count,
                authenticated_participant_keys: row_authenticated_keys,
                active_simulation_ticks: active_ticks,
                ransom_collected: ransom,
                campaign_complete_evidence_sha256: verified
                    .campaign_complete_evidence
                    .as_ref()
                    .map(|evidence| evidence.canonical_digest())
                    .transpose()
                    .map_err(|error| {
                        DbError::Corrupt(format!("campaign completion evidence digest: {error}"))
                    })?,
            });
        }
        let authenticated_participant_keys = authenticated_participant_keys
            .into_iter()
            .collect::<Vec<_>>();
        let public_named_keys = public_named_keys.into_iter().collect::<Vec<_>>();
        let campaign_controller_public_key = campaign_controller_public_key.ok_or_else(|| {
            DbError::ResultInvariant("campaign chain has no ordinal-zero controller".to_owned())
        })?;

        let full_campaign_run_id = uuid::Uuid::now_v7().to_string();
        let request = PrivateCampaignAggregateRequestV1 {
            schema_version: robin_run_protocol::SCHEMA_VERSION_V1,
            chain_id: OpaqueId::new(chain_id.clone())
                .map_err(|error| DbError::ResultInvariant(error.to_string()))?,
            full_campaign_run_id: OpaqueId::new(full_campaign_run_id.clone())
                .map_err(|error| DbError::ResultInvariant(error.to_string()))?,
            terminal_run_id: OpaqueId::new(terminal_run_id.to_owned())
                .map_err(|error| DbError::ResultInvariant(error.to_string()))?,
            campaign_complete_evidence_sha256: robin_run_protocol::Digest32::from_bytes(
                campaign_complete_evidence_sha256,
            ),
            sessions: sessions
                .iter()
                .map(|session| PrivateCampaignAggregateSessionRequestV1 {
                    ordinal: session.ordinal,
                    run_id: session.run_id.clone(),
                    verification_request_sha256: session.verification_request_sha256,
                    verification_result_sha256: session.verification_result_sha256,
                })
                .collect(),
        };
        let aggregate_request_sha256 = request.canonical_digest()?;
        let aggregate_request_json = canonical_json_string(&request)?;
        let canonical_genesis_campaign = sessions
            .first()
            .expect("nonempty sessions")
            .starting_campaign
            .clone();
        let final_campaign = sessions
            .last()
            .expect("nonempty sessions")
            .final_campaign
            .clone();
        let starting_campaign_score = sessions
            .first()
            .expect("nonempty sessions")
            .starting_campaign_score;
        let final_campaign_score = sessions
            .last()
            .expect("nonempty sessions")
            .final_campaign_score;
        let aggregate = VerifiedCampaignAggregateV1 {
            schema_version: robin_run_protocol::SCHEMA_VERSION_V1,
            aggregate_request_sha256,
            chain_id: OpaqueId::new(chain_id.clone())
                .map_err(|error| DbError::ResultInvariant(error.to_string()))?,
            full_campaign_run_id: OpaqueId::new(full_campaign_run_id.clone())
                .map_err(|error| DbError::ResultInvariant(error.to_string()))?,
            campaign_complete_terminal_run_id: OpaqueId::new(terminal_run_id.to_owned())
                .map_err(|error| DbError::ResultInvariant(error.to_string()))?,
            campaign_complete_evidence_sha256: robin_run_protocol::Digest32::from_bytes(
                campaign_complete_evidence_sha256,
            ),
            sessions,
            max_concurrent_players: aggregate_max_concurrent,
            participant_instance_count: aggregate_participant_instances,
            named_participant_instance_count: aggregate_named_instances,
            anonymous_participant_instance_count: aggregate_anonymous_instances,
            authenticated_participant_keys,
            campaign_controller_public_key,
            campaign_content_manifest_sha256: robin_run_protocol::Digest32::from_bytes(
                campaign_content_manifest_id,
            ),
            rules_config_sha256: robin_run_protocol::Digest32::from_bytes(config_id),
            ruleset_manifest_sha256: robin_run_protocol::Digest32::from_bytes(ruleset_id),
            competition_manifest_sha256: competition_manifest_id
                .map(robin_run_protocol::Digest32::from_bytes),
            canonical_genesis_campaign: canonical_genesis_campaign.clone(),
            final_campaign: final_campaign.clone(),
            starting_campaign_score,
            final_campaign_score,
            active_simulation_ticks: aggregate_ticks,
            ransom_collected: aggregate_ransom,
        };
        aggregate
            .validate_against_ruleset(published_ruleset, campaign_content_manifest)
            .map_err(|error| DbError::ResultInvariant(error.to_string()))?;
        let aggregate_sha256 = aggregate
            .canonical_digest()
            .map_err(|error| DbError::ResultInvariant(error.to_string()))?;
        let aggregate_json = canonical_json_string(&aggregate)?;
        let public_aggregate_proof =
            PublicCampaignAggregateProofV1::from_private(&aggregate, &session_public_proofs)
                .map_err(|error| DbError::ResultInvariant(error.to_string()))?;
        let public_aggregate_request_sha256 = public_aggregate_proof
            .public_request
            .canonical_digest()
            .map_err(|error| DbError::ResultInvariant(error.to_string()))?;
        if public_aggregate_request_sha256 != public_aggregate_proof.public_request_sha256 {
            return Err(DbError::ResultInvariant(
                "public aggregate request projection has an inconsistent digest".to_owned(),
            ));
        }
        let public_aggregate_result_sha256 = public_aggregate_proof
            .canonical_digest()
            .map_err(|error| DbError::ResultInvariant(error.to_string()))?;
        let public_aggregate_request_json =
            canonical_json_string(&public_aggregate_proof.public_request)?;
        let public_aggregate_result_json = canonical_json_string(&public_aggregate_proof)?;
        let public_projection_binding_json = projection_binding_json(
            aggregate_request_sha256,
            aggregate_sha256,
            public_aggregate_request_sha256,
            public_aggregate_result_sha256,
        )?;
        let accepted_sequence: i64 = sqlx::query_scalar(
            "INSERT INTO acceptance_sequences (created_at_ms) VALUES (?) RETURNING sequence",
        )
        .bind(verified_at_ms)
        .fetch_one(&mut **tx)
        .await?;
        sqlx::query(
            "INSERT INTO full_campaign_runs (id, chain_id, terminal_run_id, \
                aggregate_request_sha256, aggregate_sha256, aggregate_json, \
                aggregate_request_json, public_aggregate_request_sha256, \
                public_aggregate_request_json, public_aggregate_result_sha256, \
                public_aggregate_result_json, public_projection_binding_json, \
                campaign_complete_evidence_sha256, campaign_content_manifest_id, \
                config_id, ruleset_id, \
                canonical_campaign_state_json, \
                competition_manifest_id, starting_campaign_sha256, starting_campaign_bytes, \
                final_campaign_sha256, final_campaign_bytes, \
                starting_campaign_score, final_campaign_score, active_simulation_ticks, \
                ransom_collected, max_concurrent_players, participant_instance_count, \
                named_participant_instance_count, anonymous_participant_instance_count, \
                accepted_sequence, verified_at_ms) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, \
                     ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&full_campaign_run_id)
        .bind(&chain_id)
        .bind(terminal_run_id)
        .bind(aggregate_request_sha256.as_bytes().as_slice())
        .bind(aggregate_sha256.as_bytes().as_slice())
        .bind(&aggregate_json)
        .bind(&aggregate_request_json)
        .bind(public_aggregate_request_sha256.as_bytes().as_slice())
        .bind(&public_aggregate_request_json)
        .bind(public_aggregate_result_sha256.as_bytes().as_slice())
        .bind(&public_aggregate_result_json)
        .bind(&public_projection_binding_json)
        .bind(campaign_complete_evidence_sha256.as_slice())
        .bind(campaign_content_manifest_id.as_slice())
        .bind(config_id.as_slice())
        .bind(ruleset_id.as_slice())
        .bind(&canonical_campaign_state_json)
        .bind(
            competition_manifest_id
                .as_ref()
                .map(|digest| digest.as_slice()),
        )
        .bind(canonical_genesis_campaign.sha256.as_bytes().as_slice())
        .bind(
            i64::try_from(canonical_genesis_campaign.byte_length).map_err(|_| {
                DbError::ResultInvariant("campaign length exceeds SQLite INTEGER".to_owned())
            })?,
        )
        .bind(final_campaign.sha256.as_bytes().as_slice())
        .bind(i64::try_from(final_campaign.byte_length).map_err(|_| {
            DbError::ResultInvariant("campaign length exceeds SQLite INTEGER".to_owned())
        })?)
        .bind(i64::from(starting_campaign_score))
        .bind(i64::from(final_campaign_score))
        .bind(i64::try_from(aggregate_ticks).map_err(|_| {
            DbError::ResultInvariant("campaign tick sum exceeds SQLite INTEGER".to_owned())
        })?)
        .bind(i64::try_from(aggregate_ransom).map_err(|_| {
            DbError::ResultInvariant("campaign ransom sum exceeds SQLite INTEGER".to_owned())
        })?)
        .bind(i64::from(aggregate_max_concurrent))
        .bind(i64::from(aggregate_participant_instances))
        .bind(i64::from(aggregate_named_instances))
        .bind(i64::from(aggregate_anonymous_instances))
        .bind(accepted_sequence)
        .bind(verified_at_ms)
        .execute(&mut **tx)
        .await?;
        for (ordinal, session) in aggregate.sessions.iter().enumerate() {
            sqlx::query(
                "INSERT INTO full_campaign_sessions (full_campaign_run_id, ordinal, run_id) \
                 VALUES (?, ?, ?)",
            )
            .bind(&full_campaign_run_id)
            .bind(i64::try_from(ordinal).map_err(|_| {
                DbError::ResultInvariant("campaign session ordinal exceeds i64".to_owned())
            })?)
            .bind(session.run_id.as_str())
            .execute(&mut **tx)
            .await?;
        }
        for public_key in &public_named_keys {
            sqlx::query(
                "INSERT INTO full_campaign_participants \
                 (full_campaign_run_id, public_key) VALUES (?, ?)",
            )
            .bind(&full_campaign_run_id)
            .bind(public_key.as_bytes().as_slice())
            .execute(&mut **tx)
            .await?;
        }
        let aggregate_score = i64::from(final_campaign_score) - i64::from(starting_campaign_score);
        for (metric, value) in [
            ("original_score", aggregate_score),
            (
                "fastest_success",
                i64::try_from(aggregate_ticks).map_err(|_| {
                    DbError::ResultInvariant("campaign tick sum exceeds SQLite INTEGER".to_owned())
                })?,
            ),
        ] {
            sqlx::query(
                "INSERT INTO full_campaign_metrics (full_campaign_run_id, metric, value) \
                 VALUES (?, ?, ?)",
            )
            .bind(&full_campaign_run_id)
            .bind(metric)
            .bind(value)
            .execute(&mut **tx)
            .await?;
        }
        Ok(full_campaign_run_id)
    }

    /// Wait for SQLx return/rollback before releasing an externally held fence.
    /// This does not acquire or release that fence on the caller's behalf.
    pub async fn wait_for_idle(&self) -> anyhow::Result<()> {
        wait_for_pool_idle(&self.pool).await
    }

    /// Close under an already-held administrative fence. The caller must keep
    /// its guard alive through completion; ordinary shutdown uses close_fenced.
    pub async fn close_pool_under_fence(&self) {
        self.pool.close().await;
    }

    /// Escape hatch for corruption/concurrency fixtures, including binary tests
    /// linked against the non-test library. Production code must use the narrow
    /// database operations, wait_for_idle, and fenced shutdown instead.
    #[doc(hidden)]
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }
}

async fn recover_upload_reservations_in(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    now: i64,
) -> Result<u64, DbError> {
    let abandoned = sqlx::query(
        "UPDATE submission_upload_reservations \
         SET state = 'abandoned', lease_token = NULL, lease_expires_at_ms = NULL, \
             abandoned_at_ms = ?, updated_at_ms = ? \
         WHERE state = 'reserved' AND lease_expires_at_ms < ?",
    )
    .bind(now)
    .bind(now)
    .bind(now)
    .execute(&mut **tx)
    .await?
    .rows_affected();
    let expired = sqlx::query(
        "DELETE FROM submission_upload_reservations \
         WHERE state != 'committed' AND reservation_expires_at_ms < ?",
    )
    .bind(now)
    .execute(&mut **tx)
    .await?
    .rows_affected();
    // A reserved submission challenge is consumed before streaming. Once its
    // bounded retry reservation expires, remove that otherwise-unreferenced
    // challenge too so abandoned attackers cannot grow durable state.
    sqlx::query(
        "DELETE FROM upload_challenges \
         WHERE purpose = 'submission' AND consumed_at_ms IS NOT NULL \
           AND NOT EXISTS (SELECT 1 FROM submission_upload_reservations r \
                           WHERE r.upload_challenge_id = upload_challenges.id) \
           AND NOT EXISTS (SELECT 1 FROM submissions s \
                           WHERE s.upload_challenge_id = upload_challenges.id)",
    )
    .execute(&mut **tx)
    .await?;
    abandoned
        .checked_add(expired)
        .ok_or_else(|| DbError::Corrupt("upload recovery count overflow".to_owned()))
}

async fn lifecycle_by_submission_id(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    submission_id: &str,
) -> Result<Option<SubmissionLifecycle>, DbError> {
    let row = sqlx::query(
        "SELECT s.id, CASE WHEN f.submission_id IS NULL THEN s.status ELSE 'failed' END AS status, \
                s.rejection_code, s.created_at_ms, s.updated_at_ms, r.id AS run_id \
         FROM submissions s LEFT JOIN verified_runs r ON r.submission_id = s.id \
         LEFT JOIN submission_terminal_failures f ON f.submission_id = s.id \
         WHERE s.id = ? AND s.tombstoned_at_ms IS NULL",
    )
    .bind(submission_id)
    .fetch_optional(&mut **tx)
    .await?;
    row.map(lifecycle_from_row).transpose()
}

async fn verify_pinned_database_leaf(
    parent: &Arc<cap_std::fs::Dir>,
    leaf: &str,
    pinned: &Arc<std::fs::File>,
) -> Result<(), DbError> {
    let opened_parent = Arc::clone(parent);
    let opened_leaf = leaf.to_owned();
    let current = tokio::task::spawn_blocking(move || {
        crate::secure_fs::open_regular_file(&opened_parent, std::path::Path::new(&opened_leaf))
    })
    .await
    .map_err(|error| sqlx::Error::Io(std::io::Error::other(error)))?
    .map_err(sqlx::Error::Io)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        let expected = pinned.metadata().map_err(sqlx::Error::Io)?;
        let actual = current.metadata().map_err(sqlx::Error::Io)?;
        if (actual.dev(), actual.ino()) != (expected.dev(), expected.ino()) {
            return Err(DbError::Corrupt(
                "database leaf changed while the connection pool was opening".to_owned(),
            ));
        }
    }
    Ok(())
}

/// Install the one canonical production schema. This service has no deployed
/// pre-canonical database format to upgrade or reinterpret.
async fn run_migrations(pool: &SqlitePool) -> Result<(), DbError> {
    let mut connection = pool.acquire().await?;
    MIGRATOR.run_direct(None, &mut *connection, false).await?;
    drop(connection);
    ensure_schema_current(pool).await
}

async fn ensure_schema_current(pool: &SqlitePool) -> Result<(), DbError> {
    let rows =
        sqlx::query("SELECT version, success, checksum FROM _sqlx_migrations ORDER BY version")
            .fetch_all(pool)
            .await
            .map_err(|error| match error {
                sqlx::Error::Database(database) if database.message().contains("no such table") => {
                    sqlx::Error::Protocol(
                        "database is not migrated; run `robin-highscores-admin migrate`".to_owned(),
                    )
                }
                other => other,
            })?;
    let current = rows.last().map(|row| row.get::<i64, _>("version"));
    let checksums_match = rows
        .iter()
        .zip(MIGRATOR.migrations.iter())
        .all(|(row, migration)| {
            row.get::<i64, _>("version") == migration.version
                && row.get::<Vec<u8>, _>("checksum").as_slice() == migration.checksum.as_ref()
        });
    if rows.len() != usize::try_from(CURRENT_SCHEMA_VERSION).expect("small schema version")
        || current != Some(CURRENT_SCHEMA_VERSION)
        || rows.iter().any(|row| !row.get::<bool, _>("success"))
        || !checksums_match
    {
        return Err(DbError::Corrupt(format!(
            "database schema is not current (expected version {CURRENT_SCHEMA_VERSION}); run the explicit migration command"
        )));
    }
    Ok(())
}

async fn set_private_permissions(path: &std::path::Path, directory: bool) -> Result<(), DbError> {
    #[cfg(target_os = "linux")]
    {
        use rustix::fs::{Mode, OFlags, ResolveFlags, openat2};
        use std::os::unix::fs::PermissionsExt as _;
        let mut flags = OFlags::RDONLY | OFlags::CLOEXEC;
        if directory {
            flags |= OFlags::DIRECTORY;
        }
        let fd = openat2(
            rustix::fs::CWD,
            path,
            flags,
            Mode::empty(),
            ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
        )
        .map_err(std::io::Error::from)
        .map_err(|error| DbError::Sql(sqlx::Error::Io(error)))?;
        let file = std::fs::File::from(fd);
        let metadata = file
            .metadata()
            .map_err(|error| DbError::Sql(sqlx::Error::Io(error)))?;
        if (directory && !metadata.is_dir()) || (!directory && !metadata.is_file()) {
            return Err(DbError::Corrupt(format!(
                "database path has the wrong file type: {}",
                path.display()
            )));
        }
        let mode = if directory {
            crate::secure_fs::SHARED_PRIVATE_DIRECTORY_MODE
        } else {
            crate::secure_fs::SHARED_MUTABLE_FILE_MODE
        };
        if metadata.permissions().mode() & 0o7777 != mode {
            file.set_permissions(std::fs::Permissions::from_mode(mode))
                .map_err(|error| DbError::Sql(sqlx::Error::Io(error)))?;
        }
    }
    #[cfg(all(unix, not(target_os = "linux")))]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = if directory {
            crate::secure_fs::SHARED_PRIVATE_DIRECTORY_MODE
        } else {
            crate::secure_fs::SHARED_MUTABLE_FILE_MODE
        };
        if tokio::fs::metadata(path)
            .await
            .map_err(|error| DbError::Sql(sqlx::Error::Io(error)))?
            .permissions()
            .mode()
            & 0o7777
            != mode
        {
            tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
                .await
                .map_err(|error| DbError::Sql(sqlx::Error::Io(error)))?;
        }
    }
    Ok(())
}

async fn ensure_worker_lease(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    submission_id: &str,
    worker_id: &str,
    now: i64,
) -> Result<(), DbError> {
    let valid: i64 = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM submissions \
         WHERE id = ? AND tombstoned_at_ms IS NULL AND status = 'verifying' \
             AND lease_owner = ? AND lease_expires_at_ms >= ?)",
    )
    .bind(submission_id)
    .bind(worker_id)
    .bind(now)
    .fetch_one(&mut **tx)
    .await?;
    if valid == 0 {
        return Err(DbError::LeaseLost);
    }
    Ok(())
}

async fn insert_worker_event(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    submission_id: &str,
    kind: &str,
    worker_id: &str,
    detail: &str,
    now: i64,
) -> Result<(), DbError> {
    sqlx::query(
        "INSERT INTO worker_events \
         (submission_id, kind, worker_id, detail, created_at_ms) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(submission_id)
    .bind(kind)
    .bind(worker_id)
    .bind(detail)
    .bind(now)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn ensure_live_campaign_object(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    artifact: &ArtifactRefV1,
) -> Result<(), DbError> {
    let row = sqlx::query("SELECT byte_length, purge_state FROM campaign_objects WHERE sha256 = ?")
        .bind(artifact.sha256.as_bytes().as_slice())
        .fetch_optional(&mut **tx)
        .await?
        .ok_or_else(|| {
            DbError::ResultInvariant(
                "verified campaign artifact was not registered before acceptance".to_owned(),
            )
        })?;
    if row.try_get::<String, _>("purge_state")? != "live"
        || nonnegative_u64(row.try_get("byte_length")?, "campaign byte_length")?
            != artifact.byte_length
    {
        return Err(DbError::ResultInvariant(
            "verified campaign artifact is not live with its exact typed length".to_owned(),
        ));
    }
    Ok(())
}

fn lifecycle_from_row(row: sqlx::sqlite::SqliteRow) -> Result<SubmissionLifecycle, DbError> {
    Ok(SubmissionLifecycle {
        id: row.try_get("id")?,
        state: crate::model::SubmissionState::from_columns(
            row.try_get::<String, _>("status")?.as_str(),
            row.try_get("run_id")?,
            row.try_get("rejection_code")?,
        )
        .map_err(DbError::Corrupt)?,
        created_at_ms: nonnegative_u64(row.try_get("created_at_ms")?, "created_at_ms")?,
        updated_at_ms: nonnegative_u64(row.try_get("updated_at_ms")?, "updated_at_ms")?,
    })
}

fn fixed_32(bytes: Vec<u8>) -> Result<[u8; 32], DbError> {
    bytes
        .try_into()
        .map_err(|_| DbError::Corrupt("expected a 32-byte digest".to_owned()))
}

fn active_ruleset_predicate(column: &str, active_ruleset_ids: &[[u8; 32]]) -> String {
    if active_ruleset_ids.is_empty() {
        return "0".to_owned();
    }
    let digests = active_ruleset_ids
        .iter()
        .map(|digest| format!("X'{}'", hex::encode(digest)))
        .collect::<Vec<_>>()
        .join(",");
    format!("{column} IN ({digests})")
}

fn nonnegative_u64(value: i64, field: &str) -> Result<u64, DbError> {
    u64::try_from(value).map_err(|_| DbError::Corrupt(format!("{field} is negative")))
}

fn optional_u32(row: &sqlx::sqlite::SqliteRow, field: &str) -> Result<Option<u32>, DbError> {
    row.try_get::<Option<i64>, _>(field)?
        .map(|value| {
            u32::try_from(value)
                .map_err(|_| DbError::Corrupt(format!("{field} is outside the u32 range")))
        })
        .transpose()
}

fn compare_blob(
    row: &sqlx::sqlite::SqliteRow,
    field: &str,
    result: &[u8; 32],
) -> Result<(), DbError> {
    if row.try_get::<Vec<u8>, _>(field)?.as_slice() != result {
        return Err(DbError::ResultInvariant(format!(
            "verifier-derived {field} differs from the signed offer"
        )));
    }
    Ok(())
}

fn compare_value(row: &sqlx::sqlite::SqliteRow, field: &str, result: &str) -> Result<(), DbError> {
    if row.try_get::<String, _>(field)? != result {
        return Err(DbError::ResultInvariant(format!(
            "verifier-derived {field} differs from the signed offer"
        )));
    }
    Ok(())
}

fn validate_ranked_policy(
    result: &VerificationResultV1,
    signed: &robin_run_protocol::SignedSubmissionV1,
    verifier_executable_sha256: [u8; 32],
    build_digest: Digest32,
    build: &BuildManifestV1,
    content: &ContentManifestV1,
    campaign_content: Option<&CampaignContentManifestV1>,
    published: &PublishedRulesetV1,
    competition: Option<&CompetitionManifestV1>,
) -> Result<(), DbError> {
    build
        .validate()
        .map_err(|error| DbError::ResultInvariant(error.to_string()))?;
    published
        .validate()
        .map_err(|error| DbError::ResultInvariant(error.to_string()))?;
    content
        .validate()
        .map_err(|error| DbError::ResultInvariant(error.to_string()))?;
    if !matches!(
        published.operational_status,
        RulesetOperationalStatusV1::Active
    ) {
        return Err(DbError::ResultInvariant(
            "quarantined rulesets cannot publish ranked results".to_owned(),
        ));
    }
    let offer = &signed.submission.offer;
    let ranked = &offer.session_genesis.claim.ranked_session;
    ranked
        .validate_content_manifest(content)
        .map_err(|error| DbError::ResultInvariant(error.to_string()))?;
    let expected_campaign_content = match offer.starting_state.scope_kind() {
        RunScopeKindV1::IndividualLevel => None,
        RunScopeKindV1::Campaign => Some(campaign_content.ok_or_else(|| {
            DbError::ResultInvariant(
                "campaign run has no exact campaign content catalog".to_owned(),
            )
        })?),
    };
    if ranked.campaign_content_manifest_sha256
        != expected_campaign_content
            .map(|manifest| manifest.canonical_digest())
            .transpose()
            .map_err(|error| DbError::ResultInvariant(error.to_string()))?
        || expected_campaign_content.is_some_and(|catalog| {
            catalog.edition != content.edition
                || catalog.content_for(&content.subject) != Some(result.content_manifest_sha256)
        })
    {
        return Err(DbError::ResultInvariant(
            "ranked session content does not resolve through its exact official catalog".to_owned(),
        ));
    }
    let manifest = &published.manifest;
    let scope = match offer.starting_state.scope_kind() {
        RunScopeKindV1::IndividualLevel => RulesetBoardScopeV1::IndividualLevel,
        RunScopeKindV1::Campaign => RulesetBoardScopeV1::CampaignMission,
    };
    let participant_policy = &manifest.participant_eligibility;
    let verified = match &result.status {
        VerificationStatusV1::Verified(verified) => verified,
        _ => {
            return Err(DbError::ResultInvariant(
                "ranked policy received a non-verified result".to_owned(),
            ));
        }
    };
    manifest
        .validate_authoritative_achievements(&verified.achievements)
        .map_err(|error| DbError::ResultInvariant(error.to_string()))?;
    let campaign_controller = match &offer.starting_state {
        InitialStateExpectationV1::IndividualLevel { .. } => None,
        InitialStateExpectationV1::CampaignGenesis { .. } => {
            Some(offer.session_genesis.claim.host_public_key)
        }
        InitialStateExpectationV1::CampaignContinuation { .. } => Some(
            signed
                .submission
                .campaign_continuation_authorization
                .as_ref()
                .ok_or_else(|| {
                    DbError::ResultInvariant(
                        "campaign continuation has no controller authorization".to_owned(),
                    )
                })?
                .claim
                .campaign_controller_public_key,
        ),
    };
    if campaign_controller.is_some_and(|controller| {
        !offer
            .participant_claims
            .iter()
            .any(|claim| claim.public_key == controller)
    }) {
        return Err(DbError::ResultInvariant(
            "campaign controller is not an authenticated participant".to_owned(),
        ));
    }
    let participant_mode_allowed = if verified.max_concurrent_players == 1 {
        participant_policy.allow_single_player
    } else {
        participant_policy.allow_multiplayer
    };
    validate_ranked_score(
        verified.starting_campaign_score,
        verified.final_campaign_score,
        verified.original_score_delta,
    )?;
    if build_digest != offer.build_manifest_sha256
        || result.build_manifest_sha256 != build_digest
        || build.verifier.sha256.as_bytes() != &verifier_executable_sha256
        || published.ruleset_manifest_sha256 != offer.ruleset_manifest_sha256
        || result.ruleset_manifest_sha256 != published.ruleset_manifest_sha256
        || manifest.rules_config_sha256 != offer.rules_config_sha256
        || result.rules_config_sha256 != manifest.rules_config_sha256
        || manifest.canonical_campaign_state != offer.starting_state.campaign_state_requirement()
        || manifest.canonical_campaign_state.edition != content.edition
        || manifest
            .allowed_content_manifest_sha256
            .binary_search(&offer.content_manifest_sha256)
            .is_err()
        || result.content_manifest_sha256 != offer.content_manifest_sha256
        || manifest
            .allowed_build_manifest_sha256
            .binary_search(&offer.build_manifest_sha256)
            .is_err()
        || manifest.board_scopes.binary_search(&scope).is_err()
        || signed
            .submission
            .requested_metrics
            .iter()
            .any(|metric| manifest.metrics.binary_search(metric).is_err())
        || signed.submission.artifacts.replay.replay_schema_version
            != robin_run_protocol::CURRENT_RANKED_REPLAY_SCHEMA_VERSION_V1
        || manifest
            .replay_schema_versions
            .binary_search(&signed.submission.artifacts.replay.replay_schema_version)
            .is_err()
        || manifest
            .network_protocol_versions
            .binary_search(&build.network_protocol_version)
            .is_err()
        || !participant_mode_allowed
        || verified.max_concurrent_players < participant_policy.minimum_max_concurrent_players
        || verified.max_concurrent_players > participant_policy.maximum_max_concurrent_players
        || verified.participant_instance_count > participant_policy.maximum_participant_instances
        || (participant_policy.anonymous_policy == AnonymousParticipantPolicyV1::Forbidden
            && verified.anonymous_participant_instance_count != 0)
    {
        return Err(DbError::ResultInvariant(
            "verified result does not satisfy its immutable ranked ruleset tuple".to_owned(),
        ));
    }
    match (competition, result.competition_manifest_sha256) {
        (None, None) => {}
        (Some(competition), Some(digest)) => {
            competition
                .validate_submission_offer(offer)
                .map_err(|error| DbError::ResultInvariant(error.to_string()))?;
            let grant = offer
                .session_genesis
                .claim
                .competition_run_grant
                .as_ref()
                .ok_or_else(|| {
                    DbError::ResultInvariant("competition run has no pre-run grant".to_owned())
                })?;
            competition
                .validate_run_grant(grant)
                .map_err(|error| DbError::ResultInvariant(error.to_string()))?;
            verify_signature(
                grant.claim.grant_authority_public_key.as_bytes(),
                grant.authority_signature.as_bytes(),
                &grant
                    .signing_bytes()
                    .map_err(|error| DbError::ResultInvariant(error.to_string()))?,
            )
            .map_err(|_| {
                DbError::ResultInvariant(
                    "competition run grant signature authentication failed".to_owned(),
                )
            })?;
            let actual = competition
                .canonical_digest()
                .map_err(|error| DbError::ResultInvariant(error.to_string()))?;
            if actual != digest {
                return Err(DbError::ResultInvariant(
                    "competition manifest digest changed across verification".to_owned(),
                ));
            }
        }
        _ => {
            return Err(DbError::ResultInvariant(
                "competition manifest presence changed across verification".to_owned(),
            ));
        }
    }
    Ok(())
}

/// Mission replays retain the original game's wrapping `u32` subtotal, but a
/// ranked campaign score is never allowed to wrap its signed `i32` owner. The
/// verifier result already proves the bit-preserving wrapping relation; this
/// additional policy check rejects the wraparound cases instead of ranking a
/// small wrapped subtotal as a legitimate campaign delta.
fn validate_ranked_score(
    starting_campaign_score: i32,
    final_campaign_score: i32,
    original_score_delta: i64,
) -> Result<(), DbError> {
    let checked_delta = i64::from(final_campaign_score) - i64::from(starting_campaign_score);
    if checked_delta < 0 || checked_delta != original_score_delta {
        return Err(DbError::ResultInvariant(
            "ranked campaign score overflowed, decreased, or differs from the mission subtotal"
                .to_owned(),
        ));
    }
    Ok(())
}

const fn achievement_evaluation_name(value: VerifiedAchievementEvaluationV1) -> &'static str {
    match value {
        VerifiedAchievementEvaluationV1::Unverifiable => "unverifiable",
        VerifiedAchievementEvaluationV1::NotEarned => "not_earned",
        VerifiedAchievementEvaluationV1::Earned => "earned",
    }
}

#[cfg(test)]
fn achievement_evaluation(value: &str) -> Result<VerifiedAchievementEvaluationV1, DbError> {
    match value {
        "unverifiable" => Ok(VerifiedAchievementEvaluationV1::Unverifiable),
        "not_earned" => Ok(VerifiedAchievementEvaluationV1::NotEarned),
        "earned" => Ok(VerifiedAchievementEvaluationV1::Earned),
        other => Err(DbError::Corrupt(format!(
            "unknown stored achievement evaluation `{other}`"
        ))),
    }
}

fn is_public_rejection_code(value: &str) -> bool {
    value
        .parse::<robin_run_protocol::VerificationRejectionCodeV1>()
        .is_ok()
}

mod hex_array {
    use serde::Serializer;

    pub fn serialize<S>(value: &[u8; 32], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&hex::encode(value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use robin_run_protocol::{
        ChallengeNonce32, CompetitionRunGrantClaimV1, CompetitionRunGrantRequestClaimV1,
        PublicKey32, RankedSessionConfigV1, ResourceLocaleRootV1, SCHEMA_VERSION_V1, Signature64,
        SignatureAlgorithmV1, SimulationSeed64, SpeechTimingAuthorityV1, VerifiedAchievementV1,
    };
    use sqlx::Connection as _;

    async fn test_database() -> (tempfile::TempDir, Database) {
        let directory = tempfile::tempdir().unwrap();
        let mut config = ServerConfig::default();
        config.database_path = directory.path().join("highscores.sqlite3");
        let database = Database::migrate(&config).await.unwrap();
        (directory, database)
    }

    #[test]
    fn aggregate_recheck_rejects_h12_and_headquarters_as_full_campaign_genesis() {
        let genesis = InitialStateExpectationV1::CampaignGenesis {
            template_id: OpaqueId::new("full-campaign-genesis").unwrap(),
            campaign_state_requirement: robin_run_protocol::CanonicalCampaignStateRequirementV1 {
                edition: OfficialContentEditionV1::Full,
                kind: robin_run_protocol::CanonicalCampaignStateKindV1::FullCampaignGenesis,
                rules_config_sha256: Digest32::from_bytes([6; 32]),
            },
            campaign_sha256: Digest32::from_bytes([8; 32]),
            starting_campaign_byte_length: 321,
        };
        assert!(
            validate_aggregate_genesis_scope_subject(
                OfficialContentEditionV1::Full,
                &OfficialContentSubjectV1::FieldMission {
                    mission_id: robin_run_protocol::OFFICIAL_FULL_CAMPAIGN_GENESIS_MISSION_ID_V1
                        .to_owned(),
                },
                &genesis,
            )
            .is_ok()
        );
        for subject in [
            OfficialContentSubjectV1::FieldMission {
                mission_id: "H12_Not_MP".to_owned(),
            },
            OfficialContentSubjectV1::Headquarters {
                mission_id: robin_run_protocol::OFFICIAL_FULL_HEADQUARTERS_MISSION_ID_V1.to_owned(),
            },
        ] {
            assert!(matches!(
                validate_aggregate_genesis_scope_subject(
                    OfficialContentEditionV1::Full,
                    &subject,
                    &genesis,
                ),
                Err(DbError::ResultInvariant(_))
            ));
        }
    }

    fn grant_request(host: [u8; 32], sequence: u8) -> CompetitionRunGrantRequestV1 {
        CompetitionRunGrantRequestV1 {
            claim: CompetitionRunGrantRequestClaimV1 {
                schema_version: SCHEMA_VERSION_V1,
                request_nonce: ChallengeNonce32::from_bytes([sequence; 32]),
                host_public_key: PublicKey32::from_bytes(host),
                replay_session_id: Digest32::from_bytes([sequence.wrapping_add(1); 32]),
                host_participant_instance_id: Digest32::from_bytes([sequence.wrapping_add(2); 32]),
                host_nonce: ChallengeNonce32::from_bytes([sequence.wrapping_add(3); 32]),
                ranked_session: RankedSessionConfigV1 {
                    schema_version: SCHEMA_VERSION_V1,
                    mission_id: "Dem_Lei_MP".into(),
                    content_edition: OfficialContentEditionV1::Demo,
                    content_subject: OfficialContentSubjectV1::FieldMission {
                        mission_id: "Dem_Lei_MP".into(),
                    },
                    simulation_seed: SimulationSeed64::new(42),
                    starting_campaign_sha256: Digest32::from_bytes([4; 32]),
                    starting_campaign_byte_length: 123,
                    prepared_inputs_projection_sha256: Digest32::from_bytes([5; 32]),
                    prepared_mission_inputs_seal_sha256: Digest32::from_bytes([6; 32]),
                    build_manifest_sha256: Digest32::from_bytes([7; 32]),
                    content_manifest_sha256: Digest32::from_bytes([8; 32]),
                    campaign_content_manifest_sha256: None,
                    rules_config_sha256: Digest32::from_bytes([9; 32]),
                    ruleset_manifest_sha256: Digest32::from_bytes([10; 32]),
                    competition_manifest_sha256: Some(Digest32::from_bytes([11; 32])),
                    spellforge_content_sha256: None,
                    resource_locale_root: ResourceLocaleRootV1::new("1033").unwrap(),
                    speech_timing: SpeechTimingAuthorityV1::BaseInstallation,
                },
            },
            algorithm: SignatureAlgorithmV1::Ed25519,
            host_signature: Signature64::from_bytes([12; 64]),
        }
    }

    fn build_test_grant(
        request: &CompetitionRunGrantRequestV1,
        issued: &IssuedCompetitionRunGrant,
    ) -> Result<CompetitionRunGrantV1, DbError> {
        Ok(CompetitionRunGrantV1 {
            claim: CompetitionRunGrantClaimV1 {
                schema_version: SCHEMA_VERSION_V1,
                grant_id: OpaqueId::new(issued.id.clone()).unwrap(),
                grant_nonce: ChallengeNonce32::from_bytes(issued.nonce),
                grant_authority_public_key: PublicKey32::from_bytes([13; 32]),
                host_public_key: request.claim.host_public_key,
                competition_manifest_sha256: request
                    .claim
                    .ranked_session
                    .competition_manifest_sha256
                    .unwrap(),
                ranked_session_sha256: request.claim.ranked_session.canonical_digest().unwrap(),
                grant_request_sha256: request.canonical_digest().unwrap(),
                replay_session_id: request.claim.replay_session_id,
                host_participant_instance_id: request.claim.host_participant_instance_id,
                host_nonce: request.claim.host_nonce,
                admitted_at_unix_ms: issued.admitted_at_ms,
                expires_at_unix_ms: issued.expires_at_ms,
            },
            algorithm: SignatureAlgorithmV1::Ed25519,
            authority_signature: Signature64::from_bytes([14; 64]),
        })
    }

    async fn insert_acceptance_sequence(database: &Database, created_at_ms: i64) {
        let mut transaction = database.pool().begin().await.unwrap();
        sqlx::query("INSERT INTO acceptance_sequences (created_at_ms) VALUES (?)")
            .bind(created_at_ms)
            .execute(&mut *transaction)
            .await
            .unwrap();
        transaction.commit().await.unwrap();
    }

    async fn submission_fixture(database: &Database) -> NewSubmission {
        let public_key = [9; 32];
        let username = database
            .issue_challenge(
                ChallengePurpose::UsernameUpdate,
                public_key,
                Duration::from_secs(60),
                None,
                None,
            )
            .await
            .unwrap();
        database
            .apply_username_update(&username.id, username.nonce, public_key, "Robin")
            .await
            .unwrap();
        let challenge = database
            .issue_challenge(
                ChallengePurpose::Submission,
                public_key,
                Duration::from_secs(60),
                Some("{}"),
                Some("{}"),
            )
            .await
            .unwrap();
        database.register_replay_object(&[1; 32], 4).await.unwrap();
        database
            .register_campaign_object(&[6; 32], 5)
            .await
            .unwrap();
        NewSubmission {
            id: uuid::Uuid::now_v7().to_string(),
            upload_challenge_id: challenge.id,
            offer_json: "{}".to_owned(),
            envelope_json: "{\"immutable\":true}".to_owned(),
            signatures_json: "[]".to_owned(),
            public_metadata_json: "{}".to_owned(),
            replay_sha256: [1; 32],
            replay_bytes: 4,
            build_manifest_id: [2; 32],
            content_manifest_id: [3; 32],
            campaign_content_manifest_id: None,
            config_id: [4; 32],
            ruleset_id: [5; 32],
            mission_id: "mission".to_owned(),
            scope_kind: "individual_level".to_owned(),
            starting_campaign_sha256: [6; 32],
            starting_campaign_bytes: 5,
            controller_public_key: public_key,
            canonical_campaign_state_json: serde_json::to_string(&CanonicalCampaignStatePinV1 {
                requirement: robin_run_protocol::CanonicalCampaignStateRequirementV1 {
                    edition: robin_run_protocol::OfficialContentEditionV1::Demo,
                    kind: robin_run_protocol::CanonicalCampaignStateKindV1::IndividualTemplate,
                    rules_config_sha256: Digest32::from_bytes([4; 32]),
                },
                artifact: ArtifactRefV1 {
                    sha256: Digest32::from_bytes([6; 32]),
                    byte_length: 5,
                    media_type: robin_run_protocol::RANKED_CAMPAIGN_MEDIA_TYPE_V1.to_owned(),
                },
            })
            .unwrap(),
            starting_state_json: "{}".to_owned(),
            campaign_chain_id: None,
            predecessor_run_id: None,
            competition_manifest_id: None,
            requested_metrics_json: "[\"original_score\"]".to_owned(),
            participant_claims_json: "[]".to_owned(),
            max_concurrent_players: 1,
            participant_instance_count: 1,
            session_genesis_sha256: [7; 32],
            session_genesis_host_public_key: public_key,
            replay_session_id: [8; 32],
            session_genesis_host_nonce: [9; 32],
            participants: vec![crate::model::ParticipantClaim {
                seat: 0,
                participant_instance_id: [10; 32],
                public_key,
                public_disclosure: "named_profile".to_owned(),
            }],
        }
    }

    async fn competition_grant_completed(database: &Database, grant_id: &str) -> Option<i64> {
        sqlx::query_scalar::<_, Option<i64>>(
            "SELECT completed_at_ms FROM competition_run_grants WHERE id = ?",
        )
        .bind(grant_id)
        .fetch_one(database.pool())
        .await
        .unwrap()
    }

    fn upload_intent(submission: &NewSubmission) -> SubmissionUploadIntent {
        SubmissionUploadIntent {
            proposed_submission_id: submission.id.clone(),
            upload_challenge_id: submission.upload_challenge_id.clone(),
            offer_json: submission.offer_json.clone(),
            envelope_json: submission.envelope_json.clone(),
            controller_public_key: submission.controller_public_key,
            session_genesis_sha256: submission.session_genesis_sha256,
            session_genesis_host_public_key: submission.session_genesis_host_public_key,
            replay_session_id: submission.replay_session_id,
            session_genesis_host_nonce: submission.session_genesis_host_nonce,
            participants: submission.participants.clone(),
        }
    }

    async fn acquire_upload_fixture(
        database: &Database,
        submission: &NewSubmission,
    ) -> SubmissionUploadLease {
        match database
            .reserve_submission_upload(
                &upload_intent(submission),
                Duration::from_secs(30),
                Duration::from_secs(300),
            )
            .await
            .unwrap()
        {
            SubmissionUploadReservation::Acquired {
                lease,
                resume_uploaded: false,
            } => lease,
            other => panic!("unexpected test reservation result: {other:?}"),
        }
    }

    async fn insert_submission_fixture(
        database: &Database,
        submission: &NewSubmission,
    ) -> SubmissionLifecycle {
        match database
            .reserve_submission_upload(
                &upload_intent(submission),
                Duration::from_secs(30),
                Duration::from_secs(300),
            )
            .await
            .unwrap()
        {
            SubmissionUploadReservation::Acquired { lease, .. } => {
                database
                    .mark_submission_upload_uploaded(&lease)
                    .await
                    .unwrap();
                database
                    .finalize_submission_upload(submission, &lease)
                    .await
                    .unwrap()
            }
            SubmissionUploadReservation::Existing { lifecycle } => lifecycle,
            SubmissionUploadReservation::Busy { retry_after_ms } => {
                panic!("test upload unexpectedly busy for {retry_after_ms} ms")
            }
        }
    }

    async fn insert_indexed_hq_run(database: &Database) -> (String, String, i64) {
        let mut submission = submission_fixture(database).await;
        submission.scope_kind = "campaign".to_owned();
        submission.campaign_chain_id = Some("campaign-chain".to_owned());
        submission.campaign_content_manifest_id = Some([16; 32]);
        let lifecycle = insert_submission_fixture(database, &submission).await;
        sqlx::query("UPDATE submissions SET status = 'accepted' WHERE id = ?")
            .bind(&lifecycle.id)
            .execute(database.pool())
            .await
            .unwrap();
        let accepted_sequence: i64 = sqlx::query_scalar(
            "INSERT INTO acceptance_sequences (created_at_ms) VALUES (1) RETURNING sequence",
        )
        .fetch_one(database.pool())
        .await
        .unwrap();
        let run_id = uuid::Uuid::now_v7().to_string();
        sqlx::query(
            "INSERT INTO verified_runs (id, submission_id, verifier_build_id, \
                build_manifest_id, content_manifest_id, campaign_content_manifest_id, config_id, ruleset_id, mission_id, \
                scope_kind, canonical_campaign_state_json, \
                starting_campaign_sha256, final_campaign_sha256, campaign_chain_id, \
                final_state_sha256, result_sha256, verification_request_sha256, \
                verification_result_json, public_verification_request_sha256, \
                public_verification_request_json, public_verification_result_sha256, \
                public_verification_result_json, public_projection_binding_json, \
                input_provenance_json, terminal_outcome, replay_frames, \
                diagnostics_json, original_score_delta, active_simulation_ticks, ransom_collected, \
                starting_campaign_score, final_campaign_score, campaign_session_kind, \
                campaign_session_ordinal, campaign_hq_sequence, max_concurrent_players, \
                participant_instance_count, named_participant_instance_count, \
                anonymous_participant_instance_count, accepted_sequence, verified_at_ms, \
                starting_campaign_bytes, final_campaign_bytes) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, 'mission', 'campaign', ?, ?, ?, 'campaign-chain', ?, ?, ?, \
                     '{}', ?, '{}', ?, '{}', '{}', '{\"status\":\"rankable\"}', 'won', 1, '{}', 10, 20, 0, 0, 10, \
                     'headquarters', 0, 1, 1, 1, 1, 0, ?, 1, 5, 6)",
        )
        .bind(&run_id)
        .bind(&lifecycle.id)
        .bind([11_u8; 32].as_slice())
        .bind(submission.build_manifest_id.as_slice())
        .bind(submission.content_manifest_id.as_slice())
        .bind(
            submission
                .campaign_content_manifest_id
                .unwrap()
                .as_slice(),
        )
        .bind(submission.config_id.as_slice())
        .bind(submission.ruleset_id.as_slice())
        .bind(&submission.canonical_campaign_state_json)
        .bind(submission.starting_campaign_sha256.as_slice())
        .bind([12_u8; 32].as_slice())
        .bind([13_u8; 32].as_slice())
        .bind([14_u8; 32].as_slice())
        .bind([15_u8; 32].as_slice())
        .bind([31_u8; 32].as_slice())
        .bind([32_u8; 32].as_slice())
        .bind(accepted_sequence)
        .execute(database.pool())
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO verified_run_metrics (run_id, metric, value) \
             VALUES (?, 'original_score', 10)",
        )
        .bind(&run_id)
        .execute(database.pool())
        .await
        .unwrap();
        (run_id, lifecycle.id, accepted_sequence)
    }

    async fn register_test_identity(database: &Database, public_key: [u8; 32], username: &str) {
        let challenge = database
            .issue_challenge(
                ChallengePurpose::UsernameUpdate,
                public_key,
                Duration::from_secs(60),
                None,
                None,
            )
            .await
            .unwrap();
        database
            .apply_username_update(&challenge.id, challenge.nonce, public_key, username)
            .await
            .unwrap();
    }

    #[test]
    fn ranked_score_rejects_signed_campaign_wraparound() {
        assert!(validate_ranked_score(100, 150, 50).is_ok());
        assert!(validate_ranked_score(-100, -50, 50).is_ok());
        assert!(validate_ranked_score(i32::MAX, i32::MIN, 1).is_err());
        assert!(validate_ranked_score(100, 99, u32::MAX as i64).is_err());
        assert!(validate_ranked_score(100, 150, 49).is_err());
    }

    #[test]
    fn stored_achievement_evaluation_is_lossless_and_exhaustive() {
        for evaluation in [
            VerifiedAchievementEvaluationV1::Unverifiable,
            VerifiedAchievementEvaluationV1::NotEarned,
            VerifiedAchievementEvaluationV1::Earned,
        ] {
            let stored = achievement_evaluation_name(evaluation);
            assert_eq!(achievement_evaluation(stored).unwrap(), evaluation);
        }
        assert!(matches!(
            achievement_evaluation("false"),
            Err(DbError::Corrupt(_))
        ));
        assert!(
            !VerifiedAchievementV1 {
                achievement_id: OpaqueId::new("clean-hands").unwrap(),
                evaluation: VerifiedAchievementEvaluationV1::Unverifiable,
                evidence: Default::default(),
            }
            .is_awarded()
        );
    }

    #[tokio::test]
    async fn public_participant_queries_hide_private_ids_and_order_reused_seats_by_key() {
        let (_directory, database) = test_database().await;
        let (run_id, submission_id, _watermark) = insert_indexed_hq_run(&database).await;
        let lower_named_key = [0x11; 32];
        let higher_named_key = [0xe1; 32];
        let anonymous_key = [0x55; 32];
        register_test_identity(&database, lower_named_key, "Alan-a-Dale").await;
        register_test_identity(&database, higher_named_key, "Will Scarlet").await;
        register_test_identity(&database, anonymous_key, "Private Sentinel Name").await;

        // Insert the lower public key with the higher private instance ID. An
        // instance-ID-based query would return these two sequential occupants
        // of seat one in the opposite order.
        for (seat, instance_id, public_key, disclosure) in [
            (1_i64, [0xf1; 32], lower_named_key, "named_profile"),
            (1_i64, [0x12; 32], higher_named_key, "named_profile"),
            (0_i64, [0xa7; 32], anonymous_key, "anonymous"),
        ] {
            sqlx::query(
                "INSERT INTO submission_participants \
                 (submission_id, seat, participant_instance_id, public_key, public_disclosure) \
                 VALUES (?, ?, ?, ?, ?)",
            )
            .bind(&submission_id)
            .bind(seat)
            .bind(instance_id.as_slice())
            .bind(public_key.as_slice())
            .bind(disclosure)
            .execute(database.pool())
            .await
            .unwrap();
        }

        let expected = vec![(0, [9; 32]), (1, lower_named_key), (1, higher_named_key)];
        let participants = database.public_participants_for_run(&run_id).await.unwrap();
        assert_eq!(
            participants
                .iter()
                .map(|participant| (participant.seat, participant.identity.public_key))
                .collect::<Vec<_>>(),
            expected
        );
        assert!(
            participants
                .iter()
                .all(|participant| participant.identity.public_key != anonymous_key),
            "anonymous identities must not enter any public participant projection"
        );

        let batched = database
            .public_participants_for_runs(std::slice::from_ref(&run_id))
            .await
            .unwrap();
        assert_eq!(
            batched[&run_id]
                .iter()
                .map(|participant| (participant.seat, participant.identity.public_key))
                .collect::<Vec<_>>(),
            expected
        );

        let private_ids = sqlx::query_scalar::<_, Vec<u8>>(
            "SELECT participant_instance_id FROM submission_participants \
             WHERE submission_id = ? ORDER BY participant_instance_id",
        )
        .bind(&submission_id)
        .fetch_all(database.pool())
        .await
        .unwrap();
        assert!(private_ids.contains(&vec![0xf1; 32]));
        assert!(private_ids.contains(&vec![0x12; 32]));
        assert!(private_ids.contains(&vec![0xa7; 32]));
    }

    #[tokio::test]
    async fn headquarters_sessions_never_enter_standalone_mission_boards() {
        let (_directory, database) = test_database().await;
        let (run_id, _submission_id, watermark) = insert_indexed_hq_run(&database).await;
        sqlx::query(
            "INSERT INTO campaign_objects \
             (sha256, byte_length, created_at_ms, purge_state) VALUES (?, 1, 1, 'live')",
        )
        .bind([44_u8; 32].as_slice())
        .execute(database.pool())
        .await
        .unwrap();
        for role in ["starting", "final"] {
            sqlx::query(
                "INSERT INTO verified_run_campaign_objects (run_id, role, sha256) \
                 VALUES (?, ?, ?)",
            )
            .bind(&run_id)
            .bind(role)
            .bind([44_u8; 32].as_slice())
            .execute(database.pool())
            .await
            .unwrap();
        }
        assert_eq!(
            database
                .campaign_predecessor(&run_id)
                .await
                .unwrap()
                .chain_owner_public_key,
            [9; 32]
        );
        let filter = robin_run_protocol::RunFilterV1 {
            schema_version: robin_run_protocol::SCHEMA_VERSION_V1,
            subject: robin_run_protocol::LeaderboardSubjectV1::Mission {
                mission_id: "mission".to_owned(),
                category: robin_run_protocol::BoardCategoryV1::Campaign,
            },
            metric: robin_run_protocol::BoardMetricV1::OriginalScore,
            content: robin_run_protocol::RunContentIdentityV1::Mission {
                content_manifest_sha256: robin_run_protocol::Digest32::from_bytes([3; 32]),
            },
            rules_config_sha256: robin_run_protocol::Digest32::from_bytes([4; 32]),
            ruleset_manifest_sha256: robin_run_protocol::Digest32::from_bytes([5; 32]),
            competition_manifest_sha256: None,
            max_concurrent_players: None,
            player_public_key: None,
        };
        assert!(
            database
                .leaderboard_rows(&filter, None, 10, u64::try_from(watermark).unwrap())
                .await
                .unwrap()
                .is_empty()
        );
        assert!(matches!(
            database.public_campaign_for_run(&run_id, "starting").await,
            Err(DbError::NotFound)
        ));
    }

    #[tokio::test]
    async fn public_full_campaign_runs_are_valid_report_targets() {
        let (_directory, database) = test_database().await;
        let (terminal_run_id, _submission_id, accepted_sequence) =
            insert_indexed_hq_run(&database).await;
        let canonical_campaign_state_json: String = sqlx::query_scalar(
            "SELECT canonical_campaign_state_json FROM verified_runs WHERE id = ?",
        )
        .bind(&terminal_run_id)
        .fetch_one(database.pool())
        .await
        .unwrap();
        let aggregate_id = uuid::Uuid::now_v7().to_string();
        sqlx::query(
            "INSERT INTO full_campaign_runs (id, chain_id, terminal_run_id, \
                aggregate_request_sha256, aggregate_sha256, aggregate_json, \
                aggregate_request_json, public_aggregate_request_sha256, \
                public_aggregate_request_json, public_aggregate_result_sha256, \
                public_aggregate_result_json, public_projection_binding_json, \
                campaign_complete_evidence_sha256, campaign_content_manifest_id, config_id, ruleset_id, \
                canonical_campaign_state_json, starting_campaign_sha256, starting_campaign_bytes, \
                final_campaign_sha256, final_campaign_bytes, starting_campaign_score, \
                final_campaign_score, active_simulation_ticks, ransom_collected, \
                max_concurrent_players, participant_instance_count, \
                named_participant_instance_count, anonymous_participant_instance_count, \
                accepted_sequence, verified_at_ms) \
             VALUES (?, 'campaign-chain', ?, ?, ?, '{}', '{}', ?, '{}', ?, '{}', '{}', \
                     ?, ?, ?, ?, ?, ?, 5, ?, 6, 0, 10, 20, 0, \
                     1, 1, 1, 0, ?, 1)",
        )
        .bind(&aggregate_id)
        .bind(&terminal_run_id)
        .bind([21_u8; 32].as_slice())
        .bind([22_u8; 32].as_slice())
        .bind([24_u8; 32].as_slice())
        .bind([25_u8; 32].as_slice())
        .bind([23_u8; 32].as_slice())
        .bind([16_u8; 32].as_slice())
        .bind([4_u8; 32].as_slice())
        .bind([5_u8; 32].as_slice())
        .bind(&canonical_campaign_state_json)
        .bind([6_u8; 32].as_slice())
        .bind([12_u8; 32].as_slice())
        .bind(accepted_sequence + 1)
        .execute(database.pool())
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO full_campaign_sessions (full_campaign_run_id, ordinal, run_id) \
             VALUES (?, 0, ?)",
        )
        .bind(&aggregate_id)
        .bind(&terminal_run_id)
        .execute(database.pool())
        .await
        .unwrap();

        database
            .insert_abuse_report(
                "run",
                &aggregate_id,
                &[[5_u8; 32]],
                "other",
                "aggregate report",
                [99; 32],
                10,
                10,
                10,
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn serving_connection_refuses_to_create_or_migrate_schema() {
        let directory = tempfile::tempdir().unwrap();
        let mut config = ServerConfig::default();
        config.database_path = directory.path().join("highscores.sqlite3");
        assert!(Database::connect(&config).await.is_err());
        assert!(!config.database_path.exists());
        drop(Database::migrate(&config).await.unwrap());
        Database::connect(&config).await.unwrap();
    }

    #[test]
    fn migration_chain_is_one_canonical_production_schema() {
        assert_eq!(CURRENT_SCHEMA_VERSION, 2);
        assert_eq!(MIGRATOR.migrations.len(), 2);
        let migration = &MIGRATOR.migrations[0];
        assert_eq!(migration.version, 1);
        assert_eq!(migration.description.as_ref(), "initial");
        for forbidden in [
            "replay_encoding",
            "public_replay",
            "private_replay",
            "verified_run_public_campaign_objects",
        ] {
            assert!(
                !migration.sql.as_str().contains(forbidden),
                "canonical schema retained obsolete replay namespace {forbidden}",
            );
        }
        let maintenance = &MIGRATOR.migrations[1];
        assert_eq!(maintenance.version, 2);
        assert_eq!(maintenance.description.as_ref(), "maintenance write leases");
        assert!(
            maintenance
                .sql
                .as_str()
                .contains("CREATE TABLE maintenance_write_leases"),
            "append-only maintenance migration omitted its lease authority"
        );
        assert!(
            !migration.sql.as_str().contains("maintenance_write_leases"),
            "the immutable initial migration was rewritten"
        );
    }

    #[tokio::test]
    async fn migration_refuses_a_tampered_canonical_schema_checksum() {
        let directory = tempfile::tempdir().unwrap();
        let mut config = ServerConfig::default();
        config.database_path = directory.path().join("tampered.sqlite3");
        drop(Database::migrate(&config).await.unwrap());

        let options = SqliteConnectOptions::new().filename(&config.database_path);
        let mut connection = sqlx::SqliteConnection::connect_with(&options)
            .await
            .unwrap();
        sqlx::query("UPDATE _sqlx_migrations SET checksum = zeroblob(48) WHERE version = 1")
            .execute(&mut connection)
            .await
            .unwrap();
        connection.close().await.unwrap();

        assert!(matches!(
            Database::migrate(&config).await,
            Err(DbError::Migration(
                sqlx::migrate::MigrateError::VersionMismatch(1)
            ))
        ));
    }

    #[tokio::test]
    async fn username_changes_are_append_only_audited() {
        let (_directory, database) = test_database().await;
        let key = [42; 32];
        for username in ["Robin", "Locksley"] {
            let challenge = database
                .issue_challenge(
                    ChallengePurpose::UsernameUpdate,
                    key,
                    Duration::from_secs(60),
                    None,
                    None,
                )
                .await
                .unwrap();
            database
                .apply_username_update(&challenge.id, challenge.nonce, key, username)
                .await
                .unwrap();
        }
        let rows = sqlx::query(
            "SELECT previous_username, new_username FROM username_history \
             WHERE public_key = ? ORDER BY generation",
        )
        .bind(key.as_slice())
        .fetch_all(database.pool())
        .await
        .unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].get::<Option<String>, _>("previous_username"), None);
        assert_eq!(rows[1].get::<String, _>("previous_username"), "Robin");
        assert_eq!(rows[1].get::<String, _>("new_username"), "Locksley");
    }

    #[tokio::test]
    async fn abuse_report_quotas_are_atomic_per_ip_key_and_target() {
        let (_directory, database) = test_database().await;
        let key = [43; 32];
        let challenge = database
            .issue_challenge(
                ChallengePurpose::UsernameUpdate,
                key,
                Duration::from_secs(60),
                None,
                None,
            )
            .await
            .unwrap();
        database
            .apply_username_update(&challenge.id, challenge.nonce, key, "Marian")
            .await
            .unwrap();
        database
            .insert_abuse_report(
                "player",
                &hex::encode(key),
                &[],
                "other",
                "first",
                [1; 32],
                1,
                10,
                10,
            )
            .await
            .unwrap();
        assert!(matches!(
            database
                .insert_abuse_report(
                    "player",
                    &hex::encode(key),
                    &[],
                    "other",
                    "second",
                    [1; 32],
                    1,
                    10,
                    10,
                )
                .await,
            Err(DbError::QueueFull)
        ));
    }

    #[tokio::test]
    async fn campaign_orphans_are_claimed_and_backup_lock_blocks_gc() {
        let (_directory, database) = test_database().await;
        database
            .register_campaign_object(&[44; 32], 100)
            .await
            .unwrap();
        sqlx::query("UPDATE campaign_objects SET created_at_ms = 0")
            .execute(database.pool())
            .await
            .unwrap();
        let lock = database
            .acquire_backup_lock("test", Duration::from_secs(60))
            .await
            .unwrap();
        assert!(
            database
                .claim_campaign_gc_candidates(10_000, 1, 10)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            database
                .refresh_backup_lock(&lock, Duration::from_secs(60))
                .await
                .unwrap()
        );
        assert!(
            database
                .claimed_campaign_purges(10)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(database.release_backup_lock(&lock).await.unwrap());
        let claimed = database
            .claim_campaign_gc_candidates(10_000, 1, 10)
            .await
            .unwrap();
        assert_eq!(claimed.len(), 1);
        assert_eq!(claimed[0].sha256, [44; 32]);
        assert!(matches!(
            database
                .acquire_backup_lock("blocked", Duration::from_secs(60))
                .await,
            Err(DbError::QueueFull)
        ));
    }

    #[tokio::test]
    async fn backup_gate_closes_before_writer_drain_and_writer_heartbeats_remain_visible() {
        let (_directory, database) = test_database().await;
        let writer = database
            .acquire_maintenance_write_lease(
                MaintenanceWriteClass::ApiSensitive,
                "already-admitted",
                Duration::from_millis(500),
            )
            .await
            .unwrap();
        let backup = database
            .acquire_backup_lock("backup", Duration::from_secs(60))
            .await
            .unwrap();
        assert!(database.backup_lock_active().await.unwrap());
        assert_eq!(
            database
                .active_maintenance_write_lease_count()
                .await
                .unwrap(),
            1
        );
        assert!(matches!(
            database
                .acquire_maintenance_write_lease(
                    MaintenanceWriteClass::ApiSensitive,
                    "late-arrival",
                    Duration::from_secs(60),
                )
                .await,
            Err(DbError::QueueFull)
        ));
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(
            database
                .refresh_maintenance_write_lease(&writer, Duration::from_millis(500))
                .await
                .unwrap()
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(
            database
                .active_maintenance_write_lease_count()
                .await
                .unwrap(),
            1,
            "a long writer which heartbeats must remain visible to backup drain"
        );
        assert!(
            database
                .release_maintenance_write_lease(&writer)
                .await
                .unwrap()
        );
        assert_eq!(
            database
                .active_maintenance_write_lease_count()
                .await
                .unwrap(),
            0
        );
        assert!(database.release_backup_lock(&backup).await.unwrap());
        let after = database
            .acquire_maintenance_write_lease(
                MaintenanceWriteClass::ApiSensitive,
                "after-backup",
                Duration::from_secs(60),
            )
            .await
            .unwrap();
        assert!(
            database
                .release_maintenance_write_lease(&after)
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn maintenance_writer_class_limits_match_the_capacity_model() {
        let directory = tempfile::tempdir().unwrap();
        let mut config = ServerConfig::default();
        config.database_path = directory.path().join("highscores.sqlite3");
        config.max_concurrent_requests = 2;
        config.max_concurrent_uploads = 2;
        let database = Database::migrate(&config).await.unwrap();

        let mut leases = Vec::new();
        for (class, count) in [
            (MaintenanceWriteClass::ApiSensitive, 2),
            (MaintenanceWriteClass::ApiUpload, 2),
            (MaintenanceWriteClass::ApiMaintenance, 1),
            (MaintenanceWriteClass::Worker, 1),
            (MaintenanceWriteClass::Admin, 1),
        ] {
            for ordinal in 0..count {
                leases.push(
                    database
                        .acquire_maintenance_write_lease(
                            class,
                            &format!("{class:?}-{ordinal}"),
                            Duration::from_secs(60),
                        )
                        .await
                        .unwrap(),
                );
            }
            assert!(matches!(
                database
                    .acquire_maintenance_write_lease(
                        class,
                        &format!("{class:?}-overflow"),
                        Duration::from_secs(60),
                    )
                    .await,
                Err(DbError::QueueFull)
            ));
        }
        assert_eq!(
            database
                .active_maintenance_write_lease_count()
                .await
                .unwrap(),
            7,
            "two sensitive + two upload + three singleton writers must coexist exactly"
        );
        for lease in leases {
            assert!(
                database
                    .release_maintenance_write_lease(&lease)
                    .await
                    .unwrap()
            );
        }
    }

    #[tokio::test]
    async fn full_campaign_sessions_are_fetched_in_one_bounded_query_through_4096() {
        let (_directory, database) = test_database().await;
        let mut connection = database.pool().acquire().await.unwrap();
        sqlx::query("PRAGMA foreign_keys = OFF")
            .execute(&mut *connection)
            .await
            .unwrap();
        sqlx::query(
            "WITH RECURSIVE ordinal(value) AS ( \
                 SELECT 0 UNION ALL SELECT value + 1 FROM ordinal WHERE value < 4095 \
             ) INSERT INTO full_campaign_sessions (full_campaign_run_id, ordinal, run_id) \
               SELECT 'aggregate-stress', value, printf('session-%04d', value) FROM ordinal",
        )
        .execute(&mut *connection)
        .await
        .unwrap();
        drop(connection);
        let sessions = database
            .full_campaign_sessions_for_runs(&["aggregate-stress".to_owned()])
            .await
            .unwrap();
        let sessions = sessions.get("aggregate-stress").unwrap();
        assert_eq!(sessions.len(), 4_096);
        assert_eq!(sessions.first().unwrap(), "session-0000");
        assert_eq!(sessions.last().unwrap(), "session-4095");
    }

    #[tokio::test]
    async fn migrations_enable_wal_foreign_keys_and_security_indexes() {
        let (directory, database) = test_database().await;
        let journal_mode: String = sqlx::query_scalar("PRAGMA journal_mode")
            .fetch_one(database.pool())
            .await
            .unwrap();
        assert_eq!(journal_mode, "wal");
        let foreign_keys: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
            .fetch_one(database.pool())
            .await
            .unwrap();
        assert_eq!(foreign_keys, 1);
        let predecessor_index: String = sqlx::query_scalar(
            "SELECT sql FROM sqlite_master WHERE type = 'index' \
             AND name = 'accepted_campaign_predecessor_idx'",
        )
        .fetch_one(database.pool())
        .await
        .unwrap();
        assert!(predecessor_index.contains("status = 'accepted'"));
        let verified_schema: String = sqlx::query_scalar(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'verified_runs'",
        )
        .fetch_one(database.pool())
        .await
        .unwrap();
        assert!(verified_schema.contains("diagnostics_json"));
        assert!(!verified_schema.contains("facts_json"));
        assert!(verified_schema.contains("4294967295"));
        let submission_columns = sqlx::query("PRAGMA table_info(submissions)")
            .fetch_all(database.pool())
            .await
            .unwrap()
            .into_iter()
            .map(|row| row.try_get::<String, _>("name").unwrap())
            .collect::<BTreeSet<_>>();
        assert!(submission_columns.contains("replay_sha256"));
        assert!(submission_columns.contains("replay_bytes"));
        assert!(!submission_columns.contains("replay_encoding"));
        assert!(!submission_columns.contains("public_replay_sha256"));
        assert!(!submission_columns.contains("public_replay_bytes"));
        for required_column in [
            "starting_campaign_bytes",
            "controller_public_key",
            "canonical_campaign_state_json",
        ] {
            let not_null: i64 = sqlx::query_scalar(
                "SELECT \"notnull\" FROM pragma_table_info('submissions') WHERE name = ?",
            )
            .bind(required_column)
            .fetch_one(database.pool())
            .await
            .unwrap();
            assert_eq!(not_null, 1, "submissions.{required_column}");
        }
        let verified_run_columns = sqlx::query("PRAGMA table_info(verified_runs)")
            .fetch_all(database.pool())
            .await
            .unwrap()
            .into_iter()
            .map(|row| row.try_get::<String, _>("name").unwrap())
            .collect::<BTreeSet<_>>();
        for obsolete_projection_column in [
            "public_starting_campaign_sha256",
            "public_starting_campaign_bytes",
            "public_final_campaign_sha256",
            "public_final_campaign_bytes",
            "public_final_state_sha256",
        ] {
            assert!(!verified_run_columns.contains(obsolete_projection_column));
        }
        for required_column in [
            "public_verification_request_sha256",
            "public_verification_request_json",
            "public_verification_result_sha256",
            "public_verification_result_json",
            "public_projection_binding_json",
            "starting_campaign_bytes",
            "final_campaign_bytes",
            "canonical_campaign_state_json",
        ] {
            let not_null: i64 = sqlx::query_scalar(
                "SELECT \"notnull\" FROM pragma_table_info('verified_runs') WHERE name = ?",
            )
            .bind(required_column)
            .fetch_one(database.pool())
            .await
            .unwrap();
            assert_eq!(not_null, 1, "verified_runs.{required_column}");
        }
        let full_campaign_columns = sqlx::query("PRAGMA table_info(full_campaign_runs)")
            .fetch_all(database.pool())
            .await
            .unwrap()
            .into_iter()
            .map(|row| row.try_get::<String, _>("name").unwrap())
            .collect::<BTreeSet<_>>();
        assert!(!full_campaign_columns.contains("content_manifest_id"));
        assert!(full_campaign_columns.contains("campaign_content_manifest_id"));
        for obsolete_projection_column in [
            "public_starting_campaign_sha256",
            "public_starting_campaign_bytes",
            "public_final_campaign_sha256",
            "public_final_campaign_bytes",
        ] {
            assert!(!full_campaign_columns.contains(obsolete_projection_column));
        }
        for required_column in [
            "aggregate_request_json",
            "public_aggregate_request_sha256",
            "public_aggregate_request_json",
            "public_aggregate_result_sha256",
            "public_aggregate_result_json",
            "public_projection_binding_json",
            "campaign_content_manifest_id",
            "starting_campaign_bytes",
            "final_campaign_bytes",
            "canonical_campaign_state_json",
        ] {
            let not_null: i64 = sqlx::query_scalar(
                "SELECT \"notnull\" FROM pragma_table_info('full_campaign_runs') WHERE name = ?",
            )
            .bind(required_column)
            .fetch_one(database.pool())
            .await
            .unwrap();
            assert_eq!(not_null, 1, "full_campaign_runs.{required_column}");
        }
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' \
                 AND name = 'verified_run_public_campaign_objects'",
            )
            .fetch_one(database.pool())
            .await
            .unwrap(),
            0
        );
        let campaign_reference_view: String = sqlx::query_scalar(
            "SELECT sql FROM sqlite_master WHERE type = 'view' \
             AND name = 'campaign_object_submission_references'",
        )
        .fetch_one(database.pool())
        .await
        .unwrap();
        assert!(campaign_reference_view.contains("verified_run_campaign_objects"));
        assert!(!campaign_reference_view.contains("verified_run_public_campaign_objects"));
        let aggregate_participant_columns =
            sqlx::query("PRAGMA table_info(full_campaign_participants)")
                .fetch_all(database.pool())
                .await
                .unwrap()
                .into_iter()
                .map(|row| row.try_get::<String, _>("name").unwrap())
                .collect::<Vec<_>>();
        assert_eq!(
            aggregate_participant_columns,
            ["full_campaign_run_id", "public_key"]
        );
        let reservation_schema: String = sqlx::query_scalar(
            "SELECT sql FROM sqlite_master WHERE type = 'table' \
             AND name = 'submission_upload_reservations'",
        )
        .fetch_one(database.pool())
        .await
        .unwrap();
        assert!(reservation_schema.contains("STRICT"));
        assert!(reservation_schema.contains("UNIQUE(session_genesis_host_public_key"));
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' \
                 AND name IN ('submission_upload_reservations_expiry_idx', \
                              'submission_upload_reservations_lease_idx')",
            )
            .fetch_one(database.pool())
            .await
            .unwrap(),
            2
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(
                std::fs::metadata(directory.path())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o770
            );
            assert_eq!(
                std::fs::metadata(directory.path().join("highscores.sqlite3"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o660
            );
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn symlinked_database_path_is_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("target.sqlite3");
        std::fs::write(&target, []).unwrap();
        let link = directory.path().join("database.sqlite3");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let mut config = ServerConfig::default();
        config.database_path = link;
        let error = Database::connect(&config).await.err().unwrap();
        assert!(matches!(error, DbError::Corrupt(_) | DbError::Sql(_)));
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn symlinked_database_ancestor_is_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let real = directory.path().join("real");
        std::fs::create_dir(&real).unwrap();
        let link = directory.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let mut config = ServerConfig::default();
        config.database_path = link.join("highscores.sqlite3");
        assert!(Database::connect(&config).await.is_err());
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn pinned_database_and_wal_survive_ancestor_swap_and_reopen() {
        let directory = tempfile::tempdir().unwrap();
        let live = directory.path().join("live");
        let mut config = ServerConfig::default();
        config.database_path = live.join("data/highscores.sqlite3");
        let database = Database::migrate(&config).await.unwrap();
        insert_acceptance_sequence(&database, 101).await;

        let displaced = directory.path().join("displaced");
        tokio::fs::rename(&live, &displaced).await.unwrap();
        tokio::fs::create_dir_all(config.database_path.parent().unwrap())
            .await
            .unwrap();
        let sentinels = [
            (config.database_path.clone(), b"main-sentinel".as_slice()),
            (
                PathBuf::from(format!("{}-wal", config.database_path.display())),
                b"wal-sentinel".as_slice(),
            ),
            (
                PathBuf::from(format!("{}-shm", config.database_path.display())),
                b"shm-sentinel".as_slice(),
            ),
        ];
        for (path, bytes) in &sentinels {
            tokio::fs::write(path, bytes).await.unwrap();
        }

        insert_acceptance_sequence(&database, 102).await;
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM acceptance_sequences")
                .fetch_one(database.pool())
                .await
                .unwrap(),
            2
        );
        database.pool().close().await;
        drop(database);
        for (path, bytes) in &sentinels {
            assert_eq!(tokio::fs::read(path).await.unwrap(), *bytes);
        }

        let mut displaced_config = config.clone();
        displaced_config.database_path = displaced.join("data/highscores.sqlite3");
        let reopened = Database::connect(&displaced_config).await.unwrap();
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM acceptance_sequences")
                .fetch_one(reopened.pool())
                .await
                .unwrap(),
            2
        );
        reopened.pool().close().await;
        for (path, bytes) in &sentinels {
            assert_eq!(tokio::fs::read(path).await.unwrap(), *bytes);
        }
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn pinned_database_and_wal_survive_leaf_replacement_and_reopen() {
        let directory = tempfile::tempdir().unwrap();
        let mut config = ServerConfig::default();
        config.database_path = directory.path().join("data/highscores.sqlite3");
        let database = Database::migrate(&config).await.unwrap();
        insert_acceptance_sequence(&database, 201).await;
        let pinned_path = config.database_path.with_file_name("pinned.sqlite3");
        tokio::fs::rename(&config.database_path, &pinned_path)
            .await
            .unwrap();
        for suffix in ["-wal", "-shm"] {
            let original = PathBuf::from(format!("{}{suffix}", config.database_path.display()));
            let pinned = PathBuf::from(format!("{}{suffix}", pinned_path.display()));
            assert!(original.is_file(), "SQLite did not create {suffix}");
            tokio::fs::rename(original, pinned).await.unwrap();
        }
        let sentinels = [
            (config.database_path.clone(), b"main-sentinel".as_slice()),
            (
                PathBuf::from(format!("{}-wal", config.database_path.display())),
                b"wal-sentinel".as_slice(),
            ),
            (
                PathBuf::from(format!("{}-shm", config.database_path.display())),
                b"shm-sentinel".as_slice(),
            ),
        ];
        for (path, bytes) in &sentinels {
            tokio::fs::write(path, bytes).await.unwrap();
        }

        insert_acceptance_sequence(&database, 202).await;
        database.pool().close().await;
        drop(database);
        for (path, bytes) in &sentinels {
            assert_eq!(tokio::fs::read(path).await.unwrap(), *bytes);
        }

        let mut pinned_config = config.clone();
        pinned_config.database_path = pinned_path;
        let reopened = Database::connect(&pinned_config).await.unwrap();
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM acceptance_sequences")
                .fetch_one(reopened.pool())
                .await
                .unwrap(),
            2
        );
        reopened.pool().close().await;
        for (path, bytes) in &sentinels {
            assert_eq!(tokio::fs::read(path).await.unwrap(), *bytes);
        }
    }

    #[tokio::test]
    async fn newer_public_challenge_cannot_starve_an_older_owner_challenge() {
        let (_directory, database) = test_database().await;
        let public_key = [7; 32];
        let old = database
            .issue_challenge(
                ChallengePurpose::UsernameUpdate,
                public_key,
                Duration::from_secs(60),
                None,
                None,
            )
            .await
            .unwrap();
        let current = database
            .issue_challenge(
                ChallengePurpose::UsernameUpdate,
                public_key,
                Duration::from_secs(60),
                None,
                None,
            )
            .await
            .unwrap();

        database
            .apply_username_update(&old.id, old.nonce, public_key, "Old")
            .await
            .unwrap();
        database
            .apply_username_update(&current.id, current.nonce, public_key, "Current")
            .await
            .unwrap();
        assert_eq!(
            database
                .public_identity(&public_key)
                .await
                .unwrap()
                .username,
            "Current"
        );

        let rollback = database
            .issue_challenge(
                ChallengePurpose::UsernameUpdate,
                public_key,
                Duration::from_secs(60),
                None,
                None,
            )
            .await
            .unwrap();
        let newest = database
            .issue_challenge(
                ChallengePurpose::UsernameUpdate,
                public_key,
                Duration::from_secs(60),
                None,
                None,
            )
            .await
            .unwrap();
        database
            .apply_username_update(&newest.id, newest.nonce, public_key, "Newest")
            .await
            .unwrap();
        assert!(matches!(
            database
                .apply_username_update(&rollback.id, rollback.nonce, public_key, "Rollback")
                .await,
            Err(DbError::InvalidChallenge)
        ));
    }

    #[tokio::test]
    async fn purpose_quotas_reserve_submission_offer_capacity() {
        let directory = tempfile::tempdir().unwrap();
        let mut config = ServerConfig::default();
        config.database_path = directory.path().join("highscores.sqlite3");
        config.max_pending_submissions = 2;
        let database = Database::migrate(&config).await.unwrap();
        for key in [[1_u8; 32], [2_u8; 32]] {
            database
                .issue_challenge(
                    ChallengePurpose::UsernameUpdate,
                    key,
                    Duration::from_secs(60),
                    None,
                    None,
                )
                .await
                .unwrap();
        }
        assert!(matches!(
            database
                .issue_challenge(
                    ChallengePurpose::UsernameUpdate,
                    [3_u8; 32],
                    Duration::from_secs(60),
                    None,
                    None,
                )
                .await,
            Err(DbError::QueueFull)
        ));
        database
            .issue_challenge(
                ChallengePurpose::Submission,
                [4_u8; 32],
                Duration::from_secs(60),
                Some("{}"),
                Some("{}"),
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn failed_offer_construction_leaves_no_challenge_or_generation() {
        let (_directory, database) = test_database().await;
        let result = database
            .issue_submission_offer(
                [5_u8; 32],
                Duration::from_secs(60),
                None,
                |_issued| -> Result<((), String, String), DbError> {
                    Err(DbError::ResultInvariant("invalid offer fixture".to_owned()))
                },
            )
            .await;
        assert!(matches!(result, Err(DbError::ResultInvariant(_))));
        let challenges: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM upload_challenges WHERE purpose = 'submission'",
        )
        .fetch_one(database.pool())
        .await
        .unwrap();
        let generations: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM challenge_generations WHERE purpose = 'submission'",
        )
        .fetch_one(database.pool())
        .await
        .unwrap();
        assert_eq!((challenges, generations), (0, 0));
    }

    #[tokio::test]
    async fn competition_grants_are_end_capped_retry_stable_and_single_live() {
        let (_directory, database) = test_database().await;
        let host = [0x31; 32];
        register_test_identity(&database, host, "Grant Host").await;
        let first_request = grant_request(host, 31);
        let now = u64::try_from(now_epoch_ms().unwrap()).unwrap();
        let exclusive_end = now + 30_000;
        assert!(matches!(
            database
                .issue_competition_run_grant(&first_request, now + 10_000, now + 20_000, |issued| {
                    build_test_grant(&first_request, issued)
                },)
                .await,
            Err(DbError::InvalidChallenge)
        ));
        assert!(matches!(
            database
                .issue_competition_run_grant(
                    &first_request,
                    now.saturating_sub(10_000),
                    now,
                    |issued| build_test_grant(&first_request, issued),
                )
                .await,
            Err(DbError::InvalidChallenge)
        ));
        let first = database
            .issue_competition_run_grant(
                &first_request,
                now.saturating_sub(1),
                exclusive_end,
                |issued| build_test_grant(&first_request, issued),
            )
            .await
            .unwrap();
        assert_eq!(first.claim.expires_at_unix_ms, exclusive_end - 1);
        let retried = database
            .issue_competition_run_grant(
                &first_request,
                now.saturating_sub(1),
                exclusive_end,
                |issued| build_test_grant(&first_request, issued),
            )
            .await
            .unwrap();
        assert_eq!(retried, first);
        let mut replayed_session_request = first_request.clone();
        replayed_session_request.claim.request_nonce = ChallengeNonce32::from_bytes([91; 32]);
        replayed_session_request.host_signature = Signature64::from_bytes([92; 64]);
        assert!(matches!(
            database
                .issue_competition_run_grant(
                    &replayed_session_request,
                    now.saturating_sub(1),
                    exclusive_end,
                    |issued| build_test_grant(&replayed_session_request, issued),
                )
                .await,
            Err(DbError::InvalidChallenge)
        ));

        let replacement_request = grant_request(host, 41);
        let replacement = database
            .issue_competition_run_grant(
                &replacement_request,
                now.saturating_sub(1),
                exclusive_end,
                |issued| build_test_grant(&replacement_request, issued),
            )
            .await
            .unwrap();
        assert!(matches!(
            database
                .issue_submission_offer(host, Duration::from_secs(60), Some(&first), |_| Ok((
                    (),
                    "{}".into(),
                    "{}".into()
                )),)
                .await,
            Err(DbError::InvalidChallenge)
        ));

        let (replacement_offer, ()) = database
            .issue_submission_offer(host, Duration::from_secs(60), Some(&replacement), |_| {
                Ok(((), "{}".into(), "{}".into()))
            })
            .await
            .unwrap();
        let third_request = grant_request(host, 51);
        assert!(matches!(
            database
                .issue_competition_run_grant(
                    &third_request,
                    now.saturating_sub(1),
                    exclusive_end,
                    |issued| build_test_grant(&third_request, issued),
                )
                .await,
            Err(DbError::InvalidChallenge)
        ));

        sqlx::query("UPDATE upload_challenges SET consumed_at_ms = ? WHERE id = ?")
            .bind(now_epoch_ms().unwrap())
            .bind(&replacement_offer.id)
            .execute(database.pool())
            .await
            .unwrap();
        assert!(matches!(
            database
                .issue_competition_run_grant(
                    &third_request,
                    now.saturating_sub(1),
                    exclusive_end,
                    |issued| build_test_grant(&third_request, issued),
                )
                .await,
            Err(DbError::InvalidChallenge)
        ));
        sqlx::query(
            "UPDATE competition_run_grants SET completed_at_ms = ? WHERE upload_challenge_id = ?",
        )
        .bind(now_epoch_ms().unwrap())
        .bind(&replacement_offer.id)
        .execute(database.pool())
        .await
        .unwrap();
        database
            .issue_competition_run_grant(
                &third_request,
                now.saturating_sub(1),
                exclusive_end,
                |issued| build_test_grant(&third_request, issued),
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn competition_upload_deadline_and_completion_are_transactionally_bound() {
        let (_directory, database) = test_database().await;
        let mut submission = submission_fixture(&database).await;
        let host = submission.session_genesis_host_public_key;
        let request = grant_request(host, 71);
        let now = u64::try_from(now_epoch_ms().unwrap()).unwrap();
        let grant = database
            .issue_competition_run_grant(&request, now.saturating_sub(1), now + 30_000, |issued| {
                build_test_grant(&request, issued)
            })
            .await
            .unwrap();
        let (offer, ()) = database
            .issue_submission_offer(host, Duration::from_secs(3_600), Some(&grant), |_| {
                Ok(((), "{}".into(), "{}".into()))
            })
            .await
            .unwrap();
        assert!(offer.expires_at_ms <= grant.claim.expires_at_unix_ms);

        submission.upload_challenge_id = offer.id.clone();
        submission.competition_manifest_id = Some(
            request
                .claim
                .ranked_session
                .competition_manifest_sha256
                .expect("competition fixture")
                .into_bytes(),
        );
        let reservation = database
            .reserve_submission_upload(
                &upload_intent(&submission),
                Duration::from_secs(30),
                Duration::from_secs(24 * 60 * 60),
            )
            .await
            .unwrap();
        let SubmissionUploadReservation::Acquired { lease, .. } = reservation else {
            panic!("competition upload did not acquire its exact reservation")
        };
        assert!(lease.lease_expires_at_ms <= offer.expires_at_ms);
        assert!(lease.reservation_expires_at_ms <= offer.expires_at_ms);
        assert!(lease.reservation_expires_at_ms <= grant.claim.expires_at_unix_ms);
        database
            .mark_submission_upload_uploaded(&lease)
            .await
            .unwrap();
        assert_eq!(
            competition_grant_completed(&database, grant.claim.grant_id.as_str()).await,
            None
        );

        let mut mismatched = submission.clone();
        mismatched.competition_manifest_id = None;
        assert!(matches!(
            database
                .finalize_submission_upload(&mismatched, &lease)
                .await,
            Err(DbError::ResultInvariant(_))
        ));
        assert_eq!(
            competition_grant_completed(&database, grant.claim.grant_id.as_str()).await,
            None
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM submissions WHERE id = ?")
                .bind(&submission.id)
                .fetch_one(database.pool())
                .await
                .unwrap(),
            0,
            "failed insertion must roll back before completing the grant"
        );

        database
            .finalize_submission_upload(&submission, &lease)
            .await
            .unwrap();
        assert!(
            competition_grant_completed(&database, grant.claim.grant_id.as_str())
                .await
                .is_some()
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM submissions WHERE id = ?")
                .bind(&submission.id)
                .fetch_one(database.pool())
                .await
                .unwrap(),
            1,
            "grant completion must accompany a durable submission row"
        );
    }

    #[tokio::test]
    async fn concurrent_competition_offer_exchange_has_one_winner() {
        let (_directory, database) = test_database().await;
        let host = [0x61; 32];
        register_test_identity(&database, host, "Racing Grant Host").await;
        let request = grant_request(host, 61);
        let now = u64::try_from(now_epoch_ms().unwrap()).unwrap();
        let grant = database
            .issue_competition_run_grant(&request, now.saturating_sub(1), now + 30_000, |issued| {
                build_test_grant(&request, issued)
            })
            .await
            .unwrap();

        let first =
            database.issue_submission_offer(host, Duration::from_secs(60), Some(&grant), |_| {
                Ok(((), "{}".into(), "{}".into()))
            });
        let second =
            database.issue_submission_offer(host, Duration::from_secs(60), Some(&grant), |_| {
                Ok(((), "{}".into(), "{}".into()))
            });
        let (first, second) = tokio::join!(first, second);
        assert_eq!(usize::from(first.is_ok()) + usize::from(second.is_ok()), 1);
        let loser = if first.is_err() { first } else { second };
        assert!(matches!(loser, Err(DbError::InvalidChallenge)));
    }

    #[tokio::test]
    async fn expired_rejected_submission_releases_its_replay_reference() {
        let (_directory, database) = test_database().await;
        let submission = submission_fixture(&database).await;
        let inserted = insert_submission_fixture(&database, &submission).await;
        sqlx::query(
            "UPDATE submissions SET status = 'rejected', rejection_code = 'malformed_replay' \
             WHERE id = ?",
        )
        .bind(&inserted.id)
        .execute(database.pool())
        .await
        .unwrap();
        let rejected_at: i64 =
            sqlx::query_scalar("SELECT updated_at_ms FROM submissions WHERE id = ?")
                .bind(&inserted.id)
                .fetch_one(database.pool())
                .await
                .unwrap();
        let tombstoned_at = u64::try_from(rejected_at).unwrap() + 1;

        assert_eq!(
            database
                .expire_rejected_submissions(u64::try_from(rejected_at).unwrap(), tombstoned_at)
                .await
                .unwrap(),
            1
        );
        let tombstone: Option<i64> =
            sqlx::query_scalar("SELECT tombstoned_at_ms FROM submissions WHERE id = ?")
                .bind(&inserted.id)
                .fetch_one(database.pool())
                .await
                .unwrap();
        assert_eq!(tombstone, Some(i64::try_from(tombstoned_at).unwrap()));
        let candidates = database
            .claim_replay_gc_candidates(tombstoned_at + 1, tombstoned_at, 10)
            .await
            .unwrap();
        assert_eq!(candidates.len(), 1);
        let candidate_digests = candidates
            .into_iter()
            .map(|candidate| candidate.sha256)
            .collect::<BTreeSet<_>>();
        assert_eq!(
            candidate_digests,
            BTreeSet::from([submission.replay_sha256])
        );
    }

    #[tokio::test]
    async fn exhausted_infrastructure_failure_is_terminal_private_and_not_a_rejection() {
        let (_directory, database) = test_database().await;
        let submission = submission_fixture(&database).await;
        let inserted = insert_submission_fixture(&database, &submission).await;
        let job = database
            .lease_next("worker-1", Duration::from_secs(60))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(job.submission_id, inserted.id);
        let request_artifact_sha256 = [42_u8; 32];
        database
            .fail_job(
                &inserted.id,
                "worker-1",
                Some(&request_artifact_sha256),
                "private verifier I/O failure",
            )
            .await
            .unwrap();

        let lifecycle = database.submission_lifecycle(&inserted.id).await.unwrap();
        assert_eq!(lifecycle.state, crate::model::SubmissionState::Failed);
        assert!(
            database
                .lease_next("worker-2", Duration::from_secs(60))
                .await
                .unwrap()
                .is_none(),
            "terminal infrastructure failures must never be leased again"
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM verified_runs")
                .fetch_one(database.pool())
                .await
                .unwrap(),
            0,
            "a killed, OOMed, or timed-out verifier must not create public state"
        );
        assert!(matches!(
            database.replay_for_run(&inserted.id).await,
            Err(DbError::NotFound)
        ));
        let row = sqlx::query(
            "SELECT code, request_artifact_sha256, private_detail \
             FROM submission_terminal_failures WHERE submission_id = ?",
        )
        .bind(&inserted.id)
        .fetch_one(database.pool())
        .await
        .unwrap();
        assert_eq!(row.get::<String, _>("code"), "verification_infrastructure");
        assert_eq!(
            row.get::<Vec<u8>, _>("request_artifact_sha256"),
            request_artifact_sha256
        );
        assert_eq!(
            row.get::<String, _>("private_detail"),
            "private verifier I/O failure"
        );
    }

    #[tokio::test]
    async fn concurrent_challenge_issuance_has_unique_monotonic_generations() {
        let (_directory, database) = test_database().await;
        let mut tasks = Vec::new();
        for _ in 0..16 {
            let database = database.clone();
            tasks.push(tokio::spawn(async move {
                database
                    .issue_challenge(
                        ChallengePurpose::UsernameUpdate,
                        [8; 32],
                        Duration::from_secs(60),
                        None,
                        None,
                    )
                    .await
                    .unwrap();
            }));
        }
        for task in tasks {
            task.await.unwrap();
        }
        let row = sqlx::query(
            "SELECT COUNT(*) AS total, COUNT(DISTINCT generation) AS distinct_total, \
                    MAX(generation) AS maximum \
             FROM upload_challenges WHERE public_key = ? AND purpose = 'username_update'",
        )
        .bind([8; 32].as_slice())
        .fetch_one(database.pool())
        .await
        .unwrap();
        assert_eq!(row.get::<i64, _>("total"), 16);
        assert_eq!(row.get::<i64, _>("distinct_total"), 16);
        assert_eq!(row.get::<i64, _>("maximum"), 16);
    }

    #[tokio::test]
    async fn upload_reservation_is_pre_stream_exact_retryable_and_single_publish() {
        let (_directory, database) = test_database().await;
        let submission = submission_fixture(&database).await;
        assert!(matches!(
            database
                .reserve_submission_upload_if_admitted(
                    &upload_intent(&submission),
                    Duration::from_secs(30),
                    Duration::from_secs(300),
                    false,
                )
                .await,
            Err(DbError::AdmissionUnavailable)
        ));
        assert_eq!(
            sqlx::query_scalar::<_, Option<i64>>(
                "SELECT consumed_at_ms FROM upload_challenges WHERE id = ?"
            )
            .bind(&submission.upload_challenge_id)
            .fetch_one(database.pool())
            .await
            .unwrap(),
            None,
            "red admission must not consume the one-use challenge"
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM submission_upload_reservations")
                .fetch_one(database.pool())
                .await
                .unwrap(),
            0,
            "red admission must not create a reservation"
        );
        let lease = acquire_upload_fixture(&database, &submission).await;

        let retry_offer = database
            .stored_offer(&submission.upload_challenge_id)
            .await
            .expect("the durable reservation keeps exact-body HTTP retries routable");
        assert_eq!(retry_offer.offer_json, submission.offer_json);
        assert_eq!(
            retry_offer.public_metadata_json,
            submission.public_metadata_json
        );
        assert!(matches!(
            database
                .reserve_submission_upload_if_admitted(
                    &upload_intent(&submission),
                    Duration::from_secs(30),
                    Duration::from_secs(300),
                    false,
                )
                .await
                .unwrap(),
            SubmissionUploadReservation::Busy { .. }
        ));
        let mut conflicting_intent = upload_intent(&submission);
        conflicting_intent.envelope_json = "{\"immutable\":false}".to_owned();
        assert!(matches!(
            database
                .reserve_submission_upload(
                    &conflicting_intent,
                    Duration::from_secs(30),
                    Duration::from_secs(300),
                )
                .await,
            Err(DbError::SubmissionConflict)
        ));
        assert!(database.abandon_submission_upload(&lease).await.unwrap());
        assert!(matches!(
            database
                .reserve_submission_upload_if_admitted(
                    &upload_intent(&submission),
                    Duration::from_secs(30),
                    Duration::from_secs(300),
                    false,
                )
                .await,
            Err(DbError::AdmissionUnavailable)
        ));
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM submissions")
                .fetch_one(database.pool())
                .await
                .unwrap(),
            0,
            "a partial upload must not create a queue job"
        );

        let mut retry_intent = upload_intent(&submission);
        retry_intent.proposed_submission_id = uuid::Uuid::now_v7().to_string();
        let retried = database
            .reserve_submission_upload(
                &retry_intent,
                Duration::from_secs(30),
                Duration::from_secs(300),
            )
            .await
            .unwrap();
        let SubmissionUploadReservation::Acquired {
            lease: retry_lease,
            resume_uploaded: false,
        } = retried
        else {
            panic!("abandoned exact retry was not reacquired")
        };
        assert_eq!(retry_lease.submission_id, submission.id);
        database
            .mark_submission_upload_uploaded(&retry_lease)
            .await
            .unwrap();
        let inserted = database
            .finalize_submission_upload(&submission, &retry_lease)
            .await
            .unwrap();
        let max_concurrent_players: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM submission_participants WHERE submission_id = ?",
        )
        .bind(&inserted.id)
        .fetch_one(database.pool())
        .await
        .unwrap();
        assert_eq!(max_concurrent_players, 1);
        let completed_retry = database
            .reserve_submission_upload_if_admitted(
                &retry_intent,
                Duration::from_secs(30),
                Duration::from_secs(300),
                false,
            )
            .await
            .unwrap();
        let SubmissionUploadReservation::Existing { lifecycle } = completed_retry else {
            panic!("completed exact retry did not return its lifecycle")
        };
        assert_eq!(lifecycle.id, inserted.id);
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM submissions")
                .fetch_one(database.pool())
                .await
                .unwrap(),
            1
        );
        assert!(
            database
                .lease_next("only-worker", Duration::from_secs(30))
                .await
                .unwrap()
                .is_some()
        );
        assert!(
            database
                .lease_next("no-duplicate-worker", Duration::from_secs(30))
                .await
                .unwrap()
                .is_none(),
            "an exact retry must not create a second verifier job"
        );
    }

    #[tokio::test]
    async fn concurrent_cross_instance_reservation_has_one_ingestion_lease() {
        let directory = tempfile::tempdir().unwrap();
        let mut config = ServerConfig::default();
        config.database_path = directory.path().join("highscores.sqlite3");
        let left_db = Database::migrate(&config).await.unwrap();
        let right_db = Database::connect(&config).await.unwrap();
        let submission = submission_fixture(&left_db).await;
        let left_intent = upload_intent(&submission);
        let mut right_intent = left_intent.clone();
        right_intent.proposed_submission_id = uuid::Uuid::now_v7().to_string();
        let (left, right) = tokio::join!(
            left_db.reserve_submission_upload(
                &left_intent,
                Duration::from_secs(30),
                Duration::from_secs(300),
            ),
            right_db.reserve_submission_upload(
                &right_intent,
                Duration::from_secs(30),
                Duration::from_secs(300),
            )
        );
        let results = [left.unwrap(), right.unwrap()];
        assert_eq!(
            results
                .iter()
                .filter(|result| matches!(result, SubmissionUploadReservation::Acquired { .. }))
                .count(),
            1
        );
        assert_eq!(
            results
                .iter()
                .filter(|result| matches!(result, SubmissionUploadReservation::Busy { .. }))
                .count(),
            1
        );
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM submissions")
            .fetch_one(left_db.pool())
            .await
            .unwrap();
        assert_eq!(count, 0);
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM submission_upload_reservations")
                .fetch_one(left_db.pool())
                .await
                .unwrap(),
            1
        );
    }

    #[tokio::test]
    async fn uploaded_crash_recovery_reuses_artifacts_and_canonical_id() {
        let (_directory, database) = test_database().await;
        let submission = submission_fixture(&database).await;
        let lease = acquire_upload_fixture(&database, &submission).await;
        database
            .mark_submission_upload_uploaded(&lease)
            .await
            .unwrap();
        sqlx::query(
            "UPDATE submission_upload_reservations \
             SET lease_expires_at_ms = reserved_at_ms + 1 WHERE upload_challenge_id = ?",
        )
        .bind(&submission.upload_challenge_id)
        .execute(database.pool())
        .await
        .unwrap();
        tokio::time::sleep(Duration::from_millis(2)).await;
        database.recover_upload_reservations().await.unwrap();

        let resumed = database
            .reserve_submission_upload_if_admitted(
                &upload_intent(&submission),
                Duration::from_secs(30),
                Duration::from_secs(300),
                false,
            )
            .await
            .unwrap();
        let SubmissionUploadReservation::Acquired {
            lease: resumed_lease,
            resume_uploaded: true,
        } = resumed
        else {
            panic!("uploaded exact recovery was not resumed while new admission was red")
        };
        assert_eq!(resumed_lease.submission_id, submission.id);
        database
            .finalize_submission_upload(&submission, &resumed_lease)
            .await
            .unwrap();
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM submissions")
                .fetch_one(database.pool())
                .await
                .unwrap(),
            1
        );
    }

    #[tokio::test]
    async fn expired_abandoned_reservation_and_challenge_are_bounded() {
        let (_directory, database) = test_database().await;
        let submission = submission_fixture(&database).await;
        let lease = acquire_upload_fixture(&database, &submission).await;
        assert!(database.abandon_submission_upload(&lease).await.unwrap());
        sqlx::query(
            "UPDATE submission_upload_reservations \
             SET reservation_expires_at_ms = reserved_at_ms + 1 \
             WHERE upload_challenge_id = ?",
        )
        .bind(&submission.upload_challenge_id)
        .execute(database.pool())
        .await
        .unwrap();
        tokio::time::sleep(Duration::from_millis(2)).await;
        assert!(database.recover_upload_reservations().await.unwrap() >= 1);
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM submission_upload_reservations \
                 WHERE upload_challenge_id = ?",
            )
            .bind(&submission.upload_challenge_id)
            .fetch_one(database.pool())
            .await
            .unwrap(),
            0
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM upload_challenges WHERE id = ?")
                .bind(&submission.upload_challenge_id)
                .fetch_one(database.pool())
                .await
                .unwrap(),
            0
        );
        assert!(matches!(
            database
                .reserve_submission_upload(
                    &upload_intent(&submission),
                    Duration::from_secs(30),
                    Duration::from_secs(300),
                )
                .await,
            Err(DbError::InvalidChallenge)
        ));
    }
}
