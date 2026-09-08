//! Engine preparation consumes approved campaign proof; constructors stay private here.

use super::{
    ApprovedReplayCampaignContent, MissionLocation, ReplayCampaignApprovedContentIdentity,
};

/// Engine born from the exact campaign bytes carried by a consumed phase-two
/// capability. Keeping construction here prevents verifier adapters from
/// extracting an approved campaign, dropping the proof, and later swapping a
/// different campaign into ordinary engine bootstrap.
pub struct ApprovedReplayEngine {
    engine: robin_engine::engine::Engine,
    mission_index: usize,
    mission_location: MissionLocation,
    submitted_campaign_sha256: [u8; 32],
    approved_identity: ReplayCampaignApprovedContentIdentity,
}

/// Single-use ranked preparation born from the exact submitted campaign and
/// approved static content. The verifier may inspect the recomputed seal, but
/// only consuming this owner can create the engine used for resimulation.
pub struct ApprovedRankedReplayPreparation {
    prepared: robin_engine::simulation_inputs::RankedPreparedMissionInputs,
    mission_index: usize,
    mission_location: MissionLocation,
    submitted_campaign_sha256: [u8; 32],
    approved_identity: ReplayCampaignApprovedContentIdentity,
    starting_campaign_score: i32,
}

impl ApprovedRankedReplayPreparation {
    pub fn seal(&self) -> &robin_run_protocol::PreparedMissionInputsSealV1 {
        self.prepared.seal()
    }

    pub fn run_projection_sha256(
        &self,
    ) -> Result<robin_run_protocol::Digest32, robin_engine::simulation_inputs::ProjectionError>
    {
        self.prepared.run_projection_sha256()
    }

    pub const fn starting_campaign_score(&self) -> i32 {
        self.starting_campaign_score
    }

    pub fn into_engine(self) -> ApprovedReplayEngine {
        ApprovedReplayEngine {
            engine: robin_engine::engine::Engine::new_ranked(self.prepared),
            mission_index: self.mission_index,
            mission_location: self.mission_location,
            submitted_campaign_sha256: self.submitted_campaign_sha256,
            approved_identity: self.approved_identity,
        }
    }
}

impl ApprovedReplayEngine {
    pub fn into_parts(
        self,
    ) -> (
        robin_engine::engine::Engine,
        usize,
        MissionLocation,
        [u8; 32],
        ReplayCampaignApprovedContentIdentity,
    ) {
        (
            self.engine,
            self.mission_index,
            self.mission_location,
            self.submitted_campaign_sha256,
            self.approved_identity,
        )
    }
}

impl ApprovedReplayCampaignContent {
    /// Consume the sealed phase-two proof directly into `Engine::new`.
    /// Ranked verification never enables Original RNG parity replay.
    pub fn construct_engine(
        self,
        level: robin_engine::engine::LevelLoadArgs<'_>,
        ground_mark_sprite: Option<robin_engine::engine::GroundMarkSpriteData>,
        titbit_row_frame_counts: Vec<u16>,
        rng_seed: u64,
        sim_config: robin_engine::engine::SimConfig,
    ) -> Result<ApprovedReplayEngine, robin_engine::engine::EngineError> {
        let (
            campaign,
            mission_index,
            mission_location,
            submitted_campaign_sha256,
            approved_identity,
        ) = self.into_playback_parts();
        let engine = robin_engine::engine::Engine::new(robin_engine::engine::EngineArgs {
            campaign,
            level,
            ground_mark_sprite,
            titbit_row_frame_counts,
            rng_seed,
            original_rng_replay: None,
            sim_config,
        })?;
        Ok(ApprovedReplayEngine {
            engine,
            mission_index,
            mission_location,
            submitted_campaign_sha256,
            approved_identity,
        })
    }

    /// Consume phase-one and phase-two campaign proof into the same ranked
    /// preparation capability which admitted the mounted eight-document
    /// semantic closure. The engine remains inaccessible until the caller has
    /// compared the recomputed protocol seal with the signed job catalog.
    pub fn prepare_ranked_engine(
        self,
        level: robin_engine::engine::LevelLoadArgs<'_>,
        ground_mark_sprite: Option<robin_engine::engine::GroundMarkSpriteData>,
        titbit_row_frame_counts: Vec<u16>,
        rng_seed: u64,
        sim_config: robin_engine::engine::SimConfig,
        admission: robin_engine::simulation_inputs::RankedContentAdmissionV1<'_>,
    ) -> Result<ApprovedRankedReplayPreparation, robin_engine::engine::EngineError> {
        let (
            campaign,
            mission_index,
            mission_location,
            submitted_campaign_sha256,
            approved_identity,
        ) = self.into_playback_parts();
        let starting_campaign_score =
            campaign.get_value(robin_engine::campaign::CampaignValue::Score);
        let prepared = robin_engine::engine::Engine::prepare_ranked(
            robin_engine::engine::EngineArgs {
                campaign,
                level,
                ground_mark_sprite,
                titbit_row_frame_counts,
                rng_seed,
                original_rng_replay: None,
                sim_config,
            },
            admission,
        )?;
        Ok(ApprovedRankedReplayPreparation {
            prepared,
            mission_index,
            mission_location,
            submitted_campaign_sha256,
            approved_identity,
            starting_campaign_score,
        })
    }
}
