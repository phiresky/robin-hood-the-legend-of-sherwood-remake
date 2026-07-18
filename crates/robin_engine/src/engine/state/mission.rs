use serde::{Deserialize, Serialize};

use crate::{
    campaign::Campaign, element::EntityId, engine::MissionState, mission_stat::MissionStat,
    short_briefings::ShortBriefings,
};

/// Deterministic mission outcome, campaign, objective, and debriefing state.
///
/// `Domain` distinguishes this engine-owned state from the host-side
/// `robin_rs::MissionRuntime` lifecycle object.
#[derive(Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub(crate) struct MissionDomain {
    pub(crate) state: MissionState,
    pub(crate) cheat_used_flags: u32,
    pub(crate) short_briefings: ShortBriefings,
    pub(crate) mission_stat: MissionStat,
    pub(crate) dead_pc: Option<EntityId>,
    pub(crate) campaign: Campaign,
}

impl MissionDomain {
    pub(crate) fn new(campaign: Campaign) -> Self {
        Self {
            state: MissionState::default(),
            cheat_used_flags: 0,
            short_briefings: ShortBriefings::default(),
            mission_stat: MissionStat::default(),
            dead_pc: None,
            campaign,
        }
    }

    pub(crate) fn required_campaign(&self, context: &str) -> &Campaign {
        let _ = context;
        &self.campaign
    }

    pub(crate) fn required_campaign_mut(&mut self, context: &str) -> &mut Campaign {
        let _ = context;
        &mut self.campaign
    }

    /// Borrow the required campaign and mission statistics as disjoint parts
    /// of their common owner.
    pub(crate) fn required_campaign_and_stat(
        &mut self,
        context: &str,
    ) -> (&mut Campaign, &mut MissionStat) {
        let Self {
            campaign,
            mission_stat,
            ..
        } = self;
        let _ = context;
        (campaign, mission_stat)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_mission_domain_has_no_outcome_and_keeps_supplied_campaign() {
        let campaign = Campaign::default();
        let production_sectors = campaign.production_sectors.as_ptr();
        let mission = MissionDomain::new(campaign);

        assert!(!mission.state.mission_won);
        assert!(!mission.state.quit_won);
        assert!(!mission.state.quit_lost);
        assert!(!mission.state.quit_interrupted);
        assert_eq!(mission.cheat_used_flags, 0);
        assert_eq!(mission.short_briefings.count(true), 0);
        assert_eq!(mission.short_briefings.count(false), 0);
        assert_eq!(mission.mission_stat, MissionStat::default());
        assert!(mission.dead_pc.is_none());
        assert_eq!(
            mission.campaign.production_sectors.as_ptr(),
            production_sectors
        );
    }
}
