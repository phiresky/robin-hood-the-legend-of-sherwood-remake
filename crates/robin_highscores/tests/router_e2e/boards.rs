use crate::support::*;

async fn page(rig: &TestRig, uri: &str) -> LeaderboardPageV2 {
    let response = rig
        .send(empty_request(Method::GET, uri, Ipv4Addr::LOCALHOST))
        .await;
    assert_eq!(response.status(), StatusCode::OK, "{uri}");
    let page: LeaderboardPageV2 = json_body(response).await;
    page.validate().unwrap();
    page
}

#[tokio::test]
async fn leaderboard_ranks_ties_pages_with_cursors_and_rejects_unknown_boards() {
    let rig = TestRig::new().await;
    let keys = [0x71_u8, 0x72, 0x73, 0x74].map(|byte| SigningKey::from_bytes(&[byte; 32]));
    for (index, key) in keys.iter().enumerate() {
        rig.rename(
            key,
            &format!("Player {index}"),
            Ipv4Addr::new(127, 0, 1, index as u8),
        )
        .await;
    }
    let mut runs = Vec::new();
    for (key, label, score, ticks) in [
        (&keys[0], "rank-a", 50, 400),
        (&keys[1], "rank-b", 50, 300),
        (&keys[2], "rank-c", 40, 200),
        (&keys[3], "rank-d", 70, 500),
    ] {
        runs.push(
            rig.publish_run(
                key,
                label,
                score,
                ticks,
                ParticipantPublicDisclosureV1::NamedProfile,
            )
            .await,
        );
    }

    let first = page(&rig, &leaderboard_uri(BOARD_ID, "original_score", 2, None)).await;
    assert_eq!(
        first
            .entries
            .iter()
            .map(|entry| (entry.run_id.clone(), entry.rank))
            .collect::<Vec<_>>(),
        [(runs[3].clone(), 1), (runs[0].clone(), 2)]
    );
    let cursor = first.next_cursor.clone().unwrap();
    let second = page(
        &rig,
        &leaderboard_uri(BOARD_ID, "original_score", 2, Some(&cursor.opaque_token)),
    )
    .await;
    assert_eq!(
        second
            .entries
            .iter()
            .map(|entry| (entry.run_id.clone(), entry.position, entry.rank))
            .collect::<Vec<_>>(),
        [(runs[1].clone(), 3, 2), (runs[2].clone(), 4, 4)]
    );
    assert!(second.next_cursor.is_none());
    assert_eq!(second.previous_cursor.as_ref(), Some(&cursor));

    let fastest = page(
        &rig,
        &leaderboard_uri(BOARD_ID, "fastest_success", 10, None),
    )
    .await;
    assert_eq!(fastest.entries[0].run_id, runs[2]);

    // A cursor is bound to its exact query.
    let response = rig
        .send(empty_request(
            Method::GET,
            &leaderboard_uri(BOARD_ID, "fastest_success", 2, Some(&cursor.opaque_token)),
            Ipv4Addr::LOCALHOST,
        ))
        .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let player = protocol_public_key(&keys[1]);
    let filtered = page(
        &rig,
        &format!(
            "{}&player_public_key={player}",
            leaderboard_uri(BOARD_ID, "original_score", 10, None)
        ),
    )
    .await;
    assert_eq!(filtered.entries.len(), 1);
    assert_eq!(
        filtered.entries[0].uploader.as_ref().unwrap().public_key,
        player
    );

    for (uri, status) in [
        (
            leaderboard_uri("full-any", "original_score", 10, None),
            StatusCode::NOT_FOUND,
        ),
        (
            leaderboard_uri(BOARD_ID, "original_score", 10, None).replace(MISSION_ID, "Demo_Lin"),
            StatusCode::NOT_FOUND,
        ),
        (
            leaderboard_uri(SCORE_ONLY_BOARD_ID, "fastest_success", 10, None),
            StatusCode::BAD_REQUEST,
        ),
        (
            format!(
                "{}&surprise=1",
                leaderboard_uri(BOARD_ID, "original_score", 10, None)
            ),
            StatusCode::BAD_REQUEST,
        ),
    ] {
        let response = rig
            .send(empty_request(Method::GET, &uri, Ipv4Addr::LOCALHOST))
            .await;
        assert_eq!(response.status(), status, "{uri}");
    }
    let empty = page(
        &rig,
        &leaderboard_uri(SCORE_ONLY_BOARD_ID, "original_score", 10, None),
    )
    .await;
    assert!(empty.entries.is_empty());
}

#[tokio::test]
async fn history_anonymity_reports_and_signed_tombstone_remain_consistent() {
    let rig = TestRig::new().await;
    let owner = SigningKey::from_bytes(&[0x81; 32]);
    let rival = SigningKey::from_bytes(&[0x82; 32]);
    rig.rename(&owner, "Owner", Ipv4Addr::new(127, 0, 2, 1))
        .await;
    rig.rename(&rival, "Rival", Ipv4Addr::new(127, 0, 2, 2))
        .await;
    let named_low = rig
        .publish_run(
            &owner,
            "history-a",
            20,
            900,
            ParticipantPublicDisclosureV1::NamedProfile,
        )
        .await;
    let named_high = rig
        .publish_run(
            &owner,
            "history-b",
            60,
            950,
            ParticipantPublicDisclosureV1::NamedProfile,
        )
        .await;
    let anonymous = rig
        .publish_run(
            &owner,
            "history-c",
            99,
            100,
            ParticipantPublicDisclosureV1::Anonymous,
        )
        .await;
    rig.publish_run(
        &rival,
        "history-d",
        30,
        800,
        ParticipantPublicDisclosureV1::NamedProfile,
    )
    .await;

    let history_uri = |limit: u16, cursor: Option<&str>| {
        let mut uri = format!(
            "/api/v1/players/{}/runs?schema_version=1&limit={limit}",
            protocol_public_key(&owner)
        );
        if let Some(cursor) = cursor {
            uri.push_str("&cursor=");
            uri.push_str(cursor);
        }
        uri
    };
    let response = rig
        .send(empty_request(
            Method::GET,
            &history_uri(1, None),
            Ipv4Addr::LOCALHOST,
        ))
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    let first: PlayerRunHistoryPageV2 = json_body(response).await;
    first.validate().unwrap();
    assert_eq!(first.runs.len(), 1);
    assert_eq!(first.runs[0].run.run_id, named_high);
    // The cursor binds the exact query, including its page size.
    assert_eq!(
        rig.send(empty_request(
            Method::GET,
            &history_uri(10, first.next_cursor.as_deref()),
            Ipv4Addr::LOCALHOST,
        ))
        .await
        .status(),
        StatusCode::BAD_REQUEST
    );
    let second: PlayerRunHistoryPageV2 = json_body(
        rig.send(empty_request(
            Method::GET,
            &history_uri(1, first.next_cursor.as_deref()),
            Ipv4Addr::LOCALHOST,
        ))
        .await,
    )
    .await;
    second.validate().unwrap();
    assert_eq!(
        second
            .runs
            .iter()
            .map(|run| run.run.run_id.clone())
            .collect::<Vec<_>>(),
        [named_low.clone()],
        "anonymous runs never appear in public player history"
    );
    assert!(second.next_cursor.is_none());
    let best_score = first
        .personal_bests
        .iter()
        .find(|best| best.filter.metric == BoardMetricV1::OriginalScore)
        .unwrap();
    assert_eq!(best_score.run_id, named_high);
    assert!(
        !first
            .personal_bests
            .iter()
            .any(|best| best.run_id == anonymous)
    );

    let anonymous_detail: RunDetailV2 = json_body(
        rig.send(empty_request(
            Method::GET,
            &format!("/api/v1/runs/{anonymous}"),
            Ipv4Addr::LOCALHOST,
        ))
        .await,
    )
    .await;
    assert!(anonymous_detail.uploader.is_none());
    assert!(
        !serde_json::to_string(&anonymous_detail)
            .unwrap()
            .contains(&protocol_public_key(&owner).to_string())
    );

    assert_report_status(
        &rig,
        AbuseReportTargetV1::Run {
            run_id: named_low.clone(),
        },
        Ipv4Addr::new(127, 0, 2, 9),
        StatusCode::ACCEPTED,
    )
    .await;
    assert_report_status(
        &rig,
        AbuseReportTargetV1::Run {
            run_id: OpaqueId::new("missing-run").unwrap(),
        },
        Ipv4Addr::new(127, 0, 2, 9),
        StatusCode::NOT_FOUND,
    )
    .await;

    // A cursor minted before the owner tombstones a run is refused afterwards.
    let before = page(&rig, &leaderboard_uri(BOARD_ID, "original_score", 1, None)).await;
    let stale_cursor = before.next_cursor.unwrap().opaque_token;

    let challenge_request = DeletionChallengeRequestV1 {
        schema_version: SCHEMA_VERSION_V1,
        public_key: protocol_public_key(&rival),
        target: DeletionTargetV1::Run {
            run_id: named_high.clone(),
        },
    };
    let rival_challenge: DeletionChallengeV1 = json_body(
        rig.send(json_request(
            Method::POST,
            "/api/v1/deletion-challenges",
            &challenge_request,
            Ipv4Addr::LOCALHOST,
        ))
        .await,
    )
    .await;
    let mut rival_request = DeletionRequestEnvelopeV1 {
        schema_version: SCHEMA_VERSION_V1,
        challenge: rival_challenge,
        signature: Signature64::from_bytes([0; 64]),
    };
    rival_request.signature = sign(&rival, &rival_request.signing_bytes().unwrap());
    assert_eq!(
        rig.send(json_request(
            Method::POST,
            "/api/v1/deletion-requests",
            &rival_request,
            Ipv4Addr::LOCALHOST,
        ))
        .await
        .status(),
        StatusCode::NOT_FOUND,
        "only the uploader may tombstone a run"
    );

    let owner_challenge: DeletionChallengeV1 = json_body(
        rig.send(json_request(
            Method::POST,
            "/api/v1/deletion-challenges",
            &DeletionChallengeRequestV1 {
                public_key: protocol_public_key(&owner),
                ..challenge_request
            },
            Ipv4Addr::LOCALHOST,
        ))
        .await,
    )
    .await;
    let mut owner_request = DeletionRequestEnvelopeV1 {
        schema_version: SCHEMA_VERSION_V1,
        challenge: owner_challenge,
        signature: Signature64::from_bytes([0; 64]),
    };
    owner_request.signature = sign(&owner, &owner_request.signing_bytes().unwrap());
    let receipt = rig
        .send(json_request(
            Method::POST,
            "/api/v1/deletion-requests",
            &owner_request,
            Ipv4Addr::LOCALHOST,
        ))
        .await;
    assert_eq!(receipt.status(), StatusCode::OK);
    let receipt: DeletionReceiptV1 = json_body(receipt).await;
    receipt.validate().unwrap();

    for uri in [
        format!("/api/v1/runs/{named_high}"),
        format!("/api/v1/runs/{named_high}/replay"),
    ] {
        assert_eq!(
            rig.send(empty_request(Method::GET, &uri, Ipv4Addr::LOCALHOST))
                .await
                .status(),
            StatusCode::NOT_FOUND
        );
    }
    let after = page(&rig, &leaderboard_uri(BOARD_ID, "original_score", 10, None)).await;
    assert!(after.entries.iter().all(|entry| entry.run_id != named_high));
    assert_eq!(
        rig.send(empty_request(
            Method::GET,
            &leaderboard_uri(BOARD_ID, "original_score", 1, Some(&stale_cursor)),
            Ipv4Addr::LOCALHOST,
        ))
        .await
        .status(),
        StatusCode::CONFLICT
    );
}

#[tokio::test]
async fn operator_routes_are_absent_without_a_token_and_authenticated_with_one() {
    let rig = TestRig::new().await;
    for uri in [
        "/api/v1/operator/reports",
        "/api/v1/operator/metrics",
        "/api/v1/operator/operational-status",
    ] {
        assert_eq!(
            rig.send(empty_request(Method::GET, uri, Ipv4Addr::LOCALHOST))
                .await
                .status(),
            StatusCode::NOT_FOUND
        );
    }
    for removed in [
        "/api/v1/submission-offers",
        "/api/v1/fresh-run-preflight-grants",
        "/api/v1/campaign-continuation-preflight-grants",
        "/api/v1/competition-run-grants",
    ] {
        assert_eq!(
            rig.send(json_request(
                Method::POST,
                removed,
                &serde_json::json!({}),
                Ipv4Addr::LOCALHOST
            ))
            .await
            .status(),
            StatusCode::NOT_FOUND,
            "{removed}"
        );
    }
    for removed in [
        format!("/api/v1/builds/{}", "00".repeat(32)),
        format!("/api/v1/published-rulesets/{}", "00".repeat(32)),
        "/api/v1/runs/run/campaigns/final".to_owned(),
        "/api/v1/runs/run/sessions/0".to_owned(),
    ] {
        assert_eq!(
            rig.send(empty_request(Method::GET, &removed, Ipv4Addr::LOCALHOST))
                .await
                .status(),
            StatusCode::NOT_FOUND,
            "{removed}"
        );
    }

    let token = b"0123456789abcdef0123456789abcdef";
    let operator = rig.app_with_operator_token(token);
    let unauthorized = operator
        .clone()
        .oneshot(empty_request(
            Method::GET,
            "/api/v1/operator/metrics",
            Ipv4Addr::LOCALHOST,
        ))
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
    let metrics = operator
        .oneshot(
            Request::builder()
                .uri("/api/v1/operator/metrics")
                .header(
                    "authorization",
                    format!("Bearer {}", std::str::from_utf8(token).unwrap()),
                )
                .extension(peer(Ipv4Addr::LOCALHOST))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(metrics.status(), StatusCode::OK);
    let body = metrics.into_body().collect().await.unwrap().to_bytes();
    assert!(
        std::str::from_utf8(&body)
            .unwrap()
            .contains("robin_highscores_accepted_runs 0")
    );
}
