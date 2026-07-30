//! Adoption of the two v48 campaign streams preceding `RHEngine`.
//!
//! The first stream is the Original's pre-mission backup and the second is
//! the live campaign. Rust stores the same relationship explicitly as
//! `Campaign::pre_mission_snapshot`.

use thiserror::Error;

use crate::{campaign::Campaign, engine::EngineInner, profiles::ProfileManager};

use super::campaign::{
    LegacyCampaignBootstrap, LegacyCampaignMappingError, LegacyMissionIdentity, LegacySaveCampaigns,
};

#[derive(Debug, Error)]
pub enum LegacyCampaignAdoptError {
    #[error("cannot map saved live campaign: {0}")]
    Live(#[source] LegacyCampaignMappingError),
    #[error("cannot map saved pre-mission campaign backup: {0}")]
    Backup(#[source] LegacyCampaignMappingError),
    #[error(
        "saved campaign streams disagree on mission identity: live mission/profile {live_mission}/{live_profile}, backup {backup_mission}/{backup_profile}"
    )]
    IdentityMismatch {
        live_mission: u32,
        live_profile: usize,
        backup_mission: u32,
        backup_profile: usize,
    },
}

#[derive(Clone, Debug)]
pub struct LegacyCampaignAdoptionPlan {
    campaign: Campaign,
    pub identity: LegacyMissionIdentity,
}

impl LegacyCampaignAdoptionPlan {
    pub fn preflight(
        campaigns: &LegacySaveCampaigns,
        profiles: &ProfileManager,
        header_mission_id: u32,
    ) -> Result<Self, LegacyCampaignAdoptError> {
        let LegacyCampaignBootstrap {
            mut campaign,
            identity,
        } = campaigns
            .live
            .campaign
            .bootstrap(profiles, header_mission_id)
            .map_err(LegacyCampaignAdoptError::Live)?;
        let LegacyCampaignBootstrap {
            campaign: backup,
            identity: backup_identity,
        } = campaigns
            .backup
            .campaign
            .bootstrap(profiles, header_mission_id)
            .map_err(LegacyCampaignAdoptError::Backup)?;

        if identity.mission_id != backup_identity.mission_id
            || identity.profile_index != backup_identity.profile_index
        {
            return Err(LegacyCampaignAdoptError::IdentityMismatch {
                live_mission: identity.mission_id,
                live_profile: identity.profile_index,
                backup_mission: backup_identity.mission_id,
                backup_profile: backup_identity.profile_index,
            });
        }

        // The v48 stream predates Rust's explicit replay seed/config fields.
        // The campaign data itself is exact; the parity trace supplies the
        // deterministic RNG stream independently at the loaded-save boundary.
        campaign.pre_mission_snapshot = Some(Box::new(backup));
        campaign.pre_mission_rng_seed = None;
        campaign.pre_mission_sim_config = None;
        campaign.pre_mission_was_preselected = true;
        Ok(Self { campaign, identity })
    }

    pub fn apply(self, engine: &mut EngineInner) {
        engine.mission_domain.campaign = self.campaign;
    }
}
