//! One verifier job: bounded replay admission, board simulation policy,
//! official fresh start, deterministic resimulation and result projection.
//!
//! The job document is written by the trusted leaderboard worker; a job that
//! cannot be read or decoded is an infrastructure failure (non-zero exit).
//! Only the replay bytes are hostile. Every run-attributable problem becomes a
//! typed `Rejected` result.

use std::io::{self, Read as _};
use std::path::Path;

use robin_run_protocol::{
    CanonicalDocument as _, CanonicalValue, Digest32, InputProvenanceStatusV1, InputTaintKindV1,
    InputTaintV1, MAX_VERIFIER_JOB_BYTES_V2, SCHEMA_VERSION_V2, TerminalOutcomeV1, Validate as _,
    VerificationInfrastructureFailureCodeV1, VerificationInfrastructureFailureV1,
    VerificationLimitsV1, VerificationRejectionCodeV1, VerificationRejectionV1,
    VerificationStatusV2, VerifiedRunV2, VerifierJobV2, VerifierOutputV2,
};
use sha2::{Digest as _, Sha256};

use crate::worker_process::{
    AtomicOutputError, WorkerPaths, truncate_output, write_truncated_output,
};

/// Compiled replay transport ceiling shared with the canonical codec. Job
/// limits may lower, but never raise, this value.
pub const MAX_WORKER_REPLAY_BYTES: usize =
    robin_replay_format::DEFAULT_REPLAY_ADMISSION_LIMITS.max_input_bytes;

/// Compiled ceiling for the one output document.
pub const MAX_WORKER_RESULT_BYTES: usize = 1024 * 1024;

const STREAM_BUFFER_BYTES: usize = 64 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum WorkerRunError {
    #[error("reading verifier job failed: {0}")]
    JobRead(#[source] io::Error),
    #[error("verifier job exceeds {MAX_VERIFIER_JOB_BYTES_V2} bytes")]
    JobTooLarge,
    #[error("verifier job is invalid: {0}")]
    InvalidJob(String),
    #[error("verifier output is invalid: {0}")]
    InvalidOutput(String),
    #[error(transparent)]
    Output(#[from] AtomicOutputError),
}

/// Run one job and write its result document.
pub fn run_one_job(paths: &WorkerPaths) -> Result<(), WorkerRunError> {
    truncate_output(&paths.result)?;
    let job_artifact =
        stream_file(&paths.job, MAX_VERIFIER_JOB_BYTES_V2).map_err(WorkerRunError::JobRead)?;
    let job_bytes = job_artifact
        .retained_bytes
        .ok_or(WorkerRunError::JobTooLarge)?;
    let job = robin_run_protocol::strict_json::from_slice::<VerifierJobV2>(&job_bytes)
        .map_err(|error| WorkerRunError::InvalidJob(error.to_string()))?;
    job.validate()
        .map_err(|error| WorkerRunError::InvalidJob(error.to_string()))?;

    let (replay_sha256, input_provenance, status) =
        match verify(&job, &paths.replay, &paths.content_root) {
            Ok(verified) => verified,
            Err(failure) => (failure.replay_sha256, failure.provenance, failure.status),
        };
    let output = VerifierOutputV2 {
        schema_version: SCHEMA_VERSION_V2,
        job_sha256: job_artifact.sha256,
        replay_sha256,
        input_provenance,
        status,
    };
    let bytes = output
        .canonical_bytes()
        .map_err(|error| WorkerRunError::InvalidOutput(error.to_string()))?;
    write_truncated_output(&paths.result, &bytes, MAX_WORKER_RESULT_BYTES)?;
    Ok(())
}

/// Early exit carrying everything needed for the output document.
struct Failure {
    replay_sha256: Digest32,
    provenance: Option<InputProvenanceStatusV1>,
    status: VerificationStatusV2,
}

struct Stage {
    replay_sha256: Digest32,
    provenance: Option<InputProvenanceStatusV1>,
}

impl Stage {
    fn reject(&self, code: VerificationRejectionCodeV1, detail: &'static str) -> Failure {
        Failure {
            replay_sha256: self.replay_sha256,
            provenance: self.provenance.clone(),
            status: rejection(code, detail),
        }
    }

    fn infrastructure(
        &self,
        code: VerificationInfrastructureFailureCodeV1,
        detail: &'static str,
    ) -> Failure {
        Failure {
            replay_sha256: self.replay_sha256,
            provenance: self.provenance.clone(),
            status: infrastructure_failure(code, detail),
        }
    }
}

fn verify(
    job: &VerifierJobV2,
    replay_path: &Path,
    content_root: &Path,
) -> Result<
    (
        Digest32,
        Option<InputProvenanceStatusV1>,
        VerificationStatusV2,
    ),
    Failure,
> {
    let replay_cap = usize::try_from(job.limits.max_input_bytes)
        .unwrap_or(usize::MAX)
        .min(MAX_WORKER_REPLAY_BYTES);
    let mut stage = Stage {
        replay_sha256: job.replay.artifact.sha256,
        provenance: None,
    };
    let replay = stream_file(replay_path, replay_cap).map_err(|_| {
        stage.infrastructure(
            VerificationInfrastructureFailureCodeV1::ArtifactIoFailure,
            "replay_artifact_io",
        )
    })?;
    stage.replay_sha256 = replay.sha256;
    // The worker wrote these exact bytes; a mismatch is a supervisor fault.
    if replay.sha256 != job.replay.artifact.sha256
        || replay.byte_length != job.replay.artifact.byte_length
    {
        return Err(stage.infrastructure(
            VerificationInfrastructureFailureCodeV1::ArtifactIoFailure,
            "replay_artifact_mismatch",
        ));
    }
    let Some(replay_bytes) = replay.retained_bytes else {
        return Err(stage.reject(
            VerificationRejectionCodeV1::ResourceLimit,
            "replay_exceeds_input_limit",
        ));
    };

    let limits = replay_admission_limits(&job.limits);
    let (recorded_engine_version, data) =
        decode_replay_bounded(&replay_bytes, job.replay.replay_schema_version, &limits)
            .map_err(|failure| stage.reject(failure.code, failure.detail))?;
    let canonical = data
        .validate_ranked_hash_coverage()
        .and_then(|()| {
            robin_replay_format::encode_compact(&data, &recorded_engine_version)
                .map_err(|error| error.to_string())
        })
        .map_err(|_| {
            stage.reject(
                VerificationRejectionCodeV1::StateHashMismatch,
                "replay_hash_coverage_invalid",
            )
        })?;
    if canonical.as_slice() != replay_bytes.as_slice() {
        return Err(stage.reject(
            VerificationRejectionCodeV1::MalformedReplay,
            "replay_is_not_canonical",
        ));
    }
    let header = data.header();
    if header.mission_id != job.mission_id {
        return Err(stage.reject(
            VerificationRejectionCodeV1::StartingStateMismatch,
            "replay_mission_mismatch",
        ));
    }
    if data.contains_state_loads() && !job.allow_state_load {
        return Err(stage.reject(
            VerificationRejectionCodeV1::CommandNotAllowed,
            "board_state_load_not_allowed",
        ));
    }
    let provenance = replay_input_provenance(&data).map_err(|()| {
        stage.reject(
            VerificationRejectionCodeV1::MalformedReplay,
            "invalid_input_provenance",
        )
    })?;
    stage.provenance = Some(provenance.clone());
    if !provenance.is_rankable() {
        return Err(stage.reject(
            VerificationRejectionCodeV1::InputProvenanceIneligible,
            "replay_input_provenance_ineligible",
        ));
    }
    data.validate_canonical_ranked_command_admission()
        .map_err(|_| {
            stage.reject(
                VerificationRejectionCodeV1::MalformedReplay,
                "canonical_replay_shape_invalid",
            )
        })?;
    let submission_id = data.submission_id();
    let transcript = data
        .submission_transcript(submission_id, submission_id)
        .map_err(|_| {
            stage.reject(
                VerificationRejectionCodeV1::MalformedReplay,
                "replay_seat_events_invalid",
            )
        })?;
    data.validate_ranked_command_admission(&transcript)
        .map_err(|_| {
            stage.reject(
                VerificationRejectionCodeV1::CommandNotAllowed,
                "replay_seat_lifecycle_invalid",
            )
        })?;
    let sim_config = header.sim_config;
    let policy =
        robin_engine::ranked_rules::ranked_policy_for_board(job.simulation_policy, sim_config)
            .map_err(|_| {
                stage.reject(
                    VerificationRejectionCodeV1::ConfigMismatch,
                    "board_simulation_policy_mismatch",
                )
            })?;
    let sim_config_value = CanonicalValue::from_serializable(&sim_config).map_err(|_| {
        stage.infrastructure(
            VerificationInfrastructureFailureCodeV1::WorkerInternalFailure,
            "sim_config_projection_failed",
        )
    })?;

    let files = robin_ranked_verification::ranked_verifier::confined_official_files(
        content_root,
        &job.resource_locale_root,
    )
    .map_err(|error| {
        tracing::warn!(%error, "cannot confine official content");
        stage.infrastructure(
            VerificationInfrastructureFailureCodeV1::ArtifactIoFailure,
            "official_content_unavailable",
        )
    })?;
    let profiles = robin_ranked_verification::ranked_verifier::load_official_profiles(&files)
        .map_err(|error| {
            tracing::warn!(%error, "cannot load official profiles");
            stage.infrastructure(
                VerificationInfrastructureFailureCodeV1::ArtifactIoFailure,
                "official_profiles_unavailable",
            )
        })?;
    robin_engine::ranked_rules::validate_fresh_mission_start(
        sim_config,
        &profiles,
        job.edition,
        &job.mission_id,
        header.rng_seed,
        &header.campaign,
        &files,
    )
    .map_err(|error| {
        tracing::info!(%error, "starting campaign is not an official fresh start");
        stage.reject(
            VerificationRejectionCodeV1::StartingStateMismatch,
            "not_an_official_fresh_mission_start",
        )
    })?;

    let preparation = robin_ranked_verification::ranked_verifier::prepare_ranked_replay_mission(
        files,
        &profiles,
        &header.campaign,
        &header.mission_id,
        &limits,
        policy,
        header.rng_seed,
        sim_config,
    )
    .map_err(|error| {
        tracing::warn!(%error, "ranked mission preparation failed");
        let (code, detail) = ranked_loader_failure(&error);
        match code {
            FailureKind::Rejected(code) => stage.reject(code, detail),
            FailureKind::Infrastructure(code) => stage.infrastructure(code, detail),
        }
    })?;
    if !matches!(
        preparation.approved_mission_assets().source,
        robin_engine::mission_assets::MissionAssetSource::BuiltIn
    ) || &header.mission_assets != preparation.approved_mission_assets()
    {
        return Err(stage.reject(
            VerificationRejectionCodeV1::ContentNotAllowed,
            "replay_official_mission_assets_mismatch",
        ));
    }
    let starting_campaign_score = preparation.starting_campaign_score();
    let (approved_engine, assets) = preparation.into_engine_and_assets();
    let (engine, _, _, _) = approved_engine.into_parts();
    let resimulation =
        robin_engine::ranked_resim::resimulate_canonical_ranked_replay(engine, &assets, &data)
            .map_err(|error| {
                tracing::info!(%error, "ranked resimulation rejected the replay");
                let (code, detail) = ranked_resimulation_failure(&error);
                stage.reject(code, detail)
            })?;
    if resimulation.outcome != robin_engine::game_operation::GameCode::LevelSucceeded {
        return Err(stage.reject(
            VerificationRejectionCodeV1::TerminalInvalid,
            "mission_not_won",
        ));
    }
    let Some(achievement_results) = resimulation.mission_achievement_results else {
        return Err(stage.reject(
            VerificationRejectionCodeV1::ResultInvariantMismatch,
            "successful_terminal_missing_achievement_results",
        ));
    };
    let achievements = crate::result_projection::project_authoritative_achievements(
        achievement_results,
    )
    .map_err(|_| {
        stage.reject(
            VerificationRejectionCodeV1::ResultInvariantMismatch,
            "authoritative_achievement_projection_failed",
        )
    })?;
    let final_campaign_score = resimulation
        .final_campaign
        .get_value(robin_engine::campaign::CampaignValue::Score);
    let verified = VerifiedRunV2 {
        recorded_engine_version,
        sim_config: sim_config_value,
        max_concurrent_players: transcript.max_concurrent_players,
        participant_instance_count: transcript.participant_instance_count,
        outcome: TerminalOutcomeV1::Won,
        starting_campaign_score,
        final_campaign_score,
        original_score_delta: i64::from(
            final_campaign_score.wrapping_sub(starting_campaign_score) as u32
        ),
        final_state_sha256: resimulation.final_state_sha256,
        replay_frames: resimulation.replay_frames,
        active_simulation_ticks: resimulation.active_simulation_ticks,
        ransom_collected: u64::from(resimulation.mission_stat.collected_money),
        achievements,
    };
    if verified.validate().is_err() {
        return Err(stage.reject(
            VerificationRejectionCodeV1::ResultInvariantMismatch,
            "verified_result_invariant_mismatch",
        ));
    }
    Ok((
        stage.replay_sha256,
        stage.provenance,
        VerificationStatusV2::Verified(verified),
    ))
}

enum FailureKind {
    Rejected(VerificationRejectionCodeV1),
    Infrastructure(VerificationInfrastructureFailureCodeV1),
}

fn ranked_loader_failure(
    error: &robin_ranked_verification::ranked_verifier::RankedVerifierLoadError,
) -> (FailureKind, &'static str) {
    use robin_ranked_verification::ranked_verifier::RankedVerifierLoadError;
    match error {
        RankedVerifierLoadError::Campaign(_) | RankedVerifierLoadError::CampaignContent(_) => (
            FailureKind::Rejected(VerificationRejectionCodeV1::StartingStateMismatch),
            "starting_campaign_validation_failed",
        ),
        RankedVerifierLoadError::Engine(_)
        | RankedVerifierLoadError::SherwoodReferenceEngine(_) => (
            FailureKind::Infrastructure(
                VerificationInfrastructureFailureCodeV1::WorkerInternalFailure,
            ),
            "ranked_engine_preparation_failed",
        ),
        _ => (
            FailureKind::Infrastructure(VerificationInfrastructureFailureCodeV1::ArtifactIoFailure),
            "official_raw_content_load_failed",
        ),
    }
}

fn ranked_resimulation_failure(
    error: &robin_engine::ranked_resim::RankedResimulationError,
) -> (VerificationRejectionCodeV1, &'static str) {
    use robin_engine::ranked_resim::RankedResimulationError;
    match error {
        RankedResimulationError::MissingPeriodicHash { .. }
        | RankedResimulationError::StateHashMismatch { .. } => (
            VerificationRejectionCodeV1::StateHashMismatch,
            "periodic_state_hash_mismatch",
        ),
        RankedResimulationError::Admission { .. }
        | RankedResimulationError::TerminalCommandShape { .. } => (
            VerificationRejectionCodeV1::CommandNotAllowed,
            "ranked_replay_admission_failed",
        ),
        RankedResimulationError::FrameAdvance { .. } => (
            VerificationRejectionCodeV1::TimelineInvalid,
            "deterministic_frame_advance_failed",
        ),
        RankedResimulationError::TerminalBeforeEof { .. }
        | RankedResimulationError::EofBeforeTerminal
        | RankedResimulationError::UnsupportedTerminal { .. }
        | RankedResimulationError::ConflictingTerminalOutcomes { .. }
        | RankedResimulationError::TerminalCommandOutcomeMismatch { .. } => (
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

fn decode_replay_bounded(
    bytes: &[u8],
    expected_schema: u32,
    limits: &robin_replay_format::ReplayAdmissionLimits,
) -> Result<(String, robin_engine::replay::ReplayData), ReplayRejection> {
    let (recorded_hash, replay) = robin_replay_format::decode_compact_bounded(bytes, limits)
        .map_err(compact_replay_rejection)?;
    // Ranked boards admit only shipped SCB content. Custom mission archives
    // and embedded Spellforge executables are rejected immediately after the
    // hostile decode so no later setup can mount or execute them.
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
    if replay.header().version != expected_schema
        || expected_schema != robin_engine::replay::REPLAY_SCHEMA_VERSION
    {
        return Err(ReplayRejection {
            code: VerificationRejectionCodeV1::UnsupportedSchema,
            detail: "replay_header_schema_mismatch",
        });
    }
    Ok((recorded_hash, replay))
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
        _ => ReplayRejection {
            code: VerificationRejectionCodeV1::MalformedReplay,
            detail: "compact_replay_invalid",
        },
    }
}

fn replay_admission_limits(
    configured: &VerificationLimitsV1,
) -> robin_replay_format::ReplayAdmissionLimits {
    let compiled = robin_replay_format::DEFAULT_REPLAY_ADMISSION_LIMITS;
    let lower =
        |value: u64, ceiling: usize| usize::try_from(value).unwrap_or(usize::MAX).min(ceiling);
    let mut limits = compiled;
    limits.max_input_bytes = lower(configured.max_input_bytes, compiled.max_input_bytes);
    limits.max_compressed_bytes = lower(
        configured.max_compressed_bytes,
        compiled.max_compressed_bytes,
    );
    limits.max_decompressed_bytes = lower(
        configured.max_decompressed_bytes,
        compiled.max_decompressed_bytes,
    );
    limits.max_version_hash_bytes = lower(
        u64::from(configured.max_version_bytes),
        compiled.max_version_hash_bytes,
    );
    limits.max_mission_id_bytes = lower(
        u64::from(configured.max_mission_id_bytes),
        compiled.max_mission_id_bytes,
    );
    limits.max_campaign_bytes = lower(configured.max_campaign_bytes, compiled.max_campaign_bytes);
    limits.max_frames = lower(u64::from(configured.max_frames), compiled.max_frames);
    limits.max_entries_per_frame = lower(
        u64::from(configured.max_entries_per_frame),
        compiled.max_entries_per_frame,
    );
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

fn rejection(code: VerificationRejectionCodeV1, detail: &'static str) -> VerificationStatusV2 {
    VerificationStatusV2::Rejected(VerificationRejectionV1 {
        code,
        detail_code: Some(detail.into()),
    })
}

fn infrastructure_failure(
    code: VerificationInfrastructureFailureCodeV1,
    detail: &'static str,
) -> VerificationStatusV2 {
    VerificationStatusV2::FailedInfrastructure(VerificationInfrastructureFailureV1 {
        code,
        private_detail_code: Some(detail.into()),
    })
}

struct BoundedArtifact {
    sha256: Digest32,
    byte_length: u64,
    retained_bytes: Option<Vec<u8>>,
}

/// Hash a whole file while retaining at most `maximum_retained` bytes.
fn stream_file(path: &Path, maximum_retained: usize) -> io::Result<BoundedArtifact> {
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
            .ok_or_else(|| io::Error::other("artifact byte length overflow"))?;
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

    const SCHEMA: u32 = robin_engine::replay::REPLAY_SCHEMA_VERSION;

    #[test]
    fn bounded_reader_hashes_all_bytes_without_retaining_oversize_input() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("artifact");
        let bytes = vec![0x5a; 129];
        std::fs::write(&path, &bytes).unwrap();

        let artifact = stream_file(&path, 128).unwrap();
        assert_eq!(artifact.byte_length, 129);
        assert_eq!(artifact.sha256, Digest32::digest_bytes(&bytes));
        assert!(artifact.retained_bytes.is_none());
    }

    #[test]
    fn decode_rejects_compact_single_frame_command_amplification() {
        let replay = single_frame_replay_with_commands(4_096);
        let compact =
            robin_replay_format::encode_compact(&replay, robin_replay_format::ENGINE_VERSION_HASH)
                .unwrap();
        let limits = robin_replay_format::ReplayAdmissionLimits {
            max_input_bytes: compact.len(),
            max_entries_per_frame: 64,
            ..robin_replay_format::ReplayAdmissionLimits::default()
        };
        let failure = decode_replay_bounded(&compact, SCHEMA, &limits)
            .expect_err("compact replay must enforce the entry ceiling");
        assert_eq!(failure.code, VerificationRejectionCodeV1::ResourceLimit);
    }

    #[test]
    fn decode_accepts_another_recording_commit_and_rejects_other_schemas() {
        let replay = single_frame_replay_with_commands(0);
        let recorded_hash = "0123456789ab";
        assert_ne!(recorded_hash, robin_replay_format::ENGINE_VERSION_HASH);
        let compact = robin_replay_format::encode_compact(&replay, recorded_hash).unwrap();
        let (hash, decoded) = decode_replay_bounded(&compact, SCHEMA, &Default::default()).unwrap();
        assert_eq!(hash, recorded_hash);
        assert_eq!(
            robin_replay_format::encode_compact(&decoded, &hash).unwrap(),
            compact
        );
        let failure = decode_replay_bounded(&compact, SCHEMA + 1, &Default::default()).unwrap_err();
        assert_eq!(failure.code, VerificationRejectionCodeV1::UnsupportedSchema);
    }

    #[test]
    fn decode_rejects_custom_mission_content_as_unrankable() {
        let mut archive = single_frame_replay_with_commands(1);
        archive
            .try_edit_header(|header| header.mission_assets = archive_mission_assets())
            .unwrap();
        let compact =
            robin_replay_format::encode_compact(&archive, robin_replay_format::ENGINE_VERSION_HASH)
                .unwrap();
        let failure = decode_replay_bounded(&compact, SCHEMA, &Default::default()).unwrap_err();
        assert_eq!(failure.code, VerificationRejectionCodeV1::ContentNotAllowed);
        assert_eq!(failure.detail, "ranked_archive_mission_not_allowed");

        archive
            .try_edit_header(|header| header.spellforge_package = Some(spellforge_package()))
            .unwrap();
        let compact =
            robin_replay_format::encode_compact(&archive, robin_replay_format::ENGINE_VERSION_HASH)
                .unwrap();
        let failure = decode_replay_bounded(&compact, SCHEMA, &Default::default()).unwrap_err();
        assert_eq!(failure.code, VerificationRejectionCodeV1::ContentNotAllowed);
        assert_eq!(failure.detail, "ranked_spellforge_package_not_allowed");
    }

    #[test]
    fn decode_rejects_non_compact_bytes() {
        let disguised_jsonl = br#"{"schema_version":23,"not_bitcode":true}"#;
        let failure =
            decode_replay_bounded(disguised_jsonl, SCHEMA, &Default::default()).unwrap_err();
        assert_eq!(failure.code, VerificationRejectionCodeV1::MalformedReplay);
    }

    #[test]
    fn configured_limits_only_lower_compiled_ceilings() {
        let compiled = robin_replay_format::DEFAULT_REPLAY_ADMISSION_LIMITS;
        let huge = VerificationLimitsV1 {
            max_input_bytes: u64::MAX,
            max_compressed_bytes: u64::MAX,
            max_decompressed_bytes: u64::MAX,
            max_campaign_bytes: u64::MAX,
            max_frames: u32::MAX,
            max_version_bytes: u32::MAX,
            max_mission_id_bytes: u32::MAX,
            max_entries_per_frame: u32::MAX,
        };
        let limits = replay_admission_limits(&huge);
        assert_eq!(limits.max_input_bytes, compiled.max_input_bytes);
        assert_eq!(limits.max_frames, compiled.max_frames);
        let small = VerificationLimitsV1 {
            max_frames: 5,
            ..huge
        };
        assert_eq!(replay_admission_limits(&small).max_frames, 5);
    }
}
