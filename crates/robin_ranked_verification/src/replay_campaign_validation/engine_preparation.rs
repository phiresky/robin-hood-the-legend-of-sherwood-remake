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
    /// Consume the phase-two proof directly into a ranked engine with the
    /// board's simulation policy installed. Ranked verification never enables
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
        let engine = robin_engine::engine::Engine::new_ranked(
            robin_engine::engine::EngineArgs {
                campaign,
                level,
                ground_mark_sprite,
                titbit_row_frame_counts,
                rng_seed,
                original_rng_replay: None,
                sim_config,
            },
            simulation_policy,
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
