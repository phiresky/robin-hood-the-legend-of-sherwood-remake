//! Subprocess tests of the verifier binary contract:
//! `--job FILE --replay FILE --content-root DIR --result FILE`.

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::{Command, Output};

use robin_run_protocol::{
    ArtifactRefV1, BoardSimulationPolicyV1, CanonicalDocument as _, Digest32,
    OfficialContentEditionV1, OpaqueId, RANKED_REPLAY_MEDIA_TYPE_V1, RankedSimulationDifficultyV1,
    RankedSimulationPolicyV1, ReplayArtifactV1, SCHEMA_VERSION_V2, Validate as _,
    VerificationInfrastructureFailureCodeV1, VerificationLimitsV1, VerificationRejectionCodeV1,
    VerificationStatusV2, VerifierJobV2, VerifierOutputV2,
};

const MISSION: &str = "Dem_Lei_MP";

struct Fixture {
    _directory: tempfile::TempDir,
    job: PathBuf,
    replay: PathBuf,
    content_root: PathBuf,
    result: PathBuf,
}

impl Fixture {
    fn new(job: &[u8], replay: &[u8]) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let fixture = Self {
            job: directory.path().join("job.json"),
            replay: directory.path().join("replay.rhrec"),
            content_root: directory.path().join("content"),
            result: directory.path().join("result.json"),
            _directory: directory,
        };
        std::fs::write(&fixture.job, job).unwrap();
        std::fs::write(&fixture.replay, replay).unwrap();
        std::fs::create_dir(&fixture.content_root).unwrap();
        std::fs::write(&fixture.result, b"stale result").unwrap();
        fixture
    }

    fn args(&self) -> Vec<OsString> {
        [
            ("--job", &self.job),
            ("--replay", &self.replay),
            ("--content-root", &self.content_root),
            ("--result", &self.result),
        ]
        .into_iter()
        .flat_map(|(flag, path)| [OsString::from(flag), path.as_os_str().to_owned()])
        .collect()
    }

    fn run(&self) -> Output {
        verifier_command().args(self.args()).output().unwrap()
    }

    fn output(&self) -> VerifierOutputV2 {
        let child = self.run();
        assert!(
            child.status.success(),
            "stderr: {}",
            String::from_utf8_lossy(&child.stderr)
        );
        let bytes = std::fs::read(&self.result).unwrap();
        let document = serde_json::from_slice::<VerifierOutputV2>(&bytes).unwrap();
        document.validate().unwrap();
        assert_eq!(document.canonical_bytes().unwrap(), bytes);
        assert_eq!(
            document.job_sha256,
            Digest32::digest_bytes(std::fs::read(&self.job).unwrap())
        );
        document
    }
}

fn verifier_command() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_robin-replay-verifier"));
    command.env_clear();
    command
}

fn limits() -> VerificationLimitsV1 {
    VerificationLimitsV1 {
        max_input_bytes: 1024 * 1024,
        max_compressed_bytes: 1024 * 1024,
        max_decompressed_bytes: 4 * 1024 * 1024,
        max_base64_payload_bytes: 1024 * 1024,
        max_campaign_bytes: 1024 * 1024,
        max_frames: 10_000,
        max_version_bytes: 128,
        max_mission_id_bytes: 256,
        max_metadata_records: 1024,
        max_entries_per_frame: 1024,
    }
}

fn job_for(replay: &[u8], simulation_policy: BoardSimulationPolicyV1) -> Vec<u8> {
    let job = VerifierJobV2 {
        schema_version: SCHEMA_VERSION_V2,
        job_id: OpaqueId::new("worker-cli-job").unwrap(),
        edition: OfficialContentEditionV1::Demo,
        mission_id: MISSION.into(),
        simulation_policy,
        allow_state_load: true,
        replay: ReplayArtifactV1 {
            artifact: ArtifactRefV1 {
                sha256: Digest32::digest_bytes(replay),
                byte_length: replay.len() as u64,
                media_type: RANKED_REPLAY_MEDIA_TYPE_V1.into(),
            },
            replay_schema_version: robin_run_protocol::CURRENT_RANKED_REPLAY_SCHEMA_VERSION_V1,
        },
        resource_locale_root: "1033".into(),
        limits: limits(),
    };
    job.canonical_bytes().unwrap()
}

fn standard_medium() -> BoardSimulationPolicyV1 {
    BoardSimulationPolicyV1::Fixed {
        policy: RankedSimulationPolicyV1::standard(RankedSimulationDifficultyV1::Medium),
    }
}

/// A canonical one-frame compact replay; it cannot win, but reaches every
/// check that runs before official content is loaded.
fn compact_replay(mission_id: &str, sim_config: robin_engine::engine::SimConfig) -> Vec<u8> {
    use robin_engine::engine::SimulationFrameInput;
    use robin_engine::player_command::{PlayerCommand, PlayerInput};
    use robin_engine::replay::{
        REPLAY_SCHEMA_VERSION, ReplayData, ReplayFile, ReplayFrame, ReplayHeader,
    };
    let replay: ReplayData = ReplayFile {
        header: ReplayHeader {
            mission_id: mission_id.into(),
            mission_assets: robin_engine::mission_assets::MissionAssetDescriptor::built_in(
                mission_id, mission_id, mission_id,
            )
            .unwrap(),
            rng_seed: 7,
            sim_config,
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
                input: SimulationFrameInput::from_player_inputs(vec![PlayerInput::host(
                    PlayerCommand::CrouchDown,
                )]),
                host_controls: Vec::new(),
            },
        )]
        .into(),
        hashes: [(0, 0)].into(),
        save_markers: Default::default(),
        load_backs: Default::default(),
    }
    .try_into()
    .unwrap();
    robin_replay_format::encode_compact(&replay, robin_replay_format::ENGINE_VERSION_HASH)
        .unwrap()
        .into_bytes()
}

fn assert_rejected(output: &VerifierOutputV2, code: VerificationRejectionCodeV1, detail: &str) {
    assert!(
        matches!(
            &output.status,
            VerificationStatusV2::Rejected(rejection)
                if rejection.code == code && rejection.detail_code.as_deref() == Some(detail)
        ),
        "unexpected status: {:?}",
        output.status
    );
}

#[test]
fn exact_four_flag_contract_rejects_missing_duplicate_unknown_and_equals_forms() {
    let replay = b"not a replay";
    let fixture = Fixture::new(&job_for(replay, standard_medium()), replay);
    assert!(fixture.run().status.success());

    let mut missing = fixture.args();
    missing.truncate(missing.len() - 2);
    assert!(!verifier_command().args(missing).status().unwrap().success());

    let mut duplicate = fixture.args();
    duplicate.extend([
        OsString::from("--result"),
        fixture.result.as_os_str().to_owned(),
    ]);
    assert!(
        !verifier_command()
            .args(duplicate)
            .status()
            .unwrap()
            .success()
    );

    let mut unknown = fixture.args();
    unknown.extend([OsString::from("--future"), OsString::from("/tmp/nope")]);
    assert!(!verifier_command().args(unknown).status().unwrap().success());

    let equals = format!("--job={}", fixture.job.display());
    assert!(
        !verifier_command()
            .args([equals])
            .status()
            .unwrap()
            .success()
    );
}

#[test]
fn undecodable_jobs_exit_nonzero_and_leave_the_result_empty() {
    let replay = b"replay";
    let oversized = vec![b' '; robin_run_protocol::MAX_VERIFIER_JOB_BYTES_V2 + 1];
    let mut stale_schema: serde_json::Value =
        serde_json::from_slice(&job_for(replay, standard_medium())).unwrap();
    stale_schema["schema_version"] = serde_json::json!(1);
    for job in [
        b"{".to_vec(),
        br#"{"schema_version":2,"schema_version":2}"#.to_vec(),
        oversized,
        serde_json::to_vec(&stale_schema).unwrap(),
    ] {
        let fixture = Fixture::new(&job, replay);
        let child = fixture.run();
        assert!(!child.status.success());
        assert!(std::fs::read(&fixture.result).unwrap().is_empty());
    }
}

#[test]
fn replay_bytes_differing_from_the_job_are_an_infrastructure_failure() {
    let fixture = Fixture::new(&job_for(b"expected replay", standard_medium()), b"other");
    let output = fixture.output();
    assert_eq!(output.replay_sha256, Digest32::digest_bytes(b"other"));
    assert!(matches!(
        &output.status,
        VerificationStatusV2::FailedInfrastructure(failure)
            if failure.code == VerificationInfrastructureFailureCodeV1::ArtifactIoFailure
                && failure.private_detail_code.as_deref() == Some("replay_artifact_mismatch")
    ));
}

#[cfg(unix)]
#[test]
fn unreadable_replay_is_an_infrastructure_failure() {
    use std::os::unix::fs::PermissionsExt as _;
    let replay = b"replay";
    let fixture = Fixture::new(&job_for(replay, standard_medium()), replay);
    std::fs::set_permissions(&fixture.replay, std::fs::Permissions::from_mode(0o000)).unwrap();
    let output = fixture.output();
    assert!(matches!(
        &output.status,
        VerificationStatusV2::FailedInfrastructure(failure)
            if failure.private_detail_code.as_deref() == Some("replay_artifact_io")
    ));
}

#[test]
fn replay_over_the_job_input_limit_is_a_resource_rejection() {
    let replay = vec![b'r'; 2048];
    let mut job: serde_json::Value =
        serde_json::from_slice(&job_for(&replay, standard_medium())).unwrap();
    job["limits"]["max_input_bytes"] = serde_json::json!(1024);
    let fixture = Fixture::new(&serde_json::to_vec(&job).unwrap(), &replay);
    assert_rejected(
        &fixture.output(),
        VerificationRejectionCodeV1::ResourceLimit,
        "replay_exceeds_input_limit",
    );
}

#[test]
fn hostile_non_compact_replays_are_malformed_rejections() {
    let deeply_nested = format!("{}0{}", "[".repeat(512), "]".repeat(512)).into_bytes();
    for replay in [
        b"rhrec-garbage".to_vec(),
        vec![0xff, 0xfe, 0xfd],
        br#"{"schema_version":23,"not_bitcode":true}"#.to_vec(),
        deeply_nested,
    ] {
        let fixture = Fixture::new(&job_for(&replay, standard_medium()), &replay);
        let output = fixture.output();
        assert!(
            matches!(
                &output.status,
                VerificationStatusV2::Rejected(rejection)
                    if rejection.code == VerificationRejectionCodeV1::MalformedReplay
            ),
            "unexpected status: {:?}",
            output.status
        );
        assert!(output.input_provenance.is_none());
    }
}

#[test]
fn replay_for_another_mission_is_rejected_before_content_is_loaded() {
    let replay = compact_replay(
        "Other_Mission",
        robin_engine::engine::RankedSimulationPolicy::standard_medium().expected_config(),
    );
    let fixture = Fixture::new(&job_for(&replay, standard_medium()), &replay);
    assert_rejected(
        &fixture.output(),
        VerificationRejectionCodeV1::StartingStateMismatch,
        "replay_mission_mismatch",
    );
}

#[test]
fn replay_config_outside_the_board_policy_is_rejected() {
    let mut config =
        robin_engine::engine::RankedSimulationPolicy::standard_medium().expected_config();
    config.enable_unbinding = !config.enable_unbinding;
    let replay = compact_replay(MISSION, config);
    let fixture = Fixture::new(&job_for(&replay, standard_medium()), &replay);
    let output = fixture.output();
    assert_rejected(
        &output,
        VerificationRejectionCodeV1::ConfigMismatch,
        "board_simulation_policy_mismatch",
    );
    assert_eq!(
        output.input_provenance,
        Some(robin_run_protocol::InputProvenanceStatusV1::Rankable)
    );

    // The same replay passes the policy on an any-config board and then needs
    // official content, which this fixture does not provide.
    let fixture = Fixture::new(
        &job_for(&replay, BoardSimulationPolicyV1::AnyConfig),
        &replay,
    );
    assert!(matches!(
        &fixture.output().status,
        VerificationStatusV2::FailedInfrastructure(failure)
            if failure.code == VerificationInfrastructureFailureCodeV1::ArtifactIoFailure
    ));
}

/// End-to-end resimulation of a real won compact replay against real raw
/// content. Provide `ROBIN_VERIFIER_E2E_REPLAY` (compact `.rhrec`),
/// `ROBIN_VERIFIER_E2E_CONTENT` (raw Demo datadir) and optionally
/// `ROBIN_VERIFIER_E2E_POLICY` (`standard-medium`, default, or `any`).
#[test]
#[ignore = "requires a recorded won compact replay and raw Demo content"]
fn real_won_replay_verifies_against_raw_content() {
    let replay_path = std::env::var_os("ROBIN_VERIFIER_E2E_REPLAY").expect("replay path");
    let content = PathBuf::from(std::env::var_os("ROBIN_VERIFIER_E2E_CONTENT").expect("content"));
    let replay = std::fs::read(replay_path).unwrap();
    let policy = match std::env::var("ROBIN_VERIFIER_E2E_POLICY").as_deref() {
        Ok("any") => BoardSimulationPolicyV1::AnyConfig,
        _ => standard_medium(),
    };
    let fixture = Fixture::new(&job_for(&replay, policy), &replay);
    let args = [
        ("--job", fixture.job.clone()),
        ("--replay", fixture.replay.clone()),
        ("--content-root", content),
        ("--result", fixture.result.clone()),
    ]
    .into_iter()
    .flat_map(|(flag, path)| [OsString::from(flag), path.into_os_string()]);
    let child = verifier_command().args(args).output().unwrap();
    assert!(
        child.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&child.stderr)
    );
    let output: VerifierOutputV2 =
        serde_json::from_slice(&std::fs::read(&fixture.result).unwrap()).unwrap();
    assert!(
        matches!(output.status, VerificationStatusV2::Verified(_)),
        "unexpected status: {:?}",
        output.status
    );
}
