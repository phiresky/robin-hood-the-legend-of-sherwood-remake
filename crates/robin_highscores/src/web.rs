mod readiness;

#[cfg(test)]
use readiness::backup_age_ms_with_active_release;
use readiness::{backup_age_ms, ensure_backup_ready};
#[cfg(all(test, target_os = "linux"))]
use readiness::{
    pin_readiness_status_parent, read_bounded_from_pinned_status_parent, read_bounded_nofollow,
};

#[cfg(test)]
use crate::backup::BackupStatusV4;
#[cfg(test)]
use crate::backup::{
    BackupFileV4, BackupManifestProjectionV4, BackupRestoreSourceV4,
    load_backup_release_identity_oob,
};
use crate::config::{AdmissionProfile, CompetitionConfig, ViewerContentRequirementConfig};
use crate::db::{BoardComposition, BoardCursor, BoardRow};
#[cfg(test)]
use crate::db::{SubmissionUploadIntent, SubmissionUploadReservation};
use crate::error::ApiError;
use crate::identity::validate_username;
#[cfg(test)]
use crate::identity::verify_signature;
#[cfg(test)]
use crate::model::NewSubmission;
use crate::model::{ChallengePurpose, ParticipantClaim};
use crate::storage_admission::{
    StorageAdmissionError, ensure_offer_capacity, ensure_upload_capacity,
};
use crate::{CampaignStore, Database, ReplayStore, ServerConfig};
use axum::body::Body;
use axum::extract::{ConnectInfo, DefaultBodyLimit, Multipart, Path, Query, State};
use axum::http::header::{
    CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_ENCODING, CONTENT_LENGTH, CONTENT_TYPE,
    X_CONTENT_TYPE_OPTIONS,
};
use axum::http::{HeaderMap, HeaderValue, Method, Request, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse as _, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use base64::Engine as _;
use bytes::Bytes;
use ed25519_dalek::Signer as _;
use futures_util::stream;
use robin_run_protocol::{
    AbuseReportAcceptedV1, AbuseReportCategoryV1, AbuseReportTargetV1, AbuseReportV1,
    AchievementSummaryV1, AnonymousParticipantPolicyV1, ArtifactRefV1, BoardCategoryV1,
    BoardMetricV1, BoardMetricValueV1, CampaignChainReceiptV1, CampaignChainStateV1,
    CampaignContinuationPreflightGrantClaimV1, CampaignContinuationPreflightGrantV1,
    CampaignContinuationPreflightRequestV1, CampaignRosterContinuityV1, CampaignSessionDetailV1,
    CanonicalDocument, ChallengeNonce32, CompetitionManifestV1, CompetitionRunGrantClaimV1,
    CompetitionRunGrantRequestV1, CompetitionRunGrantV1, CompetitionStateV1, CompetitionSummaryV1,
    DeletionChallengeRequestV1, DeletionChallengeV1, DeletionReceiptV1, DeletionRequestEnvelopeV1,
    DeletionTargetV1, Digest32, FreshRunPreflightGrantClaimV1, FreshRunPreflightGrantV1,
    FreshRunPreflightRequestV1, FreshRunScopeV1, FullCampaignFacetV1, FullCampaignSessionKindV1,
    FullCampaignSessionV1, InitialStateExpectationV1, InputProvenanceStatusV1, LeaderboardCursorV1,
    LeaderboardEntryV1, LeaderboardMetadataV1, LeaderboardOrderAnchorV1, LeaderboardPageV1,
    LeaderboardQueryV1, LeaderboardSubjectV1, MissionFacetV1, OpaqueId,
    ParticipantPublicDisclosureV1, PlayerPersonalBestV1, PlayerProfileV1, PlayerRunHistoryEntryV1,
    PlayerRunHistoryPageV1, PlayerRunHistoryQueryV1, PublicBuildV1, PublicKey32,
    PublicParticipantV1, RANKED_CAMPAIGN_MEDIA_TYPE_V1, RANKED_REPLAY_MEDIA_TYPE_V1,
    RulesetBoardScopeV1, RulesetFacetV1, RulesetOperationalStatusV1, RunContentIdentityV1,
    RunDetailV1, RunFilterV1, RunMetricsV1, SCHEMA_VERSION_V1, ScopeRequestV1, Signature64,
    SignatureAlgorithmV1, SignedSubmissionV1, SubmissionAcceptedV1, SubmissionFailureCodeV1,
    SubmissionLifecycleV1, SubmissionOfferRequestV1, SubmissionOfferV1,
    SubmissionOwnerStatusChallengeRequestV1, SubmissionOwnerStatusChallengeV1,
    SubmissionOwnerStatusEnvelopeV1, SubmissionOwnerStatusResponseV1, TerminalOutcomeV1,
    TickDurationV1, UsernameChallengeRequestV1, UsernameChallengeV1, UsernameUpdateEnvelopeV1,
    Validate as _, VerificationRejectionCodeV1, VerifiedRunCompositionV1, ViewerAvailabilityV1,
    ViewerContentRequirementV1, ViewerLaunchV1,
};
use serde::{Deserialize, Serialize};
use sha2::Digest as _;
use std::collections::{BTreeMap, BTreeSet};
use std::collections::{HashMap, VecDeque};
use std::convert::Infallible;
use std::net::{IpAddr, SocketAddr};
use std::str::FromStr as _;
use std::sync::Arc;
use std::time::Duration;
use tokio_util::io::ReaderStream;
use tower_http::cors::CorsLayer;
use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::timeout::{RequestBodyTimeoutLayer, TimeoutLayer};
use tower_http::trace::TraceLayer;

const SUBMISSION_RETRY_AFTER_MS: u64 = 2_000;
const RANKED_REPLAY_SCHEMA_VERSION: u32 =
    robin_run_protocol::CURRENT_RANKED_REPLAY_SCHEMA_VERSION_V1;
const MULTIPART_ENVELOPE_OVERHEAD_BYTES: usize = 1024 * 1024;

fn submission_body_limit(config: &ServerConfig) -> Result<usize, ApiError> {
    let replay = usize::try_from(config.max_replay_bytes).map_err(|_| ApiError::Internal)?;
    let starting_campaign =
        usize::try_from(config.max_campaign_bytes).map_err(|_| ApiError::Internal)?;
    replay
        .checked_add(starting_campaign)
        .and_then(|value| value.checked_add(config.max_metadata_bytes))
        .and_then(|value| value.checked_add(MULTIPART_ENVELOPE_OVERHEAD_BYTES))
        .ok_or(ApiError::Internal)
}

async fn read_bounded_field(
    mut field: axum::extract::multipart::Field<'_>,
    limit: usize,
) -> Result<Vec<u8>, ApiError> {
    let mut bytes = Vec::with_capacity(limit.min(16 * 1024));
    while let Some(chunk) = field
        .chunk()
        .await
        .map_err(|error| ApiError::BadRequest(error.to_string()))?
    {
        let next_len = bytes
            .len()
            .checked_add(chunk.len())
            .ok_or_else(|| ApiError::PayloadTooLarge("multipart field is too large".to_owned()))?;
        if next_len > limit {
            return Err(ApiError::PayloadTooLarge(format!(
                "multipart field exceeds the {limit}-byte limit"
            )));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn require_multipart_media_type(
    field: &axum::extract::multipart::Field<'_>,
    expected: &str,
    role: &str,
) -> Result<(), ApiError> {
    if field.headers().contains_key(CONTENT_ENCODING)
        || field.headers().contains_key("content-transfer-encoding")
    {
        return Err(ApiError::BadRequest(format!(
            "`{role}` multipart field must not use a transport content encoding"
        )));
    }
    if field.content_type() != Some(expected) {
        return Err(ApiError::BadRequest(format!(
            "`{role}` multipart content type must exactly match its signed media type"
        )));
    }
    Ok(())
}

/// Apply the cheap, exact transport grammar at the public HTTP boundary.
///
/// This deliberately does not base64-decode, decompress, or deserialize the
/// replay. Those attacker-controlled expansion stages remain exclusive to
/// the contained verifier. The bounded byte buffer lets us reject a JSONL or
/// alternate-format upload before consuming its one-use upload challenge or
/// creating anything in the durable replay store.
fn preflight_ranked_replay_transport(
    bytes: &[u8],
    artifact: &ArtifactRefV1,
    required_build_hash: &str,
) -> Result<(), ApiError> {
    let actual_bytes = u64::try_from(bytes.len()).map_err(|_| {
        ApiError::PayloadTooLarge("replay byte length does not fit the transport".to_owned())
    })?;
    if actual_bytes != artifact.byte_length {
        return Err(ApiError::BadRequest(format!(
            "replay length mismatch: signed {}, received {actual_bytes}",
            artifact.byte_length
        )));
    }
    if Digest32::digest_bytes(bytes) != artifact.sha256 {
        return Err(ApiError::BadRequest(
            "replay SHA-256 does not match the signed envelope".to_owned(),
        ));
    }
    let text = std::str::from_utf8(bytes).map_err(|_| {
        ApiError::BadRequest(
            "ranked replay must use the canonical ASCII compact transport".to_owned(),
        )
    })?;
    let preflight = robin_replay_format::preflight_compact_transport(
        text,
        &robin_replay_format::DEFAULT_REPLAY_ADMISSION_LIMITS,
    )
    .map_err(|error| {
        ApiError::BadRequest(format!(
            "ranked replay is not the canonical compact transport: {error}"
        ))
    })?;
    // Unpadded base64url still has unused tail bits for payload lengths 2 or
    // 3 modulo 4. A non-zero unused bit is an alternate spelling of the same
    // decoded bytes, so reject it lexically without invoking a decoder.
    let tail_is_canonical = match preflight.base64_payload.len() % 4 {
        0 => true,
        2 => preflight
            .base64_payload
            .as_bytes()
            .last()
            .and_then(|byte| base64url_sextet(*byte))
            .is_some_and(|value| value & 0b00_1111 == 0),
        3 => preflight
            .base64_payload
            .as_bytes()
            .last()
            .and_then(|byte| base64url_sextet(*byte))
            .is_some_and(|value| value & 0b00_0011 == 0),
        _ => false,
    };
    if !tail_is_canonical {
        return Err(ApiError::BadRequest(
            "ranked replay base64url text has non-canonical trailing bits".to_owned(),
        ));
    }
    if preflight.version_hash != required_build_hash {
        return Err(ApiError::BadRequest(
            "ranked replay build identity does not match the signed build manifest".to_owned(),
        ));
    }
    Ok(())
}

const fn base64url_sextet(byte: u8) -> Option<u8> {
    match byte {
        b'A'..=b'Z' => Some(byte - b'A'),
        b'a'..=b'z' => Some(byte - b'a' + 26),
        b'0'..=b'9' => Some(byte - b'0' + 52),
        b'-' => Some(62),
        b'_' => Some(63),
        _ => None,
    }
}

#[derive(Clone)]
pub struct AppState {
    pub config: ServerConfig,
    pub database: Database,
    pub replay_store: ReplayStore,
    pub campaign_store: CampaignStore,
    /// Durable server-local secret used only to authenticate pagination state.
    pub cursor_hmac_key: [u8; 32],
    /// Dedicated authority for compact backup status; never archived in a backup.
    pub backup_authority_hmac_key: [u8; 32],
    /// Dedicated Ed25519 signing seed. Manifests pin its public key.
    pub competition_run_grant_secret_key: Option<[u8; 32]>,
    /// Dedicated Ed25519 seed pinned by every admitted immutable ruleset.
    pub run_preflight_grant_secret_key: Option<[u8; 32]>,
    pub challenge_rate_limiter: ChallengeRateLimiter,
}

#[derive(Clone)]
pub struct ChallengeRateLimiter {
    maximum_per_minute: usize,
    attempts:
        Arc<tokio::sync::Mutex<HashMap<(IpAddr, &'static str), VecDeque<tokio::time::Instant>>>>,
}

impl ChallengeRateLimiter {
    pub fn new(maximum_per_minute: u32) -> Self {
        Self {
            maximum_per_minute: maximum_per_minute as usize,
            attempts: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        }
    }

    async fn check(&self, address: IpAddr, purpose: ChallengePurpose) -> Result<(), ApiError> {
        let now = tokio::time::Instant::now();
        let cutoff = now - Duration::from_secs(60);
        let mut attempts = self.attempts.lock().await;
        if attempts.len() > 100_000 {
            attempts.retain(|_, values| values.back().is_some_and(|last| *last >= cutoff));
        }
        let values = attempts.entry((address, purpose.as_str())).or_default();
        while values.front().is_some_and(|instant| *instant < cutoff) {
            values.pop_front();
        }
        if values.len() >= self.maximum_per_minute {
            let retry_after = values.front().map_or(Duration::from_secs(60), |first| {
                (*first + Duration::from_secs(60)).saturating_duration_since(now)
            });
            return Err(ApiError::RateLimited {
                retry_after_ms: u64::try_from(retry_after.as_millis())
                    .expect("a sixty-second rate limit fits in u64 milliseconds")
                    .max(1),
            });
        }
        values.push_back(now);
        Ok(())
    }
}

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
    schema_version: u32,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CursorToken {
    filter_sha256: Digest32,
    accepted_sequence_watermark: u64,
    visibility_revision: u64,
    metric_value: i64,
    position: u64,
    rank: u64,
    accepted_sequence: i64,
    verified_at_unix_ms: u64,
    run_id: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PlayerHistoryCursorToken {
    player_public_key: PublicKey32,
    query_sha256: Digest32,
    accepted_sequence_watermark: u64,
    visibility_revision: u64,
    accepted_sequence: u64,
    run_id: String,
}

pub fn router(state: AppState) -> Result<Router, ApiError> {
    let body_limit = submission_body_limit(&state.config)?;
    let upload_router = Router::new()
        .route("/api/v1/submissions", post(submit))
        .layer(DefaultBodyLimit::max(body_limit))
        .layer(RequestBodyTimeoutLayer::new(Duration::from_secs(
            state.config.upload_timeout_seconds,
        )))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            maintenance_upload_write_gate,
        ))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            database_fence_gate,
        ))
        .layer(tower::limit::GlobalConcurrencyLimitLayer::new(
            state.config.max_concurrent_uploads,
        ))
        // This response deadline is deliberately outside the owned
        // maintenance task. A client timeout stops waiting for a response but
        // cannot cancel a state mutation or release its durable writer lease.
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            Duration::from_secs(state.config.upload_timeout_seconds),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            CACHE_CONTROL,
            HeaderValue::from_static("no-store"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ));
    // These resources contain upload nonces, opaque lifecycle identifiers,
    // deletion authorization, or moderation receipts. Apply no-store to both
    // success and error responses so browser and intermediary caches cannot
    // persist them.
    let mut sensitive_router = Router::new()
        .route(
            "/api/v1/competition-run-grants",
            post(competition_run_grant),
        )
        .route(
            "/api/v1/fresh-run-preflight-grants",
            post(fresh_run_preflight_grant),
        )
        .route(
            "/api/v1/campaign-continuation-preflight-grants",
            post(campaign_continuation_preflight_grant),
        )
        .route("/api/v1/submission-offers", post(submission_offer))
        .route(
            "/api/v1/submission-owner-status-challenges",
            post(submission_owner_status_challenge),
        )
        .route(
            "/api/v1/submissions/{submission_id}/private-status",
            post(submission_private_status),
        )
        .route("/api/v1/username-challenges", post(username_challenge))
        .route("/api/v1/deletion-challenges", post(deletion_challenge))
        .route("/api/v1/deletion-requests", post(deletion_request))
        .route("/api/v1/reports", post(abuse_report))
        .route(
            "/api/v1/players/{public_key}/username",
            put(update_username),
        );
    if state.config.moderation_bearer_token.is_some() {
        sensitive_router = sensitive_router
            .route("/api/v1/operator/reports", get(operator_reports))
            .route(
                "/api/v1/operator/reports/{report_id}/actions",
                post(operator_report_action),
            )
            .route("/api/v1/operator/moderation-audit", get(operator_audit))
            .route("/api/v1/operator/operational-status", get(operator_status))
            .route("/api/v1/operator/metrics", get(operator_metrics));
    }
    let sensitive_router = sensitive_router
        .layer(DefaultBodyLimit::max(state.config.max_metadata_bytes))
        .layer(RequestBodyTimeoutLayer::new(Duration::from_secs(
            state.config.upload_timeout_seconds,
        )))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            maintenance_sensitive_write_gate,
        ))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            database_fence_gate,
        ))
        .layer(tower::limit::GlobalConcurrencyLimitLayer::new(
            state.config.max_concurrent_requests,
        ))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            Duration::from_secs(state.config.upload_timeout_seconds),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            CACHE_CONTROL,
            HeaderValue::from_static("no-store"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ));
    let standard_router = Router::new()
        .route("/readyz", get(readiness))
        .route("/api/v1/leaderboard-metadata", get(leaderboard_metadata))
        .route("/api/v1/leaderboards", get(leaderboard))
        .route("/api/v1/runs/{run_id}", get(run_detail))
        .route("/api/v1/runs/{run_id}/replay", get(run_replay))
        .route(
            "/api/v1/runs/{run_id}/campaigns/{campaign_role}",
            get(run_public_campaign),
        )
        .route(
            "/api/v1/runs/{run_id}/sessions/{ordinal}",
            get(campaign_session_detail),
        )
        .route(
            "/api/v1/runs/{run_id}/sessions/{ordinal}/replay",
            get(campaign_session_replay),
        )
        .route(
            "/api/v1/runs/{run_id}/sessions/{ordinal}/campaigns/{campaign_role}",
            get(campaign_session_public_campaign),
        )
        .route("/api/v1/builds/{digest}", get(build_manifest))
        .route("/api/v1/content-manifests/{digest}", get(content_manifest))
        .route(
            "/api/v1/campaign-content-manifests/{digest}",
            get(campaign_content_manifest),
        )
        .route("/api/v1/rules-configs/{digest}", get(rules_config))
        .route("/api/v1/ruleset-manifests/{digest}", get(ruleset_manifest))
        .route(
            "/api/v1/published-rulesets/{digest}",
            get(published_ruleset),
        )
        .route(
            "/api/v1/competitions/{digest}",
            get(competition_manifest_route),
        )
        .route("/api/v1/policies/{digest}", get(policy_manifest))
        .route("/api/v1/players/{public_key}", get(player_profile))
        .route("/api/v1/players/{public_key}/runs", get(player_run_history))
        .layer(DefaultBodyLimit::max(state.config.max_metadata_bytes))
        .layer(RequestBodyTimeoutLayer::new(Duration::from_secs(
            state.config.upload_timeout_seconds,
        )))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            database_fence_gate,
        ))
        .layer(tower::limit::GlobalConcurrencyLimitLayer::new(
            state.config.max_concurrent_requests,
        ))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            Duration::from_secs(state.config.upload_timeout_seconds),
        ))
        // Dynamic public JSON and binary responses can change after a rename,
        // deletion, or ruleset quarantine. Immutable manifest handlers set a
        // stronger content-addressed policy before these defaults run.
        .layer(SetResponseHeaderLayer::if_not_present(
            CACHE_CONTROL,
            HeaderValue::from_static("no-store"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ));
    let database_router = standard_router.merge(sensitive_router).merge(upload_router);
    let mut router = Router::new()
        .route("/healthz", get(health))
        .merge(database_router)
        .layer(PropagateRequestIdLayer::x_request_id())
        .layer(SetRequestIdLayer::x_request_id(MakeRequestUuid))
        // DefaultMakeSpan records the complete URI, which would put player
        // public keys, submission IDs, and authenticated cursor tokens into
        // access logs. Status and latency remain available through the
        // default response callbacks under this deliberately path-free span.
        .layer(
            TraceLayer::new_for_http().make_span_with(|request: &Request<Body>| {
                tracing::info_span!(
                    "http_request",
                    method = %request.method(),
                    version = ?request.version(),
                )
            }),
        )
        .with_state(state.clone());

    if !state.config.allowed_origins.is_empty() {
        let origins = state
            .config
            .allowed_origins
            .iter()
            .map(|origin| {
                HeaderValue::from_str(origin)
                    .map_err(|_| ApiError::BadRequest(format!("invalid CORS origin: {origin}")))
            })
            .collect::<Result<Vec<_>, _>>()?;
        router = router.layer(
            CorsLayer::new()
                .allow_origin(origins)
                .allow_methods([Method::GET, Method::POST, Method::PUT])
                .allow_headers([CONTENT_TYPE]),
        );
    }
    Ok(router)
}

const API_WRITE_LEASE_TTL: Duration = Duration::from_secs(5 * 60);

async fn database_fence_gate(
    State(state): State<AppState>,
    request: Request<Body>,
    next: Next,
) -> Result<Response, ApiError> {
    let database = state.database;
    // Own the complete handler and fence token independently of the response
    // waiter. Client cancellation or an outer response timeout detaches this
    // task; it cannot release the kernel fence while SQLx rollback/return/ping
    // or handler-spawned filesystem publication is still completing.
    let owner = tokio::spawn(async move {
        let mut fence = database.begin_fenced_operation().await?;
        let operation = tokio::spawn(async move { next.run(request).await });
        let result = operation
            .await
            .map_err(|error| anyhow::anyhow!(error).context("database-backed API handler failed"));
        let finish = database.finish_fenced_operation(&mut fence).await;
        match (result, finish) {
            (Ok(response), Ok(())) => Ok(response),
            (Ok(_), Err(error)) => Err(error.into()),
            (Err(operation), Ok(())) => Err(operation),
            (Err(operation), Err(finish)) => Err(operation.context(format!(
                "database-backed API handler failed and fence drain also failed: {finish}"
            ))),
        }
    });
    match owner.await {
        Ok(Ok(response)) => Ok(response),
        Ok(Err(error)) => {
            tracing::warn!(
                error_code = crate::safe_error_code(&error),
                "database access is quiesced or failed"
            );
            Err(ApiError::Unavailable)
        }
        Err(error) => {
            tracing::error!(
                error_code = "database_fence_owner",
                task_panicked = error.is_panic(),
                task_cancelled = error.is_cancelled(),
                "database fence-owner task failed"
            );
            Err(ApiError::Unavailable)
        }
    }
}

async fn run_owned_maintenance_write<T, F>(
    database: Database,
    writer_class: crate::db::MaintenanceWriteClass,
    operation: F,
) -> anyhow::Result<T>
where
    T: Send + 'static,
    F: std::future::Future<Output = T> + Send + 'static,
{
    let owner = tokio::spawn(async move {
        // `database_fence_gate` is inside the route timeout and owns/detaches
        // the entire `next.run` future, so it already retains SH through this
        // helper and its owned task. Re-entering admission here would invert
        // lock order if backup acquired EX admission while the outer SH was
        // held. Acquire the durable lease before polling the handler instead.
        let lease = database
            .acquire_maintenance_write_lease(
                writer_class,
                "robin-highscores-api",
                API_WRITE_LEASE_TTL,
            )
            .await?;
        // A panic becomes a JoinError observed by this lease owner;
        // the lease and nested fence remain until the task is gone.
        let mut operation = tokio::spawn(operation);
        let refresh_every = API_WRITE_LEASE_TTL
            .checked_div(3)
            .filter(|interval| !interval.is_zero())
            .expect("maintenance-write lease TTL has a positive refresh interval");
        let operation_result: anyhow::Result<T> = loop {
            tokio::select! {
                response = &mut operation => {
                    break response.map_err(|error| {
                        anyhow::anyhow!(error)
                            .context("state-mutating API handler task failed")
                    });
                }
                () = tokio::time::sleep(refresh_every) => {
                    let refresh_error = match database
                        .refresh_maintenance_write_lease(&lease, API_WRITE_LEASE_TTL)
                        .await
                    {
                        Ok(true) => continue,
                        Ok(false) => anyhow::anyhow!(
                            "API maintenance-write lease expired or was replaced"
                        ),
                        Err(error) => anyhow::Error::from(error),
                    };
                    // Never release a lease while detached SQL/file
                    // work may still complete. Even after losing
                    // refresh authority, wait for terminal completion.
                    let _ = (&mut operation).await;
                    break Err(refresh_error);
                }
            }
        };
        let release = database.release_maintenance_write_lease(&lease).await;
        match (operation_result, release) {
                    (Ok(value), Ok(true)) => Ok(value),
                    (Ok(_), Ok(false)) => {
                        anyhow::bail!("API maintenance-write lease disappeared")
                    }
                    (Ok(_), Err(error)) => Err(error.into()),
                    (Err(operation), Ok(true)) => Err(operation),
                    (Err(operation), Ok(false)) => Err(operation.context(
                        "API operation failed and its maintenance-write lease disappeared",
                    )),
                    (Err(operation), Err(release)) => Err(operation.context(format!(
                        "API operation failed and releasing its maintenance-write lease also failed: {release}"
                    ))),
                }
    });
    owner
        .await
        .map_err(|error| anyhow::anyhow!(error).context("API lease-owner task failed"))?
}

async fn maintenance_upload_write_gate(
    State(state): State<AppState>,
    request: Request<Body>,
    next: Next,
) -> Result<Response, ApiError> {
    maintenance_write_gate(
        state,
        crate::db::MaintenanceWriteClass::ApiUpload,
        request,
        next,
    )
    .await
}

async fn maintenance_sensitive_write_gate(
    State(state): State<AppState>,
    request: Request<Body>,
    next: Next,
) -> Result<Response, ApiError> {
    maintenance_write_gate(
        state,
        crate::db::MaintenanceWriteClass::ApiSensitive,
        request,
        next,
    )
    .await
}

async fn maintenance_write_gate(
    state: AppState,
    writer_class: crate::db::MaintenanceWriteClass,
    request: Request<Body>,
    next: Next,
) -> Result<Response, ApiError> {
    if matches!(
        *request.method(),
        Method::GET | Method::HEAD | Method::OPTIONS
    ) {
        return Ok(next.run(request).await);
    }
    // The owned task is intentionally detached from the response future. A
    // client disconnect or the outer response deadline may stop awaiting this
    // JoinHandle, but Tokio continues the mutation and its lease heartbeat to
    // actual completion. The nested task lets us observe handler panics and
    // still release the lease only after its future is definitely gone.
    run_owned_maintenance_write(state.database, writer_class, async move {
        next.run(request).await
    })
    .await
    .map_err(|error| {
        tracing::error!(
            error_code = crate::safe_error_code(&error),
            "state-mutating API operation failed"
        );
        ApiError::Unavailable
    })
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ModerationListQuery {
    state: Option<String>,
    #[serde(default = "default_moderation_limit")]
    limit: u32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ModerationAuditQuery {
    report_id: Option<String>,
    #[serde(default = "default_moderation_limit")]
    limit: u32,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ModerationActionRequest {
    state: String,
    detail: String,
}

fn default_moderation_limit() -> u32 {
    100
}

fn authorize_operator(state: &AppState, headers: &HeaderMap) -> Result<(), ApiError> {
    use subtle::ConstantTimeEq as _;
    let expected = state
        .config
        .moderation_bearer_token
        .as_deref()
        .ok_or(ApiError::NotFound)?;
    let supplied = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.as_bytes().strip_prefix(b"Bearer "))
        .ok_or(ApiError::Unauthorized)?;
    // Compare fixed-size digests so neither an early byte mismatch nor the
    // configured token length affects the secret comparison path.
    let expected_digest = sha2::Sha256::digest(expected.as_slice());
    let supplied_digest = sha2::Sha256::digest(supplied);
    if expected_digest
        .as_slice()
        .ct_eq(supplied_digest.as_slice())
        .into()
    {
        Ok(())
    } else {
        Err(ApiError::Unauthorized)
    }
}

async fn operator_reports(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<ModerationListQuery>,
) -> Result<Json<Vec<crate::db::ModerationReportRecord>>, ApiError> {
    authorize_operator(&state, &headers)?;
    Ok(Json(
        state
            .database
            .moderation_reports(query.state.as_deref(), query.limit)
            .await?,
    ))
}

async fn operator_report_action(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(report_id): Path<String>,
    Json(action): Json<ModerationActionRequest>,
) -> Result<StatusCode, ApiError> {
    authorize_operator(&state, &headers)?;
    state
        .database
        .moderate_report(
            &report_id,
            &action.state,
            &action.detail,
            &state.config.moderation_operator_id,
        )
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn operator_audit(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<ModerationAuditQuery>,
) -> Result<Json<Vec<crate::db::ModerationAuditRecord>>, ApiError> {
    authorize_operator(&state, &headers)?;
    Ok(Json(
        state
            .database
            .moderation_audit(query.report_id.as_deref(), query.limit)
            .await?,
    ))
}

async fn health() -> Result<Json<HealthResponse>, ApiError> {
    Ok(Json(HealthResponse {
        status: "ok",
        schema_version: SCHEMA_VERSION_V1,
    }))
}

async fn readiness(State(state): State<AppState>) -> Result<Json<HealthResponse>, ApiError> {
    state.database.health_check().await?;
    // GET readiness is strictly observational. Writable create/fsync/remove
    // probes run during startup before systemd READY and on mutating admission
    // paths, which are protected by the maintenance writer lease.
    ensure_offer_capacity(
        &state.config,
        &state.database,
        &state.replay_store,
        &state.campaign_store,
    )
    .map_err(storage_admission_error)?;
    ensure_backup_ready(&state).await?;
    Ok(Json(HealthResponse {
        status: "ready",
        schema_version: SCHEMA_VERSION_V1,
    }))
}

#[derive(Debug, Serialize)]
struct OperationalStatusResponse {
    schema_version: u32,
    database: crate::db::OperationalCounts,
    replay_storage_free_bytes: u64,
    campaign_storage_free_bytes: u64,
    backup_age_ms: Option<u64>,
}

async fn operator_status(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<OperationalStatusResponse>, ApiError> {
    authorize_operator(&state, &headers)?;
    let replay_storage_free_bytes = state
        .replay_store
        .storage_volume()
        .map(|volume| volume.available_bytes())
        .map_err(|error| {
            tracing::error!(%error, "could not measure replay storage capacity");
            ApiError::Unavailable
        })?;
    let campaign_storage_free_bytes = state
        .campaign_store
        .storage_volume()
        .map(|volume| volume.available_bytes())
        .map_err(|error| {
            tracing::error!(%error, "could not measure campaign storage capacity");
            ApiError::Unavailable
        })?;
    let backup_age_ms = backup_age_ms(&state).await?;
    Ok(Json(OperationalStatusResponse {
        schema_version: SCHEMA_VERSION_V1,
        database: state.database.operational_counts().await?,
        replay_storage_free_bytes,
        campaign_storage_free_bytes,
        backup_age_ms,
    }))
}

async fn operator_metrics(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    authorize_operator(&state, &headers)?;
    let status = operator_status(State(state), headers).await?.0;
    let counts = status.database;
    let backup_age = status.backup_age_ms.unwrap_or(0);
    let body = format!(
        "# TYPE robin_highscores_submissions gauge\nrobin_highscores_submissions{{state=\"queued\"}} {}\nrobin_highscores_submissions{{state=\"rejected_retained\"}} {}\n# TYPE robin_highscores_upload_reservations gauge\nrobin_highscores_upload_reservations{{state=\"active\"}} {}\nrobin_highscores_upload_reservations{{state=\"abandoned\"}} {}\n# TYPE robin_highscores_accepted_runs gauge\nrobin_highscores_accepted_runs {}\n# TYPE robin_highscores_open_abuse_reports gauge\nrobin_highscores_open_abuse_reports {}\n# TYPE robin_highscores_replay_objects gauge\nrobin_highscores_replay_objects{{state=\"live\"}} {}\nrobin_highscores_replay_objects{{state=\"purging\"}} {}\n# TYPE robin_highscores_campaign_objects gauge\nrobin_highscores_campaign_objects{{state=\"live\"}} {}\nrobin_highscores_campaign_objects{{state=\"purging\"}} {}\n# TYPE robin_highscores_replay_storage_free_bytes gauge\nrobin_highscores_replay_storage_free_bytes {}\n# TYPE robin_highscores_campaign_storage_free_bytes gauge\nrobin_highscores_campaign_storage_free_bytes {}\n# TYPE robin_highscores_backup_age_milliseconds gauge\nrobin_highscores_backup_age_milliseconds {}\n",
        counts.queued_submissions,
        counts.rejected_retained_submissions,
        counts.active_upload_reservations,
        counts.abandoned_upload_reservations,
        counts.accepted_runs,
        counts.open_abuse_reports,
        counts.replay_objects_live,
        counts.replay_objects_purging,
        counts.campaign_objects_live,
        counts.campaign_objects_purging,
        status.replay_storage_free_bytes,
        status.campaign_storage_free_bytes,
        backup_age,
    );
    let mut response = Body::from(body).into_response();
    response.headers_mut().insert(
        CONTENT_TYPE,
        HeaderValue::from_static("text/plain; version=0.0.4; charset=utf-8"),
    );
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(response)
}

async fn ensure_offer_admission_ready(state: &AppState) -> Result<(), ApiError> {
    state
        .replay_store
        .readiness_check()
        .await
        .map_err(|error| {
            tracing::error!(
                error_code = error.safe_log_code(),
                "replay store admission check failed"
            );
            ApiError::Unavailable
        })?;
    state
        .campaign_store
        .readiness_check()
        .await
        .map_err(|error| {
            tracing::error!(
                error_code = error.safe_log_code(),
                "campaign store admission check failed"
            );
            ApiError::Unavailable
        })?;
    ensure_offer_capacity(
        &state.config,
        &state.database,
        &state.replay_store,
        &state.campaign_store,
    )
    .map_err(storage_admission_error)?;
    ensure_backup_ready(state).await
}

async fn ensure_upload_admission_ready(
    state: &AppState,
    replay_bytes: u64,
    campaign_bytes: u64,
) -> Result<(), ApiError> {
    state
        .replay_store
        .readiness_check()
        .await
        .map_err(|error| {
            tracing::error!(
                error_code = error.safe_log_code(),
                "replay store admission check failed"
            );
            ApiError::Unavailable
        })?;
    state
        .campaign_store
        .readiness_check()
        .await
        .map_err(|error| {
            tracing::error!(
                error_code = error.safe_log_code(),
                "campaign store admission check failed"
            );
            ApiError::Unavailable
        })?;
    ensure_upload_capacity(
        &state.config,
        &state.database,
        &state.replay_store,
        &state.campaign_store,
        replay_bytes,
        campaign_bytes,
    )
    .map_err(storage_admission_error)?;
    ensure_backup_ready(state).await
}

fn storage_admission_error(error: StorageAdmissionError) -> ApiError {
    let safe_code = error.safe_log_code();
    match error {
        StorageAdmissionError::ArtifactTooLarge { .. } => {
            ApiError::PayloadTooLarge(error.to_string())
        }
        _ => {
            tracing::error!(
                error_code = safe_code,
                "storage capacity admission check failed"
            );
            ApiError::Unavailable
        }
    }
}

async fn leaderboard_metadata(
    State(state): State<AppState>,
) -> Result<Json<LeaderboardMetadataV1>, ApiError> {
    let mut missions = Vec::new();
    let mut mission_ids = BTreeSet::new();
    let mut rulesets = Vec::new();
    let mut active_profile_ids = BTreeSet::new();
    for profile in &state.config.admission_profiles {
        let ruleset_digest = digest(&profile.ruleset_id)?;
        let published = state
            .config
            .manifests
            .rulesets
            .get(&ruleset_digest)
            .ok_or(ApiError::Internal)?;
        if !matches!(
            published.operational_status,
            RulesetOperationalStatusV1::Active
        ) {
            continue;
        }
        active_profile_ids.insert(profile.id.as_str());
        if mission_ids.insert(profile.mission_id()) {
            missions.push(MissionFacetV1 {
                mission_id: profile.mission_id().to_owned(),
                display_name: profile.mission_display_name.clone(),
                content_manifest_sha256: digest(&profile.content_manifest_id)?,
            });
        }
        rulesets.push(ruleset_facet(profile)?);
        if profile
            .allowed_scopes
            .iter()
            .any(|scope| scope == "campaign_genesis")
            && published
                .manifest
                .board_scopes
                .contains(&RulesetBoardScopeV1::FullCampaign)
        {
            let mut campaign = ruleset_facet(profile)?;
            campaign.content = RunContentIdentityV1::FullCampaign {
                campaign_content_manifest_sha256: digest(
                    profile
                        .campaign_content_manifest_id
                        .as_deref()
                        .ok_or(ApiError::Internal)?,
                )?,
            };
            campaign.categories = vec![BoardCategoryV1::Campaign];
            campaign.supports_full_campaign_boards = true;
            rulesets.push(campaign);
        }
    }
    missions.sort_by(|left, right| left.mission_id.cmp(&right.mission_id));
    rulesets.sort_by_key(RulesetFacetV1::identity);
    rulesets.dedup_by(|later, earlier| {
        if later.identity() != earlier.identity() {
            return false;
        }
        earlier.categories.extend(later.categories.iter().copied());
        earlier.categories.sort();
        earlier.categories.dedup();
        true
    });
    let now = crate::model::now_unix_ms()?;
    let mut competitions = state
        .config
        .competitions
        .iter()
        .filter(|competition| {
            active_profile_ids.contains(competition.admission_profile_id.as_str())
        })
        .map(|competition| competition_summary(&state.config, competition, now))
        .collect::<Result<Vec<_>, _>>()?;
    competitions.sort_by_key(|competition| competition.competition_manifest_sha256);
    let supports_full_campaign = rulesets
        .iter()
        .any(|ruleset| ruleset.supports_full_campaign_boards);
    let metadata = LeaderboardMetadataV1 {
        schema_version: SCHEMA_VERSION_V1,
        missions,
        rulesets,
        competitions,
        full_campaign: supports_full_campaign.then(|| FullCampaignFacetV1 {
            display_name: "Full Campaign".to_owned(),
            description:
                "Complete, gap-free verified campaign chains including headquarters sessions."
                    .to_owned(),
        }),
    };
    metadata
        .validate()
        .map_err(|error| configuration_error("leaderboard metadata", error))?;
    Ok(Json(metadata))
}

/// Admit one exact canonical fresh run before simulation begins. The host
/// signs the immutable engine/session tuple; the service contributes fresh
/// entropy and authoritative interval timestamps, then signs the complete
/// authorization.
async fn fresh_run_preflight_grant(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<FreshRunPreflightRequestV1>,
) -> Result<(StatusCode, Json<FreshRunPreflightGrantV1>), ApiError> {
    rate_limit_challenge(&state, peer, &headers, ChallengePurpose::Submission).await?;
    request.validate()?;
    verify_request_signature(
        request.claim.host_public_key.as_bytes(),
        request.host_signature.as_bytes(),
        &request.signing_bytes()?,
    )?;
    if !state
        .database
        .identity_exists(request.claim.host_public_key.as_bytes())
        .await?
    {
        return Err(ApiError::BadRequest(
            "host must register a username before requesting fresh-run preflight".to_owned(),
        ));
    }
    if state
        .database
        .replay_session_identity_used(
            request.claim.host_public_key.as_bytes(),
            request.claim.replay_session_id.as_bytes(),
            request.claim.host_nonce.as_bytes(),
        )
        .await?
    {
        return Err(ApiError::Conflict(
            "ranked replay-session identity was already used".to_owned(),
        ));
    }

    let profile = select_fresh_run_profile(&state.config, &request)?;
    let derived_profile =
        profile_with_session_config(&state.config, profile, &request.claim.ranked_session)?;
    let profile = &derived_profile;
    validate_fresh_run_preflight_profile(&state.config, &request, profile)?;
    let published = state
        .config
        .manifests
        .rulesets
        .get(&request.claim.ranked_session.ruleset_manifest_sha256)
        .ok_or(ApiError::Internal)?;
    let signing_key = ed25519_dalek::SigningKey::from_bytes(
        &state
            .run_preflight_grant_secret_key
            .ok_or(ApiError::Internal)?,
    );
    let authority_public_key = PublicKey32::from_bytes(signing_key.verifying_key().to_bytes());
    if authority_public_key != published.manifest.run_preflight_grant_public_key {
        return Err(ApiError::Internal);
    }

    let admitted_at_unix_ms = crate::model::now_unix_ms()?;
    let expires_at_unix_ms = admitted_at_unix_ms
        .checked_add(
            state
                .config
                .run_preflight_ttl_seconds
                .checked_mul(1_000)
                .ok_or(ApiError::Internal)?,
        )
        .ok_or(ApiError::Internal)?;
    let claim = FreshRunPreflightGrantClaimV1 {
        schema_version: SCHEMA_VERSION_V1,
        grant_id: opaque(&uuid::Uuid::now_v7().to_string())?,
        grant_nonce: ChallengeNonce32::from_bytes(rand::random()),
        grant_authority_public_key: authority_public_key,
        host_public_key: request.claim.host_public_key,
        grant_request_sha256: request
            .canonical_digest()
            .map_err(|error| ApiError::BadRequest(error.to_string()))?,
        ranked_session_sha256: request
            .claim
            .ranked_session
            .canonical_digest()
            .map_err(|error| ApiError::BadRequest(error.to_string()))?,
        replay_session_id: request.claim.replay_session_id,
        host_participant_instance_id: request.claim.host_participant_instance_id,
        host_nonce: request.claim.host_nonce,
        scope: request.claim.scope,
        starting_campaign: request.claim.starting_campaign.clone(),
        admitted_at_unix_ms,
        expires_at_unix_ms,
    };
    let authority_signature = signing_key.sign(&claim.signing_bytes()?);
    let grant = FreshRunPreflightGrantV1 {
        claim,
        algorithm: SignatureAlgorithmV1::Ed25519,
        authority_signature: Signature64::from_bytes(authority_signature.to_bytes()),
    };
    if let Err(error) = grant.validate_request(&request) {
        tracing::error!(%error, "fresh-run grant construction violated its request binding");
        return Err(ApiError::Internal);
    }
    Ok((StatusCode::CREATED, Json(grant)))
}

/// Admit a continuation before frame zero without requiring every peer to own
/// the controller's local receipt. Both the new host and immutable campaign
/// controller sign the exact request; the service resolves the active
/// predecessor from SQLite and signs the resulting authority grant.
async fn campaign_continuation_preflight_grant(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<CampaignContinuationPreflightRequestV1>,
) -> Result<(StatusCode, Json<CampaignContinuationPreflightGrantV1>), ApiError> {
    rate_limit_challenge(&state, peer, &headers, ChallengePurpose::Submission).await?;
    request.validate()?;
    verify_request_signature(
        request.claim.host_public_key.as_bytes(),
        request.host_signature.as_bytes(),
        &request.claim.host_signing_bytes()?,
    )?;
    verify_request_signature(
        request.claim.campaign_controller_public_key.as_bytes(),
        request.controller_signature.as_bytes(),
        &request.claim.controller_signing_bytes()?,
    )?;
    for public_key in &request.claim.participant_public_keys {
        if !state
            .database
            .identity_exists(public_key.as_bytes())
            .await?
        {
            return Err(ApiError::BadRequest(
                "every continuation participant must register a username before preflight"
                    .to_owned(),
            ));
        }
    }
    if state
        .database
        .replay_session_identity_used(
            request.claim.host_public_key.as_bytes(),
            request.claim.replay_session_id.as_bytes(),
            request.claim.host_nonce.as_bytes(),
        )
        .await?
    {
        return Err(ApiError::Conflict(
            "ranked replay-session identity was already used".to_owned(),
        ));
    }

    let predecessor = state
        .database
        .campaign_predecessor(request.claim.predecessor_run_id.as_str())
        .await?;
    let profile = select_continuation_preflight_profile(&state.config, &request)?;
    let derived_profile =
        profile_with_session_config(&state.config, profile, &request.claim.ranked_session)?;
    let profile = &derived_profile;
    let published = state
        .config
        .manifests
        .rulesets
        .get(&request.claim.ranked_session.ruleset_manifest_sha256)
        .ok_or(ApiError::Internal)?;
    let mut predecessor_keys = predecessor
        .participants
        .iter()
        .map(|participant| PublicKey32::from_bytes(participant.public_key))
        .collect::<Vec<_>>();
    predecessor_keys.sort_unstable();
    predecessor_keys.dedup();
    let roster_matches = match published.manifest.campaign_roster_continuity {
        CampaignRosterContinuityV1::ExactSameAuthenticatedKeysEverySession => {
            predecessor_keys == request.claim.participant_public_keys
        }
        CampaignRosterContinuityV1::UnionOfVerifiedSessionSubsets => true,
    };
    if predecessor.chain_id != request.claim.chain_id.as_str()
        || predecessor.run_id != request.claim.predecessor_run_id.as_str()
        || predecessor.result_sha256 != request.claim.predecessor_verification_sha256.into_bytes()
        || predecessor.chain_owner_public_key
            != request.claim.campaign_controller_public_key.into_bytes()
        || predecessor.final_campaign_sha256 != request.claim.starting_campaign.sha256.into_bytes()
        || predecessor.final_campaign_bytes != request.claim.starting_campaign.byte_length
        || predecessor.max_concurrent_players != request.claim.max_concurrent_players
        || !roster_matches
    {
        return Err(ApiError::Conflict(
            "campaign continuation preflight does not match the active predecessor".to_owned(),
        ));
    }
    validate_continuation_preflight_profile(&state.config, &request, profile, &predecessor)?;

    let signing_key = ed25519_dalek::SigningKey::from_bytes(
        &state
            .run_preflight_grant_secret_key
            .ok_or(ApiError::Internal)?,
    );
    let authority_public_key = PublicKey32::from_bytes(signing_key.verifying_key().to_bytes());
    if authority_public_key != published.manifest.run_preflight_grant_public_key {
        return Err(ApiError::Internal);
    }
    let admitted_at_unix_ms = crate::model::now_unix_ms()?;
    let expires_at_unix_ms = admitted_at_unix_ms
        .checked_add(
            state
                .config
                .run_preflight_ttl_seconds
                .checked_mul(1_000)
                .ok_or(ApiError::Internal)?,
        )
        .ok_or(ApiError::Internal)?;
    let claim = CampaignContinuationPreflightGrantClaimV1 {
        schema_version: SCHEMA_VERSION_V1,
        grant_id: opaque(&uuid::Uuid::now_v7().to_string())?,
        grant_nonce: ChallengeNonce32::from_bytes(rand::random()),
        grant_authority_public_key: authority_public_key,
        grant_request_sha256: request
            .canonical_digest()
            .map_err(|error| ApiError::BadRequest(error.to_string()))?,
        ranked_session_sha256: request
            .claim
            .ranked_session
            .canonical_digest()
            .map_err(|error| ApiError::BadRequest(error.to_string()))?,
        host_public_key: request.claim.host_public_key,
        campaign_controller_public_key: request.claim.campaign_controller_public_key,
        replay_session_id: request.claim.replay_session_id,
        host_participant_instance_id: request.claim.host_participant_instance_id,
        host_nonce: request.claim.host_nonce,
        max_concurrent_players: request.claim.max_concurrent_players,
        participant_public_keys: request.claim.participant_public_keys.clone(),
        chain_id: request.claim.chain_id.clone(),
        predecessor_run_id: request.claim.predecessor_run_id.clone(),
        predecessor_verification_sha256: request.claim.predecessor_verification_sha256,
        starting_campaign: request.claim.starting_campaign.clone(),
        admitted_at_unix_ms,
        expires_at_unix_ms,
    };
    let authority_signature = signing_key.sign(&claim.signing_bytes()?);
    let grant = CampaignContinuationPreflightGrantV1 {
        claim,
        algorithm: SignatureAlgorithmV1::Ed25519,
        authority_signature: Signature64::from_bytes(authority_signature.to_bytes()),
    };
    if let Err(error) = grant.validate_request(&request) {
        tracing::error!(%error, "continuation preflight grant violated request binding");
        return Err(ApiError::Internal);
    }
    Ok((StatusCode::CREATED, Json(grant)))
}

async fn competition_run_grant(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<CompetitionRunGrantRequestV1>,
) -> Result<(StatusCode, Json<CompetitionRunGrantV1>), ApiError> {
    rate_limit_challenge(&state, peer, &headers, ChallengePurpose::Submission).await?;
    request.validate()?;
    verify_request_signature(
        request.claim.host_public_key.as_bytes(),
        request.host_signature.as_bytes(),
        &request.signing_bytes()?,
    )?;
    if !state
        .database
        .identity_exists(request.claim.host_public_key.as_bytes())
        .await?
    {
        return Err(ApiError::BadRequest(
            "host must register a username before requesting a competition run grant".to_owned(),
        ));
    }
    let competition_sha256 = request
        .claim
        .ranked_session
        .competition_manifest_sha256
        .expect("validated competition grant request has a competition");
    let (competition_config, competition) =
        competition_by_digest(&state.config, competition_sha256)?;
    competition
        .validate_ranked_session(&request.claim.ranked_session)
        .map_err(|error| ApiError::Conflict(error.to_string()))?;
    let profile = state
        .config
        .admission_profiles
        .iter()
        .find(|profile| profile.id == competition_config.admission_profile_id)
        .ok_or(ApiError::Internal)?;
    let ranked = &request.claim.ranked_session;
    let expected_campaign_content = match competition.content {
        RunContentIdentityV1::Mission { .. } => None,
        RunContentIdentityV1::FullCampaign { .. } => profile
            .campaign_content_manifest_id
            .as_deref()
            .map(digest)
            .transpose()?,
    };
    if ranked.build_manifest_sha256 != digest(&profile.build_manifest_id)?
        || ranked.content_manifest_sha256 != digest(&profile.content_manifest_id)?
        || ranked.campaign_content_manifest_sha256 != expected_campaign_content
        || ranked.rules_config_sha256 != digest(&profile.config_id)?
        || ranked.ruleset_manifest_sha256 != digest(&profile.ruleset_id)?
        || ranked.mission_id != profile.mission_id()
    {
        return Err(ApiError::Conflict(
            "competition run request does not match its admission profile".to_owned(),
        ));
    }
    let signing_key = ed25519_dalek::SigningKey::from_bytes(
        &state
            .competition_run_grant_secret_key
            .ok_or(ApiError::Internal)?,
    );
    let authority_public_key = PublicKey32::from_bytes(signing_key.verifying_key().to_bytes());
    if authority_public_key != competition.competition_run_grant_public_key {
        return Err(ApiError::Internal);
    }
    let request_sha256 = request
        .canonical_digest()
        .map_err(|error| ApiError::BadRequest(error.to_string()))?;
    let ranked_session_sha256 = ranked
        .canonical_digest()
        .map_err(|error| ApiError::BadRequest(error.to_string()))?;
    let host_public_key = request.claim.host_public_key;
    let replay_session_id = request.claim.replay_session_id;
    let host_participant_instance_id = request.claim.host_participant_instance_id;
    let host_nonce = request.claim.host_nonce;
    let grant = state
        .database
        .issue_competition_run_grant(
            &request,
            competition.starts_at_unix_ms,
            competition.ends_at_unix_ms,
            move |issued| {
                let claim = CompetitionRunGrantClaimV1 {
                    schema_version: SCHEMA_VERSION_V1,
                    grant_id: OpaqueId::new(issued.id.clone())
                        .map_err(|error| crate::db::DbError::ResultInvariant(error.to_string()))?,
                    grant_nonce: ChallengeNonce32::from_bytes(issued.nonce),
                    grant_authority_public_key: authority_public_key,
                    host_public_key,
                    competition_manifest_sha256: competition_sha256,
                    ranked_session_sha256,
                    grant_request_sha256: request_sha256,
                    replay_session_id,
                    host_participant_instance_id,
                    host_nonce,
                    admitted_at_unix_ms: issued.admitted_at_ms,
                    expires_at_unix_ms: issued.expires_at_ms,
                };
                let signature = signing_key.sign(
                    &claim
                        .signing_bytes()
                        .map_err(|error| crate::db::DbError::ResultInvariant(error.to_string()))?,
                );
                Ok(CompetitionRunGrantV1 {
                    claim,
                    algorithm: SignatureAlgorithmV1::Ed25519,
                    authority_signature: Signature64::from_bytes(signature.to_bytes()),
                })
            },
        )
        .await?;
    competition
        .validate_run_grant(&grant)
        .map_err(|error| ApiError::Conflict(error.to_string()))?;
    Ok((StatusCode::CREATED, Json(grant)))
}

async fn submission_offer(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<SubmissionOfferRequestV1>,
) -> Result<(StatusCode, Json<SubmissionOfferV1>), ApiError> {
    rate_limit_challenge(&state, peer, &headers, ChallengePurpose::Submission).await?;
    request.validate()?;
    if usize::from(request.participant_instance_count) != request.participant_claims.len() {
        return Err(ApiError::BadRequest(
            "every ranked participant instance must have a distinct authenticated key and attestation"
                .to_owned(),
        ));
    }
    verify_session_attestations(&request)?;
    let session_genesis_sha256 = request
        .session_genesis
        .canonical_digest()
        .map_err(|error| ApiError::BadRequest(error.to_string()))?;
    if state
        .database
        .replay_session_genesis_used(
            request.session_genesis.claim.host_public_key.as_bytes(),
            request.session_genesis.claim.replay_session_id.as_bytes(),
            request.session_genesis.claim.host_nonce.as_bytes(),
            session_genesis_sha256.as_bytes(),
        )
        .await?
    {
        return Err(ApiError::Conflict(
            "ranked replay-session genesis was already used".to_owned(),
        ));
    }
    for claim in &request.participant_claims {
        if !state
            .database
            .identity_exists(claim.public_key.as_bytes())
            .await?
        {
            return Err(ApiError::BadRequest(format!(
                "participant seat {} must register a username before requesting an offer",
                claim.seat
            )));
        }
    }
    let scope_name = scope_request_name(&request.scope_request);
    let profile = select_profile(&state.config, &request, scope_name)?;
    let derived_profile = profile_with_session_config(
        &state.config,
        profile,
        &request.session_genesis.claim.ranked_session,
    )?;
    let profile = &derived_profile;
    if let Some(grant) = &request.session_genesis.claim.fresh_run_preflight_grant {
        let published = state
            .config
            .manifests
            .rulesets
            .get(&request.ruleset_manifest_sha256)
            .ok_or(ApiError::Internal)?;
        if grant.claim.grant_authority_public_key
            != published.manifest.run_preflight_grant_public_key
        {
            return Err(ApiError::Conflict(
                "fresh-run preflight authority does not match the current ruleset".to_owned(),
            ));
        }
        verify_request_signature(
            grant.claim.grant_authority_public_key.as_bytes(),
            grant.authority_signature.as_bytes(),
            &grant.signing_bytes()?,
        )?;
        let now = crate::model::now_unix_ms()?;
        if now < grant.claim.admitted_at_unix_ms || now > grant.claim.expires_at_unix_ms {
            return Err(ApiError::Conflict(
                "fresh-run preflight grant is not active under server time".to_owned(),
            ));
        }
    }
    if let Some(grant) = &request
        .session_genesis
        .claim
        .campaign_continuation_preflight_grant
    {
        let published = state
            .config
            .manifests
            .rulesets
            .get(&request.ruleset_manifest_sha256)
            .ok_or(ApiError::Internal)?;
        if grant.claim.grant_authority_public_key
            != published.manifest.run_preflight_grant_public_key
        {
            return Err(ApiError::Conflict(
                "continuation preflight authority does not match the current ruleset".to_owned(),
            ));
        }
        verify_request_signature(
            grant.claim.grant_authority_public_key.as_bytes(),
            grant.authority_signature.as_bytes(),
            &grant.signing_bytes()?,
        )?;
        let now = crate::model::now_unix_ms()?;
        if now < grant.claim.admitted_at_unix_ms || now > grant.claim.expires_at_unix_ms {
            return Err(ApiError::Conflict(
                "campaign continuation preflight grant is not active under server time".to_owned(),
            ));
        }
    }
    let selected_competition = request
        .competition_manifest_sha256
        .map(|digest| competition_by_digest(&state.config, digest).map(|(_, manifest)| manifest))
        .transpose()?;
    let competition_grant = request.session_genesis.claim.competition_run_grant.clone();
    match (&selected_competition, &competition_grant) {
        (None, None) => {}
        (Some(competition), Some(grant)) => {
            competition
                .validate_run_grant(grant)
                .map_err(|error| ApiError::Conflict(error.to_string()))?;
            verify_request_signature(
                grant.claim.grant_authority_public_key.as_bytes(),
                grant.authority_signature.as_bytes(),
                &grant.signing_bytes()?,
            )?;
            let now = crate::model::now_unix_ms()?;
            if now < grant.claim.admitted_at_unix_ms || now > grant.claim.expires_at_unix_ms {
                return Err(ApiError::Conflict(
                    "competition run grant is not active under server time".to_owned(),
                ));
            }
        }
        _ => {
            return Err(ApiError::Conflict(
                "competition run grant presence does not match the selected competition".to_owned(),
            ));
        }
    }
    let starting_state = starting_state(&state, &request, profile).await?;
    let ranked = &request.session_genesis.claim.ranked_session;
    let content_manifest_sha256 = digest(&profile.content_manifest_id)?;
    let content_manifest = state
        .config
        .manifests
        .content_manifests
        .get(&content_manifest_sha256)
        .ok_or(ApiError::Internal)?;
    ranked
        .validate_content_manifest(content_manifest)
        .map_err(|error| ApiError::Conflict(error.to_string()))?;
    let campaign_content_manifest_sha256 = profile
        .campaign_content_manifest_id
        .as_deref()
        .map(digest)
        .transpose()?;
    let expected_campaign_content = match request.scope_request {
        ScopeRequestV1::IndividualLevel => None,
        ScopeRequestV1::CampaignGenesis | ScopeRequestV1::CampaignContinuation { .. } => {
            Some(campaign_content_manifest_sha256.ok_or(ApiError::Internal)?)
        }
    };
    if ranked.starting_campaign_sha256 != starting_state.campaign_sha256()
        || ranked.starting_campaign_byte_length != starting_state.starting_campaign_byte_length()
        || ranked.build_manifest_sha256 != digest(&profile.build_manifest_id)?
        || ranked.content_manifest_sha256 != content_manifest_sha256
        || ranked.campaign_content_manifest_sha256 != expected_campaign_content
        || ranked.rules_config_sha256 != digest(&profile.config_id)?
        || ranked.ruleset_manifest_sha256 != digest(&profile.ruleset_id)?
        || ranked.competition_manifest_sha256 != request.competition_manifest_sha256
    {
        return Err(ApiError::Conflict(
            "signed replay-session genesis does not match the selected server profile".to_owned(),
        ));
    }
    let host_key = request
        .participant_claims
        .first()
        .expect("validated offer request always contains host seat")
        .public_key;
    let allowed_metrics = allowed_metrics_for_offer(&state.config, &request, profile)?;
    let public_metadata_json = serde_json::to_string(profile).map_err(internal_json)?;
    let build_manifest_sha256 = digest(&profile.build_manifest_id)?;
    let rules_config_sha256 = digest(&profile.config_id)?;
    let ruleset_manifest_sha256 = digest(&profile.ruleset_id)?;
    // This is the last fallible step before the transaction which issues the
    // upload challenge. A red store, capacity plan, or backup must not create
    // an offer that the service cannot safely accept.
    ensure_offer_admission_ready(&state).await?;
    let (_challenge, offer) = state
        .database
        .issue_submission_offer(
            host_key.into_bytes(),
            Duration::from_secs(state.config.challenge_ttl_seconds),
            competition_grant.as_ref(),
            move |challenge| {
                let offer = SubmissionOfferV1 {
                    schema_version: SCHEMA_VERSION_V1,
                    upload_challenge_id: OpaqueId::new(challenge.id.clone())
                        .map_err(|error| crate::db::DbError::ResultInvariant(error.to_string()))?,
                    upload_challenge_nonce: challenge.nonce,
                    expires_at_unix_ms: challenge.expires_at_ms,
                    max_concurrent_players: request.max_concurrent_players,
                    participant_instance_count: request.participant_instance_count,
                    participant_claims: request.participant_claims,
                    session_genesis: request.session_genesis,
                    mission_id: request.mission_id,
                    competition_manifest_sha256: request.competition_manifest_sha256,
                    build_manifest_sha256,
                    content_manifest_sha256,
                    rules_config_sha256,
                    ruleset_manifest_sha256,
                    starting_state,
                    allowed_metrics,
                };
                offer
                    .validate()
                    .map_err(|error| crate::db::DbError::ResultInvariant(error.to_string()))?;
                if let Some(competition) = &selected_competition {
                    competition
                        .validate_submission_offer(&offer)
                        .map_err(|error| crate::db::DbError::ResultInvariant(error.to_string()))?;
                }
                let offer_json = serde_json::to_string(&offer)
                    .map_err(|error| crate::db::DbError::ResultInvariant(error.to_string()))?;
                Ok((offer, offer_json, public_metadata_json))
            },
        )
        .await?;
    Ok((StatusCode::CREATED, Json(offer)))
}

async fn submit(
    State(state): State<AppState>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> Result<(StatusCode, Json<SubmissionAcceptedV1>), ApiError> {
    if headers.contains_key(CONTENT_ENCODING) {
        return Err(ApiError::BadRequest(
            "submission requests must not use a transport content encoding".to_owned(),
        ));
    }
    let metadata = multipart
        .next_field()
        .await
        .map_err(|error| ApiError::BadRequest(error.to_string()))?
        .ok_or_else(|| ApiError::BadRequest("missing `submission` multipart field".to_owned()))?;
    if metadata.name() != Some("submission") {
        return Err(ApiError::BadRequest(
            "the first multipart field must be `submission`".to_owned(),
        ));
    }
    require_multipart_media_type(&metadata, "application/json", "submission")?;
    let metadata_bytes = read_bounded_field(metadata, state.config.max_metadata_bytes).await?;
    let signed: SignedSubmissionV1 = serde_json::from_slice(&metadata_bytes)
        .map_err(|error| ApiError::BadRequest(format!("invalid submission JSON: {error}")))?;
    signed.validate()?;
    let artifacts = &signed.submission.artifacts;
    if artifacts.replay.replay_schema_version != RANKED_REPLAY_SCHEMA_VERSION {
        return Err(ApiError::BadRequest(format!(
            "ranked submissions require replay schema {RANKED_REPLAY_SCHEMA_VERSION}"
        )));
    }
    let now = crate::model::now_unix_ms()?;
    if signed.submission.offer.expires_at_unix_ms < now {
        return Err(ApiError::Conflict(
            "submission offer has expired".to_owned(),
        ));
    }
    if signed
        .submission
        .offer
        .session_genesis
        .claim
        .competition_run_grant
        .as_ref()
        .is_some_and(|grant| {
            now < grant.claim.admitted_at_unix_ms || now > grant.claim.expires_at_unix_ms
        })
    {
        return Err(ApiError::Conflict(
            "competition upload is outside its server-admitted interval".to_owned(),
        ));
    }
    let stored_offer = state
        .database
        .stored_offer(signed.submission.offer.upload_challenge_id.as_str())
        .await?;
    let authenticated =
        crate::submission::authenticate_reserved_offer(&signed, &stored_offer.offer_json)?;

    let build = state
        .config
        .manifests
        .builds
        .get(&signed.submission.offer.build_manifest_sha256)
        .ok_or_else(|| {
            tracing::error!(
                error_code = "submission_build_manifest_missing",
                "server-issued submission offer references a missing build manifest"
            );
            ApiError::Internal
        })?;
    if build.public_digest() != signed.submission.offer.build_manifest_sha256 {
        tracing::error!(
            error_code = "submission_build_manifest_identity_mismatch",
            "server build registry key differs from its canonical document digest"
        );
        return Err(ApiError::Internal);
    }
    let required_build_hash = build
        .semantics()
        .source_commit
        .get(..robin_replay_format::VERSION_HASH_BYTES)
        .ok_or_else(|| {
            tracing::error!(
                error_code = "submission_build_source_commit_short",
                "validated build manifest has no compact replay build prefix"
            );
            ApiError::Internal
        })?;

    // Read and lexically preflight the only replay field before reserving the
    // upload. This buffer is bounded by the configured canonical replay ceiling;
    // the scan itself borrows it and performs no attacker-sized allocation.
    let replay_field = multipart
        .next_field()
        .await
        .map_err(|error| ApiError::BadRequest(error.to_string()))?
        .ok_or_else(|| ApiError::BadRequest("missing `replay` multipart field".to_owned()))?;
    if replay_field.name() != Some("replay") {
        return Err(ApiError::BadRequest(
            "the second multipart field must be `replay`".to_owned(),
        ));
    }
    require_multipart_media_type(
        &replay_field,
        &artifacts.replay.artifact.media_type,
        "replay",
    )?;
    let replay_limit =
        usize::try_from(state.config.max_replay_bytes).map_err(|_| ApiError::Internal)?;
    let replay_bytes = read_bounded_field(replay_field, replay_limit).await?;
    preflight_ranked_replay_transport(
        &replay_bytes,
        &artifacts.replay.artifact,
        required_build_hash,
    )?;

    let lease_ttl = Duration::from_secs(
        state
            .config
            .upload_timeout_seconds
            .checked_add(30)
            .ok_or(ApiError::Internal)?,
    );
    let ingestion_state = &state;
    let ingestion = |resume_uploaded| {
        let state = ingestion_state;
        async move {
            if resume_uploaded {
                verify_reserved_upload_storage_identity(
                    state,
                    artifacts.replay.artifact.sha256.into_bytes(),
                    artifacts.replay.artifact.byte_length,
                    artifacts.starting_campaign.sha256.into_bytes(),
                    artifacts.starting_campaign.byte_length,
                )
                .await
            } else {
                async {
                    let stored_replay = state
                        .replay_store
                        .store_stream(
                            stream::iter([Ok::<_, Infallible>(Bytes::from(replay_bytes))]),
                            artifacts.replay.artifact.sha256.into_bytes(),
                            artifacts.replay.artifact.byte_length,
                        )
                        .await?;
                    // Replay bytes remain semantically opaque in the network-facing
                    // process. The contained verifier is the sole base64/zstd/bitcode
                    // decoder and canonical representation authority.
                    let starting_campaign_field = multipart
                        .next_field()
                        .await
                        .map_err(|error| ApiError::BadRequest(error.to_string()))?
                        .ok_or_else(|| {
                            ApiError::BadRequest(
                                "missing `starting_campaign` multipart field".to_owned(),
                            )
                        })?;
                    if starting_campaign_field.name() != Some("starting_campaign") {
                        return Err(ApiError::BadRequest(
                            "the third multipart field must be `starting_campaign`".to_owned(),
                        ));
                    }
                    require_multipart_media_type(
                        &starting_campaign_field,
                        &artifacts.starting_campaign.media_type,
                        "starting_campaign",
                    )?;
                    let stored_starting_campaign = state
                        .campaign_store
                        .store_stream(
                            starting_campaign_field,
                            artifacts.starting_campaign.sha256.into_bytes(),
                            artifacts.starting_campaign.byte_length,
                        )
                        .await?;
                    if multipart
                        .next_field()
                        .await
                        .map_err(|error| ApiError::BadRequest(error.to_string()))?
                        .is_some()
                    {
                        return Err(ApiError::BadRequest(
                            "submission multipart body must contain exactly three fields"
                                .to_owned(),
                        ));
                    }
                    debug_assert_eq!(
                        stored_replay.sha256,
                        artifacts.replay.artifact.sha256.into_bytes()
                    );
                    debug_assert_eq!(
                        stored_starting_campaign.sha256,
                        artifacts.starting_campaign.sha256.into_bytes()
                    );
                    Ok(())
                }
                .await
            }
        }
    };
    let lifecycle = crate::submission::complete_upload(
        &state.database,
        &authenticated,
        lease_ttl,
        Duration::from_secs(state.config.upload_reservation_ttl_seconds),
        ensure_upload_admission_ready(
            &state,
            artifacts.replay.artifact.byte_length,
            artifacts.starting_campaign.byte_length,
        ),
        ingestion,
    )
    .await?;
    submission_accepted_response(&state, lifecycle).await
}

async fn submission_accepted_response(
    state: &AppState,
    lifecycle: crate::model::SubmissionLifecycle,
) -> Result<(StatusCode, Json<SubmissionAcceptedV1>), ApiError> {
    let accepted_submission_id = lifecycle.id.clone();
    let state_value = lifecycle_state(state, lifecycle).await?;
    if !matches!(
        state_value,
        SubmissionLifecycleV1::Queued
            | SubmissionLifecycleV1::Verifying
            | SubmissionLifecycleV1::RetryPending
    ) {
        return Err(ApiError::Conflict(
            "offer was already submitted; query its lifecycle resource".to_owned(),
        ));
    }
    Ok((
        StatusCode::ACCEPTED,
        Json(SubmissionAcceptedV1 {
            schema_version: SCHEMA_VERSION_V1,
            submission_id: opaque(&accepted_submission_id)?,
            state: state_value,
            retry_after_ms: SUBMISSION_RETRY_AFTER_MS,
        }),
    ))
}

async fn verify_reserved_upload_storage_identity(
    state: &AppState,
    replay_sha256: [u8; 32],
    replay_bytes: u64,
    campaign_sha256: [u8; 32],
    campaign_bytes: u64,
) -> Result<(), ApiError> {
    drop(
        state
            .replay_store
            .open_verified(&replay_sha256, replay_bytes)
            .await?,
    );
    let campaign = state.campaign_store.open_verified(&campaign_sha256).await?;
    if campaign
        .metadata()
        .await
        .map_err(|_error| {
            tracing::error!(
                error_code = "campaign_state_storage_io",
                "campaign metadata failed"
            );
            ApiError::Internal
        })?
        .len()
        != campaign_bytes
    {
        tracing::error!(
            error_code = "campaign_state_length_mismatch",
            "reserved campaign artifact has the wrong byte length"
        );
        return Err(ApiError::Internal);
    }
    Ok(())
}

async fn submission_owner_status_challenge(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<SubmissionOwnerStatusChallengeRequestV1>,
) -> Result<(StatusCode, Json<SubmissionOwnerStatusChallengeV1>), ApiError> {
    rate_limit_challenge(&state, peer, &headers, ChallengePurpose::OwnerStatus).await?;
    request.validate()?;
    let issued = state
        .database
        .issue_owner_status_challenge(
            request.controller_public_key.into_bytes(),
            request.submission_id.as_str(),
            Duration::from_secs(state.config.challenge_ttl_seconds),
        )
        .await?;
    let challenge = SubmissionOwnerStatusChallengeV1 {
        schema_version: SCHEMA_VERSION_V1,
        owner_status_challenge_id: opaque(&issued.id)?,
        owner_status_challenge_nonce: issued.nonce,
        expires_at_unix_ms: issued.expires_at_ms,
        controller_public_key: request.controller_public_key,
        submission_id: request.submission_id,
    };
    challenge
        .validate()
        .map_err(|error| configuration_error("owner status challenge", error))?;
    Ok((StatusCode::CREATED, Json(challenge)))
}

async fn submission_private_status(
    State(state): State<AppState>,
    Path(submission_id): Path<String>,
    Json(envelope): Json<SubmissionOwnerStatusEnvelopeV1>,
) -> Result<Json<SubmissionOwnerStatusResponseV1>, ApiError> {
    envelope.validate()?;
    if envelope.challenge.submission_id.as_str() != submission_id {
        return Err(ApiError::Unauthorized);
    }
    let signing_bytes = envelope.signing_bytes()?;
    verify_request_signature(
        envelope.challenge.controller_public_key.as_bytes(),
        envelope.signature.as_bytes(),
        &signing_bytes,
    )?;
    let lifecycle = state
        .database
        .consume_owner_status_challenge(
            envelope.challenge.owner_status_challenge_id.as_str(),
            envelope.challenge.owner_status_challenge_nonce.into_bytes(),
            envelope.challenge.expires_at_unix_ms,
            envelope.challenge.controller_public_key.into_bytes(),
            &submission_id,
        )
        .await
        .map_err(|error| match error {
            crate::db::DbError::InvalidChallenge | crate::db::DbError::NotFound => {
                ApiError::Unauthorized
            }
            other => other.into(),
        })?;
    let response = SubmissionOwnerStatusResponseV1 {
        schema_version: SCHEMA_VERSION_V1,
        submission_id: envelope.challenge.submission_id.clone(),
        controller_public_key: envelope.challenge.controller_public_key,
        owner_status_envelope_sha256: envelope
            .canonical_digest()
            .map_err(|error| ApiError::BadRequest(error.to_string()))?,
        state: lifecycle_state(&state, lifecycle).await?,
    };
    response.validate().map_err(|error| {
        tracing::error!(
            error_code = "owner_status_response_invalid",
            "private status projection failed: {error}"
        );
        ApiError::Internal
    })?;
    Ok(Json(response))
}

async fn build_manifest(
    State(state): State<AppState>,
    Path(value): Path<String>,
) -> Result<Response, ApiError> {
    let digest = digest(&value).map_err(|_| ApiError::NotFound)?;
    let build = state
        .config
        .manifests
        .builds
        .get(&digest)
        .ok_or(ApiError::NotFound)?;
    if build.public_digest() != digest {
        tracing::error!(
            error_code = "build_registry_identity_mismatch",
            route_digest = %digest,
            document_digest = %build.public_digest(),
            "build registry key differs from the exact public document identity"
        );
        return Err(ApiError::Internal);
    }
    immutable_json(build.public_document())
}

async fn content_manifest(
    State(state): State<AppState>,
    Path(value): Path<String>,
) -> Result<Response, ApiError> {
    registry_document(&value, &state.config.manifests.content_manifests)
}

async fn campaign_content_manifest(
    State(state): State<AppState>,
    Path(value): Path<String>,
) -> Result<Response, ApiError> {
    registry_document(&value, &state.config.manifests.campaign_content_manifests)
}

async fn rules_config(
    State(state): State<AppState>,
    Path(value): Path<String>,
) -> Result<Response, ApiError> {
    let digest = digest(&value).map_err(|_| ApiError::NotFound)?;
    if let Some(rules) = state.config.manifests.rules_configs.get(&digest) {
        return immutable_json(rules);
    }
    let rules = state
        .database
        .public_custom_rules_config(digest)
        .await?
        .ok_or(ApiError::NotFound)?;
    immutable_json(&rules)
}

async fn ruleset_manifest(
    State(state): State<AppState>,
    Path(value): Path<String>,
) -> Result<Response, ApiError> {
    let digest = digest(&value).map_err(|_| ApiError::NotFound)?;
    let published = state
        .config
        .manifests
        .rulesets
        .get(&digest)
        .ok_or(ApiError::NotFound)?;
    immutable_json(&published.manifest)
}

async fn published_ruleset(
    State(state): State<AppState>,
    Path(value): Path<String>,
) -> Result<Response, ApiError> {
    let digest = digest(&value).map_err(|_| ApiError::NotFound)?;
    let published = state
        .config
        .manifests
        .rulesets
        .get(&digest)
        .ok_or(ApiError::NotFound)?;
    published.validate().map_err(|_error| {
        tracing::error!(
            error_code = "published_ruleset_invalid",
            "published ruleset is no longer valid"
        );
        ApiError::Internal
    })?;
    if published.ruleset_manifest_sha256 != digest {
        return Err(ApiError::Internal);
    }
    let mut response = Json(published).into_response();
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
        .headers_mut()
        .insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    Ok(response)
}

async fn competition_manifest_route(
    State(state): State<AppState>,
    Path(value): Path<String>,
) -> Result<Response, ApiError> {
    registry_document(&value, &state.config.manifests.competitions)
}

async fn policy_manifest(
    State(state): State<AppState>,
    Path(value): Path<String>,
) -> Result<Response, ApiError> {
    registry_document(&value, &state.config.manifests.policies)
}

fn registry_document<T: CanonicalDocument>(
    value: &str,
    documents: &BTreeMap<Digest32, T>,
) -> Result<Response, ApiError> {
    let key = digest(value).map_err(|_| ApiError::NotFound)?;
    immutable_json(documents.get(&key).ok_or(ApiError::NotFound)?)
}

fn immutable_json<T: CanonicalDocument>(document: &T) -> Result<Response, ApiError> {
    let bytes = document.canonical_bytes().map_err(|_error| {
        tracing::error!(
            error_code = "immutable_manifest_noncanonical",
            "immutable manifest is no longer canonical"
        );
        ApiError::Internal
    })?;
    let mut response = Body::from(bytes).into_response();
    response.headers_mut().insert(
        CONTENT_TYPE,
        HeaderValue::from_static("application/json; charset=utf-8"),
    );
    response.headers_mut().insert(
        CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=31536000, immutable"),
    );
    response
        .headers_mut()
        .insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    Ok(response)
}

async fn leaderboard(
    State(state): State<AppState>,
    Query(query): Query<LeaderboardQueryV1>,
) -> Result<Json<LeaderboardPageV1>, ApiError> {
    query.validate()?;
    if u32::from(query.limit) > state.config.max_page_size {
        return Err(ApiError::BadRequest(format!(
            "limit exceeds server maximum {}",
            state.config.max_page_size
        )));
    }
    let filter = query
        .filter()
        .map_err(|error| ApiError::BadRequest(error.to_string()))?;
    let filter_sha = filter_digest(&filter)?;
    let decoded_cursor = query
        .cursor
        .as_deref()
        .map(|cursor| decode_cursor(cursor, filter_sha, &state.cursor_hmac_key))
        .transpose()?;
    let db_cursor = decoded_cursor.as_ref().map(|cursor| BoardCursor {
        metric_value: cursor.metric_value,
        accepted_sequence: cursor.accepted_sequence,
        run_id: cursor.run_id.clone(),
    });
    let visibility_revision = state.database.leaderboard_visibility_revision().await?;
    if decoded_cursor
        .as_ref()
        .is_some_and(|cursor| cursor.visibility_revision != visibility_revision)
    {
        return Err(ApiError::Conflict(
            "leaderboard changed while paging; restart from the first page".to_owned(),
        ));
    }
    let accepted_sequence_watermark = match &decoded_cursor {
        Some(cursor) => cursor.accepted_sequence_watermark,
        None => state.database.accepted_sequence_watermark().await?,
    };
    let profile = profile_for_filter(&state.config, &filter)?;
    let published = state
        .config
        .manifests
        .rulesets
        .get(&digest(&profile.ruleset_id)?)
        .ok_or(ApiError::Internal)?;
    let mut allowed_rulesets = Vec::new();
    for (id, candidate) in &state.config.manifests.rulesets {
        if filter
            .ruleset_manifest_sha256
            .is_some_and(|selected| selected != *id)
        {
            continue;
        }
        let mut exact_filter = filter.clone();
        exact_filter.ruleset_manifest_sha256 = Some(*id);
        exact_filter.rules_config_sha256 = Some(candidate.manifest.rules_config_sha256);
        if profile_for_filter(&state.config, &exact_filter).is_err() {
            continue;
        }
        if candidate.manifest.tick_duration != published.manifest.tick_duration {
            return Err(ApiError::BadRequest(
                "combined boards require rulesets with the same simulation tick duration"
                    .to_owned(),
            ));
        }
        allowed_rulesets.push(*id);
    }
    let mut rows = state
        .database
        .leaderboard_rows(
            &filter,
            db_cursor.as_ref(),
            u32::from(query.limit) + 1,
            accepted_sequence_watermark,
            &allowed_rulesets,
        )
        .await?;
    if state.database.leaderboard_visibility_revision().await? != visibility_revision {
        return Err(ApiError::Conflict(
            "leaderboard changed while paging; restart from the first page".to_owned(),
        ));
    }
    let has_more = rows.len() > usize::from(query.limit);
    rows.truncate(usize::from(query.limit));
    let first_position = decoded_cursor
        .as_ref()
        .map_or(1, |cursor| cursor.position.saturating_add(1));
    let mut entries = Vec::with_capacity(rows.len());
    for (offset, row) in rows.iter().enumerate() {
        let position = first_position
            .checked_add(u64::try_from(offset).map_err(|_| ApiError::Internal)?)
            .ok_or(ApiError::Internal)?;
        entries.push(board_entry(
            row,
            position,
            query.metric,
            &published.manifest.tick_duration,
        )?);
    }
    let next_cursor = if has_more {
        entries
            .last()
            .map(|entry| {
                let token = CursorToken {
                    filter_sha256: filter_sha,
                    accepted_sequence_watermark,
                    visibility_revision,
                    metric_value: rows.last().ok_or(ApiError::Internal)?.metric_value,
                    position: entry.position,
                    rank: entry.rank,
                    accepted_sequence: i64::try_from(entry.accepted_sequence)
                        .map_err(|_| ApiError::Internal)?,
                    verified_at_unix_ms: entry.verified_at_unix_ms,
                    run_id: entry.run_id.as_str().to_owned(),
                };
                let opaque_token = encode_cursor(&token, &state.cursor_hmac_key)?;
                leaderboard_cursor(
                    &token,
                    opaque_token,
                    query.metric,
                    &published.manifest.tick_duration,
                )
            })
            .transpose()?
    } else {
        None
    };
    let previous_cursor = if entries.is_empty() {
        None
    } else {
        decoded_cursor
            .as_ref()
            .zip(query.cursor.as_ref())
            .map(|(cursor, opaque_token)| {
                leaderboard_cursor(
                    cursor,
                    opaque_token.clone(),
                    query.metric,
                    &published.manifest.tick_duration,
                )
            })
            .transpose()?
    };
    let page = LeaderboardPageV1 {
        schema_version: SCHEMA_VERSION_V1,
        filter,
        accepted_sequence_watermark,
        previous_cursor,
        entries,
        next_cursor,
    };
    (if page.filter.ruleset_manifest_sha256.is_some() {
        page.validate_against_ruleset(published)
    } else {
        page.validate()
    })
    .map_err(|_error| {
        tracing::error!(
            error_code = "leaderboard_page_invalid",
            "leaderboard query produced an invalid protocol page"
        );
        ApiError::Internal
    })?;
    Ok(Json(page))
}

async fn run_detail(
    State(state): State<AppState>,
    Path(run_id): Path<String>,
) -> Result<Json<RunDetailV1>, ApiError> {
    let run = match state.database.public_run(&run_id).await {
        Ok(run) => run,
        Err(crate::db::DbError::NotFound) => {
            return full_campaign_detail(&state, &run_id).await.map(Json);
        }
        Err(error) => return Err(error.into()),
    };
    if run.campaign_terminal || run.campaign_session_kind.as_deref() == Some("headquarters") {
        return Err(ApiError::NotFound);
    }
    let profile: AdmissionProfile =
        serde_json::from_str(&run.public_metadata_json).map_err(|_error| {
            tracing::error!(
                error_code = "stored_public_metadata_invalid",
                "stored public metadata is corrupt"
            );
            ApiError::Internal
        })?;
    let named = public_participants(&run.named_participants);
    let verification_proof = run.verification_proof.clone();
    let input_provenance = verification_proof.input_provenance.clone();
    let subject = LeaderboardSubjectV1::Mission {
        mission_id: run.mission_id.clone(),
        category: category(&run.scope_kind)?,
    };
    let build_manifest = state
        .config
        .manifests
        .builds
        .get(&Digest32::from_bytes(run.build_manifest_id))
        .ok_or(ApiError::Internal)?;
    let viewer = viewer_launch(
        &state.config,
        &profile,
        run.build_manifest_id,
        run.content_manifest_id,
    )?;
    let detail = RunDetailV1 {
        schema_version: SCHEMA_VERSION_V1,
        run_id: opaque(&run.run_id)?,
        rank: None,
        subject,
        mission: Some(MissionFacetV1 {
            mission_id: run.mission_id.clone(),
            display_name: profile.mission_display_name.clone(),
            content_manifest_sha256: Digest32::from_bytes(run.content_manifest_id),
        }),
        composition: VerifiedRunCompositionV1::Mission {
            replay_sha256: Digest32::from_bytes(run.replay_sha256),
        },
        outcome: TerminalOutcomeV1::Won,
        metrics: verification_proof.metrics.clone(),
        metric_value: BoardMetricValueV1::OriginalScore {
            points: verification_proof.metrics.original_score_delta,
        },
        max_concurrent_players: verification_proof.max_concurrent_players,
        participant_instance_count: u32::from(verification_proof.participant_instance_count),
        named_participant_instance_count: u32::from(
            verification_proof.named_participant_instance_count,
        ),
        anonymous_participant_instance_count: u32::from(
            verification_proof.anonymous_participant_instance_count,
        ),
        named_participants: named,
        aggregate_named_participants: Vec::new(),
        verified_at_unix_ms: run.verified_at_ms,
        public_request_sha256: Digest32::from_bytes(
            run.public_verification_request_sha256,
        ),
        public_result_sha256: Digest32::from_bytes(run.public_verification_result_sha256),
        verification_proof: Some(verification_proof.clone()),
        campaign_aggregate: None,
        replay: Some(verification_proof.public_request.replay.clone()),
        content: RunContentIdentityV1::Mission {
            content_manifest_sha256: Digest32::from_bytes(run.content_manifest_id),
        },
        campaign_content_manifest_sha256: run
            .campaign_content_manifest_id
            .map(Digest32::from_bytes),
        rules_config_sha256: Digest32::from_bytes(run.config_id),
        ruleset_manifest_sha256: Digest32::from_bytes(run.ruleset_id),
        competition_manifest_sha256: run
            .competition_manifest_id
            .map(Digest32::from_bytes),
        starting_campaign: verification_proof.starting_campaign.clone(),
        final_campaign: verification_proof.final_campaign.clone(),
        starting_campaign_score: verification_proof.starting_campaign_score,
        final_campaign_score: verification_proof.final_campaign_score,
        input_provenance,
        build: Some(PublicBuildV1 {
            manifest_sha256: Digest32::from_bytes(run.build_manifest_id),
            source_commit: build_manifest.semantics().source_commit.clone(),
            display_name: profile.build_display_name,
        }),
        achievements: verification_proof
            .achievements
            .iter()
            .cloned()
            .map(|verified| AchievementSummaryV1 {
                display_name: verified.achievement_id.as_str().to_owned(),
                verified,
            })
            .collect(),
        viewer: Some(viewer),
        full_campaign_sessions: Vec::new(),
        trust_statement: "Server replay-verified under the pinned build, official content, and rules. This proves the deterministic result, not that a human or unmodified client produced the commands."
            .to_owned(),
    };
    let published = state
        .config
        .manifests
        .rulesets
        .get(&detail.ruleset_manifest_sha256)
        .ok_or(ApiError::Internal)?;
    if !matches!(
        published.operational_status,
        RulesetOperationalStatusV1::Active
    ) {
        return Err(ApiError::NotFound);
    }
    let campaign_content = run
        .campaign_content_manifest_id
        .map(Digest32::from_bytes)
        .map(|digest| {
            state
                .config
                .manifests
                .campaign_content_manifests
                .get(&digest)
                .ok_or(ApiError::Internal)
        })
        .transpose()?;
    detail
        .validate_against_ruleset(published, campaign_content)
        .map_err(|_error| {
            tracing::error!(
                error_code = "stored_public_run_invalid",
                "stored run cannot satisfy public protocol"
            );
            ApiError::Internal
        })?;
    Ok(Json(detail))
}

async fn full_campaign_detail(state: &AppState, run_id: &str) -> Result<RunDetailV1, ApiError> {
    let run = state.database.public_full_campaign(run_id).await?;
    let public_aggregate = run.aggregate_proof.clone();
    let mut sessions = Vec::with_capacity(run.ordered_session_run_ids.len());
    for (ordinal, (session_run_id, binding)) in run
        .ordered_session_run_ids
        .iter()
        .zip(&public_aggregate.public_request.sessions)
        .enumerate()
    {
        if binding.run_id.as_str() != session_run_id || binding.ordinal != ordinal as u32 {
            return Err(ApiError::Internal);
        }
        let session = state.database.public_run(session_run_id).await?;
        let profile: AdmissionProfile = serde_json::from_str(&session.public_metadata_json)
            .map_err(|_error| {
                tracing::error!(
                    error_code = "stored_session_metadata_invalid",
                    "stored session metadata is corrupt"
                );
                ApiError::Internal
            })?;
        let build_manifest = state
            .config
            .manifests
            .builds
            .get(&Digest32::from_bytes(session.build_manifest_id))
            .ok_or(ApiError::Internal)?;
        let verification_proof = session.verification_proof.clone();
        let public_result_sha256 = verification_proof
            .canonical_digest()
            .map_err(|_| ApiError::Internal)?;
        if verification_proof.public_request_sha256 != binding.public_verification_request_sha256
            || public_result_sha256 != binding.public_verification_result_sha256
        {
            return Err(ApiError::Internal);
        }
        let achievement_summaries = verification_proof
            .achievements
            .iter()
            .cloned()
            .map(|verified| AchievementSummaryV1 {
                display_name: verified.achievement_id.as_str().to_owned(),
                verified,
            })
            .collect();
        let verified_kind = verification_proof
            .campaign_session_kind
            .as_ref()
            .ok_or(ApiError::Internal)?;
        let (session_kind, display_name, mission) = match verified_kind {
            robin_run_protocol::CampaignSessionKindV1::FieldMission { mission_id } => (
                FullCampaignSessionKindV1::FieldMission {
                    mission_id: mission_id.clone(),
                },
                profile.mission_display_name.clone(),
                Some(MissionFacetV1 {
                    mission_id: session.mission_id.clone(),
                    display_name: profile.mission_display_name.clone(),
                    content_manifest_sha256: Digest32::from_bytes(session.content_manifest_id),
                }),
            ),
            robin_run_protocol::CampaignSessionKindV1::Headquarters { hq_sequence } => (
                FullCampaignSessionKindV1::Headquarters {
                    hq_sequence: *hq_sequence,
                },
                format!("Headquarters {hq_sequence}"),
                None,
            ),
        };
        sessions.push(FullCampaignSessionV1 {
            ordinal: binding.ordinal,
            run_id: opaque(&session.run_id)?,
            session: session_kind,
            content_subject: verification_proof.public_request.content_subject.clone(),
            display_name,
            mission,
            replay: verification_proof.public_request.replay.clone(),
            public_verification_request_sha256: binding.public_verification_request_sha256,
            public_verification_result_sha256: binding.public_verification_result_sha256,
            starting_campaign: verification_proof.starting_campaign.clone(),
            final_campaign: verification_proof.final_campaign.clone(),
            starting_campaign_score: verification_proof.starting_campaign_score,
            final_campaign_score: verification_proof.final_campaign_score,
            public_campaign_complete_evidence_sha256: verification_proof
                .campaign_complete_evidence
                .as_ref()
                .map(CanonicalDocument::canonical_digest)
                .transpose()
                .map_err(|_| ApiError::Internal)?,
            content_manifest_sha256: verification_proof.public_request.content_manifest_sha256,
            rules_config_sha256: verification_proof.public_request.rules_config_sha256,
            ruleset_manifest_sha256: verification_proof.public_request.ruleset_manifest_sha256,
            competition_manifest_sha256: verification_proof
                .public_request
                .competition_manifest_sha256,
            max_concurrent_players: verification_proof.max_concurrent_players,
            participant_instance_count: verification_proof.participant_instance_count,
            named_participant_instance_count: verification_proof.named_participant_instance_count,
            anonymous_participant_instance_count: verification_proof
                .anonymous_participant_instance_count,
            named_participants: public_participants(&session.named_participants),
            input_provenance: verification_proof.input_provenance.clone(),
            metrics: verification_proof.metrics.clone(),
            achievements: achievement_summaries,
            build: PublicBuildV1 {
                manifest_sha256: Digest32::from_bytes(session.build_manifest_id),
                source_commit: build_manifest.semantics().source_commit.clone(),
                display_name: profile.build_display_name.clone(),
            },
            viewer: viewer_launch(
                &state.config,
                &profile,
                session.build_manifest_id,
                session.content_manifest_id,
            )?,
            verification_proof,
        });
    }
    let aggregate_named = aggregate_public_participants(&run.named_participants);
    let detail = RunDetailV1 {
        schema_version: SCHEMA_VERSION_V1,
        run_id: opaque(&run.run_id)?,
        rank: None,
        subject: LeaderboardSubjectV1::FullCampaign,
        mission: None,
        composition: VerifiedRunCompositionV1::FullCampaign {
            ordered_session_run_ids: run
                .ordered_session_run_ids
                .iter()
                .map(|id| opaque(id))
                .collect::<Result<Vec<_>, _>>()?,
        },
        outcome: TerminalOutcomeV1::Won,
        metrics: public_aggregate.metrics.clone(),
        metric_value: BoardMetricValueV1::OriginalScore {
            points: public_aggregate.metrics.original_score_delta,
        },
        max_concurrent_players: public_aggregate.max_concurrent_players,
        participant_instance_count: public_aggregate.participant_instance_count,
        named_participant_instance_count: public_aggregate.named_participant_instance_count,
        named_participants: Vec::new(),
        aggregate_named_participants: aggregate_named,
        anonymous_participant_instance_count: public_aggregate
            .anonymous_participant_instance_count,
        verified_at_unix_ms: run.verified_at_ms,
        public_request_sha256: Digest32::from_bytes(run.public_aggregate_request_sha256),
        public_result_sha256: Digest32::from_bytes(run.public_aggregate_result_sha256),
        verification_proof: None,
        campaign_aggregate: Some(public_aggregate.clone()),
        replay: None,
        content: RunContentIdentityV1::FullCampaign {
            campaign_content_manifest_sha256: Digest32::from_bytes(
                run.campaign_content_manifest_id,
            ),
        },
        campaign_content_manifest_sha256: None,
        rules_config_sha256: Digest32::from_bytes(run.config_id),
        ruleset_manifest_sha256: Digest32::from_bytes(run.ruleset_id),
        competition_manifest_sha256: run
            .competition_manifest_id
            .map(Digest32::from_bytes),
        starting_campaign: public_aggregate
            .canonical_genesis_campaign
            .clone(),
        final_campaign: public_aggregate.final_campaign.clone(),
        starting_campaign_score: public_aggregate.starting_campaign_score,
        final_campaign_score: public_aggregate.final_campaign_score,
        input_provenance: InputProvenanceStatusV1::Rankable,
        build: None,
        achievements: Vec::new(),
        viewer: None,
        full_campaign_sessions: sessions,
        trust_statement: "Server-reduced from a complete, ordered, gap-free chain of independently replay-verified campaign and headquarters sessions under one pinned official campaign content catalog and ruleset."
            .to_owned(),
    };
    let published = state
        .config
        .manifests
        .rulesets
        .get(&detail.ruleset_manifest_sha256)
        .ok_or(ApiError::Internal)?;
    if !matches!(
        published.operational_status,
        RulesetOperationalStatusV1::Active
    ) {
        return Err(ApiError::NotFound);
    }
    let campaign_content = state
        .config
        .manifests
        .campaign_content_manifests
        .get(&Digest32::from_bytes(run.campaign_content_manifest_id))
        .ok_or(ApiError::Internal)?;
    detail
        .validate_against_ruleset(published, Some(campaign_content))
        .map_err(|_error| {
            tracing::error!(
                error_code = "stored_public_campaign_invalid",
                "stored full campaign cannot satisfy public protocol"
            );
            ApiError::Internal
        })?;
    Ok(detail)
}

async fn run_replay(
    State(state): State<AppState>,
    Path(run_id): Path<String>,
) -> Result<Response, ApiError> {
    let (digest, bytes, ruleset) = state.database.replay_for_run(&run_id).await?;
    ensure_ruleset_is_public(&state.config, ruleset)?;
    replay_response(&state, digest, bytes).await
}

async fn run_public_campaign(
    State(state): State<AppState>,
    Path((run_id, campaign_role)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    if !matches!(campaign_role.as_str(), "starting" | "final") {
        return Err(ApiError::NotFound);
    }
    // Revalidate the immutable stored public proof used by detail before
    // serving an artifact selected only through its run-scoped public link.
    let _ = run_detail(State(state.clone()), Path(run_id.clone())).await?;
    let (artifact, ruleset) = state
        .database
        .public_campaign_for_run(&run_id, &campaign_role)
        .await?;
    ensure_ruleset_is_public(&state.config, ruleset)?;
    public_campaign_response(
        &state,
        artifact,
        &format!("verified-public-{campaign_role}.campaign"),
    )
    .await
}

async fn campaign_session_public_campaign(
    State(state): State<AppState>,
    Path((run_id, ordinal, campaign_role)): Path<(String, u32, String)>,
) -> Result<Response, ApiError> {
    let _ = full_campaign_detail(&state, &run_id).await?;
    let (artifact, ruleset) = state
        .database
        .public_campaign_for_campaign_session(&run_id, ordinal, &campaign_role)
        .await?;
    ensure_ruleset_is_public(&state.config, ruleset)?;
    public_campaign_response(
        &state,
        artifact,
        &format!("verified-public-session-{ordinal}-{campaign_role}.campaign"),
    )
    .await
}

async fn public_campaign_response(
    state: &AppState,
    artifact: ArtifactRefV1,
    filename: &str,
) -> Result<Response, ApiError> {
    let file = state
        .campaign_store
        .open_verified(artifact.sha256.as_bytes())
        .await?;
    if file.metadata().await.map_err(|_| ApiError::Internal)?.len() != artifact.byte_length {
        tracing::error!(
            error_code = "public_campaign_length_mismatch",
            "public campaign object differs from its verified link"
        );
        return Err(ApiError::Internal);
    }
    let stream = ReaderStream::new(file);
    let mut response = Response::new(Body::from_stream(stream));
    response.headers_mut().insert(
        CONTENT_TYPE,
        HeaderValue::from_str(&artifact.media_type).map_err(|_| ApiError::Internal)?,
    );
    response.headers_mut().insert(
        CONTENT_LENGTH,
        HeaderValue::from_str(&artifact.byte_length.to_string()).map_err(|_| ApiError::Internal)?,
    );
    response.headers_mut().insert(
        CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!("attachment; filename=\"{filename}\""))
            .map_err(|_| ApiError::Internal)?,
    );
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
        .headers_mut()
        .insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    Ok(response)
}

async fn campaign_session_detail(
    State(state): State<AppState>,
    Path((run_id, ordinal)): Path<(String, u32)>,
) -> Result<Json<CampaignSessionDetailV1>, ApiError> {
    let aggregate = full_campaign_detail(&state, &run_id).await?;
    let session = aggregate
        .full_campaign_sessions
        .get(usize::try_from(ordinal).map_err(|_| ApiError::NotFound)?)
        .cloned()
        .ok_or(ApiError::NotFound)?;
    let detail = CampaignSessionDetailV1 {
        schema_version: SCHEMA_VERSION_V1,
        aggregate_run_id: opaque(&run_id)?,
        public_aggregate_result_sha256: aggregate.public_result_sha256,
        ordinal,
        session,
    };
    detail
        .validate_against_aggregate(&aggregate)
        .map_err(|_error| {
            tracing::error!(
                error_code = "stored_public_session_invalid",
                "stored campaign session cannot satisfy public protocol"
            );
            ApiError::Internal
        })?;
    Ok(Json(detail))
}

async fn campaign_session_replay(
    State(state): State<AppState>,
    Path((run_id, ordinal)): Path<(String, u32)>,
) -> Result<Response, ApiError> {
    let (digest, bytes, ruleset) = state
        .database
        .replay_for_campaign_session(&run_id, ordinal)
        .await?;
    ensure_ruleset_is_public(&state.config, ruleset)?;
    replay_response(&state, digest, bytes).await
}

fn ensure_ruleset_is_public(config: &ServerConfig, ruleset: [u8; 32]) -> Result<(), ApiError> {
    let published = config
        .manifests
        .rulesets
        .get(&Digest32::from_bytes(ruleset))
        .ok_or(ApiError::Internal)?;
    if !matches!(
        published.operational_status,
        RulesetOperationalStatusV1::Active
    ) {
        return Err(ApiError::NotFound);
    }
    Ok(())
}

fn active_ruleset_ids(config: &ServerConfig) -> Vec<[u8; 32]> {
    config
        .manifests
        .rulesets
        .iter()
        .filter(|&(_digest, published)| {
            matches!(
                published.operational_status,
                RulesetOperationalStatusV1::Active
            )
        })
        .map(|(digest, _published)| digest.into_bytes())
        .collect()
}

async fn replay_response(
    state: &AppState,
    digest: [u8; 32],
    bytes: u64,
) -> Result<Response, ApiError> {
    let file = state.replay_store.open_verified(&digest, bytes).await?;
    let body = Body::from_stream(ReaderStream::new(file));
    let mut response = body.into_response();
    *response.status_mut() = StatusCode::OK;
    response.headers_mut().insert(
        CONTENT_TYPE,
        HeaderValue::from_static(RANKED_REPLAY_MEDIA_TYPE_V1),
    );
    response.headers_mut().insert(
        CONTENT_LENGTH,
        HeaderValue::from_str(&bytes.to_string()).map_err(|_| ApiError::Internal)?,
    );
    response.headers_mut().insert(
        CONTENT_DISPOSITION,
        HeaderValue::from_static("attachment; filename=\"verified-run.rhrec\""),
    );
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response.headers_mut().insert(
        axum::http::header::HeaderName::from_static("x-content-type-options"),
        HeaderValue::from_static("nosniff"),
    );
    Ok(response)
}

async fn username_challenge(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<UsernameChallengeRequestV1>,
) -> Result<(StatusCode, Json<UsernameChallengeV1>), ApiError> {
    rate_limit_challenge(&state, peer, &headers, ChallengePurpose::UsernameUpdate).await?;
    request.validate()?;
    let challenge = state
        .database
        .issue_challenge(
            ChallengePurpose::UsernameUpdate,
            request.public_key.into_bytes(),
            Duration::from_secs(state.config.challenge_ttl_seconds),
            None,
            None,
        )
        .await?;
    Ok((
        StatusCode::CREATED,
        Json(UsernameChallengeV1 {
            schema_version: SCHEMA_VERSION_V1,
            username_challenge_id: opaque(&challenge.id)?,
            username_challenge_nonce: challenge.nonce,
            expires_at_unix_ms: challenge.expires_at_ms,
        }),
    ))
}

async fn update_username(
    State(state): State<AppState>,
    Path(public_key): Path<String>,
    Json(update): Json<UsernameUpdateEnvelopeV1>,
) -> Result<Json<PlayerProfileV1>, ApiError> {
    update.validate()?;
    let path_key = PublicKey32::from_str(&public_key)
        .map_err(|error| ApiError::BadRequest(error.to_string()))?;
    if path_key != update.public_key {
        return Err(ApiError::BadRequest(
            "path public key does not match signed update".to_owned(),
        ));
    }
    let username = validate_username(&update.username)
        .map_err(|message| ApiError::BadRequest(message.to_owned()))?;
    verify_request_signature(
        update.public_key.as_bytes(),
        update.signature.as_bytes(),
        &update.signing_bytes()?,
    )?;
    state
        .database
        .apply_username_update(
            update.username_challenge_id.as_str(),
            update.username_challenge_nonce.into_bytes(),
            update.public_key.into_bytes(),
            &username,
        )
        .await?;
    Ok(Json(profile(update.public_key, username)))
}

async fn player_profile(
    State(state): State<AppState>,
    Path(public_key): Path<String>,
) -> Result<Json<PlayerProfileV1>, ApiError> {
    let key = PublicKey32::from_str(&public_key)
        .map_err(|error| ApiError::BadRequest(error.to_string()))?;
    let identity = state.database.public_identity(key.as_bytes()).await?;
    Ok(Json(profile(key, identity.username)))
}

async fn player_run_history(
    State(state): State<AppState>,
    Path(public_key): Path<String>,
    Query(query): Query<PlayerRunHistoryQueryV1>,
) -> Result<Json<PlayerRunHistoryPageV1>, ApiError> {
    query.validate()?;
    if u32::from(query.limit) > state.config.max_page_size {
        return Err(ApiError::BadRequest(format!(
            "limit exceeds server maximum {}",
            state.config.max_page_size
        )));
    }
    let key = PublicKey32::from_str(&public_key)
        .map_err(|error| ApiError::BadRequest(error.to_string()))?;
    let identity = state.database.public_identity(key.as_bytes()).await?;
    let query_sha256 = query
        .filter_for_player(key)
        .canonical_digest()
        .map_err(|error| ApiError::BadRequest(error.to_string()))?;
    let cursor = query
        .cursor
        .as_deref()
        .map(|value| decode_player_history_cursor(value, key, query_sha256, &state.cursor_hmac_key))
        .transpose()?;
    let visibility_revision = state.database.leaderboard_visibility_revision().await?;
    if cursor
        .as_ref()
        .is_some_and(|cursor| cursor.visibility_revision != visibility_revision)
    {
        return Err(ApiError::Conflict(
            "run history changed while paging; restart from the first page".to_owned(),
        ));
    }
    let accepted_sequence_watermark = match &cursor {
        Some(cursor) => cursor.accepted_sequence_watermark,
        None => state.database.accepted_sequence_watermark().await?,
    };
    let active_ruleset_ids = active_ruleset_ids(&state.config);
    let mut records = state
        .database
        .player_run_history(
            key.as_bytes(),
            &active_ruleset_ids,
            accepted_sequence_watermark,
            cursor
                .as_ref()
                .map(|cursor| (cursor.accepted_sequence, cursor.run_id.as_str())),
            u32::from(query.limit) + 1,
        )
        .await?;
    if state.database.leaderboard_visibility_revision().await? != visibility_revision {
        return Err(ApiError::Conflict(
            "run history changed while paging; restart from the first page".to_owned(),
        ));
    }
    let has_more = records.len() > usize::from(query.limit);
    records.truncate(usize::from(query.limit));
    let next_cursor = if has_more {
        let last = records.last().ok_or(ApiError::Internal)?;
        Some(encode_player_history_cursor(
            &PlayerHistoryCursorToken {
                player_public_key: key,
                query_sha256,
                accepted_sequence_watermark,
                visibility_revision,
                accepted_sequence: last.accepted_sequence,
                run_id: last.run_id.clone(),
            },
            &state.cursor_hmac_key,
        )?)
    } else {
        None
    };
    let mut runs = Vec::with_capacity(records.len());
    for record in records {
        let subject = match (&record.composition, record.mission_id) {
            (BoardComposition::Mission { .. }, Some(mission_id)) => LeaderboardSubjectV1::Mission {
                mission_id,
                category: category(&record.scope_kind)?,
            },
            (BoardComposition::FullCampaign { .. }, None) => LeaderboardSubjectV1::FullCampaign,
            _ => return Err(ApiError::Internal),
        };
        let composition = match record.composition {
            BoardComposition::Mission { replay_sha256 } => VerifiedRunCompositionV1::Mission {
                replay_sha256: Digest32::from_bytes(replay_sha256),
            },
            BoardComposition::FullCampaign {
                ordered_session_run_ids,
            } => VerifiedRunCompositionV1::FullCampaign {
                ordered_session_run_ids: ordered_session_run_ids
                    .iter()
                    .map(|run_id| opaque(run_id))
                    .collect::<Result<Vec<_>, _>>()?,
            },
        };
        let content = match &subject {
            LeaderboardSubjectV1::Mission { .. } => RunContentIdentityV1::Mission {
                content_manifest_sha256: Digest32::from_bytes(record.content_manifest_id),
            },
            LeaderboardSubjectV1::FullCampaign => RunContentIdentityV1::FullCampaign {
                campaign_content_manifest_sha256: Digest32::from_bytes(record.content_manifest_id),
            },
        };
        runs.push(PlayerRunHistoryEntryV1 {
            player_public_key: key,
            run: robin_run_protocol::RunSummaryV1 {
                schema_version: SCHEMA_VERSION_V1,
                run_id: opaque(&record.run_id)?,
                subject,
                composition,
                max_concurrent_players: record.max_concurrent_players,
                participant_instance_count: record.participant_instance_count,
                outcome: TerminalOutcomeV1::Won,
                metrics: RunMetricsV1 {
                    original_score_delta: record.original_score_delta,
                    active_simulation_ticks: record.active_simulation_ticks,
                    ransom_collected: record.ransom_collected,
                },
                content,
                rules_config_sha256: Digest32::from_bytes(record.config_id),
                ruleset_manifest_sha256: Digest32::from_bytes(record.ruleset_id),
                competition_manifest_sha256: record
                    .competition_manifest_id
                    .map(Digest32::from_bytes),
            },
            verified_at_unix_ms: record.verified_at_ms,
        });
    }
    let best_records = state
        .database
        .player_personal_bests(
            key.as_bytes(),
            &active_ruleset_ids,
            accepted_sequence_watermark,
        )
        .await?;
    let mut personal_bests = Vec::with_capacity(best_records.len());
    for best in best_records {
        let metric = metric(&best.metric)?;
        let ruleset = state
            .config
            .manifests
            .rulesets
            .get(&Digest32::from_bytes(best.ruleset_id))
            .ok_or(ApiError::Internal)?;
        let subject = match best.mission_id {
            Some(mission_id) => LeaderboardSubjectV1::Mission {
                mission_id,
                category: category(&best.scope_kind)?,
            },
            None => LeaderboardSubjectV1::FullCampaign,
        };
        let content = match &subject {
            LeaderboardSubjectV1::Mission { .. } => RunContentIdentityV1::Mission {
                content_manifest_sha256: Digest32::from_bytes(best.content_manifest_id),
            },
            LeaderboardSubjectV1::FullCampaign => RunContentIdentityV1::FullCampaign {
                campaign_content_manifest_sha256: Digest32::from_bytes(best.content_manifest_id),
            },
        };
        personal_bests.push(PlayerPersonalBestV1 {
            filter: RunFilterV1 {
                schema_version: SCHEMA_VERSION_V1,
                subject,
                metric,
                content,
                rules_config_sha256: Some(Digest32::from_bytes(best.config_id)),
                ruleset_manifest_sha256: Some(Digest32::from_bytes(best.ruleset_id)),
                competition_manifest_sha256: best.competition_manifest_id.map(Digest32::from_bytes),
                max_concurrent_players: Some(best.max_concurrent_players),
                player_public_key: Some(key),
            },
            run_id: opaque(&best.run_id)?,
            metric_value: board_metric_value(metric, best.value, &ruleset.manifest.tick_duration)?,
        });
    }
    let page = PlayerRunHistoryPageV1 {
        schema_version: SCHEMA_VERSION_V1,
        player: profile(key, identity.username),
        accepted_sequence_watermark,
        runs,
        personal_bests,
        next_cursor,
    };
    page.validate().map_err(|error| {
        tracing::error!(%error, "player history violated its public protocol");
        ApiError::Internal
    })?;
    Ok(Json(page))
}

async fn deletion_challenge(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<DeletionChallengeRequestV1>,
) -> Result<(StatusCode, Json<DeletionChallengeV1>), ApiError> {
    rate_limit_challenge(&state, peer, &headers, ChallengePurpose::Deletion).await?;
    request.validate()?;
    // Do not check target ownership before authentication. A check here would
    // let anyone submit candidate public keys and link an anonymous run to its
    // durable owner. The exact target and key are signed into the returned
    // challenge; apply_deletion performs the authoritative ownership check
    // after verifying that signature.
    let issued = state
        .database
        .issue_challenge(
            ChallengePurpose::Deletion,
            request.public_key.into_bytes(),
            Duration::from_secs(state.config.challenge_ttl_seconds),
            None,
            None,
        )
        .await?;
    let challenge = DeletionChallengeV1 {
        schema_version: SCHEMA_VERSION_V1,
        deletion_challenge_id: opaque(&issued.id)?,
        deletion_challenge_nonce: issued.nonce,
        expires_at_unix_ms: issued.expires_at_ms,
        public_key: request.public_key,
        target: request.target,
    };
    challenge
        .validate()
        .map_err(|error| configuration_error("deletion challenge", error))?;
    state
        .database
        .attach_deletion_challenge(
            &issued.id,
            &serde_json::to_string(&challenge).map_err(internal_json)?,
        )
        .await?;
    Ok((StatusCode::CREATED, Json(challenge)))
}

async fn deletion_request(
    State(state): State<AppState>,
    Json(request): Json<DeletionRequestEnvelopeV1>,
) -> Result<Json<DeletionReceiptV1>, ApiError> {
    request.validate()?;
    verify_request_signature(
        request.challenge.public_key.as_bytes(),
        request.signature.as_bytes(),
        &request.signing_bytes()?,
    )?;
    let now = crate::model::now_unix_ms()?;
    if request.challenge.expires_at_unix_ms < now {
        return Err(ApiError::Conflict(
            "deletion challenge has expired".to_owned(),
        ));
    }
    let challenge_json = serde_json::to_string(&request.challenge).map_err(internal_json)?;
    let request_json = serde_json::to_string(&request).map_err(internal_json)?;
    let (target_kind, target_id) = deletion_target_storage(&request.challenge.target);
    let retention = state
        .config
        .tombstone_retention_days
        .map(|days| {
            days.checked_mul(24 * 60 * 60)
                .map(Duration::from_secs)
                .ok_or(ApiError::Internal)
        })
        .transpose()?;
    let deleted = state
        .database
        .apply_deletion(
            request.challenge.deletion_challenge_id.as_str(),
            request.challenge.deletion_challenge_nonce.into_bytes(),
            request.challenge.public_key.into_bytes(),
            target_kind,
            target_id,
            &challenge_json,
            &request_json,
            retention,
        )
        .await?;
    Ok(Json(DeletionReceiptV1 {
        schema_version: SCHEMA_VERSION_V1,
        deletion_request_id: opaque(&deleted.id)?,
        target: request.challenge.target,
        tombstoned_at_unix_ms: deleted.tombstoned_at_ms,
        purge_eligible_at_unix_ms: deleted.purge_eligible_at_ms,
    }))
}

async fn abuse_report(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(report): Json<AbuseReportV1>,
) -> Result<(StatusCode, Json<AbuseReportAcceptedV1>), ApiError> {
    report.validate()?;
    let (target_kind, target_id) = match &report.target {
        AbuseReportTargetV1::Run { run_id } => ("run", run_id.as_str().to_owned()),
        AbuseReportTargetV1::Player { public_key } => ("player", public_key.to_string()),
    };
    let category = match report.category {
        AbuseReportCategoryV1::SuspectedCheating => "suspected_cheating",
        AbuseReportCategoryV1::OffensiveIdentity => "offensive_identity",
        AbuseReportCategoryV1::Privacy => "privacy",
        AbuseReportCategoryV1::Copyright => "copyright",
        AbuseReportCategoryV1::Other => "other",
    };
    let address = effective_client_ip(&state.config, peer, &headers)?;
    let active_ruleset_ids = active_ruleset_ids(&state.config);
    let reporter_ip_hash =
        crate::authentication::sign(&state.cursor_hmac_key, address.to_string().as_bytes());
    let (report_id, received_at_unix_ms) = state
        .database
        .insert_abuse_report(
            target_kind,
            &target_id,
            &active_ruleset_ids,
            category,
            &report.detail,
            reporter_ip_hash,
            state.config.abuse_reports_per_hour_per_ip,
            state.config.abuse_reports_per_hour_per_key,
            state.config.abuse_reports_per_hour_per_target,
        )
        .await?;
    Ok((
        StatusCode::ACCEPTED,
        Json(AbuseReportAcceptedV1 {
            schema_version: SCHEMA_VERSION_V1,
            report_id: opaque(&report_id)?,
            received_at_unix_ms,
        }),
    ))
}

fn deletion_target_storage(target: &DeletionTargetV1) -> (&'static str, &str) {
    match target {
        DeletionTargetV1::Submission { submission_id } => ("submission", submission_id.as_str()),
        DeletionTargetV1::Run { run_id } => ("run", run_id.as_str()),
    }
}

fn profile(public_key: PublicKey32, username: String) -> PlayerProfileV1 {
    PlayerProfileV1 {
        schema_version: SCHEMA_VERSION_V1,
        username,
        public_key,
        public_key_fingerprint: public_key.short_fingerprint(),
    }
}

fn viewer_launch(
    config: &ServerConfig,
    stored_profile: &AdmissionProfile,
    build_manifest_id: [u8; 32],
    content_manifest_id: [u8; 32],
) -> Result<ViewerLaunchV1, ApiError> {
    let build_digest = Digest32::from_bytes(build_manifest_id);
    let content_digest = Digest32::from_bytes(content_manifest_id);
    let unavailable = |safe_reason: String| ViewerLaunchV1 {
        build_manifest_sha256: build_digest,
        availability: ViewerAvailabilityV1::Unavailable { safe_reason },
    };
    let Some(profile) = config
        .admission_profiles
        .iter()
        .find(|profile| profile.id == stored_profile.id)
    else {
        return Ok(unavailable(
            "The authenticated viewer is no longer published for this run.".to_owned(),
        ));
    };
    if profile.build_manifest_id != build_digest.to_string()
        || profile.content_manifest_id != content_digest.to_string()
        || stored_profile.build_manifest_id != profile.build_manifest_id
        || stored_profile.content_manifest_id != profile.content_manifest_id
        || stored_profile.config_id != profile.config_id
        || stored_profile.ruleset_id != profile.ruleset_id
        || stored_profile.content_subject != profile.content_subject
    {
        return Ok(unavailable(
            "The authenticated viewer does not match this run's immutable artifacts.".to_owned(),
        ));
    }
    if !profile.viewer_available {
        let safe_reason = profile.viewer_unavailable_reason.clone().ok_or_else(|| {
            tracing::error!(
                profile_id = %profile.id,
                "viewer is disabled without its required configured reason"
            );
            ApiError::Internal
        })?;
        return Ok(unavailable(safe_reason));
    }
    let Some(build) = config.manifests.builds.get(&build_digest) else {
        return Ok(unavailable(
            "The authenticated viewer build is not published.".to_owned(),
        ));
    };
    let Some(content) = config.manifests.content_manifests.get(&content_digest) else {
        return Ok(unavailable(
            "The authenticated viewer content manifest is not published.".to_owned(),
        ));
    };
    if build.public_digest() != build_digest
        || build.public_document().validate().is_err()
        || build.semantics().validate().is_err()
        || build.semantics().viewer_artifacts.is_empty()
        || content.validate().is_err()
        || content.canonical_digest().ok() != Some(content_digest)
    {
        return Ok(unavailable(
            "The authenticated viewer artifacts are incomplete.".to_owned(),
        ));
    }
    let content_requirement = match (content.edition, profile.viewer_content_requirement) {
        (
            robin_run_protocol::OfficialContentEditionV1::Demo,
            Some(ViewerContentRequirementConfig::BundledDemo),
        ) => ViewerContentRequirementV1::BundledDemo {
            content_manifest_sha256: content_digest,
        },
        (
            robin_run_protocol::OfficialContentEditionV1::Full,
            Some(ViewerContentRequirementConfig::UserLocalRetail),
        ) => ViewerContentRequirementV1::UserLocalRetail {
            content_manifest_sha256: content_digest,
        },
        _ => {
            tracing::error!(
                profile_id = %profile.id,
                content_edition = ?content.edition,
                configured_requirement = ?profile.viewer_content_requirement,
                "viewer content requirement does not match authenticated content edition"
            );
            return Ok(unavailable(
                "The authenticated viewer content requirement is unavailable.".to_owned(),
            ));
        }
    };
    let launch = ViewerLaunchV1 {
        build_manifest_sha256: build_digest,
        availability: ViewerAvailabilityV1::Available {
            content_requirement,
        },
    };
    launch.validate().map_err(|_error| {
        tracing::error!(
            error_code = "viewer_launch_invalid",
            "configured viewer launch is invalid"
        );
        ApiError::Internal
    })?;
    Ok(launch)
}

async fn lifecycle_state(
    state: &AppState,
    lifecycle: crate::model::SubmissionLifecycle,
) -> Result<SubmissionLifecycleV1, ApiError> {
    use crate::model::SubmissionState;
    match lifecycle.state {
        SubmissionState::Queued => Ok(SubmissionLifecycleV1::Queued),
        SubmissionState::Verifying => Ok(SubmissionLifecycleV1::Verifying),
        SubmissionState::RetryPending => Ok(SubmissionLifecycleV1::RetryPending),
        SubmissionState::Accepted { run_id } => {
            let campaign_chain_receipt = campaign_receipt(state, run_id.as_str()).await?;
            Ok(SubmissionLifecycleV1::Accepted {
                run_id,
                campaign_chain_receipt,
            })
        }
        SubmissionState::Rejected { code } => {
            Ok(SubmissionLifecycleV1::Rejected {
                code,
                safe_message: safe_rejection_message(code).to_owned(),
            })
        }
        SubmissionState::Failed => Ok(SubmissionLifecycleV1::Failed {
            code: SubmissionFailureCodeV1::VerificationInfrastructure,
            safe_message:
                "Verification infrastructure failed after bounded retries; the run was not rejected."
                    .to_owned(),
        }),
    }
}

async fn campaign_receipt(
    state: &AppState,
    run_id: &str,
) -> Result<Option<CampaignChainReceiptV1>, ApiError> {
    let Some(private) = state
        .database
        .owner_campaign_receipt_context(run_id)
        .await?
    else {
        return Ok(None);
    };
    let ruleset_sha256 = Digest32::from_bytes(private.ruleset_id);
    let published = state
        .config
        .manifests
        .rulesets
        .get(&ruleset_sha256)
        .ok_or(ApiError::Internal)?;
    let campaign_content_manifest_sha256 =
        Digest32::from_bytes(private.campaign_content_manifest_id);
    let campaign_content = state
        .config
        .manifests
        .campaign_content_manifests
        .get(&campaign_content_manifest_sha256)
        .ok_or(ApiError::Internal)?;
    private
        .verification_result
        .validate_campaign_complete_evidence(
            &private.verification_request,
            &published.manifest,
            Some(campaign_content),
        )
        .map_err(|_error| {
            tracing::error!(
                error_code = "owner_campaign_receipt_private_proof_invalid",
                "retained private campaign proof failed exact validation"
            );
            ApiError::Internal
        })?;
    Ok(Some(CampaignChainReceiptV1 {
        schema_version: SCHEMA_VERSION_V1,
        chain_id: opaque(&private.chain_id)?,
        predecessor_run_id: opaque(run_id)?,
        predecessor_verification_sha256: private.predecessor_verification_sha256,
        expected_starting_campaign: private.final_campaign,
        rules_config_sha256: Digest32::from_bytes(private.config_id),
        ruleset_manifest_sha256: ruleset_sha256,
        competition_manifest_sha256: private.competition_manifest_id.map(Digest32::from_bytes),
        campaign_content_manifest_sha256,
        expected_max_concurrent_players: private.max_concurrent_players,
        participant_public_keys: private.participant_public_keys,
        campaign_controller_public_key: private.controller_public_key,
        state: match &private.completed_full_campaign_run_id {
            Some(full_campaign_run_id) => CampaignChainStateV1::Complete {
                full_campaign_run_id: opaque(full_campaign_run_id)?,
            },
            None => CampaignChainStateV1::Active,
        },
    }))
}

fn fresh_start_artifact(
    config: &ServerConfig,
    profile: &AdmissionProfile,
    ranked: &robin_run_protocol::RankedSessionConfigV1,
) -> Result<ArtifactRefV1, ApiError> {
    let published = config
        .manifests
        .rulesets
        .get(&ranked.ruleset_manifest_sha256)
        .ok_or(ApiError::NotFound)?;
    if published
        .manifest
        .canonical_start_policy
        .requires_exact_operator_artifact()
    {
        return Ok(profile.canonical_campaign_state.artifact.clone());
    }
    // The immutable policy delegates mission setup validation to replay
    // verification. This grant binds a proposal; it does not award a score.
    Ok(ArtifactRefV1 {
        sha256: ranked.starting_campaign_sha256,
        byte_length: ranked.starting_campaign_byte_length,
        media_type: RANKED_CAMPAIGN_MEDIA_TYPE_V1.into(),
    })
}

async fn starting_state(
    state: &AppState,
    request: &SubmissionOfferRequestV1,
    profile: &AdmissionProfile,
) -> Result<InitialStateExpectationV1, ApiError> {
    let fresh = fresh_start_artifact(
        &state.config,
        profile,
        &request.session_genesis.claim.ranked_session,
    )?;
    match &request.scope_request {
        ScopeRequestV1::IndividualLevel => Ok(InitialStateExpectationV1::IndividualLevel {
            template_id: opaque(&profile.template_id)?,
            campaign_state_requirement: profile.canonical_campaign_state.requirement,
            campaign_sha256: fresh.sha256,
            starting_campaign_byte_length: fresh.byte_length,
        }),
        ScopeRequestV1::CampaignGenesis => Ok(InitialStateExpectationV1::CampaignGenesis {
            template_id: opaque(&profile.template_id)?,
            campaign_state_requirement: profile.canonical_campaign_state.requirement,
            campaign_sha256: fresh.sha256,
            starting_campaign_byte_length: fresh.byte_length,
        }),
        ScopeRequestV1::CampaignContinuation {
            chain_id,
            predecessor_run_id,
        } => {
            let predecessor = state
                .database
                .campaign_predecessor(predecessor_run_id.as_str())
                .await?;
            let published = state
                .config
                .manifests
                .rulesets
                .get(&digest(&profile.ruleset_id)?)
                .ok_or(ApiError::Internal)?;
            let claims = request
                .participant_claims
                .iter()
                .map(|claim| ParticipantClaim {
                    seat: claim.seat,
                    participant_instance_id: claim.participant_instance_id.into_bytes(),
                    public_key: claim.public_key.into_bytes(),
                    public_disclosure: match claim.public_disclosure {
                        ParticipantPublicDisclosureV1::NamedProfile => "named_profile",
                        ParticipantPublicDisclosureV1::Anonymous => "anonymous",
                    }
                    .to_owned(),
                })
                .collect::<Vec<_>>();
            let mut requested_keys = claims
                .iter()
                .map(|claim| claim.public_key)
                .collect::<Vec<_>>();
            requested_keys.sort_unstable();
            let mut predecessor_keys = predecessor
                .participants
                .iter()
                .map(|claim| claim.public_key)
                .collect::<Vec<_>>();
            predecessor_keys.sort_unstable();
            let claims_match = match published.manifest.campaign_roster_continuity {
                CampaignRosterContinuityV1::ExactSameAuthenticatedKeysEverySession => {
                    requested_keys == predecessor_keys
                }
                CampaignRosterContinuityV1::UnionOfVerifiedSessionSubsets => true,
            };
            if predecessor.chain_id != chain_id.as_str()
                || predecessor.max_concurrent_players != request.max_concurrent_players
                || !claims_match
                || !continuation_owner_authorized(
                    predecessor.chain_owner_public_key,
                    &request.participant_claims,
                )
                || predecessor.campaign_content_manifest_id
                    != digest(
                        profile
                            .campaign_content_manifest_id
                            .as_deref()
                            .ok_or(ApiError::Internal)?,
                    )?
                    .into_bytes()
                || predecessor.rules_config_id != digest(&profile.config_id)?.into_bytes()
                || predecessor.ruleset_id != digest(&profile.ruleset_id)?.into_bytes()
                || predecessor.competition_manifest_id
                    != request
                        .competition_manifest_sha256
                        .map(Digest32::into_bytes)
            {
                return Err(ApiError::Conflict(
                    "campaign continuation does not match its verified predecessor".to_owned(),
                ));
            }
            Ok(InitialStateExpectationV1::CampaignContinuation {
                chain_id: chain_id.clone(),
                predecessor_run_id: predecessor_run_id.clone(),
                predecessor_verification_sha256: Digest32::from_bytes(predecessor.result_sha256),
                campaign_state_requirement: profile.canonical_campaign_state.requirement,
                campaign_sha256: Digest32::from_bytes(predecessor.final_campaign_sha256),
                starting_campaign_byte_length: predecessor.final_campaign_bytes,
            })
        }
    }
}

fn continuation_owner_authorized(
    owner: [u8; 32],
    claims: &[robin_run_protocol::ParticipantClaimV1],
) -> bool {
    claims
        .iter()
        .any(|claim| claim.public_key.as_bytes() == &owner)
}

fn profile_with_session_config(
    config: &ServerConfig,
    profile: &AdmissionProfile,
    ranked: &robin_run_protocol::RankedSessionConfigV1,
) -> Result<AdmissionProfile, ApiError> {
    ranked.validate()?;
    let published = config
        .manifests
        .rulesets
        .get(&ranked.ruleset_manifest_sha256)
        .ok_or(ApiError::NotFound)?;
    let mut derived = profile.clone();
    match (
        published.manifest.rules_config_constraint,
        &ranked.custom_rules_config,
    ) {
        (robin_run_protocol::RulesConfigConstraintV1::ExactCanonicalDigestOnly, None) => {}
        (robin_run_protocol::RulesConfigConstraintV1::AnyCanonicalSimConfig, Some(custom)) => {
            robin_engine::simulation_inputs::validate_ranked_simulation_policy_rules_config_v1(
                custom,
            )
            .map_err(|error| ApiError::BadRequest(error.to_string()))?;
            let baseline = config
                .manifests
                .rules_configs
                .get(&published.manifest.rules_config_sha256)
                .ok_or(ApiError::Internal)?;
            if custom.rules != baseline.rules
                || custom.replay_schema_version != baseline.replay_schema_version
            {
                return Err(ApiError::BadRequest(
                    "custom configuration changes unsupported ranking rules".to_owned(),
                ));
            }
            derived.config_id = ranked.rules_config_sha256.to_string();
            derived.canonical_campaign_state_path = None;
            derived
                .canonical_campaign_state
                .requirement
                .rules_config_sha256 = ranked.rules_config_sha256;
            derived.canonical_campaign_state.artifact =
                ranked.custom_canonical_campaign.clone().ok_or_else(|| {
                    ApiError::BadRequest(
                        "custom session has no canonical campaign proposal".to_owned(),
                    )
                })?;
        }
        _ => {
            return Err(ApiError::BadRequest(
                "ruleset does not admit this session configuration".to_owned(),
            ));
        }
    }
    Ok(derived)
}

fn matching_admission_profiles<'a>(
    config: &'a ServerConfig,
    mission_id: &str,
    subject: &robin_run_protocol::OfficialContentSubjectV1,
    ruleset: Digest32,
    scope: &str,
    required_profile: Option<&str>,
) -> Vec<&'a AdmissionProfile> {
    let ruleset = ruleset.to_string();
    config
        .admission_profiles
        .iter()
        .filter(|profile| {
            profile.mission_id() == mission_id
                && &profile.content_subject == subject
                && profile.ruleset_id == ruleset
                && profile
                    .allowed_scopes
                    .iter()
                    .any(|allowed| allowed == scope)
                && required_profile.is_none_or(|required| profile.id == required)
        })
        .collect()
}

fn active_competition_by_digest(
    config: &ServerConfig,
    requested: Digest32,
) -> Result<(&CompetitionConfig, CompetitionManifestV1), ApiError> {
    let (competition, manifest) = competition_by_digest(config, requested)?;
    let now = crate::model::now_unix_ms()?;
    if !(manifest.starts_at_unix_ms..manifest.ends_at_unix_ms).contains(&now) {
        return Err(ApiError::Conflict("competition is not active".to_owned()));
    }
    Ok((competition, manifest))
}

fn select_fresh_run_profile<'a>(
    config: &'a ServerConfig,
    request: &FreshRunPreflightRequestV1,
) -> Result<&'a AdmissionProfile, ApiError> {
    let ranked = &request.claim.ranked_session;
    let scope = match request.claim.scope {
        FreshRunScopeV1::IndividualLevel => "individual_level",
        FreshRunScopeV1::CampaignGenesis => "campaign_genesis",
    };
    let competition_profile = ranked
        .competition_manifest_sha256
        .map(|digest| {
            let (competition, manifest) = active_competition_by_digest(config, digest)?;
            manifest
                .validate_ranked_session(ranked)
                .map_err(|error| ApiError::Conflict(error.to_string()))?;
            Ok(competition.admission_profile_id.as_str())
        })
        .transpose()?;
    let profile = matching_admission_profiles(
        config,
        ranked.mission_id.as_str(),
        &ranked.content_subject,
        ranked.ruleset_manifest_sha256,
        scope,
        competition_profile,
    )
    .into_iter()
    .next()
    .ok_or(ApiError::NotFound)?;
    let published = config
        .manifests
        .rulesets
        .get(&ranked.ruleset_manifest_sha256)
        .ok_or(ApiError::NotFound)?;
    if !matches!(
        published.operational_status,
        RulesetOperationalStatusV1::Active
    ) {
        return Err(ApiError::NotFound);
    }
    Ok(profile)
}

fn validate_fresh_run_preflight_profile(
    config: &ServerConfig,
    request: &FreshRunPreflightRequestV1,
    profile: &AdmissionProfile,
) -> Result<(), ApiError> {
    let ranked = &request.claim.ranked_session;
    let fresh = fresh_start_artifact(config, profile, ranked)?;
    let expected_campaign_content = match request.claim.scope {
        FreshRunScopeV1::IndividualLevel => None,
        FreshRunScopeV1::CampaignGenesis => Some(digest(
            profile
                .campaign_content_manifest_id
                .as_deref()
                .ok_or(ApiError::Internal)?,
        )?),
    };
    let expected_start = match request.claim.scope {
        FreshRunScopeV1::IndividualLevel => InitialStateExpectationV1::IndividualLevel {
            template_id: opaque(&profile.template_id)?,
            campaign_state_requirement: profile.canonical_campaign_state.requirement,
            campaign_sha256: fresh.sha256,
            starting_campaign_byte_length: fresh.byte_length,
        },
        FreshRunScopeV1::CampaignGenesis => InitialStateExpectationV1::CampaignGenesis {
            template_id: opaque(&profile.template_id)?,
            campaign_state_requirement: profile.canonical_campaign_state.requirement,
            campaign_sha256: fresh.sha256,
            starting_campaign_byte_length: fresh.byte_length,
        },
    };
    robin_run_protocol::validate_official_ranked_scope_subject_v1(
        ranked.content_edition,
        &ranked.content_subject,
        &expected_start,
    )
    .map_err(|error| ApiError::Conflict(error.to_string()))?;
    let content_manifest_sha256 = digest(&profile.content_manifest_id)?;
    let content = config
        .manifests
        .content_manifests
        .get(&content_manifest_sha256)
        .ok_or(ApiError::Internal)?;
    ranked
        .validate_content_manifest(content)
        .map_err(|error| ApiError::Conflict(error.to_string()))?;
    if request.claim.starting_campaign.media_type != RANKED_CAMPAIGN_MEDIA_TYPE_V1
        || request.claim.starting_campaign != fresh
        || profile
            .canonical_campaign_state
            .requirement
            .rules_config_sha256
            != ranked.rules_config_sha256
        || profile.canonical_campaign_state.requirement.edition != ranked.content_edition
        || ranked.build_manifest_sha256 != digest(&profile.build_manifest_id)?
        || ranked.content_manifest_sha256 != content_manifest_sha256
        || ranked.campaign_content_manifest_sha256 != expected_campaign_content
        || ranked.rules_config_sha256 != digest(&profile.config_id)?
        || ranked.ruleset_manifest_sha256 != digest(&profile.ruleset_id)?
    {
        return Err(ApiError::Conflict(
            "fresh-run preflight does not match the selected immutable profile".to_owned(),
        ));
    }
    Ok(())
}

fn select_continuation_preflight_profile<'a>(
    config: &'a ServerConfig,
    request: &CampaignContinuationPreflightRequestV1,
) -> Result<&'a AdmissionProfile, ApiError> {
    let ranked = &request.claim.ranked_session;
    let competition_profile = ranked
        .competition_manifest_sha256
        .map(|digest| {
            let (competition, manifest) = active_competition_by_digest(config, digest)?;
            manifest
                .validate_ranked_session(ranked)
                .map_err(|error| ApiError::Conflict(error.to_string()))?;
            Ok(competition.admission_profile_id.as_str())
        })
        .transpose()?;
    let profile = matching_admission_profiles(
        config,
        ranked.mission_id.as_str(),
        &ranked.content_subject,
        ranked.ruleset_manifest_sha256,
        "campaign_continuation",
        competition_profile,
    )
    .into_iter()
    .next()
    .ok_or(ApiError::NotFound)?;
    let published = config
        .manifests
        .rulesets
        .get(&ranked.ruleset_manifest_sha256)
        .ok_or(ApiError::NotFound)?;
    if !matches!(
        published.operational_status,
        RulesetOperationalStatusV1::Active
    ) {
        return Err(ApiError::NotFound);
    }
    Ok(profile)
}

fn validate_continuation_preflight_profile(
    config: &ServerConfig,
    request: &CampaignContinuationPreflightRequestV1,
    profile: &AdmissionProfile,
    predecessor: &crate::db::CampaignPredecessor,
) -> Result<(), ApiError> {
    let ranked = &request.claim.ranked_session;
    let content_manifest_sha256 = digest(&profile.content_manifest_id)?;
    let content = config
        .manifests
        .content_manifests
        .get(&content_manifest_sha256)
        .ok_or(ApiError::Internal)?;
    ranked
        .validate_content_manifest(content)
        .map_err(|error| ApiError::Conflict(error.to_string()))?;
    let campaign_content_manifest_sha256 = digest(
        profile
            .campaign_content_manifest_id
            .as_deref()
            .ok_or(ApiError::Internal)?,
    )?;
    if request.claim.starting_campaign.media_type != RANKED_CAMPAIGN_MEDIA_TYPE_V1
        || request.claim.starting_campaign.sha256.into_bytes() != predecessor.final_campaign_sha256
        || request.claim.starting_campaign.byte_length != predecessor.final_campaign_bytes
        || profile
            .canonical_campaign_state
            .requirement
            .rules_config_sha256
            != ranked.rules_config_sha256
        || profile.canonical_campaign_state.requirement.edition != ranked.content_edition
        || ranked.build_manifest_sha256 != digest(&profile.build_manifest_id)?
        || ranked.content_manifest_sha256 != content_manifest_sha256
        || ranked.campaign_content_manifest_sha256 != Some(campaign_content_manifest_sha256)
        || ranked.rules_config_sha256 != digest(&profile.config_id)?
        || ranked.ruleset_manifest_sha256 != digest(&profile.ruleset_id)?
        || predecessor.campaign_content_manifest_id != campaign_content_manifest_sha256.into_bytes()
        || predecessor.rules_config_id != ranked.rules_config_sha256.into_bytes()
        || predecessor.ruleset_id != ranked.ruleset_manifest_sha256.into_bytes()
        || predecessor.competition_manifest_id
            != ranked.competition_manifest_sha256.map(Digest32::into_bytes)
    {
        return Err(ApiError::Conflict(
            "continuation preflight does not match the selected immutable profile".to_owned(),
        ));
    }
    Ok(())
}

fn select_profile<'a>(
    config: &'a ServerConfig,
    request: &SubmissionOfferRequestV1,
    scope: &str,
) -> Result<&'a AdmissionProfile, ApiError> {
    let competition_profile = request
        .competition_manifest_sha256
        .as_ref()
        .map(|requested_digest| {
            let (competition, manifest) = active_competition_by_digest(config, *requested_digest)?;
            let category_matches = matches!(
                (scope, &manifest.subject),
                (
                    "individual_level",
                    LeaderboardSubjectV1::Mission {
                        category: BoardCategoryV1::IndividualLevel,
                        ..
                    },
                ) | (
                    "campaign_genesis" | "campaign_continuation",
                    LeaderboardSubjectV1::Mission {
                        category: BoardCategoryV1::Campaign,
                        ..
                    } | LeaderboardSubjectV1::FullCampaign,
                )
            );
            if !category_matches {
                return Err(ApiError::BadRequest(
                    "competition category does not match the requested run scope".to_owned(),
                ));
            }
            Ok(competition.admission_profile_id.as_str())
        })
        .transpose()?;
    let matches = matching_admission_profiles(
        config,
        request.mission_id.as_str(),
        &request.session_genesis.claim.ranked_session.content_subject,
        request.ruleset_manifest_sha256,
        scope,
        competition_profile,
    );
    let profile = match matches.as_slice() {
        [profile] => *profile,
        [] => Err(ApiError::BadRequest(
            "mission, scope, and ruleset are not currently eligible".to_owned(),
        ))?,
        _ => {
            tracing::error!(mission = %request.mission_id, scope, "ambiguous admission profiles");
            return Err(ApiError::Internal);
        }
    };
    let published = config
        .manifests
        .rulesets
        .get(&request.ruleset_manifest_sha256)
        .ok_or_else(|| ApiError::BadRequest("unknown immutable ruleset".to_owned()))?;
    if !matches!(
        published.operational_status,
        RulesetOperationalStatusV1::Active
    ) {
        return Err(ApiError::Conflict(
            "the selected ruleset is quarantined".to_owned(),
        ));
    }
    let manifest = &published.manifest;
    let board_scope = match scope {
        "individual_level" => RulesetBoardScopeV1::IndividualLevel,
        "campaign_genesis" | "campaign_continuation" => RulesetBoardScopeV1::CampaignMission,
        _ => return Err(ApiError::Internal),
    };
    let named = u16::try_from(
        request
            .participant_claims
            .iter()
            .filter(|claim| claim.public_disclosure == ParticipantPublicDisclosureV1::NamedProfile)
            .count(),
    )
    .map_err(|_| ApiError::Internal)?;
    let anonymous = u16::try_from(
        request
            .participant_claims
            .iter()
            .filter(|claim| claim.public_disclosure == ParticipantPublicDisclosureV1::Anonymous)
            .count(),
    )
    .map_err(|_| ApiError::Internal)?;
    if named.checked_add(anonymous) != Some(request.participant_instance_count) {
        return Err(ApiError::BadRequest(
            "participant disclosure counts are inconsistent".to_owned(),
        ));
    }
    let participant_policy = &manifest.participant_eligibility;
    let mode_allowed = if request.max_concurrent_players == 1 {
        participant_policy.allow_single_player
    } else {
        participant_policy.allow_multiplayer
    };
    if !manifest.admits_rules_config_digest(
        request
            .session_genesis
            .claim
            .ranked_session
            .rules_config_sha256,
    ) || manifest
        .allowed_build_manifest_sha256
        .binary_search(
            &request
                .session_genesis
                .claim
                .ranked_session
                .build_manifest_sha256,
        )
        .is_err()
        || manifest
            .allowed_content_manifest_sha256
            .binary_search(
                &request
                    .session_genesis
                    .claim
                    .ranked_session
                    .content_manifest_sha256,
            )
            .is_err()
        || manifest.board_scopes.binary_search(&board_scope).is_err()
        || request.max_concurrent_players < participant_policy.minimum_max_concurrent_players
        || request.max_concurrent_players > participant_policy.maximum_max_concurrent_players
        || request.participant_instance_count > participant_policy.maximum_participant_instances
        || !mode_allowed
        || (participant_policy.anonymous_policy == AnonymousParticipantPolicyV1::Forbidden
            && anonymous != 0)
    {
        return Err(ApiError::BadRequest(
            "request does not satisfy the immutable ruleset policy".to_owned(),
        ));
    }
    Ok(profile)
}

fn profile_for_filter<'a>(
    config: &'a ServerConfig,
    filter: &RunFilterV1,
) -> Result<&'a AdmissionProfile, ApiError> {
    let profile_matches = |profile: &&AdmissionProfile| {
        let subject_matches = match &filter.subject {
            LeaderboardSubjectV1::Mission {
                mission_id,
                category,
            } => {
                profile.mission_id() == mission_id
                    && profile.allowed_scopes.iter().any(|scope| match category {
                        BoardCategoryV1::IndividualLevel => scope == "individual_level",
                        BoardCategoryV1::Campaign => {
                            scope == "campaign_genesis" || scope == "campaign_continuation"
                        }
                    })
            }
            LeaderboardSubjectV1::FullCampaign => profile
                .allowed_scopes
                .iter()
                .any(|scope| scope == "campaign_genesis"),
        };
        let content_matches = match filter.content {
            RunContentIdentityV1::Mission {
                content_manifest_sha256,
            } => profile.content_manifest_id == content_manifest_sha256.to_string(),
            RunContentIdentityV1::FullCampaign {
                campaign_content_manifest_sha256,
            } => profile
                .campaign_content_manifest_id
                .as_ref()
                .is_some_and(|configured| {
                    configured == &campaign_content_manifest_sha256.to_string()
                }),
        };
        let active = digest(&profile.ruleset_id)
            .ok()
            .and_then(|id| config.manifests.rulesets.get(&id))
            .is_some_and(|published| {
                matches!(
                    published.operational_status,
                    RulesetOperationalStatusV1::Active
                )
            });
        active && subject_matches
            && content_matches
            && filter.rules_config_sha256.is_none_or(|value| profile.config_id == value.to_string()
                || digest(&profile.ruleset_id).ok().and_then(|id| config.manifests.rulesets.get(&id))
                    .is_some_and(|published| published.manifest.rules_config_constraint == robin_run_protocol::RulesConfigConstraintV1::AnyCanonicalSimConfig))
            && filter.ruleset_manifest_sha256.is_none_or(|value| profile.ruleset_id == value.to_string())
            && metrics(profile).is_ok_and(|metrics| metrics.contains(&filter.metric))
    };
    let profile = if let Some(competition_digest) = filter.competition_manifest_sha256 {
        let (competition, manifest) =
            competition_by_digest(config, competition_digest).map_err(|_| ApiError::NotFound)?;
        let profile = config
            .admission_profiles
            .iter()
            .find(|profile| profile.id == competition.admission_profile_id)
            .filter(profile_matches)
            .ok_or(ApiError::NotFound)?;
        if filter.subject != manifest.subject
            || manifest.metric != filter.metric
            || manifest.content != filter.content
            || Some(manifest.rules_config_sha256) != filter.rules_config_sha256
            || Some(manifest.ruleset_manifest_sha256) != filter.ruleset_manifest_sha256
            || filter.max_concurrent_players
                != Some(manifest.participant_composition.max_concurrent_players())
        {
            return Err(ApiError::NotFound);
        }
        profile
    } else {
        config
            .admission_profiles
            .iter()
            .find(profile_matches)
            .ok_or(ApiError::NotFound)?
    };
    let published = config
        .manifests
        .rulesets
        .get(&digest(&profile.ruleset_id)?)
        .ok_or(ApiError::NotFound)?;
    if !matches!(
        published.operational_status,
        RulesetOperationalStatusV1::Active
    ) {
        return Err(ApiError::NotFound);
    }
    Ok(profile)
}

fn allowed_metrics_for_offer(
    config: &ServerConfig,
    request: &SubmissionOfferRequestV1,
    profile: &AdmissionProfile,
) -> Result<Vec<BoardMetricV1>, ApiError> {
    match request.competition_manifest_sha256 {
        Some(digest) => {
            let (_, manifest) = competition_by_digest(config, digest)?;
            Ok(vec![manifest.metric])
        }
        None => metrics(profile),
    }
}

fn ruleset_facet(profile: &AdmissionProfile) -> Result<RulesetFacetV1, ApiError> {
    let mut categories = profile
        .allowed_scopes
        .iter()
        .map(|scope| match scope.as_str() {
            "individual_level" => Ok(BoardCategoryV1::IndividualLevel),
            "campaign_genesis" | "campaign_continuation" => Ok(BoardCategoryV1::Campaign),
            _ => Err(ApiError::Internal),
        })
        .collect::<Result<Vec<_>, _>>()?;
    categories.sort();
    categories.dedup();
    // Keep the mission facet even when this profile also starts a full campaign.
    let supports_full_campaign_boards = false;
    let content = RunContentIdentityV1::Mission {
        content_manifest_sha256: digest(&profile.content_manifest_id)?,
    };
    Ok(RulesetFacetV1 {
        ruleset_manifest_sha256: digest(&profile.ruleset_id)?,
        rules_config_sha256: digest(&profile.config_id)?,
        display_name: profile.ruleset_display_name.clone(),
        preset_id: opaque(&profile.preset_id)?,
        preset_name: profile.preset_name.clone(),
        difficulty_id: opaque(&profile.difficulty_id)?,
        difficulty_name: profile.difficulty_name.clone(),
        content,
        categories,
        metrics: metrics(profile)?,
        supports_full_campaign_boards,
    })
}

fn competition_summary(
    config: &ServerConfig,
    competition: &CompetitionConfig,
    now: u64,
) -> Result<CompetitionSummaryV1, ApiError> {
    let competition_manifest_sha256 = digest(&competition.manifest_sha256)?;
    let manifest = config
        .manifests
        .competitions
        .get(&competition_manifest_sha256)
        .cloned()
        .ok_or(ApiError::Internal)?;
    Ok(CompetitionSummaryV1 {
        competition_manifest_sha256,
        state: if now < manifest.starts_at_unix_ms {
            CompetitionStateV1::Upcoming
        } else if now < manifest.ends_at_unix_ms {
            CompetitionStateV1::Active
        } else {
            CompetitionStateV1::Ended
        },
        manifest,
    })
}

fn competition_by_digest(
    config: &ServerConfig,
    requested: Digest32,
) -> Result<(&CompetitionConfig, CompetitionManifestV1), ApiError> {
    let competition = config
        .competitions
        .iter()
        .find(|competition| competition.manifest_sha256 == requested.to_string())
        .ok_or_else(|| ApiError::BadRequest("unknown competition manifest".to_owned()))?;
    let manifest = config
        .manifests
        .competitions
        .get(&requested)
        .cloned()
        .ok_or(ApiError::Internal)?;
    Ok((competition, manifest))
}

fn board_entry(
    row: &BoardRow,
    position: u64,
    metric: BoardMetricV1,
    tick_duration: &TickDurationV1,
) -> Result<LeaderboardEntryV1, ApiError> {
    let named = public_participants(&row.named_participants);
    let aggregate_named = aggregate_public_participants(&row.aggregate_named_participants);
    Ok(LeaderboardEntryV1 {
        position,
        rank: row.rank,
        run_id: opaque(&row.run_id)?,
        composition: match &row.composition {
            BoardComposition::Mission { replay_sha256 } => VerifiedRunCompositionV1::Mission {
                replay_sha256: Digest32::from_bytes(*replay_sha256),
            },
            BoardComposition::FullCampaign {
                ordered_session_run_ids,
            } => VerifiedRunCompositionV1::FullCampaign {
                ordered_session_run_ids: ordered_session_run_ids
                    .iter()
                    .map(|run_id| opaque(run_id))
                    .collect::<Result<Vec<_>, _>>()?,
            },
        },
        metric_value: board_metric_value(metric, row.metric_value, tick_duration)?,
        max_concurrent_players: row.max_concurrent_players,
        participant_instance_count: row.participant_instance_count,
        named_participant_instance_count: row.named_participant_instance_count,
        anonymous_participant_instance_count: row.anonymous_participant_instance_count,
        named_participants: named,
        aggregate_named_participants: aggregate_named,
        accepted_sequence: row.accepted_sequence,
        verified_at_unix_ms: row.verified_at_ms,
    })
}

fn board_metric_value(
    metric: BoardMetricV1,
    value: i64,
    tick_duration: &TickDurationV1,
) -> Result<BoardMetricValueV1, ApiError> {
    match metric {
        BoardMetricV1::OriginalScore => Ok(BoardMetricValueV1::OriginalScore { points: value }),
        BoardMetricV1::FastestSuccess => Ok(BoardMetricValueV1::FastestSuccess {
            active_simulation_ticks: u64::try_from(value).map_err(|_| ApiError::Internal)?,
            tick_duration: tick_duration.clone(),
        }),
    }
}

fn leaderboard_cursor(
    cursor: &CursorToken,
    opaque_token: String,
    metric: BoardMetricV1,
    tick_duration: &TickDurationV1,
) -> Result<LeaderboardCursorV1, ApiError> {
    Ok(LeaderboardCursorV1 {
        schema_version: SCHEMA_VERSION_V1,
        query_sha256: cursor.filter_sha256,
        accepted_sequence_watermark: cursor.accepted_sequence_watermark,
        last: LeaderboardOrderAnchorV1 {
            position: cursor.position,
            rank: cursor.rank,
            metric_value: board_metric_value(metric, cursor.metric_value, tick_duration)?,
            accepted_sequence: u64::try_from(cursor.accepted_sequence)
                .map_err(|_| ApiError::Internal)?,
            verified_at_unix_ms: cursor.verified_at_unix_ms,
            run_id: opaque(&cursor.run_id)?,
        },
        opaque_token,
    })
}

fn public_participants(
    participants: &[crate::db::PublicParticipantRecord],
) -> Vec<PublicParticipantV1> {
    participants
        .iter()
        .map(|participant| PublicParticipantV1 {
            seat: participant.seat,
            username: participant.identity.username.clone(),
            public_key: PublicKey32::from_bytes(participant.identity.public_key),
            public_key_fingerprint: PublicKey32::from_bytes(participant.identity.public_key)
                .short_fingerprint(),
        })
        .collect()
}

fn aggregate_public_participants(
    participants: &[crate::db::PublicIdentity],
) -> Vec<robin_run_protocol::AggregatePublicParticipantV1> {
    participants
        .iter()
        .map(|participant| {
            let public_key = PublicKey32::from_bytes(participant.public_key);
            robin_run_protocol::AggregatePublicParticipantV1 {
                current_display_name: participant.username.clone(),
                public_key,
                public_key_fingerprint: public_key.short_fingerprint(),
            }
        })
        .collect()
}

fn verify_request_signature(
    public_key: &[u8; 32],
    signature: &[u8; 64],
    signing_bytes: &[u8],
) -> Result<(), ApiError> {
    crate::identity::verify_signature(public_key, signature, signing_bytes)
        .map_err(|_| ApiError::Unauthorized)
}

fn verify_session_attestations(request: &SubmissionOfferRequestV1) -> Result<(), ApiError> {
    let genesis_bytes = request.session_genesis.signing_bytes()?;
    verify_request_signature(
        request.session_genesis.claim.host_public_key.as_bytes(),
        request.session_genesis.host_signature.as_bytes(),
        &genesis_bytes,
    )?;

    for claim in request.participant_claims.iter().skip(1) {
        let attestation = claim.join_attestation.as_ref().ok_or_else(|| {
            ApiError::BadRequest("authenticated guest has no join attestation".to_owned())
        })?;
        let bytes = attestation.signing_bytes()?;
        verify_request_signature(
            attestation.claim.public_key.as_bytes(),
            attestation.signature.as_bytes(),
            &bytes,
        )?;
    }
    Ok(())
}

async fn rate_limit_challenge(
    state: &AppState,
    peer: SocketAddr,
    headers: &HeaderMap,
    purpose: ChallengePurpose,
) -> Result<(), ApiError> {
    let address = effective_client_ip(&state.config, peer, headers)?;
    state.challenge_rate_limiter.check(address, purpose).await
}

fn effective_client_ip(
    config: &crate::config::ServerConfig,
    peer: SocketAddr,
    headers: &HeaderMap,
) -> Result<IpAddr, ApiError> {
    let trusted = config
        .trusted_proxy_cidrs
        .iter()
        .filter_map(|network| network.parse::<ipnet::IpNet>().ok())
        .any(|network| network.contains(&peer.ip()));
    if !trusted {
        return Ok(peer.ip());
    }
    let forwarded = headers
        .get("x-forwarded-for")
        .ok_or_else(|| ApiError::BadRequest("trusted proxy omitted X-Forwarded-For".to_owned()))?
        .to_str()
        .map_err(|_| {
            ApiError::BadRequest("trusted proxy sent invalid X-Forwarded-For".to_owned())
        })?;
    if forwarded.contains(',') || forwarded.trim() != forwarded {
        return Err(ApiError::BadRequest(
            "trusted proxy must supply exactly one canonical X-Forwarded-For address".to_owned(),
        ));
    }
    forwarded
        .parse::<IpAddr>()
        .map_err(|_| ApiError::BadRequest("trusted proxy sent invalid X-Forwarded-For".to_owned()))
}

fn scope_request_name(scope: &ScopeRequestV1) -> &'static str {
    match scope {
        ScopeRequestV1::IndividualLevel => "individual_level",
        ScopeRequestV1::CampaignGenesis => "campaign_genesis",
        ScopeRequestV1::CampaignContinuation { .. } => "campaign_continuation",
    }
}

fn category(value: &str) -> Result<BoardCategoryV1, ApiError> {
    match value {
        "individual_level" => Ok(BoardCategoryV1::IndividualLevel),
        "campaign" => Ok(BoardCategoryV1::Campaign),
        _ => Err(ApiError::Internal),
    }
}

fn metric(value: &str) -> Result<BoardMetricV1, ApiError> {
    match value {
        "original_score" => Ok(BoardMetricV1::OriginalScore),
        "fastest_success" => Ok(BoardMetricV1::FastestSuccess),
        _ => Err(ApiError::Internal),
    }
}

fn metrics(profile: &AdmissionProfile) -> Result<Vec<BoardMetricV1>, ApiError> {
    let mut metrics = profile
        .allowed_metrics
        .iter()
        .map(|value| metric(value))
        .collect::<Result<Vec<_>, _>>()?;
    metrics.sort();
    metrics.dedup();
    Ok(metrics)
}

fn digest(value: &str) -> Result<Digest32, ApiError> {
    value
        .parse()
        .map_err(|error: robin_run_protocol::HexError| configuration_error("digest", error))
}

fn opaque(value: &str) -> Result<OpaqueId, ApiError> {
    OpaqueId::new(value).map_err(|error| ApiError::BadRequest(error.to_string()))
}

fn filter_digest(filter: &RunFilterV1) -> Result<Digest32, ApiError> {
    robin_run_protocol::canonical_json_bytes(filter)
        .map(Digest32::digest_bytes)
        .map_err(|error| ApiError::BadRequest(error.to_string()))
}

fn encode_cursor(cursor: &CursorToken, key: &[u8; 32]) -> Result<String, ApiError> {
    encode_cursor_envelope(cursor, key)
}

fn encode_cursor_envelope<T: Serialize>(cursor: &T, key: &[u8; 32]) -> Result<String, ApiError> {
    let bytes = serde_json::to_vec(cursor).map_err(internal_json)?;
    let signature = crate::authentication::sign(key, &bytes);
    let mut authenticated = bytes;
    authenticated.extend_from_slice(signature.as_ref());
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(authenticated))
}

fn decode_cursor_envelope<T: serde::de::DeserializeOwned>(
    value: &str,
    key: &[u8; 32],
) -> Result<T, ApiError> {
    if value.len() > 2048 {
        return Err(ApiError::BadRequest("cursor is too long".to_owned()));
    }
    let authenticated = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(value)
        .map_err(|_| ApiError::BadRequest("cursor is not valid base64url".to_owned()))?;
    if authenticated.len() <= 32 {
        return Err(ApiError::BadRequest("cursor is not valid".to_owned()));
    }
    let (bytes, signature) = authenticated.split_at(authenticated.len() - 32);
    crate::authentication::verify(key, bytes, signature)
        .map_err(|_| ApiError::BadRequest("cursor authentication failed".to_owned()))?;
    serde_json::from_slice(bytes)
        .map_err(|_| ApiError::BadRequest("cursor is not valid".to_owned()))
}

fn decode_cursor(
    value: &str,
    expected_filter: Digest32,
    key: &[u8; 32],
) -> Result<CursorToken, ApiError> {
    let cursor: CursorToken = decode_cursor_envelope(value, key)?;
    if cursor.filter_sha256 != expected_filter {
        return Err(ApiError::BadRequest(
            "cursor does not belong to this leaderboard".to_owned(),
        ));
    }
    Ok(cursor)
}

fn encode_player_history_cursor(
    cursor: &PlayerHistoryCursorToken,
    key: &[u8; 32],
) -> Result<String, ApiError> {
    encode_cursor_envelope(cursor, key)
}

fn decode_player_history_cursor(
    value: &str,
    player_public_key: PublicKey32,
    query_sha256: Digest32,
    key: &[u8; 32],
) -> Result<PlayerHistoryCursorToken, ApiError> {
    let cursor: PlayerHistoryCursorToken = decode_cursor_envelope(value, key)?;
    if cursor.player_public_key != player_public_key || cursor.query_sha256 != query_sha256 {
        return Err(ApiError::BadRequest(
            "cursor does not belong to this player history query".to_owned(),
        ));
    }
    Ok(cursor)
}

fn safe_rejection_message(code: VerificationRejectionCodeV1) -> &'static str {
    match code {
        VerificationRejectionCodeV1::MalformedReplay => "The uploaded replay is malformed.",
        VerificationRejectionCodeV1::ResourceLimit => "The replay exceeds verification limits.",
        VerificationRejectionCodeV1::UnsupportedSchema => "This replay schema is not rankable.",
        VerificationRejectionCodeV1::BuildNotAllowed => "The replay build is not allowlisted.",
        VerificationRejectionCodeV1::ContentNotAllowed => "The content set is not allowlisted.",
        VerificationRejectionCodeV1::ConfigMismatch => "The replay rules do not match the board.",
        VerificationRejectionCodeV1::StartingStateMismatch => {
            "The starting campaign state does not match the offer."
        }
        VerificationRejectionCodeV1::CommandNotAllowed => {
            "The replay contains a command that is not rankable."
        }
        VerificationRejectionCodeV1::TimelineInvalid => "The replay timeline is invalid.",
        VerificationRejectionCodeV1::StateHashMismatch => {
            "The replay did not reproduce its recorded state."
        }
        VerificationRejectionCodeV1::TerminalInvalid => {
            "The replay did not end in a rankable mission victory."
        }
        VerificationRejectionCodeV1::ResultInvariantMismatch => {
            "The replay result failed a scoring invariant."
        }
        VerificationRejectionCodeV1::InputProvenanceIneligible => {
            "The replay records automation, debug input, a load, or another ineligible source."
        }
        VerificationRejectionCodeV1::SimulationBudgetExceeded => {
            "The replay exceeded the deterministic simulation budget."
        }
    }
}

fn configuration_error(context: &str, error: impl std::fmt::Display) -> ApiError {
    tracing::error!(
        error_code = "public_protocol_configuration",
        error_type = std::any::type_name_of_val(&error),
        context,
        "server configuration violates the public protocol"
    );
    ApiError::Internal
}

fn internal_json(error: serde_json::Error) -> ApiError {
    tracing::error!(
        error_code = "internal_json_serialization",
        category = ?error.classify(),
        "internal JSON serialization failed"
    );
    ApiError::Internal
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{
        artifact, published_ruleset_fixture, viewer_build, viewer_build_v2,
        viewer_content_manifest, viewer_profile,
    };
    use bytes::Bytes;
    use ed25519_dalek::SigningKey;
    use futures_util::stream;
    use http_body_util::BodyExt as _;
    use robin_run_protocol::{
        AnonymousParticipantPolicyV1, ArtifactRefV1, CanonicalCampaignStateKindV1,
        CanonicalCampaignStatePinV1, CanonicalCampaignStateRequirementV1, ImmutablePolicyKindV1,
        LeaderboardCoSignInstanceV1, LeaderboardCoSignPurposeV1, LeaderboardCoSignRequestV1,
        OfficialContentEditionV1, OfficialContentSubjectV1, PublishedRulesetV1,
        RulesConfigConstraintV1,
    };
    use sha2::Sha256;
    use std::sync::atomic::{AtomicBool, Ordering};
    use tower::ServiceExt as _;

    #[tokio::test]
    async fn owned_mutation_keeps_its_lease_after_response_cancellation() {
        use std::sync::atomic::{AtomicBool, Ordering};

        let directory = tempfile::tempdir().unwrap();
        let config = ServerConfig {
            database_path: directory.path().join("highscores.sqlite3"),
            ..Default::default()
        };
        let database = Database::migrate(&config).await.unwrap();
        let started = std::sync::Arc::new(tokio::sync::Notify::new());
        let unblock = std::sync::Arc::new(tokio::sync::Notify::new());
        let completed = std::sync::Arc::new(AtomicBool::new(false));
        let outer = tokio::spawn(run_owned_maintenance_write(
            database.clone(),
            crate::db::MaintenanceWriteClass::ApiSensitive,
            {
                let started = started.clone();
                let unblock = unblock.clone();
                let completed = completed.clone();
                async move {
                    started.notify_one();
                    unblock.notified().await;
                    completed.store(true, Ordering::SeqCst);
                }
            },
        ));
        started.notified().await;
        outer.abort();
        let _ = outer.await;
        assert_eq!(
            database
                .active_maintenance_write_lease_count()
                .await
                .unwrap(),
            1,
            "dropping the response waiter must not release an in-flight mutation"
        );
        let backup = database
            .acquire_backup_lock("test-backup", Duration::from_secs(60))
            .await
            .unwrap();
        assert_eq!(
            database
                .active_maintenance_write_lease_count()
                .await
                .unwrap(),
            1,
            "backup admission must continue to observe the detached mutation"
        );
        unblock.notify_one();
        for _ in 0..100 {
            if database
                .active_maintenance_write_lease_count()
                .await
                .unwrap()
                == 0
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(completed.load(Ordering::SeqCst));
        assert_eq!(
            database
                .active_maintenance_write_lease_count()
                .await
                .unwrap(),
            0
        );
        assert!(database.release_backup_lock(&backup).await.unwrap());

        database.close_fenced().await.unwrap();
    }

    #[tokio::test]
    async fn detached_outer_fence_survives_timeout_and_avoids_nested_admission_deadlock() {
        let directory = tempfile::tempdir().unwrap();
        let config = ServerConfig {
            database_path: directory.path().join("highscores.sqlite3"),
            ..Default::default()
        };
        let database = Database::migrate(&config).await.unwrap();
        let outer_shared_acquired = std::sync::Arc::new(tokio::sync::Barrier::new(2));
        let allow_mutation_start = std::sync::Arc::new(tokio::sync::Notify::new());
        let operation_database = database.clone();
        let entered_filesystem_gap = std::sync::Arc::new(tokio::sync::Barrier::new(2));
        let resume_later_sql = std::sync::Arc::new(tokio::sync::Notify::new());
        let later_sql_complete = std::sync::Arc::new(AtomicBool::new(false));
        let outer_complete = std::sync::Arc::new(AtomicBool::new(false));
        let gate_database = database.clone();
        let gate_outer_acquired = outer_shared_acquired.clone();
        let gate_allow_mutation = allow_mutation_start.clone();
        let gate_complete = outer_complete.clone();
        let gate_filesystem_gap = entered_filesystem_gap.clone();
        let gate_resume_later_sql = resume_later_sql.clone();
        let gate_later_sql_complete = later_sql_complete.clone();
        let gate_owner = tokio::spawn(async move {
            let mut fence = gate_database.begin_fenced_operation().await.unwrap();
            gate_outer_acquired.wait().await;
            gate_allow_mutation.notified().await;
            let operation = run_owned_maintenance_write(
                gate_database.clone(),
                crate::db::MaintenanceWriteClass::ApiSensitive,
                {
                    async move {
                        // Model filesystem publication after early SQL but
                        // before a later status update, with no pool checkout.
                        gate_filesystem_gap.wait().await;
                        gate_resume_later_sql.notified().await;
                        operation_database.health_check().await.unwrap();
                        gate_later_sql_complete.store(true, Ordering::SeqCst);
                    }
                },
            )
            .await;
            let finish = gate_database.finish_fenced_operation(&mut fence).await;
            gate_complete.store(true, Ordering::SeqCst);
            operation.unwrap();
            finish.unwrap();
        });
        let response_waiter = tokio::spawn(async move {
            tokio::time::timeout(Duration::from_millis(20), gate_owner).await
        });
        outer_shared_acquired.wait().await;
        let runtime = database.runtime_fence().clone();
        let admission = runtime.try_lock_exclusive_admission().unwrap().unwrap();
        allow_mutation_start.notify_one();
        tokio::time::timeout(Duration::from_secs(2), entered_filesystem_gap.wait())
            .await
            .expect("mutation deadlocked trying to re-enter admission under outer SH");
        assert!(
            response_waiter.await.unwrap().is_err(),
            "outer response deadline did not detach the paused mutation"
        );

        assert!(
            runtime.try_lock_exclusive_quiescence().unwrap().is_none(),
            "outer SH fence was released during the detached filesystem gap"
        );
        resume_later_sql.notify_one();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        let quiescence = loop {
            if let Some(guard) = runtime.try_lock_exclusive_quiescence().unwrap() {
                break guard;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "outer fence outlived later SQL, lease release, and pool return"
            );
            tokio::task::yield_now().await;
        };
        assert!(later_sql_complete.load(Ordering::SeqCst));
        assert!(outer_complete.load(Ordering::SeqCst));
        runtime
            .validate_exclusive_pair(&admission, &quiescence)
            .unwrap();
        drop(quiescence);
        drop(admission);
        database.close_fenced().await.unwrap();
    }

    #[tokio::test]
    async fn saturated_lane_does_not_admit_queued_request_before_backup_gate() {
        let directory = tempfile::tempdir().unwrap();
        let config = ServerConfig {
            database_path: directory.path().join("highscores.sqlite3"),
            replay_directory: directory.path().join("replays"),
            ..Default::default()
        };
        let database = Database::migrate(&config).await.unwrap();
        let state = AppState {
            database: database.clone(),
            replay_store: ReplayStore::create(config.replay_directory.clone(), 1024)
                .await
                .unwrap(),
            campaign_store: CampaignStore::create(directory.path().join("campaigns"), 1024)
                .await
                .unwrap(),
            config,
            cursor_hmac_key: [1; 32],
            backup_authority_hmac_key: [1; 32],
            competition_run_grant_secret_key: None,
            run_preflight_grant_secret_key: None,
            challenge_rate_limiter: ChallengeRateLimiter::new(10),
        };
        let first_entered = std::sync::Arc::new(tokio::sync::Barrier::new(2));
        let release_first = std::sync::Arc::new(tokio::sync::Notify::new());
        let handler_entries = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
        let handler = {
            let first_entered = first_entered.clone();
            let release_first = release_first.clone();
            let handler_entries = handler_entries.clone();
            move || {
                let first_entered = first_entered.clone();
                let release_first = release_first.clone();
                let handler_entries = handler_entries.clone();
                async move {
                    if handler_entries.fetch_add(1, Ordering::SeqCst) == 0 {
                        first_entered.wait().await;
                        release_first.notified().await;
                    }
                    StatusCode::OK
                }
            }
        };
        // Layer order is deliberate: the most recently added concurrency
        // layer is outermost, so queued calls have no database fence yet.
        let app = Router::new()
            .route("/", get(handler))
            .layer(middleware::from_fn_with_state(state, database_fence_gate))
            .layer(tower::limit::GlobalConcurrencyLimitLayer::new(1));
        let first = tokio::spawn(
            app.clone()
                .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap()),
        );
        first_entered.wait().await;
        let second =
            tokio::spawn(app.oneshot(Request::builder().uri("/").body(Body::empty()).unwrap()));
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(handler_entries.load(Ordering::SeqCst), 1);
        let backup = database
            .acquire_backup_lock("saturated-lane-test", Duration::from_secs(60))
            .await
            .unwrap();
        release_first.notify_one();
        assert_eq!(first.await.unwrap().unwrap().status(), StatusCode::OK);
        assert_eq!(
            second.await.unwrap().unwrap().status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert_eq!(
            handler_entries.load(Ordering::SeqCst),
            1,
            "queued request crossed the fence after backup admission closed"
        );

        let runtime = database.runtime_fence().clone();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        let admission = loop {
            if let Some(guard) = runtime.try_lock_exclusive_admission().unwrap() {
                break guard;
            }
            assert!(tokio::time::Instant::now() < deadline);
            tokio::task::yield_now().await;
        };
        let quiescence = runtime.try_lock_exclusive_quiescence().unwrap().unwrap();
        assert!(database.release_backup_lock(&backup).await.unwrap());
        crate::db_fence::wait_for_pool_idle(database.fixture_pool())
            .await
            .unwrap();
        runtime
            .validate_exclusive_pair(&admission, &quiescence)
            .unwrap();
        drop(quiescence);
        drop(admission);
        database.close_fenced().await.unwrap();
    }

    #[test]
    fn server_verifies_only_the_exact_fixed_co_sign_request() {
        let key = SigningKey::from_bytes(&[0x41; 32]);
        let request = LeaderboardCoSignRequestV1 {
            instance: LeaderboardCoSignInstanceV1 {
                purpose: LeaderboardCoSignPurposeV1::Submission,
                replay_session_id: Digest32::from_bytes([0x51; 32]),
                submission_offer_sha256: Digest32::from_bytes([0x61; 32]),
            },
            run_digest: Digest32::from_bytes([0x71; 32]),
        };
        let signature = key.sign(&request.signing_bytes().unwrap()).to_bytes();
        assert!(
            verify_signature(
                &key.verifying_key().to_bytes(),
                &signature,
                &request.signing_bytes().unwrap(),
            )
            .is_ok()
        );

        let mut cross_session = request;
        cross_session.instance.replay_session_id = Digest32::from_bytes([0x52; 32]);
        let mut replayed_for_another_offer = request;
        replayed_for_another_offer.instance.submission_offer_sha256 =
            Digest32::from_bytes([0x62; 32]);
        let mut substituted_purpose = request;
        substituted_purpose.instance.purpose = LeaderboardCoSignPurposeV1::CampaignContinuation;
        let mut tampered_run = request;
        tampered_run.run_digest = Digest32::from_bytes([0x72; 32]);
        for altered in [
            cross_session,
            replayed_for_another_offer,
            substituted_purpose,
            tampered_run,
        ] {
            assert!(
                verify_signature(
                    &key.verifying_key().to_bytes(),
                    &signature,
                    &altered.signing_bytes().unwrap(),
                )
                .is_err()
            );
        }
    }

    #[test]
    fn single_replay_submission_limit_is_checked_and_exact() {
        let mut config = ServerConfig {
            max_replay_bytes: 11,
            max_campaign_bytes: 13,
            max_metadata_bytes: 17,
            ..Default::default()
        };
        assert_eq!(
            submission_body_limit(&config).unwrap(),
            11 + 13 + 17 + MULTIPART_ENVELOPE_OVERHEAD_BYTES
        );

        config.max_replay_bytes = u64::MAX;
        assert!(matches!(
            submission_body_limit(&config),
            Err(ApiError::Internal)
        ));
    }

    #[tokio::test]
    async fn body_limit_accepts_exact_boundary_and_rejects_plus_one() {
        async fn consume(body: Bytes) -> StatusCode {
            assert_eq!(body.len(), 8);
            StatusCode::NO_CONTENT
        }
        let app = Router::new()
            .route("/", post(consume))
            .layer(DefaultBodyLimit::max(8));
        let exact = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/")
                    .body(Body::from(vec![0_u8; 8]))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(exact.status(), StatusCode::NO_CONTENT);
        let over = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/")
                    .body(Body::from(vec![0_u8; 9]))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(over.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }

    fn cursor() -> CursorToken {
        CursorToken {
            filter_sha256: Digest32::from_bytes([1; 32]),
            accepted_sequence_watermark: 25,
            visibility_revision: 3,
            metric_value: 12_345,
            position: 4,
            rank: 3,
            accepted_sequence: 20,
            verified_at_unix_ms: 1_234_567,
            run_id: "018f0000-0000-7000-8000-000000000000".to_owned(),
        }
    }

    async fn viewer_launch_over_http(
        config: ServerConfig,
        profile: AdmissionProfile,
        build_digest: Digest32,
        content_digest: Digest32,
    ) -> ViewerLaunchV1 {
        let app = Router::new().route(
            "/viewer",
            get(move || {
                let config = config.clone();
                let profile = profile.clone();
                async move {
                    viewer_launch(
                        &config,
                        &profile,
                        build_digest.into_bytes(),
                        content_digest.into_bytes(),
                    )
                    .map(Json)
                }
            }),
        );
        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/viewer")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap()
    }

    #[test]
    fn next_cursor_conversion_matches_entry_based_response_for_both_metrics() {
        let tick_duration = TickDurationV1 {
            numerator_micros: 50_000,
            denominator: 1,
        };
        for (metric, metric_value) in [
            (BoardMetricV1::OriginalScore, -12_345),
            (BoardMetricV1::FastestSuccess, 12_345),
        ] {
            let mut token = cursor();
            token.metric_value = metric_value;
            let row = BoardRow {
                rank: token.rank,
                run_id: token.run_id.clone(),
                composition: BoardComposition::Mission {
                    replay_sha256: [3; 32],
                },
                metric_value,
                max_concurrent_players: 1,
                participant_instance_count: 1,
                named_participant_instance_count: 0,
                anonymous_participant_instance_count: 1,
                accepted_sequence: u64::try_from(token.accepted_sequence).unwrap(),
                verified_at_ms: token.verified_at_unix_ms,
                named_participants: Vec::new(),
                aggregate_named_participants: Vec::new(),
            };
            let entry = board_entry(&row, token.position, metric, &tick_duration).unwrap();
            let opaque_token = encode_cursor(&token, &[2; 32]).unwrap();
            // The previous next-page mapping copied this already-checked entry.
            let expected = LeaderboardCursorV1 {
                schema_version: SCHEMA_VERSION_V1,
                query_sha256: token.filter_sha256,
                accepted_sequence_watermark: token.accepted_sequence_watermark,
                last: LeaderboardOrderAnchorV1 {
                    position: entry.position,
                    rank: entry.rank,
                    metric_value: entry.metric_value,
                    accepted_sequence: entry.accepted_sequence,
                    verified_at_unix_ms: entry.verified_at_unix_ms,
                    run_id: entry.run_id,
                },
                opaque_token: opaque_token.clone(),
            };
            let actual = leaderboard_cursor(&token, opaque_token, metric, &tick_duration).unwrap();
            assert_eq!(actual, expected);
            let decoded =
                decode_cursor(&actual.opaque_token, token.filter_sha256, &[2; 32]).unwrap();
            assert_eq!(decoded.visibility_revision, token.visibility_revision);
            assert_eq!(
                decoded.accepted_sequence_watermark,
                token.accepted_sequence_watermark
            );
            assert_eq!(decoded.metric_value, metric_value);
        }
        let mut invalid = cursor();
        invalid.metric_value = -1;
        assert!(
            leaderboard_cursor(
                &invalid,
                String::new(),
                BoardMetricV1::FastestSuccess,
                &tick_duration
            )
            .is_err()
        );
        invalid = cursor();
        invalid.accepted_sequence = -1;
        assert!(
            leaderboard_cursor(
                &invalid,
                String::new(),
                BoardMetricV1::OriginalScore,
                &tick_duration
            )
            .is_err()
        );
        invalid = cursor();
        invalid.run_id.clear();
        assert!(
            leaderboard_cursor(
                &invalid,
                String::new(),
                BoardMetricV1::OriginalScore,
                &tick_duration
            )
            .is_err()
        );
    }

    #[test]
    fn cursor_authentication_rejects_rank_tampering_and_other_filters() {
        let key = [2; 32];
        let encoded = encode_cursor(&cursor(), &key).unwrap();
        assert!(decode_cursor(&encoded, Digest32::from_bytes([1; 32]), &key).is_ok());
        assert!(
            decode_cursor(&encoded, Digest32::from_bytes([3; 32]), &key).is_err(),
            "a cursor must be bound to the exact board filter"
        );

        let mut decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(&encoded)
            .unwrap();
        decoded[10] ^= 1;
        let tampered = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(decoded);
        assert!(decode_cursor(&tampered, Digest32::from_bytes([1; 32]), &key).is_err());
        assert!(decode_cursor(&encoded, Digest32::from_bytes([1; 32]), &[4; 32]).is_err());
    }

    fn history_cursor() -> PlayerHistoryCursorToken {
        PlayerHistoryCursorToken {
            player_public_key: PublicKey32::from_bytes([3; 32]),
            query_sha256: Digest32::from_bytes([4; 32]),
            accepted_sequence_watermark: 25,
            visibility_revision: 3,
            accepted_sequence: 20,
            run_id: "run-1".to_owned(),
        }
    }

    // Independent legacy envelope construction, also used to authenticate
    // malformed JSON without going through the production serializer.
    fn legacy_cursor_envelope(bytes: &[u8], key: &[u8; 32]) -> String {
        let signature =
            ring::hmac::sign(&ring::hmac::Key::new(ring::hmac::HMAC_SHA256, key), bytes);
        base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode([bytes, signature.as_ref()].concat())
    }

    #[test]
    fn both_cursor_formats_preserve_legacy_bytes() {
        let key = [2; 32];
        let board_json = format!(
            r#"{{"filter_sha256":"{}","accepted_sequence_watermark":25,"visibility_revision":3,"metric_value":12345,"position":4,"rank":3,"accepted_sequence":20,"verified_at_unix_ms":1234567,"run_id":"018f0000-0000-7000-8000-000000000000"}}"#,
            "01".repeat(32)
        );
        let history_json = format!(
            r#"{{"player_public_key":"{}","query_sha256":"{}","accepted_sequence_watermark":25,"visibility_revision":3,"accepted_sequence":20,"run_id":"run-1"}}"#,
            "03".repeat(32),
            "04".repeat(32)
        );
        assert_eq!(
            encode_cursor(&cursor(), &key).unwrap(),
            legacy_cursor_envelope(board_json.as_bytes(), &key)
        );
        assert_eq!(
            encode_player_history_cursor(&history_cursor(), &key).unwrap(),
            legacy_cursor_envelope(history_json.as_bytes(), &key)
        );
    }

    #[test]
    fn history_cursor_authenticates_and_binds_player_and_query() {
        let key = [2; 32];
        let cursor = history_cursor();
        let encoded = encode_player_history_cursor(&cursor, &key).unwrap();
        let decode = |value: &str, player, query, key: &[u8; 32]| {
            decode_player_history_cursor(value, player, query, key)
        };
        let restored = decode(
            &encoded,
            cursor.player_public_key,
            cursor.query_sha256,
            &key,
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(restored).unwrap(),
            serde_json::to_value(&cursor).unwrap()
        );
        for (player, query) in [
            (PublicKey32::from_bytes([5; 32]), cursor.query_sha256),
            (cursor.player_public_key, Digest32::from_bytes([5; 32])),
        ] {
            assert_eq!(
                decode(&encoded, player, query, &key)
                    .unwrap_err()
                    .to_string(),
                "cursor does not belong to this player history query"
            );
        }
        let mut bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(&encoded)
            .unwrap();
        bytes[10] ^= 1;
        let tampered = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
        for (value, key) in [(&tampered, key), (&encoded, [9; 32])] {
            assert_eq!(
                decode(value, cursor.player_public_key, cursor.query_sha256, &key)
                    .unwrap_err()
                    .to_string(),
                "cursor authentication failed"
            );
        }
    }

    #[test]
    fn cursor_envelope_errors_preserve_precedence_for_both_formats() {
        let key = [2; 32];
        for (value, expected) in [
            ("!".repeat(2049), "cursor is too long"),
            ("!".repeat(2048), "cursor is not valid base64url"),
            (
                base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([0; 32]),
                "cursor is not valid",
            ),
            (
                base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([0; 33]),
                "cursor authentication failed",
            ),
            (
                legacy_cursor_envelope(b"not json", &key),
                "cursor is not valid",
            ),
            (legacy_cursor_envelope(b"{}", &key), "cursor is not valid"),
        ] {
            for error in [
                decode_cursor(&value, Digest32::from_bytes([9; 32]), &key).unwrap_err(),
                decode_player_history_cursor(
                    &value,
                    PublicKey32::from_bytes([9; 32]),
                    Digest32::from_bytes([9; 32]),
                    &key,
                )
                .unwrap_err(),
            ] {
                assert!(matches!(error, ApiError::BadRequest(_)));
                assert_eq!(error.to_string(), expected);
            }
        }
    }

    #[test]
    fn public_participant_dto_contains_only_durable_identity_and_seat() {
        let records = vec![
            crate::db::PublicParticipantRecord {
                seat: 0,
                identity: crate::db::PublicIdentity {
                    public_key: [0x31; 32],
                    username: "Robin".to_owned(),
                },
            },
            crate::db::PublicParticipantRecord {
                seat: 1,
                identity: crate::db::PublicIdentity {
                    public_key: [0x42; 32],
                    username: "Marian".to_owned(),
                },
            },
        ];

        let value = serde_json::to_value(public_participants(&records)).unwrap();
        let json = value.to_string();
        assert!(!json.contains("participant_instance_id"));
        assert_eq!(value[0]["seat"], 0);
        assert_eq!(value[1]["seat"], 1);
        assert_eq!(value[0]["username"], "Robin");
        assert_eq!(value[1]["username"], "Marian");
    }

    #[test]
    fn campaign_continuation_requires_the_ordinal_zero_owner_as_host() {
        let owner = PublicKey32::from_bytes([7; 32]);
        let attacker = PublicKey32::from_bytes([8; 32]);
        let owner_claim = robin_run_protocol::ParticipantClaimV1 {
            seat: 0,
            participant_instance_id: Digest32::from_bytes([9; 32]),
            public_key: owner,
            public_disclosure: ParticipantPublicDisclosureV1::NamedProfile,
            join_attestation: None,
        };
        assert!(continuation_owner_authorized(
            owner.into_bytes(),
            std::slice::from_ref(&owner_claim)
        ));
        let anonymous_owner_claim = robin_run_protocol::ParticipantClaimV1 {
            public_disclosure: ParticipantPublicDisclosureV1::Anonymous,
            ..owner_claim.clone()
        };
        assert!(continuation_owner_authorized(
            owner.into_bytes(),
            &[anonymous_owner_claim]
        ));
        assert!(!continuation_owner_authorized(owner.into_bytes(), &[]));
        let attacker_claim = robin_run_protocol::ParticipantClaimV1 {
            public_key: attacker,
            ..owner_claim
        };
        assert!(!continuation_owner_authorized(
            owner.into_bytes(),
            &[attacker_claim]
        ));
    }

    #[test]
    fn noncompetition_filters_cannot_query_a_disabled_profile_metric() {
        let published = published_ruleset_fixture();
        let mut registry = crate::config::ManifestRegistry::default();
        registry
            .rulesets
            .insert(published.ruleset_manifest_sha256, published.clone());
        let mut config = ServerConfig::default();
        let mut profile =
            viewer_profile(Digest32::from_bytes([8; 32]), Digest32::from_bytes([9; 32]));
        profile.config_id = published.manifest.rules_config_sha256.to_string();
        profile.ruleset_id = published.ruleset_manifest_sha256.to_string();
        profile.allowed_metrics = vec!["original_score".to_owned()];
        config.admission_profiles.push(profile);
        config.manifests = Arc::new(registry);
        let mut filter = RunFilterV1 {
            schema_version: SCHEMA_VERSION_V1,
            subject: LeaderboardSubjectV1::Mission {
                mission_id: "mission".to_owned(),
                category: BoardCategoryV1::IndividualLevel,
            },
            metric: BoardMetricV1::OriginalScore,
            content: RunContentIdentityV1::Mission {
                content_manifest_sha256: Digest32::from_bytes([9; 32]),
            },
            rules_config_sha256: Some(published.manifest.rules_config_sha256),
            ruleset_manifest_sha256: Some(published.ruleset_manifest_sha256),
            competition_manifest_sha256: None,
            max_concurrent_players: None,
            player_public_key: None,
        };
        assert!(profile_for_filter(&config, &filter).is_ok());
        filter.metric = BoardMetricV1::FastestSuccess;
        assert!(matches!(
            profile_for_filter(&config, &filter),
            Err(ApiError::NotFound)
        ));
    }

    #[tokio::test]
    async fn leaderboard_metadata_orders_missions_by_mission_id_not_label() {
        let directory = tempfile::tempdir().unwrap();
        let published = published_ruleset_fixture();
        let mut registry = crate::config::ManifestRegistry::default();
        registry
            .rulesets
            .insert(published.ruleset_manifest_sha256, published.clone());
        let mut config = ServerConfig {
            database_path: directory.path().join("highscores.sqlite3"),
            replay_directory: directory.path().join("replays"),
            ..Default::default()
        };
        for (id, mission_id, display_name, content) in [
            ("z-profile", "z-mission", "A", 9),
            ("a-profile", "a-mission", "Z", 10),
        ] {
            let mut profile =
                viewer_profile(Digest32::from_bytes([8; 32]), Digest32::from_bytes([9; 32]));
            profile.id = id.to_owned();
            profile.content_manifest_id = Digest32::from_bytes([content; 32]).to_string();
            profile.content_subject = OfficialContentSubjectV1::FieldMission {
                mission_id: mission_id.to_owned(),
            };
            profile.mission_display_name = display_name.to_owned();
            profile.config_id = published.manifest.rules_config_sha256.to_string();
            profile.ruleset_id = published.ruleset_manifest_sha256.to_string();
            config.admission_profiles.push(profile);
        }
        config.manifests = Arc::new(registry);
        let state =
            crate::test_support::app_state(config.clone(), directory.path().join("campaigns"))
                .await;
        let Json(metadata) = leaderboard_metadata(State(state)).await.unwrap();
        assert_eq!(
            metadata
                .missions
                .iter()
                .map(|mission| mission.mission_id.as_str())
                .collect::<Vec<_>>(),
            ["a-mission", "z-mission"]
        );
        assert_eq!(
            metadata.rulesets.len(),
            2,
            "shared rulesets must retain each mission"
        );
        for mission in &metadata.missions {
            assert!(metadata.rulesets.iter().any(|facet| facet.content
                == RunContentIdentityV1::Mission {
                    content_manifest_sha256: mission.content_manifest_sha256,
                }));
        }
        metadata.validate().unwrap();
        let mut duplicate = metadata.clone();
        duplicate.rulesets.insert(0, duplicate.rulesets[0].clone());
        assert!(
            duplicate.validate().is_err(),
            "duplicate ruleset/content facets remain invalid"
        );
    }

    #[tokio::test]
    async fn immutable_manifest_response_hashes_to_its_route_digest() {
        use robin_run_protocol::{
            CanonicalValue, ImmutablePolicyKindV1, ImmutablePolicyManifestV1,
        };

        let document = ImmutablePolicyManifestV1 {
            schema_version: SCHEMA_VERSION_V1,
            kind: ImmutablePolicyKindV1::Verification,
            version: 1,
            rules: [("allow_ranked".to_owned(), CanonicalValue::Bool(true))]
                .into_iter()
                .collect(),
        };
        let expected = document.canonical_digest().unwrap();
        let response = immutable_json(&document).unwrap();
        assert_eq!(
            response.headers().get(CACHE_CONTROL).unwrap(),
            "public, max-age=31536000, immutable"
        );
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(Digest32::digest_bytes(&bytes), expected);
    }

    #[tokio::test]
    async fn build_v2_route_serves_exact_public_document_and_no_private_identity() {
        let directory = tempfile::tempdir().unwrap();
        let public = robin_run_protocol::VersionedBuildManifest::V2(viewer_build_v2());
        let canonical = public.canonical_bytes().unwrap();
        let public_digest = public.canonical_digest().unwrap();
        let semantic_digest = public
            .backend_visible_v1()
            .unwrap()
            .canonical_digest()
            .unwrap();
        assert_ne!(public_digest, semantic_digest);

        let loaded = crate::config::LoadedBuildManifest::new(public).unwrap();
        let mut registry = crate::config::ManifestRegistry::default();
        registry.builds.insert(public_digest, loaded);
        let config = ServerConfig {
            database_path: directory.path().join("highscores.sqlite3"),
            replay_directory: directory.path().join("replays"),
            manifests: Arc::new(registry),
            ..Default::default()
        };
        let state =
            crate::test_support::app_state(config.clone(), directory.path().join("campaigns"))
                .await;
        let app = router(state).unwrap();

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/builds/{public_digest}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(CACHE_CONTROL).unwrap(),
            "public, max-age=31536000, immutable"
        );
        let body = response.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(body.as_ref(), canonical.as_slice());
        assert_eq!(Digest32::digest_bytes(&body), public_digest);
        let public_json = std::str::from_utf8(&body).unwrap();
        for private_field in [
            "projection_exporter",
            "projection_authority",
            "projection_receipt",
            "source_tree_manifest",
            "participant_instance_id",
            "chain_id",
        ] {
            assert!(!public_json.contains(private_field));
        }

        let private_documents = [
            serde_json::json!({
                "schema_version": 2,
                "projection_authority": {"public_build_manifest_sha256": public_digest},
            }),
            serde_json::json!({
                "schema_version": 2,
                "projection_exporter": {"artifact_sha256": Digest32::from_bytes([70; 32])},
            }),
            serde_json::json!({
                "schema_version": 2,
                "projection_receipt": {"public_build_manifest_sha256": public_digest},
            }),
            serde_json::json!({
                "schema_version": 2,
                "source_tree_manifest": {"source_file_count": 366},
            }),
        ];
        let private_document_digests =
            std::iter::once(semantic_digest).chain(private_documents.iter().map(|document| {
                Digest32::digest_bytes(robin_run_protocol::canonical_json_bytes(document).unwrap())
            }));
        for private_digest in private_document_digests {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(format!("/api/v1/builds/{private_digest}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::NOT_FOUND);
        }
    }

    #[tokio::test]
    async fn http_ruleset_manifest_and_mutable_publication_are_separate() {
        let directory = tempfile::tempdir().unwrap();
        let published = published_ruleset_fixture();
        let digest = published.ruleset_manifest_sha256;
        let mut registry = crate::config::ManifestRegistry::default();
        registry.rulesets.insert(digest, published.clone());
        let config = ServerConfig {
            database_path: directory.path().join("highscores.sqlite3"),
            replay_directory: directory.path().join("replays"),
            manifests: Arc::new(registry),
            ..Default::default()
        };
        let state =
            crate::test_support::app_state(config.clone(), directory.path().join("campaigns"))
                .await;
        let app = router(state).unwrap();

        let response = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri(format!("/api/v1/ruleset-manifests/{digest}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(CACHE_CONTROL).unwrap(),
            "public, max-age=31536000, immutable"
        );
        let manifest_bytes = response.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(Digest32::digest_bytes(&manifest_bytes), digest);

        let response = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri(format!("/api/v1/published-rulesets/{digest}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers().get(CACHE_CONTROL).unwrap(), "no-store");
        let body: PublishedRulesetV1 =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert_eq!(body, published);

        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .uri(format!("/api/v1/rulesets/{digest}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn uploaded_reservation_recovery_treats_replay_as_opaque_storage() {
        let directory = tempfile::tempdir().unwrap();
        let config = ServerConfig {
            database_path: directory.path().join("highscores.sqlite3"),
            replay_directory: directory.path().join("replays"),
            campaign_state_directory: directory.path().join("campaigns"),
            ..Default::default()
        };
        let replay_store = ReplayStore::create(config.replay_directory.clone(), 1024)
            .await
            .unwrap();
        let campaign_store = CampaignStore::create(config.campaign_state_directory.clone(), 1024)
            .await
            .unwrap();
        let state = AppState {
            database: Database::migrate(&config).await.unwrap(),
            replay_store: replay_store.clone(),
            campaign_store: campaign_store.clone(),
            config,
            cursor_hmac_key: [1; 32],
            backup_authority_hmac_key: [1; 32],
            competition_run_grant_secret_key: None,
            run_preflight_grant_secret_key: None,
            challenge_rate_limiter: ChallengeRateLimiter::new(10),
        };

        // Invalid UTF-8 and no compact prefix: recovering an uploaded lease is
        // a storage-identity operation, not an in-process codec invocation.
        let replay = Bytes::from_static(&[0xff, 0x00, 0x80, 0x7f]);
        let replay_digest: [u8; 32] = Sha256::digest(&replay).into();
        replay_store
            .store_stream(
                stream::iter([Ok::<_, &str>(replay.clone())]),
                replay_digest,
                replay.len() as u64,
            )
            .await
            .unwrap();
        let campaign = b"opaque campaign recovery fixture";
        let campaign_digest: [u8; 32] = Sha256::digest(campaign).into();
        campaign_store
            .import_bytes(&campaign_digest, campaign)
            .await
            .unwrap();

        verify_reserved_upload_storage_identity(
            &state,
            replay_digest,
            replay.len() as u64,
            campaign_digest,
            campaign.len() as u64,
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn replay_http_responses_serve_exact_compact_bytes() {
        let directory = tempfile::tempdir().unwrap();
        let config = ServerConfig {
            database_path: directory.path().join("highscores.sqlite3"),
            replay_directory: directory.path().join("replays"),
            ..Default::default()
        };
        let database = Database::migrate(&config).await.unwrap();
        let replay_store = ReplayStore::create(config.replay_directory.clone(), 1024)
            .await
            .unwrap();
        let state = AppState {
            config,
            database,
            replay_store: replay_store.clone(),
            campaign_store: CampaignStore::create(directory.path().join("campaigns"), 1024)
                .await
                .unwrap(),
            cursor_hmac_key: [1; 32],
            backup_authority_hmac_key: [1; 32],
            competition_run_grant_secret_key: None,
            run_preflight_grant_secret_key: None,
            challenge_rate_limiter: ChallengeRateLimiter::new(10),
        };
        let standalone = Bytes::from_static(b"rhrec-test-standalone");
        let standalone_len = standalone.len() as u64;
        let standalone_digest: [u8; 32] = Sha256::digest(&standalone).into();
        replay_store
            .store_stream(
                stream::iter([Ok::<_, &str>(standalone.clone())]),
                standalone_digest,
                standalone_len,
            )
            .await
            .unwrap();
        let compact = Bytes::from_static(b"compact-rhrec");
        let compact_len = compact.len() as u64;
        let compact_digest: [u8; 32] = Sha256::digest(&compact).into();
        replay_store
            .store_stream(
                stream::iter([Ok::<_, &str>(compact.clone())]),
                compact_digest,
                compact_len,
            )
            .await
            .unwrap();

        let standalone_state = state.clone();
        let aggregate_state = state.clone();
        let app = Router::new()
            .route(
                "/api/v1/runs/standalone/replay",
                get(move || {
                    let state = standalone_state.clone();
                    async move { replay_response(&state, standalone_digest, standalone_len).await }
                }),
            )
            .route(
                "/api/v1/runs/aggregate/sessions/1/replay",
                get(move || {
                    let state = aggregate_state.clone();
                    async move { replay_response(&state, compact_digest, compact_len).await }
                }),
            );
        for (path, expected_type, expected_body) in [
            (
                "/api/v1/runs/standalone/replay",
                RANKED_REPLAY_MEDIA_TYPE_V1,
                standalone,
            ),
            (
                "/api/v1/runs/aggregate/sessions/1/replay",
                RANKED_REPLAY_MEDIA_TYPE_V1,
                compact,
            ),
        ] {
            let response = app
                .clone()
                .oneshot(
                    axum::http::Request::builder()
                        .uri(path)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(response.headers().get(CONTENT_TYPE).unwrap(), expected_type);
            assert_eq!(
                response.into_body().collect().await.unwrap().to_bytes(),
                expected_body
            );
        }
    }

    #[tokio::test]
    async fn viewer_launch_http_is_available_only_for_exact_published_artifacts() {
        let build = viewer_build();
        build.validate().unwrap();
        let build_digest = build.canonical_digest().unwrap();
        let content = viewer_content_manifest(OfficialContentEditionV1::Demo);
        content.validate().unwrap();
        let content_digest = content.canonical_digest().unwrap();
        let profile = viewer_profile(build_digest, content_digest);
        let mut registry = crate::config::ManifestRegistry::default();
        registry.builds.insert(
            build_digest,
            crate::config::LoadedBuildManifest::new(
                robin_run_protocol::VersionedBuildManifest::V1(build),
            )
            .unwrap(),
        );
        registry.content_manifests.insert(content_digest, content);
        let mut config = ServerConfig::default();
        config.admission_profiles.push(profile.clone());
        config.manifests = Arc::new(registry);

        let exact_config = config.clone();
        let exact_profile = profile.clone();
        let substituted_config = config.clone();
        let substituted_profile = profile.clone();
        let app = Router::new()
            .route(
                "/exact",
                get(move || {
                    let config = exact_config.clone();
                    let profile = exact_profile.clone();
                    async move {
                        viewer_launch(
                            &config,
                            &profile,
                            build_digest.into_bytes(),
                            content_digest.into_bytes(),
                        )
                        .map(Json)
                    }
                }),
            )
            .route(
                "/substituted-build",
                get(move || {
                    let config = substituted_config.clone();
                    let profile = substituted_profile.clone();
                    async move {
                        viewer_launch(&config, &profile, [99; 32], content_digest.into_bytes())
                            .map(Json)
                    }
                }),
            );
        let response = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri("/exact")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let exact: ViewerLaunchV1 =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert!(matches!(
            exact.availability,
            ViewerAvailabilityV1::Available {
                content_requirement: ViewerContentRequirementV1::BundledDemo {
                    content_manifest_sha256
                }
            } if content_manifest_sha256 == content_digest
        ));

        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/substituted-build")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let substituted: ViewerLaunchV1 =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert!(matches!(
            substituted.availability,
            ViewerAvailabilityV1::Unavailable { .. }
        ));

        let mut missing_registry = config;
        missing_registry.manifests = Arc::new(crate::config::ManifestRegistry::default());
        assert!(matches!(
            viewer_launch(
                &missing_registry,
                &profile,
                build_digest.into_bytes(),
                content_digest.into_bytes()
            )
            .unwrap()
            .availability,
            ViewerAvailabilityV1::Unavailable { .. }
        ));
    }

    #[tokio::test]
    async fn viewer_launch_http_rejects_demo_full_entitlement_inversion() {
        let build = viewer_build();
        build.validate().unwrap();
        let build_digest = build.canonical_digest().unwrap();
        let loaded_build = crate::config::LoadedBuildManifest::new(
            robin_run_protocol::VersionedBuildManifest::V1(build),
        )
        .unwrap();

        for (edition, requirement, expected_available) in [
            (
                OfficialContentEditionV1::Demo,
                ViewerContentRequirementConfig::BundledDemo,
                true,
            ),
            (
                OfficialContentEditionV1::Demo,
                ViewerContentRequirementConfig::UserLocalRetail,
                false,
            ),
            (
                OfficialContentEditionV1::Full,
                ViewerContentRequirementConfig::UserLocalRetail,
                true,
            ),
            (
                OfficialContentEditionV1::Full,
                ViewerContentRequirementConfig::BundledDemo,
                false,
            ),
        ] {
            let content = viewer_content_manifest(edition);
            content.validate().unwrap();
            let content_digest = content.canonical_digest().unwrap();
            let mut profile = viewer_profile(build_digest, content_digest);
            profile.viewer_content_requirement = Some(requirement);
            let mut registry = crate::config::ManifestRegistry::default();
            registry.builds.insert(build_digest, loaded_build.clone());
            registry.content_manifests.insert(content_digest, content);
            let mut config = ServerConfig::default();
            config.admission_profiles.push(profile.clone());
            config.manifests = Arc::new(registry);

            let launch =
                viewer_launch_over_http(config, profile, build_digest, content_digest).await;
            if expected_available {
                let published_content_digest = match (edition, launch.availability) {
                    (
                        OfficialContentEditionV1::Demo,
                        ViewerAvailabilityV1::Available {
                            content_requirement:
                                ViewerContentRequirementV1::BundledDemo {
                                    content_manifest_sha256,
                                },
                        },
                    )
                    | (
                        OfficialContentEditionV1::Full,
                        ViewerAvailabilityV1::Available {
                            content_requirement:
                                ViewerContentRequirementV1::UserLocalRetail {
                                    content_manifest_sha256,
                                },
                        },
                    ) => content_manifest_sha256,
                    unexpected => panic!("unexpected exact viewer launch: {unexpected:?}"),
                };
                assert_eq!(published_content_digest, content_digest);
            } else {
                assert!(matches!(
                    launch.availability,
                    ViewerAvailabilityV1::Unavailable { .. }
                ));
            }
        }
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
            .apply_username_update(
                &challenge.id,
                challenge.nonce.into_bytes(),
                public_key,
                username,
            )
            .await
            .unwrap();
    }

    async fn insert_anonymous_host_run(
        database: &Database,
        host_public_key: [u8; 32],
        marker: u8,
        tombstoned: bool,
    ) -> String {
        let guest_public_key = [marker; 32];
        register_test_identity(database, host_public_key, &format!("Host{marker}")).await;
        register_test_identity(database, guest_public_key, &format!("Guest{marker}")).await;
        let challenge = database
            .issue_challenge(
                ChallengePurpose::Submission,
                host_public_key,
                Duration::from_secs(60),
                Some("{}"),
                Some("{}"),
            )
            .await
            .unwrap();
        let replay_sha256 = [marker.wrapping_add(1); 32];
        database
            .register_replay_object(&replay_sha256, 4)
            .await
            .unwrap();
        database
            .register_campaign_object(&[marker.wrapping_add(6); 32], 5)
            .await
            .unwrap();
        let submission = NewSubmission {
            id: uuid::Uuid::now_v7().to_string(),
            upload_challenge_id: challenge.id,
            offer_json: "{}".to_owned(),
            envelope_json: "{}".to_owned(),
            signatures_json: "[]".to_owned(),
            public_metadata_json: "{}".to_owned(),
            replay_sha256,
            replay_bytes: 4,
            build_manifest_id: [marker.wrapping_add(2); 32],
            content_manifest_id: [marker.wrapping_add(3); 32],
            campaign_content_manifest_id: None,
            config_id: [marker.wrapping_add(4); 32],
            ruleset_id: [marker.wrapping_add(5); 32],
            mission_id: "privacy-mission".to_owned(),
            scope_kind: "individual_level".to_owned(),
            starting_campaign_sha256: [marker.wrapping_add(6); 32],
            starting_campaign_bytes: 5,
            controller_public_key: host_public_key,
            canonical_campaign_state_json: serde_json::to_string(&CanonicalCampaignStatePinV1 {
                requirement: CanonicalCampaignStateRequirementV1 {
                    edition: OfficialContentEditionV1::Demo,
                    kind: CanonicalCampaignStateKindV1::IndividualTemplate,
                    rules_config_sha256: Digest32::from_bytes([marker.wrapping_add(4); 32]),
                },
                artifact: ArtifactRefV1 {
                    sha256: Digest32::from_bytes([marker.wrapping_add(6); 32]),
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
            max_concurrent_players: 2,
            participant_instance_count: 2,
            session_genesis_sha256: [marker.wrapping_add(7); 32],
            session_genesis_host_public_key: host_public_key,
            replay_session_id: [marker.wrapping_add(8); 32],
            session_genesis_host_nonce: [marker.wrapping_add(9); 32],
            participants: vec![
                ParticipantClaim {
                    seat: 0,
                    participant_instance_id: [marker.wrapping_add(10); 32],
                    public_key: host_public_key,
                    public_disclosure: "anonymous".to_owned(),
                },
                ParticipantClaim {
                    seat: 1,
                    participant_instance_id: [marker.wrapping_add(11); 32],
                    public_key: guest_public_key,
                    public_disclosure: "named_profile".to_owned(),
                },
            ],
        };
        let reservation = database
            .reserve_submission_upload(
                &SubmissionUploadIntent {
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
                },
                Duration::from_secs(30),
                Duration::from_secs(300),
            )
            .await
            .unwrap();
        let SubmissionUploadReservation::Acquired { lease, .. } = reservation else {
            panic!("privacy fixture upload reservation was not acquired");
        };
        database
            .mark_submission_upload_uploaded(&lease)
            .await
            .unwrap();
        let lifecycle = database
            .finalize_submission_upload(&submission, &lease)
            .await
            .unwrap();
        sqlx::query("UPDATE submissions SET status = 'accepted' WHERE id = ?")
            .bind(&lifecycle.id)
            .execute(database.fixture_pool())
            .await
            .unwrap();
        let accepted_sequence: i64 = sqlx::query_scalar(
            "INSERT INTO acceptance_sequences (created_at_ms) VALUES (1) RETURNING sequence",
        )
        .fetch_one(database.fixture_pool())
        .await
        .unwrap();
        let run_id = uuid::Uuid::now_v7().to_string();
        sqlx::query(
            "INSERT INTO verified_runs (id, submission_id, verifier_build_id, \
                build_manifest_id, content_manifest_id, campaign_content_manifest_id, config_id, ruleset_id, mission_id, \
                scope_kind, canonical_campaign_state_json, starting_campaign_sha256, \
                starting_campaign_bytes, final_campaign_sha256, final_campaign_bytes, campaign_chain_id, \
                final_state_sha256, result_sha256, verification_request_sha256, \
                verification_result_json, public_verification_request_sha256, \
                public_verification_request_json, public_verification_result_sha256, \
                public_verification_result_json, public_projection_binding_json, \
                input_provenance_json, terminal_outcome, replay_frames, \
                diagnostics_json, original_score_delta, active_simulation_ticks, ransom_collected, \
                starting_campaign_score, final_campaign_score, campaign_session_kind, \
                campaign_session_ordinal, campaign_hq_sequence, max_concurrent_players, \
                participant_instance_count, named_participant_instance_count, \
                anonymous_participant_instance_count, accepted_sequence, verified_at_ms) \
             VALUES (?, ?, ?, ?, ?, NULL, ?, ?, 'privacy-mission', 'individual_level', \
                     ?, ?, 5, ?, 6, NULL, ?, ?, ?, '{}', ?, '{}', ?, '{}', '{}', \
                     '{\"status\":\"rankable\"}', 'won', 1, '{}', \
                     10, 20, 0, 0, 10, NULL, NULL, NULL, 2, 2, 1, 1, ?, 1)",
        )
        .bind(&run_id)
        .bind(&lifecycle.id)
        .bind([marker.wrapping_add(12); 32].as_slice())
        .bind(submission.build_manifest_id.as_slice())
        .bind(submission.content_manifest_id.as_slice())
        .bind(submission.config_id.as_slice())
        .bind(submission.ruleset_id.as_slice())
        .bind(&submission.canonical_campaign_state_json)
        .bind(submission.starting_campaign_sha256.as_slice())
        .bind([marker.wrapping_add(13); 32].as_slice())
        .bind([marker.wrapping_add(14); 32].as_slice())
        .bind([marker.wrapping_add(15); 32].as_slice())
        .bind([marker.wrapping_add(16); 32].as_slice())
        .bind([marker.wrapping_add(17); 32].as_slice())
        .bind([marker.wrapping_add(18); 32].as_slice())
        .bind(accepted_sequence)
        .execute(database.fixture_pool())
        .await
        .unwrap();
        if tombstoned {
            sqlx::query("UPDATE submissions SET tombstoned_at_ms = created_at_ms + 1 WHERE id = ?")
                .bind(&lifecycle.id)
                .execute(database.fixture_pool())
                .await
                .unwrap();
        }
        run_id
    }

    async fn request_deletion_challenge(
        app: &Router,
        public_key: PublicKey32,
        run_id: &str,
    ) -> (
        StatusCode,
        HeaderMap,
        serde_json::Value,
        DeletionChallengeV1,
    ) {
        let request = DeletionChallengeRequestV1 {
            schema_version: SCHEMA_VERSION_V1,
            public_key,
            target: DeletionTargetV1::Run {
                run_id: OpaqueId::new(run_id).unwrap(),
            },
        };
        let mut http_request = Request::builder()
            .method(Method::POST)
            .uri("/api/v1/deletion-challenges")
            .header(CONTENT_TYPE, "application/json")
            .body(Body::from(serde_json::to_vec(&request).unwrap()))
            .unwrap();
        http_request.extensions_mut().insert(ConnectInfo(
            "127.0.0.1:30000".parse::<SocketAddr>().unwrap(),
        ));
        let response = app.clone().oneshot(http_request).await.unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let value = serde_json::from_slice(&bytes).unwrap();
        let challenge = serde_json::from_slice(&bytes).unwrap();
        (status, headers, value, challenge)
    }

    async fn submit_deletion(
        app: &Router,
        challenge: DeletionChallengeV1,
        signing_key: &ed25519_dalek::SigningKey,
    ) -> Response {
        use ed25519_dalek::Signer as _;
        let mut deletion = DeletionRequestEnvelopeV1 {
            schema_version: SCHEMA_VERSION_V1,
            challenge,
            signature: robin_run_protocol::Signature64::from_bytes([0; 64]),
        };
        deletion.signature = robin_run_protocol::Signature64::from_bytes(
            signing_key
                .sign(&deletion.signing_bytes().unwrap())
                .to_bytes(),
        );
        app.clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/v1/deletion-requests")
                    .header(CONTENT_TYPE, "application/json")
                    .body(Body::from(serde_json::to_vec(&deletion).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap()
    }

    #[tracing_test::traced_test]
    #[tokio::test]
    async fn deletion_challenge_is_not_an_unsigned_ownership_oracle() {
        let directory = tempfile::tempdir().unwrap();
        let config = ServerConfig {
            database_path: directory.path().join("highscores.sqlite3"),
            replay_directory: directory.path().join("replays"),
            challenge_requests_per_minute_per_ip: 20,
            ..Default::default()
        };
        let database = Database::migrate(&config).await.unwrap();
        let owner_signing_key = ed25519_dalek::SigningKey::from_bytes(&[0xd4; 32]);
        let owner_key = PublicKey32::from_bytes(owner_signing_key.verifying_key().to_bytes());
        let wrong_signing_key = ed25519_dalek::SigningKey::from_bytes(&[0xe5; 32]);
        let wrong_key = PublicKey32::from_bytes(wrong_signing_key.verifying_key().to_bytes());
        let deleted_owner_signing_key = ed25519_dalek::SigningKey::from_bytes(&[0xf6; 32]);
        let deleted_owner_key =
            PublicKey32::from_bytes(deleted_owner_signing_key.verifying_key().to_bytes());
        let live_run =
            insert_anonymous_host_run(&database, owner_key.into_bytes(), 0x31, false).await;
        let deleted_run =
            insert_anonymous_host_run(&database, deleted_owner_key.into_bytes(), 0x42, true).await;
        let state = AppState {
            database: database.clone(),
            replay_store: ReplayStore::create(config.replay_directory.clone(), 1024)
                .await
                .unwrap(),
            campaign_store: CampaignStore::create(directory.path().join("campaigns"), 1024)
                .await
                .unwrap(),
            config,
            cursor_hmac_key: [1; 32],
            backup_authority_hmac_key: [1; 32],
            competition_run_grant_secret_key: None,
            run_preflight_grant_secret_key: None,
            challenge_rate_limiter: ChallengeRateLimiter::new(20),
        };
        let app = router(state).unwrap();
        let nonexistent_run = "00000000-0000-7000-8000-000000000000";
        let cases = [
            (wrong_key, live_run.as_str()),
            (owner_key, live_run.as_str()),
            (owner_key, nonexistent_run),
            (deleted_owner_key, deleted_run.as_str()),
        ];
        let mut challenges = Vec::new();
        let mut expected_shape = None;
        for (public_key, run_id) in cases {
            let (status, headers, value, challenge) =
                request_deletion_challenge(&app, public_key, run_id).await;
            assert_eq!(status, StatusCode::CREATED);
            assert_eq!(headers.get(CACHE_CONTROL).unwrap(), "no-store");
            assert_eq!(headers.get(X_CONTENT_TYPE_OPTIONS).unwrap(), "nosniff");
            assert_eq!(headers.get(CONTENT_TYPE).unwrap(), "application/json");
            let shape = value
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect::<BTreeSet<_>>();
            assert_eq!(
                shape,
                [
                    "schema_version",
                    "deletion_challenge_id",
                    "deletion_challenge_nonce",
                    "expires_at_unix_ms",
                    "public_key",
                    "target",
                ]
                .into_iter()
                .map(str::to_owned)
                .collect()
            );
            if let Some(expected) = &expected_shape {
                assert_eq!(&shape, expected);
            } else {
                expected_shape = Some(shape);
            }
            assert_eq!(challenge.public_key, public_key);
            assert_eq!(
                challenge.target,
                DeletionTargetV1::Run {
                    run_id: OpaqueId::new(run_id).unwrap()
                }
            );
            challenges.push(challenge);
        }

        // A valid wrong-key signature still cannot delete the live anonymous
        // run. The exact anonymous owner signature can, and an exact owner of
        // an already tombstoned run receives the same signed-apply absence.
        let response = submit_deletion(&app, challenges[0].clone(), &wrong_signing_key).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(response.headers().get(CACHE_CONTROL).unwrap(), "no-store");
        let live_tombstone: Option<i64> = sqlx::query_scalar(
            "SELECT s.tombstoned_at_ms FROM submissions s JOIN verified_runs r \
             ON r.submission_id = s.id WHERE r.id = ?",
        )
        .bind(&live_run)
        .fetch_one(database.fixture_pool())
        .await
        .unwrap();
        assert_eq!(live_tombstone, None);

        let response = submit_deletion(&app, challenges[1].clone(), &owner_signing_key).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers().get(CACHE_CONTROL).unwrap(), "no-store");
        let receipt: DeletionReceiptV1 =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert_eq!(
            receipt.target,
            DeletionTargetV1::Run {
                run_id: OpaqueId::new(live_run.clone()).unwrap()
            }
        );
        let live_tombstone: Option<i64> = sqlx::query_scalar(
            "SELECT s.tombstoned_at_ms FROM submissions s JOIN verified_runs r \
             ON r.submission_id = s.id WHERE r.id = ?",
        )
        .bind(&live_run)
        .fetch_one(database.fixture_pool())
        .await
        .unwrap();
        assert!(live_tombstone.is_some());

        let response =
            submit_deletion(&app, challenges[3].clone(), &deleted_owner_signing_key).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        // Nested attacker-controlled data is rejected before challenge issue,
        // while the sensitive-route cache policy still covers the rejection.
        let hostile = serde_json::json!({
            "schema_version": SCHEMA_VERSION_V1,
            "public_key": "c3".repeat(32),
            "target": {
                "kind": "run",
                "run_id": "unknown-but-well-formed-run",
                "private_value": "AnonymousSentinelUsername"
            }
        });
        let mut http_request = Request::builder()
            .method(Method::POST)
            .uri("/api/v1/deletion-challenges")
            .header(CONTENT_TYPE, "application/json")
            .body(Body::from(serde_json::to_vec(&hostile).unwrap()))
            .unwrap();
        http_request.extensions_mut().insert(ConnectInfo(
            "127.0.0.1:30000".parse::<SocketAddr>().unwrap(),
        ));
        let response = app.clone().oneshot(http_request).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(response.headers().get(CACHE_CONTROL).unwrap(), "no-store");
        let body = response.into_body().collect().await.unwrap().to_bytes();
        assert!(!String::from_utf8_lossy(&body).contains("AnonymousSentinelUsername"));

        // Access logging must not record path/query material. Player keys and
        // cursor/proof strings are deliberately exercised as URI sentinels.
        let uri_key = "c7".repeat(32);
        let uri_signature = "d8".repeat(64);
        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/api/v1/players/{uri_key}?proof_signature={uri_signature}"
                    ))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        let private_path = "/srv/private/fullgame/content.bundle";
        drop(ApiError::from(crate::db::DbError::ResultInvariant(
            private_path.to_owned(),
        )));
        assert!(logs_contain("result_invariant"));
        for sentinel in [uri_key.as_str(), uri_signature.as_str(), private_path] {
            assert!(
                !logs_contain(sentinel),
                "private log sentinel leaked: {sentinel}"
            );
        }
    }

    #[tokio::test]
    async fn challenge_rate_limits_are_per_address_and_purpose() {
        let limiter = ChallengeRateLimiter::new(2);
        let address: IpAddr = "127.0.0.1".parse().unwrap();
        limiter
            .check(address, ChallengePurpose::UsernameUpdate)
            .await
            .unwrap();
        limiter
            .check(address, ChallengePurpose::UsernameUpdate)
            .await
            .unwrap();
        assert!(matches!(
            limiter
                .check(address, ChallengePurpose::UsernameUpdate)
                .await,
            Err(ApiError::RateLimited {
                retry_after_ms: 1..=60_000
            })
        ));
        limiter
            .check(address, ChallengePurpose::Submission)
            .await
            .unwrap();
        limiter
            .check(
                "127.0.0.2".parse().unwrap(),
                ChallengePurpose::UsernameUpdate,
            )
            .await
            .unwrap();
    }

    #[test]
    fn forwarding_headers_require_an_exactly_configured_trusted_proxy() {
        let peer: SocketAddr = "10.0.0.1:443".parse().unwrap();
        let forwarded: IpAddr = "203.0.113.77".parse().unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "203.0.113.77".parse().unwrap());

        let mut config = ServerConfig::default();
        assert_eq!(
            effective_client_ip(&config, peer, &headers).unwrap(),
            peer.ip()
        );

        config.trusted_proxy_cidrs = vec!["10.0.0.2/32".to_owned()];
        assert_eq!(
            effective_client_ip(&config, peer, &headers).unwrap(),
            peer.ip()
        );

        config.trusted_proxy_cidrs = vec!["10.0.0.1/32".to_owned()];
        assert_eq!(
            effective_client_ip(&config, peer, &headers).unwrap(),
            forwarded
        );

        headers.insert("x-forwarded-for", "not-an-address".parse().unwrap());
        assert!(effective_client_ip(&config, peer, &headers).is_err());
        headers.remove("x-forwarded-for");
        assert!(effective_client_ip(&config, peer, &headers).is_err());
        headers.insert(
            "x-forwarded-for",
            "203.0.113.77, 198.51.100.1".parse().unwrap(),
        );
        assert!(effective_client_ip(&config, peer, &headers).is_err());
    }

    #[tokio::test]
    async fn operator_router_requires_token_and_audits_actions() {
        let directory = tempfile::tempdir().unwrap();
        let config = ServerConfig {
            database_path: directory.path().join("highscores.sqlite3"),
            replay_directory: directory.path().join("replays"),
            moderation_bearer_token: Some(Arc::new(b"0123456789abcdef0123456789abcdef".to_vec())),
            moderation_bearer_token_path: Some(directory.path().join("token")),
            ..Default::default()
        };
        let database = Database::migrate(&config).await.unwrap();
        let key = [5; 32];
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
            .apply_username_update(&challenge.id, challenge.nonce.into_bytes(), key, "Tuck")
            .await
            .unwrap();
        let (report_id, _) = database
            .insert_abuse_report(
                "player",
                &hex::encode(key),
                &[],
                "other",
                "review this identity",
                [8; 32],
                10,
                10,
                10,
            )
            .await
            .unwrap();
        let state = AppState {
            config: config.clone(),
            database,
            replay_store: ReplayStore::create(config.replay_directory.clone(), 1024)
                .await
                .unwrap(),
            campaign_store: CampaignStore::create(directory.path().join("campaigns"), 1024)
                .await
                .unwrap(),
            cursor_hmac_key: [9; 32],
            backup_authority_hmac_key: [9; 32],
            competition_run_grant_secret_key: None,
            run_preflight_grant_secret_key: None,
            challenge_rate_limiter: ChallengeRateLimiter::new(10),
        };
        let application = router(state).unwrap();
        let unauthorized = application
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri("/api/v1/operator/reports")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
        let action = serde_json::json!({"state":"reviewing","detail":"triaged"});
        let response = application
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method(Method::POST)
                    .uri(format!("/api/v1/operator/reports/{report_id}/actions"))
                    .header(
                        axum::http::header::AUTHORIZATION,
                        "Bearer 0123456789abcdef0123456789abcdef",
                    )
                    .header(CONTENT_TYPE, "application/json")
                    .body(Body::from(action.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        let audit = application
            .oneshot(
                axum::http::Request::builder()
                    .uri(format!(
                        "/api/v1/operator/moderation-audit?report_id={report_id}"
                    ))
                    .header(
                        axum::http::header::AUTHORIZATION,
                        "Bearer 0123456789abcdef0123456789abcdef",
                    )
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(audit.status(), StatusCode::OK);
        let bytes = http_body_util::BodyExt::collect(audit.into_body())
            .await
            .unwrap()
            .to_bytes();
        let events: Vec<crate::db::ModerationAuditRecord> = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].report_id.as_deref(), Some(report_id.as_str()));
    }

    #[tokio::test]
    async fn operator_routes_are_absent_without_a_configured_secret() {
        let directory = tempfile::tempdir().unwrap();
        let config = ServerConfig {
            database_path: directory.path().join("highscores.sqlite3"),
            replay_directory: directory.path().join("replays"),
            ..Default::default()
        };
        let state = AppState {
            database: Database::migrate(&config).await.unwrap(),
            replay_store: ReplayStore::create(config.replay_directory.clone(), 1024)
                .await
                .unwrap(),
            campaign_store: CampaignStore::create(directory.path().join("campaigns"), 1024)
                .await
                .unwrap(),
            config,
            cursor_hmac_key: [7; 32],
            backup_authority_hmac_key: [7; 32],
            competition_run_grant_secret_key: None,
            run_preflight_grant_secret_key: None,
            challenge_rate_limiter: ChallengeRateLimiter::new(10),
        };
        let response = router(state)
            .unwrap()
            .oneshot(
                axum::http::Request::builder()
                    .uri("/api/v1/operator/metrics")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn backup_status_reader_rejects_parent_symlinks_and_wrong_pinned_parent() {
        use std::os::unix::fs::{PermissionsExt as _, symlink};

        fn make_private_status_parent(path: &std::path::Path, marker: &str) {
            std::fs::create_dir(path).unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
            let status = path.join("backup-status.json");
            std::fs::write(&status, format!("{{\"marker\":\"{marker}\"}}")).unwrap();
            std::fs::set_permissions(status, std::fs::Permissions::from_mode(0o400)).unwrap();
        }

        let directory = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let real_parent = directory.path().join("real-status");
        make_private_status_parent(&real_parent, "real");
        let linked_parent = directory.path().join("linked-status");
        symlink(&real_parent, &linked_parent).unwrap();
        assert!(
            read_bounded_nofollow(&linked_parent.join("backup-status.json"), 1024)
                .await
                .is_err(),
            "a symlinked configured parent must not become readiness authority"
        );

        let other_parent = directory.path().join("other-status");
        make_private_status_parent(&other_parent, "other");
        let configured_path = real_parent.join("backup-status.json");
        let pinned_other =
            pin_readiness_status_parent(&other_parent.join("backup-status.json")).unwrap();
        assert!(
            read_bounded_from_pinned_status_parent(
                &configured_path,
                &pinned_other,
                1024,
                || Ok(())
            )
            .await
            .is_err(),
            "the same status basename in another parent must not satisfy the configured path"
        );
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn backup_status_reader_rejects_parent_replacement_and_substitution() {
        use std::os::unix::fs::{PermissionsExt as _, symlink};

        fn make_private_status_parent(path: &std::path::Path, bytes: &[u8]) {
            std::fs::create_dir(path).unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
            let status = path.join("backup-status.json");
            std::fs::write(&status, bytes).unwrap();
            std::fs::set_permissions(status, std::fs::Permissions::from_mode(0o400)).unwrap();
        }

        let directory = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let bytes = br#"{"status":"authenticated"}"#;

        let replaced_parent = directory.path().join("replaced-status");
        make_private_status_parent(&replaced_parent, bytes);
        let replaced_path = replaced_parent.join("backup-status.json");
        let pinned_replaced = pin_readiness_status_parent(&replaced_path).unwrap();
        let displaced_parent = directory.path().join("displaced-status");
        assert!(
            read_bounded_from_pinned_status_parent(&replaced_path, &pinned_replaced, 1024, || {
                std::fs::rename(&replaced_parent, &displaced_parent)?;
                make_private_status_parent(&replaced_parent, bytes);
                Ok(())
            })
            .await
            .is_err(),
            "an identical replacement parent must fail final path identity closure"
        );

        let substituted_parent = directory.path().join("substituted-status");
        make_private_status_parent(&substituted_parent, bytes);
        let substituted_path = substituted_parent.join("backup-status.json");
        let pinned_substituted = pin_readiness_status_parent(&substituted_path).unwrap();
        let moved_parent = directory.path().join("moved-status");
        let alternate_parent = directory.path().join("alternate-status");
        make_private_status_parent(&alternate_parent, bytes);
        assert!(
            read_bounded_from_pinned_status_parent(
                &substituted_path,
                &pinned_substituted,
                1024,
                || {
                    std::fs::rename(&substituted_parent, &moved_parent)?;
                    symlink(&alternate_parent, &substituted_parent)?;
                    Ok(())
                }
            )
            .await
            .is_err(),
            "a substituted parent symlink must fail final path identity closure"
        );
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn backup_status_requires_authentication_and_rejects_symlinks_and_future_timestamps() {
        use std::os::unix::fs::{PermissionsExt as _, symlink};

        async fn replace_status(path: &std::path::Path, bytes: &[u8]) {
            let temporary = path.with_extension("replacement");
            tokio::fs::write(&temporary, bytes).await.unwrap();
            std::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(0o400)).unwrap();
            tokio::fs::rename(temporary, path).await.unwrap();
        }

        let directory = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let state_root = directory.path().join("state");
        let status_root = state_root.join("status");
        tokio::fs::create_dir_all(&status_root).await.unwrap();
        std::fs::set_permissions(&status_root, std::fs::Permissions::from_mode(0o700)).unwrap();
        tokio::fs::create_dir(state_root.join("backups"))
            .await
            .unwrap();
        let target = status_root.join("verified-backup.json");
        let link = status_root.join("backup-status.json");
        let release_manifest = directory.path().join("vps-release-manifest-v2.json");
        let mut release_files = vec![serde_json::json!({
            "artifact": {
                "byte_length": 1,
                "media_type": "application/octet-stream",
                "sha256": "9a".repeat(32),
            },
            "path": "README.md",
            "unix_mode": 0o440,
        })];
        for name in [
            "robin-highscores-api.service",
            "robin-highscores-backup.service",
            "robin-highscores-backup.timer",
            "robin-highscores-worker.service",
            "robin-highscores.target",
        ] {
            let bytes = format!("fixture {name}\n");
            release_files.push(serde_json::json!({
                "artifact": {
                    "byte_length": bytes.len(),
                    "media_type": "text/plain",
                    "sha256": robin_run_protocol::Digest32::digest_bytes(bytes.as_bytes()),
                },
                "path": format!("systemd/user/{name}"),
                "unix_mode": 0o440,
            }));
        }
        let release_document = serde_json::json!({
            "database_schema_version": robin_run_protocol::HIGHSCORES_DATABASE_SCHEMA_VERSION,
            "deployment": {
                "current_link": "/home/robinhood/.local/opt/robin-highscores/current",
                "home": "/home/robinhood",
                "install_root": "/home/robinhood/.local/opt/robin-highscores",
                "persistent_state_root": "/home/robinhood/.local/share/robin-highscores",
                "user": "robinhood",
            },
            "files": release_files,
            "publication_lock_sha256": "34".repeat(32),
            "publication_manifest_sha256": "56".repeat(32),
            "schema_version": 2,
            "source_commit": "0123456789abcdef0123456789abcdef01234567",
            "verifier_sha256": "78".repeat(32),
        });
        tokio::fs::write(
            &release_manifest,
            robin_run_protocol::canonical_json_bytes(&release_document).unwrap(),
        )
        .await
        .unwrap();
        let release_identity = load_backup_release_identity_oob(&release_manifest)
            .await
            .unwrap();
        let key = [7; 32];
        let created_at = u64::try_from(crate::model::now_epoch_ms().unwrap()).unwrap();
        let mut restore_sources = [
            "campaigns",
            "highscores.sqlite3",
            "replays",
            "restore/state/competition-run-grant.key",
            "restore/state/cursor-hmac.key",
            "restore/state/moderation-bearer.token",
            "restore/state/run-preflight-grant.key",
            "restore/systemd/user/robin-highscores-api.service",
            "restore/systemd/user/robin-highscores-backup.service",
            "restore/systemd/user/robin-highscores-backup.timer",
            "restore/systemd/user/robin-highscores-worker.service",
            "restore/systemd/user/robin-highscores.target",
        ]
        .into_iter()
        .map(|archive| BackupRestoreSourceV4 {
            original_absolute_path: match archive {
                "highscores.sqlite3" => {
                    "/home/robinhood/.local/share/robin-highscores/database/highscores.sqlite3"
                        .to_owned()
                }
                "replays" => "/home/robinhood/.local/share/robin-highscores/replays".to_owned(),
                "campaigns" => {
                    "/home/robinhood/.local/share/robin-highscores/campaign-states".to_owned()
                }
                archive if archive.starts_with("restore/state/") => format!(
                    "/home/robinhood/.local/share/robin-highscores/api-secrets/{}",
                    archive.trim_start_matches("restore/state/")
                ),
                archive if archive.starts_with("restore/systemd/user/") => format!(
                    "/home/robinhood/.config/systemd/user/{}",
                    archive.trim_start_matches("restore/systemd/user/")
                ),
                _ => unreachable!("fixture archive allowlist is exhaustive"),
            },
            archive_relative_path: archive.to_owned(),
        })
        .collect::<Vec<_>>();
        restore_sources
            .sort_by(|left, right| left.archive_relative_path.cmp(&right.archive_relative_path));
        let mut files = [
            "highscores.sqlite3",
            "restore/state/competition-run-grant.key",
            "restore/state/cursor-hmac.key",
            "restore/state/moderation-bearer.token",
            "restore/state/run-preflight-grant.key",
            "restore/systemd/user/robin-highscores-api.service",
            "restore/systemd/user/robin-highscores-backup.service",
            "restore/systemd/user/robin-highscores-backup.timer",
            "restore/systemd/user/robin-highscores-worker.service",
            "restore/systemd/user/robin-highscores.target",
        ]
        .into_iter()
        .map(|relative_path| BackupFileV4 {
            relative_path: relative_path.to_owned(),
            byte_length: if relative_path.starts_with("restore/systemd/user/") {
                let bytes = format!(
                    "fixture {}\n",
                    relative_path.trim_start_matches("restore/systemd/user/")
                );
                u64::try_from(bytes.len()).unwrap()
            } else if relative_path.ends_with(".key") {
                32
            } else {
                1
            },
            sha256: if relative_path.starts_with("restore/systemd/user/") {
                robin_run_protocol::Digest32::digest_bytes(
                    format!(
                        "fixture {}\n",
                        relative_path.trim_start_matches("restore/systemd/user/")
                    )
                    .as_bytes(),
                )
                .to_string()
            } else {
                "9b".repeat(32)
            },
        })
        .collect::<Vec<_>>();
        files.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
        let manifest = BackupManifestProjectionV4 {
            schema_version: 4,
            created_at_unix_ms: created_at,
            database_schema_version: crate::db::CURRENT_SCHEMA_VERSION,
            release_identity: release_identity.clone(),
            root_unix_mode: 0o700,
            restore_sources,
            directories: crate::backup::canonical_backup_directories_v4(&files).unwrap(),
            files,
        };
        let backup_id = format!("backup-v4-{created_at}-{}", "a".repeat(32));
        let status = BackupStatusV4::new_authenticated(
            backup_id.clone(),
            state_root
                .join("backups")
                .join(&backup_id)
                .to_string_lossy()
                .into_owned(),
            manifest.clone(),
            &key,
        )
        .unwrap();
        replace_status(
            &target,
            &robin_run_protocol::canonical_json_bytes(&status).unwrap(),
        )
        .await;
        symlink(&target, &link).unwrap();

        let mut config = ServerConfig {
            database_path: directory.path().join("highscores.sqlite3"),
            replay_directory: directory.path().join("replays"),
            campaign_state_directory: directory.path().join("campaigns"),
            backup_manifest_path: Some(link.clone()),
            release_manifest_path: Some(release_manifest),
            ..Default::default()
        };
        let state = AppState {
            database: Database::migrate(&config).await.unwrap(),
            replay_store: ReplayStore::create(config.replay_directory.clone(), 1024)
                .await
                .unwrap(),
            campaign_store: CampaignStore::create(config.campaign_state_directory.clone(), 1024)
                .await
                .unwrap(),
            config: config.clone(),
            cursor_hmac_key: key,
            backup_authority_hmac_key: key,
            competition_run_grant_secret_key: None,
            run_preflight_grant_secret_key: None,
            challenge_rate_limiter: ChallengeRateLimiter::new(10),
        };
        assert!(matches!(
            backup_age_ms_with_active_release(&state, &release_identity).await,
            Err(ApiError::Unavailable)
        ));

        config.backup_manifest_path = Some(link.clone());
        let mut future_manifest = manifest.clone();
        future_manifest.created_at_unix_ms = u64::MAX;
        let future_id = format!("backup-v4-{}-{}", u64::MAX, "b".repeat(32));
        let future = BackupStatusV4::new_authenticated(
            future_id.clone(),
            state_root
                .join("backups")
                .join(&future_id)
                .to_string_lossy()
                .into_owned(),
            future_manifest,
            &key,
        )
        .unwrap();
        replace_status(
            &link,
            &robin_run_protocol::canonical_json_bytes(&future).unwrap(),
        )
        .await;
        let mut state = AppState { config, ..state };
        assert!(matches!(
            backup_age_ms_with_active_release(&state, &release_identity).await,
            Err(ApiError::Unavailable)
        ));

        replace_status(
            &link,
            &robin_run_protocol::canonical_json_bytes(&status).unwrap(),
        )
        .await;
        assert!(
            backup_age_ms_with_active_release(&state, &release_identity)
                .await
                .unwrap()
                .is_some()
        );
        let mut tampered = serde_json::to_value(&status).unwrap();
        tampered["backup_manifest_sha256"] = serde_json::Value::String("cd".repeat(32));
        replace_status(
            &link,
            &robin_run_protocol::canonical_json_bytes(&tampered).unwrap(),
        )
        .await;
        assert!(matches!(
            backup_age_ms_with_active_release(&state, &release_identity).await,
            Err(ApiError::Unavailable)
        ));
        let mut wrong_manifest = manifest;
        wrong_manifest
            .release_identity
            .source_commit
            .replace_range(0..1, "f");
        let mismatched = BackupStatusV4::new_authenticated(
            format!("backup-v4-{created_at}-{}", "c".repeat(32)),
            state_root
                .join("backups")
                .join(format!("backup-v4-{created_at}-{}", "c".repeat(32)))
                .to_string_lossy()
                .into_owned(),
            wrong_manifest,
            &key,
        )
        .unwrap();
        replace_status(
            &link,
            &robin_run_protocol::canonical_json_bytes(&mismatched).unwrap(),
        )
        .await;
        assert!(matches!(
            backup_age_ms_with_active_release(&state, &release_identity).await,
            Err(ApiError::Unavailable)
        ));
        replace_status(
            &link,
            &robin_run_protocol::canonical_json_bytes(&status).unwrap(),
        )
        .await;
        state.backup_authority_hmac_key[0] ^= 1;
        assert!(matches!(
            backup_age_ms_with_active_release(&state, &release_identity).await,
            Err(ApiError::Unavailable)
        ));
    }
}
