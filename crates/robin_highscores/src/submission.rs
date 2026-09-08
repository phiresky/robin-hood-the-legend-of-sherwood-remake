//! Submission workflow: immutable identity, admission, reservation, and finalization.
//!
//! The transport authenticates and lexically preflights its bounded opaque replay
//! before entering this workflow. The artifact ingestion closure is invoked only
//! after acquiring the current reservation; committed/busy retries never invoke it.
use crate::{
    config::AdmissionProfile,
    db::{DbError, SubmissionUploadLease},
    error::ApiError,
    model::{NewSubmission, ParticipantClaim},
};
use robin_run_protocol::{
    CanonicalDocument, Digest32, InitialStateExpectationV1, ParticipantPublicDisclosureV1,
    SignedSubmissionV1, SubmissionOfferV1, Validate,
};

/// Proof of signature verification against the server's exact offer. This owner
/// deliberately has no serialization implementation: serialized records are data,
/// not a way to recreate authentication. The borrow prevents mutation of the
/// authenticated document while reservation/finalization use its one projection.
pub(crate) struct AuthenticatedSubmission<'a> {
    signed: &'a SignedSubmissionV1,
    projection: crate::db::SubmissionUploadIntent,
}

/// Authentication is independent; finalization checks the live lease in its transaction.
/// Redundant storage fields are derived here, never supplied by the HTTP adapter.
fn prepare_submission(
    authenticated: &AuthenticatedSubmission<'_>,
    lease: &SubmissionUploadLease,
) -> Result<NewSubmission, DbError> {
    let signed = authenticated.signed;
    let projection = &authenticated.projection;
    signed
        .validate()
        .map_err(|error| DbError::ResultInvariant(error.to_string()))?;
    let reserved_offer: SubmissionOfferV1 =
        serde_json::from_str(&lease.offer_json).map_err(stored_json_error)?;
    if reserved_offer != signed.submission.offer
        || lease.upload_challenge_id != signed.submission.offer.upload_challenge_id.as_str()
    {
        return Err(DbError::SubmissionConflict);
    }
    let artifacts = &signed.submission.artifacts;
    let envelope_json = projection.envelope_json.clone();
    let signatures_json =
        serde_json::to_string(&signed.participant_signatures).map_err(stored_json_error)?;
    let (scope_kind, chain_id, predecessor_id) = starting_state_storage(&signed.submission.offer);
    let admission_profile: AdmissionProfile =
        serde_json::from_str(&lease.public_metadata_json).map_err(stored_json_error)?;
    if admission_profile.canonical_campaign_state.requirement
        != signed
            .submission
            .offer
            .starting_state
            .campaign_state_requirement()
    {
        return Err(DbError::ResultInvariant(
            "signed offer campaign-state authority differs from its admission profile".to_owned(),
        ));
    }
    let canonical_campaign_state_json =
        serde_json::to_string(&admission_profile.canonical_campaign_state)
            .map_err(stored_json_error)?;
    let submission = NewSubmission {
        id: lease.submission_id.clone(),
        upload_challenge_id: signed
            .submission
            .offer
            .upload_challenge_id
            .as_str()
            .to_owned(),
        offer_json: lease.offer_json.clone(),
        envelope_json,
        signatures_json,
        public_metadata_json: lease.public_metadata_json.clone(),
        replay_sha256: artifacts.replay.artifact.sha256.into_bytes(),
        replay_bytes: artifacts.replay.artifact.byte_length,
        build_manifest_id: signed.submission.offer.build_manifest_sha256.into_bytes(),
        content_manifest_id: signed.submission.offer.content_manifest_sha256.into_bytes(),
        campaign_content_manifest_id: signed
            .submission
            .offer
            .session_genesis
            .claim
            .ranked_session
            .campaign_content_manifest_sha256
            .map(Digest32::into_bytes),
        config_id: signed.submission.offer.rules_config_sha256.into_bytes(),
        ruleset_id: signed.submission.offer.ruleset_manifest_sha256.into_bytes(),
        mission_id: signed.submission.offer.mission_id.clone(),
        scope_kind: scope_kind.to_owned(),
        starting_campaign_sha256: artifacts.starting_campaign.sha256.into_bytes(),
        starting_campaign_bytes: artifacts.starting_campaign.byte_length,
        canonical_campaign_state_json,
        controller_public_key: projection.controller_public_key,
        starting_state_json: serde_json::to_string(&signed.submission.offer.starting_state)
            .map_err(stored_json_error)?,
        campaign_chain_id: chain_id,
        predecessor_run_id: predecessor_id,
        competition_manifest_id: signed
            .submission
            .offer
            .competition_manifest_sha256
            .map(Digest32::into_bytes),
        requested_metrics_json: serde_json::to_string(&signed.submission.requested_metrics)
            .map_err(stored_json_error)?,
        participant_claims_json: serde_json::to_string(&signed.submission.offer.participant_claims)
            .map_err(stored_json_error)?,
        max_concurrent_players: signed.submission.offer.max_concurrent_players,
        participant_instance_count: signed.submission.offer.participant_instance_count,
        session_genesis_sha256: projection.session_genesis_sha256,
        session_genesis_host_public_key: projection.session_genesis_host_public_key,
        replay_session_id: projection.replay_session_id,
        session_genesis_host_nonce: projection.session_genesis_host_nonce,
        participants: projection.participants.clone(),
    };
    Ok(submission)
}

/// Verify authorizing proofs against the server's exact immutable offer.
/// Kept separate from transport parsing and from transaction-time lease checks.
pub(crate) fn authenticate_reserved_offer<'a>(
    signed: &'a SignedSubmissionV1,
    offer_json: &str,
) -> Result<AuthenticatedSubmission<'a>, crate::error::ApiError> {
    use crate::{error::ApiError, identity::verify_signature};
    // The HTTP adapter already performs this before expiry/storage lookup.
    // Retain that ordering there, while making this sole proof constructor safe
    // for any future caller: shape errors are not cryptographic rejections.
    signed
        .validate()
        .map_err(|error| ApiError::BadRequest(error.to_string()))?;
    let authoritative_offer: SubmissionOfferV1 =
        serde_json::from_str(offer_json).map_err(|_error| {
            tracing::error!(
                error_code = "stored_offer_json_invalid",
                "stored offer JSON is corrupt"
            );
            ApiError::Internal
        })?;
    if authoritative_offer != signed.submission.offer {
        return Err(ApiError::Conflict(
            "signed offer does not match the server-issued offer".to_owned(),
        ));
    }
    let signing_bytes = signed
        .signing_bytes()
        .map_err(|error| ApiError::BadRequest(error.to_string()))?;
    for participant in &signed.participant_signatures {
        verify_signature(
            participant.public_key.as_bytes(),
            participant.signature.as_bytes(),
            &signing_bytes,
        )
        .map_err(|_| ApiError::Unauthorized)?;
    }
    if let Some(authorization) = &signed.submission.campaign_continuation_authorization {
        let bytes = authorization
            .signing_bytes(&signed.submission.offer)
            .map_err(|error| ApiError::BadRequest(error.to_string()))?;
        verify_signature(
            authorization
                .claim
                .campaign_controller_public_key
                .as_bytes(),
            authorization.signature.as_bytes(),
            &bytes,
        )
        .map_err(|_| ApiError::Unauthorized)?;
    }
    Ok(AuthenticatedSubmission {
        signed,
        projection: upload_intent(signed)?,
    })
}

fn stored_json_error(error: serde_json::Error) -> DbError {
    DbError::Corrupt(format!("submission persistence document: {error}"))
}

/// Own the reserve-to-finalize transition without depending on multipart.
/// Readiness is sampled immediately before the reservation transaction. Exact
/// retries remain observable during unavailable admission; only acquired leases
/// reach the caller's artifact I/O. The caller retains its outer database fence.
pub(crate) async fn complete_upload<A, I, F>(
    database: &crate::Database,
    authenticated: &AuthenticatedSubmission<'_>,
    lease_ttl: std::time::Duration,
    reservation_ttl: std::time::Duration,
    admission: A,
    ingestion: I,
) -> Result<crate::model::SubmissionLifecycle, crate::error::ApiError>
where
    A: std::future::Future<Output = Result<(), crate::error::ApiError>>,
    I: FnOnce(bool) -> F,
    F: std::future::Future<Output = Result<(), crate::error::ApiError>>,
{
    use crate::{db::SubmissionUploadReservation, error::ApiError};
    let intent = &authenticated.projection;
    let admission_error = admission.await.err();
    let reservation = database
        .reserve_submission_upload_if_admitted(
            intent,
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
            debug_assert!(resume_uploaded || admission_error.is_none());
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

/// Complete one reserved upload. The adapter supplies bounded artifact I/O as
/// a future, so this workflow owns failure cleanup and durable transitions
/// without depending on multipart or buffering a campaign itself.
async fn finish_reserved_upload(
    database: &crate::Database,
    authenticated: &AuthenticatedSubmission<'_>,
    lease: &SubmissionUploadLease,
    resume_uploaded: bool,
    ingestion: impl std::future::Future<Output = Result<(), crate::error::ApiError>>,
) -> Result<crate::model::SubmissionLifecycle, crate::error::ApiError> {
    if let Err(error) = ingestion.await {
        abandon_failed_upload(database, lease).await;
        return Err(error);
    }
    if !resume_uploaded {
        if let Err(error) = database.mark_submission_upload_uploaded(lease).await {
            abandon_failed_upload(database, lease).await;
            return Err(error.into());
        }
    }
    let submission = prepare_submission(authenticated, lease)?;
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

fn starting_state_storage(
    offer: &SubmissionOfferV1,
) -> (&'static str, Option<String>, Option<String>) {
    match &offer.starting_state {
        InitialStateExpectationV1::IndividualLevel { .. } => ("individual_level", None, None),
        InitialStateExpectationV1::CampaignGenesis { .. } => {
            ("campaign", Some(uuid::Uuid::now_v7().to_string()), None)
        }
        InitialStateExpectationV1::CampaignContinuation {
            chain_id,
            predecessor_run_id,
            ..
        } => (
            "campaign",
            Some(chain_id.as_str().to_owned()),
            Some(predecessor_run_id.as_str().to_owned()),
        ),
    }
}

/// Derive the immutable upload identity once, outside the HTTP adapter.
fn upload_intent(
    signed: &SignedSubmissionV1,
) -> Result<crate::db::SubmissionUploadIntent, crate::error::ApiError> {
    let offer_json = serde_json::to_string(&signed.submission.offer).map_err(stored_json_error)?;
    let envelope_json = serde_json::to_string(&signed).map_err(stored_json_error)?;
    let controller_public_key = signed
        .submission
        .campaign_continuation_authorization
        .as_ref()
        .map(|authorization| authorization.claim.campaign_controller_public_key)
        .unwrap_or(
            signed
                .submission
                .offer
                .session_genesis
                .claim
                .host_public_key,
        )
        .into_bytes();
    let session_genesis_sha256 = signed
        .submission
        .offer
        .session_genesis
        .canonical_digest()
        .map_err(|error| ApiError::BadRequest(error.to_string()))?
        .into_bytes();
    let session_genesis_host_public_key = signed
        .submission
        .offer
        .session_genesis
        .claim
        .host_public_key
        .into_bytes();
    let replay_session_id = signed
        .submission
        .offer
        .session_genesis
        .claim
        .replay_session_id
        .into_bytes();
    let session_genesis_host_nonce = signed
        .submission
        .offer
        .session_genesis
        .claim
        .host_nonce
        .into_bytes();
    let participants = signed
        .submission
        .offer
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
    Ok(crate::db::SubmissionUploadIntent {
        proposed_submission_id: uuid::Uuid::now_v7().to_string(),
        upload_challenge_id: signed
            .submission
            .offer
            .upload_challenge_id
            .as_str()
            .to_owned(),
        offer_json,
        envelope_json,
        controller_public_key,
        session_genesis_sha256,
        session_genesis_host_public_key,
        replay_session_id,
        session_genesis_host_nonce,
        participants,
    })
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
        let _ = <AuthenticatedSubmission<'static> as AmbiguousIfDeserialize<_>>::check;
    }
}
