//! A mission's runtime state (age, blazon price, completion status).
//!
//! The static mission profile data (loaded from CSV) lives separately
//! in `MissionProfile`; this module only owns the serializable mutable
//! state plus the legacy save-file deserializer.

use serde::{Deserialize, Serialize};

use crate::achievement::{
    AchievementEvaluation, AchievementId, AchievementSet, MissionAchievementHistory,
};

/// Mission completion status.
#[repr(u32)]
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub enum MissionStatus {
    Available = 0,
    Won = 1,
    Lost = 2,
}

/// Runtime state of a mission.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct Mission {
    pub age: u16,
    pub blazon_price: u16,
    pub status: MissionStatus,
    /// Index into ProfileManager.missions (the static profile data).
    pub profile_idx: Option<u32>,
    /// Runtime override for `MissionProfile::ares_state_succeeded`.
    /// Profiles are `Arc`-shared and cannot be mutated directly; the
    /// `WINCAMPAIGN` cheat (sets `ares_state_succeeded = 9`) stores its
    /// override here and readers must prefer this value when set.
    pub ares_state_override: Option<i8>,
    /// Lossless achievement results from every eligible successful replay of
    /// this mission. The original game has no corresponding field.
    #[serde(default)]
    pub achievement_history: MissionAchievementHistory,
}

impl Default for Mission {
    fn default() -> Self {
        Self::new()
    }
}

impl Mission {
    pub fn new() -> Self {
        Mission {
            age: 0,
            blazon_price: 0,
            status: MissionStatus::Available,
            profile_idx: None,
            ares_state_override: None,
            achievement_history: MissionAchievementHistory::default(),
        }
    }

    pub fn is_done(&self) -> bool {
        self.status != MissionStatus::Available
    }

    /// Read-only history used by campaign-map badges and mission history UI.
    pub const fn achievement_history(&self) -> &MissionAchievementHistory {
        &self.achievement_history
    }

    pub fn achievement_badges(&self) -> AchievementSet {
        self.achievement_history.badges()
    }

    pub fn best_achievement_result(&self, id: AchievementId) -> Option<AchievementEvaluation> {
        self.achievement_history.best(id)
    }

    /// Get the profile for this mission.
    /// Panics if the profile index is not set.
    pub fn profile<'a>(
        &self,
        profiles: &'a crate::profiles::ProfileManager,
    ) -> &'a crate::profiles::MissionProfile {
        let idx = self.profile_idx.expect("Mission has no profile_idx");
        &profiles.missions[idx as usize]
    }

    /// Does this mission require blazons to play? (PSEUDO or ATTACK)
    pub fn requires_blazons(&self, profiles: &crate::profiles::ProfileManager) -> bool {
        matches!(
            self.profile(profiles).mission_type,
            crate::profiles::MissionType::Pseudo | crate::profiles::MissionType::Attack
        )
    }

    /// Does this mission produce blazons when won? (ATTACK or TACTICAL)
    pub fn produces_blazons(&self, profiles: &crate::profiles::ProfileManager) -> bool {
        matches!(
            self.profile(profiles).mission_type,
            crate::profiles::MissionType::Attack | crate::profiles::MissionType::Tactical
        )
    }

    /// Check if this mission is accessible given campaign state.
    pub fn is_accessible(
        &self,
        campaign: &crate::campaign::Campaign,
        profiles: &crate::profiles::ProfileManager,
    ) -> bool {
        self.is_accessible_why(campaign, profiles).is_ok()
    }

    /// Check accessibility with a reason string on failure.
    pub fn is_accessible_why(
        &self,
        campaign: &crate::campaign::Campaign,
        profiles: &crate::profiles::ProfileManager,
    ) -> Result<(), &'static str> {
        let p = self.profile(profiles);
        let money = campaign.get_value(crate::campaign::CampaignValue::Ransom);
        let gang_size = campaign.get_size_of_gang() as u32;
        let ares = campaign.get_ares();

        if money < p.min_ransom as i32 {
            return Err("Not enough money");
        }
        if p.max_ransom < money as u32 && p.max_ransom != 200000 {
            return Err("Too much money");
        }
        if gang_size < p.min_gang_size as u32 {
            return Err("Not enough gang members");
        }
        if (p.max_gang_size as u32) < gang_size {
            return Err("Too many gang members");
        }
        if self.age >= p.life_time {
            return Err("Age limit exceeded");
        }
        if p.ares_sensible {
            // ARES -1 is the legacy "no ARES state yet / no change"
            // sentinel used by a freshly reset campaign. It must not be
            // converted to usize for the availability-table lookup.
            if ares != -1 {
                let Some(i) = ares
                    .try_into()
                    .ok()
                    .filter(|&i: &usize| i < p.available_in_ares_state.len())
                else {
                    panic!(
                        "campaign ARES state {ares} out of bounds for mission profile_idx {:?}",
                        self.profile_idx
                    );
                };
                if !p.available_in_ares_state[i] {
                    return Err("Incompatible with ARES state");
                }
            }
        }
        if !self.are_required_missions_valid(campaign, true, profiles) {
            return Err("Some missions are required to be played");
        }
        if !self.are_required_missions_valid(campaign, false, profiles) {
            return Err("Some missions are required not to be played");
        }
        Ok(())
    }

    /// Check if prerequisite missions are in the right state.
    ///
    /// `Campaign::get_mission` keys on the *profile* id (not the
    /// campaign-vector index), and a missing prerequisite is a campaign
    /// integrity bug that should crash rather than silently unlock a mission.
    fn are_required_missions_valid(
        &self,
        campaign: &crate::campaign::Campaign,
        must_be_done: bool,
        profiles: &crate::profiles::ProfileManager,
    ) -> bool {
        let p = self.profile(profiles);
        let required = if must_be_done {
            &p.missions_required_to_be_done
        } else {
            &p.missions_required_not_to_be_done
        };
        for &mission_id in required {
            let m = campaign
                .get_mission(mission_id, profiles)
                .unwrap_or_else(|| {
                    panic!(
                        "required mission profile id {mission_id} not found in campaign \
                     (referenced by mission profile_idx {:?})",
                        self.profile_idx
                    )
                });
            if m.is_done() != must_be_done {
                return false;
            }
        }
        true
    }

    /// Compare by obligation (obligatory first).
    pub fn cmp_by_obligation(
        &self,
        other: &Mission,
        profiles: &crate::profiles::ProfileManager,
    ) -> std::cmp::Ordering {
        other
            .profile(profiles)
            .obligatory
            .cmp(&self.profile(profiles).obligatory)
    }

    /// Compare by location then priority.
    pub fn cmp_by_location_and_priority(
        &self,
        other: &Mission,
        profiles: &crate::profiles::ProfileManager,
    ) -> std::cmp::Ordering {
        let a = self.profile(profiles);
        let b = other.profile(profiles);
        a.proto_level_filename
            .cmp(&b.proto_level_filename)
            .then(a.priority.cmp(&b.priority))
    }

    pub fn get_age(&self) -> u16 {
        self.age
    }

    pub fn increase_age(&mut self, profiles: &crate::profiles::ProfileManager) {
        let life_time = self.profile(profiles).life_time;
        if life_time < 1000 || self.age == 0 {
            self.age = self.age.wrapping_add(1);
        }
    }

    pub fn reset_age(&mut self) {
        self.age = 0;
    }

    pub fn get_blazon_price(&self) -> u16 {
        self.blazon_price
    }

    pub fn increase_blazon_price(&mut self, profiles: &crate::profiles::ProfileManager) {
        self.blazon_price = self
            .blazon_price
            .wrapping_add(self.profile(profiles).blazon_inflation);
    }

    pub fn win(&mut self) {
        self.status = MissionStatus::Won;
    }

    pub fn lose(&mut self) {
        self.status = MissionStatus::Lost;
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::campaign::Campaign;
    use crate::profiles::{MissionProfile, ProfileManager};

    #[test]
    fn serde_json_round_trip() {
        let mut m = Mission::new();
        m.age = 5;
        m.blazon_price = 100;
        m.status = MissionStatus::Won;
        m.profile_idx = Some(3);

        let json = serde_json::to_string(&m).unwrap();
        let m2: Mission = serde_json::from_str(&json).unwrap();

        assert_eq!(m2.age, 5);
        assert_eq!(m2.blazon_price, 100);
        assert_eq!(m2.status, MissionStatus::Won);
        assert_eq!(m2.profile_idx, Some(3));
    }

    #[test]
    fn status_values() {
        assert_eq!(MissionStatus::Available as u32, 0);
        assert_eq!(MissionStatus::Won as u32, 1);
        assert_eq!(MissionStatus::Lost as u32, 2);
    }

    #[test]
    fn is_done() {
        let mut m = Mission::new();
        assert!(!m.is_done());
        m.win();
        assert!(m.is_done());
    }

    #[test]
    fn age_and_blazon_price_use_explicit_legacy_wrapping() {
        let mut profiles = ProfileManager::new();
        profiles.missions.push(MissionProfile {
            life_time: 10,
            blazon_inflation: 2,
            ..Default::default()
        });

        let mut mission = Mission::new();
        mission.profile_idx = Some(0);
        mission.age = u16::MAX;
        mission.blazon_price = u16::MAX;

        mission.increase_age(&profiles);
        mission.increase_blazon_price(&profiles);

        assert_eq!(mission.age, 0);
        assert_eq!(mission.blazon_price, 1);
    }

    #[test]
    fn negative_ares_skips_ares_availability_filter() {
        let mut profiles = ProfileManager::new();
        profiles.missions.push(MissionProfile {
            max_ransom: 200000,
            max_gang_size: 5,
            life_time: 1000,
            ares_sensible: true,
            available_in_ares_state: [false; 10],
            ..Default::default()
        });

        let mut mission = Mission::new();
        mission.profile_idx = Some(0);

        let campaign = Campaign::new();
        assert_eq!(campaign.get_ares(), -1);
        assert_eq!(mission.is_accessible_why(&campaign, &profiles), Ok(()));
    }
}
