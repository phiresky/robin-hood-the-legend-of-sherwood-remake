//! Actor-specific return to duty, executed synchronously against live owners.

use super::*;
use crate::ai::{AiEntityHandle, AiLockFlags, AiState, DutyFlags, GotoFlags, Stimulus, Substate};
use crate::ai_enemy::{EnemyAi, SeekFlags, task_priority};
use crate::element::Human as _;
use crate::profiles::ProfileRank;

struct DutyExecution<'a> {
    engine: &'a mut EngineInner,
    sim: &'a crate::sim_rng::SimulationContext,
    assets: &'a LevelAssets,
    owner: EntityId,
}

impl EngineInner {
    pub(in crate::engine) fn execute_specialized_ai_duty(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        flags: DutyFlags,
    ) {
        if self
            .world
            .entities
            .expect_entity(owner, format_args!("duty owner"))
            .enemy_ai()
            .is_some()
        {
            DutyExecution {
                engine: self,
                sim,
                assets,
                owner,
            }
            .enemy_duty(flags);
            return;
        }
        self.world
            .entities
            .expect_entity_mut(owner, format_args!("friendly duty owner"))
            .friendly_ai_mut()
            .expect("duty owner has neither enemy nor friendly AI")
            .fleeing_seen_enemy_counter = 0;
        if self.is_very_very_busy(owner) {
            let ai = self
                .world
                .entities
                .expect_ai_controller_mut(owner, format_args!("busy duty owner"));
            ai.non_script_lock(AiLockFlags::BUSY);
            ai.was_busy = true;
            self.execute_ai_callback(
                sim,
                assets,
                owner,
                &Stimulus::new(StimulusType::EventReturnToDuty),
            );
            return;
        }
        self.execute_common_ai_duty(sim, assets, owner, flags);
    }
}

impl DutyExecution<'_> {
    fn enemy(&self) -> &EnemyAi {
        self.engine
            .world
            .entities
            .expect_enemy_ai(self.owner, format_args!("duty owner"))
    }

    fn enemy_mut(&mut self) -> &mut EnemyAi {
        self.engine
            .world
            .entities
            .expect_enemy_ai_mut(self.owner, format_args!("duty owner"))
    }

    fn state(&mut self, state: AiState, substate: Substate) {
        self.engine
            .duty_set_state(self.sim, self.assets, self.owner, state, substate);
    }

    fn go_near(&mut self, position: crate::ai::Position, distance: i32, flags: GotoFlags) {
        self.engine
            .duty_go_near(self.sim, self.assets, self.owner, position, distance, flags);
    }

    fn timer(&mut self, frames: u32) {
        let frame = self.engine.control.frame_counter;
        self.enemy_mut().base.launch_timer(frames, frame);
    }

    fn object_id(&self, handle: u32) -> EntityId {
        self.engine.entity_id_for_index(handle).unwrap_or_else(|| {
            panic!(
                "duty owner {:?} references missing object {handle}",
                self.owner
            )
        })
    }

    fn object_position(&self, handle: AiEntityHandle) -> crate::ai::Position {
        self.engine.live_ai_position(self.object_id(handle.get()))
    }

    fn camp_soldiers(&self) -> impl Iterator<Item = EntityId> + '_ {
        let camp = self
            .engine
            .world
            .entities
            .expect_entity(self.owner, format_args!("duty camp owner"))
            .camp();
        self.engine
            .world
            .soldier_registry
            .camp(camp)
            .iter()
            .map(|&handle| EntityId::Soldier(crate::entity_id::SoldierId(handle)))
    }

    fn enemy_duty(&mut self, flags: DutyFlags) {
        self.enemy_mut().investigating_distraction = false;
        self.engine
            .execute_ai_delete_detectable_type(self.owner, crate::element::DetectableType::Beggar);

        {
            let enemy = self.enemy_mut();
            enemy.beggar_to_examine = None;
            enemy.known_enemy_strike_1 = None;
            enemy.known_enemy_strike_2 = None;
            enemy.known_enemy_strike_3 = None;
            self.engine.execute_ai_unfocus(self.owner);
        }

        self.enemy_mut().fleeing_seen_enemy_counter = 0;

        if self
            .enemy()
            .seek_flags
            .contains(SeekFlags::REPORT_OFFICER_AFTER)
            && self.enemy().base.antagonist.is_some()
            && !flags.contains(DutyFlags::BECAUSE_COULDNT_REACHPOINT)
        {
            self.state(AiState::Seeking, Substate::SeekingSoldierReturnToOfficer);
            self.enemy_mut().base.clear_emoticon();

            let position = self.enemy().officers_position;
            self.go_near(position, 40, GotoFlags::RUN);
            if self.enemy().base.already_on_point {
                self.enemy_mut().base.already_on_point = false;
            } else {
                self.timer(20);
                return;
            }
        }

        if self
            .enemy()
            .seek_flags
            .contains(SeekFlags::LOOK_FOR_HELP_AFTER)
            && !flags.contains(DutyFlags::BECAUSE_COULDNT_REACHPOINT)
        {
            self.enemy_mut().seek_flags = SeekFlags::empty();
            debug_assert_eq!(
                self.enemy().get_rank(&self.assets.profile_manager),
                ProfileRank::Soldier
            );
            if self.alert_officer_after_search() {
                return;
            }
        }

        {
            let enemy = self.enemy_mut();
            enemy.base.friends_are_alerted = false;
            enemy.seek_flags = SeekFlags::empty();
            enemy.base.sorrow_level = 0;
            enemy.phalanx_aborted = false;
            enemy.base.antagonist = None;
            enemy.current_task_priority = enemy.minimal_task_priority;
        }
        let has_missed_friend = !self
            .engine
            .world
            .entities
            .expect_entity(self.owner, format_args!("duty missed-friend owner"))
            .ai_actor_data()
            .expect("duty owner has no AI actor data")
            .detectable_lists[crate::element::DetectableType::MissedFriend as usize]
            .is_empty();
        if has_missed_friend {
            if let Some(checkpoint) = self.enemy().base.checkpoint_charly {
                self.enemy_mut()
                    .base
                    .missed_in_action
                    .push(checkpoint.get());
                self.engine
                    .execute_ai_set_checkpoint_charly(self.owner, None);
            }
        }

        let substate = self.enemy().base.current_substate;
        if substate.is_take_money() || substate.is_fight_for_money() {
            let owner = self
                .engine
                .world
                .entities
                .expect_entity(self.owner, format_args!("duty money owner"));
            let takes_money = self.enemy().base.blood_alcohol as i32
                > crate::parameters_ai::AI_DEBILITY_ALCOHOL_LIMIT
                || (owner.is_active()
                    && self
                        .engine
                        .entity_building_sector(owner.element_data().sector())
                        .is_none()
                    && self.enemy().profile(&self.assets.profile_manager).money > 0);
            if takes_money
                && !flags.contains(DutyFlags::BECAUSE_COULDNT_REACHPOINT)
                && self
                    .enemy()
                    .base
                    .interesting_object
                    .is_none_or(|object| !self.angry_officer_near(self.object_position(object)))
            {
                self.clean_seen_money();
                if self.enemy().base.interesting_object.is_none() {
                    let coin = self.take_nearest_seen_money();
                    self.enemy_mut().base.interesting_object = coin.map(AiEntityHandle::new);
                }
                if self.enemy().base.interesting_object.is_some() {
                    self.state(AiState::Wondering, Substate::WonderingApproachingMoney);
                    let object = self
                        .enemy()
                        .base
                        .interesting_object
                        .expect("money state callback cleared the required target");
                    let position = self.object_position(object);
                    self.go_near(
                        position,
                        crate::parameters_ai::AI_STOP_BEFORE_MONEY_DISTANCE,
                        GotoFlags::FIND_ACCESSIBLE,
                    );
                    self.timer(5);
                    return;
                }
            }
        }

        if self.enemy().return_to_patrol_point.sector.is_some() {
            if !self.enemy().base.patrol.is_empty() {
                self.state(AiState::Default, Substate::DefaultPatrolChiefReturnToPatrol);
                let position = self.enemy().return_to_patrol_point;
                self.engine.duty_go_to(
                    self.sim,
                    self.assets,
                    self.owner,
                    position,
                    GotoFlags::empty(),
                );
                self.enemy_mut().return_to_patrol_point.sector = None;
                return;
            }
            self.enemy_mut().return_to_patrol_point.sector = None;
        }

        if !self.enemy().other_seen_ale.is_empty()
            && !flags.contains(DutyFlags::BECAUSE_COULDNT_REACHPOINT)
        {
            let ale = AiEntityHandle::new(self.enemy_mut().other_seen_ale.remove(0));
            self.enemy_mut().base.interesting_object = Some(ale);
            self.enemy_mut().base.object_of_desire = Some(ale);
            self.state(AiState::Wondering, Substate::WonderingApproachingAle);
            let target = self
                .enemy()
                .base
                .interesting_object
                .expect("ale state callback cleared the required target");
            let position = self.object_position(target);
            self.go_near(
                position,
                crate::parameters_ai::AI_STOP_BEFORE_MONEY_DISTANCE,
                GotoFlags::FIND_ACCESSIBLE,
            );
            let owner_position = self.engine.live_ai_position(self.owner);
            self.enemy_mut().return_to_patrol_point = owner_position;
            self.timer(1);
            return;
        }

        self.engine
            .initialize_patrol_for_npc(self.assets, self.owner);
        self.engine
            .execute_common_ai_duty(self.sim, self.assets, self.owner, flags);
    }

    fn angry_officer_near(&self, position: crate::ai::Position) -> bool {
        self.camp_soldiers().any(|id| {
            let other = self
                .engine
                .world
                .entities
                .expect_ai_controller(id, format_args!("money officer"));
            if matches!(
                other.current_substate,
                Substate::WonderingOfficerSeeingBrawl
                    | Substate::WonderingOfficerApproachingBrawl
                    | Substate::WonderingOfficerFinishingBrawl
            ) {
                let other_position = self.engine.live_ai_position(id);
                (other_position.x - position.x)
                    .abs()
                    .max((other_position.y - position.y).abs())
                    < 150.0
            } else {
                false
            }
        })
    }

    fn clean_seen_money(&mut self) {
        let mut index = 0;
        while index < self.enemy().other_seen_money.len() {
            let handle = self.enemy().other_seen_money[index];
            let active = self
                .engine
                .world
                .entities
                .expect_entity(self.object_id(handle), format_args!("remembered coin"))
                .is_active();
            if active {
                index += 1;
            } else {
                self.enemy_mut().other_seen_money.remove(index);
            }
        }
        if let Some(object) = self.enemy().base.interesting_object
            && !self
                .engine
                .world
                .entities
                .expect_entity(
                    self.object_id(object.get()),
                    format_args!("interesting coin"),
                )
                .is_active()
        {
            self.enemy_mut().base.interesting_object = None;
        }
    }

    fn take_nearest_seen_money(&mut self) -> Option<u32> {
        self.clean_seen_money();
        if self.enemy().other_seen_money.is_empty() {
            return None;
        }
        let owner_position = self.engine.live_ai_position(self.owner);
        let mut nearest_index = 0;
        let mut nearest_distance = 65_432_u16;
        for (index, &handle) in self.enemy().other_seen_money.iter().enumerate() {
            let id = self.object_id(handle);
            let position = self.engine.live_ai_position(id);
            let mut distance = (position.x - owner_position.x)
                .abs()
                .max((position.y - owner_position.y).abs()) as u16;
            if self
                .engine
                .world
                .entities
                .expect_entity(id, format_args!("coin layer"))
                .element_data()
                .layer()
                != owner_position.level
            {
                distance = distance.wrapping_add(300);
            }
            if distance < nearest_distance {
                nearest_distance = distance;
                nearest_index = index;
            }
        }
        Some(self.enemy_mut().other_seen_money.remove(nearest_index))
    }

    fn alert_officer_after_search(&mut self) -> bool {
        self.engine.execute_ai_unfocus(self.owner);

        let mut nearest = None;
        let mut nearest_distance = crate::ai_enemy::combat::MAX_ALERT_OFFICER_RADIUS as u32;
        for id in self.camp_soldiers() {
            let entity = self
                .engine
                .world
                .entities
                .expect_entity(id, format_args!("officer candidate"));
            let ai = entity
                .enemy_ai()
                .expect("officer registry soldier has no Enemy AI");
            match ai.get_rank(&self.assets.profile_manager) {
                ProfileRank::Officer
                    if matches!(entity, Entity::Soldier(soldier) if soldier.is_able_to_fight())
                        && ai.base.current_state == AiState::Default
                        && !ai.base.script_locked =>
                {
                    let owner_element = self
                        .engine
                        .world
                        .entities
                        .expect_entity(self.owner, format_args!("officer search owner"))
                        .element_data();
                    let element = entity.element_data();
                    let owner_world = owner_element.position();
                    let world = element.position();
                    let mut distance = (world.x - owner_world.x)
                        .abs()
                        .max(
                            ((world.y - owner_world.y)
                                * crate::position_interface::INVERSE_ASPECT_RATIO)
                                .abs(),
                        )
                        .max((world.z - owner_world.z).abs())
                        as u32;
                    if self.engine.entity_data_in_building_sector(element) {
                        distance += (crate::parameters_ai::LAYER_CHANGE_PENALTY
                            * (i32::from(owner_element.layer()) - i32::from(element.layer()))
                                .unsigned_abs() as f32) as u32;
                    }
                    if distance < nearest_distance {
                        nearest_distance = distance;
                        nearest = Some(id);
                    }
                }
                ProfileRank::Soldier
                    if matches!(
                        ai.base.current_substate,
                        Substate::SeekingSoldierCalledByOfficer
                            | Substate::SeekingSoldierGoToOfficer
                            | Substate::SeekingSoldierGetInstructedByOfficer
                            | Substate::SeekingSoldierReturnToOfficer
                            | Substate::SeekingSoldierGiveReportToOfficer
                            | Substate::SeekingSoldierGiveAlertingReportToOfficerStart
                            | Substate::SeekingSoldierGiveAlertingReportToOfficerPoint
                            | Substate::SeekingSoldierGiveAlertingReportToOfficerEnd
                            | Substate::SeekingGroupCalledByOfficer
                            | Substate::SeekingGroupGoToOfficer
                            | Substate::SeekingGroupGetInstructedByOfficer
                            | Substate::SeekingRunningToOfficer
                            | Substate::SeekingRunningToOfficerSeen
                    ) && self
                        .engine
                        .patrol_member_visible(self.assets, self.owner, id) =>
                {
                    return false;
                }
                _ => {}
            }
        }
        let Some(officer) = nearest else {
            self.enemy_mut().seek_flags = SeekFlags::empty();
            return false;
        };
        self.enemy_mut().current_task_priority = task_priority::ALERT;
        self.state(AiState::Seeking, Substate::SeekingRunningToOfficer);
        self.enemy_mut().base.antagonist = Some(AiEntityHandle::new(officer.index()));
        self.engine.execute_ai_append_detectable(
            self.owner,
            officer,
            crate::element::DetectableType::Friend,
        );

        let entity = self
            .engine
            .world
            .entities
            .expect_entity(officer, format_args!("selected officer"));
        let passing_door =
            selected_actor_is_passing_door(&self.engine.orders.sequence_manager, officer);
        let input = extract_exact_forecast_input(self.engine, entity, passing_door)
            .expect("officer forecast requires an actor");
        let destination = crate::ai::forecast_destination_for_ia(
            self.sim,
            &input,
            &self.engine.script_domains.interactables.doors,
            &self.engine.world.fast_grid.level.sectors,
            &self.engine.world.fast_grid.level.sector_number_map,
        )
        .position;
        self.enemy_mut().gather_position = destination;
        self.go_near(
            destination,
            crate::parameters_ai::AI_TALK_DISTANCE,
            GotoFlags::RUN,
        );
        self.timer(50);
        if self.enemy().base.couldnt_reachpoint {
            self.enemy_mut().base.couldnt_reachpoint = false;
            return false;
        }
        true
    }
}
