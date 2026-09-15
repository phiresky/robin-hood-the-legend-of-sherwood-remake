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

fn owner_status_request(
    key: &SigningKey,
    submission_id: &OpaqueId,
    signed_at_unix_ms: u64,
) -> SignedSubmissionOwnerStatusRequestV2 {
    signed(
        key,
        SubmissionOwnerStatusRequestV2 {
            schema_version: SCHEMA_VERSION_V2,
            public_key: protocol_public_key(key),
            signed_at_unix_ms,
            submission_id: submission_id.clone(),
        },
    )
}

async fn send_owner_status(
    rig: &TestRig,
    path_submission_id: &OpaqueId,
    request: &SignedSubmissionOwnerStatusRequestV2,
) -> axum::response::Response {
    rig.send(json_request(
        Method::POST,
        &format!("/api/v1/submissions/{path_submission_id}/private-status"),
        request,
        Ipv4Addr::LOCALHOST,
    ))
    .await
}

async fn private_status(
    rig: &TestRig,
    key: &SigningKey,
    submission_id: &OpaqueId,
) -> axum::response::Response {
    send_owner_status(
        rig,
        submission_id,
        &owner_status_request(key, submission_id, now_ms()),
    )
    .await
}

async fn error_code(response: axum::response::Response) -> String {
    let body: serde_json::Value = json_body(response).await;
    body["error"]["code"].as_str().unwrap().to_owned()
}

#[tokio::test]
async fn signed_submission_queue_and_verified_publication_cross_the_router() {
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
    let request = owner_status_request(&owner, &accepted.submission_id, now_ms());
    let owner_response = send_owner_status(&rig, &accepted.submission_id, &request).await;
    assert_eq!(owner_response.status(), StatusCode::OK);
    assert_dynamic_headers(&owner_response);
    let owner_status: SubmissionOwnerStatusResponseV2 = json_body(owner_response).await;
    owner_status.validate_against_request(&request).unwrap();
    assert_eq!(owner_status.state, SubmissionLifecycleV1::Queued);
    // Replaying the captured request within its window returns the same
    // key-scoped view; nothing is consumed.
    assert_eq!(
        send_owner_status(&rig, &accepted.submission_id, &request)
            .await
            .status(),
        StatusCode::OK
    );

    // A stranger, an unknown submission and a path that differs from the
    // signed submission ID are indistinguishable.
    let stranger = SigningKey::from_bytes(&[0x12; 32]);
    let unknown = OpaqueId::new("018f0000-0000-7000-8000-000000000000").unwrap();
    for response in [
        private_status(&rig, &stranger, &accepted.submission_id).await,
        private_status(&rig, &owner, &unknown).await,
        send_owner_status(&rig, &unknown, &request).await,
    ] {
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            error_code(response).await,
            "signature_authentication_failed"
        );
    }

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
async fn removed_challenge_routes_and_v2_submissions_are_refused() {
    let rig = TestRig::new().await;
    let owner = SigningKey::from_bytes(&[0x13; 32]);
    rig.rename(&owner, "Legacy", Ipv4Addr::new(127, 0, 0, 12))
        .await;
    for removed in [
        "/api/v1/upload-challenges",
        "/api/v1/submission-owner-status-challenges",
        "/api/v1/username-challenges",
        "/api/v1/deletion-challenges",
    ] {
        assert_eq!(
            rig.send(json_request(
                Method::POST,
                removed,
                &serde_json::json!({ "schema_version": 2 }),
                Ipv4Addr::LOCALHOST
            ))
            .await
            .status(),
            StatusCode::NOT_FOUND,
            "{removed}"
        );
    }
    // A V2-shaped signed submission (schema 2, no signed_at) is malformed.
    let replay = compact_replay_fixture("legacy-v2");
    let mut legacy = serde_json::to_value(signed_submission(
        &owner,
        submission(&owner, &replay, ParticipantPublicDisclosureV1::NamedProfile),
    ))
    .unwrap();
    legacy["request"]["schema_version"] = serde_json::json!(2);
    let legacy = serde_json::to_vec(&legacy).unwrap();
    let response = rig
        .send(multipart_parts(&[
            ("submission", "application/json", &legacy),
            ("replay", RANKED_REPLAY_MEDIA_TYPE_V1, &replay),
        ]))
        .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(rig.count("SELECT COUNT(*) FROM submissions").await, 0);
}

#[tokio::test]
async fn board_mission_metric_and_identity_rejections_reserve_nothing() {
    let rig = TestRig::new().await;
    let owner = SigningKey::from_bytes(&[0x21; 32]);
    let unregistered = SigningKey::from_bytes(&[0x22; 32]);
    rig.rename(&owner, "Marian", Ipv4Addr::new(127, 0, 0, 3))
        .await;
    let replay = compact_replay_fixture("rejections");

    let base = submission(&owner, &replay, ParticipantPublicDisclosureV1::NamedProfile);
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

    let response = rig
        .send(multipart_request(
            &signed_submission(
                &unregistered,
                submission(
                    &unregistered,
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
        rig.count("SELECT COUNT(*) FROM submission_upload_reservations")
            .await,
        0
    );
    assert_eq!(rig.count("SELECT COUNT(*) FROM submissions").await, 0);
    let response = rig
        .send(multipart_request(&signed_submission(&owner, base), &replay))
        .await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
}

#[tokio::test]
async fn signature_freshness_operation_binding_and_exact_retry_are_enforced() {
    let rig = TestRig::new().await;
    let owner = SigningKey::from_bytes(&[0x31; 32]);
    let other = SigningKey::from_bytes(&[0x32; 32]);
    rig.rename(&owner, "Tuck", Ipv4Addr::new(127, 0, 0, 4))
        .await;
    rig.rename(&other, "John", Ipv4Addr::new(127, 0, 0, 5))
        .await;
    let replay = compact_replay_fixture("signatures");
    let window = rig.config.signed_requests.window();

    // Changing a signed field after signing.
    let mut forged = signed_submission(
        &owner,
        submission(&owner, &replay, ParticipantPublicDisclosureV1::NamedProfile),
    );
    forged.request.public_disclosure = ParticipantPublicDisclosureV1::Anonymous;
    assert_eq!(
        rig.send(multipart_request(&forged, &replay)).await.status(),
        StatusCode::UNAUTHORIZED
    );

    // Another key's signature over the owner's claim.
    let mut stolen = signed_submission(
        &owner,
        submission(&owner, &replay, ParticipantPublicDisclosureV1::NamedProfile),
    );
    stolen.signature = signed_submission(&other, stolen.request.clone()).signature;
    assert_eq!(
        rig.send(multipart_request(&stolen, &replay)).await.status(),
        StatusCode::UNAUTHORIZED
    );

    // Stale and future timestamps are rejected with their own code before
    // anything is reserved.
    let now = now_ms();
    for signed_at in [
        now - window.max_age_ms - 60_000,
        now + window.max_future_skew_ms + 60_000,
    ] {
        let mut claim = submission(&owner, &replay, ParticipantPublicDisclosureV1::NamedProfile);
        claim.signed_at_unix_ms = signed_at;
        let response = rig
            .send(multipart_request(
                &signed_submission(&owner, claim),
                &replay,
            ))
            .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{signed_at}");
        assert_eq!(error_code(response).await, "signed_request_not_fresh");
    }

    // A signature over the same key and timestamp for a different operation
    // (a username update) never authorizes a submission.
    let claim = submission(&owner, &replay, ParticipantPublicDisclosureV1::NamedProfile);
    let foreign = username_update(&owner, "Tuck", claim.signed_at_unix_ms);
    let cross_operation = SignedSubmissionV3 {
        schema_version: SCHEMA_VERSION_V2,
        request: claim.clone(),
        algorithm: SignatureAlgorithmV1::Ed25519,
        signature: foreign.signature,
    };
    assert_eq!(
        rig.send(multipart_request(&cross_operation, &replay))
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        rig.count("SELECT COUNT(*) FROM submission_upload_reservations")
            .await,
        0
    );

    let signed = signed_submission(&owner, claim);
    let first = rig.send(multipart_request(&signed, &replay)).await;
    assert_eq!(first.status(), StatusCode::ACCEPTED);
    let first: SubmissionAcceptedV1 = json_body(first).await;
    // An exact retry is idempotent and returns the same submission.
    let retry = rig.send(multipart_request(&signed, &replay)).await;
    assert_eq!(retry.status(), StatusCode::ACCEPTED);
    let retry: SubmissionAcceptedV1 = json_body(retry).await;
    assert_eq!(retry.submission_id, first.submission_id);
    // A differently signed request for the same replay is a duplicate, even
    // from the same uploader.
    let resigned = signed_submission(
        &owner,
        submission(&owner, &replay, ParticipantPublicDisclosureV1::Anonymous),
    );
    assert_eq!(
        rig.send(multipart_request(&resigned, &replay))
            .await
            .status(),
        StatusCode::CONFLICT
    );
    assert_eq!(rig.count("SELECT COUNT(*) FROM submissions").await, 1);
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
            rig.send(multipart_request(
                &signed_submission(
                    &key,
                    submission(&key, &replay, ParticipantPublicDisclosureV1::NamedProfile),
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
    let signed = signed_submission(
        &owner,
        submission(&owner, &replay, ParticipantPublicDisclosureV1::NamedProfile),
    );
    let metadata = serde_json::to_vec(&signed).unwrap();
    let jsonl = b"{\"header\":{}}\n".to_vec();
    let jsonl_signed = signed_submission(
        &owner,
        submission(&owner, &jsonl, ParticipantPublicDisclosureV1::NamedProfile),
    );
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
async fn oversized_declared_content_length_is_refused_before_the_body_is_read() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    let rig = TestRig::new().await;
    let limit = rig.config.max_replay_bytes
        + u64::try_from(rig.config.max_metadata_bytes).unwrap()
        + 1024 * 1024;
    let polled = Arc::new(AtomicBool::new(false));
    let body_polled = polled.clone();
    let body = Body::from_stream(futures_util::stream::poll_fn(move |_| {
        body_polled.store(true, Ordering::SeqCst);
        std::task::Poll::Ready(Some(Ok::<_, std::io::Error>(bytes::Bytes::from_static(
            b"--robin-router-e2e-boundary\r\n",
        ))))
    }));
    let request = Request::builder()
        .method(Method::POST)
        .uri("/api/v1/submissions")
        .header(
            CONTENT_TYPE,
            "multipart/form-data; boundary=robin-router-e2e-boundary",
        )
        .header(axum::http::header::CONTENT_LENGTH, (limit + 1).to_string())
        .extension(peer(Ipv4Addr::LOCALHOST))
        .body(body)
        .unwrap();
    let response = rig.send(request).await;
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert_dynamic_headers(&response);
    assert!(
        !polled.load(Ordering::SeqCst),
        "the oversized body was read before rejection"
    );
    assert_eq!(
        rig.database
            .active_maintenance_write_lease_count()
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn submissions_are_rate_limited_per_uploader_key_after_verification() {
    let rig = TestRig::with_config(|config| config.submissions_per_hour_per_key = 2).await;
    let owner = SigningKey::from_bytes(&[0x71; 32]);
    let other = SigningKey::from_bytes(&[0x72; 32]);
    rig.rename(&owner, "Limited", Ipv4Addr::new(127, 0, 0, 10))
        .await;
    rig.rename(&other, "Unlimited", Ipv4Addr::new(127, 0, 0, 11))
        .await;

    // Requests with a bad signature never spend the key's budget.
    let replay = compact_replay_fixture("per-key-forged");
    let mut forged = signed_submission(
        &owner,
        submission(&owner, &replay, ParticipantPublicDisclosureV1::NamedProfile),
    );
    forged.request.mission_id = "changed".to_owned();
    for _ in 0..3 {
        assert_eq!(
            rig.send(multipart_request(&forged, &replay)).await.status(),
            StatusCode::UNAUTHORIZED
        );
    }

    for label in ["per-key-1", "per-key-2"] {
        rig.submit(
            &owner,
            &compact_replay_fixture(label),
            ParticipantPublicDisclosureV1::NamedProfile,
        )
        .await;
    }
    let replay = compact_replay_fixture("per-key-3");
    let response = rig
        .send(multipart_request(
            &signed_submission(
                &owner,
                submission(&owner, &replay, ParticipantPublicDisclosureV1::NamedProfile),
            ),
            &replay,
        ))
        .await;
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(response.headers().contains_key("retry-after"));
    assert_eq!(error_code(response).await, "rate_limited");
    // The same address with a different key still has budget.
    rig.submit(&other, &replay, ParticipantPublicDisclosureV1::NamedProfile)
        .await;
    assert_eq!(rig.count("SELECT COUNT(*) FROM submissions").await, 3);
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
    let stale = owner_status_request(
        &owner,
        &accepted.submission_id,
        now_ms() - rig.config.signed_requests.window().max_age_ms - 60_000,
    );
    let response = send_owner_status(&rig, &accepted.submission_id, &stale).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(error_code(response).await, "signed_request_not_fresh");

    let response = private_status(&rig, &owner, &accepted.submission_id).await;
    let status: SubmissionOwnerStatusResponseV2 = json_body(response).await;
    assert!(matches!(
        status.state,
        SubmissionLifecycleV1::Rejected {
            code: robin_run_protocol::VerificationRejectionCodeV1::StateHashMismatch,
            ..
        }
    ));
}
