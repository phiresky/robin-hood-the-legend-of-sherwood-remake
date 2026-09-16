mod extract;
mod metering;

#[cfg(test)]
use extract::effective_client_ip;
use extract::{ClientIp, ValidatedJson};
pub use metering::RateLimiter;
use metering::{RateLimitScope, RateLimitSubject};

use crate::db::{BoardCursor, BoardQuery, BoardRow, PublicIdentity};
use crate::error::{ApiError, OptionExt as _};
use crate::identity::validate_username;
use crate::storage_admission::{
    StorageAdmissionError, ensure_maximum_upload_capacity, ensure_upload_capacity,
};
use crate::{Database, ReplayStore, ServerConfig};
use axum::body::Body;
use axum::extract::{DefaultBodyLimit, Multipart, Path, Query, State};
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
use futures_util::stream;
use robin_run_protocol::{
    AbuseReportAcceptedV1, AbuseReportCategoryV1, AbuseReportTargetV1, AbuseReportV1,
    AchievementSummaryV1, ArtifactRefV1, BoardMetricV1, BoardMetricValueV2, CanonicalDocument as _,
    CanonicalValue, DeletionReceiptV1, DeletionTargetV1, Digest32, LeaderboardCursorV2,
    LeaderboardEntryV2, LeaderboardMetadataV2, LeaderboardOrderAnchorV2, LeaderboardPageV2,
    LeaderboardQueryV2, OpaqueId, PlayerPersonalBestV2, PlayerProfileV1, PlayerRunHistoryEntryV2,
    PlayerRunHistoryPageV2, PlayerRunHistoryQueryV1, PublicAchievementDecisionV1, PublicKey32,
    PublicParticipantV1, RANKED_REPLAY_MEDIA_TYPE_V1, ReplayArtifactV1, RunDetailV2, RunFilterV2,
    RunMetricsV1, RunSummaryV2, SCHEMA_VERSION_V1, SCHEMA_VERSION_V2, SignedDeletionRequestV2,
    SignedSubmissionOwnerStatusRequestV2, SignedSubmissionV3, SignedUsernameUpdateV2,
    SubmissionAcceptedV1, SubmissionFailureCodeV1, SubmissionLifecycleV1,
    SubmissionOwnerStatusResponseV2, Validate as _, VerificationRejectionCodeV1,
    ViewerAvailabilityV2, ViewerLaunchV2,
};
use serde::{Deserialize, Serialize};
use sha2::Digest as _;
use std::convert::Infallible;
use std::str::FromStr as _;
use std::time::Duration;
use tokio_util::io::ReaderStream;
use tower_http::cors::CorsLayer;
use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::timeout::{RequestBodyTimeoutLayer, TimeoutLayer};
use tower_http::trace::TraceLayer;

const SUBMISSION_RETRY_AFTER_MS: u64 = 2_000;
const MULTIPART_ENVELOPE_OVERHEAD_BYTES: usize = 1024 * 1024;

fn submission_body_limit(config: &ServerConfig) -> Result<usize, ApiError> {
    let replay = usize::try_from(config.max_replay_bytes).map_err(|_| ApiError::Internal)?;
    replay
        .checked_add(config.max_metadata_bytes)
        .and_then(|value| value.checked_add(MULTIPART_ENVELOPE_OVERHEAD_BYTES))
        .or_internal("submission body limit overflows usize")
}

/// Refuse a declared `Content-Length` above the upload ceiling before the
/// body is read. Bodies without a declared length stay bounded by
/// `DefaultBodyLimit` while streaming.
async fn reject_oversized_upload(
    State(state): State<AppState>,
    request: Request<Body>,
    next: Next,
) -> Result<Response, ApiError> {
    let limit =
        u64::try_from(submission_body_limit(&state.config)?).map_err(|_| ApiError::Internal)?;
    if let Some(value) = request.headers().get(CONTENT_LENGTH) {
        let declared = value
            .to_str()
            .ok()
            .and_then(|text| text.parse::<u64>().ok())
            .ok_or_else(|| ApiError::BadRequest("invalid Content-Length".to_owned()))?;
        if declared > limit {
            return Err(ApiError::PayloadTooLarge(format!(
                "declared request body of {declared} bytes exceeds the {limit}-byte upload limit"
            )));
        }
    }
    Ok(next.run(request).await)
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
/// This deliberately does not decompress or deserialize the
/// replay. Those attacker-controlled expansion stages remain exclusive to
/// the contained verifier. The bounded byte buffer lets us reject a JSONL or
/// alternate-format upload before reserving an upload or creating anything in
/// the durable replay store.
fn preflight_ranked_replay_transport(
    bytes: &[u8],
    artifact: &ArtifactRefV1,
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
            "replay SHA-256 does not match the signed submission".to_owned(),
        ));
    }
    robin_replay_format::preflight_compact_transport(
        bytes,
        &robin_replay_format::DEFAULT_REPLAY_ADMISSION_LIMITS,
    )
    .map_err(|error| ApiError::BadRequest(format!("invalid binary replay: {error}")))?;
    Ok(())
}

#[derive(Clone)]
pub struct AppState {
    pub config: ServerConfig,
    pub database: Database,
    pub replay_store: ReplayStore,
    /// Durable server-local secret used only to authenticate pagination state.
    pub cursor_hmac_key: [u8; 32],
    /// Per-address and per-key request metering. Limits come from `config`.
    pub rate_limiter: RateLimiter,
}

const RATE_WINDOW_MINUTE: Duration = Duration::from_secs(60);
const RATE_WINDOW_HOUR: Duration = metering::LONGEST_RATE_WINDOW;

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
        .route(
            "/api/v1/diagnostics",
            post(submit_diagnostic).layer(DefaultBodyLimit::max(
                robin_run_protocol::diagnostics::MAX_DIAGNOSTIC_BODY_BYTES,
            )),
        )
        .layer(DefaultBodyLimit::max(body_limit))
        .layer(RequestBodyTimeoutLayer::new(Duration::from_secs(
            state.config.upload_timeout_seconds,
        )))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            maintenance_upload_write_gate,
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
        // Outside the concurrency limit and the maintenance write lease: a
        // declared body above the upload ceiling is refused before any slot,
        // lease or body byte is consumed.
        .layer(middleware::from_fn_with_state(
            state.clone(),
            reject_oversized_upload,
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            CACHE_CONTROL,
            HeaderValue::from_static("no-store"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ));
    // These resources contain opaque lifecycle identifiers, owner-signed
    // requests, deletion receipts, or moderation receipts. Apply no-store to
    // both success and error responses so browser and intermediary caches
    // cannot persist them.
    let mut sensitive_router = Router::new()
        .route(
            "/api/v1/submissions/{submission_id}/public-status",
            get(submission_public_status),
        )
        .route(
            "/api/v1/submissions/{submission_id}/private-status",
            post(submission_private_status),
        )
        .route("/api/v1/deletion-requests", post(deletion_request))
        .route("/api/v1/reports", post(abuse_report))
        .route(
            "/api/v1/players/{public_key}/username",
            put(update_username),
        );
    if state.config.moderation_bearer_token.is_some() {
        sensitive_router = sensitive_router
            .route("/api/v1/operator/reports", get(operator_reports))
            .route("/api/v1/operator/diagnostics", get(operator_diagnostics))
            .route(
                "/api/v1/operator/diagnostics/{report_id}",
                get(operator_diagnostic).delete(operator_delete_diagnostic),
            )
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
        .route("/api/v1/latest-runs", get(latest_runs))
        .route("/api/v1/runs/{run_id}", get(run_detail))
        .route("/api/v1/runs/{run_id}/replay", get(run_replay))
        .route("/api/v1/players/{public_key}", get(player_profile))
        .route("/api/v1/players/{public_key}/runs", get(player_run_history))
        .layer(DefaultBodyLimit::max(state.config.max_metadata_bytes))
        .layer(RequestBodyTimeoutLayer::new(Duration::from_secs(
            state.config.upload_timeout_seconds,
        )))
        .layer(tower::limit::GlobalConcurrencyLimitLayer::new(
            state.config.max_concurrent_requests,
        ))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            Duration::from_secs(state.config.upload_timeout_seconds),
        ))
        // Dynamic public JSON and binary responses can change after a rename,
        // deletion, or board configuration change.
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
        // Acquire the durable lease before polling the handler; this owner task
        // survives response cancellation and keeps the lease until it finishes.
        let lease = database
            .acquire_maintenance_write_lease(
                writer_class,
                "robin-highscores-api",
                API_WRITE_LEASE_TTL,
            )
            .await?;
        // A panic becomes a JoinError observed by this lease owner;
        // the lease remains until the task is gone.
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
    // actual completion.
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
    // probes run during startup and on mutating admission paths.
    ensure_maximum_upload_capacity(&state.config, &state.database, &state.replay_store)
        .map_err(storage_admission_error)?;
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
    Ok(Json(OperationalStatusResponse {
        schema_version: SCHEMA_VERSION_V1,
        database: state.database.operational_counts().await?,
        replay_storage_free_bytes,
    }))
}

async fn operator_metrics(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    authorize_operator(&state, &headers)?;
    let status = operator_status(State(state), headers).await?.0;
    let counts = status.database;
    let body = format!(
        "# TYPE robin_highscores_submissions gauge\n\
         robin_highscores_submissions{{state=\"queued\"}} {}\n\
         robin_highscores_submissions{{state=\"rejected_retained\"}} {}\n\
         robin_highscores_submissions{{state=\"failed_retained\"}} {}\n\
         # TYPE robin_highscores_upload_reservations gauge\n\
         robin_highscores_upload_reservations{{state=\"active\"}} {}\n\
         robin_highscores_upload_reservations{{state=\"abandoned\"}} {}\n\
         # TYPE robin_highscores_accepted_runs gauge\n\
         robin_highscores_accepted_runs {}\n\
         # TYPE robin_highscores_open_abuse_reports gauge\n\
         robin_highscores_open_abuse_reports {}\n\
         # TYPE robin_highscores_replay_objects gauge\n\
         robin_highscores_replay_objects{{state=\"live\"}} {}\n\
         robin_highscores_replay_objects{{state=\"purging\"}} {}\n\
         # TYPE robin_highscores_replay_storage_free_bytes gauge\n\
         robin_highscores_replay_storage_free_bytes {}\n",
        counts.queued_submissions,
        counts.rejected_retained_submissions,
        counts.failed_retained_submissions,
        counts.active_upload_reservations,
        counts.abandoned_upload_reservations,
        counts.accepted_runs,
        counts.open_abuse_reports,
        counts.replay_objects_live,
        counts.replay_objects_purging,
        status.replay_storage_free_bytes,
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

async fn replay_store_ready(state: &AppState) -> Result<(), ApiError> {
    state.replay_store.readiness_check().await.map_err(|error| {
        tracing::error!(
            error_code = error.safe_log_code(),
            "replay store admission check failed"
        );
        ApiError::Unavailable
    })
}

async fn ensure_upload_admission_ready(
    state: &AppState,
    replay_bytes: u64,
) -> Result<(), ApiError> {
    replay_store_ready(state).await?;
    ensure_upload_capacity(
        &state.config,
        &state.database,
        &state.replay_store,
        replay_bytes,
    )
    .map_err(storage_admission_error)
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
) -> Result<Json<LeaderboardMetadataV2>, ApiError> {
    state
        .config
        .leaderboard_metadata()
        .map(Json)
        .map_err(|error| configuration_error("leaderboard metadata", error))
}

async fn submit(
    State(state): State<AppState>,
    client: ClientIp,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> Result<(StatusCode, Json<SubmissionAcceptedV1>), ApiError> {
    if headers.contains_key(CONTENT_ENCODING) {
        return Err(ApiError::BadRequest(
            "submission requests must not use a transport content encoding".to_owned(),
        ));
    }
    // Cheap per-address metering before any body byte is parsed.
    let address = client.resolve(&state.config)?;
    state
        .rate_limiter
        .check(
            RateLimitScope::Submission,
            RateLimitSubject::Address(address),
            state.config.submissions_per_hour_per_ip,
            RATE_WINDOW_HOUR,
        )
        .await?;
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
    let signed: SignedSubmissionV3 =
        robin_run_protocol::strict_json::from_slice(&metadata_bytes)
            .map_err(|error| ApiError::BadRequest(format!("invalid submission JSON: {error}")))?;
    let authenticated =
        crate::submission::authenticate(&signed, &state.config, crate::model::now_unix_ms()?)?;
    // Per-key metering only counts requests that the key actually signed.
    state
        .rate_limiter
        .check(
            RateLimitScope::Submission,
            RateLimitSubject::PublicKey(authenticated.uploader_public_key()),
            state.config.submissions_per_hour_per_key,
            RATE_WINDOW_HOUR,
        )
        .await?;
    let submission = &signed.request;
    if !state
        .database
        .identity_exists(submission.uploader_public_key.as_bytes())
        .await?
    {
        return Err(ApiError::BadRequest(
            "the uploader must register a username before submitting".to_owned(),
        ));
    }

    // Read and lexically preflight the only replay field before reserving the
    // upload. This buffer is bounded by the configured canonical replay ceiling;
    // the scan itself borrows it and performs no attacker-sized allocation.
    let artifact = &submission.replay.artifact;
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
    require_multipart_media_type(&replay_field, &artifact.media_type, "replay")?;
    let replay_limit =
        usize::try_from(state.config.max_replay_bytes).map_err(|_| ApiError::Internal)?;
    let replay_bytes = read_bounded_field(replay_field, replay_limit).await?;
    preflight_ranked_replay_transport(&replay_bytes, artifact)?;
    if multipart
        .next_field()
        .await
        .map_err(|error| ApiError::BadRequest(error.to_string()))?
        .is_some()
    {
        return Err(ApiError::BadRequest(
            "submission multipart body must contain exactly two fields".to_owned(),
        ));
    }

    let lease_ttl = Duration::from_secs(
        state
            .config
            .upload_timeout_seconds
            .checked_add(30)
            .or_internal("upload timeout plus grace overflows u64")?,
    );
    let ingestion_state = &state;
    let ingestion = |resume_uploaded| {
        let state = ingestion_state;
        async move {
            if resume_uploaded {
                drop(
                    state
                        .replay_store
                        .open_verified(&artifact.sha256.into_bytes(), artifact.byte_length)
                        .await?,
                );
            } else {
                // Replay bytes remain semantically opaque in the network-facing
                // process; the contained verifier is the sole decoder.
                state
                    .replay_store
                    .store_stream(
                        stream::iter([Ok::<_, Infallible>(Bytes::from(replay_bytes))]),
                        artifact.sha256.into_bytes(),
                        artifact.byte_length,
                    )
                    .await?;
            }
            Ok(())
        }
    };
    let lifecycle = crate::submission::complete_upload(
        &state.database,
        &authenticated,
        lease_ttl,
        Duration::from_secs(state.config.upload_reservation_ttl_seconds),
        ensure_upload_admission_ready(&state, authenticated.replay_bytes()),
        ingestion,
    )
    .await?;
    submission_accepted_response(lifecycle)
}

fn submission_accepted_response(
    lifecycle: crate::model::SubmissionLifecycle,
) -> Result<(StatusCode, Json<SubmissionAcceptedV1>), ApiError> {
    let accepted_submission_id = lifecycle.id.clone();
    let state_value = lifecycle_state(lifecycle);
    if !matches!(
        state_value,
        SubmissionLifecycleV1::Queued
            | SubmissionLifecycleV1::Verifying
            | SubmissionLifecycleV1::RetryPending
    ) {
        return Err(ApiError::Conflict(
            "this signed upload already completed verification; query the submission status"
                .to_owned(),
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

async fn submission_public_status(
    State(state): State<AppState>,
    Path(value): Path<String>,
) -> Result<Response, ApiError> {
    let id = opaque(&value).map_err(|_| ApiError::NotFound)?;
    let status = state
        .database
        .public_submission_status(&id)
        .await?
        .ok_or(ApiError::NotFound)?;
    // Pending progress must not be cached as a terminal result by a proxy.
    Ok((
        [(axum::http::header::CACHE_CONTROL, "no-store")],
        Json(status),
    )
        .into_response())
}

/// Owner-signed private lifecycle. A replay of a captured request within the
/// signing window returns only what the key owner can already see, and an
/// unknown, deleted or foreign submission is indistinguishable (`401`).
async fn submission_private_status(
    State(state): State<AppState>,
    client: ClientIp,
    Path(submission_id): Path<String>,
    Json(signed): Json<SignedSubmissionOwnerStatusRequestV2>,
) -> Result<Json<SubmissionOwnerStatusResponseV2>, ApiError> {
    rate_limit_signed_request(&state, &client, RateLimitScope::OwnerStatus).await?;
    signed.verify(
        state.config.signed_requests.window(),
        crate::model::now_unix_ms()?,
    )?;
    let request = &signed.request;
    if request.submission_id.as_str() != submission_id {
        return Err(ApiError::Unauthorized);
    }
    let lifecycle = state
        .database
        .owner_submission_lifecycle(request.public_key.into_bytes(), &submission_id)
        .await
        .map_err(|error| match error {
            crate::db::DbError::NotFound => ApiError::Unauthorized,
            other => other.into(),
        })?;
    let response = SubmissionOwnerStatusResponseV2 {
        schema_version: SCHEMA_VERSION_V2,
        submission_id: request.submission_id.clone(),
        public_key: request.public_key,
        request_sha256: signed
            .canonical_digest()
            .map_err(|error| ApiError::BadRequest(error.to_string()))?,
        state: lifecycle_state(lifecycle),
    };
    response
        .validate_against_request(&signed)
        .map_err(|error| {
            tracing::error!(
                error_code = "owner_status_response_invalid",
                "private status projection failed: {error}"
            );
            ApiError::Internal
        })?;
    Ok(Json(response))
}

async fn leaderboard(
    State(state): State<AppState>,
    Query(query): Query<LeaderboardQueryV2>,
) -> Result<Json<LeaderboardPageV2>, ApiError> {
    query.validate()?;
    if u32::from(query.limit) > state.config.max_page_size {
        return Err(ApiError::BadRequest(format!(
            "limit exceeds server maximum {}",
            state.config.max_page_size
        )));
    }
    // The legacy full-any URL is a browsing filter, not a submission board.
    let boards: Vec<_> = if query.board_id.as_str() == "full-any" {
        state
            .config
            .boards
            .iter()
            .filter(|board| board.edition == robin_run_protocol::OfficialContentEditionV1::Full)
            .collect()
    } else {
        vec![
            state
                .config
                .board(&query.board_id)
                .ok_or(ApiError::NotFound)?,
        ]
    };
    let mission_boards: Vec<_> = boards
        .into_iter()
        .filter(|board| board.mission(&query.mission_id).is_some())
        .collect();
    if mission_boards.is_empty() {
        return Err(ApiError::NotFound);
    }
    let board_ids: Vec<_> = mission_boards
        .into_iter()
        .filter(|board| board.metrics.contains(&query.metric))
        .map(|board| board.board_id.as_str().to_owned())
        .collect();
    if board_ids.is_empty() {
        return Err(ApiError::BadRequest(
            "the selected boards do not rank this metric".to_owned(),
        ));
    }
    let filter = query.filter();
    let filter_sha = filter_digest(&filter)?;
    let decoded_cursor = query
        .cursor
        .as_deref()
        .map(|cursor| decode_cursor(cursor, filter_sha, &state.cursor_hmac_key))
        .transpose()?;
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
    let db_cursor = decoded_cursor.as_ref().map(|cursor| BoardCursor {
        metric_value: cursor.metric_value,
        accepted_sequence: cursor.accepted_sequence,
        run_id: cursor.run_id.clone(),
    });
    let board_query = BoardQuery {
        board_ids: &board_ids,
        mission_id: &query.mission_id,
        metric: query.metric,
        max_concurrent_players: query.max_concurrent_players,
        player_public_key: query.player_public_key.map(PublicKey32::into_bytes),
    };
    let mut rows = state
        .database
        .leaderboard_rows(
            &board_query,
            db_cursor.as_ref(),
            u32::from(query.limit) + 1,
            accepted_sequence_watermark,
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
            .or_internal("leaderboard position overflows u64")?;
        entries.push(board_entry(row, position, query.metric)?);
    }
    let next_cursor = if has_more {
        let (entry, row) = entries
            .last()
            .zip(rows.last())
            .or_internal("leaderboard page with a next cursor has no rows")?;
        let token = CursorToken {
            filter_sha256: filter_sha,
            accepted_sequence_watermark,
            visibility_revision,
            metric_value: row.metric_value,
            position: entry.position,
            rank: entry.rank,
            accepted_sequence: i64::try_from(entry.accepted_sequence)
                .map_err(|_| ApiError::Internal)?,
            verified_at_unix_ms: entry.verified_at_unix_ms,
            run_id: entry.run_id.as_str().to_owned(),
        };
        let opaque_token = encode_cursor(&token, &state.cursor_hmac_key)?;
        Some(leaderboard_cursor(&token, opaque_token, query.metric)?)
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
                leaderboard_cursor(cursor, opaque_token.clone(), query.metric)
            })
            .transpose()?
    };
    let page = LeaderboardPageV2 {
        schema_version: SCHEMA_VERSION_V2,
        filter,
        accepted_sequence_watermark,
        previous_cursor,
        entries,
        next_cursor,
    };
    page.validate().map_err(|_error| {
        tracing::error!(
            error_code = "leaderboard_page_invalid",
            "leaderboard query produced an invalid protocol page"
        );
        ApiError::Internal
    })?;
    Ok(Json(page))
}

#[derive(Serialize, Deserialize)]
struct LatestRun {
    run: RunSummaryV2,
    verified_at_unix_ms: u64,
}

#[derive(Serialize, Deserialize)]
struct LatestRunsPage {
    schema_version: u32,
    runs: Vec<LatestRun>,
}

async fn latest_runs(State(state): State<AppState>) -> Result<Json<LatestRunsPage>, ApiError> {
    let records = state
        .database
        .latest_runs(&state.config.board_ids())
        .await?;
    let runs = records
        .into_iter()
        .map(|record| {
            let run = RunSummaryV2 {
                schema_version: SCHEMA_VERSION_V2,
                run_id: opaque(&record.run_id)?,
                board_id: opaque(&record.board_id)?,
                mission_id: record.mission_id,
                max_concurrent_players: record.max_concurrent_players,
                participant_instance_count: record.participant_instance_count,
                uploader: record.uploader.as_ref().map(public_participant),
                metrics: RunMetricsV1 {
                    original_score_delta: record.original_score_delta,
                    active_simulation_ticks: record.active_simulation_ticks,
                    ransom_collected: signed_mission_money(record.ransom_collected)?,
                },
            };
            run.validate().map_err(|_| ApiError::Internal)?;
            Ok(LatestRun {
                run,
                verified_at_unix_ms: record.verified_at_ms,
            })
        })
        .collect::<Result<Vec<_>, ApiError>>()?;
    Ok(Json(LatestRunsPage {
        schema_version: 1,
        runs,
    }))
}

async fn run_detail(
    State(state): State<AppState>,
    Path(run_id): Path<String>,
) -> Result<Json<RunDetailV2>, ApiError> {
    let run = state
        .database
        .public_run(&run_id, &state.config.board_ids())
        .await?;
    let board = state
        .config
        .board(&opaque(&run.board_id)?)
        .or_internal("visible run references an unconfigured board")?;
    let sim_config: CanonicalValue = serde_json::from_str(&run.sim_config_json).map_err(|_| {
        tracing::error!(
            error_code = "stored_sim_config_invalid",
            "stored run SimConfig is corrupt"
        );
        ApiError::Internal
    })?;
    let detail = RunDetailV2 {
        schema_version: SCHEMA_VERSION_V2,
        run_id: opaque(&run.run_id)?,
        board_id: board.board_id.clone(),
        mission_id: run.mission_id,
        edition: board.edition,
        metrics: RunMetricsV1 {
            original_score_delta: run.original_score_delta,
            active_simulation_ticks: run.active_simulation_ticks,
            ransom_collected: signed_mission_money(run.ransom_collected)?,
        },
        max_concurrent_players: run.max_concurrent_players,
        participant_instance_count: run.participant_instance_count,
        uploader: run.uploader.as_ref().map(public_participant),
        verified_at_unix_ms: run.verified_at_ms,
        replay: ReplayArtifactV1 {
            artifact: ArtifactRefV1 {
                sha256: Digest32::from_bytes(run.replay_sha256),
                byte_length: run.replay_bytes,
                media_type: RANKED_REPLAY_MEDIA_TYPE_V1.to_owned(),
            },
            replay_schema_version: run.replay_schema_version,
        },
        recorded_engine_version: run.recorded_engine_version.clone(),
        sim_config,
        starting_campaign_score: run.starting_campaign_score,
        final_campaign_score: run.final_campaign_score,
        achievements: run
            .achievements
            .into_iter()
            .map(|(achievement_id, evaluation)| {
                Ok(AchievementSummaryV1 {
                    display_name: achievement_id.clone(),
                    verified: PublicAchievementDecisionV1 {
                        achievement_id: opaque(&achievement_id)?,
                        evaluation,
                    },
                })
            })
            .collect::<Result<Vec<_>, ApiError>>()?,
        viewer: ViewerLaunchV2 {
            availability: ViewerAvailabilityV2::Available,
            content_requirement: board.viewer_content_requirement,
            runtime_build: run.recorded_engine_version,
        },
    };
    detail.validate().map_err(|_error| {
        tracing::error!(
            error_code = "stored_public_run_invalid",
            "stored run cannot satisfy public protocol"
        );
        ApiError::Internal
    })?;
    Ok(Json(detail))
}

async fn run_replay(
    State(state): State<AppState>,
    Path(run_id): Path<String>,
) -> Result<Response, ApiError> {
    let (digest, bytes) = state
        .database
        .replay_for_run(&run_id, &state.config.board_ids())
        .await?;
    replay_response(&state, digest, bytes).await
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
    response
        .headers_mut()
        .insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    Ok(response)
}

/// Owner-signed rename. The signed timestamp must be newer than the last
/// accepted update for the key (`409 username_update_superseded` otherwise).
async fn update_username(
    State(state): State<AppState>,
    client: ClientIp,
    Path(public_key): Path<String>,
    Json(signed): Json<SignedUsernameUpdateV2>,
) -> Result<Json<PlayerProfileV1>, ApiError> {
    rate_limit_signed_request(&state, &client, RateLimitScope::UsernameUpdate).await?;
    let update = &signed.request;
    let path_key = PublicKey32::from_str(&public_key)
        .map_err(|error| ApiError::BadRequest(error.to_string()))?;
    if path_key != update.public_key {
        return Err(ApiError::BadRequest(
            "path public key does not match signed update".to_owned(),
        ));
    }
    let username = validate_username(&update.username)
        .map_err(|message| ApiError::BadRequest(message.to_owned()))?;
    signed.verify(
        state.config.signed_requests.window(),
        crate::model::now_unix_ms()?,
    )?;
    state
        .database
        .apply_username_update(
            update.public_key.into_bytes(),
            update.signed_at_unix_ms,
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
) -> Result<Json<PlayerRunHistoryPageV2>, ApiError> {
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
    let board_ids = state.config.board_ids();
    let mut records = state
        .database
        .player_run_history(
            key.as_bytes(),
            &board_ids,
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
        let last = records
            .last()
            .or_internal("player history page with more records has no records")?;
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
    let runs = records
        .into_iter()
        .map(|record| {
            Ok(PlayerRunHistoryEntryV2 {
                player_public_key: key,
                run: RunSummaryV2 {
                    schema_version: SCHEMA_VERSION_V2,
                    run_id: opaque(&record.run_id)?,
                    board_id: opaque(&record.board_id)?,
                    mission_id: record.mission_id,
                    max_concurrent_players: record.max_concurrent_players,
                    participant_instance_count: record.participant_instance_count,
                    uploader: record.uploader.as_ref().map(public_participant),
                    metrics: RunMetricsV1 {
                        original_score_delta: record.original_score_delta,
                        active_simulation_ticks: record.active_simulation_ticks,
                        ransom_collected: signed_mission_money(record.ransom_collected)?,
                    },
                },
                verified_at_unix_ms: record.verified_at_ms,
            })
        })
        .collect::<Result<Vec<_>, ApiError>>()?;
    let personal_bests = state
        .database
        .player_personal_bests(key.as_bytes(), &board_ids, accepted_sequence_watermark)
        .await?
        .into_iter()
        .map(|best| {
            let metric = metric(&best.metric)?;
            Ok(PlayerPersonalBestV2 {
                filter: RunFilterV2 {
                    schema_version: SCHEMA_VERSION_V2,
                    board_id: opaque(&best.board_id)?,
                    mission_id: best.mission_id,
                    metric,
                    max_concurrent_players: Some(best.max_concurrent_players),
                    player_public_key: Some(key),
                },
                run_id: opaque(&best.run_id)?,
                metric_value: board_metric_value(metric, best.value)?,
            })
        })
        .collect::<Result<Vec<_>, ApiError>>()?;
    let page = PlayerRunHistoryPageV2 {
        schema_version: SCHEMA_VERSION_V2,
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

/// Owner-signed tombstone. Ownership is only checked after the signature is
/// verified, so an unauthenticated caller cannot link an anonymous run to a
/// key. Idempotent: repeating (or replaying) the request returns the stored
/// receipt with no further effect.
async fn deletion_request(
    State(state): State<AppState>,
    client: ClientIp,
    Json(signed): Json<SignedDeletionRequestV2>,
) -> Result<Json<DeletionReceiptV1>, ApiError> {
    rate_limit_signed_request(&state, &client, RateLimitScope::Deletion).await?;
    signed.verify(
        state.config.signed_requests.window(),
        crate::model::now_unix_ms()?,
    )?;
    let request = &signed.request;
    let request_json = serde_json::to_string(&signed).map_err(internal_json)?;
    let (target_kind, target_id) = match &request.target {
        DeletionTargetV1::Submission { submission_id } => ("submission", submission_id.as_str()),
        DeletionTargetV1::Run { run_id } => ("run", run_id.as_str()),
    };
    let retention = state
        .config
        .tombstone_retention_days
        .map(|days| {
            days.checked_mul(24 * 60 * 60)
                .map(Duration::from_secs)
                .or_internal("tombstone retention in seconds overflows u64")
        })
        .transpose()?;
    let deleted = state
        .database
        .apply_deletion(
            request.public_key.into_bytes(),
            request.signed_at_unix_ms,
            target_kind,
            target_id,
            &request_json,
            retention,
        )
        .await?;
    Ok(Json(DeletionReceiptV1 {
        schema_version: SCHEMA_VERSION_V1,
        deletion_request_id: opaque(&deleted.id)?,
        target: request.target.clone(),
        tombstoned_at_unix_ms: deleted.tombstoned_at_ms,
        purge_eligible_at_unix_ms: deleted.purge_eligible_at_ms,
    }))
}

async fn abuse_report(
    State(state): State<AppState>,
    client: ClientIp,
    ValidatedJson(report): ValidatedJson<AbuseReportV1>,
) -> Result<(StatusCode, Json<AbuseReportAcceptedV1>), ApiError> {
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
    let address = client.resolve(&state.config)?;
    let reporter_ip_hash =
        crate::authentication::sign(&state.cursor_hmac_key, address.to_string().as_bytes());
    let (report_id, received_at_unix_ms) = state
        .database
        .insert_abuse_report(
            target_kind,
            &target_id,
            &state.config.board_ids(),
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

fn profile(public_key: PublicKey32, username: String) -> PlayerProfileV1 {
    PlayerProfileV1 {
        schema_version: SCHEMA_VERSION_V1,
        username,
        public_key,
        public_key_fingerprint: public_key.short_fingerprint(),
    }
}

fn lifecycle_state(lifecycle: crate::model::SubmissionLifecycle) -> SubmissionLifecycleV1 {
    use crate::model::SubmissionState;
    match lifecycle.state {
        SubmissionState::Queued => SubmissionLifecycleV1::Queued,
        SubmissionState::Verifying => SubmissionLifecycleV1::Verifying,
        SubmissionState::RetryPending => SubmissionLifecycleV1::RetryPending,
        SubmissionState::Accepted { run_id } => SubmissionLifecycleV1::Accepted { run_id },
        SubmissionState::Rejected { code } => SubmissionLifecycleV1::Rejected {
            code,
            safe_message: safe_rejection_message(code).to_owned(),
        },
        SubmissionState::Failed => SubmissionLifecycleV1::Failed {
            code: SubmissionFailureCodeV1::VerificationInfrastructure,
            safe_message:
                "Verification infrastructure failed after bounded retries; the run was not rejected."
                    .to_owned(),
        },
    }
}

fn board_entry(
    row: &BoardRow,
    position: u64,
    metric: BoardMetricV1,
) -> Result<LeaderboardEntryV2, ApiError> {
    Ok(LeaderboardEntryV2 {
        position,
        rank: row.rank,
        run_id: opaque(&row.run_id)?,
        metric_value: board_metric_value(metric, row.metric_value)?,
        max_concurrent_players: row.max_concurrent_players,
        participant_instance_count: row.participant_instance_count,
        uploader: row.uploader.as_ref().map(public_participant),
        replay_sha256: Digest32::from_bytes(row.replay_sha256),
        accepted_sequence: row.accepted_sequence,
        verified_at_unix_ms: row.verified_at_ms,
    })
}

fn board_metric_value(metric: BoardMetricV1, value: i64) -> Result<BoardMetricValueV2, ApiError> {
    match metric {
        BoardMetricV1::OriginalScore => Ok(BoardMetricValueV2::OriginalScore { points: value }),
        BoardMetricV1::FastestSuccess => Ok(BoardMetricValueV2::FastestSuccess {
            active_simulation_ticks: u64::try_from(value).map_err(|_| ApiError::Internal)?,
        }),
    }
}

fn leaderboard_cursor(
    cursor: &CursorToken,
    opaque_token: String,
    metric: BoardMetricV1,
) -> Result<LeaderboardCursorV2, ApiError> {
    Ok(LeaderboardCursorV2 {
        schema_version: SCHEMA_VERSION_V2,
        query_sha256: cursor.filter_sha256,
        accepted_sequence_watermark: cursor.accepted_sequence_watermark,
        last: LeaderboardOrderAnchorV2 {
            position: cursor.position,
            rank: cursor.rank,
            metric_value: board_metric_value(metric, cursor.metric_value)?,
            accepted_sequence: u64::try_from(cursor.accepted_sequence)
                .map_err(|_| ApiError::Internal)?,
            verified_at_unix_ms: cursor.verified_at_unix_ms,
            run_id: opaque(&cursor.run_id)?,
        },
        opaque_token,
    })
}

/// The uploader occupies seat zero of every published run.
fn public_participant(identity: &PublicIdentity) -> PublicParticipantV1 {
    let public_key = PublicKey32::from_bytes(identity.public_key);
    PublicParticipantV1 {
        seat: 0,
        username: identity.username.clone(),
        public_key,
        public_key_fingerprint: public_key.short_fingerprint(),
    }
}

/// Per-address, per-operation metering for username, deletion and private
/// status requests, applied before signature verification.
async fn rate_limit_signed_request(
    state: &AppState,
    client: &ClientIp,
    scope: RateLimitScope,
) -> Result<(), ApiError> {
    let address = client.resolve(&state.config)?;
    state
        .rate_limiter
        .check(
            scope,
            RateLimitSubject::Address(address),
            state.config.signed_requests_per_minute_per_ip,
            RATE_WINDOW_MINUTE,
        )
        .await
}

fn metric(value: &str) -> Result<BoardMetricV1, ApiError> {
    match value {
        "original_score" => Ok(BoardMetricV1::OriginalScore),
        "fastest_success" => Ok(BoardMetricV1::FastestSuccess),
        _ => Err(ApiError::Internal),
    }
}

fn opaque(value: &str) -> Result<OpaqueId, ApiError> {
    OpaqueId::new(value).map_err(|error| ApiError::BadRequest(error.to_string()))
}

fn filter_digest(filter: &RunFilterV2) -> Result<Digest32, ApiError> {
    filter
        .canonical_digest()
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
        VerificationRejectionCodeV1::ContentNotAllowed => {
            "The replay does not use official board content."
        }
        VerificationRejectionCodeV1::ConfigMismatch => {
            "The replay settings do not match the board."
        }
        VerificationRejectionCodeV1::StartingStateMismatch => {
            "The replay does not start from an official fresh mission start."
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

async fn submit_diagnostic(
    State(state): State<AppState>,
    client: ClientIp,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<
    (
        StatusCode,
        Json<robin_run_protocol::diagnostics::DiagnosticReceiptV1>,
    ),
    ApiError,
> {
    let encoding = match headers
        .get(axum::http::header::CONTENT_ENCODING)
        .and_then(|value| value.to_str().ok())
    {
        Some("zstd") => "zstd",
        Some("gzip") => "gzip",
        _ => {
            return Err(ApiError::BadRequest(
                "diagnostic uploads require Content-Encoding: zstd or gzip".into(),
            ));
        }
    };
    let address = client.resolve(&state.config)?;
    let ip_hash =
        crate::authentication::sign(&state.cursor_hmac_key, address.to_string().as_bytes());
    Ok((
        StatusCode::ACCEPTED,
        Json(
            state
                .database
                .insert_diagnostic(
                    body.to_vec(),
                    encoding,
                    headers
                        .get("x-diagnostic-kind")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or(""),
                    headers
                        .get("x-diagnostic-engine-commit")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or(""),
                    ip_hash,
                )
                .await
                .map_err(|error| match error {
                    crate::db::DbError::ResultInvariant(message) => ApiError::BadRequest(message),
                    other => ApiError::from(other),
                })?,
        ),
    ))
}

async fn operator_diagnostics(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<crate::db::DiagnosticSummary>>, ApiError> {
    authorize_operator(&state, &headers)?;
    Ok(Json(state.database.diagnostic_reports().await?))
}

async fn operator_diagnostic(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    authorize_operator(&state, &headers)?;
    let payload = state.database.diagnostic_report(&id).await?;
    let extension = match payload.encoding.as_str() {
        "zstd" => "json.zst",
        "gzip" => "json.gz",
        "identity" => "json",
        _ => return Err(ApiError::Internal),
    };
    // No Content-Encoding: downloads preserve compressed bytes even in browsers.
    Ok((
        [
            ("content-type", "application/octet-stream".to_owned()),
            (
                "content-disposition",
                format!("attachment; filename=\"report.{extension}\""),
            ),
            ("x-diagnostic-encoding", payload.encoding),
        ],
        payload.bytes,
    )
        .into_response())
}

async fn operator_delete_diagnostic(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    authorize_operator(&state, &headers)?;
    state.database.delete_diagnostic_report(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

// Stored verifier evidence retains the engine's wrapping 32-bit counter.
// Public money statistics use its signed net value, including spending.
fn signed_mission_money(raw: u64) -> Result<i64, ApiError> {
    let bits = u32::try_from(raw).map_err(|_| {
        tracing::error!(raw, "stored mission money exceeds its 32-bit counter");
        ApiError::Internal
    })?;
    Ok(i64::from(bits as i32))
}

#[cfg(test)]
mod tests;
