use crate::support::*;

async fn public_status(rig: &TestRig, submission_id: &OpaqueId) -> PublicSubmissionStateV1 {
    let response = rig
        .send(empty_request(
            Method::GET,
            &format!("/api/v1/submissions/{submission_id}/public-status"),
            Ipv4Addr::LOCALHOST,
        ))
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    let status: PublicSubmissionStatusV1 = json_body(response).await;
    status.validate().unwrap();
    status.state
}

async fn private_status(
    rig: &TestRig,
    key: &SigningKey,
    submission_id: &OpaqueId,
) -> axum::response::Response {
    let challenge: SubmissionOwnerStatusChallengeV1 = json_body(
        rig.send(json_request(
            Method::POST,
            "/api/v1/submission-owner-status-challenges",
            &SubmissionOwnerStatusChallengeRequestV1 {
                schema_version: SCHEMA_VERSION_V1,
                controller_public_key: protocol_public_key(key),
                submission_id: submission_id.clone(),
            },
            Ipv4Addr::LOCALHOST,
        ))
        .await,
    )
    .await;
    let mut envelope = SubmissionOwnerStatusEnvelopeV1 {
        schema_version: SCHEMA_VERSION_V1,
        challenge,
        algorithm: SignatureAlgorithmV1::Ed25519,
        signature: Signature64::from_bytes([0; 64]),
    };
    envelope.signature = sign(key, &envelope.signing_bytes().unwrap());
    rig.send(json_request(
        Method::POST,
        &format!("/api/v1/submissions/{submission_id}/private-status"),
        &envelope,
        Ipv4Addr::LOCALHOST,
    ))
    .await
}

#[tokio::test]
async fn upload_challenge_signed_submission_queue_and_verified_publication_cross_the_router() {
    let rig = TestRig::new().await;
    let owner = SigningKey::from_bytes(&[0x11; 32]);
    rig.rename(&owner, "Robin", Ipv4Addr::new(127, 0, 0, 2))
        .await;

    let metadata: LeaderboardMetadataV2 = json_body(
        rig.send(empty_request(
            Method::GET,
            "/api/v1/leaderboard-metadata",
            Ipv4Addr::LOCALHOST,
        ))
        .await,
    )
    .await;
    metadata.validate().unwrap();
    assert_eq!(metadata.boards.len(), 2);
    assert_eq!(metadata.tick_duration.numerator_micros, 40_000);

    let replay = compact_replay_fixture("published-run");
    let accepted = rig
        .submit(&owner, &replay, ParticipantPublicDisclosureV1::NamedProfile)
        .await;
    accepted.validate().unwrap();
    assert_eq!(accepted.state, SubmissionLifecycleV1::Queued);
    assert_eq!(
        public_status(&rig, &accepted.submission_id).await,
        PublicSubmissionStateV1::Queued
    );
    let owner_response = private_status(&rig, &owner, &accepted.submission_id).await;
    assert_eq!(owner_response.status(), StatusCode::OK);
    let owner_status: SubmissionOwnerStatusResponseV1 = json_body(owner_response).await;
    assert_eq!(owner_status.state, SubmissionLifecycleV1::Queued);
    let stranger = SigningKey::from_bytes(&[0x12; 32]);
    assert_eq!(
        private_status(&rig, &stranger, &accepted.submission_id)
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );

    let run_id = rig
        .accept_as_worker(
            &accepted.submission_id,
            robin_highscores::test_support::verified_run(100, 175, 321),
        )
        .await;
    assert_eq!(
        public_status(&rig, &accepted.submission_id).await,
        PublicSubmissionStateV1::Verified {
            run_id: run_id.clone()
        }
    );

    let detail_response = rig
        .send(empty_request(
            Method::GET,
            &format!("/api/v1/runs/{run_id}"),
            Ipv4Addr::LOCALHOST,
        ))
        .await;
    assert_eq!(detail_response.status(), StatusCode::OK);
    assert_dynamic_headers(&detail_response);
    let detail: RunDetailV2 = json_body(detail_response).await;
    detail.validate().unwrap();
    assert_eq!(detail.board_id.as_str(), BOARD_ID);
    assert_eq!(detail.metrics.original_score_delta, 75);
    assert_eq!(detail.starting_campaign_score, 100);
    assert_eq!(detail.uploader.as_ref().unwrap().username, "Robin");
    assert_eq!(detail.viewer.runtime_build, "0123456789ab");
    assert_eq!(
        detail.replay.artifact.sha256,
        Digest32::digest_bytes(&replay)
    );
    assert!(
        detail
            .achievements
            .iter()
            .any(|achievement| achievement.verified.is_awarded())
    );

    let download = rig
        .send(empty_request(
            Method::GET,
            &format!("/api/v1/runs/{run_id}/replay"),
            Ipv4Addr::LOCALHOST,
        ))
        .await;
    assert_eq!(download.status(), StatusCode::OK);
    assert_eq!(
        download.headers().get(CONTENT_TYPE).unwrap(),
        RANKED_REPLAY_MEDIA_TYPE_V1
    );
    assert_eq!(
        download
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .as_ref(),
        replay.as_slice()
    );

    let board: LeaderboardPageV2 = json_body(
        rig.send(empty_request(
            Method::GET,
            &leaderboard_uri(BOARD_ID, "fastest_success", 10, None),
            Ipv4Addr::LOCALHOST,
        ))
        .await,
    )
    .await;
    board.validate().unwrap();
    assert_eq!(board.entries.len(), 1);
    assert_eq!(
        board.entries[0].metric_value,
        BoardMetricValueV2::FastestSuccess {
            active_simulation_ticks: 321
        }
    );
}

#[tokio::test]
async fn board_mission_metric_and_identity_rejections_never_consume_the_challenge() {
    let rig = TestRig::new().await;
    let owner = SigningKey::from_bytes(&[0x21; 32]);
    let unregistered = SigningKey::from_bytes(&[0x22; 32]);
    rig.rename(&owner, "Marian", Ipv4Addr::new(127, 0, 0, 3))
        .await;
    let replay = compact_replay_fixture("rejections");

    let challenge = rig.upload_challenge(&owner, Ipv4Addr::LOCALHOST).await;
    let base = submission(
        &owner,
        challenge.clone(),
        &replay,
        ParticipantPublicDisclosureV1::NamedProfile,
    );
    let mut cases = Vec::new();
    let mut unknown_board = base.clone();
    unknown_board.board_id = OpaqueId::new("full-any").unwrap();
    cases.push(unknown_board);
    let mut unknown_mission = base.clone();
    unknown_mission.mission_id = "H01_Lin_VL".to_owned();
    cases.push(unknown_mission);
    let mut unoffered_metric = base.clone();
    unoffered_metric.board_id = OpaqueId::new(SCORE_ONLY_BOARD_ID).unwrap();
    cases.push(unoffered_metric);
    for case in cases {
        let response = rig
            .send(multipart_request(&signed_submission(&owner, case), &replay))
            .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    let unregistered_challenge = rig
        .upload_challenge(&unregistered, Ipv4Addr::LOCALHOST)
        .await;
    let response = rig
        .send(multipart_request(
            &signed_submission(
                &unregistered,
                submission(
                    &unregistered,
                    unregistered_challenge,
                    &replay,
                    ParticipantPublicDisclosureV1::Anonymous,
                ),
            ),
            &replay,
        ))
        .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(bad_request_message(response).await.contains("username"));

    assert_eq!(
        rig.count("SELECT COUNT(*) FROM upload_challenges WHERE consumed_at_ms IS NOT NULL")
            .await,
        1,
        "only the username update consumed a challenge"
    );
    assert_eq!(rig.count("SELECT COUNT(*) FROM submissions").await, 0);
    // The untouched challenge still redeems the valid submission.
    let response = rig
        .send(multipart_request(&signed_submission(&owner, base), &replay))
        .await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
}

#[tokio::test]
async fn signature_challenge_binding_expiry_and_reuse_are_enforced() {
    let rig = TestRig::new().await;
    let owner = SigningKey::from_bytes(&[0x31; 32]);
    let other = SigningKey::from_bytes(&[0x32; 32]);
    rig.rename(&owner, "Tuck", Ipv4Addr::new(127, 0, 0, 4))
        .await;
    rig.rename(&other, "John", Ipv4Addr::new(127, 0, 0, 5))
        .await;
    let replay = compact_replay_fixture("signatures");

    let challenge = rig.upload_challenge(&owner, Ipv4Addr::LOCALHOST).await;
    let mut forged = signed_submission(
        &owner,
        submission(
            &owner,
            challenge.clone(),
            &replay,
            ParticipantPublicDisclosureV1::NamedProfile,
        ),
    );
    forged.submission.public_disclosure = ParticipantPublicDisclosureV1::Anonymous;
    assert_eq!(
        rig.send(multipart_request(&forged, &replay)).await.status(),
        StatusCode::UNAUTHORIZED
    );

    // A challenge issued to `owner` cannot authorize `other`'s signed upload.
    let stolen = signed_submission(
        &other,
        submission(
            &other,
            challenge.clone(),
            &replay,
            ParticipantPublicDisclosureV1::NamedProfile,
        ),
    );
    assert_eq!(
        rig.send(multipart_request(&stolen, &replay)).await.status(),
        StatusCode::CONFLICT
    );
    let mut wrong_nonce = challenge.clone();
    wrong_nonce.upload_challenge_nonce = robin_run_protocol::ChallengeNonce32::from_bytes([9; 32]);
    let substituted = signed_submission(
        &owner,
        submission(
            &owner,
            wrong_nonce,
            &replay,
            ParticipantPublicDisclosureV1::NamedProfile,
        ),
    );
    assert_eq!(
        rig.send(multipart_request(&substituted, &replay))
            .await
            .status(),
        StatusCode::CONFLICT
    );

    let signed = signed_submission(
        &owner,
        submission(
            &owner,
            challenge.clone(),
            &replay,
            ParticipantPublicDisclosureV1::NamedProfile,
        ),
    );
    let first = rig.send(multipart_request(&signed, &replay)).await;
    assert_eq!(first.status(), StatusCode::ACCEPTED);
    let first: SubmissionAcceptedV1 = json_body(first).await;
    // An exact retry is idempotent and returns the same submission.
    let retry = rig.send(multipart_request(&signed, &replay)).await;
    assert_eq!(retry.status(), StatusCode::ACCEPTED);
    let retry: SubmissionAcceptedV1 = json_body(retry).await;
    assert_eq!(retry.submission_id, first.submission_id);
    // Reusing the consumed challenge for different content is refused.
    let other_replay = compact_replay_fixture("signatures-second");
    let reuse = signed_submission(
        &owner,
        submission(
            &owner,
            challenge,
            &other_replay,
            ParticipantPublicDisclosureV1::NamedProfile,
        ),
    );
    assert_eq!(
        rig.send(multipart_request(&reuse, &other_replay))
            .await
            .status(),
        StatusCode::CONFLICT
    );
    assert_eq!(rig.count("SELECT COUNT(*) FROM submissions").await, 1);

    let mut expired = rig.upload_challenge(&owner, Ipv4Addr::LOCALHOST).await;
    expired.expires_at_unix_ms = 1;
    let expired = signed_submission(
        &owner,
        submission(
            &owner,
            expired,
            &other_replay,
            ParticipantPublicDisclosureV1::NamedProfile,
        ),
    );
    assert_eq!(
        rig.send(multipart_request(&expired, &other_replay))
            .await
            .status(),
        StatusCode::CONFLICT
    );
}

#[tokio::test]
async fn a_pending_or_verified_replay_cannot_be_resubmitted_by_anyone() {
    let rig = TestRig::new().await;
    let owner = SigningKey::from_bytes(&[0x41; 32]);
    let copier = SigningKey::from_bytes(&[0x42; 32]);
    rig.rename(&owner, "Owner", Ipv4Addr::new(127, 0, 0, 6))
        .await;
    rig.rename(&copier, "Copier", Ipv4Addr::new(127, 0, 0, 7))
        .await;
    let replay = compact_replay_fixture("duplicate");
    let accepted = rig
        .submit(&owner, &replay, ParticipantPublicDisclosureV1::NamedProfile)
        .await;

    let copy = |key: SigningKey| {
        let rig = &rig;
        let replay = replay.clone();
        async move {
            let challenge = rig.upload_challenge(&key, Ipv4Addr::LOCALHOST).await;
            rig.send(multipart_request(
                &signed_submission(
                    &key,
                    submission(
                        &key,
                        challenge,
                        &replay,
                        ParticipantPublicDisclosureV1::NamedProfile,
                    ),
                ),
                &replay,
            ))
            .await
            .status()
        }
    };
    assert_eq!(copy(copier.clone()).await, StatusCode::CONFLICT);
    rig.accept_as_worker(
        &accepted.submission_id,
        robin_highscores::test_support::verified_run(0, 10, 10),
    )
    .await;
    assert_eq!(copy(copier).await, StatusCode::CONFLICT);
    assert_eq!(copy(owner).await, StatusCode::CONFLICT);
    assert_eq!(rig.count("SELECT COUNT(*) FROM submissions").await, 1);
}

#[tokio::test]
async fn multipart_shape_and_compact_transport_fail_before_reservation() {
    let rig = TestRig::new().await;
    let owner = SigningKey::from_bytes(&[0x51; 32]);
    rig.rename(&owner, "Shape", Ipv4Addr::new(127, 0, 0, 8))
        .await;
    let replay = compact_replay_fixture("shape");
    let challenge = rig.upload_challenge(&owner, Ipv4Addr::LOCALHOST).await;
    let signed = signed_submission(
        &owner,
        submission(
            &owner,
            challenge,
            &replay,
            ParticipantPublicDisclosureV1::NamedProfile,
        ),
    );
    let metadata = serde_json::to_vec(&signed).unwrap();
    let jsonl = b"{\"header\":{}}\n".to_vec();
    let jsonl_signed = {
        let challenge = rig.upload_challenge(&owner, Ipv4Addr::LOCALHOST).await;
        signed_submission(
            &owner,
            submission(
                &owner,
                challenge,
                &jsonl,
                ParticipantPublicDisclosureV1::NamedProfile,
            ),
        )
    };
    let jsonl_metadata = serde_json::to_vec(&jsonl_signed).unwrap();
    let mut duplicate_keys = metadata.clone();
    duplicate_keys.pop();
    duplicate_keys.extend_from_slice(b",\"schema_version\":2}");
    let cases: Vec<Vec<Part<'_>>> = vec![
        vec![("submission", "application/json", &metadata)],
        vec![
            ("replay", RANKED_REPLAY_MEDIA_TYPE_V1, &replay),
            ("submission", "application/json", &metadata),
        ],
        vec![
            ("submission", "application/json", &metadata),
            ("replay", "application/octet-stream", &replay),
        ],
        vec![
            ("submission", "application/json", &metadata),
            ("replay", RANKED_REPLAY_MEDIA_TYPE_V1, &replay),
            ("starting_campaign", "application/octet-stream", b"campaign"),
        ],
        vec![
            ("submission", "application/json", &metadata),
            ("replay", RANKED_REPLAY_MEDIA_TYPE_V1, b"truncated"),
        ],
        vec![
            ("submission", "application/json", &duplicate_keys),
            ("replay", RANKED_REPLAY_MEDIA_TYPE_V1, &replay),
        ],
        vec![
            ("submission", "application/json", &jsonl_metadata),
            ("replay", RANKED_REPLAY_MEDIA_TYPE_V1, &jsonl),
        ],
    ];
    for parts in cases {
        let response = rig.send(multipart_parts(&parts)).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
    assert_eq!(
        rig.count("SELECT COUNT(*) FROM submission_upload_reservations")
            .await,
        0
    );
    assert_eq!(rig.count("SELECT COUNT(*) FROM replay_objects").await, 0);
    let response = rig.send(multipart_request(&signed, &replay)).await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
}

#[tokio::test]
async fn rejected_submission_exposes_code_only_to_its_owner() {
    let rig = TestRig::new().await;
    let owner = SigningKey::from_bytes(&[0x61; 32]);
    rig.rename(&owner, "Rejected", Ipv4Addr::new(127, 0, 0, 9))
        .await;
    let replay = compact_replay_fixture("rejected");
    let accepted = rig
        .submit(&owner, &replay, ParticipantPublicDisclosureV1::Anonymous)
        .await;
    let job = rig
        .database
        .lease_next("rig-worker", Duration::from_secs(60))
        .await
        .unwrap()
        .unwrap();
    rig.database
        .reject_job(
            &job.submission_id,
            "rig-worker",
            "state_hash_mismatch",
            Some("frame_42"),
        )
        .await
        .unwrap();
    assert_eq!(
        public_status(&rig, &accepted.submission_id).await,
        PublicSubmissionStateV1::Rejected
    );
    let response = private_status(&rig, &owner, &accepted.submission_id).await;
    let status: SubmissionOwnerStatusResponseV1 = json_body(response).await;
    assert!(matches!(
        status.state,
        SubmissionLifecycleV1::Rejected {
            code: robin_run_protocol::VerificationRejectionCodeV1::StateHashMismatch,
            ..
        }
    ));
}
