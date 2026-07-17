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
    pub(crate) force_check: bool,
    pub(crate) short_briefings: ShortBriefings,
    pub(crate) mission_stat: MissionStat,
    pub(crate) dead_pc: Option<EntityId>,
    pub(crate) campaign: Option<Campaign>,
}

impl MissionDomain {
    pub(crate) fn new() -> Self {
        Self {
            state: MissionState::default(),
            cheat_used_flags: 0,
            force_check: false,
            short_briefings: ShortBriefings::default(),
            mission_stat: MissionStat::default(),
            dead_pc: None,
            campaign: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_mission_domain_has_no_outcome_or_campaign() {
        let mission = MissionDomain::new();

        assert!(!mission.state.mission_won);
        assert!(!mission.state.quit_won);
        assert!(!mission.state.quit_lost);
        assert!(!mission.state.quit_interrupted);
        assert_eq!(mission.cheat_used_flags, 0);
        assert!(!mission.force_check);
        assert_eq!(mission.short_briefings.count(true), 0);
        assert_eq!(mission.short_briefings.count(false), 0);
        assert_eq!(mission.mission_stat, MissionStat::default());
        assert!(mission.dead_pc.is_none());
        assert!(mission.campaign.is_none());
    }
}
