//! Swordfight admission and fighter-list queries at their calling boundary.

use super::swordfight_candidates::LiveCombatFighters;
use super::*;
use crate::ai::{AiEntityHandle, AiState, GotoFlags, Stimulus, Substate};
use crate::ai_enemy::{AiMapVec, CombatFighterAccess, SwordfightLists};
use crate::sim_rng::SimulationContext;

#[cfg(test)]
mod nearest_opponent_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::test_support::{actors::make_test_ai_soldier, square_sector};

    fn combatants() -> (EngineInner, LevelAssets, EntityId, EntityId, EntityId) {
        let mut engine = EngineInner::new();
        engine.world.fast_grid_mut().size_map(16, 16);
        engine.world.fast_grid_mut().allocate_layers(1);
        let index = engine.world.fast_grid_mut().add_sector(
            square_sector(1, 0, MapPoint::new(0.0, 0.0), MapPoint::new(1000.0, 1000.0)),
            0,
        );
        let sector = crate::position_interface::SectorHandle::new(1)
            .unwrap()
            .with_arena_index(crate::fast_find_grid::SectorIndex::new(index).unwrap());
        let owner = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
        let friend = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
        let enemy = engine.add_test_entity(make_test_ai_soldier(Camp::Royalists));
        for id in [owner, friend, enemy] {
            let entity = engine.world.entities.get_mut(id).unwrap();
            entity.element_data_mut().active = true;
            entity.element_data_mut().set_sector(Some(sector));
            entity
                .element_data_mut()
                .set_position_map(MapPoint::new(200.0, 200.0));
            let npc = entity.npc_data_mut().unwrap();
            npc.life_points = 100;
            npc.view_radius = 1000;
        }
        let mut assets = LevelAssets::new();
        crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);
        (engine, assets, owner, friend, enemy)
    }

    #[test]
    fn reconsider_lists_read_live_opponents_and_preserve_registry_order() {
        let (mut engine, assets, owner, friend, enemy) = combatants();
        let human = engine
            .world
            .entities
            .get_mut(friend)
            .unwrap()
            .human_data_mut()
            .unwrap();
        human.opponents.add_principal(enemy, None);
        human.opponents.add_principal(owner, None);
        // Allied swordfighters remain candidates even when they cannot fight.
        human.unconscious = true;
        let lists = engine.rebuild_live_swordfight_lists(&assets, owner);
        assert_eq!(
            lists.nearest_friend_solo,
            Some(AiEntityHandle::new(friend.index()))
        );
        assert_eq!(lists.number_of_friends, 2);
        assert_eq!(
            engine
                .world
                .entities
                .expect_ai_controller(owner, format_args!("test allies"))
                .list_us,
            vec![owner.index(), friend.index()]
        );
        assert_eq!(
            engine
                .world
                .entities
                .expect_enemy_ai(owner, format_args!("test enemies"))
                .list_them,
            vec![enemy.index()]
        );

        engine
            .world
            .entities
            .get_mut(friend)
            .unwrap()
            .human_data_mut()
            .unwrap()
            .opponents
            .clear();
        let lists = engine.rebuild_live_swordfight_lists(&assets, owner);
        assert_eq!(lists.number_of_friends, 1);
        assert_eq!(lists.nearest_friend_solo, None);
    }

    #[test]
    fn observation_multiplicity_requires_attacking_state_and_reads_live_target() {
        let (mut engine, assets, owner, friend, enemy) = combatants();
        let ai = engine.enemy_ai_mut(friend, "test ally");
        ai.base.current_state = AiState::Default;
        ai.base.current_substate = Substate::AttackingSwordfight;
        ai.base.primary_target = Some(AiEntityHandle::new(enemy.index()));
        engine.rebuild_live_observation_lists(&assets, owner);
        assert_eq!(
            engine
                .ai
                .global
                .primary_target_multiplicity_scratch
                .get(&enemy.index()),
            Some(&0)
        );
        engine.enemy_ai_mut(friend, "test ally").base.current_state = AiState::Attacking;
        engine.rebuild_live_observation_lists(&assets, owner);
        assert_eq!(
            engine
                .ai
                .global
                .primary_target_multiplicity_scratch
                .get(&enemy.index()),
            Some(&1)
        );

        engine
            .world
            .entities
            .get_mut(friend)
            .unwrap()
            .element_data_mut()
            .set_position(crate::coordinates::WorldPoint3D::new(200.0, 200.0, 2000.0));
        engine.rebuild_live_observation_lists(&assets, owner);
        assert_eq!(
            engine
                .ai
                .global
                .primary_target_multiplicity_scratch
                .get(&enemy.index()),
            Some(&0)
        );
    }

    #[test]
    fn principal_refresh_precedes_facing_rejection_without_building_tactical_views() {
        let (mut engine, assets, owner, _, enemy) = combatants();
        let previous = engine.add_test_entity(make_test_ai_soldier(Camp::Royalists));
        engine
            .world
            .entities
            .get_mut(owner)
            .unwrap()
            .human_data_mut()
            .unwrap()
            .opponents
            .add_principal(enemy, None);
        let ai = engine.enemy_ai_mut(owner, "test owner");
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = Substate::AttackingSwordfight;
        ai.base.primary_target = Some(AiEntityHandle::new(previous.index()));
        engine
            .world
            .entities
            .get_mut(enemy)
            .unwrap()
            .element_data_mut()
            .set_position_map(MapPoint::new(300.0, 200.0));
        engine
            .world
            .entities
            .get_mut(owner)
            .unwrap()
            .element_data_mut()
            .set_direction_instantly(12);
        engine.execute_reconsider_swordfight(
            &crate::sim_rng::test_context(),
            &assets,
            owner,
            false,
        );
        assert_eq!(
            engine
                .world
                .entities
                .expect_ai_controller(owner, format_args!("test target"))
                .primary_target,
            Some(AiEntityHandle::new(enemy.index()))
        );
        assert!(engine.ai.think_call_stack.is_empty());
    }

    #[test]
    fn live_half_plane_visibility_rechecks_owner_activity_and_target_position() {
        let (mut engine, assets, owner, _, enemy) = combatants();
        engine
            .world
            .entities
            .get_mut(owner)
            .unwrap()
            .element_data_mut()
            .set_direction_instantly(4);
        engine
            .world
            .entities
            .get_mut(enemy)
            .unwrap()
            .element_data_mut()
            .set_position_map(MapPoint::new(350.0, 200.0));
        assert!(engine.live_ai_detects_180(&assets, owner, enemy));
        assert!(
            engine
                .ai
                .view_radius_cache
                .get(None, owner, engine.control.frame_counter)
                .is_some()
        );
        engine
            .world
            .entities
            .get_mut(enemy)
            .unwrap()
            .element_data_mut()
            .set_position_map(MapPoint::new(50.0, 200.0));
        assert!(!engine.live_ai_detects_180(&assets, owner, enemy));
        engine
            .world
            .entities
            .get_mut(enemy)
            .unwrap()
            .element_data_mut()
            .set_position_map(MapPoint::new(350.0, 200.0));
        engine
            .world
            .entities
            .get_mut(owner)
            .unwrap()
            .element_data_mut()
            .active = false;
        assert!(!engine.live_ai_detects_180(&assets, owner, enemy));
    }

    fn engage(engine: &mut EngineInner, owner: EntityId, target: EntityId) {
        engine
            .world
            .entities
            .get_mut(owner)
            .unwrap()
            .human_data_mut()
            .unwrap()
            .opponents
            .add_principal(target, None);
        engine
            .world
            .entities
            .get_mut(target)
            .unwrap()
            .human_data_mut()
            .unwrap()
            .opponents
            .add_principal(owner, None);
        let ai = engine.enemy_ai_mut(owner, "test swordfight");
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = Substate::AttackingSwordfight;
        ai.base.primary_target = Some(AiEntityHandle::new(target.index()));
    }

    fn sober_combat_context() -> SimulationContext {
        let seed = (0..1000)
            .find(|seed| {
                !crate::ai_enemy::drunk_combat_freezes(&SimulationContext::with_seed(*seed), 0)
            })
            .unwrap();
        SimulationContext::with_seed(seed)
    }

    #[test]
    fn lost_target_search_uses_the_refreshed_live_principal() {
        for company in [0, 100] {
            let (mut engine, mut assets, owner, _, previous) = combatants();
            let target = engine.add_test_entity(crate::engine::test_support::actors::make_test_pc(
                crate::element::Posture::Upright,
            ));
            let sector = engine
                .world
                .entities
                .get(previous)
                .unwrap()
                .element_data()
                .sector();
            let entity = engine.world.entities.get_mut(target).unwrap();
            entity.element_data_mut().set_sector(sector);
            entity
                .element_data_mut()
                .set_position_map(MapPoint::new(350.0, 200.0));
            entity.element_data_mut().active = false;
            crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);
            engage(&mut engine, owner, target);
            let ai = engine.enemy_ai_mut(owner, "test lost target");
            ai.company_number = company;
            ai.base.primary_target = Some(AiEntityHandle::new(previous.index()));
            let expected_center = engine.live_ai_position(target);
            engine.enter_ai_think_frame(owner);
            engine.execute_reconsider_swordfight(
                &SimulationContext::with_seed(0),
                &assets,
                owner,
                false,
            );
            let ai = engine.enemy_ai(owner, "test lost outcome");
            assert_eq!(ai.missed_pc, Some(AiEntityHandle::new(target.index())));
            assert!(ai.pc_missed);
            assert_eq!(ai.base.seek_position, expected_center);
            if company == 0 {
                assert_eq!(ai.seek_center, expected_center);
                assert_eq!(ai.base.current_state, AiState::Seeking);
            } else {
                assert_eq!(
                    ai.base.current_substate,
                    Substate::AttackingOverviewLookLeft
                );
            }
            assert!(
                engine
                    .orders
                    .sequence_manager
                    .sequences_iter()
                    .any(|sequence| {
                        sequence.elements.iter().any(|element| {
                            element.owner == Some(owner)
                                && element.command == crate::element::Command::QuitSwordfight
                        })
                    })
            );
            assert_eq!(engine.ai_think_depth(), 1);
        }
    }

    #[test]
    fn live_swordfight_step_in_retains_exact_target_sector() {
        let (mut engine, assets, owner, _, target) = combatants();
        engage(&mut engine, owner, target);
        let range = LiveCombatFighters {
            engine: &engine,
            assets: &assets,
            owner,
        }
        .sword_range_maximal(owner.index());
        engine
            .world
            .entities
            .get_mut(target)
            .unwrap()
            .element_data_mut()
            .set_position_map(MapPoint::new(200.0 + range as f32 + 80.0, 200.0));
        let target_position = engine.live_ai_position(target);
        engine.enter_ai_think_frame(owner);
        engine.reconsider_live_swordfight_tactics(
            &sober_combat_context(),
            &assets,
            owner,
            false,
            SwordfightLists {
                nearest_friend_solo: None,
                number_of_friends: 1,
                number_of_swordfighting_enemies: 1,
            },
        );
        assert_eq!(
            engine
                .world
                .entities
                .expect_ai_controller(owner, format_args!("test approach"))
                .last_goto_destination,
            target_position
        );
        assert!(target_position.sector.unwrap().arena_index().is_some());
    }
}

impl EngineInner {
    pub(in crate::engine) fn execute_reconsider_swordfight(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        enemy_weak: bool,
    ) {
        let frame = self.control.frame_counter;
        let ai = self.enemy_ai_mut(owner, "swordfight heartbeat");
        if ai.base.current_substate == Substate::AttackingSwordfight {
            ai.base.launch_timer(20, frame);
        }
        if self
            .orders
            .sequence_manager
            .element_is_about_to_be_launched_or_postponed_by_current(
                &self.world.entities,
                owner,
                crate::element::Command::EnterSwordfight,
            )
        {
            return;
        }
        if self
            .expect_entity(owner, "swordfight owner")
            .human_data()
            .expect("swordfight owner must be human")
            .opponents
            .is_empty()
        {
            self.execute_ai_callback(
                sim,
                assets,
                owner,
                &Stimulus::new(StimulusType::EventQuitSwordfight),
            );
            return;
        }

        // The existing AI target is checked before refreshing the principal.
        let old_primary = self
            .enemy_ai(owner, "swordfight target")
            .base
            .primary_target
            .expect("swordfight requires an AI target");
        let old_target = self.expect_human_id_for_ai_handle(old_primary.get(), "swordfight target");
        if self.camps_are_allied(
            self.expect_entity(owner, "swordfight owner camp").camp(),
            self.expect_entity(old_target, "swordfight target camp")
                .camp(),
        ) {
            self.execute_ai_end_swordfight(sim, assets, owner);

            self.clear_live_combat_neighbours(owner);
            self.duty_set_state(
                sim,
                assets,
                owner,
                AiState::Attacking,
                Substate::AttackingQuittingSwordfight,
            );
            let frame = self.control.frame_counter;
            self.ai_mut(owner, "quit swordfight timer")
                .launch_timer(3, frame);
            return;
        }
        let primary = *self
            .expect_entity(owner, "swordfight principal owner")
            .human_data()
            .expect("swordfight owner must be human")
            .opponents
            .first()
            .expect("swordfight requires a principal opponent");
        self.ai_mut(owner, "swordfight principal").primary_target =
            Some(AiEntityHandle::new(primary.index()));
        if !self.patrol_member_visible(assets, owner, primary) {
            self.finish_live_swordfight_target_loss(sim, assets, owner, primary);
            return;
        }
        let me = self
            .expect_entity(owner, "swordfight facing owner")
            .element_data();
        let target = self
            .expect_entity(primary, "swordfight facing target")
            .element_data();
        let position = |element: &crate::element::ElementData| crate::ai::Position {
            x: element.position_map().x,
            y: element.position_map().y,
            sector: element.sector(),
            level: element.layer(),
        };
        if !crate::ai_enemy::is_facing_swordfight_target(
            &position(me),
            me.position().z,
            me.direction() as u16,
            &position(target),
            target.position().z,
        ) {
            return;
        }
        let lists = self.rebuild_live_swordfight_lists(assets, owner);
        self.reconsider_live_swordfight_tactics(sim, assets, owner, enemy_weak, lists);
    }

    fn finish_live_swordfight_target_loss(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        target: EntityId,
    ) {
        let entity = self.expect_entity(target, "lost swordfight forecast target");
        let input = extract_exact_forecast_input(
            self,
            entity,
            selected_actor_is_passing_door(
                &self.world.entities,
                &self.orders.sequence_manager,
                target,
            ),
        )
        .expect("lost swordfight forecast requires an actor");
        let forecast = crate::ai::prepare_forecast_destination_for_ia(
            &input,
            &self.script_domains.interactables.doors,
            &self.world.fast_grid.level.sectors,
            &self.world.fast_grid.level.sector_number_map,
        )
        .resolve_retaining_direction(
            sim,
            self.enemy_ai(owner, "lost direction")
                .pc_gone_away_in_this_direction,
        );
        let ai = self.enemy_ai_mut(owner, "lost swordfight target");
        ai.base.seek_position = forecast.position;
        ai.pc_gone_away_in_this_direction = forecast.direction;
        ai.missed_pc = ai.base.primary_target;
        ai.pc_missed = true;
        self.execute_ai_end_swordfight(sim, assets, owner);

        self.finish_live_lost_enemy_pursuit(sim, assets, owner);
    }

    fn nearest_live_opponent(&self, maurice: EntityId, rene: EntityId) -> Option<EntityId> {
        let position = self.live_ai_position(rene);
        let mut nearest = None;
        let mut distance = u16::MAX;
        for &opponent in self
            .expect_entity(maurice, "opponent scan")
            .human_data()
            .unwrap()
            .opponents
            .iter()
        {
            let candidate = (position.map_point() - self.live_ai_position(opponent).map_point())
                .max_norm() as u16;
            if candidate < distance {
                distance = candidate;
                nearest = Some(opponent);
            }
        }
        nearest
    }

    fn reconsider_live_swordfight_tactics(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        enemy_weak: bool,
        lists: SwordfightLists,
    ) {
        let forest_archer = self.world.weather.is_forest_level
            && self.is_player_aligned_camp(self.expect_entity(owner, "combat forest camp").camp())
            && !self
                .expect_entity(owner, "combat forest rider")
                .soldier_data()
                .is_some_and(|soldier| soldier.rider)
            && self.enemy_ai(owner, "combat forest archer").is_archer();
        if forest_archer && self.execute_ai_merry_man_forest_cassos(sim, assets, owner) {
            return;
        }
        let fighters = LiveCombatFighters {
            engine: self,
            assets,
            owner,
        };
        let mut primary = fighters
            .principal(owner.index())
            .expect("combat principal after forest gate");
        let primary_id = fighters.id(primary.get());
        if self
            .expect_entity(primary_id, "combat rebalance opponent")
            .human_data()
            .unwrap()
            .opponents
            .len()
            > 1
            && let Some(friend) = lists.nearest_friend_solo
        {
            let friend = fighters.id(friend.get());
            let nearest = self
                .nearest_live_opponent(friend, owner)
                .expect("solo fighter requires an opponent");
            if self.nearest_live_opponent(primary_id, nearest) == Some(owner) {
                self.execute_ai_rebalance_swordfight(sim, assets, owner, nearest);

                return;
            }
        }
        primary = LiveCombatFighters {
            engine: self,
            assets,
            owner,
        }
        .principal(owner.index())
        .expect("combat principal refresh");
        self.ai_mut(owner, "combat refreshed principal")
            .primary_target = Some(primary);
        if self.ai.global.stupid_soldiers_cheat {
            return;
        }
        let alcohol = self.ai(owner, "combat intoxication").blood_alcohol;
        if crate::ai_enemy::drunk_combat_freezes(sim, alcohol) {
            return;
        }
        let fighters = LiveCombatFighters {
            engine: self,
            assets,
            owner,
        };
        let range = fighters.sword_range_maximal(owner.index());
        let owner_world = self
            .expect_entity(owner, "combat charge owner")
            .element_data()
            .position();
        let target_world = self
            .expect_entity(fighters.id(primary.get()), "combat charge target")
            .element_data()
            .position();
        let dx = target_world.x - owner_world.x;
        let dy = (target_world.y - owner_world.y) * crate::position_interface::INVERSE_ASPECT_RATIO;
        let dz = target_world.z - owner_world.z;
        if enemy_weak
            && fighters.rank(owner.index()) == crate::profiles::ProfileRank::Soldier
            && (dx * dx + dy * dy + dz * dz).sqrt() > range as f32
            && fighters.fighting_ability(owner.index())
                >= crate::ai_enemy::combat::MIN_CAPACITY_CHARGE_WEAK_ENEMY
        {
            self.duty_set_state(
                sim,
                assets,
                owner,
                AiState::Attacking,
                Substate::AttackingMovingAroundOldEnemy,
            );
            let fighters = LiveCombatFighters {
                engine: self,
                assets,
                owner,
            };
            let target = fighters.position(
                fighters
                    .principal(owner.index())
                    .expect("charge principal")
                    .get(),
            );
            self.duty_go_near(
                sim,
                assets,
                owner,
                target,
                LiveCombatFighters {
                    engine: self,
                    assets,
                    owner,
                }
                .range(owner.index(), crate::weapons::WeaponDistance::Default)
                    as i32,
                GotoFlags::RUN | GotoFlags::SWORD,
            );
            return;
        }
        let trainer = self.enemy_ai(owner, "combat trainer").combat_trainer;
        if !trainer
            && (lists.number_of_friends != 1 || lists.number_of_swordfighting_enemies != 1)
            && crate::sim_rng::u32(sim, crate::sim_rng::RngSite::CombatReposition, 0..3) == 0
        {
            let candidate = self.propose_live_combat_position(assets, owner);

            let ai = self.enemy_ai_mut(owner, "combat selected position");
            ai.base.seek_position = candidate.attacker_position;
            ai.my_line_jump = candidate.line_jump;
            if candidate.change_adversary {
                ai.base.primary_target = candidate.target;
                if candidate.change_position {
                    self.duty_set_state(
                        sim,
                        assets,
                        owner,
                        AiState::Attacking,
                        Substate::AttackingApproachingNewEnemy,
                    );
                    if candidate.line_jump.is_some() {
                        self.duty_go_near(
                            sim,
                            assets,
                            owner,
                            candidate.attacker_position,
                            30,
                            GotoFlags::SWORD,
                        );
                    } else {
                        self.duty_go_to(
                            sim,
                            assets,
                            owner,
                            candidate.attacker_position,
                            GotoFlags::SWORD,
                        );
                    }
                } else {
                    self.duty_set_state(
                        sim,
                        assets,
                        owner,
                        AiState::Attacking,
                        Substate::AttackingSwordfight,
                    );
                    if let Some(target) = candidate.target {
                        let target = self
                            .expect_human_id_for_ai_handle(target.get(), "combat new principal");
                        self.set_as_new_principal_opponent(sim, assets, owner, target);
                    }

                    let frame = self.control.frame_counter;
                    self.ai_mut(owner, "combat new principal timer")
                        .launch_timer(20, frame);
                }
                return;
            }
            if candidate.change_position {
                self.duty_set_state(
                    sim,
                    assets,
                    owner,
                    AiState::Attacking,
                    Substate::AttackingMovingAroundOldEnemy,
                );
                self.duty_go_to(
                    sim,
                    assets,
                    owner,
                    candidate.attacker_position,
                    GotoFlags::SWORD,
                );
                return;
            }
        }
        // Candidate scoring can replace the principal and run reciprocal callbacks.
        let fighters = LiveCombatFighters {
            engine: self,
            assets,
            owner,
        };
        let primary = self
            .ai(owner, "combat strike target")
            .primary_target
            .expect("combat strike target");
        let primary_id = fighters.id(primary.get());
        let me = fighters.position(owner.index());
        let target = fighters.position(primary.get());
        let distance = (target.map_point() - me.map_point()).square_norm().sqrt() as u16;
        let ai = self.enemy_ai(owner, "combat step-in");
        if distance > fighters.sword_range_maximal(owner.index())
            && distance > fighters.sword_range_maximal(primary.get())
            && ai.my_line_jump.is_none()
            && !ai.combat_trainer
        {
            self.duty_set_state(
                sim,
                assets,
                owner,
                AiState::Attacking,
                Substate::AttackingMovingAroundOldEnemy,
            );
            let fighters = LiveCombatFighters {
                engine: self,
                assets,
                owner,
            };
            let distance = fighters.range(owner.index(), crate::weapons::WeaponDistance::Default);
            let position = fighters.position(
                fighters
                    .principal(owner.index())
                    .expect("step-in principal")
                    .get(),
            );
            self.duty_go_near(
                sim,
                assets,
                owner,
                position,
                distance as i32,
                GotoFlags::SWORD,
            );
            return;
        }
        if ai.combat_trainer
            && (ai.base.initial_position.map_point() - me.map_point()).max_norm() > 20.0
        {
            let post = ai.base.initial_position;
            self.duty_set_state(
                sim,
                assets,
                owner,
                AiState::Attacking,
                Substate::AttackingMovingAroundOldEnemy,
            );
            self.duty_go_to(sim, assets, owner, post, GotoFlags::SWORD);
            return;
        }
        if !self.actor_is_in_sword_recovery(primary_id)
            && self
                .expect_entity(primary_id, "strike target action")
                .actor_data()
                .unwrap()
                .action_state
                .is_sword()
        {
            self.execute_ai_sword_strike_proposal(sim, assets, owner);
        }
    }

    fn fighter_max_norm_distance(&self, owner: EntityId, target: EntityId) -> f32 {
        let me = self
            .expect_entity(owner, "fighter distance owner")
            .element_data()
            .position();
        let other = self
            .expect_entity(target, "fighter distance target")
            .element_data()
            .position();
        (other.x - me.x)
            .abs()
            .max(((other.y - me.y) * crate::position_interface::INVERSE_ASPECT_RATIO).abs())
            .max((other.z - me.z).abs())
    }

    fn fighter_can_fight(&self, fighter: EntityId) -> bool {
        match self.expect_entity(fighter, "combat fighter") {
            Entity::Pc(pc) => pc.is_able_to_fight(),
            Entity::Soldier(soldier) => soldier.is_able_to_fight(),
            _ => panic!("combat registry contains a non-fighter"),
        }
    }

    fn rebuild_live_swordfight_lists(
        &mut self,
        assets: &LevelAssets,
        owner: EntityId,
    ) -> SwordfightLists {
        let camp = self.expect_entity(owner, "swordfight list owner").camp();
        let ai = self.ai_mut(owner, "swordfight allies");
        ai.list_us.clear();
        ai.list_us.push(owner.index());
        let mut nearest_friend_solo = None;
        let mut nearest_distance = u16::MAX;
        let friend_count = self.world.fighter_registry_ids.len();
        for index in 0..friend_count {
            let friend = self.world.fighter_registry_ids[index];
            let entity = self.expect_entity(friend, "swordfight ally candidate");
            if friend == owner || !self.camps_are_allied(camp, entity.camp()) {
                continue;
            }
            let opponents = entity
                .human_data()
                .expect("fighter must be human")
                .opponents
                .len();
            if opponents == 0 {
                continue;
            }
            let distance = self.fighter_max_norm_distance(owner, friend) as u16;
            if distance >= crate::parameters_ai::MAX_SWORDFIGHT_CONSIDERATION_RADIUS as u16 {
                continue;
            }
            self.ai_mut(owner, "swordfight ally insertion")
                .list_us
                .push(friend.index());
            if opponents > 1 && distance < nearest_distance {
                nearest_friend_solo = Some(AiEntityHandle::new(friend.index()));
                nearest_distance = distance;
            }
        }
        self.enemy_ai_mut(owner, "swordfight enemy reset")
            .list_them
            .clear();
        let mut number_of_swordfighting_enemies = 0u16;
        let target_count = self.world.fighter_registry_ids.len();
        for index in 0..target_count {
            let target = self.world.fighter_registry_ids[index];
            let entity = self.expect_entity(target, "swordfight enemy candidate");
            if !self.camps_are_hostile(camp, entity.camp())
                || !self.fighter_can_fight(target)
                || !self.patrol_member_visible(assets, owner, target)
            {
                continue;
            }
            let swordfighting = !entity
                .human_data()
                .expect("fighter must be human")
                .opponents
                .is_empty();
            self.enemy_ai_mut(owner, "swordfight enemy insertion")
                .list_them
                .push(target.index());
            if swordfighting {
                number_of_swordfighting_enemies = number_of_swordfighting_enemies.wrapping_add(1);
            }
        }
        SwordfightLists {
            nearest_friend_solo,
            number_of_swordfighting_enemies,
            number_of_friends: self.ai(owner, "swordfight ally count").list_us.len() as u16,
        }
    }

    pub(in crate::engine) fn execute_reconsider_swordfight_observation(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) {
        let protected = self.refresh_ai_arrow_protection(sim, assets, owner, false);

        if protected {
            return;
        }
        self.rebuild_live_observation_lists(assets, owner);
        let primary = self.select_live_ai_primary_target(
            owner,
            crate::ai_enemy::PrimaryTargetFlags::UNOCCUPIED_STRONGLY_PREFERRED,
        );
        self.ai_mut(owner, "observation primary").primary_target = primary;
        self.focus_live_combat_target(owner);
        let Some(primary) = primary else {
            self.execute_ai_get_battle_overview(sim, assets, owner, 0);
            return;
        };
        if self.enemy_ai(owner, "observation trainer").combat_trainer {
            self.stand_observing_combat(sim, assets, owner);
            return;
        }
        if self.execute_ai_make_battle_predecisions(sim, assets, owner)
            == crate::ai::Decision::PredecisionDefensive
        {
            let target =
                self.expect_human_id_for_ai_handle(primary.get(), "defensive observation target");
            let enemy_position = self.live_ai_position(target);
            self.ai_mut(owner, "defensive observation position")
                .seek_position = enemy_position;
            let goal = crate::ai_enemy::propose_good_step_back_goal(
                self.live_ai_position(owner),
                self.expect_entity(owner, "defensive observer move box")
                    .position_iface()
                    .get_move_box(),
                enemy_position,
                crate::parameters_ai::ARCHER_GOOD_DISTANCE,
                crate::parameters_ai::ARCHER_MIN_DISTANCE,
                Some(&self.world.fast_grid),
                crate::position_interface::ASPECT_RATIO,
            );
            if let Some(goal) = goal {
                self.duty_set_state(
                    sim,
                    assets,
                    owner,
                    AiState::Fleeing,
                    Substate::FleeingRetireFromCombat,
                );
                self.duty_go_to(sim, assets, owner, goal, GotoFlags::RUN);
            } else {
                self.execute_ai_panic(
                    sim,
                    assets,
                    owner,
                    Some(enemy_position),
                    crate::parameters_ai::AI_STANDARD_PANIC_RUNS as u8,
                    crate::ai::AlertLevel::Red,
                );
            }
        }
        self.execute_observation_attack_or_step(sim, assets, owner);
    }

    fn focus_live_combat_target(&mut self, owner: EntityId) {
        let ai = self.ai_mut(owner, "combat focus");
        if ai.primary_target.is_some() {
            let target = ai.primary_target;
            self.execute_ai_focus(owner, target);
        } else {
            self.execute_ai_unfocus(owner);
        }
    }

    fn stand_observing_combat(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) {
        let target = self
            .ai(owner, "stationary observer target")
            .primary_target
            .expect("stationary observer requires a target");
        let target = self.expect_human_id_for_ai_handle(target.get(), "stationary observer target");
        let direction = (self.live_ai_position(target).map_point()
            - self.live_ai_position(owner).map_point())
        .sector_with_aspect(crate::position_interface::ASPECT_RATIO);
        self.execute_ai_direction_goal(owner, direction);

        self.focus_live_combat_target(owner);
        self.stop_ai_owner(sim, assets, owner);
        self.duty_set_state(
            sim,
            assets,
            owner,
            AiState::Attacking,
            Substate::AttackingObserve,
        );
        let frame = self.control.frame_counter;
        self.ai_mut(owner, "observer timer").launch_timer(20, frame);
    }

    pub(in crate::engine) fn execute_observation_attack_or_step(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) {
        let primary = self
            .ai(owner, "observation attack target")
            .primary_target
            .expect("observation attack requires a target");
        let fighters = LiveCombatFighters {
            engine: self,
            assets,
            owner,
        };
        let target = fighters.id(primary.get());
        let me = self.live_ai_position(owner);
        let target_position = self.live_ai_position(target);
        let distance = (me.map_point() - target_position.map_point())
            .iso_norm(crate::position_interface::ASPECT_RATIO) as u16;
        let back_to_me =
            crate::coordinates::MapVec::from_sector_iso(fighters.direction(primary.get()))
                .dot(target_position.map_point() - me.map_point())
                > 0.0;
        let principal = fighters.principal(primary.get());
        let opportunity = back_to_me
            || principal.is_none()
            || principal.is_some_and(|principal| {
                self.expect_entity(fighters.id(principal.get()), "observed principal")
                    .human_data()
                    .unwrap()
                    .opponents
                    .len()
                    >= 3
            })
            || distance < 30;
        if opportunity {
            let occupied = self
                .ai(owner, "observation competitors")
                .list_us
                .iter()
                .copied()
                .filter(|&handle| handle != owner.index())
                .any(|handle| {
                    let entity = self.expect_entity(fighters.id(handle), "observation competitor");
                    matches!(entity, Entity::Soldier(_))
                        && entity.enemy_ai().is_some_and(|ai| {
                            ai.base.primary_target == Some(primary)
                                && matches!(
                                    ai.base.current_substate,
                                    Substate::AttackingWalkingToEnemy
                                        | Substate::AttackingRunningToEnemy
                                        | Substate::AttackingChargingEnemy
                                )
                        })
                });
            if !occupied {
                self.execute_ai_attack_enemy(sim, assets, owner, target.index());
                return;
            }
        }
        self.step_while_observing_combat(sim, assets, owner, me, target_position);
    }

    fn observer_prefers_left_step(&self, owner: EntityId) -> bool {
        let direction = self
            .expect_entity(owner, "observation side direction")
            .element_data()
            .direction() as u16;
        let right = crate::coordinates::MapVec::from_sector_iso(direction).normal_iso(false);
        let me = self.live_ai_position(owner);
        let mut score = 0i16;
        for &handle in &self.ai(owner, "observation side allies").list_us {
            if handle == owner.index() {
                continue;
            }
            let id = self.expect_human_id_for_ai_handle(handle, "observation side ally");
            let Entity::Soldier(soldier) = self.expect_entity(id, "observation side ally") else {
                continue;
            };
            let ai = soldier
                .npc
                .ai_brain
                .enemy()
                .expect("observation side ally brain");
            if !matches!(
                ai.base.current_substate,
                Substate::AttackingObserve
                    | Substate::AttackingObserveAndMove
                    | Substate::AttackingProtectingWithShield
                    | Substate::AttackingAdvancingWithShield
                    | Substate::AttackingBowRunningBehindShieldBearer
                    | Substate::AttackingBowCorrectingPosition
                    | Substate::AttackingPhalanx
                    | Substate::AttackingRunningToPhalanx
                    | Substate::AttackingBowShooting
                    | Substate::AttackingBowLoading
                    | Substate::AttackingBowAiming
                    | Substate::AttackingBowObserving
                    | Substate::AttackingBowObservingLoading
            ) {
                continue;
            }
            let scalar = right.dot(self.live_ai_position(id).map_point() - me.map_point()) as i16;
            if (1..=200).contains(&scalar) {
                score = score.wrapping_add(200 - scalar);
            } else if (-200..=0).contains(&scalar) {
                score = score.wrapping_sub(200 + scalar);
            }
        }
        score > 0
    }

    fn step_while_observing_combat(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        me: crate::ai::Position,
        mut reference: crate::ai::Position,
    ) {
        let ai = self.enemy_ai(owner, "observation spacing");
        let ideal = crate::ai::AiController::value_between(
            crate::parameters_ai::OBSERVE_SWORDFIGHT_MAX_DISTANCE,
            crate::parameters_ai::OBSERVE_SWORDFIGHT_MIN_DISTANCE,
            ai.get_courage(&assets.profile_manager) as u8,
        );
        let aspect = crate::position_interface::ASPECT_RATIO;
        let mut distance = (me.map_point() - reference.map_point()).iso_norm(aspect) as u16;
        let fighters = LiveCombatFighters {
            engine: self,
            assets,
            owner,
        };
        if let Some(friend) = fighters.principal(ai.base.primary_target.unwrap().get())
            && friend.get() != owner.index()
        {
            let friend = fighters.position(friend.get());
            let friend_distance = (me.map_point() - friend.map_point()).iso_norm(aspect) as u16;
            if friend_distance < distance {
                distance = friend_distance;
                reference = friend;
            }
        }
        let move_box = self
            .expect_entity(owner, "observer move box")
            .position_iface()
            .get_move_box();
        let straight = |from: crate::ai::Position, to: crate::ai::Position| {
            self.world.fast_grid.is_straight_movement_authorized(
                from.map_point(),
                to.map_point(),
                me.level,
                move_box,
            )
        };
        let mut destination = None;
        if i32::from(distance) < i32::from(ideal) - 50
            || i32::from(distance) > i32::from(ideal) + 50
        {
            let delta = if distance < ideal {
                me.map_point() - reference.map_point()
            } else {
                reference.map_point() - me.map_point()
            };
            let mut step = delta.iso_normalize(aspect);
            let scale = f32::from(distance.abs_diff(ideal));
            step.x *= scale;
            step.y *= scale;
            let candidate = crate::ai::Position {
                x: me.x + step.x,
                y: me.y + step.y,
                ..me
            };
            if straight(me, candidate) {
                destination = Some(candidate);
            }
        }
        if destination.is_none()
            && crate::sim_rng::u32(sim, crate::sim_rng::RngSite::CombatObserveSideStep, 0..2) == 0
        {
            let preferred = self.observer_prefers_left_step(owner);
            for side in [preferred, !preferred] {
                let mut step = (reference.map_point() - me.map_point())
                    .normal_iso(side)
                    .iso_normalize(aspect);
                step.x *= crate::parameters_ai::OBSERVE_SWORDFIGHT_SIDE_STEP;
                step.y *= crate::parameters_ai::OBSERVE_SWORDFIGHT_SIDE_STEP;
                let candidate = crate::ai::Position {
                    x: me.x + step.x,
                    y: me.y + step.y,
                    ..me
                };
                if straight(me, candidate)
                    && (!straight(me, reference) || straight(candidate, reference))
                {
                    destination = Some(candidate);
                    break;
                }
            }
        }
        if let Some(destination) = destination {
            self.duty_set_state(
                sim,
                assets,
                owner,
                AiState::Attacking,
                Substate::AttackingObserveAndMove,
            );
            self.focus_live_combat_target(owner);
            self.duty_go_to(sim, assets, owner, destination, GotoFlags::SWORD);
        } else {
            self.stand_observing_combat(sim, assets, owner);
        }
    }

    fn rebuild_live_observation_lists(&mut self, assets: &LevelAssets, owner: EntityId) {
        let camp = self.expect_entity(owner, "observation list owner").camp();
        let radius = crate::parameters_ai::MAX_SWORDFIGHT_CONSIDERATION_RADIUS as f32;
        self.enemy_ai_mut(owner, "observation enemies")
            .list_them
            .clear();
        let target_count = self.world.fighter_registry_ids.len();
        for index in 0..target_count {
            let target = self.world.fighter_registry_ids[index];
            if !self.camps_are_hostile(
                camp,
                self.expect_entity(target, "observation target camp").camp(),
            ) || !self.fighter_can_fight(target)
                || !(self.fighter_max_norm_distance(owner, target) < radius)
                || !self.live_ai_detects_180(assets, owner, target)
            {
                continue;
            }
            self.enemy_ai_mut(owner, "observation enemy insertion")
                .list_them
                .push(target.index());
            self.ai
                .global
                .primary_target_multiplicity_scratch
                .insert(target.index(), 0);
        }
        let ai = self.ai_mut(owner, "observation allies");
        ai.list_us.clear();
        ai.list_us.push(owner.index());
        let friend_count = self.world.fighter_registry_ids.len();
        for index in 0..friend_count {
            let friend = self.world.fighter_registry_ids[index];
            let entity = self.expect_entity(friend, "observation ally candidate");
            if !self.camps_are_allied(camp, entity.camp())
                || friend == owner
                || !self.fighter_can_fight(friend)
                || f32::from(self.fighter_max_norm_distance(owner, friend) as u16) >= radius
            {
                continue;
            }
            let target = entity.enemy_ai().and_then(|ai| {
                (ai.base.current_state == AiState::Attacking
                    && ai.base.current_substate.is_any_swordfight())
                .then_some(ai.base.primary_target)
                .flatten()
            });
            self.ai_mut(owner, "observation ally insertion")
                .list_us
                .push(friend.index());
            if let Some(target) = target {
                let shared = self
                    .ai
                    .global
                    .primary_target_multiplicity_scratch
                    .entry(target.get())
                    .or_insert(0);
                *shared = u32::from((*shared as u16).wrapping_add(1));
            }
        }
    }
}
