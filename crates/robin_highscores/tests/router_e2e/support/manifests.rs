use super::*;

pub(crate) fn verification_limits() -> VerificationLimitsV1 {
    VerificationLimitsV1 {
        max_input_bytes: 1024 * 1024,
        max_compressed_bytes: 1024 * 1024,
        max_decompressed_bytes: 4 * 1024 * 1024,
        max_base64_payload_bytes: 4 * 1024 * 1024,
        max_campaign_bytes: 1024 * 1024,
        max_frames: 100_000,
        max_version_bytes: 256,
        max_mission_id_bytes: 256,
        max_metadata_records: 10_000,
        max_entries_per_frame: 1_000,
    }
}

pub(crate) fn artifact(byte: u8, media_type: &str) -> ArtifactRefV1 {
    ArtifactRefV1 {
        sha256: Digest32::from_bytes([byte; 32]),
        byte_length: 1,
        media_type: media_type.to_owned(),
    }
}

pub(crate) fn build_manifest() -> BuildManifestV1 {
    let mut source_commit = robin_replay_format::ENGINE_VERSION_HASH.to_owned();
    source_commit.push_str(&"0".repeat(40 - source_commit.len()));
    BuildManifestV1 {
        schema_version: SCHEMA_VERSION_V1,
        source_commit,
        cargo_lock_sha256: Digest32::from_bytes([1; 32]),
        target_triple: "x86_64-unknown-linux-gnu".to_owned(),
        cargo_profile: "release".to_owned(),
        cargo_features: vec!["replay".to_owned()],
        replay_schema_version: REPLAY_SCHEMA_VERSION,
        save_schema_version: robin_run_protocol::CURRENT_RANKED_SAVE_SCHEMA_VERSION_V1,
        network_protocol_version: NETWORK_PROTOCOL_VERSION,
        verifier: artifact(2, "application/x-executable"),
        viewer_artifacts: vec![
            NamedArtifactV1 {
                path: "viewer/entry.js".to_owned(),
                role: ViewerArtifactRoleV1::EntryJavaScript,
                artifact: artifact(3, "text/javascript"),
            },
            NamedArtifactV1 {
                path: "viewer/robin.wasm".to_owned(),
                role: ViewerArtifactRoleV1::WebAssembly,
                artifact: artifact(4, "application/wasm"),
            },
        ],
    }
}

pub(crate) fn content_manifest() -> ContentManifestV1 {
    let components = [
        SimulationContentComponentKindV1::Profiles,
        SimulationContentComponentKindV1::LoadedLevel,
        SimulationContentComponentKindV1::MissionScripts,
        SimulationContentComponentKindV1::SpriteSimulationMetadata,
        SimulationContentComponentKindV1::MapGeometryMetadata,
        SimulationContentComponentKindV1::LocalizedDeterministicText,
        SimulationContentComponentKindV1::SoundDurationTables,
        SimulationContentComponentKindV1::InterfaceSimulationMetadata,
    ]
    .into_iter()
    .enumerate()
    .map(|(index, kind)| SimulationContentComponentV1 {
        kind,
        component_schema_version: 1,
        artifact: artifact(
            u8::try_from(index).unwrap() + 20,
            SIMULATION_CONTENT_COMPONENT_MEDIA_TYPE_V1,
        ),
    })
    .collect();
    ContentManifestV1 {
        schema_version: SCHEMA_VERSION_V1,
        name: "Demo Leicester".to_owned(),
        edition: OfficialContentEditionV1::Demo,
        subject: OfficialContentSubjectV1::FieldMission {
            mission_id: MISSION_ID.to_owned(),
        },
        resource_locale_root: ResourceLocaleRootV1::new("1033").unwrap(),
        closure: ContentClosureKindV1::StaticPreparedMissionContentProjection,
        projection_schema_version: 1,
        speech_timing: SimulationSpeechTimingSourceV1::BaseInstallation,
        components,
    }
}

pub(crate) fn full_content_manifest() -> ContentManifestV1 {
    let mut manifest = content_manifest();
    manifest.name = "Full Leicester".to_owned();
    manifest.edition = OfficialContentEditionV1::Full;
    manifest.subject = OfficialContentSubjectV1::FieldMission {
        mission_id: CAMPAIGN_MISSION_ID.to_owned(),
    };
    manifest.resource_locale_root = ResourceLocaleRootV1::new("2047").unwrap();
    manifest
}

pub(crate) fn genesis_content_manifest() -> ContentManifestV1 {
    let mut manifest = full_content_manifest();
    manifest.name = "Full H01 campaign genesis".to_owned();
    manifest.subject = OfficialContentSubjectV1::FieldMission {
        mission_id: GENESIS_MISSION_ID.to_owned(),
    };
    for (index, component) in manifest.components.iter_mut().enumerate() {
        component.artifact.sha256 =
            Digest32::from_bytes([u8::try_from(index).unwrap().wrapping_add(100); 32]);
    }
    manifest
}

pub(crate) fn terminal_content_manifest() -> ContentManifestV1 {
    let mut manifest = full_content_manifest();
    manifest.name = "Full H12".to_owned();
    manifest.subject = OfficialContentSubjectV1::FieldMission {
        mission_id: HQ_MISSION_ID.to_owned(),
    };
    for (index, component) in manifest.components.iter_mut().enumerate() {
        component.artifact.sha256 =
            Digest32::from_bytes([u8::try_from(index).unwrap().wrapping_add(120); 32]);
    }
    manifest
}

pub(crate) fn policy_identity(kind: ImmutablePolicyKindV1, byte: u8) -> ImmutablePolicyIdentityV1 {
    ImmutablePolicyIdentityV1 {
        kind,
        version: 1,
        manifest_sha256: Digest32::from_bytes([byte; 32]),
    }
}

pub(crate) fn published_ruleset(
    build_sha256: Digest32,
    content_sha256: Vec<Digest32>,
    rules_config_sha256: Digest32,
    campaign_content_sha256: Option<Digest32>,
) -> robin_run_protocol::PublishedRulesetV1 {
    let campaign = campaign_content_sha256.is_some();
    let board_scopes = if campaign {
        vec![
            RulesetBoardScopeV1::CampaignMission,
            RulesetBoardScopeV1::FullCampaign,
        ]
    } else {
        vec![RulesetBoardScopeV1::IndividualLevel]
    };
    let manifest = RulesetManifestV1 {
        schema_version: SCHEMA_VERSION_V1,
        display_name: "Standard / Normal".to_owned(),
        preset_id: OpaqueId::new("standard").unwrap(),
        preset_name: "Standard".to_owned(),
        difficulty_id: OpaqueId::new("normal").unwrap(),
        difficulty_name: "Normal".to_owned(),
        rules_config_sha256,
        rules_config_constraint: RulesConfigConstraintV1::ExactCanonicalDigestOnly,
        allowed_build_manifest_sha256: vec![build_sha256],
        allowed_content_manifest_sha256: content_sha256,
        allowed_campaign_content_manifest_sha256: campaign_content_sha256
            .into_iter()
            .collect(),
        board_scopes,
        campaign_completion_policy: campaign_content_sha256.map_or(
            CampaignCompletionPolicyRequirementV1::NotOffered,
            |_| {
                CampaignCompletionPolicyRequirementV1::Required(
                    official_full_campaign_completion_policy_v1(),
                )
            },
        ),
        metrics: vec![BoardMetricV1::OriginalScore, BoardMetricV1::FastestSuccess],
        metric_ranking: vec![
            MetricRankingPolicyV1::OriginalScoreDescending,
            MetricRankingPolicyV1::FastestSuccessAscending,
        ],
        achievement_policies: official_achievement_policies_v1(),
        canonical_start_policy:
            CanonicalStartPolicyV1::RulesConfigBoundOperatorStateAndVerifiedPredecessor,
        canonical_campaign_state: CanonicalCampaignStateRequirementV1 {
            edition: if campaign {
                OfficialContentEditionV1::Full
            } else {
                OfficialContentEditionV1::Demo
            },
            kind: if campaign {
                CanonicalCampaignStateKindV1::FullCampaignGenesis
            } else {
                CanonicalCampaignStateKindV1::IndividualTemplate
            },
            rules_config_sha256,
        },
        run_preflight_grant_public_key: PublicKey32::from_bytes(
            SigningKey::from_bytes(&[0x46; 32])
                .verifying_key()
                .to_bytes(),
        ),
        full_campaign_chain_policy:
            FullCampaignChainPolicyV1::CanonicalGenesisEveryFieldAndHeadquartersSessionIndependentCompletion,
        campaign_roster_continuity: CampaignRosterContinuityV1::UnionOfVerifiedSessionSubsets,
        campaign_aggregation_consent_policy:
            CampaignAggregationConsentPolicyV1::EveryAuthenticatedKeyFinalCosignsEachSession,
        participant_eligibility: ParticipantEligibilityV1 {
            allow_single_player: true,
            allow_multiplayer: true,
            named_policy:
                NamedParticipantPolicyV1::HostGenesisGuestTransportJoinAttestationAndFinalCosign,
            anonymous_policy:
                AnonymousParticipantPolicyV1::AllowedAuthenticatedButPubliclyRedacted,
            minimum_max_concurrent_players: 1,
            maximum_max_concurrent_players: robin_run_protocol::MAX_REPLAY_SEATS_V1,
            maximum_participant_instances: robin_run_protocol::MAX_PARTICIPANT_INSTANCES_V1,
        },
        replay_schema_versions: vec![REPLAY_SCHEMA_VERSION],
        network_protocol_versions: vec![NETWORK_PROTOCOL_VERSION],
        input_provenance_policy: policy_identity(ImmutablePolicyKindV1::InputProvenance, 10),
        command_admission_policy: policy_identity(ImmutablePolicyKindV1::CommandAdmission, 11),
        submission_admission_policy: policy_identity(
            ImmutablePolicyKindV1::SubmissionAdmission,
            12,
        ),
        verifier_policy: policy_identity(ImmutablePolicyKindV1::Verification, 13),
        input_provenance_eligibility:
            InputProvenanceEligibilityV1::CurrentSchemaCanonicalReplayOnly,
        terminal_result_policy: TerminalResultPolicyV1::IndependentlyReachedWonOnly,
        score_algorithm: ScoreAlgorithmV1::OriginalMissionAttemptWrappingSubtotalCampaignDeltaV1,
        score_overflow_policy: ScoreOverflowPolicyV1::RejectCampaignOrAggregateOverflow,
        visible_tie_policy: VisibleTiePolicyV1::EqualPrimaryMetricSharesRank,
        pagination_tie_break:
            PaginationTieBreakV1::AcceptedSequenceThenVerificationTimeThenRunIdOnly,
        tick_duration: TickDurationV1 {
            numerator_micros: 50_000,
            denominator: 1,
        },
        active_time_definition: ActiveTimeDefinitionV1::SuccessfulSimulationTicks,
        frame_counting_policy:
            FrameCountingPolicyV1::ZeroBasedEventsBeforeExclusiveReplayFrameCount,
        full_campaign_time_aggregation:
            FullCampaignTimeAggregationV1::CheckedSumEveryVerifiedFieldAndHeadquartersSession,
        run_composition_policy:
            RunCompositionPolicyV1::MissionSingleReplayFullCampaignOrderedSessionsNoSyntheticReplay,
        main_board_seed_policy: RulesetSeedPolicyV1::Open,
        competition_seed_policy: RulesetSeedPolicyV1::ServerPinned,
        allow_save_creation: true,
        allow_autosave: true,
        allow_state_load: false,
        allow_mission_restart: false,
    };
    let ruleset_manifest_sha256 = manifest.canonical_digest().unwrap();
    robin_run_protocol::PublishedRulesetV1 {
        schema_version: SCHEMA_VERSION_V1,
        ruleset_manifest_sha256,
        manifest,
        operational_status: RulesetOperationalStatusV1::Active,
    }
}
