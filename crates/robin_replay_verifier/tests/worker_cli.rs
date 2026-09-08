use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use ed25519_dalek::{Signer as _, SigningKey};
use robin_replay_verifier::worker::{MAX_WORKER_REQUEST_BYTES, MAX_WORKER_RESULT_BYTES};
use robin_run_protocol::{
    ArtifactRefV1, BoardMetricV1, CampaignAggregationConsentV1, CanonicalDocument as _,
    ChallengeNonce32, Digest32, FreshRunPreflightGrantClaimV1, FreshRunPreflightGrantV1,
    FreshRunPreflightRequestClaimV1, FreshRunPreflightRequestV1, FreshRunScopeV1,
    InitialStateExpectationV1, OfficialContentEditionV1, OfficialContentSubjectV1, OpaqueId,
    ParticipantClaimV1, ParticipantPublicDisclosureV1, ParticipantSignatureV1, PublicKey32,
    RANKED_CAMPAIGN_MEDIA_TYPE_V1, RANKED_REPLAY_MEDIA_TYPE_V1, RankedSessionConfigV1,
    ReplayArtifactV1, ReplaySeatLifecycleEventV1, ReplaySeatLifecycleKindV1,
    ReplaySessionGenesisClaimV1, ReplaySessionGenesisV1, ReplaySessionTranscriptV1,
    ResourceLocaleRootV1, Signature64, SignatureAlgorithmV1, SignedSubmissionV1, SimulationSeed64,
    SpeechTimingAuthorityV1, SubmissionArtifactsV1, SubmissionEnvelopeV1, SubmissionOfferV1,
    Validate as _, VerificationInfrastructureFailureCodeV1, VerificationLimitsV1,
    VerificationRejectionCodeV1, VerificationRequestV1, VerificationStatusV1,
    VerifierAdmissionFailureCodeV1, VerifierWorkerOutputV1,
};

const REPLAY_BYTES: &[u8] = b"observed replay bytes";
const STARTING_CAMPAIGN_BYTES: &[u8] = b"starting campaign";

struct Fixture {
    _directory: tempfile::TempDir,
    request: PathBuf,
    replay: PathBuf,
    config: PathBuf,
    starting_campaign: PathBuf,
    final_campaign: PathBuf,
    result: PathBuf,
}

impl Fixture {
    fn new(request: &[u8]) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let path = |name: &str| directory.path().join(name);
        let fixture = Self {
            request: path("request.json"),
            replay: path("replay.rhrec"),
            config: path("config.json"),
            starting_campaign: path("starting-campaign.bin"),
            final_campaign: path("final-campaign.bin"),
            result: path("result.json"),
            _directory: directory,
        };
        std::fs::write(&fixture.request, request).unwrap();
        std::fs::write(&fixture.replay, REPLAY_BYTES).unwrap();
        std::fs::write(&fixture.config, b"{}").unwrap();
        std::fs::write(&fixture.starting_campaign, STARTING_CAMPAIGN_BYTES).unwrap();
        std::fs::write(&fixture.final_campaign, b"stale final campaign").unwrap();
        std::fs::write(&fixture.result, b"stale result").unwrap();
        fixture
    }

    fn exact_args(&self) -> Vec<OsString> {
        [
            ("--request", &self.request),
            ("--replay", &self.replay),
            ("--config", &self.config),
            ("--starting-campaign", &self.starting_campaign),
            ("--final-campaign", &self.final_campaign),
            ("--result", &self.result),
        ]
        .into_iter()
        .flat_map(|(flag, path)| [OsString::from(flag), path.as_os_str().to_owned()])
        .collect()
    }

    fn run(&self) -> Output {
        verifier_command().args(self.exact_args()).output().unwrap()
    }

    fn output_document(&self) -> (Vec<u8>, VerifierWorkerOutputV1) {
        let bytes = std::fs::read(&self.result).unwrap();
        let document = serde_json::from_slice::<VerifierWorkerOutputV1>(&bytes).unwrap();
        (bytes, document)
    }
}

fn verifier_command() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_robin-replay-verifier"));
    command.env_clear();
    command
}

fn authenticated_request(replay: &[u8], max_input_bytes: u64) -> Vec<u8> {
    let signing_key = SigningKey::from_bytes(&[7; 32]);
    let public_key = PublicKey32::from_bytes(signing_key.verifying_key().to_bytes());
    let starting_campaign_sha256 = Digest32::digest_bytes(STARTING_CAMPAIGN_BYTES);
    let build_manifest_sha256 = Digest32::from_bytes([4; 32]);
    let content_manifest_sha256 = Digest32::from_bytes([5; 32]);
    let rules_config_sha256 = Digest32::from_bytes([6; 32]);
    let ruleset_manifest_sha256 = Digest32::from_bytes([9; 32]);
    let host_participant_instance_id = Digest32::from_bytes([12; 32]);
    let replay_session_id = Digest32::from_bytes([11; 32]);
    let host_nonce = ChallengeNonce32::from_bytes([13; 32]);
    let ranked_session = RankedSessionConfigV1 {
        schema_version: 1,
        mission_id: "Dem_Lei_MP".into(),
        content_edition: OfficialContentEditionV1::Demo,
        content_subject: OfficialContentSubjectV1::FieldMission {
            mission_id: "Dem_Lei_MP".into(),
        },
        simulation_seed: SimulationSeed64::new(42),
        starting_campaign_sha256,
        starting_campaign_byte_length: STARTING_CAMPAIGN_BYTES.len() as u64,
        prepared_inputs_projection_sha256: Digest32::from_bytes([18; 32]),
        prepared_mission_inputs_seal_sha256: Digest32::from_bytes([19; 32]),
        build_manifest_sha256,
        content_manifest_sha256,
        campaign_content_manifest_sha256: None,
        rules_config_sha256,
        ruleset_manifest_sha256,
        competition_manifest_sha256: None,
        spellforge_content_sha256: None,
        resource_locale_root: ResourceLocaleRootV1::new("1033").unwrap(),
        speech_timing: SpeechTimingAuthorityV1::BaseInstallation,
    };
    let starting_campaign = ArtifactRefV1 {
        sha256: starting_campaign_sha256,
        byte_length: STARTING_CAMPAIGN_BYTES.len() as u64,
        media_type: RANKED_CAMPAIGN_MEDIA_TYPE_V1.into(),
    };
    let preflight_request_claim = FreshRunPreflightRequestClaimV1 {
        schema_version: 1,
        request_nonce: ChallengeNonce32::from_bytes([20; 32]),
        host_public_key: public_key,
        replay_session_id,
        host_participant_instance_id,
        host_nonce,
        scope: FreshRunScopeV1::IndividualLevel,
        starting_campaign: starting_campaign.clone(),
        ranked_session: ranked_session.clone(),
    };
    let preflight_request = FreshRunPreflightRequestV1 {
        host_signature: Signature64::from_bytes(
            signing_key
                .sign(&preflight_request_claim.signing_bytes().unwrap())
                .to_bytes(),
        ),
        claim: preflight_request_claim,
        algorithm: SignatureAlgorithmV1::Ed25519,
    };
    let grant_authority = SigningKey::from_bytes(&[21; 32]);
    let preflight_grant_claim = FreshRunPreflightGrantClaimV1 {
        schema_version: 1,
        grant_id: OpaqueId::new("worker-cli-fresh-run-grant").unwrap(),
        grant_nonce: ChallengeNonce32::from_bytes([22; 32]),
        grant_authority_public_key: PublicKey32::from_bytes(
            grant_authority.verifying_key().to_bytes(),
        ),
        host_public_key: public_key,
        grant_request_sha256: preflight_request.canonical_digest().unwrap(),
        ranked_session_sha256: ranked_session.canonical_digest().unwrap(),
        replay_session_id,
        host_participant_instance_id,
        host_nonce,
        scope: FreshRunScopeV1::IndividualLevel,
        starting_campaign,
        admitted_at_unix_ms: 1_700_000_000_000,
        expires_at_unix_ms: 1_800_000_000_000,
    };
    let preflight_grant = FreshRunPreflightGrantV1 {
        authority_signature: Signature64::from_bytes(
            grant_authority
                .sign(&preflight_grant_claim.signing_bytes().unwrap())
                .to_bytes(),
        ),
        claim: preflight_grant_claim,
        algorithm: SignatureAlgorithmV1::Ed25519,
    };
    preflight_grant
        .validate_request(&preflight_request)
        .unwrap();

    let genesis_claim = ReplaySessionGenesisClaimV1 {
        schema_version: 1,
        network_protocol_version: robin_engine::multiplayer::NET_PROTOCOL_VERSION,
        host_public_key: public_key,
        replay_session_id,
        host_participant_instance_id,
        host_nonce,
        ranked_session,
        fresh_run_preflight_grant: Some(preflight_grant),
        campaign_continuation_preflight_grant: None,
        competition_run_grant: None,
    };
    let genesis_signature = signing_key.sign(&genesis_claim.signing_bytes().unwrap());
    let genesis = ReplaySessionGenesisV1 {
        claim: genesis_claim,
        algorithm: SignatureAlgorithmV1::Ed25519,
        host_signature: Signature64::from_bytes(genesis_signature.to_bytes()),
    };
    let replay_session_transcript = ReplaySessionTranscriptV1 {
        schema_version: 1,
        session_genesis_sha256: genesis.canonical_digest().unwrap(),
        replay_session_id: genesis.claim.replay_session_id,
        host_participant_instance_id,
        participant_instance_count: 1,
        max_concurrent_players: 1,
        events: vec![ReplaySeatLifecycleEventV1 {
            event_ordinal: 0,
            replay_ordinal: 0,
            seat: 0,
            participant_instance_id: host_participant_instance_id,
            lifecycle: ReplaySeatLifecycleKindV1::Connected {
                connection_epoch: 0,
            },
        }],
    };
    let host_claim = ParticipantClaimV1 {
        seat: 0,
        participant_instance_id: host_participant_instance_id,
        public_key,
        public_disclosure: ParticipantPublicDisclosureV1::NamedProfile,
        join_attestation: None,
    };
    let offer = SubmissionOfferV1 {
        schema_version: 1,
        upload_challenge_id: OpaqueId::new("worker-cli-challenge").unwrap(),
        upload_challenge_nonce: ChallengeNonce32::from_bytes([2; 32]),
        expires_at_unix_ms: 1_800_000_000_000,
        max_concurrent_players: 1,
        participant_instance_count: 1,
        participant_claims: vec![host_claim],
        session_genesis: genesis,
        mission_id: "Dem_Lei_MP".into(),
        competition_manifest_sha256: None,
        build_manifest_sha256,
        content_manifest_sha256,
        rules_config_sha256,
        ruleset_manifest_sha256,
        starting_state: InitialStateExpectationV1::IndividualLevel {
            template_id: OpaqueId::new("leicester-default").unwrap(),
            campaign_state_requirement: robin_run_protocol::CanonicalCampaignStateRequirementV1 {
                edition: OfficialContentEditionV1::Demo,
                kind: robin_run_protocol::CanonicalCampaignStateKindV1::IndividualTemplate,
                rules_config_sha256,
            },
            campaign_sha256: starting_campaign_sha256,
            starting_campaign_byte_length: STARTING_CAMPAIGN_BYTES.len() as u64,
        },
        allowed_metrics: vec![BoardMetricV1::OriginalScore],
    };
    let submission = SubmissionEnvelopeV1 {
        schema_version: 1,
        replay_session_transcript,
        offer,
        artifacts: SubmissionArtifactsV1 {
            replay: ReplayArtifactV1 {
                artifact: ArtifactRefV1 {
                    sha256: Digest32::digest_bytes(replay),
                    byte_length: replay.len() as u64,
                    media_type: RANKED_REPLAY_MEDIA_TYPE_V1.into(),
                },
                replay_schema_version: robin_run_protocol::CURRENT_RANKED_REPLAY_SCHEMA_VERSION_V1,
            },
            starting_campaign: ArtifactRefV1 {
                sha256: starting_campaign_sha256,
                byte_length: STARTING_CAMPAIGN_BYTES.len() as u64,
                media_type: robin_run_protocol::RANKED_CAMPAIGN_MEDIA_TYPE_V1.into(),
            },
        },
        campaign_aggregation_consent: CampaignAggregationConsentV1::NotAuthorized,
        campaign_continuation_authorization: None,
        requested_metrics: vec![BoardMetricV1::OriginalScore],
    };
    let submission_signature = signing_key.sign(&submission.signing_bytes().unwrap());
    let signed_submission = SignedSubmissionV1 {
        schema_version: 1,
        submission,
        algorithm: SignatureAlgorithmV1::Ed25519,
        participant_signatures: vec![ParticipantSignatureV1 {
            public_key,
            signature: Signature64::from_bytes(submission_signature.to_bytes()),
        }],
    };
    let request = VerificationRequestV1 {
        schema_version: 1,
        request_id: OpaqueId::new("worker-cli-request").unwrap(),
        submission: signed_submission,
        limits: VerificationLimitsV1 {
            max_input_bytes,
            max_compressed_bytes: max_input_bytes,
            max_decompressed_bytes: max_input_bytes,
            max_base64_payload_bytes: max_input_bytes,
            max_campaign_bytes: 1024,
            max_frames: 10_000,
            max_version_bytes: 128,
            max_mission_id_bytes: 256,
            max_metadata_records: 1024,
            max_entries_per_frame: 1024,
        },
    };
    assert!(request.validate().is_ok());
    request.canonical_bytes().unwrap()
}

fn assert_campaign_outputs_empty(fixture: &Fixture) {
    assert!(std::fs::read(&fixture.final_campaign).unwrap().is_empty());
}

fn assert_admission_failure(
    fixture: &Fixture,
    request: &[u8],
    expected_code: VerifierAdmissionFailureCodeV1,
    expected_detail: &str,
) {
    let output = fixture.run();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_campaign_outputs_empty(fixture);

    let (bytes, document) = fixture.output_document();
    assert!(bytes.len() <= MAX_WORKER_RESULT_BYTES);
    assert!(document.validate().is_ok());
    assert_eq!(document.canonical_bytes().unwrap(), bytes);
    let VerifierWorkerOutputV1::AdmissionFailure {
        request_artifact_sha256,
        code,
        bounded_detail,
        ..
    } = document
    else {
        panic!("untrusted request unexpectedly produced a verification result")
    };
    assert_eq!(request_artifact_sha256, Digest32::digest_bytes(request));
    assert_eq!(code, expected_code);
    assert_eq!(bounded_detail.as_deref(), Some(expected_detail));
}

#[test]
fn exact_six_flag_contract_rejects_missing_duplicate_unknown_and_equals_forms() {
    let fixture = Fixture::new(b"{}");

    let valid = fixture.run();
    assert!(valid.status.success());

    let mut missing = fixture.exact_args();
    missing.truncate(missing.len() - 2);
    assert!(!verifier_command().args(missing).status().unwrap().success());

    let mut duplicate = fixture.exact_args();
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

    let mut unknown = fixture.exact_args();
    unknown.extend([OsString::from("--future"), OsString::from("/tmp/nope")]);
    assert!(!verifier_command().args(unknown).status().unwrap().success());

    let equals = format!("--request={}", fixture.request.display());
    assert!(
        !verifier_command()
            .args([equals])
            .status()
            .unwrap()
            .success()
    );
}

#[test]
fn malformed_and_recursively_duplicate_json_are_bound_only_to_raw_request_digest() {
    for (request, detail) in [
        (b"{".as_slice(), "malformed_json"),
        (
            br#"{"schema_version":1,"submission":{"nested":{"value":1,"value":2}}}"#.as_slice(),
            "duplicate_json_key",
        ),
    ] {
        let fixture = Fixture::new(request);
        assert_admission_failure(
            &fixture,
            request,
            VerifierAdmissionFailureCodeV1::MalformedRequest,
            detail,
        );
    }
}

#[test]
fn oversized_request_is_stream_hashed_but_never_retained_or_decoded() {
    let request = vec![b' '; MAX_WORKER_REQUEST_BYTES + 1];
    let fixture = Fixture::new(&request);
    assert_admission_failure(
        &fixture,
        &request,
        VerifierAdmissionFailureCodeV1::RequestTooLarge,
        "request_exceeds_worker_cap",
    );
}

#[test]
fn authenticated_job_with_invalid_operator_config_fails_as_infrastructure() {
    let request = authenticated_request(REPLAY_BYTES, 1024);
    let fixture = Fixture::new(&request);
    let child = fixture.run();
    assert!(
        child.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&child.stderr)
    );
    assert_campaign_outputs_empty(&fixture);

    let (_, document) = fixture.output_document();
    let VerifierWorkerOutputV1::VerificationResult { result, .. } = document else {
        panic!("authenticated request did not produce a verification result")
    };
    assert_eq!(
        result.artifacts.replay.artifact.sha256,
        Digest32::digest_bytes(REPLAY_BYTES)
    );
    assert!(
        matches!(
            &result.status,
            VerificationStatusV1::FailedInfrastructure(failure)
                if failure.code == VerificationInfrastructureFailureCodeV1::WorkerInternalFailure
                    && failure.private_detail_code.as_deref() == Some("job_config_invalid")
        ),
        "unexpected status: {:?}",
        result.status
    );
    assert!(!matches!(result.status, VerificationStatusV1::Verified(_)));
}

#[test]
fn authenticated_starting_campaign_digest_mismatch_is_a_typed_run_rejection() {
    let request = authenticated_request(REPLAY_BYTES, 1024);
    let fixture = Fixture::new(&request);
    std::fs::write(&fixture.starting_campaign, b"substituted campaign").unwrap();
    let child = fixture.run();
    assert!(child.status.success());
    assert_campaign_outputs_empty(&fixture);

    let (_, document) = fixture.output_document();
    let VerifierWorkerOutputV1::VerificationResult { result, .. } = document else {
        panic!("authenticated request did not produce a verification result")
    };
    assert!(
        matches!(
            &result.status,
            VerificationStatusV1::Rejected(rejection)
                if rejection.code == VerificationRejectionCodeV1::StartingStateMismatch
                    && rejection.detail_code.as_deref()
                        == Some("starting_campaign_identity_mismatch")
        ),
        "unexpected status: {:?}",
        result.status
    );
}

#[test]
fn authenticated_starting_campaign_signed_limit_is_enforced_before_decode() {
    let request = authenticated_request(REPLAY_BYTES, 1024);
    let fixture = Fixture::new(&request);
    std::fs::write(&fixture.starting_campaign, vec![0x5a; 1025]).unwrap();
    let child = fixture.run();
    assert!(child.status.success());
    assert_campaign_outputs_empty(&fixture);

    let (_, document) = fixture.output_document();
    let VerifierWorkerOutputV1::VerificationResult { result, .. } = document else {
        panic!("authenticated request did not produce a verification result")
    };
    assert!(
        matches!(
            &result.status,
            VerificationStatusV1::Rejected(rejection)
                if rejection.code == VerificationRejectionCodeV1::ResourceLimit
                    && rejection.detail_code.as_deref()
                        == Some("starting_campaign_exceeds_signed_limit")
        ),
        "unexpected status: {:?}",
        result.status
    );
}

#[test]
fn authenticated_replay_artifact_mismatch_is_a_typed_run_rejection() {
    let request = authenticated_request(b"different replay", 1024);
    let fixture = Fixture::new(&request);
    let child = fixture.run();
    assert!(child.status.success());
    assert_campaign_outputs_empty(&fixture);

    let (_, document) = fixture.output_document();
    let VerifierWorkerOutputV1::VerificationResult { result, .. } = document else {
        panic!("authenticated request did not produce a verification result")
    };
    assert_eq!(
        result.artifacts.replay.artifact.sha256,
        Digest32::digest_bytes(b"different replay")
    );
    assert!(matches!(
        result.status,
        VerificationStatusV1::Rejected(ref rejection)
            if rejection.code == VerificationRejectionCodeV1::MalformedReplay
                && rejection.detail_code.as_deref()
                    == Some("replay_artifact_identity_mismatch")
    ));
}

#[cfg(unix)]
#[test]
fn unreadable_replay_has_no_fabricated_verification_proof_digest() {
    use std::os::unix::fs::PermissionsExt as _;

    let request = authenticated_request(REPLAY_BYTES, 1024);
    let fixture = Fixture::new(&request);
    std::fs::set_permissions(&fixture.replay, std::fs::Permissions::from_mode(0o000)).unwrap();
    let child = fixture.run();
    assert!(
        child.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&child.stderr)
    );
    assert_campaign_outputs_empty(&fixture);

    let (_, document) = fixture.output_document();
    let VerifierWorkerOutputV1::AdmissionFailure {
        request_artifact_sha256,
        code,
        bounded_detail,
        ..
    } = document
    else {
        panic!("unreadable replay unexpectedly produced verification proof fields")
    };
    assert_eq!(request_artifact_sha256, Digest32::digest_bytes(&request));
    assert_eq!(code, VerifierAdmissionFailureCodeV1::WorkerInternalFailure);
    assert_eq!(bounded_detail.as_deref(), Some("replay_artifact_io"));
}

#[cfg(unix)]
#[test]
fn unreadable_starting_campaign_has_only_a_raw_request_admission_failure() {
    use std::os::unix::fs::PermissionsExt as _;

    let request = authenticated_request(REPLAY_BYTES, 1024);
    let fixture = Fixture::new(&request);
    std::fs::set_permissions(
        &fixture.starting_campaign,
        std::fs::Permissions::from_mode(0o000),
    )
    .unwrap();
    assert_admission_failure(
        &fixture,
        &request,
        VerifierAdmissionFailureCodeV1::WorkerInternalFailure,
        "starting_campaign_artifact_io",
    );
}

#[test]
fn incomplete_replay_and_starting_artifacts_are_retryable_admission_failures() {
    for (target, detail) in [
        ("replay", "replay_artifact_incomplete"),
        ("starting", "starting_campaign_artifact_incomplete"),
    ] {
        let request = authenticated_request(REPLAY_BYTES, 1024);
        let fixture = Fixture::new(&request);
        let path = match target {
            "replay" => &fixture.replay,
            "starting" => &fixture.starting_campaign,
            _ => unreachable!(),
        };
        std::fs::write(path, b"x").unwrap();
        assert_admission_failure(
            &fixture,
            &request,
            VerifierAdmissionFailureCodeV1::WorkerInternalFailure,
            detail,
        );
    }
}

#[cfg(unix)]
#[test]
fn earlier_identity_mismatches_never_mask_later_input_io_failures() {
    use std::os::unix::fs::PermissionsExt as _;

    let mismatch_request = authenticated_request(b"signed different replay", 1024);
    let fixture = Fixture::new(&mismatch_request);
    std::fs::set_permissions(
        &fixture.starting_campaign,
        std::fs::Permissions::from_mode(0o000),
    )
    .unwrap();
    assert_admission_failure(
        &fixture,
        &mismatch_request,
        VerifierAdmissionFailureCodeV1::WorkerInternalFailure,
        "starting_campaign_artifact_io",
    );
}

#[test]
fn precreated_outputs_remain_distinct_and_nonverified_campaign_is_empty() {
    let request = b"{}";
    let fixture = Fixture::new(request);
    let final_identity = file_identity(&fixture.final_campaign);
    let result_identity = file_identity(&fixture.result);
    assert_ne!(final_identity, result_identity);

    assert_admission_failure(
        &fixture,
        request,
        VerifierAdmissionFailureCodeV1::MalformedRequest,
        "malformed_json",
    );
    assert_eq!(file_identity(&fixture.final_campaign), final_identity);
    assert_eq!(file_identity(&fixture.result), result_identity);
}

#[test]
fn hostile_json_corpus_is_contained_by_a_fresh_limited_child_each_time() {
    let deeply_nested = format!("{}0{}", "[".repeat(512), "]".repeat(512)).into_bytes();
    let corpus = [
        (Vec::new(), "malformed_json"),
        (vec![0xff, 0xfe, 0xfd], "malformed_json"),
        (
            br#"{"a":[{"b":true,"b":false}]}"#.to_vec(),
            "duplicate_json_key",
        ),
        (deeply_nested, "malformed_json"),
        (br#"null trailing"#.to_vec(), "malformed_json"),
    ];

    for (request, expected_detail) in corpus {
        let fixture = Fixture::new(&request);
        assert_admission_failure(
            &fixture,
            &request,
            VerifierAdmissionFailureCodeV1::MalformedRequest,
            expected_detail,
        );
    }
}

#[cfg(unix)]
fn file_identity(path: &Path) -> (u64, u64) {
    use std::os::unix::fs::MetadataExt as _;
    let metadata = std::fs::metadata(path).unwrap();
    (metadata.dev(), metadata.ino())
}

#[cfg(not(unix))]
fn file_identity(path: &Path) -> PathBuf {
    path.to_path_buf()
}
