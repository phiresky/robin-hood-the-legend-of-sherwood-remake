//! Fail-closed implementation of the one-job verifier child boundary.
//!
//! Authenticated jobs pass through exact artifact, configuration, content,
//! campaign, and replay cross-binding before the sealed deterministic game
//! adapter can construct an engine. `Verified` is produced only from that
//! engine's terminal state and exact final campaign bytes.

use std::io::{self, Read as _};
use std::path::Path;

use robin_run_protocol::{
    ArtifactRefV1, CampaignCompleteEvidenceV1, CanonicalDocument as _, Digest32,
    InputProvenanceStatusV1, InputTaintKindV1, InputTaintV1, OfficialContentEditionV1,
    OfficialContentSubjectV1, ParticipantPublicDisclosureV1, RANKED_CAMPAIGN_MEDIA_TYPE_V1,
    RunScopeKindV1, TerminalOutcomeV1, Validate as _, VerificationInfrastructureFailureCodeV1,
    VerificationInfrastructureFailureV1, VerificationRejectionCodeV1, VerificationRejectionV1,
    VerificationRequestV1, VerificationResultV1, VerificationStatusV1, VerifiedRunV1,
    VerifierAdmissionFailureCodeV1, VerifierWorkerOutputV1,
};
use sha2::{Digest as _, Sha256};

use crate::job_config::{JobConfigError, ValidatedJobConfig, read_job_config, validate_job_config};
use crate::request_auth::{AuthenticatedVerificationRequest, authenticate_verification_request};
use crate::worker_process::{
    AtomicOutputError, WorkerPaths, truncate_output, write_truncated_output,
};

/// Compiled, unauthenticated request ceiling. The entire artifact is still
/// streamed through SHA-256 after this boundary is crossed, but no more bytes
/// are retained for JSON decoding.
pub const MAX_WORKER_REQUEST_BYTES: usize = 1024 * 1024;

/// Compiled campaign ceiling. A signed request may only lower this bound.
pub const MAX_WORKER_CAMPAIGN_BYTES: usize = 64 * 1024 * 1024;

/// Compiled replay transport ceiling shared with the sole canonical codec.
/// Per-job operator limits may lower, but never raise, this value.
pub const MAX_WORKER_REPLAY_BYTES: usize =
    robin_replay_format::DEFAULT_REPLAY_ADMISSION_LIMITS.max_input_bytes;

/// Compiled ceiling for the one canonical worker output document.
pub const MAX_WORKER_RESULT_BYTES: usize = 1024 * 1024;

const STREAM_BUFFER_BYTES: usize = 64 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum WorkerRunError {
    #[error("reading verifier request artifact failed: {0}")]
    RequestRead(#[source] io::Error),
    #[error("worker output canonicalization failed: {0}")]
    OutputCanonicalization(String),
    #[error("worker output I/O failed: {0}")]
    Output(#[from] AtomicOutputError),
}

#[derive(Debug)]
struct BoundedArtifact {
    sha256: Digest32,
    byte_length: u64,
    retained_bytes: Option<Vec<u8>>,
}

#[derive(Debug)]
struct CampaignOutputTransactionFailure {
    detail: &'static str,
    rollback_error: Option<AtomicOutputError>,
}

/// Run one already path-admitted verifier job.
///
/// Every pre-created output is cleared before any hostile document is read.
/// The final campaign output remains empty unless the authenticated job
/// reaches one genuine, validated `Verified` result.
pub fn run_one_job(paths: &WorkerPaths) -> Result<(), WorkerRunError> {
    // Clear the proof first, then all correlated campaign artifacts. If any
    // later clear fails there is no result document which could authenticate
    // stale bytes from a previous job.
    truncate_output(&paths.result)?;
    if let Err(error) = truncate_campaign_outputs(paths) {
        let _ = truncate_campaign_outputs(paths);
        return Err(WorkerRunError::Output(error));
    }

    let request_artifact = stream_request(&paths.request, MAX_WORKER_REQUEST_BYTES)
        .map_err(WorkerRunError::RequestRead)?;
    let mut fatal_campaign_rollback = None;
    let output = admit_request(paths, request_artifact, &mut fatal_campaign_rollback)?;
    if let Some(error) = fatal_campaign_rollback {
        return Err(WorkerRunError::Output(error));
    }

    // `canonical_bytes` validates and serializes exactly once. The bounded
    // writer performs the sole byte-cap check before truncating, writing, and
    // flushing the pre-created result target.
    let output_bytes = match output.canonical_bytes() {
        Ok(bytes) => bytes,
        Err(error) => {
            let _ = truncate_campaign_outputs(paths);
            return Err(WorkerRunError::OutputCanonicalization(error.to_string()));
        }
    };
    write_result_with_campaign_rollback(paths, &output_bytes, MAX_WORKER_RESULT_BYTES)
        .map_err(WorkerRunError::Output)?;
    Ok(())
}

fn write_result_with_campaign_rollback(
    paths: &WorkerPaths,
    bytes: &[u8],
    maximum_bytes: usize,
) -> Result<(), AtomicOutputError> {
    match write_truncated_output(&paths.result, bytes, maximum_bytes) {
        Ok(()) => Ok(()),
        Err(write_error) => match truncate_campaign_outputs(paths) {
            Ok(()) => Err(write_error),
            Err(rollback_error) => Err(rollback_error),
        },
    }
}

fn truncate_campaign_outputs(paths: &WorkerPaths) -> Result<(), AtomicOutputError> {
    truncate_output(&paths.final_campaign)
}

fn write_verified_campaign_outputs(
    paths: &WorkerPaths,
    final_campaign: &[u8],
    maximum_bytes: usize,
) -> Result<(), CampaignOutputTransactionFailure> {
    if write_truncated_output(&paths.final_campaign, final_campaign, maximum_bytes).is_err() {
        return Err(CampaignOutputTransactionFailure {
            detail: "final_campaign_output_failed",
            rollback_error: truncate_campaign_outputs(paths).err(),
        });
    }
    Ok(())
}

fn admit_request(
    paths: &WorkerPaths,
    request_artifact: BoundedArtifact,
    fatal_campaign_rollback: &mut Option<AtomicOutputError>,
) -> Result<VerifierWorkerOutputV1, WorkerRunError> {
    let Some(request_bytes) = request_artifact.retained_bytes else {
        return Ok(admission_failure(
            request_artifact.sha256,
            VerifierAdmissionFailureCodeV1::RequestTooLarge,
            "request_exceeds_worker_cap",
        ));
    };

    let request = match robin_run_protocol::strict_json::from_slice::<VerificationRequestV1>(
        &request_bytes,
    ) {
        Ok(request) => request,
        Err(error) => {
            let detail = if matches!(
                error,
                robin_run_protocol::strict_json::StrictJsonError::DuplicateKey(_)
            ) {
                "duplicate_json_key"
            } else {
                "malformed_json"
            };
            return Ok(admission_failure(
                request_artifact.sha256,
                VerifierAdmissionFailureCodeV1::MalformedRequest,
                detail,
            ));
        }
    };

    if request.schema_version != robin_run_protocol::SCHEMA_VERSION_V1 {
        return Ok(admission_failure(
            request_artifact.sha256,
            VerifierAdmissionFailureCodeV1::UnsupportedRequestSchema,
            "unsupported_request_schema",
        ));
    }

    let authenticated = match authenticate_verification_request(request) {
        Ok(authenticated) => authenticated,
        Err(_) => {
            return Ok(admission_failure(
                request_artifact.sha256,
                VerifierAdmissionFailureCodeV1::RequestAuthenticationFailed,
                "protocol_or_signature_invalid",
            ));
        }
    };

    Ok(admit_authenticated_job(
        paths,
        authenticated,
        request_artifact.sha256,
        fatal_campaign_rollback,
    ))
}

fn admission_failure(
    request_artifact_sha256: Digest32,
    code: VerifierAdmissionFailureCodeV1,
    detail: &'static str,
) -> VerifierWorkerOutputV1 {
    VerifierWorkerOutputV1::AdmissionFailure {
        schema_version: robin_run_protocol::SCHEMA_VERSION_V1,
        request_artifact_sha256,
        code,
        bounded_detail: Some(detail.into()),
    }
}

fn admit_authenticated_job(
    paths: &WorkerPaths,
    authenticated: AuthenticatedVerificationRequest,
    raw_request_sha256: Digest32,
    fatal_campaign_rollback: &mut Option<AtomicOutputError>,
) -> VerifierWorkerOutputV1 {
    let request = authenticated.request();
    let submission = &request.submission.submission;
    let offer = &submission.offer;
    let replay_claim = &submission.artifacts.replay;

    let replay_cap = usize::try_from(request.limits.max_input_bytes)
        .unwrap_or(usize::MAX)
        .min(MAX_WORKER_REPLAY_BYTES);
    let campaign_cap = usize::try_from(request.limits.max_campaign_bytes)
        .unwrap_or(usize::MAX)
        .min(MAX_WORKER_CAMPAIGN_BYTES);

    // Observe both externally supplied artifacts before interpreting either
    // uploader-controlled identity. Short reads are retryable supervisor
    // failures; exact-but-wrong bytes are authenticated rejections.
    let replay_read = stream_request(&paths.replay, replay_cap);
    let campaign_read = stream_request(&paths.starting_campaign, campaign_cap);
    let replay = match replay_read {
        Ok(replay) => replay,
        Err(_) => {
            return admission_failure(
                raw_request_sha256,
                VerifierAdmissionFailureCodeV1::WorkerInternalFailure,
                "replay_artifact_io",
            );
        }
    };
    let campaign = match campaign_read {
        Ok(campaign) => campaign,
        Err(_) => {
            return admission_failure(
                raw_request_sha256,
                VerifierAdmissionFailureCodeV1::WorkerInternalFailure,
                "starting_campaign_artifact_io",
            );
        }
    };
    for (observed, claimed, detail) in [
        (
            replay.byte_length,
            replay_claim.artifact.byte_length,
            "replay_artifact_incomplete",
        ),
        (
            campaign.byte_length,
            submission.artifacts.starting_campaign.byte_length,
            "starting_campaign_artifact_incomplete",
        ),
    ] {
        if observed < claimed {
            return admission_failure(
                raw_request_sha256,
                VerifierAdmissionFailureCodeV1::WorkerInternalFailure,
                detail,
            );
        }
    }

    if replay.byte_length != replay_claim.artifact.byte_length
        || replay.sha256 != replay_claim.artifact.sha256
    {
        return authenticated_output(
            authenticated,
            replay.sha256,
            rejection(
                VerificationRejectionCodeV1::MalformedReplay,
                "replay_artifact_identity_mismatch",
            ),
        );
    }
    if replay.byte_length > request.limits.max_input_bytes {
        return authenticated_output(
            authenticated,
            replay.sha256,
            rejection(
                VerificationRejectionCodeV1::ResourceLimit,
                "replay_exceeds_signed_input_limit",
            ),
        );
    }
    if campaign.byte_length > request.limits.max_campaign_bytes {
        return authenticated_output(
            authenticated,
            replay.sha256,
            rejection(
                VerificationRejectionCodeV1::ResourceLimit,
                "starting_campaign_exceeds_signed_limit",
            ),
        );
    }
    let Some(campaign_bytes) = campaign.retained_bytes else {
        return authenticated_output(
            authenticated,
            replay.sha256,
            rejection(
                VerificationRejectionCodeV1::ResourceLimit,
                "starting_campaign_exceeds_worker_cap",
            ),
        );
    };
    if campaign.sha256 != submission.artifacts.starting_campaign.sha256
        || campaign.byte_length != submission.artifacts.starting_campaign.byte_length
        || campaign.sha256 != offer.starting_state.campaign_sha256()
    {
        return authenticated_output(
            authenticated,
            replay.sha256,
            rejection(
                VerificationRejectionCodeV1::StartingStateMismatch,
                "starting_campaign_identity_mismatch",
            ),
        );
    }

    let config = match read_job_config(&paths.config) {
        Ok(config) => config,
        Err(error) => {
            return authenticated_output(authenticated, replay.sha256, job_config_failure(&error));
        }
    };
    let verifier_executable = match std::env::current_exe() {
        Ok(path) => path,
        Err(_) => {
            return authenticated_output(
                authenticated,
                replay.sha256,
                infrastructure_failure(
                    VerificationInfrastructureFailureCodeV1::WorkerInternalFailure,
                    "verifier_executable_unavailable",
                ),
            );
        }
    };
    let validated_config = match validate_job_config(config, request, &verifier_executable) {
        Ok(config) => config,
        Err(error) => {
            return authenticated_output(authenticated, replay.sha256, job_config_failure(&error));
        }
    };
    let Some(replay_bytes) = replay.retained_bytes else {
        return authenticated_output(
            authenticated,
            replay.sha256,
            rejection(
                VerificationRejectionCodeV1::ResourceLimit,
                "replay_exceeds_worker_cap",
            ),
        );
    };
    let replay_data = match decode_replay(&replay_bytes, request, replay_claim) {
        Ok(replay) => replay,
        Err(failure) => {
            return authenticated_output(
                authenticated,
                replay.sha256,
                rejection(failure.code, failure.detail),
            );
        }
    };
    let canonical_replay = match canonical_replay_artifact_bytes(&replay_data) {
        Ok(bytes) => bytes,
        Err(_) => {
            return authenticated_output(
                authenticated,
                replay.sha256,
                rejection(
                    VerificationRejectionCodeV1::MalformedReplay,
                    "replay_canonicalization_failed",
                ),
            );
        }
    };
    if canonical_replay != replay_bytes {
        return authenticated_output(
            authenticated,
            replay.sha256,
            rejection(
                VerificationRejectionCodeV1::MalformedReplay,
                "replay_is_not_canonical",
            ),
        );
    }
    if replay_data.validate_ranked_hash_coverage().is_err() {
        return authenticated_output(
            authenticated,
            replay.sha256,
            rejection(
                VerificationRejectionCodeV1::StateHashMismatch,
                "replay_hash_coverage_invalid",
            ),
        );
    }
    if replay_data.contains_state_loads()
        && !validated_config
            .config()
            .template
            .ruleset_manifest
            .allow_state_load
    {
        return authenticated_output_with_provenance(
            authenticated,
            replay.sha256,
            None,
            rejection(
                VerificationRejectionCodeV1::CommandNotAllowed,
                "ruleset_state_load_not_allowed",
            ),
        );
    }
    let input_provenance = match replay_input_provenance(&replay_data) {
        Ok(provenance) => provenance,
        Err(()) => {
            return authenticated_output(
                authenticated,
                replay.sha256,
                rejection(
                    VerificationRejectionCodeV1::MalformedReplay,
                    "invalid_input_provenance",
                ),
            );
        }
    };
    if !input_provenance.is_rankable() {
        return authenticated_output_with_provenance(
            authenticated,
            replay.sha256,
            Some(input_provenance),
            rejection(
                VerificationRejectionCodeV1::InputProvenanceIneligible,
                "replay_input_provenance_ineligible",
            ),
        );
    }
    if let Err((code, detail)) =
        validate_replay_header_and_transcript(&replay_data, &campaign_bytes, request)
    {
        return authenticated_output_with_provenance(
            authenticated,
            replay.sha256,
            Some(input_provenance),
            rejection(code, detail),
        );
    }
    match crate::job_config::sim_config_matches(
        &replay_data.header().sim_config,
        &validated_config.config().template.rules_config,
    ) {
        Ok(true) => {}
        Ok(false) => {
            return authenticated_output_with_provenance(
                authenticated,
                replay.sha256,
                Some(input_provenance),
                rejection(
                    VerificationRejectionCodeV1::ConfigMismatch,
                    "replay_sim_config_mismatch",
                ),
            );
        }
        Err(_) => {
            return authenticated_output_with_provenance(
                authenticated,
                replay.sha256,
                Some(input_provenance),
                infrastructure_failure(
                    VerificationInfrastructureFailureCodeV1::WorkerInternalFailure,
                    "sim_config_projection_failed",
                ),
            );
        }
    }

    if let Err(error) = validate_canonical_campaign_start(&validated_config, request) {
        tracing::warn!(%error, "run canonical campaign proposal was rejected");
        return authenticated_output_with_provenance(
            authenticated,
            replay.sha256,
            Some(input_provenance),
            rejection(
                VerificationRejectionCodeV1::ConfigMismatch,
                "canonical_campaign_start_mismatch",
            ),
        );
    }

    let preparation = match prepare_ranked_replay_mission(
        &validated_config,
        request,
        &campaign_bytes,
        &replay_data,
    ) {
        Ok(preparation) => preparation,
        Err(error) => {
            return authenticated_output_with_provenance(
                authenticated,
                replay.sha256,
                Some(input_provenance),
                ranked_loader_failure(&error),
            );
        }
    };
    if let Err(failure) = validate_approved_mission_assets(
        &replay_data.header().mission_assets,
        preparation.approved_mission_assets(),
    ) {
        return authenticated_output_with_provenance(
            authenticated,
            replay.sha256,
            Some(input_provenance),
            rejection(failure.code, failure.detail),
        );
    }
    let ranked = &offer.session_genesis.claim.ranked_session;
    if ranked
        .validate_prepared_inputs_seal(preparation.seal())
        .is_err()
        || preparation.run_projection_sha256().ok()
            != Some(ranked.prepared_inputs_projection_sha256)
    {
        return authenticated_output_with_provenance(
            authenticated,
            replay.sha256,
            Some(input_provenance),
            rejection(
                VerificationRejectionCodeV1::StartingStateMismatch,
                "recomputed_prepared_inputs_seal_mismatch",
            ),
        );
    }
    let starting_campaign_score = preparation.starting_campaign_score();
    let (engine, assets) =
        match consume_approved_preparation(preparation, campaign.sha256, &validated_config) {
            Ok(parts) => parts,
            Err(()) => {
                return authenticated_output_with_provenance(
                    authenticated,
                    replay.sha256,
                    Some(input_provenance),
                    infrastructure_failure(
                        VerificationInfrastructureFailureCodeV1::WorkerInternalFailure,
                        "sealed_campaign_or_content_identity_lost",
                    ),
                );
            }
        };
    let resimulation = match robin_engine::ranked_resim::resimulate_canonical_ranked_replay(
        engine,
        &assets,
        &replay_data,
    ) {
        Ok(result) => result,
        Err(error) => {
            return authenticated_output_with_provenance(
                authenticated,
                replay.sha256,
                Some(input_provenance),
                ranked_resimulation_failure(&error),
            );
        }
    };
    if resimulation.outcome != robin_engine::game_operation::GameCode::LevelSucceeded {
        return authenticated_output_with_provenance(
            authenticated,
            replay.sha256,
            Some(input_provenance),
            rejection(
                VerificationRejectionCodeV1::TerminalInvalid,
                "ruleset_requires_independent_success",
            ),
        );
    }
    let Some(achievement_results) = resimulation.mission_achievement_results else {
        return authenticated_output_with_provenance(
            authenticated,
            replay.sha256,
            Some(input_provenance),
            rejection(
                VerificationRejectionCodeV1::ResultInvariantMismatch,
                "successful_terminal_missing_achievement_results",
            ),
        );
    };
    let achievements = match crate::result_projection::project_authoritative_achievements(
        achievement_results,
        &validated_config.config().template.ruleset_manifest,
    ) {
        Ok(achievements) => achievements,
        Err(_) => {
            return authenticated_output_with_provenance(
                authenticated,
                replay.sha256,
                Some(input_provenance),
                rejection(
                    VerificationRejectionCodeV1::ResultInvariantMismatch,
                    "authoritative_achievement_projection_failed",
                ),
            );
        }
    };
    let final_campaign_score = resimulation
        .final_campaign
        .get_value(robin_engine::campaign::CampaignValue::Score);
    let original_score_delta =
        i64::from(final_campaign_score.wrapping_sub(starting_campaign_score) as u32);
    let claims = offer.participant_claims.clone();
    let named_participant_instance_count = claims
        .iter()
        .filter(|claim| claim.public_disclosure == ParticipantPublicDisclosureV1::NamedProfile)
        .count() as u16;
    let anonymous_participant_instance_count =
        claims.len() as u16 - named_participant_instance_count;
    let transcript = submission.replay_session_transcript.clone();
    let campaign_session = validated_config.config().campaign_session.as_ref();
    let scope_kind = offer.starting_state.scope_kind();
    let final_h12_is_won = resimulation.final_campaign.missions.iter().any(|mission| {
        mission.status == robin_engine::mission::MissionStatus::Won
            && mission.profile(&assets.profile_manager).mission_filename == "H12_Not_MP"
    });
    let campaign_complete_evidence = if scope_kind == RunScopeKindV1::Campaign
        && ranked.content_edition == OfficialContentEditionV1::Full
        && ranked.content_subject
            == (OfficialContentSubjectV1::FieldMission {
                mission_id: "H12_Not_MP".into(),
            })
        && final_h12_is_won
        && resimulation
            .final_campaign
            .get_progression(&assets.profile_manager)
            == 100
    {
        let Some(campaign_content_manifest_sha256) =
            validated_config.campaign_content_manifest_sha256()
        else {
            return authenticated_output_with_provenance(
                authenticated,
                replay.sha256,
                Some(input_provenance),
                infrastructure_failure(
                    VerificationInfrastructureFailureCodeV1::WorkerInternalFailure,
                    "terminal_campaign_catalog_missing",
                ),
            );
        };
        Some(CampaignCompleteEvidenceV1 {
            schema_version: robin_run_protocol::SCHEMA_VERSION_V1,
            campaign_content_manifest_sha256,
            content_manifest_sha256: validated_config.content().manifest_sha256(),
            rules_config_sha256: validated_config.rules_config_sha256(),
            ruleset_manifest_sha256: validated_config.ruleset_manifest_sha256(),
            verification_request_sha256: authenticated.canonical_sha256(),
            replay_sha256: replay.sha256,
            terminal_subject: ranked.content_subject.clone(),
            final_campaign_sha256: resimulation.final_campaign_sha256,
            final_state_sha256: resimulation.final_state_sha256,
            observed_progression_percent: 100,
        })
    } else {
        None
    };
    let final_campaign_bytes = bitcode::encode(&resimulation.final_campaign);
    if final_campaign_bytes.len() > campaign_cap {
        return authenticated_output_with_provenance(
            authenticated,
            replay.sha256,
            Some(input_provenance),
            rejection(
                VerificationRejectionCodeV1::ResourceLimit,
                "campaign_output_exceeds_signed_limit",
            ),
        );
    }
    let starting_campaign = submission.artifacts.starting_campaign.clone();
    let final_campaign = campaign_artifact(&final_campaign_bytes);
    if Digest32::digest_bytes(&final_campaign_bytes) != resimulation.final_campaign_sha256
        || starting_campaign.sha256 != campaign.sha256
        || starting_campaign.validate().is_err()
        || final_campaign.validate().is_err()
    {
        return authenticated_output_with_provenance(
            authenticated,
            replay.sha256,
            Some(input_provenance),
            infrastructure_failure(
                VerificationInfrastructureFailureCodeV1::WorkerInternalFailure,
                "campaign_artifact_projection_failed",
            ),
        );
    }
    let verified = VerifiedRunV1 {
        scope_kind,
        campaign_aggregation_consent: submission.campaign_aggregation_consent,
        campaign_session_kind: campaign_session.map(|binding| binding.kind.clone()),
        campaign_session_ordinal: campaign_session.map(|binding| binding.ordinal),
        max_concurrent_players: transcript.max_concurrent_players,
        participant_instance_count: transcript.participant_instance_count,
        named_participant_instance_count,
        anonymous_participant_instance_count,
        authenticated_participant_claims: claims,
        replay_session_transcript: transcript,
        outcome: TerminalOutcomeV1::Won,
        starting_campaign,
        final_campaign,
        starting_campaign_score,
        final_campaign_score,
        final_state_sha256: resimulation.final_state_sha256,
        replay_frames: resimulation.replay_frames,
        original_score_delta,
        active_simulation_ticks: resimulation.active_simulation_ticks,
        ransom_collected: u64::from(resimulation.mission_stat.collected_money),
        campaign_complete_evidence,
        achievements,
        diagnostics: std::collections::BTreeMap::new(),
    };
    if verified.validate().is_err() {
        return authenticated_output_with_provenance(
            authenticated,
            replay.sha256,
            Some(input_provenance),
            rejection(
                VerificationRejectionCodeV1::ResultInvariantMismatch,
                "verified_result_invariant_mismatch",
            ),
        );
    }
    let result = authenticated_result_with_provenance(
        &authenticated,
        replay.sha256,
        Some(input_provenance.clone()),
        VerificationStatusV1::Verified(verified),
    );
    if result
        .validate_campaign_complete_evidence(
            request,
            &validated_config.config().template.ruleset_manifest,
            validated_config
                .config()
                .template
                .campaign_content_manifest
                .as_ref(),
        )
        .is_err()
    {
        return authenticated_output_with_provenance(
            authenticated,
            replay.sha256,
            Some(input_provenance),
            infrastructure_failure(
                VerificationInfrastructureFailureCodeV1::WorkerInternalFailure,
                "campaign_completion_evidence_invalid",
            ),
        );
    }
    let output = VerifierWorkerOutputV1::VerificationResult {
        schema_version: robin_run_protocol::SCHEMA_VERSION_V1,
        result,
    };
    let output_bytes = match output.canonical_bytes() {
        Ok(bytes) if bytes.len() <= MAX_WORKER_RESULT_BYTES => bytes,
        _ => {
            return verifier_output_after_campaign_write_failure(
                paths,
                output,
                "verified_result_preflight_failed",
                fatal_campaign_rollback,
            );
        }
    };
    if output_bytes.is_empty() {
        unreachable!("canonical verified result is never empty")
    }
    if let Err(failure) =
        write_verified_campaign_outputs(paths, &final_campaign_bytes, campaign_cap)
    {
        if let Some(error) = failure.rollback_error {
            *fatal_campaign_rollback = Some(error);
        }
        return verifier_output_after_campaign_write_failure(
            paths,
            output,
            failure.detail,
            fatal_campaign_rollback,
        );
    }
    output
}

fn verifier_output_after_campaign_write_failure(
    paths: &WorkerPaths,
    mut output: VerifierWorkerOutputV1,
    detail: &'static str,
    fatal_campaign_rollback: &mut Option<AtomicOutputError>,
) -> VerifierWorkerOutputV1 {
    *fatal_campaign_rollback = truncate_campaign_outputs(paths).err();
    let VerifierWorkerOutputV1::VerificationResult { result, .. } = &mut output else {
        unreachable!("verified output must be an authenticated result")
    };
    result.status = infrastructure_failure(
        VerificationInfrastructureFailureCodeV1::ArtifactIoFailure,
        detail,
    );
    assert!(result.validate().is_ok());
    output
}

fn validate_canonical_campaign_start(
    config: &ValidatedJobConfig,
    request: &VerificationRequestV1,
) -> anyhow::Result<()> {
    use robin_run_protocol::{
        InitialStateExpectationV1, RulesConfigConstraintV1, SimulationContentComponentKindV1,
    };
    let template = &config.config().template;
    let custom = template.ruleset_manifest.rules_config_constraint
        == RulesConfigConstraintV1::AnyCanonicalSimConfig;
    let mission_setup = !template
        .ruleset_manifest
        .canonical_start_policy
        .requires_exact_operator_artifact();
    if !custom && !mission_setup {
        return Ok(());
    }
    let documents = config.content().ordered_documents();
    let profiles_document = documents
        .iter()
        .find(|document| document.kind == SimulationContentComponentKindV1::Profiles)
        .ok_or_else(|| anyhow::anyhow!("verified content has no Profiles component"))?;
    if custom {
        let actual = robin_engine::simulation_inputs::canonical_fresh_campaign_artifact_v1(
            &template.rules_config,
            profiles_document,
        )?;
        anyhow::ensure!(
            actual == template.canonical_campaign_state.artifact,
            "custom genesis differs from independently reconstructed fresh campaign"
        );
    }
    let submitted = &request.submission.submission;
    let ranked = &submitted.offer.session_genesis.claim.ranked_session;
    if mission_setup
        && matches!(
            submitted.offer.starting_state,
            InitialStateExpectationV1::IndividualLevel { .. }
                | InitialStateExpectationV1::CampaignGenesis { .. }
        )
    {
        let files = robin_engine::sbfile::SbFileSystem::new(std::sync::Arc::new(
            robin_util::asset_fs::AssetVfs::new(),
        ));
        let status = files.lock_ranked_verifier_primary_path_with_locale(
            config.raw_content().root(),
            template.content_manifest.resource_locale_root.as_str(),
        );
        anyhow::ensure!(
            status == robin_engine::sbfile::SBFILE_NO_ERROR,
            "cannot confine mission setup data resolver: {status}"
        );
        robin_engine::simulation_inputs::validate_canonical_mission_start_v1(
            &template.rules_config,
            profiles_document,
            ranked.content_edition,
            &ranked.content_subject,
            ranked.simulation_seed.get(),
            &submitted.artifacts.starting_campaign,
            &files,
        )?;
    }
    Ok(())
}

fn prepare_ranked_replay_mission(
    validated_config: &ValidatedJobConfig,
    request: &VerificationRequestV1,
    starting_campaign_bytes: &[u8],
    replay: &robin_engine::replay::ReplayData,
) -> Result<
    robin_ranked_verification::ranked_verifier::PreparedRankedReplayMission,
    robin_ranked_verification::ranked_verifier::RankedVerifierLoadError,
> {
    let ranked = &request
        .submission
        .submission
        .offer
        .session_genesis
        .claim
        .ranked_session;
    robin_ranked_verification::ranked_verifier::prepare_ranked_replay_mission(
        validated_config.raw_content().root(),
        starting_campaign_bytes,
        &replay.header().mission_id,
        &replay_admission_limits(request),
        validated_config.build_manifest_sha256(),
        validated_config.content().manifest_sha256(),
        validated_config.content().manifest(),
        &validated_config.content().ordered_documents(),
        &validated_config.config().template.rules_config,
        &ranked.speech_timing,
        replay.header().rng_seed,
        replay.header().sim_config,
    )
}

fn consume_approved_preparation(
    preparation: robin_ranked_verification::ranked_verifier::PreparedRankedReplayMission,
    expected_campaign_sha256: Digest32,
    validated_config: &ValidatedJobConfig,
) -> Result<
    (
        robin_engine::engine::Engine,
        robin_engine::engine::LevelAssets,
    ),
    (),
> {
    let (approved_engine, assets) = preparation.into_engine_and_assets();
    let (engine, _, _, approved_campaign_sha256, approved_identity) = approved_engine.into_parts();
    if approved_campaign_sha256 != *expected_campaign_sha256.as_bytes()
        || approved_identity.build_manifest_sha256
            != *validated_config.build_manifest_sha256().as_bytes()
        || approved_identity.content_manifest_sha256
            != *validated_config.content().manifest_sha256().as_bytes()
    {
        return Err(());
    }
    Ok((engine, assets))
}

fn ranked_loader_failure(
    error: &robin_ranked_verification::ranked_verifier::RankedVerifierLoadError,
) -> VerificationStatusV1 {
    use robin_ranked_verification::ranked_verifier::RankedVerifierLoadError;
    match error {
        RankedVerifierLoadError::Campaign(_) | RankedVerifierLoadError::CampaignContent(_) => {
            rejection(
                VerificationRejectionCodeV1::StartingStateMismatch,
                "starting_campaign_validation_failed",
            )
        }
        RankedVerifierLoadError::Engine(_)
        | RankedVerifierLoadError::SherwoodReferenceEngine(_) => infrastructure_failure(
            VerificationInfrastructureFailureCodeV1::WorkerInternalFailure,
            "ranked_engine_preparation_failed",
        ),
        _ => infrastructure_failure(
            VerificationInfrastructureFailureCodeV1::ArtifactIoFailure,
            "official_raw_content_load_failed",
        ),
    }
}

fn ranked_resimulation_failure(
    error: &robin_engine::ranked_resim::RankedResimulationError,
) -> VerificationStatusV1 {
    use robin_engine::ranked_resim::RankedResimulationError;
    match error {
        RankedResimulationError::MissingPeriodicHash { .. }
        | RankedResimulationError::StateHashMismatch { .. } => rejection(
            VerificationRejectionCodeV1::StateHashMismatch,
            "periodic_state_hash_mismatch",
        ),
        RankedResimulationError::Admission { .. }
        | RankedResimulationError::TerminalCommandShape { .. } => rejection(
            VerificationRejectionCodeV1::CommandNotAllowed,
            "ranked_replay_admission_failed",
        ),
        RankedResimulationError::FrameAdvance { .. } => rejection(
            VerificationRejectionCodeV1::TimelineInvalid,
            "deterministic_frame_advance_failed",
        ),
        RankedResimulationError::TerminalBeforeEof { .. }
        | RankedResimulationError::EofBeforeTerminal
        | RankedResimulationError::UnsupportedTerminal { .. }
        | RankedResimulationError::ConflictingTerminalOutcomes { .. }
        | RankedResimulationError::TerminalCommandOutcomeMismatch { .. } => rejection(
            VerificationRejectionCodeV1::TerminalInvalid,
            "terminal_boundary_invalid",
        ),
    }
}

#[derive(Debug)]
struct ReplayRejection {
    code: VerificationRejectionCodeV1,
    detail: &'static str,
}

fn campaign_artifact(bytes: &[u8]) -> ArtifactRefV1 {
    ArtifactRefV1 {
        sha256: Digest32::digest_bytes(bytes),
        byte_length: u64::try_from(bytes.len())
            .expect("campaign byte length is representable in the protocol"),
        media_type: RANKED_CAMPAIGN_MEDIA_TYPE_V1.into(),
    }
}

fn canonical_replay_artifact_bytes(
    replay: &robin_engine::replay::ReplayData,
) -> Result<Vec<u8>, String> {
    replay.validate_ranked_hash_coverage()?;
    robin_replay_format::encode_compact(replay, robin_replay_format::ENGINE_VERSION_HASH)
        .map(String::into_bytes)
        .map_err(|error| error.to_string())
}

fn validate_canonical_replay_shape(
    replay: &robin_engine::replay::ReplayData,
) -> Result<(), String> {
    replay.validate_canonical_ranked_command_admission()
}

fn decode_replay(
    bytes: &[u8],
    request: &VerificationRequestV1,
    artifact: &robin_run_protocol::ReplayArtifactV1,
) -> Result<robin_engine::replay::ReplayData, ReplayRejection> {
    let limits = replay_admission_limits(request);
    decode_replay_bounded(bytes, artifact, &limits)
}

fn decode_replay_bounded(
    bytes: &[u8],
    artifact: &robin_run_protocol::ReplayArtifactV1,
    limits: &robin_replay_format::ReplayAdmissionLimits,
) -> Result<robin_engine::replay::ReplayData, ReplayRejection> {
    let text = std::str::from_utf8(bytes).map_err(|_| ReplayRejection {
        code: VerificationRejectionCodeV1::MalformedReplay,
        detail: "compact_replay_not_utf8",
    })?;
    let (_engine_hash, replay) = robin_replay_format::decode_compact_for_build(
        text,
        limits,
        robin_replay_format::ENGINE_VERSION_HASH,
    )
    .map_err(compact_replay_rejection)?;
    // Ranked boards currently admit only shipping SCB content. Custom mission
    // archives and embedded Spellforge executables remain valid for the
    // contained local-playback lane, but are an explicit content-policy
    // rejection here. Keep this immediately after the sole hostile typed
    // decode boundary so no later verifier setup can mount or execute them.
    if replay.header().spellforge_package.is_some() {
        return Err(ReplayRejection {
            code: VerificationRejectionCodeV1::ContentNotAllowed,
            detail: "ranked_spellforge_package_not_allowed",
        });
    }
    if matches!(
        &replay.header().mission_assets.source,
        robin_engine::mission_assets::MissionAssetSource::Archive(_)
    ) {
        return Err(ReplayRejection {
            code: VerificationRejectionCodeV1::ContentNotAllowed,
            detail: "ranked_archive_mission_not_allowed",
        });
    }
    let signed_schema = artifact.replay_schema_version;
    if replay.header().version != signed_schema {
        return Err(ReplayRejection {
            code: VerificationRejectionCodeV1::UnsupportedSchema,
            detail: "replay_header_schema_mismatch",
        });
    }
    Ok(replay)
}

fn validate_approved_mission_assets(
    replay: &robin_engine::mission_assets::MissionAssetDescriptor,
    approved: &robin_engine::mission_assets::MissionAssetDescriptor,
) -> Result<(), ReplayRejection> {
    // The ranked loader can only construct this value through `built_in`, but
    // keep the worker boundary fail-closed if that invariant is ever changed.
    if !matches!(
        &approved.source,
        robin_engine::mission_assets::MissionAssetSource::BuiltIn
    ) || replay != approved
    {
        return Err(ReplayRejection {
            code: VerificationRejectionCodeV1::ContentNotAllowed,
            detail: "replay_official_mission_assets_mismatch",
        });
    }
    Ok(())
}

fn compact_replay_rejection(error: robin_replay_format::FormatError) -> ReplayRejection {
    use robin_replay_format::FormatError;
    match error {
        FormatError::LimitExceeded { .. } | FormatError::CountOverflow { .. } => ReplayRejection {
            code: VerificationRejectionCodeV1::ResourceLimit,
            detail: "compact_replay_resource_limit",
        },
        FormatError::UnsupportedVersion { .. } => ReplayRejection {
            code: VerificationRejectionCodeV1::UnsupportedSchema,
            detail: "compact_replay_unsupported_schema",
        },
        FormatError::EngineVersionMismatch { .. } => ReplayRejection {
            code: VerificationRejectionCodeV1::BuildNotAllowed,
            detail: "compact_engine_version_mismatch",
        },
        _ => ReplayRejection {
            code: VerificationRejectionCodeV1::MalformedReplay,
            detail: "compact_replay_invalid",
        },
    }
}

fn replay_admission_limits(
    request: &VerificationRequestV1,
) -> robin_replay_format::ReplayAdmissionLimits {
    let signed = &request.limits;
    let compiled = robin_replay_format::DEFAULT_REPLAY_ADMISSION_LIMITS;
    let mut limits = compiled;
    limits.max_input_bytes = usize::try_from(signed.max_input_bytes)
        .unwrap_or(usize::MAX)
        .min(compiled.max_input_bytes);
    limits.max_base64_payload_bytes = usize::try_from(signed.max_base64_payload_bytes)
        .unwrap_or(usize::MAX)
        .min(compiled.max_base64_payload_bytes);
    limits.max_compressed_bytes = usize::try_from(signed.max_compressed_bytes)
        .unwrap_or(usize::MAX)
        .min(compiled.max_compressed_bytes);
    limits.max_decompressed_bytes = usize::try_from(signed.max_decompressed_bytes)
        .unwrap_or(usize::MAX)
        .min(compiled.max_decompressed_bytes);
    limits.max_version_hash_bytes =
        (signed.max_version_bytes as usize).min(compiled.max_version_hash_bytes);
    limits.max_mission_id_bytes =
        (signed.max_mission_id_bytes as usize).min(compiled.max_mission_id_bytes);
    limits.max_campaign_bytes = usize::try_from(signed.max_campaign_bytes)
        .unwrap_or(usize::MAX)
        .min(MAX_WORKER_CAMPAIGN_BYTES)
        .min(compiled.max_campaign_bytes);
    limits.max_frames = (signed.max_frames as usize).min(compiled.max_frames);
    limits.max_metadata_records =
        (signed.max_metadata_records as usize).min(compiled.max_metadata_records);
    limits.max_entries_per_frame =
        (signed.max_entries_per_frame as usize).min(compiled.max_entries_per_frame);
    limits
}

fn replay_input_provenance(
    replay: &robin_engine::replay::ReplayData,
) -> Result<InputProvenanceStatusV1, ()> {
    use robin_engine::replay_rankability::{InputTaintKind, ReplayRankability};
    Ok(match replay.rankability().map_err(|_| ())? {
        ReplayRankability::Recorded { taints } if taints.is_empty() => {
            InputProvenanceStatusV1::Rankable
        }
        ReplayRankability::Recorded { taints } => InputProvenanceStatusV1::Tainted {
            taints: taints
                .into_iter()
                .map(|taint| InputTaintV1 {
                    kind: match taint.kind {
                        InputTaintKind::HttpPlayerCommand => InputTaintKindV1::HttpPlayerCommand,
                        InputTaintKind::HttpSimulationStep => InputTaintKindV1::HttpSimulationStep,
                        InputTaintKind::HttpStateMutation => InputTaintKindV1::HttpStateMutation,
                        InputTaintKind::ConsoleCommand => InputTaintKindV1::ConsoleCommand,
                        InputTaintKind::CheatCommand => InputTaintKindV1::CheatCommand,
                        InputTaintKind::HeadlessAutomation => InputTaintKindV1::HeadlessAutomation,
                        InputTaintKind::ReplayPlayback => InputTaintKindV1::ReplayPlayback,
                        InputTaintKind::StateLoad => InputTaintKindV1::StateLoad,
                        InputTaintKind::MissionRestart => InputTaintKindV1::MissionRestart,
                        InputTaintKind::DebugInputInjection => {
                            InputTaintKindV1::DebugInputInjection
                        }
                    },
                    first_frame: taint.first_frame,
                })
                .collect(),
        },
    })
}

fn validate_replay_header_and_transcript(
    replay: &robin_engine::replay::ReplayData,
    starting_campaign: &[u8],
    request: &VerificationRequestV1,
) -> Result<(), (VerificationRejectionCodeV1, &'static str)> {
    validate_replay_header(replay, starting_campaign, request)?;
    let submission = &request.submission.submission;
    let offer = &submission.offer;
    let genesis = &offer.session_genesis.claim;
    let transcript = &submission.replay_session_transcript;
    validate_canonical_replay_shape(replay).map_err(|_| {
        (
            VerificationRejectionCodeV1::MalformedReplay,
            "canonical_replay_shape_invalid",
        )
    })?;
    replay
        .validate_ranked_command_admission(transcript)
        .map_err(|_| {
            (
                VerificationRejectionCodeV1::CommandNotAllowed,
                "ranked_transcript_replay_lifecycle_mismatch",
            )
        })?;
    let session_genesis_sha256 = offer.session_genesis.canonical_digest().map_err(|_| {
        (
            VerificationRejectionCodeV1::ConfigMismatch,
            "session_genesis_digest_unavailable",
        )
    })?;
    if transcript.session_genesis_sha256 != session_genesis_sha256
        || transcript.replay_session_id != genesis.replay_session_id
        || transcript.host_participant_instance_id != genesis.host_participant_instance_id
        || transcript.max_concurrent_players != offer.max_concurrent_players
        || transcript.participant_instance_count != offer.participant_instance_count
    {
        return Err((
            VerificationRejectionCodeV1::CommandNotAllowed,
            "ranked_transcript_genesis_mismatch",
        ));
    }
    let mut transcript_participants = transcript
        .events
        .iter()
        .filter_map(|event| {
            matches!(
                event.lifecycle,
                robin_run_protocol::ReplaySeatLifecycleKindV1::Connected { .. }
            )
            .then_some((event.seat, event.participant_instance_id))
        })
        .collect::<Vec<_>>();
    transcript_participants.sort_unstable();
    transcript_participants.dedup();
    let claims = offer
        .participant_claims
        .iter()
        .map(|claim| (claim.seat, claim.participant_instance_id))
        .collect::<Vec<_>>();
    if transcript_participants != claims {
        return Err((
            VerificationRejectionCodeV1::CommandNotAllowed,
            "ranked_transcript_participant_mismatch",
        ));
    }
    Ok(())
}

fn validate_replay_header(
    replay: &robin_engine::replay::ReplayData,
    starting_campaign: &[u8],
    request: &VerificationRequestV1,
) -> Result<(), (VerificationRejectionCodeV1, &'static str)> {
    let offer = &request.submission.submission.offer;
    let ranked = &offer.session_genesis.claim.ranked_session;
    if replay.header().mission_id != offer.mission_id {
        return Err((
            VerificationRejectionCodeV1::StartingStateMismatch,
            "replay_mission_mismatch",
        ));
    }
    if replay.header().rng_seed != ranked.simulation_seed.get() {
        return Err((
            VerificationRejectionCodeV1::StartingStateMismatch,
            "replay_simulation_seed_mismatch",
        ));
    }
    if replay.header().campaign.as_slice() != starting_campaign {
        return Err((
            VerificationRejectionCodeV1::StartingStateMismatch,
            "replay_starting_campaign_mismatch",
        ));
    }
    Ok(())
}

fn authenticated_output(
    authenticated: AuthenticatedVerificationRequest,
    replay_sha256: Digest32,
    status: VerificationStatusV1,
) -> VerifierWorkerOutputV1 {
    authenticated_output_with_provenance(authenticated, replay_sha256, None, status)
}

fn authenticated_output_with_provenance(
    authenticated: AuthenticatedVerificationRequest,
    replay_sha256: Digest32,
    input_provenance: Option<InputProvenanceStatusV1>,
    status: VerificationStatusV1,
) -> VerifierWorkerOutputV1 {
    let result = authenticated_result_with_provenance(
        &authenticated,
        replay_sha256,
        input_provenance,
        status,
    );

    VerifierWorkerOutputV1::VerificationResult {
        schema_version: robin_run_protocol::SCHEMA_VERSION_V1,
        result,
    }
}

fn authenticated_result_with_provenance(
    authenticated: &AuthenticatedVerificationRequest,
    observed_replay_sha256: Digest32,
    input_provenance: Option<InputProvenanceStatusV1>,
    status: VerificationStatusV1,
) -> VerificationResultV1 {
    let request = authenticated.request();
    let offer = &request.submission.submission.offer;
    assert!(
        !matches!(&status, VerificationStatusV1::Verified(_))
            || observed_replay_sha256
                == request
                    .submission
                    .submission
                    .artifacts
                    .replay
                    .artifact
                    .sha256,
        "a verified result must bind the exact observed canonical replay"
    );

    let result = VerificationResultV1 {
        schema_version: robin_run_protocol::SCHEMA_VERSION_V1,
        request_id: request.request_id.clone(),
        verification_request_sha256: authenticated.canonical_sha256(),
        artifacts: request.submission.submission.artifacts.clone(),
        session_genesis_sha256: authenticated.session_genesis_sha256(),
        build_manifest_sha256: offer.build_manifest_sha256,
        content_manifest_sha256: offer.content_manifest_sha256,
        rules_config_sha256: offer.rules_config_sha256,
        ruleset_manifest_sha256: offer.ruleset_manifest_sha256,
        competition_manifest_sha256: offer.competition_manifest_sha256,
        input_provenance,
        status,
    };
    assert!(result.validate().is_ok());
    result
}

fn rejection(code: VerificationRejectionCodeV1, detail: &'static str) -> VerificationStatusV1 {
    VerificationStatusV1::Rejected(VerificationRejectionV1 {
        code,
        detail_code: Some(detail.into()),
    })
}

fn infrastructure_failure(
    code: VerificationInfrastructureFailureCodeV1,
    detail: &'static str,
) -> VerificationStatusV1 {
    VerificationStatusV1::FailedInfrastructure(VerificationInfrastructureFailureV1 {
        code,
        private_detail_code: Some(detail.into()),
    })
}

fn job_config_failure(error: &JobConfigError) -> VerificationStatusV1 {
    match error {
        JobConfigError::IdentityMismatch { document, .. } => match *document {
            "build_manifest" => rejection(
                VerificationRejectionCodeV1::BuildNotAllowed,
                "build_manifest_identity_mismatch",
            ),
            "content_manifest" | "campaign_content_manifest" => rejection(
                VerificationRejectionCodeV1::ContentNotAllowed,
                "content_manifest_identity_mismatch",
            ),
            "prepared_mission_inputs" | "prepared_mission_inputs_seal" => rejection(
                VerificationRejectionCodeV1::StartingStateMismatch,
                "prepared_inputs_seal_identity_mismatch",
            ),
            _ => rejection(
                VerificationRejectionCodeV1::ConfigMismatch,
                "operator_catalog_identity_mismatch",
            ),
        },
        JobConfigError::InvalidDocument { document, .. }
            if *document == "content_manifest/ranked_session" =>
        {
            rejection(
                VerificationRejectionCodeV1::ContentNotAllowed,
                "content_manifest_session_mismatch",
            )
        }
        JobConfigError::InvalidDocument { document, .. }
            if *document == "prepared_mission_inputs_seal/ranked_session" =>
        {
            rejection(
                VerificationRejectionCodeV1::StartingStateMismatch,
                "prepared_inputs_seal_session_mismatch",
            )
        }
        JobConfigError::RulesetMismatch(detail) => match *detail {
            "build_not_allowed" => rejection(
                VerificationRejectionCodeV1::BuildNotAllowed,
                "ruleset_build_not_allowed",
            ),
            "content_not_allowed" | "campaign_content_not_allowed" => rejection(
                VerificationRejectionCodeV1::ContentNotAllowed,
                "ruleset_content_not_allowed",
            ),
            _ => rejection(
                VerificationRejectionCodeV1::ConfigMismatch,
                "ruleset_tuple_mismatch",
            ),
        },
        JobConfigError::CompetitionMismatch(_) => rejection(
            VerificationRejectionCodeV1::ConfigMismatch,
            "competition_tuple_mismatch",
        ),
        JobConfigError::CampaignContentMismatch(_) => rejection(
            VerificationRejectionCodeV1::ContentNotAllowed,
            "campaign_content_tuple_mismatch",
        ),
        JobConfigError::RawContentEditionMismatch { .. } => rejection(
            VerificationRejectionCodeV1::ContentNotAllowed,
            "raw_content_edition_mismatch",
        ),
        JobConfigError::CampaignSessionMismatch(_) => rejection(
            VerificationRejectionCodeV1::StartingStateMismatch,
            "campaign_session_tuple_mismatch",
        ),
        JobConfigError::Read(_)
        | JobConfigError::Content(_)
        | JobConfigError::ContentRootIo(_)
        | JobConfigError::RawContentRootNotReadOnly
        | JobConfigError::WritableRawContentEntry(_)
        | JobConfigError::RawContentWalk { .. } => infrastructure_failure(
            VerificationInfrastructureFailureCodeV1::ArtifactIoFailure,
            "operator_catalog_io_or_mount",
        ),
        JobConfigError::VerifierArtifactMismatch => infrastructure_failure(
            VerificationInfrastructureFailureCodeV1::WorkerInternalFailure,
            "verifier_artifact_mismatch",
        ),
        JobConfigError::TooLarge { .. }
        | JobConfigError::Decode(_)
        | JobConfigError::UnsupportedSchema(_)
        | JobConfigError::UnsafeContentRoot(_)
        | JobConfigError::UnsafeContentRootComponent(_)
        | JobConfigError::UnsafeRawContentEntry(_)
        | JobConfigError::OverlappingContentRoots
        | JobConfigError::InvalidDocument { .. } => infrastructure_failure(
            VerificationInfrastructureFailureCodeV1::WorkerInternalFailure,
            "job_config_invalid",
        ),
    }
}

fn stream_request(path: &Path, maximum_retained: usize) -> io::Result<BoundedArtifact> {
    let mut input = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut byte_length = 0_u64;
    let mut retained = Some(Vec::with_capacity(
        input.metadata()?.len().min(maximum_retained as u64) as usize,
    ));
    let mut buffer = [0_u8; STREAM_BUFFER_BYTES];

    loop {
        let read = input.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        byte_length = byte_length
            .checked_add(read as u64)
            .ok_or_else(|| io::Error::other("request byte length overflow"))?;
        hasher.update(&buffer[..read]);

        if let Some(bytes) = &mut retained {
            if bytes.len().saturating_add(read) <= maximum_retained {
                bytes.extend_from_slice(&buffer[..read]);
            } else {
                retained = None;
            }
        }
    }

    Ok(BoundedArtifact {
        sha256: Digest32::from_bytes(hasher.finalize().into()),
        byte_length,
        retained_bytes: retained,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn single_frame_replay_with_commands(command_count: usize) -> robin_engine::replay::ReplayData {
        use robin_engine::engine::SimulationFrameInput;
        use robin_engine::player_command::{PlayerCommand, PlayerInput};
        use robin_engine::replay::{REPLAY_SCHEMA_VERSION, ReplayFile, ReplayFrame, ReplayHeader};

        ReplayFile {
            header: ReplayHeader {
                mission_id: "worker-resource-fixture".into(),
                mission_assets: robin_engine::mission_assets::MissionAssetDescriptor::built_in(
                    "worker-resource-fixture",
                    "worker-resource-fixture",
                    "worker-resource-fixture",
                )
                .unwrap(),
                rng_seed: 7,
                sim_config: robin_engine::engine::SimConfig::default(),
                spellforge_package: None,
                version: REPLAY_SCHEMA_VERSION,
                total_frames: 1,
                rankability: robin_engine::replay_rankability::ReplayRankability::rankable(),
                campaign: bitcode::encode(&robin_engine::campaign::Campaign::default()),
            },
            frames: [(
                0,
                ReplayFrame {
                    timeline_before: 0,
                    timeline_after: 1,
                    input: SimulationFrameInput::from_player_inputs(
                        std::iter::repeat_with(|| PlayerInput::host(PlayerCommand::CrouchDown))
                            .take(command_count)
                            .collect(),
                    ),
                    host_controls: Vec::new(),
                },
            )]
            .into(),
            hashes: [(0, 0)].into(),
            save_markers: Default::default(),
            load_backs: Default::default(),
        }
        .try_into()
        .expect("valid replay fixture")
    }

    fn replay_artifact(bytes: &[u8]) -> robin_run_protocol::ReplayArtifactV1 {
        robin_run_protocol::ReplayArtifactV1 {
            artifact: robin_run_protocol::ArtifactRefV1 {
                sha256: Digest32::digest_bytes(bytes),
                byte_length: bytes.len() as u64,
                media_type: robin_run_protocol::RANKED_REPLAY_MEDIA_TYPE_V1.into(),
            },
            replay_schema_version: robin_engine::replay::REPLAY_SCHEMA_VERSION,
        }
    }

    fn archive_mission_assets() -> robin_engine::mission_assets::MissionAssetDescriptor {
        robin_engine::mission_assets::MissionAssetDescriptor::archive(
            "worker-resource-fixture",
            "worker-resource-fixture",
            "worker-resource-fixture",
            robin_engine::mission_assets::ArchiveMissionAssets {
                mission_archive: robin_engine::mission_assets::ArchiveIdentity {
                    sha256: [1; 32],
                    bytes: 1,
                },
                selected_rhm_entry: "missions/worker-resource-fixture.rhm".into(),
                shared_archive: None,
                installed: Some(robin_engine::mission_assets::InstalledArchiveLocator {
                    root: robin_engine::mission_assets::InstalledModsRoot::ConfiguredMods,
                    mission_relative_path: "worker-resource-fixture/v1.zip".into(),
                    shared_relative_path: None,
                }),
                distributed_cache: None,
            },
        )
        .unwrap()
    }

    fn spellforge_package() -> robin_engine::spellforge::SpellforgePackage {
        let entrypoint = "worker-resource-fixture.lua".to_owned();
        let mut package = robin_engine::spellforge::SpellforgePackage {
            contract_version: robin_engine::spellforge::SPELLFORGE_CONTRACT_VERSION,
            vm_abi: format!(
                "{}{}",
                robin_engine::spellforge::SPELLFORGE_VM_ABI_SCHEME,
                "0".repeat(64),
            ),
            script_mode: robin_engine::spellforge::SpellforgeScriptMode::Replace,
            files: [(entrypoint.clone(), b"return true".to_vec())]
                .into_iter()
                .collect(),
            entrypoint,
            sha256: [0; 32],
        };
        package.sha256 = package.computed_sha256();
        package.validate_wire().unwrap();
        package
    }

    fn output_paths(directory: &tempfile::TempDir) -> WorkerPaths {
        let path = |name: &str| directory.path().join(name);
        for name in [
            "request",
            "replay",
            "config",
            "starting-campaign",
            "final-campaign",
            "result",
        ] {
            std::fs::write(path(name), b"").unwrap();
        }
        WorkerPaths {
            request: path("request"),
            replay: path("replay"),
            config: path("config"),
            starting_campaign: path("starting-campaign"),
            final_campaign: path("final-campaign"),
            result: path("result"),
        }
    }

    fn seed_campaign_outputs(paths: &WorkerPaths) {
        std::fs::write(&paths.final_campaign, b"stale-or-partial").unwrap();
    }

    #[test]
    fn bounded_request_reader_hashes_all_bytes_without_retaining_oversize_input() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("request");
        let bytes = vec![0x5a; 129];
        std::fs::write(&path, &bytes).unwrap();

        let artifact = stream_request(&path, 128).unwrap();
        assert_eq!(artifact.byte_length, 129);
        assert_eq!(artifact.sha256, Digest32::digest_bytes(&bytes));
        assert!(artifact.retained_bytes.is_none());
    }

    #[test]
    fn worker_rejects_compact_single_frame_command_amplification() {
        const COMMANDS: usize = 4_096;
        const LIMIT: usize = 64;

        let replay = single_frame_replay_with_commands(COMMANDS);
        let compact =
            robin_replay_format::encode_compact(&replay, robin_replay_format::ENGINE_VERSION_HASH)
                .unwrap()
                .into_bytes();
        let limits = robin_replay_format::ReplayAdmissionLimits {
            max_input_bytes: compact.len(),
            max_entries_per_frame: LIMIT,
            ..robin_replay_format::ReplayAdmissionLimits::default()
        };

        let failure = decode_replay_bounded(&compact, &replay_artifact(&compact), &limits)
            .expect_err("compact replay must enforce the signed entry ceiling");
        assert_eq!(failure.code, VerificationRejectionCodeV1::ResourceLimit);
    }

    #[test]
    fn selected_build_is_rejected_before_base64_or_zstd_decode() {
        let wrong_hash = if robin_replay_format::ENGINE_VERSION_HASH == "000000000000" {
            "111111111111"
        } else {
            "000000000000"
        };
        // `AAAA` is valid canonical base64url text but is neither a complete
        // zstd frame nor a replay. A build-first decoder must still classify
        // this solely as the typed build-policy rejection.
        let compact = format!("rhrec-{wrong_hash}-AAAA").into_bytes();
        let failure = decode_replay_bounded(
            &compact,
            &replay_artifact(&compact),
            &robin_replay_format::ReplayAdmissionLimits {
                max_input_bytes: compact.len(),
                ..robin_replay_format::ReplayAdmissionLimits::default()
            },
        )
        .expect_err("the selected verifier build must reject another build envelope");
        assert_eq!(failure.code, VerificationRejectionCodeV1::BuildNotAllowed);
        assert_eq!(failure.detail, "compact_engine_version_mismatch");
    }

    #[test]
    fn official_mission_descriptor_is_bound_field_for_field_to_loaded_assets() {
        let approved = robin_engine::mission_assets::MissionAssetDescriptor::built_in(
            "worker-resource-fixture",
            "worker-proto-fixture",
            "worker-map-fixture",
        )
        .unwrap();
        assert!(validate_approved_mission_assets(&approved, &approved).is_ok());

        for mismatched in [
            robin_engine::mission_assets::MissionAssetDescriptor::built_in(
                "other-mission",
                &approved.proto_level_filename,
                &approved.map_filename,
            )
            .unwrap(),
            robin_engine::mission_assets::MissionAssetDescriptor::built_in(
                &approved.mission_basename,
                "other-proto",
                &approved.map_filename,
            )
            .unwrap(),
            robin_engine::mission_assets::MissionAssetDescriptor::built_in(
                &approved.mission_basename,
                &approved.proto_level_filename,
                "other-map",
            )
            .unwrap(),
        ] {
            let failure = validate_approved_mission_assets(&mismatched, &approved)
                .expect_err("every loaded official asset identity must match exactly");
            assert_eq!(failure.code, VerificationRejectionCodeV1::ContentNotAllowed);
            assert_eq!(failure.detail, "replay_official_mission_assets_mismatch");
        }
    }

    #[test]
    fn contained_decode_rejects_custom_mission_content_as_unrankable() {
        let mut archive = single_frame_replay_with_commands(1);
        archive
            .try_edit_header(|header| header.mission_assets = archive_mission_assets())
            .unwrap();
        let compact =
            robin_replay_format::encode_compact(&archive, robin_replay_format::ENGINE_VERSION_HASH)
                .unwrap()
                .into_bytes();
        let failure = decode_replay_bounded(
            &compact,
            &replay_artifact(&compact),
            &robin_replay_format::ReplayAdmissionLimits::default(),
        )
        .expect_err("ranked verification must reject archive mission assets");
        assert_eq!(failure.code, VerificationRejectionCodeV1::ContentNotAllowed);
        assert_eq!(failure.detail, "ranked_archive_mission_not_allowed");

        archive
            .try_edit_header(|header| header.spellforge_package = Some(spellforge_package()))
            .unwrap();
        let compact =
            robin_replay_format::encode_compact(&archive, robin_replay_format::ENGINE_VERSION_HASH)
                .unwrap()
                .into_bytes();
        let failure = decode_replay_bounded(
            &compact,
            &replay_artifact(&compact),
            &robin_replay_format::ReplayAdmissionLimits::default(),
        )
        .expect_err("ranked verification must reject embedded Spellforge packages");
        assert_eq!(failure.code, VerificationRejectionCodeV1::ContentNotAllowed);
        assert_eq!(failure.detail, "ranked_spellforge_package_not_allowed");
    }

    #[test]
    fn isolated_worker_is_the_sole_semantic_boundary_for_hostile_replay_bytes() {
        let disguised_jsonl = br#"{"schema_version":23,"not_bitcode":true}"#;
        let limits = robin_replay_format::ReplayAdmissionLimits {
            max_input_bytes: disguised_jsonl.len(),
            ..robin_replay_format::ReplayAdmissionLimits::default()
        };
        let failure =
            decode_replay_bounded(disguised_jsonl, &replay_artifact(disguised_jsonl), &limits)
                .expect_err("the contained verifier must reject non-compact bytes");
        assert_eq!(failure.code, VerificationRejectionCodeV1::MalformedReplay);

        let replay = single_frame_replay_with_commands(1);
        let mut noncanonical =
            robin_replay_format::encode_compact(&replay, robin_replay_format::ENGINE_VERSION_HASH)
                .unwrap()
                .into_bytes();
        noncanonical.push(b'\n');
        let failure = decode_replay_bounded(
            &noncanonical,
            &replay_artifact(&noncanonical),
            &robin_replay_format::ReplayAdmissionLimits {
                max_input_bytes: noncanonical.len(),
                ..robin_replay_format::ReplayAdmissionLimits::default()
            },
        )
        .expect_err("the contained codec must reject noncanonical envelope bytes");
        assert_eq!(failure.code, VerificationRejectionCodeV1::MalformedReplay);
    }

    #[test]
    fn campaign_output_write_failure_rolls_back_the_output() {
        let directory = tempfile::tempdir().unwrap();
        let paths = output_paths(&directory);
        seed_campaign_outputs(&paths);

        let failure = write_verified_campaign_outputs(&paths, b"over-cap", 2).unwrap_err();
        assert_eq!(failure.detail, "final_campaign_output_failed");
        assert!(failure.rollback_error.is_none());
        assert!(std::fs::read(&paths.final_campaign).unwrap().is_empty());
    }

    #[test]
    fn result_write_failure_rolls_back_the_campaign_output() {
        let directory = tempfile::tempdir().unwrap();
        let paths = output_paths(&directory);
        seed_campaign_outputs(&paths);

        assert!(write_result_with_campaign_rollback(&paths, b"over-cap", 2).is_err());
        assert!(std::fs::read(&paths.result).unwrap().is_empty());
        assert!(std::fs::read(&paths.final_campaign).unwrap().is_empty());
    }

    #[test]
    fn campaign_rollback_reports_truncate_failure() {
        let directory = tempfile::tempdir().unwrap();
        let mut paths = output_paths(&directory);
        seed_campaign_outputs(&paths);
        let invalid = directory.path().join("invalid-output-directory");
        std::fs::create_dir(&invalid).unwrap();
        paths.final_campaign = invalid;

        assert!(truncate_campaign_outputs(&paths).is_err());
    }
}
