use crate::support::*;

#[tokio::test]
async fn custom_config_preflight_is_admitted_only_by_an_open_ruleset_and_digest_bound() {
    for any_ruleset in [false, true] {
        let rig = TestRig::new_with(RigOptions {
            any_ruleset,
            ..RigOptions::default()
        })
        .await;
        let owner = SigningKey::from_bytes(&[80; 32]);
        rig.rename(&owner, "Custom Robin", Ipv4Addr::new(127, 0, 8, 1))
            .await;
        let mut request = rig.offer_request(&owner, 80);
        let baseline = rig
            .config
            .manifests
            .rules_configs
            .get(&rig.rules_config_sha256)
            .unwrap();
        let mut sim =
            robin_engine::engine::RankedSimulationPolicy::standard_medium().expected_config();
        sim.difficulty = robin_engine::player_profile::DifficultyLevel::Legendary;
        sim.enable_unbinding = false;
        let custom =
            robin_engine::simulation_inputs::custom_rules_config_v1(baseline, sim).unwrap();
        let ranked = &mut request.session_genesis.claim.ranked_session;
        ranked.rules_config_sha256 = custom.canonical_digest().unwrap();
        ranked.custom_rules_config = Some(custom);
        ranked.custom_canonical_campaign = Some(ArtifactRefV1 {
            sha256: ranked.starting_campaign_sha256,
            byte_length: ranked.starting_campaign_byte_length,
            media_type: RANKED_CAMPAIGN_MEDIA_TYPE_V1.into(),
        });
        let mut preflight = rig.fresh_preflight_request(&request, 80).unwrap();
        preflight.host_signature = sign(&owner, &preflight.signing_bytes().unwrap());
        let response = rig
            .app
            .clone()
            .oneshot(json_request(
                Method::POST,
                "/api/v1/fresh-run-preflight-grants",
                &preflight,
                Ipv4Addr::new(127, 8, 2, 80),
            ))
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            if any_ruleset {
                StatusCode::CREATED
            } else {
                StatusCode::BAD_REQUEST
            }
        );
        if any_ruleset {
            let grant: FreshRunPreflightGrantV1 = json_body(response).await;
            grant.validate_request(&preflight).unwrap();
            preflight
                .claim
                .ranked_session
                .custom_rules_config
                .as_mut()
                .unwrap()
                .sim_config
                .insert("enable_unbinding".into(), CanonicalValue::Bool(true));
            assert!(preflight.validate().is_err());
            assert!(grant.validate_request(&preflight).is_err());
        }
    }
}

#[tokio::test]
async fn fresh_run_preflight_is_exact_signed_time_bounded_and_server_authorized() {
    let rig = TestRig::new().await;
    let owner = SigningKey::from_bytes(&[81; 32]);
    rig.rename(&owner, "Preflight Robin", Ipv4Addr::new(127, 0, 8, 1))
        .await;
    let offer_request = rig.offer_request(&owner, 81);

    let mut forged = rig
        .fresh_preflight_request(&offer_request, 81)
        .expect("fresh individual run needs preflight");
    forged.host_signature = sign(
        &SigningKey::from_bytes(&[82; 32]),
        &forged.signing_bytes().unwrap(),
    );
    let response = rig
        .app
        .clone()
        .oneshot(json_request(
            Method::POST,
            "/api/v1/fresh-run-preflight-grants",
            &forged,
            Ipv4Addr::new(127, 8, 1, 81),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let mut wrong_campaign = rig
        .fresh_preflight_request(&offer_request, 82)
        .expect("fresh individual run needs preflight");
    wrong_campaign.claim.starting_campaign.sha256 = Digest32::from_bytes([91; 32]);
    wrong_campaign.claim.ranked_session.starting_campaign_sha256 = Digest32::from_bytes([91; 32]);
    wrong_campaign.host_signature = sign(&owner, &wrong_campaign.signing_bytes().unwrap());
    wrong_campaign.validate().unwrap();
    let response = rig
        .app
        .clone()
        .oneshot(json_request(
            Method::POST,
            "/api/v1/fresh-run-preflight-grants",
            &wrong_campaign,
            Ipv4Addr::new(127, 8, 1, 82),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);

    let mut preflight = rig
        .fresh_preflight_request(&offer_request, 83)
        .expect("fresh individual run needs preflight");
    preflight.host_signature = sign(&owner, &preflight.signing_bytes().unwrap());
    let response = rig
        .app
        .clone()
        .oneshot(json_request(
            Method::POST,
            "/api/v1/fresh-run-preflight-grants",
            &preflight,
            Ipv4Addr::new(127, 8, 1, 83),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let grant: FreshRunPreflightGrantV1 = json_body(response).await;
    grant.validate_request(&preflight).unwrap();

    let mut bad_signature_request = offer_request.clone();
    let mut bad_signature_grant = grant.clone();
    bad_signature_grant.authority_signature = Signature64::from_bytes([92; 64]);
    bad_signature_request
        .session_genesis
        .claim
        .fresh_run_preflight_grant = Some(bad_signature_grant);
    bad_signature_request.session_genesis.host_signature = sign(
        &owner,
        &bad_signature_request
            .session_genesis
            .signing_bytes()
            .unwrap(),
    );
    bad_signature_request.validate().unwrap();
    let response = rig
        .app
        .clone()
        .oneshot(json_request(
            Method::POST,
            "/api/v1/submission-offers",
            &bad_signature_request,
            Ipv4Addr::new(127, 8, 1, 84),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let mut expired_request = offer_request;
    let mut expired_grant = grant;
    expired_grant.claim.admitted_at_unix_ms = 1;
    expired_grant.claim.expires_at_unix_ms = 2;
    expired_grant.authority_signature = sign(
        &SigningKey::from_bytes(&[0x46; 32]),
        &expired_grant.signing_bytes().unwrap(),
    );
    expired_request
        .session_genesis
        .claim
        .fresh_run_preflight_grant = Some(expired_grant);
    expired_request.session_genesis.host_signature = sign(
        &owner,
        &expired_request.session_genesis.signing_bytes().unwrap(),
    );
    expired_request.validate().unwrap();
    let response = rig
        .app
        .clone()
        .oneshot(json_request(
            Method::POST,
            "/api/v1/submission-offers",
            &expired_request,
            Ipv4Addr::new(127, 8, 1, 85),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
}
