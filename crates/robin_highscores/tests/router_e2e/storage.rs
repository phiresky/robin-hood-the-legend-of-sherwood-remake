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

    let denied_challenge = red_app
        .clone()
        .oneshot(json_request(
            Method::POST,
            "/api/v1/upload-challenges",
            &UploadChallengeRequestV2 {
                schema_version: SCHEMA_VERSION_V2,
                public_key: protocol_public_key(&owner),
            },
            Ipv4Addr::new(127, 0, 4, 2),
        ))
        .await
        .unwrap();
    assert_eq!(denied_challenge.status(), StatusCode::SERVICE_UNAVAILABLE);

    let challenge = rig.upload_challenge(&owner, Ipv4Addr::LOCALHOST).await;
    let replay = compact_replay_fixture("capacity-retry");
    let signed = signed_submission(
        &owner,
        submission(
            &owner,
            challenge,
            &replay,
            ParticipantPublicDisclosureV1::NamedProfile,
        ),
    );
    let denied_upload = red_app
        .oneshot(multipart_request(&signed, &replay))
        .await
        .unwrap();
    assert_eq!(denied_upload.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        sqlx::query_scalar::<_, Option<i64>>(
            "SELECT consumed_at_ms FROM upload_challenges WHERE id = ?"
        )
        .bind(
            signed
                .submission
                .upload_challenge
                .upload_challenge_id
                .as_str()
        )
        .fetch_one(rig.database.fixture_pool())
        .await
        .unwrap(),
        None,
        "red admission consumed the upload challenge"
    );
    assert_eq!(
        rig.count("SELECT COUNT(*) FROM submission_upload_reservations")
            .await,
        0
    );

    let accepted_retry = rig.send(multipart_request(&signed, &replay)).await;
    assert_eq!(accepted_retry.status(), StatusCode::ACCEPTED);
}
