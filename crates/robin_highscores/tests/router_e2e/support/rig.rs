use super::*;

pub(crate) struct RigOptions {
    pub(crate) any_ruleset: bool,
    pub(crate) campaign: bool,
    pub(crate) competition: bool,
    pub(crate) allowed_metrics: Vec<String>,
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

pub(crate) struct TestRig {
    pub(crate) _directory: tempfile::TempDir,
    pub(crate) app: Router,
    pub(crate) config: ServerConfig,
    pub(crate) database: Database,
    pub(crate) replay_store: ReplayStore,
    pub(crate) campaign_store: CampaignStore,
    pub(crate) loaded_build: LoadedBuildManifest,
    pub(crate) build: BuildManifestV1,
    pub(crate) content: ContentManifestV1,
    pub(crate) genesis_content: Option<ContentManifestV1>,
    pub(crate) terminal_content: Option<ContentManifestV1>,
    pub(crate) campaign_content: Option<CampaignContentManifestV1>,
    pub(crate) competition: Option<CompetitionManifestV1>,
    pub(crate) published_ruleset: robin_run_protocol::PublishedRulesetV1,
    pub(crate) build_sha256: Digest32,
    pub(crate) content_sha256: Digest32,
    pub(crate) rules_config_sha256: Digest32,
    pub(crate) ruleset_sha256: Digest32,
    pub(crate) starting_campaign_sha256: Digest32,
    pub(crate) starting_campaign: Vec<u8>,
    pub(crate) genesis_content_sha256: Option<Digest32>,
    pub(crate) terminal_content_sha256: Option<Digest32>,
    pub(crate) campaign_content_sha256: Option<Digest32>,
    pub(crate) competition_sha256: Option<Digest32>,
}

impl TestRig {
    pub(crate) async fn new() -> Self {
        Self::new_with(RigOptions::default()).await
    }

    pub(crate) async fn new_with(options: RigOptions) -> Self {
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

    pub(crate) fn app_with_ruleset_status(
        &self,
        operational_status: RulesetOperationalStatusV1,
    ) -> Router {
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

    pub(crate) fn app_with_storage_floor(&self, minimum_storage_free_bytes: u64) -> Router {
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

    pub(crate) fn app_with_backup_status(&self, path: std::path::PathBuf) -> Router {
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

    pub(crate) async fn rename(
        &self,
        key: &SigningKey,
        username: &str,
        peer: Ipv4Addr,
    ) -> PlayerProfileV1 {
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

    pub(crate) async fn submit(&self, key: &SigningKey, sequence: u8) -> SubmissionAcceptedV1 {
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

    pub(crate) async fn submit_request(
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

    pub(crate) async fn publish(
        &self,
        accepted: &SubmissionAcceptedV1,
        score: i32,
        final_byte: u8,
    ) -> OpaqueId {
        self.publish_individual(accepted, score, final_byte, false)
            .await
    }

    pub(crate) async fn publish_competition(
        &self,
        accepted: &SubmissionAcceptedV1,
        score: i32,
        final_byte: u8,
    ) -> OpaqueId {
        self.publish_individual(accepted, score, final_byte, true)
            .await
    }

    pub(crate) async fn store_campaign(&self, bytes: &[u8]) -> ArtifactRefV1 {
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

    pub(crate) async fn publish_individual(
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

    pub(crate) fn fresh_preflight_request(
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

    pub(crate) async fn authorize_fresh_request(
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

    pub(crate) fn continuation_preflight_request(
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

    pub(crate) async fn authorize_continuation_request(
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

    pub(crate) async fn issue_continuation_offer(
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

    pub(crate) async fn issue_offer(
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

    pub(crate) async fn upload_campaign_offer(
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

    pub(crate) async fn publish_campaign(
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

    pub(crate) async fn owner_status(
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

    pub(crate) async fn owner_status_envelope(
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

    pub(crate) async fn campaign_receipt(
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

    pub(crate) async fn delete_run(
        &self,
        owner: &SigningKey,
        run_id: &OpaqueId,
    ) -> DeletionReceiptV1 {
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

    pub(crate) fn campaign_offer_request(
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

    pub(crate) fn offer_request(&self, key: &SigningKey, sequence: u8) -> SubmissionOfferRequestV1 {
        self.individual_offer_request(key, sequence, None)
    }

    pub(crate) async fn competition_offer_request(
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

    pub(crate) fn individual_offer_request(
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
