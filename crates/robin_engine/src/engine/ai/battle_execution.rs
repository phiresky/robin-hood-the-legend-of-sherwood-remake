//! Live battle-list admission and primary selection.

use super::*;
use crate::ai::{AiEntityHandle, AiState, HumanHandle, Substate};
use crate::ai_enemy::increment_battle_target_multiplicity;
use crate::ai_enemy::{
    BattleDecisionInputs, battle_friend_is_nearer, battle_owner_target_square_distance,
};
use crate::element::Human;
use crate::sim_rng::SimulationContext;

impl EngineInner {
    pub(in crate::engine) fn execute_ai_attack_enemy(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        target: HumanHandle,
    ) {
        if matches!(self.expect_entity(owner, "attack owner"), Entity::Soldier(soldier) if soldier.soldier.rider)
            && self.execute_ai_maybe_make_rider_attack(sim, assets, owner)
        {
            return;
        }
        self.world
            .entities
            .expect_enemy_ai_mut(owner, format_args!("attack primary"))
            .base
            .primary_target = Some(AiEntityHandle::new(target));
        let target = self.expect_human_id_for_ai_handle(target, "attack target");
        debug_assert!(
            self.camps_are_hostile(
                self.expect_entity(owner, "attack owner").camp(),
                self.expect_entity(target, "attack target").camp()
            ),
            "attack target is friendly"
        );
        let position = self.live_ai_position(target);
        let ai = self
            .world
            .entities
            .expect_enemy_ai_mut(owner, format_args!("attack position"));
        ai.base.seek_position = position;
        ai.base.set_emoticon(crate::ai::EmoticonType::XMark);
        self.execute_ai_reconsider_enemy_approach(sim, assets, owner, false);
    }

    pub(in crate::engine) fn execute_ai_merry_man_forest_cassos(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) -> bool {
        let position = self.live_ai_position(owner);
        self.world
            .entities
            .expect_enemy_ai_mut(owner, format_args!("forest exit reset"))
            .base
            .my_door_index = None;
        let mut minimum = u32::MAX as f32;
        for index in 0..self.script_domains.interactables.doors.len() {
            let door = &self.script_domains.interactables.doors[index];
            if door.door_type != crate::gate::DoorType::Reinforcement {
                continue;
            }
            let distance = (position.x - door.point_in.x)
                .abs()
                .max((position.y - door.point_in.y).abs());
            if distance < minimum {
                minimum = distance;
                self.world
                    .entities
                    .expect_enemy_ai_mut(owner, format_args!("forest exit selection"))
                    .base
                    .my_door_index = crate::gate::DoorIndex::new(index as u32);
            }
        }
        if self
            .world
            .entities
            .expect_enemy_ai(owner, format_args!("forest exit result"))
            .base
            .my_door_index
            .is_none()
        {
            return false;
        }
        self.duty_set_state(
            sim,
            assets,
            owner,
            AiState::Fleeing,
            Substate::FleeingMerryManRunToLeaveMap,
        );
        let index = self
            .world
            .entities
            .expect_enemy_ai(owner, format_args!("forest exit after state"))
            .base
            .my_door_index
            .expect("forest exit retained after state callback");
        let door = &self.script_domains.interactables.doors[usize::from(index)];
        let destination = crate::ai::Position {
            x: door.point_in.x,
            y: door.point_in.y,
            level: door.layer_in,
            sector: crate::position_interface::SectorHandle::new(u16::from(door.sector_in)).map(
                |handle| handle.with_arena_index(door.sector_in_index.expect("forest exit sector")),
            ),
        };
        self.duty_go_to(sim, assets, owner, destination, crate::ai::GotoFlags::RUN);
        let ai = self
            .world
            .entities
            .expect_enemy_ai_mut(owner, format_args!("forest exit movement result"));
        ai.base.launch_timer(30, self.control.frame_counter);
        if ai.base.couldnt_reachpoint {
            ai.base.couldnt_reachpoint = false;
            return false;
        }
        true
    }

    pub(in crate::engine) fn reinitialize_live_ai_enemies(&mut self, owner: EntityId) {
        self.world
            .entities
            .expect_enemy_ai_mut(owner, format_args!("enemy list reset"))
            .list_them
            .clear();
        let count = self
            .expect_entity(owner, "enemy list owner")
            .ai_actor_data()
            .expect("AI actor")
            .detectable_lists[crate::element::DetectableType::Enemy as usize]
            .len();
        for index in 0..count {
            let detectable = &self
                .expect_entity(owner, "enemy list owner")
                .ai_actor_data()
                .expect("AI actor")
                .detectable_lists[crate::element::DetectableType::Enemy as usize][index];
            if !detectable.seen_now {
                continue;
            }
            let Some(target) = detectable.element else {
                continue;
            };
            if self.expect_entity(target, "seen enemy").is_dead() {
                continue;
            }
            self.world
                .entities
                .expect_enemy_ai_mut(owner, format_args!("enemy list admission"))
                .list_them
                .push(target.index());
        }
    }

    fn fill_live_near_fighters(&mut self, owner: EntityId, friendly: bool) -> bool {
        let camp = self.expect_entity(owner, "near fighters owner").camp();
        let origin = self
            .expect_entity(owner, "near fighters owner")
            .element_data()
            .position();
        let ai = self
            .world
            .entities
            .expect_enemy_ai_mut(owner, format_args!("near fighters reset"));
        if friendly {
            ai.base.list_us.clear();
            ai.base.list_us.push(owner.index());
        } else {
            ai.list_them.clear();
        }
        for target in self.world.fighter_registry_order() {
            let entity = self.expect_entity(target, "near fighter");
            if self.camps_are_allied(camp, entity.camp()) != friendly {
                continue;
            }
            let position = entity.element_data().position();
            let distance = (position.x - origin.x)
                .abs()
                .max(
                    ((position.y - origin.y) * crate::position_interface::INVERSE_ASPECT_RATIO)
                        .abs(),
                )
                .max((position.z - origin.z).abs());
            if distance >= 500.0
                || (friendly
                    && entity
                        .human_data()
                        .expect("fighter human")
                        .opponents
                        .is_empty())
                || target == owner
                || !battle_fighter_able(entity)
            {
                continue;
            }
            let ai = self
                .world
                .entities
                .expect_enemy_ai_mut(owner, format_args!("near fighter admission"));
            if friendly {
                ai.base.list_us.push(target.index());
            } else {
                ai.list_them.push(target.index());
            }
        }
        let ai = self
            .world
            .entities
            .expect_enemy_ai(owner, format_args!("near fighter result"));
        if friendly {
            !ai.base.list_us.is_empty()
        } else {
            !ai.list_them.is_empty()
        }
    }

    pub(in crate::engine) fn execute_ai_get_battle_overview(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        flags: u16,
    ) {
        if flags & 1 != 0 && self.fill_live_near_fighters(owner, false) {
            self.fill_live_near_fighters(owner, true);
            let primary = self
                .select_live_ai_primary_target(owner, crate::ai_enemy::PrimaryTargetFlags::empty());
            self.world
                .entities
                .expect_enemy_ai_mut(owner, format_args!("overview primary"))
                .base
                .primary_target = primary;
            if let Some(target) = primary {
                self.execute_ai_attack_enemy(sim, assets, owner, target.get());
                return;
            }
        }
        self.reinitialize_live_ai_enemies(owner);
        let ai = self
            .world
            .entities
            .expect_enemy_ai_mut(owner, format_args!("overview priority"));
        ai.current_task_priority = ai.minimal_task_priority;
        self.duty_set_state(
            sim,
            assets,
            owner,
            AiState::Attacking,
            Substate::AttackingOverviewLookLeft,
        );
        self.stop_ai_owner(sim, assets, owner);
        self.world
            .entities
            .expect_enemy_ai_mut(owner, format_args!("overview look"))
            .base
            .outbox
            .actor
            .look_sidewards = Some(crate::ai::LookDirection::Left);
        self.drain_direct_ai_owner_boundary(sim, owner, assets);
    }

    pub(in crate::engine) fn execute_ai_make_battle_predecisions(
        &self,
        sim: &SimulationContext,
        _assets: &LevelAssets,
        owner: EntityId,
    ) -> crate::ai::Decision {
        use crate::ai::Decision;
        use crate::profiles::ProfileRank;
        let ai = self
            .world
            .entities
            .expect_enemy_ai(owner, format_args!("battle predecision"));
        let entity = self.expect_entity(owner, "battle predecision owner");
        if (ai.is_archer()
            && (entity
                .ai_actor_data()
                .expect("battle AI actor")
                .number_of_arrows
                == 0
                || !entity
                    .human_data()
                    .expect("battle human")
                    .opponents
                    .is_empty()))
            || ai.base.current_state == AiState::Fleeing
        {
            return Decision::PredecisionDefensive;
        }
        let mut points = 0_u16;
        let mut officer = false;
        for &handle in &ai.base.list_us {
            let friend = self.expect_human_id_for_ai_handle(handle, "battle predecision ally");
            let value = match self.expect_entity(friend, "battle predecision ally") {
                Entity::Pc(_) => 100,
                Entity::Soldier(_) => {
                    let ally = self
                        .world
                        .entities
                        .expect_enemy_ai(friend, format_args!("battle predecision soldier"));
                    officer |= friend != owner && ally.get_rank() == ProfileRank::Officer;
                    100_u16.wrapping_add(ally.soldier_profile_pride)
                }
                _ => panic!("battle ally must be a PC or soldier"),
            };
            points = points.wrapping_add(value);
        }
        ai.battle_predecision_from_points(
            sim,
            points,
            ai.list_them.len() as u16,
            officer,
            entity.human_life_points(),
            entity.human_max_life_points(),
        )
    }

    pub(in crate::engine) fn select_live_ai_primary_target(
        &self,
        owner: EntityId,
        flags: crate::ai_enemy::PrimaryTargetFlags,
    ) -> Option<AiEntityHandle> {
        use crate::ai_enemy::PrimaryTargetFlags;
        let ai = self
            .world
            .entities
            .expect_enemy_ai(owner, format_args!("primary selection"));
        let origin = self
            .expect_entity(owner, "primary selection owner")
            .element_data()
            .position();
        let mut minimum = 65_432_u16;
        let mut selected = None;
        for &handle in &ai.list_them {
            let target = self.expect_human_id_for_ai_handle(handle, "primary selection target");
            let entity = self.expect_entity(target, "primary selection target");
            if !flags.contains(PrimaryTargetFlags::VIPS_ALLOWED)
                && !self.sleeping_enemy_attack_allowed(owner, target)
            {
                continue;
            }
            let position = entity.element_data().position();
            let dx = position.x - origin.x;
            let dy = (position.y - origin.y) * crate::position_interface::INVERSE_ASPECT_RATIO;
            let dz = position.z - origin.z;
            if dx.abs().max(dy.abs()).max(dz.abs()) > f32::from(minimum) {
                continue;
            }
            let mut distance = (dx * dx + dy * dy + dz * dz).sqrt() as u16;
            let multiplicity = self
                .ai
                .global
                .primary_target_multiplicity_scratch
                .get(&handle)
                .copied()
                .unwrap_or(0) as u16;
            if flags.contains(PrimaryTargetFlags::UNOCCUPIED_PREFERRED) {
                distance = distance.wrapping_add(100_u16.wrapping_mul(multiplicity));
            } else if flags.contains(PrimaryTargetFlags::UNOCCUPIED_STRONGLY_PREFERRED) {
                distance = distance.wrapping_add(10_000_u16.wrapping_mul(multiplicity));
            }
            if distance < minimum {
                minimum = distance;
                selected = Some(AiEntityHandle::new(handle));
            }
        }
        selected
    }

    pub(in crate::engine) fn execute_battle_decisions(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) {
        let (old_substate, inputs, unconscious) =
            self.prepare_live_battle_decisions(sim, assets, owner);
        if inputs.num_enemies_i_can_see == 0 {
            self.execute_live_battle_without_visible_enemies(sim, assets, owner, unconscious);
            return;
        }
        let (decision, cover) = self.choose_live_battle_decision(sim, assets, owner, inputs);
        if let Some(decision) = self.execute_live_battle_decision(
            sim,
            assets,
            owner,
            decision,
            old_substate,
            cover,
            inputs.alerting_soldier_near,
        ) {
            self.world
                .entities
                .expect_ai_controller_mut(owner, format_args!("battle decision log"))
                .register_log_line(crate::ai::LogLineType::BattleDecision, decision as u16);
        }
    }

    fn prepare_live_battle_decisions(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) -> (Substate, BattleDecisionInputs, Vec<HumanHandle>) {
        let ai = self
            .world
            .entities
            .expect_enemy_ai_mut(owner, format_args!("battle entry"));
        let old_substate = ai.base.current_substate;
        ai.base.outbox.actor.set_unfocus();
        self.drain_direct_ai_owner_boundary(sim, owner, assets);
        let camp = self.expect_entity(owner, "battle camp").camp();
        let mut visible = self
            .world
            .entities
            .expect_enemy_ai(owner, format_args!("battle personal enemies"))
            .list_them
            .len();
        for index in 0..visible {
            let target = self
                .world
                .entities
                .expect_enemy_ai(owner, format_args!("battle multiplicity reset"))
                .list_them[index];
            self.ai
                .global
                .primary_target_multiplicity_scratch
                .insert(target, 0);
        }
        let primary = self.select_nearest_battle_target(owner);
        let owner_world = self
            .expect_entity(owner, "battle owner position")
            .element_data()
            .position();
        let primary_distance = primary.map(|target| {
            battle_owner_target_square_distance(
                owner_world,
                self.expect_entity(target, "battle primary position")
                    .element_data()
                    .position(),
            )
        });
        let ai = self
            .world
            .entities
            .expect_enemy_ai_mut(owner, format_args!("battle primary selection"));
        ai.base.primary_target = primary.map(|target| AiEntityHandle::new(target.index()));
        ai.base.list_us.clear();
        ai.base.list_us.push(owner.index());
        let mut inputs = BattleDecisionInputs {
            friends_lower_company: 0,
            soldiers_lower_pride: false,
            simple_soldiers_near: false,
            alerting_soldier_near: false,
            min_square_enemy_distance: u32::MAX,
            num_enemies_i_can_see: visible,
            friends_nearer_to_enemy: 0,
        };

        // One registration-order scan performs admission and target injection.
        // Visibility precedes the soldier-state gate.
        for friend in self.world.fighter_registry_order() {
            if friend == owner {
                continue;
            }
            let entity = self.expect_entity(friend, "battle ally candidate");
            if !self.camps_are_allied(camp, entity.camp())
                || !battle_fighter_able(entity)
                || !self.patrol_member_visible(assets, owner, friend)
            {
                continue;
            }
            let ai = self
                .world
                .entities
                .expect_enemy_ai(owner, format_args!("battle ally owner"));
            let company = ai.company_number;
            let pride = ai.soldier_profile_pride;
            let reaction_time = ai.base.current_substate == Substate::AttackingReactiontime;
            if matches!(
                self.expect_entity(friend, "battle ally kind"),
                Entity::Pc(_)
            ) {
                self.world
                    .entities
                    .expect_enemy_ai_mut(owner, format_args!("battle PC ally"))
                    .base
                    .list_us
                    .push(friend.index());
                if company > 0 {
                    inputs.friends_lower_company = inputs.friends_lower_company.wrapping_add(1);
                }
                continue;
            }
            let ally = self
                .world
                .entities
                .expect_enemy_ai(friend, format_args!("battle soldier ally"));
            if !matches!(
                ally.base.current_state,
                AiState::Default | AiState::Wondering | AiState::Seeking | AiState::Attacking
            ) {
                continue;
            }
            let attacking = ally.base.current_state == AiState::Attacking;
            let swordfighting = ally.base.current_substate.is_any_swordfight();
            let target = ally.base.primary_target;
            inputs.alerting_soldier_near |=
                ally.base.current_substate == Substate::SeekingRunningToOfficer;
            if company > ally.company_number && (reaction_time || attacking) {
                inputs.friends_lower_company = inputs.friends_lower_company.wrapping_add(1);
            }
            inputs.soldiers_lower_pride |= pride > ally.soldier_profile_pride;
            inputs.simple_soldiers_near |= ally.get_rank() == crate::profiles::ProfileRank::Soldier;
            let ai = self
                .world
                .entities
                .expect_enemy_ai_mut(owner, format_args!("battle soldier admission"));
            ai.base.list_us.push(friend.index());
            if !attacking {
                continue;
            }
            let Some(target) = target else {
                continue;
            };
            let handle = target.get();
            if !ai.list_them.contains(&handle) {
                ai.list_them.push(handle);
            }
            if swordfighting {
                increment_battle_target_multiplicity(
                    &mut self.ai.global.primary_target_multiplicity_scratch,
                    handle,
                );
                inputs.friends_nearer_to_enemy = inputs.friends_nearer_to_enemy.wrapping_add(1);
            } else if let Some(primary) = primary
                && battle_friend_is_nearer(
                    self.live_ai_position(friend),
                    self.live_ai_position(primary),
                    primary_distance.expect("selected primary distance"),
                )
            {
                inputs.friends_nearer_to_enemy = inputs.friends_nearer_to_enemy.wrapping_add(1);
            }
        }

        let mut unconscious = Vec::new();
        let mut index = 0;
        loop {
            let Some(&handle) = self
                .world
                .entities
                .expect_enemy_ai(owner, format_args!("battle cleanup entry"))
                .list_them
                .get(index)
            else {
                break;
            };
            let target = self.expect_human_id_for_ai_handle(handle, "battle cleanup target");
            let entity = self.expect_entity(target, "battle cleanup human");
            let allied = self.camps_are_allied(camp, entity.camp());
            let able = battle_fighter_able(entity);
            if !allied && able {
                inputs.min_square_enemy_distance =
                    inputs
                        .min_square_enemy_distance
                        .min(battle_owner_target_square_distance(
                            owner_world,
                            entity.element_data().position(),
                        ));
                if !entity
                    .human_data()
                    .expect("battle target is human")
                    .opponents
                    .is_empty()
                    && self
                        .ai
                        .global
                        .primary_target_multiplicity_scratch
                        .get(&handle)
                        .copied()
                        .unwrap_or(0)
                        == 0
                {
                    self.ai
                        .global
                        .primary_target_multiplicity_scratch
                        .insert(handle, 1);
                }
                index += 1;
                continue;
            }
            if !allied {
                if index < visible {
                    visible -= 1;
                }
                if !entity.is_dead()
                    && entity.is_unconscious()
                    && entity
                        .human_data()
                        .expect("battle target is human")
                        .carrier
                        .is_none()
                {
                    unconscious.push(handle);
                }
            }
            self.world
                .entities
                .expect_enemy_ai_mut(owner, format_args!("battle cleanup removal"))
                .list_them
                .remove(index);
        }
        inputs.num_enemies_i_can_see = visible;
        (old_substate, inputs, unconscious)
    }
}

fn battle_fighter_able(entity: &Entity) -> bool {
    match entity {
        Entity::Pc(pc) => pc.is_able_to_fight(),
        Entity::Soldier(soldier) => soldier.is_able_to_fight(),
        Entity::Civilian(civilian) => civilian.is_able_to_fight(),
        _ => panic!("battle fighter is not human"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::Position;
    use crate::coordinates::WorldPoint3D;
    use crate::element::Posture;
    use crate::engine::test_support::{
        actors::{make_test_ai_soldier, make_test_pc},
        square_sector,
    };

    fn battle_fixture() -> (
        EngineInner,
        LevelAssets,
        EntityId,
        EntityId,
        EntityId,
        EntityId,
    ) {
        let mut engine = EngineInner::new();
        engine.world.fast_grid_mut().size_map(128, 128);
        engine.world.fast_grid_mut().allocate_layers(1);
        let index = engine.world.fast_grid_mut().add_sector(
            square_sector(1, 0, MapPoint::new(0.0, 0.0), MapPoint::new(2000.0, 2000.0)),
            0,
        );
        let sector = crate::position_interface::SectorHandle::new(1)
            .unwrap()
            .with_arena_index(crate::fast_find_grid::SectorIndex::new(index).unwrap());
        let owner = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
        let personal = engine.add_test_entity(make_test_pc(Posture::Upright));
        let ally = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
        let contributed = engine.add_test_entity(make_test_pc(Posture::Upright));
        for (id, x) in [
            (owner, 200.0),
            (personal, 400.0),
            (ally, 230.0),
            (contributed, 210.0),
        ] {
            let entity = engine.get_entity_mut(id).unwrap();
            entity.element_data_mut().set_sector(Some(sector));
            entity
                .element_data_mut()
                .set_position(WorldPoint3D::new(x, 200.0, 0.0));
            entity.actor_data_mut().unwrap().action_state = crate::element::ActionState::Waiting;
        }
        engine
            .get_entity_mut(owner)
            .unwrap()
            .ai_actor_data_mut()
            .unwrap()
            .view_radius = 500;
        let mut assets = LevelAssets::new();
        crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);
        let ai = engine
            .world
            .entities
            .expect_enemy_ai_mut(owner, format_args!("battle fixture owner"));
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = Substate::AttackingOverviewLookRight;
        ai.list_them = vec![personal.index()];
        ai.forced_next_battle_decision = crate::ai::Decision::Reserve;
        let ai = engine
            .world
            .entities
            .expect_enemy_ai_mut(ally, format_args!("battle fixture ally"));
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = Substate::AttackingSwordfight;
        ai.base.primary_target = Some(AiEntityHandle::new(contributed.index()));
        (engine, assets, owner, personal, ally, contributed)
    }

    #[test]
    fn live_predecision_reads_current_pride_with_uword_wrap_and_conditional_rng() {
        let (mut engine, assets, owner, personal, _, _) = battle_fixture();
        let ai = engine
            .world
            .entities
            .expect_enemy_ai_mut(owner, format_args!("predecision fixture"));
        ai.base.list_us = vec![owner.index()];
        ai.list_them = vec![personal.index(); 2];
        ai.is_archer_unit = false;
        ai.soldier_profile_courage = 0;
        for (pride, draws) in [(0, true), (1000, false), (u16::MAX, true)] {
            engine
                .world
                .entities
                .expect_enemy_ai_mut(owner, format_args!("live pride"))
                .soldier_profile_pride = pride;
            let sim = crate::sim_rng::SimulationContext::with_seed(19);
            let expected = crate::sim_rng::SimulationContext::with_seed(19);
            let decision = engine.execute_ai_make_battle_predecisions(&sim, &assets, owner);
            let expected_decision = if draws
                && crate::sim_rng::u16(&expected, crate::sim_rng::RngSite::BattleCourage, 0..100)
                    > 0
            {
                crate::ai::Decision::PredecisionDefensive
            } else {
                crate::ai::Decision::PredecisionOffensive
            };
            assert_eq!(decision, expected_decision, "pride {pride}");
            assert_eq!(
                sim.seed(),
                expected.seed(),
                "pride {pride}: exactly the conditional courage draw"
            );
        }
        engine
            .world
            .entities
            .expect_enemy_ai_mut(owner, format_args!("fleeing predecision"))
            .base
            .current_state = AiState::Fleeing;
        let sim = crate::sim_rng::SimulationContext::with_seed(19);
        let seed = sim.seed();
        assert_eq!(
            engine.execute_ai_make_battle_predecisions(&sim, &assets, owner),
            crate::ai::Decision::PredecisionDefensive
        );
        assert_eq!(sim.seed(), seed);
    }

    #[test]
    fn live_proud_decision_uses_entry_substate_for_speech() {
        use crate::ai::{Decision, LogLineType, Remark, StoredEnumWord};
        for (entry, previous, speaks) in [
            (
                Substate::AttackingReactiontime,
                Substate::DefaultOnPost,
                true,
            ),
            (
                Substate::AttackingTooProudToAttack,
                Substate::AttackingReactiontime,
                false,
            ),
        ] {
            let (mut engine, assets, owner, target) =
                super::super::battle_decision_observation_tests::fixture(false);
            engine
                .world
                .entities
                .get_mut(target)
                .unwrap()
                .element_data_mut()
                .set_position_map(MapPoint::new(250.0, 100.0));
            let ai = engine
                .world
                .entities
                .expect_enemy_ai_mut(owner, format_args!("proud fixture"));
            ai.base.current_substate = entry;
            ai.previous_substate = StoredEnumWord::new(previous);
            ai.is_vip = false;
            let result = engine.execute_live_battle_decision(
                &crate::sim_rng::test_context(),
                &assets,
                owner,
                Decision::TooProudToAttack,
                entry,
                0,
                false,
            );
            assert_eq!(result, Some(Decision::TooProudToAttack));
            let ai = engine
                .world
                .entities
                .expect_enemy_ai(owner, format_args!("proud result"));
            assert_eq!(
                ai.base.current_substate,
                Substate::AttackingTooProudToAttack
            );
            let remarks: Vec<_> = ai
                .base
                .ai_log
                .iter()
                .filter(|line| line.line_type == LogLineType::Speak)
                .map(|line| line.info)
                .collect();
            assert_eq!(
                remarks,
                if speaks {
                    vec![Remark::ProudDontFight as u16]
                } else {
                    vec![]
                }
            );
        }
    }

    #[test]
    fn live_rejected_shield_cover_clears_reciprocal_links_and_retains_candidate() {
        use crate::ai::Decision;
        for has_target in [false, true] {
            let (mut engine, assets, owner, target, bearer, _) = battle_fixture();
            engine.ai.standard_view_polygon_radius = 10;
            let ai = engine
                .world
                .entities
                .expect_enemy_ai_mut(owner, format_args!("covered archer"));
            ai.is_archer_unit = true;
            ai.base.primary_target = Some(AiEntityHandle::new(target.index()));
            let old_position = ai.base.seek_position;
            engine
                .world
                .entities
                .expect_enemy_ai_mut(bearer, format_args!("bearer target"))
                .base
                .primary_target = has_target.then_some(AiEntityHandle::new(target.index()));
            let (anchor, direction) = engine.live_shield_bearer_position(bearer);
            let [x, y] = crate::shadow_polygon::sector_to_direction(direction as i16);
            let distance = crate::ai_enemy::archer::DISTANCE_SHIELD_BEARER_ARCHER as f32;
            let expected = Position {
                x: anchor.x - x * distance,
                y: anchor.y - (y * crate::position_interface::ASPECT_RATIO) * distance,
                ..anchor
            };
            let outcome = engine.execute_ai_battle_cover(
                &crate::sim_rng::test_context(),
                &assets,
                owner,
                bearer.index(),
            );
            assert_eq!(outcome, std::ops::ControlFlow::Continue(Decision::Shoot));
            let ai = engine
                .world
                .entities
                .expect_enemy_ai(owner, format_args!("rejected cover"));
            assert_eq!(ai.shield_bearer_before_me, None);
            assert_eq!(
                ai.base.primary_target,
                has_target.then_some(AiEntityHandle::new(target.index()))
            );
            assert_eq!(
                ai.base.seek_position,
                if has_target { expected } else { old_position }
            );
            assert_eq!(
                engine
                    .world
                    .entities
                    .expect_enemy_ai(bearer, format_args!("unlinked bearer"))
                    .archer_behind_me,
                None
            );
        }
    }

    #[test]
    fn primary_selection_precedes_live_ally_injection_and_preserves_shared_claims() {
        let (mut engine, assets, owner, personal, ally, contributed) = battle_fixture();
        engine
            .ai
            .global
            .primary_target_multiplicity_scratch
            .insert(personal.index(), 9);
        engine
            .ai
            .global
            .primary_target_multiplicity_scratch
            .insert(contributed.index(), 4);
        let (_, inputs, _) =
            engine.prepare_live_battle_decisions(&crate::sim_rng::test_context(), &assets, owner);
        let ai = engine
            .world
            .entities
            .expect_enemy_ai(owner, format_args!("battle result"));
        assert_eq!(
            ai.base.primary_target,
            Some(AiEntityHandle::new(personal.index()))
        );
        assert_eq!(ai.base.list_us, vec![owner.index(), ally.index()]);
        assert_eq!(ai.list_them, vec![personal.index(), contributed.index()]);
        assert_eq!(inputs.num_enemies_i_can_see, 1);
        assert_eq!(inputs.friends_nearer_to_enemy, 1);
        assert_eq!(
            engine.ai.global.primary_target_multiplicity_scratch[&personal.index()],
            0
        );
        assert_eq!(
            engine.ai.global.primary_target_multiplicity_scratch[&contributed.index()],
            5
        );
        assert_eq!(inputs.min_square_enemy_distance, 100);
    }

    #[test]
    fn alerting_soldier_admission_is_captured_in_the_battle_scan() {
        let (mut engine, assets, owner, _, ally, _) = battle_fixture();
        let ai = engine
            .world
            .entities
            .expect_enemy_ai_mut(ally, format_args!("alerting ally"));
        ai.base.current_state = AiState::Seeking;
        ai.base.current_substate = Substate::SeekingRunningToOfficer;
        let (_, admitted, _) =
            engine.prepare_live_battle_decisions(&crate::sim_rng::test_context(), &assets, owner);
        assert!(admitted.alerting_soldier_near);
        engine
            .world
            .entities
            .get_mut(ally)
            .unwrap()
            .element_data_mut()
            .set_position_map(MapPoint::new(5000.0, 5000.0));
        let (_, out_of_view, _) =
            engine.prepare_live_battle_decisions(&crate::sim_rng::test_context(), &assets, owner);
        assert!(!out_of_view.alerting_soldier_near);
        assert!(
            admitted.alerting_soldier_near,
            "the enclosing decision retains its admission result"
        );
    }

    #[test]
    fn primary_selection_retains_list_ties_and_reads_live_multiplicity() {
        let (mut engine, _, owner, first, _, second) = battle_fixture();
        let position = engine
            .expect_entity(first, "first target")
            .element_data()
            .position();
        engine
            .get_entity_mut(second)
            .unwrap()
            .element_data_mut()
            .set_position(position);
        engine
            .world
            .entities
            .expect_enemy_ai_mut(owner, format_args!("selection targets"))
            .list_them = vec![first.index(), second.index()];
        use crate::ai_enemy::PrimaryTargetFlags;
        assert_eq!(
            engine.select_live_ai_primary_target(owner, PrimaryTargetFlags::empty()),
            Some(AiEntityHandle::new(first.index()))
        );
        engine
            .ai
            .global
            .primary_target_multiplicity_scratch
            .insert(first.index(), 1);
        assert_eq!(
            engine.select_live_ai_primary_target(owner, PrimaryTargetFlags::UNOCCUPIED_PREFERRED),
            Some(AiEntityHandle::new(second.index()))
        );
        engine
            .ai
            .global
            .primary_target_multiplicity_scratch
            .insert(second.index(), 1);
        assert_eq!(
            engine.select_live_ai_primary_target(owner, PrimaryTargetFlags::UNOCCUPIED_PREFERRED),
            Some(AiEntityHandle::new(first.index()))
        );
    }

    #[test]
    fn ally_admission_reads_current_positions_and_state() {
        let (mut engine, assets, owner, _, ally, contributed) = battle_fixture();
        engine
            .get_entity_mut(ally)
            .unwrap()
            .element_data_mut()
            .set_position(WorldPoint3D::new(1500.0, 200.0, 0.0));
        let (_, inputs, _) =
            engine.prepare_live_battle_decisions(&crate::sim_rng::test_context(), &assets, owner);
        assert_eq!(inputs.friends_nearer_to_enemy, 0);
        assert!(
            !engine
                .world
                .entities
                .expect_enemy_ai(owner, format_args!("distant ally"))
                .list_them
                .contains(&contributed.index())
        );
        engine
            .get_entity_mut(ally)
            .unwrap()
            .element_data_mut()
            .set_position(WorldPoint3D::new(230.0, 200.0, 0.0));
        engine
            .world
            .entities
            .expect_enemy_ai_mut(ally, format_args!("busy ally"))
            .base
            .current_state = AiState::Sleeping;
        engine.prepare_live_battle_decisions(&crate::sim_rng::test_context(), &assets, owner);
        assert_eq!(
            engine
                .world
                .entities
                .expect_enemy_ai(owner, format_args!("busy admission"))
                .base
                .list_us,
            vec![owner.index()]
        );
    }

    #[test]
    fn swordfighting_friend_counts_without_a_personal_primary() {
        let (mut engine, assets, owner, _, _, contributed) = battle_fixture();
        engine
            .world
            .entities
            .expect_enemy_ai_mut(owner, format_args!("empty personal list"))
            .list_them
            .clear();
        let (_, inputs, _) =
            engine.prepare_live_battle_decisions(&crate::sim_rng::test_context(), &assets, owner);
        let ai = engine
            .world
            .entities
            .expect_enemy_ai(owner, format_args!("friend-only result"));
        assert_eq!(ai.base.primary_target, None);
        assert_eq!(ai.list_them, vec![contributed.index()]);
        assert_eq!(inputs.num_enemies_i_can_see, 0);
        assert_eq!(inputs.friends_nearer_to_enemy, 1);
        assert_eq!(
            engine.ai.global.primary_target_multiplicity_scratch[&contributed.index()],
            1
        );
    }

    #[test]
    fn live_ally_scan_preserves_interleaved_pc_and_soldier_registration() {
        let (mut engine, assets, owner, first_pc, ally, second_pc) = battle_fixture();
        for id in [first_pc, second_pc] {
            let Entity::Pc(pc) = engine.get_entity_mut(id).unwrap() else {
                unreachable!()
            };
            pc.pc.cached_camp = Camp::Lacklandists;
        }
        engine
            .world
            .entities
            .expect_enemy_ai_mut(owner, format_args!("registry owner"))
            .list_them
            .clear();
        engine
            .world
            .entities
            .expect_enemy_ai_mut(ally, format_args!("registry ally"))
            .base
            .primary_target = None;
        engine.prepare_live_battle_decisions(&crate::sim_rng::test_context(), &assets, owner);
        assert_eq!(
            engine
                .world
                .entities
                .expect_enemy_ai(owner, format_args!("registry result"))
                .base
                .list_us,
            vec![
                owner.index(),
                first_pc.index(),
                ally.index(),
                second_pc.index()
            ]
        );
    }

    #[test]
    fn cleanup_removes_friends_without_consuming_personal_count() {
        let (mut engine, assets, owner, _, ally, _) = battle_fixture();
        engine.control.frame_counter = 700;
        engine
            .world
            .entities
            .expect_enemy_ai_mut(ally, format_args!("idle ally"))
            .base
            .primary_target = None;
        engine
            .world
            .entities
            .expect_enemy_ai_mut(owner, format_args!("stale friend"))
            .list_them = vec![ally.index()];
        engine.execute_battle_decisions(&crate::sim_rng::test_context(), &assets, owner);
        let ai = engine
            .world
            .entities
            .expect_enemy_ai(owner, format_args!("reserve result"));
        assert!(ai.list_them.is_empty());
        assert_eq!(ai.base.current_substate, Substate::AttackingReserve);
        assert!(ai.base.timer_is_running);
        assert_eq!(ai.base.when_does_timer_ring, 750);
    }

    #[test]
    fn cleanup_consumes_unable_personal_count_and_retains_sleeping_ids() {
        let (mut engine, assets, owner, personal, ally, _) = battle_fixture();
        engine
            .world
            .entities
            .expect_enemy_ai_mut(ally, format_args!("idle ally"))
            .base
            .primary_target = None;
        engine
            .get_entity_mut(personal)
            .unwrap()
            .human_data_mut()
            .unwrap()
            .unconscious = true;
        let (_, inputs, unconscious) =
            engine.prepare_live_battle_decisions(&crate::sim_rng::test_context(), &assets, owner);
        assert_eq!(inputs.num_enemies_i_can_see, 0);
        assert_eq!(unconscious, vec![personal.index()]);
        assert!(
            engine
                .world
                .entities
                .expect_enemy_ai(owner, format_args!("cleanup result"))
                .list_them
                .is_empty()
        );
    }

    #[test]
    fn battle_keeps_persistent_personal_targets() {
        let (mut engine, assets, owner, personal, ally, contributed) = battle_fixture();
        engine
            .world
            .entities
            .expect_enemy_ai_mut(ally, format_args!("idle ally"))
            .base
            .primary_target = None;
        engine
            .world
            .entities
            .expect_enemy_ai_mut(owner, format_args!("personal targets"))
            .list_them = vec![personal.index(), contributed.index()];
        let (_, inputs, _) =
            engine.prepare_live_battle_decisions(&crate::sim_rng::test_context(), &assets, owner);
        assert_eq!(inputs.num_enemies_i_can_see, 2);
        assert_eq!(
            engine
                .world
                .entities
                .expect_enemy_ai(owner, format_args!("persistent result"))
                .list_them,
            vec![personal.index(), contributed.index()]
        );
    }

    #[test]
    #[should_panic]
    fn absent_initial_battle_target_is_an_invariant_failure() {
        let (mut engine, assets, owner, _, _, _) = battle_fixture();
        engine
            .world
            .entities
            .expect_enemy_ai_mut(owner, format_args!("missing target"))
            .list_them = vec![u32::MAX];
        engine.prepare_live_battle_decisions(&crate::sim_rng::test_context(), &assets, owner);
    }
}
