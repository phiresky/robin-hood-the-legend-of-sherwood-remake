//! Shared deterministic service fixtures. No production authority is created implicitly.

use crate::config::{AdmissionProfile, ViewerContentRequirementConfig};
use robin_run_protocol::CURRENT_RANKED_REPLAY_SCHEMA_VERSION_V1 as RANKED_REPLAY_SCHEMA_VERSION;
use robin_run_protocol::{
    ActiveTimeDefinitionV1, AnonymousParticipantPolicyV1, ArtifactRefV1, BoardMetricV1,
    BrowserIdentitySignerBuildIdentityV2, BrowserIdentitySignerBuildRecipeV2,
    BrowserIdentitySignerDeploymentPolicyV2, BrowserPagesArtifactV2,
    BrowserPagesShellBuildIdentityV2, BrowserPagesShellBuildRecipeV2, BrowserViewerBuildIdentityV2,
    BrowserViewerEngineBuildIdentityV2, BrowserViewerEngineBuildRecipeV2, BuildManifestV1,
    BuildManifestV2, BuildToolAuthorityDocumentV1, BuildToolAuthorityV1,
    CampaignAggregationConsentPolicyV1, CampaignCompletionPolicyRequirementV1,
    CampaignRosterContinuityV1, CanonicalCampaignStateKindV1, CanonicalCampaignStatePinV1,
    CanonicalCampaignStateRequirementV1, CanonicalDocument, CanonicalStartPolicyV1,
    ContentClosureKindV1, ContentManifestV1, Digest32, FrameCountingPolicyV1,
    FullCampaignChainPolicyV1, FullCampaignTimeAggregationV1, ImmutablePolicyIdentityV1,
    ImmutablePolicyKindV1, InputProvenanceEligibilityV1, MetricRankingPolicyV1, NamedArtifactV1,
    NamedParticipantPolicyV1, NativeBuildPlatformV2, NativeLinkageV2, OfficialContentEditionV1,
    OfficialContentSubjectV1, PaginationTieBreakV1, ParticipantEligibilityV1, PublicKey32,
    PublishedRulesetV1, RANKED_REPLAY_VERIFIER_MEDIA_TYPE_V2, RulesConfigConstraintV1,
    RulesetBoardScopeV1, RulesetManifestV1, RulesetOperationalStatusV1, RulesetSeedPolicyV1,
    RunCompositionPolicyV1, RustToolchainAuthorityV1, SCHEMA_VERSION_V1,
    SIMULATION_CONTENT_COMPONENT_MEDIA_TYPE_V1, ScoreAlgorithmV1, ScoreOverflowPolicyV1,
    SimulationContentComponentKindV1, SimulationContentComponentV1, SimulationSpeechTimingSourceV1,
    TerminalResultPolicyV1, TickDurationV1, Validate, VerifierBuildIdentityV2,
    ViewerArtifactRoleV1, VisibleTiePolicyV1, official_achievement_policies_v1,
    official_full_campaign_completion_policy_v1,
};

pub(crate) fn artifact(byte: u8, media_type: &str) -> ArtifactRefV1 {
    ArtifactRefV1 {
        sha256: Digest32::from_bytes([byte; 32]),
        byte_length: 1,
        media_type: media_type.to_owned(),
    }
}

pub(crate) fn simulation_components() -> Vec<SimulationContentComponentV1> {
    [
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
    .collect()
}

pub(crate) fn viewer_build() -> BuildManifestV1 {
    BuildManifestV1 {
        schema_version: SCHEMA_VERSION_V1,
        source_commit: "a".repeat(40),
        cargo_lock_sha256: Digest32::from_bytes([1; 32]),
        target_triple: "wasm32-unknown-emscripten".to_owned(),
        cargo_profile: "release".to_owned(),
        cargo_features: vec!["replay".to_owned()],
        replay_schema_version: RANKED_REPLAY_SCHEMA_VERSION,
        save_schema_version: robin_run_protocol::CURRENT_RANKED_SAVE_SCHEMA_VERSION_V1,
        network_protocol_version: robin_run_protocol::CURRENT_RANKED_NETWORK_PROTOCOL_VERSION_V1,
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

pub(crate) fn build_tool(byte: u8, version: &str) -> BuildToolAuthorityV1 {
    BuildToolAuthorityV1 {
        version: version.to_owned(),
        authority_sha256: if version == robin_run_protocol::WASM_BINDGEN_CLI_VERSION_V1 {
            robin_run_protocol::WASM_BINDGEN_CLI_AUTHORITY_SHA256_V1
        } else {
            Digest32::from_bytes([byte; 32])
        },
    }
}

pub(crate) fn viewer_build_v2() -> BuildManifestV2 {
    let binaryen_authority: BuildToolAuthorityDocumentV1 = serde_json::from_str(include_str!(
        "../../../.github/tool-authorities/binaryen-wasm-opt-v132.json"
    ))
    .unwrap();
    let wabt_authority: BuildToolAuthorityDocumentV1 = serde_json::from_str(include_str!(
        "../../../.github/tool-authorities/wabt-wasm-strip-v1.0.41.json"
    ))
    .unwrap();
    let rust_toolchain = RustToolchainAuthorityV1 {
        schema_version: SCHEMA_VERSION_V1,
        channel: "nightly-2026-08-25".to_owned(),
        components: vec![
            "rust-src".to_owned(),
            "rustc-codegen-cranelift-preview".to_owned(),
        ],
        targets: vec!["wasm32-unknown-unknown".to_owned()],
    };
    let rust_toolchain_sha256 = rust_toolchain.canonical_digest().unwrap();
    BuildManifestV2 {
        schema_version: 2,
        source_commit: "b".repeat(40),
        cargo_lock_sha256: Digest32::from_bytes([41; 32]),
        replay_schema_version: RANKED_REPLAY_SCHEMA_VERSION,
        save_schema_version: robin_run_protocol::CURRENT_RANKED_SAVE_SCHEMA_VERSION_V1,
        network_protocol_version:
            robin_run_protocol::CURRENT_RANKED_NETWORK_PROTOCOL_VERSION_V1,
        verifier: VerifierBuildIdentityV2 {
            platform: NativeBuildPlatformV2::X86_64UnknownLinuxMusl,
            target_triple: "x86_64-unknown-linux-musl".to_owned(),
            cargo_profile: "release".to_owned(),
            cargo_features: Vec::new(),
            cargo_package: "robin_replay_verifier".to_owned(),
            cargo_binary: "robin-replay-verifier".to_owned(),
            linkage: NativeLinkageV2::FullyStaticNoInterpreterOrNeededLibraries,
            artifact: artifact(42, RANKED_REPLAY_VERIFIER_MEDIA_TYPE_V2),
        },
        viewer: BrowserViewerBuildIdentityV2 {
            engine: BrowserViewerEngineBuildIdentityV2 {
                target_triple: "wasm32-unknown-unknown".to_owned(),
                cargo_profile: "wasm-release".to_owned(),
                cargo_features: vec!["audio".to_owned()],
                cargo_package: "robin_rs".to_owned(),
                cargo_binary: "robin".to_owned(),
                recipe: BrowserViewerEngineBuildRecipeV2::WasmBindgenWebBinaryenOzStripDebugDwarfWabtStripV1,
                rust_toolchain: rust_toolchain.clone(),
                rust_toolchain_sha256,
                wasm_bindgen_cli: build_tool(43, "0.2.127"),
                binaryen_wasm_opt: BuildToolAuthorityV1 {
                    version: binaryen_authority.version.clone(),
                    authority_sha256: binaryen_authority.canonical_digest().unwrap(),
                },
                wabt_wasm_strip: BuildToolAuthorityV1 {
                    version: wabt_authority.version.clone(),
                    authority_sha256: wabt_authority.canonical_digest().unwrap(),
                },
                artifacts: vec![
                    NamedArtifactV1 {
                        path: "viewer/robin.js".to_owned(),
                        role: ViewerArtifactRoleV1::EntryJavaScript,
                        artifact: artifact(46, "text/javascript"),
                    },
                    NamedArtifactV1 {
                        path: "viewer/robin_bg.wasm".to_owned(),
                        role: ViewerArtifactRoleV1::WebAssembly,
                        artifact: artifact(47, "application/wasm"),
                    },
                ],
            },
            pages_shell: BrowserPagesShellBuildIdentityV2 {
                recipe: BrowserPagesShellBuildRecipeV2::PnpmFrozenLockfileViteStaticShellV1,
                node: build_tool(48, "24.19.0"),
                pnpm: build_tool(49, "12.3.4"),
                package_json_sha256: Digest32::from_bytes([50; 32]),
                pnpm_lock_sha256: Digest32::from_bytes([51; 32]),
                public_origin_artifacts: vec![BrowserPagesArtifactV2 {
                    path: "index.html".to_owned(),
                    artifact: artifact(52, "text/html"),
                }],
            },
            identity_signer: BrowserIdentitySignerBuildIdentityV2 {
                target_triple: "wasm32-unknown-unknown".to_owned(),
                cargo_profile: "wasm-release".to_owned(),
                cargo_features: vec!["identity-signer-bridge".to_owned()],
                cargo_package: "robin_rs".to_owned(),
                cargo_binary: "leaderboard_identity_bridge".to_owned(),
                recipe: BrowserIdentitySignerBuildRecipeV2::WasmBindgenWebSeparateOriginBridgeV1,
                deployment_policy: BrowserIdentitySignerDeploymentPolicyV2::SeparateAllowlistedOriginCspFrameAncestorsAndBridgeShaV1,
                rust_toolchain,
                rust_toolchain_sha256,
                wasm_bindgen_cli: build_tool(43, "0.2.127"),
                identity_signer_origin_artifacts: vec![
                    BrowserPagesArtifactV2 {
                        path: "identity-signer/bridge/leaderboard_identity_bridge.js"
                            .to_owned(),
                        artifact: artifact(55, "text/javascript"),
                    },
                    BrowserPagesArtifactV2 {
                        path: "identity-signer/bridge/leaderboard_identity_bridge_bg.wasm"
                            .to_owned(),
                        artifact: artifact(56, "application/wasm"),
                    },
                    BrowserPagesArtifactV2 {
                        path: "identity-signer/index.html".to_owned(),
                        artifact: artifact(54, "text/html"),
                    },
                ],
            },
        },
    }
}

pub(crate) fn viewer_profile(build: Digest32, content: Digest32) -> AdmissionProfile {
    AdmissionProfile {
        id: "viewer-profile".to_owned(),
        content_subject: OfficialContentSubjectV1::FieldMission {
            mission_id: "mission".to_owned(),
        },
        mission_display_name: "Mission".to_owned(),
        allowed_scopes: vec!["individual_level".to_owned()],
        build_manifest_id: build.to_string(),
        content_manifest_id: content.to_string(),
        campaign_content_manifest_id: None,
        config_id: Digest32::from_bytes([5; 32]).to_string(),
        ruleset_id: Digest32::from_bytes([6; 32]).to_string(),
        template_id: "template".to_owned(),
        canonical_campaign_state: CanonicalCampaignStatePinV1 {
            requirement: CanonicalCampaignStateRequirementV1 {
                edition: OfficialContentEditionV1::Demo,
                kind: CanonicalCampaignStateKindV1::IndividualTemplate,
                rules_config_sha256: Digest32::from_bytes([5; 32]),
            },
            artifact: ArtifactRefV1 {
                sha256: Digest32::from_bytes([7; 32]),
                byte_length: 7,
                media_type: robin_run_protocol::RANKED_CAMPAIGN_MEDIA_TYPE_V1.to_owned(),
            },
        },
        canonical_campaign_state_path: Some(std::path::PathBuf::from("/private/campaign-state")),
        allowed_metrics: vec!["original_score".to_owned()],
        ruleset_display_name: "Standard".to_owned(),
        preset_id: "standard".to_owned(),
        preset_name: "Standard".to_owned(),
        difficulty_id: "normal".to_owned(),
        difficulty_name: "Normal".to_owned(),
        build_display_name: "Viewer build".to_owned(),
        viewer_engine_build: "viewer".to_owned(),
        viewer_available: true,
        viewer_unavailable_reason: None,
        viewer_content_requirement: Some(ViewerContentRequirementConfig::BundledDemo),
    }
}

pub(crate) fn viewer_content_manifest(edition: OfficialContentEditionV1) -> ContentManifestV1 {
    ContentManifestV1 {
        schema_version: SCHEMA_VERSION_V1,
        name: match edition {
            OfficialContentEditionV1::Demo => "demo",
            OfficialContentEditionV1::Full => "full",
        }
        .to_owned(),
        edition,
        subject: OfficialContentSubjectV1::FieldMission {
            mission_id: "mission".to_owned(),
        },
        closure: ContentClosureKindV1::StaticPreparedMissionContentProjection,
        projection_schema_version: 1,
        resource_locale_root: robin_run_protocol::ResourceLocaleRootV1::new(match edition {
            OfficialContentEditionV1::Demo => "1033",
            OfficialContentEditionV1::Full => "2047",
        })
        .unwrap(),
        speech_timing: SimulationSpeechTimingSourceV1::BaseInstallation,
        components: simulation_components(),
    }
}

pub(crate) fn policy_identity(kind: ImmutablePolicyKindV1, byte: u8) -> ImmutablePolicyIdentityV1 {
    ImmutablePolicyIdentityV1 {
        kind,
        version: 1,
        manifest_sha256: Digest32::from_bytes([byte; 32]),
    }
}

pub(crate) fn published_ruleset_fixture() -> PublishedRulesetV1 {
    let manifest = RulesetManifestV1 {
        schema_version: SCHEMA_VERSION_V1,
        display_name: "Standard / Normal".to_owned(),
        preset_id: OpaqueId::new("standard").unwrap(),
        preset_name: "Standard".to_owned(),
        difficulty_id: OpaqueId::new("normal").unwrap(),
        difficulty_name: "Normal".to_owned(),
        rules_config_sha256: Digest32::from_bytes([5; 32]),
        rules_config_constraint: RulesConfigConstraintV1::ExactCanonicalDigestOnly,
        allowed_build_manifest_sha256: vec![Digest32::from_bytes([8; 32])],
        allowed_content_manifest_sha256: vec![Digest32::from_bytes([9; 32])],
        allowed_campaign_content_manifest_sha256: vec![Digest32::from_bytes([10; 32])],
        board_scopes: vec![
            RulesetBoardScopeV1::IndividualLevel,
            RulesetBoardScopeV1::CampaignMission,
            RulesetBoardScopeV1::FullCampaign,
        ],
        campaign_completion_policy: CampaignCompletionPolicyRequirementV1::Required(
            official_full_campaign_completion_policy_v1(),
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
            edition: OfficialContentEditionV1::Demo,
            kind: CanonicalCampaignStateKindV1::IndividualTemplate,
            rules_config_sha256: Digest32::from_bytes([5; 32]),
        },
        run_preflight_grant_public_key: PublicKey32::from_bytes(
            ed25519_dalek::SigningKey::from_bytes(&[0x46; 32])
                .verifying_key()
                .to_bytes(),
        ),
        full_campaign_chain_policy:
            FullCampaignChainPolicyV1::CanonicalGenesisEveryFieldAndHeadquartersSessionIndependentCompletion,
        campaign_roster_continuity:
            CampaignRosterContinuityV1::UnionOfVerifiedSessionSubsets,
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
            maximum_participant_instances:
                robin_run_protocol::MAX_PARTICIPANT_INSTANCES_V1,
        },
        replay_schema_versions: vec![RANKED_REPLAY_SCHEMA_VERSION],
        network_protocol_versions: vec![
            robin_run_protocol::CURRENT_RANKED_NETWORK_PROTOCOL_VERSION_V1,
        ],
        input_provenance_policy: policy_identity(
            ImmutablePolicyKindV1::InputProvenance,
            10,
        ),
        command_admission_policy: policy_identity(
            ImmutablePolicyKindV1::CommandAdmission,
            11,
        ),
        submission_admission_policy: policy_identity(
            ImmutablePolicyKindV1::SubmissionAdmission,
            12,
        ),
        verifier_policy: policy_identity(ImmutablePolicyKindV1::Verification, 13),
        input_provenance_eligibility:
            InputProvenanceEligibilityV1::CurrentSchemaCanonicalReplayOnly,
        terminal_result_policy: TerminalResultPolicyV1::IndependentlyReachedWonOnly,
        score_algorithm:
            ScoreAlgorithmV1::OriginalMissionAttemptWrappingSubtotalCampaignDeltaV1,
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
    let published = PublishedRulesetV1 {
        schema_version: SCHEMA_VERSION_V1,
        ruleset_manifest_sha256,
        manifest,
        operational_status: RulesetOperationalStatusV1::Active,
    };
    published.validate().unwrap();
    published
}

pub(crate) async fn app_state(
    config: crate::ServerConfig,
    campaign_directory: std::path::PathBuf,
) -> crate::web::AppState {
    crate::web::AppState {
        database: crate::Database::migrate(&config).await.unwrap(),
        replay_store: crate::ReplayStore::create(config.replay_directory.clone(), 1024)
            .await
            .unwrap(),
        campaign_store: crate::CampaignStore::create(campaign_directory, 1024)
            .await
            .unwrap(),
        config,
        cursor_hmac_key: [1; 32],
        backup_authority_hmac_key: [1; 32],
        competition_run_grant_secret_key: None,
        run_preflight_grant_secret_key: None,
        challenge_rate_limiter: crate::web::ChallengeRateLimiter::new(10),
    }
}
