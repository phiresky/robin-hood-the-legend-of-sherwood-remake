//! Mission debriefing statistics and mission resource IDs.
//!
//! Tracks per-mission counters shown in the debriefing screen: money collected,
//! soldiers killed/surviving, peasants recruited, score, and which PCs joined
//! the gang during the mission.

use std::collections::BTreeMap;

use crate::{element::Camp, pc_status::SpecialPeasantName};
use serde::{Deserialize, Serialize};

// ── Mission resource IDs ────────────────────────────────────────────────────

pub const RHID_DEFAULT_MISSION_PICTURE: u32 = 10001;
pub const RHID_HA: u32 = 10002;

// ── PcStatName ──────────────────────────────────────────────────────────────

/// Display-name entry captured into [`MissionStat::pc_names`] when a PC
/// joins the gang.  Carries the profile-based fallback string plus the
/// optional `PROP_NAME` SPECIAL_PEASANT override; the override is
/// resolved against the host menu-text table at debriefing render time
/// (the menu-text lookup is deferred because the native context doesn't carry a
/// `MenuTextLookup`).
#[derive(
    Debug,
    Clone,
    Default,
    Serialize,
    Deserialize,
    PartialEq,
    Eq,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct PcStatName {
    /// Profile name used as the removal key and as the displayed string
    /// when no override is set.
    pub fallback: String,
    /// Optional SPECIAL_PEASANT_A/B/C override applied via the
    /// `PROP_NAME` script native.
    pub name_override: Option<SpecialPeasantName>,
}

impl PcStatName {
    pub fn new(fallback: String, name_override: Option<SpecialPeasantName>) -> Self {
        Self {
            fallback,
            name_override,
        }
    }
}

// ── MissionStat ─────────────────────────────────────────────────────────────

/// Lossless per-allegiance soldier history for diplomacy-aware missions.
///
/// The legacy aggregate counters remain populated for campaign/debriefing
/// compatibility. This table preserves which faction each soldier belonged to
/// and whether a player character caused the death, including neutral and
/// allied casualties.
#[derive(
    Debug,
    Clone,
    Default,
    Serialize,
    Deserialize,
    PartialEq,
    Eq,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct FactionMissionStat {
    pub encountered_soldiers: u32,
    pub living_soldiers_at_end: u32,
    pub soldier_deaths: u32,
    pub player_caused_soldier_deaths: u32,
}

/// Per-mission debriefing statistics.
#[derive(
    Debug,
    Clone,
    Default,
    Serialize,
    Deserialize,
    PartialEq,
    Eq,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct MissionStat {
    /// Money collected by the player during the mission.
    pub collected_money: u32,
    /// Bonus money present in the level.
    pub bonus_money: u32,
    /// Money carried by soldiers in the level.
    pub soldier_money: u32,
    /// Number of enemy soldiers still alive at mission end.
    pub living_soldier_count: u32,
    /// Total number of enemy soldiers that were in the mission.
    pub total_soldier_count: u32,
    /// Number of new peasants recruited during the mission.
    pub new_peasant_count: u32,
    /// Number of peasants killed during the mission.
    pub killed_peasant_count: u32,
    /// Number of allied soldiers killed during the mission.
    pub killed_allied_count: u32,
    /// Score added for this mission.
    pub added_score: u32,
    /// Names of PCs who joined the gang during this mission.  Each
    /// entry carries the profile-name fallback plus an optional
    /// `PROP_NAME` override that's resolved through `MenuTextLookup`
    /// at debriefing render time.
    pub pc_names: Vec<PcStatName>,
    /// Deterministically ordered, lossless soldier history keyed by mission
    /// allegiance ID. Missing in legacy saves and therefore defaults empty.
    #[serde(default)]
    pub factions: BTreeMap<u16, FactionMissionStat>,
}

impl MissionStat {
    /// Reset all statistics to zero / empty.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Increment the killed-peasant counter.
    pub fn add_killed_peasant(&mut self) {
        self.killed_peasant_count += 1;
    }

    /// Increment the killed-allied counter.
    pub fn add_killed_allied(&mut self) {
        self.killed_allied_count += 1;
    }

    pub fn record_soldier_encounter(&mut self, camp: Camp) {
        let allegiance = camp.allegiance_id().unwrap_or_else(|| {
            panic!("cannot record soldier encounter for invalid allegiance {camp:?}")
        });
        self.factions
            .entry(allegiance)
            .or_default()
            .encountered_soldiers += 1;
    }

    pub fn record_soldier_death(&mut self, camp: Camp, caused_by_player: bool) {
        let allegiance = camp.allegiance_id().unwrap_or_else(|| {
            panic!("cannot record soldier death for invalid allegiance {camp:?}")
        });
        let faction = self.factions.entry(allegiance).or_default();
        faction.soldier_deaths += 1;
        if caused_by_player {
            faction.player_caused_soldier_deaths += 1;
        }
    }

    pub fn reset_faction_living_counts(&mut self) {
        for faction in self.factions.values_mut() {
            faction.living_soldiers_at_end = 0;
        }
    }

    pub fn set_faction_living_soldiers_at_end(&mut self, camp: Camp, count: u32) {
        let allegiance = camp.allegiance_id().unwrap_or_else(|| {
            panic!("cannot record living soldier for invalid allegiance {camp:?}")
        });
        self.factions
            .entry(allegiance)
            .or_default()
            .living_soldiers_at_end = count;
    }

    /// Add a new peasant to the recruited count.
    pub fn add_new_peasant(&mut self) {
        self.new_peasant_count += 1;
    }

    /// Add money to the collected total.  Accepts a signed delta written into
    /// the unsigned `collected_money` with wrap-around semantics — i.e. a
    /// negative delta wraps around modulo 2^32.
    pub fn add_collected_money(&mut self, amount: i32) {
        self.collected_money = self.collected_money.wrapping_add_signed(amount);
    }

    /// Add score to the per-mission added-score total.  Signed delta with
    /// wrap-around semantics on the unsigned `added_score`.
    pub fn add_score(&mut self, amount: i32) {
        self.added_score = self.added_score.wrapping_add_signed(amount);
    }

    /// Record a PC who joined the gang.
    ///
    /// `fallback` is the profile-derived display name (used as the
    /// removal key and as the rendered string when `name_override` is
    /// `None`).  `name_override` carries the SPECIAL_PEASANT slot id
    /// from a `PROP_NAME` script call, resolved through
    /// `MenuTextLookup` at render time.
    pub fn add_new_pc(&mut self, fallback: String, name_override: Option<SpecialPeasantName>) {
        self.pc_names.push(PcStatName::new(fallback, name_override));
    }

    /// Remove a PC name by its profile-name key.  Returns `true` if
    /// found and removed.  The match is against `PcStatName::fallback`
    /// — overrides don't participate (kill cascade looks up by the
    /// stable profile name, not the live override).
    pub fn remove_new_pc(&mut self, name: &str) -> bool {
        if let Some(pos) = self.pc_names.iter().position(|n| n.fallback == name) {
            self.pc_names.remove(pos);
            true
        } else {
            false
        }
    }

    /// Total money from bonuses + soldiers.
    pub fn total_level_money(&self) -> u32 {
        self.bonus_money + self.soldier_money
    }

    /// Total new members: peasants + PCs.
    pub fn total_new_members(&self) -> u32 {
        self.new_peasant_count + self.pc_names.len() as u32
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_zeroed() {
        let stat = MissionStat::default();
        assert_eq!(stat.collected_money, 0);
        assert_eq!(stat.killed_peasant_count, 0);
        assert_eq!(stat.killed_allied_count, 0);
        assert_eq!(stat.new_peasant_count, 0);
        assert_eq!(stat.added_score, 0);
        assert!(stat.pc_names.is_empty());
    }

    #[test]
    fn reset_clears_all() {
        let mut stat = MissionStat {
            collected_money: 500,
            bonus_money: 100,
            soldier_money: 200,
            living_soldier_count: 3,
            total_soldier_count: 10,
            new_peasant_count: 2,
            killed_peasant_count: 1,
            killed_allied_count: 1,
            added_score: 42,
            pc_names: vec![PcStatName::new("Robin".into(), None)],
            factions: Default::default(),
        };
        stat.reset();
        assert_eq!(stat, MissionStat::default());
    }

    #[test]
    fn add_collected_money() {
        let mut stat = MissionStat::default();
        stat.add_collected_money(100);
        stat.add_collected_money(50);
        assert_eq!(stat.collected_money, 150);
    }

    #[test]
    fn kill_counters() {
        let mut stat = MissionStat::default();
        stat.add_killed_peasant();
        stat.add_killed_peasant();
        stat.add_killed_allied();
        assert_eq!(stat.killed_peasant_count, 2);
        assert_eq!(stat.killed_allied_count, 1);
    }

    #[test]
    fn peasant_recruitment() {
        let mut stat = MissionStat::default();
        stat.add_new_peasant();
        stat.add_new_peasant();
        assert_eq!(stat.new_peasant_count, 2);
    }

    #[test]
    fn pc_add_and_remove() {
        let mut stat = MissionStat::default();
        stat.add_new_pc("Little John".into(), None);
        stat.add_new_pc("Friar Tuck".into(), None);
        assert_eq!(stat.pc_names.len(), 2);

        assert!(stat.remove_new_pc("Little John"));
        assert_eq!(
            stat.pc_names,
            vec![PcStatName::new("Friar Tuck".into(), None)]
        );

        // Removing a name that doesn't exist returns false.
        assert!(!stat.remove_new_pc("Maid Marian"));
    }

    #[test]
    fn pc_add_with_name_override() {
        let mut stat = MissionStat::default();
        stat.add_new_pc("Robin".into(), Some(SpecialPeasantName::B));
        assert_eq!(
            stat.pc_names,
            vec![PcStatName::new("Robin".into(), Some(SpecialPeasantName::B))]
        );
        // Removal still keys off the profile-name fallback.
        assert!(stat.remove_new_pc("Robin"));
        assert!(stat.pc_names.is_empty());
    }

    #[test]
    fn total_level_money() {
        let stat = MissionStat {
            bonus_money: 300,
            soldier_money: 150,
            ..Default::default()
        };
        assert_eq!(stat.total_level_money(), 450);
    }

    #[test]
    fn total_new_members() {
        let mut stat = MissionStat {
            new_peasant_count: 3,
            ..Default::default()
        };
        stat.add_new_pc("Robin".into(), None);
        stat.add_new_pc("Will Scarlet".into(), None);
        assert_eq!(stat.total_new_members(), 5);
    }

    #[test]
    fn serde_roundtrip() {
        let stat = MissionStat {
            collected_money: 999,
            bonus_money: 50,
            soldier_money: 25,
            living_soldier_count: 4,
            total_soldier_count: 12,
            new_peasant_count: 1,
            killed_peasant_count: 0,
            killed_allied_count: 2,
            added_score: 100,
            pc_names: vec![PcStatName::new(
                "Maid Marian".into(),
                Some(SpecialPeasantName::A),
            )],
            factions: Default::default(),
        };
        let json = serde_json::to_string(&stat).unwrap();
        let deser: MissionStat = serde_json::from_str(&json).unwrap();
        assert_eq!(stat, deser);
    }

    #[test]
    fn resource_constants() {
        assert_eq!(RHID_DEFAULT_MISSION_PICTURE, 10001);
        assert_eq!(RHID_HA, 10002);
    }

    #[test]
    fn faction_stats_preserve_allegiance_and_player_responsibility() {
        let mut stat = MissionStat::default();
        let neutral = Camp::Custom(7);
        stat.record_soldier_encounter(neutral);
        stat.record_soldier_encounter(neutral);
        stat.record_soldier_death(neutral, true);
        stat.set_faction_living_soldiers_at_end(neutral, 1);

        assert_eq!(
            stat.factions.get(&7),
            Some(&FactionMissionStat {
                encountered_soldiers: 2,
                living_soldiers_at_end: 1,
                soldier_deaths: 1,
                player_caused_soldier_deaths: 1,
            })
        );
    }
}
