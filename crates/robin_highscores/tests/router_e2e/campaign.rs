use crate::support::*;

#[tokio::test]
async fn campaign_owner_authorization_authentic_genesis_and_full_aggregate_cross_the_real_router() {
    let rig = TestRig::new_with(RigOptions {
        campaign: true,
        ..RigOptions::default()
    })
    .await;
    let owner = SigningKey::from_bytes(&[61; 32]);
    let attacker = SigningKey::from_bytes(&[62; 32]);
    rig.rename(&owner, "Maid Marian", Ipv4Addr::new(127, 0, 4, 1))
        .await;
    rig.rename(&attacker, "Guy of Gisborne", Ipv4Addr::new(127, 0, 4, 2))
        .await;

    let mut genesis_request = rig.campaign_offer_request(
        &owner,
        63,
        GENESIS_MISSION_ID,
        OfficialContentSubjectV1::FieldMission {
            mission_id: GENESIS_MISSION_ID.to_owned(),
        },
        rig.genesis_content_sha256.unwrap(),
        rig.starting_campaign_sha256,
        rig.starting_campaign.len() as u64,
        ScopeRequestV1::CampaignGenesis,
    );
    genesis_request.participant_claims[0].public_disclosure =
        ParticipantPublicDisclosureV1::Anonymous;
    let genesis_offer = rig.issue_offer(&owner, genesis_request.clone(), 63).await;
    let genesis_accepted = rig.upload_campaign_offer(&owner, 63, genesis_offer).await;
    let genesis_run_id = rig
        .publish_campaign(
            &genesis_accepted,
            CampaignSessionKindV1::FieldMission {
                mission_id: GENESIS_MISSION_ID.to_owned(),
            },
            0,
            0,
            100,
            64,
            false,
        )
        .await;
    let active_receipt = rig.campaign_receipt(&genesis_accepted, &owner).await;
    let private_chain_id = active_receipt.chain_id.clone();
    assert_eq!(active_receipt.predecessor_run_id, genesis_run_id);
    assert_eq!(active_receipt.state, CampaignChainStateV1::Active);
    assert_eq!(
        active_receipt.campaign_controller_public_key,
        protocol_public_key(&owner)
    );

    let continuation_scope = ScopeRequestV1::CampaignContinuation {
        chain_id: active_receipt.chain_id.clone(),
        predecessor_run_id: genesis_run_id.clone(),
    };
    let unauthorized = rig.campaign_offer_request(
        &attacker,
        70,
        CAMPAIGN_MISSION_ID,
        OfficialContentSubjectV1::FieldMission {
            mission_id: CAMPAIGN_MISSION_ID.to_owned(),
        },
        rig.content_sha256,
        active_receipt.expected_starting_campaign.sha256,
        active_receipt.expected_starting_campaign.byte_length,
        continuation_scope.clone(),
    );
    let mut unauthorized_preflight =
        rig.continuation_preflight_request(&unauthorized, &active_receipt, 70);
    unauthorized_preflight.claim.max_concurrent_players = 2;
    unauthorized_preflight.claim.participant_public_keys =
        vec![protocol_public_key(&owner), protocol_public_key(&attacker)];
    unauthorized_preflight
        .claim
        .participant_public_keys
        .sort_unstable();
    unauthorized_preflight.host_signature = sign(
        &attacker,
        &unauthorized_preflight.claim.host_signing_bytes().unwrap(),
    );
    // The claimed controller is the immutable owner, so an attacker-authored
    // controller signature must fail before any predecessor state is exposed.
    unauthorized_preflight.controller_signature = sign(
        &attacker,
        &unauthorized_preflight
            .claim
            .controller_signing_bytes()
            .unwrap(),
    );
    unauthorized_preflight.validate().unwrap();
    let unauthorized_response = rig
        .app
        .clone()
        .oneshot(json_request(
            Method::POST,
            "/api/v1/campaign-continuation-preflight-grants",
            &unauthorized_preflight,
            Ipv4Addr::new(127, 0, 4, 2),
        ))
        .await
        .unwrap();
    assert_eq!(unauthorized_response.status(), StatusCode::UNAUTHORIZED);

    let mut continuation_request = rig.campaign_offer_request(
        &owner,
        66,
        CAMPAIGN_MISSION_ID,
        OfficialContentSubjectV1::FieldMission {
            mission_id: CAMPAIGN_MISSION_ID.to_owned(),
        },
        rig.content_sha256,
        active_receipt.expected_starting_campaign.sha256,
        active_receipt.expected_starting_campaign.byte_length,
        continuation_scope,
    );
    continuation_request.participant_claims[0].public_disclosure =
        ParticipantPublicDisclosureV1::Anonymous;

    let mut wrong_predecessor =
        rig.continuation_preflight_request(&continuation_request, &active_receipt, 164);
    wrong_predecessor.claim.predecessor_verification_sha256 = Digest32::from_bytes([0xe1; 32]);
    wrong_predecessor.host_signature = sign(
        &owner,
        &wrong_predecessor.claim.host_signing_bytes().unwrap(),
    );
    wrong_predecessor.controller_signature = sign(
        &owner,
        &wrong_predecessor.claim.controller_signing_bytes().unwrap(),
    );
    let wrong_predecessor_response = rig
        .app
        .clone()
        .oneshot(json_request(
            Method::POST,
            "/api/v1/campaign-continuation-preflight-grants",
            &wrong_predecessor,
            Ipv4Addr::new(127, 0, 4, 3),
        ))
        .await
        .unwrap();
    assert_eq!(wrong_predecessor_response.status(), StatusCode::CONFLICT);

    let mut wrong_artifact =
        rig.continuation_preflight_request(&continuation_request, &active_receipt, 165);
    wrong_artifact.claim.starting_campaign.sha256 = Digest32::from_bytes([0xe2; 32]);
    wrong_artifact.claim.ranked_session.starting_campaign_sha256 = Digest32::from_bytes([0xe2; 32]);
    wrong_artifact.host_signature =
        sign(&owner, &wrong_artifact.claim.host_signing_bytes().unwrap());
    wrong_artifact.controller_signature = sign(
        &owner,
        &wrong_artifact.claim.controller_signing_bytes().unwrap(),
    );
    wrong_artifact.validate().unwrap();
    let wrong_artifact_response = rig
        .app
        .clone()
        .oneshot(json_request(
            Method::POST,
            "/api/v1/campaign-continuation-preflight-grants",
            &wrong_artifact,
            Ipv4Addr::new(127, 0, 4, 4),
        ))
        .await
        .unwrap();
    assert_eq!(wrong_artifact_response.status(), StatusCode::CONFLICT);

    let mut authorized_for_adversarial_checks = continuation_request.clone();
    rig.authorize_continuation_request(
        &owner,
        &owner,
        &active_receipt,
        &mut authorized_for_adversarial_checks,
        166,
    )
    .await;
    let mut substituted_session = authorized_for_adversarial_checks.clone();
    substituted_session.session_genesis.claim.replay_session_id = Digest32::from_bytes([0xe3; 32]);
    substituted_session.session_genesis.host_signature = Some(sign(
        &owner,
        &substituted_session.session_genesis.signing_bytes().unwrap(),
    ));
    assert!(substituted_session.validate().is_err());

    let mut forged_authority = authorized_for_adversarial_checks.clone();
    forged_authority
        .session_genesis
        .claim
        .campaign_continuation_preflight_grant
        .as_mut()
        .unwrap()
        .authority_signature = Signature64::from_bytes([0xe4; 64]);
    forged_authority.session_genesis.host_signature = Some(sign(
        &owner,
        &forged_authority.session_genesis.signing_bytes().unwrap(),
    ));
    let forged_authority_response = rig
        .app
        .clone()
        .oneshot(json_request(
            Method::POST,
            "/api/v1/submission-offers",
            &forged_authority,
            Ipv4Addr::new(127, 0, 4, 5),
        ))
        .await
        .unwrap();
    assert_eq!(forged_authority_response.status(), StatusCode::UNAUTHORIZED);

    let mut expired = authorized_for_adversarial_checks;
    let expired_grant = expired
        .session_genesis
        .claim
        .campaign_continuation_preflight_grant
        .as_mut()
        .unwrap();
    expired_grant.claim.admitted_at_unix_ms = 1;
    expired_grant.claim.expires_at_unix_ms = 2;
    expired_grant.authority_signature = sign(
        &SigningKey::from_bytes(&[0x46; 32]),
        &expired_grant.signing_bytes().unwrap(),
    );
    expired.session_genesis.host_signature = Some(sign(
        &owner,
        &expired.session_genesis.signing_bytes().unwrap(),
    ));
    expired.validate().unwrap();
    let expired_response = rig
        .app
        .clone()
        .oneshot(json_request(
            Method::POST,
            "/api/v1/submission-offers",
            &expired,
            Ipv4Addr::new(127, 0, 4, 6),
        ))
        .await
        .unwrap();
    assert_eq!(expired_response.status(), StatusCode::CREATED);
    let delayed_offer: SubmissionOfferV1 = json_body(expired_response).await;
    delayed_offer.validate().unwrap();
    assert_eq!(delayed_offer.session_genesis, expired.session_genesis);
    assert!(delayed_offer.expires_at_unix_ms > 2);

    let continuation_offer = rig
        .issue_continuation_offer(
            &owner,
            &owner,
            &active_receipt,
            continuation_request.clone(),
            66,
        )
        .await;
    let field_accepted = rig
        .upload_campaign_offer(&owner, 66, continuation_offer)
        .await;
    let field_run_id = rig
        .publish_campaign(
            &field_accepted,
            CampaignSessionKindV1::FieldMission {
                mission_id: CAMPAIGN_MISSION_ID.to_owned(),
            },
            1,
            100,
            150,
            67,
            false,
        )
        .await;
    let field_receipt = rig.campaign_receipt(&field_accepted, &owner).await;
    assert_eq!(field_receipt.state, CampaignChainStateV1::Active);

    let terminal_scope = ScopeRequestV1::CampaignContinuation {
        chain_id: field_receipt.chain_id.clone(),
        predecessor_run_id: field_run_id.clone(),
    };
    let mut terminal_request = rig.campaign_offer_request(
        &owner,
        200,
        HQ_MISSION_ID,
        OfficialContentSubjectV1::FieldMission {
            mission_id: HQ_MISSION_ID.to_owned(),
        },
        rig.terminal_content_sha256.unwrap(),
        field_receipt.expected_starting_campaign.sha256,
        field_receipt.expected_starting_campaign.byte_length,
        terminal_scope,
    );
    terminal_request.participant_claims[0].public_disclosure =
        ParticipantPublicDisclosureV1::Anonymous;
    let terminal_offer = rig
        .issue_continuation_offer(
            &owner,
            &owner,
            &field_receipt,
            terminal_request.clone(),
            200,
        )
        .await;
    let terminal_accepted = rig.upload_campaign_offer(&owner, 200, terminal_offer).await;
    let terminal_run_id = rig
        .publish_campaign(
            &terminal_accepted,
            CampaignSessionKindV1::FieldMission {
                mission_id: HQ_MISSION_ID.to_owned(),
            },
            2,
            150,
            200,
            69,
            true,
        )
        .await;
    let complete_receipt = rig.campaign_receipt(&terminal_accepted, &owner).await;
    let CampaignChainStateV1::Complete {
        full_campaign_run_id,
    } = complete_receipt.state
    else {
        panic!("terminal H12 result did not create a full-campaign aggregate")
    };

    let campaign_mission_board_response = rig
        .app
        .clone()
        .oneshot(empty_request(
            Method::GET,
            &campaign_mission_leaderboard_uri(&rig, CAMPAIGN_MISSION_ID),
            Ipv4Addr::LOCALHOST,
        ))
        .await
        .unwrap();
    assert_eq!(campaign_mission_board_response.status(), StatusCode::OK);
    let campaign_mission_board: LeaderboardPageV1 =
        json_body(campaign_mission_board_response).await;
    campaign_mission_board
        .validate_against_ruleset(&rig.published_ruleset)
        .unwrap();
    assert_eq!(campaign_mission_board.entries[0].run_id, field_run_id);

    let full_board_response = rig
        .app
        .clone()
        .oneshot(empty_request(
            Method::GET,
            &full_campaign_leaderboard_uri(&rig),
            Ipv4Addr::LOCALHOST,
        ))
        .await
        .unwrap();
    assert_eq!(full_board_response.status(), StatusCode::OK);
    let full_board: LeaderboardPageV1 = json_body(full_board_response).await;
    full_board
        .validate_against_ruleset(&rig.published_ruleset)
        .unwrap();
    assert_eq!(full_board.entries.len(), 1);
    assert_eq!(full_board.entries[0].run_id, full_campaign_run_id);
    assert_public_projection_is_redacted(&full_board);
    assert_public_json_omits_private_chain_id(
        &serde_json::to_vec(&full_board).unwrap(),
        private_chain_id.as_str(),
    );

    let aggregate_storage = sqlx::query(
        "SELECT chain_id, aggregate_request_json, aggregate_json, \
                public_aggregate_request_json, public_aggregate_result_json, \
                public_projection_binding_json FROM full_campaign_runs WHERE id = ?",
    )
    .bind(full_campaign_run_id.as_str())
    .fetch_one(rig.database.fixture_pool())
    .await
    .unwrap();
    assert_eq!(
        aggregate_storage.get::<String, _>("chain_id"),
        private_chain_id.as_str()
    );
    for private_column in ["aggregate_request_json", "aggregate_json"] {
        assert!(
            aggregate_storage
                .get::<String, _>(private_column)
                .contains(private_chain_id.as_str()),
            "private aggregate column {private_column} must retain its chain binding"
        );
    }
    for public_column in [
        "public_aggregate_request_json",
        "public_aggregate_result_json",
        "public_projection_binding_json",
    ] {
        let stored: String = aggregate_storage.get(public_column);
        assert_public_json_omits_private_chain_id(stored.as_bytes(), private_chain_id.as_str());
    }

    let aggregate_response = rig
        .app
        .clone()
        .oneshot(empty_request(
            Method::GET,
            &format!("/api/v1/runs/{full_campaign_run_id}"),
            Ipv4Addr::LOCALHOST,
        ))
        .await
        .unwrap();
    let aggregate_status = aggregate_response.status();
    let aggregate_body = aggregate_response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    assert_eq!(
        aggregate_status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&aggregate_body)
    );
    assert_public_json_omits_private_participant_id(
        &aggregate_body,
        Digest32::from_bytes([63; 32]),
    );
    assert_public_json_omits_private_participant_id(
        &aggregate_body,
        Digest32::from_bytes([66; 32]),
    );
    assert_public_json_omits_private_participant_id(
        &aggregate_body,
        Digest32::from_bytes([200; 32]),
    );
    assert_public_json_omits_private_chain_id(&aggregate_body, private_chain_id.as_str());
    let aggregate: RunDetailV1 = serde_json::from_slice(&aggregate_body).unwrap();
    aggregate
        .validate_against_ruleset(
            &rig.published_ruleset,
            Some(rig.campaign_content.as_ref().unwrap()),
        )
        .unwrap();
    assert_eq!(aggregate.full_campaign_sessions.len(), 3);
    assert_eq!(aggregate.named_participant_instance_count, 0);
    assert_eq!(aggregate.anonymous_participant_instance_count, 3);
    assert!(aggregate.named_participants.is_empty());
    assert!(
        aggregate
            .full_campaign_sessions
            .iter()
            .all(|session| session.named_participants.is_empty())
    );
    assert_eq!(aggregate.full_campaign_sessions[0].run_id, genesis_run_id);
    assert_eq!(aggregate.full_campaign_sessions[1].run_id, field_run_id);
    assert_eq!(aggregate.full_campaign_sessions[2].run_id, terminal_run_id);
    assert_eq!(
        aggregate.full_campaign_sessions[1]
            .mission
            .as_ref()
            .unwrap()
            .mission_id,
        CAMPAIGN_MISSION_ID
    );
    assert_public_projection_is_redacted(&aggregate);

    for (role, expected) in [
        ("starting", rig.starting_campaign.clone()),
        ("final", vec![69_u8; 113]),
    ] {
        rig.database
            .public_campaign_for_run(full_campaign_run_id.as_str(), role)
            .await
            .unwrap();
        let response = rig
            .app
            .clone()
            .oneshot(empty_request(
                Method::GET,
                &format!("/api/v1/runs/{full_campaign_run_id}/campaigns/{role}"),
                Ipv4Addr::LOCALHOST,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_dynamic_headers(&response);
        assert_eq!(
            response.headers().get(CONTENT_TYPE).unwrap(),
            RANKED_CAMPAIGN_MEDIA_TYPE_V1
        );
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(bytes.as_ref(), expected.as_slice());
    }

    let hq_session_response = rig
        .app
        .clone()
        .oneshot(empty_request(
            Method::GET,
            &format!("/api/v1/runs/{full_campaign_run_id}/sessions/0"),
            Ipv4Addr::LOCALHOST,
        ))
        .await
        .unwrap();
    assert_eq!(hq_session_response.status(), StatusCode::OK);
    let hq_session_body = hq_session_response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    assert_public_json_omits_private_participant_id(
        &hq_session_body,
        Digest32::from_bytes([63; 32]),
    );
    assert_public_json_omits_private_chain_id(&hq_session_body, private_chain_id.as_str());
    let hq_session: CampaignSessionDetailV1 = serde_json::from_slice(&hq_session_body).unwrap();
    hq_session
        .validate_against_ruleset(
            &aggregate,
            &rig.published_ruleset,
            rig.campaign_content.as_ref().unwrap(),
        )
        .unwrap();
    assert_eq!(hq_session.session.run_id, genesis_run_id);
    assert_public_projection_is_redacted(&hq_session);

    let terminal_replay_response = rig
        .app
        .clone()
        .oneshot(empty_request(
            Method::GET,
            &format!("/api/v1/runs/{full_campaign_run_id}/sessions/2/replay"),
            Ipv4Addr::LOCALHOST,
        ))
        .await
        .unwrap();
    assert_eq!(terminal_replay_response.status(), StatusCode::OK);
    let replay = terminal_replay_response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    assert_eq!(replay, compact_replay_fixture("campaign-run-200"));
    let session_campaign_response = rig
        .app
        .clone()
        .oneshot(empty_request(
            Method::GET,
            &format!("/api/v1/runs/{full_campaign_run_id}/sessions/2/campaigns/final"),
            Ipv4Addr::LOCALHOST,
        ))
        .await
        .unwrap();
    assert_eq!(session_campaign_response.status(), StatusCode::OK);
    assert_dynamic_headers(&session_campaign_response);
    assert_eq!(
        session_campaign_response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .as_ref(),
        vec![69_u8; 113].as_slice()
    );
    for (path, expected_status) in [
        (format!("/api/v1/runs/{genesis_run_id}"), StatusCode::OK),
        (
            format!("/api/v1/runs/{genesis_run_id}/replay"),
            StatusCode::OK,
        ),
        (
            format!("/api/v1/runs/{genesis_run_id}/campaigns/starting"),
            StatusCode::OK,
        ),
        (
            format!("/api/v1/runs/{genesis_run_id}/campaigns/final"),
            StatusCode::OK,
        ),
        (
            format!("/api/v1/runs/{terminal_run_id}"),
            StatusCode::NOT_FOUND,
        ),
        (
            format!("/api/v1/runs/{terminal_run_id}/replay"),
            StatusCode::NOT_FOUND,
        ),
        (
            format!("/api/v1/runs/{terminal_run_id}/campaigns/starting"),
            StatusCode::NOT_FOUND,
        ),
        (
            format!("/api/v1/runs/{terminal_run_id}/campaigns/final"),
            StatusCode::NOT_FOUND,
        ),
    ] {
        let response = rig
            .app
            .clone()
            .oneshot(empty_request(Method::GET, &path, Ipv4Addr::LOCALHOST))
            .await
            .unwrap();
        assert_eq!(response.status(), expected_status, "{path}");
        assert_dynamic_headers(&response);
    }

    // Dynamic SQL interpolates only these fixed column names; all values are bound.
    for column in ["aggregate_request_json", "public_aggregate_result_json"] {
        let original: String = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT {column} FROM full_campaign_runs WHERE id = ?"
        )))
        .bind(full_campaign_run_id.as_str())
        .fetch_one(rig.database.fixture_pool())
        .await
        .unwrap();
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "UPDATE full_campaign_runs SET {column} = ? WHERE id = ?"
        )))
        .bind(format!(" {original}"))
        .bind(full_campaign_run_id.as_str())
        .execute(rig.database.fixture_pool())
        .await
        .unwrap();
        let corrupt_response = rig
            .app
            .clone()
            .oneshot(empty_request(
                Method::GET,
                &format!("/api/v1/runs/{full_campaign_run_id}"),
                Ipv4Addr::LOCALHOST,
            ))
            .await
            .unwrap();
        assert_eq!(corrupt_response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_dynamic_headers(&corrupt_response);
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "UPDATE full_campaign_runs SET {column} = ? WHERE id = ?"
        )))
        .bind(original)
        .bind(full_campaign_run_id.as_str())
        .execute(rig.database.fixture_pool())
        .await
        .unwrap();
    }

    let owner_key = protocol_public_key(&owner);
    let anonymous_history_response = rig
        .app
        .clone()
        .oneshot(empty_request(
            Method::GET,
            &format!("/api/v1/players/{owner_key}/runs?schema_version=1&limit=10"),
            Ipv4Addr::LOCALHOST,
        ))
        .await
        .unwrap();
    assert_eq!(anonymous_history_response.status(), StatusCode::OK);
    let anonymous_history: PlayerRunHistoryPageV1 = json_body(anonymous_history_response).await;
    assert!(anonymous_history.runs.is_empty());
    assert!(anonymous_history.personal_bests.is_empty());

    let quarantined = rig.app_with_ruleset_status(RulesetOperationalStatusV1::Quarantined {
        audit_id: OpaqueId::new("router-e2e-quarantine").unwrap(),
        reason_code: "verifier-audit".to_owned(),
        since_unix_ms: 1,
    });
    for path in [
        format!("/api/v1/runs/{genesis_run_id}"),
        format!("/api/v1/runs/{genesis_run_id}/replay"),
        format!("/api/v1/runs/{genesis_run_id}/campaigns/starting"),
        format!("/api/v1/runs/{genesis_run_id}/campaigns/final"),
        format!("/api/v1/runs/{full_campaign_run_id}"),
        format!("/api/v1/runs/{full_campaign_run_id}/replay"),
        format!("/api/v1/runs/{full_campaign_run_id}/campaigns/starting"),
        format!("/api/v1/runs/{full_campaign_run_id}/campaigns/final"),
        format!("/api/v1/runs/{full_campaign_run_id}/sessions/0"),
        format!("/api/v1/runs/{full_campaign_run_id}/sessions/0/replay"),
        format!("/api/v1/runs/{full_campaign_run_id}/sessions/0/campaigns/starting"),
        format!("/api/v1/runs/{full_campaign_run_id}/sessions/2/campaigns/final"),
    ] {
        let response = quarantined
            .clone()
            .oneshot(empty_request(Method::GET, &path, Ipv4Addr::LOCALHOST))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
        assert_dynamic_headers(&response);
    }
    let history_response = quarantined
        .clone()
        .oneshot(empty_request(
            Method::GET,
            &format!("/api/v1/players/{owner_key}/runs?schema_version=1&limit=10"),
            Ipv4Addr::LOCALHOST,
        ))
        .await
        .unwrap();
    assert_eq!(history_response.status(), StatusCode::OK);
    let history: PlayerRunHistoryPageV1 = json_body(history_response).await;
    assert!(history.runs.is_empty());
    assert!(history.personal_bests.is_empty());

    for run_id in [&genesis_run_id, &full_campaign_run_id] {
        let response = quarantined
            .clone()
            .oneshot(json_request(
                Method::POST,
                "/api/v1/reports",
                &AbuseReportV1 {
                    schema_version: SCHEMA_VERSION_V1,
                    target: AbuseReportTargetV1::Run {
                        run_id: run_id.clone(),
                    },
                    category: AbuseReportCategoryV1::Other,
                    detail: "must not confirm a quarantined target".to_owned(),
                },
                Ipv4Addr::new(127, 30, 0, 1),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_dynamic_headers(&response);
    }

    let deletion = rig.delete_run(&owner, &full_campaign_run_id).await;
    assert_eq!(
        deletion.target,
        DeletionTargetV1::Run {
            run_id: full_campaign_run_id.clone()
        }
    );
    let retained_private_receipt = rig.campaign_receipt(&terminal_accepted, &owner).await;
    assert_eq!(
        retained_private_receipt.state,
        CampaignChainStateV1::Complete {
            full_campaign_run_id: full_campaign_run_id.clone()
        }
    );
    rig.delete_run(&owner, &genesis_run_id).await;
    assert_eq!(
        rig.campaign_receipt(&genesis_accepted, &owner).await.state,
        CampaignChainStateV1::Complete {
            full_campaign_run_id: full_campaign_run_id.clone()
        }
    );
    assert_eq!(
        rig.campaign_receipt(&terminal_accepted, &owner).await.state,
        CampaignChainStateV1::Complete {
            full_campaign_run_id: full_campaign_run_id.clone()
        }
    );
    for path in [
        format!("/api/v1/runs/{terminal_run_id}"),
        format!("/api/v1/runs/{terminal_run_id}/replay"),
        format!("/api/v1/runs/{terminal_run_id}/campaigns/starting"),
        format!("/api/v1/runs/{terminal_run_id}/campaigns/final"),
        format!("/api/v1/runs/{full_campaign_run_id}"),
        format!("/api/v1/runs/{full_campaign_run_id}/campaigns/starting"),
        format!("/api/v1/runs/{full_campaign_run_id}/campaigns/final"),
        format!("/api/v1/runs/{full_campaign_run_id}/sessions/0"),
        format!("/api/v1/runs/{full_campaign_run_id}/sessions/0/replay"),
        format!("/api/v1/runs/{full_campaign_run_id}/sessions/0/campaigns/starting"),
    ] {
        let response = rig
            .app
            .clone()
            .oneshot(empty_request(Method::GET, &path, Ipv4Addr::LOCALHOST))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
        assert_dynamic_headers(&response);
    }
    let denied_continuation_request = rig.campaign_offer_request(
        &owner,
        72,
        CAMPAIGN_MISSION_ID,
        OfficialContentSubjectV1::FieldMission {
            mission_id: CAMPAIGN_MISSION_ID.to_owned(),
        },
        rig.content_sha256,
        retained_private_receipt.expected_starting_campaign.sha256,
        retained_private_receipt
            .expected_starting_campaign
            .byte_length,
        ScopeRequestV1::CampaignContinuation {
            chain_id: retained_private_receipt.chain_id.clone(),
            predecessor_run_id: terminal_run_id,
        },
    );
    let mut denied_preflight = rig.continuation_preflight_request(
        &denied_continuation_request,
        &retained_private_receipt,
        72,
    );
    denied_preflight.host_signature = sign(
        &owner,
        &denied_preflight.claim.host_signing_bytes().unwrap(),
    );
    denied_preflight.controller_signature = sign(
        &owner,
        &denied_preflight.claim.controller_signing_bytes().unwrap(),
    );
    let denied_response = rig
        .app
        .clone()
        .oneshot(json_request(
            Method::POST,
            "/api/v1/campaign-continuation-preflight-grants",
            &denied_preflight,
            Ipv4Addr::new(127, 40, 0, 3),
        ))
        .await
        .unwrap();
    assert_eq!(denied_response.status(), StatusCode::NOT_FOUND);
}
