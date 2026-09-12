use super::*;
use crate::test_fixtures::fact;

#[cfg(target_os = "linux")]
fn synthetic_validated_staging_v3(
    staging: &PinnedPublicationStagingV3,
) -> Result<ValidatedPublicationV3> {
    Ok(ValidatedPublicationV3 {
        root_path: staging.path.clone(),
        root: staging.root.try_clone()?,
        root_parent_path: staging.parent_path.clone(),
        root_parent: staging.parent.try_clone()?,
        root_parent_identity: publication_node_identity_v3(&staging.parent.metadata()?),
        root_name: staging.name.clone(),
        inventory: publication_tree_inventory_v3_from_fd(staging.path(), &staging.root)?,
        lock_sha256: Digest32::digest_bytes(b"synthetic PublicationV3 lock"),
    })
}

#[cfg(target_os = "linux")]
fn synthetic_cloudflare_materialization_authority_v1(
    source: &Path,
) -> Result<(
    ValidatedPublicationV3,
    CloudflareMaterializationProvenanceV1,
    Vec<CloudflareMaterializedOriginInventoryV1>,
)> {
    use robin_run_protocol::{
        BrowserIdentitySignerBuildIdentityV2, BrowserIdentitySignerBuildRecipeV2,
        BrowserIdentitySignerDeploymentPolicyV2, BrowserPagesArtifactV2,
        BrowserPagesShellBuildIdentityV2, BrowserPagesShellBuildRecipeV2,
        BrowserViewerBuildIdentityV2, BrowserViewerEngineBuildIdentityV2,
        BrowserViewerEngineBuildRecipeV2, BuildToolAuthorityV1, NamedArtifactV1,
        NativeBuildPlatformV2, NativeLinkageV2, RANKED_REPLAY_VERIFIER_MEDIA_TYPE_V2,
        RustToolchainAuthorityV1, VerifierBuildIdentityV2, ViewerArtifactRoleV1,
        WASM_BINDGEN_CLI_AUTHORITY_SHA256_V1, WASM_BINDGEN_CLI_VERSION_V1,
    };

    let artifact = |bytes: &[u8], media_type: &str| ArtifactRefV1 {
        sha256: Digest32::digest_bytes(bytes),
        byte_length: u64::try_from(bytes.len()).unwrap(),
        media_type: media_type.into(),
    };
    let tool = |byte: u8, version: &str| BuildToolAuthorityV1 {
        version: version.into(),
        authority_sha256: if version == WASM_BINDGEN_CLI_VERSION_V1 {
            WASM_BINDGEN_CLI_AUTHORITY_SHA256_V1
        } else {
            Digest32::from_bytes([byte; 32])
        },
    };
    let binaryen_authority: BuildToolAuthorityDocumentV1 =
        serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../.github/tool-authorities/binaryen-wasm-opt-v132.json"
        )))?;
    let wabt_authority: BuildToolAuthorityDocumentV1 =
        serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../.github/tool-authorities/wabt-wasm-strip-v1.0.41.json"
        )))?;
    let rust_toolchain = RustToolchainAuthorityV1 {
        schema_version: 1,
        channel: "nightly-2026-08-25".into(),
        components: vec!["rust-src".into(), "rustc-codegen-cranelift-preview".into()],
        targets: vec!["wasm32-unknown-unknown".into()],
    };
    let engine_js = b"synthetic engine JavaScript";
    let engine_wasm = b"synthetic engine WebAssembly";
    let public_headers = b"synthetic public headers";
    let public_index = b"synthetic public index";
    let signer_js = b"synthetic signer JavaScript";
    let signer_wasm = b"synthetic signer WebAssembly";
    let signer_index = b"synthetic signer index";
    let build = BuildManifestV2 {
        schema_version: 2,
        source_commit: "a".repeat(40),
        cargo_lock_sha256: Digest32::digest_bytes(b"Cargo.lock"),
        replay_schema_version: robin_run_protocol::CURRENT_RANKED_REPLAY_SCHEMA_VERSION_V1,
        save_schema_version: robin_run_protocol::CURRENT_RANKED_SAVE_SCHEMA_VERSION_V1,
        network_protocol_version:
            robin_run_protocol::CURRENT_RANKED_NETWORK_PROTOCOL_VERSION_V1,
        verifier: VerifierBuildIdentityV2 {
            platform: NativeBuildPlatformV2::X86_64UnknownLinuxMusl,
            target_triple: "x86_64-unknown-linux-musl".into(),
            cargo_profile: "release".into(),
            cargo_features: vec![],
            cargo_package: "robin_replay_verifier".into(),
            cargo_binary: "robin-replay-verifier".into(),
            linkage: NativeLinkageV2::FullyStaticNoInterpreterOrNeededLibraries,
            artifact: artifact(b"synthetic verifier", RANKED_REPLAY_VERIFIER_MEDIA_TYPE_V2),
        },
        viewer: BrowserViewerBuildIdentityV2 {
            engine: BrowserViewerEngineBuildIdentityV2 {
                target_triple: "wasm32-unknown-unknown".into(),
                cargo_profile: "wasm-release".into(),
                cargo_features: vec!["audio".into()],
                cargo_package: "robin_rs".into(),
                cargo_binary: "robin".into(),
                recipe: BrowserViewerEngineBuildRecipeV2::WasmBindgenWebBinaryenOzStripDebugDwarfWabtStripV1,
                rust_toolchain_sha256: rust_toolchain.canonical_digest()?,
                rust_toolchain: rust_toolchain.clone(),
                wasm_bindgen_cli: tool(16, "0.2.127"),
                binaryen_wasm_opt: BuildToolAuthorityV1 {
                    version: binaryen_authority.version.clone(),
                    authority_sha256: binaryen_authority.canonical_digest()?,
                },
                wabt_wasm_strip: BuildToolAuthorityV1 {
                    version: wabt_authority.version.clone(),
                    authority_sha256: wabt_authority.canonical_digest()?,
                },
                artifacts: vec![
                    NamedArtifactV1 {
                        path: "viewer/robin.js".into(),
                        role: ViewerArtifactRoleV1::EntryJavaScript,
                        artifact: artifact(engine_js, "text/javascript"),
                    },
                    NamedArtifactV1 {
                        path: "viewer/robin_bg.wasm".into(),
                        role: ViewerArtifactRoleV1::WebAssembly,
                        artifact: artifact(engine_wasm, "application/wasm"),
                    },
                ],
            },
            pages_shell: BrowserPagesShellBuildIdentityV2 {
                recipe: BrowserPagesShellBuildRecipeV2::PnpmFrozenLockfileViteStaticShellV1,
                node: tool(22, "24.19.0"),
                pnpm: tool(19, "12.3.4"),
                package_json_sha256: Digest32::digest_bytes(b"package.json"),
                pnpm_lock_sha256: Digest32::digest_bytes(b"pnpm-lock.yaml"),
                public_origin_artifacts: vec![
                    BrowserPagesArtifactV2 {
                        path: "_headers".into(),
                        artifact: artifact(public_headers, "text/plain"),
                    },
                    BrowserPagesArtifactV2 {
                        path: "index.html".into(),
                        artifact: artifact(public_index, "text/html"),
                    },
                ],
            },
            identity_signer: BrowserIdentitySignerBuildIdentityV2 {
                target_triple: "wasm32-unknown-unknown".into(),
                cargo_profile: "wasm-release".into(),
                cargo_features: vec!["identity-signer-bridge".into()],
                cargo_package: "robin_rs".into(),
                cargo_binary: "leaderboard_identity_bridge".into(),
                recipe: BrowserIdentitySignerBuildRecipeV2::WasmBindgenWebSeparateOriginBridgeV1,
                deployment_policy: BrowserIdentitySignerDeploymentPolicyV2::SeparateAllowlistedOriginCspFrameAncestorsAndBridgeShaV1,
                rust_toolchain_sha256: rust_toolchain.canonical_digest()?,
                rust_toolchain,
                wasm_bindgen_cli: tool(16, "0.2.127"),
                identity_signer_origin_artifacts: vec![
                    BrowserPagesArtifactV2 {
                        path: "identity-signer/bridge/leaderboard_identity_bridge.js".into(),
                        artifact: artifact(signer_js, "text/javascript"),
                    },
                    BrowserPagesArtifactV2 {
                        path: "identity-signer/bridge/leaderboard_identity_bridge_bg.wasm".into(),
                        artifact: artifact(signer_wasm, "application/wasm"),
                    },
                    BrowserPagesArtifactV2 {
                        path: "identity-signer/index.html".into(),
                        artifact: artifact(signer_index, "text/html"),
                    },
                ],
            },
        },
    };
    build.validate()?;
    let build_sha256 = build.canonical_digest()?;
    let build_path = format!("manifests/builds/{build_sha256}.json");
    let engine_files = [
        ("viewer/robin.js", engine_js.as_slice()),
        ("viewer/robin_bg.wasm", engine_wasm.as_slice()),
    ];
    let mut source_files = vec![
        (
            format!("cloudflare-public/{build_path}"),
            canonical_json_bytes(&build)?,
        ),
        ("cloudflare-public/_headers".into(), public_headers.to_vec()),
        ("cloudflare-public/index.html".into(), public_index.to_vec()),
        (
            "cloudflare-identity-signer/identity-signer/bridge/leaderboard_identity_bridge.js"
                .into(),
            signer_js.to_vec(),
        ),
        (
            "cloudflare-identity-signer/identity-signer/bridge/leaderboard_identity_bridge_bg.wasm"
                .into(),
            signer_wasm.to_vec(),
        ),
        (
            "cloudflare-identity-signer/identity-signer/index.html".into(),
            signer_index.to_vec(),
        ),
    ];
    for named in &build.viewer.engine.artifacts {
        let bytes = engine_files
            .iter()
            .find_map(|(path, bytes)| (*path == named.path).then_some(*bytes))
            .context("synthetic engine artifact bytes are absent")?;
        source_files.push((
            format!(
                "cloudflare-public/{}",
                build_artifact_object_path_v1(build_sha256, named)?
            ),
            bytes.to_vec(),
        ));
    }
    let (datadir_authority, _, datadir_receipt) = datadir_binding()?;
    source_files.extend([
        (
            "deployment/exposure-v3.json".into(),
            canonical_json_bytes(&DeploymentExposureV3::official())?,
        ),
        (
            "deployment/datadir-authority.json".into(),
            canonical_json_bytes(&datadir_authority)?,
        ),
        (
            "deployment/datadir-deployment.json".into(),
            canonical_json_bytes(&datadir_receipt)?,
        ),
    ]);
    for (path, bytes) in source_files {
        let path = source.join(path);
        fs::create_dir_all(path.parent().context("synthetic CF file has no parent")?)?;
        fs::write(path, bytes)?;
    }
    let publication = ValidatedPublicationV3::synthetic_for_consumer_test(source)?;
    let manifest = PublicationManifestV3 {
        schema_version: 3,
        projection_authority_matrix_sha256: Digest32::digest_bytes(b"matrix"),
        official_content_digests_sha256: Digest32::digest_bytes(b"content"),
        build_manifest_sha256: build_sha256,
        viewer_build_report: fact(b"viewer report"),
        datadir_release_authority: fact(b"datadir authority"),
        datadir_deployment_receipt: fact(b"datadir receipt"),
        verifier_operator_config: fact(b"operator config"),
        campaign_states: vec![],
        rules_config_sha256: vec![],
        policy_manifest_sha256: vec![],
        ruleset_manifest_sha256: vec![],
        published_rulesets: vec![],
        competition_manifest_sha256: vec![],
        public_static_files: build
            .viewer
            .pages_shell
            .public_origin_artifacts
            .iter()
            .map(|file| PublicStaticFileArtifactV3 {
                published_path: file.path.clone(),
                artifact: file.artifact.clone(),
            })
            .collect(),
        identity_signer_files: build
            .viewer
            .identity_signer
            .identity_signer_origin_artifacts
            .iter()
            .map(|file| PublicStaticFileArtifactV3 {
                published_path: file.path.clone(),
                artifact: file.artifact.clone(),
            })
            .collect(),
    };
    let origins = derive_cloudflare_origin_inventories_v1(&publication, &manifest, &build)?;
    Ok((
        publication,
        CloudflareMaterializationProvenanceV1 {
            source_commit: "a".repeat(40),
            source_tree_sha1: "b".repeat(40),
            cargo_lock_sha256: Digest32::digest_bytes(b"Cargo.lock"),
            publication_manifest_sha256: Digest32::digest_bytes(b"PublicationManifestV3"),
            publication_lock_sha256: Digest32::digest_bytes(b"PublicationLockV3"),
        },
        origins,
    ))
}

#[cfg(target_os = "linux")]
fn make_test_tree_writable(root: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt as _;

    let inventory = publication_tree_inventory_v3(root)?;
    for file in inventory.files {
        fs::set_permissions(root.join(file.path), fs::Permissions::from_mode(0o600))?;
    }
    let mut directories = inventory.directories;
    directories.sort_by_key(|directory| {
        std::cmp::Reverse(Path::new(&directory.path).components().count())
    });
    for directory in directories {
        let path = if directory.path == "." {
            root.to_path_buf()
        } else {
            root.join(directory.path)
        };
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn pinned_json(label: &[u8]) -> PinnedArtifactSourceV3 {
    let mut artifact = fact(label);
    artifact.media_type = "application/json".into();
    PinnedArtifactSourceV3 {
        source: PathBuf::from("metadata.json"),
        artifact,
    }
}

fn datadir_binding() -> Result<(
    DatadirReleaseAuthorityV1,
    Digest32,
    DatadirDeploymentReceiptV1,
)> {
    let demo = DatadirDemoAuthorityV1 {
        content_manifest_url: DEMO_CONTENT_MANIFEST_URL.into(),
        content_manifest_sha256: Digest32::digest_bytes(b"content manifest"),
        datadir_url: DEMO_DATADIR_URL.into(),
        datadir_sha256: Digest32::digest_bytes(b"datadir"),
        datadir_byte_length: 123,
        native_content_sha256: Digest32::digest_bytes(b"native content"),
    };
    let authority = DatadirReleaseAuthorityV1 {
        schema_version: DATADIR_RELEASE_SCHEMA_VERSION,
        source_commit: "a".repeat(40),
        cargo_lock_sha256: Digest32::digest_bytes(b"Cargo.lock"),
        inventory_sha256: Digest32::digest_bytes(b"inventory"),
        worker_name: DATADIR_WORKER_NAME.into(),
        route_pattern: DATADIR_ROUTE_PATTERN.into(),
        public_root_url: DATADIR_PUBLIC_ROOT_URL.into(),
        demo,
    };
    let authority_sha256 = Digest32::digest_bytes(&canonical_json_bytes(&authority)?);
    let receipt = DatadirDeploymentReceiptV1 {
        schema_version: DATADIR_RELEASE_SCHEMA_VERSION,
        authority_sha256,
        inventory_sha256: authority.inventory_sha256,
        source_commit: authority.source_commit.clone(),
        worker_name: authority.worker_name.clone(),
        worker_version_id: "01234567-89ab-cdef-0123-456789abcdef".into(),
        route_pattern: authority.route_pattern.clone(),
        public_root_url: authority.public_root_url.clone(),
        demo: authority.demo.clone(),
    };
    Ok((authority, authority_sha256, receipt))
}

fn campaign_artifact(label: &[u8]) -> ArtifactRefV1 {
    let mut artifact = fact(label);
    artifact.media_type = RANKED_CAMPAIGN_MEDIA_TYPE_V1.into();
    artifact
}

fn campaign_matrix(rules: &[Digest32]) -> Vec<CampaignStateArtifactV3> {
    let shared = campaign_artifact(b"shared canonical campaign");
    rules
        .iter()
        .flat_map(|rules_config_sha256| {
            [
                CampaignStateArtifactV3 {
                    edition: OfficialContentEditionV1::Demo,
                    kind: CampaignStateKindV3::IndividualTemplate,
                    rules_config_sha256: *rules_config_sha256,
                    artifact: shared.clone(),
                },
                CampaignStateArtifactV3 {
                    edition: OfficialContentEditionV1::Full,
                    kind: CampaignStateKindV3::FullCampaignGenesis,
                    rules_config_sha256: *rules_config_sha256,
                    artifact: shared.clone(),
                },
            ]
        })
        .collect()
}

fn ranked_rules_config() -> Result<RulesConfigIdentityV1> {
    use robin_engine::engine::SimConfig;
    use robin_engine::player_profile::DifficultyLevel;
    use robin_run_protocol::{CanonicalValue, RankedSimulationPolicyV1};

    let CanonicalValue::Object(sim_config) = serde_json::from_value(serde_json::to_value(
        SimConfig::standard_ranked(DifficultyLevel::Medium),
    )?)?
    else {
        anyhow::bail!("SimConfig must canonicalize as an object")
    };
    Ok(RulesConfigIdentityV1 {
        schema_version: 1,
        replay_schema_version: CURRENT_RANKED_REPLAY_SCHEMA_VERSION_V1,
        ranked_simulation_policy: RankedSimulationPolicyV1::standard(
            robin_run_protocol::RankedSimulationDifficultyV1::Medium,
        ),
        sim_config,
        rules: BTreeMap::from([("ranked".into(), CanonicalValue::Bool(true))]),
    })
}

fn privacy_ruleset_manifest() -> RulesetManifestV1 {
    use robin_run_protocol::*;

    fn policy(kind: ImmutablePolicyKindV1, byte: u8) -> ImmutablePolicyIdentityV1 {
        ImmutablePolicyIdentityV1 {
            kind,
            version: 1,
            manifest_sha256: Digest32::from_bytes([byte; 32]),
        }
    }

    let rules_config_sha256 = Digest32::from_bytes([2; 32]);
    RulesetManifestV1 {
        schema_version: 1,
        display_name: "Full / Standard / Normal".into(),
        preset_id: OpaqueId::new("standard").unwrap(),
        preset_name: "Standard".into(),
        difficulty_id: OpaqueId::new("normal").unwrap(),
        difficulty_name: "Normal".into(),
        rules_config_sha256,
        rules_config_constraint: RulesConfigConstraintV1::ExactCanonicalDigestOnly,
        allowed_build_manifest_sha256: vec![Digest32::from_bytes([8; 32])],
        allowed_content_manifest_sha256: vec![Digest32::from_bytes([9; 32])],
        allowed_campaign_content_manifest_sha256: vec![Digest32::from_bytes([10; 32])],
        board_scopes: vec![
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
            edition: OfficialContentEditionV1::Full,
            kind: CanonicalCampaignStateKindV1::FullCampaignGenesis,
            rules_config_sha256,
        },
        run_preflight_grant_public_key: PublicKey32::from_bytes([44; 32]),
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
            maximum_max_concurrent_players: MAX_REPLAY_SEATS_V1,
            maximum_participant_instances: MAX_PARTICIPANT_INSTANCES_V1,
        },
        replay_schema_versions: vec![CURRENT_RANKED_REPLAY_SCHEMA_VERSION_V1],
        network_protocol_versions: vec![CURRENT_RANKED_NETWORK_PROTOCOL_VERSION_V1],
        input_provenance_policy: policy(ImmutablePolicyKindV1::InputProvenance, 10),
        command_admission_policy: policy(ImmutablePolicyKindV1::CommandAdmission, 11),
        submission_admission_policy: policy(ImmutablePolicyKindV1::SubmissionAdmission, 12),
        verifier_policy: policy(ImmutablePolicyKindV1::Verification, 13),
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
    }
}

fn privacy_competition_manifest() -> CompetitionManifestV1 {
    use robin_run_protocol::*;

    CompetitionManifestV1 {
        schema_version: 1,
        competition_id: OpaqueId::new("daily-mission-1").unwrap(),
        competition_version: 1,
        display_name: "Daily Mission 1".into(),
        description: "A pinned-seed daily board.".into(),
        subject: LeaderboardSubjectV1::Mission {
            mission_id: "mission_1".into(),
            category: BoardCategoryV1::IndividualLevel,
        },
        metric: BoardMetricV1::OriginalScore,
        rules_config_sha256: Digest32::from_bytes([2; 32]),
        ruleset_manifest_sha256: Digest32::from_bytes([3; 32]),
        canonical_campaign_state: CanonicalCampaignStateRequirementV1 {
            edition: OfficialContentEditionV1::Demo,
            kind: CanonicalCampaignStateKindV1::IndividualTemplate,
            rules_config_sha256: Digest32::from_bytes([2; 32]),
        },
        content: RunContentIdentityV1::Mission {
            content_manifest_sha256: Digest32::from_bytes([1; 32]),
        },
        seed_policy: CompetitionSeedPolicyV1::Pinned {
            simulation_seed: SimulationSeed64::new(42),
        },
        participant_composition: CompetitionParticipantCompositionV1::SinglePlayer,
        competition_run_grant_public_key: PublicKey32::from_bytes([7; 32]),
        starts_at_unix_ms: 1_800_000_000_000,
        ends_at_unix_ms: 1_800_086_400_000,
    }
}

#[test]
fn campaign_state_matrix_is_config_bound_complete_and_allows_shared_bytes() -> Result<()> {
    let mut rules = vec![
        Digest32::digest_bytes(b"standard medium"),
        Digest32::digest_bytes(b"original parity medium"),
    ];
    rules.sort();
    let states = campaign_matrix(&rules);
    ensure!(campaign_state_matrix_is_exact(&states, &rules));
    ensure!(
        states
            .iter()
            .all(|state| state.artifact == states[0].artifact
                && state.canonical_pin().validate().is_ok()),
        "test matrix does not exercise shared physical campaign bytes"
    );

    let mut legacy = serde_json::to_value(&states[0])?;
    legacy
        .as_object_mut()
        .context("campaign pin is not an object")?
        .remove("rules_config_sha256");
    ensure!(
        serde_json::from_value::<CampaignStateArtifactV3>(legacy).is_err(),
        "campaign pin accepted the former unbound schema"
    );

    let mut missing = states.clone();
    missing.pop();
    ensure!(!campaign_state_matrix_is_exact(&missing, &rules));
    let mut substituted = states;
    substituted[0].rules_config_sha256 = Digest32::digest_bytes(b"substituted rules");
    ensure!(!campaign_state_matrix_is_exact(&substituted, &rules));
    Ok(())
}

#[test]
fn campaign_state_source_must_be_canonical_fresh_campaign_bitcode() -> Result<()> {
    use robin_engine::campaign::Campaign;
    use robin_engine::player_profile::DifficultyLevel;
    use robin_engine::profiles::{CharacterProfile, MissionProfile, ProfileManager};

    let rules = ranked_rules_config()?;
    let rules_config_sha256 = rules.canonical_digest()?;
    let mut profiles = ProfileManager::new();
    for name in ["Robin des villes", "Robin des bois", "Petit Jean"] {
        profiles.characters.push(CharacterProfile {
            profile_name: name.into(),
            ..Default::default()
        });
    }
    profiles.missions.push(MissionProfile::default());
    let campaign = Campaign::from_profiles(&profiles, DifficultyLevel::Medium);
    let admitted_profiles = AdmittedProfileManagersV1 {
        demo: profiles.clone(),
        full: profiles.clone(),
    };
    let root = tempfile::tempdir()?;
    let path = root.path().join("campaign.bitcode");
    let bytes = bitcode::encode(&campaign);
    fs::write(&path, &bytes)?;
    let mut source = CampaignStateSourceV3 {
        edition: OfficialContentEditionV1::Demo,
        kind: CampaignStateKindV3::IndividualTemplate,
        rules_config_sha256,
        source: path.clone(),
        artifact: ArtifactRefV1 {
            sha256: Digest32::digest_bytes(&bytes),
            byte_length: bytes.len() as u64,
            media_type: RANKED_CAMPAIGN_MEDIA_TYPE_V1.into(),
        },
    };
    validate_campaign_state_source(&source, &rules, &admitted_profiles)?;

    // A canonical, fresh campaign with the same edition/difficulty/shape
    // is still untrusted when it was authored from a different profile
    // catalog. Exact rederivation must reject it.
    let mut substituted_profiles = profiles;
    substituted_profiles.missions[0].blazon_price += 1;
    let substituted = Campaign::from_profiles(&substituted_profiles, DifficultyLevel::Medium);
    let bytes = bitcode::encode(&substituted);
    fs::write(&path, &bytes)?;
    source.artifact.sha256 = Digest32::digest_bytes(&bytes);
    source.artifact.byte_length = bytes.len() as u64;
    ensure!(
        validate_campaign_state_source(&source, &rules, &admitted_profiles).is_err(),
        "publication accepted a fresh campaign authored from a substituted ProfileManager"
    );

    let mut progressed = campaign;
    progressed.current_mission_idx = Some(0);
    let bytes = bitcode::encode(&progressed);
    fs::write(&path, &bytes)?;
    source.artifact.sha256 = Digest32::digest_bytes(&bytes);
    source.artifact.byte_length = bytes.len() as u64;
    ensure!(
        validate_campaign_state_source(&source, &rules, &admitted_profiles).is_err(),
        "publication accepted a selected-mission campaign as canonical genesis"
    );
    Ok(())
}

#[test]
fn publication_plan_rejects_omitted_datadir_binding() -> Result<()> {
    let pin = serde_json::to_value(pinned_json(b"operator config"))?;
    let plan = serde_json::json!({
        "schema_version": PUBLICATION_PLAN_SCHEMA_VERSION,
        "official_content_authority": "authority",
        "build_draft_v2": "build.json",
        "viewer_build_report": "report.json",
        "verifier_operator_config": pin,
        "campaign_states": [],
        "policies": [],
        "published_rulesets": [],
        "transition": {"kind": "fresh"}
    });
    ensure!(
        serde_json::from_value::<OperatorPublicationPlanV3>(plan).is_err(),
        "publication plan accepted an omitted datadir authority and receipt"
    );
    Ok(())
}

#[test]
fn datadir_receipt_rejects_substitution() -> Result<()> {
    let (authority, authority_sha256, mut receipt) = datadir_binding()?;
    receipt.demo.datadir_sha256 = Digest32::digest_bytes(b"substituted datadir");
    ensure!(
        validate_datadir_binding(&authority, authority_sha256, &receipt).is_err(),
        "deployment receipt accepted a substituted Demo archive"
    );

    let (_, _, receipt) = datadir_binding()?;
    let mut substituted_authority = authority.clone();
    substituted_authority.demo.native_content_sha256 =
        Digest32::digest_bytes(b"substituted native content");
    ensure!(
        validate_datadir_binding(&substituted_authority, authority_sha256, &receipt).is_err(),
        "deployment receipt accepted a substituted release authority"
    );
    Ok(())
}

#[test]
fn prior_immutable_datadir_authority_is_independent_of_later_build() -> Result<()> {
    let (authority, authority_sha256, receipt) = datadir_binding()?;
    let later_build_source_commit = "b".repeat(40);
    let later_build_cargo_lock = Digest32::digest_bytes(b"later Cargo.lock");
    ensure!(
        authority.source_commit != later_build_source_commit
            && authority.cargo_lock_sha256 != later_build_cargo_lock,
        "test does not model a datadir produced by an earlier build"
    );
    validate_datadir_binding(&authority, authority_sha256, &receipt)?;
    Ok(())
}

#[test]
fn datadir_producer_provenance_must_remain_well_formed_and_receipt_bound() -> Result<()> {
    let (authority, authority_sha256, receipt) = datadir_binding()?;
    let mut invalid_source = authority.clone();
    invalid_source.source_commit = "A".repeat(40);
    ensure!(
        validate_datadir_binding(&invalid_source, authority_sha256, &receipt).is_err(),
        "publication accepted malformed datadir producer provenance"
    );

    let mut zero_lock = authority.clone();
    zero_lock.cargo_lock_sha256 = Digest32::from_bytes([0; 32]);
    ensure!(
        validate_datadir_binding(&zero_lock, authority_sha256, &receipt).is_err(),
        "publication accepted zero datadir Cargo.lock provenance"
    );

    let mut substituted_receipt = receipt;
    substituted_receipt.source_commit = "b".repeat(40);
    ensure!(
        validate_datadir_binding(&authority, authority_sha256, &substituted_receipt).is_err(),
        "publication accepted a receipt from another datadir producer"
    );
    Ok(())
}

#[test]
fn datadir_authority_schema_has_no_full_payload_lane() -> Result<()> {
    let (authority, _, _) = datadir_binding()?;
    let mut value = serde_json::to_value(authority)?;
    value
        .as_object_mut()
        .context("authority is not an object")?
        .insert(
            "full".into(),
            serde_json::json!({"datadir_url": "forbidden"}),
        );
    ensure!(
        serde_json::from_value::<DatadirReleaseAuthorityV1>(value).is_err(),
        "datadir authority accepted a Full retail payload field"
    );
    Ok(())
}

#[test]
fn publication_deployment_inventory_rejects_datadir_payload_bytes() -> Result<()> {
    let root = tempfile::tempdir()?;
    fs::create_dir(root.path().join("deployment"))?;
    for name in [
        "datadir-authority.json",
        "datadir-deployment.json",
        "exposure-v3.json",
    ] {
        fs::write(root.path().join("deployment").join(name), b"{}")?;
    }
    let inventory = publication_tree_inventory_v3(root.path())?;
    validate_deployment_metadata_inventory(&inventory)?;
    fs::write(
        root.path().join("deployment/v8-web-opus-q80.rhdata.zst"),
        b"forbidden datadir payload",
    )?;
    ensure!(
        validate_deployment_metadata_inventory(&publication_tree_inventory_v3(root.path())?)
            .is_err(),
        "normal publication accepted datadir payload bytes"
    );
    Ok(())
}

#[test]
fn status_transition_rejects_immutable_change_and_accepts_status_only() {
    let old = BTreeMap::from([
        ("backend/manifests/builds/a.json".into(), fact(b"build")),
        (
            "backend/manifests/published-rulesets/r.json".into(),
            fact(b"active"),
        ),
    ]);
    let mut new = old.clone();
    new.insert(
        "backend/manifests/published-rulesets/r.json".into(),
        fact(b"quarantined"),
    );
    assert_eq!(
        immutable_transition_files(&old),
        immutable_transition_files(&new)
    );
    assert_ne!(status_transition_files(&old), status_transition_files(&new));
    new.insert("backend/manifests/builds/a.json".into(), fact(b"changed"));
    assert_ne!(
        immutable_transition_files(&old),
        immutable_transition_files(&new)
    );
}

#[test]
fn public_static_paths_reject_reserved_and_ambiguous_forms() {
    for invalid in ["/index.html", "a/../b", "a\\b", "a%2fb", "", "a/"] {
        let artifact = robin_run_protocol::BrowserPagesArtifactV2 {
            path: invalid.into(),
            artifact: fact(b"page"),
        };
        assert!(artifact.validate().is_err(), "accepted {invalid:?}");
    }
    let valid = robin_run_protocol::BrowserPagesArtifactV2 {
        path: "assets/app.js".into(),
        artifact: fact(b"page"),
    };
    assert!(valid.validate().is_ok());
    assert!(matches!(
        "private/secret".split('/').next(),
        Some("builds" | "content" | "manifests" | "private")
    ));
}

#[cfg(unix)]
#[test]
fn nested_private_authority_materialization_creates_only_the_exact_parent() -> Result<()> {
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _, symlink};

    let sandbox = tempfile::tempdir()?;
    let staging = sandbox.path().join("staging");
    fs::create_dir(&staging)?;
    let authority = sandbox.path().join("authority");
    fs::create_dir(&authority)?;
    fs::create_dir(authority.join("manifests"))?;
    fs::create_dir(authority.join("empty"))?;
    fs::write(
        authority.join("official-content-digests.json"),
        b"authority root",
    )?;
    fs::write(authority.join("manifests/build.json"), b"nested manifest")?;
    fs::set_permissions(&authority, fs::Permissions::from_mode(0o555))?;
    fs::set_permissions(
        authority.join("manifests"),
        fs::Permissions::from_mode(0o555),
    )?;
    fs::set_permissions(authority.join("empty"), fs::Permissions::from_mode(0o711))?;
    fs::set_permissions(
        authority.join("manifests/build.json"),
        fs::Permissions::from_mode(0o444),
    )?;

    let private_root = create_private_publication_root(&staging)?;
    assert_eq!(private_root, staging.join("private"));
    assert_eq!(
        fs::metadata(&private_root)?.permissions().mode() & 0o777,
        0o755
    );
    let copied = private_root.join("official-content-authority");
    copy_directory_exact_preserving_modes(&authority, &copied)?;
    assert_eq!(
        fs::read(copied.join("official-content-digests.json"))?,
        b"authority root"
    );
    assert_eq!(
        fs::read(copied.join("manifests/build.json"))?,
        b"nested manifest"
    );
    assert_eq!(fs::metadata(&copied)?.permissions().mode() & 0o777, 0o555);
    assert_eq!(
        fs::metadata(copied.join("empty"))?.permissions().mode() & 0o777,
        0o711
    );
    let source_file_metadata = fs::metadata(authority.join("manifests/build.json"))?;
    let copied_file_metadata = fs::metadata(copied.join("manifests/build.json"))?;
    assert_eq!(copied_file_metadata.permissions().mode() & 0o777, 0o444);
    assert_ne!(source_file_metadata.ino(), copied_file_metadata.ino());
    assert_eq!(copied_file_metadata.nlink(), 1);
    assert_eq!(
        publication_directories(&staging)?
            .into_iter()
            .map(|directory| directory.path)
            .collect::<Vec<_>>(),
        vec![
            ".".to_owned(),
            "private".to_owned(),
            "private/official-content-authority".to_owned(),
            "private/official-content-authority/empty".to_owned(),
            "private/official-content-authority/manifests".to_owned(),
        ]
    );

    let attacked_staging = sandbox.path().join("attacked-staging");
    let outside = sandbox.path().join("outside");
    fs::create_dir(&attacked_staging)?;
    fs::create_dir(&outside)?;
    symlink(&outside, attacked_staging.join("private"))?;
    ensure!(
        create_private_publication_root(&attacked_staging).is_err(),
        "private publication root followed a symlink"
    );
    ensure!(
        fs::read_dir(&outside)?.next().is_none(),
        "symlink rejection modified the external target"
    );

    // Restore write permission so TempDir can clean up the deliberately
    // read-only source and copied authority trees.
    for directory in [
        &authority,
        &authority.join("empty"),
        &authority.join("manifests"),
        &copied,
        &copied.join("empty"),
        &copied.join("manifests"),
    ] {
        fs::set_permissions(directory, fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

#[cfg(target_os = "linux")]
#[test]
fn mode_preserving_copy_rejects_late_source_and_destination_substitution() -> Result<()> {
    let sandbox = tempfile::tempdir()?;

    let source = sandbox.path().join("source");
    fs::create_dir(&source)?;
    fs::write(source.join("payload"), b"reviewed")?;
    let destination = sandbox.path().join("destination");
    assert!(
        copy_directory_exact_preserving_modes_with(&source, &destination, || {
            fs::rename(
                destination.join("payload"),
                destination.join("copied-payload"),
            )
            .unwrap();
            fs::write(destination.join("payload"), b"reviewed").unwrap();
        })
        .is_err()
    );

    let source_two = sandbox.path().join("source-two");
    fs::create_dir(&source_two)?;
    fs::write(source_two.join("payload"), b"reviewed")?;
    let destination_two = sandbox.path().join("destination-two");
    assert!(
        copy_directory_exact_preserving_modes_with(&source_two, &destination_two, || {
            fs::write(source_two.join("payload"), b"substituted").unwrap()
        },)
        .is_err()
    );
    Ok(())
}

#[test]
fn publication_failure_never_creates_requested_output() {
    let root = tempfile::tempdir().unwrap();
    let plan = root.path().join("invalid.json");
    fs::write(&plan, b"{}").unwrap();
    let output = root.path().join("publication");
    assert!(assemble_publication_v3(&plan, &output).is_err());
    assert!(!output.exists());
}

#[test]
fn publication_lock_binds_the_wasm_bindgen_authority_document() -> Result<()> {
    let root = tempfile::tempdir()?;
    let authority: robin_run_protocol::BuildToolAuthorityDocumentV1 = serde_json::from_slice(
        include_bytes!("../../../../.github/tool-authorities/wasm-bindgen-cli-v0.2.127.json"),
    )?;
    let digest = authority.canonical_digest()?;
    let relative = format!(
        "private/official-content-authority/manifests/build-tool-authorities/{digest}.json"
    );
    let path = root.path().join(&relative);
    fs::create_dir_all(path.parent().unwrap())?;
    fs::write(&path, canonical_json_bytes(&authority)?)?;

    let lock =
        publication_lock_from_actual_for_test(root.path(), Digest32::digest_bytes(b"publication"))?;
    let entry = lock
        .files
        .iter()
        .find(|entry| entry.path == relative)
        .context("wasm-bindgen authority is absent from publication lock")?;
    assert_eq!(
        entry.artifact.sha256,
        artifact_from_file(&path, "application/octet-stream")?.sha256
    );
    assert_eq!(entry.exposure, ReleaseFileExposureV1::OperatorPrivate);
    Ok(())
}

#[test]
fn privacy_scanner_finds_cross_chunk_sentinel_and_private_json_key() -> Result<()> {
    let root = tempfile::tempdir()?;
    let binary = root.path().join("viewer.wasm");
    let needle = b"private-authority-digest";
    let mut bytes = vec![b'x'; 64 * 1024 - 7];
    bytes.extend_from_slice(needle);
    bytes.extend_from_slice(b"tail");
    fs::write(&binary, bytes)?;
    ensure!(
        file_contains_bytes(&binary, needle)?,
        "privacy scanner missed a chunk-boundary sentinel"
    );

    let public = serde_json::json!({
        "safe": [{"projection_authority_manifest_sha256": "secret"}]
    });
    ensure!(
        reject_private_json_keys(
            &public,
            "test public JSON",
            PublicJsonSchema::Other,
            &mut Vec::new(),
        )
        .is_err(),
        "privacy scanner accepted a nested private authority field"
    );
    reject_private_json_keys(
        &serde_json::json!({"build_manifest_sha256": "public"}),
        "test public JSON",
        PublicJsonSchema::Other,
        &mut Vec::new(),
    )?;
    Ok(())
}

#[test]
fn privacy_scanner_allows_only_typed_addressed_campaign_requirements() -> Result<()> {
    let root = tempfile::tempdir()?;
    let ruleset = privacy_ruleset_manifest();
    ruleset.validate()?;
    let ruleset_digest = ruleset.canonical_digest()?;
    let ruleset_value = serde_json::to_value(&ruleset)?;
    let ruleset_directory = root.path().join("ruleset-manifests");
    fs::create_dir(&ruleset_directory)?;
    fs::write(
        ruleset_directory.join(format!("{ruleset_digest}.json")),
        ruleset.canonical_bytes()?,
    )?;

    let published = PublishedRulesetV1 {
        schema_version: 1,
        ruleset_manifest_sha256: ruleset_digest,
        manifest: ruleset.clone(),
        operational_status: robin_run_protocol::RulesetOperationalStatusV1::Active,
    };
    published.validate()?;
    let published_directory = root.path().join("published-rulesets");
    fs::create_dir(&published_directory)?;
    fs::write(
        published_directory.join(format!("{ruleset_digest}.json")),
        published.canonical_bytes()?,
    )?;

    let competition = privacy_competition_manifest();
    competition.validate()?;
    let competition_digest = competition.canonical_digest()?;
    let competition_directory = root.path().join("competitions");
    fs::create_dir(&competition_directory)?;
    fs::write(
        competition_directory.join(format!("{competition_digest}.json")),
        competition.canonical_bytes()?,
    )?;
    scan_public_tree(root.path(), &[], "synthetic backend manifests")?;

    ensure!(
        public_json_schema(
            &format!("ruleset-manifests/{}.json", "A".repeat(64)),
            &ruleset_value,
            "uppercase addressed ruleset",
        )
        .is_err(),
        "privacy exception accepted an uppercase or mismatched address"
    );
    let requirement = serde_json::to_value(ruleset.canonical_campaign_state)?;
    let nested = serde_json::json!({"nested": {"canonical_campaign_state": requirement}});
    ensure!(
        reject_private_json_keys(
            &nested,
            "nested ruleset field",
            PublicJsonSchema::RulesetManifest,
            &mut Vec::new(),
        )
        .is_err(),
        "privacy exception widened to a nested field"
    );
    ensure!(
        reject_private_json_keys(
            &serde_json::json!({"canonical_campaign_state": ruleset.canonical_campaign_state}),
            "wrong document",
            PublicJsonSchema::Other,
            &mut Vec::new(),
        )
        .is_err(),
        "privacy exception widened to another document schema"
    );
    let mut requirement_with_private_payload =
        serde_json::to_value(ruleset.canonical_campaign_state)?;
    requirement_with_private_payload
        .as_object_mut()
        .context("requirement is not an object")?
        .insert("artifact".into(), serde_json::json!({"bytes": "private"}));
    ensure!(
        reject_private_json_keys(
            &serde_json::json!({
                "canonical_campaign_state": requirement_with_private_payload,
            }),
            "ruleset with private campaign payload",
            PublicJsonSchema::RulesetManifest,
            &mut Vec::new(),
        )
        .is_err(),
        "typed public requirement accepted an unknown private payload"
    );
    for forbidden_key in ["canonical_campaign_state_path", "raw_campaign_state"] {
        ensure!(
            reject_private_json_keys(
                &serde_json::json!({forbidden_key: "private"}),
                "suffixed campaign state field",
                PublicJsonSchema::Other,
                &mut Vec::new(),
            )
            .is_err(),
            "privacy scanner accepted {forbidden_key}"
        );
    }
    Ok(())
}

#[test]
fn public_tree_scanner_rejects_private_paths_and_concrete_values() -> Result<()> {
    let root = tempfile::tempdir()?;
    fs::create_dir_all(root.path().join("assets"))?;
    fs::write(root.path().join("assets/app.js"), b"const x='secret-pin'")?;
    ensure!(
        scan_public_tree(root.path(), &[b"secret-pin".to_vec()], "test origin").is_err(),
        "public scanner accepted a concrete private value"
    );
    let campaign_bytes = b"\0canonical-private-campaign\xff".to_vec();
    fs::write(root.path().join("assets/app.js"), &campaign_bytes)?;
    ensure!(
        scan_public_tree(root.path(), &[campaign_bytes], "test origin").is_err(),
        "public scanner accepted concrete private campaign bytes"
    );
    let private_path = b"/release/private/campaign-states/secret".to_vec();
    fs::write(root.path().join("assets/app.js"), &private_path)?;
    ensure!(
        scan_public_tree(root.path(), &[private_path], "test origin").is_err(),
        "public scanner accepted an absolute private campaign path"
    );
    let operator_config = b"operator-config-secret-value".to_vec();
    fs::write(root.path().join("assets/app.js"), &operator_config)?;
    ensure!(
        scan_public_tree(root.path(), &[operator_config], "test origin").is_err(),
        "public scanner accepted concrete operator-config bytes"
    );
    fs::remove_file(root.path().join("assets/app.js"))?;
    fs::write(root.path().join("projection-receipt.json"), b"{}")?;
    ensure!(
        scan_public_tree(root.path(), &[], "test origin").is_err(),
        "public scanner accepted a private namespace path"
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn failed_read_only_staging_cleanup_is_bounded_and_never_follows_symlinks() -> Result<()> {
    use std::os::unix::fs::{PermissionsExt as _, symlink};

    let sandbox = tempfile::tempdir()?;
    let output = sandbox.path().join("publication");
    let staging = create_pinned_publication_staging_v3(&output)?;
    let staging_path = staging.path().to_path_buf();
    let sealed = staging
        .path()
        .join("private/official-content-authority/manifests");
    fs::create_dir_all(&sealed)?;
    fs::write(sealed.join("authority.json"), b"sealed authority")?;
    fs::set_permissions(
        sealed.join("authority.json"),
        fs::Permissions::from_mode(0o440),
    )?;
    fs::set_permissions(&sealed, fs::Permissions::from_mode(0o550))?;
    fs::set_permissions(
        sealed.parent().context("sealed authority has no parent")?,
        fs::Permissions::from_mode(0o550),
    )?;

    let outside = sandbox.path().join("outside");
    fs::create_dir(&outside)?;
    fs::write(outside.join("sentinel"), b"outside remains unchanged")?;
    symlink(&outside, staging.path().join("outside-link"))?;

    discard_failed_publication_staging(staging)?;
    ensure!(!staging_path.exists(), "failed staging path remains");
    ensure!(
        fs::read(outside.join("sentinel"))? == b"outside remains unchanged",
        "cleanup followed a symlink outside staging"
    );
    ensure!(
        fs::metadata(&outside)?.permissions().mode() & 0o777 != 0o700,
        "cleanup changed external directory permissions"
    );
    ensure!(
        fs::read_dir(sandbox.path())?
            .collect::<std::io::Result<Vec<_>>>()?
            .iter()
            .all(|entry| !entry
                .file_name()
                .to_string_lossy()
                .starts_with(".robin-manifestctl-")),
        "failed cleanup left a same-filesystem staging directory"
    );
    Ok(())
}

#[cfg(target_os = "linux")]
#[test]
fn failed_staging_cleanup_preserves_authentic_tree_on_root_substitution() -> Result<()> {
    let sandbox = tempfile::tempdir()?;
    let output = sandbox.path().join("publication");
    let staging = create_pinned_publication_staging_v3(&output)?;
    fs::write(staging.path().join("authentic"), b"preserve me")?;
    let authentic = sandbox.path().join("authentic-stage");
    let substitute_path = staging.path().to_path_buf();
    fs::rename(staging.path(), &authentic)?;
    fs::create_dir(staging.path())?;
    fs::write(staging.path().join("substitute"), b"do not delete")?;

    ensure!(
        discard_failed_publication_staging(staging).is_err(),
        "cleanup accepted a substituted staging basename"
    );
    ensure!(
        fs::read(authentic.join("authentic"))? == b"preserve me"
            && fs::read(substitute_path.join("substitute"))? == b"do not delete",
        "uncertain cleanup removed the authentic or substitute tree"
    );
    fs::remove_dir_all(&authentic)?;
    fs::remove_dir_all(substitute_path)?;
    Ok(())
}

#[cfg(target_os = "linux")]
#[test]
fn failed_staging_cleanup_rejects_mid_operation_swap_and_hardlinks() -> Result<()> {
    let sandbox = tempfile::tempdir()?;
    let output = sandbox.path().join("publication");
    let staging = create_pinned_publication_staging_v3(&output)?;
    fs::write(staging.path().join("authentic"), b"preserve me")?;
    let authentic = sandbox.path().join("authentic-stage");
    let substitute_path = staging.path().to_path_buf();
    let cleanup = discard_failed_publication_staging_with(&staging, |operation| {
        if operation == 0 {
            fs::rename(&substitute_path, &authentic).unwrap();
            fs::create_dir(&substitute_path).unwrap();
            fs::write(substitute_path.join("substitute"), b"do not delete").unwrap();
        }
    });
    ensure!(
        cleanup.is_err()
            && authentic.join("authentic").is_file()
            && substitute_path.join("substitute").is_file(),
        "cleanup deleted a tree after a mid-operation root substitution"
    );
    fs::remove_dir_all(&authentic)?;
    fs::remove_dir_all(&substitute_path)?;
    drop(staging);

    let hardlink_output = sandbox.path().join("hardlink-publication");
    let hardlinked = create_pinned_publication_staging_v3(&hardlink_output)?;
    fs::write(hardlinked.path().join("one"), b"shared")?;
    fs::hard_link(hardlinked.path().join("one"), hardlinked.path().join("two"))?;
    let hardlinked_path = hardlinked.path().to_path_buf();
    ensure!(
        discard_failed_publication_staging(hardlinked).is_err()
            && hardlinked_path.join("one").is_file()
            && hardlinked_path.join("two").is_file(),
        "guarded cleanup deleted a hard-linked or uncertain staging tree"
    );
    fs::remove_dir_all(hardlinked_path)?;
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "android"))]
#[test]
fn publication_persistence_noreplace_race_cleans_staging_without_overwrite() -> Result<()> {
    use std::os::unix::fs::PermissionsExt as _;

    let sandbox = tempfile::tempdir()?;
    let output = sandbox.path().join("publication");
    let staging = create_pinned_publication_staging_v3(&output)?;
    let staging_path = staging.path().to_path_buf();
    fs::create_dir(staging.path().join("sealed"))?;
    fs::write(staging.path().join("sealed/data"), b"candidate")?;
    fs::set_permissions(
        staging.path().join("sealed"),
        fs::Permissions::from_mode(0o550),
    )?;
    let candidate = synthetic_validated_staging_v3(&staging)?;
    fs::create_dir(&output)?;
    fs::write(output.join("winner"), b"racing publisher")?;

    let persist_error = persist_publication_staging(&staging, &candidate, &output)
        .expect_err("NOREPLACE persistence overwrote a racing publisher");
    ensure!(
        persist_error
            .downcast_ref::<PublicationInstalledButParentSyncFailed>()
            .is_none(),
        "pre-rename failure was misclassified as an installed publication"
    );
    discard_failed_publication_staging(staging)?;
    ensure!(!staging_path.exists(), "raced staging path remains");
    ensure!(
        fs::read(output.join("winner"))? == b"racing publisher",
        "NOREPLACE persistence modified the racing output"
    );
    ensure!(
        !output.join("sealed/data").exists(),
        "NOREPLACE persistence partially merged the candidate"
    );
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "android"))]
#[test]
fn post_rename_sync_failure_reports_published_outcome_without_staging() -> Result<()> {
    let sandbox = tempfile::tempdir()?;
    let output = sandbox.path().join("publication");
    let staging = create_pinned_publication_staging_v3(&output)?;
    let staging_path = staging.path().to_path_buf();
    fs::write(staging.path().join("complete"), b"complete")?;
    let candidate = synthetic_validated_staging_v3(&staging)?;
    let outcome = persist_publication_staging_with(
        &staging,
        &candidate,
        &output,
        || {},
        |parent, source, destination| {
            use std::os::fd::AsFd as _;
            rustix::fs::renameat_with(
                parent.as_fd(),
                source,
                parent.as_fd(),
                destination,
                rustix::fs::RenameFlags::NOREPLACE,
            )?;
            Ok(())
        },
        |_| anyhow::bail!("injected parent sync failure"),
    )?;
    let PublicationPersistenceOutcome::PublishedButParentSyncFailed(sync_error) = outcome else {
        anyhow::bail!("post-rename failure was not reported as an installed publication")
    };
    ensure!(!staging_path.exists(), "renamed staging path remains");
    ensure!(
        fs::read(output.join("complete"))? == b"complete",
        "published output is incomplete after post-rename sync failure"
    );
    let expected_lock = Digest32::digest_bytes(b"publication lock");
    let classified = installed_publication_durability_error(&output, expected_lock, sync_error);
    let installed = classified
        .downcast_ref::<PublicationInstalledButParentSyncFailed>()
        .context("installed-but-unsynced error is not downcastable")?;
    ensure!(
        installed.output == output && installed.publication_lock_sha256 == expected_lock,
        "installed-but-unsynced error lost its exact output identity"
    );
    ensure!(
        format!("{:#}", installed.source).contains("injected parent sync failure"),
        "installed-but-unsynced error lost its source"
    );
    Ok(())
}

#[cfg(target_os = "linux")]
#[test]
fn publication_persistence_rejects_identical_stage_and_parent_substitution() -> Result<()> {
    let sandbox = tempfile::tempdir()?;
    let output = sandbox.path().join("publication");
    let staging = create_pinned_publication_staging_v3(&output)?;
    fs::write(staging.path().join("payload"), b"reviewed")?;
    let candidate = synthetic_validated_staging_v3(&staging)?;
    let authentic = sandbox.path().join("authentic-stage");
    let stage_path = staging.path().to_path_buf();
    let result = persist_publication_staging_with(
        &staging,
        &candidate,
        &output,
        || {
            fs::rename(&stage_path, &authentic).unwrap();
            fs::create_dir(&stage_path).unwrap();
            fs::write(stage_path.join("payload"), b"reviewed").unwrap();
        },
        |_, _, _| anyhow::bail!("rename must not run after stage substitution"),
        |_| anyhow::bail!("sync must not run after stage substitution"),
    );
    ensure!(
        result.is_err() && !output.exists(),
        "persistence accepted an identical-byte stage inode substitution"
    );
    fs::remove_dir_all(authentic)?;
    fs::remove_dir_all(stage_path)?;
    drop(candidate);
    drop(staging);

    let outer = tempfile::tempdir()?;
    let parent = outer.path().join("release-parent");
    let moved_parent = outer.path().join("authentic-parent");
    fs::create_dir(&parent)?;
    let parent_output = parent.join("publication");
    let parent_staging = create_pinned_publication_staging_v3(&parent_output)?;
    fs::write(parent_staging.path().join("payload"), b"reviewed")?;
    let parent_candidate = synthetic_validated_staging_v3(&parent_staging)?;
    let result = persist_publication_staging_with(
        &parent_staging,
        &parent_candidate,
        &parent_output,
        || {
            fs::rename(&parent, &moved_parent).unwrap();
            fs::create_dir(&parent).unwrap();
        },
        |_, _, _| anyhow::bail!("rename must not run after parent substitution"),
        |_| anyhow::bail!("sync must not run after parent substitution"),
    );
    ensure!(
        result.is_err()
            && moved_parent
                .join(
                    parent_staging
                        .path()
                        .file_name()
                        .context("stage basename is absent")?
                )
                .join("payload")
                .is_file()
            && !parent_output.exists(),
        "persistence accepted or removed a substituted output parent"
    );
    fs::remove_dir_all(&parent)?;
    fs::remove_dir_all(&moved_parent)?;
    Ok(())
}

#[cfg(target_os = "linux")]
#[test]
fn publication_persistence_reconciles_rename_side_effect_then_error() -> Result<()> {
    let sandbox = tempfile::tempdir()?;
    let output = sandbox.path().join("publication");
    let staging = create_pinned_publication_staging_v3(&output)?;
    let staging_path = staging.path().to_path_buf();
    fs::write(staging.path().join("payload"), b"reviewed")?;
    let candidate = synthetic_validated_staging_v3(&staging)?;
    let outcome = persist_publication_staging_with(
        &staging,
        &candidate,
        &output,
        || {},
        |parent, source, destination| {
            use std::os::fd::AsFd as _;
            rustix::fs::renameat_with(
                parent.as_fd(),
                source,
                parent.as_fd(),
                destination,
                rustix::fs::RenameFlags::NOREPLACE,
            )?;
            anyhow::bail!("injected error after successful rename")
        },
        |parent| {
            parent.sync_all()?;
            Ok(())
        },
    )?;
    let PublicationPersistenceOutcome::PublishedButParentSyncFailed(uncertainty) = outcome else {
        anyhow::bail!("rename side-effect error was not classified as installed uncertainty")
    };
    let uncertainty_text = format!("{uncertainty:#}");
    ensure!(
        uncertainty_text.contains("injected error after successful rename")
            && !staging_path.exists()
            && fs::read(output.join("payload"))? == b"reviewed",
        "persistence did not reconcile the exact installed candidate after rename error: {uncertainty_text}"
    );
    Ok(())
}

#[cfg(target_os = "linux")]
#[test]
fn publication_persistence_classifies_parent_swap_after_install() -> Result<()> {
    let outer = tempfile::tempdir()?;
    let parent = outer.path().join("release-parent");
    let moved_parent = outer.path().join("authentic-parent");
    fs::create_dir(&parent)?;
    let output = parent.join("publication");
    let staging = create_pinned_publication_staging_v3(&output)?;
    fs::write(staging.path().join("payload"), b"reviewed")?;
    let candidate = synthetic_validated_staging_v3(&staging)?;
    let uncertainty = persist_publication_staging_with(
        &staging,
        &candidate,
        &output,
        || {},
        |parent_fd, source, destination| {
            use std::os::fd::AsFd as _;
            rustix::fs::renameat_with(
                parent_fd.as_fd(),
                source,
                parent_fd.as_fd(),
                destination,
                rustix::fs::RenameFlags::NOREPLACE,
            )?;
            fs::rename(&parent, &moved_parent)?;
            fs::create_dir(&parent)?;
            Ok(())
        },
        |parent_fd| {
            parent_fd.sync_all()?;
            Ok(())
        },
    )
    .expect_err("post-install parent substitution was reported as a canonical installation");
    ensure!(
        uncertainty
            .downcast_ref::<PublicationPersistenceStateUncertain>()
            .is_some()
            && !output.exists()
            && fs::read(moved_parent.join("publication/payload"))? == b"reviewed",
        "post-install parent substitution was not classified as persistence uncertainty"
    );
    fs::remove_dir_all(parent)?;
    fs::remove_dir_all(moved_parent)?;
    Ok(())
}

#[cfg(target_os = "linux")]
#[test]
fn publication_persistence_rejects_output_loss_or_substitution_after_rename() -> Result<()> {
    for substitute in [false, true] {
        let sandbox = tempfile::tempdir()?;
        let output = sandbox.path().join("publication");
        let orphan = sandbox.path().join("orphaned-candidate");
        let staging = create_pinned_publication_staging_v3(&output)?;
        fs::write(staging.path().join("payload"), b"reviewed")?;
        let candidate = synthetic_validated_staging_v3(&staging)?;
        let uncertainty = persist_publication_staging_with(
            &staging,
            &candidate,
            &output,
            || {},
            |parent_fd, source, destination| {
                use std::os::fd::AsFd as _;
                rustix::fs::renameat_with(
                    parent_fd.as_fd(),
                    source,
                    parent_fd.as_fd(),
                    destination,
                    rustix::fs::RenameFlags::NOREPLACE,
                )?;
                Ok(())
            },
            |parent_fd| {
                fs::rename(&output, &orphan)?;
                if substitute {
                    fs::create_dir(&output)?;
                    fs::write(output.join("payload"), b"reviewed")?;
                }
                parent_fd.sync_all()?;
                Ok(())
            },
        )
        .expect_err("lost canonical output was reported as installed");
        let state = uncertainty
            .downcast_ref::<PublicationPersistenceStateUncertain>()
            .context("canonical output loss was not typed persistence uncertainty")?;
        ensure!(
            state.intended_output == output
                && state.parent_path == sandbox.path()
                && fs::read(orphan.join("payload"))? == b"reviewed"
                && (substitute == output.join("payload").is_file()),
            "persistence uncertainty lost orphan/retry evidence"
        );
        fs::remove_dir_all(orphan)?;
        if substitute {
            fs::remove_dir_all(output)?;
        }
    }
    Ok(())
}

#[test]
fn deployment_exposure_serves_only_reviewed_roots() -> Result<()> {
    let root = tempfile::tempdir()?;
    for directory in [
        "cloudflare-public",
        "cloudflare-identity-signer",
        "backend/manifests",
    ] {
        fs::create_dir_all(root.path().join(directory))?;
    }
    write_canonical(
        &root.path().join("deployment/exposure-v3.json"),
        &DeploymentExposureV3::official(),
    )?;
    validate_deployment_exposure(&mut publication_tree_inventory_v3(root.path())?)?;

    let mut hostile = DeploymentExposureV3::official();
    hostile.cloudflare_routes[0].script = Some("robinhood-public-site".into());
    fs::write(
        root.path().join("deployment/exposure-v3.json"),
        canonical_json_bytes(&hostile)?,
    )?;
    ensure!(
        validate_deployment_exposure(&mut publication_tree_inventory_v3(root.path())?).is_err(),
        "deployment accepted the public Worker on the VPS API route"
    );
    Ok(())
}

#[test]
fn release_exposure_classifies_only_manifest_registry_as_backend_visible() {
    assert_eq!(
        release_file_exposure("backend/manifests/builds/a.json"),
        ReleaseFileExposureV1::BackendManifest
    );
    for path in [
        "backend/publication-v3.json",
        "private/verifier/operator-config/secret",
        "deployment/exposure-v3.json",
        "publication-lock-v3.json",
    ] {
        assert_eq!(
            release_file_exposure(path),
            ReleaseFileExposureV1::OperatorPrivate
        );
    }
    assert_eq!(
        release_file_exposure("cloudflare-identity-signer/index.html"),
        ReleaseFileExposureV1::PublicStatic
    );
    assert_eq!(
        release_file_exposure("cloudflare-public/leaderboards/index.html"),
        ReleaseFileExposureV1::PublicStatic
    );
}

#[test]
fn transition_rejects_mode_and_empty_directory_substitution() -> Result<()> {
    let reviewed = tempfile::tempdir()?;
    let candidate = tempfile::tempdir()?;
    for root in [reviewed.path(), candidate.path()] {
        fs::create_dir_all(root.join("backend/manifests/builds"))?;
        fs::write(root.join("backend/manifests/builds/a.json"), b"same")?;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(
            candidate.path().join("backend/manifests/builds/a.json"),
            fs::Permissions::from_mode(0o600),
        )?;
        ensure!(
            compare_transition(reviewed.path(), candidate.path(), TransitionRule::Exact).is_err(),
            "rollback accepted a mode substitution"
        );
        fs::set_permissions(
            candidate.path().join("backend/manifests/builds/a.json"),
            fs::Permissions::from_mode(publication_unix_mode(
                &reviewed.path().join("backend/manifests/builds/a.json"),
            )?),
        )?;
    }

    fs::create_dir(candidate.path().join("unexpected-empty"))?;
    ensure!(
        compare_transition(reviewed.path(), candidate.path(), TransitionRule::Exact).is_err(),
        "rollback accepted an extra empty directory"
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn publication_inventory_rejects_symlink_and_special_node() -> Result<()> {
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir()?;
    fs::create_dir(root.path().join("real"))?;
    symlink("real", root.path().join("alias"))?;
    assert!(publication_directories(root.path()).is_err());

    let special_root = tempfile::tempdir()?;
    let _socket = std::os::unix::net::UnixListener::bind(special_root.path().join("socket"))?;
    assert!(publication_tree_inventory_v3(special_root.path()).is_err());
    Ok(())
}

#[cfg(target_os = "linux")]
#[test]
fn publication_lock_binds_complete_directory_set_and_modes() -> Result<()> {
    use std::os::unix::fs::PermissionsExt as _;

    let root = tempfile::tempdir()?;
    fs::create_dir(root.path().join("empty"))?;
    fs::create_dir_all(root.path().join("nested/leaf"))?;
    fs::write(root.path().join("nested/payload"), b"locked")?;
    fs::set_permissions(
        root.path().join("nested/payload"),
        fs::Permissions::from_mode(0o644),
    )?;
    fs::set_permissions(root.path().join("empty"), fs::Permissions::from_mode(0o711))?;
    fs::set_permissions(
        root.path().join("nested/leaf"),
        fs::Permissions::from_mode(0o750),
    )?;

    let lock =
        publication_lock_from_actual_for_test(root.path(), Digest32::digest_bytes(b"manifest"))?;
    assert_eq!(
        lock.directories
            .iter()
            .map(|directory| directory.path.as_str())
            .collect::<Vec<_>>(),
        vec![".", "empty", "nested", "nested/leaf"]
    );
    assert_eq!(
        lock.directories
            .iter()
            .find(|directory| directory.path == "empty")
            .context("empty directory is absent")?
            .unix_mode,
        0o711
    );
    assert_eq!(
        lock.directories
            .iter()
            .find(|directory| directory.path == "nested/leaf")
            .context("leaf directory is absent")?
            .unix_mode,
        0o750
    );
    validate_publication_inventory_against_lock_v3(
        &publication_tree_inventory_v3(root.path())?,
        &lock,
    )?;

    fs::create_dir(root.path().join("extra-empty"))?;
    assert!(
        validate_publication_inventory_against_lock_v3(
            &publication_tree_inventory_v3(root.path())?,
            &lock,
        )
        .is_err()
    );
    fs::remove_dir(root.path().join("extra-empty"))?;

    fs::remove_dir(root.path().join("empty"))?;
    assert!(
        validate_publication_inventory_against_lock_v3(
            &publication_tree_inventory_v3(root.path())?,
            &lock,
        )
        .is_err()
    );
    fs::create_dir(root.path().join("empty"))?;
    fs::set_permissions(root.path().join("empty"), fs::Permissions::from_mode(0o711))?;

    fs::set_permissions(
        root.path().join("nested/payload"),
        fs::Permissions::from_mode(0o600),
    )?;
    assert!(
        validate_publication_inventory_against_lock_v3(
            &publication_tree_inventory_v3(root.path())?,
            &lock,
        )
        .is_err()
    );
    fs::set_permissions(
        root.path().join("nested/payload"),
        fs::Permissions::from_mode(0o644),
    )?;

    fs::set_permissions(
        root.path().join("nested/leaf"),
        fs::Permissions::from_mode(0o700),
    )?;
    assert!(
        validate_publication_inventory_against_lock_v3(
            &publication_tree_inventory_v3(root.path())?,
            &lock,
        )
        .is_err()
    );
    Ok(())
}

#[cfg(target_os = "linux")]
#[test]
fn expected_publication_topology_rejects_prelock_extras_everywhere() -> Result<()> {
    fn fixture() -> Result<(tempfile::TempDir, ExpectedPublicationTopologyV3)> {
        let root = tempfile::tempdir()?;
        let mut expected = ExpectedPublicationTopologyV3::new();
        for (path, bytes) in [
            ("backend/manifests/builds/a.json", b"build".as_slice()),
            (
                "private/verifier/operator-config/config",
                b"operator config".as_slice(),
            ),
            ("cloudflare-public/index.html", b"public".as_slice()),
            (
                "cloudflare-identity-signer/index.html",
                b"signer".as_slice(),
            ),
        ] {
            let path = path.to_owned();
            expected.register_bytes(path.clone(), bytes)?;
            let absolute = root.path().join(path);
            fs::create_dir_all(absolute.parent().context("fixture file has no parent")?)?;
            fs::write(absolute, bytes)?;
        }
        expected.register_directory("backend/manifests/competitions")?;
        fs::create_dir_all(root.path().join("backend/manifests/competitions"))?;
        expected.validate_inventory_content(&publication_tree_inventory_v3(root.path())?)?;
        Ok((root, expected))
    }

    for attack in [
        "root-extra",
        "backend/manifests/untyped/extra.json",
        "private/verifier/operator-config/extra",
        "cloudflare-public/extra.js",
        "private/untyped/extra",
    ] {
        let (root, expected) = fixture()?;
        let attack = root.path().join(attack);
        fs::create_dir_all(attack.parent().context("attack file has no parent")?)?;
        fs::write(attack, b"attacker injected")?;
        ensure!(
            expected
                .validate_inventory_content(&publication_tree_inventory_v3(root.path())?)
                .is_err(),
            "typed PublicationV3 topology accepted extra file"
        );
    }
    for attack in [
        "unexpected-empty",
        "backend/manifests/untyped-empty",
        "private/verifier/operator-config/unexpected-empty",
        "cloudflare-public/unexpected-empty",
        "private/unexpected-empty",
    ] {
        let (root, expected) = fixture()?;
        fs::create_dir_all(root.path().join(attack))?;
        ensure!(
            expected
                .validate_inventory_content(&publication_tree_inventory_v3(root.path())?)
                .is_err(),
            "typed PublicationV3 topology accepted extra empty directory"
        );
    }

    let (root, mut expected) = fixture()?;
    ensure!(
        expected
            .register_bytes("cloudflare-public/index.html".into(), b"public")
            .is_err(),
        "typed PublicationV3 topology accepted a duplicate file registration"
    );
    expected.seal_and_validate(root.path(), &open_publication_root_v3(root.path())?)?;
    use std::os::unix::fs::PermissionsExt as _;
    fs::set_permissions(
        root.path().join("cloudflare-public/index.html"),
        fs::Permissions::from_mode(0o644),
    )?;
    ensure!(
        expected
            .validate_inventory(&publication_tree_inventory_v3(root.path())?)
            .is_err(),
        "typed PublicationV3 topology accepted a wrong file mode"
    );
    for file in publication_tree_inventory_v3(root.path())?.files {
        fs::set_permissions(
            root.path().join(file.path),
            fs::Permissions::from_mode(0o600),
        )?;
    }
    for directory in publication_tree_inventory_v3(root.path())?.directories {
        let path = if directory.path == "." {
            root.path().to_path_buf()
        } else {
            root.path().join(directory.path)
        };
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

#[cfg(target_os = "linux")]
#[test]
fn publication_inventory_rejects_hardlinks_and_nested_mount_inventory() -> Result<()> {
    let root = tempfile::tempdir()?;
    fs::write(root.path().join("one"), b"shared inode")?;
    fs::hard_link(root.path().join("one"), root.path().join("two"))?;
    assert!(publication_tree_inventory_v3(root.path()).is_err());

    let canonical = Path::new("/srv/publication-v3");
    let mountinfo = b"41 24 0:38 / /srv/publication-v3/nested rw - tmpfs tmpfs rw\n";
    assert!(reject_publication_mounts_in_v3(canonical, mountinfo).is_err());
    Ok(())
}

#[cfg(target_os = "linux")]
#[test]
fn publication_inventory_rejects_late_file_and_directory_substitution() -> Result<()> {
    use std::os::unix::fs::{PermissionsExt as _, symlink};

    let root_sandbox = tempfile::tempdir()?;
    let root_path = root_sandbox.path().join("publication");
    let moved_root = root_sandbox.path().join("reviewed-publication");
    fs::create_dir(&root_path)?;
    fs::write(root_path.join("payload"), b"reviewed")?;
    let root_fd = open_publication_root_v3(&root_path)?;
    assert!(
        publication_tree_inventory_v3_from_fd_with(&root_path, &root_fd, || {
            fs::rename(&root_path, &moved_root).unwrap();
            symlink(&moved_root, &root_path).unwrap();
        })
        .is_err(),
        "PublicationV3 accepted a root basename symlink to the exact reviewed inode"
    );
    fs::remove_file(&root_path)?;
    fs::rename(&moved_root, &root_path)?;

    let file_root = tempfile::tempdir()?;
    fs::write(file_root.path().join("payload"), b"reviewed")?;
    let file_root_fd = open_publication_root_v3(file_root.path())?;
    assert!(
        publication_tree_inventory_v3_from_fd_with(file_root.path(), &file_root_fd, || {
            fs::rename(
                file_root.path().join("payload"),
                file_root.path().join("reviewed-payload"),
            )
            .unwrap();
            symlink("reviewed-payload", file_root.path().join("payload")).unwrap();
        },)
        .is_err()
    );

    let directory_root = tempfile::tempdir()?;
    fs::create_dir(directory_root.path().join("catalog"))?;
    fs::write(directory_root.path().join("catalog/item"), b"reviewed")?;
    let directory_root_fd = open_publication_root_v3(directory_root.path())?;
    assert!(
        publication_tree_inventory_v3_from_fd_with(
            directory_root.path(),
            &directory_root_fd,
            || {
                fs::rename(
                    directory_root.path().join("catalog"),
                    directory_root.path().join("reviewed-catalog"),
                )
                .unwrap();
                fs::create_dir(directory_root.path().join("catalog")).unwrap();
                fs::write(directory_root.path().join("catalog/item"), b"reviewed").unwrap();
            },
        )
        .is_err()
    );

    let inode_root = tempfile::tempdir()?;
    fs::write(inode_root.path().join("payload"), b"reviewed")?;
    let inode_root_fd = open_publication_root_v3(inode_root.path())?;
    assert!(
        publication_tree_inventory_v3_from_fd_with(inode_root.path(), &inode_root_fd, || {
            fs::rename(
                inode_root.path().join("payload"),
                inode_root.path().join("reviewed-payload"),
            )
            .unwrap();
            fs::write(inode_root.path().join("payload"), b"reviewed").unwrap();
        },)
        .is_err()
    );

    let mode_root = tempfile::tempdir()?;
    fs::create_dir(mode_root.path().join("catalog"))?;
    fs::write(mode_root.path().join("catalog/item"), b"reviewed")?;
    let mode_root_fd = open_publication_root_v3(mode_root.path())?;
    assert!(
        publication_tree_inventory_v3_from_fd_with(mode_root.path(), &mode_root_fd, || {
            fs::set_permissions(
                mode_root.path().join("catalog"),
                fs::Permissions::from_mode(0o700),
            )
            .unwrap();
        })
        .is_err()
    );
    Ok(())
}

#[cfg(target_os = "linux")]
#[test]
fn retained_authority_rejects_root_substitution_after_validation() -> Result<()> {
    use std::os::unix::fs::symlink;

    let sandbox = tempfile::tempdir()?;
    let output = sandbox.path().join("publication");
    let staging = create_pinned_publication_staging_v3(&output)?;
    fs::write(staging.path().join("payload"), b"reviewed")?;
    let candidate = synthetic_validated_staging_v3(&staging)?;
    let authentic = sandbox.path().join("authentic-stage");
    fs::rename(staging.path(), &authentic)?;
    symlink(&authentic, staging.path())?;

    ensure!(
        validate_transition_with(&PublicationTransitionV3::Fresh, &candidate, || {}).is_err(),
        "transition accepted a retained candidate whose named root became a symlink"
    );
    fs::remove_file(staging.path())?;
    fs::rename(&authentic, staging.path())?;
    discard_failed_publication_staging(staging)?;
    Ok(())
}

#[cfg(target_os = "linux")]
#[test]
fn publication_file_hash_rejects_content_mutation_between_passes() -> Result<()> {
    use std::os::unix::fs::PermissionsExt as _;

    let root = tempfile::tempdir()?;
    let path = root.path().join("payload");
    fs::write(&path, b"reviewed")?;
    let mut file = fs::File::open(&path)?;
    let identity = publication_node_identity_v3(&file.metadata()?);
    assert!(
        stable_publication_file_artifact_v3_with(&mut file, &identity, "payload", || {
            fs::write(&path, b"substituted").unwrap()
        },)
        .is_err()
    );

    let mode_path = root.path().join("mode-payload");
    fs::write(&mode_path, b"reviewed")?;
    fs::set_permissions(&mode_path, fs::Permissions::from_mode(0o644))?;
    let mut mode_file = fs::File::open(&mode_path)?;
    let mode_identity = publication_node_identity_v3(&mode_file.metadata()?);
    assert!(
        stable_publication_file_artifact_v3_with(
            &mut mode_file,
            &mode_identity,
            "mode-payload",
            || {
                fs::set_permissions(&mode_path, fs::Permissions::from_mode(0o600)).unwrap();
            },
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn publication_lock_v3_rejects_v2_schema() -> Result<()> {
    let root = tempfile::tempdir()?;
    fs::write(root.path().join("payload"), b"locked")?;
    let mut lock =
        publication_lock_from_actual_for_test(root.path(), Digest32::digest_bytes(b"manifest"))?;
    lock.schema_version = 2;
    assert!(lock.validate().is_err());
    Ok(())
}

#[test]
fn official_campaign_offer_requires_exact_full_completion_and_no_other_policy() -> Result<()> {
    let full_catalog = Digest32::digest_bytes(b"full-campaign-catalog");
    let mission_only = [RulesetBoardScopeV1::IndividualLevel];
    let full_board = [
        RulesetBoardScopeV1::IndividualLevel,
        RulesetBoardScopeV1::FullCampaign,
    ];
    let not_offered = CampaignCompletionPolicyRequirementV1::NotOffered;
    let exact = CampaignCompletionPolicyRequirementV1::Required(
        official_full_campaign_completion_policy_v1(),
    );

    for edition in [
        OfficialContentEditionV1::Demo,
        OfficialContentEditionV1::Full,
    ] {
        validate_official_campaign_offer_fields(
            edition,
            &mission_only,
            &[],
            &not_offered,
            full_catalog,
        )?;
        ensure!(
            validate_official_campaign_offer_fields(
                edition,
                &mission_only,
                &[],
                &exact,
                full_catalog,
            )
            .is_err(),
            "a non-FullCampaign board accepted a completion policy"
        );
    }

    validate_official_campaign_offer_fields(
        OfficialContentEditionV1::Full,
        &full_board,
        &[full_catalog],
        &exact,
        full_catalog,
    )?;
    ensure!(
        validate_official_campaign_offer_fields(
            OfficialContentEditionV1::Demo,
            &full_board,
            &[full_catalog],
            &exact,
            full_catalog,
        )
        .is_err(),
        "Demo advertised a FullCampaign board"
    );
    ensure!(
        validate_official_campaign_offer_fields(
            OfficialContentEditionV1::Full,
            &full_board,
            &[full_catalog],
            &not_offered,
            full_catalog,
        )
        .is_err(),
        "FullCampaign omitted its completion policy"
    );
    let wrong_percent = CampaignCompletionPolicyRequirementV1::Required(
        robin_run_protocol::CampaignCompletionPolicyV1 {
            required_progression_percent: 99,
            ..official_full_campaign_completion_policy_v1()
        },
    );
    ensure!(
        validate_official_campaign_offer_fields(
            OfficialContentEditionV1::Full,
            &full_board,
            &[full_catalog],
            &wrong_percent,
            full_catalog,
        )
        .is_err(),
        "FullCampaign accepted less than 100% progression"
    );
    let wrong_terminal = CampaignCompletionPolicyRequirementV1::Required(
        robin_run_protocol::CampaignCompletionPolicyV1 {
            terminal_subject: robin_run_protocol::OfficialContentSubjectV1::FieldMission {
                mission_id: "H10_Yor_VL".into(),
            },
            required_progression_percent: 100,
        },
    );
    ensure!(
        validate_official_campaign_offer_fields(
            OfficialContentEditionV1::Full,
            &full_board,
            &[full_catalog],
            &wrong_terminal,
            full_catalog,
        )
        .is_err(),
        "FullCampaign accepted a terminal subject other than H12_Not_MP"
    );
    ensure!(
        validate_official_campaign_offer_fields(
            OfficialContentEditionV1::Full,
            &full_board,
            &[Digest32::digest_bytes(b"substituted-catalog")],
            &exact,
            full_catalog,
        )
        .is_err(),
        "FullCampaign accepted a substituted campaign catalog"
    );
    Ok(())
}

#[test]
fn addressed_document_loader_rejects_extra_and_substitution() -> Result<()> {
    let root = tempfile::tempdir()?;
    let config = RulesConfigIdentityV1 {
        schema_version: 1,
        replay_schema_version: CURRENT_RANKED_REPLAY_SCHEMA_VERSION_V1,
        ranked_simulation_policy: robin_run_protocol::RankedSimulationPolicyV1::standard(
            robin_run_protocol::RankedSimulationDifficultyV1::Medium,
        ),
        sim_config: serde_json::from_value(serde_json::to_value(
            robin_engine::engine::SimConfig::default(),
        )?)?,
        rules: BTreeMap::from([(
            "ranked".into(),
            robin_run_protocol::CanonicalValue::Bool(true),
        )]),
    };
    config.validate()?;
    let digest = config.canonical_digest()?;
    fs::write(
        root.path().join(format!("{digest}.json")),
        canonical_json_bytes(&config)?,
    )?;
    let loaded: BTreeMap<Digest32, RulesConfigIdentityV1> =
        load_addressed_documents(root.path(), &[digest])?;
    assert_eq!(loaded.get(&digest), Some(&config));

    fs::write(root.path().join("extra.json"), b"{}")?;
    assert!(load_addressed_documents::<RulesConfigIdentityV1>(root.path(), &[digest]).is_err());
    fs::remove_file(root.path().join("extra.json"))?;
    fs::write(root.path().join(format!("{digest}.json")), b"{}")?;
    assert!(load_addressed_documents::<RulesConfigIdentityV1>(root.path(), &[digest]).is_err());
    Ok(())
}

#[cfg(target_os = "linux")]
#[test]
fn cloudflare_materialization_copies_one_retained_authority_and_validates_exactly() -> Result<()> {
    let sandbox = tempfile::tempdir()?;
    let source = sandbox.path().join("publication");
    fs::create_dir(&source)?;
    let (mut publication, provenance, origins) =
        synthetic_cloudflare_materialization_authority_v1(&source)?;
    let output = sandbox.path().join("materialized");
    let staging = create_pinned_publication_staging_v3(&output)?;
    let (receipt_sha256, candidate) = populate_cloudflare_materialization_staging_v1(
        &staging,
        &mut publication,
        &provenance,
        &origins,
        || {},
    )?;
    assert!(matches!(
        persist_publication_staging(&staging, &candidate, &output)?,
        PublicationPersistenceOutcome::Published
    ));
    let receipt = validate_cloudflare_publication_materialization_v1(&output, receipt_sha256)?;
    ensure!(
        receipt.schema_version == 1
            && receipt.publication_schema_version == 3
            && receipt.publication_lock_sha256 == provenance.publication_lock_sha256
            && receipt.origins.len() == 3
            && receipt.output_inventory.files.iter().any(|file| {
                file.path == "cloudflare-public/_headers" && file.unix_mode == 0o444
            }),
        "materialization receipt omitted its exact source/output authority"
    );
    use std::os::unix::fs::PermissionsExt as _;
    ensure!(
        fs::metadata(&output)?.permissions().mode() & 0o777 == 0o555
            && fs::metadata(output.join("cloudflare-public/index.html"))?
                .permissions()
                .mode()
                & 0o777
                == 0o444,
        "materialization output modes are not canonical"
    );
    make_test_tree_writable(&output)?;
    Ok(())
}

#[cfg(target_os = "linux")]
#[test]
fn cloudflare_materialization_resolves_one_exact_git_commit_and_tree_from_pinned_root() -> Result<()>
{
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .context("manifest tool is not below the repository root")?;
    let repository = open_publication_root_v3(repository)?;
    let (commit, tree) = resolve_cloudflare_materialization_git_authority_v1(&repository)?;
    ensure!(
        valid_lower_hex(&commit, 40) && valid_lower_hex(&tree, 40) && commit != tree,
        "pinned Git authority did not produce one commit/tree pair"
    );
    Ok(())
}

#[cfg(target_os = "linux")]
#[test]
fn cloudflare_materialization_git_tree_ignores_replacement_objects() -> Result<()> {
    let sandbox = tempfile::tempdir()?;
    let repository = sandbox.path().join("repository");
    fs::create_dir(&repository)?;
    let git = |arguments: &[&str]| -> Result<String> {
        let output = std::process::Command::new("/usr/bin/git")
            .args(arguments)
            .current_dir(&repository)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .output()?;
        ensure!(
            output.status.success(),
            "synthetic Git command failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(String::from_utf8(output.stdout)?.trim().to_owned())
    };
    git(&["init", "--quiet"])?;
    git(&["config", "user.name", "Publication V3 test"])?;
    git(&["config", "user.email", "publication-v3@example.invalid"])?;
    fs::write(repository.join("tracked"), b"reviewed tree")?;
    git(&["add", "tracked"])?;
    git(&["commit", "--quiet", "-m", "reviewed"])?;
    let reviewed_commit = git(&["rev-parse", "HEAD^{commit}"])?;
    let reviewed_tree = git(&[
        "--no-replace-objects",
        "rev-parse",
        &format!("{reviewed_commit}^{{tree}}"),
    ])?;
    fs::write(repository.join("tracked"), b"attacker tree")?;
    git(&["add", "tracked"])?;
    git(&["commit", "--quiet", "-m", "attacker"])?;
    let attacker_commit = git(&["rev-parse", "HEAD^{commit}"])?;
    let attacker_tree = git(&[
        "--no-replace-objects",
        "rev-parse",
        &format!("{attacker_commit}^{{tree}}"),
    ])?;
    git(&[
        "--no-replace-objects",
        "checkout",
        "--quiet",
        "--detach",
        &reviewed_commit,
    ])?;
    git(&["replace", &reviewed_commit, &attacker_commit])?;

    let repository_fd = open_publication_root_v3(&repository)?;
    let (resolved_commit, resolved_tree) =
        resolve_cloudflare_materialization_git_authority_v1(&repository_fd)?;
    ensure!(
        resolved_commit == reviewed_commit
            && resolved_tree == reviewed_tree
            && resolved_tree != attacker_tree,
        "Git replacement object substituted Cloudflare source-tree provenance"
    );
    Ok(())
}

#[cfg(target_os = "linux")]
#[test]
fn cloudflare_materialization_rejects_late_source_file_directory_and_root_swaps() -> Result<()> {
    for attack in ["file", "directory", "root"] {
        let sandbox = tempfile::tempdir()?;
        let source = sandbox.path().join("publication");
        fs::create_dir(&source)?;
        let (mut publication, provenance, origins) =
            synthetic_cloudflare_materialization_authority_v1(&source)?;
        let output = sandbox.path().join("materialized");
        let staging = create_pinned_publication_staging_v3(&output)?;
        let original = sandbox.path().join(format!("original-{attack}"));
        let result = populate_cloudflare_materialization_staging_v1(
            &staging,
            &mut publication,
            &provenance,
            &origins,
            || match attack {
                "file" => {
                    let path = source.join("cloudflare-public/index.html");
                    fs::rename(&path, &original).unwrap();
                    fs::write(path, b"public index").unwrap();
                }
                "directory" => {
                    let path = source.join("cloudflare-public");
                    fs::rename(&path, &original).unwrap();
                    fs::create_dir(&path).unwrap();
                    fs::write(path.join("_headers"), b"public headers").unwrap();
                    fs::write(path.join("index.html"), b"public index").unwrap();
                }
                "root" => {
                    fs::rename(&source, &original).unwrap();
                    std::os::unix::fs::symlink(&original, &source).unwrap();
                }
                _ => unreachable!(),
            },
        );
        ensure!(
            result.is_err(),
            "Cloudflare materialization accepted late {attack} substitution"
        );
        discard_failed_publication_staging(staging)?;
        if attack == "root" {
            fs::remove_file(&source)?;
        }
    }
    Ok(())
}

#[cfg(target_os = "linux")]
#[test]
fn cloudflare_materialization_rejects_receipt_mismatch_extras_missing_and_private_leakage()
-> Result<()> {
    use std::os::unix::fs::PermissionsExt as _;

    for attack in ["extra", "empty", "missing", "private", "mode", "inventory"] {
        let sandbox = tempfile::tempdir()?;
        let source = sandbox.path().join("publication");
        fs::create_dir(&source)?;
        let (mut publication, provenance, origins) =
            synthetic_cloudflare_materialization_authority_v1(&source)?;
        let output = sandbox.path().join("materialized");
        let staging = create_pinned_publication_staging_v3(&output)?;
        let (receipt_sha256, candidate) = populate_cloudflare_materialization_staging_v1(
            &staging,
            &mut publication,
            &provenance,
            &origins,
            || {},
        )?;
        assert!(matches!(
            persist_publication_staging(&staging, &candidate, &output)?,
            PublicationPersistenceOutcome::Published
        ));
        ensure!(
            validate_cloudflare_publication_materialization_v1(
                &output,
                Digest32::digest_bytes(b"wrong receipt")
            )
            .is_err(),
            "Cloudflare materialization accepted wrong receipt digest"
        );
        fs::set_permissions(&output, fs::Permissions::from_mode(0o755))?;
        match attack {
            "extra" => fs::write(output.join("extra"), b"injected")?,
            "empty" => fs::create_dir(output.join("unexpected-empty"))?,
            "missing" => {
                fs::set_permissions(
                    output.join("cloudflare-public"),
                    fs::Permissions::from_mode(0o755),
                )?;
                fs::remove_file(output.join("cloudflare-public/index.html"))?;
            }
            "private" => {
                fs::create_dir(output.join("private"))?;
                fs::write(output.join("private/secret"), b"not deployable")?;
            }
            "mode" => fs::set_permissions(
                output.join("cloudflare-public/index.html"),
                fs::Permissions::from_mode(0o644),
            )?,
            "inventory" => {
                fs::set_permissions(
                    output.join("inventories"),
                    fs::Permissions::from_mode(0o755),
                )?;
                let public = output.join("inventories/cloudflare-public-v1.json");
                let signer = output.join("inventories/cloudflare-identity-signer-v1.json");
                let temporary = output.join("inventories/swapped.json");
                fs::rename(&public, &temporary)?;
                fs::rename(&signer, &public)?;
                fs::rename(&temporary, &signer)?;
            }
            _ => unreachable!(),
        }
        ensure!(
            validate_cloudflare_publication_materialization_v1(&output, receipt_sha256).is_err(),
            "Cloudflare materialization accepted {attack} topology attack"
        );
        make_test_tree_writable(&output)?;
    }
    Ok(())
}

#[cfg(target_os = "linux")]
#[test]
fn cloudflare_materialization_rejects_self_consistent_untyped_origin_authority() -> Result<()> {
    for attack in ["private", "empty", "protocol"] {
        let sandbox = tempfile::tempdir()?;
        let source = sandbox.path().join("publication");
        fs::create_dir(&source)?;
        let (mut publication, provenance, origins) =
            synthetic_cloudflare_materialization_authority_v1(&source)?;
        let output = sandbox.path().join("materialized");
        let staging = create_pinned_publication_staging_v3(&output)?;
        let (receipt_sha256, candidate) = populate_cloudflare_materialization_staging_v1(
            &staging,
            &mut publication,
            &provenance,
            &origins,
            || {},
        )?;
        assert!(matches!(
            persist_publication_staging(&staging, &candidate, &output)?,
            PublicationPersistenceOutcome::Published
        ));
        let mut receipt: CloudflarePublicationMaterializationV1 = strict_json_from_slice(
            &fs::read(output.join(CLOUDFLARE_MATERIALIZATION_RECEIPT_PATH))?,
        )?;
        let public_binding = receipt
            .origins
            .iter_mut()
            .find(|binding| binding.origin == CloudflareMaterializationOriginV1::Public)
            .context("synthetic materialization omits public binding")?;
        let public_inventory_path = output.join(&public_binding.inventory_path);
        let mut public: CloudflareMaterializedOriginInventoryV1 =
            strict_json_from_slice(&fs::read(&public_inventory_path)?)?;
        make_test_tree_writable(&output)?;

        match attack {
            "private" => {
                let bytes = b"self-authorized private leak";
                fs::create_dir(output.join("cloudflare-public/private"))?;
                fs::write(output.join("cloudflare-public/private/secret"), bytes)?;
                let artifact = ArtifactRefV1 {
                    sha256: Digest32::digest_bytes(bytes),
                    byte_length: u64::try_from(bytes.len())?,
                    media_type: "application/octet-stream".into(),
                };
                public.files.push(CloudflareMaterializedFileV1 {
                    path: "private/secret".into(),
                    artifact: artifact.clone(),
                    unix_mode: 0o444,
                });
                public.directories.push(PublicationDirectoryV3 {
                    path: "private".into(),
                    unix_mode: 0o555,
                });
                receipt
                    .output_inventory
                    .files
                    .push(CloudflareMaterializedFileV1 {
                        path: "cloudflare-public/private/secret".into(),
                        artifact,
                        unix_mode: 0o444,
                    });
                receipt
                    .output_inventory
                    .directories
                    .push(PublicationDirectoryV3 {
                        path: "cloudflare-public/private".into(),
                        unix_mode: 0o555,
                    });
            }
            "empty" => {
                fs::create_dir(output.join("cloudflare-public/unexpected-empty"))?;
                public.directories.push(PublicationDirectoryV3 {
                    path: "unexpected-empty".into(),
                    unix_mode: 0o555,
                });
                receipt
                    .output_inventory
                    .directories
                    .push(PublicationDirectoryV3 {
                        path: "cloudflare-public/unexpected-empty".into(),
                        unix_mode: 0o555,
                    });
            }
            "protocol" => {
                let build_file = public
                    .files
                    .iter()
                    .find(|file| {
                        file.path.starts_with("manifests/builds/") && file.path.ends_with(".json")
                    })
                    .context("synthetic public inventory omits BuildManifestV2")?
                    .clone();
                let old_digest = build_file
                    .path
                    .strip_prefix("manifests/builds/")
                    .and_then(|name| name.strip_suffix(".json"))
                    .context("synthetic BuildManifestV2 path is malformed")?
                    .to_owned();
                let old_build_path = output.join("cloudflare-public").join(&build_file.path);
                let mut build: BuildManifestV2 =
                    strict_json_from_slice(&fs::read(&old_build_path)?)?;
                build.network_protocol_version = build
                    .network_protocol_version
                    .checked_add(1)
                    .context("synthetic network protocol overflow")?;
                build.validate()?;
                let build_bytes = canonical_json_bytes(&build)?;
                let new_digest = Digest32::digest_bytes(&build_bytes).to_string();
                let new_build_relative = format!("manifests/builds/{new_digest}.json");
                let new_build_path = output.join("cloudflare-public").join(&new_build_relative);
                fs::rename(&old_build_path, &new_build_path)?;
                fs::write(&new_build_path, &build_bytes)?;
                fs::rename(
                    output.join(format!("cloudflare-public/builds/{old_digest}")),
                    output.join(format!("cloudflare-public/builds/{new_digest}")),
                )?;
                let rewrite = |path: &mut String| {
                    if *path == build_file.path {
                        *path = new_build_relative.clone();
                    } else if *path == format!("builds/{old_digest}") {
                        *path = format!("builds/{new_digest}");
                    } else if path.starts_with(&format!("builds/{old_digest}/")) {
                        *path = path.replacen(
                            &format!("builds/{old_digest}/"),
                            &format!("builds/{new_digest}/"),
                            1,
                        );
                    } else if *path == format!("cloudflare-public/builds/{old_digest}") {
                        *path = format!("cloudflare-public/builds/{new_digest}");
                    } else if path.starts_with(&format!("cloudflare-public/builds/{old_digest}/")) {
                        *path = path.replacen(
                            &format!("cloudflare-public/builds/{old_digest}/"),
                            &format!("cloudflare-public/builds/{new_digest}/"),
                            1,
                        );
                    } else if *path
                        == format!("cloudflare-public/manifests/builds/{old_digest}.json")
                    {
                        *path = format!("cloudflare-public/manifests/builds/{new_digest}.json");
                    }
                };
                for file in &mut public.files {
                    rewrite(&mut file.path);
                    if file.path == new_build_relative {
                        file.artifact.sha256 = Digest32::digest_bytes(&build_bytes);
                        file.artifact.byte_length = u64::try_from(build_bytes.len())?;
                    }
                }
                for directory in &mut public.directories {
                    rewrite(&mut directory.path);
                }
                for file in &mut receipt.output_inventory.files {
                    rewrite(&mut file.path);
                    if file.path == format!("cloudflare-public/manifests/builds/{new_digest}.json")
                    {
                        file.artifact.sha256 = Digest32::digest_bytes(&build_bytes);
                        file.artifact.byte_length = u64::try_from(build_bytes.len())?;
                    }
                }
                for directory in &mut receipt.output_inventory.directories {
                    rewrite(&mut directory.path);
                }
            }
            _ => unreachable!(),
        }
        public
            .files
            .sort_by(|left, right| left.path.cmp(&right.path));
        public
            .directories
            .sort_by(|left, right| left.path.cmp(&right.path));
        receipt
            .output_inventory
            .files
            .sort_by(|left, right| left.path.cmp(&right.path));
        receipt
            .output_inventory
            .directories
            .sort_by(|left, right| left.path.cmp(&right.path));
        let public_bytes = canonical_json_bytes(&public)?;
        fs::write(&public_inventory_path, &public_bytes)?;
        public_binding.inventory = ArtifactRefV1 {
            sha256: Digest32::digest_bytes(&public_bytes),
            byte_length: u64::try_from(public_bytes.len())?,
            media_type: "application/json".into(),
        };
        let output_public_inventory = receipt
            .output_inventory
            .files
            .iter_mut()
            .find(|file| file.path == public_binding.inventory_path)
            .context("receipt output inventory omits public inventory")?;
        output_public_inventory.artifact.sha256 = public_binding.inventory.sha256;
        output_public_inventory.artifact.byte_length = public_binding.inventory.byte_length;
        receipt.validate()?;
        let receipt_bytes = canonical_json_bytes(&receipt)?;
        let malicious_receipt_sha256 = Digest32::digest_bytes(&receipt_bytes);
        fs::write(
            output.join(CLOUDFLARE_MATERIALIZATION_RECEIPT_PATH),
            &receipt_bytes,
        )?;
        fs::write(
            output.join(CLOUDFLARE_MATERIALIZATION_RECEIPT_SIDECAR_PATH),
            malicious_receipt_sha256.to_string(),
        )?;
        let output_root = open_publication_root_v3(&output)?;
        let mut expected =
            expected_topology_from_materialized_inventory_v1(&receipt.output_inventory)?;
        expected.register_canonical(CLOUDFLARE_MATERIALIZATION_RECEIPT_PATH.into(), &receipt)?;
        expected.register_bytes(
            CLOUDFLARE_MATERIALIZATION_RECEIPT_SIDECAR_PATH.into(),
            malicious_receipt_sha256.to_string().as_bytes(),
        )?;
        expected.seal_and_validate(&output, &output_root)?;
        ensure!(
            receipt_sha256 != malicious_receipt_sha256
                && validate_cloudflare_publication_materialization_v1(
                    &output,
                    malicious_receipt_sha256,
                )
                .is_err(),
            "standalone validation accepted self-consistent {attack} origin authority"
        );
        make_test_tree_writable(&output)?;
    }
    Ok(())
}

#[cfg(target_os = "linux")]
#[test]
fn cloudflare_materialization_rejects_late_identical_output_inode_swaps() -> Result<()> {
    use std::os::unix::fs::PermissionsExt as _;

    for attack in ["file", "directory", "root"] {
        let sandbox = tempfile::tempdir()?;
        let source = sandbox.path().join("publication");
        fs::create_dir(&source)?;
        let (mut publication, provenance, origins) =
            synthetic_cloudflare_materialization_authority_v1(&source)?;
        let output = sandbox.path().join("materialized");
        let staging = create_pinned_publication_staging_v3(&output)?;
        let orphan = sandbox.path().join(format!("orphan-{attack}"));
        let staging_path = staging.path().to_path_buf();
        let result = populate_cloudflare_materialization_staging_v1(
            &staging,
            &mut publication,
            &provenance,
            &origins,
            || match attack {
                "file" => {
                    let parent = staging_path.join("cloudflare-public");
                    fs::set_permissions(&parent, fs::Permissions::from_mode(0o755)).unwrap();
                    let path = parent.join("index.html");
                    fs::rename(&path, &orphan).unwrap();
                    fs::write(&path, b"public index").unwrap();
                    fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
                    fs::set_permissions(&parent, fs::Permissions::from_mode(0o555)).unwrap();
                }
                "directory" => {
                    fs::set_permissions(&staging_path, fs::Permissions::from_mode(0o755)).unwrap();
                    let path = staging_path.join("cloudflare-public");
                    let displaced = staging_path.join("displaced-public");
                    fs::rename(&path, &displaced).unwrap();
                    copy_directory_exact_preserving_modes(&displaced, &path).unwrap();
                    make_test_tree_writable(&displaced).unwrap();
                    fs::remove_dir_all(&displaced).unwrap();
                    fs::set_permissions(&staging_path, fs::Permissions::from_mode(0o555)).unwrap();
                }
                "root" => {
                    fs::rename(&staging_path, &orphan).unwrap();
                    std::os::unix::fs::symlink(&orphan, &staging_path).unwrap();
                }
                _ => unreachable!(),
            },
        );
        ensure!(
            result.is_err(),
            "Cloudflare materialization accepted late identical {attack} output inode substitution"
        );
        if attack == "root" {
            fs::remove_file(&staging_path)?;
            fs::rename(&orphan, &staging_path)?;
        }
        discard_failed_publication_staging(staging)?;
        if orphan.exists() {
            make_test_tree_writable(&orphan).ok();
            if orphan.is_dir() {
                fs::remove_dir_all(orphan)?;
            } else {
                fs::remove_file(orphan)?;
            }
        }
    }
    Ok(())
}

#[cfg(target_os = "linux")]
#[test]
fn cloudflare_materialization_pre_persist_failures_cleanup_or_preserve_exact_evidence() -> Result<()>
{
    for attack in ["source", "candidate"] {
        let sandbox = tempfile::tempdir()?;
        let source = sandbox.path().join("publication");
        fs::create_dir(&source)?;
        let (mut publication, provenance, origins) =
            synthetic_cloudflare_materialization_authority_v1(&source)?;
        let output = sandbox.path().join("materialized");
        let staging = create_pinned_publication_staging_v3(&output)?;
        let staging_path = staging.path().to_path_buf();
        let (receipt_sha256, candidate) = populate_cloudflare_materialization_staging_v1(
            &staging,
            &mut publication,
            &provenance,
            &origins,
            || {},
        )?;
        let displaced = sandbox.path().join(format!("displaced-{attack}"));
        match attack {
            "source" => {
                let path = source.join("cloudflare-public/index.html");
                fs::rename(&path, &displaced)?;
                fs::write(path, b"public index")?;
            }
            "candidate" => {
                fs::rename(&staging_path, &displaced)?;
                std::os::unix::fs::symlink(&displaced, &staging_path)?;
            }
            _ => unreachable!(),
        }
        ensure!(
            persist_cloudflare_materialization_v1(
                staging,
                &publication,
                &candidate,
                &output,
                receipt_sha256,
            )
            .is_err(),
            "Cloudflare materialization accepted a late pre-persist {attack} substitution"
        );
        ensure!(
            !output.exists(),
            "Cloudflare materialization published after a late pre-persist {attack} substitution"
        );
        if attack == "source" {
            ensure!(
                !staging_path.exists(),
                "ordinary pre-persist source failure left staging behind"
            );
        } else {
            ensure!(
                staging_path.is_symlink() && displaced.is_dir(),
                "uncertain pre-persist candidate identity did not preserve exact evidence"
            );
            fs::remove_file(&staging_path)?;
            make_test_tree_writable(&displaced)?;
            fs::remove_dir_all(&displaced)?;
        }
    }
    Ok(())
}

#[cfg(target_os = "linux")]
#[test]
fn cloudflare_materialization_noreplace_and_sync_outcomes_reuse_exact_candidate() -> Result<()> {
    for sync_failure in [false, true] {
        let sandbox = tempfile::tempdir()?;
        let source = sandbox.path().join("publication");
        fs::create_dir(&source)?;
        let (mut publication, provenance, origins) =
            synthetic_cloudflare_materialization_authority_v1(&source)?;
        let output = sandbox.path().join("materialized");
        let staging = create_pinned_publication_staging_v3(&output)?;
        let (receipt_sha256, candidate) = populate_cloudflare_materialization_staging_v1(
            &staging,
            &mut publication,
            &provenance,
            &origins,
            || {},
        )?;
        let outcome = persist_publication_staging_with(
            &staging,
            &candidate,
            &output,
            || {},
            |parent, source, destination| {
                use std::os::fd::AsFd as _;
                rustix::fs::renameat_with(
                    parent.as_fd(),
                    source,
                    parent.as_fd(),
                    destination,
                    rustix::fs::RenameFlags::NOREPLACE,
                )?;
                Ok(())
            },
            |parent| {
                if sync_failure {
                    anyhow::bail!("injected Cloudflare materialization fsync failure")
                }
                parent.sync_all()?;
                Ok(())
            },
        )?;
        ensure!(
            matches!(
                outcome,
                PublicationPersistenceOutcome::PublishedButParentSyncFailed(_)
            ) == sync_failure,
            "Cloudflare materialization parent-sync outcome was misclassified"
        );
        validate_cloudflare_publication_materialization_v1(&output, receipt_sha256)?;
        make_test_tree_writable(&output)?;
    }

    let sandbox = tempfile::tempdir()?;
    let source = sandbox.path().join("publication");
    fs::create_dir(&source)?;
    let (mut publication, provenance, origins) =
        synthetic_cloudflare_materialization_authority_v1(&source)?;
    let output = sandbox.path().join("materialized");
    let staging = create_pinned_publication_staging_v3(&output)?;
    let (_, candidate) = populate_cloudflare_materialization_staging_v1(
        &staging,
        &mut publication,
        &provenance,
        &origins,
        || {},
    )?;
    fs::create_dir(&output)?;
    ensure!(
        persist_publication_staging(&staging, &candidate, &output).is_err() && output.is_dir(),
        "Cloudflare materialization overwrote a pre-existing output"
    );
    discard_failed_publication_staging(staging)?;
    Ok(())
}
