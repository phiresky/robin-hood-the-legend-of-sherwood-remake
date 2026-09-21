//! Engine construction consumes approved campaign proof; constructors stay private here.

use super::{ApprovedReplayCampaignContent, MissionLocation};

/// Engine born from the exact campaign bytes carried by a consumed phase-two
/// capability. Keeping construction here prevents verifier adapters from
/// extracting an approved campaign, dropping the proof, and later swapping a
/// different campaign into ordinary engine bootstrap.
pub struct ApprovedReplayEngine {
    engine: robin_engine::engine::Engine,
    mission_index: usize,
    mission_location: MissionLocation,
    submitted_campaign_sha256: [u8; 32],
    starting_campaign_score: i32,
}

impl ApprovedReplayEngine {
    pub const fn starting_campaign_score(&self) -> i32 {
        self.starting_campaign_score
    }

    pub fn into_parts(
        self,
    ) -> (
        robin_engine::engine::Engine,
        usize,
        MissionLocation,
        [u8; 32],
    ) {
        (
            self.engine,
            self.mission_index,
            self.mission_location,
            self.submitted_campaign_sha256,
        )
    }
}

impl ApprovedReplayCampaignContent {
    /// Consume the phase-two proof into an engine whose initial configuration
    /// matches the board's policy. Ranked verification never enables
    /// Original RNG parity replay.
    pub fn construct_ranked_engine(
        self,
        level: robin_engine::engine::LevelLoadArgs<'_>,
        ground_mark_sprite: Option<robin_engine::engine::GroundMarkSpriteData>,
        titbit_row_frame_counts: Vec<u16>,
        rng_seed: u64,
        sim_config: robin_engine::engine::SimConfig,
        simulation_policy: robin_engine::engine::RankedSimulationPolicy,
    ) -> Result<ApprovedReplayEngine, robin_engine::engine::EngineError> {
        let (campaign, mission_index, mission_location, submitted_campaign_sha256) =
            self.into_playback_parts();
        let starting_campaign_score =
            campaign.get_value(robin_engine::campaign::CampaignValue::Score);
        let assets = level.assets;
        simulation_policy
            .validate_config(sim_config)
            .map_err(
                |error| robin_engine::engine::EngineError::MissionLevelStage {
                    stage: "ranked simulation policy",
                    reason: error.to_string(),
                },
            )?;
        let mut engine = robin_engine::engine::Engine::new(robin_engine::engine::EngineArgs {
            campaign,
            level: robin_engine::engine::LevelLoadArgs {
                assets: &mut *assets,
                level_directory: level.level_directory,
                progress: &mut *level.progress,
                loaded: level.loaded,
                bg_pixel_dims: level.bg_pixel_dims,
            },
            ground_mark_sprite,
            titbit_row_frame_counts,
            rng_seed,
            original_rng_replay: None,
            sim_config,
        })?;
        // Frame-zero hashes follow client startup: register campaign names,
        // connect the host, and consume the pending startup effects.
        let mut names = std::array::from_fn(|_| None);
        engine
            .register_mission_peasant_names(assets, &mut names)
            .and_then(|()| {
                engine.connect_seat(
                    assets,
                    robin_engine::player_command::PlayerId::HOST,
                    String::new(),
                )
            })
            .map_err(
                |error| robin_engine::engine::EngineError::MissionLevelStage {
                    stage: "ranked mission startup",
                    reason: error.to_string(),
                },
            )?;
        Ok(ApprovedReplayEngine {
            engine,
            mission_index,
            mission_location,
            submitted_campaign_sha256,
            starting_campaign_score,
        })
    }
}
