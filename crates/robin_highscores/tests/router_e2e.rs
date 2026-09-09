use axum::Router;
use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE, X_CONTENT_TYPE_OPTIONS};
use axum::http::{Method, Request, StatusCode};
use ed25519_dalek::{Signer as _, SigningKey};
use http_body_util::BodyExt as _;
use robin_highscores::config::{
    AdmissionProfile, CompetitionConfig, LoadedBuildManifest, ManifestRegistry,
};
use robin_highscores::verifier::build_verification_request;
use robin_highscores::web::{AppState, ChallengeRateLimiter, router};
use robin_highscores::{CampaignStore, Database, ReplayStore, ServerConfig};
use robin_run_protocol::{
    AbuseReportAcceptedV1, AbuseReportCategoryV1, AbuseReportTargetV1, AbuseReportV1,
    ActiveTimeDefinitionV1, AnonymousParticipantPolicyV1, ArtifactRefV1, BoardCategoryV1,
    BoardMetricV1, BuildManifestV1, CampaignAggregationConsentPolicyV1,
    CampaignAggregationConsentV1, CampaignChainReceiptV1, CampaignChainStateV1,
    CampaignCompleteEvidenceV1, CampaignCompletionPolicyRequirementV1, CampaignContentEntryV1,
    CampaignContentManifestV1, CampaignContinuationAuthorizationClaimV1,
    CampaignContinuationAuthorizationV1, CampaignContinuationPreflightGrantV1,
    CampaignContinuationPreflightRequestClaimV1, CampaignContinuationPreflightRequestV1,
    CampaignRosterContinuityV1, CampaignSessionDetailV1, CampaignSessionKindV1,
    CanonicalCampaignStateKindV1, CanonicalCampaignStatePinV1, CanonicalCampaignStateRequirementV1,
    CanonicalDocument as _, CanonicalStartPolicyV1, CanonicalValue, ChallengeNonce32,
    CompetitionManifestV1, CompetitionParticipantCompositionV1, CompetitionRunGrantRequestClaimV1,
    CompetitionRunGrantRequestV1, CompetitionRunGrantV1, CompetitionSeedPolicyV1,
    CompetitionStateV1, ContentClosureKindV1, ContentManifestV1, DeletionChallengeRequestV1,
    DeletionChallengeV1, DeletionReceiptV1, DeletionRequestEnvelopeV1, DeletionTargetV1, Digest32,
    FrameCountingPolicyV1, FreshRunPreflightGrantV1, FreshRunPreflightRequestClaimV1,
    FreshRunPreflightRequestV1, FreshRunScopeV1, FullCampaignChainPolicyV1,
    FullCampaignTimeAggregationV1, ImmutablePolicyIdentityV1, ImmutablePolicyKindV1,
    InitialStateExpectationV1, InputProvenanceEligibilityV1, InputProvenanceStatusV1,
    LeaderboardMetadataV1, LeaderboardPageV1, LeaderboardSubjectV1, MetricRankingPolicyV1,
    NamedArtifactV1, NamedParticipantPolicyV1, OFFICIAL_FULL_CAMPAIGN_GENESIS_MISSION_ID_V1,
    OfficialContentEditionV1, OfficialContentSubjectV1, OpaqueId, PaginationTieBreakV1,
    ParticipantClaimV1, ParticipantEligibilityV1, ParticipantPublicDisclosureV1,
    ParticipantSignatureV1, PlayerProfileV1, PlayerRunHistoryPageV1, PublicKey32,
    RANKED_CAMPAIGN_MEDIA_TYPE_V1, RANKED_REPLAY_MEDIA_TYPE_V1, ReplayArtifactV1,
    ReplaySeatLifecycleEventV1, ReplaySeatLifecycleKindV1, ReplaySessionGenesisClaimV1,
    ReplaySessionGenesisV1, ReplaySessionTranscriptV1, ResourceLocaleRootV1,
    RulesConfigConstraintV1, RulesConfigIdentityV1, RulesetBoardScopeV1, RulesetManifestV1,
    RulesetOperationalStatusV1, RulesetSeedPolicyV1, RunCompositionPolicyV1, RunContentIdentityV1,
    RunDetailV1, RunScopeKindV1, SCHEMA_VERSION_V1, SIMULATION_CONTENT_COMPONENT_MEDIA_TYPE_V1,
    ScopeRequestV1, ScoreAlgorithmV1, ScoreOverflowPolicyV1, Signature64, SignatureAlgorithmV1,
    SignedSubmissionV1, SimulationContentComponentKindV1, SimulationContentComponentV1,
    SimulationSeed64, SimulationSpeechTimingSourceV1, SpeechTimingAuthorityV1,
    SubmissionAcceptedV1, SubmissionArtifactsV1, SubmissionEnvelopeV1, SubmissionLifecycleV1,
    SubmissionOfferRequestV1, SubmissionOfferV1, SubmissionOwnerStatusChallengeRequestV1,
    SubmissionOwnerStatusChallengeV1, SubmissionOwnerStatusEnvelopeV1,
    SubmissionOwnerStatusResponseV1, TerminalOutcomeV1, TerminalResultPolicyV1, TickDurationV1,
    UsernameChallengeRequestV1, UsernameChallengeV1, UsernameUpdateEnvelopeV1, Validate as _,
    VerificationLimitsV1, VerificationResultV1, VerificationStatusV1,
    VerifiedAchievementEvaluationV1, VerifiedAchievementV1, VerifiedRunV1, ViewerArtifactRoleV1,
    VisibleTiePolicyV1, official_achievement_policies_v1,
    official_full_campaign_completion_policy_v1,
};
use serde::Serialize;
use serde::de::DeserializeOwned;
use sqlx::Row as _;
use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::AsyncReadExt as _;
use tower::ServiceExt as _;

const MISSION_ID: &str = "Dem_Lei_MP";
const CAMPAIGN_MISSION_ID: &str = "H02_Not_EC";
const HQ_MISSION_ID: &str = "H12_Not_MP";
const GENESIS_MISSION_ID: &str = OFFICIAL_FULL_CAMPAIGN_GENESIS_MISSION_ID_V1;
const REPLAY_SCHEMA_VERSION: u32 = robin_run_protocol::CURRENT_RANKED_REPLAY_SCHEMA_VERSION_V1;
const NETWORK_PROTOCOL_VERSION: u32 =
    robin_run_protocol::CURRENT_RANKED_NETWORK_PROTOCOL_VERSION_V1;

struct RigOptions {
    any_ruleset: bool,
    campaign: bool,
    competition: bool,
    allowed_metrics: Vec<String>,
}

impl Default for RigOptions {
    fn default() -> Self {
        Self {
            any_ruleset: false,
            campaign: false,
            competition: false,
            allowed_metrics: vec!["original_score".to_owned(), "fastest_success".to_owned()],
        }
    }
}

struct TestRig {
    _directory: tempfile::TempDir,
    app: Router,
    config: ServerConfig,
    database: Database,
    replay_store: ReplayStore,
    campaign_store: CampaignStore,
    loaded_build: LoadedBuildManifest,
    build: BuildManifestV1,
    content: ContentManifestV1,
    genesis_content: Option<ContentManifestV1>,
    terminal_content: Option<ContentManifestV1>,
    campaign_content: Option<CampaignContentManifestV1>,
    competition: Option<CompetitionManifestV1>,
    published_ruleset: robin_run_protocol::PublishedRulesetV1,
    build_sha256: Digest32,
    content_sha256: Digest32,
    rules_config_sha256: Digest32,
    ruleset_sha256: Digest32,
    starting_campaign_sha256: Digest32,
    starting_campaign: Vec<u8>,
    genesis_content_sha256: Option<Digest32>,
    terminal_content_sha256: Option<Digest32>,
    campaign_content_sha256: Option<Digest32>,
    competition_sha256: Option<Digest32>,
}

impl TestRig {
    async fn new() -> Self {
        Self::new_with(RigOptions::default()).await
    }

    async fn new_with(options: RigOptions) -> Self {
        let _ = tracing_subscriber::fmt()
            .with_env_filter("robin_highscores=trace")
            .with_test_writer()
            .try_init();
        let directory = tempfile::tempdir().unwrap();
        let starting_campaign = b"canonical individual-level starting campaign";
        let starting_campaign_sha256 = Digest32::digest_bytes(starting_campaign);
        let starting_campaign_path = directory.path().join("starting.campaign");
        tokio::fs::write(&starting_campaign_path, starting_campaign)
            .await
            .unwrap();

        let build = build_manifest();
        build.validate().unwrap();
        let build_sha256 = build.canonical_digest().unwrap();

        let content = if options.campaign {
            full_content_manifest()
        } else {
            content_manifest()
        };
        content.validate().unwrap();
        let content_sha256 = content.canonical_digest().unwrap();
        let genesis_content = options.campaign.then(genesis_content_manifest);
        if let Some(genesis_content) = &genesis_content {
            genesis_content.validate().unwrap();
        }
        let genesis_content_sha256 = genesis_content
            .as_ref()
            .map(|manifest| manifest.canonical_digest().unwrap());
        let terminal_content = options.campaign.then(terminal_content_manifest);
        if let Some(terminal_content) = &terminal_content {
            terminal_content.validate().unwrap();
        }
        let terminal_content_sha256 = terminal_content
            .as_ref()
            .map(|manifest| manifest.canonical_digest().unwrap());
        let campaign_content = genesis_content_sha256.zip(terminal_content_sha256).map(
            |(genesis_content_sha256, terminal_content_sha256)| {
                let mut entries = vec![
                    CampaignContentEntryV1 {
                        subject: OfficialContentSubjectV1::FieldMission {
                            mission_id: CAMPAIGN_MISSION_ID.to_owned(),
                        },
                        content_manifest_sha256: content_sha256,
                    },
                    CampaignContentEntryV1 {
                        subject: OfficialContentSubjectV1::FieldMission {
                            mission_id: GENESIS_MISSION_ID.to_owned(),
                        },
                        content_manifest_sha256: genesis_content_sha256,
                    },
                    CampaignContentEntryV1 {
                        subject: OfficialContentSubjectV1::FieldMission {
                            mission_id: HQ_MISSION_ID.to_owned(),
                        },
                        content_manifest_sha256: terminal_content_sha256,
                    },
                ];
                entries.sort_by(|left, right| left.subject.cmp(&right.subject));
                CampaignContentManifestV1 {
                    schema_version: SCHEMA_VERSION_V1,
                    edition: OfficialContentEditionV1::Full,
                    entries,
                }
            },
        );
        if let Some(campaign_content) = &campaign_content {
            campaign_content.validate().unwrap();
        }
        let campaign_content_sha256 = campaign_content
            .as_ref()
            .map(|manifest| manifest.canonical_digest().unwrap());

        let CanonicalValue::Object(sim_config) =
            CanonicalValue::from_serializable(&robin_engine::engine::SimConfig::default()).unwrap()
        else {
            panic!("SimConfig must canonicalize as an object")
        };
        let rules_config = RulesConfigIdentityV1 {
            schema_version: SCHEMA_VERSION_V1,
            replay_schema_version: REPLAY_SCHEMA_VERSION,
            ranked_simulation_policy: robin_run_protocol::RankedSimulationPolicyV1::standard(
                robin_run_protocol::RankedSimulationDifficultyV1::Medium,
            ),
            sim_config,
            rules: [("ranked".to_owned(), CanonicalValue::Bool(true))]
                .into_iter()
                .collect(),
        };
        rules_config.validate().unwrap();
        let rules_config_sha256 = rules_config.canonical_digest().unwrap();

        let mut content_digests = vec![content_sha256];
        if let Some(genesis_content_sha256) = genesis_content_sha256 {
            content_digests.push(genesis_content_sha256);
        }
        if let Some(terminal_content_sha256) = terminal_content_sha256 {
            content_digests.push(terminal_content_sha256);
        }
        content_digests.sort_unstable();
        let mut published_ruleset = published_ruleset(
            build_sha256,
            content_digests,
            rules_config_sha256,
            campaign_content_sha256,
        );
        if options.any_ruleset {
            published_ruleset.manifest.rules_config_constraint =
                RulesConfigConstraintV1::AnyCanonicalSimConfig;
            published_ruleset.manifest.preset_id = OpaqueId::new("any").unwrap();
            published_ruleset.manifest.difficulty_id = OpaqueId::new("any").unwrap();
            published_ruleset.ruleset_manifest_sha256 =
                published_ruleset.manifest.canonical_digest().unwrap();
        }
        published_ruleset.validate().unwrap();
        let ruleset_sha256 = published_ruleset.ruleset_manifest_sha256;
        let competition = options.competition.then(|| {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64;
            CompetitionManifestV1 {
                schema_version: SCHEMA_VERSION_V1,
                competition_id: OpaqueId::new("router-e2e-weekly").unwrap(),
                competition_version: 1,
                display_name: "The Leicester Trial".to_owned(),
                description: "Pinned-seed router integration fixture".to_owned(),
                subject: LeaderboardSubjectV1::Mission {
                    mission_id: MISSION_ID.to_owned(),
                    category: BoardCategoryV1::IndividualLevel,
                },
                metric: BoardMetricV1::OriginalScore,
                rules_config_sha256,
                ruleset_manifest_sha256: ruleset_sha256,
                canonical_campaign_state: published_ruleset.manifest.canonical_campaign_state,
                content: RunContentIdentityV1::Mission {
                    content_manifest_sha256: content_sha256,
                },
                seed_policy: CompetitionSeedPolicyV1::Pinned {
                    simulation_seed: SimulationSeed64::new(777),
                },
                participant_composition: CompetitionParticipantCompositionV1::SinglePlayer,
                competition_run_grant_public_key: protocol_public_key(&SigningKey::from_bytes(
                    &[0x47; 32],
                )),
                starts_at_unix_ms: now.saturating_sub(60_000),
                ends_at_unix_ms: now.saturating_add(3_600_000),
            }
        });
        if let Some(competition) = &competition {
            competition.validate().unwrap();
        }
        let competition_sha256 = competition
            .as_ref()
            .map(|manifest| manifest.canonical_digest().unwrap());

        let loaded_build = LoadedBuildManifest::new(
            robin_run_protocol::VersionedBuildManifest::V1(build.clone()),
        )
        .unwrap();
        let mut registry = ManifestRegistry::default();
        registry.builds.insert(build_sha256, loaded_build.clone());
        registry
            .content_manifests
            .insert(content_sha256, content.clone());
        if let (Some(genesis_content_sha256), Some(genesis_content)) =
            (genesis_content_sha256, genesis_content.clone())
        {
            registry
                .content_manifests
                .insert(genesis_content_sha256, genesis_content);
        }
        if let (Some(terminal_content_sha256), Some(terminal_content)) =
            (terminal_content_sha256, terminal_content.clone())
        {
            registry
                .content_manifests
                .insert(terminal_content_sha256, terminal_content);
        }
        if let (Some(campaign_content_sha256), Some(campaign_content)) =
            (campaign_content_sha256, campaign_content.clone())
        {
            registry
                .campaign_content_manifests
                .insert(campaign_content_sha256, campaign_content);
        }
        registry
            .rules_configs
            .insert(rules_config_sha256, rules_config);
        registry
            .rulesets
            .insert(ruleset_sha256, published_ruleset.clone());
        if let (Some(competition_sha256), Some(competition)) =
            (competition_sha256, competition.clone())
        {
            registry
                .competitions
                .insert(competition_sha256, competition);
        }

        let mut config = ServerConfig::default();
        config.database_path = directory.path().join("highscores.sqlite3");
        config.replay_directory = directory.path().join("replays");
        config.campaign_state_directory = directory.path().join("campaigns");
        config.max_campaign_bytes = 1024 * 1024;
        config.cursor_secret_path = directory.path().join("cursor.key");
        config.challenge_ttl_seconds = 60;
        config.minimum_storage_free_bytes = 64 * 1024 * 1024;
        config.abuse_reports_per_hour_per_ip = 1;
        config.abuse_reports_per_hour_per_key = 3;
        config.abuse_reports_per_hour_per_target = 2;
        config.admission_profiles.push(AdmissionProfile {
            id: if options.campaign {
                "full-leicester-standard".to_owned()
            } else {
                "demo-leicester-standard".to_owned()
            },
            content_subject: OfficialContentSubjectV1::FieldMission {
                mission_id: if options.campaign {
                    CAMPAIGN_MISSION_ID.to_owned()
                } else {
                    MISSION_ID.to_owned()
                },
            },
            mission_display_name: "The Disgrace of Leicester".to_owned(),
            allowed_scopes: if options.campaign {
                vec!["campaign_continuation".to_owned()]
            } else {
                vec!["individual_level".to_owned()]
            },
            build_manifest_id: build_sha256.to_string(),
            content_manifest_id: content_sha256.to_string(),
            campaign_content_manifest_id: campaign_content_sha256.map(|value| value.to_string()),
            config_id: rules_config_sha256.to_string(),
            ruleset_id: ruleset_sha256.to_string(),
            template_id: if options.campaign {
                "full-leicester-default".to_owned()
            } else {
                "demo-leicester-default".to_owned()
            },
            canonical_campaign_state: CanonicalCampaignStatePinV1 {
                requirement: published_ruleset.manifest.canonical_campaign_state,
                artifact: ArtifactRefV1 {
                    sha256: starting_campaign_sha256,
                    byte_length: starting_campaign.len() as u64,
                    media_type: RANKED_CAMPAIGN_MEDIA_TYPE_V1.to_owned(),
                },
            },
            canonical_campaign_state_path: Some(starting_campaign_path.clone()),
            allowed_metrics: options.allowed_metrics.clone(),
            ruleset_display_name: "Standard / Normal".to_owned(),
            preset_id: "standard".to_owned(),
            preset_name: "Standard".to_owned(),
            difficulty_id: "normal".to_owned(),
            difficulty_name: "Normal".to_owned(),
            build_display_name: "Test verifier build".to_owned(),
            viewer_engine_build: "test-viewer".to_owned(),
            viewer_available: false,
            viewer_unavailable_reason: Some("No test viewer bundle is published.".to_owned()),
            viewer_content_requirement: None,
        });
        if options.campaign {
            config.admission_profiles.push(AdmissionProfile {
                id: "full-h01-genesis-standard".to_owned(),
                content_subject: OfficialContentSubjectV1::FieldMission {
                    mission_id: GENESIS_MISSION_ID.to_owned(),
                },
                mission_display_name: "Robin's Godfather".to_owned(),
                allowed_scopes: vec![
                    "campaign_genesis".to_owned(),
                    "campaign_continuation".to_owned(),
                ],
                build_manifest_id: build_sha256.to_string(),
                content_manifest_id: genesis_content_sha256.unwrap().to_string(),
                campaign_content_manifest_id: Some(campaign_content_sha256.unwrap().to_string()),
                config_id: rules_config_sha256.to_string(),
                ruleset_id: ruleset_sha256.to_string(),
                template_id: "full-h01-genesis-default".to_owned(),
                canonical_campaign_state: CanonicalCampaignStatePinV1 {
                    requirement: published_ruleset.manifest.canonical_campaign_state,
                    artifact: ArtifactRefV1 {
                        sha256: starting_campaign_sha256,
                        byte_length: starting_campaign.len() as u64,
                        media_type: RANKED_CAMPAIGN_MEDIA_TYPE_V1.to_owned(),
                    },
                },
                canonical_campaign_state_path: Some(starting_campaign_path),
                allowed_metrics: options.allowed_metrics.clone(),
                ruleset_display_name: "Standard / Normal".to_owned(),
                preset_id: "standard".to_owned(),
                preset_name: "Standard".to_owned(),
                difficulty_id: "normal".to_owned(),
                difficulty_name: "Normal".to_owned(),
                build_display_name: "Test verifier build".to_owned(),
                viewer_engine_build: "test-viewer".to_owned(),
                viewer_available: false,
                viewer_unavailable_reason: Some("No test viewer bundle is published.".to_owned()),
                viewer_content_requirement: None,
            });
            config.admission_profiles.push(AdmissionProfile {
                id: "full-h12-standard".to_owned(),
                content_subject: OfficialContentSubjectV1::FieldMission {
                    mission_id: HQ_MISSION_ID.to_owned(),
                },
                mission_display_name: "The Last Arrow".to_owned(),
                allowed_scopes: vec!["campaign_continuation".to_owned()],
                build_manifest_id: build_sha256.to_string(),
                content_manifest_id: terminal_content_sha256.unwrap().to_string(),
                campaign_content_manifest_id: Some(campaign_content_sha256.unwrap().to_string()),
                config_id: rules_config_sha256.to_string(),
                ruleset_id: ruleset_sha256.to_string(),
                template_id: "full-h12-default".to_owned(),
                canonical_campaign_state: CanonicalCampaignStatePinV1 {
                    requirement: published_ruleset.manifest.canonical_campaign_state,
                    artifact: ArtifactRefV1 {
                        sha256: starting_campaign_sha256,
                        byte_length: starting_campaign.len() as u64,
                        media_type: RANKED_CAMPAIGN_MEDIA_TYPE_V1.to_owned(),
                    },
                },
                canonical_campaign_state_path: Some(directory.path().join("starting.campaign")),
                allowed_metrics: options.allowed_metrics.clone(),
                ruleset_display_name: "Standard / Normal".to_owned(),
                preset_id: "standard".to_owned(),
                preset_name: "Standard".to_owned(),
                difficulty_id: "normal".to_owned(),
                difficulty_name: "Normal".to_owned(),
                build_display_name: "Test verifier build".to_owned(),
                viewer_engine_build: "test-viewer".to_owned(),
                viewer_available: false,
                viewer_unavailable_reason: Some("No test viewer bundle is published.".to_owned()),
                viewer_content_requirement: None,
            });
        }
        if let Some(competition_sha256) = competition_sha256 {
            config.competitions.push(CompetitionConfig {
                manifest_sha256: competition_sha256.to_string(),
                admission_profile_id: "demo-leicester-standard".to_owned(),
            });
        }
        config.manifests = Arc::new(registry);

        // Production API/worker startup uses `Database::connect`; only this
        // test fixture invokes the explicit administrative migration path.
        let database = Database::migrate(&config).await.unwrap();
        let replay_store = ReplayStore::create(config.replay_directory.clone(), 1024)
            .await
            .unwrap();
        let campaign_store = CampaignStore::create(
            config.campaign_state_directory.clone(),
            config.max_campaign_bytes,
        )
        .await
        .unwrap();
        let app = router(AppState {
            config: config.clone(),
            database: database.clone(),
            replay_store: replay_store.clone(),
            campaign_store: campaign_store.clone(),
            cursor_hmac_key: [0xa5; 32],
            backup_authority_hmac_key: [0xa6; 32],
            competition_run_grant_secret_key: options.competition.then_some([0x47; 32]),
            run_preflight_grant_secret_key: Some([0x46; 32]),
            challenge_rate_limiter: ChallengeRateLimiter::new(1_000),
        })
        .unwrap();

        Self {
            _directory: directory,
            app,
            config,
            database,
            replay_store,
            campaign_store,
            loaded_build,
            build,
            content,
            genesis_content,
            terminal_content,
            campaign_content,
            competition,
            published_ruleset,
            build_sha256,
            content_sha256,
            rules_config_sha256,
            ruleset_sha256,
            starting_campaign_sha256,
            starting_campaign: starting_campaign.to_vec(),
            genesis_content_sha256,
            terminal_content_sha256,
            campaign_content_sha256,
            competition_sha256,
        }
    }

    fn app_with_ruleset_status(&self, operational_status: RulesetOperationalStatusV1) -> Router {
        let mut config = self.config.clone();
        let registry = Arc::make_mut(&mut config.manifests);
        registry
            .rulesets
            .get_mut(&self.ruleset_sha256)
            .unwrap()
            .operational_status = operational_status;
        router(AppState {
            config,
            database: self.database.clone(),
            replay_store: self.replay_store.clone(),
            campaign_store: self.campaign_store.clone(),
            cursor_hmac_key: [0xa5; 32],
            backup_authority_hmac_key: [0xa6; 32],
            competition_run_grant_secret_key: self.competition.is_some().then_some([0x47; 32]),
            run_preflight_grant_secret_key: Some([0x46; 32]),
            challenge_rate_limiter: ChallengeRateLimiter::new(1_000),
        })
        .unwrap()
    }

    fn app_with_storage_floor(&self, minimum_storage_free_bytes: u64) -> Router {
        let mut config = self.config.clone();
        config.minimum_storage_free_bytes = minimum_storage_free_bytes;
        router(AppState {
            config,
            database: self.database.clone(),
            replay_store: self.replay_store.clone(),
            campaign_store: self.campaign_store.clone(),
            cursor_hmac_key: [0xa5; 32],
            backup_authority_hmac_key: [0xa6; 32],
            competition_run_grant_secret_key: self.competition.is_some().then_some([0x47; 32]),
            run_preflight_grant_secret_key: Some([0x46; 32]),
            challenge_rate_limiter: ChallengeRateLimiter::new(1_000),
        })
        .unwrap()
    }

    fn app_with_backup_status(&self, path: std::path::PathBuf) -> Router {
        let mut config = self.config.clone();
        config.backup_manifest_path = Some(path);
        config.maximum_backup_age_hours = Some(1);
        router(AppState {
            config,
            database: self.database.clone(),
            replay_store: self.replay_store.clone(),
            campaign_store: self.campaign_store.clone(),
            cursor_hmac_key: [0xa5; 32],
            backup_authority_hmac_key: [0xa6; 32],
            competition_run_grant_secret_key: self.competition.is_some().then_some([0x47; 32]),
            run_preflight_grant_secret_key: Some([0x46; 32]),
            challenge_rate_limiter: ChallengeRateLimiter::new(1_000),
        })
        .unwrap()
    }

    async fn rename(&self, key: &SigningKey, username: &str, peer: Ipv4Addr) -> PlayerProfileV1 {
        let public_key = protocol_public_key(key);
        let challenge_request = UsernameChallengeRequestV1 {
            schema_version: SCHEMA_VERSION_V1,
            public_key,
        };
        let challenge_response = self
            .app
            .clone()
            .oneshot(json_request(
                Method::POST,
                "/api/v1/username-challenges",
                &challenge_request,
                peer,
            ))
            .await
            .unwrap();
        assert_eq!(challenge_response.status(), StatusCode::CREATED);
        let challenge: UsernameChallengeV1 = json_body(challenge_response).await;
        challenge.validate().unwrap();

        let mut update = UsernameUpdateEnvelopeV1 {
            schema_version: SCHEMA_VERSION_V1,
            username_challenge_id: challenge.username_challenge_id,
            username_challenge_nonce: challenge.username_challenge_nonce,
            public_key,
            username: username.to_owned(),
            signature: Signature64::default(),
        };
        update.signature = sign(key, &update.signing_bytes().unwrap());
        update.validate().unwrap();
        let response = self
            .app
            .clone()
            .oneshot(json_request(
                Method::PUT,
                &format!("/api/v1/players/{public_key}/username"),
                &update,
                peer,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let profile: PlayerProfileV1 = json_body(response).await;
        profile.validate().unwrap();
        profile
    }

    async fn submit(&self, key: &SigningKey, sequence: u8) -> SubmissionAcceptedV1 {
        let offer_request = self.offer_request(key, sequence);
        self.submit_request(
            key,
            sequence,
            offer_request,
            vec![BoardMetricV1::OriginalScore, BoardMetricV1::FastestSuccess],
            CampaignAggregationConsentV1::NotAuthorized,
            None,
        )
        .await
    }

    async fn submit_request(
        &self,
        key: &SigningKey,
        sequence: u8,
        mut offer_request: SubmissionOfferRequestV1,
        requested_metrics: Vec<BoardMetricV1>,
        campaign_aggregation_consent: CampaignAggregationConsentV1,
        continuation_authorization: Option<CampaignContinuationAuthorizationV1>,
    ) -> SubmissionAcceptedV1 {
        self.authorize_fresh_request(key, &mut offer_request, sequence)
            .await;
        let offer_response = self
            .app
            .clone()
            .oneshot(json_request(
                Method::POST,
                "/api/v1/submission-offers",
                &offer_request,
                Ipv4Addr::new(127, 10, 0, sequence),
            ))
            .await
            .unwrap();
        assert_eq!(offer_response.status(), StatusCode::CREATED);
        let offer: SubmissionOfferV1 = json_body(offer_response).await;
        offer.validate().unwrap();
        assert_eq!(offer.session_genesis, offer_request.session_genesis);
        assert_eq!(offer.participant_claims, offer_request.participant_claims);

        let replay = compact_replay_fixture(&format!("run-{sequence}"));
        let artifacts = submission_artifacts(&replay, &self.starting_campaign);
        let envelope = SubmissionEnvelopeV1 {
            schema_version: SCHEMA_VERSION_V1,
            replay_session_transcript: replay_session_transcript(&offer),
            offer,
            artifacts,
            campaign_aggregation_consent,
            campaign_continuation_authorization: continuation_authorization,
            requested_metrics,
        };
        let signed = SignedSubmissionV1 {
            schema_version: SCHEMA_VERSION_V1,
            algorithm: SignatureAlgorithmV1::Ed25519,
            participant_signatures: vec![ParticipantSignatureV1 {
                public_key: protocol_public_key(key),
                signature: sign(key, &envelope.signing_bytes().unwrap()),
            }],
            submission: envelope,
        };
        signed.validate().unwrap();

        let response = self
            .app
            .clone()
            .oneshot(multipart_request(&signed, &replay, &self.starting_campaign))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::ACCEPTED);
        let accepted: SubmissionAcceptedV1 = json_body(response).await;
        accepted.validate().unwrap();
        assert_eq!(accepted.state, SubmissionLifecycleV1::Queued);
        accepted
    }

    async fn publish(
        &self,
        accepted: &SubmissionAcceptedV1,
        score: i32,
        final_byte: u8,
    ) -> OpaqueId {
        self.publish_individual(accepted, score, final_byte, false)
            .await
    }

    async fn publish_competition(
        &self,
        accepted: &SubmissionAcceptedV1,
        score: i32,
        final_byte: u8,
    ) -> OpaqueId {
        self.publish_individual(accepted, score, final_byte, true)
            .await
    }

    async fn store_campaign(&self, bytes: &[u8]) -> ArtifactRefV1 {
        let artifact = campaign_artifact(bytes);
        self.campaign_store
            .import_bytes(artifact.sha256.as_bytes(), bytes)
            .await
            .unwrap();
        self.database
            .register_campaign_object(artifact.sha256.as_bytes(), artifact.byte_length)
            .await
            .unwrap();
        artifact
    }

    async fn publish_individual(
        &self,
        accepted: &SubmissionAcceptedV1,
        score: i32,
        final_byte: u8,
        competition: bool,
    ) -> OpaqueId {
        let worker_id = format!("router-e2e-worker-{final_byte}");
        let job = self
            .database
            .lease_next(&worker_id, Duration::from_secs(60))
            .await
            .unwrap()
            .expect("the HTTP upload must create a leaseable queue entry");
        assert_eq!(job.submission_id, accepted.submission_id.as_str());
        let request = build_verification_request(&job, verification_limits()).unwrap();
        let request_sha256 = request.canonical_digest().unwrap();
        self.database
            .record_verification_request(
                &job.submission_id,
                &worker_id,
                &request,
                &robin_run_protocol::VerifierJobRouteV1::from_request(&request),
                Digest32::digest_bytes(b"router-e2e-job-config"),
                self.published_ruleset
                    .manifest
                    .verifier_policy
                    .manifest_sha256,
            )
            .await
            .unwrap();

        let signed = &request.submission;
        let offer = &signed.submission.offer;
        let final_campaign = self.store_campaign(&[final_byte; 97]).await;
        let session_genesis_sha256 = offer.session_genesis.canonical_digest().unwrap();
        let claims = offer.participant_claims.clone();
        let named_participant_instance_count = claims
            .iter()
            .filter(|claim| claim.public_disclosure == ParticipantPublicDisclosureV1::NamedProfile)
            .count() as u16;
        let verified = VerifiedRunV1 {
            scope_kind: RunScopeKindV1::IndividualLevel,
            campaign_aggregation_consent: CampaignAggregationConsentV1::NotAuthorized,
            campaign_session_kind: None,
            campaign_session_ordinal: None,
            max_concurrent_players: 1,
            participant_instance_count: 1,
            named_participant_instance_count,
            anonymous_participant_instance_count: 1 - named_participant_instance_count,
            authenticated_participant_claims: claims,
            replay_session_transcript: signed.submission.replay_session_transcript.clone(),
            outcome: TerminalOutcomeV1::Won,
            starting_campaign: signed.submission.artifacts.starting_campaign.clone(),
            final_campaign,
            starting_campaign_score: 0,
            final_campaign_score: score,
            final_state_sha256: Digest32::from_bytes([final_byte.wrapping_add(3); 32]),
            replay_frames: 100,
            original_score_delta: i64::from(score),
            active_simulation_ticks: 1_000_u64.saturating_sub(u64::from(final_byte)),
            ransom_collected: u64::from(final_byte) * 10,
            campaign_complete_evidence: None,
            achievements: self
                .published_ruleset
                .manifest
                .achievement_policies
                .iter()
                .map(|policy| VerifiedAchievementV1 {
                    achievement_id: policy.achievement_id.clone(),
                    evaluation: VerifiedAchievementEvaluationV1::NotEarned,
                    evidence: BTreeMap::new(),
                })
                .collect(),
            diagnostics: BTreeMap::new(),
        };
        let result = VerificationResultV1 {
            schema_version: SCHEMA_VERSION_V1,
            request_id: accepted.submission_id.clone(),
            verification_request_sha256: request_sha256,
            artifacts: signed.submission.artifacts.clone(),
            session_genesis_sha256,
            build_manifest_sha256: self.build_sha256,
            content_manifest_sha256: self.content_sha256,
            rules_config_sha256: self.rules_config_sha256,
            ruleset_manifest_sha256: self.ruleset_sha256,
            competition_manifest_sha256: if competition {
                Some(
                    self.competition_sha256
                        .expect("competition publication requires fixture"),
                )
            } else {
                None
            },
            input_provenance: Some(InputProvenanceStatusV1::Rankable),
            status: VerificationStatusV1::Verified(verified),
        };
        result.validate().unwrap();
        let substituted = self
            .database
            .accept_job(
                accepted.submission_id.as_str(),
                &worker_id,
                self.build.verifier.sha256.into_bytes(),
                Digest32::digest_bytes(b"substituted-router-e2e-job-config"),
                &result,
                &self.loaded_build,
                &self.content,
                None,
                &self.published_ruleset,
                competition.then(|| self.competition.as_ref().unwrap()),
            )
            .await;
        assert!(matches!(
            substituted,
            Err(robin_highscores::db::DbError::ResultInvariant(_))
        ));
        let run_id = self
            .database
            .accept_job(
                accepted.submission_id.as_str(),
                &worker_id,
                self.build.verifier.sha256.into_bytes(),
                Digest32::digest_bytes(b"router-e2e-job-config"),
                &result,
                &self.loaded_build,
                &self.content,
                None,
                &self.published_ruleset,
                competition.then(|| self.competition.as_ref().unwrap()),
            )
            .await
            .unwrap();
        OpaqueId::new(run_id).unwrap()
    }

    fn fresh_preflight_request(
        &self,
        request: &SubmissionOfferRequestV1,
        sequence: u8,
    ) -> Option<FreshRunPreflightRequestV1> {
        let scope = match request.scope_request {
            ScopeRequestV1::IndividualLevel => FreshRunScopeV1::IndividualLevel,
            ScopeRequestV1::CampaignGenesis => FreshRunScopeV1::CampaignGenesis,
            ScopeRequestV1::CampaignContinuation { .. } => {
                assert!(
                    request
                        .session_genesis
                        .claim
                        .fresh_run_preflight_grant
                        .is_none()
                );
                return None;
            }
        };
        let ranked = &request.session_genesis.claim.ranked_session;
        Some(FreshRunPreflightRequestV1 {
            claim: FreshRunPreflightRequestClaimV1 {
                schema_version: SCHEMA_VERSION_V1,
                request_nonce: ChallengeNonce32::from_bytes([sequence.wrapping_add(110); 32]),
                host_public_key: request.session_genesis.claim.host_public_key,
                replay_session_id: request.session_genesis.claim.replay_session_id,
                host_participant_instance_id: request
                    .session_genesis
                    .claim
                    .host_participant_instance_id,
                host_nonce: request.session_genesis.claim.host_nonce,
                scope,
                starting_campaign: ArtifactRefV1 {
                    sha256: ranked.starting_campaign_sha256,
                    byte_length: ranked.starting_campaign_byte_length,
                    media_type: RANKED_CAMPAIGN_MEDIA_TYPE_V1.to_owned(),
                },
                ranked_session: ranked.clone(),
            },
            algorithm: SignatureAlgorithmV1::Ed25519,
            host_signature: Signature64::default(),
        })
    }

    async fn authorize_fresh_request(
        &self,
        owner: &SigningKey,
        request: &mut SubmissionOfferRequestV1,
        sequence: u8,
    ) {
        let Some(mut preflight) = self.fresh_preflight_request(request, sequence) else {
            request.validate().unwrap();
            return;
        };
        preflight.host_signature = sign(owner, &preflight.signing_bytes().unwrap());
        let response = self
            .app
            .clone()
            .oneshot(json_request(
                Method::POST,
                "/api/v1/fresh-run-preflight-grants",
                &preflight,
                Ipv4Addr::new(127, 8, 0, sequence),
            ))
            .await
            .unwrap();
        if response.status() != StatusCode::CREATED {
            let status = response.status();
            let body = response.into_body().collect().await.unwrap().to_bytes();
            panic!(
                "fresh-run preflight returned {status}, expected 201: {}",
                String::from_utf8_lossy(&body)
            );
        }
        let grant: FreshRunPreflightGrantV1 = json_body(response).await;
        grant.validate_request(&preflight).unwrap();
        request.session_genesis.claim.fresh_run_preflight_grant = Some(grant);
        request.session_genesis.host_signature =
            sign(owner, &request.session_genesis.signing_bytes().unwrap());
        request.validate().unwrap();
    }

    fn continuation_preflight_request(
        &self,
        request: &SubmissionOfferRequestV1,
        receipt: &CampaignChainReceiptV1,
        sequence: u8,
    ) -> CampaignContinuationPreflightRequestV1 {
        let ScopeRequestV1::CampaignContinuation {
            chain_id,
            predecessor_run_id,
        } = &request.scope_request
        else {
            panic!("continuation preflight requires a continuation offer request")
        };
        assert_eq!(chain_id, &receipt.chain_id);
        assert_eq!(predecessor_run_id, &receipt.predecessor_run_id);
        let mut participant_public_keys = request
            .participant_claims
            .iter()
            .map(|participant| participant.public_key)
            .collect::<Vec<_>>();
        participant_public_keys.sort_unstable();
        participant_public_keys.dedup();
        let ranked = &request.session_genesis.claim.ranked_session;
        CampaignContinuationPreflightRequestV1 {
            claim: CampaignContinuationPreflightRequestClaimV1 {
                schema_version: SCHEMA_VERSION_V1,
                request_nonce: ChallengeNonce32::from_bytes([sequence.wrapping_add(111); 32]),
                host_public_key: request.session_genesis.claim.host_public_key,
                campaign_controller_public_key: receipt.campaign_controller_public_key,
                replay_session_id: request.session_genesis.claim.replay_session_id,
                host_participant_instance_id: request
                    .session_genesis
                    .claim
                    .host_participant_instance_id,
                host_nonce: request.session_genesis.claim.host_nonce,
                max_concurrent_players: request.max_concurrent_players,
                participant_public_keys,
                chain_id: chain_id.clone(),
                predecessor_run_id: predecessor_run_id.clone(),
                predecessor_verification_sha256: receipt.predecessor_verification_sha256,
                starting_campaign: receipt.expected_starting_campaign.clone(),
                ranked_session: ranked.clone(),
            },
            algorithm: SignatureAlgorithmV1::Ed25519,
            host_signature: Signature64::default(),
            controller_signature: Signature64::default(),
        }
    }

    async fn authorize_continuation_request(
        &self,
        host: &SigningKey,
        controller: &SigningKey,
        receipt: &CampaignChainReceiptV1,
        request: &mut SubmissionOfferRequestV1,
        sequence: u8,
    ) {
        let mut preflight = self.continuation_preflight_request(request, receipt, sequence);
        preflight.host_signature = sign(host, &preflight.claim.host_signing_bytes().unwrap());
        preflight.controller_signature = sign(
            controller,
            &preflight.claim.controller_signing_bytes().unwrap(),
        );
        preflight.validate().unwrap();
        let response = self
            .app
            .clone()
            .oneshot(json_request(
                Method::POST,
                "/api/v1/campaign-continuation-preflight-grants",
                &preflight,
                Ipv4Addr::new(127, 8, 1, sequence),
            ))
            .await
            .unwrap();
        if response.status() != StatusCode::CREATED {
            let status = response.status();
            let body = response.into_body().collect().await.unwrap().to_bytes();
            panic!(
                "continuation preflight returned {status}, expected 201: {}",
                String::from_utf8_lossy(&body)
            );
        }
        let grant: CampaignContinuationPreflightGrantV1 = json_body(response).await;
        grant.validate_request(&preflight).unwrap();
        request
            .session_genesis
            .claim
            .campaign_continuation_preflight_grant = Some(grant);
        request.session_genesis.host_signature =
            sign(host, &request.session_genesis.signing_bytes().unwrap());
        request.validate().unwrap();
    }

    async fn issue_continuation_offer(
        &self,
        host: &SigningKey,
        controller: &SigningKey,
        receipt: &CampaignChainReceiptV1,
        mut request: SubmissionOfferRequestV1,
        sequence: u8,
    ) -> SubmissionOfferV1 {
        self.authorize_continuation_request(host, controller, receipt, &mut request, sequence)
            .await;
        let response = self
            .app
            .clone()
            .oneshot(json_request(
                Method::POST,
                "/api/v1/submission-offers",
                &request,
                Ipv4Addr::new(127, 20, 1, sequence),
            ))
            .await
            .unwrap();
        if response.status() != StatusCode::CREATED {
            let status = response.status();
            let body = response.into_body().collect().await.unwrap().to_bytes();
            panic!(
                "continuation offer returned {status}, expected 201: {}",
                String::from_utf8_lossy(&body)
            );
        }
        let offer: SubmissionOfferV1 = json_body(response).await;
        offer.validate().unwrap();
        offer
    }

    async fn issue_offer(
        &self,
        owner: &SigningKey,
        mut request: SubmissionOfferRequestV1,
        sequence: u8,
    ) -> SubmissionOfferV1 {
        self.authorize_fresh_request(owner, &mut request, sequence)
            .await;
        let response = self
            .app
            .clone()
            .oneshot(json_request(
                Method::POST,
                "/api/v1/submission-offers",
                &request,
                Ipv4Addr::new(127, 20, 0, sequence),
            ))
            .await
            .unwrap();
        if response.status() != StatusCode::CREATED {
            let status = response.status();
            let body = response.into_body().collect().await.unwrap().to_bytes();
            panic!(
                "offer request returned {status}, expected 201: {}",
                String::from_utf8_lossy(&body)
            );
        }
        let offer: SubmissionOfferV1 = json_body(response).await;
        offer.validate().unwrap();
        offer
    }

    async fn upload_campaign_offer(
        &self,
        owner: &SigningKey,
        sequence: u8,
        offer: SubmissionOfferV1,
    ) -> SubmissionAcceptedV1 {
        let replay = compact_replay_fixture(&format!("campaign-run-{sequence}"));
        let starting_campaign =
            if offer.starting_state.campaign_sha256() == self.starting_campaign_sha256 {
                self.starting_campaign.clone()
            } else {
                let mut file = self
                    .campaign_store
                    .open_verified(offer.starting_state.campaign_sha256().as_bytes())
                    .await
                    .unwrap();
                let mut bytes = Vec::new();
                file.read_to_end(&mut bytes).await.unwrap();
                bytes
            };
        let artifacts = submission_artifacts(&replay, &starting_campaign);
        let continuation = match &offer.starting_state {
            InitialStateExpectationV1::CampaignContinuation {
                chain_id,
                predecessor_run_id,
                predecessor_verification_sha256,
                ..
            } => {
                let mut authorization = CampaignContinuationAuthorizationV1 {
                    claim: CampaignContinuationAuthorizationClaimV1 {
                        schema_version: SCHEMA_VERSION_V1,
                        campaign_controller_public_key: protocol_public_key(owner),
                        chain_id: chain_id.clone(),
                        predecessor_run_id: predecessor_run_id.clone(),
                        predecessor_verification_sha256: *predecessor_verification_sha256,
                        next_session_genesis_sha256: offer
                            .session_genesis
                            .canonical_digest()
                            .unwrap(),
                        next_artifacts: artifacts.clone(),
                    },
                    algorithm: SignatureAlgorithmV1::Ed25519,
                    signature: Signature64::default(),
                };
                authorization.signature =
                    sign(owner, &authorization.signing_bytes(&offer).unwrap());
                authorization.validate().unwrap();
                Some(authorization)
            }
            InitialStateExpectationV1::CampaignGenesis { .. } => None,
            InitialStateExpectationV1::IndividualLevel { .. } => {
                panic!("campaign uploader received an individual offer")
            }
        };
        let envelope = SubmissionEnvelopeV1 {
            schema_version: SCHEMA_VERSION_V1,
            replay_session_transcript: replay_session_transcript(&offer),
            offer,
            artifacts,
            campaign_aggregation_consent:
                CampaignAggregationConsentV1::AuthorizeSignedSessionInServerRecognizedChainV1,
            campaign_continuation_authorization: continuation,
            requested_metrics: vec![BoardMetricV1::OriginalScore, BoardMetricV1::FastestSuccess],
        };
        let signed = SignedSubmissionV1 {
            schema_version: SCHEMA_VERSION_V1,
            algorithm: SignatureAlgorithmV1::Ed25519,
            participant_signatures: vec![ParticipantSignatureV1 {
                public_key: protocol_public_key(owner),
                signature: sign(owner, &envelope.signing_bytes().unwrap()),
            }],
            submission: envelope,
        };
        signed.validate().unwrap();
        let response = self
            .app
            .clone()
            .oneshot(multipart_request(&signed, &replay, &starting_campaign))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::ACCEPTED);
        let accepted: SubmissionAcceptedV1 = json_body(response).await;
        accepted.validate().unwrap();
        accepted
    }

    async fn publish_campaign(
        &self,
        accepted: &SubmissionAcceptedV1,
        session_kind: CampaignSessionKindV1,
        session_ordinal: u32,
        starting_score: i32,
        final_score: i32,
        final_byte: u8,
        campaign_complete: bool,
    ) -> OpaqueId {
        let worker_id = format!("router-e2e-campaign-worker-{final_byte}");
        let job = self
            .database
            .lease_next(&worker_id, Duration::from_secs(60))
            .await
            .unwrap()
            .expect("campaign upload must create a leaseable queue entry");
        assert_eq!(job.submission_id, accepted.submission_id.as_str());
        let request = build_verification_request(&job, verification_limits()).unwrap();
        let request_sha256 = request.canonical_digest().unwrap();
        self.database
            .record_verification_request(
                &job.submission_id,
                &worker_id,
                &request,
                &robin_run_protocol::VerifierJobRouteV1::from_request(&request),
                Digest32::digest_bytes(b"router-e2e-campaign-job-config"),
                self.published_ruleset
                    .manifest
                    .verifier_policy
                    .manifest_sha256,
            )
            .await
            .unwrap();

        let signed = &request.submission;
        let offer = &signed.submission.offer;
        let final_campaign = self.store_campaign(&[final_byte; 113]).await;
        let final_state_sha256 = Digest32::from_bytes([final_byte.wrapping_add(3); 32]);
        let session_genesis_sha256 = offer.session_genesis.canonical_digest().unwrap();
        match (&offer.starting_state, session_ordinal) {
            (InitialStateExpectationV1::CampaignGenesis { .. }, 0)
            | (InitialStateExpectationV1::CampaignContinuation { .. }, 1..) => {}
            (InitialStateExpectationV1::CampaignGenesis { .. }, _)
            | (InitialStateExpectationV1::CampaignContinuation { .. }, 0) => {
                panic!("campaign session ordinal does not match the offer scope")
            }
            (InitialStateExpectationV1::IndividualLevel { .. }, _) => {
                panic!("campaign verifier received individual starting state")
            }
        }
        let named_participant_instance_count = offer
            .participant_claims
            .iter()
            .filter(|claim| claim.public_disclosure == ParticipantPublicDisclosureV1::NamedProfile)
            .count() as u16;
        let verified = VerifiedRunV1 {
            scope_kind: RunScopeKindV1::Campaign,
            campaign_aggregation_consent:
                CampaignAggregationConsentV1::AuthorizeSignedSessionInServerRecognizedChainV1,
            campaign_session_kind: Some(session_kind),
            campaign_session_ordinal: Some(session_ordinal),
            max_concurrent_players: 1,
            participant_instance_count: 1,
            named_participant_instance_count,
            anonymous_participant_instance_count: 1 - named_participant_instance_count,
            authenticated_participant_claims: offer.participant_claims.clone(),
            replay_session_transcript: signed.submission.replay_session_transcript.clone(),
            outcome: TerminalOutcomeV1::Won,
            starting_campaign: signed.submission.artifacts.starting_campaign.clone(),
            final_campaign: final_campaign.clone(),
            starting_campaign_score: starting_score,
            final_campaign_score: final_score,
            final_state_sha256,
            replay_frames: 120,
            original_score_delta: i64::from(final_score.wrapping_sub(starting_score) as u32),
            active_simulation_ticks: u64::from(final_byte) * 10,
            ransom_collected: u64::from(final_byte),
            campaign_complete_evidence: campaign_complete.then(|| CampaignCompleteEvidenceV1 {
                schema_version: SCHEMA_VERSION_V1,
                campaign_content_manifest_sha256: self.campaign_content_sha256.unwrap(),
                content_manifest_sha256: offer.content_manifest_sha256,
                rules_config_sha256: self.rules_config_sha256,
                ruleset_manifest_sha256: self.ruleset_sha256,
                verification_request_sha256: request_sha256,
                replay_sha256: signed.submission.artifacts.replay.artifact.sha256,
                terminal_subject: offer
                    .session_genesis
                    .claim
                    .ranked_session
                    .content_subject
                    .clone(),
                final_campaign_sha256: final_campaign.sha256,
                final_state_sha256,
                observed_progression_percent: 100,
            }),
            achievements: self
                .published_ruleset
                .manifest
                .achievement_policies
                .iter()
                .map(|policy| VerifiedAchievementV1 {
                    achievement_id: policy.achievement_id.clone(),
                    evaluation: VerifiedAchievementEvaluationV1::NotEarned,
                    evidence: BTreeMap::new(),
                })
                .collect(),
            diagnostics: BTreeMap::new(),
        };
        let content = if offer.content_manifest_sha256 == self.content_sha256 {
            &self.content
        } else if offer.content_manifest_sha256 == self.genesis_content_sha256.unwrap() {
            self.genesis_content
                .as_ref()
                .expect("campaign genesis content fixture")
        } else if offer.content_manifest_sha256 == self.terminal_content_sha256.unwrap() {
            self.terminal_content
                .as_ref()
                .expect("terminal content fixture")
        } else {
            panic!("campaign offer references content outside the fixture catalog")
        };
        let result = VerificationResultV1 {
            schema_version: SCHEMA_VERSION_V1,
            request_id: accepted.submission_id.clone(),
            verification_request_sha256: request_sha256,
            artifacts: signed.submission.artifacts.clone(),
            session_genesis_sha256,
            build_manifest_sha256: self.build_sha256,
            content_manifest_sha256: offer.content_manifest_sha256,
            rules_config_sha256: self.rules_config_sha256,
            ruleset_manifest_sha256: self.ruleset_sha256,
            competition_manifest_sha256: None,
            input_provenance: Some(InputProvenanceStatusV1::Rankable),
            status: VerificationStatusV1::Verified(verified),
        };
        result.validate().unwrap();
        let run_id = self
            .database
            .accept_job(
                accepted.submission_id.as_str(),
                &worker_id,
                self.build.verifier.sha256.into_bytes(),
                Digest32::digest_bytes(b"router-e2e-campaign-job-config"),
                &result,
                &self.loaded_build,
                content,
                self.campaign_content.as_ref(),
                &self.published_ruleset,
                None,
            )
            .await
            .unwrap();
        OpaqueId::new(run_id).unwrap()
    }

    async fn owner_status(
        &self,
        accepted: &SubmissionAcceptedV1,
        owner: &SigningKey,
    ) -> SubmissionOwnerStatusResponseV1 {
        let envelope = self
            .owner_status_envelope(&accepted.submission_id, owner)
            .await;
        let response = self
            .app
            .clone()
            .oneshot(json_request(
                Method::POST,
                &format!(
                    "/api/v1/submissions/{}/private-status",
                    accepted.submission_id
                ),
                &envelope,
                Ipv4Addr::LOCALHOST,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let status: SubmissionOwnerStatusResponseV1 = json_body(response).await;
        status.validate().unwrap();
        status
    }

    async fn owner_status_envelope(
        &self,
        submission_id: &OpaqueId,
        owner: &SigningKey,
    ) -> SubmissionOwnerStatusEnvelopeV1 {
        let challenge_request = SubmissionOwnerStatusChallengeRequestV1 {
            schema_version: SCHEMA_VERSION_V1,
            controller_public_key: protocol_public_key(owner),
            submission_id: submission_id.clone(),
        };
        let response = self
            .app
            .clone()
            .oneshot(json_request(
                Method::POST,
                "/api/v1/submission-owner-status-challenges",
                &challenge_request,
                Ipv4Addr::LOCALHOST,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        let challenge: SubmissionOwnerStatusChallengeV1 = json_body(response).await;
        challenge.validate().unwrap();
        let mut envelope = SubmissionOwnerStatusEnvelopeV1 {
            schema_version: SCHEMA_VERSION_V1,
            challenge,
            algorithm: SignatureAlgorithmV1::Ed25519,
            signature: Signature64::default(),
        };
        envelope.signature = sign(owner, &envelope.signing_bytes().unwrap());
        envelope.validate().unwrap();
        envelope
    }

    async fn campaign_receipt(
        &self,
        accepted: &SubmissionAcceptedV1,
        owner: &SigningKey,
    ) -> CampaignChainReceiptV1 {
        let status = self.owner_status(accepted, owner).await;
        let SubmissionLifecycleV1::Accepted {
            campaign_chain_receipt: Some(receipt),
            ..
        } = status.state
        else {
            panic!("campaign submission did not expose an accepted chain receipt")
        };
        receipt
    }

    async fn delete_run(&self, owner: &SigningKey, run_id: &OpaqueId) -> DeletionReceiptV1 {
        let challenge_response = self
            .app
            .clone()
            .oneshot(json_request(
                Method::POST,
                "/api/v1/deletion-challenges",
                &DeletionChallengeRequestV1 {
                    schema_version: SCHEMA_VERSION_V1,
                    public_key: protocol_public_key(owner),
                    target: DeletionTargetV1::Run {
                        run_id: run_id.clone(),
                    },
                },
                Ipv4Addr::new(127, 40, 0, 1),
            ))
            .await
            .unwrap();
        assert_eq!(challenge_response.status(), StatusCode::CREATED);
        let challenge: DeletionChallengeV1 = json_body(challenge_response).await;
        let mut deletion = DeletionRequestEnvelopeV1 {
            schema_version: SCHEMA_VERSION_V1,
            challenge,
            signature: Signature64::default(),
        };
        deletion.signature = sign(owner, &deletion.signing_bytes().unwrap());
        let response = self
            .app
            .clone()
            .oneshot(json_request(
                Method::POST,
                "/api/v1/deletion-requests",
                &deletion,
                Ipv4Addr::new(127, 40, 0, 2),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let receipt: DeletionReceiptV1 = json_body(response).await;
        receipt.validate().unwrap();
        receipt
    }

    fn campaign_offer_request(
        &self,
        key: &SigningKey,
        sequence: u8,
        mission_id: &str,
        content_subject: OfficialContentSubjectV1,
        content_manifest_sha256: Digest32,
        starting_campaign_sha256: Digest32,
        starting_campaign_byte_length: u64,
        scope_request: ScopeRequestV1,
    ) -> SubmissionOfferRequestV1 {
        let public_key = protocol_public_key(key);
        let participant_instance_id = Digest32::from_bytes([sequence; 32]);
        let mut session_genesis = ReplaySessionGenesisV1 {
            claim: ReplaySessionGenesisClaimV1 {
                schema_version: SCHEMA_VERSION_V1,
                network_protocol_version: NETWORK_PROTOCOL_VERSION,
                host_public_key: public_key,
                replay_session_id: Digest32::from_bytes([sequence.wrapping_add(40); 32]),
                host_participant_instance_id: participant_instance_id,
                host_nonce: ChallengeNonce32::from_bytes([sequence.wrapping_add(80); 32]),
                ranked_session: robin_run_protocol::RankedSessionConfigV1 {
                    custom_rules_config: None,
                    custom_canonical_campaign: None,
                    schema_version: SCHEMA_VERSION_V1,
                    mission_id: mission_id.to_owned(),
                    content_edition: OfficialContentEditionV1::Full,
                    content_subject,
                    simulation_seed: SimulationSeed64::new(u64::from(sequence)),
                    starting_campaign_sha256,
                    starting_campaign_byte_length,
                    prepared_inputs_projection_sha256: Digest32::from_bytes(
                        [sequence.wrapping_add(1); 32],
                    ),
                    prepared_mission_inputs_seal_sha256: Digest32::from_bytes(
                        [sequence.wrapping_add(2); 32],
                    ),
                    build_manifest_sha256: self.build_sha256,
                    content_manifest_sha256,
                    campaign_content_manifest_sha256: self.campaign_content_sha256,
                    rules_config_sha256: self.rules_config_sha256,
                    ruleset_manifest_sha256: self.ruleset_sha256,
                    competition_manifest_sha256: None,
                    spellforge_content_sha256: None,
                    resource_locale_root: ResourceLocaleRootV1::new("2047").unwrap(),
                    speech_timing: SpeechTimingAuthorityV1::BaseInstallation,
                },
                fresh_run_preflight_grant: None,
                campaign_continuation_preflight_grant: None,
                competition_run_grant: None,
            },
            algorithm: SignatureAlgorithmV1::Ed25519,
            host_signature: Signature64::default(),
        };
        session_genesis.host_signature = sign(key, &session_genesis.signing_bytes().unwrap());

        SubmissionOfferRequestV1 {
            schema_version: SCHEMA_VERSION_V1,
            max_concurrent_players: 1,
            participant_instance_count: 1,
            participant_claims: vec![ParticipantClaimV1 {
                seat: 0,
                participant_instance_id,
                public_key,
                public_disclosure: ParticipantPublicDisclosureV1::NamedProfile,
                join_attestation: None,
            }],
            session_genesis,
            mission_id: mission_id.to_owned(),
            scope_request,
            ruleset_manifest_sha256: self.ruleset_sha256,
            competition_manifest_sha256: None,
        }
    }

    fn offer_request(&self, key: &SigningKey, sequence: u8) -> SubmissionOfferRequestV1 {
        self.individual_offer_request(key, sequence, None)
    }

    async fn competition_offer_request(
        &self,
        key: &SigningKey,
        sequence: u8,
    ) -> SubmissionOfferRequestV1 {
        let mut offer = self.individual_offer_request(
            key,
            sequence,
            Some(self.competition_sha256.expect("competition fixture")),
        );
        let mut grant_request = CompetitionRunGrantRequestV1 {
            claim: CompetitionRunGrantRequestClaimV1 {
                schema_version: SCHEMA_VERSION_V1,
                request_nonce: ChallengeNonce32::from_bytes([sequence.wrapping_add(100); 32]),
                host_public_key: offer.session_genesis.claim.host_public_key,
                replay_session_id: offer.session_genesis.claim.replay_session_id,
                host_participant_instance_id: offer
                    .session_genesis
                    .claim
                    .host_participant_instance_id,
                host_nonce: offer.session_genesis.claim.host_nonce,
                ranked_session: offer.session_genesis.claim.ranked_session.clone(),
            },
            algorithm: SignatureAlgorithmV1::Ed25519,
            host_signature: Signature64::default(),
        };
        grant_request.host_signature = sign(key, &grant_request.signing_bytes().unwrap());
        grant_request.validate().unwrap();
        let response = self
            .app
            .clone()
            .oneshot(json_request(
                Method::POST,
                "/api/v1/competition-run-grants",
                &grant_request,
                Ipv4Addr::new(127, 9, 0, sequence),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        let grant: CompetitionRunGrantV1 = json_body(response).await;
        grant.validate_request(&grant_request).unwrap();
        offer.session_genesis.claim.competition_run_grant = Some(grant);
        offer.session_genesis.host_signature =
            sign(key, &offer.session_genesis.signing_bytes().unwrap());
        self.authorize_fresh_request(key, &mut offer, sequence)
            .await;
        offer
    }

    fn individual_offer_request(
        &self,
        key: &SigningKey,
        sequence: u8,
        competition_manifest_sha256: Option<Digest32>,
    ) -> SubmissionOfferRequestV1 {
        let public_key = protocol_public_key(key);
        let participant_instance_id = Digest32::from_bytes([sequence; 32]);
        let mut session_genesis = ReplaySessionGenesisV1 {
            claim: ReplaySessionGenesisClaimV1 {
                schema_version: SCHEMA_VERSION_V1,
                network_protocol_version: NETWORK_PROTOCOL_VERSION,
                host_public_key: public_key,
                replay_session_id: Digest32::from_bytes([sequence.wrapping_add(40); 32]),
                host_participant_instance_id: participant_instance_id,
                host_nonce: ChallengeNonce32::from_bytes([sequence.wrapping_add(80); 32]),
                ranked_session: robin_run_protocol::RankedSessionConfigV1 {
                    custom_rules_config: None,
                    custom_canonical_campaign: None,
                    schema_version: SCHEMA_VERSION_V1,
                    mission_id: MISSION_ID.to_owned(),
                    content_edition: OfficialContentEditionV1::Demo,
                    content_subject: OfficialContentSubjectV1::FieldMission {
                        mission_id: MISSION_ID.to_owned(),
                    },
                    simulation_seed: competition_manifest_sha256.map_or_else(
                        || SimulationSeed64::new(u64::from(sequence)),
                        |_| SimulationSeed64::new(777),
                    ),
                    starting_campaign_sha256: self.starting_campaign_sha256,
                    starting_campaign_byte_length: self.starting_campaign.len() as u64,
                    prepared_inputs_projection_sha256: Digest32::from_bytes(
                        [sequence.wrapping_add(1); 32],
                    ),
                    prepared_mission_inputs_seal_sha256: Digest32::from_bytes(
                        [sequence.wrapping_add(2); 32],
                    ),
                    build_manifest_sha256: self.build_sha256,
                    content_manifest_sha256: self.content_sha256,
                    campaign_content_manifest_sha256: None,
                    rules_config_sha256: self.rules_config_sha256,
                    ruleset_manifest_sha256: self.ruleset_sha256,
                    competition_manifest_sha256,
                    spellforge_content_sha256: None,
                    resource_locale_root: ResourceLocaleRootV1::new("1033").unwrap(),
                    speech_timing: SpeechTimingAuthorityV1::BaseInstallation,
                },
                fresh_run_preflight_grant: None,
                campaign_continuation_preflight_grant: None,
                competition_run_grant: None,
            },
            algorithm: SignatureAlgorithmV1::Ed25519,
            host_signature: Signature64::default(),
        };
        session_genesis.host_signature = sign(key, &session_genesis.signing_bytes().unwrap());

        SubmissionOfferRequestV1 {
            schema_version: SCHEMA_VERSION_V1,
            max_concurrent_players: 1,
            participant_instance_count: 1,
            participant_claims: vec![ParticipantClaimV1 {
                seat: 0,
                participant_instance_id,
                public_key,
                public_disclosure: ParticipantPublicDisclosureV1::NamedProfile,
                join_attestation: None,
            }],
            session_genesis,
            mission_id: MISSION_ID.to_owned(),
            scope_request: ScopeRequestV1::IndividualLevel,
            ruleset_manifest_sha256: self.ruleset_sha256,
            competition_manifest_sha256,
        }
    }
}

#[tokio::test]
async fn red_storage_keeps_health_live_and_does_not_issue_or_consume_admission() {
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

    let mut request = rig.offer_request(&owner, 41);
    rig.authorize_fresh_request(&owner, &mut request, 41).await;
    let denied_offer = red_app
        .clone()
        .oneshot(json_request(
            Method::POST,
            "/api/v1/submission-offers",
            &request,
            Ipv4Addr::new(127, 0, 4, 2),
        ))
        .await
        .unwrap();
    assert_eq!(denied_offer.status(), StatusCode::SERVICE_UNAVAILABLE);

    let offer = rig
        .issue_offer(&owner, rig.offer_request(&owner, 42), 42)
        .await;
    let replay = compact_replay_fixture("capacity-retry");
    let envelope = SubmissionEnvelopeV1 {
        schema_version: SCHEMA_VERSION_V1,
        replay_session_transcript: replay_session_transcript(&offer),
        artifacts: submission_artifacts(&replay, &rig.starting_campaign),
        offer,
        campaign_aggregation_consent: CampaignAggregationConsentV1::NotAuthorized,
        campaign_continuation_authorization: None,
        requested_metrics: vec![BoardMetricV1::OriginalScore],
    };
    let signed = SignedSubmissionV1 {
        schema_version: SCHEMA_VERSION_V1,
        algorithm: SignatureAlgorithmV1::Ed25519,
        participant_signatures: vec![ParticipantSignatureV1 {
            public_key: protocol_public_key(&owner),
            signature: sign(&owner, &envelope.signing_bytes().unwrap()),
        }],
        submission: envelope,
    };
    signed.validate().unwrap();

    let denied_upload = red_app
        .oneshot(multipart_request(&signed, &replay, &rig.starting_campaign))
        .await
        .unwrap();
    assert_eq!(denied_upload.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        sqlx::query_scalar::<_, Option<i64>>(
            "SELECT consumed_at_ms FROM upload_challenges WHERE id = ?"
        )
        .bind(signed.submission.offer.upload_challenge_id.as_str())
        .fetch_one(rig.database.pool())
        .await
        .unwrap(),
        None,
        "red admission consumed the upload challenge"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM submission_upload_reservations")
            .fetch_one(rig.database.pool())
            .await
            .unwrap(),
        0
    );

    let accepted_retry = rig
        .app
        .clone()
        .oneshot(multipart_request(&signed, &replay, &rig.starting_campaign))
        .await
        .unwrap();
    assert_eq!(accepted_retry.status(), StatusCode::ACCEPTED);
}

#[tokio::test]
async fn missing_authenticated_backup_blocks_offers_and_fresh_upload_reservations() {
    let rig = TestRig::new().await;
    let owner = SigningKey::from_bytes(&[0x6e; 32]);
    rig.rename(&owner, "Backup Robin", Ipv4Addr::new(127, 0, 5, 1))
        .await;
    let offer_request = rig.offer_request(&owner, 51);
    let offer = rig.issue_offer(&owner, offer_request, 51).await;
    let replay = compact_replay_fixture("backup-retry");
    let envelope = SubmissionEnvelopeV1 {
        schema_version: SCHEMA_VERSION_V1,
        replay_session_transcript: replay_session_transcript(&offer),
        artifacts: submission_artifacts(&replay, &rig.starting_campaign),
        offer,
        campaign_aggregation_consent: CampaignAggregationConsentV1::NotAuthorized,
        campaign_continuation_authorization: None,
        requested_metrics: vec![BoardMetricV1::OriginalScore],
    };
    let signed = SignedSubmissionV1 {
        schema_version: SCHEMA_VERSION_V1,
        algorithm: SignatureAlgorithmV1::Ed25519,
        participant_signatures: vec![ParticipantSignatureV1 {
            public_key: protocol_public_key(&owner),
            signature: sign(&owner, &envelope.signing_bytes().unwrap()),
        }],
        submission: envelope,
    };
    signed.validate().unwrap();

    let red_app =
        rig.app_with_backup_status(rig.config.database_path.with_file_name("missing.json"));
    let mut denied_offer_request = rig.offer_request(&owner, 52);
    rig.authorize_fresh_request(&owner, &mut denied_offer_request, 52)
        .await;
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
    let denied_offer = red_app
        .clone()
        .oneshot(json_request(
            Method::POST,
            "/api/v1/submission-offers",
            &denied_offer_request,
            Ipv4Addr::new(127, 0, 5, 2),
        ))
        .await
        .unwrap();
    assert_eq!(denied_offer.status(), StatusCode::SERVICE_UNAVAILABLE);
    let denied_upload = red_app
        .oneshot(multipart_request(&signed, &replay, &rig.starting_campaign))
        .await
        .unwrap();
    assert_eq!(denied_upload.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        sqlx::query_scalar::<_, Option<i64>>(
            "SELECT consumed_at_ms FROM upload_challenges WHERE id = ?"
        )
        .bind(signed.submission.offer.upload_challenge_id.as_str())
        .fetch_one(rig.database.pool())
        .await
        .unwrap(),
        None
    );
}

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

#[tokio::test]
async fn signed_rename_offer_upload_status_and_verified_publication_cross_the_real_router() {
    let rig = TestRig::new().await;
    let owner = SigningKey::from_bytes(&[7; 32]);
    let owner_key = protocol_public_key(&owner);

    assert_eq!(
        rig.rename(&owner, "Robin", Ipv4Addr::new(127, 0, 0, 7))
            .await
            .username,
        "Robin"
    );
    assert_eq!(
        rig.rename(&owner, "Robin of Locksley", Ipv4Addr::new(127, 0, 0, 8))
            .await
            .username,
        "Robin of Locksley"
    );

    let history = sqlx::query(
        "SELECT previous_username, new_username FROM username_history \
         WHERE public_key = ? ORDER BY generation",
    )
    .bind(owner_key.as_bytes().as_slice())
    .fetch_all(rig.database.pool())
    .await
    .unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(
        history[0].get::<Option<String>, _>("previous_username"),
        None
    );
    assert_eq!(history[0].get::<String, _>("new_username"), "Robin");
    assert_eq!(history[1].get::<String, _>("previous_username"), "Robin");
    assert_eq!(
        history[1].get::<String, _>("new_username"),
        "Robin of Locksley"
    );

    let profile_response = rig
        .app
        .clone()
        .oneshot(empty_request(
            Method::GET,
            &format!("/api/v1/players/{owner_key}"),
            Ipv4Addr::LOCALHOST,
        ))
        .await
        .unwrap();
    assert_eq!(profile_response.status(), StatusCode::OK);
    assert_dynamic_headers(&profile_response);
    let profile: PlayerProfileV1 = json_body(profile_response).await;
    assert_eq!(profile.username, "Robin of Locksley");

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
    assert_dynamic_headers(&metadata_response);
    let metadata: LeaderboardMetadataV1 = json_body(metadata_response).await;
    metadata.validate().unwrap();
    assert_eq!(metadata.missions.len(), 1);
    assert_eq!(metadata.missions[0].mission_id, MISSION_ID);

    let accepted = rig.submit(&owner, 1).await;
    let queued = rig.owner_status(&accepted, &owner).await;
    queued.validate().unwrap();
    assert_eq!(queued.state, SubmissionLifecycleV1::Queued);

    let run_id = rig.publish(&accepted, 12_345, 21).await;
    let private_participant_instance_id = Digest32::from_bytes([1; 32]);
    let private_completion_digest = Digest32::from_bytes([0xc7; 32]);
    sqlx::query("UPDATE verified_runs SET campaign_complete_evidence_sha256 = ? WHERE id = ?")
        .bind(private_completion_digest.as_bytes().as_slice())
        .bind(run_id.as_str())
        .execute(rig.database.pool())
        .await
        .unwrap();
    let corrupted_evidence = sqlx::query(
        "UPDATE verified_run_achievements \
         SET evidence_json = ? WHERE run_id = ?",
    )
    .bind(r#"{"PRIVATE_ACHIEVEMENT_EVIDENCE_SENTINEL":1.5}"#)
    .bind(run_id.as_str())
    .execute(rig.database.pool())
    .await
    .unwrap();
    assert!(corrupted_evidence.rows_affected() > 0);
    let public_projection_storage = sqlx::query(
        "SELECT public_verification_request_json, public_verification_result_json, \
                public_projection_binding_json FROM verified_runs WHERE id = ?",
    )
    .bind(run_id.as_str())
    .fetch_one(rig.database.pool())
    .await
    .unwrap();
    for column in [
        "public_verification_request_json",
        "public_verification_result_json",
        "public_projection_binding_json",
    ] {
        let stored: String = public_projection_storage.get(column);
        assert_public_json_omits_private_participant_id(
            stored.as_bytes(),
            private_participant_instance_id,
        );
    }
    let accepted_status = rig.owner_status(&accepted, &owner).await;
    accepted_status.validate().unwrap();
    assert_eq!(
        accepted_status.state,
        SubmissionLifecycleV1::Accepted {
            run_id: run_id.clone(),
            campaign_chain_receipt: None,
        }
    );

    let attacker = SigningKey::from_bytes(&[8; 32]);
    let missing_submission = OpaqueId::new("00000000-0000-7000-8000-000000000001").unwrap();
    let wrong_key_envelope = rig
        .owner_status_envelope(&accepted.submission_id, &attacker)
        .await;
    let missing_envelope = rig
        .owner_status_envelope(&missing_submission, &attacker)
        .await;
    let wrong_key_response = rig
        .app
        .clone()
        .oneshot(json_request(
            Method::POST,
            &format!(
                "/api/v1/submissions/{}/private-status",
                accepted.submission_id
            ),
            &wrong_key_envelope,
            Ipv4Addr::LOCALHOST,
        ))
        .await
        .unwrap();
    let missing_response = rig
        .app
        .clone()
        .oneshot(json_request(
            Method::POST,
            &format!("/api/v1/submissions/{missing_submission}/private-status"),
            &missing_envelope,
            Ipv4Addr::LOCALHOST,
        ))
        .await
        .unwrap();
    assert_eq!(wrong_key_response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(missing_response.status(), StatusCode::UNAUTHORIZED);
    let wrong_key_body = wrong_key_response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    let missing_body = missing_response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    assert_eq!(wrong_key_body, missing_body);

    let replayed_response = rig
        .app
        .clone()
        .oneshot(json_request(
            Method::POST,
            &format!(
                "/api/v1/submissions/{}/private-status",
                accepted.submission_id
            ),
            &wrong_key_envelope,
            Ipv4Addr::LOCALHOST,
        ))
        .await
        .unwrap();
    assert_eq!(replayed_response.status(), StatusCode::UNAUTHORIZED);

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
    assert_dynamic_headers(&detail_response);
    let detail_body = detail_response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    assert_public_json_omits_private_participant_id(&detail_body, private_participant_instance_id);
    assert!(
        !String::from_utf8_lossy(&detail_body).contains("PRIVATE_ACHIEVEMENT_EVIDENCE_SENTINEL")
    );
    assert!(
        !String::from_utf8_lossy(&detail_body)
            .contains(&hex::encode(private_completion_digest.as_bytes()))
    );
    let detail: RunDetailV1 = serde_json::from_slice(&detail_body).unwrap();
    detail
        .validate_against_ruleset(&rig.published_ruleset, None)
        .unwrap();
    assert_eq!(detail.run_id, run_id);
    assert_eq!(detail.metrics.original_score_delta, 12_345);
    assert_eq!(detail.named_participants[0].username, "Robin of Locksley");
    assert!(detail.verification_proof.is_some());
    assert_public_projection_is_redacted(&detail);

    let replay_response = rig
        .app
        .clone()
        .oneshot(empty_request(
            Method::GET,
            &format!("/api/v1/runs/{run_id}/replay"),
            Ipv4Addr::LOCALHOST,
        ))
        .await
        .unwrap();
    assert_eq!(replay_response.status(), StatusCode::OK);
    assert_dynamic_headers(&replay_response);
    assert_eq!(
        replay_response.headers().get(CONTENT_TYPE).unwrap(),
        RANKED_REPLAY_MEDIA_TYPE_V1
    );
    let replay = replay_response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    assert_eq!(replay, compact_replay_fixture("run-1"));

    let missing_response = rig
        .app
        .clone()
        .oneshot(empty_request(
            Method::GET,
            "/api/v1/runs/00000000-0000-7000-8000-000000000000",
            Ipv4Addr::LOCALHOST,
        ))
        .await
        .unwrap();
    assert_eq!(missing_response.status(), StatusCode::NOT_FOUND);
    assert_dynamic_headers(&missing_response);

    let immutable_response = rig
        .app
        .clone()
        .oneshot(empty_request(
            Method::GET,
            &format!("/api/v1/builds/{}", rig.build_sha256),
            Ipv4Addr::LOCALHOST,
        ))
        .await
        .unwrap();
    assert_eq!(immutable_response.status(), StatusCode::OK);
    assert_eq!(
        immutable_response.headers().get(CACHE_CONTROL).unwrap(),
        "public, max-age=31536000, immutable"
    );
    assert_eq!(
        immutable_response
            .headers()
            .get(X_CONTENT_TYPE_OPTIONS)
            .unwrap(),
        "nosniff"
    );
    let immutable_body = immutable_response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    assert_eq!(
        immutable_body.as_ref(),
        rig.build.canonical_bytes().unwrap().as_slice()
    );
    assert_eq!(Digest32::digest_bytes(&immutable_body), rig.build_sha256);
}

#[tokio::test]
async fn truncated_three_part_multipart_fails_before_creating_submission_state() {
    let rig = TestRig::new().await;
    const BOUNDARY: &str = "robin-truncated-boundary";
    let body = format!(
        "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"submission\"\r\nContent-Type: application/json\r\n\r\n{{"
    );
    let response = rig
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/v1/submissions")
                .header(
                    CONTENT_TYPE,
                    format!("multipart/form-data; boundary={BOUNDARY}"),
                )
                .extension(peer(Ipv4Addr::LOCALHOST))
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        rig.database
            .operational_counts()
            .await
            .unwrap()
            .queued_submissions,
        0
    );
}

#[tokio::test]
async fn authenticated_submission_rejects_shape_signature_and_offer_before_reservation() {
    let rig = TestRig::new().await;
    let owner = SigningKey::from_bytes(&[93; 32]);
    rig.rename(&owner, "Authentication Robin", Ipv4Addr::new(127, 0, 9, 3))
        .await;
    let offer = rig
        .issue_offer(&owner, rig.offer_request(&owner, 93), 93)
        .await;
    let replay = compact_replay_fixture("authentication-93");
    let signed = signed_submission(&owner, offer, &replay, &rig.starting_campaign);

    let mut malformed = signed.clone();
    malformed.participant_signatures.clear();
    let mut invalid_signature = signed.clone();
    invalid_signature.participant_signatures[0].signature = sign(
        &SigningKey::from_bytes(&[94; 32]),
        &signed.signing_bytes().unwrap(),
    );
    invalid_signature.validate().unwrap();
    // A legal independent offer mutation preserves document shape, but is not
    // the exact server-issued offer. It must reject as conflict before crypto.
    let mut different_offer = signed.clone();
    different_offer.submission.offer.upload_challenge_nonce =
        robin_run_protocol::ChallengeNonce32::from_bytes([95; 32]);
    different_offer.validate().unwrap();
    for (candidate, status) in [
        (&malformed, StatusCode::BAD_REQUEST),
        (&invalid_signature, StatusCode::UNAUTHORIZED),
        (&different_offer, StatusCode::CONFLICT),
    ] {
        let response = rig
            .app
            .clone()
            .oneshot(metadata_only_multipart_request(candidate))
            .await
            .unwrap();
        assert_eq!(response.status(), status);
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM submission_upload_reservations")
            .fetch_one(rig.database.pool())
            .await
            .unwrap();
        assert_eq!(count, 0);
    }
}

#[tokio::test]
async fn reserved_upload_failures_abandon_lease_and_exact_retry_finalizes_once() {
    for extra_field in [false, true] {
        let rig = TestRig::new().await;
        let owner = SigningKey::from_bytes(&[91; 32]);
        rig.rename(&owner, "Workflow Robin", Ipv4Addr::new(127, 0, 9, 1))
            .await;
        let offer = rig
            .issue_offer(&owner, rig.offer_request(&owner, 91), 91)
            .await;
        let replay = compact_replay_fixture("workflow-91");
        let signed = signed_submission(&owner, offer, &replay, &rig.starting_campaign);
        let request = multipart_request(&signed, &replay, &rig.starting_campaign);
        let (parts, body) = request.into_parts();
        let mut bytes = body.collect().await.unwrap().to_bytes().to_vec();
        const END: &[u8] = b"\r\n--robin-router-e2e-boundary--\r\n";
        assert!(bytes.ends_with(END));
        if extra_field {
            bytes.truncate(bytes.len() - END.len());
            bytes.extend_from_slice(b"\r\n--robin-router-e2e-boundary\r\nContent-Disposition: form-data; name=\"extra\"\r\n\r\nforbidden\r\n--robin-router-e2e-boundary--\r\n");
        } else {
            // Valid authenticated replay, followed by an interrupted campaign.
            bytes.truncate(bytes.len() - END.len() - 1);
        }
        let response = rig
            .app
            .clone()
            .oneshot(Request::from_parts(parts, Body::from(bytes)))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let state: (String, Option<String>) = sqlx::query_as(
            "SELECT state, lease_token FROM submission_upload_reservations WHERE upload_challenge_id = ?",
        )
        .bind(signed.submission.offer.upload_challenge_id.as_str())
        .fetch_one(rig.database.pool()).await.unwrap();
        assert_eq!(state, ("abandoned".to_owned(), None));
        let submissions: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM submissions")
            .fetch_one(rig.database.pool())
            .await
            .unwrap();
        assert_eq!(submissions, 0, "partial artifacts must never be finalized");

        // Model a concurrent exact retry holding the reservation. A valid replay
        // with deliberately wrong campaign bytes must stop at Busy, without
        // invoking campaign ingestion (which would return BadRequest).
        sqlx::query(
            "UPDATE submission_upload_reservations SET state = 'reserved', \
             lease_token = ?, lease_expires_at_ms = reservation_expires_at_ms, \
             abandoned_at_ms = NULL WHERE upload_challenge_id = ?",
        )
        .bind(uuid::Uuid::now_v7().to_string())
        .bind(signed.submission.offer.upload_challenge_id.as_str())
        .execute(rig.database.pool())
        .await
        .unwrap();
        let busy = rig
            .app
            .clone()
            .oneshot(multipart_request(&signed, &replay, b"not the campaign"))
            .await
            .unwrap();
        assert_eq!(busy.status(), StatusCode::CONFLICT);
        let busy_body: serde_json::Value = json_body(busy).await;
        assert_eq!(busy_body["error"]["code"], "upload_in_progress");
        sqlx::query(
            "UPDATE submission_upload_reservations SET state = 'abandoned', \
             lease_token = NULL, lease_expires_at_ms = NULL, \
             abandoned_at_ms = updated_at_ms WHERE upload_challenge_id = ?",
        )
        .bind(signed.submission.offer.upload_challenge_id.as_str())
        .execute(rig.database.pool())
        .await
        .unwrap();

        // Cleanup releases the lease, not the immutable signed identity. An
        // exact retry may reuse content-addressed artifacts and finalize once.
        for _ in 0..2 {
            let response = rig
                .app
                .clone()
                .oneshot(multipart_request(&signed, &replay, &rig.starting_campaign))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::ACCEPTED);
        }
        let committed = rig
            .app
            .clone()
            .oneshot(multipart_request(&signed, &replay, b"not the campaign"))
            .await
            .unwrap();
        assert_eq!(
            committed.status(),
            StatusCode::ACCEPTED,
            "committed retries do not ingest campaign bytes"
        );
        let submissions: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM submissions")
            .fetch_one(rig.database.pool())
            .await
            .unwrap();
        assert_eq!(submissions, 1);
        let persisted = sqlx::query(
            "SELECT s.envelope_json, s.controller_public_key, s.session_genesis_sha256, \
             r.envelope_json AS reserved_envelope, r.controller_public_key AS reserved_controller, \
             r.session_genesis_sha256 AS reserved_genesis \
             FROM submissions s JOIN submission_upload_reservations r \
             ON r.submission_id = s.id",
        )
        .fetch_one(rig.database.pool())
        .await
        .unwrap();
        assert_eq!(
            persisted.get::<String, _>("envelope_json"),
            serde_json::to_string(&signed).unwrap()
        );
        assert_eq!(
            persisted.get::<String, _>("envelope_json"),
            persisted.get::<String, _>("reserved_envelope")
        );
        for (final_column, reserved_column) in [
            ("controller_public_key", "reserved_controller"),
            ("session_genesis_sha256", "reserved_genesis"),
        ] {
            assert_eq!(
                persisted.get::<Vec<u8>, _>(final_column),
                persisted.get::<Vec<u8>, _>(reserved_column)
            );
        }
    }
}

#[tokio::test]
async fn submission_ingress_accepts_only_the_exact_compact_transport_before_reservation() {
    let rig = TestRig::new().await;
    let owner = SigningKey::from_bytes(&[72; 32]);
    rig.rename(&owner, "Compact Robin", Ipv4Addr::new(127, 0, 2, 0))
        .await;

    let valid_replay = compact_replay_fixture("run-72");
    let valid_offer = rig
        .issue_offer(&owner, rig.offer_request(&owner, 72), 72)
        .await;
    let valid_signed =
        signed_submission(&owner, valid_offer, &valid_replay, &rig.starting_campaign);
    let valid_challenge = valid_signed.submission.offer.upload_challenge_id.as_str();

    let mut encoded = multipart_request(&valid_signed, &valid_replay, &rig.starting_campaign);
    encoded.headers_mut().insert(
        axum::http::header::CONTENT_ENCODING,
        axum::http::HeaderValue::from_static("identity"),
    );
    let encoded_response = rig.app.clone().oneshot(encoded).await.unwrap();
    assert_eq!(encoded_response.status(), StatusCode::BAD_REQUEST);
    let wrong_media_response = rig
        .app
        .clone()
        .oneshot(multipart_request_with_replay_transport(
            &valid_signed,
            &valid_replay,
            &rig.starting_campaign,
            "replay",
            "application/json",
        ))
        .await
        .unwrap();
    assert_eq!(wrong_media_response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        sqlx::query_scalar::<_, Option<i64>>(
            "SELECT consumed_at_ms FROM upload_challenges WHERE id = ?",
        )
        .bind(valid_challenge)
        .fetch_one(rig.database.pool())
        .await
        .unwrap(),
        None,
        "transport encoding or media-lane rejection consumed the one-use challenge"
    );

    let build_hash = robin_replay_format::ENGINE_VERSION_HASH;
    let invalid_replays = [
        (
            74,
            "local JSONL recorder format",
            b"{\"schema_version\":29,\"not_compact\":true}\n".to_vec(),
        ),
        (
            75,
            "missing compact prefix",
            format!("replay-{build_hash}-YWJj").into_bytes(),
        ),
        (
            76,
            "padded base64 variant",
            format!("rhrec-{build_hash}-YWJj=").into_bytes(),
        ),
        (
            77,
            "non-base64url alphabet",
            format!("rhrec-{build_hash}-YWJ+").into_bytes(),
        ),
        (
            78,
            "wrong build prefix",
            b"rhrec-000000000000-YWJj".to_vec(),
        ),
        (
            79,
            "non-canonical base64url tail bits",
            format!("rhrec-{build_hash}-AB").into_bytes(),
        ),
    ];
    for (sequence, label, replay) in invalid_replays {
        let offer = rig
            .issue_offer(&owner, rig.offer_request(&owner, sequence), sequence)
            .await;
        let signed = signed_submission(&owner, offer, &replay, &rig.starting_campaign);
        let challenge_id = signed
            .submission
            .offer
            .upload_challenge_id
            .as_str()
            .to_owned();
        let replay_digest = signed.submission.artifacts.replay.artifact.sha256;
        let response = rig
            .app
            .clone()
            .oneshot(multipart_request(&signed, &replay, &rig.starting_campaign))
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "{label} was admitted"
        );
        assert_eq!(
            sqlx::query_scalar::<_, Option<i64>>(
                "SELECT consumed_at_ms FROM upload_challenges WHERE id = ?",
            )
            .bind(&challenge_id)
            .fetch_one(rig.database.pool())
            .await
            .unwrap(),
            None,
            "{label} consumed the one-use challenge"
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM submission_upload_reservations WHERE upload_challenge_id = ?",
            )
            .bind(&challenge_id)
            .fetch_one(rig.database.pool())
            .await
            .unwrap(),
            0,
            "{label} created a durable upload reservation"
        );
        assert!(
            !rig.replay_store
                .path_for_digest(replay_digest.as_bytes())
                .exists(),
            "{label} reached durable replay storage"
        );
    }
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM submissions")
            .fetch_one(rig.database.pool())
            .await
            .unwrap(),
        0
    );
    assert!(
        rig.database
            .lease_next("lexical-rejection-probe", Duration::from_secs(60))
            .await
            .unwrap()
            .is_none(),
        "a rejected transport created a verifier queue job"
    );

    let accepted_response = rig
        .app
        .clone()
        .oneshot(multipart_request(
            &valid_signed,
            &valid_replay,
            &rig.starting_campaign,
        ))
        .await
        .unwrap();
    assert_eq!(accepted_response.status(), StatusCode::ACCEPTED);
    let accepted: SubmissionAcceptedV1 = json_body(accepted_response).await;
    assert_eq!(accepted.state, SubmissionLifecycleV1::Queued);
    assert!(
        sqlx::query_scalar::<_, Option<i64>>(
            "SELECT consumed_at_ms FROM upload_challenges WHERE id = ?",
        )
        .bind(valid_challenge)
        .fetch_one(rig.database.pool())
        .await
        .unwrap()
        .is_some()
    );
    assert_eq!(
        tokio::fs::read(
            rig.replay_store
                .path_for_digest(Digest32::digest_bytes(&valid_replay).as_bytes()),
        )
        .await
        .unwrap(),
        valid_replay
    );
    let job = rig
        .database
        .lease_next("valid-compact-worker", Duration::from_secs(60))
        .await
        .unwrap()
        .expect("valid compact replay must create a verifier queue job");
    assert_eq!(job.submission_id, accepted.submission_id.as_str());
}

#[tokio::test]
async fn wrong_body_never_reserves_and_exact_compact_retries_queue_once() {
    let rig = TestRig::new().await;
    let owner = SigningKey::from_bytes(&[73; 32]);
    rig.rename(&owner, "Retrying Robin", Ipv4Addr::new(127, 0, 2, 1))
        .await;
    let sequence = 73;
    let offer = rig
        .issue_offer(&owner, rig.offer_request(&owner, sequence), sequence)
        .await;
    let replay = compact_replay_fixture("retry-73");
    let envelope = SubmissionEnvelopeV1 {
        schema_version: SCHEMA_VERSION_V1,
        replay_session_transcript: replay_session_transcript(&offer),
        offer,
        artifacts: submission_artifacts(&replay, &rig.starting_campaign),
        campaign_aggregation_consent: CampaignAggregationConsentV1::NotAuthorized,
        campaign_continuation_authorization: None,
        requested_metrics: vec![BoardMetricV1::OriginalScore],
    };
    let signed = SignedSubmissionV1 {
        schema_version: SCHEMA_VERSION_V1,
        algorithm: SignatureAlgorithmV1::Ed25519,
        participant_signatures: vec![ParticipantSignatureV1 {
            public_key: protocol_public_key(&owner),
            signature: sign(&owner, &envelope.signing_bytes().unwrap()),
        }],
        submission: envelope,
    };
    signed.validate().unwrap();

    for legacy_field in ["private_replay", "public_replay"] {
        let response = rig
            .app
            .clone()
            .oneshot(multipart_request_with_replay_field(
                &signed,
                &replay,
                &rig.starting_campaign,
                legacy_field,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM submissions")
                .fetch_one(rig.database.pool())
                .await
                .unwrap(),
            0,
            "legacy multipart field {legacy_field} must fail closed"
        );
    }

    let partial = rig
        .app
        .clone()
        .oneshot(metadata_only_multipart_request(&signed))
        .await
        .unwrap();
    assert_eq!(partial.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM submissions")
            .fetch_one(rig.database.pool())
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM submission_upload_reservations WHERE upload_challenge_id = ?",
        )
        .bind(signed.submission.offer.upload_challenge_id.as_str())
        .fetch_one(rig.database.pool())
        .await
        .unwrap(),
        0,
        "a missing replay field must fail before reservation"
    );
    let mut wrong_replay = replay.clone();
    wrong_replay[0] ^= 1;
    let failed = rig
        .app
        .clone()
        .oneshot(multipart_request(
            &signed,
            &wrong_replay,
            &rig.starting_campaign,
        ))
        .await
        .unwrap();
    let failed_status = failed.status();
    let failed_body = failed.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(
        failed_status,
        StatusCode::BAD_REQUEST,
        "wrong-body response: {}",
        String::from_utf8_lossy(&failed_body)
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM submissions")
            .fetch_one(rig.database.pool())
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM submission_upload_reservations WHERE upload_challenge_id = ?",
        )
        .bind(signed.submission.offer.upload_challenge_id.as_str())
        .fetch_one(rig.database.pool())
        .await
        .unwrap(),
        0,
        "a digest or lexical mismatch must fail before reservation"
    );
    assert_eq!(
        sqlx::query_scalar::<_, Option<i64>>(
            "SELECT consumed_at_ms FROM upload_challenges WHERE id = ?",
        )
        .bind(signed.submission.offer.upload_challenge_id.as_str())
        .fetch_one(rig.database.pool())
        .await
        .unwrap(),
        None,
        "a rejected replay body consumed its challenge"
    );
    let replay_digest = signed
        .submission
        .artifacts
        .replay
        .artifact
        .sha256
        .into_bytes();
    assert!(!rig.replay_store.path_for_digest(&replay_digest).exists());
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM replay_objects WHERE sha256 = ?")
            .bind(replay_digest.as_slice())
            .fetch_one(rig.database.pool())
            .await
            .unwrap(),
        0,
        "a failed stream must not register verifier-visible storage"
    );

    let accepted_response = rig
        .app
        .clone()
        .oneshot(multipart_request(&signed, &replay, &rig.starting_campaign))
        .await
        .unwrap();
    assert_eq!(accepted_response.status(), StatusCode::ACCEPTED);
    let accepted: SubmissionAcceptedV1 = json_body(accepted_response).await;
    let stored_replay = tokio::fs::read(rig.replay_store.path_for_digest(&replay_digest))
        .await
        .unwrap();
    assert_eq!(stored_replay, replay);

    // Even a response-loss retry must still carry the exact compact transport;
    // committed state is not a content-negotiation bypass.
    let completed_retry = rig
        .app
        .clone()
        .oneshot(multipart_request(
            &signed,
            &wrong_replay,
            b"wrong campaign body",
        ))
        .await
        .unwrap();
    assert_eq!(completed_retry.status(), StatusCode::BAD_REQUEST);
    let exact_retry = rig
        .app
        .clone()
        .oneshot(multipart_request(&signed, &replay, &rig.starting_campaign))
        .await
        .unwrap();
    assert_eq!(exact_retry.status(), StatusCode::ACCEPTED);
    let retried: SubmissionAcceptedV1 = json_body(exact_retry).await;
    assert_eq!(retried.submission_id, accepted.submission_id);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM submissions")
            .fetch_one(rig.database.pool())
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        tokio::fs::read(rig.replay_store.path_for_digest(&replay_digest))
            .await
            .unwrap(),
        replay
    );
}

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
    transplanted.session_genesis.host_signature = sign(
        &owner,
        &transplanted.session_genesis.signing_bytes().unwrap(),
    );
    assert!(transplanted.validate().is_err());
    let mut forged_authority = authorized_request.clone();
    forged_authority
        .session_genesis
        .claim
        .competition_run_grant
        .as_mut()
        .unwrap()
        .authority_signature = Signature64::from_bytes([0xa4; 64]);
    forged_authority.session_genesis.host_signature = sign(
        &owner,
        &forged_authority.session_genesis.signing_bytes().unwrap(),
    );
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
    substituted_session.session_genesis.host_signature = sign(
        &owner,
        &substituted_session.session_genesis.signing_bytes().unwrap(),
    );
    assert!(substituted_session.validate().is_err());

    let mut forged_authority = authorized_for_adversarial_checks.clone();
    forged_authority
        .session_genesis
        .claim
        .campaign_continuation_preflight_grant
        .as_mut()
        .unwrap()
        .authority_signature = Signature64::from_bytes([0xe4; 64]);
    forged_authority.session_genesis.host_signature = sign(
        &owner,
        &forged_authority.session_genesis.signing_bytes().unwrap(),
    );
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
    expired.session_genesis.host_signature =
        sign(&owner, &expired.session_genesis.signing_bytes().unwrap());
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
    assert_eq!(expired_response.status(), StatusCode::CONFLICT);

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
    .fetch_one(rig.database.pool())
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
        .fetch_one(rig.database.pool())
        .await
        .unwrap();
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "UPDATE full_campaign_runs SET {column} = ? WHERE id = ?"
        )))
        .bind(format!(" {original}"))
        .bind(full_campaign_run_id.as_str())
        .execute(rig.database.pool())
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
        .execute(rig.database.pool())
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

async fn assert_report_status(
    rig: &TestRig,
    target: AbuseReportTargetV1,
    peer: Ipv4Addr,
    expected: StatusCode,
) {
    let response = rig
        .app
        .clone()
        .oneshot(json_request(
            Method::POST,
            "/api/v1/reports",
            &AbuseReportV1 {
                schema_version: SCHEMA_VERSION_V1,
                target,
                category: AbuseReportCategoryV1::Other,
                detail: "Router E2E moderation signal".to_owned(),
            },
            peer,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), expected);
    if expected == StatusCode::ACCEPTED {
        let accepted: AbuseReportAcceptedV1 = json_body(response).await;
        accepted.validate().unwrap();
    }
}

fn assert_public_projection_is_redacted(value: &impl Serialize) {
    let public_value = serde_json::to_value(value).unwrap();
    assert!(public_value.get("verification_result").is_none());
    let public_json = public_value.to_string();
    for forbidden in [
        "authenticated_participant_claims",
        "participant_claims",
        "canonical_campaign_state",
        "campaign_state_requirement",
        "canonical_campaign_state_json",
        "campaign_chain_receipt",
        "expected_starting_campaign",
        "controller_public_key",
        "participant_instance_id",
        "chain_id",
    ] {
        assert!(
            !public_json.contains(forbidden),
            "public JSON leaked forbidden field {forbidden}"
        );
    }
}

fn assert_public_json_omits_private_participant_id(bytes: &[u8], private_id: Digest32) {
    let json = std::str::from_utf8(bytes).unwrap();
    assert!(
        !json.contains("participant_instance_id"),
        "public JSON exposed a private participant-instance field"
    );
    let sentinel = hex::encode(private_id.as_bytes());
    let sentinel_context = json.find(&sentinel).map(|position| {
        &json[position.saturating_sub(80)..(position + sentinel.len() + 80).min(json.len())]
    });
    assert!(
        sentinel_context.is_none(),
        "public JSON exposed private participant-instance sentinel {sentinel}: {sentinel_context:?}"
    );
}

fn assert_public_json_omits_private_chain_id(bytes: &[u8], private_chain_id: &str) {
    let json = std::str::from_utf8(bytes).unwrap();
    assert!(
        !json.contains("chain_id"),
        "public JSON exposed a private campaign-chain field"
    );
    assert!(
        !json.contains(private_chain_id),
        "public JSON exposed private campaign-chain sentinel {private_chain_id}"
    );
}

fn leaderboard_uri(rig: &TestRig, limit: u16, cursor: Option<&str>) -> String {
    let mut uri = format!(
        "/api/v1/leaderboards?schema_version=1&subject_kind=mission&mission_id={MISSION_ID}\
         &mission_scope=individual_level&metric=original_score\
         &content_identity_sha256={}&rules_config_sha256={}\
         &ruleset_manifest_sha256={}&limit={limit}",
        rig.content_sha256, rig.rules_config_sha256, rig.ruleset_sha256
    );
    uri.retain(|character| !character.is_ascii_whitespace());
    if let Some(cursor) = cursor {
        uri.push_str("&cursor=");
        uri.push_str(cursor);
    }
    uri
}

fn mission_leaderboard_uri(
    rig: &TestRig,
    mission_id: &str,
    mission_scope: &str,
    metric: &str,
) -> String {
    let content = match mission_id {
        GENESIS_MISSION_ID => rig.genesis_content_sha256.unwrap(),
        HQ_MISSION_ID => rig.terminal_content_sha256.unwrap(),
        _ => rig.content_sha256,
    };
    format!(
        "/api/v1/leaderboards?schema_version=1&subject_kind=mission&mission_id={mission_id}\
         &mission_scope={mission_scope}&metric={metric}&content_identity_sha256={content}\
         &rules_config_sha256={}&ruleset_manifest_sha256={}&limit=10",
        rig.rules_config_sha256, rig.ruleset_sha256
    )
    .chars()
    .filter(|character| !character.is_ascii_whitespace())
    .collect()
}

fn campaign_mission_leaderboard_uri(rig: &TestRig, mission_id: &str) -> String {
    mission_leaderboard_uri(rig, mission_id, "campaign", "original_score")
}

fn full_campaign_leaderboard_uri(rig: &TestRig) -> String {
    format!(
        "/api/v1/leaderboards?schema_version=1&subject_kind=full_campaign&metric=original_score\
         &content_identity_sha256={}&rules_config_sha256={}\
         &ruleset_manifest_sha256={}&limit=10",
        rig.campaign_content_sha256.unwrap(),
        rig.rules_config_sha256,
        rig.ruleset_sha256
    )
    .chars()
    .filter(|character| !character.is_ascii_whitespace())
    .collect()
}

fn competition_leaderboard_uri(rig: &TestRig, competition_sha256: Digest32) -> String {
    format!(
        "/api/v1/leaderboards?schema_version=1&subject_kind=mission&mission_id={MISSION_ID}\
         &mission_scope=individual_level&metric=original_score\
         &content_identity_sha256={}&rules_config_sha256={}\
         &ruleset_manifest_sha256={}&competition_manifest_sha256={competition_sha256}\
         &max_concurrent_players=1&limit=10",
        rig.content_sha256, rig.rules_config_sha256, rig.ruleset_sha256
    )
    .chars()
    .filter(|character| !character.is_ascii_whitespace())
    .collect()
}

fn protocol_public_key(key: &SigningKey) -> PublicKey32 {
    PublicKey32::from_bytes(key.verifying_key().to_bytes())
}

fn sign(key: &SigningKey, bytes: &[u8]) -> Signature64 {
    Signature64::from_bytes(key.sign(bytes).to_bytes())
}

fn compact_replay_fixture(label: &str) -> Vec<u8> {
    use robin_engine::replay::{REPLAY_SCHEMA_VERSION, ReplayFile, ReplayHeader};
    let replay: robin_engine::replay::ReplayData = ReplayFile {
        header: ReplayHeader {
            mission_id: label.to_owned(),
            rng_seed: 1,
            sim_config: robin_engine::engine::SimConfig::default(),
            version: REPLAY_SCHEMA_VERSION,
            total_frames: 0,
            rankability: robin_engine::replay_rankability::ReplayRankability::rankable(),
            mission_assets: robin_engine::mission_assets::MissionAssetDescriptor::built_in(
                label, label, label,
            )
            .unwrap(),
            spellforge_package: None,
            campaign: bitcode::encode(&robin_engine::campaign::Campaign::default()),
        },
        frames: Default::default(),
        hashes: Default::default(),
        save_markers: Default::default(),
        load_backs: Default::default(),
    }
    .try_into()
    .expect("valid replay fixture");
    robin_replay_format::encode_compact(&replay, robin_replay_format::ENGINE_VERSION_HASH)
        .unwrap()
        .into_bytes()
}

fn replay_artifact(bytes: &[u8]) -> ReplayArtifactV1 {
    ReplayArtifactV1 {
        artifact: ArtifactRefV1 {
            sha256: Digest32::digest_bytes(bytes),
            byte_length: u64::try_from(bytes.len()).unwrap(),
            media_type: RANKED_REPLAY_MEDIA_TYPE_V1.to_owned(),
        },
        replay_schema_version: REPLAY_SCHEMA_VERSION,
    }
}

fn campaign_artifact(bytes: &[u8]) -> ArtifactRefV1 {
    ArtifactRefV1 {
        sha256: Digest32::digest_bytes(bytes),
        byte_length: u64::try_from(bytes.len()).unwrap(),
        media_type: RANKED_CAMPAIGN_MEDIA_TYPE_V1.to_owned(),
    }
}

fn submission_artifacts(replay: &[u8], starting_campaign: &[u8]) -> SubmissionArtifactsV1 {
    SubmissionArtifactsV1 {
        replay: replay_artifact(replay),
        starting_campaign: campaign_artifact(starting_campaign),
    }
}

fn signed_submission(
    key: &SigningKey,
    offer: SubmissionOfferV1,
    replay: &[u8],
    starting_campaign: &[u8],
) -> SignedSubmissionV1 {
    let envelope = SubmissionEnvelopeV1 {
        schema_version: SCHEMA_VERSION_V1,
        replay_session_transcript: replay_session_transcript(&offer),
        offer,
        artifacts: submission_artifacts(replay, starting_campaign),
        campaign_aggregation_consent: CampaignAggregationConsentV1::NotAuthorized,
        campaign_continuation_authorization: None,
        requested_metrics: vec![BoardMetricV1::OriginalScore],
    };
    let signed = SignedSubmissionV1 {
        schema_version: SCHEMA_VERSION_V1,
        algorithm: SignatureAlgorithmV1::Ed25519,
        participant_signatures: vec![ParticipantSignatureV1 {
            public_key: protocol_public_key(key),
            signature: sign(key, &envelope.signing_bytes().unwrap()),
        }],
        submission: envelope,
    };
    signed.validate().unwrap();
    signed
}

fn replay_session_transcript(offer: &SubmissionOfferV1) -> ReplaySessionTranscriptV1 {
    let host = offer.session_genesis.claim.host_participant_instance_id;
    ReplaySessionTranscriptV1 {
        schema_version: SCHEMA_VERSION_V1,
        session_genesis_sha256: offer.session_genesis.canonical_digest().unwrap(),
        replay_session_id: offer.session_genesis.claim.replay_session_id,
        host_participant_instance_id: host,
        participant_instance_count: offer.participant_instance_count,
        max_concurrent_players: offer.max_concurrent_players,
        events: vec![ReplaySeatLifecycleEventV1 {
            event_ordinal: 0,
            replay_ordinal: 0,
            seat: 0,
            participant_instance_id: host,
            lifecycle: ReplaySeatLifecycleKindV1::Connected {
                connection_epoch: 0,
            },
        }],
    }
}

fn peer(address: Ipv4Addr) -> ConnectInfo<SocketAddr> {
    ConnectInfo(SocketAddr::new(IpAddr::V4(address), 41_000))
}

fn assert_dynamic_headers(response: &axum::response::Response) {
    assert_eq!(response.headers().get(CACHE_CONTROL).unwrap(), "no-store");
    assert_eq!(
        response.headers().get(X_CONTENT_TYPE_OPTIONS).unwrap(),
        "nosniff"
    );
}

fn json_request<T: Serialize>(
    method: Method,
    uri: &str,
    value: &T,
    address: Ipv4Addr,
) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header(CONTENT_TYPE, "application/json")
        .extension(peer(address))
        .body(Body::from(serde_json::to_vec(value).unwrap()))
        .unwrap()
}

fn empty_request(method: Method, uri: &str, address: Ipv4Addr) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .extension(peer(address))
        .body(Body::empty())
        .unwrap()
}

fn multipart_request(
    signed: &SignedSubmissionV1,
    replay: &[u8],
    starting_campaign: &[u8],
) -> Request<Body> {
    multipart_request_with_replay_field(signed, replay, starting_campaign, "replay")
}

fn multipart_request_with_replay_field(
    signed: &SignedSubmissionV1,
    replay: &[u8],
    starting_campaign: &[u8],
    replay_field: &str,
) -> Request<Body> {
    multipart_request_with_replay_transport(
        signed,
        replay,
        starting_campaign,
        replay_field,
        RANKED_REPLAY_MEDIA_TYPE_V1,
    )
}

fn multipart_request_with_replay_transport(
    signed: &SignedSubmissionV1,
    replay: &[u8],
    starting_campaign: &[u8],
    replay_field: &str,
    replay_media_type: &str,
) -> Request<Body> {
    const BOUNDARY: &str = "robin-router-e2e-boundary";
    let metadata = serde_json::to_vec(signed).unwrap();
    let mut body = Vec::new();
    body.extend_from_slice(format!("--{BOUNDARY}\r\n").as_bytes());
    body.extend_from_slice(
        b"Content-Disposition: form-data; name=\"submission\"\r\nContent-Type: application/json\r\n\r\n",
    );
    body.extend_from_slice(&metadata);
    body.extend_from_slice(format!("\r\n--{BOUNDARY}\r\n").as_bytes());
    body.extend_from_slice(
        format!(
            "Content-Disposition: form-data; name=\"{replay_field}\"; filename=\"replay.rhrec\"\r\nContent-Type: {replay_media_type}\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(replay);
    body.extend_from_slice(format!("\r\n--{BOUNDARY}\r\n").as_bytes());
    body.extend_from_slice(
        format!(
            "Content-Disposition: form-data; name=\"starting_campaign\"; filename=\"starting.campaign\"\r\nContent-Type: {RANKED_CAMPAIGN_MEDIA_TYPE_V1}\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(starting_campaign);
    body.extend_from_slice(format!("\r\n--{BOUNDARY}--\r\n").as_bytes());
    Request::builder()
        .method(Method::POST)
        .uri("/api/v1/submissions")
        .header(
            CONTENT_TYPE,
            format!("multipart/form-data; boundary={BOUNDARY}"),
        )
        .extension(peer(Ipv4Addr::LOCALHOST))
        .body(Body::from(body))
        .unwrap()
}

fn metadata_only_multipart_request(signed: &SignedSubmissionV1) -> Request<Body> {
    const BOUNDARY: &str = "robin-router-partial-boundary";
    let metadata = serde_json::to_vec(signed).unwrap();
    let mut body = Vec::new();
    body.extend_from_slice(format!("--{BOUNDARY}\r\n").as_bytes());
    body.extend_from_slice(
        b"Content-Disposition: form-data; name=\"submission\"\r\nContent-Type: application/json\r\n\r\n",
    );
    body.extend_from_slice(&metadata);
    body.extend_from_slice(format!("\r\n--{BOUNDARY}--\r\n").as_bytes());
    Request::builder()
        .method(Method::POST)
        .uri("/api/v1/submissions")
        .header(
            CONTENT_TYPE,
            format!("multipart/form-data; boundary={BOUNDARY}"),
        )
        .extension(peer(Ipv4Addr::LOCALHOST))
        .body(Body::from(body))
        .unwrap()
}

async fn json_body<T: DeserializeOwned>(response: axum::response::Response) -> T {
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap_or_else(|error| {
        panic!(
            "could not decode {status} response as {}: {error}; body={}",
            std::any::type_name::<T>(),
            String::from_utf8_lossy(&bytes)
        )
    })
}

fn verification_limits() -> VerificationLimitsV1 {
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

fn artifact(byte: u8, media_type: &str) -> ArtifactRefV1 {
    ArtifactRefV1 {
        sha256: Digest32::from_bytes([byte; 32]),
        byte_length: 1,
        media_type: media_type.to_owned(),
    }
}

fn build_manifest() -> BuildManifestV1 {
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

fn content_manifest() -> ContentManifestV1 {
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

fn full_content_manifest() -> ContentManifestV1 {
    let mut manifest = content_manifest();
    manifest.name = "Full Leicester".to_owned();
    manifest.edition = OfficialContentEditionV1::Full;
    manifest.subject = OfficialContentSubjectV1::FieldMission {
        mission_id: CAMPAIGN_MISSION_ID.to_owned(),
    };
    manifest.resource_locale_root = ResourceLocaleRootV1::new("2047").unwrap();
    manifest
}

fn genesis_content_manifest() -> ContentManifestV1 {
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

fn terminal_content_manifest() -> ContentManifestV1 {
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

fn policy_identity(kind: ImmutablePolicyKindV1, byte: u8) -> ImmutablePolicyIdentityV1 {
    ImmutablePolicyIdentityV1 {
        kind,
        version: 1,
        manifest_sha256: Digest32::from_bytes([byte; 32]),
    }
}

fn published_ruleset(
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
