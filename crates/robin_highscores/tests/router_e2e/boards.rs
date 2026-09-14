use crate::support::*;

#[tokio::test]
async fn pagination_player_history_report_quotas_and_signed_tombstone_remain_consistent() {
    let rig = TestRig::new().await;
    let owner = SigningKey::from_bytes(&[31; 32]);
    let other = SigningKey::from_bytes(&[32; 32]);
    let owner_key = protocol_public_key(&owner);
    let other_key = protocol_public_key(&other);
    rig.rename(&owner, "Marian", Ipv4Addr::new(127, 0, 1, 1))
        .await;
    rig.rename(&other, "Little John", Ipv4Addr::new(127, 0, 1, 2))
        .await;

    let mut runs = Vec::new();
    for (sequence, score, final_byte) in [(2, 300, 41), (3, 200, 42), (4, 100, 43)] {
        let accepted = rig.submit(&owner, sequence).await;
        runs.push(rig.publish(&accepted, score, final_byte).await);
    }

    let first_page_response = rig
        .app
        .clone()
        .oneshot(empty_request(
            Method::GET,
            &leaderboard_uri(&rig, 2, None),
            Ipv4Addr::LOCALHOST,
        ))
        .await
        .unwrap();
    assert_eq!(first_page_response.status(), StatusCode::OK);
    let first_page: LeaderboardPageV1 = json_body(first_page_response).await;
    first_page
        .validate_against_ruleset(&rig.published_ruleset)
        .unwrap();
    assert_eq!(first_page.entries.len(), 2);
    assert_eq!(first_page.entries[0].run_id, runs[0]);
    assert_eq!(first_page.entries[1].run_id, runs[1]);
    assert_eq!(first_page.entries[0].rank, 1);
    assert_eq!(first_page.entries[1].rank, 2);
    assert_public_projection_is_redacted(&first_page);
    let cursor = first_page
        .next_cursor
        .as_ref()
        .unwrap()
        .opaque_token
        .clone();

    let second_page_response = rig
        .app
        .clone()
        .oneshot(empty_request(
            Method::GET,
            &leaderboard_uri(&rig, 2, Some(&cursor)),
            Ipv4Addr::LOCALHOST,
        ))
        .await
        .unwrap();
    assert_eq!(second_page_response.status(), StatusCode::OK);
    let second_page: LeaderboardPageV1 = json_body(second_page_response).await;
    second_page
        .validate_against_ruleset(&rig.published_ruleset)
        .unwrap();
    assert_eq!(second_page.entries.len(), 1);
    assert_eq!(second_page.entries[0].run_id, runs[2]);
    assert_eq!(second_page.entries[0].rank, 3);
    assert_public_projection_is_redacted(&second_page);

    let history_response = rig
        .app
        .clone()
        .oneshot(empty_request(
            Method::GET,
            &format!("/api/v1/players/{owner_key}/runs?schema_version=1&limit=2"),
            Ipv4Addr::LOCALHOST,
        ))
        .await
        .unwrap();
    assert_eq!(history_response.status(), StatusCode::OK);
    let history: PlayerRunHistoryPageV1 = json_body(history_response).await;
    history.validate().unwrap();
    assert_eq!(history.runs.len(), 2);
    assert!(history.next_cursor.is_some());
    assert_eq!(history.personal_bests.len(), 2);
    assert_public_projection_is_redacted(&history);
    assert_eq!(
        history
            .personal_bests
            .iter()
            .find(|best| best.filter.metric == BoardMetricV1::OriginalScore)
            .unwrap()
            .run_id,
        runs[0]
    );
    assert_eq!(
        history
            .personal_bests
            .iter()
            .find(|best| best.filter.metric == BoardMetricV1::FastestSuccess)
            .unwrap()
            .run_id,
        runs[2]
    );

    assert_report_status(
        &rig,
        AbuseReportTargetV1::Player {
            public_key: owner_key,
        },
        Ipv4Addr::new(10, 0, 0, 1),
        StatusCode::ACCEPTED,
    )
    .await;
    assert_report_status(
        &rig,
        AbuseReportTargetV1::Player {
            public_key: other_key,
        },
        Ipv4Addr::new(10, 0, 0, 1),
        StatusCode::TOO_MANY_REQUESTS,
    )
    .await;
    assert_report_status(
        &rig,
        AbuseReportTargetV1::Player {
            public_key: owner_key,
        },
        Ipv4Addr::new(10, 0, 0, 2),
        StatusCode::ACCEPTED,
    )
    .await;
    assert_report_status(
        &rig,
        AbuseReportTargetV1::Player {
            public_key: owner_key,
        },
        Ipv4Addr::new(10, 0, 0, 3),
        StatusCode::TOO_MANY_REQUESTS,
    )
    .await;
    assert_report_status(
        &rig,
        AbuseReportTargetV1::Run {
            run_id: runs[0].clone(),
        },
        Ipv4Addr::new(10, 0, 0, 3),
        StatusCode::ACCEPTED,
    )
    .await;
    assert_report_status(
        &rig,
        AbuseReportTargetV1::Run {
            run_id: runs[1].clone(),
        },
        Ipv4Addr::new(10, 0, 0, 4),
        StatusCode::TOO_MANY_REQUESTS,
    )
    .await;

    let deletion_challenge_response = rig
        .app
        .clone()
        .oneshot(json_request(
            Method::POST,
            "/api/v1/deletion-challenges",
            &DeletionChallengeRequestV1 {
                schema_version: SCHEMA_VERSION_V1,
                public_key: owner_key,
                target: DeletionTargetV1::Run {
                    run_id: runs[1].clone(),
                },
            },
            Ipv4Addr::new(127, 0, 2, 1),
        ))
        .await
        .unwrap();
    assert_eq!(deletion_challenge_response.status(), StatusCode::CREATED);
    let challenge: DeletionChallengeV1 = json_body(deletion_challenge_response).await;
    challenge.validate().unwrap();
    let mut deletion = DeletionRequestEnvelopeV1 {
        schema_version: SCHEMA_VERSION_V1,
        challenge,
        signature: Signature64::default(),
    };
    deletion.signature = sign(&owner, &deletion.signing_bytes().unwrap());
    deletion.validate().unwrap();
    let deletion_response = rig
        .app
        .clone()
        .oneshot(json_request(
            Method::POST,
            "/api/v1/deletion-requests",
            &deletion,
            Ipv4Addr::LOCALHOST,
        ))
        .await
        .unwrap();
    assert_eq!(deletion_response.status(), StatusCode::OK);
    let receipt: DeletionReceiptV1 = json_body(deletion_response).await;
    receipt.validate().unwrap();
    assert_eq!(
        receipt.target,
        DeletionTargetV1::Run {
            run_id: runs[1].clone()
        }
    );

    for path in [
        format!("/api/v1/runs/{}", runs[1]),
        format!("/api/v1/runs/{}/replay", runs[1]),
    ] {
        let response = rig
            .app
            .clone()
            .oneshot(empty_request(Method::GET, &path, Ipv4Addr::LOCALHOST))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    let stale_cursor_response = rig
        .app
        .clone()
        .oneshot(empty_request(
            Method::GET,
            &leaderboard_uri(&rig, 2, Some(&cursor)),
            Ipv4Addr::LOCALHOST,
        ))
        .await
        .unwrap();
    assert_eq!(stale_cursor_response.status(), StatusCode::CONFLICT);

    let refreshed_response = rig
        .app
        .clone()
        .oneshot(empty_request(
            Method::GET,
            &leaderboard_uri(&rig, 10, None),
            Ipv4Addr::LOCALHOST,
        ))
        .await
        .unwrap();
    let refreshed: LeaderboardPageV1 = json_body(refreshed_response).await;
    assert_eq!(refreshed.entries.len(), 2);
    assert!(
        refreshed
            .entries
            .iter()
            .all(|entry| entry.run_id != runs[1])
    );

    let refreshed_history_response = rig
        .app
        .clone()
        .oneshot(empty_request(
            Method::GET,
            &format!("/api/v1/players/{owner_key}/runs?schema_version=1&limit=10"),
            Ipv4Addr::LOCALHOST,
        ))
        .await
        .unwrap();
    let refreshed_history: PlayerRunHistoryPageV1 = json_body(refreshed_history_response).await;
    refreshed_history.validate().unwrap();
    assert_eq!(refreshed_history.runs.len(), 2);
    assert!(
        refreshed_history
            .runs
            .iter()
            .all(|entry| entry.run.run_id != runs[1])
    );
}

#[tokio::test]
async fn active_competition_offer_upload_publication_and_board_cross_the_real_router() {
    let rig = TestRig::new_with(RigOptions {
        competition: true,
        ..RigOptions::default()
    })
    .await;
    let owner = SigningKey::from_bytes(&[51; 32]);
    rig.rename(&owner, "Will Scarlet", Ipv4Addr::new(127, 0, 3, 1))
        .await;
    let competition_sha256 = rig.competition_sha256.unwrap();

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
    let metadata: LeaderboardMetadataV1 = json_body(metadata_response).await;
    metadata.validate().unwrap();
    assert_eq!(metadata.competitions.len(), 1);
    assert_eq!(
        metadata.competitions[0].competition_manifest_sha256,
        competition_sha256
    );
    assert_eq!(metadata.competitions[0].state, CompetitionStateV1::Active);

    let manifest_response = rig
        .app
        .clone()
        .oneshot(empty_request(
            Method::GET,
            &format!("/api/v1/competitions/{competition_sha256}"),
            Ipv4Addr::LOCALHOST,
        ))
        .await
        .unwrap();
    assert_eq!(manifest_response.status(), StatusCode::OK);
    let manifest: CompetitionManifestV1 = json_body(manifest_response).await;
    manifest.validate().unwrap();
    assert_eq!(manifest.canonical_digest().unwrap(), competition_sha256);

    let prefetched = rig.competition_offer_request(&owner, 49).await;
    let replacement = rig.competition_offer_request(&owner, 50).await;
    let invalidated_response = rig
        .app
        .clone()
        .oneshot(json_request(
            Method::POST,
            "/api/v1/submission-offers",
            &prefetched,
            Ipv4Addr::new(127, 10, 0, 49),
        ))
        .await
        .unwrap();
    assert_eq!(invalidated_response.status(), StatusCode::CONFLICT);

    let authorized_request = replacement;
    let mut transplanted = authorized_request.clone();
    transplanted.session_genesis.claim.replay_session_id = Digest32::from_bytes([0xee; 32]);
    transplanted.session_genesis.host_signature = Some(sign(
        &owner,
        &transplanted.session_genesis.signing_bytes().unwrap(),
    ));
    assert!(transplanted.validate().is_err());
    let mut forged_authority = authorized_request.clone();
    forged_authority
        .session_genesis
        .claim
        .competition_run_grant
        .as_mut()
        .unwrap()
        .authority_signature = Signature64::from_bytes([0xa4; 64]);
    forged_authority.session_genesis.host_signature = Some(sign(
        &owner,
        &forged_authority.session_genesis.signing_bytes().unwrap(),
    ));
    let forged_response = rig
        .app
        .clone()
        .oneshot(json_request(
            Method::POST,
            "/api/v1/submission-offers",
            &forged_authority,
            Ipv4Addr::new(127, 10, 0, 51),
        ))
        .await
        .unwrap();
    assert_eq!(forged_response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        authorized_request
            .session_genesis
            .claim
            .ranked_session
            .simulation_seed,
        SimulationSeed64::new(777)
    );
    let accepted = rig
        .submit_request(
            &owner,
            52,
            authorized_request.clone(),
            vec![BoardMetricV1::OriginalScore],
            CampaignAggregationConsentV1::NotAuthorized,
            None,
        )
        .await;
    let replayed_grant_response = rig
        .app
        .clone()
        .oneshot(json_request(
            Method::POST,
            "/api/v1/submission-offers",
            &authorized_request,
            Ipv4Addr::new(127, 10, 0, 52),
        ))
        .await
        .unwrap();
    assert_eq!(replayed_grant_response.status(), StatusCode::CONFLICT);
    let _next_attempt_after_completed_upload = rig.competition_offer_request(&owner, 53).await;

    let run_id = rig.publish_competition(&accepted, 44_000, 54).await;
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
    let detail: RunDetailV1 = json_body(detail_response).await;
    detail
        .validate_against_ruleset(&rig.published_ruleset, None)
        .unwrap();
    assert_eq!(detail.competition_manifest_sha256, Some(competition_sha256));

    let leaderboard_response = rig
        .app
        .clone()
        .oneshot(empty_request(
            Method::GET,
            &competition_leaderboard_uri(&rig, competition_sha256),
            Ipv4Addr::LOCALHOST,
        ))
        .await
        .unwrap();
    assert_eq!(leaderboard_response.status(), StatusCode::OK);
    let leaderboard: LeaderboardPageV1 = json_body(leaderboard_response).await;
    leaderboard
        .validate_against_ruleset(&rig.published_ruleset)
        .unwrap();
    assert_eq!(leaderboard.entries.len(), 1);
    assert_eq!(leaderboard.entries[0].run_id, run_id);
}

#[tokio::test]
async fn disabled_board_and_operator_metrics_are_absent_at_the_real_router() {
    let rig = TestRig::new_with(RigOptions {
        allowed_metrics: vec!["original_score".to_owned()],
        ..RigOptions::default()
    })
    .await;
    let owner = SigningKey::from_bytes(&[71; 32]);
    rig.rename(&owner, "Much", Ipv4Addr::new(127, 0, 5, 1))
        .await;
    let offer = rig
        .issue_offer(&owner, rig.offer_request(&owner, 71), 71)
        .await;
    assert_eq!(offer.allowed_metrics, vec![BoardMetricV1::OriginalScore]);

    let fastest_response = rig
        .app
        .clone()
        .oneshot(empty_request(
            Method::GET,
            &mission_leaderboard_uri(&rig, MISSION_ID, "individual_level", "fastest_success"),
            Ipv4Addr::LOCALHOST,
        ))
        .await
        .unwrap();
    assert_eq!(fastest_response.status(), StatusCode::NOT_FOUND);

    let original_response = rig
        .app
        .clone()
        .oneshot(empty_request(
            Method::GET,
            &mission_leaderboard_uri(&rig, MISSION_ID, "individual_level", "original_score"),
            Ipv4Addr::LOCALHOST,
        ))
        .await
        .unwrap();
    assert_eq!(original_response.status(), StatusCode::OK);

    for path in [
        "/api/v1/operator/metrics",
        "/api/v1/operator/operational-status",
    ] {
        let response = rig
            .app
            .clone()
            .oneshot(empty_request(Method::GET, path, Ipv4Addr::LOCALHOST))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
