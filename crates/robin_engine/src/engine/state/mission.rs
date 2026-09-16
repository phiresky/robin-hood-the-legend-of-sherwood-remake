use serde::{Deserialize, Serialize};

use crate::{
    achievement::MissionAchievementState,
    campaign::{Campaign, CampaignValue},
    diplomacy::DiplomacyState,
    element::EntityId,
    engine::{HostEffects, MissionState, SoundCommand},
    mission_stat::MissionStat,
    short_briefings::ShortBriefings,
};

/// Deterministic mission outcome, campaign, objective, and debriefing state.
///
/// `Domain` distinguishes this engine-owned state from the host-side
/// `robin_rs::MissionRuntime` lifecycle object.
#[derive(
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub(crate) struct MissionDomain {
    pub(crate) state: MissionState,
    pub(crate) cheat_used_flags: u32,
    pub(crate) short_briefings: ShortBriefings,
    pub(crate) mission_stat: MissionStat,
    /// Deterministic live achievement evidence and frozen successful result.
    /// Native save/replay versions must be bumped when this foundation is
    /// integrated with the other feature branches.
    pub(crate) achievements: MissionAchievementState,
    pub(crate) dead_pc: Option<EntityId>,
    pub(crate) diplomacy: DiplomacyState,
    pub(crate) campaign: Campaign,
}

impl MissionDomain {
    pub(crate) fn new(campaign: Campaign) -> Self {
        Self {
            state: MissionState::default(),
            cheat_used_flags: 0,
            short_briefings: ShortBriefings::default(),
            mission_stat: MissionStat::default(),
            achievements: MissionAchievementState::from_mission_start(),
            dead_pc: None,
            diplomacy: DiplomacyState::default(),
            campaign,
        }
    }

    pub(crate) fn campaign(&self) -> &Campaign {
        &self.campaign
    }

    pub(crate) fn campaign_mut(&mut self) -> &mut Campaign {
        &mut self.campaign
    }

    /// Mutate a campaign value with the usual addition side effects.
    /// In addition to the raw field write, RANSOM credits to the
    /// per-mission collected-money counter and (for positive deltas
    /// after the first frame) emits the `CashWon` jingle; SCORE credits
    /// to the per-mission added-score counter.  Other campaign values
    /// have no extra side effects.
    pub(crate) fn add_campaign_value(
        &mut self,
        side_effects: &mut HostEffects,
        frame_counter: u32,
        name: CampaignValue,
        amount: i32,
    ) {
        self.campaign.values[name] += amount;
        // Credit the mission-stat counters unconditionally for
        // RANSOM/SCORE — only the CashWon jingle is gated on
        // `amount > 0 && frame_counter > 0`.
        match name {
            CampaignValue::Ransom => {
                self.mission_stat.add_collected_money(amount);
                if amount > 0 && frame_counter > 0 {
                    side_effects
                        .sounds
                        .push(SoundCommand::Jingle(crate::sound::Jingle::CashWon));
                }
            }
            CampaignValue::Score => {
                self.mission_stat.add_score(amount);
            }
            _ => {}
        }
    }

    /// Campaign-only tail of a won mission's teardown, run after the
    /// entity/coma phases: soldier and score bonuses, post-mission peasant
    /// recruitment, and blazon consumption.
    pub(crate) fn apply_won_updates(
        &mut self,
        side_effects: &mut HostEffects,
        frame_counter: u32,
        sim: &crate::sim_rng::SimulationContext,
        profiles: &crate::profiles::ProfileManager,
        living: u32,
        dead: u32,
        tied_score: i32,
        difficulty: crate::player_profile::DifficultyLevel,
    ) {
        // The original game adds the counts from this exit-time NPC scan when quitting a mission
        // directly to the campaign. `mStat.ulTotalSoldierCount` is the
        // load-time mission total and is not a source for either delta.
        self.add_campaign_value(
            side_effects,
            frame_counter,
            CampaignValue::LivingSoldiers,
            living as i32,
        );
        self.add_campaign_value(
            side_effects,
            frame_counter,
            CampaignValue::DeadSoldiers,
            dead as i32,
        );

        self.add_campaign_value(
            side_effects,
            frame_counter,
            CampaignValue::Score,
            tied_score,
        );

        let idx = self
            .campaign
            .current_mission_idx
            .expect("quit-mission updates: current mission disappeared");
        let mission_type = self.campaign.missions[idx].profile(profiles).mission_type;
        if mission_type != crate::profiles::MissionType::Ambush {
            self.add_campaign_value(side_effects, frame_counter, CampaignValue::Score, 1000);
        }

        // The original game applies difficulty to recruitment only after the score updates
        // above. The application resolves that difficulty into the command,
        // so replay and multiplayer execution cannot consult ambient state.
        let recruited = self
            .campaign
            .recruit_post_mission_peasants(sim, living, dead, difficulty, profiles);
        self.mission_stat.new_peasant_count = recruited;
        tracing::info!("Post-mission warcrime recruitment: {recruited} new peasants");

        self.campaign.consume_blazons_post_mission(profiles);
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
        assert_eq!(
            mission.achievements.tracking_provenance(),
            crate::achievement::AchievementTrackingProvenance::MissionStart
        );
        assert!(mission.dead_pc.is_none());
        assert_eq!(
            mission.campaign.production_sectors.as_ptr(),
            production_sectors
        );
    }
}
