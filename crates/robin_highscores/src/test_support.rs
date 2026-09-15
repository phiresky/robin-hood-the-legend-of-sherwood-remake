//! Shared deterministic service fixtures. No production authority is created implicitly.

use robin_run_protocol::{
    BoardMetricV1, BoardMissionV2, BoardSimulationPolicyV1, BoardV2, CanonicalValue, Digest32,
    InputProvenanceStatusV1, OfficialContentEditionV1, OpaqueId, RankedSimulationDifficultyV1,
    RankedSimulationPolicyV1, SCHEMA_VERSION_V2, TerminalOutcomeV1,
    VerifiedAchievementEvaluationV1, VerifiedAchievementV1, VerifiedRunV2, VerifierOutputV2,
    ViewerContentRequirementV2, official_achievement_policies_v1,
};

/// A Demo Standard/Normal board with the given ID and missions.
pub fn demo_board(board_id: &str, missions: &[&str]) -> BoardV2 {
    BoardV2 {
        board_id: OpaqueId::new(board_id).unwrap(),
        display_name: "Demo / Standard / Normal".to_owned(),
        edition: OfficialContentEditionV1::Demo,
        preset_id: "standard".to_owned(),
        preset_name: "Standard".to_owned(),
        difficulty_id: "normal".to_owned(),
        difficulty_name: "Normal".to_owned(),
        simulation_policy: BoardSimulationPolicyV1::Fixed {
            policy: RankedSimulationPolicyV1::standard(RankedSimulationDifficultyV1::Medium),
        },
        allow_state_load: true,
        metrics: vec![BoardMetricV1::OriginalScore, BoardMetricV1::FastestSuccess],
        viewer_content_requirement: ViewerContentRequirementV2::BundledDemo,
        missions: missions
            .iter()
            .map(|mission| BoardMissionV2 {
                mission_id: (*mission).to_owned(),
                display_name: format!("Mission {mission}"),
            })
            .collect(),
    }
}

/// A valid verifier-authored verified run with the given scores and ticks.
pub fn verified_run(
    starting_campaign_score: i32,
    final_campaign_score: i32,
    active_simulation_ticks: u64,
) -> VerifiedRunV2 {
    VerifiedRunV2 {
        recorded_engine_version: "0123456789ab".to_owned(),
        sim_config: CanonicalValue::Object(
            [(
                "difficulty".to_owned(),
                CanonicalValue::String("Medium".to_owned()),
            )]
            .into_iter()
            .collect(),
        ),
        max_concurrent_players: 1,
        participant_instance_count: 1,
        outcome: TerminalOutcomeV1::Won,
        starting_campaign_score,
        final_campaign_score,
        original_score_delta: i64::from(final_campaign_score) - i64::from(starting_campaign_score),
        final_state_sha256: Digest32::from_bytes([7; 32]),
        replay_frames: 100,
        active_simulation_ticks,
        ransom_collected: 3,
        achievements: official_achievement_policies_v1()
            .into_iter()
            .map(|policy| VerifiedAchievementV1 {
                evaluation: if policy.achievement_id.as_str() == "clean-hands" {
                    VerifiedAchievementEvaluationV1::Earned
                } else {
                    VerifiedAchievementEvaluationV1::NotEarned
                },
                achievement_id: policy.achievement_id,
                evidence: Default::default(),
            })
            .collect(),
    }
}

/// The verifier output document for `run`, bound to a job and replay digest.
pub fn verified_output(
    job_sha256: Digest32,
    replay_sha256: Digest32,
    run: VerifiedRunV2,
) -> VerifierOutputV2 {
    VerifierOutputV2 {
        schema_version: SCHEMA_VERSION_V2,
        job_sha256,
        replay_sha256,
        input_provenance: Some(InputProvenanceStatusV1::Rankable),
        status: robin_run_protocol::VerificationStatusV2::Verified(run),
    }
}

/// Temporary deployment root plus the server configuration pointing into it.
///
/// Process-local test resource (owns a `TempDir`), so it is intentionally not
/// serializable. The directory must outlive every database or store opened
/// from `config`: bind it to a named variable, never `_`.
pub struct TestDeployment {
    pub directory: tempfile::TempDir,
    pub config: crate::ServerConfig,
}

impl Default for TestDeployment {
    fn default() -> Self {
        Self::new()
    }
}

impl TestDeployment {
    /// `database_path = <tempdir>/highscores.sqlite3`; every other setting is
    /// `ServerConfig::default()`.
    pub fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let config = crate::ServerConfig {
            database_path: directory.path().join("highscores.sqlite3"),
            replay_directory: directory.path().join("replays"),
            ..Default::default()
        };
        Self { directory, config }
    }

    /// Overrides further settings; the closure receives the temporary root.
    pub fn configure(
        mut self,
        configure: impl FnOnce(&std::path::Path, &mut crate::ServerConfig),
    ) -> Self {
        configure(self.directory.path(), &mut self.config);
        self
    }

    /// Creates and migrates the database and hands back the owned parts.
    pub async fn migrate(self) -> (tempfile::TempDir, crate::ServerConfig, crate::Database) {
        let database = crate::Database::migrate(&self.config).await.unwrap();
        (self.directory, self.config, database)
    }
}

pub async fn app_state(config: crate::ServerConfig) -> crate::web::AppState {
    crate::web::AppState {
        database: crate::Database::migrate(&config).await.unwrap(),
        replay_store: crate::ReplayStore::create(
            config.replay_directory.clone(),
            config.max_replay_bytes,
        )
        .await
        .unwrap(),
        config,
        cursor_hmac_key: [1; 32],
        challenge_rate_limiter: crate::web::ChallengeRateLimiter::new(10),
    }
}
