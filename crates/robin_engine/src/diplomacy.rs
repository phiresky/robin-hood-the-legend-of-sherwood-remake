//! Deterministic mission diplomacy.
//!
//! The original game has two camps and treats different camps as enemies.
//! Hackable missions retain that rule by default, but may override any pair
//! with a symmetric allied/neutral/hostile relationship.  Runtime changes
//! live in authoritative engine state, so saves, rollback, replays and
//! multiplayer hashes all observe the same matrix.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::element_kinds::Camp;

/// Relationship between two valid allegiances.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
#[serde(rename_all = "snake_case")]
pub enum Relationship {
    Allied,
    Neutral,
    Hostile,
}

impl Relationship {
    /// Stable script/console representation: allied=0, neutral=1, hostile=2.
    pub fn from_script_value(value: i32) -> Result<Self, String> {
        match value {
            0 => Ok(Self::Allied),
            1 => Ok(Self::Neutral),
            2 => Ok(Self::Hostile),
            _ => Err(format!(
                "invalid diplomacy relationship {value}; expected 0 (allied), 1 (neutral), or 2 (hostile)"
            )),
        }
    }

    pub const fn script_value(self) -> i32 {
        match self {
            Self::Allied => 0,
            Self::Neutral => 1,
            Self::Hostile => 2,
        }
    }
}

/// One JSON-authored relationship override.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
#[serde(deny_unknown_fields)]
pub struct DiplomacyRule {
    pub first: u16,
    pub second: u16,
    pub relationship: Relationship,
}

/// Editable diplomacy block in a hackable level descriptor.
#[derive(
    Debug,
    Clone,
    Default,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
#[serde(deny_unknown_fields)]
pub struct DiplomacyDefinition {
    /// Allegiances controlled by the players. Every pair in the coalition is
    /// allied, and all members are considered player-aligned for UI/stats.
    #[serde(default)]
    pub player_coalition: Vec<u16>,
    #[serde(default)]
    pub relationships: Vec<DiplomacyRule>,
}

/// Validated authoritative relationship matrix.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct DiplomacyState {
    enabled: bool,
    npc_faction_wars: bool,
    player_coalition: BTreeSet<u16>,
    relationships: BTreeMap<(u16, u16), Relationship>,
    revision: u64,
}

impl Default for DiplomacyState {
    fn default() -> Self {
        let mut player_coalition = BTreeSet::new();
        player_coalition.insert(Camp::ROYALIST_ID);
        Self {
            enabled: false,
            npc_faction_wars: true,
            player_coalition,
            relationships: BTreeMap::new(),
            revision: 0,
        }
    }
}

impl DiplomacyState {
    pub fn from_definition(
        enabled: bool,
        npc_faction_wars: bool,
        definition: Option<&DiplomacyDefinition>,
    ) -> Result<Self, String> {
        let mut state = Self {
            enabled,
            npc_faction_wars,
            ..Self::default()
        };
        let Some(definition) = definition else {
            return Ok(state);
        };

        if !definition.player_coalition.is_empty() {
            state.player_coalition.clear();
            state
                .player_coalition
                .extend(definition.player_coalition.iter().copied());
        }
        for &first in &state.player_coalition {
            for &second in &state.player_coalition {
                if first < second {
                    state
                        .relationships
                        .insert((first, second), Relationship::Allied);
                }
            }
        }
        for rule in &definition.relationships {
            state.set_relationship_ids(rule.first, rule.second, rule.relationship)?;
        }
        Ok(state)
    }

    pub const fn enabled(&self) -> bool {
        self.enabled
    }

    pub const fn revision(&self) -> u64 {
        self.revision
    }

    pub const fn npc_faction_wars(&self) -> bool {
        self.npc_faction_wars
    }

    pub fn set_npc_faction_wars(&mut self, enabled: bool) {
        if self.npc_faction_wars != enabled {
            self.npc_faction_wars = enabled;
            self.revision = self.revision.wrapping_add(1);
        }
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        if self.enabled != enabled {
            self.enabled = enabled;
            self.revision = self.revision.wrapping_add(1);
        }
    }

    fn valid_id(camp: Camp) -> u16 {
        camp.allegiance_id()
            .unwrap_or_else(|| panic!("diplomacy query requires a valid allegiance, got {camp:?}"))
    }

    fn key(first: u16, second: u16) -> (u16, u16) {
        if first <= second {
            (first, second)
        } else {
            (second, first)
        }
    }

    pub fn relationship(&self, first: Camp, second: Camp) -> Relationship {
        self.relationship_ids(Self::valid_id(first), Self::valid_id(second))
    }

    pub fn relationship_ids(&self, first: u16, second: u16) -> Relationship {
        if first == second {
            return Relationship::Allied;
        }
        if !self.enabled {
            return Relationship::Hostile;
        }
        self.relationships
            .get(&Self::key(first, second))
            .copied()
            .unwrap_or(Relationship::Hostile)
    }

    pub fn is_hostile(&self, first: Camp, second: Camp) -> bool {
        self.relationship(first, second) == Relationship::Hostile
    }

    pub fn is_allied(&self, first: Camp, second: Camp) -> bool {
        self.relationship(first, second) == Relationship::Allied
    }

    pub fn actors_may_fight(
        &self,
        first_camp: Camp,
        first_is_pc: bool,
        second_camp: Camp,
        second_is_pc: bool,
    ) -> bool {
        self.is_hostile(first_camp, second_camp)
            && (self.npc_faction_wars || first_is_pc || second_is_pc)
    }

    pub fn is_player_aligned(&self, camp: Camp) -> bool {
        if !self.enabled {
            return Self::valid_id(camp) == Camp::ROYALIST_ID;
        }
        self.player_coalition.contains(&Self::valid_id(camp))
    }

    pub fn player_coalition(&self) -> &BTreeSet<u16> {
        &self.player_coalition
    }

    pub fn set_relationship(
        &mut self,
        first: Camp,
        second: Camp,
        relationship: Relationship,
    ) -> Result<(), String> {
        self.set_relationship_ids(Self::valid_id(first), Self::valid_id(second), relationship)
    }

    pub fn set_relationship_ids(
        &mut self,
        first: u16,
        second: u16,
        relationship: Relationship,
    ) -> Result<(), String> {
        if first == second {
            if relationship != Relationship::Allied {
                return Err(format!("allegiance {first} must remain allied with itself"));
            }
            return Ok(());
        }
        if self.player_coalition.contains(&first)
            && self.player_coalition.contains(&second)
            && relationship != Relationship::Allied
        {
            return Err(format!(
                "player-coalition allegiances {first} and {second} must remain allied"
            ));
        }
        let key = Self::key(first, second);
        if self.relationships.insert(key, relationship) != Some(relationship) {
            self.revision = self.revision.wrapping_add(1);
        }
        Ok(())
    }
}

/// Rebuild relationship-derived entity caches after an authoritative matrix
/// edit. Kept outside `EngineInner` so both frame commands and synchronous Lua
/// natives execute the identical transition.
pub(crate) fn reconcile_entities(
    entities: &mut crate::entities::Entities,
    diplomacy: &DiplomacyState,
) {
    use crate::element::{Detectable, DetectableType, Entity, EntityId};

    let humans = entities
        .actors()
        .filter_map(|(id, entity)| {
            entity.is_human().then_some((
                EntityId::from(id),
                entity.camp(),
                entity.is_pc(),
                entity.is_soldier(),
            ))
        })
        .collect::<Vec<_>>();
    let camps = humans
        .iter()
        .map(|(id, camp, _, _)| (*id, *camp))
        .collect::<BTreeMap<_, _>>();
    let actors_by_handle = humans
        .iter()
        .map(|(id, camp, is_pc, _)| (id.index(), (*camp, *is_pc)))
        .collect::<BTreeMap<_, _>>();

    for (id, own_camp, own_is_pc, _) in &humans {
        let entity = entities
            .get_mut(*id)
            .unwrap_or_else(|| panic!("diplomacy reconciliation actor {id:?} disappeared"));
        if let Some(human) = entity.human_data_mut() {
            let retained = human
                .opponents
                .iter_with_jump_lines()
                .filter(|(opponent, _)| {
                    humans
                        .iter()
                        .find(|(id, _, _, _)| id == opponent)
                        .is_some_and(|(_, camp, is_pc, _)| {
                            diplomacy.actors_may_fight(*own_camp, *own_is_pc, *camp, *is_pc)
                        })
                })
                .collect::<Vec<_>>();
            human.opponents = crate::element::SwordfightOpponents::from_pairs(retained);
        }
    }

    let npc_ids = entities.npc_ids().collect::<Vec<_>>();
    for npc_id in npc_ids {
        let (npc_camp, npc_is_soldier) = {
            let npc = entities
                .get(npc_id)
                .unwrap_or_else(|| panic!("diplomacy NPC {npc_id:?} disappeared"));
            (npc.camp(), npc.is_soldier())
        };
        let npc = entities
            .get_mut(npc_id)
            .and_then(Entity::ai_actor_data_mut)
            .unwrap_or_else(|| panic!("diplomacy NPC {npc_id:?} has no AI actor data"));
        let enemies = &mut npc.detectable_lists[DetectableType::Enemy as usize];
        enemies.retain(|detectable| {
            detectable
                .element
                .and_then(|id| camps.get(&id).copied())
                .is_some_and(|camp| diplomacy.is_hostile(npc_camp, camp))
        });
        for (target_id, target_camp, target_is_pc, target_is_soldier) in &humans {
            if *target_id == npc_id
                || !crate::ai_detectable_filter::should_add_enemy_detectable_with(
                    diplomacy,
                    npc_camp,
                    npc_is_soldier,
                    *target_is_pc,
                    *target_is_soldier,
                    *target_camp,
                )
                || enemies
                    .iter()
                    .any(|detectable| detectable.element == Some(*target_id))
            {
                continue;
            }
            enemies.push(Detectable {
                element: Some(*target_id),
                detectable_type: DetectableType::Enemy,
                seen_last_frame: false,
                heard_last_frame: false,
                seen_now: false,
                shadow_seen_now: false,
                shadow_seen_last_frame: false,
                last_visibility: 0.0,
            });
        }
        for detectable_type in [DetectableType::Friend, DetectableType::MissedFriend] {
            npc.detectable_lists[detectable_type as usize].retain(|detectable| {
                detectable
                    .element
                    .and_then(|id| camps.get(&id).copied())
                    .is_some_and(|camp| diplomacy.is_allied(npc_camp, camp))
            });
        }
        if let Some(enemy) = npc.ai_brain.enemy_mut() {
            enemy.base.list_us.retain(|handle| {
                actors_by_handle
                    .get(handle)
                    .is_some_and(|(camp, _)| diplomacy.is_allied(npc_camp, *camp))
            });
            enemy.list_them.retain(|handle| {
                actors_by_handle.get(handle).is_some_and(|(camp, is_pc)| {
                    diplomacy.actors_may_fight(npc_camp, false, *camp, *is_pc)
                })
            });
            if enemy.base.primary_target != 0
                && !actors_by_handle
                    .get(&enemy.base.primary_target)
                    .is_some_and(|(camp, is_pc)| {
                        diplomacy.actors_may_fight(npc_camp, false, *camp, *is_pc)
                    })
            {
                enemy.base.primary_target = 0;
                enemy.base.outbox.actor.set_focus(0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_preserves_distinct_id_hostility() {
        let state = DiplomacyState::default();
        assert!(state.is_allied(Camp::Custom(7), Camp::Custom(7)));
        assert!(state.is_hostile(Camp::Royalists, Camp::Custom(7)));
    }

    #[test]
    fn authored_relationships_are_symmetric_and_coalition_is_allied() {
        let state = DiplomacyState::from_definition(
            true,
            true,
            Some(&DiplomacyDefinition {
                player_coalition: vec![0, 4],
                relationships: vec![DiplomacyRule {
                    first: 2,
                    second: 3,
                    relationship: Relationship::Neutral,
                }],
            }),
        )
        .unwrap();
        assert!(state.is_allied(Camp::Royalists, Camp::Custom(4)));
        assert_eq!(
            state.relationship(Camp::Custom(3), Camp::Custom(2)),
            Relationship::Neutral
        );
        assert!(state.is_player_aligned(Camp::Custom(4)));
    }

    #[test]
    fn same_allegiance_cannot_be_made_hostile() {
        let mut state = DiplomacyState::default();
        assert!(
            state
                .set_relationship(Camp::Custom(9), Camp::Custom(9), Relationship::Hostile)
                .is_err()
        );
    }

    #[test]
    fn player_coalition_cannot_be_split_by_authored_or_runtime_rule() {
        let definition = DiplomacyDefinition {
            player_coalition: vec![0, 4],
            relationships: vec![DiplomacyRule {
                first: 0,
                second: 4,
                relationship: Relationship::Neutral,
            }],
        };
        assert!(DiplomacyState::from_definition(true, true, Some(&definition)).is_err());

        let mut state = DiplomacyState::from_definition(
            true,
            true,
            Some(&DiplomacyDefinition {
                player_coalition: vec![0, 4],
                relationships: Vec::new(),
            }),
        )
        .unwrap();
        assert!(
            state
                .set_relationship_ids(0, 4, Relationship::Hostile)
                .is_err()
        );
    }

    #[test]
    fn disabling_diplomacy_restores_legacy_player_alignment() {
        let mut state = DiplomacyState::from_definition(
            true,
            true,
            Some(&DiplomacyDefinition {
                player_coalition: vec![0, 4],
                relationships: Vec::new(),
            }),
        )
        .unwrap();
        assert!(state.is_player_aligned(Camp::Custom(4)));
        state.set_enabled(false);
        assert!(!state.is_player_aligned(Camp::Custom(4)));
        assert!(state.is_player_aligned(Camp::Royalists));
    }
}
