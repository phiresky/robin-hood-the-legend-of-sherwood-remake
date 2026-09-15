//! Submission workflow: authentication, board admission, reservation, and finalization.
//!
//! The transport lexically preflights its bounded opaque replay before entering
//! this workflow. The artifact ingestion closure is invoked only after
//! acquiring the current reservation; completed/busy retries never invoke it.
use crate::{
    config::ServerConfig,
    db::{DbError, SubmissionUploadIntent, SubmissionUploadLease, SubmissionUploadReservation},
    error::ApiError,
    model::{NewSubmission, SubmissionLifecycle},
};
use robin_run_protocol::{ParticipantPublicDisclosureV1, SignedSubmissionV3, canonical_json_bytes};

/// Proof that a signed submission passed shape validation, the freshness
/// window, its uploader's signature and the configured board. Deliberately not
/// serializable: a stored record is data, never a way to recreate
/// authentication.
pub(crate) struct AuthenticatedSubmission {
    intent: SubmissionUploadIntent,
    projection: NewSubmission,
}

impl AuthenticatedSubmission {
    pub(crate) fn replay_bytes(&self) -> u64 {
        self.projection.replay_bytes
    }

    pub(crate) fn uploader_public_key(&self) -> [u8; 32] {
        self.projection.uploader_public_key
    }
}

/// Validate, authenticate and board-admit one decoded signed submission at
/// server time `now_unix_ms`.
pub(crate) fn authenticate(
    signed: &SignedSubmissionV3,
    config: &ServerConfig,
    now_unix_ms: u64,
) -> Result<AuthenticatedSubmission, ApiError> {
    signed.verify(config.signed_requests.window(), now_unix_ms)?;
    let submission = &signed.request;
    let board = config
        .board(&submission.board_id)
        .ok_or_else(|| ApiError::BadRequest(format!("unknown board `{}`", submission.board_id)))?;
    if board.mission(&submission.mission_id).is_none() {
        return Err(ApiError::BadRequest(format!(
            "mission `{}` is not part of board `{}`",
            submission.mission_id, submission.board_id
        )));
    }
    if submission
        .requested_metrics
        .iter()
        .any(|metric| !board.metrics.contains(metric))
    {
        return Err(ApiError::BadRequest(
            "requested metrics are not offered by the board".to_owned(),
        ));
    }
    let signed_request_json =
        String::from_utf8(canonical_json_bytes(signed)?).map_err(|_| ApiError::Internal)?;
    let replay = &submission.replay.artifact;
    let uploader_public_key = submission.uploader_public_key.into_bytes();
    let proposed_submission_id = uuid::Uuid::now_v7().to_string();
    Ok(AuthenticatedSubmission {
        intent: SubmissionUploadIntent {
            proposed_submission_id: proposed_submission_id.clone(),
            signed_request_json: signed_request_json.clone(),
            uploader_public_key,
            replay_sha256: replay.sha256.into_bytes(),
        },
        projection: NewSubmission {
            id: proposed_submission_id,
            signed_request_json,
            uploader_public_key,
            public_disclosure: match submission.public_disclosure {
                ParticipantPublicDisclosureV1::NamedProfile => "named_profile",
                ParticipantPublicDisclosureV1::Anonymous => "anonymous",
            },
            board_id: submission.board_id.as_str().to_owned(),
            mission_id: submission.mission_id.clone(),
            replay_sha256: replay.sha256.into_bytes(),
            replay_bytes: replay.byte_length,
            replay_schema_version: submission.replay.replay_schema_version,
            requested_metrics_json: serde_json::to_string(&submission.requested_metrics)
                .map_err(|_| ApiError::Internal)?,
        },
    })
}

/// Own the reserve-to-finalize transition without depending on multipart.
/// Readiness is sampled immediately before the reservation transaction.
/// Completed exact retries remain observable during unavailable admission;
/// only acquired leases reach the caller's artifact I/O.
pub(crate) async fn complete_upload<A, I, F>(
    database: &crate::Database,
    authenticated: &AuthenticatedSubmission,
    lease_ttl: std::time::Duration,
    reservation_ttl: std::time::Duration,
    admission: A,
    ingestion: I,
) -> Result<SubmissionLifecycle, ApiError>
where
    A: std::future::Future<Output = Result<(), ApiError>>,
    I: FnOnce(bool) -> F,
    F: std::future::Future<Output = Result<(), ApiError>>,
{
    let admission_error = admission.await.err();
    let reservation = database
        .reserve_submission_upload_if_admitted(
            &authenticated.intent,
            lease_ttl,
            reservation_ttl,
            admission_error.is_none(),
        )
        .await;
    let reservation = match reservation {
        Ok(reservation) => reservation,
        Err(DbError::AdmissionUnavailable) => {
            return Err(admission_error.unwrap_or(ApiError::Unavailable));
        }
        Err(error) => return Err(error.into()),
    };
    match reservation {
        SubmissionUploadReservation::Existing { lifecycle } => Ok(lifecycle),
        SubmissionUploadReservation::Busy { retry_after_ms } => {
            Err(ApiError::UploadInProgress { retry_after_ms })
        }
        SubmissionUploadReservation::Acquired {
            lease,
            resume_uploaded,
        } => {
            finish_reserved_upload(
                database,
                authenticated,
                &lease,
                resume_uploaded,
                ingestion(resume_uploaded),
            )
            .await
        }
    }
}

async fn finish_reserved_upload(
    database: &crate::Database,
    authenticated: &AuthenticatedSubmission,
    lease: &SubmissionUploadLease,
    resume_uploaded: bool,
    ingestion: impl std::future::Future<Output = Result<(), ApiError>>,
) -> Result<SubmissionLifecycle, ApiError> {
    if let Err(error) = ingestion.await {
        abandon_failed_upload(database, lease).await;
        return Err(error);
    }
    if !resume_uploaded && let Err(error) = database.mark_submission_upload_uploaded(lease).await {
        abandon_failed_upload(database, lease).await;
        return Err(error.into());
    }
    // A resumed reservation keeps its original canonical submission ID rather
    // than the retry's proposal.
    let mut submission = authenticated.projection.clone();
    submission.id = lease.submission_id.clone();
    Ok(database
        .finalize_submission_upload(&submission, lease)
        .await?)
}

async fn abandon_failed_upload(database: &crate::Database, lease: &SubmissionUploadLease) {
    match database.abandon_submission_upload(lease).await {
        Ok(true) => {}
        Ok(false) => tracing::warn!("failed upload no longer held its reservation lease"),
        Err(error) => tracing::error!(
            error_code = error.safe_log_code(),
            "could not release failed upload reservation"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::AuthenticatedSubmission;

    #[test]
    fn authentication_authority_cannot_be_deserialized() {
        // Inference has exactly one implementation today. Adding Deserialize
        // creates a second candidate and makes this test fail to compile.
        trait AmbiguousIfDeserialize<Marker> {
            fn check() {}
        }
        impl<T: ?Sized> AmbiguousIfDeserialize<()> for T {}
        enum Deserializable {}
        impl<T: serde::Deserialize<'static>> AmbiguousIfDeserialize<Deserializable> for T {}
        let _ = <AuthenticatedSubmission as AmbiguousIfDeserialize<_>>::check;
    }
}
