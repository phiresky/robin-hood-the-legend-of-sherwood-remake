use super::*;

/// A migrated database, replay store and real router with two Demo boards:
/// `demo-standard-normal` (both metrics) and `demo-score-only`.
pub(crate) struct TestRig {
    pub(crate) directory: tempfile::TempDir,
    pub(crate) app: Router,
    pub(crate) config: ServerConfig,
    pub(crate) database: Database,
    pub(crate) replay_store: ReplayStore,
}

impl TestRig {
    pub(crate) async fn new() -> Self {
        let _ = tracing_subscriber::fmt()
            .with_env_filter("robin_highscores=trace")
            .with_test_writer()
            .try_init();
        let directory = tempfile::tempdir().unwrap();
        let mut score_only =
            robin_highscores::test_support::demo_board(SCORE_ONLY_BOARD_ID, &[MISSION_ID]);
        score_only.metrics = vec![BoardMetricV1::OriginalScore];
        let config = ServerConfig {
            database_path: directory.path().join("highscores.sqlite3"),
            replay_directory: directory.path().join("replays"),
            challenge_requests_per_minute_per_ip: 1_000,
            boards: vec![
                robin_highscores::test_support::demo_board(BOARD_ID, &[MISSION_ID]),
                score_only,
            ],
            ..Default::default()
        };
        config.validate().unwrap();
        let database = Database::migrate(&config).await.unwrap();
        let replay_store =
            ReplayStore::create(config.replay_directory.clone(), config.max_replay_bytes)
                .await
                .unwrap();
        let app = Self::router_for(&config, &database, &replay_store);
        Self {
            directory,
            app,
            config,
            database,
            replay_store,
        }
    }

    fn router_for(
        config: &ServerConfig,
        database: &Database,
        replay_store: &ReplayStore,
    ) -> Router {
        router(AppState {
            config: config.clone(),
            database: database.clone(),
            replay_store: replay_store.clone(),
            cursor_hmac_key: [0x5a; 32],
            challenge_rate_limiter: ChallengeRateLimiter::new(
                config.challenge_requests_per_minute_per_ip,
            ),
        })
        .unwrap()
    }

    /// The same deployment with a different free-space floor.
    pub(crate) fn app_with_storage_floor(&self, minimum_storage_free_bytes: u64) -> Router {
        let mut config = self.config.clone();
        config.minimum_storage_free_bytes = minimum_storage_free_bytes;
        Self::router_for(&config, &self.database, &self.replay_store)
    }

    /// The same deployment with operator routes enabled.
    pub(crate) fn app_with_operator_token(&self, token: &[u8]) -> Router {
        let mut config = self.config.clone();
        config.moderation_bearer_token_path = Some(self.directory.path().join("token"));
        config.moderation_bearer_token = Some(std::sync::Arc::new(token.to_vec()));
        Self::router_for(&config, &self.database, &self.replay_store)
    }

    pub(crate) async fn send(&self, request: Request<Body>) -> axum::response::Response {
        self.app.clone().oneshot(request).await.unwrap()
    }

    pub(crate) async fn rename(&self, key: &SigningKey, username: &str, address: Ipv4Addr) {
        let challenge: UsernameChallengeV1 = json_body(
            self.send(json_request(
                Method::POST,
                "/api/v1/username-challenges",
                &UsernameChallengeRequestV1 {
                    schema_version: SCHEMA_VERSION_V1,
                    public_key: protocol_public_key(key),
                },
                address,
            ))
            .await,
        )
        .await;
        let mut update = UsernameUpdateEnvelopeV1 {
            schema_version: SCHEMA_VERSION_V1,
            username_challenge_id: challenge.username_challenge_id,
            username_challenge_nonce: challenge.username_challenge_nonce,
            public_key: protocol_public_key(key),
            username: username.to_owned(),
            signature: Signature64::from_bytes([0; 64]),
        };
        update.signature = sign(key, &update.signing_bytes().unwrap());
        let response = self
            .send(json_request(
                Method::PUT,
                &format!("/api/v1/players/{}/username", protocol_public_key(key)),
                &update,
                address,
            ))
            .await;
        assert_eq!(response.status(), StatusCode::OK);
    }

    pub(crate) async fn upload_challenge(
        &self,
        key: &SigningKey,
        address: Ipv4Addr,
    ) -> UploadChallengeV1 {
        let response = self
            .send(json_request(
                Method::POST,
                "/api/v1/upload-challenges",
                &UploadChallengeRequestV2 {
                    schema_version: SCHEMA_VERSION_V2,
                    public_key: protocol_public_key(key),
                },
                address,
            ))
            .await;
        assert_eq!(response.status(), StatusCode::CREATED);
        assert_dynamic_headers(&response);
        let challenge: UploadChallengeV1 = json_body(response).await;
        challenge.validate().unwrap();
        challenge
    }

    /// Challenge, sign and upload `replay` to the default board; returns the
    /// accepted queue response.
    pub(crate) async fn submit(
        &self,
        key: &SigningKey,
        replay: &[u8],
        disclosure: ParticipantPublicDisclosureV1,
    ) -> SubmissionAcceptedV1 {
        let challenge = self.upload_challenge(key, Ipv4Addr::LOCALHOST).await;
        let signed = signed_submission(key, submission(key, challenge, replay, disclosure));
        let response = self.send(multipart_request(&signed, replay)).await;
        assert_eq!(
            response.status(),
            StatusCode::ACCEPTED,
            "{}",
            bad_request_message(response).await
        );
        json_body(response).await
    }

    /// Act as the verifier worker: lease the next job, which must be
    /// `submission_id`, and publish `run` for it. Returns the run ID.
    pub(crate) async fn accept_as_worker(
        &self,
        submission_id: &OpaqueId,
        run: VerifiedRunV2,
    ) -> OpaqueId {
        let job = self
            .database
            .lease_next("rig-worker", Duration::from_secs(60))
            .await
            .unwrap()
            .expect("a queued job");
        assert_eq!(job.submission_id, submission_id.as_str());
        let board = self
            .config
            .board(&OpaqueId::new(job.board_id.clone()).unwrap())
            .unwrap();
        let job_sha256 = Digest32::digest_bytes(b"rig verifier job");
        let output = robin_highscores::test_support::verified_output(
            job_sha256,
            Digest32::from_bytes(job.replay_sha256),
            run,
        );
        OpaqueId::new(
            self.database
                .accept_job(&job.submission_id, "rig-worker", job_sha256, board, &output)
                .await
                .unwrap(),
        )
        .unwrap()
    }

    /// Upload and publish one named run with the given score delta and ticks.
    pub(crate) async fn publish_run(
        &self,
        key: &SigningKey,
        label: &str,
        score: i32,
        ticks: u64,
        disclosure: ParticipantPublicDisclosureV1,
    ) -> OpaqueId {
        let replay = compact_replay_fixture(label);
        let accepted = self.submit(key, &replay, disclosure).await;
        self.accept_as_worker(
            &accepted.submission_id,
            robin_highscores::test_support::verified_run(0, score, ticks),
        )
        .await
    }

    pub(crate) async fn count(&self, sql: &'static str) -> i64 {
        sqlx::query_scalar::<_, i64>(sql)
            .fetch_one(self.database.fixture_pool())
            .await
            .unwrap()
    }
}
