use crate::test_fixtures::campaign_artifact;
use crate::test_fixtures::id;

use super::*;
use crate::RulesetManifestV1;
fn replay_artifact(byte: u8) -> ReplayArtifactV1 {
    crate::test_fixtures::replay_artifact(byte, 123)
}
fn submission_artifacts() -> crate::SubmissionArtifactsV1 {
    crate::test_fixtures::submission_artifacts(replay_artifact(1), campaign_artifact(8, 321))
}

fn ranked_session() -> RankedSessionConfigV1 {
    RankedSessionConfigV1 {
        custom_rules_config: None,
        custom_canonical_campaign: None,
        schema_version: 1,
        mission_id: "Dem_Lei_MP".into(),
        content_edition: OfficialContentEditionV1::Demo,
        content_subject: OfficialContentSubjectV1::FieldMission {
            mission_id: "Dem_Lei_MP".into(),
        },
        simulation_seed: SimulationSeed64::new(42),
        starting_campaign_sha256: Digest32::from_bytes([8; 32]),
        starting_campaign_byte_length: 321,
        prepared_inputs_projection_sha256: Digest32::from_bytes([18; 32]),
        prepared_mission_inputs_seal_sha256: Digest32::from_bytes([19; 32]),
        build_manifest_sha256: Digest32::from_bytes([4; 32]),
        content_manifest_sha256: Digest32::from_bytes([5; 32]),
        campaign_content_manifest_sha256: None,
        rules_config_sha256: Digest32::from_bytes([6; 32]),
        ruleset_manifest_sha256: Digest32::from_bytes([7; 32]),
        competition_manifest_sha256: None,
        spellforge_content_sha256: None,
        resource_locale_root: ResourceLocaleRootV1::new("1033").unwrap(),
        speech_timing: SpeechTimingAuthorityV1::LanguagePack {
            canonical_locale: "en-US".into(),
        },
    }
}
fn fresh_preflight_grant(
    ranked: &RankedSessionConfigV1,
    scope: FreshRunScopeV1,
) -> FreshRunPreflightGrantV1 {
    crate::test_fixtures::fresh_preflight_grant(
        ranked,
        scope,
        20,
        3,
        11,
        13,
        campaign_artifact(8, 321),
        1_700_000_000_000,
        1_800_000_000_000,
    )
}

fn continuation_preflight_grant(
    genesis: &ReplaySessionGenesisClaimV1,
    chain_id: OpaqueId,
    predecessor_run_id: OpaqueId,
    predecessor_verification_sha256: Digest32,
    campaign_controller_public_key: PublicKey32,
    participant_public_keys: Vec<PublicKey32>,
    max_concurrent_players: u16,
) -> CampaignContinuationPreflightGrantV1 {
    CampaignContinuationPreflightGrantV1 {
        claim: CampaignContinuationPreflightGrantClaimV1 {
            schema_version: 1,
            grant_id: id("continuation-grant-1"),
            grant_nonce: ChallengeNonce32::from_bytes([40; 32]),
            grant_authority_public_key: PublicKey32::from_bytes([21; 32]),
            grant_request_sha256: Digest32::from_bytes([41; 32]),
            ranked_session_sha256: genesis.ranked_session.canonical_digest().unwrap(),
            host_public_key: genesis.host_public_key,
            campaign_controller_public_key,
            replay_session_id: genesis.replay_session_id,
            host_participant_instance_id: genesis.host_participant_instance_id,
            host_nonce: genesis.host_nonce,
            max_concurrent_players,
            participant_public_keys,
            chain_id,
            predecessor_run_id,
            predecessor_verification_sha256,
            starting_campaign: ArtifactRefV1 {
                sha256: genesis.ranked_session.starting_campaign_sha256,
                byte_length: genesis.ranked_session.starting_campaign_byte_length,
                media_type: RANKED_CAMPAIGN_MEDIA_TYPE_V1.into(),
            },
            admitted_at_unix_ms: 1,
            expires_at_unix_ms: 1_800_000_000_000,
        },
        algorithm: SignatureAlgorithmV1::Ed25519,
        authority_signature: Signature64::from_bytes([42; 64]),
    }
}

fn session_genesis() -> ReplaySessionGenesisV1 {
    let ranked_session = ranked_session();
    let fresh_run_preflight_grant = Some(fresh_preflight_grant(
        &ranked_session,
        FreshRunScopeV1::IndividualLevel,
    ));
    ReplaySessionGenesisV1 {
        claim: ReplaySessionGenesisClaimV1 {
            schema_version: 1,
            network_protocol_version: crate::CURRENT_RANKED_NETWORK_PROTOCOL_VERSION_V1,
            host_public_key: PublicKey32::from_bytes([3; 32]),
            replay_session_id: Digest32::from_bytes([11; 32]),
            host_participant_instance_id: Digest32::from_bytes([12; 32]),
            host_nonce: ChallengeNonce32::from_bytes([13; 32]),
            ranked_session,
            fresh_run_preflight_grant,
            campaign_continuation_preflight_grant: None,
            competition_run_grant: None,
        },
        algorithm: SignatureAlgorithmV1::Ed25519,
        host_signature: Signature64::from_bytes([14; 64]),
    }
}

#[test]
fn missing_grant_fields_are_not_compatibility_lanes() {
    let genesis = session_genesis();
    let mut missing = serde_json::to_value(&genesis).unwrap();
    missing
        .get_mut("claim")
        .unwrap()
        .as_object_mut()
        .unwrap()
        .remove("competition_run_grant");
    assert!(serde_json::from_value::<ReplaySessionGenesisV1>(missing).is_err());

    let mut missing = serde_json::to_value(&genesis).unwrap();
    missing
        .get_mut("claim")
        .unwrap()
        .as_object_mut()
        .unwrap()
        .remove("fresh_run_preflight_grant");
    assert!(serde_json::from_value::<ReplaySessionGenesisV1>(missing).is_err());

    let mut missing = serde_json::to_value(&genesis).unwrap();
    missing
        .get_mut("claim")
        .unwrap()
        .as_object_mut()
        .unwrap()
        .remove("campaign_continuation_preflight_grant");
    assert!(serde_json::from_value::<ReplaySessionGenesisV1>(missing).is_err());

    let encoded = serde_json::to_value(genesis).unwrap();
    assert!(encoded["claim"]["competition_run_grant"].is_null());
    assert!(encoded["claim"]["fresh_run_preflight_grant"].is_object());
    assert!(encoded["claim"]["campaign_continuation_preflight_grant"].is_null());

    let mut both = session_genesis();
    both.claim.campaign_continuation_preflight_grant = Some(continuation_preflight_grant(
        &both.claim,
        id("chain-1"),
        id("run-1"),
        Digest32::from_bytes([47; 32]),
        both.claim.host_public_key,
        vec![both.claim.host_public_key],
        1,
    ));
    assert!(matches!(
        both.validate(),
        Err(ValidationError::ClaimMismatch {
            field: "session_genesis.run_preflight_grant_presence"
        })
    ));

    // Wire decoding preserves populated grants independently of the
    // mutually exclusive scope checks above. Each nullable field must be
    // present, including when it explicitly carries no grant.
    both.claim.competition_run_grant = Some(CompetitionRunGrantV1 {
        claim: CompetitionRunGrantClaimV1 {
            schema_version: 1,
            grant_id: id("competition-grant"),
            grant_nonce: ChallengeNonce32::from_bytes([1; 32]),
            grant_authority_public_key: PublicKey32::from_bytes([2; 32]),
            host_public_key: both.claim.host_public_key,
            competition_manifest_sha256: Digest32::from_bytes([3; 32]),
            ranked_session_sha256: Digest32::from_bytes([4; 32]),
            grant_request_sha256: Digest32::from_bytes([5; 32]),
            replay_session_id: both.claim.replay_session_id,
            host_participant_instance_id: both.claim.host_participant_instance_id,
            host_nonce: both.claim.host_nonce,
            admitted_at_unix_ms: 1,
            expires_at_unix_ms: 2,
        },
        algorithm: SignatureAlgorithmV1::Ed25519,
        authority_signature: Signature64::from_bytes([6; 64]),
    });
    let populated = serde_json::to_value(&both).unwrap();
    assert_eq!(
        serde_json::from_value::<ReplaySessionGenesisV1>(populated.clone()).unwrap(),
        both
    );
    for field in [
        "fresh_run_preflight_grant",
        "campaign_continuation_preflight_grant",
        "competition_run_grant",
    ] {
        assert!(populated["claim"][field].is_object());
        let mut explicit_null = populated.clone();
        explicit_null["claim"][field] = serde_json::Value::Null;
        let decoded: ReplaySessionGenesisV1 =
            serde_json::from_value(explicit_null.clone()).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), explicit_null);

        let mut missing = populated.clone();
        missing["claim"].as_object_mut().unwrap().remove(field);
        let error = serde_json::from_value::<ReplaySessionGenesisV1>(missing).unwrap_err();
        assert!(
            error
                .to_string()
                .contains(&format!("missing field `{field}`"))
        );
    }
}

#[test]
fn fresh_run_preflight_grant_is_exactly_bound_and_not_scope_substitutable() {
    let ranked = ranked_session();
    let request = FreshRunPreflightRequestV1 {
        claim: FreshRunPreflightRequestClaimV1 {
            schema_version: 1,
            request_nonce: ChallengeNonce32::from_bytes([31; 32]),
            host_public_key: PublicKey32::from_bytes([3; 32]),
            replay_session_id: Digest32::from_bytes([11; 32]),
            host_participant_instance_id: Digest32::from_bytes([12; 32]),
            host_nonce: ChallengeNonce32::from_bytes([13; 32]),
            scope: FreshRunScopeV1::IndividualLevel,
            starting_campaign: campaign_artifact(8, 321),
            ranked_session: ranked.clone(),
        },
        algorithm: SignatureAlgorithmV1::Ed25519,
        host_signature: Signature64::from_bytes([32; 64]),
    };
    request.validate().unwrap();

    let mut grant = fresh_preflight_grant(&ranked, FreshRunScopeV1::IndividualLevel);
    grant.claim.grant_request_sha256 = request.canonical_digest().unwrap();
    grant.validate_request(&request).unwrap();

    let mut wrong_scope = grant.clone();
    wrong_scope.claim.scope = FreshRunScopeV1::CampaignGenesis;
    assert!(matches!(
        wrong_scope.validate_request(&request),
        Err(ValidationError::ClaimMismatch {
            field: "fresh_run_preflight_grant.request_binding"
        })
    ));

    let mut wrong_session = grant.clone();
    wrong_session.claim.replay_session_id = Digest32::from_bytes([33; 32]);
    assert!(wrong_session.validate_request(&request).is_err());

    let mut wrong_artifact = grant.clone();
    wrong_artifact.claim.starting_campaign.sha256 = Digest32::from_bytes([34; 32]);
    assert!(wrong_artifact.validate_request(&request).is_err());

    let mut wrong_ranked_tuple = grant;
    wrong_ranked_tuple.claim.ranked_session_sha256 = Digest32::from_bytes([35; 32]);
    assert!(wrong_ranked_tuple.validate_request(&request).is_err());
}

#[test]
fn continuation_preflight_is_dual_signed_and_rejects_tuple_substitution() {
    let mut ranked = ranked_session();
    ranked.content_edition = OfficialContentEditionV1::Full;
    ranked.campaign_content_manifest_sha256 = Some(Digest32::from_bytes([36; 32]));
    let request = CampaignContinuationPreflightRequestV1 {
        claim: CampaignContinuationPreflightRequestClaimV1 {
            schema_version: 1,
            request_nonce: ChallengeNonce32::from_bytes([37; 32]),
            host_public_key: PublicKey32::from_bytes([3; 32]),
            campaign_controller_public_key: PublicKey32::from_bytes([4; 32]),
            replay_session_id: Digest32::from_bytes([11; 32]),
            host_participant_instance_id: Digest32::from_bytes([12; 32]),
            host_nonce: ChallengeNonce32::from_bytes([13; 32]),
            max_concurrent_players: 2,
            participant_public_keys: vec![
                PublicKey32::from_bytes([3; 32]),
                PublicKey32::from_bytes([4; 32]),
            ],
            chain_id: id("chain-1"),
            predecessor_run_id: id("run-1"),
            predecessor_verification_sha256: Digest32::from_bytes([38; 32]),
            starting_campaign: campaign_artifact(8, 321),
            ranked_session: ranked.clone(),
        },
        algorithm: SignatureAlgorithmV1::Ed25519,
        host_signature: Signature64::from_bytes([39; 64]),
        controller_signature: Signature64::from_bytes([40; 64]),
    };
    request.validate().unwrap();
    assert_ne!(
        request.claim.host_signing_bytes().unwrap(),
        request.claim.controller_signing_bytes().unwrap()
    );
    let grant = CampaignContinuationPreflightGrantV1 {
        claim: CampaignContinuationPreflightGrantClaimV1 {
            schema_version: 1,
            grant_id: id("continuation-grant-2"),
            grant_nonce: ChallengeNonce32::from_bytes([41; 32]),
            grant_authority_public_key: PublicKey32::from_bytes([42; 32]),
            grant_request_sha256: request.canonical_digest().unwrap(),
            ranked_session_sha256: ranked.canonical_digest().unwrap(),
            host_public_key: request.claim.host_public_key,
            campaign_controller_public_key: request.claim.campaign_controller_public_key,
            replay_session_id: request.claim.replay_session_id,
            host_participant_instance_id: request.claim.host_participant_instance_id,
            host_nonce: request.claim.host_nonce,
            max_concurrent_players: request.claim.max_concurrent_players,
            participant_public_keys: request.claim.participant_public_keys.clone(),
            chain_id: request.claim.chain_id.clone(),
            predecessor_run_id: request.claim.predecessor_run_id.clone(),
            predecessor_verification_sha256: request.claim.predecessor_verification_sha256,
            starting_campaign: request.claim.starting_campaign.clone(),
            admitted_at_unix_ms: 1_700_000_000_000,
            expires_at_unix_ms: 1_800_000_000_000,
        },
        algorithm: SignatureAlgorithmV1::Ed25519,
        authority_signature: Signature64::from_bytes([43; 64]),
    };
    grant.validate_request(&request).unwrap();

    let mut substituted = request.clone();
    substituted.claim.replay_session_id = Digest32::from_bytes([44; 32]);
    assert!(grant.validate_request(&substituted).is_err());
    let mut substituted = request.clone();
    substituted.claim.campaign_controller_public_key = PublicKey32::from_bytes([5; 32]);
    substituted.claim.participant_public_keys[1] = PublicKey32::from_bytes([5; 32]);
    assert!(grant.validate_request(&substituted).is_err());
    let mut substituted = request.clone();
    substituted.claim.predecessor_verification_sha256 = Digest32::from_bytes([45; 32]);
    assert!(grant.validate_request(&substituted).is_err());
    let mut substituted = request;
    substituted.claim.starting_campaign.sha256 = Digest32::from_bytes([46; 32]);
    substituted.claim.ranked_session.starting_campaign_sha256 = Digest32::from_bytes([46; 32]);
    assert!(grant.validate_request(&substituted).is_err());
}

#[test]
fn fresh_scopes_require_a_preflight_grant_and_continuations_forbid_one() {
    let mut fresh = offer();
    fresh.session_genesis.claim.fresh_run_preflight_grant = None;
    assert!(matches!(
        fresh.validate(),
        Err(ValidationError::ClaimMismatch {
            field: "submission_offer.run_preflight_grant_presence"
        })
    ));

    let mut wrong_scope = offer();
    wrong_scope
        .session_genesis
        .claim
        .fresh_run_preflight_grant
        .as_mut()
        .unwrap()
        .claim
        .scope = FreshRunScopeV1::CampaignGenesis;
    assert!(matches!(
        wrong_scope.validate(),
        Err(ValidationError::ClaimMismatch {
            field: "fresh_run_preflight_grant.offer_binding"
        })
    ));
}
fn host_claim() -> ParticipantClaimV1 {
    crate::test_fixtures::host_claim(3)
}

fn single_player_transcript(genesis_sha256: Digest32) -> ReplaySessionTranscriptV1 {
    ReplaySessionTranscriptV1 {
        schema_version: 1,
        session_genesis_sha256: genesis_sha256,
        replay_session_id: Digest32::from_bytes([11; 32]),
        host_participant_instance_id: Digest32::from_bytes([12; 32]),
        participant_instance_count: 1,
        max_concurrent_players: 1,
        events: vec![ReplaySeatLifecycleEventV1 {
            event_ordinal: 0,
            replay_ordinal: 0,
            seat: 0,
            participant_instance_id: Digest32::from_bytes([12; 32]),
            lifecycle: ReplaySeatLifecycleKindV1::Connected {
                connection_epoch: 0,
            },
        }],
    }
}

fn verified_run(genesis_sha256: Digest32) -> VerifiedRunV1 {
    VerifiedRunV1 {
        scope_kind: RunScopeKindV1::IndividualLevel,
        campaign_aggregation_consent: CampaignAggregationConsentV1::NotAuthorized,
        campaign_session_kind: None,
        campaign_session_ordinal: None,
        max_concurrent_players: 1,
        participant_instance_count: 1,
        named_participant_instance_count: 1,
        anonymous_participant_instance_count: 0,
        authenticated_participant_claims: vec![host_claim()],
        replay_session_transcript: single_player_transcript(genesis_sha256),
        outcome: TerminalOutcomeV1::Won,
        starting_campaign: campaign_artifact(8, 321),
        final_campaign: campaign_artifact(7, 400),
        starting_campaign_score: 0,
        final_campaign_score: 1_000,
        final_state_sha256: Digest32::from_bytes([8; 32]),
        replay_frames: 100,
        original_score_delta: 1_000,
        active_simulation_ticks: 90,
        ransom_collected: 250,
        campaign_complete_evidence: None,
        achievements: Vec::new(),
        diagnostics: BTreeMap::new(),
    }
}

fn guest_attestation(genesis: &ReplaySessionGenesisV1) -> NamedSeatJoinAttestationV1 {
    NamedSeatJoinAttestationV1 {
        claim: NamedSeatJoinClaimV1 {
            schema_version: 1,
            session_genesis_sha256: genesis.canonical_digest().unwrap(),
            public_key: PublicKey32::from_bytes([16; 32]),
            transport_endpoint_id: PublicKey32::from_bytes([17; 32]),
            host_endpoint_id: genesis.claim.host_public_key,
            replay_session_id: genesis.claim.replay_session_id,
            participant_instance_id: Digest32::from_bytes([15; 32]),
            seat: 1,
            connection_epoch: 0,
            join_event_ordinal: 1,
            mission_id: genesis.claim.ranked_session.mission_id.clone(),
            content_manifest_sha256: genesis.claim.ranked_session.content_manifest_sha256,
            rules_config_sha256: genesis.claim.ranked_session.rules_config_sha256,
            ruleset_manifest_sha256: genesis.claim.ranked_session.ruleset_manifest_sha256,
            competition_manifest_sha256: genesis.claim.ranked_session.competition_manifest_sha256,
            host_nonce: genesis.claim.host_nonce,
        },
        algorithm: SignatureAlgorithmV1::Ed25519,
        signature: Signature64::from_bytes([17; 64]),
    }
}

fn multiplayer_transcript(genesis: &ReplaySessionGenesisV1) -> ReplaySessionTranscriptV1 {
    ReplaySessionTranscriptV1 {
        schema_version: 1,
        session_genesis_sha256: genesis.canonical_digest().unwrap(),
        replay_session_id: genesis.claim.replay_session_id,
        host_participant_instance_id: genesis.claim.host_participant_instance_id,
        participant_instance_count: 2,
        max_concurrent_players: 2,
        events: vec![
            ReplaySeatLifecycleEventV1 {
                event_ordinal: 0,
                replay_ordinal: 0,
                seat: 0,
                participant_instance_id: genesis.claim.host_participant_instance_id,
                lifecycle: ReplaySeatLifecycleKindV1::Connected {
                    connection_epoch: 0,
                },
            },
            ReplaySeatLifecycleEventV1 {
                event_ordinal: 1,
                replay_ordinal: 1,
                seat: 1,
                participant_instance_id: Digest32::from_bytes([15; 32]),
                lifecycle: ReplaySeatLifecycleKindV1::Connected {
                    connection_epoch: 0,
                },
            },
            ReplaySeatLifecycleEventV1 {
                event_ordinal: 2,
                replay_ordinal: 10,
                seat: 1,
                participant_instance_id: Digest32::from_bytes([15; 32]),
                lifecycle: ReplaySeatLifecycleKindV1::Disconnected,
            },
            ReplaySeatLifecycleEventV1 {
                event_ordinal: 3,
                replay_ordinal: 20,
                seat: 1,
                participant_instance_id: Digest32::from_bytes([15; 32]),
                lifecycle: ReplaySeatLifecycleKindV1::Connected {
                    connection_epoch: 1,
                },
            },
        ],
    }
}

fn offer() -> SubmissionOfferV1 {
    SubmissionOfferV1 {
        schema_version: 1,
        upload_challenge_id: id("challenge-1"),
        upload_challenge_nonce: ChallengeNonce32::from_bytes([2; 32]),
        expires_at_unix_ms: 1_800_000_000_000,
        max_concurrent_players: 1,
        participant_instance_count: 1,
        participant_claims: vec![host_claim()],
        session_genesis: session_genesis(),
        mission_id: "Dem_Lei_MP".into(),
        competition_manifest_sha256: None,
        build_manifest_sha256: Digest32::from_bytes([4; 32]),
        content_manifest_sha256: Digest32::from_bytes([5; 32]),
        rules_config_sha256: Digest32::from_bytes([6; 32]),
        ruleset_manifest_sha256: Digest32::from_bytes([7; 32]),
        starting_state: InitialStateExpectationV1::IndividualLevel {
            template_id: id("leicester-default"),
            campaign_state_requirement: crate::CanonicalCampaignStateRequirementV1 {
                edition: crate::OfficialContentEditionV1::Demo,
                kind: crate::CanonicalCampaignStateKindV1::IndividualTemplate,
                rules_config_sha256: Digest32::from_bytes([6; 32]),
            },
            campaign_sha256: Digest32::from_bytes([8; 32]),
            starting_campaign_byte_length: 321,
        },
        allowed_metrics: vec![BoardMetricV1::OriginalScore, BoardMetricV1::FastestSuccess],
    }
}

fn grant_binding_request(offer: &SubmissionOfferV1) -> SubmissionOfferRequestV1 {
    SubmissionOfferRequestV1 {
        schema_version: offer.schema_version,
        max_concurrent_players: offer.max_concurrent_players,
        participant_instance_count: offer.participant_instance_count,
        participant_claims: offer.participant_claims.clone(),
        session_genesis: offer.session_genesis.clone(),
        mission_id: offer.mission_id.clone(),
        scope_request: match &offer.starting_state {
            InitialStateExpectationV1::IndividualLevel { .. } => ScopeRequestV1::IndividualLevel,
            InitialStateExpectationV1::CampaignGenesis { .. } => ScopeRequestV1::CampaignGenesis,
            InitialStateExpectationV1::CampaignContinuation {
                chain_id,
                predecessor_run_id,
                ..
            } => ScopeRequestV1::CampaignContinuation {
                chain_id: chain_id.clone(),
                predecessor_run_id: predecessor_run_id.clone(),
            },
        },
        ruleset_manifest_sha256: offer.ruleset_manifest_sha256,
        competition_manifest_sha256: offer.competition_manifest_sha256,
    }
}

fn continuation_binding_offer() -> SubmissionOfferV1 {
    let mut offer = offer();
    let InitialStateExpectationV1::IndividualLevel {
        campaign_state_requirement,
        campaign_sha256,
        starting_campaign_byte_length,
        ..
    } = offer.starting_state
    else {
        unreachable!()
    };
    offer.starting_state = InitialStateExpectationV1::CampaignContinuation {
        chain_id: id("chain-1"),
        predecessor_run_id: id("predecessor-1"),
        predecessor_verification_sha256: Digest32::from_bytes([17; 32]),
        campaign_state_requirement,
        campaign_sha256,
        starting_campaign_byte_length,
    };
    offer.session_genesis.claim.fresh_run_preflight_grant = None;
    offer
        .session_genesis
        .claim
        .campaign_continuation_preflight_grant = Some(continuation_preflight_grant(
        &offer.session_genesis.claim,
        id("chain-1"),
        id("predecessor-1"),
        Digest32::from_bytes([17; 32]),
        host_claim().public_key,
        vec![host_claim().public_key],
        1,
    ));
    offer
}

fn assert_grant_binding_error(result: Result<(), ValidationError>, field: &'static str) {
    assert_eq!(result, Err(ValidationError::ClaimMismatch { field }));
}

#[test]
fn preflight_submission_identity_mutations_reach_all_four_adapters() {
    let fresh = offer();
    let continuation = continuation_binding_offer();
    let mutations: [(&str, fn(&mut ReplaySessionGenesisClaimV1)); 5] = [
        ("host key", |claim| {
            claim.host_public_key = PublicKey32::from_bytes([90; 32])
        }),
        ("session", |claim| {
            claim.replay_session_id = Digest32::from_bytes([90; 32])
        }),
        ("host instance", |claim| {
            claim.host_participant_instance_id = Digest32::from_bytes([90; 32])
        }),
        ("host nonce", |claim| {
            claim.host_nonce = ChallengeNonce32::from_bytes([90; 32])
        }),
        ("ranked digest", |claim| {
            claim.ranked_session.rules_config_sha256 = Digest32::from_bytes([90; 32])
        }),
    ];
    for base in [&fresh, &continuation] {
        for (label, mutate) in mutations {
            let mut changed = base.clone();
            mutate(&mut changed.session_genesis.claim);
            let request = grant_binding_request(&changed);
            if let Some(grant) = &base.session_genesis.claim.fresh_run_preflight_grant {
                assert!(
                    grant
                        .validate_offer(base, FreshRunScopeV1::IndividualLevel)
                        .is_ok(),
                    "{label}"
                );
                assert!(
                    grant
                        .validate_offer_request(
                            &grant_binding_request(base),
                            FreshRunScopeV1::IndividualLevel
                        )
                        .is_ok()
                );
                assert_grant_binding_error(
                    grant.validate_offer(&changed, FreshRunScopeV1::IndividualLevel),
                    "fresh_run_preflight_grant.offer_binding",
                );
                assert_grant_binding_error(
                    grant.validate_offer_request(&request, FreshRunScopeV1::IndividualLevel),
                    "fresh_run_preflight_grant.offer_binding",
                );
            } else {
                let grant = base
                    .session_genesis
                    .claim
                    .campaign_continuation_preflight_grant
                    .as_ref()
                    .unwrap();
                assert!(grant.validate_offer(base).is_ok(), "{label}");
                assert!(
                    grant
                        .validate_offer_request(&grant_binding_request(base))
                        .is_ok()
                );
                assert_grant_binding_error(
                    grant.validate_offer(&changed),
                    "campaign_continuation_preflight_grant.offer_binding",
                );
                assert_grant_binding_error(
                    grant.validate_offer_request(&request),
                    "campaign_continuation_preflight_grant.offer_binding",
                );
            }
        }
    }
}

#[test]
fn fresh_campaign_binding_uses_request_claim_but_offer_expectation() {
    let base = offer();
    let grant = base
        .session_genesis
        .claim
        .fresh_run_preflight_grant
        .as_ref()
        .unwrap();
    for field in ["sha256", "byte_length"] {
        // Keep ranked identity bound while replacing just the campaign claim.
        let mut changed = base.clone();
        let ranked = &mut changed.session_genesis.claim.ranked_session;
        if field == "sha256" {
            ranked.starting_campaign_sha256 = Digest32::from_bytes([90; 32]);
        } else {
            ranked.starting_campaign_byte_length += 1;
        }
        let mut rebound = grant.clone();
        rebound.claim.ranked_session_sha256 = ranked.canonical_digest().unwrap();
        assert_grant_binding_error(
            rebound.validate_offer_request(
                &grant_binding_request(&changed),
                FreshRunScopeV1::IndividualLevel,
            ),
            "fresh_run_preflight_grant.offer_binding",
        );
        assert!(
            rebound
                .validate_offer(&changed, FreshRunScopeV1::IndividualLevel)
                .is_ok()
        );

        let mut changed = base.clone();
        let InitialStateExpectationV1::IndividualLevel {
            campaign_sha256,
            starting_campaign_byte_length,
            ..
        } = &mut changed.starting_state
        else {
            unreachable!()
        };
        if field == "sha256" {
            *campaign_sha256 = Digest32::from_bytes([90; 32]);
        } else {
            *starting_campaign_byte_length += 1;
        }
        assert_grant_binding_error(
            grant.validate_offer(&changed, FreshRunScopeV1::IndividualLevel),
            "fresh_run_preflight_grant.offer_binding",
        );
        assert!(
            grant
                .validate_offer_request(
                    &grant_binding_request(&changed),
                    FreshRunScopeV1::IndividualLevel
                )
                .is_ok()
        );
    }
    assert_grant_binding_error(
        grant.validate_offer(&base, FreshRunScopeV1::CampaignGenesis),
        "fresh_run_preflight_grant.offer_binding",
    );
    assert_grant_binding_error(
        grant.validate_offer_request(
            &grant_binding_request(&base),
            FreshRunScopeV1::CampaignGenesis,
        ),
        "fresh_run_preflight_grant.offer_binding",
    );
}

#[test]
fn continuation_binding_mutations_preserve_request_and_offer_authority() {
    let base = continuation_binding_offer();
    let grant = base
        .session_genesis
        .claim
        .campaign_continuation_preflight_grant
        .as_ref()
        .unwrap();
    let mutations = [
        (
            "/starting_state/chain_id",
            serde_json::json!("other-chain"),
            true,
        ),
        (
            "/starting_state/predecessor_run_id",
            serde_json::json!("other-run"),
            true,
        ),
        (
            "/starting_state/predecessor_verification_sha256",
            serde_json::json!(Digest32::from_bytes([90; 32])),
            false,
        ),
        (
            "/starting_state/campaign_sha256",
            serde_json::json!(Digest32::from_bytes([90; 32])),
            false,
        ),
        (
            "/starting_state/starting_campaign_byte_length",
            serde_json::json!(322),
            false,
        ),
        ("/max_concurrent_players", serde_json::json!(2), true),
        (
            "/participant_claims/0/public_key",
            serde_json::json!(PublicKey32::from_bytes([90; 32])),
            true,
        ),
    ];
    for (path, value, request_must_reject) in mutations {
        let mut json = serde_json::to_value(&base).unwrap();
        *json.pointer_mut(path).unwrap() = value;
        let changed: SubmissionOfferV1 = serde_json::from_value(json).unwrap();
        assert_grant_binding_error(
            grant.validate_offer(&changed),
            "campaign_continuation_preflight_grant.offer_binding",
        );
        let result = grant.validate_offer_request(&grant_binding_request(&changed));
        if request_must_reject {
            assert_grant_binding_error(
                result,
                "campaign_continuation_preflight_grant.offer_binding",
            );
        } else {
            assert!(result.is_ok(), "{path} belongs only to the admitted offer");
        }
    }

    // Controller must be a durable member, not necessarily the current host.
    let controller = PublicKey32::from_bytes([90; 32]);
    let mut grant = grant.clone();
    grant.claim.campaign_controller_public_key = controller;
    grant.claim.participant_public_keys.push(controller);
    grant.claim.max_concurrent_players = 2;
    assert!(grant.validate().is_ok());
    let mut changed = base.clone();
    changed.max_concurrent_players = 2;
    assert!(!grant.binds_participants(2, &changed.participant_claims));
    assert_grant_binding_error(
        grant.validate_offer(&changed),
        "campaign_continuation_preflight_grant.offer_binding",
    );
    assert_grant_binding_error(
        grant.validate_offer_request(&grant_binding_request(&changed)),
        "campaign_continuation_preflight_grant.offer_binding",
    );
    let mut participant = host_claim();
    participant.public_key = controller;
    changed.participant_claims.push(participant);
    assert!(grant.validate_offer(&changed).is_ok());
    assert!(
        grant
            .validate_offer_request(&grant_binding_request(&changed))
            .is_ok()
    );
}

#[test]
fn grant_shape_and_continuation_scope_errors_precede_binding_errors() {
    let base = offer();
    let mut fresh = base
        .session_genesis
        .claim
        .fresh_run_preflight_grant
        .clone()
        .unwrap();
    fresh.claim.host_nonce = ChallengeNonce32::from_bytes([0; 32]);
    for result in [
        fresh.validate_offer(&base, FreshRunScopeV1::CampaignGenesis),
        fresh.validate_offer_request(
            &grant_binding_request(&base),
            FreshRunScopeV1::CampaignGenesis,
        ),
    ] {
        assert_grant_binding_error(result, "fresh_run_preflight_grant.identity_or_interval");
    }
    let continuation = continuation_binding_offer();
    let mut grant = continuation
        .session_genesis
        .claim
        .campaign_continuation_preflight_grant
        .unwrap();
    assert_grant_binding_error(
        grant.validate_offer(&base),
        "campaign_continuation_preflight_grant.offer_scope",
    );
    assert_grant_binding_error(
        grant.validate_offer_request(&grant_binding_request(&base)),
        "campaign_continuation_preflight_grant.offer_scope",
    );
    grant.claim.host_nonce = ChallengeNonce32::from_bytes([0; 32]);
    assert_grant_binding_error(
        grant.validate_offer(&base),
        "campaign_continuation_preflight_grant.identity_or_interval",
    );
    assert_grant_binding_error(
        grant.validate_offer_request(&grant_binding_request(&base)),
        "campaign_continuation_preflight_grant.identity_or_interval",
    );
}

#[test]
fn offer_binding_rejects_each_substituted_request_field() {
    let offer = offer();
    let request = SubmissionOfferRequestV1 {
        schema_version: offer.schema_version,
        max_concurrent_players: offer.max_concurrent_players,
        participant_instance_count: offer.participant_instance_count,
        participant_claims: offer.participant_claims.clone(),
        session_genesis: offer.session_genesis.clone(),
        mission_id: offer.mission_id.clone(),
        scope_request: ScopeRequestV1::IndividualLevel,
        ruleset_manifest_sha256: offer.ruleset_manifest_sha256,
        competition_manifest_sha256: offer.competition_manifest_sha256,
    };
    assert!(crate::validate_offer_binding(&request, &offer).is_ok());
    let mutations = [
        ("schema_version", serde_json::json!(2)),
        ("max_concurrent_players", serde_json::json!(2)),
        ("participant_instance_count", serde_json::json!(2)),
        ("participant_claims", serde_json::json!([])),
        ("mission_id", serde_json::json!("changed")),
        (
            "ruleset_manifest_sha256",
            serde_json::json!(Digest32::from_bytes([99; 32])),
        ),
        (
            "competition_manifest_sha256",
            serde_json::json!(Digest32::from_bytes([99; 32])),
        ),
    ];
    for (field, replacement) in mutations {
        let mut value = serde_json::to_value(&request).unwrap();
        value[field] = replacement;
        let changed = serde_json::from_value(value).unwrap();
        assert_eq!(
            crate::validate_offer_binding(&changed, &offer),
            Err(ValidationError::ClaimMismatch { field })
        );
    }
    let mut changed = request.clone();
    changed.session_genesis.host_signature = Signature64::from_bytes([99; 64]);
    assert_eq!(
        crate::validate_offer_binding(&changed, &offer),
        Err(ValidationError::ClaimMismatch {
            field: "session_genesis"
        })
    );

    let mut offered = offer.clone();
    offered.starting_state = InitialStateExpectationV1::CampaignContinuation {
        chain_id: id("chain"),
        predecessor_run_id: id("previous"),
        predecessor_verification_sha256: Digest32::from_bytes([99; 32]),
        campaign_state_requirement: offer.starting_state.campaign_state_requirement(),
        campaign_sha256: offer.starting_state.campaign_sha256(),
        starting_campaign_byte_length: 321,
    };
    changed = request;
    changed.scope_request = ScopeRequestV1::CampaignGenesis;
    assert!(crate::validate_offer_binding(&changed, &offered).is_err());
    changed.scope_request = ScopeRequestV1::CampaignContinuation {
        chain_id: id("chain"),
        predecessor_run_id: id("previous"),
    };
    assert!(crate::validate_offer_binding(&changed, &offered).is_ok());
    changed.scope_request = ScopeRequestV1::CampaignContinuation {
        chain_id: id("other"),
        predecessor_run_id: id("previous"),
    };
    assert!(crate::validate_offer_binding(&changed, &offered).is_err());
    changed.scope_request = ScopeRequestV1::CampaignContinuation {
        chain_id: id("chain"),
        predecessor_run_id: id("other"),
    };
    assert!(crate::validate_offer_binding(&changed, &offered).is_err());
}

fn full_campaign_genesis_offer(subject: OfficialContentSubjectV1) -> SubmissionOfferV1 {
    let mut offer = offer();
    let mission_id = subject.mission_id().to_owned();
    offer.mission_id = mission_id.clone();
    offer.starting_state = InitialStateExpectationV1::CampaignGenesis {
        template_id: id("full-campaign-genesis"),
        campaign_state_requirement: crate::CanonicalCampaignStateRequirementV1 {
            edition: OfficialContentEditionV1::Full,
            kind: crate::CanonicalCampaignStateKindV1::FullCampaignGenesis,
            rules_config_sha256: offer.rules_config_sha256,
        },
        campaign_sha256: Digest32::from_bytes([8; 32]),
        starting_campaign_byte_length: 321,
    };
    let ranked = &mut offer.session_genesis.claim.ranked_session;
    ranked.mission_id = mission_id;
    ranked.content_edition = OfficialContentEditionV1::Full;
    ranked.content_subject = subject;
    ranked.campaign_content_manifest_sha256 = Some(Digest32::from_bytes([15; 32]));
    let grant = fresh_preflight_grant(ranked, FreshRunScopeV1::CampaignGenesis);
    offer.session_genesis.claim.fresh_run_preflight_grant = Some(grant);
    offer
}

fn submission() -> SubmissionEnvelopeV1 {
    let offer = offer();
    let replay_session_transcript =
        single_player_transcript(offer.session_genesis.canonical_digest().unwrap());
    SubmissionEnvelopeV1 {
        schema_version: 1,
        offer,
        replay_session_transcript,
        artifacts: submission_artifacts(),
        campaign_aggregation_consent: CampaignAggregationConsentV1::NotAuthorized,
        campaign_continuation_authorization: None,
        requested_metrics: vec![BoardMetricV1::OriginalScore],
    }
}

#[test]
fn submission_signing_bytes_are_fixed_and_bind_one_use_offer() {
    let mut submission = submission();
    let bytes = submission.signing_bytes().unwrap();
    assert_eq!(bytes.len(), LEADERBOARD_CO_SIGN_PAYLOAD_LENGTH_V1);
    assert!(bytes.starts_with(LEADERBOARD_CO_SIGN_PAYLOAD_DOMAIN_V1));
    assert_eq!(
        submission.co_sign_request().unwrap().instance.purpose,
        LeaderboardCoSignPurposeV1::Submission
    );
    let digest = Digest32::digest_bytes(bytes);
    submission.offer.expires_at_unix_ms -= 1;
    assert_ne!(
        digest,
        Digest32::digest_bytes(submission.signing_bytes().unwrap())
    );
}

#[test]
fn co_sign_payload_layout_is_an_exact_bitcode_round_trip_fixture() {
    let request = LeaderboardCoSignRequestV1 {
        instance: LeaderboardCoSignInstanceV1 {
            purpose: LeaderboardCoSignPurposeV1::Submission,
            replay_session_id: Digest32::from_bytes([0x11; 32]),
            submission_offer_sha256: Digest32::from_bytes([0x22; 32]),
        },
        run_digest: Digest32::from_bytes([0x33; 32]),
    };
    let bytes = request.signing_bytes().unwrap();
    let domain_end = LEADERBOARD_CO_SIGN_PAYLOAD_DOMAIN_V1.len();
    assert_eq!(&bytes[..domain_end], LEADERBOARD_CO_SIGN_PAYLOAD_DOMAIN_V1);
    assert_eq!(bytes[domain_end], 2);
    assert_eq!(&bytes[domain_end + 1..domain_end + 33], &[0x11; 32]);
    assert_eq!(&bytes[domain_end + 33..domain_end + 65], &[0x22; 32]);
    assert_eq!(&bytes[domain_end + 65..], &[0x33; 32]);
    assert_eq!(
        bitcode::decode::<LeaderboardCoSignRequestV1>(&bitcode::encode(&request)).unwrap(),
        request
    );
}

#[test]
fn co_sign_purposes_are_not_substitutable() {
    let submission = submission();
    let final_request = submission.co_sign_request().unwrap();
    let mut continuation_request = final_request;
    continuation_request.instance.purpose = LeaderboardCoSignPurposeV1::CampaignContinuation;
    assert_ne!(
        final_request.signing_bytes().unwrap(),
        continuation_request.signing_bytes().unwrap()
    );
}

#[test]
fn ranked_session_cross_binds_exact_prepared_inputs_seal() {
    let mut ranked = session_genesis().claim.ranked_session;
    let mut seal = PreparedMissionInputsSealV1 {
        schema_version: 1,
        prepared_inputs_projection_sha256: ranked.prepared_inputs_projection_sha256,
        content_manifest_sha256: ranked.content_manifest_sha256,
        content_edition: ranked.content_edition,
        content_subject: ranked.content_subject.clone(),
        starting_campaign_sha256: ranked.starting_campaign_sha256,
        starting_campaign_byte_length: ranked.starting_campaign_byte_length,
        simulation_seed: ranked.simulation_seed,
        rules_config_sha256: ranked.rules_config_sha256,
        resource_locale_root: ranked.resource_locale_root.clone(),
        speech_timing: ranked.speech_timing.clone(),
        spellforge_content_sha256: None,
        original_rng_replay_sha256: None,
    };
    ranked.prepared_mission_inputs_seal_sha256 = seal.canonical_digest().unwrap();
    assert!(ranked.validate_prepared_inputs_seal(&seal).is_ok());

    seal.starting_campaign_byte_length += 1;
    assert!(ranked.validate_prepared_inputs_seal(&seal).is_err());
    seal.starting_campaign_byte_length = ranked.starting_campaign_byte_length;

    seal.resource_locale_root = ResourceLocaleRootV1::new("2047").unwrap();
    assert!(ranked.validate_prepared_inputs_seal(&seal).is_err());
    seal.resource_locale_root = ranked.resource_locale_root.clone();

    seal.prepared_inputs_projection_sha256 = Digest32::from_bytes([99; 32]);
    assert!(ranked.validate_prepared_inputs_seal(&seal).is_err());
    seal.prepared_inputs_projection_sha256 = ranked.prepared_inputs_projection_sha256;
    seal.original_rng_replay_sha256 = Some(Digest32::from_bytes([98; 32]));
    assert!(matches!(
        ranked.validate_prepared_inputs_seal(&seal),
        Err(ValidationError::ClaimMismatch {
            field: "prepared_mission_inputs_seal.unranked_input_mode"
        })
    ));
}

#[test]
fn starting_campaign_length_is_nonzero_and_cross_bound_to_offer() {
    let mut offer = offer();
    assert!(offer.validate().is_ok());

    offer.starting_state = InitialStateExpectationV1::IndividualLevel {
        template_id: id("leicester-default"),
        campaign_state_requirement: offer.starting_state.campaign_state_requirement(),
        campaign_sha256: Digest32::from_bytes([8; 32]),
        starting_campaign_byte_length: 0,
    };
    assert!(offer.validate().is_err());

    offer.starting_state = InitialStateExpectationV1::IndividualLevel {
        template_id: id("leicester-default"),
        campaign_state_requirement: offer.starting_state.campaign_state_requirement(),
        campaign_sha256: Digest32::from_bytes([8; 32]),
        starting_campaign_byte_length: 322,
    };
    assert!(matches!(
        offer.validate(),
        Err(ValidationError::ClaimMismatch {
            field: "fresh_run_preflight_grant.offer_binding"
        })
    ));
}

#[test]
fn signed_offer_allows_only_h01_as_full_campaign_genesis() {
    let h01 = full_campaign_genesis_offer(OfficialContentSubjectV1::FieldMission {
        mission_id: crate::OFFICIAL_FULL_CAMPAIGN_GENESIS_MISSION_ID_V1.to_owned(),
    });
    assert!(h01.validate().is_ok());

    let h12 = full_campaign_genesis_offer(OfficialContentSubjectV1::FieldMission {
        mission_id: "H12_Not_MP".to_owned(),
    });
    assert!(matches!(
        h12.validate(),
        Err(ValidationError::ClaimMismatch {
            field: "ranked_scope_subject.official_lane"
        })
    ));

    let headquarters = full_campaign_genesis_offer(OfficialContentSubjectV1::Headquarters {
        mission_id: crate::OFFICIAL_FULL_HEADQUARTERS_MISSION_ID_V1.to_owned(),
    });
    assert!(matches!(
        headquarters.validate(),
        Err(ValidationError::ClaimMismatch {
            field: "ranked_scope_subject.official_lane"
        })
    ));
}

#[test]
fn official_full_continuations_allow_later_field_and_headquarters_subjects() {
    let continuation = InitialStateExpectationV1::CampaignContinuation {
        chain_id: id("campaign-chain"),
        predecessor_run_id: id("predecessor-run"),
        predecessor_verification_sha256: Digest32::from_bytes([17; 32]),
        campaign_state_requirement: crate::CanonicalCampaignStateRequirementV1 {
            edition: OfficialContentEditionV1::Full,
            kind: crate::CanonicalCampaignStateKindV1::FullCampaignGenesis,
            rules_config_sha256: Digest32::from_bytes([6; 32]),
        },
        campaign_sha256: Digest32::from_bytes([8; 32]),
        starting_campaign_byte_length: 321,
    };
    for subject in [
        OfficialContentSubjectV1::FieldMission {
            mission_id: "H12_Not_MP".to_owned(),
        },
        OfficialContentSubjectV1::Headquarters {
            mission_id: crate::OFFICIAL_FULL_HEADQUARTERS_MISSION_ID_V1.to_owned(),
        },
    ] {
        assert!(
            validate_official_ranked_scope_subject_v1(
                OfficialContentEditionV1::Full,
                &subject,
                &continuation,
            )
            .is_ok()
        );
    }
}

#[test]
fn offer_rejects_campaign_state_authority_from_another_rules_config() {
    let mut offer = offer();
    assert!(offer.validate().is_ok());
    let InitialStateExpectationV1::IndividualLevel {
        campaign_state_requirement,
        ..
    } = &mut offer.starting_state
    else {
        panic!("offer fixture is individual-level")
    };
    campaign_state_requirement.rules_config_sha256 = Digest32::from_bytes([99; 32]);
    assert!(matches!(
        offer.validate(),
        Err(ValidationError::ClaimMismatch {
            field: "submission_offer.starting_state.rules_config_sha256"
        })
    ));
}

#[test]
fn campaign_continuation_binds_exact_starting_campaign_length() {
    let mut continuation = InitialStateExpectationV1::CampaignContinuation {
        chain_id: id("chain-1"),
        predecessor_run_id: id("run-1"),
        predecessor_verification_sha256: Digest32::from_bytes([3; 32]),
        campaign_state_requirement: crate::CanonicalCampaignStateRequirementV1 {
            edition: crate::OfficialContentEditionV1::Demo,
            kind: crate::CanonicalCampaignStateKindV1::IndividualTemplate,
            rules_config_sha256: Digest32::from_bytes([6; 32]),
        },
        campaign_sha256: Digest32::from_bytes([4; 32]),
        starting_campaign_byte_length: 987,
    };
    assert!(continuation.validate().is_ok());
    let baseline = crate::canonical_json_bytes(&continuation).unwrap();
    if let InitialStateExpectationV1::CampaignContinuation {
        starting_campaign_byte_length,
        ..
    } = &mut continuation
    {
        *starting_campaign_byte_length = 988;
    }
    assert_ne!(
        baseline,
        crate::canonical_json_bytes(&continuation).unwrap()
    );
    if let InitialStateExpectationV1::CampaignContinuation {
        starting_campaign_byte_length,
        ..
    } = &mut continuation
    {
        *starting_campaign_byte_length = 0;
    }
    assert!(continuation.validate().is_err());
}

#[test]
fn anonymous_display_still_requires_authenticated_claim_and_cosignature() {
    let mut submission = submission();
    submission.offer.participant_claims[0].public_disclosure =
        ParticipantPublicDisclosureV1::Anonymous;
    let signed = SignedSubmissionV1 {
        schema_version: 1,
        participant_signatures: vec![ParticipantSignatureV1 {
            public_key: submission.offer.participant_claims[0].public_key,
            signature: Signature64::from_bytes([9; 64]),
        }],
        submission,
        algorithm: SignatureAlgorithmV1::Ed25519,
    };
    assert!(signed.validate().is_ok());

    let mut missing_claim = signed;
    missing_claim.submission.offer.participant_claims.clear();
    assert!(missing_claim.validate().is_err());
}

#[test]
fn infrastructure_failure_is_not_a_run_rejection() {
    let status = VerificationStatusV1::FailedInfrastructure(VerificationInfrastructureFailureV1 {
        code: VerificationInfrastructureFailureCodeV1::WorkerInternalFailure,
        private_detail_code: Some("worker_panicked".into()),
    });
    assert!(status.validate().is_ok());
    assert!(!matches!(status, VerificationStatusV1::Rejected(_)));
}

#[test]
fn participant_signatures_cover_every_authenticated_seat() {
    let submission = submission();
    let signed = SignedSubmissionV1 {
        schema_version: 1,
        participant_signatures: vec![ParticipantSignatureV1 {
            public_key: submission.offer.participant_claims[0].public_key,
            signature: Signature64::from_bytes([9; 64]),
        }],
        submission,
        algorithm: SignatureAlgorithmV1::Ed25519,
    };
    assert!(signed.validate().is_ok());
    assert_eq!(signed.submission.offer.max_concurrent_players, 1);
    let mut extra = signed.clone();
    extra
        .participant_signatures
        .push(extra.participant_signatures[0].clone());
    assert_eq!(
        extra.validate(),
        Err(ValidationError::InvalidParticipantSignatures)
    );
}

#[test]
fn named_guest_join_is_bound_to_genesis_transcript_and_final_claim() {
    let genesis = session_genesis();
    let speech_manifest = ContentManifestV1 {
        schema_version: 1,
        name: "test-content".into(),
        edition: crate::OfficialContentEditionV1::Demo,
        subject: OfficialContentSubjectV1::FieldMission {
            mission_id: "Dem_Lei_MP".into(),
        },
        closure: crate::ContentClosureKindV1::StaticPreparedMissionContentProjection,
        projection_schema_version: 2,
        resource_locale_root: ResourceLocaleRootV1::new("1033").unwrap(),
        speech_timing: SimulationSpeechTimingSourceV1::LanguagePack {
            canonical_locale: "en-US".into(),
        },
        components: [
            crate::SimulationContentComponentKindV1::Profiles,
            crate::SimulationContentComponentKindV1::LoadedLevel,
            crate::SimulationContentComponentKindV1::MissionScripts,
            crate::SimulationContentComponentKindV1::SpriteSimulationMetadata,
            crate::SimulationContentComponentKindV1::MapGeometryMetadata,
            crate::SimulationContentComponentKindV1::LocalizedDeterministicText,
            crate::SimulationContentComponentKindV1::SoundDurationTables,
            crate::SimulationContentComponentKindV1::InterfaceSimulationMetadata,
        ]
        .into_iter()
        .enumerate()
        .map(|(index, kind)| crate::SimulationContentComponentV1 {
            kind,
            component_schema_version: 1,
            artifact: ArtifactRefV1 {
                sha256: Digest32::from_bytes([index as u8 + 20; 32]),
                byte_length: index as u64 + 20,
                media_type: crate::SIMULATION_CONTENT_COMPONENT_MEDIA_TYPE_V1.into(),
            },
        })
        .collect(),
    };
    let mut ranked_config = genesis.claim.ranked_session.clone();
    ranked_config.content_manifest_sha256 = speech_manifest.canonical_digest().unwrap();
    assert!(
        ranked_config
            .validate_content_manifest(&speech_manifest)
            .is_ok()
    );
    ranked_config.resource_locale_root = ResourceLocaleRootV1::new("2047").unwrap();
    assert!(
        ranked_config
            .validate_content_manifest(&speech_manifest)
            .is_err()
    );
    ranked_config.resource_locale_root = ResourceLocaleRootV1::new("1033").unwrap();
    ranked_config.speech_timing = SpeechTimingAuthorityV1::LanguagePack {
        canonical_locale: "de-DE".into(),
    };
    assert!(
        ranked_config
            .validate_content_manifest(&speech_manifest)
            .is_err()
    );
    let mut base_speech = genesis.clone();
    base_speech.claim.ranked_session.speech_timing = SpeechTimingAuthorityV1::BaseInstallation;
    base_speech.claim.fresh_run_preflight_grant = Some(fresh_preflight_grant(
        &base_speech.claim.ranked_session,
        FreshRunScopeV1::IndividualLevel,
    ));
    assert!(base_speech.validate().is_ok());
    assert_ne!(
        base_speech.claim.canonical_digest().unwrap(),
        genesis.claim.canonical_digest().unwrap()
    );
    let guest_attestation = guest_attestation(&genesis);
    assert!(
        guest_attestation
            .signing_bytes()
            .unwrap()
            .starts_with(NAMED_SEAT_JOIN_SIGNATURE_DOMAIN_V1)
    );
    let claims = vec![
        host_claim(),
        ParticipantClaimV1 {
            seat: 1,
            participant_instance_id: guest_attestation.claim.participant_instance_id,
            public_key: guest_attestation.claim.public_key,
            public_disclosure: ParticipantPublicDisclosureV1::NamedProfile,
            join_attestation: Some(guest_attestation),
        },
    ];
    let transcript = multiplayer_transcript(&genesis);
    assert!(validate_participants(2, 2, &claims).is_ok());
    assert!(validate_participant_session_context(&genesis, &claims).is_ok());
    assert!(validate_participants_against_transcript(&claims, &transcript).is_ok());

    let mut anonymous_claims = claims.clone();
    anonymous_claims[1].public_disclosure = ParticipantPublicDisclosureV1::Anonymous;
    assert!(validate_participants(2, 2, &anonymous_claims).is_ok());
    assert!(validate_participant_session_context(&genesis, &anonymous_claims).is_ok());
    assert!(validate_participants_against_transcript(&anonymous_claims, &transcript).is_ok());

    let mut claim_only = claims.clone();
    claim_only[1]
        .join_attestation
        .as_mut()
        .unwrap()
        .claim
        .session_genesis_sha256 = genesis.claim.canonical_digest().unwrap();
    assert!(validate_participant_session_context(&genesis, &claim_only).is_err());

    let mut substituted_signature = genesis.clone();
    substituted_signature.host_signature = Signature64::from_bytes([0xfe; 64]);
    assert!(substituted_signature.validate().is_ok());
    assert!(validate_participant_session_context(&substituted_signature, &claims).is_err());

    let mut same_key_anonymous = claims.clone();
    let host_public_key = same_key_anonymous[0].public_key;
    same_key_anonymous[1].public_disclosure = ParticipantPublicDisclosureV1::Anonymous;
    same_key_anonymous[1].public_key = host_public_key;
    same_key_anonymous[1]
        .join_attestation
        .as_mut()
        .unwrap()
        .claim
        .public_key = host_public_key;
    assert_eq!(
        validate_participants(2, 2, &same_key_anonymous),
        Err(ValidationError::InvalidParticipantClaims),
        "public anonymity must not permit two occupied seats to reuse one durable key"
    );

    let mut substituted = claims.clone();
    substituted[1]
        .join_attestation
        .as_mut()
        .unwrap()
        .claim
        .session_genesis_sha256 = Digest32::from_bytes([99; 32]);
    assert!(validate_participant_session_context(&genesis, &substituted).is_err());
}

#[test]
fn replay_session_transcript_rejects_lifecycle_mutations_and_leaks_no_identity() {
    let genesis = session_genesis();
    let transcript = multiplayer_transcript(&genesis);
    assert_eq!(transcript.validate_and_derive_counts().unwrap(), (2, 2));

    let mut epoch_gap = transcript.clone();
    epoch_gap.events[3].lifecycle = ReplaySeatLifecycleKindV1::Connected {
        connection_epoch: 2,
    };
    assert!(epoch_gap.validate().is_err());

    let mut host_disconnect = transcript.clone();
    host_disconnect.events.push(ReplaySeatLifecycleEventV1 {
        event_ordinal: 4,
        replay_ordinal: 21,
        seat: 0,
        participant_instance_id: genesis.claim.host_participant_instance_id,
        lifecycle: ReplaySeatLifecycleKindV1::Disconnected,
    });
    assert!(host_disconnect.validate().is_err());

    let mut huge_seat = transcript.clone();
    huge_seat.events[1].seat = u16::MAX;
    assert!(huge_seat.validate().is_err());

    let mut removed_and_renumbered = transcript.clone();
    removed_and_renumbered.events.remove(2);
    removed_and_renumbered.events[2].event_ordinal = 2;
    assert!(removed_and_renumbered.validate().is_err());

    let mut moved_instance = transcript.clone();
    moved_instance.events[3].seat = 2;
    assert!(moved_instance.validate().is_err());

    let mut occupied_replacement = transcript.clone();
    occupied_replacement.events[2] = ReplaySeatLifecycleEventV1 {
        event_ordinal: 2,
        replay_ordinal: 10,
        seat: 1,
        participant_instance_id: Digest32::from_bytes([18; 32]),
        lifecycle: ReplaySeatLifecycleKindV1::Connected {
            connection_epoch: 0,
        },
    };
    assert!(occupied_replacement.validate().is_err());

    let json = serde_json::to_string(&transcript).unwrap();
    assert!(!json.contains("username"));
    assert!(!json.contains("endpoint"));
    assert!(!json.contains(&PublicKey32::from_bytes([16; 32]).to_string()));
}

#[test]
fn replay_events_must_precede_the_terminal_frame() {
    let genesis = session_genesis();
    let genesis_sha256 = genesis.canonical_digest().unwrap();
    let mut run = verified_run(genesis_sha256);
    let guest_attestation = guest_attestation(&genesis);
    run.max_concurrent_players = 2;
    run.participant_instance_count = 2;
    run.named_participant_instance_count = 2;
    run.authenticated_participant_claims
        .push(ParticipantClaimV1 {
            seat: 1,
            participant_instance_id: guest_attestation.claim.participant_instance_id,
            public_key: guest_attestation.claim.public_key,
            public_disclosure: ParticipantPublicDisclosureV1::NamedProfile,
            join_attestation: Some(guest_attestation),
        });
    run.replay_session_transcript = multiplayer_transcript(&genesis);
    run.replay_session_transcript.events[3].replay_ordinal = run.replay_frames;
    assert!(matches!(
        run.validate(),
        Err(ValidationError::CountOutOfRange {
            field: "verified_run.replay_session_transcript.replay_ordinal"
        })
    ));
}

#[test]
fn requested_metrics_must_be_nonempty_canonical_subset() {
    let mut submission = submission();
    submission.requested_metrics = vec![];
    assert!(matches!(
        submission.validate(),
        Err(ValidationError::InvalidMetrics { .. })
    ));
    submission.requested_metrics = vec![BoardMetricV1::FastestSuccess];
    submission.offer.allowed_metrics = vec![BoardMetricV1::OriginalScore];
    assert_eq!(
        submission.validate(),
        Err(ValidationError::MetricsNotOffered)
    );
}

#[test]
fn ranked_replay_requires_fixed_compact_media_type() {
    let replay = replay_artifact(1);
    assert!(replay.validate().is_ok());
    for media_type in [
        "application/x-robin-rhrec+jsonl",
        "application/octet-stream",
    ] {
        let mut alternate = replay.clone();
        alternate.artifact.media_type = media_type.into();
        assert!(alternate.validate().is_err());
    }
}

#[test]
fn ranked_replay_artifact_rejects_precanonical_schema() {
    let mut replay = replay_artifact(1);
    replay.replay_schema_version = 19;
    assert!(matches!(
        replay.validate(),
        Err(ValidationError::ClaimMismatch {
            field: "replay.replay_schema_version"
        })
    ));
}

#[test]
fn submission_signature_binds_canonical_replay_and_exact_starting_campaign() {
    let submission = submission();
    assert!(submission.validate().is_ok());
    let baseline = submission.signing_bytes().unwrap();

    let mut substituted_replay = submission.clone();
    substituted_replay.artifacts.replay.artifact.sha256 = Digest32::from_bytes([2; 32]);
    assert!(substituted_replay.validate().is_ok());
    assert_ne!(baseline, substituted_replay.signing_bytes().unwrap());

    let mut substituted_length = submission.clone();
    substituted_length.artifacts.starting_campaign.byte_length += 1;
    assert!(substituted_length.validate().is_err());
    assert!(substituted_length.signing_bytes().is_err());

    let mut wrong_media = submission.clone();
    wrong_media.artifacts.starting_campaign.media_type = RANKED_REPLAY_MEDIA_TYPE_V1.into();
    assert!(wrong_media.validate().is_err());

    let mut missing = serde_json::to_value(&submission).unwrap();
    missing
        .as_object_mut()
        .unwrap()
        .get_mut("artifacts")
        .unwrap()
        .as_object_mut()
        .unwrap()
        .remove("replay");
    assert!(serde_json::from_value::<SubmissionEnvelopeV1>(missing).is_err());

    let mut legacy_dual = serde_json::to_value(submission).unwrap();
    let artifacts = legacy_dual["artifacts"].as_object_mut().unwrap();
    let replay = artifacts.remove("replay").unwrap();
    artifacts.insert("private_replay".into(), replay.clone());
    artifacts.insert("public_replay".into(), replay);
    assert!(serde_json::from_value::<SubmissionEnvelopeV1>(legacy_dual).is_err());
}

#[test]
fn username_signature_domain_excludes_signature_but_binds_username() {
    let mut update = UsernameUpdateEnvelopeV1 {
        schema_version: 1,
        username_challenge_id: id("username-challenge"),
        username_challenge_nonce: ChallengeNonce32::from_bytes([10; 32]),
        public_key: PublicKey32::from_bytes([11; 32]),
        username: "Robin".into(),
        signature: Signature64::from_bytes([12; 64]),
    };
    assert!(update.validate_signing_claim().is_ok());
    let bytes = update.signing_bytes().unwrap();
    assert!(bytes.starts_with(USERNAME_UPDATE_SIGNATURE_DOMAIN_V1));
    update.signature = Signature64::from_bytes([13; 64]);
    assert_eq!(bytes, update.signing_bytes().unwrap());
    update.username = "Marian".into();
    assert_ne!(bytes, update.signing_bytes().unwrap());

    update.signature = Signature64::from_bytes([0; 64]);
    assert!(update.validate_signing_claim().is_ok());
    assert!(update.validate().is_err());
}

#[test]
fn unknown_fields_are_rejected() {
    let mut value = serde_json::to_value(submission()).unwrap();
    value
        .as_object_mut()
        .unwrap()
        .insert("surprise".into(), serde_json::Value::Bool(true));
    assert!(serde_json::from_value::<SubmissionEnvelopeV1>(value).is_err());
}

#[test]
fn result_digest_is_canonical() {
    let genesis_sha256 = session_genesis().canonical_digest().unwrap();
    let result = VerificationResultV1 {
        schema_version: 1,
        request_id: id("verification-1"),
        verification_request_sha256: Digest32::from_bytes([10; 32]),
        artifacts: submission_artifacts(),
        session_genesis_sha256: genesis_sha256,
        build_manifest_sha256: Digest32::from_bytes([2; 32]),
        content_manifest_sha256: Digest32::from_bytes([3; 32]),
        rules_config_sha256: Digest32::from_bytes([4; 32]),
        ruleset_manifest_sha256: Digest32::from_bytes([5; 32]),
        competition_manifest_sha256: None,
        input_provenance: Some(InputProvenanceStatusV1::Rankable),
        status: VerificationStatusV1::Verified(verified_run(genesis_sha256)),
    };
    assert_eq!(
        result.canonical_digest().unwrap(),
        result.canonical_digest().unwrap()
    );
    assert!(result.validate().is_ok());
    let mut substituted = result;
    substituted.session_genesis_sha256 = Digest32::from_bytes([99; 32]);
    assert!(substituted.validate().is_err());
}

#[test]
fn campaign_completion_evidence_is_typed_and_cross_bound_to_result() {
    let genesis_sha256 = session_genesis().canonical_digest().unwrap();
    let mut result = VerificationResultV1 {
        schema_version: 1,
        request_id: id("verification-1"),
        verification_request_sha256: Digest32::from_bytes([10; 32]),
        artifacts: submission_artifacts(),
        session_genesis_sha256: genesis_sha256,
        build_manifest_sha256: Digest32::from_bytes([2; 32]),
        content_manifest_sha256: Digest32::from_bytes([3; 32]),
        rules_config_sha256: Digest32::from_bytes([4; 32]),
        ruleset_manifest_sha256: Digest32::from_bytes([5; 32]),
        competition_manifest_sha256: None,
        input_provenance: Some(InputProvenanceStatusV1::Rankable),
        status: VerificationStatusV1::Verified(verified_run(genesis_sha256)),
    };
    let VerificationStatusV1::Verified(run) = &mut result.status else {
        unreachable!()
    };
    run.scope_kind = RunScopeKindV1::Campaign;
    run.campaign_aggregation_consent =
        CampaignAggregationConsentV1::AuthorizeSignedSessionInServerRecognizedChainV1;
    run.campaign_session_kind = Some(CampaignSessionKindV1::FieldMission {
        mission_id: "H12_Not_MP".into(),
    });
    run.campaign_session_ordinal = Some(42);
    run.campaign_complete_evidence = Some(CampaignCompleteEvidenceV1 {
        schema_version: 1,
        campaign_content_manifest_sha256: Digest32::from_bytes([9; 32]),
        content_manifest_sha256: result.content_manifest_sha256,
        rules_config_sha256: result.rules_config_sha256,
        ruleset_manifest_sha256: result.ruleset_manifest_sha256,
        verification_request_sha256: result.verification_request_sha256,
        replay_sha256: result.artifacts.replay.artifact.sha256,
        terminal_subject: OfficialContentSubjectV1::FieldMission {
            mission_id: "H12_Not_MP".into(),
        },
        final_campaign_sha256: run.final_campaign.sha256,
        final_state_sha256: run.final_state_sha256,
        observed_progression_percent: 100,
    });
    assert!(result.validate().is_ok());
    let baseline = result.canonical_digest().unwrap();

    let mut forged = result.clone();
    let VerificationStatusV1::Verified(run) = &mut forged.status else {
        unreachable!()
    };
    run.campaign_complete_evidence
        .as_mut()
        .unwrap()
        .replay_sha256 = Digest32::from_bytes([99; 32]);
    assert!(forged.validate().is_err());

    let mut forged = result.clone();
    let VerificationStatusV1::Verified(run) = &mut forged.status else {
        unreachable!()
    };
    run.campaign_complete_evidence
        .as_mut()
        .unwrap()
        .final_campaign_sha256 = Digest32::from_bytes([98; 32]);
    assert!(forged.validate().is_err());

    let mut forged = result.clone();
    let VerificationStatusV1::Verified(run) = &mut forged.status else {
        unreachable!()
    };
    run.campaign_complete_evidence
        .as_mut()
        .unwrap()
        .final_state_sha256 = Digest32::from_bytes([97; 32]);
    assert!(forged.validate().is_err());

    let VerificationStatusV1::Verified(run) = &mut result.status else {
        unreachable!()
    };
    run.campaign_complete_evidence
        .as_mut()
        .unwrap()
        .observed_progression_percent = 99;
    assert!(result.validate().is_ok());
    assert_ne!(baseline, result.canonical_digest().unwrap());
}

fn completion_contract_fixture() -> (
    VerificationRequestV1,
    RulesetManifestV1,
    CampaignContentManifestV1,
    VerificationResultV1,
) {
    let terminal_subject = OfficialContentSubjectV1::FieldMission {
        mission_id: "H12_Not_MP".into(),
    };
    let terminal_content_sha256 = Digest32::from_bytes([5; 32]);
    let campaign_content = CampaignContentManifestV1 {
        schema_version: 1,
        edition: OfficialContentEditionV1::Full,
        entries: vec![crate::CampaignContentEntryV1 {
            subject: terminal_subject.clone(),
            content_manifest_sha256: terminal_content_sha256,
        }],
    };
    let campaign_content_sha256 = campaign_content.canonical_digest().unwrap();

    let mut unsigned = submission();
    unsigned.offer.mission_id = "H12_Not_MP".into();
    unsigned.offer.starting_state = InitialStateExpectationV1::CampaignContinuation {
        chain_id: id("full-campaign-chain"),
        predecessor_run_id: id("full-campaign-predecessor"),
        predecessor_verification_sha256: Digest32::from_bytes([17; 32]),
        campaign_state_requirement: crate::CanonicalCampaignStateRequirementV1 {
            edition: crate::OfficialContentEditionV1::Full,
            kind: crate::CanonicalCampaignStateKindV1::FullCampaignGenesis,
            rules_config_sha256: unsigned.offer.rules_config_sha256,
        },
        campaign_sha256: Digest32::from_bytes([8; 32]),
        starting_campaign_byte_length: 321,
    };
    unsigned.campaign_aggregation_consent =
        CampaignAggregationConsentV1::AuthorizeSignedSessionInServerRecognizedChainV1;
    let ranked = &mut unsigned.offer.session_genesis.claim.ranked_session;
    ranked.mission_id = "H12_Not_MP".into();
    ranked.content_edition = OfficialContentEditionV1::Full;
    ranked.content_subject = terminal_subject.clone();
    ranked.content_manifest_sha256 = terminal_content_sha256;
    ranked.campaign_content_manifest_sha256 = Some(campaign_content_sha256);
    unsigned.offer.content_manifest_sha256 = terminal_content_sha256;
    unsigned
        .offer
        .session_genesis
        .claim
        .fresh_run_preflight_grant = None;

    let mut ruleset = crate::manifest::tests::ruleset_manifest(unsigned.offer.rules_config_sha256);
    ruleset.allowed_build_manifest_sha256 = vec![unsigned.offer.build_manifest_sha256];
    ruleset.allowed_content_manifest_sha256 = vec![terminal_content_sha256];
    ruleset.allowed_campaign_content_manifest_sha256 = vec![campaign_content_sha256];
    let ruleset_sha256 = ruleset.canonical_digest().unwrap();
    unsigned.offer.ruleset_manifest_sha256 = ruleset_sha256;
    unsigned
        .offer
        .session_genesis
        .claim
        .ranked_session
        .ruleset_manifest_sha256 = ruleset_sha256;
    unsigned
        .offer
        .session_genesis
        .claim
        .campaign_continuation_preflight_grant = Some(continuation_preflight_grant(
        &unsigned.offer.session_genesis.claim,
        id("full-campaign-chain"),
        id("full-campaign-predecessor"),
        Digest32::from_bytes([17; 32]),
        host_claim().public_key,
        vec![host_claim().public_key],
        1,
    ));
    unsigned.campaign_continuation_authorization = Some(CampaignContinuationAuthorizationV1 {
        claim: CampaignContinuationAuthorizationClaimV1 {
            schema_version: 1,
            campaign_controller_public_key: host_claim().public_key,
            chain_id: id("full-campaign-chain"),
            predecessor_run_id: id("full-campaign-predecessor"),
            predecessor_verification_sha256: Digest32::from_bytes([17; 32]),
            next_session_genesis_sha256: unsigned.offer.session_genesis.canonical_digest().unwrap(),
            next_artifacts: unsigned.artifacts.clone(),
        },
        algorithm: SignatureAlgorithmV1::Ed25519,
        signature: Signature64::from_bytes([18; 64]),
    });
    unsigned.replay_session_transcript.session_genesis_sha256 =
        unsigned.offer.session_genesis.canonical_digest().unwrap();
    assert!(unsigned.validate().is_ok());

    let request = VerificationRequestV1 {
        schema_version: 1,
        request_id: id("completion-verification"),
        submission: SignedSubmissionV1 {
            schema_version: 1,
            submission: unsigned,
            algorithm: SignatureAlgorithmV1::Ed25519,
            participant_signatures: vec![ParticipantSignatureV1 {
                public_key: host_claim().public_key,
                signature: Signature64::from_bytes([42; 64]),
            }],
        },
        limits: VerificationLimitsV1 {
            max_input_bytes: 1024,
            max_compressed_bytes: 1024,
            max_decompressed_bytes: 4096,
            max_base64_payload_bytes: 2048,
            max_campaign_bytes: 4096,
            max_frames: 1000,
            max_version_bytes: 128,
            max_mission_id_bytes: 256,
            max_metadata_records: 64,
            max_entries_per_frame: 64,
        },
    };
    assert!(request.validate().is_ok());
    let request_sha256 = request.canonical_digest().unwrap();
    let offer = &request.submission.submission.offer;
    let genesis_sha256 = offer.session_genesis.canonical_digest().unwrap();
    let mut run = verified_run(genesis_sha256);
    run.scope_kind = RunScopeKindV1::Campaign;
    run.campaign_aggregation_consent =
        CampaignAggregationConsentV1::AuthorizeSignedSessionInServerRecognizedChainV1;
    run.campaign_session_kind = Some(CampaignSessionKindV1::FieldMission {
        mission_id: "H12_Not_MP".into(),
    });
    run.campaign_session_ordinal = Some(42);
    run.campaign_complete_evidence = Some(CampaignCompleteEvidenceV1 {
        schema_version: 1,
        campaign_content_manifest_sha256: campaign_content_sha256,
        content_manifest_sha256: terminal_content_sha256,
        rules_config_sha256: offer.rules_config_sha256,
        ruleset_manifest_sha256: ruleset_sha256,
        verification_request_sha256: request_sha256,
        replay_sha256: request
            .submission
            .submission
            .artifacts
            .replay
            .artifact
            .sha256,
        terminal_subject,
        final_campaign_sha256: run.final_campaign.sha256,
        final_state_sha256: run.final_state_sha256,
        observed_progression_percent: 100,
    });
    let result = VerificationResultV1 {
        schema_version: 1,
        request_id: request.request_id.clone(),
        verification_request_sha256: request_sha256,
        artifacts: request.submission.submission.artifacts.clone(),
        session_genesis_sha256: genesis_sha256,
        build_manifest_sha256: offer.build_manifest_sha256,
        content_manifest_sha256: offer.content_manifest_sha256,
        rules_config_sha256: offer.rules_config_sha256,
        ruleset_manifest_sha256: offer.ruleset_manifest_sha256,
        competition_manifest_sha256: offer.competition_manifest_sha256,
        input_provenance: Some(InputProvenanceStatusV1::Rankable),
        status: VerificationStatusV1::Verified(run),
    };
    (request, ruleset, campaign_content, result)
}

#[test]
fn continuation_and_final_requests_are_ordered_and_cross_bound() {
    let (verification, _, _, _) = completion_contract_fixture();
    let submission = &verification.submission.submission;
    let authorization = submission
        .campaign_continuation_authorization
        .as_ref()
        .unwrap();
    let continuation = authorization
        .claim
        .co_sign_request(&submission.offer)
        .unwrap();
    assert_eq!(
        continuation.instance.purpose,
        LeaderboardCoSignPurposeV1::CampaignContinuation
    );
    assert_eq!(
        authorization.signing_bytes(&submission.offer).unwrap(),
        continuation.signing_bytes().unwrap()
    );

    let final_request = submission.co_sign_request().unwrap();
    assert_eq!(
        final_request.instance.purpose,
        LeaderboardCoSignPurposeV1::Submission
    );
    assert_ne!(
        continuation.signing_bytes().unwrap(),
        final_request.signing_bytes().unwrap()
    );

    let mut another_offer = submission.offer.clone();
    another_offer.upload_challenge_nonce = ChallengeNonce32::from_bytes([0xa5; 32]);
    let another_session = authorization.claim.co_sign_request(&another_offer).unwrap();
    assert_ne!(continuation.instance, another_session.instance);
    assert_ne!(
        continuation.signing_bytes().unwrap(),
        another_session.signing_bytes().unwrap()
    );

    let mut different_replay = authorization.claim.clone();
    different_replay.next_artifacts.replay.artifact.sha256 = Digest32::from_bytes([0xb6; 32]);
    assert_ne!(
        continuation.run_digest,
        different_replay
            .co_sign_request(&submission.offer)
            .unwrap()
            .run_digest
    );
}

#[test]
fn completion_evidence_rejects_policy_catalog_and_request_substitution() {
    let (request, ruleset, campaign_content, result) = completion_contract_fixture();
    assert!(
        result
            .validate_campaign_complete_evidence(&request, &ruleset, Some(&campaign_content),)
            .is_ok()
    );

    let mut swapped_artifacts = result.clone();
    swapped_artifacts.artifacts.replay.artifact.sha256 = Digest32::from_bytes([99; 32]);
    assert!(
        swapped_artifacts
            .validate_campaign_complete_evidence(&request, &ruleset, Some(&campaign_content))
            .is_err()
    );

    let mut forged_result = result.clone();
    let VerificationStatusV1::Verified(run) = &mut forged_result.status else {
        unreachable!()
    };
    run.campaign_complete_evidence
        .as_mut()
        .unwrap()
        .observed_progression_percent = 99;
    assert!(
        forged_result
            .validate_campaign_complete_evidence(&request, &ruleset, Some(&campaign_content),)
            .is_err()
    );

    let mut missing = result.clone();
    let VerificationStatusV1::Verified(run) = &mut missing.status else {
        unreachable!()
    };
    run.campaign_complete_evidence = None;
    assert!(
        missing
            .validate_campaign_complete_evidence(&request, &ruleset, Some(&campaign_content),)
            .is_err()
    );

    let mut substituted_catalog = campaign_content.clone();
    substituted_catalog.entries[0].subject = OfficialContentSubjectV1::FieldMission {
        mission_id: "H11_Not_MP".into(),
    };
    assert!(
        result
            .validate_campaign_complete_evidence(&request, &ruleset, Some(&substituted_catalog),)
            .is_err()
    );

    let mut substituted_request = request.clone();
    substituted_request.request_id = id("different-verification");
    assert!(
        result
            .validate_campaign_complete_evidence(
                &substituted_request,
                &ruleset,
                Some(&campaign_content),
            )
            .is_err()
    );
}

#[test]
fn taints_collapse_to_stable_public_reasons() {
    let tainted = InputProvenanceStatusV1::Tainted {
        taints: vec![
            InputTaintV1 {
                kind: InputTaintKindV1::HttpPlayerCommand,
                first_frame: 3,
            },
            InputTaintV1 {
                kind: InputTaintKindV1::HttpSimulationStep,
                first_frame: 4,
            },
        ],
    };
    assert_eq!(
        tainted.public_reasons(),
        vec![InputIneligibilityReasonV1::HttpAutomation]
    );
}

fn verified_campaign() -> VerifiedCampaignAggregateV1 {
    let first = VerifiedCampaignSessionV1 {
        ordinal: 0,
        run_id: id("session-1"),
        kind: CampaignSessionKindV1::FieldMission {
            mission_id: "mission_1".into(),
        },
        content_subject: OfficialContentSubjectV1::FieldMission {
            mission_id: "mission_1".into(),
        },
        campaign_aggregation_consent:
            CampaignAggregationConsentV1::AuthorizeSignedSessionInServerRecognizedChainV1,
        replay: replay_artifact(1),
        build_manifest_sha256: Digest32::from_bytes([18; 32]),
        content_manifest_sha256: Digest32::from_bytes([29; 32]),
        rules_config_sha256: Digest32::from_bytes([30; 32]),
        ruleset_manifest_sha256: Digest32::from_bytes([31; 32]),
        competition_manifest_sha256: None,
        verification_request_sha256: Digest32::from_bytes([20; 32]),
        verification_result_sha256: Digest32::from_bytes([21; 32]),
        starting_campaign: campaign_artifact(22, 100),
        final_campaign: campaign_artifact(23, 110),
        starting_campaign_score: 0,
        final_campaign_score: 1_000,
        max_concurrent_players: 1,
        participant_instance_count: 1,
        named_participant_instance_count: 1,
        anonymous_participant_instance_count: 0,
        authenticated_participant_keys: vec![PublicKey32::from_bytes([28; 32])],
        active_simulation_ticks: 90,
        ransom_collected: 25,
        campaign_complete_evidence_sha256: None,
    };
    let second = VerifiedCampaignSessionV1 {
        ordinal: 1,
        run_id: id("session-2"),
        kind: CampaignSessionKindV1::Headquarters { hq_sequence: 1 },
        content_subject: OfficialContentSubjectV1::Headquarters {
            mission_id: "sherwood".into(),
        },
        campaign_aggregation_consent:
            CampaignAggregationConsentV1::AuthorizeSignedSessionInServerRecognizedChainV1,
        replay: replay_artifact(2),
        build_manifest_sha256: Digest32::from_bytes([17; 32]),
        content_manifest_sha256: first.content_manifest_sha256,
        rules_config_sha256: first.rules_config_sha256,
        ruleset_manifest_sha256: first.ruleset_manifest_sha256,
        competition_manifest_sha256: first.competition_manifest_sha256,
        verification_request_sha256: Digest32::from_bytes([24; 32]),
        verification_result_sha256: Digest32::from_bytes([25; 32]),
        starting_campaign: first.final_campaign.clone(),
        final_campaign: campaign_artifact(26, 120),
        starting_campaign_score: first.final_campaign_score,
        final_campaign_score: 1_050,
        max_concurrent_players: 1,
        participant_instance_count: 1,
        named_participant_instance_count: 1,
        anonymous_participant_instance_count: 0,
        authenticated_participant_keys: first.authenticated_participant_keys.clone(),
        active_simulation_ticks: 10,
        ransom_collected: 5,
        campaign_complete_evidence_sha256: Some(Digest32::from_bytes([27; 32])),
    };
    VerifiedCampaignAggregateV1 {
        schema_version: 1,
        aggregate_request_sha256: Digest32::from_bytes([19; 32]),
        chain_id: id("chain-1"),
        full_campaign_run_id: id("full-campaign-1"),
        campaign_complete_terminal_run_id: second.run_id.clone(),
        campaign_complete_evidence_sha256: Digest32::from_bytes([27; 32]),
        sessions: vec![first.clone(), second.clone()],
        max_concurrent_players: 1,
        participant_instance_count: 2,
        named_participant_instance_count: 2,
        anonymous_participant_instance_count: 0,
        authenticated_participant_keys: first.authenticated_participant_keys.clone(),
        campaign_controller_public_key: first.authenticated_participant_keys[0],
        campaign_content_manifest_sha256: Digest32::from_bytes([29; 32]),
        rules_config_sha256: Digest32::from_bytes([30; 32]),
        ruleset_manifest_sha256: Digest32::from_bytes([31; 32]),
        competition_manifest_sha256: None,
        canonical_genesis_campaign: first.starting_campaign.clone(),
        final_campaign: second.final_campaign.clone(),
        starting_campaign_score: first.starting_campaign_score,
        final_campaign_score: second.final_campaign_score,
        active_simulation_ticks: 100,
        ransom_collected: 30,
    }
}

#[test]
fn full_campaign_aggregate_requires_exact_chain_and_metric_sums() {
    let aggregate = verified_campaign();
    assert!(aggregate.validate().is_ok());
    assert_eq!(aggregate.metrics().original_score_delta, 1_050);
    assert_eq!(aggregate.metrics().active_simulation_ticks, 100);

    let mut broken = aggregate.clone();
    broken.sessions[1].starting_campaign.sha256 = Digest32::from_bytes([99; 32]);
    assert!(broken.validate().is_err());

    let mut wrong_score_continuity = aggregate.clone();
    wrong_score_continuity.sessions[1].starting_campaign_score -= 1;
    assert!(wrong_score_continuity.validate().is_err());

    let mut wrong_ransom = aggregate.clone();
    wrong_ransom.ransom_collected += 1;
    assert!(wrong_ransom.validate().is_err());

    let mut no_completion_evidence = aggregate.clone();
    no_completion_evidence.campaign_complete_evidence_sha256 = Digest32::default();
    assert!(no_completion_evidence.validate().is_err());

    let mut wrong_ticks = aggregate;
    wrong_ticks.active_simulation_ticks += 1;
    assert!(wrong_ticks.validate().is_err());
}

#[test]
fn campaign_roster_continuity_policy_is_enforced() {
    let mut aggregate = verified_campaign();
    let late_guest = PublicKey32::from_bytes([29; 32]);
    aggregate.sessions[1]
        .authenticated_participant_keys
        .push(late_guest);
    aggregate.sessions[1].named_participant_instance_count += 1;
    aggregate.sessions[1].participant_instance_count += 1;
    aggregate.named_participant_instance_count += 1;
    aggregate.participant_instance_count += 1;
    aggregate.authenticated_participant_keys.push(late_guest);

    assert!(aggregate.validate().is_ok());
    assert!(
        aggregate
            .validate_roster_continuity(CampaignRosterContinuityV1::UnionOfVerifiedSessionSubsets,)
            .is_ok()
    );
    assert!(matches!(
        aggregate.validate_roster_continuity(
            CampaignRosterContinuityV1::ExactSameAuthenticatedKeysEverySession,
        ),
        Err(ValidationError::ClaimMismatch {
            field: "verified_campaign.ruleset_roster_continuity"
        })
    ));
}

#[test]
fn worker_admission_failure_is_canonical_and_cannot_fake_proof_fields() {
    let output = VerifierWorkerOutputV1::AdmissionFailure {
        schema_version: crate::SCHEMA_VERSION_V1,
        request_artifact_sha256: Digest32::digest_bytes(br#"{"broken":true}"#),
        code: VerifierAdmissionFailureCodeV1::MalformedRequest,
        bounded_detail: Some("duplicate_json_key".into()),
    };
    assert!(output.validate().is_ok());
    let canonical = output.canonical_bytes().unwrap();
    assert_eq!(
        serde_json::from_slice::<VerifierWorkerOutputV1>(&canonical).unwrap(),
        output
    );

    let mut missing_artifact_identity = output.clone();
    let VerifierWorkerOutputV1::AdmissionFailure {
        request_artifact_sha256,
        ..
    } = &mut missing_artifact_identity
    else {
        unreachable!()
    };
    *request_artifact_sha256 = Digest32::default();
    assert!(missing_artifact_identity.validate().is_err());

    let unknown = br#"{"outcome":"admission_failure","schema_version":1,"request_artifact_sha256":"abababababababababababababababababababababababababababababababab","code":"malformed_request","bounded_detail":null,"invented_request_id":"forbidden"}"#;
    assert!(serde_json::from_slice::<VerifierWorkerOutputV1>(unknown).is_err());
}
