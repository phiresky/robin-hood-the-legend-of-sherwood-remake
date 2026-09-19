//! Combat event statements execute against the current owner and target.
#[cfg(test)]
mod tests;
use super::*;
use crate::ai::{AiState, EmoticonType, GotoFlags, Position, Remark, StimulusType, Substate};
use crate::ai_enemy::{AiMapVec, EnemyAi, SeekFlags, UNDEFINED_DIRECTION};
use crate::engine::TickCtx;

impl EngineInner {
    pub(in crate::engine) fn execute_ai_combat_unexpected_event(
        &mut self,
        tcx: TickCtx<'_>,
        owner: EntityId,
        stimulus: &crate::ai::Stimulus,
    ) -> bool {
        if stimulus.stimulus_type != StimulusType::EventEnemyNear {
            return false;
        }
        let crate::ai::StimulusInfo::Human(target) = stimulus.info else {
            tracing::warn!(?owner, info = ?stimulus.info, "Enemy-near event requires a human target");
            return true;
        };
        if matches!(
            self.combat_event_ai(owner).base.current_substate,
            Substate::AttackingReactiontimeTurning
                | Substate::AttackingReactiontime
                | Substate::AttackingApproachToObserve
                | Substate::AttackingObserve
        ) {
            self.combat_event_ai_mut(owner).base.primary_target = Some(target);
            self.execute_ai_begin_swordfight(tcx, owner);
        }
        true
    }

    fn combat_event_ai(&self, owner: EntityId) -> &EnemyAi {
        self.enemy_ai(owner, "combat event owner")
    }
    fn combat_event_ai_mut(&mut self, owner: EntityId) -> &mut EnemyAi {
        self.enemy_ai_mut(owner, "combat event owner")
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
    fn combat_event_focus(&mut self, owner: EntityId) {
        let target = self.combat_event_ai(owner).base.primary_target;
        self.execute_ai_focus(owner, target);
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
}

impl AiOwnerCtx<'_> {
    fn combat_event_state(&mut self, substate: Substate, delay: u32) {
        self.duty_set_state(AiState::Attacking, substate);
        self.engine.combat_event_timer(self.owner, delay);
    }

    pub(super) fn duty_face_position_ground(&mut self, position: Position) {
        let target = crate::ai::ai_position_to_point_3d(
            &self.engine.world.fast_grid,
            self.engine.sight_obstacles(self.tcx.assets),
            position,
        );
        let body = self
            .engine
            .expect_entity(self.owner, "combat facing owner")
            .element_data()
            .position();
        let direction = crate::position_interface::vector_to_sector_0_to_15_iso(
            target.x - body.x,
            target.y - body.y,
        );
        self.duty_face_direction(direction as u16);
    }

    fn combat_event_face_primary(&mut self) {
        let target = self.engine.combat_event_primary(self.owner);
        let position = self.engine.live_ai_position(target);
        let elevation = self
            .engine
            .expect_entity(target, "combat face elevation")
            .element_data()
            .position()
            .z as i16;
        self.duty_face_position_signed_elevation(position, elevation, false);
    }

    pub(super) fn duty_face_position_signed_elevation(
        &mut self,
        position: Position,
        elevation: i16,
        fast: bool,
    ) {
        let direction = if elevation == -1 {
            let target = crate::ai::ai_position_to_point_3d(
                &self.engine.world.fast_grid,
                self.engine.sight_obstacles(self.tcx.assets),
                position,
            );
            let body = self
                .engine
                .expect_entity(self.owner, "combat facing owner")
                .element_data()
                .position();
            crate::position_interface::vector_to_sector_0_to_15_iso(
                target.x - body.x,
                target.y - body.y,
            )
        } else {
            let here = self.engine.live_ai_position(self.owner);
            let z = self
                .engine
                .expect_entity(self.owner, "combat facing elevation")
                .element_data()
                .position()
                .z;
            crate::position_interface::vector_to_sector_0_to_15_iso(
                position.x - here.x,
                position.y - here.y + f32::from(elevation) - z,
            )
        };
        if !fast {
            self.duty_face_direction(direction as u16);
            return;
        }
        let entity = self
            .engine
            .expect_entity(self.owner, "combat fast facing actor");
        if entity.element_data().direction() as u16 == direction as u16
            && matches!(
                entity.actor_data().expect("combat actor").action_state,
                crate::element::ActionState::Waiting | crate::element::ActionState::Bored
            )
        {
            self.engine
                .combat_event_ai_mut(self.owner)
                .base
                .already_turned = true;
        } else {
            self.engine
                .launch_live_ai_turn(self.tcx, self.owner, direction, true);
        }
    }

    fn combat_event_raise_sword(&mut self) {
        self.launch_ai_raise_sword();
    }

    fn combat_event_command(&mut self, command: crate::element::Command) {
        self.engine.launch_element(
            self.tcx,
            crate::sequence::SequenceElement::new(1, command, Some(self.owner)),
        );
    }

    fn combat_event_stop(&mut self) {
        self.stop_ai_owner();
    }

    fn combat_event_say(&mut self, remark: Remark) {
        self.execute_ai_speech(crate::ai::AiSpeechAttempt { remark, flags: 0 });
    }

    pub(in crate::engine) fn execute_ai_combat_expected_event(
        &mut self,
        event: StimulusType,
    ) -> bool {
        use crate::element::Command;
        use StimulusType::*;
        use Substate::*;
        let substate = self
            .engine
            .combat_event_ai(self.owner)
            .base
            .current_substate;
        match (substate, event) {
            (AttackingReactiontimeTurning, EventDone | EventTimer) => {
                self.duty_set_state(AiState::Attacking, AttackingReactiontime);
                let target = self.engine.combat_event_primary(self.owner);
                let delay = if self.engine.live_actor_animation(target)
                    == Some(crate::order::OrderType::RunningUpright)
                {
                    crate::parameters_ai::AI_RUNNING_ENEMY_REACTIONTIME as u32
                } else if self.engine.combat_event_distance(self.owner, target) < 30.0 {
                    1
                } else {
                    crate::parameters_ai::AI_QUICK_ENEMY_REACTIONTIME as u32
                };
                self.engine.combat_event_timer(self.owner, delay);
            }
            (AttackingReactiontime, EventTimer) => {
                if self
                    .engine
                    .expect_entity(self.owner, "reaction posture")
                    .element_data()
                    .posture()
                    == crate::element::Posture::LeaningOut
                    && self.engine.combat_event_ai(self.owner).is_archer()
                {
                    self.engine.reinitialize_live_ai_enemies(self.owner);
                    self.duty_set_state(AiState::Attacking, AttackingReactiontimeBending);
                    self.combat_event_command(Command::EquipBowDown);
                } else {
                    self.execute_battle_decisions();
                }
            }
            (AttackingReactiontimeRunning, EventTimer | EventReachPoint) => {
                self.combat_event_stop();
                self.execute_battle_decisions();
            }
            (AttackingReactiontimeBending, EventDone) => self.execute_battle_decisions(),
            (
                AttackingRunningToEnemy | AttackingWalkingToEnemy | AttackingChargingEnemy,
                EventReachPoint | EventTimer,
            ) => {
                self.execute_ai_reconsider_enemy_approach(event == EventReachPoint);
            }
            (AttackingSwordfight, EventTimer | EventDone | EventReachPoint) => {
                self.engine
                    .combat_event_ai_mut(self.owner)
                    .base
                    .set_emoticon(EmoticonType::None);

                self.execute_reconsider_swordfight(false);
                self.engine
                    .combat_insult_after_reconsider(self.tcx, self.owner);
            }
            (AttackingSwordfightSpecialStrike, EventDone | EventTimer) => {
                self.combat_event_state(AttackingSwordfight, 20);
            }
            (AttackingSwordfightParade, EventTimer) => {
                if self
                    .engine
                    .expect_entity(self.owner, "parade action")
                    .actor_data()
                    .expect("parade actor")
                    .action_state
                    == crate::element::ActionState::ParryingSword
                {
                    self.combat_event_command(Command::StopParrySword);
                }
                self.combat_event_state(AttackingSwordfight, 20);
            }
            (AttackingSwordfightStepBack, EventReachPoint) => {
                self.combat_event_state(AttackingSwordfight, 20)
            }
            (AttackingApproachingNewEnemy, EventReachPoint) => {
                self.combat_event_approached_new_enemy()
            }
            (AttackingMovingAroundOldEnemy, EventReachPoint) => {
                self.combat_event_state(AttackingSwordfight, 20);
                self.execute_reconsider_swordfight(false);
            }
            (AttackingQuittingSwordfight, EventTimer) => {
                if self
                    .engine
                    .expect_entity(self.owner, "quitting action")
                    .actor_data()
                    .expect("quitting actor")
                    .action_state
                    .is_sword()
                {
                    if !self
                        .engine
                        .expect_entity(self.owner, "quitting opponents")
                        .human_data()
                        .expect("quitting human")
                        .opponents
                        .is_empty()
                    {
                        let already_quitting = self
                            .engine
                            .current_sequence_element_for_actor(self.owner)
                            .and_then(|(sequence, index)| {
                                self.engine.seq().get_element(sequence, index)
                            })
                            .is_some_and(|element| {
                                element.command == crate::element::Command::QuitSwordfight
                            });
                        if !already_quitting {
                            self.execute_ai_end_swordfight();
                        }
                    }
                    self.engine.combat_event_timer(self.owner, 3);
                } else {
                    self.execute_ai_get_battle_overview(0);
                }
            }
            (AttackingOverviewLookLeft, EventDone) => {
                self.duty_set_state(AiState::Attacking, AttackingOverviewLookRight);
                self.execute_ai_look_sidewards(crate::ai::LookDirection::Right);
            }
            (AttackingOverviewLookRight, EventDone) => {
                self.engine.combat_event_timer(self.owner, 10)
            }
            (
                AttackingOverviewLookRight
                | AttackingLastReserve
                | AttackingReserveOverview
                | AttackingOfficerGivingOrdersWaiting,
                EventTimer,
            ) => {
                if substate == AttackingOfficerGivingOrdersWaiting {
                    self.engine.reinitialize_live_ai_enemies(self.owner);
                }
                if substate == AttackingOfficerGivingOrdersWaiting
                    && self.engine.combat_event_ai(self.owner).list_them.is_empty()
                {
                    let center = self.engine.combat_event_ai(self.owner).base.seek_position;
                    self.execute_ai_seek_area(
                        center,
                        crate::parameters_ai::AI_LOST_ENEMY_SEEK_RADIUS as u16,
                        SeekFlags::LOCATION_FIRST,
                        UNDEFINED_DIRECTION,
                    );
                } else {
                    self.execute_battle_decisions();
                }
            }
            (AttackingReserve, EventTimer | CallCoordinate) => {
                self.combat_event_reserve(event == EventTimer)
            }
            (AttackingObserve, EventTimer) | (AttackingObserveAndMove, EventReachPoint) => {
                self.execute_reconsider_swordfight_observation()
            }
            (AttackingTowerGuardObserve | AttackingDoorFightWaiting, EventTimer) => {
                self.execute_ai_get_battle_overview(0)
            }
            (AttackingTowerGuardAlert, EventDone) => {
                let center = self.engine.combat_event_ai(self.owner).base.seek_position;
                self.execute_ai_tower_guard_alert(center);
            }
            (AttackingDoorFightDelay, EventTimer) => {
                self.duty_set_state(AiState::Attacking, AttackingDoorFightLeaving);
                let destination = self.engine.combat_event_ai(self.owner).base.seek_position;
                self.duty_go_to(destination, GotoFlags::RUN);
            }
            (AttackingDoorFightLeaving, EventReachPoint) => {
                self.duty_set_state(AiState::Attacking, AttackingDoorFightTurning);
                let direction = self.engine.combat_event_ai(self.owner).gather_direction;
                self.duty_face_direction(direction);
            }
            (AttackingDoorFightTurning, EventDone) => {
                if self
                    .engine
                    .combat_event_ai(self.owner)
                    .base
                    .primary_target
                    .is_none()
                {
                    self.combat_event_state(AttackingDoorFightWaiting, 150);
                } else {
                    self.execute_ai_begin_swordfight();
                }
            }
            (AttackingReturnToOtherPcAfterMenacing, EventDone) => {
                self.execute_ai_begin_swordfight()
            }
            (AttackingRunningToLadder, EventReachPoint) => {
                self.combat_event_face_primary();
                self.engine.combat_event_focus(self.owner);
                self.combat_event_state(AttackingWaitingAtLadder, 1);
            }
            (AttackingRunningToLadder, EventTimer) => {
                self.execute_ai_reconsider_enemy_approach(false)
            }
            (AttackingWaitingAtLadder, EventTimer) => {
                let target = self.engine.combat_event_primary(self.owner);
                let sector = self
                    .engine
                    .expect_entity(target, "ladder target sector")
                    .element_data()
                    .sector()
                    .expect("ladder target requires sector");
                if self
                    .engine
                    .world
                    .fast_grid
                    .sector_type_for_handle(sector)
                    .is_lift()
                {
                    self.combat_event_face_primary();
                    self.engine.combat_event_focus(self.owner);
                    self.engine.combat_event_timer(self.owner, 20);
                } else {
                    self.execute_ai_reconsider_enemy_approach(false);
                }
            }
            (AttackingRunToAvengerOnRoof, EventReachPoint) => {
                let position = self.engine.combat_event_ai(self.owner).base.seek_position;
                self.duty_face_position_ground(position);
                self.combat_event_state(AttackingWaitForAvengerOnRoof, 100);
            }
            (AttackingWaitForAvengerOnRoof, EventTimer) => {
                if self
                    .engine
                    .combat_event_visible_primary(self.tcx.assets, self.owner)
                {
                    let position = self
                        .engine
                        .live_ai_position(self.engine.combat_event_primary(self.owner));
                    self.duty_face_position_ground(position);
                    self.engine.combat_event_timer(self.owner, 30);
                } else {
                    let center = self.engine.live_ai_position(self.owner);
                    self.execute_ai_seek_area(
                        center,
                        crate::parameters_ai::AI_LOST_ENEMY_SEEK_RADIUS as u16,
                        SeekFlags::empty(),
                        UNDEFINED_DIRECTION,
                    );
                }
            }
            (AttackingOfficerGivingOrders, EventDone) => {
                self.duty_set_state(AiState::Attacking, AttackingOfficerGivingOrdersWaiting);
                self.engine
                    .combat_event_ai_mut(self.owner)
                    .base
                    .friends_are_alerted = true;
                self.engine.combat_event_timer(self.owner, 20);
            }
            (
                AttackingArcherWaitOnArcheryPath
                | AttackingArcherWaitOnArcheryPathBending
                | AttackingArcherWaitOnBendPoint,
                EventTimer,
            ) => self.execute_ai_return_to_duty(crate::ai::DutyFlags::empty()),
            (AttackingDummyBehaviour, EventDone) => {
                let direction = (self
                    .engine
                    .expect_entity(self.owner, "dummy direction")
                    .element_data()
                    .direction() as u16
                    + 3)
                    & 15;
                self.duty_face_direction(direction);
            }
            (AttackingApproachToObserve, EventTimer) => {
                let target = self
                    .engine
                    .live_ai_position(self.engine.combat_event_primary(self.owner));
                let here = self.engine.live_ai_position(self.owner);
                let direction = crate::position_interface::vector_to_sector_0_to_15_iso(
                    target.x - here.x,
                    target.y - here.y,
                );
                self.engine
                    .execute_ai_direction_goal(self.owner, direction as u16);

                self.combat_event_stop();
                self.combat_event_raise_sword();
                self.duty_set_state(AiState::Attacking, AttackingObserve);
                self.engine
                    .combat_event_ai_mut(self.owner)
                    .base
                    .set_emoticon(EmoticonType::None);

                self.engine.combat_event_timer(self.owner, 50);
            }
            (AttackingTooProudToAttack, EventTimer) => {
                self.engine.reinitialize_live_ai_enemies(self.owner);
                self.engine
                    .combat_event_ai_mut(self.owner)
                    .base
                    .set_emoticon(EmoticonType::None);

                self.duty_set_state(AiState::Attacking, AttackingTooProudToAttackOverview);
                if crate::sim_rng::u32(self.tcx.sim, crate::sim_rng::RngSite::TooProudLook, 0..16)
                    == 0
                {
                    self.execute_ai_look_sidewards(crate::ai::LookDirection::LeftRight);
                } else {
                    self.engine.combat_event_timer(self.owner, 20);
                }
            }
            (AttackingTooProudToAttackOverview, EventDone) => {
                self.engine.combat_event_focus(self.owner);
                self.engine.combat_event_timer(self.owner, 5);
            }
            (AttackingTooProudToAttackOverview, EventTimer) => {
                self.execute_battle_decisions();
                let ai = self.engine.combat_event_ai(self.owner);
                if ai.base.current_substate.is_any_swordfight() {
                    let remark = if ai.is_vip {
                        Remark::VipProudFinallyFight
                    } else {
                        Remark::ProudFinallyFight
                    };
                    self.combat_event_say(remark);
                }
            }
            (AttackingTooProudToAttackRetire, EventReachPoint) => {
                self.duty_set_state(AiState::Attacking, AttackingTooProudToAttackRetireTurn);
                let position = self.engine.combat_event_ai(self.owner).base.seek_position;
                self.duty_face_position_ground(position);
            }
            (AttackingTooProudToAttackRetireTurn, EventDone)
            | (AttackingTooProudToAttackApproach, EventReachPoint) => {
                if self
                    .engine
                    .combat_event_visible_primary(self.tcx.assets, self.owner)
                {
                    self.execute_battle_decisions();
                } else {
                    self.execute_ai_get_battle_overview(0);
                }
            }
            (AttackingArcherRetireFromCombat, EventReachPoint) => {
                self.duty_set_state(AiState::Attacking, AttackingArcherRetireFromCombatTurn);
                if self
                    .engine
                    .combat_event_ai(self.owner)
                    .base
                    .primary_target
                    .is_some()
                {
                    let target = self.engine.combat_event_primary(self.owner);
                    let position = self.engine.live_ai_position(target);
                    let elevation = self
                        .engine
                        .expect_entity(target, "retiring face target")
                        .element_data()
                        .position()
                        .z as i16;
                    self.duty_face_position_signed_elevation(position, elevation, true);
                } else {
                    let position = self.engine.combat_event_ai(self.owner).base.seek_position;
                    self.duty_face_position_signed_elevation(position, 1, false);
                }
            }
            (AttackingArcherRetireFromCombatTurn, EventDone) => {
                if self
                    .engine
                    .combat_event_visible_primary(self.tcx.assets, self.owner)
                {
                    let target = self
                        .engine
                        .combat_event_ai(self.owner)
                        .base
                        .primary_target
                        .expect("visible target")
                        .get();
                    if !self
                        .engine
                        .combat_event_ai(self.owner)
                        .list_them
                        .contains(&target)
                    {
                        self.engine
                            .combat_event_ai_mut(self.owner)
                            .list_them
                            .push(target);
                    }
                    self.execute_battle_decisions();
                } else {
                    self.execute_ai_get_battle_overview(0);
                }
            }
            (AttackingApproachingSleepingEnemy, EventReachPoint) => {
                self.combat_event_face_primary()
            }
            (AttackingApproachingSleepingEnemy, EventDone) => self.combat_event_sleeping_enemy(),
            (AttackingKillingSleepingEnemy, EventDone) => {
                self.combat_event_say(Remark::KilledAdversary);
                self.execute_ai_get_battle_overview(0);
            }
            (AttackingRiderChargingApproaching, EventGaloppLoopEnd) => {
                if !self.execute_ai_maybe_make_rider_attack() {
                    self.duty_set_state(AiState::Attacking, AttackingRunningToEnemy);
                    self.execute_ai_reconsider_enemy_approach(true);
                }
            }
            (AttackingRiderChargingApproaching, EventReachPoint) => {
                self.execute_ai_get_battle_overview(0)
            }
            (AttackingRiderChargingPassing, EventReachPoint) => {
                self.duty_set_state(AiState::Attacking, AttackingRiderChargingGettingDistance);
                if let Some(goal) = self.engine.combat_event_rider_retreat_goal(self.owner) {
                    self.duty_go_to(goal, GotoFlags::RUN);
                } else {
                    self.engine.dispatch_think_with_drain(
                        self.tcx,
                        self.owner,
                        &crate::ai::Stimulus::new(EventReachPoint),
                    );
                }
            }
            (AttackingRiderChargingGettingDistance, EventReachPoint) => {
                let position = self.engine.combat_event_ai(self.owner).base.seek_position;
                self.duty_face_position_ground(position);
                self.duty_set_state(AiState::Attacking, AttackingRiderChargingReturning);
            }
            (AttackingRiderChargingReturning, EventDone) => {
                self.engine.reinitialize_live_ai_enemies(self.owner);
                if self.engine.combat_event_ai(self.owner).list_them.is_empty() {
                    self.duty_set_state(
                        AiState::Attacking,
                        AttackingRiderChargingApproachingBlindly,
                    );
                    let destination = self.engine.combat_event_ai(self.owner).base.seek_position;
                    self.duty_go_to(destination, GotoFlags::RUN);
                } else {
                    self.execute_battle_decisions();
                }
            }
            (AttackingRiderChargingApproachingBlindly, EventReachPoint) => {
                self.duty_set_state(AiState::Wondering, WonderingLooking1);
                self.engine.combat_event_timer(self.owner, 30);
            }
            _ => return false,
        }
        true
    }

    fn combat_event_sleeping_enemy(&mut self) {
        let target = self.engine.combat_event_primary(self.owner);
        let entity = self.engine.expect_entity(target, "sleeping target");
        if !entity.is_unconscious() {
            self.execute_ai_get_battle_overview(0);
            return;
        }
        if let Entity::Pc(pc) = entity {
            let index = pc
                .pc
                .campaign_description_index
                .expect("sleeping PC campaign character") as usize;
            if self.engine.mission_domain.campaign.characters[index]
                .status
                .in_coma
            {
                if pc.pc.guard.is_some() {
                    self.execute_ai_return_to_duty(crate::ai::DutyFlags::empty());
                    return;
                }
                // The chosen patient survives the state and command callbacks.
                let EntityId::Pc(patient) = target else {
                    unreachable!()
                };
                self.combat_event_stop();
                self.duty_set_state(AiState::Menacing, Substate::MenacingPcInComa);
                if self.engine.combat_event_ai(self.owner).is_vip {
                    self.combat_event_stop();
                    self.combat_event_raise_sword();
                } else {
                    self.combat_event_say(Remark::MenacesPcInComa);
                    self.combat_event_command(crate::element::Command::StartMenace);
                    self.engine.set_live_guarded_pc(self.owner, Some(patient));
                }
                self.engine.combat_event_timer(self.owner, 20);
                return;
            }
        }
        if self.engine.combat_event_distance(self.owner, target) > 40.0 {
            let position = self.engine.live_ai_position(target);
            self.duty_go_near(position, 20, GotoFlags::RUN);
        } else {
            self.duty_set_state(AiState::Attacking, Substate::AttackingKillingSleepingEnemy);
            self.combat_event_stop();
            let target = self.engine.combat_event_primary(self.owner);
            let mut sequence = crate::sequence::Sequence::new();
            sequence.append_element(crate::sequence::SequenceElement::new_interaction(
                1,
                crate::element::Command::SwordstrikeDown,
                Some(self.owner),
                Some(target),
            ));
            self.engine.launch_sequence(self.tcx, sequence);
        }
    }

    fn combat_event_approached_new_enemy(&mut self) {
        let target = self.engine.combat_event_primary(self.owner);
        assert!(
            self.engine
                .sleeping_enemy_attack_allowed(self.owner, target)
        );
        let weapon = self.engine.combat_event_ai(self.owner).hth_weapon_id;
        let range = self
            .tcx
            .assets
            .profile_manager
            .get_hth_weapon(weapon)
            .expect("combat sword profile")
            .distance[crate::weapons::WeaponDistance::Default as usize];
        let margin = u32::from(range) + 10;
        let close = self.engine.combat_event_square_distance(self.owner, target)
            < margin.wrapping_mul(margin) as f32;
        if !close {
            let position = self.engine.live_ai_position(target);
            self.duty_go_near(position, i32::from(range), GotoFlags::RUN);
            if !self
                .engine
                .combat_event_ai(self.owner)
                .base
                .already_on_point
            {
                return;
            }
            self.engine
                .combat_event_ai_mut(self.owner)
                .base
                .already_on_point = false;
        }
        self.combat_event_state(Substate::AttackingSwordfight, 20);
        let target = self.engine.combat_event_ai(self.owner).base.primary_target;
        if let Some(target) = target {
            let target = self
                .engine
                .expect_human_id_for_ai_handle(target.get(), "combat principal");
            self.engine
                .set_as_new_principal_opponent(self.tcx, self.owner, target);
        }
    }

    fn combat_event_reserve(&mut self, coordinate: bool) {
        if coordinate {
            let count = self.engine.combat_event_ai(self.owner).base.list_us.len();
            for index in 0..count {
                let handle = self.engine.combat_event_ai(self.owner).base.list_us[index];
                let friend = self
                    .engine
                    .expect_human_id_for_ai_handle(handle, "reserve friend");
                if friend != self.owner
                    && matches!(
                        self.engine.expect_entity(friend, "reserve friend kind"),
                        Entity::Soldier(_)
                    )
                    && self
                        .engine
                        .ai(friend, "reserve friend state")
                        .current_substate
                        == Substate::AttackingReserve
                {
                    self.engine.dispatch_think_with_drain(
                        self.tcx,
                        friend,
                        &crate::ai::Stimulus::new(StimulusType::CallCoordinate),
                    );
                }
            }
        }
        self.engine.reinitialize_live_ai_enemies(self.owner);
        self.engine
            .combat_event_ai_mut(self.owner)
            .base
            .set_emoticon(EmoticonType::None);

        self.combat_event_state(Substate::AttackingReserveOverview, 20);
    }
}
