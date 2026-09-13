use crate::support::*;

#[tokio::test]
async fn red_storage_keeps_health_live_and_does_not_issue_or_consume_admission() {
    let rig = TestRig::new().await;
    let owner = SigningKey::from_bytes(&[0x6d; 32]);
    rig.rename(&owner, "Capacity Robin", Ipv4Addr::new(127, 0, 4, 1))
        .await;
    let red_app = rig.app_with_storage_floor(u64::MAX);

    let health = red_app
        .clone()
        .oneshot(empty_request(Method::GET, "/healthz", Ipv4Addr::LOCALHOST))
        .await
        .unwrap();
    assert_eq!(health.status(), StatusCode::OK);
    let readiness = red_app
        .clone()
        .oneshot(empty_request(Method::GET, "/readyz", Ipv4Addr::LOCALHOST))
        .await
        .unwrap();
    assert_eq!(readiness.status(), StatusCode::SERVICE_UNAVAILABLE);

    let mut request = rig.offer_request(&owner, 41);
    rig.authorize_fresh_request(&owner, &mut request, 41).await;
    let denied_offer = red_app
        .clone()
        .oneshot(json_request(
            Method::POST,
            "/api/v1/submission-offers",
            &request,
            Ipv4Addr::new(127, 0, 4, 2),
        ))
        .await
        .unwrap();
    assert_eq!(denied_offer.status(), StatusCode::SERVICE_UNAVAILABLE);

    let offer = rig
        .issue_offer(&owner, rig.offer_request(&owner, 42), 42)
        .await;
    let replay = compact_replay_fixture("capacity-retry");
    let envelope = SubmissionEnvelopeV1 {
        schema_version: SCHEMA_VERSION_V1,
        replay_session_transcript: replay_session_transcript(&offer),
        artifacts: submission_artifacts(&replay, &rig.starting_campaign),
        offer,
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

    let denied_upload = red_app
        .oneshot(multipart_request(&signed, &replay, &rig.starting_campaign))
        .await
        .unwrap();
    assert_eq!(denied_upload.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        sqlx::query_scalar::<_, Option<i64>>(
            "SELECT consumed_at_ms FROM upload_challenges WHERE id = ?"
        )
        .bind(signed.submission.offer.upload_challenge_id.as_str())
        .fetch_one(rig.database.fixture_pool())
        .await
        .unwrap(),
        None,
        "red admission consumed the upload challenge"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM submission_upload_reservations")
            .fetch_one(rig.database.fixture_pool())
            .await
            .unwrap(),
        0
    );

    let accepted_retry = rig
        .app
        .clone()
        .oneshot(multipart_request(&signed, &replay, &rig.starting_campaign))
        .await
        .unwrap();
    assert_eq!(accepted_retry.status(), StatusCode::ACCEPTED);
}

#[tokio::test]
async fn missing_authenticated_backup_blocks_offers_and_fresh_upload_reservations() {
    let rig = TestRig::new().await;
    let owner = SigningKey::from_bytes(&[0x6e; 32]);
    rig.rename(&owner, "Backup Robin", Ipv4Addr::new(127, 0, 5, 1))
        .await;
    let offer_request = rig.offer_request(&owner, 51);
    let offer = rig.issue_offer(&owner, offer_request, 51).await;
    let replay = compact_replay_fixture("backup-retry");
    let envelope = SubmissionEnvelopeV1 {
        schema_version: SCHEMA_VERSION_V1,
        replay_session_transcript: replay_session_transcript(&offer),
        artifacts: submission_artifacts(&replay, &rig.starting_campaign),
        offer,
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

    let red_app =
        rig.app_with_backup_status(rig.config.database_path.with_file_name("missing.json"));
    let mut denied_offer_request = rig.offer_request(&owner, 52);
    rig.authorize_fresh_request(&owner, &mut denied_offer_request, 52)
        .await;
    let health = red_app
        .clone()
        .oneshot(empty_request(Method::GET, "/healthz", Ipv4Addr::LOCALHOST))
        .await
        .unwrap();
    assert_eq!(health.status(), StatusCode::OK);
    let readiness = red_app
        .clone()
        .oneshot(empty_request(Method::GET, "/readyz", Ipv4Addr::LOCALHOST))
        .await
        .unwrap();
    assert_eq!(readiness.status(), StatusCode::SERVICE_UNAVAILABLE);
    let denied_offer = red_app
        .clone()
        .oneshot(json_request(
            Method::POST,
            "/api/v1/submission-offers",
            &denied_offer_request,
            Ipv4Addr::new(127, 0, 5, 2),
        ))
        .await
        .unwrap();
    assert_eq!(denied_offer.status(), StatusCode::SERVICE_UNAVAILABLE);
    let denied_upload = red_app
        .oneshot(multipart_request(&signed, &replay, &rig.starting_campaign))
        .await
        .unwrap();
    assert_eq!(denied_upload.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        sqlx::query_scalar::<_, Option<i64>>(
            "SELECT consumed_at_ms FROM upload_challenges WHERE id = ?"
        )
        .bind(signed.submission.offer.upload_challenge_id.as_str())
        .fetch_one(rig.database.fixture_pool())
        .await
        .unwrap(),
        None
    );
}
