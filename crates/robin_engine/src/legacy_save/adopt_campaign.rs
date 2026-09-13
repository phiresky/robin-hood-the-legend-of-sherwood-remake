//! Adoption of the two v48 campaign streams preceding engine state.
//!
//! The first stream is the Original's pre-mission backup and the second is
//! the live campaign. Rust stores the same relationship explicitly as
//! `Campaign::pre_mission_snapshot`.

use crate::{campaign::Campaign, engine::EngineInner, profiles::ProfileManager};

use super::{
    adopt_common::{AdoptErrorKind, LegacyAdoptError},
    campaign::{LegacyCampaignAdoption, LegacyCampaignBootstrap, LegacySaveCampaigns},
};

#[derive(Clone, Debug)]
pub struct LegacyCampaignAdoptionPlan {
    campaign: Campaign,
}

impl LegacyCampaignAdoptionPlan {
    pub fn preflight(
        campaigns: &LegacySaveCampaigns,
        profiles: &ProfileManager,
        header_mission_id: u32,
    ) -> Result<Self, LegacyAdoptError> {
        let LegacyCampaignBootstrap {
            mut campaign,
            identity,
        } = campaigns
            .live
            .campaign
            .bootstrap(profiles, header_mission_id)
            .map_err(|error| error.context("cannot map saved live campaign"))?;
        let LegacyCampaignBootstrap {
            campaign: backup,
            identity: backup_identity,
        } = campaigns
            .backup
            .campaign
            .bootstrap(profiles, header_mission_id)
            .map_err(|error| error.context("cannot map saved pre-mission campaign backup"))?;

        if identity.mission_id != backup_identity.mission_id
            || identity.profile_index != backup_identity.profile_index
        {
            return Err(AdoptErrorKind::CampaignIdentityMismatch {
                live_mission: identity.mission_id,
                live_profile: identity.profile_index,
                backup_mission: backup_identity.mission_id,
                backup_profile: backup_identity.profile_index,
            }
            .into());
        }

        // The v48 stream predates Rust's explicit replay seed/config fields.
        // The campaign data itself is exact; the parity trace supplies the
        // deterministic RNG stream independently at the loaded-save boundary.
        campaign.pre_mission_snapshot = Some(backup.replace_snapshot(()));
        campaign.pre_mission_rng_seed = None;
        campaign.pre_mission_sim_config = None;
        campaign.pre_mission_was_preselected = true;
        Ok(Self { campaign })
    }

    pub(crate) fn apply(self, engine: &mut EngineInner) {
        engine.mission_domain.campaign = self.campaign;
    }
}
