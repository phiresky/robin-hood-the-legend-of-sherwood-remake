//! A mission's runtime state (age, blazon price, completion status).
//!
//! The static mission profile data (loaded from CSV) lives separately
//! in `MissionProfile`; this module only owns the serializable mutable
//! state plus the legacy save-file deserializer.

use serde::{Deserialize, Serialize};

use crate::achievement::{AchievementEvaluation, AchievementId, AchievementSet};
use crate::campaign_history::MissionAttemptHistory;

/// An unmet entry condition, independent of campaign selection and presentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MissionEligibilityFailure {
    MinimumRansom { required: u32, current: i32 },
    MaximumRansom { allowed: u32, current: i32 },
    MinimumGang { required: u16, current: u32 },
    MaximumGang { allowed: u16, current: u32 },
    Expired,
    StoryState,
    Prerequisite { mission_id: u32, must_be_done: bool },
}

impl MissionEligibilityFailure {
    /// Compatibility diagnostics used by campaign selection logging.
    pub const fn reason(self) -> &'static str {
        match self {
            Self::MinimumRansom { .. } => "Not enough money",
            Self::MaximumRansom { .. } => "Too much money",
            Self::MinimumGang { .. } => "Not enough gang members",
            Self::MaximumGang { .. } => "Too many gang members",
            Self::Expired => "Age limit exceeded",
            Self::StoryState => "Incompatible with ARES state",
            Self::Prerequisite {
                must_be_done: true, ..
            } => "Some missions are required to be played",
            Self::Prerequisite {
                must_be_done: false,
                ..
            } => "Some missions are required not to be played",
        }
    }
}

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
    /// Lossless terminal records for every played attempt, including losses,
    /// interruptions, and successful history replays.
    pub attempt_history: MissionAttemptHistory,
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
            attempt_history: MissionAttemptHistory::default(),
        }
    }

    pub fn is_done(&self) -> bool {
        self.status != MissionStatus::Available
    }

    pub fn achievement_badges(&self) -> AchievementSet {
        self.attempt_history.eligible_badges()
    }

    /// Host-attested badges suitable for campaign/lifetime aggregation.
    pub fn attested_achievement_badges(&self) -> AchievementSet {
        self.attempt_history.eligible_badges()
    }

    pub fn best_achievement_result(&self, id: AchievementId) -> Option<AchievementEvaluation> {
        self.attempt_history.best_eligible_achievement(id)
    }

    /// Whether a successful historical record exists but cannot truthfully
    /// answer whether `id` was eligible and earned.
    ///
    /// This is deliberately separate from a known failed or policy-blocked
    /// attempt. Incomplete Original imports and missing calculated results are
    /// unknown evidence; they must never be promoted to either success or a
    /// fabricated failure.
    pub fn achievement_evidence_incomplete(&self, id: AchievementId) -> bool {
        if self.attested_achievement_badges().contains(id) {
            return false;
        }
        self.attempt_history.attempts().iter().any(|attempt| {
            if attempt.outcome() != crate::campaign_history::MissionAttemptOutcome::Won {
                return false;
            }
            if attempt.source() == crate::campaign_history::MissionAttemptSource::OriginalSaveImport
            {
                return true;
            }
            let Some(results) = attempt.achievements() else {
                return true;
            };
            match results.evaluation(id) {
                AchievementEvaluation::Unverifiable => true,
                AchievementEvaluation::Failed | AchievementEvaluation::NotApplicable => false,
                AchievementEvaluation::Earned => attempt.achievement_attestation().is_none(),
            }
        })
    }

    pub const fn attempt_history(&self) -> &MissionAttemptHistory {
        &self.attempt_history
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
        match self.eligibility_failures(campaign, profiles).next() {
            Some(failure) => Err(failure.reason()),
            None => Ok(()),
        }
    }

    /// Unmet conditions in legacy evaluation order. Evaluation is lazy: simulation
    /// stops at its first failure, while presentation can inspect every failure.
    /// Missing prerequisite profiles and invalid ARES states are integrity bugs,
    /// not ordinary locked-mission reasons, and panic only when visited.
    pub fn eligibility_failures<'a>(
        &'a self,
        campaign: &'a crate::campaign::Campaign,
        profiles: &'a crate::profiles::ProfileManager,
    ) -> impl Iterator<Item = MissionEligibilityFailure> + 'a {
        use MissionEligibilityFailure as Failure;
        let p = self.profile(profiles);
        let money = campaign.get_value(crate::campaign::CampaignValue::Ransom);
        let gang_size = campaign.get_size_of_gang() as u32;
        let ares = campaign.get_ares();
        let resources = std::iter::once_with(move || {
            (money < p.min_ransom as i32).then_some(Failure::MinimumRansom {
                required: p.min_ransom,
                current: money,
            })
        })
        .chain(std::iter::once_with(move || {
            (p.max_ransom < money as u32 && p.max_ransom != 200000).then_some(
                Failure::MaximumRansom {
                    allowed: p.max_ransom,
                    current: money,
                },
            )
        }))
        .chain(std::iter::once_with(move || {
            (gang_size < p.min_gang_size as u32).then_some(Failure::MinimumGang {
                required: p.min_gang_size,
                current: gang_size,
            })
        }))
        .chain(std::iter::once_with(move || {
            ((p.max_gang_size as u32) < gang_size).then_some(Failure::MaximumGang {
                allowed: p.max_gang_size,
                current: gang_size,
            })
        }))
        .chain(std::iter::once_with(move || {
            (self.age >= p.life_time).then_some(Failure::Expired)
        }))
        .flatten();
        let story = std::iter::once_with(move || {
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
                        return Some(Failure::StoryState);
                    }
                }
            }
            None
        })
        .flatten();
        let prerequisites = p
            .missions_required_to_be_done
            .iter()
            .map(|&id| (id, true))
            .chain(
                p.missions_required_not_to_be_done
                    .iter()
                    .map(|&id| (id, false)),
            )
            .filter_map(move |(mission_id, must_be_done)| {
                // Profile IDs are not campaign-vector indices.
                let m = campaign
                    .get_mission(mission_id, profiles)
                    .unwrap_or_else(|| {
                        panic!(
                            "required mission profile id {mission_id} not found in campaign \
                     (referenced by mission profile_idx {:?})",
                            self.profile_idx
                        )
                    });
                (m.is_done() != must_be_done).then_some(Failure::Prerequisite {
                    mission_id,
                    must_be_done,
                })
            });
        resources.chain(story).chain(prerequisites)
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

    fn eligibility_fixture() -> (Mission, Campaign, ProfileManager) {
        let mut profiles = ProfileManager::new();
        profiles.missions.push(MissionProfile {
            id: 42,
            max_ransom: 200000,
            max_gang_size: u16::MAX,
            life_time: 10,
            ..Default::default()
        });
        let mission = Mission {
            profile_idx: Some(0),
            ..Mission::new()
        };
        (mission, Campaign::new(), profiles)
    }

    #[test]
    fn eligibility_numeric_boundaries_and_legacy_money_casts() {
        use crate::campaign::CampaignValue;
        use MissionEligibilityFailure as Failure;
        let (mut mission, mut campaign, mut profiles) = eligibility_fixture();
        let p = &mut profiles.missions[0];
        p.min_ransom = 100;
        p.max_ransom = 100;
        p.min_gang_size = 2;
        p.max_gang_size = 2;
        campaign.gang_indices = vec![0, 1];
        mission.age = 9;
        campaign.set_value(CampaignValue::Ransom, 100);
        assert!(mission.is_accessible(&campaign, &profiles));
        campaign.set_value(CampaignValue::Ransom, 99);
        assert_eq!(
            mission
                .eligibility_failures(&campaign, &profiles)
                .collect::<Vec<_>>(),
            [Failure::MinimumRansom {
                required: 100,
                current: 99
            }]
        );
        campaign.set_value(CampaignValue::Ransom, 101);
        assert_eq!(
            mission.eligibility_failures(&campaign, &profiles).next(),
            Some(Failure::MaximumRansom {
                allowed: 100,
                current: 101
            })
        );
        campaign.set_value(CampaignValue::Ransom, 100);
        campaign.gang_indices.pop();
        assert_eq!(
            mission.eligibility_failures(&campaign, &profiles).next(),
            Some(Failure::MinimumGang {
                required: 2,
                current: 1
            })
        );
        campaign.gang_indices.extend([1, 2]);
        assert_eq!(
            mission.eligibility_failures(&campaign, &profiles).next(),
            Some(Failure::MaximumGang {
                allowed: 2,
                current: 3
            })
        );
        campaign.gang_indices.pop();
        mission.age = 10;
        assert_eq!(
            mission.eligibility_failures(&campaign, &profiles).next(),
            Some(Failure::Expired)
        );
        mission.age = 0;
        profiles.missions[0].max_ransom = 200000;
        campaign.set_value(CampaignValue::Ransom, i32::MAX);
        assert!(mission.is_accessible(&campaign, &profiles));
        profiles.missions[0].min_ransom = u32::MAX;
        profiles.missions[0].max_ransom = 100;
        campaign.set_value(CampaignValue::Ransom, -1);
        // Preserve original signed minimum and unsigned maximum comparisons.
        assert_eq!(
            mission
                .eligibility_failures(&campaign, &profiles)
                .collect::<Vec<_>>(),
            [Failure::MaximumRansom {
                allowed: 100,
                current: -1
            }]
        );
    }

    #[test]
    fn all_failures_preserve_order_and_prerequisite_profile_identity() {
        use crate::campaign::CampaignValue;
        use MissionEligibilityFailure as Failure;
        let (mut mission, mut campaign, mut profiles) = eligibility_fixture();
        profiles.missions.push(MissionProfile {
            id: 700,
            ..Default::default()
        });
        profiles.missions.push(MissionProfile {
            id: 800,
            ..Default::default()
        });
        // Neither vector order nor profile indices match prerequisite IDs.
        campaign.missions = vec![
            Mission {
                profile_idx: Some(2),
                status: MissionStatus::Lost,
                ..Mission::new()
            },
            Mission {
                profile_idx: Some(1),
                ..Mission::new()
            },
        ];
        let p = &mut profiles.missions[0];
        p.min_ransom = 100;
        p.max_ransom = 10;
        p.min_gang_size = 3;
        p.max_gang_size = 1;
        p.ares_sensible = true;
        p.available_in_ares_state = [false; 10];
        p.missions_required_to_be_done = vec![700];
        p.missions_required_not_to_be_done = vec![800];
        campaign.gang_indices = vec![0, 1];
        campaign.set_value(CampaignValue::Ransom, 50);
        campaign.set_ares(9);
        mission.age = 10;
        let failures = mission
            .eligibility_failures(&campaign, &profiles)
            .collect::<Vec<_>>();
        assert_eq!(
            failures,
            [
                Failure::MinimumRansom {
                    required: 100,
                    current: 50
                },
                Failure::MaximumRansom {
                    allowed: 10,
                    current: 50
                },
                Failure::MinimumGang {
                    required: 3,
                    current: 2
                },
                Failure::MaximumGang {
                    allowed: 1,
                    current: 2
                },
                Failure::Expired,
                Failure::StoryState,
                Failure::Prerequisite {
                    mission_id: 700,
                    must_be_done: true
                },
                Failure::Prerequisite {
                    mission_id: 800,
                    must_be_done: false
                },
            ]
        );
        assert_eq!(
            mission.is_accessible_why(&campaign, &profiles),
            Err(failures[0].reason())
        );
    }

    #[test]
    fn first_failure_does_not_visit_corrupt_later_conditions() {
        let (mission, mut campaign, mut profiles) = eligibility_fixture();
        campaign.set_value(crate::campaign::CampaignValue::Ransom, 0);
        profiles.missions[0].min_ransom = 1;
        profiles.missions[0].ares_sensible = true;
        profiles.missions[0].missions_required_to_be_done = vec![999];
        campaign.set_ares(-2);
        assert_eq!(
            mission.is_accessible_why(&campaign, &profiles),
            Err("Not enough money")
        );
        profiles.missions[0].min_ransom = 0;
        profiles.missions[0].ares_sensible = false;
        profiles.missions.push(MissionProfile {
            id: 700,
            ..Default::default()
        });
        campaign.missions.push(Mission {
            profile_idx: Some(1),
            ..Mission::new()
        });
        profiles.missions[0].missions_required_to_be_done = vec![700, 999];
        profiles.missions[0].missions_required_not_to_be_done = vec![998];
        assert_eq!(
            mission.is_accessible_why(&campaign, &profiles),
            Err("Some missions are required to be played")
        );
        profiles.missions[0].missions_required_to_be_done.clear();
        profiles.missions[0].missions_required_not_to_be_done = vec![700, 999];
        campaign.missions[0].lose();
        assert_eq!(
            mission.is_accessible_why(&campaign, &profiles),
            Err("Some missions are required not to be played")
        );
    }

    #[test]
    #[should_panic(
        expected = "required mission profile id 999 not found in campaign (referenced by mission profile_idx Some(0))"
    )]
    fn visited_missing_prerequisite_reports_identity() {
        let (mission, campaign, mut profiles) = eligibility_fixture();
        profiles.missions[0].missions_required_to_be_done = vec![999];
        mission.eligibility_failures(&campaign, &profiles).next();
    }

    #[test]
    #[should_panic(
        expected = "campaign ARES state 10 out of bounds for mission profile_idx Some(0)"
    )]
    fn visited_out_of_range_story_state_reports_identity() {
        let (mission, mut campaign, mut profiles) = eligibility_fixture();
        profiles.missions[0].ares_sensible = true;
        campaign.set_ares(10);
        mission.is_accessible_why(&campaign, &profiles).unwrap();
    }

    #[test]
    #[should_panic(
        expected = "campaign ARES state -2 out of bounds for mission profile_idx Some(0)"
    )]
    fn invalid_negative_story_state_is_not_the_sentinel() {
        let (mission, mut campaign, mut profiles) = eligibility_fixture();
        profiles.missions[0].ares_sensible = true;
        campaign.set_ares(-2);
        mission.is_accessible_why(&campaign, &profiles).unwrap();
    }

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
