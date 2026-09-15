use crate::support::*;

#[tokio::test]
async fn red_storage_keeps_health_live_and_reserves_nothing() {
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

    let replay = compact_replay_fixture("capacity-retry");
    let signed = signed_submission(
        &owner,
        submission(&owner, &replay, ParticipantPublicDisclosureV1::NamedProfile),
    );
    let denied_upload = red_app
        .oneshot(multipart_request(&signed, &replay))
        .await
        .unwrap();
    assert_eq!(denied_upload.status(), StatusCode::SERVICE_UNAVAILABLE);
    for table in [
        "SELECT COUNT(*) FROM submission_upload_reservations",
        "SELECT COUNT(*) FROM submissions",
        "SELECT COUNT(*) FROM replay_objects",
    ] {
        assert_eq!(rig.count(table).await, 0, "{table}");
    }

    // The same signed request succeeds once capacity is green, within its
    // freshness window.
    let accepted_retry = rig.send(multipart_request(&signed, &replay)).await;
    assert_eq!(accepted_retry.status(), StatusCode::ACCEPTED);
}
