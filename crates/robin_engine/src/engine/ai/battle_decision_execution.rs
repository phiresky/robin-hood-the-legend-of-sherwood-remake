//! Battle decisions retain local choices while actor calls complete inline.

use super::*;
#[cfg(test)]
use crate::ai::AiEntityHandle;
use crate::ai::{
    AiSpeechAttempt, AiState, Decision, DutyFlags, EmoticonType, GotoFlags, HumanHandle, Remark,
    Substate,
};
use crate::ai_enemy::{
    AiMapVec, BattleDecisionInputs, PrimaryTargetFlags, SeekFlags, archer, combat,
};
use crate::sim_rng::SimulationContext;
use std::ops::ControlFlow;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::ai::battle_decision_observation_tests::fixture;

    #[test]
    fn alert_and_tower_decisions_without_targets_settle_as_reserve() {
        for decision in [
            Decision::AlertSoldiers,
            Decision::TowerGuardAlert,
            Decision::TowerGuardObserve,
        ] {
            let (mut engine, assets, owner, _) = fixture(false);
            engine
                .world
                .entities
                .expect_enemy_ai_mut(owner, format_args!("empty battle"))
                .list_them
                .clear();
            let result = engine.execute_live_battle_decision(
                &crate::sim_rng::test_context(),
                &assets,
                owner,
                decision,
                Substate::AttackingReactiontime,
                0,
                false,
            );
            assert_eq!(result, Some(Decision::Reserve));
            let ai = engine
                .world
                .entities
                .expect_enemy_ai(owner, format_args!("reserve fallback"));
            assert_eq!(ai.base.primary_target, None);
            assert!(!ai.base.friends_are_alerted);
            assert_eq!(ai.base.current_substate, Substate::AttackingReserve);
            assert_eq!(ai.base.when_does_timer_ring, 150);
            assert_eq!(engine.ai_think_depth(), 1);
        }
    }

    #[test]
    fn forced_tower_decision_retains_force_and_reads_current_exact_target_position() {
        let (mut engine, assets, owner, target) = fixture(false);
        let ai = engine
            .world
            .entities
            .expect_enemy_ai_mut(owner, format_args!("forced tower"));
        ai.base.owner_entity_id = Some(owner);
        ai.tower_guard = true;
        ai.forced_next_battle_decision = Decision::TowerGuardAlert;
        for (x, y) in [(650.0, 100.0), (610.0, 130.0)] {
            engine
                .world
                .entities
                .get_mut(target)
                .unwrap()
                .element_data_mut()
                .set_position_map(MapPoint::new(x, y));
            let expected = engine.live_ai_position(target);
            engine.execute_battle_decisions(&crate::sim_rng::test_context(), &assets, owner);
            let ai = engine
                .world
                .entities
                .expect_enemy_ai(owner, format_args!("tower outcome"));
            assert_eq!(ai.base.seek_position, expected);
            assert_eq!(
                ai.base.primary_target,
                Some(AiEntityHandle::new(target.index()))
            );
            assert_eq!(ai.base.current_substate, Substate::AttackingTowerGuardAlert);
            assert_eq!(ai.forced_next_battle_decision, Decision::TowerGuardAlert);
            assert_eq!(
                ai.base.ai_log.last().unwrap().info,
                Decision::TowerGuardAlert as u16
            );
        }
    }
}

impl EngineInner {
    pub(super) fn battle_primary(&self, owner: EntityId) -> Option<EntityId> {
        self.world
            .entities
            .expect_ai_controller(owner, format_args!("battle primary"))
            .primary_target
            .map(|target| self.expect_human_id_for_ai_handle(target.get(), "battle primary"))
    }

    fn select_battle_primary(
        &mut self,
        owner: EntityId,
        flags: PrimaryTargetFlags,
    ) -> Option<EntityId> {
        let target = self.select_live_ai_primary_target(owner, flags);
        self.world
            .entities
            .expect_ai_controller_mut(owner, format_args!("battle target selection"))
            .primary_target = target;
        target.map(|target| {
            self.expect_human_id_for_ai_handle(target.get(), "selected battle target")
        })
    }

    fn focus_battle_primary(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) {
        let ai = self
            .world
            .entities
            .expect_ai_controller_mut(owner, format_args!("battle focus"));
        let target = ai.primary_target;
        self.execute_ai_focus(owner, target);
    }

    pub(super) fn battle_state_timer(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        state: AiState,
        substate: Substate,
        duration: u32,
    ) {
        self.duty_set_state(sim, assets, owner, state, substate);
        self.world
            .entities
            .expect_ai_controller_mut(owner, format_args!("battle timer"))
            .launch_timer(duration, self.control.frame_counter);
    }

    pub(in crate::engine) fn execute_ai_battle_reserve(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) {
        self.select_battle_primary(
            owner,
            PrimaryTargetFlags::UNOCCUPIED_PREFERRED | PrimaryTargetFlags::VIPS_ALLOWED,
        );
        self.focus_battle_primary(sim, assets, owner);
        self.battle_state_timer(
            sim,
            assets,
            owner,
            AiState::Attacking,
            Substate::AttackingReserve,
            50,
        );
    }

    fn battle_command(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        command: crate::element::Command,
    ) {
        self.launch_element(
            sim,
            assets,
            crate::sequence::SequenceElement::new(1, command, Some(owner)),
        );
    }

    fn battle_panic_remark(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) {
        let remark = if crate::sim_rng::bool(sim, crate::sim_rng::RngSite::BattlePanicRemark) {
            Remark::Cassos
        } else {
            Remark::Panic
        };
        self.execute_ai_speech(sim, assets, owner, AiSpeechAttempt { remark, flags: 0 });
    }

    fn battle_forest_merry_man(&self, owner: EntityId) -> bool {
        let entity = self.expect_entity(owner, "forest battle owner");
        self.world.weather.is_forest_level
            && self.is_player_aligned_camp(entity.camp())
            && !entity.soldier_data().is_some_and(|soldier| soldier.rider)
    }

    fn battle_should_follow_lost_enemy(&self, owner: EntityId) -> bool {
        let ai = self
            .world
            .entities
            .expect_enemy_ai(owner, format_args!("battle pursuit policy"));
        let entity = self.expect_entity(owner, "battle pursuit owner");
        ai.base.blood_alcohol as i32 <= crate::parameters_ai::AI_DEBILITY_ALCOHOL_LIMIT
            && (!(entity.is_active()
                && !self.entity_data_in_building_sector(entity.element_data()))
                || (!ai.combat_trainer && ai.company_number != 100))
    }

    pub(super) fn execute_live_battle_without_visible_enemies(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        unconscious: Vec<HumanHandle>,
    ) {
        let ai = self
            .world
            .entities
            .expect_enemy_ai(owner, format_args!("battle without enemies"));
        if ai.combat_trainer {
            self.execute_ai_return_to_duty(sim, assets, owner, DutyFlags::empty());
        } else if ai.my_shooting_point.is_some() {
            let below = self
                .expect_entity(owner, "waiting archer elevation")
                .element_data()
                .position()
                .z
                >= f32::from(ai.enemy_had_this_elevation) + 50.0;
            let (command, substate) = if below {
                (
                    crate::element::Command::EquipBowDown,
                    Substate::AttackingArcherWaitOnArcheryPathBending,
                )
            } else {
                (
                    crate::element::Command::EquipBow,
                    Substate::AttackingArcherWaitOnArcheryPath,
                )
            };
            self.battle_command(sim, assets, owner, command);
            self.battle_state_timer(sim, assets, owner, AiState::Attacking, substate, 1000);
        } else if ai.enemy_seen_below
            && ai.is_archer()
            && self
                .expect_entity(owner, "waiting archer posture")
                .element_data()
                .posture()
                == crate::element::Posture::LeaningOut
        {
            self.battle_state_timer(
                sim,
                assets,
                owner,
                AiState::Attacking,
                Substate::AttackingArcherWaitOnBendPoint,
                500,
            );
        } else if let Some(&target) = ai.list_them.first() {
            let target = self.expect_human_id_for_ai_handle(target, "ally's visible enemy");
            let center = self.live_ai_position(target);
            self.world
                .entities
                .expect_ai_controller_mut(owner, format_args!("ally's enemy search"))
                .seek_position = center;
            self.execute_ai_seek_area(
                sim,
                assets,
                owner,
                center,
                crate::parameters_ai::AI_LOST_ENEMY_SEEK_RADIUS as u16,
                SeekFlags::LOCATION_FIRST,
                crate::ai_enemy::UNDEFINED_DIRECTION,
            );
        } else if ai.pc_missed
            && ai.missed_pc.is_some_and(|target| {
                self.expect_entity(
                    self.expect_human_id_for_ai_handle(target.get(), "missed battle target"),
                    "missed battle target",
                )
                .is_pc()
            })
            && self.battle_should_follow_lost_enemy(owner)
        {
            self.execute_ai_speech(
                sim,
                assets,
                owner,
                AiSpeechAttempt {
                    remark: Remark::HuntsEnemy,
                    flags: 0,
                },
            );
            let ai = self
                .world
                .entities
                .expect_enemy_ai(owner, format_args!("missed battle forecast"));
            if let Some(target) = ai.missed_pc {
                let target = self
                    .expect_human_id_for_ai_handle(target.get(), "missed battle forecast target");
                let input = extract_exact_forecast_input(
                    self,
                    self.expect_entity(target, "missed battle forecast"),
                    selected_actor_is_passing_door(
                        &self.world.entities,
                        &self.orders.sequence_manager,
                        target,
                    ),
                )
                .expect("forecast actor");
                let forecast = crate::ai::prepare_forecast_destination_for_ia(
                    &input,
                    &self.script_domains.interactables.doors,
                    &self.world.fast_grid.level.sectors,
                    &self.world.fast_grid.level.sector_number_map,
                )
                .resolve_retaining_direction(sim, ai.pc_gone_away_in_this_direction);
                let ai = self
                    .world
                    .entities
                    .expect_enemy_ai_mut(owner, format_args!("missed battle forecast result"));
                ai.base.seek_position = forecast.position;
                ai.pc_gone_away_in_this_direction = forecast.direction;
            }
            let ai = self
                .world
                .entities
                .expect_enemy_ai(owner, format_args!("missed battle search"));
            let (center, direction) = (ai.base.seek_position, ai.pc_gone_away_in_this_direction);
            self.execute_ai_seek_area(
                sim,
                assets,
                owner,
                center,
                crate::parameters_ai::AI_LOST_ENEMY_SEEK_RADIUS as u16,
                SeekFlags::LOCATION_FIRST | SeekFlags::HOUSE,
                direction,
            );
        } else if !unconscious.is_empty() && !self.battle_forest_merry_man(owner) {
            self.execute_approach_sleeping_enemies(sim, assets, owner, unconscious);
        } else {
            let camp = self
                .expect_entity(owner, "sleeping enemy search camp")
                .camp();
            self.execute_kill_nearby_sleeping_enemies(sim, assets, owner, camp);
        }
    }

    pub(super) fn choose_live_battle_decision(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        inputs: BattleDecisionInputs,
    ) -> (Decision, HumanHandle) {
        let forced = self
            .world
            .entities
            .expect_enemy_ai(owner, format_args!("forced battle decision"))
            .forced_next_battle_decision;
        if forced != Decision::None {
            assert!(
                matches!(
                    forced,
                    Decision::Cassos
                        | Decision::Fight
                        | Decision::Observe
                        | Decision::Reserve
                        | Decision::Menace
                        | Decision::Shoot
                        | Decision::ArcherStepBack
                        | Decision::LookForHelp
                        | Decision::TooProudToAttack
                        | Decision::TowerGuardAlert
                        | Decision::TowerGuardObserve
                        | Decision::ArcherObserve
                ),
                "unsupported forced battle decision {forced:?}"
            );
            return (forced, 0);
        }
        let predecision = self.execute_ai_make_battle_predecisions(sim, assets, owner);
        let ai = self
            .world
            .entities
            .expect_enemy_ai(owner, format_args!("battle decision tree"));
        if ai.combat_trainer {
            return (Decision::Observe, 0);
        }
        if predecision != Decision::PredecisionOffensive {
            let only_soldiers = !ai.list_them.iter().any(|&target| {
                self.expect_entity(
                    self.expect_human_id_for_ai_handle(target, "battle enemy kind"),
                    "battle enemy kind",
                )
                .is_pc()
            });
            let decision = if ai.is_archer()
                && self
                    .expect_entity(owner, "battle arrows")
                    .ai_actor_data()
                    .unwrap()
                    .number_of_arrows
                    == 0
            {
                Decision::RunForNewArrows
            } else if !ai.base.friends_are_alerted && !only_soldiers && ai.base.blood_alcohol == 0 {
                match ai.get_rank(&assets.profile_manager) {
                    crate::profiles::ProfileRank::Soldier => Decision::LookForHelp,
                    crate::profiles::ProfileRank::Officer => Decision::RunAndAlertSoldiers,
                    _ => Decision::Cassos,
                }
            } else {
                Decision::Cassos
            };
            return (decision, 0);
        }
        if ai.is_archer() && ai.base.blood_alcohol == 0 {
            if ai.tower_guard {
                return (
                    if ai.base.friends_are_alerted {
                        Decision::Shoot
                    } else {
                        Decision::TowerGuardAlert
                    },
                    0,
                );
            }
            if self.live_archer_is_too_near(assets, owner, ai.base.primary_target) {
                return (Decision::ArcherStepBack, 0);
            }
            let ai = self
                .world
                .entities
                .expect_enemy_ai(owner, format_args!("archer cover decision"));
            if let Some(bearer) = ai.shield_bearer_before_me {
                let bearer_id =
                    self.expect_human_id_for_ai_handle(bearer.get(), "archer shield bearer");
                let (position, direction) = self.live_shield_bearer_position(bearer_id);
                let [dx, dy] = crate::shadow_polygon::sector_to_direction(direction as i16);
                let scale = archer::DISTANCE_SHIELD_BEARER_ARCHER as f32;
                let desired = crate::coordinates::MapVec::new(
                    dx * scale,
                    (dy * crate::position_interface::ASPECT_RATIO) * scale,
                );
                let actual = position.map_point() - self.live_ai_position(owner).map_point();
                return if (actual - desired).max_norm() < archer::COVER_POINT_TOLERANCE as f32 {
                    (Decision::Shoot, 0)
                } else {
                    (Decision::CoverBehindShieldBearer, bearer.get())
                };
            }
            if ai.my_shooting_point.is_some() {
                return (Decision::Shoot, 0);
            }
            if self.choose_ai_good_shooting_point(assets, owner) {
                return (Decision::RunToArcheryPoint, 0);
            }
            return self
                .nearest_live_free_shield_bearer(assets, owner)
                .map_or((Decision::Shoot, 0), |bearer| {
                    (Decision::CoverBehindShieldBearer, bearer.index())
                });
        }
        if ai.tower_guard {
            return (
                if !ai.base.friends_are_alerted {
                    Decision::TowerGuardAlert
                } else if inputs.min_square_enemy_distance
                    < combat::MIN_SQUARE_RESERVE_DISTANCE as u32
                {
                    Decision::Fight
                } else {
                    Decision::TowerGuardObserve
                },
                0,
            );
        }
        if ai.get_rank(&assets.profile_manager) == crate::profiles::ProfileRank::Officer
            && inputs.simple_soldiers_near
            && !ai.base.friends_are_alerted
            && ai.base.blood_alcohol == 0
        {
            return (Decision::AlertSoldiers, 0);
        }
        if inputs.friends_lower_company >= ai.list_them.len() as u16
            && inputs.min_square_enemy_distance > combat::MIN_SQUARE_RESERVE_DISTANCE as u32
        {
            return (Decision::Reserve, 0);
        }
        if ai.company_number == 100
            && inputs.min_square_enemy_distance > combat::MIN_SQUARE_RESERVE_DISTANCE as u32
        {
            return (Decision::LastReserve, 0);
        }
        if inputs.soldiers_lower_pride && self.live_ai_is_too_proud_to_attack(assets, owner) {
            return (Decision::TooProudToAttack, 0);
        }
        if self.expect_entity(owner, "observing soldier camp").camp() == Camp::Lacklandists
            && !inputs.soldiers_lower_pride
        {
            let courage = self
                .world
                .entities
                .expect_enemy_ai(owner, format_args!("observe courage"))
                .get_courage(&assets.profile_manager);
            let enemies = inputs.num_enemies_i_can_see as f32;
            if f32::from(inputs.friends_nearer_to_enemy)
                >= enemies + enemies * (0.045_f32 * f32::from(courage))
            {
                return (Decision::Observe, 0);
            }
        }
        (Decision::Fight, 0)
    }

    pub(in crate::engine) fn execute_live_battle_decision(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        mut decision: Decision,
        old_substate: Substate,
        cover: HumanHandle,
        alerting_near: bool,
    ) -> Option<Decision> {
        loop {
            let outcome = match decision {
                Decision::Fight => {
                    if let Some(target) =
                        self.select_battle_primary(owner, PrimaryTargetFlags::UNOCCUPIED_PREFERRED)
                    {
                        self.execute_ai_attack_enemy(sim, assets, owner, target.index());
                        let ai = self
                            .world
                            .entities
                            .expect_ai_controller_mut(owner, format_args!("battle attack result"));
                        if ai.couldnt_reachpoint {
                            ai.couldnt_reachpoint = false;
                            ControlFlow::Continue(Decision::Observe)
                        } else {
                            ControlFlow::Break(true)
                        }
                    } else {
                        ControlFlow::Continue(Decision::Observe)
                    }
                }
                Decision::Reserve => {
                    self.execute_ai_battle_reserve(sim, assets, owner);
                    ControlFlow::Break(true)
                }
                Decision::LastReserve => {
                    self.select_battle_primary(
                        owner,
                        PrimaryTargetFlags::UNOCCUPIED_PREFERRED | PrimaryTargetFlags::VIPS_ALLOWED,
                    );
                    if self
                        .expect_entity(owner, "reserve action")
                        .actor_data()
                        .unwrap()
                        .action_state
                        .is_sword()
                    {
                        if crate::sim_rng::u32(sim, crate::sim_rng::RngSite::BattleProvoke, 0..4)
                            == 0
                        {
                            self.battle_command(
                                sim,
                                assets,
                                owner,
                                crate::element::Command::Provoke,
                            );
                        } else if let Some(target) = self.battle_primary(owner) {
                            let direction = (self.live_ai_position(target).map_point()
                                - self.live_ai_position(owner).map_point())
                            .sector_with_aspect(crate::position_interface::ASPECT_RATIO);
                            self.world
                                .entities
                                .expect_entity_mut(
                                    owner,
                                    format_args!("instant AI direction owner"),
                                )
                                .element_data_mut()
                                .set_direction_instantly(direction as i16);
                        }
                    } else {
                        self.launch_ai_raise_sword(sim, assets, owner);
                    }
                    self.focus_battle_primary(sim, assets, owner);
                    self.battle_state_timer(
                        sim,
                        assets,
                        owner,
                        AiState::Attacking,
                        Substate::AttackingLastReserve,
                        50,
                    );
                    ControlFlow::Break(true)
                }
                Decision::Observe => {
                    ControlFlow::Break(self.execute_live_battle_observe(sim, assets, owner))
                }
                Decision::Shoot => self.execute_live_shoot_decision(sim, assets, owner),
                Decision::Cassos => {
                    if !self.battle_forest_merry_man(owner)
                        || !self.execute_ai_merry_man_forest_cassos(sim, assets, owner)
                    {
                        self.battle_panic_remark(sim, assets, owner);
                        let target =
                            self.select_battle_primary(owner, PrimaryTargetFlags::VIPS_ALLOWED);
                        let center = target.map(|target| self.live_ai_position(target));
                        self.execute_ai_panic(
                            sim,
                            assets,
                            owner,
                            center,
                            crate::parameters_ai::AI_STANDARD_PANIC_RUNS as u8,
                            crate::ai::AlertLevel::Red,
                        );
                    }
                    ControlFlow::Break(true)
                }
                Decision::LookForHelp => {
                    self.execute_live_look_for_help_decision(sim, assets, owner, alerting_near)
                }
                Decision::AlertSoldiers => {
                    if let Some(target) =
                        self.select_battle_primary(owner, PrimaryTargetFlags::VIPS_ALLOWED)
                    {
                        self.world
                            .entities
                            .expect_ai_controller_mut(owner, format_args!("battle alert latch"))
                            .friends_are_alerted = true;
                        let center = self.live_ai_position(target);
                        if self.execute_ai_command_soldiers_to_attack(sim, assets, owner, center) {
                            self.execute_ai_speech(
                                sim,
                                assets,
                                owner,
                                AiSpeechAttempt {
                                    remark: Remark::OfficerGivesAttackOrder,
                                    flags: 0,
                                },
                            );
                            ControlFlow::Break(true)
                        } else {
                            ControlFlow::Continue(Decision::Reserve)
                        }
                    } else {
                        ControlFlow::Continue(Decision::Reserve)
                    }
                }
                Decision::RunAndAlertSoldiers => {
                    let target = self
                        .select_battle_primary(owner, PrimaryTargetFlags::VIPS_ALLOWED)
                        .expect("run-and-alert requires target");
                    self.world
                        .entities
                        .expect_ai_controller_mut(owner, format_args!("battle run alert latch"))
                        .friends_are_alerted = true;
                    let center = self.live_ai_position(target);
                    if self.execute_ai_run_and_alert_soldiers(sim, assets, owner, center) {
                        self.battle_panic_remark(sim, assets, owner);
                        ControlFlow::Break(true)
                    } else {
                        ControlFlow::Continue(Decision::Cassos)
                    }
                }
                Decision::TowerGuardAlert | Decision::TowerGuardObserve => {
                    if let Some(target) =
                        self.select_battle_primary(owner, PrimaryTargetFlags::VIPS_ALLOWED)
                    {
                        let position = self.live_ai_position(target);
                        let ai = self
                            .world
                            .entities
                            .expect_ai_controller_mut(owner, format_args!("tower target"));
                        ai.friends_are_alerted = true;
                        ai.seek_position = position;
                        if decision == Decision::TowerGuardAlert {
                            self.duty_set_state(
                                sim,
                                assets,
                                owner,
                                AiState::Attacking,
                                Substate::AttackingTowerGuardAlert,
                            );
                            let position = self
                                .world
                                .entities
                                .expect_ai_controller(owner, format_args!("tower point"))
                                .seek_position;
                            self.duty_point_to(sim, assets, owner, position);
                        } else {
                            self.duty_set_state(
                                sim,
                                assets,
                                owner,
                                AiState::Attacking,
                                Substate::AttackingTowerGuardObserve,
                            );
                            let target = self.battle_primary(owner).expect("tower face target");
                            let position = self.live_ai_position(target);
                            let elevation = self
                                .expect_entity(target, "tower face elevation")
                                .element_data()
                                .position()
                                .z;
                            self.duty_face_position_at_elevation(
                                sim, assets, owner, position, elevation,
                            );
                            self.world
                                .entities
                                .expect_ai_controller_mut(
                                    owner,
                                    format_args!("tower observation timer"),
                                )
                                .launch_timer(100, self.control.frame_counter);
                        }
                        ControlFlow::Break(true)
                    } else {
                        ControlFlow::Continue(Decision::Reserve)
                    }
                }
                Decision::Menace => {
                    self.select_battle_primary(owner, PrimaryTargetFlags::VIPS_ALLOWED)
                        .expect("menace requires target");
                    self.battle_state_timer(
                        sim,
                        assets,
                        owner,
                        AiState::Menacing,
                        Substate::MenacingPcInComa,
                        crate::parameters_ai::AI_MENACING_PATIENCE as u32,
                    );
                    ControlFlow::Break(true)
                }
                Decision::RunForNewArrows => {
                    self.execute_ai_battle_run_for_arrows(sim, assets, owner)
                }
                Decision::RunToArcheryPoint => {
                    self.execute_ai_battle_archery_point(sim, assets, owner)
                }
                Decision::TooProudToAttack => {
                    self.execute_ai_battle_too_proud(sim, assets, owner, old_substate)
                }
                Decision::ArcherStepBack => {
                    self.execute_ai_battle_archer_step_back(sim, assets, owner, old_substate)
                }
                Decision::ArcherObserve => {
                    self.execute_ai_battle_archer_observe(sim, assets, owner)
                }
                Decision::CoverBehindShieldBearer => {
                    self.execute_ai_battle_cover(sim, assets, owner, cover)
                }
                _ => panic!("unsupported battle decision {decision:?}"),
            };
            match outcome {
                ControlFlow::Continue(next) => decision = next,
                ControlFlow::Break(log) => return log.then_some(decision),
            }
        }
    }

    fn execute_live_look_for_help_decision(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        alerting_near: bool,
    ) -> ControlFlow<bool, Decision> {
        self.select_battle_primary(owner, PrimaryTargetFlags::VIPS_ALLOWED);
        self.world
            .entities
            .expect_ai_controller_mut(owner, format_args!("look for help latch"))
            .friends_are_alerted = true;
        if alerting_near || !self.execute_ai_alert_officer(sim, assets, owner) {
            ControlFlow::Continue(Decision::Cassos)
        } else {
            self.battle_panic_remark(sim, assets, owner);
            ControlFlow::Break(true)
        }
    }

    fn execute_live_battle_observe(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) -> bool {
        self.select_battle_primary(
            owner,
            PrimaryTargetFlags::UNOCCUPIED_PREFERRED | PrimaryTargetFlags::VIPS_ALLOWED,
        );
        self.focus_battle_primary(sim, assets, owner);
        let ai = self
            .world
            .entities
            .expect_enemy_ai(owner, format_args!("battle observe"));
        let trainer = ai.combat_trainer;
        if !trainer {
            let target = self.battle_primary(owner).expect("observe requires target");
            let destination = self.live_ai_position(target);
            let distance = crate::ai::AiController::value_between(
                crate::parameters_ai::OBSERVE_SWORDFIGHT_MAX_DISTANCE,
                crate::parameters_ai::OBSERVE_SWORDFIGHT_MIN_DISTANCE,
                ai.get_courage(&assets.profile_manager) as u8,
            );
            self.duty_go_near(
                sim,
                assets,
                owner,
                destination,
                i32::from(distance),
                GotoFlags::empty(),
            );
        }
        self.world
            .entities
            .expect_ai_controller_mut(owner, format_args!("observe emoticon"))
            .set_emoticon(EmoticonType::XMark);
        self.battle_state_timer(
            sim,
            assets,
            owner,
            AiState::Attacking,
            Substate::AttackingApproachToObserve,
            if trainer { 1 } else { 50 },
        );
        if !trainer
            && self
                .world
                .entities
                .expect_ai_controller(owner, format_args!("observe route result"))
                .couldnt_reachpoint
        {
            let target = self.battle_primary(owner).expect("observe roof target");
            let wait = precompute_avenger_on_roof_wait_position(
                &self.world.entities,
                self.script_domains.interactables.doors.as_slice(),
                &self.orders.sequence_manager,
                owner,
                target,
                |element| super::ai_view_position_sector(self, element),
                &|sector| self.building_sector_is_authorized(sector),
                &|sector| self.get_sector_lift_type(sector),
            );
            if let Some(wait) = wait {
                self.world
                    .entities
                    .expect_ai_controller_mut(owner, format_args!("observe roof route reset"))
                    .couldnt_reachpoint = false;
                self.duty_set_state(
                    sim,
                    assets,
                    owner,
                    AiState::Attacking,
                    Substate::AttackingRunToAvengerOnRoof,
                );
                self.duty_go_near(sim, assets, owner, wait, 50, GotoFlags::RUN);
                let target = self
                    .battle_primary(owner)
                    .expect("observe roof current target");
                let position = self.live_ai_position(target);
                self.world
                    .entities
                    .expect_ai_controller_mut(owner, format_args!("observe roof seek position"))
                    .seek_position = position;
                return false;
            }
        }
        true
    }
}
