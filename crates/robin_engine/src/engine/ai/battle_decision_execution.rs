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
use crate::engine::TickCtx;
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
                TickCtx::new(&crate::sim_rng::test_context(), &assets),
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
            engine.execute_battle_decisions(
                TickCtx::new(&crate::sim_rng::test_context(), &assets),
                owner,
            );
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
        self.ai(owner, "battle primary")
            .primary_target
            .map(|target| self.expect_human_id_for_ai_handle(target.get(), "battle primary"))
    }

    fn select_battle_primary(
        &mut self,
        owner: EntityId,
        flags: PrimaryTargetFlags,
    ) -> Option<EntityId> {
        let target = self.select_live_ai_primary_target(owner, flags);
        self.ai_mut(owner, "battle target selection").primary_target = target;
        target.map(|target| {
            self.expect_human_id_for_ai_handle(target.get(), "selected battle target")
        })
    }

    fn focus_battle_primary(&mut self, owner: EntityId) {
        let ai = self.ai_mut(owner, "battle focus");
        let target = ai.primary_target;
        self.execute_ai_focus(owner, target);
    }

    fn battle_forest_merry_man(&self, owner: EntityId) -> bool {
        let entity = self.expect_entity(owner, "forest battle owner");
        self.world.weather.is_forest_level
            && self.is_player_aligned_camp(entity.camp())
            && !entity.soldier_data().is_some_and(|soldier| soldier.rider)
    }

    fn battle_should_follow_lost_enemy(&self, owner: EntityId) -> bool {
        let ai = self.enemy_ai(owner, "battle pursuit policy");
        let entity = self.expect_entity(owner, "battle pursuit owner");
        ai.base.blood_alcohol as i32 <= crate::parameters_ai::AI_DEBILITY_ALCOHOL_LIMIT
            && (!(entity.is_active()
                && !self.entity_data_in_building_sector(entity.element_data()))
                || (!ai.combat_trainer && ai.company_number != 100))
    }

    #[cfg(test)]
    pub(super) fn execute_live_battle_without_visible_enemies(
        &mut self,
        tcx: TickCtx<'_>,
        owner: EntityId,
        unconscious: Vec<HumanHandle>,
    ) {
        AiOwnerCtx::new(self, tcx, owner).execute_live_battle_without_visible_enemies(unconscious)
    }

    pub(super) fn choose_live_battle_decision(
        &mut self,
        tcx: TickCtx<'_>,
        owner: EntityId,
        inputs: BattleDecisionInputs,
    ) -> (Decision, HumanHandle) {
        let forced = self
            .enemy_ai(owner, "forced battle decision")
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
        let predecision = self.execute_ai_make_battle_predecisions(tcx, owner);
        let ai = self.enemy_ai(owner, "battle decision tree");
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
                match ai.get_rank(&tcx.assets.profile_manager) {
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
            if self.live_archer_is_too_near(tcx.assets, owner, ai.base.primary_target) {
                return (Decision::ArcherStepBack, 0);
            }
            let ai = self.enemy_ai(owner, "archer cover decision");
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
            if self.choose_ai_good_shooting_point(tcx.assets, owner) {
                return (Decision::RunToArcheryPoint, 0);
            }
            return self
                .nearest_live_free_shield_bearer(tcx.assets, owner)
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
        if ai.get_rank(&tcx.assets.profile_manager) == crate::profiles::ProfileRank::Officer
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
        if inputs.soldiers_lower_pride && self.live_ai_is_too_proud_to_attack(tcx.assets, owner) {
            return (Decision::TooProudToAttack, 0);
        }
        if self.expect_entity(owner, "observing soldier camp").camp() == Camp::Lacklandists
            && !inputs.soldiers_lower_pride
        {
            let courage = self
                .enemy_ai(owner, "observe courage")
                .get_courage(&tcx.assets.profile_manager);
            let enemies = inputs.num_enemies_i_can_see as f32;
            if f32::from(inputs.friends_nearer_to_enemy)
                >= enemies + enemies * (0.045_f32 * f32::from(courage))
            {
                return (Decision::Observe, 0);
            }
        }
        (Decision::Fight, 0)
    }

    #[cfg(test)]
    pub(in crate::engine) fn execute_live_battle_decision(
        &mut self,
        tcx: TickCtx<'_>,
        owner: EntityId,
        decision: Decision,
        old_substate: Substate,
        cover: HumanHandle,
        alerting_near: bool,
    ) -> Option<Decision> {
        AiOwnerCtx::new(self, tcx, owner).execute_live_battle_decision(
            decision,
            old_substate,
            cover,
            alerting_near,
        )
    }
}

impl AiOwnerCtx<'_> {
    pub(super) fn battle_state_timer(&mut self, state: AiState, substate: Substate, duration: u32) {
        self.duty_set_state(state, substate);
        self.engine
            .world
            .entities
            .expect_ai_controller_mut(self.owner, format_args!("battle timer"))
            .launch_timer(duration, self.engine.control.frame_counter);
    }

    pub(in crate::engine) fn execute_ai_battle_reserve(&mut self) {
        self.engine.select_battle_primary(
            self.owner,
            PrimaryTargetFlags::UNOCCUPIED_PREFERRED | PrimaryTargetFlags::VIPS_ALLOWED,
        );
        self.engine.focus_battle_primary(self.owner);
        self.battle_state_timer(AiState::Attacking, Substate::AttackingReserve, 50);
    }

    fn battle_command(&mut self, command: crate::element::Command) {
        self.engine.launch_element(
            TickCtx::new(self.sim, self.assets),
            crate::sequence::SequenceElement::new(1, command, Some(self.owner)),
        );
    }

    fn battle_panic_remark(&mut self) {
        let remark = if crate::sim_rng::bool(self.sim, crate::sim_rng::RngSite::BattlePanicRemark) {
            Remark::Cassos
        } else {
            Remark::Panic
        };
        self.execute_ai_speech(AiSpeechAttempt { remark, flags: 0 });
    }

    pub(super) fn execute_live_battle_without_visible_enemies(
        &mut self,
        unconscious: Vec<HumanHandle>,
    ) {
        let ai = self.engine.enemy_ai(self.owner, "battle without enemies");
        if ai.combat_trainer {
            self.execute_ai_return_to_duty(DutyFlags::empty());
        } else if ai.my_shooting_point.is_some() {
            let below = self
                .engine
                .expect_entity(self.owner, "waiting archer elevation")
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
            self.battle_command(command);
            self.battle_state_timer(AiState::Attacking, substate, 1000);
        } else if ai.enemy_seen_below
            && ai.is_archer()
            && self
                .engine
                .expect_entity(self.owner, "waiting archer posture")
                .element_data()
                .posture()
                == crate::element::Posture::LeaningOut
        {
            self.battle_state_timer(
                AiState::Attacking,
                Substate::AttackingArcherWaitOnBendPoint,
                500,
            );
        } else if let Some(&target) = ai.list_them.first() {
            let target = self
                .engine
                .expect_human_id_for_ai_handle(target, "ally's visible enemy");
            let center = self.engine.live_ai_position(target);
            self.engine
                .ai_mut(self.owner, "ally's enemy search")
                .seek_position = center;
            self.execute_ai_seek_area(
                center,
                crate::parameters_ai::AI_LOST_ENEMY_SEEK_RADIUS as u16,
                SeekFlags::LOCATION_FIRST,
                crate::ai_enemy::UNDEFINED_DIRECTION,
            );
        } else if ai.pc_missed
            && ai.missed_pc.is_some_and(|target| {
                self.engine
                    .expect_entity(
                        self.engine
                            .expect_human_id_for_ai_handle(target.get(), "missed battle target"),
                        "missed battle target",
                    )
                    .is_pc()
            })
            && self.engine.battle_should_follow_lost_enemy(self.owner)
        {
            self.execute_ai_speech(AiSpeechAttempt {
                remark: Remark::HuntsEnemy,
                flags: 0,
            });
            let ai = self.engine.enemy_ai(self.owner, "missed battle forecast");
            if let Some(target) = ai.missed_pc {
                let target = self
                    .engine
                    .expect_human_id_for_ai_handle(target.get(), "missed battle forecast target");
                let input = extract_exact_forecast_input(
                    self.engine,
                    self.engine.expect_entity(target, "missed battle forecast"),
                    selected_actor_is_passing_door(
                        &self.engine.entities(),
                        &self.engine.seq(),
                        target,
                    ),
                )
                .expect("forecast actor");
                let forecast = crate::ai::prepare_forecast_destination_for_ia(
                    &input,
                    &self.engine.script_domains.interactables.doors,
                    &self.engine.world.fast_grid.level.sectors,
                    &self.engine.world.fast_grid.level.sector_number_map,
                )
                .resolve_retaining_direction(self.sim, ai.pc_gone_away_in_this_direction);
                let ai = self
                    .engine
                    .enemy_ai_mut(self.owner, "missed battle forecast result");
                ai.base.seek_position = forecast.position;
                ai.pc_gone_away_in_this_direction = forecast.direction;
            }
            let ai = self.engine.enemy_ai(self.owner, "missed battle search");
            let (center, direction) = (ai.base.seek_position, ai.pc_gone_away_in_this_direction);
            self.execute_ai_seek_area(
                center,
                crate::parameters_ai::AI_LOST_ENEMY_SEEK_RADIUS as u16,
                SeekFlags::LOCATION_FIRST | SeekFlags::HOUSE,
                direction,
            );
        } else if !unconscious.is_empty() && !self.engine.battle_forest_merry_man(self.owner) {
            self.engine.execute_approach_sleeping_enemies(
                TickCtx::new(self.sim, self.assets),
                self.owner,
                unconscious,
            );
        } else {
            let camp = self
                .engine
                .expect_entity(self.owner, "sleeping enemy search camp")
                .camp();
            self.engine.execute_kill_nearby_sleeping_enemies(
                TickCtx::new(self.sim, self.assets),
                self.owner,
                camp,
            );
        }
    }

    pub(in crate::engine) fn execute_live_battle_decision(
        &mut self,
        mut decision: Decision,
        old_substate: Substate,
        cover: HumanHandle,
        alerting_near: bool,
    ) -> Option<Decision> {
        loop {
            let outcome = match decision {
                Decision::Fight => {
                    if let Some(target) = self
                        .engine
                        .select_battle_primary(self.owner, PrimaryTargetFlags::UNOCCUPIED_PREFERRED)
                    {
                        self.execute_ai_attack_enemy(target.index());
                        let ai = self.engine.ai_mut(self.owner, "battle attack result");
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
                    self.execute_ai_battle_reserve();
                    ControlFlow::Break(true)
                }
                Decision::LastReserve => {
                    self.engine.select_battle_primary(
                        self.owner,
                        PrimaryTargetFlags::UNOCCUPIED_PREFERRED | PrimaryTargetFlags::VIPS_ALLOWED,
                    );
                    if self
                        .engine
                        .expect_entity(self.owner, "reserve action")
                        .actor_data()
                        .unwrap()
                        .action_state
                        .is_sword()
                    {
                        if crate::sim_rng::u32(
                            self.sim,
                            crate::sim_rng::RngSite::BattleProvoke,
                            0..4,
                        ) == 0
                        {
                            self.battle_command(crate::element::Command::Provoke);
                        } else if let Some(target) = self.engine.battle_primary(self.owner) {
                            let direction = (self.engine.live_ai_position(target).map_point()
                                - self.engine.live_ai_position(self.owner).map_point())
                            .sector_with_aspect(crate::position_interface::ASPECT_RATIO);
                            self.engine
                                .entities_mut()
                                .expect_entity_mut(
                                    self.owner,
                                    format_args!("instant AI direction owner"),
                                )
                                .element_data_mut()
                                .set_direction_instantly(direction as i16);
                        }
                    } else {
                        self.launch_ai_raise_sword();
                    }
                    self.engine.focus_battle_primary(self.owner);
                    self.battle_state_timer(AiState::Attacking, Substate::AttackingLastReserve, 50);
                    ControlFlow::Break(true)
                }
                Decision::Observe => ControlFlow::Break(self.execute_live_battle_observe()),
                Decision::Shoot => self.execute_live_shoot_decision(),
                Decision::Cassos => {
                    if !self.engine.battle_forest_merry_man(self.owner)
                        || !self.execute_ai_merry_man_forest_cassos()
                    {
                        self.battle_panic_remark();
                        let target = self
                            .engine
                            .select_battle_primary(self.owner, PrimaryTargetFlags::VIPS_ALLOWED);
                        let center = target.map(|target| self.engine.live_ai_position(target));
                        self.engine.execute_ai_panic(
                            TickCtx::new(self.sim, self.assets),
                            self.owner,
                            center,
                            crate::parameters_ai::AI_STANDARD_PANIC_RUNS as u8,
                            crate::ai::AlertLevel::Red,
                        );
                    }
                    ControlFlow::Break(true)
                }
                Decision::LookForHelp => self.execute_live_look_for_help_decision(alerting_near),
                Decision::AlertSoldiers => {
                    if let Some(target) = self
                        .engine
                        .select_battle_primary(self.owner, PrimaryTargetFlags::VIPS_ALLOWED)
                    {
                        self.engine
                            .ai_mut(self.owner, "battle alert latch")
                            .friends_are_alerted = true;
                        let center = self.engine.live_ai_position(target);
                        if self.engine.execute_ai_command_soldiers_to_attack(
                            TickCtx::new(self.sim, self.assets),
                            self.owner,
                            center,
                        ) {
                            self.execute_ai_speech(AiSpeechAttempt {
                                remark: Remark::OfficerGivesAttackOrder,
                                flags: 0,
                            });
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
                        .engine
                        .select_battle_primary(self.owner, PrimaryTargetFlags::VIPS_ALLOWED)
                        .expect("run-and-alert requires target");
                    self.engine
                        .ai_mut(self.owner, "battle run alert latch")
                        .friends_are_alerted = true;
                    let center = self.engine.live_ai_position(target);
                    if self.execute_ai_run_and_alert_soldiers(center) {
                        self.battle_panic_remark();
                        ControlFlow::Break(true)
                    } else {
                        ControlFlow::Continue(Decision::Cassos)
                    }
                }
                Decision::TowerGuardAlert | Decision::TowerGuardObserve => {
                    if let Some(target) = self
                        .engine
                        .select_battle_primary(self.owner, PrimaryTargetFlags::VIPS_ALLOWED)
                    {
                        let position = self.engine.live_ai_position(target);
                        let ai = self.engine.ai_mut(self.owner, "tower target");
                        ai.friends_are_alerted = true;
                        ai.seek_position = position;
                        if decision == Decision::TowerGuardAlert {
                            self.duty_set_state(
                                AiState::Attacking,
                                Substate::AttackingTowerGuardAlert,
                            );
                            let position = self.engine.ai(self.owner, "tower point").seek_position;
                            self.duty_point_to(position);
                        } else {
                            self.duty_set_state(
                                AiState::Attacking,
                                Substate::AttackingTowerGuardObserve,
                            );
                            let target = self
                                .engine
                                .battle_primary(self.owner)
                                .expect("tower face target");
                            let position = self.engine.live_ai_position(target);
                            let elevation = self
                                .engine
                                .expect_entity(target, "tower face elevation")
                                .element_data()
                                .position()
                                .z;
                            self.duty_face_position_at_elevation(position, elevation);
                            self.engine
                                .world
                                .entities
                                .expect_ai_controller_mut(
                                    self.owner,
                                    format_args!("tower observation timer"),
                                )
                                .launch_timer(100, self.engine.control.frame_counter);
                        }
                        ControlFlow::Break(true)
                    } else {
                        ControlFlow::Continue(Decision::Reserve)
                    }
                }
                Decision::Menace => {
                    self.engine
                        .select_battle_primary(self.owner, PrimaryTargetFlags::VIPS_ALLOWED)
                        .expect("menace requires target");
                    self.battle_state_timer(
                        AiState::Menacing,
                        Substate::MenacingPcInComa,
                        crate::parameters_ai::AI_MENACING_PATIENCE as u32,
                    );
                    ControlFlow::Break(true)
                }
                Decision::RunForNewArrows => self.execute_ai_battle_run_for_arrows(),
                Decision::RunToArcheryPoint => self.engine.execute_ai_battle_archery_point(
                    TickCtx::new(self.sim, self.assets),
                    self.owner,
                ),
                Decision::TooProudToAttack => self.execute_ai_battle_too_proud(old_substate),
                Decision::ArcherStepBack => self.engine.execute_ai_battle_archer_step_back(
                    TickCtx::new(self.sim, self.assets),
                    self.owner,
                    old_substate,
                ),
                Decision::ArcherObserve => self.execute_ai_battle_archer_observe(),
                Decision::CoverBehindShieldBearer => self.engine.execute_ai_battle_cover(
                    TickCtx::new(self.sim, self.assets),
                    self.owner,
                    cover,
                ),
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
        alerting_near: bool,
    ) -> ControlFlow<bool, Decision> {
        self.engine
            .select_battle_primary(self.owner, PrimaryTargetFlags::VIPS_ALLOWED);
        self.engine
            .ai_mut(self.owner, "look for help latch")
            .friends_are_alerted = true;
        if alerting_near || !self.execute_ai_alert_officer() {
            ControlFlow::Continue(Decision::Cassos)
        } else {
            self.battle_panic_remark();
            ControlFlow::Break(true)
        }
    }

    fn execute_live_battle_observe(&mut self) -> bool {
        self.engine.select_battle_primary(
            self.owner,
            PrimaryTargetFlags::UNOCCUPIED_PREFERRED | PrimaryTargetFlags::VIPS_ALLOWED,
        );
        self.engine.focus_battle_primary(self.owner);
        let ai = self.engine.enemy_ai(self.owner, "battle observe");
        let trainer = ai.combat_trainer;
        if !trainer {
            let target = self
                .engine
                .battle_primary(self.owner)
                .expect("observe requires target");
            let destination = self.engine.live_ai_position(target);
            let distance = crate::ai::AiController::value_between(
                crate::parameters_ai::OBSERVE_SWORDFIGHT_MAX_DISTANCE,
                crate::parameters_ai::OBSERVE_SWORDFIGHT_MIN_DISTANCE,
                ai.get_courage(&self.assets.profile_manager) as u8,
            );
            self.duty_go_near(destination, i32::from(distance), GotoFlags::empty());
        }
        self.engine
            .ai_mut(self.owner, "observe emoticon")
            .set_emoticon(EmoticonType::XMark);
        self.battle_state_timer(
            AiState::Attacking,
            Substate::AttackingApproachToObserve,
            if trainer { 1 } else { 50 },
        );
        if !trainer
            && self
                .engine
                .ai(self.owner, "observe route result")
                .couldnt_reachpoint
        {
            let target = self
                .engine
                .battle_primary(self.owner)
                .expect("observe roof target");
            let wait = precompute_avenger_on_roof_wait_position(
                &self.engine.entities(),
                self.engine.script_domains.interactables.doors.as_slice(),
                &self.engine.seq(),
                self.owner,
                target,
                |element| super::ai_view_position_sector(self.engine, element),
                &|sector| self.engine.building_sector_is_authorized(sector),
                &|sector| self.engine.get_sector_lift_type(sector),
            );
            if let Some(wait) = wait {
                self.engine
                    .ai_mut(self.owner, "observe roof route reset")
                    .couldnt_reachpoint = false;
                self.duty_set_state(AiState::Attacking, Substate::AttackingRunToAvengerOnRoof);
                self.duty_go_near(wait, 50, GotoFlags::RUN);
                let target = self
                    .engine
                    .battle_primary(self.owner)
                    .expect("observe roof current target");
                let position = self.engine.live_ai_position(target);
                self.engine
                    .ai_mut(self.owner, "observe roof seek position")
                    .seek_position = position;
                return false;
            }
        }
        true
    }
}
