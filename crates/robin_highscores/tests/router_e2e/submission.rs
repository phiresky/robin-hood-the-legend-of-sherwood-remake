use crate::support::*;

#[tokio::test]
async fn recorded_replay_upload_does_not_require_a_preflight_or_saved_admission() {
    let rig = TestRig::new().await;
    let owner = SigningKey::from_bytes(&[72; 32]);
    rig.rename(&owner, "Replay Uploader", Ipv4Addr::LOCALHOST)
        .await;
    let replay = compact_replay_fixture("recorded-upload");
    let mut request = rig.offer_request(&owner, 72);
    let genesis = &mut request.session_genesis;
    genesis.host_signature = None;
    genesis.claim.fresh_run_preflight_grant = None;
    genesis.claim.ranked_session.recorded_replay = Some(replay_artifact(&replay));
    genesis.claim.replay_session_id = Digest32::digest_bytes(&replay);
    genesis
        .claim
        .ranked_session
        .prepared_inputs_projection_sha256 = None;
    genesis
        .claim
        .ranked_session
        .prepared_mission_inputs_seal_sha256 = None;
    request.validate().unwrap();
    let response = rig
        .app
        .clone()
        .oneshot(json_request(
            Method::POST,
            "/api/v1/submission-offers",
            &request,
            Ipv4Addr::LOCALHOST,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let offer: SubmissionOfferV1 = json_body(response).await;
    let signed = signed_submission(&owner, offer, &replay, &rig.starting_campaign);
    let response = rig
        .app
        .clone()
        .oneshot(multipart_request(&signed, &replay, &rig.starting_campaign))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let accepted: SubmissionAcceptedV1 = json_body(response).await;
    assert_eq!(
        rig.owner_status(&accepted, &owner).await.state,
        SubmissionLifecycleV1::Queued
    );
}

#[tokio::test]
async fn signed_rename_offer_upload_status_and_verified_publication_cross_the_real_router() {
    let rig = TestRig::new().await;
    let owner = SigningKey::from_bytes(&[7; 32]);
    let owner_key = protocol_public_key(&owner);

    assert_eq!(
        rig.rename(&owner, "Robin", Ipv4Addr::new(127, 0, 0, 7))
            .await
            .username,
        "Robin"
    );
    assert_eq!(
        rig.rename(&owner, "Robin of Locksley", Ipv4Addr::new(127, 0, 0, 8))
            .await
            .username,
        "Robin of Locksley"
    );

    let history = sqlx::query(
        "SELECT previous_username, new_username FROM username_history \
         WHERE public_key = ? ORDER BY generation",
    )
    .bind(owner_key.as_bytes().as_slice())
    .fetch_all(rig.database.fixture_pool())
    .await
    .unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(
        history[0].get::<Option<String>, _>("previous_username"),
        None
    );
    assert_eq!(history[0].get::<String, _>("new_username"), "Robin");
    assert_eq!(history[1].get::<String, _>("previous_username"), "Robin");
    assert_eq!(
        history[1].get::<String, _>("new_username"),
        "Robin of Locksley"
    );

    let profile_response = rig
        .app
        .clone()
        .oneshot(empty_request(
            Method::GET,
            &format!("/api/v1/players/{owner_key}"),
            Ipv4Addr::LOCALHOST,
        ))
        .await
        .unwrap();
    assert_eq!(profile_response.status(), StatusCode::OK);
    assert_dynamic_headers(&profile_response);
    let profile: PlayerProfileV1 = json_body(profile_response).await;
    assert_eq!(profile.username, "Robin of Locksley");

    let metadata_response = rig
        .app
        .clone()
        .oneshot(empty_request(
            Method::GET,
            "/api/v1/leaderboard-metadata",
            Ipv4Addr::LOCALHOST,
        ))
        .await
        .unwrap();
    assert_eq!(metadata_response.status(), StatusCode::OK);
    assert_dynamic_headers(&metadata_response);
    let metadata: LeaderboardMetadataV1 = json_body(metadata_response).await;
    metadata.validate().unwrap();
    assert_eq!(metadata.missions.len(), 1);
    assert_eq!(metadata.missions[0].mission_id, MISSION_ID);

    let accepted = rig.submit(&owner, 1).await;
    let queued = rig.owner_status(&accepted, &owner).await;
    queued.validate().unwrap();
    assert_eq!(queued.state, SubmissionLifecycleV1::Queued);

    let run_id = rig.publish(&accepted, 12_345, 21).await;
    let private_participant_instance_id = Digest32::from_bytes([1; 32]);
    let private_completion_digest = Digest32::from_bytes([0xc7; 32]);
    sqlx::query("UPDATE verified_runs SET campaign_complete_evidence_sha256 = ? WHERE id = ?")
        .bind(private_completion_digest.as_bytes().as_slice())
        .bind(run_id.as_str())
        .execute(rig.database.fixture_pool())
        .await
        .unwrap();
    let corrupted_evidence = sqlx::query(
        "UPDATE verified_run_achievements \
         SET evidence_json = ? WHERE run_id = ?",
    )
    .bind(r#"{"PRIVATE_ACHIEVEMENT_EVIDENCE_SENTINEL":1.5}"#)
    .bind(run_id.as_str())
    .execute(rig.database.fixture_pool())
    .await
    .unwrap();
    assert!(corrupted_evidence.rows_affected() > 0);
    let public_projection_storage = sqlx::query(
        "SELECT public_verification_request_json, public_verification_result_json, \
                public_projection_binding_json FROM verified_runs WHERE id = ?",
    )
    .bind(run_id.as_str())
    .fetch_one(rig.database.fixture_pool())
    .await
    .unwrap();
    for column in [
        "public_verification_request_json",
        "public_verification_result_json",
        "public_projection_binding_json",
    ] {
        let stored: String = public_projection_storage.get(column);
        assert_public_json_omits_private_participant_id(
            stored.as_bytes(),
            private_participant_instance_id,
        );
    }
    let accepted_status = rig.owner_status(&accepted, &owner).await;
    accepted_status.validate().unwrap();
    assert_eq!(
        accepted_status.state,
        SubmissionLifecycleV1::Accepted {
            run_id: run_id.clone(),
            campaign_chain_receipt: None,
        }
    );

    let attacker = SigningKey::from_bytes(&[8; 32]);
    let missing_submission = OpaqueId::new("00000000-0000-7000-8000-000000000001").unwrap();
    let wrong_key_envelope = rig
        .owner_status_envelope(&accepted.submission_id, &attacker)
        .await;
    let missing_envelope = rig
        .owner_status_envelope(&missing_submission, &attacker)
        .await;
    let wrong_key_response = rig
        .app
        .clone()
        .oneshot(json_request(
            Method::POST,
            &format!(
                "/api/v1/submissions/{}/private-status",
                accepted.submission_id
            ),
            &wrong_key_envelope,
            Ipv4Addr::LOCALHOST,
        ))
        .await
        .unwrap();
    let missing_response = rig
        .app
        .clone()
        .oneshot(json_request(
            Method::POST,
            &format!("/api/v1/submissions/{missing_submission}/private-status"),
            &missing_envelope,
            Ipv4Addr::LOCALHOST,
        ))
        .await
        .unwrap();
    assert_eq!(wrong_key_response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(missing_response.status(), StatusCode::UNAUTHORIZED);
    let wrong_key_body = wrong_key_response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    let missing_body = missing_response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    assert_eq!(wrong_key_body, missing_body);

    let replayed_response = rig
        .app
        .clone()
        .oneshot(json_request(
            Method::POST,
            &format!(
                "/api/v1/submissions/{}/private-status",
                accepted.submission_id
            ),
            &wrong_key_envelope,
            Ipv4Addr::LOCALHOST,
        ))
        .await
        .unwrap();
    assert_eq!(replayed_response.status(), StatusCode::UNAUTHORIZED);

    let detail_response = rig
        .app
        .clone()
        .oneshot(empty_request(
            Method::GET,
            &format!("/api/v1/runs/{run_id}"),
            Ipv4Addr::LOCALHOST,
        ))
        .await
        .unwrap();
    assert_eq!(detail_response.status(), StatusCode::OK);
    assert_dynamic_headers(&detail_response);
    let detail_body = detail_response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    assert_public_json_omits_private_participant_id(&detail_body, private_participant_instance_id);
    assert!(
        !String::from_utf8_lossy(&detail_body).contains("PRIVATE_ACHIEVEMENT_EVIDENCE_SENTINEL")
    );
    assert!(
        !String::from_utf8_lossy(&detail_body)
            .contains(&hex::encode(private_completion_digest.as_bytes()))
    );
    let detail: RunDetailV1 = serde_json::from_slice(&detail_body).unwrap();
    detail
        .validate_against_ruleset(&rig.published_ruleset, None)
        .unwrap();
    assert_eq!(detail.run_id, run_id);
    assert_eq!(detail.metrics.original_score_delta, 12_345);
    assert_eq!(detail.named_participants[0].username, "Robin of Locksley");
    assert!(detail.verification_proof.is_some());
    assert_public_projection_is_redacted(&detail);

    let replay_response = rig
        .app
        .clone()
        .oneshot(empty_request(
            Method::GET,
            &format!("/api/v1/runs/{run_id}/replay"),
            Ipv4Addr::LOCALHOST,
        ))
        .await
        .unwrap();
    assert_eq!(replay_response.status(), StatusCode::OK);
    assert_dynamic_headers(&replay_response);
    assert_eq!(
        replay_response.headers().get(CONTENT_TYPE).unwrap(),
        RANKED_REPLAY_MEDIA_TYPE_V1
    );
    let replay = replay_response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    assert_eq!(replay, compact_replay_fixture("run-1"));

    let missing_response = rig
        .app
        .clone()
        .oneshot(empty_request(
            Method::GET,
            "/api/v1/runs/00000000-0000-7000-8000-000000000000",
            Ipv4Addr::LOCALHOST,
        ))
        .await
        .unwrap();
    assert_eq!(missing_response.status(), StatusCode::NOT_FOUND);
    assert_dynamic_headers(&missing_response);

    let immutable_response = rig
        .app
        .clone()
        .oneshot(empty_request(
            Method::GET,
            &format!("/api/v1/builds/{}", rig.build_sha256),
            Ipv4Addr::LOCALHOST,
        ))
        .await
        .unwrap();
    assert_eq!(immutable_response.status(), StatusCode::OK);
    assert_eq!(
        immutable_response.headers().get(CACHE_CONTROL).unwrap(),
        "public, max-age=31536000, immutable"
    );
    assert_eq!(
        immutable_response
            .headers()
            .get(X_CONTENT_TYPE_OPTIONS)
            .unwrap(),
        "nosniff"
    );
    let immutable_body = immutable_response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    assert_eq!(
        immutable_body.as_ref(),
        rig.build.canonical_bytes().unwrap().as_slice()
    );
    assert_eq!(Digest32::digest_bytes(&immutable_body), rig.build_sha256);
}

#[tokio::test]
async fn truncated_three_part_multipart_fails_before_creating_submission_state() {
    let rig = TestRig::new().await;
    const BOUNDARY: &str = "robin-truncated-boundary";
    let body = format!(
        "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"submission\"\r\nContent-Type: application/json\r\n\r\n{{"
    );
    let response = rig
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/v1/submissions")
                .header(
                    CONTENT_TYPE,
                    format!("multipart/form-data; boundary={BOUNDARY}"),
                )
                .extension(peer(Ipv4Addr::LOCALHOST))
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        rig.database
            .operational_counts()
            .await
            .unwrap()
            .queued_submissions,
        0
    );
}

#[tokio::test]
async fn authenticated_submission_rejects_shape_signature_and_offer_before_reservation() {
    let rig = TestRig::new().await;
    let owner = SigningKey::from_bytes(&[93; 32]);
    rig.rename(&owner, "Authentication Robin", Ipv4Addr::new(127, 0, 9, 3))
        .await;
    let offer = rig
        .issue_offer(&owner, rig.offer_request(&owner, 93), 93)
        .await;
    let replay = compact_replay_fixture("authentication-93");
    let signed = signed_submission(&owner, offer, &replay, &rig.starting_campaign);

    let mut malformed = signed.clone();
    malformed.participant_signatures.clear();
    let mut invalid_signature = signed.clone();
    invalid_signature.participant_signatures[0].signature = sign(
        &SigningKey::from_bytes(&[94; 32]),
        &signed.signing_bytes().unwrap(),
    );
    invalid_signature.validate().unwrap();
    // A legal independent offer mutation preserves document shape, but is not
    // the exact server-issued offer. It must reject as conflict before crypto.
    let mut different_offer = signed.clone();
    different_offer.submission.offer.upload_challenge_nonce =
        robin_run_protocol::ChallengeNonce32::from_bytes([95; 32]);
    different_offer.validate().unwrap();
    for (candidate, status) in [
        (&malformed, StatusCode::BAD_REQUEST),
        (&invalid_signature, StatusCode::UNAUTHORIZED),
        (&different_offer, StatusCode::CONFLICT),
    ] {
        let response = rig
            .app
            .clone()
            .oneshot(metadata_only_multipart_request(candidate))
            .await
            .unwrap();
        assert_eq!(response.status(), status);
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM submission_upload_reservations")
            .fetch_one(rig.database.fixture_pool())
            .await
            .unwrap();
        assert_eq!(count, 0);
    }
}

#[tokio::test]
async fn reserved_upload_failures_abandon_lease_and_exact_retry_finalizes_once() {
    for extra_field in [false, true] {
        let rig = TestRig::new().await;
        let owner = SigningKey::from_bytes(&[91; 32]);
        rig.rename(&owner, "Workflow Robin", Ipv4Addr::new(127, 0, 9, 1))
            .await;
        let offer = rig
            .issue_offer(&owner, rig.offer_request(&owner, 91), 91)
            .await;
        let replay = compact_replay_fixture("workflow-91");
        let signed = signed_submission(&owner, offer, &replay, &rig.starting_campaign);
        let request = multipart_request(&signed, &replay, &rig.starting_campaign);
        let (parts, body) = request.into_parts();
        let mut bytes = body.collect().await.unwrap().to_bytes().to_vec();
        const END: &[u8] = b"\r\n--robin-router-e2e-boundary--\r\n";
        assert!(bytes.ends_with(END));
        if extra_field {
            bytes.truncate(bytes.len() - END.len());
            bytes.extend_from_slice(b"\r\n--robin-router-e2e-boundary\r\nContent-Disposition: form-data; name=\"extra\"\r\n\r\nforbidden\r\n--robin-router-e2e-boundary--\r\n");
        } else {
            // Valid authenticated replay, followed by an interrupted campaign.
            bytes.truncate(bytes.len() - END.len() - 1);
        }
        let response = rig
            .app
            .clone()
            .oneshot(Request::from_parts(parts, Body::from(bytes)))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let state: (String, Option<String>) = sqlx::query_as(
            "SELECT state, lease_token FROM submission_upload_reservations WHERE upload_challenge_id = ?",
        )
        .bind(signed.submission.offer.upload_challenge_id.as_str())
        .fetch_one(rig.database.fixture_pool()).await.unwrap();
        assert_eq!(state, ("abandoned".to_owned(), None));
        let submissions: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM submissions")
            .fetch_one(rig.database.fixture_pool())
            .await
            .unwrap();
        assert_eq!(submissions, 0, "partial artifacts must never be finalized");

        // Model a concurrent exact retry holding the reservation. A valid replay
        // with deliberately wrong campaign bytes must stop at Busy, without
        // invoking campaign ingestion (which would return BadRequest).
        sqlx::query(
            "UPDATE submission_upload_reservations SET state = 'reserved', \
             lease_token = ?, lease_expires_at_ms = reservation_expires_at_ms, \
             abandoned_at_ms = NULL WHERE upload_challenge_id = ?",
        )
        .bind(uuid::Uuid::now_v7().to_string())
        .bind(signed.submission.offer.upload_challenge_id.as_str())
        .execute(rig.database.fixture_pool())
        .await
        .unwrap();
        let busy = rig
            .app
            .clone()
            .oneshot(multipart_request(&signed, &replay, b"not the campaign"))
            .await
            .unwrap();
        assert_eq!(busy.status(), StatusCode::CONFLICT);
        let busy_body: serde_json::Value = json_body(busy).await;
        assert_eq!(busy_body["error"]["code"], "upload_in_progress");
        sqlx::query(
            "UPDATE submission_upload_reservations SET state = 'abandoned', \
             lease_token = NULL, lease_expires_at_ms = NULL, \
             abandoned_at_ms = updated_at_ms WHERE upload_challenge_id = ?",
        )
        .bind(signed.submission.offer.upload_challenge_id.as_str())
        .execute(rig.database.fixture_pool())
        .await
        .unwrap();

        // Cleanup releases the lease, not the immutable signed identity. An
        // exact retry may reuse content-addressed artifacts and finalize once.
        for _ in 0..2 {
            let response = rig
                .app
                .clone()
                .oneshot(multipart_request(&signed, &replay, &rig.starting_campaign))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::ACCEPTED);
        }
        let committed = rig
            .app
            .clone()
            .oneshot(multipart_request(&signed, &replay, b"not the campaign"))
            .await
            .unwrap();
        assert_eq!(
            committed.status(),
            StatusCode::ACCEPTED,
            "committed retries do not ingest campaign bytes"
        );
        let submissions: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM submissions")
            .fetch_one(rig.database.fixture_pool())
            .await
            .unwrap();
        assert_eq!(submissions, 1);
        let persisted = sqlx::query(
            "SELECT s.envelope_json, s.controller_public_key, s.session_genesis_sha256, \
             r.envelope_json AS reserved_envelope, r.controller_public_key AS reserved_controller, \
             r.session_genesis_sha256 AS reserved_genesis \
             FROM submissions s JOIN submission_upload_reservations r \
             ON r.submission_id = s.id",
        )
        .fetch_one(rig.database.fixture_pool())
        .await
        .unwrap();
        assert_eq!(
            persisted.get::<String, _>("envelope_json"),
            serde_json::to_string(&signed).unwrap()
        );
        assert_eq!(
            persisted.get::<String, _>("envelope_json"),
            persisted.get::<String, _>("reserved_envelope")
        );
        for (final_column, reserved_column) in [
            ("controller_public_key", "reserved_controller"),
            ("session_genesis_sha256", "reserved_genesis"),
        ] {
            assert_eq!(
                persisted.get::<Vec<u8>, _>(final_column),
                persisted.get::<Vec<u8>, _>(reserved_column)
            );
        }
    }
}

#[tokio::test]
async fn submission_ingress_accepts_only_the_exact_compact_transport_before_reservation() {
    let rig = TestRig::new().await;
    let owner = SigningKey::from_bytes(&[72; 32]);
    rig.rename(&owner, "Compact Robin", Ipv4Addr::new(127, 0, 2, 0))
        .await;

    // The compact source prefix is provenance: a recording from another
    // commit is admitted when replay and network versions match.
    let valid_replay = String::from_utf8(compact_replay_fixture("run-72"))
        .unwrap()
        .replacen(robin_replay_format::ENGINE_VERSION_HASH, "0123456789ab", 1)
        .into_bytes();
    let valid_offer = rig
        .issue_offer(&owner, rig.offer_request(&owner, 72), 72)
        .await;
    let valid_signed =
        signed_submission(&owner, valid_offer, &valid_replay, &rig.starting_campaign);
    let valid_challenge = valid_signed.submission.offer.upload_challenge_id.as_str();

    let mut encoded = multipart_request(&valid_signed, &valid_replay, &rig.starting_campaign);
    encoded.headers_mut().insert(
        axum::http::header::CONTENT_ENCODING,
        axum::http::HeaderValue::from_static("identity"),
    );
    let encoded_response = rig.app.clone().oneshot(encoded).await.unwrap();
    assert_eq!(encoded_response.status(), StatusCode::BAD_REQUEST);
    let wrong_media_response = rig
        .app
        .clone()
        .oneshot(multipart_request_with_replay_transport(
            &valid_signed,
            &valid_replay,
            &rig.starting_campaign,
            "replay",
            "application/json",
        ))
        .await
        .unwrap();
    assert_eq!(wrong_media_response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        sqlx::query_scalar::<_, Option<i64>>(
            "SELECT consumed_at_ms FROM upload_challenges WHERE id = ?",
        )
        .bind(valid_challenge)
        .fetch_one(rig.database.fixture_pool())
        .await
        .unwrap(),
        None,
        "transport encoding or media-lane rejection consumed the one-use challenge"
    );

    let build_hash = robin_replay_format::ENGINE_VERSION_HASH;
    let invalid_replays = [
        (
            74,
            "local JSONL recorder format",
            b"{\"schema_version\":29,\"not_compact\":true}\n".to_vec(),
        ),
        (
            75,
            "missing compact prefix",
            format!("replay-{build_hash}-YWJj").into_bytes(),
        ),
        (
            76,
            "padded base64 variant",
            format!("rhrec-{build_hash}-YWJj=").into_bytes(),
        ),
        (
            77,
            "non-base64url alphabet",
            format!("rhrec-{build_hash}-YWJ+").into_bytes(),
        ),
        (
            78,
            "malformed source prefix",
            b"rhrec-not-a-commit-YWJj".to_vec(),
        ),
        (
            79,
            "non-canonical base64url tail bits",
            format!("rhrec-{build_hash}-AB").into_bytes(),
        ),
    ];
    for (sequence, label, replay) in invalid_replays {
        let offer = rig
            .issue_offer(&owner, rig.offer_request(&owner, sequence), sequence)
            .await;
        let signed = signed_submission(&owner, offer, &replay, &rig.starting_campaign);
        let challenge_id = signed
            .submission
            .offer
            .upload_challenge_id
            .as_str()
            .to_owned();
        let replay_digest = signed.submission.artifacts.replay.artifact.sha256;
        let response = rig
            .app
            .clone()
            .oneshot(multipart_request(&signed, &replay, &rig.starting_campaign))
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "{label} was admitted"
        );
        assert_eq!(
            sqlx::query_scalar::<_, Option<i64>>(
                "SELECT consumed_at_ms FROM upload_challenges WHERE id = ?",
            )
            .bind(&challenge_id)
            .fetch_one(rig.database.fixture_pool())
            .await
            .unwrap(),
            None,
            "{label} consumed the one-use challenge"
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM submission_upload_reservations WHERE upload_challenge_id = ?",
            )
            .bind(&challenge_id)
            .fetch_one(rig.database.fixture_pool())
            .await
            .unwrap(),
            0,
            "{label} created a durable upload reservation"
        );
        assert!(
            !rig.replay_store
                .path_for_digest(replay_digest.as_bytes())
                .exists(),
            "{label} reached durable replay storage"
        );
    }
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM submissions")
            .fetch_one(rig.database.fixture_pool())
            .await
            .unwrap(),
        0
    );
    assert!(
        rig.database
            .lease_next("lexical-rejection-probe", Duration::from_secs(60))
            .await
            .unwrap()
            .is_none(),
        "a rejected transport created a verifier queue job"
    );

    let accepted_response = rig
        .app
        .clone()
        .oneshot(multipart_request(
            &valid_signed,
            &valid_replay,
            &rig.starting_campaign,
        ))
        .await
        .unwrap();
    assert_eq!(accepted_response.status(), StatusCode::ACCEPTED);
    let accepted: SubmissionAcceptedV1 = json_body(accepted_response).await;
    assert_eq!(accepted.state, SubmissionLifecycleV1::Queued);
    assert!(
        sqlx::query_scalar::<_, Option<i64>>(
            "SELECT consumed_at_ms FROM upload_challenges WHERE id = ?",
        )
        .bind(valid_challenge)
        .fetch_one(rig.database.fixture_pool())
        .await
        .unwrap()
        .is_some()
    );
    assert_eq!(
        tokio::fs::read(
            rig.replay_store
                .path_for_digest(Digest32::digest_bytes(&valid_replay).as_bytes()),
        )
        .await
        .unwrap(),
        valid_replay
    );
    let job = rig
        .database
        .lease_next("valid-compact-worker", Duration::from_secs(60))
        .await
        .unwrap()
        .expect("valid compact replay must create a verifier queue job");
    assert_eq!(job.submission_id, accepted.submission_id.as_str());
}

#[tokio::test]
async fn wrong_body_never_reserves_and_exact_compact_retries_queue_once() {
    let rig = TestRig::new().await;
    let owner = SigningKey::from_bytes(&[73; 32]);
    rig.rename(&owner, "Retrying Robin", Ipv4Addr::new(127, 0, 2, 1))
        .await;
    let sequence = 73;
    let offer = rig
        .issue_offer(&owner, rig.offer_request(&owner, sequence), sequence)
        .await;
    let replay = compact_replay_fixture("retry-73");
    let envelope = SubmissionEnvelopeV1 {
        schema_version: SCHEMA_VERSION_V1,
        replay_session_transcript: replay_session_transcript(&offer),
        offer,
        artifacts: submission_artifacts(&replay, &rig.starting_campaign),
        campaign_aggregation_consent: CampaignAggregationConsentV1::NotAuthorized,
        campaign_continuation_authorization: None,
        requested_metrics: vec![BoardMetricV1::OriginalScore],
    };
    let signed = SignedSubmissionV1 {
        schema_version: SCHEMA_VERSION_V1,
        algorithm: SignatureAlgorithmV1::Ed25519,
        participant_signatures: vec![ParticipantSignatureV1 {
            public_key: protocol_public_key(&owner),
            signature: sign(&owner, &envelope.signing_bytes().unwrap()),
        }],
        submission: envelope,
    };
    signed.validate().unwrap();

    for legacy_field in ["private_replay", "public_replay"] {
        let response = rig
            .app
            .clone()
            .oneshot(multipart_request_with_replay_field(
                &signed,
                &replay,
                &rig.starting_campaign,
                legacy_field,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM submissions")
                .fetch_one(rig.database.fixture_pool())
                .await
                .unwrap(),
            0,
            "legacy multipart field {legacy_field} must fail closed"
        );
    }

    let partial = rig
        .app
        .clone()
        .oneshot(metadata_only_multipart_request(&signed))
        .await
        .unwrap();
    assert_eq!(partial.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM submissions")
            .fetch_one(rig.database.fixture_pool())
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM submission_upload_reservations WHERE upload_challenge_id = ?",
        )
        .bind(signed.submission.offer.upload_challenge_id.as_str())
        .fetch_one(rig.database.fixture_pool())
        .await
        .unwrap(),
        0,
        "a missing replay field must fail before reservation"
    );
    let mut wrong_replay = replay.clone();
    wrong_replay[0] ^= 1;
    let failed = rig
        .app
        .clone()
        .oneshot(multipart_request(
            &signed,
            &wrong_replay,
            &rig.starting_campaign,
        ))
        .await
        .unwrap();
    let failed_status = failed.status();
    let failed_body = failed.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(
        failed_status,
        StatusCode::BAD_REQUEST,
        "wrong-body response: {}",
        String::from_utf8_lossy(&failed_body)
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM submissions")
            .fetch_one(rig.database.fixture_pool())
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM submission_upload_reservations WHERE upload_challenge_id = ?",
        )
        .bind(signed.submission.offer.upload_challenge_id.as_str())
        .fetch_one(rig.database.fixture_pool())
        .await
        .unwrap(),
        0,
        "a digest or lexical mismatch must fail before reservation"
    );
    assert_eq!(
        sqlx::query_scalar::<_, Option<i64>>(
            "SELECT consumed_at_ms FROM upload_challenges WHERE id = ?",
        )
        .bind(signed.submission.offer.upload_challenge_id.as_str())
        .fetch_one(rig.database.fixture_pool())
        .await
        .unwrap(),
        None,
        "a rejected replay body consumed its challenge"
    );
    let replay_digest = signed
        .submission
        .artifacts
        .replay
        .artifact
        .sha256
        .into_bytes();
    assert!(!rig.replay_store.path_for_digest(&replay_digest).exists());
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM replay_objects WHERE sha256 = ?")
            .bind(replay_digest.as_slice())
            .fetch_one(rig.database.fixture_pool())
            .await
            .unwrap(),
        0,
        "a failed stream must not register verifier-visible storage"
    );

    let accepted_response = rig
        .app
        .clone()
        .oneshot(multipart_request(&signed, &replay, &rig.starting_campaign))
        .await
        .unwrap();
    assert_eq!(accepted_response.status(), StatusCode::ACCEPTED);
    let accepted: SubmissionAcceptedV1 = json_body(accepted_response).await;
    let stored_replay = tokio::fs::read(rig.replay_store.path_for_digest(&replay_digest))
        .await
        .unwrap();
    assert_eq!(stored_replay, replay);

    // Even a response-loss retry must still carry the exact compact transport;
    // committed state is not a content-negotiation bypass.
    let completed_retry = rig
        .app
        .clone()
        .oneshot(multipart_request(
            &signed,
            &wrong_replay,
            b"wrong campaign body",
        ))
        .await
        .unwrap();
    assert_eq!(completed_retry.status(), StatusCode::BAD_REQUEST);
    let exact_retry = rig
        .app
        .clone()
        .oneshot(multipart_request(&signed, &replay, &rig.starting_campaign))
        .await
        .unwrap();
    assert_eq!(exact_retry.status(), StatusCode::ACCEPTED);
    let retried: SubmissionAcceptedV1 = json_body(exact_retry).await;
    assert_eq!(retried.submission_id, accepted.submission_id);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM submissions")
            .fetch_one(rig.database.fixture_pool())
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        tokio::fs::read(rig.replay_store.path_for_digest(&replay_digest))
            .await
            .unwrap(),
        replay
    );
}

#[tokio::test]
async fn public_submission_progress_is_minimal_and_disappears_when_deleted() {
    use robin_run_protocol::{PublicSubmissionStateV1, PublicSubmissionStatusV1};
    let rig = TestRig::new().await;
    let owner = SigningKey::from_bytes(&[19; 32]);
    rig.rename(&owner, "Public status test", Ipv4Addr::new(127, 0, 0, 19))
        .await;
    let accepted = rig.submit(&owner, 1).await;
    let path = format!(
        "/api/v1/submissions/{}/public-status",
        accepted.submission_id
    );
    let response = rig
        .app
        .clone()
        .oneshot(empty_request(Method::GET, &path, Ipv4Addr::LOCALHOST))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let value: serde_json::Value = json_body(response).await;
    assert_eq!(
        value,
        serde_json::json!({"schema_version": 1, "submission_id": accepted.submission_id, "state": {"state": "queued"}})
    );
    for phase in ["verifying", "retry_pending", "rejected"] {
        sqlx::query("UPDATE submissions SET status = ?, lease_owner = CASE WHEN ? = 'verifying' THEN 'worker' ELSE NULL END, lease_expires_at_ms = CASE WHEN ? = 'verifying' THEN created_at_ms + 30000 ELSE NULL END, rejection_code = CASE WHEN ? = 'rejected' THEN 'PRIVATE_REJECTION_DETAIL' ELSE NULL END WHERE id = ?")
            .bind(phase).bind(phase).bind(phase).bind(phase).bind(accepted.submission_id.as_str()).execute(rig.database.fixture_pool()).await.unwrap();
        let response = rig
            .app
            .clone()
            .oneshot(empty_request(Method::GET, &path, Ipv4Addr::LOCALHOST))
            .await
            .unwrap();
        let value: serde_json::Value = json_body(response).await;
        assert_eq!(
            value,
            serde_json::json!({"schema_version": 1, "submission_id": accepted.submission_id, "state": {"state": phase}})
        );
    }
    sqlx::query("INSERT INTO submission_terminal_failures (submission_id, code, private_detail, failed_at_ms) VALUES (?, 'verification_infrastructure', 'PRIVATE_INFRASTRUCTURE_DETAIL', 1)")
        .bind(accepted.submission_id.as_str()).execute(rig.database.fixture_pool()).await.unwrap();
    let response = rig
        .app
        .clone()
        .oneshot(empty_request(Method::GET, &path, Ipv4Addr::LOCALHOST))
        .await
        .unwrap();
    let value: serde_json::Value = json_body(response).await;
    assert_eq!(
        value,
        serde_json::json!({"schema_version": 1, "submission_id": accepted.submission_id, "state": {"state": "failed"}})
    );
    sqlx::query("DELETE FROM submission_terminal_failures WHERE submission_id = ?")
        .bind(accepted.submission_id.as_str())
        .execute(rig.database.fixture_pool())
        .await
        .unwrap();
    sqlx::query("UPDATE submissions SET status = 'queued', rejection_code = NULL WHERE id = ?")
        .bind(accepted.submission_id.as_str())
        .execute(rig.database.fixture_pool())
        .await
        .unwrap();
    let run_id = rig.publish(&accepted, 123, 21).await;
    let response = rig
        .app
        .clone()
        .oneshot(empty_request(Method::GET, &path, Ipv4Addr::LOCALHOST))
        .await
        .unwrap();
    let status: PublicSubmissionStatusV1 = json_body(response).await;
    assert_eq!(status.state, PublicSubmissionStateV1::Verified { run_id });
    sqlx::query("UPDATE submissions SET tombstoned_at_ms = created_at_ms + 1 WHERE id = ?")
        .bind(accepted.submission_id.as_str())
        .execute(rig.database.fixture_pool())
        .await
        .unwrap();
    let deleted = rig
        .app
        .clone()
        .oneshot(empty_request(Method::GET, &path, Ipv4Addr::LOCALHOST))
        .await
        .unwrap();
    let missing = rig
        .app
        .clone()
        .oneshot(empty_request(
            Method::GET,
            "/api/v1/submissions/missing/public-status",
            Ipv4Addr::LOCALHOST,
        ))
        .await
        .unwrap();
    assert_eq!(deleted.status(), StatusCode::NOT_FOUND);
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        deleted.into_body().collect().await.unwrap().to_bytes(),
        missing.into_body().collect().await.unwrap().to_bytes()
    );
}

async fn bad_request_message(response: axum::response::Response) -> String {
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body: serde_json::Value = json_body(response).await;
    assert_eq!(body["error"]["code"], "bad_request");
    body["error"]["message"].as_str().unwrap().to_owned()
}

async fn upload_reservation_count(rig: &TestRig) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM submission_upload_reservations")
        .fetch_one(rig.database.fixture_pool())
        .await
        .unwrap()
}

#[tokio::test]
async fn ranked_submission_rejects_session_network_protocol_mismatch_before_reservation() {
    let rig = TestRig::new().await;
    let owner = SigningKey::from_bytes(&[96; 32]);
    rig.rename(&owner, "Network Robin", Ipv4Addr::new(127, 0, 9, 6))
        .await;

    // The host signs a genesis claiming a different network protocol than the
    // verifier build manifest. Offer issuance does not pin this field, so the
    // server issues a fully authentic offer that only submit can reject.
    let mismatched_network = NETWORK_PROTOCOL_VERSION + 1;
    assert_ne!(
        rig.loaded_build.semantics().network_protocol_version,
        mismatched_network
    );
    let mut request = rig.offer_request(&owner, 96);
    request.session_genesis.claim.network_protocol_version = mismatched_network;
    request.session_genesis.host_signature = Some(sign(
        &owner,
        &request.session_genesis.signing_bytes().unwrap(),
    ));
    let offer = rig.issue_offer(&owner, request, 96).await;
    assert_eq!(
        offer.session_genesis.claim.network_protocol_version,
        mismatched_network
    );
    let replay = compact_replay_fixture("network-96");
    let signed = signed_submission(&owner, offer, &replay, &rig.starting_campaign);

    for request in [
        metadata_only_multipart_request(&signed),
        multipart_request(&signed, &replay, &rig.starting_campaign),
    ] {
        let response = rig.app.clone().oneshot(request).await.unwrap();
        assert_eq!(
            bad_request_message(response).await,
            "ranked replay and network versions do not match the signed build manifest"
        );
    }
    assert_eq!(upload_reservation_count(&rig).await, 0);

    // The matching-version path through the same rig keeps accepting.
    let accepted = rig.submit(&owner, 97).await;
    assert_eq!(accepted.state, SubmissionLifecycleV1::Queued);
}

#[tokio::test]
async fn ranked_submission_rejects_signed_replay_schema_mismatch_before_reservation() {
    let rig = TestRig::new().await;
    let owner = SigningKey::from_bytes(&[98; 32]);
    rig.rename(&owner, "Schema Robin", Ipv4Addr::new(127, 0, 9, 8))
        .await;
    let offer = rig
        .issue_offer(&owner, rig.offer_request(&owner, 98), 98)
        .await;
    let replay = compact_replay_fixture("schema-98");

    // A replay artifact claiming a replay schema other than the build
    // manifest's. The protocol refuses to produce signing bytes for such an
    // envelope (`signing_bytes` validates, and `ReplayArtifactV1::validate`
    // pins the claim to the current ranked schema), so no participant can sign
    // it: sign the matching envelope and then change only the schema claim.
    // `submit` validates document shape before any signature check, so the
    // rejection below is caused by the schema claim alone. The server build
    // and rules config are likewise pinned to the current schema at load,
    // which is why this mismatch never reaches the manifest comparison.
    let mismatched_schema = REPLAY_SCHEMA_VERSION + 1;
    assert_ne!(
        rig.loaded_build.semantics().replay_schema_version,
        mismatched_schema
    );
    let mut signed = signed_submission(&owner, offer, &replay, &rig.starting_campaign);
    signed.submission.artifacts.replay.replay_schema_version = mismatched_schema;
    assert!(signed.submission.signing_bytes().is_err());
    assert!(signed.validate().is_err());

    for request in [
        metadata_only_multipart_request(&signed),
        multipart_request(&signed, &replay, &rig.starting_campaign),
    ] {
        let response = rig.app.clone().oneshot(request).await.unwrap();
        let message = bad_request_message(response).await;
        assert!(
            message.contains("replay.replay_schema_version"),
            "unexpected rejection: {message}"
        );
    }
    assert_eq!(upload_reservation_count(&rig).await, 0);
}
