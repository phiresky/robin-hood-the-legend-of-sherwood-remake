//! Cooperative party construction and selection policy.
use super::EngineInner;
use crate::{
    coop::CharacterControl,
    element::{Entity, EntityId},
    player_command::PlayerId,
};

impl EngineInner {
    pub(super) fn initialize_coop_party(&mut self) {
        let rules = self.control.sim_config.coop;
        rules.validate().expect("invalid cooperative mission rules");
        if rules.players == 1 && rules.control == CharacterControl::Shared {
            return;
        }
        let originals: Vec<_> = self
            .world
            .pc_ids
            .iter()
            .copied()
            .filter(|&id| {
                self.get_entity(id)
                    .and_then(Entity::pc_data)
                    .is_some_and(|pc| {
                        pc.playable
                            && pc.mission_role == crate::human_control::MissionRole::PlayerParty
                            && pc.coop_origin.is_none()
                    })
            })
            .collect();
        if originals.is_empty() {
            tracing::warn!("co-op mission has no playable party to duplicate");
            return;
        }
        let mut party = originals.clone();
        while party.len() < rules.players as usize {
            let slot = party.len();
            let source = originals
                .get(usize::from(rules.duplicate_choices[slot]))
                .copied()
                .unwrap_or_else(|| {
                    tracing::warn!(
                        slot,
                        "chosen co-op duplicate is unavailable; using first mission hero"
                    );
                    originals[0]
                });
            let mut copy = self
                .get_entity(source)
                .expect("party source exists")
                .clone();
            let Entity::Pc(pc) = &mut copy else {
                unreachable!()
            };
            let description =
                pc.pc
                    .campaign_description_index
                    .expect("co-op hero requires campaign identity") as usize;
            let description = self.mission_domain.campaign.characters[description].clone();
            pc.pc.campaign_description_index =
                Some(self.mission_domain.campaign.characters.len() as u32);
            self.mission_domain.campaign.characters.push(description);
            pc.pc.coop_origin = Some(source);
            pc.pc.list_index = self.world.pc_ids.len() as u8;
            let id = self.add_entity(copy);
            self.add_detectable_for_all_npc(id, crate::element::DetectableType::Enemy);
            party.push(id);
        }
        let percent = self.coop_enemy_health_percent();
        let ids = self.world.soldier_registry.all().to_vec();
        for id in ids {
            let id = EntityId::Soldier(crate::element::SoldierId(id));
            if self.fog_entity_is_hostile(id) {
                if let Some(entity) = self.world.entities.get_mut(id) {
                    Self::scale_coop_enemy_health(entity, percent);
                }
            }
        }
        let mut assigned_party = Vec::new();
        for index in 0..rules.players as usize {
            self.ensure_seat(PlayerId(index as u8));
            let assigned = party
                .get(usize::from(rules.assignments[index]))
                .copied()
                .filter(|id| !assigned_party.contains(id))
                .unwrap_or_else(|| {
                    tracing::warn!(
                        index,
                        "co-op assignment unavailable; using first unassigned hero"
                    );
                    *party
                        .iter()
                        .find(|id| !assigned_party.contains(id))
                        .expect("party fills player slots")
                });
            assigned_party.push(assigned);
            self.players.seats[index].assigned_character = Some(assigned);
            self.players.seats[index].selection = vec![assigned];
            self.players.seats[index].follow_element = Some(assigned);
            self.players.seats[index].locker_active = true;
        }
    }

    pub(super) fn coop_enemy_health_percent(&self) -> i32 {
        if self.control.sim_config.coop.players <= 1
            || self.control.sim_config.coop.enemy_health_per_duplicate == 0
        {
            return 100;
        }
        let copies = self
            .world
            .pc_ids
            .iter()
            .filter(|&&id| {
                self.get_entity(id)
                    .and_then(Entity::pc_data)
                    .is_some_and(|pc| pc.coop_origin.is_some())
            })
            .count();
        100 + copies as i32 * i32::from(self.control.sim_config.coop.enemy_health_per_duplicate)
    }

    pub(super) fn scale_coop_enemy_health(entity: &mut Entity, percent: i32) {
        if let Entity::Soldier(soldier) = entity {
            let scale = |hp: i16| {
                if hp > 0 {
                    ((i32::from(hp) * percent + 99) / 100).min(i16::MAX as i32) as i16
                } else {
                    hp
                }
            };
            soldier.npc.life_points = scale(soldier.npc.life_points);
            soldier.soldier.cached_max_life_points = scale(soldier.soldier.cached_max_life_points);
        }
    }

    /// Export campaign progress without temporary cooperative party members.
    /// A surviving copy continues the required hero when its first instance died.
    pub fn export_coop_campaign(&self) -> crate::campaign::Campaign {
        let mut campaign = self.mission_domain.campaign.clone();
        let mut temporary = std::collections::BTreeSet::new();
        let mut continued = std::collections::BTreeSet::new();
        for &id in &self.world.pc_ids {
            let Some(copy) = self.get_entity(id).and_then(Entity::pc_data) else {
                continue;
            };
            let Some(origin) = copy.coop_origin else {
                continue;
            };
            let Some(original) = self.get_entity(origin).and_then(Entity::pc_data) else {
                continue;
            };
            let source = copy
                .campaign_description_index
                .expect("copy campaign identity") as usize;
            let target = original
                .campaign_description_index
                .expect("original campaign identity") as usize;
            // A restored pre-mission checkpoint already excludes temporary entries.
            if source >= campaign.characters.len() {
                continue;
            }
            temporary.insert(source);
            if original.life_points <= 0 && copy.life_points > 0 && continued.insert(target) {
                campaign.characters[target].status = campaign.characters[source].status.clone();
                campaign.characters[target].status.life_points = copy.life_points;
            }
        }
        let remap = |index: usize| {
            (!temporary.contains(&index)).then(|| index - temporary.range(..index).count())
        };
        campaign.characters = campaign
            .characters
            .into_iter()
            .enumerate()
            .filter_map(|(index, character)| (!temporary.contains(&index)).then_some(character))
            .collect();
        for list in [
            &mut campaign.gang_indices,
            &mut campaign.reservist_indices,
            &mut campaign.mission_team_indices,
        ] {
            *list = list.iter().filter_map(|&index| remap(index)).collect();
        }
        for set in [
            &mut campaign.deeds.lost_members,
            &mut campaign.deeds.contributing_veterans,
            &mut campaign.deeds.workers,
        ] {
            *set = set.iter().filter_map(|&index| remap(index)).collect();
        }
        campaign
    }

    pub(super) fn coop_copy_survives(&self, victim: EntityId) -> bool {
        if self.control.sim_config.coop.players <= 1 {
            return false;
        }
        let Some(pc) = self.get_entity(victim).and_then(Entity::pc_data) else {
            return false;
        };
        let origin = pc.coop_origin.unwrap_or(victim);
        self.world.pc_ids.iter().copied().any(|id| {
            id != victim
                && self
                    .get_entity(id)
                    .and_then(Entity::pc_data)
                    .is_some_and(|other| {
                        other.coop_origin.unwrap_or(id) == origin && other.life_points > 0
                    })
        })
    }

    pub fn coop_can_select(&self, seat: usize, id: EntityId) -> bool {
        match self.control.sim_config.coop.control {
            CharacterControl::Shared => true,
            CharacterControl::Exclusive => {
                !self.players.seats.iter().enumerate().any(|(other, state)| {
                    other != seat && state.is_active(other) && state.selection.contains(&id)
                })
            }
            CharacterControl::Assigned => self
                .players
                .seats
                .get(seat)
                .is_some_and(|state| state.assigned_character == Some(id)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::Posture;
    use crate::engine::test_support::actors::make_test_pc;

    fn party(count: usize, players: u8) -> EngineInner {
        let mut engine = EngineInner::new();
        engine.control.sim_config.coop.players = players;
        for _ in 0..count {
            let mut pc = make_test_pc(Posture::Upright);
            if let Entity::Pc(pc) = &mut pc {
                pc.pc.playable = true;
                pc.pc.life_points = 100;
            }
            engine.add_test_entity(pc);
        }
        engine.initialize_coop_party();
        engine
    }
    #[test]
    fn fills_five_slots_and_keeps_one_hero_alive_until_last_copy_dies() {
        let mut engine = party(1, 5);
        assert_eq!(engine.world.pc_ids.len(), 5);
        let ids = engine.world.pc_ids.clone();
        for &id in &ids[..4] {
            if let Some(Entity::Pc(pc)) = engine.world.entities.get_mut(id) {
                pc.pc.life_points = 0;
            }
            assert!(engine.coop_copy_survives(id));
        }
        if let Some(Entity::Pc(pc)) = engine.world.entities.get_mut(ids[4]) {
            pc.pc.life_points = 0;
        }
        for id in ids {
            assert!(!engine.coop_copy_survives(id));
        }
    }
    #[test]
    fn ownership_rules_and_disconnect() {
        let mut engine = party(2, 2);
        let other = engine.world.pc_ids[1];
        engine.players.seats[1].connected = true;
        assert!(engine.coop_can_select(0, other));
        engine.control.sim_config.coop.control = CharacterControl::Exclusive;
        assert!(!engine.coop_can_select(0, other));
        assert!(engine.coop_can_select(1, other));
        engine.players.seats[1].connected = false;
        assert!(engine.coop_can_select(0, other));
        engine.control.sim_config.coop.control = CharacterControl::Assigned;
        assert!(!engine.coop_can_select(0, other));
        assert!(engine.coop_can_select(1, other));
    }
    #[test]
    fn existing_full_party_is_not_duplicated() {
        assert_eq!(party(5, 2).world.pc_ids.len(), 5);
    }
    #[test]
    fn copies_have_independent_coma_state_and_export_one_survivor() {
        let mut engine = party(1, 3);
        let ids = engine.world.pc_ids.clone();
        let descriptions: Vec<_> = ids
            .iter()
            .map(|&id| {
                engine
                    .get_entity(id)
                    .unwrap()
                    .pc_data()
                    .unwrap()
                    .campaign_description_index
                    .unwrap() as usize
            })
            .collect();
        assert_eq!(descriptions, vec![0, 1, 2]);
        engine.mission_domain.campaign.characters[0].status.in_coma = true;
        assert!(!engine.mission_domain.campaign.characters[1].status.in_coma);
        engine
            .world
            .entities
            .get_mut(ids[0])
            .unwrap()
            .pc_data_mut()
            .unwrap()
            .life_points = 0;
        engine.mission_domain.campaign.gang_indices = vec![0];
        engine.mission_domain.campaign.mission_team_indices = vec![0];
        // A recruit added after temporary copies must keep its index references.
        engine
            .mission_domain
            .campaign
            .characters
            .push(Default::default());
        engine.mission_domain.campaign.gang_indices.push(3);
        engine.mission_domain.campaign.deeds.workers.insert(3);
        let exported = engine.export_coop_campaign();
        assert_eq!(exported.characters.len(), 2);
        assert_eq!(exported.characters[0].status.life_points, 100);
        assert!(!exported.characters[0].status.in_coma);
        assert_eq!(exported.gang_indices, vec![0, 1]);
        assert!(exported.deeds.workers.contains(&1));
    }

    #[test]
    fn reinforcements_scale_only_when_hostile_and_never_revive_dead_soldiers() {
        let mut engine = party(1, 3);
        let make = |camp, hp| {
            let mut soldier =
                crate::engine::test_support::actors::make_test_soldier(Posture::Upright);
            let Entity::Soldier(data) = &mut soldier else {
                unreachable!()
            };
            data.soldier.cached_camp = camp;
            data.npc.life_points = hp;
            data.soldier.cached_max_life_points = 100;
            soldier
        };
        let hostile = engine.add_test_entity(make(crate::element::Camp::Lacklandists, 100));
        let ally = engine.add_test_entity(make(crate::element::Camp::Royalists, 100));
        let dead = engine.add_test_entity(make(crate::element::Camp::Lacklandists, 0));
        assert_eq!(
            engine
                .get_entity(hostile)
                .unwrap()
                .npc_data()
                .unwrap()
                .life_points,
            150
        );
        assert_eq!(
            engine
                .get_entity(ally)
                .unwrap()
                .npc_data()
                .unwrap()
                .life_points,
            100
        );
        assert_eq!(
            engine
                .get_entity(dead)
                .unwrap()
                .npc_data()
                .unwrap()
                .life_points,
            0
        );
    }

    #[test]
    fn unavailable_assignment_falls_back_without_sharing_ownership() {
        let mut engine = party(2, 2);
        engine.control.sim_config.coop.assignments = [4, 3, 2, 1, 0];
        engine.initialize_coop_party();
        assert_ne!(
            engine.players.seats[0].assigned_character,
            engine.players.seats[1].assigned_character
        );
    }

    #[test]
    fn assigned_single_player_can_select_assigned_hero() {
        let mut engine = party(1, 1);
        engine.control.sim_config.coop.control = CharacterControl::Assigned;
        engine.initialize_coop_party();
        assert!(engine.coop_can_select(0, engine.world.pc_ids[0]));
    }
}
