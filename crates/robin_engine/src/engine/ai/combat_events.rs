//! Combat event statements execute against the current owner and target.
#[cfg(test)]
mod tests;
use super::*;
use crate::ai::{AiState, EmoticonType, GotoFlags, Position, Remark, StimulusType, Substate};
use crate::ai_enemy::{AiMapVec, EnemyAi, SeekFlags, UNDEFINED_DIRECTION};
use crate::sim_rng::SimulationContext;

impl EngineInner {
    fn combat_event_ai(&self, owner: EntityId) -> &EnemyAi {
        self.world
            .entities
            .expect_enemy_ai(owner, format_args!("combat event owner"))
    }
    fn combat_event_ai_mut(&mut self, owner: EntityId) -> &mut EnemyAi {
        self.world
            .entities
            .expect_enemy_ai_mut(owner, format_args!("combat event owner"))
    }
    fn combat_event_primary(&self, owner: EntityId) -> EntityId {
        let target = self
            .combat_event_ai(owner)
            .base
            .primary_target
            .expect("combat event requires primary target");
        self.expect_human_id_for_ai_handle(target.get(), "combat event primary")
    }
    fn combat_event_timer(&mut self, owner: EntityId, delay: u32) {
        let frame = self.control.frame_counter;
        self.combat_event_ai_mut(owner)
            .base
            .launch_timer(delay, frame);
    }
    fn combat_event_state(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        substate: Substate,
        delay: u32,
    ) {
        self.duty_set_state(sim, assets, owner, AiState::Attacking, substate);
        self.combat_event_timer(owner, delay);
    }
    fn combat_event_focus(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) {
        let target = self.combat_event_ai(owner).base.primary_target;
        self.combat_event_ai_mut(owner)
            .base
            .outbox
            .actor
            .set_focus(target);
        self.drain_direct_ai_owner_boundary(sim, owner, assets);
    }
    fn combat_event_face_position(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        position: Position,
    ) {
        let target = crate::ai::ai_position_to_point_3d(
            &self.world.fast_grid,
            self.sight_obstacles(assets),
            position,
        );
        let body = self
            .expect_entity(owner, "combat facing owner")
            .element_data()
            .position();
        let direction = crate::position_interface::vector_to_sector_0_to_15_iso(
            target.x - body.x,
            target.y - body.y,
        );
        self.duty_face_direction(sim, assets, owner, direction as u16);
    }
    fn combat_event_face_primary(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) {
        let target = self.combat_event_primary(owner);
        let position = self.live_ai_position(target);
        let elevation = self
            .expect_entity(target, "combat face elevation")
            .element_data()
            .position()
            .z as i16;
        self.combat_event_face_elevation(sim, assets, owner, position, elevation, false);
    }
    fn combat_event_face_elevation(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        position: Position,
        elevation: i16,
        fast: bool,
    ) {
        let direction = if elevation == -1 {
            let target = crate::ai::ai_position_to_point_3d(
                &self.world.fast_grid,
                self.sight_obstacles(assets),
                position,
            );
            let body = self
                .expect_entity(owner, "combat facing owner")
                .element_data()
                .position();
            crate::position_interface::vector_to_sector_0_to_15_iso(
                target.x - body.x,
                target.y - body.y,
            )
        } else {
            let here = self.live_ai_position(owner);
            let z = self
                .expect_entity(owner, "combat facing elevation")
                .element_data()
                .position()
                .z;
            crate::position_interface::vector_to_sector_0_to_15_iso(
                position.x - here.x,
                position.y - here.y + f32::from(elevation) - z,
            )
        };
        if !fast {
            self.duty_face_direction(sim, assets, owner, direction as u16);
            return;
        }
        let entity = self.expect_entity(owner, "combat fast facing actor");
        if entity.element_data().direction() as u16 == direction as u16
            && matches!(
                entity.actor_data().expect("combat actor").action_state,
                crate::element::ActionState::Waiting | crate::element::ActionState::Bored
            )
        {
            self.combat_event_ai_mut(owner).base.already_turned = true;
        } else {
            let mut order = crate::order::AiOrderIntent::face_direction(direction);
            order.fast_turn = true;
            self.combat_event_ai_mut(owner)
                .base
                .outbox
                .actor
                .orders
                .push(order);
            self.drain_direct_ai_owner_boundary(sim, owner, assets);
        }
    }
    fn combat_event_raise_sword(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) {
        let actor = &mut self.combat_event_ai_mut(owner).base.outbox.actor;
        actor.enter_swordfight = Some(crate::ai::EnterSwordfightRequest::RaiseSword);
        actor.enter_swordfight_jump_line = None;
        self.drain_direct_ai_owner_boundary(sim, owner, assets);
    }
    fn combat_event_command(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        command: crate::element::Command,
    ) {
        self.combat_event_ai_mut(owner)
            .base
            .outbox
            .actor
            .launch_commands
            .push(command);
        self.drain_direct_ai_owner_boundary(sim, owner, assets);
    }
    fn combat_event_stop(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) {
        self.combat_event_ai_mut(owner).base.stop_all();
        self.drain_direct_ai_owner_boundary(sim, owner, assets);
    }
    fn combat_event_say(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        remark: Remark,
    ) {
        self.owner_work_speech(
            sim,
            assets,
            owner,
            crate::ai::AiSpeechAttempt { remark, flags: 0 },
        );
    }
    fn combat_event_visible_primary(&mut self, assets: &LevelAssets, owner: EntityId) -> bool {
        self.combat_event_ai(owner)
            .base
            .primary_target
            .is_some_and(|target| {
                let id =
                    self.expect_human_id_for_ai_handle(target.get(), "combat visibility target");
                self.live_ai_detects_180(assets, owner, id)
            })
    }

    pub(in crate::engine) fn execute_ai_combat_expected_event(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        event: StimulusType,
    ) -> bool {
        use crate::element::Command;
        use StimulusType::*;
        use Substate::*;
        let substate = self.combat_event_ai(owner).base.current_substate;
        match (substate, event) {
            (AttackingReactiontimeTurning, EventDone | EventTimer) => {
                self.duty_set_state(
                    sim,
                    assets,
                    owner,
                    AiState::Attacking,
                    AttackingReactiontime,
                );
                let target = self.combat_event_primary(owner);
                let delay = if self.live_actor_animation(target)
                    == Some(crate::order::OrderType::RunningUpright)
                {
                    crate::parameters_ai::AI_RUNNING_ENEMY_REACTIONTIME as u32
                } else if self.combat_event_distance(owner, target) < 30.0 {
                    1
                } else {
                    crate::parameters_ai::AI_QUICK_ENEMY_REACTIONTIME as u32
                };
                self.combat_event_timer(owner, delay);
            }
            (AttackingReactiontime, EventTimer) => {
                if self
                    .expect_entity(owner, "reaction posture")
                    .element_data()
                    .posture()
                    == crate::element::Posture::LeaningOut
                    && self.combat_event_ai(owner).is_archer()
                {
                    self.reinitialize_live_ai_enemies(owner);
                    self.duty_set_state(
                        sim,
                        assets,
                        owner,
                        AiState::Attacking,
                        AttackingReactiontimeBending,
                    );
                    self.combat_event_command(sim, assets, owner, Command::EquipBowDown);
                } else {
                    self.execute_battle_decisions(sim, assets, owner);
                }
            }
            (AttackingReactiontimeRunning, EventTimer | EventReachPoint) => {
                self.combat_event_stop(sim, assets, owner);
                self.execute_battle_decisions(sim, assets, owner);
            }
            (AttackingReactiontimeBending, EventDone) => {
                self.execute_battle_decisions(sim, assets, owner)
            }
            (
                AttackingRunningToEnemy | AttackingWalkingToEnemy | AttackingChargingEnemy,
                EventReachPoint | EventTimer,
            ) => {
                self.execute_ai_reconsider_enemy_approach(
                    sim,
                    assets,
                    owner,
                    event == EventReachPoint,
                );
            }
            (AttackingSwordfight, EventTimer | EventDone | EventReachPoint) => {
                if !self.combat_event_ai(owner).pending_special_strike {
                    self.combat_event_ai_mut(owner)
                        .base
                        .set_emoticon(EmoticonType::None);
                    self.drain_direct_ai_owner_boundary(sim, owner, assets);
                    self.execute_reconsider_swordfight(sim, assets, owner, false);
                    self.combat_event_ai_mut(owner)
                        .swordfight_insult_after_reconsider();
                    self.drain_direct_ai_owner_boundary(sim, owner, assets);
                }
            }
            (AttackingSwordfightSpecialStrike, EventDone | EventTimer) => {
                self.combat_event_ai_mut(owner).pending_special_strike = false;
                self.combat_event_state(sim, assets, owner, AttackingSwordfight, 20);
                let frame = self.control.frame_counter;
                self.combat_event_ai_mut(owner).next_sword_strike_frame = frame + 20;
            }
            (AttackingSwordfightParade, EventTimer) => {
                if self
                    .expect_entity(owner, "parade action")
                    .actor_data()
                    .expect("parade actor")
                    .action_state
                    == crate::element::ActionState::ParryingSword
                {
                    self.combat_event_command(sim, assets, owner, Command::StopParrySword);
                }
                self.combat_event_state(sim, assets, owner, AttackingSwordfight, 20);
            }
            (AttackingSwordfightStepBack, EventReachPoint) => {
                self.combat_event_state(sim, assets, owner, AttackingSwordfight, 20)
            }
            (AttackingApproachingNewEnemy, EventReachPoint) => {
                self.combat_event_approached_new_enemy(sim, assets, owner)
            }
            (AttackingMovingAroundOldEnemy, EventReachPoint) => {
                self.combat_event_state(sim, assets, owner, AttackingSwordfight, 20);
                self.execute_reconsider_swordfight(sim, assets, owner, false);
            }
            (AttackingQuittingSwordfight, EventTimer) => {
                if self
                    .expect_entity(owner, "quitting action")
                    .actor_data()
                    .expect("quitting actor")
                    .action_state
                    .is_sword()
                {
                    if !self
                        .expect_entity(owner, "quitting opponents")
                        .human_data()
                        .expect("quitting human")
                        .opponents
                        .is_empty()
                    {
                        self.combat_event_ai_mut(owner)
                            .base
                            .outbox
                            .actor
                            .retry_quit_swordfight = true;
                        self.drain_direct_ai_owner_boundary(sim, owner, assets);
                    }
                    self.combat_event_timer(owner, 3);
                } else {
                    self.execute_ai_get_battle_overview(sim, assets, owner, 0);
                }
            }
            (AttackingOverviewLookLeft, EventDone) => {
                self.duty_set_state(
                    sim,
                    assets,
                    owner,
                    AiState::Attacking,
                    AttackingOverviewLookRight,
                );
                self.combat_event_ai_mut(owner)
                    .base
                    .outbox
                    .actor
                    .look_sidewards = Some(crate::ai::LookDirection::Right);
                self.drain_direct_ai_owner_boundary(sim, owner, assets);
            }
            (AttackingOverviewLookRight, EventDone) => self.combat_event_timer(owner, 10),
            (
                AttackingOverviewLookRight
                | AttackingLastReserve
                | AttackingReserveOverview
                | AttackingOfficerGivingOrdersWaiting,
                EventTimer,
            ) => {
                if substate == AttackingOfficerGivingOrdersWaiting {
                    self.reinitialize_live_ai_enemies(owner);
                }
                if substate == AttackingOfficerGivingOrdersWaiting
                    && self.combat_event_ai(owner).list_them.is_empty()
                {
                    let center = self.combat_event_ai(owner).base.seek_position;
                    self.execute_ai_seek_area(
                        sim,
                        assets,
                        owner,
                        center,
                        crate::parameters_ai::AI_LOST_ENEMY_SEEK_RADIUS as u16,
                        SeekFlags::LOCATION_FIRST,
                        UNDEFINED_DIRECTION,
                    );
                } else {
                    self.execute_battle_decisions(sim, assets, owner);
                }
            }
            (AttackingReserve, EventTimer | CallCoordinate) => {
                self.combat_event_reserve(sim, assets, owner, event == EventTimer)
            }
            (AttackingObserve, EventTimer) | (AttackingObserveAndMove, EventReachPoint) => {
                self.execute_reconsider_swordfight_observation(sim, assets, owner)
            }
            (AttackingTowerGuardObserve | AttackingDoorFightWaiting, EventTimer) => {
                self.execute_ai_get_battle_overview(sim, assets, owner, 0)
            }
            (AttackingTowerGuardAlert, EventDone) => {
                let center = self.combat_event_ai(owner).base.seek_position;
                self.execute_ai_tower_guard_alert(sim, assets, owner, center);
            }
            (AttackingDoorFightDelay, EventTimer) => {
                self.duty_set_state(
                    sim,
                    assets,
                    owner,
                    AiState::Attacking,
                    AttackingDoorFightLeaving,
                );
                let destination = self.combat_event_ai(owner).base.seek_position;
                self.duty_go_to(sim, assets, owner, destination, GotoFlags::RUN);
            }
            (AttackingDoorFightLeaving, EventReachPoint) => {
                self.duty_set_state(
                    sim,
                    assets,
                    owner,
                    AiState::Attacking,
                    AttackingDoorFightTurning,
                );
                let direction = self.combat_event_ai(owner).gather_direction;
                self.duty_face_direction(sim, assets, owner, direction);
            }
            (AttackingDoorFightTurning, EventDone) => {
                if self.combat_event_ai(owner).base.primary_target.is_none() {
                    self.combat_event_state(sim, assets, owner, AttackingDoorFightWaiting, 150);
                } else {
                    self.execute_ai_begin_swordfight(sim, assets, owner);
                }
            }
            (AttackingReturnToOtherPcAfterMenacing, EventDone) => {
                self.execute_ai_begin_swordfight(sim, assets, owner)
            }
            (AttackingRunningToLadder, EventReachPoint) => {
                self.combat_event_face_primary(sim, assets, owner);
                self.combat_event_focus(sim, assets, owner);
                self.combat_event_state(sim, assets, owner, AttackingWaitingAtLadder, 1);
            }
            (AttackingRunningToLadder, EventTimer) => {
                self.execute_ai_reconsider_enemy_approach(sim, assets, owner, false)
            }
            (AttackingWaitingAtLadder, EventTimer) => {
                let target = self.combat_event_primary(owner);
                let sector = self
                    .expect_entity(target, "ladder target sector")
                    .element_data()
                    .sector()
                    .expect("ladder target requires sector");
                if self
                    .world
                    .fast_grid
                    .sector_type_for_handle(sector)
                    .is_lift()
                {
                    self.combat_event_face_primary(sim, assets, owner);
                    self.combat_event_focus(sim, assets, owner);
                    self.combat_event_timer(owner, 20);
                } else {
                    self.execute_ai_reconsider_enemy_approach(sim, assets, owner, false);
                }
            }
            (AttackingRunToAvengerOnRoof, EventReachPoint) => {
                let position = self.combat_event_ai(owner).base.seek_position;
                self.combat_event_face_position(sim, assets, owner, position);
                self.combat_event_state(sim, assets, owner, AttackingWaitForAvengerOnRoof, 100);
            }
            (AttackingWaitForAvengerOnRoof, EventTimer) => {
                if self.combat_event_visible_primary(assets, owner) {
                    let position = self.live_ai_position(self.combat_event_primary(owner));
                    self.combat_event_face_position(sim, assets, owner, position);
                    self.combat_event_timer(owner, 30);
                } else {
                    let center = self.live_ai_position(owner);
                    self.execute_ai_seek_area(
                        sim,
                        assets,
                        owner,
                        center,
                        crate::parameters_ai::AI_LOST_ENEMY_SEEK_RADIUS as u16,
                        SeekFlags::empty(),
                        UNDEFINED_DIRECTION,
                    );
                }
            }
            (AttackingOfficerGivingOrders, EventDone) => {
                self.duty_set_state(
                    sim,
                    assets,
                    owner,
                    AiState::Attacking,
                    AttackingOfficerGivingOrdersWaiting,
                );
                self.combat_event_ai_mut(owner).base.friends_are_alerted = true;
                self.combat_event_timer(owner, 20);
            }
            (
                AttackingArcherWaitOnArcheryPath
                | AttackingArcherWaitOnArcheryPathBending
                | AttackingArcherWaitOnBendPoint,
                EventTimer,
            ) => self.execute_ai_return_to_duty(sim, assets, owner, crate::ai::DutyFlags::empty()),
            (AttackingDummyBehaviour, EventDone) => {
                let direction = (self
                    .expect_entity(owner, "dummy direction")
                    .element_data()
                    .direction() as u16
                    + 3)
                    & 15;
                self.duty_face_direction(sim, assets, owner, direction);
            }
            (AttackingApproachToObserve, EventTimer) => {
                let target = self.live_ai_position(self.combat_event_primary(owner));
                let here = self.live_ai_position(owner);
                let direction = crate::position_interface::vector_to_sector_0_to_15_iso(
                    target.x - here.x,
                    target.y - here.y,
                );
                self.combat_event_ai_mut(owner)
                    .base
                    .set_direction_goal(direction as u16);
                self.drain_direct_ai_owner_boundary(sim, owner, assets);
                self.combat_event_stop(sim, assets, owner);
                self.combat_event_raise_sword(sim, assets, owner);
                self.duty_set_state(sim, assets, owner, AiState::Attacking, AttackingObserve);
                self.combat_event_ai_mut(owner)
                    .base
                    .set_emoticon(EmoticonType::None);
                self.drain_direct_ai_owner_boundary(sim, owner, assets);
                self.combat_event_timer(owner, 50);
            }
            (AttackingTooProudToAttack, EventTimer) => {
                self.reinitialize_live_ai_enemies(owner);
                self.combat_event_ai_mut(owner)
                    .base
                    .set_emoticon(EmoticonType::None);
                self.drain_direct_ai_owner_boundary(sim, owner, assets);
                self.duty_set_state(
                    sim,
                    assets,
                    owner,
                    AiState::Attacking,
                    AttackingTooProudToAttackOverview,
                );
                if crate::sim_rng::u32(sim, crate::sim_rng::RngSite::TooProudLook, 0..16) == 0 {
                    self.combat_event_ai_mut(owner)
                        .base
                        .outbox
                        .actor
                        .look_sidewards = Some(crate::ai::LookDirection::LeftRight);
                    self.drain_direct_ai_owner_boundary(sim, owner, assets);
                } else {
                    self.combat_event_timer(owner, 20);
                }
            }
            (AttackingTooProudToAttackOverview, EventDone) => {
                self.combat_event_focus(sim, assets, owner);
                self.combat_event_timer(owner, 5);
            }
            (AttackingTooProudToAttackOverview, EventTimer) => {
                self.execute_battle_decisions(sim, assets, owner);
                let ai = self.combat_event_ai(owner);
                if ai.base.current_substate.is_any_swordfight() {
                    let remark = if ai.is_vip {
                        Remark::VipProudFinallyFight
                    } else {
                        Remark::ProudFinallyFight
                    };
                    self.combat_event_say(sim, assets, owner, remark);
                }
            }
            (AttackingTooProudToAttackRetire, EventReachPoint) => {
                self.duty_set_state(
                    sim,
                    assets,
                    owner,
                    AiState::Attacking,
                    AttackingTooProudToAttackRetireTurn,
                );
                let position = self.combat_event_ai(owner).base.seek_position;
                self.combat_event_face_position(sim, assets, owner, position);
            }
            (AttackingTooProudToAttackRetireTurn, EventDone)
            | (AttackingTooProudToAttackApproach, EventReachPoint) => {
                if self.combat_event_visible_primary(assets, owner) {
                    self.execute_battle_decisions(sim, assets, owner);
                } else {
                    self.execute_ai_get_battle_overview(sim, assets, owner, 0);
                }
            }
            (AttackingArcherRetireFromCombat, EventReachPoint) => {
                self.duty_set_state(
                    sim,
                    assets,
                    owner,
                    AiState::Attacking,
                    AttackingArcherRetireFromCombatTurn,
                );
                if self.combat_event_ai(owner).base.primary_target.is_some() {
                    let target = self.combat_event_primary(owner);
                    let position = self.live_ai_position(target);
                    let elevation = self
                        .expect_entity(target, "retiring face target")
                        .element_data()
                        .position()
                        .z as i16;
                    self.combat_event_face_elevation(sim, assets, owner, position, elevation, true);
                } else {
                    let position = self.combat_event_ai(owner).base.seek_position;
                    self.combat_event_face_elevation(sim, assets, owner, position, 1, false);
                }
            }
            (AttackingArcherRetireFromCombatTurn, EventDone) => {
                if self.combat_event_visible_primary(assets, owner) {
                    let target = self
                        .combat_event_ai(owner)
                        .base
                        .primary_target
                        .expect("visible target")
                        .get();
                    if !self.combat_event_ai(owner).list_them.contains(&target) {
                        self.combat_event_ai_mut(owner).list_them.push(target);
                    }
                    self.execute_battle_decisions(sim, assets, owner);
                } else {
                    self.execute_ai_get_battle_overview(sim, assets, owner, 0);
                }
            }
            (AttackingApproachingSleepingEnemy, EventReachPoint) => {
                self.combat_event_face_primary(sim, assets, owner)
            }
            (AttackingApproachingSleepingEnemy, EventDone) => {
                self.combat_event_sleeping_enemy(sim, assets, owner)
            }
            (AttackingKillingSleepingEnemy, EventDone) => {
                self.combat_event_say(sim, assets, owner, Remark::KilledAdversary);
                self.execute_ai_get_battle_overview(sim, assets, owner, 0);
            }
            (AttackingRiderChargingApproaching, EventGaloppLoopEnd) => {
                if !self.execute_ai_maybe_make_rider_attack(sim, assets, owner) {
                    self.duty_set_state(
                        sim,
                        assets,
                        owner,
                        AiState::Attacking,
                        AttackingRunningToEnemy,
                    );
                    self.execute_ai_reconsider_enemy_approach(sim, assets, owner, true);
                }
            }
            (AttackingRiderChargingApproaching, EventReachPoint) => {
                self.execute_ai_get_battle_overview(sim, assets, owner, 0)
            }
            (AttackingRiderChargingPassing, EventReachPoint) => {
                self.duty_set_state(
                    sim,
                    assets,
                    owner,
                    AiState::Attacking,
                    AttackingRiderChargingGettingDistance,
                );
                if let Some(goal) = self.combat_event_rider_retreat_goal(owner) {
                    self.duty_go_to(sim, assets, owner, goal, GotoFlags::RUN);
                } else {
                    self.dispatch_think_with_drain(
                        sim,
                        owner,
                        &crate::ai::Stimulus::new(EventReachPoint),
                        Option::None,
                        assets,
                    );
                }
            }
            (AttackingRiderChargingGettingDistance, EventReachPoint) => {
                let position = self.combat_event_ai(owner).base.seek_position;
                self.combat_event_face_position(sim, assets, owner, position);
                self.duty_set_state(
                    sim,
                    assets,
                    owner,
                    AiState::Attacking,
                    AttackingRiderChargingReturning,
                );
            }
            (AttackingRiderChargingReturning, EventDone) => {
                self.reinitialize_live_ai_enemies(owner);
                if self.combat_event_ai(owner).list_them.is_empty() {
                    self.duty_set_state(
                        sim,
                        assets,
                        owner,
                        AiState::Attacking,
                        AttackingRiderChargingApproachingBlindly,
                    );
                    let destination = self.combat_event_ai(owner).base.seek_position;
                    self.duty_go_to(sim, assets, owner, destination, GotoFlags::RUN);
                } else {
                    self.execute_battle_decisions(sim, assets, owner);
                }
            }
            (AttackingRiderChargingApproachingBlindly, EventReachPoint) => {
                self.duty_set_state(sim, assets, owner, AiState::Wondering, WonderingLooking1);
                self.combat_event_timer(owner, 30);
            }
            _ => return false,
        }
        true
    }

    fn combat_event_rider_retreat_goal(&self, owner: EntityId) -> Option<Position> {
        let position = self.live_ai_position(owner);
        let entity = self.expect_entity(owner, "rider retreat owner");
        let direction = entity.element_data().direction();
        let move_box = entity.position_iface().get_move_box();
        let mut distance = 500.0;
        while distance > 10.0 {
            for relative in [0i32, 1, -1] {
                // Preserve the authored signed remainder and sector mask.
                let sector = ((i32::from(direction) + relative) % 15) as u16 & 15;
                let vector = crate::coordinates::MapVec::from_sector_iso(sector);
                let goal = Position {
                    x: position.x + vector.x * distance,
                    y: position.y + vector.y * distance,
                    ..position
                };
                if self.world.fast_grid.is_straight_movement_authorized(
                    crate::coordinates::MapPoint::new(position.x, position.y),
                    crate::coordinates::MapPoint::new(goal.x, goal.y),
                    position.level,
                    move_box,
                ) {
                    return Some(goal);
                }
            }
            distance -= 10.0;
        }
        None
    }
    fn combat_event_sleeping_enemy(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) {
        let target = self.combat_event_primary(owner);
        let entity = self.expect_entity(target, "sleeping target");
        if !entity.is_unconscious() {
            self.execute_ai_get_battle_overview(sim, assets, owner, 0);
            return;
        }
        if let Entity::Pc(pc) = entity {
            let index = pc
                .pc
                .campaign_description_index
                .expect("sleeping PC campaign character") as usize;
            if self.mission_domain.campaign.characters[index]
                .status
                .in_coma
            {
                if pc.pc.guard.is_some() {
                    self.execute_ai_return_to_duty(
                        sim,
                        assets,
                        owner,
                        crate::ai::DutyFlags::empty(),
                    );
                    return;
                }
                // The chosen patient survives the state and command callbacks.
                let EntityId::Pc(patient) = target else {
                    unreachable!()
                };
                self.combat_event_stop(sim, assets, owner);
                self.duty_set_state(
                    sim,
                    assets,
                    owner,
                    AiState::Menacing,
                    Substate::MenacingPcInComa,
                );
                if self.combat_event_ai(owner).is_vip {
                    self.combat_event_stop(sim, assets, owner);
                    self.combat_event_raise_sword(sim, assets, owner);
                } else {
                    self.combat_event_say(sim, assets, owner, Remark::MenacesPcInComa);
                    self.combat_event_command(
                        sim,
                        assets,
                        owner,
                        crate::element::Command::StartMenace,
                    );
                    self.combat_event_ai_mut(owner)
                        .set_guarded_pc(Some(patient));
                    self.drain_direct_ai_owner_boundary(sim, owner, assets);
                }
                self.combat_event_timer(owner, 20);
                return;
            }
        }
        if self.combat_event_distance(owner, target) > 40.0 {
            let position = self.live_ai_position(target);
            self.duty_go_near(sim, assets, owner, position, 20, GotoFlags::RUN);
        } else {
            self.duty_set_state(
                sim,
                assets,
                owner,
                AiState::Attacking,
                Substate::AttackingKillingSleepingEnemy,
            );
            self.combat_event_stop(sim, assets, owner);
            let target = self.combat_event_primary(owner);
            let mut sequence = crate::sequence::Sequence::new();
            sequence.append_element(crate::sequence::SequenceElement::new_interaction(
                1,
                crate::element::Command::SwordstrikeDown,
                Some(owner),
                Some(target),
            ));
            self.combat_event_ai_mut(owner)
                .base
                .outbox
                .actor
                .launch_sequences
                .push(sequence);
            self.drain_direct_ai_owner_boundary(sim, owner, assets);
        }
    }
    fn combat_event_distance(&self, owner: EntityId, target: EntityId) -> f32 {
        self.combat_event_square_distance(owner, target).sqrt()
    }
    fn combat_event_square_distance(&self, owner: EntityId, target: EntityId) -> f32 {
        let a = self
            .expect_entity(owner, "combat distance owner")
            .element_data()
            .position();
        let b = self
            .expect_entity(target, "combat distance target")
            .element_data()
            .position();
        let dx = b.x - a.x;
        let dy = (b.y - a.y) * crate::position_interface::INVERSE_ASPECT_RATIO;
        let dz = b.z - a.z;
        dx * dx + dy * dy + dz * dz
    }
    fn combat_event_approached_new_enemy(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) {
        let target = self.combat_event_primary(owner);
        assert!(self.sleeping_enemy_attack_allowed(owner, target));
        let weapon = self.combat_event_ai(owner).hth_weapon_id;
        let range = assets
            .profile_manager
            .get_hth_weapon(weapon)
            .expect("combat sword profile")
            .distance[crate::weapons::WeaponDistance::Default as usize];
        let margin = u32::from(range) + 10;
        let close =
            self.combat_event_square_distance(owner, target) < margin.wrapping_mul(margin) as f32;
        if !close {
            let position = self.live_ai_position(target);
            self.duty_go_near(
                sim,
                assets,
                owner,
                position,
                i32::from(range),
                GotoFlags::RUN,
            );
            if !self.combat_event_ai(owner).base.already_on_point {
                return;
            }
            self.combat_event_ai_mut(owner).base.already_on_point = false;
        }
        self.combat_event_state(sim, assets, owner, Substate::AttackingSwordfight, 20);
        let target = self.combat_event_ai(owner).base.primary_target;
        self.combat_event_ai_mut(owner)
            .base
            .outbox
            .actor
            .set_principal = target;
        self.drain_direct_ai_owner_boundary(sim, owner, assets);
    }
    fn combat_event_reserve(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        coordinate: bool,
    ) {
        if coordinate {
            let count = self.combat_event_ai(owner).base.list_us.len();
            for index in 0..count {
                let handle = self.combat_event_ai(owner).base.list_us[index];
                let friend = self.expect_human_id_for_ai_handle(handle, "reserve friend");
                if friend != owner
                    && matches!(
                        self.expect_entity(friend, "reserve friend kind"),
                        Entity::Soldier(_)
                    )
                    && self
                        .world
                        .entities
                        .expect_ai_controller(friend, format_args!("reserve friend state"))
                        .current_substate
                        == Substate::AttackingReserve
                {
                    self.dispatch_think_with_drain(
                        sim,
                        friend,
                        &crate::ai::Stimulus::new(StimulusType::CallCoordinate),
                        Some(owner),
                        assets,
                    );
                }
            }
        }
        self.reinitialize_live_ai_enemies(owner);
        self.combat_event_ai_mut(owner)
            .base
            .set_emoticon(EmoticonType::None);
        self.drain_direct_ai_owner_boundary(sim, owner, assets);
        self.combat_event_state(sim, assets, owner, Substate::AttackingReserveOverview, 20);
    }
}
