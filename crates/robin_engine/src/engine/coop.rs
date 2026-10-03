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
        if rules.team_len() == 0 && rules.players == 1 && rules.control == CharacterControl::Shared
        {
            return;
        }
        let mut party: Vec<_> = self
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
                    })
            })
            .collect();
        // Portrait order and seat assignment must agree. Equal-priority
        // copies can otherwise move ahead of their original during the
        // level's priority sort, making both controllers appear to start on
        // the second portrait.
        party.sort_by_key(|&id| {
            self.get_entity(id)
                .and_then(Entity::pc_data)
                .is_some_and(|pc| pc.coop_origin.is_some())
        });
        let originals: Vec<_> = party
            .iter()
            .copied()
            .filter(|&id| {
                self.get_entity(id)
                    .and_then(Entity::pc_data)
                    .is_some_and(|pc| pc.coop_origin.is_none())
            })
            .collect();
        if originals.is_empty() {
            tracing::warn!("co-op mission has no playable party to duplicate");
            return;
        }
        let mut party = party;
        while rules.team_len() == 0 && party.len() < rules.players as usize {
            let slot = party.len();
            let preferred = if rules.campaign {
                originals
                    .iter()
                    .position(|&id| {
                        self.get_entity(id)
                            .and_then(Entity::pc_data)
                            .is_some_and(|pc| pc.robin)
                    })
                    .unwrap_or(0)
            } else {
                usize::from(rules.duplicate_choices[slot])
            };
            let source = originals.get(preferred).copied().unwrap_or_else(|| {
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
        // Keep the portrait order and the seat-assignment order identical.
        // Copies are appended to the entity table, but a later level or save
        // operation can reorder `pc_ids`; relying on that incidental order
        // lets both seats resolve to the duplicate portrait.
        let copy_status: std::collections::HashMap<_, _> = self
            .world
            .pc_ids
            .iter()
            .copied()
            .map(|id| {
                let is_copy = self
                    .get_entity(id)
                    .and_then(Entity::pc_data)
                    .is_some_and(|pc| pc.coop_origin.is_some());
                (id, is_copy)
            })
            .collect();
        self.world
            .pc_ids
            .sort_by_key(|id| copy_status.get(id).copied().unwrap_or(false));
        party = self
            .world
            .pc_ids
            .iter()
            .copied()
            .filter(|id| party.contains(id))
            .collect();
        if rules.team_len() > 0 {
            self.world.pc_ids.sort_by_key(|&id| {
                self.world
                    .entities
                    .get(id)
                    .and_then(Entity::pc_data)
                    .and_then(|pc| pc.campaign_description_index)
                    .unwrap_or(u32::MAX)
            });
            party.sort_by_key(|&id| {
                self.get_entity(id)
                    .and_then(Entity::pc_data)
                    .and_then(|pc| pc.campaign_description_index)
                    .expect("team identity")
            });
        }
        if rules.team_len() > 0 {
            for slot in 0..rules.team_len() {
                let origin = rules.team[..slot]
                    .iter()
                    .position(|&code| code == rules.team[slot])
                    .map(|index| party[index]);
                self.get_entity_mut(party[slot])
                    .and_then(Entity::pc_data_mut)
                    .expect("selected party member")
                    .coop_origin = origin;
            }
        }
        // The duplicate is cloned from the mission hero after the normal
        // single-player priority selection has opened that hero's portrait.
        // Co-op selection is seat-owned, so discard that inherited visual
        // state before applying the deterministic seat assignments below.
        for &id in &party {
            if let Some(Entity::Pc(pc)) = self.get_entity_mut(id) {
                pc.pc.portrait.open = false;
            }
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
            let assigned = if rules.control == CharacterControl::Shared && rules.team_len() > 0 {
                party[index % party.len()]
            } else {
                party
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
                    })
            };
            assigned_party.push(assigned);
            self.players.seats[index].assigned_character = Some(assigned);
            self.players.seats[index].selection = vec![assigned];
            if let Some(Entity::Pc(pc)) = self.get_entity_mut(assigned) {
                pc.pc.portrait.open = !pc.pc.portrait.burned;
            }
            tracing::info!(
                seat = index,
                ?assigned,
                portrait_slot = self.world.pc_ids.iter().position(|&id| id == assigned),
                "co-op initial character assignment"
            );
        }
    }

    pub(super) fn coop_enemy_health_percent(&self) -> i32 {
        if self.control.sim_config.coop.team_len() > 0 {
            return 100
                + self.control.sim_config.coop.duplicate_count() as i32
                    * i32::from(self.control.sim_config.coop.enemy_health_per_duplicate);
        }
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
        let rules = self.control.sim_config.coop;
        if rules.team_len() > 0 {
            let Some(slot) = self
                .get_entity(victim)
                .and_then(Entity::pc_data)
                .and_then(|pc| pc.campaign_description_index)
            else {
                return false;
            };
            let Some(&code) = rules.team.get(slot as usize).filter(|&&code| code != 0) else {
                return false;
            };
            return self.world.pc_ids.iter().copied().any(|id| {
                id != victim
                    && self
                        .get_entity(id)
                        .and_then(Entity::pc_data)
                        .is_some_and(|pc| {
                            pc.life_points > 0
                                && pc
                                    .campaign_description_index
                                    .and_then(|slot| rules.team.get(slot as usize))
                                    == Some(&code)
                        })
            });
        }
        if rules.players <= 1 {
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
    fn robin_copies_share_consumption_pickups_and_export_without_sharing_health() {
        use crate::profiles::{Action, CharacterProfile};
        let mut engine = party(1, 3);
        let ids = engine.world.pc_ids.clone();
        let mut assets = crate::engine::LevelAssets::new();
        std::sync::Arc::make_mut(&mut assets.profile_manager)
            .characters
            .push(CharacterProfile {
                actions: [Action::Eat, Action::Bow, Action::Purse],
                action_max_ammo: [6, 12, 6],
                ..Default::default()
            });
        for &id in &ids {
            let pc = engine
                .world
                .entities
                .get_mut(id)
                .unwrap()
                .pc_data_mut()
                .unwrap();
            pc.robin = true;
            pc.disabled_actions = vec![false; 3];
            pc.current_action = Action::Eat;
            pc.saved_action = Action::Eat;
        }
        engine.mission_domain.campaign.characters[0]
            .status
            .set_ammo(Action::Eat, 2);
        engine.mission_domain.campaign.characters[1]
            .status
            .life_points = 23;
        engine.synchronize_robin_inventory(&assets);
        engine.consume_ration_without_speech(&assets, ids[1], Action::Eat);
        engine.consume_ration_without_speech(&assets, ids[2], Action::Eat);
        for &id in &ids {
            let pc = engine.get_entity(id).unwrap().pc_data().unwrap();
            assert_eq!(
                engine
                    .pc_inventory_description(pc)
                    .unwrap()
                    .status
                    .get_ammo(Action::Eat),
                0
            );
            assert!(pc.disabled_actions[0]);
            assert_eq!(pc.current_action, Action::NoAction);
            assert_eq!(pc.saved_action, Action::NoAction);
        }
        let pickup = engine
            .handle_bonus_pickup(&assets, ids[2], Action::Eat, 20)
            .unwrap();
        assert_eq!(pickup.taken, 6);
        assert_eq!(pickup.remainder, 14);
        for &id in &ids {
            let pc = engine.get_entity(id).unwrap().pc_data().unwrap();
            assert_eq!(
                engine
                    .pc_inventory_description(pc)
                    .unwrap()
                    .status
                    .get_ammo(Action::Eat),
                6
            );
            assert!(!pc.disabled_actions[0]);
        }
        assert_eq!(
            engine.mission_domain.campaign.characters[1]
                .status
                .life_points,
            23
        );
        // Stock remains available when its original owner dies.
        engine
            .world
            .entities
            .get_mut(ids[0])
            .unwrap()
            .pc_data_mut()
            .unwrap()
            .life_points = 0;
        engine.consume_ration_without_speech(&assets, ids[1], Action::Eat);
        let exported = engine.export_coop_campaign();
        assert_eq!(exported.characters.len(), 1);
        assert_eq!(exported.characters[0].status.get_ammo(Action::Eat), 5);
        let bytes = super::super::snapshot::encode_native_engine_inner(&engine);
        let mut restored = super::super::snapshot::decode_native_engine_inner(&bytes).unwrap();
        restored.consume_ration_without_speech(&assets, ids[2], Action::Eat);
        assert_eq!(
            restored.export_coop_campaign().characters[0]
                .status
                .get_ammo(Action::Eat),
            4
        );
    }

    #[test]
    fn campaign_fills_missing_slots_with_robin_and_keeps_existing_heroes() {
        let mut engine = party(2, 1);
        let robin = engine.world.pc_ids[1];
        engine
            .world
            .entities
            .get_mut(robin)
            .unwrap()
            .pc_data_mut()
            .unwrap()
            .robin = true;
        engine.control.sim_config.coop.campaign = true;
        engine.control.sim_config.coop.players = 4;
        engine.initialize_coop_party();
        assert_eq!(engine.world.pc_ids.len(), 4);
        for &id in &engine.world.pc_ids[2..] {
            let pc = engine.get_entity(id).unwrap().pc_data().unwrap();
            assert!(pc.robin);
            assert_eq!(pc.coop_origin, Some(robin));
        }
        engine.initialize_coop_party();
        assert_eq!(engine.world.pc_ids.len(), 4);
    }

    #[test]
    fn explicit_team_does_not_grow_to_player_count_and_preserves_slot_order() {
        let mut engine = party(2, 1);
        engine.control.sim_config.coop.team = [b'R', b'R', 0, 0, 0];
        engine.control.sim_config.coop.players = 5;
        engine.world.pc_ids.reverse();
        engine.initialize_coop_party();
        assert_eq!(engine.world.pc_ids.len(), 2);
        let ids = engine.world.pc_ids.clone();
        for index in 0..5 {
            assert_eq!(
                engine.players.seats[index].assigned_character,
                Some(ids[index % 2])
            );
        }
        assert_eq!(
            engine
                .get_entity(ids[1])
                .and_then(Entity::pc_data)
                .unwrap()
                .coop_origin,
            Some(ids[0])
        );
        assert_eq!(engine.coop_enemy_health_percent(), 125);
        if let Some(Entity::Pc(pc)) = engine.world.entities.get_mut(ids[0]) {
            pc.pc.life_points = 0;
        }
        assert!(engine.coop_copy_survives(ids[0]));
        if let Some(Entity::Pc(pc)) = engine.world.entities.get_mut(ids[1]) {
            pc.pc.life_points = 0;
        }
        assert!(!engine.coop_copy_survives(ids[0]));
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
    fn two_player_party_assigns_first_and_duplicate_to_different_seats() {
        let engine = party(1, 2);
        assert_eq!(engine.world.pc_ids.len(), 2);
        assert_ne!(
            engine.players.seats[0].selection,
            engine.players.seats[1].selection
        );
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
