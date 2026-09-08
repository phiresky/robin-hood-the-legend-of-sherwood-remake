#[cfg(test)]
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[cfg(test)]
use crate::{
    BoardMetricV1, CanonicalDocument as _, CanonicalValue, OpaqueId, PublicKey32,
    VerifiedAchievementEvaluationV1, VerifiedAchievementV1,
};
use crate::{Digest32, Validate, ValidationError};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactRefV1 {
    pub sha256: Digest32,
    pub byte_length: u64,
    pub media_type: String,
}

impl Validate for ArtifactRefV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.sha256.is_zero() || self.byte_length == 0 {
            return Err(ValidationError::Zero {
                field: "artifact.sha256/byte_length",
            });
        }
        crate::validation::text("artifact.media_type", &self.media_type, 128)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ViewerArtifactRoleV1 {
    EntryJavaScript,
    WebAssembly,
    /// Executable JavaScript dependency imported by the single entry module.
    /// This is distinct from non-executable auxiliary data so the complete
    /// wasm-bindgen module graph remains explicit and digest-bound.
    JavaScriptModule {
        name: String,
    },
    Auxiliary {
        name: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NamedArtifactV1 {
    /// Canonical relative path below the deployment's immutable, compile-time
    /// allowlisted viewer origin. It is never an arbitrary URL.
    pub path: String,
    pub role: ViewerArtifactRoleV1,
    pub artifact: ArtifactRefV1,
}

impl Validate for NamedArtifactV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::artifact_relative_url_path("artifact.path", &self.path)?;
        self.artifact.validate()?;
        let is_javascript_path = self.path.ends_with(".js");
        let is_webassembly_path = self.path.ends_with(".wasm");
        let lowercase_path = self.path.to_ascii_lowercase();
        let has_executable_path_extension = [".js", ".mjs", ".cjs", ".jsx", ".wasm"]
            .iter()
            .any(|extension| lowercase_path.ends_with(extension));
        match &self.role {
            ViewerArtifactRoleV1::EntryJavaScript
                if self.artifact.media_type != "text/javascript" =>
            {
                Err(ValidationError::ClaimMismatch {
                    field: "artifact.entry_javascript.media_type",
                })
            }
            ViewerArtifactRoleV1::EntryJavaScript if !is_javascript_path => {
                Err(ValidationError::ClaimMismatch {
                    field: "artifact.entry_javascript.path",
                })
            }
            ViewerArtifactRoleV1::WebAssembly if self.artifact.media_type != "application/wasm" => {
                Err(ValidationError::ClaimMismatch {
                    field: "artifact.webassembly.media_type",
                })
            }
            ViewerArtifactRoleV1::WebAssembly if !is_webassembly_path => {
                Err(ValidationError::ClaimMismatch {
                    field: "artifact.webassembly.path",
                })
            }
            ViewerArtifactRoleV1::JavaScriptModule { name } => {
                crate::validation::text("artifact.javascript_module.name", name, 128)?;
                if self.path != format!("viewer/{name}") {
                    return Err(ValidationError::ClaimMismatch {
                        field: "artifact.javascript_module.path",
                    });
                }
                if !is_javascript_path {
                    return Err(ValidationError::ClaimMismatch {
                        field: "artifact.javascript_module.path",
                    });
                }
                if self.artifact.media_type != "text/javascript" {
                    return Err(ValidationError::ClaimMismatch {
                        field: "artifact.javascript_module.media_type",
                    });
                }
                Ok(())
            }
            ViewerArtifactRoleV1::Auxiliary { name } => {
                crate::validation::text("artifact.auxiliary.name", name, 128)?;
                if has_executable_path_extension {
                    return Err(ValidationError::ClaimMismatch {
                        field: "artifact.auxiliary.executable_path",
                    });
                }
                if matches!(
                    self.artifact.media_type.as_str(),
                    "text/javascript" | "application/javascript" | "application/wasm"
                ) {
                    return Err(ValidationError::ClaimMismatch {
                        field: "artifact.auxiliary.executable_media_type",
                    });
                }
                Ok(())
            }
            ViewerArtifactRoleV1::EntryJavaScript | ViewerArtifactRoleV1::WebAssembly => Ok(()),
        }
    }
}

mod build;
#[cfg(test)]
use build::validate_viewer_artifacts_v1;
pub use build::{
    BINARYEN_WASM_OPT_VERSION_V1, BrowserIdentitySignerBuildIdentityV2,
    BrowserIdentitySignerBuildRecipeV2, BrowserIdentitySignerDeploymentPolicyV2,
    BrowserPagesArtifactV2, BrowserPagesShellBuildIdentityV2, BrowserPagesShellBuildRecipeV2,
    BrowserViewerBuildIdentityV2, BrowserViewerEngineBuildIdentityV2,
    BrowserViewerEngineBuildRecipeV2, BuildManifestV1, BuildManifestV2,
    BuildToolAuthorityDocumentV1, BuildToolAuthorityV1, BuildToolRoleV1, NativeBuildPlatformV2,
    NativeLinkageV2, OFFICIAL_PROJECTION_EXPORTER_MEDIA_TYPE_V2,
    OFFICIAL_VIEWER_BUILD_REPORT_SCHEMA_VERSION_V2, OfficialProjectionAuthorityManifestV2,
    OfficialProjectionExporterBuildIdentityV2, OfficialProjectionExporterPlatformV2,
    OfficialViewerBuildReportV2, OfficialViewerOriginArtifactInventoryV2,
    RANKED_REPLAY_VERIFIER_MEDIA_TYPE_V2, RustToolchainAuthorityV1, VerifierBuildIdentityV2,
    VersionedBuildManifest, WABT_WASM_STRIP_VERSION_V1, WASM_BINDGEN_CLI_AUTHORITY_SHA256_V1,
    WASM_BINDGEN_CLI_VERSION_V1, build_artifact_object_path_v1,
};
mod content;
#[cfg(test)]
use content::OFFICIAL_SIMULATION_COMPONENT_FILE_COUNT_PER_SUBJECT_V1;
pub use content::{
    CampaignContentEntryV1, CampaignContentManifestV1, ContentClosureKindV1, ContentFileRoleV1,
    ContentFileV1, ContentManifestV1, OFFICIAL_DEMO_FIELD_MISSION_IDS_V1,
    OFFICIAL_FULL_CAMPAIGN_GENESIS_MISSION_ID_V1, OFFICIAL_FULL_FIELD_MISSION_IDS_V1,
    OFFICIAL_FULL_HEADQUARTERS_MISSION_ID_V1, OFFICIAL_PROJECTION_EXPORT_REPORT_SCHEMA_VERSION_V2,
    OFFICIAL_PROJECTION_EXPORTER_VERSION_V2, OFFICIAL_PROJECTION_RECEIPT_SCHEMA_VERSION_V2,
    OFFICIAL_SIMULATION_CONTENT_PROJECTION_SCHEMA_VERSION_V1, OfficialBuiltInOverlayBindingV2,
    OfficialBuiltInOverlayKindV2, OfficialBuiltInOverlaySourceManifestV2, OfficialContentEditionV1,
    OfficialContentSubjectV1, OfficialProjectionAudioDurationPolicyV1,
    OfficialProjectionCampaignPolicyV1, OfficialProjectionDifficultyV1,
    OfficialProjectionExecutionPolicyV1, OfficialProjectionExportReportV2,
    OfficialProjectionExporterIdentityV1, OfficialProjectionExporterIdentityV2,
    OfficialProjectionHostStatePolicyV1, OfficialProjectionLocalePolicyV1,
    OfficialProjectionOverlayPolicyV1, OfficialProjectionSourceFormatV1,
    OfficialProjectionSubjectReceiptV1, OfficialSimulationProjectionReceiptV1,
    OfficialSimulationProjectionReceiptV2, OfficialSourceClosureKindV2, OfficialSourceFileV1,
    OfficialSourceTreeManifestV1, OfficialSourceTreeManifestV2, ResourceLocaleRootV1,
    SIMULATION_CONTENT_COMPONENT_MEDIA_TYPE_V1, SimulationContentComponentDocumentV1,
    SimulationContentComponentKindV1, SimulationContentComponentV1, SimulationSpeechTimingSourceV1,
    demo_content_object_path_v1, official_content_manifest_name_v1, official_content_subjects_v1,
    simulation_component_filename_v1, simulation_content_component_relative_path_v1,
    validate_official_content_subjects_v1, validate_official_projection_receipt_matrix_v2,
};
mod ruleset;
pub use ruleset::{
    AchievementPolicyModeV1, AchievementPolicyV1, ActiveTimeDefinitionV1,
    AnonymousParticipantPolicyV1, CampaignAggregationConsentPolicyV1,
    CampaignCompletionPolicyRequirementV1, CampaignCompletionPolicyV1, CampaignRosterContinuityV1,
    CanonicalCampaignStateKindV1, CanonicalCampaignStatePinV1, CanonicalCampaignStateRequirementV1,
    CanonicalStartPolicyV1, FrameCountingPolicyV1, FullCampaignChainPolicyV1,
    FullCampaignTimeAggregationV1, ImmutablePolicyIdentityV1, ImmutablePolicyKindV1,
    ImmutablePolicyManifestV1, InputProvenanceEligibilityV1, MetricRankingPolicyV1,
    NamedParticipantPolicyV1, PaginationTieBreakV1, ParticipantEligibilityV1, PublishedRulesetV1,
    RANKED_SIMULATION_POLICY_VERSION_V1, RankedSimulationDifficultyV1, RankedSimulationPolicyV1,
    RankedSimulationPresetV1, RulesConfigConstraintV1, RulesConfigIdentityV1, RulesetBoardScopeV1,
    RulesetManifestV1, RulesetOperationalStatusV1, RulesetSeedPolicyV1, RunCompositionPolicyV1,
    ScoreAlgorithmV1, ScoreOverflowPolicyV1, TerminalResultPolicyV1, TickDurationV1,
    VisibleTiePolicyV1, official_achievement_policies_v1,
    official_full_campaign_completion_policy_v1,
};
#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn artifact(byte: u8) -> ArtifactRefV1 {
        ArtifactRefV1 {
            sha256: Digest32::from_bytes([byte; 32]),
            byte_length: u64::from(byte),
            media_type: "application/octet-stream".into(),
        }
    }

    fn artifact_with_media(byte: u8, media_type: &str) -> ArtifactRefV1 {
        let mut artifact = artifact(byte);
        artifact.media_type = media_type.into();
        artifact
    }

    #[test]
    fn canonical_campaign_state_authority_is_exact_and_typed() {
        let rules_config_sha256 = Digest32::from_bytes([41; 32]);
        let demo = CanonicalCampaignStateRequirementV1 {
            edition: OfficialContentEditionV1::Demo,
            kind: CanonicalCampaignStateKindV1::IndividualTemplate,
            rules_config_sha256,
        };
        assert!(demo.validate().is_ok());

        let mut wrong_kind = demo;
        wrong_kind.kind = CanonicalCampaignStateKindV1::FullCampaignGenesis;
        assert!(matches!(
            wrong_kind.validate(),
            Err(ValidationError::ClaimMismatch {
                field: "canonical_campaign_state_requirement.edition_kind"
            })
        ));

        let mut missing_config = demo;
        missing_config.rules_config_sha256 = Digest32::default();
        assert!(matches!(
            missing_config.validate(),
            Err(ValidationError::Zero {
                field: "canonical_campaign_state_requirement.rules_config_sha256"
            })
        ));

        let mut pin = CanonicalCampaignStatePinV1 {
            requirement: demo,
            artifact: artifact_with_media(42, crate::RANKED_CAMPAIGN_MEDIA_TYPE_V1),
        };
        assert!(pin.validate().is_ok());
        pin.artifact.media_type = "application/octet-stream".into();
        assert!(matches!(
            pin.validate(),
            Err(ValidationError::ClaimMismatch {
                field: "canonical_campaign_state_pin.artifact.media_type"
            })
        ));
    }

    #[test]
    fn ruleset_rejects_campaign_state_from_another_rules_config() {
        let mut ruleset = ruleset_manifest(Digest32::from_bytes([43; 32]));
        assert!(ruleset.validate().is_ok());
        ruleset.canonical_campaign_state.rules_config_sha256 = Digest32::from_bytes([44; 32]);
        assert!(matches!(
            ruleset.validate(),
            Err(ValidationError::ClaimMismatch {
                field: "ruleset.canonical_campaign_state.rules_config_sha256"
            })
        ));
    }

    fn simulation_component(
        kind: SimulationContentComponentKindV1,
        byte: u8,
    ) -> SimulationContentComponentV1 {
        SimulationContentComponentV1 {
            kind,
            component_schema_version: 1,
            artifact: artifact_with_media(byte, SIMULATION_CONTENT_COMPONENT_MEDIA_TYPE_V1),
        }
    }

    fn simulation_components() -> Vec<SimulationContentComponentV1> {
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
        .map(|(index, kind)| simulation_component(kind, index as u8 + 1))
        .collect()
    }

    fn policy_identity(kind: ImmutablePolicyKindV1, byte: u8) -> ImmutablePolicyIdentityV1 {
        ImmutablePolicyIdentityV1 {
            kind,
            version: 1,
            manifest_sha256: Digest32::from_bytes([byte; 32]),
        }
    }

    pub(crate) fn ruleset_manifest(rules_config_sha256: Digest32) -> RulesetManifestV1 {
        RulesetManifestV1 {
            schema_version: 1,
            display_name: "Standard / Normal".into(),
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
                maximum_max_concurrent_players: crate::MAX_REPLAY_SEATS_V1,
                maximum_participant_instances: crate::MAX_PARTICIPANT_INSTANCES_V1,
            },
            replay_schema_versions: vec![crate::CURRENT_RANKED_REPLAY_SCHEMA_VERSION_V1],
            network_protocol_versions: vec![
                crate::CURRENT_RANKED_NETWORK_PROTOCOL_VERSION_V1,
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
        }
    }

    #[test]
    fn build_manifest_digest_is_canonical_and_sensitive() {
        let mut manifest = BuildManifestV1 {
            schema_version: 1,
            source_commit: "a".repeat(40),
            cargo_lock_sha256: Digest32::from_bytes([1; 32]),
            target_triple: "x86_64-unknown-linux-gnu".into(),
            cargo_profile: "verifier".into(),
            cargo_features: vec!["native-fs".into(), "replay".into()],
            replay_schema_version: crate::CURRENT_RANKED_REPLAY_SCHEMA_VERSION_V1,
            save_schema_version: crate::CURRENT_RANKED_SAVE_SCHEMA_VERSION_V1,
            network_protocol_version: crate::CURRENT_RANKED_NETWORK_PROTOCOL_VERSION_V1,
            verifier: artifact(2),
            viewer_artifacts: vec![
                NamedArtifactV1 {
                    path: "viewer/entry.js".into(),
                    role: ViewerArtifactRoleV1::EntryJavaScript,
                    artifact: artifact_with_media(3, "text/javascript"),
                },
                NamedArtifactV1 {
                    path: "viewer/robin.wasm".into(),
                    role: ViewerArtifactRoleV1::WebAssembly,
                    artifact: artifact_with_media(4, "application/wasm"),
                },
            ],
        };
        let first = manifest.canonical_digest().unwrap();
        assert_eq!(first, manifest.canonical_digest().unwrap());
        assert_eq!(
            build_artifact_object_path_v1(first, &manifest.viewer_artifacts[0]).unwrap(),
            format!("builds/{first}/viewer/entry.js")
        );
        manifest.replay_schema_version += 1;
        assert_ne!(first, manifest.canonical_digest().unwrap());
        manifest.cargo_features.swap(0, 1);
        assert!(manifest.canonical_digest().is_err());
        manifest.cargo_features.sort();
        for unsafe_path in [
            "https://evil.invalid/robin.wasm",
            "javascript:alert.js",
            "data:text/javascript,x",
            "viewer/%2e%2e/x.js",
            "viewer/x.js?redirect=evil",
            "viewer/x.js#fragment",
            "//evil.invalid/x.js",
        ] {
            manifest.viewer_artifacts[0].path = unsafe_path.into();
            assert!(manifest.validate().is_err(), "accepted {unsafe_path}");
        }
    }

    #[test]
    fn viewer_closure_accepts_typed_javascript_modules_and_rejects_executable_auxiliaries() {
        let module = NamedArtifactV1 {
            path: "viewer/snippets/browser_identity_client.js".into(),
            role: ViewerArtifactRoleV1::JavaScriptModule {
                name: "snippets/browser_identity_client.js".into(),
            },
            artifact: artifact_with_media(5, "text/javascript"),
        };
        assert_eq!(
            serde_json::to_value(ViewerArtifactRoleV1::EntryJavaScript).unwrap()["kind"],
            "entry_java_script"
        );
        assert_eq!(
            serde_json::to_value(&module.role).unwrap()["kind"],
            "java_script_module"
        );
        assert!(module.validate().is_ok());

        let mut wrong_media = module.clone();
        wrong_media.artifact.media_type = "application/wasm".into();
        assert!(wrong_media.validate().is_err());

        let mut wrong_path = module.clone();
        wrong_path.path = "viewer/snippets/substituted.js".into();
        assert!(wrong_path.validate().is_err());

        let mut wrong_extension = module.clone();
        wrong_extension.path = "viewer/snippets/browser_identity_client.json".into();
        wrong_extension.role = ViewerArtifactRoleV1::JavaScriptModule {
            name: "snippets/browser_identity_client.json".into(),
        };
        assert!(wrong_extension.validate().is_err());

        let mut executable_auxiliary = module.clone();
        executable_auxiliary.role = ViewerArtifactRoleV1::Auxiliary {
            name: "snippets/browser_identity_client.js".into(),
        };
        assert!(executable_auxiliary.validate().is_err());

        let mut disguised_executable_auxiliary = executable_auxiliary;
        disguised_executable_auxiliary.artifact.media_type = "application/json".into();
        assert!(disguised_executable_auxiliary.validate().is_err());

        let wrong_entry_path = NamedArtifactV1 {
            path: "viewer/entry.json".into(),
            role: ViewerArtifactRoleV1::EntryJavaScript,
            artifact: artifact_with_media(7, "text/javascript"),
        };
        assert!(wrong_entry_path.validate().is_err());

        let wrong_wasm_path = NamedArtifactV1 {
            path: "viewer/module.json".into(),
            role: ViewerArtifactRoleV1::WebAssembly,
            artifact: artifact_with_media(8, "application/wasm"),
        };
        assert!(wrong_wasm_path.validate().is_err());

        let mut closure = vec![
            NamedArtifactV1 {
                path: "viewer/robin.js".into(),
                role: ViewerArtifactRoleV1::EntryJavaScript,
                artifact: artifact_with_media(3, "text/javascript"),
            },
            NamedArtifactV1 {
                path: "viewer/robin_bg.wasm".into(),
                role: ViewerArtifactRoleV1::WebAssembly,
                artifact: artifact_with_media(4, "application/wasm"),
            },
            module,
        ];
        closure.sort_by(|left, right| left.path.cmp(&right.path));
        assert!(validate_viewer_artifacts_v1(&closure).is_ok());

        let mut duplicate_role = closure;
        duplicate_role.push(NamedArtifactV1 {
            path: "viewer/snippets/browser_identity_vault.js".into(),
            role: ViewerArtifactRoleV1::JavaScriptModule {
                name: "snippets/browser_identity_client.js".into(),
            },
            artifact: artifact_with_media(6, "text/javascript"),
        });
        duplicate_role.sort_by(|left, right| left.path.cmp(&right.path));
        assert!(validate_viewer_artifacts_v1(&duplicate_role).is_err());
    }

    fn projection_build_v2() -> BuildManifestV2 {
        let binaryen_authority: BuildToolAuthorityDocumentV1 =
            serde_json::from_str(include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../.github/tool-authorities/binaryen-wasm-opt-v132.json"
            )))
            .unwrap();
        let wabt_authority: BuildToolAuthorityDocumentV1 =
            serde_json::from_str(include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../.github/tool-authorities/wabt-wasm-strip-v1.0.41.json"
            )))
            .unwrap();
        let rust_toolchain = RustToolchainAuthorityV1 {
            schema_version: 1,
            channel: "nightly-2026-08-25".into(),
            components: vec!["rust-src".into(), "rustc-codegen-cranelift-preview".into()],
            targets: vec!["wasm32-unknown-unknown".into()],
        };
        BuildManifestV2 {
            schema_version: 2,
            source_commit: "b".repeat(40),
            cargo_lock_sha256: Digest32::from_bytes([11; 32]),
            replay_schema_version: crate::CURRENT_RANKED_REPLAY_SCHEMA_VERSION_V1,
            save_schema_version: crate::CURRENT_RANKED_SAVE_SCHEMA_VERSION_V1,
            network_protocol_version: crate::CURRENT_RANKED_NETWORK_PROTOCOL_VERSION_V1,
            verifier: VerifierBuildIdentityV2 {
                platform: NativeBuildPlatformV2::X86_64UnknownLinuxMusl,
                target_triple: "x86_64-unknown-linux-musl".into(),
                cargo_profile: "release".into(),
                cargo_features: vec![],
                cargo_package: "robin_replay_verifier".into(),
                cargo_binary: "robin-replay-verifier".into(),
                linkage: NativeLinkageV2::FullyStaticNoInterpreterOrNeededLibraries,
                artifact: artifact_with_media(12, RANKED_REPLAY_VERIFIER_MEDIA_TYPE_V2),
            },
            viewer: BrowserViewerBuildIdentityV2 {
                engine: BrowserViewerEngineBuildIdentityV2 {
                    target_triple: "wasm32-unknown-unknown".into(),
                    cargo_profile: "wasm-release".into(),
                    cargo_features: vec!["audio".into()],
                    cargo_package: "robin_rs".into(),
                    cargo_binary: "robin".into(),
                    recipe: BrowserViewerEngineBuildRecipeV2::WasmBindgenWebBinaryenOzStripDebugDwarfWabtStripV1,
                    rust_toolchain_sha256: rust_toolchain.canonical_digest().unwrap(),
                    rust_toolchain: rust_toolchain.clone(),
                    wasm_bindgen_cli: build_tool(16, "0.2.127"),
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
                            path: "viewer/robin.js".into(),
                            role: ViewerArtifactRoleV1::EntryJavaScript,
                            artifact: artifact_with_media(13, "text/javascript"),
                        },
                        NamedArtifactV1 {
                            path: "viewer/robin_bg.wasm".into(),
                            role: ViewerArtifactRoleV1::WebAssembly,
                            artifact: artifact_with_media(14, "application/wasm"),
                        },
                    ],
                },
                pages_shell: BrowserPagesShellBuildIdentityV2 {
                    recipe:
                        BrowserPagesShellBuildRecipeV2::PnpmFrozenLockfileViteStaticShellV1,
                    node: build_tool(22, "24.19.0"),
                    pnpm: build_tool(19, "12.3.4"),
                    package_json_sha256: Digest32::from_bytes([20; 32]),
                    pnpm_lock_sha256: Digest32::from_bytes([21; 32]),
                    public_origin_artifacts: vec![BrowserPagesArtifactV2 {
                        path: "index.html".into(),
                        artifact: artifact_with_media(22, "text/html"),
                    }],
                },
                identity_signer: BrowserIdentitySignerBuildIdentityV2 {
                    target_triple: "wasm32-unknown-unknown".into(),
                    cargo_profile: "wasm-release".into(),
                    cargo_features: vec!["identity-signer-bridge".into()],
                    cargo_package: "robin_rs".into(),
                    cargo_binary: "leaderboard_identity_bridge".into(),
                    recipe:
                        BrowserIdentitySignerBuildRecipeV2::WasmBindgenWebSeparateOriginBridgeV1,
                    deployment_policy: BrowserIdentitySignerDeploymentPolicyV2::SeparateAllowlistedOriginCspFrameAncestorsAndBridgeShaV1,
                    rust_toolchain_sha256: rust_toolchain.canonical_digest().unwrap(),
                    rust_toolchain,
                    wasm_bindgen_cli: build_tool(16, "0.2.127"),
                    identity_signer_origin_artifacts: vec![
                        BrowserPagesArtifactV2 {
                            path: "identity-signer/bridge/leaderboard_identity_bridge.js".into(),
                            artifact: artifact_with_media(24, "text/javascript"),
                        },
                        BrowserPagesArtifactV2 {
                            path: "identity-signer/bridge/leaderboard_identity_bridge_bg.wasm"
                                .into(),
                            artifact: artifact_with_media(25, "application/wasm"),
                        },
                        BrowserPagesArtifactV2 {
                            path: "identity-signer/index.html".into(),
                            artifact: artifact_with_media(23, "text/html"),
                        },
                    ],
                },
            },
        }
    }

    #[test]
    fn isolated_signer_package_migration_preserves_historical_identity_and_closed_recipe() {
        use ed25519_dalek::Signer;

        let historical = projection_build_v2();
        historical.validate().unwrap();
        let historical_digest = historical.canonical_digest().unwrap();
        let key = ed25519_dalek::SigningKey::from_bytes(&[7; 32]);
        let historical_bytes = historical.canonical_bytes().unwrap();
        let historical_signature = key.sign(&historical_bytes);
        let mut current = historical.clone();
        current.viewer.identity_signer.cargo_package = "robin_identity_signer".into();
        current.validate().unwrap();
        assert_ne!(historical_digest, current.canonical_digest().unwrap());
        assert_eq!(historical_digest, historical.canonical_digest().unwrap());
        key.verifying_key()
            .verify_strict(
                &historical.canonical_bytes().unwrap(),
                &historical_signature,
            )
            .unwrap();
        assert!(
            key.verifying_key()
                .verify_strict(&current.canonical_bytes().unwrap(), &historical_signature)
                .is_err(),
            "a historical signature must never authorize relabeling the signer package"
        );
        current.viewer.identity_signer.cargo_package = "untrusted_signer".into();
        assert!(current.validate().is_err());
        current.viewer.identity_signer.cargo_package = "robin_identity_signer".into();
        current
            .viewer
            .identity_signer
            .cargo_features
            .push("audio".into());
        assert!(current.validate().is_err());
    }

    fn build_tool(byte: u8, version: &str) -> BuildToolAuthorityV1 {
        BuildToolAuthorityV1 {
            version: version.into(),
            authority_sha256: if version == WASM_BINDGEN_CLI_VERSION_V1 {
                WASM_BINDGEN_CLI_AUTHORITY_SHA256_V1
            } else {
                Digest32::from_bytes([byte; 32])
            },
        }
    }

    #[test]
    fn official_wasm_tool_authorities_are_exact_upstream_distributions() {
        let wasm_bindgen_json = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../.github/tool-authorities/wasm-bindgen-cli-v0.2.127.json"
        ));
        let binaryen_json = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../.github/tool-authorities/binaryen-wasm-opt-v132.json"
        ));
        let wabt_json = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../.github/tool-authorities/wabt-wasm-strip-v1.0.41.json"
        ));
        let wasm_bindgen: BuildToolAuthorityDocumentV1 =
            serde_json::from_str(wasm_bindgen_json).unwrap();
        let binaryen: BuildToolAuthorityDocumentV1 = serde_json::from_str(binaryen_json).unwrap();
        let wabt: BuildToolAuthorityDocumentV1 = serde_json::from_str(wabt_json).unwrap();
        wasm_bindgen.validate().unwrap();
        binaryen.validate().unwrap();
        wabt.validate().unwrap();
        assert_eq!(
            wasm_bindgen.canonical_bytes().unwrap(),
            wasm_bindgen_json.trim_end().as_bytes()
        );
        assert_eq!(
            wasm_bindgen.canonical_digest().unwrap(),
            WASM_BINDGEN_CLI_AUTHORITY_SHA256_V1
        );
        assert_eq!(
            binaryen.canonical_bytes().unwrap(),
            binaryen_json.trim_end().as_bytes()
        );
        assert_eq!(
            wabt.canonical_bytes().unwrap(),
            wabt_json.trim_end().as_bytes()
        );

        let installer = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../scripts/install_pinned_wasm_tools.sh"
        ));
        for exact_fact in [
            binaryen.distribution_url.clone(),
            binaryen.distribution.sha256.to_string(),
            binaryen.distribution.byte_length.to_string(),
            wabt.distribution_url.clone(),
            wabt.distribution.sha256.to_string(),
            wabt.distribution.byte_length.to_string(),
            "wasm-opt version 132 (version_132)".into(),
            "1.0.41".into(),
        ] {
            assert!(
                installer.contains(&exact_fact),
                "installer drifted from {exact_fact}"
            );
        }
        let wasm_bindgen_installer = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../scripts/install_pinned_wasm_bindgen.sh"
        ));
        for exact_fact in [
            wasm_bindgen.distribution_url.clone(),
            wasm_bindgen.distribution.sha256.to_string(),
            wasm_bindgen.distribution.byte_length.to_string(),
            "420a0f944d2521032fbdc6a46d1519b7943ffe8254d019dc37ab8865428fceb9".into(),
            "59375".into(),
            "wasm-bindgen 0.2.127".into(),
        ] {
            assert!(
                wasm_bindgen_installer.contains(&exact_fact),
                "wasm-bindgen installer drifted from {exact_fact}"
            );
        }

        let build = projection_build_v2();
        build
            .viewer
            .engine
            .wasm_bindgen_cli
            .validate_against(BuildToolRoleV1::WasmBindgenCli, &wasm_bindgen)
            .unwrap();
        build
            .viewer
            .engine
            .binaryen_wasm_opt
            .validate_against(BuildToolRoleV1::BinaryenWasmOpt, &binaryen)
            .unwrap();
        build
            .viewer
            .engine
            .wabt_wasm_strip
            .validate_against(BuildToolRoleV1::WabtWasmStrip, &wabt)
            .unwrap();
        build
            .validate_wasm_tool_authorities(&wasm_bindgen, &binaryen, &wabt)
            .unwrap();

        let mut swapped_distribution = binaryen.clone();
        swapped_distribution.distribution.sha256 = wabt.distribution.sha256;
        assert!(swapped_distribution.validate().is_err());

        let mut moving_version = wabt.clone();
        moving_version.version = "latest".into();
        assert!(moving_version.validate().is_err());

        let mut substituted_role = build.viewer.engine.binaryen_wasm_opt.clone();
        substituted_role.authority_sha256 = wabt.canonical_digest().unwrap();
        assert!(
            substituted_role
                .validate_against(BuildToolRoleV1::BinaryenWasmOpt, &binaryen)
                .is_err()
        );
    }

    fn projection_authority_v2(build: &BuildManifestV2) -> OfficialProjectionAuthorityManifestV2 {
        OfficialProjectionAuthorityManifestV2 {
            schema_version: 2,
            public_build_manifest_sha256: build.canonical_digest().unwrap(),
            source_commit: build.source_commit.clone(),
            cargo_lock_sha256: build.cargo_lock_sha256,
            replay_schema_version: build.replay_schema_version,
            save_schema_version: build.save_schema_version,
            network_protocol_version: build.network_protocol_version,
            projection_exporter: OfficialProjectionExporterBuildIdentityV2 {
                platform: OfficialProjectionExporterPlatformV2::X86_64UnknownLinuxMusl,
                target_triple: "x86_64-unknown-linux-musl".into(),
                cargo_profile: "release".into(),
                cargo_features: vec!["projection-export".into()],
                cargo_package: "robin_rs".into(),
                cargo_example: "export_simulation_content".into(),
                linkage: NativeLinkageV2::FullyStaticNoInterpreterOrNeededLibraries,
                exporter_version: OFFICIAL_PROJECTION_EXPORTER_VERSION_V2,
                simulation_content_projection_schema_version:
                    OFFICIAL_SIMULATION_CONTENT_PROJECTION_SCHEMA_VERSION_V1,
                artifact: artifact_with_media(15, OFFICIAL_PROJECTION_EXPORTER_MEDIA_TYPE_V2),
            },
        }
    }

    fn official_rules_config() -> RulesConfigIdentityV1 {
        RulesConfigIdentityV1 {
            schema_version: 1,
            replay_schema_version: crate::CURRENT_RANKED_REPLAY_SCHEMA_VERSION_V1,
            ranked_simulation_policy: RankedSimulationPolicyV1::standard(
                RankedSimulationDifficultyV1::Medium,
            ),
            sim_config: BTreeMap::from([
                ("difficulty".into(), CanonicalValue::String("Medium".into())),
                ("script_enabled".into(), CanonicalValue::Bool(true)),
            ]),
            rules: BTreeMap::from([("ranked".into(), CanonicalValue::Bool(true))]),
        }
    }

    fn official_execution_policy() -> OfficialProjectionExecutionPolicyV1 {
        OfficialProjectionExecutionPolicyV1 {
            schema_version: 1,
            policy_version: 1,
            difficulty: OfficialProjectionDifficultyV1::Medium,
            simulation_seed: crate::SimulationSeed64::new(0),
            rules_config: official_rules_config(),
            campaign:
                OfficialProjectionCampaignPolicyV1::FreshRepresentativeCampaignPerOfficialSubjectV1,
            locale:
                OfficialProjectionLocalePolicyV1::ExactReceiptLcidBeforeResourceInitializationV1,
            audio_durations:
                OfficialProjectionAudioDurationPolicyV1::RebuildFromSourceClosureNoPersistentCacheV1,
            overlays:
                OfficialProjectionOverlayPolicyV1::BuiltInCoreOnlyRejectUserModEnvironmentV1,
            host_state:
                OfficialProjectionHostStatePolicyV1::CanonicalInMemoryNoPersistedPreferencesIdentitySaveOrEnvironmentV1,
        }
    }

    fn source_file(path: &str, byte: u8) -> OfficialSourceFileV1 {
        OfficialSourceFileV1 {
            path: path.into(),
            sha256: Digest32::from_bytes([byte; 32]),
            byte_length: u64::from(byte),
        }
    }

    fn source_tree_v2(
        edition: OfficialContentEditionV1,
        source_format: OfficialProjectionSourceFormatV1,
    ) -> OfficialSourceTreeManifestV2 {
        let (closure_kind, mut files) = match source_format {
            OfficialProjectionSourceFormatV1::LooseNativeV1 => {
                let locale = match edition {
                    OfficialContentEditionV1::Demo => "1033",
                    OfficialContentEditionV1::Full => "2047",
                };
                (
                    OfficialSourceClosureKindV2::LooseNativeSimulationConsumedV1,
                    vec![
                        source_file(&format!("{locale}/Data/Text/Level.res"), 20),
                        source_file("Data/Configuration/profile.cpf", 21),
                    ],
                )
            }
            OfficialProjectionSourceFormatV1::ShippingDatadirV10 => (
                OfficialSourceClosureKindV2::ShippingDatadirV10ArchiveAndReferencedSplitsV1,
                vec![
                    source_file("Data/datadir.bin", 22),
                    source_file("Data/missions/mission-00.rhmission.zst", 23),
                ],
            ),
        };
        files.sort_by(|left, right| left.path.cmp(&right.path));
        OfficialSourceTreeManifestV2 {
            schema_version: 2,
            edition,
            source_format,
            closure_kind,
            files,
        }
    }

    fn core_overlay_v2() -> OfficialBuiltInOverlaySourceManifestV2 {
        OfficialBuiltInOverlaySourceManifestV2 {
            schema_version: 2,
            kind: OfficialBuiltInOverlayKindV2::CoreDatadirV1,
            files: vec![source_file("Data/Configuration/core.json", 30)],
        }
    }

    fn official_subject_receipts(
        edition: OfficialContentEditionV1,
    ) -> Vec<OfficialProjectionSubjectReceiptV1> {
        let locale = match edition {
            OfficialContentEditionV1::Demo => "1033",
            OfficialContentEditionV1::Full => "2047",
        };
        official_content_subjects_v1(edition)
            .into_iter()
            .map(|subject| OfficialProjectionSubjectReceiptV1 {
                content_manifest: ContentManifestV1 {
                    schema_version: 1,
                    name: official_content_manifest_name_v1(edition, &subject),
                    edition,
                    subject,
                    closure: ContentClosureKindV1::StaticPreparedMissionContentProjection,
                    projection_schema_version:
                        OFFICIAL_SIMULATION_CONTENT_PROJECTION_SCHEMA_VERSION_V1,
                    resource_locale_root: ResourceLocaleRootV1::new(locale).unwrap(),
                    speech_timing: SimulationSpeechTimingSourceV1::BaseInstallation,
                    components: simulation_components(),
                },
            })
            .collect()
    }

    fn projection_receipt_v2(
        edition: OfficialContentEditionV1,
        source_format: OfficialProjectionSourceFormatV1,
        projection_authority: &OfficialProjectionAuthorityManifestV2,
        rules_config: &RulesConfigIdentityV1,
        overlay: &OfficialBuiltInOverlaySourceManifestV2,
    ) -> (
        OfficialSourceTreeManifestV2,
        OfficialSimulationProjectionReceiptV2,
    ) {
        let source = source_tree_v2(edition, source_format);
        let execution_policy = official_execution_policy();
        assert_eq!(&execution_policy.rules_config, rules_config);
        let receipt = OfficialSimulationProjectionReceiptV2 {
            schema_version: OFFICIAL_PROJECTION_RECEIPT_SCHEMA_VERSION_V2,
            exporter: OfficialProjectionExporterIdentityV2 {
                exporter_version: projection_authority.projection_exporter.exporter_version,
                simulation_content_projection_schema_version: projection_authority
                    .projection_exporter
                    .simulation_content_projection_schema_version,
                platform: projection_authority.projection_exporter.platform,
                source_format,
                projection_authority_manifest_sha256: projection_authority
                    .canonical_digest()
                    .unwrap(),
                exporter_artifact: projection_authority.projection_exporter.artifact.clone(),
            },
            edition,
            source_tree_manifest_sha256: source.canonical_digest().unwrap(),
            source_file_count: u32::try_from(source.files.len()).unwrap(),
            built_in_overlay: OfficialBuiltInOverlayBindingV2 {
                kind: overlay.kind,
                source_manifest_sha256: overlay.canonical_digest().unwrap(),
                source_file_count: u32::try_from(overlay.files.len()).unwrap(),
            },
            rules_config_sha256: rules_config.canonical_digest().unwrap(),
            execution_policy_sha256: execution_policy.canonical_digest().unwrap(),
            execution_policy,
            subjects: official_subject_receipts(edition),
        };
        (source, receipt)
    }

    fn projection_export_report_v2(
        receipt: &OfficialSimulationProjectionReceiptV2,
        build: &BuildManifestV2,
    ) -> OfficialProjectionExportReportV2 {
        let subjects = receipt
            .subjects
            .iter()
            .map(|subject| subject.content_manifest.subject.clone())
            .collect::<Vec<_>>();
        OfficialProjectionExportReportV2 {
            schema_version: OFFICIAL_PROJECTION_EXPORT_REPORT_SCHEMA_VERSION_V2,
            edition: receipt.edition,
            source_format: receipt.exporter.source_format,
            catalog_root: "/output/catalog".into(),
            receipt_root: "/output/receipt".into(),
            source_tree_manifest_sha256: receipt.source_tree_manifest_sha256,
            projection_receipt_sha256: receipt.canonical_digest().unwrap(),
            public_build_manifest_sha256: build.canonical_digest().unwrap(),
            projection_authority_manifest_sha256: receipt
                .exporter
                .projection_authority_manifest_sha256,
            rules_config_sha256: receipt.rules_config_sha256,
            execution_policy_sha256: receipt.execution_policy_sha256,
            core_overlay_manifest_sha256: receipt.built_in_overlay.source_manifest_sha256,
            exporter_artifact: receipt.exporter.exporter_artifact.clone(),
            source_file_count: receipt.source_file_count,
            core_overlay_file_count: receipt.built_in_overlay.source_file_count,
            resource_locale_root: receipt.subjects[0]
                .content_manifest
                .resource_locale_root
                .clone(),
            component_file_count: u32::try_from(
                subjects.len() * OFFICIAL_SIMULATION_COMPONENT_FILE_COUNT_PER_SUBJECT_V1,
            )
            .unwrap(),
            subjects,
        }
    }

    #[test]
    fn projection_receipt_v2_cross_binds_every_private_authority() {
        let build = projection_build_v2();
        let authority = projection_authority_v2(&build);
        let rules = official_rules_config();
        let overlay = core_overlay_v2();
        let (source, receipt) = projection_receipt_v2(
            OfficialContentEditionV1::Demo,
            OfficialProjectionSourceFormatV1::LooseNativeV1,
            &authority,
            &rules,
            &overlay,
        );
        receipt
            .validate_against(&build, &authority, &rules, &source, &overlay)
            .unwrap();

        let mut swapped_receipt = receipt.clone();
        swapped_receipt.exporter.exporter_artifact.sha256 = Digest32::from_bytes([99; 32]);
        assert!(
            swapped_receipt
                .validate_against(&build, &authority, &rules, &source, &overlay)
                .is_err()
        );

        let mut swapped_build = build.clone();
        swapped_build.source_commit = "c".repeat(40);
        assert!(
            receipt
                .validate_against(&swapped_build, &authority, &rules, &source, &overlay)
                .is_err()
        );

        let mut swapped_authority = authority.clone();
        swapped_authority.projection_exporter.artifact.sha256 = Digest32::from_bytes([96; 32]);
        assert!(
            receipt
                .validate_against(&build, &swapped_authority, &rules, &source, &overlay)
                .is_err()
        );

        let mut swapped_rules = rules.clone();
        swapped_rules
            .sim_config
            .insert("difficulty".into(), CanonicalValue::String("hard".into()));
        assert!(
            receipt
                .validate_against(&build, &authority, &swapped_rules, &source, &overlay)
                .is_err()
        );

        let mut swapped_overlay = overlay.clone();
        swapped_overlay.files[0].sha256 = Digest32::from_bytes([98; 32]);
        assert!(
            receipt
                .validate_against(&build, &authority, &rules, &source, &swapped_overlay)
                .is_err()
        );

        let mut swapped_source = source.clone();
        swapped_source.files[0].sha256 = Digest32::from_bytes([97; 32]);
        assert!(
            receipt
                .validate_against(&build, &authority, &rules, &swapped_source, &overlay)
                .is_err()
        );

        let mut swapped_execution = receipt.clone();
        swapped_execution
            .execution_policy
            .rules_config
            .sim_config
            .insert("difficulty".into(), CanonicalValue::String("Hard".into()));
        assert!(swapped_execution.validate().is_err());
    }

    #[test]
    fn public_build_v2_is_private_authority_free_and_projects_exact_backend_v1() {
        let build = projection_build_v2();
        build.validate().unwrap();
        let canonical = build.canonical_bytes().unwrap();
        let public_json = String::from_utf8(canonical.clone()).unwrap();
        for forbidden in [
            "projection_exporter",
            "projection_authority",
            OFFICIAL_PROJECTION_EXPORTER_MEDIA_TYPE_V2,
            "/authority/",
            "/input",
            "/core",
        ] {
            assert!(
                !public_json.contains(forbidden),
                "public BuildManifestV2 leaked {forbidden}"
            );
        }
        let public_content =
            serde_json::to_string(&official_subject_receipts(OfficialContentEditionV1::Demo))
                .unwrap();
        for forbidden in [
            "projection_exporter",
            "projection_authority",
            OFFICIAL_PROJECTION_EXPORTER_MEDIA_TYPE_V2,
        ] {
            assert!(
                !public_content.contains(forbidden),
                "public content catalog leaked {forbidden}"
            );
        }

        let mut injected: serde_json::Value = serde_json::from_slice(&canonical).unwrap();
        injected.as_object_mut().unwrap().insert(
            "projection_exporter".into(),
            serde_json::Value::String("private".into()),
        );
        assert!(serde_json::from_value::<BuildManifestV2>(injected).is_err());

        let projected = build.backend_visible_v1().unwrap();
        let expected = BuildManifestV1 {
            schema_version: 1,
            source_commit: build.source_commit.clone(),
            cargo_lock_sha256: build.cargo_lock_sha256,
            target_triple: "wasm32-unknown-unknown".into(),
            cargo_profile: "wasm-release".into(),
            cargo_features: vec!["audio".into()],
            replay_schema_version: crate::CURRENT_RANKED_REPLAY_SCHEMA_VERSION_V1,
            save_schema_version: crate::CURRENT_RANKED_SAVE_SCHEMA_VERSION_V1,
            network_protocol_version: crate::CURRENT_RANKED_NETWORK_PROTOCOL_VERSION_V1,
            verifier: artifact_with_media(12, RANKED_REPLAY_VERIFIER_MEDIA_TYPE_V2),
            viewer_artifacts: build.viewer.engine.artifacts.clone(),
        };
        assert_eq!(projected, expected);
        assert_eq!(projected.target_triple, "wasm32-unknown-unknown");
        assert_eq!(projected.cargo_profile, "wasm-release");
        assert_eq!(projected.cargo_features, ["audio"]);
        let mut different_replay_schema = projected.clone();
        different_replay_schema.replay_schema_version += 1;
        assert_ne!(
            projected.canonical_digest().unwrap(),
            different_replay_schema.canonical_digest().unwrap(),
            "the public build digest must bind the shared replay schema constant"
        );

        let decoded: VersionedBuildManifest = serde_json::from_slice(&canonical).unwrap();
        assert!(matches!(decoded, VersionedBuildManifest::V2(_)));
        assert_eq!(decoded.backend_visible_v1().unwrap(), projected);

        // A public body alone cannot mint a receipt: only the separately
        // supplied private authority owns an exporter recipe and artifact.
        let authority = projection_authority_v2(&build);
        authority.validate_against(&build).unwrap();
        let mut wrong_public = build.clone();
        wrong_public.viewer.pages_shell.package_json_sha256 = Digest32::from_bytes([99; 32]);
        assert!(authority.validate_against(&wrong_public).is_err());
    }

    #[test]
    fn build_v2_rejects_cross_role_artifacts_and_wrong_fixed_recipes() {
        let build = projection_build_v2();
        let authority = projection_authority_v2(&build);

        let mut verifier_as_viewer = build.clone();
        verifier_as_viewer.viewer.engine.artifacts[0].artifact = build.verifier.artifact.clone();
        assert!(verifier_as_viewer.validate().is_err());

        let mut viewer_as_verifier = build.clone();
        viewer_as_verifier.verifier.artifact = build.viewer.engine.artifacts[1].artifact.clone();
        assert!(viewer_as_verifier.validate().is_err());

        let mut verifier_wrong_target = build.clone();
        verifier_wrong_target.verifier.target_triple = "x86_64-unknown-linux-gnu".into();
        assert!(verifier_wrong_target.validate().is_err());

        let mut verifier_wrong_binary = build.clone();
        verifier_wrong_binary.verifier.cargo_binary = "robin".into();
        assert!(verifier_wrong_binary.validate().is_err());

        let mut viewer_wrong_features = build.clone();
        viewer_wrong_features.viewer.engine.cargo_features = vec![];
        assert!(viewer_wrong_features.validate().is_err());

        let mut viewer_wrong_package = build.clone();
        viewer_wrong_package.viewer.engine.cargo_package = "robin_replay_viewer".into();
        assert!(viewer_wrong_package.validate().is_err());

        let mut viewer_wrong_toolchain = build.clone();
        viewer_wrong_toolchain.viewer.engine.rust_toolchain.channel = "stable".into();
        assert!(viewer_wrong_toolchain.validate().is_err());

        let mut viewer_moving_node_channel = build.clone();
        viewer_moving_node_channel.viewer.pages_shell.node.version = "24.19".into();
        assert!(viewer_moving_node_channel.validate().is_err());

        let mut signer_as_engine = build.clone();
        signer_as_engine
            .viewer
            .identity_signer
            .identity_signer_origin_artifacts[2]
            .artifact = build.viewer.engine.artifacts[1].artifact.clone();
        assert!(signer_as_engine.validate().is_err());

        let mut signer_wrong_binary = build.clone();
        signer_wrong_binary.viewer.identity_signer.cargo_binary = "robin".into();
        assert!(signer_wrong_binary.validate().is_err());

        let mut signer_wrong_wasm_bindgen = build.clone();
        signer_wrong_wasm_bindgen
            .viewer
            .identity_signer
            .wasm_bindgen_cli
            .authority_sha256 = Digest32::from_bytes([88; 32]);
        assert!(signer_wrong_wasm_bindgen.validate().is_err());

        let mut exporter_as_verifier = authority.clone();
        exporter_as_verifier.projection_exporter.artifact = build.verifier.artifact.clone();
        assert!(exporter_as_verifier.validate().is_err());

        let mut exporter_wrong_example = authority.clone();
        exporter_wrong_example.projection_exporter.cargo_example = "batch_run".into();
        assert!(exporter_wrong_example.validate().is_err());

        let mut exporter_wrong_features = authority;
        exporter_wrong_features
            .projection_exporter
            .cargo_features
            .push("audio".into());
        assert!(exporter_wrong_features.validate().is_err());
    }

    #[test]
    fn viewer_build_report_is_exactly_cross_bound_per_origin() {
        let build = projection_build_v2();
        let report = OfficialViewerBuildReportV2::from_public_build(&build).unwrap();
        report.validate_against(&build).unwrap();
        let canonical = report.canonical_bytes().unwrap();
        let decoded: OfficialViewerBuildReportV2 = serde_json::from_slice(&canonical).unwrap();
        assert_eq!(decoded, report);

        let engine_inventory = OfficialViewerOriginArtifactInventoryV2::Engine {
            schema_version: OFFICIAL_VIEWER_BUILD_REPORT_SCHEMA_VERSION_V2,
            source_commit: build.source_commit.clone(),
            cargo_lock_sha256: build.cargo_lock_sha256,
            artifacts: report.engine_artifacts.clone(),
        };
        engine_inventory.validate().unwrap();
        let inventory_bytes = engine_inventory.canonical_bytes().unwrap();
        let inventory_decoded: OfficialViewerOriginArtifactInventoryV2 =
            serde_json::from_slice(&inventory_bytes).unwrap();
        assert_eq!(inventory_decoded, engine_inventory);

        let mut swapped_engine = report.clone();
        swapped_engine.engine_artifacts[0].artifact.sha256 =
            swapped_engine.pages_shell_artifacts[0].artifact.sha256;
        assert!(swapped_engine.validate_against(&build).is_err());

        let mut swapped_origin = report.clone();
        swapped_origin.identity_signer_artifacts[0].artifact =
            swapped_origin.pages_shell_artifacts[0].artifact.clone();
        assert!(swapped_origin.validate_against(&build).is_err());

        let mut moving_tool = report;
        moving_tool.binaryen_wasm_opt.version = "latest".into();
        assert!(moving_tool.validate_against(&build).is_err());
    }

    #[test]
    fn projection_export_report_v2_is_typed_canonical_and_cross_bound() {
        let build = projection_build_v2();
        let authority = projection_authority_v2(&build);
        let rules = official_rules_config();
        let overlay = core_overlay_v2();
        let (source, receipt) = projection_receipt_v2(
            OfficialContentEditionV1::Demo,
            OfficialProjectionSourceFormatV1::LooseNativeV1,
            &authority,
            &rules,
            &overlay,
        );
        let report = projection_export_report_v2(&receipt, &build);
        report
            .validate_against(&receipt, &build, &authority, &rules, &source, &overlay)
            .unwrap();
        let canonical = report.canonical_bytes().unwrap();
        let decoded: OfficialProjectionExportReportV2 = serde_json::from_slice(&canonical).unwrap();
        assert_eq!(decoded, report);

        let mut swapped = report.clone();
        swapped.projection_receipt_sha256 = Digest32::from_bytes([95; 32]);
        assert!(
            swapped
                .validate_against(&receipt, &build, &authority, &rules, &source, &overlay,)
                .is_err()
        );

        let mut unsafe_path = report;
        unsafe_path.catalog_root = "/output/../escaped".into();
        assert!(unsafe_path.validate().is_err());
    }

    #[test]
    fn projection_v2_rejects_v1_builds_wrong_closures_and_case_collisions() {
        let historical: VersionedBuildManifest = serde_json::from_slice(
            &BuildManifestV1 {
                schema_version: 1,
                source_commit: "a".repeat(40),
                cargo_lock_sha256: Digest32::from_bytes([1; 32]),
                target_triple: "x86_64-unknown-linux-gnu".into(),
                cargo_profile: "release".into(),
                cargo_features: vec![],
                replay_schema_version: crate::CURRENT_RANKED_REPLAY_SCHEMA_VERSION_V1,
                save_schema_version: 60,
                network_protocol_version: 28,
                verifier: artifact(2),
                viewer_artifacts: vec![
                    NamedArtifactV1 {
                        path: "viewer/entry.js".into(),
                        role: ViewerArtifactRoleV1::EntryJavaScript,
                        artifact: artifact_with_media(3, "text/javascript"),
                    },
                    NamedArtifactV1 {
                        path: "viewer/robin.wasm".into(),
                        role: ViewerArtifactRoleV1::WebAssembly,
                        artifact: artifact_with_media(4, "application/wasm"),
                    },
                ],
            }
            .canonical_bytes()
            .unwrap(),
        )
        .unwrap();
        assert!(matches!(historical, VersionedBuildManifest::V1(_)));
        assert!(historical.require_public_v2().is_err());

        let mut wrong_pair = source_tree_v2(
            OfficialContentEditionV1::Demo,
            OfficialProjectionSourceFormatV1::LooseNativeV1,
        );
        wrong_pair.closure_kind =
            OfficialSourceClosureKindV2::ShippingDatadirV10ArchiveAndReferencedSplitsV1;
        assert!(wrong_pair.validate().is_err());

        let mut collision = source_tree_v2(
            OfficialContentEditionV1::Demo,
            OfficialProjectionSourceFormatV1::LooseNativeV1,
        );
        collision.files = vec![
            source_file("Data/Levels/A.rhm", 40),
            source_file("Data/Levels/a.rhm", 41),
        ];
        assert!(collision.validate().is_err());

        let mut authentic_demo_casing = source_tree_v2(
            OfficialContentEditionV1::Demo,
            OfficialProjectionSourceFormatV1::LooseNativeV1,
        );
        authentic_demo_casing.files = vec![
            source_file("1033/data/Text/Level.res", 42),
            source_file("DATA/Configuration/profile.cpf", 43),
        ];
        authentic_demo_casing
            .files
            .sort_by(|left, right| left.path.cmp(&right.path));
        authentic_demo_casing.validate().unwrap();

        for forbidden in [
            "DATA/Savegame/slot.sav",
            "Data/sblibng.log",
            "Data/Levels/.codex-tmp/debug.rhm",
            "Data/Levels/replay.jsonl",
            "1033/data/Text/CaMpAiGn.BcK",
            "1033/data/CaChE/audio_durations.json",
            "1033/data/Text/nested/SaVeGaMe/slot.sav",
        ] {
            let mut mutable = authentic_demo_casing.clone();
            mutable.files.push(source_file(forbidden, 44));
            mutable
                .files
                .sort_by(|left, right| left.path.cmp(&right.path));
            assert!(mutable.validate().is_err(), "accepted {forbidden}");
        }

        let mut wrong_platform = projection_build_v2();
        wrong_platform.verifier.target_triple = "wasm32-unknown-emscripten".into();
        assert!(wrong_platform.validate().is_err());

        let mut wrong_features = projection_build_v2();
        wrong_features
            .verifier
            .cargo_features
            .push("native-fs".into());
        assert!(wrong_features.validate().is_err());

        let mut wrong_profile = projection_build_v2();
        wrong_profile.viewer.engine.cargo_profile = "debug".into();
        assert!(wrong_profile.validate().is_err());

        let mut wrong_seed = official_execution_policy();
        wrong_seed.simulation_seed = crate::SimulationSeed64::new(1);
        assert!(wrong_seed.validate().is_err());
    }

    #[test]
    fn projection_receipt_matrix_requires_four_equal_authority_lanes() {
        let build = projection_build_v2();
        let authority = projection_authority_v2(&build);
        let rules = official_rules_config();
        let overlay = core_overlay_v2();
        let mut receipts = Vec::new();
        for edition in [
            OfficialContentEditionV1::Demo,
            OfficialContentEditionV1::Full,
        ] {
            for format in [
                OfficialProjectionSourceFormatV1::LooseNativeV1,
                OfficialProjectionSourceFormatV1::ShippingDatadirV10,
            ] {
                receipts
                    .push(projection_receipt_v2(edition, format, &authority, &rules, &overlay).1);
            }
        }
        validate_official_projection_receipt_matrix_v2(&receipts).unwrap();
        receipts[1].subjects[0].content_manifest.components[0]
            .artifact
            .sha256 = Digest32::from_bytes([96; 32]);
        assert!(validate_official_projection_receipt_matrix_v2(&receipts).is_err());
    }

    #[test]
    fn content_manifest_requires_complete_canonical_projection() {
        let mut manifest = ContentManifestV1 {
            schema_version: 1,
            name: "leicester-demo".into(),
            edition: OfficialContentEditionV1::Demo,
            subject: OfficialContentSubjectV1::FieldMission {
                mission_id: "Dem_Lei_MP".into(),
            },
            closure: ContentClosureKindV1::StaticPreparedMissionContentProjection,
            projection_schema_version: 2,
            resource_locale_root: ResourceLocaleRootV1::new("1033").unwrap(),
            speech_timing: SimulationSpeechTimingSourceV1::LanguagePack {
                canonical_locale: "en-US".into(),
            },
            components: vec![],
        };
        assert!(manifest.validate().is_err());
        manifest.components = simulation_components();
        manifest.components.swap(0, 1);
        assert!(manifest.validate().is_err());
        manifest.components = simulation_components();
        let manifest_sha256 = manifest.canonical_digest().unwrap();
        let object_path =
            demo_content_object_path_v1(manifest_sha256, &manifest.components[0]).unwrap();
        assert_eq!(
            object_path,
            format!(
                "content/{manifest_sha256}/objects/{}",
                manifest.components[0].artifact.sha256
            )
        );
    }

    #[test]
    fn resource_locale_root_is_one_unambiguous_numeric_component() {
        assert_eq!(ResourceLocaleRootV1::new("1033").unwrap().as_str(), "1033");
        assert_eq!(ResourceLocaleRootV1::new("2047").unwrap().as_str(), "2047");
        for invalid in [
            "",
            "0",
            "01033",
            ".",
            "..",
            "en-US",
            "1033/data",
            "1033\\data",
        ] {
            assert!(
                ResourceLocaleRootV1::new(invalid).is_err(),
                "accepted invalid locale component {invalid:?}"
            );
        }
    }

    #[test]
    fn simulation_component_paths_are_safe_and_headquarters_is_canonical() {
        let field = OfficialContentSubjectV1::FieldMission {
            mission_id: "Dem Lei/ä".into(),
        };
        assert_eq!(
            simulation_content_component_relative_path_v1(
                &field,
                SimulationContentComponentKindV1::LoadedLevel,
            )
            .unwrap(),
            "field-missions/44656d204c65692fc3a4/loaded_level.bitcode"
        );
        let first_hq = OfficialContentSubjectV1::Headquarters {
            mission_id: "Sherwood_A".into(),
        };
        let second_hq = OfficialContentSubjectV1::Headquarters {
            mission_id: "Sherwood_B".into(),
        };
        assert_eq!(
            simulation_content_component_relative_path_v1(
                &first_hq,
                SimulationContentComponentKindV1::Profiles,
            )
            .unwrap(),
            simulation_content_component_relative_path_v1(
                &second_hq,
                SimulationContentComponentKindV1::Profiles,
            )
            .unwrap()
        );
    }

    #[test]
    fn official_subject_matrix_rejects_cross_edition_substitution() {
        let demo = official_content_subjects_v1(OfficialContentEditionV1::Demo);
        assert_eq!(
            demo,
            vec![OfficialContentSubjectV1::FieldMission {
                mission_id: "Dem_Lei_MP".into()
            }]
        );
        assert!(
            validate_official_content_subjects_v1(OfficialContentEditionV1::Demo, &demo).is_ok()
        );

        let full = official_content_subjects_v1(OfficialContentEditionV1::Full);
        assert_eq!(full.len(), 39);
        assert!(full.windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(
            full.last(),
            Some(&OfficialContentSubjectV1::Headquarters {
                mission_id: "Sherwood".into()
            })
        );
        assert!(
            validate_official_content_subjects_v1(OfficialContentEditionV1::Full, &full).is_ok()
        );

        let mut hybrid_demo = demo;
        hybrid_demo.push(OfficialContentSubjectV1::Headquarters {
            mission_id: OFFICIAL_FULL_HEADQUARTERS_MISSION_ID_V1.into(),
        });
        assert!(
            validate_official_content_subjects_v1(OfficialContentEditionV1::Demo, &hybrid_demo,)
                .is_err()
        );
    }

    #[test]
    fn projection_receipt_binds_raw_tree_and_exact_content_manifests() {
        let source_tree = OfficialSourceTreeManifestV1 {
            schema_version: 1,
            edition: OfficialContentEditionV1::Demo,
            source_format: OfficialProjectionSourceFormatV1::LooseNativeV1,
            files: vec![OfficialSourceFileV1 {
                path: "Data/Levels/Dem_Lei_MP.RHM".into(),
                sha256: Digest32::from_bytes([31; 32]),
                byte_length: 0,
            }],
        };
        assert!(source_tree.validate().is_ok());
        let subject = official_content_subjects_v1(OfficialContentEditionV1::Demo)
            .into_iter()
            .next()
            .unwrap();
        let content_manifest = ContentManifestV1 {
            schema_version: 1,
            name: official_content_manifest_name_v1(OfficialContentEditionV1::Demo, &subject),
            edition: OfficialContentEditionV1::Demo,
            subject,
            closure: ContentClosureKindV1::StaticPreparedMissionContentProjection,
            projection_schema_version: 2,
            resource_locale_root: ResourceLocaleRootV1::new("1033").unwrap(),
            speech_timing: SimulationSpeechTimingSourceV1::BaseInstallation,
            components: simulation_components(),
        };
        let receipt = OfficialSimulationProjectionReceiptV1 {
            schema_version: 1,
            exporter: OfficialProjectionExporterIdentityV1 {
                exporter_version: 1,
                source_format: OfficialProjectionSourceFormatV1::LooseNativeV1,
            },
            edition: OfficialContentEditionV1::Demo,
            source_tree_manifest_sha256: source_tree.canonical_digest().unwrap(),
            source_file_count: 1,
            subjects: vec![OfficialProjectionSubjectReceiptV1 { content_manifest }],
        };
        assert!(receipt.validate().is_ok());

        let mut wrong_name = receipt.clone();
        wrong_name.subjects[0].content_manifest.name = "operator guess".into();
        assert!(wrong_name.validate().is_err());
        let mut hybrid = receipt;
        hybrid.subjects[0].content_manifest.subject = OfficialContentSubjectV1::Headquarters {
            mission_id: OFFICIAL_FULL_HEADQUARTERS_MISSION_ID_V1.into(),
        };
        assert!(hybrid.validate().is_err());

        let mut repeated_file = source_tree;
        repeated_file.files.push(repeated_file.files[0].clone());
        assert!(repeated_file.validate().is_err());
    }

    #[test]
    fn campaign_content_catalog_is_subject_sorted_and_stable_across_hq_repeats() {
        let mission = ContentManifestV1 {
            schema_version: 1,
            name: "nottingham-full".into(),
            edition: OfficialContentEditionV1::Full,
            subject: OfficialContentSubjectV1::FieldMission {
                mission_id: "S01_Not_MP".into(),
            },
            closure: ContentClosureKindV1::StaticPreparedMissionContentProjection,
            projection_schema_version: 2,
            resource_locale_root: ResourceLocaleRootV1::new("2047").unwrap(),
            speech_timing: SimulationSpeechTimingSourceV1::BaseInstallation,
            components: simulation_components(),
        };
        let mut catalog = CampaignContentManifestV1 {
            schema_version: 1,
            edition: OfficialContentEditionV1::Full,
            entries: vec![
                CampaignContentEntryV1 {
                    subject: OfficialContentSubjectV1::FieldMission {
                        mission_id: "S01_Not_MP".into(),
                    },
                    content_manifest_sha256: mission.canonical_digest().unwrap(),
                },
                CampaignContentEntryV1 {
                    subject: OfficialContentSubjectV1::Headquarters {
                        mission_id: "sherwood".into(),
                    },
                    content_manifest_sha256: Digest32::from_bytes([42; 32]),
                },
            ],
        };
        assert!(catalog.validate().is_ok());
        assert_eq!(
            catalog.content_for(&OfficialContentSubjectV1::Headquarters {
                mission_id: "sherwood".into(),
            }),
            Some(Digest32::from_bytes([42; 32]))
        );
        catalog.entries.reverse();
        assert!(catalog.validate().is_err());
    }

    #[test]
    fn rules_config_digest_ignores_map_insertion_order() {
        let mut left = BTreeMap::new();
        left.insert("scripts".into(), CanonicalValue::Bool(true));
        left.insert("difficulty".into(), CanonicalValue::String("Hard".into()));
        let mut right = BTreeMap::new();
        right.insert("difficulty".into(), CanonicalValue::String("Hard".into()));
        right.insert("scripts".into(), CanonicalValue::Bool(true));
        let rules = BTreeMap::from([("loads".into(), CanonicalValue::Bool(false))]);
        let a = RulesConfigIdentityV1 {
            schema_version: 1,
            replay_schema_version: crate::CURRENT_RANKED_REPLAY_SCHEMA_VERSION_V1,
            ranked_simulation_policy: RankedSimulationPolicyV1::standard(
                RankedSimulationDifficultyV1::Hard,
            ),
            sim_config: left,
            rules: rules.clone(),
        };
        let b = RulesConfigIdentityV1 {
            sim_config: right,
            ..a.clone()
        };
        assert_eq!(a.canonical_bytes().unwrap(), b.canonical_bytes().unwrap());
        assert_eq!(a.canonical_digest().unwrap(), b.canonical_digest().unwrap());
    }

    #[test]
    fn ruleset_manifest_and_rules_config_have_distinct_content_addresses() {
        let rules_config = RulesConfigIdentityV1 {
            schema_version: 1,
            replay_schema_version: crate::CURRENT_RANKED_REPLAY_SCHEMA_VERSION_V1,
            ranked_simulation_policy: RankedSimulationPolicyV1::standard(
                RankedSimulationDifficultyV1::Medium,
            ),
            sim_config: BTreeMap::from([(
                "difficulty".into(),
                CanonicalValue::String("Medium".into()),
            )]),
            rules: BTreeMap::from([("state_load".into(), CanonicalValue::Bool(false))]),
        };
        let rules_config_sha256 = rules_config.canonical_digest().unwrap();
        let ruleset = ruleset_manifest(rules_config_sha256);
        assert!(ruleset.validate().is_ok());
        assert_ne!(ruleset.canonical_digest().unwrap(), rules_config_sha256);

        let mut noncanonical = ruleset;
        noncanonical.board_scopes.swap(0, 1);
        assert!(noncanonical.canonical_digest().is_err());
    }

    #[test]
    fn ruleset_labels_are_not_free_aliases_for_ranked_simulation_policy() {
        let config = official_rules_config();
        let mut ruleset = ruleset_manifest(config.canonical_digest().unwrap());
        ruleset
            .validate_ranked_simulation_policy(&config)
            .expect("Standard/Normal labels match the typed policy");

        ruleset.difficulty_id = OpaqueId::new("easy").unwrap();
        ruleset.difficulty_name = "Easy".into();
        assert!(ruleset.validate_ranked_simulation_policy(&config).is_err());

        let mut original = config;
        original.ranked_simulation_policy =
            RankedSimulationPolicyV1::original_parity(RankedSimulationDifficultyV1::Medium);
        ruleset = ruleset_manifest(original.canonical_digest().unwrap());
        assert!(
            ruleset
                .validate_ranked_simulation_policy(&original)
                .is_err()
        );
    }

    #[test]
    fn ranked_simulation_policy_schema_and_difficulty_fail_closed() {
        let mut config = official_rules_config();
        config.ranked_simulation_policy.version += 1;
        assert!(config.validate().is_err());

        let json = serde_json::json!({
            "version": 1,
            "preset": "standard",
            "difficulty": "legendary"
        });
        assert!(serde_json::from_value::<RankedSimulationPolicyV1>(json).is_err());

        let mut hard_as_medium = official_rules_config();
        hard_as_medium
            .sim_config
            .insert("difficulty".into(), CanonicalValue::String("Hard".into()));
        assert!(hard_as_medium.validate().is_err());
    }

    #[test]
    fn immutable_policy_document_is_exactly_content_addressed() {
        let mut policy = ImmutablePolicyManifestV1 {
            schema_version: 1,
            kind: ImmutablePolicyKindV1::CommandAdmission,
            version: 3,
            rules: BTreeMap::from([(
                "ui_command_sources".into(),
                CanonicalValue::Array(vec![CanonicalValue::String("native_ui".into())]),
            )]),
        };
        let first = policy.canonical_digest().unwrap();
        policy.version += 1;
        assert_ne!(first, policy.canonical_digest().unwrap());
        policy.rules.clear();
        assert!(policy.canonical_digest().is_err());
    }

    #[test]
    fn every_ruleset_field_participates_in_canonical_identity() {
        let manifest = ruleset_manifest(Digest32::from_bytes([7; 32]));
        let baseline = manifest.canonical_digest().unwrap();
        let value = serde_json::to_value(&manifest).unwrap();
        let object = value.as_object().unwrap();
        assert!(object.len() >= 34, "new policy fields must stay explicit");

        // Removing any direct policy dimension changes the canonical digest
        // and can no longer deserialize as a RulesetManifestV1. This covers
        // fixed one-variant policies as well as fields with valid alternatives.
        for field in object.keys() {
            let mut removed = value.clone();
            removed.as_object_mut().unwrap().remove(field);
            let removed_digest = Digest32::digest_bytes(
                crate::canonical_json_bytes(&removed).expect("JSON remains canonicalizable"),
            );
            assert_ne!(baseline, removed_digest, "field {field} was not committed");
            assert!(
                serde_json::from_value::<RulesetManifestV1>(removed).is_err(),
                "field {field} unexpectedly became optional"
            );
        }

        let mut mutations: Vec<Box<dyn Fn(&mut RulesetManifestV1)>> = vec![
            Box::new(|m| m.display_name.push_str(" v2")),
            Box::new(|m| m.preset_id = OpaqueId::new("original").unwrap()),
            Box::new(|m| m.preset_name = "Original".into()),
            Box::new(|m| m.difficulty_id = OpaqueId::new("hard").unwrap()),
            Box::new(|m| m.difficulty_name = "Hard".into()),
            Box::new(|m| {
                m.rules_config_sha256 = Digest32::from_bytes([20; 32]);
                m.canonical_campaign_state.rules_config_sha256 = m.rules_config_sha256;
            }),
            Box::new(|m| {
                m.canonical_campaign_state.edition = OfficialContentEditionV1::Demo;
                m.canonical_campaign_state.kind = CanonicalCampaignStateKindV1::IndividualTemplate;
            }),
            Box::new(|m| {
                m.allowed_build_manifest_sha256
                    .push(Digest32::from_bytes([20; 32]))
            }),
            Box::new(|m| {
                m.allowed_content_manifest_sha256
                    .push(Digest32::from_bytes([20; 32]))
            }),
            Box::new(|m| {
                m.allowed_campaign_content_manifest_sha256[0] = Digest32::from_bytes([20; 32])
            }),
            Box::new(|m| {
                m.board_scopes.pop();
                m.allowed_campaign_content_manifest_sha256.clear();
                m.campaign_completion_policy = CampaignCompletionPolicyRequirementV1::NotOffered;
            }),
            Box::new(|m| {
                m.campaign_completion_policy =
                    CampaignCompletionPolicyRequirementV1::Required(CampaignCompletionPolicyV1 {
                        required_progression_percent: 99,
                        ..official_full_campaign_completion_policy_v1()
                    })
            }),
            Box::new(|m| {
                m.metrics.pop();
                m.metric_ranking.pop();
            }),
            Box::new(|m| m.achievement_policies[0].mode = AchievementPolicyModeV1::Reported),
            Box::new(|m| {
                m.participant_eligibility.anonymous_policy = AnonymousParticipantPolicyV1::Forbidden
            }),
            Box::new(|m| {
                m.campaign_roster_continuity =
                    CampaignRosterContinuityV1::ExactSameAuthenticatedKeysEverySession
            }),
            Box::new(|m| m.participant_eligibility.maximum_participant_instances -= 1),
            Box::new(|m| {
                m.network_protocol_versions
                    .push(crate::CURRENT_RANKED_NETWORK_PROTOCOL_VERSION_V1 + 1)
            }),
            Box::new(|m| m.input_provenance_policy.version += 1),
            Box::new(|m| m.command_admission_policy.version += 1),
            Box::new(|m| m.submission_admission_policy.version += 1),
            Box::new(|m| m.verifier_policy.version += 1),
            Box::new(|m| m.tick_duration.numerator_micros += 1),
            Box::new(|m| m.main_board_seed_policy = RulesetSeedPolicyV1::ServerPinned),
            Box::new(|m| m.competition_seed_policy = RulesetSeedPolicyV1::Open),
            Box::new(|m| m.allow_save_creation = !m.allow_save_creation),
            Box::new(|m| m.allow_autosave = !m.allow_autosave),
            Box::new(|m| m.allow_state_load = !m.allow_state_load),
            Box::new(|m| m.allow_mission_restart = !m.allow_mission_restart),
        ];
        for mutate in mutations.drain(..) {
            let mut changed = manifest.clone();
            mutate(&mut changed);
            assert!(changed.validate().is_ok());
            assert_ne!(baseline, changed.canonical_digest().unwrap());
        }
    }

    #[test]
    fn full_campaign_rulesets_require_a_catalog_bound_completion_policy() {
        let mut ruleset = ruleset_manifest(Digest32::from_bytes([7; 32]));
        assert_eq!(
            ruleset.campaign_completion_policy,
            CampaignCompletionPolicyRequirementV1::Required(
                official_full_campaign_completion_policy_v1()
            )
        );

        let mut missing_policy = ruleset.clone();
        missing_policy.campaign_completion_policy =
            CampaignCompletionPolicyRequirementV1::NotOffered;
        assert!(missing_policy.validate().is_err());

        let mut forbidden_policy = ruleset.clone();
        forbidden_policy.board_scopes.pop();
        forbidden_policy
            .allowed_campaign_content_manifest_sha256
            .clear();
        assert!(forbidden_policy.validate().is_err());

        let terminal = ruleset
            .campaign_completion_policy
            .required()
            .unwrap()
            .terminal_subject
            .clone();
        let mut catalog = CampaignContentManifestV1 {
            schema_version: 1,
            edition: OfficialContentEditionV1::Full,
            entries: vec![CampaignContentEntryV1 {
                subject: terminal,
                content_manifest_sha256: Digest32::from_bytes([44; 32]),
            }],
        };
        assert!(
            ruleset
                .validate_campaign_completion_catalog(&catalog)
                .is_ok()
        );
        catalog.entries[0].subject = OfficialContentSubjectV1::FieldMission {
            mission_id: "H11_Not_MP".into(),
        };
        assert!(
            ruleset
                .validate_campaign_completion_catalog(&catalog)
                .is_err()
        );

        ruleset.campaign_completion_policy =
            CampaignCompletionPolicyRequirementV1::Required(CampaignCompletionPolicyV1 {
                required_progression_percent: 0,
                ..official_full_campaign_completion_policy_v1()
            });
        assert!(ruleset.validate().is_err());
    }

    #[test]
    fn achievement_catalog_is_canonical_and_validates_complete_results() {
        let mut manifest = ruleset_manifest(Digest32::from_bytes([7; 32]));
        let expected_ids = [
            "all-enemies-stashed",
            "clean-hands",
            "ghost",
            "pile-o-bones",
        ];
        assert_eq!(
            manifest
                .achievement_policies
                .iter()
                .map(|policy| policy.achievement_id.as_str())
                .collect::<Vec<_>>(),
            expected_ids
        );
        assert!(
            manifest
                .achievement_policies
                .iter()
                .all(|policy| policy.mode == AchievementPolicyModeV1::Required)
        );
        assert!(manifest.validate().is_ok());

        let mut results = manifest
            .achievement_policies
            .iter()
            .map(|policy| VerifiedAchievementV1 {
                achievement_id: policy.achievement_id.clone(),
                evaluation: VerifiedAchievementEvaluationV1::NotEarned,
                evidence: BTreeMap::new(),
            })
            .collect::<Vec<_>>();
        assert!(
            manifest
                .validate_authoritative_achievements(&results)
                .is_ok()
        );

        results[0].evaluation = VerifiedAchievementEvaluationV1::Unverifiable;
        assert!(
            manifest
                .validate_authoritative_achievements(&results)
                .is_err()
        );
        manifest.achievement_policies[0].mode = AchievementPolicyModeV1::Reported;
        assert!(
            manifest
                .validate_authoritative_achievements(&results)
                .is_ok()
        );

        results.pop();
        assert!(
            manifest
                .validate_authoritative_achievements(&results)
                .is_err()
        );

        let mut out_of_order = manifest.clone();
        out_of_order.achievement_policies.swap(0, 1);
        assert!(out_of_order.validate().is_err());
        let mut duplicate = manifest;
        duplicate.achievement_policies[1].achievement_id =
            duplicate.achievement_policies[0].achievement_id.clone();
        assert!(duplicate.validate().is_err());
    }

    #[test]
    fn quarantine_can_only_deny_an_unchanged_ruleset() {
        let manifest = ruleset_manifest(Digest32::from_bytes([7; 32]));
        let ruleset_manifest_sha256 = manifest.canonical_digest().unwrap();
        let published = PublishedRulesetV1 {
            schema_version: 1,
            ruleset_manifest_sha256,
            manifest,
            operational_status: RulesetOperationalStatusV1::Quarantined {
                audit_id: OpaqueId::new("security-incident-1").unwrap(),
                reason_code: "verifier_quarantined".into(),
                since_unix_ms: 1,
            },
        };
        assert!(published.validate().is_ok());
        let mut substituted = published;
        substituted.manifest.allow_state_load = true;
        assert!(substituted.validate().is_err());
    }
}
